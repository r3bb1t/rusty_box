use thiserror::Error;

pub type Result<T> = core::result::Result<T, CpuError>;

#[derive(Error, Debug)]
pub enum CpuError {
    #[error("Shadow stack prematurely busy is left set !")]
    ShadowStackPrematurelyBusy,

    // smm
    #[error("smram map[{index}] = {value}")]
    SmramMap { index: usize, value: u32 },

    #[error(transparent)]
    CpuId(#[from] super::cpuid::CpuIdError),

    #[error("Decoder error")]
    Decoder(#[from] super::decoder::DecodeError),
}
