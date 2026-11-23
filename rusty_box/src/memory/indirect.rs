//! An extra RAM range in a CPU-only physical address space.

use crate::config::BxPhyAddress;

/// Independently stored, byte-addressable RAM. The engine passes offsets
/// relative to the attached range. Reads fill the entire destination, and
/// writes update the entire source range. Both operations are infallible.
/// Reads leave guest contents unchanged. The emulator chooses transfer
/// widths and may split transfers, including accesses made by the MMU.
/// Checked host transfers can span more than one page of this range.
pub trait IndirectRam {
    fn read(&mut self, offset: u64, out: &mut [u8]);
    fn write(&mut self, offset: u64, data: &[u8]);
}

/// A nonempty physical interval aligned to 4 KiB at both ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IndirectRamRange {
    pub start: BxPhyAddress,
    pub len: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum IndirectRamError {
    #[error("indirect RAM must cover whole pages and fit in the physical address type")]
    InvalidRange,
    #[error("indirect RAM overlaps an existing RAM or ROM range")]
    Overlap,
    #[error("this memory loan already carries indirect RAM")]
    AlreadyAttached,
}

/// R8 erasure only on the opted-in slow RAM route. The provider lends no
/// pointers into its representation and cannot back a direct cache entry.
pub(super) struct IndirectRamLoan<'a> {
    pub(super) start: BxPhyAddress,
    pub(super) end: BxPhyAddress,
    pub(super) ram: &'a mut (dyn IndirectRam + Send + Sync),
}

// A shared loan exposes range information. Provider access always needs an
// exclusive borrow, so erasure preserves the original loan's shared unwind trait.
impl core::panic::RefUnwindSafe for IndirectRamLoan<'_> {}

impl IndirectRamLoan<'_> {
    pub(super) fn reborrow(&mut self) -> IndirectRamLoan<'_> {
        IndirectRamLoan {
            start: self.start,
            end: self.end,
            ram: self.ram,
        }
    }

    pub(super) fn contains(&self, address: BxPhyAddress) -> bool {
        self.start <= address && address < self.end
    }

    pub(super) fn offset(&self, address: BxPhyAddress, len: usize) -> Option<u64> {
        if !self.contains(address) || u64::try_from(len).ok()? > self.end - address {
            return None;
        }
        Some(address - self.start)
    }
}
