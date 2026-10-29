//! BFloat16 arithmetic (FEAT_BF16).
//!
//! BFDOT and BFMMLA (Arm BFDotAdd, without FEAT_EBF16) follow BFloat16's
//! own computation behaviours rather than FPCR: their products (BFMulH)
//! and sums (FPAdd_BF16) take a denormal operand as a zero of its sign
//! (BFUnpack), round to odd, make a result below 2^-126 before rounding a
//! zero of its sign and one that overflows an infinity (BFRound), give the
//! default NaN for a NaN operand or an invalid operation, and raise no
//! exception. Round to odd is round toward zero with the lowest bit set
//! where that lost anything, so each is a single-precision operation in
//! round toward zero with flush to zero and the default NaN, on a status of
//! its own, with its overflow and its inexact result corrected. Of FPCR,
//! they read FPCR.AH alone: it is the default NaN's sign (FPDefaultNaN).
//!
//! BFCVT and BFCVTN (Arm FPConvertBF) round a single-precision value to
//! BFloat16 under FPCR, as any FPRound does, with BFloat16's 7 fraction
//! bits in single precision's exponent range — under FPCR.AH = 1 with every
//! denormal flushed, rounding to nearest and no exception.

use super::softfloat::{
    softfloat_raise_flags, Handling, SoftFloatStatus, FLAG_INEXACT, FLAG_INVALID, FLAG_OVERFLOW,
    FLAG_UNDERFLOW, ROUND_MINMAG,
};
use super::specialize::{f32_flush_input, float32_default_nan};
use super::{f32_addsub, f32_mul, ArmFpStatus, FpRounding};

/// Arm BFUnpack's fixed flush to zero: a denormal is a zero of its sign.
fn unpacked(a: u32) -> u32 {
    if (a >> 23) & 0xFF == 0 {
        a & 0x8000_0000
    } else {
        a
    }
}

/// Arm BFRound of the exact result `op` computes: rounded to odd, a result
/// below 2^-126 before rounding flushed to a zero of its sign, an overflow
/// an infinity, a NaN the default NaN of FPCR.AH (`handling`).
fn rounded_to_odd<F: FnOnce(&mut SoftFloatStatus) -> u32>(handling: Handling, op: F) -> u32 {
    // BFRound flushes before rounding whatever FPCR.AH is: the status is
    // the standard one, and only the default NaN follows `handling`.
    let mut status = SoftFloatStatus {
        softfloat_rounding_mode: ROUND_MINMAG,
        default_nan_mode: true,
        flush_to_zero: true,
        ..SoftFloatStatus::default()
    };
    let r = op(&mut status);
    let flags = status.softfloat_exception_flags;
    if flags & FLAG_OVERFLOW != 0 {
        // Round toward zero stops at the largest normal.
        return (r & 0x8000_0000) | 0x7F80_0000;
    }
    let exp = (r >> 23) & 0xFF;
    if exp == 0xFF && r & 0x007F_FFFF != 0 {
        return float32_default_nan(&SoftFloatStatus { handling, ..status });
    }
    // A zero, exact or flushed, stays one; so does an infinity.
    if exp != 0 && exp != 0xFF && flags & FLAG_INEXACT != 0 {
        r | 1
    } else {
        r
    }
}

/// Arm BFMulH: the product of two BFloat16 values, in single precision.
fn bf16_mul(a: u16, b: u16, handling: Handling) -> u32 {
    rounded_to_odd(handling, |s| {
        f32_mul::f32_mul(
            unpacked(u32::from(a) << 16),
            unpacked(u32::from(b) << 16),
            s,
        )
    })
}

/// Arm FPAdd_BF16: the sum of two single-precision values.
fn bf16_add(a: u32, b: u32, handling: Handling) -> u32 {
    rounded_to_odd(handling, |s| {
        f32_addsub::f32_add(unpacked(a), unpacked(b), s)
    })
}

/// BFDOT and BFMMLA on one element: Arm BFDotAdd without FEAT_EBF16,
/// `addend + (op1_a*op2_a + op1_b*op2_b)`, under FPCR.AH (`handling`).
pub(crate) fn bf16_dot_add(
    addend: u32,
    op1_a: u16,
    op1_b: u16,
    op2_a: u16,
    op2_b: u16,
    handling: Handling,
) -> u32 {
    let product = bf16_add(
        bf16_mul(op1_a, op2_a, handling),
        bf16_mul(op1_b, op2_b, handling),
        handling,
    );
    bf16_add(addend, product, handling)
}

/// BFCVT and BFCVTN on one element: Arm FPConvertBF, single-precision `a`
/// rounded by `rounding` to BFloat16, under FPCR.FZ (a denormal operand a
/// zero, raising input denormal) and FPCR.DN, raising Arm FPRoundBase's
/// exceptions: inexact, underflow for an inexact result tiny before
/// rounding, overflow for one rounded to an infinity, and invalid for a
/// signalling NaN. Under FPCR.AH = 1 it runs with every denormal flushed,
/// rounding to nearest and no exception (FPConvertBF's `altfp`).
pub(crate) fn f32_to_bf16(a: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> u16 {
    let rounding = match fp.handling() {
        Handling::Standard => rounding,
        Handling::Alternate => FpRounding::TieEven,
    };
    fp.with_altfp_overrides(|fp| to_bf16(a, rounding, &mut fp.status))
}

/// [`f32_to_bf16`] in the environment it chose.
fn to_bf16(a: u32, rounding: FpRounding, s: &mut SoftFloatStatus) -> u16 {
    let a = f32_flush_input(a, s);
    let negative = a >> 31 == 1;
    let exp = (a >> 23) & 0xFF;
    let frac = a & 0x007F_FFFF;
    if exp == 0xFF && frac != 0 {
        // Arm FPConvertNaN keeps the payload, quieted.
        if frac & 0x0040_0000 == 0 {
            softfloat_raise_flags(s, FLAG_INVALID);
        }
        let nan = if s.default_nan_mode {
            float32_default_nan(s)
        } else {
            a | 0x0040_0000
        };
        return (nan >> 16) as u16;
    }
    let truncated = a >> 16;
    let lost = a & 0xFFFF;
    if exp == 0xFF || lost == 0 {
        // An infinity, a zero, or a value BFloat16 holds exactly.
        return truncated as u16;
    }
    let up = match rounding {
        FpRounding::TieEven => lost > 0x8000 || (lost == 0x8000 && truncated & 1 == 1),
        // FPCR.RMode cannot select it, so no instruction asks for it here
        // (FPRoundBase asserts so); rounding to nearest, ties away.
        FpRounding::TieAway => lost >= 0x8000,
        FpRounding::PosInf => !negative,
        FpRounding::NegInf => negative,
        FpRounding::Zero => false,
    };
    let result = truncated + u32::from(up);
    if exp == 0 {
        softfloat_raise_flags(s, FLAG_UNDERFLOW);
    }
    if (result >> 7) & 0xFF == 0xFF {
        softfloat_raise_flags(s, FLAG_OVERFLOW);
    }
    softfloat_raise_flags(s, FLAG_INEXACT);
    result as u16
}
