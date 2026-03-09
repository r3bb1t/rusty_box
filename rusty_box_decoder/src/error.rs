//! Decoder error types and result alias.

use super::decoder::tables::BxDecodeError;

pub type DecodeResult<T> = core::result::Result<T, DecodeError>;

/// Decoder error type
///
/// This enum represents all possible errors that can occur during instruction decoding.
/// It includes both decoder-specific errors, buffer underflow errors, and common conversion errors.
///
/// Error mapping from C++ source:
/// - `return(-1)` in C++ → Buffer underflow errors (PrefixBufferUnderflow, OpcodeBufferUnderflow, etc.)
/// - `return(BX_IA_ERROR)` or `ia_opcode = BX_IA_ERROR` → Decoder(BxDecodeError::BxIllegalOpcode)
/// - `assign_srcs()` returning non-OK → Decoder(BxDecodeError::...) (via From conversion)
/// - `fetchImmediate()` returning -1 → ImmediateBufferUnderflow
/// - `parseModrm64/32()` returning NULL → ModRmBufferUnderflow or SibBufferUnderflow or DisplacementBufferUnderflow
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Bochs decoder-specific errors
    ///
    /// Maps to: `BxDecodeError` enum values from `assign_srcs()` and other validation functions.
    /// These include illegal opcode, illegal VEX/EVEX/XOP conditions, illegal lock prefix, etc.
    Decoder(BxDecodeError),

    /// Integer conversion error - occurs when a value cannot be converted to the target type
    ///
    /// This error is automatically converted from `TryFromIntError` when using `try_into()`
    /// or `TryFrom` conversions. For example, when converting a u32 to u8 and the value
    /// is too large.
    IntegerConversion(core::num::TryFromIntError),

    /// Buffer underflow: not enough bytes to complete instruction decoding
    ///
    /// Maps to: `return(-1)` when `bytes.is_empty()` or general buffer exhaustion.
    /// C++ equivalent: `if (remainingInPage == 0) return(-1)` at function start.
    BufferUnderflow,

    /// Buffer underflow while parsing prefixes
    ///
    /// Maps to: `return(-1)` in prefix parsing switch cases when `remain == 0`.
    PrefixBufferUnderflow,

    /// Buffer underflow while parsing opcode
    ///
    /// Maps to: `return(-1)` when parsing 0x0F escape or 0F 38/3A opcodes and `remain == 0`.
    OpcodeBufferUnderflow,

    /// Buffer underflow while parsing ModRM byte
    ///
    /// Maps to: `return(-1)` when `parseModrm64/32()` returns NULL (remain == 0 before ModRM).
    ModRmBufferUnderflow,

    /// Buffer underflow while parsing SIB byte
    ///
    /// Maps to: `return(-1)` when `decodeModrm64/32()` needs SIB but `remain == 0`.
    SibBufferUnderflow,

    /// Buffer underflow while parsing displacement
    ///
    /// Maps to: `return(-1)` when `decodeModrm64/32()` needs displacement but insufficient bytes.
    DisplacementBufferUnderflow,

    /// Buffer underflow while parsing immediate value
    ///
    /// Maps to: `return(-1)` from `fetchImmediate()` when insufficient bytes for immediate.
    ImmediateBufferUnderflow,

    /// Invalid segment register index
    ///
    /// Occurs when decoding MOV r/m16, Sreg (0x8C) or MOV Sreg, r/m16 (0x8E) and the
    /// ModRM.nnn field contains an invalid segment register index (6 or 7).
    /// Valid segment registers are: ES(0), CS(1), SS(2), DS(3), FS(4), GS(5).
    /// Per x86 specification, indices 6 and 7 should cause #UD (Undefined Opcode) exception.
    InvalidSegmentRegister { index: u8, opcode: u8 },
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Decoder(e) => write!(f, "{e}"),
            Self::IntegerConversion(e) => {
                write!(f, "integer conversion failed: {e}")
            }
            Self::BufferUnderflow => {
                write!(f, "buffer underflow: not enough bytes to decode instruction")
            }
            Self::PrefixBufferUnderflow => write!(f, "buffer underflow parsing prefixes"),
            Self::OpcodeBufferUnderflow => write!(f, "buffer underflow parsing opcode"),
            Self::ModRmBufferUnderflow => write!(f, "buffer underflow parsing ModRM"),
            Self::SibBufferUnderflow => write!(f, "buffer underflow parsing SIB"),
            Self::DisplacementBufferUnderflow => write!(f, "buffer underflow parsing displacement"),
            Self::ImmediateBufferUnderflow => write!(f, "buffer underflow parsing immediate"),
            Self::InvalidSegmentRegister { index, opcode } => {
                write!(
                    f,
                    "invalid segment register index {index} in opcode {opcode:#04x} (valid: 0-5)"
                )
            }
        }
    }
}

impl core::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Decoder(e) => Some(e),
            Self::IntegerConversion(e) => Some(e),
            _ => None,
        }
    }
}

impl From<BxDecodeError> for DecodeError {
    fn from(e: BxDecodeError) -> Self {
        Self::Decoder(e)
    }
}

impl From<core::num::TryFromIntError> for DecodeError {
    fn from(e: core::num::TryFromIntError) -> Self {
        Self::IntegerConversion(e)
    }
}

impl core::fmt::Display for BxDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BxDecodeOk => write!(f, "decode ok"),
            Self::BxIllegalOpcode => write!(f, "illegal opcode"),
            Self::BxIllegalLockPrefix => write!(f, "illegal lock prefix"),
            Self::BxIllegalVexXopVvv => write!(f, "illegal VEX/XOP VVV"),
            Self::BxIllegalVexXopWithSsePrefix => write!(f, "illegal VEX/XOP with SSE prefix"),
            Self::BxIllegalVexXopWithRexPrefix => write!(f, "illegal VEX/XOP with REX prefix"),
            Self::BxIllegalVexXopOpcodeMap => write!(f, "illegal VEX/XOP opcode map"),
            Self::BxVexXopBadVectorLength => write!(f, "VEX/XOP bad vector length"),
            Self::BxVsibForbiddenAsize16 => write!(f, "VSIB forbidden in 16-bit address size"),
            Self::BxVsibIllegalSibIndex => write!(f, "VSIB illegal SIB index"),
            Self::BxEvexReservedBitsSet => write!(f, "EVEX reserved bits set"),
            Self::BxEvexIllegalEvexBSaeNotAllowed => write!(f, "EVEX B/SAE not allowed"),
            Self::BxEvexIllegalEvexBBroadcastNotAllowed => {
                write!(f, "EVEX broadcast not allowed")
            }
            Self::BxEvexIllegalKmaskRegister => write!(f, "EVEX illegal k-mask register"),
            Self::BxEvexIllegalZeroMaskingWithKmaskSrcOrDest => {
                write!(f, "EVEX illegal zero masking with k-mask src/dest")
            }
            Self::BxEvexIllegalZeroMaskingVsib => write!(f, "EVEX illegal zero masking VSIB"),
            Self::BxEvexIllegalZeroMaskingMemoryDestination => {
                write!(f, "EVEX illegal zero masking memory destination")
            }
            Self::BxAmxIllegalTileRegister => write!(f, "AMX illegal tile register"),
            Self::Other => write!(f, "other decode error"),
            Self::NoMoreLen => write!(f, "no more length available"),
            Self::U32toUsize => write!(f, "u32 to usize conversion failed"),
            Self::Ud32 => write!(f, "undefined 32-bit instruction"),
            Self::ModRmParseFail => write!(f, "ModRM parse failed"),
            Self::ThreeDNow => write!(f, "3DNow! instruction error"),
            Self::DecodeModrm32 => write!(f, "decode ModRM32 failed"),
            Self::ParseModrm32 => write!(f, "parse ModRM32 failed"),
            Self::Execute1NotImplemented => write!(f, "execute1 not implemented"),
        }
    }
}

impl core::error::Error for BxDecodeError {}
