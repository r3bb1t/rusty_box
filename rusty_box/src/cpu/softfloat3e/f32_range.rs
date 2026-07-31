#![allow(dead_code, non_snake_case)]
//! Single-precision exponent/mantissa extraction and scaling — the
//! primitives behind AVX-512 VGETEXPPS, VGETMANTPS and VSCALEFPS.
//! Ported from Bochs softfloat3e/f32_getExp.cc, f32_getMant.cc and
//! f32_scalef.cc.

use super::int_to_float::i32_to_f32;
use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Bochs softfloat3e/f32_getExp.cc `f32_getExp` — the unbiased exponent of
/// `a` as a float32.
pub(in crate::cpu) fn f32_get_exp(a: float32, status: &mut SoftFloatStatus) -> float32 {
    let mut exp_a = exp_f32(a);
    let sig_a = frac_f32(a);

    if exp_a == 0xFF {
        if sig_a != 0 {
            return softfloat_propagate_nan_f32(a, 0, status);
        }
        return pack_to_f32(false, 0xFF, 0);
    }
    if exp_a == 0 {
        if sig_a == 0 || softfloat_denormalsAreZeros(status) {
            return pack_to_f32(true, 0xFF, 0); // -inf
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        exp_a = norm_subnormal_f32_sig(sig_a).exp;
    }
    i32_to_f32(exp_a as i32 - 0x7F, status)
}

/// Bochs softfloat3e/f32_getMant.cc `f32_getMant`. `sign_ctrl` is imm8[3:2]
/// and `interv` is imm8[1:0] of VGETMANTPS.
pub(in crate::cpu) fn f32_get_mant(
    a: float32,
    status: &mut SoftFloatStatus,
    sign_ctrl: i32,
    interv: i32,
) -> float32 {
    let sign_a = sign_f32(a);
    let mut exp_a = exp_f32(a);
    let mut sig_a = frac_f32(a);
    // `~sign_ctrl & signA` in C, on a bool-as-int operand.
    let out_sign = ((!sign_ctrl) & (sign_a as i32)) & 1 != 0;

    if exp_a == 0xFF {
        if sig_a != 0 {
            return softfloat_propagate_nan_f32(a, 0, status);
        }
        if sign_a && (sign_ctrl & 0x2) != 0 {
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOAT32_DEFAULT_NAN;
        }
        return pack_to_f32(out_sign, 0x7F, 0);
    }
    if exp_a == 0 && (sig_a == 0 || softfloat_denormalsAreZeros(status)) {
        return pack_to_f32(out_sign, 0x7F, 0);
    }
    if sign_a && (sign_ctrl & 0x2) != 0 {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOAT32_DEFAULT_NAN;
    }
    if exp_a == 0 {
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f32_sig(sig_a);
        exp_a = ns.exp;
        sig_a = ns.sig & 0x7FFFFF;
    }

    match interv {
        0x0 => exp_a = 0x7F,                          // interval [1,2)
        0x1 => {
            exp_a -= 0x7F;
            exp_a = 0x7F - (exp_a & 0x1);
        } // interval [1/2,2)
        0x2 => exp_a = 0x7E,                          // interval [1/2,1)
        _ => exp_a = 0x7F - ((sig_a >> 22) & 0x1) as i16, // interval [3/4,3/2)
    }
    pack_to_f32(out_sign, exp_a, sig_a)
}

/// Bochs softfloat3e/f32_scalef.cc `f32_scalef` — `a` multiplied by 2 raised
/// to the integral part of `b`.
pub(in crate::cpu) fn f32_scalef(a: float32, b: float32, status: &mut SoftFloatStatus) -> float32 {
    let sign_a = sign_f32(a);
    let mut exp_a = exp_f32(a);
    let mut sig_a = frac_f32(a);
    let sign_b = sign_f32(b);
    let exp_b = exp_f32(b);
    let mut sig_b = frac_f32(b);
    let scale: i32;

    if exp_b == 0xFF && sig_b != 0 {
        return softfloat_propagate_nan_f32(a, b, status);
    }
    if softfloat_denormalsAreZeros(status) {
        if exp_a == 0 {
            sig_a = 0;
        }
        if exp_b == 0 {
            sig_b = 0;
        }
    }

    if exp_a == 0xFF {
        if sig_a != 0 {
            let a_is_signaling_nan = (sig_a & 0x0040_0000) == 0;
            if a_is_signaling_nan || exp_b != 0xFF || sig_b != 0 {
                return softfloat_propagate_nan_f32(a, b, status);
            }
            return if sign_b { 0 } else { pack_to_f32(false, 0xFF, 0) };
        }
        if exp_b == 0xFF && sign_b {
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOAT32_DEFAULT_NAN;
        }
        return a;
    }
    if exp_a == 0 {
        if sig_a == 0 {
            if exp_b == 0xFF && !sign_b {
                softfloat_raiseFlags(status, FLAG_INVALID);
                return FLOAT32_DEFAULT_NAN;
            }
            return pack_to_f32(sign_a, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
    }

    if (exp_b as u32 | sig_b) == 0 {
        return a;
    }
    if exp_b == 0xFF {
        return if sign_b {
            pack_to_f32(sign_a, 0, 0)
        } else {
            pack_to_f32(sign_a, 0xFF, 0)
        };
    }
    if exp_b >= 0x8E {
        // Obvious overflow / underflow: let the rounder produce the result.
        return round_pack_to_f32(sign_a, if sign_b { -0x7F } else { 0xFF }, sig_a, status);
    }
    if exp_b <= 0x7E {
        scale = -(sign_b as i32);
    } else {
        let shift_count = exp_b as i32 - 0x9E;
        sig_b = (sig_b | 0x800000) << 8;
        let mut s = (sig_b >> (-shift_count)) as i32;
        if sign_b {
            if (sig_b << (shift_count & 31)) != 0 {
                s += 1;
            }
            s = -s;
        }
        scale = s.clamp(-0x200, 0x200);
    }

    if exp_a != 0 {
        sig_a |= 0x0080_0000;
    } else {
        exp_a += 1;
    }
    exp_a += (scale - 1) as i16;
    sig_a <<= 7;
    norm_round_pack_to_f32(sign_a, exp_a, sig_a, status)
}
