// Ported wholesale from Berkeley SoftFloat 3e / Bochs softfloat3e: these
// modules carry the complete primitive surface, part of which no x86
// instruction reaches yet. Kept for parity with upstream rather than
// trimmed to current callers.
#![allow(dead_code)]
//! SoftFloat status, rounding modes, exception flags, and helper functions.
//! Ported from Berkeley SoftFloat 3e.

use super::softfloat_types::ExtFloat80;

/// Software floating-point status — passed by `&mut` to all operations.
#[derive(Debug, Clone)]
pub(in crate::cpu) struct SoftFloatStatus {
    pub softfloat_rounding_mode: u8,
    pub softfloat_exception_flags: i32,
    pub softfloat_exception_masks: i32,
    pub softfloat_suppress_exception: i32,
    pub(in crate::cpu) softfloat_denormals_are_zeros: bool,
    pub(in crate::cpu) softfloat_flush_underflow_to_zero: bool,
    /// Rounding precision for 80-bit extended double-precision.
    /// Valid values are 32, 64, and 80.
    pub extf80_rounding_precision: u8,
}

impl Default for SoftFloatStatus {
    fn default() -> Self {
        Self {
            softfloat_rounding_mode: ROUND_NEAR_EVEN,
            softfloat_exception_flags: 0,
            softfloat_exception_masks: 0x3f,
            softfloat_suppress_exception: 0,
            softfloat_denormals_are_zeros: false,
            softfloat_flush_underflow_to_zero: false,
            extf80_rounding_precision: 80,
        }
    }
}

// Rounding modes
pub(in crate::cpu) const ROUND_NEAR_EVEN: u8 = 0;
pub(in crate::cpu) const ROUND_MIN: u8 = 1;
pub(in crate::cpu) const ROUND_DOWN: u8 = ROUND_MIN;
pub(in crate::cpu) const ROUND_MAX: u8 = 2;
pub(in crate::cpu) const ROUND_UP: u8 = ROUND_MAX;
pub(in crate::cpu) const ROUND_MINMAG: u8 = 3;
pub(in crate::cpu) const ROUND_TO_ZERO: u8 = ROUND_MINMAG;
pub(in crate::cpu) const ROUND_NEAR_MAXMAG: u8 = 4;

// Exception flags
pub(in crate::cpu) const FLAG_INVALID: i32 = 0x01;
pub(in crate::cpu) const FLAG_DENORMAL: i32 = 0x02;
pub(in crate::cpu) const FLAG_DIVBYZERO: i32 = 0x04;
pub(in crate::cpu) const FLAG_INFINITE: i32 = FLAG_DIVBYZERO;
pub(in crate::cpu) const FLAG_OVERFLOW: i32 = 0x08;
pub(in crate::cpu) const FLAG_UNDERFLOW: i32 = 0x10;
pub(in crate::cpu) const FLAG_INEXACT: i32 = 0x20;

pub(in crate::cpu) const ALL_EXCEPTIONS_MASK: i32 = 0x3f;

/// C1 flag for ExtFloat80 rounding direction
pub(in crate::cpu) const RAISE_SW_C1: i32 = 0x0200;

// Relation constants
pub(in crate::cpu) const RELATION_LESS: i32 = -1;
pub(in crate::cpu) const RELATION_EQUAL: i32 = 0;
pub(in crate::cpu) const RELATION_GREATER: i32 = 1;
pub(in crate::cpu) const RELATION_UNORDERED: i32 = 2;

/// Floating-point class
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub(in crate::cpu) enum SoftFloatClass {
    Zero = 0,
    SNaN = 1,
    QNaN = 2,
    NegativeInf = 3,
    PositiveInf = 4,
    Denormal = 5,
    Normalized = 6,
}

// --- Helper functions on SoftFloatStatus ---

#[inline]
pub(in crate::cpu) fn softfloat_set_flags(status: &mut SoftFloatStatus, flags: i32) {
    status.softfloat_exception_flags = flags;
}

#[inline]
pub(in crate::cpu) fn softfloat_raise_flags(status: &mut SoftFloatStatus, flags: i32) {
    status.softfloat_exception_flags |= flags;
}

#[inline]
pub(in crate::cpu) fn softfloat_is_masked_exception(status: &SoftFloatStatus, flags: i32) -> bool {
    (status.softfloat_exception_masks & flags) != 0
}

#[inline]
pub(in crate::cpu) fn softfloat_suppress_exception(status: &mut SoftFloatStatus, flags: i32) {
    status.softfloat_suppress_exception |= flags;
}

#[inline]
pub(in crate::cpu) fn softfloat_get_rounding_mode(status: &SoftFloatStatus) -> u8 {
    status.softfloat_rounding_mode
}

#[inline]
pub(in crate::cpu) fn softfloat_denormals_are_zeros(status: &SoftFloatStatus) -> bool {
    status.softfloat_denormals_are_zeros
}

#[inline]
pub(in crate::cpu) fn softfloat_flush_underflow_to_zero(status: &SoftFloatStatus) -> bool {
    status.softfloat_flush_underflow_to_zero
}

#[inline]
pub(in crate::cpu) fn softfloat_extf80_rounding_precision(status: &SoftFloatStatus) -> u8 {
    status.extf80_rounding_precision
}

#[inline]
pub(in crate::cpu) fn softfloat_get_exception_flags(status: &SoftFloatStatus) -> i32 {
    status.softfloat_exception_flags & !status.softfloat_suppress_exception
}

#[inline]
pub(in crate::cpu) fn softfloat_set_rounding_up(status: &mut SoftFloatStatus) {
    status.softfloat_exception_flags |= RAISE_SW_C1;
}

// --- ExtFloat80 helpers (from softfloat-extra.h and softfloat-specialize.h) ---

#[inline]
pub(in crate::cpu) fn extf80_sign(a: ExtFloat80) -> bool {
    (a.sign_exp >> 15) != 0
}

#[inline]
pub(in crate::cpu) fn extf80_exp(a: ExtFloat80) -> i32 {
    (a.sign_exp & 0x7FFF) as i32
}

#[inline]
pub(in crate::cpu) fn extf80_fraction(a: ExtFloat80) -> u64 {
    a.signif
}

#[inline]
pub(in crate::cpu) fn extf80_is_unsupported(a: ExtFloat80) -> bool {
    ((a.sign_exp & 0x7FFF) != 0) && (a.signif & 0x8000000000000000 == 0)
}

#[inline]
pub(in crate::cpu) fn extf80_is_nan(a: ExtFloat80) -> bool {
    ((a.sign_exp & 0x7FFF) == 0x7FFF) && (a.signif & 0x7FFFFFFFFFFFFFFF != 0)
}

#[inline]
pub(in crate::cpu) fn extf80_is_signaling_nan(a: ExtFloat80) -> bool {
    ((a.sign_exp & 0x7FFF) == 0x7FFF)
        && (a.signif & 0x4000000000000000 == 0)
        && (a.signif & 0x3FFFFFFFFFFFFFFF != 0)
}

#[inline]
pub(in crate::cpu) fn floatx80_chs(a: ExtFloat80) -> ExtFloat80 {
    ExtFloat80 {
        signif: a.signif,
        sign_exp: a.sign_exp ^ 0x8000,
    }
}

#[inline]
pub(in crate::cpu) fn floatx80_abs(a: ExtFloat80) -> ExtFloat80 {
    ExtFloat80 {
        signif: a.signif,
        sign_exp: a.sign_exp & 0x7FFF,
    }
}

// f16 helpers
#[inline]
pub(in crate::cpu) fn f16_is_nan(a: u16) -> bool {
    ((!a & 0x7C00) == 0) && ((a & 0x03FF) != 0)
}

#[inline]
pub(in crate::cpu) fn f16_is_signaling_nan(a: u16) -> bool {
    ((a & 0x7E00) == 0x7C00) && ((a & 0x01FF) != 0)
}

// f32 helpers
#[inline]
pub(in crate::cpu) fn f32_sign(a: u32) -> bool {
    (a >> 31) != 0
}

#[inline]
pub(in crate::cpu) fn f32_exp(a: u32) -> i16 {
    ((a >> 23) & 0xFF) as i16
}

#[inline]
pub(in crate::cpu) fn f32_fraction(a: u32) -> u32 {
    a & 0x007FFFFF
}

#[inline]
pub(in crate::cpu) fn f32_is_nan(a: u32) -> bool {
    ((!a & 0x7F800000) == 0) && ((a & 0x007FFFFF) != 0)
}

#[inline]
pub(in crate::cpu) fn f32_is_signaling_nan(a: u32) -> bool {
    ((a & 0x7FC00000) == 0x7F800000) && ((a & 0x003FFFFF) != 0)
}

/// Bochs softfloat3e/include/softfloat-extra.h `f32_denormal_to_zero`.
#[inline]
pub(in crate::cpu) fn f32_denormal_to_zero(a: u32) -> u32 {
    if f32_exp(a) == 0 && f32_fraction(a) != 0 {
        return a & 0x80000000;
    }
    a
}

// f64 helpers
#[inline]
pub(in crate::cpu) fn f64_sign(a: u64) -> bool {
    (a >> 63) != 0
}

#[inline]
pub(in crate::cpu) fn f64_exp(a: u64) -> i16 {
    ((a >> 52) & 0x7FF) as i16
}

#[inline]
pub(in crate::cpu) fn f64_fraction(a: u64) -> u64 {
    a & 0x000FFFFFFFFFFFFF
}

#[inline]
pub(in crate::cpu) fn f64_is_nan(a: u64) -> bool {
    ((!a & 0x7FF0000000000000) == 0) && ((a & 0x000FFFFFFFFFFFFF) != 0)
}

#[inline]
pub(in crate::cpu) fn f64_is_signaling_nan(a: u64) -> bool {
    ((a & 0x7FF8000000000000) == 0x7FF0000000000000) && ((a & 0x0007FFFFFFFFFFFF) != 0)
}

/// Bochs softfloat3e/include/softfloat-extra.h `f64_denormal_to_zero`.
#[inline]
pub(in crate::cpu) fn f64_denormal_to_zero(a: u64) -> u64 {
    if f64_exp(a) == 0 && f64_fraction(a) != 0 {
        return a & 0x8000000000000000;
    }
    a
}
