mod error;
pub(crate) mod memory_rusty_box;
pub mod memory_stub;
pub mod misc_mem;

//#[cfg(test)]
//mod tests;

pub use super::error::Result;
use crate::{
    config::BxPhyAddress,
    cpu::{rusty_box::MemoryAccessType, BxCpuC, BxCpuIdTrait},
    memory::misc_mem::FLASH_READ_ARRAY,
};
use alloc::{
    boxed::Box,
    vec::{self, Vec},
};
pub use error::*;

use core::cell::{Cell, UnsafeCell};

#[cfg(all(feature = "bx_large_ram_file", feature = "std"))]
use std::fs::File;

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
    actual_vector: Vec<u8>,
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

type MemoryHandlerT = fn(BxPhyAddress, u32, &dyn core::any::Any, &dyn core::any::Any) -> bool;
type MemoryDirectAccessHandlerT<'a> =
    fn(&dyn core::any::Any, BxPhyAddress, MemoryAccessType, &dyn core::any::Any) -> &'a mut [u8];

#[derive(Debug)]
pub(super) struct MemoryHandlerStruct<'a> {
    next: Option<Box<MemoryHandlerStruct<'a>>>, // Correctly represent the linked list
    param: Box<dyn core::any::Any>,
    begin: BxPhyAddress,
    end: BxPhyAddress,
    bitmap: u16,
    read_handler: MemoryHandlerT,
    write_handler: MemoryHandlerT,
    da_handler: Option<MemoryDirectAccessHandlerT<'a>>,
}

//#define BIOS_MAP_LAST128K(addr) (((addr) | 0xfff00000) & BIOS_MASK)

static BIOS_ROM_LOWER: u8 = 0x01;
static BIOS_ROM_EXTENDED: u8 = 0x02;
static BIOS_ROM_1MEG: u8 = 0x04;

#[derive(Debug)]
pub struct BxMemC<'a> {
    memory_handlers: Vec<Option<MemoryHandlerStruct<'a>>>,
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

    inherited_memory_stub: BxMemoryStubC,
}

// implement getters and setters for memory stub
impl BxMemoryStubC {
    #[allow(clippy::mut_from_ref)]
    pub fn actual_vector<'a>(&'a mut self) -> &'a mut [u8] {
        //unsafe { &mut (*self.actual_vector.get()) }
        //unsafe { &mut (*self.actual_vector.get()) }
        &mut self.actual_vector
    }

    #[allow(clippy::mut_from_ref)]
    fn blocks_offsets(&self) -> &mut Vec<Block> {
        unsafe { &mut (*self.blocks_offsets.get()) }
    }

    pub fn vector<'a>(&'a mut self) -> &'a mut [u8] {
        //&mut self.actual_vector()[self.vector_offset..]
        &mut self.actual_vector[self.vector_offset..]
    }

    pub fn rom(&mut self) -> &mut [u8] {
        //&mut (self.actual_vector()[self.rom_offset..])
        &mut self.actual_vector[self.rom_offset..]
    }

    pub fn bogus(&mut self) -> &mut [u8] {
        //&mut (self.actual_vector()[self.bogus_offset..])
        &mut self.actual_vector[self.bogus_offset..]
    }

    //pub fn block_by_index(&self, index: usize) -> Option<&mut [u8]> {
    //    if let Block::Block { offset } = self.blocks_offsets().get(index)? {
    //        let block_ptr = &mut self.vector()[*offset..self.block_size];
    //        Some(block_ptr)
    //    } else {
    //        None
    //    }
    //}

    #[cfg(all(feature = "bx_large_ram_file", feature = "std"))]
    #[allow(clippy::mut_from_ref)]
    fn overflow_file_mut(&self) -> &mut File {
        unsafe { &mut *self.overflow_file.get() }
    }
}

impl<'m> BxMemC<'m> {
    pub(crate) fn get_vector<I: BxCpuIdTrait>(
        &mut self,
        cpus: &[&BxCpuC<I>],
        addr: BxPhyAddress,
    ) -> Result<&mut [u8]> {
        self.inherited_memory_stub.get_vector(addr, cpus)
    }

    #[cfg(feature = "bx_support_monitor_mwait")]
    pub(super) fn is_monitor<I: BxCpuIdTrait>(
        cpus: &[&BxCpuC<I>],
        begin_addr: BxPhyAddress,
        len: u32,
    ) -> bool {
        BxMemoryStubC::is_monitor(cpus, begin_addr, len)
    }

    pub(crate) fn get_memory_len(&self) -> usize {
        self.inherited_memory_stub.len
    }
}
