use tempfile::tempfile;

use crate::{Error, Result};

use super::BxMemoryStubC;
use super::{BIOSROMSZ, EXROMSIZE};

use crate::config::BxPhyAddress;
use crate::cpu::BxCpuC;
use crate::pc_system::a20_addr;

use std::cell::{Cell, UnsafeCell};
use std::fs::File;
use std::sync::Arc;

#[inline]
fn is_power_of_2(x: u32) -> bool {
    (x & (x - 1)) == 0
}

const BX_MEM_VECTOR_ALIGN: u64 = 4096;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Block<'a> {
    Block(Option<&'a mut [u8]>),
    SwappedOut,
}

// implement getters and setters
impl BxMemoryStubC {
    #[allow(clippy::mut_from_ref)]
    pub fn actual_vector(&self) -> &mut Vec<u8> {
        unsafe { &mut (*self.actual_vector.get()) }
    }
    //
    //fn set_vector(&self, vector: Vec<u8>) {
    //    unsafe { *self.vector_offset.get() = vector }
    //}
    //
    //fn get_blocks(&self) -> &'a mut Vec<Option<UnsafeCell<&'a mut [u8]>>> {
    //    unsafe { &mut *self.blocks.get() }
    //}

    #[allow(clippy::mut_from_ref)]
    fn blocks_offsets(&self) -> &mut Vec<Option<usize>> {
        unsafe { &mut (*self.blocks_offsets.get()) }
    }

    //fn blocks(&self) -> &mut Vec<Option<&mut u8>> {
    //    let offsets =
    //    todo!()
    //}

    pub fn vector(&self) -> &mut [u8] {
        &mut (self.actual_vector()[self.vector_offset.get()..])
    }

    pub fn rom(&self) -> &mut [u8] {
        &mut (self.actual_vector()[self.rom_offset..])
    }

    pub fn bogus(&self) -> &mut [u8] {
        &mut (self.actual_vector()[self.bogus_offset..])
    }

    //fn blocks_by_index(&self, index: usize) -> Option<&'a mut [u8]> {
    //    if let Some(ref mut val) = &mut self.get_blocks().get_mut(index)? {
    //        let a: &mut [u8] = unsafe { *val.get() };
    //        let b = &mut *a;
    //        Some(b)
    //    } else {
    //        None
    //    }
    //    //todo!()
    //}
    pub fn blocks_by_index(&self, index: usize) -> Option<&mut [u8]> {
        if let Some(offset) = self.blocks_offsets().get_mut(index)? {
            let block_ptr = &mut self.vector()[*offset..];
            Some(block_ptr)
        } else {
            None
        }
    }

    fn overflow_file(&self) -> &mut Option<File> {
        unsafe { &mut *self.overflow_file.get() }
    }
}

impl BxMemoryStubC {
    fn get_actual_vector_and_offset_to_vector(bytes: u64, alignment: u64) -> (Vec<u8>, usize) {
        let test_mask: u64 = alignment - 1;
        let actual_vector_size = bytes + test_mask;
        let actual_vector = vec![0u8; actual_vector_size as usize];
        let actual_vector_ptr = actual_vector.as_ptr() as u64;
        let masked = ((actual_vector_ptr + test_mask) & !test_mask) - actual_vector_ptr;
        (actual_vector, masked as usize)
    }

    pub fn create_and_init(guest: u64, host: u64, block_size: u32) -> Result<Self> {
        // accept only memory size which is multiply of 1M
        if (host & 0xfffff) == 0 || (guest & 0xfffff) == 0 {
            return Err(Error::MemorySizeIsNotAMultiplyOf1Megabyte);
        }

        if !is_power_of_2(block_size) {
            return Err(Error::BlockSizeIsNotAPowerOfTwo(block_size));
        }

        let (actual_vector, vector_offset) = Self::get_actual_vector_and_offset_to_vector(
            (host + BIOSROMSZ + EXROMSIZE + 4096) as _,
            BX_MEM_VECTOR_ALIGN,
        );
        tracing::info!(
            "allocated memory at {:p}. after alignment, vector={:p}, block_size = {}k",
            actual_vector.as_ptr(),
            actual_vector[vector_offset..].as_ptr(),
            //vector.as_ptr(),
            block_size / 1024
        );

        let len: u64 = guest;
        let allocated: u64 = host;
        let rom_offset = host as usize;
        let bogus_offset = (host + BIOSROMSZ + EXROMSIZE) as usize;

        // block must be large enough to fit num_blocks in 32-bit
        assert!((len / block_size as u64) <= 0xffffffff);

        let num_blocks: u32 = len as u32 / block_size;
        tracing::info!("{:.2}MB", len as f64 / (1024.0 * 1024.0));
        tracing::info!("mem block size = {:8X}, blocks={}", block_size, num_blocks);

        let mut blocks = Vec::with_capacity(num_blocks as usize);
        let used_blocks = if false {
            // Map each block to the corresponding location in actual_vector
            for idx in 0..num_blocks {
                blocks.push(Some((idx * block_size) as usize));
            }
            num_blocks
        } else {
            for _ in 0..num_blocks {
                blocks.push(None);
            }
            0
        };
        //todo!()

        //let swapped_out =
        //    (std::ptr::null::<u8>() as isize - std::mem::size_of::<u8>() as isize) as *const u8;
        Ok(Self {
            actual_vector: UnsafeCell::new(actual_vector),
            len: Cell::new(len),
            allocated: Cell::new(allocated),
            block_size: Cell::new(block_size),
            blocks_offsets: UnsafeCell::new(blocks),
            vector_offset: Cell::new(vector_offset),
            rom_offset,
            bogus_offset,

            used_blocks: Cell::new(used_blocks),
            #[cfg(feature = "bx_large_ram_file")]
            next_swapout_idx: Cell::new(0),
            #[cfg(feature = "bx_large_ram_file")]
            overflow_file: None,
            //swapped_out,
        })
    }

    //// NOTE: Returns offset to blocks (blocks[block]) instead of reference
    //fn get_vector(&self, addr: BxPhyAddress) -> &mut Option<UnsafeCell<&'a mut [u8]>> {
    //    let blocks = self.get_blocks();
    //    let block: u32 = addr as u32 / self.block_size.get() as u32;
    //
    //    if cfg!(feature = "bx_large_ram_file") {
    //        if blocks[block as usize].is_none() {
    //            // allocate block
    //        } else if let Some(block) = &blocks[block as usize] {
    //            // allocate block
    //        }
    //    } else {
    //        if blocks[block as usize].is_some() {
    //            // allocate block
    //        }
    //    }
    //
    //    // TODO: check if "+block" is correct
    //    let offset: u32 = addr as u32 & (self.block_size.get() as u32 - 1 + block);
    //    &mut blocks[offset as usize]
    //}

    //#[cfg(feature = "bx_large_ram_file")]
    //fn read_block(&mut self, block: u32) {
    //    use std::io::{Read, Seek};
    //    let binding = self.overflow_file.clone();
    //    let mut overflow_file = binding.lock().unwrap();
    //
    //    let block_address: u64 = (block * self.block_size.get() as u32).into();
    //
    //    if overflow_file
    //        .seek(std::io::SeekFrom::Start(block_address))
    //        .is_err()
    //    {
    //        panic!(
    //            "FATAL ERROR: Could not seek to {:x} in memory overflow file!",
    //            block_address
    //        )
    //    }
    //
    //    let blocks_reference = self.blocks_by_index(block as usize).unwrap();
    //    let read_result = overflow_file.read_exact(blocks_reference);
    //
    //    // Check for EOF
    //    let mut single_byte_buf = [0u8];
    //    let read_single_byte_result = overflow_file.read(&mut single_byte_buf);
    //
    //    let is_eof = if let Ok(bytes_read) = read_single_byte_result {
    //        bytes_read == 0
    //    } else {
    //        // Seek back one byte
    //        overflow_file.seek_relative(-1).unwrap();
    //        false
    //    };
    //
    //    if read_result.is_err() || is_eof {
    //        panic!(
    //            "FATAL ERROR: Could not read from {:X} in memory overflow file!",
    //            block_address
    //        );
    //    }
    //}
    //

    pub fn allocate_block(&self, block: u32, cpus: &[BxCpuC]) -> Result<()> {
        let max_blocks: u32 = self.allocated.get() as u32 / self.block_size.get();

        if cfg!(feature = "bx_large_ram_file") {
            let used_blocks = self.used_blocks.get();
            if used_blocks >= max_blocks {
                let original_replacement_block = self.next_swapout_idx.get();
                // Find a block to replace
                let mut used_for_tlb: bool;
                let mut buffer;
                loop {
                    loop {
                        // Just increment 'next_swapout_idx' before comparison
                        {
                            let current_next_swapout_idx = self.next_swapout_idx.get();
                            self.next_swapout_idx.set(current_next_swapout_idx + 1);
                        }
                        if self.next_swapout_idx.get()
                            == self.len.get() as u32 / self.block_size.get()
                        {
                            self.next_swapout_idx.set(0);
                        }

                        if self.next_swapout_idx.get() == original_replacement_block {
                            return Err(Error::InsufficientRam);
                        }
                        let current_block =
                            self.blocks_by_index(self.next_swapout_idx.get() as usize);
                        buffer = Block::Block(current_block);
                        if buffer == Block::SwappedOut {
                            break;
                        }
                    }

                    used_for_tlb = false;
                    let Block::Block(buffer_as_ref) = &buffer else {
                        unreachable!("tried to tread buffer as ref")
                    };

                    let buffer_end = buffer_as_ref
                        .as_ref()
                        .unwrap()
                        .as_ptr()
                        .wrapping_add(self.block_size.get() as _);

                    for cpu in cpus {
                        used_for_tlb = cpu.check_addr_in_tlb_buffers(&buffer, &buffer_end.clone());
                    }

                    if !used_for_tlb {
                        break;
                    }
                }

                let address: BxPhyAddress =
                    (self.next_swapout_idx.get() + self.block_size.get()).into();

                if (unsafe { *self.overflow_file.get() }).is_some() {
                    let temp_file = tempfile().map_err(|_| Error::UnableToCreateTempFile)?;
                    self.overflow_file = Some(Arc::new(temp_file.into()));
                }
            }
        } else {
            // Legacy default allocator
            if self.used_blocks.get() >= max_blocks {
                return Err(Error::AllAvailibleMemoryAllocated);
            } else {
                //BX_MEM_THIS blocks[block] = BX_MEM_THIS vector + (BX_MEM_THIS used_blocks * BX_MEM_THIS block_size);
                let old_used_blocks = self.used_blocks.get();
                self.used_blocks.set(old_used_blocks + 1);
            }
            tracing::debug!(
                "allocate_block: used_blocks={:X} of {:X}",
                self.used_blocks.get(),
                max_blocks
            );
        }

        todo!()
    }

    pub fn get_vector(&self, addr: &BxPhyAddress, cpus: &[BxCpuC]) -> Result<&mut [u8]> {
        let block: usize = (addr / self.block_size.get() as BxPhyAddress) as _;
        let blocks = self.blocks_offsets();

        if cfg!(feature = "bx_large_ram_file") {
            // TODO: Continue here and check if swapped out if always null
            if blocks[block].is_none() {
                self.allocate_block(block as u32, cpus)?;
            }
        }

        let a = blocks[block];
        let a = blocks[block].unwrap() + (*addr as usize & (self.block_size.get() as usize - 1));
        todo!()
    }

    pub fn dbg_fetch_mem(
        &self,
        _cpu: BxCpuC,
        addr: BxPhyAddress,
        mut len: u32,
        buf: &mut [u8],
        cpus: &[BxCpuC],
    ) -> Result<bool> {
        let mut a20_addr_: BxPhyAddress = a20_addr(addr);
        let mut ret = true;
        let mut buf_offset = 0;

        while len > 0 {
            if a20_addr_ < self.len.get() {
                // TODO: Check if its really index 0
                buf[buf_offset] = *self.get_vector(&a20_addr_, cpus)?.first().unwrap();
            } else if cfg!(feature = "bx_phy_address_long") && a20_addr_ > 0xffffffff {
                buf[buf_offset] = 0xff;
                ret = false;
            } else {
                buf[buf_offset] = 0xff;
                ret = false;
            }
            len -= 1;

            buf_offset += 1;
            // TODO: I'm not sure about 8
            a20_addr_ += 8;
        }

        Ok(ret)
    }

    //fn allocate_block(&self, block: usize) {
    //    todo!()
    //}
}
