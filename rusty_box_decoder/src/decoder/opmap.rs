//! Primary opcode map — one-byte and two-byte (0F) opcode tables.
//!
//! Table naming follows Bochs convention: `BxOpcodeTableXX` where XX is the
//! opcode byte in hex. Matches Bochs `fetchdecode_opmap.h`.

#![allow(non_upper_case_globals, unused)]

use super::*;
use super::tables::*;
use crate::opcode::Opcode;

// opcode 00
pub(super) const BxOpcodeTable00: [u64; 1] = [last_opcode_lockable(0, Opcode::AddEbGb)];

// opcode 01
pub(super) const BxOpcodeTable01: [u64; 3] = [
    form_opcode_lockable(ATTR_OS64, Opcode::AddEqGq as _),
    form_opcode_lockable(ATTR_OS32, Opcode::AddEdGd),
    last_opcode_lockable(ATTR_OS16, Opcode::AddEwGw),
];

// opcode 02
pub(super) const BxOpcodeTable02: [u64; 1] = [last_opcode(0, Opcode::AddGbEb)];

// opcode 03
pub(super) const BxOpcodeTable03: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::AddGqEq),
    form_opcode(ATTR_OS32, Opcode::AddGdEd),
    last_opcode(ATTR_OS16, Opcode::AddGwEw),
];

// opcode 04
pub(super) const BxOpcodeTable04: [u64; 1] = [last_opcode(0, Opcode::AddAlib)];

// opcode 05
pub(super) const BxOpcodeTable05: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::AddRaxid),
    form_opcode(ATTR_OS32, Opcode::AddEaxid),
    last_opcode(ATTR_OS16, Opcode::AddAxiw),
];

// opcode 06
pub(super) const BxOpcodeTable06: [u64; 2] = [
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PushOp32Sw),
    last_opcode(ATTR_OS16 | ATTR_IS32, Opcode::PushOp16Sw),
];

// opcode 07
pub(super) const BxOpcodeTable07: [u64; 2] = [
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PopOp32Sw),
    last_opcode(ATTR_OS16 | ATTR_IS32, Opcode::PopOp16Sw),
];

// opcode 08
pub(super) const BxOpcodeTable08: [u64; 1] = [last_opcode_lockable(0, Opcode::OrEbGb)];

// opcode 09
pub(super) const BxOpcodeTable09: [u64; 3] = [
    form_opcode_lockable(ATTR_OS64, Opcode::OrEqGq),
    form_opcode_lockable(ATTR_OS32, Opcode::OrEdGd),
    last_opcode_lockable(ATTR_OS16, Opcode::OrEwGw),
];

// opcode 0A
pub(super) const BxOpcodeTable0A: [u64; 1] = [last_opcode(0, Opcode::OrGbEb)];

// opcode 0B
pub(super) const BxOpcodeTable0B: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::OrGqEq),
    form_opcode(ATTR_OS32, Opcode::OrGdEd),
    last_opcode(ATTR_OS16, Opcode::OrGwEw),
];

// opcode 0C
pub(super) const BxOpcodeTable0C: [u64; 1] = [last_opcode(0, Opcode::OrAlib)];

// opcode 0D
pub(super) const BxOpcodeTable0D: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::OrRaxid),
    form_opcode(ATTR_OS32, Opcode::OrEaxid),
    last_opcode(ATTR_OS16, Opcode::OrAxiw),
];

// opcode 0E
pub(super) const BxOpcodeTable0E: [u64; 2] = [
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PushOp32Sw),
    last_opcode(ATTR_OS16 | ATTR_IS32, Opcode::PushOp16Sw),
];

// opcode 10
pub(super) const BxOpcodeTable10: [u64; 1] = [last_opcode_lockable(0, Opcode::AdcEbGb)];

// opcode 11
pub(super) const BxOpcodeTable11: [u64; 3] = [
    form_opcode_lockable(ATTR_OS64, Opcode::AdcEqGq),
    form_opcode_lockable(ATTR_OS32, Opcode::AdcEdGd),
    last_opcode_lockable(ATTR_OS16, Opcode::AdcEwGw),
];

// opcode 12
pub(super) const BxOpcodeTable12: [u64; 1] = [last_opcode(0, Opcode::AdcGbEb)];

// opcode 13
pub(super) const BxOpcodeTable13: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::AdcGqEq),
    form_opcode(ATTR_OS32, Opcode::AdcGdEd),
    last_opcode(ATTR_OS16, Opcode::AdcGwEw),
];

// opcode 14
pub(super) const BxOpcodeTable14: [u64; 1] = [last_opcode(0, Opcode::AdcAlib)];

// opcode 15
pub(super) const BxOpcodeTable15: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::AdcRaxid),
    form_opcode(ATTR_OS32, Opcode::AdcEaxid),
    last_opcode(ATTR_OS16, Opcode::AdcAxiw),
];

// opcode 16
pub(super) const BxOpcodeTable16: [u64; 2] = [
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PushOp32Sw),
    last_opcode(ATTR_OS16 | ATTR_IS32, Opcode::PushOp16Sw),
];

// opcode 17
pub(super) const BxOpcodeTable17: [u64; 2] = [
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PopOp32Sw),
    last_opcode(ATTR_OS16 | ATTR_IS32, Opcode::PopOp16Sw),
];

// opcode 18
pub(super) const BxOpcodeTable18: [u64; 1] = [last_opcode_lockable(0, Opcode::SbbEbGb)];

// opcode 19
pub(super) const BxOpcodeTable19: [u64; 3] = [
    form_opcode_lockable(ATTR_OS64, Opcode::SbbEqGq),
    form_opcode_lockable(ATTR_OS32, Opcode::SbbEdGd),
    last_opcode_lockable(ATTR_OS16, Opcode::SbbEwGw),
];

// opcode 1A
pub(super) const BxOpcodeTable1A: [u64; 1] = [last_opcode(0, Opcode::SbbGbEb)];

// opcode 1B
pub(super) const BxOpcodeTable1B: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::SbbGqEq),
    form_opcode(ATTR_OS32, Opcode::SbbGdEd),
    last_opcode(ATTR_OS16, Opcode::SbbGwEw),
];

// opcode 1C
pub(super) const BxOpcodeTable1C: [u64; 1] = [last_opcode(0, Opcode::SbbAlib)];

// opcode 1D
pub(super) const BxOpcodeTable1D: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::SbbRaxid),
    form_opcode(ATTR_OS32, Opcode::SbbEaxid),
    last_opcode(ATTR_OS16, Opcode::SbbAxiw),
];

// opcode 1E
pub(super) const BxOpcodeTable1E: [u64; 2] = [
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PushOp32Sw),
    last_opcode(ATTR_OS16 | ATTR_IS32, Opcode::PushOp16Sw),
];

// opcode 1F
pub(super) const BxOpcodeTable1F: [u64; 2] = [
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PopOp32Sw),
    last_opcode(ATTR_OS16 | ATTR_IS32, Opcode::PopOp16Sw),
];

// opcode 20
pub(super) const BxOpcodeTable20: [u64; 1] = [last_opcode_lockable(0, Opcode::AndEbGb)];

// opcode 21
pub(super) const BxOpcodeTable21: [u64; 3] = [
    form_opcode_lockable(ATTR_OS64, Opcode::AndEqGq),
    form_opcode_lockable(ATTR_OS32, Opcode::AndEdGd),
    last_opcode_lockable(ATTR_OS16, Opcode::AndEwGw),
];

// opcode 22
pub(super) const BxOpcodeTable22: [u64; 1] = [last_opcode(0, Opcode::AndGbEb)];

// opcode 23
pub(super) const BxOpcodeTable23: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::AndGqEq),
    form_opcode(ATTR_OS32, Opcode::AndGdEd),
    last_opcode(ATTR_OS16, Opcode::AndGwEw),
];

// opcode 24
pub(super) const BxOpcodeTable24: [u64; 1] = [last_opcode(0, Opcode::AndAlib)];

// opcode 25
pub(super) const BxOpcodeTable25: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::AndRaxid),
    form_opcode(ATTR_OS32, Opcode::AndEaxid),
    last_opcode(ATTR_OS16, Opcode::AndAxiw),
];

// opcode 27
pub(super) const BxOpcodeTable27: [u64; 1] = [last_opcode(0, Opcode::Daa)];

// opcode 28
pub(super) const BxOpcodeTable28: [u64; 1] = [last_opcode_lockable(0, Opcode::SubEbGb)];

// opcode 29
pub(super) const BxOpcodeTable29: [u64; 6] = [
    form_opcode(ATTR_OS64 | ATTR_SRC_EQ_DST, Opcode::SubEqGqZeroIdiom),
    form_opcode(ATTR_OS32 | ATTR_SRC_EQ_DST, Opcode::SubEdGdZeroIdiom),
    form_opcode(ATTR_OS16 | ATTR_SRC_EQ_DST, Opcode::SubEwGwZeroIdiom),
    form_opcode_lockable(ATTR_OS64, Opcode::SubEqGq),
    form_opcode_lockable(ATTR_OS32, Opcode::SubEdGd),
    last_opcode_lockable(ATTR_OS16, Opcode::SubEwGw),
];

// opcode 2A
pub(super) const BxOpcodeTable2A: [u64; 1] = [last_opcode(0, Opcode::SubGbEb)];

// opcode 2B
pub(super) const BxOpcodeTable2B: [u64; 6] = [
    form_opcode(ATTR_OS64 | ATTR_SRC_EQ_DST, Opcode::SubGqEqZeroIdiom),
    form_opcode(ATTR_OS32 | ATTR_SRC_EQ_DST, Opcode::SubGdEdZeroIdiom),
    form_opcode(ATTR_OS16 | ATTR_SRC_EQ_DST, Opcode::SubGwEwZeroIdiom),
    form_opcode(ATTR_OS64, Opcode::SubGqEq),
    form_opcode(ATTR_OS32, Opcode::SubGdEd),
    last_opcode(ATTR_OS16, Opcode::SubGwEw),
];

// opcode 2C
pub(super) const BxOpcodeTable2C: [u64; 1] = [last_opcode(0, Opcode::SubAlib)];

// opcode 2D
pub(super) const BxOpcodeTable2D: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::SubRaxid),
    form_opcode(ATTR_OS32, Opcode::SubEaxid),
    last_opcode(ATTR_OS16, Opcode::SubAxiw),
];

// opcode 2F
pub(super) const BxOpcodeTable2F: [u64; 1] = [last_opcode(0, Opcode::Das)];

// opcode 30
pub(super) const BxOpcodeTable30: [u64; 1] = [last_opcode_lockable(0, Opcode::XorEbGb)];

// opcode 31
pub(super) const BxOpcodeTable31: [u64; 6] = [
    form_opcode(ATTR_OS64 | ATTR_SRC_EQ_DST, Opcode::XorEqGqZeroIdiom),
    form_opcode(ATTR_OS32 | ATTR_SRC_EQ_DST, Opcode::XorEdGdZeroIdiom),
    form_opcode(ATTR_OS16 | ATTR_SRC_EQ_DST, Opcode::XorEwGwZeroIdiom),
    form_opcode_lockable(ATTR_OS64, Opcode::XorEqGq),
    form_opcode_lockable(ATTR_OS32, Opcode::XorEdGd),
    last_opcode_lockable(ATTR_OS16, Opcode::XorEwGw),
];

// opcode 32
pub(super) const BxOpcodeTable32: [u64; 1] = [last_opcode(0, Opcode::XorGbEb)];

// opcode 33
pub(super) const BxOpcodeTable33: [u64; 6] = [
    form_opcode(ATTR_OS64 | ATTR_SRC_EQ_DST, Opcode::XorGqEqZeroIdiom),
    form_opcode(ATTR_OS32 | ATTR_SRC_EQ_DST, Opcode::XorGdEdZeroIdiom),
    form_opcode(ATTR_OS16 | ATTR_SRC_EQ_DST, Opcode::XorGwEwZeroIdiom),
    form_opcode(ATTR_OS64, Opcode::XorGqEq),
    form_opcode(ATTR_OS32, Opcode::XorGdEd),
    last_opcode(ATTR_OS16, Opcode::XorGwEw),
];

// opcode 34
pub(super) const BxOpcodeTable34: [u64; 1] = [last_opcode(0, Opcode::XorAlib)];

// opcode 35
pub(super) const BxOpcodeTable35: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::XorRaxid),
    form_opcode(ATTR_OS32, Opcode::XorEaxid),
    last_opcode(ATTR_OS16, Opcode::XorAxiw),
];

// opcode 37
pub(super) const BxOpcodeTable37: [u64; 1] = [last_opcode(0, Opcode::Aaa)];

// opcode 38
pub(super) const BxOpcodeTable38: [u64; 1] = [last_opcode(0, Opcode::CmpEbGb)];

// opcode 39
pub(super) const BxOpcodeTable39: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::CmpEqGq),
    form_opcode(ATTR_OS32, Opcode::CmpEdGd),
    last_opcode(ATTR_OS16, Opcode::CmpEwGw),
];

// opcode 3A
pub(super) const BxOpcodeTable3A: [u64; 1] = [last_opcode(0, Opcode::CmpGbEb)];

// opcode 3B
pub(super) const BxOpcodeTable3B: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::CmpGqEq),
    form_opcode(ATTR_OS32, Opcode::CmpGdEd),
    last_opcode(ATTR_OS16, Opcode::CmpGwEw),
];

// opcode 3C
pub(super) const BxOpcodeTable3C: [u64; 1] = [last_opcode(0, Opcode::CmpAlib)];

// opcode 3D
pub(super) const BxOpcodeTable3D: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::CmpRaxid),
    form_opcode(ATTR_OS32, Opcode::CmpEaxid),
    last_opcode(ATTR_OS16, Opcode::CmpAxiw),
];

// opcode 3F
pub(super) const BxOpcodeTable3F: [u64; 1] = [last_opcode(0, Opcode::Aas)];

// opcode 40 - 47
pub(super) const BxOpcodeTable40x47: [u64; 2] = [
    form_opcode_lockable(ATTR_OS32 | ATTR_IS32, Opcode::IncEd),
    last_opcode_lockable(ATTR_OS16 | ATTR_IS32, Opcode::IncEw),
];

// opcode 48 - 4F
pub(super) const BxOpcodeTable48x4F: [u64; 2] = [
    form_opcode_lockable(ATTR_OS32 | ATTR_IS32, Opcode::DecEd),
    last_opcode_lockable(ATTR_OS16 | ATTR_IS32, Opcode::DecEw),
];

// opcode 50 - 57
pub(super) const BxOpcodeTable50x57: [u64; 3] = [
    form_opcode(ATTR_OS32_64 | ATTR_IS64, Opcode::PushEq),
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PushEd),
    last_opcode(ATTR_OS16, Opcode::PushEw),
];

// opcode 58 - 5F
pub(super) const BxOpcodeTable58x5F: [u64; 3] = [
    form_opcode(ATTR_OS32_64 | ATTR_IS64, Opcode::PopEq),
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PopEd),
    last_opcode(ATTR_OS16, Opcode::PopEw),
];

// opcode 60
pub(super) const BxOpcodeTable60: [u64; 2] = [
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PushaOp32),
    last_opcode(ATTR_OS16 | ATTR_IS32, Opcode::PushaOp16),
];

// opcode 61
pub(super) const BxOpcodeTable61: [u64; 2] = [
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PopaOp32),
    last_opcode(ATTR_OS16 | ATTR_IS32, Opcode::PopaOp16),
];

// opcode 62 - EVEX prefix
pub(super) const BxOpcodeTable62: [u64; 2] = [
    form_opcode(ATTR_OS32 | ATTR_MOD_MEM | ATTR_IS32, Opcode::BoundGdMa),
    last_opcode(ATTR_OS16 | ATTR_MOD_MEM | ATTR_IS32, Opcode::BoundGwMa),
];

// opcode 63
pub(super) const BxOpcodeTable63_32: [u64; 1] = [last_opcode(ATTR_OS16_32, Opcode::ArplEwGw)];
pub(super) const BxOpcodeTable63_64: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::MovsxdGqEd),
    form_opcode(ATTR_OS32, Opcode::MovOp64GdEd), // MOVSX_GdEd
    last_opcode(ATTR_OS16, Opcode::MovGwEw),     // MOVSX_GwEw
];

// opcode 68
pub(super) const BxOpcodeTable68: [u64; 3] = [
    form_opcode(ATTR_OS32_64 | ATTR_IS64, Opcode::PushOp64Id),
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PushId),
    last_opcode(ATTR_OS16, Opcode::PushIw),
];

// opcode 69
pub(super) const BxOpcodeTable69: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::ImulGqEqId),
    form_opcode(ATTR_OS32, Opcode::ImulGdEdId),
    last_opcode(ATTR_OS16, Opcode::ImulGwEwIw),
];

// opcode 6A
pub(super) const BxOpcodeTable6A: [u64; 3] = [
    form_opcode(ATTR_OS32_64 | ATTR_IS64, Opcode::PushOp64SIb),
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PushSIb32), // imm8 sign extended to 32-bit
    last_opcode(ATTR_OS16, Opcode::PushSIb16),             // imm8 sign extended to 16-bit
];

// opcode 6B
pub(super) const BxOpcodeTable6B: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::ImulGqEqsIb),
    form_opcode(ATTR_OS32, Opcode::ImulGdEdsIb),
    last_opcode(ATTR_OS16, Opcode::ImulGwEwsIb),
];

// opcode 6C
pub(super) const BxOpcodeTable6C: [u64; 1] = [last_opcode(0, Opcode::RepInsbYbDx)];

// opcode 6D
pub(super) const BxOpcodeTable6D: [u64; 2] = [
    form_opcode(ATTR_OS32_64, Opcode::RepInsdYdDx),
    last_opcode(ATTR_OS16, Opcode::RepInswYwDx),
];

// opcode 6E
pub(super) const BxOpcodeTable6E: [u64; 1] = [last_opcode(0, Opcode::RepOutsbDxxb)];

// opcode 6F
pub(super) const BxOpcodeTable6F: [u64; 2] = [
    form_opcode(ATTR_OS32_64, Opcode::RepOutsdDxxd),
    last_opcode(ATTR_OS16, Opcode::RepOutswDxxw),
];

// opcode 70
pub(super) const BxOpcodeTable70_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JoJbd),
    last_opcode(ATTR_OS16, Opcode::JoJbw),
];

pub(super) const BxOpcodeTable70_64: [u64; 1] = [last_opcode(0, Opcode::JoJbq)];

// opcode 71
pub(super) const BxOpcodeTable71_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnoJbd),
    last_opcode(ATTR_OS16, Opcode::JnoJbw),
];

pub(super) const BxOpcodeTable71_64: [u64; 1] = [last_opcode(0, Opcode::JnoJbq)];

// opcode 72
pub(super) const BxOpcodeTable72_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JbJbd),
    last_opcode(ATTR_OS16, Opcode::JbJbw),
];

pub(super) const BxOpcodeTable72_64: [u64; 1] = [last_opcode(0, Opcode::JbJbq)];

// opcode 73
pub(super) const BxOpcodeTable73_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnbJbd),
    last_opcode(ATTR_OS16, Opcode::JnbJbw),
];

pub(super) const BxOpcodeTable73_64: [u64; 1] = [last_opcode(0, Opcode::JnbJbq)];

// opcode 74
pub(super) const BxOpcodeTable74_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JzJbd),
    last_opcode(ATTR_OS16, Opcode::JzJbw),
];

pub(super) const BxOpcodeTable74_64: [u64; 1] = [last_opcode(0, Opcode::JzJbq)];

// opcode 75
pub(super) const BxOpcodeTable75_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnzJbd),
    last_opcode(ATTR_OS16, Opcode::JnzJbw),
];

pub(super) const BxOpcodeTable75_64: [u64; 1] = [last_opcode(0, Opcode::JnzJbq)];

// opcode 76
pub(super) const BxOpcodeTable76_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JbeJbd),
    last_opcode(ATTR_OS16, Opcode::JbeJbw),
];

pub(super) const BxOpcodeTable76_64: [u64; 1] = [last_opcode(0, Opcode::JbeJbq)];

// opcode 77
pub(super) const BxOpcodeTable77_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnbeJbd),
    last_opcode(ATTR_OS16, Opcode::JnbeJbw),
];

pub(super) const BxOpcodeTable77_64: [u64; 1] = [last_opcode(0, Opcode::JnbeJbq)];

// opcode 78
pub(super) const BxOpcodeTable78_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JsJbd),
    last_opcode(ATTR_OS16, Opcode::JsJbw),
];

pub(super) const BxOpcodeTable78_64: [u64; 1] = [last_opcode(0, Opcode::JsJbq)];

// opcode 79
pub(super) const BxOpcodeTable79_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnsJbd),
    last_opcode(ATTR_OS16, Opcode::JnsJbw),
];

pub(super) const BxOpcodeTable79_64: [u64; 1] = [last_opcode(0, Opcode::JnsJbq)];

// opcode 7A
pub(super) const BxOpcodeTable7A_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JpJbd),
    last_opcode(ATTR_OS16, Opcode::JpJbw),
];

pub(super) const BxOpcodeTable7A_64: [u64; 1] = [last_opcode(0, Opcode::JpJbq)];

// opcode 7B
pub(super) const BxOpcodeTable7B_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnpJbd),
    last_opcode(ATTR_OS16, Opcode::JnpJbw),
];

pub(super) const BxOpcodeTable7B_64: [u64; 1] = [last_opcode(0, Opcode::JnpJbq)];

// opcode 7C
pub(super) const BxOpcodeTable7C_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JlJbd),
    last_opcode(ATTR_OS16, Opcode::JlJbw),
];

pub(super) const BxOpcodeTable7C_64: [u64; 1] = [last_opcode(0, Opcode::JlJbq)];

// opcode 7D
pub(super) const BxOpcodeTable7D_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnlJbd),
    last_opcode(ATTR_OS16, Opcode::JnlJbw),
];

pub(super) const BxOpcodeTable7D_64: [u64; 1] = [last_opcode(0, Opcode::JnlJbq)];

// opcode 7E
pub(super) const BxOpcodeTable7E_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JleJbd),
    last_opcode(ATTR_OS16, Opcode::JleJbw),
];

pub(super) const BxOpcodeTable7E_64: [u64; 1] = [last_opcode(0, Opcode::JleJbq)];

// opcode 7F
pub(super) const BxOpcodeTable7F_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnleJbd),
    last_opcode(ATTR_OS16, Opcode::JnleJbw),
];

pub(super) const BxOpcodeTable7F_64: [u64; 1] = [last_opcode(0, Opcode::JnleJbq)];

// opcode 80
pub(super) const BxOpcodeTable80: [u64; 8] = [
    form_opcode_lockable(ATTR_NNN0, Opcode::AddEbIb),
    form_opcode_lockable(ATTR_NNN1, Opcode::OrEbIb),
    form_opcode_lockable(ATTR_NNN2, Opcode::AdcEbIb),
    form_opcode_lockable(ATTR_NNN3, Opcode::SbbEbIb),
    form_opcode_lockable(ATTR_NNN4, Opcode::AndEbIb),
    form_opcode_lockable(ATTR_NNN5, Opcode::SubEbIb),
    form_opcode_lockable(ATTR_NNN6, Opcode::XorEbIb),
    last_opcode(ATTR_NNN7, Opcode::CmpEbIb),
];

// opcode 81
pub(super) const BxOpcodeTable81: [u64; 24] = [
    form_opcode_lockable(ATTR_NNN0 | ATTR_OS64, Opcode::AddEqId),
    form_opcode_lockable(ATTR_NNN1 | ATTR_OS64, Opcode::OrEqId),
    form_opcode_lockable(ATTR_NNN2 | ATTR_OS64, Opcode::AdcEqId),
    form_opcode_lockable(ATTR_NNN3 | ATTR_OS64, Opcode::SbbEqId),
    form_opcode_lockable(ATTR_NNN4 | ATTR_OS64, Opcode::AndEqId),
    form_opcode_lockable(ATTR_NNN5 | ATTR_OS64, Opcode::SubEqId),
    form_opcode_lockable(ATTR_NNN6 | ATTR_OS64, Opcode::XorEqId),
    form_opcode(ATTR_NNN7 | ATTR_OS64, Opcode::CmpEqId),
    form_opcode_lockable(ATTR_NNN0 | ATTR_OS32, Opcode::AddEdId),
    form_opcode_lockable(ATTR_NNN1 | ATTR_OS32, Opcode::OrEdId),
    form_opcode_lockable(ATTR_NNN2 | ATTR_OS32, Opcode::AdcEdId),
    form_opcode_lockable(ATTR_NNN3 | ATTR_OS32, Opcode::SbbEdId),
    form_opcode_lockable(ATTR_NNN4 | ATTR_OS32, Opcode::AndEdId),
    form_opcode_lockable(ATTR_NNN5 | ATTR_OS32, Opcode::SubEdId),
    form_opcode_lockable(ATTR_NNN6 | ATTR_OS32, Opcode::XorEdId),
    form_opcode(ATTR_NNN7 | ATTR_OS32, Opcode::CmpEdId),
    form_opcode_lockable(ATTR_NNN0 | ATTR_OS16, Opcode::AddEwIw),
    form_opcode_lockable(ATTR_NNN1 | ATTR_OS16, Opcode::OrEwIw),
    form_opcode_lockable(ATTR_NNN2 | ATTR_OS16, Opcode::AdcEwIw),
    form_opcode_lockable(ATTR_NNN3 | ATTR_OS16, Opcode::SbbEwIw),
    form_opcode_lockable(ATTR_NNN4 | ATTR_OS16, Opcode::AndEwIw),
    form_opcode_lockable(ATTR_NNN5 | ATTR_OS16, Opcode::SubEwIw),
    form_opcode_lockable(ATTR_NNN6 | ATTR_OS16, Opcode::XorEwIw),
    last_opcode(ATTR_NNN7 | ATTR_OS16, Opcode::CmpEwIw),
];

// opcode 83
pub(super) const BxOpcodeTable83: [u64; 24] = [
    form_opcode_lockable(ATTR_NNN0 | ATTR_OS64, Opcode::AddEqsIb),
    form_opcode_lockable(ATTR_NNN1 | ATTR_OS64, Opcode::OrEqsIb),
    form_opcode_lockable(ATTR_NNN2 | ATTR_OS64, Opcode::AdcEqsIb),
    form_opcode_lockable(ATTR_NNN3 | ATTR_OS64, Opcode::SbbEqsIb),
    form_opcode_lockable(ATTR_NNN4 | ATTR_OS64, Opcode::AndEqsIb),
    form_opcode_lockable(ATTR_NNN5 | ATTR_OS64, Opcode::SubEqsIb),
    form_opcode_lockable(ATTR_NNN6 | ATTR_OS64, Opcode::XorEqsIb),
    form_opcode(ATTR_NNN7 | ATTR_OS64, Opcode::CmpEqsIb),
    form_opcode_lockable(ATTR_NNN0 | ATTR_OS32, Opcode::AddEdsIb),
    form_opcode_lockable(ATTR_NNN1 | ATTR_OS32, Opcode::OrEdsIb),
    form_opcode_lockable(ATTR_NNN2 | ATTR_OS32, Opcode::AdcEdsIb),
    form_opcode_lockable(ATTR_NNN3 | ATTR_OS32, Opcode::SbbEdsIb),
    form_opcode_lockable(ATTR_NNN4 | ATTR_OS32, Opcode::AndEdsIb),
    form_opcode_lockable(ATTR_NNN5 | ATTR_OS32, Opcode::SubEdsIb),
    form_opcode_lockable(ATTR_NNN6 | ATTR_OS32, Opcode::XorEdsIb),
    form_opcode(ATTR_NNN7 | ATTR_OS32, Opcode::CmpEdsIb),
    form_opcode_lockable(ATTR_NNN0 | ATTR_OS16, Opcode::AddEwsIb),
    form_opcode_lockable(ATTR_NNN1 | ATTR_OS16, Opcode::OrEwsIb),
    form_opcode_lockable(ATTR_NNN2 | ATTR_OS16, Opcode::AdcEwsIb),
    form_opcode_lockable(ATTR_NNN3 | ATTR_OS16, Opcode::SbbEwsIb),
    form_opcode_lockable(ATTR_NNN4 | ATTR_OS16, Opcode::AndEwsIb),
    form_opcode_lockable(ATTR_NNN5 | ATTR_OS16, Opcode::SubEwsIb),
    form_opcode_lockable(ATTR_NNN6 | ATTR_OS16, Opcode::XorEwsIb),
    last_opcode(ATTR_NNN7 | ATTR_OS16, Opcode::CmpEwsIb),
];

// opcode 84
pub(super) const BxOpcodeTable84: [u64; 1] = [last_opcode(0, Opcode::TestEbGb)];

// opcode 85
pub(super) const BxOpcodeTable85: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::TestEqGq),
    form_opcode(ATTR_OS32, Opcode::TestEdGd),
    last_opcode(ATTR_OS16, Opcode::TestEwGw),
];

// opcode 86
pub(super) const BxOpcodeTable86: [u64; 1] = [last_opcode_lockable(0, Opcode::XchgEbGb)];

// opcode 87
pub(super) const BxOpcodeTable87: [u64; 3] = [
    form_opcode_lockable(ATTR_OS64, Opcode::XchgEqGq),
    form_opcode_lockable(ATTR_OS32, Opcode::XchgEdGd),
    last_opcode_lockable(ATTR_OS16, Opcode::XchgEwGw),
];

// opcode 88
pub(super) const BxOpcodeTable88: [u64; 1] = [last_opcode(0, Opcode::MovEbGb)];

// opcode 89 - split for better emulation performance
pub(super) const BxOpcodeTable89: [u64; 4] = [
    form_opcode(ATTR_OS64, Opcode::MovEqGq),
    form_opcode(ATTR_OS32 | ATTR_IS64, Opcode::MovOp64EdGd),
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::MovOp32EdGd),
    last_opcode(ATTR_OS16, Opcode::MovEwGw),
];

// opcode 8A
pub(super) const BxOpcodeTable8A: [u64; 1] = [last_opcode(0, Opcode::MovGbEb)];

// opcode 8B - split for better emulation performance
pub(super) const BxOpcodeTable8B: [u64; 4] = [
    form_opcode(ATTR_OS64, Opcode::MovGqEq),
    form_opcode(ATTR_OS32 | ATTR_IS64, Opcode::MovOp64GdEd),
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::MovOp32GdEd),
    last_opcode(ATTR_OS16, Opcode::MovGwEw),
];

// opcode 8C
pub(super) const BxOpcodeTable8C: [u64; 1] = [last_opcode(0, Opcode::MovEwSw)];

// opcode 8D
pub(super) const BxOpcodeTable8D: [u64; 3] = [
    form_opcode(ATTR_OS64 | ATTR_MOD_MEM, Opcode::LeaGqM),
    form_opcode(ATTR_OS32 | ATTR_MOD_MEM, Opcode::LeaGdM),
    last_opcode(ATTR_OS16 | ATTR_MOD_MEM, Opcode::LeaGwM),
];

// opcode 8E
pub(super) const BxOpcodeTable8E: [u64; 1] = [last_opcode(0, Opcode::MovSwEw)];

// opcode 8F - XOP prefix
pub(super) const BxOpcodeTable8F: [u64; 3] = [
    form_opcode(ATTR_IS64 | ATTR_OS32_64 | ATTR_NNN0, Opcode::PopEq),
    form_opcode(ATTR_IS32 | ATTR_OS32 | ATTR_NNN0, Opcode::PopEd),
    last_opcode(ATTR_OS16 | ATTR_NNN0, Opcode::PopEw),
];

// opcode 90 - 97
pub(super) const BxOpcodeTable90x97: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::XchgRrxRax), // handles also XCHG R8, RAX
    form_opcode(ATTR_OS32, Opcode::XchgErxEax), // handles also XCHG R8d, EAX
    last_opcode(ATTR_OS16, Opcode::XchgRxax),   // handles also XCHG R8w, AX
];

// opcode 98
pub(super) const BxOpcodeTable98: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::Cdqe),
    form_opcode(ATTR_OS32, Opcode::Cwde),
    last_opcode(ATTR_OS16, Opcode::Cbw),
];

// opcode 99
pub(super) const BxOpcodeTable99: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::Cqo),
    form_opcode(ATTR_OS32, Opcode::Cdq),
    last_opcode(ATTR_OS16, Opcode::Cwd),
];

// opcode 9A
pub(super) const BxOpcodeTable9A: [u64; 2] = [
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::CallfOp32Ap),
    last_opcode(ATTR_OS16 | ATTR_IS32, Opcode::CallfOp16Ap),
];

// opcode 9B
pub(super) const BxOpcodeTable9B: [u64; 1] = [last_opcode(0, Opcode::Fwait)];

// opcode 9C
pub(super) const BxOpcodeTable9C: [u64; 3] = [
    form_opcode(ATTR_OS32_64 | ATTR_IS64, Opcode::PushfFq),
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PushfFd),
    last_opcode(ATTR_OS16, Opcode::PushfFw),
];

// opcode 9D
pub(super) const BxOpcodeTable9D: [u64; 3] = [
    form_opcode(ATTR_OS32_64 | ATTR_IS64, Opcode::PopfFq),
    form_opcode(ATTR_OS32 | ATTR_IS32, Opcode::PopfFd),
    last_opcode(ATTR_OS16, Opcode::PopfFw),
];

// opcode 9E
pub(super) const BxOpcodeTable9E_32: [u64; 1] = [last_opcode(0, Opcode::Sahf)];
pub(super) const BxOpcodeTable9E_64: [u64; 1] = [last_opcode(0, Opcode::SahfLm)];

// opcode 9F
pub(super) const BxOpcodeTable9F_32: [u64; 1] = [last_opcode(0, Opcode::Lahf)];
pub(super) const BxOpcodeTable9F_64: [u64; 1] = [last_opcode(0, Opcode::LahfLm)];

// opcode A0
pub(super) const BxOpcodeTableA0_32: [u64; 1] = [last_opcode(0, Opcode::MovAlod)];
pub(super) const BxOpcodeTableA0_64: [u64; 1] = [last_opcode(0, Opcode::MovAloq)];

// opcode A1
pub(super) const BxOpcodeTableA1_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::MovEaxod),
    last_opcode(ATTR_OS16, Opcode::MovAxod),
];

pub(super) const BxOpcodeTableA1_64: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::MovRaxoq),
    form_opcode(ATTR_OS32, Opcode::MovEaxoq),
    last_opcode(ATTR_OS16, Opcode::MovAxoq),
];

// opcode A2
pub(super) const BxOpcodeTableA2_32: [u64; 1] = [last_opcode(0, Opcode::MovOdAl)];
pub(super) const BxOpcodeTableA2_64: [u64; 1] = [last_opcode(0, Opcode::MovOqAl)];

// opcode A3
pub(super) const BxOpcodeTableA3_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::MovOdEax),
    last_opcode(ATTR_OS16, Opcode::MovOdAx),
];

pub(super) const BxOpcodeTableA3_64: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::MovOqRax),
    form_opcode(ATTR_OS32, Opcode::MovOqEax),
    last_opcode(ATTR_OS16, Opcode::MovOqAx),
];

// opcode A4
pub(super) const BxOpcodeTableA4: [u64; 1] = [last_opcode(0, Opcode::RepMovsbYbXb)];

// opcode A5
pub(super) const BxOpcodeTableA5: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::RepMovsqYqXq),
    form_opcode(ATTR_OS32, Opcode::RepMovsdYdXd),
    last_opcode(ATTR_OS16, Opcode::RepMovswYwXw),
];

// opcode A6
pub(super) const BxOpcodeTableA6: [u64; 1] = [last_opcode(0, Opcode::RepCmpsbXbYb)];

// opcode A7
pub(super) const BxOpcodeTableA7: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::RepCmpsqXqYq),
    form_opcode(ATTR_OS32, Opcode::RepCmpsdXdYd),
    last_opcode(ATTR_OS16, Opcode::RepCmpswXwYw),
];

// opcode A8
pub(super) const BxOpcodeTableA8: [u64; 1] = [last_opcode(0, Opcode::TestAlib)];

// opcode A9
pub(super) const BxOpcodeTableA9: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::TestRaxid),
    form_opcode(ATTR_OS32, Opcode::TestEaxid),
    last_opcode(ATTR_OS16, Opcode::TestAxiw),
];

// opcode AA
pub(super) const BxOpcodeTableAA: [u64; 1] = [last_opcode(0, Opcode::RepStosbYbAl)];

// opcode AB
pub(super) const BxOpcodeTableAB: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::RepStosqYqRax),
    form_opcode(ATTR_OS32, Opcode::RepStosdYdEax),
    last_opcode(ATTR_OS16, Opcode::RepStoswYwAx),
];

// opcode AC
pub(super) const BxOpcodeTableAC: [u64; 1] = [last_opcode(0, Opcode::RepLodsbAlxb)];

// opcode AD
pub(super) const BxOpcodeTableAD: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::RepLodsqRaxxq),
    form_opcode(ATTR_OS32, Opcode::RepLodsdEaxxd),
    last_opcode(ATTR_OS16, Opcode::RepLodswAxxw),
];

// opcode AE
pub(super) const BxOpcodeTableAE: [u64; 1] = [last_opcode(0, Opcode::RepScasbAlyb)];

// opcode AF
pub(super) const BxOpcodeTableAF: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::RepScasqRaxyq),
    form_opcode(ATTR_OS32, Opcode::RepScasdEaxyd),
    last_opcode(ATTR_OS16, Opcode::RepScaswAxyw),
];

// opcode B0 - B7
pub(super) const BxOpcodeTableB0xB7: [u64; 1] = [last_opcode(0, Opcode::MovEbIb)];

// opcode B8 - BF
pub(super) const BxOpcodeTableB8xBF: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::MovRrxiq),
    form_opcode(ATTR_OS32, Opcode::MovEdId),
    last_opcode(ATTR_OS16, Opcode::MovEwIw),
];

// opcode D0
pub(super) const BxOpcodeTableC0: [u64; 8] = [
    form_opcode(ATTR_NNN0, Opcode::RolEbIb),
    form_opcode(ATTR_NNN1, Opcode::RorEbIb),
    form_opcode(ATTR_NNN2, Opcode::RclEbIb),
    form_opcode(ATTR_NNN3, Opcode::RcrEbIb),
    form_opcode(ATTR_NNN4, Opcode::ShlEbIb),
    form_opcode(ATTR_NNN5, Opcode::ShrEbIb),
    form_opcode(ATTR_NNN6, Opcode::ShlEbIb),
    last_opcode(ATTR_NNN7, Opcode::SarEbIb),
];

// opcode C1
pub(super) const BxOpcodeTableC1: [u64; 24] = [
    form_opcode(ATTR_NNN0 | ATTR_OS64, Opcode::RolEqIb),
    form_opcode(ATTR_NNN1 | ATTR_OS64, Opcode::RorEqIb),
    form_opcode(ATTR_NNN2 | ATTR_OS64, Opcode::RclEqIb),
    form_opcode(ATTR_NNN3 | ATTR_OS64, Opcode::RcrEqIb),
    form_opcode(ATTR_NNN4 | ATTR_OS64, Opcode::ShlEqIb),
    form_opcode(ATTR_NNN5 | ATTR_OS64, Opcode::ShrEqIb),
    form_opcode(ATTR_NNN6 | ATTR_OS64, Opcode::ShlEqIb),
    form_opcode(ATTR_NNN7 | ATTR_OS64, Opcode::SarEqIb),
    form_opcode(ATTR_NNN0 | ATTR_OS32, Opcode::RolEdIb),
    form_opcode(ATTR_NNN1 | ATTR_OS32, Opcode::RorEdIb),
    form_opcode(ATTR_NNN2 | ATTR_OS32, Opcode::RclEdIb),
    form_opcode(ATTR_NNN3 | ATTR_OS32, Opcode::RcrEdIb),
    form_opcode(ATTR_NNN4 | ATTR_OS32, Opcode::ShlEdIb),
    form_opcode(ATTR_NNN5 | ATTR_OS32, Opcode::ShrEdIb),
    form_opcode(ATTR_NNN6 | ATTR_OS32, Opcode::ShlEdIb),
    form_opcode(ATTR_NNN7 | ATTR_OS32, Opcode::SarEdIb),
    form_opcode(ATTR_NNN0 | ATTR_OS16, Opcode::RolEwIb),
    form_opcode(ATTR_NNN1 | ATTR_OS16, Opcode::RorEwIb),
    form_opcode(ATTR_NNN2 | ATTR_OS16, Opcode::RclEwIb),
    form_opcode(ATTR_NNN3 | ATTR_OS16, Opcode::RcrEwIb),
    form_opcode(ATTR_NNN4 | ATTR_OS16, Opcode::ShlEwIb),
    form_opcode(ATTR_NNN5 | ATTR_OS16, Opcode::ShrEwIb),
    form_opcode(ATTR_NNN6 | ATTR_OS16, Opcode::ShlEwIb),
    last_opcode(ATTR_NNN7 | ATTR_OS16, Opcode::SarEwIb),
];

// opcode C2
pub(super) const BxOpcodeTableC2_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::RetOp32Iw),
    last_opcode(ATTR_OS16, Opcode::RetOp16Iw),
];

pub(super) const BxOpcodeTableC2_64: [u64; 1] = [last_opcode(0, Opcode::RetOp64Iw)];

// opcode C3
pub(super) const BxOpcodeTableC3_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::RetOp32),
    last_opcode(ATTR_OS16, Opcode::RetOp16),
];

pub(super) const BxOpcodeTableC3_64: [u64; 1] = [last_opcode(0, Opcode::RetOp64)];

// opcode C4 - VEX prefix
pub(super) const BxOpcodeTableC4_32: [u64; 2] = [
    form_opcode(ATTR_OS32 | ATTR_MOD_MEM | ATTR_IS32, Opcode::LesGdMp),
    last_opcode(ATTR_OS16 | ATTR_MOD_MEM | ATTR_IS32, Opcode::LesGwMp),
];

// opcode C5 - VEX prefix
pub(super) const BxOpcodeTableC5_32: [u64; 2] = [
    form_opcode(ATTR_OS32 | ATTR_MOD_MEM | ATTR_IS32, Opcode::LdsGdMp),
    last_opcode(ATTR_OS16 | ATTR_MOD_MEM | ATTR_IS32, Opcode::LdsGwMp),
];

// opcode C6
pub(super) const BxOpcodeTableC6: [u64; 1] = [last_opcode(ATTR_NNN0, Opcode::MovEbIb)];

// opcode C7
pub(super) const BxOpcodeTableC7: [u64; 3] = [
    form_opcode(ATTR_NNN0 | ATTR_OS64, Opcode::MovEqId),
    form_opcode(ATTR_NNN0 | ATTR_OS32, Opcode::MovEdId),
    last_opcode(ATTR_NNN0 | ATTR_OS16, Opcode::MovEwIw),
];

// opcode C8
pub(super) const BxOpcodeTableC8_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::EnterOp32IwIb),
    last_opcode(ATTR_OS16, Opcode::EnterOp16IwIb),
];

pub(super) const BxOpcodeTableC8_64: [u64; 1] = [last_opcode(0, Opcode::EnterOp64IwIb)];

// opcode C9
pub(super) const BxOpcodeTableC9_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::LeaveOp32),
    last_opcode(ATTR_OS16, Opcode::LeaveOp16),
];

pub(super) const BxOpcodeTableC9_64: [u64; 1] = [last_opcode(0, Opcode::LeaveOp64)];

// opcode CA
pub(super) const BxOpcodeTableCA: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::RetfOp64Iw),
    form_opcode(ATTR_OS32, Opcode::RetfOp32Iw),
    last_opcode(ATTR_OS16, Opcode::RetfOp16Iw),
];

// opcode CB
pub(super) const BxOpcodeTableCB: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::RetfOp64),
    form_opcode(ATTR_OS32, Opcode::RetfOp32),
    last_opcode(ATTR_OS16, Opcode::RetfOp16),
];

// opcode CC
pub(super) const BxOpcodeTableCC: [u64; 1] = [last_opcode(0, Opcode::INT3)];

// opcode CD
pub(super) const BxOpcodeTableCD: [u64; 1] = [last_opcode_lockable(0, Opcode::IntIb)];

// opcode CE
pub(super) const BxOpcodeTableCE: [u64; 1] = [last_opcode(0, Opcode::Int0)];

// opcode CF
pub(super) const BxOpcodeTableCF_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::IretOp32),
    last_opcode(ATTR_OS16, Opcode::IretOp16),
];

pub(super) const BxOpcodeTableCF_64: [u64; 1] = [last_opcode(0, Opcode::IretOp64)];

// opcode D0
pub(super) const BxOpcodeTableD0: [u64; 8] = [
    form_opcode(ATTR_NNN0, Opcode::RolEbI1),
    form_opcode(ATTR_NNN1, Opcode::RorEbI1),
    form_opcode(ATTR_NNN2, Opcode::RclEbI1),
    form_opcode(ATTR_NNN3, Opcode::RcrEbI1),
    form_opcode(ATTR_NNN4, Opcode::ShlEbI1),
    form_opcode(ATTR_NNN5, Opcode::ShrEbI1),
    form_opcode(ATTR_NNN6, Opcode::ShlEbI1),
    last_opcode(ATTR_NNN7, Opcode::SarEbI1),
];

// opcode D1
pub(super) const BxOpcodeTableD1: [u64; 24] = [
    form_opcode(ATTR_NNN0 | ATTR_OS64, Opcode::RolEqI1),
    form_opcode(ATTR_NNN1 | ATTR_OS64, Opcode::RorEqI1),
    form_opcode(ATTR_NNN2 | ATTR_OS64, Opcode::RclEqI1),
    form_opcode(ATTR_NNN3 | ATTR_OS64, Opcode::RcrEqI1),
    form_opcode(ATTR_NNN4 | ATTR_OS64, Opcode::ShlEqI1),
    form_opcode(ATTR_NNN5 | ATTR_OS64, Opcode::ShrEqI1),
    form_opcode(ATTR_NNN6 | ATTR_OS64, Opcode::ShlEqI1),
    form_opcode(ATTR_NNN7 | ATTR_OS64, Opcode::SarEqI1),
    form_opcode(ATTR_NNN0 | ATTR_OS32, Opcode::RolEdI1),
    form_opcode(ATTR_NNN1 | ATTR_OS32, Opcode::RorEdI1),
    form_opcode(ATTR_NNN2 | ATTR_OS32, Opcode::RclEdI1),
    form_opcode(ATTR_NNN3 | ATTR_OS32, Opcode::RcrEdI1),
    form_opcode(ATTR_NNN4 | ATTR_OS32, Opcode::ShlEdI1),
    form_opcode(ATTR_NNN5 | ATTR_OS32, Opcode::ShrEdI1),
    form_opcode(ATTR_NNN6 | ATTR_OS32, Opcode::ShlEdI1),
    form_opcode(ATTR_NNN7 | ATTR_OS32, Opcode::SarEdI1),
    form_opcode(ATTR_NNN0 | ATTR_OS16, Opcode::RolEwI1),
    form_opcode(ATTR_NNN1 | ATTR_OS16, Opcode::RorEwI1),
    form_opcode(ATTR_NNN2 | ATTR_OS16, Opcode::RclEwI1),
    form_opcode(ATTR_NNN3 | ATTR_OS16, Opcode::RcrEwI1),
    form_opcode(ATTR_NNN4 | ATTR_OS16, Opcode::ShlEwI1),
    form_opcode(ATTR_NNN5 | ATTR_OS16, Opcode::ShrEwI1),
    form_opcode(ATTR_NNN6 | ATTR_OS16, Opcode::ShlEwI1),
    last_opcode(ATTR_NNN7 | ATTR_OS16, Opcode::SarEwI1),
];

// opcode D2
pub(super) const BxOpcodeTableD2: [u64; 8] = [
    form_opcode(ATTR_NNN0, Opcode::RolEb),
    form_opcode(ATTR_NNN1, Opcode::RorEb),
    form_opcode(ATTR_NNN2, Opcode::RclEb),
    form_opcode(ATTR_NNN3, Opcode::RcrEb),
    form_opcode(ATTR_NNN4, Opcode::ShlEb),
    form_opcode(ATTR_NNN5, Opcode::ShrEb),
    form_opcode(ATTR_NNN6, Opcode::ShlEb),
    last_opcode(ATTR_NNN7, Opcode::SarEb),
];

// opcode D3
pub(super) const BxOpcodeTableD3: [u64; 24] = [
    form_opcode(ATTR_NNN0 | ATTR_OS64, Opcode::RolEq),
    form_opcode(ATTR_NNN1 | ATTR_OS64, Opcode::RorEq),
    form_opcode(ATTR_NNN2 | ATTR_OS64, Opcode::RclEq),
    form_opcode(ATTR_NNN3 | ATTR_OS64, Opcode::RcrEq),
    form_opcode(ATTR_NNN4 | ATTR_OS64, Opcode::ShlEq),
    form_opcode(ATTR_NNN5 | ATTR_OS64, Opcode::ShrEq),
    form_opcode(ATTR_NNN6 | ATTR_OS64, Opcode::ShlEq),
    form_opcode(ATTR_NNN7 | ATTR_OS64, Opcode::SarEq),
    form_opcode(ATTR_NNN0 | ATTR_OS32, Opcode::RolEd),
    form_opcode(ATTR_NNN1 | ATTR_OS32, Opcode::RorEd),
    form_opcode(ATTR_NNN2 | ATTR_OS32, Opcode::RclEd),
    form_opcode(ATTR_NNN3 | ATTR_OS32, Opcode::RcrEd),
    form_opcode(ATTR_NNN4 | ATTR_OS32, Opcode::ShlEd),
    form_opcode(ATTR_NNN5 | ATTR_OS32, Opcode::ShrEd),
    form_opcode(ATTR_NNN6 | ATTR_OS32, Opcode::ShlEd),
    form_opcode(ATTR_NNN7 | ATTR_OS32, Opcode::SarEd),
    form_opcode(ATTR_NNN0 | ATTR_OS16, Opcode::RolEw),
    form_opcode(ATTR_NNN1 | ATTR_OS16, Opcode::RorEw),
    form_opcode(ATTR_NNN2 | ATTR_OS16, Opcode::RclEw),
    form_opcode(ATTR_NNN3 | ATTR_OS16, Opcode::RcrEw),
    form_opcode(ATTR_NNN4 | ATTR_OS16, Opcode::ShlEw),
    form_opcode(ATTR_NNN5 | ATTR_OS16, Opcode::ShrEw),
    form_opcode(ATTR_NNN6 | ATTR_OS16, Opcode::ShlEw),
    last_opcode(ATTR_NNN7 | ATTR_OS16, Opcode::SarEw),
];

// opcode D4
pub(super) const BxOpcodeTableD4: [u64; 1] = [last_opcode(ATTR_IS32, Opcode::Aam)];

// opcode D5
pub(super) const BxOpcodeTableD5: [u64; 1] = [last_opcode(ATTR_IS32, Opcode::Aad)];

// opcode D6
pub(super) const BxOpcodeTableD6: [u64; 1] = [last_opcode(0, Opcode::Salc)];

// opcode D7
pub(super) const BxOpcodeTableD7: [u64; 1] = [last_opcode(0, Opcode::Xlat)];

// opcode E0
pub(super) const BxOpcodeTableE0_32: [u64; 2] = [
    form_opcode(ATTR_IS32 | ATTR_OS32, Opcode::LoopneJbd),
    last_opcode(ATTR_IS32 | ATTR_OS16, Opcode::LoopneJbw),
];

pub(super) const BxOpcodeTableE0_64: [u64; 1] = [last_opcode(ATTR_IS64, Opcode::LoopneJbq)];

// opcode E1
pub(super) const BxOpcodeTableE1_32: [u64; 2] = [
    form_opcode(ATTR_IS32 | ATTR_OS32, Opcode::LoopeJbd),
    last_opcode(ATTR_IS32 | ATTR_OS16, Opcode::LoopeJbw),
];

pub(super) const BxOpcodeTableE1_64: [u64; 1] = [last_opcode(ATTR_IS64, Opcode::LoopeJbq)];

// opcode E2
pub(super) const BxOpcodeTableE2_32: [u64; 2] = [
    form_opcode(ATTR_IS32 | ATTR_OS32, Opcode::LoopJbd),
    last_opcode(ATTR_IS32 | ATTR_OS16, Opcode::LoopJbw),
];

pub(super) const BxOpcodeTableE2_64: [u64; 1] = [last_opcode(ATTR_IS64, Opcode::LoopJbq)];

// opcode E3
pub(super) const BxOpcodeTableE3_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JecxzJbd),
    last_opcode(ATTR_OS16, Opcode::JcxzJbw),
];

pub(super) const BxOpcodeTableE3_64: [u64; 1] = [last_opcode(ATTR_IS64, Opcode::JrcxzJbq)];

// opcode E4
pub(super) const BxOpcodeTableE4: [u64; 1] = [last_opcode(0, Opcode::InAlib)];

// opcode E5
pub(super) const BxOpcodeTableE5: [u64; 2] = [
    form_opcode(ATTR_OS32_64, Opcode::InEaxib),
    last_opcode(ATTR_OS16, Opcode::InAxib),
];

// opcode E6
pub(super) const BxOpcodeTableE6: [u64; 1] = [last_opcode(0, Opcode::OutIbAl)];

// opcode E7
pub(super) const BxOpcodeTableE7: [u64; 2] = [
    form_opcode(ATTR_OS32_64, Opcode::OutIbEax),
    last_opcode(ATTR_OS16, Opcode::OutIbAx),
];

// opcode E8
pub(super) const BxOpcodeTableE8_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::CallJd),
    last_opcode(ATTR_OS16, Opcode::CallJw),
];

pub(super) const BxOpcodeTableE8_64: [u64; 1] = [last_opcode(0, Opcode::CallJq)];

// opcode E9
pub(super) const BxOpcodeTableE9_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JmpJd),
    last_opcode(ATTR_OS16, Opcode::JmpJw),
];

pub(super) const BxOpcodeTableE9_64: [u64; 1] = [last_opcode(0, Opcode::JmpJq)];

// opcode EA
pub(super) const BxOpcodeTableEA_32: [u64; 1] = [last_opcode(0, Opcode::JmpfAp)];

// opcode EB
pub(super) const BxOpcodeTableEB_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JmpJbd),
    last_opcode(ATTR_OS16, Opcode::JmpJbw),
];

pub(super) const BxOpcodeTableEB_64: [u64; 1] = [last_opcode(0, Opcode::JmpJbq)];

// opcode EC
pub(super) const BxOpcodeTableEC: [u64; 1] = [last_opcode(0, Opcode::InAlDx)];

// opcode ED
pub(super) const BxOpcodeTableED: [u64; 2] = [
    form_opcode(ATTR_OS32_64, Opcode::InEaxDx),
    last_opcode(ATTR_OS16, Opcode::InAxDx),
];

// opcode EE
pub(super) const BxOpcodeTableEE: [u64; 1] = [last_opcode(0, Opcode::OutDxAl)];

// opcode EF
pub(super) const BxOpcodeTableEF: [u64; 2] = [
    form_opcode(ATTR_OS32_64, Opcode::OutDxEax),
    last_opcode(ATTR_OS16, Opcode::OutDxAx),
];

// opcode F1
pub(super) const BxOpcodeTableF1: [u64; 1] = [last_opcode(0, Opcode::INT1)];

// opcode F4
pub(super) const BxOpcodeTableF4: [u64; 1] = [last_opcode(0, Opcode::Hlt)];

// opcode F5
pub(super) const BxOpcodeTableF5: [u64; 1] = [last_opcode(0, Opcode::Cmc)];

// opcode F6
pub(super) const BxOpcodeTableF6: [u64; 8] = [
    form_opcode(ATTR_NNN0, Opcode::TestEbIb),
    form_opcode(ATTR_NNN1, Opcode::TestEbIb),
    form_opcode_lockable(ATTR_NNN2, Opcode::NotEb),
    form_opcode_lockable(ATTR_NNN3, Opcode::NegEb),
    form_opcode(ATTR_NNN4, Opcode::MulAleb),
    form_opcode(ATTR_NNN5, Opcode::ImulAleb),
    form_opcode(ATTR_NNN6, Opcode::DivAleb),
    last_opcode(ATTR_NNN7, Opcode::IdivAleb),
];

// opcode F7
pub(super) const BxOpcodeTableF7: [u64; 24] = [
    form_opcode(ATTR_NNN0 | ATTR_OS64, Opcode::TestEqId),
    form_opcode(ATTR_NNN1 | ATTR_OS64, Opcode::TestEqId),
    form_opcode_lockable(ATTR_NNN2 | ATTR_OS64, Opcode::NotEq),
    form_opcode_lockable(ATTR_NNN3 | ATTR_OS64, Opcode::NegEq),
    form_opcode(ATTR_NNN4 | ATTR_OS64, Opcode::MulRaxeq),
    form_opcode(ATTR_NNN5 | ATTR_OS64, Opcode::ImulRaxeq),
    form_opcode(ATTR_NNN6 | ATTR_OS64, Opcode::DivRaxeq),
    form_opcode(ATTR_NNN7 | ATTR_OS64, Opcode::IdivRaxeq),
    form_opcode(ATTR_NNN0 | ATTR_OS32, Opcode::TestEdId),
    form_opcode(ATTR_NNN1 | ATTR_OS32, Opcode::TestEdId),
    form_opcode_lockable(ATTR_NNN2 | ATTR_OS32, Opcode::NotEd),
    form_opcode_lockable(ATTR_NNN3 | ATTR_OS32, Opcode::NegEd),
    form_opcode(ATTR_NNN4 | ATTR_OS32, Opcode::MulEaxed),
    form_opcode(ATTR_NNN5 | ATTR_OS32, Opcode::ImulEaxed),
    form_opcode(ATTR_NNN6 | ATTR_OS32, Opcode::DivEaxed),
    form_opcode(ATTR_NNN7 | ATTR_OS32, Opcode::IdivEaxed),
    form_opcode(ATTR_NNN0 | ATTR_OS16, Opcode::TestEwIw),
    form_opcode(ATTR_NNN1 | ATTR_OS16, Opcode::TestEwIw),
    form_opcode_lockable(ATTR_NNN2 | ATTR_OS16, Opcode::NotEw),
    form_opcode_lockable(ATTR_NNN3 | ATTR_OS16, Opcode::NegEw),
    form_opcode(ATTR_NNN4 | ATTR_OS16, Opcode::MulAxew),
    form_opcode(ATTR_NNN5 | ATTR_OS16, Opcode::ImulAxew),
    form_opcode(ATTR_NNN6 | ATTR_OS16, Opcode::DivAxew),
    last_opcode(ATTR_NNN7 | ATTR_OS16, Opcode::IdivAxew),
];

// opcode F8
pub(super) const BxOpcodeTableF8: [u64; 1] = [last_opcode(0, Opcode::Clc)];

// opcode F9
pub(super) const BxOpcodeTableF9: [u64; 1] = [last_opcode(0, Opcode::Stc)];

// opcode FA
pub(super) const BxOpcodeTableFA: [u64; 1] = [last_opcode(0, Opcode::Cli)];

// opcode FB
pub(super) const BxOpcodeTableFB: [u64; 1] = [last_opcode(0, Opcode::Sti)];

// opcode FC
pub(super) const BxOpcodeTableFC: [u64; 1] = [last_opcode(0, Opcode::Cld)];

// opcode FD
pub(super) const BxOpcodeTableFD: [u64; 1] = [last_opcode(0, Opcode::Std)];

// opcode FE
pub(super) const BxOpcodeTableFE: [u64; 2] = [
    form_opcode_lockable(ATTR_NNN0, Opcode::IncEb),
    last_opcode_lockable(ATTR_NNN1, Opcode::DecEb),
];

// opcode FF
pub(super) const BxOpcodeTableFF: [u64; 21] = [
    //0
    form_opcode_lockable(ATTR_NNN0 | ATTR_OS64, Opcode::IncEq),
    form_opcode_lockable(ATTR_NNN0 | ATTR_OS32, Opcode::IncEd),
    form_opcode_lockable(ATTR_NNN0 | ATTR_OS16, Opcode::IncEw),
    //1
    form_opcode_lockable(ATTR_NNN1 | ATTR_OS64, Opcode::DecEq),
    form_opcode_lockable(ATTR_NNN1 | ATTR_OS32, Opcode::DecEd),
    form_opcode_lockable(ATTR_NNN1 | ATTR_OS16, Opcode::DecEw),
    //2
    form_opcode(ATTR_NNN2 | ATTR_IS64, Opcode::CallEq), // regardless of Osize
    form_opcode(ATTR_NNN2 | ATTR_IS32 | ATTR_OS16, Opcode::CallEw),
    form_opcode(ATTR_NNN2 | ATTR_IS32 | ATTR_OS32, Opcode::CallEd),
    //3
    form_opcode(ATTR_NNN3 | ATTR_OS64 | ATTR_MOD_MEM, Opcode::CallfOp64Ep),
    form_opcode(ATTR_NNN3 | ATTR_OS32 | ATTR_MOD_MEM, Opcode::CallfOp32Ep),
    form_opcode(ATTR_NNN3 | ATTR_OS16 | ATTR_MOD_MEM, Opcode::CallfOp16Ep),
    //4
    form_opcode(ATTR_NNN4 | ATTR_IS64, Opcode::JmpEq), // regardless of Osize
    form_opcode(ATTR_NNN4 | ATTR_IS32 | ATTR_OS16, Opcode::JmpEw),
    form_opcode(ATTR_NNN4 | ATTR_IS32 | ATTR_OS32, Opcode::JmpEd),
    //5
    form_opcode(ATTR_NNN5 | ATTR_OS64 | ATTR_MOD_MEM, Opcode::JmpfOp64Ep),
    form_opcode(ATTR_NNN5 | ATTR_OS32 | ATTR_MOD_MEM, Opcode::JmpfOp32Ep),
    form_opcode(ATTR_NNN5 | ATTR_OS16 | ATTR_MOD_MEM, Opcode::JmpfOp16Ep),
    //6
    form_opcode(ATTR_NNN6 | ATTR_IS64 | ATTR_OS32_64, Opcode::PushEq),
    form_opcode(ATTR_NNN6 | ATTR_OS32, Opcode::PushEd),
    last_opcode(ATTR_NNN6 | ATTR_OS16, Opcode::PushEw),
];

// opcode 0F 00
pub(super) const BxOpcodeTable0F00: [u64; 6] = [
    form_opcode(ATTR_NNN0, Opcode::SldtEw),
    form_opcode(ATTR_NNN1, Opcode::StrEw),
    form_opcode(ATTR_NNN2, Opcode::LldtEw),
    form_opcode(ATTR_NNN3, Opcode::LtrEw),
    form_opcode(ATTR_NNN4, Opcode::VerrEw),
    last_opcode(ATTR_NNN5, Opcode::VerwEw),
];

// opcode 0F 01
pub(super) const BxOpcodeTable0F01: [u64; 48] = [
    form_opcode(ATTR_IS32 | ATTR_MOD_MEM | ATTR_NNN0, Opcode::SgdtMs),
    form_opcode(ATTR_IS32 | ATTR_MOD_MEM | ATTR_NNN1, Opcode::SidtMs),
    form_opcode(ATTR_IS32 | ATTR_MOD_MEM | ATTR_NNN2, Opcode::LgdtMs),
    form_opcode(ATTR_IS32 | ATTR_MOD_MEM | ATTR_NNN3, Opcode::LidtMs),
    form_opcode(ATTR_IS64 | ATTR_MOD_MEM | ATTR_NNN0, Opcode::SgdtOp64Ms),
    form_opcode(ATTR_IS64 | ATTR_MOD_MEM | ATTR_NNN1, Opcode::SidtOp64Ms),
    form_opcode(ATTR_IS64 | ATTR_MOD_MEM | ATTR_NNN2, Opcode::LgdtOp64Ms),
    form_opcode(ATTR_IS64 | ATTR_MOD_MEM | ATTR_NNN3, Opcode::LidtOp64Ms),
    form_opcode(ATTR_NNN4, Opcode::SmswEw),
    form_opcode(ATTR_NNN6, Opcode::LmswEw),
    form_opcode(ATTR_NNN7 | ATTR_MOD_MEM, Opcode::Invlpg),
    form_opcode(
        ATTR_NNN0 | ATTR_RRR1 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Vmcall,
    ), // 0F 01 C1
    form_opcode(
        ATTR_NNN0 | ATTR_RRR2 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Vmlaunch,
    ), // 0F 01 C2
    form_opcode(
        ATTR_NNN0 | ATTR_RRR3 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Vmresume,
    ), // 0F 01 C3
    form_opcode(
        ATTR_NNN0 | ATTR_RRR4 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Vmxoff,
    ), // 0F 01 C4
    form_opcode(
        ATTR_NNN0 | ATTR_RRR6 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Wrmsrns,
    ), // 0F 01 C6
    form_opcode(
        ATTR_NNN0 | ATTR_RRR6 | ATTR_MODC0 | ATTR_SSE_PREFIX_F2 | ATTR_IS64,
        Opcode::Rdmsrlist,
    ),
    form_opcode(
        ATTR_NNN0 | ATTR_RRR6 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3 | ATTR_IS64,
        Opcode::Wrmsrlist,
    ),
    form_opcode(
        ATTR_NNN1 | ATTR_RRR0 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Monitor,
    ), // 0F 01 C8
    form_opcode(
        ATTR_NNN1 | ATTR_RRR1 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Mwait,
    ), // 0F 01 C9
    form_opcode(
        ATTR_NNN1 | ATTR_RRR2 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Clac,
    ), // 0F 01 CA
    form_opcode(
        ATTR_NNN1 | ATTR_RRR3 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Stac,
    ), // 0F 01 CB
    form_opcode(
        ATTR_NNN2 | ATTR_RRR0 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Xgetbv,
    ), // 0F 01 D0
    form_opcode(
        ATTR_NNN2 | ATTR_RRR1 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Xsetbv,
    ), // 0F 01 D1
    form_opcode(
        ATTR_NNN2 | ATTR_RRR4 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Vmfunc,
    ), // 0F 01 D4
    form_opcode(ATTR_NNN3 | ATTR_RRR0 | ATTR_MODC0, Opcode::Vmrun),
    form_opcode(ATTR_NNN3 | ATTR_RRR1 | ATTR_MODC0, Opcode::Vmmcall),
    form_opcode(ATTR_NNN3 | ATTR_RRR2 | ATTR_MODC0, Opcode::Vmload),
    form_opcode(ATTR_NNN3 | ATTR_RRR3 | ATTR_MODC0, Opcode::Vmsave),
    form_opcode(ATTR_NNN3 | ATTR_RRR4 | ATTR_MODC0, Opcode::Stgi),
    form_opcode(ATTR_NNN3 | ATTR_RRR5 | ATTR_MODC0, Opcode::Clgi),
    form_opcode(ATTR_NNN3 | ATTR_RRR6 | ATTR_MODC0, Opcode::Skinit),
    form_opcode(ATTR_NNN3 | ATTR_RRR7 | ATTR_MODC0, Opcode::Invlpga),
    form_opcode(
        ATTR_NNN5 | ATTR_RRR0 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Serialize,
    ), // 0F 01 E8
    form_opcode(
        ATTR_NNN5 | ATTR_RRR0 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3,
        Opcode::Setssbsy,
    ), // F3 0F 01 E8
    form_opcode(
        ATTR_NNN5 | ATTR_RRR2 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3,
        Opcode::Saveprevssp,
    ), // F3 0F 01 EA
    form_opcode(
        ATTR_NNN5 | ATTR_MOD_MEM | ATTR_SSE_PREFIX_F3,
        Opcode::Rstorssp,
    ),
    form_opcode(
        ATTR_NNN5 | ATTR_RRR6 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Rdpkru,
    ), // 0F 01 EE
    form_opcode(
        ATTR_NNN5 | ATTR_RRR7 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX,
        Opcode::Wrpkru,
    ), // 0F 01 EF
    form_opcode(
        ATTR_IS64 | ATTR_NNN5 | ATTR_RRR4 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3,
        Opcode::Uiret,
    ),
    form_opcode(
        ATTR_IS64 | ATTR_NNN5 | ATTR_RRR5 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3,
        Opcode::Testui,
    ),
    form_opcode(
        ATTR_IS64 | ATTR_NNN5 | ATTR_RRR6 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3,
        Opcode::Clui,
    ),
    form_opcode(
        ATTR_IS64 | ATTR_NNN5 | ATTR_RRR7 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3,
        Opcode::Stui,
    ),
    form_opcode(
        ATTR_NNN7 | ATTR_RRR0 | ATTR_MODC0 | ATTR_IS64,
        Opcode::Swapgs,
    ), // 0F 01 F8
    form_opcode(ATTR_NNN7 | ATTR_RRR1 | ATTR_MODC0, Opcode::Rdtscp), // 0F 01 F9
    form_opcode(ATTR_NNN7 | ATTR_RRR2 | ATTR_MODC0, Opcode::Monitorx), // 0F 01 FA
    form_opcode(ATTR_NNN7 | ATTR_RRR3 | ATTR_MODC0, Opcode::Mwaitx), // 0F 01 FB
    last_opcode(ATTR_NNN7 | ATTR_RRR4 | ATTR_MODC0, Opcode::Clzero), // 0F 01 FC
];

// opcode 0F 02
pub(super) const BxOpcodeTable0F02: [u64; 2] = [
    form_opcode(ATTR_OS32_64, Opcode::LarGdEw),
    last_opcode(ATTR_OS16, Opcode::LarGwEw),
];

// opcode 0F 03
pub(super) const BxOpcodeTable0F03: [u64; 2] = [
    form_opcode(ATTR_OS32_64, Opcode::LslGdEw),
    last_opcode(ATTR_OS16, Opcode::LslGwEw),
];

// opcode 0F 05
pub(super) const BxOpcodeTable0F05_32: [u64; 1] = [last_opcode(0, Opcode::SyscallLegacy)];
pub(super) const BxOpcodeTable0F05_64: [u64; 1] = [last_opcode(0, Opcode::Syscall)];

// opcode 0F 06
pub(super) const BxOpcodeTable0F06: [u64; 1] = [last_opcode(0, Opcode::Clts)];

// opcode 0F 07
pub(super) const BxOpcodeTable0F07_32: [u64; 1] = [last_opcode(0, Opcode::SysretLegacy)];
pub(super) const BxOpcodeTable0F07_64: [u64; 1] = [last_opcode(0, Opcode::Sysret)];

// opcode 0F 08
pub(super) const BxOpcodeTable0F08: [u64; 1] = [last_opcode(0, Opcode::Invd)];

// opcode 0F 09
pub(super) const BxOpcodeTable0F09: [u64; 1] = [last_opcode(0, Opcode::Wbinvd)];

// opcode 0F 0B
pub(super) const BxOpcodeTable0F0B: [u64; 1] = [last_opcode(0, Opcode::Ud2)];

// opcode 0F 0D - 3DNow! PREFETCHW on AMD, NOP on older Intel CPUs
pub(super) const BxOpcodeTable0F0D: [u64; 1] = [last_opcode(0, Opcode::PrefetchwMb)];

// opcode 0F 0E - 3DNow! FEMMS
pub(super) const BxOpcodeTable0F0E: [u64; 1] = [last_opcode(0, Opcode::Femms)];

// opcode 0F 0F - 3DNow! Opcode Table

// opcode 0F 10
pub(super) const BxOpcodeTable0F10: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::MovupsVpsWps),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::MovupdVpdWpd),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::MovssVssWss),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::MovsdVsdWsd),
];

// opcode 0F 11
pub(super) const BxOpcodeTable0F11: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::MovupsWpsVps),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::MovupdWpdVpd),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::MovssWssVss),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::MovsdWsdVsd),
];

// opcode 0F 12
pub(super) const BxOpcodeTable0F12: [u64; 5] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_MOD_MEM, Opcode::MovlpsVpsMq),
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_MODC0, Opcode::MovhlpsVpsWps),
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM, Opcode::MovlpdVsdMq),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::MovsldupVpsWps),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::MovddupVpdWq),
];

// opcode 0F 13
pub(super) const BxOpcodeTable0F13: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_MOD_MEM, Opcode::MovlpsMqVps),
    last_opcode(ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM, Opcode::MovlpdMqVsd),
];

// opcode 0F 14
pub(super) const BxOpcodeTable0F14: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::UnpcklpsVpsWdq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::UnpcklpdVpdWdq),
];

// opcode 0F 15
pub(super) const BxOpcodeTable0F15: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::UnpckhpsVpsWdq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::UnpckhpdVpdWdq),
];

// opcode 0F 16
pub(super) const BxOpcodeTable0F16: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_MOD_MEM, Opcode::MovhpsVpsMq),
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_MODC0, Opcode::MovlhpsVpsWps),
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM, Opcode::MovhpdVsdMq),
    last_opcode(ATTR_SSE_PREFIX_F3, Opcode::MovshdupVpsWps),
];

// opcode 0F 17
pub(super) const BxOpcodeTable0F17: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_MOD_MEM, Opcode::MovhpsMqVps),
    last_opcode(ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM, Opcode::MovhpdMqVsd),
];

// opcode 0F 18 - opcode group G16, PREFETCH hints
pub(super) const BxOpcodeTable0F18: [u64; 5] = [
    form_opcode(ATTR_NNN0, Opcode::PrefetchntaMb),
    form_opcode(ATTR_NNN1, Opcode::Prefetcht0Mb),
    form_opcode(ATTR_NNN2, Opcode::Prefetcht1Mb),
    form_opcode(ATTR_NNN3, Opcode::Prefetcht2Mb),
    last_opcode(0, Opcode::PrefetchMb),
];

// opcode 0F 1E
pub(super) const BxOpcodeTable0F1E: [u64; 5] = [
    form_opcode(
        ATTR_OS16_32 | ATTR_NNN1 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3,
        Opcode::Rdsspd,
    ),
    form_opcode(
        ATTR_OS64 | ATTR_NNN1 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3,
        Opcode::Rdsspq,
    ),
    form_opcode(
        ATTR_NNN7 | ATTR_RRR2 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3,
        Opcode::Endbranch64,
    ),
    form_opcode(
        ATTR_NNN7 | ATTR_RRR3 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3,
        Opcode::Endbranch32,
    ),
    last_opcode(0, Opcode::Nop), // multi byte-NOP
];

// opcode 0F 19 - 0F 1F: multi-byte NOP
pub(super) const BxOpcodeTableMultiByteNOP: [u64; 1] = [last_opcode(0, Opcode::Nop)];

// opcode 0F 20
pub(super) const BxOpcodeTable0F20_32: [u64; 4] = [
    form_opcode(ATTR_NNN0, Opcode::MovRdCr0),
    form_opcode(ATTR_NNN2, Opcode::MovRdCr2),
    form_opcode(ATTR_NNN3, Opcode::MovRdCr3),
    last_opcode(ATTR_NNN4, Opcode::MovRdCr4),
];

pub(super) const BxOpcodeTable0F20_64: [u64; 4] = [
    form_opcode(ATTR_NNN0, Opcode::MovRqCr0),
    form_opcode(ATTR_NNN2, Opcode::MovRqCr2),
    form_opcode(ATTR_NNN3, Opcode::MovRqCr3),
    last_opcode(ATTR_NNN4, Opcode::MovRqCr4),
];

// opcode 0F 21
pub(super) const BxOpcodeTable0F21_32: [u64; 1] = [last_opcode(0, Opcode::MovRdDd)];
pub(super) const BxOpcodeTable0F21_64: [u64; 1] = [last_opcode(0, Opcode::MovRqDq)];

// opcode 0F 22
pub(super) const BxOpcodeTable0F22_32: [u64; 4] = [
    form_opcode(ATTR_NNN0, Opcode::MovCr0rd),
    form_opcode(ATTR_NNN2, Opcode::MovCr2rd),
    form_opcode(ATTR_NNN3, Opcode::MovCr3rd),
    last_opcode(ATTR_NNN4, Opcode::MovCr4rd),
];

pub(super) const BxOpcodeTable0F22_64: [u64; 4] = [
    form_opcode(ATTR_NNN0, Opcode::MovCr0rq),
    form_opcode(ATTR_NNN2, Opcode::MovCr2rq),
    form_opcode(ATTR_NNN3, Opcode::MovCr3rq),
    last_opcode(ATTR_NNN4, Opcode::MovCr4rq),
];

// opcode 0F 23
pub(super) const BxOpcodeTable0F23_32: [u64; 1] = [last_opcode(0, Opcode::MovDdRd)];
pub(super) const BxOpcodeTable0F23_64: [u64; 1] = [last_opcode(0, Opcode::MovDqRq)];

// opcode 0F 24
pub(super) const BxOpcodeTable0F24: [u64; 1] = [last_opcode(0, Opcode::IaError)]; // BX_IA_MOV_RdTd not implemented
                                                                                  // opcode 0F 26
pub(super) const BxOpcodeTable0F26: [u64; 1] = [last_opcode(0, Opcode::IaError)]; // BX_IA_MOV_TdRd not implemented

// opcode 0F 28
pub(super) const BxOpcodeTable0F28: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::MovapsVpsWps),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::MovapdVpdWpd),
];

// opcode 0F 29
pub(super) const BxOpcodeTable0F29: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::MovapsWpsVps),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::MovapdWpdVpd),
];

// opcode 0F 2A
pub(super) const BxOpcodeTable0F2A: [u64; 6] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::Cvtpi2psVpsQq),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::Cvtpi2pdVpdQq),
    form_opcode(ATTR_SSE_PREFIX_F3 | ATTR_OS64, Opcode::Cvtsi2ssVssEq),
    form_opcode(ATTR_SSE_PREFIX_F2 | ATTR_OS64, Opcode::Cvtsi2sdVsdEq),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::Cvtsi2ssVssEd),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::Cvtsi2sdVsdEd),
];

// opcode 0F 2B
pub(super) const BxOpcodeTable0F2B: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_MOD_MEM, Opcode::MovntpsMpsVps),
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM, Opcode::MovntpdMpdVpd),
    form_opcode(ATTR_SSE_PREFIX_F3 | ATTR_MOD_MEM, Opcode::MovntssMssVss),
    last_opcode(ATTR_SSE_PREFIX_F2 | ATTR_MOD_MEM, Opcode::MovntsdMsdVsd),
];

// opcode 0F 2C
pub(super) const BxOpcodeTable0F2C: [u64; 6] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::Cvttps2piPqWps),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::Cvttpd2piPqWpd),
    form_opcode(ATTR_SSE_PREFIX_F3 | ATTR_OS64, Opcode::Cvttss2siGqWss),
    form_opcode(ATTR_SSE_PREFIX_F2 | ATTR_OS64, Opcode::Cvttsd2siGqWsd),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::Cvttss2siGdWss),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::Cvttsd2siGdWsd),
];

// opcode 0F 2D
pub(super) const BxOpcodeTable0F2D: [u64; 6] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::Cvtps2piPqWps),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::Cvtpd2piPqWpd),
    form_opcode(ATTR_SSE_PREFIX_F3 | ATTR_OS64, Opcode::Cvtss2siGqWss),
    form_opcode(ATTR_SSE_PREFIX_F2 | ATTR_OS64, Opcode::Cvtsd2siGqWsd),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::Cvtss2siGdWss),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::Cvtsd2siGdWsd),
];

// opcode 0F 2E
pub(super) const BxOpcodeTable0F2E: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::UcomissVssWss),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::UcomisdVsdWsd),
];

// opcode 0F 2F
pub(super) const BxOpcodeTable0F2F: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::ComissVssWss),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::ComisdVsdWsd),
];

// opcode 0F 30
pub(super) const BxOpcodeTable0F30: [u64; 1] = [last_opcode(0, Opcode::Wrmsr)];

// opcode 0F 31 - end trace to avoid multiple TSC samples in one cycle
pub(super) const BxOpcodeTable0F31: [u64; 1] = [last_opcode(0, Opcode::Rdtsc)];

// opcode 0F 32 - end trace to avoid multiple TSC samples in one cycle
pub(super) const BxOpcodeTable0F32: [u64; 1] = [last_opcode(0, Opcode::Rdmsr)];

// opcode 0F 33
pub(super) const BxOpcodeTable0F33: [u64; 1] = [last_opcode(0, Opcode::Rdpmc)];

// opcode 0F 34
pub(super) const BxOpcodeTable0F34: [u64; 1] = [last_opcode(0, Opcode::Sysenter)];

// opcode 0F 35
pub(super) const BxOpcodeTable0F35: [u64; 1] = [last_opcode(0, Opcode::Sysexit)];

// opcode 0F 37
pub(super) const BxOpcodeTable0F37: [u64; 1] = [last_opcode(ATTR_SSE_NO_PREFIX, Opcode::Getsec)];

// opcode 0F 40
pub(super) const BxOpcodeTable0F40: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::CmovoGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovoGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovoGwEw),
];

// opcode 0F 41 — KAND (VEX.L1) + CMOVno (non-VEX)
pub(super) const BxOpcodeTable0F41: [u64; 7] = [
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KandwKgwKhwKew),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W1 | ATTR_SSE_NO_PREFIX, Opcode::KandqKgqKhqKeq),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KandbKgbKhbKeb),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KanddKgdKhdKed),
    form_opcode(ATTR_OS64, Opcode::CmovnoGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovnoGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovnoGwEw),
];

// opcode 0F 42 — KANDN (VEX.L1) + CMOVb (non-VEX)
pub(super) const BxOpcodeTable0F42: [u64; 7] = [
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KandnwKgwKhwKew),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W1 | ATTR_SSE_NO_PREFIX, Opcode::KandnqKgqKhqKeq),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KandnbKgbKhbKeb),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KandndKgdKhdKed),
    form_opcode(ATTR_OS64, Opcode::CmovbGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovbGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovbGwEw),
];

// opcode 0F 43
pub(super) const BxOpcodeTable0F43: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::CmovnbGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovnbGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovnbGwEw),
];

// opcode 0F 44 — KNOT (VEX.L0, 2-operand) + CMOVz (non-VEX)
pub(super) const BxOpcodeTable0F44: [u64; 7] = [
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KnotwKgwKew),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_NO_PREFIX, Opcode::KnotqKgqKeq),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KnotbKgbKeb),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KnotdKgdKed),
    form_opcode(ATTR_OS64, Opcode::CmovzGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovzGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovzGwEw),
];

// opcode 0F 45 — KOR (VEX.L1) + CMOVnz (non-VEX)
pub(super) const BxOpcodeTable0F45: [u64; 7] = [
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KorwKgwKhwKew),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W1 | ATTR_SSE_NO_PREFIX, Opcode::KorqKgqKhqKeq),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KorbKgbKhbKeb),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KordKgdKhdKed),
    form_opcode(ATTR_OS64, Opcode::CmovnzGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovnzGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovnzGwEw),
];

// opcode 0F 46 — KXNOR (VEX.L1) + CMOVbe (non-VEX)
pub(super) const BxOpcodeTable0F46: [u64; 7] = [
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KxnorwKgwKhwKew),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W1 | ATTR_SSE_NO_PREFIX, Opcode::KxnorqKgqKhqKeq),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KxnorbKgbKhbKeb),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KxnordKgdKhdKed),
    form_opcode(ATTR_OS64, Opcode::CmovbeGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovbeGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovbeGwEw),
];

// opcode 0F 47 — KXOR (VEX.L1) + CMOVnbe (non-VEX)
pub(super) const BxOpcodeTable0F47: [u64; 7] = [
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KxorwKgwKhwKew),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W1 | ATTR_SSE_NO_PREFIX, Opcode::KxorqKgqKhqKeq),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KxorbKgbKhbKeb),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KxordKgdKhdKed),
    form_opcode(ATTR_OS64, Opcode::CmovnbeGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovnbeGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovnbeGwEw),
];

// opcode 0F 48
pub(super) const BxOpcodeTable0F48: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::CmovsGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovsGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovsGwEw),
];

// opcode 0F 49
pub(super) const BxOpcodeTable0F49: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::CmovnsGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovnsGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovnsGwEw),
];

// opcode 0F 4A — KADD (VEX.L1) + CMOVp (non-VEX)
pub(super) const BxOpcodeTable0F4A: [u64; 7] = [
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KaddwKgwKhwKew),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W1 | ATTR_SSE_NO_PREFIX, Opcode::KaddqKgqKhqKeq),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KaddbKgbKhbKeb),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KadddKgdKhdKed),
    form_opcode(ATTR_OS64, Opcode::CmovpGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovpGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovpGwEw),
];

// opcode 0F 4B — KUNPCK (VEX.L1) + CMOVnp (non-VEX)
pub(super) const BxOpcodeTable0F4B: [u64; 6] = [
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KunpckbwKgwKhbKeb),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W1 | ATTR_SSE_NO_PREFIX, Opcode::KunpckdqKgqKhdKed),
    form_opcode(ATTR_VEX | ATTR_VL256 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KunpckwdKgdKhwKew),
    form_opcode(ATTR_OS64, Opcode::CmovnpGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovnpGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovnpGwEw),
];

// opcode 0F 4C
pub(super) const BxOpcodeTable0F4C: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::CmovlGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovlGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovlGwEw),
];

// opcode 0F 4D
pub(super) const BxOpcodeTable0F4D: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::CmovnlGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovnlGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovnlGwEw),
];

// opcode 0F 4E
pub(super) const BxOpcodeTable0F4E: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::CmovleGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovleGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovleGwEw),
];

// opcode 0F 4F
pub(super) const BxOpcodeTable0F4F: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::CmovnleGqEq),
    form_opcode(ATTR_OS32, Opcode::CmovnleGdEd),
    last_opcode(ATTR_OS16, Opcode::CmovnleGwEw),
];

// opcode 0F 50
pub(super) const BxOpcodeTable0F50: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_MODC0, Opcode::MovmskpsGdUps),
    last_opcode(ATTR_SSE_PREFIX_66 | ATTR_MODC0, Opcode::MovmskpdGdUpd),
];

// opcode 0F 51
pub(super) const BxOpcodeTable0F51: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::SqrtpsVpsWps),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::SqrtpdVpdWpd),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::SqrtssVssWss),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::SqrtsdVsdWsd),
];

// opcode 0F 52
pub(super) const BxOpcodeTable0F52: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::RsqrtpsVpsWps),
    last_opcode(ATTR_SSE_PREFIX_F3, Opcode::RsqrtssVssWss),
];

// opcode 0F 53
pub(super) const BxOpcodeTable0F53: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::RcppsVpsWps),
    last_opcode(ATTR_SSE_PREFIX_F3, Opcode::RcpssVssWss),
];

// opcode 0F 54
pub(super) const BxOpcodeTable0F54: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::AndpsVpsWps),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::AndpdVpdWpd),
];

// opcode 0F 55
pub(super) const BxOpcodeTable0F55: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::AndnpsVpsWps),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::AndnpdVpdWpd),
];

// opcode 0F 56
pub(super) const BxOpcodeTable0F56: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::OrpsVpsWps),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::OrpdVpdWpd),
];

// opcode 0F 57
pub(super) const BxOpcodeTable0F57: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::XorpsVpsWps),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::XorpdVpdWpd),
];

// opcode 0F 58
pub(super) const BxOpcodeTable0F58: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::AddpsVpsWps),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::AddpdVpdWpd),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::AddssVssWss),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::AddsdVsdWsd),
];

// opcode 0F 59
pub(super) const BxOpcodeTable0F59: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::MulpsVpsWps),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::MulpdVpdWpd),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::MulssVssWss),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::MulsdVsdWsd),
];

// opcode 0F 5A
pub(super) const BxOpcodeTable0F5A: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::Cvtps2pdVpdWps),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::Cvtpd2psVpsWpd),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::Cvtss2sdVsdWss),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::Cvtsd2ssVssWsd),
];

// opcode 0F 5B
pub(super) const BxOpcodeTable0F5B: [u64; 3] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::Cvtdq2psVpsWdq),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::Cvtps2dqVdqWps),
    last_opcode(ATTR_SSE_PREFIX_F3, Opcode::Cvttps2dqVdqWps),
];

// opcode 0F 5C
pub(super) const BxOpcodeTable0F5C: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::SubpsVpsWps),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::SubpdVpdWpd),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::SubssVssWss),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::SubsdVsdWsd),
];

// opcode 0F 5D
pub(super) const BxOpcodeTable0F5D: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::MinpsVpsWps),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::MinpdVpdWpd),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::MinssVssWss),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::MinsdVsdWsd),
];

// opcode 0F 5E
pub(super) const BxOpcodeTable0F5E: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::DivpsVpsWps),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::DivpdVpdWpd),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::DivssVssWss),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::DivsdVsdWsd),
];

// opcode 0F 5F
pub(super) const BxOpcodeTable0F5F: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::MaxpsVpsWps),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::MaxpdVpdWpd),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::MaxssVssWss),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::MaxsdVsdWsd),
];

// opcode 0F 60
pub(super) const BxOpcodeTable0F60: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PunpcklbwPqQd),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PunpcklbwVdqWdq),
];

// opcode 0F 61
pub(super) const BxOpcodeTable0F61: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PunpcklwdPqQd),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PunpcklwdVdqWdq),
];

// opcode 0F 62
pub(super) const BxOpcodeTable0F62: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PunpckldqPqQd),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PunpckldqVdqWdq),
];

// opcode 0F 63
pub(super) const BxOpcodeTable0F63: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PacksswbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PacksswbVdqWdq),
];

// opcode 0F 64
pub(super) const BxOpcodeTable0F64: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PcmpgtbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpgtbVdqWdq),
];

// opcode 0F 65
pub(super) const BxOpcodeTable0F65: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PcmpgtwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpgtwVdqWdq),
];

// opcode 0F 66
pub(super) const BxOpcodeTable0F66: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PcmpgtdPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpgtdVdqWdq),
];

// opcode 0F 67
pub(super) const BxOpcodeTable0F67: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PackuswbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PackuswbVdqWdq),
];

// opcode 0F 68
pub(super) const BxOpcodeTable0F68: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PunpckhbwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PunpckhbwVdqWdq),
];

// opcode 0F 69
pub(super) const BxOpcodeTable0F69: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PunpckhwdPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PunpckhwdVdqWdq),
];

// opcode 0F 6A
pub(super) const BxOpcodeTable0F6A: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PunpckhdqPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PunpckhdqVdqWdq),
];

// opcode 0F 6B
pub(super) const BxOpcodeTable0F6B: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PackssdwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PackssdwVdqWdq),
];

// opcode 0F 6C - 0F 6D
pub(super) const BxOpcodeTable0F6C: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PunpcklqdqVdqWdq)];
pub(super) const BxOpcodeTable0F6D: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PunpckhqdqVdqWdq)];

// opcode 0F 6E
pub(super) const BxOpcodeTable0F6E: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_OS64, Opcode::MovqPqEq),
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_OS64, Opcode::MovqVdqEq),
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::MovdPqEd),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::MovdVdqEd),
];

// opcode 0F 6F
pub(super) const BxOpcodeTable0F6F: [u64; 3] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::MovqPqQq),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::MovdqaVdqWdq),
    last_opcode(ATTR_SSE_PREFIX_F3, Opcode::MovdquVdqWdq),
];

// opcode 0F 70
pub(super) const BxOpcodeTable0F70: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PshufwPqQqIb),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::PshufdVdqWdqIb),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::PshufhwVdqWdqIb),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::PshuflwVdqWdqIb),
];

// opcode 0F 71
pub(super) const BxOpcodeTable0F71: [u64; 6] = [
    form_opcode(
        ATTR_NNN2 | ATTR_SSE_NO_PREFIX | ATTR_MODC0,
        Opcode::PsrlwNqIb,
    ),
    form_opcode(
        ATTR_NNN2 | ATTR_SSE_PREFIX_66 | ATTR_MODC0,
        Opcode::PsrlwUdqIb,
    ),
    form_opcode(
        ATTR_NNN4 | ATTR_SSE_NO_PREFIX | ATTR_MODC0,
        Opcode::PsrawNqIb,
    ),
    form_opcode(
        ATTR_NNN4 | ATTR_SSE_PREFIX_66 | ATTR_MODC0,
        Opcode::PsrawUdqIb,
    ),
    form_opcode(
        ATTR_NNN6 | ATTR_SSE_NO_PREFIX | ATTR_MODC0,
        Opcode::PsllwNqIb,
    ),
    last_opcode(
        ATTR_NNN6 | ATTR_SSE_PREFIX_66 | ATTR_MODC0,
        Opcode::PsllwUdqIb,
    ),
];

// opcode 0F 72 — Group 13
// nnn=0: VPRORD (EVEX), nnn=1: VPROLD (EVEX),
// nnn=2: PSRLD/VPSRLD, nnn=4: PSRAD/VPSRAD, nnn=6: PSLLD/VPSLLD
// Note: Bochs legacy BxOpcodeTable0F72 has 6 entries (no VPRORD/VPROLD), but
// Bochs has a separate BxOpcodeGroup_EVEX_0F72 table. Our decoder has NO separate
// EVEX opcode map — EVEX instructions share the same opmap tables with ATTR flags.
// These EVEX entries MUST stay here. Flags match Bochs fetchdecode_opmap_evex.cc.
pub(super) const BxOpcodeTable0F72: [u64; 8] = [
    form_opcode(
        ATTR_NNN0 | ATTR_SSE_PREFIX_66 | ATTR_VEX_W0 | ATTR_MODC0,
        Opcode::EvexVprordUdqIb,
    ),
    form_opcode(
        ATTR_NNN1 | ATTR_SSE_PREFIX_66 | ATTR_VEX_W0 | ATTR_MODC0,
        Opcode::EvexVproldUdqIb,
    ),
    form_opcode(
        ATTR_NNN2 | ATTR_SSE_NO_PREFIX | ATTR_MODC0,
        Opcode::PsrldNqIb,
    ),
    form_opcode(
        ATTR_NNN2 | ATTR_SSE_PREFIX_66 | ATTR_MODC0,
        Opcode::PsrldUdqIb,
    ),
    form_opcode(
        ATTR_NNN4 | ATTR_SSE_NO_PREFIX | ATTR_MODC0,
        Opcode::PsradNqIb,
    ),
    form_opcode(
        ATTR_NNN4 | ATTR_SSE_PREFIX_66 | ATTR_MODC0,
        Opcode::PsradUdqIb,
    ),
    form_opcode(
        ATTR_NNN6 | ATTR_SSE_NO_PREFIX | ATTR_MODC0,
        Opcode::PslldNqIb,
    ),
    last_opcode(
        ATTR_NNN6 | ATTR_SSE_PREFIX_66 | ATTR_MODC0,
        Opcode::PslldUdqIb,
    ),
];

// opcode 0F 73
pub(super) const BxOpcodeTable0F73: [u64; 6] = [
    form_opcode(
        ATTR_NNN2 | ATTR_SSE_NO_PREFIX | ATTR_MODC0,
        Opcode::PsrlqNqIb,
    ),
    form_opcode(
        ATTR_NNN2 | ATTR_SSE_PREFIX_66 | ATTR_MODC0,
        Opcode::PsrlqUdqIb,
    ),
    form_opcode(
        ATTR_NNN3 | ATTR_SSE_PREFIX_66 | ATTR_MODC0,
        Opcode::PsrldqUdqIb,
    ),
    form_opcode(
        ATTR_NNN6 | ATTR_SSE_NO_PREFIX | ATTR_MODC0,
        Opcode::PsllqNqIb,
    ),
    form_opcode(
        ATTR_NNN6 | ATTR_SSE_PREFIX_66 | ATTR_MODC0,
        Opcode::PsllqUdqIb,
    ),
    last_opcode(
        ATTR_NNN7 | ATTR_SSE_PREFIX_66 | ATTR_MODC0,
        Opcode::PslldqUdqIb,
    ),
];

// opcode 0F 74
pub(super) const BxOpcodeTable0F74: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PcmpeqbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpeqbVdqWdq),
];

// opcode 0F 75
pub(super) const BxOpcodeTable0F75: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PcmpeqwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpeqwVdqWdq),
];

// opcode 0F 76
pub(super) const BxOpcodeTable0F76: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PcmpeqdPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpeqdVdqWdq),
];

// opcode 0F 77
pub(super) const BxOpcodeTable0F77: [u64; 1] = [last_opcode(ATTR_SSE_NO_PREFIX, Opcode::Emms)];

// opcode 0F 78
pub(super) const BxOpcodeTable0F78: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_IS32, Opcode::VmreadEdGd),
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_IS64, Opcode::VmreadEqGq),
    // SSE4A by AMD
    form_opcode(
        ATTR_SSE_PREFIX_66 | ATTR_MODC0 | ATTR_NNN0,
        Opcode::ExtrqUdqIbIb,
    ),
    last_opcode(ATTR_SSE_PREFIX_F2 | ATTR_MODC0, Opcode::InsertqVdqUqIbIb),
];

// opcode 0F 79
pub(super) const BxOpcodeTable0F79: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_IS32, Opcode::VmwriteGdEd),
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_IS64, Opcode::VmwriteGqEq),
    // SSE4A by AMD
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_MODC0, Opcode::ExtrqVdqUq),
    last_opcode(ATTR_SSE_PREFIX_F2 | ATTR_MODC0, Opcode::InsertqVdqUdq),
];

// opcode 0F 7C
pub(super) const BxOpcodeTable0F7C: [u64; 2] = [
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::HaddpdVpdWpd),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::HaddpsVpsWps),
];

// opcode 0F 7D
pub(super) const BxOpcodeTable0F7D: [u64; 2] = [
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::HsubpdVpdWpd),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::HsubpsVpsWps),
];

// opcode 0F 7E
pub(super) const BxOpcodeTable0F7E: [u64; 5] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_OS64, Opcode::MovqEqPq),
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_OS64, Opcode::MovqEqVq),
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::MovdEdPq),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::MovdEdVd),
    last_opcode(ATTR_SSE_PREFIX_F3, Opcode::MovqVqWq),
];

// opcode 0F 7F
pub(super) const BxOpcodeTable0F7F: [u64; 3] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::MovqQqPq),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::MovdqaWdqVdq),
    last_opcode(ATTR_SSE_PREFIX_F3, Opcode::MovdquWdqVdq),
];

// opcode 0F 80
pub(super) const BxOpcodeTable0F80_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JoJd),
    last_opcode(ATTR_OS16, Opcode::JoJw),
];

pub(super) const BxOpcodeTable0F80_64: [u64; 1] = [last_opcode(0, Opcode::JoJq)];

// opcode 0F 81
pub(super) const BxOpcodeTable0F81_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnoJd),
    last_opcode(ATTR_OS16, Opcode::JnoJw),
];

pub(super) const BxOpcodeTable0F81_64: [u64; 1] = [last_opcode(0, Opcode::JnoJq)];

// opcode 0F 82
pub(super) const BxOpcodeTable0F82_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JbJd),
    last_opcode(ATTR_OS16, Opcode::JbJw),
];

pub(super) const BxOpcodeTable0F82_64: [u64; 1] = [last_opcode(0, Opcode::JbJq)];

// opcode 0F 83
pub(super) const BxOpcodeTable0F83_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnbJd),
    last_opcode(ATTR_OS16, Opcode::JnbJw),
];

pub(super) const BxOpcodeTable0F83_64: [u64; 1] = [last_opcode(0, Opcode::JnbJq)];

// opcode 0F 84
pub(super) const BxOpcodeTable0F84_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JzJd),
    last_opcode(ATTR_OS16, Opcode::JzJw),
];

pub(super) const BxOpcodeTable0F84_64: [u64; 1] = [last_opcode(0, Opcode::JzJq)];

// opcode 0F 85
pub(super) const BxOpcodeTable0F85_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnzJd),
    last_opcode(ATTR_OS16, Opcode::JnzJw),
];

pub(super) const BxOpcodeTable0F85_64: [u64; 1] = [last_opcode(0, Opcode::JnzJq)];

// opcode 0F 86
pub(super) const BxOpcodeTable0F86_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JbeJd),
    last_opcode(ATTR_OS16, Opcode::JbeJw),
];

pub(super) const BxOpcodeTable0F86_64: [u64; 1] = [last_opcode(0, Opcode::JbeJq)];

// opcode 0F 87
pub(super) const BxOpcodeTable0F87_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnbeJd),
    last_opcode(ATTR_OS16, Opcode::JnbeJw),
];

pub(super) const BxOpcodeTable0F87_64: [u64; 1] = [last_opcode(0, Opcode::JnbeJq)];

// opcode 0F 88
pub(super) const BxOpcodeTable0F88_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JsJd),
    last_opcode(ATTR_OS16, Opcode::JsJw),
];

pub(super) const BxOpcodeTable0F88_64: [u64; 1] = [last_opcode(0, Opcode::JsJq)];

// opcode 0F 89
pub(super) const BxOpcodeTable0F89_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnsJd),
    last_opcode(ATTR_OS16, Opcode::JnsJw),
];

pub(super) const BxOpcodeTable0F89_64: [u64; 1] = [last_opcode(0, Opcode::JnsJq)];

// opcode 0F 8A
pub(super) const BxOpcodeTable0F8A_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JpJd),
    last_opcode(ATTR_OS16, Opcode::JpJw),
];

pub(super) const BxOpcodeTable0F8A_64: [u64; 1] = [last_opcode(0, Opcode::JpJq)];

// opcode 0F 8B
pub(super) const BxOpcodeTable0F8B_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnpJd),
    last_opcode(ATTR_OS16, Opcode::JnpJw),
];

pub(super) const BxOpcodeTable0F8B_64: [u64; 1] = [last_opcode(0, Opcode::JnpJq)];

// opcode 0F 8C
pub(super) const BxOpcodeTable0F8C_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JlJd),
    last_opcode(ATTR_OS16, Opcode::JlJw),
];

pub(super) const BxOpcodeTable0F8C_64: [u64; 1] = [last_opcode(0, Opcode::JlJq)];

// opcode 0F 8D
pub(super) const BxOpcodeTable0F8D_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnlJd),
    last_opcode(ATTR_OS16, Opcode::JnlJw),
];

pub(super) const BxOpcodeTable0F8D_64: [u64; 1] = [last_opcode(0, Opcode::JnlJq)];

// opcode 0F 8E
pub(super) const BxOpcodeTable0F8E_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JleJd),
    last_opcode(ATTR_OS16, Opcode::JleJw),
];

pub(super) const BxOpcodeTable0F8E_64: [u64; 1] = [last_opcode(0, Opcode::JleJq)];

// opcode 0F 8F
pub(super) const BxOpcodeTable0F8F_32: [u64; 2] = [
    form_opcode(ATTR_OS32, Opcode::JnleJd),
    last_opcode(ATTR_OS16, Opcode::JnleJw),
];

pub(super) const BxOpcodeTable0F8F_64: [u64; 1] = [last_opcode(0, Opcode::JnleJq)];

// opcode 0F 90 - 0F 9F
// opcode 0F 90 — KMOV load (VEX.L0) + SETcc (non-VEX)
pub(super) const BxOpcodeTable0F90: [u64; 5] = [
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KmovwKgwKew),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_NO_PREFIX, Opcode::KmovqKgqKeq),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KmovbKgbKeb),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KmovdKgdKed),
    last_opcode(0, Opcode::SetoEb),
];
// opcode 0F 91 — KMOV store (VEX.L0) + SETcc (non-VEX)
pub(super) const BxOpcodeTable0F91: [u64; 5] = [
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KmovwKewKgw),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_NO_PREFIX, Opcode::KmovqKeqKgq),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KmovbKebKgb),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KmovdKedKgd),
    last_opcode(0, Opcode::SetnoEb),
];
// opcode 0F 92 — KMOV GPR→K (VEX.L0) + SETcc (non-VEX)
pub(super) const BxOpcodeTable0F92: [u64; 5] = [
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KmovwKgwEw),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KmovbKgbEb),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_F2, Opcode::KmovdKgdEd),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_F2, Opcode::KmovqKgqEq),
    last_opcode(0, Opcode::SetbEb),
];
// opcode 0F 93 — KMOV K→GPR (VEX.L0) + SETcc (non-VEX)
pub(super) const BxOpcodeTable0F93: [u64; 5] = [
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KmovwGdKew),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KmovbGdKeb),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_F2, Opcode::KmovdGdKed),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_F2, Opcode::KmovqGqKeq),
    last_opcode(0, Opcode::SetnbEb),
];
pub(super) const BxOpcodeTable0F94: [u64; 1] = [last_opcode(0, Opcode::SetzEb)];
pub(super) const BxOpcodeTable0F95: [u64; 1] = [last_opcode(0, Opcode::SetnzEb)];
pub(super) const BxOpcodeTable0F96: [u64; 1] = [last_opcode(0, Opcode::SetbeEb)];
pub(super) const BxOpcodeTable0F97: [u64; 1] = [last_opcode(0, Opcode::SetnbeEb)];
// opcode 0F 98 — KORTEST (VEX.L0) + SETcc (non-VEX)
pub(super) const BxOpcodeTable0F98: [u64; 5] = [
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KortestwKgwKew),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_NO_PREFIX, Opcode::KortestqKgqKeq),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KortestbKgbKeb),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KortestdKgdKed),
    last_opcode(0, Opcode::SetsEb),
];
// opcode 0F 99 — KTEST (VEX.L0) + SETcc (non-VEX)
pub(super) const BxOpcodeTable0F99: [u64; 5] = [
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_NO_PREFIX, Opcode::KtestwKgwKew),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_NO_PREFIX, Opcode::KtestqKgqKeq),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KtestbKgbKeb),
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KtestdKgdKed),
    last_opcode(0, Opcode::SetnsEb),
];
pub(super) const BxOpcodeTable0F9A: [u64; 1] = [last_opcode(0, Opcode::SetpEb)];
pub(super) const BxOpcodeTable0F9B: [u64; 1] = [last_opcode(0, Opcode::SetnpEb)];
pub(super) const BxOpcodeTable0F9C: [u64; 1] = [last_opcode(0, Opcode::SetlEb)];
pub(super) const BxOpcodeTable0F9D: [u64; 1] = [last_opcode(0, Opcode::SetnlEb)];
pub(super) const BxOpcodeTable0F9E: [u64; 1] = [last_opcode(0, Opcode::SetleEb)];
pub(super) const BxOpcodeTable0F9F: [u64; 1] = [last_opcode(0, Opcode::SetnleEb)];

// opcode 0F A0
pub(super) const BxOpcodeTable0FA0: [u64; 3] = [
    form_opcode(ATTR_IS64 | ATTR_OS32_64, Opcode::PushOp64Sw),
    form_opcode(ATTR_IS32 | ATTR_OS32, Opcode::PushOp32Sw),
    last_opcode(ATTR_OS16, Opcode::PushOp16Sw),
];

// opcode 0F A1
pub(super) const BxOpcodeTable0FA1: [u64; 3] = [
    form_opcode(ATTR_IS64 | ATTR_OS32_64, Opcode::PopOp64Sw),
    form_opcode(ATTR_IS32 | ATTR_OS32, Opcode::PopOp32Sw),
    last_opcode(ATTR_OS16, Opcode::PopOp16Sw),
];

// opcode 0F A2
pub(super) const BxOpcodeTable0FA2: [u64; 1] = [last_opcode(0, Opcode::Cpuid)];

// opcode 0F A3
pub(super) const BxOpcodeTable0FA3: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::BtEqGq),
    form_opcode(ATTR_OS32, Opcode::BtEdGd),
    last_opcode(ATTR_OS16, Opcode::BtEwGw),
];

// opcode 0F A4
pub(super) const BxOpcodeTable0FA4: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::ShldEqGqIb),
    form_opcode(ATTR_OS32, Opcode::ShldEdGdIb),
    last_opcode(ATTR_OS16, Opcode::ShldEwGwIb),
];

// opcode 0F A5
pub(super) const BxOpcodeTable0FA5: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::ShldEqGq),
    form_opcode(ATTR_OS32, Opcode::ShldEdGd),
    last_opcode(ATTR_OS16, Opcode::ShldEwGw),
];

// opcode 0F A8
pub(super) const BxOpcodeTable0FA8: [u64; 3] = [
    form_opcode(ATTR_IS64 | ATTR_OS32_64, Opcode::PushOp64Sw),
    form_opcode(ATTR_IS32 | ATTR_OS32, Opcode::PushOp32Sw),
    last_opcode(ATTR_OS16, Opcode::PushOp16Sw),
];

// opcode 0F A9
pub(super) const BxOpcodeTable0FA9: [u64; 3] = [
    form_opcode(ATTR_IS64 | ATTR_OS32_64, Opcode::PopOp64Sw),
    form_opcode(ATTR_IS32 | ATTR_OS32, Opcode::PopOp32Sw),
    last_opcode(ATTR_OS16, Opcode::PopOp16Sw),
];

// opcode 0F AA
pub(super) const BxOpcodeTable0FAA: [u64; 1] = [last_opcode(0, Opcode::Rsm)];

// opcode 0F AB
pub(super) const BxOpcodeTable0FAB: [u64; 3] = [
    form_opcode_lockable(ATTR_OS64, Opcode::BtsEqGq),
    form_opcode_lockable(ATTR_OS32, Opcode::BtsEdGd),
    last_opcode_lockable(ATTR_OS16, Opcode::BtsEwGw),
];

// opcode 0F AC
pub(super) const BxOpcodeTable0FAC: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::ShrdEqGqIb),
    form_opcode(ATTR_OS32, Opcode::ShrdEdGdIb),
    last_opcode(ATTR_OS16, Opcode::ShrdEwGwIb),
];

// opcode 0F AD
pub(super) const BxOpcodeTable0FAD: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::ShrdEqGq),
    form_opcode(ATTR_OS32, Opcode::ShrdEdGd),
    last_opcode(ATTR_OS16, Opcode::ShrdEwGw),
];

// opcode 0F AE
pub(super) const BxOpcodeTable0FAE: [u64; 28] = [
    form_opcode(
        ATTR_OS16_32 | ATTR_IS64 | ATTR_MODC0 | ATTR_NNN0 | ATTR_SSE_PREFIX_F3,
        Opcode::RdfsbaseEd,
    ),
    form_opcode(
        ATTR_OS16_32 | ATTR_IS64 | ATTR_MODC0 | ATTR_NNN1 | ATTR_SSE_PREFIX_F3,
        Opcode::RdgsbaseEd,
    ),
    form_opcode(
        ATTR_OS16_32 | ATTR_IS64 | ATTR_MODC0 | ATTR_NNN2 | ATTR_SSE_PREFIX_F3,
        Opcode::WrfsbaseEd,
    ),
    form_opcode(
        ATTR_OS16_32 | ATTR_IS64 | ATTR_MODC0 | ATTR_NNN3 | ATTR_SSE_PREFIX_F3,
        Opcode::WrgsbaseEd,
    ),
    form_opcode(
        ATTR_OS64 | ATTR_MODC0 | ATTR_NNN0 | ATTR_SSE_PREFIX_F3,
        Opcode::RdfsbaseEq,
    ),
    form_opcode(
        ATTR_OS64 | ATTR_MODC0 | ATTR_NNN1 | ATTR_SSE_PREFIX_F3,
        Opcode::RdgsbaseEq,
    ),
    form_opcode(
        ATTR_OS64 | ATTR_MODC0 | ATTR_NNN2 | ATTR_SSE_PREFIX_F3,
        Opcode::WrfsbaseEq,
    ),
    form_opcode(
        ATTR_OS64 | ATTR_MODC0 | ATTR_NNN3 | ATTR_SSE_PREFIX_F3,
        Opcode::WrgsbaseEq,
    ),
    form_opcode(
        ATTR_OS16_32 | ATTR_NNN5 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3,
        Opcode::Incsspd,
    ),
    form_opcode(
        ATTR_OS64 | ATTR_NNN5 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3,
        Opcode::Incsspq,
    ),
    form_opcode(ATTR_NNN5 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX, Opcode::Lfence),
    form_opcode(ATTR_NNN6 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX, Opcode::Mfence),
    form_opcode(ATTR_NNN7 | ATTR_MODC0 | ATTR_SSE_NO_PREFIX, Opcode::Sfence),
    form_opcode(
        ATTR_NNN6 | ATTR_MODC0 | ATTR_SSE_PREFIX_66,
        Opcode::TpauseEd,
    ),
    form_opcode(
        ATTR_NNN6 | ATTR_MODC0 | ATTR_SSE_PREFIX_F2,
        Opcode::UmwaitEd,
    ),
    form_opcode(
        ATTR_NNN6 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3 | ATTR_OS64,
        Opcode::UmonitorEq,
    ),
    form_opcode(
        ATTR_NNN6 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3,
        Opcode::UmonitorEd,
    ),
    form_opcode(
        ATTR_NNN0 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::Fxsave,
    ),
    form_opcode(
        ATTR_NNN1 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::Fxrstor,
    ),
    form_opcode(
        ATTR_NNN2 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::Ldmxcsr,
    ),
    form_opcode(
        ATTR_NNN3 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::Stmxcsr,
    ),
    form_opcode(ATTR_NNN4 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX, Opcode::Xsave),
    form_opcode(
        ATTR_NNN5 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::Xrstor,
    ),
    form_opcode(
        ATTR_NNN6 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::Xsaveopt,
    ),
    form_opcode(ATTR_NNN6 | ATTR_MOD_MEM | ATTR_SSE_PREFIX_66, Opcode::Clwb),
    form_opcode(
        ATTR_NNN6 | ATTR_MOD_MEM | ATTR_SSE_PREFIX_F3,
        Opcode::Clrssbsy,
    ),
    form_opcode(
        ATTR_NNN7 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::Clflush,
    ),
    last_opcode(
        ATTR_NNN7 | ATTR_MOD_MEM | ATTR_SSE_PREFIX_66,
        Opcode::Clflushopt,
    ),
];

// opcode 0F AF
pub(super) const BxOpcodeTable0FAF: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::ImulGqEq),
    form_opcode(ATTR_OS32, Opcode::ImulGdEd),
    last_opcode(ATTR_OS16, Opcode::ImulGwEw),
];

// opcode 0F B0
pub(super) const BxOpcodeTable0FB0: [u64; 1] = [last_opcode_lockable(0, Opcode::CmpxchgEbGb)];

// opcode 0F B1
pub(super) const BxOpcodeTable0FB1: [u64; 3] = [
    form_opcode_lockable(ATTR_OS64, Opcode::CmpxchgEqGq),
    form_opcode_lockable(ATTR_OS32, Opcode::CmpxchgEdGd),
    last_opcode_lockable(ATTR_OS16, Opcode::CmpxchgEwGw),
];

// opcode 0F B2
pub(super) const BxOpcodeTable0FB2: [u64; 3] = [
    form_opcode(ATTR_OS64 | ATTR_MOD_MEM, Opcode::LssGqMp), // TODO: LSS_GdMp for AMD CPU
    form_opcode(ATTR_OS32 | ATTR_MOD_MEM, Opcode::LssGdMp),
    last_opcode(ATTR_OS16 | ATTR_MOD_MEM, Opcode::LssGwMp),
];

// opcode 0F B3
pub(super) const BxOpcodeTable0FB3: [u64; 3] = [
    form_opcode_lockable(ATTR_OS64, Opcode::BtrEqGq),
    form_opcode_lockable(ATTR_OS32, Opcode::BtrEdGd),
    last_opcode_lockable(ATTR_OS16, Opcode::BtrEwGw),
];

// opcode 0F B4
pub(super) const BxOpcodeTable0FB4: [u64; 3] = [
    form_opcode(ATTR_OS64 | ATTR_MOD_MEM, Opcode::LfsGqMp), // TODO: LFS_GdMp for AMD CPU
    form_opcode(ATTR_OS32 | ATTR_MOD_MEM, Opcode::LfsGdMp),
    last_opcode(ATTR_OS16 | ATTR_MOD_MEM, Opcode::LfsGwMp),
];

// opcode 0F B5
pub(super) const BxOpcodeTable0FB5: [u64; 3] = [
    form_opcode(ATTR_OS64 | ATTR_MOD_MEM, Opcode::LgsGqMp), // TODO: LGS_GdMp for AMD CPU
    form_opcode(ATTR_OS32 | ATTR_MOD_MEM, Opcode::LgsGdMp),
    last_opcode(ATTR_OS16 | ATTR_MOD_MEM, Opcode::LgsGwMp),
];

// opcode 0F B6
pub(super) const BxOpcodeTable0FB6: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::MovzxGqEb),
    form_opcode(ATTR_OS32, Opcode::MovzxGdEb),
    last_opcode(ATTR_OS16, Opcode::MovzxGwEb),
];

// opcode 0F B7
pub(super) const BxOpcodeTable0FB7: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::MovzxGqEw),
    form_opcode(ATTR_OS32, Opcode::MovzxGdEw),
    last_opcode(ATTR_OS16, Opcode::MovGwEw), // MOVZX_GwEw
];

// opcode 0F B8
pub(super) const BxOpcodeTable0FB8: [u64; 3] = [
    form_opcode(ATTR_OS64 | ATTR_SSE_PREFIX_F3, Opcode::PopcntGqEq),
    form_opcode(ATTR_OS32 | ATTR_SSE_PREFIX_F3, Opcode::PopcntGdEd),
    last_opcode(ATTR_OS16 | ATTR_SSE_PREFIX_F3, Opcode::PopcntGwEw),
];

// opcode 0F B9
pub(super) const BxOpcodeTable0FB9: [u64; 1] = [last_opcode(0, Opcode::Ud1)];

// opcode 0F BA
pub(super) const BxOpcodeTable0FBA: [u64; 12] = [
    form_opcode(ATTR_NNN4 | ATTR_OS64, Opcode::BtEqIb),
    form_opcode_lockable(ATTR_NNN5 | ATTR_OS64, Opcode::BtsEqIb),
    form_opcode_lockable(ATTR_NNN6 | ATTR_OS64, Opcode::BtrEqIb),
    form_opcode_lockable(ATTR_NNN7 | ATTR_OS64, Opcode::BtcEqIb),
    form_opcode(ATTR_NNN4 | ATTR_OS32, Opcode::BtEdIb),
    form_opcode_lockable(ATTR_NNN5 | ATTR_OS32, Opcode::BtsEdIb),
    form_opcode_lockable(ATTR_NNN6 | ATTR_OS32, Opcode::BtrEdIb),
    form_opcode_lockable(ATTR_NNN7 | ATTR_OS32, Opcode::BtcEdIb),
    form_opcode(ATTR_NNN4 | ATTR_OS16, Opcode::BtEwIb),
    form_opcode_lockable(ATTR_NNN5 | ATTR_OS16, Opcode::BtsEwIb),
    form_opcode_lockable(ATTR_NNN6 | ATTR_OS16, Opcode::BtrEwIb),
    last_opcode_lockable(ATTR_NNN7 | ATTR_OS16, Opcode::BtcEwIb),
];

// opcode 0F BB
pub(super) const BxOpcodeTable0FBB: [u64; 3] = [
    form_opcode_lockable(ATTR_OS64, Opcode::BtcEqGq),
    form_opcode_lockable(ATTR_OS32, Opcode::BtcEdGd),
    last_opcode_lockable(ATTR_OS16, Opcode::BtcEwGw),
];

// opcode 0F BC
pub(super) const BxOpcodeTable0FBC: [u64; 6] = [
    form_opcode(ATTR_OS64 | ATTR_SSE_PREFIX_F3, Opcode::TzcntGqEq),
    form_opcode(ATTR_OS32 | ATTR_SSE_PREFIX_F3, Opcode::TzcntGdEd),
    form_opcode(ATTR_OS16 | ATTR_SSE_PREFIX_F3, Opcode::TzcntGwEw),
    form_opcode(ATTR_OS64, Opcode::BsfGqEq),
    form_opcode(ATTR_OS32, Opcode::BsfGdEd),
    last_opcode(ATTR_OS16, Opcode::BsfGwEw),
];

// opcode 0F BD
pub(super) const BxOpcodeTable0FBD: [u64; 6] = [
    form_opcode(ATTR_OS64 | ATTR_SSE_PREFIX_F3, Opcode::LzcntGqEq),
    form_opcode(ATTR_OS32 | ATTR_SSE_PREFIX_F3, Opcode::LzcntGdEd),
    form_opcode(ATTR_OS16 | ATTR_SSE_PREFIX_F3, Opcode::LzcntGwEw),
    form_opcode(ATTR_OS64, Opcode::BsrGqEq),
    form_opcode(ATTR_OS32, Opcode::BsrGdEd),
    last_opcode(ATTR_OS16, Opcode::BsrGwEw),
];

// opcode 0F BE
pub(super) const BxOpcodeTable0FBE: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::MovsxGqEb),
    form_opcode(ATTR_OS32, Opcode::MovsxGdEb),
    last_opcode(ATTR_OS16, Opcode::MovsxGwEb),
];

// opcode 0F BF
pub(super) const BxOpcodeTable0FBF: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::MovsxGqEw),
    form_opcode(ATTR_OS32, Opcode::MovsxGdEw),
    last_opcode(ATTR_OS16, Opcode::MovGwEw), // MOVSX_GwEw
];

// opcode 0F C0
pub(super) const BxOpcodeTable0FC0: [u64; 1] = [last_opcode_lockable(0, Opcode::XaddEbGb)];

// opcode 0F C1
pub(super) const BxOpcodeTable0FC1: [u64; 3] = [
    form_opcode_lockable(ATTR_OS64, Opcode::XaddEqGq),
    form_opcode_lockable(ATTR_OS32, Opcode::XaddEdGd),
    last_opcode_lockable(ATTR_OS16, Opcode::XaddEwGw),
];

// opcode 0F C2
pub(super) const BxOpcodeTable0FC2: [u64; 4] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::CmppsVpsWpsIb),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::CmppdVpdWpdIb),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::CmpssVssWssIb),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::CmpsdVsdWsdIb),
];

pub(super) const BxOpcodeTable0FC3: [u64; 3] = [
    form_opcode(
        ATTR_SSE_NO_PREFIX | ATTR_MOD_MEM | ATTR_IS64 | ATTR_OS16_32,
        Opcode::MovntiOp64MdGd,
    ),
    form_opcode(
        ATTR_SSE_NO_PREFIX | ATTR_MOD_MEM | ATTR_IS64 | ATTR_OS64,
        Opcode::MovntiMqGq,
    ),
    last_opcode(
        ATTR_SSE_NO_PREFIX | ATTR_MOD_MEM | ATTR_IS32,
        Opcode::MovntiOp32MdGd,
    ),
];

// opcode 0F C4
pub(super) const BxOpcodeTable0FC4: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PinsrwPqEwIb),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PinsrwVdqEwIb),
];

// opcode 0F C5
pub(super) const BxOpcodeTable0FC5: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_MODC0, Opcode::PextrwGdNqIb),
    last_opcode(ATTR_SSE_PREFIX_66 | ATTR_MODC0, Opcode::PextrwGdUdqIb),
];

// opcode 0F C6
pub(super) const BxOpcodeTable0FC6: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::ShufpsVpsWpsIb),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::ShufpdVpdWpdIb),
];

// opcode 0F C7
pub(super) const BxOpcodeTable0FC7: [u64; 17] = [
    form_opcode(
        ATTR_OS16 | ATTR_MODC0 | ATTR_NNN6 | ATTR_NO_SSE_PREFIX_F2_F3,
        Opcode::RdrandEw,
    ),
    form_opcode(
        ATTR_OS16 | ATTR_MODC0 | ATTR_NNN7 | ATTR_NO_SSE_PREFIX_F2_F3,
        Opcode::RdseedEw,
    ),
    form_opcode(
        ATTR_OS32 | ATTR_MODC0 | ATTR_NNN6 | ATTR_NO_SSE_PREFIX_F2_F3,
        Opcode::RdrandEd,
    ),
    form_opcode(
        ATTR_OS32 | ATTR_MODC0 | ATTR_NNN7 | ATTR_NO_SSE_PREFIX_F2_F3,
        Opcode::RdseedEd,
    ),
    form_opcode(
        ATTR_OS64 | ATTR_MODC0 | ATTR_NNN6 | ATTR_NO_SSE_PREFIX_F2_F3,
        Opcode::RdrandEq,
    ),
    form_opcode(
        ATTR_OS64 | ATTR_MODC0 | ATTR_NNN7 | ATTR_NO_SSE_PREFIX_F2_F3,
        Opcode::RdseedEq,
    ),
    form_opcode(
        ATTR_IS64 | ATTR_MODC0 | ATTR_NNN6 | ATTR_SSE_PREFIX_F3,
        Opcode::SenduipiGq,
    ),
    form_opcode(ATTR_NNN7 | ATTR_MODC0 | ATTR_SSE_PREFIX_F3, Opcode::RdpidEd),
    form_opcode_lockable(ATTR_OS16_32 | ATTR_NNN1 | ATTR_MOD_MEM, Opcode::Cmpxchg8b),
    form_opcode_lockable(ATTR_OS64 | ATTR_NNN1 | ATTR_MOD_MEM, Opcode::CMPXCHG16B),
    form_opcode(
        ATTR_NNN3 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::Xrstors,
    ),
    form_opcode(
        ATTR_NNN4 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::Xsavec,
    ),
    form_opcode(
        ATTR_NNN5 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::Xsaves,
    ),
    form_opcode(
        ATTR_NNN6 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::VmptrldMq,
    ),
    form_opcode(
        ATTR_NNN6 | ATTR_MOD_MEM | ATTR_SSE_PREFIX_66,
        Opcode::VmclearMq,
    ),
    form_opcode(
        ATTR_NNN6 | ATTR_MOD_MEM | ATTR_SSE_PREFIX_F3,
        Opcode::VmxonMq,
    ),
    last_opcode(
        ATTR_NNN7 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::VmptrstMq,
    ),
];

// opcode 0F C8 - 0F CF
pub(super) const BxOpcodeTable0FC8x0FCF: [u64; 3] = [
    form_opcode(ATTR_OS64, Opcode::BswapRrx),
    form_opcode(ATTR_OS32, Opcode::BswapErx),
    last_opcode(ATTR_OS16, Opcode::BswapRx),
];

// opcode 0F D0
pub(super) const BxOpcodeTable0FD0: [u64; 2] = [
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::AddsubpdVpdWpd),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::AddsubpsVpsWps),
];

// opcode 0F D1
pub(super) const BxOpcodeTable0FD1: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsrlwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsrlwVdqWdq),
];

// opcode 0F D2
pub(super) const BxOpcodeTable0FD2: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsrldPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsrldVdqWdq),
];

// opcode 0F D3
pub(super) const BxOpcodeTable0FD3: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsrlqPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsrlqVdqWdq),
];

// opcode 0F D4
pub(super) const BxOpcodeTable0FD4: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PaddqPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PaddqVdqWdq),
];

// opcode 0F D5
pub(super) const BxOpcodeTable0FD5: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PmullwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmullwVdqWdq),
];

// opcode 0F D6
pub(super) const BxOpcodeTable0FD6: [u64; 3] = [
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::MovqWqVq),
    form_opcode(ATTR_SSE_PREFIX_F3 | ATTR_MODC0, Opcode::Movq2dqVdqQq),
    last_opcode(ATTR_SSE_PREFIX_F2 | ATTR_MODC0, Opcode::Movdq2qPqUdq),
];

// opcode 0F D7
pub(super) const BxOpcodeTable0FD7: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_MODC0, Opcode::PmovmskbGdNq),
    last_opcode(ATTR_SSE_PREFIX_66 | ATTR_MODC0, Opcode::PmovmskbGdUdq),
];

// opcode 0F D8
pub(super) const BxOpcodeTable0FD8: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsubusbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsubusbVdqWdq),
];

// opcode 0F D9
pub(super) const BxOpcodeTable0FD9: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsubuswPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsubuswVdqWdq),
];

// opcode 0F DA
pub(super) const BxOpcodeTable0FDA: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PminubPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PminubVdqWdq),
];

// opcode 0F DB
pub(super) const BxOpcodeTable0FDB: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PandPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PandVdqWdq),
];

// opcode 0F DC
pub(super) const BxOpcodeTable0FDC: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PaddusbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PaddusbVdqWdq),
];

// opcode 0F DD
pub(super) const BxOpcodeTable0FDD: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PadduswPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PadduswVdqWdq),
];

// opcode 0F DE
pub(super) const BxOpcodeTable0FDE: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PmaxubPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmaxubVdqWdq),
];

// opcode 0F DF
pub(super) const BxOpcodeTable0FDF: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PandnPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PandnVdqWdq),
];

// opcode 0F E0
pub(super) const BxOpcodeTable0FE0: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PavgbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PavgbVdqWdq),
];

// opcode 0F E1
pub(super) const BxOpcodeTable0FE1: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsrawPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsrawVdqWdq),
];

// opcode 0F E2
pub(super) const BxOpcodeTable0FE2: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsradPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsradVdqWdq),
];

// opcode 0F E3
pub(super) const BxOpcodeTable0FE3: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PavgwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PavgwVdqWdq),
];

// opcode 0F E4
pub(super) const BxOpcodeTable0FE4: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PmulhuwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmulhuwVdqWdq),
];

// opcode 0F E5
pub(super) const BxOpcodeTable0FE5: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PmulhwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmulhwVdqWdq),
];

// opcode 0F E6
pub(super) const BxOpcodeTable0FE6: [u64; 3] = [
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::Cvttpd2dqVqWpd),
    form_opcode(ATTR_SSE_PREFIX_F3, Opcode::Cvtdq2pdVpdWq),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::Cvtpd2dqVqWpd),
];

// opcode 0F E7
pub(super) const BxOpcodeTable0FE7: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_MOD_MEM, Opcode::MovntqMqPq),
    last_opcode(ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM, Opcode::MovntdqMdqVdq),
];

// opcode 0F E8
pub(super) const BxOpcodeTable0FE8: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsubsbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsubsbVdqWdq),
];

// opcode 0F E9
pub(super) const BxOpcodeTable0FE9: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsubswPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsubswVdqWdq),
];

// opcode 0F EA
pub(super) const BxOpcodeTable0FEA: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PminswPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PminswVdqWdq),
];

// opcode 0F EB
pub(super) const BxOpcodeTable0FEB: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PorPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PorVdqWdq),
];

// opcode 0F EC
pub(super) const BxOpcodeTable0FEC: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PaddsbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PaddsbVdqWdq),
];

// opcode 0F ED
pub(super) const BxOpcodeTable0FED: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PaddswPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PaddswVdqWdq),
];

// opcode 0F EE
pub(super) const BxOpcodeTable0FEE: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PmaxswPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmaxswVdqWdq),
];

// opcode 0F EF
pub(super) const BxOpcodeTable0FEF: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PxorPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PxorVdqWdq),
];

// opcode 0F F0
pub(super) const BxOpcodeTable0FF0: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_F2 | ATTR_MOD_MEM,
    Opcode::LddquVdqMdq,
)];

// opcode 0F F1
pub(super) const BxOpcodeTable0FF1: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsllwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsllwVdqWdq),
];

// opcode 0F F2
pub(super) const BxOpcodeTable0FF2: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PslldPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PslldVdqWdq),
];

// opcode 0F F3
pub(super) const BxOpcodeTable0FF3: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsllqPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsllqVdqWdq),
];

// opcode 0F F4
pub(super) const BxOpcodeTable0FF4: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PmuludqPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmuludqVdqWdq),
];

// opcode 0F F5
pub(super) const BxOpcodeTable0FF5: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PmaddwdPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmaddwdVdqWdq),
];

// opcode 0F F6
pub(super) const BxOpcodeTable0FF6: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsadbwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsadbwVdqWdq),
];

// opcode 0F F7
pub(super) const BxOpcodeTable0FF7: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX | ATTR_MODC0, Opcode::MaskmovqPqNq),
    last_opcode(ATTR_SSE_PREFIX_66 | ATTR_MODC0, Opcode::MaskmovdquVdqUdq),
];

// opcode 0F F8
pub(super) const BxOpcodeTable0FF8: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsubbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsubbVdqWdq),
];

// opcode 0F F9
pub(super) const BxOpcodeTable0FF9: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsubwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsubwVdqWdq),
];

// opcode 0F FA
pub(super) const BxOpcodeTable0FFA: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsubdPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsubdVdqWdq),
];

// opcode 0F FB
pub(super) const BxOpcodeTable0FFB: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsubqPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsubqVdqWdq),
];

// opcode 0F FC
pub(super) const BxOpcodeTable0FFC: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PaddbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PaddbVdqWdq),
];

// opcode 0F FD
pub(super) const BxOpcodeTable0FFD: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PaddwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PaddwVdqWdq),
];

// opcode 0F FE
pub(super) const BxOpcodeTable0FFE: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PadddPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PadddVdqWdq),
];

// opcode 0F FF
pub(super) const BxOpcodeTable0FFF: [u64; 1] = [last_opcode(0, Opcode::Ud0)];
