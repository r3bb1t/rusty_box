#![allow(dead_code, non_snake_case)]
//! Float32 division. Ported from Berkeley SoftFloat 3e f32_div.c
//! (SOFTFLOAT_FAST_DIV64TO32 path, as configured by Bochs).

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

pub(in crate::cpu) fn f32_div(a: float32, b: float32, status: &mut SoftFloatStatus) -> float32 {
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

    if exp_a == 0xFF {
        if sig_a != 0 {
            return softfloat_propagate_nan_f32(a, b, status);
        }
        if exp_b == 0xFF {
            if sig_b != 0 {
                return softfloat_propagate_nan_f32(a, b, status);
            }
            // invalid: inf/inf
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOAT32_DEFAULT_NAN;
        }
        if sig_b != 0 && exp_b == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_to_f32(sign_z, 0xFF, 0); // infinity
    }
    if exp_b == 0xFF {
        if sig_b != 0 {
            return softfloat_propagate_nan_f32(a, b, status);
        }
        if sig_a != 0 && exp_a == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_to_f32(sign_z, 0, 0); // zero (finite / inf)
    }

    if exp_b == 0 {
        if sig_b == 0 {
            if exp_a == 0 && sig_a == 0 {
                // 0/0 invalid
                softfloat_raiseFlags(status, FLAG_INVALID);
                return FLOAT32_DEFAULT_NAN;
            }
            softfloat_raiseFlags(status, FLAG_INFINITE); // divide-by-zero
            return pack_to_f32(sign_z, 0xFF, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f32_sig(sig_b);
        exp_b = ns.exp;
        sig_b = ns.sig;
    }
    if exp_a == 0 {
        if sig_a == 0 {
            return pack_to_f32(sign_z, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let ns = norm_subnormal_f32_sig(sig_a);
        exp_a = ns.exp;
        sig_a = ns.sig;
    }

    let mut exp_z = exp_a - exp_b + 0x7E;
    sig_a |= 0x0080_0000;
    sig_b |= 0x0080_0000;
    let sig64_a: u64;
    if sig_a < sig_b {
        exp_z -= 1;
        sig64_a = (sig_a as u64) << 31;
    } else {
        sig64_a = (sig_a as u64) << 30;
    }
    let mut sig_z = (sig64_a / (sig_b as u64)) as u32;
    if (sig_z & 0x3F) == 0 {
        sig_z |= ((sig_b as u64) * (sig_z as u64) != sig64_a) as u32;
    }
    round_pack_to_f32(sign_z, exp_z, sig_z, status)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn div(a: f32, b: f32) -> f32 {
        let mut st = SoftFloatStatus::default();
        f32::from_bits(f32_div(a.to_bits(), b.to_bits(), &mut st))
    }
    #[test]
    fn f32_div_matches_native_rne() {
        let vals: [f32; 20] = [
            0.0, -0.0, 1.0, -1.0, 2.0, 0.5, 3.14159, -2.71828, 1e18, 1e-18,
            f32::MIN_POSITIVE, 123456.789, 0.1, 0.3, 7.0, 65504.0, 1.0000001,
            9.0, 3.0, 1.5,
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
        assert_eq!(div(1.0, 0.0).to_bits(), f32::INFINITY.to_bits());
        assert_eq!(div(-1.0, 0.0).to_bits(), f32::NEG_INFINITY.to_bits());
        assert!(div(0.0, 0.0).is_nan());
    }
}
