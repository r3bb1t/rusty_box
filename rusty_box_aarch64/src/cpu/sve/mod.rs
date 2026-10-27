//! The SVE instructions FEAT_SME makes legal in Streaming SVE mode: their
//! enable check, the vector and predicate values they compute on, and the
//! shared pseudocode they call (`PredTest`, `DecodePredCount`, the
//! saturating conversions).
//!
//! This processor implements FEAT_SME and not FEAT_SVE, so an SVE
//! instruction executes only in Streaming SVE mode, at the Streaming SVE
//! vector length; outside it, `CheckSVEEnabled` takes the SME trap
//! (`CheckStreamingSVEEnabled`). The instructions are in `int` (integer
//! arithmetic, element counts and immediates), `perm` (permutes, copies and
//! selects), `pred` (compares, predicate operations, `WHILE` and predicate
//! counts), `fp` (floating point) and `mem` (contiguous loads, stores and
//! prefetches).
//!
//! Provenance (R7): the Arm A64 ISA XML, release 2026-09 (Arm ARM M.d), the
//! instruction pages each handler names and the shared pseudocode in
//! `aarch64/functions/sve`. Each handler follows its page's decode and
//! operation; where a page raises UNDEFINED in its decode, the handler
//! does so before the enable check, as the pseudocode orders it.

mod fp;
mod int;
mod mem;
mod perm;
mod pred;
#[cfg(test)]
#[path = "../../../tests/unit/sve.rs"]
mod tests;

use super::exception::Route;
use super::exec::{sext, Fault};
use super::features::VecLen;
use super::simd_arith::{int as simd_int, sat};
use super::sme::SmeTrap;
use super::{Cpu, Levels};

/// `MAX_VL` in bytes.
const MAX_VL_BYTES: usize = VecLen::MAX.bytes();

/// `MAX_PL` in bytes.
const MAX_PL_BYTES: usize = VecLen::MAX.predicate_bytes();

/// An SVE element size, `esize`: the vector element an instruction
/// operates on, and the predicate bits that govern it (one per byte).
///
/// Closed (R5): the four sizes `8 << UInt(size)` gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Esize {
    B,
    H,
    S,
    D,
}

impl Esize {
    /// `8 << UInt(size)`, from a two-bit `size` field.
    pub(crate) const fn from_size(size: u32) -> Self {
        match size & 3 {
            0 => Self::B,
            1 => Self::H,
            2 => Self::S,
            _ => Self::D,
        }
    }

    /// `8 << HighestSetBitNZ(tsize)`, from a nonzero four-bit `tsize` field.
    pub(crate) const fn from_tsize(tsize: u32) -> Self {
        Self::from_size(31 - (tsize & 0xF).leading_zeros())
    }

    /// `esize` in bits.
    #[inline]
    pub(crate) const fn bits(self) -> u32 {
        8 << self as u32
    }

    /// `esize` in bytes: also the predicate bits each element has.
    #[inline]
    pub(crate) const fn bytes(self) -> usize {
        1 << self as u32
    }

    /// `log2(esize DIV 8)`: the size of an access of one element, as
    /// [`Cpu::read_mem`] and [`Cpu::write_mem`] take it.
    #[inline]
    pub(crate) const fn log2_bytes(self) -> u32 {
        self as u32
    }

    /// The element's bits, as a mask of the low `esize` bits.
    #[inline]
    pub(crate) const fn mask(self) -> u64 {
        u64::MAX >> (64 - self.bits())
    }

    /// The element of `value`'s low bits as `SInt`.
    #[inline]
    pub(crate) const fn signed(self, value: u64) -> i64 {
        let shift = 64 - self.bits();
        ((value << shift) as i64) >> shift
    }

    /// The elements of this size in a vector of length `vl`:
    /// `VL DIV esize`.
    #[inline]
    pub(crate) const fn elements(self, vl: VecLen) -> usize {
        vl.bits() as usize / self.bits() as usize
    }
}

/// A scalable vector's value, `bits(VL)`: the low `VL` bits of the
/// storage are the vector, little-endian like a Z register.
#[derive(Clone, Copy)]
pub(crate) struct Zv {
    bytes: [u8; MAX_VL_BYTES],
}

impl Zv {
    /// `Zeros(VL)`.
    pub(crate) const ZERO: Self = Self { bytes: [0; MAX_VL_BYTES] };

    /// `operand[e*:esize]`, zero-extended.
    #[inline]
    pub(crate) fn get(&self, e: usize, es: Esize) -> u64 {
        let n = es.bytes();
        let mut raw = [0u8; 8];
        raw[..n].copy_from_slice(&self.bytes[e * n..e * n + n]);
        u64::from_le_bytes(raw)
    }

    /// `result[e*:esize] = value<esize-1:0>`.
    #[inline]
    pub(crate) fn set(&mut self, e: usize, es: Esize, value: u64) {
        let n = es.bytes();
        self.bytes[e * n..e * n + n].copy_from_slice(&value.to_le_bytes()[..n]);
    }

    /// Byte `i` of the vector.
    #[inline]
    pub(crate) fn byte(&self, i: usize) -> u8 {
        self.bytes[i]
    }

    /// `result<8*i+7:8*i> = value`.
    #[inline]
    pub(crate) fn set_byte(&mut self, i: usize, value: u8) {
        self.bytes[i] = value;
    }

    /// The vector of `vl` whose every element of `es` is `value`:
    /// `Replicate(value<esize-1:0>)`.
    pub(crate) fn replicate(value: u64, es: Esize, vl: VecLen) -> Self {
        let mut z = Self::ZERO;
        for e in 0..es.elements(vl) {
            z.set(e, es, value);
        }
        z
    }

    /// The low 128 bits, `V<n>`'s view.
    #[inline]
    pub(crate) fn low128(&self) -> u128 {
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&self.bytes[..16]);
        u128::from_le_bytes(raw)
    }

    /// The bytes the storage holds above bit 127, which a vector of length
    /// `vl` uses the first `vl.bytes() - 16` of.
    #[inline]
    pub(crate) fn upper(&self) -> &[u8] {
        &self.bytes[16..]
    }

    /// The vector whose bytes are `bytes`, the rest zero.
    pub(crate) fn from_bytes(bytes: &[u8]) -> Self {
        let mut z = Self::ZERO;
        z.bytes[..bytes.len()].copy_from_slice(bytes);
        z
    }

    /// The bytes the storage holds.
    #[inline]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// A vector whose low 128 bits are `low` and whose bytes above are
    /// `upper`'s, the rest zero.
    pub(crate) fn from_parts(low: u128, upper: &[u8]) -> Self {
        let mut z = Self::ZERO;
        z.bytes[..16].copy_from_slice(&low.to_le_bytes());
        z.bytes[16..16 + upper.len()].copy_from_slice(upper);
        z
    }
}

/// A predicate's value, `bits(PL)`: one bit per vector byte, the bit of
/// element `e` of size `esize` at `e * esize DIV 8`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pv {
    bytes: [u8; MAX_PL_BYTES],
}

impl Pv {
    /// `Zeros(PL)`.
    pub(crate) const ZERO: Self = Self { bytes: [0; MAX_PL_BYTES] };

    /// `Ones(PL)` for vectors of length `vl`.
    pub(crate) fn ones(vl: VecLen) -> Self {
        let mut p = Self::ZERO;
        p.bytes[..vl.predicate_bytes()].fill(0xFF);
        p
    }

    /// Bit `n` of the predicate.
    #[inline]
    pub(crate) fn bit(&self, n: usize) -> bool {
        self.bytes[n / 8] & (1 << (n % 8)) != 0
    }

    /// `result[n] = value`.
    #[inline]
    pub(crate) fn set_bit(&mut self, n: usize, value: bool) {
        let byte = &mut self.bytes[n / 8];
        if value {
            *byte |= 1 << (n % 8);
        } else {
            *byte &= !(1 << (n % 8));
        }
    }

    /// `ActivePredicateElement(pred, e, esize)` (and `PredicateElement`,
    /// which reads the same bit).
    #[inline]
    pub(crate) fn active(&self, e: usize, es: Esize) -> bool {
        self.bit(e * es.bytes())
    }

    /// `result[e*:psize] = ZeroExtend(pbit)`: the element's low predicate
    /// bit is `value`, its others zero.
    #[inline]
    pub(crate) fn set_element(&mut self, e: usize, es: Esize, value: bool) {
        let first = e * es.bytes();
        for n in first..first + es.bytes() {
            self.set_bit(n, false);
        }
        self.set_bit(first, value);
    }

    /// `pred[e*:psize]`: all `esize DIV 8` predicate bits of element `e`.
    #[inline]
    pub(crate) fn field(&self, e: usize, es: Esize) -> u8 {
        let first = e * es.bytes();
        (0..es.bytes()).fold(0, |v, i| v | (u8::from(self.bit(first + i)) << i))
    }

    /// `result[e*:psize] = value`.
    #[inline]
    pub(crate) fn set_field(&mut self, e: usize, es: Esize, value: u8) {
        let first = e * es.bytes();
        for i in 0..es.bytes() {
            self.set_bit(first + i, value & (1 << i) != 0);
        }
    }

    /// Byte `i` of the predicate: bits `8*i+7..8*i`.
    #[inline]
    pub(crate) fn byte(&self, i: usize) -> u8 {
        self.bytes[i]
    }

    /// `result<8*i+7:8*i> = value`.
    #[inline]
    pub(crate) fn set_byte(&mut self, i: usize, value: u8) {
        self.bytes[i] = value;
    }

    /// The bytes the storage holds.
    #[inline]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The predicate whose bytes are `bytes`, the rest zero.
    pub(crate) fn from_bytes(bytes: &[u8]) -> Self {
        let mut p = Self::ZERO;
        p.bytes[..bytes.len()].copy_from_slice(bytes);
        p
    }
}

/// Arm `PredTest(mask, result, esize)`: N is the first active element of
/// `result`, Z that no active element is, C the inverse of the last active
/// element, V zero; as `PSTATE.NZCV` holds them (bits 31:28).
pub(crate) fn pred_test(mask: &Pv, result: &Pv, es: Esize, vl: VecLen) -> u64 {
    let elements = es.elements(vl);
    let first = (0..elements).find(|&e| mask.active(e, es)).is_some_and(|e| result.active(e, es));
    let none = !(0..elements).any(|e| mask.active(e, es) && result.active(e, es));
    let last = (0..elements).rev().find(|&e| mask.active(e, es)).is_some_and(|e| result.active(e, es));
    (u64::from(first) << 31) | (u64::from(none) << 30) | (u64::from(!last) << 29)
}

/// Arm `DecodePredCount(bitpattern, esize)` at vector length `vl`: how many
/// elements a predicate constraint `pattern` names.
pub(crate) fn decode_pred_count(pattern: u32, es: Esize, vl: VecLen) -> usize {
    let elements = es.elements(vl);
    let at_least = |n: usize| if elements >= n { n } else { 0 };
    match pattern & 0x1F {
        // POW2: the largest power of two that fits (`FloorPow2`).
        0b00000 => 1 << (usize::BITS - 1 - elements.leading_zeros()),
        // VL1 to VL8.
        n @ 0b00001..=0b01000 => at_least(n as usize),
        // VL16 to VL256.
        n @ 0b01001..=0b01101 => at_least(16 << (n - 0b01001)),
        // MUL4, MUL3, ALL.
        0b11101 => elements - elements % 4,
        0b11110 => elements - elements % 3,
        0b11111 => elements,
        _ => 0,
    }
}

/// Arm `SatQ(value, esize, unsigned)` as the SVE instructions use it,
/// `(result, -) = SatQ(...)`: `value` clamped to the range of an `es`
/// element, the saturation flag discarded (an SVE saturation never sets
/// FPSR.QC).
pub(crate) fn sat_q(value: i128, es: Esize, unsigned: bool) -> u64 {
    sat_q_bits(value, es.bits(), unsigned)
}

/// [`sat_q`] for an `n`-bit result, `n` at most 64: the scalar forms
/// saturate to 32 or 64 bits.
pub(crate) fn sat_q_bits(value: i128, n: u32, unsigned: bool) -> u64 {
    let mut saturated = false;
    sat(value, n, unsigned, &mut saturated)
}

/// The element of `value` (its low `esize` bits) as `UInt` or `SInt`, by
/// `unsigned`: Arm `Int`.
#[inline]
pub(crate) fn int(value: u64, es: Esize, unsigned: bool) -> i128 {
    simd_int(value & es.mask(), es.bits(), unsigned)
}

/// The high half of the `2 * esize`-bit product of elements `a` and `b`
/// (SMULH, UMULH): exact for 64-bit elements, the unsigned product in
/// `u128` and the signed one in `i128`.
pub(crate) fn mul_high(a: u64, b: u64, es: Esize, unsigned: bool) -> u64 {
    let high = if unsigned {
        ((u128::from(a & es.mask()) * u128::from(b & es.mask())) >> es.bits()) as u64
    } else {
        ((i128::from(es.signed(a)) * i128::from(es.signed(b))) >> es.bits()) as u64
    };
    high & es.mask()
}

/// The saturating increment or decrement of a general register by
/// `delta` (SQINCx, UQINCx, SQDECx, UQDECx and their predicate-count
/// forms): of the whole 64-bit register with `wide`, else of `Wdn`, the
/// 32-bit result sign- or zero-extended to 64 bits by `unsigned`.
pub(crate) fn sat_add_scalar(value: u64, delta: i128, wide: bool, unsigned: bool) -> u64 {
    if wide {
        let operand = if unsigned { i128::from(value) } else { i128::from(value as i64) };
        sat_q_bits(operand + delta, 64, unsigned)
    } else {
        let operand = if unsigned { i128::from(value as u32) } else { i128::from(value as i32) };
        let result = sat_q_bits(operand + delta, 32, unsigned);
        if unsigned {
            result
        } else {
            sext(result, 32)
        }
    }
}

impl<L: Levels> Cpu<L> {
    /// Arm `CheckSVEEnabled` on this processor, which implements FEAT_SME
    /// and not FEAT_SVE, and `CurrentVL` after it. In Streaming SVE mode
    /// that is `CheckSMEEnabled`; outside it, `CheckStreamingSVEEnabled`,
    /// which takes the same checks and then the SME trap: an SVE
    /// instruction executes only in Streaming SVE mode, at the Streaming
    /// SVE vector length.
    pub(crate) fn sve_check(&self) -> Result<VecLen, Fault> {
        self.check_sme_enabled()?;
        if !self.svcr.sm {
            return Err(self.sme_trap(SmeTrap::NotStreaming, Route::Class));
        }
        Ok(self.current_svl())
    }
}
