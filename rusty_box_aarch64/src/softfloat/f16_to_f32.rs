//! Half-precision to single-precision conversion.
//! Ported from Berkeley SoftFloat 3e `f16_to_f32.cc`.

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Convert a Float16 to Float32.
///
/// Bochs softfloat3e `f16_to_f32`. The NaN path is Arm FPConvertNaN (the
/// closed form of `softfloat_f16UIToCommonNaN` followed by
/// `softfloat_commonNaNToF32UI`): the sign is kept, the payload is shifted
/// left by 13 and the quiet bit is forced on; FPCR.DN gives the default NaN.
/// A half-precision denormal is never flushed (Arm FPUnpackCV forces FZ16
/// off) and is exactly representable in single precision.
pub(crate) fn f16_to_f32(a: Float16, status: &mut SoftFloatStatus) -> Float32 {
    let sign = sign_f16(a);
    let mut exp = exp_f16(a);
    let mut frac = frac_f16(a);

    if exp == 0x1F {
        if frac != 0 {
            if f16_is_signaling_nan(a) {
                softfloat_raise_flags(status, FLAG_INVALID);
            }
            if softfloat_default_nan_mode(status) {
                return FLOAT32_DEFAULT_NAN;
            }
            return ((sign as u32) << 31) | 0x7FC0_0000 | ((frac as u32) << 13);
        }
        return pack_to_f32(sign, 0xFF, 0);
    }

    if exp == 0 {
        if frac == 0 {
            return pack_to_f32(sign, 0, 0);
        }
        softfloat_raise_flags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f16_sig(frac);
        exp = norm.exp - 1;
        frac = norm.sig;
    }

    pack_to_f32(sign, exp + 0x70, (frac as u32) << 13)
}
