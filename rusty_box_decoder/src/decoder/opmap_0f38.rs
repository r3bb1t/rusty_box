#![allow(non_upper_case_globals)]
#![allow(unused)] // TODO: don't forget to uncomment

use super::fetchdecode::*;
use super::fetchdecode_generated::*;
use super::ia_opcodes::Opcode;
/* ************************************************************************ */
/* 3-byte opcode table (Table A-4, 0F 38) */

// opcode 0F 38 00
pub(super) const BxOpcodeTable0F3800: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PshufbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PshufbVdqWdq),
];

// opcode 0F 38 01
pub const BxOpcodeTable0F3801: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PhaddwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PhaddwVdqWdq),
];

// opcode 0F 38 02
pub const BxOpcodeTable0F3802: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PhadddPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PhadddVdqWdq),
];

// opcode 0F 38 03
pub const BxOpcodeTable0F3803: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PhaddswPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PhaddswVdqWdq),
];

// opcode 0F 38 04
pub const BxOpcodeTable0F3804: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PmaddubswPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmaddubswVdqWdq),
];

// opcode 0F 38 05
pub const BxOpcodeTable0F3805: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PhsubwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PhsubwVdqWdq),
];

// opcode 0F 38 06
pub const BxOpcodeTable0F3806: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PhsubdPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PhsubdVdqWdq),
];

// opcode 0F 38 07
pub const BxOpcodeTable0F3807: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PhsubswPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PhsubswVdqWdq),
];

// opcode 0F 38 08
pub const BxOpcodeTable0F3808: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsignbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsignbVdqWdq),
];

// opcode 0F 38 09
pub const BxOpcodeTable0F3809: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsignwPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsignwVdqWdq),
];

// opcode 0F 38 0A
pub const BxOpcodeTable0F380A: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PsigndPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PsigndVdqWdq),
];

// opcode 0F 38 0B
pub const BxOpcodeTable0F380B: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PmulhrswPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmulhrswVdqWdq),
];

pub const BxOpcodeTable0F3810: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PblendvbVdqWdq)];
pub const BxOpcodeTable0F3814: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::BlendvpsVpsWps)];
pub const BxOpcodeTable0F3815: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::BlendvpdVpdWpd)];
pub const BxOpcodeTable0F3817: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PtestVdqWdq)];

// opcode 0F 38 1C
pub const BxOpcodeTable0F381C: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PabsbPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PabsbVdqWdq),
];

// opcode 0F 38 1D
pub const BxOpcodeTable0F381D: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PabswPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PabswVdqWdq),
];

// opcode 0F 38 1E
pub const BxOpcodeTable0F381E: [u64; 2] = [
    form_opcode(ATTR_SSE_NO_PREFIX, Opcode::PabsdPqQq),
    last_opcode(ATTR_SSE_PREFIX_66, Opcode::PabsdVdqWdq),
];

pub const BxOpcodeTable0F3820: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmovsxbwVdqWq)];
pub const BxOpcodeTable0F3821: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmovsxbdVdqWd)];
pub const BxOpcodeTable0F3822: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmovsxbqVdqWw)];
pub const BxOpcodeTable0F3823: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmovsxwdVdqWq)];
pub const BxOpcodeTable0F3824: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmovsxwqVdqWd)];
pub const BxOpcodeTable0F3825: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmovsxdqVdqWq)];
pub const BxOpcodeTable0F3828: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmuldqVdqWdq)];
pub const BxOpcodeTable0F3829: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpeqqVdqWdq)];
pub const BxOpcodeTable0F382A: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM,
    Opcode::MovntdqaVdqMdq,
)];
pub const BxOpcodeTable0F382B: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PackusdwVdqWdq)];
pub const BxOpcodeTable0F3830: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmovzxbwVdqWq)];
pub const BxOpcodeTable0F3831: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmovzxbdVdqWd)];
pub const BxOpcodeTable0F3832: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmovzxbqVdqWw)];
pub const BxOpcodeTable0F3833: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmovzxwdVdqWq)];
pub const BxOpcodeTable0F3834: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmovzxwqVdqWd)];
pub const BxOpcodeTable0F3835: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmovzxdqVdqWq)];
pub const BxOpcodeTable0F3837: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PcmpgtqVdqWdq)];
pub const BxOpcodeTable0F3838: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PminsbVdqWdq)];
pub const BxOpcodeTable0F3839: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PminsdVdqWdq)];
pub const BxOpcodeTable0F383A: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PminuwVdqWdq)];
pub const BxOpcodeTable0F383B: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PminudVdqWdq)];
pub const BxOpcodeTable0F383C: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmaxsbVdqWdq)];
pub const BxOpcodeTable0F383D: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmaxsdVdqWdq)];
pub const BxOpcodeTable0F383E: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmaxuwVdqWdq)];
pub const BxOpcodeTable0F383F: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmaxudVdqWdq)];
pub const BxOpcodeTable0F3840: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PmulldVdqWdq)];
pub const BxOpcodeTable0F3841: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::PhminposuwVdqWdq)];
pub const BxOpcodeTable0F3880: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM,
    Opcode::Invept,
)];
pub const BxOpcodeTable0F3881: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM,
    Opcode::Invvpid,
)];
pub const BxOpcodeTable0F3882: [u64; 1] = [last_opcode(
    ATTR_SSE_PREFIX_66 | ATTR_MOD_MEM,
    Opcode::Invpcid,
)];

pub const BxOpcodeTable0F388A: [u64; 1] = [last_opcode(
    ATTR_SSE_NO_PREFIX | ATTR_MOD_MEM | ATTR_IS64,
    Opcode::MovrsGbEb,
)];
pub const BxOpcodeTable0F388B: [u64; 3] = [
    form_opcode(
        ATTR_SSE_NO_PREFIX | ATTR_MOD_MEM | ATTR_OS64 | ATTR_IS64,
        Opcode::MovrsGqEq,
    ),
    form_opcode(
        ATTR_SSE_NO_PREFIX | ATTR_MOD_MEM | ATTR_OS32 | ATTR_IS64,
        Opcode::MovrsGdEd,
    ),
    last_opcode(
        ATTR_SSE_NO_PREFIX | ATTR_MOD_MEM | ATTR_OS16 | ATTR_IS64,
        Opcode::MovrsGwEw,
    ),
];

pub const BxOpcodeTable0F38C8: [u64; 1] =
    [last_opcode(ATTR_SSE_NO_PREFIX, Opcode::Sha1nexteVdqWdq)];
pub const BxOpcodeTable0F38C9: [u64; 1] = [last_opcode(ATTR_SSE_NO_PREFIX, Opcode::Sha1msg1VdqWdq)];
pub const BxOpcodeTable0F38CA: [u64; 1] = [last_opcode(ATTR_SSE_NO_PREFIX, Opcode::Sha1msg2VdqWdq)];
pub const BxOpcodeTable0F38CB: [u64; 1] =
    [last_opcode(ATTR_SSE_NO_PREFIX, Opcode::Sha256rnds2VdqWdq)];
pub const BxOpcodeTable0F38CC: [u64; 1] =
    [last_opcode(ATTR_SSE_NO_PREFIX, Opcode::Sha256msg1VdqWdq)];
pub const BxOpcodeTable0F38CD: [u64; 1] =
    [last_opcode(ATTR_SSE_NO_PREFIX, Opcode::Sha256msg2VdqWdq)];
pub const BxOpcodeTable0F38CF: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::Gf2p8mulbVdqWdq)];
pub const BxOpcodeTable0F38DB: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::AesimcVdqWdq)];
pub const BxOpcodeTable0F38DC: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::AesencVdqWdq)];
pub const BxOpcodeTable0F38DD: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::AesenclastVdqWdq)];
pub const BxOpcodeTable0F38DE: [u64; 1] = [last_opcode(ATTR_SSE_PREFIX_66, Opcode::AesdecVdqWdq)];
pub const BxOpcodeTable0F38DF: [u64; 1] =
    [last_opcode(ATTR_SSE_PREFIX_66, Opcode::AesdeclastVdqWdq)];

// opcode 0F 38 F0
pub const BxOpcodeTable0F38F0: [u64; 4] = [
    form_opcode(
        ATTR_NO_SSE_PREFIX_F2_F3 | ATTR_OS16 | ATTR_MOD_MEM,
        Opcode::MovbeGwMw,
    ),
    form_opcode(
        ATTR_NO_SSE_PREFIX_F2_F3 | ATTR_OS32 | ATTR_MOD_MEM,
        Opcode::MovbeGdMd,
    ),
    form_opcode(
        ATTR_NO_SSE_PREFIX_F2_F3 | ATTR_OS64 | ATTR_MOD_MEM,
        Opcode::MovbeGqMq,
    ),
    last_opcode(ATTR_SSE_PREFIX_F2, Opcode::Crc32GdEb),
];

// opcode 0F 38 F1
pub const BxOpcodeTable0F38F1: [u64; 6] = [
    form_opcode(
        ATTR_NO_SSE_PREFIX_F2_F3 | ATTR_OS64 | ATTR_MOD_MEM,
        Opcode::MovbeMqGq,
    ),
    form_opcode(
        ATTR_NO_SSE_PREFIX_F2_F3 | ATTR_OS32 | ATTR_MOD_MEM,
        Opcode::MovbeMdGd,
    ),
    form_opcode(
        ATTR_NO_SSE_PREFIX_F2_F3 | ATTR_OS16 | ATTR_MOD_MEM,
        Opcode::MovbeMwGw,
    ),
    form_opcode(ATTR_SSE_PREFIX_F2 | ATTR_OS64, Opcode::Crc32GdEq),
    form_opcode(ATTR_SSE_PREFIX_F2 | ATTR_OS32, Opcode::Crc32GdEd),
    last_opcode(ATTR_SSE_PREFIX_F2 | ATTR_OS16, Opcode::Crc32GdEw),
];

// opcode 0F 38 F6
pub const BxOpcodeTable0F38F5: [u64; 2] = [
    form_opcode(
        ATTR_OS64 | ATTR_MOD_MEM | ATTR_SSE_PREFIX_66,
        Opcode::Wrussq,
    ),
    last_opcode(
        ATTR_OS16_32 | ATTR_MOD_MEM | ATTR_SSE_PREFIX_66,
        Opcode::Wrussd,
    ),
];

// opcode 0F 38 F6
pub const BxOpcodeTable0F38F6: [u64; 6] = [
    form_opcode(ATTR_OS64 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX, Opcode::Wrssq),
    form_opcode(
        ATTR_OS16_32 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::Wrssd,
    ),
    form_opcode(ATTR_SSE_PREFIX_66 | ATTR_OS64, Opcode::AdcxGqEq),
    form_opcode(ATTR_SSE_PREFIX_F3 | ATTR_OS64, Opcode::AdoxGqEq),
    form_opcode(ATTR_SSE_PREFIX_66, Opcode::AdcxGdEd),
    last_opcode(ATTR_SSE_PREFIX_F3, Opcode::AdoxGdEd),
];

// opcode 0F 38 F8
pub const BxOpcodeTable0F38F8: [u64; 2] = [
    form_opcode(
        ATTR_OS64 | ATTR_MOD_MEM | ATTR_SSE_PREFIX_66,
        Opcode::Movdir64bGqMdq,
    ),
    last_opcode(ATTR_MOD_MEM | ATTR_SSE_PREFIX_66, Opcode::Movdir64bGdMdq),
];

// opcode 0F 38 F9
pub const BxOpcodeTable0F38F9: [u64; 2] = [
    form_opcode(
        ATTR_OS64 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::MovdiriMqGq,
    ),
    last_opcode(ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX, Opcode::MovdiriMdGd),
];

// opcode 0F 38 FC
pub const BxOpcodeTable0F38FC: [u64; 8] = [
    form_opcode(
        ATTR_OS64 | ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX,
        Opcode::AaddEqGq,
    ),
    form_opcode(
        ATTR_OS64 | ATTR_MOD_MEM | ATTR_SSE_PREFIX_66,
        Opcode::AandEqGq,
    ),
    form_opcode(
        ATTR_OS64 | ATTR_MOD_MEM | ATTR_SSE_PREFIX_F2,
        Opcode::AorEqGq,
    ),
    form_opcode(
        ATTR_OS64 | ATTR_MOD_MEM | ATTR_SSE_PREFIX_F3,
        Opcode::AxorEqGq,
    ),
    form_opcode(ATTR_MOD_MEM | ATTR_SSE_NO_PREFIX, Opcode::AaddEdGd),
    form_opcode(ATTR_MOD_MEM | ATTR_SSE_PREFIX_66, Opcode::AandEdGd),
    form_opcode(ATTR_MOD_MEM | ATTR_SSE_PREFIX_F2, Opcode::AorEdGd),
    last_opcode(ATTR_MOD_MEM | ATTR_SSE_PREFIX_F3, Opcode::AxorEdGd),
];

// /* ************************************************************************ */
//
pub(super) const BxOpcodeTable0F38: [&[u64]; 256] = [
    // 3-byte opcode 0x0F 0x38
    /* 0F 38 00 */ &BxOpcodeTable0F3800,
    /* 0F 38 01 */ &BxOpcodeTable0F3801,
    /* 0F 38 02 */ &BxOpcodeTable0F3802,
    /* 0F 38 03 */ &BxOpcodeTable0F3803,
    /* 0F 38 04 */ &BxOpcodeTable0F3804,
    /* 0F 38 05 */ &BxOpcodeTable0F3805,
    /* 0F 38 06 */ &BxOpcodeTable0F3806,
    /* 0F 38 07 */ &BxOpcodeTable0F3807,
    /* 0F 38 08 */ &BxOpcodeTable0F3808,
    /* 0F 38 09 */ &BxOpcodeTable0F3809,
    /* 0F 38 0A */ &BxOpcodeTable0F380A,
    /* 0F 38 0B */ &BxOpcodeTable0F380B,
    /* 0F 38 0C */ &BxOpcodeGroup_ERR,
    /* 0F 38 0D */ &BxOpcodeGroup_ERR,
    /* 0F 38 0E */ &BxOpcodeGroup_ERR,
    /* 0F 38 0F */ &BxOpcodeGroup_ERR,
    /* 0F 38 10 */ &BxOpcodeTable0F3810,
    /* 0F 38 11 */ &BxOpcodeGroup_ERR,
    /* 0F 38 12 */ &BxOpcodeGroup_ERR,
    /* 0F 38 13 */ &BxOpcodeGroup_ERR,
    /* 0F 38 14 */ &BxOpcodeTable0F3814,
    /* 0F 38 15 */ &BxOpcodeTable0F3815,
    /* 0F 38 16 */ &BxOpcodeGroup_ERR,
    /* 0F 38 17 */ &BxOpcodeTable0F3817,
    /* 0F 38 18 */ &BxOpcodeGroup_ERR,
    /* 0F 38 19 */ &BxOpcodeGroup_ERR,
    /* 0F 38 1A */ &BxOpcodeGroup_ERR,
    /* 0F 38 1B */ &BxOpcodeGroup_ERR,
    /* 0F 38 1C */ &BxOpcodeTable0F381C,
    /* 0F 38 1D */ &BxOpcodeTable0F381D,
    /* 0F 38 1E */ &BxOpcodeTable0F381E,
    /* 0F 38 1F */ &BxOpcodeGroup_ERR,
    /* 0F 38 20 */ &BxOpcodeTable0F3820,
    /* 0F 38 21 */ &BxOpcodeTable0F3821,
    /* 0F 38 22 */ &BxOpcodeTable0F3822,
    /* 0F 38 23 */ &BxOpcodeTable0F3823,
    /* 0F 38 24 */ &BxOpcodeTable0F3824,
    /* 0F 38 25 */ &BxOpcodeTable0F3825,
    /* 0F 38 26 */ &BxOpcodeGroup_ERR,
    /* 0F 38 27 */ &BxOpcodeGroup_ERR,
    /* 0F 38 28 */ &BxOpcodeTable0F3828,
    /* 0F 38 29 */ &BxOpcodeTable0F3829,
    /* 0F 38 2A */ &BxOpcodeTable0F382A,
    /* 0F 38 2B */ &BxOpcodeTable0F382B,
    /* 0F 38 2C */ &BxOpcodeGroup_ERR,
    /* 0F 38 2D */ &BxOpcodeGroup_ERR,
    /* 0F 38 2E */ &BxOpcodeGroup_ERR,
    /* 0F 38 2F */ &BxOpcodeGroup_ERR,
    /* 0F 38 30 */ &BxOpcodeTable0F3830,
    /* 0F 38 31 */ &BxOpcodeTable0F3831,
    /* 0F 38 32 */ &BxOpcodeTable0F3832,
    /* 0F 38 33 */ &BxOpcodeTable0F3833,
    /* 0F 38 34 */ &BxOpcodeTable0F3834,
    /* 0F 38 35 */ &BxOpcodeTable0F3835,
    /* 0F 38 36 */ &BxOpcodeGroup_ERR,
    /* 0F 38 37 */ &BxOpcodeTable0F3837,
    /* 0F 38 38 */ &BxOpcodeTable0F3838,
    /* 0F 38 39 */ &BxOpcodeTable0F3839,
    /* 0F 38 3A */ &BxOpcodeTable0F383A,
    /* 0F 38 3B */ &BxOpcodeTable0F383B,
    /* 0F 38 3C */ &BxOpcodeTable0F383C,
    /* 0F 38 3D */ &BxOpcodeTable0F383D,
    /* 0F 38 3E */ &BxOpcodeTable0F383E,
    /* 0F 38 3F */ &BxOpcodeTable0F383F,
    /* 0F 38 40 */ &BxOpcodeTable0F3840,
    /* 0F 38 41 */ &BxOpcodeTable0F3841,
    /* 0F 38 42 */ &BxOpcodeGroup_ERR,
    /* 0F 38 43 */ &BxOpcodeGroup_ERR,
    /* 0F 38 44 */ &BxOpcodeGroup_ERR,
    /* 0F 38 45 */ &BxOpcodeGroup_ERR,
    /* 0F 38 46 */ &BxOpcodeGroup_ERR,
    /* 0F 38 47 */ &BxOpcodeGroup_ERR,
    /* 0F 38 48 */ &BxOpcodeGroup_ERR,
    /* 0F 38 49 */ &BxOpcodeGroup_ERR,
    /* 0F 38 4A */ &BxOpcodeGroup_ERR,
    /* 0F 38 4B */ &BxOpcodeGroup_ERR,
    /* 0F 38 4C */ &BxOpcodeGroup_ERR,
    /* 0F 38 4D */ &BxOpcodeGroup_ERR,
    /* 0F 38 4E */ &BxOpcodeGroup_ERR,
    /* 0F 38 4F */ &BxOpcodeGroup_ERR,
    /* 0F 38 50 */ &BxOpcodeGroup_ERR,
    /* 0F 38 51 */ &BxOpcodeGroup_ERR,
    /* 0F 38 52 */ &BxOpcodeGroup_ERR,
    /* 0F 38 53 */ &BxOpcodeGroup_ERR,
    /* 0F 38 54 */ &BxOpcodeGroup_ERR,
    /* 0F 38 55 */ &BxOpcodeGroup_ERR,
    /* 0F 38 56 */ &BxOpcodeGroup_ERR,
    /* 0F 38 57 */ &BxOpcodeGroup_ERR,
    /* 0F 38 58 */ &BxOpcodeGroup_ERR,
    /* 0F 38 59 */ &BxOpcodeGroup_ERR,
    /* 0F 38 5A */ &BxOpcodeGroup_ERR,
    /* 0F 38 5B */ &BxOpcodeGroup_ERR,
    /* 0F 38 5C */ &BxOpcodeGroup_ERR,
    /* 0F 38 5D */ &BxOpcodeGroup_ERR,
    /* 0F 38 5E */ &BxOpcodeGroup_ERR,
    /* 0F 38 5F */ &BxOpcodeGroup_ERR,
    /* 0F 38 60 */ &BxOpcodeGroup_ERR,
    /* 0F 38 61 */ &BxOpcodeGroup_ERR,
    /* 0F 38 62 */ &BxOpcodeGroup_ERR,
    /* 0F 38 63 */ &BxOpcodeGroup_ERR,
    /* 0F 38 64 */ &BxOpcodeGroup_ERR,
    /* 0F 38 65 */ &BxOpcodeGroup_ERR,
    /* 0F 38 66 */ &BxOpcodeGroup_ERR,
    /* 0F 38 67 */ &BxOpcodeGroup_ERR,
    /* 0F 38 68 */ &BxOpcodeGroup_ERR,
    /* 0F 38 69 */ &BxOpcodeGroup_ERR,
    /* 0F 38 6A */ &BxOpcodeGroup_ERR,
    /* 0F 38 6B */ &BxOpcodeGroup_ERR,
    /* 0F 38 6C */ &BxOpcodeGroup_ERR,
    /* 0F 38 6D */ &BxOpcodeGroup_ERR,
    /* 0F 38 6E */ &BxOpcodeGroup_ERR,
    /* 0F 38 6F */ &BxOpcodeGroup_ERR,
    /* 0F 38 70 */ &BxOpcodeGroup_ERR,
    /* 0F 38 71 */ &BxOpcodeGroup_ERR,
    /* 0F 38 72 */ &BxOpcodeGroup_ERR,
    /* 0F 38 73 */ &BxOpcodeGroup_ERR,
    /* 0F 38 74 */ &BxOpcodeGroup_ERR,
    /* 0F 38 75 */ &BxOpcodeGroup_ERR,
    /* 0F 38 76 */ &BxOpcodeGroup_ERR,
    /* 0F 38 77 */ &BxOpcodeGroup_ERR,
    /* 0F 38 78 */ &BxOpcodeGroup_ERR,
    /* 0F 38 79 */ &BxOpcodeGroup_ERR,
    /* 0F 38 7A */ &BxOpcodeGroup_ERR,
    /* 0F 38 7B */ &BxOpcodeGroup_ERR,
    /* 0F 38 7C */ &BxOpcodeGroup_ERR,
    /* 0F 38 7D */ &BxOpcodeGroup_ERR,
    /* 0F 38 7E */ &BxOpcodeGroup_ERR,
    /* 0F 38 7F */ &BxOpcodeGroup_ERR,
    /* 0F 38 80 */ &BxOpcodeTable0F3880,
    /* 0F 38 81 */ &BxOpcodeTable0F3881,
    /* 0F 38 82 */ &BxOpcodeTable0F3882,
    /* 0F 38 83 */ &BxOpcodeGroup_ERR,
    /* 0F 38 84 */ &BxOpcodeGroup_ERR,
    /* 0F 38 85 */ &BxOpcodeGroup_ERR,
    /* 0F 38 86 */ &BxOpcodeGroup_ERR,
    /* 0F 38 87 */ &BxOpcodeGroup_ERR,
    /* 0F 38 88 */ &BxOpcodeGroup_ERR,
    /* 0F 38 89 */ &BxOpcodeGroup_ERR,
    /* 0F 38 8A */ &BxOpcodeTable0F388A,
    /* 0F 38 8B */ &BxOpcodeTable0F388B,
    /* 0F 38 8C */ &BxOpcodeGroup_ERR,
    /* 0F 38 8D */ &BxOpcodeGroup_ERR,
    /* 0F 38 8E */ &BxOpcodeGroup_ERR,
    /* 0F 38 8F */ &BxOpcodeGroup_ERR,
    /* 0F 38 90 */ &BxOpcodeGroup_ERR,
    /* 0F 38 91 */ &BxOpcodeGroup_ERR,
    /* 0F 38 92 */ &BxOpcodeGroup_ERR,
    /* 0F 38 93 */ &BxOpcodeGroup_ERR,
    /* 0F 38 94 */ &BxOpcodeGroup_ERR,
    /* 0F 38 95 */ &BxOpcodeGroup_ERR,
    /* 0F 38 96 */ &BxOpcodeGroup_ERR,
    /* 0F 38 97 */ &BxOpcodeGroup_ERR,
    /* 0F 38 98 */ &BxOpcodeGroup_ERR,
    /* 0F 38 99 */ &BxOpcodeGroup_ERR,
    /* 0F 38 9A */ &BxOpcodeGroup_ERR,
    /* 0F 38 9B */ &BxOpcodeGroup_ERR,
    /* 0F 38 9C */ &BxOpcodeGroup_ERR,
    /* 0F 38 9D */ &BxOpcodeGroup_ERR,
    /* 0F 38 9E */ &BxOpcodeGroup_ERR,
    /* 0F 38 9F */ &BxOpcodeGroup_ERR,
    /* 0F 38 A0 */ &BxOpcodeGroup_ERR,
    /* 0F 38 A1 */ &BxOpcodeGroup_ERR,
    /* 0F 38 A2 */ &BxOpcodeGroup_ERR,
    /* 0F 38 A3 */ &BxOpcodeGroup_ERR,
    /* 0F 38 A4 */ &BxOpcodeGroup_ERR,
    /* 0F 38 A5 */ &BxOpcodeGroup_ERR,
    /* 0F 38 A6 */ &BxOpcodeGroup_ERR,
    /* 0F 38 A7 */ &BxOpcodeGroup_ERR,
    /* 0F 38 A8 */ &BxOpcodeGroup_ERR,
    /* 0F 38 A9 */ &BxOpcodeGroup_ERR,
    /* 0F 38 AA */ &BxOpcodeGroup_ERR,
    /* 0F 38 AB */ &BxOpcodeGroup_ERR,
    /* 0F 38 AC */ &BxOpcodeGroup_ERR,
    /* 0F 38 AD */ &BxOpcodeGroup_ERR,
    /* 0F 38 AE */ &BxOpcodeGroup_ERR,
    /* 0F 38 AF */ &BxOpcodeGroup_ERR,
    /* 0F 38 B0 */ &BxOpcodeGroup_ERR,
    /* 0F 38 B1 */ &BxOpcodeGroup_ERR,
    /* 0F 38 B2 */ &BxOpcodeGroup_ERR,
    /* 0F 38 B3 */ &BxOpcodeGroup_ERR,
    /* 0F 38 B4 */ &BxOpcodeGroup_ERR,
    /* 0F 38 B5 */ &BxOpcodeGroup_ERR,
    /* 0F 38 B6 */ &BxOpcodeGroup_ERR,
    /* 0F 38 B7 */ &BxOpcodeGroup_ERR,
    /* 0F 38 B8 */ &BxOpcodeGroup_ERR,
    /* 0F 38 B9 */ &BxOpcodeGroup_ERR,
    /* 0F 38 BA */ &BxOpcodeGroup_ERR,
    /* 0F 38 BB */ &BxOpcodeGroup_ERR,
    /* 0F 38 BC */ &BxOpcodeGroup_ERR,
    /* 0F 38 BD */ &BxOpcodeGroup_ERR,
    /* 0F 38 BE */ &BxOpcodeGroup_ERR,
    /* 0F 38 BF */ &BxOpcodeGroup_ERR,
    /* 0F 38 C0 */ &BxOpcodeGroup_ERR,
    /* 0F 38 C1 */ &BxOpcodeGroup_ERR,
    /* 0F 38 C2 */ &BxOpcodeGroup_ERR,
    /* 0F 38 C3 */ &BxOpcodeGroup_ERR,
    /* 0F 38 C4 */ &BxOpcodeGroup_ERR,
    /* 0F 38 C5 */ &BxOpcodeGroup_ERR,
    /* 0F 38 C6 */ &BxOpcodeGroup_ERR,
    /* 0F 38 C7 */ &BxOpcodeGroup_ERR,
    /* 0F 38 C8 */ &BxOpcodeTable0F38C8,
    /* 0F 38 C9 */ &BxOpcodeTable0F38C9,
    /* 0F 38 CA */ &BxOpcodeTable0F38CA,
    /* 0F 38 CB */ &BxOpcodeTable0F38CB,
    /* 0F 38 CC */ &BxOpcodeTable0F38CC,
    /* 0F 38 CD */ &BxOpcodeTable0F38CD,
    /* 0F 38 CE */ &BxOpcodeGroup_ERR,
    /* 0F 38 CF */ &BxOpcodeTable0F38CF,
    /* 0F 38 D0 */ &BxOpcodeGroup_ERR,
    /* 0F 38 D1 */ &BxOpcodeGroup_ERR,
    /* 0F 38 D2 */ &BxOpcodeGroup_ERR,
    /* 0F 38 D3 */ &BxOpcodeGroup_ERR,
    /* 0F 38 D4 */ &BxOpcodeGroup_ERR,
    /* 0F 38 D5 */ &BxOpcodeGroup_ERR,
    /* 0F 38 D6 */ &BxOpcodeGroup_ERR,
    /* 0F 38 D7 */ &BxOpcodeGroup_ERR,
    /* 0F 38 D8 */ &BxOpcodeGroup_ERR,
    /* 0F 38 D9 */ &BxOpcodeGroup_ERR,
    /* 0F 38 DA */ &BxOpcodeGroup_ERR,
    /* 0F 38 DB */ &BxOpcodeTable0F38DB,
    /* 0F 38 DC */ &BxOpcodeTable0F38DC,
    /* 0F 38 DD */ &BxOpcodeTable0F38DD,
    /* 0F 38 DE */ &BxOpcodeTable0F38DE,
    /* 0F 38 DF */ &BxOpcodeTable0F38DF,
    /* 0F 38 E0 */ &BxOpcodeGroup_ERR,
    /* 0F 38 E1 */ &BxOpcodeGroup_ERR,
    /* 0F 38 E2 */ &BxOpcodeGroup_ERR,
    /* 0F 38 E3 */ &BxOpcodeGroup_ERR,
    /* 0F 38 E4 */ &BxOpcodeGroup_ERR,
    /* 0F 38 E5 */ &BxOpcodeGroup_ERR,
    /* 0F 38 E6 */ &BxOpcodeGroup_ERR,
    /* 0F 38 E7 */ &BxOpcodeGroup_ERR,
    /* 0F 38 E8 */ &BxOpcodeGroup_ERR,
    /* 0F 38 E9 */ &BxOpcodeGroup_ERR,
    /* 0F 38 EA */ &BxOpcodeGroup_ERR,
    /* 0F 38 EB */ &BxOpcodeGroup_ERR,
    /* 0F 38 EC */ &BxOpcodeGroup_ERR,
    /* 0F 38 ED */ &BxOpcodeGroup_ERR,
    /* 0F 38 EE */ &BxOpcodeGroup_ERR,
    /* 0F 38 EF */ &BxOpcodeGroup_ERR,
    /* 0F 38 F0 */ &BxOpcodeTable0F38F0,
    /* 0F 38 F1 */ &BxOpcodeTable0F38F1,
    /* 0F 38 F2 */ &BxOpcodeGroup_ERR,
    /* 0F 38 F3 */ &BxOpcodeGroup_ERR,
    /* 0F 38 F4 */ &BxOpcodeGroup_ERR,
    /* 0F 38 F5 */ &BxOpcodeTable0F38F5,
    /* 0F 38 F6 */ &BxOpcodeTable0F38F6,
    /* 0F 38 F7 */ &BxOpcodeGroup_ERR,
    /* 0F 38 F8 */ &BxOpcodeTable0F38F8,
    /* 0F 38 F9 */ &BxOpcodeTable0F38F9,
    /* 0F 38 FA */ &BxOpcodeGroup_ERR,
    /* 0F 38 FB */ &BxOpcodeGroup_ERR,
    /* 0F 38 FC */ &BxOpcodeTable0F38FC,
    /* 0F 38 FD */ &BxOpcodeGroup_ERR,
    /* 0F 38 FE */ &BxOpcodeGroup_ERR,
    /* 0F 38 FF */ &BxOpcodeGroup_ERR,
];
//
// #endif // BX_CPU_LEVEL >= 6
//
