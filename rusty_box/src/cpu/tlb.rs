#![allow(private_interfaces, unused_assignments, dead_code)]

use crate::config::{BxAddress, BxPhyAddress, BxPtrEquiv};

pub type BxMemType = u32;

pub type BxHostpageaddr = BxPtrEquiv;

pub const LPF_MASK: BxAddress = 0xfffffffffffff000u64;

const PPF_MASK: BxPhyAddress = 0xfffffffffffff000u64;

const TLB_GLOBAL_PAGE: u32 = 0x80000000;

const BX_INVALID_TLB_ENTRY: u64 = 0xffffffffffffffffu64;

#[derive(Default)]
pub(crate) struct TLBEntry {
    /// linear page frame
    pub(crate) lpf: BxAddress,
    // physical page frame
    pub(crate) ppf: BxPhyAddress,
    pub(crate) host_page_addr: BxHostpageaddr,
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

impl TLBEntry {
    fn new() -> Self {
        let lpf = BX_INVALID_TLB_ENTRY;
        let access_bits = 0;

        let ppf = 0;
        let host_page_addr = 0;

        let pkey = 0;

        let lpf_mask = 0;

        let memtype = MemType::default();

        Self {
            lpf,
            ppf,
            host_page_addr,
            access_bits,
            pkey,
            lpf_mask,
            memtype,
        }
    }

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

// Our TLB struct, generic over the number of entries:
pub struct Tlb<const SIZE: usize> {
    pub(crate) entries: [TLBEntry; SIZE],

    pub(crate) split_large: bool,
}

impl<const SIZE: usize> Tlb<SIZE> {
    /// Create a new, flushed TLB
    pub fn new() -> Self {
        // Initialize each entry via its `Default` or `new()` constructor:
        let entries: [TLBEntry; SIZE] = {
            // Trick: build from an array of `TLBEntry::new()`
            core::array::from_fn(|_| TLBEntry::new())
        };

        // If we had a split_large field, initialize it here:
        let split_large = false;

        Self {
            entries,
            split_large,
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
    pub(super) fn get_entry_of(&mut self, lpf: u64, len: u32) -> &mut TLBEntry {
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


    /// Host page currently visible to the external eviction sidecar.
    ///
    /// Invalid entries deliberately contribute zero so an invalidation removes
    /// the pin immediately instead of retaining stale over-pinning.
    #[inline]
    pub(super) fn pinned_host_page(&self, slot: usize) -> usize {
        let entry = &self.entries[slot];
        if entry.valid() {
            entry.host_page_addr as usize
        } else {
            0
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
