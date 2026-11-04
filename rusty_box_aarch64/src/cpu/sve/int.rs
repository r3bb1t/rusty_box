//! SVE integer arithmetic: the predicated and unpredicated binary, unary,
//! shift, multiply-add and reduction instructions, the element counts and
//! the instructions with an immediate. Each handler names the Arm pages it
//! is.

use super::{
    decode_pred_count, int, mul_high, sat_add_scalar, sat_doubling_mul_high, sat_q, Esize, Zv,
};
use crate::cpu::crypto::polynomial_mult;
use crate::cpu::dp::{decode_bit_masks, ror};
use crate::cpu::exec::{bit, bits, sext, ExecResult, Flow};
use crate::cpu::features::VecLen;
use crate::cpu::fp::FpType;
use crate::cpu::simd::vfp_expand_imm;
use crate::cpu::{Cpu, Levels};

/// The shifts the SVE instructions apply to an element: Arm `ASR`, `LSR`
/// and `LSL`, whose amount may be the whole element or more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shift {
    Asr,
    Lsr,
    Lsl,
}

impl Shift {
    /// The element `value` of `es` shifted by `amount`.
    fn apply(self, value: u64, amount: u64, es: Esize) -> u64 {
        let width = u64::from(es.bits());
        match self {
            Self::Asr => {
                let amount = amount.min(width - 1) as u32;
                (es.signed(value) >> amount) as u64 & es.mask()
            }
            Self::Lsr if amount >= width => 0,
            Self::Lsr => (value & es.mask()) >> amount,
            Self::Lsl if amount >= width => 0,
            Self::Lsl => (value << amount) & es.mask(),
        }
    }
}

/// The two-operand operations of `sve_int_pred_bin`, by `op0:opc`
/// (bits 20:16); ADD, MAX and MIN are also SVE2's pairwise operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BinOp {
    Add,
    Sub,
    SubReversed,
    Max { unsigned: bool },
    Min { unsigned: bool },
    AbsDiff { unsigned: bool },
    Mul,
    MulHigh { unsigned: bool },
    Div { unsigned: bool },
    DivReversed { unsigned: bool },
    Orr,
    Eor,
    And,
    Bic,
}

impl BinOp {
    const fn from_field(op: u32) -> Option<Self> {
        Some(match op {
            0b00000 => Self::Add,
            0b00001 => Self::Sub,
            0b00011 => Self::SubReversed,
            0b01000 => Self::Max { unsigned: false },
            0b01001 => Self::Max { unsigned: true },
            0b01010 => Self::Min { unsigned: false },
            0b01011 => Self::Min { unsigned: true },
            0b01100 => Self::AbsDiff { unsigned: false },
            0b01101 => Self::AbsDiff { unsigned: true },
            0b10000 => Self::Mul,
            0b10010 => Self::MulHigh { unsigned: false },
            0b10011 => Self::MulHigh { unsigned: true },
            0b10100 => Self::Div { unsigned: false },
            0b10101 => Self::Div { unsigned: true },
            0b10110 => Self::DivReversed { unsigned: false },
            0b10111 => Self::DivReversed { unsigned: true },
            0b11000 => Self::Orr,
            0b11001 => Self::Eor,
            0b11010 => Self::And,
            0b11011 => Self::Bic,
            _ => return None,
        })
    }

    /// The operation on elements `a` and `b` of `es`.
    pub(super) fn apply(self, a: u64, b: u64, es: Esize) -> u64 {
        let m = es.mask();
        match self {
            Self::Add => a.wrapping_add(b) & m,
            Self::Sub => a.wrapping_sub(b) & m,
            Self::SubReversed => b.wrapping_sub(a) & m,
            Self::Max { unsigned } => int(a, es, unsigned).max(int(b, es, unsigned)) as u64 & m,
            Self::Min { unsigned } => int(a, es, unsigned).min(int(b, es, unsigned)) as u64 & m,
            Self::AbsDiff { unsigned } => {
                (int(a, es, unsigned) - int(b, es, unsigned)).unsigned_abs() as u64 & m
            }
            Self::Mul => a.wrapping_mul(b) & m,
            Self::MulHigh { unsigned } => mul_high(a, b, es, unsigned),
            Self::Div { unsigned } => divide(a, b, es, unsigned),
            Self::DivReversed { unsigned } => divide(b, a, es, unsigned),
            Self::Orr => a | b,
            Self::Eor => a ^ b,
            Self::And => a & b,
            Self::Bic => a & !b,
        }
    }
}

/// SDIV and UDIV's element: the quotient rounded towards zero, and zero
/// for a zero divisor.
fn divide(dividend: u64, divisor: u64, es: Esize, unsigned: bool) -> u64 {
    let (n, d) = (int(dividend, es, unsigned), int(divisor, es, unsigned));
    if d == 0 {
        0
    } else {
        (n / d) as u64 & es.mask()
    }
}

/// The reductions of `sve_int_reduce_0`, `_1` and `_2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reduction {
    SignedAdd,
    UnsignedAdd,
    Max { unsigned: bool },
    Min { unsigned: bool },
    Orr,
    Eor,
    And,
}

/// The shifts by immediate, predicated (`sve_int_bin_pred_shift_0`), by
/// `opc:L:U`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShiftImmediate {
    Asr,
    Lsr,
    Lsl,
    /// ASRD: an arithmetic shift rounding towards zero.
    Asrd,
    Sqshl,
    Uqshl,
    Srshr,
    Urshr,
    Sqshlu,
}

/// The unary operations, predicated (`sve_int_un_pred_arit_0` and `_1`), by
/// `op0<0>:opc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unary {
    /// SXTB, SXTH, SXTW: the low `Esize` bits sign-extended.
    SignExtend(Esize),
    /// UXTB, UXTH, UXTW: the low `Esize` bits zero-extended.
    ZeroExtend(Esize),
    Abs,
    Neg,
    Cls,
    Clz,
    Cnt,
    Cnot,
    Fabs,
    Fneg,
    Not,
}

/// The unpredicated multiplies (`sve_int_mul_b`, `sve_int_sqdmulh`), by
/// `op0<1:0>:opc` (bits 12:10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MulUnpredicated {
    Mul,
    Pmul,
    MulHigh {
        unsigned: bool,
    },
    /// SQDMULH, SQRDMULH (`rounding`).
    DoublingHigh {
        rounding: bool,
    },
}

/// The forms of `sve_countelt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementCount {
    /// SQINCx, UQINCx, SQDECx, UQDECx (vector).
    SaturatingVector,
    /// INCx, DECx (vector).
    Vector,
    /// CNTx.
    Count,
    /// INCx, DECx (scalar).
    Scalar,
    /// SQINCx, UQINCx, SQDECx, UQDECx (scalar), 64-bit with `wide`.
    SaturatingScalar { wide: bool },
}

/// The amounts an element shift by immediate takes from `tsize:imm3`.
#[derive(Debug, Clone, Copy)]
pub(super) struct ShiftAmounts {
    /// A right shift's `2 * esize - UInt(tsize:imm3)`, 1 to `esize`.
    pub(super) right: u64,
    /// A left shift's `UInt(tsize:imm3) - esize`, 0 to `esize - 1`.
    pub(super) left: u64,
}

/// The shift amounts of `tsize:imm3` for elements of `es`, the size
/// `tsize`'s highest set bit gives (the source elements' of the long and
/// narrowing shifts).
pub(super) fn shift_amounts(tsize_imm3: u32, es: Esize) -> ShiftAmounts {
    let encoded = u64::from(tsize_imm3);
    let width = u64::from(es.bits());
    ShiftAmounts {
        right: 2 * width - encoded,
        left: encoded - width,
    }
}

impl<L: Levels> Cpu<L> {
    /// ADD, SUB, SUBR, SMAX, UMAX, SMIN, UMIN, SABD, UABD, MUL, SMULH,
    /// UMULH, SDIV, UDIV, SDIVR, UDIVR, ORR, EOR, AND, BIC (vectors,
    /// predicated): `00000100 size 0 op0:opc 000 Pg Zm Zdn`. The divides
    /// take 32- and 64-bit elements only.
    pub(crate) fn exec_sve_int_bin_pred(&mut self, pc: u64, insn: u32) -> ExecResult {
        let Some(op) = BinOp::from_field(bits(insn, 20, 16)) else {
            return self.undefined(pc, insn);
        };
        let size = bits(insn, 23, 22);
        if matches!(op, BinOp::Div { .. } | BinOp::DivReversed { .. }) && size < 0b10 {
            return self.undefined(pc, insn);
        }
        let es = Esize::from_size(size);
        let (g, m, dn) = (bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let mask = self.p(g, vl);
        let operand1 = self.z(dn, vl);
        let operand2 = self.z(m, vl);
        let mut result = operand1;
        for e in 0..es.elements(vl) {
            if mask.active(e, es) {
                result.set(
                    e,
                    es,
                    op.apply(operand1.get(e, es), operand2.get(e, es), es),
                );
            }
        }
        self.set_z(dn, vl, &result);
        Ok(Flow::Next)
    }

    /// SADDV, UADDV, SMAXV, UMAXV, SMINV, UMINV, ORV, EORV, ANDV:
    /// `00000100 size 0 op0:op:U 001 Pg Zn Vd`, the reduction of the active
    /// elements into `V<d>` (the adds as 64 bits, the rest as `esize`).
    /// SADDV takes 8- to 32-bit elements.
    pub(crate) fn exec_sve_int_reduce(&mut self, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 23, 22);
        let es = Esize::from_size(size);
        let op = match bits(insn, 20, 16) {
            0b00000 if size != 0b11 => Reduction::SignedAdd,
            0b00001 => Reduction::UnsignedAdd,
            0b01000 => Reduction::Max { unsigned: false },
            0b01001 => Reduction::Max { unsigned: true },
            0b01010 => Reduction::Min { unsigned: false },
            0b01011 => Reduction::Min { unsigned: true },
            0b11000 => Reduction::Orr,
            0b11001 => Reduction::Eor,
            0b11010 => Reduction::And,
            _ => return self.undefined(pc, insn),
        };
        let (g, n, d) = (bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let mask = self.p(g, vl);
        let operand = self.z(n, vl);
        let active = (0..es.elements(vl))
            .filter(|&e| mask.active(e, es))
            .map(|e| operand.get(e, es));
        let result = match op {
            Reduction::SignedAdd => {
                active.fold(0i64, |sum, x| sum.wrapping_add(es.signed(x))) as u64
            }
            Reduction::UnsignedAdd => active.fold(0u64, u64::wrapping_add),
            Reduction::Max { unsigned: false } => {
                active
                    .map(|x| es.signed(x))
                    .fold(es.signed(1 << (es.bits() - 1)), i64::max) as u64
                    & es.mask()
            }
            Reduction::Max { unsigned: true } => active.fold(0, u64::max),
            Reduction::Min { unsigned: false } => {
                active
                    .map(|x| es.signed(x))
                    .fold(es.signed(es.mask() >> 1), i64::min) as u64
                    & es.mask()
            }
            Reduction::Min { unsigned: true } => active.fold(es.mask(), u64::min),
            Reduction::Orr => active.fold(0, |r, x| r | x),
            Reduction::Eor => active.fold(0, |r, x| r ^ x),
            Reduction::And => active.fold(es.mask(), |r, x| r & x),
        };
        self.write_v(d as usize, u128::from(result));
        Ok(Flow::Next)
    }

    /// MOVPRFX (predicated), `00000100 size 010 00 M 001 Pg Zn Zd`, and
    /// MOVPRFX (unpredicated), `00000100 001 00000 101111 Zn Zd`: a copy of
    /// `Z<n>` (its active elements, the rest from `Z<d>` with `M = 1` or
    /// zero), which the instruction after it is to destroy.
    pub(crate) fn exec_sve_movprfx(&mut self, insn: u32) -> ExecResult {
        let (n, d) = (bits(insn, 9, 5), bits(insn, 4, 0));
        let predicated = !bit(insn, 21);
        let vl = self.sve_check()?;
        let operand1 = self.z(n, vl);
        let result = if predicated {
            let es = Esize::from_size(bits(insn, 23, 22));
            let mask = self.p(bits(insn, 12, 10), vl);
            let mut result = if bit(insn, 16) {
                self.z(d, vl)
            } else {
                Zv::ZERO
            };
            for e in 0..es.elements(vl) {
                if mask.active(e, es) {
                    result.set(e, es, operand1.get(e, es));
                }
            }
            result
        } else {
            operand1
        };
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// The shifts, predicated (`00000100 xx 0 op0 100 Pg ...`): ASR, LSR,
    /// LSL, ASRD, SQSHL, UQSHL, SRSHR, URSHR, SQSHLU by immediate (`op0 =
    /// 0x`, the element size and amount in `tszh:tszl:imm3`), ASR, LSR,
    /// LSL, ASRR, LSRR, LSLR by vector (`op0 = 10`) and ASR, LSR, LSL by
    /// wide elements (`op0 = 11`, 8- to 32-bit elements).
    pub(crate) fn exec_sve_shift_pred(&mut self, pc: u64, insn: u32) -> ExecResult {
        let g = bits(insn, 12, 10);
        let dn = bits(insn, 4, 0);
        match bits(insn, 20, 19) {
            0b00 | 0b01 => {
                let tsize = (bits(insn, 23, 22) << 2) | bits(insn, 9, 8);
                if tsize == 0 {
                    return self.undefined(pc, insn);
                }
                let es = Esize::from_tsize(tsize);
                let ShiftAmounts { right, left } =
                    shift_amounts((tsize << 3) | bits(insn, 7, 5), es);
                let op = match bits(insn, 19, 16) {
                    0b0000 => ShiftImmediate::Asr,
                    0b0001 => ShiftImmediate::Lsr,
                    0b0011 => ShiftImmediate::Lsl,
                    0b0100 => ShiftImmediate::Asrd,
                    0b0110 => ShiftImmediate::Sqshl,
                    0b0111 => ShiftImmediate::Uqshl,
                    0b1100 => ShiftImmediate::Srshr,
                    0b1101 => ShiftImmediate::Urshr,
                    0b1111 => ShiftImmediate::Sqshlu,
                    _ => return self.undefined(pc, insn),
                };
                let vl = self.sve_check()?;
                let mask = self.p(g, vl);
                let operand1 = self.z(dn, vl);
                let mut result = operand1;
                for e in 0..es.elements(vl) {
                    if !mask.active(e, es) {
                        continue;
                    }
                    let x = operand1.get(e, es);
                    let r = match op {
                        ShiftImmediate::Asr => Shift::Asr.apply(x, right, es),
                        ShiftImmediate::Lsr => Shift::Lsr.apply(x, right, es),
                        ShiftImmediate::Lsl => Shift::Lsl.apply(x, left, es),
                        ShiftImmediate::Asrd => {
                            let v = i128::from(es.signed(x));
                            let v = if v < 0 { v + (1 << right) - 1 } else { v };
                            (v >> right) as u64 & es.mask()
                        }
                        ShiftImmediate::Sqshl => sat_q(int(x, es, false) << left, es, false),
                        ShiftImmediate::Uqshl => sat_q(int(x, es, true) << left, es, true),
                        ShiftImmediate::Srshr => {
                            ((int(x, es, false) + (1 << (right - 1))) >> right) as u64 & es.mask()
                        }
                        ShiftImmediate::Urshr => {
                            ((int(x, es, true) + (1 << (right - 1))) >> right) as u64 & es.mask()
                        }
                        ShiftImmediate::Sqshlu => sat_q(int(x, es, false) << left, es, true),
                    };
                    result.set(e, es, r);
                }
                self.set_z(dn, vl, &result);
            }
            op0 => {
                let size = bits(insn, 23, 22);
                let wide = op0 == 0b11;
                if wide && size == 0b11 {
                    return self.undefined(pc, insn);
                }
                let es = Esize::from_size(size);
                let (reversed, shift) = match bits(insn, 18, 16) {
                    0b000 => (false, Shift::Asr),
                    0b001 => (false, Shift::Lsr),
                    0b011 => (false, Shift::Lsl),
                    0b100 if !wide => (true, Shift::Asr),
                    0b101 if !wide => (true, Shift::Lsr),
                    0b111 if !wide => (true, Shift::Lsl),
                    _ => return self.undefined(pc, insn),
                };
                let m = bits(insn, 9, 5);
                let vl = self.sve_check()?;
                let mask = self.p(g, vl);
                let operand1 = self.z(dn, vl);
                let operand2 = self.z(m, vl);
                let mut result = operand1;
                for e in 0..es.elements(vl) {
                    if !mask.active(e, es) {
                        continue;
                    }
                    let (value, amount) = if wide {
                        (
                            operand1.get(e, es),
                            operand2.get(e * es.bytes() / 8, Esize::D),
                        )
                    } else if reversed {
                        (operand2.get(e, es), operand1.get(e, es))
                    } else {
                        (operand1.get(e, es), operand2.get(e, es))
                    };
                    result.set(e, es, shift.apply(value, amount, es));
                }
                self.set_z(dn, vl, &result);
            }
        }
        Ok(Flow::Next)
    }

    /// The unary operations, predicated (merging): SXTB, UXTB, SXTH, UXTH,
    /// SXTW, UXTW, ABS, NEG (`00000100 size 010 opc 101 Pg Zn Zd`) and CLS,
    /// CLZ, CNT, CNOT, FABS, FNEG, NOT (`00000100 size 011 opc 101 ...`).
    /// The extends take elements wider than their source; FABS and FNEG
    /// take 16- to 64-bit elements and leave a NaN alone under FPCR.AH
    /// (FPAbs, FPNeg).
    pub(crate) fn exec_sve_int_unary(&mut self, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 23, 22);
        let es = Esize::from_size(size);
        let op = match (bits(insn, 19, 16), size) {
            (0b0000, 0b01..) => Unary::SignExtend(Esize::B),
            (0b0001, 0b01..) => Unary::ZeroExtend(Esize::B),
            (0b0010, 0b10..) => Unary::SignExtend(Esize::H),
            (0b0011, 0b10..) => Unary::ZeroExtend(Esize::H),
            (0b0100, 0b11) => Unary::SignExtend(Esize::S),
            (0b0101, 0b11) => Unary::ZeroExtend(Esize::S),
            (0b0110, _) => Unary::Abs,
            (0b0111, _) => Unary::Neg,
            (0b1000, _) => Unary::Cls,
            (0b1001, _) => Unary::Clz,
            (0b1010, _) => Unary::Cnt,
            (0b1011, _) => Unary::Cnot,
            (0b1100, 0b01..) => Unary::Fabs,
            (0b1101, 0b01..) => Unary::Fneg,
            (0b1110, _) => Unary::Not,
            _ => return self.undefined(pc, insn),
        };
        let (g, n, d) = (bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let env = self.fp_env();
        let ty = match es {
            Esize::H => FpType::Half,
            Esize::S => FpType::Single,
            Esize::B | Esize::D => FpType::Double,
        };
        let mask = self.p(g, vl);
        let operand = self.z(n, vl);
        let mut result = self.z(d, vl);
        let width = es.bits();
        for e in 0..es.elements(vl) {
            if !mask.active(e, es) {
                continue;
            }
            let x = operand.get(e, es);
            let r = match op {
                Unary::SignExtend(from) => sext(x & from.mask(), from.bits()),
                Unary::ZeroExtend(from) => x & from.mask(),
                Unary::Abs => es.signed(x).unsigned_abs(),
                Unary::Neg => x.wrapping_neg(),
                // CLS: CountLeadingZeroBits(x<N-1:1> EOR x<N-2:0>).
                Unary::Cls => u64::from(
                    (((x >> 1) ^ x) & (es.mask() >> 1)).leading_zeros() - (64 - (width - 1)),
                ),
                Unary::Clz => u64::from((x & es.mask()).leading_zeros() - (64 - width)),
                Unary::Cnt => u64::from((x & es.mask()).count_ones()),
                Unary::Cnot => u64::from(x & es.mask() == 0),
                Unary::Fabs => env.abs(ty, x),
                Unary::Fneg => env.neg(ty, x),
                Unary::Not => !x,
            };
            result.set(e, es, r & es.mask());
        }
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// MLA, MLS (`00000100 size 0 Zm 01 op Pg Zn Zda`): `Zda +/- Zn * Zm`;
    /// MAD, MSB (`00000100 size 0 Zm 11 op Pg Za Zdn`): `Za +/- Zdn * Zm`;
    /// inactive elements keep the destination's.
    pub(crate) fn exec_sve_mul_add(&mut self, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let (m, g, n9, d) = (
            bits(insn, 20, 16),
            bits(insn, 12, 10),
            bits(insn, 9, 5),
            bits(insn, 4, 0),
        );
        let subtract = bit(insn, 13);
        let multiplicand_is_dest = bit(insn, 15);
        let vl = self.sve_check()?;
        let mask = self.p(g, vl);
        let dest = self.z(d, vl);
        let other = self.z(n9, vl);
        let operand2 = self.z(m, vl);
        // MLA: the addend is Zda, the multiplicand Zn; MAD: the addend Za,
        // the multiplicand Zdn.
        let (multiplicand, addend) = if multiplicand_is_dest {
            (&dest, &other)
        } else {
            (&other, &dest)
        };
        let mut result = dest;
        for e in 0..es.elements(vl) {
            if mask.active(e, es) {
                let product = multiplicand.get(e, es).wrapping_mul(operand2.get(e, es));
                let a = addend.get(e, es);
                let r = if subtract {
                    a.wrapping_sub(product)
                } else {
                    a.wrapping_add(product)
                };
                result.set(e, es, r);
            }
        }
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// ADD, SUB, SQADD, UQADD, SQSUB, UQSUB (vectors, unpredicated):
    /// `00000100 size 1 Zm 000 opc Zn Zd`.
    pub(crate) fn exec_sve_add_sub(&mut self, pc: u64, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let (m, n, d) = (bits(insn, 20, 16), bits(insn, 9, 5), bits(insn, 4, 0));
        let opc = bits(insn, 12, 10);
        if matches!(opc, 0b010 | 0b011) {
            return self.undefined(pc, insn);
        }
        let vl = self.sve_check()?;
        let operand1 = self.z(n, vl);
        let operand2 = self.z(m, vl);
        let mut result = Zv::ZERO;
        for e in 0..es.elements(vl) {
            let (a, b) = (operand1.get(e, es), operand2.get(e, es));
            let r = match opc {
                0b000 => a.wrapping_add(b),
                0b001 => a.wrapping_sub(b),
                0b100 => sat_q(int(a, es, false) + int(b, es, false), es, false),
                0b101 => sat_q(int(a, es, true) + int(b, es, true), es, true),
                0b110 => sat_q(int(a, es, false) - int(b, es, false), es, false),
                _ => sat_q(int(a, es, true) - int(b, es, true), es, true),
            };
            result.set(e, es, r);
        }
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// The bitwise instructions on whole vectors: AND, ORR, EOR, BIC
    /// (`00000100 opc 1 Zm 001100 Zn Zd`), XAR (`00000100 tszh 1 tszl imm3
    /// 001101 Zm Zdn`), EOR3, BCAX, BSL, BSL1N, BSL2N, NBSL (`00000100 opc 1
    /// Zm 00111 o2 Zk Zdn`).
    pub(crate) fn exec_sve_bitwise(&mut self, pc: u64, insn: u32) -> ExecResult {
        let opc = bits(insn, 23, 22);
        match bits(insn, 12, 10) {
            0b100 => {
                let (m, n, d) = (bits(insn, 20, 16), bits(insn, 9, 5), bits(insn, 4, 0));
                let vl = self.sve_check()?;
                let operand1 = self.z(n, vl);
                let operand2 = self.z(m, vl);
                let mut result = Zv::ZERO;
                for e in 0..Esize::D.elements(vl) {
                    let (a, b) = (operand1.get(e, Esize::D), operand2.get(e, Esize::D));
                    let r = match opc {
                        0b00 => a & b,
                        0b01 => a | b,
                        0b10 => a ^ b,
                        _ => a & !b,
                    };
                    result.set(e, Esize::D, r);
                }
                self.set_z(d, vl, &result);
            }
            0b101 => {
                let tsize = (opc << 2) | bits(insn, 20, 19);
                if tsize == 0 {
                    return self.undefined(pc, insn);
                }
                let es = Esize::from_tsize(tsize);
                let rotate = shift_amounts((tsize << 3) | bits(insn, 18, 16), es).right;
                let (m, dn) = (bits(insn, 9, 5), bits(insn, 4, 0));
                let vl = self.sve_check()?;
                let operand1 = self.z(dn, vl);
                let operand2 = self.z(m, vl);
                let mut result = Zv::ZERO;
                for e in 0..es.elements(vl) {
                    let x = operand1.get(e, es) ^ operand2.get(e, es);
                    result.set(e, es, ror(x, rotate as u32 % es.bits(), es.bits()));
                }
                self.set_z(dn, vl, &result);
            }
            op0 => {
                let o2 = op0 & 1 != 0;
                if !o2 && opc >= 0b10 {
                    return self.undefined(pc, insn);
                }
                let (m, k, dn) = (bits(insn, 20, 16), bits(insn, 9, 5), bits(insn, 4, 0));
                let vl = self.sve_check()?;
                let operand1 = self.z(dn, vl);
                let operand2 = self.z(m, vl);
                let operand3 = self.z(k, vl);
                let mut result = Zv::ZERO;
                for e in 0..Esize::D.elements(vl) {
                    let (a, b, c) = (
                        operand1.get(e, Esize::D),
                        operand2.get(e, Esize::D),
                        operand3.get(e, Esize::D),
                    );
                    let r = match (opc, o2) {
                        (0b00, false) => a ^ b ^ c,
                        (0b01, false) => a ^ (b & !c),
                        (0b00, true) => (a & c) | (b & !c),
                        (0b01, true) => (!a & c) | (b & !c),
                        (0b10, true) => (a & c) | (!b & !c),
                        _ => !((a & c) | (b & !c)),
                    };
                    result.set(e, Esize::D, r);
                }
                self.set_z(dn, vl, &result);
            }
        }
        Ok(Flow::Next)
    }

    /// The shifts, unpredicated: ASR, LSR, LSL by wide elements (`00000100
    /// size 1 Zm 1000 opc Zn Zd`, 8- to 32-bit elements) and by immediate
    /// (`00000100 tszh 1 tszl imm3 1001 opc Zn Zd`).
    pub(crate) fn exec_sve_shift_unpred(&mut self, pc: u64, insn: u32) -> ExecResult {
        let shift = match bits(insn, 11, 10) {
            0b00 => Shift::Asr,
            0b01 => Shift::Lsr,
            0b11 => Shift::Lsl,
            _ => return self.undefined(pc, insn),
        };
        let (n, d) = (bits(insn, 9, 5), bits(insn, 4, 0));
        if bit(insn, 12) {
            let tsize = (bits(insn, 23, 22) << 2) | bits(insn, 20, 19);
            if tsize == 0 {
                return self.undefined(pc, insn);
            }
            let es = Esize::from_tsize(tsize);
            let ShiftAmounts { right, left } = shift_amounts((tsize << 3) | bits(insn, 18, 16), es);
            let amount = if shift == Shift::Lsl { left } else { right };
            let vl = self.sve_check()?;
            let operand1 = self.z(n, vl);
            let mut result = Zv::ZERO;
            for e in 0..es.elements(vl) {
                result.set(e, es, shift.apply(operand1.get(e, es), amount, es));
            }
            self.set_z(d, vl, &result);
        } else {
            let size = bits(insn, 23, 22);
            if size == 0b11 {
                return self.undefined(pc, insn);
            }
            let es = Esize::from_size(size);
            let m = bits(insn, 20, 16);
            let vl = self.sve_check()?;
            let operand1 = self.z(n, vl);
            let operand2 = self.z(m, vl);
            let mut result = Zv::ZERO;
            for e in 0..es.elements(vl) {
                let amount = operand2.get(e * es.bytes() / 8, Esize::D);
                result.set(e, es, shift.apply(operand1.get(e, es), amount, es));
            }
            self.set_z(d, vl, &result);
        }
        Ok(Flow::Next)
    }

    /// INDEX: `00000100 size 1 Rm|imm5b 0100 op Rn|imm5 Zd`, element `e`
    /// the start plus `e` times the increment, each a signed immediate or
    /// the low `esize` bits of a general register.
    pub(crate) fn exec_sve_index(&mut self, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let d = bits(insn, 4, 0);
        let (start_field, step_field) = (bits(insn, 9, 5), bits(insn, 20, 16));
        let start = if bit(insn, 10) {
            es.signed(self.x_or_zr(start_field))
        } else {
            sext(u64::from(start_field), 5) as i64
        };
        let step = if bit(insn, 11) {
            es.signed(self.x_or_zr(step_field))
        } else {
            sext(u64::from(step_field), 5) as i64
        };
        let vl = self.sve_check()?;
        let mut result = Zv::ZERO;
        for e in 0..es.elements(vl) {
            result.set(
                e,
                es,
                start.wrapping_add((e as i64).wrapping_mul(step)) as u64,
            );
        }
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// MUL, PMUL, SMULH, UMULH (`00000100 size 1 Zm 0110 opc Zn Zd`, PMUL on
    /// bytes) and SQDMULH, SQRDMULH (`00000100 size 1 Zm 01110 R Zn Zd`),
    /// vectors, unpredicated.
    pub(crate) fn exec_sve_mul_unpred(&mut self, pc: u64, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let (m, n, d) = (bits(insn, 20, 16), bits(insn, 9, 5), bits(insn, 4, 0));
        let op = match bits(insn, 12, 10) {
            0b000 => MulUnpredicated::Mul,
            0b001 if es == Esize::B => MulUnpredicated::Pmul,
            0b010 => MulUnpredicated::MulHigh { unsigned: false },
            0b011 => MulUnpredicated::MulHigh { unsigned: true },
            0b100 => MulUnpredicated::DoublingHigh { rounding: false },
            0b101 => MulUnpredicated::DoublingHigh { rounding: true },
            _ => return self.undefined(pc, insn),
        };
        let vl = self.sve_check()?;
        let operand1 = self.z(n, vl);
        let operand2 = self.z(m, vl);
        let mut result = Zv::ZERO;
        for e in 0..es.elements(vl) {
            let (a, b) = (operand1.get(e, es), operand2.get(e, es));
            let r = match op {
                MulUnpredicated::Mul => a.wrapping_mul(b),
                MulUnpredicated::Pmul => polynomial_mult(a, b, 8) as u64,
                MulUnpredicated::MulHigh { unsigned } => mul_high(a, b, es, unsigned),
                MulUnpredicated::DoublingHigh { rounding } => {
                    sat_doubling_mul_high(a, b, es, rounding)
                }
            };
            result.set(e, es, r);
        }
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// The element counts (`sve_countelt`): CNTB, CNTH, CNTW, CNTD; INCx and
    /// DECx of a general register and of a vector; SQINCx, UQINCx, SQDECx,
    /// UQDECx of a general register (32-bit, sign- or zero-extended, or
    /// 64-bit) and of a vector. Each adds or subtracts the count a pattern
    /// names times `imm4 + 1`.
    pub(crate) fn exec_sve_element_count(&mut self, pc: u64, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let pattern = bits(insn, 9, 5);
        let imm = i128::from(bits(insn, 19, 16) + 1);
        let dn = bits(insn, 4, 0);
        let form = match (bit(insn, 20), bits(insn, 13, 11)) {
            (false, 0b000 | 0b001) => ElementCount::SaturatingVector,
            (true, 0b000) => ElementCount::Vector,
            (false, 0b100) => ElementCount::Count,
            (true, 0b100) => ElementCount::Scalar,
            (wide, 0b110 | 0b111) => ElementCount::SaturatingScalar { wide },
            _ => return self.undefined(pc, insn),
        };
        // The saturating forms have D at bit 11 and U at 10; the others D
        // at 10.
        let saturating = matches!(
            form,
            ElementCount::SaturatingVector | ElementCount::SaturatingScalar { .. }
        );
        let (decrement, unsigned) = if saturating {
            (bit(insn, 11), bit(insn, 10))
        } else {
            (bit(insn, 10), false)
        };
        let vl = self.sve_check()?;
        let count = decode_pred_count(pattern, es, vl) as i128 * imm;
        let delta = if decrement { -count } else { count };
        match form {
            ElementCount::SaturatingVector => {
                let operand = self.z(dn, vl);
                let mut result = Zv::ZERO;
                for e in 0..es.elements(vl) {
                    result.set(
                        e,
                        es,
                        sat_q(int(operand.get(e, es), es, unsigned) + delta, es, unsigned),
                    );
                }
                self.set_z(dn, vl, &result);
            }
            ElementCount::Vector => {
                let operand = self.z(dn, vl);
                let mut result = Zv::ZERO;
                for e in 0..es.elements(vl) {
                    result.set(e, es, operand.get(e, es).wrapping_add(delta as u64));
                }
                self.set_z(dn, vl, &result);
            }
            ElementCount::Count => self.set_x_or_zr(dn, count as u64),
            ElementCount::Scalar => {
                let value = self.x_or_zr(dn).wrapping_add(delta as u64);
                self.set_x_or_zr(dn, value);
            }
            ElementCount::SaturatingScalar { wide } => {
                let value = sat_add_scalar(self.x_or_zr(dn), delta, wide, unsigned);
                self.set_x_or_zr(dn, value);
            }
        }
        Ok(Flow::Next)
    }

    /// AND, ORR, EOR (immediate) (`00000101 opc 0000 imm13 Zdn`) and DUPM
    /// (`00000101 110000 imm13 Zd`): the 64-bit bitmask immediate Arm
    /// `DecodeBitMasks` makes of `imm13`, UNDEFINED where it is reserved.
    pub(crate) fn exec_sve_bitmask_imm(&mut self, pc: u64, insn: u32) -> ExecResult {
        let imm13 = bits(insn, 17, 5);
        let Some(masks) = decode_bit_masks(
            imm13 & 0x1000 != 0,
            imm13 & 0x3F,
            (imm13 >> 6) & 0x3F,
            true,
            64,
        ) else {
            return self.undefined(pc, insn);
        };
        let imm = masks.wmask;
        let dn = bits(insn, 4, 0);
        let opc = bits(insn, 23, 22);
        let vl = self.sve_check()?;
        let operand = self.z(dn, vl);
        let mut result = Zv::ZERO;
        for e in 0..Esize::D.elements(vl) {
            let x = operand.get(e, Esize::D);
            let r = match opc {
                0b00 => x | imm,
                0b01 => x ^ imm,
                0b10 => x & imm,
                _ => imm,
            };
            result.set(e, Esize::D, r);
        }
        self.set_z(dn, vl, &result);
        Ok(Flow::Next)
    }

    /// CPY (immediate), `00000101 size 01 Pg 0 M sh imm8 Zd` (the signed
    /// `imm8`, shifted by 8 with `sh = 1`; inactive elements merged with `M
    /// = 1` or zeroed), and FCPY, `00000101 size 01 Pg 110 imm8 Zd` (the
    /// floating-point constant `VFPExpandImm` makes of `imm8`; 16- to
    /// 64-bit elements).
    pub(crate) fn exec_sve_copy_imm(&mut self, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 23, 22);
        let es = Esize::from_size(size);
        let (g, d) = (bits(insn, 19, 16), bits(insn, 4, 0));
        let imm8 = u64::from(bits(insn, 12, 5));
        let floating = bit(insn, 15);
        let (imm, merging) = if floating {
            let ty = match size {
                0b01 => FpType::Half,
                0b10 => FpType::Single,
                0b11 => FpType::Double,
                _ => return self.undefined(pc, insn),
            };
            (vfp_expand_imm(imm8, ty), true)
        } else {
            if size == 0b00 && bit(insn, 13) {
                return self.undefined(pc, insn);
            }
            let imm = sext(imm8, 8) << if bit(insn, 13) { 8 } else { 0 };
            (imm, bit(insn, 14))
        };
        let vl = self.sve_check()?;
        let mask = self.p(g, vl);
        let mut result = if merging { self.z(d, vl) } else { Zv::ZERO };
        for e in 0..es.elements(vl) {
            if mask.active(e, es) {
                result.set(e, es, imm);
            }
        }
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// The wide immediates, unpredicated (`00100101 size 1 op0 opc 11 ...
    /// imm8 Zdn`): ADD, SUB, SUBR, SQADD, UQADD, SQSUB, UQSUB (the unsigned
    /// `imm8`, shifted by 8 with `sh = 1`), SMAX, UMAX, SMIN, UMIN (`imm8`
    /// signed for the signed forms), MUL (signed `imm8`), DUP (signed
    /// `imm8`, shifted by 8 with `sh = 1`) and FDUP (`VFPExpandImm`).
    pub(crate) fn exec_sve_wide_imm(&mut self, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 23, 22);
        let es = Esize::from_size(size);
        let imm8 = u64::from(bits(insn, 12, 5));
        let sh = bit(insn, 13);
        let dn = bits(insn, 4, 0);
        let opc = bits(insn, 18, 16);
        match bits(insn, 20, 19) {
            0b00 => {
                if size == 0b00 && sh {
                    return self.undefined(pc, insn);
                }
                let imm = i128::from(imm8 << if sh { 8 } else { 0 });
                if opc == 0b010 {
                    return self.undefined(pc, insn);
                }
                let vl = self.sve_check()?;
                let operand = self.z(dn, vl);
                let mut result = Zv::ZERO;
                for e in 0..es.elements(vl) {
                    let x = operand.get(e, es);
                    let r = match opc {
                        0b000 => x.wrapping_add(imm as u64),
                        0b001 => x.wrapping_sub(imm as u64),
                        0b011 => (imm as u64).wrapping_sub(x),
                        0b100 => sat_q(int(x, es, false) + imm, es, false),
                        0b101 => sat_q(int(x, es, true) + imm, es, true),
                        0b110 => sat_q(int(x, es, false) - imm, es, false),
                        _ => sat_q(int(x, es, true) - imm, es, true),
                    };
                    result.set(e, es, r);
                }
                self.set_z(dn, vl, &result);
            }
            0b01 | 0b10 => {
                let multiply = bits(insn, 20, 19) == 0b10;
                if sh || (multiply && opc != 0) || (!multiply && opc > 0b011) {
                    return self.undefined(pc, insn);
                }
                let unsigned = !multiply && opc & 1 != 0;
                let imm = if unsigned {
                    i128::from(imm8)
                } else {
                    i128::from(sext(imm8, 8) as i64)
                };
                let vl = self.sve_check()?;
                let operand = self.z(dn, vl);
                let mut result = Zv::ZERO;
                for e in 0..es.elements(vl) {
                    let x = int(operand.get(e, es), es, unsigned);
                    let r = match (multiply, opc) {
                        (true, _) => x * imm,
                        (false, 0b000 | 0b001) => x.max(imm),
                        (false, _) => x.min(imm),
                    };
                    result.set(e, es, r as u64);
                }
                self.set_z(dn, vl, &result);
            }
            _ => {
                let floating = bit(insn, 16);
                if bits(insn, 18, 17) != 0 {
                    return self.undefined(pc, insn);
                }
                let imm = if floating {
                    let ty = match size {
                        0b01 => FpType::Half,
                        0b10 => FpType::Single,
                        0b11 => FpType::Double,
                        _ => return self.undefined(pc, insn),
                    };
                    if sh {
                        return self.undefined(pc, insn);
                    }
                    vfp_expand_imm(imm8, ty)
                } else {
                    if size == 0b00 && sh {
                        return self.undefined(pc, insn);
                    }
                    sext(imm8, 8) << if sh { 8 } else { 0 }
                };
                let vl = self.sve_check()?;
                self.set_z(dn, vl, &Zv::replicate(imm, es, vl));
            }
        }
        Ok(Flow::Next)
    }

    /// ADDVL, ADDPL: `00000100 0 op 1 Rn 01010 imm6 Rd`, at the vector
    /// length (see [`Cpu::add_vector_length`]).
    pub(crate) fn exec_addvl(&mut self, insn: u32) -> ExecResult {
        let vl = self.sve_check()?;
        self.add_vector_length(insn, vl);
        Ok(Flow::Next)
    }

    /// RDVL: `00000100 1 0 1 11111 01010 imm6 Rd`, of the vector length
    /// (see [`Cpu::read_vector_length`]).
    pub(crate) fn exec_rdvl(&mut self, insn: u32) -> ExecResult {
        let vl = self.sve_check()?;
        self.read_vector_length(insn, vl);
        Ok(Flow::Next)
    }

    /// ADDVL, ADDPL, ADDSVL and ADDSPL after their checks: `Rd|SP` becomes
    /// `Rn|SP` plus `imm6` times `vl` in bytes (`op` = 0) or its predicate
    /// length in bytes (`op` = 1).
    pub(crate) fn add_vector_length(&mut self, insn: u32, vl: VecLen) {
        let imm = sext(u64::from(bits(insn, 10, 5)), 6);
        let unit = if bit(insn, 22) {
            vl.predicate_bytes()
        } else {
            vl.bytes()
        };
        let result = self
            .x_or_sp(bits(insn, 20, 16))
            .wrapping_add(imm.wrapping_mul(unit as u64));
        self.set_x_or_sp(bits(insn, 4, 0), result);
    }

    /// RDVL and RDSVL after their checks: `Rd` (XZR at 31) becomes `imm6`
    /// times `vl` in bytes.
    pub(crate) fn read_vector_length(&mut self, insn: u32, vl: VecLen) {
        let imm = sext(u64::from(bits(insn, 10, 5)), 6);
        self.set_x_or_zr(bits(insn, 4, 0), imm.wrapping_mul(vl.bytes() as u64));
    }
}
