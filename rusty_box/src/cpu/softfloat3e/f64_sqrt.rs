#![allow(dead_code, non_snake_case)]
//! Float64 square root. Ported from Berkeley SoftFloat 3e f64_sqrt.c.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

pub(crate) fn f64_sqrt(a: float64, status: &mut SoftFloatStatus) -> float64 {
    let sign_a = sign_f64(a);
    let mut exp_a = exp_f64(a);
    let mut sig_a = frac_f64(a);
    let mut ui_a = a;

    if exp_a == 0x7FF {
        if sig_a != 0 {
            return softfloat_propagate_nan_f64(a, 0, status);
        }
        if !sign_a {
            return a; // +inf
        }
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOAT64_DEFAULT_NAN;
    }

    if softfloat_denormalsAreZeros(status) && exp_a == 0 {
        sig_a = 0;
        ui_a = pack_to_f64(sign_a, 0, 0);
    }

    if sign_a {
        if (exp_a as u64 | sig_a) == 0 {
            return ui_a; // -0
        }
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOAT64_DEFAULT_NAN;
    }

    if exp_a == 0 {
        if sig_a == 0 {
            return ui_a; // +0
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f64_sig(sig_a);
        exp_a = ns.exp;
        sig_a = ns.sig;
    }

    // `sig32_z` is guaranteed to be a lower bound on the square root of
    // `sig32_a`, which makes it also a lower bound on the square root of
    // `sig_a`.
    let exp_z = ((exp_a - 0x3FF) >> 1) + 0x3FE;
    exp_a &= 1;
    sig_a |= 0x0010_0000_0000_0000;
    let sig32_a = (sig_a >> 21) as u32;
    let recip_sqrt32 = approx_recip_sqrt32_1(exp_a as u32, sig32_a);
    let mut sig32_z = (((sig32_a as u64) * (recip_sqrt32 as u64)) >> 32) as u32;
    if exp_a != 0 {
        sig_a <<= 8;
        sig32_z >>= 1;
    } else {
        sig_a <<= 9;
    }
    let mut rem = sig_a.wrapping_sub((sig32_z as u64).wrapping_mul(sig32_z as u64));
    let q = ((((rem >> 2) as u32 as u64).wrapping_mul(recip_sqrt32 as u64)) >> 32) as u32;
    let mut sig_z = (((sig32_z as u64) << 32) | (1 << 5)).wrapping_add((q as u64) << 3);

    if (sig_z & 0x1FF) < 0x22 {
        sig_z &= !0x3Fu64;
        let shifted_sig_z = sig_z >> 6;
        rem = (sig_a << 52).wrapping_sub(shifted_sig_z.wrapping_mul(shifted_sig_z));
        if (rem & 0x8000_0000_0000_0000) != 0 {
            sig_z -= 1;
        } else if rem != 0 {
            sig_z |= 1;
        }
    }
    round_pack_to_f64(false, exp_z, sig_z, status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqrt(a: f64) -> f64 {
        let mut st = SoftFloatStatus::default();
        f64::from_bits(f64_sqrt(a.to_bits(), &mut st))
    }

    #[test]
    fn f64_sqrt_matches_native_rne() {
        let vals: [f64; 20] = [
            0.0,
            1.0,
            2.0,
            3.0,
            4.0,
            0.5,
            core::f64::consts::PI,
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
            1.0000000000000002,
            f64::MAX,
        ];
        for &a in &vals {
            let (n, s) = (a.sqrt(), sqrt(a));
            assert_eq!(s.to_bits(), n.to_bits(), "sqrt({a})");
        }
    }

    #[test]
    fn f64_sqrt_specials() {
        assert_eq!(sqrt(-0.0).to_bits(), (-0.0f64).to_bits());
        assert!(sqrt(-1.0).is_nan()); // invalid
        assert_eq!(sqrt(f64::INFINITY).to_bits(), f64::INFINITY.to_bits());
        assert!(sqrt(f64::NEG_INFINITY).is_nan());
    }
}
