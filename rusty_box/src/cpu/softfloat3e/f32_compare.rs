#![allow(dead_code, non_snake_case)]
//! Float32 ordering comparison and min/max.
//! Ported from Bochs softfloat3e/f32_compare.cc + f32_min.cc + f32_max.cc.

use super::f32_class::f32_class;
use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;

/// Compare two single-precision numbers, returning one of `RELATION_LESS`,
/// `RELATION_EQUAL`, `RELATION_GREATER` or `RELATION_UNORDERED`.
/// Bochs softfloat3e/f32_compare.cc `f32_compare`.
pub(crate) fn f32_compare_full(
    mut a: float32,
    mut b: float32,
    quiet: bool,
    status: &mut SoftFloatStatus,
) -> i32 {
    let a_class = f32_class(a);
    let b_class = f32_class(b);

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
            a &= 0x8000_0000;
        } else {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
    }
    if b_class == SoftFloatClass::Denormal {
        if softfloat_denormalsAreZeros(status) {
            b &= 0x8000_0000;
        } else {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
    }

    if a == b || ((a | b) << 1) == 0 {
        return RELATION_EQUAL;
    }
    let sign_a = sign_f32(a);
    let sign_b = sign_f32(b);
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
/// Bochs softfloat.h `f32_compare(a, b, status)`.
#[inline]
pub(crate) fn f32_compare(a: float32, b: float32, status: &mut SoftFloatStatus) -> i32 {
    f32_compare_full(a, b, false, status)
}

/// Quiet compare — a QNaN operand does not raise #I.
/// Bochs softfloat.h `f32_compare_quiet`.
#[inline]
pub(crate) fn f32_compare_quiet(a: float32, b: float32, status: &mut SoftFloatStatus) -> i32 {
    f32_compare_full(a, b, true, status)
}

/// Bochs softfloat3e/f32_min.cc `f32_min`. When both operands compare equal
/// (including ±0.0) the *second* operand is returned, matching SSE MINPS.
pub(crate) fn f32_min(mut a: float32, mut b: float32, status: &mut SoftFloatStatus) -> float32 {
    if softfloat_denormalsAreZeros(status) {
        a = f32_denormal_to_zero(a);
        b = f32_denormal_to_zero(b);
    }
    if f32_compare(a, b, status) == RELATION_LESS {
        a
    } else {
        b
    }
}

/// Bochs softfloat3e/f32_max.cc `f32_max`.
pub(crate) fn f32_max(mut a: float32, mut b: float32, status: &mut SoftFloatStatus) -> float32 {
    if softfloat_denormalsAreZeros(status) {
        a = f32_denormal_to_zero(a);
        b = f32_denormal_to_zero(b);
    }
    if f32_compare(a, b, status) == RELATION_GREATER {
        a
    } else {
        b
    }
}
