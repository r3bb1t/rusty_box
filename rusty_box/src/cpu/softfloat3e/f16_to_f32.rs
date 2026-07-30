#![allow(dead_code, non_snake_case)]
//! Half-precision to single-precision conversion.
//! Ported from Berkeley SoftFloat 3e `f16_to_f32.cc`.

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;

/// Convert a float16 to float32.
///
/// Bochs softfloat3e `f16_to_f32`. The NaN path is the closed form of
/// `softfloat_f16UIToCommonNaN` followed by `softfloat_commonNaNToF32UI`
/// (8086-SSE specialization): the payload is shifted left by 13 and the
/// quiet bit is forced on.
pub(crate) fn f16_to_f32(a: float16, status: &mut SoftFloatStatus) -> float32 {
    let sign = sign_f16(a);
    let mut exp = exp_f16(a);
    let mut frac = frac_f16(a);

    if exp == 0x1F {
        if frac != 0 {
            // softfloat_f16UIToCommonNaN raises #I on a signaling NaN.
            if f16_is_signaling_nan(a) {
                softfloat_raiseFlags(status, FLAG_INVALID);
            }
            return ((sign as u32) << 31) | 0x7FC0_0000 | ((a as u32) << 13);
        }
        return pack_to_f32(sign, 0xFF, 0);
    }

    if exp == 0 {
        if frac == 0 || softfloat_denormalsAreZeros(status) {
            return pack_to_f32(sign, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f16_sig(frac);
        exp = norm.exp - 1;
        frac = norm.sig as u16;
    }

    pack_to_f32(sign, exp + 0x70, (frac as u32) << 13)
}
