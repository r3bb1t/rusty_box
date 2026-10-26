//! Half-precision arithmetic (FEAT_FP16), computed in double precision.
//!
//! Every half-precision value is a double-precision one exactly, and the
//! operations here are exact in double precision or round to the same
//! half-precision result as one rounding of the exact value would:
//!
//! - a sum, difference or product of two half-precision values has at most
//!   41 significant bits, so double precision holds it exactly; so does
//!   `a*b + c` whose result is within half precision's range (and one that
//!   is beyond it overflows in either computation);
//! - a quotient or square root rounded to double and then to half is the
//!   correctly rounded half result: the half-precision grid is part of the
//!   double one, so two directed roundings are one, and double precision
//!   has more than `2p + 2` bits for round to nearest (S. A. Figueroa,
//!   "When is double rounding innocuous?", SIGNUM 30(3), 1995);
//! - comparisons, FRINT*, minimum and maximum, and conversions to an
//!   integer work on the exact value.
//!
//! The double-precision operations apply Arm's NaN rules (FPProcessNaNs,
//! FPProcessNaNs3, the default NaN under FPCR.DN), which do not depend on
//! the precision. A half-precision NaN widens with its payload and its
//! quiet bit in place, so a signaling NaN stays signaling, and narrows back
//! (Arm FPConvertNaN) to the same half-precision NaN, quieted. A widened
//! value is never a double-precision denormal, so FPCR.FZ, which half
//! precision ignores, changes nothing here.
//!
//! FPCR.FZ16 is half precision's flush to zero (Arm FPUnpackBase and
//! FPRoundBase with `N == 16`): a denormal operand is a zero, with no
//! exception, and a result below the smallest normal before rounding is a
//! zero of its sign, raising underflow. FPCR.AHP does not apply: FPUnpack
//! and FPRound clear it.
//!
//! The conversions from an integer, which Arm defines as FixedToFP with
//! FPRound, round the exact value once here, since a 64-bit integer may
//! need more bits than double precision has.

use super::f64_to_f16;
use super::internals::{exp_f16, exp_f64, frac_f16, frac_f64, round_pack_to_f16, sign_f16, sign_f64};
use super::softfloat::{softfloat_raise_flags, FLAG_UNDERFLOW};
use super::{ArmFpStatus, FpOrdering, FpRounding, FusedMulAdd, InexactReport, NanSignal};

/// A binary interchange format wider than half precision, by the widths of
/// its exponent and fraction.
#[derive(Clone, Copy)]
struct Wider {
    exp_bits: u32,
    frac_bits: u32,
}

const DOUBLE: Wider = Wider { exp_bits: 11, frac_bits: 52 };
const SINGLE: Wider = Wider { exp_bits: 8, frac_bits: 23 };

/// `a` in the wider `format`: exact, a denormal normalized, a NaN's payload
/// and quiet bit kept in place.
fn widen(a: u16, format: Wider) -> u64 {
    let Wider { exp_bits, frac_bits } = format;
    let bias = (1u64 << (exp_bits - 1)) - 1;
    let sign = u64::from(sign_f16(a)) << (exp_bits + frac_bits);
    let exp = exp_f16(a) as u64;
    let frac = u64::from(frac_f16(a));
    if exp == 0x1F {
        return sign | (((1 << exp_bits) - 1) << frac_bits) | (frac << (frac_bits - 10));
    }
    if exp == 0 {
        if frac == 0 {
            return sign;
        }
        // A denormal: frac × 2^-24, normalized.
        let top = 63 - frac.leading_zeros();
        let normalized = (frac << (frac_bits - top)) & ((1 << frac_bits) - 1);
        return sign | ((bias - 24 + u64::from(top)) << frac_bits) | normalized;
    }
    sign | ((exp + bias - 15) << frac_bits) | (frac << (frac_bits - 10))
}

/// Arm FPUnpack's flush with `N == 16`: under FPCR.FZ16 a denormal operand
/// is a zero of its sign (no exception).
fn flushed(a: u16, fp: &ArmFpStatus) -> u16 {
    if fp.flush_half && exp_f16(a) == 0 {
        a & 0x8000
    } else {
        a
    }
}

/// Arm FPUnpack with `N == 16`: the operand, flushed, in double precision.
fn unpack(a: u16, fp: &ArmFpStatus) -> u64 {
    widen(flushed(a, fp), DOUBLE)
}

/// Arm FPRound with `N == 16` of the double-precision result `op` computes:
/// under FPCR.FZ16 a nonzero finite value below 2^-14 is a zero of its sign
/// and raises underflow alone (FPRoundBase flushes before it rounds, so the
/// inexact the double-precision operation may have raised is not); any
/// other is rounded by FPCR.RMode (a NaN converted by FPConvertNaN), with
/// the operation's exceptions.
fn rounded<F: FnOnce(&mut ArmFpStatus) -> u64>(fp: &mut ArmFpStatus, op: F) -> u16 {
    let before = fp.status.softfloat_exception_flags;
    let v = op(fp);
    let exp = exp_f64(v);
    let tiny = exp < 1023 - 14 && (exp != 0 || frac_f64(v) != 0);
    if fp.flush_half && tiny {
        fp.status.softfloat_exception_flags = before;
        softfloat_raise_flags(&mut fp.status, FLAG_UNDERFLOW);
        return (u16::from(sign_f64(v))) << 15;
    }
    f64_to_f16::f64_to_f16(v, &mut fp.status)
}

/// FADD (half). Arm FPAdd.
pub(crate) fn f16_add(a: u16, b: u16, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_add(unpack(a, fp), unpack(b, fp), fp))
}

/// FSUB (half). Arm FPSub.
pub(crate) fn f16_sub(a: u16, b: u16, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_sub(unpack(a, fp), unpack(b, fp), fp))
}

/// FMUL (half). Arm FPMul.
pub(crate) fn f16_mul(a: u16, b: u16, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_mul(unpack(a, fp), unpack(b, fp), fp))
}

/// FDIV (half). Arm FPDiv.
pub(crate) fn f16_div(a: u16, b: u16, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_div(unpack(a, fp), unpack(b, fp), fp))
}

/// FSQRT (half). Arm FPSqrt.
pub(crate) fn f16_sqrt(a: u16, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_sqrt(unpack(a, fp), fp))
}

/// FMADD/FMSUB/FNMADD/FNMSUB (half) and FMLA/FMLS, `addend` being Ra (or
/// Vd). Arm FPMulAdd after the form's operand negations.
pub(crate) fn f16_mul_add(form: FusedMulAdd, addend: u16, n: u16, m: u16, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_mul_add(form, unpack(addend, fp), unpack(n, fp), unpack(m, fp), fp))
}

/// FMAX (half). Arm FPMax.
pub(crate) fn f16_max(a: u16, b: u16, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_max(unpack(a, fp), unpack(b, fp), fp))
}

/// FMIN (half). Arm FPMin.
pub(crate) fn f16_min(a: u16, b: u16, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_min(unpack(a, fp), unpack(b, fp), fp))
}

/// FMAXNM (half). Arm FPMaxNum.
pub(crate) fn f16_maxnum(a: u16, b: u16, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_maxnum(unpack(a, fp), unpack(b, fp), fp))
}

/// FMINNM (half). Arm FPMinNum.
pub(crate) fn f16_minnum(a: u16, b: u16, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_minnum(unpack(a, fp), unpack(b, fp), fp))
}

/// FMULX (half). Arm FPMulX.
pub(crate) fn f16_mulx(a: u16, b: u16, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_mulx(unpack(a, fp), unpack(b, fp), fp))
}

/// FRECPS (half). Arm FPRecipStepFused: `2.0 - a*b` fused, exact in double
/// precision.
pub(crate) fn f16_recip_step(a: u16, b: u16, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_recip_step(unpack(a, fp), unpack(b, fp), fp))
}

/// FRSQRTS (half). Arm FPRSqrtStepFused: `(3.0 - a*b) / 2.0` fused, exact
/// in double precision.
pub(crate) fn f16_rsqrt_step(a: u16, b: u16, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_rsqrt_step(unpack(a, fp), unpack(b, fp), fp))
}

/// FCMP/FCMPE (half) and the ordering behind FCMEQ/FCMGE/FCMGT. Arm
/// FPCompare.
pub(crate) fn f16_compare(a: u16, b: u16, signal: NanSignal, fp: &mut ArmFpStatus) -> FpOrdering {
    super::f64_compare(unpack(a, fp), unpack(b, fp), signal, fp)
}

/// FRINT* (half). Arm FPRoundInt.
pub(crate) fn f16_round_to_int(a: u16, rounding: FpRounding, report: InexactReport, fp: &mut ArmFpStatus) -> u16 {
    rounded(fp, |fp| super::f64_round_to_int(unpack(a, fp), rounding, report, fp))
}

/// Half precision to a signed integer of 64 bits (`wide`) or 32, scaled by
/// 2^`fbits` first. Arm FPToFixed: NaN gives 0, overflow saturates, both
/// raise invalid; any other rounded result raises inexact. The result is
/// the integer's bits, zero-extended.
pub(crate) fn f16_to_signed(a: u16, fbits: u32, rounding: FpRounding, wide: bool, fp: &mut ArmFpStatus) -> u64 {
    let a = unpack(a, fp);
    match (wide, fbits) {
        (false, 0) => u64::from(super::f64_to_i32(a, rounding, fp) as u32),
        (true, 0) => super::f64_to_i64(a, rounding, fp) as u64,
        (false, _) => u64::from(super::f64_to_i32_fixed(a, fbits, rounding, fp) as u32),
        (true, _) => super::f64_to_i64_fixed(a, fbits, rounding, fp) as u64,
    }
}

/// Half precision to an unsigned integer of 64 bits (`wide`) or 32, scaled
/// by 2^`fbits` first; as [`f16_to_signed`], negative results saturating to
/// zero.
pub(crate) fn f16_to_unsigned(a: u16, fbits: u32, rounding: FpRounding, wide: bool, fp: &mut ArmFpStatus) -> u64 {
    let a = unpack(a, fp);
    match (wide, fbits) {
        (false, 0) => u64::from(super::f64_to_u32(a, rounding, fp)),
        (true, 0) => super::f64_to_u64(a, rounding, fp),
        (false, _) => u64::from(super::f64_to_u32_fixed(a, fbits, rounding, fp)),
        (true, _) => super::f64_to_u64_fixed(a, fbits, rounding, fp),
    }
}

/// An integer, `magnitude` with the sign `negative`, divided by 2^`fbits`
/// and rounded once to half precision by `rounding`. Arm FixedToFP with
/// `N == 16` (FPRound, so FPCR.FZ16 applies).
pub(crate) fn int_to_f16(
    magnitude: u64,
    negative: bool,
    fbits: u32,
    rounding: FpRounding,
    fp: &mut ArmFpStatus,
) -> u16 {
    if magnitude == 0 {
        return 0;
    }
    let top = 63 - magnitude.leading_zeros() as i32;
    // round_pack_to_f16 takes the significand with its leading bit at bit
    // 14 and four rounding bits below the half-precision fraction, and the
    // biased exponent less one: the value is 2^top × 1.f, scaled by
    // 2^-fbits.
    let exp = top + 14 - fbits as i32;
    let sig = if top >= 14 {
        let dropped = magnitude & ((1u64 << (top - 14)) - 1);
        (magnitude >> (top - 14)) as u16 | u16::from(dropped != 0)
    } else {
        (magnitude << (14 - top)) as u16
    };
    if fp.flush_half && exp < 0 {
        softfloat_raise_flags(&mut fp.status, FLAG_UNDERFLOW);
        return u16::from(negative) << 15;
    }
    // `top` <= 63 and `fbits` <= 64: the exponent is within -50..=77.
    let exp = exp as i16;
    super::with_rounding(fp, rounding, |s| round_pack_to_f16(negative, exp, sig, s))
}

/// FMLAL, FMLSL, FMLAL2, FMLSL2 (FEAT_FHM) on one element: Arm FPMulAddH,
/// `addend + n*m` in single precision with one rounding (FMLSL as the
/// [`FusedMulAdd::Fmsub`] form, `n` negated). The half-precision operands
/// are unpacked under FPCR.FZ16 and widened exactly, their product exact in
/// single precision; the addend is unpacked under FPCR.FZ and the result
/// rounded as single precision. NaNs are chosen as `FPProcessNaNs3H` does,
/// the addend first, a half-precision NaN converted (Arm FPConvertNaN).
pub(crate) fn f16_mul_add_long(form: FusedMulAdd, addend: u32, n: u16, m: u16, fp: &mut ArmFpStatus) -> u32 {
    let n = widen(flushed(n, fp), SINGLE) as u32;
    let m = widen(flushed(m, fp), SINGLE) as u32;
    super::f32_mul_add(form, addend, n, m, fp)
}
