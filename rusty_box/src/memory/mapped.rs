//! Borrowed physical memory for CPU-only x86 execution.
//!
//! The provider decides where its bytes appear. The engine borrows the map,
//! backing and code stamps together, and validates offsets before using them.
//! Maps are fixed for the duration of a loan.

use core::ops::Range;

use super::flat::{FlatMemory, FlatMemoryError, FlatParts};
use crate::config::BxPhyAddress;

/// The guest's write policy for a physical window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegionKind {
    Ram,
    /// Reads and instruction fetch see the backing. Guest stores are ignored.
    Rom,
}

/// Map page-aligned guest addresses into a page-aligned offset of an arena.
/// The arena's host address itself need not be aligned.
#[derive(Clone, Copy, Debug)]
pub struct PhysicalRegion {
    pub guest_start: BxPhyAddress,
    pub arena_offset: usize,
    pub len: usize,
    pub kind: RegionKind,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MemoryMapError {
    #[error("physical region {index} is empty, unaligned or out of bounds")]
    InvalidRegion { index: usize },
    #[error("physical region {index} is out of guest address order or overlaps another region")]
    GuestOrder { index: usize },
    #[error("physical region {index} aliases backing from an earlier region")]
    BackingOverlap { index: usize },
    #[error(transparent)]
    StampStorage(#[from] FlatMemoryError),
}

/// A sparse physical map over a packed, exclusively borrowed arena.
///
/// Regions must be sorted by guest start and disjoint in both guest and arena
/// coordinates. The map takes no ownership of the arena and requires no
/// allocation. Data reads from holes return 0xff for each missing byte and
/// writes to holes have no effect. Fetch requires a RAM or ROM mapping.
pub struct MappedMemory<'a> {
    arena: FlatMemory<'a>,
    regions: &'a [PhysicalRegion],
}

impl<'a> MappedMemory<'a> {
    pub fn new(
        arena: &'a mut [u8],
        stamps: &'a mut [u32],
        regions: &'a [PhysicalRegion],
    ) -> Result<Self, MemoryMapError> {
        let mut guest_end = BxPhyAddress::new(0);
        for (index, region) in regions.iter().enumerate() {
            let Some(arena_end) = region.arena_offset.checked_add(region.len) else {
                return Err(MemoryMapError::InvalidRegion { index });
            };
            let Some(end) = u64::try_from(region.len)
                .ok()
                .and_then(|len| region.guest_start.checked_add(len))
            else {
                return Err(MemoryMapError::InvalidRegion { index });
            };
            if region.len == 0
                || region.len % 4096 != 0
                || region.arena_offset % 4096 != 0
                || region.guest_start.page_offset() != 0
                || arena_end > arena.len()
            {
                return Err(MemoryMapError::InvalidRegion { index });
            }
            if region.guest_start < guest_end {
                return Err(MemoryMapError::GuestOrder { index });
            }
            // Earlier regions have already passed the checked-end test.
            if regions[..index].iter().any(|previous| {
                previous.arena_offset < arena_end
                    && region.arena_offset < previous.arena_offset + previous.len
            }) {
                return Err(MemoryMapError::BackingOverlap { index });
            }
            guest_end = end;
        }
        Ok(Self {
            arena: FlatMemory::new(arena, stamps)?,
            regions,
        })
    }

    /// Lend this particular map to the engine. The provider may replace it
    /// or update the arena after execution returns the exclusive borrow.
    pub fn loan(&mut self) -> MemoryLoan<'_> {
        let mut parts = self.arena.parts();
        parts.regions = Some(self.regions);
        MemoryLoan(parts)
    }
}

/// Checked storage borrowed for one execution session. Created by
/// [`MappedMemory::loan`] or [`FlatMemory::loan`].
pub struct MemoryLoan<'a>(pub(crate) FlatParts<'a>);

/// Resolve the remaining contiguous bytes in one validated physical window.
pub(super) fn mapped_span(
    regions: &[PhysicalRegion],
    addr: BxPhyAddress,
    writable: bool,
) -> Option<Range<usize>> {
    let index = regions.partition_point(|region| region.guest_start <= addr);
    let region = regions.get(index.checked_sub(1)?)?;
    let offset = usize::try_from(addr - region.guest_start).ok()?;
    if offset >= region.len || (writable && region.kind == RegionKind::Rom) {
        return None;
    }
    Some(region.arena_offset + offset..region.arena_offset + region.len)
}
