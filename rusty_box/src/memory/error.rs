use thiserror::Error;

use crate::config::BxPhyAddress;

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
    #[cfg(all(feature = "std", feature = "bx_large_ram_file"))]
    #[error("Unable to allocate memory overflow file: {0}")]
    UnableToCreateTempFile(std::io::Error),
    #[cfg(all(feature = "std", feature = "bx_large_ram_file"))]
    #[error("FATAL ERROR: Could not seek to {0:x} in overflow file! {1}")]
    CantSeekToAddressOverflowFile(usize, std::io::Error),
    #[cfg(all(feature = "std", feature = "bx_large_ram_file"))]
    #[error("FATAL ERROR: Could not write at {0:x} in overflow file! {1}")]
    FailedToWriteToOverflowFIle(usize, std::io::Error),

    #[error("Tried to write monitored page at addr: {0:x}")]
    WriteMonitoredPage(usize),

    #[error("write_physical_page: cross page access at address {addr:#X}, len={len}")]
    WritePhysicalPage { addr: BxPhyAddress, len: usize },

    #[error("read_physical_page: cross page access at address {addr:#X}, len={len}")]
    ReadPhysicalPage { addr: BxPhyAddress, len: usize },
}
