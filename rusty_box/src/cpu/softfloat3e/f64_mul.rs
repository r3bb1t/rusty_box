//! Float64 multiplication. Ported from Berkeley SoftFloat 3e f64_mul.c.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

pub(in crate::cpu) fn f64_mul(a: Float64, b: Float64, status: &mut SoftFloatStatus) -> Float64 {
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

    // `magBits` distinguishes inf*0 (invalid) from inf*finite (infinity).
    if exp_a == 0x7FF {
        if sig_a != 0 || (exp_b == 0x7FF && sig_b != 0) {
            return softfloat_propagate_nan_f64(a, b, status);
        }
        let mag_bits = (exp_b as u64) | sig_b;
        if sig_b != 0 && exp_b == 0 {
            softfloat_raise_flags(status, FLAG_DENORMAL);
        }
        return mul_inf_arg(sign_z, mag_bits, status);
    }
    if exp_b == 0x7FF {
        if sig_b != 0 {
            return softfloat_propagate_nan_f64(a, b, status);
        }
        let mag_bits = (exp_a as u64) | sig_a;
        if sig_a != 0 && exp_a == 0 {
            softfloat_raise_flags(status, FLAG_DENORMAL);
        }
        return mul_inf_arg(sign_z, mag_bits, status);
    }

    if exp_a == 0 {
        if sig_a == 0 {
            if sig_b != 0 && exp_b == 0 {
                softfloat_raise_flags(status, FLAG_DENORMAL);
            }
            return pack_to_f64(sign_z, 0, 0);
        }
        softfloat_raise_flags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f64_sig(sig_a);
        exp_a = ns.exp;
        sig_a = ns.sig;
    }
    if exp_b == 0 {
        if sig_b == 0 {
            if sig_a != 0 && exp_a == 0 {
                softfloat_raise_flags(status, FLAG_DENORMAL);
            }
            return pack_to_f64(sign_z, 0, 0);
        }
        softfloat_raise_flags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f64_sig(sig_b);
        exp_b = ns.exp;
        sig_b = ns.sig;
    }

    let mut exp_z = exp_a + exp_b - 0x3FF;
    sig_a = (sig_a | 0x0010_0000_0000_0000) << 10;
    sig_b = (sig_b | 0x0010_0000_0000_0000) << 11;
    let (sig128_z_64, sig128_z_0) = mul64_to_128(sig_a, sig_b);
    let mut sig_z = sig128_z_64 | ((sig128_z_0 != 0) as u64);
    if sig_z < 0x4000_0000_0000_0000 {
        exp_z -= 1;
        sig_z <<= 1;
    }
    round_pack_to_f64(sign_z, exp_z, sig_z, status)
}

/// Berkeley SoftFloat `f64_mul` `infArg` label: an infinite operand yields
/// infinity, except against a zero operand, which is invalid.
fn mul_inf_arg(sign_z: bool, mag_bits: u64, status: &mut SoftFloatStatus) -> Float64 {
    if mag_bits == 0 {
        softfloat_raise_flags(status, FLAG_INVALID);
        FLOAT64_DEFAULT_NAN
    } else {
        pack_to_f64(sign_z, 0x7FF, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mul(a: f64, b: f64) -> f64 {
        let mut st = SoftFloatStatus::default();
        f64::from_bits(f64_mul(a.to_bits(), b.to_bits(), &mut st))
    }

    #[test]
    fn f64_mul_matches_native_rne() {
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
            3.0,
            1.0000000000000002,
            f64::MAX,
            f64::MIN,
        ];
        for &a in &vals {
            for &b in &vals {
                let (n, s) = (a * b, mul(a, b));
                if n.is_nan() {
                    assert!(s.is_nan(), "{a} * {b}");
                } else {
                    assert_eq!(s.to_bits(), n.to_bits(), "{a} * {b}");
                }
            }
        }
    }

    #[test]
    fn f64_mul_specials() {
        assert!(mul(f64::INFINITY, 0.0).is_nan()); // inf * 0 → invalid
        assert_eq!(
            mul(f64::INFINITY, 2.0).to_bits(),
            f64::INFINITY.to_bits()
        );
        assert_eq!(
            mul(f64::INFINITY, -2.0).to_bits(),
            f64::NEG_INFINITY.to_bits()
        );
        assert_eq!(mul(f64::MAX, f64::MAX).to_bits(), f64::INFINITY.to_bits());
    }
}
