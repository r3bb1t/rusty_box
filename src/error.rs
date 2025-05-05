use thiserror::Error;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Memory(#[from] crate::memory::MemoryError),

    #[error(transparent)]
    Infallible(#[from] std::convert::Infallible),

    #[error(transparent)]
    TryFromInt(#[from] std::num::TryFromIntError),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
