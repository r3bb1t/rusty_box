#![allow(dead_code)]
//! FPATAN implementation: compute atan2(y, x).
//! Ported from Bochs cpu/fpu/fpatan.cc using Float128 polynomial evaluation.

use super::super::softfloat3e::softfloat::*;
use super::super::softfloat3e::softfloat_types::floatx80;
use super::super::softfloat3e::specialize::*;
use super::super::softfloat3e::internals::*;
use super::super::softfloat3e::primitives::*;
use super::super::softfloat3e::extf80_addsub::{extf80_add, extf80_sub};
use super::super::softfloat3e::f128::*;
use super::poly::*;

/// PI as floatx80 (for fpatan)
const FLOATX80_PI: floatx80 = floatx80 {
    signif: 0xc90fdaa22168c235,
    sign_exp: 0x4000,
};

// --- atan_arr[11] from Bochs fpatan.cc ---

const ATAN_ARR: [Float128; 11] = [
    Float128::new(0x3fff000000000000, 0x0000000000000000), /*  1 */
    Float128::new(0xbffd555555555555, 0x5555555555555555), /*  3 */
    Float128::new(0x3ffc999999999999, 0x999999999999999a), /*  5 */
    Float128::new(0xbffc249249249249, 0x2492492492492492), /*  7 */
    Float128::new(0x3ffbc71c71c71c71, 0xc71c71c71c71c71c), /*  9 */
    Float128::new(0xbffb745d1745d174, 0x5d1745d1745d1746), /* 11 */
    Float128::new(0x3ffb3b13b13b13b1, 0x3b13b13b13b13b14), /* 13 */
    Float128::new(0xbffb111111111111, 0x1111111111111111), /* 15 */
    Float128::new(0x3ffae1e1e1e1e1e1, 0xe1e1e1e1e1e1e1e2), /* 17 */
    Float128::new(0xbffaaf286bca1af2, 0x86bca1af286bca1b), /* 19 */
    Float128::new(0x3ffa861861861861, 0x8618618618618618), /* 21 */
];

/// Polynomial approximation for atan(x), |x| < 1/4 (from Bochs fpatan.cc poly_atan).
fn poly_atan(x: Float128, status: &mut SoftFloatStatus) -> Float128 {
    odd_poly(x, &ATAN_ARR, status)
}

/// Compute atan2(b, a) = atan(b/a) with proper quadrant handling.
/// a = ST(0) (x), b = ST(1) (y).
/// Ported from Bochs fpatan.cc using Float128 polynomial evaluation.
pub(crate) fn fpatan_impl(a: floatx80, b: floatx80, status: &mut SoftFloatStatus) -> floatx80 {
    // Handle unsupported encodings
    if extf80_is_unsupported(a) || extf80_is_unsupported(b) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOATX80_DEFAULT_NAN;
    }

    let a_sig = extf80_fraction(a);
    let mut a_exp = extf80_exp(a);
    let a_sign = extf80_sign(a);
    let b_sig = extf80_fraction(b);
    let mut b_exp = extf80_exp(b);
    let b_sign = extf80_sign(b);

    let z_sign = a_sign ^ b_sign;

    // b (y) is NaN or Infinity
    if b_exp == 0x7FFF {
        if (b_sig << 1) != 0 {
            return softfloat_propagate_nan_extf80(a.sign_exp, a_sig, b.sign_exp, b_sig, status);
        }
        // b is infinity
        if a_exp == 0x7FFF {
            if (a_sig << 1) != 0 {
                return softfloat_propagate_nan_extf80(a.sign_exp, a_sig, b.sign_exp, b_sig, status);
            }
            // Both infinity
            if a_sign {
                // atan2(y, -inf) = 3pi/4 * sign(y)
                return round_pack_to_extf80(
                    b_sign, FLOATX80_3PI4_EXP as i32, FLOAT_3PI4_HI, FLOAT_3PI4_LO, 80, status,
                );
            } else {
                // atan2(y, +inf) = pi/4 * sign(y)
                return round_pack_to_extf80(
                    b_sign, FLOATX80_PI4_EXP as i32, FLOAT_PI_HI, FLOAT_PI_LO, 80, status,
                );
            }
        }
        if a_sig != 0 && a_exp == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        // atan2(inf, finite) = pi/2 * sign(y)
        return round_pack_to_extf80(
            b_sign, FLOATX80_PI2_EXP as i32, FLOAT_PI_HI, FLOAT_PI_LO, 80, status,
        );
    }

    // a (x) is NaN or Infinity
    if a_exp == 0x7FFF {
        if (a_sig << 1) != 0 {
            return softfloat_propagate_nan_extf80(a.sign_exp, a_sig, b.sign_exp, b_sig, status);
        }
        if b_sig != 0 && b_exp == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        // a is infinity
        if a_sign {
            // atan2(y, -inf) = pi * sign(y)
            return round_pack_to_extf80(
                b_sign, FLOATX80_PI_EXP as i32, FLOAT_PI_HI, FLOAT_PI_LO, 80, status,
            );
        } else {
            // atan2(y, +inf) = 0 * sign(y)
            return pack_floatx80(b_sign, 0, 0);
        }
    }

    // b (y) is zero
    if b_exp == 0 {
        if b_sig == 0 {
            if a_sig != 0 && a_exp == 0 {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }
            // atan2(0, x): if x negative -> pi, if x positive -> 0
            if a_sign {
                return round_pack_to_extf80(
                    b_sign, FLOATX80_PI_EXP as i32, FLOAT_PI_HI, FLOAT_PI_LO, 80, status,
                );
            } else {
                return pack_floatx80(b_sign, 0, 0);
            }
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(b_sig);
        b_exp = norm.exp + 1;
    }

    // a (x) is zero
    if a_exp == 0 {
        if a_sig == 0 {
            // atan2(y, 0) = pi/2 * sign(y)
            return round_pack_to_extf80(
                b_sign, FLOATX80_PI2_EXP as i32, FLOAT_PI_HI, FLOAT_PI_LO, 80, status,
            );
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(a_sig);
        a_exp = norm.exp + 1;
    }

    softfloat_raiseFlags(status, FLAG_INEXACT);

    // Re-read sigs in case they were normalized
    let a_sig = if extf80_exp(a) == 0 && extf80_fraction(a) != 0 {
        norm_subnormal_extf80_sig(extf80_fraction(a)).sig
    } else {
        a_sig
    };
    let b_sig = if extf80_exp(b) == 0 && extf80_fraction(b) != 0 {
        norm_subnormal_extf80_sig(extf80_fraction(b)).sig
    } else {
        b_sig
    };

    // |a| = |b| ==> return PI/4
    if a_sig == b_sig && a_exp == b_exp {
        if a_sign {
            return round_pack_to_extf80(b_sign, FLOATX80_3PI4_EXP as i32, FLOAT_3PI4_HI, FLOAT_3PI4_LO, 80, status);
        } else {
            return round_pack_to_extf80(b_sign, FLOATX80_PI4_EXP as i32, FLOAT_PI_HI, FLOAT_PI_LO, 80, status);
        }
    }

    // Using Float128 for approximation
    let a128 = norm_round_pack_to_f128(false, a_exp - 0x10, a_sig, 0, status);
    let b128 = norm_round_pack_to_f128(false, b_exp - 0x10, b_sig, 0, status);
    let mut x;
    let mut swap = false;
    let mut add_pi6 = false;
    let mut add_pi4 = false;

    if a_exp > b_exp || (a_exp == b_exp && a_sig > b_sig) {
        x = f128_div(b128, a128, status);
    } else {
        x = f128_div(a128, b128, status);
        swap = true;
    }

    let x_exp = exp_f128_ui64(x.v64);

    if x_exp <= (FLOATX80_EXP_BIAS as i32) - 40 {
        // Skip polynomial, tiny argument
    } else if x.v64 >= 0x3ffe800000000000 {
        // 3/4 < x < 1: arctan(x) = arctan((x-1)/(x+1)) + pi/4
        let t1 = f128_sub(x, FLOAT128_ONE, status);
        let t2 = f128_add(x, FLOAT128_ONE, status);
        x = f128_div(t1, t2, status);
        add_pi4 = true;

        x = poly_atan(x, status);
        if add_pi4 { x = f128_add(x, FLOAT128_PI4, status); }
    } else if x_exp >= 0x3FFD {
        // 1/4 < x < 3/4: arctan(x) = arctan((x*sqrt(3)-1)/(x+sqrt(3))) + pi/6
        let t1 = f128_mul(x, FLOAT128_SQRT3, status);
        let t2 = f128_add(x, FLOAT128_SQRT3, status);
        x = f128_sub(t1, FLOAT128_ONE, status);
        x = f128_div(x, t2, status);
        add_pi6 = true;

        x = poly_atan(x, status);
        if add_pi6 { x = f128_add(x, FLOAT128_PI6, status); }
    } else {
        // |x| < 1/4: direct polynomial
        x = poly_atan(x, status);
    }

    if swap {
        x = f128_sub(FLOAT128_PI2, x, status);
    }

    let result = f128_to_extf80(x, status);
    let result = if z_sign { floatx80_chs(result) } else { result };
    let r_sign = extf80_sign(result);
    if !b_sign && r_sign {
        return extf80_add(result, FLOATX80_PI, status);
    }
    if b_sign && !r_sign {
        return extf80_sub(result, FLOATX80_PI, status);
    }
    result
}
