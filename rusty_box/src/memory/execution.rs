//! Borrowed x86 storage, not a device bus. Cached RAM hits retain typed host
//! offsets; physical-map decisions stay outside instruction handlers.
use core::ops::Range;

use super::flat::{FlatParts, checked_range, mark_write};
use super::{BxMemC, CpuMemoryPolicy, MemoryError, PhysAccess, Result};
use crate::config::{A20Mask, BxPhyAddress};
use crate::cpu::rusty_box::MemoryAccessType;

pub(crate) enum ExecutionMemory<'a> {
    Pc(&'a mut BxMemC),
    Flat(FlatParts<'a>),
}

/// One immutable observation under the execution loan; neither read can
/// dispatch a device or relocate backing. Avoid two backend tests per trace.
pub(crate) struct CacheState {
    pub(crate) code_sequence: u64,
    pub(crate) residency_epoch: u64,
}
impl ExecutionMemory<'_> {
    #[inline(always)]
    pub(crate) fn cache_state(&self) -> CacheState {
        match self {
            Self::Pc(mem) => CacheState {
                code_sequence: mem.smc_seq_next(),
                residency_epoch: mem.swap_epoch(),
            },
            Self::Flat(mem) => CacheState {
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
        // Both environments place low RAM identically. A warm direct-page
        // hit needs no backend test unless the span reaches the PC aperture.
        // Flat spans crossing/above that aperture remain dense below.
        if addr < super::memory_rusty_box::BX_PCI_HOLE_START {
            let span = checked_range(addr, len, capacity)?;
            if span.end as u64 <= super::memory_rusty_box::BX_PCI_HOLE_START.get() {
                return Some(span);
            }
        }
        match self {
            Self::Pc(_) => super::memory_rusty_box::bx_guest_ram_span(addr, len, capacity),
            Self::Flat(_) => checked_range(addr, len, capacity),
        }
    }

    #[cfg(test)]
    pub(crate) fn pc_mut(&mut self) -> Option<&mut BxMemC> {
        match self {
            Self::Pc(mem) => Some(mem),
            Self::Flat(_) => None,
        }
    }

    #[inline]
    pub(crate) fn a20_mask(&self) -> A20Mask {
        match self {
            Self::Pc(mem) => mem.a20_mask(),
            Self::Flat(_) => A20Mask::OPEN,
        }
    }

    pub(crate) fn a20_addr(&self, addr: BxPhyAddress) -> BxPhyAddress {
        self.a20_mask().apply(addr)
    }

    pub(crate) fn get_memory_len(&self) -> usize {
        match self {
            Self::Pc(mem) => mem.get_memory_len(),
            Self::Flat(mem) => mem.bytes.len(),
        }
    }

    #[inline(always)]
    pub(crate) fn swap_epoch(&self) -> u64 {
        match self {
            Self::Pc(mem) => mem.swap_epoch(),
            Self::Flat(_) => 0,
        }
    }

    pub(crate) fn identity_guest_base(&mut self) -> (*mut u8, usize) {
        match self {
            Self::Pc(mem) => mem.identity_guest_base(),
            Self::Flat(mem) => (mem.bytes.as_mut_ptr(), mem.bytes.len()),
        }
    }

    pub(crate) fn allocation_span(&mut self) -> (*mut u8, usize) {
        match self {
            Self::Pc(mem) => mem.allocation_span(),
            Self::Flat(mem) => (mem.bytes.as_mut_ptr(), mem.bytes.len()),
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
            Self::Flat(_) => {}
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
            Self::Flat(mem) => {
                if (rw as u32 & 1) != 0 && policy.monitor_hit() {
                    return Ok(None);
                }
                Ok(checked_range(addr, 1, mem.bytes.len())
                    .map(|range| range.start..mem.bytes.len()))
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
            Self::Flat(mem) => Ok(mem.bytes.get_mut(range)),
        }
    }

    pub(crate) fn physical_access_len(&self, addr: BxPhyAddress, len: usize) -> usize {
        match self {
            Self::Pc(mem) => mem.mmio.access_len(mem.a20_addr(addr), len),
            Self::Flat(_) => len,
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
            Self::Flat(mem) => {
                if len > out.len() || len > 4096 - addr.page_offset() as usize {
                    return Err(MemoryError::ReadPhysicalPage { addr, len }.into());
                }
                for (offset, byte) in out[..len].iter_mut().enumerate() {
                    *byte = checked_range(addr.wrapping_add(offset as u64), 1, mem.bytes.len())
                        .map_or(0xFF, |range| mem.bytes[range.start]);
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
            Self::Flat(mem) => {
                if len > data.len() || len > 4096 - addr.page_offset() as usize {
                    return Err(MemoryError::WritePhysicalPage { addr, len }.into());
                }
                for (offset, byte) in data[..len].iter().enumerate() {
                    if let Some(range) =
                        checked_range(addr.wrapping_add(offset as u64), 1, mem.bytes.len())
                    {
                        mem.bytes[range.start] = *byte;
                    }
                }
                mark_write(mem.stamps, mem.writes, addr, len);
                Ok(PhysAccess::Done)
            }
        }
    }

    pub(crate) fn read_ram(&mut self, addr: BxPhyAddress, out: &mut [u8]) -> Result<usize> {
        match self {
            Self::Pc(mem) => mem.read_ram(addr, out),
            Self::Flat(mem) => {
                let range = checked_range(addr, out.len(), mem.bytes.len())
                    .ok_or(MemoryError::RamImageOutOfRange)?;
                out.copy_from_slice(&mem.bytes[range]);
                Ok(out.len())
            }
        }
    }

    /// Tracked host/DMA writer between interpreter runs in the same loan.
    pub(crate) fn write_ram(&mut self, addr: BxPhyAddress, data: &[u8]) -> Result<usize> {
        match self {
            Self::Pc(mem) => mem.write_ram(addr, data),
            Self::Flat(mem) => {
                let range = checked_range(addr, data.len(), mem.bytes.len())
                    .ok_or(MemoryError::RamImageOutOfRange)?;
                mem.bytes[range].copy_from_slice(data);
                mark_write(mem.stamps, mem.writes, addr, data.len());
                Ok(data.len())
            }
        }
    }

    pub(crate) fn smc_mark_icache_mask(&mut self, addr: BxPhyAddress, mask: u32) {
        match self {
            Self::Pc(mem) => mem.smc_mark_icache_mask(addr, mask),
            Self::Flat(mem) => {
                if let Some(stamp) = usize::try_from(addr.get() >> 12)
                    .ok()
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
            Self::Flat(mem) => usize::try_from(addr.get() >> 12)
                .ok()
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
    pub(crate) fn smc_dec_write_stamp(&mut self, addr: BxPhyAddress, len: u32) -> u64 {
        match self {
            Self::Pc(mem) => {
                mem.smc_dec_write_stamp(addr, len);
                mem.smc_seq_next()
            }
            Self::Flat(mem) => {
                if len as usize <= 4096 - addr.page_offset() as usize {
                    if let Ok(index) = usize::try_from(addr.get() >> 12) {
                        if index < mem.stamps.len() {
                            mem.writes.write(mem.stamps, index, addr, len);
                        }
                    }
                } else {
                    mark_write(mem.stamps, mem.writes, addr, len as usize);
                }
                mem.writes.next()
            }
        }
    }

    #[inline]
    pub(crate) fn smc_seq_next(&self) -> u64 {
        match self {
            Self::Pc(mem) => mem.smc_seq_next(),
            Self::Flat(mem) => mem.writes.next(),
        }
    }

    pub(crate) fn smc_pending_since(
        &self,
        since: u64,
    ) -> (bool, &[crate::cpu::icache::PendingSmc]) {
        match self {
            Self::Pc(mem) => mem.smc_pending_since(since),
            Self::Flat(mem) => mem.writes.since(since),
        }
    }
}
