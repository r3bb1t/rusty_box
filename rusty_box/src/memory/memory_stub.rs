#![allow(dead_code)]
#[cfg(feature = "alloc")]
use alloc::{vec, vec::Vec};
use byteorder::{ByteOrder, LittleEndian};
#[cfg(feature = "std")]
use tempfile::tempfile;

use super::{Block, BxMemoryStubC, MemoryError, Result};
use crate::cpu::cpuid::BxCpuIdTrait;

use crate::config::{BxPhyAddress, MAX_MEM_BLOCKS};
use crate::config::BxPhyAddress as A20Mask;
use crate::cpu::cpu::BxCpuC;
use crate::cpu::icache::BxPageWriteStampTable;
use crate::memory::memory_rusty_box::{bx_is_pci_hole_addr, bx_translate_gpa_to_linear, BIOSROMSZ, EXROMSIZE};
use crate::misc::bswap::{
    read_host_dword_to_little_endian, read_host_qword_to_little_endian,
    read_host_word_to_little_endian, write_host_dword_to_little_endian,
    write_host_qword_to_little_endian, write_host_word_to_little_endian,
};

use core::cell::{Cell, UnsafeCell};

#[cfg(feature = "std")]
use std::io::{Read, Seek, SeekFrom, Write};

#[inline]
fn is_power_of_2(x: usize) -> bool {
    (x & (x - 1)) == 0
}

const BX_MEM_VECTOR_ALIGN: usize = 4096;

impl BxMemoryStubC {
    #[cfg(feature = "alloc")]
    pub fn alloc_vector_aligned(bytes: usize, alignment: usize) -> (Vec<u8>, usize) {
        let test_mask: usize = alignment - 1;
        let actual_vector_size = bytes + test_mask;
        // Use alloc_zeroed with page alignment to avoid UEFI pool allocator
        // limitations on large contiguous allocations.
        let layout = alloc::alloc::Layout::from_size_align(actual_vector_size, alignment)
            .expect("invalid layout for memory vector");
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            alloc::alloc::handle_alloc_error(layout);
        }
        let actual_vector = unsafe { Vec::from_raw_parts(ptr, actual_vector_size, actual_vector_size) };
        let actual_vector_ptr = actual_vector.as_ptr() as usize;
        let masked: usize = ((actual_vector_ptr + test_mask) & !test_mask) - actual_vector_ptr;
        (actual_vector, masked)
    }

    pub fn get_memory_len(&self) -> usize {
        self.len
    }

    #[cfg(feature = "alloc")]
    pub fn create_and_init(guest: usize, host: usize, block_size: usize) -> Result<alloc::boxed::Box<Self>> {
        const ONE_MEGABYTE: usize = 1 << 20;

        if !host.is_multiple_of(ONE_MEGABYTE) || !guest.is_multiple_of(ONE_MEGABYTE) {
            return Err(MemoryError::MemorySizeIsNotAMultiplyOf1Megabyte.into());
        }

        if !is_power_of_2(block_size) {
            return Err(MemoryError::BlockSizeIsNotAPowerOfTwo(block_size).into());
        }

        let (mut actual_vector, vector_offset) =
            Self::alloc_vector_aligned(host + BIOSROMSZ + EXROMSIZE + 4096, BX_MEM_VECTOR_ALIGN);
        tracing::debug!(
            "allocated memory at {:p}. after alignment, vector={:p}, block_size = {}k",
            actual_vector.as_ptr(),
            actual_vector[vector_offset..].as_ptr(),
            block_size / 1024
        );

        let len = guest;
        let allocated = host;
        let rom_offset = host;
        let bogus_offset = host + BIOSROMSZ + EXROMSIZE;

        // Initialize ROM and bogus memory with 0xFF (matching C++ memset)
        let rom_start = vector_offset + rom_offset;
        let rom_end = rom_start + BIOSROMSZ + EXROMSIZE + 4096;
        if rom_end <= actual_vector.len() {
            actual_vector[rom_start..rom_end].fill(0xFF);
        }

        assert!((len / block_size) <= 0xffffffff);

        let num_blocks = len / block_size;
        assert!(num_blocks <= MAX_MEM_BLOCKS, "num_blocks {} exceeds MAX_MEM_BLOCKS", num_blocks);
        tracing::debug!("{}MB", len / (1024 * 1024));
        tracing::debug!("mem block size = {:8X}, blocks={}", block_size, num_blocks);

        // Allocate BxMemoryStubC on the heap to avoid stack overflow.
        // blocks_offsets is 262KB — too large for UEFI's 128KB stack.
        let layout = alloc::alloc::Layout::new::<Self>();
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) } as *mut Self;
        if ptr.is_null() {
            alloc::alloc::handle_alloc_error(layout);
        }

        let actual_vector_ptr = actual_vector.as_mut_ptr();
        let actual_vector_len = actual_vector.len();
        core::mem::forget(actual_vector);

        unsafe {
            core::ptr::addr_of_mut!((*ptr).actual_vector).write(actual_vector_ptr);
            core::ptr::addr_of_mut!((*ptr).actual_vector_len).write(actual_vector_len);
            core::ptr::addr_of_mut!((*ptr).len).write(len);
            core::ptr::addr_of_mut!((*ptr).allocated).write(allocated);
            core::ptr::addr_of_mut!((*ptr).block_size).write(block_size);
            core::ptr::addr_of_mut!((*ptr).num_blocks).write(num_blocks);
            core::ptr::addr_of_mut!((*ptr).vector_offset).write(vector_offset);
            core::ptr::addr_of_mut!((*ptr).rom_offset).write(rom_offset);
            core::ptr::addr_of_mut!((*ptr).bogus_offset).write(bogus_offset);
            // Initialize blocks to SwappedOut in-place on heap
            let blocks = &mut *(*ptr).blocks_offsets.get();
            for b in blocks.iter_mut().take(num_blocks) {
                *b = Block::SwappedOut;
            }
            #[cfg(feature = "std")]
            {
                let overflow_file = tempfile().map_err(MemoryError::UnableToCreateTempFile)?;
                core::ptr::addr_of_mut!((*ptr).overflow_file).write(UnsafeCell::new(overflow_file));
            }
            Ok(alloc::boxed::Box::from_raw(ptr))
        }
    }

    /// Create a memory stub from an externally-provided buffer (no-alloc path).
    ///
    /// # Safety
    /// `ptr` must point to a valid, exclusively-owned buffer of at least `len` bytes
    /// that remains valid for the lifetime of this struct.
    pub unsafe fn create_from_raw(
        ptr: *mut u8,
        len: usize,
        guest: usize,
        host: usize,
        block_size: usize,
    ) -> Result<Self> {
        let vector_offset = 0; // caller is responsible for alignment
        let rom_offset = host;
        let bogus_offset = host + BIOSROMSZ + EXROMSIZE;
        let num_blocks = guest / block_size;
        assert!(num_blocks <= MAX_MEM_BLOCKS, "num_blocks {} exceeds MAX_MEM_BLOCKS", num_blocks);

        #[cfg(feature = "std")]
        let overflow_file = tempfile().map_err(MemoryError::UnableToCreateTempFile)?;
        Ok(Self {
            actual_vector: ptr,
            actual_vector_len: len,
            len: guest,
            allocated: host,
            block_size,
            blocks_offsets: UnsafeCell::new([Block::SwappedOut; MAX_MEM_BLOCKS]),
            num_blocks,
            vector_offset,
            rom_offset,
            bogus_offset,
            used_blocks: Cell::new(0),
            apic_scratch: [0u8; 4096],
            next_swapout_idx: Cell::new(0),
            #[cfg(feature = "std")]
            overflow_file: UnsafeCell::new(overflow_file),
        })
    }

    pub(super) fn get_vector<'a, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
        &'a mut self,
        addr: BxPhyAddress,
        _cpus: &[&BxCpuC<'_, I, T>],
    ) -> Result<&'a mut [u8]> {
        // Memory is contiguous in actual_vector[vector_offset..].
        // Use the full physical address as the index into the flat memory view.
        //
        // BUG FIX: The old code used `addr & (block_size - 1)` (within-block offset)
        // which mapped ALL addresses to block 0's range. For example, a write to
        // physical 0x9FF00 with block_size=128KB would go to offset 0x1FF00 instead
        // of the correct 0x9FF00. This caused the BIOS IPL table (at 0x9FF00) to be
        // written to the wrong address, and any data above 128KB to be misplaced.
        let addr_usize = addr as usize;
        let vo = self.vector_offset;
        let bo = self.bogus_offset;
        let ptr = self.actual_vector;
        let len = self.actual_vector_len;
        let start = vo + addr_usize;
        // SAFETY: actual_vector is valid for actual_vector_len bytes (invariant of the struct)
        let av = unsafe { core::slice::from_raw_parts_mut(ptr, len) };
        if start < av.len() {
            Ok(&mut av[start..])
        } else {
            Ok(&mut av[bo..])
        }
    }

    #[cfg(feature = "std")]
    fn read_block(&self, block: usize) -> Result<()> {
        let block_address = block * self.block_size;
        let chosen_block = self.block_by_index(block)
            .ok_or(MemoryError::Internal("block_by_index returned None during read"))?;
        let overflow_file = self.overflow_file_mut();

        overflow_file.seek(SeekFrom::Start(block_address.try_into()?))?;
        overflow_file.read_exact(chosen_block)?;

        Ok(())
    }

    pub fn allocate_block<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(&self, block: usize, cpus: &[&BxCpuC<'_, I, T>]) -> Result<()> {
        #[cfg(feature = "std")]
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

                    let (buffer_offset, buffer_end) = {
                        let Block::Block { offset } = buffer else {
                            unreachable!("expected Block::Block variant for allocated memory")
                        };
                        (offset, offset + self.block_size)
                    };

                    for cpu in cpus {
                        used_for_tlb = cpu.check_addr_in_tlb_buffers(buffer_offset, buffer_end);
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
                    .seek(SeekFrom::Start(address as u64))
                    .map_err(|e| MemoryError::CantSeekToAddressOverflowFile(address, e))?;

                overflow_file
                    .write_all(self.block_by_index(self.next_swapout_idx.get())
                        .ok_or(MemoryError::Internal("block_by_index returned None during swap-out"))?)
                    .map_err(|e| MemoryError::FailedToWriteToOverflowFIle(address, e))?;

                // Mark swapped out block
                self.blocks_offsets()[self.next_swapout_idx.get()] = Block::SwappedOut;
                // TODO: Continue here
                self.blocks_offsets()[block] = buffer;

                self.read_block(block)?;
                tracing::trace!(
                    "allocate_block: block={:#x}, replaced {:#x}",
                    block,
                    self.next_swapout_idx.get()
                )
            }
        }

        Ok(())
    }

    pub fn dbg_fetch_mem<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
        &mut self,
        _cpu: BxCpuC<I, T>,
        addr: BxPhyAddress,
        mut len: u32,
        buf: &mut [u8],
        cpus: &[&BxCpuC<I, T>],
        a20_mask: A20Mask,
    ) -> Result<bool> {
        let mut a20_addr: BxPhyAddress = addr & a20_mask;
        let mut ret = true;
        let mut buf_offset = 0;

        while len > 0 {
            if a20_addr < self.len.try_into()? {
                buf[buf_offset] = *self.get_vector(a20_addr, cpus)?
                    .first()
                    .ok_or(MemoryError::Internal("get_vector returned empty slice"))?;
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

    /// Write bytes to physical memory for debugger (Bochs memory.cc dbg_set_mem).
    /// Writes directly to the flat memory vector bypassing device handlers.
    #[cfg(any(feature = "bx_debugger", feature = "bx_gdb_stub"))]
    pub fn dbg_set_mem(&mut self, addr: BxPhyAddress, len: u32, buf: &[u8]) -> bool {
        let vo = self.vector_offset;
        let av = self.actual_vector_mut();
        for i in 0..len as usize {
            if i >= buf.len() {
                break;
            }
            let phys = addr as usize + i;
            let idx = vo + phys;
            if idx < av.len() {
                av[idx] = buf[i];
            }
        }
        true
    }

    /// Compute CRC32 over physical memory range for debugger (Bochs memory.cc dbg_crc32).
    #[cfg(any(feature = "bx_debugger", feature = "bx_gdb_stub"))]
    pub fn dbg_crc32(&self, addr1: BxPhyAddress, addr2: BxPhyAddress, crc: &mut u32) -> bool {
        let vo = self.vector_offset;
        let av = self.actual_vector_slice();
        let mut c = 0xFFFF_FFFFu32;
        let mut addr = addr1;
        while addr <= addr2 {
            let idx = vo + addr as usize;
            let byte = if idx < av.len() {
                av[idx]
            } else {
                0xFF
            };
            for bit in 0..8u32 {
                let b = ((c ^ (byte as u32 >> bit)) & 1) != 0;
                c >>= 1;
                if b {
                    c ^= 0xEDB88320; // CRC32 polynomial
                }
            }
            addr += 1;
        }
        *crc = c;
        true
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
    fn get_host_mem_addr<'a, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
        &'a mut self,
        cpus: &[&BxCpuC<I, T>],
        addr: BxPhyAddress,
        rw: u32,
        a20_mask: A20Mask,
    ) -> Result<Option<&'a mut [u8]>> {
        let a20_addr = addr & a20_mask;

        let write = rw & 1 != 0;

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

    pub(crate) fn write_physical_page<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
        &mut self,
        cpus: &[&BxCpuC<I, T>],
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

        Self::is_monitor(cpus, a20_addr, len.try_into()?);

        if bx_is_pci_hole_addr(a20_addr) {
            // PCI MMIO hole — writes are silently dropped
            return Ok(());
        }
        if bx_translate_gpa_to_linear(a20_addr) < self.len.try_into()? {
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

            let mut data_ptr_offset = if cfg!(target_endian = "little") {
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

                    if cfg!(target_endian = "little") {
                        data_ptr_offset += 8;
                    } else {
                        data_ptr_offset -= 8
                    }

                    if len == 0 {
                        return Ok(());
                    }
                } else {
                    // Single byte write — Bochs misc_mem.cc: *data_ptr++
                    page_write_stamp_table.dec_write_stamp_with_len(a20_addr, 1);
                    self.get_vector(a20_addr, cpus)?[0] = data[data_ptr_offset];

                    if len == 1 {
                        return Ok(());
                    }

                    len -= 1;
                    a20_addr += 1;
                    data_ptr_offset += 1;
                }

                page_write_stamp_table.dec_write_stamp(a20_addr);
            }
        }
        Ok(())
    }

    pub(crate) fn read_physical_page<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
        &mut self,
        cpus: &[&BxCpuC<I, T>],
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

        if bx_is_pci_hole_addr(a20_addr) {
            // PCI MMIO hole — reads return 0xFF
            data[..len].fill(0xff);
            return Ok(());
        }
        if bx_translate_gpa_to_linear(a20_addr) < self.len.try_into()? {
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
            // Handle non-standard lengths by copying byte-by-byte or in chunks
            let mem_vector = self.get_vector(a20_addr, cpus)?;

            #[cfg(target_endian = "little")]
            {
                // For little endian, copy directly
                let mut remaining = len;
                let mut offset = 0;
                let mut addr_offset = 0;

                // Read in chunks of 8 bytes if possible
                while remaining >= 8 {
                    let val =
                        read_host_qword_to_little_endian(&mem_vector[addr_offset..addr_offset + 8]);
                    LittleEndian::write_u64(&mut data[offset..offset + 8], val);
                    remaining -= 8;
                    offset += 8;
                    addr_offset += 8;
                }

                // Handle remaining bytes
                if remaining > 0 {
                    data[offset..offset + remaining]
                        .copy_from_slice(&mem_vector[addr_offset..addr_offset + remaining]);
                }
            }

            #[cfg(target_endian = "big")]
            {
                // For big endian, copy in reverse order
                let mut remaining = len;
                let mut data_ptr_offset = len - 1;
                let mut addr_offset = 0;

                while remaining > 0 {
                    data[data_ptr_offset] = mem_vector[addr_offset];
                    remaining -= 1;
                    if remaining > 0 {
                        data_ptr_offset -= 1;
                        addr_offset += 1;
                    }
                }
            }

            Ok(())
        } else {
            // access outside limits of physical memory
            let bogus = self.bogus();
            let fill_len = len.min(bogus.len());
            data[..fill_len].copy_from_slice(&bogus[..fill_len]);
            if len > fill_len {
                data[fill_len..].fill(0xff);
            }
            Ok(())
        }
    }

    pub(super) fn is_monitor<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
        cpus: &[&BxCpuC<I, T>],
        begin_addr: BxPhyAddress,
        len: u32,
    ) -> bool {
        cpus.iter().any(|cpu| cpu.is_monitor(begin_addr, len))
    }

    fn check_monitor<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
        cpus: &mut [BxCpuC<I, T>],
        begin_addr: BxPhyAddress,
        len: u32,
    ) -> Result<()> {
        for cpu in cpus {
            cpu.check_monitor(begin_addr, len)?
        }
        Ok(())
    }
}
