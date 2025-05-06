use tempfile::tempfile;

use super::{Block, BxMemoryStubC, MemoryError, Result, BIOSROMSZ, EXROMSIZE};

use crate::config::BxPhyAddress;
use crate::cpu::BxCpuC;
use crate::pc_system::a20_addr;

use std::cell::{Cell, UnsafeCell};
use std::io::{Read, Seek};
use std::io::{SeekFrom, Write};

#[inline]
fn is_power_of_2(x: usize) -> bool {
    (x & (x - 1)) == 0
}

const BX_MEM_VECTOR_ALIGN: usize = 4096;

impl BxMemoryStubC {
    fn get_actual_vector_and_offset_to_vector(bytes: usize, alignment: usize) -> (Vec<u8>, usize) {
        // Validate alignment

        // Calculate the mask and actual vector size
        let test_mask: usize = alignment - 1;
        let actual_vector_size = bytes + test_mask;

        // Create the vector
        let actual_vector = vec![0u8; actual_vector_size];

        // Calculate the pointer and offset using unsafe block
        let actual_vector_ptr = actual_vector.as_ptr() as usize;
        let masked: usize = ((actual_vector_ptr + test_mask) & !test_mask) - actual_vector_ptr;

        (actual_vector, masked)
    }

    pub fn create_and_init(guest: usize, host: usize, block_size: usize) -> Result<Self> {
        // accept only memory size which is multiply of 1M
        const ONE_MEGABYTE: usize = 1 << 20; // 1 MB in bytes

        if (host % ONE_MEGABYTE) != 0 || (guest % ONE_MEGABYTE) != 0 {
            return Err(MemoryError::MemorySizeIsNotAMultiplyOf1Megabyte.into());
        }

        if !is_power_of_2(block_size) {
            return Err(MemoryError::BlockSizeIsNotAPowerOfTwo(block_size).into());
        }

        let (actual_vector, vector_offset) = Self::get_actual_vector_and_offset_to_vector(
            host + BIOSROMSZ + EXROMSIZE + 4096,
            BX_MEM_VECTOR_ALIGN,
        );
        tracing::info!(
            "allocated memory at {:p}. after alignment, vector={:p}, block_size = {}k",
            actual_vector.as_ptr(),
            actual_vector[vector_offset..].as_ptr(),
            //vector.as_ptr(),
            block_size / 1024
        );

        let len = guest;
        let allocated = host;
        let rom_offset = host;
        let bogus_offset = host + BIOSROMSZ + EXROMSIZE;

        // block must be large enough to fit num_blocks in 32-bit
        assert!((len / block_size) <= 0xffffffff);

        let num_blocks = len / block_size;
        tracing::info!("{:.2}MB", len as f64 / (1024.0 * 1024.0));
        tracing::info!("mem block size = {:8X}, blocks={}", block_size, num_blocks);

        let mut blocks = Vec::with_capacity(num_blocks);
        let used_blocks = if false {
            // Map each block to the corresponding location in actual_vector
            for idx in 0..num_blocks {
                blocks.push(Block::Block {
                    offset: idx * block_size,
                });
            }
            num_blocks
        } else {
            for _ in 0..num_blocks {
                blocks.push(Block::SwappedOut);
            }
            0
        };
        //todo!()

        //let swapped_out =
        //    (std::ptr::null::<u8>() as isize - std::mem::size_of::<u8>() as isize) as *const u8;
        //
        #[cfg(feature = "bx_large_ram_file")]
        let overflow_file = tempfile().map_err(MemoryError::UnableToCreateTempFile)?;
        Ok(Self {
            actual_vector: UnsafeCell::new(actual_vector),
            len,
            allocated,
            block_size,
            blocks_offsets: UnsafeCell::new(blocks),
            vector_offset,
            rom_offset,
            bogus_offset,

            used_blocks: Cell::new(used_blocks),
            #[cfg(feature = "bx_large_ram_file")]
            next_swapout_idx: Cell::new(0),
            #[cfg(feature = "bx_large_ram_file")]
            overflow_file: UnsafeCell::new(overflow_file),
            //swapped_out,
        })
    }

    #[cfg(feature = "bx_large_ram_file")]
    fn read_block(&self, block: usize) -> Result<()> {
        let block_address = block * self.block_size;
        let chosen_block = self.block_by_index(block).unwrap();
        let overflow_file = self.overflow_file_mut();

        overflow_file.seek(SeekFrom::Start(block_address.try_into()?))?;

        overflow_file.read_exact(chosen_block)?;

        Ok(())
    }

    pub fn allocate_block(&self, block: usize, cpus: &[BxCpuC]) -> Result<()> {
        let max_blocks = self.allocated / self.block_size;

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
                        if self.next_swapout_idx.get() == self.len / self.block_size {
                            self.next_swapout_idx.set(0);
                        }

                        if self.next_swapout_idx.get() == original_replacement_block {
                            return Err(MemoryError::InsufficientRam.into());
                        }
                        //let current_block = self.block_by_index(self.next_swapout_idx.get());
                        buffer = Block::Block {
                            offset: self.next_swapout_idx.get(),
                        };
                        if buffer == Block::SwappedOut {
                            break;
                        }
                    }

                    used_for_tlb = false;

                    let buffer_end;
                    {
                        let Block::Block { offset: buffer } = buffer else {
                            unreachable!()
                        };
                        buffer_end = buffer + self.block_size
                    }
                    //let buffer_end = buffer_as_ref
                    //    .as_ref()
                    //    .unwrap()
                    //    .as_ptr()
                    //    .wrapping_add(self.block_size.get());

                    for cpu in cpus {
                        used_for_tlb = cpu.check_addr_in_tlb_buffers(&buffer, buffer_end);
                    }

                    if !used_for_tlb {
                        break;
                    }
                }

                let address: BxPhyAddress = self.next_swapout_idx.get() + self.block_size;

                // Write swapped out block
                let overflow_file = &mut self.overflow_file_mut();
                overflow_file
                    // FIXME: don't unwrap
                    .seek(SeekFrom::Start(
                        address
                            .try_into()
                            .expect("An error occured while seeking in overflow file"),
                    ))
                    .map_err(|e| MemoryError::CantSeekToAddressOverflowFile(address, e))?;

                // TODO: Don't unwrap
                overflow_file
                    .write_all(self.block_by_index(self.next_swapout_idx.get()).unwrap())
                    .map_err(|e| MemoryError::FailedToWriteToOverflowFIle(address, e))?;

                // Mark swapped out block
                self.blocks_offsets()[self.next_swapout_idx.get()] = Block::SwappedOut;
                // TODO: Continue here
                self.blocks_offsets()[block] = buffer;

                self.read_block(block)?;
                tracing::debug!(
                    "allocate_block: block={:#x}, replaced {:#x}",
                    block,
                    self.next_swapout_idx.get()
                )
            }
        } else {
            // Legacy default allocator
            if self.used_blocks.get() >= max_blocks {
                return Err(MemoryError::AllAvailibleMemoryAllocated.into());
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
        let block: usize = addr / self.block_size;
        let blocks = self.blocks_offsets();

        if cfg!(feature = "bx_large_ram_file") {
            // TODO: Continue here and check if swapped out if always null
            if let Block::SwappedOut = blocks[block] {
                self.allocate_block(block, cpus)?;
            }
        } else {
            self.allocate_block(block, cpus)?;
        }

        let offset =
            (self.block_by_index(block).unwrap().as_ptr() as usize + addr) & (self.block_size - 1);
        Ok(&mut self.vector()[offset..self.block_size])
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
            if a20_addr_ < self.len {
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
