#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! ExtFloat80 classification.
//! Ported from Berkeley SoftFloat 3e: extF80_class.c

use super::softfloat_types::*;
use super::softfloat::*;
use super::internals::*;

/// Classify an extFloat80 value.
pub fn extf80_class(a: floatx80) -> SoftFloatClass {
    let sign_a = sign_extf80(a.sign_exp);
    let exp_a = exp_extf80(a.sign_exp) as i32;
    let sig_a = a.signif;

    if exp_a == 0 {
        if sig_a == 0 {
            return SoftFloatClass::Zero;
        }
        return SoftFloatClass::Denormal; // denormal or pseudo-denormal
    }

    // Valid numbers have the MS bit set
    if (sig_a & 0x8000000000000000) == 0 {
        return SoftFloatClass::SNaN; // report unsupported as SNaN
    }

    if exp_a == 0x7FFF {
        if (sig_a << 1) == 0 {
            return if sign_a {
                SoftFloatClass::NegativeInf
            } else {
                SoftFloatClass::PositiveInf
            };
        }
        return if (sig_a & 0x4000000000000000) != 0 {
            SoftFloatClass::QNaN
        } else {
            SoftFloatClass::SNaN
        };
    }

    SoftFloatClass::Normalized
}
