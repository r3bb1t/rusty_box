#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! ExtFloat80 division.
//! Ported from Berkeley SoftFloat 3e: extF80_div.c

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

pub fn extf80_div(a: floatx80, b: floatx80, status: &mut SoftFloatStatus) -> floatx80 {
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
        if (sig_a & 0x7FFFFFFFFFFFFFFF) != 0 {
            return softfloat_propagate_nan_extf80(
                a.sign_exp, a.signif, b.sign_exp, b.signif, status,
            );
        }
        if exp_b == 0x7FFF {
            if (sig_b & 0x7FFFFFFFFFFFFFFF) != 0 {
                return softfloat_propagate_nan_extf80(
                    a.sign_exp, a.signif, b.sign_exp, b.signif, status,
                );
            }
            // Inf / Inf = invalid
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOATX80_DEFAULT_NAN;
        }
        if exp_b == 0 && sig_b != 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_floatx80(sign_z, 0x7FFF, 0x8000000000000000);
    }
    if exp_b == 0x7FFF {
        if (sig_b & 0x7FFFFFFFFFFFFFFF) != 0 {
            return softfloat_propagate_nan_extf80(
                a.sign_exp, a.signif, b.sign_exp, b.signif, status,
            );
        }
        if exp_a == 0 && sig_a != 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_floatx80(sign_z, 0, 0);
    }

    // Handle B denormals
    if exp_b == 0 {
        exp_b = 1;
        if sig_b != 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
    }
    if (sig_b & 0x8000000000000000) == 0 {
        if sig_b == 0 {
            if sig_a == 0 {
                // 0/0 = invalid
                softfloat_raiseFlags(status, FLAG_INVALID);
                return FLOATX80_DEFAULT_NAN;
            }
            // x/0 = divide by zero
            softfloat_raiseFlags(status, FLAG_DIVBYZERO);
            return pack_floatx80(sign_z, 0x7FFF, 0x8000000000000000);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(sig_b);
        exp_b += norm.exp;
        sig_b = norm.sig;
    }

    // Handle A denormals
    if exp_a == 0 {
        exp_a = 1;
        if sig_a != 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
    }
    if (sig_a & 0x8000000000000000) == 0 {
        if sig_a == 0 {
            return pack_floatx80(sign_z, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(sig_a);
        exp_a += norm.exp;
        sig_a = norm.sig;
    }

    // Main division using reciprocal approximation
    let mut exp_z = exp_a - exp_b + 0x3FFF;
    let (rem_hi, rem_lo) = if sig_a < sig_b {
        exp_z -= 1;
        short_shift_left128(0, sig_a, 32)
    } else {
        short_shift_left128(0, sig_a, 31)
    };

    let recip32 = approx_recip32_1((sig_b >> 32) as u32);
    let mut sig_z: u64 = 0;
    let mut rem = (rem_hi, rem_lo);
    let mut ix: i32 = 2;
    let mut q: u32;

    loop {
        let q64 = ((rem.0 >> 2) as u32 as u64).wrapping_mul(recip32 as u64);
        q = ((q64.wrapping_add(0x80000000)) >> 32) as u32;
        ix -= 1;
        if ix < 0 {
            break;
        }
        rem = short_shift_left128(rem.0, rem.1, 29);
        let term = mul64_by_shifted32_to128(sig_b, q);
        rem = sub128(rem.0, rem.1, term.0, term.1);
        if (rem.0 & 0x8000000000000000) != 0 {
            q = q.wrapping_sub(1);
            rem = add128(rem.0, rem.1, sig_b >> 32, sig_b << 32);
        }
        sig_z = (sig_z << 29).wrapping_add(q as u64);
    }

    // Refinement
    if ((q.wrapping_add(1)) & 0x3FFFFF) < 2 {
        rem = short_shift_left128(rem.0, rem.1, 29);
        let term = mul64_by_shifted32_to128(sig_b, q);
        rem = sub128(rem.0, rem.1, term.0, term.1);
        let term2 = short_shift_left128(0, sig_b, 32);
        if (rem.0 & 0x8000000000000000) != 0 {
            q = q.wrapping_sub(1);
            rem = add128(rem.0, rem.1, term2.0, term2.1);
        } else if le128(term2.0, term2.1, rem.0, rem.1) {
            q = q.wrapping_add(1);
            rem = sub128(rem.0, rem.1, term2.0, term2.1);
        }
        if (rem.0 | rem.1) != 0 {
            q |= 1;
        }
    }

    sig_z = (sig_z << 6).wrapping_add((q >> 23) as u64);
    let sig_z_extra = (q as u64) << 41;

    round_pack_to_extf80(
        sign_z,
        exp_z,
        sig_z,
        sig_z_extra,
        softfloat_extF80_roundingPrecision(status),
        status,
    )
}
