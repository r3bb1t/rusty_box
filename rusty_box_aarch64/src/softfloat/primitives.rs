//! Primitive bit-manipulation and multi-precision integer operations.
//! Ported from Berkeley SoftFloat 3e primitives.h / s_*.c.
//!
//! Design notes:
//! - Uses Rust's `.leading_zeros()` for CLZ (compiles to hardware LZCNT/BSR)
//! - Uses Rust's native `u128` for 128-bit multiply instead of manual decomposition

// --- Shift-right-jam: shift right and jam any discarded bits into LSB ---

/// Shifts 'a' right by 'dist' (1..63), jamming discarded bits into LSB.
#[inline]
pub(crate) fn short_shift_right_jam64(a: u64, dist: u8) -> u64 {
    debug_assert!((1..=63).contains(&dist));
    (a >> dist) | (((a & ((1u64 << dist) - 1)) != 0) as u64)
}

/// Shifts 'a' right by 'dist' (any value), jamming discarded bits into LSB.
/// If dist >= 32, result is 0 or 1.
#[inline]
pub(crate) fn shift_right_jam32(a: u32, dist: u16) -> u32 {
    if dist < 31 {
        (a >> dist) | (((a << ((!dist).wrapping_add(1) & 31)) != 0) as u32)
    } else {
        (a != 0) as u32
    }
}

/// Shifts 'a' right by 'dist' (any value), jamming discarded bits into LSB.
/// If dist >= 64, result is 0 or 1.
#[inline]
pub(crate) fn shift_right_jam64(a: u64, dist: u32) -> u64 {
    if dist < 63 {
        (a >> dist) | (((a << ((!dist).wrapping_add(1) & 63)) != 0) as u64)
    } else {
        (a != 0) as u64
    }
}

// --- 128-bit shift operations ---

/// Berkeley `softfloat_shortShiftLeft128`: shifts the 128-bit value
/// (a64, a0) left by 'dist' (0..63); a distance of 0 is the identity.
///
/// Berkeley states the range 1 to 63 and computes the high word as
/// `a64<<dist | a0>>(-dist & 63)`, which at 0 ORs `a0` into it.
/// `softfloat_mulAddF64` shifts by 0 when its difference's top set bit is
/// bit 62, and then `a0` is 0, so both forms give its result.
#[inline]
pub(crate) fn short_shift_left128(a64: u64, a0: u64, dist: u8) -> (u64, u64) {
    debug_assert!(dist <= 63);
    let z = ((u128::from(a64) << 64) | u128::from(a0)) << (dist & 63);
    ((z >> 64) as u64, z as u64)
}

/// Berkeley `softfloat_shortShiftRightJam128`: shifts the 128-bit value
/// (a64, a0) right by 'dist' (1..63), jamming discarded bits into the LSB.
#[inline]
pub(crate) fn short_shift_right_jam128(a64: u64, a0: u64, dist: u8) -> (u64, u64) {
    debug_assert!((1..=63).contains(&dist));
    let neg_dist = dist.wrapping_neg() & 63;
    let z_v64 = a64 >> dist;
    let z_v0 = (a64 << neg_dist) | (a0 >> dist) | (((a0 << neg_dist) != 0) as u64);
    (z_v64, z_v0)
}

/// Shifts the 128-bit value (a64, a0) right by 'dist' with jam.
pub(crate) fn shift_right_jam128(a64: u64, a0: u64, dist: u32) -> (u64, u64) {
    if dist < 64 {
        if dist == 0 {
            return (a64, a0);
        }
        let z64 = a64 >> dist;
        let z0 = (a64 << ((!dist).wrapping_add(1) & 63))
            | (a0 >> dist)
            | (((a0 << ((!dist).wrapping_add(1) & 63)) != 0) as u64);
        (z64, z0)
    } else if dist == 64 {
        (0, a64 | ((a0 != 0) as u64))
    } else if dist < 128 {
        let d = dist - 64;
        (
            0,
            (a64 >> d) | ((((a64 << ((!d).wrapping_add(1) & 63)) | a0) != 0) as u64),
        )
    } else {
        (0, ((a64 | a0) != 0) as u64)
    }
}

// --- Count leading zeros using Rust intrinsics ---

#[inline]
pub(crate) fn count_leading_zeros16(a: u16) -> u8 {
    if a == 0 {
        16
    } else {
        a.leading_zeros() as u8
    }
}

#[inline]
pub(crate) fn count_leading_zeros32(a: u32) -> u8 {
    if a == 0 {
        32
    } else {
        a.leading_zeros() as u8
    }
}

#[inline]
pub(crate) fn count_leading_zeros64(a: u64) -> u8 {
    if a == 0 {
        64
    } else {
        a.leading_zeros() as u8
    }
}

// --- 128-bit arithmetic using Rust u128 ---

/// Adds two 128-bit values: (a64, a0) + (b64, b0) = (z64, z0)
#[inline]
pub(crate) fn add128(a64: u64, a0: u64, b64: u64, b0: u64) -> (u64, u64) {
    let a = ((a64 as u128) << 64) | (a0 as u128);
    let b = ((b64 as u128) << 64) | (b0 as u128);
    let z = a.wrapping_add(b);
    ((z >> 64) as u64, z as u64)
}

/// Subtracts two 128-bit values: (a64, a0) - (b64, b0) = (z64, z0)
#[inline]
pub(crate) fn sub128(a64: u64, a0: u64, b64: u64, b0: u64) -> (u64, u64) {
    let a = ((a64 as u128) << 64) | (a0 as u128);
    let b = ((b64 as u128) << 64) | (b0 as u128);
    let z = a.wrapping_sub(b);
    ((z >> 64) as u64, z as u64)
}

/// Multiplies two 64-bit values producing a 128-bit result: a * b = (z64, z0)
#[inline]
pub(crate) fn mul64_to_128(a: u64, b: u64) -> (u64, u64) {
    let z = (a as u128) * (b as u128);
    ((z >> 64) as u64, z as u64)
}

// --- Reciprocal square root approximation tables (s_approxRecipSqrt_1Ks.c) ---

pub(crate) static APPROX_RECIP_SQRT_1K0S: [u16; 16] = [
    0xB4C9, 0xFFAB, 0xAA7D, 0xF11C, 0xA1C5, 0xE4C7, 0x9A43, 0xDA29, 0x93B5, 0xD0E5, 0x8DED, 0xC8B7,
    0x88C6, 0xC16D, 0x8424, 0xBAE1,
];

pub(crate) static APPROX_RECIP_SQRT_1K1S: [u16; 16] = [
    0xA5A5, 0xEA42, 0x8C21, 0xC62D, 0x788F, 0xAA7F, 0x6928, 0x94B6, 0x5CC7, 0x8335, 0x52A6, 0x74E2,
    0x4A3E, 0x68FE, 0x432B, 0x5EFD,
];

/// Reciprocal approximation for division.
/// Returns an approximation to 1/A where A = a * 2^-31 (so A is in [1, 2)).
#[inline]
pub(crate) fn approx_recip32_1(a: u32) -> u32 {
    (0x7FFFFFFFFFFFFFFF_u64 / (a as u64)) as u32
}

/// Reciprocal square root approximation for sqrt.
/// Returns approximation to 1/sqrt(A).
pub(crate) fn approx_recip_sqrt32_1(odd_exp: u32, a: u32) -> u32 {
    // Ported from Berkeley SoftFloat 3e s_approxRecipSqrt32_1.c
    let index = ((a >> 27) & 0xE) as usize + odd_exp as usize;
    let eps = (a >> 12) as u16;
    // C: uint16_t r0 — compute with 32-bit wrap, then truncate to 16 bits.
    let r0: u16 = (APPROX_RECIP_SQRT_1K0S[index] as u32)
        .wrapping_sub((APPROX_RECIP_SQRT_1K1S[index] as u32).wrapping_mul(eps as u32) >> 20)
        as u16;
    let mut e_sqr_r0: u32 = (r0 as u32).wrapping_mul(r0 as u32);
    if odd_exp == 0 {
        e_sqr_r0 <<= 1;
    }
    // C: sigma0 = ~(uint32_t)(((uint32_t) ESqrR0 * (uint64_t) a) >> 23)
    let sigma0: u32 = !(((e_sqr_r0 as u64).wrapping_mul(a as u64) >> 23) as u32);
    let mut r: u32 = ((r0 as u32) << 16)
        .wrapping_add(((r0 as u64).wrapping_mul(sigma0 as u64) >> 25) as u32);
    let sqr_sigma0: u32 = ((sigma0 as u64).wrapping_mul(sigma0 as u64) >> 32) as u32;
    // C: r += ((uint32_t)((r>>1)+(r>>3)-((uint32_t)r0<<14)) * (uint64_t)sqrSigma0) >> 48
    let inner: u32 = (r >> 1).wrapping_add(r >> 3).wrapping_sub((r0 as u32) << 14);
    r = r.wrapping_add(((inner as u64).wrapping_mul(sqr_sigma0 as u64) >> 48) as u32);
    if r & 0x8000_0000 == 0 {
        r = 0x8000_0000;
    }
    r
}
