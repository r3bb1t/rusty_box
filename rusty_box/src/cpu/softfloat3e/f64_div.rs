//! Float64 division. Ported from Berkeley SoftFloat 3e f64_div.c.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

pub(in crate::cpu) fn f64_div(a: Float64, b: Float64, status: &mut SoftFloatStatus) -> Float64 {
    let sign_a = sign_f64(a);
    let mut exp_a = exp_f64(a);
    let mut sig_a = frac_f64(a);
    let sign_b = sign_f64(b);
    let mut exp_b = exp_f64(b);
    let mut sig_b = frac_f64(b);
    let sign_z = sign_a ^ sign_b;

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
            return softfloat_propagate_nan_f64(a, b, status);
        }
        if exp_b == 0x7FF {
            if sig_b != 0 {
                return softfloat_propagate_nan_f64(a, b, status);
            }
            // inf / inf
            softfloat_raise_flags(status, FLAG_INVALID);
            return FLOAT64_DEFAULT_NAN;
        }
        if sig_b != 0 && exp_b == 0 {
            softfloat_raise_flags(status, FLAG_DENORMAL);
        }
        return pack_to_f64(sign_z, 0x7FF, 0); // infinity
    }
    if exp_b == 0x7FF {
        if sig_b != 0 {
            return softfloat_propagate_nan_f64(a, b, status);
        }
        if sig_a != 0 && exp_a == 0 {
            softfloat_raise_flags(status, FLAG_DENORMAL);
        }
        return pack_to_f64(sign_z, 0, 0); // finite / inf
    }

    if exp_b == 0 {
        if sig_b == 0 {
            if (exp_a as u64 | sig_a) == 0 {
                // 0 / 0
                softfloat_raise_flags(status, FLAG_INVALID);
                return FLOAT64_DEFAULT_NAN;
            }
            softfloat_raise_flags(status, FLAG_INFINITE); // divide-by-zero
            return pack_to_f64(sign_z, 0x7FF, 0);
        }
        softfloat_raise_flags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f64_sig(sig_b);
        exp_b = ns.exp;
        sig_b = ns.sig;
    }
    if exp_a == 0 {
        if sig_a == 0 {
            return pack_to_f64(sign_z, 0, 0);
        }
        softfloat_raise_flags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f64_sig(sig_a);
        exp_a = ns.exp;
        sig_a = ns.sig;
    }

    let mut exp_z = exp_a - exp_b + 0x3FE;
    sig_a |= 0x0010_0000_0000_0000;
    sig_b |= 0x0010_0000_0000_0000;
    if sig_a < sig_b {
        exp_z -= 1;
        sig_a <<= 11;
    } else {
        sig_a <<= 10;
    }
    sig_b <<= 11;

    let recip32 = approx_recip32_1((sig_b >> 32) as u32).wrapping_sub(2);
    let sig32_z = ((((sig_a >> 32) as u32 as u64).wrapping_mul(recip32 as u64)) >> 32) as u32;
    let mut double_term = sig32_z << 1;
    let mut rem = (sig_a
        .wrapping_sub((double_term as u64).wrapping_mul((sig_b >> 32) as u32 as u64))
        << 28)
        .wrapping_sub((double_term as u64).wrapping_mul(((sig_b as u32) >> 4) as u64));
    let mut q = (((rem >> 32) as u32 as u64).wrapping_mul(recip32 as u64) >> 32) as u32;
    q = q.wrapping_add(4);
    let mut sig_z = ((sig32_z as u64) << 32).wrapping_add((q as u64) << 4);

    if (sig_z & 0x1FF) < (4 << 4) {
        q &= !7;
        sig_z &= !0x7Fu64;
        double_term = q << 1;
        rem = (rem
            .wrapping_sub((double_term as u64).wrapping_mul((sig_b >> 32) as u32 as u64))
            << 28)
            .wrapping_sub((double_term as u64).wrapping_mul(((sig_b as u32) >> 4) as u64));
        if (rem & 0x8000_0000_0000_0000) != 0 {
            sig_z = sig_z.wrapping_sub(1 << 7);
        } else if rem != 0 {
            sig_z |= 1;
        }
    }
    round_pack_to_f64(sign_z, exp_z, sig_z, status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn div(a: f64, b: f64) -> f64 {
        let mut st = SoftFloatStatus::default();
        f64::from_bits(f64_div(a.to_bits(), b.to_bits(), &mut st))
    }

    #[test]
    fn f64_div_matches_native_rne() {
        let vals: [f64; 22] = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            2.0,
            0.5,
            core::f64::consts::PI,
            -core::f64::consts::E,
            1e150,
            1e-150,
            1e300,
            1e-300,
            f64::MIN_POSITIVE,
            f64::MIN_POSITIVE / 4.0,
            123456.789,
            0.1,
            0.3,
            7.0,
            9.0,
            3.0,
            1.5,
            f64::MAX,
        ];
        for &a in &vals {
            for &b in &vals {
                let (n, s) = (a / b, div(a, b));
                if n.is_nan() {
                    assert!(s.is_nan(), "{a} / {b}");
                } else {
                    assert_eq!(s.to_bits(), n.to_bits(), "{a} / {b}");
                }
            }
        }
        assert_eq!(div(1.0, 0.0).to_bits(), f64::INFINITY.to_bits());
        assert_eq!(div(-1.0, 0.0).to_bits(), f64::NEG_INFINITY.to_bits());
        assert!(div(0.0, 0.0).is_nan());
        assert!(div(f64::INFINITY, f64::INFINITY).is_nan());
    }
}
