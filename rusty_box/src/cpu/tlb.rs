#![allow(private_interfaces, unused_assignments, dead_code)]

use crate::config::{BxAddress, BxPhyAddress, BxPtrEquiv};

pub type BxMemType = u32;

pub type BxHostpageaddr = BxPtrEquiv;

pub const LPF_MASK: BxAddress = 0xfffffffffffff000u64;

const PPF_MASK: BxPhyAddress = 0xfffffffffffff000u64;

const TLB_GLOBAL_PAGE: u32 = 0x80000000;

const BX_INVALID_TLB_ENTRY: u64 = 0xffffffffffffffffu64;

/// What a TLB entry caches so a hit can be served without re-resolving the
/// mapping.
///
/// The two TLBs cache different things, because they map different memory.
/// The data TLB only ever caches identity-backed guest RAM, so it can name a
/// page by its number within the RAM allocation ([`RamPage`]) — plain data
/// that stays valid however the allocation is reached. Instruction fetch also
/// runs out of ROM, which is not guest RAM and has no RAM page number, so the
/// instruction TLB names pages within the whole allocation ([`AllocPage`]).
/// Both answer the same question — where in host memory does this page live —
/// so the fast paths are written once, here.
pub(crate) trait CachedHostPage: Copy {
    /// Host address of this page. `base` is the start of the region whose
    /// pages this implementation numbers.
    fn host_addr(self, base: *mut u8) -> BxHostpageaddr;

    /// Offset of this page from the start of the whole memory allocation.
    ///
    /// `region_offset` is where the region this implementation numbers begins
    /// within that allocation, so each implementation only has to know its own
    /// origin. The eviction sidecar speaks exclusively in these, which is what
    /// lets one comparison cover both TLBs even though they measure from
    /// different places.
    fn alloc_offset_from(self, region_offset: usize) -> usize;
}

/// A page of guest RAM named by where it starts *within the RAM allocation*,
/// not by a host address.
///
/// An offset survives everything an address does not: it does not depend on
/// where the allocation sits, so it cannot be left dangling by a move and
/// carries nothing that would keep the CPU from being `Send`.
///
/// Held as a byte offset biased by one, so `Option<RamPage>` costs no more
/// than the bare integer — offset 0 is the legitimate first page of RAM and so
/// cannot itself serve as the niche. Byte offset rather than page number
/// deliberately: the entry is the same size either way (alignment padding
/// absorbs the four bytes a page number would save), and a byte offset lets a
/// hit fold the whole reconstruction, bias included, into one address
/// computation instead of a shift and an add. The bias lives only in the two
/// methods below.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct RamPage(core::num::NonZeroUsize);

impl RamPage {
    /// The page beginning `offset` bytes into the RAM allocation.
    ///
    /// `None` when `offset` does not begin a page, or is so large that biasing
    /// it wraps. Both answers mean "no direct access", so the caller takes the
    /// slow path — correct either way.
    #[inline(always)]
    pub(crate) fn from_ram_offset(offset: usize) -> Option<Self> {
        if (offset & 0xFFF) != 0 {
            return None;
        }
        core::num::NonZeroUsize::new(offset.wrapping_add(1)).map(Self)
    }

    /// Byte offset of this page into the RAM allocation.
    #[inline(always)]
    pub(crate) fn ram_offset(self) -> usize {
        self.0.get() - 1
    }
}

impl CachedHostPage for RamPage {
    #[inline(always)]
    fn host_addr(self, base: *mut u8) -> BxHostpageaddr {
        (base as BxHostpageaddr).wrapping_add(self.ram_offset() as BxHostpageaddr)
    }

    #[inline(always)]
    fn alloc_offset_from(self, region_offset: usize) -> usize {
        // Identity guest RAM begins `region_offset` bytes into the allocation.
        region_offset.wrapping_add(self.ram_offset())
    }
}

/// A page of directly-fetchable memory, named by where it starts within the
/// whole memory allocation.
///
/// Deliberately a different type from [`RamPage`], measured from a different
/// base. Instruction fetch also runs out of the ROM image and the bogus page,
/// which live in the same allocation as guest RAM but *past* it, and out of
/// relocated RAM slots when residency is partial — none of which identity
/// guest RAM can name. The two bases happen to be equal whenever residency is
/// full, so nothing but the type system would catch a mix-up: it would pass
/// every test and corrupt every ROM fetch the moment host memory is smaller
/// than guest memory.
///
/// Biased by one for the same reason `RamPage` is — offset 0 is the first
/// legitimate byte of the allocation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct AllocPage(core::num::NonZeroUsize);

impl AllocPage {
    /// The page beginning `offset` bytes into the memory allocation.
    ///
    /// Unlike `RamPage` this does not require page alignment: it records where
    /// a fetch window starts, and a window handed out near the end of a region
    /// legitimately starts mid-page.
    #[inline(always)]
    pub(crate) fn from_alloc_offset(offset: usize) -> Option<Self> {
        core::num::NonZeroUsize::new(offset.wrapping_add(1)).map(Self)
    }

    /// Byte offset of this page into the memory allocation.
    #[inline(always)]
    pub(crate) fn alloc_offset(self) -> usize {
        self.0.get() - 1
    }
}

impl CachedHostPage for AllocPage {
    #[inline(always)]
    fn host_addr(self, base: *mut u8) -> BxHostpageaddr {
        (base as BxHostpageaddr).wrapping_add(self.alloc_offset() as BxHostpageaddr)
    }

    #[inline(always)]
    fn alloc_offset_from(self, _region_offset: usize) -> usize {
        // Already measured from the allocation — its region IS the allocation.
        self.alloc_offset()
    }
}

/// No direct host mapping — take the slow path.
pub(crate) const NO_DIRECT_ACCESS: Option<RamPage> = None;

/// The raw bits of a mapping, or 0 when absent — for the paths that OR in a
/// page offset before turning the result into a pointer.
#[inline(always)]
pub(crate) fn host_page_addr_bits<P: CachedHostPage>(
    base: *mut u8,
    page: Option<P>,
) -> BxHostpageaddr {
    match page {
        Some(p) => p.host_addr(base),
        None => 0,
    }
}

/// The host pointer for a mapping the caller has already established is
/// present. `None` yields a null pointer, which every caller has guarded
/// against with `is_some()` — the same contract as the old zero sentinel,
/// now stated where it cannot be skipped by accident.
#[inline(always)]
pub(crate) fn host_page_ptr<P: CachedHostPage>(base: *mut u8, page: Option<P>) -> *mut u8 {
    match page {
        Some(p) => p.host_addr(base) as *mut u8,
        None => core::ptr::null_mut(),
    }
}

/// Absence must cost nothing: each cached form reserves a niche, so `None` is
/// the all-zero pattern and the entry is the size it would be as a bare
/// integer. A regression here would grow every one of the 3072 TLB entries
/// per CPU.
const _: () =
    assert!(core::mem::size_of::<Option<AllocPage>>() == core::mem::size_of::<usize>());
const _: () =
    assert!(core::mem::size_of::<Option<RamPage>>() == core::mem::size_of::<usize>());

pub(crate) struct TLBEntry<P> {
    /// linear page frame
    pub(crate) lpf: BxAddress,
    // physical page frame
    pub(crate) ppf: BxPhyAddress,
    /// Where the page backing this entry lives in host memory, or `None` when
    /// the page must take the slow path — it carries MMIO handlers, is ROM, or
    /// falls outside guest RAM.
    ///
    /// Bochs stores `hostPageAddr` here for the same reason: a hit can then be
    /// served without re-resolving the mapping. It spells "no mapping" as a
    /// zero address; `Option` says the same thing in the type, and niche
    /// optimisation makes it the same byte — see the size assertions above,
    /// which are what keep this free.
    pub(crate) host_page: Option<P>,
    pub(crate) access_bits: u32,
    pub(super) pkey: u32,
    // linear address mask of the page size
    pub(crate) lpf_mask: u32,
    pub(super) memtype: MemType, // (note from bochs)  // keep it Bit32u for alignment
}

#[derive(Default, Copy, Clone)]
enum MemType {
    #[default]
    UC = 0,
    WC = 1,
    Reserved2 = 2,
    Reserved3 = 3,
    WT = 4,
    WP = 5,
    WB = 6,
    UcWeak = 7, // PAT only
    Invalid = 8,
}

impl<P> TLBEntry<P> {
    /// An invalid entry — `lpf` carries the sentinel every lookup compares
    /// against. A `const` so a whole TLB can be built without running code.
    const INVALID: Self = Self {
        lpf: BX_INVALID_TLB_ENTRY,
        ppf: 0,
        host_page: None,
        access_bits: 0,
        pkey: 0,
        lpf_mask: 0,
        memtype: MemType::UC,
    };

    fn valid(&self) -> bool {
        self.lpf != BX_INVALID_TLB_ENTRY
    }

    fn invalidate(&mut self) {
        self.lpf = BX_INVALID_TLB_ENTRY;
        self.access_bits = 0
    }

    fn get_memtype(&self) -> MemType {
        {
            self.memtype
        }
    }

    /// Page can be read from the given privilege level.
    /// Bochs tlb.h `isReadOK`: `accessBits & (0x01 << user) & rd_pkey[pkey]`.
    /// The protection-key allow-mask is AND-ed in on EVERY hit, not just on
    /// the walk — callers built without PKEY support pass `u32::MAX`.
    #[inline]
    pub(crate) fn is_read_ok(&self, user: u32, pkey_mask: u32) -> bool {
        (self.access_bits & (0x01u32 << user) & pkey_mask) != 0
    }

    /// Page can be written from the given privilege level.
    /// Bochs tlb.h `isWriteOK`: `accessBits & (0x04 << user) & wr_pkey[pkey]`.
    #[inline]
    pub(crate) fn is_write_ok(&self, user: u32, pkey_mask: u32) -> bool {
        (self.access_bits & (0x04u32 << user) & pkey_mask) != 0
    }

    /// CET: page can be read as shadow stack from the given privilege level.
    /// Bochs tlb.h isShadowStackReadOK macro. With protection keys enabled
    /// (BX_SUPPORT_PKEYS), the entry's PKEY allow-mask (passed as `pkey_mask`)
    /// is AND-ed in; callers without PKEY support pass `u32::MAX`.
    /// `user` must be 0 (supervisor) or 1 (user) — used as a shift amount.
    #[inline]
    pub(crate) fn is_shadow_stack_read_ok(&self, user: u32, pkey_mask: u32) -> bool {
        (self.access_bits & (0x10u32 << user) & pkey_mask) != 0
    }

    /// CET: page can be written as shadow stack from the given privilege level.
    /// Bochs tlb.h isShadowStackWriteOK macro.
    #[inline]
    pub(crate) fn is_shadow_stack_write_ok(&self, user: u32, pkey_mask: u32) -> bool {
        (self.access_bits & (0x40u32 << user) & pkey_mask) != 0
    }
}

// Our TLB struct, generic over what its entries cache and how many there are:
pub struct Tlb<P, const SIZE: usize> {
    pub(crate) entries: [TLBEntry<P>; SIZE],

    pub(crate) split_large: bool,
}

impl<P, const SIZE: usize> Tlb<P, SIZE> {
    /// Create a new, flushed TLB
    /// A flushed TLB, constructible in a const context.
    ///
    /// `const` so it can live in `.bss` under no_alloc. `core::array::from_fn`
    /// is not const, so the entry array is built from a const element.
    pub const fn new() -> Self {
        Self {
            entries: [TLBEntry::<P>::INVALID; SIZE],
            split_large: false,
        }
    }

    /// Given a linear page‐frame number (lpf) and optional len,
    /// compute which TLB‐slot it maps to.
    #[inline]
    pub fn get_index_of(&self, lpf: u64, len: u32) -> usize {
        // Mirror: ((size-1)<<12) mask, then shift down by 12
        let tlb_mask = ((SIZE - 1) as u64) << 12;

        ((lpf.wrapping_add(len as u64) & tlb_mask) >> 12) as usize
    }

    /// Get a mutable reference to the matching entry
    #[inline]
    pub(super) fn get_entry_of(&mut self, lpf: u64, len: u32) -> &mut TLBEntry<P> {
        let i = self.get_index_of(lpf, len);
        &mut self.entries[i]
    }
    /// Invalidate the direct-mapped slot selected for a prospective mapping.
    ///
    /// Unlike `invlpg`, this deliberately clears a colliding entry even when
    /// it maps a different linear page.  Callers use it before an allocation
    /// that may need to evict the old host-backed page.
    #[inline]
    pub(super) fn invalidate_slot(&mut self, laddr: u64, len: u32) {
        let slot = self.get_index_of(laddr, len);
        self.entries[slot].invalidate();
    }

    /// Invalidate all entries
    pub fn flush(&mut self) {
        for entry in &mut self.entries {
            entry.invalidate();
        }
        self.split_large = false;
    }

    /// Invalidate all non‐global entries (only if CPU ≥ 6)
    pub fn flush_non_global(&mut self) {
        self.flush_non_global_publishing(|_| {});
    }

    /// Non‐global flush that reports each invalidated slot index so the caller
    /// can fuse pin‐sidecar removal into the same pass (Track B). Behaviourally
    /// identical to `flush_non_global`; `on_invalidate(slot)` runs for every
    /// entry this clears and for none of the entries it keeps.
    #[inline]
    pub(super) fn flush_non_global_publishing<F: FnMut(usize)>(&mut self, mut on_invalidate: F) {
        let mut lpf_mask_accum: u32 = 0;
        for (slot, entry) in self.entries.iter_mut().enumerate() {
            if entry.valid() {
                if (entry.access_bits & TLB_GLOBAL_PAGE) == 0 {
                    entry.invalidate();
                    on_invalidate(slot);
                } else {
                    lpf_mask_accum |= entry.lpf_mask;
                }
            }
        }
        // If any large‐page mask bit remains, we keep split_large = true
        self.split_large = lpf_mask_accum > 0xFFF;
    }

    /// Invalidate a single page (INVLPG)
    pub fn invlpg(&mut self, laddr: u64) {
        self.invlpg_publishing(laddr, |_| {});
    }

    /// INVLPG that reports each invalidated slot index so the caller can fuse
    /// pin‐sidecar removal into the same invalidation (Track B). Behaviourally
    /// identical to `invlpg`: the non‐split path clears at most one slot, the
    /// split‐large path clears every entry whose page contains `laddr`.
    #[inline]
    pub(super) fn invlpg_publishing<F: FnMut(usize)>(&mut self, laddr: u64, mut on_invalidate: F) {
        if self.split_large {
            // We have to scan all entries to handle large pages specially
            let mut lpf_mask_accum: u32 = 0;
            for (slot, entry) in self.entries.iter_mut().enumerate() {
                if entry.valid() {
                    let emask = entry.lpf_mask as u64;
                    if (laddr & !emask) == (entry.lpf & !emask) {
                        entry.invalidate();
                        on_invalidate(slot);
                    } else {
                        lpf_mask_accum |= entry.lpf_mask;
                    }
                }
            }

            self.split_large = lpf_mask_accum > 0xFFF;
            return;
        }

        // Otherwise (not split‐large), simple single‐slot INVLPG:
        let idx = self.get_index_of(laddr, 0);
        let entry = &mut self.entries[idx];
        if lpf_of(entry.lpf) == lpf_of(laddr) {
            entry.invalidate();
            on_invalidate(idx);
        }
    }


}

impl<P: CachedHostPage, const SIZE: usize> Tlb<P, SIZE> {
    /// Allocation offset currently visible to the external eviction sidecar.
    ///
    /// Invalid entries deliberately contribute `None` so an invalidation
    /// removes the pin immediately instead of retaining stale over-pinning.
    #[inline]
    pub(super) fn pinned_alloc_offset(&self, region_offset: usize, slot: usize) -> Option<usize> {
        let entry = &self.entries[slot];
        if entry.valid() {
            entry
                .host_page
                .map(|page| page.alloc_offset_from(region_offset))
        } else {
            None
        }
    }
}

#[inline]
pub(super) fn page_offset<I>(laddr: I) -> u32
where
    I: Into<BxAddress>,
{
    (laddr.into() as u32) & 0xfff
}

#[inline]
pub(super) fn lpf_of(laddr: BxAddress) -> BxAddress {
    laddr & LPF_MASK
}

#[inline]
pub(super) fn ppf_of(paddr: BxAddress) -> BxAddress {
    paddr & PPF_MASK
}

#[cfg(test)]
mod const_initialiser_tests {
    use super::*;

    /// `Tlb::new` builds its entries from a const element now. That element
    /// must carry the invalid-line sentinel, exactly as the deleted
    /// `TLBEntry::new()` did — a zeroed entry does NOT, because zero is a
    /// legitimate linear page frame, so the const must not be defined by
    /// zeroing.
    #[test]
    fn a_fresh_tlb_holds_only_invalid_entries() {
        assert_eq!(TLBEntry::<RamPage>::INVALID.lpf, BX_INVALID_TLB_ENTRY);
        assert_ne!(TLBEntry::<RamPage>::INVALID.lpf, 0);
        let tlb = Tlb::<RamPage, 8>::new();
        for index in 0..8 {
            assert_eq!(tlb.entries[index].lpf, BX_INVALID_TLB_ENTRY);
            assert!(tlb.entries[index].host_page.is_none());
        }
        assert!(!tlb.split_large);
    }

    /// A RAM page number must survive the round trip exactly, including page
    /// zero — the case the bias exists for. A `None` here would mean the data
    /// TLB silently declined to cache the first page of guest RAM.
    #[test]
    fn ram_pages_round_trip_including_page_zero() {
        for offset in [0usize, 0x1000, 0x4_0000, 0xFFFF_F000] {
            let page = RamPage::from_ram_offset(offset)
                .unwrap_or_else(|| panic!("offset {offset:#x} is a legitimate RAM page"));
            assert_eq!(page.ram_offset(), offset);
            assert_eq!(page.host_addr(0x1_0000 as *mut u8) as usize, 0x1_0000 + offset);
        }
        // Not the start of a page, so it names no page at all.
        assert!(RamPage::from_ram_offset(0x1001).is_none());
    }
}
