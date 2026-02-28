#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! ExtFloat80 to i16 conversion.
//! Ported from Berkeley SoftFloat 3e.

use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

pub fn extf80_to_i16(a: floatx80, status: &mut SoftFloatStatus) -> i16 {
    let val = extf80_to_i32(a, status);
    if (val < -32768) || (val > 32767) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return INT16_INDEFINITE;
    }
    val as i16
}

pub fn extf80_to_i16_round_to_zero(a: floatx80, status: &mut SoftFloatStatus) -> i16 {
    let val = extf80_to_i32_round_to_zero(a, status);
    if (val < -32768) || (val > 32767) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return INT16_INDEFINITE;
    }
    val as i16
}

fn extf80_to_i32(a: floatx80, status: &mut SoftFloatStatus) -> i32 {
    let rounding_mode = softfloat_getRoundingMode(status);
    super::extf80_to_i32::extf80_to_i32(a, rounding_mode, true, status)
}

fn extf80_to_i32_round_to_zero(a: floatx80, status: &mut SoftFloatStatus) -> i32 {
    super::extf80_to_i32::extf80_to_i32_round_to_zero(a, true, status)
}
