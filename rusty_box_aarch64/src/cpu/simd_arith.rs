//! Advanced SIMD integer arithmetic (Arm ARM C3.5, C7.2), vector and scalar.
//!
//! Each handler serves a vector encoding class (`0 Q U 01110 …`) and its
//! scalar twin (`01 U 11110 …`). A scalar operation is the vector operation
//! on one element, written zero-extended to 128 bits; building every result
//! up from zero gives that, and the zeroed upper half of a 64-bit vector
//! result, for free.
//!
//! Intermediate values are exact `i128` integers, standing in for the
//! pseudocode's unbounded `integer`: every element is at most 64 bits wide,
//! and no operation here produces more than 127 significant bits. Results
//! are then truncated, or clamped by `SatQ`, which sets `FPSR.QC` when it
//! clamps.
//!
//! The floating-point opcodes of these classes are handed to `simd_fp`.
//! The later integer extensions are here too: SQRDMLAH and SQRDMLSH
//! (FEAT_RDM), the dot products (FEAT_DotProd, FEAT_I8MM) and the 8-bit
//! matrix multiplies (FEAT_I8MM), from the Arm A64 ISA XML pages of each.

use super::crypto::polynomial_mult;
use super::dp::ones;
use super::exec::{bit, bits, sext, ExecResult, Flow};
use super::features::Feature;
use super::fp::FpType;
use super::simd::{at_datasize, elem, set_elem};
use super::Cpu;

/// `FPSR.QC`: set by a saturating instruction whose result saturated.
pub(crate) const FPSR_QC: u64 = 1 << 27;

#[inline]
const fn datasize(q: bool) -> u32 {
    if q {
        128
    } else {
        64
    }
}

/// An element (already masked to its size by [`elem`]) as a signed integer.
#[inline]
fn sint(v: u64, esize: u32) -> i128 {
    i128::from(sext(v, esize) as i64)
}

/// An element (already masked to its size by [`elem`]) as an unsigned
/// integer.
#[inline]
fn uint(v: u64) -> i128 {
    i128::from(v)
}

/// Arm ARM `Int(x, unsigned)`, of an element already masked to `esize`
/// bits.
#[inline]
pub(crate) fn int(v: u64, esize: u32, unsigned: bool) -> i128 {
    if unsigned {
        uint(v)
    } else {
        sint(v, esize)
    }
}

/// The low `esize` bits of an exact result.
#[inline]
fn low(value: i128, esize: u32) -> u64 {
    (value as u64) & ones(esize)
}

/// Arm ARM `SatQ(value, esize, unsigned)`; records a clamp in `qc`.
pub(crate) fn sat(value: i128, esize: u32, unsigned: bool, qc: &mut bool) -> u64 {
    let (min, max) = if unsigned {
        (0, (1i128 << esize) - 1)
    } else {
        (-(1i128 << (esize - 1)), (1i128 << (esize - 1)) - 1)
    };
    let clamped = if value > max {
        *qc = true;
        max
    } else if value < min {
        *qc = true;
        min
    } else {
        value
    };
    low(clamped, esize)
}

#[inline]
const fn all_if(condition: bool) -> u64 {
    if condition {
        u64::MAX
    } else {
        0
    }
}

/// SQDMULH and SQRDMULH before their saturation: `(2 * a * b + rounding)
/// >> esize`, the rounding `2^(esize-1)` with `rounding`. Computed as `(a
/// * b + rounding / 2) >> (esize - 1)`, the same integer, which stays in
/// `i128` for the 64-bit elements of SVE, where `2 * a * b` need not.
pub(crate) fn doubling_mul_high(a: i128, b: i128, esize: u32, rounding: bool) -> i128 {
    let half_rounding = if rounding { 1i128 << (esize - 2) } else { 0 };
    (a * b + half_rounding) >> (esize - 1)
}

/// SQRDMLAH, SQRDMLSH (`subtract`) and SQRDCMLAH before their
/// saturation: `((acc << esize) +/- 2 * a * b + 2^(esize-1)) >> esize`.
/// Computed as `acc + ((+/-a * b + 2^(esize-2)) >> (esize - 1))`, the same
/// integer, which stays in `i128` for the 64-bit elements of SVE.
pub(crate) fn rounding_doubling_mul_add(
    acc: i128,
    a: i128,
    b: i128,
    esize: u32,
    subtract: bool,
) -> i128 {
    let product = if subtract { -(a * b) } else { a * b };
    acc + ((product + (1i128 << (esize - 2))) >> (esize - 1))
}

/// SQRDMLAH and SQRDMLSH on one lane (FEAT_RDM): `element3 << esize`
/// plus, or with `subtract` minus, twice the product, shifted right by
/// `esize` with rounding (Arm `RShr`) and saturated (`SignedSatQ`).
fn rdm_lane(a: u64, b: u64, d: u64, esize: u32, subtract: bool, qc: &mut bool) -> u64 {
    let value = rounding_doubling_mul_add(
        sint(d, esize),
        sint(a, esize),
        sint(b, esize),
        esize,
        subtract,
    );
    sat(value, esize, false, qc)
}

/// A register's bytes as one operand of a dot product or an 8-bit matrix
/// multiply, each read signed or, with `unsigned`, unsigned.
#[derive(Clone, Copy)]
struct Bytes {
    v: u128,
    unsigned: bool,
}

/// The sum of the `count` products of the bytes of `a` from byte `a_first`
/// with those of `b` from byte `b_first`, each byte Arm's `Int(byte,
/// unsigned)`: the inner loop of the dot products and of `MatMulAdd`.
fn byte_products(a: Bytes, a_first: u32, b: Bytes, b_first: u32, count: u32) -> i128 {
    (0..count)
        .map(|k| {
            int(elem(a.v, a_first + k, 8), 8, a.unsigned)
                * int(elem(b.v, b_first + k, 8), 8, b.unsigned)
        })
        .sum()
}

/// The shift of a shift by register: `element`, an `esize`-bit value,
/// shifted left by `shift` when it is positive, right (an arithmetic shift
/// of the exact value) when negative, with `rounding` adding half of the
/// last bit shifted out. Exact, but for a left shift by `esize` or more,
/// which gives a value whose low `esize` bits are zero and which saturates
/// as the exact one does.
pub(crate) fn shift_signed(element: i128, shift: i32, esize: u32, rounding: bool) -> i128 {
    let n = shift.unsigned_abs();
    if shift >= 0 {
        if n >= esize {
            // Every bit leaves the element: the low bits are zero, and a
            // saturating shift clamps toward the element's sign.
            element.signum() << 100
        } else {
            element << n
        }
    } else if n > 64 {
        // |element| < 2^64 < 2^(n-1): the rounded quotient is 0, and the
        // truncated one is 0 or, for a negative element, -1.
        if !rounding && element < 0 {
            -1
        } else {
            0
        }
    } else {
        let round = if rounding { 1i128 << (n - 1) } else { 0 };
        (element + round) >> n
    }
}

/// The shift of SSHL/USHL/SRSHL/URSHL/SQSHL/UQSHL/SQRSHL/UQRSHL (register):
/// `element` shifted by the signed low byte of `amount` ([`shift_signed`]),
/// then truncated, or saturated when `saturate` gives the result's
/// signedness.
fn shift_by_register(
    element: i128,
    amount: u64,
    esize: u32,
    rounding: bool,
    saturate: Option<bool>,
    qc: &mut bool,
) -> u64 {
    let value = shift_signed(element, i32::from(amount as u8 as i8), esize, rounding);
    match saturate {
        Some(unsigned) => sat(value, esize, unsigned, qc),
        None => low(value, esize),
    }
}

/// Arm ARM `CountLeadingZeroBits` of an `width`-bit value.
#[inline]
fn clz(value: u64, width: u32) -> u64 {
    if value == 0 {
        u64::from(width)
    } else {
        u64::from(value.leading_zeros() - (64 - width))
    }
}

/// Place a 64-bit narrowed result: the low half of the register with the
/// high half zeroed, or for the `2` forms (`Q = 1`) the high half with the
/// low half kept.
#[inline]
pub(super) fn narrow_into(d: u128, part: u128, upper: bool) -> u128 {
    if upper {
        (d & u128::from(u64::MAX)) | (part << 64)
    } else {
        part
    }
}

/// One lane of a same-width operation: `(n, m, d, esize, qc) -> result`.
type LaneOp = fn(u64, u64, u64, u32, &mut bool) -> u64;

/// The widening operations of the three-different and by-element classes.
#[derive(Clone, Copy)]
enum LongOp {
    Add,
    Sub,
    AbsDiff,
    AbsDiffAcc,
    Mul,
    MulAcc,
    MulSub,
    SatDoubleMul,
    SatDoubleMulAcc,
    SatDoubleMulSub,
    PolyMul,
}

/// One lane of a widening operation: `x` and `y` are the extended source
/// elements, `acc` the destination's `wide`-bit element.
fn long_lane(op: LongOp, x: i128, y: i128, acc: u64, wide: u32, qc: &mut bool) -> u64 {
    match op {
        LongOp::Add => low(x + y, wide),
        LongOp::Sub => low(x - y, wide),
        LongOp::AbsDiff => low((x - y).abs(), wide),
        LongOp::AbsDiffAcc => acc.wrapping_add(low((x - y).abs(), wide)),
        LongOp::Mul => low(x * y, wide),
        LongOp::MulAcc => acc.wrapping_add(low(x * y, wide)),
        LongOp::MulSub => acc.wrapping_sub(low(x * y, wide)),
        LongOp::SatDoubleMul => sat(2 * x * y, wide, false, qc),
        LongOp::SatDoubleMulAcc | LongOp::SatDoubleMulSub => {
            // SQDMLAL/SQDMLSL saturate twice: the doubled product, then the
            // accumulation.
            let product = sint(sat(2 * x * y, wide, false, qc), wide);
            let acc = sint(acc, wide);
            let sum = if matches!(op, LongOp::SatDoubleMulAcc) {
                acc + product
            } else {
                acc - product
            };
            sat(sum, wide, false, qc)
        }
        LongOp::PolyMul => polynomial_mult(low(x, 8), low(y, 8), 8) as u64,
    }
}

impl Cpu {
    #[inline]
    fn note_saturation(&mut self, saturated: bool) {
        if saturated {
            self.fpsr |= FPSR_QC;
        }
    }

    /// Three registers of the same type, vector (`0 Q U 01110 size 1 Rm
    /// opcode 1 Rn Rd`) and scalar (`01 U 11110 size 1 Rm opcode 1 Rn Rd`).
    pub(crate) fn exec_simd_same(&mut self, pc: u64, insn: u32, scalar: bool) -> ExecResult {
        let q = bit(insn, 30);
        let u = bit(insn, 29);
        let size = bits(insn, 23, 22);
        let opcode = bits(insn, 15, 11);
        let n = self.v[bits(insn, 9, 5) as usize];
        let m = self.v[bits(insn, 20, 16) as usize];
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];

        if opcode >= 0b11000 {
            return self.exec_simd_same_fp(pc, insn, scalar, FpType::from_sz(bit(insn, 22)));
        }
        if opcode == 0b00011 {
            // AND, BIC, ORR, ORN, EOR, BSL, BIT, BIF.
            if scalar {
                return self.undefined(pc, insn);
            }
            self.simd_check()?;
            let r = match (u, size) {
                (false, 0) => n & m,
                (false, 1) => n & !m,
                (false, 2) => n | m,
                (false, _) => n | !m,
                (true, 0) => n ^ m,
                (true, 1) => (d & n) | (!d & m),
                (true, 2) => (d & !m) | (n & m),
                (true, _) => (d & m) | (n & !m),
            };
            self.write_v(rd, at_datasize(r, q));
            return Ok(Flow::Next);
        }

        let allowed = if scalar {
            match opcode {
                0b00001 | 0b00101 | 0b01001 | 0b01011 => true,
                0b00110 | 0b00111 | 0b01000 | 0b01010 | 0b10000 | 0b10001 => size == 3,
                0b10110 => size == 1 || size == 2,
                _ => false,
            }
        } else if size == 3 && !q {
            false
        } else {
            match opcode {
                0b00001
                | 0b00101
                | 0b00110
                | 0b00111
                | 0b01000..=0b01011
                | 0b10000
                | 0b10001
                | 0b10111 => true,
                0b10011 if u => size == 0,
                0b10110 => size == 1 || size == 2,
                _ => size != 3,
            }
        };
        if !allowed {
            return self.undefined(pc, insn);
        }

        let op: LaneOp = match (opcode, u) {
            // SHADD, UHADD, SRHADD, URHADD, SHSUB, UHSUB
            (0b00000, false) => |a, b, _, e, _| low((sint(a, e) + sint(b, e)) >> 1, e),
            (0b00000, true) => |a, b, _, e, _| low((uint(a) + uint(b)) >> 1, e),
            (0b00010, false) => |a, b, _, e, _| low((sint(a, e) + sint(b, e) + 1) >> 1, e),
            (0b00010, true) => |a, b, _, e, _| low((uint(a) + uint(b) + 1) >> 1, e),
            (0b00100, false) => |a, b, _, e, _| low((sint(a, e) - sint(b, e)) >> 1, e),
            (0b00100, true) => |a, b, _, e, _| low((uint(a) - uint(b)) >> 1, e),
            // SQADD, UQADD, SQSUB, UQSUB
            (0b00001, false) => |a, b, _, e, qc| sat(sint(a, e) + sint(b, e), e, false, qc),
            (0b00001, true) => |a, b, _, e, qc| sat(uint(a) + uint(b), e, true, qc),
            (0b00101, false) => |a, b, _, e, qc| sat(sint(a, e) - sint(b, e), e, false, qc),
            (0b00101, true) => |a, b, _, e, qc| sat(uint(a) - uint(b), e, true, qc),
            // CMGT, CMHI, CMGE, CMHS
            (0b00110, false) => |a, b, _, e, _| all_if(sint(a, e) > sint(b, e)),
            (0b00110, true) => |a, b, _, _, _| all_if(a > b),
            (0b00111, false) => |a, b, _, e, _| all_if(sint(a, e) >= sint(b, e)),
            (0b00111, true) => |a, b, _, _, _| all_if(a >= b),
            // SSHL, USHL, SQSHL, UQSHL, SRSHL, URSHL, SQRSHL, UQRSHL
            (0b01000, false) => {
                |a, b, _, e, qc| shift_by_register(sint(a, e), b, e, false, None, qc)
            }
            (0b01000, true) => |a, b, _, e, qc| shift_by_register(uint(a), b, e, false, None, qc),
            (0b01001, false) => {
                |a, b, _, e, qc| shift_by_register(sint(a, e), b, e, false, Some(false), qc)
            }
            (0b01001, true) => {
                |a, b, _, e, qc| shift_by_register(uint(a), b, e, false, Some(true), qc)
            }
            (0b01010, false) => {
                |a, b, _, e, qc| shift_by_register(sint(a, e), b, e, true, None, qc)
            }
            (0b01010, true) => |a, b, _, e, qc| shift_by_register(uint(a), b, e, true, None, qc),
            (0b01011, false) => {
                |a, b, _, e, qc| shift_by_register(sint(a, e), b, e, true, Some(false), qc)
            }
            (0b01011, true) => {
                |a, b, _, e, qc| shift_by_register(uint(a), b, e, true, Some(true), qc)
            }
            // SMAX, UMAX, SMIN, UMIN and their pairwise forms
            (0b01100, false) | (0b10100, false) => {
                |a, b, _, e, _| if sint(a, e) >= sint(b, e) { a } else { b }
            }
            (0b01100, true) | (0b10100, true) => |a, b, _, _, _| a.max(b),
            (0b01101, false) | (0b10101, false) => {
                |a, b, _, e, _| if sint(a, e) <= sint(b, e) { a } else { b }
            }
            (0b01101, true) | (0b10101, true) => |a, b, _, _, _| a.min(b),
            // SABD, UABD, SABA, UABA
            (0b01110, false) => |a, b, _, e, _| low((sint(a, e) - sint(b, e)).abs(), e),
            (0b01110, true) => |a, b, _, e, _| low((uint(a) - uint(b)).abs(), e),
            (0b01111, false) => {
                |a, b, d, e, _| d.wrapping_add(low((sint(a, e) - sint(b, e)).abs(), e))
            }
            (0b01111, true) => |a, b, d, e, _| d.wrapping_add(low((uint(a) - uint(b)).abs(), e)),
            // ADD, SUB, CMTST, CMEQ
            (0b10000, false) => |a, b, _, _, _| a.wrapping_add(b),
            (0b10000, true) => |a, b, _, _, _| a.wrapping_sub(b),
            (0b10001, false) => |a, b, _, _, _| all_if(a & b != 0),
            (0b10001, true) => |a, b, _, _, _| all_if(a == b),
            // MLA, MLS, MUL, PMUL
            (0b10010, false) => |a, b, d, _, _| d.wrapping_add(a.wrapping_mul(b)),
            (0b10010, true) => |a, b, d, _, _| d.wrapping_sub(a.wrapping_mul(b)),
            (0b10011, false) => |a, b, _, _, _| a.wrapping_mul(b),
            (0b10011, true) => |a, b, _, _, _| polynomial_mult(a, b, 8) as u64,
            // SQDMULH, SQRDMULH
            (0b10110, false) => |a, b, _, e, qc| {
                sat(
                    doubling_mul_high(sint(a, e), sint(b, e), e, false),
                    e,
                    false,
                    qc,
                )
            },
            (0b10110, true) => |a, b, _, e, qc| {
                sat(
                    doubling_mul_high(sint(a, e), sint(b, e), e, true),
                    e,
                    false,
                    qc,
                )
            },
            // ADDP
            (0b10111, false) => |a, b, _, _, _| a.wrapping_add(b),
            // Every other opcode below 11000 except 00011 is listed above
            // for both U values; ADDP has no U = 1 form.
            _ => return self.undefined(pc, insn),
        };
        self.simd_check()?;
        let esize = 8u32 << size;
        let lanes = if scalar { 1 } else { datasize(q) / esize };
        let pairwise = !scalar && matches!(opcode, 0b10100 | 0b10101 | 0b10111);
        let mut qc = false;
        let mut r = 0u128;
        for e in 0..lanes {
            let (a, b) = if pairwise {
                // concat = Vm:Vn; result[e] = op(concat[2e], concat[2e+1]).
                let pick = |i: u32| {
                    if i < lanes {
                        elem(n, i, esize)
                    } else {
                        elem(m, i - lanes, esize)
                    }
                };
                (pick(2 * e), pick(2 * e + 1))
            } else {
                (elem(n, e, esize), elem(m, e, esize))
            };
            set_elem(
                &mut r,
                e,
                esize,
                op(a, b, elem(d, e, esize), esize, &mut qc),
            );
        }
        self.note_saturation(qc);
        self.write_v(rd, r);
        Ok(Flow::Next)
    }

    /// Two-register miscellaneous, vector (`0 Q U 01110 size 10000 opcode 10
    /// Rn Rd`) and scalar (`01 U 11110 size 10000 opcode 10 Rn Rd`).
    pub(crate) fn exec_simd_misc(&mut self, pc: u64, insn: u32, scalar: bool) -> ExecResult {
        let q = bit(insn, 30);
        let u = bit(insn, 29);
        let size = bits(insn, 23, 22);
        let opcode = bits(insn, 16, 12);
        let n = self.v[bits(insn, 9, 5) as usize];
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let esize = 8u32 << size;
        let lanes = if scalar { 1 } else { datasize(q) / esize };

        if opcode >= 0b10110 || ((0b01100..=0b01111).contains(&opcode) && size >= 2) {
            return self.exec_simd_misc_fp(pc, insn, scalar, FpType::from_sz(bit(insn, 22)));
        }

        let mut qc = false;
        let mut r = 0u128;
        match (opcode, u) {
            // REV64, REV32, REV16: reverse elements within containers.
            (0b00000, _) | (0b00001, false) if !scalar => {
                let container = match (opcode, u) {
                    (0b00000, false) => 64,
                    (0b00000, true) => 32,
                    _ => 16,
                };
                if esize >= container {
                    return self.undefined(pc, insn);
                }
                self.simd_check()?;
                let per = container / esize;
                for e in 0..lanes {
                    let src = e - e % per + (per - 1 - e % per);
                    set_elem(&mut r, e, esize, elem(n, src, esize));
                }
            }
            // SADDLP, UADDLP, SADALP, UADALP: add adjacent pairs into
            // double-width lanes, optionally accumulating.
            (0b00010, _) | (0b00110, _) if !scalar => {
                if size == 3 {
                    return self.undefined(pc, insn);
                }
                self.simd_check()?;
                let wide = 2 * esize;
                for e in 0..datasize(q) / wide {
                    let sum = int(elem(n, 2 * e, esize), esize, u)
                        + int(elem(n, 2 * e + 1, esize), esize, u);
                    let acc = if opcode == 0b00110 {
                        elem(d, e, wide)
                    } else {
                        0
                    };
                    set_elem(&mut r, e, wide, acc.wrapping_add(low(sum, wide)));
                }
            }
            // SUQADD (signed Vd + unsigned Vn), USQADD (unsigned Vd + signed
            // Vn).
            (0b00011, _) => {
                if !scalar && size == 3 && !q {
                    return self.undefined(pc, insn);
                }
                self.simd_check()?;
                for e in 0..lanes {
                    let (a, acc) = (elem(n, e, esize), elem(d, e, esize));
                    let v = if u {
                        sat(uint(acc) + sint(a, esize), esize, true, &mut qc)
                    } else {
                        sat(sint(acc, esize) + uint(a), esize, false, &mut qc)
                    };
                    set_elem(&mut r, e, esize, v);
                }
            }
            // CLS, CLZ
            (0b00100, _) if !scalar => {
                if size == 3 {
                    return self.undefined(pc, insn);
                }
                self.simd_check()?;
                for e in 0..lanes {
                    let a = elem(n, e, esize);
                    let v = if u {
                        clz(a, esize)
                    } else {
                        // CountLeadingSignBits: CLZ(x<N-1:1> EOR x<N-2:0>).
                        clz((a >> 1) ^ (a & ones(esize - 1)), esize - 1)
                    };
                    set_elem(&mut r, e, esize, v);
                }
            }
            // CNT, NOT, RBIT
            (0b00101, false) if !scalar && size == 0 => {
                self.simd_check()?;
                for e in 0..lanes {
                    set_elem(&mut r, e, 8, u64::from(elem(n, e, 8).count_ones()));
                }
            }
            (0b00101, true) if !scalar && size == 0 => {
                self.simd_check()?;
                r = at_datasize(!n, q);
            }
            (0b00101, true) if !scalar && size == 1 => {
                self.simd_check()?;
                for e in 0..datasize(q) / 8 {
                    set_elem(
                        &mut r,
                        e,
                        8,
                        u64::from((elem(n, e, 8) as u8).reverse_bits()),
                    );
                }
            }
            // SQABS, SQNEG
            (0b00111, _) => {
                if !scalar && size == 3 && !q {
                    return self.undefined(pc, insn);
                }
                self.simd_check()?;
                for e in 0..lanes {
                    let a = sint(elem(n, e, esize), esize);
                    let v = if u { -a } else { a.abs() };
                    set_elem(&mut r, e, esize, sat(v, esize, false, &mut qc));
                }
            }
            // CMGT, CMGE, CMEQ, CMLE, CMLT (zero), ABS, NEG
            (0b01000, _) | (0b01001, _) | (0b01010, false) | (0b01011, _) => {
                if (scalar && size != 3) || (!scalar && size == 3 && !q) {
                    return self.undefined(pc, insn);
                }
                self.simd_check()?;
                for e in 0..lanes {
                    let a = elem(n, e, esize);
                    let s = sint(a, esize);
                    let v = match (opcode, u) {
                        (0b01000, false) => all_if(s > 0),
                        (0b01000, true) => all_if(s >= 0),
                        (0b01001, false) => all_if(s == 0),
                        (0b01001, true) => all_if(s <= 0),
                        (0b01010, _) => all_if(s < 0),
                        (_, false) => low(s.abs(), esize),
                        (_, true) => low(-s, esize),
                    };
                    set_elem(&mut r, e, esize, v);
                }
            }
            // XTN, SQXTUN, SQXTN, UQXTN: narrow each double-width element.
            (0b10010, _) | (0b10100, _) => {
                if size == 3 || (scalar && opcode == 0b10010 && !u) {
                    return self.undefined(pc, insn);
                }
                self.simd_check()?;
                let wide = 2 * esize;
                let count = if scalar { 1 } else { 64 / esize };
                let mut part = 0u128;
                for e in 0..count {
                    let a = elem(n, e, wide);
                    let v = match (opcode, u) {
                        (0b10010, false) => a,
                        (0b10010, true) => sat(sint(a, wide), esize, true, &mut qc),
                        (_, false) => sat(sint(a, wide), esize, false, &mut qc),
                        (_, true) => sat(uint(a), esize, true, &mut qc),
                    };
                    set_elem(&mut part, e, esize, v);
                }
                r = narrow_into(d, part, !scalar && q);
            }
            // SHLL, SHLL2: widen half of Vn, shifted left by the element size.
            (0b10011, true) if !scalar => {
                if size == 3 {
                    return self.undefined(pc, insn);
                }
                self.simd_check()?;
                let half = if q { n >> 64 } else { n };
                for e in 0..64 / esize {
                    set_elem(&mut r, e, 2 * esize, elem(half, e, esize) << esize);
                }
            }
            _ => return self.undefined(pc, insn),
        }
        self.note_saturation(qc);
        self.write_v(rd, r);
        Ok(Flow::Next)
    }

    /// Shift by immediate, vector (`0 Q U 011110 immh immb opcode 1 Rn Rd`)
    /// and scalar (`01 U 111110 immh immb opcode 1 Rn Rd`).
    pub(crate) fn exec_simd_shift_imm(&mut self, pc: u64, insn: u32, scalar: bool) -> ExecResult {
        let q = bit(insn, 30);
        let u = bit(insn, 29);
        let immh = bits(insn, 22, 19);
        let immhb = bits(insn, 22, 16);
        let opcode = bits(insn, 15, 11);
        if immh == 0 {
            return self.undefined(pc, insn);
        }
        if matches!(opcode, 0b11100 | 0b11111) {
            return self.exec_simd_shift_fixed(pc, insn, scalar);
        }
        let esize = 8u32 << (31 - immh.leading_zeros());
        let n = self.v[bits(insn, 9, 5) as usize];
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];

        let narrowing = (0b10000..=0b10011).contains(&opcode);
        let reserved = if narrowing || opcode == 0b10100 {
            immh & 8 != 0
                || (scalar && opcode == 0b10100)
                || (scalar && !u && (opcode == 0b10000 || opcode == 0b10001))
        } else if scalar {
            // Below 64 bits only the saturating left shifts have scalar forms.
            immh & 8 == 0 && !matches!(opcode, 0b01100 | 0b01110)
        } else {
            immh & 8 != 0 && !q
        };
        if reserved {
            return self.undefined(pc, insn);
        }
        // For the narrowing and widening forms `esize` is the narrow size.
        let right = 2 * esize - immhb;
        let left = immhb - esize;

        let mut qc = false;
        let mut r = 0u128;
        if narrowing {
            // SHRN, RSHRN, SQSHRUN, SQRSHRUN, SQSHRN, UQSHRN, SQRSHRN, UQRSHRN
            let rounding = opcode & 1 == 1;
            let (source_unsigned, saturate) = match (opcode, u) {
                (0b10000 | 0b10001, false) => (true, None),
                (0b10000 | 0b10001, true) => (false, Some(true)),
                (_, unsigned) => (unsigned, Some(unsigned)),
            };
            self.simd_check()?;
            let wide = 2 * esize;
            let round = if rounding { 1i128 << (right - 1) } else { 0 };
            let count = if scalar { 1 } else { 64 / esize };
            let mut part = 0u128;
            for e in 0..count {
                let value = (int(elem(n, e, wide), wide, source_unsigned) + round) >> right;
                let v = match saturate {
                    Some(unsigned) => sat(value, esize, unsigned, &mut qc),
                    None => low(value, esize),
                };
                set_elem(&mut part, e, esize, v);
            }
            r = narrow_into(d, part, !scalar && q);
        } else if opcode == 0b10100 {
            // SSHLL, USHLL (and the SXTL, UXTL aliases): widen half of Vn.
            self.simd_check()?;
            let half = if q { n >> 64 } else { n };
            for e in 0..64 / esize {
                let wide = low(int(elem(half, e, esize), esize, u) << left, 2 * esize);
                set_elem(&mut r, e, 2 * esize, wide);
            }
        } else {
            #[derive(Clone, Copy)]
            enum Shift {
                /// SSHR, USHR, SRSHR, URSHR, and with `accumulate` SSRA,
                /// USRA, SRSRA, URSRA.
                Right { accumulate: bool },
                /// SRI.
                InsertRight,
                /// SHL.
                Left,
                /// SLI.
                InsertLeft,
                /// SQSHLU.
                SaturateLeftUnsigned,
                /// SQSHL, UQSHL (immediate).
                SaturateLeft,
            }
            let shift = match (opcode, u) {
                (0b00000 | 0b00100, _) => Shift::Right { accumulate: false },
                (0b00010 | 0b00110, _) => Shift::Right { accumulate: true },
                (0b01000, true) => Shift::InsertRight,
                (0b01010, false) => Shift::Left,
                (0b01010, true) => Shift::InsertLeft,
                (0b01100, true) => Shift::SaturateLeftUnsigned,
                (0b01110, _) => Shift::SaturateLeft,
                _ => return self.undefined(pc, insn),
            };
            self.simd_check()?;
            let lanes = if scalar { 1 } else { datasize(q) / esize };
            // The rounding forms are the ones with opcode<2> set.
            let round = if opcode & 0b100 != 0 {
                1i128 << (right - 1)
            } else {
                0
            };
            for e in 0..lanes {
                let a = elem(n, e, esize);
                let acc = elem(d, e, esize);
                let v = match shift {
                    Shift::Right { accumulate } => {
                        let shifted = low((int(a, esize, u) + round) >> right, esize);
                        if accumulate {
                            acc.wrapping_add(shifted)
                        } else {
                            shifted
                        }
                    }
                    // Keep the destination's top `right` bits.
                    Shift::InsertRight => {
                        if right >= esize {
                            acc
                        } else {
                            (a >> right) | (acc & !(ones(esize) >> right) & ones(esize))
                        }
                    }
                    Shift::Left => a << left,
                    // Keep the destination's low `left` bits.
                    Shift::InsertLeft => (a << left) | (acc & ones(left)),
                    Shift::SaturateLeftUnsigned => {
                        sat(sint(a, esize) << left, esize, true, &mut qc)
                    }
                    Shift::SaturateLeft => sat(int(a, esize, u) << left, esize, u, &mut qc),
                };
                set_elem(&mut r, e, esize, v);
            }
        }
        self.note_saturation(qc);
        self.write_v(rd, r);
        Ok(Flow::Next)
    }

    /// Three registers of different widths, vector (`0 Q U 01110 size 1 Rm
    /// opcode 00 Rn Rd`) and scalar (`01 U 11110 size 1 Rm opcode 00 Rn Rd`).
    /// `size` names the narrow element; `Q = 1` selects the `2` forms, which
    /// read (or for the narrowing forms write) the high halves.
    pub(crate) fn exec_simd_diff(&mut self, pc: u64, insn: u32, scalar: bool) -> ExecResult {
        let q = bit(insn, 30);
        let u = bit(insn, 29);
        let size = bits(insn, 23, 22);
        let opcode = bits(insn, 15, 12);
        let n = self.v[bits(insn, 9, 5) as usize];
        let m = self.v[bits(insn, 20, 16) as usize];
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let saturating = matches!(opcode, 0b1001 | 0b1011 | 0b1101);
        // PMULL and PMULL2 (Arm ARM PMULL, PMULL2): size 0 multiplies bytes
        // into halfwords, size 3 one doubleword into a quadword, UNDEFINED
        // without FEAT_PMULL (`pmull_advsimd.xml`), and sizes 1 and 2 are
        // reserved. Size 3 is reserved for every other opcode.
        let pmull_64 = opcode == 0b1110 && !u && !scalar && size == 3;
        let reserved = (size == 3 && !pmull_64)
            || (pmull_64 && !self.implements(Feature::Pmull))
            || (scalar && !(saturating && !u))
            || (saturating && (u || size == 0))
            || (opcode == 0b1110 && (u || matches!(size, 1 | 2)));
        if reserved {
            return self.undefined(pc, insn);
        }
        if pmull_64 {
            self.simd_check()?;
            let half = u32::from(q);
            self.write_v(
                rd,
                polynomial_mult(elem(n, half, 64), elem(m, half, 64), 64),
            );
            return Ok(Flow::Next);
        }
        // ADDHN, RADDHN, SUBHN, RSUBHN narrow; everything else widens.
        let narrowing = opcode == 0b0100 || opcode == 0b0110;
        let op = match opcode {
            0b0000 | 0b0001 => LongOp::Add,
            0b0010 | 0b0011 => LongOp::Sub,
            0b0101 => LongOp::AbsDiffAcc,
            0b0111 => LongOp::AbsDiff,
            0b1000 => LongOp::MulAcc,
            0b1001 => LongOp::SatDoubleMulAcc,
            0b1010 => LongOp::MulSub,
            0b1011 => LongOp::SatDoubleMulSub,
            0b1100 => LongOp::Mul,
            0b1101 => LongOp::SatDoubleMul,
            0b1110 => LongOp::PolyMul,
            // The narrowing opcodes use `op` only for their sign: Add or Sub.
            0b0100 => LongOp::Add,
            0b0110 => LongOp::Sub,
            _ => return self.undefined(pc, insn),
        };
        self.simd_check()?;
        let esize = 8u32 << size;
        let wide = 2 * esize;
        let upper = !scalar && q;
        let mut qc = false;

        if narrowing {
            // ADDHN, RADDHN, SUBHN, RSUBHN: the high half of each wide sum or
            // difference.
            let round = if u { 1i128 << (esize - 1) } else { 0 };
            let mut part = 0u128;
            for e in 0..64 / esize {
                let (a, b) = (uint(elem(n, e, wide)), uint(elem(m, e, wide)));
                let value = if matches!(op, LongOp::Add) {
                    a + b
                } else {
                    a - b
                };
                set_elem(&mut part, e, esize, low((value + round) >> esize, esize));
            }
            self.write_v(rd, narrow_into(d, part, upper));
            return Ok(Flow::Next);
        }

        // SADDW, UADDW, SSUBW, USUBW take Vn at the wide size.
        let wide_n = matches!(opcode, 0b0001 | 0b0011);
        let half = |v: u128| if upper { v >> 64 } else { v };
        let (hn, hm) = (half(n), half(m));
        let count = if scalar { 1 } else { 64 / esize };
        let mut r = 0u128;
        for e in 0..count {
            let x = if wide_n {
                int(elem(n, e, wide), wide, u)
            } else {
                int(elem(hn, e, esize), esize, u)
            };
            let y = int(elem(hm, e, esize), esize, u);
            set_elem(
                &mut r,
                e,
                wide,
                long_lane(op, x, y, elem(d, e, wide), wide, &mut qc),
            );
        }
        self.note_saturation(qc);
        self.write_v(rd, r);
        Ok(Flow::Next)
    }

    /// Vector by element, vector (`0 Q U 01111 size L M Rm opcode H 0 Rn
    /// Rd`) and scalar (`01 U 11111 size L M Rm opcode H 0 Rn Rd`).
    pub(crate) fn exec_simd_elem(&mut self, pc: u64, insn: u32, scalar: bool) -> ExecResult {
        let q = bit(insn, 30);
        let u = bit(insn, 29);
        let size = bits(insn, 23, 22);
        let opcode = bits(insn, 15, 12);
        if matches!(opcode, 0b0001 | 0b0101 | 0b1001) {
            return self.exec_simd_elem_fp(pc, insn, scalar);
        }
        let (h, l, mbit) = (bits(insn, 11, 11), bits(insn, 21, 21), bits(insn, 20, 20));
        let (index, rm) = match size {
            1 => ((h << 2) | (l << 1) | mbit, bits(insn, 19, 16)),
            2 => ((h << 1) | l, bits(insn, 20, 16)),
            _ => return self.undefined(pc, insn),
        };
        let esize = 8u32 << size;
        let n = self.v[bits(insn, 9, 5) as usize];
        let element = elem(self.v[rm as usize], index, esize);
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];

        enum Kind {
            Same(LaneOp),
            Long(LongOp),
        }
        let kind = match (opcode, u) {
            (0b0000, true) if !scalar => {
                Kind::Same(|a, b, d, _, _| d.wrapping_add(a.wrapping_mul(b)))
            }
            (0b0100, true) if !scalar => {
                Kind::Same(|a, b, d, _, _| d.wrapping_sub(a.wrapping_mul(b)))
            }
            (0b1000, false) if !scalar => Kind::Same(|a, b, _, _, _| a.wrapping_mul(b)),
            (0b1100, false) => Kind::Same(|a, b, _, e, qc| {
                sat(
                    doubling_mul_high(sint(a, e), sint(b, e), e, false),
                    e,
                    false,
                    qc,
                )
            }),
            (0b1101, false) => Kind::Same(|a, b, _, e, qc| {
                sat(
                    doubling_mul_high(sint(a, e), sint(b, e), e, true),
                    e,
                    false,
                    qc,
                )
            }),
            (0b1101, true) => Kind::Same(|a, b, d, e, qc| rdm_lane(a, b, d, e, false, qc)),
            (0b1111, true) => Kind::Same(|a, b, d, e, qc| rdm_lane(a, b, d, e, true, qc)),
            (0b0010, _) if !scalar => Kind::Long(LongOp::MulAcc),
            (0b0110, _) if !scalar => Kind::Long(LongOp::MulSub),
            (0b1010, _) if !scalar => Kind::Long(LongOp::Mul),
            (0b0011, false) => Kind::Long(LongOp::SatDoubleMulAcc),
            (0b0111, false) => Kind::Long(LongOp::SatDoubleMulSub),
            (0b1011, false) => Kind::Long(LongOp::SatDoubleMul),
            _ => return self.undefined(pc, insn),
        };
        self.simd_check()?;
        let mut qc = false;
        let mut r = 0u128;
        match kind {
            Kind::Same(op) => {
                let lanes = if scalar { 1 } else { datasize(q) / esize };
                for e in 0..lanes {
                    set_elem(
                        &mut r,
                        e,
                        esize,
                        op(
                            elem(n, e, esize),
                            element,
                            elem(d, e, esize),
                            esize,
                            &mut qc,
                        ),
                    );
                }
            }
            Kind::Long(op) => {
                let wide = 2 * esize;
                let half = if !scalar && q { n >> 64 } else { n };
                let count = if scalar { 1 } else { 64 / esize };
                let y = int(element, esize, u);
                for e in 0..count {
                    let x = int(elem(half, e, esize), esize, u);
                    set_elem(
                        &mut r,
                        e,
                        wide,
                        long_lane(op, x, y, elem(d, e, wide), wide, &mut qc),
                    );
                }
            }
        }
        self.note_saturation(qc);
        self.write_v(rd, r);
        Ok(Flow::Next)
    }

    /// Across lanes: `0 Q U 01110 size 11000 opcode 10 Rn Rd` — ADDV,
    /// SADDLV, UADDLV, SMAXV, UMAXV, SMINV, UMINV.
    pub(crate) fn exec_simd_across(&mut self, pc: u64, insn: u32) -> ExecResult {
        let q = bit(insn, 30);
        let u = bit(insn, 29);
        let size = bits(insn, 23, 22);
        let opcode = bits(insn, 16, 12);
        if matches!(opcode, 0b01100 | 0b01111) {
            return self.exec_simd_across_fp(pc, insn);
        }
        #[derive(Clone, Copy)]
        enum Across {
            /// ADDV.
            Add,
            /// SADDLV, UADDLV.
            AddLong,
            /// SMAXV, UMAXV.
            Max,
            /// SMINV, UMINV.
            Min,
        }
        let op = match (opcode, u) {
            (0b11011, false) => Across::Add,
            (0b00011, _) => Across::AddLong,
            (0b01010, _) => Across::Max,
            (0b11010, _) => Across::Min,
            _ => return self.undefined(pc, insn),
        };
        if size == 3 || (size == 2 && !q) {
            return self.undefined(pc, insn);
        }
        self.simd_check()?;
        let n = self.v[bits(insn, 9, 5) as usize];
        let esize = 8u32 << size;
        let lanes = datasize(q) / esize;
        let first = int(elem(n, 0, esize), esize, u);
        let (mut max, mut min, mut sum) = (first, first, 0i128);
        for e in 0..lanes {
            let a = int(elem(n, e, esize), esize, u);
            max = max.max(a);
            min = min.min(a);
            sum += a;
        }
        let value = match op {
            Across::Add => low(sum, esize),
            Across::AddLong => low(sum, 2 * esize),
            Across::Max => low(max, esize),
            Across::Min => low(min, esize),
        };
        self.write_v(bits(insn, 4, 0) as usize, u128::from(value));
        Ok(Flow::Next)
    }

    /// SQRDMLAH and SQRDMLSH (FEAT_RDM), vector (`0 Q 1 01110 size 0 Rm 1
    /// 000 S 1 Rn Rd`) and scalar (`01 1 11110 size 0 Rm 1 000 S 1 Rn Rd`):
    /// 16- and 32-bit elements only.
    pub(crate) fn exec_simd_rdm(&mut self, pc: u64, insn: u32, scalar: bool) -> ExecResult {
        let q = bit(insn, 30);
        let size = bits(insn, 23, 22);
        if size == 0b00 || size == 0b11 {
            return self.undefined(pc, insn);
        }
        let subtract = bit(insn, 11);
        self.simd_check()?;
        let esize = 8u32 << size;
        let n = self.v[bits(insn, 9, 5) as usize];
        let m = self.v[bits(insn, 20, 16) as usize];
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let lanes = if scalar { 1 } else { datasize(q) / esize };
        let mut qc = false;
        let mut r = 0u128;
        for e in 0..lanes {
            let lane = rdm_lane(
                elem(n, e, esize),
                elem(m, e, esize),
                elem(d, e, esize),
                esize,
                subtract,
                &mut qc,
            );
            set_elem(&mut r, e, esize, lane);
        }
        self.note_saturation(qc);
        self.write_v(rd, r);
        Ok(Flow::Next)
    }

    /// The dot products: SDOT, UDOT (FEAT_DotProd) and USDOT, SUDOT
    /// (FEAT_I8MM), vector (`0 Q U 01110 size 0 Rm 1 001 x 1 Rn Rd`) and by
    /// element (`0 Q U 01111 size L M Rm 111 x H 0 Rn Rd`, `size` being
    /// `US 0` for SUDOT and USDOT). Each 32-bit lane of Vd adds, wrapping,
    /// the four products of the bytes of its lane of Vn with those of the
    /// same lane of Vm, or of lane `H:L` of Vm by element.
    pub(crate) fn exec_simd_dot(&mut self, pc: u64, insn: u32, by_element: bool) -> ExecResult {
        let q = bit(insn, 30);
        let u = bit(insn, 29);
        let mixed = if by_element {
            bit(insn, 12)
        } else {
            bit(insn, 11)
        };
        // Whether Vn's bytes and Vm's are unsigned. USDOT is unsigned by
        // signed, SUDOT signed by unsigned (by element, `US` says which).
        let (n_unsigned, m_unsigned) = if mixed {
            let us = !by_element || bit(insn, 23);
            (us, !us)
        } else {
            if bits(insn, 23, 22) != 0b10 {
                return self.undefined(pc, insn);
            }
            (u, u)
        };
        self.simd_check()?;
        let n = Bytes {
            v: self.v[bits(insn, 9, 5) as usize],
            unsigned: n_unsigned,
        };
        let m = Bytes {
            v: self.v[bits(insn, 20, 16) as usize],
            unsigned: m_unsigned,
        };
        // By element, every lane takes lane H:L of Vm.
        let index = by_element.then(|| (bits(insn, 11, 11) << 1) | bits(insn, 21, 21));
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let mut r = 0u128;
        for e in 0..datasize(q) / 32 {
            let m_lane = index.unwrap_or(e);
            let sum = uint(elem(d, e, 32)) + byte_products(n, 4 * e, m, 4 * m_lane, 4);
            set_elem(&mut r, e, 32, low(sum, 32));
        }
        self.write_v(rd, r);
        Ok(Flow::Next)
    }

    /// SMMLA, USMMLA, UMMLA (FEAT_I8MM): `0 1 U 01110 10 0 Rm 1 010 B 1 Rn
    /// Rd`, Arm `MatMulAdd`. Vn holds a 2x8 matrix of bytes by rows, Vm a
    /// 2x8 one whose rows are the columns of the 8x2 matrix it stands for,
    /// and Vd a 2x2 matrix of 32-bit words by rows, which the product is
    /// added to, wrapping. USMMLA's Vn is unsigned and its Vm signed.
    pub(crate) fn exec_simd_mmla(&mut self, pc: u64, insn: u32) -> ExecResult {
        let (n_unsigned, m_unsigned) = match (bit(insn, 29), bit(insn, 11)) {
            (false, false) => (false, false),
            (false, true) => (true, false),
            (true, false) => (true, true),
            // The decoder allocates no such word to this class.
            (true, true) => return self.undefined(pc, insn),
        };
        self.simd_check()?;
        let n = Bytes {
            v: self.v[bits(insn, 9, 5) as usize],
            unsigned: n_unsigned,
        };
        let m = Bytes {
            v: self.v[bits(insn, 20, 16) as usize],
            unsigned: m_unsigned,
        };
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let mut r = 0u128;
        for i in 0..2 {
            for j in 0..2 {
                let sum = uint(elem(d, 2 * i + j, 32)) + byte_products(n, 8 * i, m, 8 * j, 8);
                set_elem(&mut r, 2 * i + j, 32, low(sum, 32));
            }
        }
        self.write_v(rd, r);
        Ok(Flow::Next)
    }

    /// Scalar pairwise: `01 U 11110 size 11000 opcode 10 Rn Rd` — ADDP
    /// (scalar).
    pub(crate) fn exec_simd_scalar_pair(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bit(insn, 29) {
            return self.exec_simd_pair_fp(pc, insn, FpType::from_sz(bit(insn, 22)));
        }
        // With U = 0 every opcode but ADDP's is half precision (FEAT_FP16,
        // which the decoder checks).
        if bits(insn, 16, 12) != 0b11011 {
            return self.exec_simd_pair_fp(pc, insn, FpType::Half);
        }
        if bits(insn, 23, 22) != 3 {
            return self.undefined(pc, insn);
        }
        self.simd_check()?;
        let n = self.v[bits(insn, 9, 5) as usize];
        let sum = (n as u64).wrapping_add((n >> 64) as u64);
        self.write_v(bits(insn, 4, 0) as usize, u128::from(sum));
        Ok(Flow::Next)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/simd_extensions.rs"]
mod extension_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturation_clamps_to_the_element_range_and_says_so() {
        let mut qc = false;
        assert_eq!(sat(127, 8, false, &mut qc), 0x7F);
        assert!(!qc);
        assert_eq!(sat(128, 8, false, &mut qc), 0x7F);
        assert!(qc);
        let mut qc = false;
        assert_eq!(sat(-129, 8, false, &mut qc), 0x80);
        assert!(qc);
        let mut qc = false;
        assert_eq!(sat(-1, 64, true, &mut qc), 0);
        assert!(qc);
        let mut qc = false;
        assert_eq!(sat(i128::from(u64::MAX), 64, true, &mut qc), u64::MAX);
        assert!(!qc);
    }

    #[test]
    fn register_shifts_cover_out_of_range_amounts() {
        let mut qc = false;
        // Left by more than the element: zero, or saturated for SQSHL.
        assert_eq!(shift_by_register(1, 8, 8, false, None, &mut qc), 0);
        assert_eq!(
            shift_by_register(1, 8, 8, false, Some(false), &mut qc),
            0x7F
        );
        assert!(qc);
        let mut qc = false;
        assert_eq!(
            shift_by_register(-1, 100, 64, false, Some(false), &mut qc),
            1 << 63
        );
        assert!(qc);
        // Right by -128: all sign bits when truncating, zero when rounding.
        let mut qc = false;
        assert_eq!(
            shift_by_register(-5, 0x80, 16, false, None, &mut qc),
            0xFFFF
        );
        assert_eq!(shift_by_register(-5, 0x80, 16, true, None, &mut qc), 0);
        // Rounding right shift by one: 3 -> 2, -3 -> -1 (round half up).
        assert_eq!(shift_by_register(3, 0xFF, 8, true, None, &mut qc), 2);
        assert_eq!(shift_by_register(-3, 0xFF, 8, true, None, &mut qc), 0xFF);
        assert!(!qc);
    }

    #[test]
    fn leading_sign_bits_exclude_the_sign_bit_itself() {
        // CLS of 0x00 and 0xFF (8-bit) is 7; of 0x40 is 0.
        for (a, want) in [(0x00u64, 7u64), (0xFF, 7), (0x40, 0), (0x20, 1), (0xC0, 1)] {
            assert_eq!(clz((a >> 1) ^ (a & ones(7)), 7), want, "cls({a:#x})");
        }
    }
}
