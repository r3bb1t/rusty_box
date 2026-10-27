//! FEAT_SME2's multi-vector instructions on ZA array vectors
//! (`mortlach_multi_array_1a` to `_2b`): the products, dot products and
//! sums of one, two or four vectors with one vector or with as many, into
//! groups of ZA's horizontal vectors.
//!
//! An instruction on `nreg` registers divides ZA's `SVL DIV 8` vectors into
//! `nreg` strides of `vstride = SVL DIV 8 DIV nreg`; it selects vector
//! `(UInt(Wv) + offset) MOD vstride` of the first stride, `Wv` one of W8 to
//! W11, and register `r`'s results go to the same vector of stride `r`. A
//! long operation's results span two or four consecutive vectors from
//! there, the index rounded down to a multiple of two or four, element `e`
//! of vector `i` from the source elements `width * e + i`. A single source
//! register `Zn` names a group that wraps from Z31 to Z0.
//!
//! Each executes in Streaming SVE mode with ZA storage enabled
//! (`CheckStreamingSVEAndZAEnabled`). The floating-point ones compute as
//! `FPMulAdd_ZA`, `FPMulAddH_ZA`, `BFMulAddH_ZA`, `FPDotAdd_ZA`,
//! `FPAdd_ZA` and `FPSub_ZA` do, with FPCR.DN read as 1 and no exception,
//! so they never write FPSR; BFDOT follows BFloat16's own behaviours
//! (`BFDotAdd`), as FEAT_SME's BFMOPA does. The 64-bit forms of
//! FEAT_SME_I16I64 and FEAT_SME_F64F64 (`sz = 1`) are UNDEFINED.
//!
//! Provenance (R7): the Arm A64 ISA XML, release 2026-09 (Arm ARM M.d), the
//! instruction pages each handler names and the shared pseudocode in
//! `shared/functions/float` (`FPMulAdd_ZA`, `FPMulAddH_ZA`, `BFMulAddH_ZA`,
//! `FPDotAdd_ZA`, `BFDotAdd`, `FPAdd_ZA`, `FPSub_ZA`, `FPNeg`, `BFNeg`).

#[cfg(test)]
#[path = "../../tests/unit/za_array.rs"]
mod tests;

use super::exec::{bit, bits, ExecResult, Flow};
use super::features::VecLen;
use super::fp::{fp_binary, fp_mul_add, FpBinary, FpType};
use super::sve::{group, Esize, Zv};
use super::za::{PairFormat, Signedness};
use super::Cpu;
use crate::softfloat::FusedMulAdd;

/// The ZA array vectors a multi-vector instruction selects: its vector
/// select register, its offset and its number of registers.
#[derive(Debug, Clone, Copy)]
struct ArrayVectors {
    /// `Wv`, one of W8 to W11 (`'010':Rv`, bits 14:13).
    v: u32,
    /// The offset added to `Wv`, scaled to the vectors each register's
    /// results span.
    offset: u32,
    nreg: usize,
}

impl ArrayVectors {
    /// The vectors `insn` selects with results spanning `width` vectors per
    /// register (one, two or four). With bit 23 set (`mortlach_multi_array_2a`,
    /// `_2b`), two or four registers by bit 16. Else (`_1a`, `_1b`), one for
    /// `op1 = 0x1` (bits 12:10), both groups' `mortlach_multi1_*` classes,
    /// and two or four by bit 20 for the rest. The offset is the low bits
    /// `off3`, `off2` or `o1`, as many as leave three bits with the scale,
    /// one more for one register.
    fn of(insn: u32, width: usize) -> Self {
        let nreg = match (bit(insn, 23), bit(insn, 16), bits(insn, 12, 10), bit(insn, 20)) {
            (true, true, _, _) => 4,
            (true, false, _, _) => 2,
            (false, _, 0b001 | 0b011, _) => 1,
            (false, _, _, true) => 4,
            (false, _, _, false) => 2,
        };
        let k = width.trailing_zeros();
        let hi = 2 - k + u32::from(nreg == 1);
        Self {
            v: 8 + bits(insn, 14, 13),
            offset: bits(insn, hi, 0) << k,
            nreg,
        }
    }
}

/// Where the vectors [`ArrayVectors`] selects lie: register `r`'s span of
/// vectors starts at `first + r * vstride`.
#[derive(Debug, Clone, Copy)]
struct Span {
    first: usize,
    vstride: usize,
}

/// The second operand of a multi-vector instruction on ZA array vectors.
#[derive(Debug, Clone, Copy)]
enum Second {
    /// `'0':Zm` (bits 19:16), by every register of the group.
    Vector(u32),
    /// The group from `Zm:'0'` or `Zm:'00'` (bits 20:17 or 20:18), register
    /// `r` by `Zm + r`.
    Group(u32),
}

impl Second {
    /// The register that register `r` of the first group goes with.
    fn register(self, r: usize) -> u32 {
        match self {
            Self::Vector(m) => m,
            Self::Group(m) => m + r as u32,
        }
    }
}

/// The operands of a multi-vector instruction on ZA array vectors of two
/// sources: the vectors it selects, the group from `Zn` (register `r`
/// being `(n + r) MOD 32`) and the second operand.
#[derive(Debug, Clone, Copy)]
struct ArrayRegs {
    at: ArrayVectors,
    n: u32,
    m: Second,
}

impl ArrayRegs {
    /// The operands of `insn` with results spanning `width` vectors per
    /// register. With bit 23 set, a group by a group: `Zn:'0'` or `Zn:'00'`
    /// (bits 9:6 or 9:7) by the group from `Zm`. Else one register or a
    /// group from `Zn` (bits 9:5) by one vector.
    fn of(insn: u32, width: usize) -> Self {
        let at = ArrayVectors::of(insn, width);
        let (n, m) = if bit(insn, 23) {
            (group(insn, 9, at.nreg), Second::Group(group(insn, 20, at.nreg)))
        } else {
            (bits(insn, 9, 5), Second::Vector(bits(insn, 19, 16)))
        };
        Self { at, n, m }
    }
}

/// The integer products of `mortlach_multi*_zz_za_mla_long_*` and the dot
/// products: each source element `Int`'d by its own signedness.
#[derive(Debug, Clone, Copy)]
struct Signs {
    n: Signedness,
    m: Signedness,
}

impl Signs {
    /// SMLALL, SMLSLL, UMLALL, UMLSLL, USMLALL and SUMLALL, SMLAL to UMLSL,
    /// and SDOT, UDOT, USDOT and SUDOT: both by `U` (bit 4), or with
    /// `mixed`, `Zn`'s unsigned and `Zm`'s signed for `U = 0`, the other way
    /// for `U = 1`.
    fn of(insn: u32, mixed: bool) -> Self {
        let u = bit(insn, 4);
        if mixed {
            Self {
                n: Signedness::of(!u),
                m: Signedness::of(u),
            }
        } else {
            Self {
                n: Signedness::of(u),
                m: Signedness::of(u),
            }
        }
    }

    /// The product of element `i` of `zn` and of `zm`, both of `es`.
    fn product(self, zn: &Zv, zm: &Zv, i: usize, es: Esize) -> i128 {
        self.n.int(zn.get(i, es), es) * self.m.int(zm.get(i, es), es)
    }
}

impl Cpu {
    /// Where the vectors `at` selects lie, for results spanning `width`
    /// vectors per register: vector `(UInt(Wv) + offset) MOD vstride`,
    /// rounded down to a multiple of `width`, of each register's stride.
    fn array_span(&self, at: ArrayVectors, width: usize, svl: VecLen) -> Span {
        let vstride = svl.bytes() / at.nreg;
        let wv = u64::from(self.x_or_zr(at.v) as u32);
        let selected = ((wv + u64::from(at.offset)) % vstride as u64) as usize;
        Span {
            first: selected - selected % width,
            vstride,
        }
    }

    /// The operation `op` of a multi-vector instruction on ZA array vectors
    /// with the operands `regs`: for each register `r`, each of the `width`
    /// vectors its results span becomes `op(vector, Zn + r, its second
    /// operand, i)`, `i` the vector's place among them.
    fn za_array<F: FnMut(&Zv, &Zv, &Zv, usize) -> Zv>(
        &mut self,
        regs: ArrayRegs,
        width: usize,
        svl: VecLen,
        mut op: F,
    ) {
        let span = self.array_span(regs.at, width, svl);
        for r in 0..regs.at.nreg {
            let zn = self.z((regs.n + r as u32) % 32, svl);
            let zm = self.z(regs.m.register(r), svl);
            for i in 0..width {
                let index = span.first + r * span.vstride + i;
                let result = op(&self.za_vector(index, svl), &zn, &zm, i);
                self.set_za_vector(index, svl, &result);
            }
        }
    }

    /// Each word of the vector `at` selects in register `r`'s stride
    /// becomes `op(word, the word of Zm + r)`.
    fn za_accumulate<F: FnMut(u64, u64) -> u64>(&mut self, at: ArrayVectors, m: u32, svl: VecLen, mut op: F) {
        let span = self.array_span(at, 1, svl);
        for r in 0..at.nreg {
            let zm = self.z(m + r as u32, svl);
            let index = span.first + r * span.vstride;
            let mut vector = self.za_vector(index, svl);
            for e in 0..Esize::S.elements(svl) {
                vector.set(e, Esize::S, op(vector.get(e, Esize::S), zm.get(e, Esize::S)));
            }
            self.set_za_vector(index, svl, &vector);
        }
    }

    /// SMLALL, SMLSLL, UMLALL, UMLSLL, USMLALL and SUMLALL of one, two or
    /// four vectors by one, and of two or four by as many
    /// (`smlall_za_zzv_1` to `usmlall_za_zzw_s4x4`: [`ArrayRegs`], `U` bit
    /// 4, `S` bit 3, the mixed signs by `op`, bit 2): word `e` of ZA vector
    /// `i` of four gains, or by `S` loses, the product of byte `4 * e + i`
    /// of the register and of its second operand, modulo 2^32. `sz = 1`,
    /// FEAT_SME_I16I64's form of halfwords into doublewords, is UNDEFINED.
    pub(crate) fn exec_sme_array_int_long_long(&mut self, pc: u64, insn: u32) -> ExecResult {
        match bit(insn, 22) {
            false => self.int_mul_add_long(insn, Signs::of(insn, bit(insn, 2)), Esize::B),
            true => self.undefined(pc, insn),
        }
    }

    /// SMLAL, SMLSL, UMLAL, UMLSL of one, two or four vectors by one, and of
    /// two or four by as many (`smlal_za_zzv_1` to `umlsl_za_zzw_4x4`:
    /// [`ArrayRegs`], `U` bit 4, `S` bit 3): word `e` of ZA vector `i` of
    /// two gains, or loses, the product of halfword `2 * e + i` of the
    /// register and of its second operand, modulo 2^32.
    pub(crate) fn exec_sme_array_int_long(&mut self, insn: u32) -> ExecResult {
        self.int_mul_add_long(insn, Signs::of(insn, false), Esize::H)
    }

    /// The long integer products of `es` into words, `S` bit 3: word `e`
    /// of ZA vector `i` of the `width = 4 / es` each register's results
    /// span gains, or loses, the product of element `width * e + i`.
    fn int_mul_add_long(&mut self, insn: u32, signs: Signs, es: Esize) -> ExecResult {
        let width = Esize::S.bytes() / es.bytes();
        let subtract = bit(insn, 3);
        let regs = ArrayRegs::of(insn, width);
        let svl = self.check_streaming_sve_and_za_enabled()?;
        self.za_array(regs, width, svl, |acc, zn, zm, i| {
            let mut result = *acc;
            for e in 0..Esize::S.elements(svl) {
                let product = signs.product(zn, zm, width * e + i, es) as u64;
                let sum = acc.get(e, Esize::S);
                result.set(e, Esize::S, if subtract { sum.wrapping_sub(product) } else { sum.wrapping_add(product) });
            }
            result
        });
        Ok(Flow::Next)
    }

    /// FMLAL, FMLSL, BFMLAL, BFMLSL of one, two or four vectors by one, and
    /// of two or four by as many (`fmlal_za_zzv_1` to `bfmlsl_za_zzw_4x4`:
    /// [`ArrayRegs`], BFloat16 by `op`, bit 4, `S` bit 3): single-precision
    /// element `e` of ZA vector `i` of two plus the product of 16-bit
    /// element `2 * e + i` of the register, negated by `S`, and of its
    /// second operand, in one rounding: `FPMulAddH_ZA` and `FPNeg`, or
    /// `BFMulAddH_ZA` and `BFNeg` ([`PairFormat`]).
    pub(crate) fn exec_sme_array_fp_long(&mut self, insn: u32) -> ExecResult {
        let format = if bit(insn, 4) { PairFormat::BFloat16 } else { PairFormat::Half };
        let subtract = bit(insn, 3);
        let regs = ArrayRegs::of(insn, 2);
        let svl = self.check_streaming_sve_and_za_enabled()?;
        let mut env = self.fp_env().with_default_nan();
        self.za_array(regs, 2, svl, |acc, zn, zm, i| {
            let mut result = *acc;
            for e in 0..Esize::S.elements(svl) {
                let n = zn.get(2 * e + i, Esize::H) as u16;
                let n = if subtract { format.negate(n, &env) } else { n };
                let m = zm.get(2 * e + i, Esize::H) as u16;
                let sum = acc.get(e, Esize::S) as u32;
                result.set(e, Esize::S, u64::from(format.mul_add(sum, n, m, &mut env)));
            }
            result
        });
        Ok(Flow::Next)
    }

    /// SDOT, UDOT, USDOT and SUDOT of two or four vectors by one, and of two
    /// or four by as many (`sdot_za_zzv_2x1` to `udot_za32_zzw_4x4`:
    /// [`ArrayRegs`]): word `e` of the ZA vector gains the sum of the
    /// products of bytes `4 * e` to `4 * e + 3` of the register and of its
    /// second operand (`sz = 0`, bit 3 clear), mixed in sign (`sz = 0`, bit
    /// 3 set: [`Signs`]), or of halfwords `2 * e` and `2 * e + 1` (`sz = 1`,
    /// bit 3 set), modulo 2^32. `sz = 1` with bit 3 clear, FEAT_SME_I16I64's
    /// four-way form of halfwords into doublewords, is UNDEFINED.
    pub(crate) fn exec_sme_array_int_dot(&mut self, pc: u64, insn: u32) -> ExecResult {
        let (signs, es) = match (bit(insn, 22), bit(insn, 3)) {
            (false, false) => (Signs::of(insn, false), Esize::B),
            (false, true) => (Signs::of(insn, true), Esize::B),
            (true, true) => (Signs::of(insn, false), Esize::H),
            (true, false) => return self.undefined(pc, insn),
        };
        let parts = Esize::S.bytes() / es.bytes();
        let regs = ArrayRegs::of(insn, 1);
        let svl = self.check_streaming_sve_and_za_enabled()?;
        self.za_array(regs, 1, svl, |acc, zn, zm, _| {
            let mut result = *acc;
            for e in 0..Esize::S.elements(svl) {
                let dot: i128 = (0..parts).map(|k| signs.product(zn, zm, parts * e + k, es)).sum();
                result.set(e, Esize::S, acc.get(e, Esize::S).wrapping_add(dot as u64));
            }
            result
        });
        Ok(Flow::Next)
    }

    /// FDOT and BFDOT of two or four vectors by one, and of two or four by
    /// as many (`fdot_za_zzv_2x1` to `bfdot_za_zzw_4x4`: [`ArrayRegs`],
    /// BFloat16 by bit 4): single-precision element `e` of the ZA vector
    /// plus the products of 16-bit elements `2 * e` and `2 * e + 1` of the
    /// register and of its second operand (`FPDotAdd_ZA`, `BFDotAdd`).
    pub(crate) fn exec_sme_array_fp_dot(&mut self, insn: u32) -> ExecResult {
        let format = if bit(insn, 4) { PairFormat::BFloat16 } else { PairFormat::Half };
        let regs = ArrayRegs::of(insn, 1);
        let svl = self.check_streaming_sve_and_za_enabled()?;
        let mut env = self.fp_env().with_default_nan();
        self.za_array(regs, 1, svl, |acc, zn, zm, _| {
            let mut result = *acc;
            for e in 0..Esize::S.elements(svl) {
                let pair = |z: &Zv| [z.get(2 * e, Esize::H) as u16, z.get(2 * e + 1, Esize::H) as u16];
                let sum = acc.get(e, Esize::S) as u32;
                result.set(e, Esize::S, u64::from(format.dot(sum, pair(zn), pair(zm), &mut env)));
            }
            result
        });
        Ok(Flow::Next)
    }

    /// FMLA and FMLS of two or four vectors by one, and of two or four by as
    /// many (`fmla_za_zzv_2x1` to `fmls_za_zzw_4x4`: [`ArrayRegs`], `S` bit
    /// 3): single-precision element `e` of the ZA vector plus the product
    /// of the register's, negated by `S` (`FPNeg`), and its second
    /// operand's (`FPMulAdd_ZA`). `sz = 1`, FEAT_SME_F64F64's
    /// double-precision form, is UNDEFINED.
    pub(crate) fn exec_sme_array_fp_mul_add(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bit(insn, 22) {
            return self.undefined(pc, insn);
        }
        let form = if bit(insn, 3) { FusedMulAdd::Fmsub } else { FusedMulAdd::Fmadd };
        let regs = ArrayRegs::of(insn, 1);
        let svl = self.check_streaming_sve_and_za_enabled()?;
        let mut env = self.fp_env().with_default_nan();
        self.za_array(regs, 1, svl, |acc, zn, zm, _| {
            let mut result = *acc;
            for e in 0..Esize::S.elements(svl) {
                let (sum, n, m) = (acc.get(e, Esize::S), zn.get(e, Esize::S), zm.get(e, Esize::S));
                result.set(e, Esize::S, fp_mul_add(form, FpType::Single, sum, n, m, &mut env));
            }
            result
        });
        Ok(Flow::Next)
    }

    /// ADD and SUB of two or four vectors and one, and of two or four and as
    /// many, to ZA (`add_za_zzv_2x1` to `sub_za_zzw_4x4`: [`ArrayRegs`], `S`
    /// bit 3): each word of the ZA vector becomes the register's plus, or
    /// minus, its second operand's, modulo 2^32, whatever it held. `sz =
    /// 1`, FEAT_SME_I16I64's form of doublewords, is UNDEFINED.
    pub(crate) fn exec_sme_array_add_vectors(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bit(insn, 22) {
            return self.undefined(pc, insn);
        }
        let subtract = bit(insn, 3);
        let regs = ArrayRegs::of(insn, 1);
        let svl = self.check_streaming_sve_and_za_enabled()?;
        self.za_array(regs, 1, svl, |_, zn, zm, _| {
            let mut result = Zv::ZERO;
            for e in 0..Esize::S.elements(svl) {
                let (n, m) = (zn.get(e, Esize::S), zm.get(e, Esize::S));
                result.set(e, Esize::S, if subtract { n.wrapping_sub(m) } else { n.wrapping_add(m) });
            }
            result
        });
        Ok(Flow::Next)
    }

    /// ADD, SUB, FADD and FSUB of two or four vectors to ZA (`add_za_zw_2x2`
    /// to `fsub_za_zw_4x4`: [`ArrayVectors`], the group `Zm:'0'` or
    /// `Zm:'00'` from bits 9:6 or 9:7, an integer sum by bit 4, `S` bit 3):
    /// each word of the ZA vector plus, or minus, the register's, modulo
    /// 2^32 or as `FPAdd_ZA` and `FPSub_ZA` of single-precision values.
    /// `sz = 1`, the forms of doublewords and double-precision values of
    /// FEAT_SME_I16I64 and FEAT_SME_F64F64, is UNDEFINED.
    pub(crate) fn exec_sme_array_accumulate(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bit(insn, 22) {
            return self.undefined(pc, insn);
        }
        let at = ArrayVectors::of(insn, 1);
        let m = group(insn, 9, at.nreg);
        let subtract = bit(insn, 3);
        let svl = self.check_streaming_sve_and_za_enabled()?;
        match (bit(insn, 4), subtract) {
            (true, false) => self.za_accumulate(at, m, svl, u64::wrapping_add),
            (true, true) => self.za_accumulate(at, m, svl, u64::wrapping_sub),
            (false, _) => {
                let op = if subtract { FpBinary::Sub } else { FpBinary::Add };
                let mut env = self.fp_env().with_default_nan();
                self.za_accumulate(at, m, svl, |a, b| fp_binary(op, FpType::Single, a, b, &mut env));
            }
        }
        Ok(Flow::Next)
    }
}
