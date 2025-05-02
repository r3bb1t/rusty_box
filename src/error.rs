use thiserror::Error;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    // Memory
    #[error("FATAL ERROR: all available memory is already allocated!")]
    AllAvailibleMemoryAllocated,
    #[error("Block size {0} is not power of two!")]
    BlockSizeIsNotAPowerOfTwo(u32),
    #[error(
        "FATAL ERROR: Insufficient working RAM, all blocks are currently used for TLB entries!"
    )]
    InsufficientRam,
    #[error("Memory is not a multiply of 1 megabyte")]
    MemorySizeIsNotAMultiplyOf1Megabyte,
    #[error("Unable to allocate memory overflow file")]
    UnableToCreateTempFile,
}
