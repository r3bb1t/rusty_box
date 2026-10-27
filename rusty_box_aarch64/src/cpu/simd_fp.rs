//! Advanced SIMD floating point (Arm ARM C7.2), vector and scalar: the FP
//! opcodes of the three-same, two-register-miscellaneous, shift-by-immediate
//! (fixed-point conversions), by-element, across-lanes and scalar-pairwise
//! classes. Every element goes through the operations in `fp` under one
//! `FpEnv` per instruction, so each lane's exceptions accumulate into FPSR.
//!
//! As in `simd_arith`, a scalar form is the vector operation on one element,
//! and building results up from zero gives the zeroed upper bits — or,
//! under FPCR.NEP (FEAT_AFP), those of the source register each scalar
//! form's page names (`Cpu::fp_set_merging`); the pairwise, across-lanes
//! and compare-with-zero forms zero them whatever FPCR.NEP is. A vector of
//! doubles needs `Q = 1`.

use super::exec::{bit, bits, ExecResult, Flow};
use super::features::Feature;
use super::fp::{
    fp_binary, fp_compare, fp_compare_holds, fp_convert, fp_convert_round_odd, fp_mul_add,
    fp_to_int, fp_unary, int_to_fp, round_int_n, FpBinary, FpCompare, FpConversion, FpType,
    FpUnary, FrintPrecision, IntSize, IntWidth,
};
use super::fp_estimate::{urecpe, ursqrte};
use super::simd::{elem, set_elem};
use super::simd_arith::narrow_into;
use super::Cpu;
use crate::softfloat::{self as sf, FpOrdering, FpRounding, FusedMulAdd, InexactReport, NanSignal};

#[inline]
const fn datasize(q: bool) -> u32 {
    if q {
        128
    } else {
        64
    }
}

/// A comparison against +0.0 (FCMGT, FCMGE, FCMEQ, FCMLE, FCMLT `#0.0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZeroCompare {
    Gt,
    Ge,
    Eq,
    Le,
    Lt,
}

impl Cpu {
    /// Three-same floating point of precision `ty`, vector (`0 Q U 01110 a
    /// sz 1 Rm 11 opc 1 Rn Rd`, and `0 Q U 01110 a 10 Rm 00 opc 1 Rn Rd` for
    /// half precision) and scalar (`01 U 11110 ...`, likewise).
    pub(crate) fn exec_simd_same_fp(
        &mut self,
        pc: u64,
        insn: u32,
        scalar: bool,
        ty: FpType,
    ) -> ExecResult {
        let q = bit(insn, 30);
        let u = bit(insn, 29);
        let a = bit(insn, 23);
        let opc = bits(insn, 13, 11);

        #[derive(Clone, Copy)]
        enum Op {
            Binary(FpBinary),
            Fused(FusedMulAdd),
            Compare(FpCompare),
            Pairwise(FpBinary),
        }
        let op = match (u, a, opc) {
            (false, false, 0b000) => Op::Binary(FpBinary::MaxNum),
            (false, false, 0b001) => Op::Fused(FusedMulAdd::Fmadd),
            (false, false, 0b010) => Op::Binary(FpBinary::Add),
            (false, false, 0b011) => Op::Binary(FpBinary::MulX),
            (false, false, 0b100) => Op::Compare(FpCompare::Eq),
            (false, false, 0b110) => Op::Binary(FpBinary::Max),
            (false, false, 0b111) => Op::Binary(FpBinary::RecipStep),
            (false, true, 0b000) => Op::Binary(FpBinary::MinNum),
            (false, true, 0b001) => Op::Fused(FusedMulAdd::Fmsub),
            (false, true, 0b010) => Op::Binary(FpBinary::Sub),
            (false, true, 0b110) => Op::Binary(FpBinary::Min),
            (false, true, 0b111) => Op::Binary(FpBinary::RSqrtStep),
            (true, false, 0b000) => Op::Pairwise(FpBinary::MaxNum),
            (true, false, 0b010) => Op::Pairwise(FpBinary::Add),
            (true, false, 0b011) => Op::Binary(FpBinary::Mul),
            (true, false, 0b100) => Op::Compare(FpCompare::Ge),
            (true, false, 0b101) => Op::Compare(FpCompare::AbsGe),
            (true, false, 0b110) => Op::Pairwise(FpBinary::Max),
            (true, false, 0b111) => Op::Binary(FpBinary::Div),
            (true, true, 0b000) => Op::Pairwise(FpBinary::MinNum),
            (true, true, 0b010) => Op::Binary(FpBinary::AbsDiff),
            (true, true, 0b100) => Op::Compare(FpCompare::Gt),
            (true, true, 0b101) => Op::Compare(FpCompare::AbsGt),
            (true, true, 0b110) => Op::Pairwise(FpBinary::Min),
            _ => return self.undefined(pc, insn),
        };
        let has_scalar_form = matches!(
            op,
            Op::Binary(
                FpBinary::MulX | FpBinary::RecipStep | FpBinary::RSqrtStep | FpBinary::AbsDiff
            ) | Op::Compare(_)
        );
        if (scalar && !has_scalar_form) || (!scalar && ty == FpType::Double && !q) {
            return self.undefined(pc, insn);
        }
        // Scalar FMULX, FRECPS and FRSQRTS are legal in Streaming SVE mode
        // (Arm ARM FMULX, FRECPS, FRSQRTS: `if elements == 1 then
        // AArch64_CheckFPEnabled()`).
        if scalar
            && matches!(
                op,
                Op::Binary(FpBinary::MulX | FpBinary::RecipStep | FpBinary::RSqrtStep)
            )
        {
            self.fp_check()?;
        } else {
            self.simd_check()?;
        }
        let n = self.v[bits(insn, 9, 5) as usize];
        let m = self.v[bits(insn, 20, 16) as usize];
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let esize = ty.esize();
        let lanes = if scalar { 1 } else { datasize(q) / esize };
        let mut env = self.fp_env();
        let mut r = 0u128;
        for e in 0..lanes {
            let (x, y) = (elem(n, e, esize), elem(m, e, esize));
            let value = match op {
                Op::Binary(op) => fp_binary(op, ty, x, y, &mut env),
                Op::Fused(form) => fp_mul_add(form, ty, elem(d, e, esize), x, y, &mut env),
                Op::Compare(cmp) => {
                    if fp_compare_holds(cmp, ty, x, y, &mut env) {
                        u64::MAX
                    } else {
                        0
                    }
                }
                Op::Pairwise(op) => {
                    // concat = Vm:Vn; result[e] = op(concat[2e], concat[2e+1]).
                    let pick = |i: u32| {
                        if i < lanes {
                            elem(n, i, esize)
                        } else {
                            elem(m, i - lanes, esize)
                        }
                    };
                    fp_binary(op, ty, pick(2 * e), pick(2 * e + 1), &mut env)
                }
            };
            set_elem(&mut r, e, esize, value);
        }
        self.fp_commit(&env);
        if scalar {
            // `elements == 1 && IsMerging(FPCR())`: the compares merge
            // into Vm, FABD, FMULX, FRECPS and FRSQRTS into Vn.
            let merge = match op {
                Op::Compare(_) => bits(insn, 20, 16),
                Op::Binary(_) | Op::Fused(_) | Op::Pairwise(_) => bits(insn, 9, 5),
            };
            self.fp_set_merging(rd as u32, merge, ty.mask(), r as u64);
        } else {
            self.write_v(rd, r);
        }
        Ok(Flow::Next)
    }

    /// Two-register miscellaneous floating point of precision `ty`, vector
    /// (`0 Q U 01110 a sz 10000 opcode 10 Rn Rd`, and `0 Q U 01110 a 1111 00
    /// opcode 10 Rn Rd` for half precision) and scalar (`01 U 11110 ...`,
    /// likewise), with URECPE and URSQRTE. Half precision has no FCVTN,
    /// FCVTL, FCVTXN, URECPE or URSQRTE.
    pub(crate) fn exec_simd_misc_fp(
        &mut self,
        pc: u64,
        insn: u32,
        scalar: bool,
        ty: FpType,
    ) -> ExecResult {
        let q = bit(insn, 30);
        let u = bit(insn, 29);
        let a = bit(insn, 23);
        let sz = ty == FpType::Double;
        let half = ty == FpType::Half;
        let opcode = bits(insn, 16, 12);
        let env = self.fp_env();

        /// An operation on each element, at the element's own width.
        #[derive(Clone, Copy)]
        enum Each {
            Unary(FpUnary),
            Zero(ZeroCompare),
            ToInt(FpRounding, bool),
            FromInt(bool),
            /// FRINT32Z, FRINT32X, FRINT64Z, FRINT64X (FEAT_FRINTTS).
            RoundIntN(FpRounding, IntSize),
            URecipEstimate,
            URSqrtEstimate,
        }
        #[derive(Clone, Copy)]
        enum Op {
            Each(Each),
            /// FCVTN: halve each element's precision.
            Narrow,
            /// FCVTXN: double to single, rounding to odd.
            NarrowOdd,
            /// BFCVTN: single precision to BFloat16.
            NarrowBf16,
            /// FCVTL: double each element's precision.
            Long,
        }
        let round = |rounding| {
            Op::Each(Each::Unary(FpUnary::RoundInt(
                rounding,
                InexactReport::Suppress,
            )))
        };
        let op = match (u, a, opcode) {
            (false, false, 0b10110) => Op::Narrow,
            (false, false, 0b10111) => Op::Long,
            (false, false, 0b11000) => round(FpRounding::TieEven),
            (false, false, 0b11001) => round(FpRounding::NegInf),
            (false, false, 0b11010) => Op::Each(Each::ToInt(FpRounding::TieEven, true)),
            (false, false, 0b11011) => Op::Each(Each::ToInt(FpRounding::NegInf, true)),
            (false, false, 0b11100) => Op::Each(Each::ToInt(FpRounding::TieAway, true)),
            (false, false, 0b11101) => Op::Each(Each::FromInt(true)),
            (false, false, 0b11110) => Op::Each(Each::RoundIntN(FpRounding::Zero, IntSize::Bits32)),
            (false, false, 0b11111) => Op::Each(Each::RoundIntN(FpRounding::Zero, IntSize::Bits64)),
            (false, true, 0b10110) => Op::NarrowBf16,
            (false, true, 0b01100) => Op::Each(Each::Zero(ZeroCompare::Gt)),
            (false, true, 0b01101) => Op::Each(Each::Zero(ZeroCompare::Eq)),
            (false, true, 0b01110) => Op::Each(Each::Zero(ZeroCompare::Lt)),
            (false, true, 0b01111) => Op::Each(Each::Unary(FpUnary::Abs)),
            (false, true, 0b11000) => round(FpRounding::PosInf),
            (false, true, 0b11001) => round(FpRounding::Zero),
            (false, true, 0b11010) => Op::Each(Each::ToInt(FpRounding::PosInf, true)),
            (false, true, 0b11011) => Op::Each(Each::ToInt(FpRounding::Zero, true)),
            (false, true, 0b11100) => Op::Each(Each::URecipEstimate),
            (false, true, 0b11101) => Op::Each(Each::Unary(FpUnary::RecipEstimate)),
            (false, true, 0b11111) => Op::Each(Each::Unary(FpUnary::RecpX)),
            (true, false, 0b10110) => Op::NarrowOdd,
            (true, false, 0b11000) => round(FpRounding::TieAway),
            (true, false, 0b11001) => Op::Each(Each::Unary(FpUnary::RoundInt(
                env.rounding(),
                InexactReport::Raise,
            ))),
            (true, false, 0b11010) => Op::Each(Each::ToInt(FpRounding::TieEven, false)),
            (true, false, 0b11011) => Op::Each(Each::ToInt(FpRounding::NegInf, false)),
            (true, false, 0b11100) => Op::Each(Each::ToInt(FpRounding::TieAway, false)),
            (true, false, 0b11101) => Op::Each(Each::FromInt(false)),
            (true, false, 0b11110) => Op::Each(Each::RoundIntN(env.rounding(), IntSize::Bits32)),
            (true, false, 0b11111) => Op::Each(Each::RoundIntN(env.rounding(), IntSize::Bits64)),
            (true, true, 0b01100) => Op::Each(Each::Zero(ZeroCompare::Ge)),
            (true, true, 0b01101) => Op::Each(Each::Zero(ZeroCompare::Le)),
            (true, true, 0b01111) => Op::Each(Each::Unary(FpUnary::Neg)),
            (true, true, 0b11001) => Op::Each(Each::Unary(FpUnary::RoundInt(
                env.rounding(),
                InexactReport::Suppress,
            ))),
            (true, true, 0b11010) => Op::Each(Each::ToInt(FpRounding::PosInf, false)),
            (true, true, 0b11011) => Op::Each(Each::ToInt(FpRounding::Zero, false)),
            (true, true, 0b11100) => Op::Each(Each::URSqrtEstimate),
            (true, true, 0b11101) => Op::Each(Each::Unary(FpUnary::RSqrtEstimate)),
            (true, true, 0b11111) => Op::Each(Each::Unary(FpUnary::Sqrt)),
            _ => return self.undefined(pc, insn),
        };
        let reserved = match op {
            // Only the estimates, conversions and comparisons have scalar
            // forms, and FRECPX has only a scalar form.
            Op::Each(Each::Unary(FpUnary::RecpX)) => !scalar,
            // FRINT32* and FRINT64* (`if sz::Q == '10'`) are single and
            // double precision vectors, BFCVTN a vector of singles; the
            // half-precision class has neither.
            Op::Each(Each::RoundIntN(..)) => scalar || half || (sz && !q),
            Op::NarrowBf16 => scalar || half || sz,
            Op::Each(
                Each::Unary(FpUnary::RecipEstimate | FpUnary::RSqrtEstimate)
                | Each::Zero(_)
                | Each::ToInt(..)
                | Each::FromInt(_),
            ) => !scalar && sz && !q,
            Op::Each(Each::Unary(_)) => scalar || (sz && !q),
            Op::Each(Each::URecipEstimate | Each::URSqrtEstimate) => scalar || sz || half,
            Op::Narrow | Op::Long => scalar || half,
            Op::NarrowOdd => !sz,
        };
        if reserved {
            return self.undefined(pc, insn);
        }
        // Scalar FRECPE and FRSQRTE, and FRECPX (scalar only), are legal in
        // Streaming SVE mode (Arm ARM FRECPE, FRSQRTE: `if elements == 1
        // then AArch64_CheckFPEnabled()`; FRECPX: `AArch64_CheckFPEnabled()`).
        // The scalar conversions are legal there only with FEAT_FPRCVT,
        // which this core lacks.
        if scalar
            && matches!(
                op,
                Op::Each(Each::Unary(
                    FpUnary::RecipEstimate | FpUnary::RSqrtEstimate | FpUnary::RecpX
                ))
            )
        {
            self.fp_check()?;
        } else {
            self.simd_check()?;
        }
        let n = self.v[bits(insn, 9, 5) as usize];
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let mut env = env;
        let mut r = 0u128;
        match op {
            Op::Narrow | Op::NarrowOdd => {
                // Source elements of 2×esize from all of Vn; the narrow result
                // fills half of Vd (the upper half for the `2` forms).
                let (esize, conversion) = if sz {
                    (32, FpConversion::DoubleToSingle)
                } else {
                    (16, FpConversion::SingleToHalf)
                };
                let count = if scalar { 1 } else { 64 / esize };
                let mut part = 0u128;
                for e in 0..count {
                    let x = elem(n, e, 2 * esize);
                    let v = if matches!(op, Op::NarrowOdd) {
                        fp_convert_round_odd(x, &mut env)
                    } else {
                        fp_convert(conversion, x, &mut env)
                    };
                    set_elem(&mut part, e, esize, v);
                }
                r = if scalar {
                    part
                } else {
                    narrow_into(d, part, q)
                };
            }
            Op::NarrowBf16 => {
                // Arm FPConvertBF of each single of Vn into `Vpart(d, Q)`.
                let rounding = env.rounding();
                let mut part = 0u128;
                for e in 0..4 {
                    let v = sf::f32_to_bf16(elem(n, e, 32) as u32, rounding, &mut env.status);
                    set_elem(&mut part, e, 16, u64::from(v));
                }
                r = narrow_into(d, part, q);
            }
            Op::Long => {
                let (esize, conversion) = if sz {
                    (32, FpConversion::SingleToDouble)
                } else {
                    (16, FpConversion::HalfToSingle)
                };
                let half = if q { n >> 64 } else { n };
                for e in 0..64 / esize {
                    let v = fp_convert(conversion, elem(half, e, esize), &mut env);
                    set_elem(&mut r, e, 2 * esize, v);
                }
            }
            Op::Each(each) => {
                let esize = ty.esize();
                let lanes = if scalar { 1 } else { datasize(q) / esize };
                for e in 0..lanes {
                    let x = elem(n, e, esize);
                    let v = match each {
                        Each::Unary(op) => fp_unary(op, ty, x, &mut env),
                        Each::Zero(cmp) => {
                            let signal = if cmp == ZeroCompare::Eq {
                                NanSignal::Quiet
                            } else {
                                NanSignal::Signaling
                            };
                            let ordering = fp_compare(ty, x, FpType::ZERO, signal, &mut env);
                            let holds = match cmp {
                                ZeroCompare::Gt => ordering == FpOrdering::Greater,
                                ZeroCompare::Ge => {
                                    matches!(ordering, FpOrdering::Greater | FpOrdering::Equal)
                                }
                                ZeroCompare::Eq => ordering == FpOrdering::Equal,
                                ZeroCompare::Le => {
                                    matches!(ordering, FpOrdering::Less | FpOrdering::Equal)
                                }
                                ZeroCompare::Lt => ordering == FpOrdering::Less,
                            };
                            if holds {
                                u64::MAX
                            } else {
                                0
                            }
                        }
                        Each::ToInt(rounding, signed) => {
                            fp_to_int(ty, x, 0, rounding, signed, IntWidth::OfElement, &mut env)
                        }
                        Each::FromInt(signed) => {
                            int_to_fp(ty, x, 0, signed, IntWidth::OfElement, &mut env)
                        }
                        Each::RoundIntN(rounding, size) => {
                            let precision = if sz {
                                FrintPrecision::Double
                            } else {
                                FrintPrecision::Single
                            };
                            round_int_n(precision, x, rounding, size, &mut env)
                        }
                        Each::URecipEstimate => u64::from(urecpe(x as u32)),
                        Each::URSqrtEstimate => u64::from(ursqrte(x as u32)),
                    };
                    set_elem(&mut r, e, esize, v);
                }
            }
        }
        self.fp_commit(&env);
        match op {
            // The compares with zero clear the rest of the register whatever
            // FPCR.NEP is; the estimates, FCVTXN and the conversions merge
            // into Vd (`elements == 1 && IsMerging(FPCR())`).
            Op::Each(Each::Zero(_)) => self.write_v(rd, r),
            Op::NarrowOdd if scalar => {
                self.fp_set_merging(rd as u32, rd as u32, 0xFFFF_FFFF, r as u64)
            }
            Op::Each(_) if scalar => self.fp_set_merging(rd as u32, rd as u32, ty.mask(), r as u64),
            Op::Each(_) | Op::Narrow | Op::NarrowOdd | Op::NarrowBf16 | Op::Long => {
                self.write_v(rd, r)
            }
        }
        Ok(Flow::Next)
    }

    /// SCVTF, UCVTF, FCVTZS, FCVTZU (vector and scalar, fixed point):
    /// shift-by-immediate opcodes 11100 and 11111, with
    /// `fbits = 2 × esize − immh:immb`.
    pub(crate) fn exec_simd_shift_fixed(&mut self, pc: u64, insn: u32, scalar: bool) -> ExecResult {
        let q = bit(insn, 30);
        let u = bit(insn, 29);
        let immh = bits(insn, 22, 19);
        let to_float = bits(insn, 15, 11) == 0b11100;
        // immh 001x is half precision, UNDEFINED without FEAT_FP16 (the
        // decode pseudocode of each page); 000x is unallocated.
        let ty = if immh & 0b1000 != 0 {
            FpType::Double
        } else if immh & 0b0100 != 0 {
            FpType::Single
        } else if immh & 0b0010 != 0 && self.implements(Feature::Fp16) {
            FpType::Half
        } else {
            return self.undefined(pc, insn);
        };
        if !scalar && ty == FpType::Double && !q {
            return self.undefined(pc, insn);
        }
        self.simd_check()?;
        let esize = ty.esize();
        let fbits = 2 * esize - bits(insn, 22, 16);
        let n = self.v[bits(insn, 9, 5) as usize];
        let rd = bits(insn, 4, 0) as usize;
        let lanes = if scalar { 1 } else { datasize(q) / esize };
        let mut env = self.fp_env();
        let mut r = 0u128;
        for e in 0..lanes {
            let x = elem(n, e, esize);
            let v = if to_float {
                int_to_fp(ty, x, fbits, !u, IntWidth::OfElement, &mut env)
            } else {
                fp_to_int(
                    ty,
                    x,
                    fbits,
                    FpRounding::Zero,
                    !u,
                    IntWidth::OfElement,
                    &mut env,
                )
            };
            set_elem(&mut r, e, esize, v);
        }
        self.fp_commit(&env);
        if scalar {
            self.fp_set_merging(rd as u32, rd as u32, ty.mask(), r as u64);
        } else {
            self.write_v(rd, r);
        }
        Ok(Flow::Next)
    }

    /// FMLA, FMLS, FMUL, FMULX (by element), vector (`0 Q U 01111 1 sz L M
    /// Rm opcode H 0 Rn Rd`, and `0 Q U 01111 00 L M Rm opcode H 0 Rn Rd`
    /// for half precision) and scalar (`01 U 11111 ...`, likewise).
    pub(crate) fn exec_simd_elem_fp(&mut self, pc: u64, insn: u32, scalar: bool) -> ExecResult {
        let q = bit(insn, 30);
        let u = bit(insn, 29);
        #[derive(Clone, Copy)]
        enum Op {
            Fused(FusedMulAdd),
            Binary(FpBinary),
        }
        let op = match (bits(insn, 15, 12), u) {
            (0b0001, false) => Op::Fused(FusedMulAdd::Fmadd),
            (0b0101, false) => Op::Fused(FusedMulAdd::Fmsub),
            (0b1001, false) => Op::Binary(FpBinary::Mul),
            (0b1001, true) => Op::Binary(FpBinary::MulX),
            _ => return self.undefined(pc, insn),
        };
        // size 00 is half precision (FEAT_FP16, which the decoder checks);
        // 01 is unallocated.
        let ty = match bits(insn, 23, 22) {
            0b00 => FpType::Half,
            0b10 => FpType::Single,
            0b11 => FpType::Double,
            _ => return self.undefined(pc, insn),
        };
        let (h, l, m) = (bits(insn, 11, 11), bits(insn, 21, 21), bits(insn, 20, 20));
        // Half precision indexes with H:L:M and so reaches V0-V15 only.
        let (index, rm) = match ty {
            FpType::Half => ((h << 2) | (l << 1) | m, bits(insn, 19, 16)),
            FpType::Single => ((h << 1) | l, bits(insn, 20, 16)),
            FpType::Double if l == 0 => (h, bits(insn, 20, 16)),
            FpType::Double => return self.undefined(pc, insn),
        };
        if !scalar && ty == FpType::Double && !q {
            return self.undefined(pc, insn);
        }
        self.simd_check()?;
        let esize = ty.esize();
        let element = elem(self.v[rm as usize], index, esize);
        let n = self.v[bits(insn, 9, 5) as usize];
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let lanes = if scalar { 1 } else { datasize(q) / esize };
        let mut env = self.fp_env();
        let mut r = 0u128;
        for e in 0..lanes {
            let x = elem(n, e, esize);
            let v = match op {
                Op::Fused(form) => fp_mul_add(form, ty, elem(d, e, esize), x, element, &mut env),
                Op::Binary(op) => fp_binary(op, ty, x, element, &mut env),
            };
            set_elem(&mut r, e, esize, v);
        }
        self.fp_commit(&env);
        if scalar {
            // FMLA and FMLS merge into Vd, FMUL and FMULX into Vn.
            let merge = match op {
                Op::Fused(_) => rd as u32,
                Op::Binary(_) => bits(insn, 9, 5),
            };
            self.fp_set_merging(rd as u32, merge, ty.mask(), r as u64);
        } else {
            self.write_v(rd, r);
        }
        Ok(Flow::Next)
    }

    /// FMAXNMV, FMINNMV, FMAXV, FMINV: `0 Q U 01110 a 0 11000 opcode 10 Rn
    /// Rd`, single precision ×4 with `U = 1`, half precision ×4 or ×8 with
    /// `U = 0` (FEAT_FP16, which the decoder checks). Arm `Reduce` pairs
    /// halves recursively: `op(op(e0, e1), op(e2, e3))` for four elements.
    pub(crate) fn exec_simd_across_fp(&mut self, pc: u64, insn: u32) -> ExecResult {
        let op = match (bits(insn, 16, 12), bit(insn, 23)) {
            (0b01100, false) => FpBinary::MaxNum,
            (0b01100, true) => FpBinary::MinNum,
            (0b01111, false) => FpBinary::Max,
            (0b01111, true) => FpBinary::Min,
            _ => return self.undefined(pc, insn),
        };
        let q = bit(insn, 30);
        let ty = if bit(insn, 29) {
            FpType::Single
        } else {
            FpType::Half
        };
        if bit(insn, 22) || (ty == FpType::Single && !q) {
            return self.undefined(pc, insn);
        }
        self.simd_check()?;
        let n = self.v[bits(insn, 9, 5) as usize];
        let esize = ty.esize();
        let mut lanes = [0u64; 8];
        let mut count = (datasize(q) / esize) as usize;
        for (e, lane) in lanes.iter_mut().enumerate().take(count) {
            *lane = elem(n, e as u32, esize);
        }
        let mut env = self.fp_env();
        while count > 1 {
            for i in 0..count / 2 {
                lanes[i] = fp_binary(op, ty, lanes[2 * i], lanes[2 * i + 1], &mut env);
            }
            count /= 2;
        }
        self.fp_commit(&env);
        self.write_v(bits(insn, 4, 0) as usize, u128::from(lanes[0]));
        Ok(Flow::Next)
    }

    /// FMAXNMP, FMINNMP, FADDP, FMAXP, FMINP (scalar) of precision `ty`:
    /// `01 1 11110 a sz 11000 opcode 10 Rn Rd`, and `01 0 11110 a 0 11000
    /// opcode 10 Rn Rd` for half precision — the two low elements of Vn.
    pub(crate) fn exec_simd_pair_fp(&mut self, pc: u64, insn: u32, ty: FpType) -> ExecResult {
        let op = match (bits(insn, 16, 12), bit(insn, 23)) {
            (0b01100, false) => FpBinary::MaxNum,
            (0b01100, true) => FpBinary::MinNum,
            (0b01101, false) => FpBinary::Add,
            (0b01111, false) => FpBinary::Max,
            (0b01111, true) => FpBinary::Min,
            _ => return self.undefined(pc, insn),
        };
        self.simd_check()?;
        let esize = ty.esize();
        let n = self.v[bits(insn, 9, 5) as usize];
        let mut env = self.fp_env();
        let result = fp_binary(op, ty, elem(n, 0, esize), elem(n, 1, esize), &mut env);
        self.fp_commit(&env);
        self.write_v(bits(insn, 4, 0) as usize, u128::from(result));
        Ok(Flow::Next)
    }

    /// FMLAL, FMLSL, FMLAL2, FMLSL2 (FEAT_FHM), vector (`0 Q U 01110 S 0 1
    /// Rm 11x01 1 Rn Rd`) and by element (`0 Q U 01111 1 0 L M Rm x S 00 H 0
    /// Rn Rd`, `S` at bit 14). Each 32-bit lane of Vd accumulates the product
    /// of two half-precision elements (Arm `FPMulAddH`), negated for FMLSL:
    /// the elements of the low half of Vn, or with `U = 1` (the `2` forms)
    /// the high half, times the matching elements of Vm's same half, or by
    /// element one element of V0-V15.
    pub(crate) fn exec_simd_fmlal(&mut self, insn: u32, by_element: bool) -> ExecResult {
        let q = bit(insn, 30);
        let upper = bit(insn, 29);
        let subtract = if by_element {
            bit(insn, 14)
        } else {
            bit(insn, 23)
        };
        self.simd_check()?;
        let datasize = datasize(q);
        let half = datasize / 2;
        // Arm `Vpart[n, part]` at `datasize / 2` bits.
        let part = |v: u128| {
            let v = if upper { v >> half } else { v };
            v & ((1u128 << half) - 1)
        };
        let operand1 = part(self.v[bits(insn, 9, 5) as usize]);
        // By element, `V[Rm<3:0>]` and the element `H:L:M` of it for every
        // lane; otherwise Vm's part, element for element.
        let operand2 = if by_element {
            self.v[bits(insn, 19, 16) as usize]
        } else {
            part(self.v[bits(insn, 20, 16) as usize])
        };
        let index = (bits(insn, 11, 11) << 2) | (bits(insn, 21, 21) << 1) | bits(insn, 20, 20);
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let form = if subtract {
            FusedMulAdd::Fmsub
        } else {
            FusedMulAdd::Fmadd
        };
        let mut env = self.fp_env();
        let mut r = 0u128;
        for e in 0..datasize / 32 {
            let a = elem(operand1, e, 16) as u16;
            let b = elem(operand2, if by_element { index } else { e }, 16) as u16;
            let acc = elem(d, e, 32) as u32;
            set_elem(
                &mut r,
                e,
                32,
                u64::from(sf::f16_mul_add_long(form, acc, a, b, &mut env.status)),
            );
        }
        self.fp_commit(&env);
        self.write_v(rd, r);
        Ok(Flow::Next)
    }

    /// The element type FCMLA and FCADD read from `size`: `if size == '00'
    /// then EndOfDecode(Decode_UNDEF)`, and half precision (`01`) only
    /// with FEAT_FP16.
    fn complex_type(&self, size: u32) -> Option<FpType> {
        match size {
            0b01 if self.implements(Feature::Fp16) => Some(FpType::Half),
            0b10 => Some(FpType::Single),
            0b11 => Some(FpType::Double),
            _ => None,
        }
    }

    /// FCMLA (FEAT_FCMA), vector (`0 Q 1 01110 size 0 Rm 110 rot 1 Rn Rd`)
    /// and by element (`0 Q 1 01111 size L M Rm 0 rot 1 H 0 Rn Rd`). Each
    /// pair of elements of Vd, a complex number (real, imaginary),
    /// accumulates with two fused multiply-adds (Arm FPMulAdd) the product
    /// of Vn's pair and Vm's pair (by element, the pair `index` of Vm)
    /// rotated by `rot` × 90°: the rotation picks which element of Vn's
    /// pair both halves take, and negates Vm's elements (FPNeg).
    pub(crate) fn exec_simd_fcmla(&mut self, pc: u64, insn: u32, by_element: bool) -> ExecResult {
        let q = bit(insn, 30);
        let size = bits(insn, 23, 22);
        let h = bits(insn, 11, 11);
        let l = bits(insn, 21, 21);
        let refused = if by_element {
            size == 0b11 || (size == 0b10 && (l == 1 || !q)) || (size == 0b01 && h == 1 && !q)
        } else {
            size == 0b11 && !q
        };
        let Some(ty) = self.complex_type(size).filter(|_| !refused) else {
            return self.undefined(pc, insn);
        };
        self.simd_check()?;
        let (rm, rot, index) = if by_element {
            let index = if size == 0b01 { (h << 1) | l } else { h };
            (bits(insn, 20, 16), bits(insn, 14, 13), Some(index))
        } else {
            (bits(insn, 20, 16), bits(insn, 12, 11), None)
        };
        let esize = ty.esize();
        let n = self.v[bits(insn, 9, 5) as usize];
        let m = self.v[rm as usize];
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let mut env = self.fp_env();
        let mut r = 0u128;
        for e in 0..datasize(q) / esize / 2 {
            let mut neg = |x: u64| fp_unary(FpUnary::Neg, ty, x, &mut env);
            let pair = index.unwrap_or(e);
            let (m_real, m_imag) = (elem(m, 2 * pair, esize), elem(m, 2 * pair + 1, esize));
            let (n_real, n_imag) = (elem(n, 2 * e, esize), elem(n, 2 * e + 1, esize));
            // Arm's element1 to element4: Vm's factor and Vn's for the real
            // half, then for the imaginary half.
            let (element1, element2, element3, element4) = match rot {
                0b00 => (m_real, n_real, m_imag, n_real),
                0b01 => (neg(m_imag), n_imag, m_real, n_imag),
                0b10 => (neg(m_real), n_real, neg(m_imag), n_real),
                _ => (m_imag, n_imag, neg(m_real), n_imag),
            };
            let real = fp_mul_add(
                FusedMulAdd::Fmadd,
                ty,
                elem(d, 2 * e, esize),
                element2,
                element1,
                &mut env,
            );
            let imag = fp_mul_add(
                FusedMulAdd::Fmadd,
                ty,
                elem(d, 2 * e + 1, esize),
                element4,
                element3,
                &mut env,
            );
            set_elem(&mut r, 2 * e, esize, real);
            set_elem(&mut r, 2 * e + 1, esize, imag);
        }
        self.fp_commit(&env);
        self.write_v(rd, r);
        Ok(Flow::Next)
    }

    /// FCADD (FEAT_FCMA): `0 Q 1 01110 size 0 Rm 111 rot 0 1 Rn Rd`. Each
    /// pair of elements of Vn, a complex number, adds Vm's pair rotated by
    /// 90° (`rot = 0`) or 270° (Arm FPAdd, FPNeg).
    pub(crate) fn exec_simd_fcadd(&mut self, pc: u64, insn: u32) -> ExecResult {
        let q = bit(insn, 30);
        let size = bits(insn, 23, 22);
        let Some(ty) = self.complex_type(size).filter(|_| !(size == 0b11 && !q)) else {
            return self.undefined(pc, insn);
        };
        self.simd_check()?;
        let rot = bit(insn, 12);
        let esize = ty.esize();
        let n = self.v[bits(insn, 9, 5) as usize];
        let m = self.v[bits(insn, 20, 16) as usize];
        let mut env = self.fp_env();
        let mut r = 0u128;
        for e in 0..datasize(q) / esize / 2 {
            let mut neg = |x: u64| fp_unary(FpUnary::Neg, ty, x, &mut env);
            let (m_real, m_imag) = (elem(m, 2 * e, esize), elem(m, 2 * e + 1, esize));
            let (element1, element3) = if rot {
                (m_imag, neg(m_real))
            } else {
                (neg(m_imag), m_real)
            };
            let real = fp_binary(FpBinary::Add, ty, elem(n, 2 * e, esize), element1, &mut env);
            let imag = fp_binary(
                FpBinary::Add,
                ty,
                elem(n, 2 * e + 1, esize),
                element3,
                &mut env,
            );
            set_elem(&mut r, 2 * e, esize, real);
            set_elem(&mut r, 2 * e + 1, esize, imag);
        }
        self.fp_commit(&env);
        self.write_v(bits(insn, 4, 0) as usize, r);
        Ok(Flow::Next)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/fp16.rs"]
mod fp16_tests;
