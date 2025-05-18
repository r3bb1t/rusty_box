mod error;
pub mod memory_stub;

#[cfg(test)]
mod tests;

pub use super::error::Result;
use crate::config::BxPhyAddress;
use alloc::{boxed::Box, vec::Vec};
pub use error::*;

use core::cell::{Cell, UnsafeCell};

#[cfg(all(feature = "bx_large_ram_file", feature = "std"))]
use std::fs::File;

/// 4M BIOS ROM @0xffc00000, must be a power of 2
pub(super) static BIOSROMSZ: usize = 1 << 22;
/// ROMs 0xc0000-0xdffff (area 0xe0000-0xfffff=bios mapped)
pub(super) static EXROMSIZE: usize = 0x20000;

pub(super) static BIOS_MASK: usize = BIOSROMSZ - 1;
pub(super) static EXROM_MASK: usize = EXROMSIZE - 1;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Block {
    Block { offset: usize },
    SwappedOut,
}

#[derive(Debug)]
pub struct BxMemoryStubC {
    /// could be > 4G
    pub(super) len: usize,
    /// could be > 4G
    allocated: usize,
    /// individual block size, must be power of 2
    block_size: usize,
    actual_vector: UnsafeCell<Vec<u8>>,
    /// aligned correctly
    vector_offset: usize,
    /// None if swapped out
    blocks_offsets: UnsafeCell<Vec<Block>>,
    /// 512k BIOS rom space + 128k expansion rom space
    rom_offset: usize,
    /// 4k for unexisting memory
    bogus_offset: usize,

    used_blocks: Cell<usize>,

    #[cfg(feature = "bx_large_ram_file")]
    next_swapout_idx: Cell<usize>,
    #[cfg(all(feature = "std", feature = "bx_large_ram_file"))]
    //overflow_file: Option<Arc<Mutex<std::fs::File>>>,
    overflow_file: UnsafeCell<File>,
    //#[cfg(feature = "bx_large_ram_file")]
    //swapped_out: *const u8,
}

type Unsigned = u32;
type MemoryHandlerT = fn(BxPhyAddress, u32, dyn core::any::Any, dyn core::any::Any) -> bool;
type MemoryDirectAccessHandlerT = fn(BxPhyAddress, Unsigned, dyn core::any::Any) -> Vec<u8>;

#[derive(Debug)]
pub(super) struct MemoryHandlerStruct {
    memory_handler_struct: Box<Self>,
    param: Box<dyn core::any::Any>,
    begin: BxPhyAddress,
    end: BxPhyAddress,
    bitmap: u16,
    read_handler: MemoryHandlerT,
    write_handler: MemoryHandlerT,
    da_handler: MemoryHandlerT,
}

//#define BIOS_MAP_LAST128K(addr) (((addr) | 0xfff00000) & BIOS_MASK)

static BIOS_ROM_LOWER: u8 = 0x01;
static BIOS_ROM_EXTENDED: u8 = 0x02;
static BIOS_ROM_1MEG: u8 = 0x04;

#[derive(Debug)]
enum MemoryAreaT {
    BxMemAreaC0000 = 0,
    BxMemAreaC4000,
    BxMemAreaC8000,
    BxMemAreaCc000,
    BxMemAreaD0000,
    BxMemAreaD4000,
    BxMemAreaD8000,
    BxMemAreaDc000,
    BxMemAreaE0000,
    BxMemAreaE4000,
    BxMemAreaE8000,
    BxMemAreaEc000,
    BxMemAreaF0000,
}

#[derive(Debug)]
pub struct BxMemC {
    memory_handlers: Vec<MemoryHandlerStruct>,
    pci_enabled: bool,
    bios_write_enabled: bool,

    smram_available: bool,
    smram_enable: bool,
    smram_restricted: bool,

    rom_present: [bool; 65],
    memory_type: [[bool; 2]; 13],
    bios_rom_addr: u32,
    bios_rom_access: u8,
    flash_type: u8,
    flash_status: u8,
    flash_wsm_state: u8,
    flash_modified: bool,
}

// implement getters and setters for memory stub
impl BxMemoryStubC {
    #[allow(clippy::mut_from_ref)]
    pub fn actual_vector(&self) -> &mut Vec<u8> {
        unsafe { &mut (*self.actual_vector.get()) }
    }

    #[allow(clippy::mut_from_ref)]
    fn blocks_offsets(&self) -> &mut Vec<Block> {
        unsafe { &mut (*self.blocks_offsets.get()) }
    }

    pub fn vector(&self) -> &mut [u8] {
        &mut (self.actual_vector()[self.vector_offset..])
    }

    pub fn rom(&self) -> &mut [u8] {
        &mut (self.actual_vector()[self.rom_offset..])
    }

    pub fn bogus(&self) -> &mut [u8] {
        &mut (self.actual_vector()[self.bogus_offset..])
    }

    pub fn block_by_index(&self, index: usize) -> Option<&mut [u8]> {
        if let Block::Block { offset } = self.blocks_offsets().get(index)? {
            let block_ptr = &mut self.vector()[*offset..self.block_size];
            Some(block_ptr)
        } else {
            None
        }
    }

    #[cfg(all(feature = "bx_large_ram_file", feature = "std"))]
    #[allow(clippy::mut_from_ref)]
    fn overflow_file_mut(&self) -> &mut File {
        unsafe { &mut *self.overflow_file.get() }
    }
}
