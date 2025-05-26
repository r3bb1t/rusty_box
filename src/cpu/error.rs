use thiserror::Error;

pub type Result<T> = core::result::Result<T, CpuError>;

#[derive(Error, Debug)]
pub enum CpuError {
    #[error(transparent)]
    CpuId(#[from] super::cpuid::CpuIdError),

    #[error("Decoder error")]
    Decoder(#[from] super::decoder::DecodeError),
}
