//! Single-precision to double-precision conversion.
//! Ported from Bochs softfloat3e/f32_to_f64.cc.

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Bochs softfloat3e `f32_to_f64`. The NaN path is Arm FPConvertNaN (the
/// closed form of `softfloat_f32UIToCommonNaN` followed by
/// `softfloat_commonNaNToF64UI`): the sign is kept, the payload moves left 29
/// bits and the quiet bit is forced on; FPCR.DN gives the default NaN.
pub(crate) fn f32_to_f64(a: Float32, status: &mut SoftFloatStatus) -> Float64 {
    let sign = sign_f32(a);
    let mut exp = exp_f32(a);
    let mut frac = frac_f32(a);

    if exp == 0xFF {
        if frac != 0 {
            if f32_is_signaling_nan(a) {
                softfloat_raise_flags(status, FLAG_INVALID);
            }
            if softfloat_default_nan_mode(status) {
                return float64_default_nan(status);
            }
            return ((sign as u64) << 63) | 0x7FF8_0000_0000_0000 | ((frac as u64) << 29);
        }
        return pack_to_f64(sign, 0x7FF, 0);
    }

    if exp == 0 {
        if frac == 0 {
            return pack_to_f64(sign, 0, 0);
        }
        softfloat_raise_flags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f32_sig(frac);
        exp = ns.exp - 1;
        frac = ns.sig;
    }

    pack_to_f64(sign, exp + 0x380, (frac as u64) << 29)
}
