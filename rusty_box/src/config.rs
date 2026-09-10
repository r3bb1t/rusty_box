/// A guest physical address.
///
/// Facade: defined once, beside the device API that names it most, and
/// re-exported here under the spelling this crate has always used — it appears
/// throughout this crate's own public memory and CPU surface.
pub use rusty_box_devices::api::BxPhyAddress;

pub type BxAddress = u64;

#[cfg(target_pointer_width = "32")]
pub type BxPtrEquiv = u32;
/// The non-zero counterpart of [`BxPtrEquiv`], so a pointer-sized value can be
/// held in an `Option` for free — the niche makes `None` the all-zero pattern.
#[cfg(target_pointer_width = "32")]
pub type BxPtrEquivNonZero = core::num::NonZeroU32;

#[cfg(target_pointer_width = "64")]
pub type BxPtrEquiv = u64;
/// See the 32-bit arm above.
#[cfg(target_pointer_width = "64")]
pub type BxPtrEquivNonZero = core::num::NonZeroU64;

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
