/// A guest physical address.
///
/// Facade: defined once, beside the device API that names it most, and
/// re-exported here under the spelling this crate has always used — it appears
/// throughout this crate's own public memory and CPU surface.
pub use rusty_box_devices::api::BxPhyAddress;

/// The A20 gate's mask (Bochs `a20_mask`, which Bochs types
/// `bx_phy_address`): the bits of a physical address an access keeps,
/// every one with the gate open and all but bit 20 with it closed. A mask,
/// not an address: it only ever applies to one ([`A20Mask::apply`]).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct A20Mask(u64);

impl A20Mask {
    /// The gate open: every address bit kept.
    pub const OPEN: Self = Self(u64::MAX);
    /// The gate closed: bit 20 forced to zero, as on an 8086.
    pub const CLOSED: Self = Self(0xFFFF_FFFF_FFEF_FFFF);

    /// The mask of a gate that is open where `enabled` holds.
    #[inline(always)]
    pub const fn of_gate(enabled: bool) -> Self {
        if enabled {
            Self::OPEN
        } else {
            Self::CLOSED
        }
    }

    /// The mask whose bits are `bits`, as a snapshot holds it; a caller
    /// checks it is [`A20Mask::OPEN`] or [`A20Mask::CLOSED`].
    #[inline(always)]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// The mask's bits: as a snapshot holds them, and as the CPU's own
    /// address arithmetic, which keeps its addresses as `u64`, applies them.
    #[inline(always)]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// `addr` as the gate passes it.
    #[inline(always)]
    pub const fn apply(self, addr: BxPhyAddress) -> BxPhyAddress {
        addr.masked(self.0)
    }
}

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
