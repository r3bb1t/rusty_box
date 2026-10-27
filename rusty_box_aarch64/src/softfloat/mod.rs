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
//! here, before the operation sees it: under FPCR.FZ with FPCR.AH = 0 a
//! denormal operand becomes a zero of the same sign and sets IDC, and under
//! FPCR.FIZ (FEAT_AFP) it becomes one silently. Under FPCR.AH = 1 an
//! operation that uses a denormal operand sets IDC instead, in the cases
//! its pseudocode calls FPProcessDenorms, which each function here names.
//! Half-precision operands follow FPCR.FZ16 alone (`half`).

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

use self::min_max::{Extreme, MinMaxRules};
use self::softfloat::*;
use self::specialize::{f32_flush_input, f32_report_denormals, f64_flush_input, f64_report_denormals};

pub(crate) use self::arm_ops::{
    f16_ahp_to_f32, f16_ahp_to_f64, f32_log_b, f32_mulx, f32_recip_step, f32_rsqrt_step, f32_scale, f32_to_f16_ahp,
    f32_to_i32_fixed, f32_to_i64_fixed, f32_to_u32_fixed, f32_to_u64_fixed, f64_log_b, f64_mulx, f64_recip_step,
    f64_rsqrt_step, f64_scale, f64_to_f16_ahp, f64_to_f32_round_odd, f64_to_i32_fixed, f64_to_i32_js,
    f64_to_i64_fixed, f64_to_u32_fixed, f64_to_u64_fixed, i64_to_f32_fixed, i64_to_f64_fixed, u64_to_f32_fixed,
    u64_to_f64_fixed,
};
pub(crate) use self::bfloat::{bf16_dot_add, f32_to_bf16};
pub(crate) use self::half::{
    f16_add, f16_compare, f16_div, f16_dot, f16_log_b, f16_max, f16_maxnum, f16_min, f16_minnum, f16_mul, f16_mul_add,
    f16_mul_add_long, f16_mulx, f16_recip_step, f16_round_to_int, f16_rsqrt_step, f16_scale, f16_sqrt, f16_sub,
    f16_to_signed, f16_to_unsigned, int_to_f16,
};
pub(crate) use self::softfloat::{FpOrdering, Handling, InputFlush};

// FPCR fields (Arm ARM FPCR).
/// FPCR.FIZ (FEAT_AFP): flush denormal single- and double-precision
/// operands to zero, raising nothing.
pub(crate) const FPCR_FIZ: u64 = 1 << 0;
/// FPCR.AH (FEAT_AFP): alternate handling ([`Handling::Alternate`]).
pub(crate) const FPCR_AH: u64 = 1 << 1;
/// FPCR.NEP (FEAT_AFP): a scalar result merges into the upper elements of a
/// source register instead of zeroing them (Arm `IsMerging`).
pub(crate) const FPCR_NEP: u64 = 1 << 2;
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

/// The four A64 fused multiply-add forms. Arm negates operands (FPNeg,
/// which flips a NaN's sign too under FPCR.AH = 0) before FPMulAdd.
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
/// arithmetic reads (RMode, FZ, FZ16, DN, and FEAT_AFP's AH and FIZ; the
/// caller picks the AHP conversions in `arm_ops` itself) and the FPSR
/// cumulative exception bits the operations accumulate.
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
    /// accumulated exception bits. A processor without FEAT_AFP keeps
    /// FPCR.AH and FPCR.FIZ zero.
    pub(crate) fn set_fpcr(&mut self, fpcr: u64) {
        let alternate = fpcr & FPCR_AH != 0;
        self.status.softfloat_rounding_mode = FpRounding::from_fpcr(fpcr).softfloat_mode();
        self.status.flush_to_zero = fpcr & FPCR_FZ != 0;
        self.status.default_nan_mode = fpcr & FPCR_DN != 0;
        self.status.handling = if alternate { Handling::Alternate } else { Handling::Standard };
        // FPUnpackBase: `fz = FPCR.FZ && !altfp`, `fiz = FPCR.FIZ`; only
        // the first raises Input Denormal.
        self.status.input_flush = if fpcr & FPCR_FZ != 0 && !alternate {
            InputFlush::Reported
        } else if fpcr & FPCR_FIZ != 0 {
            InputFlush::Unreported
        } else {
            InputFlush::Off
        };
        self.flush_half = fpcr & FPCR_FZ16 != 0;
    }

    /// FPCR.AH.
    #[inline]
    pub(crate) fn handling(&self) -> Handling {
        self.status.handling
    }

    /// These controls with FPCR.DN set (Arm `fpcr.DN = '1'`): every NaN
    /// result the default NaN.
    pub(crate) fn with_default_nan(mut self) -> Self {
        self.status.default_nan_mode = true;
        self
    }

    /// Arm FPDefaultNaN{16} under these controls: negative under FPCR.AH.
    pub(crate) fn default_nan_f16(&self) -> u16 {
        specialize::float16_default_nan(&self.status)
    }

    /// Arm FPDefaultNaN{32}.
    pub(crate) fn default_nan_f32(&self) -> u32 {
        specialize::float32_default_nan(&self.status)
    }

    /// Arm FPDefaultNaN{64}.
    pub(crate) fn default_nan_f64(&self) -> u64 {
        specialize::float64_default_nan(&self.status)
    }

    /// Runs `op` as Arm runs FPRecipStepFused, FPRSqrtStepFused, BFMulAddH
    /// and FPConvertBF: under FPCR.AH = 0 under FPCR, accumulating what it
    /// raises; under FPCR.AH = 1 with the pseudocode's `altfp` overrides,
    /// `fpcr.[FIZ,FZ] = '11'` and `fpcr.RMode = '00'`, so every single- and
    /// double-precision denormal flushes, operand and result alike, and
    /// rounding is to nearest, and with `fpexc` FALSE, so nothing `op`
    /// raises reaches the FPSR.
    pub(crate) fn with_altfp_overrides<T>(&mut self, op: impl FnOnce(&mut ArmFpStatus) -> T) -> T {
        match self.status.handling {
            Handling::Standard => op(self),
            Handling::Alternate => {
                let mut fp = *self;
                fp.status.softfloat_exception_flags = 0;
                fp.status.flush_to_zero = true;
                fp.status.input_flush = InputFlush::Unreported;
                fp.status.softfloat_rounding_mode = ROUND_NEAR_EVEN;
                op(&mut fp)
            }
        }
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

/// The rules FMAX and FMIN follow: FPMax's and FPMin's `altfp`, FPCR.AH.
fn min_max_rules(s: &SoftFloatStatus) -> MinMaxRules {
    match s.handling {
        Handling::Standard => MinMaxRules::Ieee,
        Handling::Alternate => MinMaxRules::Alternate,
    }
}

// ============================================================
// Single precision
// ============================================================

/// Arm FPNeg (single): the sign flipped — a NaN's too under FPCR.AH = 0,
/// whose sign then has no consequence and stays.
#[inline]
pub(crate) fn f32_neg(a: u32, s: &SoftFloatStatus) -> u32 {
    if s.handling == Handling::Alternate && internals::is_nan_f32(a) {
        a
    } else {
        a ^ 0x8000_0000
    }
}

/// Arm FPProcessDenorms where an operation calls it once FPProcessNaNs
/// found no NaN among `operands`.
#[inline]
fn f32_report_denormals_unless_nan(operands: &[u32], s: &mut SoftFloatStatus) {
    if s.handling == Handling::Alternate && !operands.iter().any(|&x| internals::is_nan_f32(x)) {
        f32_report_denormals(operands, s);
    }
}

/// FPMulAdd's Invalid Operation without a NaN operand: an infinity times a
/// zero, or an infinite product added to an infinity of the other sign.
fn f32_mul_add_invalid(addend: u32, n: u32, m: u32) -> bool {
    let inf = |x: u32| x & 0x7FFF_FFFF == 0x7F80_0000;
    let zero = |x: u32| x & 0x7FFF_FFFF == 0;
    (inf(n) && zero(m))
        || (zero(n) && inf(m))
        || (inf(addend) && (inf(n) || inf(m)) && (addend ^ n ^ m) & 0x8000_0000 != 0)
}

/// FADD (single). Arm FPAdd.
pub(crate) fn f32_add(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    f32_report_denormals_unless_nan(&[a, b], s);
    f32_addsub::f32_add(a, b, s)
}

/// FSUB (single). Arm FPSub.
pub(crate) fn f32_sub(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    f32_report_denormals_unless_nan(&[a, b], s);
    f32_addsub::f32_sub(a, b, s)
}

/// FMUL (single). Arm FPMul.
pub(crate) fn f32_mul(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    f32_report_denormals_unless_nan(&[a, b], s);
    f32_mul::f32_mul(a, b, s)
}

/// FDIV (single). Arm FPDiv: a used denormal is reported unless the
/// divisor is a zero.
pub(crate) fn f32_div(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    if b & 0x7FFF_FFFF != 0 {
        f32_report_denormals_unless_nan(&[a, b], s);
    }
    f32_div::f32_div(a, b, s)
}

/// FSQRT (single). Arm FPSqrt: a used denormal is reported when it is
/// positive (a negative one is an invalid operand).
pub(crate) fn f32_sqrt(a: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    if a & 0x8000_0000 == 0 {
        f32_report_denormals(&[a], s);
    }
    f32_sqrt::f32_sqrt(a, s)
}

/// FMADD/FMSUB/FNMADD/FNMSUB (single), `addend` being Ra. Arm FPMulAdd
/// after the form's operand negations (FPNeg); a used denormal is reported
/// unless a NaN or an invalid operation decided the result.
pub(crate) fn f32_mul_add(form: FusedMulAdd, addend: u32, n: u32, m: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let addend = if form.negates_addend() { f32_neg(addend, s) } else { addend };
    let n = if form.negates_multiplicand() { f32_neg(n, s) } else { n };
    let addend = f32_flush_input(addend, s);
    let n = f32_flush_input(n, s);
    let m = f32_flush_input(m, s);
    if s.handling == Handling::Alternate && !f32_mul_add_invalid(addend, n, m) {
        f32_report_denormals_unless_nan(&[addend, n, m], s);
    }
    f32_mul_add::f32_mul_add(n, m, addend, s)
}

/// FMAX (single). Arm FPMax.
pub(crate) fn f32_max(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    min_max::f32_min_max(a, b, Extreme::Max, min_max_rules(s), s)
}

/// FMIN (single). Arm FPMin.
pub(crate) fn f32_min(a: u32, b: u32, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    let b = f32_flush_input(b, s);
    min_max::f32_min_max(a, b, Extreme::Min, min_max_rules(s), s)
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
    f32_report_denormals_unless_nan(&[a, b], s);
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

const F64_SIGN: u64 = 0x8000_0000_0000_0000;

/// Arm FPNeg (double); see [`f32_neg`].
#[inline]
pub(crate) fn f64_neg(a: u64, s: &SoftFloatStatus) -> u64 {
    if s.handling == Handling::Alternate && internals::is_nan_f64(a) {
        a
    } else {
        a ^ F64_SIGN
    }
}

/// Double-precision form of [`f32_report_denormals_unless_nan`].
#[inline]
fn f64_report_denormals_unless_nan(operands: &[u64], s: &mut SoftFloatStatus) {
    if s.handling == Handling::Alternate && !operands.iter().any(|&x| internals::is_nan_f64(x)) {
        f64_report_denormals(operands, s);
    }
}

/// Double-precision form of [`f32_mul_add_invalid`].
fn f64_mul_add_invalid(addend: u64, n: u64, m: u64) -> bool {
    let inf = |x: u64| x & !F64_SIGN == 0x7FF0_0000_0000_0000;
    let zero = |x: u64| x & !F64_SIGN == 0;
    (inf(n) && zero(m)) || (zero(n) && inf(m)) || (inf(addend) && (inf(n) || inf(m)) && (addend ^ n ^ m) & F64_SIGN != 0)
}

/// FADD (double). Arm FPAdd.
pub(crate) fn f64_add(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    f64_report_denormals_unless_nan(&[a, b], s);
    f64_addsub::f64_add(a, b, s)
}

/// FSUB (double). Arm FPSub.
pub(crate) fn f64_sub(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    f64_report_denormals_unless_nan(&[a, b], s);
    f64_addsub::f64_sub(a, b, s)
}

/// FMUL (double). Arm FPMul.
pub(crate) fn f64_mul(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    f64_report_denormals_unless_nan(&[a, b], s);
    f64_mul::f64_mul(a, b, s)
}

/// FDIV (double). Arm FPDiv; see [`f32_div`].
pub(crate) fn f64_div(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    if b & !F64_SIGN != 0 {
        f64_report_denormals_unless_nan(&[a, b], s);
    }
    f64_div::f64_div(a, b, s)
}

/// FSQRT (double). Arm FPSqrt; see [`f32_sqrt`].
pub(crate) fn f64_sqrt(a: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    if a & F64_SIGN == 0 {
        f64_report_denormals(&[a], s);
    }
    f64_sqrt::f64_sqrt(a, s)
}

/// FMADD/FMSUB/FNMADD/FNMSUB (double), `addend` being Ra. Arm FPMulAdd
/// after the form's operand negations; see [`f32_mul_add`].
pub(crate) fn f64_mul_add(form: FusedMulAdd, addend: u64, n: u64, m: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let addend = if form.negates_addend() { f64_neg(addend, s) } else { addend };
    let n = if form.negates_multiplicand() { f64_neg(n, s) } else { n };
    let addend = f64_flush_input(addend, s);
    let n = f64_flush_input(n, s);
    let m = f64_flush_input(m, s);
    if s.handling == Handling::Alternate && !f64_mul_add_invalid(addend, n, m) {
        f64_report_denormals_unless_nan(&[addend, n, m], s);
    }
    f64_mul_add::f64_mul_add(n, m, addend, s)
}

/// FMAX (double). Arm FPMax.
pub(crate) fn f64_max(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    min_max::f64_min_max(a, b, Extreme::Max, min_max_rules(s), s)
}

/// FMIN (double). Arm FPMin.
pub(crate) fn f64_min(a: u64, b: u64, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    let b = f64_flush_input(b, s);
    min_max::f64_min_max(a, b, Extreme::Min, min_max_rules(s), s)
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
    f64_report_denormals_unless_nan(&[a, b], s);
    f64_compare::f64_compare(a, b, signal == NanSignal::Quiet, s)
}

/// FRINT* (double). Arm FPRoundInt.
pub(crate) fn f64_round_to_int(a: u64, rounding: FpRounding, report: InexactReport, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    f64_round_to_int::f64_round_to_int(a, rounding.softfloat_mode(), report == InexactReport::Raise, s)
}

// ============================================================
// Precision conversions (FCVT), FPCR.RMode rounding. Arm FPConvert: a
// single- or double-precision denormal operand is reported under FPCR.AH =
// 1 (FPProcessDenorm), a half-precision one never.
// ============================================================

/// FCVT Dd, Sn. Arm FPConvert.
pub(crate) fn f32_to_f64(a: u32, fp: &mut ArmFpStatus) -> u64 {
    let s = &mut fp.status;
    let a = f32_flush_input(a, s);
    f32_report_denormals(&[a], s);
    f32_to_f64::f32_to_f64(a, s)
}

/// FCVT Sd, Dn. Arm FPConvert.
pub(crate) fn f64_to_f32(a: u64, fp: &mut ArmFpStatus) -> u32 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    f64_report_denormals(&[a], s);
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
    f32_report_denormals(&[a], s);
    f32_to_f16::f32_to_f16(a, s)
}

/// FCVT Hd, Dn. Arm FPConvert (IEEE half precision; the result is never
/// flushed).
pub(crate) fn f64_to_f16(a: u64, fp: &mut ArmFpStatus) -> u16 {
    let s = &mut fp.status;
    let a = f64_flush_input(a, s);
    f64_report_denormals(&[a], s);
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
