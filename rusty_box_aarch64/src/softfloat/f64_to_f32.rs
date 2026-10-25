//! Double-precision to single-precision conversion.
//! Ported from Bochs softfloat3e/f64_to_f32.cc.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Bochs softfloat3e `f64_to_f32`. The NaN path is Arm FPConvertNaN (the
/// closed form of `softfloat_f64UIToCommonNaN` followed by
/// `softfloat_commonNaNToF32UI`): the sign is kept, the top 22 payload bits
/// survive and the quiet bit is forced on; FPCR.DN gives the default NaN.
pub(crate) fn f64_to_f32(a: Float64, status: &mut SoftFloatStatus) -> Float32 {
    let sign = sign_f64(a);
    let exp = exp_f64(a);
    let frac = frac_f64(a);

    if exp == 0x7FF {
        if frac != 0 {
            if f64_is_signaling_nan(a) {
                softfloat_raise_flags(status, FLAG_INVALID);
            }
            if softfloat_default_nan_mode(status) {
                return FLOAT32_DEFAULT_NAN;
            }
            return ((sign as u32) << 31) | 0x7FC0_0000 | ((frac >> 29) as u32);
        }
        return pack_to_f32(sign, 0xFF, 0);
    }

    if exp == 0 && frac != 0 {
        softfloat_raise_flags(status, FLAG_DENORMAL);
    }

    let frac32 = short_shift_right_jam64(frac, 22) as u32;
    if (exp as u32 | frac32) == 0 {
        return pack_to_f32(sign, 0, 0);
    }

    round_pack_to_f32(sign, exp - 0x381, frac32 | 0x4000_0000, status)
}
