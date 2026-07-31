#![allow(dead_code, non_snake_case)]
//! Double-precision to single-precision conversion.
//! Ported from Bochs softfloat3e/f64_to_f32.cc.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;

/// Bochs softfloat3e `f64_to_f32`. The NaN path is the closed form of
/// `softfloat_f64UIToCommonNaN` followed by `softfloat_commonNaNToF32UI`
/// (8086-SSE specialization).
pub(crate) fn f64_to_f32(a: float64, status: &mut SoftFloatStatus) -> float32 {
    let sign = sign_f64(a);
    let exp = exp_f64(a);
    let frac = frac_f64(a);

    if exp == 0x7FF {
        if frac != 0 {
            // softfloat_f64UIToCommonNaN raises #I on a signaling NaN.
            if f64_is_signaling_nan(a) {
                softfloat_raiseFlags(status, FLAG_INVALID);
            }
            return ((sign as u32) << 31) | 0x7FC0_0000 | (((a << 12) >> 41) as u32);
        }
        return pack_to_f32(sign, 0xFF, 0);
    }

    if exp == 0 && frac != 0 {
        if softfloat_denormalsAreZeros(status) {
            return pack_to_f32(sign, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
    }

    let frac32 = short_shift_right_jam64(frac, 22) as u32;
    if (exp as u32 | frac32) == 0 {
        return pack_to_f32(sign, 0, 0);
    }

    round_pack_to_f32(sign, exp - 0x381, frac32 | 0x4000_0000, status)
}
