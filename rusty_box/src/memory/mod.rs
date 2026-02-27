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
};
use alloc::{
    boxed::Box,
    vec::Vec,
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

    /// Zero-initialized 4KB scratch buffer for APIC MMIO (0xFEE00000-0xFEEFFFFF)
    apic_scratch: [u8; 4096],

    #[cfg(feature = "bx_large_ram_file")]
    next_swapout_idx: Cell<usize>,
    #[cfg(all(feature = "std", feature = "bx_large_ram_file"))]
    //overflow_file: Option<Arc<Mutex<std::fs::File>>>,
    overflow_file: UnsafeCell<File>,
    //#[cfg(feature = "bx_large_ram_file")]
    //swapped_out: *const u8,
}

type Unsigned = u32;

// Memory handler signature: (addr, len, data, param) -> bool
// data is mutable for reads (handler writes to it) and const for writes (handler reads from it)
// Using *mut for both to match C void* semantics
pub(super) type MemoryHandlerT = fn(BxPhyAddress, u32, *mut core::ffi::c_void, *const core::ffi::c_void) -> bool;
pub(super) type MemoryDirectAccessHandlerT<'a> =
    fn(&dyn core::any::Any, BxPhyAddress, MemoryAccessType, &dyn core::any::Any) -> &'a mut [u8];

#[derive(Debug)]
pub(super) struct MemoryHandlerStruct<'a> {
    next: Option<Box<MemoryHandlerStruct<'a>>>, // Correctly represent the linked list
    param: *const core::ffi::c_void, // Pointer to device instance (like I/O handlers)
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

    /// A20 address mask - controls address line 20 gating
    /// This is synchronized from BxPcSystemC when A20 state changes
    a20_mask: BxPhyAddress,
}

impl BxMemC<'_> {
    /// Apply A20 masking to an address
    #[inline]
    pub fn a20_addr(&self, addr: BxPhyAddress) -> BxPhyAddress {
        addr & self.a20_mask
    }

    /// Set the A20 mask (called when A20 line state changes)
    pub fn set_a20_mask(&mut self, mask: BxPhyAddress) {
        self.a20_mask = mask;
    }

    /// Peek at raw RAM bytes (no A20 masking, no memory handlers).
    /// Returns a slice of up to `len` bytes starting at `addr`, or empty if out of bounds.
    pub fn peek_ram(&self, addr: usize, len: usize) -> &[u8] {
        let stub = &self.inherited_memory_stub;
        let real_addr = stub.vector_offset + addr;
        let ram = &stub.actual_vector;
        if real_addr < ram.len() {
            let end = (real_addr + len).min(ram.len());
            &ram[real_addr..end]
        } else {
            &[]
        }
    }

    /// Get the current A20 mask
    pub fn a20_mask(&self) -> BxPhyAddress {
        self.a20_mask
    }

    /// Disable SMRAM (System Management RAM)
    /// 
    /// Matches BX_MEM_C::disable_smram() from cpp_orig/bochs/memory/misc_mem.cc:888-893
    pub fn disable_smram(&mut self) {
        self.smram_available = false;
        self.smram_enable = false;
        self.smram_restricted = false;
    }
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

    pub fn apic_scratch(&mut self) -> &mut [u8] {
        &mut self.apic_scratch
    }

    /// Get a mutable reference to a memory block by index
    #[cfg(all(feature = "bx_large_ram_file", feature = "std"))]
    #[allow(clippy::mut_from_ref)]
    pub fn block_by_index(&self, index: usize) -> Option<&mut [u8]> {
        if let Some(Block::Block { offset }) = self.blocks_offsets().get(index) {
            let start = self.vector_offset + *offset;
            let _end = start + self.block_size;
            // SAFETY: We're accessing within bounds of actual_vector via interior mutability pattern
            let vec_ptr = self.actual_vector.as_ptr() as *mut u8;
            let slice = unsafe { core::slice::from_raw_parts_mut(vec_ptr.add(start), self.block_size) };
            Some(slice)
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

    /// Direct read access to physical RAM for debug inspection (with vector_offset applied)
    pub(crate) fn ram_slice(&self) -> &[u8] {
        let stub = &self.inherited_memory_stub;
        &stub.actual_vector[stub.vector_offset..]
    }

    /// Get raw pointer to memory for direct CPU access
    /// SAFETY: Caller must ensure the pointer is only used while memory is valid
    pub fn get_raw_memory_ptr(&mut self) -> (*mut u8, usize) {
        let ptr = self.inherited_memory_stub.actual_vector.as_mut_ptr();
        let len = self.inherited_memory_stub.actual_vector.len();
        (ptr, len)
    }

    /// Count how many registered (non-None) memory handlers exist (for diagnostics).
    pub fn memory_handler_info(&self) -> usize {
        self.memory_handlers.iter().filter(|h| h.is_some()).count()
    }

    /// Read bytes from the ROM array at the given offset (for diagnostics).
    pub fn peek_rom(&self, offset: usize, len: usize) -> Vec<u8> {
        let stub = &self.inherited_memory_stub;
        let rom_start = stub.rom_offset;
        let rom = &stub.actual_vector[rom_start..];
        let end = (offset + len).min(rom.len());
        if offset < rom.len() {
            rom[offset..end].to_vec()
        } else {
            Vec::new()
        }
    }
}
impl<'m> BxMemC<'m> {
    pub fn init_memory(
        &mut self,
        guest_size: usize,
        host_size: usize,
        block_size: usize,
    ) -> Result<()> {
        let mem_stub = BxMemoryStubC::create_and_init(guest_size, host_size, block_size)?;
        self.inherited_memory_stub = mem_stub;
        self.rom_present = [false; 65];
        self.bios_rom_addr = 0xffff0000;
        self.memory_type = [[false, false]; 13];
        Ok(())
    }
}
