//! Float32 classification. Ported from Bochs softfloat3e/f32_class.cc.

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;

/// Bochs softfloat3e/f32_class.cc `f32_class`.
pub(in crate::cpu) fn f32_class(a: Float32) -> SoftFloatClass {
    let sign_a = sign_f32(a);
    let exp_a = exp_f32(a);
    let sig_a = frac_f32(a);

    if exp_a == 0xFF {
        if sig_a == 0 {
            return if sign_a {
                SoftFloatClass::NegativeInf
            } else {
                SoftFloatClass::PositiveInf
            };
        }
        return if (sig_a & 0x0040_0000) != 0 {
            SoftFloatClass::QNaN
        } else {
            SoftFloatClass::SNaN
        };
    }

    if exp_a == 0 {
        if sig_a == 0 {
            return SoftFloatClass::Zero;
        }
        return SoftFloatClass::Denormal;
    }

    SoftFloatClass::Normalized
}
