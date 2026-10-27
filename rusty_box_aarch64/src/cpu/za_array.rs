//! FEAT_SME2's multi-vector instructions on ZA array vectors
//! (`mortlach_multi_array_1a` to `_2b`, and by an indexed element
//! `mortlach_multi_indexed_1` to `_3`): the products, dot products and
//! sums of one, two or four vectors with one vector, with as many, or with
//! an indexed element, into groups of ZA's horizontal vectors; and MOVA
//! between such a group and two or four registers
//! (`mortlach_multi2_za_insert_ctg` to `mortlach_multi4_za_extract_ctg`).
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
//! By an indexed element, each result element takes the second operand's
//! element, or group of elements, `index` of the 128-bit segment holding
//! it (Arm's `s = segmentbase + index`). The vertical dot products FVDOT,
//! BFVDOT, SVDOT, UVDOT, USVDOT and SUVDOT sum, for register `r`'s result
//! element `e`, the elements `nreg * e + r` of every register of the group,
//! each with its own element of the indexed group.
//!
//! Each executes in Streaming SVE mode with ZA storage enabled
//! (`CheckStreamingSVEAndZAEnabled`). The floating-point ones compute as
//! `FPMulAdd_ZA`, `FPMulAddH_ZA`, `BFMulAddH_ZA`, `FPDotAdd_ZA`,
//! `FPAdd_ZA` and `FPSub_ZA` do, with FPCR.DN read as 1 and no exception,
//! so they never write FPSR; BFDOT and BFVDOT follow BFloat16's own
//! behaviours (`BFDotAdd`), as FEAT_SME's BFMOPA does. The 64-bit forms of
//! FEAT_SME_I16I64 and FEAT_SME_F64F64 are UNDEFINED.
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
use super::za::{PairFormat, Signedness, ZaMove};
use super::{Cpu, Levels};
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
    /// register (one, two or four). By an indexed element (bit 21 clear,
    /// `mortlach_multi_indexed_1` to `_3`): one register for bit 20 clear,
    /// else two or four by bit 15. Of vectors (bit 21 set): with bit 23 set
    /// (`mortlach_multi_array_2a`, `_2b`), two or four registers by bit 16;
    /// else (`_1a`, `_1b`), one for `op1 = 0x1` (bits 12:10), both groups'
    /// `mortlach_multi1_*` classes, and two or four by bit 20 for the rest.
    /// The offset is the low bits `off3`, `off2` or `o1`, as many as leave
    /// three bits with the scale, one more for one register.
    fn of(insn: u32, width: usize) -> Self {
        let nreg = if bit(insn, 21) {
            match (bit(insn, 23), bit(insn, 16), bits(insn, 12, 10), bit(insn, 20)) {
                (true, true, _, _) => 4,
                (true, false, _, _) => 2,
                (false, _, 0b001 | 0b011, _) => 1,
                (false, _, _, true) => 4,
                (false, _, _, false) => 2,
            }
        } else {
            match (bit(insn, 20), bit(insn, 15)) {
                (false, _) => 1,
                (true, false) => 2,
                (true, true) => 4,
            }
        };
        let k = width.trailing_zeros();
        let hi = 2 - k + u32::from(nreg == 1);
        Self {
            v: 8 + bits(insn, 14, 13),
            offset: bits(insn, hi, 0) << k,
            nreg,
        }
    }

    /// The vectors a MOVA between ZA array vectors and a group of registers
    /// selects (`mortlach_multi2_za_insert_ctg` to
    /// `mortlach_multi4_za_extract_ctg`): two registers for bits 12:10 =
    /// `010`, four for `011`, by bit 10; the offset `off3` the three bits
    /// from `lo` up, unscaled.
    fn of_move(insn: u32, lo: u32) -> Self {
        Self {
            v: 8 + bits(insn, 14, 13),
            offset: bits(insn, lo + 2, lo),
            nreg: if bit(insn, 10) { 4 } else { 2 },
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

/// Which element of the second operand goes with an element of the first.
/// The result elements are words, four to a 128-bit segment.
#[derive(Debug, Clone, Copy)]
enum Pairing {
    /// The element in the same place.
    Same,
    /// From the segment holding the result element: element `index` of a
    /// long operation's sources, or group `index` of the elements one
    /// result element sums (Arm's `s = segmentbase + index`).
    Indexed(usize),
}

impl Pairing {
    /// The second operand's element going with element `width * e + i` of
    /// the first in a long operation: `e` the result element, `i` the
    /// vector of the `width` each register's results span.
    fn long(self, e: usize, width: usize, i: usize) -> usize {
        match self {
            Self::Same => width * e + i,
            Self::Indexed(index) => width * (e - e % 4) + index,
        }
    }

    /// The second operand's element going with element `parts * e + k` of
    /// the first in a dot product of `parts` elements to a result element
    /// (one for FMLA).
    fn dot(self, e: usize, parts: usize, k: usize) -> usize {
        match self {
            Self::Same => parts * e + k,
            Self::Indexed(index) => parts * (e - e % 4 + index) + k,
        }
    }
}

/// The operands of a multi-vector instruction on ZA array vectors of two
/// sources: the vectors it selects, the group from `Zn` (register `r`
/// being `(n + r) MOD 32`), the second operand, and how their elements
/// pair.
#[derive(Debug, Clone, Copy)]
struct ArrayRegs {
    at: ArrayVectors,
    n: u32,
    m: Second,
    pairing: Pairing,
}

impl ArrayRegs {
    /// The operands of `insn` with results spanning `width` vectors per
    /// register. By an indexed element (bit 21 clear), the group from `Zn`
    /// (bits 9:5, `Zn:'0'` at 9:6 or `Zn:'00'` at 9:7) by `'0':Zm` (bits
    /// 19:16) and its index: `i4h:i4l` of the four-way products (bits 15
    /// and 12:10 for one register, 11:10 and 2:1 for a group), `i3h:i3l` of
    /// the two-way ones (bits 15 and 11:10, or 11:10 and 2) and `i2` (bits
    /// 11:10) of the rest. Of vectors with bit 23 set, a group by a group:
    /// `Zn:'0'` or `Zn:'00'` (bits 9:6 or 9:7) by the group from `Zm`. Else
    /// one register or a group from `Zn` (bits 9:5) by one vector.
    fn of(insn: u32, width: usize) -> Self {
        let at = ArrayVectors::of(insn, width);
        let (n, m, pairing) = match (bit(insn, 21), bit(insn, 23)) {
            (true, true) => (group(insn, 9, at.nreg), Second::Group(group(insn, 20, at.nreg)), Pairing::Same),
            (true, false) => (bits(insn, 9, 5), Second::Vector(bits(insn, 19, 16)), Pairing::Same),
            (false, _) => {
                let index = match (width, at.nreg == 1) {
                    (4, true) => (bits(insn, 15, 15) << 3) | bits(insn, 12, 10),
                    (4, false) => (bits(insn, 11, 10) << 2) | bits(insn, 2, 1),
                    (2, true) => (bits(insn, 15, 15) << 2) | bits(insn, 11, 10),
                    (2, false) => (bits(insn, 11, 10) << 1) | bits(insn, 2, 2),
                    // One vector per register: `i2`.
                    _ => bits(insn, 11, 10),
                };
                let pairing = Pairing::Indexed(index as usize);
                (group(insn, 9, at.nreg), Second::Vector(bits(insn, 19, 16)), pairing)
            }
        };
        Self { at, n, m, pairing }
    }

    /// The four-way products' mixed-sign bit `op`: bit 5 of a group by an
    /// indexed byte, bit 2 of the rest.
    fn mixed_sign(self, insn: u32) -> bool {
        match (self.pairing, self.at.nreg) {
            (Pairing::Indexed(_), 2 | 4) => bit(insn, 5),
            _ => bit(insn, 2),
        }
    }
}

/// The integer products of the long products and the dot products: each
/// source element `Int`'d by its own signedness.
#[derive(Debug, Clone, Copy)]
struct Signs {
    n: Signedness,
    m: Signedness,
}

impl Signs {
    /// SMLALL, SMLSLL, UMLALL, UMLSLL, USMLALL and SUMLALL, SMLAL to UMLSL,
    /// SDOT, UDOT, USDOT and SUDOT, and SVDOT, UVDOT, USVDOT and SUVDOT:
    /// both by `U` (bit 4), or with `mixed`, `Zn`'s unsigned and `Zm`'s
    /// signed for `U = 0`, the other way for `U = 1`.
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

    /// The product of `n`, an element of the first operand, and `m`, of the
    /// second, both of `es`.
    fn product(self, n: u64, m: u64, es: Esize) -> i128 {
        self.n.int(n, es) * self.m.int(m, es)
    }
}

impl<L: Levels> Cpu<L> {
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

    /// Registers `first` to `first + nreg - 1`, an aligned group; the
    /// places past it zero.
    fn z_group(&self, first: u32, nreg: usize, svl: VecLen) -> [Zv; 4] {
        core::array::from_fn(|k| if k < nreg { self.z(first + k as u32, svl) } else { Zv::ZERO })
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

    /// Each word `e` of the vector `at` selects in register `r`'s stride
    /// becomes `op(word, e, r)`.
    fn za_each_word<F: FnMut(u64, usize, usize) -> u64>(&mut self, at: ArrayVectors, svl: VecLen, mut op: F) {
        let span = self.array_span(at, 1, svl);
        for r in 0..at.nreg {
            let index = span.first + r * span.vstride;
            let mut vector = self.za_vector(index, svl);
            for e in 0..Esize::S.elements(svl) {
                vector.set(e, Esize::S, op(vector.get(e, Esize::S), e, r));
            }
            self.set_za_vector(index, svl, &vector);
        }
    }

    /// SMLALL, SMLSLL, UMLALL, UMLSLL, USMLALL and SUMLALL of one, two or
    /// four vectors by one, by as many or by an indexed byte
    /// (`smlall_za_zzv_1` to `usmlall_za_zzi_s4xi`: [`ArrayRegs`], `U` bit
    /// 4, `S` bit 3, the mixed signs by `op`, [`ArrayRegs::mixed_sign`]):
    /// word `e` of ZA vector `i` of four gains, or by `S` loses, the
    /// product of byte `4 * e + i` of the register and its pair in the
    /// second operand, modulo 2^32. `sz = 1` by vectors, FEAT_SME_I16I64's
    /// form of halfwords into doublewords, is UNDEFINED; by an indexed
    /// byte, bit 22 is `op0`'s low bit, clear in these classes.
    pub(crate) fn exec_sme_array_int_long_long(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bit(insn, 22) {
            return self.undefined(pc, insn);
        }
        let regs = ArrayRegs::of(insn, 4);
        self.int_mul_add_long(insn, regs, Signs::of(insn, regs.mixed_sign(insn)), Esize::B)
    }

    /// SMLAL, SMLSL, UMLAL, UMLSL of one, two or four vectors by one, by as
    /// many or by an indexed halfword (`smlal_za_zzv_1` to
    /// `umlsl_za_zzi_4xi`: [`ArrayRegs`], `U` bit 4, `S` bit 3): word `e` of
    /// ZA vector `i` of two gains, or loses, the product of halfword
    /// `2 * e + i` of the register and its pair in the second operand,
    /// modulo 2^32.
    pub(crate) fn exec_sme_array_int_long(&mut self, insn: u32) -> ExecResult {
        self.int_mul_add_long(insn, ArrayRegs::of(insn, 2), Signs::of(insn, false), Esize::H)
    }

    /// The long integer products of `es` into words, `S` bit 3: word `e`
    /// of ZA vector `i` of the `width = 4 / es` each register's results
    /// span gains, or loses, the product of element `width * e + i` and its
    /// pair ([`Pairing::long`]).
    fn int_mul_add_long(&mut self, insn: u32, regs: ArrayRegs, signs: Signs, es: Esize) -> ExecResult {
        let width = Esize::S.bytes() / es.bytes();
        let subtract = bit(insn, 3);
        let pairing = regs.pairing;
        let svl = self.check_streaming_sve_and_za_enabled()?;
        self.za_array(regs, width, svl, |acc, zn, zm, i| {
            let mut result = *acc;
            for e in 0..Esize::S.elements(svl) {
                let (n, m) = (zn.get(width * e + i, es), zm.get(pairing.long(e, width, i), es));
                let product = signs.product(n, m, es) as u64;
                let sum = acc.get(e, Esize::S);
                result.set(e, Esize::S, if subtract { sum.wrapping_sub(product) } else { sum.wrapping_add(product) });
            }
            result
        });
        Ok(Flow::Next)
    }

    /// FMLAL, FMLSL, BFMLAL, BFMLSL of one, two or four vectors by one, by
    /// as many or by an indexed element (`fmlal_za_zzv_1` to
    /// `bfmlsl_za_zzi_4xi`: [`ArrayRegs`], BFloat16 by `op`, bit 4, `S` bit
    /// 3): single-precision element `e` of ZA vector `i` of two plus the
    /// product of 16-bit element `2 * e + i` of the register, negated by
    /// `S`, and its pair in the second operand, in one rounding:
    /// `FPMulAddH_ZA` and `FPNeg`, or `BFMulAddH_ZA` and `BFNeg`
    /// ([`PairFormat`]).
    pub(crate) fn exec_sme_array_fp_long(&mut self, insn: u32) -> ExecResult {
        let format = if bit(insn, 4) { PairFormat::BFloat16 } else { PairFormat::Half };
        let subtract = bit(insn, 3);
        let regs = ArrayRegs::of(insn, 2);
        let pairing = regs.pairing;
        let svl = self.check_streaming_sve_and_za_enabled()?;
        let mut env = self.fp_env().with_default_nan();
        self.za_array(regs, 2, svl, |acc, zn, zm, i| {
            let mut result = *acc;
            for e in 0..Esize::S.elements(svl) {
                let n = zn.get(2 * e + i, Esize::H) as u16;
                let n = if subtract { format.negate(n, &env) } else { n };
                let m = zm.get(pairing.long(e, 2, i), Esize::H) as u16;
                let sum = acc.get(e, Esize::S) as u32;
                result.set(e, Esize::S, u64::from(format.mul_add(sum, n, m, &mut env)));
            }
            result
        });
        Ok(Flow::Next)
    }

    /// SDOT, UDOT, USDOT and SUDOT of two or four vectors by one, and of two
    /// or four by as many (`sdot_za_zzv_2x1` to `udot_za32_zzw_4x4`:
    /// [`ArrayRegs`]): bytes (bit 3 clear), mixed in sign (`sz = 0`, bit 3
    /// set: [`Signs`]), or halfwords (`sz = 1`, bit 3 set), as
    /// [`Cpu::int_dot`]. `sz = 1` with bit 3 clear, FEAT_SME_I16I64's
    /// four-way form of halfwords into doublewords, is UNDEFINED.
    pub(crate) fn exec_sme_array_int_dot(&mut self, pc: u64, insn: u32) -> ExecResult {
        match (bit(insn, 22), bit(insn, 3)) {
            (false, false) => self.int_dot(insn, Signs::of(insn, false), Esize::B),
            (false, true) => self.int_dot(insn, Signs::of(insn, true), Esize::B),
            (true, true) => self.int_dot(insn, Signs::of(insn, false), Esize::H),
            (true, false) => self.undefined(pc, insn),
        }
    }

    /// SDOT, UDOT, USDOT and SUDOT of two or four vectors of bytes, and SDOT
    /// and UDOT of halfwords, by an indexed group (`sdot_za32_zzi_2xi` to
    /// `sudot_za_zzi_s4xi`: [`ArrayRegs`], bytes by bit 5, mixed signs by
    /// bit 3), as [`Cpu::int_dot`].
    pub(crate) fn exec_sme_indexed_int_dot(&mut self, insn: u32) -> ExecResult {
        let es = if bit(insn, 5) { Esize::B } else { Esize::H };
        self.int_dot(insn, Signs::of(insn, bit(insn, 3)), es)
    }

    /// The integer dot products of `es` into words: word `e` of the ZA
    /// vector gains the sum of the products of elements `parts * e` to
    /// `parts * e + parts - 1` of the register and their pairs
    /// ([`Pairing::dot`]), `parts = 4 / es`, modulo 2^32.
    fn int_dot(&mut self, insn: u32, signs: Signs, es: Esize) -> ExecResult {
        let parts = Esize::S.bytes() / es.bytes();
        let regs = ArrayRegs::of(insn, 1);
        let pairing = regs.pairing;
        let svl = self.check_streaming_sve_and_za_enabled()?;
        self.za_array(regs, 1, svl, |acc, zn, zm, _| {
            let mut result = *acc;
            for e in 0..Esize::S.elements(svl) {
                let dot: i128 = (0..parts)
                    .map(|k| signs.product(zn.get(parts * e + k, es), zm.get(pairing.dot(e, parts, k), es), es))
                    .sum();
                result.set(e, Esize::S, acc.get(e, Esize::S).wrapping_add(dot as u64));
            }
            result
        });
        Ok(Flow::Next)
    }

    /// FDOT and BFDOT of two or four vectors by one, by as many or by an
    /// indexed pair (`fdot_za_zzv_2x1` to `bfdot_za_zzi_4xi`:
    /// [`ArrayRegs`], BFloat16 by bit 4): single-precision element `e` of
    /// the ZA vector plus the products of 16-bit elements `2 * e` and `2 *
    /// e + 1` of the register and their pairs (`FPDotAdd_ZA`, `BFDotAdd`).
    pub(crate) fn exec_sme_array_fp_dot(&mut self, insn: u32) -> ExecResult {
        let format = if bit(insn, 4) { PairFormat::BFloat16 } else { PairFormat::Half };
        let regs = ArrayRegs::of(insn, 1);
        let pairing = regs.pairing;
        let svl = self.check_streaming_sve_and_za_enabled()?;
        let mut env = self.fp_env().with_default_nan();
        self.za_array(regs, 1, svl, |acc, zn, zm, _| {
            let mut result = *acc;
            for e in 0..Esize::S.elements(svl) {
                let row = [zn.get(2 * e, Esize::H) as u16, zn.get(2 * e + 1, Esize::H) as u16];
                let col = [0, 1].map(|k| zm.get(pairing.dot(e, 2, k), Esize::H) as u16);
                let sum = acc.get(e, Esize::S) as u32;
                result.set(e, Esize::S, u64::from(format.dot(sum, row, col, &mut env)));
            }
            result
        });
        Ok(Flow::Next)
    }

    /// FMLA and FMLS of two or four vectors by one, and of two or four by as
    /// many (`fmla_za_zzv_2x1` to `fmls_za_zzw_4x4`: [`ArrayRegs`], `S` bit
    /// 3), as [`Cpu::fp_mul_add_za`]. `sz = 1`, FEAT_SME_F64F64's
    /// double-precision form, is UNDEFINED.
    pub(crate) fn exec_sme_array_fp_mul_add(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bit(insn, 22) {
            return self.undefined(pc, insn);
        }
        let form = if bit(insn, 3) { FusedMulAdd::Fmsub } else { FusedMulAdd::Fmadd };
        self.fp_mul_add_za(insn, form)
    }

    /// FMLA and FMLS of two or four vectors by an indexed element
    /// (`fmla_za_zzi_s2xi` to `fmls_za_zzi_s4xi`: [`ArrayRegs`], FMLS by
    /// bit 4), as [`Cpu::fp_mul_add_za`].
    pub(crate) fn exec_sme_indexed_fp_mul_add(&mut self, insn: u32) -> ExecResult {
        let form = if bit(insn, 4) { FusedMulAdd::Fmsub } else { FusedMulAdd::Fmadd };
        self.fp_mul_add_za(insn, form)
    }

    /// Single-precision element `e` of the ZA vector plus the product of
    /// the register's, negated by `form`'s FMLS (`FPNeg`), and its pair
    /// (`FPMulAdd_ZA`).
    fn fp_mul_add_za(&mut self, insn: u32, form: FusedMulAdd) -> ExecResult {
        let regs = ArrayRegs::of(insn, 1);
        let pairing = regs.pairing;
        let svl = self.check_streaming_sve_and_za_enabled()?;
        let mut env = self.fp_env().with_default_nan();
        self.za_array(regs, 1, svl, |acc, zn, zm, _| {
            let mut result = *acc;
            for e in 0..Esize::S.elements(svl) {
                let (sum, n, m) = (acc.get(e, Esize::S), zn.get(e, Esize::S), zm.get(pairing.dot(e, 1, 0), Esize::S));
                result.set(e, Esize::S, fp_mul_add(form, FpType::Single, sum, n, m, &mut env));
            }
            result
        });
        Ok(Flow::Next)
    }

    /// FVDOT, BFVDOT, SVDOT, UVDOT, USVDOT and SUVDOT of two or four
    /// vectors by an indexed group (`fvdot_za_zzi_2xi` to
    /// `suvdot_za_zzi_s4xi`: [`ArrayRegs`]): word `e` of register `r`'s ZA
    /// vector gains the products of element `nreg * e + r` of each register
    /// `k` of the group and element `k` of the indexed group. With bit 5
    /// set, integers, halfwords from two registers or bytes from four
    /// ([`Signs`], mixed signs by bit 3), modulo 2^32; else two registers
    /// of half-precision or BFloat16 (bit 4) values (`FPDotAdd_ZA`,
    /// `BFDotAdd`).
    pub(crate) fn exec_sme_indexed_vertical_dot(&mut self, insn: u32) -> ExecResult {
        let regs = ArrayRegs::of(insn, 1);
        let (nreg, pairing) = (regs.at.nreg, regs.pairing);
        let svl = self.check_streaming_sve_and_za_enabled()?;
        let zn = self.z_group(regs.n, nreg, svl);
        let zm = self.z(regs.m.register(0), svl);
        if bit(insn, 5) {
            let es = if nreg == 2 { Esize::H } else { Esize::B };
            let signs = Signs::of(insn, bit(insn, 3));
            self.za_each_word(regs.at, svl, |acc, e, r| {
                let dot: i128 = (0..nreg)
                    .map(|k| signs.product(zn[k].get(nreg * e + r, es), zm.get(pairing.dot(e, nreg, k), es), es))
                    .sum();
                acc.wrapping_add(dot as u64)
            });
        } else {
            let format = if bit(insn, 4) { PairFormat::BFloat16 } else { PairFormat::Half };
            let mut env = self.fp_env().with_default_nan();
            self.za_each_word(regs.at, svl, |acc, e, r| {
                let row = [0, 1].map(|k| zn[k].get(2 * e + r, Esize::H) as u16);
                let col = [0, 1].map(|k| zm.get(pairing.dot(e, 2, k), Esize::H) as u16);
                u64::from(format.dot(acc as u32, row, col, &mut env))
            });
        }
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
        let subtract = bit(insn, 3);
        let svl = self.check_streaming_sve_and_za_enabled()?;
        let zm = self.z_group(group(insn, 9, at.nreg), at.nreg, svl);
        match (bit(insn, 4), subtract) {
            (true, false) => self.za_each_word(at, svl, |a, e, r| a.wrapping_add(zm[r].get(e, Esize::S))),
            (true, true) => self.za_each_word(at, svl, |a, e, r| a.wrapping_sub(zm[r].get(e, Esize::S))),
            (false, _) => {
                let op = if subtract { FpBinary::Sub } else { FpBinary::Add };
                let mut env = self.fp_env().with_default_nan();
                self.za_each_word(at, svl, |a, e, r| fp_binary(op, FpType::Single, a, zm[r].get(e, Esize::S), &mut env));
            }
        }
        Ok(Flow::Next)
    }

    /// MOVA between two or four registers and ZA array vectors
    /// (`mova_za_mz2_1`, `mova_za_mz4_1`: `11000000 00000 1 00 0 Rv 01 nreg
    /// Zn 000 off3`; `mova_mz_za2_1`, `mova_mz_za4_1`: `11000000 00000 1 10
    /// 0 Rv 01 nreg 00 off3 Zd`): register `r` of the group [`ZaMove`] names
    /// to or from the vector [`ArrayVectors::of_move`] selects in stride
    /// `r`.
    pub(crate) fn exec_sme_move_group_array(&mut self, insn: u32, way: ZaMove) -> ExecResult {
        let at = ArrayVectors::of_move(insn, way.za_field());
        let svl = self.check_streaming_sve_and_za_enabled()?;
        let span = self.array_span(at, 1, svl);
        let z = group(insn, way.registers(), at.nreg);
        for r in 0..at.nreg {
            let index = span.first + r * span.vstride;
            match way {
                ZaMove::Insert => {
                    let value = self.z(z + r as u32, svl);
                    self.set_za_vector(index, svl, &value);
                }
                ZaMove::Extract => {
                    let value = self.za_vector(index, svl);
                    self.set_z(z + r as u32, svl, &value);
                }
            }
        }
        Ok(Flow::Next)
    }
}
