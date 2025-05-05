use thiserror::Error;

#[derive(Error, Debug)]
pub enum MemoryError {
    // Memory
    #[error("FATAL ERROR: all available memory is already allocated!")]
    AllAvailibleMemoryAllocated,
    #[error("Block size {0} is not power of two!")]
    BlockSizeIsNotAPowerOfTwo(usize),
    #[error(
        "FATAL ERROR: Insufficient working RAM, all blocks are currently used for TLB entries!"
    )]
    InsufficientRam,
    #[error("Memory is not a multiply of 1 megabyte")]
    MemorySizeIsNotAMultiplyOf1Megabyte,
    #[error("Unable to allocate memory overflow file: {0}")]
    UnableToCreateTempFile(std::io::Error),
    #[error("FATAL ERROR: Could not seek to {0:x} in overflow file! {1}")]
    CantSeekToAddressOverflowFile(usize, std::io::Error),
    #[error("FATAL ERROR: Could not write at {0:x} in overflow file! {1}")]
    FailedToWriteToOverflowFIle(usize, std::io::Error),
}
