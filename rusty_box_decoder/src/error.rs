use super::fetchdecode_generated::BxDecodeError;
use thiserror::Error;

pub type DecodeResult<T> = core::result::Result<T, DecodeError>;

/// Decoder error type
///
/// This enum represents all possible errors that can occur during instruction decoding.
/// It includes both decoder-specific errors, buffer underflow errors, and common conversion errors.
///
/// Error mapping from C++ source:
/// - `return(-1)` in C++ → Buffer underflow errors (PrefixBufferUnderflow, OpcodeBufferUnderflow, etc.)
/// - `return(BX_IA_ERROR)` or `ia_opcode = BX_IA_ERROR` → Decoder(BxDecodeError::BxIllegalOpcode)
/// - `assign_srcs()` returning non-OK → Decoder(BxDecodeError::...) (via #[from] conversion)
/// - `fetchImmediate()` returning -1 → ImmediateBufferUnderflow
/// - `parseModrm64/32()` returning NULL → ModRmBufferUnderflow or SibBufferUnderflow or DisplacementBufferUnderflow
#[derive(Error, Debug)]
pub enum DecodeError {
    /// Bochs decoder-specific errors
    ///
    /// Maps to: `BxDecodeError` enum values from `assign_srcs()` and other validation functions.
    /// These include illegal opcode, illegal VEX/EVEX/XOP conditions, illegal lock prefix, etc.
    #[error(transparent)]
    Decoder(#[from] BxDecodeError),

    /// Integer conversion error - occurs when a value cannot be converted to the target type
    ///
    /// This error is automatically converted from `TryFromIntError` when using `try_into()`
    /// or `TryFrom` conversions. For example, when converting a u32 to u8 and the value
    /// is too large.
    #[error("integer conversion failed: value out of range for target type")]
    IntegerConversion(#[from] core::num::TryFromIntError),

    /// Buffer underflow: not enough bytes to complete instruction decoding
    ///
    /// Maps to: `return(-1)` when `bytes.is_empty()` or general buffer exhaustion.
    /// C++ equivalent: `if (remainingInPage == 0) return(-1)` at function start.
    #[error("buffer underflow: not enough bytes to complete instruction decoding")]
    BufferUnderflow,

    /// Buffer underflow while parsing prefixes
    ///
    /// Maps to: `return(-1)` in prefix parsing switch cases when `remain == 0`.
    /// C++ locations: fetchdecode64.cc lines 1396, 1403, 1412, 1421, 1429, 1437, 1444, 1451
    ///                 fetchdecode32.cc lines 1924, 1932, 1938, 1946, 1955, 1962, 1968
    #[error("buffer underflow while parsing prefixes")]
    PrefixBufferUnderflow,

    /// Buffer underflow while parsing opcode
    ///
    /// Maps to: `return(-1)` when parsing 0x0F escape or 0F 38/3A opcodes and `remain == 0`.
    /// C++ locations: fetchdecode64.cc lines 1403, 1463
    ///                 fetchdecode32.cc lines 1924, 1980
    #[error("buffer underflow while parsing opcode")]
    OpcodeBufferUnderflow,

    /// Buffer underflow while parsing ModRM byte
    ///
    /// Maps to: `return(-1)` when `parseModrm64/32()` returns NULL (remain == 0 before ModRM).
    /// C++ locations: fetchdecode64.cc lines 742, 830, 987, 1100, 1150, 1188, 1255, 1295
    ///                 fetchdecode32.cc lines 866, 1431, 1572, 1670, 1707, 1740, 1791, 1824
    #[error("buffer underflow while parsing ModRM byte")]
    ModRmBufferUnderflow,

    /// Buffer underflow while parsing SIB byte
    ///
    /// Maps to: `return(-1)` when `decodeModrm64/32()` needs SIB but `remain == 0`.
    /// C++ locations: fetchdecode64.cc line 682 (decodeModrm64)
    ///                 fetchdecode32.cc line 752 (decodeModrm32)
    #[error("buffer underflow while parsing SIB byte")]
    SibBufferUnderflow,

    /// Buffer underflow while parsing displacement
    ///
    /// Maps to: `return(-1)` when `decodeModrm64/32()` needs displacement but insufficient bytes.
    /// C++ locations: fetchdecode64.cc lines 714, 728 (decodeModrm64)
    ///                 fetchdecode32.cc lines 738, 775, 804, 824, 840, 851 (decodeModrm32)
    #[error("buffer underflow while parsing displacement")]
    DisplacementBufferUnderflow,

    /// Buffer underflow while parsing immediate value
    ///
    /// Maps to: `return(-1)` from `fetchImmediate()` when insufficient bytes for immediate.
    /// C++ locations: fetchdecode64.cc lines 867, 876, 1023 (VEX/EVEX/XOP immediate)
    ///                 fetchdecode32.cc lines 906, 916, 926, 936, 945, 955, 979, 989, 999, etc. (fetchImmediate)
    #[error("buffer underflow while parsing immediate value")]
    ImmediateBufferUnderflow,

    /// Invalid segment register index
    ///
    /// Occurs when decoding MOV r/m16, Sreg (0x8C) or MOV Sreg, r/m16 (0x8E) and the
    /// ModRM.nnn field contains an invalid segment register index (6 or 7).
    /// Valid segment registers are: ES(0), CS(1), SS(2), DS(3), FS(4), GS(5).
    /// Per x86 specification, indices 6 and 7 should cause #UD (Undefined Opcode) exception.
    #[error("Invalid segment register index {index} in opcode {opcode:#04x} (valid: 0-5)")]
    InvalidSegmentRegister { index: u8, opcode: u8 },
}

// Implement Display for BxDecodeError to support #[error(transparent)]
impl core::fmt::Display for BxDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BxDecodeError::BxDecodeOk => write!(f, "decode ok"),
            BxDecodeError::BxIllegalOpcode => write!(f, "illegal opcode"),
            BxDecodeError::BxIllegalLockPrefix => write!(f, "illegal lock prefix"),
            BxDecodeError::BxIllegalVexXopVvv => write!(f, "illegal VEX/XOP VVV"),
            BxDecodeError::BxIllegalVexXopWithSsePrefix => {
                write!(f, "illegal VEX/XOP with SSE prefix")
            }
            BxDecodeError::BxIllegalVexXopWithRexPrefix => {
                write!(f, "illegal VEX/XOP with REX prefix")
            }
            BxDecodeError::BxIllegalVexXopOpcodeMap => write!(f, "illegal VEX/XOP opcode map"),
            BxDecodeError::BxVexXopBadVectorLength => write!(f, "VEX/XOP bad vector length"),
            BxDecodeError::BxVsibForbiddenAsize16 => {
                write!(f, "VSIB forbidden in 16-bit address size")
            }
            BxDecodeError::BxVsibIllegalSibIndex => write!(f, "VSIB illegal SIB index"),
            BxDecodeError::BxEvexReservedBitsSet => write!(f, "EVEX reserved bits set"),
            BxDecodeError::BxEvexIllegalEvexBSaeNotAllowed => {
                write!(f, "EVEX illegal B/SAE not allowed")
            }
            BxDecodeError::BxEvexIllegalEvexBBroadcastNotAllowed => {
                write!(f, "EVEX illegal broadcast not allowed")
            }
            BxDecodeError::BxEvexIllegalKmaskRegister => write!(f, "EVEX illegal k-mask register"),
            BxDecodeError::BxEvexIllegalZeroMaskingWithKmaskSrcOrDest => {
                write!(f, "EVEX illegal zero masking with k-mask src/dest")
            }
            BxDecodeError::BxEvexIllegalZeroMaskingVsib => {
                write!(f, "EVEX illegal zero masking VSIB")
            }
            BxDecodeError::BxEvexIllegalZeroMaskingMemoryDestination => {
                write!(f, "EVEX illegal zero masking memory destination")
            }
            BxDecodeError::BxAmxIllegalTileRegister => write!(f, "AMX illegal tile register"),
            BxDecodeError::Other => write!(f, "other decode error"),
            BxDecodeError::NoMoreLen => write!(f, "no more length available"),
            BxDecodeError::U32toUsize => write!(f, "u32 to usize conversion failed"),
            BxDecodeError::Ud32 => write!(f, "undefined 32-bit instruction"),
            BxDecodeError::ModRmParseFail => write!(f, "ModRM parse failed"),
            BxDecodeError::ThreeDNow => write!(f, "3DNow! instruction error"),
            BxDecodeError::DecodeModrm32 => write!(f, "decode ModRM32 failed"),
            BxDecodeError::ParseModrm32 => write!(f, "parse ModRM32 failed"),
            BxDecodeError::Execute1NotImplemented => write!(f, "execute1 not implemented"),
        }
    }
}
