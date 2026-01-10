use alloc::vec::Vec;
use byteorder::{ByteOrder, LittleEndian};
#[cfg(feature = "std")]
use tempfile::tempfile;

use super::{Block, BxMemoryStubC, MemoryError, Result};
use crate::cpu::cpuid::BxCpuIdTrait;

use crate::config::BxPhyAddress;
use crate::cpu::cpu::BxCpuC;
use crate::cpu::icache::BxPageWriteStampTable;
use crate::memory::memory_rusty_box::{BIOSROMSZ, EXROMSIZE};
use crate::misc::bswap::{
    read_host_dword_to_little_endian, read_host_qword_to_little_endian,
    read_host_word_to_little_endian, write_host_dword_to_little_endian,
    write_host_qword_to_little_endian, write_host_word_to_little_endian,
};
use crate::config::BxPhyAddress as A20Mask;

use core::cell::{Cell, UnsafeCell};

#[cfg(feature = "std")]
use std::io::{Read, Seek, SeekFrom, Write};

#[inline]
fn is_power_of_2(x: usize) -> bool {
    (x & (x - 1)) == 0
}

const BX_MEM_VECTOR_ALIGN: usize = 4096;

impl BxMemoryStubC {
    pub fn alloc_vector_aligned(bytes: usize, alignment: usize) -> (Vec<u8>, usize) {
        // Validate alignment

        // Calculate the mask and actual vector size
        let test_mask: usize = alignment - 1;
        let actual_vector_size = bytes + test_mask;

        // Create the vector
        let mut actual_vector = Vec::new();
        // And initialize it
        (0..actual_vector_size).for_each(|_| actual_vector.push(0));

        // Calculate the pointer and offset using unsafe block
        let actual_vector_ptr = actual_vector.as_ptr() as usize;
        let masked: usize = ((actual_vector_ptr + test_mask) & !test_mask) - actual_vector_ptr;

        (actual_vector, masked)
    }

    pub fn get_memory_len(&self) -> usize {
        self.len
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

        let (actual_vector, vector_offset) =
            Self::alloc_vector_aligned(host + BIOSROMSZ + EXROMSIZE + 4096, BX_MEM_VECTOR_ALIGN);
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

        #[cfg(all(feature = "std", feature = "bx_large_ram_file"))]
        let overflow_file = tempfile().map_err(MemoryError::UnableToCreateTempFile)?;
        Ok(Self {
            actual_vector,
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
            #[cfg(all(feature = "std", feature = "bx_large_ram_file"))]
            overflow_file: UnsafeCell::new(overflow_file),
            //swapped_out,
        })
    }

    pub(super) fn get_vector<'a, I: BxCpuIdTrait>(
        &'a mut self,
        addr: BxPhyAddress,
        cpus: &[&BxCpuC<I>],
    ) -> Result<&'a mut [u8]> {
        let block: usize = (addr / self.block_size as u64) as _;
        let blocks = self.blocks_offsets();

        if cfg!(feature = "bx_large_ram_file") {
            // TODO: Continue here and check if swapped out if always null
            if let Block::SwappedOut = blocks[block] {
                self.allocate_block(block, cpus)?;
            }
        } else {
            self.allocate_block(block, cpus)?;
        }

        let offset = (addr & (self.block_size - 1) as u64) as u32;
        //Ok(self.block_by_index(block).unwrap().as_ptr() as )
        //Ok(&mut self.vector()[offset as usize..(self.block_size as usize)])
        let block_size = self.block_size as usize;
        Ok(&mut self.vector()[offset as usize..block_size])

        //let offset = (self.block_by_index(block).unwrap().as_ptr() as usize + *addr as usize)
        //    & (self.block_size - 1);
        //Ok(&mut self.vector()[offset..(self.block_size as usize)])
    }

    #[cfg(all(feature = "std", feature = "bx_large_ram_file"))]
    fn read_block(&self, block: usize) -> Result<()> {
        let block_address = block * self.block_size;
        let chosen_block = self.block_by_index(block).unwrap();
        let overflow_file = self.overflow_file_mut();

        overflow_file.seek(SeekFrom::Start(block_address.try_into()?))?;
        overflow_file.read_exact(chosen_block)?;

        Ok(())
    }

    pub fn allocate_block<I: BxCpuIdTrait>(&self, block: usize, cpus: &[&BxCpuC<I>]) -> Result<()> {
        #[cfg(all(feature = "std", feature = "bx_large_ram_file"))]
        {
            let max_blocks = self.allocated / self.block_size;
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

                    for cpu in cpus {
                        used_for_tlb = cpu.check_addr_in_tlb_buffers(&buffer, buffer_end);
                    }

                    if !used_for_tlb {
                        break;
                    }
                }

                let address: usize = self.next_swapout_idx.get() + self.block_size;

                // Write swapped out block
                let overflow_file = &mut self.overflow_file_mut();
                overflow_file
                    // FIXME: don't unwrap
                    .seek(SeekFrom::Start(
                        address as u64
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
        }

        #[cfg(not(feature = "bx_large_ram_file"))]
        {
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

        Ok(())
    }

    pub fn dbg_fetch_mem<'a, I: BxCpuIdTrait>(
        &'a mut self,
        _cpu: BxCpuC<I>,
        addr: BxPhyAddress,
        mut len: u32,
        buf: &mut [u8],
        cpus: &[&BxCpuC<I>],
        a20_mask: A20Mask,
    ) -> Result<bool> {
        let mut a20_addr: BxPhyAddress = addr & a20_mask;
        let mut ret = true;
        let mut buf_offset = 0;

        while len > 0 {
            if a20_addr < self.len.try_into()? {
                // TODO: Check if its really index 0
                buf[buf_offset] = *self.get_vector(a20_addr, cpus)?.first().unwrap();
            } else if cfg!(feature = "bx_phy_address_long") && a20_addr > 0xffffffff {
                buf[buf_offset] = 0xff;
                ret = false;
            } else {
                buf[buf_offset] = 0xff;
                ret = false;
            }
            len -= 1;

            buf_offset += 1;
            // TODO: I'm not sure about 1
            a20_addr += 1;
        }

        Ok(ret)
    }

    #[cfg(any(feature = "bx_debugger", feature = "bx_gdb_stub"))]
    pub fn dbg_set_mem<I: BxCpuIdTrait>(
        _cpus: &[BxCpuC<I>],
        _addr: BxPhyAddress,
        _len: u32,
        _buf: &mut [u8],
    ) -> bool {
        unimplemented!()
    }

    #[cfg(any(feature = "bx_debugger", feature = "bx_gdb_stub"))]
    pub fn dbg_crc32(_addr1: BxPhyAddress, _addr2: BxPhyAddress, _crc: &[u32]) -> bool {
        unimplemented!()
    }

    ///
    /// Return a host address corresponding to the guest physical memory
    /// address (with A20 already applied), given that the calling
    /// code will perform an 'op' operation.  This address will be
    /// used for direct access to guest memory.
    /// Values of 'op' are { BX_READ, BX_WRITE, BX_EXECUTE, BX_RW }.
    ///
    ///
    /// The other assumption is that the calling code _only_ accesses memory
    /// directly within the page that encompasses the address requested.
    ///
    fn get_host_mem_addr<'a, I: BxCpuIdTrait>(
        &'a mut self,
        cpus: &[&BxCpuC<I>],
        addr: BxPhyAddress,
        rw: u32,
        a20_mask: A20Mask,
    ) -> Result<Option<&'a mut [u8]>> {
        let a20_addr = addr & a20_mask;

        let write = rw & 1 != 0;

        #[cfg(feature = "bx_support_monitor_mwait")]
        if write && Self::is_monitor(cpus, a20_addr & !0xfff, 0xfff) {
            // TODO: Consider actually returning error

            // Vetoed! Write monitored page!
            return Ok(None);
        }

        if !write {
            if addr < self.len.try_into()? {
                Ok(Some(self.get_vector(addr, cpus)?))
            } else {
                Ok(Some(&mut self.bogus()[a20_addr as usize & 0xfff..]))
            }
        } else if a20_addr >= self.len.try_into()? {
            Ok(None)
        } else {
            Ok(Some(self.get_vector(addr, cpus)?))
        }
    }

    fn write_physical_page<'a, I: BxCpuIdTrait>(
        &'a mut self,
        cpus: &[&BxCpuC<I>],
        page_write_stamp_table: &mut BxPageWriteStampTable,
        addr: BxPhyAddress,
        mut len: usize,
        data: &mut [u8],
        a20_mask: A20Mask,
    ) -> Result<()> {
        let mut a20_addr = addr & a20_mask;

        // Note: accesses should always be contained within a single page
        if (addr >> 12) != ((addr + len as u64 - 1) >> 12) {
            return Err(MemoryError::WritePhysicalPage { addr, len }.into());
        }

        #[cfg(feature = "bx_support_monitor_mwait")]
        Self::is_monitor(cpus, a20_addr, len.try_into()?);

        // TODO: When everything will work, add rust enums for that
        if a20_addr < self.len.try_into()? {
            // all of data is within limits of physical memory
            if len == 8 {
                page_write_stamp_table.dec_write_stamp_with_len(a20_addr, 8);
                write_host_qword_to_little_endian(
                    self.get_vector(a20_addr, cpus)?,
                    LittleEndian::read_u64(data),
                );
                return Ok(());
            } else if len == 4 {
                page_write_stamp_table.dec_write_stamp_with_len(a20_addr, 8);
                write_host_dword_to_little_endian(
                    self.get_vector(a20_addr, cpus)?,
                    LittleEndian::read_u32(data),
                );
                return Ok(());
            } else if len == 2 {
                page_write_stamp_table.dec_write_stamp_with_len(a20_addr, 8);
                write_host_word_to_little_endian(
                    self.get_vector(a20_addr, cpus)?,
                    LittleEndian::read_u16(data),
                );
                return Ok(());
            } else if len == 1 {
                page_write_stamp_table.dec_write_stamp_with_len(a20_addr, 8);
                self.get_vector(a20_addr, cpus)?[0] = data[0];
                return Ok(());
            }
            // len == other, just fall thru to special cases handling

            let mut data_ptr_offset = if cfg!(feature = "bx_little_endian") {
                0
            } else {
                len - 1
            };

            loop {
                if (len & 7) == 0 {
                    page_write_stamp_table.dec_write_stamp_with_len(a20_addr, 8);
                    write_host_qword_to_little_endian(
                        self.get_vector(a20_addr, cpus)?,
                        LittleEndian::read_u64(&data[data_ptr_offset..]),
                    );
                    len -= 8;
                    a20_addr += 8;

                    if cfg!(feature = "bx_little_endian") {
                        data_ptr_offset += 8;
                    } else {
                        data_ptr_offset -= 8
                    }

                    if len == 0 {
                        return Ok(());
                    }
                } else {
                    page_write_stamp_table.dec_write_stamp_with_len(a20_addr, 8);
                    self.get_vector(a20_addr, cpus)?[0] = data[0];

                    if len == 1 {
                        return Ok(());
                    }

                    len -= 1;
                    if cfg!(feature = "bx_little_endian") {
                        data_ptr_offset += 8;
                    } else {
                        data_ptr_offset -= 8
                    }
                }

                page_write_stamp_table.dec_write_stamp(a20_addr);
            }
        } else {
            tracing::debug!("Write outside the limits of physical memory ({a20_addr:#x}) (ignore)");
        }
        Ok(())
    }

    fn read_physical_page<'a, I: BxCpuIdTrait>(
        &'a mut self,
        cpus: &[&BxCpuC<I>],
        addr: BxPhyAddress,
        len: usize,
        data: &mut [u8],
        a20_mask: A20Mask,
    ) -> Result<()> {
        let a20_addr = addr & a20_mask;

        // Note: accesses should always be contained within a single page
        if (addr >> 12) != ((addr + len as u64 - 1) >> 12) {
            return Err(MemoryError::ReadPhysicalPage { addr, len }.into());
        }

        if a20_addr < self.len.try_into()? {
            // all of data is within limits of physical memory
            if len == 8 {
                let val = read_host_qword_to_little_endian(self.get_vector(a20_addr, cpus)?);
                LittleEndian::write_u64(data, val);
                return Ok(());
            } else if len == 4 {
                let val = read_host_dword_to_little_endian(self.get_vector(a20_addr, cpus)?);
                LittleEndian::write_u32(data, val);
                return Ok(());
            } else if len == 2 {
                let val = read_host_word_to_little_endian(self.get_vector(a20_addr, cpus)?);
                LittleEndian::write_u16(data, val);
                return Ok(());
            } else if len == 1 {
                let val = self.get_vector(a20_addr, cpus)?[0];
                data[0] = val;
                return Ok(());
            }
            // len == other, just fall thru to special cases handling

            let _data_ptr_offset = if cfg!(feature = "bx_little_endian") {
                0
            } else {
                len - 1
            };

            todo!()
        } else {
            // access outside limits of physical memory
            let bogus = self.bogus();
            bogus.fill(0xff);
        }
        todo!()
    }

    #[cfg(feature = "bx_support_monitor_mwait")]
    pub(super) fn is_monitor<I: BxCpuIdTrait>(
        cpus: &[&BxCpuC<I>],
        begin_addr: BxPhyAddress,
        len: u32,
    ) -> bool {
        cpus.iter().any(|cpu| cpu.is_monitor(begin_addr, len))
    }

    #[cfg(feature = "bx_support_monitor_mwait")]
    fn check_monitor<I: BxCpuIdTrait>(
        cpus: &mut [BxCpuC<I>],
        begin_addr: BxPhyAddress,
        len: u32,
    ) -> Result<()> {
        for cpu in cpus {
            cpu.check_monitor(begin_addr, len)?
        }
        Ok(())
    }
}
