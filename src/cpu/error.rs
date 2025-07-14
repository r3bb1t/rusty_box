use thiserror::Error;

use crate::config::BxPhyAddress;

pub type Result<T> = core::result::Result<T, CpuError>;

#[derive(Error, Debug)]
pub enum CpuError {
    #[error("Shadow stack prematurely busy is left set !")]
    ShadowStackPrematurelyBusy,

    #[error("prefetch: running in bogus memory, pAddr={p_addr:#x}")]
    PrefetchBogusMemory { p_addr: BxPhyAddress },

    #[error("prefetch: getHostMemAddr vetoed direct read, pAddr={p_addr:#x}")]
    VetoedDirectRead { p_addr: BxPhyAddress },

    // smm
    #[error("smram map[{index}] = {value}")]
    SmramMap { index: usize, value: u32 },

    #[error(transparent)]
    CpuId(#[from] super::cpuid::CpuIdError),

    #[error("Decoder error")]
    Decoder(#[from] super::decoder::DecodeError),
}
