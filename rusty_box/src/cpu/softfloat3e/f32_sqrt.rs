#![allow(dead_code, non_snake_case)]
//! Float32 square root. Ported from Berkeley SoftFloat 3e f32_sqrt.c.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

pub(in crate::cpu) fn f32_sqrt(mut a: float32, status: &mut SoftFloatStatus) -> float32 {
    let sign_a = sign_f32(a);
    let mut exp_a = exp_f32(a);
    let mut sig_a = frac_f32(a);

    if exp_a == 0xFF {
        if sig_a != 0 {
            return softfloat_propagate_nan_f32(a, 0, status);
        }
        if !sign_a {
            return a; // +inf
        }
        softfloat_raiseFlags(status, FLAG_INVALID); // sqrt(-inf)
        return FLOAT32_DEFAULT_NAN;
    }

    if softfloat_denormalsAreZeros(status) && exp_a == 0 {
        sig_a = 0;
        a = pack_to_f32(sign_a, 0, 0);
    }

    if sign_a {
        if exp_a == 0 && sig_a == 0 {
            return a; // -0
        }
        softfloat_raiseFlags(status, FLAG_INVALID); // sqrt(negative)
        return FLOAT32_DEFAULT_NAN;
    }

    if exp_a == 0 {
        if sig_a == 0 {
            return a; // +0
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f32_sig(sig_a);
        exp_a = ns.exp;
        sig_a = ns.sig;
    }

    let exp_z = ((exp_a - 0x7F) >> 1) + 0x7E;
    exp_a &= 1;
    let sig_a = (sig_a | 0x0080_0000) << 8;
    let mut sig_z =
        (((sig_a as u64) * (approx_recip_sqrt32_1(exp_a as u32, sig_a) as u64)) >> 32) as u32;
    if exp_a != 0 {
        sig_z >>= 1;
    }

    sig_z += 2;
    if (sig_z & 0x3F) < 2 {
        let shifted_sig_z = sig_z >> 2;
        let neg_rem = shifted_sig_z.wrapping_mul(shifted_sig_z);
        sig_z &= !3u32;
        if (neg_rem & 0x8000_0000) != 0 {
            sig_z |= 1;
        } else if neg_rem != 0 {
            sig_z -= 1;
        }
    }
    round_pack_to_f32(false, exp_z, sig_z, status)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sqrt(a: f32) -> f32 {
        let mut st = SoftFloatStatus::default();
        f32::from_bits(f32_sqrt(a.to_bits(), &mut st))
    }
    #[test]
    fn f32_sqrt_matches_native_rne() {
        let vals: [f32; 18] = [
            0.0, -0.0, 1.0, 2.0, 4.0, 0.25, 3.14159, 1e18, 1e-18,
            f32::MIN_POSITIVE, 123456.789, 0.1, 7.0, 65504.0, 2.0000002,
            9.0, 1.5, 1000000.0,
        ];
        for &a in &vals {
            let (n, s) = (a.sqrt(), sqrt(a));
            if n.is_nan() {
                assert!(s.is_nan(), "sqrt {a}");
            } else {
                assert_eq!(s.to_bits(), n.to_bits(), "sqrt {a}");
            }
        }
        assert!(sqrt(-1.0).is_nan());
        assert_eq!(sqrt(f32::INFINITY).to_bits(), f32::INFINITY.to_bits());
    }
}
