#![allow(unused_assignments, dead_code)]

mod error;
pub(crate) mod memory_rusty_box;
pub mod memory_stub;
pub mod misc_mem;
pub mod mmio;
pub mod permissions;

//#[cfg(test)]
//mod tests;

pub use super::error::Result;
use crate::{
    config::{BxPhyAddress, MAX_HANDLER_OVERFLOW, MAX_MEM_BLOCKS},
    cpu::{BxCpuC, BxCpuIdTrait},
};
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
pub use error::*;

use core::cell::{Cell, UnsafeCell};

#[cfg(feature = "std")]
use std::fs::File;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    actual_vector: *mut u8,
    actual_vector_len: usize,
    /// Allocation layout for owned buffers; `None` for external raw memory.
    actual_vector_layout: Option<core::alloc::Layout>,
    /// aligned correctly
    vector_offset: usize,
    /// None if swapped out
    blocks_offsets: UnsafeCell<[Block; MAX_MEM_BLOCKS]>,
    num_blocks: usize,
    /// 512k BIOS rom space + 128k expansion rom space
    rom_offset: usize,
    /// 4k for unexisting memory
    bogus_offset: usize,

    used_blocks: Cell<usize>,

    /// Machine-wide SMC page-write-stamp table — Bochs icache.h
    /// `bxPageWriteStampTable::fineGranularityMapping`. ONE table for the
    /// whole machine (Bochs has a single global instance): trace creation by
    /// ANY cpu marks it, and a write hitting marked lines must invalidate
    /// EVERY cpu's icache (Bochs icache.cc `handleSMC` loops over
    /// BX_SMP_PROCESSORS).
    #[cfg(feature = "alloc")]
    smc_stamps: Vec<u32>,
    #[cfg(not(feature = "alloc"))]
    smc_stamps: [u32; crate::cpu::icache::SMC_STAMP_ENTRIES],
    /// Queued cross-cpu SMC invalidations, drained by the emulator at
    /// round-robin slice boundaries (no sibling cpu can execute before the
    /// drain, so deferral is observably identical to Bochs's synchronous
    /// `handleSMC` loop). `smc_seq_next` is a monotonic event counter; each
    /// cpu keeps a watermark (`BxCpuC::smc_seq_seen`) so it applies exactly
    /// the events it has not seen.
    smc_pending: [crate::cpu::icache::PendingSmc; crate::cpu::icache::SMC_PENDING_CAP],
    smc_pending_len: usize,
    smc_seq_next: u64,
    /// CPUs whose watermark is below this must flush their whole icache
    /// (an event was dropped on pending-queue overflow).
    smc_overflow_seq: u64,

    /// Zero-initialized 4KB scratch buffer for APIC MMIO (0xFEE00000-0xFEEFFFFF)
    apic_scratch: [u8; 4096],

    next_swapout_idx: Cell<usize>,
    #[cfg(feature = "std")]
    //overflow_file: Option<Arc<Mutex<std::fs::File>>>,
    overflow_file: UnsafeCell<File>,
    //swapped_out: *const u8,
}

// SAFETY: The raw pointer `actual_vector` is owned exclusively by this struct
// (allocated once, never aliased). UnsafeCell fields are only accessed single-threaded.
unsafe impl Send for BxMemoryStubC {}

impl Drop for BxMemoryStubC {
    fn drop(&mut self) {
        #[cfg(feature = "alloc")]
        if let Some(layout) = self.actual_vector_layout.take() {
            unsafe { alloc::alloc::dealloc(self.actual_vector, layout) };
        }
    }
}

type Unsigned = u32;

/// Identifies which device owns a memory-mapped I/O handler.
///
/// Each variant carries a raw pointer to the device instance, replacing the
/// former `*const c_void` param + fn-ptr pair with a typed discriminant that
/// the dispatch code in `misc_mem.rs` matches on directly.
#[derive(Clone, Copy)]
pub(crate) enum MemoryDeviceId {
    Vga(*mut crate::iodev::vga::BxVgaC),
    IoApic(*mut crate::iodev::ioapic::BxIoApic),
    None,
}

impl core::fmt::Debug for MemoryDeviceId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Vga(p) => write!(f, "Vga({:p})", p),
            Self::IoApic(p) => write!(f, "IoApic({:p})", p),
            Self::None => write!(f, "None"),
        }
    }
}

impl MemoryDeviceId {
    /// Dereference the VGA device pointer.
    ///
    /// # Safety (internal)
    /// The raw pointer was set once at init and remains valid for the emulator lifetime.
    /// Aliasing is the caller's responsibility (same as the prior inline `unsafe` blocks).
    #[inline(always)]
    pub(crate) fn vga_mut(&self) -> Option<&mut crate::iodev::vga::BxVgaC> {
        match self {
            MemoryDeviceId::Vga(ptr) => Some(unsafe { &mut **ptr }),
            _ => None,
        }
    }

    /// Dereference the IOAPIC device pointer.
    ///
    /// # Safety (internal)
    /// The raw pointer was set once at init and remains valid for the emulator lifetime.
    /// Aliasing is the caller's responsibility (same as the prior inline `unsafe` blocks).
    #[inline(always)]
    pub(crate) fn ioapic_mut(&self) -> Option<&mut crate::iodev::ioapic::BxIoApic> {
        match self {
            MemoryDeviceId::IoApic(ptr) => Some(unsafe { &mut **ptr }),
            _ => None,
        }
    }

    /// Whether two ids refer to the same device instance (pointer identity).
    /// Used by `unregister_memory_handlers` to match the handler to remove.
    #[inline]
    pub(crate) fn same_device(&self, other: &MemoryDeviceId) -> bool {
        match (self, other) {
            (MemoryDeviceId::Vga(a), MemoryDeviceId::Vga(b)) => core::ptr::eq(*a, *b),
            (MemoryDeviceId::IoApic(a), MemoryDeviceId::IoApic(b)) => core::ptr::eq(*a, *b),
            (MemoryDeviceId::None, MemoryDeviceId::None) => true,
            _ => false,
        }
    }
}

#[derive(Debug)]
pub(super) struct MemoryHandlerStruct {
    next: Option<u16>,
    pub(super) begin: BxPhyAddress,
    pub(super) end: BxPhyAddress,
    bitmap: u16,
    pub(super) device_id: MemoryDeviceId,
}

//#define BIOS_MAP_LAST128K(addr) (((addr) | 0xfff00000) & BIOS_MASK)

pub(crate) const BIOS_ROM_LOWER: u8 = 0x01;
pub(crate) const BIOS_ROM_EXTENDED: u8 = 0x02;
pub(crate) const BIOS_ROM_1MEG: u8 = 0x04;

#[derive(Debug)]
pub struct BxMemC<'a> {
    #[cfg(feature = "alloc")]
    memory_handlers: Vec<Option<MemoryHandlerStruct>>,
    #[cfg(not(feature = "alloc"))]
    memory_handlers: [Option<MemoryHandlerStruct>; 4096],
    handler_overflow: [Option<MemoryHandlerStruct>; MAX_HANDLER_OVERFLOW],
    handler_overflow_count: usize,
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

    /// Keeps the lifetime parameter used by callers (CPU borrows, emulator context).
    _marker: core::marker::PhantomData<&'a ()>,
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
        let ram = stub.actual_vector_slice();
        if real_addr < ram.len() {
            let end = (real_addr + len).min(ram.len());
            &ram[real_addr..end]
        } else {
            &[]
        }
    }

    /// Write raw RAM bytes (no A20 masking, no memory handlers).
    /// Mirror of `peek_ram` for device-initiated physical writes (bus-master
    /// DMA). Bochs `BX_MEM_C::dmaWritePhysicalPage` (memory.cc) likewise
    /// bypasses the handler layer and writes RAM pages directly.
    /// Writes up to `data.len()` bytes at `addr`, truncated at end of RAM.
    pub fn poke_ram(&mut self, addr: usize, data: &[u8]) {
        if data.is_empty() {
            return;
        }

        let stub = &mut self.inherited_memory_stub;
        let Some(real_addr) = stub.vector_offset.checked_add(addr) else {
            return;
        };
        let ram = stub.actual_vector_mut();
        if real_addr >= ram.len() {
            return;
        }

        let copied_len = data.len().min(ram.len() - real_addr);
        let end = real_addr + copied_len;
        ram[real_addr..end].copy_from_slice(&data[..copied_len]);

        // Bochs memory.cc dmaWritePhysicalPage:
        // pageWriteStampTable.decWriteStamp(a20addr) — device writes must
        // invalidate cached traces on every touched page. Bochs callers
        // chunk per page; poke_ram accepts multi-page spans, so walk only
        // the pages that actually received bytes.
        let mut page = (addr as BxPhyAddress) & !0xFFF;
        let last_page = ((addr + copied_len - 1) as BxPhyAddress) & !0xFFF;
        loop {
            stub.smc_dec_write_stamp_page(page);
            if page >= last_page {
                break;
            }
            page += 0x1000;
        }
    }

    /// Get the current A20 mask
    pub fn a20_mask(&self) -> BxPhyAddress {
        self.a20_mask
    }

    /// Get mutable access to the underlying memory stub for snapshot save/restore.
    pub fn get_stub_mut(&mut self) -> &mut BxMemoryStubC {
        &mut self.inherited_memory_stub
    }

    // ── SMC write-stamp table forwarders (table lives in the stub) ─────────

    /// Bochs icache.h `bxPageWriteStampTable::markICacheMask`.
    #[inline]
    pub(crate) fn smc_mark_icache_mask(&mut self, p_addr: BxPhyAddress, mask: u32) {
        self.inherited_memory_stub
            .smc_mark_icache_mask(p_addr, mask);
    }

    /// Whether a single-page physical range overlaps cached instruction lines.
    #[inline]
    pub(crate) fn smc_range_has_stamps(&self, p_addr: BxPhyAddress, len: u32) -> bool {
        self.inherited_memory_stub.smc_range_has_stamps(p_addr, len)
    }

    /// Bochs icache.h `bxPageWriteStampTable::decWriteStamp(pAddr, len)`.
    #[inline]
    pub(crate) fn smc_dec_write_stamp(&mut self, p_addr: BxPhyAddress, len: u32) {
        self.inherited_memory_stub.smc_dec_write_stamp(p_addr, len);
    }

    /// Sequence number the next SMC event will get (cpu watermark compare).
    #[inline]
    pub(crate) fn smc_seq_next(&self) -> u64 {
        self.inherited_memory_stub.smc_seq_next()
    }

    /// Events a watermark of `since` has not seen: `(needs_full_flush, events)`.
    #[inline]
    pub(crate) fn smc_pending_since(
        &self,
        since: u64,
    ) -> (bool, &[crate::cpu::icache::PendingSmc]) {
        self.inherited_memory_stub.smc_pending_since(since)
    }

    /// Drop drained SMC events (emulator, after every cpu caught up).
    #[inline]
    pub(crate) fn smc_clear_pending(&mut self) {
        self.inherited_memory_stub.smc_clear_pending();
    }

    /// True when SMC events are queued (per-slice drain early-out).
    #[inline]
    pub(crate) fn smc_has_pending(&self) -> bool {
        self.inherited_memory_stub.smc_has_pending()
    }

    /// Bochs icache.h `bxPageWriteStampTable::resetWriteStamps` (hardware reset).
    pub(crate) fn smc_reset_stamps(&mut self) {
        self.inherited_memory_stub.smc_reset_stamps();
    }

    /// Enable SMRAM (System Management RAM) with the given DOPEN/DCLS state.
    ///
    /// Matches BX_MEM_C::enable_smram(bool enable, bool restricted) from
    /// cpp_orig/bochs/memory/misc_mem.cc: `enable` is DOPEN (SMM space open
    /// for non-SMM-mode CPU accesses), `restricted` is DCLS (SMM space closed
    /// to data references while still open to code fetches).
    pub fn enable_smram(&mut self, enable: bool, restricted: bool) {
        self.smram_available = true;
        self.smram_enable = enable;
        self.smram_restricted = restricted;
    }

    /// Disable SMRAM (System Management RAM)
    ///
    /// Matches BX_MEM_C::disable_smram() from cpp_orig/bochs/memory/misc_mem.cc
    pub fn disable_smram(&mut self) {
        self.smram_available = false;
        self.smram_enable = false;
        self.smram_restricted = false;
    }

    /// Snapshot of SMRAM control state: (available, enable/DOPEN, restricted/DCLS).
    /// Test/diagnostic accessor — the actual A0000-BFFFF routing decision is
    /// made directly against these flags in misc_mem.rs
    /// (get_host_mem_addr/read_physical_page/write_physical_page), untouched here.
    pub(crate) fn smram_state(&self) -> (bool, bool, bool) {
        (
            self.smram_available,
            self.smram_enable,
            self.smram_restricted,
        )
    }

    /// Test/diagnostic accessor for the PAM-derived memory type of a shadow
    /// RAM area. Mirrors `set_memory_type`: `area` is one of the 13 memory
    /// areas (C0000..F0000), `rw` is 0 = read path, 1 = write path.
    pub(crate) fn memory_type(&self, area: usize, rw: usize) -> bool {
        self.memory_type[area][rw]
    }

    /// Set whether writes to the BIOS ROM region are allowed outside PAM
    /// shadow RAM (both the C0000-FFFFF non-shadowed path and the
    /// top-of-address-space BIOS mirror). Driven by the PIIX3 XBCS register
    /// bit 2 (Bochs pci2isa.cc `pci_write_handler` case 0x4e ->
    /// `DEV_mem_set_bios_write`); Bochs `BX_MEM_C::set_bios_write`
    /// (misc_mem.cc).
    pub fn set_bios_write_enabled(&mut self, enabled: bool) {
        self.bios_write_enabled = enabled;
    }

    /// Test/diagnostic accessor for the current BIOS-write-enable state.
    pub(crate) fn bios_write_enabled(&self) -> bool {
        self.bios_write_enabled
    }

    /// Set or clear one region bit of the BIOS ROM access bitmask (`region`
    /// is one of `BIOS_ROM_LOWER`/`BIOS_ROM_EXTENDED`/`BIOS_ROM_1MEG`).
    /// Matches Bochs `BX_MEM_C::set_bios_rom_access` (misc_mem.cc), driven by
    /// PIIX3 XBCS bits 6-7 (pci2isa.cc case 0x4e ->
    /// `DEV_mem_set_bios_rom_access`). As in upstream Bochs, this bitmask is
    /// tracked for parity but not consulted by any read/write path — Bochs
    /// itself logs "BIOS enable switches not supported" when these bits
    /// change and never reads `bios_rom_access` back anywhere.
    pub fn set_bios_rom_access(&mut self, region: u8, enabled: bool) {
        if enabled {
            self.bios_rom_access |= region;
        } else {
            self.bios_rom_access &= !region;
        }
    }

    /// Test/diagnostic accessor for the BIOS ROM access bitmask.
    pub(crate) fn bios_rom_access(&self) -> u8 {
        self.bios_rom_access
    }
}

// implement getters and setters for memory stub
impl BxMemoryStubC {
    /// Reconstruct the full backing buffer as a shared slice.
    pub(crate) fn actual_vector_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.actual_vector, self.actual_vector_len) }
    }

    /// Reconstruct the full backing buffer as a mutable slice.
    pub(crate) fn actual_vector_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.actual_vector, self.actual_vector_len) }
    }

    pub fn actual_vector(&mut self) -> &mut [u8] {
        self.actual_vector_mut()
    }

    #[allow(clippy::mut_from_ref)]
    fn blocks_offsets(&self) -> &mut [Block] {
        let arr = unsafe { &mut (*self.blocks_offsets.get()) };
        &mut arr[..self.num_blocks]
    }

    pub fn vector(&mut self) -> &mut [u8] {
        let vo = self.vector_offset;
        &mut self.actual_vector_mut()[vo..]
    }

    pub fn rom(&mut self) -> &mut [u8] {
        let ro = self.rom_offset;
        &mut self.actual_vector_mut()[ro..]
    }

    pub fn bogus(&mut self) -> &mut [u8] {
        let bo = self.bogus_offset;
        &mut self.actual_vector_mut()[bo..]
    }

    pub fn apic_scratch(&mut self) -> &mut [u8] {
        &mut self.apic_scratch
    }

    /// Get a mutable reference to a memory block by index
    #[cfg(feature = "std")]
    #[allow(clippy::mut_from_ref)]
    pub fn block_by_index(&self, index: usize) -> Option<&mut [u8]> {
        if let Some(Block::Block { offset }) = self.blocks_offsets().get(index) {
            let start = self.vector_offset + *offset;
            // SAFETY: We're accessing within bounds of actual_vector via interior mutability pattern
            let slice = unsafe {
                core::slice::from_raw_parts_mut(self.actual_vector.add(start), self.block_size)
            };
            Some(slice)
        } else {
            None
        }
    }

    #[cfg(feature = "std")]
    #[allow(clippy::mut_from_ref)]
    fn overflow_file_mut(&self) -> &mut File {
        unsafe { &mut *self.overflow_file.get() }
    }
}

impl<'m> BxMemC<'m> {
    pub(crate) fn get_vector<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
        &mut self,
        cpus: &[&BxCpuC<I, T>],
        addr: BxPhyAddress,
    ) -> Result<&mut [u8]> {
        self.inherited_memory_stub.get_vector(addr, cpus)
    }

    pub(super) fn is_monitor<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
        cpus: &[&BxCpuC<I, T>],
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
        let v = stub.actual_vector_slice();
        &v[stub.vector_offset..]
    }

    /// Get raw pointer to memory for direct CPU access
    /// SAFETY: Caller must ensure the pointer is only used while memory is valid
    pub fn get_raw_memory_ptr(&mut self) -> (*mut u8, usize) {
        let ptr = self.inherited_memory_stub.actual_vector;
        let len = self.inherited_memory_stub.actual_vector_len;
        (ptr, len)
    }

    /// Get a raw pointer to physical address 0 in host memory, plus the usable RAM length.
    ///
    /// This accounts for `vector_offset` alignment padding, so
    /// `returned_ptr.add(phys_addr)` gives the byte at physical address `phys_addr`.
    ///
    /// SAFETY: Caller must ensure the pointer is only used while memory is valid.
    pub fn get_ram_base_ptr(&mut self) -> (*mut u8, usize) {
        let vo = self.inherited_memory_stub.vector_offset;
        let ptr = unsafe { self.inherited_memory_stub.actual_vector.add(vo) };
        let len = self.inherited_memory_stub.len; // guest RAM size
        (ptr, len)
    }

    /// Count how many registered (non-None) memory handlers exist (for diagnostics).
    pub fn memory_handler_info(&self) -> usize {
        self.memory_handlers.iter().filter(|h| h.is_some()).count()
    }

    /// Set memory type for a specific area (PAM register support).
    /// Bochs: BX_MEM_C::set_memory_type() (misc_mem.cc)
    ///
    /// `area` is one of the 13 memory areas (C0000..F0000, 16KB each).
    /// `rw`: 0 = read path, 1 = write path.
    /// `dram`: true = DRAM (shadow RAM), false = ROM.
    pub fn set_memory_type(&mut self, area: usize, rw: usize, dram: bool) {
        if area < 13 && rw < 2 {
            tracing::trace!(
                "set_memory_type: area={}, rw={}, dram={} (was {})",
                area,
                rw,
                dram,
                self.memory_type[area][rw]
            );
            self.memory_type[area][rw] = dram;
        }
    }

    /// Read bytes from the ROM array at the given offset (for diagnostics).
    pub fn peek_rom(&self, offset: usize, len: usize) -> &[u8] {
        let stub = &self.inherited_memory_stub;
        let rom_start = stub.rom_offset;
        let v = stub.actual_vector_slice();
        let rom = &v[rom_start..];
        let end = (offset + len).min(rom.len());
        if offset < rom.len() {
            &rom[offset..end]
        } else {
            &[]
        }
    }
}
#[cfg(feature = "alloc")]
impl<'m> BxMemC<'m> {
    pub fn init_memory(
        &mut self,
        guest_size: usize,
        host_size: usize,
        block_size: usize,
    ) -> Result<()> {
        let mem_stub = BxMemoryStubC::create_and_init(guest_size, host_size, block_size)?;
        self.inherited_memory_stub = *mem_stub;
        self.rom_present = [false; 65];
        self.bios_rom_addr = 0xffff0000;
        self.memory_type = [[false, false]; 13];
        Ok(())
    }
}
