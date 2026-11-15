//! Single-precision to signed-integer conversions.
//! Ported from Berkeley SoftFloat 3e f32_to_i32.c and f32_to_i64.c (via
//! Bochs softfloat3e). Under the Arm specialization a NaN converts to 0 while
//! overflow saturates, so the upstream
//! `#if (i32_fromNaN != i32_fromPosOverflow)` NaN pre-check is live.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Berkeley `f32_to_i32`.
pub(crate) fn f32_to_i32(
    a: Float32,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i32 {
    let sign = sign_f32(a);
    let exp = exp_f32(a);
    let mut sig = frac_f32(a);

    if exp == 0xFF && sig != 0 {
        softfloat_raise_flags(status, FLAG_INVALID);
        return I32_FROM_NAN;
    }
    if exp != 0 {
        sig |= 0x0080_0000;
    }

    let mut sig64 = (sig as u64) << 32;
    let shift_dist = 0xAA - exp;
    if 0 < shift_dist {
        sig64 = shift_right_jam64(sig64, shift_dist as u32);
    }
    softfloat_round_to_i32(sign, sig64, rounding_mode, exact, status)
}

/// Berkeley `f32_to_i64`.
pub(crate) fn f32_to_i64(
    a: Float32,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i64 {
    let sign = sign_f32(a);
    let exp = exp_f32(a);
    let mut sig = frac_f32(a);

    let shift_dist = 0xBE - exp;
    if shift_dist < 0 {
        softfloat_raise_flags(status, FLAG_INVALID);
        return if exp == 0xFF && sig != 0 {
            I64_FROM_NAN
        } else if sign {
            I64_FROM_NEG_OVERFLOW
        } else {
            I64_FROM_POS_OVERFLOW
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
    softfloat_round_to_i64(sign, sig64, extra, rounding_mode, exact, status)
}
