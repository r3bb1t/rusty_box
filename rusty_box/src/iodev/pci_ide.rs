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
//!
//! DLX Linux uses PIO mode (not DMA), so the BM-DMA registers are
//! present for BIOS detection/configuration but won't be exercised
//! during DLX boot. The DMA transfer logic is stubbed for now.

use alloc::vec;
use alloc::vec::Vec;

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
#[derive(Debug)]
pub struct BxPciIde {
    /// PCI configuration space (256 bytes)
    pub pci_conf: [u8; PCI_CONF_SIZE],

    /// BM-DMA state for 2 channels (primary and secondary)
    pub bmdma: [BmDmaChannel; 2],

    /// BAR4 I/O base address (BM-DMA registers)
    pub bmdma_base: u32,
}

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
        self.pci_conf[0x04] = 0x01; // I/O space enabled
        self.pci_conf[0x06] = 0x80;
        self.pci_conf[0x07] = 0x02;
        // IDE timing registers (pci_ide.cc:130-136)
        self.pci_conf[0x40] = 0x00;
        self.pci_conf[0x41] = 0x80; // Channel 0 enabled
        self.pci_conf[0x42] = 0x00;
        self.pci_conf[0x43] = 0x80; // Channel 1 enabled
        self.pci_conf[0x44] = 0x00;

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
                    // Start DMA
                    self.bmdma[channel].cmd_ssbm = true;
                    self.bmdma[channel].status |= 0x01;
                    self.bmdma[channel].prd_current = self.bmdma[channel].dtpr;
                    self.bmdma[channel].buffer_top = 0;
                    self.bmdma[channel].buffer_idx = 0;
                    tracing::info!(
                        "BM-DMA start ch={}, DTPR={:#010x}",
                        channel,
                        self.bmdma[channel].dtpr
                    );
                    // Note: In Bochs, a timer is activated here to process PRDs
                    // For now, DMA processing is not implemented (DLX uses PIO)
                } else if (value & 0x01 == 0) && self.bmdma[channel].cmd_ssbm {
                    // Stop DMA
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

        // Start DMA
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
