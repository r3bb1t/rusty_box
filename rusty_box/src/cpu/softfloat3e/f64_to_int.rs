#![allow(dead_code, non_snake_case)]
//! Double-precision to signed-integer conversions.
//! Ported from Bochs softfloat3e/f64_to_i32.cc, f64_to_i32_r_minMag.cc,
//! f64_to_i64.cc and f64_to_i64_r_minMag.cc.
//!
//! As in [`super::f32_to_int`], the upstream `#if (i32_fromNaN != ...)` NaN
//! pre-check is vacuous under Bochs's 8086-SSE specialization and is
//! therefore absent: NaN falls through to the overflow path, which yields
//! the same integer indefinite value.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Bochs softfloat3e `f64_to_i32`.
pub(crate) fn f64_to_i32(
    a: float64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i32 {
    let sign = sign_f64(a);
    let exp = exp_f64(a);
    let mut sig = frac_f64(a);

    if exp != 0 {
        sig |= 0x0010_0000_0000_0000;
    } else if softfloat_denormalsAreZeros(status) {
        return 0;
    }

    let shift_dist = 0x427 - exp;
    if 0 < shift_dist {
        sig = shift_right_jam64(sig, shift_dist as u32);
    }
    softfloat_round_to_i32(sign, sig, rounding_mode, exact, status)
}

/// Bochs softfloat3e `f64_to_i32_r_minMag` (round toward zero).
pub(crate) fn f64_to_i32_r_min_mag(
    a: float64,
    exact: bool,
    saturate: bool,
    status: &mut SoftFloatStatus,
) -> i32 {
    let exp = exp_f64(a);
    let mut sig = frac_f64(a);

    if softfloat_denormalsAreZeros(status) && exp == 0 && sig != 0 {
        return 0;
    }

    let shift_dist = 0x433 - exp;
    if 53 <= shift_dist {
        if exact && (exp as u64 | sig) != 0 {
            softfloat_raiseFlags(status, FLAG_INEXACT);
        }
        return 0;
    }
    let sign = sign_f64(a);
    if shift_dist < 22 {
        if sign && exp == 0x41E && sig < 0x0000_0000_0020_0000 {
            if exact && sig != 0 {
                softfloat_raiseFlags(status, FLAG_INEXACT);
            }
            return -0x7FFF_FFFF - 1;
        }
        let nan_response = if saturate { 0 } else { I32_FROM_NAN };
        let neg_overflow = if saturate {
            I32_MIN_NEGATIVE_VALUE
        } else {
            I32_FROM_NEG_OVERFLOW
        };
        let pos_overflow = if saturate {
            I32_MAX_POSITIVE_VALUE
        } else {
            I32_FROM_POS_OVERFLOW
        };
        softfloat_raiseFlags(status, FLAG_INVALID);
        return if exp == 0x7FF && sig != 0 {
            nan_response
        } else if sign {
            neg_overflow
        } else {
            pos_overflow
        };
    }
    sig |= 0x0010_0000_0000_0000;
    let abs_z = (sig >> shift_dist) as i32;
    if exact && ((abs_z as u32 as u64) << shift_dist) != sig {
        softfloat_raiseFlags(status, FLAG_INEXACT);
    }
    if sign {
        -abs_z
    } else {
        abs_z
    }
}

/// Bochs softfloat3e `f64_to_i64`.
pub(crate) fn f64_to_i64(
    a: float64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i64 {
    let sign = sign_f64(a);
    let exp = exp_f64(a);
    let mut sig = frac_f64(a);

    if exp != 0 {
        sig |= 0x0010_0000_0000_0000;
    } else if softfloat_denormalsAreZeros(status) {
        return 0;
    }

    let shift_dist = 0x433 - exp;
    let sig_extra;
    if shift_dist <= 0 {
        if shift_dist < -11 {
            softfloat_raiseFlags(status, FLAG_INVALID);
            return if exp == 0x7FF && frac_f64(a) != 0 {
                I64_FROM_NAN
            } else if sign {
                I64_FROM_NEG_OVERFLOW
            } else {
                I64_FROM_POS_OVERFLOW
            };
        }
        sig_extra = (sig << (-shift_dist), 0u64);
    } else {
        sig_extra = shift_right_jam64_extra(sig, 0, shift_dist as u32);
    }
    softfloat_round_to_i64(
        sign,
        sig_extra.0,
        sig_extra.1,
        rounding_mode,
        exact,
        status,
    )
}

/// Bochs softfloat3e `f64_to_i64_r_minMag` (round toward zero).
pub(crate) fn f64_to_i64_r_min_mag(
    a: float64,
    exact: bool,
    saturate: bool,
    status: &mut SoftFloatStatus,
) -> i64 {
    let sign = sign_f64(a);
    let exp = exp_f64(a);
    let mut sig = frac_f64(a);

    if softfloat_denormalsAreZeros(status) && exp == 0 && sig != 0 {
        return 0;
    }

    let shift_dist = 0x433 - exp;
    let abs_z: u64;
    if shift_dist <= 0 {
        if shift_dist < -10 {
            if a == pack_to_f64(true, 0x43E, 0) {
                return -0x7FFF_FFFF_FFFF_FFFF - 1;
            }
            let nan_response = if saturate { 0 } else { I64_FROM_NAN };
            let neg_overflow = if saturate {
                I64_MIN_NEGATIVE_VALUE
            } else {
                I64_FROM_NEG_OVERFLOW
            };
            let pos_overflow = if saturate {
                I64_MAX_POSITIVE_VALUE
            } else {
                I64_FROM_POS_OVERFLOW
            };
            softfloat_raiseFlags(status, FLAG_INVALID);
            return if exp == 0x7FF && sig != 0 {
                nan_response
            } else if sign {
                neg_overflow
            } else {
                pos_overflow
            };
        }
        sig |= 0x0010_0000_0000_0000;
        abs_z = sig << (-shift_dist);
    } else {
        if 53 <= shift_dist {
            if exact && (exp as u64 | sig) != 0 {
                softfloat_raiseFlags(status, FLAG_INEXACT);
            }
            return 0;
        }
        sig |= 0x0010_0000_0000_0000;
        abs_z = sig >> shift_dist;
        if exact && (abs_z << shift_dist) != sig {
            softfloat_raiseFlags(status, FLAG_INEXACT);
        }
    }
    if sign {
        (abs_z as i64).wrapping_neg()
    } else {
        abs_z as i64
    }
}

#[cfg(test)]
mod tests {
    use super::super::f32_to_int::*;
    use super::*;

    // CVTTSD2SI / CVTTSS2SI semantics: truncate toward zero, integer
    // indefinite on NaN or out-of-range.
    #[test]
    fn float_to_int_truncating_matches_x86() {
        let cases: [(f64, i32); 10] = [
            (0.0, 0),
            (1.9, 1),
            (-1.9, -1),
            (2147483647.4, 2147483647),
            (-2147483648.5, -2147483648),
            (2147483648.0, i32::MIN),   // out of range → indefinite
            (-2147483649.0, i32::MIN),  // out of range → indefinite
            (f64::NAN, i32::MIN),
            (f64::INFINITY, i32::MIN),
            (f64::NEG_INFINITY, i32::MIN),
        ];
        for (v, want) in cases {
            let mut st = SoftFloatStatus::default();
            assert_eq!(
                f64_to_i32_r_min_mag(v.to_bits(), false, false, &mut st),
                want,
                "f64_to_i32_r_minMag({v})"
            );
        }
        let mut st = SoftFloatStatus::default();
        assert_eq!(f32_to_i32_r_min_mag((1.9f32).to_bits(), false, false, &mut st), 1);
        let mut st = SoftFloatStatus::default();
        assert_eq!(f32_to_i64_r_min_mag((-1.9f32).to_bits(), false, false, &mut st), -1);
        let mut st = SoftFloatStatus::default();
        assert_eq!(
            f64_to_i64_r_min_mag(f64::NAN.to_bits(), false, false, &mut st),
            i64::MIN
        );
    }

    // CVTSD2SI rounds with MXCSR.RC; under the default RNE, 2.5 → 2.
    #[test]
    fn float_to_int_rounding_uses_rounding_mode() {
        let mut st = SoftFloatStatus::default();
        assert_eq!(f64_to_i32((2.5f64).to_bits(), ROUND_NEAR_EVEN, false, &mut st), 2);
        let mut st = SoftFloatStatus::default();
        assert_eq!(f64_to_i32((3.5f64).to_bits(), ROUND_NEAR_EVEN, false, &mut st), 4);
        let mut st = SoftFloatStatus::default();
        assert_eq!(f64_to_i32((2.9f64).to_bits(), ROUND_TO_ZERO, false, &mut st), 2);
        let mut st = SoftFloatStatus::default();
        assert_eq!(f64_to_i32((2.1f64).to_bits(), ROUND_MAX, false, &mut st), 3);
        let mut st = SoftFloatStatus::default();
        assert_eq!(f64_to_i32((2.9f64).to_bits(), ROUND_MIN, false, &mut st), 2);
    }
}
