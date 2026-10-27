//! Half-precision to double-precision conversion.
//! Ported from Berkeley SoftFloat 3e f16_to_f64.c.

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Berkeley `f16_to_f64`. The NaN path is Arm FPConvertNaN (the closed form
/// of `softfloat_f16UIToCommonNaN` followed by `softfloat_commonNaNToF64UI`):
/// the sign is kept, the payload moves left 42 bits and the quiet bit is
/// forced on; FPCR.DN gives the default NaN. A half-precision denormal is
/// never flushed (Arm FPUnpackCV forces FZ16 off).
pub(crate) fn f16_to_f64(a: Float16, status: &mut SoftFloatStatus) -> Float64 {
    let sign = sign_f16(a);
    let mut exp = exp_f16(a);
    let mut frac = frac_f16(a);

    if exp == 0x1F {
        if frac != 0 {
            if f16_is_signaling_nan(a) {
                softfloat_raise_flags(status, FLAG_INVALID);
            }
            if softfloat_default_nan_mode(status) {
                return float64_default_nan(status);
            }
            return ((sign as u64) << 63) | 0x7FF8_0000_0000_0000 | ((frac as u64) << 42);
        }
        return pack_to_f64(sign, 0x7FF, 0);
    }

    if exp == 0 {
        if frac == 0 {
            return pack_to_f64(sign, 0, 0);
        }
        softfloat_raise_flags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f16_sig(frac);
        exp = norm.exp - 1;
        frac = norm.sig;
    }

    pack_to_f64(sign, exp + 0x3F0, (frac as u64) << 42)
}
