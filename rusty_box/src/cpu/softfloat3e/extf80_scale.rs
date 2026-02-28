#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! ExtFloat80 scale (FSCALE instruction support).
//! Ported from Berkeley SoftFloat 3e: extF80_scale.c

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Scale extFloat80 value `a` by `b`:
/// Truncates `b` to integer, adds to exponent of `a`.
pub fn extf80_scale(a: floatx80, b: floatx80, status: &mut SoftFloatStatus) -> floatx80 {
    // Handle unsupported
    if extf80_is_unsupported(a) || extf80_is_unsupported(b) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOATX80_DEFAULT_NAN;
    }

    let sign_a = sign_extf80(a.sign_exp);
    let mut exp_a = exp_extf80(a.sign_exp) as i32;
    let mut sig_a = a.signif;
    let sign_b = sign_extf80(b.sign_exp);
    let exp_b = exp_extf80(b.sign_exp) as i32;
    let sig_b = b.signif;

    // A is NaN/Inf
    if exp_a == 0x7FFF {
        if (sig_a << 1) != 0 || ((exp_b == 0x7FFF) && (sig_b << 1) != 0) {
            return softfloat_propagate_nan_extf80(
                a.sign_exp, a.signif, b.sign_exp, b.signif, status,
            );
        }
        if (exp_b == 0x7FFF) && sign_b {
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOATX80_DEFAULT_NAN;
        }
        if sig_b != 0 && exp_b == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return a;
    }

    // B is NaN/Inf
    if exp_b == 0x7FFF {
        if (sig_b << 1) != 0 {
            return softfloat_propagate_nan_extf80(
                a.sign_exp, a.signif, b.sign_exp, b.signif, status,
            );
        }
        if (exp_a as u64 | sig_a) == 0 {
            if !sign_b {
                softfloat_raiseFlags(status, FLAG_INVALID);
                return FLOATX80_DEFAULT_NAN;
            }
            return a;
        }
        if sig_a != 0 && exp_a == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        if sign_b {
            return pack_floatx80(sign_a, 0, 0);
        }
        return pack_floatx80(sign_a, 0x7FFF, 0x8000000000000000);
    }

    // A is denormal
    if exp_a == 0 {
        if sig_b != 0 && exp_b == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        if sig_a == 0 {
            return a;
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(sig_a);
        exp_a = norm.exp + 1;
        sig_a = norm.sig;
        if exp_b < 0x3FFF {
            return norm_round_pack_to_extf80(sign_a, exp_a, sig_a, 0, 80, status);
        }
    }

    // B is zero or denormal
    if exp_b == 0 {
        if sig_b == 0 {
            return a;
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(sig_b);
        let _exp_b = norm.exp + 1;
        let _sig_b = norm.sig;
        // After normalization, if exp_b < 0x3FFF the scale is < 1.0 integer → no effect
        return a;
    }

    if exp_b > 0x400E {
        // Exponent too large → overflow or underflow
        return round_pack_to_extf80(
            sign_a,
            if sign_b { -0x3FFF } else { 0x7FFF },
            sig_a,
            0,
            80,
            status,
        );
    }

    if exp_b < 0x3FFF {
        return a; // Scale < 1.0 → no integer part → no effect
    }

    let shift_count = 0x403E - exp_b;
    let sig_b_trunc = sig_b >> shift_count;
    let mut scale = sig_b_trunc as i32;
    if sign_b {
        scale = -scale;
    }

    round_pack_to_extf80(sign_a, exp_a + scale, sig_a, 0, 80, status)
}
