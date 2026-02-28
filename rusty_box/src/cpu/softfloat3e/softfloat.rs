#![allow(non_camel_case_types, dead_code)]
//! SoftFloat status, rounding modes, exception flags, and helper functions.
//! Ported from Berkeley SoftFloat 3e.

use super::softfloat_types::floatx80;

/// Software floating-point status — passed by `&mut` to all operations.
#[derive(Debug, Clone)]
pub struct SoftFloatStatus {
    pub softfloat_roundingMode: u8,
    pub softfloat_exceptionFlags: i32,
    pub softfloat_exceptionMasks: i32,
    pub softfloat_suppressException: i32,
    pub(crate) softfloat_denormals_are_zeros: bool,
    pub(crate) softfloat_flush_underflow_to_zero: bool,
    /// Rounding precision for 80-bit extended double-precision.
    /// Valid values are 32, 64, and 80.
    pub extF80_roundingPrecision: u8,
}

impl Default for SoftFloatStatus {
    fn default() -> Self {
        Self {
            softfloat_roundingMode: ROUND_NEAR_EVEN,
            softfloat_exceptionFlags: 0,
            softfloat_exceptionMasks: 0x3f,
            softfloat_suppressException: 0,
            softfloat_denormals_are_zeros: false,
            softfloat_flush_underflow_to_zero: false,
            extF80_roundingPrecision: 80,
        }
    }
}

// Rounding modes
pub const ROUND_NEAR_EVEN: u8 = 0;
pub const ROUND_MIN: u8 = 1;
pub const ROUND_DOWN: u8 = ROUND_MIN;
pub const ROUND_MAX: u8 = 2;
pub const ROUND_UP: u8 = ROUND_MAX;
pub const ROUND_MINMAG: u8 = 3;
pub const ROUND_TO_ZERO: u8 = ROUND_MINMAG;
pub const ROUND_NEAR_MAXMAG: u8 = 4;

// Exception flags
pub const FLAG_INVALID: i32 = 0x01;
pub const FLAG_DENORMAL: i32 = 0x02;
pub const FLAG_DIVBYZERO: i32 = 0x04;
pub const FLAG_INFINITE: i32 = FLAG_DIVBYZERO;
pub const FLAG_OVERFLOW: i32 = 0x08;
pub const FLAG_UNDERFLOW: i32 = 0x10;
pub const FLAG_INEXACT: i32 = 0x20;

pub const ALL_EXCEPTIONS_MASK: i32 = 0x3f;

/// C1 flag for floatx80 rounding direction
pub const RAISE_SW_C1: i32 = 0x0200;

// Relation constants
pub const RELATION_LESS: i32 = -1;
pub const RELATION_EQUAL: i32 = 0;
pub const RELATION_GREATER: i32 = 1;
pub const RELATION_UNORDERED: i32 = 2;

/// Floating-point class
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SoftFloatClass {
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
pub fn softfloat_setFlags(status: &mut SoftFloatStatus, flags: i32) {
    status.softfloat_exceptionFlags = flags;
}

#[inline]
pub fn softfloat_raiseFlags(status: &mut SoftFloatStatus, flags: i32) {
    status.softfloat_exceptionFlags |= flags;
}

#[inline]
pub fn softfloat_isMaskedException(status: &SoftFloatStatus, flags: i32) -> bool {
    (status.softfloat_exceptionMasks & flags) != 0
}

#[inline]
pub fn softfloat_suppressException(status: &mut SoftFloatStatus, flags: i32) {
    status.softfloat_suppressException |= flags;
}

#[inline]
pub fn softfloat_getRoundingMode(status: &SoftFloatStatus) -> u8 {
    status.softfloat_roundingMode
}

#[inline]
pub fn softfloat_denormalsAreZeros(status: &SoftFloatStatus) -> bool {
    status.softfloat_denormals_are_zeros
}

#[inline]
pub fn softfloat_flushUnderflowToZero(status: &SoftFloatStatus) -> bool {
    status.softfloat_flush_underflow_to_zero
}

#[inline]
pub fn softfloat_extF80_roundingPrecision(status: &SoftFloatStatus) -> u8 {
    status.extF80_roundingPrecision
}

#[inline]
pub fn softfloat_getExceptionFlags(status: &SoftFloatStatus) -> i32 {
    status.softfloat_exceptionFlags & !status.softfloat_suppressException
}

#[inline]
pub fn softfloat_setRoundingUp(status: &mut SoftFloatStatus) {
    status.softfloat_exceptionFlags |= RAISE_SW_C1;
}

// --- floatx80 helpers (from softfloat-extra.h and softfloat-specialize.h) ---

#[inline]
pub fn extf80_sign(a: floatx80) -> bool {
    (a.sign_exp >> 15) != 0
}

#[inline]
pub fn extf80_exp(a: floatx80) -> i32 {
    (a.sign_exp & 0x7FFF) as i32
}

#[inline]
pub fn extf80_fraction(a: floatx80) -> u64 {
    a.signif
}

#[inline]
pub fn extf80_is_unsupported(a: floatx80) -> bool {
    ((a.sign_exp & 0x7FFF) != 0) && (a.signif & 0x8000000000000000 == 0)
}

#[inline]
pub fn extf80_is_nan(a: floatx80) -> bool {
    ((a.sign_exp & 0x7FFF) == 0x7FFF) && (a.signif & 0x7FFFFFFFFFFFFFFF != 0)
}

#[inline]
pub fn extf80_is_signaling_nan(a: floatx80) -> bool {
    ((a.sign_exp & 0x7FFF) == 0x7FFF)
        && (a.signif & 0x4000000000000000 == 0)
        && (a.signif & 0x3FFFFFFFFFFFFFFF != 0)
}

#[inline]
pub fn floatx80_chs(a: floatx80) -> floatx80 {
    floatx80 {
        signif: a.signif,
        sign_exp: a.sign_exp ^ 0x8000,
    }
}

#[inline]
pub fn floatx80_abs(a: floatx80) -> floatx80 {
    floatx80 {
        signif: a.signif,
        sign_exp: a.sign_exp & 0x7FFF,
    }
}

// f32 helpers
#[inline]
pub fn f32_sign(a: u32) -> bool {
    (a >> 31) != 0
}

#[inline]
pub fn f32_exp(a: u32) -> i16 {
    ((a >> 23) & 0xFF) as i16
}

#[inline]
pub fn f32_fraction(a: u32) -> u32 {
    a & 0x007FFFFF
}

#[inline]
pub fn f32_is_nan(a: u32) -> bool {
    ((!a & 0x7F800000) == 0) && ((a & 0x007FFFFF) != 0)
}

#[inline]
pub fn f32_is_signaling_nan(a: u32) -> bool {
    ((a & 0x7FC00000) == 0x7F800000) && ((a & 0x003FFFFF) != 0)
}

// f64 helpers
#[inline]
pub fn f64_sign(a: u64) -> bool {
    (a >> 63) != 0
}

#[inline]
pub fn f64_exp(a: u64) -> i16 {
    ((a >> 52) & 0x7FF) as i16
}

#[inline]
pub fn f64_fraction(a: u64) -> u64 {
    a & 0x000FFFFFFFFFFFFF
}

#[inline]
pub fn f64_is_nan(a: u64) -> bool {
    ((!a & 0x7FF0000000000000) == 0) && ((a & 0x000FFFFFFFFFFFFF) != 0)
}

#[inline]
pub fn f64_is_signaling_nan(a: u64) -> bool {
    ((a & 0x7FF8000000000000) == 0x7FF0000000000000) && ((a & 0x0007FFFFFFFFFFFF) != 0)
}
