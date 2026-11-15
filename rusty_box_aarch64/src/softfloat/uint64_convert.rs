//! 64-bit unsigned-integer conversions.
//! Ported from Berkeley SoftFloat 3e s_roundToUI64.c, ui64_to_f32.c,
//! ui64_to_f64.c, f32_to_ui64.c and f64_to_ui64.c (via Bochs softfloat3e).

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Berkeley `softfloat_roundToUI64`. `sig` and `sig_extra` form a 128-bit
/// fixed-point value with the binary point between them.
pub(crate) fn softfloat_round_to_ui64(
    sign: bool,
    mut sig: u64,
    sig_extra: u64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> u64 {
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
    if sig_extra != 0 && exact {
        softfloat_raise_flags(status, FLAG_INEXACT);
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

/// Berkeley `ui64_to_f32`.
pub(crate) fn ui64_to_f32(a: u64, status: &mut SoftFloatStatus) -> Float32 {
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

/// Berkeley `ui64_to_f64`.
pub(crate) fn ui64_to_f64(a: u64, status: &mut SoftFloatStatus) -> Float64 {
    if a == 0 {
        return 0;
    }
    if (a & 0x8000_0000_0000_0000) != 0 {
        round_pack_to_f64(false, 0x43D, short_shift_right_jam64(a, 1), status)
    } else {
        norm_round_pack_to_f64(false, 0x43C, a, status)
    }
}

/// Berkeley `f32_to_ui64`.
pub(crate) fn f32_to_ui64(
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

/// Berkeley `f64_to_ui64`.
pub(crate) fn f64_to_ui64(
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
