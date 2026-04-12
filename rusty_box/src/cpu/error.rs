use thiserror::Error;

use alloc::string::String;

use crate::{config::BxPhyAddress, cpu::cpu::Exception};

pub type Result<T> = core::result::Result<T, CpuError>;

#[derive(Error, Debug)]
pub enum CpuError {
    #[error("exception({vector:?}): bad vector, error_code={error_code}")]
    BadVector { vector: Exception, error_code: u16 },

    #[error("Shadow stack prematurely busy is left set !")]
    ShadowStackPrematurelyBusy,

    #[error("CPU/Emulator not initialized - call initialize() first")]
    CpuNotInitialized,

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

    #[error(transparent)]
    TryFromIntError(#[from] core::num::TryFromIntError),

    #[error(transparent)]
    Memory(#[from] crate::memory::MemoryError),

    #[error("Unimplemented instruction or feature")]
    UnimplementedInstruction,

    #[error("Unimplemented opcode: {opcode}")]
    UnimplementedOpcode { opcode: String },

    /// Bochs-style control flow: exceptions/interrupt delivery longjmp back to the
    /// main decode loop. We model that by unwinding the current instruction/trace
    /// and restarting decode.
    #[error("cpu loop restart (bochs longjmp)")]
    CpuLoopRestart,

    /// x86 exception generated during execution (e.g., #UD, #GP, #PF)
    /// The exception has been delivered via IVT/IDT, but execution cannot continue
    /// (e.g., unhandled exception handler address is 0000:0000)
    #[error("x86 exception #{vector} delivered but unhandled")]
    Exception { vector: u8 },
}
