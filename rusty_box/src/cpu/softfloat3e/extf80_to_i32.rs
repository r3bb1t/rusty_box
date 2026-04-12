#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! ExtFloat80 to i32 conversions.
//! Ported from Berkeley SoftFloat 3e: extF80_to_i32.c, extF80_to_i32_r_minMag.c

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Convert extFloat80 to i32 using given rounding mode.
pub fn extf80_to_i32(
    a: floatx80,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i32 {
    // Handle unsupported
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return I32_FROM_NAN;
    }

    let sign = sign_extf80(a.sign_exp);
    let exp = exp_extf80(a.sign_exp) as i32;
    let sig = a.signif;

    // NaN handling (i32_fromNaN == i32_fromPosOverflow == i32_fromNegOverflow on x86)
    // All overflow to 0x80000000

    let shift_dist = 0x4032 - exp;
    let shift_dist = if shift_dist <= 0 { 1 } else { shift_dist };
    let sig = shift_right_jam64(sig, shift_dist as u32);
    softfloat_round_to_i32(sign, sig, rounding_mode, exact, status)
}

/// Convert extFloat80 to i32 truncating toward zero.
pub fn extf80_to_i32_round_to_zero(a: floatx80, exact: bool, status: &mut SoftFloatStatus) -> i32 {
    // Handle unsupported
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return I32_FROM_NAN;
    }

    let exp = exp_extf80(a.sign_exp) as i32;
    let sig = a.signif;

    let shift_dist = 0x403E - exp;
    if shift_dist >= 64 {
        if exact && (exp as u64 | sig) != 0 {
            softfloat_raiseFlags(status, FLAG_INEXACT);
        }
        return 0;
    }

    let sign = sign_extf80(a.sign_exp);
    if shift_dist < 33 {
        // Check for exactly INT32_MIN
        if a.sign_exp == pack_to_extf80_sign_exp(true, 0x401E) && sig < 0x8000000100000000 {
            if exact && (sig & 0x00000000FFFFFFFF) != 0 {
                softfloat_raiseFlags(status, FLAG_INEXACT);
            }
            return i32::MIN;
        }
        softfloat_raiseFlags(status, FLAG_INVALID);
        return if (exp == 0x7FFF) && (sig & 0x7FFFFFFFFFFFFFFF) != 0 {
            I32_FROM_NAN
        } else if sign {
            I32_FROM_NEG_OVERFLOW
        } else {
            I32_FROM_POS_OVERFLOW
        };
    }

    let abs_z = (sig >> shift_dist) as i32;
    if exact && ((abs_z as u64) << shift_dist) != sig {
        softfloat_raiseFlags(status, FLAG_INEXACT);
    }
    if sign {
        -abs_z
    } else {
        abs_z
    }
}
