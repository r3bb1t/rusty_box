#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! ExtFloat80 comparison.
//! Ported from Berkeley SoftFloat 3e: extF80_compare.c

use super::softfloat_types::*;
use super::softfloat::*;
use super::internals::*;
use super::extf80_class::extf80_class;

/// Compare two extFloat80 values.
/// Returns RELATION_EQUAL, RELATION_LESS, RELATION_GREATER, or RELATION_UNORDERED.
/// If `quiet` is false, signaling NaN and quiet NaN both raise invalid.
/// If `quiet` is true, only signaling NaN raises invalid.
pub fn extf80_compare(a: floatx80, b: floatx80, quiet: bool, status: &mut SoftFloatStatus) -> i32 {
    let a_class = extf80_class(a);
    let b_class = extf80_class(b);

    // SNaN always raises invalid
    if a_class == SoftFloatClass::SNaN || b_class == SoftFloatClass::SNaN {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return RELATION_UNORDERED;
    }

    // QNaN: raise invalid only for signaling comparison
    if a_class == SoftFloatClass::QNaN || b_class == SoftFloatClass::QNaN {
        if !quiet {
            softfloat_raiseFlags(status, FLAG_INVALID);
        }
        return RELATION_UNORDERED;
    }

    // Denormal flag
    if a_class == SoftFloatClass::Denormal || b_class == SoftFloatClass::Denormal {
        softfloat_raiseFlags(status, FLAG_DENORMAL);
    }

    let sign_a = sign_extf80(a.sign_exp);
    let mut exp_a = exp_extf80(a.sign_exp) as i32;
    let mut sig_a = a.signif;
    let sign_b = sign_extf80(b.sign_exp);
    let mut exp_b = exp_extf80(b.sign_exp) as i32;
    let mut sig_b = b.signif;

    // Handle zeros
    if a_class == SoftFloatClass::Zero {
        if b_class == SoftFloatClass::Zero {
            return RELATION_EQUAL;
        }
        return if sign_b { RELATION_GREATER } else { RELATION_LESS };
    }
    if b_class == SoftFloatClass::Zero || sign_a != sign_b {
        return if sign_a { RELATION_LESS } else { RELATION_GREATER };
    }

    // Normalize denormals
    if a_class == SoftFloatClass::Denormal {
        let norm = norm_subnormal_extf80_sig(sig_a);
        exp_a += norm.exp + 1;
        sig_a = norm.sig;
    }
    if b_class == SoftFloatClass::Denormal {
        let norm = norm_subnormal_extf80_sig(sig_b);
        exp_b += norm.exp + 1;
        sig_b = norm.sig;
    }

    if exp_a == exp_b && sig_a == sig_b {
        return RELATION_EQUAL;
    }

    let less_than = if sign_a {
        (exp_b < exp_a) || ((exp_b == exp_a) && (sig_b < sig_a))
    } else {
        (exp_a < exp_b) || ((exp_a == exp_b) && (sig_a < sig_b))
    };

    if less_than { RELATION_LESS } else { RELATION_GREATER }
}
