//! MMIO (Memory-Mapped I/O) callback regions.
//!
//! When a physical address falls in a registered MMIO range, the
//! read/write is dispatched to a user callback instead of RAM.

#[cfg(feature = "alloc")]
use alloc::boxed::Box;

use crate::config::MAX_MMIO_REGIONS;

/// A registered MMIO region.
pub(crate) struct MmioRegion {
    pub(crate) start: u64,
    pub(crate) size: u64,
    #[cfg(feature = "alloc")]
    pub(crate) read_cb: Box<dyn FnMut(u64, usize) -> u64 + Send>,
    #[cfg(not(feature = "alloc"))]
    pub(crate) read_cb: fn(u64, usize) -> u64,
    #[cfg(feature = "alloc")]
    pub(crate) write_cb: Box<dyn FnMut(u64, usize, u64) + Send>,
    #[cfg(not(feature = "alloc"))]
    pub(crate) write_cb: fn(u64, usize, u64),
}

/// Registry of MMIO regions, checked on physical memory access.
pub struct MmioRegistry {
    pub(crate) regions: [Option<MmioRegion>; MAX_MMIO_REGIONS],
    count: usize,
}

impl MmioRegistry {
    pub fn new() -> Self {
        Self {
            regions: core::array::from_fn(|_| None),
            count: 0,
        }
    }

    /// Register an MMIO region. Reads and writes to [addr, addr+size) go
    /// to callbacks instead of RAM.
    #[cfg(feature = "alloc")]
    pub fn map(
        &mut self,
        addr: u64,
        size: u64,
        read_cb: Box<dyn FnMut(u64, usize) -> u64 + Send>,
        write_cb: Box<dyn FnMut(u64, usize, u64) + Send>,
    ) {
        assert!(self.count < MAX_MMIO_REGIONS, "MMIO region overflow");
        // Find first empty slot
        for slot in self.regions.iter_mut() {
            if slot.is_none() {
                *slot = Some(MmioRegion { start: addr, size, read_cb, write_cb });
                self.count += 1;
                return;
            }
        }
        unreachable!("count < MAX but no empty slot");
    }

    /// Register an MMIO region with function-pointer callbacks (no-alloc path).
    #[cfg(not(feature = "alloc"))]
    pub fn map(
        &mut self,
        addr: u64,
        size: u64,
        read_cb: fn(u64, usize) -> u64,
        write_cb: fn(u64, usize, u64),
    ) {
        assert!(self.count < MAX_MMIO_REGIONS, "MMIO region overflow");
        for slot in self.regions.iter_mut() {
            if slot.is_none() {
                *slot = Some(MmioRegion { start: addr, size, read_cb, write_cb });
                self.count += 1;
                return;
            }
        }
        unreachable!("count < MAX but no empty slot");
    }

    /// Remove all MMIO regions overlapping [addr, addr+size).
    pub fn unmap(&mut self, addr: u64, size: u64) {
        let q_end = addr + size;
        for slot in self.regions.iter_mut() {
            if let Some(r) = slot {
                let r_end = r.start + r.size;
                // Remove if overlapping
                if !(r_end <= addr || r.start >= q_end) {
                    *slot = None;
                    self.count -= 1;
                }
            }
        }
    }

    /// Find region containing `addr`. Returns mutable ref for dispatch.
    #[inline]
    pub(crate) fn find_mut(&mut self, addr: u64) -> Option<&mut MmioRegion> {
        self.regions.iter_mut().filter_map(|s| s.as_mut()).find(|r| {
            addr >= r.start && addr < r.start + r.size
        })
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

impl Default for MmioRegistry {
    fn default() -> Self { Self::new() }
}
