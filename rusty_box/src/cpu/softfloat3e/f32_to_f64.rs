#![allow(dead_code, non_snake_case)]
//! Single-precision to double-precision conversion.
//! Ported from Bochs softfloat3e/f32_to_f64.cc.

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;

/// Bochs softfloat3e `f32_to_f64`. The NaN path is the closed form of
/// `softfloat_f32UIToCommonNaN` followed by `softfloat_commonNaNToF64UI`
/// (8086-SSE specialization): the payload moves left 29 bits and the quiet
/// bit is forced on.
pub(in crate::cpu) fn f32_to_f64(a: float32, status: &mut SoftFloatStatus) -> float64 {
    let sign = sign_f32(a);
    let mut exp = exp_f32(a);
    let mut frac = frac_f32(a);

    if exp == 0xFF {
        if frac != 0 {
            // softfloat_f32UIToCommonNaN raises #I on a signaling NaN.
            if f32_is_signaling_nan(a) {
                softfloat_raiseFlags(status, FLAG_INVALID);
            }
            return ((sign as u64) << 63) | 0x7FF8_0000_0000_0000 | (((a as u64) << 41) >> 12);
        }
        return pack_to_f64(sign, 0x7FF, 0);
    }

    if exp == 0 {
        if frac == 0 || softfloat_denormalsAreZeros(status) {
            return pack_to_f64(sign, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f32_sig(frac);
        exp = ns.exp - 1;
        frac = ns.sig;
    }

    pack_to_f64(sign, exp + 0x380, (frac as u64) << 29)
}
