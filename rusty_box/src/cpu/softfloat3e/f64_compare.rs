#![allow(dead_code, non_snake_case)]
//! Float64 ordering comparison and min/max.
//! Ported from Bochs softfloat3e/f64_compare.cc + f64_min.cc + f64_max.cc.

use super::f64_class::f64_class;
use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;

/// Compare two double-precision numbers, returning one of `RELATION_LESS`,
/// `RELATION_EQUAL`, `RELATION_GREATER` or `RELATION_UNORDERED`.
/// Bochs softfloat3e/f64_compare.cc `f64_compare`.
pub(crate) fn f64_compare_full(
    mut a: float64,
    mut b: float64,
    quiet: bool,
    status: &mut SoftFloatStatus,
) -> i32 {
    let a_class = f64_class(a);
    let b_class = f64_class(b);

    if a_class == SoftFloatClass::SNaN || b_class == SoftFloatClass::SNaN {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return RELATION_UNORDERED;
    }
    if a_class == SoftFloatClass::QNaN || b_class == SoftFloatClass::QNaN {
        if !quiet {
            softfloat_raiseFlags(status, FLAG_INVALID);
        }
        return RELATION_UNORDERED;
    }
    if a_class == SoftFloatClass::Denormal {
        if softfloat_denormalsAreZeros(status) {
            a &= 0x8000_0000_0000_0000;
        } else {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
    }
    if b_class == SoftFloatClass::Denormal {
        if softfloat_denormalsAreZeros(status) {
            b &= 0x8000_0000_0000_0000;
        } else {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
    }

    if a == b || ((a | b) << 1) == 0 {
        return RELATION_EQUAL;
    }
    let sign_a = sign_f64(a);
    let sign_b = sign_f64(b);
    if sign_a != sign_b {
        return if sign_a {
            RELATION_LESS
        } else {
            RELATION_GREATER
        };
    }
    if sign_a ^ (a < b) {
        return RELATION_LESS;
    }
    RELATION_GREATER
}

/// Signaling compare — raises #I on a QNaN operand.
/// Bochs softfloat.h `f64_compare(a, b, status)`.
#[inline]
pub(crate) fn f64_compare(a: float64, b: float64, status: &mut SoftFloatStatus) -> i32 {
    f64_compare_full(a, b, false, status)
}

/// Quiet compare — a QNaN operand does not raise #I.
/// Bochs softfloat.h `f64_compare_quiet`.
#[inline]
pub(crate) fn f64_compare_quiet(a: float64, b: float64, status: &mut SoftFloatStatus) -> i32 {
    f64_compare_full(a, b, true, status)
}

/// Bochs softfloat3e/f64_min.cc `f64_min`. When both operands compare equal
/// (including ±0.0) the *second* operand is returned, matching SSE MINPD.
pub(crate) fn f64_min(mut a: float64, mut b: float64, status: &mut SoftFloatStatus) -> float64 {
    if softfloat_denormalsAreZeros(status) {
        a = f64_denormal_to_zero(a);
        b = f64_denormal_to_zero(b);
    }
    if f64_compare(a, b, status) == RELATION_LESS {
        a
    } else {
        b
    }
}

/// Bochs softfloat3e/f64_max.cc `f64_max`.
pub(crate) fn f64_max(mut a: float64, mut b: float64, status: &mut SoftFloatStatus) -> float64 {
    if softfloat_denormalsAreZeros(status) {
        a = f64_denormal_to_zero(a);
        b = f64_denormal_to_zero(b);
    }
    if f64_compare(a, b, status) == RELATION_GREATER {
        a
    } else {
        b
    }
}
