//! The host's IEEE 754 arithmetic, where it gives exactly the bits and the
//! exceptions SoftFloat gives.
//!
//! QEMU fpu/softfloat.c computes on the host ("hardfloat": `can_use_fpu`,
//! `float32_gen2`/`float64_gen2`, `f64_is_zon2`, `f64_addsubmul_post`)
//! when the rounding is to nearest even and inexact is already raised, so
//! that whether the host's result is inexact does not matter, and when the
//! operands are of a kind on which the host and SoftFloat agree. This
//! module applies that rule more narrowly:
//!
//! - both operands are normal, so that FPCR.FZ flushes neither and no NaN
//!   or infinity has a rule of its own;
//! - the result is finite, and larger in magnitude than the smallest
//!   normal number.
//!
//! The result bound means the operation did not overflow. It also means
//! the exact value was not tiny: Arm detects tininess before rounding, and
//! an exact value below the smallest normal rounds to at most the smallest
//! normal itself, which is left to SoftFloat. So FPCR.FZ has no tiny result
//! to flush, and FPCR.DN has no NaN to choose.
//!
//! IEEE 754 then defines the result as the exact value rounded to nearest
//! even, which is what both compute. The only exception either raises is
//! inexact, which is already set. Every other case is `None`, and SoftFloat
//! computes it.
//!
//! Rust's `f32` and `f64` arithmetic is IEEE 754 binary32 and binary64,
//! rounding to nearest even, on every target but 32-bit x86 without SSE2:
//! there x87 rounds twice, and this module answers `None`.

use super::softfloat::{FLAG_INEXACT, ROUND_NEAR_EVEN};
use super::ArmFpStatus;

#[cfg(test)]
#[path = "../../tests/unit/host_float.rs"]
mod tests;

/// Whether the host's arithmetic rounds once, as IEEE 754 binary32 and
/// binary64 do.
const HOST_ROUNDS_ONCE: bool = !cfg!(all(target_arch = "x86", not(target_feature = "sse2")));

/// The operations the host computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Arith {
    Add,
    Sub,
    Mul,
    Div,
}

/// Whether the host may compute under `fp`: rounding to nearest even,
/// with inexact already raised.
#[inline(always)]
fn host_may_round(fp: &ArmFpStatus) -> bool {
    HOST_ROUNDS_ONCE
        && fp.status.softfloat_rounding_mode == ROUND_NEAR_EVEN
        && fp.status.softfloat_exception_flags & FLAG_INEXACT != 0
}

/// Whether the binary64 `bits` are a normal number.
#[inline(always)]
const fn f64_normal(bits: u64) -> bool {
    let exp = (bits >> 52) & 0x7FF;
    exp != 0 && exp != 0x7FF
}

/// Whether the binary32 `bits` are a normal number.
#[inline(always)]
const fn f32_normal(bits: u32) -> bool {
    let exp = (bits >> 23) & 0xFF;
    exp != 0 && exp != 0xFF
}

/// `a op b` in binary64, if the host computes it as SoftFloat does.
#[inline(always)]
pub(super) fn f64_arith(op: Arith, a: u64, b: u64, fp: &ArmFpStatus) -> Option<u64> {
    if !(host_may_round(fp) && f64_normal(a) && f64_normal(b)) {
        return None;
    }
    let (x, y) = (f64::from_bits(a), f64::from_bits(b));
    let r = match op {
        Arith::Add => x + y,
        Arith::Sub => x - y,
        Arith::Mul => x * y,
        Arith::Div => x / y,
    }
    .to_bits();
    // The magnitude, above the smallest normal and below infinity (a NaN's
    // magnitude is above infinity's).
    let magnitude = r & !(1 << 63);
    (magnitude > f64::MIN_POSITIVE.to_bits() && magnitude < f64::INFINITY.to_bits()).then_some(r)
}

/// `a op b` in binary32, if the host computes it as SoftFloat does.
#[inline(always)]
pub(super) fn f32_arith(op: Arith, a: u32, b: u32, fp: &ArmFpStatus) -> Option<u32> {
    if !(host_may_round(fp) && f32_normal(a) && f32_normal(b)) {
        return None;
    }
    let (x, y) = (f32::from_bits(a), f32::from_bits(b));
    let r = match op {
        Arith::Add => x + y,
        Arith::Sub => x - y,
        Arith::Mul => x * y,
        Arith::Div => x / y,
    }
    .to_bits();
    let magnitude = r & !(1 << 31);
    (magnitude > f32::MIN_POSITIVE.to_bits() && magnitude < f32::INFINITY.to_bits()).then_some(r)
}
