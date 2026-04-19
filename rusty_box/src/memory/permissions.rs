//! Page-level memory permissions bitmap.
//!
//! Tracks per-page READ/WRITE/EXEC permissions, checked on TLB hit
//! when the `instrumentation` feature is enabled. Zero overhead when
//! disabled — the bitmap doesn't exist.

use crate::config::MAX_PERM_PAGES;
use crate::cpu::instrumentation::MemPerms;

const PAGE_SIZE: u64 = 4096;

/// Per-page permission bitmap. Each page gets one byte storing R/W/X bits
/// (matching `MemPerms` layout). Default: all pages have ALL permissions.
pub struct PagePermissions {
    /// 1 byte per page: bits 0=R, 1=W, 2=X (matches MemPerms layout)
    bitmap: [u8; MAX_PERM_PAGES],
    /// Number of pages tracked
    page_count: usize,
}

impl PagePermissions {
    /// Create permissions bitmap covering `size` bytes of physical address space.
    /// All pages default to ALL permissions.
    pub fn new(size: u64) -> Self {
        let page_count = ((size + PAGE_SIZE - 1) / PAGE_SIZE) as usize;
        assert!(
            page_count <= MAX_PERM_PAGES,
            "page_count {} exceeds MAX_PERM_PAGES {}",
            page_count,
            MAX_PERM_PAGES
        );
        let mut bitmap = [0u8; MAX_PERM_PAGES];
        let bits = MemPerms::ALL.bits();
        bitmap[..page_count].fill(bits);
        Self {
            bitmap,
            page_count,
        }
    }

    /// Set permissions for a range of physical addresses.
    /// Addresses are page-aligned down; size is rounded up to page boundary.
    pub fn set(&mut self, addr: u64, size: usize, perms: MemPerms) {
        let start_page = (addr / PAGE_SIZE) as usize;
        let end_page = ((addr + size as u64 + PAGE_SIZE - 1) / PAGE_SIZE) as usize;
        for page in start_page..end_page.min(self.page_count) {
            self.bitmap[page] = perms.bits();
        }
    }

    /// Check if an access at `addr` with `perms_needed` is allowed.
    #[inline]
    pub fn check(&self, addr: u64, perms_needed: MemPerms) -> bool {
        let page = (addr / PAGE_SIZE) as usize;
        if page >= self.page_count {
            return true; // out of range = permissive
        }
        let page_perms = MemPerms::from_bits_truncate(self.bitmap[page]);
        page_perms.contains(perms_needed)
    }
}
