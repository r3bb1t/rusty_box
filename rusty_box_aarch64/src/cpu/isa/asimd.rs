//! The Advanced SIMD vector groups of Arm group `simd_dp` (Arm ARM C4.1,
//! "Data Processing -- Scalar Floating-Point and Advanced SIMD", the rows
//! with `op0 = 0xx0`).

use super::{Class, Feature};
use crate::cpu::exec::bits;

/// Arm group `asimdtbl`: Advanced SIMD table lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimdtbl {
    TblL11,
    TbxL11,
    TblL22,
    TbxL22,
    TblL33,
    TbxL33,
    TblL44,
    TbxL44,
}

/// Arm group `asimdperm`: Advanced SIMD permute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimdperm {
    Uzp1,
    Trn1,
    Zip1,
    Uzp2,
    Trn2,
    Zip2,
}

/// Arm group `asimdext`: Advanced SIMD extract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimdext {
    Ext,
}

/// Arm group `asimdins`: Advanced SIMD copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimdins {
    DupDvV,
    DupDrR,
    SmovWW,
    UmovWW,
    InsIrR,
    SmovXX,
    UmovXX,
    InsIvV,
}

/// Arm group `asimdmisc`: Advanced SIMD two-register miscellaneous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimdmisc {
    Rev64R,
    Rev16R,
    SaddlpP,
    SuqaddR,
    ClsR,
    CntR,
    SadalpP,
    SqabsR,
    CmgtZ,
    CmeqZ,
    CmltZ,
    AbsR,
    XtnN,
    SqxtnN,
    FcvtnN,
    FcvtlL,
    FrintnR,
    FrintmR,
    FcvtnsR,
    FcvtmsR,
    FcvtasR,
    ScvtfR,
    FcmgtFz,
    FcmeqFz,
    FcmltFz,
    FabsR,
    FrintpR,
    FrintzR,
    FcvtpsR,
    FcvtzsR,
    UrecpeR,
    FrecpeR,
    Rev32R,
    UaddlpP,
    UsqaddR,
    ClzR,
    UadalpP,
    SqnegR,
    CmgeZ,
    CmleZ,
    NegR,
    SqxtunN,
    ShllS,
    UqxtnN,
    FcvtxnN,
    FrintaR,
    FrintxR,
    FcvtnuR,
    FcvtmuR,
    FcvtauR,
    UcvtfR,
    NotR,
    RbitR,
    FcmgeFz,
    FcmleFz,
    FnegR,
    FrintiR,
    FcvtpuR,
    FcvtzuR,
    UrsqrteR,
    FrsqrteR,
    FsqrtR,
}

/// Arm group `asimdall`: Advanced SIMD across lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimdall {
    Saddlv,
    Smaxv,
    Sminv,
    Addv,
    FmaxnmvH,
    FmaxvH,
    FminnmvH,
    FminvH,
    Uaddlv,
    Umaxv,
    Uminv,
    FmaxnmvSd,
    FmaxvSd,
    FminnmvSd,
    FminvSd,
}

impl Asimdall {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Saddlv
            | Self::Smaxv
            | Self::Sminv
            | Self::Addv
            | Self::Uaddlv
            | Self::Umaxv
            | Self::Uminv
            | Self::FmaxnmvSd
            | Self::FmaxvSd
            | Self::FminnmvSd
            | Self::FminvSd => Feature::AdvSimd,
            Self::FmaxnmvH | Self::FmaxvH | Self::FminnmvH | Self::FminvH => Feature::Fp16,
        }
    }
}

/// Arm group `asimddiff`: Advanced SIMD three different.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimddiff {
    SaddlL,
    SaddwW,
    SsublL,
    SsubwW,
    AddhnN,
    SabalL,
    SubhnN,
    SabdlL,
    SmlalL,
    SqdmlalL,
    SmlslL,
    SqdmlslL,
    SmullL,
    SqdmullL,
    PmullL,
    UaddlL,
    UaddwW,
    UsublL,
    UsubwW,
    RaddhnN,
    UabalL,
    RsubhnN,
    UabdlL,
    UmlalL,
    UmlslL,
    UmullL,
}

/// Arm group `asimdsame`: Advanced SIMD three same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimdsame {
    Shadd,
    Sqadd,
    Srhadd,
    Shsub,
    Sqsub,
    Cmgt,
    Cmge,
    Sshl,
    Sqshl,
    Srshl,
    Sqrshl,
    Smax,
    Smin,
    Sabd,
    Saba,
    Add,
    Cmtst,
    Mla,
    Mul,
    Smaxp,
    Sminp,
    Sqdmulh,
    Addp,
    Fmaxnm,
    Fmla,
    Fadd,
    Fmulx,
    Fcmeq,
    Fmax,
    Frecps,
    And,
    Bic,
    Fminnm,
    Fmls,
    Fsub,
    Fmin,
    Frsqrts,
    Orr,
    Orn,
    Uhadd,
    Uqadd,
    Urhadd,
    Uhsub,
    Uqsub,
    Cmhi,
    Cmhs,
    Ushl,
    Uqshl,
    Urshl,
    Uqrshl,
    Umax,
    Umin,
    Uabd,
    Uaba,
    Sub,
    Cmeq,
    Mls,
    Pmul,
    Umaxp,
    Uminp,
    Sqrdmulh,
    Fmaxnmp,
    Faddp,
    Fmul,
    Fcmge,
    Facge,
    Fmaxp,
    Fdiv,
    Eor,
    Bsl,
    Fminnmp,
    Fabd,
    Fcmgt,
    Facgt,
    Fminp,
    Bit,
    Bif,
    FmlalF,
    FmlslF,
    Fmlal2F,
    Fmlsl2F,
}

impl Asimdsame {
    pub(crate) const fn class(self) -> Class {
        match self {
            Self::Shadd
            | Self::Sqadd
            | Self::Srhadd
            | Self::Shsub
            | Self::Sqsub
            | Self::Cmgt
            | Self::Cmge
            | Self::Sshl
            | Self::Sqshl
            | Self::Srshl
            | Self::Sqrshl
            | Self::Smax
            | Self::Smin
            | Self::Sabd
            | Self::Saba
            | Self::Add
            | Self::Cmtst
            | Self::Mla
            | Self::Mul
            | Self::Smaxp
            | Self::Sminp
            | Self::Sqdmulh
            | Self::Addp
            | Self::Fmaxnm
            | Self::Fmla
            | Self::Fadd
            | Self::Fmulx
            | Self::Fcmeq
            | Self::Fmax
            | Self::Frecps
            | Self::And
            | Self::Bic
            | Self::Fminnm
            | Self::Fmls
            | Self::Fsub
            | Self::Fmin
            | Self::Frsqrts
            | Self::Orr
            | Self::Orn
            | Self::Uhadd
            | Self::Uqadd
            | Self::Urhadd
            | Self::Uhsub
            | Self::Uqsub
            | Self::Cmhi
            | Self::Cmhs
            | Self::Ushl
            | Self::Uqshl
            | Self::Urshl
            | Self::Uqrshl
            | Self::Umax
            | Self::Umin
            | Self::Uabd
            | Self::Uaba
            | Self::Sub
            | Self::Cmeq
            | Self::Mls
            | Self::Pmul
            | Self::Umaxp
            | Self::Uminp
            | Self::Sqrdmulh
            | Self::Fmaxnmp
            | Self::Faddp
            | Self::Fmul
            | Self::Fcmge
            | Self::Facge
            | Self::Fmaxp
            | Self::Fdiv
            | Self::Eor
            | Self::Bsl
            | Self::Fminnmp
            | Self::Fabd
            | Self::Fcmgt
            | Self::Facgt
            | Self::Fminp
            | Self::Bit
            | Self::Bif => Class::SimdThreeSame,
            Self::FmlalF | Self::FmlslF | Self::Fmlal2F | Self::Fmlsl2F => Class::SimdFpMulAddLong,
        }
    }

    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Shadd
            | Self::Sqadd
            | Self::Srhadd
            | Self::Shsub
            | Self::Sqsub
            | Self::Cmgt
            | Self::Cmge
            | Self::Sshl
            | Self::Sqshl
            | Self::Srshl
            | Self::Sqrshl
            | Self::Smax
            | Self::Smin
            | Self::Sabd
            | Self::Saba
            | Self::Add
            | Self::Cmtst
            | Self::Mla
            | Self::Mul
            | Self::Smaxp
            | Self::Sminp
            | Self::Sqdmulh
            | Self::Addp
            | Self::Fmaxnm
            | Self::Fmla
            | Self::Fadd
            | Self::Fmulx
            | Self::Fcmeq
            | Self::Fmax
            | Self::Frecps
            | Self::And
            | Self::Bic
            | Self::Fminnm
            | Self::Fmls
            | Self::Fsub
            | Self::Fmin
            | Self::Frsqrts
            | Self::Orr
            | Self::Orn
            | Self::Uhadd
            | Self::Uqadd
            | Self::Urhadd
            | Self::Uhsub
            | Self::Uqsub
            | Self::Cmhi
            | Self::Cmhs
            | Self::Ushl
            | Self::Uqshl
            | Self::Urshl
            | Self::Uqrshl
            | Self::Umax
            | Self::Umin
            | Self::Uabd
            | Self::Uaba
            | Self::Sub
            | Self::Cmeq
            | Self::Mls
            | Self::Pmul
            | Self::Umaxp
            | Self::Uminp
            | Self::Sqrdmulh
            | Self::Fmaxnmp
            | Self::Faddp
            | Self::Fmul
            | Self::Fcmge
            | Self::Facge
            | Self::Fmaxp
            | Self::Fdiv
            | Self::Eor
            | Self::Bsl
            | Self::Fminnmp
            | Self::Fabd
            | Self::Fcmgt
            | Self::Facgt
            | Self::Fminp
            | Self::Bit
            | Self::Bif => Feature::AdvSimd,
            Self::FmlalF | Self::FmlslF | Self::Fmlal2F | Self::Fmlsl2F => Feature::Fhm,
        }
    }
}

/// Arm group `asimdimm`: Advanced SIMD modified immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimdimm {
    MoviLSl,
    OrrLSl,
    MoviLHl,
    OrrLHl,
    MoviMSm,
    MoviNB,
    FmovSS,
    MvniLSl,
    BicLSl,
    MvniLHl,
    BicLHl,
    MvniMSm,
    MoviDDs,
    MoviD2D,
    FmovD2D,
    FmovHH,
}

impl Asimdimm {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::MoviLSl
            | Self::OrrLSl
            | Self::MoviLHl
            | Self::OrrLHl
            | Self::MoviMSm
            | Self::MoviNB
            | Self::FmovSS
            | Self::MvniLSl
            | Self::BicLSl
            | Self::MvniLHl
            | Self::BicLHl
            | Self::MvniMSm
            | Self::MoviDDs
            | Self::MoviD2D
            | Self::FmovD2D => Feature::AdvSimd,
            Self::FmovHH => Feature::Fp16,
        }
    }
}

/// Arm group `asimdshf`: Advanced SIMD shift by immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimdshf {
    SshrR,
    SsraR,
    SrshrR,
    SrsraR,
    ShlR,
    SqshlR,
    ShrnN,
    RshrnN,
    SqshrnN,
    SqrshrnN,
    SshllL,
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
    UshllL,
    UcvtfC,
    FcvtzuC,
}

/// Arm group `asimdelem`: Advanced SIMD vector x indexed element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimdelem {
    SmlalL,
    SqdmlalL,
    SmlslL,
    SqdmlslL,
    MulR,
    SmullL,
    SqdmullL,
    SqdmulhR,
    SqrdmulhR,
    FmlaRSd,
    FmlsRSd,
    FmulRSd,
    MlaR,
    UmlalL,
    MlsR,
    UmlslL,
    UmullL,
    FmulxRSd,
    SdotD,
    SudotD,
    UsdotD,
    SqrdmlahR,
    UdotD,
    SqrdmlshR,
    FmlaRhH,
    FmlsRhH,
    FmulRhH,
    FmulxRhH,
    FmlalLh,
    FmlslLh,
    Fmlal2Lh,
    Fmlsl2Lh,
}

impl Asimdelem {
    pub(crate) const fn class(self) -> Class {
        match self {
            Self::SmlalL
            | Self::SqdmlalL
            | Self::SmlslL
            | Self::SqdmlslL
            | Self::MulR
            | Self::SmullL
            | Self::SqdmullL
            | Self::SqdmulhR
            | Self::SqrdmulhR
            | Self::FmlaRSd
            | Self::FmlsRSd
            | Self::FmulRSd
            | Self::MlaR
            | Self::UmlalL
            | Self::MlsR
            | Self::UmlslL
            | Self::UmullL
            | Self::FmulxRSd
            | Self::SqrdmlahR
            | Self::SqrdmlshR
            | Self::FmlaRhH
            | Self::FmlsRhH
            | Self::FmulRhH
            | Self::FmulxRhH => Class::SimdIndexedElement,
            Self::SdotD | Self::UdotD | Self::SudotD | Self::UsdotD => Class::SimdDotProductIndexedElement,
            Self::FmlalLh | Self::FmlslLh | Self::Fmlal2Lh | Self::Fmlsl2Lh => Class::SimdFpMulAddLongIndexedElement,
        }
    }

    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::SmlalL
            | Self::SqdmlalL
            | Self::SmlslL
            | Self::SqdmlslL
            | Self::MulR
            | Self::SmullL
            | Self::SqdmullL
            | Self::SqdmulhR
            | Self::SqrdmulhR
            | Self::FmlaRSd
            | Self::FmlsRSd
            | Self::FmulRSd
            | Self::MlaR
            | Self::UmlalL
            | Self::MlsR
            | Self::UmlslL
            | Self::UmullL
            | Self::FmulxRSd => Feature::AdvSimd,
            Self::SdotD | Self::UdotD => Feature::DotProd,
            Self::SudotD | Self::UsdotD => Feature::I8mm,
            Self::SqrdmlahR | Self::SqrdmlshR => Feature::Rdm,
            Self::FmlaRhH | Self::FmlsRhH | Self::FmulRhH | Self::FmulxRhH => Feature::Fp16,
            Self::FmlalLh | Self::FmlslLh | Self::Fmlal2Lh | Self::Fmlsl2Lh => Feature::Fhm,
        }
    }
}

/// Arm group `asimdsame2`: Advanced SIMD three-register extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimdsame2 {
    SdotD,
    UsdotD,
    Sqrdmlah,
    Sqrdmlsh,
    UdotD,
    SmmlaG,
    UsmmlaG,
    UmmlaG,
}

impl Asimdsame2 {
    pub(crate) const fn class(self) -> Class {
        match self {
            Self::Sqrdmlah | Self::Sqrdmlsh => Class::SimdRoundingDoublingMultiplyAccumulate,
            Self::SdotD | Self::UdotD | Self::UsdotD => Class::SimdDotProduct,
            Self::SmmlaG | Self::UsmmlaG | Self::UmmlaG => Class::SimdMatrixMultiply,
        }
    }

    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::SdotD | Self::UdotD => Feature::DotProd,
            Self::UsdotD | Self::SmmlaG | Self::UsmmlaG | Self::UmmlaG => Feature::I8mm,
            Self::Sqrdmlah | Self::Sqrdmlsh => Feature::Rdm,
        }
    }
}

pub(super) fn asimdtbl(word: u32) -> Option<Asimdtbl> {
    let op2 = bits(word, 23, 22);
    let len = bits(word, 14, 13);
    let op = bits(word, 12, 12);
    // op2 = 01, 10 and 11 hold LUTI2 and LUTI4 (FEAT_LUT).
    match (op2, len, op) {
        (0b00, 0b00, 0b0) => Some(Asimdtbl::TblL11),
        (0b00, 0b00, 0b1) => Some(Asimdtbl::TbxL11),
        (0b00, 0b01, 0b0) => Some(Asimdtbl::TblL22),
        (0b00, 0b01, 0b1) => Some(Asimdtbl::TbxL22),
        (0b00, 0b10, 0b0) => Some(Asimdtbl::TblL33),
        (0b00, 0b10, 0b1) => Some(Asimdtbl::TbxL33),
        (0b00, 0b11, 0b0) => Some(Asimdtbl::TblL44),
        (0b00, 0b11, 0b1) => Some(Asimdtbl::TbxL44),
        _ => None,
    }
}

pub(super) fn asimdperm(word: u32) -> Option<Asimdperm> {
    let opcode = bits(word, 14, 12);
    match opcode {
        0b001 => Some(Asimdperm::Uzp1),
        0b010 => Some(Asimdperm::Trn1),
        0b011 => Some(Asimdperm::Zip1),
        0b101 => Some(Asimdperm::Uzp2),
        0b110 => Some(Asimdperm::Trn2),
        0b111 => Some(Asimdperm::Zip2),
        _ => None,
    }
}

pub(super) fn asimdext(word: u32) -> Option<Asimdext> {
    let op2 = bits(word, 23, 22);
    match op2 {
        0b00 => Some(Asimdext::Ext),
        _ => None,
    }
}

pub(super) fn asimdins(word: u32) -> Option<Asimdins> {
    let q = bits(word, 30, 30);
    let op = bits(word, 29, 29);
    let imm5 = bits(word, 20, 16);
    let imm4 = bits(word, 14, 11);
    match (q, op, imm5, imm4) {
        (_, 0b0, _, 0b0000) => Some(Asimdins::DupDvV),
        (_, 0b0, _, 0b0001) => Some(Asimdins::DupDrR),
        (0b0, 0b0, _, 0b0101) => Some(Asimdins::SmovWW),
        (0b0, 0b0, _, 0b0111) => Some(Asimdins::UmovWW),
        (0b1, 0b0, _, 0b0011) => Some(Asimdins::InsIrR),
        (0b1, 0b0, _, 0b0101) => Some(Asimdins::SmovXX),
        (0b1, 0b0, 0b01000 | 0b11000, 0b0111) => Some(Asimdins::UmovXX),
        (0b1, 0b1, _, _) => Some(Asimdins::InsIvV),
        _ => None,
    }
}

pub(super) fn asimdmisc(word: u32) -> Option<Asimdmisc> {
    let u = bits(word, 29, 29);
    let size = bits(word, 23, 22);
    let opcode = bits(word, 16, 12);
    // FRINT32Z, FRINT64Z, FRINT32X and FRINT64X are FEAT_FRINTTS, BFCVTN
    // FEAT_BF16, and F1CVTL, F2CVTL, BF1CVTL and BF2CVTL FEAT_FP8.
    match (u, size, opcode) {
        (0b0, _, 0b00000) => Some(Asimdmisc::Rev64R),
        (0b0, _, 0b00001) => Some(Asimdmisc::Rev16R),
        (0b0, _, 0b00010) => Some(Asimdmisc::SaddlpP),
        (0b0, _, 0b00011) => Some(Asimdmisc::SuqaddR),
        (0b0, _, 0b00100) => Some(Asimdmisc::ClsR),
        (0b0, _, 0b00101) => Some(Asimdmisc::CntR),
        (0b0, _, 0b00110) => Some(Asimdmisc::SadalpP),
        (0b0, _, 0b00111) => Some(Asimdmisc::SqabsR),
        (0b0, _, 0b01000) => Some(Asimdmisc::CmgtZ),
        (0b0, _, 0b01001) => Some(Asimdmisc::CmeqZ),
        (0b0, _, 0b01010) => Some(Asimdmisc::CmltZ),
        (0b0, _, 0b01011) => Some(Asimdmisc::AbsR),
        (0b0, _, 0b10010) => Some(Asimdmisc::XtnN),
        (0b0, _, 0b10100) => Some(Asimdmisc::SqxtnN),
        (0b0, 0b00 | 0b01, 0b10110) => Some(Asimdmisc::FcvtnN),
        (0b0, 0b00 | 0b01, 0b10111) => Some(Asimdmisc::FcvtlL),
        (0b0, 0b00 | 0b01, 0b11000) => Some(Asimdmisc::FrintnR),
        (0b0, 0b00 | 0b01, 0b11001) => Some(Asimdmisc::FrintmR),
        (0b0, 0b00 | 0b01, 0b11010) => Some(Asimdmisc::FcvtnsR),
        (0b0, 0b00 | 0b01, 0b11011) => Some(Asimdmisc::FcvtmsR),
        (0b0, 0b00 | 0b01, 0b11100) => Some(Asimdmisc::FcvtasR),
        (0b0, 0b00 | 0b01, 0b11101) => Some(Asimdmisc::ScvtfR),
        (0b0, 0b10 | 0b11, 0b01100) => Some(Asimdmisc::FcmgtFz),
        (0b0, 0b10 | 0b11, 0b01101) => Some(Asimdmisc::FcmeqFz),
        (0b0, 0b10 | 0b11, 0b01110) => Some(Asimdmisc::FcmltFz),
        (0b0, 0b10 | 0b11, 0b01111) => Some(Asimdmisc::FabsR),
        (0b0, 0b10 | 0b11, 0b11000) => Some(Asimdmisc::FrintpR),
        (0b0, 0b10 | 0b11, 0b11001) => Some(Asimdmisc::FrintzR),
        (0b0, 0b10 | 0b11, 0b11010) => Some(Asimdmisc::FcvtpsR),
        (0b0, 0b10 | 0b11, 0b11011) => Some(Asimdmisc::FcvtzsR),
        (0b0, 0b10 | 0b11, 0b11100) => Some(Asimdmisc::UrecpeR),
        (0b0, 0b10 | 0b11, 0b11101) => Some(Asimdmisc::FrecpeR),
        (0b1, _, 0b00000) => Some(Asimdmisc::Rev32R),
        (0b1, _, 0b00010) => Some(Asimdmisc::UaddlpP),
        (0b1, _, 0b00011) => Some(Asimdmisc::UsqaddR),
        (0b1, _, 0b00100) => Some(Asimdmisc::ClzR),
        (0b1, _, 0b00110) => Some(Asimdmisc::UadalpP),
        (0b1, _, 0b00111) => Some(Asimdmisc::SqnegR),
        (0b1, _, 0b01000) => Some(Asimdmisc::CmgeZ),
        (0b1, _, 0b01001) => Some(Asimdmisc::CmleZ),
        (0b1, _, 0b01011) => Some(Asimdmisc::NegR),
        (0b1, _, 0b10010) => Some(Asimdmisc::SqxtunN),
        (0b1, _, 0b10011) => Some(Asimdmisc::ShllS),
        (0b1, _, 0b10100) => Some(Asimdmisc::UqxtnN),
        (0b1, 0b01, 0b10110) => Some(Asimdmisc::FcvtxnN),
        (0b1, 0b00 | 0b01, 0b11000) => Some(Asimdmisc::FrintaR),
        (0b1, 0b00 | 0b01, 0b11001) => Some(Asimdmisc::FrintxR),
        (0b1, 0b00 | 0b01, 0b11010) => Some(Asimdmisc::FcvtnuR),
        (0b1, 0b00 | 0b01, 0b11011) => Some(Asimdmisc::FcvtmuR),
        (0b1, 0b00 | 0b01, 0b11100) => Some(Asimdmisc::FcvtauR),
        (0b1, 0b00 | 0b01, 0b11101) => Some(Asimdmisc::UcvtfR),
        (0b1, 0b00, 0b00101) => Some(Asimdmisc::NotR),
        (0b1, 0b01, 0b00101) => Some(Asimdmisc::RbitR),
        (0b1, 0b10 | 0b11, 0b01100) => Some(Asimdmisc::FcmgeFz),
        (0b1, 0b10 | 0b11, 0b01101) => Some(Asimdmisc::FcmleFz),
        (0b1, 0b10 | 0b11, 0b01111) => Some(Asimdmisc::FnegR),
        (0b1, 0b10 | 0b11, 0b11001) => Some(Asimdmisc::FrintiR),
        (0b1, 0b10 | 0b11, 0b11010) => Some(Asimdmisc::FcvtpuR),
        (0b1, 0b10 | 0b11, 0b11011) => Some(Asimdmisc::FcvtzuR),
        (0b1, 0b10 | 0b11, 0b11100) => Some(Asimdmisc::UrsqrteR),
        (0b1, 0b10 | 0b11, 0b11101) => Some(Asimdmisc::FrsqrteR),
        (0b1, 0b10 | 0b11, 0b11111) => Some(Asimdmisc::FsqrtR),
        _ => None,
    }
}

pub(super) fn asimdall(word: u32) -> Option<Asimdall> {
    let q = bits(word, 30, 30);
    let u = bits(word, 29, 29);
    let size = bits(word, 23, 22);
    let opcode = bits(word, 16, 12);
    match (u, opcode) {
        (0b0, 0b00011) => Some(Asimdall::Saddlv),
        (0b0, 0b01010) => Some(Asimdall::Smaxv),
        (0b0, 0b11010) => Some(Asimdall::Sminv),
        (0b0, 0b11011) => Some(Asimdall::Addv),
        (0b0, 0b01100) => match size {
            0b00 => Some(Asimdall::FmaxnmvH),
            0b10 => Some(Asimdall::FminnmvH),
            _ => None,
        },
        (0b0, 0b01111) => match size {
            0b00 => Some(Asimdall::FmaxvH),
            0b10 => Some(Asimdall::FminvH),
            _ => None,
        },
        (0b1, 0b00011) => Some(Asimdall::Uaddlv),
        (0b1, 0b01010) => Some(Asimdall::Umaxv),
        (0b1, 0b11010) => Some(Asimdall::Uminv),
        (0b1, 0b01100) => match (q, size) {
            (0b1, 0b00) => Some(Asimdall::FmaxnmvSd),
            (0b1, 0b10) => Some(Asimdall::FminnmvSd),
            _ => None,
        },
        (0b1, 0b01111) => match (q, size) {
            (0b1, 0b00) => Some(Asimdall::FmaxvSd),
            (0b1, 0b10) => Some(Asimdall::FminvSd),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn asimddiff(word: u32) -> Option<Asimddiff> {
    let u = bits(word, 29, 29);
    let opcode = bits(word, 15, 12);
    match (u, opcode) {
        (0b0, 0b0000) => Some(Asimddiff::SaddlL),
        (0b0, 0b0001) => Some(Asimddiff::SaddwW),
        (0b0, 0b0010) => Some(Asimddiff::SsublL),
        (0b0, 0b0011) => Some(Asimddiff::SsubwW),
        (0b0, 0b0100) => Some(Asimddiff::AddhnN),
        (0b0, 0b0101) => Some(Asimddiff::SabalL),
        (0b0, 0b0110) => Some(Asimddiff::SubhnN),
        (0b0, 0b0111) => Some(Asimddiff::SabdlL),
        (0b0, 0b1000) => Some(Asimddiff::SmlalL),
        (0b0, 0b1001) => Some(Asimddiff::SqdmlalL),
        (0b0, 0b1010) => Some(Asimddiff::SmlslL),
        (0b0, 0b1011) => Some(Asimddiff::SqdmlslL),
        (0b0, 0b1100) => Some(Asimddiff::SmullL),
        (0b0, 0b1101) => Some(Asimddiff::SqdmullL),
        (0b0, 0b1110) => Some(Asimddiff::PmullL),
        (0b1, 0b0000) => Some(Asimddiff::UaddlL),
        (0b1, 0b0001) => Some(Asimddiff::UaddwW),
        (0b1, 0b0010) => Some(Asimddiff::UsublL),
        (0b1, 0b0011) => Some(Asimddiff::UsubwW),
        (0b1, 0b0100) => Some(Asimddiff::RaddhnN),
        (0b1, 0b0101) => Some(Asimddiff::UabalL),
        (0b1, 0b0110) => Some(Asimddiff::RsubhnN),
        (0b1, 0b0111) => Some(Asimddiff::UabdlL),
        (0b1, 0b1000) => Some(Asimddiff::UmlalL),
        (0b1, 0b1010) => Some(Asimddiff::UmlslL),
        (0b1, 0b1100) => Some(Asimddiff::UmullL),
        _ => None,
    }
}

pub(super) fn asimdsame(word: u32) -> Option<Asimdsame> {
    let u = bits(word, 29, 29);
    let size = bits(word, 23, 22);
    let opcode = bits(word, 15, 11);
    // FSCALE is FEAT_FP8.
    match (u, size, opcode) {
        (0b0, _, 0b00000) => Some(Asimdsame::Shadd),
        (0b0, _, 0b00001) => Some(Asimdsame::Sqadd),
        (0b0, _, 0b00010) => Some(Asimdsame::Srhadd),
        (0b0, _, 0b00100) => Some(Asimdsame::Shsub),
        (0b0, _, 0b00101) => Some(Asimdsame::Sqsub),
        (0b0, _, 0b00110) => Some(Asimdsame::Cmgt),
        (0b0, _, 0b00111) => Some(Asimdsame::Cmge),
        (0b0, _, 0b01000) => Some(Asimdsame::Sshl),
        (0b0, _, 0b01001) => Some(Asimdsame::Sqshl),
        (0b0, _, 0b01010) => Some(Asimdsame::Srshl),
        (0b0, _, 0b01011) => Some(Asimdsame::Sqrshl),
        (0b0, _, 0b01100) => Some(Asimdsame::Smax),
        (0b0, _, 0b01101) => Some(Asimdsame::Smin),
        (0b0, _, 0b01110) => Some(Asimdsame::Sabd),
        (0b0, _, 0b01111) => Some(Asimdsame::Saba),
        (0b0, _, 0b10000) => Some(Asimdsame::Add),
        (0b0, _, 0b10001) => Some(Asimdsame::Cmtst),
        (0b0, _, 0b10010) => Some(Asimdsame::Mla),
        (0b0, _, 0b10011) => Some(Asimdsame::Mul),
        (0b0, _, 0b10100) => Some(Asimdsame::Smaxp),
        (0b0, _, 0b10101) => Some(Asimdsame::Sminp),
        (0b0, _, 0b10110) => Some(Asimdsame::Sqdmulh),
        (0b0, _, 0b10111) => Some(Asimdsame::Addp),
        (0b0, 0b00 | 0b01, 0b11000) => Some(Asimdsame::Fmaxnm),
        (0b0, 0b00 | 0b01, 0b11001) => Some(Asimdsame::Fmla),
        (0b0, 0b00 | 0b01, 0b11010) => Some(Asimdsame::Fadd),
        (0b0, 0b00 | 0b01, 0b11011) => Some(Asimdsame::Fmulx),
        (0b0, 0b00 | 0b01, 0b11100) => Some(Asimdsame::Fcmeq),
        (0b0, 0b00 | 0b01, 0b11110) => Some(Asimdsame::Fmax),
        (0b0, 0b00 | 0b01, 0b11111) => Some(Asimdsame::Frecps),
        (0b0, 0b00, 0b00011) => Some(Asimdsame::And),
        (0b0, 0b01, 0b00011) => Some(Asimdsame::Bic),
        (0b0, 0b10 | 0b11, 0b11000) => Some(Asimdsame::Fminnm),
        (0b0, 0b10 | 0b11, 0b11001) => Some(Asimdsame::Fmls),
        (0b0, 0b10 | 0b11, 0b11010) => Some(Asimdsame::Fsub),
        (0b0, 0b10 | 0b11, 0b11110) => Some(Asimdsame::Fmin),
        (0b0, 0b10 | 0b11, 0b11111) => Some(Asimdsame::Frsqrts),
        (0b0, 0b10, 0b00011) => Some(Asimdsame::Orr),
        (0b0, 0b11, 0b00011) => Some(Asimdsame::Orn),
        (0b1, _, 0b00000) => Some(Asimdsame::Uhadd),
        (0b1, _, 0b00001) => Some(Asimdsame::Uqadd),
        (0b1, _, 0b00010) => Some(Asimdsame::Urhadd),
        (0b1, _, 0b00100) => Some(Asimdsame::Uhsub),
        (0b1, _, 0b00101) => Some(Asimdsame::Uqsub),
        (0b1, _, 0b00110) => Some(Asimdsame::Cmhi),
        (0b1, _, 0b00111) => Some(Asimdsame::Cmhs),
        (0b1, _, 0b01000) => Some(Asimdsame::Ushl),
        (0b1, _, 0b01001) => Some(Asimdsame::Uqshl),
        (0b1, _, 0b01010) => Some(Asimdsame::Urshl),
        (0b1, _, 0b01011) => Some(Asimdsame::Uqrshl),
        (0b1, _, 0b01100) => Some(Asimdsame::Umax),
        (0b1, _, 0b01101) => Some(Asimdsame::Umin),
        (0b1, _, 0b01110) => Some(Asimdsame::Uabd),
        (0b1, _, 0b01111) => Some(Asimdsame::Uaba),
        (0b1, _, 0b10000) => Some(Asimdsame::Sub),
        (0b1, _, 0b10001) => Some(Asimdsame::Cmeq),
        (0b1, _, 0b10010) => Some(Asimdsame::Mls),
        (0b1, _, 0b10011) => Some(Asimdsame::Pmul),
        (0b1, _, 0b10100) => Some(Asimdsame::Umaxp),
        (0b1, _, 0b10101) => Some(Asimdsame::Uminp),
        (0b1, _, 0b10110) => Some(Asimdsame::Sqrdmulh),
        (0b1, 0b00 | 0b01, 0b11000) => Some(Asimdsame::Fmaxnmp),
        (0b1, 0b00 | 0b01, 0b11010) => Some(Asimdsame::Faddp),
        (0b1, 0b00 | 0b01, 0b11011) => Some(Asimdsame::Fmul),
        (0b1, 0b00 | 0b01, 0b11100) => Some(Asimdsame::Fcmge),
        (0b1, 0b00 | 0b01, 0b11101) => Some(Asimdsame::Facge),
        (0b1, 0b00 | 0b01, 0b11110) => Some(Asimdsame::Fmaxp),
        (0b1, 0b00 | 0b01, 0b11111) => Some(Asimdsame::Fdiv),
        (0b1, 0b00, 0b00011) => Some(Asimdsame::Eor),
        (0b1, 0b01, 0b00011) => Some(Asimdsame::Bsl),
        (0b1, 0b10 | 0b11, 0b11000) => Some(Asimdsame::Fminnmp),
        (0b1, 0b10 | 0b11, 0b11010) => Some(Asimdsame::Fabd),
        (0b1, 0b10 | 0b11, 0b11100) => Some(Asimdsame::Fcmgt),
        (0b1, 0b10 | 0b11, 0b11101) => Some(Asimdsame::Facgt),
        (0b1, 0b10 | 0b11, 0b11110) => Some(Asimdsame::Fminp),
        (0b1, 0b10, 0b00011) => Some(Asimdsame::Bit),
        (0b1, 0b11, 0b00011) => Some(Asimdsame::Bif),
        (0b0, 0b00, 0b11101) => Some(Asimdsame::FmlalF),
        (0b0, 0b10, 0b11101) => Some(Asimdsame::FmlslF),
        (0b1, 0b00, 0b11001) => Some(Asimdsame::Fmlal2F),
        (0b1, 0b10, 0b11001) => Some(Asimdsame::Fmlsl2F),
        _ => None,
    }
}

pub(super) fn asimdimm(word: u32) -> Option<Asimdimm> {
    let q = bits(word, 30, 30);
    let op = bits(word, 29, 29);
    let cmode = bits(word, 15, 12);
    let o2 = bits(word, 11, 11);
    match (q, op, cmode, o2) {
        (_, 0b0, 0b0000 | 0b0010 | 0b0100 | 0b0110, 0b0) => Some(Asimdimm::MoviLSl),
        (_, 0b0, 0b0001 | 0b0011 | 0b0101 | 0b0111, 0b0) => Some(Asimdimm::OrrLSl),
        (_, 0b0, 0b1000 | 0b1010, 0b0) => Some(Asimdimm::MoviLHl),
        (_, 0b0, 0b1001 | 0b1011, 0b0) => Some(Asimdimm::OrrLHl),
        (_, 0b0, 0b1100 | 0b1101, 0b0) => Some(Asimdimm::MoviMSm),
        (_, 0b0, 0b1110, 0b0) => Some(Asimdimm::MoviNB),
        (_, 0b0, 0b1111, 0b0) => Some(Asimdimm::FmovSS),
        (_, 0b1, 0b0000 | 0b0010 | 0b0100 | 0b0110, 0b0) => Some(Asimdimm::MvniLSl),
        (_, 0b1, 0b0001 | 0b0011 | 0b0101 | 0b0111, 0b0) => Some(Asimdimm::BicLSl),
        (_, 0b1, 0b1000 | 0b1010, 0b0) => Some(Asimdimm::MvniLHl),
        (_, 0b1, 0b1001 | 0b1011, 0b0) => Some(Asimdimm::BicLHl),
        (_, 0b1, 0b1100 | 0b1101, 0b0) => Some(Asimdimm::MvniMSm),
        (0b0, 0b1, 0b1110, 0b0) => Some(Asimdimm::MoviDDs),
        (0b1, 0b1, 0b1110, 0b0) => Some(Asimdimm::MoviD2D),
        (0b1, 0b1, 0b1111, 0b0) => Some(Asimdimm::FmovD2D),
        (_, 0b0, 0b1111, 0b1) => Some(Asimdimm::FmovHH),
        _ => None,
    }
}

pub(super) fn asimdshf(word: u32) -> Option<Asimdshf> {
    let u = bits(word, 29, 29);
    let opcode = bits(word, 15, 11);
    match (u, opcode) {
        (0b0, 0b00000) => Some(Asimdshf::SshrR),
        (0b0, 0b00010) => Some(Asimdshf::SsraR),
        (0b0, 0b00100) => Some(Asimdshf::SrshrR),
        (0b0, 0b00110) => Some(Asimdshf::SrsraR),
        (0b0, 0b01010) => Some(Asimdshf::ShlR),
        (0b0, 0b01110) => Some(Asimdshf::SqshlR),
        (0b0, 0b10000) => Some(Asimdshf::ShrnN),
        (0b0, 0b10001) => Some(Asimdshf::RshrnN),
        (0b0, 0b10010) => Some(Asimdshf::SqshrnN),
        (0b0, 0b10011) => Some(Asimdshf::SqrshrnN),
        (0b0, 0b10100) => Some(Asimdshf::SshllL),
        (0b0, 0b11100) => Some(Asimdshf::ScvtfC),
        (0b0, 0b11111) => Some(Asimdshf::FcvtzsC),
        (0b1, 0b00000) => Some(Asimdshf::UshrR),
        (0b1, 0b00010) => Some(Asimdshf::UsraR),
        (0b1, 0b00100) => Some(Asimdshf::UrshrR),
        (0b1, 0b00110) => Some(Asimdshf::UrsraR),
        (0b1, 0b01000) => Some(Asimdshf::SriR),
        (0b1, 0b01010) => Some(Asimdshf::SliR),
        (0b1, 0b01100) => Some(Asimdshf::SqshluR),
        (0b1, 0b01110) => Some(Asimdshf::UqshlR),
        (0b1, 0b10000) => Some(Asimdshf::SqshrunN),
        (0b1, 0b10001) => Some(Asimdshf::SqrshrunN),
        (0b1, 0b10010) => Some(Asimdshf::UqshrnN),
        (0b1, 0b10011) => Some(Asimdshf::UqrshrnN),
        (0b1, 0b10100) => Some(Asimdshf::UshllL),
        (0b1, 0b11100) => Some(Asimdshf::UcvtfC),
        (0b1, 0b11111) => Some(Asimdshf::FcvtzuC),
        _ => None,
    }
}

pub(super) fn asimdelem(word: u32) -> Option<Asimdelem> {
    let u = bits(word, 29, 29);
    let size = bits(word, 23, 22);
    let opcode = bits(word, 15, 12);
    // The remaining opcodes are FEAT_FCMA, FEAT_BF16 and FEAT_FP8 forms.
    match (u, size, opcode) {
        (0b0, _, 0b0010) => Some(Asimdelem::SmlalL),
        (0b0, _, 0b0011) => Some(Asimdelem::SqdmlalL),
        (0b0, _, 0b0110) => Some(Asimdelem::SmlslL),
        (0b0, _, 0b0111) => Some(Asimdelem::SqdmlslL),
        (0b0, _, 0b1000) => Some(Asimdelem::MulR),
        (0b0, _, 0b1010) => Some(Asimdelem::SmullL),
        (0b0, _, 0b1011) => Some(Asimdelem::SqdmullL),
        (0b0, _, 0b1100) => Some(Asimdelem::SqdmulhR),
        (0b0, _, 0b1101) => Some(Asimdelem::SqrdmulhR),
        (0b0, 0b10 | 0b11, 0b0001) => Some(Asimdelem::FmlaRSd),
        (0b0, 0b10 | 0b11, 0b0101) => Some(Asimdelem::FmlsRSd),
        (0b0, 0b10 | 0b11, 0b1001) => Some(Asimdelem::FmulRSd),
        (0b1, _, 0b0000) => Some(Asimdelem::MlaR),
        (0b1, _, 0b0010) => Some(Asimdelem::UmlalL),
        (0b1, _, 0b0100) => Some(Asimdelem::MlsR),
        (0b1, _, 0b0110) => Some(Asimdelem::UmlslL),
        (0b1, _, 0b1010) => Some(Asimdelem::UmullL),
        (0b1, 0b10 | 0b11, 0b1001) => Some(Asimdelem::FmulxRSd),
        (0b0, _, 0b1110) => Some(Asimdelem::SdotD),
        (0b0, 0b00, 0b1111) => Some(Asimdelem::SudotD),
        (0b0, 0b10, 0b1111) => Some(Asimdelem::UsdotD),
        (0b1, _, 0b1101) => Some(Asimdelem::SqrdmlahR),
        (0b1, _, 0b1110) => Some(Asimdelem::UdotD),
        (0b1, _, 0b1111) => Some(Asimdelem::SqrdmlshR),
        (0b0, 0b00, 0b0001) => Some(Asimdelem::FmlaRhH),
        (0b0, 0b00, 0b0101) => Some(Asimdelem::FmlsRhH),
        (0b0, 0b00, 0b1001) => Some(Asimdelem::FmulRhH),
        (0b1, 0b00, 0b1001) => Some(Asimdelem::FmulxRhH),
        (0b0, 0b10, 0b0000) => Some(Asimdelem::FmlalLh),
        (0b0, 0b10, 0b0100) => Some(Asimdelem::FmlslLh),
        (0b1, 0b10, 0b1000) => Some(Asimdelem::Fmlal2Lh),
        (0b1, 0b10, 0b1100) => Some(Asimdelem::Fmlsl2Lh),
        _ => None,
    }
}

pub(super) fn asimdsame2(word: u32) -> Option<Asimdsame2> {
    let q = bits(word, 30, 30);
    let u = bits(word, 29, 29);
    let size = bits(word, 23, 22);
    let opcode = bits(word, 14, 11);
    // The other opcodes are FEAT_FCMA, FEAT_BF16 and FEAT_FP8 forms and
    // later matrix multiplies.
    match (u, opcode) {
        (0b0, 0b0010) => Some(Asimdsame2::SdotD),
        (0b0, 0b0011) => match size {
            0b10 => Some(Asimdsame2::UsdotD),
            _ => None,
        },
        (0b1, 0b0000) => Some(Asimdsame2::Sqrdmlah),
        (0b1, 0b0001) => Some(Asimdsame2::Sqrdmlsh),
        (0b1, 0b0010) => Some(Asimdsame2::UdotD),
        (0b0, 0b0100) => match (q, size) {
            (0b1, 0b10) => Some(Asimdsame2::SmmlaG),
            _ => None,
        },
        (0b0, 0b0101) => match (q, size) {
            (0b1, 0b10) => Some(Asimdsame2::UsmmlaG),
            _ => None,
        },
        (0b1, 0b0100) => match (q, size) {
            (0b1, 0b10) => Some(Asimdsame2::UmmlaG),
            _ => None,
        },
        _ => None,
    }
}

/// Arm group `asimdsamefp16`: Advanced SIMD three same (FP16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimdsamefp16 {
    Fmaxnm,
    Fmla,
    Fadd,
    Fmulx,
    Fcmeq,
    Fmax,
    Frecps,
    Fminnm,
    Fmls,
    Fsub,
    Fmin,
    Frsqrts,
    Fmaxnmp,
    Faddp,
    Fmul,
    Fcmge,
    Facge,
    Fmaxp,
    Fdiv,
    Fminnmp,
    Fabd,
    Fcmgt,
    Facgt,
    Fminp,
}

pub(super) fn asimdsamefp16(word: u32) -> Option<Asimdsamefp16> {
    let u = bits(word, 29, 29);
    let a = bits(word, 23, 23);
    let opcode = bits(word, 13, 11);
    match (u, a, opcode) {
        (0b0, 0b0, 0b000) => Some(Asimdsamefp16::Fmaxnm),
        (0b0, 0b0, 0b001) => Some(Asimdsamefp16::Fmla),
        (0b0, 0b0, 0b010) => Some(Asimdsamefp16::Fadd),
        (0b0, 0b0, 0b011) => Some(Asimdsamefp16::Fmulx),
        (0b0, 0b0, 0b100) => Some(Asimdsamefp16::Fcmeq),
        (0b0, 0b0, 0b110) => Some(Asimdsamefp16::Fmax),
        (0b0, 0b0, 0b111) => Some(Asimdsamefp16::Frecps),
        (0b0, 0b1, 0b000) => Some(Asimdsamefp16::Fminnm),
        (0b0, 0b1, 0b001) => Some(Asimdsamefp16::Fmls),
        (0b0, 0b1, 0b010) => Some(Asimdsamefp16::Fsub),
        (0b0, 0b1, 0b110) => Some(Asimdsamefp16::Fmin),
        (0b0, 0b1, 0b111) => Some(Asimdsamefp16::Frsqrts),
        (0b1, 0b0, 0b000) => Some(Asimdsamefp16::Fmaxnmp),
        (0b1, 0b0, 0b010) => Some(Asimdsamefp16::Faddp),
        (0b1, 0b0, 0b011) => Some(Asimdsamefp16::Fmul),
        (0b1, 0b0, 0b100) => Some(Asimdsamefp16::Fcmge),
        (0b1, 0b0, 0b101) => Some(Asimdsamefp16::Facge),
        (0b1, 0b0, 0b110) => Some(Asimdsamefp16::Fmaxp),
        (0b1, 0b0, 0b111) => Some(Asimdsamefp16::Fdiv),
        (0b1, 0b1, 0b000) => Some(Asimdsamefp16::Fminnmp),
        (0b1, 0b1, 0b010) => Some(Asimdsamefp16::Fabd),
        (0b1, 0b1, 0b100) => Some(Asimdsamefp16::Fcmgt),
        (0b1, 0b1, 0b101) => Some(Asimdsamefp16::Facgt),
        (0b1, 0b1, 0b110) => Some(Asimdsamefp16::Fminp),
        _ => None,
    }
}

impl Asimdsamefp16 {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Fmaxnm
            | Self::Fmla
            | Self::Fadd
            | Self::Fmulx
            | Self::Fcmeq
            | Self::Fmax
            | Self::Frecps
            | Self::Fminnm
            | Self::Fmls
            | Self::Fsub
            | Self::Fmin
            | Self::Frsqrts
            | Self::Fmaxnmp
            | Self::Faddp
            | Self::Fmul
            | Self::Fcmge
            | Self::Facge
            | Self::Fmaxp
            | Self::Fdiv
            | Self::Fminnmp
            | Self::Fabd
            | Self::Fcmgt
            | Self::Facgt
            | Self::Fminp => Feature::Fp16,
        }
    }
}

/// Arm group `asimdmiscfp16`: Advanced SIMD two-register miscellaneous (FP16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asimdmiscfp16 {
    FrintnR,
    FrintmR,
    FcvtnsR,
    FcvtmsR,
    FcvtasR,
    ScvtfR,
    FcmgtFz,
    FcmeqFz,
    FcmltFz,
    FabsR,
    FrintpR,
    FrintzR,
    FcvtpsR,
    FcvtzsR,
    FrecpeR,
    FrintaR,
    FrintxR,
    FcvtnuR,
    FcvtmuR,
    FcvtauR,
    UcvtfR,
    FcmgeFz,
    FcmleFz,
    FnegR,
    FrintiR,
    FcvtpuR,
    FcvtzuR,
    FrsqrteR,
    FsqrtR,
}

pub(super) fn asimdmiscfp16(word: u32) -> Option<Asimdmiscfp16> {
    let u = bits(word, 29, 29);
    let a = bits(word, 23, 23);
    let opcode = bits(word, 16, 12);
    match (u, a, opcode) {
        (0b0, 0b0, 0b11000) => Some(Asimdmiscfp16::FrintnR),
        (0b0, 0b0, 0b11001) => Some(Asimdmiscfp16::FrintmR),
        (0b0, 0b0, 0b11010) => Some(Asimdmiscfp16::FcvtnsR),
        (0b0, 0b0, 0b11011) => Some(Asimdmiscfp16::FcvtmsR),
        (0b0, 0b0, 0b11100) => Some(Asimdmiscfp16::FcvtasR),
        (0b0, 0b0, 0b11101) => Some(Asimdmiscfp16::ScvtfR),
        (0b0, 0b1, 0b01100) => Some(Asimdmiscfp16::FcmgtFz),
        (0b0, 0b1, 0b01101) => Some(Asimdmiscfp16::FcmeqFz),
        (0b0, 0b1, 0b01110) => Some(Asimdmiscfp16::FcmltFz),
        (0b0, 0b1, 0b01111) => Some(Asimdmiscfp16::FabsR),
        (0b0, 0b1, 0b11000) => Some(Asimdmiscfp16::FrintpR),
        (0b0, 0b1, 0b11001) => Some(Asimdmiscfp16::FrintzR),
        (0b0, 0b1, 0b11010) => Some(Asimdmiscfp16::FcvtpsR),
        (0b0, 0b1, 0b11011) => Some(Asimdmiscfp16::FcvtzsR),
        (0b0, 0b1, 0b11101) => Some(Asimdmiscfp16::FrecpeR),
        (0b1, 0b0, 0b11000) => Some(Asimdmiscfp16::FrintaR),
        (0b1, 0b0, 0b11001) => Some(Asimdmiscfp16::FrintxR),
        (0b1, 0b0, 0b11010) => Some(Asimdmiscfp16::FcvtnuR),
        (0b1, 0b0, 0b11011) => Some(Asimdmiscfp16::FcvtmuR),
        (0b1, 0b0, 0b11100) => Some(Asimdmiscfp16::FcvtauR),
        (0b1, 0b0, 0b11101) => Some(Asimdmiscfp16::UcvtfR),
        (0b1, 0b1, 0b01100) => Some(Asimdmiscfp16::FcmgeFz),
        (0b1, 0b1, 0b01101) => Some(Asimdmiscfp16::FcmleFz),
        (0b1, 0b1, 0b01111) => Some(Asimdmiscfp16::FnegR),
        (0b1, 0b1, 0b11001) => Some(Asimdmiscfp16::FrintiR),
        (0b1, 0b1, 0b11010) => Some(Asimdmiscfp16::FcvtpuR),
        (0b1, 0b1, 0b11011) => Some(Asimdmiscfp16::FcvtzuR),
        (0b1, 0b1, 0b11101) => Some(Asimdmiscfp16::FrsqrteR),
        (0b1, 0b1, 0b11111) => Some(Asimdmiscfp16::FsqrtR),
        _ => None,
    }
}

impl Asimdmiscfp16 {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::FrintnR
            | Self::FrintmR
            | Self::FcvtnsR
            | Self::FcvtmsR
            | Self::FcvtasR
            | Self::ScvtfR
            | Self::FcmgtFz
            | Self::FcmeqFz
            | Self::FcmltFz
            | Self::FabsR
            | Self::FrintpR
            | Self::FrintzR
            | Self::FcvtpsR
            | Self::FcvtzsR
            | Self::FrecpeR
            | Self::FrintaR
            | Self::FrintxR
            | Self::FcvtnuR
            | Self::FcvtmuR
            | Self::FcvtauR
            | Self::UcvtfR
            | Self::FcmgeFz
            | Self::FcmleFz
            | Self::FnegR
            | Self::FrintiR
            | Self::FcvtpuR
            | Self::FcvtzuR
            | Self::FrsqrteR
            | Self::FsqrtR => Feature::Fp16,
        }
    }
}
