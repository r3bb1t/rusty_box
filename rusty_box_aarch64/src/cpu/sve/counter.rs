//! FEAT_SME2's predicate-as-counter: a predicate register whose low 16 bits
//! count the active elements of up to four vectors (`CounterToPredicate`,
//! `EncodePredCount`, `PredCountTest`), and the instructions that write or
//! read one: WHILE and PTRUE (predicate-as-counter), PEXT, CNTP
//! (predicate-as-counter), and the WHILE that writes a pair of predicates.
//! A predicate-as-counter register is PN8 to PN15 where an encoding names
//! it in three bits (`'1':PNd`), any of PN0 to PN15 in four.
//!
//! Provenance (R7): the Arm A64 ISA XML, release 2026-09 (Arm ARM M.d), the
//! instruction pages each handler names and the shared pseudocode in
//! `aarch64/functions/sme` (`CounterToPredicate`, `EncodePredCount`,
//! `PredCountTest`).

#[cfg(test)]
#[path = "../../../tests/unit/pred_counter.rs"]
mod tests;

use super::pred::WhileCompare;
use super::{Esize, Pv};
use crate::cpu::exec::{bit, bits, ExecResult, Flow};
use crate::cpu::features::VecLen;
use crate::cpu::Cpu;

/// A predicate-as-counter, the low 16 bits of a predicate register, as
/// `CounterToPredicate{PL*4}` reads them at one vector length: the low set
/// bit gives the counter's element size (bit 0 bytes, to bit 3
/// doublewords), the bits above it to `FloorLog2(PL * 4)` the count, and
/// bit 15 whether the count's elements are the inactive ones, of the
/// elements four vectors hold at that size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PredCounter {
    es: Esize,
    elements: usize,
    count: usize,
    invert: bool,
}

impl PredCounter {
    /// The counter predicate `p` holds, at vector length `vl`. A low
    /// nibble of zero activates no element, as a count of zero not
    /// inverted does.
    pub(crate) fn of(p: &Pv, vl: VecLen) -> Self {
        let pred = u16::from_le_bytes([p.byte(0), p.byte(1)]);
        let lsb = pred.trailing_zeros();
        if lsb > 3 {
            return Self {
                es: Esize::B,
                elements: 0,
                count: 0,
                invert: false,
            };
        }
        let es = Esize::from_size(lsb);
        // `maxbit = FloorLog2(PL * 4)`, PL being VL DIV 8 bits.
        let maxbit = (vl.bits() / 2).ilog2();
        Self {
            es,
            elements: 4 * es.elements(vl),
            count: ((u32::from(pred) >> (lsb + 1)) & ((1 << (maxbit - lsb)) - 1)) as usize,
            invert: pred & 0x8000 != 0,
        }
    }

    /// `ActivePredicateElement{PL*4}(mask, e, esize)` of the predicate the
    /// counter stands for: its bit `e * esize DIV 8`, set where that byte
    /// begins one of the counter's own elements and the element is active.
    #[inline]
    pub(crate) fn active(self, e: usize, es: Esize) -> bool {
        let n = e * es.bytes();
        let own = self.es.bytes();
        n.is_multiple_of(own) && n / own < self.elements && ((n / own < self.count) != self.invert)
    }
}

/// Arm `EncodePredCount(esize, elements, count, invert)`: the
/// predicate-as-counter that makes `count` of `elements` active, the first
/// ones, or with `invert` the last ones; zero-extended to a predicate.
pub(crate) fn encode_pred_count(es: Esize, elements: usize, count: usize, invert: bool) -> Pv {
    if count == 0 {
        return Pv::ZERO;
    }
    let (count, invert) = match (invert, count == elements) {
        (true, _) => (elements - count, true),
        (false, true) => (0, true),
        (false, false) => (count, false),
    };
    let k = es.log2_bytes();
    let value = (u16::from(invert) << 15) | ((count << (k + 1)) as u16 & 0x7FFF) | (1 << k);
    Pv::from_bytes(&value.to_le_bytes())
}

/// Arm `PredCountTest(elements, count, invert)`: N that the first element
/// is active, Z that none is, C that the last is not, V zero; as
/// `PSTATE.NZCV` holds them (bits 31:28).
pub(crate) fn pred_count_test(elements: usize, count: usize, invert: bool) -> u64 {
    let (n, c) = if invert {
        (count == elements, count == 0)
    } else {
        (count != 0, count != elements)
    };
    (u64::from(n) << 31) | (u64::from(count == 0) << 30) | (u64::from(c) << 29)
}

impl Cpu {
    /// WHILEGE, WHILEGT, WHILELT, WHILELE, WHILEHS, WHILEHI, WHILELO,
    /// WHILELS (predicate-as-counter) (`whilege_pn_rr_` to
    /// `whilels_pn_rr_`: `00100101 size 1 Rm 01 vl 0 U lt Rn 1 eq PNd`): the
    /// count of the elements of `2 << vl` vectors that compare true in a
    /// row, from the first counting up or the last counting down, as a
    /// predicate-as-counter in PN8 to PN15; `PSTATE.NZCV` by
    /// `PredCountTest`.
    pub(crate) fn exec_sve_while_counter(&mut self, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let width = 2 << bits(insn, 13, 13);
        let compare = WhileCompare::of(
            insn,
            bit(insn, 3),
            Esize::D,
            self.x_or_zr(bits(insn, 9, 5)),
            self.x_or_zr(bits(insn, 20, 16)),
        );
        let d = 8 + bits(insn, 2, 0);
        let vl = self.sve_check()?;
        let elements = width * es.elements(vl);
        let count = compare.count(elements);
        let invert = !compare.up;
        self.pstate
            .set_nzcv(pred_count_test(elements, count, invert));
        self.set_p(d, vl, &encode_pred_count(es, elements, count, invert));
        Ok(Flow::Next)
    }

    /// WHILEGE, WHILEGT, WHILELT, WHILELE, WHILEHS, WHILEHI, WHILELO,
    /// WHILELS (pair of predicates) (`whilege_pp_rr_` to `whilels_pp_rr_`:
    /// `00100101 size 1 Rm 01 01 U lt Rn 1 Pd eq`): the WHILE of two
    /// vectors' elements, the first vector's predicate to `Pd:0` and the
    /// second's to `Pd:1`; `PSTATE.NZCV` by `PredTest` over both, which of
    /// the first or the last `count` elements active is
    /// `PredCountTest(elements, count, invert)` of the same WHILE to a
    /// counter.
    pub(crate) fn exec_sve_while_pair(&mut self, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let compare = WhileCompare::of(
            insn,
            bit(insn, 0),
            Esize::D,
            self.x_or_zr(bits(insn, 9, 5)),
            self.x_or_zr(bits(insn, 20, 16)),
        );
        let d = bits(insn, 3, 1) << 1;
        let vl = self.sve_check()?;
        let elements = es.elements(vl);
        let count = compare.count(2 * elements);
        let mut halves = [Pv::ZERO; 2];
        for (r, half) in halves.iter_mut().enumerate() {
            for e in 0..elements {
                half.set_element(e, es, compare.active(r * elements + e, 2 * elements, count));
            }
        }
        self.pstate
            .set_nzcv(pred_count_test(2 * elements, count, !compare.up));
        self.set_p(d, vl, &halves[0]);
        self.set_p(d + 1, vl, &halves[1]);
        Ok(Flow::Next)
    }

    /// PTRUE (predicate-as-counter) (`ptrue_pn_i_`: `00100101 size 1 00000
    /// 01 111 000000 1 0 PNd`): every element active, as a
    /// predicate-as-counter in PN8 to PN15.
    pub(crate) fn exec_sve_ptrue_counter(&mut self, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let d = 8 + bits(insn, 2, 0);
        let vl = self.sve_check()?;
        let elements = es.elements(vl);
        self.set_p(d, vl, &encode_pred_count(es, elements, elements, false));
        Ok(Flow::Next)
    }

    /// PEXT (predicate) (`pext_pn_rr_`: `00100101 size 1 00000 01 110 0 imm2
    /// PNn 1 Pd`) and PEXT (pair of predicates) (`pext_pp_rr_`: `... 110 10
    /// i1 PNn 1 Pd`, by bit 10): vector `imm2` of the predicate-as-counter
    /// PN8 to PN15 names, as a predicate in `Pd`, or vectors `2 * i1` and
    /// `2 * i1 + 1` in `Pd` and `(Pd + 1) MOD 16`.
    pub(crate) fn exec_sve_pext(&mut self, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let (n, d) = (8 + bits(insn, 7, 5), bits(insn, 3, 0));
        // The vectors extracted: from `first`, `count` of them.
        let (first, count) = if bit(insn, 10) {
            (2 * bits(insn, 8, 8) as usize, 2)
        } else {
            (bits(insn, 9, 8) as usize, 1)
        };
        let vl = self.sve_check()?;
        let counter = PredCounter::of(&self.p(n, vl), vl);
        let elements = es.elements(vl);
        for r in 0..count {
            let mut result = Pv::ZERO;
            for e in 0..elements {
                result.set_element(e, es, counter.active((first + r) * elements + e, es));
            }
            self.set_p((d + r as u32) % 16, vl, &result);
        }
        Ok(Flow::Next)
    }

    /// CNTP (predicate-as-counter) (`cntp_r_pn_`: `00100101 size 100 000 10
    /// 000 vl 1 PNn Rd`): how many elements of the first `2 << vl` vectors
    /// the predicate-as-counter `PNn` (PN0 to PN15) makes active, to `Xd`.
    pub(crate) fn exec_sve_cntp_counter(&mut self, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let width = 2 << bits(insn, 10, 10);
        let (n, d) = (bits(insn, 8, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let counter = PredCounter::of(&self.p(n, vl), vl);
        let count = (0..width * es.elements(vl))
            .filter(|&e| counter.active(e, es))
            .count();
        self.set_x_or_zr(d, count as u64);
        Ok(Flow::Next)
    }
}
