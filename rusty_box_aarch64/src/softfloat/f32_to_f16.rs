//! Single-precision to half-precision conversion.
//! Ported from Berkeley SoftFloat 3e `f32_to_f16.cc`.

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Convert a Float32 to Float16.
///
/// Bochs softfloat3e `f32_to_f16`. The NaN path is Arm FPConvertNaN (the
/// closed form of `softfloat_f32UIToCommonNaN` followed by
/// `softfloat_commonNaNToF16UI`, `sign<<15 | 0x7E00 | v64>>54`): the sign is
/// kept, the top 9 payload bits survive and the quiet bit is forced on;
/// FPCR.DN gives the default NaN.
pub(crate) fn f32_to_f16(a: Float32, status: &mut SoftFloatStatus) -> Float16 {
    let sign = sign_f32(a);
    let exp = exp_f32(a);
    let frac = frac_f32(a);

    if exp == 0xFF {
        if frac != 0 {
            if f32_is_signaling_nan(a) {
                softfloat_raise_flags(status, FLAG_INVALID);
            }
            if softfloat_default_nan_mode(status) {
                return float16_default_nan(status);
            }
            return ((sign as u16) << 15) | 0x7E00 | ((frac >> 13) as u16);
        }
        return pack_to_f16(sign, 0x1F, 0);
    }

    if exp == 0 && frac != 0 {
        softfloat_raise_flags(status, FLAG_DENORMAL);
    }

    // Sticky-collapse the 9 bits that do not survive the narrowing.
    let frac16 = ((frac >> 9) | u32::from((frac & 0x1FF) != 0)) as u16;
    if exp == 0 && frac16 == 0 {
        return pack_to_f16(sign, 0, 0);
    }

    round_pack_to_f16(sign, exp - 0x71, frac16 | 0x4000, status)
}
