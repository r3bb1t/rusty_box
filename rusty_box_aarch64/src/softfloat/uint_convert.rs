//! 32-bit unsigned-integer conversions.
//! Ported from Berkeley SoftFloat 3e s_roundToUI32.c, ui32_to_f32.c,
//! ui32_to_f64.c, f32_to_ui32.c and f64_to_ui32.c (via Bochs softfloat3e).
//! Under the Arm specialization a NaN and a negative overflow both convert
//! to 0, so the upstream `#if (ui32_fromNaN != ui32_fromPosOverflow)` NaN
//! pre-check is live; it is written as the direct NaN response.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Berkeley `softfloat_roundToUI32`: `sig` is a fixed-point value with the
/// binary point between bits 11 and 12. A negative value that rounds to zero
/// is zero; any other negative value is invalid.
pub(crate) fn softfloat_round_to_ui32(
    sign: bool,
    mut sig: u64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> u32 {
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
    if round_bits != 0 && exact {
        softfloat_raise_flags(status, FLAG_INEXACT);
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

/// Berkeley `ui32_to_f32`.
pub(crate) fn ui32_to_f32(a: u32, status: &mut SoftFloatStatus) -> Float32 {
    if a == 0 {
        return 0;
    }
    if (a & 0x8000_0000) != 0 {
        round_pack_to_f32(false, 0x9D, (a >> 1) | (a & 1), status)
    } else {
        norm_round_pack_to_f32(false, 0x9C, a, status)
    }
}

/// Berkeley `ui32_to_f64`. Exact for every u32.
pub(crate) fn ui32_to_f64(a: u32) -> Float64 {
    if a == 0 {
        return 0;
    }
    let shift_dist = count_leading_zeros32(a) as i16 + 21;
    pack_to_f64(false, 0x432 - shift_dist, (a as u64) << shift_dist)
}

/// Berkeley `f32_to_ui32`.
pub(crate) fn f32_to_ui32(
    a: Float32,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> u32 {
    let sign = sign_f32(a);
    let exp = exp_f32(a);
    let mut sig = frac_f32(a);

    if exp == 0xFF && sig != 0 {
        softfloat_raise_flags(status, FLAG_INVALID);
        return UI32_FROM_NAN;
    }
    if exp != 0 {
        sig |= 0x0080_0000;
    }

    let mut sig64 = (sig as u64) << 32;
    let shift_dist = 0xAA - exp;
    if 0 < shift_dist {
        sig64 = shift_right_jam64(sig64, shift_dist as u32);
    }
    softfloat_round_to_ui32(sign, sig64, rounding_mode, exact, status)
}

/// Berkeley `f64_to_ui32`.
pub(crate) fn f64_to_ui32(
    a: Float64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> u32 {
    let sign = sign_f64(a);
    let exp = exp_f64(a);
    let mut sig = frac_f64(a);

    if exp == 0x7FF && sig != 0 {
        softfloat_raise_flags(status, FLAG_INVALID);
        return UI32_FROM_NAN;
    }
    if exp != 0 {
        sig |= 0x0010_0000_0000_0000;
    }

    let shift_dist = 0x427 - exp;
    if 0 < shift_dist {
        sig = shift_right_jam64(sig, shift_dist as u32);
    }
    softfloat_round_to_ui32(sign, sig, rounding_mode, exact, status)
}
