//! SVE predicate-generating and predicate-reading instructions: the integer
//! compares, the predicate logical operations, the breaks, PTEST, PFIRST,
//! PNEXT, PTRUE, PFALSE, PSEL, WHILE, CTERM, CNTP and the inc/dec by
//! predicate count. Each handler names the Arm pages it is.

use super::{decode_pred_count, int, pred_test, sat_add_scalar, sat_q, Esize, Pv, Zv};
use crate::cpu::exec::{bit, bits, sext, ExecResult, Flow};
use crate::cpu::features::VecLen;
use crate::cpu::{Cpu, Levels};

/// Arm `CmpOp`: the comparison of an integer compare or `WHILE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CmpOp {
    Eq,
    Ne,
    Ge,
    Lt,
    Gt,
    Le,
}

impl CmpOp {
    fn holds(self, a: i128, b: i128) -> bool {
        match self {
            Self::Eq => a == b,
            Self::Ne => a != b,
            Self::Ge => a >= b,
            Self::Lt => a < b,
            Self::Gt => a > b,
            Self::Le => a <= b,
        }
    }
}

/// Where an integer compare's second operand comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operand2 {
    /// `Z<m>`'s element of the same size.
    Vector,
    /// `Z<m>`'s 64-bit element that holds the element compared.
    Wide,
    /// An immediate, already as `UInt` or `SInt`.
    Immediate(i128),
}

/// The active elements of a predicate counted: `mask AND operand`'s.
fn count_active(mask: &Pv, operand: &Pv, es: Esize, vl: VecLen) -> usize {
    (0..es.elements(vl))
        .filter(|&e| mask.active(e, es) && operand.active(e, es))
        .count()
}

/// What a WHILE compares: `Xn` stepping by one each element, and `Xm`, as
/// `UInt` or `SInt` of their low `rsize` bits.
#[derive(Debug, Clone, Copy)]
pub(super) struct WhileCompare {
    operand1: u64,
    operand2: u64,
    rsize: Esize,
    unsigned: bool,
    cmp: CmpOp,
    /// Counting up (`operand1 + 1` each element, from the first element),
    /// else down (from the last).
    pub(super) up: bool,
}

impl WhileCompare {
    /// The comparison of the WHILE `insn` between `Xn` (`operand1`) and
    /// `Xm` (`operand2`) at `rsize` bits: WHILELT, WHILELE, WHILELO and
    /// WHILELS count up by `lt` (bit 10), WHILEGE, WHILEGT, WHILEHS and
    /// WHILEHI down; unsigned by `U` (bit 11), and with `eq` the inclusive
    /// compare.
    pub(super) fn of(insn: u32, eq: bool, rsize: Esize, operand1: u64, operand2: u64) -> Self {
        let up = bit(insn, 10);
        let cmp = match (up, eq) {
            (true, false) => CmpOp::Lt,
            (true, true) => CmpOp::Le,
            (false, false) => CmpOp::Ge,
            (false, true) => CmpOp::Gt,
        };
        Self {
            operand1,
            operand2,
            rsize,
            unsigned: bit(insn, 11),
            cmp,
            up,
        }
    }

    /// How many of `iterations` elements a WHILE makes active, in the
    /// order it visits them: the compare holding each time in a row (Arm's
    /// `last`), `operand1` wrapping at `rsize` bits.
    pub(super) fn count(self, iterations: usize) -> usize {
        let element2 = int(self.operand2, self.rsize, self.unsigned);
        let mut operand1 = self.operand1;
        let mut count = 0;
        while count < iterations
            && self
                .cmp
                .holds(int(operand1, self.rsize, self.unsigned), element2)
        {
            count += 1;
            operand1 = if self.up {
                operand1.wrapping_add(1)
            } else {
                operand1.wrapping_sub(1)
            };
        }
        count
    }

    /// Whether element `e` of `elements` is one the WHILE makes active,
    /// `count` of them having compared true: the first `count` counting
    /// up, the last `count` counting down.
    pub(super) fn active(self, e: usize, elements: usize, count: usize) -> bool {
        if self.up {
            e < count
        } else {
            e >= elements - count
        }
    }
}

impl<L: Levels> Cpu<L> {
    /// The integer compares: CMP<cc> (vectors, `00100100 size 0 Zm op 0 o2
    /// Pg Zn ne Pd`), CMP<cc> (wide elements, `... size 0 Zm U 1 lt Pg Zn ne
    /// Pd`, 8- to 32-bit elements), CMPHS, CMPHI, CMPLO, CMPLS (unsigned
    /// immediate, `00100100 size 1 imm7 lt Pg Zn ne Pd`) and CMP<cc> (signed
    /// immediate, `00100101 size 0 imm5 op 0 o2 Pg Zn ne Pd`). Each writes
    /// the predicate of the active elements that compare true and sets
    /// `PSTATE.NZCV` by `PredTest`.
    pub(crate) fn exec_sve_compare(&mut self, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 23, 22);
        let es = Esize::from_size(size);
        let (g, n, d) = (bits(insn, 12, 10), bits(insn, 9, 5), bits(insn, 3, 0));
        let ne = bit(insn, 4);
        let (cmp, unsigned, source) = match (bit(insn, 24), bit(insn, 21)) {
            // `o2 = 1` is CMPEQ and CMPNE, by wide elements with `op = 0`.
            (false, false) if !bit(insn, 14) => {
                let (op, o2) = (bit(insn, 15), bit(insn, 13));
                let source = if o2 && !op {
                    Operand2::Wide
                } else {
                    Operand2::Vector
                };
                match (op, o2, ne) {
                    (false, false, false) => (CmpOp::Ge, true, source),
                    (true, false, false) => (CmpOp::Ge, false, source),
                    (_, true, false) => (CmpOp::Eq, false, source),
                    (false, false, true) => (CmpOp::Gt, true, source),
                    (true, false, true) => (CmpOp::Gt, false, source),
                    (_, true, true) => (CmpOp::Ne, false, source),
                }
            }
            (false, false) => {
                let unsigned = bit(insn, 15);
                let cmp = match (bit(insn, 13), ne) {
                    (false, false) => CmpOp::Ge,
                    (false, true) => CmpOp::Gt,
                    (true, false) => CmpOp::Lt,
                    (true, true) => CmpOp::Le,
                };
                (cmp, unsigned, Operand2::Wide)
            }
            (false, true) => {
                let imm = i128::from(bits(insn, 20, 14));
                let cmp = match (bit(insn, 13), ne) {
                    (false, false) => CmpOp::Ge,
                    (false, true) => CmpOp::Gt,
                    (true, false) => CmpOp::Lt,
                    (true, true) => CmpOp::Le,
                };
                (cmp, true, Operand2::Immediate(imm))
            }
            (true, _) => {
                let imm = i128::from(sext(u64::from(bits(insn, 20, 16)), 5) as i64);
                let cmp = match (bit(insn, 15), bit(insn, 13), ne) {
                    (false, false, false) => CmpOp::Ge,
                    (false, false, true) => CmpOp::Gt,
                    (false, true, false) => CmpOp::Lt,
                    (false, true, true) => CmpOp::Le,
                    (true, false, false) => CmpOp::Eq,
                    (true, false, true) => CmpOp::Ne,
                    (true, true, _) => return self.undefined(pc, insn),
                };
                (cmp, false, Operand2::Immediate(imm))
            }
        };
        if source == Operand2::Wide && size == 0b11 {
            return self.undefined(pc, insn);
        }
        let m = bits(insn, 20, 16);
        let vl = self.sve_check()?;
        let mask = self.p(g, vl);
        let operand1 = self.z(n, vl);
        let operand2 = match source {
            Operand2::Immediate(_) => Zv::ZERO,
            Operand2::Vector | Operand2::Wide => self.z(m, vl),
        };
        let mut result = Pv::ZERO;
        for e in 0..es.elements(vl) {
            if !mask.active(e, es) {
                continue;
            }
            let element1 = int(operand1.get(e, es), es, unsigned);
            let element2 = match source {
                Operand2::Vector => int(operand2.get(e, es), es, unsigned),
                Operand2::Wide => int(
                    operand2.get(e * es.bytes() / 8, Esize::D),
                    Esize::D,
                    unsigned,
                ),
                Operand2::Immediate(imm) => imm,
            };
            result.set_element(e, es, cmp.holds(element1, element2));
        }
        self.pstate.set_nzcv(pred_test(&mask, &result, es, vl));
        self.set_p(d, vl, &result);
        Ok(Flow::Next)
    }

    /// AND, BIC, ORR, ORN, EOR, NOR, NAND (predicates, zeroing), their
    /// flag-setting forms, and SEL (predicates): `00100101 op S 00 Pm 01 Pg
    /// o2 Pn o3 Pd`, bit by bit, inactive bits zero (SEL: `Pm`'s).
    pub(crate) fn exec_sve_pred_logical(&mut self, insn: u32) -> ExecResult {
        let (m, g, n, d) = (
            bits(insn, 19, 16),
            bits(insn, 13, 10),
            bits(insn, 8, 5),
            bits(insn, 3, 0),
        );
        let setflags = bit(insn, 22);
        let op = (bit(insn, 23), bit(insn, 9), bit(insn, 4));
        let vl = self.sve_check()?;
        let mask = self.p(g, vl);
        let operand1 = self.p(n, vl);
        let operand2 = self.p(m, vl);
        let mut result = Pv::ZERO;
        for i in 0..vl.predicate_bytes() {
            let (gm, a, b) = (mask.byte(i), operand1.byte(i), operand2.byte(i));
            let r = match op {
                (false, false, false) => a & b,
                (false, false, true) => a & !b,
                (true, false, false) => a | b,
                (true, false, true) => a | !b,
                (false, true, false) => a ^ b,
                (false, true, true) => (a & gm) | (b & !gm),
                (true, true, false) => !(a | b),
                (true, true, true) => !(a & b),
            };
            let r = if op == (false, true, true) { r } else { r & gm };
            result.set_byte(i, r);
        }
        if setflags {
            self.pstate
                .set_nzcv(pred_test(&mask, &result, Esize::B, vl));
        }
        self.set_p(d, vl, &result);
        Ok(Flow::Next)
    }

    /// The breaks: BRKPA, BRKPB (`00100101 0 S 00 Pm 11 Pg 0 Pn B Pd`),
    /// BRKA, BRKB (`00100101 B S 010000 01 Pg 0 Pn M Pd`, merging with `M =
    /// 1`) and BRKN (`00100101 0 S 011000 01 Pg 0 Pn 0 Pdm`), and their
    /// flag-setting forms.
    pub(crate) fn exec_sve_break(&mut self, insn: u32) -> ExecResult {
        let (g, n, d) = (bits(insn, 13, 10), bits(insn, 8, 5), bits(insn, 3, 0));
        let setflags = bit(insn, 22);
        let es = Esize::B;
        let vl = self.sve_check()?;
        let mask = self.p(g, vl);
        let operand1 = self.p(n, vl);
        let elements = es.elements(vl);
        let mut result = Pv::ZERO;
        let mut flags_mask = mask;
        if bit(insn, 15) {
            // BRKPA, BRKPB: from the last active element of Pn, true until
            // the first active true element of Pm (after it, BRKPA; before
            // it, BRKPB).
            let operand2 = self.p(bits(insn, 19, 16), vl);
            let before = bit(insn, 4);
            let mut last = (0..elements)
                .rev()
                .find(|&e| mask.active(e, es))
                .is_some_and(|e| operand1.active(e, es));
            for e in 0..elements {
                if mask.active(e, es) {
                    if before {
                        last = last && !operand2.active(e, es);
                    }
                    result.set_element(e, es, last);
                    if !before {
                        last = last && !operand2.active(e, es);
                    }
                }
            }
        } else if bits(insn, 19, 16) == 0b1000 {
            // BRKN: Pdm if the last active element of Pn is true, else zero.
            let last = (0..elements)
                .rev()
                .find(|&e| mask.active(e, es))
                .is_some_and(|e| operand1.active(e, es));
            if last {
                result = self.p(d, vl);
            }
            flags_mask = Pv::ones(vl);
        } else {
            // BRKA, BRKB: true up to the first active true element of Pn
            // (after it, BRKA; before it, BRKB).
            let before = bit(insn, 23);
            let merging = bit(insn, 4) && !setflags;
            let operand2 = if merging { self.p(d, vl) } else { Pv::ZERO };
            let mut broken = false;
            for e in 0..elements {
                let element = operand1.active(e, es);
                if mask.active(e, es) {
                    if before {
                        broken = broken || element;
                    }
                    result.set_element(e, es, !broken);
                    if !before {
                        broken = broken || element;
                    }
                } else if merging {
                    result.set_element(e, es, operand2.active(e, es));
                }
            }
        }
        if setflags {
            self.pstate
                .set_nzcv(pred_test(&flags_mask, &result, es, vl));
        }
        self.set_p(d, vl, &result);
        Ok(Flow::Next)
    }

    /// PTEST (`00100101 01 010000 11 Pg 0 Pn 0 0000`), PFIRST (`... 01 011000
    /// 1100000 Pg 0 Pdn`), PNEXT (`... size 011001 1100010 Pv 0 Pdn`), PTRUE
    /// and PTRUES (`... size 01100 S 111000 pattern 0 Pd`), PFALSE, and PSEL
    /// (`00100101 i1 tszh 1 tszl Rv 01 Pn 0 Pm 0 Pd`).
    pub(crate) fn exec_sve_pred_misc(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bit(insn, 21) {
            return self.exec_psel(pc, insn);
        }
        let es = Esize::from_size(bits(insn, 23, 22));
        match (bits(insn, 19, 16), bits(insn, 13, 11)) {
            (0b0000, _) => {
                let (g, n) = (bits(insn, 13, 10), bits(insn, 8, 5));
                let vl = self.sve_check()?;
                let mask = self.p(g, vl);
                let result = self.p(n, vl);
                self.pstate
                    .set_nzcv(pred_test(&mask, &result, Esize::B, vl));
            }
            (0b1000, 0b000) => {
                let (g, dn) = (bits(insn, 8, 5), bits(insn, 3, 0));
                let vl = self.sve_check()?;
                let mask = self.p(g, vl);
                let mut result = self.p(dn, vl);
                if let Some(first) = (0..Esize::B.elements(vl)).find(|&e| mask.active(e, Esize::B))
                {
                    result.set_bit(first, true);
                }
                self.pstate
                    .set_nzcv(pred_test(&mask, &result, Esize::B, vl));
                self.set_p(dn, vl, &result);
            }
            (0b1001, 0b000) => {
                let (v, dn) = (bits(insn, 8, 5), bits(insn, 3, 0));
                let vl = self.sve_check()?;
                let mask = self.p(v, vl);
                let operand = self.p(dn, vl);
                let elements = es.elements(vl);
                let start = (0..elements)
                    .rev()
                    .find(|&e| operand.active(e, es))
                    .map_or(0, |e| e + 1);
                let mut result = Pv::ZERO;
                if let Some(next) = (start..elements).find(|&e| mask.active(e, es)) {
                    result.set_element(next, es, true);
                }
                self.pstate.set_nzcv(pred_test(&mask, &result, es, vl));
                self.set_p(dn, vl, &result);
            }
            (0b1000 | 0b1001, 0b100) if !bit(insn, 10) => {
                let (pattern, d) = (bits(insn, 9, 5), bits(insn, 3, 0));
                let setflags = bit(insn, 16);
                let vl = self.sve_check()?;
                let count = decode_pred_count(pattern, es, vl);
                let mut result = Pv::ZERO;
                for e in 0..es.elements(vl) {
                    result.set_element(e, es, e < count);
                }
                if setflags {
                    self.pstate.set_nzcv(pred_test(&result, &result, es, vl));
                }
                self.set_p(d, vl, &result);
            }
            (0b1000, 0b100) => {
                let d = bits(insn, 3, 0);
                let vl = self.sve_check()?;
                self.set_p(d, vl, &Pv::ZERO);
            }
            _ => return self.undefined(pc, insn),
        }
        Ok(Flow::Next)
    }

    /// PSEL: `Pn` if element `(W<12+Rv> + imm) MOD elements` of `Pm` is
    /// active, else zero; the element size and `imm` from `i1:tszh:tszl`.
    fn exec_psel(&mut self, pc: u64, insn: u32) -> ExecResult {
        let tsz = (bits(insn, 22, 22) << 3) | bits(insn, 20, 18);
        let imm5 = (bits(insn, 23, 23) << 4) | tsz;
        let (es, imm) = match tsz {
            0b1000 => (Esize::D, imm5 >> 4),
            t if t & 0b111 == 0b100 => (Esize::S, imm5 >> 3),
            t if t & 0b11 == 0b10 => (Esize::H, imm5 >> 2),
            t if t & 1 == 1 => (Esize::B, imm5 >> 1),
            _ => return self.undefined(pc, insn),
        };
        let (n, m, d) = (bits(insn, 13, 10), bits(insn, 8, 5), bits(insn, 3, 0));
        let v = 12 + bits(insn, 17, 16);
        let vl = self.sve_check()?;
        let operand1 = self.p(n, vl);
        let operand2 = self.p(m, vl);
        let idx = u64::from(self.x_or_zr(v) as u32);
        let element = ((idx + u64::from(imm)) % es.elements(vl) as u64) as usize;
        let result = if operand2.active(element, es) {
            operand1
        } else {
            Pv::ZERO
        };
        self.set_p(d, vl, &result);
        Ok(Flow::Next)
    }

    /// WHILELT, WHILELE, WHILELO, WHILELS (counting up from `Rn`) and
    /// WHILEGE, WHILEGT, WHILEHS, WHILEHI (counting down from `Rn`, SVE2):
    /// `00100101 size 1 Rm 000 sf U lt Rn eq Pd`; and WHILEWR, WHILERW
    /// (`00100101 size 1 Rm 001100 Rn rw Pd`, SVE2). Each sets `PSTATE.NZCV`
    /// by `PredTest`.
    pub(crate) fn exec_sve_while(&mut self, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let (m, n, d) = (bits(insn, 20, 16), bits(insn, 9, 5), bits(insn, 3, 0));
        let (x, y) = (self.x_or_zr(n), self.x_or_zr(m));
        let vl = self.sve_check()?;
        let elements = es.elements(vl);
        let mut result = Pv::ZERO;
        if bits(insn, 13, 12) == 0b11 {
            let difference = i128::from(y) - i128::from(x);
            let difference = if bit(insn, 4) {
                difference.abs()
            } else {
                difference
            };
            let diff = difference.div_euclid(es.bytes() as i128);
            for e in 0..elements {
                result.set_element(e, es, diff <= 0 || (e as i128) < diff);
            }
        } else {
            let rsize = if bit(insn, 12) { Esize::D } else { Esize::S };
            let compare = WhileCompare::of(insn, bit(insn, 4), rsize, x, y);
            let count = compare.count(elements);
            for e in 0..elements {
                result.set_element(e, es, compare.active(e, elements, count));
            }
        }
        self.pstate
            .set_nzcv(pred_test(&Pv::ones(vl), &result, es, vl));
        self.set_p(d, vl, &result);
        Ok(Flow::Next)
    }

    /// CTERMEQ, CTERMNE: `00100101 1 sz 1 Rm 001000 Rn ne 0000` — N set and V
    /// clear when the low 32 or 64 bits of `Rn` and `Rm` compare true, else
    /// N clear and V the inverse of C; Z and C are left alone.
    pub(crate) fn exec_sve_cterm(&mut self, insn: u32) -> ExecResult {
        let es = if bit(insn, 22) { Esize::D } else { Esize::S };
        let (a, b) = (
            self.x_or_zr(bits(insn, 9, 5)) & es.mask(),
            self.x_or_zr(bits(insn, 20, 16)) & es.mask(),
        );
        self.sve_check()?;
        let term = if bit(insn, 4) { a != b } else { a == b };
        let p = &mut self.pstate;
        if term {
            p.n = true;
            p.v = false;
        } else {
            p.n = false;
            p.v = !p.c;
        }
        Ok(Flow::Next)
    }

    /// CNTP (`00100101 size 100000 10 Pg 0 Pn Rd`), and the inc/dec by
    /// predicate count (`00100101 size 101 op0 D|op U|D 1000 op1 ... Pm
    /// Rdn|Zdn`): INCP, DECP and SQINCP, UQINCP, SQDECP, UQDECP of a vector
    /// (16- to 64-bit elements) and of a general register (32-bit, sign- or
    /// zero-extended, or 64-bit).
    pub(crate) fn exec_sve_pred_count(&mut self, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 23, 22);
        let es = Esize::from_size(size);
        let dn = bits(insn, 4, 0);
        if !bit(insn, 19) {
            let (g, n) = (bits(insn, 13, 10), bits(insn, 8, 5));
            let vl = self.sve_check()?;
            let count = count_active(&self.p(g, vl), &self.p(n, vl), es, vl);
            self.set_x_or_zr(dn, count as u64);
            return Ok(Flow::Next);
        }
        let saturating = !bit(insn, 18);
        let scalar = bit(insn, 11);
        let (decrement, unsigned) = if saturating {
            (bit(insn, 17), bit(insn, 16))
        } else {
            (bit(insn, 16), false)
        };
        if !scalar && size == 0b00 {
            return self.undefined(pc, insn);
        }
        let m = bits(insn, 8, 5);
        let vl = self.sve_check()?;
        let operand2 = self.p(m, vl);
        let count = count_active(&Pv::ones(vl), &operand2, es, vl) as i128;
        let delta = if decrement { -count } else { count };
        if scalar {
            let value = self.x_or_zr(dn);
            let result = if saturating {
                sat_add_scalar(value, delta, bit(insn, 10), unsigned)
            } else {
                value.wrapping_add(delta as u64)
            };
            self.set_x_or_zr(dn, result);
        } else {
            let operand1 = self.z(dn, vl);
            let mut result = Zv::ZERO;
            for e in 0..es.elements(vl) {
                let x = operand1.get(e, es);
                let r = if saturating {
                    sat_q(int(x, es, unsigned) + delta, es, unsigned)
                } else {
                    x.wrapping_add(delta as u64)
                };
                result.set(e, es, r);
            }
            self.set_z(dn, vl, &result);
        }
        Ok(Flow::Next)
    }
}
