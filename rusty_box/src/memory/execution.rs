//! Borrowed x86 storage, not a device bus. Cached RAM hits retain typed host
//! offsets; physical-map decisions stay outside instruction handlers.
use core::ops::Range;

use super::direct::{DirectOffset, DirectParts};
use super::flat::{FlatParts, checked_range};
use super::{BxMemC, CpuMemoryPolicy, MemoryError, PhysAccess, Result};
use crate::config::{A20Mask, BxPhyAddress};
use crate::cpu::rusty_box::MemoryAccessType;

// The tag is read on trace lookup and on cached stores. An explicit tag keeps
// those paths from decoding the niche of the optional indirect RAM loan.
#[repr(u8)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ExecutionMemory<'a> {
    Pc(&'a mut BxMemC),
    Borrowed(FlatParts<'a>),
    Direct(DirectParts<'a>),
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
            Self::Direct(memory) => ExecutionMemory::Direct(memory.reborrow()),
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
            Self::Direct(mem) => CacheState {
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
            Self::Direct(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn pc_mut(&mut self) -> Option<&mut BxMemC> {
        match self {
            Self::Pc(mem) => Some(mem),
            Self::Borrowed(_) | Self::Direct(_) => None,
        }
    }

    #[inline]
    pub(crate) fn a20_mask(&self) -> A20Mask {
        match self {
            Self::Pc(mem) => mem.a20_mask(),
            Self::Borrowed(_) | Self::Direct(_) => A20Mask::OPEN,
        }
    }

    pub(crate) fn a20_addr(&self, addr: BxPhyAddress) -> BxPhyAddress {
        self.a20_mask().apply(addr)
    }

    pub(crate) fn get_memory_len(&self) -> usize {
        match self {
            Self::Pc(mem) => mem.get_memory_len(),
            Self::Borrowed(mem) => mem.bytes.len(),
            Self::Direct(_) => 0,
        }
    }

    #[inline(always)]
    pub(crate) fn swap_epoch(&self) -> u64 {
        match self {
            Self::Pc(mem) => mem.swap_epoch(),
            Self::Borrowed(_) | Self::Direct(_) => 0,
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
            Self::Direct(_) => return None,
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
            // A persistent offset for SVM must name the primary allocation.
            Self::Direct(_) => Ok(None),
        }
    }

    pub(crate) fn fetch_range(
        &mut self,
        addr: BxPhyAddress,
        policy: CpuMemoryPolicy,
    ) -> Result<Option<Range<usize>>> {
        match self {
            Self::Direct(mem) => Ok(mem.span_from(addr, false)),
            _ => self.host_mem_range(addr, MemoryAccessType::Execute, policy),
        }
    }

    pub(crate) fn fetch_ptr(&self, offset: DirectOffset) -> *const u8 {
        let Self::Direct(mem) = self else {
            unreachable!("direct fetch coordinate must be resolved against its original loan");
        };
        mem.fetch_ptr(offset)
    }

    pub(crate) fn direct_ram_page(&self, addr: BxPhyAddress) -> Option<crate::cpu::tlb::RamPage> {
        let Self::Direct(mem) = self else { return None };
        let span = mem.span_from(addr, true)?;
        if span.len() < 4096 {
            return None;
        }
        crate::cpu::tlb::RamPage::from_ram_offset(span.start)
    }

    pub(crate) fn ram_ptr(&mut self, offset: DirectOffset) -> *mut u8 {
        let Self::Direct(mem) = self else {
            unreachable!("direct RAM coordinate must be resolved against its original loan");
        };
        mem.ram_ptr(offset)
    }

    pub(crate) fn permits_external_page(&self, page: BxPhyAddress) -> bool {
        match self {
            Self::Pc(_) => false,
            Self::Borrowed(mem) => !mem.has_backing_at(page),
            Self::Direct(mem) => !mem.has_backing_at(page),
        }
    }

    pub(crate) fn get_host_mem_addr(
        &mut self,
        addr: BxPhyAddress,
        rw: MemoryAccessType,
        policy: CpuMemoryPolicy,
    ) -> Result<Option<&mut [u8]>> {
        if let Self::Direct(mem) = self {
            let writable = (rw as u32 & 1) != 0;
            if writable && policy.monitor_hit() {
                return Ok(None);
            }
            return Ok(mem.bytes_at(addr, writable));
        }
        let Some(range) = self.host_mem_range(addr, rw, policy)? else {
            return Ok(None);
        };
        match self {
            Self::Pc(mem) => {
                Ok(mem.allocation_slice(u64::try_from(range.start)?, u64::try_from(range.len())?))
            }
            Self::Borrowed(mem) => Ok(mem.bytes.get_mut(range)),
            Self::Direct(_) => Ok(None),
        }
    }

    pub(crate) fn physical_access_len(&self, addr: BxPhyAddress, len: usize) -> usize {
        match self {
            Self::Pc(mem) => mem.mmio.access_len(mem.a20_addr(addr), len),
            Self::Borrowed(_) | Self::Direct(_) => len,
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
            Self::Direct(mem) => {
                if len > out.len() || len > 4096 - addr.page_offset() as usize {
                    return Err(MemoryError::ReadPhysicalPage { addr, len }.into());
                }
                let out = &mut out[..len];
                if !mem.read(addr, out) {
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
            Self::Direct(mem) => {
                if len > data.len() || len > 4096 - addr.page_offset() as usize {
                    return Err(MemoryError::WritePhysicalPage { addr, len }.into());
                }
                mem.write(addr, &data[..len]);
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
            Self::Direct(mem) => {
                if !mem.read(addr, out) {
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
            Self::Direct(mem) => {
                if !mem.write(addr, data) {
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
            Self::Direct(mem) => mem.mark_icache(addr, mask),
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
            Self::Direct(mem) => mem.has_stamps(addr, len),
        }
    }

    /// Publish a write and observe its sequence under the same backend match.
    /// The CPU still checks the sequence after every store, including a miss
    /// in the stamp table: another writer may have left pending events.
    /// A cached page comes from a width-aware DTLB hit and covers the whole
    /// store. Callers pass no page for routed and split writes.
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
                if let Some(page) = cached_page {
                    // The full page is backed and has a corresponding stamp
                    // for the duration of the loan. Publish the guest GPA.
                    mem.writes
                        .write(mem.stamps, page.ram_offset() >> 12, addr, len);
                } else if len as usize <= 4096 - addr.page_offset() as usize {
                    if let Some(index) = mem.stamp_index(addr).filter(|&i| i < mem.stamps.len()) {
                        mem.writes.write(mem.stamps, index, addr, len);
                    }
                } else {
                    mem.mark_write(addr, len as usize);
                }
                mem.writes.next()
            }
            Self::Direct(mem) => {
                let cached_offset =
                    cached_page.and_then(|page| DirectOffset::decode(page.ram_offset()));
                if let Some(offset) = cached_offset {
                    mem.mark_cached_write(offset, addr, len);
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
            Self::Direct(mem) => mem.writes.next(),
        }
    }

    pub(crate) fn smc_pending_since(
        &self,
        since: u64,
    ) -> (bool, &[crate::cpu::icache::PendingSmc]) {
        match self {
            Self::Pc(mem) => mem.smc_pending_since(since),
            Self::Borrowed(mem) => mem.writes.since(since),
            Self::Direct(mem) => mem.writes.since(since),
        }
    }
}
