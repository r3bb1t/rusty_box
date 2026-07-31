#![allow(dead_code, non_snake_case)]
//! Single-precision to signed-integer conversions.
//! Ported from Bochs softfloat3e/f32_to_i32.cc, f32_to_i32_r_minMag.cc,
//! f32_to_i64.cc and f32_to_i64_r_minMag.cc.
//!
//! The `#if (i32_fromNaN != i32_fromPosOverflow)` guard in the upstream
//! sources is vacuous under Bochs's 8086-SSE specialization — all three
//! responses are the integer indefinite value — so the NaN pre-check it
//! wraps is absent here too, and NaN falls through to the overflow path.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Bochs softfloat3e `f32_to_i32`.
pub(in crate::cpu) fn f32_to_i32(
    a: float32,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i32 {
    let sign = sign_f32(a);
    let exp = exp_f32(a);
    let mut sig = frac_f32(a);

    if exp != 0 {
        sig |= 0x0080_0000;
    } else if softfloat_denormalsAreZeros(status) {
        return 0;
    }

    let mut sig64 = (sig as u64) << 32;
    let shift_dist = 0xAA - exp;
    if 0 < shift_dist {
        sig64 = shift_right_jam64(sig64, shift_dist as u32);
    }
    softfloat_round_to_i32(sign, sig64, rounding_mode, exact, status)
}

/// Bochs softfloat3e `f32_to_i32_r_minMag` (round toward zero).
pub(in crate::cpu) fn f32_to_i32_r_min_mag(
    a: float32,
    exact: bool,
    saturate: bool,
    status: &mut SoftFloatStatus,
) -> i32 {
    let exp = exp_f32(a);
    let mut sig = frac_f32(a);

    if softfloat_denormalsAreZeros(status) && exp == 0 && sig != 0 {
        return 0;
    }

    let shift_dist = 0x9E - exp;
    if 32 <= shift_dist {
        if exact && (exp as u32 | sig) != 0 {
            softfloat_raiseFlags(status, FLAG_INEXACT);
        }
        return 0;
    }
    let sign = sign_f32(a);
    if shift_dist <= 0 {
        if a == pack_to_f32(true, 0x9E, 0) {
            return -0x7FFF_FFFF - 1;
        }
        let nan_response = if saturate { 0 } else { I32_FROM_NAN };
        let neg_overflow = if saturate {
            I32_MIN_NEGATIVE_VALUE
        } else {
            I32_FROM_NEG_OVERFLOW
        };
        let pos_overflow = if saturate {
            I32_MAX_POSITIVE_VALUE
        } else {
            I32_FROM_POS_OVERFLOW
        };
        softfloat_raiseFlags(status, FLAG_INVALID);
        return if exp == 0xFF && sig != 0 {
            nan_response
        } else if sign {
            neg_overflow
        } else {
            pos_overflow
        };
    }
    sig = (sig | 0x0080_0000) << 8;
    let abs_z = (sig >> shift_dist) as i32;
    if exact && ((abs_z as u32) << shift_dist) != sig {
        softfloat_raiseFlags(status, FLAG_INEXACT);
    }
    if sign {
        -abs_z
    } else {
        abs_z
    }
}

/// Bochs softfloat3e `f32_to_i64`.
pub(in crate::cpu) fn f32_to_i64(
    a: float32,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i64 {
    let sign = sign_f32(a);
    let exp = exp_f32(a);
    let mut sig = frac_f32(a);

    if softfloat_denormalsAreZeros(status) && exp == 0 && sig != 0 {
        return 0;
    }

    let shift_dist = 0xBE - exp;
    if shift_dist < 0 {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return if exp == 0xFF && sig != 0 {
            I64_FROM_NAN
        } else if sign {
            I64_FROM_NEG_OVERFLOW
        } else {
            I64_FROM_POS_OVERFLOW
        };
    }
    if exp != 0 {
        sig |= 0x0080_0000;
    }
    let mut sig64 = (sig as u64) << 40;
    let mut extra = 0u64;
    if shift_dist != 0 {
        let (v, e) = shift_right_jam64_extra(sig64, 0, shift_dist as u32);
        sig64 = v;
        extra = e;
    }
    softfloat_round_to_i64(sign, sig64, extra, rounding_mode, exact, status)
}

/// Bochs softfloat3e `f32_to_i64_r_minMag` (round toward zero).
pub(in crate::cpu) fn f32_to_i64_r_min_mag(
    a: float32,
    exact: bool,
    saturate: bool,
    status: &mut SoftFloatStatus,
) -> i64 {
    let exp = exp_f32(a);
    let mut sig = frac_f32(a);

    if softfloat_denormalsAreZeros(status) && exp == 0 && sig != 0 {
        return 0;
    }

    let mut shift_dist = 0xBE - exp;
    if 64 <= shift_dist {
        if exact && (exp as u32 | sig) != 0 {
            softfloat_raiseFlags(status, FLAG_INEXACT);
        }
        return 0;
    }
    let sign = sign_f32(a);
    if shift_dist <= 0 {
        if a == pack_to_f32(true, 0xBE, 0) {
            return -0x7FFF_FFFF_FFFF_FFFF - 1;
        }
        let nan_response = if saturate { 0 } else { I64_FROM_NAN };
        let neg_overflow = if saturate {
            I64_MIN_NEGATIVE_VALUE
        } else {
            I64_FROM_NEG_OVERFLOW
        };
        let pos_overflow = if saturate {
            I64_MAX_POSITIVE_VALUE
        } else {
            I64_FROM_POS_OVERFLOW
        };
        softfloat_raiseFlags(status, FLAG_INVALID);
        return if exp == 0xFF && sig != 0 {
            nan_response
        } else if sign {
            neg_overflow
        } else {
            pos_overflow
        };
    }
    sig |= 0x0080_0000;
    let sig64 = (sig as u64) << 40;
    let abs_z = (sig64 >> shift_dist) as i64;
    shift_dist = 40 - shift_dist;
    if exact && shift_dist < 0 && (sig << (shift_dist & 31)) != 0 {
        softfloat_raiseFlags(status, FLAG_INEXACT);
    }
    if sign {
        -abs_z
    } else {
        abs_z
    }
}
