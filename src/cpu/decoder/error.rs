use super::fetchdecode_generated::BxDecodeError;
use thiserror::Error;

pub type DecodeResult<T> = core::result::Result<T, DecodeError>;

/// Decoder error type
/// 
/// This enum represents all possible errors that can occur during instruction decoding.
/// It includes both decoder-specific errors and common conversion errors.
#[derive(Error, Debug)]
pub enum DecodeError {
    /// Bochs decoder-specific errors
    #[error(transparent)]
    Decoder(#[from] BxDecodeError),
    
    /// Integer conversion error - occurs when a value cannot be converted to the target type
    /// 
    /// This error is automatically converted from `TryFromIntError` when using `try_into()`
    /// or `TryFrom` conversions. For example, when converting a u32 to u8 and the value
    /// is too large.
    #[error("integer conversion failed: value out of range for target type")]
    IntegerConversion(#[from] core::num::TryFromIntError),
}

// Implement Display for BxDecodeError to support #[error(transparent)]
impl core::fmt::Display for BxDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BxDecodeError::BxDecodeOk => write!(f, "decode ok"),
            BxDecodeError::BxIllegalOpcode => write!(f, "illegal opcode"),
            BxDecodeError::BxIllegalLockPrefix => write!(f, "illegal lock prefix"),
            BxDecodeError::BxIllegalVexXopVvv => write!(f, "illegal VEX/XOP VVV"),
            BxDecodeError::BxIllegalVexXopWithSsePrefix => write!(f, "illegal VEX/XOP with SSE prefix"),
            BxDecodeError::BxIllegalVexXopWithRexPrefix => write!(f, "illegal VEX/XOP with REX prefix"),
            BxDecodeError::BxIllegalVexXopOpcodeMap => write!(f, "illegal VEX/XOP opcode map"),
            BxDecodeError::BxVexXopBadVectorLength => write!(f, "VEX/XOP bad vector length"),
            BxDecodeError::BxVsibForbiddenAsize16 => write!(f, "VSIB forbidden in 16-bit address size"),
            BxDecodeError::BxVsibIllegalSibIndex => write!(f, "VSIB illegal SIB index"),
            BxDecodeError::BxEvexReservedBitsSet => write!(f, "EVEX reserved bits set"),
            BxDecodeError::BxEvexIllegalEvexBSaeNotAllowed => write!(f, "EVEX illegal B/SAE not allowed"),
            BxDecodeError::BxEvexIllegalEvexBBroadcastNotAllowed => write!(f, "EVEX illegal broadcast not allowed"),
            BxDecodeError::BxEvexIllegalKmaskRegister => write!(f, "EVEX illegal k-mask register"),
            BxDecodeError::BxEvexIllegalZeroMaskingWithKmaskSrcOrDest => write!(f, "EVEX illegal zero masking with k-mask src/dest"),
            BxDecodeError::BxEvexIllegalZeroMaskingVsib => write!(f, "EVEX illegal zero masking VSIB"),
            BxDecodeError::BxEvexIllegalZeroMaskingMemoryDestination => write!(f, "EVEX illegal zero masking memory destination"),
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
