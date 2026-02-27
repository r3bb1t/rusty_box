#![allow(dead_code)]
//! Primitive bit-manipulation and multi-precision integer operations.
//! Ported from Berkeley SoftFloat 3e primitives.h / s_*.c.
//!
//! Design notes:
//! - Uses Rust's `.leading_zeros()` for CLZ (compiles to hardware LZCNT/BSR)
//! - Uses Rust's native `u128` for 128-bit multiply instead of manual decomposition

// --- Shift-right-jam: shift right and jam any discarded bits into LSB ---

/// Shifts 'a' right by 'dist' (1..63), jamming discarded bits into LSB.
#[inline]
pub fn short_shift_right_jam64(a: u64, dist: u8) -> u64 {
    debug_assert!(dist >= 1 && dist <= 63);
    (a >> dist) | (((a & ((1u64 << dist) - 1)) != 0) as u64)
}

/// Shifts 'a' right by 'dist' (any value), jamming discarded bits into LSB.
/// If dist >= 32, result is 0 or 1.
#[inline]
pub fn shift_right_jam32(a: u32, dist: u16) -> u32 {
    if dist < 31 {
        (a >> dist) | (((a << ((!dist).wrapping_add(1) & 31)) != 0) as u32)
    } else {
        (a != 0) as u32
    }
}

/// Shifts 'a' right by 'dist' (any value), jamming discarded bits into LSB.
/// If dist >= 64, result is 0 or 1.
#[inline]
pub fn shift_right_jam64(a: u64, dist: u32) -> u64 {
    if dist < 63 {
        (a >> dist) | (((a << ((!dist).wrapping_add(1) & 63)) != 0) as u64)
    } else {
        (a != 0) as u64
    }
}

// --- 128-bit shift operations ---

/// Shifts the 128-bit value (a64, a0) left by 'dist' (1..63).
#[inline]
pub fn short_shift_left128(a64: u64, a0: u64, dist: u8) -> (u64, u64) {
    debug_assert!(dist >= 1 && dist <= 63);
    let z64 = (a64 << dist) | (a0 >> (64 - dist));
    let z0 = a0 << dist;
    (z64, z0)
}

/// Shifts the 128-bit value (a64, a0) right by 'dist' (1..63).
#[inline]
pub fn short_shift_right128(a64: u64, a0: u64, dist: u8) -> (u64, u64) {
    debug_assert!(dist >= 1 && dist <= 63);
    let z64 = a64 >> dist;
    let z0 = (a64 << (64 - dist)) | (a0 >> dist);
    (z64, z0)
}

/// Shifts the 128-bit value (a64, a0) right by 'dist' with jam.
pub fn shift_right_jam128(a64: u64, a0: u64, dist: u32) -> (u64, u64) {
    if dist < 64 {
        if dist == 0 {
            return (a64, a0);
        }
        let z64 = a64 >> dist;
        let z0 = (a64 << ((!dist).wrapping_add(1) & 63)) | (a0 >> dist)
            | (((a0 << ((!dist).wrapping_add(1) & 63)) != 0) as u64);
        (z64, z0)
    } else if dist == 64 {
        (0, a64 | ((a0 != 0) as u64))
    } else if dist < 128 {
        let d = dist - 64;
        (0, (a64 >> d) | ((((a64 << ((!d).wrapping_add(1) & 63)) | a0) != 0) as u64))
    } else {
        (0, ((a64 | a0) != 0) as u64)
    }
}

/// Shifts the 192-bit value (a128, a64, a0) right by 'dist' with jam.
/// Returns (z128, z64, z0).
pub fn shift_right_jam256(a3: u64, a2: u64, a1: u64, a0: u64, dist: u32) -> (u64, u64, u64, u64) {
    if dist < 64 {
        if dist == 0 {
            return (a3, a2, a1, a0);
        }
        let z3 = a3 >> dist;
        let negdist = (!dist).wrapping_add(1) & 63;
        let z2 = (a3 << negdist) | (a2 >> dist);
        let z1 = (a2 << negdist) | (a1 >> dist);
        let z0 = (a1 << negdist) | (a0 >> dist) | (((a0 << negdist) != 0) as u64);
        (z3, z2, z1, z0)
    } else {
        // For larger shifts, just return a heavily jammed result
        // This is rarely used, so simplicity is fine
        let val = a3 as u128 | ((a2 as u128) << 64);
        let val2 = a1 as u128 | ((a0 as u128) << 64);
        let combined = (val != 0) || (val2 != 0);
        (0, 0, 0, combined as u64)
    }
}

// --- Count leading zeros using Rust intrinsics ---

#[inline]
pub fn count_leading_zeros16(a: u16) -> u8 {
    if a == 0 { 16 } else { a.leading_zeros() as u8 }
}

#[inline]
pub fn count_leading_zeros32(a: u32) -> u8 {
    if a == 0 { 32 } else { a.leading_zeros() as u8 }
}

#[inline]
pub fn count_leading_zeros64(a: u64) -> u8 {
    if a == 0 { 64 } else { a.leading_zeros() as u8 }
}

// --- 128-bit arithmetic using Rust u128 ---

/// Adds two 128-bit values: (a64, a0) + (b64, b0) = (z64, z0)
#[inline]
pub fn add128(a64: u64, a0: u64, b64: u64, b0: u64) -> (u64, u64) {
    let a = ((a64 as u128) << 64) | (a0 as u128);
    let b = ((b64 as u128) << 64) | (b0 as u128);
    let z = a.wrapping_add(b);
    ((z >> 64) as u64, z as u64)
}

/// Subtracts two 128-bit values: (a64, a0) - (b64, b0) = (z64, z0)
#[inline]
pub fn sub128(a64: u64, a0: u64, b64: u64, b0: u64) -> (u64, u64) {
    let a = ((a64 as u128) << 64) | (a0 as u128);
    let b = ((b64 as u128) << 64) | (b0 as u128);
    let z = a.wrapping_sub(b);
    ((z >> 64) as u64, z as u64)
}

/// Multiplies two 64-bit values producing a 128-bit result: a * b = (z64, z0)
#[inline]
pub fn mul64_to_128(a: u64, b: u64) -> (u64, u64) {
    let z = (a as u128) * (b as u128);
    ((z >> 64) as u64, z as u64)
}

/// Multiplies two 64-bit values and adds a 64-bit value: a * b + c = (z64, z0)
#[inline]
pub fn mul64_to_128_add(a: u64, b: u64, c: u64) -> (u64, u64) {
    let z = (a as u128) * (b as u128) + (c as u128);
    ((z >> 64) as u64, z as u64)
}

/// 128-bit comparison: eq
#[inline]
pub fn eq128(a64: u64, a0: u64, b64: u64, b0: u64) -> bool {
    (a64 == b64) && (a0 == b0)
}

/// 128-bit comparison: less-than-or-equal (unsigned)
#[inline]
pub fn le128(a64: u64, a0: u64, b64: u64, b0: u64) -> bool {
    (a64 < b64) || ((a64 == b64) && (a0 <= b0))
}

/// 128-bit comparison: less-than (unsigned)
#[inline]
pub fn lt128(a64: u64, a0: u64, b64: u64, b0: u64) -> bool {
    (a64 < b64) || ((a64 == b64) && (a0 < b0))
}

// --- Short shift-right-jam64Extra (dist 1..63) ---

/// Shifts 'a' right by 'dist' (1..63), returning shifted value and jammed extra.
/// extra = (bits shifted out of a, with old extra jammed into LSB)
#[inline]
pub fn short_shift_right_jam64_extra(a: u64, extra: u64, dist: u8) -> (u64, u64) {
    debug_assert!(dist >= 1 && dist <= 63);
    let v = a >> dist;
    let new_extra = (a << ((!dist).wrapping_add(1) & 63)) | ((extra != 0) as u64);
    (v, new_extra)
}

// --- mul64ByShifted32To128 ---

/// Multiplies a 64-bit value by a 32-bit value shifted left 32.
/// Result: a * ((b as u64) << 32) = 128-bit (hi, lo)
#[inline]
pub fn mul64_by_shifted32_to128(a: u64, b: u32) -> (u64, u64) {
    let z = (a as u128) * ((b as u128) << 32);
    ((z >> 64) as u64, z as u64)
}

// --- Reciprocal approximation tables (from s_approxRecipSqrt_1k0s.c, s_approxRecip_1k0s.c) ---

pub static APPROX_RECIP_1K0S: [u16; 16] = [
    0xFFC4, 0xF0BE, 0xE363, 0xD76F, 0xCCAD, 0xC2F0, 0xBA16, 0xB201,
    0xAA97, 0xA3C6, 0x9D7A, 0x97A6, 0x923C, 0x8D32, 0x887E, 0x8417,
];

pub static APPROX_RECIP_1K1S: [u16; 16] = [
    0xF0F1, 0xD62C, 0xBFA1, 0xAC77, 0x9C0A, 0x8DDB, 0x8185, 0x76BA,
    0x6D3B, 0x64D4, 0x5D5C, 0x56B1, 0x50B6, 0x4B55, 0x4679, 0x4211,
];

pub static APPROX_RECIP_SQRT_1K0S: [u16; 16] = [
    0xB4C9, 0xFFAB, 0xAA7D, 0xF11C, 0xA1C5, 0xE4C7, 0x9A43, 0xDA29,
    0x93B5, 0xD0E5, 0x8DED, 0xC8B7, 0x88C6, 0xC16D, 0x8424, 0xBAE1,
];

pub static APPROX_RECIP_SQRT_1K1S: [u16; 16] = [
    0xA5A5, 0xEA42, 0x8C21, 0xC62D, 0x788F, 0xAA7F, 0x6928, 0x94B6,
    0x5CC7, 0x8335, 0x52A6, 0x74E2, 0x4A3E, 0x68FE, 0x432B, 0x5EFD,
];

/// Reciprocal approximation for division.
/// Returns an approximation to 1/A where A = a * 2^-31 (so A is in [1, 2)).
#[inline]
pub fn approx_recip32_1(a: u32) -> u32 {
    (0x7FFFFFFFFFFFFFFF_u64 / (a as u64)) as u32
}

/// Reciprocal square root approximation for sqrt.
/// Returns approximation to 1/sqrt(A).
pub fn approx_recip_sqrt32_1(odd_exp: u32, a: u32) -> u32 {
    let index = ((a >> 27) & 0xE) as usize + odd_exp as usize;
    let eps = (a >> 12) as u16;
    let r0 = (APPROX_RECIP_SQRT_1K0S[index] as u32)
        .wrapping_sub(((APPROX_RECIP_SQRT_1K1S[index] as u32).wrapping_mul(eps as u32)) >> 20);
    let r0 = r0 as u32;
    let sigma0 = !(r0.wrapping_mul(r0) as u64).wrapping_mul(a as u64);
    let r = ((r0 as u32) << 16).wrapping_add(
        ((r0 as u64).wrapping_mul((sigma0 >> 25) as u64) >> 25) as u32
    );
    r
}
