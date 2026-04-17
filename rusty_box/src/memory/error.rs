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
    #[cfg(feature = "std")]
    #[error("Unable to allocate memory overflow file: {0}")]
    UnableToCreateTempFile(std::io::Error),
    #[cfg(feature = "std")]
    #[error("FATAL ERROR: Could not seek to {0:x} in overflow file! {1}")]
    CantSeekToAddressOverflowFile(usize, std::io::Error),
    #[cfg(feature = "std")]
    #[error("FATAL ERROR: Could not write at {0:x} in overflow file! {1}")]
    FailedToWriteToOverflowFIle(usize, std::io::Error),
    #[error("Internal memory error: {0}")]
    Internal(&'static str),

    #[error("Tried to write monitored page at addr: {0:x}")]
    WriteMonitoredPage(usize),

    #[error("write_physical_page: cross page access at address {addr:#X}, len={len}")]
    WritePhysicalPage { addr: BxPhyAddress, len: usize },

    #[error("read_physical_page: cross page access at address {addr:#X}, len={len}")]
    ReadPhysicalPage { addr: BxPhyAddress, len: usize },

    // ROM loading / BIOS
    #[error("ROM image is too large (max {0} bytes)")]
    RomTooLarge(usize),
    #[error("System BIOS must end at 0xfffff, but ends at {0:#x}")]
    SystemBiosInvalidEnd(u64),
    #[error("ROM image size must be a multiple of 512 bytes")]
    RomSizeNotMultipleOf512,
    #[error("ROM image must start at a 2KB boundary")]
    RomNot2kAligned,
    #[error("ROM address space out of range")]
    RomAddressOutOfRange,
    #[error("ROM address space {0:#x} already in use")]
    RomAddressAlreadyInUse(usize),

    // Memory handlers
    #[error("Invalid address range for memory handler")]
    InvalidAddressRange,
    #[error("Overlapping memory handlers")]
    OverlappingHandlers,

    // Paging errors (converted to CPU page faults)
    #[error("Page not present")]
    PageNotPresent,
    #[error("Page protection violation")]
    PageProtectionViolation,
    #[error("Page reserved bit violation")]
    PageReservedBitViolation,
}
