#![allow(dead_code)]
//! FPREM/FPREM1 partial remainder implementation.
//! Ported from Bochs cpu/fpu/fprem.cc.

use super::super::softfloat3e::internals::*;
use super::super::softfloat3e::primitives::*;
use super::super::softfloat3e::softfloat::*;
use super::super::softfloat3e::softfloat_types::floatx80;
use super::super::softfloat3e::specialize::*;

/// Estimate 128/64 division: returns an approximate quotient q such that
/// (a_hi:a_lo) / b is approximately q.  Used by remainder_kernel.
fn fprem_estimate_div_128_to_64(a_hi: u64, a_lo: u64, b: u64) -> u64 {
    if a_hi >= b {
        return 0xFFFFFFFFFFFFFFFF;
    }
    // Use Rust u128 for the estimation
    let a128 = ((a_hi as u128) << 64) | (a_lo as u128);
    let b128 = b as u128;
    let q = a128 / b128;
    if q > 0xFFFFFFFFFFFFFFFF_u128 {
        0xFFFFFFFFFFFFFFFF
    } else {
        q as u64
    }
}

/// Executes single exponent reduction cycle for FPREM/FPREM1.
/// Ported from Bochs fprem.cc remainder_kernel().
pub(crate) fn remainder_kernel(
    a_sig0: u64,
    b_sig: u64,
    exp_diff: u8,
    z_sig0: &mut u64,
    z_sig1: &mut u64,
) -> u64 {
    let mut a_sig1: u64 = 0;
    let mut a_sig0 = a_sig0;

    // shortShift128Left(a_sig1, a_sig0, exp_diff, &a_sig1, &a_sig0)
    let (new_hi, new_lo) = if exp_diff > 0 && exp_diff < 64 {
        short_shift_left128(a_sig1, a_sig0, exp_diff)
    } else if exp_diff == 0 {
        (a_sig1, a_sig0)
    } else {
        (a_sig0, 0) // exp_diff >= 64
    };
    a_sig1 = new_hi;
    a_sig0 = new_lo;

    let q = fprem_estimate_div_128_to_64(a_sig1, a_sig0, b_sig);

    // term = b_sig * q (128-bit product)
    let (term_hi, term_lo) = mul64_to_128(b_sig, q);

    // z = (a_sig1:a_sig0) - (term_hi:term_lo)
    let (mut zh, mut zl) = sub128(a_sig1, a_sig0, term_hi, term_lo);

    let mut q = q;
    // while (int64_t)zh < 0
    while (zh as i64) < 0 {
        q = q.wrapping_sub(1);
        let (nh, nl) = add128(zh, zl, 0, b_sig);
        zh = nh;
        zl = nl;
    }

    *z_sig0 = zl;
    *z_sig1 = zh;
    q
}

/// Core FPREM implementation, shared between FPREM (truncation) and FPREM1 (round-to-nearest).
/// Returns: -1 on error/NaN, 0 on complete, 1 on overflow (incomplete reduction).
/// Ported from Bochs fprem.cc do_fprem().
pub(crate) fn do_fprem(
    a: floatx80,
    b: floatx80,
    r: &mut floatx80,
    q: &mut u64,
    rounding_mode: u8,
    status: &mut SoftFloatStatus,
) -> i32 {
    *q = 0;

    // handle unsupported extended double-precision floating encodings
    if extf80_is_unsupported(a) || extf80_is_unsupported(b) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        *r = FLOATX80_DEFAULT_NAN;
        return -1;
    }

    let mut a_sig0 = extf80_fraction(a);
    let mut a_exp = extf80_exp(a);
    let mut a_sign = extf80_sign(a);
    let mut b_sig = extf80_fraction(b);
    let mut b_exp = extf80_exp(b);

    if a_exp == 0x7FFF {
        if (a_sig0 << 1) != 0 || ((b_exp == 0x7FFF) && (b_sig << 1) != 0) {
            *r = softfloat_propagate_nan_extf80(a.sign_exp, a.signif, b.sign_exp, b.signif, status);
            return -1;
        }
        softfloat_raiseFlags(status, FLAG_INVALID);
        *r = FLOATX80_DEFAULT_NAN;
        return -1;
    }

    if b_exp == 0x7FFF {
        if (b_sig << 1) != 0 {
            *r = softfloat_propagate_nan_extf80(a.sign_exp, a.signif, b.sign_exp, b.signif, status);
            return -1;
        }
        if a_exp == 0 && a_sig0 != 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
            let norm = norm_subnormal_extf80_sig(a_sig0);
            a_exp = norm.exp + 1;
            a_sig0 = norm.sig;
            *r = if (a.signif & 0x8000000000000000) != 0 {
                pack_floatx80(a_sign, a_exp, a_sig0)
            } else {
                a
            };
            return 0;
        }
        *r = a;
        return 0;
    }

    if b_exp == 0 {
        if b_sig == 0 {
            softfloat_raiseFlags(status, FLAG_INVALID);
            *r = FLOATX80_DEFAULT_NAN;
            return -1;
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(b_sig);
        b_exp = norm.exp + 1;
        b_sig = norm.sig;
    }

    if a_exp == 0 {
        if a_sig0 == 0 {
            *r = a;
            return 0;
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(a_sig0);
        a_exp = norm.exp + 1;
        a_sig0 = norm.sig;
    }

    let exp_diff = a_exp - b_exp;
    let mut overflow = 0i32;
    let mut a_sig1: u64 = 0;
    let z_exp;

    if exp_diff >= 64 {
        let n = ((exp_diff & 0x1f) | 0x20) as u8;
        remainder_kernel(a_sig0, b_sig, n, &mut a_sig0, &mut a_sig1);
        z_exp = a_exp - n as i32;
        overflow = 1;
    } else {
        z_exp = b_exp;

        if exp_diff < 0 {
            if exp_diff < -1 {
                *r = if (a.signif & 0x8000000000000000) != 0 {
                    pack_floatx80(a_sign, a_exp, a_sig0)
                } else {
                    a
                };
                return 0;
            }
            // shortShift128Right(a_sig0, 0, 1, ...)
            a_sig1 = a_sig0 << 63;
            a_sig0 >>= 1;
            // exp_diff is now effectively 0 for the algorithm
        } else if exp_diff > 0 {
            *q = remainder_kernel(
                a_sig0,
                b_sig,
                exp_diff as u8,
                &mut a_sig0,
                &mut a_sig1,
            );
        } else {
            // exp_diff == 0
            if b_sig <= a_sig0 {
                a_sig0 -= b_sig;
                *q = 1;
            }
        }

        if rounding_mode == ROUND_NEAR_EVEN {
            // shortShift128Right(b_sig, 0, 1, &term0, &term1)
            let term0 = b_sig >> 1;
            let term1 = b_sig << 63;

            if !lt128(a_sig0, a_sig1, term0, term1) {
                let is_lt = lt128(term0, term1, a_sig0, a_sig1);
                let is_eq = eq128(a_sig0, a_sig1, term0, term1);

                if (is_eq && (*q & 1) != 0) || is_lt {
                    a_sign = !a_sign;
                    *q = q.wrapping_add(1);
                }
                if is_lt {
                    let (sh, sl) = sub128(b_sig, 0, a_sig0, a_sig1);
                    a_sig0 = sh;
                    a_sig1 = sl;
                }
            }
        }
    }

    *r = norm_round_pack_to_extf80(a_sign, z_exp, a_sig0, a_sig1, 80, status);
    overflow
}
