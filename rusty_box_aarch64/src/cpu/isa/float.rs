//! The scalar floating-point groups of Arm group `simd_dp` (Arm ARM C4.1,
//! "Data Processing -- Scalar Floating-Point and Advanced SIMD", the rows
//! with `op0 = x0x1`). `ftype = 11`, half precision, is FEAT_FP16 except
//! in the precision conversions.

use crate::cpu::exec::bits;

/// Arm group `float2fix`: Conversion between floating-point and
/// fixed-point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Float2fix {
    ScvtfS32,
    UcvtfS32,
    Fcvtzs32s,
    Fcvtzu32s,
    ScvtfD32,
    UcvtfD32,
    Fcvtzs32d,
    Fcvtzu32d,
    ScvtfS64,
    UcvtfS64,
    Fcvtzs64s,
    Fcvtzu64s,
    ScvtfD64,
    UcvtfD64,
    Fcvtzs64d,
    Fcvtzu64d,
}

/// Arm group `float2int`: Conversion between floating-point and integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Float2int {
    Fcvtns32s,
    Fcvtnu32s,
    ScvtfS32,
    UcvtfS32,
    Fcvtas32s,
    Fcvtau32s,
    Fmov32s,
    FmovS32,
    Fcvtps32s,
    Fcvtpu32s,
    Fcvtms32s,
    Fcvtmu32s,
    Fcvtzs32s,
    Fcvtzu32s,
    Fcvtns32d,
    Fcvtnu32d,
    ScvtfD32,
    UcvtfD32,
    Fcvtas32d,
    Fcvtau32d,
    Fcvtps32d,
    Fcvtpu32d,
    Fcvtms32d,
    Fcvtmu32d,
    Fcvtzs32d,
    Fcvtzu32d,
    Fcvtns64s,
    Fcvtnu64s,
    ScvtfS64,
    UcvtfS64,
    Fcvtas64s,
    Fcvtau64s,
    Fcvtps64s,
    Fcvtpu64s,
    Fcvtms64s,
    Fcvtmu64s,
    Fcvtzs64s,
    Fcvtzu64s,
    Fcvtns64d,
    Fcvtnu64d,
    ScvtfD64,
    UcvtfD64,
    Fcvtas64d,
    Fcvtau64d,
    Fmov64d,
    FmovD64,
    Fcvtps64d,
    Fcvtpu64d,
    Fcvtms64d,
    Fcvtmu64d,
    Fcvtzs64d,
    Fcvtzu64d,
    Fmov64vx,
    FmovV64i,
}

/// Arm group `floatdp1`: Floating-point data-processing (1 source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatdp1 {
    FmovS,
    FabsS,
    FnegS,
    FsqrtS,
    FcvtDs,
    FcvtHs,
    FrintnS,
    FrintpS,
    FrintmS,
    FrintzS,
    FrintaS,
    FrintxS,
    FrintiS,
    FmovD,
    FabsD,
    FnegD,
    FsqrtD,
    FcvtSd,
    FcvtHd,
    FrintnD,
    FrintpD,
    FrintmD,
    FrintzD,
    FrintaD,
    FrintxD,
    FrintiD,
    FcvtSh,
    FcvtDh,
}

/// Arm group `floatcmp`: Floating-point compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatcmp {
    FcmpS,
    FcmpSz,
    FcmpeS,
    FcmpeSz,
    FcmpD,
    FcmpDz,
    FcmpeD,
    FcmpeDz,
}

/// Arm group `floatimm`: Floating-point immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatimm {
    FmovS,
    FmovD,
}

/// Arm group `floatccmp`: Floating-point conditional compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatccmp {
    FccmpS,
    FccmpeS,
    FccmpD,
    FccmpeD,
}

/// Arm group `floatdp2`: Floating-point data-processing (2 source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatdp2 {
    FmulS,
    FdivS,
    FaddS,
    FsubS,
    FmaxS,
    FminS,
    FmaxnmS,
    FminnmS,
    FnmulS,
    FmulD,
    FdivD,
    FaddD,
    FsubD,
    FmaxD,
    FminD,
    FmaxnmD,
    FminnmD,
    FnmulD,
}

/// Arm group `floatsel`: Floating-point conditional select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatsel {
    FcselS,
    FcselD,
}

/// Arm group `floatdp3`: Floating-point data-processing (3 source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatdp3 {
    FmaddS,
    FmsubS,
    FnmaddS,
    FnmsubS,
    FmaddD,
    FmsubD,
    FnmaddD,
    FnmsubD,
}

pub(super) fn float2fix(word: u32) -> Option<Float2fix> {
    let sf = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let rmode = bits(word, 20, 19);
    let opcode = bits(word, 18, 16);
    match (sf, s, ftype, rmode, opcode) {
        (0b0, 0b0, 0b00, 0b00, 0b010) => Some(Float2fix::ScvtfS32),
        (0b0, 0b0, 0b00, 0b00, 0b011) => Some(Float2fix::UcvtfS32),
        (0b0, 0b0, 0b00, 0b11, 0b000) => Some(Float2fix::Fcvtzs32s),
        (0b0, 0b0, 0b00, 0b11, 0b001) => Some(Float2fix::Fcvtzu32s),
        (0b0, 0b0, 0b01, 0b00, 0b010) => Some(Float2fix::ScvtfD32),
        (0b0, 0b0, 0b01, 0b00, 0b011) => Some(Float2fix::UcvtfD32),
        (0b0, 0b0, 0b01, 0b11, 0b000) => Some(Float2fix::Fcvtzs32d),
        (0b0, 0b0, 0b01, 0b11, 0b001) => Some(Float2fix::Fcvtzu32d),
        (0b1, 0b0, 0b00, 0b00, 0b010) => Some(Float2fix::ScvtfS64),
        (0b1, 0b0, 0b00, 0b00, 0b011) => Some(Float2fix::UcvtfS64),
        (0b1, 0b0, 0b00, 0b11, 0b000) => Some(Float2fix::Fcvtzs64s),
        (0b1, 0b0, 0b00, 0b11, 0b001) => Some(Float2fix::Fcvtzu64s),
        (0b1, 0b0, 0b01, 0b00, 0b010) => Some(Float2fix::ScvtfD64),
        (0b1, 0b0, 0b01, 0b00, 0b011) => Some(Float2fix::UcvtfD64),
        (0b1, 0b0, 0b01, 0b11, 0b000) => Some(Float2fix::Fcvtzs64d),
        (0b1, 0b0, 0b01, 0b11, 0b001) => Some(Float2fix::Fcvtzu64d),
        _ => None,
    }
}

pub(super) fn float2int(word: u32) -> Option<Float2int> {
    let sf = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let rmode = bits(word, 20, 19);
    let opcode = bits(word, 18, 16);
    // ftype = 11 is FEAT_FP16; rmode 01, 10 and 11 with opcode 01x, 10x and
    // 11x hold the FEAT_FPRCVT conversions and FJCVTZS (FEAT_JSCVT).
    match (sf, s, ftype, rmode, opcode) {
        (0b0, 0b0, 0b00, 0b00, 0b000) => Some(Float2int::Fcvtns32s),
        (0b0, 0b0, 0b00, 0b00, 0b001) => Some(Float2int::Fcvtnu32s),
        (0b0, 0b0, 0b00, 0b00, 0b010) => Some(Float2int::ScvtfS32),
        (0b0, 0b0, 0b00, 0b00, 0b011) => Some(Float2int::UcvtfS32),
        (0b0, 0b0, 0b00, 0b00, 0b100) => Some(Float2int::Fcvtas32s),
        (0b0, 0b0, 0b00, 0b00, 0b101) => Some(Float2int::Fcvtau32s),
        (0b0, 0b0, 0b00, 0b00, 0b110) => Some(Float2int::Fmov32s),
        (0b0, 0b0, 0b00, 0b00, 0b111) => Some(Float2int::FmovS32),
        (0b0, 0b0, 0b00, 0b01, 0b000) => Some(Float2int::Fcvtps32s),
        (0b0, 0b0, 0b00, 0b01, 0b001) => Some(Float2int::Fcvtpu32s),
        (0b0, 0b0, 0b00, 0b10, 0b000) => Some(Float2int::Fcvtms32s),
        (0b0, 0b0, 0b00, 0b10, 0b001) => Some(Float2int::Fcvtmu32s),
        (0b0, 0b0, 0b00, 0b11, 0b000) => Some(Float2int::Fcvtzs32s),
        (0b0, 0b0, 0b00, 0b11, 0b001) => Some(Float2int::Fcvtzu32s),
        (0b0, 0b0, 0b01, 0b00, 0b000) => Some(Float2int::Fcvtns32d),
        (0b0, 0b0, 0b01, 0b00, 0b001) => Some(Float2int::Fcvtnu32d),
        (0b0, 0b0, 0b01, 0b00, 0b010) => Some(Float2int::ScvtfD32),
        (0b0, 0b0, 0b01, 0b00, 0b011) => Some(Float2int::UcvtfD32),
        (0b0, 0b0, 0b01, 0b00, 0b100) => Some(Float2int::Fcvtas32d),
        (0b0, 0b0, 0b01, 0b00, 0b101) => Some(Float2int::Fcvtau32d),
        (0b0, 0b0, 0b01, 0b01, 0b000) => Some(Float2int::Fcvtps32d),
        (0b0, 0b0, 0b01, 0b01, 0b001) => Some(Float2int::Fcvtpu32d),
        (0b0, 0b0, 0b01, 0b10, 0b000) => Some(Float2int::Fcvtms32d),
        (0b0, 0b0, 0b01, 0b10, 0b001) => Some(Float2int::Fcvtmu32d),
        (0b0, 0b0, 0b01, 0b11, 0b000) => Some(Float2int::Fcvtzs32d),
        (0b0, 0b0, 0b01, 0b11, 0b001) => Some(Float2int::Fcvtzu32d),
        (0b1, 0b0, 0b00, 0b00, 0b000) => Some(Float2int::Fcvtns64s),
        (0b1, 0b0, 0b00, 0b00, 0b001) => Some(Float2int::Fcvtnu64s),
        (0b1, 0b0, 0b00, 0b00, 0b010) => Some(Float2int::ScvtfS64),
        (0b1, 0b0, 0b00, 0b00, 0b011) => Some(Float2int::UcvtfS64),
        (0b1, 0b0, 0b00, 0b00, 0b100) => Some(Float2int::Fcvtas64s),
        (0b1, 0b0, 0b00, 0b00, 0b101) => Some(Float2int::Fcvtau64s),
        (0b1, 0b0, 0b00, 0b01, 0b000) => Some(Float2int::Fcvtps64s),
        (0b1, 0b0, 0b00, 0b01, 0b001) => Some(Float2int::Fcvtpu64s),
        (0b1, 0b0, 0b00, 0b10, 0b000) => Some(Float2int::Fcvtms64s),
        (0b1, 0b0, 0b00, 0b10, 0b001) => Some(Float2int::Fcvtmu64s),
        (0b1, 0b0, 0b00, 0b11, 0b000) => Some(Float2int::Fcvtzs64s),
        (0b1, 0b0, 0b00, 0b11, 0b001) => Some(Float2int::Fcvtzu64s),
        (0b1, 0b0, 0b01, 0b00, 0b000) => Some(Float2int::Fcvtns64d),
        (0b1, 0b0, 0b01, 0b00, 0b001) => Some(Float2int::Fcvtnu64d),
        (0b1, 0b0, 0b01, 0b00, 0b010) => Some(Float2int::ScvtfD64),
        (0b1, 0b0, 0b01, 0b00, 0b011) => Some(Float2int::UcvtfD64),
        (0b1, 0b0, 0b01, 0b00, 0b100) => Some(Float2int::Fcvtas64d),
        (0b1, 0b0, 0b01, 0b00, 0b101) => Some(Float2int::Fcvtau64d),
        (0b1, 0b0, 0b01, 0b00, 0b110) => Some(Float2int::Fmov64d),
        (0b1, 0b0, 0b01, 0b00, 0b111) => Some(Float2int::FmovD64),
        (0b1, 0b0, 0b01, 0b01, 0b000) => Some(Float2int::Fcvtps64d),
        (0b1, 0b0, 0b01, 0b01, 0b001) => Some(Float2int::Fcvtpu64d),
        (0b1, 0b0, 0b01, 0b10, 0b000) => Some(Float2int::Fcvtms64d),
        (0b1, 0b0, 0b01, 0b10, 0b001) => Some(Float2int::Fcvtmu64d),
        (0b1, 0b0, 0b01, 0b11, 0b000) => Some(Float2int::Fcvtzs64d),
        (0b1, 0b0, 0b01, 0b11, 0b001) => Some(Float2int::Fcvtzu64d),
        (0b1, 0b0, 0b10, 0b01, 0b110) => Some(Float2int::Fmov64vx),
        (0b1, 0b0, 0b10, 0b01, 0b111) => Some(Float2int::FmovV64i),
        _ => None,
    }
}

pub(super) fn floatdp1(word: u32) -> Option<Floatdp1> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let opcode = bits(word, 20, 15);
    match (m, s, ftype, opcode) {
        (0b0, 0b0, 0b00, 0b000000) => Some(Floatdp1::FmovS),
        (0b0, 0b0, 0b00, 0b000001) => Some(Floatdp1::FabsS),
        (0b0, 0b0, 0b00, 0b000010) => Some(Floatdp1::FnegS),
        (0b0, 0b0, 0b00, 0b000011) => Some(Floatdp1::FsqrtS),
        (0b0, 0b0, 0b00, 0b000101) => Some(Floatdp1::FcvtDs),
        (0b0, 0b0, 0b00, 0b000111) => Some(Floatdp1::FcvtHs),
        (0b0, 0b0, 0b00, 0b001000) => Some(Floatdp1::FrintnS),
        (0b0, 0b0, 0b00, 0b001001) => Some(Floatdp1::FrintpS),
        (0b0, 0b0, 0b00, 0b001010) => Some(Floatdp1::FrintmS),
        (0b0, 0b0, 0b00, 0b001011) => Some(Floatdp1::FrintzS),
        (0b0, 0b0, 0b00, 0b001100) => Some(Floatdp1::FrintaS),
        (0b0, 0b0, 0b00, 0b001110) => Some(Floatdp1::FrintxS),
        (0b0, 0b0, 0b00, 0b001111) => Some(Floatdp1::FrintiS),
        (0b0, 0b0, 0b01, 0b000000) => Some(Floatdp1::FmovD),
        (0b0, 0b0, 0b01, 0b000001) => Some(Floatdp1::FabsD),
        (0b0, 0b0, 0b01, 0b000010) => Some(Floatdp1::FnegD),
        (0b0, 0b0, 0b01, 0b000011) => Some(Floatdp1::FsqrtD),
        (0b0, 0b0, 0b01, 0b000100) => Some(Floatdp1::FcvtSd),
        (0b0, 0b0, 0b01, 0b000111) => Some(Floatdp1::FcvtHd),
        (0b0, 0b0, 0b01, 0b001000) => Some(Floatdp1::FrintnD),
        (0b0, 0b0, 0b01, 0b001001) => Some(Floatdp1::FrintpD),
        (0b0, 0b0, 0b01, 0b001010) => Some(Floatdp1::FrintmD),
        (0b0, 0b0, 0b01, 0b001011) => Some(Floatdp1::FrintzD),
        (0b0, 0b0, 0b01, 0b001100) => Some(Floatdp1::FrintaD),
        (0b0, 0b0, 0b01, 0b001110) => Some(Floatdp1::FrintxD),
        (0b0, 0b0, 0b01, 0b001111) => Some(Floatdp1::FrintiD),
        (0b0, 0b0, 0b11, 0b000100) => Some(Floatdp1::FcvtSh),
        (0b0, 0b0, 0b11, 0b000101) => Some(Floatdp1::FcvtDh),
        _ => None,
    }
}

pub(super) fn floatcmp(word: u32) -> Option<Floatcmp> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let rm = bits(word, 20, 16);
    let op = bits(word, 15, 14);
    let opcode2 = bits(word, 4, 0);
    // Rm is should-be-zero in the compares with zero: fixed here.
    match (m, s, ftype, rm, op, opcode2) {
        (0b0, 0b0, 0b00, _, 0b00, 0b00000) => Some(Floatcmp::FcmpS),
        (0b0, 0b0, 0b00, 0b00000, 0b00, 0b01000) => Some(Floatcmp::FcmpSz),
        (0b0, 0b0, 0b00, _, 0b00, 0b10000) => Some(Floatcmp::FcmpeS),
        (0b0, 0b0, 0b00, 0b00000, 0b00, 0b11000) => Some(Floatcmp::FcmpeSz),
        (0b0, 0b0, 0b01, _, 0b00, 0b00000) => Some(Floatcmp::FcmpD),
        (0b0, 0b0, 0b01, 0b00000, 0b00, 0b01000) => Some(Floatcmp::FcmpDz),
        (0b0, 0b0, 0b01, _, 0b00, 0b10000) => Some(Floatcmp::FcmpeD),
        (0b0, 0b0, 0b01, 0b00000, 0b00, 0b11000) => Some(Floatcmp::FcmpeDz),
        _ => None,
    }
}

pub(super) fn floatimm(word: u32) -> Option<Floatimm> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let imm5 = bits(word, 9, 5);
    match (m, s, ftype, imm5) {
        (0b0, 0b0, 0b00, 0b00000) => Some(Floatimm::FmovS),
        (0b0, 0b0, 0b01, 0b00000) => Some(Floatimm::FmovD),
        _ => None,
    }
}

pub(super) fn floatccmp(word: u32) -> Option<Floatccmp> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let op = bits(word, 4, 4);
    match (m, s, ftype, op) {
        (0b0, 0b0, 0b00, 0b0) => Some(Floatccmp::FccmpS),
        (0b0, 0b0, 0b00, 0b1) => Some(Floatccmp::FccmpeS),
        (0b0, 0b0, 0b01, 0b0) => Some(Floatccmp::FccmpD),
        (0b0, 0b0, 0b01, 0b1) => Some(Floatccmp::FccmpeD),
        _ => None,
    }
}

pub(super) fn floatdp2(word: u32) -> Option<Floatdp2> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let opcode = bits(word, 15, 12);
    match (m, s, ftype, opcode) {
        (0b0, 0b0, 0b00, 0b0000) => Some(Floatdp2::FmulS),
        (0b0, 0b0, 0b00, 0b0001) => Some(Floatdp2::FdivS),
        (0b0, 0b0, 0b00, 0b0010) => Some(Floatdp2::FaddS),
        (0b0, 0b0, 0b00, 0b0011) => Some(Floatdp2::FsubS),
        (0b0, 0b0, 0b00, 0b0100) => Some(Floatdp2::FmaxS),
        (0b0, 0b0, 0b00, 0b0101) => Some(Floatdp2::FminS),
        (0b0, 0b0, 0b00, 0b0110) => Some(Floatdp2::FmaxnmS),
        (0b0, 0b0, 0b00, 0b0111) => Some(Floatdp2::FminnmS),
        (0b0, 0b0, 0b00, 0b1000) => Some(Floatdp2::FnmulS),
        (0b0, 0b0, 0b01, 0b0000) => Some(Floatdp2::FmulD),
        (0b0, 0b0, 0b01, 0b0001) => Some(Floatdp2::FdivD),
        (0b0, 0b0, 0b01, 0b0010) => Some(Floatdp2::FaddD),
        (0b0, 0b0, 0b01, 0b0011) => Some(Floatdp2::FsubD),
        (0b0, 0b0, 0b01, 0b0100) => Some(Floatdp2::FmaxD),
        (0b0, 0b0, 0b01, 0b0101) => Some(Floatdp2::FminD),
        (0b0, 0b0, 0b01, 0b0110) => Some(Floatdp2::FmaxnmD),
        (0b0, 0b0, 0b01, 0b0111) => Some(Floatdp2::FminnmD),
        (0b0, 0b0, 0b01, 0b1000) => Some(Floatdp2::FnmulD),
        _ => None,
    }
}

pub(super) fn floatsel(word: u32) -> Option<Floatsel> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    match (m, s, ftype) {
        (0b0, 0b0, 0b00) => Some(Floatsel::FcselS),
        (0b0, 0b0, 0b01) => Some(Floatsel::FcselD),
        _ => None,
    }
}

pub(super) fn floatdp3(word: u32) -> Option<Floatdp3> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let o1 = bits(word, 21, 21);
    let o0 = bits(word, 15, 15);
    match (m, s, ftype, o1, o0) {
        (0b0, 0b0, 0b00, 0b0, 0b0) => Some(Floatdp3::FmaddS),
        (0b0, 0b0, 0b00, 0b0, 0b1) => Some(Floatdp3::FmsubS),
        (0b0, 0b0, 0b00, 0b1, 0b0) => Some(Floatdp3::FnmaddS),
        (0b0, 0b0, 0b00, 0b1, 0b1) => Some(Floatdp3::FnmsubS),
        (0b0, 0b0, 0b01, 0b0, 0b0) => Some(Floatdp3::FmaddD),
        (0b0, 0b0, 0b01, 0b0, 0b1) => Some(Floatdp3::FmsubD),
        (0b0, 0b0, 0b01, 0b1, 0b0) => Some(Floatdp3::FnmaddD),
        (0b0, 0b0, 0b01, 0b1, 0b1) => Some(Floatdp3::FnmsubD),
        _ => None,
    }
}
