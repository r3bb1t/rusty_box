pub mod memory_stub;

#[cfg(test)]
mod tests;

use std::{
    cell::{Cell, RefCell, UnsafeCell},
    fs::File,
    sync::{Arc, Mutex},
};

use crate::config::BxPhyAddress;

/// 4M BIOS ROM @0xffc00000, must be a power of 2
pub(super) static BIOSROMSZ: u64 = 1 << 22;
/// ROMs 0xc0000-0xdffff (area 0xe0000-0xfffff=bios mapped)
pub(super) static EXROMSIZE: u64 = 0x20000;

pub(super) static BIOS_MASK: u64 = BIOSROMSZ - 1;
pub(super) static EXROM_MASK: u64 = EXROMSIZE - 1;

pub struct BxMemoryStubC {
    /// could be > 4G
    len: Cell<u64>,
    /// could be > 4G
    allocated: Cell<u64>,
    /// individual block size, must be power of 2
    block_size: Cell<u32>,
    actual_vector: UnsafeCell<Vec<u8>>,
    /// aligned correctly
    vector_offset: Cell<usize>,
    /// None if swapped out
    blocks_offsets: UnsafeCell<Vec<Option<usize>>>,
    /// 512k BIOS rom space + 128k expansion rom space
    rom_offset: usize,
    /// 4k for unexisting memory
    bogus_offset: usize,

    used_blocks: Cell<u32>,

    #[cfg(feature = "bx_large_ram_file")]
    next_swapout_idx: Cell<u32>,
    #[cfg(feature = "bx_large_ram_file")]
    //overflow_file: Option<Arc<Mutex<std::fs::File>>>,
    overflow_file: UnsafeCell<Option<File>>,
    //#[cfg(feature = "bx_large_ram_file")]
    //swapped_out: *const u8,
}

type Unsigned = u32;
type MemoryHandlerT = fn(BxPhyAddress, u32, dyn std::any::Any, dyn std::any::Any) -> bool;
type MemoryDirectAccessHandlerT = fn(BxPhyAddress, Unsigned, dyn std::any::Any) -> Vec<u8>;

pub(super) struct MemoryHandlerStruct {
    memory_handler_struct: Box<Self>,
    param: Box<dyn std::any::Any>,
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
