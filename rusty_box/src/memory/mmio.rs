//! MMIO (Memory-Mapped I/O) callback regions.
//!
//! When a physical address falls in a registered MMIO range, the
//! read/write is dispatched to a user callback instead of RAM.

#[cfg(feature = "alloc")]
use alloc::boxed::Box;

use crate::config::{BxPhyAddress, MAX_MMIO_REGIONS};

/// A registered MMIO region: `size` bytes from `start`.
pub(crate) struct MmioRegion {
    pub(crate) start: BxPhyAddress,
    pub(crate) size: u64,
    #[cfg(feature = "alloc")]
    pub(crate) read_cb: Box<dyn FnMut(BxPhyAddress, usize) -> u64 + Send>,
    #[cfg(not(feature = "alloc"))]
    pub(crate) read_cb: fn(BxPhyAddress, usize) -> u64,
    #[cfg(feature = "alloc")]
    pub(crate) write_cb: Box<dyn FnMut(BxPhyAddress, usize, u64) + Send>,
    #[cfg(not(feature = "alloc"))]
    pub(crate) write_cb: fn(BxPhyAddress, usize, u64),
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
        addr: BxPhyAddress,
        size: u64,
        read_cb: Box<dyn FnMut(BxPhyAddress, usize) -> u64 + Send>,
        write_cb: Box<dyn FnMut(BxPhyAddress, usize, u64) + Send>,
    ) {
        assert!(self.count < MAX_MMIO_REGIONS, "MMIO region overflow");
        // Find first empty slot
        for slot in self.regions.iter_mut() {
            if slot.is_none() {
                *slot = Some(MmioRegion {
                    start: addr,
                    size,
                    read_cb,
                    write_cb,
                });
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
        addr: BxPhyAddress,
        size: u64,
        read_cb: fn(BxPhyAddress, usize) -> u64,
        write_cb: fn(BxPhyAddress, usize, u64),
    ) {
        assert!(self.count < MAX_MMIO_REGIONS, "MMIO region overflow");
        for slot in self.regions.iter_mut() {
            if slot.is_none() {
                *slot = Some(MmioRegion {
                    start: addr,
                    size,
                    read_cb,
                    write_cb,
                });
                self.count += 1;
                return;
            }
        }
        unreachable!("count < MAX but no empty slot");
    }

    /// Remove all MMIO regions overlapping [addr, addr+size).
    pub fn unmap(&mut self, addr: BxPhyAddress, size: u64) {
        for slot in self.regions.iter_mut() {
            if let Some(r) = slot {
                // An empty range overlaps nothing; two others overlap where
                // one starts inside the other.
                let overlaps = size != 0
                    && (addr.offset_from(r.start) < r.size || r.start.offset_from(addr) < size);
                if overlaps {
                    *slot = None;
                    self.count -= 1;
                }
            }
        }
    }

    /// Find region containing `addr`. Returns mutable ref for dispatch.
    #[inline]
    pub(crate) fn find_mut(&mut self, addr: BxPhyAddress) -> Option<&mut MmioRegion> {
        self.regions
            .iter_mut()
            .filter_map(|s| s.as_mut())
            .find(|r| addr.offset_from(r.start) < r.size)
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

impl Default for MmioRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;

    fn registry_with(start: u64, size: u64) -> MmioRegistry {
        let mut registry = MmioRegistry::new();
        registry.map(
            BxPhyAddress::new(start),
            size,
            Box::new(|_, _| 0),
            Box::new(|_, _, _| {}),
        );
        registry
    }

    #[test]
    fn a_region_holds_its_first_byte_and_not_its_end() {
        let mut registry = registry_with(0xF000_0000, 0x1000);
        assert!(registry.find_mut(BxPhyAddress::new(0xEFFF_FFFF)).is_none());
        assert!(registry.find_mut(BxPhyAddress::new(0xF000_0000)).is_some());
        assert!(registry.find_mut(BxPhyAddress::new(0xF000_0FFF)).is_some());
        assert!(registry.find_mut(BxPhyAddress::new(0xF000_1000)).is_none());
    }

    #[test]
    fn a_region_may_end_at_the_top_of_the_address_space() {
        let mut registry = registry_with(u64::MAX - 0xFFF, 0x1000);
        assert!(registry.find_mut(BxPhyAddress::new(u64::MAX)).is_some());
        assert!(registry.find_mut(BxPhyAddress::new(0)).is_none());
        registry.unmap(BxPhyAddress::new(u64::MAX), 1);
        assert!(registry.is_empty());
    }

    #[test]
    fn unmap_removes_only_regions_its_range_overlaps() {
        let mut registry = registry_with(0x1000, 0x1000);
        registry.unmap(BxPhyAddress::new(0x2000), 0x1000);
        registry.unmap(BxPhyAddress::new(0x0), 0x1000);
        registry.unmap(BxPhyAddress::new(0x1800), 0);
        assert!(
            !registry.is_empty(),
            "adjacent and empty ranges overlap nothing"
        );
        registry.unmap(BxPhyAddress::new(0x0), 0x1001);
        assert!(
            registry.is_empty(),
            "a range reaching its first byte overlaps it"
        );

        let mut registry = registry_with(0x1000, 0x1000);
        registry.unmap(BxPhyAddress::new(0x1FFF), 0x10);
        assert!(
            registry.is_empty(),
            "a range starting at its last byte overlaps it"
        );
    }
}
