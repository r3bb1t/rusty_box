//! Arm operations built on the SoftFloat core that Berkeley SoftFloat has no
//! counterpart for:
//!
//! - the fixed-point conversions (Arm `FixedToFP` and `FPToFixed` with
//!   fraction bits: SCVTF, UCVTF, FCVTZS, FCVTZU with `#fbits`);
//! - FJCVTZS's JavaScript conversion (`FPToFixedJS`);
//! - FCVTXN's round-to-odd narrowing (`FPRounding_ODD`);
//! - the alternative half-precision format selected by FPCR.AHP, which has
//!   no infinities or NaNs (`FPConvert`, `FPRoundBase` with `fpcr.AHP`);
//! - FMULX (`FPMulX`) and the Newton-Raphson steps FRECPS (`FPRecipStepFused`)
//!   and FRSQRTS (`FPRSqrtStepFused`).
//!
//! Behaviour follows the Arm ARM pseudocode named on each function; where it
//! is ambiguous, QEMU 11.1.2 was the reference: `target/arm/tcg/vfp_helper.c`
//! for the conversions, `target/arm/tcg/helper-a64.c` for FMULX
//! (`HELPER(vfp_mulxs)`, `HELPER(vfp_mulxd)`), FRECPS (`DO_RECPS`), FRSQRTS
//! (`DO_RSQRTSF`) and FCVTXN (`HELPER(fcvtx_f64_to_f32)`, which narrows
//! under `float_round_to_odd`), and `fpu/softfloat-parts.c.inc`
//! (`partsN(uncanon_normal)`) for round-to-odd itself.

use super::f32_mul::f32_mul as sf_f32_mul;
use super::f32_mul_add::f32_mul_add as sf_f32_mul_add;
use super::f64_mul::f64_mul as sf_f64_mul;
use super::f64_mul_add::f64_mul_add as sf_f64_mul_add;
use super::f64_to_f32::f64_to_f32 as sf_f64_to_f32;
use super::f64_to_int::{f64_to_i32 as sf_f64_to_i32, f64_to_i64 as sf_f64_to_i64};
use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::specialize::{
    f32_flush_input, f32_report_denormals, f64_flush_input, f64_report_denormals, softfloat_propagate_nan_f32,
    softfloat_propagate_nan_f64, tininess_before_rounding,
};
use super::uint64_convert::f64_to_ui64 as sf_f64_to_ui64;
use super::uint_convert::f64_to_ui32 as sf_f64_to_ui32;
use super::{
    f32_neg, f32_report_denormals_unless_nan, f64_neg, f64_report_denormals_unless_nan, with_rounding, ArmFpStatus,
    FpRounding,
};

const F32_SIGN: u32 = 0x8000_0000;
const F64_SIGN: u64 = 0x8000_0000_0000_0000;
const F32_ONE_POINT_FIVE: u32 = 0x3FC0_0000;
const F32_TWO: u32 = 0x4000_0000;
const F64_ONE_POINT_FIVE: u64 = 0x3FF8_0000_0000_0000;
const F64_TWO: u64 = 0x4000_0000_0000_0000;

#[inline]
fn f32_is_inf(a: u32) -> bool {
    a & !F32_SIGN == 0x7F80_0000
}
#[inline]
fn f32_is_zero(a: u32) -> bool {
    a & !F32_SIGN == 0
}
#[inline]
fn f64_is_inf(a: u64) -> bool {
    a & !F64_SIGN == 0x7FF0_0000_0000_0000
}
#[inline]
fn f64_is_zero(a: u64) -> bool {
    a & !F64_SIGN == 0
}

// ============================================================
// Fixed point to floating point (Arm FixedToFP)
// ============================================================

/// `magnitude` (negated when `negative`) divided by 2^`fbits`, rounded once
/// to single precision. The quotient's exponent lies far inside the
/// format's normal range for any 64-bit magnitude and `fbits <= 64`.
fn fixed_to_f32(negative: bool, magnitude: u64, fbits: u32, status: &mut SoftFloatStatus) -> u32 {
    if magnitude == 0 {
        return 0;
    }
    let lz = u32::from(count_leading_zeros64(magnitude));
    let norm = magnitude << lz;
    // Leading bit at 30 with the dropped bits jammed, as round_pack wants.
    let sig = ((norm >> 33) as u32) | u32::from(norm & ((1 << 33) - 1) != 0);
    round_pack_to_f32(negative, (0x9C + 33 - lz as i32 - fbits as i32) as i16, sig, status)
}

/// As [`fixed_to_f32`], to double precision.
fn fixed_to_f64(negative: bool, magnitude: u64, fbits: u32, status: &mut SoftFloatStatus) -> u64 {
    if magnitude == 0 {
        return 0;
    }
    let lz = u32::from(count_leading_zeros64(magnitude));
    let norm = magnitude << lz;
    // Leading bit at 62 with the dropped bit jammed.
    let sig = (norm >> 1) | (norm & 1);
    round_pack_to_f64(negative, (0x43C + 1 - lz as i32 - fbits as i32) as i16, sig, status)
}

/// SCVTF (fixed point) to single: a signed value with `fbits` fraction bits.
pub(crate) fn i64_to_f32_fixed(a: i64, fbits: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> u32 {
    with_rounding(fp, rounding, |s| fixed_to_f32(a < 0, a.unsigned_abs(), fbits, s))
}

/// UCVTF (fixed point) to single.
pub(crate) fn u64_to_f32_fixed(a: u64, fbits: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> u32 {
    with_rounding(fp, rounding, |s| fixed_to_f32(false, a, fbits, s))
}

/// SCVTF (fixed point) to double.
pub(crate) fn i64_to_f64_fixed(a: i64, fbits: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> u64 {
    with_rounding(fp, rounding, |s| fixed_to_f64(a < 0, a.unsigned_abs(), fbits, s))
}

/// UCVTF (fixed point) to double.
pub(crate) fn u64_to_f64_fixed(a: u64, fbits: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> u64 {
    with_rounding(fp, rounding, |s| fixed_to_f64(false, a, fbits, s))
}

// ============================================================
// Floating point to fixed point (Arm FPToFixed with fraction bits)
// ============================================================

/// A single-precision value as a double, exactly and without exceptions. A
/// NaN stays a NaN: its payload does not matter, because FPToFixed turns
/// every NaN into 0 with Invalid Operation.
fn f32_widened(a: u32) -> u64 {
    let sign = sign_f32(a);
    let exp = exp_f32(a);
    let frac = frac_f32(a);
    if exp == 0xFF {
        return if frac != 0 {
            pack_to_f64(sign, 0x7FF, 0x0008_0000_0000_0000)
        } else {
            pack_to_f64(sign, 0x7FF, 0)
        };
    }
    if exp == 0 {
        if frac == 0 {
            return pack_to_f64(sign, 0, 0);
        }
        let norm = norm_subnormal_f32_sig(frac);
        return pack_to_f64(sign, norm.exp - 1 + 0x380, u64::from(norm.sig) << 29);
    }
    pack_to_f64(sign, exp + 0x380, u64::from(frac) << 29)
}

/// `a` (already flushed) times 2^`fbits`, exactly, as a double — or a stand-in
/// that every rounding mode turns into the same integer with the same
/// exceptions:
///
/// - a denormal's product is below 2^-958, far under one half, so it rounds
///   exactly like the smallest normal of its sign (to 0 or ±1 by the
///   rounding direction, inexact either way);
/// - a product beyond the double range saturates exactly like an infinity
///   of its sign (Invalid Operation alone).
fn f64_scaled(a: u64, fbits: u32) -> u64 {
    let sign = sign_f64(a);
    let mut exp = exp_f64(a);
    let mut frac = frac_f64(a);
    if exp == 0x7FF || (exp == 0 && frac == 0) {
        return a;
    }
    if exp == 0 {
        exp = 1;
        frac = 0;
    }
    let scaled = i32::from(exp) + fbits as i32;
    if scaled >= 0x7FF {
        return pack_to_f64(sign, 0x7FF, 0);
    }
    pack_to_f64(sign, scaled as i16, frac)
}

/// FCVTZS (fixed point) from single to a 32-bit integer with `fbits`
/// fraction bits.
pub(crate) fn f32_to_i32_fixed(a: u32, fbits: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> i32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    sf_f64_to_i32(f64_scaled(f32_widened(a), fbits), rounding.softfloat_mode(), true, s)
}

/// FCVTZS (fixed point) from single to a 64-bit integer.
pub(crate) fn f32_to_i64_fixed(a: u32, fbits: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> i64 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    sf_f64_to_i64(f64_scaled(f32_widened(a), fbits), rounding.softfloat_mode(), true, s)
}

/// FCVTZU (fixed point) from single to a 32-bit integer.
pub(crate) fn f32_to_u32_fixed(a: u32, fbits: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    sf_f64_to_ui32(f64_scaled(f32_widened(a), fbits), rounding.softfloat_mode(), true, s)
}

/// FCVTZU (fixed point) from single to a 64-bit integer.
pub(crate) fn f32_to_u64_fixed(a: u32, fbits: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    sf_f64_to_ui64(f64_scaled(f32_widened(a), fbits), rounding.softfloat_mode(), true, s)
}

/// FCVTZS (fixed point) from double to a 32-bit integer.
pub(crate) fn f64_to_i32_fixed(a: u64, fbits: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> i32 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    sf_f64_to_i32(f64_scaled(a, fbits), rounding.softfloat_mode(), true, s)
}

/// FCVTZS (fixed point) from double to a 64-bit integer.
pub(crate) fn f64_to_i64_fixed(a: u64, fbits: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> i64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    sf_f64_to_i64(f64_scaled(a, fbits), rounding.softfloat_mode(), true, s)
}

/// FCVTZU (fixed point) from double to a 32-bit integer.
pub(crate) fn f64_to_u32_fixed(a: u64, fbits: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    sf_f64_to_ui32(f64_scaled(a, fbits), rounding.softfloat_mode(), true, s)
}

/// FCVTZU (fixed point) from double to a 64-bit integer.
pub(crate) fn f64_to_u64_fixed(a: u64, fbits: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    sf_f64_to_ui64(f64_scaled(a, fbits), rounding.softfloat_mode(), true, s)
}

// ============================================================
// JavaScript conversion (FJCVTZS, Arm FPToFixedJS)
// ============================================================

/// What FJCVTZS makes of a double: the 32 bits it writes, and `Z`, whether
/// they are the value exactly.
pub(crate) struct JsConversion {
    pub(crate) value: u32,
    pub(crate) exact: bool,
}

/// FJCVTZS: Arm FPToFixedJS, `op` rounded toward zero to an integer and
/// wrapped to its low 32 bits (JavaScript's ToInt32). A NaN, an infinity or
/// an integer outside the 32-bit signed range raises invalid, and gives 0
/// for the first two; a rounded value raises inexact; `exact` holds when
/// neither did and the value is not -0.0 or a denormal FPCR.FZ flushed.
pub(crate) fn f64_to_i32_js(op: u64, fp: &mut ArmFpStatus) -> JsConversion {
    const FRAC: u64 = (1 << 52) - 1;
    let s = &mut fp.status;
    let a = f64_flush_input(op, s);
    let negative = op >> 63 == 1;
    let exp = ((a >> 52) & 0x7FF) as i32;
    let frac = a & FRAC;
    if exp == 0x7FF {
        softfloat_raise_flags(s, FLAG_INVALID);
        return JsConversion { value: 0, exact: false };
    }
    if exp == 0 && frac == 0 {
        return JsConversion { value: 0, exact: !negative && op & FRAC == 0 };
    }
    // The value is `sig` × 2^`shift`.
    let (sig, shift) = if exp == 0 { (frac, -1074) } else { (frac | (1 << 52), exp - 1075) };
    let (low, in_range, rounded) = if shift >= 0 {
        // At least 2^52 in magnitude: beyond the range, the low bits zero
        // from 2^32 up.
        let low = if shift >= 32 { 0 } else { (sig << shift) as u32 };
        (low, false, false)
    } else {
        let (magnitude, rounded) = if shift <= -64 {
            (0, true)
        } else {
            (sig >> -shift, sig & ((1 << -shift) - 1) != 0)
        };
        let limit = if negative { 1 << 31 } else { (1 << 31) - 1 };
        (magnitude as u32, magnitude <= limit, rounded)
    };
    let value = if negative { low.wrapping_neg() } else { low };
    if !in_range {
        softfloat_raise_flags(s, FLAG_INVALID);
    } else if rounded {
        softfloat_raise_flags(s, FLAG_INEXACT);
    }
    JsConversion { value, exact: in_range && !rounded }
}

// ============================================================
// Round to odd (FCVTXN)
// ============================================================

/// FCVTXN: Arm FPConvert to single precision with `FPRounding_ODD` — round
/// toward zero, then set the lowest bit if that lost anything. Overflow
/// gives the largest normal (OFC, IXC); FPCR.FZ still flushes tiny results,
/// and FPRoundBase returns a flushed zero before rounding to odd, so its
/// lowest bit stays clear (under FPCR.AH = 1 the flush raises Inexact too).
pub(crate) fn f64_to_f32_round_odd(a: u64, fp: &mut ArmFpStatus) -> u32 {
    let mut local = SoftFloatStatus {
        softfloat_rounding_mode: ROUND_MINMAG,
        softfloat_exception_flags: 0,
        ..fp.status
    };
    let a = f64_flush_input(a, &mut local);
    f64_report_denormals(&[a], &mut local);
    let result = sf_f64_to_f32(a, &mut local);
    fp.status.softfloat_exception_flags |= local.softfloat_exception_flags;
    // Toward zero, a nonzero operand gives a zero only when it is below the
    // smallest denormal, which FPCR.FZ flushes.
    let flushed = local.flush_to_zero && result & !F32_SIGN == 0 && a & !F64_SIGN != 0;
    if local.softfloat_exception_flags & FLAG_INEXACT != 0 && !flushed {
        result | 1
    } else {
        result
    }
}

// ============================================================
// Alternative half precision (FPCR.AHP)
// ============================================================

/// The largest alternative half-precision magnitude (131008), signed.
#[inline]
fn ahp_max(sign: bool) -> u16 {
    (u16::from(sign) << 15) | 0x7FFF
}

/// FCVT from an alternative half-precision value: exact and exception
/// free. The format has no infinities or NaNs; exponent 31 is an ordinary
/// binade, and half-precision inputs are never flushed.
pub(crate) fn f16_ahp_to_f32(a: u16) -> u32 {
    let sign = sign_f16(a);
    let exp = exp_f16(a);
    let frac = frac_f16(a);
    if exp == 0 {
        if frac == 0 {
            return pack_to_f32(sign, 0, 0);
        }
        let norm = norm_subnormal_f16_sig(frac);
        return pack_to_f32(sign, norm.exp - 1 + 0x70, u32::from(norm.sig) << 13);
    }
    pack_to_f32(sign, exp + 0x70, u32::from(frac) << 13)
}

/// As [`f16_ahp_to_f32`], to double precision.
pub(crate) fn f16_ahp_to_f64(a: u16) -> u64 {
    let sign = sign_f16(a);
    let exp = exp_f16(a);
    let frac = frac_f16(a);
    if exp == 0 {
        if frac == 0 {
            return pack_to_f64(sign, 0, 0);
        }
        let norm = norm_subnormal_f16_sig(frac);
        return pack_to_f64(sign, norm.exp - 1 + 0x3F0, u64::from(norm.sig) << 42);
    }
    pack_to_f64(sign, exp + 0x3F0, u64::from(frac) << 42)
}

/// Berkeley `softfloat_roundPackToF16` for the alternative format
/// (`FPRoundBase` with `fpcr.AHP`): exponent 31 holds ordinary values, and a
/// result whose rounded exponent would exceed it saturates to the largest
/// magnitude, raising Invalid Operation and nothing else. The subnormal
/// range is that of IEEE half precision, tininess is decided as FPCR.AH
/// says, and it never flushes.
fn round_pack_to_f16_ahp(sign: bool, exp: i16, sig: u16, status: &mut SoftFloatStatus) -> u16 {
    let rounding_mode = softfloat_get_rounding_mode(status);
    let round_near_even = rounding_mode == ROUND_NEAR_EVEN;
    let mut round_increment: u16 = 0x8;
    if !round_near_even && rounding_mode != ROUND_NEAR_MAXMAG {
        round_increment = if rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX }) {
            0xF
        } else {
            0
        };
    }
    let mut round_bits = sig & 0xF;
    let mut exp = exp;
    let mut sig = sig;

    if 0x1E <= (exp as u16) {
        if exp < 0 {
            let is_tiny =
                tininess_before_rounding(status) || f16_tiny_after_rounding(sign, exp, sig, rounding_mode);
            sig = shift_right_jam32(u32::from(sig), (-(exp as i32)) as u16) as u16;
            exp = 0;
            round_bits = sig & 0xF;
            if is_tiny && round_bits != 0 {
                softfloat_raise_flags(status, FLAG_UNDERFLOW);
            }
        } else if 0x1E < exp || 0x8000 <= u32::from(sig) + u32::from(round_increment) {
            softfloat_raise_flags(status, FLAG_INVALID);
            return ahp_max(sign);
        }
    }
    sig = (sig + round_increment) >> 4;
    if round_bits != 0 {
        softfloat_raise_flags(status, FLAG_INEXACT);
    }
    if round_near_even && round_bits == 8 {
        sig &= !1;
    }
    if sig == 0 {
        exp = 0;
    }
    pack_to_f16(sign, exp, sig)
}

/// FCVT Hd, Sn under FPCR.AHP: a NaN becomes a zero of its sign and an
/// infinity the largest magnitude, both raising Invalid Operation (Arm
/// FPConvert); other values round as [`round_pack_to_f16_ahp`] does, a
/// denormal one reported under FPCR.AH = 1 (FPProcessDenorm).
pub(crate) fn f32_to_f16_ahp(a: u32, fp: &mut ArmFpStatus) -> u16 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    f32_report_denormals(&[a], s);
    let sign = sign_f32(a);
    let exp = exp_f32(a);
    let frac = frac_f32(a);
    if exp == 0xFF {
        softfloat_raise_flags(s, FLAG_INVALID);
        return if frac != 0 { pack_to_f16(sign, 0, 0) } else { ahp_max(sign) };
    }
    // Sticky-collapse the 9 bits that do not survive the narrowing.
    let frac16 = ((frac >> 9) | u32::from(frac & 0x1FF != 0)) as u16;
    if exp == 0 && frac16 == 0 {
        return pack_to_f16(sign, 0, 0);
    }
    round_pack_to_f16_ahp(sign, exp - 0x71, frac16 | 0x4000, s)
}

/// FCVT Hd, Dn under FPCR.AHP; see [`f32_to_f16_ahp`].
pub(crate) fn f64_to_f16_ahp(a: u64, fp: &mut ArmFpStatus) -> u16 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    f64_report_denormals(&[a], s);
    let sign = sign_f64(a);
    let exp = exp_f64(a);
    let frac = frac_f64(a);
    if exp == 0x7FF {
        softfloat_raise_flags(s, FLAG_INVALID);
        return if frac != 0 { pack_to_f16(sign, 0, 0) } else { ahp_max(sign) };
    }
    let frac16 = short_shift_right_jam64(frac, 38) as u16;
    if (exp as u16 | frac16) == 0 {
        return pack_to_f16(sign, 0, 0);
    }
    round_pack_to_f16_ahp(sign, exp - 0x3F1, frac16 | 0x4000, s)
}

// ============================================================
// FMULX, FRECPS, FRSQRTS
// ============================================================

/// FMULX (single): Arm FPMulX — FPMul, except that infinity times zero is 2.0
/// with the exclusive-or of the operands' signs.
pub(crate) fn f32_mulx(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    f32_report_denormals_unless_nan(&[a, b], s);
    if (f32_is_inf(a) && f32_is_zero(b)) || (f32_is_zero(a) && f32_is_inf(b)) {
        return F32_TWO | ((a ^ b) & F32_SIGN);
    }
    sf_f32_mul(a, b, s)
}

/// FMULX (double).
pub(crate) fn f64_mulx(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    f64_report_denormals_unless_nan(&[a, b], s);
    if (f64_is_inf(a) && f64_is_zero(b)) || (f64_is_zero(a) && f64_is_inf(b)) {
        return F64_TWO | ((a ^ b) & F64_SIGN);
    }
    sf_f64_mul(a, b, s)
}

/// Runs a Newton-Raphson step as its pseudocode asks
/// ([`ArmFpStatus::with_altfp_overrides`]).
fn step<T>(fp: &mut ArmFpStatus, op: impl FnOnce(&mut SoftFloatStatus) -> T) -> T {
    fp.with_altfp_overrides(|fp| op(&mut fp.status))
}

/// FRECPS (single): Arm FPRecipStepFused — `2.0 - a*b` with one rounding;
/// infinity times zero gives 2.0. `a` is negated (FPNeg) before NaN
/// selection, which takes `a` then `b`.
pub(crate) fn f32_recip_step(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    step(fp, |s| {
        let a = f32_flush_input(f32_neg(a, s), s);
        let b = f32_flush_input(b, s);
        if (f32_is_inf(a) && f32_is_zero(b)) || (f32_is_zero(a) && f32_is_inf(b)) {
            return F32_TWO;
        }
        sf_f32_mul_add(a, b, F32_TWO, s)
    })
}

/// FRECPS (double).
pub(crate) fn f64_recip_step(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    step(fp, |s| {
        let a = f64_flush_input(f64_neg(a, s), s);
        let b = f64_flush_input(b, s);
        if (f64_is_inf(a) && f64_is_zero(b)) || (f64_is_zero(a) && f64_is_inf(b)) {
            return F64_TWO;
        }
        sf_f64_mul_add(a, b, F64_TWO, s)
    })
}

/// FRSQRTS (single): Arm FPRSqrtStepFused — `(3.0 - a*b) / 2.0` with one
/// rounding; infinity times zero gives 1.5. Computed as `1.5 + (-a)*(b/2)`
/// with whichever operand halves exactly halved (its exponent field is at
/// least 2). When neither does, both are below 2^-125, so the product only
/// decides the direction of the rounding away from 1.5 and need not be
/// halved.
pub(crate) fn f32_rsqrt_step(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    step(fp, |s| f32_rsqrt_step_under(a, b, s))
}

/// [`f32_rsqrt_step`] in the environment [`step`] chose.
fn f32_rsqrt_step_under(a: u32, b: u32, s: &mut SoftFloatStatus) -> u32 {
    let a = f32_flush_input(f32_neg(a, s), s);
    let b = f32_flush_input(b, s);
    if is_nan_f32(a) || is_nan_f32(b) {
        return sf_f32_mul_add(a, b, F32_ONE_POINT_FIVE, s);
    }
    if (f32_is_inf(a) && f32_is_zero(b)) || (f32_is_zero(a) && f32_is_inf(b)) {
        return F32_ONE_POINT_FIVE;
    }
    if f32_is_inf(a) || f32_is_inf(b) {
        return 0x7F80_0000 | ((a ^ b) & F32_SIGN);
    }
    let (a, b) = if exp_f32(b) >= 2 {
        (a, b - (1 << 23))
    } else if exp_f32(a) >= 2 {
        (a - (1 << 23), b)
    } else {
        (a, b)
    };
    sf_f32_mul_add(a, b, F32_ONE_POINT_FIVE, s)
}

/// FRSQRTS (double); see [`f32_rsqrt_step`] (the unhalved case has both
/// operands below 2^-1021).
pub(crate) fn f64_rsqrt_step(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    step(fp, |s| f64_rsqrt_step_under(a, b, s))
}

/// [`f64_rsqrt_step`] in the environment [`step`] chose.
fn f64_rsqrt_step_under(a: u64, b: u64, s: &mut SoftFloatStatus) -> u64 {
    let a = f64_flush_input(f64_neg(a, s), s);
    let b = f64_flush_input(b, s);
    if is_nan_f64(a) || is_nan_f64(b) {
        return sf_f64_mul_add(a, b, F64_ONE_POINT_FIVE, s);
    }
    if (f64_is_inf(a) && f64_is_zero(b)) || (f64_is_zero(a) && f64_is_inf(b)) {
        return F64_ONE_POINT_FIVE;
    }
    if f64_is_inf(a) || f64_is_inf(b) {
        return 0x7FF0_0000_0000_0000 | ((a ^ b) & F64_SIGN);
    }
    let (a, b) = if exp_f64(b) >= 2 {
        (a, b - (1 << 52))
    } else if exp_f64(a) >= 2 {
        (a - (1 << 52), b)
    } else {
        (a, b)
    };
    sf_f64_mul_add(a, b, F64_ONE_POINT_FIVE, s)
}

// ============================================================
// FSCALE, FLOGB (SVE)
// ============================================================

/// Arm FPClampScale: `scale` limited to the range over which the result
/// of scaling an operand of biased exponent `exp`, in a format of
/// `exp_bits` and `frac_bits`, can change; beyond it every result has
/// overflowed or rounded to zero alike.
pub(super) fn clamp_scale(scale: i64, exp: i64, exp_bits: u32, frac_bits: u32) -> i64 {
    let emax = (1i64 << exp_bits) - 1;
    let min_scale = -(i64::from(frac_bits) + 1);
    let max_scale = emax + i64::from(frac_bits) + 1;
    (min_scale - exp).max(scale.min(max_scale - exp))
}

/// FSCALE (single): Arm FPScale — `a` × 2^`scale` rounded once, the scale
/// clamped first (FPClampScale). A NaN is processed (FPProcessNaN), and a
/// zero or an infinity is its own result.
pub(crate) fn f32_scale(a: u32, scale: i64, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    if is_nan_f32(a) {
        return softfloat_propagate_nan_f32(a, a, s);
    }
    if f32_is_zero(a) || f32_is_inf(a) {
        return a;
    }
    let exp = i64::from(exp_f32(a));
    let scale = clamp_scale(scale, exp, 8, 23);
    // A denormal is `frac` × 2^(1 - bias), without the hidden bit.
    let (exp, sig) = if exp == 0 {
        (1, frac_f32(a))
    } else {
        (exp, frac_f32(a) | 0x0080_0000)
    };
    let result = norm_round_pack_to_f32(sign_f32(a), (exp - 1 + scale) as i16, sig << 7, s);
    f32_report_denormals(&[a], s);
    result
}

/// FSCALE (double); see [`f32_scale`].
pub(crate) fn f64_scale(a: u64, scale: i64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    if is_nan_f64(a) {
        return softfloat_propagate_nan_f64(a, a, s);
    }
    if f64_is_zero(a) || f64_is_inf(a) {
        return a;
    }
    let exp = i64::from(exp_f64(a));
    let scale = clamp_scale(scale, exp, 11, 52);
    let (exp, sig) = if exp == 0 {
        (1, frac_f64(a))
    } else {
        (exp, frac_f64(a) | 0x0010_0000_0000_0000)
    };
    let result = norm_round_pack_to_f64(sign_f64(a), (exp - 1 + scale) as i16, sig << 10, s);
    f64_report_denormals(&[a], s);
    result
}

/// FLOGB (single): Arm FPLogB — the base-2 exponent of `a`'s magnitude, a
/// 32-bit integer: the most negative integer for a NaN or a zero, raising
/// Invalid Operation, and the most positive for an infinity.
pub(crate) fn f32_log_b(a: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    if is_nan_f32(a) || f32_is_zero(a) {
        softfloat_raise_flags(s, FLAG_INVALID);
        return 0x8000_0000;
    }
    if f32_is_inf(a) {
        return 0x7FFF_FFFF;
    }
    f32_report_denormals(&[a], s);
    let exp = i32::from(exp_f32(a));
    let log = if exp == 0 {
        31 - frac_f32(a).leading_zeros() as i32 - 149
    } else {
        exp - 127
    };
    log as u32
}

/// FLOGB (double); see [`f32_log_b`].
pub(crate) fn f64_log_b(a: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    if is_nan_f64(a) || f64_is_zero(a) {
        softfloat_raise_flags(s, FLAG_INVALID);
        return 0x8000_0000_0000_0000;
    }
    if f64_is_inf(a) {
        return 0x7FFF_FFFF_FFFF_FFFF;
    }
    f64_report_denormals(&[a], s);
    let exp = i64::from(exp_f64(a));
    let log = if exp == 0 {
        63 - i64::from(frac_f64(a).leading_zeros()) - 1074
    } else {
        exp - 1023
    };
    log as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nearest() -> ArmFpStatus {
        ArmFpStatus::from_fpcr(0)
    }

    #[test]
    fn fixed_point_conversions_scale_by_the_fraction_bits() {
        let mut fp = nearest();
        // 3 with one fraction bit is 1.5; -1 with 64 fraction bits is -2^-64.
        assert_eq!(i64_to_f32_fixed(3, 1, FpRounding::TieEven, &mut fp), 0x3FC0_0000);
        assert_eq!(i64_to_f64_fixed(-1, 64, FpRounding::TieEven, &mut fp), 0xBBF0_0000_0000_0000);
        // 1.75 to fixed point with two fraction bits is 7.
        assert_eq!(f32_to_i32_fixed(0x3FE0_0000, 2, FpRounding::Zero, &mut fp), 7);
        assert_eq!(fp.cumulative_fpsr(), 0, "all of these are exact");
        // 2^64 - 1 needs 64 significant bits: it rounds to 2^64.
        assert_eq!(u64_to_f32_fixed(u64::MAX, 0, FpRounding::TieEven, &mut fp), 0x5F80_0000);
        assert_eq!(fp.cumulative_fpsr(), super::super::FPSR_IXC);
        // 2^31 with one fraction bit saturates a 32-bit integer, raising
        // Invalid Operation alone.
        let mut fp = nearest();
        assert_eq!(f32_to_i32_fixed(0x4F00_0000, 1, FpRounding::Zero, &mut fp), i32::MAX);
        assert_eq!(fp.cumulative_fpsr(), super::super::FPSR_IOC);
    }

    #[test]
    fn denormal_doubles_round_like_any_tiny_value() {
        let mut fp = nearest();
        // The smallest denormal with 64 fraction bits: tiny but nonzero, so
        // rounding toward +infinity gives 1 and toward zero gives 0.
        assert_eq!(f64_to_i64_fixed(1, 64, FpRounding::PosInf, &mut fp), 1);
        assert_eq!(f64_to_i64_fixed(1, 64, FpRounding::Zero, &mut fp), 0);
        assert_eq!(fp.cumulative_fpsr(), super::super::FPSR_IXC);
    }

    #[test]
    fn round_to_odd_sets_the_last_bit_when_inexact() {
        let mut fp = nearest();
        // 1 + 2^-30 is between two singles: toward zero gives 1.0, and the
        // odd bit marks the loss.
        assert_eq!(f64_to_f32_round_odd(0x3FF0_0000_0040_0000, &mut fp), 0x3F80_0001);
        assert_eq!(f64_to_f32_round_odd(0x3FF0_0000_0000_0000, &mut fp), 0x3F80_0000);
    }

    #[test]
    fn alternative_half_precision_has_no_infinity() {
        let mut fp = nearest();
        // 0x7C00 is 65536, and the largest value 131008 survives a round trip.
        assert_eq!(f16_ahp_to_f32(0x7C00), 0x4780_0000);
        assert_eq!(f32_to_f16_ahp(0x47FF_E000, &mut fp), 0x7FFF);
        assert_eq!(fp.cumulative_fpsr(), 0);
        // Infinity and NaN raise Invalid Operation.
        assert_eq!(f32_to_f16_ahp(0xFF80_0000, &mut fp), 0xFFFF);
        assert_eq!(f32_to_f16_ahp(0x7FC0_0000, &mut fp), 0x0000);
        assert_eq!(fp.cumulative_fpsr(), super::super::FPSR_IOC);
    }

    #[test]
    fn step_instructions_special_case_infinity_times_zero() {
        let mut fp = nearest();
        assert_eq!(f32_recip_step(0x7F80_0000, 0, &mut fp), F32_TWO);
        assert_eq!(f32_rsqrt_step(0, 0xFF80_0000, &mut fp), F32_ONE_POINT_FIVE);
        assert_eq!(f32_mulx(0x8000_0000, 0x7F80_0000, &mut fp), 0xC000_0000);
        // FRECPS(2, 0.5) = 2 - 1 = 1; FRSQRTS(1, 1) = (3 - 1) / 2 = 1.
        assert_eq!(f32_recip_step(0x4000_0000, 0x3F00_0000, &mut fp), 0x3F80_0000);
        assert_eq!(f32_rsqrt_step(0x3F80_0000, 0x3F80_0000, &mut fp), 0x3F80_0000);
        // FRSQRTS(2, 1) = (3 - 2) / 2 = 0.5.
        assert_eq!(f64_rsqrt_step(0x4000_0000_0000_0000, 0x3FF0_0000_0000_0000, &mut fp), 0x3FE0_0000_0000_0000);
        assert_eq!(fp.cumulative_fpsr(), 0);
    }
}
