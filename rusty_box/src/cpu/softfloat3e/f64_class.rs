#![allow(dead_code, non_snake_case)]
//! Float64 classification. Ported from Bochs softfloat3e/f64_class.cc.

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;

/// Bochs softfloat3e/f64_class.cc `f64_class`.
pub(crate) fn f64_class(a: float64) -> SoftFloatClass {
    let sign_a = sign_f64(a);
    let exp_a = exp_f64(a);
    let sig_a = frac_f64(a);

    if exp_a == 0x7FF {
        if sig_a == 0 {
            return if sign_a {
                SoftFloatClass::NegativeInf
            } else {
                SoftFloatClass::PositiveInf
            };
        }
        return if (sig_a & 0x0008_0000_0000_0000) != 0 {
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
