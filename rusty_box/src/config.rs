// pub type BxPhyAddress = u64;
pub type BxPhyAddress = u64;

pub type BxAddress = u64;

#[cfg(target_pointer_width = "32")]
pub type BxPtrEquiv = u32;

#[cfg(target_pointer_width = "64")]
pub type BxPtrEquiv = u64;

#[cfg(not(any(target_pointer_width = "32", target_pointer_width = "64")))]
compile_error!("could not define BxPtrEquivT to size of pointer");


// ── No-alloc sizing constants ──────────────────────────────────────────────

/// 4M BIOS ROM (must match memory_rusty_box::BIOSROMSZ)
pub const BIOSROMSZ: usize = 1 << 22;

/// Expansion ROM 0xc0000-0xdffff (must match memory_rusty_box::EXROMSIZE)
pub const EXROMSIZE: usize = 0x20000;

/// Total buffer size needed for a given amount of guest memory.
/// Includes BIOS ROM + expansion ROM + bogus page + alignment padding.
pub const fn mem_buffer_size(guest_bytes: usize) -> usize {
    guest_bytes + BIOSROMSZ + EXROMSIZE + 4096 + 4096
}

/// Maximum memory blocks (2GB / 128KB block size).
pub const MAX_MEM_BLOCKS: usize = 16384;

/// Maximum permission bitmap pages (1GB / 4KB page size).
pub const MAX_PERM_PAGES: usize = 262144;

/// Maximum MMIO regions for device mapping.
pub const MAX_MMIO_REGIONS: usize = 16;

/// Overflow pool for chained memory handlers.
pub const MAX_HANDLER_OVERFLOW: usize = 16;
