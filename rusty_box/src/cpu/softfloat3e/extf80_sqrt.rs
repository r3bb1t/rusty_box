#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! ExtFloat80 square root.
//! Ported from Berkeley SoftFloat 3e: extF80_sqrt.c

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

pub fn extf80_sqrt(a: floatx80, status: &mut SoftFloatStatus) -> floatx80 {
    // Handle unsupported encodings
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOATX80_DEFAULT_NAN;
    }

    let sign_a = sign_extf80(a.sign_exp);
    let mut exp_a = exp_extf80(a.sign_exp) as i32;
    let mut sig_a = a.signif;

    // NaN / Inf
    if exp_a == 0x7FFF {
        if (sig_a & 0x7FFFFFFFFFFFFFFF) != 0 {
            return softfloat_propagate_nan_extf80(a.sign_exp, a.signif, 0, 0, status);
        }
        if !sign_a {
            return a; // sqrt(+Inf) = +Inf
        }
        // sqrt(-Inf) = invalid
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOATX80_DEFAULT_NAN;
    }

    // Negative
    if sign_a {
        if (exp_a as u64 | sig_a) == 0 {
            return pack_floatx80(sign_a, 0, 0); // sqrt(-0) = -0
        }
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOATX80_DEFAULT_NAN;
    }

    // Handle denormals
    if exp_a == 0 {
        exp_a = 1;
        if sig_a != 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
    }
    if (sig_a & 0x8000000000000000) == 0 {
        if sig_a == 0 {
            return pack_floatx80(false, 0, 0);
        }
        let norm = norm_subnormal_extf80_sig(sig_a);
        exp_a += norm.exp;
        sig_a = norm.sig;
    }

    // Main sqrt computation
    let exp_z = ((exp_a - 0x3FFF) >> 1) + 0x3FFF;
    let exp_a_odd = exp_a & 1;
    let sig32_a = (sig_a >> 32) as u32;
    let recip_sqrt32 = approx_recip_sqrt32_1(exp_a_odd as u32, sig32_a);
    let mut sig32_z = (((sig32_a as u64).wrapping_mul(recip_sqrt32 as u64)) >> 32) as u32;

    let mut rem: (u64, u64);
    if exp_a_odd != 0 {
        sig32_z >>= 1;
        rem = short_shift_left128(0, sig_a, 61);
    } else {
        rem = short_shift_left128(0, sig_a, 62);
    }
    rem.0 = rem
        .0
        .wrapping_sub((sig32_z as u64).wrapping_mul(sig32_z as u64));

    // First Newton-Raphson refinement
    let mut q = ((rem.0 >> 2) as u32 as u64).wrapping_mul(recip_sqrt32 as u64) >> 32;
    let x64 = (sig32_z as u64) << 32;
    let mut sig_z = x64.wrapping_add(q << 3);
    let y = short_shift_left128(rem.0, rem.1, 29);

    loop {
        let term = mul64_by_shifted32_to128(x64.wrapping_add(sig_z), q as u32);
        rem = sub128(y.0, y.1, term.0, term.1);
        if (rem.0 & 0x8000000000000000) == 0 {
            break;
        }
        q -= 1;
        sig_z -= 1 << 3;
    }

    // Second refinement
    q = (((rem.0 >> 2).wrapping_mul(recip_sqrt32 as u64)) >> 32).wrapping_add(2);
    let x64 = sig_z;
    sig_z = (sig_z << 1).wrapping_add(q >> 25);
    let mut sig_z_extra = q << 39;

    if (q & 0xFFFFFF) <= 2 {
        let q_masked = q & !0xFFFFu64;
        sig_z_extra = q_masked << 39;
        let term = mul64_by_shifted32_to128(x64.wrapping_add(q_masked >> 27), q_masked as u32);
        let x64_2 = ((q_masked << 5) as u32 as u64).wrapping_mul(q_masked as u32 as u64);
        let term = add128(term.0, term.1, 0, x64_2);
        let rem_shifted = short_shift_left128(rem.0, rem.1, 28);
        let rem2 = sub128(rem_shifted.0, rem_shifted.1, term.0, term.1);
        if (rem2.0 & 0x8000000000000000) != 0 {
            if sig_z_extra == 0 {
                sig_z -= 1;
            }
            sig_z_extra = sig_z_extra.wrapping_sub(1);
        } else {
            if (rem2.0 | rem2.1) != 0 {
                sig_z_extra |= 1;
            }
        }
    }

    round_pack_to_extf80(
        false,
        exp_z,
        sig_z,
        sig_z_extra,
        softfloat_extF80_roundingPrecision(status),
        status,
    )
}
