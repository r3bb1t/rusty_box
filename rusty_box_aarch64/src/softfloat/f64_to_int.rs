//! Double-precision to signed-integer conversions.
//! Ported from Berkeley SoftFloat 3e f64_to_i32.c and f64_to_i64.c (via
//! Bochs softfloat3e). As in [`super::f32_to_int`], the upstream NaN
//! pre-check is live under the Arm specialization.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Berkeley `f64_to_i32`.
pub(crate) fn f64_to_i32(
    a: Float64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i32 {
    let sign = sign_f64(a);
    let exp = exp_f64(a);
    let mut sig = frac_f64(a);

    if exp == 0x7FF && sig != 0 {
        softfloat_raise_flags(status, FLAG_INVALID);
        return I32_FROM_NAN;
    }
    if exp != 0 {
        sig |= 0x0010_0000_0000_0000;
    }

    let shift_dist = 0x427 - exp;
    if 0 < shift_dist {
        sig = shift_right_jam64(sig, shift_dist as u32);
    }
    softfloat_round_to_i32(sign, sig, rounding_mode, exact, status)
}

/// Berkeley `f64_to_i64`.
pub(crate) fn f64_to_i64(
    a: Float64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i64 {
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
                I64_FROM_NAN
            } else if sign {
                I64_FROM_NEG_OVERFLOW
            } else {
                I64_FROM_POS_OVERFLOW
            };
        }
        (sig << (-shift_dist), 0u64)
    } else {
        shift_right_jam64_extra(sig, 0, shift_dist as u32)
    };
    softfloat_round_to_i64(sign, sig_v, sig_extra, rounding_mode, exact, status)
}
