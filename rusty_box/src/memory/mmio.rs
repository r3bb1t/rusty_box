//! MMIO (Memory-Mapped I/O) callback regions.
//!
//! When a physical address falls in a registered MMIO range, the
//! read/write is dispatched to a user callback instead of RAM.

#[cfg(feature = "alloc")]
use alloc::boxed::Box;

use crate::config::{BxPhyAddress, MAX_MMIO_REGIONS};
use crate::emulator::hardware::{IsaIrq, PcDevice, PcDeviceAttachRefusal, WindowOffset};

/// A stateful x86 PC MMIO device. Offsets are relative to the attached
/// page. Values are little-endian and accesses may be split into bytes.
pub trait MmioDevice: Send {
    fn read(&mut self, offset: u64, len: usize) -> u64;
    fn write(&mut self, offset: u64, len: usize, value: u64);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MmioAttachRefusal {
    #[error("MMIO device requires a page-aligned physical address")]
    Unaligned,
    #[error("MMIO device page overlaps an existing mapping")]
    Overlap,
    #[error("MMIO registry is full")]
    Full,
    #[error("MMIO device attachment requires a single x86 processor")]
    MultipleProcessors,
}

/// A failed attachment returns the still-owned device loan.
#[derive(thiserror::Error)]
#[error("{reason}")]
pub struct MmioAttachError {
    pub reason: MmioAttachRefusal,
    pub device: &'static mut dyn MmioDevice,
}

impl core::fmt::Debug for MmioAttachError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.reason.fmt(f)
    }
}

#[cfg(feature = "alloc")]
type ReadCallback = Box<dyn FnMut(BxPhyAddress, usize) -> u64 + Send>;
#[cfg(feature = "alloc")]
type WriteCallback = Box<dyn FnMut(BxPhyAddress, usize, u64) + Send>;
#[cfg(not(feature = "alloc"))]
type ReadCallback = fn(BxPhyAddress, usize) -> u64;
#[cfg(not(feature = "alloc"))]
type WriteCallback = fn(BxPhyAddress, usize, u64);

enum Target {
    Callbacks {
        read: ReadCallback,
        write: WriteCallback,
    },
    Device(&'static mut dyn MmioDevice),
    Composed(ComposedDevice),
}

pub(crate) struct ComposedDevice {
    pub(crate) line: IsaIrq,
    pub(crate) device: PcDevice,
}

/// All registry ranges, projected around the exclusively borrowed endpoint.
pub(crate) struct DmaExclusions<'a> {
    before: &'a [Option<MmioRegion>],
    page: BxPhyAddress,
    after: &'a [Option<MmioRegion>],
}

impl DmaExclusions<'_> {
    pub(crate) fn overlaps(&self, page: BxPhyAddress) -> bool {
        self.page == page
            || self
                .before
                .iter()
                .chain(self.after)
                .flatten()
                .any(|region| region.overlaps(page, 4096))
    }
}

/// A registered MMIO region: `size` bytes from `start`.
pub(crate) struct MmioRegion {
    pub(crate) start: BxPhyAddress,
    pub(crate) size: u64,
    target: Target,
}

impl MmioRegion {
    fn overlaps(&self, start: BxPhyAddress, size: u64) -> bool {
        size != 0
            && self.size != 0
            && (start.offset_from(self.start) < self.size || self.start.offset_from(start) < size)
    }

    pub(crate) fn read(&mut self, address: BxPhyAddress, len: usize) -> u64 {
        match &mut self.target {
            Target::Callbacks { read, .. } => read(address, len),
            Target::Device(device) => device.read(address - self.start, len),
            Target::Composed(attached) => attached
                .device
                .endpoint()
                .read(WindowOffset(address - self.start), len),
        }
    }

    /// True when this write was handled without machine capabilities.
    pub(crate) fn write_local(&mut self, address: BxPhyAddress, len: usize, value: u64) -> bool {
        match &mut self.target {
            Target::Callbacks { write, .. } => write(address, len, value),
            Target::Device(device) => device.write(address - self.start, len, value),
            Target::Composed(_) => return false,
        }
        true
    }
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

    /// Register an MMIO callback region. Device pages own their ranges
    /// exclusively, so a callback overlapping a device page is rejected.
    pub fn map(
        &mut self,
        addr: BxPhyAddress,
        size: u64,
        read_cb: ReadCallback,
        write_cb: WriteCallback,
    ) {
        assert!(
            !self
                .regions
                .iter()
                .flatten()
                .any(|region| !matches!(region.target, Target::Callbacks { .. })
                    && region.overlaps(addr, size)),
            "callback MMIO overlaps a device page"
        );
        let Some(slot) = self.regions.iter_mut().find(|slot| slot.is_none()) else {
            panic!("MMIO region overflow");
        };
        *slot = Some(MmioRegion {
            start: addr,
            size,
            target: Target::Callbacks {
                read: read_cb,
                write: write_cb,
            },
        });
        self.count += 1;
    }

    pub(crate) fn attach_device(
        &mut self,
        addr: BxPhyAddress,
        device: &'static mut dyn MmioDevice,
    ) -> Result<(), MmioAttachError> {
        match self.attach_page(addr, Target::Device(device)) {
            Ok(()) => Ok(()),
            Err((reason, Target::Device(device))) => Err(MmioAttachError { reason, device }),
            Err(_) => unreachable!(),
        }
    }

    fn attach_page(
        &mut self,
        addr: BxPhyAddress,
        target: Target,
    ) -> Result<(), (MmioAttachRefusal, Target)> {
        let slot = match self.vacant_page(addr) {
            Ok(slot) => slot,
            Err(reason) => return Err((reason, target)),
        };
        self.insert_page(slot, addr, target);
        Ok(())
    }

    fn vacant_page(&self, addr: BxPhyAddress) -> Result<usize, MmioAttachRefusal> {
        if addr.page_offset() != 0 {
            return Err(MmioAttachRefusal::Unaligned);
        }
        if self
            .regions
            .iter()
            .flatten()
            .any(|region| region.overlaps(addr, 4096))
        {
            return Err(MmioAttachRefusal::Overlap);
        }
        self.regions
            .iter()
            .position(Option::is_none)
            .ok_or(MmioAttachRefusal::Full)
    }

    fn insert_page(&mut self, index: usize, addr: BxPhyAddress, target: Target) {
        debug_assert!(self.regions[index].is_none());
        self.regions[index] = Some(MmioRegion {
            start: addr,
            size: 4096,
            target,
        });
        self.count += 1;
    }

    pub(crate) fn composed_slot(
        &self,
        addr: BxPhyAddress,
        line: IsaIrq,
    ) -> Result<usize, PcDeviceAttachRefusal> {
        if self.regions.iter().flatten().any(
            |region| matches!(&region.target, Target::Composed(attached) if attached.line == line),
        ) {
            return Err(PcDeviceAttachRefusal::IrqInUse);
        }
        Ok(self.vacant_page(addr)?)
    }

    pub(crate) fn insert_composed(
        &mut self,
        slot: usize,
        addr: BxPhyAddress,
        line: IsaIrq,
        device: PcDevice,
    ) {
        self.insert_page(
            slot,
            addr,
            Target::Composed(ComposedDevice { line, device }),
        );
    }

    pub(crate) fn detach_composed(&mut self, addr: BxPhyAddress) -> Option<ComposedDevice> {
        let slot = self.regions.iter_mut().find(|slot| {
            slot.as_ref().is_some_and(|region| {
                region.start == addr && matches!(region.target, Target::Composed(_))
            })
        })?;
        let Target::Composed(attached) = slot.take()?.target else {
            unreachable!()
        };
        self.count -= 1;
        Some(attached)
    }

    pub(crate) fn composed_at(
        &mut self,
        addr: BxPhyAddress,
    ) -> Option<(&mut ComposedDevice, DmaExclusions<'_>)> {
        let index = self.regions.iter().position(|slot| {
            slot.as_ref()
                .is_some_and(|region| addr.offset_from(region.start) < region.size)
        })?;
        let (before, tail) = self.regions.split_at_mut(index);
        let (current, after) = tail.split_first_mut()?;
        let region = current.as_mut()?;
        let Target::Composed(attached) = &mut region.target else {
            return None;
        };
        Some((
            attached,
            DmaExclusions {
                before,
                page: region.start,
                after,
            },
        ))
    }

    pub(crate) fn detach_device(
        &mut self,
        addr: BxPhyAddress,
    ) -> Option<&'static mut dyn MmioDevice> {
        let slot = self.regions.iter_mut().find(|slot| {
            slot.as_ref().is_some_and(|region| {
                region.start == addr && matches!(region.target, Target::Device(_))
            })
        })?;
        if let Target::Device(device) = slot.take()?.target {
            self.count -= 1;
            return Some(device);
        }
        None
    }

    /// Remove callback regions overlapping [addr, addr+size). Device pages
    /// are removed through their detach operation, which returns the loan.
    pub fn unmap(&mut self, addr: BxPhyAddress, size: u64) {
        for slot in &mut self.regions {
            if slot.as_ref().is_some_and(|region| {
                matches!(region.target, Target::Callbacks { .. }) && region.overlaps(addr, size)
            }) {
                *slot = None;
                self.count -= 1;
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

    pub(crate) fn has_composed(&self) -> bool {
        !self.is_empty()
            && self
                .regions
                .iter()
                .flatten()
                .any(|region| matches!(region.target, Target::Composed(_)))
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// RAM prefix before a callback region, including sub-page boundaries.
    pub(crate) fn direct_len(&self, addr: BxPhyAddress, len: usize) -> usize {
        if self.is_empty() {
            return len;
        }
        let mut available = len;
        for region in self.regions.iter().flatten() {
            if addr.offset_from(region.start) < region.size {
                return 0;
            }
            if region.size != 0 && region.start > addr {
                available =
                    available.min(usize::try_from(region.start - addr).unwrap_or(usize::MAX));
            }
        }
        available
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
