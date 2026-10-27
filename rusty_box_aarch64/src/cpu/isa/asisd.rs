//! The Advanced SIMD scalar groups of Arm group `simd_dp` (Arm ARM C4.1,
//! "Data Processing -- Scalar Floating-Point and Advanced SIMD", the rows
//! with `op0 = 01x1`).

use super::Feature;
use crate::cpu::exec::bits;

/// Arm group `asisdone`: Advanced SIMD scalar copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisdone {
    Dup,
}

/// Arm group `asisdmisc`: Advanced SIMD scalar two-register miscellaneous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisdmisc {
    SuqaddR,
    SqabsR,
    CmgtZ,
    CmeqZ,
    CmltZ,
    AbsR,
    SqxtnN,
    FcvtnsR,
    FcvtmsR,
    FcvtasR,
    ScvtfR,
    FcmgtFz,
    FcmeqFz,
    FcmltFz,
    FcvtpsR,
    FcvtzsR,
    FrecpeR,
    FrecpxR,
    UsqaddR,
    SqnegR,
    CmgeZ,
    CmleZ,
    NegR,
    SqxtunN,
    UqxtnN,
    FcvtxnN,
    FcvtnuR,
    FcvtmuR,
    FcvtauR,
    UcvtfR,
    FcmgeFz,
    FcmleFz,
    FcvtpuR,
    FcvtzuR,
    FrsqrteR,
}

/// Arm group `asisdpair`: Advanced SIMD scalar pairwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisdpair {
    Addp,
    FmaxnmpSd,
    FaddpSd,
    FmaxpSd,
    FminnmpSd,
    FminpSd,
    FmaxnmpH,
    FaddpH,
    FmaxpH,
    FminnmpH,
    FminpH,
}

impl Asisdpair {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Addp
            | Self::FmaxnmpSd
            | Self::FaddpSd
            | Self::FmaxpSd
            | Self::FminnmpSd
            | Self::FminpSd => Feature::AdvSimd,
            Self::FmaxnmpH | Self::FaddpH | Self::FmaxpH | Self::FminnmpH | Self::FminpH => Feature::Fp16,
        }
    }
}

/// Arm group `asisddiff`: Advanced SIMD scalar three different.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisddiff {
    Sqdmlal,
    Sqdmlsl,
    Sqdmull,
}

/// Arm group `asisdsame`: Advanced SIMD scalar three same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisdsame {
    Sqadd,
    Sqsub,
    Cmgt,
    Cmge,
    Sshl,
    Sqshl,
    Srshl,
    Sqrshl,
    Add,
    Cmtst,
    Sqdmulh,
    Fmulx,
    Fcmeq,
    Frecps,
    Frsqrts,
    Uqadd,
    Uqsub,
    Cmhi,
    Cmhs,
    Ushl,
    Uqshl,
    Urshl,
    Uqrshl,
    Sub,
    Cmeq,
    Sqrdmulh,
    Fcmge,
    Facge,
    Fabd,
    Fcmgt,
    Facgt,
}

/// Arm group `asisdshf`: Advanced SIMD scalar shift by immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisdshf {
    SshrR,
    SsraR,
    SrshrR,
    SrsraR,
    ShlR,
    SqshlR,
    SqshrnN,
    SqrshrnN,
    ScvtfC,
    FcvtzsC,
    UshrR,
    UsraR,
    UrshrR,
    UrsraR,
    SriR,
    SliR,
    SqshluR,
    UqshlR,
    SqshrunN,
    SqrshrunN,
    UqshrnN,
    UqrshrnN,
    UcvtfC,
    FcvtzuC,
}

/// Arm group `asisdelem`: Advanced SIMD scalar x indexed element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisdelem {
    SqdmlalL,
    SqdmlslL,
    SqdmullL,
    SqdmulhR,
    SqrdmulhR,
    FmlaRSd,
    FmlsRSd,
    FmulRSd,
    FmulxRSd,
    SqrdmlahR,
    SqrdmlshR,
    FmlaRhH,
    FmlsRhH,
    FmulRhH,
    FmulxRhH,
}

impl Asisdelem {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::SqdmlalL
            | Self::SqdmlslL
            | Self::SqdmullL
            | Self::SqdmulhR
            | Self::SqrdmulhR
            | Self::FmlaRSd
            | Self::FmlsRSd
            | Self::FmulRSd
            | Self::FmulxRSd => Feature::AdvSimd,
            Self::SqrdmlahR | Self::SqrdmlshR => Feature::Rdm,
            Self::FmlaRhH | Self::FmlsRhH | Self::FmulRhH | Self::FmulxRhH => Feature::Fp16,
        }
    }
}

/// Arm group `asisdsame2`: Advanced SIMD scalar three same extra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisdsame2 {
    Sqrdmlah,
    Sqrdmlsh,
}

impl Asisdsame2 {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Sqrdmlah | Self::Sqrdmlsh => Feature::Rdm,
        }
    }
}

pub(super) fn asisdone(word: u32) -> Option<Asisdone> {
    let op = bits(word, 29, 29);
    let imm4 = bits(word, 14, 11);
    match (op, imm4) {
        (0b0, 0b0000) => Some(Asisdone::Dup),
        _ => None,
    }
}

pub(super) fn asisdmisc(word: u32) -> Option<Asisdmisc> {
    let u = bits(word, 29, 29);
    let size = bits(word, 23, 22);
    let opcode = bits(word, 16, 12);
    match (u, size, opcode) {
        (0b0, _, 0b00011) => Some(Asisdmisc::SuqaddR),
        (0b0, _, 0b00111) => Some(Asisdmisc::SqabsR),
        (0b0, 0b11, 0b01000) => Some(Asisdmisc::CmgtZ),
        (0b0, 0b11, 0b01001) => Some(Asisdmisc::CmeqZ),
        (0b0, 0b11, 0b01010) => Some(Asisdmisc::CmltZ),
        (0b0, 0b11, 0b01011) => Some(Asisdmisc::AbsR),
        (0b0, _, 0b10100) => Some(Asisdmisc::SqxtnN),
        (0b0, 0b00 | 0b01, 0b11010) => Some(Asisdmisc::FcvtnsR),
        (0b0, 0b00 | 0b01, 0b11011) => Some(Asisdmisc::FcvtmsR),
        (0b0, 0b00 | 0b01, 0b11100) => Some(Asisdmisc::FcvtasR),
        (0b0, 0b00 | 0b01, 0b11101) => Some(Asisdmisc::ScvtfR),
        (0b0, 0b10 | 0b11, 0b01100) => Some(Asisdmisc::FcmgtFz),
        (0b0, 0b10 | 0b11, 0b01101) => Some(Asisdmisc::FcmeqFz),
        (0b0, 0b10 | 0b11, 0b01110) => Some(Asisdmisc::FcmltFz),
        (0b0, 0b10 | 0b11, 0b11010) => Some(Asisdmisc::FcvtpsR),
        (0b0, 0b10 | 0b11, 0b11011) => Some(Asisdmisc::FcvtzsR),
        (0b0, 0b10 | 0b11, 0b11101) => Some(Asisdmisc::FrecpeR),
        (0b0, 0b10 | 0b11, 0b11111) => Some(Asisdmisc::FrecpxR),
        (0b1, _, 0b00011) => Some(Asisdmisc::UsqaddR),
        (0b1, _, 0b00111) => Some(Asisdmisc::SqnegR),
        (0b1, 0b11, 0b01000) => Some(Asisdmisc::CmgeZ),
        (0b1, 0b11, 0b01001) => Some(Asisdmisc::CmleZ),
        (0b1, 0b11, 0b01011) => Some(Asisdmisc::NegR),
        (0b1, _, 0b10010) => Some(Asisdmisc::SqxtunN),
        (0b1, _, 0b10100) => Some(Asisdmisc::UqxtnN),
        (0b1, 0b01, 0b10110) => Some(Asisdmisc::FcvtxnN),
        (0b1, 0b00 | 0b01, 0b11010) => Some(Asisdmisc::FcvtnuR),
        (0b1, 0b00 | 0b01, 0b11011) => Some(Asisdmisc::FcvtmuR),
        (0b1, 0b00 | 0b01, 0b11100) => Some(Asisdmisc::FcvtauR),
        (0b1, 0b00 | 0b01, 0b11101) => Some(Asisdmisc::UcvtfR),
        (0b1, 0b10 | 0b11, 0b01100) => Some(Asisdmisc::FcmgeFz),
        (0b1, 0b10 | 0b11, 0b01101) => Some(Asisdmisc::FcmleFz),
        (0b1, 0b10 | 0b11, 0b11010) => Some(Asisdmisc::FcvtpuR),
        (0b1, 0b10 | 0b11, 0b11011) => Some(Asisdmisc::FcvtzuR),
        (0b1, 0b10 | 0b11, 0b11101) => Some(Asisdmisc::FrsqrteR),
        _ => None,
    }
}

pub(super) fn asisdpair(word: u32) -> Option<Asisdpair> {
    let u = bits(word, 29, 29);
    let size = bits(word, 23, 22);
    let opcode = bits(word, 16, 12);
    match (u, size, opcode) {
        (0b0, 0b11, 0b11011) => Some(Asisdpair::Addp),
        (0b1, 0b00 | 0b01, 0b01100) => Some(Asisdpair::FmaxnmpSd),
        (0b1, 0b00 | 0b01, 0b01101) => Some(Asisdpair::FaddpSd),
        (0b1, 0b00 | 0b01, 0b01111) => Some(Asisdpair::FmaxpSd),
        (0b1, 0b10 | 0b11, 0b01100) => Some(Asisdpair::FminnmpSd),
        (0b1, 0b10 | 0b11, 0b01111) => Some(Asisdpair::FminpSd),
        (0b0, 0b00, 0b01100) => Some(Asisdpair::FmaxnmpH),
        (0b0, 0b00, 0b01101) => Some(Asisdpair::FaddpH),
        (0b0, 0b00, 0b01111) => Some(Asisdpair::FmaxpH),
        (0b0, 0b10, 0b01100) => Some(Asisdpair::FminnmpH),
        (0b0, 0b10, 0b01111) => Some(Asisdpair::FminpH),
        _ => None,
    }
}

pub(super) fn asisddiff(word: u32) -> Option<Asisddiff> {
    let u = bits(word, 29, 29);
    let opcode = bits(word, 15, 12);
    match (u, opcode) {
        (0b0, 0b1001) => Some(Asisddiff::Sqdmlal),
        (0b0, 0b1011) => Some(Asisddiff::Sqdmlsl),
        (0b0, 0b1101) => Some(Asisddiff::Sqdmull),
        _ => None,
    }
}

pub(super) fn asisdsame(word: u32) -> Option<Asisdsame> {
    let u = bits(word, 29, 29);
    let size = bits(word, 23, 22);
    let opcode = bits(word, 15, 11);
    match (u, size, opcode) {
        (0b0, _, 0b00001) => Some(Asisdsame::Sqadd),
        (0b0, _, 0b00101) => Some(Asisdsame::Sqsub),
        (0b0, 0b11, 0b00110) => Some(Asisdsame::Cmgt),
        (0b0, 0b11, 0b00111) => Some(Asisdsame::Cmge),
        (0b0, 0b11, 0b01000) => Some(Asisdsame::Sshl),
        (0b0, _, 0b01001) => Some(Asisdsame::Sqshl),
        (0b0, 0b11, 0b01010) => Some(Asisdsame::Srshl),
        (0b0, _, 0b01011) => Some(Asisdsame::Sqrshl),
        (0b0, 0b11, 0b10000) => Some(Asisdsame::Add),
        (0b0, 0b11, 0b10001) => Some(Asisdsame::Cmtst),
        (0b0, _, 0b10110) => Some(Asisdsame::Sqdmulh),
        (0b0, 0b00 | 0b01, 0b11011) => Some(Asisdsame::Fmulx),
        (0b0, 0b00 | 0b01, 0b11100) => Some(Asisdsame::Fcmeq),
        (0b0, 0b00 | 0b01, 0b11111) => Some(Asisdsame::Frecps),
        (0b0, 0b10 | 0b11, 0b11111) => Some(Asisdsame::Frsqrts),
        (0b1, _, 0b00001) => Some(Asisdsame::Uqadd),
        (0b1, _, 0b00101) => Some(Asisdsame::Uqsub),
        (0b1, 0b11, 0b00110) => Some(Asisdsame::Cmhi),
        (0b1, 0b11, 0b00111) => Some(Asisdsame::Cmhs),
        (0b1, 0b11, 0b01000) => Some(Asisdsame::Ushl),
        (0b1, _, 0b01001) => Some(Asisdsame::Uqshl),
        (0b1, 0b11, 0b01010) => Some(Asisdsame::Urshl),
        (0b1, _, 0b01011) => Some(Asisdsame::Uqrshl),
        (0b1, 0b11, 0b10000) => Some(Asisdsame::Sub),
        (0b1, 0b11, 0b10001) => Some(Asisdsame::Cmeq),
        (0b1, _, 0b10110) => Some(Asisdsame::Sqrdmulh),
        (0b1, 0b00 | 0b01, 0b11100) => Some(Asisdsame::Fcmge),
        (0b1, 0b00 | 0b01, 0b11101) => Some(Asisdsame::Facge),
        (0b1, 0b10 | 0b11, 0b11010) => Some(Asisdsame::Fabd),
        (0b1, 0b10 | 0b11, 0b11100) => Some(Asisdsame::Fcmgt),
        (0b1, 0b10 | 0b11, 0b11101) => Some(Asisdsame::Facgt),
        _ => None,
    }
}

pub(super) fn asisdshf(word: u32) -> Option<Asisdshf> {
    let u = bits(word, 29, 29);
    let immh = bits(word, 22, 19);
    let opcode = bits(word, 15, 11);
    // immh = 0000 is unallocated; the right shifts and inserts need
    // immh = 1xxx, a doubleword element.
    match (u, immh, opcode) {
        (_, 0b0000, _) => None,
        (0b0, 0b1000..=0b1111, 0b00000) => Some(Asisdshf::SshrR),
        (0b0, 0b1000..=0b1111, 0b00010) => Some(Asisdshf::SsraR),
        (0b0, 0b1000..=0b1111, 0b00100) => Some(Asisdshf::SrshrR),
        (0b0, 0b1000..=0b1111, 0b00110) => Some(Asisdshf::SrsraR),
        (0b0, 0b1000..=0b1111, 0b01010) => Some(Asisdshf::ShlR),
        (0b0, _, 0b01110) => Some(Asisdshf::SqshlR),
        (0b0, _, 0b10010) => Some(Asisdshf::SqshrnN),
        (0b0, _, 0b10011) => Some(Asisdshf::SqrshrnN),
        (0b0, _, 0b11100) => Some(Asisdshf::ScvtfC),
        (0b0, _, 0b11111) => Some(Asisdshf::FcvtzsC),
        (0b1, 0b1000..=0b1111, 0b00000) => Some(Asisdshf::UshrR),
        (0b1, 0b1000..=0b1111, 0b00010) => Some(Asisdshf::UsraR),
        (0b1, 0b1000..=0b1111, 0b00100) => Some(Asisdshf::UrshrR),
        (0b1, 0b1000..=0b1111, 0b00110) => Some(Asisdshf::UrsraR),
        (0b1, 0b1000..=0b1111, 0b01000) => Some(Asisdshf::SriR),
        (0b1, 0b1000..=0b1111, 0b01010) => Some(Asisdshf::SliR),
        (0b1, _, 0b01100) => Some(Asisdshf::SqshluR),
        (0b1, _, 0b01110) => Some(Asisdshf::UqshlR),
        (0b1, _, 0b10000) => Some(Asisdshf::SqshrunN),
        (0b1, _, 0b10001) => Some(Asisdshf::SqrshrunN),
        (0b1, _, 0b10010) => Some(Asisdshf::UqshrnN),
        (0b1, _, 0b10011) => Some(Asisdshf::UqrshrnN),
        (0b1, _, 0b11100) => Some(Asisdshf::UcvtfC),
        (0b1, _, 0b11111) => Some(Asisdshf::FcvtzuC),
        _ => None,
    }
}

pub(super) fn asisdelem(word: u32) -> Option<Asisdelem> {
    let u = bits(word, 29, 29);
    let size = bits(word, 23, 22);
    let opcode = bits(word, 15, 12);
    match (u, size, opcode) {
        (0b0, _, 0b0011) => Some(Asisdelem::SqdmlalL),
        (0b0, _, 0b0111) => Some(Asisdelem::SqdmlslL),
        (0b0, _, 0b1011) => Some(Asisdelem::SqdmullL),
        (0b0, _, 0b1100) => Some(Asisdelem::SqdmulhR),
        (0b0, _, 0b1101) => Some(Asisdelem::SqrdmulhR),
        (0b0, 0b10 | 0b11, 0b0001) => Some(Asisdelem::FmlaRSd),
        (0b0, 0b10 | 0b11, 0b0101) => Some(Asisdelem::FmlsRSd),
        (0b0, 0b10 | 0b11, 0b1001) => Some(Asisdelem::FmulRSd),
        (0b1, 0b10 | 0b11, 0b1001) => Some(Asisdelem::FmulxRSd),
        (0b1, _, 0b1101) => Some(Asisdelem::SqrdmlahR),
        (0b1, _, 0b1111) => Some(Asisdelem::SqrdmlshR),
        (0b0, 0b00, 0b0001) => Some(Asisdelem::FmlaRhH),
        (0b0, 0b00, 0b0101) => Some(Asisdelem::FmlsRhH),
        (0b0, 0b00, 0b1001) => Some(Asisdelem::FmulRhH),
        (0b1, 0b00, 0b1001) => Some(Asisdelem::FmulxRhH),
        _ => None,
    }
}

pub(super) fn asisdsame2(word: u32) -> Option<Asisdsame2> {
    let u = bits(word, 29, 29);
    let opcode = bits(word, 14, 11);
    // The other opcodes are unallocated.
    match (u, opcode) {
        (0b1, 0b0000) => Some(Asisdsame2::Sqrdmlah),
        (0b1, 0b0001) => Some(Asisdsame2::Sqrdmlsh),
        _ => None,
    }
}

/// Arm group `asisdsamefp16`: Advanced SIMD scalar three same FP16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisdsamefp16 {
    Fmulx,
    Fcmeq,
    Frecps,
    Frsqrts,
    Fcmge,
    Facge,
    Fabd,
    Fcmgt,
    Facgt,
}

pub(super) fn asisdsamefp16(word: u32) -> Option<Asisdsamefp16> {
    let u = bits(word, 29, 29);
    let a = bits(word, 23, 23);
    let opcode = bits(word, 13, 11);
    match (u, a, opcode) {
        (0b0, 0b0, 0b011) => Some(Asisdsamefp16::Fmulx),
        (0b0, 0b0, 0b100) => Some(Asisdsamefp16::Fcmeq),
        (0b0, 0b0, 0b111) => Some(Asisdsamefp16::Frecps),
        (0b0, 0b1, 0b111) => Some(Asisdsamefp16::Frsqrts),
        (0b1, 0b0, 0b100) => Some(Asisdsamefp16::Fcmge),
        (0b1, 0b0, 0b101) => Some(Asisdsamefp16::Facge),
        (0b1, 0b1, 0b010) => Some(Asisdsamefp16::Fabd),
        (0b1, 0b1, 0b100) => Some(Asisdsamefp16::Fcmgt),
        (0b1, 0b1, 0b101) => Some(Asisdsamefp16::Facgt),
        _ => None,
    }
}

impl Asisdsamefp16 {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Fmulx
            | Self::Fcmeq
            | Self::Frecps
            | Self::Frsqrts
            | Self::Fcmge
            | Self::Facge
            | Self::Fabd
            | Self::Fcmgt
            | Self::Facgt => Feature::Fp16,
        }
    }
}

/// Arm group `asisdmiscfp16`: Advanced SIMD scalar two-register miscellaneous FP16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisdmiscfp16 {
    FcvtnsR,
    FcvtmsR,
    FcvtasR,
    ScvtfR,
    FcmgtFz,
    FcmeqFz,
    FcmltFz,
    FcvtpsR,
    FcvtzsR,
    FrecpeR,
    FrecpxR,
    FcvtnuR,
    FcvtmuR,
    FcvtauR,
    UcvtfR,
    FcmgeFz,
    FcmleFz,
    FcvtpuR,
    FcvtzuR,
    FrsqrteR,
}

pub(super) fn asisdmiscfp16(word: u32) -> Option<Asisdmiscfp16> {
    let u = bits(word, 29, 29);
    let a = bits(word, 23, 23);
    let opcode = bits(word, 16, 12);
    match (u, a, opcode) {
        (0b0, 0b0, 0b11010) => Some(Asisdmiscfp16::FcvtnsR),
        (0b0, 0b0, 0b11011) => Some(Asisdmiscfp16::FcvtmsR),
        (0b0, 0b0, 0b11100) => Some(Asisdmiscfp16::FcvtasR),
        (0b0, 0b0, 0b11101) => Some(Asisdmiscfp16::ScvtfR),
        (0b0, 0b1, 0b01100) => Some(Asisdmiscfp16::FcmgtFz),
        (0b0, 0b1, 0b01101) => Some(Asisdmiscfp16::FcmeqFz),
        (0b0, 0b1, 0b01110) => Some(Asisdmiscfp16::FcmltFz),
        (0b0, 0b1, 0b11010) => Some(Asisdmiscfp16::FcvtpsR),
        (0b0, 0b1, 0b11011) => Some(Asisdmiscfp16::FcvtzsR),
        (0b0, 0b1, 0b11101) => Some(Asisdmiscfp16::FrecpeR),
        (0b0, 0b1, 0b11111) => Some(Asisdmiscfp16::FrecpxR),
        (0b1, 0b0, 0b11010) => Some(Asisdmiscfp16::FcvtnuR),
        (0b1, 0b0, 0b11011) => Some(Asisdmiscfp16::FcvtmuR),
        (0b1, 0b0, 0b11100) => Some(Asisdmiscfp16::FcvtauR),
        (0b1, 0b0, 0b11101) => Some(Asisdmiscfp16::UcvtfR),
        (0b1, 0b1, 0b01100) => Some(Asisdmiscfp16::FcmgeFz),
        (0b1, 0b1, 0b01101) => Some(Asisdmiscfp16::FcmleFz),
        (0b1, 0b1, 0b11010) => Some(Asisdmiscfp16::FcvtpuR),
        (0b1, 0b1, 0b11011) => Some(Asisdmiscfp16::FcvtzuR),
        (0b1, 0b1, 0b11101) => Some(Asisdmiscfp16::FrsqrteR),
        _ => None,
    }
}

impl Asisdmiscfp16 {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::FcvtnsR
            | Self::FcvtmsR
            | Self::FcvtasR
            | Self::ScvtfR
            | Self::FcmgtFz
            | Self::FcmeqFz
            | Self::FcmltFz
            | Self::FcvtpsR
            | Self::FcvtzsR
            | Self::FrecpeR
            | Self::FrecpxR
            | Self::FcvtnuR
            | Self::FcvtmuR
            | Self::FcvtauR
            | Self::UcvtfR
            | Self::FcmgeFz
            | Self::FcmleFz
            | Self::FcvtpuR
            | Self::FcvtzuR
            | Self::FrsqrteR => Feature::Fp16,
        }
    }
}
