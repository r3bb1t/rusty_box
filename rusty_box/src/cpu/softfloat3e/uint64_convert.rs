//! 64-bit unsigned-integer conversions — the primitives behind the AVX512_DQ
//! VCVT*UQQ forms. Ported from Bochs softfloat3e/s_roundToUI64.cc,
//! ui64_to_f32.cc, ui64_to_f64.cc, f32_to_ui64.cc, f32_to_ui64_r_minMag.cc,
//! f64_to_ui64.cc and f64_to_ui64_r_minMag.cc.
//!
//! As with the 32-bit forms in [`super::uint_convert`], Bochs's 8086-SSE
//! specialization gives NaN, positive overflow and negative overflow the same
//! response, so the `#if (ui64_fromNaN != ui64_fromPosOverflow)` guard around
//! the NaN pre-check is vacuous and the check is correctly absent.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Bochs specialize.h `ui64_minValue` / `ui64_maxValue`, used only by the
/// saturating forms.
pub(in crate::cpu) const UI64_MIN_VALUE: u64 = 0;
pub(in crate::cpu) const UI64_MAX_VALUE: u64 = 0xFFFF_FFFF_FFFF_FFFF;

/// Bochs softfloat3e/s_roundToUI64.cc `softfloat_roundToUI64`. `sig` and
/// `sig_extra` form a 128-bit fixed-point value with the binary point between
/// them.
pub(in crate::cpu) fn softfloat_round_to_ui64(
    sign: bool,
    mut sig: u64,
    sig_extra: u64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> u64 {
    let orig_sig = sig;

    // The C code reaches its `increment` label from two arms of this
    // condition; the flag stands in for the goto.
    let increment = if rounding_mode == ROUND_NEAR_MAXMAG || rounding_mode == ROUND_NEAR_EVEN {
        sig_extra >= 0x8000_0000_0000_0000
    } else if sign {
        if (sig | sig_extra) == 0 {
            return 0;
        }
        if rounding_mode == ROUND_MIN {
            return round_to_ui64_invalid(sign, status);
        }
        false
    } else {
        rounding_mode == ROUND_MAX && sig_extra != 0
    };

    if increment {
        sig = sig.wrapping_add(1);
        if sig == 0 {
            return round_to_ui64_invalid(sign, status);
        }
        if sig_extra == 0x8000_0000_0000_0000 && rounding_mode == ROUND_NEAR_EVEN {
            sig &= !1u64;
        }
    }
    if sign && sig != 0 {
        return round_to_ui64_invalid(sign, status);
    }
    if sig_extra != 0 {
        if exact {
            softfloat_raise_flags(status, FLAG_INEXACT);
        }
        if sig > orig_sig {
            softfloat_set_rounding_up(status);
        }
    }
    sig
}

#[inline]
fn round_to_ui64_invalid(sign: bool, status: &mut SoftFloatStatus) -> u64 {
    softfloat_raise_flags(status, FLAG_INVALID);
    if sign {
        UI64_FROM_NEG_OVERFLOW
    } else {
        UI64_FROM_POS_OVERFLOW
    }
}

/// Bochs softfloat3e/ui64_to_f32.cc `ui64_to_f32`.
pub(in crate::cpu) fn ui64_to_f32(a: u64, status: &mut SoftFloatStatus) -> Float32 {
    let mut shift_dist = count_leading_zeros64(a) as i16 - 40;
    if 0 <= shift_dist {
        return if a != 0 {
            pack_to_f32(false, 0x95 - shift_dist, (a as u32) << shift_dist)
        } else {
            0
        };
    }
    shift_dist += 7;
    let sig = if shift_dist < 0 {
        short_shift_right_jam64(a, (-shift_dist) as u8) as u32
    } else {
        (a as u32) << shift_dist
    };
    round_pack_to_f32(false, 0x9C - shift_dist, sig, status)
}

/// Bochs softfloat3e/ui64_to_f64.cc `ui64_to_f64`.
pub(in crate::cpu) fn ui64_to_f64(a: u64, status: &mut SoftFloatStatus) -> Float64 {
    if a == 0 {
        return 0;
    }
    if (a & 0x8000_0000_0000_0000) != 0 {
        round_pack_to_f64(false, 0x43D, short_shift_right_jam64(a, 1), status)
    } else {
        norm_round_pack_to_f64(false, 0x43C, a, status)
    }
}

/// Bochs softfloat3e/f32_to_ui64.cc `f32_to_ui64`.
pub(in crate::cpu) fn f32_to_ui64(
    a: Float32,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> u64 {
    let sign = sign_f32(a);
    let exp = exp_f32(a);
    let mut sig = frac_f32(a);

    let shift_dist = 0xBE - exp;
    if shift_dist < 0 {
        softfloat_raise_flags(status, FLAG_INVALID);
        return if exp == 0xFF && sig != 0 {
            UI64_FROM_NAN
        } else if sign {
            UI64_FROM_NEG_OVERFLOW
        } else {
            UI64_FROM_POS_OVERFLOW
        };
    }

    if exp != 0 {
        sig |= 0x0080_0000;
    } else if softfloat_denormals_are_zeros(status) {
        return 0;
    }
    let mut sig64 = (sig as u64) << 40;
    let mut extra = 0u64;
    if shift_dist != 0 {
        let (v, e) = shift_right_jam64_extra(sig64, 0, shift_dist as u32);
        sig64 = v;
        extra = e;
    }
    softfloat_round_to_ui64(sign, sig64, extra, rounding_mode, exact, status)
}

/// Bochs softfloat3e/f32_to_ui64_r_minMag.cc `f32_to_ui64_r_minMag`.
pub(in crate::cpu) fn f32_to_ui64_r_min_mag(
    a: Float32,
    exact: bool,
    saturate: bool,
    status: &mut SoftFloatStatus,
) -> u64 {
    let exp = exp_f32(a);
    let mut sig = frac_f32(a);

    if softfloat_denormals_are_zeros(status) && exp == 0 && sig != 0 {
        return 0;
    }

    let mut shift_dist = 0xBE - exp;
    if 64 <= shift_dist {
        if exact && (exp as u32 | sig) != 0 {
            softfloat_raise_flags(status, FLAG_INEXACT);
        }
        return 0;
    }
    let sign = sign_f32(a);
    if sign || shift_dist < 0 {
        softfloat_raise_flags(status, FLAG_INVALID);
        return if exp == 0xFF && sig != 0 {
            if saturate {
                0
            } else {
                UI64_FROM_NAN
            }
        } else if sign {
            if saturate {
                UI64_MIN_VALUE
            } else {
                UI64_FROM_NEG_OVERFLOW
            }
        } else if saturate {
            UI64_MAX_VALUE
        } else {
            UI64_FROM_POS_OVERFLOW
        };
    }
    sig |= 0x0080_0000;
    let sig64 = (sig as u64) << 40;
    let z = sig64 >> shift_dist;
    shift_dist = 40 - shift_dist;
    if exact && shift_dist < 0 && (sig << (shift_dist & 31)) != 0 {
        softfloat_raise_flags(status, FLAG_INEXACT);
    }
    z
}

/// Bochs softfloat3e/f64_to_ui64.cc `f64_to_ui64`.
pub(in crate::cpu) fn f64_to_ui64(
    a: Float64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> u64 {
    let sign = sign_f64(a);
    let exp = exp_f64(a);
    let mut sig = frac_f64(a);

    if exp != 0 {
        sig |= 0x0010_0000_0000_0000;
    } else if softfloat_denormals_are_zeros(status) {
        return 0;
    }
    let shift_dist = 0x433 - exp;
    let (sig_v, sig_extra) = if shift_dist <= 0 {
        if shift_dist < -11 {
            softfloat_raise_flags(status, FLAG_INVALID);
            return if exp == 0x7FF && frac_f64(a) != 0 {
                UI64_FROM_NAN
            } else if sign {
                UI64_FROM_NEG_OVERFLOW
            } else {
                UI64_FROM_POS_OVERFLOW
            };
        }
        (sig << (-shift_dist), 0)
    } else {
        shift_right_jam64_extra(sig, 0, shift_dist as u32)
    };
    softfloat_round_to_ui64(sign, sig_v, sig_extra, rounding_mode, exact, status)
}

/// Bochs softfloat3e/f64_to_ui64_r_minMag.cc `f64_to_ui64_r_minMag`.
pub(in crate::cpu) fn f64_to_ui64_r_min_mag(
    a: Float64,
    exact: bool,
    saturate: bool,
    status: &mut SoftFloatStatus,
) -> u64 {
    let exp = exp_f64(a);
    let mut sig = frac_f64(a);

    if softfloat_denormals_are_zeros(status) && exp == 0 && sig != 0 {
        return 0;
    }

    let shift_dist = 0x433 - exp;
    if 53 <= shift_dist {
        if exact && (exp as u64 | sig) != 0 {
            softfloat_raise_flags(status, FLAG_INEXACT);
        }
        return 0;
    }
    let sign = sign_f64(a);
    let invalid = sign || shift_dist < -11;
    if !invalid {
        if shift_dist <= 0 {
            return (sig | 0x0010_0000_0000_0000) << (-shift_dist);
        }
        sig |= 0x0010_0000_0000_0000;
        let z = sig >> shift_dist;
        if exact && (sig << ((-shift_dist) & 63)) != 0 {
            softfloat_raise_flags(status, FLAG_INEXACT);
        }
        return z;
    }
    softfloat_raise_flags(status, FLAG_INVALID);
    if exp == 0x7FF && sig != 0 {
        if saturate {
            0
        } else {
            UI64_FROM_NAN
        }
    } else if sign {
        if saturate {
            UI64_MIN_VALUE
        } else {
            UI64_FROM_NEG_OVERFLOW
        }
    } else if saturate {
        UI64_MAX_VALUE
    } else {
        UI64_FROM_POS_OVERFLOW
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui64_to_float_matches_the_host() {
        for &v in &[
            0u64,
            1,
            2,
            0x00FF_FFFF,
            0x0100_0001,
            0x7FFF_FFFF_FFFF_FFFF,
            0x8000_0000_0000_0000,
            0xFFFF_FFFF_FFFF_FFFF,
            18_446_744_073_709_549_568,
        ] {
            let mut st = SoftFloatStatus::default();
            assert_eq!(
                ui64_to_f32(v, &mut st),
                (v as f32).to_bits(),
                "ui64_to_f32({v})"
            );
            let mut st = SoftFloatStatus::default();
            assert_eq!(
                ui64_to_f64(v, &mut st),
                (v as f64).to_bits(),
                "ui64_to_f64({v})"
            );
        }
    }

    #[test]
    fn float_to_ui64_saturates_to_the_indefinite_value() {
        // Anything negative, NaN, or above 2^64-1 gives the unsigned integer
        // indefinite value; a negative fraction truncates to zero first.
        for (v, want) in [
            (0.0f64, 0u64),
            (1.9, 1),
            (-0.5, 0),
            (-1.0, u64::MAX),
            (9.007199254740992e15, 9_007_199_254_740_992),
            // Largest f64 strictly below 2^64 — exactly representable, so it
            // converts rather than saturating.
            (1.8446744073709550e19, 18_446_744_073_709_549_568),
            // 2^64 itself is one ulp past the top of the range.
            (1.8446744073709552e19, u64::MAX),
            (f64::NAN, u64::MAX),
            (f64::INFINITY, u64::MAX),
        ] {
            let mut st = SoftFloatStatus::default();
            assert_eq!(
                f64_to_ui64_r_min_mag(v.to_bits(), false, false, &mut st),
                want,
                "f64_to_ui64_r_minMag({v})"
            );
        }
        for (v, want) in [(0.0f32, 0u64), (1.9, 1), (-1.0, u64::MAX), (1e20, u64::MAX)] {
            let mut st = SoftFloatStatus::default();
            assert_eq!(
                f32_to_ui64_r_min_mag(v.to_bits(), false, false, &mut st),
                want,
                "f32_to_ui64_r_minMag({v})"
            );
        }
        // The saturating AVX10.2 responses replace those with the endpoints.
        let mut st = SoftFloatStatus::default();
        assert_eq!(
            f64_to_ui64_r_min_mag((-1.0f64).to_bits(), false, true, &mut st),
            UI64_MIN_VALUE
        );
        let mut st = SoftFloatStatus::default();
        assert_eq!(
            f64_to_ui64_r_min_mag(f64::INFINITY.to_bits(), false, true, &mut st),
            UI64_MAX_VALUE
        );
        let mut st = SoftFloatStatus::default();
        assert_eq!(
            f64_to_ui64_r_min_mag(f64::NAN.to_bits(), false, true, &mut st),
            0
        );
    }

    #[test]
    fn float_to_ui64_honours_the_rounding_mode() {
        let cases = [
            (2.5f64, ROUND_NEAR_EVEN, 2u64),
            (3.5, ROUND_NEAR_EVEN, 4),
            (2.5, ROUND_NEAR_MAXMAG, 3),
            (2.1, ROUND_MAX, 3),
            (2.9, ROUND_MIN, 2),
            (2.9, ROUND_MINMAG, 2),
        ];
        for (v, rc, want) in cases {
            let mut st = SoftFloatStatus::default();
            assert_eq!(
                f64_to_ui64(v.to_bits(), rc, false, &mut st),
                want,
                "f64_to_ui64({v}, rc={rc})"
            );
            let mut st = SoftFloatStatus::default();
            assert_eq!(
                f32_to_ui64((v as f32).to_bits(), rc, false, &mut st),
                want,
                "f32_to_ui64({v}, rc={rc})"
            );
        }
        // Rounding a negative fraction up to zero is legal; rounding it down
        // is not, and raises #I.
        let mut st = SoftFloatStatus::default();
        assert_eq!(f64_to_ui64((-0.5f64).to_bits(), ROUND_MAX, false, &mut st), 0);
        assert_eq!(softfloat_get_exception_flags(&st) & FLAG_INVALID, 0);
        let mut st = SoftFloatStatus::default();
        assert_eq!(
            f64_to_ui64((-0.5f64).to_bits(), ROUND_MIN, false, &mut st),
            u64::MAX
        );
        assert_ne!(softfloat_get_exception_flags(&st) & FLAG_INVALID, 0);
    }
}
