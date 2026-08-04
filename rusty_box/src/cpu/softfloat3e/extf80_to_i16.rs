//! ExtFloat80 to i16 conversion.
//! Ported from Berkeley SoftFloat 3e.

use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

pub(in crate::cpu) fn extf80_to_i16(a: ExtFloat80, status: &mut SoftFloatStatus) -> i16 {
    let val = extf80_to_i32(a, status);
    if !(-32768..=32767).contains(&val) {
        softfloat_raise_flags(status, FLAG_INVALID);
        return INT16_INDEFINITE;
    }
    val as i16
}

pub(in crate::cpu) fn extf80_to_i16_round_to_zero(a: ExtFloat80, status: &mut SoftFloatStatus) -> i16 {
    let val = extf80_to_i32_round_to_zero(a, status);
    if !(-32768..=32767).contains(&val) {
        softfloat_raise_flags(status, FLAG_INVALID);
        return INT16_INDEFINITE;
    }
    val as i16
}

fn extf80_to_i32(a: ExtFloat80, status: &mut SoftFloatStatus) -> i32 {
    let rounding_mode = softfloat_get_rounding_mode(status);
    super::extf80_to_i32::extf80_to_i32(a, rounding_mode, true, status)
}

fn extf80_to_i32_round_to_zero(a: ExtFloat80, status: &mut SoftFloatStatus) -> i32 {
    super::extf80_to_i32::extf80_to_i32_round_to_zero(a, true, status)
}
