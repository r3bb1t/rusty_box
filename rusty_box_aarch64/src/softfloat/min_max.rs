//! Arm FMAX/FMIN/FMAXNM/FMINNM.
//!
//! Not a SoftFloat port: Berkeley SoftFloat 3e has no min/max, and the Bochs
//! softfloat3e `f32_min`/`f32_max` implement x86 MINPS/MAXPS (second operand
//! on ties and NaNs). These follow the Arm ARM pseudocode FPMax, FPMin,
//! FPMaxNum and FPMinNum. The operands arrive already through FPUnpack
//! (operand flushing), so an exact operand is returned unchanged unless
//! FPRound flushes it: FPRound of a representable value is that value.

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

/// The rules FPMax and FPMin follow, their `altfp` argument: FMAX and FMIN
/// pass FPCR.AH, FPMaxNum and FPMinNum always the IEEE ones.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MinMaxRules {
    /// NaNs through FPProcessNaNs; between zeros, the maximum is -0 only
    /// when both are -0 and the minimum +0 only when both are +0; the
    /// result rounded under FPCR, so FPCR.FZ may flush it.
    Ieee,
    /// FPCR.AH = 1's FMAX and FMIN: a NaN operand, or two zeros of
    /// opposite signs, give the second operand as it is, a NaN raising
    /// Invalid Operation; the result is never flushed.
    Alternate,
}

#[inline]
fn is_qnan_f32(a: Float32) -> bool {
    is_nan_f32(a) && !f32_is_signaling_nan(a)
}

#[inline]
fn is_qnan_f64(a: Float64) -> bool {
    is_nan_f64(a) && !f64_is_signaling_nan(a)
}

/// Arm FPMax / FPMin under `rules`. A used denormal operand raises Input
/// Denormal under FPCR.AH = 1 (FPProcessDenorms) when no NaN decided the
/// result.
pub(crate) fn f32_min_max(
    a: Float32,
    b: Float32,
    extreme: Extreme,
    rules: MinMaxRules,
    status: &mut SoftFloatStatus,
) -> Float32 {
    if rules == MinMaxRules::Alternate {
        if ((a | b) & 0x7FFF_FFFF) == 0 && (a ^ b) & 0x8000_0000 != 0 {
            return b;
        }
        if is_nan_f32(a) || is_nan_f32(b) {
            softfloat_raise_flags(status, FLAG_INVALID);
            return b;
        }
    }
    if is_nan_f32(a) || is_nan_f32(b) {
        return softfloat_propagate_nan_f32(a, b, status);
    }
    f32_report_denormals(&[a, b], status);
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
    let result = if take_a { a } else { b };
    if rules == MinMaxRules::Ieee
        && softfloat_flush_underflow_to_zero(status)
        && exp_f32(result) == 0
        && frac_f32(result) != 0
    {
        // FPRound flushes a denormal value: reachable under FPCR.AH = 1
        // alone, as FPCR.FZ flushed the operands under FPCR.AH = 0.
        softfloat_raise_flags(status, flush_flags(status));
        return result & 0x8000_0000;
    }
    result
}

/// Arm FPMaxNum / FPMinNum: a single quiet NaN is replaced by the infinity
/// that never wins (-inf for the maximum, +inf for the minimum) before
/// FPMax / FPMin with the IEEE rules — except, under FPCR.AH = 1, when
/// both operands are NaNs. Two quiet NaNs, or any signaling NaN, still
/// propagate.
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
    let both_nan = is_nan_f32(a) && is_nan_f32(b);
    let (a, b) = if status.handling == Handling::Alternate && both_nan {
        (a, b)
    } else {
        match (is_qnan_f32(a), is_qnan_f32(b)) {
            (true, false) => (loser, b),
            (false, true) => (a, loser),
            _ => (a, b),
        }
    };
    f32_min_max(a, b, extreme, MinMaxRules::Ieee, status)
}

/// Double-precision form of [`f32_min_max`].
pub(crate) fn f64_min_max(
    a: Float64,
    b: Float64,
    extreme: Extreme,
    rules: MinMaxRules,
    status: &mut SoftFloatStatus,
) -> Float64 {
    const SIGN: Float64 = 0x8000_0000_0000_0000;
    if rules == MinMaxRules::Alternate {
        if ((a | b) & !SIGN) == 0 && (a ^ b) & SIGN != 0 {
            return b;
        }
        if is_nan_f64(a) || is_nan_f64(b) {
            softfloat_raise_flags(status, FLAG_INVALID);
            return b;
        }
    }
    if is_nan_f64(a) || is_nan_f64(b) {
        return softfloat_propagate_nan_f64(a, b, status);
    }
    f64_report_denormals(&[a, b], status);
    if ((a | b) & !SIGN) == 0 {
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
    let result = if take_a { a } else { b };
    if rules == MinMaxRules::Ieee
        && softfloat_flush_underflow_to_zero(status)
        && exp_f64(result) == 0
        && frac_f64(result) != 0
    {
        softfloat_raise_flags(status, flush_flags(status));
        return result & SIGN;
    }
    result
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
    let both_nan = is_nan_f64(a) && is_nan_f64(b);
    let (a, b) = if status.handling == Handling::Alternate && both_nan {
        (a, b)
    } else {
        match (is_qnan_f64(a), is_qnan_f64(b)) {
            (true, false) => (loser, b),
            (false, true) => (a, loser),
            _ => (a, b),
        }
    };
    f64_min_max(a, b, extreme, MinMaxRules::Ieee, status)
}
