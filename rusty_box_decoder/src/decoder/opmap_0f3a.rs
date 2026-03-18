//! Three-byte opcode map (0F 3A) — SSE/AVX immediate-operand tables.
//!
//! Matches Bochs `fetchdecode_opmap_0f3a.h` (Intel Table A-5).

#![allow(non_upper_case_globals, unused)]

use super::*;
use super::tables::*;
use crate::opcode::Opcode;

/* ************************************************************************ */
/* 3-byte opcode table (Table A-5, 0F 3A) */

pub(super) const BxOpcodeTable0F3A08: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::RoundpsVpsWpsIb)];
pub(super) const BxOpcodeTable0F3A09: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::RoundpdVpdWpdIb)];
pub(super) const BxOpcodeTable0F3A0A: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::RoundssVssWssIb)];
pub(super) const BxOpcodeTable0F3A0B: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::RoundsdVsdWsdIb)];
pub(super) const BxOpcodeTable0F3A0C: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::BlendpsVpsWpsIb)];
pub(super) const BxOpcodeTable0F3A0D: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::BlendpdVpdWpdIb)];
pub(super) const BxOpcodeTable0F3A0E: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PblendwVdqWdqIb)];

pub(super) const BxOpcodeTable0F3A0F: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PalignrPqQqIb),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PalignrVdqWdqIb),
];

pub(super) const BxOpcodeTable0F3A14: [u64; 2] = [
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_MODC0, Opcode::PextrbEdVdqIbR),
    last_opcode(ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM, Opcode::PextrbMbVdqIbM),
];
pub(super) const BxOpcodeTable0F3A15: [u64; 2] = [
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_MODC0, Opcode::PextrwEdVdqIbR),
    last_opcode(ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM, Opcode::PextrwMwVdqIbM),
];

// opcode 0F 3A 16
pub(super) const BxOpcodeTable0F3A16: [u64; 2] = [
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_OS64, Opcode::PextrqEqVdqIb),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PextrdEdVdqIb),
];

pub(super) const BxOpcodeTable0F3A17: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::ExtractpsEdVpsIb)];
pub(super) const BxOpcodeTable0F3A20: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PinsrbVdqEbIb)];
pub(super) const BxOpcodeTable0F3A21: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::InsertpsVpsWssIb)];

// opcode 0F 3A 22
pub(super) const BxOpcodeTable0F3A22: [u64; 2] = [
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_OS64, Opcode::PinsrqVdqEqIb),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PinsrdVdqEdIb),
];

// VINSERTF128 — VEX.256.66.0F3A.W0 18 /r ib
pub(super) const BxOpcodeTable0F3A18: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_66 | ATTR_VL256 | ATTR_VEX_W0,
    Opcode::V256Vinsertf128VdqHdqWdqIb,
)];

// VINSERTI128 — VEX.256.66.0F3A.W0 38 /r ib
pub(super) const BxOpcodeTable0F3A38: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_66 | ATTR_VL256 | ATTR_VEX_W0,
    Opcode::V256Vinserti128VdqHdqWdqIb,
)];

// VEXTRACTI128 — VEX.256.66.0F3A.W0 39 /r ib
pub(super) const BxOpcodeTable0F3A39: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_66 | ATTR_VL256 | ATTR_VEX_W0,
    Opcode::V256Vextracti128WdqVdqIb,
)];

pub(super) const BxOpcodeTable0F3A40: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::DppsVpsWpsIb)];
pub(super) const BxOpcodeTable0F3A41: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::DppdVpdWpdIb)];
pub(super) const BxOpcodeTable0F3A42: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::MpsadbwVdqWdqIb)];
pub(super) const BxOpcodeTable0F3A44: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PclmulqdqVdqWdqIb)];

pub(super) const BxOpcodeTable0F3A60: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpestrmVdqWdqIb)];
pub(super) const BxOpcodeTable0F3A61: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpestriVdqWdqIb)];
pub(super) const BxOpcodeTable0F3A62: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpistrmVdqWdqIb)];
pub(super) const BxOpcodeTable0F3A63: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpistriVdqWdqIb)];

pub(super) const BxOpcodeTable0F3ACC: [u64; 1] =
    [last_opcode(ATTR_SSE_NO_PREFIX, Opcode::Sha1rnds4VdqWdqIb)];
pub(super) const BxOpcodeTable0F3ACE: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_66,
    Opcode::Gf2p8affineqbVdqWdqIb,
)];
pub(super) const BxOpcodeTable0F3ACF: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_66,
    Opcode::Gf2p8affineinvqbVdqWdqIb,
)];
pub(super) const BxOpcodeTable0F3ADF: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_66,
    Opcode::AeskeygenassistVdqWdqIb,
)];

// VPERM2I128 (VEX.256.66.0F3A.W0 46 /r ib)
pub(super) const BxOpcodeTable0F3A46: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::V256Vperm2i128VdqHdqWdqIb)];

// RORX (VEX.LZ.F2.0F3A.W0/W1 F0 /r ib) — BMI2 rotate right extract
// Bochs fetchdecode_opmap_avx.cc:1602
pub(super) const BxOpcodeTable0F3AF0: [u64; 2] = [
    form_opcode(ATTR_SSE_PREFIX_F2 | ATTR_VL128 | ATTR_VEX_W0, Opcode::RorxGdEdIb),
    last_opcode(ATTR_SSE_PREFIX_F2 | ATTR_VL128 | ATTR_VEX_W1 | ATTR_IS64, Opcode::RorxGqEqIb),
];

/* ************************************************************************ */

// VPBLENDD — AVX2 Blend Packed Dwords (VEX.66.0F3A.W0 02 /r ib)
// Bochs: ATTR_SSE_PREFIX_66 | ATTR_VEX_W0 → BX_IA_VPBLENDD_VdqHdqWdqIb
// Works for both VL128 and VL256 (handler checks get_vl)
pub(super) const BxOpcodeTable0F3A02: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66 | ATTR_VEX_W0, Opcode::VpblenddVdqHdqWdqIb)];

// KSHIFTL/KSHIFTR — VEX.L0.66.0F3A.W0/W1 30-33 /r ib
pub(super) const BxOpcodeTable0F3A30: [u64; 2] = [
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KshiftlbKgbKebIb),
    last_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KshiftrbKgbKebIb),
];
pub(super) const BxOpcodeTable0F3A31: [u64; 2] = [
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KshiftlwKgwKewIb),
    last_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KshiftrwKgwKewIb),
];
pub(super) const BxOpcodeTable0F3A32: [u64; 2] = [
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KshiftldKgdKedIb),
    last_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KshiftrdKgdKedIb),
];
pub(super) const BxOpcodeTable0F3A33: [u64; 2] = [
    form_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W0 | ATTR_SSE_PREFIX_66, Opcode::KshiftlqKgqKeqIb),
    last_opcode(ATTR_VEX | ATTR_VL128 | ATTR_VEX_W1 | ATTR_SSE_PREFIX_66, Opcode::KshiftrqKgqKeqIb),
];

pub(super) const BxOpcodeTable0F3A: [&[u64]; 256] = [
    // 3-byte opcode 0x0F 0x3A
    /* 0F 3A 00 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 01 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 02 */ &BxOpcodeTable0F3A02,
    /* 0F 3A 03 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 04 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 05 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 06 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 07 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 08 */ &BxOpcodeTable0F3A08,
    /* 0F 3A 09 */ &BxOpcodeTable0F3A09,
    /* 0F 3A 0A */ &BxOpcodeTable0F3A0A,
    /* 0F 3A 0B */ &BxOpcodeTable0F3A0B,
    /* 0F 3A 0C */ &BxOpcodeTable0F3A0C,
    /* 0F 3A 0D */ &BxOpcodeTable0F3A0D,
    /* 0F 3A 0E */ &BxOpcodeTable0F3A0E,
    /* 0F 3A 0F */ &BxOpcodeTable0F3A0F,
    /* 0F 3A 10 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 11 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 12 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 13 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 14 */ &BxOpcodeTable0F3A14,
    /* 0F 3A 15 */ &BxOpcodeTable0F3A15,
    /* 0F 3A 16 */ &BxOpcodeTable0F3A16,
    /* 0F 3A 17 */ &BxOpcodeTable0F3A17,
    /* 0F 3A 18 */ &BxOpcodeTable0F3A18,
    /* 0F 3A 19 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 1A */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 1B */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 1C */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 1D */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 1E */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 1F */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 20 */ &BxOpcodeTable0F3A20,
    /* 0F 3A 21 */ &BxOpcodeTable0F3A21,
    /* 0F 3A 22 */ &BxOpcodeTable0F3A22,
    /* 0F 3A 23 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 24 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 25 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 26 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 27 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 28 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 29 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 2A */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 2B */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 2C */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 2D */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 2E */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 2F */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 30 */ &BxOpcodeTable0F3A30,
    /* 0F 3A 31 */ &BxOpcodeTable0F3A31,
    /* 0F 3A 32 */ &BxOpcodeTable0F3A32,
    /* 0F 3A 33 */ &BxOpcodeTable0F3A33,
    /* 0F 3A 34 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 35 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 36 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 37 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 38 */ &BxOpcodeTable0F3A38,
    /* 0F 3A 39 */ &BxOpcodeTable0F3A39,
    /* 0F 3A 3A */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 3B */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 3C */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 3D */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 3E */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 3F */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 40 */ &BxOpcodeTable0F3A40,
    /* 0F 3A 41 */ &BxOpcodeTable0F3A41,
    /* 0F 3A 42 */ &BxOpcodeTable0F3A42,
    /* 0F 3A 43 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 44 */ &BxOpcodeTable0F3A44,
    /* 0F 3A 45 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 46 */ &BxOpcodeTable0F3A46,
    /* 0F 3A 47 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 48 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 49 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 4A */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 4B */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 4C */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 4D */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 4E */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 4F */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 50 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 51 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 52 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 53 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 54 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 55 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 56 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 57 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 58 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 59 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 5A */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 5B */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 5C */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 5D */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 5E */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 5F */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 60 */ &BxOpcodeTable0F3A60,
    /* 0F 3A 61 */ &BxOpcodeTable0F3A61,
    /* 0F 3A 62 */ &BxOpcodeTable0F3A62,
    /* 0F 3A 64 */ &BxOpcodeTable0F3A63,
    /* 0F 3A 64 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 65 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 66 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 67 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 68 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 69 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 6A */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 6B */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 6C */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 6D */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 6E */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 6F */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 70 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 71 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 72 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 73 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 74 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 75 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 76 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 77 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 78 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 79 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 7A */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 7B */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 7C */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 7D */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 7E */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 7F */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 80 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 81 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 82 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 83 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 84 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 85 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 86 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 87 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 88 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 89 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 8A */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 8B */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 8C */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 8D */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 8E */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 8F */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 90 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 91 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 92 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 93 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 94 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 95 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 96 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 97 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 98 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 99 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 9A */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 9B */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 9C */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 9D */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 9E */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A 9F */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A A0 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A A1 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A A2 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A A3 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A A4 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A A5 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A A6 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A A7 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A A8 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A A9 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A AA */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A AB */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A AC */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A AD */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A AE */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A AF */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A B0 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A B1 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A B2 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A B3 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A B4 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A B5 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A B6 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A B7 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A B8 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A B9 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A BA */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A BB */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A BC */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A BD */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A BE */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A BF */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A C0 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A C1 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A C2 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A C3 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A C4 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A C5 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A C6 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A C7 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A C8 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A C9 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A CA */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A CB */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A CC */ &BxOpcodeTable0F3ACC,
    /* 0F 3A CD */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A CE */ &BxOpcodeTable0F3ACE,
    /* 0F 3A CF */ &BxOpcodeTable0F3ACF,
    /* 0F 3A D0 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A D1 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A D2 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A D3 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A D4 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A D5 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A D6 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A D7 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A D8 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A D9 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A DA */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A DB */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A DC */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A DD */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A DE */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A DF */ &BxOpcodeTable0F3ADF,
    /* 0F 3A E0 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A E1 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A E2 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A E3 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A E4 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A E5 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A E6 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A E7 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A E8 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A E9 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A EA */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A EB */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A EC */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A ED */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A EE */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A EF */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A F0 */ &BxOpcodeTable0F3AF0,
    /* 0F 3A F1 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A F2 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A F3 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A F4 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A F5 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A F6 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A F7 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A F8 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A F9 */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A FA */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A FB */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A FC */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A FD */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A FE */ &BX_OPCODE_GROUP_ERR,
    /* 0F 3A FF */ &BX_OPCODE_GROUP_ERR,
];
