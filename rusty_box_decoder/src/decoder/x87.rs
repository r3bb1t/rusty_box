//! x87 FPU and 3DNow! opcode tables (matching Bochs `fetchdecode_x87.h`).

use crate::opcode::Opcode;

pub(super) const BX3_DNOW_OPCODE: [Opcode; 256] = [
    // 256 entries for 3DNow opcodes, by suffix
    /* 00 */ Opcode::IaError,
    /* 01 */ Opcode::IaError,
    /* 02 */ Opcode::IaError,
    /* 03 */ Opcode::IaError,
    /* 04 */ Opcode::IaError,
    /* 05 */ Opcode::IaError,
    /* 06 */ Opcode::IaError,
    /* 07 */ Opcode::IaError,
    /* 08 */ Opcode::IaError,
    /* 09 */ Opcode::IaError,
    /* 0A */ Opcode::IaError,
    /* 0B */ Opcode::IaError,
    /* 0C */ Opcode::Pi2fwPqQq,
    /* 0D */ Opcode::Pi2fdPqQq,
    /* 0E */ Opcode::IaError,
    /* 0F */ Opcode::IaError,
    /* 10 */ Opcode::IaError,
    /* 11 */ Opcode::IaError,
    /* 12 */ Opcode::IaError,
    /* 13 */ Opcode::IaError,
    /* 14 */ Opcode::IaError,
    /* 15 */ Opcode::IaError,
    /* 16 */ Opcode::IaError,
    /* 17 */ Opcode::IaError,
    /* 18 */ Opcode::IaError,
    /* 19 */ Opcode::IaError,
    /* 1A */ Opcode::IaError,
    /* 1B */ Opcode::IaError,
    /* 1C */ Opcode::Pf2iwPqQq,
    /* 1D */ Opcode::Pf2idPqQq,
    /* 1E */ Opcode::IaError,
    /* 1F */ Opcode::IaError,
    /* 20 */ Opcode::IaError,
    /* 21 */ Opcode::IaError,
    /* 22 */ Opcode::IaError,
    /* 23 */ Opcode::IaError,
    /* 24 */ Opcode::IaError,
    /* 25 */ Opcode::IaError,
    /* 26 */ Opcode::IaError,
    /* 27 */ Opcode::IaError,
    /* 28 */ Opcode::IaError,
    /* 29 */ Opcode::IaError,
    /* 2A */ Opcode::IaError,
    /* 2B */ Opcode::IaError,
    /* 2C */ Opcode::IaError,
    /* 2D */ Opcode::IaError,
    /* 2E */ Opcode::IaError,
    /* 2F */ Opcode::IaError,
    /* 30 */ Opcode::IaError,
    /* 31 */ Opcode::IaError,
    /* 32 */ Opcode::IaError,
    /* 33 */ Opcode::IaError,
    /* 34 */ Opcode::IaError,
    /* 35 */ Opcode::IaError,
    /* 36 */ Opcode::IaError,
    /* 37 */ Opcode::IaError,
    /* 38 */ Opcode::IaError,
    /* 39 */ Opcode::IaError,
    /* 3A */ Opcode::IaError,
    /* 3B */ Opcode::IaError,
    /* 3C */ Opcode::IaError,
    /* 3D */ Opcode::IaError,
    /* 3E */ Opcode::IaError,
    /* 3F */ Opcode::IaError,
    /* 40 */ Opcode::IaError,
    /* 41 */ Opcode::IaError,
    /* 42 */ Opcode::IaError,
    /* 43 */ Opcode::IaError,
    /* 44 */ Opcode::IaError,
    /* 45 */ Opcode::IaError,
    /* 46 */ Opcode::IaError,
    /* 47 */ Opcode::IaError,
    /* 48 */ Opcode::IaError,
    /* 49 */ Opcode::IaError,
    /* 4A */ Opcode::IaError,
    /* 4B */ Opcode::IaError,
    /* 4C */ Opcode::IaError,
    /* 4D */ Opcode::IaError,
    /* 4E */ Opcode::IaError,
    /* 4F */ Opcode::IaError,
    /* 50 */ Opcode::IaError,
    /* 51 */ Opcode::IaError,
    /* 52 */ Opcode::IaError,
    /* 53 */ Opcode::IaError,
    /* 54 */ Opcode::IaError,
    /* 55 */ Opcode::IaError,
    /* 56 */ Opcode::IaError,
    /* 57 */ Opcode::IaError,
    /* 58 */ Opcode::IaError,
    /* 59 */ Opcode::IaError,
    /* 5A */ Opcode::IaError,
    /* 5B */ Opcode::IaError,
    /* 5C */ Opcode::IaError,
    /* 5D */ Opcode::IaError,
    /* 5E */ Opcode::IaError,
    /* 5F */ Opcode::IaError,
    /* 60 */ Opcode::IaError,
    /* 61 */ Opcode::IaError,
    /* 62 */ Opcode::IaError,
    /* 63 */ Opcode::IaError,
    /* 64 */ Opcode::IaError,
    /* 65 */ Opcode::IaError,
    /* 66 */ Opcode::IaError,
    /* 67 */ Opcode::IaError,
    /* 68 */ Opcode::IaError,
    /* 69 */ Opcode::IaError,
    /* 6A */ Opcode::IaError,
    /* 6B */ Opcode::IaError,
    /* 6C */ Opcode::IaError,
    /* 6D */ Opcode::IaError,
    /* 6E */ Opcode::IaError,
    /* 6F */ Opcode::IaError,
    /* 70 */ Opcode::IaError,
    /* 71 */ Opcode::IaError,
    /* 72 */ Opcode::IaError,
    /* 73 */ Opcode::IaError,
    /* 74 */ Opcode::IaError,
    /* 75 */ Opcode::IaError,
    /* 76 */ Opcode::IaError,
    /* 77 */ Opcode::IaError,
    /* 78 */ Opcode::IaError,
    /* 79 */ Opcode::IaError,
    /* 7A */ Opcode::IaError,
    /* 7B */ Opcode::IaError,
    /* 7C */ Opcode::IaError,
    /* 7D */ Opcode::IaError,
    /* 7E */ Opcode::IaError,
    /* 7F */ Opcode::IaError,
    /* 80 */ Opcode::IaError,
    /* 81 */ Opcode::IaError,
    /* 82 */ Opcode::IaError,
    /* 83 */ Opcode::IaError,
    /* 84 */ Opcode::IaError,
    /* 85 */ Opcode::IaError,
    /* 86 */ Opcode::IaError,
    /* 87 */ Opcode::IaError,
    /* 88 */ Opcode::IaError,
    /* 89 */ Opcode::IaError,
    /* 8A */ Opcode::PfnaccPqQq,
    /* 8B */ Opcode::IaError,
    /* 8C */ Opcode::IaError,
    /* 8D */ Opcode::IaError,
    /* 8E */ Opcode::PfpnaccPqQq,
    /* 8F */ Opcode::IaError,
    /* 90 */ Opcode::PfcmpgePqQq,
    /* 91 */ Opcode::IaError,
    /* 92 */ Opcode::IaError,
    /* 93 */ Opcode::IaError,
    /* 94 */ Opcode::PfminPqQq,
    /* 95 */ Opcode::IaError,
    /* 96 */ Opcode::PfrcpPqQq,
    /* 97 */ Opcode::PfrsqrtPqQq,
    /* 98 */ Opcode::IaError,
    /* 99 */ Opcode::IaError,
    /* 9A */ Opcode::PfsubPqQq,
    /* 9B */ Opcode::IaError,
    /* 9C */ Opcode::IaError,
    /* 9D */ Opcode::IaError,
    /* 9E */ Opcode::PfaddPqQq,
    /* 9F */ Opcode::IaError,
    /* A0 */ Opcode::PfcmpgtPqQq,
    /* A1 */ Opcode::IaError,
    /* A2 */ Opcode::IaError,
    /* A3 */ Opcode::IaError,
    /* A4 */ Opcode::PfmaxPqQq,
    /* A5 */ Opcode::IaError,
    /* A6 */ Opcode::Pfrcpit1PqQq,
    /* A7 */ Opcode::Pfrsqit1PqQq,
    /* A8 */ Opcode::IaError,
    /* A9 */ Opcode::IaError,
    /* AA */ Opcode::PfsubrPqQq,
    /* AB */ Opcode::IaError,
    /* AC */ Opcode::IaError,
    /* AD */ Opcode::IaError,
    /* AE */ Opcode::PfaccPqQq,
    /* AF */ Opcode::IaError,
    /* B0 */ Opcode::PfcmpeqPqQq,
    /* B1 */ Opcode::IaError,
    /* B2 */ Opcode::IaError,
    /* B3 */ Opcode::IaError,
    /* B4 */ Opcode::PfmulPqQq,
    /* B5 */ Opcode::IaError,
    /* B6 */ Opcode::Pfrcpit2PqQq,
    /* B7 */ Opcode::PmulhrwPqQq,
    /* B8 */ Opcode::IaError,
    /* B9 */ Opcode::IaError,
    /* BA */ Opcode::IaError,
    /* BB */ Opcode::PswapdPqQq,
    /* BC */ Opcode::IaError,
    /* BD */ Opcode::IaError,
    /* BE */ Opcode::IaError,
    /* BF */ Opcode::PavgbPqQq,
    /* C0 */ Opcode::IaError,
    /* C1 */ Opcode::IaError,
    /* C2 */ Opcode::IaError,
    /* C3 */ Opcode::IaError,
    /* C4 */ Opcode::IaError,
    /* C5 */ Opcode::IaError,
    /* C6 */ Opcode::IaError,
    /* C7 */ Opcode::IaError,
    /* C8 */ Opcode::IaError,
    /* C9 */ Opcode::IaError,
    /* CA */ Opcode::IaError,
    /* CB */ Opcode::IaError,
    /* CC */ Opcode::IaError,
    /* CD */ Opcode::IaError,
    /* CE */ Opcode::IaError,
    /* CF */ Opcode::IaError,
    /* D0 */ Opcode::IaError,
    /* D1 */ Opcode::IaError,
    /* D2 */ Opcode::IaError,
    /* D3 */ Opcode::IaError,
    /* D4 */ Opcode::IaError,
    /* D5 */ Opcode::IaError,
    /* D6 */ Opcode::IaError,
    /* D7 */ Opcode::IaError,
    /* D8 */ Opcode::IaError,
    /* D9 */ Opcode::IaError,
    /* DA */ Opcode::IaError,
    /* DB */ Opcode::IaError,
    /* DC */ Opcode::IaError,
    /* DD */ Opcode::IaError,
    /* DE */ Opcode::IaError,
    /* DF */ Opcode::IaError,
    /* E0 */ Opcode::IaError,
    /* E1 */ Opcode::IaError,
    /* E2 */ Opcode::IaError,
    /* E3 */ Opcode::IaError,
    /* E4 */ Opcode::IaError,
    /* E5 */ Opcode::IaError,
    /* E6 */ Opcode::IaError,
    /* E7 */ Opcode::IaError,
    /* E8 */ Opcode::IaError,
    /* E9 */ Opcode::IaError,
    /* EA */ Opcode::IaError,
    /* EB */ Opcode::IaError,
    /* EC */ Opcode::IaError,
    /* ED */ Opcode::IaError,
    /* EE */ Opcode::IaError,
    /* EF */ Opcode::IaError,
    /* F0 */ Opcode::IaError,
    /* F1 */ Opcode::IaError,
    /* F2 */ Opcode::IaError,
    /* F3 */ Opcode::IaError,
    /* F4 */ Opcode::IaError,
    /* F5 */ Opcode::IaError,
    /* F6 */ Opcode::IaError,
    /* F7 */ Opcode::IaError,
    /* F8 */ Opcode::IaError,
    /* F9 */ Opcode::IaError,
    /* FA */ Opcode::IaError,
    /* FB */ Opcode::IaError,
    /* FC */ Opcode::IaError,
    /* FD */ Opcode::IaError,
    /* FE */ Opcode::IaError,
    /* FF */ Opcode::IaError,
];

// x87 Floating Point opcode tables
// Based on cpp_orig/bochs/cpu/decoder/fetchdecode_x87.h

/// D8 opcode table (64+8 entries = 72 total)
/// /m form: 0-7 (8 entries)
/// /r form: 8-71 (64 entries, D8 C0-FF)
pub(super) const BX_OPCODE_INFO_FLOATING_POINT_D8: [Opcode; 72] = [
    // /m form (0-7)
    Opcode::FaddSingleReal,  // 0
    Opcode::FmulSingleReal,  // 1
    Opcode::FcomSingleReal,  // 2
    Opcode::FcompSingleReal, // 3
    Opcode::FsubSingleReal,  // 4
    Opcode::FsubrSingleReal, // 5
    Opcode::FdivSingleReal,  // 6
    Opcode::FdivrSingleReal, // 7
    // /r form (8-71, D8 C0-FF, 64 entries)
    Opcode::FaddSt0Stj,
    Opcode::FaddSt0Stj,
    Opcode::FaddSt0Stj,
    Opcode::FaddSt0Stj,
    Opcode::FaddSt0Stj,
    Opcode::FaddSt0Stj,
    Opcode::FaddSt0Stj,
    Opcode::FaddSt0Stj, // D8 C0-C7
    Opcode::FmulSt0Stj,
    Opcode::FmulSt0Stj,
    Opcode::FmulSt0Stj,
    Opcode::FmulSt0Stj,
    Opcode::FmulSt0Stj,
    Opcode::FmulSt0Stj,
    Opcode::FmulSt0Stj,
    Opcode::FmulSt0Stj, // D8 C8-CF
    Opcode::FcomSti,
    Opcode::FcomSti,
    Opcode::FcomSti,
    Opcode::FcomSti,
    Opcode::FcomSti,
    Opcode::FcomSti,
    Opcode::FcomSti,
    Opcode::FcomSti, // D8 D0-D7
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti, // D8 D8-DF
    Opcode::FsubSt0Stj,
    Opcode::FsubSt0Stj,
    Opcode::FsubSt0Stj,
    Opcode::FsubSt0Stj,
    Opcode::FsubSt0Stj,
    Opcode::FsubSt0Stj,
    Opcode::FsubSt0Stj,
    Opcode::FsubSt0Stj, // D8 E0-E7
    Opcode::FsubrSt0Stj,
    Opcode::FsubrSt0Stj,
    Opcode::FsubrSt0Stj,
    Opcode::FsubrSt0Stj,
    Opcode::FsubrSt0Stj,
    Opcode::FsubrSt0Stj,
    Opcode::FsubrSt0Stj,
    Opcode::FsubrSt0Stj, // D8 E8-EF
    Opcode::FdivSt0Stj,
    Opcode::FdivSt0Stj,
    Opcode::FdivSt0Stj,
    Opcode::FdivSt0Stj,
    Opcode::FdivSt0Stj,
    Opcode::FdivSt0Stj,
    Opcode::FdivSt0Stj,
    Opcode::FdivSt0Stj, // D8 F0-F7
    Opcode::FdivrSt0Stj,
    Opcode::FdivrSt0Stj,
    Opcode::FdivrSt0Stj,
    Opcode::FdivrSt0Stj,
    Opcode::FdivrSt0Stj,
    Opcode::FdivrSt0Stj,
    Opcode::FdivrSt0Stj,
    Opcode::FdivrSt0Stj, // D8 F8-FF
];

/// D9 opcode table — Matching Bochs fetchdecode_x87.h lines 133-213
pub(super) const BX_OPCODE_INFO_FLOATING_POINT_D9: [Opcode; 72] = [
    // /m form (0-7): memory operand addressed by ModRM
    Opcode::FldSingleReal,  // /0: FLD single-real
    Opcode::IaError,        // /1: (reserved)
    Opcode::FstSingleReal,  // /2: FST single-real
    Opcode::FstpSingleReal, // /3: FSTP single-real
    Opcode::Fldenv,         // /4: FLDENV
    Opcode::Fldcw,          // /5: FLDCW
    Opcode::Fnstenv,        // /6: FNSTENV
    Opcode::Fnstcw,         // /7: FNSTCW
    // /r form (8-71): D9 C0-FF register forms
    Opcode::FldSti,
    Opcode::FldSti,
    Opcode::FldSti,
    Opcode::FldSti,
    Opcode::FldSti,
    Opcode::FldSti,
    Opcode::FldSti,
    Opcode::FldSti, // D9 C0-C7: FLD ST(i)
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti, // D9 C8-CF: FXCH ST(i)
    Opcode::Fnop,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError, // D9 D0-D7: FNOP, errors
    Opcode::FstpSpecialSti,
    Opcode::FstpSpecialSti,
    Opcode::FstpSpecialSti,
    Opcode::FstpSpecialSti,
    Opcode::FstpSpecialSti,
    Opcode::FstpSpecialSti,
    Opcode::FstpSpecialSti,
    Opcode::FstpSpecialSti, // D9 D8-DF: FSTP_SPECIAL (undocumented)
    Opcode::Fchs,
    Opcode::Fabs,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::Ftst,
    Opcode::Fxam,
    Opcode::IaError,
    Opcode::IaError, // D9 E0-E7: FCHS, FABS, -, -, FTST, FXAM, -, -
    Opcode::FLD1,
    Opcode::Fldl2t,
    Opcode::Fldl2e,
    Opcode::Fldpi,
    Opcode::Fldlg2,
    Opcode::Fldln2,
    Opcode::Fldz,
    Opcode::IaError, // D9 E8-EF: FLD1, FLDL2T, FLDL2E, FLDPI, FLDLG2, FLDLN2, FLDZ, -
    Opcode::F2XM1,
    Opcode::FYL2X,
    Opcode::Fptan,
    Opcode::Fpatan,
    Opcode::Fxtract,
    Opcode::FPREM1,
    Opcode::Fdecstp,
    Opcode::Fincstp, // D9 F0-F7
    Opcode::Fprem,
    Opcode::FYL2XP1,
    Opcode::Fsqrt,
    Opcode::Fsincos,
    Opcode::Frndint,
    Opcode::Fscale,
    Opcode::Fsin,
    Opcode::Fcos, // D9 F8-FF
];

/// DA opcode table — Matching Bochs fetchdecode_x87.h lines 214-290
pub(super) const BX_OPCODE_INFO_FLOATING_POINT_DA: [Opcode; 72] = [
    // /m form (0-7): 32-bit integer memory operand
    Opcode::FiaddDwordInteger,  // /0: FIADD dword
    Opcode::FimulDwordInteger,  // /1: FIMUL dword
    Opcode::FicomDwordInteger,  // /2: FICOM dword
    Opcode::FicompDwordInteger, // /3: FICOMP dword
    Opcode::FisubDwordInteger,  // /4: FISUB dword
    Opcode::FisubrDwordInteger, // /5: FISUBR dword
    Opcode::FidivDwordInteger,  // /6: FIDIV dword
    Opcode::FidivrDwordInteger, // /7: FIDIVR dword
    // /r form (8-71): DA C0-FF register forms
    Opcode::FcmovbSt0Stj,
    Opcode::FcmovbSt0Stj,
    Opcode::FcmovbSt0Stj,
    Opcode::FcmovbSt0Stj,
    Opcode::FcmovbSt0Stj,
    Opcode::FcmovbSt0Stj,
    Opcode::FcmovbSt0Stj,
    Opcode::FcmovbSt0Stj, // DA C0-C7: FCMOVB
    Opcode::FcmoveSt0Stj,
    Opcode::FcmoveSt0Stj,
    Opcode::FcmoveSt0Stj,
    Opcode::FcmoveSt0Stj,
    Opcode::FcmoveSt0Stj,
    Opcode::FcmoveSt0Stj,
    Opcode::FcmoveSt0Stj,
    Opcode::FcmoveSt0Stj, // DA C8-CF: FCMOVE
    Opcode::FcmovbeSt0Stj,
    Opcode::FcmovbeSt0Stj,
    Opcode::FcmovbeSt0Stj,
    Opcode::FcmovbeSt0Stj,
    Opcode::FcmovbeSt0Stj,
    Opcode::FcmovbeSt0Stj,
    Opcode::FcmovbeSt0Stj,
    Opcode::FcmovbeSt0Stj, // DA D0-D7: FCMOVBE
    Opcode::FcmovuSt0Stj,
    Opcode::FcmovuSt0Stj,
    Opcode::FcmovuSt0Stj,
    Opcode::FcmovuSt0Stj,
    Opcode::FcmovuSt0Stj,
    Opcode::FcmovuSt0Stj,
    Opcode::FcmovuSt0Stj,
    Opcode::FcmovuSt0Stj, // DA D8-DF: FCMOVU
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError, // DA E0-E7: (reserved)
    Opcode::IaError,
    Opcode::Fucompp,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError, // DA E8-EF: -, FUCOMPP, -...
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError, // DA F0-F7
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError, // DA F8-FF
];

/// DB opcode table — Matching Bochs fetchdecode_x87.h lines 291-371
pub(super) const BX_OPCODE_INFO_FLOATING_POINT_DB: [Opcode; 72] = [
    // /m form (0-7)
    Opcode::FildDwordInteger,  // /0: FILD dword
    Opcode::FisttpMd,          // /1: FISTTP dword (SSE3)
    Opcode::FistDwordInteger,  // /2: FIST dword
    Opcode::FistpDwordInteger, // /3: FISTP dword
    Opcode::IaError,           // /4: (reserved)
    Opcode::FldExtendedReal,   // /5: FLD extended-real (80-bit)
    Opcode::IaError,           // /6: (reserved)
    Opcode::FstpExtendedReal,  // /7: FSTP extended-real (80-bit)
    // /r form (8-71): DB C0-FF register forms
    Opcode::FcmovnbSt0Stj,
    Opcode::FcmovnbSt0Stj,
    Opcode::FcmovnbSt0Stj,
    Opcode::FcmovnbSt0Stj,
    Opcode::FcmovnbSt0Stj,
    Opcode::FcmovnbSt0Stj,
    Opcode::FcmovnbSt0Stj,
    Opcode::FcmovnbSt0Stj, // DB C0-C7: FCMOVNB
    Opcode::FcmovneSt0Stj,
    Opcode::FcmovneSt0Stj,
    Opcode::FcmovneSt0Stj,
    Opcode::FcmovneSt0Stj,
    Opcode::FcmovneSt0Stj,
    Opcode::FcmovneSt0Stj,
    Opcode::FcmovneSt0Stj,
    Opcode::FcmovneSt0Stj, // DB C8-CF: FCMOVNE
    Opcode::FcmovnbeSt0Stj,
    Opcode::FcmovnbeSt0Stj,
    Opcode::FcmovnbeSt0Stj,
    Opcode::FcmovnbeSt0Stj,
    Opcode::FcmovnbeSt0Stj,
    Opcode::FcmovnbeSt0Stj,
    Opcode::FcmovnbeSt0Stj,
    Opcode::FcmovnbeSt0Stj, // DB D0-D7: FCMOVNBE
    Opcode::FcmovnuSt0Stj,
    Opcode::FcmovnuSt0Stj,
    Opcode::FcmovnuSt0Stj,
    Opcode::FcmovnuSt0Stj,
    Opcode::FcmovnuSt0Stj,
    Opcode::FcmovnuSt0Stj,
    Opcode::FcmovnuSt0Stj,
    Opcode::FcmovnuSt0Stj, // DB D8-DF: FCMOVNU
    Opcode::Fplegacy,
    Opcode::Fplegacy,
    Opcode::Fnclex,
    Opcode::Fninit,
    Opcode::Fplegacy,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError, // DB E0-E7: FENI,FDISI,FNCLEX,FNINIT,FSETPM,-,-,-
    Opcode::FucomiSt0Stj,
    Opcode::FucomiSt0Stj,
    Opcode::FucomiSt0Stj,
    Opcode::FucomiSt0Stj,
    Opcode::FucomiSt0Stj,
    Opcode::FucomiSt0Stj,
    Opcode::FucomiSt0Stj,
    Opcode::FucomiSt0Stj, // DB E8-EF: FUCOMI
    Opcode::FcomiSt0Stj,
    Opcode::FcomiSt0Stj,
    Opcode::FcomiSt0Stj,
    Opcode::FcomiSt0Stj,
    Opcode::FcomiSt0Stj,
    Opcode::FcomiSt0Stj,
    Opcode::FcomiSt0Stj,
    Opcode::FcomiSt0Stj, // DB F0-F7: FCOMI
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError, // DB F8-FF: (reserved)
];

/// DC opcode table — Matching Bochs fetchdecode_x87.h lines 372-448
pub(super) const BX_OPCODE_INFO_FLOATING_POINT_DC: [Opcode; 72] = [
    // /m form (0-7): double-real (64-bit) memory operand
    Opcode::FaddDoubleReal,  // /0: FADD double-real
    Opcode::FmulDoubleReal,  // /1: FMUL double-real
    Opcode::FcomDoubleReal,  // /2: FCOM double-real
    Opcode::FcompDoubleReal, // /3: FCOMP double-real
    Opcode::FsubDoubleReal,  // /4: FSUB double-real
    Opcode::FsubrDoubleReal, // /5: FSUBR double-real
    Opcode::FdivDoubleReal,  // /6: FDIV double-real
    Opcode::FdivrDoubleReal, // /7: FDIVR double-real
    // /r form (8-71): DC C0-FF register forms
    Opcode::FaddStiSt0,
    Opcode::FaddStiSt0,
    Opcode::FaddStiSt0,
    Opcode::FaddStiSt0,
    Opcode::FaddStiSt0,
    Opcode::FaddStiSt0,
    Opcode::FaddStiSt0,
    Opcode::FaddStiSt0, // DC C0-C7: FADD ST(i),ST(0)
    Opcode::FmulStiSt0,
    Opcode::FmulStiSt0,
    Opcode::FmulStiSt0,
    Opcode::FmulStiSt0,
    Opcode::FmulStiSt0,
    Opcode::FmulStiSt0,
    Opcode::FmulStiSt0,
    Opcode::FmulStiSt0, // DC C8-CF: FMUL ST(i),ST(0)
    Opcode::FcomSti,
    Opcode::FcomSti,
    Opcode::FcomSti,
    Opcode::FcomSti,
    Opcode::FcomSti,
    Opcode::FcomSti,
    Opcode::FcomSti,
    Opcode::FcomSti, // DC D0-D7: FCOM (undocumented)
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti, // DC D8-DF: FCOMP (undocumented)
    Opcode::FsubrStiSt0,
    Opcode::FsubrStiSt0,
    Opcode::FsubrStiSt0,
    Opcode::FsubrStiSt0,
    Opcode::FsubrStiSt0,
    Opcode::FsubrStiSt0,
    Opcode::FsubrStiSt0,
    Opcode::FsubrStiSt0, // DC E0-E7: FSUBR ST(i),ST(0)
    Opcode::FsubStiSt0,
    Opcode::FsubStiSt0,
    Opcode::FsubStiSt0,
    Opcode::FsubStiSt0,
    Opcode::FsubStiSt0,
    Opcode::FsubStiSt0,
    Opcode::FsubStiSt0,
    Opcode::FsubStiSt0, // DC E8-EF: FSUB ST(i),ST(0)
    Opcode::FdivrStiSt0,
    Opcode::FdivrStiSt0,
    Opcode::FdivrStiSt0,
    Opcode::FdivrStiSt0,
    Opcode::FdivrStiSt0,
    Opcode::FdivrStiSt0,
    Opcode::FdivrStiSt0,
    Opcode::FdivrStiSt0, // DC F0-F7: FDIVR ST(i),ST(0)
    Opcode::FdivStiSt0,
    Opcode::FdivStiSt0,
    Opcode::FdivStiSt0,
    Opcode::FdivStiSt0,
    Opcode::FdivStiSt0,
    Opcode::FdivStiSt0,
    Opcode::FdivStiSt0,
    Opcode::FdivStiSt0, // DC F8-FF: FDIV ST(i),ST(0)
];

/// DD opcode table — Matching Bochs fetchdecode_x87.h lines 449-529
pub(super) const BX_OPCODE_INFO_FLOATING_POINT_DD: [Opcode; 72] = [
    // /m form (0-7)
    Opcode::FldDoubleReal,  // /0: FLD double-real
    Opcode::FisttpMq,       // /1: FISTTP qword (SSE3)
    Opcode::FstDoubleReal,  // /2: FST double-real
    Opcode::FstpDoubleReal, // /3: FSTP double-real
    Opcode::Frstor,         // /4: FRSTOR
    Opcode::IaError,        // /5: (reserved)
    Opcode::Fnsave,         // /6: FNSAVE
    Opcode::Fnstsw,         // /7: FNSTSW (memory form)
    // /r form (8-71): DD C0-FF register forms
    Opcode::FfreeSti,
    Opcode::FfreeSti,
    Opcode::FfreeSti,
    Opcode::FfreeSti,
    Opcode::FfreeSti,
    Opcode::FfreeSti,
    Opcode::FfreeSti,
    Opcode::FfreeSti, // DD C0-C7: FFREE ST(i)
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti, // DD C8-CF: FXCH (undocumented)
    Opcode::FstSti,
    Opcode::FstSti,
    Opcode::FstSti,
    Opcode::FstSti,
    Opcode::FstSti,
    Opcode::FstSti,
    Opcode::FstSti,
    Opcode::FstSti, // DD D0-D7: FST ST(i)
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti, // DD D8-DF: FSTP ST(i)
    Opcode::FucomSti,
    Opcode::FucomSti,
    Opcode::FucomSti,
    Opcode::FucomSti,
    Opcode::FucomSti,
    Opcode::FucomSti,
    Opcode::FucomSti,
    Opcode::FucomSti, // DD E0-E7: FUCOM ST(i)
    Opcode::FucompSti,
    Opcode::FucompSti,
    Opcode::FucompSti,
    Opcode::FucompSti,
    Opcode::FucompSti,
    Opcode::FucompSti,
    Opcode::FucompSti,
    Opcode::FucompSti, // DD E8-EF: FUCOMP ST(i)
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError, // DD F0-F7
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError, // DD F8-FF
];

/// DE opcode table — Matching Bochs fetchdecode_x87.h lines 530-606
pub(super) const BX_OPCODE_INFO_FLOATING_POINT_DE: [Opcode; 72] = [
    // /m form (0-7): 16-bit integer memory operand
    Opcode::FiaddWordInteger,  // /0: FIADD word
    Opcode::FimulWordInteger,  // /1: FIMUL word
    Opcode::FicomWordInteger,  // /2: FICOM word
    Opcode::FicompWordInteger, // /3: FICOMP word
    Opcode::FisubWordInteger,  // /4: FISUB word
    Opcode::FisubrWordInteger, // /5: FISUBR word
    Opcode::FidivWordInteger,  // /6: FIDIV word
    Opcode::FidivrWordInteger, // /7: FIDIVR word
    // /r form (8-71): DE C0-FF register forms
    Opcode::FaddpStiSt0,
    Opcode::FaddpStiSt0,
    Opcode::FaddpStiSt0,
    Opcode::FaddpStiSt0,
    Opcode::FaddpStiSt0,
    Opcode::FaddpStiSt0,
    Opcode::FaddpStiSt0,
    Opcode::FaddpStiSt0, // DE C0-C7: FADDP
    Opcode::FmulpStiSt0,
    Opcode::FmulpStiSt0,
    Opcode::FmulpStiSt0,
    Opcode::FmulpStiSt0,
    Opcode::FmulpStiSt0,
    Opcode::FmulpStiSt0,
    Opcode::FmulpStiSt0,
    Opcode::FmulpStiSt0, // DE C8-CF: FMULP
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti,
    Opcode::FcompSti, // DE D0-D7: FCOMP (undocumented)
    Opcode::IaError,
    Opcode::Fcompp,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError, // DE D8-DF: -, FCOMPP, -...
    Opcode::FsubrpStiSt0,
    Opcode::FsubrpStiSt0,
    Opcode::FsubrpStiSt0,
    Opcode::FsubrpStiSt0,
    Opcode::FsubrpStiSt0,
    Opcode::FsubrpStiSt0,
    Opcode::FsubrpStiSt0,
    Opcode::FsubrpStiSt0, // DE E0-E7: FSUBRP
    Opcode::FsubpStiSt0,
    Opcode::FsubpStiSt0,
    Opcode::FsubpStiSt0,
    Opcode::FsubpStiSt0,
    Opcode::FsubpStiSt0,
    Opcode::FsubpStiSt0,
    Opcode::FsubpStiSt0,
    Opcode::FsubpStiSt0, // DE E8-EF: FSUBP
    Opcode::FdivrpStiSt0,
    Opcode::FdivrpStiSt0,
    Opcode::FdivrpStiSt0,
    Opcode::FdivrpStiSt0,
    Opcode::FdivrpStiSt0,
    Opcode::FdivrpStiSt0,
    Opcode::FdivrpStiSt0,
    Opcode::FdivrpStiSt0, // DE F0-F7: FDIVRP
    Opcode::FdivpStiSt0,
    Opcode::FdivpStiSt0,
    Opcode::FdivpStiSt0,
    Opcode::FdivpStiSt0,
    Opcode::FdivpStiSt0,
    Opcode::FdivpStiSt0,
    Opcode::FdivpStiSt0,
    Opcode::FdivpStiSt0, // DE F8-FF: FDIVP
];

/// DF opcode table — Matching Bochs fetchdecode_x87.h lines 607-687
pub(super) const BX_OPCODE_INFO_FLOATING_POINT_DF: [Opcode; 72] = [
    // /m form (0-7)
    Opcode::FildWordInteger,   // /0: FILD word
    Opcode::FisttpMw,          // /1: FISTTP word (SSE3)
    Opcode::FistWordInteger,   // /2: FIST word
    Opcode::FistpWordInteger,  // /3: FISTP word
    Opcode::FbldPackedBcd,     // /4: FBLD packed-BCD
    Opcode::FildQwordInteger,  // /5: FILD qword
    Opcode::FbstpPackedBcd,    // /6: FBSTP packed-BCD
    Opcode::FistpQwordInteger, // /7: FISTP qword
    // /r form (8-71): DF C0-FF register forms
    Opcode::FfreepSti,
    Opcode::FfreepSti,
    Opcode::FfreepSti,
    Opcode::FfreepSti,
    Opcode::FfreepSti,
    Opcode::FfreepSti,
    Opcode::FfreepSti,
    Opcode::FfreepSti, // DF C0-C7: FFREEP (287 compat)
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti,
    Opcode::FxchSti, // DF C8-CF: FXCH (undocumented)
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti, // DF D0-D7: FSTP (undocumented)
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti,
    Opcode::FstpSti, // DF D8-DF: FSTP (undocumented)
    Opcode::FnstswAx,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError, // DF E0-E7: FNSTSW AX, then errors
    Opcode::FucomipSt0Stj,
    Opcode::FucomipSt0Stj,
    Opcode::FucomipSt0Stj,
    Opcode::FucomipSt0Stj,
    Opcode::FucomipSt0Stj,
    Opcode::FucomipSt0Stj,
    Opcode::FucomipSt0Stj,
    Opcode::FucomipSt0Stj, // DF E8-EF: FUCOMIP
    Opcode::FcomipSt0Stj,
    Opcode::FcomipSt0Stj,
    Opcode::FcomipSt0Stj,
    Opcode::FcomipSt0Stj,
    Opcode::FcomipSt0Stj,
    Opcode::FcomipSt0Stj,
    Opcode::FcomipSt0Stj,
    Opcode::FcomipSt0Stj, // DF F0-F7: FCOMIP
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError,
    Opcode::IaError, // DF F8-FF
];
