//! Decoder constants — opcode table attributes, operand descriptors, and bit field offsets.
//!
//! Mirrors Bochs `cpu/decoder/fetchdecode.h` lines 30-494.
//!
//! These constants are used by the opcode lookup tables in `opmap*.rs`
//! and by the decoders in `decode32.rs` / `decode64.rs`.
//!
//! # Decoding Mask (decmask) Layout — 24 bits
//!
//! The decmask is a 24-bit value encoding instruction attributes (operand/address
//! size, SSE prefix, mod field, register indices) used to select the correct
//! opcode from multi-entry opcode tables.
//!
//! ```text
//!  ┌───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬─────┬─────┐
//!  │23 │22 │21 │20 │19 │18 │17 │16 │15 │14 │13 │12 │11 │10 │ 9 │ 8 │ 7 │6:4  │3:0  │
//!  ├───┼───┼───┼───┼───┼───┼───┼───┼───┼───┼───┼───┼───┼───┼───┼───┼───┼─────┼─────┤
//!  │OS │OS │AS │AS │SSE│SSE│LCK│MOD│IS │VEX│EVX│XOP│VL │VL │VXW│MK │SRC│ RRR │ NNN │
//!  │64 │32 │64 │32 │F23│PFX│   │C0 │64 │   │   │   │512│128│   │ K0│=  │     │     │
//!  │   │   │   │   │   │   │   │   │   │   │   │   │   │256│   │   │DST│     │     │
//!  └───┴───┴───┴───┴───┴───┴───┴───┴───┴───┴───┴───┴───┴───┴───┴───┴───┴─────┴─────┘
//! ```
//!
//! Each ATTR_* constant packs a 24-bit **value** and a 24-bit **mask** into a
//! single `u64`: `((value << offset) << 24) | (mask << offset)`.
//! The table lookup ANDs the mask with the decmask and compares against the value.

#![allow(dead_code)]             // ATTR_* / OP_* defined for completeness
#![allow(non_upper_case_globals)] // OP_* names match Bochs (OP_Eb, OP_Gd, etc.)

use super::last_opcode;

// ============================================================================
// Core formula matching Bochs fetchdecode.h
// ============================================================================

/// Compute a decmask attribute entry.
///
/// Bochs formula (fetchdecode.h line 415+):
/// ```text
///   ((BX_CONST64(value) << offset) << 24) | (BX_CONST64(mask) << offset)
/// ```
///
/// The upper 24 bits of the result hold `value << offset` (what we want to match),
/// and the lower 24 bits hold `mask << offset` (which bits to test).
pub(crate) const fn attr(value: u64, mask: u64, offset: u32) -> u64 {
    ((value << offset) << 24) | (mask << offset)
}

// ============================================================================
// EVEX preparation flag (Bochs fetchdecode.h line 73)
// ============================================================================

pub(crate) const BX_PREPARE_EVEX: u32 = 128;

// ============================================================================
// Decoding mask bit offsets (Bochs fetchdecode.h lines 395-413)
// ============================================================================

pub(crate) const OS64_OFFSET: u32 = 23;
pub(crate) const OS32_OFFSET: u32 = 22;
pub(crate) const AS64_OFFSET: u32 = 21;
pub(crate) const AS32_OFFSET: u32 = 20;
pub(crate) const SSE_PREFIX_F2_F3_OFFSET: u32 = 19;
pub(crate) const SSE_PREFIX_OFFSET: u32 = 18;
pub(crate) const LOCK_PREFIX_OFFSET: u32 = 17;
pub(crate) const MODC0_OFFSET: u32 = 16;
pub(crate) const IS64_OFFSET: u32 = 15;
pub(crate) const VEX_OFFSET: u32 = 14;
pub(crate) const EVEX_OFFSET: u32 = 13;
pub(crate) const XOP_OFFSET: u32 = 12;
pub(crate) const VEX_VL_512_OFFSET: u32 = 11;
pub(crate) const VEX_VL_128_256_OFFSET: u32 = 10;
pub(crate) const VEX_W_OFFSET: u32 = 9;
pub(crate) const MASK_K0_OFFSET: u32 = 8;
pub(crate) const SRC_EQ_DST_OFFSET: u32 = 7;
pub(crate) const RRR_OFFSET: u32 = 4;
pub(crate) const NNN_OFFSET: u32 = 0;

// ============================================================================
// ATTR_* constants — computed from Bochs formulas (fetchdecode.h lines 415-486)
// ============================================================================

// Operand size attributes
pub(crate) const ATTR_OS64: u64 = attr(3, 3, OS32_OFFSET);
pub(crate) const ATTR_OS32: u64 = attr(1, 3, OS32_OFFSET);
pub(crate) const ATTR_OS16: u64 = attr(0, 3, OS32_OFFSET);
pub(crate) const ATTR_OS16_32: u64 = attr(0, 1, OS64_OFFSET);
pub(crate) const ATTR_OS32_64: u64 = attr(1, 1, OS32_OFFSET);

// Address size attributes
pub(crate) const ATTR_AS64: u64 = attr(3, 3, AS32_OFFSET);
pub(crate) const ATTR_AS32: u64 = attr(1, 3, AS32_OFFSET);
pub(crate) const ATTR_AS16: u64 = attr(0, 3, AS32_OFFSET);
pub(crate) const ATTR_AS16_32: u64 = attr(0, 1, AS64_OFFSET);
pub(crate) const ATTR_AS32_64: u64 = attr(1, 1, AS32_OFFSET);

// Mode attributes
pub(crate) const ATTR_IS32: u64 = attr(0, 1, IS64_OFFSET);
pub(crate) const ATTR_IS64: u64 = attr(1, 1, IS64_OFFSET);

// SSE prefix attributes
pub(crate) const ATTR_SSE_NO_PREFIX: u64 = attr(0, 3, SSE_PREFIX_OFFSET);
pub(crate) const ATTR_SSE_PREFIX_66: u64 = attr(1, 3, SSE_PREFIX_OFFSET);
pub(crate) const ATTR_SSE_PREFIX_F3: u64 = attr(2, 3, SSE_PREFIX_OFFSET);
pub(crate) const ATTR_SSE_PREFIX_F2: u64 = attr(3, 3, SSE_PREFIX_OFFSET);
pub(crate) const ATTR_NO_SSE_PREFIX_F2_F3: u64 = attr(0, 1, SSE_PREFIX_F2_F3_OFFSET);

// Lock/ModRM attributes
pub(crate) const ATTR_LOCK_PREFIX_NOT_ALLOWED: u64 = attr(0, 1, LOCK_PREFIX_OFFSET);
pub(crate) const ATTR_LOCK: u64 = attr(1, 1, LOCK_PREFIX_OFFSET);
pub(crate) const ATTR_MODC0: u64 = attr(1, 1, MODC0_OFFSET);
pub(crate) const ATTR_NO_MODC0: u64 = attr(0, 1, MODC0_OFFSET);
pub(crate) const ATTR_MOD_REG: u64 = ATTR_MODC0;
pub(crate) const ATTR_MOD_MEM: u64 = ATTR_NO_MODC0;

// VEX/EVEX/XOP attributes
pub(crate) const ATTR_VEX: u64 = attr(1, 1, VEX_OFFSET);
pub(crate) const ATTR_EVEX: u64 = attr(1, 1, EVEX_OFFSET);
pub(crate) const ATTR_XOP: u64 = attr(1, 1, XOP_OFFSET);
pub(crate) const ATTR_VL128: u64 = attr(0, 3, VEX_VL_128_256_OFFSET);
pub(crate) const ATTR_VL256: u64 = attr(1, 3, VEX_VL_128_256_OFFSET);
pub(crate) const ATTR_VL512: u64 = attr(3, 3, VEX_VL_128_256_OFFSET);
pub(crate) const ATTR_VL256_512: u64 = attr(1, 1, VEX_VL_128_256_OFFSET);
pub(crate) const ATTR_VL128_256: u64 = attr(0, 1, VEX_VL_512_OFFSET);
pub(crate) const ATTR_VEX_L0: u64 = ATTR_VL128;
pub(crate) const ATTR_VEX_W0: u64 = attr(0, 1, VEX_W_OFFSET);
pub(crate) const ATTR_VEX_W1: u64 = attr(1, 1, VEX_W_OFFSET);
pub(crate) const ATTR_NO_VEX_EVEX_XOP: u64 = attr(0, 3, XOP_OFFSET);
pub(crate) const ATTR_MASK_K0: u64 = attr(1, 1, MASK_K0_OFFSET);
pub(crate) const ATTR_MASK_REQUIRED: u64 = attr(0, 1, MASK_K0_OFFSET);

// Source/register encoding attributes
pub(crate) const ATTR_SRC_EQ_DST: u64 = ATTR_MOD_REG | attr(1, 1, SRC_EQ_DST_OFFSET);
pub(crate) const ATTR_RRR0: u64 = attr(0, 7, RRR_OFFSET);
pub(crate) const ATTR_RRR1: u64 = attr(1, 7, RRR_OFFSET);
pub(crate) const ATTR_RRR2: u64 = attr(2, 7, RRR_OFFSET);
pub(crate) const ATTR_RRR3: u64 = attr(3, 7, RRR_OFFSET);
pub(crate) const ATTR_RRR4: u64 = attr(4, 7, RRR_OFFSET);
pub(crate) const ATTR_RRR5: u64 = attr(5, 7, RRR_OFFSET);
pub(crate) const ATTR_RRR6: u64 = attr(6, 7, RRR_OFFSET);
pub(crate) const ATTR_RRR7: u64 = attr(7, 7, RRR_OFFSET);
pub(crate) const ATTR_NNN0: u64 = attr(0, 7, NNN_OFFSET);
pub(crate) const ATTR_NNN1: u64 = attr(1, 7, NNN_OFFSET);
pub(crate) const ATTR_NNN2: u64 = attr(2, 7, NNN_OFFSET);
pub(crate) const ATTR_NNN3: u64 = attr(3, 7, NNN_OFFSET);
pub(crate) const ATTR_NNN4: u64 = attr(4, 7, NNN_OFFSET);
pub(crate) const ATTR_NNN5: u64 = attr(5, 7, NNN_OFFSET);
pub(crate) const ATTR_NNN6: u64 = attr(6, 7, NNN_OFFSET);
pub(crate) const ATTR_NNN7: u64 = attr(7, 7, NNN_OFFSET);

// ============================================================================
// Compile-time verification: computed values match Bochs
// ============================================================================

const _: () = {
    assert!(ATTR_OS64 == 211106245115904);
    assert!(ATTR_OS32 == 70368756760576);
    assert!(ATTR_OS16 == 12582912);
    assert!(ATTR_OS16_32 == 8388608);
    assert!(ATTR_OS32_64 == 70368748371968);
    assert!(ATTR_AS64 == 52776561278976);
    assert!(ATTR_AS32 == 17592189190144);
    assert!(ATTR_AS16 == 3145728);
    assert!(ATTR_AS16_32 == 2097152);
    assert!(ATTR_AS32_64 == 17592187092992);
    assert!(ATTR_IS32 == 32768);
    assert!(ATTR_IS64 == 549755846656);
    assert!(ATTR_SSE_NO_PREFIX == 786432);
    assert!(ATTR_SSE_PREFIX_66 == 4398047297536);
    assert!(ATTR_SSE_PREFIX_F3 == 8796093808640);
    assert!(ATTR_SSE_PREFIX_F2 == 13194140319744);
    assert!(ATTR_NO_SSE_PREFIX_F2_F3 == 524288);
    assert!(ATTR_LOCK_PREFIX_NOT_ALLOWED == 131072);
    assert!(ATTR_LOCK == 2199023386624);
    assert!(ATTR_MODC0 == 1099511693312);
    assert!(ATTR_NO_MODC0 == 65536);
    assert!(ATTR_MOD_REG == 1099511693312);
    assert!(ATTR_MOD_MEM == 65536);
    assert!(ATTR_VEX == 274877923328);
    assert!(ATTR_EVEX == 137438961664);
    assert!(ATTR_XOP == 68719480832);
    assert!(ATTR_VL128 == 3072);
    assert!(ATTR_VL256 == 17179872256);
    assert!(ATTR_VL512 == 51539610624);
    assert!(ATTR_VL256_512 == 17179870208);
    assert!(ATTR_VL128_256 == 2048);
    assert!(ATTR_VEX_L0 == 3072);
    assert!(ATTR_VEX_W0 == 512);
    assert!(ATTR_VEX_W1 == 8589935104);
    assert!(ATTR_NO_VEX_EVEX_XOP == 12288);
    assert!(ATTR_MASK_K0 == 4294967552);
    assert!(ATTR_MASK_REQUIRED == 256);
    assert!(ATTR_SRC_EQ_DST == 1101659177088);
    assert!(ATTR_RRR0 == 112);
    assert!(ATTR_RRR1 == 268435568);
    assert!(ATTR_RRR2 == 536871024);
    assert!(ATTR_RRR3 == 805306480);
    assert!(ATTR_RRR4 == 1073741936);
    assert!(ATTR_RRR5 == 1342177392);
    assert!(ATTR_RRR6 == 1610612848);
    assert!(ATTR_RRR7 == 1879048304);
    assert!(ATTR_NNN0 == 7);
    assert!(ATTR_NNN1 == 16777223);
    assert!(ATTR_NNN2 == 33554439);
    assert!(ATTR_NNN3 == 50331655);
    assert!(ATTR_NNN4 == 67108871);
    assert!(ATTR_NNN5 == 83886087);
    assert!(ATTR_NNN6 == 100663303);
    assert!(ATTR_NNN7 == 117440519);
};

// ============================================================================
// SSE prefix enum (Bochs fetchdecode.h lines 32-36)
// ============================================================================

/// SSE prefix state decoded from legacy prefixes.
///
/// In SSE/SSE2+, the legacy prefixes 66/F2/F3 are repurposed
/// as opcode extensions (not size/rep overrides).
#[repr(u32)]
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub enum SsePrefix {
    PrefixNone = 0,
    Prefix66 = 1,
    PrefixF3 = 2,
    PrefixF2 = 3,
}

// ============================================================================
// BxDecodeError — matching Bochs bx_decode_error_t (fetchdecode.h lines 38-57)
// ============================================================================

/// Decoder-specific error codes matching Bochs `bx_decode_error_t` exactly.
///
/// 18 variants (0-17), no Rust-only extensions.
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub enum BxDecodeError {
    BxDecodeOk = 0,
    BxIllegalOpcode = 1,
    BxIllegalLockPrefix = 2,
    BxIllegalVexXopVvv = 3,
    BxIllegalVexXopWithSsePrefix = 4,
    BxIllegalVexXopWithRexPrefix = 5,
    BxIllegalVexXopOpcodeMap = 6,
    BxVexXopBadVectorLength = 7,
    BxVsibForbiddenAsize16 = 8,
    BxVsibIllegalSibIndex = 9,
    BxEvexReservedBitsSet = 10,
    BxEvexIllegalEvexBSaeNotAllowed = 11,
    BxEvexIllegalEvexBBroadcastNotAllowed = 12,
    BxEvexIllegalKmaskRegister = 13,
    BxEvexIllegalZeroMaskingWithKmaskSrcOrDest = 14,
    BxEvexIllegalZeroMaskingVsib = 15,
    BxEvexIllegalZeroMaskingMemoryDestination = 16,
    BxAmxIllegalTileRegister = 17,
}

impl Default for BxDecodeError {
    fn default() -> Self {
        Self::BxVsibIllegalSibIndex
    }
}

// ============================================================================
// Operand descriptor enums (Bochs fetchdecode.h lines 112-201)
// ============================================================================

/// Where the source operand should be taken from.
/// Bochs `BX_SRC_*` (fetchdecode.h lines 113-125).
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum OperandSource {
    /// No source, implicit source, or immediate
    None = 0,
    /// AL/AX/EAX/RAX or ST(0) for x87
    Eax = 1,
    /// Source from modrm.nnn
    Nnn = 2,
    /// Register or memory reference from modrm.rm
    Rm = 3,
    /// Register or EVEX memory reference from modrm.rm
    VectorRm = 4,
    /// Source from (E)VEX.vvv
    Vvv = 5,
    /// Source from immediate byte (VEX is4 field)
    Vib = 6,
    /// Gather/scatter vector index (VSIB)
    Vsib = 7,
    /// Immediate value
    Imm = 8,
    /// Immediate value used as branch offset
    BranchOffset = 9,
    /// Implicit register or memory reference
    Implicit = 10,
}

/// Register type / memory access size hint.
/// Bochs register type enum (fetchdecode.h lines 130-147).
///
/// Used as the TYPE field in `form_src(type, source)` when source is
/// `OperandSource::Rm` or `OperandSource::Nnn`.
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum RegisterType {
    NoRegister = 0x0,
    Gpr8 = 0x1,
    Gpr16 = 0x2,
    Gpr32 = 0x3,
    Gpr64 = 0x4,
    FpuReg = 0x5,
    MmxReg = 0x6,
    MmxHalfReg = 0x7,
    VmmReg = 0x8,
    KmaskReg = 0x9,
    KmaskRegPair = 0xA,
    TmmReg = 0xB,
    SegReg = 0xC,
    CReg = 0xD,
    DReg = 0xE,
}

/// EVEX vector memory reference size, used together with `OperandSource::VectorRm`.
/// Bochs `BX_VMM_*` (fetchdecode.h lines 150-166).
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum VectorRmType {
    FullVector = 0x0,
    FullVectorW = 0x1,
    ScalarByte = 0x2,
    ScalarWord = 0x3,
    ScalarDword = 0x4,
    ScalarQword = 0x5,
    Scalar = 0x6,
    HalfVector = 0x7,
    HalfVectorW = 0x8,
    QuarterVector = 0x9,
    QuarterVectorW = 0xA,
    EighthVector = 0xB,
    Vec128 = 0xC,
    Vec256 = 0xD,
}

/// Immediate operand forms.
/// Bochs `BX_IMM*` / `BX_DIRECT_*` (fetchdecode.h lines 169-184).
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ImmediateForm {
    /// Implicit 1 (e.g., SHL r, 1)
    Imm1 = 0x1,
    /// 8-bit immediate
    ImmB = 0x2,
    /// 8-bit immediate sign-extended to 16-bit
    ImmBwSe = 0x3,
    /// 8-bit immediate sign-extended to 32-bit
    ImmBdSe = 0x4,
    /// 16-bit immediate
    ImmW = 0x5,
    /// 32-bit immediate
    ImmD = 0x6,
    /// 64-bit immediate
    ImmQ = 0x7,
    /// Second 8-bit immediate (e.g., ENTER imm16, imm8)
    ImmB2 = 0x8,
    /// Direct far pointer (seg:offset)
    DirectPtr = 0x9,
    // Encodings 0xA-0xB free
    /// Direct memory reference, byte
    DirectMemrefB = 0xC,
    /// Direct memory reference, word
    DirectMemrefW = 0xD,
    /// Direct memory reference, dword
    DirectMemrefD = 0xE,
    /// Direct memory reference, qword
    DirectMemrefQ = 0xF,
}

/// Implicit register or memory references (for string ops, CL shifts, DX I/O).
/// Bochs `BX_RSIREF_*` / `BX_RDIREF_*` / `BX_USECL` / `BX_USEDX`
/// (fetchdecode.h lines 187-201).
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ImplicitRef {
    RsiRefB = 0x1,
    RsiRefW = 0x2,
    RsiRefD = 0x3,
    RsiRefQ = 0x4,
    RdiRefB = 0x5,
    RdiRefW = 0x6,
    RdiRefD = 0x7,
    RdiRefQ = 0x8,
    MmxRdiRef = 0x9,
    VecRdiRef = 0xA,
    UseCl = 0xB,
    UseDx = 0xC,
}

// ============================================================================
// form_src() and OP_* operand descriptor constants
// (Bochs fetchdecode.h lines 203-370)
// ============================================================================

/// Pack a register type/size and operand source into a single byte.
///
/// ```text
///  ┌───────────┬───────────┐
///  │ type (4b) │ source(4b)│
///  │  bits 7-4 │  bits 3-0 │
///  └───────────┴───────────┘
/// ```
///
/// Bochs macro: `#define BX_FORM_SRC(type, src) (((type) << 4) | (src))`
pub(crate) const fn form_src(reg_type: u8, src: u8) -> u8 {
    (reg_type << 4) | src
}

/// Extract operand source (low 4 bits). Bochs: `BX_DISASM_SRC_ORIGIN(desc)`
pub(crate) const fn src_origin(desc: u8) -> u8 {
    desc & 0xF
}

/// Extract register type (high 4 bits). Bochs: `BX_DISASM_SRC_TYPE(desc)`
pub(crate) const fn src_type(desc: u8) -> u8 {
    desc >> 4
}

// --- No operand ---
pub(crate) const OP_NONE: u8 = OperandSource::None as u8;

// --- GPR from modrm.rm ---
pub(crate) const OP_Eb: u8 = form_src(RegisterType::Gpr8 as u8, OperandSource::Rm as u8);
pub(crate) const OP_Ew: u8 = form_src(RegisterType::Gpr16 as u8, OperandSource::Rm as u8);
pub(crate) const OP_Ed: u8 = form_src(RegisterType::Gpr32 as u8, OperandSource::Rm as u8);
pub(crate) const OP_Eq: u8 = form_src(RegisterType::Gpr64 as u8, OperandSource::Rm as u8);

// --- GPR from modrm.nnn ---
pub(crate) const OP_Gb: u8 = form_src(RegisterType::Gpr8 as u8, OperandSource::Nnn as u8);
pub(crate) const OP_Gw: u8 = form_src(RegisterType::Gpr16 as u8, OperandSource::Nnn as u8);
pub(crate) const OP_Gd: u8 = form_src(RegisterType::Gpr32 as u8, OperandSource::Nnn as u8);
pub(crate) const OP_Gq: u8 = form_src(RegisterType::Gpr64 as u8, OperandSource::Nnn as u8);

// --- Accumulator (EAX family) ---
pub(crate) const OP_AL_REG: u8 = form_src(RegisterType::Gpr8 as u8, OperandSource::Eax as u8);
pub(crate) const OP_AX_REG: u8 = form_src(RegisterType::Gpr16 as u8, OperandSource::Eax as u8);
pub(crate) const OP_EAX_REG: u8 = form_src(RegisterType::Gpr32 as u8, OperandSource::Eax as u8);
pub(crate) const OP_RAX_REG: u8 = form_src(RegisterType::Gpr64 as u8, OperandSource::Eax as u8);

// --- Implicit CL, DX ---
pub(crate) const OP_CL_REG: u8 = form_src(ImplicitRef::UseCl as u8, OperandSource::Implicit as u8);
pub(crate) const OP_DX_REG: u8 = form_src(ImplicitRef::UseDx as u8, OperandSource::Implicit as u8);

// --- Immediate operands ---
pub(crate) const OP_I1: u8 = form_src(ImmediateForm::Imm1 as u8, OperandSource::Imm as u8);
pub(crate) const OP_IB: u8 = form_src(ImmediateForm::ImmB as u8, OperandSource::Imm as u8);
pub(crate) const OP_S_IBW: u8 = form_src(ImmediateForm::ImmBwSe as u8, OperandSource::Imm as u8);
pub(crate) const OP_S_IBD: u8 = form_src(ImmediateForm::ImmBdSe as u8, OperandSource::Imm as u8);
pub(crate) const OP_IW: u8 = form_src(ImmediateForm::ImmW as u8, OperandSource::Imm as u8);
pub(crate) const OP_ID: u8 = form_src(ImmediateForm::ImmD as u8, OperandSource::Imm as u8);
pub(crate) const OP_S_ID: u8 = OP_ID; // Alias: sign-extended dword immediate
pub(crate) const OP_IQ: u8 = form_src(ImmediateForm::ImmQ as u8, OperandSource::Imm as u8);
pub(crate) const OP_IB2: u8 = form_src(ImmediateForm::ImmB2 as u8, OperandSource::Imm as u8);

// --- Branch offsets ---
pub(crate) const OP_JW: u8 = form_src(ImmediateForm::ImmW as u8, OperandSource::BranchOffset as u8);
pub(crate) const OP_JD: u8 = form_src(ImmediateForm::ImmD as u8, OperandSource::BranchOffset as u8);
pub(crate) const OP_JQ: u8 = OP_JD; // Same encoding — Jq uses sign-extended dword
pub(crate) const OP_JBW: u8 = form_src(ImmediateForm::ImmBwSe as u8, OperandSource::BranchOffset as u8);
pub(crate) const OP_JBD: u8 = form_src(ImmediateForm::ImmBdSe as u8, OperandSource::BranchOffset as u8);
pub(crate) const OP_JBQ: u8 = OP_JBD; // Same encoding — Jbq uses sign-extended byte→dword

// --- Memory-only operands ---
pub(crate) const OP_M: u8 = form_src(RegisterType::NoRegister as u8, OperandSource::Rm as u8);
pub(crate) const OP_MT: u8 = form_src(RegisterType::FpuReg as u8, OperandSource::Rm as u8);
pub(crate) const OP_MDQ: u8 = form_src(VectorRmType::FullVector as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_MB: u8 = OP_Eb; // Aliases: memory form = same encoding as register form
pub(crate) const OP_MW: u8 = OP_Ew;
pub(crate) const OP_MD: u8 = OP_Ed;
pub(crate) const OP_MQ: u8 = OP_Eq;

// --- MMX operands ---
pub(crate) const OP_PQ: u8 = form_src(RegisterType::MmxReg as u8, OperandSource::Nnn as u8);
pub(crate) const OP_QD: u8 = form_src(RegisterType::MmxHalfReg as u8, OperandSource::Rm as u8);
pub(crate) const OP_QQ: u8 = form_src(RegisterType::MmxReg as u8, OperandSource::Rm as u8);

// --- XMM/YMM/ZMM from modrm.nnn ---
pub(crate) const OP_VDQ: u8 = form_src(RegisterType::VmmReg as u8, OperandSource::Nnn as u8);
pub(crate) const OP_VQQ: u8 = OP_VDQ;
pub(crate) const OP_VPH: u8 = OP_VDQ;
pub(crate) const OP_VPS: u8 = OP_VDQ;
pub(crate) const OP_VPD: u8 = OP_VDQ;
pub(crate) const OP_VSH: u8 = OP_VDQ;
pub(crate) const OP_VSS: u8 = OP_VDQ;
pub(crate) const OP_VSD: u8 = OP_VDQ;
pub(crate) const OP_VQ: u8 = OP_VDQ;
pub(crate) const OP_VD: u8 = OP_VDQ;

// --- XMM/YMM/ZMM from modrm.rm (scalar/vector variants) ---
pub(crate) const OP_WQ: u8 = form_src(VectorRmType::ScalarQword as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_WD: u8 = form_src(VectorRmType::ScalarDword as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_WW: u8 = form_src(VectorRmType::ScalarWord as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_WB: u8 = form_src(VectorRmType::ScalarByte as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_WDQ: u8 = form_src(VectorRmType::FullVector as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_WPH: u8 = OP_WDQ;
pub(crate) const OP_WPS: u8 = OP_WDQ;
pub(crate) const OP_WPD: u8 = OP_WDQ;
pub(crate) const OP_WSH: u8 = form_src(VectorRmType::ScalarWord as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_WSS: u8 = form_src(VectorRmType::ScalarDword as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_WSD: u8 = form_src(VectorRmType::ScalarQword as u8, OperandSource::VectorRm as u8);

// --- EVEX memory destination variants ---
pub(crate) const OP_M_VPH: u8 = form_src(VectorRmType::FullVectorW as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VPS: u8 = form_src(VectorRmType::FullVector as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VPD: u8 = OP_M_VPS;
pub(crate) const OP_M_VPH16: u8 = form_src(VectorRmType::ScalarWord as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VPS32: u8 = form_src(VectorRmType::ScalarDword as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VPD64: u8 = form_src(VectorRmType::ScalarQword as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VDQ: u8 = OP_M_VPS;
pub(crate) const OP_M_VQQ: u8 = OP_M_VPS;
pub(crate) const OP_M_VSH: u8 = form_src(VectorRmType::ScalarWord as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VSS: u8 = form_src(VectorRmType::ScalarDword as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VSD: u8 = form_src(VectorRmType::ScalarQword as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VDQ8: u8 = form_src(VectorRmType::ScalarByte as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VDQ16: u8 = form_src(VectorRmType::ScalarWord as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VDQ32: u8 = form_src(VectorRmType::ScalarDword as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VDQ64: u8 = form_src(VectorRmType::ScalarQword as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VHV: u8 = form_src(VectorRmType::HalfVector as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VHVW: u8 = form_src(VectorRmType::HalfVectorW as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VQV: u8 = form_src(VectorRmType::QuarterVector as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VQVW: u8 = form_src(VectorRmType::QuarterVectorW as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VOV: u8 = form_src(VectorRmType::EighthVector as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VDQ128: u8 = form_src(VectorRmType::Vec128 as u8, OperandSource::VectorRm as u8);
pub(crate) const OP_M_VDQ256: u8 = form_src(VectorRmType::Vec256 as u8, OperandSource::VectorRm as u8);

// --- VSIB ---
pub(crate) const OP_VSIB: u8 = form_src(VectorRmType::Scalar as u8, OperandSource::Vsib as u8);

// --- XMM/YMM/ZMM from VEX.vvv ---
pub(crate) const OP_HDQ: u8 = form_src(RegisterType::VmmReg as u8, OperandSource::Vvv as u8);
pub(crate) const OP_HPH: u8 = OP_HDQ;
pub(crate) const OP_HPS: u8 = OP_HDQ;
pub(crate) const OP_HPD: u8 = OP_HDQ;
pub(crate) const OP_HSH: u8 = OP_HDQ;
pub(crate) const OP_HSS: u8 = OP_HDQ;
pub(crate) const OP_HSD: u8 = OP_HDQ;

// --- GPR from VEX.vvv (BMI) ---
pub(crate) const OP_BD: u8 = form_src(RegisterType::Gpr32 as u8, OperandSource::Vvv as u8);
pub(crate) const OP_BQ: u8 = form_src(RegisterType::Gpr64 as u8, OperandSource::Vvv as u8);

// --- XMM from immediate byte (VEX is4) ---
pub(crate) const OP_VIB: u8 = form_src(RegisterType::VmmReg as u8, OperandSource::Vib as u8);

// --- Control/Debug/Segment registers ---
pub(crate) const OP_CD: u8 = form_src(RegisterType::CReg as u8, OperandSource::Nnn as u8);
pub(crate) const OP_CQ: u8 = OP_CD;
pub(crate) const OP_DD: u8 = form_src(RegisterType::DReg as u8, OperandSource::Nnn as u8);
pub(crate) const OP_DQ: u8 = OP_DD;
pub(crate) const OP_SW: u8 = form_src(RegisterType::SegReg as u8, OperandSource::Nnn as u8);

// --- Direct memory reference (moffs) ---
pub(crate) const OP_OB: u8 = form_src(ImmediateForm::DirectMemrefB as u8, OperandSource::Imm as u8);
pub(crate) const OP_OW: u8 = form_src(ImmediateForm::DirectMemrefW as u8, OperandSource::Imm as u8);
pub(crate) const OP_OD: u8 = form_src(ImmediateForm::DirectMemrefD as u8, OperandSource::Imm as u8);
pub(crate) const OP_OQ: u8 = form_src(ImmediateForm::DirectMemrefQ as u8, OperandSource::Imm as u8);

// --- Direct far pointer ---
pub(crate) const OP_AP: u8 = form_src(ImmediateForm::DirectPtr as u8, OperandSource::Imm as u8);

// --- K-mask registers ---
pub(crate) const OP_KGB: u8 = form_src(RegisterType::KmaskReg as u8, OperandSource::Nnn as u8);
pub(crate) const OP_KEB: u8 = form_src(RegisterType::KmaskReg as u8, OperandSource::Rm as u8);
pub(crate) const OP_KHB: u8 = form_src(RegisterType::KmaskReg as u8, OperandSource::Vvv as u8);
pub(crate) const OP_KGW: u8 = OP_KGB;
pub(crate) const OP_KEW: u8 = OP_KEB;
pub(crate) const OP_KHW: u8 = OP_KHB;
pub(crate) const OP_KGD: u8 = OP_KGB;
pub(crate) const OP_KED: u8 = OP_KEB;
pub(crate) const OP_KHD: u8 = OP_KHB;
pub(crate) const OP_KGQ: u8 = OP_KGB;
pub(crate) const OP_KEQ: u8 = OP_KEB;
pub(crate) const OP_KHQ: u8 = OP_KHB;
pub(crate) const OP_KGQ2: u8 = form_src(RegisterType::KmaskRegPair as u8, OperandSource::Nnn as u8);

// --- AMX tile registers ---
pub(crate) const OP_TRM: u8 = form_src(RegisterType::TmmReg as u8, OperandSource::Rm as u8);
pub(crate) const OP_TNNN: u8 = form_src(RegisterType::TmmReg as u8, OperandSource::Nnn as u8);
pub(crate) const OP_TREG: u8 = form_src(RegisterType::TmmReg as u8, OperandSource::Vvv as u8);

// --- x87 FPU ---
pub(crate) const OP_ST0: u8 = form_src(RegisterType::FpuReg as u8, OperandSource::Eax as u8);
pub(crate) const OP_STI: u8 = form_src(RegisterType::FpuReg as u8, OperandSource::Rm as u8);

// --- Implicit RSI-referenced memory (string source) ---
pub(crate) const OP_XB: u8 = form_src(ImplicitRef::RsiRefB as u8, OperandSource::Implicit as u8);
pub(crate) const OP_XW: u8 = form_src(ImplicitRef::RsiRefW as u8, OperandSource::Implicit as u8);
pub(crate) const OP_XD: u8 = form_src(ImplicitRef::RsiRefD as u8, OperandSource::Implicit as u8);
pub(crate) const OP_XQ: u8 = form_src(ImplicitRef::RsiRefQ as u8, OperandSource::Implicit as u8);

// --- Implicit RDI-referenced memory (string destination) ---
pub(crate) const OP_YB: u8 = form_src(ImplicitRef::RdiRefB as u8, OperandSource::Implicit as u8);
pub(crate) const OP_YW: u8 = form_src(ImplicitRef::RdiRefW as u8, OperandSource::Implicit as u8);
pub(crate) const OP_YD: u8 = form_src(ImplicitRef::RdiRefD as u8, OperandSource::Implicit as u8);
pub(crate) const OP_YQ: u8 = form_src(ImplicitRef::RdiRefQ as u8, OperandSource::Implicit as u8);

// --- Implicit RDI MMX/vector (MASKMOVQ/MASKMOVDQU) ---
pub(crate) const OP_S_YQ: u8 = form_src(ImplicitRef::MmxRdiRef as u8, OperandSource::Implicit as u8);
pub(crate) const OP_S_YDQ: u8 = form_src(ImplicitRef::VecRdiRef as u8, OperandSource::Implicit as u8);

// ============================================================================
// Error group sentinel
// ============================================================================

pub(crate) const BX_OPCODE_GROUP_ERR: [u64; 1] =
    [last_opcode(0, crate::opcode::Opcode::IaError)];
