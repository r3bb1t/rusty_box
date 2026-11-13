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
    /// The gate closed: bit 20 forced to zero, as on an 8086.
    pub const CLOSED: Self = Self(0xFFFF_FFFF_FFEF_FFFF);
    /// The gate open: every address bit kept.
    pub const OPEN: Self = Self(u64::MAX);

    /// The mask of a gate that is open where `enabled` holds.
    #[inline(always)]
    pub const fn of_gate(enabled: bool) -> Self {
        if enabled { Self::OPEN } else { Self::CLOSED }
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

    /// Whether the gate is open: the mask keeps bit 20.
    #[inline(always)]
    pub const fn is_open(self) -> bool {
        self.0 & (1 << 20) != 0
    }

    /// `addr` as the gate passes it.
    #[inline(always)]
    pub const fn apply(self, addr: BxPhyAddress) -> BxPhyAddress {
        addr.masked(self.0)
    }
}

pub type BxAddress = u64;

/// A guest linear address (Bochs `bx_address` where it names a linear
/// address): a segment's base plus an offset, before paging translates it.
///
/// An address comes into being where a segment base meets an offset
/// (Bochs `get_laddr32`/`get_laddr64`), or where the architecture names one
/// (a descriptor table's base, an `INVLPG` operand, a debug register), and
/// leaves as a bare `u64` ([`get`]) only where a number is what is wanted: a
/// TLB index, a value a guest reads back (`CR2`, an exit qualification), a
/// snapshot. Its arithmetic is a position's (R4), as [`BxPhyAddress`]'s is:
/// an address plus a distance is an address, an address minus an address a
/// distance, modulo 2^64; masking is by name ([`page_base`], [`page_offset`],
/// [`masked`]).
///
/// [`get`]: BxLinAddress::get
/// [`page_base`]: BxLinAddress::page_base
/// [`page_offset`]: BxLinAddress::page_offset
/// [`masked`]: BxLinAddress::masked
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BxLinAddress(u64);

impl BxLinAddress {
    /// The linear address `addr`.
    #[inline(always)]
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    /// The 32-bit linear address `addr`, as legacy and compatibility mode
    /// form one (Bochs `get_laddr32`).
    #[inline(always)]
    pub const fn from_u32(addr: u32) -> Self {
        Self(addr as u64)
    }

    /// The address as a number.
    #[inline(always)]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The low 32 bits of the address, as a 32-bit linear address keeps
    /// them.
    #[inline(always)]
    pub const fn low32(self) -> u32 {
        self.0 as u32
    }

    /// The 4 KiB page the address lies in, as the address of its first byte
    /// (Bochs `LPFOf`).
    #[inline(always)]
    pub const fn page_base(self) -> Self {
        Self(self.0 & !0xFFF)
    }

    /// How far into its 4 KiB page the address lies (Bochs `PAGE_OFFSET`).
    #[inline(always)]
    pub const fn page_offset(self) -> u64 {
        self.0 & 0xFFF
    }

    /// Whether the address is a multiple of `align`, a power of two.
    #[inline(always)]
    pub const fn is_aligned(self, align: u64) -> bool {
        self.0 & (align - 1) == 0
    }

    /// The byte offset, in a translation table of 8-byte entries at `level`
    /// (0 the page table, 1 the page directory, and so on up), of the entry
    /// that maps this address (Bochs paging.cc `(laddr >> (9 + 9*leaf)) &
    /// 0xff8`).
    #[inline(always)]
    pub const fn entry_offset_64(self, level: usize) -> u64 {
        (self.0 >> (9 + 9 * level)) & 0xFF8
    }

    /// The index, in a legacy translation table of 4-byte entries at
    /// `level` (0 the page table, 1 the page directory), of the entry that
    /// maps this address: bits 21:12 or 31:22.
    #[inline(always)]
    pub const fn entry_index_32(self, level: usize) -> u64 {
        (self.0 >> (12 + 10 * level)) & 0x3FF
    }

    /// The index of the PAE page-directory-pointer-table entry that maps
    /// this address: bits 31:30.
    #[inline(always)]
    pub const fn pdpte_index(self) -> usize {
        ((self.0 >> 30) & 0x3) as usize
    }

    /// The first byte of the 4 KiB page after the one the address lies in,
    /// modulo 2^64: where the rest of a page-crossing access lies.
    #[inline(always)]
    pub const fn next_page(self) -> Self {
        Self((self.0 | 0xFFF).wrapping_add(1))
    }

    /// The address with only the bits `mask` keeps: a mode's address width,
    /// an alignment, a large page's frame.
    #[inline(always)]
    pub const fn masked(self, mask: u64) -> Self {
        Self(self.0 & mask)
    }

    /// `distance` bytes past the address, modulo 2^64.
    #[inline(always)]
    pub const fn wrapping_add(self, distance: u64) -> Self {
        Self(self.0.wrapping_add(distance))
    }

    /// `distance` bytes past the address, or `None` past the end of the
    /// address space.
    #[inline(always)]
    pub const fn checked_add(self, distance: u64) -> Option<Self> {
        match self.0.checked_add(distance) {
            Some(addr) => Some(Self(addr)),
            None => None,
        }
    }

    /// `distance` bytes before the address, modulo 2^64.
    #[inline(always)]
    pub const fn wrapping_sub(self, distance: u64) -> Self {
        Self(self.0.wrapping_sub(distance))
    }

    /// How far past `base` the address lies, modulo 2^64.
    #[inline(always)]
    pub const fn offset_from(self, base: Self) -> u64 {
        self.0.wrapping_sub(base.0)
    }

    /// The physical address an access to this linear address reaches with
    /// paging off: the same number, through the A20 gate (Bochs `A20ADDR`
    /// applied to a linear address).
    #[inline(always)]
    pub const fn unpaged(self, a20: A20Mask) -> BxPhyAddress {
        a20.apply(BxPhyAddress::new(self.0))
    }
}

impl core::ops::Add<u64> for BxLinAddress {
    type Output = Self;

    /// [`BxLinAddress::wrapping_add`].
    #[inline(always)]
    fn add(self, distance: u64) -> Self {
        self.wrapping_add(distance)
    }
}

impl core::ops::AddAssign<u64> for BxLinAddress {
    #[inline(always)]
    fn add_assign(&mut self, distance: u64) {
        *self = self.wrapping_add(distance);
    }
}

impl core::ops::Sub for BxLinAddress {
    type Output = u64;

    /// [`BxLinAddress::offset_from`].
    #[inline(always)]
    fn sub(self, base: Self) -> u64 {
        self.offset_from(base)
    }
}

impl core::fmt::LowerHex for BxLinAddress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::LowerHex::fmt(&self.0, f)
    }
}

impl core::fmt::UpperHex for BxLinAddress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::UpperHex::fmt(&self.0, f)
    }
}

/// An effective address: an offset within a segment, before the segment's
/// base is added (Intel SDM Vol. 1 §3.7.5; Bochs `bx_address eaddr` and the
/// `offset` the `read_virtual_*`/`write_virtual_*` accessors take).
///
/// An offset comes into being where an instruction forms one — its ModRM
/// or SIB operand (Bochs `BX_CPU_RESOLVE_ADDR`), a direct memory offset, a
/// stack or string register used as its pointer — and becomes a linear
/// address only through `get_laddr` (Bochs `get_laddr32`/`get_laddr64`). A
/// register's value stays a number until it is used as an offset. Its
/// arithmetic is a position's, as [`BxLinAddress`]'s is; the address size
/// an offset wraps at is the caller's, applied by [`masked`] or by
/// [`low32`]/[`low16`].
///
/// [`masked`]: BxEffAddress::masked
/// [`low32`]: BxEffAddress::low32
/// [`low16`]: BxEffAddress::low16
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BxEffAddress(u64);

impl BxEffAddress {
    /// The offset `offset`.
    #[inline(always)]
    pub const fn new(offset: u64) -> Self {
        Self(offset)
    }

    /// The 32-bit offset `offset`, as 16- and 32-bit addressing forms one.
    #[inline(always)]
    pub fn from_u32(offset: u32) -> Self {
        Self(u64::from(offset))
    }

    /// The 16-bit offset `offset`, as 16-bit addressing forms one.
    #[inline(always)]
    pub fn from_u16(offset: u16) -> Self {
        Self(u64::from(offset))
    }

    /// The offset as a number.
    #[inline(always)]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The low 32 bits of the offset, as 32-bit addressing keeps them.
    #[inline(always)]
    pub const fn low32(self) -> u32 {
        self.0 as u32
    }

    /// The low 16 bits of the offset, as 16-bit addressing keeps them.
    #[inline(always)]
    pub const fn low16(self) -> u16 {
        self.0 as u16
    }

    /// The offset with only the bits `mask` keeps: an address size's width
    /// (Bochs `i->asize_mask()`), an alignment.
    #[inline(always)]
    pub const fn masked(self, mask: u64) -> Self {
        Self(self.0 & mask)
    }

    /// Whether the offset is a multiple of `align`, a power of two.
    #[inline(always)]
    pub const fn is_aligned(self, align: u64) -> bool {
        self.0 & (align - 1) == 0
    }

    /// `distance` bytes past the offset, modulo 2^64.
    #[inline(always)]
    pub const fn wrapping_add(self, distance: u64) -> Self {
        Self(self.0.wrapping_add(distance))
    }

    /// `distance` bytes before the offset, modulo 2^64.
    #[inline(always)]
    pub const fn wrapping_sub(self, distance: u64) -> Self {
        Self(self.0.wrapping_sub(distance))
    }
}

impl core::fmt::LowerHex for BxEffAddress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::LowerHex::fmt(&self.0, f)
    }
}

impl core::fmt::UpperHex for BxEffAddress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::UpperHex::fmt(&self.0, f)
    }
}

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
