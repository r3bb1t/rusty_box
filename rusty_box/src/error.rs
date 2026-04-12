use thiserror::Error;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Cpu(#[from] crate::cpu::CpuError),

    #[error(transparent)]
    Memory(#[from] crate::memory::MemoryError),

    #[error(transparent)]
    PcSystem(#[from] crate::pc_system::PcSystemError),

    #[error(transparent)]
    Infallible(#[from] core::convert::Infallible),

    #[error(transparent)]
    TryFromInt(#[from] core::num::TryFromIntError),

    #[cfg(feature = "std")]
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
