#![allow(dead_code)]
//! F2XM1 implementation: compute 2^x - 1.
//! Ported from Bochs cpu/fpu/f2xm1.cc using Float128 polynomial evaluation.

use super::super::softfloat3e::f128::*;
use super::super::softfloat3e::internals::*;
use super::super::softfloat3e::primitives::*;
use super::super::softfloat3e::softfloat::*;
use super::super::softfloat3e::softfloat_types::ExtFloat80;
use super::super::softfloat3e::specialize::*;
use super::poly::*;

/// 1.0 in ExtFloat80 format
const FLOATX80_ONE: ExtFloat80 = ExtFloat80 {
    signif: 0x8000000000000000,
    sign_exp: 0x3FFF,
};

/// -1.0 in ExtFloat80 format
const FLOATX80_NEG_ONE: ExtFloat80 = ExtFloat80 {
    signif: 0x8000000000000000,
    sign_exp: 0xBFFF,
};

/// -0.5 in ExtFloat80 format
const FLOATX80_NEG_HALF: ExtFloat80 = ExtFloat80 {
    signif: 0x8000000000000000,
    sign_exp: 0xBFFE,
};

// --- exp_arr[15] from Bochs f2xm1.cc ---

const EXP_ARR: [Float128; 15] = [
    Float128::new(0x3FFF000000000000, 0x0000000000000000), /*  1 */
    Float128::new(0x3FFE000000000000, 0x0000000000000000), /*  2 */
    Float128::new(0x3FFC555555555555, 0x5555555555555555), /*  3 */
    Float128::new(0x3FFA555555555555, 0x5555555555555555), /*  4 */
    Float128::new(0x3FF8111111111111, 0x1111111111111111), /*  5 */
    Float128::new(0x3FF56C16C16C16C1, 0x6C16C16C16C16C17), /*  6 */
    Float128::new(0x3FF2A01A01A01A01, 0xA01A01A01A01A01A), /*  7 */
    Float128::new(0x3FEFA01A01A01A01, 0xA01A01A01A01A01A), /*  8 */
    Float128::new(0x3FEC71DE3A556C73, 0x38FAAC1C88E50017), /*  9 */
    Float128::new(0x3FE927E4FB7789F5, 0xC72EF016D3EA6679), /* 10 */
    Float128::new(0x3FE5AE64567F544E, 0x38FE747E4B837DC7), /* 11 */
    Float128::new(0x3FE21EED8EFF8D89, 0x7B544DA987ACFE85), /* 12 */
    Float128::new(0x3FDE6124613A86D0, 0x97CA38331D23AF68), /* 13 */
    Float128::new(0x3FDA93974A8C07C9, 0xD20BADF145DFA3E5), /* 14 */
    Float128::new(0x3FD6AE7F3E733B81, 0xF11D8656B0EE8CB0), /* 15 */
];

/// Polynomial approximation for e^x - 1 (from Bochs f2xm1.cc poly_exp).
/// Required: -1 < x < 1
pub(in crate::cpu) fn poly_exp(x: Float128, status: &mut SoftFloatStatus) -> Float128 {
    let t = eval_poly(x, &EXP_ARR, status);
    f128_mul(t, x, status)
}

/// Compute 2^a - 1 for extended precision float `a`.
/// Ported from Bochs f2xm1.cc using Float128 polynomial evaluation.
pub(in crate::cpu) fn f2xm1_impl(a: ExtFloat80, status: &mut SoftFloatStatus) -> ExtFloat80 {
    // Handle unsupported encodings
    if extf80_is_unsupported(a) {
        softfloat_raise_flags(status, FLAG_INVALID);
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
        softfloat_raise_flags(status, FLAG_DENORMAL | FLAG_INEXACT);
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

    softfloat_raise_flags(status, FLAG_INEXACT);

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
