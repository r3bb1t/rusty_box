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

#[cfg(feature = "std")]
use std::io::{self, Error, ErrorKind, Read, Write};

#[cfg(feature = "std")]
use crate::snapshot::{
    bounds, checked_snapshot_len_add, checked_snapshot_len_mul, SnapshotReader, SnapshotWriteExt,
};

#[cfg(feature = "std")]
const PIIX3_SNAPSHOT_IDENTITY_BYTES: [usize; 9] = [0, 1, 2, 3, 8, 9, 10, 11, 0x0e];

#[cfg(feature = "std")]
fn invalid_piix3_snapshot(message: &'static str) -> io::Error {
    Error::new(ErrorKind::InvalidData, message)
}

#[cfg(feature = "std")]
fn validate_piix3_snapshot_identity(
    saved: &[u8; PCI_CONF_SIZE],
    live: &[u8; PCI_CONF_SIZE],
) -> io::Result<()> {
    for index in PIIX3_SNAPSHOT_IDENTITY_BYTES {
        if saved[index] != live[index] {
            return Err(invalid_piix3_snapshot(
                "snapshot PIIX3 PCI identity does not match live configuration",
            ));
        }
    }
    Ok(())
}
use crate::cpu::ResetReason;
/// PCI configuration space size
const PCI_CONF_SIZE: usize = 256;

/// APM command port (Bochs pci2isa.cc)
pub const APM_CMD_PORT: u16 = 0x00B2;
/// APM status port (Bochs pci2isa.cc)
pub const APM_STS_PORT: u16 = 0x00B3;
/// ELCR1 — Edge/Level Control Register for master PIC (Bochs pci2isa.cc)
pub const ELCR1_PORT: u16 = 0x04D0;
/// ELCR2 — Edge/Level Control Register for slave PIC (Bochs pci2isa.cc)
pub const ELCR2_PORT: u16 = 0x04D1;
/// CPU reset register (Bochs pci2isa.cc)
pub const PCI_RESET_PORT: u16 = 0x0CF9;

/// Valid IRQ mask for PCI routing (Bochs pci2isa.cc)
/// Bits set for IRQs that can be used: 3,4,5,6,7,9,10,11,12,14,15
const VALID_PCI_IRQ_MASK: u16 = 0xDEF8;

/// Deferred memory-subsystem update a PIIX3 config-space write requires.
/// Bochs applies the BIOS-write-enable state to the memory object
/// synchronously inside `pci_write_handler` (pci2isa.cc case 0x4e); here the
/// memory system lives outside the bridge (borrow-separated), so
/// `devices.rs` defers via `bios_write_needs_update` and drains it at the
/// shared machine boundary once memory is available, matching
/// `PciBridgeWriteEffects` in `pci.rs`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Piix3WriteEffects {
    /// The XBCS register (0x4E) changed a bit that affects BIOS-ROM
    /// write-enable state; must be re-applied to memory.
    pub bios_write_changed: bool,
}

/// PIIX3 PCI-to-ISA bridge state.
/// Bochs: bx_piix3_c (pci2isa.h)
#[derive(Debug)]
pub struct BxPiix3 {
    /// PCI device/function number (PIIX3: bus 0, dev 1, func 0 = 0x08)
    pub devfunc: u8,

    /// PCI configuration space (256 bytes)
    pub pci_conf: [u8; PCI_CONF_SIZE],

    /// Edge/Level Control Register 1 (master PIC IRQs 0-7)
    /// Bochs: s.elcr1 (pci2isa.h)
    pub elcr1: u8,
    /// Edge/Level Control Register 2 (slave PIC IRQs 8-15)
    /// Bochs: s.elcr2 (pci2isa.h)
    pub elcr2: u8,

    /// APM command register (Bochs: s.apmc, pci2isa.h)
    pub apmc: u8,
    /// APM status register (Bochs: s.apms, pci2isa.h)
    pub apms: u8,

    /// PCI IRQ level tracking: [pirq_line][irq_number]
    /// Bochs: s.irq_level[4][16] (pci2isa.h)
    /// Each entry is a bitmask of which devices are asserting that IRQ through that PIRQ
    pub irq_level: [[u32; 16]; 4],

    /// CPU reset register (Bochs: s.pci_reset, pci2isa.h)
    pub pci_reset: u8,

    /// Flag: ELCR1 changed — emulator should call pic.set_mode()
    pub elcr1_changed: bool,
    /// Flag: ELCR2 changed — emulator should call pic.set_mode()
    pub elcr2_changed: bool,
    /// Flag: reset requested — emulator should handle CPU reset
    pub reset_request: Option<ResetReason>,
}

impl Default for BxPiix3 {
    fn default() -> Self {
        Self::new()
    }
}

impl BxPiix3 {
    /// Create a new PIIX3 bridge.
    /// Bochs: bx_piix3_c::init() (pci2isa.cc)
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
    /// Bochs: init_pci_conf(0x8086, 0x7000, 0x00, 0x060100, 0x80, 0) (pci2isa.cc)
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
    /// Bochs: bx_piix3_c::reset() (pci2isa.cc)
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

        // Reset PIRQ routing to disabled (pci2isa.cc)
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
    /// Bochs: bx_piix3_c::read() (pci2isa.cc)
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
    /// Bochs: bx_piix3_c::write() (pci2isa.cc)
    pub fn write(&mut self, address: u16, value: u32, io_len: u8) {
        match address {
            // APM command port (pci2isa.cc)
            0x00B2 => {
                // Note: In Bochs this forwards to ACPI generate_smi()
                // In our architecture, the ACPI device also listens on 0xB2
                self.apmc = value as u8;
                if io_len == 2 {
                    self.apms = (value >> 8) as u8;
                }
            }
            // APM status port (pci2isa.cc)
            0x00B3 => {
                self.apms = value as u8;
            }
            // ELCR1 — master PIC edge/level (pci2isa.cc)
            0x04D0 => {
                let v = (value as u8) & 0xF8; // bits 0-2 always edge
                if v != self.elcr1 {
                    self.elcr1 = v;
                    self.elcr1_changed = true;
                    tracing::debug!("ELCR1 = {:#04x}", self.elcr1);
                }
            }
            // ELCR2 — slave PIC edge/level (pci2isa.cc)
            0x04D1 => {
                let v = (value as u8) & 0xDE; // bits 0 and 5 always edge
                if v != self.elcr2 {
                    self.elcr2 = v;
                    self.elcr2_changed = true;
                    tracing::debug!("ELCR2 = {:#04x}", self.elcr2);
                }
            }
            // CPU reset register (pci2isa.cc)
            0x0CF9 => {
                tracing::debug!("CPU reset register write: {:#04x}", value);
                self.pci_reset = (value as u8) & 0x02;
                if (value as u8) & 0x04 != 0 {
                    if self.pci_reset != 0 {
                        self.reset_request = Some(ResetReason::Hardware);
                    } else {
                        self.reset_request = Some(ResetReason::Software);
                    }
                }
            }
            _ => {}
        }
    }

    // ─── PCI Configuration Space ─────────────────────────────────────────

    /// Write to PCI configuration space.
    /// Bochs: bx_piix3_c::pci_write_handler() (pci2isa.cc)
    /// Returns which deferred memory-subsystem updates the caller must apply
    /// (the XBCS register changed a bit affecting BIOS-ROM write-enable).
    pub fn pci_write(&mut self, address: u8, value: u32, io_len: u8) -> Piix3WriteEffects {
        let mut effects = Piix3WriteEffects::default();
        // BARs are read-only
        if (0x10..0x34).contains(&address) {
            return effects;
        }

        for i in 0..io_len as usize {
            let addr = address as usize + i;
            if addr >= PCI_CONF_SIZE {
                break;
            }
            let value8 = ((value >> (i * 8)) & 0xFF) as u8;
            let oldval = self.pci_conf[addr];

            match addr {
                // Command register (pci2isa.cc)
                0x04 => {
                    self.pci_conf[addr] = (value8 & 0x08) | 0x07;
                }
                // Command high byte (pci2isa.cc) — i440FX
                0x05 => {
                    self.pci_conf[addr] = value8 & 0x01;
                }
                // Status lo — read-only (pci2isa.cc)
                0x06 => {}
                // Status hi — write-1-to-clear (pci2isa.cc) — i440FX
                0x07 => {
                    let clear_bits = value8 & 0x78;
                    self.pci_conf[addr] = (oldval & !clear_bits) | 0x02;
                }
                // XBCS register (pci2isa.cc) — BIOS write enable / rom access
                0x4E => {
                    if (value8 & 0x04) != (oldval & 0x04) {
                        tracing::trace!("BIOS write support set to {}", (value8 & 0x04) != 0);
                        effects.bios_write_changed = true;
                    }
                    if (value8 & 0xC0) != (oldval & 0xC0) {
                        tracing::trace!(
                            "BIOS enable switches: lower={} extended={}",
                            (value8 >> 6) & 1,
                            (value8 >> 7) & 1
                        );
                        effects.bios_write_changed = true;
                    }
                    self.pci_conf[addr] = value8;
                }
                // APIC enable / BIOS extended access (pci2isa.cc)
                0x4F => {
                    self.pci_conf[addr] = value8 & 0x01;
                    // bit 0: I/O APIC enable
                    // In Bochs, this calls DEV_ioapic_set_enabled()
                    tracing::trace!("PIIX3: APIC enable = {}", value8 & 0x01);
                }
                // PIRQ routing registers (pci2isa.cc)
                0x60..=0x63 => {
                    let v = value8 & 0x8F; // bits 4-6 reserved
                    if v != oldval {
                        self.pci_conf[addr] = v;
                        tracing::debug!(
                            "PCI IRQ routing: PIRQ{}# set to {:#04x}",
                            (b'A' + (addr as u8 - 0x60)) as char,
                            v
                        );
                    }
                }
                // USB function enable (pci2isa.cc)
                0x6A => {
                    self.pci_conf[addr] = value8 & 0xD7;
                }
                // APIC base address (pci2isa.cc)
                0x80 => {
                    self.pci_conf[addr] = value8 & 0x7F;
                }
                // Default
                _ => {
                    self.pci_conf[addr] = value8;
                }
            }
        }
        effects
    }

    /// Apply the XBCS register (0x4E) BIOS-write-enable bits to the memory
    /// subsystem. Bochs: bx_piix3_c::pci_write_handler() (pci2isa.cc) case
    /// 0x4e's `DEV_mem_set_bios_write`/`DEV_mem_set_bios_rom_access` calls.
    /// Idempotent: derives state from the already-committed
    /// `pci_conf[0x4E]`, so it is safe to call any number of times from the
    /// shared machine-boundary drain.
    pub fn apply_bios_write_to_memory<'c>(&self, mem: &mut crate::memory::BxMemC<'c>) {
        let v = self.pci_conf[0x4E];
        mem.set_bios_write_enabled((v & 0x04) != 0);
        mem.set_bios_rom_access(crate::memory::BIOS_ROM_LOWER, (v & 0x40) != 0);
        mem.set_bios_rom_access(crate::memory::BIOS_ROM_EXTENDED, (v & 0x80) != 0);
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
    /// Bochs: bx_piix3_c::pci_set_irq() (pci2isa.cc)
    ///
    /// Returns Some(irq, level) if the PIC IRQ line should change, None otherwise.
    pub fn pci_set_irq(&mut self, devfunc: u8, line: u8, level: bool) -> Option<(u8, bool)> {
        let device = devfunc >> 3;

        // Compute PIRQ index from device slot and interrupt line
        // Bochs pci2isa.cc (i440FX path)
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
                    tracing::trace!(
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
                    tracing::trace!(
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

    /// Exact byte count for this ISA bridge's contribution to the combined
    /// PCI payload. The enclosing PCI codec owns the section-version prefix.
    #[cfg(feature = "std")]
    pub(crate) fn snapshot_v3_body_len(&self) -> io::Result<u64> {
        let config_len = u64::try_from(PCI_CONF_SIZE)
            .map_err(|_| invalid_piix3_snapshot("PIIX3 config size does not fit u64"))?;
        let irq_cells = checked_snapshot_len_mul(4, 16)?;
        let irq_bytes = checked_snapshot_len_mul(irq_cells, 4)?;
        let mut len = checked_snapshot_len_add(1, config_len)?;
        len = checked_snapshot_len_add(len, 4)?;
        len = checked_snapshot_len_add(len, irq_bytes)?;
        len = checked_snapshot_len_add(len, 4)?;
        if len > bounds::MAX_SNAPSHOT_SECTION_LEN {
            return Err(invalid_piix3_snapshot(
                "PIIX3 snapshot body exceeds section bound",
            ));
        }
        Ok(len)
    }

    /// Stream the mutable PIIX3 configuration, PIC-routing, and reset state.
    #[cfg(feature = "std")]
    pub(crate) fn save_snapshot_v3_body<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(self.devfunc)?;
        writer.write_bytes(&self.pci_conf)?;
        writer.write_u8(self.elcr1)?;
        writer.write_u8(self.elcr2)?;
        writer.write_u8(self.apmc)?;
        writer.write_u8(self.apms)?;
        for pirq_levels in &self.irq_level {
            for &level in pirq_levels {
                writer.write_u32(level)?;
            }
        }
        writer.write_u8(self.pci_reset)?;
        writer.write_bool(self.elcr1_changed)?;
        writer.write_bool(self.elcr2_changed)?;
        writer.write_u8(match self.reset_request {
            None => 0,
            Some(ResetReason::Software) => 1,
            Some(ResetReason::Hardware) => 2,
        })
    }

    /// Restore PIIX3 state without applying ELCR modes, reset requests, or
    /// PCI interrupt edges. Those effects are restored once every component
    /// of the enclosing PCI section has validated.
    #[cfg(feature = "std")]
    pub(crate) fn restore_snapshot_v3_body<R: Read>(
        &mut self,
        reader: &mut SnapshotReader<R>,
    ) -> io::Result<()> {
        let devfunc = reader.read_u8()?;
        let mut pci_conf = [0u8; PCI_CONF_SIZE];
        reader.read_bytes(&mut pci_conf)?;
        let elcr1 = reader.read_u8()?;
        let elcr2 = reader.read_u8()?;
        let apmc = reader.read_u8()?;
        let apms = reader.read_u8()?;
        let mut irq_level = [[0u32; 16]; 4];
        for pirq_levels in &mut irq_level {
            for level in pirq_levels {
                *level = reader.read_u32()?;
            }
        }
        let pci_reset = reader.read_u8()?;
        let elcr1_changed = reader.read_bool()?;
        let elcr2_changed = reader.read_bool()?;
        let reset_request = match reader.read_u8()? {
            0 => None,
            1 => Some(ResetReason::Software),
            2 => Some(ResetReason::Hardware),
            _ => return Err(invalid_piix3_snapshot("snapshot PIIX3 reset reason is invalid")),
        };

        let expected_devfunc = super::pci::pci_device(1, 0);
        if self.devfunc != expected_devfunc || devfunc != self.devfunc {
            return Err(invalid_piix3_snapshot(
                "snapshot PIIX3 device/function does not match live topology",
            ));
        }
        validate_piix3_snapshot_identity(&pci_conf, &self.pci_conf)?;
        for &pirq_route in &pci_conf[0x60..0x64] {
            if (pirq_route & 0x70) != 0 {
                return Err(invalid_piix3_snapshot(
                    "snapshot PIIX3 PIRQ route uses reserved bits",
                ));
            }
            if (pirq_route & 0x80) == 0 {
                let irq = pirq_route & 0x0f;
                if (VALID_PCI_IRQ_MASK & (1u16 << u32::from(irq))) == 0 {
                    return Err(invalid_piix3_snapshot(
                        "snapshot PIIX3 PIRQ route selects an invalid IRQ",
                    ));
                }
            }
        }
        if (elcr1 & 0x07) != 0 || (elcr2 & !0xDE) != 0 {
            return Err(invalid_piix3_snapshot(
                "snapshot PIIX3 ELCR contains fixed edge-triggered bits",
            ));
        }
        if (pci_reset & !0x02) != 0 {
            return Err(invalid_piix3_snapshot(
                "snapshot PIIX3 reset register contains reserved bits",
            ));
        }

        self.devfunc = devfunc;
        self.pci_conf = pci_conf;
        self.elcr1 = elcr1;
        self.elcr2 = elcr2;
        self.apmc = apmc;
        self.apms = apms;
        self.irq_level = irq_level;
        self.pci_reset = pci_reset;
        self.elcr1_changed = elcr1_changed;
        self.elcr2_changed = elcr2_changed;
        self.reset_request = reset_request;
        Ok(())
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
        assert_eq!(bridge.reset_request, Some(ResetReason::Hardware));
    }

    // ─── Finding #35b: XBCS (0x4E) BIOS write-enable wiring ──────────────────
    //
    // Bochs pci2isa.cc bx_piix3_c::pci_write_handler case 0x4e:
    //   if ((value8 & 0x04) != (oldval & 0x04)) DEV_mem_set_bios_write(...)
    //   if ((value8 & 0xc0) != (oldval & 0xc0)) DEV_mem_set_bios_rom_access(...) x2
    //   pci_conf[address+i] = value8;   (always stored)

    #[test]
    fn test_xbcs_write_signals_effects_only_on_relevant_bit_change() {
        let mut bridge = BxPiix3::new();
        bridge.reset();
        // pci_conf[0x4e] == 0x03 after reset (bit2=0, bits6-7=0).

        // Writing the same "no relevant bits changed" pattern must not signal.
        let effects = bridge.pci_write(0x4E, 0x03, 1);
        assert!(!effects.bios_write_changed);

        // Setting bit2 (BIOS write enable) must signal.
        let effects = bridge.pci_write(0x4E, 0x07, 1); // 0x03 | 0x04
        assert!(effects.bios_write_changed);
        assert_eq!(bridge.pci_conf[0x4E], 0x07, "byte is always stored");

        // Re-writing the identical value must not signal again.
        let effects = bridge.pci_write(0x4E, 0x07, 1);
        assert!(!effects.bios_write_changed);

        // Changing only the rom-access bits (6-7) must also signal.
        let effects = bridge.pci_write(0x4E, 0xC7, 1); // 0x07 | 0xC0
        assert!(effects.bios_write_changed);
    }

    #[test]
    fn test_apply_bios_write_to_memory_drives_mem_from_xbcs() {
        use crate::memory::{BxMemC, BxMemoryStubC};

        let mut bridge = BxPiix3::new();
        bridge.reset();
        let stub = BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
        let mut mem = BxMemC::new(stub, false);

        // Bit2 set -> BIOS write enabled; bits 6-7 clear -> both rom-access
        // region bits disabled.
        bridge.pci_write(0x4E, 0x04, 1);
        bridge.apply_bios_write_to_memory(&mut mem);
        assert!(mem.bios_write_enabled());
        assert_eq!(mem.bios_rom_access(), 0x00);

        // Bit2 clear, bits 6+7 set -> BIOS write disabled; both rom-access
        // region bits enabled.
        bridge.pci_write(0x4E, 0xC0, 1);
        bridge.apply_bios_write_to_memory(&mut mem);
        assert!(!mem.bios_write_enabled());
        assert_eq!(mem.bios_rom_access(), 0x03); // BIOS_ROM_LOWER | BIOS_ROM_EXTENDED
    }

    #[test]
    fn test_piix3_reset_does_not_touch_bios_write_enable() {
        // Bochs bx_piix3_c::reset() sets pci_conf[0x4e] = 0x03 directly,
        // WITHOUT going through pci_write_handler -- so it never calls
        // DEV_mem_set_bios_write(). The memory-side bios_write_enabled state
        // is deliberately left untouched by a guest reset in upstream Bochs;
        // rusty_box's reset() must not synthesize a deferred update either.
        let mut bridge = BxPiix3::new();
        bridge.pci_write(0x4E, 0x07, 1); // enable BIOS write
        bridge.reset();
        assert_eq!(bridge.pci_conf[0x4E], 0x03, "register value resets");
    }
}
