#![allow(dead_code)]
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
#[cfg(feature = "std")]
use tempfile::tempfile;

use super::{Block, BxMemoryStubC, CpuTlbPin, MemoryError, Result};
#[cfg(feature = "std")]
use super::{MemorySnapshotGeometry, MemorySnapshotResidency};
use crate::config::BxPhyAddress as A20Mask;
use crate::config::{BxPhyAddress, MAX_MEM_BLOCKS};
use crate::memory::memory_rusty_box::{
    bx_guest_ram_span, bx_is_pci_hole_addr, BIOSROMSZ, EXROMSIZE,
};

use core::cell::{Cell, UnsafeCell};

#[cfg(feature = "std")]
use std::io::{Read, Seek, SeekFrom, Write};

#[inline]
fn is_power_of_2(x: usize) -> bool {
    (x & (x - 1)) == 0
}

const BX_MEM_VECTOR_ALIGN: usize = 4096;

#[cfg(feature = "std")]
const SNAPSHOT_IO_CHUNK: usize = 64 * 1024;

#[cfg(feature = "std")]
#[inline]
fn snapshot_invalid(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

#[cfg(feature = "std")]
#[inline]
fn snapshot_other(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, message)
}

#[cfg(feature = "alloc")]
struct OwnedAlignedBuffer {
    ptr: core::ptr::NonNull<u8>,
    len: usize,
    layout: alloc::alloc::Layout,
}

#[cfg(feature = "alloc")]
impl OwnedAlignedBuffer {
    fn allocate(bytes: usize, alignment: usize) -> Result<Self> {
        let layout = alloc::alloc::Layout::from_size_align(bytes, alignment)
            .map_err(|_| MemoryError::UnableToAllocateGuestMemory(bytes))?;
        let ptr = core::ptr::NonNull::new(unsafe { alloc::alloc::alloc_zeroed(layout) })
            .ok_or(MemoryError::UnableToAllocateGuestMemory(bytes))?;
        Ok(Self {
            ptr,
            len: bytes,
            layout,
        })
    }

    fn into_raw_parts(self) -> (*mut u8, usize, alloc::alloc::Layout) {
        let owned = core::mem::ManuallyDrop::new(self);
        (owned.ptr.as_ptr(), owned.len, owned.layout)
    }
}

#[cfg(feature = "alloc")]
impl core::ops::Deref for OwnedAlignedBuffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        unsafe { core::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

#[cfg(feature = "alloc")]
impl core::ops::DerefMut for OwnedAlignedBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { core::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

#[cfg(feature = "alloc")]
impl Drop for OwnedAlignedBuffer {
    fn drop(&mut self) {
        unsafe { alloc::alloc::dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

impl BxMemoryStubC {

    pub fn get_memory_len(&self) -> usize {
        self.len
    }

    #[cfg(feature = "alloc")]
    pub fn create_and_init(
        guest: usize,
        host: usize,
        block_size: usize,
    ) -> Result<alloc::boxed::Box<Self>> {
        const ONE_MEGABYTE: usize = 1 << 20;

        if !host.is_multiple_of(ONE_MEGABYTE) || !guest.is_multiple_of(ONE_MEGABYTE) {
            return Err(MemoryError::MemorySizeIsNotAMultiplyOf1Megabyte.into());
        }

        if !is_power_of_2(block_size) {
            return Err(MemoryError::BlockSizeIsNotAPowerOfTwo(block_size).into());
        }
        #[cfg(not(feature = "std"))]
        if host < guest {
            return Err(MemoryError::InsufficientRam.into());
        }

        let resident_backing_len = if host < guest {
            host.checked_add(block_size - 1)
                .map(|bytes| bytes & !(block_size - 1))
                .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?
        } else {
            host
        };
        if guest != 0 && resident_backing_len == 0 {
            return Err(MemoryError::InsufficientRam.into());
        }

        let aux_len = BIOSROMSZ
            .checked_add(EXROMSIZE)
            .and_then(|n| n.checked_add(4096))
            .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?;
        let total_len = resident_backing_len
            .checked_add(aux_len)
            .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?;
        let mut actual_vector = OwnedAlignedBuffer::allocate(total_len, BX_MEM_VECTOR_ALIGN)?;
        let vector_offset = 0;
        tracing::debug!(
            "allocated memory at {:p}. after alignment, vector={:p}, block_size = {}k",
            actual_vector.as_ptr(),
            actual_vector[vector_offset..].as_ptr(),
            block_size / 1024
        );

        let len = guest;
        let allocated = host;
        let rom_offset = resident_backing_len;
        let bogus_offset = resident_backing_len
            .checked_add(BIOSROMSZ)
            .and_then(|n| n.checked_add(EXROMSIZE))
            .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?;

        let rom_start = vector_offset + rom_offset;
        actual_vector[rom_start..].fill(0xFF);

        let num_blocks = len
            .checked_add(block_size - 1)
            .ok_or(MemoryError::UnableToAllocateGuestMemory(len))?
            / block_size;
        if num_blocks > MAX_MEM_BLOCKS {
            return Err(MemoryError::UnableToAllocateGuestMemory(len).into());
        }
        tracing::debug!("{}MB", len / (1024 * 1024));
        tracing::debug!("mem block size = {:8X}, blocks={}", block_size, num_blocks);

        let mut smc_stamps = Vec::new();
        smc_stamps
            .try_reserve_exact(crate::cpu::icache::SMC_STAMP_ENTRIES)
            .map_err(|_| {
                MemoryError::UnableToAllocateGuestMemory(
                    crate::cpu::icache::SMC_STAMP_ENTRIES * core::mem::size_of::<u32>(),
                )
            })?;
        smc_stamps.resize(crate::cpu::icache::SMC_STAMP_ENTRIES, 0u32);
        #[cfg(feature = "std")]
        let overflow_file = {
            let file = tempfile().map_err(MemoryError::UnableToCreateTempFile)?;
            if host < guest {
                file.set_len(u64::try_from(guest).map_err(|_| MemoryError::InsufficientRam)?)?;
            }
            file
        };
        // blocks_offsets is 262KB — too large for UEFI's 128KB stack.
        let layout = alloc::alloc::Layout::new::<Self>();
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) } as *mut Self;
        if ptr.is_null() {
            return Err(MemoryError::UnableToAllocateGuestMemory(layout.size()).into());
        }

        let (actual_vector_ptr, actual_vector_len, actual_vector_layout) =
            actual_vector.into_raw_parts();

        unsafe {
            core::ptr::addr_of_mut!((*ptr).actual_vector).write(actual_vector_ptr);
            core::ptr::addr_of_mut!((*ptr).actual_vector_len).write(actual_vector_len);
            core::ptr::addr_of_mut!((*ptr).actual_vector_layout).write(Some(actual_vector_layout));
            core::ptr::addr_of_mut!((*ptr).len).write(len);
            core::ptr::addr_of_mut!((*ptr).allocated).write(allocated);
            core::ptr::addr_of_mut!((*ptr).resident_backing_len).write(resident_backing_len);
            core::ptr::addr_of_mut!((*ptr).block_size).write(block_size);
            core::ptr::addr_of_mut!((*ptr).num_blocks).write(num_blocks);
            core::ptr::addr_of_mut!((*ptr).vector_offset).write(vector_offset);
            core::ptr::addr_of_mut!((*ptr).rom_offset).write(rom_offset);
            core::ptr::addr_of_mut!((*ptr).bogus_offset).write(bogus_offset);
            // Initialize blocks to SwappedOut in-place on heap
            let blocks = &mut *(*ptr).blocks_offsets.get();
            if allocated >= len {
                for (guest_block, entry) in blocks.iter_mut().take(num_blocks).enumerate() {
                    *entry = Block::Block {
                        offset: guest_block * block_size,
                    };
                }
                core::ptr::addr_of_mut!((*ptr).used_blocks).write(Cell::new(num_blocks));
            } else {
                for entry in blocks.iter_mut().take(num_blocks) {
                    *entry = Block::SwappedOut;
                }
                core::ptr::addr_of_mut!((*ptr).used_blocks).write(Cell::new(0));
            }
            core::ptr::addr_of_mut!((*ptr).next_swapout_idx).write(Cell::new(0));
            // Full residency lays blocks out as an identity map above.
            core::ptr::addr_of_mut!((*ptr).identity_map).write(Cell::new(allocated >= len));
            #[cfg(feature = "std")]
            core::ptr::addr_of_mut!((*ptr).overflow_file).write(UnsafeCell::new(overflow_file));
            // Machine-wide SMC write-stamp table (Bochs icache.h
            // bxPageWriteStampTable ctor allocates + resetWriteStamps).
            core::ptr::addr_of_mut!((*ptr).smc_stamps).write(smc_stamps);
            core::ptr::addr_of_mut!((*ptr).smc_pending).write(
                [crate::cpu::icache::PendingSmc::default(); crate::cpu::icache::SMC_PENDING_CAP],
            );
            core::ptr::addr_of_mut!((*ptr).smc_pending_len).write(0);
            core::ptr::addr_of_mut!((*ptr).smc_seq_next).write(0);
            core::ptr::addr_of_mut!((*ptr).smc_overflow_seq).write(0);
            Ok(alloc::boxed::Box::from_raw(ptr))
        }
    }

    /// Create a memory stub from an externally-provided buffer (no-alloc path).
    ///
    /// # Safety
    /// `ptr` must be a non-null, 4096-byte-aligned, valid, exclusively-owned
    /// buffer of `len` bytes that outlives the returned stub.
    pub unsafe fn create_from_raw(
        ptr: *mut u8,
        len: usize,
        guest: usize,
        host: usize,
        block_size: usize,
    ) -> Result<Self> {
        if !is_power_of_2(block_size) {
            return Err(MemoryError::BlockSizeIsNotAPowerOfTwo(block_size).into());
        }
        #[cfg(not(feature = "std"))]
        if host < guest {
            return Err(MemoryError::InsufficientRam.into());
        }
        if ptr.is_null() || (ptr as usize & (BX_MEM_VECTOR_ALIGN - 1)) != 0 {
            return Err(MemoryError::Internal("raw memory must be 4K aligned").into());
        }
        let resident_backing_len = if host < guest {
            host.checked_add(block_size - 1)
                .map(|bytes| bytes & !(block_size - 1))
                .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?
        } else {
            host
        };
        if guest != 0 && resident_backing_len == 0 {
            return Err(MemoryError::InsufficientRam.into());
        }

        let aux_len = BIOSROMSZ
            .checked_add(EXROMSIZE)
            .and_then(|n| n.checked_add(4096))
            .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?;
        if len
            < resident_backing_len
                .checked_add(aux_len)
                .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?
        {
            return Err(MemoryError::UnableToAllocateGuestMemory(host).into());
        }
        let vector_offset = 0;
        let rom_offset = resident_backing_len;
        let bogus_offset = resident_backing_len
            .checked_add(BIOSROMSZ)
            .and_then(|n| n.checked_add(EXROMSIZE))
            .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?;
        let num_blocks = guest
            .checked_add(block_size - 1)
            .ok_or(MemoryError::UnableToAllocateGuestMemory(guest))?
            / block_size;
        if num_blocks > MAX_MEM_BLOCKS {
            return Err(MemoryError::UnableToAllocateGuestMemory(guest).into());
        }

        #[cfg(feature = "std")]
        let overflow_file = {
            let file = tempfile().map_err(MemoryError::UnableToCreateTempFile)?;
            if host < guest {
                file.set_len(u64::try_from(guest).map_err(|_| MemoryError::InsufficientRam)?)?;
            }
            file
        };
        #[cfg(feature = "alloc")]
        let smc_stamps = {
            let mut v = Vec::new();
            v.try_reserve_exact(crate::cpu::icache::SMC_STAMP_ENTRIES)
                .map_err(|_| {
                    MemoryError::UnableToAllocateGuestMemory(
                        crate::cpu::icache::SMC_STAMP_ENTRIES * core::mem::size_of::<u32>(),
                    )
                })?;
            v.resize(crate::cpu::icache::SMC_STAMP_ENTRIES, 0u32);
            v
        };
        #[cfg(not(feature = "alloc"))]
        let smc_stamps = [0u32; crate::cpu::icache::SMC_STAMP_ENTRIES];
        let mut blocks = [Block::SwappedOut; MAX_MEM_BLOCKS];
        let used_blocks = if host >= guest {
            for (guest_block, entry) in blocks.iter_mut().take(num_blocks).enumerate() {
                *entry = Block::Block {
                    offset: guest_block * block_size,
                };
            }
            num_blocks
        } else {
            0
        };
        Ok(Self {
            actual_vector: ptr,
            actual_vector_len: len,
            actual_vector_layout: None,
            len: guest,
            allocated: host,
            resident_backing_len,
            block_size,
            blocks_offsets: UnsafeCell::new(blocks),
            num_blocks,
            vector_offset,
            rom_offset,
            bogus_offset,
            used_blocks: Cell::new(used_blocks),
            smc_stamps,
            smc_pending: [crate::cpu::icache::PendingSmc::default();
                crate::cpu::icache::SMC_PENDING_CAP],
            smc_pending_len: 0,
            smc_seq_next: 0,
            smc_overflow_seq: 0,
            apic_scratch: [0u8; 4096],
            next_swapout_idx: Cell::new(0),
            // Full residency lays blocks out as an identity map above.
            identity_map: Cell::new(host >= guest),
            #[cfg(feature = "std")]
            overflow_file: UnsafeCell::new(overflow_file),
        })
    }

    // ── Machine-wide SMC write-stamp table ─────────────────────────────────
    // Bochs icache.h bxPageWriteStampTable: ONE shared instance per machine.
    // Trace creation by any cpu marks lines here; any write hitting marked
    // lines must invalidate EVERY cpu's icache (Bochs icache.cc handleSMC).

    /// Bochs icache.h `bxPageWriteStampTable::markICacheMask`.
    #[inline]
    pub(crate) fn smc_mark_icache_mask(&mut self, p_addr: BxPhyAddress, mask: u32) {
        self.smc_stamps[crate::cpu::icache::smc_page_index(p_addr)] |= mask;
    }

    /// Return whether any cached instruction line overlaps this single-page
    /// physical range. Bulk writers use this non-mutating probe to fall back
    /// to scalar ordering before consuming externally visible input.
    #[inline]
    pub(crate) fn smc_range_has_stamps(&self, p_addr: BxPhyAddress, len: u32) -> bool {
        let stamps = self.smc_stamps[crate::cpu::icache::smc_page_index(p_addr)];
        stamps != 0 && stamps & crate::cpu::icache::smc_cache_line_mask(p_addr, len) != 0
    }

    /// Bochs icache.h `bxPageWriteStampTable::decWriteStamp(pAddr, len)`:
    /// check a write against the stamp table; on a hit, clear the lines and
    /// queue the invalidation for every cpu (Bochs calls `handleSMC`
    /// synchronously; the emulator drains the queue at slice boundaries, and
    /// cpu-context writers apply it to themselves immediately via their
    /// `smc_seq_seen` watermark).
    #[inline]
    pub(crate) fn smc_dec_write_stamp(&mut self, p_addr: BxPhyAddress, len: u32) {
        let index = crate::cpu::icache::smc_page_index(p_addr);
        let stamps = self.smc_stamps[index];
        if stamps == 0 {
            return;
        }
        let mask = crate::cpu::icache::smc_cache_line_mask(p_addr, len);
        if stamps & mask == 0 {
            return;
        }
        self.smc_stamps[index] = stamps & !mask;
        self.smc_push_pending(p_addr, mask);
    }

    /// Bochs icache.h `bxPageWriteStampTable::decWriteStamp(pAddr)` — the
    /// whole-page variant used by handler-path and DMA writes (`handleSMC`
    /// with mask 0xffffffff).
    #[inline]
    pub(crate) fn smc_dec_write_stamp_page(&mut self, p_addr: BxPhyAddress) {
        let index = crate::cpu::icache::smc_page_index(p_addr);
        if self.smc_stamps[index] == 0 {
            return;
        }
        self.smc_stamps[index] = 0;
        self.smc_push_pending(p_addr, u32::MAX);
    }

    fn smc_push_pending(&mut self, p_addr: BxPhyAddress, mask: u32) {
        if self.smc_pending_len < crate::cpu::icache::SMC_PENDING_CAP {
            self.smc_pending[self.smc_pending_len] =
                crate::cpu::icache::PendingSmc { p_addr, mask };
            self.smc_pending_len += 1;
        } else {
            // Queue full: every cpu that has not caught up past this event
            // must do a full icache flush instead (conservative, correct).
            self.smc_overflow_seq = self.smc_seq_next + 1;
        }
        self.smc_seq_next += 1;
    }

    /// Sequence number the next SMC event will get. A cpu whose
    /// `smc_seq_seen` watermark is below this has invalidations to apply.
    #[inline]
    pub(crate) fn smc_seq_next(&self) -> u64 {
        self.smc_seq_next
    }

    /// Events a watermark of `since` has not seen yet.
    /// Returns `(needs_full_flush, new_events)`.
    #[inline]
    pub(crate) fn smc_pending_since(
        &self,
        since: u64,
    ) -> (bool, &[crate::cpu::icache::PendingSmc]) {
        let needs_full_flush = since < self.smc_overflow_seq;
        let base = self.smc_seq_next - self.smc_pending_len as u64;
        let start = since.saturating_sub(base) as usize;
        (
            needs_full_flush,
            &self.smc_pending[start.min(self.smc_pending_len)..self.smc_pending_len],
        )
    }

    /// Drop drained events. Called by the emulator once every cpu's
    /// watermark has caught up (sequence numbers stay monotonic).
    #[inline]
    pub(crate) fn smc_clear_pending(&mut self) {
        self.smc_pending_len = 0;
    }

    /// True when SMC events are queued. An empty queue means every cpu is
    /// caught up (the drain only clears it after catching every cpu up), so
    /// the per-slice drain can early-out on a single load.
    #[inline]
    pub(crate) fn smc_has_pending(&self) -> bool {
        self.smc_pending_len != 0
    }

    /// Bochs icache.h `bxPageWriteStampTable::resetWriteStamps` — hardware
    /// reset only (every cpu's icache is flushed there too).
    pub(crate) fn smc_reset_stamps(&mut self) {
        self.smc_stamps.fill(0);
        self.smc_pending_len = 0;
    }

    #[cfg(feature = "std")]
    fn snapshot_expected_num_blocks(&self) -> std::io::Result<usize> {
        let last_byte = self
            .block_size
            .checked_sub(1)
            .ok_or_else(|| snapshot_invalid("snapshot block size is zero"))?;
        self.len
            .checked_add(last_byte)
            .ok_or_else(|| snapshot_invalid("snapshot block count overflow"))
            .map(|bytes| bytes / self.block_size)
    }

    #[cfg(feature = "std")]
    #[inline]
    fn snapshot_resident_capacity(&self) -> usize {
        // Full backing has an identity resident entry for every logical guest
        // block, including a partial final block. Swapped backing may use only
        // complete host slots because eviction always exchanges a full block.
        if self.allocated >= self.len {
            self.num_blocks
        } else {
            (self.resident_backing_len / self.block_size).min(self.num_blocks)
        }
    }

    #[cfg(feature = "std")]
    fn snapshot_logical_block_len(&self, guest_block: usize) -> std::io::Result<usize> {
        if guest_block >= self.num_blocks {
            return Err(snapshot_invalid("snapshot guest block is out of range"));
        }
        let start = guest_block
            .checked_mul(self.block_size)
            .ok_or_else(|| snapshot_invalid("snapshot guest block offset overflow"))?;
        if start >= self.len {
            return Err(snapshot_invalid("snapshot guest block starts beyond guest RAM"));
        }
        Ok((self.len - start).min(self.block_size))
    }

    #[cfg(feature = "std")]
    fn snapshot_slot_offset(&self, slot: usize, logical_len: usize) -> std::io::Result<usize> {
        if slot >= self.snapshot_resident_capacity() {
            return Err(snapshot_invalid("snapshot resident slot is out of range"));
        }
        if logical_len > self.block_size {
            return Err(snapshot_invalid("snapshot logical block exceeds slot"));
        }
        let offset = slot
            .checked_mul(self.block_size)
            .ok_or_else(|| snapshot_invalid("snapshot resident slot offset overflow"))?;
        let end = offset
            .checked_add(logical_len)
            .ok_or_else(|| snapshot_invalid("snapshot resident slot length overflow"))?;
        if end > self.resident_backing_len {
            return Err(snapshot_invalid("snapshot resident slot exceeds host backing"));
        }
        self.vector_offset
            .checked_add(end)
            .filter(|end| *end <= self.actual_vector_len)
            .ok_or_else(|| snapshot_invalid("snapshot resident slot exceeds backing buffer"))?;
        Ok(offset)
    }

    #[cfg(feature = "std")]
    fn snapshot_resident_block(&self, slot: usize, len: usize) -> std::io::Result<&[u8]> {
        let offset = self.snapshot_slot_offset(slot, len)?;
        let start = self
            .vector_offset
            .checked_add(offset)
            .ok_or_else(|| snapshot_invalid("snapshot resident block offset overflow"))?;
        Ok(unsafe { core::slice::from_raw_parts(self.actual_vector.add(start), len) })
    }

    #[cfg(feature = "std")]
    fn snapshot_resident_block_mut(
        &mut self,
        slot: usize,
        len: usize,
    ) -> std::io::Result<&mut [u8]> {
        let offset = self.snapshot_slot_offset(slot, len)?;
        let start = self
            .vector_offset
            .checked_add(offset)
            .ok_or_else(|| snapshot_invalid("snapshot resident block offset overflow"))?;
        Ok(unsafe { core::slice::from_raw_parts_mut(self.actual_vector.add(start), len) })
    }
    /// Return the configured block geometry without exposing host backing.
    #[cfg(feature = "std")]
    pub(super) fn snapshot_geometry(&self) -> MemorySnapshotGeometry {
        MemorySnapshotGeometry {
            guest_len: self.len as u64,
            host_ram_len: self.allocated as u64,
            block_size: self.block_size as u64,
            num_blocks: self.num_blocks as u32,
            resident_capacity: self.snapshot_resident_capacity() as u32,
            used_blocks: self.used_blocks.get() as u32,
            next_swapout_guest_block: self.next_swapout_idx.get() as u32,
        }
    }

    /// Describe one guest block's current backing without making it resident.
    #[cfg(feature = "std")]
    pub(super) fn snapshot_residency(
        &self,
        guest_block: u32,
    ) -> std::io::Result<MemorySnapshotResidency> {
        let guest_block = usize::try_from(guest_block)
            .map_err(|_| snapshot_invalid("snapshot guest block conversion failed"))?;
        let logical_len = self.snapshot_logical_block_len(guest_block)?;
        match self.blocks_offsets()[guest_block] {
            Block::SwappedOut => Ok(MemorySnapshotResidency::Swapped),
            Block::Block { offset } => {
                if offset % self.block_size != 0 {
                    return Err(snapshot_invalid("snapshot resident block offset is unaligned"));
                }
                let slot = offset / self.block_size;
                self.snapshot_slot_offset(slot, logical_len)?;
                Ok(MemorySnapshotResidency::Resident {
                    slot: u32::try_from(slot)
                        .map_err(|_| snapshot_invalid("snapshot resident slot conversion failed"))?,
                })
            }
        }
    }

    /// Stream one logical guest block in GPA order without changing residency.
    #[cfg(feature = "std")]
    pub(super) fn write_snapshot_block<W: Write>(
        &self,
        guest_block: u32,
        out: &mut W,
    ) -> std::io::Result<()> {
        let guest_block = usize::try_from(guest_block)
            .map_err(|_| snapshot_invalid("snapshot guest block conversion failed"))?;
        let logical_len = self.snapshot_logical_block_len(guest_block)?;
        match self.snapshot_residency(
            u32::try_from(guest_block)
                .map_err(|_| snapshot_invalid("snapshot guest block conversion failed"))?,
        )? {
            MemorySnapshotResidency::Resident { slot } => {
                let slot = usize::try_from(slot)
                    .map_err(|_| snapshot_invalid("snapshot resident slot conversion failed"))?;
                out.write_all(self.snapshot_resident_block(slot, logical_len)?)
            }
            MemorySnapshotResidency::Swapped => {
                let offset = guest_block
                    .checked_mul(self.block_size)
                    .ok_or_else(|| snapshot_invalid("snapshot overflow offset overflow"))?;
                // Keep the UnsafeCell-backed file borrow shorter than the
                // caller-controlled writer callback. Every chunk seeks by
                // absolute guest-block offset, so dropping the file borrow
                // between chunks cannot affect stream order.
                let mut scratch = [0u8; SNAPSHOT_IO_CHUNK];
                let mut remaining = logical_len;
                while remaining != 0 {
                    let chunk_len = remaining.min(scratch.len());
                    let chunk_offset = offset
                        .checked_add(logical_len - remaining)
                        .ok_or_else(|| snapshot_invalid("snapshot overflow offset overflow"))?;
                    {
                        let file = self.overflow_file_mut();
                        file.seek(SeekFrom::Start(
                            u64::try_from(chunk_offset).map_err(|_| {
                                snapshot_invalid("snapshot overflow offset conversion failed")
                            })?,
                        ))?;
                        let mut read = 0;
                        while read != chunk_len {
                            let count = file.read(&mut scratch[read..chunk_len])?;
                            if count == 0 {
                                scratch[read..chunk_len].fill(0);
                                break;
                            }
                            read += count;
                        }
                    }
                    out.write_all(&scratch[..chunk_len])?;
                    remaining -= chunk_len;
                }
                Ok(())
            }
        }
    }

    /// Restore one logical guest block to its saved slot or overflow extent.
    ///
    /// The block map itself remains untouched until `finish_snapshot_restore`
    /// has validated all descriptors and the complete transfer succeeds.
    #[cfg(feature = "std")]
    pub(super) fn read_snapshot_block<R: Read>(
        &mut self,
        guest_block: u32,
        saved: MemorySnapshotResidency,
        input: &mut R,
    ) -> std::io::Result<()> {
        let guest_block = usize::try_from(guest_block)
            .map_err(|_| snapshot_invalid("snapshot guest block conversion failed"))?;
        let logical_len = self.snapshot_logical_block_len(guest_block)?;
        match saved {
            MemorySnapshotResidency::Resident { slot } => {
                let slot = usize::try_from(slot)
                    .map_err(|_| snapshot_invalid("snapshot resident slot conversion failed"))?;
                input.read_exact(self.snapshot_resident_block_mut(slot, logical_len)?)
            }
            MemorySnapshotResidency::Swapped => {
                let offset = guest_block
                    .checked_mul(self.block_size)
                    .ok_or_else(|| snapshot_invalid("snapshot overflow offset overflow"))?;
                let mut scratch = [0u8; SNAPSHOT_IO_CHUNK];
                let mut remaining = logical_len;
                while remaining != 0 {
                    let chunk_len = remaining.min(scratch.len());
                    input.read_exact(&mut scratch[..chunk_len])?;
                    let chunk_offset = offset
                        .checked_add(logical_len - remaining)
                        .ok_or_else(|| snapshot_invalid("snapshot overflow offset overflow"))?;
                    {
                        let file = self.overflow_file_mut();
                        file.seek(SeekFrom::Start(
                            u64::try_from(chunk_offset).map_err(|_| {
                                snapshot_invalid("snapshot overflow offset conversion failed")
                            })?,
                        ))?;
                        file.write_all(&scratch[..chunk_len])?;
                    }
                    remaining -= chunk_len;
                }
                Ok(())
            }
        }
    }

    /// Validate and atomically install snapshot residency metadata.
    #[cfg(feature = "std")]
    pub(super) fn finish_snapshot_restore(
        &mut self,
        geometry: MemorySnapshotGeometry,
        saved_map: &[MemorySnapshotResidency],
    ) -> std::io::Result<()> {
        let expected_num_blocks = self.snapshot_expected_num_blocks()?;
        let resident_capacity = self.snapshot_resident_capacity();
        if geometry.guest_len != self.len as u64
            || geometry.host_ram_len != self.allocated as u64
            || geometry.block_size != self.block_size as u64
            || usize::try_from(geometry.num_blocks)
                .map_err(|_| snapshot_invalid("snapshot block count conversion failed"))?
                != expected_num_blocks
            || self.num_blocks != expected_num_blocks
            || usize::try_from(geometry.resident_capacity)
                .map_err(|_| snapshot_invalid("snapshot resident capacity conversion failed"))?
                != resident_capacity
            || saved_map.len() != expected_num_blocks
        {
            return Err(snapshot_invalid("snapshot memory geometry does not match machine"));
        }

        let used_blocks = usize::try_from(geometry.used_blocks)
            .map_err(|_| snapshot_invalid("snapshot used block count conversion failed"))?;
        if used_blocks > resident_capacity || used_blocks > expected_num_blocks {
            return Err(snapshot_invalid("snapshot used block count is out of range"));
        }

        let next_swapout = usize::try_from(geometry.next_swapout_guest_block)
            .map_err(|_| snapshot_invalid("snapshot swap cursor conversion failed"))?;
        if (expected_num_blocks == 0 && next_swapout != 0)
            || (expected_num_blocks != 0 && next_swapout >= expected_num_blocks)
        {
            return Err(snapshot_invalid("snapshot swap cursor is out of range"));
        }

        // Descriptor storage is O(number of blocks), never O(guest RAM).
        // Allocate only while validating; byte streaming itself is fixed-size.
        let mut seen_slots = std::vec::Vec::new();
        seen_slots
            .try_reserve_exact(resident_capacity)
            .map_err(|_| snapshot_other("unable to validate snapshot resident slots"))?;
        seen_slots.resize(resident_capacity, false);

        let mut resident_count = 0usize;
        for (guest_block, &saved) in saved_map.iter().enumerate() {
            if let MemorySnapshotResidency::Resident { slot } = saved {
                let slot = usize::try_from(slot)
                    .map_err(|_| snapshot_invalid("snapshot resident slot conversion failed"))?;
                self.snapshot_slot_offset(slot, self.snapshot_logical_block_len(guest_block)?)?;
                let seen = seen_slots
                    .get_mut(slot)
                    .ok_or_else(|| snapshot_invalid("snapshot resident slot is out of range"))?;
                if *seen {
                    return Err(snapshot_invalid("snapshot resident slots are not unique"));
                }
                *seen = true;
                resident_count += 1;
            }
        }
        if resident_count != used_blocks
            || seen_slots[..used_blocks].iter().any(|seen| !seen)
        {
            return Err(snapshot_invalid(
                "snapshot resident slots do not form a dense used prefix",
            ));
        }

        // Flush all transferred swapped bytes before changing ownership.
        self.overflow_file_mut().flush()?;

        // A partial final guest block never makes physical tail bytes
        // architectural. Fully backed RAM can have no tail at all, so clear
        // only the bytes that actually exist in its resident slot.
        if let Some(MemorySnapshotResidency::Resident { slot }) = saved_map.last().copied() {
            let final_len = self.snapshot_logical_block_len(expected_num_blocks - 1)?;
            if final_len < self.block_size {
                let slot = usize::try_from(slot)
                    .map_err(|_| snapshot_invalid("snapshot resident slot conversion failed"))?;
                let slot_offset = self.snapshot_slot_offset(slot, final_len)?;
                let tail_start_offset = slot_offset
                    .checked_add(final_len)
                    .ok_or_else(|| snapshot_invalid("snapshot final slot tail overflow"))?;
                let tail_len = self
                    .resident_backing_len
                    .saturating_sub(tail_start_offset)
                    .min(self.block_size - final_len);
                if tail_len != 0 {
                    let tail_start = self
                        .vector_offset
                        .checked_add(tail_start_offset)
                        .ok_or_else(|| snapshot_invalid("snapshot final slot tail overflow"))?;
                    unsafe {
                        core::slice::from_raw_parts_mut(self.actual_vector.add(tail_start), tail_len)
                    }
                    .fill(0);
                }
            }
        }

        // Commit only after every descriptor, byte transfer, and overflow flush
        // succeeded. ROM, bogus/APIC scratch, padding, and CPU TLB pointers are
        // intentionally outside this block-logical state.
        for (guest_block, saved) in saved_map.iter().copied().enumerate() {
            self.blocks_offsets()[guest_block] = match saved {
                MemorySnapshotResidency::Swapped => Block::SwappedOut,
                MemorySnapshotResidency::Resident { slot } => Block::Block {
                    offset: usize::try_from(slot)
                        .map_err(|_| snapshot_invalid("snapshot resident slot conversion failed"))?
                        * self.block_size,
                },
            };
        }
        self.used_blocks.set(used_blocks);
        self.next_swapout_idx.set(next_swapout);
        self.recompute_identity_map();
        self.smc_stamps.fill(0);
        self.smc_pending.fill(crate::cpu::icache::PendingSmc::default());
        self.smc_pending_len = 0;
        self.smc_seq_next = 0;
        self.smc_overflow_seq = 0;
        Ok(())
    }

    /// Return the resident host slice for an already translated guest-RAM
    /// offset. The slice never crosses a guest block.
    pub(super) fn get_vector_offset<'a>(
        &'a mut self,
        addr: usize,
        pins: &[CpuTlbPin],
    ) -> Result<&'a mut [u8]> {
        if addr >= self.len {
            return Err(MemoryError::Internal("translated RAM offset out of range").into());
        }
        let guest_block = addr / self.block_size;
        if matches!(self.blocks_offsets()[guest_block], Block::SwappedOut) {
            self.allocate_block(guest_block, pins)?;
        }
        let Block::Block { offset } = self.blocks_offsets()[guest_block] else {
            return Err(MemoryError::Internal("allocated block is not resident").into());
        };
        let within = addr & (self.block_size - 1);
        let start = self
            .vector_offset
            .checked_add(offset)
            .and_then(|n| n.checked_add(within))
            .ok_or(MemoryError::Internal("resident block offset overflow"))?;
        let remaining = (self.block_size - within).min(self.len - addr);
        Ok(unsafe { core::slice::from_raw_parts_mut(self.actual_vector.add(start), remaining) })
    }


    #[inline]
    fn logical_block_len(&self, block: usize) -> usize {
        self.len
            .saturating_sub(block * self.block_size)
            .min(self.block_size)
    }

    #[cfg(feature = "std")]
    fn read_block_into(&self, block: usize, slot_offset: usize) -> Result<()> {
        let logical_len = self.logical_block_len(block);
        let slot_end = slot_offset
            .checked_add(self.block_size)
            .ok_or(MemoryError::Internal("resident slot overflow"))?;
        if slot_end > self.resident_backing_len {
            return Err(MemoryError::Internal("resident slot outside host backing").into());
        }
        let chosen = unsafe {
            core::slice::from_raw_parts_mut(
                self.actual_vector.add(self.vector_offset + slot_offset),
                self.block_size,
            )
        };
        chosen.fill(0);
        let offset = block
            .checked_mul(self.block_size)
            .ok_or(MemoryError::Internal("overflow file offset overflow"))?;
        let file = self.overflow_file_mut();
        file.seek(SeekFrom::Start(u64::try_from(offset)?))
            .map_err(|e| MemoryError::CantSeekToAddressOverflowFile(offset, e))?;
        file.read_exact(&mut chosen[..logical_len])?;
        Ok(())
    }

    pub(crate) fn allocate_block(&self, block: usize, pins: &[CpuTlbPin]) -> Result<()> {
        if block >= self.num_blocks {
            return Err(MemoryError::Internal("guest block out of range").into());
        }
        if !matches!(self.blocks_offsets()[block], Block::SwappedOut) {
            return Ok(());
        }
        #[cfg(not(feature = "std"))]
        {
            let _ = pins;
            return Err(MemoryError::InsufficientRam.into());
        }
        #[cfg(feature = "std")]
        {
            let capacity = self.resident_backing_len / self.block_size;
            if capacity == 0 {
                return Err(MemoryError::InsufficientRam.into());
            }
            let used_blocks = self.used_blocks.get();
            let (slot_offset, victim, uses_new_slot) = if used_blocks < capacity {
                (used_blocks * self.block_size, None, true)
            } else {
                let mut selected = None;
                for _ in 0..self.num_blocks {
                    let guest = self.next_swapout_idx.get();
                    self.next_swapout_idx.set((guest + 1) % self.num_blocks);
                    let Block::Block { offset } = self.blocks_offsets()[guest] else {
                        continue;
                    };
                    let start = unsafe { self.actual_vector.add(self.vector_offset + offset) }
                        as usize;
                    if !pins
                        .iter()
                        .any(|pin| pin.is_range_pinned(start, start + self.block_size))
                    {
                        selected = Some((guest, offset));
                        break;
                    }
                }
                let (guest, offset) = selected.ok_or(MemoryError::InsufficientRam)?;
                (offset, Some(guest), false)
            };
            if let Some(victim_guest) = victim {
                let logical_len = self.logical_block_len(victim_guest);
                let file_offset = victim_guest
                    .checked_mul(self.block_size)
                    .ok_or(MemoryError::Internal("overflow file offset overflow"))?;
                let victim_bytes = unsafe {
                    core::slice::from_raw_parts(
                        self.actual_vector.add(self.vector_offset + slot_offset),
                        logical_len,
                    )
                };
                let file = self.overflow_file_mut();
                file.seek(SeekFrom::Start(u64::try_from(file_offset)?))
                    .map_err(|e| MemoryError::CantSeekToAddressOverflowFile(file_offset, e))?;
                file.write_all(victim_bytes)
                    .map_err(|e| MemoryError::FailedToWriteToOverflowFIle(file_offset, e))?;
            }
            if let Err(error) = self.read_block_into(block, slot_offset) {
                // The victim remains logically resident until target reload has
                // completed. Restore its slot from the just-persisted bytes.
                if let Some(victim_guest) = victim {
                    let _ = self.read_block_into(victim_guest, slot_offset);
                }
                return Err(error);
            }
            if let Some(victim_guest) = victim {
                self.blocks_offsets()[victim_guest] = Block::SwappedOut;
            }
            self.blocks_offsets()[block] = Block::Block {
                offset: slot_offset,
            };
            if uses_new_slot {
                self.used_blocks.set(used_blocks + 1);
            }
            // Swapping regime: blocks land at arbitrary slots (and this path
            // is only reachable when residency is partial), so the identity
            // map is broken. Exact by construction — under full residency no
            // block is ever SwappedOut and this function is never entered.
            self.identity_map.set(false);
            Ok(())
        }
    }


    pub(crate) fn write_physical_page(
        &mut self,
        pins: &[CpuTlbPin],
        addr: BxPhyAddress,
        len: usize,
        data: &mut [u8],
        a20_mask: A20Mask,
    ) -> Result<()> {
        if data.len() < len {
            return Err(MemoryError::WritePhysicalPage { addr, len }.into());
        }
        if len == 0 {
            return Ok(());
        }
        let a20_addr = addr & a20_mask;

        // Note: accesses should always be contained within a single page
        if (addr >> 12) != ((addr + len as u64 - 1) >> 12) {
            return Err(MemoryError::WritePhysicalPage { addr, len }.into());
        }

        if bx_is_pci_hole_addr(a20_addr) {
            // PCI MMIO hole — writes are silently dropped
            return Ok(());
        }
        if bx_guest_ram_span(a20_addr, len, self.len).is_some() {
            // A typed physical access may straddle independently resident
            // guest blocks.  Do not hand a block-short slice to endian helpers.
            for (offset, byte) in data.iter().copied().take(len).enumerate() {
                let byte_addr = a20_addr + offset as u64;
                self.smc_dec_write_stamp(byte_addr, 1);
                let span = bx_guest_ram_span(byte_addr, 1, self.len)
                    .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
                self.get_vector_offset(span.start, pins)?[0] = byte;
            }
            return Ok(());
        }
        Ok(())
    }

    pub(crate) fn read_physical_page(
        &mut self,
        pins: &[CpuTlbPin],
        addr: BxPhyAddress,
        len: usize,
        data: &mut [u8],
        a20_mask: A20Mask,
    ) -> Result<()> {
        let a20_addr = addr & a20_mask;

        if data.len() < len {
            return Err(MemoryError::ReadPhysicalPage { addr, len }.into());
        }
        if len == 0 {
            return Ok(());
        }
        // Note: accesses should always be contained within a single page
        if (addr >> 12) != ((addr + len as u64 - 1) >> 12) {
            return Err(MemoryError::ReadPhysicalPage { addr, len }.into());
        }

        if bx_is_pci_hole_addr(a20_addr) {
            // PCI MMIO hole — reads return 0xFF
            data[..len].fill(0xff);
            return Ok(());
        }
        if bx_guest_ram_span(a20_addr, len, self.len).is_some() {
            // The resident primitive is block-bounded; assemble typed accesses
            // bytewise when a guest-block boundary lies within the span.
            for (offset, byte) in data.iter_mut().take(len).enumerate() {
                let byte_addr = a20_addr + offset as u64;
                let span = bx_guest_ram_span(byte_addr, 1, self.len)
                    .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
                *byte = self.get_vector_offset(span.start, pins)?[0];
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

}
