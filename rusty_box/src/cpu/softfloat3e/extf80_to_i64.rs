#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! ExtFloat80 to i64 conversions.
//! Ported from Berkeley SoftFloat 3e: extF80_to_i64.c, extF80_to_i64_r_minMag.c

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Convert extFloat80 to i64 using given rounding mode.
pub fn extf80_to_i64(
    a: floatx80,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i64 {
    // Handle unsupported
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return I64_FROM_NAN;
    }

    let sign = sign_extf80(a.sign_exp);
    let exp = exp_extf80(a.sign_exp) as i32;
    let sig = a.signif;

    let shift_dist = 0x403E - exp;
    if shift_dist <= 0 {
        if shift_dist != 0 {
            // Too large
            softfloat_raiseFlags(status, FLAG_INVALID);
            return if (exp == 0x7FFF) && (sig & 0x7FFFFFFFFFFFFFFF) != 0 {
                I64_FROM_NAN
            } else if sign {
                I64_FROM_NEG_OVERFLOW
            } else {
                I64_FROM_POS_OVERFLOW
            };
        }
        // Exact fit
        return softfloat_round_to_i64(sign, sig, 0, rounding_mode, exact, status);
    }

    let (v, extra) = shift_right_jam64_extra(sig, 0, shift_dist as u32);
    softfloat_round_to_i64(sign, v, extra, rounding_mode, exact, status)
}

/// Convert extFloat80 to i64 truncating toward zero.
pub fn extf80_to_i64_round_to_zero(a: floatx80, exact: bool, status: &mut SoftFloatStatus) -> i64 {
    // Handle unsupported
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return I64_FROM_NAN;
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
    if shift_dist <= 0 {
        // Check for exactly INT64_MIN
        if a.sign_exp == pack_to_extf80_sign_exp(true, 0x403E) && sig == 0x8000000000000000 {
            return i64::MIN;
        }
        softfloat_raiseFlags(status, FLAG_INVALID);
        return if (exp == 0x7FFF) && (sig & 0x7FFFFFFFFFFFFFFF) != 0 {
            I64_FROM_NAN
        } else if sign {
            I64_FROM_NEG_OVERFLOW
        } else {
            I64_FROM_POS_OVERFLOW
        };
    }

    let abs_z = (sig >> shift_dist) as i64;
    if exact && ((sig << ((-shift_dist) & 63)) != 0) {
        softfloat_raiseFlags(status, FLAG_INEXACT);
    }
    if sign {
        -abs_z
    } else {
        abs_z
    }
}
