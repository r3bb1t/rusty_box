//! Arm group `control` (Arm ARM C4.1, "Branches, Exception Generating and
//! System instructions"): the A64 space with `op1 = 101x`.

use super::{Class, Encoding, Feature};
use crate::cpu::exec::bits;

/// Arm group `condbranch`: Conditional branch (immediate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Condbranch {
    B,
}

/// Arm group `exception`: Exception generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Exception {
    SvcEx,
    HvcEx,
    SmcEx,
    BrkEx,
    HltEx,
    Dcps1Dc,
    Dcps2Dc,
    Dcps3Dc,
}

/// Arm group `hints`: Hints. `HintHm` is every hint without an encoding of
/// its own here, which executes as a NOP (Arm ARM C6.2, HINT), and so does
/// a hint of a feature the processor does not implement (its decode's
/// `EndOfDecode(Decode_NOP)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Hints {
    NopHi,
    YieldHi,
    WfeHi,
    WfiHi,
    SevHi,
    SevlHi,
    XpaclriHi,
    Pacia1716Hi,
    Pacib1716Hi,
    Autia1716Hi,
    Autib1716Hi,
    EsbHi,
    CsdbHi,
    PaciazHi,
    PaciaspHi,
    PacibzHi,
    PacibspHi,
    AutiazHi,
    AutiaspHi,
    AutibzHi,
    AutibspHi,
    BtiHb,
    HintHm,
}

/// Arm group `barriers`: Barriers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Barriers {
    ClrexBn,
    DsbBon,
    DsbBo,
    DmbBo,
    IsbBi,
    Sb,
}

impl Barriers {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::ClrexBn | Self::DsbBo | Self::DmbBo | Self::IsbBi => Feature::Base,
            Self::DsbBon => Feature::Xs,
            Self::Sb => Feature::Sb,
        }
    }
}

/// Arm group `pstate`: PSTATE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Pstate {
    MsrSi,
    CfinvM,
    XaflagM,
    AxflagM,
}

impl Pstate {
    /// MSR (immediate) is a system instruction; CFINV, XAFLAG and AXFLAG
    /// share a handler, which reads which from `op2`.
    pub(crate) const fn class(self) -> Class {
        match self {
            Self::MsrSi => Class::System,
            Self::CfinvM | Self::XaflagM | Self::AxflagM => Class::FlagManipulation,
        }
    }

    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::MsrSi => Feature::Base,
            Self::CfinvM => Feature::FlagM,
            Self::XaflagM | Self::AxflagM => Feature::FlagM2,
        }
    }
}

/// Arm group `systeminstrswithreg`: System instructions with register
/// argument, all of them FEAT_WFxT's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Systeminstrswithreg {
    Wfet,
    Wfit,
}

/// Arm group `systeminstrs`: System instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Systeminstrs {
    SysCr,
    SyslRc,
}

/// Arm group `systemmove`: System register move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Systemmove {
    MsrSr,
    MrsRs,
}

/// Arm group `branch_reg`: Unconditional branch (register).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BranchReg {
    Br64,
    Braaz64,
    Brabz64,
    Blr64,
    Blraaz64,
    Blrabz64,
    Ret64r,
    Retaa64e,
    Retab64e,
    Eret64e,
    Eretaa64e,
    Eretab64e,
    Drps64e,
    Braa64p,
    Brab64p,
    Blraa64p,
    Blrab64p,
}

/// Arm group `branch_imm`: Unconditional branch (immediate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BranchImm {
    B,
    Bl,
}

/// Arm group `compbranch`: Compare and branch (immediate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Compbranch {
    Cbz32,
    Cbnz32,
    Cbz64,
    Cbnz64,
}

/// Arm group `testbranch`: Test and branch (immediate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Testbranch {
    Tbz,
    Tbnz,
}

pub(super) fn control(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 31, 29);
    match op0 {
        // op1<13:12>: 01 is miscbranch (FEAT_PAuth_LR).
        0b010 => match bits(word, 25, 24) {
            0b00 => condbranch(word).map(Encoding::Condbranch),
            _ => None,
        },
        0b110 => match bits(word, 25, 24) {
            0b00 => exception(word).map(Encoding::Exception),
            0b01 => system(word),
            0b10 | 0b11 => branch_reg(word).map(Encoding::BranchReg),
            _ => None,
        },
        0b000 | 0b100 => branch_imm(word).map(Encoding::BranchImm),
        0b001 | 0b101 => match bits(word, 25, 25) {
            0b0 => compbranch(word).map(Encoding::Compbranch),
            0b1 => testbranch(word).map(Encoding::Testbranch),
            _ => None,
        },
        // 011 and 111: the compare-and-branch forms of FEAT_CMPBR and the
        // fault forms of FEAT_CFLT.
        _ => None,
    }
}

/// `op0 = 110, op1 = 01xxxxxxxxxxxx`: the system instruction space, by
/// `op1<11:0>`. `op1<11:10> = 01` holds the 128-bit system instructions
/// (FEAT_SYSINSTR128, FEAT_SYSREG128).
fn system(word: u32) -> Option<Encoding> {
    match (bits(word, 23, 22), bits(word, 21, 19)) {
        // By op1<6:4> and CRn: CRn = 0001 is systeminstrswithreg
        // (FEAT_WFxT), and every CRn = 0010 word outside the hint space is
        // extendedhints (FEAT_HINTE).
        (0b00, 0b000) => match (bits(word, 18, 16), bits(word, 15, 12)) {
            (0b011, 0b0010) => match bits(word, 4, 0) {
                0b11111 => hints(word).map(Encoding::Hints),
                _ => None,
            },
            (0b011, 0b0001) => systeminstrswithreg(word).map(Encoding::Systeminstrswithreg),
            (0b011, 0b0011) => barriers(word).map(Encoding::Barriers),
            (_, 0b0100) => pstate(word).map(Encoding::Pstate),
            _ => None,
        },
        (0b00, 0b001 | 0b101) => systeminstrs(word).map(Encoding::Systeminstrs),
        (0b00, 0b010 | 0b011 | 0b110 | 0b111) => systemmove(word).map(Encoding::Systemmove),
        _ => None,
    }
}

fn condbranch(word: u32) -> Option<Condbranch> {
    let o0 = bits(word, 4, 4);
    match o0 {
        0b0 => Some(Condbranch::B),
        // BC (FEAT_HBC).
        _ => None,
    }
}

fn exception(word: u32) -> Option<Exception> {
    let opc = bits(word, 23, 21);
    let op2 = bits(word, 4, 2);
    let ll = bits(word, 1, 0);
    match (opc, op2, ll) {
        (0b000, 0b000, 0b01) => Some(Exception::SvcEx),
        (0b000, 0b000, 0b10) => Some(Exception::HvcEx),
        (0b000, 0b000, 0b11) => Some(Exception::SmcEx),
        (0b001, 0b000, 0b00) => Some(Exception::BrkEx),
        (0b010, 0b000, 0b00) => Some(Exception::HltEx),
        (0b101, 0b000, 0b01) => Some(Exception::Dcps1Dc),
        (0b101, 0b000, 0b10) => Some(Exception::Dcps2Dc),
        (0b101, 0b000, 0b11) => Some(Exception::Dcps3Dc),
        // FLT (FEAT_CFLT), TENTER (FEAT_TEV).
        _ => None,
    }
}

fn hints(word: u32) -> Option<Hints> {
    let crm = bits(word, 11, 8);
    let op2 = bits(word, 7, 5);
    match (crm, op2) {
        (0b0000, 0b000) => Some(Hints::NopHi),
        (0b0000, 0b001) => Some(Hints::YieldHi),
        (0b0000, 0b010) => Some(Hints::WfeHi),
        (0b0000, 0b011) => Some(Hints::WfiHi),
        (0b0000, 0b100) => Some(Hints::SevHi),
        (0b0000, 0b101) => Some(Hints::SevlHi),
        (0b0000, 0b111) => Some(Hints::XpaclriHi),
        (0b0001, 0b000) => Some(Hints::Pacia1716Hi),
        (0b0001, 0b010) => Some(Hints::Pacib1716Hi),
        (0b0001, 0b100) => Some(Hints::Autia1716Hi),
        (0b0001, 0b110) => Some(Hints::Autib1716Hi),
        (0b0010, 0b000) => Some(Hints::EsbHi),
        (0b0010, 0b100) => Some(Hints::CsdbHi),
        (0b0011, 0b000) => Some(Hints::PaciazHi),
        (0b0011, 0b001) => Some(Hints::PaciaspHi),
        (0b0011, 0b010) => Some(Hints::PacibzHi),
        (0b0011, 0b011) => Some(Hints::PacibspHi),
        (0b0011, 0b100) => Some(Hints::AutiazHi),
        (0b0011, 0b101) => Some(Hints::AutiaspHi),
        (0b0011, 0b110) => Some(Hints::AutibzHi),
        (0b0011, 0b111) => Some(Hints::AutibspHi),
        (0b0100, 0b000 | 0b010 | 0b100 | 0b110) => Some(Hints::BtiHb),
        _ => Some(Hints::HintHm),
    }
}

impl Hints {
    /// The handler class, by [`Hints::feature`]: the pointer authentication
    /// hints have their own; every other hint shares the hint one. Both
    /// handlers read which hint from `CRm` and `op2`. BTI executes as a NOP:
    /// what it does is the check of PSTATE.BTYPE every instruction makes
    /// before it executes ([`crate::cpu::Cpu::branch_target_check`]).
    pub(crate) const fn class(self) -> Class {
        if matches!(self.feature(), Feature::PAuth) {
            Class::PointerAuthHint
        } else {
            Class::Hint
        }
    }

    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::NopHi
            | Self::YieldHi
            | Self::WfeHi
            | Self::WfiHi
            | Self::SevHi
            | Self::SevlHi
            | Self::CsdbHi
            | Self::HintHm => Feature::Base,
            Self::XpaclriHi
            | Self::Pacia1716Hi
            | Self::Pacib1716Hi
            | Self::Autia1716Hi
            | Self::Autib1716Hi
            | Self::PaciazHi
            | Self::PaciaspHi
            | Self::PacibzHi
            | Self::PacibspHi
            | Self::AutiazHi
            | Self::AutiaspHi
            | Self::AutibzHi
            | Self::AutibspHi => Feature::PAuth,
            Self::BtiHb => Feature::Bti,
            Self::EsbHi => Feature::Ras,
        }
    }
}

fn barriers(word: u32) -> Option<Barriers> {
    let crm = bits(word, 11, 8);
    let op2 = bits(word, 7, 5);
    let rt = bits(word, 4, 0);
    match (crm, op2, rt) {
        (0b0010 | 0b0110 | 0b1010 | 0b1110, 0b001, 0b11111) => Some(Barriers::DsbBon),
        (_, 0b010, 0b11111) => Some(Barriers::ClrexBn),
        (_, 0b100, 0b11111) => Some(Barriers::DsbBo),
        (_, 0b101, 0b11111) => Some(Barriers::DmbBo),
        (_, 0b110, 0b11111) => Some(Barriers::IsbBi),
        (0b0000, 0b111, 0b11111) => Some(Barriers::Sb),
        _ => None,
    }
}

fn pstate(word: u32) -> Option<Pstate> {
    let op1 = bits(word, 18, 16);
    let crm = bits(word, 11, 8);
    let op2 = bits(word, 7, 5);
    let rt = bits(word, 4, 0);
    match (op1, crm, op2, rt) {
        (0b000, 0b0000, 0b000, 0b11111) => Some(Pstate::CfinvM),
        (0b000, 0b0000, 0b001, 0b11111) => Some(Pstate::XaflagM),
        (0b000, 0b0000, 0b010, 0b11111) => Some(Pstate::AxflagM),
        // MSR (immediate) is not these op1 and op2, whatever CRm.
        (0b000, _, 0b000..=0b010, _) => None,
        (_, _, _, 0b11111) => Some(Pstate::MsrSi),
        _ => None,
    }
}

fn systeminstrswithreg(word: u32) -> Option<Systeminstrswithreg> {
    let crm = bits(word, 11, 8);
    let op2 = bits(word, 7, 5);
    match (crm, op2) {
        (0b0000, 0b000) => Some(Systeminstrswithreg::Wfet),
        (0b0000, 0b001) => Some(Systeminstrswithreg::Wfit),
        _ => None,
    }
}

fn systeminstrs(word: u32) -> Option<Systeminstrs> {
    let l = bits(word, 21, 21);
    match l {
        0b0 => Some(Systeminstrs::SysCr),
        0b1 => Some(Systeminstrs::SyslRc),
        _ => None,
    }
}

fn systemmove(word: u32) -> Option<Systemmove> {
    let l = bits(word, 21, 21);
    match l {
        0b0 => Some(Systemmove::MsrSr),
        0b1 => Some(Systemmove::MrsRs),
        _ => None,
    }
}

fn branch_reg(word: u32) -> Option<BranchReg> {
    let opc = bits(word, 24, 21);
    let op2 = bits(word, 20, 16);
    let op3 = bits(word, 15, 10);
    let rn = bits(word, 9, 5);
    let op4 = bits(word, 4, 0);
    // RETAASPPCR and RETABSPPCR (FEAT_PAuth_LR) are RETAA and RETAB with
    // op4 other than 11111; TEXIT (FEAT_TEV), and tchange_reg and
    // tchange_imm (FEAT_S1POE2) share this op1.
    match (opc, op2, op3) {
        (0b0000, 0b11111, 0b000000) => match op4 {
            0b00000 => Some(BranchReg::Br64),
            _ => None,
        },
        (0b0000, 0b11111, 0b000010) => match op4 {
            0b11111 => Some(BranchReg::Braaz64),
            _ => None,
        },
        (0b0000, 0b11111, 0b000011) => match op4 {
            0b11111 => Some(BranchReg::Brabz64),
            _ => None,
        },
        (0b0001, 0b11111, 0b000000) => match op4 {
            0b00000 => Some(BranchReg::Blr64),
            _ => None,
        },
        (0b0001, 0b11111, 0b000010) => match op4 {
            0b11111 => Some(BranchReg::Blraaz64),
            _ => None,
        },
        (0b0001, 0b11111, 0b000011) => match op4 {
            0b11111 => Some(BranchReg::Blrabz64),
            _ => None,
        },
        (0b0010, 0b11111, 0b000000) => match op4 {
            0b00000 => Some(BranchReg::Ret64r),
            _ => None,
        },
        (0b0010, 0b11111, 0b000010) => match (rn, op4) {
            (0b11111, 0b11111) => Some(BranchReg::Retaa64e),
            _ => None,
        },
        (0b0010, 0b11111, 0b000011) => match (rn, op4) {
            (0b11111, 0b11111) => Some(BranchReg::Retab64e),
            _ => None,
        },
        (0b0100, 0b11111, 0b000000) => match (rn, op4) {
            (0b11111, 0b00000) => Some(BranchReg::Eret64e),
            _ => None,
        },
        (0b0100, 0b11111, 0b000010) => match (rn, op4) {
            (0b11111, 0b11111) => Some(BranchReg::Eretaa64e),
            _ => None,
        },
        (0b0100, 0b11111, 0b000011) => match (rn, op4) {
            (0b11111, 0b11111) => Some(BranchReg::Eretab64e),
            _ => None,
        },
        (0b0101, 0b11111, 0b000000) => match (rn, op4) {
            (0b11111, 0b00000) => Some(BranchReg::Drps64e),
            _ => None,
        },
        (0b1000, 0b11111, 0b000010) => Some(BranchReg::Braa64p),
        (0b1000, 0b11111, 0b000011) => Some(BranchReg::Brab64p),
        (0b1001, 0b11111, 0b000010) => Some(BranchReg::Blraa64p),
        (0b1001, 0b11111, 0b000011) => Some(BranchReg::Blrab64p),
        _ => None,
    }
}

impl BranchReg {
    /// The handler class, by [`BranchReg::feature`]: the branches and
    /// returns that authenticate their target have their own (the handler
    /// reads which from `opc`, `op3` and `op4`); the rest share the
    /// register-branch one.
    pub(crate) const fn class(self) -> Class {
        if matches!(self.feature(), Feature::PAuth) {
            Class::AuthenticatedBranch
        } else {
            Class::BranchRegister
        }
    }

    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Br64 | Self::Blr64 | Self::Ret64r | Self::Eret64e | Self::Drps64e => {
                Feature::Base
            }
            Self::Braaz64
            | Self::Brabz64
            | Self::Blraaz64
            | Self::Blrabz64
            | Self::Retaa64e
            | Self::Retab64e
            | Self::Eretaa64e
            | Self::Eretab64e
            | Self::Braa64p
            | Self::Brab64p
            | Self::Blraa64p
            | Self::Blrab64p => Feature::PAuth,
        }
    }
}

fn branch_imm(word: u32) -> Option<BranchImm> {
    let op = bits(word, 31, 31);
    match op {
        0b0 => Some(BranchImm::B),
        0b1 => Some(BranchImm::Bl),
        _ => None,
    }
}

fn compbranch(word: u32) -> Option<Compbranch> {
    let sf = bits(word, 31, 31);
    let op = bits(word, 24, 24);
    match (sf, op) {
        (0b0, 0b0) => Some(Compbranch::Cbz32),
        (0b0, 0b1) => Some(Compbranch::Cbnz32),
        (0b1, 0b0) => Some(Compbranch::Cbz64),
        (0b1, 0b1) => Some(Compbranch::Cbnz64),
        _ => None,
    }
}

fn testbranch(word: u32) -> Option<Testbranch> {
    let op = bits(word, 24, 24);
    match op {
        0b0 => Some(Testbranch::Tbz),
        0b1 => Some(Testbranch::Tbnz),
        _ => None,
    }
}
