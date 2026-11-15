//! Internal routines: field extraction, normalization, round-and-pack and
//! round-to-integer.
//! Ported from Berkeley SoftFloat 3e internals.h / s_roundPackToF16.c /
//! s_roundPackToF32.c / s_roundPackToF64.c / s_normRoundPackTo*.c /
//! s_roundToI32.c / s_roundToI64.c (via Bochs softfloat3e), with Arm
//! FPRoundBase flush-to-zero added to the round-and-pack paths.

use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

// ============================================================
// Field extraction macros (from internals.h)
// ============================================================

// --- Float16 ---
#[inline]
pub(crate) fn sign_f16(a: u16) -> bool {
    (a >> 15) != 0
}
#[inline]
pub(crate) fn exp_f16(a: u16) -> i16 {
    ((a >> 10) & 0x1F) as i16
}
#[inline]
pub(crate) fn frac_f16(a: u16) -> u16 {
    a & 0x03FF
}
#[inline]
pub(crate) fn pack_to_f16(sign: bool, exp: i16, sig: u16) -> u16 {
    ((sign as u16) << 15)
        .wrapping_add((exp as u16) << 10)
        .wrapping_add(sig)
}

// --- Float32 ---
#[inline]
pub(crate) fn sign_f32(a: u32) -> bool {
    (a >> 31) != 0
}
#[inline]
pub(crate) fn exp_f32(a: u32) -> i16 {
    ((a >> 23) & 0xFF) as i16
}
#[inline]
pub(crate) fn frac_f32(a: u32) -> u32 {
    a & 0x007FFFFF
}
#[inline]
pub(crate) fn pack_to_f32(sign: bool, exp: i16, sig: u32) -> u32 {
    ((sign as u32) << 31)
        .wrapping_add((exp as u32) << 23)
        .wrapping_add(sig)
}
#[inline]
pub(crate) fn is_nan_f32(a: u32) -> bool {
    ((!a & 0x7F800000) == 0) && ((a & 0x007FFFFF) != 0)
}

// --- Float64 ---
#[inline]
pub(crate) fn sign_f64(a: u64) -> bool {
    (a >> 63) != 0
}
#[inline]
pub(crate) fn exp_f64(a: u64) -> i16 {
    ((a >> 52) & 0x7FF) as i16
}
#[inline]
pub(crate) fn frac_f64(a: u64) -> u64 {
    a & 0x000FFFFFFFFFFFFF
}
#[inline]
pub(crate) fn pack_to_f64(sign: bool, exp: i16, sig: u64) -> u64 {
    ((sign as u64) << 63)
        .wrapping_add((exp as u64) << 52)
        .wrapping_add(sig)
}
#[inline]
pub(crate) fn is_nan_f64(a: u64) -> bool {
    ((!a & 0x7FF0000000000000) == 0) && ((a & 0x000FFFFFFFFFFFFF) != 0)
}

// ============================================================
// Normalization of subnormal significands
// ============================================================

pub(crate) struct ExpSig16 {
    pub(crate) exp: i16,
    pub(crate) sig: u16,
}

pub(crate) struct ExpSig32 {
    pub(crate) exp: i16,
    pub(crate) sig: u32,
}

pub(crate) struct ExpSig64 {
    pub(crate) exp: i16,
    pub(crate) sig: u64,
}

/// Berkeley `softfloat_normSubnormalF16Sig`.
pub(crate) fn norm_subnormal_f16_sig(sig: u16) -> ExpSig16 {
    let shift = count_leading_zeros16(sig) as i16 - 5;
    ExpSig16 {
        exp: 1 - shift,
        sig: sig << shift,
    }
}

/// Berkeley `softfloat_normSubnormalF32Sig`.
pub(crate) fn norm_subnormal_f32_sig(sig: u32) -> ExpSig32 {
    let shift = count_leading_zeros32(sig) as i16 - 8;
    ExpSig32 {
        exp: 1 - shift,
        sig: sig << shift,
    }
}

/// Berkeley `softfloat_normSubnormalF64Sig`.
pub(crate) fn norm_subnormal_f64_sig(sig: u64) -> ExpSig64 {
    let shift = count_leading_zeros64(sig) as i16 - 11;
    ExpSig64 {
        exp: 1 - shift,
        sig: sig << shift,
    }
}

// ============================================================
// Round-and-pack
// ============================================================
//
// `exp` < 0 with the significand normalized (leading bit at the top of the
// field) is exactly "below the minimum normal before rounding", which is
// Arm's tininess rule and its FPCR.FZ flush condition alike under FPCR.AH =
// 0 (FPRoundBase); a result flushed then is a zero of the result's sign and
// raises underflow without inexact. Under FPCR.AH = 1 both are decided
// after rounding with an unbounded exponent (Berkeley's
// `softfloat_tininess_afterRounding` test), and a flushed result raises
// underflow and inexact.

/// The flags a result flushed to zero under FPCR.FZ (or FPCR.FZ16) raises:
/// Underflow alone under FPCR.AH = 0, which flushes before rounding;
/// Underflow and Inexact under FPCR.AH = 1, which flushes after (Arm
/// FPRoundBase).
#[inline]
pub(crate) fn flush_flags(status: &SoftFloatStatus) -> i32 {
    match status.handling {
        Handling::Standard => FLAG_UNDERFLOW,
        Handling::Alternate => FLAG_UNDERFLOW | FLAG_INEXACT,
    }
}

/// What `softfloat_roundPackToF16` adds below the half-precision fraction
/// before truncating, for a value of sign `sign` rounded by `rounding_mode`.
#[inline]
fn f16_round_increment(sign: bool, rounding_mode: u8) -> u16 {
    if rounding_mode == ROUND_NEAR_EVEN || rounding_mode == ROUND_NEAR_MAXMAG {
        0x8
    } else if rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX }) {
        0xF
    } else {
        0
    }
}

/// Berkeley's `softfloat_tininess_afterRounding` test for half precision:
/// whether `sig` × 2^`exp`, as [`round_pack_to_f16`] takes them with `exp`
/// below zero, rounds by `rounding_mode` with an unbounded exponent to a
/// magnitude below the smallest normal (Arm FPRoundBase under FPCR.AH = 1).
#[inline]
pub(crate) fn f16_tiny_after_rounding(sign: bool, exp: i16, sig: u16, rounding_mode: u8) -> bool {
    exp < -1 || u32::from(sig) + u32::from(f16_round_increment(sign, rounding_mode)) < 0x8000
}

/// Berkeley `softfloat_roundPackToF16`. Half precision is reached only by
/// conversions, whose Arm FPRoundCV forces FZ16 off, and by the
/// half-precision arithmetic in `half`, which applies FPCR.FZ16 itself, so
/// it never flushes.
pub(crate) fn round_pack_to_f16(
    sign: bool,
    exp: i16,
    sig: u16,
    status: &mut SoftFloatStatus,
) -> Float16 {
    let rounding_mode = softfloat_get_rounding_mode(status);
    let round_near_even = rounding_mode == ROUND_NEAR_EVEN;
    let round_increment = f16_round_increment(sign, rounding_mode);
    let mut round_bits = sig & 0xF;
    let mut exp = exp;
    let mut sig = sig;

    if 0x1D <= (exp as u16) {
        if exp < 0 {
            let is_tiny = tininess_before_rounding(status)
                || f16_tiny_after_rounding(sign, exp, sig, rounding_mode);
            sig = shift_right_jam32(sig as u32, (-(exp as i32)) as u16) as u16;
            exp = 0;
            round_bits = sig & 0xF;
            if is_tiny && round_bits != 0 {
                softfloat_raise_flags(status, FLAG_UNDERFLOW);
            }
        } else if (0x1D < exp) || (0x8000 <= (sig as u32).wrapping_add(round_increment as u32)) {
            softfloat_raise_flags(status, FLAG_OVERFLOW | FLAG_INEXACT);
            return pack_to_f16(sign, 0x1F, 0).wrapping_sub((round_increment == 0) as u16);
        }
    }
    sig = (sig.wrapping_add(round_increment)) >> 4;
    if round_bits != 0 {
        softfloat_raise_flags(status, FLAG_INEXACT);
    }
    sig &= !((((round_bits ^ 8) == 0) as u16) & (round_near_even as u16));
    if sig == 0 {
        exp = 0;
    }
    pack_to_f16(sign, exp, sig)
}

/// Berkeley `softfloat_roundPackToF32`, with Arm FPCR.FZ flushing.
pub(crate) fn round_pack_to_f32(
    sign: bool,
    mut exp: i16,
    mut sig: u32,
    status: &mut SoftFloatStatus,
) -> Float32 {
    let rounding_mode = softfloat_get_rounding_mode(status);
    let round_near_even = rounding_mode == ROUND_NEAR_EVEN;
    let mut round_increment: u32 = 0x40;
    if !round_near_even && rounding_mode != ROUND_NEAR_MAXMAG {
        round_increment = if rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX }) {
            0x7F
        } else {
            0
        };
    }
    let mut round_bits = sig & 0x7F;

    if 0xFD <= (exp as u16) {
        if exp < 0 {
            let is_tiny = tininess_before_rounding(status)
                || (exp < -1)
                || (sig.wrapping_add(round_increment) < 0x8000_0000);
            if is_tiny && softfloat_flush_underflow_to_zero(status) {
                softfloat_raise_flags(status, flush_flags(status));
                return pack_to_f32(sign, 0, 0);
            }
            sig = shift_right_jam32(sig, (-exp) as u16);
            exp = 0;
            round_bits = sig & 0x7F;
            if is_tiny && round_bits != 0 {
                softfloat_raise_flags(status, FLAG_UNDERFLOW);
            }
        } else if 0xFD < exp || 0x8000_0000 <= sig.wrapping_add(round_increment) {
            softfloat_raise_flags(status, FLAG_OVERFLOW | FLAG_INEXACT);
            return pack_to_f32(sign, 0xFF, 0).wrapping_sub((round_increment == 0) as u32);
        }
    }

    sig = sig.wrapping_add(round_increment) >> 7;
    if round_bits != 0 {
        softfloat_raise_flags(status, FLAG_INEXACT);
    }
    if round_near_even && round_bits == 0x40 {
        sig &= !1u32;
    }
    if sig == 0 {
        exp = 0;
    }
    pack_to_f32(sign, exp, sig)
}

/// Berkeley `softfloat_normRoundPackToF32`.
pub(crate) fn norm_round_pack_to_f32(
    sign: bool,
    exp: i16,
    sig: u32,
    status: &mut SoftFloatStatus,
) -> Float32 {
    let shift = count_leading_zeros32(sig) as i16 - 1;
    round_pack_to_f32(sign, exp - shift, sig << shift, status)
}

/// Berkeley `softfloat_roundPackToF64`, with Arm FPCR.FZ flushing.
pub(crate) fn round_pack_to_f64(
    sign: bool,
    mut exp: i16,
    mut sig: u64,
    status: &mut SoftFloatStatus,
) -> Float64 {
    let rounding_mode = softfloat_get_rounding_mode(status);
    let round_near_even = rounding_mode == ROUND_NEAR_EVEN;
    let mut round_increment: u64 = 0x200;
    if !round_near_even && rounding_mode != ROUND_NEAR_MAXMAG {
        round_increment = if rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX }) {
            0x3FF
        } else {
            0
        };
    }
    let mut round_bits = sig & 0x3FF;

    if 0x7FD <= (exp as u16) {
        if exp < 0 {
            let is_tiny = tininess_before_rounding(status)
                || (exp < -1)
                || (sig.wrapping_add(round_increment) < 0x8000_0000_0000_0000);
            if is_tiny && softfloat_flush_underflow_to_zero(status) {
                softfloat_raise_flags(status, flush_flags(status));
                return pack_to_f64(sign, 0, 0);
            }
            sig = shift_right_jam64(sig, (-exp) as u32);
            exp = 0;
            round_bits = sig & 0x3FF;
            if is_tiny && round_bits != 0 {
                softfloat_raise_flags(status, FLAG_UNDERFLOW);
            }
        } else if 0x7FD < exp || 0x8000_0000_0000_0000 <= sig.wrapping_add(round_increment) {
            softfloat_raise_flags(status, FLAG_OVERFLOW | FLAG_INEXACT);
            return pack_to_f64(sign, 0x7FF, 0).wrapping_sub((round_increment == 0) as u64);
        }
    }

    sig = sig.wrapping_add(round_increment) >> 10;
    if round_bits != 0 {
        softfloat_raise_flags(status, FLAG_INEXACT);
    }
    if round_near_even && round_bits == 0x200 {
        sig &= !1u64;
    }
    if sig == 0 {
        exp = 0;
    }
    pack_to_f64(sign, exp, sig)
}

/// Berkeley `softfloat_normRoundPackToF64`.
pub(crate) fn norm_round_pack_to_f64(
    sign: bool,
    exp: i16,
    sig: u64,
    status: &mut SoftFloatStatus,
) -> Float64 {
    let shift = count_leading_zeros64(sig) as i16 - 1;
    round_pack_to_f64(sign, exp - shift, sig << shift, status)
}

/// Berkeley `softfloat_shiftRightJam64Extra`: `a:extra` shifted right by
/// `dist`, with every bit shifted past `extra` jammed into its LSB.
#[inline]
pub(crate) fn shift_right_jam64_extra(a: u64, extra: u64, dist: u32) -> (u64, u64) {
    if dist < 64 {
        if dist == 0 {
            return (a, extra);
        }
        let v = a >> dist;
        let new_extra = (a << ((!dist).wrapping_add(1) & 63)) | ((extra != 0) as u64);
        (v, new_extra)
    } else {
        let new_extra = if dist == 64 {
            a | ((extra != 0) as u64)
        } else {
            ((a | extra) != 0) as u64
        };
        (0, new_extra)
    }
}

// ============================================================
// Round-to-integer helpers
// ============================================================

/// The response for a conversion `softfloat_roundToI32` cannot represent:
/// the invalid flag, and the saturated value for the sign.
fn round_to_i32_invalid(sign: bool, status: &mut SoftFloatStatus) -> i32 {
    softfloat_raise_flags(status, FLAG_INVALID);
    if sign {
        I32_FROM_NEG_OVERFLOW
    } else {
        I32_FROM_POS_OVERFLOW
    }
}

/// Berkeley `softfloat_roundToI32`: `sig` is a fixed-point value with the
/// binary point between bits 11 and 12.
pub(crate) fn softfloat_round_to_i32(
    sign: bool,
    sig: u64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i32 {
    let mut round_increment: u32 = 0x800;
    if rounding_mode != ROUND_NEAR_MAXMAG && rounding_mode != ROUND_NEAR_EVEN {
        round_increment = 0;
        if rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX }) {
            round_increment = 0xFFF;
        }
    }
    let round_bits = (sig & 0xFFF) as u32;
    let sig = sig.wrapping_add(u64::from(round_increment));
    if sig & 0xFFFF_F000_0000_0000 != 0 {
        return round_to_i32_invalid(sign, status);
    }
    let mut sig32 = (sig >> 12) as u32;
    if round_bits == 0x800 && rounding_mode == ROUND_NEAR_EVEN {
        sig32 &= !1;
    }
    let z = (if sign { sig32.wrapping_neg() } else { sig32 }) as i32;
    if z != 0 && ((z < 0) ^ sign) {
        return round_to_i32_invalid(sign, status);
    }
    if round_bits != 0 && exact {
        softfloat_raise_flags(status, FLAG_INEXACT);
    }
    z
}

/// The response for a conversion `softfloat_roundToI64` cannot represent:
/// the invalid flag, and the saturated value for the sign.
fn round_to_i64_invalid(sign: bool, status: &mut SoftFloatStatus) -> i64 {
    softfloat_raise_flags(status, FLAG_INVALID);
    if sign {
        I64_FROM_NEG_OVERFLOW
    } else {
        I64_FROM_POS_OVERFLOW
    }
}

/// Berkeley `softfloat_roundToI64`: the 128-bit fixed-point value
/// `sig:sig_extra`, binary point between the words.
pub(crate) fn softfloat_round_to_i64(
    sign: bool,
    sig: u64,
    sig_extra: u64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i64 {
    let increment = if rounding_mode == ROUND_NEAR_MAXMAG || rounding_mode == ROUND_NEAR_EVEN {
        0x8000_0000_0000_0000 <= sig_extra
    } else {
        sig_extra != 0 && rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX })
    };
    let mut sig = sig;
    if increment {
        sig = sig.wrapping_add(1);
        if sig == 0 {
            return round_to_i64_invalid(sign, status);
        }
        if sig_extra == 0x8000_0000_0000_0000 && rounding_mode == ROUND_NEAR_EVEN {
            sig &= !1;
        }
    }
    let z = (if sign { sig.wrapping_neg() } else { sig }) as i64;
    if z != 0 && ((z < 0) ^ sign) {
        return round_to_i64_invalid(sign, status);
    }
    if sig_extra != 0 && exact {
        softfloat_raise_flags(status, FLAG_INEXACT);
    }
    z
}
