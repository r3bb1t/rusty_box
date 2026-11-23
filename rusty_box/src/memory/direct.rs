//! Independently owned direct RAM and ROM for CPU-only x86 execution.

use core::ops::Range;

use super::code_writes::CodeWrites;
use super::indirect::IndirectRamLoan;
use super::mapped::{LoanBacking, MemoryLoan, RegionKind};
use crate::config::BxPhyAddress;
use crate::cpu::icache::smc_cache_line_mask;

const PAGE: usize = 4096;
const INDEX_BITS: u32 = 4;
const OFFSET_BITS: u32 = usize::BITS - INDEX_BITS - 1;
const TAG: usize = 1 << (usize::BITS - 1);
const OFFSET_MASK: usize = (1 << OFFSET_BITS) - 1;
const MAX_REGIONS: usize = 1 << INDEX_BITS;

/// A direct physical interval. Bytes and one code stamp per page are lent
/// exclusively. The host address of either slice need not be page-aligned.
pub struct DirectRegion<'a> {
    pub guest_start: BxPhyAddress,
    pub bytes: &'a mut [u8],
    pub stamps: &'a mut [u32],
    pub kind: RegionKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DirectMapError {
    #[error("direct memory requires 1 to {max} regions")]
    RegionCount { max: usize },
    #[error(
        "direct region {index} must cover whole pages and fit in the address and backing limits"
    )]
    InvalidRegion { index: usize },
    #[error("direct region {index} is out of guest address order or overlaps another region")]
    GuestOrder { index: usize },
    #[error("direct region {index} requires {required} code stamps and has {provided}")]
    StampStorage {
        index: usize,
        required: usize,
        provided: usize,
    },
}

/// A physical map over independent slices with caller-supplied stamp storage.
///
/// Regions must be sorted and disjoint and cover whole guest pages. A session
/// exclusively borrows the descriptors and all their storage. RAM is readable,
/// writable and fetchable, ROM ignores guest writes, and holes read as 0xff.
/// CPU paging still controls linear access permissions.
///
/// ```compile_fail,E0499
/// use rusty_box::config::BxPhyAddress;
/// use rusty_box::memory::{direct::{DirectMemory, DirectRegion}, mapped::RegionKind};
/// let (mut bytes, mut stamps) = ([0; 4096], [0]);
/// let mut regions = [DirectRegion {
///     guest_start: BxPhyAddress::new(0), bytes: &mut bytes,
///     stamps: &mut stamps, kind: RegionKind::Ram,
/// }];
/// let mut memory = DirectMemory::new(&mut regions).unwrap();
/// let loan = memory.loan();
/// bytes[0] = 0xf4;
/// drop(loan);
/// ```
///
/// ```compile_fail,E0515
/// use rusty_box::config::BxPhyAddress;
/// use rusty_box::memory::{direct::{DirectMemory, DirectRegion}, mapped::{MemoryLoan, RegionKind}};
/// fn dangling<'a>(bytes: &'a mut [u8], stamps: &'a mut [u32]) -> MemoryLoan<'a> {
///     let mut regions = [DirectRegion {
///         guest_start: BxPhyAddress::new(0), bytes, stamps, kind: RegionKind::Ram,
///     }];
///     let mut memory = DirectMemory::new(&mut regions).unwrap();
///     memory.loan()
/// }
/// ```
pub struct DirectMemory<'map, 'bytes> {
    regions: &'map mut [DirectRegion<'bytes>],
    writes: CodeWrites,
}

impl<'map, 'bytes> DirectMemory<'map, 'bytes> {
    pub const MAX_REGIONS: usize = MAX_REGIONS;
    /// Largest supported slice, in bytes. Depends on the host pointer width.
    pub const MAX_REGION_LEN: usize = OFFSET_MASK & !(PAGE - 1);

    pub fn new(regions: &'map mut [DirectRegion<'bytes>]) -> Result<Self, DirectMapError> {
        if regions.is_empty() || regions.len() > Self::MAX_REGIONS {
            return Err(DirectMapError::RegionCount {
                max: Self::MAX_REGIONS,
            });
        }
        let mut guest_end = BxPhyAddress::new(0);
        for (index, region) in regions.iter().enumerate() {
            let len = region.bytes.len();
            let end = region
                .guest_start
                .checked_add(len as u64)
                .ok_or(DirectMapError::InvalidRegion { index })?;
            if len == 0
                || len > Self::MAX_REGION_LEN
                || len % PAGE != 0
                || region.guest_start.page_offset() != 0
            {
                return Err(DirectMapError::InvalidRegion { index });
            }
            if region.guest_start < guest_end {
                return Err(DirectMapError::GuestOrder { index });
            }
            let required = len / PAGE;
            if region.stamps.len() < required {
                return Err(DirectMapError::StampStorage {
                    index,
                    required,
                    provided: region.stamps.len(),
                });
            }
            guest_end = end;
        }
        for region in &mut *regions {
            region.stamps[..region.bytes.len() / PAGE].fill(0);
        }
        Ok(Self {
            regions,
            writes: CodeWrites::new(),
        })
    }

    /// Bind this map to one CPU session. A new bind discards old CPU backing
    /// cache entries, even if the allocator reused the same host addresses.
    pub fn loan(&mut self) -> MemoryLoan<'_> {
        let count = self.regions.len();
        MemoryLoan(LoanBacking::Direct(DirectParts {
            regions: lend_regions(self.regions.iter_mut()),
            count,
            writes: &mut self.writes,
            indirect: None,
        }))
    }
}

/// A cached byte coordinate in a particular region of the current loan.
/// Offsets of single allocations fit below TAG (the Rust slice size bound).
/// The end of a region also fits below the next index, so bounded fetch ranges
/// can be clamped without changing their region.
#[derive(Clone, Copy)]
pub(crate) struct DirectOffset(usize);

impl DirectOffset {
    fn new(index: usize, local: usize) -> Self {
        Self(TAG | (index << OFFSET_BITS) | local)
    }

    #[inline(always)]
    pub(crate) fn decode(offset: usize) -> Option<Self> {
        (offset & TAG != 0).then_some(Self(offset))
    }

    fn index(self) -> usize {
        (self.0 & !TAG) >> OFFSET_BITS
    }

    fn local(self) -> usize {
        self.0 & OFFSET_MASK
    }
}

pub(crate) struct DirectParts<'a> {
    regions: [Option<DirectRegion<'a>>; MAX_REGIONS],
    count: usize,
    pub(super) writes: &'a mut CodeWrites,
    pub(super) indirect: Option<IndirectRamLoan<'a>>,
}

fn lend_regions<'a, 'bytes: 'a>(
    mut regions: impl Iterator<Item = &'a mut DirectRegion<'bytes>>,
) -> [Option<DirectRegion<'a>>; MAX_REGIONS] {
    // Reborrow the slice fields individually. A mutable view of the original
    // descriptor slice would permit replacing a field with a shorter loan.
    core::array::from_fn(|_| {
        regions.next().map(|region| DirectRegion {
            guest_start: region.guest_start,
            bytes: region.bytes,
            stamps: region.stamps,
            kind: region.kind,
        })
    })
}

impl DirectParts<'_> {
    pub(super) fn reborrow(&mut self) -> DirectParts<'_> {
        DirectParts {
            regions: lend_regions(self.regions.iter_mut().flatten()),
            count: self.count,
            writes: self.writes,
            indirect: self.indirect.as_mut().map(IndirectRamLoan::reborrow),
        }
    }

    pub(super) fn overlaps(&self, start: BxPhyAddress, end: BxPhyAddress) -> bool {
        self.regions.iter().flatten().any(|region| {
            region.guest_start < end
                && start < region.guest_start.wrapping_add(region.bytes.len() as u64)
        })
    }

    fn locate(&self, addr: BxPhyAddress, writable: bool) -> Option<(DirectOffset, usize)> {
        let index = self.regions[..self.count]
            .partition_point(|region| region.as_ref().is_some_and(|r| r.guest_start <= addr))
            .checked_sub(1)?;
        let region = self.regions[index].as_ref()?;
        let local = usize::try_from(addr - region.guest_start).ok()?;
        if local >= region.bytes.len() || (writable && region.kind == RegionKind::Rom) {
            return None;
        }
        Some((DirectOffset::new(index, local), region.bytes.len() - local))
    }

    pub(super) fn span_from(&self, addr: BxPhyAddress, writable: bool) -> Option<Range<usize>> {
        let (offset, remaining) = self.locate(addr, writable)?;
        Some(offset.0..offset.0 + remaining)
    }

    pub(super) fn has_backing_at(&self, addr: BxPhyAddress) -> bool {
        self.locate(addr, false).is_some()
            || self.indirect.as_ref().is_some_and(|ram| ram.contains(addr))
    }

    pub(super) fn bytes_at(&mut self, addr: BxPhyAddress, writable: bool) -> Option<&mut [u8]> {
        let (offset, _) = self.locate(addr, writable)?;
        self.regions[offset.index()]
            .as_mut()?
            .bytes
            .get_mut(offset.local()..)
    }

    pub(super) fn read(&mut self, addr: BxPhyAddress, out: &mut [u8]) -> bool {
        if let Some(bytes) = self
            .bytes_at(addr, false)
            .and_then(|bytes| bytes.get(..out.len()))
        {
            out.copy_from_slice(bytes);
            true
        } else {
            IndirectRamLoan::read(self.indirect.as_mut(), addr, out)
        }
    }

    pub(super) fn write(&mut self, addr: BxPhyAddress, data: &[u8]) -> bool {
        if let Some(bytes) = self
            .bytes_at(addr, true)
            .and_then(|bytes| bytes.get_mut(..data.len()))
        {
            bytes.copy_from_slice(data);
            self.mark_write(addr, data.len());
            true
        } else {
            IndirectRamLoan::write(self.indirect.as_mut(), addr, data)
        }
    }

    pub(super) fn fetch_ptr(&self, offset: DirectOffset) -> *const u8 {
        let Some(region) = &self.regions[offset.index()] else {
            unreachable!("cached fetch coordinate must name a region in this loan");
        };
        region.bytes.as_ptr().wrapping_add(offset.local())
    }

    pub(super) fn ram_ptr(&mut self, offset: DirectOffset) -> *mut u8 {
        let Some(region) = &mut self.regions[offset.index()] else {
            unreachable!("cached RAM coordinate must name a region in this loan");
        };
        region.bytes.as_mut_ptr().wrapping_add(offset.local())
    }

    pub(super) fn mark_icache(&mut self, addr: BxPhyAddress, mask: u32) {
        let Some((offset, _)) = self.locate(addr, false) else {
            return;
        };
        if let Some(region) = &mut self.regions[offset.index()] {
            region.stamps[offset.local() / PAGE] |= mask;
        }
    }

    pub(super) fn has_stamps(&self, addr: BxPhyAddress, len: u32) -> bool {
        let Some((offset, _)) = self.locate(addr, false) else {
            return false;
        };
        self.regions[offset.index()].as_ref().is_some_and(|region| {
            region.stamps[offset.local() / PAGE] & smc_cache_line_mask(addr, len) != 0
        })
    }

    pub(super) fn mark_write(&mut self, mut addr: BxPhyAddress, mut len: usize) {
        while len != 0 {
            let count = len.min(PAGE - addr.page_offset() as usize);
            if let Some((offset, _)) = self.locate(addr, true) {
                self.mark_cached_write(offset, addr, count as u32);
            }
            addr = addr.wrapping_add(count as u64);
            len -= count;
        }
    }

    pub(super) fn mark_cached_write(&mut self, offset: DirectOffset, addr: BxPhyAddress, len: u32) {
        if let Some(region) = &mut self.regions[offset.index()] {
            self.writes
                .write(region.stamps, offset.local() / PAGE, addr, len);
        }
    }
}
