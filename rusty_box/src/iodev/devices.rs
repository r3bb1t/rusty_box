#![allow(unused_assignments, dead_code)]
//! Device Initialization and Management
//!
//! This module implements the device initialization sequence from Bochs,
//! including the Port 0x92 System Control handler for A20 line control.
//!
//! ## Device Architecture
//!
//! The device system mirrors Bochs' plugin architecture:
//! - Core devices (PIC, PIT, DMA, CMOS, Keyboard) are always present
//! - Standard devices (HardDrive, Floppy, VGA) are configurable
//! - Each device registers its own I/O port handlers

#[cfg(feature = "alloc")]
use alloc::format;
#[cfg(feature = "alloc")]
use alloc::string::String;

use crate::{
    cpu::ResetReason,
    memory::{BxMemC, CpuTlbPin},
    pc_system::BxPcSystemC,
    Result,
};
#[cfg(feature = "std")]
use std::io::{self, Error, ErrorKind, Read, Write};

#[cfg(feature = "std")]
use crate::snapshot::{checked_snapshot_len_add, SnapshotReader, SnapshotWriteExt};


use super::acpi::BxAcpiCtrl;
use super::cmos::{BxCmosC, CMOS_ADDR, CMOS_DATA};
use super::dma::BxDmaC;
use super::fw_cfg::BxFwCfg;
use super::harddrv::BxHardDriveC;
use super::ioapic::BxIoApic;
use super::keyboard::{BxKeyboardC, KBD_DATA_PORT, KBD_STATUS_PORT};
use super::pci::BxPciBridge;
use super::pci2isa::BxPiix3;
use super::pci_ide::BxPciIde;
use super::pic::{BxPicC, PIC_MASTER_CMD, PIC_MASTER_DATA, PIC_SLAVE_CMD, PIC_SLAVE_DATA};
use super::pit::{
    BxPitC, PIT_CONTROL, PIT_COUNTER0, PIT_COUNTER1, PIT_COUNTER2, PIT_SYSTEM_CONTROL_B,
};
use super::serial::BxSerialC;
use super::vga::BxVgaC;
use super::BxDevicesC;
use super::DeviceId;

/// Port 0x92 - System Control Port
/// Bit 0: Fast A20 gate control (1 = A20 enabled)
/// Bit 1: Fast reset (writing 1 triggers CPU reset)
const PORT_92H: u16 = 0x0092;

/// Port 92h state storage
#[derive(Debug, Default, Clone)]
pub struct Port92State {
    /// Current value of port 92h
    pub(crate) value: u8,
}

/// Fetch a BM-DMA PRD entry (physical address, raw size dword) from guest
/// RAM. Reads past a hole or the configured guest length are zero-filled.
fn read_bmdma_prd(
    mem: &mut BxMemC<'_>,
    pins: &[CpuTlbPin],
    prd_addr: u32,
) -> (u32, u32) {
    let mut raw = [0u8; 8];
    match mem.read_ram(pins, prd_addr as u64, &mut raw) {
        Ok(_) => {}
        Err(error) => tracing::error!("BM-DMA PRD read at {prd_addr:#x} failed: {error:?}"),
    }
    (
        u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
        u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]),
    )
}

/// Bochs devices.cc `bx_pci_device_c::pci_write_handler_common` — config-space
/// bytes that are always read-only, regardless of which device is targeted:
/// vendor/device ID (0x00-0x03), revision + class code (0x08-0x0B), header
/// type (0x0E), and interrupt pin (0x3D). Applied uniformly BEFORE any
/// device's own `pci_write` runs. BARs, command/status, cache-line/latency,
/// expansion ROM, and interrupt line are untouched by this filter.
fn pci_common_reg_is_readonly(addr: u8) -> bool {
    matches!(addr, 0x00..=0x03 | 0x08..=0x0B | 0x0E | 0x3D)
}

/// Bochs devices.cc `bx_pci_device_c::pci_write_handler_common` — gate a raw
/// PCI config-space write BEFORE it reaches the device-specific `pci_write`.
/// Bochs checks only the write's STARTING offset (`address`, i.e. `reg_addr`
/// here); there is no per-byte mid-span filtering, so a multi-byte write
/// that starts on a writable register reaches the device unfiltered even if
/// it spills into a read-only byte (e.g. a dword write starting at 0x0C
/// still overwrites the read-only header-type byte at 0x0E — an obscure
/// upstream quirk, not a bug to "fix").
///
/// Returns `None` if the whole write must be dropped (starting offset is in
/// `pci_common_reg_is_readonly`). Returns `Some((addr, value, len))` with the
/// write to forward to the device otherwise; for a write starting at 0x3C
/// (interrupt line) Bochs stores only that single byte
/// (`pci_conf[0x3c] = (Bit8u)value`) regardless of `io_len`, so the returned
/// write is clamped to one byte.
fn pci_write_common_gate(reg_addr: u8, value: u32, io_len: u8) -> Option<(u8, u32, u8)> {
    if pci_common_reg_is_readonly(reg_addr) {
        None
    } else if reg_addr == 0x3C {
        Some((reg_addr, value & 0xFF, 1))
    } else {
        Some((reg_addr, value, io_len))
    }
}

/// Mapping effects committed by one scheduler-boundary pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MachineBoundaryEffects {
    pub(crate) memory_mapping_changed: bool,
}


/// Desired PLATFORM effects and mapping targets decoded from `DeviceManager`.
///
/// The decoder deliberately leaves all live registrations and committed bases
/// untouched. The machine-level restore path cross-checks these targets against
/// the PCI, ACPI, and VGA sections, then relocates from captured live topology
/// before committing them.
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PlatformSnapshotRestore {
    pub(crate) pci_conf_addr: u32,
    pub(crate) port92_a20_gate: bool,
    pub(crate) port92_a20_change_pending: bool,
    pub(crate) port92_reset_request: Option<ResetReason>,
    pub(crate) pci_ide_bar4_needs_reregister: bool,
    pub(crate) acpi_pm_needs_reregister: bool,
    pub(crate) acpi_sm_needs_reregister: bool,
    pub(crate) pam_needs_update: bool,
    pub(crate) smram_needs_update: bool,
    pub(crate) bios_write_needs_update: bool,
    pub(crate) vga_bar_needs_reregister: bool,
    pub(crate) committed_bmdma_ports_base: u16,
    pub(crate) committed_pm_ports_base: u16,
    pub(crate) committed_sm_ports_base: u16,
    pub(crate) desired_bmdma_base: u32,
    pub(crate) desired_pm_base: u32,
    pub(crate) desired_sm_base: u32,
    pub(crate) committed_vga_lfb_base: u32,
    pub(crate) committed_vga_mmio_base: u32,
    pub(crate) desired_vga_lfb_base: u32,
    pub(crate) desired_vga_mmio_base: u32,
}
/// Unified Device Manager
///
/// Holds all hardware devices and manages their initialization,
/// reset, and I/O port registration. This mirrors Bochs' `bx_devices_c`.
#[derive(Debug)]
pub struct DeviceManager {
    /// 8259 PIC (Programmable Interrupt Controller)
    pub(crate) pic: BxPicC,
    /// 8254 PIT (Programmable Interval Timer)
    pub(crate) pit: BxPitC,
    /// CMOS/RTC
    pub(crate) cmos: BxCmosC,
    /// 8237 DMA Controller
    pub(crate) dma: BxDmaC,
    /// 8042 Keyboard Controller
    pub(crate) keyboard: BxKeyboardC,
    /// High Precision Event Timer (Bochs iodev/hpet.cc)
    pub(crate) hpet: super::hpet::BxHpetC,
    /// ATA/IDE Hard Drive Controller
    pub(crate) harddrv: BxHardDriveC,
    /// VGA Display Controller
    pub(crate) vga: BxVgaC,
    /// I/O APIC (82093AA) — interrupt routing for APIC-based systems
    /// Bochs: `bx_ioapic_c *pluginIOAPIC` (iodev/iodev.h)
    pub(crate) ioapic: BxIoApic,
    /// PIIX4 ACPI Power Management controller
    /// Bochs: `bx_acpi_ctrl_c *pluginACPIController` (iodev/iodev.h)
    pub(crate) acpi: BxAcpiCtrl,
    /// i440FX PCI Host Bridge (bus 0, dev 0, func 0)
    /// Bochs: `bx_pci_bridge_c *pluginPciBridge` (iodev/iodev.h)
    pub(crate) pci_bridge: BxPciBridge,
    /// PIIX3 PCI-to-ISA Bridge (bus 0, dev 1, func 0)
    /// Bochs: `bx_piix3_c *pluginPci2IsaBridge` (iodev/iodev.h)
    pub(crate) pci2isa: BxPiix3,
    /// PIIX3 PCI IDE Controller (bus 0, dev 1, func 1)
    /// Bochs: `bx_pci_ide_c *pluginPciIdeController` (iodev/iodev.h)
    pub(crate) pci_ide: BxPciIde,
    /// 16550 UART Serial Port Controller (COM1-COM4)
    /// Bochs: `bx_serial_c *pluginSerial` (iodev/iodev.h)
    pub(crate) serial: BxSerialC,
    /// QEMU fw_cfg Firmware Configuration Device
    /// Bochs: `bx_fw_cfg_c *theFwCfgDevice` (iodev/fw_cfg.h)
    pub(crate) fw_cfg: BxFwCfg,
    /// PCI configuration address register (shadow copy for handler dispatch)
    /// Bochs: bx_devices_c::pci_conf_addr (devices.cc)
    pub(crate) pci_conf_addr: u32,
    /// Deferred: PCI IDE BAR4 changed, needs BM-DMA port re-registration
    pub(crate) pci_ide_bar4_needs_reregister: bool,
    /// Deferred: ACPI PM base changed, needs port re-registration
    pub(crate) acpi_pm_needs_reregister: bool,
    /// Deferred: ACPI SMBus base changed, needs port re-registration
    pub(crate) acpi_sm_needs_reregister: bool,
    /// Deferred: PAM registers changed, needs memory type update
    pub pam_needs_update: bool,
    /// Deferred: the SMRAM control register (0x72) was written, needs SMRAM
    /// routing re-applied to memory (Bochs pci.cc smram_control's
    /// mem->enable_smram()/disable_smram() calls).
    pub smram_needs_update: bool,
    /// Deferred: the PIIX3 XBCS register (0x4E) changed a bit affecting
    /// BIOS-ROM write-enable state, needs re-applied to memory (Bochs
    /// pci2isa.cc pci_write_handler case 0x4e's
    /// DEV_mem_set_bios_write()/DEV_mem_set_bios_rom_access() calls).
    pub bios_write_needs_update: bool,
    /// Deferred: a VGA PCI BAR (LFB or MMIO) changed, needs memory-handler
    /// (re)registration at the new base.
    pub(crate) vga_bar_needs_reregister: bool,
    /// Diagnostic: PIT IRQ0 rising edges applied to the PIC
    pub diag_pit_fires: u64,
    /// Diagnostic: raise_irq(0) latched (irq_in was 0)
    pub diag_irq0_latched: u64,
    /// Diagnostic: raise_irq(0) skipped (irq_in was already 1)
    pub diag_irq0_already_high: u64,
    /// Diagnostic: iac() calls
    pub diag_iac_count: u64,
    /// Diagnostic: iac vector histogram [0..256]
    pub diag_vector_hist: [u32; 256],
    /// Pointer to BxMemC for fw_cfg DMA. Set temporarily during CPU execution.
    pub(crate) mem_ptr: Option<core::ptr::NonNull<BxMemC<'static>>>,
    /// Complete stable TLB-pin slice for fw_cfg DMA allocation/eviction.
    pub(crate) active_tlb_pins: Option<core::ptr::NonNull<CpuTlbPin>>,
    pub(crate) active_tlb_pin_count: usize,
    /// I/O base the BM-DMA ports are currently registered at (0 = none).
    /// Lets a BAR4 move unregister the old range first, matching Bochs
    /// devices.cc pci_write_handler_common BAR remapping.
    pub(crate) bmdma_ports_base: u16,
    /// ACPI PM ports currently registered in the I/O dispatcher (0 = none).
    pub(crate) pm_ports_base: u16,
    /// ACPI SMBus ports currently registered in the I/O dispatcher (0 = none).
    pub(crate) sm_ports_base: u16,
    /// BM-DMA sector staging buffer. Bochs pci_ide.cc timer() hands the drive
    /// a pointer into the channel bounce buffer; here the drive callbacks need
    /// `&mut BxPciIde` (abort/IRQ paths) while that buffer lives inside it, so
    /// sectors stage through this scratch instead. Sized to the largest PRD
    /// chunk (0x10000) so no transfer is ever clamped.
    pub(crate) bmdma_scratch: [u8; 0x10000],
    /// System Control Port (Port 92h) — A20 gate and fast reset
    pub(crate) port92: SystemControlPort,
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceManager {
    /// Set the pre-boot VBE display mode on the VGA controller: raises the DISPI
    /// capability ceiling and seeds the power-on dimensions. Preserved across
    /// guest resets.
    pub fn set_vga_preferred_mode(&mut self, width: u16, height: u16, bpp: u16) {
        self.vga.set_preferred_mode(width, height, bpp);
    }

    /// Whether any I/O-produced machine effect must be applied before the
    /// next guest instruction.
    pub(crate) fn has_pending_machine_boundary(&self) -> bool {
        self.pci_ide_bar4_needs_reregister
            || self.acpi_pm_needs_reregister
            || self.acpi_sm_needs_reregister
            || self.pam_needs_update
            || self.smram_needs_update
            || self.bios_write_needs_update
            || self.vga_bar_needs_reregister
            || self.port92.a20_change_pending
            || self.keyboard.a20_change_pending
            || self.port92.reset_request.is_some()
            || self.keyboard.reset_requested.is_some()
            || self.pci2isa.reset_request.is_some()
            // Bochs acpi.cc apic_bus_deliver_smi is synchronous with the OUT
            // to SMI_CMD; ending the slice here delivers the SMI to CPU 0
            // before the guest's next instruction. Drained by the emulator's
            // service_scheduler_boundary BEFORE the apply/quiesce loop.
            || self.acpi.smi_request_pending
    }

    /// Drain all reset producers, with hardware reset taking precedence over
    /// any software request issued in the same boundary.
    pub(crate) fn take_reset_request(&mut self) -> Option<ResetReason> {
        let port92 = self.port92.reset_request.take();
        let keyboard = self.keyboard.reset_requested.take();
        let pci = self.pci2isa.reset_request.take();
        if matches!(port92, Some(ResetReason::Hardware))
            || matches!(keyboard, Some(ResetReason::Hardware))
            || matches!(pci, Some(ResetReason::Hardware))
        {
            Some(ResetReason::Hardware)
        } else if port92.is_some() || keyboard.is_some() || pci.is_some() {
            Some(ResetReason::Software)
        } else {
            None
        }
    }

    /// Create a new device manager with all devices.
    pub fn new() -> Self {
        Self {
            pic: BxPicC::new(),
            pit: BxPitC::new(),
            cmos: BxCmosC::new(),
            dma: BxDmaC::new(),
            keyboard: BxKeyboardC::new(),
            hpet: super::hpet::BxHpetC::new(),
            harddrv: BxHardDriveC::new(),
            vga: BxVgaC::new(),
            ioapic: BxIoApic::new(),
            acpi: BxAcpiCtrl::new(),
            pci_bridge: BxPciBridge::new(),
            pci2isa: BxPiix3::new(),
            pci_ide: BxPciIde::new(),
            serial: BxSerialC::new(1), // COM1 only
            fw_cfg: BxFwCfg::new(),
            pci_conf_addr: 0,
            pci_ide_bar4_needs_reregister: false,
            acpi_pm_needs_reregister: false,
            acpi_sm_needs_reregister: false,
            pam_needs_update: false,
            smram_needs_update: false,
            bios_write_needs_update: false,
            vga_bar_needs_reregister: false,
            diag_pit_fires: 0,
            diag_irq0_latched: 0,
            diag_irq0_already_high: 0,
            diag_iac_count: 0,
            diag_vector_hist: [0; 256],
            mem_ptr: None,
            active_tlb_pins: None,
            active_tlb_pin_count: 0,
            bmdma_ports_base: 0,
            pm_ports_base: 0,
            sm_ports_base: 0,
            bmdma_scratch: [0; 0x10000],
            port92: SystemControlPort::new(),
        }
    }

    /// Initialize all devices and register I/O handlers
    ///
    /// Matches device loading order from cpp_orig/bochs/iodev/devices.cc:
    /// 1. CMOS (line 250)
    /// 2. DMA (line 251)
    /// 3. PIC (line 252)
    /// 4. PIT (line 253)
    /// 5. VGA (line 254-256)
    /// 6. Keyboard (line 262)
    /// 7. Hard drive (line 275-277)
    pub fn init(&mut self, io: &mut BxDevicesC, mem: &mut BxMemC) -> Result<()> {
        tracing::debug!("Initializing device manager");

        // Initialize each device in original Bochs order
        // 1. CMOS
        self.cmos.init();
        // 2. DMA
        self.dma.init();
        // 3. PIC
        self.pic.init();
        // 4. PIT
        self.pit.init();
        // 5. VGA
        self.vga.init(io, mem)?;
        // 6. Keyboard
        self.keyboard.init();
        // 7. Hard drive
        self.harddrv.init();
        // 8. I/O APIC (Bochs: pluginIOAPIC->init() in devices.cc)
        self.ioapic.init(mem)?;
        // 8b. HPET (Bochs: PLUGTYPE_STANDARD hpet plugin — hpet.cc init()
        // registers the fixed MMIO window; the rombios32 ACPI builder then
        // probes 0xFED00000 for the 0x8086 vendor id).
        {
            use super::hpet::{HPET_BASE, HPET_LEN};
            let device_id = crate::memory::MemoryDeviceId::Hpet(&mut self.hpet as *mut _);
            mem.register_memory_handlers(device_id, HPET_BASE, HPET_BASE + HPET_LEN - 1)?;
        }
        // 9. ACPI Power Management (Bochs: pluginACPIController->init() in devices.cc)
        self.acpi.reset();
        // 10. PCI bus devices (Bochs: pluginPciBridge->init(), pluginPci2IsaBridge->init(), etc.)
        {
            self.pci_bridge.reset();
            self.pci2isa.reset();
            self.pci_ide.reset();
        }

        // Register I/O handlers for each device (order doesn't matter for handlers)
        self.register_cmos_handlers(io);
        self.register_dma_handlers(io);
        self.register_pic_handlers(io);
        self.register_pit_handlers(io);
        self.register_keyboard_handlers(io);
        self.register_harddrv_handlers(io);
        self.register_serial_handlers(io);
        self.register_acpi_handlers(io);
        self.register_pci_handlers(io);
        self.register_fw_cfg_handlers(io);
        // Register BM-DMA ports if BAR4 is pre-configured (for direct boot without BIOS)
        if self.pci_ide.bmdma_base > 0 {
            self.register_pci_ide_bmdma_ports(io);
        }

        tracing::debug!("Device manager initialization complete");
        Ok(())
    }

    /// Reset all devices
    pub fn reset(&mut self, reset_type: ResetReason) -> Result<()> {
        tracing::debug!("Device manager reset: {:?}", reset_type);

        self.pic.reset();
        // Deliberate no-op: Bochs pit82c54.cc reset(type) is empty — the
        // PIT counters keep their programming across a guest reset.
        self.pit.reset();
        self.cmos.reset();
        self.dma.reset();
        self.keyboard.reset();
        self.harddrv.reset();
        self.vga.reset();
        self.serial.reset();
        self.ioapic.reset();
        // Bochs hpet.cc reset(): comparators stop, state clears, and the
        // PIT/RTC pins re-enable (queued; the emulator drains after reset).
        self.hpet.reset();
        self.acpi.reset();
        self.fw_cfg.reset();
        {
            self.pci_bridge.reset();
            self.pci2isa.reset();
            self.pci_ide.reset();
            self.pci_conf_addr = 0;
            self.pci_ide_bar4_needs_reregister =
                self.bmdma_ports_base != self.pci_ide.bmdma_base as u16;
            self.acpi_pm_needs_reregister =
                self.pm_ports_base != self.acpi.pm_base as u16;
            self.acpi_sm_needs_reregister =
                self.sm_ports_base != self.acpi.sm_base as u16;
            self.vga_bar_needs_reregister = self.vga.peek_pending_lfb_relocate().is_some()
                || self.vga.peek_pending_mmio_relocate().is_some();
            // Re-apply the reset SMRAM/XBCS state synchronously before the
            // guest resumes. Hardware reset may already have disabled SMRAM;
            // the operation is idempotent.
            self.smram_needs_update = true;
            self.bios_write_needs_update = true;
            // Bochs pci.cc bx_pci_bridge_c::reset() re-applies memory type
            // for every PAM area directly (DEV_mem_set_memory_type loop)
            // right after zeroing the PAM config bytes, so the shadow-RAM
            // state matches the reset PAM config immediately. rusty_box
            // can't touch memory here (BxMemC isn't available to
            // DeviceManager::reset), so defer it the same way pci_write's
            // PAM branch does; drained by the next shared machine boundary.
            self.pam_needs_update = true;
        }

        Ok(())
    }

    /// Register PIC I/O handlers.
    /// Note: ELCR1/ELCR2 (0x4D0/0x4D1) are NOT PIC ports in Bochs — they
    /// belong to the PIIX3 PCI-to-ISA bridge (pci2isa.cc), which forwards
    /// mode changes to `BxPicC::set_mode()`. See `register_pci_handlers`.
    fn register_pic_handlers(&mut self, io: &mut BxDevicesC) {
        for port in [
            PIC_MASTER_CMD,
            PIC_MASTER_DATA,
            PIC_SLAVE_CMD,
            PIC_SLAVE_DATA,
        ] {
            io.register_io_handler(DeviceId::Pic, port, "8259 PIC", 0x1);
        }
    }

    /// Register PIT I/O handlers
    fn register_pit_handlers(&mut self, io: &mut BxDevicesC) {
        // Bochs pit.cc bx_pit_c::init registers 0x40-0x43 AND 0x61 (System
        // Control Port B) for the PIT.
        for port in [
            PIT_COUNTER0,
            PIT_COUNTER1,
            PIT_COUNTER2,
            PIT_CONTROL,
            PIT_SYSTEM_CONTROL_B,
        ] {
            io.register_io_handler(DeviceId::Pit, port, "8254 PIT", 0x1);
        }
    }

    /// Register CMOS I/O handlers
    fn register_cmos_handlers(&mut self, io: &mut BxDevicesC) {
        io.register_io_handler(DeviceId::Cmos, CMOS_ADDR, "CMOS Address", 0x1);
        io.register_io_handler(DeviceId::Cmos, CMOS_DATA, "CMOS Data", 0x1);
        // Bochs cmos.cc — extended CMOS RAM ports (addresses 0x80-0xFF)
        io.register_io_handler(DeviceId::Cmos, 0x0072, "Ext CMOS RAM", 0x1);
        io.register_io_handler(DeviceId::Cmos, 0x0073, "Ext CMOS RAM", 0x1);
    }

    /// Register DMA I/O handlers (Bochs dma.cc)
    fn register_dma_handlers(&mut self, io: &mut BxDevicesC) {
        // DMA1 ports 0x0000-0x000F (Bochs dma.cc)
        for port in 0x0000..=0x000F_u16 {
            io.register_io_handler(DeviceId::Dma, port, "DMA controller", 0x1);
        }

        // Page registers 0x0080-0x008F (Bochs dma.cc)
        for port in 0x0080..=0x008F_u16 {
            io.register_io_handler(DeviceId::Dma, port, "DMA controller", 0x1);
        }

        // DMA2 ports 0x00C0-0x00DE, step 2 (Bochs dma.cc)
        let mut port = 0x00C0_u16;
        while port <= 0x00DE {
            io.register_io_handler(DeviceId::Dma, port, "DMA controller", 0x1);
            port += 2;
        }
    }

    /// Register Keyboard I/O handlers
    fn register_keyboard_handlers(&mut self, io: &mut BxDevicesC) {
        // Issue #610 — Darwin boot fix: allow 1/2/4-byte reads (width 7), write stays 1-byte
        io.register_io_read_handler(DeviceId::Keyboard, KBD_DATA_PORT, "Keyboard Data", 0x7);
        io.register_io_write_handler(DeviceId::Keyboard, KBD_DATA_PORT, "Keyboard Data", 0x1);
        io.register_io_read_handler(
            DeviceId::Keyboard,
            KBD_STATUS_PORT,
            "Keyboard Status/Command",
            0x7,
        );
        io.register_io_write_handler(
            DeviceId::Keyboard,
            KBD_STATUS_PORT,
            "Keyboard Status/Command",
            0x1,
        );
        // Port 0x61 (System Control B) belongs to the PIT — Bochs pit.cc
        // bx_pit_c::init registers it; see register_pit_handlers.
    }

    /// Register Hard Drive I/O handlers
    fn register_harddrv_handlers(&mut self, io: &mut BxDevicesC) {
        // Primary ATA (0x1F0-0x1F7, 0x3F6)
        for port in 0x1F0..=0x1F7_u16 {
            io.register_io_handler(DeviceId::HardDrive, port, "ATA Primary", 0x7);
        }
        io.register_io_handler(DeviceId::HardDrive, 0x3F6, "ATA Primary Control", 0x1);

        // Secondary ATA (0x170-0x177, 0x376)
        for port in 0x170..=0x177_u16 {
            io.register_io_handler(DeviceId::HardDrive, port, "ATA Secondary", 0x7);
        }
        io.register_io_handler(DeviceId::HardDrive, 0x376, "ATA Secondary Control", 0x1);
    }

    /// Register Serial Port I/O handlers
    fn register_serial_handlers(&mut self, io: &mut BxDevicesC) {
        // COM1: 0x3F8-0x3FF (8 registers)
        for port in 0x3F8..=0x3FF_u16 {
            io.register_io_handler(DeviceId::Serial, port, "16550 COM1", 0x1);
        }
    }

    /// Register ACPI I/O handlers.
    /// Static ports: SMI command (0xB2), ACPI debug (0xB044).
    /// Dynamic ports (PM/SM base) are re-registered when PCI config changes.
    fn register_acpi_handlers(&mut self, io: &mut BxDevicesC) {
        // SMI command port (0xB2) — Bochs acpi.cc
        io.register_io_write_handler(DeviceId::Acpi, 0x00B2, "ACPI SMI Command", 0x1);

        // ACPI debug port (0xB044) — Bochs acpi.cc
        io.register_io_handler(DeviceId::Acpi, 0xB044, "ACPI Debug", 0x7);
    }

    /// Register ACPI PM I/O port range (called when PM base changes via PCI config).
    pub fn register_acpi_pm_ports(&mut self, io: &mut BxDevicesC) {
        let base = self.acpi.pm_base as u16;
        if base == 0 {
            return;
        }
        // Register 64 ports at PM base — Bochs acpi.cc
        for offset in 0..64u16 {
            let mask = self.acpi.pm_io_mask(offset as u8);
            if mask != 0 {
                io.register_io_handler(DeviceId::Acpi, base + offset, "ACPI PM", mask);
            }
        }
        self.acpi.pm_ports_registered = true;
        self.pm_ports_base = base;
    }

    /// Register ACPI SMBus I/O port range (called when SM base changes via PCI config).
    pub fn register_acpi_sm_ports(&mut self, io: &mut BxDevicesC) {
        let base = self.acpi.sm_base as u16;
        if base == 0 {
            return;
        }
        // Register 16 ports at SM base — Bochs acpi.cc
        for offset in 0..16u16 {
            let mask = self.acpi.sm_io_mask(offset as u8);
            if mask != 0 {
                io.register_io_handler(DeviceId::Acpi, base + offset, "ACPI SMBus", mask);
            }
        }
        self.acpi.sm_ports_registered = true;
        self.sm_ports_base = base;
    }

    /// Register PCI bus I/O handlers.
    /// Ports: 0xCF8 (config address), 0xCFC-0xCFF (config data),
    /// PIIX3 I/O ports (ELCR, CPU reset), and PCI IDE BM-DMA ports.
    /// Bochs: devices.cc (PCI bridge init order)
    fn register_pci_handlers(&mut self, io: &mut BxDevicesC) {
        // PCI config address register (0xCF8) — 4-byte write only
        io.register_io_handler(
            DeviceId::Pci,
            super::pci::PCI_CONFIG_ADDR,
            "PCI Config Addr",
            0x4,
        );

        // PCI config data register (0xCFC-0xCFF) — 1/2/4-byte
        for port in 0x0CFC..=0x0CFF_u16 {
            io.register_io_handler(DeviceId::Pci, port, "PCI Config Data", 0x7);
        }

        // PIIX3 I/O ports: APM (0xB2-0xB3), ELCR (0x4D0-0x4D1), CPU reset
        // (0xCF9). Bochs pci2isa.cc init(): the APM command port (0xB2) WRITE
        // handler is registered with mask 3 so the 16-bit `outw 0xB2, ax`
        // idiom reaches the handler (apms loads from the high byte); all
        // other ports and the 0xB2 read side are 1-byte.
        io.register_io_read_handler(DeviceId::Pci, super::pci2isa::APM_CMD_PORT, "PIIX3", 0x1);
        io.register_io_write_handler(DeviceId::Pci, super::pci2isa::APM_CMD_PORT, "PIIX3", 0x3);
        for port in [
            super::pci2isa::APM_STS_PORT,
            super::pci2isa::ELCR1_PORT,
            super::pci2isa::ELCR2_PORT,
            super::pci2isa::PCI_RESET_PORT,
        ] {
            io.register_io_handler(DeviceId::Pci, port, "PIIX3", 0x1);
        }
    }

    /// Register fw_cfg I/O handlers.
    /// Ports: 0x510 (selector), 0x511 (data), 0x514-0x51B (DMA).
    fn register_fw_cfg_handlers(&mut self, io: &mut BxDevicesC) {
        // Selector port: 1-byte read, 2-byte write
        io.register_io_read_handler(DeviceId::FwCfg, 0x510, "fw_cfg selector", 0x1);
        io.register_io_write_handler(DeviceId::FwCfg, 0x510, "fw_cfg selector", 0x3);
        // Data port: 1-byte read and write
        io.register_io_read_handler(DeviceId::FwCfg, 0x511, "fw_cfg data", 0x1);
        io.register_io_write_handler(DeviceId::FwCfg, 0x511, "fw_cfg data", 0x3);
        // DMA ports: 0x514-0x51B, 1/2/4-byte read and write
        for port in 0x514..=0x51B_u16 {
            io.register_io_handler(DeviceId::FwCfg, port, "fw_cfg dma", 0x7);
        }
    }

    /// Route a PCI config space read to the correct device.
    /// Bochs: devices.cc bx_devices_c::pci_read_handler() (inline in read_handler)
    fn pci_io_read(&self, address: u16, io_len: u8) -> u32 {
        match address {
            // Config address register (0xCF8)
            0x0CF8 => self.pci_conf_addr,
            // Config data register (0xCFC-0xCFF)
            0x0CFC..=0x0CFF => {
                let conf_addr = self.pci_conf_addr;
                if conf_addr & 0x8000_0000 == 0 {
                    return 0xFFFF_FFFF; // not enabled
                }
                let bus = ((conf_addr >> 16) & 0xFF) as u8;
                let devfunc = ((conf_addr >> 8) & 0xFF) as u8;
                let reg = (conf_addr & 0xFC) as u8;
                let offset = (address - 0x0CFC) as u8;

                if bus != 0 {
                    return 0xFFFF_FFFF; // only bus 0 implemented
                }

                let reg_addr = reg.wrapping_add(offset);
                self.pci_device_read(devfunc, reg_addr, io_len)
            }
            // APM + ELCR + CPU reset ports → PIIX3
            0x00B2 | 0x00B3 | 0x04D0 | 0x04D1 | 0x0CF9 => self.pci2isa.read(address),
            _ => {
                // BM-DMA ports
                let base = self.pci_ide.bmdma_base as u16;
                if base > 0 && address >= base && address < base + 16 {
                    self.pci_ide.bmdma_read(address, io_len)
                } else {
                    0xFFFF_FFFF
                }
            }
        }
    }

    /// Dispatch a PCI config read to the correct device by devfunc.
    /// Bochs: DEV_pci_rd_memtype() routing in devices.cc
    fn pci_device_read(&self, devfunc: u8, address: u8, io_len: u8) -> u32 {
        match devfunc {
            // Device 0, Func 0: i440FX host bridge
            0x00 => self.pci_bridge.pci_read(address, io_len),
            // Device 1, Func 0: PIIX3 PCI-to-ISA bridge
            0x08 => self.pci2isa.pci_read(address, io_len),
            // Device 1, Func 1: PIIX3 IDE controller
            0x09 => self.pci_ide.pci_read(address, io_len),
            // Device 1, Func 3: PIIX4 ACPI controller
            0x0B => self.acpi.pci_read(address, io_len),
            // Device 2, Func 0: PCI VGA (returns 0xFFFFFFFF when pci_vga is off)
            0x10 => self.vga.pci_read(address, io_len),
            // Unrecognized device
            _ => 0xFFFF_FFFF,
        }
    }

    /// Relocate PCI IDE BM-DMA I/O ports to the currently programmed BAR4.
    fn register_pci_ide_bmdma_ports(&mut self, io: &mut BxDevicesC) {
        let old_base = self.bmdma_ports_base;
        let new_base = self.pci_ide.bmdma_base as u16;
        if old_base == new_base {
            return;
        }
        if old_base != 0 {
            for offset in 0..16u16 {
                if self.pci_ide.bmdma_io_mask(offset as u8) != 0 {
                    io.unregister_io_handler(old_base + offset);
                }
            }
        }
        if new_base != 0 {
            for offset in 0..16u16 {
                let mask = self.pci_ide.bmdma_io_mask(offset as u8);
                if mask != 0 {
                    io.register_io_handler(
                        DeviceId::Pci,
                        new_base + offset,
                        "PCI IDE BM-DMA",
                        mask,
                    );
                }
            }
        }
        self.bmdma_ports_base = new_base;
        tracing::debug!(
            "PCI IDE BM-DMA ports relocated {old_base:#06x} -> {new_base:#06x}"
        );
    }

    fn relocate_acpi_pm_ports(&mut self, io: &mut BxDevicesC) {
        let old_base = self.pm_ports_base;
        let new_base = self.acpi.pm_base as u16;
        if old_base == new_base {
            return;
        }
        if old_base != 0 {
            for offset in 0..64u16 {
                if self.acpi.pm_io_mask(offset as u8) != 0 {
                    io.unregister_io_handler(old_base + offset);
                }
            }
        }
        self.acpi.pm_ports_registered = false;
        self.pm_ports_base = 0;
        if new_base != 0 {
            self.register_acpi_pm_ports(io);
        }
    }

    fn relocate_acpi_sm_ports(&mut self, io: &mut BxDevicesC) {
        let old_base = self.sm_ports_base;
        let new_base = self.acpi.sm_base as u16;
        if old_base == new_base {
            return;
        }
        if old_base != 0 {
            for offset in 0..16u16 {
                if self.acpi.sm_io_mask(offset as u8) != 0 {
                    io.unregister_io_handler(old_base + offset);
                }
            }
        }
        self.acpi.sm_ports_registered = false;
        self.sm_ports_base = 0;
        if new_base != 0 {
            self.register_acpi_sm_ports(io);
        }
    }

    /// Apply every queued PCI/memory effect in Bochs-observable order.
    ///
    /// Each producer flag is cleared only after its operation succeeds. A
    /// failed VGA relocation therefore retains both the committed old mapping
    /// and the pending BAR request.
    pub(crate) fn apply_pending_machine_boundary(
        &mut self,
        io: &mut BxDevicesC,
        mem: &mut crate::memory::BxMemC<'_>,
    ) -> Result<MachineBoundaryEffects> {
        let mut effects = MachineBoundaryEffects::default();

        if self.pci_ide_bar4_needs_reregister {
            self.register_pci_ide_bmdma_ports(io);
            self.pci_ide_bar4_needs_reregister = false;
        }
        if self.acpi_pm_needs_reregister {
            self.relocate_acpi_pm_ports(io);
            self.acpi_pm_needs_reregister = false;
        }
        if self.acpi_sm_needs_reregister {
            self.relocate_acpi_sm_ports(io);
            self.acpi_sm_needs_reregister = false;
        }
        if self.pam_needs_update {
            self.pci_bridge.apply_pam_to_memory(mem);
            self.pam_needs_update = false;
            effects.memory_mapping_changed = true;
        }
        if self.smram_needs_update {
            self.pci_bridge.apply_smram_to_memory(mem);
            self.smram_needs_update = false;
            effects.memory_mapping_changed = true;
        }
        if self.bios_write_needs_update {
            self.pci2isa.apply_bios_write_to_memory(mem);
            self.bios_write_needs_update = false;
            effects.memory_mapping_changed = true;
        }
        if self.vga_bar_needs_reregister {
            effects.memory_mapping_changed |= self.reregister_vga_bars(mem)?;
            self.vga_bar_needs_reregister = false;
        }

        io.pci_conf_addr = self.pci_conf_addr;
        Ok(effects)
    }

    /// Transactionally relocate both VGA PCI memory BARs.
    fn reregister_vga_bars(
        &mut self,
        mem: &mut crate::memory::BxMemC<'_>,
    ) -> Result<bool> {
        use crate::iodev::vga::PCI_VGA_MMIO_SIZE;
        let device_id = crate::memory::MemoryDeviceId::Vga(&mut self.vga as *mut BxVgaC);
        let mut changed = false;

        if let Some((old_base, new_base)) = self.vga.peek_pending_lfb_relocate() {
            let size = u64::from(self.vga.lfb_size());
            let old_range =
                (old_base != 0).then_some((u64::from(old_base), u64::from(old_base) + size - 1));
            let new_range =
                (new_base != 0).then_some((u64::from(new_base), u64::from(new_base) + size - 1));
            mem.relocate_memory_handlers(device_id, old_range, new_range)?;
            self.vga.commit_pending_lfb_relocate();
            changed = old_base != new_base;
            tracing::info!("VGA LFB relocated {old_base:#010x} -> {new_base:#010x}");
        }

        if let Some((old_base, new_base)) = self.vga.peek_pending_mmio_relocate() {
            let size = u64::from(PCI_VGA_MMIO_SIZE);
            let old_range =
                (old_base != 0).then_some((u64::from(old_base), u64::from(old_base) + size - 1));
            let new_range =
                (new_base != 0).then_some((u64::from(new_base), u64::from(new_base) + size - 1));
            mem.relocate_memory_handlers(device_id, old_range, new_range)?;
            self.vga.commit_pending_mmio_relocate();
            changed |= old_base != new_base;
            tracing::info!("VGA MMIO relocated {old_base:#010x} -> {new_base:#010x}");
        }

        Ok(changed)
    }

    /// BM-DMA timer — walks the PRD table and pumps data between the drive
    /// and guest RAM. Bochs pci_ide.cc `bx_pci_ide_c::timer()`.
    ///
    /// Bochs transfers directly between the drive and the channel bounce
    /// buffer; here each sector passes through a small stack buffer because
    /// the drive callbacks need `&mut BxPciIde` (abort/IRQ paths) while the
    /// bounce buffer also lives in `BxPciIde::bmdma[channel]`.
    pub(crate) fn pci_ide_timer<'c>(
        &mut self,
        channel: usize,
        pcs: &mut crate::pc_system::BxPcSystemC,
        mem: &mut crate::memory::BxMemC<'c>,
        pins: &[CpuTlbPin],
    ) {
        if channel >= 2 {
            return;
        }
        let DeviceManager {
            ref mut pci_ide,
            ref mut harddrv,
            ref mut pic,
            ref mut bmdma_scratch,
            ..
        } = *self;

        // Bochs pci_ide.cc timer: engine stopped or no PRD — nothing to do.
        if (pci_ide.bmdma[channel].status & 0x01) == 0 || pci_ide.bmdma[channel].prd_current == 0 {
            return;
        }
        let timer_index = match pci_ide.bmdma[channel].timer_index {
            Some(index) => index,
            None => {
                tracing::error!("BM-DMA ch={channel}: timer fired without a registered handle");
                return;
            }
        };
        // Bochs pci_ide.cc timer: READ DMA waits for the drive's data_ready
        // handshake (bmdma_start_transfer), re-polling at 1 us.
        if pci_ide.bmdma[channel].cmd_rwcon && !pci_ide.bmdma[channel].data_ready {
            if let Err(error) = pcs.activate_timer_usec(timer_index, 1, false) {
                tracing::error!("BM-DMA ch={channel}: data-ready re-arm failed: {error:?}");
            }
            return;
        }

        // Fetch the current PRD entry (8 bytes: physical addr, size) from
        // guest RAM. Bochs pci_ide.cc timer: DEV_MEM_READ_PHYSICAL.
        let (prd_addr, prd_size_raw) =
            read_bmdma_prd(mem, pins, pci_ide.bmdma[channel].prd_current);
        let mut size = (prd_size_raw & 0xfffe) as usize;
        if size == 0 {
            size = 0x10000;
        }

        if pci_ide.bmdma[channel].cmd_rwcon {
            // READ DMA: drive -> bounce buffer -> guest RAM (pci_ide.cc timer)
            tracing::trace!("BM-DMA read ch={channel} addr={prd_addr:#010x} size={size:#x}");
            let buffered = pci_ide.bmdma[channel].buffer_top - pci_ide.bmdma[channel].buffer_idx;
            let mut count = size as i64 - buffered as i64;
            while count > 0 {
                let mut sector_size = count as u32;
                if harddrv.bmdma_read_sector(
                    channel as u8,
                    bmdma_scratch,
                    &mut sector_size,
                    pic,
                    pci_ide,
                ) {
                    let top = pci_ide.bmdma[channel].buffer_top;
                    let len = (sector_size as usize).min(bmdma_scratch.len());
                    let end = (top + len).min(pci_ide.bmdma[channel].buffer.len());
                    pci_ide.bmdma[channel].buffer[top..end]
                        .copy_from_slice(&bmdma_scratch[..end - top]);
                    pci_ide.bmdma[channel].buffer_top = end;
                    count -= sector_size as i64;
                } else {
                    break;
                }
            }
            if count > 0 {
                // Drive ran dry mid-PRD: abort (pci_ide.cc timer).
                harddrv.bmdma_complete(channel as u8, pic, pci_ide);
                return;
            }
            let idx = pci_ide.bmdma[channel].buffer_idx;
            let end = (idx + size).min(pci_ide.bmdma[channel].buffer.len());
            let payload = &pci_ide.bmdma[channel].buffer[idx..end];
            match mem.write_ram(pins, prd_addr as u64, payload) {
                Ok(copied) if copied == payload.len() => {
                    pci_ide.bmdma[channel].buffer_idx = end;
                }
                Ok(copied) => {
                    tracing::error!(
                        "BM-DMA read ch={channel}: guest write accepted {copied}/{} bytes",
                        payload.len()
                    );
                    harddrv.bmdma_abort(channel as u8, pic, pci_ide);
                    return;
                }
                Err(error) => {
                    tracing::error!("BM-DMA read ch={channel}: guest write failed: {error:?}");
                    harddrv.bmdma_abort(channel as u8, pic, pci_ide);
                    return;
                }
            }
        } else {
            // WRITE DMA: guest RAM -> bounce buffer -> drive (pci_ide.cc timer)
            tracing::trace!("BM-DMA write ch={channel} addr={prd_addr:#010x} size={size:#x}");
            let top = pci_ide.bmdma[channel].buffer_top;
            let end = (top + size).min(pci_ide.bmdma[channel].buffer.len());
            let guest_buffer = &mut pci_ide.bmdma[channel].buffer[top..end];
            guest_buffer.fill(0);
            let copied = match mem.read_ram(pins, prd_addr as u64, guest_buffer) {
                Ok(copied) => copied,
                Err(error) => {
                    tracing::error!("BM-DMA write ch={channel}: guest read failed: {error:?}");
                    0
                }
            };
            // Guest PRD holes/out-of-range tails are deterministic zeroes.
            guest_buffer[copied..].fill(0);
            pci_ide.bmdma[channel].buffer_top = end;

            let mut count =
                (pci_ide.bmdma[channel].buffer_top - pci_ide.bmdma[channel].buffer_idx) as i64;
            while count > 511 {
                let idx = pci_ide.bmdma[channel].buffer_idx;
                bmdma_scratch[..512]
                    .copy_from_slice(&pci_ide.bmdma[channel].buffer[idx..idx + 512]);
                if harddrv.bmdma_write_sector(channel as u8, &bmdma_scratch[..512], pic, pci_ide) {
                    pci_ide.bmdma[channel].buffer_idx += 512;
                    count -= 512;
                } else {
                    break;
                }
            }
            if count >= 512 {
                // Drive refused a sector mid-PRD: abort (pci_ide.cc timer).
                harddrv.bmdma_complete(channel as u8, pic, pci_ide);
                return;
            }
        }

        if prd_size_raw & 0x8000_0000 != 0 {
            // End of PRD table: transfer done (pci_ide.cc timer).
            pci_ide.bmdma[channel].status &= !0x01;
            pci_ide.bmdma[channel].status |= 0x04;
            pci_ide.bmdma[channel].prd_current = 0;
            harddrv.bmdma_complete(channel as u8, pic, pci_ide);
        } else {
            // Compact residue to the buffer start and move to the next PRD
            // (pci_ide.cc timer: memmove + prd_current += 8 + re-arm).
            let idx = pci_ide.bmdma[channel].buffer_idx;
            let top = pci_ide.bmdma[channel].buffer_top;
            let residue = top - idx;
            if residue > 0 {
                pci_ide.bmdma[channel].buffer.copy_within(idx..top, 0);
            }
            pci_ide.bmdma[channel].buffer_top = residue;
            pci_ide.bmdma[channel].buffer_idx = 0;
            pci_ide.bmdma[channel].prd_current += 8;
            let (_, next_size_raw) =
                read_bmdma_prd(mem, pins, pci_ide.bmdma[channel].prd_current);
            let mut next_size = next_size_raw & 0xfffe;
            if next_size == 0 {
                next_size = 0x10000;
            }
            if let Err(error) = pcs.activate_timer_usec(timer_index, (next_size >> 4) | 0x10, false)
            {
                tracing::error!("BM-DMA ch={channel}: next-PRD re-arm failed: {error:?}");
            }
        }
    }

    /// Replay pending PIT counter-0 OUT transitions into the PIC as IRQ0
    /// raise/lower calls — Bochs pit.cc bx_pit_c::irq_handler (raise_irq(0)
    /// on OUT 0→1, lower_irq(0) on 1→0), which Bochs invokes synchronously
    /// from pit82c54.cc set_OUT on every transition (clocking, count
    /// writes, control-word writes, GATE changes alike).
    ///
    /// rusty_box records the transitions on the counter and replays them
    /// here, at the same points Bochs runs periodic()/timer.write(): after
    /// every PIT port access and every device tick. The CPU never executes
    /// between the recorded transitions, so replaying them back-to-back is
    /// observably identical to Bochs's immediate callbacks.
    ///
    /// Transitions strictly alternate (set_OUT fires only on an actual
    /// change), so the sequence is fully determined by (count, final
    /// level). Replay is capped at the last three transitions: with no CPU
    /// execution in between, each additional leading lower/raise pair is
    /// idempotent for the PIC IRR/irq_in state and for the IOAPIC forward
    /// consumers (repeated edge deliveries re-set the same LAPIC IRR bit).
    ///
    /// Returns the number of IRQ0 rising edges in the full (uncapped)
    /// sequence, for diagnostics.
    pub(crate) fn replay_pit_irq0_events(
        transitions: u32,
        level: bool,
        pic: &mut BxPicC,
    ) -> u32 {
        if transitions == 0 {
            return 0;
        }
        let replay = transitions.min(3);
        // The k-th replayed level, ending at `level`, alternating backwards.
        let mut lvl = if replay % 2 == 1 { level } else { !level };
        for _ in 0..replay {
            if lvl {
                pic.raise_irq(0);
            } else {
                pic.lower_irq(0);
            }
            lvl = !lvl;
        }
        if level {
            transitions.div_ceil(2)
        } else {
            transitions / 2
        }
    }

    pub(crate) fn service_pit_irq0(pit: &mut BxPitC, pic: &mut BxPicC) -> u32 {
        let (transitions, level) = pit.drain_irq0_events();
        // Bochs pit.cc irq_handler: with irq_enabled clear (HPET legacy
        // mode), OUT transitions are consumed but never reach the PIC.
        if !pit.irq_enabled {
            return 0;
        }
        Self::replay_pit_irq0_events(transitions, level, pic)
    }

    /// Drain PIT IRQ0 transitions into the PIC and update diagnostics.
    pub(crate) fn drain_pit_irq0(&mut self) {
        let was_high = self.pic.master.irq_in[0] != 0;
        let rising = Self::service_pit_irq0(&mut self.pit, &mut self.pic);
        if rising > 0 {
            self.diag_pit_fires += rising as u64;
            self.diag_irq0_latched += 1;
            if was_high {
                self.diag_irq0_already_high += 1;
            }
        }
    }


    /// Check if an interrupt is pending
    pub fn has_interrupt(&self) -> bool {
        self.pic.has_interrupt()
    }

    /// Acknowledge interrupt and get vector
    pub fn iac(&mut self) -> u8 {
        self.diag_iac_count += 1;
        let vector = self.pic.iac();
        self.diag_vector_hist[vector as usize] += 1;
        vector
    }

    /// Get A20 state from keyboard controller
    pub fn get_a20_from_keyboard(&self) -> bool {
        self.keyboard.get_a20_enabled()
    }

    /// Get ATA I/O counts for diagnostics
    pub fn ata_io_counts(&self) -> (u64, u64) {
        (0, 0)
    }

    #[cfg(feature = "alloc")]
    /// Get PIC diagnostic string
    pub fn pic_diag(&self) -> String {
        format!(
            "ISR={:#04x} IRR={:#04x} IMR={:#04x} int_pin={} irq_in[0]={} master_offset={:#04x} slave_offset={:#04x} master_auto_eoi={} slave_auto_eoi={} master_edge_level={:#04x} slave_edge_level={:#04x}",
            self.pic.master.isr,
            self.pic.master.irr,
            self.pic.master.imr,
            self.pic.master.int_pin,
            self.pic.master.irq_in[0],
            self.pic.master.interrupt_offset,
            self.pic.slave.interrupt_offset,
            self.pic.master.auto_eoi,
            self.pic.slave.auto_eoi,
            self.pic.master.edge_level,
            self.pic.slave.edge_level,
        )
    }

    /// Drain serial port TX output for diagnostics
    pub fn drain_serial_tx(&mut self, port_index: usize) -> impl Iterator<Item = u8> + '_ {
        self.serial.drain_tx_output(port_index)
    }

    /// Get keyboard diagnostic info
    pub fn kbd_diag(&self) -> (u64, u8, bool, bool, bool, bool) {
        (
            self.keyboard.diag_port60_read_count,
            self.keyboard.diag_port60_last_value,
            self.keyboard.kbd_controller.kbd_clock_enabled,
            self.keyboard.kbd_internal_buffer.scanning_enabled,
            self.keyboard.kbd_controller.scancodes_translate,
            self.keyboard.kbd_controller.outb,
        )
    }

    #[cfg(feature = "alloc")]
    /// Get ATA controller diagnostic string
    pub fn ata_diag(&self) -> String {
        self.harddrv.diag_string()
    }

    #[cfg(feature = "alloc")]
    /// Get full interrupt chain diagnostic summary (for end-of-run reporting)
    pub fn interrupt_chain_diag(&self) -> String {
        let c0 = &self.pit.counters[0];
        format!(
            "PIT: pit_fires={} irq0_latched={} irq0_already_high={}\n\
             PIT counter0: mode={:?} inlatch={} count={} count_written={} gate={} output={} first_pass={}\n\
             PIC master: ISR={:#04x} IRR={:#04x} IMR={:#04x} int_pin={} irq_in[0..8]=[{},{},{},{},{},{},{},{}]\n\
             PIC slave:  ISR={:#04x} IRR={:#04x} IMR={:#04x} int_pin={} irq_in[0..8]=[{},{},{},{},{},{},{},{}]\n\
             PIC master_offset={:#04x} slave_offset={:#04x}\n\
             IAC calls={} vector_hist[0x20]={} vector_hist[0x21]={} vector_hist[0x08]={} vector_hist[0x2E]={}",
            self.diag_pit_fires,
            self.diag_irq0_latched, self.diag_irq0_already_high,
            c0.mode, c0.inlatch, c0.count, c0.count_written, c0.gate, c0.output, c0.first_pass,
            self.pic.master.isr, self.pic.master.irr, self.pic.master.imr,
            self.pic.master.int_pin,
            self.pic.master.irq_in[0], self.pic.master.irq_in[1],
            self.pic.master.irq_in[2], self.pic.master.irq_in[3],
            self.pic.master.irq_in[4], self.pic.master.irq_in[5],
            self.pic.master.irq_in[6], self.pic.master.irq_in[7],
            self.pic.slave.isr, self.pic.slave.irr, self.pic.slave.imr,
            self.pic.slave.int_pin,
            self.pic.slave.irq_in[0], self.pic.slave.irq_in[1],
            self.pic.slave.irq_in[2], self.pic.slave.irq_in[3],
            self.pic.slave.irq_in[4], self.pic.slave.irq_in[5],
            self.pic.slave.irq_in[6], self.pic.slave.irq_in[7],
            self.pic.master.interrupt_offset, self.pic.slave.interrupt_offset,
            self.diag_iac_count,
            self.diag_vector_hist[0x20], self.diag_vector_hist[0x21],
            self.diag_vector_hist[0x08], self.diag_vector_hist[0x2E],
        )
    }

    // ─── Dispatch methods called from BxDevicesC via DeviceId ───

    /// Port 92h read dispatch (System Control Port)
    pub(crate) fn port92_read(&self, _port: u16, _io_len: u8) -> u32 {
        self.port92.read() as u32
    }

    /// Port 92h write dispatch (System Control Port)
    pub(crate) fn port92_write(&mut self, _port: u16, value: u32, _io_len: u8) {
        self.port92.write(value as u8);
    }

    /// PCI I/O read dispatch
    pub(crate) fn pci_read(&self, address: u16, io_len: u8) -> u32 {
        self.pci_io_read(address, io_len)
    }

    /// PCI I/O write dispatch
    pub(crate) fn pci_write(&mut self, address: u16, value: u32, io_len: u8) {
        match address {
            0x0CF8 => {
                self.pci_conf_addr = value;
            }
            0x0CFC..=0x0CFF => {
                let conf_addr = self.pci_conf_addr;
                if conf_addr & 0x8000_0000 == 0 {
                    return;
                }
                let bus = ((conf_addr >> 16) & 0xFF) as u8;
                let devfunc = ((conf_addr >> 8) & 0xFF) as u8;
                let reg = (conf_addr & 0xFC) as u8;
                let offset = (address - 0x0CFC) as u8;
                if bus != 0 {
                    return;
                }
                let reg_addr = reg + offset;
                // Bochs devices.cc pci_write_handler_common: gate on the
                // write's STARTING offset only (pci_write_common_gate), then
                // dispatch the (possibly 0x3C-clamped) write to the target
                // device's pci_write exactly once, for every devfunc.
                match devfunc {
                    0x00 => {
                        if let Some((addr, val, len)) =
                            pci_write_common_gate(reg_addr, value, io_len)
                        {
                            let effects = self.pci_bridge.pci_write(addr, val, len);
                            if effects.pam_changed {
                                self.pam_needs_update = true;
                            }
                            if effects.smram_changed {
                                self.smram_needs_update = true;
                            }
                        }
                    }
                    0x08 => {
                        if let Some((addr, val, len)) =
                            pci_write_common_gate(reg_addr, value, io_len)
                        {
                            let effects = self.pci2isa.pci_write(addr, val, len);
                            if effects.bios_write_changed {
                                self.bios_write_needs_update = true;
                            }
                        }
                    }
                    0x09 => {
                        if let Some((addr, val, len)) =
                            pci_write_common_gate(reg_addr, value, io_len)
                        {
                            if self.pci_ide.pci_write(addr, val, len) {
                                self.pci_ide_bar4_needs_reregister = true;
                            }
                        }
                    }
                    0x0B => {
                        if let Some((addr, val, len)) =
                            pci_write_common_gate(reg_addr, value, io_len)
                        {
                            let (pm, sm) = self.acpi.pci_write(addr, val, len);
                            if pm {
                                self.acpi_pm_needs_reregister = true;
                            }
                            if sm {
                                self.acpi_sm_needs_reregister = true;
                            }
                        }
                    }
                    0x10 => {
                        if let Some((addr, val, len)) =
                            pci_write_common_gate(reg_addr, value, io_len)
                        {
                            let change = self.vga.pci_write(addr, val, len);
                            if change.lfb || change.mmio {
                                self.vga_bar_needs_reregister = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            0x00B2 | 0x00B3 | 0x04D0 | 0x04D1 | 0x0CF9 => {
                self.pci2isa.write(address, value, io_len);
                if address == 0x00B2 {
                    // Bochs pci2isa.cc case 0x00b2: apmc/apms are stored by
                    // pci2isa.write above; DEV_acpi_generate_smi delivers the
                    // SMI (when APMC_EN is set) and the GUEST's SMM handler is
                    // what acknowledges the command — e.g. the BIOS relocation
                    // handler's `out 0xb3, 0`. apms is never cleared here.
                    self.acpi.generate_smi(value as u8);
                }
                // Bochs pci2isa.cc write case 0x04d0/0x04d1:
                // DEV_pic_set_mode(is_master, elcr) — forward the new
                // edge/level trigger mode to the 8259 whose ELCR changed.
                if self.pci2isa.elcr1_changed {
                    self.pci2isa.elcr1_changed = false;
                    self.pic.set_mode(true, self.pci2isa.elcr1);
                }
                if self.pci2isa.elcr2_changed {
                    self.pci2isa.elcr2_changed = false;
                    self.pic.set_mode(false, self.pci2isa.elcr2);
                }
            }
            _ => {
                let base = self.pci_ide.bmdma_base as u16;
                if base > 0 && address >= base && address < base + 16 {
                    self.pci_ide.bmdma_write(address, value, io_len);
                }
            }
        }
    }

    /// ACPI I/O read dispatch
    pub(crate) fn acpi_read(&mut self, address: u16, io_len: u8, icount: u64) -> u32 {
        self.acpi.read(address, io_len, icount)
    }

    /// ACPI I/O write dispatch
    pub(crate) fn acpi_write(&mut self, address: u16, value: u32, io_len: u8, icount: u64) {
        if address == 0x00B2 {
            self.acpi.generate_smi(value as u8);
        } else {
            self.acpi.write(address, value, io_len, icount);
        }
    }

    /// PCI IDE I/O read dispatch (BM-DMA ports)
    pub(crate) fn pci_ide_read(&self, address: u16, io_len: u8) -> u32 {
        self.pci_ide.bmdma_read(address, io_len)
    }

    /// PCI IDE I/O write dispatch (BM-DMA ports)
    pub(crate) fn pci_ide_write(&mut self, address: u16, value: u32, io_len: u8) {
        self.pci_ide.bmdma_write(address, value, io_len);
    }

    /// fw_cfg I/O write dispatch — reconstructs the stable active pin slice.
    pub(crate) fn fw_cfg_write(&mut self, address: u16, value: u32, io_len: u8) {
        let mem = self.mem_ptr.map(|mut p| unsafe { p.as_mut() });
        let pins = match (self.active_tlb_pins, self.active_tlb_pin_count) {
            (Some(ptr), count) => unsafe { core::slice::from_raw_parts(ptr.as_ptr(), count) },
            (None, 0) => &[],
            (None, count) => {
                tracing::error!("fw_cfg DMA: missing pin storage for {count} active CPUs");
                &[]
            }
        };
        self.fw_cfg.write_port(address, value, io_len, mem, pins);
    }
}

impl BxDevicesC {
    /// Initialize all devices
    ///
    /// This is the main device initialization function corresponding to
    /// `DEV_init_devices()` / `bx_devices_c::init()` in Bochs.
    ///
    /// # Arguments
    /// * `mem` - Memory subsystem reference
    /// * `port92_state` - Optional pointer to SystemControlPort for Port 92h handling
    pub fn init(&mut self, _mem: &mut BxMemC) -> Result<()> {
        tracing::debug!("Initializing device subsystem");

        // Register Port 92h - System Control Port (A20 gate, fast reset)
        self.register_io_handler(DeviceId::Port92, PORT_92H, "Port 92h System Control", 0x1);

        tracing::debug!("Device initialization complete");
        Ok(())
    }

    /// Initialize devices with PC system reference for A20 control
    ///
    /// This variant allows devices to control the A20 line during operation.
    pub fn init_with_pc_system(
        &mut self,
        _mem: &mut BxMemC,
        _pc_system: &mut BxPcSystemC,
    ) -> Result<()> {
        self.init(_mem)
    }

    /// Reset all devices
    ///
    /// Matches bx_devices_c::reset() from cpp_orig/bochs/iodev/devices.cc
    ///
    /// # Arguments
    /// * `reset_type` - Type of reset (Hardware or Software)
    pub fn reset(&mut self, reset_type: ResetReason) -> Result<()> {
        match reset_type {
            ResetReason::Hardware => {
                tracing::debug!("Device hardware reset");
                {
                    // Clear PCI configuration address (line 402)
                    self.pci_conf_addr = 0;
                }
                // Note: mem->disable_smram() at line 405 - SMRAM disable not yet implemented
                // Note: bx_reset_plugins(type) at line 406 - done via device_manager.reset()
                // Note: release_keys() at line 407 - keyboard key release not yet implemented
                // Note: paste.stop = 1 at line 409 - paste buffer stop not yet implemented
            }
            ResetReason::Software => {
                tracing::debug!("Device software reset");
            }
        }
        Ok(())
    }

    /// Register device state for save/restore functionality
    pub fn register_state(&mut self) -> Result<()> {
        tracing::trace!("Device state registered");
        Ok(())
    }
}

/// Helper structure for managing Port 92h state
/// This is used by the Emulator to track and respond to Port 92h changes
#[derive(Debug, Default)]
pub struct SystemControlPort {
    /// Last written value to port 92h
    pub value: u8,
    /// Most recently requested A20 gate value from port 92h. The central
    /// boundary synchronizes this mirror to the machine gate after quiescing.
    pub a20_gate: bool,
    /// Set only on an actual A20 gate transition. The central boundary keeps
    /// this mirror in sync with the machine gate after every quiesce, so a
    /// compare against the mirror is a compare against the machine state.
    pub(crate) a20_change_pending: bool,
    /// Reset request type from port 92h bit 0 (Bochs treats it as software reset).
    pub reset_request: Option<ResetReason>,
}

impl SystemControlPort {
    /// Create a new System Control Port state
    pub fn new() -> Self {
        Self {
            value: 0,
            a20_gate: true, // A20 enabled by default on modern systems
            a20_change_pending: false,
            reset_request: None,
        }
    }

    /// Queue a port 92h A20/reset request for the central machine boundary.
    /// Callers observe latched work through `a20_change_pending` and
    /// `reset_request` (also aggregated by `has_pending_machine_boundary`).
    pub fn write(&mut self, value: u8) {
        self.value = value;
        // Bochs pc_system.cc bx_pc_system_c::set_enable_a20 fires
        // MemoryMappingChanged() only "If there has been a transition"; a
        // repeated same-value write is architecturally a no-op. The mirror is
        // re-synced from the machine gate at every boundary quiesce, so the
        // transition compare below is against real machine state.
        let new_gate = (value & 0x02) != 0;
        if new_gate != self.a20_gate {
            self.a20_gate = new_gate;
            self.a20_change_pending = true;
        }
        self.reset_request = if (value & 0x01) != 0 {
            Some(ResetReason::Software)
        } else {
            None
        };
    }

    /// Read current port 92h value
    /// Bochs devices.cc: return(BX_GET_ENABLE_A20() << 1)
    pub fn read(&self) -> u8 {
        // Bit 1 = A20 gate state, Bit 0 = 0 (reset trigger write-only)
        if self.a20_gate {
            0x02
        } else {
            0x00
        }
    }
}

#[cfg(feature = "std")]
fn invalid_platform_snapshot(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

#[cfg(feature = "std")]
fn validate_snapshot_io_base(base: u32, alignment: u32, span: u32) -> io::Result<()> {
    if base == 0 {
        return Ok(());
    }
    if alignment == 0
        || !alignment.is_power_of_two()
        || span == 0
        || base & (alignment - 1) != 0
    {
        return Err(invalid_platform_snapshot(
            "snapshot I/O mapping base is not canonically aligned",
        ));
    }
    let end = base.checked_add(span - 1).ok_or_else(|| {
        invalid_platform_snapshot("snapshot I/O mapping range overflows")
    })?;
    if end > u32::from(u16::MAX) {
        return Err(invalid_platform_snapshot(
            "snapshot I/O mapping range exceeds the port space",
        ));
    }
    Ok(())
}

#[cfg(feature = "std")]
fn validate_snapshot_memory_bar(base: u32, size: u32) -> io::Result<()> {
    if base == 0 {
        return Ok(());
    }
    if size == 0 || !size.is_power_of_two() || base & (size - 1) != 0 {
        return Err(invalid_platform_snapshot(
            "snapshot PCI memory BAR is not canonically aligned",
        ));
    }
    base.checked_add(size - 1).ok_or_else(|| {
        invalid_platform_snapshot("snapshot PCI memory BAR range overflows")
    })?;
    Ok(())
}

#[cfg(feature = "std")]
fn validate_snapshot_mapping_flag(
    pending: bool,
    committed: u32,
    desired: u32,
) -> io::Result<()> {
    if pending != (committed != desired) {
        return Err(invalid_platform_snapshot(
            "snapshot mapping flag and bases are incoherent",
        ));
    }
    Ok(())
}

#[cfg(feature = "std")]
impl SystemControlPort {
    /// Number of bytes emitted by the PLATFORM port-92 component body.
    pub(crate) fn snapshot_v3_body_len(&self) -> io::Result<u64> {
        self.validate_snapshot_v3_state()?;
        checked_snapshot_len_add(4, u64::from(self.reset_request.is_some()))
    }

    /// Stream the desired A20 state, its pending boundary bit, and a pending
    /// reset request. This is state capture only; it never updates the
    /// machine-wide A20 view or executes a reset.
    pub(crate) fn save_snapshot_v3_body<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.validate_snapshot_v3_state()?;

        writer.write_u8(self.value)?;
        writer.write_bool(self.a20_gate)?;
        writer.write_bool(self.a20_change_pending)?;
        writer.write_bool(self.reset_request.is_some())?;
        if self.reset_request.is_some() {
            // Port 92h can only originate a software reset. The validation
            // above rejects any other enum value before output begins.
            writer.write_u8(0)?;
        }
        Ok(())
    }

    /// Decode port-92 continuation state without applying the desired A20
    /// value or consuming the pending reset request.
    pub(crate) fn restore_snapshot_v3_body<R: Read>(
        &mut self,
        reader: &mut SnapshotReader<R>,
    ) -> io::Result<()> {
        let value = reader.read_u8()?;
        let a20_gate = reader.read_bool()?;
        let a20_change_pending = reader.read_bool()?;
        let reset_request = if reader.read_bool()? {
            match reader.read_u8()? {
                0 => Some(ResetReason::Software),
                _ => {
                    return Err(invalid_platform_snapshot(
                        "snapshot port 92h reset reason is invalid",
                    ));
                }
            }
        } else {
            None
        };

        self.value = value;
        self.a20_gate = a20_gate;
        self.a20_change_pending = a20_change_pending;
        self.reset_request = reset_request;
        Ok(())
    }

    fn validate_snapshot_v3_state(&self) -> io::Result<()> {
        if matches!(self.reset_request, Some(ResetReason::Hardware)) {
            return Err(invalid_platform_snapshot(
                "port 92h cannot carry a hardware reset request",
            ));
        }
        Ok(())
    }
}

#[cfg(feature = "std")]
impl DeviceManager {
    /// Number of bytes emitted by the PLATFORM DeviceManager component body.
    ///
    /// Dynamic port and memory registrations remain live topology. This body
    /// instead records their saved committed identities, desired targets, and
    /// the exact deferred effects that the parent must resume in order.
    pub(crate) fn snapshot_v3_body_len(&self) -> io::Result<u64> {
        let desired_vga = self.vga.snapshot_v3_mapping_target();
        let committed_vga = self.vga.snapshot_v3_committed_mapping_target();
        self.validate_snapshot_v3_state(desired_vga, committed_vga)?;

        let port92_len = self.port92.snapshot_v3_body_len()?;
        // PCI latch (4), seven deferred flags, three committed I/O bases
        // (6), three desired I/O bases (12), and two committed plus two
        // desired VGA BAR bases (16).
        checked_snapshot_len_add(port92_len, 45)
    }

    /// Stream deferred mapping/routing state. No device codec is nested here:
    /// fw_cfg, PCI/ACPI/VGA, and the other device families own their own
    /// section bodies.
    pub(crate) fn save_snapshot_v3_body<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let desired_vga = self.vga.snapshot_v3_mapping_target();
        let committed_vga = self.vga.snapshot_v3_committed_mapping_target();
        self.validate_snapshot_v3_state(desired_vga, committed_vga)?;

        self.port92.save_snapshot_v3_body(writer)?;
        writer.write_u32(self.pci_conf_addr)?;
        writer.write_bool(self.pci_ide_bar4_needs_reregister)?;
        writer.write_bool(self.acpi_pm_needs_reregister)?;
        writer.write_bool(self.acpi_sm_needs_reregister)?;
        writer.write_bool(self.pam_needs_update)?;
        writer.write_bool(self.smram_needs_update)?;
        writer.write_bool(self.bios_write_needs_update)?;
        writer.write_bool(self.vga_bar_needs_reregister)?;
        writer.write_u16(self.bmdma_ports_base)?;
        writer.write_u16(self.pm_ports_base)?;
        writer.write_u16(self.sm_ports_base)?;
        writer.write_u32(self.pci_ide.bmdma_base)?;
        writer.write_u32(self.acpi.pm_base)?;
        writer.write_u32(self.acpi.sm_base)?;
        writer.write_u32(committed_vga.lfb_base)?;
        writer.write_u32(committed_vga.mmio_base)?;
        writer.write_u32(desired_vga.lfb_base)?;
        writer.write_u32(desired_vga.mmio_base)
    }

    /// Decode pending PLATFORM effects without altering live I/O or memory
    /// registrations. The returned targets are committed only after the
    /// machine-level decoder has cross-validated every device section and
    /// relocated the captured live topology.
    pub(crate) fn restore_snapshot_v3_body<R: Read>(
        &mut self,
        reader: &mut SnapshotReader<R>,
    ) -> io::Result<PlatformSnapshotRestore> {
        self.port92.restore_snapshot_v3_body(reader)?;
        let pci_conf_addr = reader.read_u32()?;
        let pci_ide_bar4_needs_reregister = reader.read_bool()?;
        let acpi_pm_needs_reregister = reader.read_bool()?;
        let acpi_sm_needs_reregister = reader.read_bool()?;
        let pam_needs_update = reader.read_bool()?;
        let smram_needs_update = reader.read_bool()?;
        let bios_write_needs_update = reader.read_bool()?;
        let vga_bar_needs_reregister = reader.read_bool()?;
        let committed_bmdma_ports_base = reader.read_u16()?;
        let committed_pm_ports_base = reader.read_u16()?;
        let committed_sm_ports_base = reader.read_u16()?;
        let desired_bmdma_base = reader.read_u32()?;
        let desired_pm_base = reader.read_u32()?;
        let desired_sm_base = reader.read_u32()?;
        let committed_vga_lfb_base = reader.read_u32()?;
        let committed_vga_mmio_base = reader.read_u32()?;
        let desired_vga_lfb_base = reader.read_u32()?;
        let desired_vga_mmio_base = reader.read_u32()?;

        Self::validate_snapshot_v3_topology(
            pci_ide_bar4_needs_reregister,
            acpi_pm_needs_reregister,
            acpi_sm_needs_reregister,
            vga_bar_needs_reregister,
            committed_bmdma_ports_base,
            committed_pm_ports_base,
            committed_sm_ports_base,
            desired_bmdma_base,
            desired_pm_base,
            desired_sm_base,
            committed_vga_lfb_base,
            committed_vga_mmio_base,
            desired_vga_lfb_base,
            desired_vga_mmio_base,
            self.vga.lfb_size(),
        )?;

        Ok(PlatformSnapshotRestore {
            pci_conf_addr,
            port92_a20_gate: self.port92.a20_gate,
            port92_a20_change_pending: self.port92.a20_change_pending,
            port92_reset_request: self.port92.reset_request,
            pci_ide_bar4_needs_reregister,
            acpi_pm_needs_reregister,
            acpi_sm_needs_reregister,
            pam_needs_update,
            smram_needs_update,
            bios_write_needs_update,
            vga_bar_needs_reregister,
            committed_bmdma_ports_base,
            committed_pm_ports_base,
            committed_sm_ports_base,
            desired_bmdma_base,
            desired_pm_base,
            desired_sm_base,
            committed_vga_lfb_base,
            committed_vga_mmio_base,
            desired_vga_lfb_base,
            desired_vga_mmio_base,
        })
    }

    fn validate_snapshot_v3_state(
        &self,
        desired_vga: super::vga::VgaSnapshotRestoreTarget,
        committed_vga: super::vga::VgaSnapshotRestoreTarget,
    ) -> io::Result<()> {
        self.port92.validate_snapshot_v3_state()?;
        if self.acpi.pm_ports_registered != (self.pm_ports_base != 0)
            || self.acpi.sm_ports_registered != (self.sm_ports_base != 0)
        {
            return Err(invalid_platform_snapshot(
                "live ACPI port registration state is incoherent",
            ));
        }
        Self::validate_snapshot_v3_topology(
            self.pci_ide_bar4_needs_reregister,
            self.acpi_pm_needs_reregister,
            self.acpi_sm_needs_reregister,
            self.vga_bar_needs_reregister,
            self.bmdma_ports_base,
            self.pm_ports_base,
            self.sm_ports_base,
            self.pci_ide.bmdma_base,
            self.acpi.pm_base,
            self.acpi.sm_base,
            committed_vga.lfb_base,
            committed_vga.mmio_base,
            desired_vga.lfb_base,
            desired_vga.mmio_base,
            self.vga.lfb_size(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_snapshot_v3_topology(
        pci_ide_pending: bool,
        acpi_pm_pending: bool,
        acpi_sm_pending: bool,
        vga_pending: bool,
        committed_bmdma: u16,
        committed_pm: u16,
        committed_sm: u16,
        desired_bmdma: u32,
        desired_pm: u32,
        desired_sm: u32,
        committed_vga_lfb: u32,
        committed_vga_mmio: u32,
        desired_vga_lfb: u32,
        desired_vga_mmio: u32,
        vga_lfb_size: u32,
    ) -> io::Result<()> {
        validate_snapshot_io_base(desired_bmdma, 16, 16)?;
        validate_snapshot_io_base(u32::from(committed_bmdma), 16, 16)?;
        validate_snapshot_io_base(desired_pm, 64, 64)?;
        validate_snapshot_io_base(u32::from(committed_pm), 64, 64)?;
        validate_snapshot_io_base(desired_sm, 16, 16)?;
        validate_snapshot_io_base(u32::from(committed_sm), 16, 16)?;
        validate_snapshot_memory_bar(desired_vga_lfb, vga_lfb_size)?;
        validate_snapshot_memory_bar(committed_vga_lfb, vga_lfb_size)?;
        validate_snapshot_memory_bar(
            desired_vga_mmio,
            super::vga::PCI_VGA_MMIO_SIZE,
        )?;
        validate_snapshot_memory_bar(
            committed_vga_mmio,
            super::vga::PCI_VGA_MMIO_SIZE,
        )?;

        validate_snapshot_mapping_flag(
            pci_ide_pending,
            u32::from(committed_bmdma),
            desired_bmdma,
        )?;
        validate_snapshot_mapping_flag(acpi_pm_pending, u32::from(committed_pm), desired_pm)?;
        validate_snapshot_mapping_flag(acpi_sm_pending, u32::from(committed_sm), desired_sm)?;
        if vga_pending != (committed_vga_mmio != desired_vga_mmio
            || committed_vga_lfb != desired_vga_lfb)
        {
            return Err(invalid_platform_snapshot(
                "snapshot VGA mapping flag and bases are incoherent",
            ));
        }
        Ok(())
    }
}

#[cfg(feature = "std")]
impl DeviceManager {
    /// Applies only snapshot-decoded machine effects.  Unlike the scheduler
    /// boundary path this never polls devices, drains work, or synthesizes
    /// timer requests.
    pub(crate) fn apply_snapshot_v3_restore(
        &mut self,
        io: &mut BxDevicesC,
        mem: &mut BxMemC<'_>,
        live_bmdma: u16,
        live_pm: u16,
        live_sm: u16,
        live_vga: super::vga::VgaSnapshotRestoreTarget,
        platform: PlatformSnapshotRestore,
        pci: super::pci_ide::PciIdeSnapshotTopology,
        acpi: super::acpi::AcpiSnapshotRestore,
        vga: super::vga::VgaSnapshotRestoreTarget,
    ) -> Result<()> {
        if self.bmdma_ports_base != live_bmdma
            || self.pm_ports_base != live_pm
            || self.sm_ports_base != live_sm
            || self.vga.snapshot_v3_committed_mapping_target() != live_vga
        {
            return Err(crate::Error::Io(Error::new(
                ErrorKind::InvalidData,
                "snapshot live mapping topology changed while restoring",
            )));
        }

        if (!self.pci2isa.elcr1_changed
            && self.pic.master.edge_level != self.pci2isa.elcr1)
            || (!self.pci2isa.elcr2_changed
                && self.pic.slave.edge_level != self.pci2isa.elcr2)
        {
            return Err(crate::Error::Io(Error::new(
                ErrorKind::InvalidData,
                "snapshot PIIX and PIC trigger modes disagree",
            )));
        }
        self.pic.set_mode(true, self.pci2isa.elcr1);
        self.pic.set_mode(false, self.pci2isa.elcr2);
        self.pci2isa.elcr1_changed = false;
        self.pci2isa.elcr2_changed = false;

        self.pci_bridge.apply_pam_to_memory(mem);
        self.pci_bridge.apply_smram_to_memory(mem);
        self.pci2isa.apply_bios_write_to_memory(mem);
        self.pam_needs_update = false;
        self.smram_needs_update = false;
        self.bios_write_needs_update = false;

        self.pci_ide.bmdma_base = pci.bmdma_base;
        self.register_pci_ide_bmdma_ports(io);
        self.pci_ide_bar4_needs_reregister = false;

        self.acpi.pm_base = acpi.pm_base;
        self.relocate_acpi_pm_ports(io);
        self.acpi_pm_needs_reregister = false;
        self.acpi.sm_base = acpi.sm_base;
        self.relocate_acpi_sm_ports(io);
        self.acpi_sm_needs_reregister = false;

        let device_id = crate::memory::MemoryDeviceId::Vga(&mut self.vga as *mut BxVgaC);
        let lfb_size = u64::from(self.vga.lfb_size());
        let old_lfb = (live_vga.lfb_base != 0).then_some((
            u64::from(live_vga.lfb_base),
            u64::from(live_vga.lfb_base) + lfb_size - 1,
        ));
        let new_lfb = (vga.lfb_base != 0).then_some((
            u64::from(vga.lfb_base),
            u64::from(vga.lfb_base) + lfb_size - 1,
        ));
        mem.relocate_memory_handlers(device_id, old_lfb, new_lfb)?;
        let old_mmio = (live_vga.mmio_base != 0).then_some((
            u64::from(live_vga.mmio_base),
            u64::from(live_vga.mmio_base) + u64::from(super::vga::PCI_VGA_MMIO_SIZE) - 1,
        ));
        let new_mmio = (vga.mmio_base != 0).then_some((
            u64::from(vga.mmio_base),
            u64::from(vga.mmio_base) + u64::from(super::vga::PCI_VGA_MMIO_SIZE) - 1,
        ));
        mem.relocate_memory_handlers(device_id, old_mmio, new_mmio)?;
        self.vga.commit_snapshot_v3_mapping_target(vga);
        self.vga_bar_needs_reregister = false;

        self.pci_conf_addr = platform.pci_conf_addr;
        io.pci_conf_addr = platform.pci_conf_addr;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::{
        core_i7_skylake::Corei7SkylakeX,
        instrumentation::{CpuSetupMode, X86Reg},
        CpuError,
    };
    use crate::emulator::{Emulator, EmulatorConfig};

    #[test]
    fn test_system_control_port() {
        let mut port = SystemControlPort::new();

        // Initially A20 is enabled
        assert!(port.a20_gate);
        assert!(port.reset_request.is_none());

        // Disable A20 (bit 1 = 0)
        port.write(0x00);
        assert!(port.a20_change_pending);
        assert!(!port.a20_gate);

        // Enable A20 again (bit 1 = 1)
        port.write(0x02);
        assert!(port.a20_change_pending);
        assert!(port.a20_gate);

        // Trigger reset (bit 0 = 1)
        port.write(0x01);
        assert_eq!(port.reset_request, Some(ResetReason::Software));
    }

    #[test]
    fn port92_repeated_value_does_not_latch_boundary() {
        // Bochs pc_system.cc set_enable_a20: only an actual gate transition
        // produces machine work; a same-value write is a no-op.
        let mut port = SystemControlPort::new();

        // Gate starts enabled; rewriting the enabled value latches nothing.
        port.write(0x02);
        assert!(!port.a20_change_pending);

        // A genuine transition latches boundary work.
        port.write(0x00);
        assert!(port.a20_change_pending);

        // Simulate the central boundary draining the request.
        port.a20_change_pending = false;

        // Repeating the now-current value latches nothing again.
        port.write(0x00);
        assert!(!port.a20_change_pending);

        // A reset-bearing write latches its reset without a spurious A20 latch.
        port.write(0x01);
        assert!(!port.a20_change_pending);
        assert_eq!(port.reset_request, Some(ResetReason::Software));
    }

    #[test]
    fn pit_irq0_mirrors_counter0_out_level() {
        // Finding #32d: IRQ0 must mirror counter 0's OUT LEVEL (Bochs
        // pit.cc irq_handler: raise on 0→1, lower on 1→0) — not a
        // synthesized lower+raise pulse.
        let mut pit = BxPitC::new();
        let mut pic = BxPicC::new();

        // Program counter 0: mode 2 (rate generator), count 10.
        pit.write(PIT_CONTROL, 0x34, 1, 0);
        pit.write(PIT_COUNTER0, 10, 1, 0);
        pit.write(PIT_COUNTER0, 0, 1, 0);
        assert_eq!(DeviceManager::service_pit_irq0(&mut pit, &mut pic), 0);

        // Ticks 1..=10 (pit82c54.cc clock_all domain): OUT pulses LOW.
        pit.clock_pit_ticks(10);
        assert_eq!(DeviceManager::service_pit_irq0(&mut pit, &mut pic), 0);
        assert_eq!(pic.master.irq_in[0], 0);

        // Tick 11: reload → OUT HIGH → IRQ0 raised and latched.
        pit.clock_pit_ticks(1);
        assert_eq!(DeviceManager::service_pit_irq0(&mut pit, &mut pic), 1);
        assert_eq!(pic.master.irq_in[0], 1);
        assert_ne!(pic.master.irr & 0x01, 0);

        // A full period in one batch: lower then raise (in order), ending
        // with the line high and a fresh edge latched.
        pit.clock_pit_ticks(10);
        assert_eq!(DeviceManager::service_pit_irq0(&mut pit, &mut pic), 1);
        assert_eq!(pic.master.irq_in[0], 1);
        assert_ne!(pic.master.irr & 0x01, 0);
    }

    #[test]
    fn pit_control_word_write_drives_irq0_edge() {
        // Finding #32d: OUT transitions caused by CONTROL-WORD writes must
        // reach the PIC (Bochs pit82c54.cc write_ctrl's set_OUT invokes the
        // out_handler on any transition).
        let mut pit = BxPitC::new();
        let mut pic = BxPicC::new();

        // Mode 0 control word forces OUT low (power-on OUT is high).
        pit.write(PIT_CONTROL, 0x30, 1, 0);
        assert_eq!(DeviceManager::service_pit_irq0(&mut pit, &mut pic), 0);
        assert_eq!(pic.master.irq_in[0], 0);

        // Count 5: terminal count at tick 6 → OUT high → IRQ0 raised.
        pit.write(PIT_COUNTER0, 5, 1, 0);
        pit.write(PIT_COUNTER0, 0, 1, 0);
        pit.clock_pit_ticks(6);
        assert_eq!(DeviceManager::service_pit_irq0(&mut pit, &mut pic), 1);
        assert_eq!(pic.master.irq_in[0], 1);
        assert_ne!(pic.master.irr & 0x01, 0);

        // A new mode 0 control word forces OUT high→low: the PIC must see
        // the lower (line drops, IRR bit cleared) purely from the
        // control-word write.
        pit.write(PIT_CONTROL, 0x30, 1, 0);
        assert_eq!(DeviceManager::service_pit_irq0(&mut pit, &mut pic), 0);
        assert_eq!(pic.master.irq_in[0], 0);
        assert_eq!(pic.master.irr & 0x01, 0);
    }

    // DeviceManager is large (VGA text buffers etc.); build it on a big stack,
    // like the cet.rs tests, to avoid overflowing the small default test stack.
    fn on_big_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }
    const GUEST_TEST_CODE: u64 = 0x1000;

    fn guest_emulator(pci_vga: bool) -> Box<Emulator<'static, Corei7SkylakeX>> {
        let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
            EmulatorConfig::default(),
            CpuSetupMode::FlatProtected32,
        )
        .unwrap();
        if pci_vga {
            emu.device_manager.vga.enable_pci();
        }
        // `new_with_mode` deliberately skips device registration.
        emu.devices.init(&mut emu.memory).unwrap();
        emu.device_manager
            .init(&mut emu.devices, &mut emu.memory)
            .unwrap();
        emu
    }

    fn guest_pci_bar_write(
        emu: &mut Emulator<'static, Corei7SkylakeX>,
        devfunc: u8,
        register: u8,
        value: u32,
    ) -> crate::cpu::Result<u64> {
        let conf_addr =
            0x8000_0000u32 | (u32::from(devfunc) << 8) | u32::from(register & 0xFC);
        let code_address = GUEST_TEST_CODE
            + 0x1000
            + u64::from(register) * 0x100
            + ((u64::from(value) >> 20) & 0x0fff) * 0x20;
        let mut code = Vec::with_capacity(24);
        // mov edx,0xcf8; mov eax,conf_addr; out dx,eax;
        // mov edx,0xcfc; mov eax,value; out dx,eax.
        code.extend_from_slice(&[0xBA, 0xF8, 0x0C, 0x00, 0x00, 0xB8]);
        code.extend_from_slice(&conf_addr.to_le_bytes());
        code.push(0xEF);
        code.extend_from_slice(&[0xBA, 0xFC, 0x0C, 0x00, 0x00, 0xB8]);
        code.extend_from_slice(&value.to_le_bytes());
        code.push(0xEF);
        emu.virt_write(code_address, &code).unwrap();
        emu.reg_write(X86Reg::Rip, code_address);
        unsafe { emu.run_cpu_batch(64) }
    }

    fn guest_inb(emu: &mut Emulator<'static, Corei7SkylakeX>, port: u16) -> crate::cpu::Result<u8> {
        let code = [
            0xBA,
            port as u8,
            (port >> 8) as u8,
            0x00,
            0x00,
            0xEC, // in al,dx
        ];
        emu.virt_write(GUEST_TEST_CODE, &code).unwrap();
        emu.reg_write(X86Reg::Rip, GUEST_TEST_CODE);
        unsafe { emu.run_cpu_batch(2) }?;
        assert_eq!(emu.devices.last_io_read_port, port);
        Ok(emu.reg_read(X86Reg::Rax) as u8)
    }

    fn guest_memory_read(
        emu: &mut Emulator<'static, Corei7SkylakeX>,
        address: u32,
    ) -> crate::cpu::Result<u64> {
        let code_address =
            GUEST_TEST_CODE + 0x40000 + ((u64::from(address) >> 20) & 0xff) * 0x10;
        let mut code = Vec::with_capacity(6);
        code.extend_from_slice(&[0x8A, 0x05]); // mov al,byte ptr [disp32]
        code.extend_from_slice(&address.to_le_bytes());
        emu.virt_write(code_address, &code).unwrap();
        emu.reg_write(X86Reg::Rip, code_address);
        unsafe { emu.run_cpu_batch(1) }
    }

    #[test]
    fn vga_bar_moves_are_visible_before_next_access() {
        on_big_stack(|| {
            use crate::memory::MemoryDeviceId;

            let mut emu = guest_emulator(true);
            let lfb_size = u64::from(emu.device_manager.vga.lfb_size());
            let initial_lfb =
                emu.device_manager.vga.pci_read(0x10, 4) & !(emu.device_manager.vga.lfb_size() - 1);

            // Guest OUTs route through BxDevicesC; run_cpu_batch must consume
            // the requested boundary before this following guest memory access.
            guest_pci_bar_write(&mut emu, 0x10, 0x10, 0xE800_0000).unwrap();
            assert!(emu.device_manager.vga.peek_pending_lfb_relocate().is_none());
            guest_memory_read(&mut emu, 0xE800_0000).unwrap();
            assert!(
                emu.memory
                    .register_memory_handlers(
                        MemoryDeviceId::None,
                        u64::from(initial_lfb),
                        u64::from(initial_lfb) + lfb_size - 1,
                    )
                    .is_ok(),
                "the old LFB must be free before the following guest access"
            );
            emu.memory
                .unregister_memory_handlers(
                    MemoryDeviceId::None,
                    u64::from(initial_lfb),
                    u64::from(initial_lfb) + lfb_size - 1,
                )
                .unwrap();

            guest_pci_bar_write(&mut emu, 0x10, 0x18, 0xF000_0000).unwrap();
            guest_memory_read(&mut emu, 0xF000_0500).unwrap();
            assert!(emu.device_manager.vga.is_mmio_addr(0xF000_0500));

            guest_pci_bar_write(&mut emu, 0x10, 0x18, 0xF100_0000).unwrap();
            guest_memory_read(&mut emu, 0xF100_0500).unwrap();
            assert!(!emu.device_manager.vga.is_mmio_addr(0xF000_0500));
            assert!(emu.device_manager.vga.is_mmio_addr(0xF100_0500));
            assert!(
                emu.memory
                    .register_memory_handlers(MemoryDeviceId::None, 0xF000_0000, 0xF000_0FFF)
                    .is_ok(),
                "moving BAR2 must unregister the previous MMIO window"
            );

            // A conflicting target must fail atomically: the committed LFB
            // remains live, while both relocation latches remain queued for a
            // later retry.
            let committed_lfb =
                emu.device_manager.vga.pci_read(0x10, 4) & !(emu.device_manager.vga.lfb_size() - 1);
            let failed_target = 0xD000_0000u32;
            emu.memory
                .register_memory_handlers(
                    MemoryDeviceId::None,
                    u64::from(failed_target),
                    u64::from(failed_target) + lfb_size - 1,
                )
                .unwrap();
            let error =
                guest_pci_bar_write(&mut emu, 0x10, 0x10, failed_target).unwrap_err();
            assert!(matches!(error, CpuError::MachineBoundaryFailed));
            assert_eq!(
                emu.device_manager.vga.peek_pending_lfb_relocate(),
                Some((committed_lfb, failed_target))
            );
            assert!(emu.device_manager.vga_bar_needs_reregister);
            assert!(
                emu.memory
                    .register_memory_handlers(
                        MemoryDeviceId::None,
                        u64::from(committed_lfb),
                        u64::from(committed_lfb) + lfb_size - 1,
                    )
                    .is_err(),
                "the old LFB handler must survive a failed relocation"
            );
        });
    }

    #[test]
    fn bmdma_prd_and_payload_work_in_swapped_block() {
        on_big_stack(|| {
            use crate::iodev::harddrv::DeviceType;
            use crate::memory::{BxMemC, BxMemoryStubC};
            use crate::pc_system::{BxPcSystemC, TimerOwner};

            let mut dm = DeviceManager::new();
            let stub = BxMemoryStubC::create_and_init(4 << 20, 1 << 20, 1 << 20).unwrap();
            let mut mem = BxMemC::new(stub, false);
            mem.set_a20_mask(u64::MAX);
            let mut pcs = BxPcSystemC::new();

            // Register the channel-0 BM-DMA timer as the emulator would.
            let handle = pcs
                .register_timer(TimerOwner::PciIdeCh0, 0, false, false, "test bmdma")
                .unwrap();
            dm.pci_ide.bmdma[0].timer_index = Some(handle);

            // In-memory disk: 2 sectors with a recognizable pattern.
            let disk: &'static [u8] =
                alloc::vec::Vec::leak((0..1024u32).map(|i| (i % 251) as u8).collect());
            {
                let drive = &mut dm.harddrv.channels[0].drives[0];
                drive.device_type = DeviceType::Disk;
                drive.attach_data_ref(disk);
            }

            // BIOS assigns BAR4 → BM-DMA present.
            assert!(dm.pci_ide.pci_write(0x20, 0x0000_C001, 4));
            assert!(dm.pci_ide.bmdma_present());
            // Guest builds a single-entry PRD table in one swapped block and
            // targets a second swapped block with the disk payload.
            let mut prd = [0u8; 8];
            prd[0..4].copy_from_slice(&0x0030_0000u32.to_le_bytes());
            prd[4..8].copy_from_slice(&(1024u32 | 0x8000_0000).to_le_bytes());
            assert_eq!(mem.write_ram(&[], 0x0020_0000, &prd).unwrap(), prd.len());
            mem.smc_mark_icache_mask(0x0030_0000, u32::MAX);
            let before_smc = mem.smc_seq_next();

            // Guest issues READ DMA (LBA 0, 2 sectors) via the port interface.
            {
                let DeviceManager {
                    ref mut harddrv,
                    ref mut pic,
                    ref mut pci_ide,
                    ..
                } = dm;
                harddrv.write(0x1F2, 2, 1, pic, pci_ide); // sector count
                harddrv.write(0x1F3, 0, 1, pic, pci_ide); // LBA 7:0
                harddrv.write(0x1F4, 0, 1, pic, pci_ide); // LBA 15:8
                harddrv.write(0x1F5, 0, 1, pic, pci_ide); // LBA 23:16
                harddrv.write(0x1F6, 0xE0, 1, pic, pci_ide); // LBA mode, drive 0
                harddrv.write(0x1F7, 0xC8, 1, pic, pci_ide); // READ DMA
            }
            // Bochs harddrv.cc: READ DMA arms the seek timer; only its
            // deadline (seek_timer) signals bmdma_start_transfer.
            assert!(
                !dm.pci_ide.bmdma[0].data_ready,
                "READ DMA must not start BM-DMA before the seek deadline"
            );
            assert!(dm.harddrv.take_pending_seek_arm(0, 0).is_some());
            {
                let DeviceManager {
                    ref mut harddrv,
                    ref mut pic,
                    ref mut pci_ide,
                    ..
                } = dm;
                harddrv.seek_timer(0b00, pic, pci_ide);
            }
            assert!(
                dm.pci_ide.bmdma[0].data_ready,
                "seek_timer must signal bmdma_start_transfer"
            );

            dm.pci_ide.bmdma_write(0xC004, 0x0020_0000, 4);
            dm.pci_ide.bmdma_write(0xC000, 0x09, 1);
            let arm = dm.pci_ide.take_pending_timer_arm(0);
            assert_eq!(arm, Some(1));
            pcs.activate_timer_usec(handle, 1, false).unwrap();

            // Timer fires: single PRD with EOT completes the transfer.
            dm.pci_ide_timer(0, &mut pcs, &mut mem, &[]);

            let mut guest_payload = [0; 1024];
            assert_eq!(
                mem.read_ram(&[], 0x0030_0000, &mut guest_payload).unwrap(),
                guest_payload.len()
            );
            assert_eq!(
                guest_payload.as_slice(),
                disk,
                "disk data must land at the swapped PRD target"
            );
            assert!(
                mem.smc_seq_next() > before_smc,
                "BM-DMA guest writes must emit SMC invalidations"
            );
            let status = dm.pci_ide.bmdma[0].status;
            assert_eq!(status & 0x01, 0, "engine active bit must clear on EOT");
            assert_ne!(status & 0x04, 0, "IRQ bit must set on EOT");
            assert_eq!(dm.pci_ide.bmdma[0].prd_current, 0);
            let drive = &dm.harddrv.channels[0].drives[0];
            assert!(
                drive.controller.interrupt_pending,
                "bmdma_complete must raise the drive interrupt"
            );
        });
    }

    #[test]
    fn ide_bar4_is_live_on_the_next_instruction() {
        on_big_stack(|| {
            let mut emu = guest_emulator(false);

            guest_pci_bar_write(&mut emu, 0x09, 0x20, 0x0000_C001).unwrap();
            assert_eq!(emu.devices.read_handlers[0xC000].device_id, DeviceId::Pci);
            assert_eq!(emu.devices.write_handlers[0xC004].device_id, DeviceId::Pci);
            let reads_before = emu.devices.diag_io_reads;
            let _ = guest_inb(&mut emu, 0xC000).unwrap();
            assert_eq!(emu.devices.diag_io_reads, reads_before + 1);

            guest_pci_bar_write(&mut emu, 0x09, 0x20, 0x0000_D001).unwrap();
            assert_eq!(emu.devices.read_handlers[0xC000].device_id, DeviceId::None);
            assert_eq!(emu.devices.write_handlers[0xC004].device_id, DeviceId::None);
            assert_eq!(emu.devices.read_handlers[0xD000].device_id, DeviceId::Pci);
            assert_eq!(emu.devices.write_handlers[0xD004].device_id, DeviceId::Pci);
            let reads_before = emu.devices.diag_io_reads;
            let _ = guest_inb(&mut emu, 0xD000).unwrap();
            assert_eq!(emu.devices.diag_io_reads, reads_before + 1);
        });
    }

    #[test]
    fn bmdma_start_queues_timer_request_at_issuing_epoch() {
        on_big_stack(|| {
            use crate::iodev::{DeviceTimerOwner, TimerRequest};

            let mut dm = DeviceManager::new();
            let mut io = BxDevicesC::new();

            // BAR4 assigned; BM-DMA ports registered on the I/O bus.
            assert!(dm.pci_ide.pci_write(0x20, 0x0000_C001, 4));
            dm.register_pci_ide_bmdma_ports(&mut io);

            io.set_device_manager(core::ptr::NonNull::from(&mut dm));

            // Guest programs DTPR and starts the engine. Bochs requests the
            // one-tick BM-DMA callback at this issuing instruction's epoch.
            io.outp(0xC004, 0x8000, 4, 41);
            io.outp(0xC000, 0x09, 1, 41);
            io.clear_device_manager();

            assert_eq!(
                dm.pci_ide.take_pending_timer_arm(0),
                None,
                "I/O transport must drain the IDE producer"
            );
            assert!(io.take_scheduler_boundary_requested());
            let requests = io.take_timer_requests();
            assert_eq!(
                requests.get(DeviceTimerOwner::PciIdeCh0),
                TimerRequest::Activate {
                    deadline_ticks: 42,
                    period_ticks: 1,
                    continuous: false,
                }
            );
            assert_eq!(
                requests.get(DeviceTimerOwner::PciIdeCh1),
                TimerRequest::Unchanged
            );
        });
    }

    #[test]
    fn vga_pci_bar2_commit_registers_mmio_window() {
        on_big_stack(|| {
            use crate::memory::{BxMemC, BxMemoryStubC, MemoryDeviceId};

            let mut dm = DeviceManager::new();
            dm.vga.enable_pci();
            let stub = BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
            let mut mem = BxMemC::new(stub, false);

            let change = dm.vga.pci_write(0x18, 0xF000_0000, 4);
            assert!(change.mmio);
            dm.vga_bar_needs_reregister = true;
            dm.reregister_vga_bars(&mut mem).unwrap();

            assert!(dm.vga.is_mmio_addr(0xF000_0500));
            assert!(
                mem.register_memory_handlers(MemoryDeviceId::None, 0xF000_0000, 0xF000_0FFF)
                    .is_err(),
                "BAR2 MMIO window must be registered"
            );
        });
    }

    // ─── Finding #2: port 0xCF9 (PIIX3 reset control) registration/dispatch ───

    #[test]
    fn pci_reset_port_cf9_registers_and_dispatches_through_io_bus() {
        on_big_stack(|| {
            let mut dm = DeviceManager::new();
            let mut io = BxDevicesC::new();
            dm.register_pci_handlers(&mut io);

            assert_eq!(
                io.write_handlers[0x0CF9].device_id,
                DeviceId::Pci,
                "port 0xCF9 write must be registered (Bochs pci2isa.cc init)"
            );
            assert_eq!(
                io.read_handlers[0x0CF9].device_id,
                DeviceId::Pci,
                "port 0xCF9 read must be registered (Bochs pci2isa.cc init)"
            );

            io.set_device_manager(core::ptr::NonNull::from(&mut dm));
            // Set reset type = hardware (bit1), then trigger (bit1|bit2) —
            // Bochs pci2isa.cc write case 0x0cf9.
            io.outp(0x0CF9, 0x02, 1, 0);
            io.outp(0x0CF9, 0x06, 1, 0);
            io.clear_device_manager();

            assert_eq!(
                dm.pci2isa.reset_request,
                Some(ResetReason::Hardware),
                "OUT 0xCF9,0x06 with reset_type=hardware must request a hardware reset"
            );

            io.set_device_manager(core::ptr::NonNull::from(&mut dm));
            let value = io.inp(0x0CF9, 1, 0);
            io.clear_device_manager();
            assert_eq!(
                value, 0x02,
                "read of 0xCF9 must return the stored pci_reset value, not the unhandled sentinel"
            );
        });
    }

    // ─── Finding #3: ELCR writes must reach BxPicC::set_mode ───

    #[test]
    fn elcr_write_drains_into_pic_set_mode() {
        on_big_stack(|| {
            let mut dm = DeviceManager::new();
            let mut io = BxDevicesC::new();
            dm.register_pci_handlers(&mut io);

            io.set_device_manager(core::ptr::NonNull::from(&mut dm));
            // ELCR1 bit5 -> IRQ5 level-triggered (Bochs pci2isa.cc write case
            // 0x04d0: DEV_pic_set_mode(1, elcr1)).
            io.outp(0x04D0, 0x20, 1, 0);
            io.clear_device_manager();

            assert_eq!(dm.pci2isa.elcr1, 0x20);
            assert!(
                !dm.pci2isa.elcr1_changed,
                "elcr1_changed must be drained by the write dispatch"
            );
            assert_eq!(
                dm.pic.master.edge_level, 0x20,
                "pic.set_mode(true, elcr1) must mirror ELCR1 into master edge_level"
            );

            io.set_device_manager(core::ptr::NonNull::from(&mut dm));
            // ELCR2 bit2 -> IRQ10 level-triggered (Bochs pci2isa.cc write case
            // 0x04d1: DEV_pic_set_mode(0, elcr2)).
            io.outp(0x04D1, 0x04, 1, 0);
            io.clear_device_manager();

            assert_eq!(dm.pci2isa.elcr2, 0x04);
            assert!(
                !dm.pci2isa.elcr2_changed,
                "elcr2_changed must be drained by the write dispatch"
            );
            assert_eq!(
                dm.pic.slave.edge_level, 0x04,
                "pic.set_mode(false, elcr2) must mirror ELCR2 into slave edge_level"
            );
        });
    }

    #[test]
    fn level_triggered_irq_keeps_irr_set_after_iac_ack() {
        on_big_stack(|| {
            let mut dm = DeviceManager::new();
            let mut io = BxDevicesC::new();
            dm.register_pci_handlers(&mut io);

            // Mark IRQ5 level-triggered via the real ELCR1 port write path.
            io.set_device_manager(core::ptr::NonNull::from(&mut dm));
            io.outp(0x04D0, 0x20, 1, 0);
            io.clear_device_manager();
            assert_eq!(dm.pic.master.edge_level, 0x20);

            // Unmask IRQ5 and assert the line (a level-triggered device holds
            // the line high until it services the condition).
            dm.pic.master.imr &= !(1 << 5);
            dm.pic.raise_irq(5);
            assert_ne!(
                dm.pic.master.irr & (1 << 5),
                0,
                "IRR must be set once the line is raised"
            );

            let vector = dm.pic.iac();
            assert_eq!(vector, dm.pic.master.interrupt_offset + 5);

            // Level-triggered: IRR must stay set after ack because the guest
            // hasn't lowered the line yet (Bochs pic.cc IAC edge_level gate).
            assert_ne!(
                dm.pic.master.irr & (1 << 5),
                0,
                "level-triggered IRQ must keep IRR set after ack"
            );
        });
    }

    // ─── Finding #21: common PCI config-space read-only filter ───

    #[test]
    fn pci_config_write_blocks_common_readonly_bytes_but_allows_bar_and_command() {
        on_big_stack(|| {
            fn conf_addr(devfunc: u8, reg: u8) -> u32 {
                0x8000_0000u32 | ((devfunc as u32) << 8) | (reg as u32 & 0xFC)
            }

            let mut dm = DeviceManager::new();
            const PIIX3: u8 = 0x08;
            const PCI_IDE: u8 = 0x09;

            // Vendor/device ID (0x00-0x03) must stay read-only.
            let vendor_before = dm.pci2isa.pci_conf[0x00];
            dm.pci_conf_addr = conf_addr(PIIX3, 0x00);
            dm.pci_write(0x0CFC, 0xDEAD_BEEF, 4);
            assert_eq!(
                dm.pci2isa.pci_conf[0x00], vendor_before,
                "vendor ID must stay read-only"
            );

            // Revision + class code (0x08-0x0B) must stay read-only.
            let class_before = dm.pci2isa.pci_conf[0x0A];
            dm.pci_conf_addr = conf_addr(PIIX3, 0x08);
            dm.pci_write(0x0CFC, 0xFFFF_FFFF, 4);
            assert_eq!(
                dm.pci2isa.pci_conf[0x0A], class_before,
                "class code must stay read-only"
            );

            // Header type (0x0E) must stay read-only when a write STARTS
            // there (Bochs gates only on the starting offset).
            let htype_before = dm.pci2isa.pci_conf[0x0E];
            dm.pci_conf_addr = conf_addr(PIIX3, 0x0C);
            dm.pci_write(0x0CFE, 0xFF, 1); // offset 2 -> reg_addr 0x0C+2 = 0x0E
            assert_eq!(
                dm.pci2isa.pci_conf[0x0E], htype_before,
                "header type must stay read-only when the write starts there"
            );

            // Bochs parity quirk: a dword write STARTING at 0x0C (cache-line
            // size, writable) is NOT gated at all, so it reaches the device
            // unfiltered even though it spills into the read-only header-type
            // byte at 0x0E -- Bochs checks only the starting offset, there is
            // no per-byte mid-span protection.
            dm.pci_conf_addr = conf_addr(PIIX3, 0x0C);
            dm.pci_write(0x0CFC, 0xFFFF_FFFF, 4);
            assert_eq!(
                dm.pci2isa.pci_conf[0x0C], 0xFF,
                "cache-line size byte must remain writable"
            );
            assert_eq!(
                dm.pci2isa.pci_conf[0x0D], 0xFF,
                "latency timer byte must remain writable"
            );
            assert_eq!(
                dm.pci2isa.pci_conf[0x0E], 0xFF,
                "a write starting at a writable offset reaches the device \
                 unfiltered even if it spills into a read-only byte (Bochs \
                 devices.cc pci_write_handler_common start-offset-only gate)"
            );

            // Interrupt pin (0x3D) must stay read-only when a write STARTS
            // there.
            let intpin_before = dm.pci2isa.pci_conf[0x3D];
            dm.pci_conf_addr = conf_addr(PIIX3, 0x3C);
            dm.pci_write(0x0CFD, 0xFF, 1); // offset 1 -> reg_addr 0x3C+1 = 0x3D
            assert_eq!(
                dm.pci2isa.pci_conf[0x3D], intpin_before,
                "interrupt pin must stay read-only when the write starts there"
            );

            // Bochs special-cases a write STARTING at 0x3C (interrupt line):
            // only that single byte is stored regardless of io_len, so a
            // dword write starting at 0x3C must NOT reach 0x3D/0x3E/0x3F.
            let byte3e_before = dm.pci2isa.pci_conf[0x3E];
            let byte3f_before = dm.pci2isa.pci_conf[0x3F];
            dm.pci_conf_addr = conf_addr(PIIX3, 0x3C);
            dm.pci_write(0x0CFC, 0xFFFF_FFFF, 4);
            assert_eq!(
                dm.pci2isa.pci_conf[0x3C], 0xFF,
                "interrupt line must remain writable"
            );
            assert_eq!(
                dm.pci2isa.pci_conf[0x3D], intpin_before,
                "a write starting at 0x3C must not reach the interrupt pin \
                 byte -- Bochs clamps it to a single-byte store"
            );
            assert_eq!(
                dm.pci2isa.pci_conf[0x3E], byte3e_before,
                "a write starting at 0x3C must not reach byte 0x3E"
            );
            assert_eq!(
                dm.pci2isa.pci_conf[0x3F], byte3f_before,
                "a write starting at 0x3C must not reach byte 0x3F"
            );

            // Command register (0x04) must remain writable through the filter.
            dm.pci_conf_addr = conf_addr(PIIX3, 0x04);
            dm.pci_write(0x0CFC, 0x0F, 1);
            assert_eq!(
                dm.pci2isa.pci_conf[0x04], 0x0F,
                "command register must remain writable through the filter"
            );

            // BAR4 (PCI IDE BM-DMA base, 0x20) must remain writable through
            // the filter — it is nowhere near the read-only byte set.
            dm.pci_conf_addr = conf_addr(PCI_IDE, 0x20);
            dm.pci_write(0x0CFC, 0x0000_C001, 4);
            assert!(
                dm.pci_ide.bmdma_present(),
                "BAR4 write must reach the device through the filter"
            );
        });
    }

    // ─── Finding #8: SMRAM control register (0x72) drives memory shadowing ───

    #[test]
    fn smram_register_write_defers_then_applies_to_memory() {
        on_big_stack(|| {
            use crate::memory::{BxMemC, BxMemoryStubC};

            fn conf_addr(devfunc: u8, reg: u8) -> u32 {
                0x8000_0000u32 | ((devfunc as u32) << 8) | (reg as u32 & 0xFC)
            }

            let mut dm = DeviceManager::new();
            let mut io = BxDevicesC::new();
            let stub = BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
            let mut mem = BxMemC::new(stub, false);

            // Host bridge is devfunc 0x00; SMRAM control lives at 0x72
            // (reg 0x70 + offset 2, since 0x72 & 0xFC == 0x70).
            dm.pci_conf_addr = conf_addr(0x00, 0x72);

            // SMRAME|DOPEN (0x48): SMRAM open, unrestricted.
            dm.pci_write(0xCFE, 0x48, 1);
            assert!(
                dm.smram_needs_update,
                "writing 0x72 must set the deferred flag"
            );
            dm.apply_pending_machine_boundary(&mut io, &mut mem)
                .unwrap();
            assert!(!dm.smram_needs_update, "drain must clear the flag");
            assert_eq!(mem.smram_state(), (true, true, false));

            // SMRAME off (0x02): SMRAM fully disabled.
            dm.pci_conf_addr = conf_addr(0x00, 0x72);
            dm.pci_write(0xCFE, 0x02, 1);
            assert!(dm.smram_needs_update);
            dm.apply_pending_machine_boundary(&mut io, &mut mem)
                .unwrap();
            assert_eq!(mem.smram_state(), (false, false, false));

            // Illegal DOPEN&&DCLS combo (SMRAME|DOPEN|DCLS = 0x68): Bochs
            // BX_PANICs; rusty_box must not crash the host on a guest
            // register write and instead treats it as disabled.
            dm.pci_conf_addr = conf_addr(0x00, 0x72);
            dm.pci_write(0xCFE, 0x68, 1);
            assert!(dm.smram_needs_update);
            dm.apply_pending_machine_boundary(&mut io, &mut mem)
                .unwrap();
            assert_eq!(mem.smram_state(), (false, false, false));
        });
    }

    // ─── Finding #20a: PAM config re-applied to memory after guest reset ─────
    //
    // Bochs pci.cc bx_pci_bridge_c::reset() zeroes the PAM config bytes AND
    // re-applies memory type for every PAM area directly
    // (DEV_mem_set_memory_type loop), so the shadow-RAM routing tracks the
    // reset PAM config immediately. rusty_box queues `pam_needs_update` for
    // the shared machine-boundary drain.

    #[test]
    fn pam_config_reapplied_to_memory_after_reset() {
        on_big_stack(|| {
            use crate::memory::{BxMemC, BxMemoryStubC};

            fn conf_addr(devfunc: u8, reg: u8) -> u32 {
                0x8000_0000u32 | ((devfunc as u32) << 8) | (reg as u32 & 0xFC)
            }

            let mut dm = DeviceManager::new();
            let mut io = BxDevicesC::new();
            let stub = BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
            let mut mem = BxMemC::new(stub, false);

            // PAM reg 0x59 controls area 12 (F0000): bit4=read, bit5=write.
            // Enable read+write shadow RAM on it. 0x59 & 0xFC == 0x58, so the
            // byte lands at data port 0xCFC + (0x59 - 0x58) == 0xCFD.
            dm.pci_conf_addr = conf_addr(0x00, 0x59);
            dm.pci_write(0xCFD, 0x30, 1); // area12: read=1, write=1
            assert!(dm.pam_needs_update);
            dm.apply_pending_machine_boundary(&mut io, &mut mem)
                .unwrap();
            assert_eq!(
                mem.memory_type(12, 1),
                true,
                "area 12 (F0000) write-shadow must be enabled before reset"
            );

            // Guest hardware reset: PAM config bytes go back to 0, and the
            // memory shadow state must track that -- Bochs re-applies it
            // synchronously inside bx_pci_bridge_c::reset(); rusty_box must
            // defer-and-drain the same way pci_write's PAM branch does.
            dm.reset(ResetReason::Hardware).unwrap();
            assert_eq!(
                dm.pci_bridge.pci_conf[0x59], 0x00,
                "reset must zero the PAM config byte"
            );
            assert!(
                dm.pam_needs_update,
                "reset must mark PAM for re-application to memory"
            );
            dm.apply_pending_machine_boundary(&mut io, &mut mem)
                .unwrap();
            assert!(!dm.pam_needs_update, "drain must clear the flag");
            assert_eq!(
                mem.memory_type(12, 1),
                false,
                "area 12 (F0000) write-shadow must be back to the reset default"
            );
        });
    }
    #[test]
    fn pci_out_applies_pam_smram_and_xbcs_before_resume() {
        on_big_stack(|| {
            use crate::memory::{BxMemC, BxMemoryStubC};

            fn conf_addr(devfunc: u8, reg: u8) -> u32 {
                0x8000_0000u32 | ((devfunc as u32) << 8) | (reg as u32 & 0xFC)
            }

            let mut dm = DeviceManager::new();
            let mut io = BxDevicesC::new();
            dm.register_pci_handlers(&mut io);
            let stub = BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
            let mut mem = BxMemC::new(stub, false);

            io.set_device_manager(core::ptr::NonNull::from(&mut dm));
            io.outp(0x0CF8, conf_addr(0x00, 0x59), 4, 11);
            io.outp(0x0CFD, 0x30, 1, 11);
            io.outp(0x0CF8, conf_addr(0x00, 0x72), 4, 11);
            io.outp(0x0CFE, 0x48, 1, 11);
            io.outp(0x0CF8, conf_addr(0x08, 0x4E), 4, 11);
            io.outp(0x0CFE, 0x04, 1, 11);
            io.clear_device_manager();

            assert!(io.take_scheduler_boundary_requested());
            assert!(dm.pam_needs_update);
            assert!(dm.smram_needs_update);
            assert!(dm.bios_write_needs_update);
            assert!(!mem.memory_type(12, 1));
            assert_eq!(mem.smram_state(), (false, false, false));
            assert!(!mem.bios_write_enabled());

            dm.apply_pending_machine_boundary(&mut io, &mut mem)
                .unwrap();
            assert!(mem.memory_type(12, 1));
            assert_eq!(mem.smram_state(), (true, true, false));
            assert!(mem.bios_write_enabled());
            assert!(!dm.has_pending_machine_boundary());
        });
    }

    #[test]
    fn acpi_bar_moves_unregister_old_ranges_synchronously() {
        on_big_stack(|| {
            let mut emu = guest_emulator(false);

            guest_pci_bar_write(&mut emu, 0x0B, 0x40, 0x0000_B001).unwrap();
            guest_pci_bar_write(&mut emu, 0x0B, 0x90, 0x0000_B101).unwrap();
            assert_eq!(emu.devices.read_handlers[0xB000].device_id, DeviceId::Acpi);
            assert_eq!(emu.devices.write_handlers[0xB100].device_id, DeviceId::Acpi);
            let _ = guest_inb(&mut emu, 0xB000).unwrap();
            let _ = guest_inb(&mut emu, 0xB100).unwrap();

            guest_pci_bar_write(&mut emu, 0x0B, 0x40, 0x0000_C001).unwrap();
            guest_pci_bar_write(&mut emu, 0x0B, 0x90, 0x0000_C101).unwrap();
            assert_eq!(emu.devices.read_handlers[0xB000].device_id, DeviceId::None);
            assert_eq!(emu.devices.write_handlers[0xB100].device_id, DeviceId::None);
            assert_eq!(emu.devices.read_handlers[0xC000].device_id, DeviceId::Acpi);
            assert_eq!(emu.devices.write_handlers[0xC100].device_id, DeviceId::Acpi);
            let _ = guest_inb(&mut emu, 0xC000).unwrap();
            let _ = guest_inb(&mut emu, 0xC100).unwrap();
        });
    }
    #[test]
    fn platform_snapshot_restores_port92_a20_and_pending_reset() {
        on_big_stack(|| {
            use crate::memory::{BxMemC, BxMemoryStubC};
            use crate::snapshot::SnapshotReader;
            use std::io::Cursor;

            let mut source = DeviceManager::new();
            source.port92.write(0x01);
            assert!(source.port92.reset_request.is_some());
            source.pci_conf_addr = 0x8000_0900;
            source.pam_needs_update = true;

            let mut saved = Vec::new();
            source.save_snapshot_v3_body(&mut saved).unwrap();

            let mut target = DeviceManager::new();
            let mut io = BxDevicesC::new();
            let mut mem = BxMemC::new(
                BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap(),
                false,
            );
            io.init(&mut mem).unwrap();
            target.register_pci_handlers(&mut io);
            target.register_fw_cfg_handlers(&mut io);
            io.pci_conf_addr = 0xA000_0000;

            assert_eq!(io.read_handlers[PORT_92H as usize].device_id, DeviceId::Port92);
            assert_eq!(io.write_handlers[0x0CF8].device_id, DeviceId::Pci);
            assert_eq!(io.read_handlers[0x0511].device_id, DeviceId::FwCfg);

            let mut reader =
                SnapshotReader::new(Cursor::new(saved.clone()), saved.len() as u64).unwrap();
            let restored = target.restore_snapshot_v3_body(&mut reader).unwrap();
            reader.finish_exact().unwrap();

            assert_eq!(target.port92.value, 0x01);
            assert_eq!(target.port92.read(), 0x00, "the guest-visible A20 gate is disabled");
            assert_eq!(restored.port92_a20_gate, false);
            assert!(restored.port92_a20_change_pending);
            assert_eq!(restored.port92_reset_request, Some(ResetReason::Software));
            assert!(
                target.has_pending_machine_boundary(),
                "restored A20 and reset effects must remain queued for the machine boundary"
            );
            assert_eq!(restored.pci_conf_addr, source.pci_conf_addr);
            assert!(restored.pam_needs_update);
            assert_eq!(
                io.pci_conf_addr, 0xA000_0000,
                "component decode must not overwrite the live I/O dispatch latch"
            );

            assert_eq!(io.read_handlers[PORT_92H as usize].device_id, DeviceId::Port92);
            assert_eq!(io.write_handlers[0x0CF8].device_id, DeviceId::Pci);
            assert_eq!(io.read_handlers[0x0511].device_id, DeviceId::FwCfg);
        });
    }

}
