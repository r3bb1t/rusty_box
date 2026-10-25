//! Double-precision to half-precision conversion.
//! Ported from Berkeley SoftFloat 3e f64_to_f16.c.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Berkeley `f64_to_f16`. The NaN path is Arm FPConvertNaN (the closed form
/// of `softfloat_f64UIToCommonNaN` followed by `softfloat_commonNaNToF16UI`):
/// the sign is kept, the top 9 payload bits survive and the quiet bit is
/// forced on; FPCR.DN gives the default NaN.
pub(crate) fn f64_to_f16(a: Float64, status: &mut SoftFloatStatus) -> Float16 {
    let sign = sign_f64(a);
    let exp = exp_f64(a);
    let frac = frac_f64(a);

    if exp == 0x7FF {
        if frac != 0 {
            if f64_is_signaling_nan(a) {
                softfloat_raise_flags(status, FLAG_INVALID);
            }
            if softfloat_default_nan_mode(status) {
                return FLOAT16_DEFAULT_NAN;
            }
            return ((sign as u16) << 15) | 0x7E00 | ((frac >> 42) as u16);
        }
        return pack_to_f16(sign, 0x1F, 0);
    }

    if exp == 0 && frac != 0 {
        softfloat_raise_flags(status, FLAG_DENORMAL);
    }

    let frac16 = short_shift_right_jam64(frac, 38) as u16;
    if (exp as u16 | frac16) == 0 {
        return pack_to_f16(sign, 0, 0);
    }

    round_pack_to_f16(sign, exp - 0x3F1, frac16 | 0x4000, status)
}
