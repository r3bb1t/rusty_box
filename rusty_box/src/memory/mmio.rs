//! MMIO (Memory-Mapped I/O) callback regions.
//!
//! When a physical address falls in a registered MMIO range, the
//! read/write is dispatched to a user callback instead of RAM.

use alloc::{boxed::Box, vec::Vec};

/// A registered MMIO region.
pub(crate) struct MmioRegion {
    pub(crate) start: u64,
    pub(crate) size: u64,
    pub(crate) read_cb: Box<dyn FnMut(u64, usize) -> u64 + Send>,
    pub(crate) write_cb: Box<dyn FnMut(u64, usize, u64) + Send>,
}

/// Registry of MMIO regions, checked on physical memory access.
pub struct MmioRegistry {
    pub(crate) regions: Vec<MmioRegion>,
}

impl MmioRegistry {
    pub fn new() -> Self {
        Self { regions: Vec::new() }
    }

    /// Register an MMIO region. Reads and writes to [addr, addr+size) go
    /// to callbacks instead of RAM.
    pub fn map(
        &mut self,
        addr: u64,
        size: u64,
        read_cb: Box<dyn FnMut(u64, usize) -> u64 + Send>,
        write_cb: Box<dyn FnMut(u64, usize, u64) + Send>,
    ) {
        self.regions.push(MmioRegion { start: addr, size, read_cb, write_cb });
    }

    /// Remove all MMIO regions overlapping [addr, addr+size).
    pub fn unmap(&mut self, addr: u64, size: u64) {
        self.regions.retain(|r| {
            let r_end = r.start + r.size;
            let q_end = addr + size;
            // Keep if no overlap
            r_end <= addr || r.start >= q_end
        });
    }

    /// Find region containing `addr`. Returns mutable ref for dispatch.
    #[inline]
    pub(crate) fn find_mut(&mut self, addr: u64) -> Option<&mut MmioRegion> {
        self.regions.iter_mut().find(|r| addr >= r.start && addr < r.start + r.size)
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
}

impl Default for MmioRegistry {
    fn default() -> Self { Self::new() }
}
