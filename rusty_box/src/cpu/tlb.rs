use alloc::vec::Vec;

use crate::config::{BxAddress, BxPhyAddress, BxPtrEquiv};

pub type BxMemType = u32;

pub type BxHostpageaddr = BxPtrEquiv;

pub const LPF_MASK: BxAddress = 0xfffffffffffff000u64;

#[cfg(feature = "bx_phy_address_long")]
const PPF_MASK: BxPhyAddress = 0xfffffffffffff000u64;
#[cfg(not(feature = "bx_phy_address_long"))]
const PPF_MASK: BxPhyAddress = 0xfffff000;

const TLB_GLOBAL_PAGE: u32 = 0x80000000;

const BX_INVALID_TLB_ENTRY: u64 = 0xffffffffffffffffu64;

pub(super) struct TLBEntry {
    /// linear page frame
    pub(super) lpf: BxAddress,
    // physical page frame
    pub(super) ppf: BxPhyAddress,
    pub(super) host_page_addr: BxHostpageaddr,
    pub(super) access_bits: u32,
    #[cfg(feature = "bx_support_pkeys")]
    pub(super) pkey: u32,
    // linear address mask of the page size
    pub(super) lpf_mask: u32,
    #[cfg(feature = "bx_support_memtype")]
    pub(super) memtype: MemType, // (note from bochs)  // keep it Bit32u for alignment
}

#[derive(Default)]
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

        #[cfg(feature = "bx_support_pkeys")]
        let pkey = 0;

        let lpf_mask = 0;

        #[cfg(feature = "bx_support_memtype")]
        let memtype = MemType::default();

        Self {
            lpf,
            ppf,
            host_page_addr,
            access_bits,
            #[cfg(feature = "bx_support_pkeys")]
            pkey,
            lpf_mask,
            #[cfg(feature = "bx_support_memtype")]
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
        #[cfg(feature = "bx_support_memtype")]
        {
            self.memtype
        }
        #[cfg(not(feature = "bx_support_memtype"))]
        {
            // emulate the `#else #define MEMTYPE(x) (BX_MEMTYPE_UC)`
            MemType::UC
        }
    }
}

// Our TLB struct, generic over the number of entries:
pub struct Tlb<const SIZE: usize> {
    entries: [TLBEntry; SIZE],

    split_large: bool,
}

impl<const SIZE: usize> Tlb<SIZE> {
    /// Create a new, flushed TLB
    pub fn new() -> Self {
        // Initialize each entry via its `Default` or `new()` constructor:
        let entries: [TLBEntry; SIZE] = {
            // Trick: build from an array of `TLBEntry::new()`
            let mut tmp: Vec<TLBEntry> = Vec::with_capacity(SIZE);
            for _ in 0..SIZE {
                tmp.push(TLBEntry::new());
            }
            tmp.try_into().unwrap_or_else(|_| panic!("SIZE mismatch"))
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
        let idx = ((lpf.wrapping_add(len as u64) & tlb_mask) >> 12) as usize;
        idx
    }

    /// Get a mutable reference to the matching entry
    #[inline]
    pub fn get_entry_of(&mut self, lpf: u64, len: u32) -> &mut TLBEntry {
        let i = self.get_index_of(lpf, len);
        &mut self.entries[i]
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
        let mut lpf_mask_accum: u32 = 0;

        for entry in &mut self.entries {
            if entry.valid() {
                if (entry.access_bits & TLB_GLOBAL_PAGE) == 0 {
                    entry.invalidate();
                } else {
                    lpf_mask_accum |= entry.lpf_mask;
                }
            }
        }
        // If any large‐page mask bit remains, we keep split_large = true
        self.split_large = (lpf_mask_accum > 0xFFF);
    }

    /// Invalidate a single page (INVLPG)
    pub fn invlpg(&mut self, laddr: u64) {
        if self.split_large {
            // We have to scan all entries to handle large pages specially
            let mut lpf_mask_accum: u32 = 0;

            for entry in &mut self.entries {
                if entry.valid() {
                    let emask = entry.lpf_mask as u64;
                    // same logic: if they map to the same page region, invalidate
                    if (laddr & !emask) == (entry.lpf & !emask) {
                        entry.invalidate();
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
