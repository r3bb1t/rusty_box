use super::fetchdecode_generated::{self, BxDecodeError};

pub type DecodeResult<T> = core::result::Result<T, DecodeError>;

pub type DecodeError = fetchdecode_generated::BxDecodeError;

impl core::fmt::Display for BxDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Decoder error: {self}")
    }
}
