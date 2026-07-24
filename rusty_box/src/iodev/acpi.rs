#![allow(dead_code)]
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

#[cfg(feature = "std")]
use std::io::{self, Read, Write};

#[cfg(feature = "std")]
use crate::snapshot::{
    bounds, checked_snapshot_len_add, checked_snapshot_len_mul, SnapshotReader, SnapshotWriteExt,
    SNAPSHOT_SECTION_VERSION,
};

/// PM timer frequency: 3.579545 MHz (ACPI spec, section 4.7.3.3)
const PM_FREQ: u64 = 3_579_545;

/// Debug I/O port address (Bochs acpi.cc)
const ACPI_DBG_IO_ADDR: u16 = 0xB044;

/// SMI command port (ACPI spec — FADT SmiCmd field)
/// The BIOS writes ACPI_ENABLE/ACPI_DISABLE here.
const SMI_CMD_PORT: u16 = 0x00B2;

/// ACPI enable command value (Bochs acpi.cc)
const ACPI_ENABLE: u8 = 0xF1;
/// ACPI disable command value (Bochs acpi.cc)
const ACPI_DISABLE: u8 = 0xF0;

// ─── PM Status Register bits (Bochs acpi.cc) ──────────────────────────

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
/// Bochs acpi.cc
const ACPI_PM_IOMASK: [u8; 64] = [
    3, 0, 3, 0, 3, 0, 0, 0, 4, 0, 0, 0, 3, 1, 3, 1, 7, 1, 3, 1, 1, 1, 0, 0, 3, 1, 0, 0, 7, 1, 3, 1,
    3, 1, 0, 0, 0, 0, 0, 0, 7, 1, 3, 1, 7, 1, 3, 1, 1, 1, 1, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// I/O access mask for SMBus register space (16 ports).
/// Bochs acpi.cc
const ACPI_SM_IOMASK: [u8; 16] = [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 0, 2, 0, 0, 0];

// ─── SMBus state ─────────────────────────────────────────────────────────────

/// SMBus host controller state (Bochs acpi.h)
#[derive(Debug, Clone, Default)]
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

// ─── PCI Configuration Space ─────────────────────────────────────────────────

/// PCI configuration space size
const PCI_CONF_SIZE: usize = 256;

// ─── ACPI Controller ─────────────────────────────────────────────────────────

/// PIIX4 ACPI Power Management controller.
/// Bochs: bx_acpi_ctrl_c (acpi.h, acpi.cc)
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
    /// Fixed PC-system timer registered for PM timer overflow ownership.
    ///
    /// This is initialized by the device manager and deliberately survives a
    /// soft reset, matching the lifetime of Bochs's virtual timer slot.
    pub(crate) overflow_timer_handle: Option<usize>,

    /// Generic PM register space (56 bytes, Bochs: s.pmreg[0x38])
    pmreg: [u8; 0x38],

    /// SMBus host controller state
    smbus: SmBusState,

    /// PCI configuration space (256 bytes)
    pub pci_conf: [u8; PCI_CONF_SIZE],

    /// Accumulated microseconds for PM timer computation.
    /// Mirrors Bochs `bx_virt_timer.time_usec()`: `tick()` advances the
    /// synchronized base, and PM timer reads add the live icount delta so tight
    /// guest polling loops do not see a stale timer for an entire CPU batch.
    pub time_usec: u64,
    /// Whether icount-based live PM timer sync has been initialized.
    has_icount_sync: bool,
    /// CPU icount at the last `time_usec` synchronization point.
    icount_at_sync: u64,
    /// Instructions per second used to convert icount deltas to usec.
    ips: u64,
    /// Host clock anchor for Bochs-style realtime PM timer synchronization.
    #[cfg(feature = "std")]
    realtime_start: Option<std::time::Instant>,

    /// IRQ 9 level (SCI) — the emulator loop syncs this to the PIC.
    pub irq9_level: bool,

    /// A `generate_smi` fired with APMC_EN set (Bochs acpi.cc
    /// `apic_bus_deliver_smi()`); the emulator drains this at the scheduler
    /// boundary and delivers the SMI to CPU 0. Transient (set by an OUT to
    /// SMI_CMD, consumed at the very next boundary before any further guest
    /// instruction), so it is deliberately not snapshotted — snapshots are
    /// taken at serviced boundaries where it is always clear.
    pub(crate) smi_request_pending: bool,

    /// Whether PM I/O ports are registered (tracks pm_base changes)
    pub(crate) pm_ports_registered: bool,
    /// Whether SM I/O ports are registered (tracks sm_base changes)
    pub(crate) sm_ports_registered: bool,

    /// When true, seed QEMU-like PM base defaults for UEFI firmware (OVMF).
    pub uefi_enabled: bool,
}

/// Deferred ACPI topology and timer binding selected by a restored snapshot.
///
/// The parent restore transaction validates the timer owner and atomically
/// relocates from the live PM/SM I/O ranges before committing these bases.
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AcpiSnapshotRestore {
    pub(crate) pm_base: u32,
    pub(crate) sm_base: u32,
    pub(crate) overflow_timer_handle: Option<usize>,
}

impl Default for BxAcpiCtrl {
    fn default() -> Self {
        Self::new()
    }
}

impl BxAcpiCtrl {
    #[cfg(feature = "std")]
    fn invalid_snapshot_v3(message: &'static str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }

    #[cfg(feature = "std")]
    fn validate_snapshot_v3_pci_identity(pci_conf: &[u8; PCI_CONF_SIZE]) -> io::Result<()> {
        const PIIX4_PM_IDENTITY: [(usize, u8); 8] = [
            (0x00, 0x86),
            (0x01, 0x80),
            (0x02, 0x13),
            (0x03, 0x71),
            (0x08, 0x03),
            (0x09, 0x00),
            (0x0A, 0x80),
            (0x0B, 0x06),
        ];

        for &(offset, expected) in &PIIX4_PM_IDENTITY {
            if pci_conf[offset] != expected {
                return Err(Self::invalid_snapshot_v3(
                    "ACPI immutable PCI identity differs",
                ));
            }
        }
        Ok(())
    }

    #[cfg(feature = "std")]
    fn validate_snapshot_v3_bases(
        pm_base: u32,
        sm_base: u32,
        pci_conf: &[u8; PCI_CONF_SIZE],
    ) -> io::Result<()> {
        let pmbar = u32::from_le_bytes([
            pci_conf[0x40],
            pci_conf[0x41],
            pci_conf[0x42],
            pci_conf[0x43],
        ]);
        let smbar = u32::from_le_bytes([
            pci_conf[0x90],
            pci_conf[0x91],
            pci_conf[0x92],
            pci_conf[0x93],
        ]);

        let pm_low = pmbar & 0x3F;
        if (pm_low != 0 && pm_low != 1)
            || pm_base & 0x3F != 0
            || pm_base > 0xFFC0
            || pm_base != pmbar & 0xFFC0
        {
            return Err(Self::invalid_snapshot_v3(
                "ACPI PM base is malformed or does not match its PCI BAR",
            ));
        }

        let sm_low = smbar & 0x0F;
        if (sm_low != 0 && sm_low != 1)
            || sm_base & 0x0F != 0
            || sm_base > 0xFFF0
            || sm_base != smbar & 0xFFF0
        {
            return Err(Self::invalid_snapshot_v3(
                "ACPI SMBus base is malformed or does not match its PCI BAR",
            ));
        }

        Ok(())
    }

    #[cfg(feature = "std")]
    #[allow(clippy::too_many_arguments)]
    fn validate_snapshot_v3_state(
        &self,
        devfunc: u8,
        uefi_enabled: bool,
        ips: u64,
        pmsts: u16,
        pmen: u16,
        pmcntrl: u16,
        smbus_index: u8,
        pci_conf: &[u8; PCI_CONF_SIZE],
        pm_base: u32,
        sm_base: u32,
    ) -> io::Result<()> {
        const PM_STATUS_MASK: u16 = 0x8731;
        const PM_ENABLE_MASK: u16 = 0x0521;
        const PM_CONTROL_MASK: u16 = 0x3C07;

        if devfunc != self.devfunc {
            return Err(Self::invalid_snapshot_v3(
                "ACPI PCI device/function differs from live configuration",
            ));
        }
        if uefi_enabled != self.uefi_enabled {
            return Err(Self::invalid_snapshot_v3(
                "ACPI UEFI configuration differs from live configuration",
            ));
        }
        if ips != self.ips {
            return Err(Self::invalid_snapshot_v3(
                "ACPI instructions-per-second differs from live configuration",
            ));
        }
        if pmsts & !PM_STATUS_MASK != 0
            || pmen & !PM_ENABLE_MASK != 0
            || pmcntrl & !PM_CONTROL_MASK != 0
        {
            return Err(Self::invalid_snapshot_v3(
                "ACPI PM register contains reserved bits",
            ));
        }
        if usize::from(smbus_index) >= 32 {
            return Err(Self::invalid_snapshot_v3("ACPI SMBus block index is invalid"));
        }

        Self::validate_snapshot_v3_pci_identity(pci_conf)?;
        Self::validate_snapshot_v3_bases(pm_base, sm_base, pci_conf)
    }

    /// Exact byte count for the single-section ACPI v3 payload.
    #[cfg(feature = "std")]
    pub(crate) fn snapshot_v3_len(&self) -> io::Result<u64> {
        self.validate_snapshot_v3_state(
            self.devfunc,
            self.uefi_enabled,
            self.ips,
            self.pmsts,
            self.pmen,
            self.pmcntrl,
            self.smbus.index,
            &self.pci_conf,
            self.pm_base,
            self.sm_base,
        )?;

        let pmreg_len = checked_snapshot_len_mul(
            u64::try_from(self.pmreg.len())
                .map_err(|_| Self::invalid_snapshot_v3("ACPI PM register length does not fit"))?,
            1,
        )?;
        let smbus_data_len = checked_snapshot_len_mul(
            u64::try_from(self.smbus.data.len())
                .map_err(|_| Self::invalid_snapshot_v3("ACPI SMBus data length does not fit"))?,
            1,
        )?;
        let pci_conf_len = checked_snapshot_len_mul(
            u64::try_from(self.pci_conf.len())
                .map_err(|_| Self::invalid_snapshot_v3("ACPI PCI config length does not fit"))?,
            1,
        )?;
        let pm_register_len = checked_snapshot_len_mul(3, 2)?;
        let smbus_register_len = checked_snapshot_len_mul(7, 1)?;
        let base_len = checked_snapshot_len_mul(2, 4)?;

        let mut len = 0;
        for component_len in [
            4,
            1,
            1,
            8,
            pm_register_len,
            8,
            1,
            pmreg_len,
            smbus_register_len,
            smbus_data_len,
            pci_conf_len,
            8,
            1,
            base_len,
        ] {
            len = checked_snapshot_len_add(len, component_len)?;
        }
        if let Some(handle) = self.overflow_timer_handle {
            u64::try_from(handle)
                .map_err(|_| Self::invalid_snapshot_v3("ACPI timer handle does not fit"))?;
            len = checked_snapshot_len_add(len, 8)?;
        }
        if len > bounds::MAX_SNAPSHOT_SECTION_LEN {
            return Err(Self::invalid_snapshot_v3(
                "ACPI snapshot payload exceeds implementation bound",
            ));
        }
        Ok(len)
    }

    /// Stream all serializable ACPI state into a versioned v3 section payload.
    #[cfg(feature = "std")]
    pub(crate) fn save_snapshot_v3<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.snapshot_v3_len()?;

        writer.write_u32(SNAPSHOT_SECTION_VERSION)?;
        writer.write_u8(self.devfunc)?;
        writer.write_bool(self.uefi_enabled)?;
        writer.write_u64(self.ips)?;
        writer.write_u16(self.pmsts)?;
        writer.write_u16(self.pmen)?;
        writer.write_u16(self.pmcntrl)?;
        writer.write_u64(self.tmr_overflow_time)?;
        writer.write_bool(self.overflow_timer_handle.is_some())?;
        if let Some(handle) = self.overflow_timer_handle {
            writer.write_u64(
                u64::try_from(handle)
                    .map_err(|_| Self::invalid_snapshot_v3("ACPI timer handle does not fit"))?,
            )?;
        }
        writer.write_bytes(&self.pmreg)?;
        writer.write_u8(self.smbus.stat)?;
        writer.write_u8(self.smbus.ctl)?;
        writer.write_u8(self.smbus.cmd)?;
        writer.write_u8(self.smbus.addr)?;
        writer.write_u8(self.smbus.data0)?;
        writer.write_u8(self.smbus.data1)?;
        writer.write_u8(self.smbus.index)?;
        writer.write_bytes(&self.smbus.data)?;
        writer.write_bytes(&self.pci_conf)?;
        writer.write_u64(self.time_usec)?;
        writer.write_bool(self.irq9_level)?;
        writer.write_u32(self.pm_base)?;
        writer.write_u32(self.sm_base)
    }

    /// Restore ACPI state without changing live I/O registrations.
    ///
    /// PM/SM bases and the raw timer slot are returned for parent-owned
    /// validation and atomic topology relocation.
    #[cfg(feature = "std")]
    pub(crate) fn restore_snapshot_v3<R: Read>(
        &mut self,
        reader: &mut SnapshotReader<R>,
    ) -> io::Result<AcpiSnapshotRestore> {
        let section_version = reader.read_u32()?;
        if section_version != SNAPSHOT_SECTION_VERSION {
            return Err(Self::invalid_snapshot_v3(
                "unsupported ACPI snapshot section version",
            ));
        }

        let devfunc = reader.read_u8()?;
        let uefi_enabled = reader.read_bool()?;
        let ips = reader.read_u64()?;
        let pmsts = reader.read_u16()?;
        let pmen = reader.read_u16()?;
        let pmcntrl = reader.read_u16()?;
        let tmr_overflow_time = reader.read_u64()?;
        let overflow_timer_handle = if reader.read_bool()? {
            Some(
                usize::try_from(reader.read_u64()?).map_err(|_| {
                    Self::invalid_snapshot_v3("ACPI timer handle does not fit host index")
                })?,
            )
        } else {
            None
        };
        let mut pmreg = [0; 0x38];
        reader.read_bytes(&mut pmreg)?;
        let smbus_stat = reader.read_u8()?;
        let smbus_ctl = reader.read_u8()?;
        let smbus_cmd = reader.read_u8()?;
        let smbus_addr = reader.read_u8()?;
        let smbus_data0 = reader.read_u8()?;
        let smbus_data1 = reader.read_u8()?;
        let smbus_index = reader.read_u8()?;
        let mut smbus_data = [0; 32];
        reader.read_bytes(&mut smbus_data)?;
        let mut pci_conf = [0; PCI_CONF_SIZE];
        reader.read_bytes(&mut pci_conf)?;
        let time_usec = reader.read_u64()?;
        let irq9_level = reader.read_bool()?;
        let pm_base = reader.read_u32()?;
        let sm_base = reader.read_u32()?;
        reader.finish_exact()?;

        self.validate_snapshot_v3_state(
            devfunc,
            uefi_enabled,
            ips,
            pmsts,
            pmen,
            pmcntrl,
            smbus_index,
            &pci_conf,
            pm_base,
            sm_base,
        )?;

        self.pmsts = pmsts;
        self.pmen = pmen;
        self.pmcntrl = pmcntrl;
        self.tmr_overflow_time = tmr_overflow_time;
        self.pmreg = pmreg;
        self.smbus = SmBusState {
            stat: smbus_stat,
            ctl: smbus_ctl,
            cmd: smbus_cmd,
            addr: smbus_addr,
            data0: smbus_data0,
            data1: smbus_data1,
            index: smbus_index,
            data: smbus_data,
        };
        self.pci_conf = pci_conf;
        self.time_usec = time_usec;
        self.irq9_level = irq9_level;

        Ok(AcpiSnapshotRestore {
            pm_base,
            sm_base,
            overflow_timer_handle,
        })
    }

    /// Recreate host-only timer anchors and derive SCI without injecting an edge.
    #[cfg(feature = "std")]
    pub(crate) fn post_restore_snapshot_v3(&mut self, system_ticks: u64) -> bool {
        self.has_icount_sync = self.ips != 0;
        self.icount_at_sync = system_ticks;

        if self.realtime_start.is_some() {
            let now = std::time::Instant::now();
            self.realtime_start =
                now.checked_sub(std::time::Duration::from_micros(self.time_usec));
        }

        self.pm_update_sci(system_ticks);
        self.irq9_level
    }

    /// Create a new ACPI controller instance.
    /// Bochs: bx_acpi_ctrl_c::bx_acpi_ctrl_c() (acpi.cc)
    pub fn new() -> Self {
        let mut ctrl = Self {
            devfunc: 0x0B, // BX_PCI_DEVICE(1, 3) = (1 << 3) | 3 = 0x0B
            pm_base: 0,
            sm_base: 0,
            pmsts: 0,
            pmen: 0,
            pmcntrl: 0,
            tmr_overflow_time: 0xFF_FFFF, // 24-bit max (Bochs acpi.cc)
            overflow_timer_handle: None,
            pmreg: [0; 0x38],
            smbus: SmBusState::default(),
            pci_conf: [0; PCI_CONF_SIZE],
            time_usec: 0,
            has_icount_sync: false,
            icount_at_sync: 0,
            ips: 0,
            #[cfg(feature = "std")]
            realtime_start: None,
            irq9_level: false,
            smi_request_pending: false,
            pm_ports_registered: false,
            sm_ports_registered: false,
            uefi_enabled: false,
        };
        ctrl.init_pci_conf();
        ctrl
    }

    /// Initialize PCI configuration space with PIIX4 PM identity.
    /// Bochs: init_pci_conf(0x8086, 0x7113, 0x03, 0x068000, 0x00, 0) (acpi.cc)
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
    /// Bochs: bx_acpi_ctrl_c::reset() (acpi.cc)
    pub fn reset(&mut self) {
        // PCI command/status (acpi.cc)
        self.pci_conf[0x04] = 0x00;
        self.pci_conf[0x05] = 0x00;
        self.pci_conf[0x06] = 0x80; // status_devsel_medium
        self.pci_conf[0x07] = 0x02;
        self.pci_conf[0x3C] = 0x00; // IRQ

        // PM base 0x40-0x43 (acpi.cc)
        // When running with UEFI firmware (OVMF), seed QEMU-like defaults so the
        // ACPI PM timer is available very early.
        if !self.uefi_enabled {
            // Upstream-compatible reset behavior.
            self.pci_conf[0x40] = 0x01;
            self.pci_conf[0x41] = 0x00;
            self.pci_conf[0x42] = 0x00;
            self.pci_conf[0x43] = 0x00;
        } else {
            let pmbar = u32::from_le_bytes([
                self.pci_conf[0x40],
                self.pci_conf[0x41],
                self.pci_conf[0x42],
                self.pci_conf[0x43],
            ]);
            if (pmbar & 0xFFC0) == 0 {
                // Default to 0xB000 (I/O BAR => bit0 set).
                self.pci_conf[0x40] = 0x01;
                self.pci_conf[0x41] = 0xB0;
                self.pci_conf[0x42] = 0x00;
                self.pci_conf[0x43] = 0x00;
            } else {
                // Preserve base, keep read-only bit semantics on the low byte.
                self.pci_conf[0x40] = (self.pci_conf[0x40] & 0xC0) | 0x01;
            }
        }

        // Clear DEVACTB (acpi.cc)
        self.pci_conf[0x58] = 0x00;
        self.pci_conf[0x59] = 0x00;

        // Device resources (acpi.cc)
        self.pci_conf[0x5A] = 0x00;
        self.pci_conf[0x5B] = 0x00;
        self.pci_conf[0x5F] = 0x90;
        self.pci_conf[0x63] = 0x60;
        self.pci_conf[0x67] = 0x98;

        // SM base 0x90-0x93 (acpi.cc)
        if !self.uefi_enabled {
            // Upstream-compatible reset behavior.
            self.pci_conf[0x90] = 0x01;
            self.pci_conf[0x91] = 0x00;
            self.pci_conf[0x92] = 0x00;
            self.pci_conf[0x93] = 0x00;
        } else {
            let smbar = u32::from_le_bytes([
                self.pci_conf[0x90],
                self.pci_conf[0x91],
                self.pci_conf[0x92],
                self.pci_conf[0x93],
            ]);
            if (smbar & 0xFFF0) == 0 {
                // Default to 0xB100 (I/O BAR => bit0 set).
                self.pci_conf[0x90] = 0x01;
                self.pci_conf[0x91] = 0xB1;
                self.pci_conf[0x92] = 0x00;
                self.pci_conf[0x93] = 0x00;
            } else {
                // Preserve base, keep read-only bit semantics on the low byte.
                self.pci_conf[0x90] = (self.pci_conf[0x90] & 0xF0) | 0x01;
            }
        }

        // Clear PM state (acpi.cc)
        self.pmsts = 0;
        self.pmen = 0;
        self.pmcntrl = 0;
        self.tmr_overflow_time = 0xFF_FFFF;
        self.pmreg = [0; 0x38];

        self.time_usec = 0;
        self.has_icount_sync = false;
        self.icount_at_sync = 0;
        #[cfg(feature = "std")]
        if self.realtime_start.is_some() {
            self.realtime_start = Some(std::time::Instant::now());
        }
        // Clear SMBus state (acpi.cc)
        self.smbus = SmBusState::default();

        self.irq9_level = false;
        self.smi_request_pending = false;

        // Map PM/SM I/O windows when the BAR is configured (e.g. UEFI defaults).
        let pmbar = u32::from_le_bytes([
            self.pci_conf[0x40],
            self.pci_conf[0x41],
            self.pci_conf[0x42],
            self.pci_conf[0x43],
        ]);
        if (pmbar & 0xFFC0) != 0 {
            self.pm_base = pmbar & 0xFFC0;
        }
        let smbar = u32::from_le_bytes([
            self.pci_conf[0x90],
            self.pci_conf[0x91],
            self.pci_conf[0x92],
            self.pci_conf[0x93],
        ]);
        if (smbar & 0xFFF0) != 0 {
            self.sm_base = smbar & 0xFFF0;
        }
    }

    /// Initialize icount-based PM timer synchronization.
    pub fn init_icount_sync(&mut self, icount: u64, ips: u64) {
        self.has_icount_sync = true;
        self.icount_at_sync = icount;
        self.ips = ips;
    }

    /// Enable Bochs-style realtime synchronization for the ACPI PM timer.
    #[cfg(feature = "std")]
    pub fn enable_realtime_sync(&mut self) {
        self.realtime_start = Some(std::time::Instant::now());
    }


    // ─── PM Timer ────────────────────────────────────────────────────────

    #[inline]
    fn live_time_usec(&self, icount: u64) -> u64 {
        #[cfg(feature = "std")]
        if let Some(start) = self.realtime_start {
            return start.elapsed().as_micros() as u64;
        }

        if self.has_icount_sync && self.ips != 0 && icount >= self.icount_at_sync {
            self.time_usec
                .wrapping_add((icount - self.icount_at_sync).saturating_mul(1_000_000) / self.ips)
        } else {
            self.time_usec
        }
    }

    /// Get the 24-bit PM timer value.
    /// Bochs: get_pmtmr() (acpi.cc)
    fn get_pmtmr(&self, icount: u64) -> u32 {
        let value = muldiv64(self.live_time_usec(icount), PM_FREQ as u32, 1_000_000);
        (value & 0xFF_FFFF) as u32
    }

    /// Get PM status with timer overflow check.
    /// Bochs: get_pmsts() (acpi.cc)
    fn get_pmsts(&mut self, icount: u64) -> u16 {
        let value = muldiv64(self.live_time_usec(icount), PM_FREQ as u32, 1_000_000);
        if value >= self.tmr_overflow_time {
            self.pmsts |= PmStatus::TMROF_STS.bits();
        }
        self.pmsts
    }

    /// Update SCI interrupt level based on current status and enable.
    /// Bochs: pm_update_sci() (acpi.cc)
    fn pm_update_sci(&mut self, icount: u64) {
        let pmsts = self.get_pmsts(icount);
        // SCI fires if any enabled status bit is set
        // Bochs acpi.cc: (pmsts & pmen) & (RTC_EN | PWRBTN_EN | GBL_EN | TMROF_EN)
        let sci_mask = PmEnable::RTC_EN.bits()
            | PmEnable::PWRBTN_EN.bits()
            | PmEnable::GBL_EN.bits()
            | PmEnable::TMROF_EN.bits();
        let sci_level = (pmsts & self.pmen & sci_mask) != 0;
        self.set_irq_level(sci_level);

    }
    #[inline]
    fn pm_timer_ticks_at_usec(time_usec: u64) -> u64 {
        muldiv64(time_usec, PM_FREQ as u32, 1_000_000)
    }

    #[inline]
    fn overflow_armed(&self) -> bool {
        self.pmen & PmEnable::TMROF_EN.bits() != 0
            && self.pmsts & PmStatus::TMROF_STS.bits() == 0
    }

    #[inline]
    fn overflow_remaining_from_usec(&self, current_usec: u64) -> Option<u64> {
        if !self.overflow_armed() {
            return None;
        }

        // Bochs schedules the first whole microsecond whose PM tick value has
        // reached the current overflow boundary (acpi.cc:329-332).
        let expire_usec = muldiv64(self.tmr_overflow_time, 1_000_000, PM_FREQ as u32) + 1;
        (expire_usec > current_usec).then_some(expire_usec - current_usec)
    }

    /// Return the relative delay until the enabled PM timer overflow.
    ///
    /// The caller converts this microsecond delay to a fixed pc-system owner
    /// deadline. Re-evaluating after every PM1 register access makes both
    /// rearming and SCI changes visible before guest execution resumes.
    pub(crate) fn overflow_delay_usec(&mut self, system_ticks: u64) -> Option<u64> {
        self.pm_update_sci(system_ticks);
        self.overflow_remaining_from_usec(self.live_time_usec(system_ticks))
    }

    /// Service the ACPI PM-overflow timer owner and return a rearm delay.
    ///
    /// A realtime pc-system deadline is only a prediction. If its callback is
    /// early relative to the freshly sampled host clock, leave PM overflow
    /// state unchanged and return the newly predicted remaining delay.
    pub(crate) fn overflow_timer(&mut self, system_ticks: u64) -> Option<u64> {
        #[cfg(feature = "std")]
        if self.realtime_start.is_some() && self.overflow_armed() {
            let current_usec = self.live_time_usec(system_ticks);
            if Self::pm_timer_ticks_at_usec(current_usec) < self.tmr_overflow_time {
                return self.overflow_remaining_from_usec(current_usec);
            }
        }

        self.overflow_delay_usec(system_ticks)
    }

    /// Set IRQ 9 level (ACPI SCI).
    /// Bochs: set_irq_level() (acpi.cc)
    fn set_irq_level(&mut self, level: bool) {
        self.irq9_level = level;
    }

    /// Handle SMI command (ACPI enable/disable).
    /// Bochs: generate_smi() (acpi.cc)
    /// Bochs acpi.cc `generate_smi`: the ACPI enable/disable commands toggle
    /// SCI_EN directly (ACPI specs 3.0, 4.7.2.5), and when APMC_EN
    /// (`pci_conf[0x5b]` bit 1, set by the BIOS via the SMI-control dword at
    /// config 0x58) is enabled, an SMI is delivered to CPU 0
    /// (`apic_bus_deliver_smi`) — the emulator drains `smi_request_pending`
    /// at the next scheduler boundary.
    pub fn generate_smi(&mut self, value: u8) {
        if value == ACPI_ENABLE {
            self.pmcntrl |= PmControl::SCI_EN.bits();
        } else if value == ACPI_DISABLE {
            self.pmcntrl &= !PmControl::SCI_EN.bits();
        }

        if (self.pci_conf[0x5b] & 0x02) != 0 {
            self.smi_request_pending = true;
        }
    }

    // ─── I/O Port Handlers ───────────────────────────────────────────────

    /// Read from PM or SMBus register space.
    /// Bochs: read_handler() / read() (acpi.cc)
    pub fn read(&mut self, address: u16, io_len: u8, icount: u64) -> u32 {
        let mut value: u32 = 0xFFFF_FFFF;

        if self.pm_base != 0 && (address as u32 & 0xFFC0) == self.pm_base {
            // PM register space — check if PM decode is enabled (PCI config 0x80 bit 0)
            // Bochs acpi.cc
            if (self.pci_conf[0x80] & 0x01) == 0 {
                return value;
            }
            let reg = (address as u32 & 0x3F) as u8;
            match reg {
                // PM1 Status (acpi.cc)
                0x00 => {
                    value = self.get_pmsts(icount) as u32;
                }
                // PM1 Enable (acpi.cc)
                0x02 => {
                    value = self.pmen as u32;
                }
                // PM1 Control (acpi.cc)
                0x04 => {
                    value = self.pmcntrl as u32;
                }
                // PM Timer (acpi.cc)
                0x08 => {
                    value = self.get_pmtmr(icount);
                }
                // Generic PM registers (acpi.cc)
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
            tracing::trace!(
                "ACPI PM read reg={:#04x} value={:#010x} len={}",
                reg,
                value,
                io_len
            );
        } else if self.sm_base != 0 && (address as u32 & 0xFFF0) == self.sm_base {
            // SMBus register space — check decode enable
            // Bochs acpi.cc
            if (self.pci_conf[0x04] & 0x01) == 0 && (self.pci_conf[0xD2] & 0x01) == 0 {
                return value;
            }
            let reg = (address as u32 & 0x0F) as u8;
            match reg {
                // SMBus status (acpi.cc)
                0x00 => value = self.smbus.stat as u32,
                // SMBus control (acpi.cc) — reading resets block index
                0x02 => {
                    self.smbus.index = 0;
                    value = (self.smbus.ctl & 0x1F) as u32;
                }
                // SMBus command (acpi.cc)
                0x03 => value = self.smbus.cmd as u32,
                // SMBus address (acpi.cc)
                0x04 => value = self.smbus.addr as u32,
                // SMBus data0 (acpi.cc)
                0x05 => value = self.smbus.data0 as u32,
                // SMBus data1 (acpi.cc)
                0x06 => value = self.smbus.data1 as u32,
                // SMBus block data (acpi.cc)
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
                    tracing::trace!("ACPI SMBus read reg={:#04x} not implemented", reg);
                }
            }
            tracing::trace!("ACPI SMBus read reg={:#04x} value={:#010x}", reg, value);
        }

        value
    }

    /// Write to PM or SMBus register space.
    /// Bochs: write_handler() / write() (acpi.cc)
    pub fn write(&mut self, address: u16, value: u32, io_len: u8, icount: u64) {
        if self.pm_base != 0 && (address as u32 & 0xFFC0) == self.pm_base {
            // PM register space
            if (self.pci_conf[0x80] & 0x01) == 0 {
                return;
            }
            let reg = (address as u32 & 0x3F) as u8;
            tracing::trace!(
                "ACPI PM write reg={:#04x} value={:#010x} len={}",
                reg,
                value,
                io_len
            );
            match reg {
                // PM1 Status — write-1-to-clear (acpi.cc)
                0x00 => {
                    let pmsts = self.get_pmsts(icount);
                    // If clearing TMROF_STS, recompute next overflow time
                    if pmsts & (value as u16) & PmStatus::TMROF_STS.bits() != 0 {
                        let d = muldiv64(self.live_time_usec(icount), PM_FREQ as u32, 1_000_000);
                        self.tmr_overflow_time = (d + 0x80_0000) & !0x7F_FFFF;
                    }
                    self.pmsts &= !(value as u16);
                    self.pm_update_sci(icount);
                }
                // PM1 Enable (acpi.cc)
                0x02 => {
                    self.pmen = value as u16;
                    self.pm_update_sci(icount);
                }
                // PM1 Control (acpi.cc)
                0x04 => {
                    self.pmcntrl = (value as u16) & !PmControl::SUS_EN.bits();
                    if (value as u16) & PmControl::SUS_EN.bits() != 0 {
                        let sus_typ = (value >> 10) & 7;
                        match sus_typ {
                            0 => {
                                // Soft power off (acpi.cc)
                                tracing::debug!("ACPI: soft power off requested");
                            }
                            1 => {
                                // Suspend to RAM (acpi.cc)
                                tracing::debug!("ACPI: suspend to RAM requested");
                                self.pmsts |=
                                    PmStatus::RSM_STS.bits() | PmStatus::PWRBTN_STS.bits();
                            }
                            _ => {}
                        }
                    }
                    self.pm_update_sci(icount);
                }
                // Write-ignored registers (acpi.cc)
                0x0C | 0x0D | 0x14 | 0x15 | 0x18 | 0x19 | 0x1C | 0x1D | 0x1E | 0x1F | 0x30
                | 0x31 | 0x32 => {}
                // Generic PM registers (acpi.cc)
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
            tracing::trace!("ACPI SMBus write reg={:#04x} value={:#04x}", reg, value);
            match reg {
                // SMBus status — clear on write (acpi.cc)
                0x00 => {
                    self.smbus.stat = 0;
                    self.smbus.index = 0;
                }
                // SMBus control (acpi.cc)
                0x02 => {
                    self.smbus.ctl = 0;
                    // Bochs acpi.cc also has "TODO: execute SMBus command" —
                    // SMBus transaction execution is unimplemented in Bochs itself.
                }
                // SMBus command (acpi.cc)
                0x03 => self.smbus.cmd = 0,
                // SMBus address (acpi.cc)
                0x04 => self.smbus.addr = 0,
                // SMBus data0 (acpi.cc)
                0x05 => self.smbus.data0 = 0,
                // SMBus data1 (acpi.cc)
                0x06 => self.smbus.data1 = 0,
                // SMBus block data (acpi.cc)
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
                    tracing::trace!("ACPI SMBus write reg={:#04x} not implemented", reg);
                }
            }
        } else {
            // Debug port (0xB044) — Bochs acpi.cc
            tracing::trace!("ACPI DBG: {:#010x}", value);
        }
    }

    // ─── PCI Configuration Space ─────────────────────────────────────────

    /// Write to PCI configuration space.
    /// Bochs: pci_write_handler() (acpi.cc)
    ///
    /// Returns (pm_base_changed, sm_base_changed) to signal that the emulator
    /// should re-register I/O ports.
    pub fn pci_write(&mut self, address: u8, value: u32, io_len: u8) -> (bool, bool) {
        let mut pm_base_change = false;
        let mut sm_base_change = false;

        // Addresses 0x10-0x33 are ignored (BAR region) — acpi.cc
        if (0x10..0x34).contains(&address) {
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
                // Command register (acpi.cc)
                0x04 => {
                    self.pci_conf[addr] = (value8 & 0xFE) | (value8 & 0x01);
                }
                // Status lo-byte — write disallowed (acpi.cc)
                0x06 => {}
                // PM base 0x40 (acpi.cc)
                0x40 => {
                    let v = (value8 & 0xC0) | 0x01;
                    pm_base_change |= v != oldval;
                    self.pci_conf[addr] = v;
                }
                // PM base 0x41-0x43 (acpi.cc)
                0x41..=0x43 => {
                    pm_base_change |= value8 != oldval;
                    self.pci_conf[addr] = value8;
                }
                // SM base 0x90 (acpi.cc)
                0x90 => {
                    let v = (value8 & 0xF0) | 0x01;
                    sm_base_change |= v != oldval;
                    self.pci_conf[addr] = v;
                }
                // SM base 0x91-0x93 (acpi.cc, fall-through to default)
                0x91..=0x93 => {
                    sm_base_change |= value8 != oldval;
                    self.pci_conf[addr] = value8;
                }
                // Default: store value (acpi.cc)
                _ => {
                    self.pci_conf[addr] = value8;
                }
            }
        }

        // Update base addresses if changed (acpi.cc)
        if pm_base_change {
            let new_base = u32::from_le_bytes([
                self.pci_conf[0x40],
                self.pci_conf[0x41],
                self.pci_conf[0x42],
                self.pci_conf[0x43],
            ]) & 0xFFC0; // Mask to 64-port alignment
            self.pm_base = new_base;
            tracing::debug!("ACPI: new PM base address: {:#06x}", self.pm_base);
        }

        if sm_base_change {
            let new_base = u32::from_le_bytes([
                self.pci_conf[0x90],
                self.pci_conf[0x91],
                self.pci_conf[0x92],
                self.pci_conf[0x93],
            ]) & 0xFFF0; // Mask to 16-port alignment
            self.sm_base = new_base;
            tracing::debug!("ACPI: new SM base address: {:#06x}", self.sm_base);
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
/// Ported from QEMU/Bochs: muldiv64() (acpi.cc)
fn muldiv64(a: u64, b: u32, c: u32) -> u64 {
    let a_lo = a as u32 as u64;
    let a_hi = a >> 32;

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
        assert_eq!(acpi.get_pmtmr(0), 0);
        // After 1 second (1,000,000 usec), timer should be ~3,579,545
        acpi.time_usec = 1_000_000;
        let tmr = acpi.get_pmtmr(0);
        assert_eq!(tmr, PM_FREQ as u32 & 0xFF_FFFF);
        // After ~2.34 seconds, should wrap (24-bit)
        acpi.time_usec = 5_000_000; // ~5 seconds
        let tmr = acpi.get_pmtmr(0);
        assert!(tmr < 0xFF_FFFF); // Must have wrapped
    }

    #[test]
    fn test_pm_timer_read_uses_live_icount_delta() {
        let mut acpi = BxAcpiCtrl::new();
        acpi.pm_base = 0xB000;
        acpi.pci_conf[0x80] = 0x01; // Enable PM decode
        acpi.init_icount_sync(1_000, 1_000_000);

        let pm_addr = acpi.pm_base as u16 + 0x08;
        assert_eq!(acpi.read(pm_addr, 4, 1_000), 0);
        assert_eq!(acpi.read(pm_addr, 4, 1_001_000), PM_FREQ as u32 & 0xFF_FFFF);
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
        acpi.write(pm_addr, PmStatus::PWRBTN_STS.bits() as u32, 2, 0);

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
            acpi.write(sm_addr + 0x07, i, 1, 0);
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
        assert_eq!(acpi.get_pmsts(0) & PmStatus::TMROF_STS.bits(), 0);

        // Advance past overflow point
        // 100 PM ticks = 100 / 3_579_545 seconds = ~28 usec
        acpi.time_usec = 100; // ~358 PM ticks at 3.58 MHz
        let pmsts = acpi.get_pmsts(0);
        assert_ne!(pmsts & PmStatus::TMROF_STS.bits(), 0);
    }

    #[test]
    fn generate_smi_delivers_only_with_apmc_en() {
        // Bochs acpi.cc generate_smi: SCI_EN toggles regardless; the SMI is
        // delivered to CPU 0 only when pci_conf[0x5b] bit 1 (APMC_EN, set by
        // the BIOS via the SMI-control dword at config 0x58) is enabled.
        let mut acpi = BxAcpiCtrl::new();

        acpi.generate_smi(0x00);
        assert!(!acpi.smi_request_pending, "no SMI without APMC_EN");

        acpi.generate_smi(ACPI_ENABLE);
        assert_ne!(acpi.pmcntrl & PmControl::SCI_EN.bits(), 0, "SCI_EN set");
        assert!(!acpi.smi_request_pending);

        // The BIOS smm_init: pci_config_writel(d, 0x58, value | (1 << 25)).
        acpi.pci_write(0x58, 1 << 25, 4);
        acpi.generate_smi(0x00);
        assert!(acpi.smi_request_pending, "APMC_EN set: SMI delivered");

        acpi.smi_request_pending = false;
        acpi.generate_smi(ACPI_DISABLE);
        assert_eq!(acpi.pmcntrl & PmControl::SCI_EN.bits(), 0, "SCI_EN cleared");
        assert!(acpi.smi_request_pending, "delivery is independent of the command value");
    }

    #[test]
    fn acpi_tmrof_enable_arms_exact_overflow_sci() {
        let mut acpi = BxAcpiCtrl::new();
        acpi.pm_base = 0xB000;
        acpi.pci_conf[0x80] = 0x01;
        acpi.init_icount_sync(0, 1_000_000);

        acpi.write(
            acpi.pm_base as u16 + 0x02,
            PmEnable::TMROF_EN.bits() as u32,
            2,
            0,
        );
        let delay = acpi.overflow_delay_usec(0).expect("TMROF should arm");
        assert_eq!(
            delay,
            muldiv64(0xFF_FFFF, 1_000_000, PM_FREQ as u32) + 1
        );
        assert!(!acpi.irq9_level);

        assert_eq!(acpi.overflow_timer(delay), None);
        assert_ne!(acpi.pmsts & PmStatus::TMROF_STS.bits(), 0);
        assert!(acpi.irq9_level);
    }

    #[cfg(feature = "std")]
    #[test]
    fn realtime_acpi_overflow_rearms_when_owner_arrives_early() {
        let mut acpi = BxAcpiCtrl::new();
        acpi.enable_realtime_sync();
        acpi.pmen = PmEnable::TMROF_EN.bits();

        let predicted = acpi
            .overflow_delay_usec(0)
            .expect("future host overflow should be scheduled");
        let overflow_time = acpi.tmr_overflow_time;
        let pmsts = acpi.pmsts;

        let rearmed = acpi
            .overflow_timer(0)
            .expect("early callback should rearm from host time");
        assert!(rearmed > 0);
        assert!(rearmed <= predicted);
        assert_eq!(acpi.tmr_overflow_time, overflow_time);
        assert_eq!(acpi.pmsts, pmsts);
        assert!(!acpi.irq9_level);
    }

    #[cfg(feature = "std")]
    #[test]
    fn pm_timer_realtime_sync_advances_without_icount_progress() {
        let mut acpi = BxAcpiCtrl::new();
        acpi.pm_base = 0xB000;
        acpi.pci_conf[0x80] = 0x01; // Enable PM decode
        acpi.init_icount_sync(1_000, 300_000_000);
        acpi.enable_realtime_sync();

        let pm_addr = acpi.pm_base as u16 + 0x08;
        let before = acpi.read(pm_addr, 4, 1_000);
        std::thread::sleep(std::time::Duration::from_millis(10));
        let after = acpi.read(pm_addr, 4, 1_000);

        assert!(
            after > before,
            "ACPI PM timer should advance from host realtime even when icount is unchanged"
        );
    }
}
