//! Caller-owned dense physical RAM, independent of PC address policy.
//!
//! Addresses `0..len` are RAM, including legacy PC apertures. No A20 gate,
//! SMRAM, PCI hole, ROM tail, residency map, overflow file or allocation.
//! Unassigned physical data reads return open-bus 0xff; writes have no target.
//! These are not guest #PFs: paging faults still come from the real x86 MMU.
//! Instruction fetch requires actual backing and exits when none is present.
use super::code_writes::CodeWrites;
use crate::config::BxPhyAddress;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FlatMemoryError {
    #[error("flat RAM needs {required} code stamps, supplied {provided}")]
    StampStorage { required: usize, provided: usize },
    #[error("flat RAM access out of range: {address:#x}, length={len}")]
    OutOfRange { address: BxPhyAddress, len: usize },
}

/// Physical bytes and one code stamp per 4 KiB page, exclusively borrowed.
/// Host/DMA writers use `write`; no mutable byte slice escapes. Storage needs
/// neither host alignment nor a 'static lifetime. Construction allocates nothing.
#[derive(Debug)]
pub struct FlatMemory<'ram> {
    bytes: &'ram mut [u8],
    stamps: &'ram mut [u32],
    writes: CodeWrites,
}
impl<'ram> FlatMemory<'ram> {
    pub const fn required_stamps(bytes: usize) -> usize {
        bytes / 4096 + if bytes % 4096 == 0 { 0 } else { 1 }
    }

    pub fn new(bytes: &'ram mut [u8], stamps: &'ram mut [u32]) -> Result<Self, FlatMemoryError> {
        let required = Self::required_stamps(bytes.len());
        if stamps.len() < required {
            return Err(FlatMemoryError::StampStorage {
                required,
                provided: stamps.len(),
            });
        }
        let stamps = &mut stamps[..required];
        stamps.fill(0);
        Ok(Self {
            bytes,
            stamps,
            writes: CodeWrites::new(),
        })
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn read(&self, address: BxPhyAddress, out: &mut [u8]) -> Result<(), FlatMemoryError> {
        let range = checked_range(address, out.len(), self.bytes.len()).ok_or(
            FlatMemoryError::OutOfRange {
                address,
                len: out.len(),
            },
        )?;
        out.copy_from_slice(&self.bytes[range]);
        Ok(())
    }

    pub fn write(&mut self, address: BxPhyAddress, bytes: &[u8]) -> Result<(), FlatMemoryError> {
        let range = checked_range(address, bytes.len(), self.bytes.len()).ok_or(
            FlatMemoryError::OutOfRange {
                address,
                len: bytes.len(),
            },
        )?;
        self.bytes[range].copy_from_slice(bytes);
        mark_write(self.stamps, &mut self.writes, address, bytes.len());
        Ok(())
    }

    pub(crate) fn parts(&mut self) -> FlatParts<'_> {
        FlatParts {
            bytes: self.bytes,
            stamps: self.stamps,
            writes: &mut self.writes,
        }
    }
}
/// Named reborrow: the storage owner's lifetime does not escape into the CPU.
pub(crate) struct FlatParts<'a> {
    pub(crate) bytes: &'a mut [u8],
    pub(crate) stamps: &'a mut [u32],
    pub(crate) writes: &'a mut CodeWrites,
}
pub(crate) fn checked_range(
    address: BxPhyAddress,
    len: usize,
    capacity: usize,
) -> Option<core::ops::Range<usize>> {
    let start = usize::try_from(address.get()).ok()?;
    let end = start.checked_add(len)?;
    (end <= capacity).then_some(start..end)
}
pub(crate) fn mark_write(
    stamps: &mut [u32],
    writes: &mut CodeWrites,
    mut address: BxPhyAddress,
    mut len: usize,
) {
    while len != 0 {
        let count = len.min(4096 - address.page_offset() as usize);
        if let Ok(index) = usize::try_from(address.get() >> 12) {
            if index < stamps.len() {
                writes.write(stamps, index, address, count as u32);
            }
        }
        address = address.wrapping_add(count as u64);
        len -= count;
    }
}
