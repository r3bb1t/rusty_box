//! Arm group `sve` (Arm ARM C4.1, "SVE encodings"): the A64 space with
//! `op1 = 0010`. Its groups nest one level deeper than the other top-level
//! groups': `sve` holds groups (`sve_alloca`, ...), which hold the classes.
//!
//! The encodings here are those of FEAT_SME, which is still being built
//! (`Feature::complete`): the streaming vector length instructions. The SVE
//! instructions FEAT_SME makes legal in Streaming SVE mode are not here
//! yet.

use super::Encoding;
use crate::cpu::exec::bits;

/// Arm group `sve_int_arith_svl`: SVE stack frame adjustment (streaming).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntArithSvl {
    AddsvlRRi,
    AddsplRRi,
}

/// Arm group `sve_int_read_svl_a`: Streaming SVE stack frame size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntReadSvlA {
    RdsvlRI,
}

pub(super) fn sve(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 31, 29);
    // op1<7> and op1<4>, then op2<5:2>.
    match (op0, bits(word, 24, 24), bits(word, 21, 21), bits(word, 15, 12)) {
        (0b000, 0b0, 0b1, 0b0101) => sve_alloca(word),
        _ => None,
    }
}

/// Arm group `sve_alloca`: SVE stack frame adjustment and size. `op1 = 0`
/// is `sve_int_arith_vl` and `sve_int_read_vl_a`, ADDVL, ADDPL and RDVL
/// (FEAT_SVE).
fn sve_alloca(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 23, 23);
    let op1 = bits(word, 11, 11);
    match (op0, op1) {
        (0b0, 0b1) => sve_int_arith_svl(word).map(Encoding::SveIntArithSvl),
        (0b1, 0b1) => sve_int_read_svl_a(word).map(Encoding::SveIntReadSvlA),
        _ => None,
    }
}

fn sve_int_arith_svl(word: u32) -> Option<SveIntArithSvl> {
    let op = bits(word, 22, 22);
    match op {
        0b0 => Some(SveIntArithSvl::AddsvlRRi),
        0b1 => Some(SveIntArithSvl::AddsplRRi),
        _ => None,
    }
}

fn sve_int_read_svl_a(word: u32) -> Option<SveIntReadSvlA> {
    let op = bits(word, 22, 22);
    let opc2 = bits(word, 20, 16);
    match (op, opc2) {
        (0b0, 0b11111) => Some(SveIntReadSvlA::RdsvlRI),
        _ => None,
    }
}
