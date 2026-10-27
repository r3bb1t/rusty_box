//! The SVE floating-point instructions FEAT_SME makes legal in Streaming
//! SVE mode, with SVE2's pairwise operations, odd-element conversions,
//! FLOGB and widening multiply-adds, and SVE's BFloat16 instructions.
//!
//! Each element operation is the one the scalar and Advanced SIMD
//! instructions use (`cpu/fp.rs`), under FPCR: AH, FIZ, FZ, FZ16, DN and
//! RMode apply as there, and the exceptions of the elements computed, the
//! active ones, accumulate into FPSR. FPCR.NEP does not apply (it merges
//! scalar results), and the precision conversions read FPCR.AHP as 0
//! (`FPConvertSVE`). A `size` of `00`, no floating-point element, is
//! UNDEFINED, which the decode raises before the enable check.
//!
//! Provenance (R7): the Arm A64 ISA XML, release 2026-09, the instruction
//! pages each handler names and the shared pseudocode they call
//! (`FPReducePredicated`, `FPReduce`, `FPConvertSVE`, `FPScale`,
//! `FPClampScale`, `FPLogB`, `FPMulAddH`, `BFMulAddH`, `BFDotAdd`).

use super::{segment_base, Esize, Pv, Zv};
use crate::cpu::exec::{bit, bits, sext, ExecResult, Flow};
use crate::cpu::fp::{
    bf16_mul_add_long, bf16_neg, fp_binary, fp_compare, fp_compare_holds, fp_convert, fp_convert_round_odd, fp_log_b, fp_mul_add,
    fp_scale, fp_to_int, fp_unary, int_to_fp, FpBinary, FpCompare, FpConversion, FpEnv, FpType, FpUnary, IntWidth,
};
use crate::cpu::za::PairFormat;
use crate::cpu::Cpu;
use crate::softfloat::{self as sf, FpOrdering, FpRounding, FusedMulAdd, InexactReport, NanSignal};

/// The floating-point element of `size`: `None` for `00`.
pub(super) const fn fp_type(size: u32) -> Option<FpType> {
    match size & 3 {
        0b01 => Some(FpType::Half),
        0b10 => Some(FpType::Single),
        0b11 => Some(FpType::Double),
        _ => None,
    }
}

/// The element size of a floating-point type.
pub(super) const fn esize(ty: FpType) -> Esize {
    match ty {
        FpType::Half => Esize::H,
        FpType::Single => Esize::S,
        FpType::Double => Esize::D,
    }
}

/// The multiply-add form a negation of the multiplicand (`op1_neg`) and of
/// the addend (`op3_neg`) make: Arm FPMulAdd after FPNeg.
const fn mul_add_form(negate_multiplicand: bool, negate_addend: bool) -> FusedMulAdd {
    match (negate_multiplicand, negate_addend) {
        (false, false) => FusedMulAdd::Fmadd,
        (true, false) => FusedMulAdd::Fmsub,
        (true, true) => FusedMulAdd::Fnmadd,
        (false, true) => FusedMulAdd::Fnmsub,
    }
}

/// A predicated arithmetic operation of `sve_fp_2op_p_zds` or
/// `sve_fp_2op_i_p_zds` on `(Zdn, operand2)`.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy)]
enum PredicatedOp {
    Forward(FpBinary),
    /// FSUBR, FDIVR: the operation with its operands swapped.
    Reversed(FpBinary),
    /// FSCALE: `Zdn` × 2^`SInt(Zm)`.
    Scale,
}

/// The second operand of a predicated arithmetic instruction.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy)]
enum Operand2 {
    Vector(u32),
    Immediate(u64),
}

/// The comparison an `sve_fp_3op_p_pd` or `sve_fp_2op_p_pd` encoding
/// makes of `(Zn, Zm)` or `(Zn, #0.0)`.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy)]
enum Comparison {
    Eq,
    Ne,
    Ge,
    Gt,
    /// `b > a`.
    Lt,
    /// `b >= a`.
    Le,
    Unordered,
    AbsGe,
    AbsGt,
}

impl Comparison {
    /// Arm FPCompareEQ, NE, GE, GT and UN: whether `a op b` holds.
    fn holds(self, ty: FpType, a: u64, b: u64, env: &mut FpEnv) -> bool {
        match self {
            Self::Eq => fp_compare_holds(FpCompare::Eq, ty, a, b, env),
            Self::Ne => !fp_compare_holds(FpCompare::Eq, ty, a, b, env),
            Self::Ge => fp_compare_holds(FpCompare::Ge, ty, a, b, env),
            Self::Gt => fp_compare_holds(FpCompare::Gt, ty, a, b, env),
            Self::Lt => fp_compare_holds(FpCompare::Gt, ty, b, a, env),
            Self::Le => fp_compare_holds(FpCompare::Ge, ty, b, a, env),
            Self::Unordered => fp_compare(ty, a, b, NanSignal::Quiet, env) == FpOrdering::Unordered,
            Self::AbsGe => fp_compare_holds(FpCompare::AbsGe, ty, a, b, env),
            Self::AbsGt => fp_compare_holds(FpCompare::AbsGt, ty, a, b, env),
        }
    }
}

/// A reduction of `sve_fp_fast_red`: its operation and the identity an
/// inactive element contributes.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy)]
enum Reduction {
    Add,
    MaxNum,
    MinNum,
    Max,
    Min,
}

impl Reduction {
    const fn op(self) -> FpBinary {
        match self {
            Self::Add => FpBinary::Add,
            Self::MaxNum => FpBinary::MaxNum,
            Self::MinNum => FpBinary::MinNum,
            Self::Max => FpBinary::Max,
            Self::Min => FpBinary::Min,
        }
    }

    /// FADDV's `FPZero('0')`, FMAXNMV's and FMINNMV's `FPDefaultNaN`,
    /// FMAXV's `FPInfinity('1')`, FMINV's `FPInfinity('0')`.
    fn identity(self, ty: FpType, env: &FpEnv) -> u64 {
        match self {
            Self::Add => 0,
            Self::MaxNum | Self::MinNum => env.default_nan(ty),
            Self::Max => ty.infinity(true),
            Self::Min => ty.infinity(false),
        }
    }
}

/// A predicated unary operation of `sve_fp_unary`, on elements of
/// `container` bits whose low bits it reads and whose result it writes
/// extended.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy)]
enum UnaryOp {
    /// FRINT*, FRECPX, FSQRT: within one precision.
    Same(FpType, FpUnary),
    /// FLOGB.
    LogB(FpType),
    /// FCVT (`FPConvertSVE`), zero-extended.
    Convert(Esize, FpConversion),
    /// FCVTX: double to single, rounding to odd.
    ConvertRoundOdd,
    /// BFCVT: single to BFloat16 (`FPConvertBF`).
    ConvertBf16,
    /// FCVTZS, FCVTZU: to an integer, rounding toward zero, then sign- or
    /// zero-extended to the container.
    ToInt { shape: IntConversion, signed: bool },
    /// SCVTF, UCVTF: from an integer, zero-extended.
    FromInt { shape: IntConversion, signed: bool },
}

/// What an integer conversion of `sve_fp_2op_p_zd_c` or `_d` converts:
/// its container, its floating-point type and its integer's width.
#[derive(Debug, Clone, Copy)]
struct IntConversion {
    container: Esize,
    ty: FpType,
    width: IntWidth,
}

impl IntConversion {
    /// The conversion `size` and `opc<2:1>` name, the same in SCVTF, UCVTF,
    /// FCVTZS and FCVTZU; `None` where they name none.
    const fn of(size: u32, opc: u32) -> Option<Self> {
        let (container, ty, width) = match (size, opc) {
            (0b10, 0b10) => (Esize::S, FpType::Single, IntWidth::Bits32),
            (0b11, 0b00) => (Esize::D, FpType::Double, IntWidth::Bits32),
            (0b11, 0b10) => (Esize::D, FpType::Single, IntWidth::Bits64),
            (0b11, 0b11) => (Esize::D, FpType::Double, IntWidth::Bits64),
            (0b01, 0b01) => (Esize::H, FpType::Half, IntWidth::OfElement),
            (0b01, 0b10) => (Esize::S, FpType::Half, IntWidth::Bits32),
            (0b01, 0b11) => (Esize::D, FpType::Half, IntWidth::Bits64),
            _ => return None,
        };
        Some(Self { container, ty, width })
    }
}

impl UnaryOp {
    /// The element size the operation's predicate and container use.
    const fn container(self) -> Esize {
        match self {
            Self::Same(ty, _) | Self::LogB(ty) => esize(ty),
            Self::Convert(container, _) => container,
            Self::ConvertRoundOdd => Esize::D,
            Self::ConvertBf16 => Esize::S,
            Self::ToInt { shape, .. } | Self::FromInt { shape, .. } => shape.container,
        }
    }

    fn apply(self, a: u64, env: &mut FpEnv) -> u64 {
        match self {
            Self::Same(ty, op) => fp_unary(op, ty, a, env),
            Self::LogB(ty) => fp_log_b(ty, a, env),
            Self::Convert(_, conversion) => fp_convert(conversion, a, env),
            Self::ConvertRoundOdd => fp_convert_round_odd(a, env),
            Self::ConvertBf16 => {
                let rounding = env.rounding();
                u64::from(sf::f32_to_bf16(a as u32, rounding, &mut env.status))
            }
            Self::ToInt { shape, signed } => {
                let r = fp_to_int(shape.ty, a, 0, FpRounding::Zero, signed, shape.width, env);
                if signed {
                    sext(r, shape.width.bits(shape.ty))
                } else {
                    r
                }
            }
            Self::FromInt { shape, signed } => int_to_fp(shape.ty, a, 0, signed, shape.width, env),
        }
    }
}

impl Cpu {
    /// FADD, FSUB, FMUL, FSUBR, FMAXNM, FMINNM, FMAX, FMIN, FABD, FSCALE,
    /// FMULX, FDIVR, FDIV of vectors (`01100101 size 00 opc 100 Pg Zm
    /// Zdn`), and FADD to FMIN with an immediate (`01100101 size 011 opc
    /// 100 Pg 0000 i1 Zdn`: 0.5 or 1.0, FMUL's 0.5 or 2.0, the extrema's
    /// 0.0 or 1.0); predicated, the inactive elements of Zdn unchanged.
    pub(crate) fn exec_sve_fp_arith_pred(&mut self, pc: u64, insn: u32) -> ExecResult {
        let Some(ty) = fp_type(bits(insn, 23, 22)) else {
            return self.undefined(pc, insn);
        };
        let immediate = bit(insn, 20);
        let opc = if immediate { bits(insn, 18, 16) } else { bits(insn, 19, 16) };
        let op = match opc {
            0b0000 => PredicatedOp::Forward(FpBinary::Add),
            0b0001 => PredicatedOp::Forward(FpBinary::Sub),
            0b0010 => PredicatedOp::Forward(FpBinary::Mul),
            0b0011 => PredicatedOp::Reversed(FpBinary::Sub),
            0b0100 => PredicatedOp::Forward(FpBinary::MaxNum),
            0b0101 => PredicatedOp::Forward(FpBinary::MinNum),
            0b0110 => PredicatedOp::Forward(FpBinary::Max),
            0b0111 => PredicatedOp::Forward(FpBinary::Min),
            0b1000 => PredicatedOp::Forward(FpBinary::AbsDiff),
            0b1001 => PredicatedOp::Scale,
            0b1010 => PredicatedOp::Forward(FpBinary::MulX),
            0b1100 => PredicatedOp::Reversed(FpBinary::Div),
            0b1101 => PredicatedOp::Forward(FpBinary::Div),
            _ => return self.undefined(pc, insn),
        };
        let operand2 = if immediate {
            let i1 = bit(insn, 5);
            let imm = match (opc, i1) {
                (0b010, true) => ty.two(),
                (0b100..=0b111, false) => 0,
                (_, true) => ty.one(),
                (_, false) => ty.point_five(),
            };
            Operand2::Immediate(imm)
        } else {
            Operand2::Vector(bits(insn, 9, 5))
        };
        let (g, dn) = (bits(insn, 12, 10), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let es = esize(ty);
        let mask = self.p(g, vl);
        let operand1 = self.z(dn, vl);
        let vector2 = match operand2 {
            Operand2::Vector(m) => self.z(m, vl),
            Operand2::Immediate(_) => Zv::ZERO,
        };
        let mut env = self.fp_env();
        let mut result = operand1;
        for e in (0..es.elements(vl)).filter(|&e| mask.active(e, es)) {
            let a = operand1.get(e, es);
            let b = match operand2 {
                Operand2::Vector(_) => vector2.get(e, es),
                Operand2::Immediate(imm) => imm,
            };
            let r = match op {
                PredicatedOp::Forward(f) => fp_binary(f, ty, a, b, &mut env),
                PredicatedOp::Reversed(f) => fp_binary(f, ty, b, a, &mut env),
                PredicatedOp::Scale => fp_scale(ty, a, es.signed(b), &mut env),
            };
            result.set(e, es, r);
        }
        self.fp_commit(&env);
        self.set_z(dn, vl, &result);
        Ok(Flow::Next)
    }

    /// FADD, FSUB, FMUL, FRECPS, FRSQRTS, unpredicated: `01100101 size 0
    /// Zm 000 opc Zn Zd`.
    pub(crate) fn exec_sve_fp_arith_unpred(&mut self, pc: u64, insn: u32) -> ExecResult {
        let Some(ty) = fp_type(bits(insn, 23, 22)) else {
            return self.undefined(pc, insn);
        };
        let op = match bits(insn, 12, 10) {
            0b000 => FpBinary::Add,
            0b001 => FpBinary::Sub,
            0b010 => FpBinary::Mul,
            0b110 => FpBinary::RecipStep,
            0b111 => FpBinary::RSqrtStep,
            _ => return self.undefined(pc, insn),
        };
        let (m, n, d) = (bits(insn, 20, 16), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let es = esize(ty);
        let operand1 = self.z(n, vl);
        let operand2 = self.z(m, vl);
        let mut env = self.fp_env();
        let mut result = Zv::ZERO;
        for e in 0..es.elements(vl) {
            result.set(e, es, fp_binary(op, ty, operand1.get(e, es), operand2.get(e, es), &mut env));
        }
        self.fp_commit(&env);
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// FADDV, FMAXNMV, FMINNMV, FMAXV, FMINV: `01100101 size 000 opc 001
    /// Pg Zn Vd`. Arm FPReducePredicated: the inactive elements are the
    /// operation's identity, and the vector halves recursively, each
    /// result `op(lower half's, upper half's)`; the scalar is written to
    /// Vd, the rest of Zd zeroed.
    pub(crate) fn exec_sve_fp_reduce(&mut self, pc: u64, insn: u32) -> ExecResult {
        let Some(ty) = fp_type(bits(insn, 23, 22)) else {
            return self.undefined(pc, insn);
        };
        let reduction = match bits(insn, 18, 16) {
            0b000 => Reduction::Add,
            0b100 => Reduction::MaxNum,
            0b101 => Reduction::MinNum,
            0b110 => Reduction::Max,
            0b111 => Reduction::Min,
            _ => return self.undefined(pc, insn),
        };
        let (g, n, d) = (bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let es = esize(ty);
        let mask = self.p(g, vl);
        let operand = self.z(n, vl);
        let mut env = self.fp_env();
        let identity = reduction.identity(ty, &env);
        let mut lanes = [0u64; super::MAX_VL_BYTES / 2];
        let mut count = es.elements(vl);
        for (e, lane) in lanes.iter_mut().enumerate().take(count) {
            *lane = if mask.active(e, es) { operand.get(e, es) } else { identity };
        }
        // The element count is a power of two (CurrentVL is), so pairing
        // neighbours level by level is Arm's recursive halving.
        while count > 1 {
            for i in 0..count / 2 {
                lanes[i] = fp_binary(reduction.op(), ty, lanes[2 * i], lanes[2 * i + 1], &mut env);
            }
            count /= 2;
        }
        self.fp_commit(&env);
        self.write_v(d as usize, u128::from(lanes[0]));
        Ok(Flow::Next)
    }

    /// FCMGE, FCMGT, FCMEQ, FCMNE, FCMUO, FACGE, FACGT of vectors
    /// (`01100101 size 0 Zm op 1 o2 Pg Zn o3 Pd`) and FCMGE, FCMGT, FCMLT,
    /// FCMLE, FCMEQ, FCMNE with zero (`01100101 size 0100 eq lt 001 Pg Zn
    /// ne Pd`): each active element of Pd the comparison's result, the
    /// inactive ones zero.
    pub(crate) fn exec_sve_fp_compare(&mut self, pc: u64, insn: u32) -> ExecResult {
        let Some(ty) = fp_type(bits(insn, 23, 22)) else {
            return self.undefined(pc, insn);
        };
        let with_zero = !bit(insn, 14);
        let comparison = if with_zero {
            match (bits(insn, 17, 17), bits(insn, 16, 16), bits(insn, 4, 4)) {
                (0, 0, 0) => Comparison::Ge,
                (1, 0, 0) => Comparison::Eq,
                (0, 0, 1) => Comparison::Gt,
                (0, 1, 0) => Comparison::Lt,
                (1, 1, 0) => Comparison::Ne,
                (0, 1, 1) => Comparison::Le,
                _ => return self.undefined(pc, insn),
            }
        } else {
            match (bits(insn, 15, 15), bits(insn, 13, 13), bits(insn, 4, 4)) {
                (0, 0, 0) => Comparison::Ge,
                (1, 0, 0) => Comparison::Unordered,
                (1, 0, 1) => Comparison::AbsGe,
                (1, 1, 1) => Comparison::AbsGt,
                (0, 0, 1) => Comparison::Gt,
                (0, 1, 0) => Comparison::Eq,
                (0, 1, 1) => Comparison::Ne,
                _ => return self.undefined(pc, insn),
            }
        };
        let (m, g, n, d) = (bits(insn, 20, 16), bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 3, 0));
        let vl = self.sve_check()?;
        let es = esize(ty);
        let mask = self.p(g, vl);
        let operand1 = self.z(n, vl);
        let operand2 = if with_zero { Zv::ZERO } else { self.z(m, vl) };
        let mut env = self.fp_env();
        let mut result = Pv::ZERO;
        for e in (0..es.elements(vl)).filter(|&e| mask.active(e, es)) {
            let holds = comparison.holds(ty, operand1.get(e, es), operand2.get(e, es), &mut env);
            result.set_element(e, es, holds);
        }
        self.fp_commit(&env);
        self.set_p(d, vl, &result);
        Ok(Flow::Next)
    }

    /// The predicated unary operations, merging (`01100101 xx 0 op0 xxx
    /// 101 Pg Zn Zd`): FRINTN, FRINTP, FRINTM, FRINTZ, FRINTA, FRINTX,
    /// FRINTI; FCVT between precisions, FCVTX and BFCVT; FRECPX and FSQRT;
    /// SCVTF and UCVTF; FLOGB, FCVTZS and FCVTZU. The inactive elements of
    /// Zd are unchanged.
    pub(crate) fn exec_sve_fp_unary_pred(&mut self, pc: u64, insn: u32) -> ExecResult {
        let mut env = self.fp_env().with_ieee_half();
        let Some(op) = Self::sve_fp_unary_op(insn, env.rounding()) else {
            return self.undefined(pc, insn);
        };
        let (g, n, d) = (bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let es = op.container();
        let mask = self.p(g, vl);
        let operand = self.z(n, vl);
        let mut result = self.z(d, vl);
        for e in (0..es.elements(vl)).filter(|&e| mask.active(e, es)) {
            result.set(e, es, op.apply(operand.get(e, es), &mut env));
        }
        self.fp_commit(&env);
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// The operation of an `sve_fp_unary` encoding (by bits 20:18), `None`
    /// where the decode is UNDEFINED. FRINTX and FRINTI round by
    /// `fpcr_rounding`, FPCR.RMode's.
    fn sve_fp_unary_op(insn: u32, fpcr_rounding: FpRounding) -> Option<UnaryOp> {
        let size = bits(insn, 23, 22);
        let signed = !bit(insn, 16);
        match bits(insn, 20, 18) {
            0b000 | 0b001 => {
                let ty = fp_type(size)?;
                let (rounding, report) = match bits(insn, 18, 16) {
                    0b000 => (FpRounding::TieEven, InexactReport::Suppress),
                    0b001 => (FpRounding::PosInf, InexactReport::Suppress),
                    0b010 => (FpRounding::NegInf, InexactReport::Suppress),
                    0b011 => (FpRounding::Zero, InexactReport::Suppress),
                    0b100 => (FpRounding::TieAway, InexactReport::Suppress),
                    0b110 => (fpcr_rounding, InexactReport::Raise),
                    0b111 => (fpcr_rounding, InexactReport::Suppress),
                    _ => return None,
                };
                Some(UnaryOp::Same(ty, FpUnary::RoundInt(rounding, report)))
            }
            0b010 => {
                let conversion = |container, from, to| {
                    FpConversion::between(from, to).map(|c| UnaryOp::Convert(container, c))
                };
                use crate::cpu::fp::Precision::{Double, Half, Single};
                match (size, bits(insn, 17, 16)) {
                    (0b00, 0b10) => Some(UnaryOp::ConvertRoundOdd),
                    (0b10, 0b00) => conversion(Esize::S, Single, Half),
                    (0b10, 0b01) => conversion(Esize::S, Half, Single),
                    (0b10, 0b10) => Some(UnaryOp::ConvertBf16),
                    (0b11, 0b00) => conversion(Esize::D, Double, Half),
                    (0b11, 0b01) => conversion(Esize::D, Half, Double),
                    (0b11, 0b10) => conversion(Esize::D, Double, Single),
                    (0b11, 0b11) => conversion(Esize::D, Single, Double),
                    _ => None,
                }
            }
            0b011 => {
                let ty = fp_type(size)?;
                match bits(insn, 17, 16) {
                    0b00 => Some(UnaryOp::Same(ty, FpUnary::RecpX)),
                    0b01 => Some(UnaryOp::Same(ty, FpUnary::Sqrt)),
                    _ => None,
                }
            }
            0b100 | 0b101 => {
                let shape = IntConversion::of(size, bits(insn, 18, 17))?;
                Some(UnaryOp::FromInt { shape, signed })
            }
            _ => {
                if size == 0b00 {
                    return match (bits(insn, 18, 17), signed) {
                        (0b00, _) => None,
                        (logb_size, true) => fp_type(logb_size).map(UnaryOp::LogB),
                        (_, false) => None,
                    };
                }
                let shape = IntConversion::of(size, bits(insn, 18, 17))?;
                Some(UnaryOp::ToInt { shape, signed })
            }
        }
    }

    /// FRECPE, FRSQRTE, unpredicated: `01100101 size 001 11 op 0011 00 Zn
    /// Zd`.
    pub(crate) fn exec_sve_fp_estimate(&mut self, pc: u64, insn: u32) -> ExecResult {
        let Some(ty) = fp_type(bits(insn, 23, 22)) else {
            return self.undefined(pc, insn);
        };
        let op = if bit(insn, 16) { FpUnary::RSqrtEstimate } else { FpUnary::RecipEstimate };
        let (n, d) = (bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let es = esize(ty);
        let operand = self.z(n, vl);
        let mut env = self.fp_env();
        let mut result = Zv::ZERO;
        for e in 0..es.elements(vl) {
            result.set(e, es, fp_unary(op, ty, operand.get(e, es), &mut env));
        }
        self.fp_commit(&env);
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// FMLA, FMLS, FNMLA, FNMLS (`01100101 size 1 Zm 0 opc Pg Zn Zda`:
    /// Zda accumulates) and FMAD, FMSB, FNMAD, FNMSB (`01100101 size 1 Za
    /// 1 opc Pg Zm Zdn`: Zdn is the multiplicand and the result), each
    /// one fused multiply-add per active element (FPNeg before FPMulAdd):
    /// `opc<1>` negates the addend (`op3_neg`), and `opc<0> != opc<1>`
    /// the multiplicand (`op1_neg`: FMLS and FNMLA, FMSB and FNMAD); the
    /// inactive elements of the destination unchanged.
    pub(crate) fn exec_sve_fp_mul_add(&mut self, pc: u64, insn: u32) -> ExecResult {
        let Some(ty) = fp_type(bits(insn, 23, 22)) else {
            return self.undefined(pc, insn);
        };
        let form = mul_add_form(bit(insn, 13) != bit(insn, 14), bit(insn, 14));
        let writes_addend = !bit(insn, 15);
        let (r16, g, r5, r0) = (bits(insn, 20, 16), bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 4, 0));
        // FMLA: addend Zda (r0), multiplicand Zn (r5), multiplier Zm (r16).
        // FMAD: addend Za (r16), multiplicand Zdn (r0), multiplier Zm (r5).
        let (addend, multiplicand, multiplier) = if writes_addend { (r0, r5, r16) } else { (r16, r0, r5) };
        let vl = self.sve_check()?;
        let es = esize(ty);
        let mask = self.p(g, vl);
        let a = self.z(addend, vl);
        let n = self.z(multiplicand, vl);
        let m = self.z(multiplier, vl);
        let mut env = self.fp_env();
        let mut result = self.z(r0, vl);
        for e in (0..es.elements(vl)).filter(|&e| mask.active(e, es)) {
            result.set(e, es, fp_mul_add(form, ty, a.get(e, es), n.get(e, es), m.get(e, es), &mut env));
        }
        self.fp_commit(&env);
        self.set_z(r0, vl, &result);
        Ok(Flow::Next)
    }

    /// FMLA, FMLS (indexed: `01100100 size 1 opc 0000 0 op Zn Zda`) and
    /// FMUL (indexed: `01100100 size 1 opc 0010 0 0 Zn Zd`), unpredicated:
    /// each element with the element `index` of its 128-bit segment of
    /// Zm. Half precision takes `i3h:i3l` (bits 22, 20:19) and Z0-Z7,
    /// single `i2` (20:19) and Z0-Z7, double `i1` (20) and Z0-Z15.
    pub(crate) fn exec_sve_fp_indexed(&mut self, insn: u32) -> ExecResult {
        let (ty, index, m) = match bits(insn, 23, 22) {
            0b00 | 0b01 => (FpType::Half, (bits(insn, 22, 22) << 2) | bits(insn, 20, 19), bits(insn, 18, 16)),
            0b10 => (FpType::Single, bits(insn, 20, 19), bits(insn, 18, 16)),
            _ => (FpType::Double, bits(insn, 20, 20), bits(insn, 19, 16)),
        };
        let multiply_only = bit(insn, 13);
        let form = mul_add_form(bit(insn, 10), false);
        let (n, d) = (bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let es = esize(ty);
        let op1 = self.z(n, vl);
        let op2 = self.z(m, vl);
        let accumulator = self.z(d, vl);
        let mut env = self.fp_env();
        let mut result = Zv::ZERO;
        for e in 0..es.elements(vl) {
            let element2 = op2.get(segment_base(e, 16 / es.bytes()) + index as usize, es);
            let r = if multiply_only {
                fp_binary(FpBinary::Mul, ty, op1.get(e, es), element2, &mut env)
            } else {
                fp_mul_add(form, ty, accumulator.get(e, es), op1.get(e, es), element2, &mut env)
            };
            result.set(e, es, r);
        }
        self.fp_commit(&env);
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// FCMLA (predicated: `01100100 size 0 Zm 0 rot Pg Zn Zda`; indexed:
    /// `01100100 1 size<0> 1 opc 0001 rot Zn Zda`) and FCADD (`01100100
    /// size 00000 rot 100 Pg Zm Zdn`). Each pair of elements is a complex
    /// number (real, imaginary). FCMLA accumulates two fused
    /// multiply-adds of Zn's pair by Zm's pair rotated by `rot` × 90°
    /// (by index, the pair `index` of each 128-bit segment of Zm, half
    /// precision with `i2` and Z0-Z7, single with `i1` and Z0-Z15); FCADD
    /// adds Zm's pair rotated by 90° or 270°. A predicated pair's halves
    /// are each governed by its own element; inactive halves unchanged.
    pub(crate) fn exec_sve_fp_complex(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bit(insn, 21) {
            return self.exec_sve_fcmla_indexed(insn);
        }
        let Some(ty) = fp_type(bits(insn, 23, 22)) else {
            return self.undefined(pc, insn);
        };
        let es = esize(ty);
        if bit(insn, 15) {
            // FCADD: `rot = 0` subtracts the imaginary part (90°).
            let sub_i = !bit(insn, 16);
            let (g, m, dn) = (bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 4, 0));
            let vl = self.sve_check()?;
            let mask = self.p(g, vl);
            let operand1 = self.z(dn, vl);
            let operand2 = self.z(m, vl);
            let mut env = self.fp_env();
            let mut result = operand1;
            for p in 0..es.elements(vl) / 2 {
                let (real, imag) = (2 * p, 2 * p + 1);
                if mask.active(real, es) {
                    let mut element2 = operand2.get(imag, es);
                    if sub_i {
                        element2 = env.neg(ty, element2);
                    }
                    result.set(real, es, fp_binary(FpBinary::Add, ty, operand1.get(real, es), element2, &mut env));
                }
                if mask.active(imag, es) {
                    let mut element2 = operand2.get(real, es);
                    if !sub_i {
                        element2 = env.neg(ty, element2);
                    }
                    result.set(imag, es, fp_binary(FpBinary::Add, ty, operand1.get(imag, es), element2, &mut env));
                }
            }
            self.fp_commit(&env);
            self.set_z(dn, vl, &result);
            return Ok(Flow::Next);
        }
        let rot = bits(insn, 14, 13);
        let (m, g, n, da) = (bits(insn, 20, 16), bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let mask = self.p(g, vl);
        let op1 = self.z(n, vl);
        let op2 = self.z(m, vl);
        let mut env = self.fp_env();
        let mut result = self.z(da, vl);
        let rotation = Rotation::of(rot);
        for p in 0..es.elements(vl) / 2 {
            let (real, imag) = (2 * p, 2 * p + 1);
            let elt1_a = op1.get(2 * p + rotation.sel_a, es);
            if mask.active(real, es) {
                let mut elt2_a = op2.get(2 * p + rotation.sel_a, es);
                if rotation.neg_r {
                    elt2_a = env.neg(ty, elt2_a);
                }
                let r = fp_mul_add(FusedMulAdd::Fmadd, ty, result.get(real, es), elt1_a, elt2_a, &mut env);
                result.set(real, es, r);
            }
            if mask.active(imag, es) {
                let mut elt2_b = op2.get(2 * p + rotation.sel_b, es);
                if rotation.neg_i {
                    elt2_b = env.neg(ty, elt2_b);
                }
                let r = fp_mul_add(FusedMulAdd::Fmadd, ty, result.get(imag, es), elt1_a, elt2_b, &mut env);
                result.set(imag, es, r);
            }
        }
        self.fp_commit(&env);
        self.set_z(da, vl, &result);
        Ok(Flow::Next)
    }

    /// FCMLA (indexed), unpredicated; see [`Cpu::exec_sve_fp_complex`].
    fn exec_sve_fcmla_indexed(&mut self, insn: u32) -> ExecResult {
        let (ty, index, m) = if bit(insn, 22) {
            (FpType::Single, bits(insn, 20, 20), bits(insn, 19, 16))
        } else {
            (FpType::Half, bits(insn, 20, 19), bits(insn, 18, 16))
        };
        let rotation = Rotation::of(bits(insn, 11, 10));
        let (n, da) = (bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let es = esize(ty);
        let op1 = self.z(n, vl);
        let op2 = self.z(m, vl);
        let op3 = self.z(da, vl);
        let pairs_per_segment = 16 / (2 * es.bytes());
        let mut env = self.fp_env();
        let mut result = Zv::ZERO;
        for p in 0..es.elements(vl) / 2 {
            let s = segment_base(p, pairs_per_segment) + index as usize;
            let elt1_a = op1.get(2 * p + rotation.sel_a, es);
            let mut elt2_a = op2.get(2 * s + rotation.sel_a, es);
            let mut elt2_b = op2.get(2 * s + rotation.sel_b, es);
            if rotation.neg_r {
                elt2_a = env.neg(ty, elt2_a);
            }
            if rotation.neg_i {
                elt2_b = env.neg(ty, elt2_b);
            }
            let real = fp_mul_add(FusedMulAdd::Fmadd, ty, op3.get(2 * p, es), elt1_a, elt2_a, &mut env);
            let imag = fp_mul_add(FusedMulAdd::Fmadd, ty, op3.get(2 * p + 1, es), elt1_a, elt2_b, &mut env);
            result.set(2 * p, es, real);
            result.set(2 * p + 1, es, imag);
        }
        self.fp_commit(&env);
        self.set_z(da, vl, &result);
        Ok(Flow::Next)
    }

    /// FADDP, FMAXNMP, FMINNMP, FMAXP, FMINP (`01100100 size 010 opc 100
    /// Pg Zm Zdn`): each active even element the operation on the pair
    /// of Zdn it starts, each active odd one on the pair of Zm it ends;
    /// the inactive elements unchanged.
    pub(crate) fn exec_sve_fp_pairwise(&mut self, pc: u64, insn: u32) -> ExecResult {
        let Some(ty) = fp_type(bits(insn, 23, 22)) else {
            return self.undefined(pc, insn);
        };
        let op = match bits(insn, 18, 16) {
            0b000 => FpBinary::Add,
            0b100 => FpBinary::MaxNum,
            0b101 => FpBinary::MinNum,
            0b110 => FpBinary::Max,
            0b111 => FpBinary::Min,
            _ => return self.undefined(pc, insn),
        };
        let (g, m, dn) = (bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let es = esize(ty);
        let mask = self.p(g, vl);
        let operand1 = self.z(dn, vl);
        let operand2 = self.z(m, vl);
        let mut env = self.fp_env();
        let mut result = operand1;
        for e in (0..es.elements(vl)).filter(|&e| mask.active(e, es)) {
            let (a, b) = if e % 2 == 0 {
                (operand1.get(e, es), operand1.get(e + 1, es))
            } else {
                (operand2.get(e - 1, es), operand2.get(e, es))
            };
            result.set(e, es, fp_binary(op, ty, a, b, &mut env));
        }
        self.fp_commit(&env);
        self.set_z(dn, vl, &result);
        Ok(Flow::Next)
    }

    /// FCVTNT, FCVTXNT, BFCVTNT (narrowing into the odd half-elements) and
    /// FCVTLT (widening from them): `01100100 opc 0010 opc2 101 Pg Zn Zd`,
    /// merging. FCVTNT and FCVTLT convert by `FPConvertSVE`, FCVTXNT
    /// rounds to odd, BFCVTNT is `FPConvertBF`.
    pub(crate) fn exec_sve_fp_convert_odd(&mut self, pc: u64, insn: u32) -> ExecResult {
        use crate::cpu::fp::Precision::{Double, Half, Single};
        let narrow = |container, from, to| FpConversion::between(from, to).map(|c| (container, OddConvert::Narrow(c)));
        let widen = |container, from, to| FpConversion::between(from, to).map(|c| (container, OddConvert::Widen(c)));
        let decoded = match (bits(insn, 23, 22), bits(insn, 17, 16)) {
            (0b00, 0b10) => Some((Esize::D, OddConvert::NarrowRoundOdd)),
            (0b10, 0b00) => narrow(Esize::S, Single, Half),
            (0b10, 0b01) => widen(Esize::S, Half, Single),
            (0b10, 0b10) => Some((Esize::S, OddConvert::NarrowBf16)),
            (0b11, 0b10) => narrow(Esize::D, Double, Single),
            (0b11, 0b11) => widen(Esize::D, Single, Double),
            _ => None,
        };
        let Some((es, convert)) = decoded else {
            return self.undefined(pc, insn);
        };
        let half = match es {
            Esize::D => Esize::S,
            _ => Esize::H,
        };
        let (g, n, d) = (bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let mask = self.p(g, vl);
        let operand = self.z(n, vl);
        let mut env = self.fp_env().with_ieee_half();
        let mut result = self.z(d, vl);
        for e in (0..es.elements(vl)).filter(|&e| mask.active(e, es)) {
            match convert {
                OddConvert::Narrow(c) => result.set(2 * e + 1, half, fp_convert(c, operand.get(e, es), &mut env)),
                OddConvert::NarrowRoundOdd => {
                    result.set(2 * e + 1, half, fp_convert_round_odd(operand.get(e, es), &mut env));
                }
                OddConvert::NarrowBf16 => {
                    let rounding = env.rounding();
                    let r = sf::f32_to_bf16(operand.get(e, es) as u32, rounding, &mut env.status);
                    result.set(2 * e + 1, half, u64::from(r));
                }
                OddConvert::Widen(c) => result.set(e, es, fp_convert(c, operand.get(2 * e + 1, half), &mut env)),
            }
        }
        self.fp_commit(&env);
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// The widening multiply-adds and dot products, unpredicated: FMLALB,
    /// FMLALT, FMLSLB, FMLSLT and BFMLALB, BFMLALT, BFMLSLB, BFMLSLT
    /// (`01100100 1 o2 1 Zm 10 op 00 T Zn Zda`; indexed `01100100 1 o2 1
    /// i3h Zm 01 op 0 i3l T Zn Zda`), and FDOT and BFDOT (`01100100 0 op 1
    /// Zm 100000 Zn Zda`; indexed `01100100 0 op 1 i2 Zm 010000 Zn Zda`).
    /// Each 32-bit element of Zda accumulates: for the long forms, the
    /// product of the even (`B`) or odd (`T`) 16-bit element of its pair in
    /// Zn, negated by `op` (FPNeg, BFNeg), with the same of Zm, or by index
    /// the 16-bit element `i3h:i3l` of the segment (Arm FPMulAddH,
    /// BFMulAddH); for the dot products, the dot product of its pair with
    /// Zm's, or by index the pair `i2` of the segment, of half-precision
    /// values in two roundings (FPDotAdd) or of BFloat16 ones (BFDotAdd).
    pub(crate) fn exec_sve_fp_widening(&mut self, insn: u32) -> ExecResult {
        let indexed = !bit(insn, 15);
        let dot = !bit(insn, 23);
        let bf16 = bit(insn, 22);
        let subtract = bit(insn, 13);
        let top = usize::from(bit(insn, 10));
        let (m, index) = if indexed {
            let index = if dot { bits(insn, 20, 19) } else { (bits(insn, 20, 19) << 1) | bits(insn, 11, 11) };
            (bits(insn, 18, 16), index as usize)
        } else {
            (bits(insn, 20, 16), 0)
        };
        let (n, da) = (bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let op1 = self.z(n, vl);
        let op2 = self.z(m, vl);
        let op3 = self.z(da, vl);
        let mut env = self.fp_env();
        let mut result = Zv::ZERO;
        for e in 0..Esize::S.elements(vl) {
            let acc = op3.get(e, Esize::S) as u32;
            let base = segment_base(e, 4);
            let sum = if dot {
                let pair = if indexed { base + index } else { e };
                let row = [0, 1].map(|k| op1.get(2 * e + k, Esize::H) as u16);
                let col = [0, 1].map(|k| op2.get(2 * pair + k, Esize::H) as u16);
                if bf16 {
                    sf::bf16_dot_add(acc, row[0], row[1], col[0], col[1], env.handling())
                } else {
                    PairFormat::Half.dot(acc, row, col, &mut env)
                }
            } else {
                let element1 = op1.get(2 * e + top, Esize::H) as u16;
                let s = if indexed { 2 * base + index } else { 2 * e + top };
                let element2 = op2.get(s, Esize::H) as u16;
                if bf16 {
                    let element1 = if subtract { bf16_neg(element1, &env) } else { element1 };
                    bf16_mul_add_long(acc, element1, element2, &mut env)
                } else {
                    let form = mul_add_form(subtract, false);
                    sf::f16_mul_add_long(form, acc, element1, element2, &mut env.status)
                }
            };
            result.set(e, Esize::S, u64::from(sum));
        }
        self.fp_commit(&env);
        self.set_z(da, vl, &result);
        Ok(Flow::Next)
    }
}

/// A conversion between an element and the odd half-element of its
/// container (FCVTNT, FCVTXNT, BFCVTNT, FCVTLT).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy)]
enum OddConvert {
    /// The element converted into the odd half-element.
    Narrow(FpConversion),
    /// FCVTXNT: double to single, rounding to odd.
    NarrowRoundOdd,
    /// BFCVTNT: single to BFloat16.
    NarrowBf16,
    /// FCVTLT: the odd half-element converted to the element.
    Widen(FpConversion),
}

/// FCMLA's `rot`: which element of Zn's pair both halves take (`sel_a`),
/// Zm's element for the imaginary half (`sel_b`), and which of Zm's
/// elements are negated.
#[derive(Debug, Clone, Copy)]
struct Rotation {
    sel_a: usize,
    sel_b: usize,
    neg_r: bool,
    neg_i: bool,
}

impl Rotation {
    const fn of(rot: u32) -> Self {
        let (r0, r1) = (rot & 1, (rot >> 1) & 1);
        Self {
            sel_a: r0 as usize,
            sel_b: (r0 ^ 1) as usize,
            neg_r: r0 != r1,
            neg_i: r1 == 1,
        }
    }
}
