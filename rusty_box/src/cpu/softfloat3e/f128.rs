#![allow(dead_code, non_snake_case, non_camel_case_types)]
//! Float128 (quad-precision) arithmetic library for the Rusty Box FPU.
//!
//! Ported from Berkeley SoftFloat 3e f128_*.c files and Bochs
//! softfloat-helpers.h / fpu_constant.h.
//!
//! Used by FPU transcendental instructions (sin, cos, tan, atan, log, exp)
//! which require float128 for internal precision.
//!
//! Float128 format (IEEE 754 binary128):
//!   - Sign:     bit 127 (MSB of v64)
//!   - Exponent: bits 126:112 (15 bits, bias = 16383 = 0x3FFF)
//!   - Fraction: bits 111:0 (112 bits = 48 bits in v64 + 64 bits in v0)

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

// ============================================================
// Float128 type
// ============================================================

/// 128-bit floating-point value with explicit high/low u64 fields.
/// v64 = high 64 bits (sign + exponent + upper 48 bits of fraction)
/// v0  = low 64 bits (lower 64 bits of fraction)
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Float128 {
    pub(crate) v64: u64,
    pub(crate) v0: u64,
}

impl Float128 {
    #[inline]
    pub const fn new(v64: u64, v0: u64) -> Self {
        Self { v64, v0 }
    }

    /// Create from the existing float128_t (u128) type alias.
    #[inline]
    pub const fn from_u128(val: u128) -> Self {
        Self {
            v64: (val >> 64) as u64,
            v0: val as u64,
        }
    }

    /// Convert to u128.
    #[inline]
    pub const fn to_u128(self) -> u128 {
        ((self.v64 as u128) << 64) | (self.v0 as u128)
    }
}

// ============================================================
// Float128 constants
// ============================================================

pub const FLOAT128_DEFAULT_NAN_V64: u64 = 0xFFFF800000000000;
pub const FLOAT128_DEFAULT_NAN_V0: u64 = 0;
pub const FLOAT128_DEFAULT_NAN: Float128 = Float128 {
    v64: FLOAT128_DEFAULT_NAN_V64,
    v0: FLOAT128_DEFAULT_NAN_V0,
};

pub const FLOAT128_POSITIVE_ZERO: Float128 = Float128 { v64: 0, v0: 0 };
pub const FLOAT128_EXP_BIAS: i32 = 0x3FFF;

// ============================================================
// FPU constants (from fpu_constant.h)
// ============================================================

// PI constants (Pentium-compatible 68-bit precision)
pub const FLOATX80_PI_EXP: u16 = 0x4000;
pub const FLOAT_PI_HI: u64 = 0xc90fdaa22168c234;
pub const FLOAT_PI_LO: u64 = 0xC000000000000000;

pub const FLOATX80_PI2_EXP: u16 = 0x3FFF;
pub const FLOATX80_PI4_EXP: u16 = 0x3FFE;

// 3PI/4 constants
pub const FLOATX80_3PI4_EXP: u16 = 0x4000;
pub const FLOAT_3PI4_HI: u64 = 0x96cbe3f9990e91a7;
pub const FLOAT_3PI4_LO: u64 = 0x9000000000000000;

// 1/LN2 constants
pub const FLOAT_LN2INV_EXP: i32 = 0x3FFF;
pub const FLOAT_LN2INV_HI: u64 = 0xb8aa3b295c17f0bb;
pub const FLOAT_LN2INV_LO: u64 = 0xC000000000000000;

// ============================================================
// Field extraction helpers
// ============================================================

/// Extract sign from Float128 high word.
#[inline]
pub(crate) fn sign_f128_ui64(v64: u64) -> bool {
    (v64 >> 63) != 0
}

/// Extract biased exponent from Float128 high word.
#[inline]
pub(crate) fn exp_f128_ui64(v64: u64) -> i32 {
    ((v64 >> 48) & 0x7FFF) as i32
}

/// Extract fraction (significand without implicit bit) from Float128 high word.
#[inline]
pub(crate) fn frac_f128_ui64(v64: u64) -> u64 {
    v64 & 0x0000_FFFF_FFFF_FFFF
}

/// Pack sign, exponent, and significand high bits into Float128 high word.
#[inline]
pub(crate) fn pack_to_f128_ui64(sign: bool, exp: i32, sig64: u64) -> u64 {
    ((sign as u64) << 63) + ((exp as u64) << 48) + sig64
}

/// Pack a complete Float128.
#[inline]
pub(crate) fn pack_float128(sign: bool, exp: i32, sig64: u64, sig0: u64) -> Float128 {
    Float128 {
        v64: pack_to_f128_ui64(sign, exp, sig64),
        v0: sig0,
    }
}

// ============================================================
// NaN detection for Float128
// ============================================================

/// Is this a NaN (any NaN)?
#[inline]
pub(crate) fn is_nan_f128_ui(v64: u64, v0: u64) -> bool {
    ((!v64 & 0x7FFF000000000000) == 0) && (v0 != 0 || (v64 & 0x0000FFFFFFFFFFFF) != 0)
}

/// Is this a signaling NaN?
#[inline]
pub(crate) fn is_sig_nan_f128_ui(v64: u64, v0: u64) -> bool {
    ((v64 & 0x7FFF800000000000) == 0x7FFF000000000000)
        && (v0 != 0 || (v64 & 0x00007FFFFFFFFFFF) != 0)
}

// ============================================================
// NaN propagation for Float128
// ============================================================

/// Propagate NaN for Float128 operands (x86-SSE semantics).
pub(crate) fn softfloat_propagate_nan_f128_ui(
    ui_a64: u64,
    ui_a0: u64,
    ui_b64: u64,
    ui_b0: u64,
    status: &mut SoftFloatStatus,
) -> Float128 {
    let is_sig_nan_a = is_sig_nan_f128_ui(ui_a64, ui_a0);
    if is_sig_nan_a || is_sig_nan_f128_ui(ui_b64, ui_b0) {
        softfloat_raiseFlags(status, FLAG_INVALID);
    }
    let (mut v64, v0) = if is_sig_nan_a || is_nan_f128_ui(ui_a64, ui_a0) {
        (ui_a64, ui_a0)
    } else {
        (ui_b64, ui_b0)
    };
    // Set the quiet bit
    v64 |= 0x0000800000000000;
    Float128 { v64, v0 }
}

// ============================================================
// Subnormal normalization for Float128
// ============================================================

pub(crate) struct ExpSig128 {
    pub(crate) exp: i32,
    pub(crate) sig_v64: u64,
    pub(crate) sig_v0: u64,
}

pub(crate) fn norm_subnormal_f128_sig(sig64: u64, sig0: u64) -> ExpSig128 {
    if sig64 == 0 {
        let shift_dist = count_leading_zeros64(sig0) as i8 - 15;
        let exp = -63 - shift_dist as i32;
        if shift_dist < 0 {
            let neg = (-shift_dist) as u8;
            ExpSig128 {
                exp,
                sig_v64: sig0 >> neg,
                sig_v0: sig0 << (shift_dist as u8 & 63),
            }
        } else {
            ExpSig128 {
                exp,
                sig_v64: sig0 << (shift_dist as u8),
                sig_v0: 0,
            }
        }
    } else {
        let shift_dist = count_leading_zeros64(sig64) as i8 - 15;
        let exp = 1 - shift_dist as i32;
        let (v64, v0) = short_shift_left128(sig64, sig0, shift_dist as u8);
        ExpSig128 {
            exp,
            sig_v64: v64,
            sig_v0: v0,
        }
    }
}

// ============================================================
// 128-bit extra shift helpers (used by round-and-pack)
// ============================================================

/// Shift right by dist (1..63) with extra jam word.
/// Returns (v64, v0, extra).
#[inline]
pub(crate) fn short_shift_right_jam128_extra(
    a64: u64,
    a0: u64,
    extra: u64,
    dist: u8,
) -> (u64, u64, u64) {
    let neg_dist = (-(dist as i8)) as u8;
    let z_v64 = a64 >> dist;
    let z_v0 = (a64 << (neg_dist & 63)) | (a0 >> dist);
    let z_extra = (a0 << (neg_dist & 63)) | ((extra != 0) as u64);
    (z_v64, z_v0, z_extra)
}

/// Shift right by dist (any value) with extra jam word.
/// Returns (v64, v0, extra).
pub(crate) fn shift_right_jam128_extra(
    a64: u64,
    a0: u64,
    mut extra: u64,
    dist: u32,
) -> (u64, u64, u64) {
    let u8_neg_dist = (-(dist as i8)) as u8;
    if dist < 64 {
        let z_v64 = a64 >> dist;
        let z_v0 = (a64 << (u8_neg_dist & 63)) | (a0 >> dist);
        let z_extra = a0 << (u8_neg_dist & 63);
        (z_v64, z_v0, z_extra | ((extra != 0) as u64))
    } else if dist == 64 {
        (0, a64, a0 | ((extra != 0) as u64))
    } else {
        extra |= a0;
        if dist < 128 {
            let z_v0 = a64 >> (dist & 63);
            let z_extra = a64 << (u8_neg_dist & 63);
            (0, z_v0, z_extra | ((extra != 0) as u64))
        } else {
            let z_extra = if dist == 128 { a64 } else { (a64 != 0) as u64 };
            (0, 0, z_extra | ((extra != 0) as u64))
        }
    }
}

/// Short shift right with jam (1..63), combines shifted-out bits into v0 LSB.
#[inline]
pub(crate) fn short_shift_right_jam128(a64: u64, a0: u64, dist: u8) -> (u64, u64) {
    let neg_dist = (-(dist as i8)) as u8;
    let z_v64 = a64 >> dist;
    let z_v0 = (a64 << (neg_dist & 63)) | (a0 >> dist) | (((a0 << (neg_dist & 63)) != 0) as u64);
    (z_v64, z_v0)
}

// ============================================================
// 128-bit x 32-bit multiply (used by f128_div)
// ============================================================

/// Multiply 128-bit value by 32-bit scalar, returning 128-bit result.
pub(crate) fn mul128_by_32(a64: u64, a0: u64, b: u32) -> (u64, u64) {
    let b = b as u64;
    let z_v0 = a0.wrapping_mul(b);
    let mid = ((a0 >> 32) as u32 as u64).wrapping_mul(b);
    let carry = ((z_v0 >> 32) as u32).wrapping_sub(mid as u32);
    let z_v64 = a64
        .wrapping_mul(b)
        .wrapping_add((mid.wrapping_add(carry as u64)) >> 32);
    (z_v64, z_v0)
}

// ============================================================
// 128x128 -> 256-bit multiply
// ============================================================

/// Multiply two 128-bit values to produce a 256-bit result.
/// Returns [z0, z1, z2, z3] where z0 is the lowest 64 bits
/// and z3 is the highest 64 bits (little-endian word order).
pub(crate) fn mul128_to_256(a64: u64, a0: u64, b64: u64, b0: u64) -> [u64; 4] {
    let (p0_v64, p0_v0) = mul64_to_128(a0, b0);
    let (p64_v64, p64_v0) = mul64_to_128(a64, b0);
    let mut z64 = p64_v0.wrapping_add(p0_v64);
    let mut z128 = p64_v64.wrapping_add((z64 < p64_v0) as u64);
    let (p128_v64, p128_v0) = mul64_to_128(a64, b64);
    z128 = z128.wrapping_add(p128_v0);
    let mut z192 = p128_v64.wrapping_add((z128 < p128_v0) as u64);
    let (p64b_v64, p64b_v0) = mul64_to_128(a0, b64);
    z64 = z64.wrapping_add(p64b_v0);
    let carry = (z64 < p64b_v0) as u64;
    let p64b_v64 = p64b_v64.wrapping_add(carry);
    z128 = z128.wrapping_add(p64b_v64);
    z192 = z192.wrapping_add((z128 < p64b_v64) as u64);

    [p0_v0, z64, z128, z192]
}

// ============================================================
// 192-bit integer operations (from softfloat-helpers.h)
// ============================================================

/// Multiply 128-bit value (a_hi, a_lo) by 64-bit value b, producing 192-bit result.
/// Returns (z2, z1, z0) where z2 is the highest 64 bits.
pub(crate) fn mul128_by_64_to_192(a_hi: u64, a_lo: u64, b: u64) -> (u64, u64, u64) {
    // Use mul128_to_256 with b_hi = 0
    let result = mul128_to_256(a_hi, a_lo, 0, b);
    // result[3] should be 0 since b_hi=0
    debug_assert!(result[3] == 0);
    (result[2], result[1], result[0])
}

/// Add two 192-bit values.
/// (a0, a1, a2) + (b0, b1, b2) = (z0, z1, z2)
/// where a0/b0/z0 are the highest words and a2/b2/z2 are the lowest.
pub(crate) fn add192(a0: u64, a1: u64, a2: u64, b0: u64, b1: u64, b2: u64) -> (u64, u64, u64) {
    let z2 = a2.wrapping_add(b2);
    let carry1 = (z2 < a2) as u64;
    let mut z1 = a1.wrapping_add(b1);
    let carry0 = (z1 < a1) as u64;
    let mut z0 = a0.wrapping_add(b0);
    z1 = z1.wrapping_add(carry1);
    z0 = z0.wrapping_add((z1 < carry1) as u64);
    z0 = z0.wrapping_add(carry0);
    (z0, z1, z2)
}

/// Subtract two 192-bit values.
/// (a0, a1, a2) - (b0, b1, b2) = (z0, z1, z2)
pub(crate) fn sub192(a0: u64, a1: u64, a2: u64, b0: u64, b1: u64, b2: u64) -> (u64, u64, u64) {
    let z2 = a2.wrapping_sub(b2);
    let borrow1 = (a2 < b2) as u64;
    let mut z1 = a1.wrapping_sub(b1);
    let borrow0 = (a1 < b1) as u64;
    let mut z0 = a0.wrapping_sub(b0);
    z1 = z1.wrapping_sub(borrow1);
    // If z1 wrapped past zero when subtracting borrow1 (only when old z1 was 0 and borrow1 was 1)
    z0 = z0.wrapping_sub(if a1.wrapping_sub(b1) < borrow1 { 1 } else { 0 });
    z0 = z0.wrapping_sub(borrow0);
    (z0, z1, z2)
}

// ============================================================
// estimateDiv128To64 (from softfloat-helpers.h)
// ============================================================

/// Approximation to the 64-bit quotient of dividing (a0, a1) by b.
/// b must be >= 2^63. Result is within [q, q+2] of exact quotient.
pub(crate) fn estimate_div_128_to_64(a0: u64, a1: u64, b: u64) -> u64 {
    if b <= a0 {
        return 0xFFFFFFFFFFFFFFFF;
    }
    let b0 = b >> 32;
    let mut z = if (b0 << 32) <= a0 {
        0xFFFFFFFF00000000u64
    } else {
        (a0 / b0) << 32
    };
    let (term_v64, term_v0) = mul64_to_128(b, z);
    let (mut rem_v64, mut rem_v0) = sub128(a0, a1, term_v64, term_v0);
    while (rem_v64 as i64) < 0 {
        z = z.wrapping_sub(0x100000000);
        let b1 = b << 32;
        let (new_v64, new_v0) = add128(rem_v64, rem_v0, b0, b1);
        rem_v64 = new_v64;
        rem_v0 = new_v0;
    }
    rem_v64 = (rem_v64 << 32) | (rem_v0 >> 32);
    z |= if (b0 << 32) <= rem_v64 {
        0xFFFFFFFF
    } else {
        rem_v64 / b0
    };
    z
}

// ============================================================
// Round-and-pack to Float128
// ============================================================

/// Round and pack a Float128 result.
///
/// NOTE: Bochs trims precision to ~80 bits to match hardware x86 which uses
/// only 67-bit internal precision for transcendentals. We replicate this
/// behavior: sigExtra is zeroed and sig0 is masked to upper 32 bits.
pub(crate) fn round_pack_to_f128(
    sign: bool,
    mut exp: i32,
    mut sig64: u64,
    mut sig0: u64,
    mut sig_extra: u64,
    status: &mut SoftFloatStatus,
) -> Float128 {
    // Artificially reduce precision to match hardware x86
    sig_extra = 0;
    sig0 &= 0xFFFFFFFF00000000;

    let do_increment = 0x8000000000000000 <= sig_extra;

    if 0x7FFD <= (exp as u32).wrapping_sub(1) {
        if exp < 0 {
            let is_tiny = (exp < -1)
                || !do_increment
                || lt128(sig64, sig0, 0x0001FFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF);
            let (new_v64, new_v0, new_extra) =
                shift_right_jam128_extra(sig64, sig0, sig_extra, (-exp) as u32);
            sig64 = new_v64;
            sig0 = new_v0;
            sig_extra = new_extra;
            exp = 0;
            if is_tiny && sig_extra != 0 {
                softfloat_raiseFlags(status, FLAG_UNDERFLOW);
            }
            let do_increment = 0x8000000000000000 <= sig_extra;
            if sig_extra != 0 {
                softfloat_raiseFlags(status, FLAG_INEXACT);
            }
            if do_increment {
                let (new64, new0) = add128(sig64, sig0, 0, 1);
                sig64 = new64;
                sig0 = new0 & !(((sig_extra & 0x7FFFFFFFFFFFFFFF) == 0) as u64);
            } else {
                if (sig64 | sig0) == 0 {
                    exp = 0;
                }
            }
            return Float128 {
                v64: pack_to_f128_ui64(sign, exp, sig64),
                v0: sig0,
            };
        } else if (0x7FFD < exp)
            || ((exp == 0x7FFD)
                && eq128(sig64, sig0, 0x0001FFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF)
                && do_increment)
        {
            softfloat_raiseFlags(status, FLAG_OVERFLOW | FLAG_INEXACT);
            return Float128 {
                v64: pack_to_f128_ui64(sign, 0x7FFF, 0),
                v0: 0,
            };
        }
    }

    if sig_extra != 0 {
        softfloat_raiseFlags(status, FLAG_INEXACT);
    }
    if do_increment {
        let (new64, new0) = add128(sig64, sig0, 0, 1);
        sig64 = new64;
        sig0 = new0 & !(((sig_extra & 0x7FFFFFFFFFFFFFFF) == 0) as u64);
    } else {
        if (sig64 | sig0) == 0 {
            exp = 0;
        }
    }
    Float128 {
        v64: pack_to_f128_ui64(sign, exp, sig64),
        v0: sig0,
    }
}

/// Normalize, round, and pack to Float128.
pub(crate) fn norm_round_pack_to_f128(
    sign: bool,
    mut exp: i32,
    mut sig64: u64,
    mut sig0: u64,
    status: &mut SoftFloatStatus,
) -> Float128 {
    if sig64 == 0 {
        exp -= 64;
        sig64 = sig0;
        sig0 = 0;
    }
    let shift_dist = count_leading_zeros64(sig64) as i8 - 15;
    exp -= shift_dist as i32;
    let sig_extra;
    if 0 <= shift_dist {
        if shift_dist != 0 {
            let (new64, new0) = short_shift_left128(sig64, sig0, shift_dist as u8);
            sig64 = new64;
            sig0 = new0;
        }
        if (exp as u32) < 0x7FFD {
            let result_exp = if (sig64 | sig0) != 0 { exp } else { 0 };
            return Float128 {
                v64: pack_to_f128_ui64(sign, result_exp, sig64),
                v0: sig0,
            };
        }
        sig_extra = 0;
    } else {
        let neg_shift = (-shift_dist) as u8;
        let (new64, new0, new_extra) = short_shift_right_jam128_extra(sig64, sig0, 0, neg_shift);
        sig64 = new64;
        sig0 = new0;
        sig_extra = new_extra;
    }
    round_pack_to_f128(sign, exp, sig64, sig0, sig_extra, status)
}

// ============================================================
// softfloat_addMagsF128 / softfloat_subMagsF128
// ============================================================

fn add_mags_f128(
    ui_a64: u64,
    ui_a0: u64,
    ui_b64: u64,
    ui_b0: u64,
    sign_z: bool,
    status: &mut SoftFloatStatus,
) -> Float128 {
    let exp_a = exp_f128_ui64(ui_a64);
    let mut sig_a = (frac_f128_ui64(ui_a64), ui_a0); // (v64, v0)
    let exp_b = exp_f128_ui64(ui_b64);
    let mut sig_b = (frac_f128_ui64(ui_b64), ui_b0);
    let exp_diff = exp_a - exp_b;

    if exp_diff == 0 {
        if exp_a == 0x7FFF {
            if (sig_a.0 | sig_a.1 | sig_b.0 | sig_b.1) != 0 {
                return softfloat_propagate_nan_f128_ui(ui_a64, ui_a0, ui_b64, ui_b0, status);
            }
            return Float128 {
                v64: ui_a64,
                v0: ui_a0,
            };
        }
        let (sig_z64, sig_z0) = add128(sig_a.0, sig_a.1, sig_b.0, sig_b.1);
        if exp_a == 0 {
            return Float128 {
                v64: pack_to_f128_ui64(sign_z, 0, sig_z64),
                v0: sig_z0,
            };
        }
        let exp_z = exp_a;
        let sig_z64 = sig_z64 | 0x0002000000000000;
        let sig_z_extra = 0u64;
        // shiftRight1
        let (z64, z0, z_extra) = short_shift_right_jam128_extra(sig_z64, sig_z0, sig_z_extra, 1);
        return round_pack_to_f128(sign_z, exp_z, z64, z0, z_extra, status);
    }

    let mut exp_z;
    let sig_z_extra;

    if exp_diff < 0 {
        if exp_b == 0x7FFF {
            if (sig_b.0 | sig_b.1) != 0 {
                return softfloat_propagate_nan_f128_ui(ui_a64, ui_a0, ui_b64, ui_b0, status);
            }
            return Float128 {
                v64: pack_to_f128_ui64(sign_z, 0x7FFF, 0),
                v0: 0,
            };
        }
        exp_z = exp_b;
        if exp_a != 0 {
            sig_a.0 |= 0x0001000000000000;
        } else {
            let new_diff = exp_diff + 1;
            if new_diff == 0 {
                // newlyAligned
                let (sig_z64, sig_z0) =
                    add128(sig_a.0 | 0x0001000000000000, sig_a.1, sig_b.0, sig_b.1);
                exp_z -= 1;
                if sig_z64 < 0x0002000000000000 {
                    return round_pack_to_f128(sign_z, exp_z, sig_z64, sig_z0, 0, status);
                }
                exp_z += 1;
                let (z64, z0, z_extra) = short_shift_right_jam128_extra(sig_z64, sig_z0, 0, 1);
                return round_pack_to_f128(sign_z, exp_z, z64, z0, z_extra, status);
            }
        }
        let (new64, new0, new_extra) =
            shift_right_jam128_extra(sig_a.0, sig_a.1, 0, (-exp_diff) as u32);
        sig_a = (new64, new0);
        sig_z_extra = new_extra;
    } else {
        if exp_a == 0x7FFF {
            if (sig_a.0 | sig_a.1) != 0 {
                return softfloat_propagate_nan_f128_ui(ui_a64, ui_a0, ui_b64, ui_b0, status);
            }
            return Float128 {
                v64: ui_a64,
                v0: ui_a0,
            };
        }
        exp_z = exp_a;
        if exp_b != 0 {
            sig_b.0 |= 0x0001000000000000;
        } else {
            let new_diff = exp_diff - 1;
            if new_diff == 0 {
                // newlyAligned
                let (sig_z64, sig_z0) =
                    add128(sig_a.0 | 0x0001000000000000, sig_a.1, sig_b.0, sig_b.1);
                exp_z -= 1;
                if sig_z64 < 0x0002000000000000 {
                    return round_pack_to_f128(sign_z, exp_z, sig_z64, sig_z0, 0, status);
                }
                exp_z += 1;
                let (z64, z0, z_extra) = short_shift_right_jam128_extra(sig_z64, sig_z0, 0, 1);
                return round_pack_to_f128(sign_z, exp_z, z64, z0, z_extra, status);
            }
        }
        let (new64, new0, new_extra) =
            shift_right_jam128_extra(sig_b.0, sig_b.1, 0, exp_diff as u32);
        sig_b = (new64, new0);
        sig_z_extra = new_extra;
    }

    // newlyAligned (common path)
    let (sig_z64, sig_z0) = add128(sig_a.0 | 0x0001000000000000, sig_a.1, sig_b.0, sig_b.1);
    exp_z -= 1;
    if sig_z64 < 0x0002000000000000 {
        return round_pack_to_f128(sign_z, exp_z, sig_z64, sig_z0, sig_z_extra, status);
    }
    exp_z += 1;
    // shiftRight1
    let (z64, z0, z_extra) = short_shift_right_jam128_extra(sig_z64, sig_z0, sig_z_extra, 1);
    round_pack_to_f128(sign_z, exp_z, z64, z0, z_extra, status)
}

fn sub_mags_f128(
    ui_a64: u64,
    ui_a0: u64,
    ui_b64: u64,
    ui_b0: u64,
    mut sign_z: bool,
    status: &mut SoftFloatStatus,
) -> Float128 {
    let exp_a = exp_f128_ui64(ui_a64);
    let mut sig_a = short_shift_left128(frac_f128_ui64(ui_a64), ui_a0, 4);
    let exp_b = exp_f128_ui64(ui_b64);
    let mut sig_b = short_shift_left128(frac_f128_ui64(ui_b64), ui_b0, 4);
    let exp_diff = exp_a - exp_b;

    if exp_diff > 0 {
        // expABigger
        if exp_a == 0x7FFF {
            if (sig_a.0 | sig_a.1) != 0 {
                return softfloat_propagate_nan_f128_ui(ui_a64, ui_a0, ui_b64, ui_b0, status);
            }
            return Float128 {
                v64: ui_a64,
                v0: ui_a0,
            };
        }
        if exp_b != 0 {
            sig_b.0 |= 0x0010000000000000;
        } else {
            let new_diff = exp_diff - 1;
            if new_diff == 0 {
                // newlyAlignedABigger
                let exp_z = exp_a;
                sig_a.0 |= 0x0010000000000000;
                let (z64, z0) = sub128(sig_a.0, sig_a.1, sig_b.0, sig_b.1);
                return norm_round_pack_to_f128(sign_z, exp_z - 5, z64, z0, status);
            }
        }
        sig_b = shift_right_jam128(sig_b.0, sig_b.1, exp_diff as u32);
        // newlyAlignedABigger
        let exp_z = exp_a;
        sig_a.0 |= 0x0010000000000000;
        let (z64, z0) = sub128(sig_a.0, sig_a.1, sig_b.0, sig_b.1);
        return norm_round_pack_to_f128(sign_z, exp_z - 5, z64, z0, status);
    }

    if exp_diff < 0 {
        // expBBigger
        if exp_b == 0x7FFF {
            if (sig_b.0 | sig_b.1) != 0 {
                return softfloat_propagate_nan_f128_ui(ui_a64, ui_a0, ui_b64, ui_b0, status);
            }
            return Float128 {
                v64: pack_to_f128_ui64(!sign_z, 0x7FFF, 0),
                v0: 0,
            };
        }
        if exp_a != 0 {
            sig_a.0 |= 0x0010000000000000;
        } else {
            let new_diff = exp_diff + 1;
            if new_diff == 0 {
                // newlyAlignedBBigger
                let exp_z = exp_b;
                sig_b.0 |= 0x0010000000000000;
                sign_z = !sign_z;
                let (z64, z0) = sub128(sig_b.0, sig_b.1, sig_a.0, sig_a.1);
                return norm_round_pack_to_f128(sign_z, exp_z - 5, z64, z0, status);
            }
        }
        sig_a = shift_right_jam128(sig_a.0, sig_a.1, (-exp_diff) as u32);
        // newlyAlignedBBigger
        let exp_z = exp_b;
        sig_b.0 |= 0x0010000000000000;
        sign_z = !sign_z;
        let (z64, z0) = sub128(sig_b.0, sig_b.1, sig_a.0, sig_a.1);
        return norm_round_pack_to_f128(sign_z, exp_z - 5, z64, z0, status);
    }

    // exp_diff == 0
    if exp_a == 0x7FFF {
        if (sig_a.0 | sig_a.1 | sig_b.0 | sig_b.1) != 0 {
            return softfloat_propagate_nan_f128_ui(ui_a64, ui_a0, ui_b64, ui_b0, status);
        }
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOAT128_DEFAULT_NAN;
    }
    let mut exp_z = exp_a;
    if exp_z == 0 {
        exp_z = 1;
    }
    if sig_b.0 < sig_a.0 {
        // aBigger
        let (z64, z0) = sub128(sig_a.0, sig_a.1, sig_b.0, sig_b.1);
        return norm_round_pack_to_f128(sign_z, exp_z - 5, z64, z0, status);
    }
    if sig_a.0 < sig_b.0 {
        // bBigger
        sign_z = !sign_z;
        let (z64, z0) = sub128(sig_b.0, sig_b.1, sig_a.0, sig_a.1);
        return norm_round_pack_to_f128(sign_z, exp_z - 5, z64, z0, status);
    }
    if sig_b.1 < sig_a.1 {
        let (z64, z0) = sub128(sig_a.0, sig_a.1, sig_b.0, sig_b.1);
        return norm_round_pack_to_f128(sign_z, exp_z - 5, z64, z0, status);
    }
    if sig_a.1 < sig_b.1 {
        sign_z = !sign_z;
        let (z64, z0) = sub128(sig_b.0, sig_b.1, sig_a.0, sig_a.1);
        return norm_round_pack_to_f128(sign_z, exp_z - 5, z64, z0, status);
    }
    // exact zero
    Float128 {
        v64: pack_to_f128_ui64(softfloat_getRoundingMode(status) == ROUND_MIN, 0, 0),
        v0: 0,
    }
}

// ============================================================
// f128_add / f128_sub
// ============================================================

/// Float128 addition.
pub(crate) fn f128_add(a: Float128, b: Float128, status: &mut SoftFloatStatus) -> Float128 {
    let sign_a = sign_f128_ui64(a.v64);
    let sign_b = sign_f128_ui64(b.v64);
    if sign_a == sign_b {
        add_mags_f128(a.v64, a.v0, b.v64, b.v0, sign_a, status)
    } else {
        sub_mags_f128(a.v64, a.v0, b.v64, b.v0, sign_a, status)
    }
}

/// Float128 subtraction.
pub(crate) fn f128_sub(a: Float128, b: Float128, status: &mut SoftFloatStatus) -> Float128 {
    let sign_a = sign_f128_ui64(a.v64);
    let sign_b = sign_f128_ui64(b.v64);
    if sign_a == sign_b {
        sub_mags_f128(a.v64, a.v0, b.v64, b.v0, sign_a, status)
    } else {
        add_mags_f128(a.v64, a.v0, b.v64, b.v0, sign_a, status)
    }
}

// ============================================================
// f128_mul
// ============================================================

/// Float128 multiplication.
pub(crate) fn f128_mul(a: Float128, b: Float128, status: &mut SoftFloatStatus) -> Float128 {
    let ui_a64 = a.v64;
    let ui_a0 = a.v0;
    let sign_a = sign_f128_ui64(ui_a64);
    let exp_a = exp_f128_ui64(ui_a64);
    let mut sig_a = (frac_f128_ui64(ui_a64), ui_a0);

    let ui_b64 = b.v64;
    let ui_b0 = b.v0;
    let sign_b = sign_f128_ui64(ui_b64);
    let mut exp_b = exp_f128_ui64(ui_b64);
    let mut sig_b = (frac_f128_ui64(ui_b64), ui_b0);

    let sign_z = sign_a ^ sign_b;

    // Handle NaN/Inf for A
    if exp_a == 0x7FFF {
        if (sig_a.0 | sig_a.1) != 0 || ((exp_b == 0x7FFF) && (sig_b.0 | sig_b.1) != 0) {
            return softfloat_propagate_nan_f128_ui(ui_a64, ui_a0, ui_b64, ui_b0, status);
        }
        let mag_bits = (exp_b as u64) | sig_b.0 | sig_b.1;
        if mag_bits == 0 {
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOAT128_DEFAULT_NAN;
        }
        return Float128 {
            v64: pack_to_f128_ui64(sign_z, 0x7FFF, 0),
            v0: 0,
        };
    }
    // Handle NaN/Inf for B
    if exp_b == 0x7FFF {
        if (sig_b.0 | sig_b.1) != 0 {
            return softfloat_propagate_nan_f128_ui(ui_a64, ui_a0, ui_b64, ui_b0, status);
        }
        let mag_bits = (exp_a as u64) | sig_a.0 | sig_a.1;
        if mag_bits == 0 {
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOAT128_DEFAULT_NAN;
        }
        return Float128 {
            v64: pack_to_f128_ui64(sign_z, 0x7FFF, 0),
            v0: 0,
        };
    }

    // Handle subnormals
    let mut exp_a = exp_a;
    if exp_a == 0 {
        if (sig_a.0 | sig_a.1) == 0 {
            return Float128 {
                v64: pack_to_f128_ui64(sign_z, 0, 0),
                v0: 0,
            };
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f128_sig(sig_a.0, sig_a.1);
        exp_a = norm.exp;
        sig_a = (norm.sig_v64, norm.sig_v0);
    }
    if exp_b == 0 {
        if (sig_b.0 | sig_b.1) == 0 {
            return Float128 {
                v64: pack_to_f128_ui64(sign_z, 0, 0),
                v0: 0,
            };
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f128_sig(sig_b.0, sig_b.1);
        exp_b = norm.exp;
        sig_b = (norm.sig_v64, norm.sig_v0);
    }

    let mut exp_z = exp_a + exp_b - 0x4000;
    sig_a.0 |= 0x0001000000000000;
    sig_b = short_shift_left128(sig_b.0, sig_b.1, 16);
    let sig256z = mul128_to_256(sig_a.0, sig_a.1, sig_b.0, sig_b.1);
    let mut sig_z_extra = sig256z[1] | ((sig256z[0] != 0) as u64);
    let (mut sig_z64, mut sig_z0) = add128(sig256z[3], sig256z[2], sig_a.0, sig_a.1);

    if 0x0002000000000000 <= sig_z64 {
        exp_z += 1;
        let (new64, new0, new_extra) =
            short_shift_right_jam128_extra(sig_z64, sig_z0, sig_z_extra, 1);
        sig_z64 = new64;
        sig_z0 = new0;
        sig_z_extra = new_extra;
    }
    round_pack_to_f128(sign_z, exp_z, sig_z64, sig_z0, sig_z_extra, status)
}

// ============================================================
// f128_div
// ============================================================

/// Float128 division.
pub(crate) fn f128_div(a: Float128, b: Float128, status: &mut SoftFloatStatus) -> Float128 {
    let ui_a64 = a.v64;
    let ui_a0 = a.v0;
    let sign_a = sign_f128_ui64(ui_a64);
    let exp_a = exp_f128_ui64(ui_a64);
    let mut sig_a = (frac_f128_ui64(ui_a64), ui_a0);

    let ui_b64 = b.v64;
    let ui_b0 = b.v0;
    let sign_b = sign_f128_ui64(ui_b64);
    let mut exp_b = exp_f128_ui64(ui_b64);
    let mut sig_b = (frac_f128_ui64(ui_b64), ui_b0);

    let sign_z = sign_a ^ sign_b;

    // Handle NaN/Inf for A
    if exp_a == 0x7FFF {
        if (sig_a.0 | sig_a.1) != 0 {
            return softfloat_propagate_nan_f128_ui(ui_a64, ui_a0, ui_b64, ui_b0, status);
        }
        if exp_b == 0x7FFF {
            if (sig_b.0 | sig_b.1) != 0 {
                return softfloat_propagate_nan_f128_ui(ui_a64, ui_a0, ui_b64, ui_b0, status);
            }
            // Inf / Inf = invalid
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOAT128_DEFAULT_NAN;
        }
        return Float128 {
            v64: pack_to_f128_ui64(sign_z, 0x7FFF, 0),
            v0: 0,
        };
    }
    // Handle NaN/Inf for B
    if exp_b == 0x7FFF {
        if (sig_b.0 | sig_b.1) != 0 {
            return softfloat_propagate_nan_f128_ui(ui_a64, ui_a0, ui_b64, ui_b0, status);
        }
        // A / Inf = 0
        return Float128 {
            v64: pack_to_f128_ui64(sign_z, 0, 0),
            v0: 0,
        };
    }

    // Handle subnormals / zeros
    let mut exp_a = exp_a;
    if exp_b == 0 {
        if (sig_b.0 | sig_b.1) == 0 {
            if (exp_a as u64 | sig_a.0 | sig_a.1) == 0 {
                // 0/0 = invalid
                softfloat_raiseFlags(status, FLAG_INVALID);
                return FLOAT128_DEFAULT_NAN;
            }
            // A/0 = inf
            softfloat_raiseFlags(status, FLAG_DIVBYZERO);
            return Float128 {
                v64: pack_to_f128_ui64(sign_z, 0x7FFF, 0),
                v0: 0,
            };
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f128_sig(sig_b.0, sig_b.1);
        exp_b = norm.exp;
        sig_b = (norm.sig_v64, norm.sig_v0);
    }
    if exp_a == 0 {
        if (sig_a.0 | sig_a.1) == 0 {
            return Float128 {
                v64: pack_to_f128_ui64(sign_z, 0, 0),
                v0: 0,
            };
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f128_sig(sig_a.0, sig_a.1);
        exp_a = norm.exp;
        sig_a = (norm.sig_v64, norm.sig_v0);
    }

    let mut exp_z = exp_a - exp_b + 0x3FFE;
    sig_a.0 |= 0x0001000000000000;
    sig_b.0 |= 0x0001000000000000;

    let mut rem = sig_a;
    if lt128(sig_a.0, sig_a.1, sig_b.0, sig_b.1) {
        exp_z -= 1;
        rem = add128(sig_a.0, sig_a.1, sig_a.0, sig_a.1);
    }

    let recip32 = approx_recip32_1((sig_b.0 >> 17) as u32);
    let mut qs = [0u32; 3];

    // Three iterations of the division loop
    for ix in (0..3).rev() {
        let q64 = ((rem.0 >> 19) as u32 as u64).wrapping_mul(recip32 as u64);
        let mut q = ((q64.wrapping_add(0x80000000)) >> 32) as u32;
        rem = short_shift_left128(rem.0, rem.1, 29);
        let term = mul128_by_32(sig_b.0, sig_b.1, q);
        rem = sub128(rem.0, rem.1, term.0, term.1);
        if (rem.0 & 0x8000000000000000) != 0 {
            q = q.wrapping_sub(1);
            rem = add128(rem.0, rem.1, sig_b.0, sig_b.1);
        }
        qs[ix] = q;
    }

    // Final refinement iteration
    let q64 = ((rem.0 >> 19) as u32 as u64).wrapping_mul(recip32 as u64);
    let mut q = ((q64.wrapping_add(0x80000000)) >> 32) as u32;

    if ((q.wrapping_add(1)) & 7) < 2 {
        rem = short_shift_left128(rem.0, rem.1, 29);
        let term = mul128_by_32(sig_b.0, sig_b.1, q);
        rem = sub128(rem.0, rem.1, term.0, term.1);
        if (rem.0 & 0x8000000000000000) != 0 {
            q = q.wrapping_sub(1);
            rem = add128(rem.0, rem.1, sig_b.0, sig_b.1);
        } else if le128(sig_b.0, sig_b.1, rem.0, rem.1) {
            q = q.wrapping_add(1);
            rem = sub128(rem.0, rem.1, sig_b.0, sig_b.1);
        }
        if (rem.0 | rem.1) != 0 {
            q |= 1;
        }
    }

    let sig_z_extra = (q as u64) << 60;
    let term = short_shift_left128(0, qs[1] as u64, 54);
    let sig_z = add128(
        (qs[2] as u64) << 19,
        ((qs[0] as u64) << 25).wrapping_add((q as u64) >> 4),
        term.0,
        term.1,
    );
    round_pack_to_f128(sign_z, exp_z, sig_z.0, sig_z.1, sig_z_extra, status)
}

// ============================================================
// f128_mulAdd (fused multiply-add)
// ============================================================

/// Fused multiply-add operation flags (matches Bochs enum).
pub const SOFTFLOAT_MULADD_SUB_C: u8 = 1; // negate addend
pub const SOFTFLOAT_MULADD_SUB_PROD: u8 = 2; // negate product

/// Float128 fused multiply-add: a*b + c (with operation modifier).
/// op=0: a*b+c, op=1: a*b-c, op=2: -(a*b)+c, op=3: -(a*b)-c
pub(crate) fn f128_mul_add(
    a: Float128,
    b: Float128,
    c: Float128,
    op: u8,
    status: &mut SoftFloatStatus,
) -> Float128 {
    let ui_a64 = a.v64;
    let ui_a0 = a.v0;
    let ui_b64 = b.v64;
    let ui_b0 = b.v0;
    let ui_c64 = c.v64;
    let ui_c0 = c.v0;

    let sign_a = sign_f128_ui64(ui_a64);
    let mut exp_a = exp_f128_ui64(ui_a64);
    let mut sig_a = (frac_f128_ui64(ui_a64), ui_a0);

    let sign_b = sign_f128_ui64(ui_b64);
    let mut exp_b = exp_f128_ui64(ui_b64);
    let mut sig_b = (frac_f128_ui64(ui_b64), ui_b0);

    let sign_c = sign_f128_ui64(ui_c64) ^ ((op & SOFTFLOAT_MULADD_SUB_C) != 0);
    let mut exp_c = exp_f128_ui64(ui_c64);
    let mut sig_c = (frac_f128_ui64(ui_c64), ui_c0);

    let mut sign_z = sign_a ^ sign_b ^ ((op & SOFTFLOAT_MULADD_SUB_PROD) != 0);

    // Handle NaN/Inf for A
    if exp_a == 0x7FFF {
        if (sig_a.0 | sig_a.1) != 0 || ((exp_b == 0x7FFF) && (sig_b.0 | sig_b.1) != 0) {
            let uiz = softfloat_propagate_nan_f128_ui(ui_a64, ui_a0, ui_b64, ui_b0, status);
            return softfloat_propagate_nan_f128_ui(uiz.v64, uiz.v0, ui_c64, ui_c0, status);
        }
        let mag_bits = (exp_b as u64) | sig_b.0 | sig_b.1;
        if (sig_c.0 | sig_c.1) != 0 && exp_c == 0x7FFF {
            let uiz = Float128 { v64: 0, v0: 0 };
            return softfloat_propagate_nan_f128_ui(uiz.v64, uiz.v0, ui_c64, ui_c0, status);
        }
        if mag_bits != 0 {
            let uiz = Float128 {
                v64: pack_to_f128_ui64(sign_z, 0x7FFF, 0),
                v0: 0,
            };
            if exp_c != 0x7FFF {
                return uiz;
            }
            if sign_z == sign_c {
                return uiz;
            }
        }
        softfloat_raiseFlags(status, FLAG_INVALID);
        let uiz = FLOAT128_DEFAULT_NAN;
        return softfloat_propagate_nan_f128_ui(uiz.v64, uiz.v0, ui_c64, ui_c0, status);
    }
    // Handle NaN/Inf for B
    if exp_b == 0x7FFF {
        if (sig_b.0 | sig_b.1) != 0 {
            let uiz = softfloat_propagate_nan_f128_ui(ui_a64, ui_a0, ui_b64, ui_b0, status);
            return softfloat_propagate_nan_f128_ui(uiz.v64, uiz.v0, ui_c64, ui_c0, status);
        }
        let mag_bits = (exp_a as u64) | sig_a.0 | sig_a.1;
        if (sig_c.0 | sig_c.1) != 0 && exp_c == 0x7FFF {
            let uiz = Float128 { v64: 0, v0: 0 };
            return softfloat_propagate_nan_f128_ui(uiz.v64, uiz.v0, ui_c64, ui_c0, status);
        }
        if mag_bits != 0 {
            let uiz = Float128 {
                v64: pack_to_f128_ui64(sign_z, 0x7FFF, 0),
                v0: 0,
            };
            if exp_c != 0x7FFF {
                return uiz;
            }
            if sign_z == sign_c {
                return uiz;
            }
        }
        softfloat_raiseFlags(status, FLAG_INVALID);
        let uiz = FLOAT128_DEFAULT_NAN;
        return softfloat_propagate_nan_f128_ui(uiz.v64, uiz.v0, ui_c64, ui_c0, status);
    }
    // Handle NaN/Inf for C
    if exp_c == 0x7FFF {
        if (sig_c.0 | sig_c.1) != 0 {
            let uiz = Float128 { v64: 0, v0: 0 };
            return softfloat_propagate_nan_f128_ui(uiz.v64, uiz.v0, ui_c64, ui_c0, status);
        }
        return Float128 {
            v64: ui_c64,
            v0: ui_c0,
        };
    }

    // Handle subnormals for A and B
    if exp_a == 0 {
        if (sig_a.0 | sig_a.1) == 0 {
            // zeroProd
            let uiz = Float128 {
                v64: ui_c64,
                v0: ui_c0,
            };
            if (exp_c as u64 | sig_c.0 | sig_c.1) == 0 && sign_z != sign_c {
                return Float128 {
                    v64: pack_to_f128_ui64(softfloat_getRoundingMode(status) == ROUND_MIN, 0, 0),
                    v0: 0,
                };
            }
            return uiz;
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f128_sig(sig_a.0, sig_a.1);
        exp_a = norm.exp;
        sig_a = (norm.sig_v64, norm.sig_v0);
    }
    if exp_b == 0 {
        if (sig_b.0 | sig_b.1) == 0 {
            // zeroProd
            let uiz = Float128 {
                v64: ui_c64,
                v0: ui_c0,
            };
            if (exp_c as u64 | sig_c.0 | sig_c.1) == 0 && sign_z != sign_c {
                return Float128 {
                    v64: pack_to_f128_ui64(softfloat_getRoundingMode(status) == ROUND_MIN, 0, 0),
                    v0: 0,
                };
            }
            return uiz;
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f128_sig(sig_b.0, sig_b.1);
        exp_b = norm.exp;
        sig_b = (norm.sig_v64, norm.sig_v0);
    }

    // Compute product A*B
    let mut exp_z = exp_a + exp_b - 0x3FFE;
    sig_a.0 |= 0x0001000000000000;
    sig_b.0 |= 0x0001000000000000;
    sig_a = short_shift_left128(sig_a.0, sig_a.1, 8);
    sig_b = short_shift_left128(sig_b.0, sig_b.1, 15);
    let mut sig256z = mul128_to_256(sig_a.0, sig_a.1, sig_b.0, sig_b.1);
    let mut sig_z = (sig256z[3], sig256z[2]);
    let mut shift_dist: i32 = 0;
    if (sig_z.0 & 0x0100000000000000) == 0 {
        exp_z -= 1;
        shift_dist = -1;
    }

    // Handle subnormal C
    if exp_c == 0 {
        if (sig_c.0 | sig_c.1) == 0 {
            shift_dist += 8;
            // goto sigZ path
            let sig_z_extra = sig256z[1] | sig256z[0];
            let sig_z_extra_jammed =
                (sig_z.1 << (64 - shift_dist as u32)) | ((sig_z_extra != 0) as u64);
            let (z64, z0) = short_shift_right128(sig_z.0, sig_z.1, shift_dist as u8);
            return round_pack_to_f128(sign_z, exp_z - 1, z64, z0, sig_z_extra_jammed, status);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f128_sig(sig_c.0, sig_c.1);
        exp_c = norm.exp;
        sig_c = (norm.sig_v64, norm.sig_v0);
    }
    sig_c.0 |= 0x0001000000000000;
    sig_c = short_shift_left128(sig_c.0, sig_c.1, 8);

    // Align product and addend
    let exp_diff = exp_z - exp_c;

    // The full mulAdd logic is complex with many branches. For the Bochs
    // use case (transcendentals with near-equal exponents), we implement
    // a simplified but correct version that handles the common cases.
    //
    // For same-sign addition, we add and shift. For different-sign, we
    // subtract the smaller from the larger.

    if sign_z == sign_c {
        // Same sign: add magnitudes
        if exp_diff <= 0 {
            // C is larger or equal exponent
            exp_z = exp_c;
            if exp_diff < 0 {
                // Shift product right by |exp_diff|
                let total_shift = (-exp_diff) + shift_dist;
                if total_shift > 0 {
                    // Shift sig_z right (absorbing sig256z low words)
                    let (new64, new0) = if total_shift < 128 {
                        shift_right_jam128(sig_z.0, sig_z.1, total_shift as u32)
                    } else {
                        (0, ((sig_z.0 | sig_z.1) != 0) as u64)
                    };
                    sig_z = (new64, new0);
                } else if total_shift < 0 {
                    // Need to shift sig_z left (rare case: shift_dist=-1, exp_diff=0)
                    sig_z = short_shift_left128(sig_z.0, sig_z.1, (-total_shift) as u8);
                }
            } else if shift_dist != 0 {
                // exp_diff == 0 but shift_dist = -1
                sig_z = short_shift_left128(sig_z.0, sig_z.1, 1);
            }
            let (zz64, zz0) = add128(sig_c.0, sig_c.1, sig_z.0, sig_z.1);
            let mut sd = 8;
            if (zz64 & 0x0200000000000000) != 0 {
                exp_z += 1;
                sd = 9;
            }
            let sig_z_extra = sig256z[1] | sig256z[0];
            let extra_jammed = (zz0 << (64 - sd as u32)) | ((sig_z_extra != 0) as u64);
            let (z64, z0) = short_shift_right128(zz64, zz0, sd as u8);
            round_pack_to_f128(sign_z, exp_z - 1, z64, z0, extra_jammed, status)
        } else {
            // Product has larger exponent
            if shift_dist != 0 {
                // Double sig256z (shift left by 1)
                // sig256z = sig256z + sig256z
                let mut carry = 0u64;
                for elem in sig256z.iter_mut() {
                    let sum = (*elem as u128) * 2 + carry as u128;
                    *elem = sum as u64;
                    carry = (sum >> 64) as u64;
                }
                sig_z = (sig256z[3], sig256z[2]);
            }
            if exp_diff == 0 {
                // Already aligned
            } else {
                // Shift C right by exp_diff
                let (new64, new0) = if exp_diff < 128 {
                    shift_right_jam128(sig_c.0, sig_c.1, exp_diff as u32)
                } else {
                    (0, ((sig_c.0 | sig_c.1) != 0) as u64)
                };
                sig_c = (new64, new0);
            }
            let (zz64, zz0) = add128(sig_z.0, sig_z.1, sig_c.0, sig_c.1);
            let mut sd = 8;
            if (zz64 & 0x0200000000000000) != 0 {
                exp_z += 1;
                sd = 9;
            }
            let sig_z_extra = sig256z[1] | sig256z[0];
            let extra_jammed = (zz0 << (64 - sd as u32)) | ((sig_z_extra != 0) as u64);
            let (z64, z0) = short_shift_right128(zz64, zz0, sd as u8);
            round_pack_to_f128(sign_z, exp_z - 1, z64, z0, extra_jammed, status)
        }
    } else {
        // Different signs: subtract magnitudes
        if exp_diff < 0 {
            // C has larger exponent — result sign is sign_c
            sign_z = sign_c;
            exp_z = exp_c;
            let total_shift = (-exp_diff) + shift_dist;
            if total_shift > 0 {
                let (new64, new0) = if total_shift < 128 {
                    shift_right_jam128(sig_z.0, sig_z.1, total_shift as u32)
                } else {
                    (0, ((sig_z.0 | sig_z.1) != 0) as u64)
                };
                sig_z = (new64, new0);
            } else if total_shift < 0 {
                sig_z = short_shift_left128(sig_z.0, sig_z.1, (-total_shift) as u8);
            }
            let (mut zz64, mut zz0) = sub128(sig_c.0, sig_c.1, sig_z.0, sig_z.1);
            // Account for extra low bits
            let sig_z_extra = sig256z[1] | sig256z[0];
            if sig_z_extra != 0 && total_shift > 0 {
                let (sub64, sub0) = sub128(zz64, zz0, 0, 1);
                zz64 = sub64;
                zz0 = sub0;
            }
            // Normalize
            norm_round_pack_to_f128(sign_z, exp_z - 1 + 8 - 15, zz64, zz0, status)
        } else if exp_diff == 0 {
            // Same exponent — subtract and figure out which is larger
            if shift_dist != 0 {
                sig_z = short_shift_left128(sig_z.0, sig_z.1, 1);
            }
            let (mut zz64, mut zz0) = sub128(sig_z.0, sig_z.1, sig_c.0, sig_c.1);
            if (zz64 | zz0) == 0 && sig256z[1] == 0 && sig256z[0] == 0 {
                // Complete cancellation
                return Float128 {
                    v64: pack_to_f128_ui64(softfloat_getRoundingMode(status) == ROUND_MIN, 0, 0),
                    v0: 0,
                };
            }
            if (zz64 & 0x8000000000000000) != 0 {
                sign_z = !sign_z;
                let (neg64, neg0) = sub128(0, 0, zz64, zz0);
                zz64 = neg64;
                zz0 = neg0;
            }
            norm_round_pack_to_f128(sign_z, exp_z - 1 + 8 - 15, zz64, zz0, status)
        } else {
            // Product has larger exponent
            if shift_dist != 0 {
                let mut carry = 0u64;
                for elem in sig256z.iter_mut() {
                    let sum = (*elem as u128) * 2 + carry as u128;
                    *elem = sum as u64;
                    carry = (sum >> 64) as u64;
                }
                sig_z = (sig256z[3], sig256z[2]);
            }
            // Shift C right by exp_diff
            let (new64, new0) = if exp_diff < 128 {
                shift_right_jam128(sig_c.0, sig_c.1, exp_diff as u32)
            } else {
                (0, ((sig_c.0 | sig_c.1) != 0) as u64)
            };
            sig_c = (new64, new0);
            let (zz64, zz0) = sub128(sig_z.0, sig_z.1, sig_c.0, sig_c.1);
            let mut sd = 8;
            if (zz64 & 0x0100000000000000) == 0 {
                sd = 7;
                exp_z -= 1;
            }
            let sig_z_extra = sig256z[1] | sig256z[0];
            let extra_jammed = (zz0 << (64 - sd as u32)) | ((sig_z_extra != 0) as u64);
            let (z64, z0) = short_shift_right128(zz64, zz0, sd as u8);
            round_pack_to_f128(sign_z, exp_z - 1, z64, z0, extra_jammed, status)
        }
    }
}

// ============================================================
// Conversions: extf80 <-> Float128
// ============================================================

/// Convert extFloat80 (80-bit extended precision) to Float128.
pub(crate) fn extf80_to_f128(a: floatx80, status: &mut SoftFloatStatus) -> Float128 {
    let ui_a64 = a.sign_exp;
    let ui_a0 = a.signif;

    // Handle unsupported encodings
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOAT128_DEFAULT_NAN;
    }

    let exp = ui_a64 & 0x7FFF;
    let frac = ui_a0 & 0x7FFFFFFFFFFFFFFF;

    if exp == 0x7FFF && frac != 0 {
        // NaN — convert through common NaN representation
        // Quieten signaling NaN
        softfloat_raiseFlags(status, FLAG_INVALID);
        let sign = sign_f128_ui64((ui_a64 as u64) << 48);
        let mut v64 = pack_to_f128_ui64(sign, 0x7FFF, 0);
        v64 |= 0x0000800000000000; // set quiet bit
        return Float128 { v64, v0: 0 };
    }

    let sign = (ui_a64 >> 15) != 0;
    let frac128 = short_shift_left128(0, frac, 49);
    Float128 {
        v64: pack_to_f128_ui64(sign, exp as i32, frac128.0),
        v0: frac128.1,
    }
}

/// Convert Float128 to extFloat80 (80-bit extended precision).
pub(crate) fn f128_to_extf80(a: Float128, status: &mut SoftFloatStatus) -> floatx80 {
    let ui_a64 = a.v64;
    let ui_a0 = a.v0;
    let sign = sign_f128_ui64(ui_a64);
    let exp = exp_f128_ui64(ui_a64);
    let mut frac64 = frac_f128_ui64(ui_a64);
    let frac0 = ui_a0;

    if exp == 0x7FFF {
        if (frac64 | frac0) != 0 {
            // NaN
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOATX80_DEFAULT_NAN;
        }
        // Infinity
        let sign_exp = pack_to_extf80_sign_exp(sign, 0x7FFF);
        return floatx80::new(sign_exp, 0x8000000000000000);
    }

    if exp == 0 {
        if (frac64 | frac0) == 0 {
            let sign_exp = pack_to_extf80_sign_exp(sign, 0);
            return floatx80::new(sign_exp, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f128_sig(frac64, frac0);
        let exp = norm.exp;
        frac64 = norm.sig_v64;
        let frac0 = norm.sig_v0;
        let sig128 = short_shift_left128(frac64 | 0x0001000000000000, frac0, 15);
        return round_pack_to_extf80(sign, exp, sig128.0, sig128.1, 80, status);
    }

    let sig128 = short_shift_left128(frac64 | 0x0001000000000000, frac0, 15);
    round_pack_to_extf80(sign, exp, sig128.0, sig128.1, 80, status)
}

// ============================================================
// Conversion: i32 -> Float128
// ============================================================

/// Convert a signed 32-bit integer to Float128.
pub(crate) fn i32_to_f128(a: i32) -> Float128 {
    if a == 0 {
        return FLOAT128_POSITIVE_ZERO;
    }
    let sign = a < 0;
    let abs_a = if sign { (-(a as i64)) as u32 } else { a as u32 };
    let shift_dist = count_leading_zeros32(abs_a) + 17;
    let v64 = pack_to_f128_ui64(
        sign,
        0x402E - shift_dist as i32,
        (abs_a as u64) << shift_dist,
    );
    Float128 { v64, v0: 0 }
}

/// Convert a signed 64-bit integer to Float128.
pub(crate) fn i64_to_f128(a: i64) -> Float128 {
    if a == 0 {
        return FLOAT128_POSITIVE_ZERO;
    }
    let sign = a < 0;
    let abs_a = if sign {
        if a == i64::MIN {
            a as u64 // 0x8000000000000000
        } else {
            (-a) as u64
        }
    } else {
        a as u64
    };
    let shift_dist = count_leading_zeros64(abs_a) as i32;
    // f128 exponent for value 2^63 is 0x3FFF + 63 = 0x403E
    let exp = 0x403E - shift_dist;
    if shift_dist >= 49 {
        // Entire significand fits in v64
        let v64 = pack_to_f128_ui64(sign, exp, abs_a << (shift_dist - 49));
        Float128 { v64, v0: 0 }
    } else if shift_dist >= 0 {
        let (hi, lo) = short_shift_left128(0, abs_a, (shift_dist + 15) as u8);
        // hi has the fraction bits that go in v64, lo has the rest
        Float128 {
            v64: pack_to_f128_ui64(sign, exp, hi),
            v0: lo,
        }
    } else {
        // shift_dist < 0 shouldn't happen for valid abs_a > 0
        Float128 {
            v64: pack_to_f128_ui64(sign, exp, abs_a >> (-shift_dist)),
            v0: abs_a << (64 + shift_dist),
        }
    }
}

// ============================================================
// Float128 sign manipulation helpers
// ============================================================

/// Negate a Float128 value (flip the sign bit).
#[inline]
pub(crate) fn f128_chs(a: Float128) -> Float128 {
    Float128 {
        v64: a.v64 ^ 0x8000000000000000,
        v0: a.v0,
    }
}

/// Absolute value of a Float128 (clear the sign bit).
#[inline]
pub(crate) fn f128_abs(a: Float128) -> Float128 {
    Float128 {
        v64: a.v64 & 0x7FFFFFFFFFFFFFFF,
        v0: a.v0,
    }
}

/// Check if a Float128 is zero (positive or negative).
#[inline]
pub(crate) fn f128_is_zero(a: Float128) -> bool {
    ((a.v64 & 0x7FFFFFFFFFFFFFFF) | a.v0) == 0
}

/// Check if a Float128 is NaN.
#[inline]
pub(crate) fn f128_is_nan(a: Float128) -> bool {
    is_nan_f128_ui(a.v64, a.v0)
}

/// Check if a Float128 is infinity.
#[inline]
pub(crate) fn f128_is_inf(a: Float128) -> bool {
    ((a.v64 & 0x7FFFFFFFFFFFFFFF) == 0x7FFF000000000000) && a.v0 == 0
}

/// Compare Float128 values: return true if a < b (unsigned magnitude comparison).
#[inline]
pub(crate) fn f128_lt_mag(a: Float128, b: Float128) -> bool {
    let a_mag64 = a.v64 & 0x7FFFFFFFFFFFFFFF;
    let b_mag64 = b.v64 & 0x7FFFFFFFFFFFFFFF;
    lt128(a_mag64, a.v0, b_mag64, b.v0)
}

// ============================================================
// 256-bit helpers (used by mulAdd)
// ============================================================

/// Shift a 256-bit value (stored as [u64; 4], little-endian) right by `dist` with jam.
pub(crate) fn shift_right_jam_256m(a: &[u64; 4], dist: i32) -> [u64; 4] {
    if dist <= 0 {
        return *a;
    }
    let val = ((a[3] as u128) << 64) | (a[2] as u128);
    let val_lo = ((a[1] as u128) << 64) | (a[0] as u128);
    if dist < 64 {
        let d = dist as u32;
        let (z3, z2, z1, z0) = shift_right_jam256(a[3], a[2], a[1], a[0], d);
        [z0, z1, z2, z3]
    } else {
        // For larger shifts, collapse everything
        let combined = (val != 0) || (val_lo != 0);
        [combined as u64, 0, 0, 0]
    }
}

/// Add two 256-bit values stored as [u64; 4] in little-endian word order.
pub(crate) fn add_256m(a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
    let mut result = [0u64; 4];
    let mut carry = 0u64;
    for i in 0..4 {
        let sum = (a[i] as u128) + (b[i] as u128) + (carry as u128);
        result[i] = sum as u64;
        carry = (sum >> 64) as u64;
    }
    result
}

/// Subtract two 256-bit values stored as [u64; 4] in little-endian word order.
pub(crate) fn sub_256m(a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
    let mut result = [0u64; 4];
    let mut borrow = 0u64;
    for i in 0..4 {
        let diff = (a[i] as u128)
            .wrapping_sub(b[i] as u128)
            .wrapping_sub(borrow as u128);
        result[i] = diff as u64;
        borrow = if (a[i] as u128) < (b[i] as u128) + (borrow as u128) {
            1
        } else {
            0
        };
    }
    result
}

// ============================================================
// PI/2 and related Float128 constants for argument reduction
// ============================================================

/// PI as Float128 (exponent 0x4000, fraction from FLOAT_PI_HI/LO).
pub(crate) const FLOAT128_PI: Float128 = Float128 {
    v64: ((FLOATX80_PI_EXP as u64) << 48) | (FLOAT_PI_HI >> 16),
    v0: (FLOAT_PI_HI << 48) | (FLOAT_PI_LO >> 16),
};

/// 1/LN2 as Float128.
pub(crate) const FLOAT128_LN2INV: Float128 = Float128 {
    v64: ((FLOAT_LN2INV_EXP as u64) << 48) | (FLOAT_LN2INV_HI >> 16),
    v0: (FLOAT_LN2INV_HI << 48) | (FLOAT_LN2INV_LO >> 16),
};

// LN(2) significand (96-bit precision, BETTER_THAN_PENTIUM variant from Bochs)
pub const LN2_SIG_HI: u64 = 0xb17217f7d1cf79ab;
pub const LN2_SIG_LO: u64 = 0xc9e3b39800000000;

// sqrt(2)/2 significand (for fyl2x range check)
pub const SQRT2_HALF_SIG: u64 = 0xb504f333f9de6484;

/// LN(2) as Float128 (from Bochs f2xm1.cc)
pub(crate) const FLOAT128_LN2: Float128 = Float128 {
    v64: 0x3ffe62e42fefa39e,
    v0: 0xf35793c7673007e6,
};

/// 2/LN(2) as Float128 (from Bochs fyl2x.cc)
pub(crate) const FLOAT128_LN2INV2: Float128 = Float128 {
    v64: 0x400071547652b82f,
    v0: 0xe1777d0ffda0d23a,
};

/// 1.0 as Float128
pub(crate) const FLOAT128_ONE: Float128 = Float128 {
    v64: 0x3fff000000000000,
    v0: 0x0000000000000000,
};

/// 2.0 as Float128
pub(crate) const FLOAT128_TWO: Float128 = Float128 {
    v64: 0x4000000000000000,
    v0: 0x0000000000000000,
};

/// sqrt(3) as Float128 (from Bochs fpatan.cc)
pub(crate) const FLOAT128_SQRT3: Float128 = Float128 {
    v64: 0x3fffbb67ae8584ca,
    v0: 0xa73b25742d7078b8,
};

/// PI/2 as Float128 (from Bochs fpatan.cc)
pub(crate) const FLOAT128_PI2: Float128 = Float128 {
    v64: 0x3fff921fb54442d1,
    v0: 0x8469898CC5170416,
};

/// PI/4 as Float128 (from Bochs fpatan.cc)
pub(crate) const FLOAT128_PI4: Float128 = Float128 {
    v64: 0x3ffe921fb54442d1,
    v0: 0x8469898CC5170416,
};

/// PI/6 as Float128 (from Bochs fpatan.cc)
pub(crate) const FLOAT128_PI6: Float128 = Float128 {
    v64: 0x3ffe0c152382d736,
    v0: 0x58465BB32E0F580F,
};

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_unpack() {
        let v64 = pack_to_f128_ui64(false, 0x3FFF, 0);
        assert_eq!(exp_f128_ui64(v64), 0x3FFF);
        assert!(!sign_f128_ui64(v64));
        assert_eq!(frac_f128_ui64(v64), 0);

        let v64_neg = pack_to_f128_ui64(true, 0x4000, 0x0000123456789ABC);
        assert!(sign_f128_ui64(v64_neg));
        assert_eq!(exp_f128_ui64(v64_neg), 0x4000);
        assert_eq!(frac_f128_ui64(v64_neg), 0x0000123456789ABC);
    }

    #[test]
    fn test_i32_to_f128() {
        let z = i32_to_f128(0);
        assert_eq!(z, FLOAT128_POSITIVE_ZERO);

        let one = i32_to_f128(1);
        assert_eq!(exp_f128_ui64(one.v64), 0x3FFF);
        assert!(!sign_f128_ui64(one.v64));

        let neg = i32_to_f128(-1);
        assert!(sign_f128_ui64(neg.v64));
        assert_eq!(exp_f128_ui64(neg.v64), 0x3FFF);
    }

    #[test]
    fn test_f128_add_zero() {
        let mut status = SoftFloatStatus::default();
        let a = i32_to_f128(0);
        let b = i32_to_f128(0);
        let c = f128_add(a, b, &mut status);
        assert!(f128_is_zero(c));
    }

    #[test]
    fn test_f128_mul_one() {
        let mut status = SoftFloatStatus::default();
        let one = i32_to_f128(1);
        let two = i32_to_f128(2);
        let result = f128_mul(one, two, &mut status);
        let expected = i32_to_f128(2);
        assert_eq!(result.v64, expected.v64);
        assert_eq!(result.v0, expected.v0);
    }

    #[test]
    fn test_f128_sign_helpers() {
        let pos = i32_to_f128(42);
        let neg = f128_chs(pos);
        assert!(sign_f128_ui64(neg.v64));
        let abs_neg = f128_abs(neg);
        assert_eq!(abs_neg, pos);
    }

    #[test]
    fn test_f128_is_nan_inf() {
        let nan = FLOAT128_DEFAULT_NAN;
        assert!(f128_is_nan(nan));
        assert!(!f128_is_inf(nan));

        let inf = Float128 {
            v64: pack_to_f128_ui64(false, 0x7FFF, 0),
            v0: 0,
        };
        assert!(f128_is_inf(inf));
        assert!(!f128_is_nan(inf));
    }

    #[test]
    fn test_mul128_by_32() {
        let (hi, lo) = mul128_by_32(0, 100, 3);
        assert_eq!(hi, 0);
        assert_eq!(lo, 300);
    }

    #[test]
    fn test_add192() {
        let (z0, z1, z2) = add192(0, 0, 1, 0, 0, 0xFFFFFFFFFFFFFFFF);
        assert_eq!(z2, 0);
        assert_eq!(z1, 1);
        assert_eq!(z0, 0);
    }

    #[test]
    fn test_estimate_div() {
        // Simple case: divide 1.0 by itself
        let q = estimate_div_128_to_64(0, 0x8000000000000000, 0x8000000000000000);
        // Should be close to 0 since a0 < b
        assert!(q < 3);
    }

    #[test]
    fn test_short_shift_right_jam128_extra_basic() {
        let (v64, v0, extra) = short_shift_right_jam128_extra(0x100, 0x200, 0, 1);
        assert_eq!(v64, 0x80);
        assert_eq!(v0, 0x100);
        assert_eq!(extra, 0);
    }
}
