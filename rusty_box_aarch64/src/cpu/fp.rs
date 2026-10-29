//! Scalar floating point (Arm ARM C3.6, C7.2), and the element operations
//! the Advanced SIMD floating-point instructions share with it.
//!
//! Values are handled as their register bits. FMOV, FABS, FNEG and FCSEL
//! only move or select bits: the architecture defines them to raise no
//! exception and to pass NaNs through unchanged but for FABS's and FNEG's
//! sign, which under FPCR.AH = 1 (FEAT_AFP) a NaN keeps too. Everything
//! else goes through `crate::softfloat` (the rusty_box softfloat3e port
//! with the Arm specialization) under an [`FpEnv`] built from FPCR, and the
//! exceptions an instruction raises are ORed into FPSR's cumulative bits
//! when it completes. Floating-point exceptions never trap on this core
//! (FPCR.{IOE,DZE,OFE,UFE,IXE,IDE} are RAZ/WI), so every instruction
//! completes.
//!
//! A scalar result zeroes the rest of its register, or under FPCR.NEP
//! (FEAT_AFP, Arm `IsMerging`) takes it from the source register the
//! instruction's page names ([`Cpu::fp_set_merging`]).

use super::exec::{bit, bits, sext, ExecResult, Flow};
use super::features::Feature;
use super::fp_estimate::{self, EstimateControl, EstimatePrecision, DOUBLE, HALF, SINGLE};
use super::simd::vfp_expand_imm;
use super::Cpu;
use crate::softfloat::{
    self as sf, ArmFpStatus, FpOrdering, FpRounding, FusedMulAdd, Handling, InexactReport,
    NanSignal,
};

/// FPCR.AHP: half-precision conversions use the alternative format.
const FPCR_AHP: u64 = 1 << 26;

/// Precision of an arithmetic operand. Half precision (FEAT_FP16) is an
/// arithmetic precision only on a processor that implements it: the
/// decoder hands a handler no half-precision encoding otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FpType {
    Half,
    Single,
    Double,
}

impl FpType {
    /// `ftype` 00 single, 01 double, 11 half; 10 is reserved.
    pub(crate) const fn from_ftype(ftype: u32) -> Option<Self> {
        match ftype {
            0 => Some(Self::Single),
            1 => Some(Self::Double),
            3 => Some(Self::Half),
            _ => None,
        }
    }

    /// The Advanced SIMD `sz` bit: 0 single, 1 double.
    pub(crate) const fn from_sz(sz: bool) -> Self {
        if sz {
            Self::Double
        } else {
            Self::Single
        }
    }

    pub(crate) const fn esize(self) -> u32 {
        match self {
            Self::Half => 16,
            Self::Single => 32,
            Self::Double => 64,
        }
    }

    pub(crate) const fn sign_bit(self) -> u64 {
        match self {
            Self::Half => 1 << 15,
            Self::Single => 1 << 31,
            Self::Double => 1 << 63,
        }
    }

    pub(crate) const fn mask(self) -> u64 {
        match self {
            Self::Half => 0xFFFF,
            Self::Single => 0xFFFF_FFFF,
            Self::Double => u64::MAX,
        }
    }

    /// The exponent field's bits.
    const fn exponent_mask(self) -> u64 {
        match self {
            Self::Half => 0x7C00,
            Self::Single => 0x7F80_0000,
            Self::Double => 0x7FF0_0000_0000_0000,
        }
    }

    /// Arm FPInfinity: the infinity of sign `negative`.
    pub(crate) const fn infinity(self, negative: bool) -> u64 {
        self.exponent_mask() | if negative { self.sign_bit() } else { 0 }
    }

    /// Arm FPOne('0').
    pub(crate) const fn one(self) -> u64 {
        match self {
            Self::Half => 0x3C00,
            Self::Single => 0x3F80_0000,
            Self::Double => 0x3FF0_0000_0000_0000,
        }
    }

    /// Arm FPTwo('0').
    pub(crate) const fn two(self) -> u64 {
        match self {
            Self::Half => 0x4000,
            Self::Single => 0x4000_0000,
            Self::Double => 0x4000_0000_0000_0000,
        }
    }

    /// Arm FPPointFive('0').
    pub(crate) const fn point_five(self) -> u64 {
        match self {
            Self::Half => 0x3800,
            Self::Single => 0x3F00_0000,
            Self::Double => 0x3FE0_0000_0000_0000,
        }
    }

    /// Whether `a` is a NaN of this precision.
    pub(crate) const fn is_nan(self, a: u64) -> bool {
        let exp = self.exponent_mask();
        a & exp == exp && a & self.mask() & !(exp | self.sign_bit()) != 0
    }

    /// Positive zero is all zero bits in every precision.
    pub(crate) const ZERO: u64 = 0;
}

/// The integer an FP-to-integer or integer-to-FP conversion writes or
/// reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntWidth {
    /// As wide as the floating-point element (the Advanced SIMD forms).
    OfElement,
    /// A W register.
    Bits32,
    /// An X register.
    Bits64,
}

impl IntWidth {
    /// The general-register width the `sf` bit selects.
    pub(crate) const fn from_sf(sf: bool) -> Self {
        if sf {
            Self::Bits64
        } else {
            Self::Bits32
        }
    }

    /// The integer's width in bits, for elements of `ty`.
    pub(crate) const fn bits(self, ty: FpType) -> u32 {
        match self {
            Self::OfElement => ty.esize(),
            Self::Bits32 => 32,
            Self::Bits64 => 64,
        }
    }
}

/// The integer range FRINT32* and FRINT64* round within (Arm FPRoundIntN's
/// `intsize`). Not an [`IntWidth`]: no integer is written or read, and
/// there is no element-sized range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntSize {
    Bits32,
    Bits64,
}

impl IntSize {
    const fn bits(self) -> u32 {
        match self {
            Self::Bits32 => 32,
            Self::Bits64 => 64,
        }
    }
}

/// A precision FEAT_FRINTTS rounds: Arm FPRoundIntN's `N`, 32 or 64.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrintPrecision {
    Single,
    Double,
}

impl FrintPrecision {
    const fn ty(self) -> FpType {
        match self {
            Self::Single => FpType::Single,
            Self::Double => FpType::Double,
        }
    }
}

/// Arm FPRoundIntN: `a` rounded to an integral value by `rounding`, raising
/// inexact when that changed it (FPRound of the integer). An infinity, a
/// NaN, or a value outside the signed range of `size` gives the most
/// negative integer of `size` and raises invalid alone.
pub(crate) fn round_int_n(
    precision: FrintPrecision,
    a: u64,
    rounding: FpRounding,
    size: IntSize,
    env: &mut FpEnv,
) -> u64 {
    let mut scratch = env.status;
    scratch.set_cumulative_fpsr(0);
    let (r, frac_bits) = match precision {
        FrintPrecision::Single => (
            u64::from(sf::f32_round_to_int(
                a as u32,
                rounding,
                InexactReport::Raise,
                &mut scratch,
            )),
            23,
        ),
        FrintPrecision::Double => (
            sf::f64_round_to_int(a, rounding, InexactReport::Raise, &mut scratch),
            52,
        ),
    };
    let ty = precision.ty();
    let exp_mask = ty.mask() >> (frac_bits + 1);
    let bias = exp_mask >> 1;
    let exp = (r >> frac_bits) & exp_mask;
    let limit_exp = bias + u64::from(size.bits()) - 1;
    let most_negative = ty.sign_bit() | (limit_exp << frac_bits);
    // An integral value at least 2^(size - 1) in magnitude, an infinity or
    // a NaN is outside the range, -2^(size - 1) itself apart.
    if exp >= limit_exp && r != most_negative {
        env.raise(fp_estimate::fpsr::IOC);
        return most_negative;
    }
    env.raise(scratch.cumulative_fpsr());
    r
}

/// A format FCVT converts between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Precision {
    Half,
    Single,
    Double,
}

impl Precision {
    /// The `ftype`/`opc` encoding: 00 single, 01 double, 11 half.
    const fn from_field(field: u32) -> Option<Self> {
        match field {
            0 => Some(Self::Single),
            1 => Some(Self::Double),
            3 => Some(Self::Half),
            _ => None,
        }
    }

    pub(crate) const fn mask(self) -> u64 {
        match self {
            Self::Half => 0xFFFF,
            Self::Single => 0xFFFF_FFFF,
            Self::Double => u64::MAX,
        }
    }
}

/// One FCVT: a conversion between two different precisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FpConversion {
    HalfToSingle,
    HalfToDouble,
    SingleToHalf,
    SingleToDouble,
    DoubleToHalf,
    DoubleToSingle,
}

impl FpConversion {
    pub(crate) const fn between(from: Precision, to: Precision) -> Option<Self> {
        match (from, to) {
            (Precision::Half, Precision::Single) => Some(Self::HalfToSingle),
            (Precision::Half, Precision::Double) => Some(Self::HalfToDouble),
            (Precision::Single, Precision::Half) => Some(Self::SingleToHalf),
            (Precision::Single, Precision::Double) => Some(Self::SingleToDouble),
            (Precision::Double, Precision::Half) => Some(Self::DoubleToHalf),
            (Precision::Double, Precision::Single) => Some(Self::DoubleToSingle),
            _ => None,
        }
    }
}

/// The floating-point environment of one instruction: the FPCR controls and
/// the exceptions raised so far.
pub(crate) struct FpEnv {
    pub(crate) status: ArmFpStatus,
    fpcr: u64,
    /// The processor's single-precision FRECPE and FRSQRTE estimates.
    estimates: EstimatePrecision,
}

impl FpEnv {
    fn ahp(&self) -> bool {
        self.fpcr & FPCR_AHP != 0
    }

    /// Arm FPDefaultNaN: the quiet NaN with no payload, negative under
    /// FPCR.AH = 1.
    pub(crate) fn default_nan(&self, ty: FpType) -> u64 {
        match ty {
            FpType::Half => u64::from(self.status.default_nan_f16()),
            FpType::Single => u64::from(self.status.default_nan_f32()),
            FpType::Double => self.status.default_nan_f64(),
        }
    }

    /// The environment Arm FPConvertSVE converts in: FPCR.AHP read as 0,
    /// half precision always IEEE.
    pub(crate) fn with_ieee_half(mut self) -> Self {
        self.fpcr &= !FPCR_AHP;
        self
    }

    /// The environment the SME instructions that accumulate into ZA
    /// compute in (Arm `FPMulAdd_ZA`, `FPDotAdd_ZA`): FPCR.DN read as 1.
    pub(crate) fn with_default_nan(mut self) -> Self {
        self.status = self.status.with_default_nan();
        self
    }

    /// FPCR.RMode, for the operations that round by it.
    pub(crate) fn rounding(&self) -> FpRounding {
        self.status.rounding()
    }

    /// FPCR.AH.
    pub(crate) fn handling(&self) -> Handling {
        self.status.handling()
    }

    /// Raise FPSR cumulative exception bits computed outside softfloat.
    fn raise(&mut self, fpsr_bits: u64) {
        let raised = self.status.cumulative_fpsr();
        self.status.set_cumulative_fpsr(raised | fpsr_bits);
    }

    /// Arm FPNeg: `a` with its sign flipped — except, under FPCR.AH = 1, a
    /// NaN, whose sign has no consequence.
    pub(crate) fn neg(&self, ty: FpType, a: u64) -> u64 {
        if self.handling() == Handling::Alternate && ty.is_nan(a) {
            a
        } else {
            (a ^ ty.sign_bit()) & ty.mask()
        }
    }

    /// Arm FPAbs: `a` with its sign cleared — except, under FPCR.AH = 1, a
    /// NaN.
    pub(crate) fn abs(&self, ty: FpType, a: u64) -> u64 {
        if self.handling() == Handling::Alternate && ty.is_nan(a) {
            a
        } else {
            a & !ty.sign_bit() & ty.mask()
        }
    }
}

/// Two-operand element operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FpBinary {
    Add,
    Sub,
    Mul,
    Div,
    Max,
    Min,
    MaxNum,
    MinNum,
    /// FMULX: FMUL, with infinity times zero giving ±2.0.
    MulX,
    /// FRECPS.
    RecipStep,
    /// FRSQRTS.
    RSqrtStep,
    /// FABD: the absolute value of the difference (FPAbs of FPSub).
    AbsDiff,
    /// FNMUL: the negated product (FPNeg of FPMul).
    NegMul,
}

/// One-operand element operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FpUnary {
    Abs,
    Neg,
    Sqrt,
    /// FRINT* with its rounding and whether it reports inexact (FRINTX).
    RoundInt(FpRounding, InexactReport),
    /// FRECPE.
    RecipEstimate,
    /// FRSQRTE.
    RSqrtEstimate,
    /// FRECPX.
    RecpX,
}

/// The compare-to-mask operations (FCMEQ, FCMGE, FCMGT, FACGE, FACGT and the
/// compare-with-zero forms, whose operands are ordered by the caller).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FpCompare {
    Eq,
    Ge,
    Gt,
    /// `|a| >= |b|`.
    AbsGe,
    /// `|a| > |b|`.
    AbsGt,
}

/// `op(a, b)` on one element.
pub(crate) fn fp_binary(op: FpBinary, ty: FpType, a: u64, b: u64, env: &mut FpEnv) -> u64 {
    let fp = &mut env.status;
    // FABD's difference and FNMUL's product; FPAbs and FPNeg follow.
    let result = match ty {
        FpType::Half => {
            let (a, b) = (a as u16, b as u16);
            u64::from(match op {
                FpBinary::Add => sf::f16_add(a, b, fp),
                FpBinary::Sub | FpBinary::AbsDiff => sf::f16_sub(a, b, fp),
                FpBinary::Mul | FpBinary::NegMul => sf::f16_mul(a, b, fp),
                FpBinary::Div => sf::f16_div(a, b, fp),
                FpBinary::Max => sf::f16_max(a, b, fp),
                FpBinary::Min => sf::f16_min(a, b, fp),
                FpBinary::MaxNum => sf::f16_maxnum(a, b, fp),
                FpBinary::MinNum => sf::f16_minnum(a, b, fp),
                FpBinary::MulX => sf::f16_mulx(a, b, fp),
                FpBinary::RecipStep => sf::f16_recip_step(a, b, fp),
                FpBinary::RSqrtStep => sf::f16_rsqrt_step(a, b, fp),
            })
        }
        FpType::Single => {
            let (a, b) = (a as u32, b as u32);
            u64::from(match op {
                FpBinary::Add => sf::f32_add(a, b, fp),
                FpBinary::Sub | FpBinary::AbsDiff => sf::f32_sub(a, b, fp),
                FpBinary::Mul | FpBinary::NegMul => sf::f32_mul(a, b, fp),
                FpBinary::Div => sf::f32_div(a, b, fp),
                FpBinary::Max => sf::f32_max(a, b, fp),
                FpBinary::Min => sf::f32_min(a, b, fp),
                FpBinary::MaxNum => sf::f32_maxnum(a, b, fp),
                FpBinary::MinNum => sf::f32_minnum(a, b, fp),
                FpBinary::MulX => sf::f32_mulx(a, b, fp),
                FpBinary::RecipStep => sf::f32_recip_step(a, b, fp),
                FpBinary::RSqrtStep => sf::f32_rsqrt_step(a, b, fp),
            })
        }
        FpType::Double => match op {
            FpBinary::Add => sf::f64_add(a, b, fp),
            FpBinary::Sub | FpBinary::AbsDiff => sf::f64_sub(a, b, fp),
            FpBinary::Mul | FpBinary::NegMul => sf::f64_mul(a, b, fp),
            FpBinary::Div => sf::f64_div(a, b, fp),
            FpBinary::Max => sf::f64_max(a, b, fp),
            FpBinary::Min => sf::f64_min(a, b, fp),
            FpBinary::MaxNum => sf::f64_maxnum(a, b, fp),
            FpBinary::MinNum => sf::f64_minnum(a, b, fp),
            FpBinary::MulX => sf::f64_mulx(a, b, fp),
            FpBinary::RecipStep => sf::f64_recip_step(a, b, fp),
            FpBinary::RSqrtStep => sf::f64_rsqrt_step(a, b, fp),
        },
    };
    match op {
        FpBinary::AbsDiff => env.abs(ty, result),
        FpBinary::NegMul => env.neg(ty, result),
        FpBinary::Add
        | FpBinary::Sub
        | FpBinary::Mul
        | FpBinary::Div
        | FpBinary::Max
        | FpBinary::Min
        | FpBinary::MaxNum
        | FpBinary::MinNum
        | FpBinary::MulX
        | FpBinary::RecipStep
        | FpBinary::RSqrtStep => result,
    }
}

/// `op(a)` on one element.
pub(crate) fn fp_unary(op: FpUnary, ty: FpType, a: u64, env: &mut FpEnv) -> u64 {
    let format = match ty {
        FpType::Half => HALF,
        FpType::Single => SINGLE,
        FpType::Double => DOUBLE,
    };
    let estimate = |e: fp_estimate::Estimate, env: &mut FpEnv| {
        env.raise(e.fpsr);
        e.bits
    };
    let control = EstimateControl::from_fpcr(env.fpcr, env.estimates);
    match op {
        FpUnary::Abs => env.abs(ty, a),
        FpUnary::Neg => env.neg(ty, a),
        FpUnary::Sqrt => match ty {
            FpType::Half => u64::from(sf::f16_sqrt(a as u16, &mut env.status)),
            FpType::Single => u64::from(sf::f32_sqrt(a as u32, &mut env.status)),
            FpType::Double => sf::f64_sqrt(a, &mut env.status),
        },
        FpUnary::RoundInt(rounding, report) => match ty {
            FpType::Half => u64::from(sf::f16_round_to_int(
                a as u16,
                rounding,
                report,
                &mut env.status,
            )),
            FpType::Single => u64::from(sf::f32_round_to_int(
                a as u32,
                rounding,
                report,
                &mut env.status,
            )),
            FpType::Double => sf::f64_round_to_int(a, rounding, report, &mut env.status),
        },
        FpUnary::RecipEstimate => estimate(fp_estimate::frecpe(format, a, control), env),
        FpUnary::RSqrtEstimate => estimate(fp_estimate::frsqrte(format, a, control), env),
        FpUnary::RecpX => estimate(fp_estimate::frecpx(format, a, control), env),
    }
}

/// Arm FPMulAdd in one of the four A64 forms; `addend` is Ra (or Vd).
pub(crate) fn fp_mul_add(
    form: FusedMulAdd,
    ty: FpType,
    addend: u64,
    n: u64,
    m: u64,
    env: &mut FpEnv,
) -> u64 {
    match ty {
        FpType::Half => u64::from(sf::f16_mul_add(
            form,
            addend as u16,
            n as u16,
            m as u16,
            &mut env.status,
        )),
        FpType::Single => u64::from(sf::f32_mul_add(
            form,
            addend as u32,
            n as u32,
            m as u32,
            &mut env.status,
        )),
        FpType::Double => sf::f64_mul_add(form, addend, n, m, &mut env.status),
    }
}

/// Arm FPCompare.
pub(crate) fn fp_compare(
    ty: FpType,
    a: u64,
    b: u64,
    signal: NanSignal,
    env: &mut FpEnv,
) -> FpOrdering {
    match ty {
        FpType::Half => sf::f16_compare(a as u16, b as u16, signal, &mut env.status),
        FpType::Single => sf::f32_compare(a as u32, b as u32, signal, &mut env.status),
        FpType::Double => sf::f64_compare(a, b, signal, &mut env.status),
    }
}

/// FPCompareEQ/GE/GT (and the absolute forms): whether `a op b` holds.
pub(crate) fn fp_compare_holds(op: FpCompare, ty: FpType, a: u64, b: u64, env: &mut FpEnv) -> bool {
    let (a, b) = match op {
        FpCompare::AbsGe | FpCompare::AbsGt => (env.abs(ty, a), env.abs(ty, b)),
        FpCompare::Eq | FpCompare::Ge | FpCompare::Gt => (a, b),
    };
    match op {
        FpCompare::Eq => fp_compare(ty, a, b, NanSignal::Quiet, env) == FpOrdering::Equal,
        FpCompare::Ge | FpCompare::AbsGe => matches!(
            fp_compare(ty, a, b, NanSignal::Signaling, env),
            FpOrdering::Greater | FpOrdering::Equal
        ),
        FpCompare::Gt | FpCompare::AbsGt => {
            fp_compare(ty, a, b, NanSignal::Signaling, env) == FpOrdering::Greater
        }
    }
}

/// Arm FPToFixed: `a` × 2^`fbits` rounded to a signed or unsigned integer
/// of `width`, saturating; the result is zero-extended.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fp_to_int(
    ty: FpType,
    a: u64,
    fbits: u32,
    rounding: FpRounding,
    signed: bool,
    width: IntWidth,
    env: &mut FpEnv,
) -> u64 {
    let wide = width.bits(ty) == 64;
    if ty == FpType::Half && width.bits(ty) == 16 {
        return half_to_int16(a as u16, fbits, rounding, signed, env);
    }
    let fp = &mut env.status;
    match (ty, signed, wide, fbits) {
        (FpType::Half, true, _, _) => sf::f16_to_signed(a as u16, fbits, rounding, wide, fp),
        (FpType::Half, false, _, _) => sf::f16_to_unsigned(a as u16, fbits, rounding, wide, fp),
        (FpType::Single, true, false, 0) => {
            u64::from(sf::f32_to_i32(a as u32, rounding, fp) as u32)
        }
        (FpType::Single, true, true, 0) => sf::f32_to_i64(a as u32, rounding, fp) as u64,
        (FpType::Single, false, false, 0) => u64::from(sf::f32_to_u32(a as u32, rounding, fp)),
        (FpType::Single, false, true, 0) => sf::f32_to_u64(a as u32, rounding, fp),
        (FpType::Double, true, false, 0) => u64::from(sf::f64_to_i32(a, rounding, fp) as u32),
        (FpType::Double, true, true, 0) => sf::f64_to_i64(a, rounding, fp) as u64,
        (FpType::Double, false, false, 0) => u64::from(sf::f64_to_u32(a, rounding, fp)),
        (FpType::Double, false, true, 0) => sf::f64_to_u64(a, rounding, fp),
        (FpType::Single, true, false, _) => {
            u64::from(sf::f32_to_i32_fixed(a as u32, fbits, rounding, fp) as u32)
        }
        (FpType::Single, true, true, _) => {
            sf::f32_to_i64_fixed(a as u32, fbits, rounding, fp) as u64
        }
        (FpType::Single, false, false, _) => {
            u64::from(sf::f32_to_u32_fixed(a as u32, fbits, rounding, fp))
        }
        (FpType::Single, false, true, _) => sf::f32_to_u64_fixed(a as u32, fbits, rounding, fp),
        (FpType::Double, true, false, _) => {
            u64::from(sf::f64_to_i32_fixed(a, fbits, rounding, fp) as u32)
        }
        (FpType::Double, true, true, _) => sf::f64_to_i64_fixed(a, fbits, rounding, fp) as u64,
        (FpType::Double, false, false, _) => {
            u64::from(sf::f64_to_u32_fixed(a, fbits, rounding, fp))
        }
        (FpType::Double, false, true, _) => sf::f64_to_u64_fixed(a, fbits, rounding, fp),
    }
}

/// Arm FPToFixed with a 16-bit result, for half-precision elements: the
/// value rounded to an integer, which fits 64 bits whatever it is, then
/// saturated to 16 bits. A saturated result raises Invalid Operation alone,
/// without the Inexact the rounding may have raised (`SatQ`, then
/// `elsif error != 0.0`).
fn half_to_int16(a: u16, fbits: u32, rounding: FpRounding, signed: bool, env: &mut FpEnv) -> u64 {
    let mut scratch = env.status;
    scratch.set_cumulative_fpsr(0);
    let (result, saturated) = if signed {
        let wide = sf::f16_to_signed(a, fbits, rounding, true, &mut scratch) as i64;
        let narrow = wide.clamp(-0x8000, 0x7FFF);
        (narrow as u64, narrow != wide)
    } else {
        let wide = sf::f16_to_unsigned(a, fbits, rounding, true, &mut scratch);
        (wide.min(0xFFFF), wide > 0xFFFF)
    };
    if saturated {
        env.raise(fp_estimate::fpsr::IOC);
    } else {
        env.raise(scratch.cumulative_fpsr());
    }
    result & 0xFFFF
}

/// Arm FixedToFP: a signed or unsigned integer of `width` (the low bits of
/// `value`), divided by 2^`fbits`, rounded by FPCR.RMode.
pub(crate) fn int_to_fp(
    ty: FpType,
    value: u64,
    fbits: u32,
    signed: bool,
    width: IntWidth,
    env: &mut FpEnv,
) -> u64 {
    let rounding = env.rounding();
    let bits = width.bits(ty);
    let wide = bits == 64;
    let fp = &mut env.status;
    match (ty, signed, wide, fbits) {
        (FpType::Half, _, _, _) => {
            let v = value & (u64::MAX >> (64 - bits));
            let negative = signed && v >> (bits - 1) == 1;
            // The magnitude of a negative `bits`-bit integer.
            let magnitude = if negative {
                v.wrapping_neg() & (u64::MAX >> (64 - bits))
            } else {
                v
            };
            u64::from(sf::int_to_f16(magnitude, negative, fbits, rounding, fp))
        }
        (FpType::Single, true, false, 0) => u64::from(sf::i32_to_f32(value as i32, rounding, fp)),
        (FpType::Single, true, true, 0) => u64::from(sf::i64_to_f32(value as i64, rounding, fp)),
        (FpType::Single, false, false, 0) => u64::from(sf::u32_to_f32(value as u32, rounding, fp)),
        (FpType::Single, false, true, 0) => u64::from(sf::u64_to_f32(value, rounding, fp)),
        (FpType::Double, true, false, 0) => sf::i32_to_f64(value as i32),
        (FpType::Double, true, true, 0) => sf::i64_to_f64(value as i64, rounding, fp),
        (FpType::Double, false, false, 0) => sf::u32_to_f64(value as u32),
        (FpType::Double, false, true, 0) => sf::u64_to_f64(value, rounding, fp),
        (FpType::Single | FpType::Double, true, _, _) => {
            let v = if wide {
                value as i64
            } else {
                sext(value, 32) as i64
            };
            if ty == FpType::Single {
                u64::from(sf::i64_to_f32_fixed(v, fbits, rounding, fp))
            } else {
                sf::i64_to_f64_fixed(v, fbits, rounding, fp)
            }
        }
        (FpType::Single | FpType::Double, false, _, _) => {
            let v = if wide { value } else { value & 0xFFFF_FFFF };
            if ty == FpType::Single {
                u64::from(sf::u64_to_f32_fixed(v, fbits, rounding, fp))
            } else {
                sf::u64_to_f64_fixed(v, fbits, rounding, fp)
            }
        }
    }
}

/// Arm FPConvert between precisions, honouring FPCR.AHP for half precision.
pub(crate) fn fp_convert(conversion: FpConversion, a: u64, env: &mut FpEnv) -> u64 {
    let ahp = env.ahp();
    let fp = &mut env.status;
    match conversion {
        FpConversion::SingleToDouble => sf::f32_to_f64(a as u32, fp),
        FpConversion::DoubleToSingle => u64::from(sf::f64_to_f32(a, fp)),
        FpConversion::HalfToSingle if ahp => u64::from(sf::f16_ahp_to_f32(a as u16)),
        FpConversion::HalfToSingle => u64::from(sf::f16_to_f32(a as u16, fp)),
        FpConversion::HalfToDouble if ahp => sf::f16_ahp_to_f64(a as u16),
        FpConversion::HalfToDouble => sf::f16_to_f64(a as u16, fp),
        FpConversion::SingleToHalf if ahp => u64::from(sf::f32_to_f16_ahp(a as u32, fp)),
        FpConversion::SingleToHalf => u64::from(sf::f32_to_f16(a as u32, fp)),
        FpConversion::DoubleToHalf if ahp => u64::from(sf::f64_to_f16_ahp(a, fp)),
        FpConversion::DoubleToHalf => u64::from(sf::f64_to_f16(a, fp)),
    }
}

/// FCVTXN on one element: double to single, rounding to odd.
pub(crate) fn fp_convert_round_odd(a: u64, env: &mut FpEnv) -> u64 {
    u64::from(sf::f64_to_f32_round_odd(a, &mut env.status))
}

/// Arm BFMulAddH: `addend` plus the product of the BFloat16 values `n`
/// and `m`, widened to single precision, in one fused multiply-add under
/// FPCR — under FPCR.AH = 1 with every denormal flushed, rounding to
/// nearest and no exception.
pub(crate) fn bf16_mul_add_long(addend: u32, n: u16, m: u16, env: &mut FpEnv) -> u32 {
    let (n, m) = (u32::from(n) << 16, u32::from(m) << 16);
    env.status
        .with_altfp_overrides(|fp| sf::f32_mul_add(FusedMulAdd::Fmadd, addend, n, m, fp))
}

/// Arm BFNeg: the BFloat16 value `a` with its sign inverted — except,
/// under FPCR.AH = 1, a NaN. A BFloat16 value is the high half of a
/// single-precision one, so FPNeg of that is BFNeg of this.
pub(crate) fn bf16_neg(a: u16, env: &FpEnv) -> u16 {
    (env.neg(FpType::Single, u64::from(a) << 16) >> 16) as u16
}

/// Arm FPScale: `a` × 2^`scale`, rounded once.
pub(crate) fn fp_scale(ty: FpType, a: u64, scale: i64, env: &mut FpEnv) -> u64 {
    let fp = &mut env.status;
    match ty {
        FpType::Half => u64::from(sf::f16_scale(a as u16, scale, fp)),
        FpType::Single => u64::from(sf::f32_scale(a as u32, scale, fp)),
        FpType::Double => sf::f64_scale(a, scale, fp),
    }
}

/// Arm FPLogB: the base-2 exponent of `a`'s magnitude, an integer of the
/// element's width.
pub(crate) fn fp_log_b(ty: FpType, a: u64, env: &mut FpEnv) -> u64 {
    let fp = &mut env.status;
    match ty {
        FpType::Half => u64::from(sf::f16_log_b(a as u16, fp)),
        FpType::Single => u64::from(sf::f32_log_b(a as u32, fp)),
        FpType::Double => sf::f64_log_b(a, fp),
    }
}

/// M (bit 31) or S (bit 29) set: unallocated in every scalar FP encoding
/// that is not an integer conversion (where bit 31 is `sf`).
#[inline]
const fn m_or_s(insn: u32) -> bool {
    bit(insn, 31) || bit(insn, 29)
}

/// The precision of a scalar FP data-processing word: `None` when M or S
/// is set or `ftype` is the reserved `10`. Half precision (`ftype` 11) is
/// [`FpType::Half`]; the decoder executes it only with FEAT_FP16.
pub(crate) const fn scalar_type(insn: u32) -> Option<FpType> {
    if m_or_s(insn) {
        return None;
    }
    FpType::from_ftype(bits(insn, 23, 22))
}

/// The operation of an FP data-processing (2 source) word, from its
/// `opcode`: `None` for the unallocated opcodes.
pub(crate) const fn dp2_operation(insn: u32) -> Option<FpBinary> {
    Some(match bits(insn, 15, 12) {
        0b0000 => FpBinary::Mul,
        0b0001 => FpBinary::Div,
        0b0010 => FpBinary::Add,
        0b0011 => FpBinary::Sub,
        0b0100 => FpBinary::Max,
        0b0101 => FpBinary::Min,
        0b0110 => FpBinary::MaxNum,
        0b0111 => FpBinary::MinNum,
        0b1000 => FpBinary::NegMul,
        _ => return None,
    })
}

/// The form of an FP data-processing (3 source) word, from `o1` and `o0`.
pub(crate) const fn dp3_form(insn: u32) -> FusedMulAdd {
    match (bit(insn, 21), bit(insn, 15)) {
        (false, false) => FusedMulAdd::Fmadd,
        (false, true) => FusedMulAdd::Fmsub,
        (true, false) => FusedMulAdd::Fnmadd,
        (true, true) => FusedMulAdd::Fnmsub,
    }
}

/// The registers of an FP data-processing (3 source) instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FpDp3Regs {
    /// Rd, the result.
    pub(crate) d: u32,
    /// Rn, the multiplicand.
    pub(crate) n: u32,
    /// Rm, the multiplier.
    pub(crate) m: u32,
    /// Ra, the addend.
    pub(crate) a: u32,
}

/// The rounding an FCVT{N,P,M,Z}* `rmode` field selects.
pub(crate) const fn rounding_from_rmode(rmode: u32) -> FpRounding {
    match rmode {
        0 => FpRounding::TieEven,
        1 => FpRounding::PosInf,
        2 => FpRounding::NegInf,
        _ => FpRounding::Zero,
    }
}

impl Cpu {
    /// The floating-point environment for an instruction about to execute:
    /// the FPCR controls, and the exceptions FPSR has accumulated so far.
    #[inline]
    pub(crate) fn fp_env(&self) -> FpEnv {
        let estimates = if self.implements(Feature::Rpres) {
            EstimatePrecision::Increased
        } else {
            EstimatePrecision::Base
        };
        FpEnv {
            status: ArmFpStatus::with_fpsr(self.fpcr, self.fpsr),
            fpcr: self.fpcr,
            estimates,
        }
    }

    /// FPCR.AH, which the operations that read nothing else of FPCR (BFDOT,
    /// BFMMLA) take without a whole [`FpEnv`].
    #[inline]
    pub(crate) fn fp_handling(&self) -> Handling {
        if self.fpcr & sf::FPCR_AH != 0 {
            Handling::Alternate
        } else {
            Handling::Standard
        }
    }

    /// Arm `IsMerging(FPCR)`: whether a scalar result's upper elements come
    /// from a source register (FPCR.NEP, FEAT_AFP) instead of being zeroed.
    /// In Streaming SVE mode NEP reads as 0 unless the full A64 instruction
    /// set is enabled there (`IsFullA64Enabled`), which takes
    /// FEAT_SME_FA64, which this processor lacks.
    #[inline]
    pub(crate) fn fp_merging(&self) -> bool {
        self.fpcr & sf::FPCR_NEP != 0 && !self.svcr.sm
    }

    /// Write a scalar result to `Vd`: `result = if IsMerging(FPCR()) then
    /// V(merge) else Zeros` with the low `mask` bits replaced, `merge` being
    /// the source register the instruction's page names.
    #[inline]
    pub(crate) fn fp_set_merging(&mut self, d: u32, merge: u32, mask: u64, value: u64) {
        let upper = if self.fp_merging() {
            self.v[merge as usize] & !u128::from(mask)
        } else {
            0
        };
        self.write_v(d as usize, upper | u128::from(value & mask));
    }

    /// Accumulate the exceptions an instruction raised into FPSR.
    #[inline]
    pub(crate) fn fp_commit(&mut self, env: &FpEnv) {
        self.fpsr |= env.status.cumulative_fpsr();
    }

    /// Read the low bits of a SIMD&FP register as a scalar.
    #[inline]
    pub(crate) fn fp_get(&self, n: u32, mask: u64) -> u64 {
        (self.v[n as usize] as u64) & mask
    }

    /// Write a scalar result: the rest of the register is cleared.
    #[inline]
    pub(crate) fn fp_set(&mut self, d: u32, mask: u64, value: u64) {
        self.write_v(d as usize, u128::from(value & mask));
    }

    /// FMOV (scalar, immediate): `000 11110 ftype 1 imm8 100 imm5 Rd`.
    pub(crate) fn exec_fmov_imm(&mut self, pc: u64, insn: u32) -> ExecResult {
        if m_or_s(insn) || bits(insn, 9, 5) != 0 {
            return self.undefined(pc, insn);
        }
        let Some(ty) = FpType::from_ftype(bits(insn, 23, 22)) else {
            return self.undefined(pc, insn);
        };
        self.fp_check()?;
        let imm8 = u64::from(bits(insn, 20, 13));
        self.fp_set(bits(insn, 4, 0), ty.mask(), vfp_expand_imm(imm8, ty));
        Ok(Flow::Next)
    }

    /// FCSEL: `000 11110 ftype 1 Rm cond 11 Rn Rd`.
    pub(crate) fn exec_fcsel(&mut self, pc: u64, insn: u32) -> ExecResult {
        let Some(ty) = FpType::from_ftype(bits(insn, 23, 22)).filter(|_| !m_or_s(insn)) else {
            return self.undefined(pc, insn);
        };
        self.fp_check()?;
        let src = if self.condition_holds(bits(insn, 15, 12)) {
            bits(insn, 9, 5)
        } else {
            bits(insn, 20, 16)
        };
        let value = self.fp_get(src, ty.mask());
        self.fp_set(bits(insn, 4, 0), ty.mask(), value);
        Ok(Flow::Next)
    }

    /// One source: `000 11110 ftype 1 opcode 10000 Rn Rd` — FMOV, FABS,
    /// FNEG, FSQRT, FCVT, BFCVT and FRINT*.
    pub(crate) fn exec_fp_dp1(&mut self, pc: u64, insn: u32) -> ExecResult {
        if m_or_s(insn) {
            return self.undefined(pc, insn);
        }
        let ftype = bits(insn, 23, 22);
        let opcode = bits(insn, 20, 15);
        let n = bits(insn, 9, 5);
        let d = bits(insn, 4, 0);

        if opcode >> 2 == 0b0001 {
            if (ftype, opcode & 3) == (0b01, 0b10) {
                // BFCVT (FEAT_BF16, which the decoder checks), in the space
                // of FCVT's reserved destination: single precision to
                // BFloat16.
                self.fp_check()?;
                let mut env = self.fp_env();
                let rounding = env.rounding();
                let a = self.fp_get(n, FpType::Single.mask());
                let result = sf::f32_to_bf16(a as u32, rounding, &mut env.status);
                self.fp_commit(&env);
                self.fp_set_merging(d, d, FpType::Half.mask(), u64::from(result));
                return Ok(Flow::Next);
            }
            // FCVT: ftype is the source precision, opc (bits 16:15) the
            // destination's.
            let conversion = match (
                Precision::from_field(ftype),
                Precision::from_field(opcode & 3),
            ) {
                (Some(from), Some(to)) => FpConversion::between(from, to),
                _ => None,
            };
            let Some(conversion) = conversion else {
                return self.undefined(pc, insn);
            };
            self.fp_check()?;
            let (from_mask, to_mask) = match conversion {
                FpConversion::HalfToSingle => (Precision::Half.mask(), Precision::Single.mask()),
                FpConversion::HalfToDouble => (Precision::Half.mask(), Precision::Double.mask()),
                FpConversion::SingleToHalf => (Precision::Single.mask(), Precision::Half.mask()),
                FpConversion::SingleToDouble => {
                    (Precision::Single.mask(), Precision::Double.mask())
                }
                FpConversion::DoubleToHalf => (Precision::Double.mask(), Precision::Half.mask()),
                FpConversion::DoubleToSingle => {
                    (Precision::Double.mask(), Precision::Single.mask())
                }
            };
            let mut env = self.fp_env();
            let result = fp_convert(conversion, self.fp_get(n, from_mask), &mut env);
            self.fp_commit(&env);
            self.fp_set_merging(d, d, to_mask, result);
            return Ok(Flow::Next);
        }
        if opcode >> 2 == 0b0100 {
            // FRINT32Z, FRINT32X, FRINT64Z, FRINT64X (FEAT_FRINTTS, which
            // the decoder checks): `if ftype IN {'1x'} then
            // EndOfDecode(Decode_UNDEF)`. `op<0>` rounds by FPCR.RMode
            // rather than toward zero, `op<1>` within 64 bits rather than 32.
            let precision = match ftype {
                0b00 => FrintPrecision::Single,
                0b01 => FrintPrecision::Double,
                _ => return self.undefined(pc, insn),
            };
            self.fp_check()?;
            let mut env = self.fp_env();
            let rounding = if opcode & 1 == 1 {
                env.rounding()
            } else {
                FpRounding::Zero
            };
            let size = if opcode & 2 == 2 {
                IntSize::Bits64
            } else {
                IntSize::Bits32
            };
            let mask = precision.ty().mask();
            let result = round_int_n(precision, self.fp_get(n, mask), rounding, size, &mut env);
            self.fp_commit(&env);
            self.fp_set_merging(d, d, mask, result);
            return Ok(Flow::Next);
        }

        let Some(ty) = FpType::from_ftype(ftype) else {
            return self.undefined(pc, insn);
        };
        let mut env = self.fp_env();
        let op = match opcode {
            0b000000 => None,
            0b000001 => Some(FpUnary::Abs),
            0b000010 => Some(FpUnary::Neg),
            0b000011 => Some(FpUnary::Sqrt),
            0b001000 => Some(FpUnary::RoundInt(
                FpRounding::TieEven,
                InexactReport::Suppress,
            )),
            0b001001 => Some(FpUnary::RoundInt(
                FpRounding::PosInf,
                InexactReport::Suppress,
            )),
            0b001010 => Some(FpUnary::RoundInt(
                FpRounding::NegInf,
                InexactReport::Suppress,
            )),
            0b001011 => Some(FpUnary::RoundInt(FpRounding::Zero, InexactReport::Suppress)),
            0b001100 => Some(FpUnary::RoundInt(
                FpRounding::TieAway,
                InexactReport::Suppress,
            )),
            0b001110 => Some(FpUnary::RoundInt(env.rounding(), InexactReport::Raise)),
            0b001111 => Some(FpUnary::RoundInt(env.rounding(), InexactReport::Suppress)),
            _ => return self.undefined(pc, insn),
        };
        self.fp_check()?;
        let a = self.fp_get(n, ty.mask());
        match op {
            // FMOV (register) zeroes the rest of the register whatever
            // FPCR.NEP is.
            None => self.fp_set(d, ty.mask(), a),
            Some(op) => {
                let result = fp_unary(op, ty, a, &mut env);
                self.fp_commit(&env);
                self.fp_set_merging(d, d, ty.mask(), result);
            }
        }
        Ok(Flow::Next)
    }

    /// Two sources: `000 11110 ftype 1 Rm opcode 10 Rn Rd`.
    pub(crate) fn exec_fp_dp2(&mut self, pc: u64, insn: u32) -> ExecResult {
        let (Some(ty), Some(op)) = (scalar_type(insn), dp2_operation(insn)) else {
            return self.undefined(pc, insn);
        };
        self.fp_dp2(
            op,
            ty,
            bits(insn, 4, 0),
            bits(insn, 9, 5),
            bits(insn, 20, 16),
        )
    }

    /// The FP data-processing (2 source) operation `op` of precision `ty`:
    /// `Vd = Vn op Vm`.
    #[inline]
    pub(crate) fn fp_dp2(
        &mut self,
        op: FpBinary,
        ty: FpType,
        d: u32,
        n: u32,
        m: u32,
    ) -> ExecResult {
        self.fp_check()?;
        let a = self.fp_get(n, ty.mask());
        let b = self.fp_get(m, ty.mask());
        let mut env = self.fp_env();
        let result = fp_binary(op, ty, a, b, &mut env);
        self.fp_commit(&env);
        self.fp_set_merging(d, n, ty.mask(), result);
        Ok(Flow::Next)
    }

    /// FMADD/FMSUB/FNMADD/FNMSUB: `000 11111 ftype o1 Rm o0 Ra Rn Rd`.
    pub(crate) fn exec_fp_dp3(&mut self, pc: u64, insn: u32) -> ExecResult {
        let Some(ty) = scalar_type(insn) else {
            return self.undefined(pc, insn);
        };
        let regs = FpDp3Regs {
            d: bits(insn, 4, 0),
            n: bits(insn, 9, 5),
            m: bits(insn, 20, 16),
            a: bits(insn, 14, 10),
        };
        self.fp_dp3(dp3_form(insn), ty, regs)
    }

    /// The fused multiply-add `form` of precision `ty` over `regs`.
    #[inline]
    pub(crate) fn fp_dp3(&mut self, form: FusedMulAdd, ty: FpType, regs: FpDp3Regs) -> ExecResult {
        self.fp_check()?;
        let n = self.fp_get(regs.n, ty.mask());
        let m = self.fp_get(regs.m, ty.mask());
        let a = self.fp_get(regs.a, ty.mask());
        let mut env = self.fp_env();
        let result = fp_mul_add(form, ty, a, n, m, &mut env);
        self.fp_commit(&env);
        self.fp_set_merging(regs.d, regs.a, ty.mask(), result);
        Ok(Flow::Next)
    }

    /// FCMP/FCMPE: `000 11110 ftype 1 Rm 00 1000 Rn opc 000`, with
    /// `opc<0>` comparing against +0.0 and `opc<1>` (E) signalling on any
    /// NaN.
    pub(crate) fn exec_fp_compare(&mut self, pc: u64, insn: u32) -> ExecResult {
        let Some(ty) = FpType::from_ftype(bits(insn, 23, 22)).filter(|_| !m_or_s(insn)) else {
            return self.undefined(pc, insn);
        };
        if bits(insn, 15, 14) != 0 || bits(insn, 2, 0) != 0 {
            return self.undefined(pc, insn);
        }
        self.fp_check()?;
        let a = self.fp_get(bits(insn, 9, 5), ty.mask());
        let b = if bit(insn, 3) {
            FpType::ZERO
        } else {
            self.fp_get(bits(insn, 20, 16), ty.mask())
        };
        let signal = if bit(insn, 4) {
            NanSignal::Signaling
        } else {
            NanSignal::Quiet
        };
        let mut env = self.fp_env();
        let ordering = fp_compare(ty, a, b, signal, &mut env);
        self.fp_commit(&env);
        self.pstate.set_nzcv(u64::from(ordering.nzcv()) << 28);
        Ok(Flow::Next)
    }

    /// FCCMP/FCCMPE: `000 11110 ftype 1 Rm cond 01 Rn op nzcv`.
    pub(crate) fn exec_fp_cond_compare(&mut self, pc: u64, insn: u32) -> ExecResult {
        let Some(ty) = FpType::from_ftype(bits(insn, 23, 22)).filter(|_| !m_or_s(insn)) else {
            return self.undefined(pc, insn);
        };
        self.fp_check()?;
        if self.condition_holds(bits(insn, 15, 12)) {
            let a = self.fp_get(bits(insn, 9, 5), ty.mask());
            let b = self.fp_get(bits(insn, 20, 16), ty.mask());
            let signal = if bit(insn, 4) {
                NanSignal::Signaling
            } else {
                NanSignal::Quiet
            };
            let mut env = self.fp_env();
            let ordering = fp_compare(ty, a, b, signal, &mut env);
            self.fp_commit(&env);
            self.pstate.set_nzcv(u64::from(ordering.nzcv()) << 28);
        } else {
            self.pstate.set_nzcv(u64::from(bits(insn, 3, 0)) << 28);
        }
        Ok(Flow::Next)
    }

    /// Conversions between FP and general registers:
    /// `sf 0 0 11110 ftype 1 rmode opcode 000000 Rn Rd`. Every form but
    /// FJCVTZS, the `FMOV` of `Vn.D[1]` among them, makes the scalar check
    /// (`AArch64_CheckFPEnabled` on the FMOV (general) page), so each is
    /// legal in Streaming SVE mode; FJCVTZS makes the Advanced SIMD one.
    pub(crate) fn exec_fp_int(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bit(insn, 29) {
            return self.undefined(pc, insn);
        }
        let sf = bit(insn, 31);
        let ftype = bits(insn, 23, 22);
        let rmode = bits(insn, 20, 19);
        let opcode = bits(insn, 18, 16);
        let n = bits(insn, 9, 5);
        let d = bits(insn, 4, 0);
        match (sf, ftype, rmode, opcode) {
            // FJCVTZS Wd, Dn (FEAT_JSCVT, which the decoder checks): the
            // W register zero-extended, and NZCV = 0, Z, 0, 0.
            (false, 1, 3, 0b110) => {
                self.simd_check()?;
                let mut env = self.fp_env();
                let converted =
                    sf::f64_to_i32_js(self.fp_get(n, FpType::Double.mask()), &mut env.status);
                self.fp_commit(&env);
                self.set_x_or_zr(d, u64::from(converted.value));
                self.pstate.set_nzcv(u64::from(converted.exact) << 30);
                return Ok(Flow::Next);
            }
            // FMOV Wd, Sn / FMOV Xd, Dn
            (false, 0, 0, 0b110) | (true, 1, 0, 0b110) => {
                self.fp_check()?;
                let ty = if sf { FpType::Double } else { FpType::Single };
                let v = self.fp_get(n, ty.mask());
                self.set_x_or_zr(d, v);
                return Ok(Flow::Next);
            }
            // FMOV Sd, Wn / FMOV Dd, Xn
            (false, 0, 0, 0b111) | (true, 1, 0, 0b111) => {
                self.fp_check()?;
                let ty = if sf { FpType::Double } else { FpType::Single };
                let v = self.x_or_zr(n);
                self.fp_set(d, ty.mask(), v);
                return Ok(Flow::Next);
            }
            // FMOV Wd, Hn / FMOV Xd, Hn (FEAT_FP16): zero-extended.
            (_, 3, 0, 0b110) => {
                self.fp_check()?;
                let v = self.fp_get(n, FpType::Half.mask());
                self.set_x_or_zr(d, v);
                return Ok(Flow::Next);
            }
            // FMOV Hd, Wn / FMOV Hd, Xn (FEAT_FP16): the low 16 bits.
            (_, 3, 0, 0b111) => {
                self.fp_check()?;
                let v = self.x_or_zr(n);
                self.fp_set(d, FpType::Half.mask(), v);
                return Ok(Flow::Next);
            }
            // FMOV Xd, Vn.D[1]
            (true, 2, 1, 0b110) => {
                self.fp_check()?;
                let v = (self.v[n as usize] >> 64) as u64;
                self.set_x_or_zr(d, v);
                return Ok(Flow::Next);
            }
            // FMOV Vd.D[1], Xn: the low half is preserved (Arm `Vpart`,
            // which writes all 128 bits back).
            (true, 2, 1, 0b111) => {
                self.fp_check()?;
                let v = self.x_or_zr(n);
                let low = self.v[d as usize] & u128::from(u64::MAX);
                self.write_v(d as usize, low | (u128::from(v) << 64));
                return Ok(Flow::Next);
            }
            _ => {}
        }
        let Some(ty) = FpType::from_ftype(ftype) else {
            return self.undefined(pc, insn);
        };
        enum Conversion {
            ToInt(FpRounding, bool),
            FromInt(bool),
        }
        let conversion = match (rmode, opcode) {
            // FCVT{N,P,M,Z}{S,U}
            (_, 0b000) => Conversion::ToInt(rounding_from_rmode(rmode), true),
            (_, 0b001) => Conversion::ToInt(rounding_from_rmode(rmode), false),
            // SCVTF, UCVTF
            (0, 0b010) => Conversion::FromInt(true),
            (0, 0b011) => Conversion::FromInt(false),
            // FCVTAS, FCVTAU
            (0, 0b100) => Conversion::ToInt(FpRounding::TieAway, true),
            (0, 0b101) => Conversion::ToInt(FpRounding::TieAway, false),
            _ => return self.undefined(pc, insn),
        };
        self.fp_check()?;
        let mut env = self.fp_env();
        match conversion {
            Conversion::ToInt(rounding, signed) => {
                let a = self.fp_get(n, ty.mask());
                let v = fp_to_int(ty, a, 0, rounding, signed, IntWidth::from_sf(sf), &mut env);
                self.fp_commit(&env);
                self.set_x_or_zr(d, v);
            }
            Conversion::FromInt(signed) => {
                let x = self.x_or_zr(n);
                let v = int_to_fp(ty, x, 0, signed, IntWidth::from_sf(sf), &mut env);
                self.fp_commit(&env);
                self.fp_set_merging(d, d, ty.mask(), v);
            }
        }
        Ok(Flow::Next)
    }

    /// Fixed-point conversions: `sf 0 0 11110 ftype 0 rmode opcode scale Rn
    /// Rd` — SCVTF, UCVTF, FCVTZS, FCVTZU with `fbits = 64 - scale`.
    pub(crate) fn exec_fp_fixed(&mut self, pc: u64, insn: u32) -> ExecResult {
        let sf = bit(insn, 31);
        let Some(ty) = FpType::from_ftype(bits(insn, 23, 22)).filter(|_| !bit(insn, 29)) else {
            return self.undefined(pc, insn);
        };
        let scale = bits(insn, 15, 10);
        if !sf && scale < 32 {
            return self.undefined(pc, insn);
        }
        let fbits = 64 - scale;
        let n = bits(insn, 9, 5);
        let d = bits(insn, 4, 0);
        match (bits(insn, 20, 19), bits(insn, 18, 16)) {
            (0b11, opcode @ (0b000 | 0b001)) => {
                self.fp_check()?;
                let mut env = self.fp_env();
                let a = self.fp_get(n, ty.mask());
                let v = fp_to_int(
                    ty,
                    a,
                    fbits,
                    FpRounding::Zero,
                    opcode == 0,
                    IntWidth::from_sf(sf),
                    &mut env,
                );
                self.fp_commit(&env);
                self.set_x_or_zr(d, v);
            }
            (0b00, opcode @ (0b010 | 0b011)) => {
                self.fp_check()?;
                let mut env = self.fp_env();
                let x = self.x_or_zr(n);
                let v = int_to_fp(
                    ty,
                    x,
                    fbits,
                    opcode == 0b010,
                    IntWidth::from_sf(sf),
                    &mut env,
                );
                self.fp_commit(&env);
                self.fp_set_merging(d, d, ty.mask(), v);
            }
            _ => return self.undefined(pc, insn),
        }
        Ok(Flow::Next)
    }
}
