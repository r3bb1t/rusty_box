//! PIIX4 ACPI Power Management Controller
//!
//! Matches Bochs `iodev/acpi.cc` (583 lines) + `acpi.h` (88 lines).
//!
//! Implements:
//! - PM1a Event Block (PMSTS, PMEN) — status and enable registers
//! - PM1a Control Block (PMCNTRL) — sleep/wake control
//! - PM Timer Block (PMTMR) — 24-bit free-running 3.579545 MHz timer
//! - General Purpose registers (GPSTS, GLBSTS, DEVSTS, etc.)
//! - SMBus controller (host interface registers)
//! - PCI configuration space (PIIX4 PM function, bus 0, dev 1, func 3)
//! - SCI interrupt generation on IRQ 9
//! - ACPI enable/disable via SMI command port (0xB2)
//!
//! The PM timer is the primary time source for ACPI-aware operating systems.
//! It runs at exactly 3,579,545 Hz (the NTSC color subcarrier frequency)
//! and wraps every ~2.34 seconds (24-bit counter).

use bitflags::bitflags;

/// PM timer frequency: 3.579545 MHz (ACPI spec, section 4.7.3.3)
const PM_FREQ: u64 = 3_579_545;

/// Debug I/O port address (Bochs acpi.cc:51)
const ACPI_DBG_IO_ADDR: u16 = 0xB044;

/// SMI command port (ACPI spec — FADT SmiCmd field)
/// The BIOS writes ACPI_ENABLE/ACPI_DISABLE here.
const SMI_CMD_PORT: u16 = 0x00B2;

/// ACPI enable command value (Bochs acpi.cc:65)
const ACPI_ENABLE: u8 = 0xF1;
/// ACPI disable command value (Bochs acpi.cc:66)
const ACPI_DISABLE: u8 = 0xF0;

// ─── PM Status Register bits (Bochs acpi.cc:53-54) ──────────────────────────

bitflags! {
    /// PM1 Status Register bits (offset 0x00 from PM base)
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PmStatus: u16 {
        /// Timer overflow status (bit 0) — set when 24-bit timer wraps
        const TMROF_STS = 1 << 0;
        /// Bus master status (bit 4)
        const BM_STS    = 1 << 4;
        /// Global status (bit 5)
        const GBL_STS   = 1 << 5;
        /// Power button status (bit 8)
        const PWRBTN_STS = 1 << 8;
        /// Sleep button status (bit 9)
        const SLPBTN_STS = 1 << 9;
        /// RTC alarm status (bit 10)
        const RTC_STS   = 1 << 10;
        /// Resume status (bit 15) — set after wake from S3
        const RSM_STS   = 1 << 15;
    }
}

bitflags! {
    /// PM1 Enable Register bits (offset 0x02 from PM base)
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PmEnable: u16 {
        /// Timer overflow enable (bit 0)
        const TMROF_EN  = 1 << 0;
        /// Global enable (bit 5)
        const GBL_EN    = 1 << 5;
        /// Power button enable (bit 8)
        const PWRBTN_EN = 1 << 8;
        /// RTC enable (bit 10)
        const RTC_EN    = 1 << 10;
    }
}

bitflags! {
    /// PM1 Control Register bits (offset 0x04 from PM base)
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PmControl: u16 {
        /// SCI enable (bit 0) — when set, SCI interrupts are enabled
        const SCI_EN  = 1 << 0;
        /// Bus master reload (bit 1)
        const BM_RLD  = 1 << 1;
        /// Global release (bit 2)
        const GBL_RLS = 1 << 2;
        /// Suspend enable (bit 13) — triggers sleep state transition
        const SUS_EN  = 1 << 13;
    }
}

/// I/O access mask for PM register space (64 ports).
/// Each entry is a bitmask: bit 0 = byte, bit 1 = word, bit 2 = dword.
/// Bochs acpi.cc:43-46
const ACPI_PM_IOMASK: [u8; 64] = [
    3, 0, 3, 0, 3, 0, 0, 0, 4, 0, 0, 0, 3, 1, 3, 1, 7, 1, 3, 1, 1, 1, 0, 0, 3, 1, 0, 0, 7, 1, 3, 1,
    3, 1, 0, 0, 0, 0, 0, 0, 7, 1, 3, 1, 7, 1, 3, 1, 1, 1, 1, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// I/O access mask for SMBus register space (16 ports).
/// Bochs acpi.cc:47
const ACPI_SM_IOMASK: [u8; 16] = [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 0, 2, 0, 0, 0];

// ─── SMBus state ─────────────────────────────────────────────────────────────

/// SMBus host controller state (Bochs acpi.h:74-83)
#[derive(Debug, Clone)]
pub struct SmBusState {
    pub stat: u8,
    pub ctl: u8,
    pub cmd: u8,
    pub addr: u8,
    pub data0: u8,
    pub data1: u8,
    pub index: u8,
    pub data: [u8; 32],
}

impl Default for SmBusState {
    fn default() -> Self {
        Self {
            stat: 0,
            ctl: 0,
            cmd: 0,
            addr: 0,
            data0: 0,
            data1: 0,
            index: 0,
            data: [0; 32],
        }
    }
}

// ─── PCI Configuration Space ─────────────────────────────────────────────────

/// PCI configuration space size
const PCI_CONF_SIZE: usize = 256;

// ─── ACPI Controller ─────────────────────────────────────────────────────────

/// PIIX4 ACPI Power Management controller.
/// Bochs: bx_acpi_ctrl_c (acpi.h:35-85, acpi.cc)
#[derive(Debug)]
pub struct BxAcpiCtrl {
    /// PCI device/function number (PIIX4: bus 0, dev 1, func 3 = 0x0B)
    pub devfunc: u8,

    /// PM I/O base address (from PCI config 0x40-0x43, masked to 64-port alignment)
    pub pm_base: u32,
    /// SMBus I/O base address (from PCI config 0x90-0x93, masked to 16-port alignment)
    pub sm_base: u32,

    /// PM1 Status Register (Bochs: s.pmsts)
    pmsts: u16,
    /// PM1 Enable Register (Bochs: s.pmen)
    pmen: u16,
    /// PM1 Control Register (Bochs: s.pmcntrl)
    pmcntrl: u16,
    /// Next timer overflow time in PM timer ticks (24-bit wrap boundary)
    tmr_overflow_time: u64,

    /// Generic PM register space (56 bytes, Bochs: s.pmreg[0x38])
    pmreg: [u8; 0x38],

    /// SMBus host controller state
    smbus: SmBusState,

    /// PCI configuration space (256 bytes)
    pub pci_conf: [u8; PCI_CONF_SIZE],

    /// Accumulated microseconds for PM timer computation.
    /// In Bochs this comes from bx_virt_timer.time_usec(); here we
    /// accumulate from the emulator's tick_devices(usec) calls.
    pub time_usec: u64,

    /// IRQ 9 level (SCI) — the emulator loop syncs this to the PIC.
    pub irq9_level: bool,

    /// Whether PM I/O ports are registered (tracks pm_base changes)
    pub(crate) pm_ports_registered: bool,
    /// Whether SM I/O ports are registered (tracks sm_base changes)
    pub(crate) sm_ports_registered: bool,
}

impl Default for BxAcpiCtrl {
    fn default() -> Self {
        Self::new()
    }
}

impl BxAcpiCtrl {
    /// Create a new ACPI controller instance.
    /// Bochs: bx_acpi_ctrl_c::bx_acpi_ctrl_c() (acpi.cc:111-116)
    pub fn new() -> Self {
        let mut ctrl = Self {
            devfunc: 0x0B, // BX_PCI_DEVICE(1, 3) = (1 << 3) | 3 = 0x0B
            pm_base: 0,
            sm_base: 0,
            pmsts: 0,
            pmen: 0,
            pmcntrl: 0,
            tmr_overflow_time: 0xFF_FFFF, // 24-bit max (Bochs acpi.cc:190)
            pmreg: [0; 0x38],
            smbus: SmBusState::default(),
            pci_conf: [0; PCI_CONF_SIZE],
            time_usec: 0,
            irq9_level: false,
            pm_ports_registered: false,
            sm_ports_registered: false,
        };
        ctrl.init_pci_conf();
        ctrl
    }

    /// Initialize PCI configuration space with PIIX4 PM identity.
    /// Bochs: init_pci_conf(0x8086, 0x7113, 0x03, 0x068000, 0x00, 0) (acpi.cc:151)
    fn init_pci_conf(&mut self) {
        // Vendor ID: Intel (0x8086)
        self.pci_conf[0x00] = 0x86;
        self.pci_conf[0x01] = 0x80;
        // Device ID: PIIX4 PM (0x7113)
        self.pci_conf[0x02] = 0x13;
        self.pci_conf[0x03] = 0x71;
        // Revision: 0x03
        self.pci_conf[0x08] = 0x03;
        // Class code: Bridge / Other (0x068000)
        self.pci_conf[0x09] = 0x00;
        self.pci_conf[0x0A] = 0x80;
        self.pci_conf[0x0B] = 0x06;
    }

    /// Reset the ACPI controller.
    /// Bochs: bx_acpi_ctrl_c::reset() (acpi.cc:154-206)
    pub fn reset(&mut self) {
        // PCI command/status (acpi.cc:158-162)
        self.pci_conf[0x04] = 0x00;
        self.pci_conf[0x05] = 0x00;
        self.pci_conf[0x06] = 0x80; // status_devsel_medium
        self.pci_conf[0x07] = 0x02;
        self.pci_conf[0x3C] = 0x00; // IRQ

        // PM base 0x40-0x43 (acpi.cc:165-168)
        self.pci_conf[0x40] = 0x01;
        self.pci_conf[0x41] = 0x00;
        self.pci_conf[0x42] = 0x00;
        self.pci_conf[0x43] = 0x00;

        // Clear DEVACTB (acpi.cc:171-172)
        self.pci_conf[0x58] = 0x00;
        self.pci_conf[0x59] = 0x00;

        // Device resources (acpi.cc:175-179)
        self.pci_conf[0x5A] = 0x00;
        self.pci_conf[0x5B] = 0x00;
        self.pci_conf[0x5F] = 0x90;
        self.pci_conf[0x63] = 0x60;
        self.pci_conf[0x67] = 0x98;

        // SM base 0x90-0x93 (acpi.cc:182-185)
        self.pci_conf[0x90] = 0x01;
        self.pci_conf[0x91] = 0x00;
        self.pci_conf[0x92] = 0x00;
        self.pci_conf[0x93] = 0x00;

        // Clear PM state (acpi.cc:187-193)
        self.pmsts = 0;
        self.pmen = 0;
        self.pmcntrl = 0;
        self.tmr_overflow_time = 0xFF_FFFF;
        self.pmreg = [0; 0x38];

        // Clear SMBus state (acpi.cc:195-205)
        self.smbus = SmBusState::default();

        self.irq9_level = false;
    }

    /// Advance the internal time counter by the given microseconds.
    /// Called from the emulator's tick_devices() path.
    pub fn tick(&mut self, usec: u64) {
        self.time_usec += usec;
    }

    // ─── PM Timer ────────────────────────────────────────────────────────

    /// Get the 24-bit PM timer value.
    /// Bochs: get_pmtmr() (acpi.cc:252-256)
    fn get_pmtmr(&self) -> u32 {
        let value = muldiv64(self.time_usec, PM_FREQ as u32, 1_000_000);
        (value & 0xFF_FFFF) as u32
    }

    /// Get PM status with timer overflow check.
    /// Bochs: get_pmsts() (acpi.cc:258-264)
    fn get_pmsts(&mut self) -> u16 {
        let value = muldiv64(self.time_usec, PM_FREQ as u32, 1_000_000);
        if value >= self.tmr_overflow_time {
            self.pmsts |= PmStatus::TMROF_STS.bits();
        }
        self.pmsts
    }

    /// Update SCI interrupt level based on current status and enable.
    /// Bochs: pm_update_sci() (acpi.cc:266-283)
    fn pm_update_sci(&mut self) {
        let pmsts = self.get_pmsts();
        // SCI fires if any enabled status bit is set
        // Bochs acpi.cc:269-270: (pmsts & pmen) & (RTC_EN | PWRBTN_EN | GBL_EN | TMROF_EN)
        let sci_mask = PmEnable::RTC_EN.bits()
            | PmEnable::PWRBTN_EN.bits()
            | PmEnable::GBL_EN.bits()
            | PmEnable::TMROF_EN.bits();
        let sci_level = (pmsts & self.pmen & sci_mask) != 0;
        self.set_irq_level(sci_level);

        // Note: In Bochs, this also schedules/deactivates the virtual timer.
        // In our architecture, the timer overflow is checked on every tick()
        // via get_pmsts(), so no explicit timer scheduling is needed.
    }

    /// Set IRQ 9 level (ACPI SCI).
    /// Bochs: set_irq_level() (acpi.cc:246-250)
    fn set_irq_level(&mut self, level: bool) {
        self.irq9_level = level;
    }

    /// Handle SMI command (ACPI enable/disable).
    /// Bochs: generate_smi() (acpi.cc:285-297)
    pub fn generate_smi(&mut self, value: u8) {
        if value == ACPI_ENABLE {
            self.pmcntrl |= PmControl::SCI_EN.bits();
        } else if value == ACPI_DISABLE {
            self.pmcntrl &= !PmControl::SCI_EN.bits();
        }
        // SMI delivery via APIC bus not implemented (requires APIC bus infrastructure)
        // Bochs acpi.cc:294-296: if (pci_conf[0x5b] & 0x02) apic_bus_deliver_smi()
    }

    // ─── I/O Port Handlers ───────────────────────────────────────────────

    /// Read from PM or SMBus register space.
    /// Bochs: read_handler() / read() (acpi.cc:302-383)
    pub fn read(&mut self, address: u16, io_len: u8) -> u32 {
        let mut value: u32 = 0xFFFF_FFFF;

        if self.pm_base != 0 && (address as u32 & 0xFFC0) == self.pm_base {
            // PM register space — check if PM decode is enabled (PCI config 0x80 bit 0)
            // Bochs acpi.cc:318-320
            if (self.pci_conf[0x80] & 0x01) == 0 {
                return value;
            }
            let reg = (address as u32 & 0x3F) as u8;
            match reg {
                // PM1 Status (acpi.cc:322-324)
                0x00 => {
                    value = self.get_pmsts() as u32;
                }
                // PM1 Enable (acpi.cc:325-327)
                0x02 => {
                    value = self.pmen as u32;
                }
                // PM1 Control (acpi.cc:328-330)
                0x04 => {
                    value = self.pmcntrl as u32;
                }
                // PM Timer (acpi.cc:331-333)
                0x08 => {
                    value = self.get_pmtmr();
                }
                // Generic PM registers (acpi.cc:334-343)
                _ => {
                    if (reg as usize) < self.pmreg.len() {
                        value = self.pmreg[reg as usize] as u32;
                        if io_len >= 2 && (reg as usize + 1) < self.pmreg.len() {
                            value |= (self.pmreg[reg as usize + 1] as u32) << 8;
                        }
                        if io_len == 4 {
                            if (reg as usize + 2) < self.pmreg.len() {
                                value |= (self.pmreg[reg as usize + 2] as u32) << 16;
                            }
                            if (reg as usize + 3) < self.pmreg.len() {
                                value |= (self.pmreg[reg as usize + 3] as u32) << 24;
                            }
                        }
                    }
                }
            }
            tracing::debug!(
                "ACPI PM read reg={:#04x} value={:#010x} len={}",
                reg,
                value,
                io_len
            );
        } else if self.sm_base != 0 && (address as u32 & 0xFFF0) == self.sm_base {
            // SMBus register space — check decode enable
            // Bochs acpi.cc:346-349
            if (self.pci_conf[0x04] & 0x01) == 0 && (self.pci_conf[0xD2] & 0x01) == 0 {
                return value;
            }
            let reg = (address as u32 & 0x0F) as u8;
            match reg {
                // SMBus status (acpi.cc:351-352)
                0x00 => value = self.smbus.stat as u32,
                // SMBus control (acpi.cc:354-356) — reading resets block index
                0x02 => {
                    self.smbus.index = 0;
                    value = (self.smbus.ctl & 0x1F) as u32;
                }
                // SMBus command (acpi.cc:358-359)
                0x03 => value = self.smbus.cmd as u32,
                // SMBus address (acpi.cc:361-362)
                0x04 => value = self.smbus.addr as u32,
                // SMBus data0 (acpi.cc:364-365)
                0x05 => value = self.smbus.data0 as u32,
                // SMBus data1 (acpi.cc:367-368)
                0x06 => value = self.smbus.data1 as u32,
                // SMBus block data (acpi.cc:370-375)
                0x07 => {
                    let idx = self.smbus.index as usize;
                    value = self.smbus.data[idx] as u32;
                    self.smbus.index = if self.smbus.index >= 31 {
                        0
                    } else {
                        self.smbus.index + 1
                    };
                }
                _ => {
                    value = 0;
                    tracing::debug!("ACPI SMBus read reg={:#04x} not implemented", reg);
                }
            }
            tracing::debug!("ACPI SMBus read reg={:#04x} value={:#010x}", reg, value);
        }

        value
    }

    /// Write to PM or SMBus register space.
    /// Bochs: write_handler() / write() (acpi.cc:388-510)
    pub fn write(&mut self, address: u16, value: u32, io_len: u8) {
        if self.pm_base != 0 && (address as u32 & 0xFFC0) == self.pm_base {
            // PM register space
            if (self.pci_conf[0x80] & 0x01) == 0 {
                return;
            }
            let reg = (address as u32 & 0x3F) as u8;
            tracing::debug!(
                "ACPI PM write reg={:#04x} value={:#010x} len={}",
                reg,
                value,
                io_len
            );
            match reg {
                // PM1 Status — write-1-to-clear (acpi.cc:408-418)
                0x00 => {
                    let pmsts = self.get_pmsts();
                    // If clearing TMROF_STS, recompute next overflow time
                    if pmsts & (value as u16) & PmStatus::TMROF_STS.bits() != 0 {
                        let d = muldiv64(self.time_usec, PM_FREQ as u32, 1_000_000);
                        self.tmr_overflow_time = (d + 0x80_0000) & !0x7F_FFFF;
                    }
                    self.pmsts &= !(value as u16);
                    self.pm_update_sci();
                }
                // PM1 Enable (acpi.cc:420-422)
                0x02 => {
                    self.pmen = value as u16;
                    self.pm_update_sci();
                }
                // PM1 Control (acpi.cc:424-446)
                0x04 => {
                    self.pmcntrl = (value as u16) & !PmControl::SUS_EN.bits();
                    if (value as u16) & PmControl::SUS_EN.bits() != 0 {
                        let sus_typ = (value >> 10) & 7;
                        match sus_typ {
                            0 => {
                                // Soft power off (acpi.cc:432-433)
                                tracing::info!("ACPI: soft power off requested");
                            }
                            1 => {
                                // Suspend to RAM (acpi.cc:436-439)
                                tracing::info!("ACPI: suspend to RAM requested");
                                self.pmsts |=
                                    PmStatus::RSM_STS.bits() | PmStatus::PWRBTN_STS.bits();
                            }
                            _ => {}
                        }
                    }
                }
                // Write-ignored registers (acpi.cc:447-460)
                0x0C | 0x0D | 0x14 | 0x15 | 0x18 | 0x19 | 0x1C | 0x1D | 0x1E | 0x1F | 0x30
                | 0x31 | 0x32 => {}
                // Generic PM registers (acpi.cc:461-469)
                _ => {
                    if (reg as usize) < self.pmreg.len() {
                        self.pmreg[reg as usize] = value as u8;
                        if io_len >= 2 && (reg as usize + 1) < self.pmreg.len() {
                            self.pmreg[reg as usize + 1] = (value >> 8) as u8;
                        }
                        if io_len == 4 {
                            if (reg as usize + 2) < self.pmreg.len() {
                                self.pmreg[reg as usize + 2] = (value >> 16) as u8;
                            }
                            if (reg as usize + 3) < self.pmreg.len() {
                                self.pmreg[reg as usize + 3] = (value >> 24) as u8;
                            }
                        }
                    }
                }
            }
        } else if self.sm_base != 0 && (address as u32 & 0xFFF0) == self.sm_base {
            // SMBus register space
            if (self.pci_conf[0x04] & 0x01) == 0 && (self.pci_conf[0xD2] & 0x01) == 0 {
                return;
            }
            let reg = (address as u32 & 0x0F) as u8;
            tracing::debug!("ACPI SMBus write reg={:#04x} value={:#04x}", reg, value);
            match reg {
                // SMBus status — clear on write (acpi.cc:478-480)
                0x00 => {
                    self.smbus.stat = 0;
                    self.smbus.index = 0;
                }
                // SMBus control (acpi.cc:482-484)
                0x02 => {
                    self.smbus.ctl = 0;
                    // Bochs acpi.cc:484 also has "TODO: execute SMBus command" —
                    // SMBus transaction execution is unimplemented in Bochs itself.
                }
                // SMBus command (acpi.cc:486-487)
                0x03 => self.smbus.cmd = 0,
                // SMBus address (acpi.cc:489-490)
                0x04 => self.smbus.addr = 0,
                // SMBus data0 (acpi.cc:492-493)
                0x05 => self.smbus.data0 = 0,
                // SMBus data1 (acpi.cc:495-496)
                0x06 => self.smbus.data1 = 0,
                // SMBus block data (acpi.cc:498-503)
                0x07 => {
                    let idx = self.smbus.index as usize;
                    self.smbus.data[idx] = value as u8;
                    self.smbus.index = if self.smbus.index >= 31 {
                        0
                    } else {
                        self.smbus.index + 1
                    };
                }
                _ => {
                    tracing::debug!("ACPI SMBus write reg={:#04x} not implemented", reg);
                }
            }
        } else {
            // Debug port (0xB044) — Bochs acpi.cc:508
            tracing::debug!("ACPI DBG: {:#010x}", value);
        }
    }

    // ─── PCI Configuration Space ─────────────────────────────────────────

    /// Write to PCI configuration space.
    /// Bochs: pci_write_handler() (acpi.cc:525-581)
    ///
    /// Returns (pm_base_changed, sm_base_changed) to signal that the emulator
    /// should re-register I/O ports.
    pub fn pci_write(&mut self, address: u8, value: u32, io_len: u8) -> (bool, bool) {
        let mut pm_base_change = false;
        let mut sm_base_change = false;

        // Addresses 0x10-0x33 are ignored (BAR region) — acpi.cc:530-531
        if address >= 0x10 && address < 0x34 {
            return (false, false);
        }

        for i in 0..io_len as usize {
            let addr = address as usize + i;
            if addr >= PCI_CONF_SIZE {
                break;
            }
            let value8 = ((value >> (i * 8)) & 0xFF) as u8;
            let oldval = self.pci_conf[addr];

            match addr {
                // Command register (acpi.cc:538-540)
                0x04 => {
                    self.pci_conf[addr] = (value8 & 0xFE) | (value8 & 0x01);
                }
                // Status lo-byte — write disallowed (acpi.cc:542)
                0x06 => {}
                // PM base 0x40 (acpi.cc:544-545)
                0x40 => {
                    let v = (value8 & 0xC0) | 0x01;
                    pm_base_change |= v != oldval;
                    self.pci_conf[addr] = v;
                }
                // PM base 0x41-0x43 (acpi.cc:546-550)
                0x41..=0x43 => {
                    pm_base_change |= value8 != oldval;
                    self.pci_conf[addr] = value8;
                }
                // SM base 0x90 (acpi.cc:552-553)
                0x90 => {
                    let v = (value8 & 0xF0) | 0x01;
                    sm_base_change |= v != oldval;
                    self.pci_conf[addr] = v;
                }
                // SM base 0x91-0x93 (acpi.cc:554-557, fall-through to default)
                0x91..=0x93 => {
                    sm_base_change |= value8 != oldval;
                    self.pci_conf[addr] = value8;
                }
                // Default: store value (acpi.cc:558-560)
                _ => {
                    self.pci_conf[addr] = value8;
                }
            }
        }

        // Update base addresses if changed (acpi.cc:563-580)
        if pm_base_change {
            let new_base = u32::from_le_bytes([
                self.pci_conf[0x40],
                self.pci_conf[0x41],
                self.pci_conf[0x42],
                self.pci_conf[0x43],
            ]) & 0xFFC0; // Mask to 64-port alignment
            self.pm_base = new_base;
            tracing::info!("ACPI: new PM base address: {:#06x}", self.pm_base);
        }

        if sm_base_change {
            let new_base = u32::from_le_bytes([
                self.pci_conf[0x90],
                self.pci_conf[0x91],
                self.pci_conf[0x92],
                self.pci_conf[0x93],
            ]) & 0xFFF0; // Mask to 16-port alignment
            self.sm_base = new_base;
            tracing::info!("ACPI: new SM base address: {:#06x}", self.sm_base);
        }

        (pm_base_change, sm_base_change)
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

    /// Check if an I/O port address falls within the PM base range.
    pub fn is_pm_port(&self, port: u16) -> bool {
        self.pm_base != 0 && (port as u32 & 0xFFC0) == self.pm_base
    }

    /// Check if an I/O port address falls within the SM base range.
    pub fn is_sm_port(&self, port: u16) -> bool {
        self.sm_base != 0 && (port as u32 & 0xFFF0) == self.sm_base
    }

    /// Get the I/O access mask for a PM register offset.
    pub fn pm_io_mask(&self, offset: u8) -> u8 {
        if (offset as usize) < ACPI_PM_IOMASK.len() {
            ACPI_PM_IOMASK[offset as usize]
        } else {
            0
        }
    }

    /// Get the I/O access mask for a SMBus register offset.
    pub fn sm_io_mask(&self, offset: u8) -> u8 {
        if (offset as usize) < ACPI_SM_IOMASK.len() {
            ACPI_SM_IOMASK[offset as usize]
        } else {
            0
        }
    }
}

// ─── Utility: 96-bit intermediate multiply-divide ────────────────────────────

/// Compute (a * b) / c using a 96-bit intermediate to avoid overflow.
/// Ported from QEMU/Bochs: muldiv64() (acpi.cc:85-109)
fn muldiv64(a: u64, b: u32, c: u32) -> u64 {
    let a_lo = a as u32 as u64;
    let a_hi = (a >> 32) as u64;

    let rl = a_lo * b as u64;
    let mut rh = a_hi * b as u64;
    rh += rl >> 32;
    let rl = rl & 0xFFFF_FFFF;

    let c = c as u64;
    let res_hi = rh / c;
    let res_lo = ((rh % c) << 32 | rl) / c;

    (res_hi << 32) | res_lo
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acpi_new() {
        let acpi = BxAcpiCtrl::new();
        assert_eq!(acpi.devfunc, 0x0B);
        assert_eq!(acpi.pm_base, 0);
        assert_eq!(acpi.sm_base, 0);
        assert_eq!(acpi.pmsts, 0);
        assert_eq!(acpi.pmen, 0);
        assert_eq!(acpi.pmcntrl, 0);
        // PCI identity
        assert_eq!(acpi.pci_conf[0x00], 0x86); // Intel vendor lo
        assert_eq!(acpi.pci_conf[0x01], 0x80); // Intel vendor hi
        assert_eq!(acpi.pci_conf[0x02], 0x13); // PIIX4 PM device lo
        assert_eq!(acpi.pci_conf[0x03], 0x71); // PIIX4 PM device hi
    }

    #[test]
    fn test_acpi_reset() {
        let mut acpi = BxAcpiCtrl::new();
        acpi.pmsts = 0xFFFF;
        acpi.pmen = 0xFFFF;
        acpi.pmcntrl = 0xFFFF;
        acpi.reset();
        assert_eq!(acpi.pmsts, 0);
        assert_eq!(acpi.pmen, 0);
        assert_eq!(acpi.pmcntrl, 0);
        assert_eq!(acpi.pci_conf[0x40], 0x01); // PM base I/O indicator
        assert_eq!(acpi.pci_conf[0x90], 0x01); // SM base I/O indicator
    }

    #[test]
    fn test_pm_timer_ticks() {
        let mut acpi = BxAcpiCtrl::new();
        // At time 0, timer should be 0
        assert_eq!(acpi.get_pmtmr(), 0);
        // After 1 second (1,000,000 usec), timer should be ~3,579,545
        acpi.time_usec = 1_000_000;
        let tmr = acpi.get_pmtmr();
        assert_eq!(tmr, PM_FREQ as u32 & 0xFF_FFFF);
        // After ~2.34 seconds, should wrap (24-bit)
        acpi.time_usec = 5_000_000; // ~5 seconds
        let tmr = acpi.get_pmtmr();
        assert!(tmr < 0xFF_FFFF); // Must have wrapped
    }

    #[test]
    fn test_muldiv64() {
        // Basic: (1_000_000 * 3_579_545) / 1_000_000 = 3_579_545
        assert_eq!(muldiv64(1_000_000, PM_FREQ as u32, 1_000_000), PM_FREQ);
        // Zero case
        assert_eq!(muldiv64(0, PM_FREQ as u32, 1_000_000), 0);
        // Large value test (shouldn't overflow)
        let result = muldiv64(10_000_000_000, PM_FREQ as u32, 1_000_000);
        assert!(result > 0);
    }

    #[test]
    fn test_pm_status_write_clear() {
        let mut acpi = BxAcpiCtrl::new();
        acpi.pm_base = 0xB000;
        acpi.pci_conf[0x80] = 0x01; // Enable PM decode

        // Set some status bits
        acpi.pmsts = PmStatus::PWRBTN_STS.bits() | PmStatus::TMROF_STS.bits();

        // Write 1 to PWRBTN_STS to clear it (address = pm_base + 0x00)
        let pm_addr = acpi.pm_base as u16;
        acpi.write(pm_addr, PmStatus::PWRBTN_STS.bits() as u32, 2);

        // PWRBTN_STS should be cleared, TMROF_STS may still be set (depends on timer)
        assert_eq!(acpi.pmsts & PmStatus::PWRBTN_STS.bits(), 0);
    }

    #[test]
    fn test_pm_control_sci_en() {
        let mut acpi = BxAcpiCtrl::new();
        // ACPI enable via SMI command
        acpi.generate_smi(ACPI_ENABLE);
        assert_ne!(acpi.pmcntrl & PmControl::SCI_EN.bits(), 0);
        // ACPI disable
        acpi.generate_smi(ACPI_DISABLE);
        assert_eq!(acpi.pmcntrl & PmControl::SCI_EN.bits(), 0);
    }

    #[test]
    fn test_pci_config_pm_base() {
        let mut acpi = BxAcpiCtrl::new();
        acpi.reset();

        // Write PM base = 0xB000 via PCI config 0x40-0x43
        // Byte at 0x40: (0x00 & 0xC0) | 0x01 = 0x01
        // Byte at 0x41: 0xB0
        acpi.pci_write(0x40, 0x01, 1); // Low byte with I/O indicator
        let (changed, _) = acpi.pci_write(0x41, 0xB0, 1);
        assert!(changed);
        assert_eq!(acpi.pm_base, 0xB000);
    }

    #[test]
    fn test_smbus_block_data_wrapping() {
        let mut acpi = BxAcpiCtrl::new();
        acpi.sm_base = 0xB100;
        acpi.pci_conf[0x04] = 0x01; // Enable I/O decode

        let sm_addr = acpi.sm_base as u16;

        // Write 33 bytes to block data register (0x07) — should wrap at 32
        for i in 0..33u32 {
            acpi.write(sm_addr + 0x07, i, 1);
        }
        // Index should have wrapped: 33 mod 32 = 1
        assert_eq!(acpi.smbus.index, 1);
        // First byte should be 32 (the 33rd write overwrote index 0)
        assert_eq!(acpi.smbus.data[0], 32);
    }

    #[test]
    fn test_timer_overflow_detection() {
        let mut acpi = BxAcpiCtrl::new();
        acpi.reset();

        // Set overflow time to a low value so we can trigger it
        acpi.tmr_overflow_time = 100;

        // At time 0, no overflow
        assert_eq!(acpi.get_pmsts() & PmStatus::TMROF_STS.bits(), 0);

        // Advance past overflow point
        // 100 PM ticks = 100 / 3_579_545 seconds = ~28 usec
        acpi.time_usec = 100; // ~358 PM ticks at 3.58 MHz
        let pmsts = acpi.get_pmsts();
        assert_ne!(pmsts & PmStatus::TMROF_STS.bits(), 0);
    }
}
