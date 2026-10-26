//! SVE2's integer instructions FEAT_SME makes legal in Streaming SVE mode
//! with `op1<7> = 0` (Arm's `sve_intx_muladd_unpred`,
//! `sve_intx_predicated`, `sve_intx_clamp` and `sve_intx_by_indexed_elem`
//! groups), with SVE's SDOT and UDOT and FEAT_I8MM's USDOT and SUDOT
//! among them: the dot products, the complex integer multiply-adds and
//! CDOT, the long multiply-adds and multiplies, the rounding doubling
//! multiplies, the multiplies by element, the predicated saturating,
//! rounding, halving and pairwise operations, URECPE, URSQRTE, SQABS,
//! SQNEG, SADALP, UADALP, SCLAMP and UCLAMP.
//!
//! An SVE saturation never sets FPSR.QC ([`sat_q`]). Every product and sum
//! is exact in `i128` before it is truncated or saturated, as the
//! pseudocode's integers are unbounded; the doubling multiplies of 64-bit
//! elements stay exact through [`doubling_mul_high`] and
//! [`rounding_doubling_mul_add`]. Each handler decodes its word again into
//! a closed set of operations; a word its classes do not hold, which the
//! decoder never passes, is UNDEFINED.
//!
//! Provenance (R7): the Arm A64 ISA XML, release 2026-09, the instruction
//! pages each handler names and the shared pseudocode they call
//! (`SignedSat`, `UnsignedSat`, `ShiftSat`, `UnsignedRecipEstimate`,
//! `UnsignedRSqrtEstimate`).

use super::int::BinOp;
use super::{int, sat_q, segment_base, Esize, Zv};
use crate::cpu::exec::{bit, bits, ExecResult, Flow};
use crate::cpu::fp_estimate::{urecpe, ursqrte};
use crate::cpu::simd_arith::{doubling_mul_high, rounding_doubling_mul_add, shift_signed};
use crate::cpu::Cpu;

/// Arm's `rot` of CMLA, SQRDCMLAH and CDOT (bits 11:10).
#[derive(Debug, Clone, Copy)]
struct Rotation(u32);

impl Rotation {
    /// `UInt(rot<0>)`: the element of each pair of Zn CMLA's two products
    /// take, and the one of Zm the real product takes.
    const fn sel_a(self) -> usize {
        (self.0 & 1) as usize
    }

    /// `UInt(NOT(rot<0>))`: the element of Zm the imaginary product takes.
    const fn sel_b(self) -> usize {
        1 - self.sel_a()
    }

    /// CMLA's `sub_r = rot<0> != rot<1>`.
    const fn sub_r(self) -> bool {
        self.0 & 1 != self.0 >> 1
    }

    /// CMLA's `sub_i = rot<1> == '1'`.
    const fn sub_i(self) -> bool {
        self.0 >> 1 == 1
    }

    /// CDOT's `sub_i = rot<0> == rot<1>`.
    const fn dot_sub_i(self) -> bool {
        !self.sub_r()
    }
}

/// The element of Zm an indexed instruction takes: `index` within each
/// 128-bit segment of `Zm`.
#[derive(Debug, Clone, Copy)]
struct Indexed {
    index: usize,
    m: u32,
}

impl Indexed {
    /// The fields of MLA, MLS, SQRDMLAH, SQRDMLSH, SQDMULH, SQRDMULH and
    /// MUL by element: halfwords (`size<1> = 0`) take `i3h:i3l` (bits 22,
    /// 20:19) and Z0-Z7, words (`size = 10`) `i2` (20:19) and Z0-Z7,
    /// doublewords `i1` (20) and Z0-Z15.
    const fn by_size(insn: u32) -> Self {
        match bits(insn, 23, 22) {
            0b10 => Self { index: bits(insn, 20, 19) as usize, m: bits(insn, 18, 16) },
            0b11 => Self { index: bits(insn, 20, 20) as usize, m: bits(insn, 19, 16) },
            _ => Self { index: ((bits(insn, 22, 22) << 2) | bits(insn, 20, 19)) as usize, m: bits(insn, 18, 16) },
        }
    }

    /// The fields of the indexed dot products, CDOT, CMLA and SQRDCMLAH:
    /// `size = 10` takes `i2` (bits 20:19) and Z0-Z7, `size = 11` `i1` (20)
    /// and Z0-Z15.
    const fn pair(insn: u32) -> Self {
        if bit(insn, 22) {
            Self { index: bits(insn, 20, 20) as usize, m: bits(insn, 19, 16) }
        } else {
            Self { index: bits(insn, 20, 19) as usize, m: bits(insn, 18, 16) }
        }
    }

    /// The fields of the long instructions by element: words from
    /// halfwords (`size = 10`) take `i3h:i3l` (bits 20:19, 11) and Z0-Z7,
    /// doublewords from words (`size = 11`) `i2h:i2l` (bits 20, 11) and
    /// Z0-Z15.
    const fn long(insn: u32) -> Self {
        if bit(insn, 22) {
            Self { index: ((bits(insn, 20, 20) << 1) | bits(insn, 11, 11)) as usize, m: bits(insn, 19, 16) }
        } else {
            Self { index: ((bits(insn, 20, 19) << 1) | bits(insn, 11, 11)) as usize, m: bits(insn, 18, 16) }
        }
    }
}

/// What a long instruction makes of the product of two half-size
/// elements.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy)]
enum LongOp {
    /// SMLALB to UMLSLT: the accumulator plus or minus the product,
    /// wrapping.
    MulAdd { subtract: bool, unsigned: bool },
    /// SQDMLALB to SQDMLSLT, SQDMLALBT, SQDMLSLBT: `SignedSat(acc +/-
    /// SInt(SignedSat(2 * product)))`.
    DoublingMulAdd { subtract: bool },
    /// SMULLB to UMULLT (indexed): the product.
    Mul { unsigned: bool },
    /// SQDMULLB, SQDMULLT (indexed): `SignedSat(2 * product)`.
    DoublingMul,
}

impl LongOp {
    const fn unsigned(self) -> bool {
        match self {
            Self::MulAdd { unsigned, .. } | Self::Mul { unsigned } => unsigned,
            Self::DoublingMulAdd { .. } | Self::DoublingMul => false,
        }
    }

    /// The element of `es` it writes, from the accumulator's element `acc`
    /// and the sources `a` and `b`.
    fn apply(self, acc: u64, a: i128, b: i128, es: Esize) -> u64 {
        let product = a * b;
        match self {
            Self::MulAdd { subtract, .. } => {
                let acc = i128::from(acc);
                (if subtract { acc - product } else { acc + product }) as u64
            }
            Self::DoublingMulAdd { subtract } => {
                let doubled = int(sat_q(2 * product, es, false), es, false);
                let acc = int(acc, es, false);
                sat_q(if subtract { acc - doubled } else { acc + doubled }, es, false)
            }
            Self::Mul { .. } => product as u64,
            Self::DoublingMul => sat_q(2 * product, es, false),
        }
    }
}

/// A multiply of same-size elements, of Zn's by Zm's or by Zm's element.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy)]
enum MulOp {
    /// MLA, MLS (indexed): the accumulator plus or minus the product,
    /// wrapping.
    MulAdd { subtract: bool },
    /// MUL (indexed).
    Mul,
    /// SQRDMLAH, SQRDMLSH.
    RoundingDoublingMulAdd { subtract: bool },
    /// SQDMULH, SQRDMULH (indexed).
    DoublingHigh { rounding: bool },
}

impl MulOp {
    /// Whether it reads the accumulator, Zda.
    const fn accumulates(self) -> bool {
        matches!(self, Self::MulAdd { .. } | Self::RoundingDoublingMulAdd { .. })
    }
}

/// A predicated binary operation of `sve_intx_predicated` on `(a, b)`:
/// Zdn's element and Zm's, swapped by the reversed forms.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy)]
enum BinaryOp {
    /// SRSHL, URSHL, SQSHL, UQSHL, SQRSHL, UQRSHL: `a` shifted by
    /// `ShiftSat(SInt(b), esize)`, left or (negative) right, a right shift
    /// rounding with `rounding`, the result saturated with `saturating`
    /// and else truncated.
    Shift { saturating: bool, rounding: bool, unsigned: bool },
    /// SHADD, UHADD, SRHADD, URHADD: `(a + b (+ 1)) >> 1`.
    HalvingAdd { rounding: bool, unsigned: bool },
    /// SHSUB, UHSUB: `(a - b) >> 1`.
    HalvingSub { unsigned: bool },
    /// SQADD, UQADD.
    SaturatingAdd { unsigned: bool },
    /// SQSUB, UQSUB.
    SaturatingSub { unsigned: bool },
    /// SUQADD (`SignedSat(SInt(a) + UInt(b))`) and USQADD
    /// (`UnsignedSat(UInt(a) + SInt(b))`, `unsigned`).
    SaturatingAddMixed { unsigned: bool },
    /// ADDP, SMAXP, UMAXP, SMINP, UMINP: the operation on a pair of
    /// elements.
    Pairwise(BinOp),
}

impl BinaryOp {
    fn apply(self, a: u64, b: u64, es: Esize) -> u64 {
        match self {
            Self::Shift { saturating, rounding, unsigned } => {
                let n = i64::from(es.bits());
                let shift = es.signed(b).clamp(-(n + 1), n + 1) as i32;
                let res = shift_signed(int(a, es, unsigned), shift, es.bits(), rounding);
                if saturating {
                    sat_q(res, es, unsigned)
                } else {
                    res as u64
                }
            }
            Self::HalvingAdd { rounding, unsigned } => {
                ((int(a, es, unsigned) + int(b, es, unsigned) + i128::from(rounding)) >> 1) as u64
            }
            Self::HalvingSub { unsigned } => ((int(a, es, unsigned) - int(b, es, unsigned)) >> 1) as u64,
            Self::SaturatingAdd { unsigned } => sat_q(int(a, es, unsigned) + int(b, es, unsigned), es, unsigned),
            Self::SaturatingSub { unsigned } => sat_q(int(a, es, unsigned) - int(b, es, unsigned), es, unsigned),
            Self::SaturatingAddMixed { unsigned } => {
                sat_q(int(a, es, unsigned) + int(b, es, !unsigned), es, unsigned)
            }
            Self::Pairwise(op) => op.apply(a, b, es),
        }
    }
}

/// An operation of URECPE, URSQRTE, SQABS, SQNEG, SADALP and UADALP.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy)]
enum UnaryOp {
    /// URECPE, URSQRTE (`square_root`), of words.
    Estimate { square_root: bool },
    /// SQABS, SQNEG (`negate`).
    Saturating { negate: bool },
    /// SADALP, UADALP: the pair of half-size elements of Zn, `Int`'d by
    /// `unsigned`, added to Zda's element.
    AccumulatePairs { unsigned: bool },
}

/// The signedness of a dot product's operands: SDOT's, UDOT's, USDOT's
/// (Zn unsigned, Zm signed) or SUDOT's.
#[derive(Debug, Clone, Copy)]
struct DotSigns {
    n_unsigned: bool,
    m_unsigned: bool,
}

impl Cpu {
    /// SDOT, UDOT (vectors: `01000100 size 0 Zm 00000 U Zn Zda`; indexed:
    /// `01000100 size 1 opc 00000 U Zn Zda`), and FEAT_I8MM's USDOT
    /// (vectors: `01000100 10 0 Zm 011110 Zn Zda`), USDOT and SUDOT
    /// (indexed: `01000100 10 1 i2 Zm 00011 U Zn Zda`). Each element of Zda
    /// accumulates, wrapping, the four products of its quarter-size
    /// elements of Zn with the same of Zm, or by index with those of the
    /// element `index` of its 128-bit segment of Zm. The decode takes
    /// `size = 1x` alone: words of bytes, or (`size<0>`) doublewords of
    /// halfwords.
    pub(crate) fn exec_sve_intx_dot(&mut self, pc: u64, insn: u32) -> ExecResult {
        let indexed = bit(insn, 21);
        let u = bit(insn, 10);
        let signs = match (indexed, bits(insn, 15, 10)) {
            // SDOT, UDOT.
            (_, 0b000000 | 0b000001) => DotSigns { n_unsigned: u, m_unsigned: u },
            // USDOT.
            (false, 0b011110) | (true, 0b000110) => DotSigns { n_unsigned: true, m_unsigned: false },
            // SUDOT (by element).
            (true, 0b000111) => DotSigns { n_unsigned: false, m_unsigned: true },
            _ => return self.undefined(pc, insn),
        };
        let (es, q) = if bit(insn, 22) { (Esize::D, Esize::H) } else { (Esize::S, Esize::B) };
        let form = if indexed { Some(Indexed::pair(insn)) } else { None };
        let m = form.map_or(bits(insn, 20, 16), |f| f.m);
        let (n, da) = (bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let op1 = self.z(n, vl);
        let op2 = self.z(m, vl);
        let op3 = self.z(da, vl);
        let mut result = Zv::ZERO;
        for e in 0..es.elements(vl) {
            let s = form.map_or(e, |f| segment_base(e, 16 / es.bytes()) + f.index);
            let mut sum = i128::from(op3.get(e, es));
            for i in 0..4 {
                let a = int(op1.get(4 * e + i, q), q, signs.n_unsigned);
                let b = int(op2.get(4 * s + i, q), q, signs.m_unsigned);
                sum += a * b;
            }
            result.set(e, es, sum as u64);
        }
        self.set_z(da, vl, &result);
        Ok(Flow::Next)
    }

    /// The long multiplies, each element of Zd from the product of two
    /// half-size elements: SMLALB, SMLALT, SMLSLB, SMLSLT, UMLALB, UMLALT,
    /// UMLSLB, UMLSLT (vectors: `01000100 size 0 Zm 010 S U T Zn Zda`;
    /// indexed: `01000100 size 1 opc 10 S U il T Zn Zda`), SQDMLALB,
    /// SQDMLALT, SQDMLSLB, SQDMLSLT (vectors: `... 0110 S T`; indexed:
    /// `... 001 S il T`), SQDMLALBT and SQDMLSLBT (`... 00010 S`), and
    /// indexed SMULLB, SMULLT, UMULLB, UMULLT (`... 110 U il T`), SQDMULLB
    /// and SQDMULLT (`... 1110 il T`). The even (`B`) or odd (`T`) source
    /// element of each pair of Zn is multiplied by the same of Zm
    /// (SQDMLALBT: Zn's even by Zm's odd), or by index by the source element
    /// `index` of its 128-bit segment of Zm. `size = 00` is UNDEFINED; the
    /// indexed forms are of `size = 1x` alone.
    pub(crate) fn exec_sve_intx_mul_long(&mut self, pc: u64, insn: u32) -> ExecResult {
        let indexed = bit(insn, 21);
        let t = usize::from(bit(insn, 10));
        let (op, sel1, sel2) = if indexed {
            let op = match bits(insn, 15, 13) {
                0b100 | 0b101 => LongOp::MulAdd { subtract: bit(insn, 13), unsigned: bit(insn, 12) },
                0b001 => LongOp::DoublingMulAdd { subtract: bit(insn, 12) },
                0b110 => LongOp::Mul { unsigned: bit(insn, 12) },
                0b111 if !bit(insn, 12) => LongOp::DoublingMul,
                _ => return self.undefined(pc, insn),
            };
            (op, t, t)
        } else {
            match bits(insn, 15, 11) {
                // SQDMLALBT, SQDMLSLBT: Zn's even element by Zm's odd.
                0b00001 => (LongOp::DoublingMulAdd { subtract: bit(insn, 10) }, 0, 1),
                0b01000..=0b01011 => (LongOp::MulAdd { subtract: bit(insn, 12), unsigned: bit(insn, 11) }, t, t),
                0b01100 | 0b01101 => (LongOp::DoublingMulAdd { subtract: bit(insn, 11) }, t, t),
                _ => return self.undefined(pc, insn),
            }
        };
        let (es, src) = match bits(insn, 23, 22) {
            0b00 => return self.undefined(pc, insn),
            0b01 => (Esize::H, Esize::B),
            0b10 => (Esize::S, Esize::H),
            _ => (Esize::D, Esize::S),
        };
        let form = if indexed { Some(Indexed::long(insn)) } else { None };
        let m = form.map_or(bits(insn, 20, 16), |f| f.m);
        let (n, d) = (bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let op1 = self.z(n, vl);
        let op2 = self.z(m, vl);
        let acc = self.z(d, vl);
        let mut result = Zv::ZERO;
        for e in 0..es.elements(vl) {
            let s2 = form.map_or(2 * e + sel2, |f| 2 * segment_base(e, 16 / es.bytes()) + f.index);
            let a = int(op1.get(2 * e + sel1, src), src, op.unsigned());
            let b = int(op2.get(s2, src), src, op.unsigned());
            result.set(e, es, op.apply(acc.get(e, es), a, b, es));
        }
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// The same-size multiplies: MLA, MLS (indexed: `01000100 size 1 opc
    /// 00001 S Zn Zda`), MUL (indexed: `... 111110 Zn Zd`), SQRDMLAH,
    /// SQRDMLSH (vectors: `01000100 size 0 Zm 01110 S Zn Zda`; indexed:
    /// `... 00010 S Zn Zda`), and SQDMULH, SQRDMULH (indexed: `... 11110 R
    /// Zn Zd`). By index each element of Zn is multiplied by the element
    /// `index` of its 128-bit segment of Zm.
    pub(crate) fn exec_sve_intx_mul(&mut self, pc: u64, insn: u32) -> ExecResult {
        let indexed = bit(insn, 21);
        let op = match (indexed, bits(insn, 15, 11)) {
            (false, 0b01110) | (true, 0b00010) => MulOp::RoundingDoublingMulAdd { subtract: bit(insn, 10) },
            (true, 0b00001) => MulOp::MulAdd { subtract: bit(insn, 10) },
            (true, 0b11110) => MulOp::DoublingHigh { rounding: bit(insn, 10) },
            (true, 0b11111) if !bit(insn, 10) => MulOp::Mul,
            _ => return self.undefined(pc, insn),
        };
        let size = bits(insn, 23, 22);
        let (es, form) = if indexed {
            let es = match size {
                0b10 => Esize::S,
                0b11 => Esize::D,
                _ => Esize::H,
            };
            (es, Some(Indexed::by_size(insn)))
        } else {
            (Esize::from_size(size), None)
        };
        let m = form.map_or(bits(insn, 20, 16), |f| f.m);
        let (n, d) = (bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let op1 = self.z(n, vl);
        let op2 = self.z(m, vl);
        let acc = if op.accumulates() { self.z(d, vl) } else { Zv::ZERO };
        let mut result = Zv::ZERO;
        for e in 0..es.elements(vl) {
            let s = form.map_or(e, |f| segment_base(e, 16 / es.bytes()) + f.index);
            let (a, b) = (op1.get(e, es), op2.get(s, es));
            let r = match op {
                MulOp::MulAdd { subtract } => {
                    let product = a.wrapping_mul(b);
                    let d = acc.get(e, es);
                    if subtract {
                        d.wrapping_sub(product)
                    } else {
                        d.wrapping_add(product)
                    }
                }
                MulOp::Mul => a.wrapping_mul(b),
                MulOp::RoundingDoublingMulAdd { subtract } => {
                    let value = rounding_doubling_mul_add(
                        int(acc.get(e, es), es, false),
                        int(a, es, false),
                        int(b, es, false),
                        es.bits(),
                        subtract,
                    );
                    sat_q(value, es, false)
                }
                MulOp::DoublingHigh { rounding } => {
                    sat_q(doubling_mul_high(int(a, es, false), int(b, es, false), es.bits(), rounding), es, false)
                }
            };
            result.set(e, es, r);
        }
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// The binary operations of `sve_intx_predicated` (`01000100 size 0
    /// op0 U 10 op1 Pg Zm Zdn`, `op0` bits 20:17), merging. `op0 = 0 Q R N`
    /// with `op1 = 0`: SRSHL, SQSHL, SQRSHL, URSHL, UQSHL, UQRSHL and their
    /// reversed forms (`R`). `op0 = 10 R S` with `op1 = 0`: SHADD, SHSUB,
    /// SRHADD, SHSUBR and the unsigned forms; `op0 = 10 opc` with `op1 = 1`:
    /// ADDP (`opc = 00`, `U = 1`), SMAXP, UMAXP (`10`), SMINP, UMINP (`11`).
    /// `op0 = 11 op S` with `op1 = 0`: SQADD, SQSUB, SUQADD, USQADD (`op`
    /// set, `S` clear), SQSUBR (both set), and the unsigned forms. A
    /// pairwise operation's even element is of the pair of Zdn it starts,
    /// its odd one of the pair of Zm it ends.
    pub(crate) fn exec_sve_intx_binary_pred(&mut self, pc: u64, insn: u32) -> ExecResult {
        let (r, s, u) = (bit(insn, 18), bit(insn, 17), bit(insn, 16));
        let (op, reversed) = match (bits(insn, 20, 19), bit(insn, 13)) {
            // `Q = 0, N = 0` is no shift of this class.
            (0b00, false) if !s => return self.undefined(pc, insn),
            (0b00 | 0b01, false) => (BinaryOp::Shift { saturating: bit(insn, 19), rounding: s, unsigned: u }, r),
            (0b10, false) => match (r, s) {
                (false, false) => (BinaryOp::HalvingAdd { rounding: false, unsigned: u }, false),
                (false, true) => (BinaryOp::HalvingSub { unsigned: u }, false),
                (true, false) => (BinaryOp::HalvingAdd { rounding: true, unsigned: u }, false),
                (true, true) => (BinaryOp::HalvingSub { unsigned: u }, true),
            },
            (0b10, true) => match bits(insn, 18, 17) {
                0b00 if u => (BinaryOp::Pairwise(BinOp::Add), false),
                0b10 => (BinaryOp::Pairwise(BinOp::Max { unsigned: u }), false),
                0b11 => (BinaryOp::Pairwise(BinOp::Min { unsigned: u }), false),
                _ => return self.undefined(pc, insn),
            },
            (0b11, false) => match (r, s) {
                (false, false) => (BinaryOp::SaturatingAdd { unsigned: u }, false),
                (false, true) => (BinaryOp::SaturatingSub { unsigned: u }, false),
                (true, false) => (BinaryOp::SaturatingAddMixed { unsigned: u }, false),
                (true, true) => (BinaryOp::SaturatingSub { unsigned: u }, true),
            },
            _ => return self.undefined(pc, insn),
        };
        let es = Esize::from_size(bits(insn, 23, 22));
        let (g, m, dn) = (bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let mask = self.p(g, vl);
        let operand1 = self.z(dn, vl);
        let operand2 = self.z(m, vl);
        let mut result = operand1;
        for e in (0..es.elements(vl)).filter(|&e| mask.active(e, es)) {
            let (a, b) = if matches!(op, BinaryOp::Pairwise(_)) {
                if e % 2 == 0 {
                    (operand1.get(e, es), operand1.get(e + 1, es))
                } else {
                    (operand2.get(e - 1, es), operand2.get(e, es))
                }
            } else if reversed {
                (operand2.get(e, es), operand1.get(e, es))
            } else {
                (operand1.get(e, es), operand2.get(e, es))
            };
            result.set(e, es, op.apply(a, b, es));
        }
        self.set_z(dn, vl, &result);
        Ok(Flow::Next)
    }

    /// URECPE, URSQRTE, SQABS, SQNEG (`01000100 size 0 0 Q 0 Z op 101 Pg
    /// Zn Zd`, merging with `Z = 0`) and SADALP, UADALP (`01000100 size 0
    /// 0010 U 101 Pg Zn Zda`), merging. URECPE and URSQRTE are of words
    /// alone (`size = 10`, other sizes UNDEFINED); SADALP and UADALP add each
    /// pair of half-size elements of Zn to Zda's element (`size = 00`
    /// UNDEFINED).
    pub(crate) fn exec_sve_intx_unary_pred(&mut self, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 23, 22);
        let op16 = bit(insn, 16);
        let op = match (bits(insn, 20, 17), size) {
            (0b0000, 0b10) => UnaryOp::Estimate { square_root: op16 },
            (0b0100, _) => UnaryOp::Saturating { negate: op16 },
            (0b0010, 0b01..) => UnaryOp::AccumulatePairs { unsigned: op16 },
            _ => return self.undefined(pc, insn),
        };
        let es = Esize::from_size(size);
        // SADALP's and UADALP's source elements: `size` is not `00`.
        let half = Esize::from_size(size.saturating_sub(1));
        let (g, n, d) = (bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let mask = self.p(g, vl);
        let operand = self.z(n, vl);
        let mut result = self.z(d, vl);
        for e in (0..es.elements(vl)).filter(|&e| mask.active(e, es)) {
            let r = match op {
                UnaryOp::Estimate { square_root } => {
                    let element = operand.get(e, es) as u32;
                    u64::from(if square_root { ursqrte(element) } else { urecpe(element) })
                }
                UnaryOp::Saturating { negate } => {
                    let element = int(operand.get(e, es), es, false);
                    sat_q(if negate { -element } else { element.abs() }, es, false)
                }
                UnaryOp::AccumulatePairs { unsigned } => {
                    let sum = int(operand.get(2 * e, half), half, unsigned)
                        + int(operand.get(2 * e + 1, half), half, unsigned);
                    result.get(e, es).wrapping_add(sum as u64)
                }
            };
            result.set(e, es, r);
        }
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// SCLAMP, UCLAMP: `01000100 size 0 Zm 11000 U Zn Zd`. Each element of
    /// Zd is clamped to Zn's element below and Zm's above, `Min(Max(Zn, Zd),
    /// Zm)`, signed or unsigned.
    pub(crate) fn exec_sve_intx_clamp(&mut self, insn: u32) -> ExecResult {
        let unsigned = bit(insn, 10);
        let es = Esize::from_size(bits(insn, 23, 22));
        let (m, n, d) = (bits(insn, 20, 16), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let low = self.z(n, vl);
        let high = self.z(m, vl);
        let mut result = self.z(d, vl);
        for e in 0..es.elements(vl) {
            let value = int(result.get(e, es), es, unsigned);
            let clamped = value.max(int(low.get(e, es), es, unsigned)).min(int(high.get(e, es), es, unsigned));
            result.set(e, es, clamped as u64);
        }
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// CMLA and SQRDCMLAH (vectors: `01000100 size 0 Zm 001 op rot Zn
    /// Zda`; indexed: `01000100 size 1 opc 0110 rot Zn Zda`, `0111` for
    /// SQRDCMLAH) and CDOT (vectors: `01000100 size 0 Zm 0001 rot Zn Zda`;
    /// indexed: `01000100 size 1 opc 0100 rot Zn Zda`). CMLA adds to each
    /// pair (real, imaginary) of Zda the products of one element of Zn's
    /// pair with Zm's pair, rotated by `rot` x 90 degrees, wrapping;
    /// SQRDCMLAH doubles them, rounds and saturates as SQRDMLAH does. CDOT
    /// adds to each element of Zda the real parts of the two complex
    /// products of its quarter-size pairs of Zn with Zm's, rotated. By
    /// index, Zm's pair (CDOT's four elements) is the one `index` of its
    /// 128-bit segment. CDOT by vector with `size = 0x` is UNDEFINED.
    pub(crate) fn exec_sve_intx_complex(&mut self, pc: u64, insn: u32) -> ExecResult {
        let indexed = bit(insn, 21);
        // (CDOT, SQRDCMLAH).
        let (cdot, saturating) = match (indexed, bits(insn, 15, 12)) {
            (false, 0b0001) | (true, 0b0100) => (true, false),
            (false, 0b0010) | (true, 0b0110) => (false, false),
            (false, 0b0011) | (true, 0b0111) => (false, true),
            _ => return self.undefined(pc, insn),
        };
        if cdot && !bit(insn, 23) {
            return self.undefined(pc, insn);
        }
        let es = match (indexed, cdot) {
            // CDOT: `size = 1x`, words or (`size<0>`) doublewords.
            (_, true) => {
                if bit(insn, 22) {
                    Esize::D
                } else {
                    Esize::S
                }
            }
            // CMLA, SQRDCMLAH by element: `size = 1x`, halfwords or
            // (`size<0>`) words.
            (true, false) => {
                if bit(insn, 22) {
                    Esize::S
                } else {
                    Esize::H
                }
            }
            (false, false) => Esize::from_size(bits(insn, 23, 22)),
        };
        let form = if indexed { Some(Indexed::pair(insn)) } else { None };
        let m = form.map_or(bits(insn, 20, 16), |f| f.m);
        let rotation = Rotation(bits(insn, 11, 10));
        let (n, da) = (bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let op1 = self.z(n, vl);
        let op2 = self.z(m, vl);
        let op3 = self.z(da, vl);
        let mut result = Zv::ZERO;
        if cdot {
            let q = if es == Esize::D { Esize::H } else { Esize::B };
            for e in 0..es.elements(vl) {
                let s = form.map_or(e, |f| segment_base(e, 16 / es.bytes()) + f.index);
                let mut sum = i128::from(op3.get(e, es));
                for i in 0..2 {
                    let elt1_r = int(op1.get(4 * e + 2 * i, q), q, false);
                    let elt1_i = int(op1.get(4 * e + 2 * i + 1, q), q, false);
                    let elt2_a = int(op2.get(4 * s + 2 * i + rotation.sel_a(), q), q, false);
                    let elt2_b = int(op2.get(4 * s + 2 * i + rotation.sel_b(), q), q, false);
                    sum += elt1_r * elt2_a;
                    sum += if rotation.dot_sub_i() { -(elt1_i * elt2_b) } else { elt1_i * elt2_b };
                }
                result.set(e, es, sum as u64);
            }
        } else {
            let pairs_per_segment = 16 / (2 * es.bytes());
            for p in 0..es.elements(vl) / 2 {
                let s = form.map_or(p, |f| segment_base(p, pairs_per_segment) + f.index);
                let elt1_a = int(op1.get(2 * p + rotation.sel_a(), es), es, false);
                let elt2_a = int(op2.get(2 * s + rotation.sel_a(), es), es, false);
                let elt2_b = int(op2.get(2 * s + rotation.sel_b(), es), es, false);
                let elt3_r = int(op3.get(2 * p, es), es, false);
                let elt3_i = int(op3.get(2 * p + 1, es), es, false);
                let (real, imag) = if saturating {
                    let real = rounding_doubling_mul_add(elt3_r, elt1_a, elt2_a, es.bits(), rotation.sub_r());
                    let imag = rounding_doubling_mul_add(elt3_i, elt1_a, elt2_b, es.bits(), rotation.sub_i());
                    (sat_q(real, es, false), sat_q(imag, es, false))
                } else {
                    let product_r = elt1_a * elt2_a;
                    let product_i = elt1_a * elt2_b;
                    let real = if rotation.sub_r() { elt3_r - product_r } else { elt3_r + product_r };
                    let imag = if rotation.sub_i() { elt3_i - product_i } else { elt3_i + product_i };
                    (real as u64, imag as u64)
                };
                result.set(2 * p, es, real);
                result.set(2 * p + 1, es, imag);
            }
        }
        self.set_z(da, vl, &result);
        Ok(Flow::Next)
    }
}
