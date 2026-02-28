#![allow(dead_code)]
//! FYL2X and FYL2XP1 implementation: compute y*log2(x) and y*log2(x+1).
//! Ported from Bochs cpu/fpu/fyl2x.cc using Float128 polynomial evaluation.

use super::super::softfloat3e::extf80_addsub::extf80_add;
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

// --- ln_arr[9] from Bochs fyl2x.cc ---

const LN_ARR: [Float128; 9] = [
    Float128::new(0x3fff000000000000, 0x0000000000000000), /*  1 */
    Float128::new(0x3ffd555555555555, 0x5555555555555555), /*  3 */
    Float128::new(0x3ffc999999999999, 0x999999999999999a), /*  5 */
    Float128::new(0x3ffc249249249249, 0x2492492492492492), /*  7 */
    Float128::new(0x3ffbc71c71c71c71, 0xc71c71c71c71c71c), /*  9 */
    Float128::new(0x3ffb745d1745d174, 0x5d1745d1745d1746), /* 11 */
    Float128::new(0x3ffb3b13b13b13b1, 0x3b13b13b13b13b14), /* 13 */
    Float128::new(0x3ffb111111111111, 0x1111111111111111), /* 15 */
    Float128::new(0x3ffae1e1e1e1e1e1, 0xe1e1e1e1e1e1e1e2), /* 17 */
];

/// Polynomial approximation for (1/2)*ln((1+u)/(1-u)) (from Bochs fyl2x.cc poly_ln).
fn poly_ln(x: Float128, status: &mut SoftFloatStatus) -> Float128 {
    odd_poly(x, &LN_ARR, status)
}

/// Compute log2(x) for sqrt(2)/2 < x < sqrt(2) (from Bochs fyl2x.cc poly_l2).
fn poly_l2(x: Float128, status: &mut SoftFloatStatus) -> Float128 {
    let x_p1 = f128_add(x, FLOAT128_ONE, status);
    let x_m1 = f128_sub(x, FLOAT128_ONE, status);
    let u = f128_div(x_m1, x_p1, status);
    let ln_val = poly_ln(u, status);
    f128_mul(ln_val, FLOAT128_LN2INV2, status)
}

/// Compute log2(x+1) using the identity ln(1+x) = 2*atanh(x/(x+2))
/// (from Bochs fyl2x.cc poly_l2p1).
fn poly_l2p1(x: Float128, status: &mut SoftFloatStatus) -> Float128 {
    let x_plus2 = f128_add(x, FLOAT128_TWO, status);
    let u = f128_div(x, x_plus2, status);
    let ln_val = poly_ln(u, status);
    f128_mul(ln_val, FLOAT128_LN2INV2, status)
}

/// Compute y * log2(x) where x=a (ST(0)) and y=b (ST(1)).
/// Ported from Bochs fyl2x.cc using Float128 polynomial evaluation.
pub(crate) fn fyl2x_impl(a: floatx80, b: floatx80, status: &mut SoftFloatStatus) -> floatx80 {
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

    let z_sign = !b_sign; // bSign ^ 1

    // a is NaN or Infinity
    if a_exp == 0x7FFF {
        if (a_sig << 1) != 0 || ((b_exp == 0x7FFF) && (b_sig << 1) != 0) {
            return softfloat_propagate_nan_extf80(a.sign_exp, a_sig, b.sign_exp, b_sig, status);
        }
        if a_sign {
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOATX80_DEFAULT_NAN;
        } else {
            if b_exp == 0 {
                if b_sig == 0 {
                    softfloat_raiseFlags(status, FLAG_INVALID);
                    return FLOATX80_DEFAULT_NAN;
                }
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }
            return pack_floatx80(b_sign, 0x7FFF, 0x8000000000000000);
        }
    }

    // b is NaN or Infinity
    if b_exp == 0x7FFF {
        if (b_sig << 1) != 0 {
            return softfloat_propagate_nan_extf80(a.sign_exp, a_sig, b.sign_exp, b_sig, status);
        }
        if a_sign && ((a_exp as u64) | a_sig) != 0 {
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOATX80_DEFAULT_NAN;
        }
        if a_sig != 0 && a_exp == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        if a_exp < 0x3FFF {
            return pack_floatx80(z_sign, 0x7FFF, 0x8000000000000000);
        }
        if a_exp == 0x3FFF && (a_sig << 1) == 0 {
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOATX80_DEFAULT_NAN;
        }
        return pack_floatx80(b_sign, 0x7FFF, 0x8000000000000000);
    }

    // a is zero
    if a_exp == 0 {
        if a_sig == 0 {
            if (b_exp as u64 | b_sig) == 0 {
                softfloat_raiseFlags(status, FLAG_INVALID);
                return FLOATX80_DEFAULT_NAN;
            }
            softfloat_raiseFlags(status, FLAG_DIVBYZERO);
            return pack_floatx80(z_sign, 0x7FFF, 0x8000000000000000);
        }
        if a_sign {
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOATX80_DEFAULT_NAN;
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(a_sig);
        a_exp = norm.exp + 1;
        // a_sig shadowed below
    }

    if a_sign {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOATX80_DEFAULT_NAN;
    }

    // b is zero
    if b_exp == 0 {
        if b_sig == 0 {
            if a_exp < 0x3FFF {
                return pack_floatx80(z_sign, 0, 0);
            }
            return pack_floatx80(b_sign, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(b_sig);
        b_exp = norm.exp + 1;
        // b_sig shadowed below
    }

    // x = 1.0 exactly: log2(1) = 0, y*0 = 0
    if a_exp == 0x3FFF && (a_sig << 1) == 0 {
        return pack_floatx80(b_sign, 0, 0);
    }

    softfloat_raiseFlags(status, FLAG_INEXACT);

    // Re-read a_sig in case it was normalized above
    let a_sig = if extf80_exp(a) == 0 && extf80_fraction(a) != 0 {
        let norm = norm_subnormal_extf80_sig(extf80_fraction(a));
        norm.sig
    } else {
        a_sig
    };

    let mut exp_diff = a_exp - 0x3FFF;
    let mut a_exp_adj: i32 = 0;
    if a_sig >= SQRT2_HALF_SIG {
        exp_diff += 1;
        a_exp_adj = -1;
    }

    // Using Float128 for approximation
    let b128 = norm_round_pack_to_f128(b_sign, b_exp - 0x10, b_sig, 0, status);

    let (z_sig0, z_sig1) = short_shift_right128(a_sig << 1, 0, 16);
    let x = pack_float128(false, a_exp_adj + 0x3FFF, z_sig0, z_sig1);
    let mut x128 = poly_l2(x, status);
    x128 = f128_add(x128, i32_to_f128(exp_diff), status);
    x128 = f128_mul(x128, b128, status);
    f128_to_extf80(x128, status)
}

/// Compute y * log2(x + 1) where x=a (ST(0)) and y=b (ST(1)).
/// Ported from Bochs fyl2x.cc fyl2xp1() using Float128 polynomial evaluation.
pub(crate) fn fyl2xp1_impl(a: floatx80, b: floatx80, status: &mut SoftFloatStatus) -> floatx80 {
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

    // a is NaN or Infinity
    if a_exp == 0x7FFF {
        if (a_sig << 1) != 0 || ((b_exp == 0x7FFF) && (b_sig << 1) != 0) {
            return softfloat_propagate_nan_extf80(a.sign_exp, a_sig, b.sign_exp, b_sig, status);
        }
        if a_sign {
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOATX80_DEFAULT_NAN;
        } else {
            if b_exp == 0 {
                if b_sig == 0 {
                    softfloat_raiseFlags(status, FLAG_INVALID);
                    return FLOATX80_DEFAULT_NAN;
                }
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }
            return pack_floatx80(b_sign, 0x7FFF, 0x8000000000000000);
        }
    }

    // b is NaN or Infinity
    if b_exp == 0x7FFF {
        if (b_sig << 1) != 0 {
            return softfloat_propagate_nan_extf80(a.sign_exp, a_sig, b.sign_exp, b_sig, status);
        }
        if a_exp == 0 {
            if a_sig == 0 {
                softfloat_raiseFlags(status, FLAG_INVALID);
                return FLOATX80_DEFAULT_NAN;
            }
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_floatx80(z_sign, 0x7FFF, 0x8000000000000000);
    }

    // a is zero
    if a_exp == 0 {
        if a_sig == 0 {
            if b_sig != 0 && b_exp == 0 {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }
            return pack_floatx80(z_sign, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(a_sig);
        a_exp = norm.exp + 1;
        // a_sig normalized below
    }

    // b is zero
    if b_exp == 0 {
        if b_sig == 0 {
            return pack_floatx80(z_sign, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(b_sig);
        b_exp = norm.exp + 1;
        // b_sig normalized below
    }

    softfloat_raiseFlags(status, FLAG_INEXACT);

    // If a is negative and |a| >= 1, Bochs just returns a
    if a_sign && a_exp >= 0x3FFF {
        return a;
    }

    // Big argument: use fyl2x(a+1, b)
    if a_exp >= 0x3FFC {
        let a_plus_one = extf80_add(a, FLOATX80_ONE, status);
        return fyl2x_impl(a_plus_one, b, status);
    }

    // Re-read a_sig in case it was normalized
    let a_sig = if extf80_exp(a) == 0 && extf80_fraction(a) != 0 {
        let norm = norm_subnormal_extf80_sig(extf80_fraction(a));
        norm.sig
    } else {
        a_sig
    };

    // Handle tiny argument: first-order approximation (a*b)/ln(2)
    if a_exp < (FLOATX80_EXP_BIAS as i32) - 70 {
        let mut z_exp = a_exp + FLOAT_LN2INV_EXP - 0x3FFE;

        let (mut z_sig0, mut z_sig1, _z_sig2) =
            mul128_by_64_to_192(FLOAT_LN2INV_HI, FLOAT_LN2INV_LO, a_sig);
        if (z_sig0 as i64) >= 0 {
            let (s0, s1) = short_shift_left128(z_sig0, z_sig1, 1);
            z_sig0 = s0;
            z_sig1 = s1;
            z_exp -= 1;
        }

        // Re-read b_sig
        let b_sig = if extf80_exp(b) == 0 && extf80_fraction(b) != 0 {
            let norm = norm_subnormal_extf80_sig(extf80_fraction(b));
            norm.sig
        } else {
            b_sig
        };

        z_exp = z_exp + b_exp - 0x3FFE;
        let (z0, z1, _z2) = mul128_by_64_to_192(z_sig0, z_sig1, b_sig);
        z_sig0 = z0;
        z_sig1 = z1;
        if (z_sig0 as i64) >= 0 {
            let (s0, s1) = short_shift_left128(z_sig0, z_sig1, 1);
            z_sig0 = s0;
            z_sig1 = s1;
            z_exp -= 1;
        }

        return round_pack_to_extf80(a_sign ^ b_sign, z_exp, z_sig0, z_sig1, 80, status);
    }

    // Using Float128 for approximation
    let b128 = norm_round_pack_to_f128(b_sign, b_exp - 0x10, b_sig, 0, status);

    let (z_sig0, z_sig1) = short_shift_right128(a_sig << 1, 0, 16);
    let x = pack_float128(a_sign, a_exp, z_sig0, z_sig1);
    let mut x128 = poly_l2p1(x, status);
    x128 = f128_mul(x128, b128, status);
    f128_to_extf80(x128, status)
}
