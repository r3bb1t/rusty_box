//! Unsigned-integer conversions used by the AVX-512 VCVT*U* forms.
//! Ported from Bochs softfloat3e/ui32_to_f32.cc, f32_to_ui32.cc,
//! f32_to_ui32_r_minMag.cc and s_roundToUI32.cc.
//!
//! As with the signed conversions, the upstream `#if (ui32_fromNaN != ...)`
//! guard is vacuous under Bochs's 8086-SSE specialization — all three
//! responses are 0xFFFFFFFF — so the NaN pre-check it wraps is absent and
//! NaN falls through to the overflow path.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Largest and smallest representable values, used only by the saturating
/// forms. Bochs specialize.h `ui32_minValue` / `ui32_maxValue`.
pub(in crate::cpu) const UI32_MIN_VALUE: u32 = 0;
pub(in crate::cpu) const UI32_MAX_VALUE: u32 = 0xFFFF_FFFF;

/// Bochs softfloat3e/s_roundToUI32.cc `softfloat_roundToUI32`.
pub(in crate::cpu) fn softfloat_round_to_ui32(
    sign: bool,
    mut sig: u64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> u32 {
    let orig_sig = sig >> 12;
    let mut round_increment: u32 = 0x800;
    if rounding_mode != ROUND_NEAR_MAXMAG && rounding_mode != ROUND_NEAR_EVEN {
        round_increment = 0;
        if sign {
            if sig == 0 {
                return 0;
            }
            if rounding_mode == ROUND_MIN {
                return round_to_ui32_invalid(sign, status);
            }
        } else if rounding_mode == ROUND_MAX {
            round_increment = 0xFFF;
        }
    }
    let round_bits = (sig & 0xFFF) as u32;
    sig = sig.wrapping_add(round_increment as u64);
    if (sig & 0xFFFF_F000_0000_0000) != 0 {
        return round_to_ui32_invalid(sign, status);
    }
    let mut z = (sig >> 12) as u32;
    if round_bits == 0x800 && rounding_mode == ROUND_NEAR_EVEN {
        z &= !1u32;
    }
    if sign && z != 0 {
        return round_to_ui32_invalid(sign, status);
    }
    if round_bits != 0 {
        if exact {
            softfloat_raise_flags(status, FLAG_INEXACT);
        }
        if z as u64 > orig_sig {
            softfloat_set_rounding_up(status);
        }
    }
    z
}

#[inline]
fn round_to_ui32_invalid(sign: bool, status: &mut SoftFloatStatus) -> u32 {
    softfloat_raise_flags(status, FLAG_INVALID);
    if sign {
        UI32_FROM_NEG_OVERFLOW
    } else {
        UI32_FROM_POS_OVERFLOW
    }
}

/// Bochs softfloat3e/ui32_to_f32.cc `ui32_to_f32`.
pub(in crate::cpu) fn ui32_to_f32(a: u32, status: &mut SoftFloatStatus) -> Float32 {
    if a == 0 {
        return 0;
    }
    if (a & 0x8000_0000) != 0 {
        round_pack_to_f32(false, 0x9D, (a >> 1) | (a & 1), status)
    } else {
        norm_round_pack_to_f32(false, 0x9C, a, status)
    }
}

/// Bochs softfloat3e/ui32_to_f64.cc `ui32_to_f64`. Exact for every u32.
pub(in crate::cpu) fn ui32_to_f64(a: u32) -> Float64 {
    if a == 0 {
        return 0;
    }
    let shift_dist = count_leading_zeros32(a) as i16 + 21;
    pack_to_f64(false, 0x432 - shift_dist, (a as u64) << shift_dist)
}

/// Bochs softfloat3e/f32_to_ui32.cc `f32_to_ui32`.
pub(in crate::cpu) fn f32_to_ui32(
    a: Float32,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> u32 {
    let sign = sign_f32(a);
    let exp = exp_f32(a);
    let mut sig = frac_f32(a);

    if exp != 0 {
        sig |= 0x0080_0000;
    } else if softfloat_denormals_are_zeros(status) {
        return 0;
    }

    let mut sig64 = (sig as u64) << 32;
    let shift_dist = 0xAA - exp;
    if 0 < shift_dist {
        sig64 = shift_right_jam64(sig64, shift_dist as u32);
    }
    softfloat_round_to_ui32(sign, sig64, rounding_mode, exact, status)
}

/// Bochs softfloat3e/f32_to_ui32_r_minMag.cc `f32_to_ui32_r_minMag`.
pub(in crate::cpu) fn f32_to_ui32_r_min_mag(
    a: Float32,
    exact: bool,
    saturate: bool,
    status: &mut SoftFloatStatus,
) -> u32 {
    let exp = exp_f32(a);
    let mut sig = frac_f32(a);

    if softfloat_denormals_are_zeros(status) && exp == 0 && sig != 0 {
        return 0;
    }

    let shift_dist = 0x9E - exp;
    if 32 <= shift_dist {
        if exact && (exp as u32 | sig) != 0 {
            softfloat_raise_flags(status, FLAG_INEXACT);
        }
        return 0;
    }
    let sign = sign_f32(a);
    if sign || shift_dist < 0 {
        let nan_response = if saturate { 0 } else { UI32_FROM_NAN };
        let neg_overflow = if saturate {
            UI32_MIN_VALUE
        } else {
            UI32_FROM_NEG_OVERFLOW
        };
        let pos_overflow = if saturate {
            UI32_MAX_VALUE
        } else {
            UI32_FROM_POS_OVERFLOW
        };
        softfloat_raise_flags(status, FLAG_INVALID);
        return if exp == 0xFF && sig != 0 {
            nan_response
        } else if sign {
            neg_overflow
        } else {
            pos_overflow
        };
    }
    sig = (sig | 0x0080_0000) << 8;
    let z = sig >> shift_dist;
    if exact && (z << shift_dist) != sig {
        softfloat_raise_flags(status, FLAG_INEXACT);
    }
    z
}

/// Bochs softfloat3e/f64_to_ui32.cc `f64_to_ui32`.
pub(in crate::cpu) fn f64_to_ui32(
    a: Float64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> u32 {
    let sign = sign_f64(a);
    let exp = exp_f64(a);
    let mut sig = frac_f64(a);

    if exp != 0 {
        sig |= 0x0010_0000_0000_0000;
    } else if softfloat_denormals_are_zeros(status) {
        return 0;
    }

    let shift_dist = 0x427 - exp;
    if 0 < shift_dist {
        sig = shift_right_jam64(sig, shift_dist as u32);
    }
    softfloat_round_to_ui32(sign, sig, rounding_mode, exact, status)
}

/// Bochs softfloat3e/f64_to_ui32_r_minMag.cc `f64_to_ui32_r_minMag`.
pub(in crate::cpu) fn f64_to_ui32_r_min_mag(
    a: Float64,
    exact: bool,
    saturate: bool,
    status: &mut SoftFloatStatus,
) -> u32 {
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
    if sign || shift_dist < 21 {
        let nan_response = if saturate { 0 } else { UI32_FROM_NAN };
        let neg_overflow = if saturate {
            UI32_MIN_VALUE
        } else {
            UI32_FROM_NEG_OVERFLOW
        };
        let pos_overflow = if saturate {
            UI32_MAX_VALUE
        } else {
            UI32_FROM_POS_OVERFLOW
        };
        softfloat_raise_flags(status, FLAG_INVALID);
        return if exp == 0x7FF && sig != 0 {
            nan_response
        } else if sign {
            neg_overflow
        } else {
            pos_overflow
        };
    }
    sig |= 0x0010_0000_0000_0000;
    let z = (sig >> shift_dist) as u32;
    if exact && ((z as u64) << shift_dist) != sig {
        softfloat_raise_flags(status, FLAG_INEXACT);
    }
    z
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_conversions_match_x86() {
        for &v in &[0u32, 1, 2, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, 16_777_217] {
            let mut st = SoftFloatStatus::default();
            assert_eq!(
                f32::from_bits(ui32_to_f32(v, &mut st)).to_bits(),
                (v as f32).to_bits(),
                "ui32_to_f32({v})"
            );
            assert_eq!(
                f64::from_bits(ui32_to_f64(v)).to_bits(),
                (v as f64).to_bits(),
                "ui32_to_f64({v})"
            );
        }

        // Truncating float -> u32: anything negative, NaN or out of range
        // yields the unsigned integer indefinite value 0xFFFFFFFF.
        for (v, want) in [
            (0.0f64, 0u32),
            (1.9, 1),
            (4294967295.0, 0xFFFF_FFFF),
            (-0.5, 0), // truncates to zero before the sign check
            (-1.0, 0xFFFF_FFFF),
            (4294967296.0, 0xFFFF_FFFF),
            (f64::NAN, 0xFFFF_FFFF),
        ] {
            let mut st = SoftFloatStatus::default();
            assert_eq!(
                f64_to_ui32_r_min_mag(v.to_bits(), false, false, &mut st),
                want,
                "f64_to_ui32_r_minMag({v})"
            );
        }

        let mut st = SoftFloatStatus::default();
        assert_eq!(f32_to_ui32_r_min_mag((1.9f32).to_bits(), false, false, &mut st), 1);
        let mut st = SoftFloatStatus::default();
        assert_eq!(f32_to_ui32((2.5f32).to_bits(), ROUND_NEAR_EVEN, false, &mut st), 2);
        let mut st = SoftFloatStatus::default();
        assert_eq!(f32_to_ui32((2.1f32).to_bits(), ROUND_MAX, false, &mut st), 3);
    }
}
