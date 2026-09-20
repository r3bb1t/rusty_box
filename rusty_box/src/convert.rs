//! Integer conversions this crate needs, each proved once.
//!
//! `std` deliberately omits `From<u32> for usize`, because a 16-bit target has
//! a narrower `usize` than a `u32`. This crate builds for no such target —
//! x86_64, wasm32, `x86_64-unknown-uefi` and `x86_64-unknown-none` are the
//! whole list — and the assertion below is what turns that from a comment into
//! a fact the compiler checks.
//!
//! The point is the arithmetic that used to sit at the call sites. Roughly
//! forty of them read `usize::try_from(n).expect("u32 fits usize")`: a panic
//! that could not fire, written as though it could, on the hottest bulk paths
//! in the emulator. The `as` casts those conversions need are real — a cast
//! that narrows does it silently — so they live here, two of them, each beside
//! the reason it cannot lose a bit, rather than scattered where a reviewer has
//! to re-derive the argument (R5: one choke point per hazard). `std` omits the
//! `usize`-to-`u64` direction for the mirror-image reason: a target with a
//! `usize` wider than 64 bits would lose bits going the other way.

/// The width assumption every conversion in this module rests on.
///
/// A target outside this range is a compile error here rather than a silent
/// truncation somewhere in the string unit.
const _: () = assert!(
    usize::BITS >= 32 && usize::BITS <= 64,
    "rusty_box assumes a usize between 32 and 64 bits wide"
);

/// Widen a `u32` to `usize`.
///
/// Lossless because `usize` is at least 32 bits wide on every target this
/// crate builds for, which the assertion above enforces.
#[inline(always)]
pub(crate) const fn usize_from_u32(value: u32) -> usize {
    value as usize
}

/// Widen a `usize` to `u64`.
///
/// Lossless because `usize` is at most 64 bits wide on every target this crate
/// builds for, which the assertion above enforces.
#[inline(always)]
pub(crate) const fn u64_from_usize(value: usize) -> u64 {
    value as u64
}

/// The offset of an address within its 4 KiB page.
///
/// Total for any address, and without a width assumption: the mask leaves at
/// most twelve bits, and every target's `usize` holds a value that small. The
/// mask and the cast are on the same line so the two cannot drift apart.
#[inline(always)]
pub(crate) const fn page_offset(address: u64) -> usize {
    (address & 0xFFF) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The widening conversions keep every bit, at the edges where a narrowing
    /// cast would not.
    #[test]
    fn widening_keeps_every_bit() {
        assert_eq!(usize_from_u32(u32::MAX), 4_294_967_295);
        assert_eq!(usize_from_u32(0x8000_0000), 2_147_483_648);
        assert_eq!(usize_from_u32(0), 0);
        // Round-tripping the largest value is what proves nothing was lost:
        // a conversion that truncated could not come back equal.
        assert_eq!(usize::try_from(u64_from_usize(usize::MAX)), Ok(usize::MAX));
        assert_eq!(u64_from_usize(0), 0);
        assert_eq!(u64_from_usize(0x1234_5678), 0x1234_5678);
    }

    /// A page offset is the low twelve bits and nothing else — in particular
    /// it does not carry the page number down with it.
    #[test]
    fn a_page_offset_is_the_low_twelve_bits() {
        assert_eq!(page_offset(0), 0);
        assert_eq!(page_offset(0xFFF), 0xFFF);
        assert_eq!(page_offset(0x1000), 0);
        assert_eq!(page_offset(0xDEAD_BEEF_0000_1234), 0x234);
        assert_eq!(page_offset(u64::MAX), 0xFFF);
    }
}
