//! SVE permutes, copies and selects: EXT, DUP, TBL, TBX, INSR, the
//! unpacks, REV, ZIP, UZP and TRN of vectors and of predicates, CPY,
//! LASTA, LASTB, CLASTA, CLASTB, the reverses within elements, SPLICE and
//! SEL. Each handler names the Arm pages it is.

use super::{Esize, Pv, Zv};
use crate::cpu::exec::{bit, bits, sext, ExecResult, Flow};
use crate::cpu::features::VecLen;
use crate::cpu::Cpu;

/// ZIP, UZP and TRN, of vectors and of predicates, by `opc<2:1>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interleave {
    Zip,
    Uzp,
    Trn,
}

/// Where one destination element of ZIP, UZP or TRN comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Source {
    /// From the second source operand, else the first.
    second: bool,
    /// Which element of it.
    index: usize,
}

impl Interleave {
    const fn from_opc(opc: u32) -> Option<Self> {
        match opc {
            0b00 => Some(Self::Zip),
            0b01 => Some(Self::Uzp),
            0b10 => Some(Self::Trn),
            _ => None,
        }
    }

    /// Destination element `i` of `elements`, for the `part` 0 (ZIP1, UZP1,
    /// TRN1) or 1 (ZIP2, UZP2, TRN2) form, as each page's loop writes it.
    const fn source(self, part: usize, elements: usize, i: usize) -> Source {
        let pairs = elements / 2;
        match self {
            // result[2p+0] = op1[base+p], result[2p+1] = op2[base+p].
            Self::Zip => Source { second: i % 2 == 1, index: part * pairs + i / 2 },
            // result[p] = op1[2p+part], result[pairs+p] = op2[2p+part].
            Self::Uzp => Source { second: i >= pairs, index: 2 * (i % pairs) + part },
            // result[2p+0] = op1[2p+part], result[2p+1] = op2[2p+part].
            Self::Trn => Source { second: i % 2 == 1, index: 2 * (i / 2) + part },
        }
    }
}

/// The predicate permutes of `sve_perm_predicates`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PredicatePermute {
    /// PUNPKLO, PUNPKHI.
    Unpack { high: bool },
    /// REV (predicate).
    Reverse,
    /// ZIP, UZP, TRN (predicates).
    Interleave(Interleave),
}

/// The predicated permutes of `sve_perm_pred`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PredicatedPermute {
    /// CPY (SIMD&FP scalar).
    CopyV,
    /// CPY (scalar), from `Rn|SP`.
    CopyR,
    /// LASTA, LASTB to a SIMD&FP register.
    LastV,
    /// LASTA, LASTB to a general register.
    LastR,
    /// REVB, REVH, REVW (the containers reversed within each element) and
    /// RBIT (`None`).
    Reverse(Option<Esize>),
    /// CLASTA, CLASTB to a vector.
    ClastZ,
    /// CLASTA, CLASTB to a SIMD&FP register.
    ClastV,
    /// CLASTA, CLASTB to a general register.
    ClastR,
    /// SPLICE, destructive or constructive (SVE2).
    Splice { constructive: bool },
    /// REVD.
    Revd,
}

/// `LastActiveElement(mask, esize)`: the highest active element, if any.
fn last_active(mask: &Pv, es: Esize, vl: VecLen) -> Option<usize> {
    (0..es.elements(vl)).rev().find(|&e| mask.active(e, es))
}

/// The element LASTA (`after`) or LASTB extracts: the one after the last
/// active element, wrapping to the first, or the last active element,
/// the vector's last without one.
fn last_index(mask: &Pv, es: Esize, vl: VecLen, after: bool) -> usize {
    let elements = es.elements(vl);
    match (last_active(mask, es, vl), after) {
        (Some(last), true) => (last + 1) % elements,
        (None, true) => 0,
        (Some(last), false) => last,
        (None, false) => elements - 1,
    }
}

/// The element CLASTA (`after`) or CLASTB extracts, if any element is
/// active.
fn conditional_last_index(mask: &Pv, es: Esize, vl: VecLen, after: bool) -> Option<usize> {
    last_active(mask, es, vl).map(|last| if after { (last + 1) % es.elements(vl) } else { last })
}

/// The bytes of `es`-sized elements `e`, within a vector: `operand[e*:esize]`
/// for any element size, the 128-bit ones of DUP (indexed) and REVD among
/// them.
fn copy_bytes(to: &mut Zv, to_byte: usize, from: &Zv, from_byte: usize, len: usize) {
    for i in 0..len {
        to.set_byte(to_byte + i, from.byte(from_byte + i));
    }
}

impl Cpu {
    /// EXT: `00000101 0 op0 1 imm8h 000 imm8l Zm|Zn Zdn|Zd` — the bytes from
    /// `imm8` on of `Z<s2>:Z<s1>`, `s1 = Zdn, s2 = Zm` (destructive, `op0 =
    /// 0`) or `s1 = Zn, s2 = Zn + 1` (constructive, SVE2); an offset beyond
    /// the vector gives `Z<s1>`.
    pub(crate) fn exec_sve_ext(&mut self, insn: u32) -> ExecResult {
        let position = ((bits(insn, 20, 16) << 3) | bits(insn, 12, 10)) as usize;
        let d = bits(insn, 4, 0);
        let (s1, s2) = if bit(insn, 22) {
            let n = bits(insn, 9, 5);
            (n, (n + 1) % 32)
        } else {
            (d, bits(insn, 9, 5))
        };
        let vl = self.sve_check()?;
        let operand1 = self.z(s1, vl);
        let operand2 = self.z(s2, vl);
        let len = vl.bytes();
        let result = if position >= len {
            operand1
        } else {
            let mut result = Zv::ZERO;
            for i in 0..len {
                let j = position + i;
                result.set_byte(i, if j < len { operand1.byte(j) } else { operand2.byte(j - len) });
            }
            result
        };
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// The unpredicated permutes: DUP (indexed), `00000101 imm2 1 tsz 001000
    /// Zn Zd` (`tsz`'s lowest set bit the element size, 8 to 128 bits, the
    /// bits above it and `imm2` the index; an index beyond the vector gives
    /// zeros); TBL with one and two table registers and TBX (`00000101 size
    /// 1 Zm 0010 1op|001100 Zn Zd`); and `00000101 size 1 op0 op1 001110 ...`:
    /// DUP (scalar, `Rn|SP`), INSR (scalar and SIMD&FP), SUNPKLO, SUNPKHI,
    /// UUNPKLO, UUNPKHI (16- to 64-bit elements) and REV (vector).
    pub(crate) fn exec_sve_perm_unpred(&mut self, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 23, 22);
        let es = Esize::from_size(size);
        let (n, d) = (bits(insn, 9, 5), bits(insn, 4, 0));
        match bits(insn, 15, 10) {
            0b001000 => {
                let tsz = bits(insn, 20, 16);
                if tsz == 0 {
                    return self.undefined(pc, insn);
                }
                let lsb = tsz.trailing_zeros();
                let ebytes = 1usize << lsb;
                let index = (((size << 5) | tsz) >> (lsb + 1)) as usize;
                let vl = self.sve_check()?;
                let operand = self.z(n, vl);
                let elements = vl.bytes() / ebytes;
                let mut result = Zv::ZERO;
                if index < elements {
                    for e in 0..elements {
                        copy_bytes(&mut result, e * ebytes, &operand, index * ebytes, ebytes);
                    }
                }
                self.set_z(d, vl, &result);
            }
            0b001010..=0b001100 => {
                let m = bits(insn, 20, 16);
                let (double_table, extend) = match bits(insn, 12, 10) {
                    0b010 => (true, false),
                    0b011 => (false, true),
                    _ => (false, false),
                };
                let vl = self.sve_check()?;
                let indexes = self.z(m, vl);
                let bottom = self.z(n, vl);
                let top = self.z((n + 1) % 32, vl);
                let elements = es.elements(vl);
                let table_elems = if double_table { 2 * elements } else { elements };
                let mut result = if extend { self.z(d, vl) } else { Zv::ZERO };
                for e in 0..elements {
                    let idx = indexes.get(e, es);
                    if idx < table_elems as u64 {
                        let idx = idx as usize;
                        let value = if idx < elements { bottom.get(idx, es) } else { top.get(idx - elements, es) };
                        result.set(e, es, value);
                    } else if !extend {
                        result.set(e, es, 0);
                    }
                }
                self.set_z(d, vl, &result);
            }
            0b001110 => match (bits(insn, 20, 19), bits(insn, 18, 16)) {
                (0b00, 0b000) => {
                    let value = self.x_or_sp(n);
                    let vl = self.sve_check()?;
                    self.set_z(d, vl, &Zv::replicate(value, es, vl));
                }
                (0b00 | 0b10, 0b100) => {
                    let from_v = bit(insn, 20);
                    let src = if from_v { self.v[n as usize] as u64 } else { self.x_or_zr(n) };
                    let vl = self.sve_check()?;
                    let dest = self.z(d, vl);
                    let mut result = Zv::ZERO;
                    result.set(0, es, src);
                    for e in 1..es.elements(vl) {
                        result.set(e, es, dest.get(e - 1, es));
                    }
                    self.set_z(d, vl, &result);
                }
                (0b10, 0b000..=0b011) => {
                    if size == 0b00 {
                        return self.undefined(pc, insn);
                    }
                    let unsigned = bit(insn, 17);
                    let hi = bit(insn, 16);
                    let half = match es {
                        Esize::H => Esize::B,
                        Esize::S => Esize::H,
                        Esize::B | Esize::D => Esize::S,
                    };
                    let vl = self.sve_check()?;
                    let operand = self.z(n, vl);
                    let elements = es.elements(vl);
                    let offset = if hi { elements } else { 0 };
                    let mut result = Zv::ZERO;
                    for e in 0..elements {
                        let x = operand.get(e + offset, half);
                        let r = if unsigned { x } else { sext(x, half.bits()) };
                        result.set(e, es, r);
                    }
                    self.set_z(d, vl, &result);
                }
                (0b11, 0b000) => {
                    let vl = self.sve_check()?;
                    let operand = self.z(n, vl);
                    let elements = es.elements(vl);
                    let mut result = Zv::ZERO;
                    for e in 0..elements {
                        result.set(e, es, operand.get(elements - 1 - e, es));
                    }
                    self.set_z(d, vl, &result);
                }
                _ => return self.undefined(pc, insn),
            },
            _ => return self.undefined(pc, insn),
        }
        Ok(Flow::Next)
    }

    /// The predicate permutes: ZIP1, ZIP2, UZP1, UZP2, TRN1, TRN2 (`00000101
    /// size 10 Pm 010 opc H 0 Pn 0 Pd`), PUNPKLO, PUNPKHI (`00000101 00 11000
    /// H 0100000 Pn 0 Pd`) and REV (predicate). Whole predicate elements,
    /// every bit of each, move.
    pub(crate) fn exec_sve_perm_predicate(&mut self, pc: u64, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let (n, d) = (bits(insn, 8, 5), bits(insn, 3, 0));
        let op = match bits(insn, 20, 16) {
            0b10000 | 0b10001 => PredicatePermute::Unpack { high: bit(insn, 16) },
            0b10100 => PredicatePermute::Reverse,
            0b00000..=0b01111 => match Interleave::from_opc(bits(insn, 12, 11)) {
                Some(kind) => PredicatePermute::Interleave(kind),
                None => return self.undefined(pc, insn),
            },
            _ => return self.undefined(pc, insn),
        };
        let vl = self.sve_check()?;
        let operand1 = self.p(n, vl);
        let elements = es.elements(vl);
        let mut result = Pv::ZERO;
        match op {
            // PUNPKLO, PUNPKHI: the low or high half's byte predicates as
            // halfword predicates.
            PredicatePermute::Unpack { high } => {
                let offset = if high { Esize::H.elements(vl) } else { 0 };
                for e in 0..Esize::H.elements(vl) {
                    result.set_element(e, Esize::H, operand1.active(e + offset, Esize::B));
                }
            }
            PredicatePermute::Reverse => {
                for e in 0..elements {
                    result.set_field(e, es, operand1.field(elements - 1 - e, es));
                }
            }
            PredicatePermute::Interleave(kind) => {
                let operand2 = self.p(bits(insn, 19, 16), vl);
                let part = usize::from(bit(insn, 10));
                for i in 0..elements {
                    let source = kind.source(part, elements, i);
                    let from = if source.second { &operand2 } else { &operand1 };
                    result.set_field(i, es, from.field(source.index, es));
                }
            }
        }
        self.set_p(d, vl, &result);
        Ok(Flow::Next)
    }

    /// ZIP1, ZIP2, UZP1, UZP2, TRN1, TRN2 (vectors): `00000101 size 1 Zm 011
    /// opc Zn Zd`.
    pub(crate) fn exec_sve_perm_elements(&mut self, pc: u64, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let (m, n, d) = (bits(insn, 20, 16), bits(insn, 9, 5), bits(insn, 4, 0));
        let opc = bits(insn, 12, 10);
        let Some(kind) = Interleave::from_opc(opc >> 1) else { return self.undefined(pc, insn) };
        let part = (opc & 1) as usize;
        let vl = self.sve_check()?;
        let operand1 = self.z(n, vl);
        let operand2 = self.z(m, vl);
        let elements = es.elements(vl);
        let mut result = Zv::ZERO;
        for i in 0..elements {
            let source = kind.source(part, elements, i);
            let from = if source.second { &operand2 } else { &operand1 };
            result.set(i, es, from.get(source.index, es));
        }
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }

    /// The predicated permutes (`00000101 size 1 op0 op1 op2 10 op3 Pg ...`):
    /// CPY (SIMD&FP scalar and scalar, `Rn|SP`), LASTA and LASTB (to a
    /// SIMD&FP or general register), CLASTA and CLASTB (to a vector, a
    /// SIMD&FP or a general register), REVB, REVH, REVW, RBIT, REVD
    /// (merging) and SPLICE (destructive and constructive).
    pub(crate) fn exec_sve_perm_predicated(&mut self, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 23, 22);
        let es = Esize::from_size(size);
        let g = bits(insn, 12, 10);
        let (n, d) = (bits(insn, 9, 5), bits(insn, 4, 0));
        let after = !bit(insn, 16);
        let op = match (bits(insn, 20, 17), bit(insn, 13)) {
            (0b0000, false) => PredicatedPermute::CopyV,
            (0b0100, true) => PredicatedPermute::CopyR,
            (0b0001, false) => PredicatedPermute::LastV,
            (0b0000, true) => PredicatedPermute::LastR,
            (0b0010 | 0b0011, false) => match (bits(insn, 17, 16), size) {
                (0b00, 0b01..) => PredicatedPermute::Reverse(Some(Esize::B)),
                (0b01, 0b10..) => PredicatedPermute::Reverse(Some(Esize::H)),
                (0b10, 0b11) => PredicatedPermute::Reverse(Some(Esize::S)),
                (0b11, _) => PredicatedPermute::Reverse(None),
                _ => return self.undefined(pc, insn),
            },
            (0b0100, false) => PredicatedPermute::ClastZ,
            (0b0101, false) => PredicatedPermute::ClastV,
            (0b1000, true) => PredicatedPermute::ClastR,
            (0b0110, false) => PredicatedPermute::Splice { constructive: bit(insn, 16) },
            (0b0111, false) if size == 0b00 && !bit(insn, 16) => PredicatedPermute::Revd,
            _ => return self.undefined(pc, insn),
        };
        let vl = self.sve_check()?;
        let mask = self.p(g, vl);
        match op {
            PredicatedPermute::CopyV | PredicatedPermute::CopyR => {
                let value = if op == PredicatedPermute::CopyR { self.x_or_sp(n) } else { self.v[n as usize] as u64 };
                let mut result = self.z(d, vl);
                for e in 0..es.elements(vl) {
                    if mask.active(e, es) {
                        result.set(e, es, value);
                    }
                }
                self.set_z(d, vl, &result);
            }
            PredicatedPermute::LastV => {
                let operand = self.z(n, vl);
                let value = operand.get(last_index(&mask, es, vl, after), es);
                self.write_v(d as usize, u128::from(value));
            }
            // W<d> for 8- to 32-bit elements.
            PredicatedPermute::LastR => {
                let operand = self.z(n, vl);
                let value = operand.get(last_index(&mask, es, vl, after), es);
                self.set_x_or_zr(d, value);
            }
            PredicatedPermute::Reverse(swap) => {
                let operand = self.z(n, vl);
                let mut result = self.z(d, vl);
                for e in 0..es.elements(vl) {
                    if !mask.active(e, es) {
                        continue;
                    }
                    let x = operand.get(e, es);
                    let r = match swap {
                        Some(sw) => {
                            let containers = es.bytes() / sw.bytes();
                            (0..containers).fold(0u64, |r, i| {
                                let c = (x >> (i as u32 * sw.bits())) & sw.mask();
                                r | (c << ((containers - 1 - i) as u32 * sw.bits()))
                            })
                        }
                        None => x.reverse_bits() >> (64 - es.bits()),
                    };
                    result.set(e, es, r);
                }
                self.set_z(d, vl, &result);
            }
            // The element broadcast, or Z<dn> unchanged when no element is
            // active.
            PredicatedPermute::ClastZ => {
                if let Some(last) = conditional_last_index(&mask, es, vl, after) {
                    let operand2 = self.z(n, vl);
                    let value = operand2.get(last, es);
                    self.set_z(d, vl, &Zv::replicate(value, es, vl));
                } else {
                    let operand1 = self.z(d, vl);
                    self.set_z(d, vl, &operand1);
                }
            }
            PredicatedPermute::ClastV => {
                let value = match conditional_last_index(&mask, es, vl, after) {
                    Some(last) => self.z(n, vl).get(last, es),
                    None => self.v[d as usize] as u64 & es.mask(),
                };
                self.write_v(d as usize, u128::from(value));
            }
            PredicatedPermute::ClastR => {
                let value = match conditional_last_index(&mask, es, vl, after) {
                    Some(last) => self.z(n, vl).get(last, es),
                    None => self.x_or_zr(d) & es.mask(),
                };
                self.set_x_or_zr(d, value);
            }
            // The active segment of the first source, from its first to its
            // last active element, then the second source's elements from
            // its lowest.
            PredicatedPermute::Splice { constructive } => {
                let (s1, s2) = if constructive { (n, (n + 1) % 32) } else { (d, n) };
                let operand1 = self.z(s1, vl);
                let operand2 = self.z(s2, vl);
                let elements = es.elements(vl);
                let mut result = Zv::ZERO;
                let mut x = 0;
                if let Some(last) = last_active(&mask, es, vl) {
                    let first = (0..=last).find(|&e| mask.active(e, es)).unwrap_or(last);
                    for e in first..=last {
                        result.set(x, es, operand1.get(e, es));
                        x += 1;
                    }
                }
                for e in 0..elements - x {
                    result.set(x + e, es, operand2.get(e, es));
                }
                self.set_z(d, vl, &result);
            }
            // The doublewords of each active quadword swapped.
            PredicatedPermute::Revd => {
                let operand = self.z(n, vl);
                let mut result = self.z(d, vl);
                for q in 0..vl.bytes() / 16 {
                    if mask.bit(q * 16) {
                        copy_bytes(&mut result, q * 16, &operand, q * 16 + 8, 8);
                        copy_bytes(&mut result, q * 16 + 8, &operand, q * 16, 8);
                    }
                }
                self.set_z(d, vl, &result);
            }
        }
        Ok(Flow::Next)
    }

    /// SEL (vectors): `00000101 size 1 Zm 11 Pv Zn Zd` — the active elements
    /// of `Z<n>`, the rest of `Z<m>`.
    pub(crate) fn exec_sve_sel(&mut self, insn: u32) -> ExecResult {
        let es = Esize::from_size(bits(insn, 23, 22));
        let (m, v, n, d) = (bits(insn, 20, 16), bits(insn, 13, 10), bits(insn, 9, 5), bits(insn, 4, 0));
        let vl = self.sve_check()?;
        let mask = self.p(v, vl);
        let operand1 = self.z(n, vl);
        let mut result = self.z(m, vl);
        for e in 0..es.elements(vl) {
            if mask.active(e, es) {
                result.set(e, es, operand1.get(e, es));
            }
        }
        self.set_z(d, vl, &result);
        Ok(Flow::Next)
    }
}
