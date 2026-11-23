//! Borrowed x86 storage, not a device bus. Cached RAM hits retain typed host
//! offsets; physical-map decisions stay outside instruction handlers.
use core::ops::Range;

use super::flat::{FlatParts, checked_range};
use super::{BxMemC, CpuMemoryPolicy, MemoryError, PhysAccess, Result};
use crate::config::{A20Mask, BxPhyAddress};
use crate::cpu::rusty_box::MemoryAccessType;

pub(crate) enum ExecutionMemory<'a> {
    Pc(&'a mut BxMemC),
    Borrowed(FlatParts<'a>),
}

/// One immutable observation under the execution loan; neither read can
/// dispatch a device or relocate backing. Avoid two backend tests per trace.
pub(crate) struct CacheState {
    pub(crate) code_sequence: u64,
    pub(crate) residency_epoch: u64,
}
impl ExecutionMemory<'_> {
    pub(crate) fn reborrow(&mut self) -> ExecutionMemory<'_> {
        match self {
            Self::Pc(memory) => ExecutionMemory::Pc(memory),
            Self::Borrowed(memory) => ExecutionMemory::Borrowed(memory.reborrow()),
        }
    }

    #[inline(always)]
    pub(crate) fn cache_state(&self) -> CacheState {
        match self {
            Self::Pc(mem) => CacheState {
                code_sequence: mem.smc_seq_next(),
                residency_epoch: mem.swap_epoch(),
            },
            Self::Borrowed(mem) => CacheState {
                code_sequence: mem.writes.next(),
                residency_epoch: 0,
            },
        }
    }

    /// Numerical RAM placement and bounds only. A cached direct page already
    /// passed map eligibility at fill; mapping changes invalidate that page.
    /// Keep the existing PCI-hole/bounds rejection on bulk cache hits.
    #[inline(always)]
    pub(crate) fn backing_ram_span(
        &self,
        addr: BxPhyAddress,
        len: usize,
        capacity: usize,
    ) -> Option<Range<usize>> {
        if let Self::Borrowed(mem) = self {
            if mem.regions.is_some() {
                let span = mem.span_from(addr, true)?;
                return (len <= span.len()).then(|| span.start..span.start + len);
            }
        }
        // PC and dense RAM place low RAM identically. Only spans reaching
        // the PCI aperture need the placement policy below.
        if addr < super::memory_rusty_box::BX_PCI_HOLE_START {
            let span = checked_range(addr, len, capacity)?;
            if span.end as u64 <= super::memory_rusty_box::BX_PCI_HOLE_START.get() {
                return Some(span);
            }
        }
        match self {
            Self::Pc(_) => super::memory_rusty_box::bx_guest_ram_span(addr, len, capacity),
            Self::Borrowed(_) => checked_range(addr, len, capacity),
        }
    }

    #[cfg(test)]
    pub(crate) fn pc_mut(&mut self) -> Option<&mut BxMemC> {
        match self {
            Self::Pc(mem) => Some(mem),
            Self::Borrowed(_) => None,
        }
    }

    #[inline]
    pub(crate) fn a20_mask(&self) -> A20Mask {
        match self {
            Self::Pc(mem) => mem.a20_mask(),
            Self::Borrowed(_) => A20Mask::OPEN,
        }
    }

    pub(crate) fn a20_addr(&self, addr: BxPhyAddress) -> BxPhyAddress {
        self.a20_mask().apply(addr)
    }

    pub(crate) fn get_memory_len(&self) -> usize {
        match self {
            Self::Pc(mem) => mem.get_memory_len(),
            Self::Borrowed(mem) => mem.bytes.len(),
        }
    }

    #[inline(always)]
    pub(crate) fn swap_epoch(&self) -> u64 {
        match self {
            Self::Pc(mem) => mem.swap_epoch(),
            Self::Borrowed(_) => 0,
        }
    }

    /// Already A20-adjusted physical address to an identity RAM offset.
    #[inline]
    pub(crate) fn ram_span(
        &self,
        addr: BxPhyAddress,
        len: usize,
        capacity: usize,
    ) -> Option<Range<usize>> {
        match self {
            Self::Pc(mem) => {
                let end = addr.checked_add(u64::try_from(len).ok()?)?;
                if !((addr.get() < 0xA0000 && end.get() <= 0xA0000) || addr.get() >= 0x100000)
                    || mem.mmio.overlaps_page(addr)
                    || mem
                        .mmio
                        .overlaps_page(BxPhyAddress::new(end.get().saturating_sub(1)))
                {
                    return None;
                }
            }
            Self::Borrowed(_) => {}
        }
        self.backing_ram_span(addr, len, capacity)
    }

    pub(crate) fn host_mem_range(
        &mut self,
        addr: BxPhyAddress,
        rw: MemoryAccessType,
        policy: CpuMemoryPolicy,
    ) -> Result<Option<Range<usize>>> {
        match self {
            Self::Pc(mem) => Ok(mem.host_mem_range(addr, rw, policy)?.and_then(|range| {
                let a20 = mem.a20_addr(addr);
                // A direct mapping at a device-owned address witnesses the
                // PC's SMRAM execute override. Ordinary MMIO returned None.
                if mem.mmio.covers(a20) {
                    return Some(range);
                }
                let len = mem
                    .mmio
                    .direct_len(a20, range.len().min(4096 - addr.page_offset() as usize));
                (len != 0).then_some(range.start..range.start + len)
            })),
            Self::Borrowed(mem) => {
                if (rw as u32 & 1) != 0 && policy.monitor_hit() {
                    return Ok(None);
                }
                Ok(mem.span_from(addr, (rw as u32 & 1) != 0))
            }
        }
    }

    pub(crate) fn get_host_mem_addr(
        &mut self,
        addr: BxPhyAddress,
        rw: MemoryAccessType,
        policy: CpuMemoryPolicy,
    ) -> Result<Option<&mut [u8]>> {
        let Some(range) = self.host_mem_range(addr, rw, policy)? else {
            return Ok(None);
        };
        match self {
            Self::Pc(mem) => {
                Ok(mem.allocation_slice(u64::try_from(range.start)?, u64::try_from(range.len())?))
            }
            Self::Borrowed(mem) => Ok(mem.bytes.get_mut(range)),
        }
    }

    pub(crate) fn physical_access_len(&self, addr: BxPhyAddress, len: usize) -> usize {
        match self {
            Self::Pc(mem) => mem.mmio.access_len(mem.a20_addr(addr), len),
            Self::Borrowed(_) => len,
        }
    }

    pub(crate) fn read_physical_page(
        &mut self,
        policy: CpuMemoryPolicy,
        addr: BxPhyAddress,
        len: usize,
        out: &mut [u8],
    ) -> Result<PhysAccess> {
        match self {
            Self::Pc(mem) => mem.read_physical_page(policy, addr, len, out),
            Self::Borrowed(mem) => {
                if len > out.len() || len > 4096 - addr.page_offset() as usize {
                    return Err(MemoryError::ReadPhysicalPage { addr, len }.into());
                }
                // Sparse windows are page aligned. The only partial backed
                // page is the end of dense RAM.
                let out = &mut out[..len];
                if let Some(span) = mem.span_from(addr, false) {
                    let count = len.min(span.len());
                    out[..count].copy_from_slice(&mem.bytes[span.start..span.start + count]);
                    out[count..].fill(0xFF);
                } else if !mem.read_indirect(addr, out) {
                    out.fill(0xFF);
                }
                Ok(PhysAccess::Done)
            }
        }
    }

    pub(crate) fn write_physical_page(
        &mut self,
        policy: CpuMemoryPolicy,
        addr: BxPhyAddress,
        len: usize,
        data: &mut [u8],
    ) -> Result<PhysAccess> {
        match self {
            Self::Pc(mem) => mem.write_physical_page(policy, addr, len, data),
            Self::Borrowed(mem) => {
                if len > data.len() || len > 4096 - addr.page_offset() as usize {
                    return Err(MemoryError::WritePhysicalPage { addr, len }.into());
                }
                if let Some(span) = mem.span_from(addr, true) {
                    let count = len.min(span.len());
                    mem.bytes[span.start..span.start + count].copy_from_slice(&data[..count]);
                    mem.mark_write(addr, count);
                } else if mem.write_indirect(addr, &data[..len]) {
                    return Ok(PhysAccess::Done);
                }
                Ok(PhysAccess::Done)
            }
        }
    }

    pub(crate) fn read_ram(&mut self, addr: BxPhyAddress, out: &mut [u8]) -> Result<usize> {
        match self {
            Self::Pc(mem) => mem.read_ram(addr, out),
            Self::Borrowed(mem) => {
                if out.is_empty()
                    && mem.regions.is_none()
                    && checked_range(addr, 0, mem.bytes.len()).is_some()
                {
                    return Ok(0);
                }
                if let Some(span) = mem
                    .span_from(addr, false)
                    .filter(|span| out.len() <= span.len())
                {
                    let range = span.start..span.start + out.len();
                    out.copy_from_slice(&mem.bytes[range]);
                } else if !mem.read_indirect(addr, out) {
                    return Err(MemoryError::RamImageOutOfRange.into());
                }
                Ok(out.len())
            }
        }
    }

    /// Tracked host/DMA writer between interpreter runs in the same loan.
    pub(crate) fn write_ram(&mut self, addr: BxPhyAddress, data: &[u8]) -> Result<usize> {
        match self {
            Self::Pc(mem) => mem.write_ram(addr, data),
            Self::Borrowed(mem) => {
                if data.is_empty()
                    && mem.regions.is_none()
                    && checked_range(addr, 0, mem.bytes.len()).is_some()
                {
                    return Ok(0);
                }
                if let Some(span) = mem
                    .span_from(addr, true)
                    .filter(|span| data.len() <= span.len())
                {
                    let range = span.start..span.start + data.len();
                    mem.bytes[range].copy_from_slice(data);
                    mem.mark_write(addr, data.len());
                } else if !mem.write_indirect(addr, data) {
                    return Err(MemoryError::RamImageOutOfRange.into());
                }
                Ok(data.len())
            }
        }
    }

    pub(crate) fn smc_mark_icache_mask(&mut self, addr: BxPhyAddress, mask: u32) {
        match self {
            Self::Pc(mem) => mem.smc_mark_icache_mask(addr, mask),
            Self::Borrowed(mem) => {
                if let Some(stamp) = mem
                    .stamp_index(addr)
                    .and_then(|index| mem.stamps.get_mut(index))
                {
                    *stamp |= mask;
                }
            }
        }
    }

    #[inline(always)]
    pub(crate) fn smc_range_has_stamps(&self, addr: BxPhyAddress, len: u32) -> bool {
        match self {
            Self::Pc(mem) => mem.smc_range_has_stamps(addr, len),
            Self::Borrowed(mem) => mem
                .stamp_index(addr)
                .and_then(|index| mem.stamps.get(index))
                .is_some_and(|stamp| {
                    stamp & crate::cpu::icache::smc_cache_line_mask(addr, len) != 0
                }),
        }
    }

    /// Publish a write and observe its sequence under the same backend match.
    /// The CPU still checks the sequence after every store, including a miss
    /// in the stamp table: another writer may have left pending events.
    #[inline(always)]
    pub(crate) fn smc_dec_write_stamp(
        &mut self,
        addr: BxPhyAddress,
        len: u32,
        cached_page: Option<crate::cpu::tlb::RamPage>,
    ) -> u64 {
        match self {
            Self::Pc(mem) => {
                mem.smc_dec_write_stamp(addr, len);
                mem.smc_seq_next()
            }
            Self::Borrowed(mem) => {
                if len as usize <= 4096 - addr.page_offset() as usize {
                    // The DTLB's direct RAM proof uses arena coordinates.
                    // It remains valid under the exclusive loan, and every
                    // rebind invalidates it. Events still use the guest GPA.
                    let index = if let Some(page) = cached_page {
                        Some(page.ram_offset() >> 12)
                    } else {
                        mem.stamp_index(addr)
                    };
                    if let Some(index) = index {
                        if index < mem.stamps.len() {
                            mem.writes.write(mem.stamps, index, addr, len);
                        }
                    }
                } else {
                    mem.mark_write(addr, len as usize);
                }
                mem.writes.next()
            }
        }
    }

    #[inline]
    pub(crate) fn smc_seq_next(&self) -> u64 {
        match self {
            Self::Pc(mem) => mem.smc_seq_next(),
            Self::Borrowed(mem) => mem.writes.next(),
        }
    }

    pub(crate) fn smc_pending_since(
        &self,
        since: u64,
    ) -> (bool, &[crate::cpu::icache::PendingSmc]) {
        match self {
            Self::Pc(mem) => mem.smc_pending_since(since),
            Self::Borrowed(mem) => mem.writes.since(since),
        }
    }
}
