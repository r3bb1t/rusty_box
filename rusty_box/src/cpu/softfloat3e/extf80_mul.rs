#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! ExtFloat80 multiplication.
//! Ported from Berkeley SoftFloat 3e: extF80_mul.c

use super::softfloat_types::*;
use super::softfloat::*;
use super::primitives::*;
use super::specialize::*;
use super::internals::*;

pub fn extf80_mul(a: floatx80, b: floatx80, status: &mut SoftFloatStatus) -> floatx80 {
    // Handle unsupported encodings
    if extf80_is_unsupported(a) || extf80_is_unsupported(b) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOATX80_DEFAULT_NAN;
    }

    let sign_a = sign_extf80(a.sign_exp);
    let mut exp_a = exp_extf80(a.sign_exp) as i32;
    let mut sig_a = a.signif;
    let sign_b = sign_extf80(b.sign_exp);
    let mut exp_b = exp_extf80(b.sign_exp) as i32;
    let mut sig_b = b.signif;
    let sign_z = sign_a ^ sign_b;

    // NaN/Inf handling
    if exp_a == 0x7FFF {
        if (sig_a & 0x7FFFFFFFFFFFFFFF) != 0
            || ((exp_b == 0x7FFF) && (sig_b & 0x7FFFFFFFFFFFFFFF) != 0)
        {
            return softfloat_propagate_nan_extf80(a.sign_exp, a.signif, b.sign_exp, b.signif, status);
        }
        let mag_bits = (exp_b as u64) | sig_b;
        if mag_bits == 0 {
            // Inf * 0 = invalid
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOATX80_DEFAULT_NAN;
        }
        if (exp_b == 0 && sig_b != 0) || (exp_a == 0 && sig_a != 0) {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_floatx80(sign_z, 0x7FFF, 0x8000000000000000);
    }
    if exp_b == 0x7FFF {
        if (sig_b & 0x7FFFFFFFFFFFFFFF) != 0 {
            return softfloat_propagate_nan_extf80(a.sign_exp, a.signif, b.sign_exp, b.signif, status);
        }
        let mag_bits = (exp_a as u64) | sig_a;
        if mag_bits == 0 {
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOATX80_DEFAULT_NAN;
        }
        if (exp_a == 0 && sig_a != 0) || (exp_b == 0 && sig_b != 0) {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_floatx80(sign_z, 0x7FFF, 0x8000000000000000);
    }

    // Handle denormals for A
    if exp_a == 0 {
        exp_a = 1;
        if sig_a != 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
    }
    if (sig_a & 0x8000000000000000) == 0 {
        if sig_a == 0 {
            if exp_b == 0 && sig_b != 0 {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }
            return pack_floatx80(sign_z, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(sig_a);
        exp_a += norm.exp;
        sig_a = norm.sig;
    }

    // Handle denormals for B
    if exp_b == 0 {
        exp_b = 1;
        if sig_b != 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
    }
    if (sig_b & 0x8000000000000000) == 0 {
        if sig_b == 0 {
            return pack_floatx80(sign_z, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(sig_b);
        exp_b += norm.exp;
        sig_b = norm.sig;
    }

    // Main multiply
    let mut exp_z = exp_a + exp_b - 0x3FFE;
    let (mut sig_z_hi, sig_z_lo) = mul64_to_128(sig_a, sig_b);

    if (sig_z_hi & 0x8000000000000000) == 0 {
        exp_z -= 1;
        let (h, l) = add128(sig_z_hi, sig_z_lo, sig_z_hi, sig_z_lo);
        sig_z_hi = h;
        round_pack_to_extf80(
            sign_z, exp_z, sig_z_hi, l,
            softfloat_extF80_roundingPrecision(status), status,
        )
    } else {
        round_pack_to_extf80(
            sign_z, exp_z, sig_z_hi, sig_z_lo,
            softfloat_extF80_roundingPrecision(status), status,
        )
    }
}
