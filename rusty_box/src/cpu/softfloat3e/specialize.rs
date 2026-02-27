#![allow(non_camel_case_types, dead_code)]
//! Specialization constants and NaN propagation for x86 FPU.
//! Ported from Bochs softfloat-specialize.h and softfloat3e/include/specialize.h.

use super::softfloat_types::*;
use super::softfloat::*;

// --- Indefinite values for integer conversions ---
pub const INT16_INDEFINITE: i16 = -0x7FFFi16 - 1;  // 0x8000
pub const INT32_INDEFINITE: i32 = -0x7FFFFFFFi32 - 1;  // 0x80000000
pub const INT64_INDEFINITE: i64 = -0x7FFFFFFFFFFFFFFFi64 - 1;  // 0x8000000000000000

pub const UINT16_INDEFINITE: u16 = 0xFFFF;
pub const UINT32_INDEFINITE: u32 = 0xFFFFFFFF;
pub const UINT64_INDEFINITE: u64 = 0xFFFFFFFFFFFFFFFF;

// --- Integer overflow/NaN conversion values ---
pub const I32_FROM_POS_OVERFLOW: i32 = i32::MIN;
pub const I32_FROM_NEG_OVERFLOW: i32 = i32::MIN;
pub const I32_FROM_NAN: i32 = i32::MIN;
pub const I64_FROM_POS_OVERFLOW: i64 = i64::MIN;
pub const I64_FROM_NEG_OVERFLOW: i64 = i64::MIN;
pub const I64_FROM_NAN: i64 = i64::MIN;
pub const UI32_FROM_POS_OVERFLOW: u32 = 0xFFFFFFFF;
pub const UI32_FROM_NEG_OVERFLOW: u32 = 0xFFFFFFFF;
pub const UI32_FROM_NAN: u32 = 0xFFFFFFFF;
pub const UI64_FROM_POS_OVERFLOW: u64 = 0xFFFFFFFFFFFFFFFF;
pub const UI64_FROM_NEG_OVERFLOW: u64 = 0xFFFFFFFFFFFFFFFF;
pub const UI64_FROM_NAN: u64 = 0xFFFFFFFFFFFFFFFF;

// --- Float16 constants ---
pub const FLOAT16_DEFAULT_NAN: float16 = 0xFE00;
pub const FLOAT16_EXP_BIAS: i32 = 0xF;

// --- Float32 constants ---
pub const FLOAT32_NEGATIVE_INF: float32 = 0xFF800000;
pub const FLOAT32_POSITIVE_INF: float32 = 0x7F800000;
pub const FLOAT32_NEGATIVE_ZERO: float32 = 0x80000000;
pub const FLOAT32_POSITIVE_ZERO: float32 = 0x00000000;
pub const FLOAT32_NEGATIVE_ONE: float32 = 0xBF800000;
pub const FLOAT32_POSITIVE_ONE: float32 = 0x3F800000;
pub const FLOAT32_MAX_FLOAT: float32 = 0x7F7FFFFF;
pub const FLOAT32_MIN_FLOAT: float32 = 0xFF7FFFFF;
pub const FLOAT32_DEFAULT_NAN: float32 = 0xFFC00000;
pub const FLOAT32_EXP_BIAS: i32 = 0x7F;

// --- Float64 constants ---
pub const FLOAT64_NEGATIVE_INF: float64 = 0xFFF0000000000000;
pub const FLOAT64_POSITIVE_INF: float64 = 0x7FF0000000000000;
pub const FLOAT64_NEGATIVE_ZERO: float64 = 0x8000000000000000;
pub const FLOAT64_POSITIVE_ZERO: float64 = 0x0000000000000000;
pub const FLOAT64_NEGATIVE_ONE: float64 = 0xBFF0000000000000;
pub const FLOAT64_POSITIVE_ONE: float64 = 0x3FF0000000000000;
pub const FLOAT64_MAX_FLOAT: float64 = 0x7FEFFFFFFFFFFFFF;
pub const FLOAT64_MIN_FLOAT: float64 = 0xFFEFFFFFFFFFFFFF;
pub const FLOAT64_DEFAULT_NAN: float64 = 0xFFF8000000000000;
pub const FLOAT64_EXP_BIAS: i32 = 0x3FF;

// --- ExtFloat80 constants ---
pub const FLOATX80_DEFAULT_NAN_EXP: u16 = 0xFFFF;
pub const FLOATX80_DEFAULT_NAN_FRACTION: u64 = 0xC000000000000000;
pub const FLOATX80_EXP_BIAS: i32 = 0x3FFF;

pub const FLOATX80_DEFAULT_NAN: floatx80 = floatx80 {
    signif: FLOATX80_DEFAULT_NAN_FRACTION,
    sign_exp: FLOATX80_DEFAULT_NAN_EXP,
};

// --- Pack helpers ---

#[inline]
pub fn pack_float16(sign: bool, exp: i16, sig: u16) -> float16 {
    ((sign as u16) << 15) + ((exp as u16) << 10) + sig
}

#[inline]
pub fn pack_float32(sign: bool, exp: i16, sig: u32) -> float32 {
    ((sign as u32) << 31) + ((exp as u32) << 23) + sig
}

#[inline]
pub fn pack_float64(sign: bool, exp: i16, sig: u64) -> float64 {
    ((sign as u64) << 63) + ((exp as u64) << 52) + sig
}

#[inline]
pub fn pack_floatx80(sign: bool, exp: i32, sig: u64) -> floatx80 {
    floatx80 {
        signif: sig,
        sign_exp: ((sign as u16) << 15) | (exp as u16),
    }
}

#[inline]
pub fn pack_to_extf80(sign_exp: u16, signif: u64) -> floatx80 {
    floatx80 { signif, sign_exp }
}

// --- NaN propagation for extFloat80 ---
pub fn softfloat_propagate_nan_extf80(
    a_sign_exp: u16, a_signif: u64,
    b_sign_exp: u16, b_signif: u64,
    status: &mut SoftFloatStatus,
) -> floatx80 {
    let a_is_snan = is_sig_nan_extf80(a_sign_exp, a_signif);
    let b_is_snan = is_sig_nan_extf80(b_sign_exp, b_signif);
    let a_is_nan = is_nan_extf80(a_sign_exp, a_signif);
    let b_is_nan = is_nan_extf80(b_sign_exp, b_signif);

    // Raise invalid if either is signaling NaN
    if a_is_snan | b_is_snan {
        softfloat_raiseFlags(status, FLAG_INVALID);
    }

    // Quieten signaling NaN by setting the quiet bit
    let mut a_signif_q = a_signif | 0x4000000000000000; // set quiet bit
    let b_signif_q = b_signif | 0x4000000000000000;

    if a_is_snan {
        if b_is_snan {
            // Both signaling: return the one with larger significand
            if a_signif_q < b_signif_q {
                return floatx80 { signif: b_signif_q, sign_exp: b_sign_exp };
            }
        }
        return floatx80 { signif: a_signif_q, sign_exp: a_sign_exp };
    }

    if a_is_nan {
        if b_is_snan {
            return floatx80 { signif: a_signif_q, sign_exp: a_sign_exp };
        }
        if b_is_nan {
            // Both quiet NaN: return the one with larger significand
            if a_signif_q < b_signif_q {
                return floatx80 { signif: b_signif_q, sign_exp: b_sign_exp };
            }
            if b_signif_q < a_signif_q {
                return floatx80 { signif: a_signif_q, sign_exp: a_sign_exp };
            }
            // Equal significands: return smaller sign_exp
            if a_sign_exp < b_sign_exp {
                return floatx80 { signif: a_signif_q, sign_exp: a_sign_exp };
            }
        }
        return floatx80 { signif: a_signif_q, sign_exp: a_sign_exp };
    }

    // Only b is NaN
    floatx80 { signif: b_signif_q, sign_exp: b_sign_exp }
}

/// Propagate NaN for f32
pub fn softfloat_propagate_nan_f32(a: float32, b: float32, status: &mut SoftFloatStatus) -> float32 {
    let a_is_snan = f32_is_signaling_nan(a);
    let b_is_snan = f32_is_signaling_nan(b);

    // Quieten SNaN
    let a_q = a | 0x00400000;
    let b_q = b | 0x00400000;

    if a_is_snan | b_is_snan {
        softfloat_raiseFlags(status, FLAG_INVALID);
    }

    if a_is_snan {
        if b_is_snan { return if a_q < b_q { b_q } else { a_q }; }
        return if f32_is_nan(b) { b_q } else { a_q };
    }
    if f32_is_nan(a) {
        if b_is_snan { return a_q; }
        if f32_is_nan(b) { return if a_q < b_q { b_q } else { a_q }; }
        return a_q;
    }
    b_q
}

/// Propagate NaN for f64
pub fn softfloat_propagate_nan_f64(a: float64, b: float64, status: &mut SoftFloatStatus) -> float64 {
    let a_is_snan = f64_is_signaling_nan(a);
    let b_is_snan = f64_is_signaling_nan(b);

    let a_q = a | 0x0008000000000000;
    let b_q = b | 0x0008000000000000;

    if a_is_snan | b_is_snan {
        softfloat_raiseFlags(status, FLAG_INVALID);
    }

    if a_is_snan {
        if b_is_snan { return if a_q < b_q { b_q } else { a_q }; }
        return if f64_is_nan(b) { b_q } else { a_q };
    }
    if f64_is_nan(a) {
        if b_is_snan { return a_q; }
        if f64_is_nan(b) { return if a_q < b_q { b_q } else { a_q }; }
        return a_q;
    }
    b_q
}

// --- Internal NaN detection helpers ---

#[inline]
fn is_nan_extf80(a64: u16, a0: u64) -> bool {
    ((a64 & 0x7FFF) == 0x7FFF) && ((a0 & 0x7FFFFFFFFFFFFFFF) != 0)
}

#[inline]
fn is_sig_nan_extf80(a64: u16, a0: u64) -> bool {
    ((a64 & 0x7FFF) == 0x7FFF)
        && (a0 & 0x4000000000000000 == 0)
        && (a0 & 0x3FFFFFFFFFFFFFFF != 0)
}
