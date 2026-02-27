#![allow(dead_code)]
//! FSIN, FCOS, FSINCOS, FPTAN implementation.
//! Ported from Bochs cpu/fpu/fsincos.cc using Float128 polynomial evaluation.

use super::super::softfloat3e::softfloat::*;
use super::super::softfloat3e::softfloat_types::floatx80;
use super::super::softfloat3e::specialize::*;
use super::super::softfloat3e::internals::*;
use super::super::softfloat3e::primitives::*;
use super::super::softfloat3e::f128::*;
use super::poly::*;

/// 1.0 in floatx80 format
const FLOATX80_ONE: floatx80 = floatx80 {
    signif: 0x8000000000000000,
    sign_exp: 0x3FFF,
};

// --- sin_arr[11] from Bochs fsincos.cc ---

const SIN_ARR: [Float128; 11] = [
    Float128::new(0x3fff000000000000, 0x0000000000000000), /*  1 */
    Float128::new(0xbffc555555555555, 0x5555555555555555), /*  3 */
    Float128::new(0x3ff8111111111111, 0x1111111111111111), /*  5 */
    Float128::new(0xbff2a01a01a01a01, 0xa01a01a01a01a01a), /*  7 */
    Float128::new(0x3fec71de3a556c73, 0x38faac1c88e50017), /*  9 */
    Float128::new(0xbfe5ae64567f544e, 0x38fe747e4b837dc7), /* 11 */
    Float128::new(0x3fde6124613a86d0, 0x97ca38331d23af68), /* 13 */
    Float128::new(0xbfd6ae7f3e733b81, 0xf11d8656b0ee8cb0), /* 15 */
    Float128::new(0x3fce952c77030ad4, 0xa6b2605197771b00), /* 17 */
    Float128::new(0xbfc62f49b4681415, 0x724ca1ec3b7b9675), /* 19 */
    Float128::new(0x3fbd71b8ef6dcf57, 0x18bef146fcee6e45), /* 21 */
];

// --- cos_arr[11] from Bochs fsincos.cc ---

const COS_ARR: [Float128; 11] = [
    Float128::new(0x3fff000000000000, 0x0000000000000000), /*  0 */
    Float128::new(0xbffe000000000000, 0x0000000000000000), /*  2 */
    Float128::new(0x3ffa555555555555, 0x5555555555555555), /*  4 */
    Float128::new(0xbff56c16c16c16c1, 0x6c16c16c16c16c17), /*  6 */
    Float128::new(0x3fefa01a01a01a01, 0xa01a01a01a01a01a), /*  8 */
    Float128::new(0xbfe927e4fb7789f5, 0xc72ef016d3ea6679), /* 10 */
    Float128::new(0x3fe21eed8eff8d89, 0x7b544da987acfe85), /* 12 */
    Float128::new(0xbfda93974a8c07c9, 0xd20badf145dfa3e5), /* 14 */
    Float128::new(0x3fd2ae7f3e733b81, 0xf11d8656b0ee8cb0), /* 16 */
    Float128::new(0xbfca6827863b97d9, 0x77bb004886a2c2ab), /* 18 */
    Float128::new(0x3fc1e542ba402022, 0x507a9cad2bf8f0bb), /* 20 */
];

/// Polynomial approximation for sin(x), 0 <= x <= pi/4 (from Bochs fsincos.cc poly_sin).
fn poly_sin(x: Float128, status: &mut SoftFloatStatus) -> Float128 {
    odd_poly(x, &SIN_ARR, status)
}

/// Polynomial approximation for cos(x), 0 <= x <= pi/4 (from Bochs fsincos.cc poly_cos).
fn poly_cos(x: Float128, status: &mut SoftFloatStatus) -> Float128 {
    even_poly(x, &COS_ARR, status)
}

// ---------------------------------------------------------------------------
// Trigonometric argument reduction (from Bochs fsincos.cc)
// ---------------------------------------------------------------------------

/// Reduce trigonometric function argument using 128-bit precision PI approximation.
/// Returns the quotient q (number of PI/2 multiples subtracted).
/// Ported from Bochs fsincos.cc argument_reduction_kernel().
fn argument_reduction_kernel(a_sig0: u64, exp: i32, z_sig0: &mut u64, z_sig1: &mut u64) -> u64 {
    let mut a_sig1: u64 = 0;
    let a_sig0_local = a_sig0;

    // shortShift128Left(aSig1=0, aSig0, Exp, &aSig1, &aSig0)
    let (hi, lo) = short_shift_left128(0, a_sig0_local, exp as u8);
    a_sig1 = hi;
    let a_sig0_shifted = lo;

    let q = estimate_div_128_to_64(a_sig1, a_sig0_shifted, FLOAT_PI_HI);
    let (term0, term1, mut term2) = mul128_by_64_to_192(FLOAT_PI_HI, FLOAT_PI_LO, q);
    let (mut r1, mut r0) = sub128(a_sig1, a_sig0_shifted, term0, term1);

    while (r1 as i64) < 0 {
        let _q_adj = q.wrapping_sub(1);
        let (n1, n0, nt2) = add192(r1, r0, term2, 0, FLOAT_PI_HI, FLOAT_PI_LO);
        r1 = n1;
        r0 = n0;
        term2 = nt2;
    }

    *z_sig0 = r0;
    *z_sig1 = term2;
    q
}

/// Reduce trigonometric argument to [0, PI/2] range.
/// Returns quadrant (q & 3). May negate zSign.
/// Ported from Bochs fsincos.cc reduce_trig_arg().
fn reduce_trig_arg(exp_diff: i32, z_sign: &mut bool, a_sig0: &mut u64, a_sig1: &mut u64) -> i32 {
    let mut q: u64 = 0;
    let mut exp_diff = exp_diff;

    if exp_diff < 0 {
        let (hi, lo) = short_shift_right128(*a_sig0, 0, 1);
        *a_sig0 = hi;
        *a_sig1 = lo;
        exp_diff = 0;
    }

    if exp_diff > 0 {
        q = argument_reduction_kernel(*a_sig0, exp_diff, a_sig0, a_sig1);
    } else {
        if FLOAT_PI_HI <= *a_sig0 {
            *a_sig0 -= FLOAT_PI_HI;
            q = 1;
        }
    }

    let (term0, term1) = short_shift_right128(FLOAT_PI_HI, FLOAT_PI_LO, 1);
    if !lt128(*a_sig0, *a_sig1, term0, term1) {
        let is_lt = lt128(term0, term1, *a_sig0, *a_sig1);
        let is_eq = eq128(*a_sig0, *a_sig1, term0, term1);

        if (is_eq && (q & 1) != 0) || is_lt {
            *z_sign = !*z_sign;
            q += 1;
        }
        if is_lt {
            let (s0, s1) = sub128(FLOAT_PI_HI, FLOAT_PI_LO, *a_sig0, *a_sig1);
            *a_sig0 = s0;
            *a_sig1 = s1;
        }
    }

    (q & 3) as i32
}

/// Compute sin or cos approximation from reduced argument.
/// Ported from Bochs fsincos.cc sincos_approximation().
fn sincos_approximation(neg: bool, r: Float128, quotient: u64, status: &mut SoftFloatStatus) -> floatx80 {
    let mut neg = neg;
    let result;

    if (quotient & 1) != 0 {
        result = poly_cos(r, status);
        neg = false;
    } else {
        result = poly_sin(r, status);
    }

    let ext = f128_to_extf80(result, status);
    if (quotient & 2) != 0 {
        neg = !neg;
    }
    if neg { floatx80_chs(ext) } else { ext }
}

// ---------------------------------------------------------------------------
// Public result types
// ---------------------------------------------------------------------------

/// Result type for single sin or cos computation.
pub(crate) enum SinCosResult {
    /// Argument out of range (|x| >= 2^63)
    OutOfRange,
    /// Computed value
    Value(floatx80),
}

/// Result type for simultaneous sin+cos computation.
pub(crate) enum SinCosBothResult {
    /// Argument out of range
    OutOfRange,
    /// sin and cos values
    Values(floatx80, floatx80),
}

/// Result type for tangent computation.
pub(crate) enum FtanResult {
    /// Argument out of range
    OutOfRange,
    /// Result is NaN (both sin and cos slots get the same NaN)
    Nan(floatx80),
    /// Computed tangent value
    Value(floatx80),
}

// ---------------------------------------------------------------------------
// Implementation functions
// ---------------------------------------------------------------------------

/// Compute sin or cos of a floatx80 value using Float128 polynomial evaluation.
/// Ported from Bochs fsincos.cc fsincos().
pub(crate) fn fsincos_single(a: floatx80, want_sin: bool, status: &mut SoftFloatStatus) -> SinCosResult {
    // Handle unsupported encodings
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return SinCosResult::Value(FLOATX80_DEFAULT_NAN);
    }

    let mut a_sig0 = extf80_fraction(a);
    let a_exp = extf80_exp(a);
    let a_sign = extf80_sign(a);

    // Infinity or NaN
    if a_exp == 0x7FFF {
        if (a_sig0 << 1) != 0 {
            let nan = softfloat_propagate_nan_extf80(a.sign_exp, a_sig0, 0, 0, status);
            return SinCosResult::Value(nan);
        }
        softfloat_raiseFlags(status, FLAG_INVALID);
        return SinCosResult::Value(FLOATX80_DEFAULT_NAN);
    }

    // Zero
    if a_exp == 0 {
        if a_sig0 == 0 {
            if want_sin {
                return SinCosResult::Value(a); // sin(0) = 0
            } else {
                return SinCosResult::Value(FLOATX80_ONE); // cos(0) = 1
            }
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);

        // Pseudo-denormal (no integer bit set)
        if (a_sig0 & 0x8000000000000000) == 0 {
            softfloat_raiseFlags(status, FLAG_INEXACT);
            if want_sin {
                softfloat_raiseFlags(status, FLAG_UNDERFLOW);
                return SinCosResult::Value(a);
            } else {
                return SinCosResult::Value(FLOATX80_ONE);
            }
        }

        let norm = norm_subnormal_extf80_sig(a_sig0);
        // a_exp and a_sig0 updated for reduction below
        let a_exp = norm.exp + 1;
        a_sig0 = norm.sig;

        // For tiny denormals, skip reduction
        let z_exp = FLOATX80_EXP_BIAS;
        let exp_diff = a_exp - z_exp;
        if exp_diff <= -68 {
            if want_sin {
                return SinCosResult::Value(pack_floatx80(a_sign, a_exp, a_sig0));
            } else {
                return SinCosResult::Value(FLOATX80_ONE);
            }
        }
    }

    let mut z_sign = a_sign;
    let z_exp = FLOATX80_EXP_BIAS;
    let exp_diff = a_exp - z_exp;

    // Argument out of range: |x| >= 2^63
    if exp_diff >= 63 {
        return SinCosResult::OutOfRange;
    }

    softfloat_raiseFlags(status, FLAG_INEXACT);

    let mut a_sig1: u64 = 0;
    let mut q: i32 = 0;
    let z_exp_final;

    if exp_diff < -1 {
        // Doesn't require reduction
        if exp_diff <= -68 {
            if want_sin {
                return SinCosResult::Value(pack_floatx80(a_sign, a_exp, a_sig0));
            } else {
                return SinCosResult::Value(FLOATX80_ONE);
            }
        }
        z_exp_final = a_exp;
    } else {
        q = reduce_trig_arg(exp_diff, &mut z_sign, &mut a_sig0, &mut a_sig1);
        z_exp_final = z_exp;
    }

    // Argument reduction completed — use Float128 for approximation
    let r = norm_round_pack_to_f128(false, z_exp_final - 0x10, a_sig0, a_sig1, status);

    if a_sign { q = -q; }
    let q_unsigned = q as u64;

    if want_sin {
        SinCosResult::Value(sincos_approximation(z_sign, r, q_unsigned, status))
    } else {
        SinCosResult::Value(sincos_approximation(z_sign, r, q_unsigned.wrapping_add(1), status))
    }
}

/// Compute both sin and cos of a floatx80 value using Float128 polynomial evaluation.
/// Ported from Bochs fsincos.cc fsincos().
pub(crate) fn fsincos_both(a: floatx80, status: &mut SoftFloatStatus) -> SinCosBothResult {
    // Handle unsupported encodings
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        let nan = FLOATX80_DEFAULT_NAN;
        return SinCosBothResult::Values(nan, nan);
    }

    let mut a_sig0 = extf80_fraction(a);
    let a_exp = extf80_exp(a);
    let a_sign = extf80_sign(a);

    // Infinity or NaN
    if a_exp == 0x7FFF {
        if (a_sig0 << 1) != 0 {
            let nan = softfloat_propagate_nan_extf80(a.sign_exp, a_sig0, 0, 0, status);
            return SinCosBothResult::Values(nan, nan);
        }
        softfloat_raiseFlags(status, FLAG_INVALID);
        let nan = FLOATX80_DEFAULT_NAN;
        return SinCosBothResult::Values(nan, nan);
    }

    // Zero
    if a_exp == 0 {
        if a_sig0 == 0 {
            return SinCosBothResult::Values(a, FLOATX80_ONE);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);

        // Pseudo-denormal
        if (a_sig0 & 0x8000000000000000) == 0 {
            softfloat_raiseFlags(status, FLAG_INEXACT | FLAG_UNDERFLOW);
            return SinCosBothResult::Values(a, FLOATX80_ONE);
        }

        let norm = norm_subnormal_extf80_sig(a_sig0);
        let a_exp = norm.exp + 1;
        a_sig0 = norm.sig;

        let z_exp = FLOATX80_EXP_BIAS;
        let exp_diff = a_exp - z_exp;
        if exp_diff <= -68 {
            let tiny = pack_floatx80(a_sign, a_exp, a_sig0);
            return SinCosBothResult::Values(tiny, FLOATX80_ONE);
        }
    }

    let mut z_sign = a_sign;
    let z_exp = FLOATX80_EXP_BIAS;
    let exp_diff = a_exp - z_exp;

    // Argument out of range: |x| >= 2^63
    if exp_diff >= 63 {
        return SinCosBothResult::OutOfRange;
    }

    softfloat_raiseFlags(status, FLAG_INEXACT);

    let mut a_sig1: u64 = 0;
    let mut q: i32 = 0;
    let z_exp_final;

    if exp_diff < -1 {
        if exp_diff <= -68 {
            let tiny = pack_floatx80(a_sign, a_exp, a_sig0);
            return SinCosBothResult::Values(tiny, FLOATX80_ONE);
        }
        z_exp_final = a_exp;
    } else {
        q = reduce_trig_arg(exp_diff, &mut z_sign, &mut a_sig0, &mut a_sig1);
        z_exp_final = z_exp;
    }

    // Argument reduction completed
    let r = norm_round_pack_to_f128(false, z_exp_final - 0x10, a_sig0, a_sig1, status);

    if a_sign { q = -q; }
    let q_unsigned = q as u64;

    let sin_result = sincos_approximation(z_sign, r, q_unsigned, status);
    let cos_result = sincos_approximation(z_sign, r, q_unsigned.wrapping_add(1), status);

    SinCosBothResult::Values(sin_result, cos_result)
}

/// Compute tangent of a floatx80 value using Float128 polynomial evaluation.
/// Ported from Bochs fsincos.cc ftan().
pub(crate) fn ftan_impl(a: floatx80, status: &mut SoftFloatStatus) -> FtanResult {
    // Handle unsupported encodings
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FtanResult::Nan(FLOATX80_DEFAULT_NAN);
    }

    let mut a_sig0 = extf80_fraction(a);
    let a_exp = extf80_exp(a);
    let a_sign = extf80_sign(a);

    // Infinity or NaN
    if a_exp == 0x7FFF {
        if (a_sig0 << 1) != 0 {
            let nan = softfloat_propagate_nan_extf80(a.sign_exp, a_sig0, 0, 0, status);
            return FtanResult::Nan(nan);
        }
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FtanResult::Nan(FLOATX80_DEFAULT_NAN);
    }

    // Zero
    if a_exp == 0 {
        if a_sig0 == 0 {
            return FtanResult::Value(a); // tan(0) = 0
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        // Pseudo-denormal
        if (a_sig0 & 0x8000000000000000) == 0 {
            softfloat_raiseFlags(status, FLAG_INEXACT | FLAG_UNDERFLOW);
            return FtanResult::Value(a);
        }

        let norm = norm_subnormal_extf80_sig(a_sig0);
        let _a_exp = norm.exp + 1;
        a_sig0 = norm.sig;
    }

    let mut z_sign = a_sign;
    let z_exp = FLOATX80_EXP_BIAS;
    let exp_diff = a_exp - z_exp;

    // Argument out of range: |x| >= 2^63
    if exp_diff >= 63 {
        return FtanResult::OutOfRange;
    }

    softfloat_raiseFlags(status, FLAG_INEXACT);

    let mut a_sig1: u64 = 0;
    let mut q: i32 = 0;
    let z_exp_final;

    if exp_diff < -1 {
        if exp_diff <= -68 {
            return FtanResult::Value(pack_floatx80(a_sign, a_exp, a_sig0));
        }
        z_exp_final = a_exp;
    } else {
        q = reduce_trig_arg(exp_diff, &mut z_sign, &mut a_sig0, &mut a_sig1);
        z_exp_final = z_exp;
    }

    // Argument reduction completed
    let r = norm_round_pack_to_f128(false, z_exp_final - 0x10, a_sig0, a_sig1, status);

    let sin_r = poly_sin(r, status);
    let cos_r = poly_cos(r, status);

    let result;
    if (q & 1) != 0 {
        result = f128_div(cos_r, sin_r, status);
        z_sign = !z_sign;
    } else {
        result = f128_div(sin_r, cos_r, status);
    }

    let ext = f128_to_extf80(result, status);
    let ext = if z_sign { floatx80_chs(ext) } else { ext };
    FtanResult::Value(ext)
}
