#![allow(non_upper_case_globals)]
#![allow(unused)] // TODO: don't forget to uncomment

use super::fetchdecode::*;
use super::fetchdecode_generated::*;
use super::ia_opcodes::Opcode;

/* ************************************************************************ */
/* 3-byte opcode table (Table A-5, 0F 3A) */

use super::fetchdecode_generated::*;

pub const BxOpcodeTable0F3A08: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::RoundpsVpsWpsIb)];
pub const BxOpcodeTable0F3A09: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::RoundpdVpdWpdIb)];
pub const BxOpcodeTable0F3A0A: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::RoundssVssWssIb)];
pub const BxOpcodeTable0F3A0B: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::RoundsdVsdWsdIb)];
pub const BxOpcodeTable0F3A0C: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::BlendpsVpsWpsIb)];
pub const BxOpcodeTable0F3A0D: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::BlendpdVpdWpdIb)];
pub const BxOpcodeTable0F3A0E: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PblendwVdqWdqIb)];

pub const BxOpcodeTable0F3A0F: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PalignrPqQqIb),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PalignrVdqWdqIb),
];

pub const BxOpcodeTable0F3A14: [u64; 2] = [
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_MODC0, Opcode::PextrbEdVdqIbR),
    last_opcode(ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM, Opcode::PextrbMbVdqIbM),
];
pub const BxOpcodeTable0F3A15: [u64; 2] = [
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_MODC0, Opcode::PextrwEdVdqIbR),
    last_opcode(ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM, Opcode::PextrwMwVdqIbM),
];

// opcode 0F 3A 16
pub const BxOpcodeTable0F3A16: [u64; 2] = [
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_OS64, Opcode::PextrqEqVdqIb),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PextrdEdVdqIb),
];

pub const BxOpcodeTable0F3A17: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::ExtractpsEdVpsIb)];
pub const BxOpcodeTable0F3A20: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PinsrbVdqEbIb)];
pub const BxOpcodeTable0F3A21: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::InsertpsVpsWssIb)];

// opcode 0F 3A 22
pub const BxOpcodeTable0F3A22: [u64; 2] = [
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_OS64, Opcode::PinsrqVdqEqIb),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PinsrdVdqEdIb),
];

pub const BxOpcodeTable0F3A40: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::DppsVpsWpsIb)];
pub const BxOpcodeTable0F3A41: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::DppdVpdWpdIb)];
pub const BxOpcodeTable0F3A42: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::MpsadbwVdqWdqIb)];
pub const BxOpcodeTable0F3A44: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PclmulqdqVdqWdqIb)];

pub const BxOpcodeTable0F3A60: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpestrmVdqWdqIb)];
pub const BxOpcodeTable0F3A61: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpestriVdqWdqIb)];
pub const BxOpcodeTable0F3A62: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpistrmVdqWdqIb)];
pub const BxOpcodeTable0F3A63: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpistriVdqWdqIb)];

pub const BxOpcodeTable0F3ACC: [u64; 1] =
    [last_opcode(ATTR_SSE_NO_PREFIX, Opcode::Sha1rnds4VdqWdqIb)];
pub const BxOpcodeTable0F3ACE: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_66,
    Opcode::Gf2p8affineqbVdqWdqIb,
)];
pub const BxOpcodeTable0F3ACF: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_66,
    Opcode::Gf2p8affineinvqbVdqWdqIb,
)];
pub const BxOpcodeTable0F3ADF: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_66,
    Opcode::AeskeygenassistVdqWdqIb,
)];

/* ************************************************************************ */

pub const BxOpcodeTable0F3A: [&[u64]; 256] = [
    // 3-byte opcode 0x0F 0x3A
    /* 0F 3A 00 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 01 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 02 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 03 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 04 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 05 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 06 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 07 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 08 */ &BxOpcodeTable0F3A08,
    /* 0F 3A 09 */ &BxOpcodeTable0F3A09,
    /* 0F 3A 0A */ &BxOpcodeTable0F3A0A,
    /* 0F 3A 0B */ &BxOpcodeTable0F3A0B,
    /* 0F 3A 0C */ &BxOpcodeTable0F3A0C,
    /* 0F 3A 0D */ &BxOpcodeTable0F3A0D,
    /* 0F 3A 0E */ &BxOpcodeTable0F3A0E,
    /* 0F 3A 0F */ &BxOpcodeTable0F3A0F,
    /* 0F 3A 10 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 11 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 12 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 13 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 14 */ &BxOpcodeTable0F3A14,
    /* 0F 3A 15 */ &BxOpcodeTable0F3A15,
    /* 0F 3A 16 */ &BxOpcodeTable0F3A16,
    /* 0F 3A 17 */ &BxOpcodeTable0F3A17,
    /* 0F 3A 18 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 19 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 1A */ &BxOpcodeGroup_ERR,
    /* 0F 3A 1B */ &BxOpcodeGroup_ERR,
    /* 0F 3A 1C */ &BxOpcodeGroup_ERR,
    /* 0F 3A 1D */ &BxOpcodeGroup_ERR,
    /* 0F 3A 1E */ &BxOpcodeGroup_ERR,
    /* 0F 3A 1F */ &BxOpcodeGroup_ERR,
    /* 0F 3A 20 */ &BxOpcodeTable0F3A20,
    /* 0F 3A 21 */ &BxOpcodeTable0F3A21,
    /* 0F 3A 22 */ &BxOpcodeTable0F3A22,
    /* 0F 3A 23 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 24 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 25 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 26 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 27 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 28 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 29 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 2A */ &BxOpcodeGroup_ERR,
    /* 0F 3A 2B */ &BxOpcodeGroup_ERR,
    /* 0F 3A 2C */ &BxOpcodeGroup_ERR,
    /* 0F 3A 2D */ &BxOpcodeGroup_ERR,
    /* 0F 3A 2E */ &BxOpcodeGroup_ERR,
    /* 0F 3A 2F */ &BxOpcodeGroup_ERR,
    /* 0F 3A 30 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 31 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 32 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 33 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 34 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 35 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 36 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 37 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 38 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 39 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 3A */ &BxOpcodeGroup_ERR,
    /* 0F 3A 3B */ &BxOpcodeGroup_ERR,
    /* 0F 3A 3C */ &BxOpcodeGroup_ERR,
    /* 0F 3A 3D */ &BxOpcodeGroup_ERR,
    /* 0F 3A 3E */ &BxOpcodeGroup_ERR,
    /* 0F 3A 3F */ &BxOpcodeGroup_ERR,
    /* 0F 3A 40 */ &BxOpcodeTable0F3A40,
    /* 0F 3A 41 */ &BxOpcodeTable0F3A41,
    /* 0F 3A 42 */ &BxOpcodeTable0F3A42,
    /* 0F 3A 43 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 44 */ &BxOpcodeTable0F3A44,
    /* 0F 3A 45 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 46 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 47 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 48 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 49 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 4A */ &BxOpcodeGroup_ERR,
    /* 0F 3A 4B */ &BxOpcodeGroup_ERR,
    /* 0F 3A 4C */ &BxOpcodeGroup_ERR,
    /* 0F 3A 4D */ &BxOpcodeGroup_ERR,
    /* 0F 3A 4E */ &BxOpcodeGroup_ERR,
    /* 0F 3A 4F */ &BxOpcodeGroup_ERR,
    /* 0F 3A 50 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 51 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 52 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 53 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 54 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 55 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 56 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 57 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 58 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 59 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 5A */ &BxOpcodeGroup_ERR,
    /* 0F 3A 5B */ &BxOpcodeGroup_ERR,
    /* 0F 3A 5C */ &BxOpcodeGroup_ERR,
    /* 0F 3A 5D */ &BxOpcodeGroup_ERR,
    /* 0F 3A 5E */ &BxOpcodeGroup_ERR,
    /* 0F 3A 5F */ &BxOpcodeGroup_ERR,
    /* 0F 3A 60 */ &BxOpcodeTable0F3A60,
    /* 0F 3A 61 */ &BxOpcodeTable0F3A61,
    /* 0F 3A 62 */ &BxOpcodeTable0F3A62,
    /* 0F 3A 64 */ &BxOpcodeTable0F3A63,
    /* 0F 3A 64 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 65 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 66 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 67 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 68 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 69 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 6A */ &BxOpcodeGroup_ERR,
    /* 0F 3A 6B */ &BxOpcodeGroup_ERR,
    /* 0F 3A 6C */ &BxOpcodeGroup_ERR,
    /* 0F 3A 6D */ &BxOpcodeGroup_ERR,
    /* 0F 3A 6E */ &BxOpcodeGroup_ERR,
    /* 0F 3A 6F */ &BxOpcodeGroup_ERR,
    /* 0F 3A 70 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 71 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 72 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 73 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 74 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 75 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 76 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 77 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 78 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 79 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 7A */ &BxOpcodeGroup_ERR,
    /* 0F 3A 7B */ &BxOpcodeGroup_ERR,
    /* 0F 3A 7C */ &BxOpcodeGroup_ERR,
    /* 0F 3A 7D */ &BxOpcodeGroup_ERR,
    /* 0F 3A 7E */ &BxOpcodeGroup_ERR,
    /* 0F 3A 7F */ &BxOpcodeGroup_ERR,
    /* 0F 3A 80 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 81 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 82 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 83 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 84 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 85 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 86 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 87 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 88 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 89 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 8A */ &BxOpcodeGroup_ERR,
    /* 0F 3A 8B */ &BxOpcodeGroup_ERR,
    /* 0F 3A 8C */ &BxOpcodeGroup_ERR,
    /* 0F 3A 8D */ &BxOpcodeGroup_ERR,
    /* 0F 3A 8E */ &BxOpcodeGroup_ERR,
    /* 0F 3A 8F */ &BxOpcodeGroup_ERR,
    /* 0F 3A 90 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 91 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 92 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 93 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 94 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 95 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 96 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 97 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 98 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 99 */ &BxOpcodeGroup_ERR,
    /* 0F 3A 9A */ &BxOpcodeGroup_ERR,
    /* 0F 3A 9B */ &BxOpcodeGroup_ERR,
    /* 0F 3A 9C */ &BxOpcodeGroup_ERR,
    /* 0F 3A 9D */ &BxOpcodeGroup_ERR,
    /* 0F 3A 9E */ &BxOpcodeGroup_ERR,
    /* 0F 3A 9F */ &BxOpcodeGroup_ERR,
    /* 0F 3A A0 */ &BxOpcodeGroup_ERR,
    /* 0F 3A A1 */ &BxOpcodeGroup_ERR,
    /* 0F 3A A2 */ &BxOpcodeGroup_ERR,
    /* 0F 3A A3 */ &BxOpcodeGroup_ERR,
    /* 0F 3A A4 */ &BxOpcodeGroup_ERR,
    /* 0F 3A A5 */ &BxOpcodeGroup_ERR,
    /* 0F 3A A6 */ &BxOpcodeGroup_ERR,
    /* 0F 3A A7 */ &BxOpcodeGroup_ERR,
    /* 0F 3A A8 */ &BxOpcodeGroup_ERR,
    /* 0F 3A A9 */ &BxOpcodeGroup_ERR,
    /* 0F 3A AA */ &BxOpcodeGroup_ERR,
    /* 0F 3A AB */ &BxOpcodeGroup_ERR,
    /* 0F 3A AC */ &BxOpcodeGroup_ERR,
    /* 0F 3A AD */ &BxOpcodeGroup_ERR,
    /* 0F 3A AE */ &BxOpcodeGroup_ERR,
    /* 0F 3A AF */ &BxOpcodeGroup_ERR,
    /* 0F 3A B0 */ &BxOpcodeGroup_ERR,
    /* 0F 3A B1 */ &BxOpcodeGroup_ERR,
    /* 0F 3A B2 */ &BxOpcodeGroup_ERR,
    /* 0F 3A B3 */ &BxOpcodeGroup_ERR,
    /* 0F 3A B4 */ &BxOpcodeGroup_ERR,
    /* 0F 3A B5 */ &BxOpcodeGroup_ERR,
    /* 0F 3A B6 */ &BxOpcodeGroup_ERR,
    /* 0F 3A B7 */ &BxOpcodeGroup_ERR,
    /* 0F 3A B8 */ &BxOpcodeGroup_ERR,
    /* 0F 3A B9 */ &BxOpcodeGroup_ERR,
    /* 0F 3A BA */ &BxOpcodeGroup_ERR,
    /* 0F 3A BB */ &BxOpcodeGroup_ERR,
    /* 0F 3A BC */ &BxOpcodeGroup_ERR,
    /* 0F 3A BD */ &BxOpcodeGroup_ERR,
    /* 0F 3A BE */ &BxOpcodeGroup_ERR,
    /* 0F 3A BF */ &BxOpcodeGroup_ERR,
    /* 0F 3A C0 */ &BxOpcodeGroup_ERR,
    /* 0F 3A C1 */ &BxOpcodeGroup_ERR,
    /* 0F 3A C2 */ &BxOpcodeGroup_ERR,
    /* 0F 3A C3 */ &BxOpcodeGroup_ERR,
    /* 0F 3A C4 */ &BxOpcodeGroup_ERR,
    /* 0F 3A C5 */ &BxOpcodeGroup_ERR,
    /* 0F 3A C6 */ &BxOpcodeGroup_ERR,
    /* 0F 3A C7 */ &BxOpcodeGroup_ERR,
    /* 0F 3A C8 */ &BxOpcodeGroup_ERR,
    /* 0F 3A C9 */ &BxOpcodeGroup_ERR,
    /* 0F 3A CA */ &BxOpcodeGroup_ERR,
    /* 0F 3A CB */ &BxOpcodeGroup_ERR,
    /* 0F 3A CC */ &BxOpcodeTable0F3ACC,
    /* 0F 3A CD */ &BxOpcodeGroup_ERR,
    /* 0F 3A CE */ &BxOpcodeTable0F3ACE,
    /* 0F 3A CF */ &BxOpcodeTable0F3ACF,
    /* 0F 3A D0 */ &BxOpcodeGroup_ERR,
    /* 0F 3A D1 */ &BxOpcodeGroup_ERR,
    /* 0F 3A D2 */ &BxOpcodeGroup_ERR,
    /* 0F 3A D3 */ &BxOpcodeGroup_ERR,
    /* 0F 3A D4 */ &BxOpcodeGroup_ERR,
    /* 0F 3A D5 */ &BxOpcodeGroup_ERR,
    /* 0F 3A D6 */ &BxOpcodeGroup_ERR,
    /* 0F 3A D7 */ &BxOpcodeGroup_ERR,
    /* 0F 3A D8 */ &BxOpcodeGroup_ERR,
    /* 0F 3A D9 */ &BxOpcodeGroup_ERR,
    /* 0F 3A DA */ &BxOpcodeGroup_ERR,
    /* 0F 3A DB */ &BxOpcodeGroup_ERR,
    /* 0F 3A DC */ &BxOpcodeGroup_ERR,
    /* 0F 3A DD */ &BxOpcodeGroup_ERR,
    /* 0F 3A DE */ &BxOpcodeGroup_ERR,
    /* 0F 3A DF */ &BxOpcodeTable0F3ADF,
    /* 0F 3A E0 */ &BxOpcodeGroup_ERR,
    /* 0F 3A E1 */ &BxOpcodeGroup_ERR,
    /* 0F 3A E2 */ &BxOpcodeGroup_ERR,
    /* 0F 3A E3 */ &BxOpcodeGroup_ERR,
    /* 0F 3A E4 */ &BxOpcodeGroup_ERR,
    /* 0F 3A E5 */ &BxOpcodeGroup_ERR,
    /* 0F 3A E6 */ &BxOpcodeGroup_ERR,
    /* 0F 3A E7 */ &BxOpcodeGroup_ERR,
    /* 0F 3A E8 */ &BxOpcodeGroup_ERR,
    /* 0F 3A E9 */ &BxOpcodeGroup_ERR,
    /* 0F 3A EA */ &BxOpcodeGroup_ERR,
    /* 0F 3A EB */ &BxOpcodeGroup_ERR,
    /* 0F 3A EC */ &BxOpcodeGroup_ERR,
    /* 0F 3A ED */ &BxOpcodeGroup_ERR,
    /* 0F 3A EE */ &BxOpcodeGroup_ERR,
    /* 0F 3A EF */ &BxOpcodeGroup_ERR,
    /* 0F 3A F0 */ &BxOpcodeGroup_ERR,
    /* 0F 3A F1 */ &BxOpcodeGroup_ERR,
    /* 0F 3A F2 */ &BxOpcodeGroup_ERR,
    /* 0F 3A F3 */ &BxOpcodeGroup_ERR,
    /* 0F 3A F4 */ &BxOpcodeGroup_ERR,
    /* 0F 3A F5 */ &BxOpcodeGroup_ERR,
    /* 0F 3A F6 */ &BxOpcodeGroup_ERR,
    /* 0F 3A F7 */ &BxOpcodeGroup_ERR,
    /* 0F 3A F8 */ &BxOpcodeGroup_ERR,
    /* 0F 3A F9 */ &BxOpcodeGroup_ERR,
    /* 0F 3A FA */ &BxOpcodeGroup_ERR,
    /* 0F 3A FB */ &BxOpcodeGroup_ERR,
    /* 0F 3A FC */ &BxOpcodeGroup_ERR,
    /* 0F 3A FD */ &BxOpcodeGroup_ERR,
    /* 0F 3A FE */ &BxOpcodeGroup_ERR,
    /* 0F 3A FF */ &BxOpcodeGroup_ERR,
];
