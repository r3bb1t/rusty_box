//! PIIX3 PCI IDE Controller with Bus Master DMA
//!
//! Matches Bochs `iodev/pci_ide.cc` (459 lines) + `pci_ide.h` (84 lines).
//!
//! Implements:
//! - PCI IDE controller (PIIX3) — bus 0, device 1, function 1
//! - Bus Master DMA registers at configurable I/O base (BAR4)
//! - Two IDE channels (primary and secondary)
//! - BM-DMA command, status, and descriptor table pointer registers
//! - Physical Region Descriptor (PRD) table processing
//! - Timer-driven DMA transfers (Bochs pci_ide.cc:251-336)

use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_void;

use super::harddrv::BxHardDriveC;
use crate::pc_system::BxPcSystemC;

/// PCI configuration space size
const PCI_CONF_SIZE: usize = 256;

/// BM-DMA I/O mask for the 16-port register block.
/// Bochs: bmdma_iomask[16] (pci_ide.cc:42)
const BMDMA_IOMASK: [u8; 16] = [1, 0, 1, 0, 4, 0, 0, 0, 1, 0, 1, 0, 4, 0, 0, 0];

/// BM-DMA buffer size per channel (128 KB)
const BMDMA_BUFFER_SIZE: usize = 0x20000;

/// BM-DMA channel state.
/// Bochs: pci_ide.h:62-73
#[derive(Debug)]
pub struct BmDmaChannel {
    /// Start/Stop Bus Master (bit 0 of command register)
    pub cmd_ssbm: bool,
    /// Read/Write Control (bit 3 of command register): true = read (device→memory)
    pub cmd_rwcon: bool,
    /// Status register (bit 0=active, bit 2=IRQ, bits 5-6=simplex)
    pub status: u8,
    /// Descriptor Table Pointer Register (PRD list base address)
    pub dtpr: u32,
    /// Current PRD being processed
    pub prd_current: u32,
    /// DMA data buffer (128 KB)
    pub buffer: Vec<u8>,
    /// Buffer write pointer offset
    pub buffer_top: usize,
    /// Buffer read pointer offset
    pub buffer_idx: usize,
    /// Data ready flag (set when disk has data for DMA transfer)
    pub data_ready: bool,
    /// Timer handle for pc_system (Bochs pci_ide.h:68)
    pub timer_index: Option<usize>,
}

impl BmDmaChannel {
    fn new() -> Self {
        Self {
            cmd_ssbm: false,
            cmd_rwcon: false,
            status: 0,
            dtpr: 0,
            prd_current: 0,
            buffer: vec![0u8; BMDMA_BUFFER_SIZE],
            buffer_top: 0,
            buffer_idx: 0,
            data_ready: false,
            timer_index: None,
        }
    }

    fn reset(&mut self) {
        self.cmd_ssbm = false;
        self.cmd_rwcon = false;
        self.status = 0;
        self.dtpr = 0;
        self.prd_current = 0;
        self.buffer_top = 0;
        self.buffer_idx = 0;
        self.data_ready = false;
    }
}

/// PIIX3 PCI IDE controller.
/// Bochs: bx_pci_ide_c (pci_ide.h:37-83, pci_ide.cc)
pub struct BxPciIde {
    /// PCI configuration space (256 bytes)
    pub pci_conf: [u8; PCI_CONF_SIZE],

    /// BM-DMA state for 2 channels (primary and secondary)
    pub bmdma: [BmDmaChannel; 2],

    /// BAR4 I/O base address (BM-DMA registers)
    pub bmdma_base: u32,

    /// Raw pointer to pc_system for timer activation (Bochs bx_pc_system)
    pub pc_system_ptr: *mut BxPcSystemC,

    /// Raw pointer to hard drive controller for bmdma_read/write_sector
    pub harddrv_ptr: *mut BxHardDriveC,

    /// Raw pointer to guest physical RAM base (from BxMemC::get_ram_base_ptr)
    pub ram_ptr: *mut u8,

    /// Guest RAM size in bytes
    pub ram_len: usize,
}

// Debug impl that skips raw pointers
impl core::fmt::Debug for BxPciIde {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BxPciIde")
            .field("bmdma_base", &self.bmdma_base)
            .field("bmdma", &self.bmdma)
            .finish()
    }
}

// SAFETY: Raw pointers are only dereferenced within single-threaded emulator context
unsafe impl Send for BxPciIde {}
unsafe impl Sync for BxPciIde {}

impl Default for BxPciIde {
    fn default() -> Self {
        Self::new()
    }
}

impl BxPciIde {
    /// Create a new PCI IDE controller.
    /// Bochs: bx_pci_ide_c::init() (pci_ide.cc:79-122)
    pub fn new() -> Self {
        let mut ide = Self {
            pci_conf: [0; PCI_CONF_SIZE],
            bmdma: [BmDmaChannel::new(), BmDmaChannel::new()],
            bmdma_base: 0,
            pc_system_ptr: core::ptr::null_mut(),
            harddrv_ptr: core::ptr::null_mut(),
            ram_ptr: core::ptr::null_mut(),
            ram_len: 0,
        };
        ide.init_pci_conf();
        ide
    }

    /// Initialize PCI configuration space with PIIX3 IDE identity.
    /// Bochs: init_pci_conf(0x8086, 0x7010, 0x00, 0x010180, 0x00, 0) (pci_ide.cc:111)
    fn init_pci_conf(&mut self) {
        // Vendor ID: Intel (0x8086)
        self.pci_conf[0x00] = 0x86;
        self.pci_conf[0x01] = 0x80;
        // Device ID: PIIX3 IDE (0x7010)
        self.pci_conf[0x02] = 0x10;
        self.pci_conf[0x03] = 0x70;
        // Revision: 0x00
        self.pci_conf[0x08] = 0x00;
        // Class code: IDE controller (0x010180) — native-mode capable, both channels
        self.pci_conf[0x09] = 0x80;
        self.pci_conf[0x0A] = 0x01;
        self.pci_conf[0x0B] = 0x01;
        // Header type: single function (but shared with ISA bridge)
        self.pci_conf[0x0E] = 0x00;
    }

    /// Reset the PCI IDE controller.
    /// Bochs: bx_pci_ide_c::reset() (pci_ide.cc:124-148)
    pub fn reset(&mut self) {
        self.pci_conf[0x04] = 0x05; // I/O space + bus master enabled
        self.pci_conf[0x06] = 0x80;
        self.pci_conf[0x07] = 0x02;
        // IDE timing registers (pci_ide.cc:130-136)
        self.pci_conf[0x40] = 0x00;
        self.pci_conf[0x41] = 0x80; // Channel 0 enabled
        self.pci_conf[0x42] = 0x00;
        self.pci_conf[0x43] = 0x80; // Channel 1 enabled
        self.pci_conf[0x44] = 0x00;

        // BAR4: Bus Master DMA base address.
        // The BIOS normally writes this during POST (0xC001 = I/O at 0xC000).
        // Pre-configure so direct-boot kernels see BM-DMA without BIOS.
        // Bochs bochsrc: ata: ... ioaddr1=0xc000
        self.pci_conf[0x20] = 0x01; // I/O space indicator + low nibble
        self.pci_conf[0x21] = 0xC0; // Base = 0xC000
        self.pci_conf[0x22] = 0x00;
        self.pci_conf[0x23] = 0x00;
        self.bmdma_base = 0xC000;

        // Reset BM-DMA state
        for ch in self.bmdma.iter_mut() {
            ch.reset();
        }
    }

    /// Check if BM-DMA is present (BAR4 configured).
    /// Bochs: bx_pci_ide_c::bmdma_present() (pci_ide.cc:226-229)
    pub fn bmdma_present(&self) -> bool {
        self.bmdma_base > 0
    }

    /// Signal that data is ready for DMA transfer on a channel.
    /// Bochs: bx_pci_ide_c::bmdma_start_transfer() (pci_ide.cc:231-236)
    pub fn bmdma_start_transfer(&mut self, channel: u8) {
        if (channel as usize) < 2 {
            self.bmdma[channel as usize].data_ready = true;
        }
    }

    /// Set IRQ pending bit in BM-DMA status register.
    /// Bochs: bx_pci_ide_c::bmdma_set_irq() (pci_ide.cc:238-243)
    pub fn bmdma_set_irq(&mut self, channel: u8) {
        if (channel as usize) < 2 {
            self.bmdma[channel as usize].status |= 0x04;
        }
    }

    // ─── Timer Handlers ─────────────────────────────────────────────────

    /// Timer handler for channel 0.
    /// Bochs: bx_pci_ide_c::timer_handler() (pci_ide.cc:245-250)
    pub fn timer_handler_ch0(this_ptr: *mut c_void) {
        if this_ptr.is_null() {
            return;
        }
        let ide = unsafe { &mut *(this_ptr as *mut BxPciIde) };
        ide.timer(0);
    }

    /// Timer handler for channel 1.
    pub fn timer_handler_ch1(this_ptr: *mut c_void) {
        if this_ptr.is_null() {
            return;
        }
        let ide = unsafe { &mut *(this_ptr as *mut BxPciIde) };
        ide.timer(1);
    }

    /// BM-DMA timer function — processes PRD tables and transfers data.
    /// Bochs: bx_pci_ide_c::timer() (pci_ide.cc:251-336)
    fn timer(&mut self, channel: usize) {
        if channel >= 2 {
            return;
        }

        // Guard: return if DMA not active or no PRD address
        // Bochs pci_ide.cc:264-267
        if (self.bmdma[channel].status & 0x01) == 0
            || self.bmdma[channel].prd_current == 0
        {
            return;
        }

        // If READ DMA and data not ready, reschedule and return
        // Bochs pci_ide.cc:268-271
        if self.bmdma[channel].cmd_rwcon && !self.bmdma[channel].data_ready {
            self.activate_channel_timer(channel, 1);
            return;
        }

        // Read PRD entry from guest RAM: addr(4 bytes) + size(4 bytes)
        // Bochs pci_ide.cc:272-276
        let prd_addr = self.mem_read_physical_dword(self.bmdma[channel].prd_current);
        let prd_size_raw = self.mem_read_physical_dword(self.bmdma[channel].prd_current + 4);
        let mut size = (prd_size_raw & 0xFFFE) as usize;
        if size == 0 {
            size = 0x10000;
        }

        if self.bmdma[channel].cmd_rwcon {
            // READ DMA: device → memory
            // Bochs pci_ide.cc:278-292
            tracing::debug!("BM-DMA READ to addr={:#010x}, size={:#x}", prd_addr, size);
            let mut count = size as i32
                - (self.bmdma[channel].buffer_top as i32 - self.bmdma[channel].buffer_idx as i32);
            while count > 0 {
                let mut sector_size = count as u32;
                if self.harddrv_bmdma_read_sector(channel as u8, &mut sector_size) {
                    self.bmdma[channel].buffer_top += sector_size as usize;
                    count -= sector_size as i32;
                } else {
                    break;
                }
            }
            if count > 0 {
                // Not enough data — complete the transfer
                self.harddrv_bmdma_complete(channel as u8);
                return;
            }
            // Write buffer data to guest physical memory
            self.mem_write_physical_dma(
                prd_addr,
                size,
                self.bmdma[channel].buffer_idx,
                channel,
            );
            self.bmdma[channel].buffer_idx += size;
        } else {
            // WRITE DMA: memory → device
            // Bochs pci_ide.cc:293-306
            tracing::debug!("BM-DMA WRITE from addr={:#010x}, size={:#x}", prd_addr, size);
            self.mem_read_physical_dma(
                prd_addr,
                size,
                self.bmdma[channel].buffer_top,
                channel,
            );
            self.bmdma[channel].buffer_top += size;
            let mut count = (self.bmdma[channel].buffer_top - self.bmdma[channel].buffer_idx) as i32;
            while count > 511 {
                if self.harddrv_bmdma_write_sector(channel as u8) {
                    self.bmdma[channel].buffer_idx += 512;
                    count -= 512;
                } else {
                    break;
                }
            }
            if count >= 512 {
                // Write failed — complete
                self.harddrv_bmdma_complete(channel as u8);
                return;
            }
        }

        // Check EOT (End Of Table) bit in PRD size field
        // Bochs pci_ide.cc:307-336
        if (prd_size_raw & 0x8000_0000) != 0 {
            // EOT: transfer complete
            self.bmdma[channel].status &= !0x01; // clear active
            self.bmdma[channel].status |= 0x04; // set IRQ
            self.bmdma[channel].prd_current = 0;
            self.harddrv_bmdma_complete(channel as u8);
        } else {
            // More PRDs: compact buffer, advance to next PRD
            // Bochs pci_ide.cc:315-333
            let remaining =
                self.bmdma[channel].buffer_top - self.bmdma[channel].buffer_idx;
            if remaining > 0 {
                self.bmdma[channel]
                    .buffer
                    .copy_within(self.bmdma[channel].buffer_idx..self.bmdma[channel].buffer_top, 0);
            }
            self.bmdma[channel].buffer_top = remaining;
            self.bmdma[channel].buffer_idx = 0;

            // Advance to next PRD entry
            self.bmdma[channel].prd_current += 8;

            // Read next PRD size for timer period calculation
            let next_prd_size_raw =
                self.mem_read_physical_dword(self.bmdma[channel].prd_current + 4);
            let mut next_size = (next_prd_size_raw & 0xFFFE) as u32;
            if next_size == 0 {
                next_size = 0x10000;
            }
            // Bochs pci_ide.cc:335: (size >> 4) | 0x10
            let timer_period = (next_size >> 4) | 0x10;
            self.activate_channel_timer(channel, timer_period as u64);
        }
    }

    // ─── Timer Activation Helper ────────────────────────────────────────

    /// Activate the timer for a specific channel.
    /// Bochs: bx_pc_system.activate_timer() calls
    fn activate_channel_timer(&mut self, channel: usize, period: u64) {
        if self.pc_system_ptr.is_null() {
            return;
        }
        if let Some(handle) = self.bmdma[channel].timer_index {
            let pc_system = unsafe { &mut *self.pc_system_ptr };
            let _ = pc_system.activate_timer(handle, period, false);
        }
    }

    // ─── Physical Memory Access Helpers ─────────────────────────────────

    /// Read a dword from guest physical memory.
    /// Bochs: DEV_MEM_READ_PHYSICAL (pci_ide.cc:272-273)
    fn mem_read_physical_dword(&self, addr: u32) -> u32 {
        if self.ram_ptr.is_null() {
            return 0;
        }
        let a = addr as usize;
        if a + 4 > self.ram_len {
            return 0;
        }
        unsafe {
            let p = self.ram_ptr.add(a);
            u32::from_le_bytes([*p, *p.add(1), *p.add(2), *p.add(3)])
        }
    }

    /// Write DMA buffer data to guest physical memory.
    /// Bochs: DEV_MEM_WRITE_PHYSICAL_DMA (pci_ide.cc:289)
    fn mem_write_physical_dma(
        &self,
        guest_addr: u32,
        size: usize,
        buffer_offset: usize,
        channel: usize,
    ) {
        if self.ram_ptr.is_null() {
            return;
        }
        let a = guest_addr as usize;
        if a + size > self.ram_len {
            tracing::warn!("BM-DMA write out of bounds: addr={:#x} size={:#x}", guest_addr, size);
            return;
        }
        let src = &self.bmdma[channel].buffer[buffer_offset..buffer_offset + size];
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), self.ram_ptr.add(a), size);
        }
    }

    /// Read guest physical memory into DMA buffer.
    /// Bochs: DEV_MEM_READ_PHYSICAL_DMA (pci_ide.cc:294)
    fn mem_read_physical_dma(
        &mut self,
        guest_addr: u32,
        size: usize,
        buffer_offset: usize,
        channel: usize,
    ) {
        if self.ram_ptr.is_null() {
            return;
        }
        let a = guest_addr as usize;
        if a + size > self.ram_len {
            tracing::warn!("BM-DMA read out of bounds: addr={:#x} size={:#x}", guest_addr, size);
            return;
        }
        let dst = &mut self.bmdma[channel].buffer[buffer_offset..buffer_offset + size];
        unsafe {
            core::ptr::copy_nonoverlapping(self.ram_ptr.add(a), dst.as_mut_ptr(), size);
        }
    }

    // ─── Hard Drive DMA Callbacks ───────────────────────────────────────
    // These delegate to BxHardDriveC methods via raw pointer.

    /// Call harddrv.bmdma_read_sector() via raw pointer.
    /// Bochs: DEV_hd_bmdma_read_sector (pci_ide.cc:282)
    fn harddrv_bmdma_read_sector(&mut self, channel: u8, sector_size: &mut u32) -> bool {
        if self.harddrv_ptr.is_null() {
            return false;
        }
        let harddrv = unsafe { &mut *self.harddrv_ptr };
        let buffer_top = self.bmdma[channel as usize].buffer_top;
        harddrv.bmdma_read_sector(
            channel,
            &mut self.bmdma[channel as usize].buffer[buffer_top..],
            sector_size,
        )
    }

    /// Call harddrv.bmdma_write_sector() via raw pointer.
    /// Bochs: DEV_hd_bmdma_write_sector (pci_ide.cc:299)
    fn harddrv_bmdma_write_sector(&mut self, channel: u8) -> bool {
        if self.harddrv_ptr.is_null() {
            return false;
        }
        let harddrv = unsafe { &mut *self.harddrv_ptr };
        let buffer_idx = self.bmdma[channel as usize].buffer_idx;
        let buffer_end = buffer_idx + 512;
        let mut sector_buf = [0u8; 512];
        sector_buf.copy_from_slice(&self.bmdma[channel as usize].buffer[buffer_idx..buffer_end]);
        harddrv.bmdma_write_sector(channel, &sector_buf)
    }

    /// Call harddrv.bmdma_complete() via raw pointer.
    /// Bochs: DEV_hd_bmdma_complete (pci_ide.cc:291, 305, 311)
    fn harddrv_bmdma_complete(&self, channel: u8) {
        if self.harddrv_ptr.is_null() {
            return;
        }
        let harddrv = unsafe { &mut *self.harddrv_ptr };
        harddrv.bmdma_complete(channel);
    }

    // ─── BM-DMA I/O Read ─────────────────────────────────────────────────

    /// Read from BM-DMA register space.
    /// Bochs: bx_pci_ide_c::read() (pci_ide.cc:349-377)
    pub fn bmdma_read(&self, address: u16, _io_len: u8) -> u32 {
        if self.bmdma_base == 0 {
            return 0xFFFF_FFFF;
        }
        let offset = (address as u32).wrapping_sub(self.bmdma_base) as u8;
        let channel = (offset >> 3) as usize;
        let reg = offset & 0x07;

        if channel >= 2 {
            return 0xFFFF_FFFF;
        }

        match reg {
            // Command register (pci_ide.cc:361-364)
            0x00 => {
                let value = (self.bmdma[channel].cmd_ssbm as u32)
                    | ((self.bmdma[channel].cmd_rwcon as u32) << 3);
                tracing::debug!("BM-DMA read command ch={}, val={:#04x}", channel, value);
                value
            }
            // Status register (pci_ide.cc:366-369)
            0x02 => {
                let value = self.bmdma[channel].status as u32;
                tracing::debug!("BM-DMA read status ch={}, val={:#04x}", channel, value);
                value
            }
            // Descriptor Table Pointer (pci_ide.cc:370-373)
            0x04 => {
                let value = self.bmdma[channel].dtpr;
                tracing::debug!("BM-DMA read DTPR ch={}, val={:#010x}", channel, value);
                value
            }
            _ => 0xFFFF_FFFF,
        }
    }

    // ─── BM-DMA I/O Write ────────────────────────────────────────────────

    /// Write to BM-DMA register space.
    /// Bochs: bx_pci_ide_c::write() (pci_ide.cc:391-429)
    pub fn bmdma_write(&mut self, address: u16, value: u32, _io_len: u8) {
        if self.bmdma_base == 0 {
            return;
        }
        let offset = (address as u32).wrapping_sub(self.bmdma_base) as u8;
        let channel = (offset >> 3) as usize;
        let reg = offset & 0x07;

        if channel >= 2 {
            return;
        }

        match reg {
            // Command register (pci_ide.cc:402-417)
            0x00 => {
                tracing::debug!("BM-DMA write command ch={}, val={:#04x}", channel, value);
                self.bmdma[channel].cmd_rwcon = (value >> 3) & 1 != 0;
                if (value & 0x01 != 0) && !self.bmdma[channel].cmd_ssbm {
                    // Start DMA — Bochs pci_ide.cc:405-412
                    self.bmdma[channel].cmd_ssbm = true;
                    self.bmdma[channel].status |= 0x01;
                    self.bmdma[channel].prd_current = self.bmdma[channel].dtpr;
                    self.bmdma[channel].buffer_top = 0;
                    self.bmdma[channel].buffer_idx = 0;
                    tracing::info!(
                        "BM-DMA start ch={}, DTPR={:#010x}, rwcon={}",
                        channel,
                        self.bmdma[channel].dtpr,
                        if self.bmdma[channel].cmd_rwcon { "read" } else { "write" },
                    );
                    // Activate timer with period=1 (fires ASAP)
                    // Bochs pci_ide.cc:411
                    self.activate_channel_timer(channel, 1);
                } else if (value & 0x01 == 0) && self.bmdma[channel].cmd_ssbm {
                    // Stop DMA — Bochs pci_ide.cc:413-416
                    self.bmdma[channel].cmd_ssbm = false;
                    self.bmdma[channel].status &= !0x01;
                    self.bmdma[channel].data_ready = false;
                    tracing::info!("BM-DMA stop ch={}", channel);
                }
            }
            // Status register — write (pci_ide.cc:418-423)
            0x02 => {
                tracing::debug!("BM-DMA write status ch={}, val={:#04x}", channel, value);
                // Bits 5-6 (simplex): writable
                // Bit 0 (active): read-only (preserved)
                // Bits 1-2 (error/IRQ): write-1-to-clear
                self.bmdma[channel].status = ((value as u8) & 0x60)
                    | (self.bmdma[channel].status & 0x01)
                    | (self.bmdma[channel].status & (!(value as u8) & 0x06));
            }
            // Descriptor Table Pointer (pci_ide.cc:424-427)
            0x04 => {
                self.bmdma[channel].dtpr = value & 0xFFFF_FFFC; // aligned to 4 bytes
                tracing::debug!(
                    "BM-DMA write DTPR ch={}, val={:#010x}",
                    channel,
                    self.bmdma[channel].dtpr
                );
            }
            _ => {}
        }
    }

    /// Get the I/O access mask for a BM-DMA register offset.
    pub fn bmdma_io_mask(&self, offset: u8) -> u8 {
        if (offset as usize) < BMDMA_IOMASK.len() {
            BMDMA_IOMASK[offset as usize]
        } else {
            0
        }
    }

    // ─── PCI Configuration Space ─────────────────────────────────────────

    /// Write to PCI configuration space.
    /// Bochs: bx_pci_ide_c::pci_write_handler() (pci_ide.cc:433-457)
    pub fn pci_write(&mut self, address: u8, value: u32, io_len: u8) -> bool {
        let mut bar4_changed = false;

        // BAR0-BAR3 and some reserved ranges are read-only (pci_ide.cc:435-437)
        if (address >= 0x10 && address < 0x20) || (address > 0x23 && address < 0x40) {
            return false;
        }

        for i in 0..io_len as usize {
            let addr = address as usize + i;
            if addr >= PCI_CONF_SIZE {
                break;
            }
            let value8 = ((value >> (i * 8)) & 0xFF) as u8;
            let oldval = self.pci_conf[addr];

            match addr {
                // Status registers — read-only (pci_ide.cc:444-446)
                0x05 | 0x06 => {}
                // Command register (pci_ide.cc:447-448)
                0x04 => {
                    self.pci_conf[addr] = value8 & 0x05;
                }
                // BAR4 (BM-DMA base address)
                0x20..=0x23 => {
                    bar4_changed |= value8 != oldval;
                    self.pci_conf[addr] = value8;
                    // Log BAR4 writes to track kernel reassignment
                    if bar4_changed {
                        let cur = u32::from_le_bytes([
                            self.pci_conf[0x20], self.pci_conf[0x21],
                            self.pci_conf[0x22], self.pci_conf[0x23],
                        ]);
                        tracing::debug!("PCI IDE: BAR4 write byte[{}]={:#04x} → raw={:#010x}",
                            addr, value8, cur);
                    }
                }
                // Default: store (pci_ide.cc:450-453)
                _ => {
                    self.pci_conf[addr] = value8;
                }
            }
        }

        // Update BAR4 if changed
        if bar4_changed {
            let new_base = u32::from_le_bytes([
                self.pci_conf[0x20],
                self.pci_conf[0x21],
                self.pci_conf[0x22],
                self.pci_conf[0x23],
            ]) & 0xFFF0; // Align to 16 ports
            if new_base != self.bmdma_base {
                self.bmdma_base = new_base;
                tracing::info!("PCI IDE: new BM-DMA base address: {:#06x}", self.bmdma_base);
            }
        }

        bar4_changed
    }

    /// Read from PCI configuration space.
    pub fn pci_read(&self, address: u8, io_len: u8) -> u32 {
        let mut value: u32 = 0;
        for i in 0..io_len as usize {
            let addr = address as usize + i;
            if addr < PCI_CONF_SIZE {
                value |= (self.pci_conf[addr] as u32) << (i * 8);
            }
        }
        value
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pci_ide_new() {
        let ide = BxPciIde::new();
        // Vendor: Intel
        assert_eq!(ide.pci_conf[0x00], 0x86);
        assert_eq!(ide.pci_conf[0x01], 0x80);
        // Device: PIIX3 IDE
        assert_eq!(ide.pci_conf[0x02], 0x10);
        assert_eq!(ide.pci_conf[0x03], 0x70);
        // Class: IDE controller
        assert_eq!(ide.pci_conf[0x0B], 0x01);
        assert_eq!(ide.pci_conf[0x0A], 0x01);
        // DMA base not set
        assert_eq!(ide.bmdma_base, 0);
    }

    #[test]
    fn test_pci_ide_reset() {
        let mut ide = BxPciIde::new();
        ide.bmdma[0].cmd_ssbm = true;
        ide.bmdma[0].status = 0xFF;
        ide.reset();
        assert!(!ide.bmdma[0].cmd_ssbm);
        assert_eq!(ide.bmdma[0].status, 0);
        assert_eq!(ide.pci_conf[0x04], 0x01); // I/O enabled
        assert_eq!(ide.pci_conf[0x41], 0x80); // Channel 0 enabled
    }

    #[test]
    fn test_bmdma_status_write_clear() {
        let mut ide = BxPciIde::new();
        ide.bmdma_base = 0xC000;
        ide.bmdma[0].status = 0x05; // active + IRQ pending

        // Write 1 to bit 2 (IRQ) to clear it, but active (bit 0) is preserved
        ide.bmdma_write(0xC002, 0x04, 1);
        assert_eq!(ide.bmdma[0].status, 0x01); // active preserved, IRQ cleared
    }

    #[test]
    fn test_bmdma_dtpr_alignment() {
        let mut ide = BxPciIde::new();
        ide.bmdma_base = 0xC000;
        ide.bmdma_write(0xC004, 0xDEADBEEF, 4);
        // Low 2 bits should be masked
        assert_eq!(ide.bmdma[0].dtpr, 0xDEADBEEC);
    }

    #[test]
    fn test_bar4_pci_write() {
        let mut ide = BxPciIde::new();
        ide.reset();
        // Write BAR4 = 0xC001 (I/O type indicator in bit 0)
        let changed = ide.pci_write(0x20, 0x0000C001, 4);
        assert!(changed);
        assert_eq!(ide.bmdma_base, 0xC000); // masked to 16-port alignment
    }

    #[test]
    fn test_bmdma_start_stop() {
        let mut ide = BxPciIde::new();
        ide.bmdma_base = 0xC000;
        ide.bmdma[0].dtpr = 0x1000;

        // Start DMA (timer activation will be no-op since pc_system_ptr is null)
        ide.bmdma_write(0xC000, 0x01, 1);
        assert!(ide.bmdma[0].cmd_ssbm);
        assert_eq!(ide.bmdma[0].status & 0x01, 0x01);
        assert_eq!(ide.bmdma[0].prd_current, 0x1000);

        // Stop DMA
        ide.bmdma_write(0xC000, 0x00, 1);
        assert!(!ide.bmdma[0].cmd_ssbm);
        assert_eq!(ide.bmdma[0].status & 0x01, 0x00);
    }
}
