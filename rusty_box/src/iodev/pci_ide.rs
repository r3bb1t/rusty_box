#![allow(unused_variables)]
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
//! - Timer-driven DMA transfers (Bochs pci_ide.cc)

/// PCI configuration space size
const PCI_CONF_SIZE: usize = 256;

/// BM-DMA I/O mask for the 16-port register block.
/// Bochs: bmdma_iomask[16] (pci_ide.cc)
const BMDMA_IOMASK: [u8; 16] = [1, 0, 1, 0, 4, 0, 0, 0, 1, 0, 1, 0, 4, 0, 0, 0];

/// BM-DMA buffer size per channel (128 KB)
const BMDMA_BUFFER_SIZE: usize = 0x20000;

/// BM-DMA channel state.
/// Bochs: pci_ide.h
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
    pub buffer: [u8; BMDMA_BUFFER_SIZE],
    /// Buffer write pointer offset
    pub buffer_top: usize,
    /// Buffer read pointer offset
    pub buffer_idx: usize,
    /// Data ready flag (set when disk has data for DMA transfer)
    pub data_ready: bool,
    /// Timer handle for pc_system (Bochs pci_ide.h)
    pub timer_index: Option<usize>,
}

impl core::fmt::Debug for BmDmaChannel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BmDmaChannel")
            .field("cmd_ssbm", &self.cmd_ssbm)
            .field("cmd_rwcon", &self.cmd_rwcon)
            .field("status", &self.status)
            .field("dtpr", &self.dtpr)
            .field("prd_current", &self.prd_current)
            .field("buffer", &format_args!("[u8; {}]", self.buffer.len()))
            .field("buffer_top", &self.buffer_top)
            .field("buffer_idx", &self.buffer_idx)
            .field("data_ready", &self.data_ready)
            .field("timer_index", &self.timer_index)
            .finish()
    }
}

impl BmDmaChannel {
    fn new() -> Self {
        Self {
            cmd_ssbm: false,
            cmd_rwcon: false,
            status: 0,
            dtpr: 0,
            prd_current: 0,
            buffer: [0u8; BMDMA_BUFFER_SIZE],
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
/// Bochs: bx_pci_ide_c (pci_ide.h, pci_ide.cc)
#[derive(Debug)]
pub struct BxPciIde {
    /// PCI configuration space (256 bytes)
    pub pci_conf: [u8; PCI_CONF_SIZE],

    /// BM-DMA state for 2 channels (primary and secondary)
    pub bmdma: [BmDmaChannel; 2],

    /// BAR4 I/O base address (BM-DMA registers)
    pub bmdma_base: u32,

    /// Deferred one-shot timer arm request per channel, in microseconds.
    /// Set by the BM-DMA command-register write (Bochs pci_ide.cc write:
    /// `bx_pc_system.activate_timer(timer_index, 1, 0)`); drained by the
    /// emulator loop, which owns `BxPcSystemC`. The I/O dispatch path has no
    /// pc_system access, so the arm is deferred by at most one CPU batch.
    pub(crate) pending_timer_arm: [Option<u32>; 2],
}

impl Default for BxPciIde {
    fn default() -> Self {
        Self::new()
    }
}

impl BxPciIde {
    /// Create a new PCI IDE controller.
    /// Bochs: bx_pci_ide_c::init() (pci_ide.cc)
    pub fn new() -> Self {
        let mut ide = Self {
            pci_conf: [0; PCI_CONF_SIZE],
            bmdma: [BmDmaChannel::new(), BmDmaChannel::new()],
            bmdma_base: 0,
            pending_timer_arm: [None; 2],
        };
        ide.init_pci_conf();
        ide
    }

    /// Initialize PCI configuration space with PIIX3 IDE identity.
    /// Bochs: init_pci_conf(0x8086, 0x7010, 0x00, 0x010180, 0x00, 0) (pci_ide.cc)
    fn init_pci_conf(&mut self) {
        // Vendor ID: Intel (0x8086)
        self.pci_conf[0x00] = 0x86;
        self.pci_conf[0x01] = 0x80;
        // Device ID: PIIX3 IDE (0x7010)
        self.pci_conf[0x02] = 0x10;
        self.pci_conf[0x03] = 0x70;
        // Revision: 0x00
        self.pci_conf[0x08] = 0x00;
        // Class code 0x010180: IDE controller, ISA-compatible, bus-master
        // capable (prog-if 0x80). Bochs pci_ide.cc init_pci_conf(0x8086,
        // 0x7010, 0x00, 0x010180, 0x00, 0).
        self.pci_conf[0x09] = 0x80;
        self.pci_conf[0x0A] = 0x01;
        self.pci_conf[0x0B] = 0x01;
        // Header type: single function (but shared with ISA bridge)
        self.pci_conf[0x0E] = 0x00;
    }

    /// Reset the PCI IDE controller.
    /// Bochs: bx_pci_ide_c::reset() (pci_ide.cc)
    pub fn reset(&mut self) {
        self.pci_conf[0x04] = 0x01; // I/O space enabled (no bus master until DMA works)
        self.pci_conf[0x06] = 0x80;
        self.pci_conf[0x07] = 0x02;
        // IDE timing registers (pci_ide.cc)
        self.pci_conf[0x40] = 0x00;
        self.pci_conf[0x41] = 0x80; // Channel 0 enabled
        self.pci_conf[0x42] = 0x00;
        self.pci_conf[0x43] = 0x80; // Channel 1 enabled
        self.pci_conf[0x44] = 0x00;

        // BAR4 (pci_conf[0x20..0x24]) and bmdma_base are NOT reset: BAR
        // assignments persist across reset, matching Bochs where reset()
        // leaves pci_bar[] untouched (pci_ide.cc reset).

        // Reset BM-DMA state
        for ch in self.bmdma.iter_mut() {
            ch.reset();
        }
        self.pending_timer_arm = [None; 2];
    }

    /// Check if BM-DMA is present (BAR4 configured).
    /// Bochs: bx_pci_ide_c::bmdma_present() (pci_ide.cc)
    pub fn bmdma_present(&self) -> bool {
        self.bmdma_base > 0
    }

    /// Signal that data is ready for DMA transfer on a channel.
    /// Bochs: bx_pci_ide_c::bmdma_start_transfer() (pci_ide.cc)
    pub fn bmdma_start_transfer(&mut self, channel: u8) {
        if (channel as usize) < 2 {
            self.bmdma[channel as usize].data_ready = true;
        }
    }

    /// Set IRQ pending bit in BM-DMA status register.
    /// Bochs: bx_pci_ide_c::bmdma_set_irq() (pci_ide.cc)
    pub fn bmdma_set_irq(&mut self, channel: u8) {
        if (channel as usize) < 2 {
            self.bmdma[channel as usize].status |= 0x04;
        }
    }

    /// Drain a deferred one-shot timer arm request (microseconds) for a
    /// channel. Set by the BM-DMA command-register write; the emulator loop
    /// (which owns `BxPcSystemC`) drains it and activates the channel timer.
    pub(crate) fn take_pending_timer_arm(&mut self, channel: usize) -> Option<u32> {
        if channel < 2 {
            self.pending_timer_arm[channel].take()
        } else {
            None
        }
    }

    // ─── BM-DMA I/O Read ─────────────────────────────────────────────────

    /// Read from BM-DMA register space.
    /// Bochs: bx_pci_ide_c::read() (pci_ide.cc)
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
            // Command register (pci_ide.cc)
            0x00 => {
                let value = (self.bmdma[channel].cmd_ssbm as u32)
                    | ((self.bmdma[channel].cmd_rwcon as u32) << 3);
                tracing::trace!("BM-DMA read command ch={}, val={:#04x}", channel, value);
                value
            }
            // Status register (pci_ide.cc)
            0x02 => {
                let value = self.bmdma[channel].status as u32;
                tracing::trace!("BM-DMA read status ch={}, val={:#04x}", channel, value);
                value
            }
            // Descriptor Table Pointer (pci_ide.cc)
            0x04 => {
                let value = self.bmdma[channel].dtpr;
                tracing::trace!("BM-DMA read DTPR ch={}, val={:#010x}", channel, value);
                value
            }
            _ => 0xFFFF_FFFF,
        }
    }

    // ─── BM-DMA I/O Write ────────────────────────────────────────────────

    /// Write to BM-DMA register space.
    /// Bochs: bx_pci_ide_c::write() (pci_ide.cc)
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
            // Command register (pci_ide.cc)
            0x00 => {
                tracing::trace!("BM-DMA write command ch={}, val={:#04x}", channel, value);
                self.bmdma[channel].cmd_rwcon = (value >> 3) & 1 != 0;
                if (value & 0x01 != 0) && !self.bmdma[channel].cmd_ssbm {
                    // Start DMA — Bochs pci_ide.cc
                    self.bmdma[channel].cmd_ssbm = true;
                    self.bmdma[channel].status |= 0x01;
                    self.bmdma[channel].prd_current = self.bmdma[channel].dtpr;
                    self.bmdma[channel].buffer_top = 0;
                    self.bmdma[channel].buffer_idx = 0;
                    tracing::debug!(
                        "BM-DMA start ch={}, DTPR={:#010x}, rwcon={}",
                        channel,
                        self.bmdma[channel].dtpr,
                        if self.bmdma[channel].cmd_rwcon {
                            "read"
                        } else {
                            "write"
                        },
                    );
                    // Bochs pci_ide.cc write:
                    // bx_pc_system.activate_timer(timer_index, 1, 0).
                    // Deferred: the emulator loop drains this and arms the
                    // one-shot (I/O dispatch has no pc_system access).
                    self.pending_timer_arm[channel] = Some(1);
                } else if (value & 0x01 == 0) && self.bmdma[channel].cmd_ssbm {
                    // Stop DMA — Bochs pci_ide.cc
                    self.bmdma[channel].cmd_ssbm = false;
                    self.bmdma[channel].status &= !0x01;
                    self.bmdma[channel].data_ready = false;
                    tracing::debug!("BM-DMA stop ch={}", channel);
                }
            }
            // Status register — write (pci_ide.cc)
            0x02 => {
                tracing::trace!("BM-DMA write status ch={}, val={:#04x}", channel, value);
                // Bits 5-6 (simplex): writable
                // Bit 0 (active): read-only (preserved)
                // Bits 1-2 (error/IRQ): write-1-to-clear
                self.bmdma[channel].status = ((value as u8) & 0x60)
                    | (self.bmdma[channel].status & 0x01)
                    | (self.bmdma[channel].status & (!(value as u8) & 0x06));
            }
            // Descriptor Table Pointer (pci_ide.cc)
            0x04 => {
                self.bmdma[channel].dtpr = value & 0xFFFF_FFFC; // aligned to 4 bytes
                tracing::trace!(
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
    /// Bochs: bx_pci_ide_c::pci_write_handler() (pci_ide.cc)
    #[inline(never)]
    pub fn pci_write(&mut self, address: u8, mut value: u32, io_len: u8) -> bool {
        // BAR0-BAR3 and reserved 0x24..0x40 are read-only (Bochs pci_ide.cc
        // pci_write_handler skips 0x10..0x20 and 0x24..0x40; BAR4 at
        // 0x20..0x24 IS writable — the 16-port BM-DMA I/O BAR).
        if (0x10..0x20).contains(&address) || (address > 0x23 && address < 0x40) {
            return false;
        }

        // BAR4 size probe: a full-dword write of >= 0xfffffff0 must read back
        // the size mask for the 16-port I/O BAR. Bochs devices.cc
        // pci_write_handler_common (generic BAR sizing for init_bar_io(4, 16)).
        const BAR4_SIZE: u32 = 16;
        let is_bar4 = (0x20..0x24).contains(&address);
        let mut probe = false;
        if is_bar4 && value >= 0xffff_fff0 {
            value = (value & !(BAR4_SIZE - 1)) | 0x01; // I/O-space type bit
            probe = true;
        }

        let mut bar4_written = false;
        for i in 0..io_len as usize {
            let addr = address as usize + i;
            if addr >= PCI_CONF_SIZE {
                break;
            }
            let mut value8 = ((value >> (i * 8)) & 0xFF) as u8;
            let oldval = self.pci_conf[addr];

            match addr {
                // Status registers — read-only (pci_ide.cc)
                0x05 | 0x06 => {}
                // Command register (pci_ide.cc): I/O enable + bus master only
                0x04 => {
                    self.pci_conf[addr] = value8 & 0x05;
                }
                // BAR4 low byte: keep the I/O-space type bit (Bochs
                // devices.cc pci_write_handler_common BAR type preservation).
                0x20 => {
                    value8 = (value8 & 0xF0) | 0x01;
                    if value8 != oldval {
                        bar4_written = true;
                    }
                    self.pci_conf[addr] = value8;
                }
                // BAR4 upper bytes: stored verbatim
                0x21..=0x23 => {
                    if value8 != oldval {
                        bar4_written = true;
                    }
                    self.pci_conf[addr] = value8;
                }
                // Default: store (pci_ide.cc)
                _ => {
                    self.pci_conf[addr] = value8;
                }
            }
        }

        // Commit the new BM-DMA base — never on a size probe (the probe value
        // is transient; the BIOS writes the real base right after).
        let mut bar4_changed = false;
        if bar4_written && !probe {
            let new_base = u32::from_le_bytes([
                self.pci_conf[0x20],
                self.pci_conf[0x21],
                self.pci_conf[0x22],
                self.pci_conf[0x23],
            ]) & !(BAR4_SIZE - 1);
            if new_base != self.bmdma_base {
                self.bmdma_base = new_base;
                bar4_changed = true;
                tracing::debug!("PCI IDE: new BM-DMA base address: {:#06x}", self.bmdma_base);
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
    fn test_bar4_pci_write_commits_base() {
        let mut ide = BxPciIde::new();
        ide.reset();
        // BIOS assigns the 16-port I/O BAR (Bochs init_bar_io(4, 16, ...)).
        let changed = ide.pci_write(0x20, 0x0000C001, 4);
        assert!(changed);
        assert_eq!(ide.bmdma_base, 0xC000);
        assert!(ide.bmdma_present());
        // Low nibble keeps the I/O-space type bit.
        assert_eq!(ide.pci_conf[0x20] & 0x0F, 0x01);
    }

    #[test]
    fn test_bar4_size_probe_never_commits() {
        let mut ide = BxPciIde::new();
        ide.reset();
        // Size probe: full-ones write must read back the 16-port size mask
        // and must NOT move the committed base.
        let changed = ide.pci_write(0x20, 0xFFFF_FFFF, 4);
        assert!(!changed);
        assert_eq!(ide.bmdma_base, 0);
        assert_eq!(ide.pci_read(0x20, 4), 0xFFFF_FFF1);
        // The real base write right after the probe commits normally.
        let changed = ide.pci_write(0x20, 0x0000C001, 4);
        assert!(changed);
        assert_eq!(ide.bmdma_base, 0xC000);
    }

    #[test]
    fn test_bar4_base_survives_reset() {
        let mut ide = BxPciIde::new();
        ide.reset();
        assert!(ide.pci_write(0x20, 0x0000C001, 4));
        ide.reset();
        // Bochs pci_ide.cc reset() leaves BAR assignments untouched.
        assert_eq!(ide.bmdma_base, 0xC000);
        assert!(ide.bmdma_present());
    }

    #[test]
    fn test_prog_if_advertises_bus_master() {
        let ide = BxPciIde::new();
        // Class code 0x010180 — prog-if bit 7 (bus master capable). Linux
        // ata_piix only probes BAR4 when this bit is set.
        assert_eq!(ide.pci_conf[0x09], 0x80);
        assert_eq!(ide.pci_conf[0x0A], 0x01);
        assert_eq!(ide.pci_conf[0x0B], 0x01);
    }

    #[test]
    fn test_bmdma_start_stop() {
        let mut ide = BxPciIde::new();
        ide.bmdma_base = 0xC000;
        ide.bmdma[0].dtpr = 0x1000;

        // Start DMA — requests the deferred 1 us one-shot timer arm
        // (Bochs pci_ide.cc write: activate_timer(timer_index, 1, 0)).
        ide.bmdma_write(0xC000, 0x01, 1);
        assert!(ide.bmdma[0].cmd_ssbm);
        assert_eq!(ide.bmdma[0].status & 0x01, 0x01);
        assert_eq!(ide.bmdma[0].prd_current, 0x1000);
        assert_eq!(ide.take_pending_timer_arm(0), Some(1));
        assert_eq!(ide.take_pending_timer_arm(0), None); // drained

        // Stop DMA
        ide.bmdma_write(0xC000, 0x00, 1);
        assert!(!ide.bmdma[0].cmd_ssbm);
        assert_eq!(ide.bmdma[0].status & 0x01, 0x00);
        assert!(!ide.bmdma[0].data_ready);
    }
}
