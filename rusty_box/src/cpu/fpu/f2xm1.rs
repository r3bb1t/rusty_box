#![allow(dead_code)]
//! F2XM1 implementation: compute 2^x - 1.
//! Ported from Bochs cpu/fpu/f2xm1.cc using Float128 polynomial evaluation.

use super::super::softfloat3e::f128::*;
use super::super::softfloat3e::internals::*;
use super::super::softfloat3e::primitives::*;
use super::super::softfloat3e::softfloat::*;
use super::super::softfloat3e::softfloat_types::floatx80;
use super::super::softfloat3e::specialize::*;
use super::poly::*;

/// 1.0 in floatx80 format
const FLOATX80_ONE: floatx80 = floatx80 {
    signif: 0x8000000000000000,
    sign_exp: 0x3FFF,
};

/// -1.0 in floatx80 format
const FLOATX80_NEG_ONE: floatx80 = floatx80 {
    signif: 0x8000000000000000,
    sign_exp: 0xBFFF,
};

/// -0.5 in floatx80 format
const FLOATX80_NEG_HALF: floatx80 = floatx80 {
    signif: 0x8000000000000000,
    sign_exp: 0xBFFE,
};

// --- exp_arr[15] from Bochs f2xm1.cc ---

const EXP_ARR: [Float128; 15] = [
    Float128::new(0x3fff000000000000, 0x0000000000000000), /*  1 */
    Float128::new(0x3ffe000000000000, 0x0000000000000000), /*  2 */
    Float128::new(0x3ffc555555555555, 0x5555555555555555), /*  3 */
    Float128::new(0x3ffa555555555555, 0x5555555555555555), /*  4 */
    Float128::new(0x3ff8111111111111, 0x1111111111111111), /*  5 */
    Float128::new(0x3ff56c16c16c16c1, 0x6c16c16c16c16c17), /*  6 */
    Float128::new(0x3ff2a01a01a01a01, 0xa01a01a01a01a01a), /*  7 */
    Float128::new(0x3fefa01a01a01a01, 0xa01a01a01a01a01a), /*  8 */
    Float128::new(0x3fec71de3a556c73, 0x38faac1c88e50017), /*  9 */
    Float128::new(0x3fe927e4fb7789f5, 0xc72ef016d3ea6679), /* 10 */
    Float128::new(0x3fe5ae64567f544e, 0x38fe747e4b837dc7), /* 11 */
    Float128::new(0x3fe21eed8eff8d89, 0x7b544da987acfe85), /* 12 */
    Float128::new(0x3fde6124613a86d0, 0x97ca38331d23af68), /* 13 */
    Float128::new(0x3fda93974a8c07c9, 0xd20badf145dfa3e5), /* 14 */
    Float128::new(0x3fd6ae7f3e733b81, 0xf11d8656b0ee8cb0), /* 15 */
];

/// Polynomial approximation for e^x - 1 (from Bochs f2xm1.cc poly_exp).
/// Required: -1 < x < 1
pub(crate) fn poly_exp(x: Float128, status: &mut SoftFloatStatus) -> Float128 {
    let t = eval_poly(x, &EXP_ARR, status);
    f128_mul(t, x, status)
}

/// Compute 2^a - 1 for extended precision float `a`.
/// Ported from Bochs f2xm1.cc using Float128 polynomial evaluation.
pub(crate) fn f2xm1_impl(a: floatx80, status: &mut SoftFloatStatus) -> floatx80 {
    // Handle unsupported encodings
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOATX80_DEFAULT_NAN;
    }

    let a_sig = extf80_fraction(a);
    let mut a_exp = extf80_exp(a);
    let a_sign = extf80_sign(a);

    // NaN or Infinity
    if a_exp == 0x7FFF {
        if (a_sig << 1) != 0 {
            return softfloat_propagate_nan_extf80(a.sign_exp, a_sig, 0, 0, status);
        }
        // 2^(+inf) - 1 = +inf; 2^(-inf) - 1 = -1
        return if a_sign { FLOATX80_NEG_ONE } else { a };
    }

    // Zero or denormal
    if a_exp == 0 {
        if a_sig == 0 {
            return a; // 2^0 - 1 = 0
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL | FLAG_INEXACT);
        let norm = norm_subnormal_extf80_sig(a_sig);
        a_exp = norm.exp + 1;
        let a_sig = norm.sig;

        // tiny_argument: 2^x - 1 ~ x * ln(2) via 192-bit integer multiply
        let (z_sig0, z_sig1, _z_sig2) = mul128_by_64_to_192(LN2_SIG_HI, LN2_SIG_LO, a_sig);
        let (mut z_sig0, mut z_sig1) = (z_sig0, z_sig1);
        if (z_sig0 as i64) >= 0 {
            let (s0, s1) = short_shift_left128(z_sig0, z_sig1, 1);
            z_sig0 = s0;
            z_sig1 = s1;
            a_exp -= 1;
        }
        return round_pack_to_extf80(a_sign, a_exp, z_sig0, z_sig1, 80, status);
    }

    softfloat_raiseFlags(status, FLAG_INEXACT);

    if a_exp < 0x3FFF {
        if a_exp < FLOATX80_EXP_BIAS - 68 {
            // tiny_argument: 2^x - 1 ~ x * ln(2) via 192-bit integer multiply
            let (z_sig0, z_sig1, _z_sig2) = mul128_by_64_to_192(LN2_SIG_HI, LN2_SIG_LO, a_sig);
            let (mut z_sig0, mut z_sig1) = (z_sig0, z_sig1);
            if (z_sig0 as i64) >= 0 {
                let (s0, s1) = short_shift_left128(z_sig0, z_sig1, 1);
                z_sig0 = s0;
                z_sig1 = s1;
            }
            return round_pack_to_extf80(a_sign, a_exp, z_sig0, z_sig1, 80, status);
        }

        // General case: 2^x - 1 = e^(x*ln2) - 1 using Float128 polynomial
        let x = extf80_to_f128(a, status);
        let x_ln2 = f128_mul(x, FLOAT128_LN2, status);
        let result = poly_exp(x_ln2, status);
        return f128_to_extf80(result, status);
    }

    // |a| >= 1: special cases
    // -1.0 exactly: 2^(-1) - 1 = -0.5
    if a.sign_exp == 0xBFFF && (a_sig << 1) == 0 {
        return FLOATX80_NEG_HALF;
    }

    // For |a| >= 1, just return a (x87 spec says -1 <= x <= 1, Bochs returns a)
    a
}
