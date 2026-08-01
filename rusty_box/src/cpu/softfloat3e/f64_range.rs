//! Double-precision exponent/mantissa extraction and scaling — the
//! primitives behind AVX-512 VGETEXPPD, VGETMANTPD and VSCALEFPD.
//! Ported from Bochs softfloat3e/f64_getExp.cc, f64_getMant.cc and
//! f64_scalef.cc.

use super::int_to_float::i32_to_f64;
use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Bochs softfloat3e/f64_getExp.cc `f64_getExp`.
pub(in crate::cpu) fn f64_get_exp(a: Float64, status: &mut SoftFloatStatus) -> Float64 {
    let mut exp_a = exp_f64(a);
    let sig_a = frac_f64(a);

    if exp_a == 0x7FF {
        if sig_a != 0 {
            return softfloat_propagate_nan_f64(a, 0, status);
        }
        return pack_to_f64(false, 0x7FF, 0);
    }
    if exp_a == 0 {
        if sig_a == 0 || softfloat_denormals_are_zeros(status) {
            return pack_to_f64(true, 0x7FF, 0); // -inf
        }
        softfloat_raise_flags(status, FLAG_DENORMAL);
        exp_a = norm_subnormal_f64_sig(sig_a).exp;
    }
    i32_to_f64(exp_a as i32 - 0x3FF)
}

/// Bochs softfloat3e/f64_getMant.cc `f64_getMant`.
pub(in crate::cpu) fn f64_get_mant(
    a: Float64,
    status: &mut SoftFloatStatus,
    sign_ctrl: i32,
    interv: i32,
) -> Float64 {
    let sign_a = sign_f64(a);
    let mut exp_a = exp_f64(a);
    let mut sig_a = frac_f64(a);
    let out_sign = ((!sign_ctrl) & (sign_a as i32)) & 1 != 0;

    if exp_a == 0x7FF {
        if sig_a != 0 {
            return softfloat_propagate_nan_f64(a, 0, status);
        }
        if sign_a && (sign_ctrl & 0x2) != 0 {
            softfloat_raise_flags(status, FLAG_INVALID);
            return FLOAT64_DEFAULT_NAN;
        }
        return pack_to_f64(out_sign, 0x3FF, 0);
    }
    if exp_a == 0 && (sig_a == 0 || softfloat_denormals_are_zeros(status)) {
        return pack_to_f64(out_sign, 0x3FF, 0);
    }
    if sign_a && (sign_ctrl & 0x2) != 0 {
        softfloat_raise_flags(status, FLAG_INVALID);
        return FLOAT64_DEFAULT_NAN;
    }
    if exp_a == 0 {
        softfloat_raise_flags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f64_sig(sig_a);
        exp_a = ns.exp;
        sig_a = ns.sig & 0xF_FFFF_FFFF_FFFF;
    }

    match interv {
        0x0 => exp_a = 0x3FF,                             // interval [1,2)
        0x1 => {
            exp_a -= 0x3FF;
            exp_a = 0x3FF - (exp_a & 0x1);
        } // interval [1/2,2)
        0x2 => exp_a = 0x3FE,                             // interval [1/2,1)
        _ => exp_a = 0x3FF - ((sig_a >> 51) & 0x1) as i16, // interval [3/4,3/2)
    }
    pack_to_f64(out_sign, exp_a, sig_a)
}

/// Bochs softfloat3e/f64_scalef.cc `f64_scalef`.
pub(in crate::cpu) fn f64_scalef(a: Float64, b: Float64, status: &mut SoftFloatStatus) -> Float64 {
    let sign_a = sign_f64(a);
    let mut exp_a = exp_f64(a);
    let mut sig_a = frac_f64(a);
    let sign_b = sign_f64(b);
    let exp_b = exp_f64(b);
    let mut sig_b = frac_f64(b);
    let scale: i32;

    if exp_b == 0x7FF && sig_b != 0 {
        return softfloat_propagate_nan_f64(a, b, status);
    }
    if softfloat_denormals_are_zeros(status) {
        if exp_a == 0 {
            sig_a = 0;
        }
        if exp_b == 0 {
            sig_b = 0;
        }
    }

    if exp_a == 0x7FF {
        if sig_a != 0 {
            let a_is_signaling_nan = (sig_a & 0x0008_0000_0000_0000) == 0;
            if a_is_signaling_nan || exp_b != 0x7FF || sig_b != 0 {
                return softfloat_propagate_nan_f64(a, b, status);
            }
            return if sign_b { 0 } else { pack_to_f64(false, 0x7FF, 0) };
        }
        if exp_b == 0x7FF && sign_b {
            softfloat_raise_flags(status, FLAG_INVALID);
            return FLOAT64_DEFAULT_NAN;
        }
        return a;
    }
    if exp_a == 0 {
        if sig_a == 0 {
            if exp_b == 0x7FF && !sign_b {
                softfloat_raise_flags(status, FLAG_INVALID);
                return FLOAT64_DEFAULT_NAN;
            }
            return pack_to_f64(sign_a, 0, 0);
        }
        softfloat_raise_flags(status, FLAG_DENORMAL);
    }

    if (exp_b as u64 | sig_b) == 0 {
        return a;
    }
    if exp_b == 0x7FF {
        return if sign_b {
            pack_to_f64(sign_a, 0, 0)
        } else {
            pack_to_f64(sign_a, 0x7FF, 0)
        };
    }
    if 0x40F <= exp_b {
        // Obvious overflow / underflow: let the rounder produce the result.
        return round_pack_to_f64(sign_a, if sign_b { -0x3FF } else { 0x7FF }, sig_a, status);
    }
    if exp_b < 0x3FF {
        scale = -(sign_b as i32);
    } else {
        sig_b |= 0x0010_0000_0000_0000;
        let shift_count = 0x433 - exp_b as u32;
        let prev_sig_b = sig_b;
        sig_b >>= shift_count;
        let mut s = sig_b as i32;
        if sign_b {
            if (sig_b << shift_count) != prev_sig_b {
                s += 1;
            }
            s = -s;
        }
        scale = s.clamp(-0x1000, 0x1000);
    }

    if exp_a != 0 {
        sig_a |= 0x0010_0000_0000_0000;
    } else {
        exp_a += 1;
    }
    exp_a += (scale - 1) as i16;
    sig_a <<= 10;
    norm_round_pack_to_f64(sign_a, exp_a, sig_a, status)
}
