//! Float32 ordering comparison.
//! Ported from Bochs softfloat3e/f32_compare.cc, with the result as an
//! [`FpOrdering`] and the x86 denormal reporting left to the callers' FPCR.FZ
//! input flushing (Arm FPCompare).

use super::f32_class::f32_class;
use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;

/// Bochs softfloat3e/f32_compare.cc `f32_compare`. A signaling NaN always
/// raises invalid; a quiet NaN raises it only when `quiet` is false (FCMPE).
pub(crate) fn f32_compare(
    a: Float32,
    b: Float32,
    quiet: bool,
    status: &mut SoftFloatStatus,
) -> FpOrdering {
    let a_class = f32_class(a);
    let b_class = f32_class(b);

    if a_class == SoftFloatClass::SNaN || b_class == SoftFloatClass::SNaN {
        softfloat_raise_flags(status, FLAG_INVALID);
        return FpOrdering::Unordered;
    }
    if a_class == SoftFloatClass::QNaN || b_class == SoftFloatClass::QNaN {
        if !quiet {
            softfloat_raise_flags(status, FLAG_INVALID);
        }
        return FpOrdering::Unordered;
    }

    if a == b || ((a | b) << 1) == 0 {
        return FpOrdering::Equal;
    }
    let sign_a = sign_f32(a);
    let sign_b = sign_f32(b);
    if sign_a != sign_b {
        return if sign_a {
            FpOrdering::Less
        } else {
            FpOrdering::Greater
        };
    }
    if sign_a ^ (a < b) {
        return FpOrdering::Less;
    }
    FpOrdering::Greater
}
