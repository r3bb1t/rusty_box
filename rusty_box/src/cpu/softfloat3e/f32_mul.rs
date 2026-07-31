#![allow(dead_code, non_snake_case)]
//! Float32 multiplication. Ported from Berkeley SoftFloat 3e f32_mul.c.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

pub(in crate::cpu) fn f32_mul(a: float32, b: float32, status: &mut SoftFloatStatus) -> float32 {
    let sign_a = sign_f32(a);
    let mut exp_a = exp_f32(a);
    let mut sig_a = frac_f32(a);
    let sign_b = sign_f32(b);
    let mut exp_b = exp_f32(b);
    let mut sig_b = frac_f32(b);
    let sign_z = sign_a ^ sign_b;

    if softfloat_denormalsAreZeros(status) {
        if exp_a == 0 {
            sig_a = 0;
        }
        if exp_b == 0 {
            sig_b = 0;
        }
    }

    // inf/NaN argument handling (goto infArg / propagateNaN).
    let inf_arg = |sign_z: bool, mag_bits: u32, status: &mut SoftFloatStatus| -> float32 {
        if mag_bits == 0 {
            softfloat_raiseFlags(status, FLAG_INVALID);
            FLOAT32_DEFAULT_NAN
        } else {
            pack_to_f32(sign_z, 0xFF, 0)
        }
    };

    if exp_a == 0xFF {
        if sig_a != 0 || (exp_b == 0xFF && sig_b != 0) {
            return softfloat_propagate_nan_f32(a, b, status);
        }
        let mag_bits = (exp_b as u32) | sig_b;
        if sig_b != 0 && exp_b == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return inf_arg(sign_z, mag_bits, status);
    }
    if exp_b == 0xFF {
        if sig_b != 0 {
            return softfloat_propagate_nan_f32(a, b, status);
        }
        let mag_bits = (exp_a as u32) | sig_a;
        if sig_a != 0 && exp_a == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return inf_arg(sign_z, mag_bits, status);
    }

    if exp_a == 0 {
        if sig_a == 0 {
            if sig_b != 0 && exp_b == 0 {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }
            return pack_to_f32(sign_z, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f32_sig(sig_a);
        exp_a = ns.exp;
        sig_a = ns.sig;
    }
    if exp_b == 0 {
        if sig_b == 0 {
            if sig_a != 0 && exp_a == 0 {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }
            return pack_to_f32(sign_z, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f32_sig(sig_b);
        exp_b = ns.exp;
        sig_b = ns.sig;
    }

    let mut exp_z = exp_a + exp_b - 0x7F;
    let sig_a = (sig_a | 0x0080_0000) << 7;
    let sig_b = (sig_b | 0x0080_0000) << 8;
    let mut sig_z = short_shift_right_jam64((sig_a as u64) * (sig_b as u64), 32) as u32;
    if sig_z < 0x4000_0000 {
        exp_z -= 1;
        sig_z <<= 1;
    }
    round_pack_to_f32(sign_z, exp_z, sig_z, status)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mul(a: f32, b: f32) -> f32 {
        let mut st = SoftFloatStatus::default();
        f32::from_bits(f32_mul(a.to_bits(), b.to_bits(), &mut st))
    }
    #[test]
    fn f32_mul_matches_native_rne() {
        let vals: [f32; 22] = [
            0.0, -0.0, 1.0, -1.0, 2.0, 0.5, 3.14159, -2.71828, 1e18, -1e18,
            1e-18, f32::MIN_POSITIVE, f32::MIN_POSITIVE * 3.0, 123456.789, 0.1,
            0.3, 7.0, 65504.0, 1.0000001, 0.9999999, 16_777_217.0, 1.5,
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
        assert!(mul(0.0, f32::INFINITY).is_nan()); // 0 * inf = NaN
        assert_eq!(mul(f32::MAX, f32::MAX).to_bits(), f32::INFINITY.to_bits());
    }
}
