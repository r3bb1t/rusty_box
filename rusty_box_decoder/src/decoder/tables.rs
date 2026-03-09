//! Decoder constants — opcode table attributes and bit field offsets.
//!
//! These constants are used by the opcode lookup tables in
//! `opmap*.rs` and by the decoders in `decode32.rs` / `decode64.rs`.

#![allow(dead_code)] // ATTR_* and *_OFFSET constants are defined for completeness;
                      // many will be used when VEX/EVEX/XOP opcode tables are added.

use super::last_opcode;

// ============================================================================
// EVEX preparation flag
// ============================================================================

pub(crate) const BX_PREPARE_EVEX: u32 = 128;

// ============================================================================
// SSE prefix enum — used by both decoders
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
// Decode error enum — used in error.rs
// ============================================================================

/// Decoder-specific error codes matching Bochs decode error constants.
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
    Other,
    NoMoreLen,
    U32toUsize,
    Ud32,
    ModRmParseFail,
    ThreeDNow,
    DecodeModrm32,
    ParseModrm32,
    Execute1NotImplemented,
}

impl Default for BxDecodeError {
    /// Defaults to `BxVsibIllegalSibIndex` — matches Bochs `bx_decode_error_t` default.
    fn default() -> Self {
        Self::BxVsibIllegalSibIndex
    }
}

// ============================================================================
// Decoding mask bit offsets — used to build the decmask for opcode lookup
// ============================================================================

/// Bit offset constants for the decoding mask (`decmask`).
///
/// The decmask is a 24-bit value encoding instruction attributes
/// (operand/address size, SSE prefix, mod field, register indices)
/// used to select the correct opcode from multi-entry opcode tables.
///
/// ```text
///  23  22  21  20  19  18  17  16  15  14  13  12  11  10   9   8   7  6:4  3:0
///  os  os  as  as  sse sse lck mod is  vex evx xop vl  vl  vxw mk  eq rrr  nnn
///  64  32  64  32  f23 pfx         64          512 128     0   dst
/// ```
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
// ATTR_* constants — opcode table attribute bitmasks
// ============================================================================

// Operand size attributes
pub(crate) const ATTR_OS64: u64 = 211106245115904;
pub(crate) const ATTR_OS32: u64 = 70368756760576;
pub(crate) const ATTR_OS16: u64 = 12582912;
pub(crate) const ATTR_OS16_32: u64 = 8388608;
pub(crate) const ATTR_OS32_64: u64 = 70368748371968;

// Address size attributes
pub(crate) const ATTR_AS64: u64 = 52776561278976;
pub(crate) const ATTR_AS32: u64 = 17592189190144;
pub(crate) const ATTR_AS16: u64 = 3145728;
pub(crate) const ATTR_AS16_32: u64 = 2097152;
pub(crate) const ATTR_AS32_64: u64 = 17592187092992;

// Mode attributes
pub(crate) const ATTR_IS32: u64 = 32768;
pub(crate) const ATTR_IS64: u64 = 549755846656;

// SSE prefix attributes
pub(crate) const ATTR_SSE_NO_PREFIX: u64 = 786432;
pub(crate) const ATTR_SSE_PREFIX_66: u64 = 4398047297536;
pub(crate) const ATTR_SSE_PREFIX_F3: u64 = 8796093808640;
pub(crate) const ATTR_SSE_PREFIX_F2: u64 = 13194140319744;
pub(crate) const ATTR_NO_SSE_PREFIX_F2_F3: u64 = 524288;

// Lock/ModRM attributes
pub(crate) const ATTR_LOCK_PREFIX_NOT_ALLOWED: u64 = 131072;
pub(crate) const ATTR_LOCK: u64 = 2199023386624;
pub(crate) const ATTR_MODC0: u64 = 1099511693312;
pub(crate) const ATTR_NO_MODC0: u64 = 65536;
pub(crate) const ATTR_MOD_REG: u64 = 1099511693312;
pub(crate) const ATTR_MOD_MEM: u64 = 65536;

// VEX/EVEX/XOP attributes
pub(crate) const ATTR_VEX: u64 = 274877923328;
pub(crate) const ATTR_EVEX: u64 = 137438961664;
pub(crate) const ATTR_XOP: u64 = 68719480832;
pub(crate) const ATTR_VL128: u64 = 3072;
pub(crate) const ATTR_VL256: u64 = 17179872256;
pub(crate) const ATTR_VL512: u64 = 51539610624;
pub(crate) const ATTR_VL256_512: u64 = 17179870208;
pub(crate) const ATTR_VL128_256: u64 = 2048;
pub(crate) const ATTR_VEX_L0: u64 = 3072;
pub(crate) const ATTR_VEX_W0: u64 = 512;
pub(crate) const ATTR_VEX_W1: u64 = 8589935104;
pub(crate) const ATTR_NO_VEX_EVEX_XOP: u64 = 12288;
pub(crate) const ATTR_MASK_K0: u64 = 4294967552;
pub(crate) const ATTR_MASK_REQUIRED: u64 = 256;

// Source/register encoding attributes
pub(crate) const ATTR_SRC_EQ_DST: u64 = 1101659177088;
pub(crate) const ATTR_RRR0: u64 = 112;
pub(crate) const ATTR_RRR1: u64 = 268435568;
pub(crate) const ATTR_RRR2: u64 = 536871024;
pub(crate) const ATTR_RRR3: u64 = 805306480;
pub(crate) const ATTR_RRR4: u64 = 1073741936;
pub(crate) const ATTR_RRR5: u64 = 1342177392;
pub(crate) const ATTR_RRR6: u64 = 1610612848;
pub(crate) const ATTR_RRR7: u64 = 1879048304;
pub(crate) const ATTR_NNN0: u64 = 7;
pub(crate) const ATTR_NNN1: u64 = 16777223;
pub(crate) const ATTR_NNN2: u64 = 33554439;
pub(crate) const ATTR_NNN3: u64 = 50331655;
pub(crate) const ATTR_NNN4: u64 = 67108871;
pub(crate) const ATTR_NNN5: u64 = 83886087;
pub(crate) const ATTR_NNN6: u64 = 100663303;
pub(crate) const ATTR_NNN7: u64 = 117440519;

// ============================================================================
// Error group sentinel
// ============================================================================

pub(crate) const BX_OPCODE_GROUP_ERR: [u64; 1] =
    [last_opcode(0, crate::opcode::Opcode::IaError)];
