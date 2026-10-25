//! Arm group `dpimm` (Arm ARM C4.1, "Data Processing -- Immediate"): the
//! A64 space with `op1 = 100x`.

use super::Encoding;
use crate::cpu::exec::bits;

/// Arm group `pcreladdr`: PC-rel. addressing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Pcreladdr {
    Adr,
    Adrp,
}

/// Arm group `addsub_imm`: Add/subtract (immediate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AddsubImm {
    Add32,
    Adds32s,
    Sub32,
    Subs32s,
    Add64,
    Adds64s,
    Sub64,
    Subs64s,
}

/// Arm group `log_imm`: Logical (immediate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LogImm {
    And32,
    Orr32,
    Eor32,
    Ands32s,
    And64,
    Orr64,
    Eor64,
    Ands64s,
}

/// Arm group `movewide`: Move wide (immediate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Movewide {
    Movn32,
    Movz32,
    Movk32,
    Movn64,
    Movz64,
    Movk64,
}

/// Arm group `bitfield`: Bitfield.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Bitfield {
    Sbfm32m,
    Bfm32m,
    Ubfm32m,
    Sbfm64m,
    Bfm64m,
    Ubfm64m,
}

/// Arm group `extract`: Extract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Extract {
    Extr32,
    Extr64,
}

pub(super) fn dpimm(word: u32) -> Option<Encoding> {
    let op1 = bits(word, 25, 22);
    match op1 {
        0b0000..=0b0011 => pcreladdr(word).map(Encoding::Pcreladdr),
        0b0100 | 0b0101 => addsub_imm(word).map(Encoding::AddsubImm),
        // 0110 addsub_immtags (FEAT_MTE), 0111 minmax_imm (FEAT_CSSC).
        0b1000 | 0b1001 => log_imm(word).map(Encoding::LogImm),
        0b1010 | 0b1011 => movewide(word).map(Encoding::Movewide),
        0b1100 | 0b1101 => bitfield(word).map(Encoding::Bitfield),
        // With op0 = 11, dp_1src_imm (FEAT_PAuth_LR); the group's leaves
        // fix op21 = 00.
        0b1110 | 0b1111 => extract(word).map(Encoding::Extract),
        _ => None,
    }
}

fn pcreladdr(word: u32) -> Option<Pcreladdr> {
    let op = bits(word, 31, 31);
    match op {
        0b0 => Some(Pcreladdr::Adr),
        0b1 => Some(Pcreladdr::Adrp),
        _ => None,
    }
}

fn addsub_imm(word: u32) -> Option<AddsubImm> {
    let sf = bits(word, 31, 31);
    let op = bits(word, 30, 30);
    let s = bits(word, 29, 29);
    match (sf, op, s) {
        (0b0, 0b0, 0b0) => Some(AddsubImm::Add32),
        (0b0, 0b0, 0b1) => Some(AddsubImm::Adds32s),
        (0b0, 0b1, 0b0) => Some(AddsubImm::Sub32),
        (0b0, 0b1, 0b1) => Some(AddsubImm::Subs32s),
        (0b1, 0b0, 0b0) => Some(AddsubImm::Add64),
        (0b1, 0b0, 0b1) => Some(AddsubImm::Adds64s),
        (0b1, 0b1, 0b0) => Some(AddsubImm::Sub64),
        (0b1, 0b1, 0b1) => Some(AddsubImm::Subs64s),
        _ => None,
    }
}

fn log_imm(word: u32) -> Option<LogImm> {
    let sf = bits(word, 31, 31);
    let opc = bits(word, 30, 29);
    let n = bits(word, 22, 22);
    match (sf, opc, n) {
        (0b0, 0b00, 0b0) => Some(LogImm::And32),
        (0b0, 0b01, 0b0) => Some(LogImm::Orr32),
        (0b0, 0b10, 0b0) => Some(LogImm::Eor32),
        (0b0, 0b11, 0b0) => Some(LogImm::Ands32s),
        (0b1, 0b00, _) => Some(LogImm::And64),
        (0b1, 0b01, _) => Some(LogImm::Orr64),
        (0b1, 0b10, _) => Some(LogImm::Eor64),
        (0b1, 0b11, _) => Some(LogImm::Ands64s),
        _ => None,
    }
}

fn movewide(word: u32) -> Option<Movewide> {
    let sf = bits(word, 31, 31);
    let opc = bits(word, 30, 29);
    let hw = bits(word, 22, 21);
    match (sf, opc, hw) {
        (0b0, 0b00, 0b00 | 0b01) => Some(Movewide::Movn32),
        (0b0, 0b10, 0b00 | 0b01) => Some(Movewide::Movz32),
        (0b0, 0b11, 0b00 | 0b01) => Some(Movewide::Movk32),
        (0b1, 0b00, _) => Some(Movewide::Movn64),
        (0b1, 0b10, _) => Some(Movewide::Movz64),
        (0b1, 0b11, _) => Some(Movewide::Movk64),
        _ => None,
    }
}

fn bitfield(word: u32) -> Option<Bitfield> {
    let sf = bits(word, 31, 31);
    let opc = bits(word, 30, 29);
    let n = bits(word, 22, 22);
    match (sf, opc, n) {
        (0b0, 0b00, 0b0) => Some(Bitfield::Sbfm32m),
        (0b0, 0b01, 0b0) => Some(Bitfield::Bfm32m),
        (0b0, 0b10, 0b0) => Some(Bitfield::Ubfm32m),
        (0b1, 0b00, 0b1) => Some(Bitfield::Sbfm64m),
        (0b1, 0b01, 0b1) => Some(Bitfield::Bfm64m),
        (0b1, 0b10, 0b1) => Some(Bitfield::Ubfm64m),
        _ => None,
    }
}

fn extract(word: u32) -> Option<Extract> {
    let sf = bits(word, 31, 31);
    let op21 = bits(word, 30, 29);
    let n = bits(word, 22, 22);
    let o0 = bits(word, 21, 21);
    match (sf, op21, n, o0) {
        // imms<5> = 0: a 32-bit extract takes its position from imms<4:0>.
        (0b0, 0b00, 0b0, 0b0) => match bits(word, 15, 15) {
            0b0 => Some(Extract::Extr32),
            _ => None,
        },
        (0b1, 0b00, 0b1, 0b0) => Some(Extract::Extr64),
        _ => None,
    }
}
