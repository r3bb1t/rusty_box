//! Arm group `control` (Arm ARM C4.1, "Branches, Exception Generating and
//! System instructions"): the A64 space with `op1 = 101x`.

use super::Encoding;
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
/// its own here, which executes as a NOP (Arm ARM C6.2, HINT).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Hints {
    NopHi,
    YieldHi,
    WfeHi,
    WfiHi,
    SevHi,
    SevlHi,
    CsdbHi,
    HintHm,
}

/// Arm group `barriers`: Barriers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Barriers {
    ClrexBn,
    DsbBo,
    DmbBo,
    IsbBi,
}

/// Arm group `pstate`: PSTATE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Pstate {
    MsrSi,
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
    Blr64,
    Ret64r,
    Eret64e,
    Drps64e,
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
        (0b0010, 0b100) => Some(Hints::CsdbHi),
        _ => Some(Hints::HintHm),
    }
}

fn barriers(word: u32) -> Option<Barriers> {
    let op2 = bits(word, 7, 5);
    let rt = bits(word, 4, 0);
    match (op2, rt) {
        (0b010, 0b11111) => Some(Barriers::ClrexBn),
        (0b100, 0b11111) => Some(Barriers::DsbBo),
        (0b101, 0b11111) => Some(Barriers::DmbBo),
        (0b110, 0b11111) => Some(Barriers::IsbBi),
        // DSB nXS (FEAT_XS), SB (FEAT_SB).
        _ => None,
    }
}

fn pstate(word: u32) -> Option<Pstate> {
    let op1 = bits(word, 18, 16);
    let op2 = bits(word, 7, 5);
    let rt = bits(word, 4, 0);
    match (op1, op2, rt) {
        // CFINV, XAFLAG and AXFLAG (FEAT_FlagM, FEAT_FlagM2).
        (0b000, 0b000..=0b010, _) => None,
        (_, _, 0b11111) => Some(Pstate::MsrSi),
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
    match (opc, op2, op3, op4) {
        (0b0000, 0b11111, 0b000000, 0b00000) => Some(BranchReg::Br64),
        (0b0001, 0b11111, 0b000000, 0b00000) => Some(BranchReg::Blr64),
        (0b0010, 0b11111, 0b000000, 0b00000) => Some(BranchReg::Ret64r),
        (0b0100, 0b11111, 0b000000, 0b00000) => match rn {
            0b11111 => Some(BranchReg::Eret64e),
            _ => None,
        },
        (0b0101, 0b11111, 0b000000, 0b00000) => match rn {
            0b11111 => Some(BranchReg::Drps64e),
            _ => None,
        },
        // The pointer-authenticating branches (FEAT_PAuth, FEAT_PAuth_LR),
        // TEXIT (FEAT_TEV), and tchange_reg and tchange_imm (FEAT_S1POE2),
        // which share this op1.
        _ => None,
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
