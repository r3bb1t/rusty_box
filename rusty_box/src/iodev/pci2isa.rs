//! PIIX3 PCI-to-ISA Bridge
//!
//! Matches Bochs `iodev/pci2isa.cc` (463 lines) + `pci2isa.h` (69 lines).
//!
//! Implements:
//! - PCI-to-ISA bridge (PIIX3) — bus 0, device 1, function 0
//! - PCI IRQ routing: 4 PIRQ lines (A/B/C/D) → ISA IRQs via config 0x60-0x63
//! - Edge/Level Control Registers (ELCR) at ports 0x04D0-0x04D1
//! - APM (Advanced Power Management) ports 0x00B2-0x00B3
//! - CPU reset register at port 0x0CF9
//! - BIOS write enable and ROM access control
//!
//! The PIIX3 bridge routes PCI interrupts (INTA#-INTD#) to ISA IRQs
//! based on PIRQ routing registers. Each PIRQ can be mapped to any
//! ISA IRQ or disabled (bit 7 = 1).

/// PCI configuration space size
const PCI_CONF_SIZE: usize = 256;

/// APM command port (Bochs pci2isa.cc:83)
pub const APM_CMD_PORT: u16 = 0x00B2;
/// APM status port (Bochs pci2isa.cc:84)
pub const APM_STS_PORT: u16 = 0x00B3;
/// ELCR1 — Edge/Level Control Register for master PIC (Bochs pci2isa.cc:85)
pub const ELCR1_PORT: u16 = 0x04D0;
/// ELCR2 — Edge/Level Control Register for slave PIC (Bochs pci2isa.cc:86)
pub const ELCR2_PORT: u16 = 0x04D1;
/// CPU reset register (Bochs pci2isa.cc:87)
pub const PCI_RESET_PORT: u16 = 0x0CF9;

/// Valid IRQ mask for PCI routing (Bochs pci2isa.cc:212)
/// Bits set for IRQs that can be used: 3,4,5,6,7,9,10,11,12,14,15
const VALID_PCI_IRQ_MASK: u16 = 0xDEF8;

/// PIIX3 PCI-to-ISA bridge state.
/// Bochs: bx_piix3_c (pci2isa.h:33-66)
#[derive(Debug)]
pub struct BxPiix3 {
    /// PCI device/function number (PIIX3: bus 0, dev 1, func 0 = 0x08)
    pub devfunc: u8,

    /// PCI configuration space (256 bytes)
    pub pci_conf: [u8; PCI_CONF_SIZE],

    /// Edge/Level Control Register 1 (master PIC IRQs 0-7)
    /// Bochs: s.elcr1 (pci2isa.h:53)
    pub elcr1: u8,
    /// Edge/Level Control Register 2 (slave PIC IRQs 8-15)
    /// Bochs: s.elcr2 (pci2isa.h:54)
    pub elcr2: u8,

    /// APM command register (Bochs: s.apmc, pci2isa.h:55)
    pub apmc: u8,
    /// APM status register (Bochs: s.apms, pci2isa.h:56)
    pub apms: u8,

    /// PCI IRQ level tracking: [pirq_line][irq_number]
    /// Bochs: s.irq_level[4][16] (pci2isa.h:57)
    /// Each entry is a bitmask of which devices are asserting that IRQ through that PIRQ
    pub irq_level: [[u32; 16]; 4],

    /// CPU reset register (Bochs: s.pci_reset, pci2isa.h:58)
    pub pci_reset: u8,

    /// Flag: ELCR1 changed — emulator should call pic.set_mode()
    pub elcr1_changed: bool,
    /// Flag: ELCR2 changed — emulator should call pic.set_mode()
    pub elcr2_changed: bool,
    /// Flag: reset requested — emulator should handle CPU reset
    pub reset_request: Option<bool>, // Some(true) = hardware, Some(false) = software
}

impl Default for BxPiix3 {
    fn default() -> Self {
        Self::new()
    }
}

impl BxPiix3 {
    /// Create a new PIIX3 bridge.
    /// Bochs: bx_piix3_c::init() (pci2isa.cc:69-118)
    pub fn new() -> Self {
        let mut bridge = Self {
            devfunc: super::pci::pci_device(1, 0), // 0x08
            pci_conf: [0; PCI_CONF_SIZE],
            elcr1: 0,
            elcr2: 0,
            apmc: 0,
            apms: 0,
            irq_level: [[0; 16]; 4],
            pci_reset: 0,
            elcr1_changed: false,
            elcr2_changed: false,
            reset_request: None,
        };
        bridge.init_pci_conf();
        bridge
    }

    /// Initialize PCI configuration space with PIIX3 identity.
    /// Bochs: init_pci_conf(0x8086, 0x7000, 0x00, 0x060100, 0x80, 0) (pci2isa.cc:106)
    fn init_pci_conf(&mut self) {
        // Vendor ID: Intel (0x8086)
        self.pci_conf[0x00] = 0x86;
        self.pci_conf[0x01] = 0x80;
        // Device ID: PIIX3 (0x7000)
        self.pci_conf[0x02] = 0x00;
        self.pci_conf[0x03] = 0x70;
        // Revision: 0x00
        self.pci_conf[0x08] = 0x00;
        // Class code: ISA bridge (0x060100)
        self.pci_conf[0x09] = 0x00;
        self.pci_conf[0x0A] = 0x01;
        self.pci_conf[0x0B] = 0x06;
        // Header type: 0x80 (multi-function)
        self.pci_conf[0x0E] = 0x80;
        // Command register
        self.pci_conf[0x04] = 0x07;
        // PIRQ routing: disabled (bit 7 set)
        self.pci_conf[0x60] = 0x80;
        self.pci_conf[0x61] = 0x80;
        self.pci_conf[0x62] = 0x80;
        self.pci_conf[0x63] = 0x80;
    }

    /// Reset the PIIX3 bridge.
    /// Bochs: bx_piix3_c::reset() (pci2isa.cc:120-159)
    pub fn reset(&mut self) {
        self.pci_conf[0x05] = 0x00;
        self.pci_conf[0x06] = 0x00;
        self.pci_conf[0x07] = 0x02;
        self.pci_conf[0x4C] = 0x4D;
        self.pci_conf[0x4E] = 0x03;
        self.pci_conf[0x4F] = 0x00;
        self.pci_conf[0x69] = 0x02;
        self.pci_conf[0x70] = 0x80;
        self.pci_conf[0x76] = 0x0C;
        self.pci_conf[0x77] = 0x0C;
        self.pci_conf[0x78] = 0x02;
        self.pci_conf[0x79] = 0x00;
        self.pci_conf[0x80] = 0x00;
        self.pci_conf[0x82] = 0x00;
        self.pci_conf[0xA0] = 0x08;
        self.pci_conf[0xA2] = 0x00;
        self.pci_conf[0xA3] = 0x00;
        self.pci_conf[0xA4] = 0x00;
        self.pci_conf[0xA5] = 0x00;
        self.pci_conf[0xA6] = 0x00;
        self.pci_conf[0xA7] = 0x00;
        self.pci_conf[0xA8] = 0x0F;
        self.pci_conf[0xAA] = 0x00;
        self.pci_conf[0xAB] = 0x00;
        self.pci_conf[0xAC] = 0x00;
        self.pci_conf[0xAE] = 0x00;

        // Reset PIRQ routing to disabled (pci2isa.cc:150-152)
        for i in 0..4 {
            self.pci_conf[0x60 + i] = 0x80;
        }

        self.elcr1 = 0x00;
        self.elcr2 = 0x00;
        self.pci_reset = 0x00;
        self.apms = 0x00;
        self.apmc = 0x00;
        self.elcr1_changed = false;
        self.elcr2_changed = false;
        self.reset_request = None;

        // Clear IRQ levels
        self.irq_level = [[0; 16]; 4];
    }

    // ─── I/O Port Read Handler ───────────────────────────────────────────

    /// Read from PCI-to-ISA bridge I/O ports.
    /// Bochs: bx_piix3_c::read() (pci2isa.cc:241-265)
    pub fn read(&self, address: u16) -> u32 {
        match address {
            0x00B2 => self.apmc as u32,
            0x00B3 => self.apms as u32,
            0x04D0 => self.elcr1 as u32,
            0x04D1 => self.elcr2 as u32,
            0x0CF9 => self.pci_reset as u32,
            _ => 0xFFFF_FFFF,
        }
    }

    /// Write to PCI-to-ISA bridge I/O ports.
    /// Bochs: bx_piix3_c::write() (pci2isa.cc:277-326)
    pub fn write(&mut self, address: u16, value: u32, io_len: u8) {
        match address {
            // APM command port (pci2isa.cc:284-293)
            0x00B2 => {
                // Note: In Bochs this forwards to ACPI generate_smi()
                // In our architecture, the ACPI device also listens on 0xB2
                self.apmc = value as u8;
                if io_len == 2 {
                    self.apms = (value >> 8) as u8;
                }
            }
            // APM status port (pci2isa.cc:295-296)
            0x00B3 => {
                self.apms = value as u8;
            }
            // ELCR1 — master PIC edge/level (pci2isa.cc:298-304)
            0x04D0 => {
                let v = (value as u8) & 0xF8; // bits 0-2 always edge
                if v != self.elcr1 {
                    self.elcr1 = v;
                    self.elcr1_changed = true;
                    tracing::info!("ELCR1 = {:#04x}", self.elcr1);
                }
            }
            // ELCR2 — slave PIC edge/level (pci2isa.cc:306-312)
            0x04D1 => {
                let v = (value as u8) & 0xDE; // bits 0 and 5 always edge
                if v != self.elcr2 {
                    self.elcr2 = v;
                    self.elcr2_changed = true;
                    tracing::info!("ELCR2 = {:#04x}", self.elcr2);
                }
            }
            // CPU reset register (pci2isa.cc:314-324)
            0x0CF9 => {
                tracing::info!("CPU reset register write: {:#04x}", value);
                self.pci_reset = (value as u8) & 0x02;
                if (value as u8) & 0x04 != 0 {
                    if self.pci_reset != 0 {
                        self.reset_request = Some(true); // hardware reset
                    } else {
                        self.reset_request = Some(false); // software reset
                    }
                }
            }
            _ => {}
        }
    }

    // ─── PCI Configuration Space ─────────────────────────────────────────

    /// Write to PCI configuration space.
    /// Bochs: bx_piix3_c::pci_write_handler() (pci2isa.cc:329-420)
    pub fn pci_write(&mut self, address: u8, value: u32, io_len: u8) {
        // BARs are read-only
        if address >= 0x10 && address < 0x34 {
            return;
        }

        for i in 0..io_len as usize {
            let addr = address as usize + i;
            if addr >= PCI_CONF_SIZE {
                break;
            }
            let value8 = ((value >> (i * 8)) & 0xFF) as u8;
            let oldval = self.pci_conf[addr];

            match addr {
                // Command register (pci2isa.cc:342-343)
                0x04 => {
                    self.pci_conf[addr] = (value8 & 0x08) | 0x07;
                }
                // Command high byte (pci2isa.cc:345-347) — i440FX
                0x05 => {
                    self.pci_conf[addr] = value8 & 0x01;
                }
                // Status lo — read-only (pci2isa.cc:349-350)
                0x06 => {}
                // Status hi — write-1-to-clear (pci2isa.cc:351-358) — i440FX
                0x07 => {
                    let clear_bits = value8 & 0x78;
                    self.pci_conf[addr] = (oldval & !clear_bits) | 0x02;
                }
                // XBCS register (pci2isa.cc:359-371) — BIOS write enable
                0x4E => {
                    if (value8 & 0x04) != (oldval & 0x04) {
                        tracing::debug!("BIOS write support set to {}", (value8 & 0x04) != 0);
                    }
                    self.pci_conf[addr] = value8;
                }
                // APIC enable / BIOS extended access (pci2isa.cc:372-386)
                0x4F => {
                    self.pci_conf[addr] = value8 & 0x01;
                    // bit 0: I/O APIC enable
                    // In Bochs, this calls DEV_ioapic_set_enabled()
                    tracing::debug!("PIIX3: APIC enable = {}", value8 & 0x01);
                }
                // PIRQ routing registers (pci2isa.cc:387-397)
                0x60..=0x63 => {
                    let v = value8 & 0x8F; // bits 4-6 reserved
                    if v != oldval {
                        self.pci_conf[addr] = v;
                        tracing::info!(
                            "PCI IRQ routing: PIRQ{}# set to {:#04x}",
                            (b'A' + (addr as u8 - 0x60)) as char,
                            v
                        );
                    }
                }
                // USB function enable (pci2isa.cc:398-403)
                0x6A => {
                    self.pci_conf[addr] = value8 & 0xD7;
                }
                // APIC base address (pci2isa.cc:404-413)
                0x80 => {
                    self.pci_conf[addr] = value8 & 0x7F;
                }
                // Default
                _ => {
                    self.pci_conf[addr] = value8;
                }
            }
        }
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

    // ─── PCI IRQ Routing ─────────────────────────────────────────────────

    /// Route a PCI interrupt to an ISA IRQ.
    /// Bochs: bx_piix3_c::pci_set_irq() (pci2isa.cc:185-229)
    ///
    /// Returns Some(irq, level) if the PIC IRQ line should change, None otherwise.
    pub fn pci_set_irq(&mut self, devfunc: u8, line: u8, level: bool) -> Option<(u8, bool)> {
        let device = devfunc >> 3;

        // Compute PIRQ index from device slot and interrupt line
        // Bochs pci2isa.cc:196-203 (i440FX path)
        let pirq = if device == 1 {
            (line - 1) & 3
        } else if device < 7 {
            ((device - 1) + line - 1) & 3 // slot + line - 1 (slot = device - 1 for simple mapping)
        } else {
            ((device - 1) + line - 2) & 3
        };

        let irq = self.pci_conf[0x60 + pirq as usize];

        // Check if IRQ is valid and routable
        if irq < 16 && ((1u16 << irq) & VALID_PCI_IRQ_MASK) != 0 {
            if level {
                // Check if no other device was asserting this IRQ through any PIRQ
                let was_asserted = self.irq_level[0][irq as usize] != 0
                    || self.irq_level[1][irq as usize] != 0
                    || self.irq_level[2][irq as usize] != 0
                    || self.irq_level[3][irq as usize] != 0;

                self.irq_level[pirq as usize][irq as usize] |= 1 << device;

                if !was_asserted {
                    tracing::debug!(
                        "INT{} -> PIRQ{} -> IRQ {} = 1",
                        (line + 64) as char, // 'A', 'B', etc.
                        (pirq + 65) as char,
                        irq
                    );
                    return Some((irq, true));
                }
            } else {
                self.irq_level[pirq as usize][irq as usize] &= !(1 << device);

                // Only deassert if no other device is asserting through any PIRQ
                let still_asserted = self.irq_level[0][irq as usize] != 0
                    || self.irq_level[1][irq as usize] != 0
                    || self.irq_level[2][irq as usize] != 0
                    || self.irq_level[3][irq as usize] != 0;

                if !still_asserted {
                    tracing::debug!(
                        "INT{} -> PIRQ{} -> IRQ {} = 0",
                        (line + 64) as char,
                        (pirq + 65) as char,
                        irq
                    );
                    return Some((irq, false));
                }
            }
        }

        None
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_piix3_new() {
        let bridge = BxPiix3::new();
        assert_eq!(bridge.devfunc, 0x08);
        // Vendor: Intel
        assert_eq!(bridge.pci_conf[0x00], 0x86);
        assert_eq!(bridge.pci_conf[0x01], 0x80);
        // Device: PIIX3
        assert_eq!(bridge.pci_conf[0x02], 0x00);
        assert_eq!(bridge.pci_conf[0x03], 0x70);
        // Class: ISA bridge
        assert_eq!(bridge.pci_conf[0x0B], 0x06);
        assert_eq!(bridge.pci_conf[0x0A], 0x01);
        // Header: multi-function
        assert_eq!(bridge.pci_conf[0x0E], 0x80);
        // PIRQ disabled
        for i in 0..4 {
            assert_eq!(bridge.pci_conf[0x60 + i], 0x80);
        }
    }

    #[test]
    fn test_piix3_reset() {
        let mut bridge = BxPiix3::new();
        bridge.elcr1 = 0xFF;
        bridge.apmc = 0xFF;
        bridge.reset();
        assert_eq!(bridge.elcr1, 0x00);
        assert_eq!(bridge.apmc, 0x00);
        assert_eq!(bridge.pci_conf[0x07], 0x02);
    }

    #[test]
    fn test_elcr_write() {
        let mut bridge = BxPiix3::new();
        // ELCR1: bits 0-2 always edge (masked to 0xF8)
        bridge.write(0x04D0, 0xFF, 1);
        assert_eq!(bridge.elcr1, 0xF8);
        assert!(bridge.elcr1_changed);
        // ELCR2: bits 0 and 5 always edge (masked to 0xDE)
        bridge.write(0x04D1, 0xFF, 1);
        assert_eq!(bridge.elcr2, 0xDE);
        assert!(bridge.elcr2_changed);
    }

    #[test]
    fn test_apm_ports() {
        let mut bridge = BxPiix3::new();
        bridge.write(0x00B2, 0x42, 1);
        assert_eq!(bridge.apmc, 0x42);
        assert_eq!(bridge.read(0x00B2), 0x42);
        bridge.write(0x00B3, 0x55, 1);
        assert_eq!(bridge.apms, 0x55);
        assert_eq!(bridge.read(0x00B3), 0x55);
    }

    #[test]
    fn test_pirq_routing() {
        let mut bridge = BxPiix3::new();
        // Device 2 (slot 1), line INTA:
        // pirq = ((device-1) + line-1) & 3 = ((2-1)+1-1) & 3 = 1 -> PIRQB (0x61)
        // Set PIRQB to route to IRQ 10
        bridge.pci_write(0x61, 0x0A, 1);
        assert_eq!(bridge.pci_conf[0x61], 0x0A);

        let result = bridge.pci_set_irq(0x10, 1, true); // devfunc=0x10 -> device=2
        assert!(result.is_some());
        let (irq, level) = result.unwrap();
        assert_eq!(irq, 10);
        assert!(level);
    }

    #[test]
    fn test_cpu_reset_register() {
        let mut bridge = BxPiix3::new();
        // Write reset type (bit 1) then trigger (bit 2)
        bridge.write(0x0CF9, 0x02, 1); // Set reset type = hardware
        assert_eq!(bridge.pci_reset, 0x02);
        assert!(bridge.reset_request.is_none());
        bridge.write(0x0CF9, 0x06, 1); // Set type + trigger
        assert!(bridge.reset_request.is_some());
        assert_eq!(bridge.reset_request, Some(true)); // hardware reset
    }
}
