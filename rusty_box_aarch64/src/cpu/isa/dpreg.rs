//! Arm group `dpreg` (Arm ARM C4.1, "Data Processing -- Register"): the A64
//! space with `op1 = x101`.

use super::Feature;
use super::{Class, Encoding};
use crate::cpu::exec::bits;

/// Arm group `dp_2src`: Data-processing (2 source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Dp2src {
    Udiv32,
    Sdiv32,
    Lslv32,
    Lsrv32,
    Asrv32,
    Rorv32,
    Udiv64,
    Sdiv64,
    Lslv64,
    Lsrv64,
    Asrv64,
    Rorv64,
    Crc32b32c,
    Crc32h32c,
    Crc32w32c,
    Crc32cb32c,
    Crc32ch32c,
    Crc32cw32c,
    Crc32x64c,
    Crc32cx64c,
    Pacga64p,
}

/// Arm group `dp_1src`: Data-processing (1 source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Dp1src {
    Rbit32,
    Rev1632,
    Rev32,
    Clz32,
    Cls32,
    Rbit64,
    Rev1664,
    Rev3264,
    Rev64,
    Clz64,
    Cls64,
    Pacia64p,
    Pacib64p,
    Pacda64p,
    Pacdb64p,
    Autia64p,
    Autib64p,
    Autda64p,
    Autdb64p,
    Paciza64z,
    Pacizb64z,
    Pacdza64z,
    Pacdzb64z,
    Autiza64z,
    Autizb64z,
    Autdza64z,
    Autdzb64z,
    Xpaci64z,
    Xpacd64z,
}

/// Arm group `log_shift`: Logical (shifted register).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LogShift {
    And32,
    Bic32,
    Orr32,
    Orn32,
    Eor32,
    Eon32,
    Ands32,
    Bics32,
    And64,
    Bic64,
    Orr64,
    Orn64,
    Eor64,
    Eon64,
    Ands64,
    Bics64,
}

/// Arm group `addsub_shift`: Add/subtract (shifted register).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AddsubShift {
    Add32,
    Adds32,
    Sub32,
    Subs32,
    Add64,
    Adds64,
    Sub64,
    Subs64,
}

/// Arm group `addsub_ext`: Add/subtract (extended register).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AddsubExt {
    Add32,
    Adds32s,
    Sub32,
    Subs32s,
    Add64,
    Adds64s,
    Sub64,
    Subs64s,
}

/// Arm group `addsub_carry`: Add/subtract (with carry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AddsubCarry {
    Adc32,
    Adcs32,
    Sbc32,
    Sbcs32,
    Adc64,
    Adcs64,
    Sbc64,
    Sbcs64,
}

/// Arm group `condcmp_reg`: Conditional compare (register).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CondcmpReg {
    Ccmn32,
    Ccmp32,
    Ccmn64,
    Ccmp64,
}

/// Arm group `condcmp_imm`: Conditional compare (immediate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CondcmpImm {
    Ccmn32,
    Ccmp32,
    Ccmn64,
    Ccmp64,
}

/// Arm group `condsel`: Conditional select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Condsel {
    Csel32,
    Csinc32,
    Csinv32,
    Csneg32,
    Csel64,
    Csinc64,
    Csinv64,
    Csneg64,
}

/// Arm group `rmif`: Rotate right into flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Rmif {
    Rmif,
}

/// Arm group `setf`: Evaluate into flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Setf {
    Setf8,
    Setf16,
}

/// Arm group `dp_3src`: Data-processing (3 source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Dp3src {
    Madd32a,
    Msub32a,
    Madd64a,
    Msub64a,
    Smaddl64wa,
    Smsubl64wa,
    Smulh64,
    Umaddl64wa,
    Umsubl64wa,
    Umulh64,
}

pub(super) fn dpreg(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 30, 30);
    let op1 = bits(word, 28, 28);
    let op2 = bits(word, 24, 21);
    let op3 = bits(word, 15, 10);
    match op1 {
        // op2<3>, then op2<0>.
        0b0 => match (bits(word, 24, 24), bits(word, 21, 21)) {
            (0b0, _) => log_shift(word).map(Encoding::LogShift),
            (0b1, 0b0) => addsub_shift(word).map(Encoding::AddsubShift),
            (0b1, 0b1) => addsub_ext(word).map(Encoding::AddsubExt),
            _ => None,
        },
        0b1 => match bits(word, 24, 24) {
            0b1 => dp_3src(word).map(Encoding::Dp3src),
            0b0 => match op2 {
                // op3 = 001xxx is addsub_pt (FEAT_CPA).
                0b0000 => match op3 {
                    0b000000 => addsub_carry(word).map(Encoding::AddsubCarry),
                    0b000001 | 0b100001 => rmif(word).map(Encoding::Rmif),
                    0b000010 | 0b010010 | 0b100010 | 0b110010 => setf(word).map(Encoding::Setf),
                    _ => None,
                },
                0b0010 => match bits(word, 11, 11) {
                    0b0 => condcmp_reg(word).map(Encoding::CondcmpReg),
                    0b1 => condcmp_imm(word).map(Encoding::CondcmpImm),
                    _ => None,
                },
                0b0100 => condsel(word).map(Encoding::Condsel),
                0b0110 => match op0 {
                    0b0 => dp_2src(word).map(Encoding::Dp2src),
                    0b1 => dp_1src(word).map(Encoding::Dp1src),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

fn rmif(word: u32) -> Option<Rmif> {
    let sf = bits(word, 31, 31);
    let op = bits(word, 30, 30);
    let s = bits(word, 29, 29);
    let o2 = bits(word, 4, 4);
    match (sf, op, s, o2) {
        (0b1, 0b0, 0b1, 0b0) => Some(Rmif::Rmif),
        _ => None,
    }
}

fn setf(word: u32) -> Option<Setf> {
    let sf = bits(word, 31, 31);
    let op = bits(word, 30, 30);
    let s = bits(word, 29, 29);
    let opcode2 = bits(word, 20, 15);
    let sz = bits(word, 14, 14);
    let o3 = bits(word, 4, 4);
    let mask = bits(word, 3, 0);
    match (sf, op, s, opcode2, sz, o3, mask) {
        (0b0, 0b0, 0b1, 0b000000, 0b0, 0b0, 0b1101) => Some(Setf::Setf8),
        (0b0, 0b0, 0b1, 0b000000, 0b1, 0b0, 0b1101) => Some(Setf::Setf16),
        _ => None,
    }
}

fn dp_2src(word: u32) -> Option<Dp2src> {
    let sf = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let opcode = bits(word, 15, 10);
    // opcode 0110xx is the register minimum and maximum (FEAT_CSSC); with
    // sf = 1, 000000, 000100 and 000101 are SUBP(S), IRG and GMI
    // (FEAT_MTE). CRC32 and CRC32C (opcode 010 C sz) take a 64-bit operand
    // exactly when sz = 11, and only with sf = 1.
    match (sf, s, opcode) {
        (0b0, 0b0, 0b000010) => Some(Dp2src::Udiv32),
        (0b0, 0b0, 0b000011) => Some(Dp2src::Sdiv32),
        (0b0, 0b0, 0b001000) => Some(Dp2src::Lslv32),
        (0b0, 0b0, 0b001001) => Some(Dp2src::Lsrv32),
        (0b0, 0b0, 0b001010) => Some(Dp2src::Asrv32),
        (0b0, 0b0, 0b001011) => Some(Dp2src::Rorv32),
        (0b1, 0b0, 0b000010) => Some(Dp2src::Udiv64),
        (0b1, 0b0, 0b000011) => Some(Dp2src::Sdiv64),
        (0b1, 0b0, 0b001000) => Some(Dp2src::Lslv64),
        (0b1, 0b0, 0b001001) => Some(Dp2src::Lsrv64),
        (0b1, 0b0, 0b001010) => Some(Dp2src::Asrv64),
        (0b1, 0b0, 0b001011) => Some(Dp2src::Rorv64),
        (0b1, 0b0, 0b001100) => Some(Dp2src::Pacga64p),
        (0b0, 0b0, 0b010000) => Some(Dp2src::Crc32b32c),
        (0b0, 0b0, 0b010001) => Some(Dp2src::Crc32h32c),
        (0b0, 0b0, 0b010010) => Some(Dp2src::Crc32w32c),
        (0b0, 0b0, 0b010100) => Some(Dp2src::Crc32cb32c),
        (0b0, 0b0, 0b010101) => Some(Dp2src::Crc32ch32c),
        (0b0, 0b0, 0b010110) => Some(Dp2src::Crc32cw32c),
        (0b1, 0b0, 0b010011) => Some(Dp2src::Crc32x64c),
        (0b1, 0b0, 0b010111) => Some(Dp2src::Crc32cx64c),
        _ => None,
    }
}

fn dp_1src(word: u32) -> Option<Dp1src> {
    let sf = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let opcode2 = bits(word, 20, 16);
    let opcode = bits(word, 15, 10);
    let rn = bits(word, 9, 5);
    // opcode 00011x and 001000 are CTZ, CNT and ABS (FEAT_CSSC); with
    // opcode2 = 00001, opcode 10xxxx holds the pointer authentication
    // instructions of FEAT_PAuth_LR.
    match (sf, s, opcode2, opcode) {
        (0b0, 0b0, 0b00000, 0b000000) => Some(Dp1src::Rbit32),
        (0b0, 0b0, 0b00000, 0b000001) => Some(Dp1src::Rev1632),
        (0b0, 0b0, 0b00000, 0b000010) => Some(Dp1src::Rev32),
        (0b0, 0b0, 0b00000, 0b000100) => Some(Dp1src::Clz32),
        (0b0, 0b0, 0b00000, 0b000101) => Some(Dp1src::Cls32),
        (0b1, 0b0, 0b00000, 0b000000) => Some(Dp1src::Rbit64),
        (0b1, 0b0, 0b00000, 0b000001) => Some(Dp1src::Rev1664),
        (0b1, 0b0, 0b00000, 0b000010) => Some(Dp1src::Rev3264),
        (0b1, 0b0, 0b00000, 0b000011) => Some(Dp1src::Rev64),
        (0b1, 0b0, 0b00000, 0b000100) => Some(Dp1src::Clz64),
        (0b1, 0b0, 0b00000, 0b000101) => Some(Dp1src::Cls64),
        (0b1, 0b0, 0b00001, 0b000000) => Some(Dp1src::Pacia64p),
        (0b1, 0b0, 0b00001, 0b000001) => Some(Dp1src::Pacib64p),
        (0b1, 0b0, 0b00001, 0b000010) => Some(Dp1src::Pacda64p),
        (0b1, 0b0, 0b00001, 0b000011) => Some(Dp1src::Pacdb64p),
        (0b1, 0b0, 0b00001, 0b000100) => Some(Dp1src::Autia64p),
        (0b1, 0b0, 0b00001, 0b000101) => Some(Dp1src::Autib64p),
        (0b1, 0b0, 0b00001, 0b000110) => Some(Dp1src::Autda64p),
        (0b1, 0b0, 0b00001, 0b000111) => Some(Dp1src::Autdb64p),
        (0b1, 0b0, 0b00001, 0b001000) => match rn {
            0b11111 => Some(Dp1src::Paciza64z),
            _ => None,
        },
        (0b1, 0b0, 0b00001, 0b001001) => match rn {
            0b11111 => Some(Dp1src::Pacizb64z),
            _ => None,
        },
        (0b1, 0b0, 0b00001, 0b001010) => match rn {
            0b11111 => Some(Dp1src::Pacdza64z),
            _ => None,
        },
        (0b1, 0b0, 0b00001, 0b001011) => match rn {
            0b11111 => Some(Dp1src::Pacdzb64z),
            _ => None,
        },
        (0b1, 0b0, 0b00001, 0b001100) => match rn {
            0b11111 => Some(Dp1src::Autiza64z),
            _ => None,
        },
        (0b1, 0b0, 0b00001, 0b001101) => match rn {
            0b11111 => Some(Dp1src::Autizb64z),
            _ => None,
        },
        (0b1, 0b0, 0b00001, 0b001110) => match rn {
            0b11111 => Some(Dp1src::Autdza64z),
            _ => None,
        },
        (0b1, 0b0, 0b00001, 0b001111) => match rn {
            0b11111 => Some(Dp1src::Autdzb64z),
            _ => None,
        },
        (0b1, 0b0, 0b00001, 0b010000) => match rn {
            0b11111 => Some(Dp1src::Xpaci64z),
            _ => None,
        },
        (0b1, 0b0, 0b00001, 0b010001) => match rn {
            0b11111 => Some(Dp1src::Xpacd64z),
            _ => None,
        },
        _ => None,
    }
}

fn log_shift(word: u32) -> Option<LogShift> {
    let sf = bits(word, 31, 31);
    let opc = bits(word, 30, 29);
    let n = bits(word, 21, 21);
    match (sf, opc, n) {
        (0b0, 0b00, 0b0) => Some(LogShift::And32),
        (0b0, 0b00, 0b1) => Some(LogShift::Bic32),
        (0b0, 0b01, 0b0) => Some(LogShift::Orr32),
        (0b0, 0b01, 0b1) => Some(LogShift::Orn32),
        (0b0, 0b10, 0b0) => Some(LogShift::Eor32),
        (0b0, 0b10, 0b1) => Some(LogShift::Eon32),
        (0b0, 0b11, 0b0) => Some(LogShift::Ands32),
        (0b0, 0b11, 0b1) => Some(LogShift::Bics32),
        (0b1, 0b00, 0b0) => Some(LogShift::And64),
        (0b1, 0b00, 0b1) => Some(LogShift::Bic64),
        (0b1, 0b01, 0b0) => Some(LogShift::Orr64),
        (0b1, 0b01, 0b1) => Some(LogShift::Orn64),
        (0b1, 0b10, 0b0) => Some(LogShift::Eor64),
        (0b1, 0b10, 0b1) => Some(LogShift::Eon64),
        (0b1, 0b11, 0b0) => Some(LogShift::Ands64),
        (0b1, 0b11, 0b1) => Some(LogShift::Bics64),
        _ => None,
    }
}

fn addsub_shift(word: u32) -> Option<AddsubShift> {
    let sf = bits(word, 31, 31);
    let op = bits(word, 30, 30);
    let s = bits(word, 29, 29);
    match (sf, op, s) {
        (0b0, 0b0, 0b0) => Some(AddsubShift::Add32),
        (0b0, 0b0, 0b1) => Some(AddsubShift::Adds32),
        (0b0, 0b1, 0b0) => Some(AddsubShift::Sub32),
        (0b0, 0b1, 0b1) => Some(AddsubShift::Subs32),
        (0b1, 0b0, 0b0) => Some(AddsubShift::Add64),
        (0b1, 0b0, 0b1) => Some(AddsubShift::Adds64),
        (0b1, 0b1, 0b0) => Some(AddsubShift::Sub64),
        (0b1, 0b1, 0b1) => Some(AddsubShift::Subs64),
        _ => None,
    }
}

fn addsub_ext(word: u32) -> Option<AddsubExt> {
    let sf = bits(word, 31, 31);
    let op = bits(word, 30, 30);
    let s = bits(word, 29, 29);
    let opt = bits(word, 23, 22);
    match (sf, op, s, opt) {
        (0b0, 0b0, 0b0, 0b00) => Some(AddsubExt::Add32),
        (0b0, 0b0, 0b1, 0b00) => Some(AddsubExt::Adds32s),
        (0b0, 0b1, 0b0, 0b00) => Some(AddsubExt::Sub32),
        (0b0, 0b1, 0b1, 0b00) => Some(AddsubExt::Subs32s),
        (0b1, 0b0, 0b0, 0b00) => Some(AddsubExt::Add64),
        (0b1, 0b0, 0b1, 0b00) => Some(AddsubExt::Adds64s),
        (0b1, 0b1, 0b0, 0b00) => Some(AddsubExt::Sub64),
        (0b1, 0b1, 0b1, 0b00) => Some(AddsubExt::Subs64s),
        _ => None,
    }
}

fn addsub_carry(word: u32) -> Option<AddsubCarry> {
    let sf = bits(word, 31, 31);
    let op = bits(word, 30, 30);
    let s = bits(word, 29, 29);
    match (sf, op, s) {
        (0b0, 0b0, 0b0) => Some(AddsubCarry::Adc32),
        (0b0, 0b0, 0b1) => Some(AddsubCarry::Adcs32),
        (0b0, 0b1, 0b0) => Some(AddsubCarry::Sbc32),
        (0b0, 0b1, 0b1) => Some(AddsubCarry::Sbcs32),
        (0b1, 0b0, 0b0) => Some(AddsubCarry::Adc64),
        (0b1, 0b0, 0b1) => Some(AddsubCarry::Adcs64),
        (0b1, 0b1, 0b0) => Some(AddsubCarry::Sbc64),
        (0b1, 0b1, 0b1) => Some(AddsubCarry::Sbcs64),
        _ => None,
    }
}

fn condcmp_reg(word: u32) -> Option<CondcmpReg> {
    let sf = bits(word, 31, 31);
    let op = bits(word, 30, 30);
    let s = bits(word, 29, 29);
    let o2 = bits(word, 10, 10);
    let o3 = bits(word, 4, 4);
    match (sf, op, s, o2, o3) {
        (0b0, 0b0, 0b1, 0b0, 0b0) => Some(CondcmpReg::Ccmn32),
        (0b0, 0b1, 0b1, 0b0, 0b0) => Some(CondcmpReg::Ccmp32),
        (0b1, 0b0, 0b1, 0b0, 0b0) => Some(CondcmpReg::Ccmn64),
        (0b1, 0b1, 0b1, 0b0, 0b0) => Some(CondcmpReg::Ccmp64),
        _ => None,
    }
}

fn condcmp_imm(word: u32) -> Option<CondcmpImm> {
    let sf = bits(word, 31, 31);
    let op = bits(word, 30, 30);
    let s = bits(word, 29, 29);
    let o2 = bits(word, 10, 10);
    let o3 = bits(word, 4, 4);
    match (sf, op, s, o2, o3) {
        (0b0, 0b0, 0b1, 0b0, 0b0) => Some(CondcmpImm::Ccmn32),
        (0b0, 0b1, 0b1, 0b0, 0b0) => Some(CondcmpImm::Ccmp32),
        (0b1, 0b0, 0b1, 0b0, 0b0) => Some(CondcmpImm::Ccmn64),
        (0b1, 0b1, 0b1, 0b0, 0b0) => Some(CondcmpImm::Ccmp64),
        _ => None,
    }
}

fn condsel(word: u32) -> Option<Condsel> {
    let sf = bits(word, 31, 31);
    let op = bits(word, 30, 30);
    let s = bits(word, 29, 29);
    let op2 = bits(word, 11, 10);
    match (sf, op, s, op2) {
        (0b0, 0b0, 0b0, 0b00) => Some(Condsel::Csel32),
        (0b0, 0b0, 0b0, 0b01) => Some(Condsel::Csinc32),
        (0b0, 0b1, 0b0, 0b00) => Some(Condsel::Csinv32),
        (0b0, 0b1, 0b0, 0b01) => Some(Condsel::Csneg32),
        (0b1, 0b0, 0b0, 0b00) => Some(Condsel::Csel64),
        (0b1, 0b0, 0b0, 0b01) => Some(Condsel::Csinc64),
        (0b1, 0b1, 0b0, 0b00) => Some(Condsel::Csinv64),
        (0b1, 0b1, 0b0, 0b01) => Some(Condsel::Csneg64),
        _ => None,
    }
}

fn dp_3src(word: u32) -> Option<Dp3src> {
    let sf = bits(word, 31, 31);
    let op54 = bits(word, 30, 29);
    let op31 = bits(word, 23, 21);
    let o0 = bits(word, 15, 15);
    let ra = bits(word, 14, 10);
    // Ra is should-be-one in SMULH and UMULH: fixed here. op31 = 011 is
    // MADDPT and MSUBPT (FEAT_CPA).
    match (sf, op54, op31, o0, ra) {
        (0b0, 0b00, 0b000, 0b0, _) => Some(Dp3src::Madd32a),
        (0b0, 0b00, 0b000, 0b1, _) => Some(Dp3src::Msub32a),
        (0b1, 0b00, 0b000, 0b0, _) => Some(Dp3src::Madd64a),
        (0b1, 0b00, 0b000, 0b1, _) => Some(Dp3src::Msub64a),
        (0b1, 0b00, 0b001, 0b0, _) => Some(Dp3src::Smaddl64wa),
        (0b1, 0b00, 0b001, 0b1, _) => Some(Dp3src::Smsubl64wa),
        (0b1, 0b00, 0b010, 0b0, 0b11111) => Some(Dp3src::Smulh64),
        (0b1, 0b00, 0b101, 0b0, _) => Some(Dp3src::Umaddl64wa),
        (0b1, 0b00, 0b101, 0b1, _) => Some(Dp3src::Umsubl64wa),
        (0b1, 0b00, 0b110, 0b0, 0b11111) => Some(Dp3src::Umulh64),
        _ => None,
    }
}

impl Dp2src {
    /// The handler class: CRC32 and CRC32C have their own, as does PACGA;
    /// the rest share the data-processing one. [`Dp2src::feature`] splits
    /// the same way.
    pub(crate) const fn class(self) -> Class {
        match self {
            Self::Udiv32
            | Self::Sdiv32
            | Self::Lslv32
            | Self::Lsrv32
            | Self::Asrv32
            | Self::Rorv32
            | Self::Udiv64
            | Self::Sdiv64
            | Self::Lslv64
            | Self::Lsrv64
            | Self::Asrv64
            | Self::Rorv64 => Class::DataProcessing2Source,
            Self::Crc32b32c
            | Self::Crc32h32c
            | Self::Crc32w32c
            | Self::Crc32cb32c
            | Self::Crc32ch32c
            | Self::Crc32cw32c
            | Self::Crc32x64c
            | Self::Crc32cx64c => Class::Crc32,
            Self::Pacga64p => Class::PointerAuthGeneric,
        }
    }
}

impl Dp2src {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Udiv32
            | Self::Sdiv32
            | Self::Lslv32
            | Self::Lsrv32
            | Self::Asrv32
            | Self::Rorv32
            | Self::Udiv64
            | Self::Sdiv64
            | Self::Lslv64
            | Self::Lsrv64
            | Self::Asrv64
            | Self::Rorv64 => Feature::Base,
            Self::Crc32b32c
            | Self::Crc32h32c
            | Self::Crc32w32c
            | Self::Crc32cb32c
            | Self::Crc32ch32c
            | Self::Crc32cw32c
            | Self::Crc32x64c
            | Self::Crc32cx64c => Feature::Crc32,
            Self::Pacga64p => Feature::PAuth,
        }
    }
}

impl Dp1src {
    /// The handler class, by [`Dp1src::feature`]: the pointer
    /// authentication instructions have their own (the handler reads which
    /// from `opcode`); the rest share the data-processing one.
    pub(crate) const fn class(self) -> Class {
        if matches!(self.feature(), Feature::PAuth) {
            Class::PointerAuth
        } else {
            Class::DataProcessing1Source
        }
    }

    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Rbit32
            | Self::Rev1632
            | Self::Rev32
            | Self::Clz32
            | Self::Cls32
            | Self::Rbit64
            | Self::Rev1664
            | Self::Rev3264
            | Self::Rev64
            | Self::Clz64
            | Self::Cls64 => Feature::Base,
            Self::Pacia64p
            | Self::Pacib64p
            | Self::Pacda64p
            | Self::Pacdb64p
            | Self::Autia64p
            | Self::Autib64p
            | Self::Autda64p
            | Self::Autdb64p
            | Self::Paciza64z
            | Self::Pacizb64z
            | Self::Pacdza64z
            | Self::Pacdzb64z
            | Self::Autiza64z
            | Self::Autizb64z
            | Self::Autdza64z
            | Self::Autdzb64z
            | Self::Xpaci64z
            | Self::Xpacd64z => Feature::PAuth,
        }
    }
}
