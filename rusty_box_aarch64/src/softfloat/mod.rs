//! AArch64 floating-point arithmetic on raw IEEE 754 bit patterns.
//!
//! The arithmetic is the rusty_box port of Berkeley SoftFloat 3e (via Bochs
//! softfloat3e) restricted to half, single and double precision, with the Arm
//! specialization in [`specialize`]. This file is the surface an AArch64
//! executor calls: [`ArmFpStatus`] carries the FPCR controls and accumulates
//! the FPSR cumulative exception bits, and each operation takes its operands
//! as the register bits (`u16`/`u32`/`u64`).
//!
//! Every single- and double-precision operand passes through Arm FPUnpack
//! here, before the operation sees it: under FPCR.FZ a denormal operand
//! becomes a zero of the same sign and sets IDC. Half-precision operands are
//! never flushed (FPUnpackCV forces FZ16 off).

mod arm_ops;
mod bfloat;
mod f16_to_f32;
mod f16_to_f64;
mod f32_addsub;
mod f32_class;
mod f32_compare;
mod f32_div;
mod f32_mul;
mod f32_mul_add;
mod f32_round_to_int;
mod f32_sqrt;
mod f32_to_f16;
mod f32_to_f64;
mod f32_to_int;
mod f64_addsub;
mod f64_class;
mod f64_compare;
mod f64_div;
mod f64_mul;
mod f64_mul_add;
mod f64_round_to_int;
mod f64_sqrt;
mod f64_to_f16;
mod f64_to_f32;
mod f64_to_int;
mod half;
mod int_to_float;
mod internals;
mod min_max;
mod primitives;
#[allow(clippy::module_inception)]
mod softfloat;
mod softfloat_types;
mod specialize;
mod uint64_convert;
mod uint_convert;

#[cfg(test)]
#[path = "../../tests/unit/softfloat.rs"]
mod tests;

use self::min_max::Extreme;
use self::softfloat::*;
use self::specialize::{f32_flush_input, f64_flush_input};

pub(crate) use self::arm_ops::{
    f16_ahp_to_f32, f16_ahp_to_f64, f32_mulx, f32_recip_step, f32_rsqrt_step, f32_to_f16_ahp, f32_to_i32_fixed,
    f32_to_i64_fixed, f32_to_u32_fixed, f32_to_u64_fixed, f64_mulx, f64_recip_step, f64_rsqrt_step,
    f64_to_f16_ahp, f64_to_f32_round_odd, f64_to_i32_fixed, f64_to_i32_js, f64_to_i64_fixed, f64_to_u32_fixed,
    f64_to_u64_fixed, i64_to_f32_fixed, i64_to_f64_fixed, u64_to_f32_fixed, u64_to_f64_fixed,
};
pub(crate) use self::bfloat::{bf16_dot_add, f32_to_bf16};
pub(crate) use self::half::{
    f16_add, f16_compare, f16_div, f16_max, f16_maxnum, f16_min, f16_minnum, f16_mul, f16_mul_add, f16_mul_add_long,
    f16_mulx, f16_recip_step, f16_round_to_int, f16_rsqrt_step, f16_sqrt, f16_sub, f16_to_signed, f16_to_unsigned,
    int_to_f16,
};
pub(crate) use self::softfloat::FpOrdering;

// FPCR fields (Arm ARM FPCR).
const FPCR_RMODE_SHIFT: u32 = 22;
const FPCR_FZ: u64 = 1 << 24;
/// FPCR.FZ16 (FEAT_FP16): flush to zero for half-precision arithmetic.
pub(crate) const FPCR_FZ16: u64 = 1 << 19;
const FPCR_DN: u64 = 1 << 25;

// FPSR cumulative exception bits (Arm ARM FPSR).
const FPSR_IOC: u64 = 1 << 0;
const FPSR_DZC: u64 = 1 << 1;
const FPSR_OFC: u64 = 1 << 2;
const FPSR_UFC: u64 = 1 << 3;
const FPSR_IXC: u64 = 1 << 4;
const FPSR_IDC: u64 = 1 << 7;

/// The FPSR bits [`ArmFpStatus`] accumulates: IOC, DZC, OFC, UFC, IXC, IDC.
pub(crate) const FPSR_CUMULATIVE_MASK: u64 =
    FPSR_IOC | FPSR_DZC | FPSR_OFC | FPSR_UFC | FPSR_IXC | FPSR_IDC;

/// Each status flag and the FPSR cumulative bit it sets, one to one.
/// `FLAG_DENORMAL` (a denormal used unflushed) has no FPSR bit under
/// FPCR.AH = 0 and is absent.
const FLAG_TO_FPSR: [(i32, u64); 6] = [
    (FLAG_INVALID, FPSR_IOC),
    (FLAG_DIVBYZERO, FPSR_DZC),
    (FLAG_OVERFLOW, FPSR_OFC),
    (FLAG_UNDERFLOW, FPSR_UFC),
    (FLAG_INEXACT, FPSR_IXC),
    (FLAG_INPUT_DENORMAL, FPSR_IDC),
];

/// Arm FPRounding: the rounding a conversion or FRINT applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FpRounding {
    /// Round to nearest, ties to even (FPCR.RMode 0b00; FCVTN*, FRINTN).
    TieEven,
    /// Round toward +infinity (FPCR.RMode 0b01; FCVTP*, FRINTP).
    PosInf,
    /// Round toward -infinity (FPCR.RMode 0b10; FCVTM*, FRINTM).
    NegInf,
    /// Round toward zero (FPCR.RMode 0b11; FCVTZ*, FRINTZ).
    Zero,
    /// Round to nearest, ties away from zero (FCVTA*, FRINTA).
    TieAway,
}

impl FpRounding {
    /// The rounding FPCR.RMode (bits 23:22) selects.
    fn from_fpcr(fpcr: u64) -> Self {
        match (fpcr >> FPCR_RMODE_SHIFT) & 0b11 {
            0b00 => FpRounding::TieEven,
            0b01 => FpRounding::PosInf,
            0b10 => FpRounding::NegInf,
            // The two-bit field leaves only 0b11.
            _ => FpRounding::Zero,
        }
    }

    /// The SoftFloat `softfloat_round_*` mode for this rounding.
    fn softfloat_mode(self) -> u8 {
        match self {
            FpRounding::TieEven => ROUND_NEAR_EVEN,
            FpRounding::PosInf => ROUND_MAX,
            FpRounding::NegInf => ROUND_MIN,
            FpRounding::Zero => ROUND_MINMAG,
            FpRounding::TieAway => ROUND_NEAR_MAXMAG,
        }
    }
}

/// Whether a comparison treats a quiet NaN operand as an invalid operation.
/// A signaling NaN is invalid either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NanSignal {
    /// FCMP, FCMEQ: only a signaling NaN raises invalid.
    Quiet,
    /// FCMPE, FCMGE, FCMGT: any NaN raises invalid.
    Signaling,
}

/// Whether FRINT* reports a result that differs from its operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InexactReport {
    /// FRINTN/P/M/Z/A/I: no inexact exception.
    Suppress,
    /// FRINTX: inexact when the result differs from the operand.
    Raise,
}

/// The four A64 fused multiply-add forms. Arm negates operands bitwise
/// (FPNeg, which flips a NaN's sign too) before FPMulAdd.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FusedMulAdd {
    /// FMADD / FMLA: `a + n*m`.
    Fmadd,
    /// FMSUB / FMLS: `a - n*m` (n negated).
    Fmsub,
    /// FNMADD: `-a - n*m` (a and n negated).
    Fnmadd,
    /// FNMSUB: `-a + n*m` (a negated).
    Fnmsub,
}

impl FusedMulAdd {
    fn negates_addend(self) -> bool {
        matches!(self, FusedMulAdd::Fnmadd | FusedMulAdd::Fnmsub)
    }

    fn negates_multiplicand(self) -> bool {
        matches!(self, FusedMulAdd::Fmsub | FusedMulAdd::Fnmadd)
    }
}

/// The floating-point environment of one AArch64 PE: the FPCR controls the
/// arithmetic reads (RMode, FZ, DN; the caller picks the AHP conversions in
/// `arm_ops` itself) and the FPSR cumulative exception bits the operations
/// accumulate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArmFpStatus {
    status: SoftFloatStatus,
    /// FPCR.FZ16, which only the half-precision arithmetic in `half`
    /// reads.
    flush_half: bool,
}

impl ArmFpStatus {
    /// A status with the controls of `fpcr` and no exception accumulated.
    pub(crate) fn from_fpcr(fpcr: u64) -> Self {
        let mut fp = Self {
            status: SoftFloatStatus::default(),
            flush_half: false,
        };
        fp.set_fpcr(fpcr);
        fp
    }

    /// Adopts the controls of a new FPCR value (MSR FPCR), keeping the
    /// accumulated exception bits.
    pub(crate) fn set_fpcr(&mut self, fpcr: u64) {
        self.status.softfloat_rounding_mode = FpRounding::from_fpcr(fpcr).softfloat_mode();
        self.status.flush_to_zero = fpcr & FPCR_FZ != 0;
        self.status.default_nan_mode = fpcr & FPCR_DN != 0;
        self.flush_half = fpcr & FPCR_FZ16 != 0;
    }

    /// The FPCR.RMode rounding, for the operations Arm defines to use it
    /// (arithmetic, SCVTF/UCVTF, FCVT, FRINTI, FRINTX).
    pub(crate) fn rounding(&self) -> FpRounding {
        match self.status.softfloat_rounding_mode {
            ROUND_MAX => FpRounding::PosInf,
            ROUND_MIN => FpRounding::NegInf,
            ROUND_MINMAG => FpRounding::Zero,
            ROUND_NEAR_MAXMAG => FpRounding::TieAway,
            _ => FpRounding::TieEven,
        }
    }

    /// The accumulated exceptions as FPSR cumulative bits (IOC = bit 0,
    /// DZC = 1, OFC = 2, UFC = 3, IXC = 4, IDC = 7); every other bit is 0.
    pub(crate) fn cumulative_fpsr(&self) -> u64 {
        let flags = self.status.softfloat_exception_flags;
        FLAG_TO_FPSR
            .iter()
            .filter(|(flag, _)| flags & flag != 0)
            .fold(0, |bits, (_, fpsr_bit)| bits | fpsr_bit)
    }

    /// Replaces the accumulated exceptions with the cumulative bits of an
    /// FPSR value (MSR FPSR); bits outside [`FPSR_CUMULATIVE_MASK`] are
    /// ignored.
    pub(crate) fn set_cumulative_fpsr(&mut self, fpsr: u64) {
        self.status.softfloat_exception_flags = FLAG_TO_FPSR
            .iter()
            .filter(|(_, fpsr_bit)| fpsr & fpsr_bit != 0)
            .fold(0, |flags, (flag, _)| flags | flag);
    }
}

/// Runs `op` under `rounding` instead of FPCR.RMode, accumulating its
/// exceptions into `fp`.
fn with_rounding<T>(
    fp: &mut ArmFpStatus,
    rounding: FpRounding,
    op: impl FnOnce(&mut SoftFloatStatus) -> T,
) -> T {
    let mut local = SoftFloatStatus {
        softfloat_rounding_mode: rounding.softfloat_mode(),
        ..fp.status
    };
    let result = op(&mut local);
    fp.status.softfloat_exception_flags = local.softfloat_exception_flags;
    result
}

// ============================================================
// Single precision
// ============================================================

/// FADD (single). Arm FPAdd.
pub(crate) fn f32_add(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    f32_addsub::f32_add(a, b, s)
}

/// FSUB (single). Arm FPSub.
pub(crate) fn f32_sub(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    f32_addsub::f32_sub(a, b, s)
}

/// FMUL (single). Arm FPMul.
pub(crate) fn f32_mul(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    f32_mul::f32_mul(a, b, s)
}

/// FDIV (single). Arm FPDiv.
pub(crate) fn f32_div(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    f32_div::f32_div(a, b, s)
}

/// FSQRT (single). Arm FPSqrt.
pub(crate) fn f32_sqrt(a: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    f32_sqrt::f32_sqrt(a, s)
}

/// FMADD/FMSUB/FNMADD/FNMSUB (single), `addend` being Ra. Arm FPMulAdd
/// after the form's operand negations.
pub(crate) fn f32_mul_add(form: FusedMulAdd, addend: u32, n: u32, m: u32, fp: &mut ArmFpStatus) -> u32 {
    let addend = if form.negates_addend() { addend ^ 0x8000_0000 } else { addend };
    let n = if form.negates_multiplicand() { n ^ 0x8000_0000 } else { n };
    let s = &mut fp.status;
    let addend = f32_flush_input(addend, s);
    let n = f32_flush_input(n, s);
    let m = f32_flush_input(m, s);
    f32_mul_add::f32_mul_add(n, m, addend, s)
}

/// FMAX (single). Arm FPMax.
pub(crate) fn f32_max(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    min_max::f32_min_max(a, b, Extreme::Max, s)
}

/// FMIN (single). Arm FPMin.
pub(crate) fn f32_min(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    min_max::f32_min_max(a, b, Extreme::Min, s)
}

/// FMAXNM (single). Arm FPMaxNum.
pub(crate) fn f32_maxnum(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    min_max::f32_min_max_num(a, b, Extreme::Max, s)
}

/// FMINNM (single). Arm FPMinNum.
pub(crate) fn f32_minnum(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    min_max::f32_min_max_num(a, b, Extreme::Min, s)
}

/// FCMP/FCMPE (single) and the ordering behind FCMEQ/FCMGE/FCMGT.
/// Arm FPCompare; [`FpOrdering::nzcv`] gives the flags FCMP writes.
pub(crate) fn f32_compare(a: u32, b: u32, signal: NanSignal, fp: &mut ArmFpStatus) -> FpOrdering {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    f32_compare::f32_compare(a, b, signal == NanSignal::Quiet, s)
}

/// FRINT* (single). Arm FPRoundInt.
pub(crate) fn f32_round_to_int(a: u32, rounding: FpRounding, report: InexactReport, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    f32_round_to_int::f32_round_to_int(a, rounding.softfloat_mode(), report == InexactReport::Raise, s)
}

// ============================================================
// Double precision
// ============================================================

/// FADD (double). Arm FPAdd.
pub(crate) fn f64_add(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    f64_addsub::f64_add(a, b, s)
}

/// FSUB (double). Arm FPSub.
pub(crate) fn f64_sub(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    f64_addsub::f64_sub(a, b, s)
}

/// FMUL (double). Arm FPMul.
pub(crate) fn f64_mul(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    f64_mul::f64_mul(a, b, s)
}

/// FDIV (double). Arm FPDiv.
pub(crate) fn f64_div(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    f64_div::f64_div(a, b, s)
}

/// FSQRT (double). Arm FPSqrt.
pub(crate) fn f64_sqrt(a: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    f64_sqrt::f64_sqrt(a, s)
}

/// FMADD/FMSUB/FNMADD/FNMSUB (double), `addend` being Ra. Arm FPMulAdd
/// after the form's operand negations.
pub(crate) fn f64_mul_add(form: FusedMulAdd, addend: u64, n: u64, m: u64, fp: &mut ArmFpStatus) -> u64 {
    const SIGN: u64 = 0x8000_0000_0000_0000;
    let addend = if form.negates_addend() { addend ^ SIGN } else { addend };
    let n = if form.negates_multiplicand() { n ^ SIGN } else { n };
    let s = &mut fp.status;
    let addend = f64_flush_input(addend, s);
    let n = f64_flush_input(n, s);
    let m = f64_flush_input(m, s);
    f64_mul_add::f64_mul_add(n, m, addend, s)
}

/// FMAX (double). Arm FPMax.
pub(crate) fn f64_max(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    min_max::f64_min_max(a, b, Extreme::Max, s)
}

/// FMIN (double). Arm FPMin.
pub(crate) fn f64_min(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    min_max::f64_min_max(a, b, Extreme::Min, s)
}

/// FMAXNM (double). Arm FPMaxNum.
pub(crate) fn f64_maxnum(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    min_max::f64_min_max_num(a, b, Extreme::Max, s)
}

/// FMINNM (double). Arm FPMinNum.
pub(crate) fn f64_minnum(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    min_max::f64_min_max_num(a, b, Extreme::Min, s)
}

/// FCMP/FCMPE (double) and the ordering behind FCMEQ/FCMGE/FCMGT.
/// Arm FPCompare.
pub(crate) fn f64_compare(a: u64, b: u64, signal: NanSignal, fp: &mut ArmFpStatus) -> FpOrdering {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    f64_compare::f64_compare(a, b, signal == NanSignal::Quiet, s)
}

/// FRINT* (double). Arm FPRoundInt.
pub(crate) fn f64_round_to_int(a: u64, rounding: FpRounding, report: InexactReport, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    f64_round_to_int::f64_round_to_int(a, rounding.softfloat_mode(), report == InexactReport::Raise, s)
}

// ============================================================
// Precision conversions (FCVT), FPCR.RMode rounding
// ============================================================

/// FCVT Dd, Sn. Arm FPConvert.
pub(crate) fn f32_to_f64(a: u32, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    f32_to_f64::f32_to_f64(a, s)
}

/// FCVT Sd, Dn. Arm FPConvert.
pub(crate) fn f64_to_f32(a: u64, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    f64_to_f32::f64_to_f32(a, s)
}

/// FCVT Sd, Hn. Arm FPConvert (exact; a half-precision input is never
/// flushed).
pub(crate) fn f16_to_f32(a: u16, fp: &mut ArmFpStatus) -> u32 {
    f16_to_f32::f16_to_f32(a, &mut fp.status)
}

/// FCVT Dd, Hn. Arm FPConvert (exact; a half-precision input is never
/// flushed).
pub(crate) fn f16_to_f64(a: u16, fp: &mut ArmFpStatus) -> u64 {
    f16_to_f64::f16_to_f64(a, &mut fp.status)
}

/// FCVT Hd, Sn. Arm FPConvert (IEEE half precision; the result is never
/// flushed).
pub(crate) fn f32_to_f16(a: u32, fp: &mut ArmFpStatus) -> u16 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    f32_to_f16::f32_to_f16(a, s)
}

/// FCVT Hd, Dn. Arm FPConvert (IEEE half precision; the result is never
/// flushed).
pub(crate) fn f64_to_f16(a: u64, fp: &mut ArmFpStatus) -> u16 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    f64_to_f16::f64_to_f16(a, s)
}

// ============================================================
// Float to integer (FCVT{N,P,M,Z,A}{S,U}). Arm FPToFixed with no fraction
// bits: NaN gives 0, overflow saturates, both raise invalid; any other
// rounded result raises inexact.
// ============================================================

/// Single precision to `i32`.
pub(crate) fn f32_to_i32(a: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> i32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    f32_to_int::f32_to_i32(a, rounding.softfloat_mode(), true, s)
}

/// Single precision to `i64`.
pub(crate) fn f32_to_i64(a: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> i64 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    f32_to_int::f32_to_i64(a, rounding.softfloat_mode(), true, s)
}

/// Single precision to `u32` (negative results saturate to 0).
pub(crate) fn f32_to_u32(a: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    uint_convert::f32_to_ui32(a, rounding.softfloat_mode(), true, s)
}

/// Single precision to `u64` (negative results saturate to 0).
pub(crate) fn f32_to_u64(a: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    uint64_convert::f32_to_ui64(a, rounding.softfloat_mode(), true, s)
}

/// Double precision to `i32`.
pub(crate) fn f64_to_i32(a: u64, rounding: FpRounding, fp: &mut ArmFpStatus) -> i32 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    f64_to_int::f64_to_i32(a, rounding.softfloat_mode(), true, s)
}

/// Double precision to `i64`.
pub(crate) fn f64_to_i64(a: u64, rounding: FpRounding, fp: &mut ArmFpStatus) -> i64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    f64_to_int::f64_to_i64(a, rounding.softfloat_mode(), true, s)
}

/// Double precision to `u32` (negative results saturate to 0).
pub(crate) fn f64_to_u32(a: u64, rounding: FpRounding, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    uint_convert::f64_to_ui32(a, rounding.softfloat_mode(), true, s)
}

/// Double precision to `u64` (negative results saturate to 0).
pub(crate) fn f64_to_u64(a: u64, rounding: FpRounding, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    uint64_convert::f64_to_ui64(a, rounding.softfloat_mode(), true, s)
}

// ============================================================
// Integer to float (SCVTF/UCVTF). Arm FixedToFP; SCVTF/UCVTF pass
// `fp.rounding()`. The 32-bit-to-double forms are always exact.
// ============================================================

/// `i32` to single precision.
pub(crate) fn i32_to_f32(a: i32, rounding: FpRounding, fp: &mut ArmFpStatus) -> u32 {
    with_rounding(fp, rounding, |s| int_to_float::i32_to_f32(a, s))
}

/// `i64` to single precision.
pub(crate) fn i64_to_f32(a: i64, rounding: FpRounding, fp: &mut ArmFpStatus) -> u32 {
    with_rounding(fp, rounding, |s| int_to_float::i64_to_f32(a, s))
}

/// `u32` to single precision.
pub(crate) fn u32_to_f32(a: u32, rounding: FpRounding, fp: &mut ArmFpStatus) -> u32 {
    with_rounding(fp, rounding, |s| uint_convert::ui32_to_f32(a, s))
}

/// `u64` to single precision.
pub(crate) fn u64_to_f32(a: u64, rounding: FpRounding, fp: &mut ArmFpStatus) -> u32 {
    with_rounding(fp, rounding, |s| uint64_convert::ui64_to_f32(a, s))
}

/// `i32` to double precision (exact).
pub(crate) fn i32_to_f64(a: i32) -> u64 {
    int_to_float::i32_to_f64(a)
}

/// `u32` to double precision (exact).
pub(crate) fn u32_to_f64(a: u32) -> u64 {
    uint_convert::ui32_to_f64(a)
}

/// `i64` to double precision.
pub(crate) fn i64_to_f64(a: i64, rounding: FpRounding, fp: &mut ArmFpStatus) -> u64 {
    with_rounding(fp, rounding, |s| int_to_float::i64_to_f64(a, s))
}

/// `u64` to double precision.
pub(crate) fn u64_to_f64(a: u64, rounding: FpRounding, fp: &mut ArmFpStatus) -> u64 {
    with_rounding(fp, rounding, |s| uint64_convert::ui64_to_f64(a, s))
}
