//! Arm FMAX/FMIN/FMAXNM/FMINNM.
//!
//! Not a SoftFloat port: Berkeley SoftFloat 3e has no min/max, and the Bochs
//! softfloat3e `f32_min`/`f32_max` implement x86 MINPS/MAXPS (second operand
//! on ties and NaNs). These follow the Arm ARM pseudocode FPMax, FPMin,
//! FPMaxNum and FPMinNum for FPCR.AH = 0. The operands arrive already
//! through FPUnpack (FPCR.FZ flushing), so an exact operand is returned
//! unchanged: FPRound of a representable value is that value.

use super::f32_compare::f32_compare;
use super::f64_compare::f64_compare;
use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Which extreme a min/max selects.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Extreme {
    Max,
    Min,
}

#[inline]
fn is_qnan_f32(a: Float32) -> bool {
    is_nan_f32(a) && !f32_is_signaling_nan(a)
}

#[inline]
fn is_qnan_f64(a: Float64) -> bool {
    is_nan_f64(a) && !f64_is_signaling_nan(a)
}

/// Arm FPMax / FPMin. A NaN operand goes through FPProcessNaNs. Between
/// zeros, the maximum is -0 only when both are -0 and the minimum is +0 only
/// when both are +0 (FPMax `sign1 AND sign2`, FPMin `sign1 OR sign2`).
pub(crate) fn f32_min_max(
    a: Float32,
    b: Float32,
    extreme: Extreme,
    status: &mut SoftFloatStatus,
) -> Float32 {
    if is_nan_f32(a) || is_nan_f32(b) {
        return softfloat_propagate_nan_f32(a, b, status);
    }
    if ((a | b) & 0x7FFF_FFFF) == 0 {
        return match extreme {
            Extreme::Max => a & b,
            Extreme::Min => a | b,
        };
    }
    let ordering = f32_compare(a, b, true, status);
    let take_a = match extreme {
        Extreme::Max => ordering == FpOrdering::Greater,
        Extreme::Min => ordering == FpOrdering::Less,
    };
    if take_a {
        a
    } else {
        b
    }
}

/// Arm FPMaxNum / FPMinNum: a single quiet NaN is replaced by the infinity
/// that never wins (-inf for the maximum, +inf for the minimum) before
/// FPMax / FPMin. Two quiet NaNs, or any signaling NaN, still propagate.
pub(crate) fn f32_min_max_num(
    a: Float32,
    b: Float32,
    extreme: Extreme,
    status: &mut SoftFloatStatus,
) -> Float32 {
    let loser: Float32 = match extreme {
        Extreme::Max => 0xFF80_0000,
        Extreme::Min => 0x7F80_0000,
    };
    let (a, b) = match (is_qnan_f32(a), is_qnan_f32(b)) {
        (true, false) => (loser, b),
        (false, true) => (a, loser),
        _ => (a, b),
    };
    f32_min_max(a, b, extreme, status)
}

/// Double-precision form of [`f32_min_max`].
pub(crate) fn f64_min_max(
    a: Float64,
    b: Float64,
    extreme: Extreme,
    status: &mut SoftFloatStatus,
) -> Float64 {
    if is_nan_f64(a) || is_nan_f64(b) {
        return softfloat_propagate_nan_f64(a, b, status);
    }
    if ((a | b) & 0x7FFF_FFFF_FFFF_FFFF) == 0 {
        return match extreme {
            Extreme::Max => a & b,
            Extreme::Min => a | b,
        };
    }
    let ordering = f64_compare(a, b, true, status);
    let take_a = match extreme {
        Extreme::Max => ordering == FpOrdering::Greater,
        Extreme::Min => ordering == FpOrdering::Less,
    };
    if take_a {
        a
    } else {
        b
    }
}

/// Double-precision form of [`f32_min_max_num`].
pub(crate) fn f64_min_max_num(
    a: Float64,
    b: Float64,
    extreme: Extreme,
    status: &mut SoftFloatStatus,
) -> Float64 {
    let loser: Float64 = match extreme {
        Extreme::Max => 0xFFF0_0000_0000_0000,
        Extreme::Min => 0x7FF0_0000_0000_0000,
    };
    let (a, b) = match (is_qnan_f64(a), is_qnan_f64(b)) {
        (true, false) => (loser, b),
        (false, true) => (a, loser),
        _ => (a, b),
    };
    f64_min_max(a, b, extreme, status)
}
