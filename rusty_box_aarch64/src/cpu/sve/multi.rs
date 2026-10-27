//! FEAT_SME2's multi-vector SVE operations (`mortlach_multi_sve_*`), on
//! groups of two or four consecutive vectors whose first register is a
//! multiple of the group's size: SEL under a predicate-as-counter; SMAX,
//! SMIN, UMAX, UMIN, SRSHL, URSHL, ADD and SQDMULH, and FMAX, FMIN, FMAXNM
//! and FMINNM, of a group by one vector or by a group; SCLAMP, UCLAMP and
//! FCLAMP; ZIP and UZP of elements and of quadwords; SQRSHR, SQRSHRU,
//! UQRSHR and their interleaving forms, SQCVT, SQCVTU, UQCVT and theirs;
//! BFCVT, FCVT, BFCVTN and FCVTN; FCVTZS, FCVTZU, SCVTF, UCVTF, FRINTN,
//! FRINTP, FRINTM and FRINTA of words; SUNPK and UUNPK. And the SVE2.1
//! instructions FEAT_SME2 makes legal that share their operations: the
//! interleaving SQRSHRN, SQRSHRUN, UQRSHRN, SQCVTN, SQCVTUN and UQCVTN of
//! two vectors (`sve_intx_multi_shift_narrow`,
//! `sve_intx_multi_extract_narrow`) and FCLAMP of one (`sve_fp_clamp`).
//!
//! Each executes in Streaming SVE mode alone (`CheckStreamingSVEEnabled`,
//! which on this processor `CheckSVEEnabled` is, `Cpu::sve_check`)
//! and reads every source register before it writes a destination, as the
//! pseudocode's `results` arrays do, so a destination group that overlaps a
//! source reads the source as it was. The floating-point operations compute
//! as the SVE ones do (`sve::fp`), under FPCR, their exceptions
//! accumulating in FPSR, and the conversion to half precision reads
//! FPCR.AHP as 0 (`FPConvertSVE`). No saturation sets FPSR.QC.
//!
//! Provenance (R7): the Arm A64 ISA XML, release 2026-09 (Arm ARM M.d), the
//! instruction pages each handler names.

#[cfg(test)]
#[path = "../../../tests/unit/multi.rs"]
mod tests;

use super::counter::PredCounter;
use super::fp::{esize, fp_type};
use super::int::BinOp;
use super::{clamp, group, int, sat_doubling_mul_high, sat_q, shift_by_element, Esize, Zv};
use crate::cpu::exec::{bit, bits, ExecResult, Flow};
use crate::cpu::features::VecLen;
use crate::cpu::fp::{
    fp_binary, fp_convert, fp_to_int, fp_unary, int_to_fp, FpBinary, FpConversion, FpType, FpUnary,
    IntWidth,
};
use crate::cpu::Cpu;
use crate::softfloat::{self as sf, FpRounding, InexactReport};

/// The registers of a multi-vector binary operation (`mortlach_multi_sve_2a`
/// to `_2d1`): `nreg` vectors from `dn`, which the result overwrites, by
/// one vector `m` (`step` 0) or by the group from `m` (`step` 1).
#[derive(Debug, Clone, Copy)]
struct BinaryRegs {
    dn: u32,
    nreg: usize,
    m: u32,
    step: u32,
}

impl BinaryRegs {
    /// Two or four vectors by bit 11 (`Zdn:'0'`, `Zdn:'00'`), by one of Z0
    /// to Z15 (`'0':Zm`, bits 19:16) or, with bit 12 set, by a group of as
    /// many (`Zm:'0'`, `Zm:'00'`, bits 20:17 or 20:18).
    fn of(insn: u32) -> Self {
        let nreg = if bit(insn, 11) { 4 } else { 2 };
        let (m, step) = if bit(insn, 12) {
            (group(insn, 20, nreg), 1)
        } else {
            (bits(insn, 19, 16), 0)
        };
        Self {
            dn: group(insn, 4, nreg),
            nreg,
            m,
            step,
        }
    }
}

/// An integer operation of `mortlach_multi_sve_2a` to `_2d1`.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy)]
enum IntBinary {
    /// SMAX, SMIN, UMAX, UMIN and ADD, as SVE's.
    Sve(BinOp),
    /// SRSHL, URSHL: shifted by `ShiftSat(SInt(b), esize)`, left or
    /// (negative) right, a right shift rounding; truncated.
    RoundingShift { unsigned: bool },
    /// SQDMULH: `SignedSat((2 * a * b) >> esize)`.
    DoublingMulHigh,
}

impl IntBinary {
    /// The operation by bit 10 (SQDMULH, with bits 9:5 clear), then bits
    /// 9:8 and 7:5: 00 and 000 the maximum, 00 and 001 the minimum, 10 and
    /// 001 the rounding shift, 11 and 000 the addition; signed or unsigned
    /// by `U` (bit 0). `None` for the rest, which the decoder names no
    /// integer operation of.
    fn of(insn: u32) -> Option<Self> {
        let unsigned = bit(insn, 0);
        Some(match (bit(insn, 10), bits(insn, 9, 8), bits(insn, 7, 5)) {
            (true, 0b00, 0b000) => Self::DoublingMulHigh,
            (false, 0b00, 0b000) => Self::Sve(BinOp::Max { unsigned }),
            (false, 0b00, 0b001) => Self::Sve(BinOp::Min { unsigned }),
            (false, 0b10, 0b001) => Self::RoundingShift { unsigned },
            (false, 0b11, 0b000) => Self::Sve(BinOp::Add),
            _ => return None,
        })
    }

    fn apply(self, a: u64, b: u64, es: Esize) -> u64 {
        match self {
            Self::Sve(op) => op.apply(a, b, es),
            Self::RoundingShift { unsigned } => shift_by_element(a, b, es, unsigned, true) as u64,
            Self::DoublingMulHigh => sat_doubling_mul_high(a, b, es, false),
        }
    }
}

/// What a clamp of `mortlach_multi_sve_3` clamps.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy)]
enum Clamp {
    /// SCLAMP, UCLAMP (`unsigned`): `Min(Max(Zn, Zd), Zm)`.
    Int { unsigned: bool },
    /// FCLAMP: `FPMinNum(FPMaxNum(Zn, Zd), Zm)`.
    Float(FpType),
}

/// The registers of a clamp of `mortlach_multi_sve_3`: `nreg` vectors from
/// `d`, each element clamped between `Zn`'s and `Zm`'s.
#[derive(Debug, Clone, Copy)]
struct ClampRegs {
    d: u32,
    nreg: usize,
    n: u32,
    m: u32,
}

impl ClampRegs {
    /// Two or four vectors by bit 11 (`Zd:'0'`, `Zd:'00'`), between `Zn`
    /// (bits 9:5) and `Zm` (bits 20:16).
    fn of(insn: u32) -> Self {
        let nreg = if bit(insn, 11) { 4 } else { 2 };
        Self {
            d: group(insn, 4, nreg),
            nreg,
            n: bits(insn, 9, 5),
            m: bits(insn, 20, 16),
        }
    }
}

/// What a saturating narrow of `mortlach_multi_sve_3` or `_4` makes of
/// `nreg` vectors: elements of `es` from elements `nreg` times as wide, of
/// the group from `n`, to `d`.
#[derive(Debug, Clone, Copy)]
struct Narrow {
    n: u32,
    d: u32,
    nreg: usize,
    es: Esize,
    /// A source element is `UInt`, else `SInt`.
    unsigned_source: bool,
    /// The result saturates to an unsigned range, else a signed one.
    unsigned_result: bool,
    /// A rounding shift right by this many bits first; none at 0.
    shift: u32,
    /// Element `e` of register `r` goes to `nreg * e + r`, else to
    /// `r * elements + e`.
    interleave: bool,
}

/// An operation on words of `mortlach_multi_sve_4`'s conversions and
/// roundings.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy)]
enum WordOp {
    /// FCVTZS, FCVTZU: `FPToFixed` rounding toward zero.
    ToInt { signed: bool },
    /// SCVTF, UCVTF: `FixedToFP` by FPCR's rounding.
    FromInt { signed: bool },
    /// FRINTN, FRINTP, FRINTM, FRINTA: `FPRoundInt`, not exact.
    Round(FpRounding),
}

impl Cpu {
    /// Writes `results`, which an instruction computed from its sources
    /// first, to the registers from `first` up.
    fn write_group(&mut self, first: u32, results: &[Zv], vl: VecLen) {
        for (r, result) in (0u32..).zip(results) {
            self.set_z(first + r, vl, result);
        }
    }

    /// The binary operation `op` on elements of `es` of the group `regs`
    /// names: element `e` of its register `r` with element `e` of `Zm` (or
    /// of `Zm + r`), each result to `Zdn + r`.
    fn multi_binary<F: FnMut(u64, u64) -> u64>(
        &mut self,
        regs: BinaryRegs,
        es: Esize,
        vl: VecLen,
        mut op: F,
    ) {
        let mut results = [Zv::ZERO; 4];
        for (r, result) in (0u32..).zip(results[..regs.nreg].iter_mut()) {
            let (a, b) = (self.z(regs.dn + r, vl), self.z(regs.m + r * regs.step, vl));
            for e in 0..es.elements(vl) {
                result.set(e, es, op(a.get(e, es), b.get(e, es)));
            }
        }
        self.write_group(regs.dn, &results[..regs.nreg], vl);
    }

    /// SEL of two or four vectors (`sel_mz_p_zz_2`, `_4`: `11000001 size 1
    /// Zm 0|01 100 PNv Zn 0|00 Zd 0|00`, four by bit 16): element `e` of
    /// register `r` from the group from `Zn` where the predicate-as-counter
    /// PNv (PN8 to PN15) makes its element `r * elements + e` active, else
    /// from the group from `Zm`.
    pub(crate) fn exec_sve_multi_sel(&mut self, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let nreg = if bit(insn, 16) { 4 } else { 2 };
        let (n, m, d) = (
            group(insn, 9, nreg),
            group(insn, 20, nreg),
            group(insn, 4, nreg),
        );
        let v = 8 + bits(insn, 12, 10);
        let vl = self.check_streaming_sve_enabled()?;
        let counter = PredCounter::of(&self.p(v, vl), vl);
        let elements = es.elements(vl);
        let mut results = [Zv::ZERO; 4];
        for (r, result) in (0u32..).zip(results[..nreg].iter_mut()) {
            let (a, b) = (self.z(n + r, vl), self.z(m + r, vl));
            for e in 0..elements {
                let source = if counter.active(r as usize * elements + e, es) {
                    &a
                } else {
                    &b
                };
                result.set(e, es, source.get(e, es));
            }
        }
        self.write_group(d, &results[..nreg], vl);
        Ok(Flow::Next)
    }

    /// SMAX, SMIN, UMAX, UMIN, SRSHL, URSHL, ADD and SQDMULH of two or four
    /// vectors by one or by as many (`smax_mz_zzv_2x1` to
    /// `sqdmulh_mz_zzw_4x4`: [`BinaryRegs`], [`IntBinary`]), the results
    /// to the group from `Zdn`.
    pub(crate) fn exec_sve_multi_int_binary(&mut self, pc: u64, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let regs = BinaryRegs::of(insn);
        let Some(op) = IntBinary::of(insn) else {
            return self.undefined(pc, insn);
        };
        let vl = self.check_streaming_sve_enabled()?;
        self.multi_binary(regs, es, vl, |a, b| op.apply(a, b, es));
        Ok(Flow::Next)
    }

    /// FMAX, FMIN, FMAXNM, FMINNM of two or four vectors by one or by as
    /// many (`fmax_mz_zzv_2x1` to `fminnm_mz_zzw_4x4`: [`BinaryRegs`]): the
    /// number forms by bit 5, the minimum by bit 0; `size = 00` is
    /// FEAT_SVE_B16B16's BFloat16 forms, which the decoder does not name.
    pub(crate) fn exec_sve_multi_fp_binary(&mut self, pc: u64, insn: u32) -> ExecResult {
        let Some(ty) = fp_type(bits(insn, 23, 22)) else {
            return self.undefined(pc, insn);
        };
        let op = match (bit(insn, 5), bit(insn, 0)) {
            (false, false) => FpBinary::Max,
            (false, true) => FpBinary::Min,
            (true, false) => FpBinary::MaxNum,
            (true, true) => FpBinary::MinNum,
        };
        let regs = BinaryRegs::of(insn);
        let vl = self.check_streaming_sve_enabled()?;
        let mut env = self.fp_env();
        self.multi_binary(regs, esize(ty), vl, |a, b| {
            fp_binary(op, ty, a, b, &mut env)
        });
        self.fp_commit(&env);
        Ok(Flow::Next)
    }

    /// SCLAMP, UCLAMP and FCLAMP of two or four vectors (`fclamp_mz_zz_2`
    /// to `uclamp_mz_zz_4`: `11000001 size 1 Zm 110 0 x x Zn Zd x U|0`,
    /// four by bit 11, an integer clamp by bit 10): each element of the
    /// group from `Zd` clamped between `Zn`'s below and `Zm`'s above
    /// ([`Clamp`]).
    pub(crate) fn exec_sve_multi_clamp(&mut self, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 23, 22);
        let kind = if bit(insn, 10) {
            Clamp::Int {
                unsigned: bit(insn, 0),
            }
        } else {
            let Some(ty) = fp_type(size) else {
                return self.undefined(pc, insn);
            };
            Clamp::Float(ty)
        };
        let regs = ClampRegs::of(insn);
        let vl = self.check_streaming_sve_enabled()?;
        let es = Esize::from_size(size);
        match kind {
            Clamp::Int { unsigned } => self.multi_clamp(regs, es, vl, |x, low, high| {
                clamp(x, low, high, es, unsigned)
            }),
            Clamp::Float(ty) => self.fp_clamp(regs, ty, es, vl),
        }
        Ok(Flow::Next)
    }

    /// The clamp `op` of each element of the group `regs` names, `op(x,
    /// low, high)` with `Zn`'s element below and `Zm`'s above.
    fn multi_clamp<F: FnMut(u64, u64, u64) -> u64>(
        &mut self,
        regs: ClampRegs,
        es: Esize,
        vl: VecLen,
        mut op: F,
    ) {
        let (low, high) = (self.z(regs.n, vl), self.z(regs.m, vl));
        let mut results = [Zv::ZERO; 4];
        for (r, result) in (0u32..).zip(results[..regs.nreg].iter_mut()) {
            let value = self.z(regs.d + r, vl);
            for e in 0..es.elements(vl) {
                result.set(e, es, op(value.get(e, es), low.get(e, es), high.get(e, es)));
            }
        }
        self.write_group(regs.d, &results[..regs.nreg], vl);
    }

    /// ZIP and UZP of two vectors into two (`zip_mz_zz_2`, `uzp_mz_zz_2`,
    /// and of quadwords `_2q`: `11000001 size 1 Zm 110 10x Zn Zd op`, by bit
    /// 0) and of four into four (`zip_mz_z_4`, `uzp_mz_z_4`, and of
    /// quadwords `_4q`: `11000001 size 1 101 1x 111000 Zn 00 Zd op 0`, by
    /// bit 1). ZIP interleaves the sources' elements, each destination
    /// taking its slice of each source in turn; UZP undoes it. A quadword
    /// form, and four vectors of doublewords, need a longest vector length
    /// that holds them (else UNDEFINED before the enable check), and the
    /// current vector length must hold `nreg` elements (else UNDEFINED
    /// after it).
    pub(crate) fn exec_sve_multi_zip(&mut self, pc: u64, insn: u32) -> ExecResult {
        let four = bit(insn, 13);
        let (quadwords, unzip) = if four {
            (bits(insn, 17, 16) == 0b11, bit(insn, 1))
        } else {
            (bits(insn, 12, 10) == 0b101, bit(insn, 0))
        };
        // An element moves whole, a quadword as two doublewords.
        let (es, parts) = if quadwords {
            (Esize::D, 2)
        } else {
            (Esize::from_size(bits(insn, 23, 22)), 1)
        };
        let bytes = es.bytes() * parts;
        let nreg = if four { 4 } else { 2 };
        // `MaxImplementedSVL() < 256` (`< 512` four quadwords, `size ==
        // '11'` four doublewords) is UNDEFINED in the decode.
        let least_max_svl = match (four, quadwords, bytes) {
            (false, true, _) => 256,
            (true, true, _) => 512,
            (true, false, 8) => 256,
            _ => 0,
        };
        if (self.imp.max_svl.bits() as usize) < least_max_svl {
            return self.undefined(pc, insn);
        }
        let d = group(insn, 4, nreg);
        let mut sources = [0u32; 4];
        if four {
            let n = group(insn, 9, 4);
            for (r, source) in (0u32..).zip(sources.iter_mut()) {
                *source = n + r;
            }
        } else {
            sources[0] = bits(insn, 9, 5);
            sources[1] = bits(insn, 20, 16);
        }
        let vl = self.check_streaming_sve_enabled()?;
        if vl.bytes() < bytes * nreg {
            return self.undefined(pc, insn);
        }
        let mut operands = [Zv::ZERO; 4];
        for (operand, &source) in operands.iter_mut().zip(&sources[..nreg]) {
            *operand = self.z(source, vl);
        }
        // The elements each destination takes from each source: `pairs` or
        // `quads`.
        let slice = vl.bytes() / (bytes * nreg);
        let mut results = [Zv::ZERO; 4];
        for r in 0..nreg {
            for p in 0..slice {
                for i in 0..nreg {
                    let (to, to_element, from, from_element) = if unzip {
                        (i, r * slice + p, r, nreg * p + i)
                    } else {
                        (r, nreg * p + i, i, r * slice + p)
                    };
                    for k in 0..parts {
                        results[to].set(
                            to_element * parts + k,
                            es,
                            operands[from].get(from_element * parts + k, es),
                        );
                    }
                }
            }
        }
        self.write_group(d, &results[..nreg], vl);
        Ok(Flow::Next)
    }

    /// The saturating narrow `narrow` describes, to `Zd`.
    fn multi_narrow(&mut self, narrow: Narrow, vl: VecLen) {
        let wide = Esize::from_size(narrow.es.log2_bytes() + narrow.nreg.trailing_zeros());
        let elements = wide.elements(vl);
        let mut result = Zv::ZERO;
        for r in 0..narrow.nreg {
            let operand = self.z(narrow.n + r as u32, vl);
            for e in 0..elements {
                let value = int(operand.get(e, wide), wide, narrow.unsigned_source);
                let value = match narrow.shift {
                    0 => value,
                    shift => (value + (1i128 << (shift - 1))) >> shift,
                };
                let index = if narrow.interleave {
                    narrow.nreg * e + r
                } else {
                    r * elements + e
                };
                result.set(
                    index,
                    narrow.es,
                    sat_q(value, narrow.es, narrow.unsigned_result),
                );
            }
        }
        self.set_z(narrow.d, vl, &result);
    }

    /// SQRSHR, SQRSHRU, UQRSHR of two vectors of words to halfwords
    /// (`sqrshr_z_mz2_` to `uqrshr_z_mz2_`: `11000001 111 op imm4 110101 Zn
    /// U Zd`, a shift of `16 - imm4`), and of four vectors to a quarter of
    /// their element size, with SQRSHRN, SQRSHRUN, UQRSHRN interleaving
    /// (`sqrshr_z_mz4_` to `uqrshrn_z_mz4_`: `11000001 tsize 1 imm5 110 11 N
    /// Zn op U Zd`, a shift of `8 * esize - (tsize:imm5)`; `tsize = 00` is
    /// UNDEFINED): each element rounded and shifted right, then saturated,
    /// a signed source to an unsigned result by `op`.
    pub(crate) fn exec_sve_multi_narrow_shift(&mut self, pc: u64, insn: u32) -> ExecResult {
        let unsigned = bit(insn, 5);
        let narrow = if bit(insn, 11) {
            let tsize = bits(insn, 23, 22);
            if tsize == 0 {
                return self.undefined(pc, insn);
            }
            let es = Esize::from_tsize(tsize);
            Narrow {
                n: group(insn, 9, 4),
                d: bits(insn, 4, 0),
                nreg: 4,
                es,
                unsigned_source: unsigned,
                unsigned_result: unsigned || bit(insn, 6),
                shift: 8 * es.bits() - ((tsize << 5) | bits(insn, 20, 16)),
                interleave: bit(insn, 10),
            }
        } else {
            Narrow {
                n: group(insn, 9, 2),
                d: bits(insn, 4, 0),
                nreg: 2,
                es: Esize::H,
                unsigned_source: unsigned,
                unsigned_result: unsigned || bit(insn, 20),
                shift: 16 - bits(insn, 19, 16),
                interleave: false,
            }
        };
        let vl = self.check_streaming_sve_enabled()?;
        self.multi_narrow(narrow, vl);
        Ok(Flow::Next)
    }

    /// SQCVT, SQCVTU, UQCVT of two vectors of words to halfwords
    /// (`sqcvt_z_mz2_` to `uqcvt_z_mz2_`: `11000001 0 op 1 00011 111000 Zn
    /// U Zd`) and of four vectors to a quarter of their element size, bytes
    /// or halfwords by `sz` (bit 23), with SQCVTN, SQCVTUN, UQCVTN
    /// interleaving (`sqcvt_z_mz4_` to `uqcvtn_z_mz4_`: `11000001 sz op 1
    /// 10011 111000 Zn N U Zd`): each element saturated, a signed source to
    /// an unsigned result by `op`.
    pub(crate) fn exec_sve_multi_narrow_int(&mut self, insn: u32) -> ExecResult {
        let unsigned = bit(insn, 5);
        let four = bit(insn, 20);
        let narrow = Narrow {
            n: group(insn, 9, if four { 4 } else { 2 }),
            d: bits(insn, 4, 0),
            nreg: if four { 4 } else { 2 },
            // Halfwords from words; from four vectors, `8 << UInt(sz)`.
            es: if four && !bit(insn, 23) {
                Esize::B
            } else {
                Esize::H
            },
            unsigned_source: unsigned,
            unsigned_result: unsigned || bit(insn, 22),
            shift: 0,
            interleave: four && bit(insn, 6),
        };
        let vl = self.check_streaming_sve_enabled()?;
        self.multi_narrow(narrow, vl);
        Ok(Flow::Next)
    }

    /// SVE2.1's SQRSHRN, SQRSHRUN and UQRSHRN (`sqrshrn_z_mz2_` to
    /// `uqrshrn_z_mz2_`: `01000101 101 1 imm4 00 op1 U 1 0 Zn 0 Zd`, a shift
    /// of `16 - imm4`) and SQCVTN, SQCVTUN and UQCVTN (`sqcvtn_z_mz2_` to
    /// `uqcvtn_z_mz2_`: `01000101 001 10 001 010 opc 0 Zn 0 Zd`), by bit
    /// 23: the two vectors of words from `Zn:'0'` to halfwords, interleaved,
    /// each element rounded and shifted right or not, then saturated. The
    /// source is unsigned by `U` (bit 12) or `opc<0>` (bit 11), and a signed
    /// source saturates to an unsigned result by `op1 = 0` or `opc<1>`.
    pub(crate) fn exec_sve_narrow_pair(&mut self, insn: u32) -> ExecResult {
        let (unsigned_source, unsigned_result, shift) = if bit(insn, 23) {
            let unsigned = bit(insn, 12);
            (
                unsigned,
                unsigned || !bit(insn, 13),
                16 - bits(insn, 19, 16),
            )
        } else {
            let unsigned = bit(insn, 11);
            (unsigned, unsigned || bit(insn, 12), 0)
        };
        let narrow = Narrow {
            n: group(insn, 9, 2),
            d: bits(insn, 4, 0),
            nreg: 2,
            es: Esize::H,
            unsigned_source,
            unsigned_result,
            shift,
            interleave: true,
        };
        let vl = self.sve_check()?;
        self.multi_narrow(narrow, vl);
        Ok(Flow::Next)
    }

    /// SVE2.1's FCLAMP of one vector (`fclamp_z_zz_`: `01100100 size 1 Zm
    /// 001001 Zn Zd`): each element of `Zd` clamped between `Zn`'s below
    /// and `Zm`'s above, `FPMinNum(FPMaxNum(Zn, Zd), Zm)` under FPCR, as of
    /// a group. `size = 00`, BFCLAMP, the decoder names no encoding of.
    pub(crate) fn exec_sve_fp_clamp(&mut self, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 23, 22);
        let Some(ty) = fp_type(size) else {
            return self.undefined(pc, insn);
        };
        let regs = ClampRegs {
            d: bits(insn, 4, 0),
            nreg: 1,
            n: bits(insn, 9, 5),
            m: bits(insn, 20, 16),
        };
        let vl = self.sve_check()?;
        self.fp_clamp(regs, ty, Esize::from_size(size), vl);
        Ok(Flow::Next)
    }

    /// FCLAMP of the vectors `regs` names, `FPMinNum(FPMaxNum(Zn, Zd), Zm)`
    /// of `ty`, its exceptions accumulating in FPSR.
    fn fp_clamp(&mut self, regs: ClampRegs, ty: FpType, es: Esize, vl: VecLen) {
        let mut env = self.fp_env();
        self.multi_clamp(regs, es, vl, |x, low, high| {
            let max = fp_binary(FpBinary::MaxNum, ty, low, x, &mut env);
            fp_binary(FpBinary::MinNum, ty, max, high, &mut env)
        });
        self.fp_commit(&env);
    }

    /// BFCVT, FCVT, BFCVTN, FCVTN of two vectors (`bfcvt_z_mz2_` to
    /// `fcvtn_z_mz2_`: `11000001 0 op 1 00000 111000 Zn N Zd`): their
    /// single-precision elements to BFloat16 (`op`, `FPConvertBF`) or half
    /// precision (`FPConvertSVE`), the first vector's results then the
    /// second's, or interleaved (`N`).
    pub(crate) fn exec_sve_multi_narrow_fp(&mut self, insn: u32) -> ExecResult {
        let bf16 = bit(insn, 22);
        let interleave = bit(insn, 5);
        let (n, d) = (group(insn, 9, 2), bits(insn, 4, 0));
        let vl = self.check_streaming_sve_enabled()?;
        let mut env = self.fp_env().with_ieee_half();
        let (first, second) = (self.z(n, vl), self.z(n + 1, vl));
        let elements = Esize::S.elements(vl);
        let mut result = Zv::ZERO;
        for e in 0..elements {
            let mut convert = |a: u64| {
                if bf16 {
                    let rounding = env.rounding();
                    u64::from(sf::f32_to_bf16(a as u32, rounding, &mut env.status))
                } else {
                    fp_convert(FpConversion::SingleToHalf, a, &mut env)
                }
            };
            let (r1, r2) = (
                convert(first.get(e, Esize::S)),
                convert(second.get(e, Esize::S)),
            );
            let (i1, i2) = if interleave {
                (2 * e, 2 * e + 1)
            } else {
                (e, elements + e)
            };
            result.set(i1, Esize::H, r1);
            result.set(i2, Esize::H, r2);
        }
        self.fp_commit(&env);
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// FCVTZS, FCVTZU, SCVTF, UCVTF, FRINTN, FRINTP, FRINTM, FRINTA of two
    /// or four vectors of words (`fcvtzs_mz_z_2` to `frinta_mz_z_4`: four
    /// by bit 20; a rounding by bit 19, by `opc` (bits 18:16); else a
    /// conversion to an integer by bits 17:16 = 01, from one by 10;
    /// unsigned by `U`, bit 5), to the group from `Zd` ([`WordOp`]).
    pub(crate) fn exec_sve_multi_fp_unary(&mut self, pc: u64, insn: u32) -> ExecResult {
        let signed = !bit(insn, 5);
        let op = match (bit(insn, 19), bits(insn, 18, 16)) {
            (true, 0b000) => WordOp::Round(FpRounding::TieEven),
            (true, 0b001) => WordOp::Round(FpRounding::PosInf),
            (true, 0b010) => WordOp::Round(FpRounding::NegInf),
            (true, 0b100) => WordOp::Round(FpRounding::TieAway),
            (false, 0b001 | 0b101) => WordOp::ToInt { signed },
            (false, 0b010 | 0b110) => WordOp::FromInt { signed },
            _ => return self.undefined(pc, insn),
        };
        let nreg = if bit(insn, 20) { 4 } else { 2 };
        let (n, d) = (group(insn, 9, nreg), group(insn, 4, nreg));
        let vl = self.check_streaming_sve_enabled()?;
        let mut env = self.fp_env();
        let mut results = [Zv::ZERO; 4];
        for (r, result) in (0u32..).zip(results[..nreg].iter_mut()) {
            let operand = self.z(n + r, vl);
            for e in 0..Esize::S.elements(vl) {
                let a = operand.get(e, Esize::S);
                let value = match op {
                    WordOp::ToInt { signed } => fp_to_int(
                        FpType::Single,
                        a,
                        0,
                        FpRounding::Zero,
                        signed,
                        IntWidth::Bits32,
                        &mut env,
                    ),
                    WordOp::FromInt { signed } => {
                        int_to_fp(FpType::Single, a, 0, signed, IntWidth::Bits32, &mut env)
                    }
                    WordOp::Round(rounding) => fp_unary(
                        FpUnary::RoundInt(rounding, InexactReport::Suppress),
                        FpType::Single,
                        a,
                        &mut env,
                    ),
                };
                result.set(e, Esize::S, value);
            }
        }
        self.fp_commit(&env);
        self.write_group(d, &results[..nreg], vl);
        Ok(Flow::Next)
    }

    /// SUNPK and UUNPK of one vector to two (`sunpk_mz_z_2`, `uunpk_mz_z_2`:
    /// `11000001 size 1 00101 111000 Zn Zd U`) and of two to four
    /// (`sunpk_mz_z_4`, `uunpk_mz_z_4`: `... 10101 111000 Zn 0 Zd 0 U`):
    /// source register `r`'s low half to destination `2 * r` and its high
    /// half to `2 * r + 1`, each element sign- or zero-extended (`U`) to
    /// twice its size. `size = 00` is UNDEFINED.
    pub(crate) fn exec_sve_multi_unpack(&mut self, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 23, 22);
        if size == 0 {
            return self.undefined(pc, insn);
        }
        let (es, half) = (Esize::from_size(size), Esize::from_size(size - 1));
        let unsigned = bit(insn, 0);
        let four = bit(insn, 20);
        let (n, d, nreg) = if four {
            (group(insn, 9, 2), group(insn, 4, 4), 4)
        } else {
            (bits(insn, 9, 5), group(insn, 4, 2), 2)
        };
        let vl = self.check_streaming_sve_enabled()?;
        let elements = es.elements(vl);
        let mut results = [Zv::ZERO; 4];
        for r in 0..nreg / 2 {
            let operand = self.z(n + r as u32, vl);
            for (i, result) in results[2 * r..2 * r + 2].iter_mut().enumerate() {
                for e in 0..elements {
                    result.set(
                        e,
                        es,
                        int(operand.get(i * elements + e, half), half, unsigned) as u64,
                    );
                }
            }
        }
        self.write_group(d, &results[..nreg], vl);
        Ok(Flow::Next)
    }
}
