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

use crate::{cpu::ResetReason, memory::BxMemC, pc_system::BxPcSystemC, Result};

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
/// RAM. Bochs pci_ide.cc timer: DEV_MEM_READ_PHYSICAL(prd_current, 4, ...) x2.
/// Reads past the end of RAM zero-fill (deterministic guest-error behavior).
fn read_bmdma_prd(mem: &crate::memory::BxMemC<'_>, prd_addr: u32) -> (u32, u32) {
    let mut raw = [0u8; 8];
    let bytes = mem.peek_ram(prd_addr as usize, 8);
    raw[..bytes.len()].copy_from_slice(bytes);
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
    /// Diagnostic: total tick() calls
    pub diag_tick_count: u64,
    /// Diagnostic: total usec passed to tick()
    pub diag_total_usec: u64,
    /// Diagnostic: iac vector histogram [0..256]
    pub diag_vector_hist: [u32; 256],
    /// Pointer to BxMemC for fw_cfg DMA. Set temporarily during CPU execution.
    pub(crate) mem_ptr: Option<core::ptr::NonNull<BxMemC<'static>>>,
    /// Pointer to BxPcSystemC so I/O writes can arm timers within the same
    /// instruction (Bochs pci_ide.cc write: bx_pc_system.activate_timer).
    /// Same lifecycle as `mem_ptr`: set before CPU execution, cleared after.
    pub(crate) pcs_ptr: Option<core::ptr::NonNull<crate::pc_system::BxPcSystemC>>,
    /// I/O base the BM-DMA ports are currently registered at (0 = none).
    /// Lets a BAR4 move unregister the old range first, matching Bochs
    /// devices.cc pci_write_handler_common BAR remapping.
    pub(crate) bmdma_ports_base: u16,
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

    /// Create a new device manager with all devices
    pub fn new() -> Self {
        Self {
            pic: BxPicC::new(),
            pit: BxPitC::new(),
            cmos: BxCmosC::new(),
            dma: BxDmaC::new(),
            keyboard: BxKeyboardC::new(),
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
            diag_tick_count: 0,
            diag_total_usec: 0,
            diag_vector_hist: [0; 256],
            mem_ptr: None,
            pcs_ptr: None,
            bmdma_ports_base: 0,
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
        self.acpi.reset();
        self.fw_cfg.reset();
        {
            self.pci_bridge.reset();
            self.pci2isa.reset();
            self.pci_ide.reset();
            self.pci_conf_addr = 0;
            self.pci_ide_bar4_needs_reregister = false;
            self.acpi_pm_needs_reregister = false;
            self.acpi_sm_needs_reregister = false;
            self.vga_bar_needs_reregister = false;
            // pci_bridge.reset() (above) already set pci_conf[0x72] = 0x02
            // (SMRAME off) and the emulator calls mem.disable_smram()
            // directly on hardware reset, so no deferred re-apply is needed.
            self.smram_needs_update = false;
            // Bochs pci.cc bx_pci_bridge_c::reset() re-applies memory type
            // for every PAM area directly (DEV_mem_set_memory_type loop)
            // right after zeroing the PAM config bytes, so the shadow-RAM
            // state matches the reset PAM config immediately. rusty_box
            // can't touch memory here (BxMemC isn't available to
            // DeviceManager::reset), so defer it the same way pci_write's
            // PAM branch does; drained by the next process_pci_deferred.
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

        // PIIX3 I/O ports: APM (0xB2-0xB3), ELCR (0x4D0-0x4D1), CPU reset (0xCF9)
        // Bochs pci2isa.cc init(): all five registered as 1-byte ports.
        for port in [
            super::pci2isa::APM_CMD_PORT,
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

    /// Register PCI IDE BM-DMA I/O ports when BAR4 changes.
    fn register_pci_ide_bmdma_ports(&mut self, io: &mut BxDevicesC) {
        let base = self.pci_ide.bmdma_base as u16;
        if base == 0 {
            return;
        }
        // A BAR4 move first unregisters the old range, matching Bochs
        // devices.cc pci_write_handler_common (DEV_unregister_io*_handler on
        // the previous BAR base before registering the new one).
        let old_base = self.bmdma_ports_base;
        if old_base != 0 && old_base != base {
            for offset in 0..16u16 {
                if self.pci_ide.bmdma_io_mask(offset as u8) != 0 {
                    io.unregister_io_handler(old_base + offset);
                }
            }
        }
        for offset in 0..16u16 {
            let mask = self.pci_ide.bmdma_io_mask(offset as u8);
            if mask != 0 {
                io.register_io_handler(DeviceId::Pci, base + offset, "PCI IDE BM-DMA", mask);
            }
        }
        self.bmdma_ports_base = base;
        tracing::debug!("PCI IDE BM-DMA ports registered at base {:#06x}", base);
    }

    /// Process deferred PCI port re-registrations.
    /// Called from the emulator loop when both DeviceManager and BxDevicesC are available.
    pub fn process_pci_deferred<'c>(
        &mut self,
        io: &mut BxDevicesC,
        mem: &mut crate::memory::BxMemC<'c>,
    ) {
        if self.pci_ide_bar4_needs_reregister {
            self.pci_ide_bar4_needs_reregister = false;
            if self.pci_ide.bmdma_base > 0 {
                self.register_pci_ide_bmdma_ports(io);
            }
        }
        if self.acpi_pm_needs_reregister {
            self.acpi_pm_needs_reregister = false;
            if self.acpi.pm_base != 0 {
                self.register_acpi_pm_ports(io);
            }
        }
        if self.acpi_sm_needs_reregister {
            self.acpi_sm_needs_reregister = false;
            if self.acpi.sm_base != 0 {
                self.register_acpi_sm_ports(io);
            }
        }
        if self.pam_needs_update {
            self.pam_needs_update = false;
            self.pci_bridge.apply_pam_to_memory(mem);
        }
        if self.smram_needs_update {
            self.smram_needs_update = false;
            self.pci_bridge.apply_smram_to_memory(mem);
        }
        if self.bios_write_needs_update {
            self.bios_write_needs_update = false;
            self.pci2isa.apply_bios_write_to_memory(mem);
        }
        if self.vga_bar_needs_reregister {
            self.vga_bar_needs_reregister = false;
            self.reregister_vga_bars(mem);
        }
        // Sync pci_conf_addr to BxDevicesC
        io.pci_conf_addr = self.pci_conf_addr;
    }

    /// Apply committed VGA PCI BAR bases to the memory system: move the LFB
    /// handler to a BIOS-assigned BAR0 base, and register the BAR2 MMIO window.
    /// Bochs vga.cc pci_bar_change_notify + DEV_pci_set_base_mem.
    fn reregister_vga_bars<'c>(&mut self, mem: &mut crate::memory::BxMemC<'c>) {
        use crate::iodev::vga::PCI_VGA_MMIO_SIZE;
        let device_id = crate::memory::MemoryDeviceId::Vga(&mut self.vga as *mut BxVgaC);

        // BAR0: relocate the linear framebuffer.
        if let Some((old_base, new_base)) = self.vga.take_pending_lfb_relocate() {
            let size = self.vga.lfb_size() as u64;
            let old_begin = old_base as u64;
            let new_begin = new_base as u64;
            if let Err(error) =
                mem.unregister_memory_handlers(device_id, old_begin, old_begin + size - 1)
            {
                tracing::error!("VGA LFB unregister at {old_base:#010x} failed: {error:?}");
            }
            match mem.register_memory_handlers(device_id, new_begin, new_begin + size - 1) {
                Ok(()) => {
                    self.vga.set_lfb_base(new_base);
                    tracing::info!("VGA LFB relocated {old_base:#010x} -> {new_base:#010x}");
                }
                Err(error) => {
                    tracing::error!("VGA LFB register at {new_base:#010x} failed: {error:?}");
                }
            }
        }

        // BAR2: register the VBE MMIO window at the assigned base.
        if let Some(new_base) = self.vga.take_pending_mmio_base() {
            let begin = new_base as u64;
            let end = begin + PCI_VGA_MMIO_SIZE as u64 - 1;
            match mem.register_memory_handlers(device_id, begin, end) {
                Ok(()) => {
                    self.vga.set_mmio_base(new_base);
                    tracing::info!("VGA BAR2 MMIO registered at {new_base:#010x}");
                }
                Err(error) => {
                    tracing::error!("VGA BAR2 MMIO register at {new_base:#010x} failed: {error:?}");
                }
            }
        }
    }

    /// BM-DMA timer — walks the PRD table and pumps data between the drive
    /// and guest RAM. Bochs pci_ide.cc `bx_pci_ide_c::timer()`.
    ///
    /// Bochs transfers directly between the drive and the channel bounce
    /// buffer; here each sector passes through a small stack buffer because
    /// the drive callbacks need `&mut BxPciIde` (abort/IRQ paths) while the
    /// bounce buffer also lives in `BxPciIde::bmdma[channel]`.
    pub fn pci_ide_timer<'c>(
        &mut self,
        channel: usize,
        pcs: &mut crate::pc_system::BxPcSystemC,
        mem: &mut crate::memory::BxMemC<'c>,
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
        let (prd_addr, prd_size_raw) = read_bmdma_prd(mem, pci_ide.bmdma[channel].prd_current);
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
            mem.poke_ram(prd_addr as usize, &pci_ide.bmdma[channel].buffer[idx..end]);
            pci_ide.bmdma[channel].buffer_idx = end;
        } else {
            // WRITE DMA: guest RAM -> bounce buffer -> drive (pci_ide.cc timer)
            tracing::trace!("BM-DMA write ch={channel} addr={prd_addr:#010x} size={size:#x}");
            let top = pci_ide.bmdma[channel].buffer_top;
            let end = (top + size).min(pci_ide.bmdma[channel].buffer.len());
            let data = mem.peek_ram(prd_addr as usize, end - top);
            let copied = data.len();
            pci_ide.bmdma[channel].buffer[top..top + copied].copy_from_slice(data);
            // Guest PRD pointing past end of RAM: zero-fill the shortfall so
            // the transfer stays deterministic (Bochs reads whatever the
            // physical page returns).
            pci_ide.bmdma[channel].buffer[top + copied..end].fill(0);
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
            let (_, next_size_raw) = read_bmdma_prd(mem, pci_ide.bmdma[channel].prd_current);
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
    pub(crate) fn service_pit_irq0(pit: &mut BxPitC, pic: &mut BxPicC) -> u32 {
        let (transitions, level) = pit.drain_irq0_events();
        if transitions == 0 {
            return 0;
        }
        let replay = transitions.min(3);
        // The k-th replayed level, ending at `level`, alternating backwards.
        let mut lvl = if replay % 2 == 1 { level } else { !level };
        for _ in 0..replay {
            // raise_irq/lower_irq queue the IOAPIC forward internally
            // (enqueue_ioapic_forward); every service_pit_irq0 call site
            // (inp/outp dispatch and DeviceManager::tick) drains
            // take_ioapic_forwards() afterwards, so the Option return —
            // the same event for callers that forward synchronously — is
            // already handled through that queue.
            if lvl {
                pic.raise_irq(0);
            } else {
                pic.lower_irq(0);
            }
            lvl = !lvl;
        }
        // Rising edges in an alternating sequence of `transitions` levels
        // ending at `level`.
        if level {
            transitions.div_ceil(2)
        } else {
            transitions / 2
        }
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

    /// Simulate time passing for timer-based devices
    /// Returns true if any interrupt is pending
    pub fn tick(
        &mut self,
        usec: u64,
        icount: u64,
        mut lapic: Option<&mut crate::cpu::apic::BxLocalApic>,
    ) -> bool {
        self.diag_tick_count += 1;
        self.diag_total_usec += usec;

        // Tick PIT/RTC first to generate periodic interrupts (Bochs-like behavior).
        // PIT drives IRQ0 through counter 0's OUT pin (Bochs pit.cc
        // irq_handler); CMOS/RTC drives IRQ8 when enabled.
        self.pit.tick(usec, icount);
        self.drain_pit_irq0();

        // CMOS: process IRQ8 lower BEFORE raise (from REG_STAT_C read)
        if self.cmos.check_irq8_lower() {
            self.pic.lower_irq(8);
        }
        self.cmos.tick(usec);
        if self.cmos.check_irq8() {
            self.pic.raise_irq(8);
        }

        // Keyboard: process IRQ lower requests BEFORE raises (matching Bochs
        // DEV_pic_lower_irq() calls in port 0x60 read handler, keyboard.cc/340)
        if self.keyboard.check_irq1_lower() {
            self.pic.lower_irq(1);
        }
        if self.keyboard.check_irq12_lower() {
            self.pic.lower_irq(12);
        }

        // Keyboard periodic: transfer internal buffers → output buffer,
        // collect IRQ requests. Returns bitmask: bit0=IRQ1, bit1=IRQ12.
        let kbd_irq = self.keyboard.periodic(usec as u32);
        if kbd_irq & 0x01 != 0 {
            self.pic.raise_irq(1);
        }
        if kbd_irq & 0x02 != 0 {
            self.pic.raise_irq(12);
        }

        // ACPI PM timer: tick and sync IRQ 9 (SCI) to PIC
        {
            self.acpi.tick(usec, icount);
            if self.acpi.irq9_level {
                self.pic.raise_irq(9);
            } else {
                self.pic.lower_irq(9);
            }
        }

        // Serial port: forward pending IRQ raise/lower to PIC
        for (irq, raise) in self.serial.take_pending_irqs() {
            if raise {
                self.pic.raise_irq(irq);
            } else {
                self.pic.lower_irq(irq);
            }
        }

        // Drain all IOAPIC forwards accumulated during device ticking
        {
            let (fwds, count) = self.pic.take_ioapic_forwards();
            let DeviceManager {
                ref mut pic,
                ref mut ioapic,
                ..
            } = *self;
            for &(irq, level) in &fwds[..count] {
                ioapic.set_irq_level(irq, level, Some(&mut *pic), lapic.as_deref_mut());
            }
        }

        self.pic.has_interrupt()
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
            "PIT: ticks={} total_usec={} pit_fires={} irq0_latched={} irq0_already_high={}\n\
             PIT counter0: mode={:?} inlatch={} count={} count_written={} gate={} output={} first_pass={}\n\
             PIC master: ISR={:#04x} IRR={:#04x} IMR={:#04x} int_pin={} irq_in[0..8]=[{},{},{},{},{},{},{},{}]\n\
             PIC slave:  ISR={:#04x} IRR={:#04x} IMR={:#04x} int_pin={} irq_in[0..8]=[{},{},{},{},{},{},{},{}]\n\
             PIC master_offset={:#04x} slave_offset={:#04x}\n\
             IAC calls={} vector_hist[0x20]={} vector_hist[0x21]={} vector_hist[0x08]={} vector_hist[0x2E]={}",
            self.diag_tick_count, self.diag_total_usec, self.diag_pit_fires,
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
                    self.acpi.generate_smi(value as u8);
                    self.pci2isa.apms = 0;
                    tracing::trace!(
                        "APM command {:#04x}: forwarded to ACPI, apms cleared (no SMM)",
                        value
                    );
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

    /// fw_cfg I/O write dispatch — passes memory ref for DMA.
    pub(crate) fn fw_cfg_write(&mut self, address: u16, value: u32, io_len: u8) {
        let mem = self.mem_ptr.map(|mut p| unsafe { p.as_mut() });
        self.fw_cfg.write_port(address, value, io_len, mem);
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
    /// A20 gate state from port 92h
    pub a20_gate: bool,
    /// Reset request type from port 92h bit 0 (Bochs treats it as software reset).
    pub reset_request: Option<ResetReason>,
}

impl SystemControlPort {
    /// Create a new System Control Port state
    pub fn new() -> Self {
        Self {
            value: 0,
            a20_gate: true, // A20 enabled by default on modern systems
            reset_request: None,
        }
    }

    /// Process a write to port 92h
    pub fn write(&mut self, value: u8) -> bool {
        let old_a20 = self.a20_gate;

        self.value = value;
        // Bochs devices.cc: bit 1 = A20 gate, bit 0 = fast reset
        self.a20_gate = (value & 0x02) != 0;
        self.reset_request = if (value & 0x01) != 0 {
            Some(ResetReason::Software)
        } else {
            None
        };

        // Return true if A20 state changed
        old_a20 != self.a20_gate
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_control_port() {
        let mut port = SystemControlPort::new();

        // Initially A20 is enabled
        assert!(port.a20_gate);
        assert!(port.reset_request.is_none());

        // Disable A20 (bit 1 = 0)
        let changed = port.write(0x00);
        assert!(changed); // State changed
        assert!(!port.a20_gate);

        // Enable A20 again (bit 1 = 1)
        let changed = port.write(0x02);
        assert!(changed);
        assert!(port.a20_gate);

        // Write same value (no change)
        let changed = port.write(0x02);
        assert!(!changed);

        // Trigger reset (bit 0 = 1)
        port.write(0x01);
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

    #[test]
    fn vga_pci_bar0_commit_relocates_lfb_handler() {
        on_big_stack(|| {
            use crate::memory::{BxMemC, BxMemoryStubC, MemoryDeviceId};

            let mut dm = DeviceManager::new();
            dm.vga.enable_pci();
            let stub = BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
            let mut mem = BxMemC::new(stub, false);

            // Register the LFB at its default base (as vga.init would).
            let vga_id = MemoryDeviceId::Vga(&mut dm.vga as *mut BxVgaC);
            let size = dm.vga.lfb_size() as u64;
            mem.register_memory_handlers(vga_id, 0xE000_0000, 0xE000_0000 + size - 1)
                .unwrap();

            // Guest relocates BAR0; process the deferred move.
            let change = dm.vga.pci_write(0x10, 0xE800_0000, 4);
            assert!(change.lfb);
            dm.vga_bar_needs_reregister = true;
            dm.reregister_vga_bars(&mut mem);

            // Old base is free again; new base is occupied (1-page probes).
            assert!(
                mem.register_memory_handlers(MemoryDeviceId::None, 0xE000_0000, 0xE00F_FFFF)
                    .is_ok(),
                "old LFB base must be freed"
            );
            assert!(
                mem.register_memory_handlers(MemoryDeviceId::None, 0xE800_0000, 0xE80F_FFFF)
                    .is_err(),
                "new LFB base must be occupied"
            );
        });
    }

    #[test]
    fn bmdma_read_dma_end_to_end_moves_disk_data_to_guest_ram() {
        on_big_stack(|| {
            use crate::iodev::harddrv::DeviceType;
            use crate::memory::{BxMemC, BxMemoryStubC};
            use crate::pc_system::{BxPcSystemC, TimerOwner};

            let mut dm = DeviceManager::new();
            let stub = BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
            let mut mem = BxMemC::new(stub, false);
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

            // Guest builds a single-entry PRD table at 0x8000:
            // target 0x4000, 1024 bytes, EOT.
            let mut prd = [0u8; 8];
            prd[0..4].copy_from_slice(&0x4000u32.to_le_bytes());
            prd[4..8].copy_from_slice(&(1024u32 | 0x8000_0000).to_le_bytes());
            mem.poke_ram(0x8000, &prd);

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
            assert!(
                dm.pci_ide.bmdma[0].data_ready,
                "READ DMA must signal bmdma_start_transfer"
            );

            // Guest programs DTPR and starts the engine (read direction).
            dm.pci_ide.bmdma_write(0xC004, 0x8000, 4);
            dm.pci_ide.bmdma_write(0xC000, 0x09, 1);
            let arm = dm.pci_ide.take_pending_timer_arm(0);
            assert_eq!(arm, Some(1));
            pcs.activate_timer_usec(handle, 1, false).unwrap();

            // Timer fires: single PRD with EOT completes the transfer.
            dm.pci_ide_timer(0, &mut pcs, &mut mem);

            assert_eq!(
                mem.peek_ram(0x4000, 1024),
                disk,
                "disk data must land at the PRD target"
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
    fn bmdma_bar4_move_unregisters_old_ports() {
        on_big_stack(|| {
            let mut dm = DeviceManager::new();
            let mut io = BxDevicesC::new();

            // BIOS assigns BAR4 at 0xC000 and the ports register there.
            assert!(dm.pci_ide.pci_write(0x20, 0x0000_C001, 4));
            dm.register_pci_ide_bmdma_ports(&mut io);
            assert_eq!(io.read_handlers[0xC000].device_id, DeviceId::Pci);
            assert_eq!(io.write_handlers[0xC004].device_id, DeviceId::Pci);

            // Guest moves BAR4 to 0xD000: the old range must be unregistered
            // (Bochs devices.cc pci_write_handler_common BAR remapping).
            assert!(dm.pci_ide.pci_write(0x20, 0x0000_D001, 4));
            dm.register_pci_ide_bmdma_ports(&mut io);
            assert_eq!(io.read_handlers[0xC000].device_id, DeviceId::None);
            assert_eq!(io.write_handlers[0xC004].device_id, DeviceId::None);
            assert_eq!(io.read_handlers[0xD000].device_id, DeviceId::Pci);
            assert_eq!(io.write_handlers[0xD004].device_id, DeviceId::Pci);
        });
    }

    #[test]
    fn bmdma_start_arms_timer_within_the_io_write() {
        on_big_stack(|| {
            use crate::pc_system::{BxPcSystemC, TimerOwner};

            let mut dm = DeviceManager::new();
            let mut io = BxDevicesC::new();
            let mut pcs = BxPcSystemC::new();
            pcs.initialize(1_000_000); // 1 tick = 1 us

            let handle = pcs
                .register_timer(TimerOwner::PciIdeCh0, 0, false, false, "test bmdma")
                .unwrap();
            dm.pci_ide.bmdma[0].timer_index = Some(handle);

            // BAR4 assigned; BM-DMA ports registered on the I/O bus.
            assert!(dm.pci_ide.pci_write(0x20, 0x0000_C001, 4));
            dm.register_pci_ide_bmdma_ports(&mut io);

            // Wire the execution-time pointers exactly like the emulator loop.
            io.set_device_manager(core::ptr::NonNull::from(&mut dm));
            dm.pcs_ptr = Some(core::ptr::NonNull::from(&mut pcs));

            // Guest programs DTPR and starts the engine: the one-shot must be
            // armed during this same outp (Bochs pci_ide.cc write:
            // bx_pc_system.activate_timer(timer_index, 1, 0)).
            io.outp(0xC004, 0x8000, 4, 0);
            io.outp(0xC000, 0x09, 1, 0);
            io.clear_device_manager();
            dm.pcs_ptr = None;

            assert_eq!(
                dm.pci_ide.take_pending_timer_arm(0),
                None,
                "arm must be drained by the outp itself"
            );
            pcs.tickn(1);
            let (owners, _, count) = pcs.take_fired_timers();
            assert_eq!(count, 1, "the 1 us one-shot must fire on the next tick");
            assert_eq!(owners[0], TimerOwner::PciIdeCh0);
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
            dm.reregister_vga_bars(&mut mem);

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
            dm.process_pci_deferred(&mut io, &mut mem);
            assert!(!dm.smram_needs_update, "drain must clear the flag");
            assert_eq!(mem.smram_state(), (true, true, false));

            // SMRAME off (0x02): SMRAM fully disabled.
            dm.pci_conf_addr = conf_addr(0x00, 0x72);
            dm.pci_write(0xCFE, 0x02, 1);
            assert!(dm.smram_needs_update);
            dm.process_pci_deferred(&mut io, &mut mem);
            assert_eq!(mem.smram_state(), (false, false, false));

            // Illegal DOPEN&&DCLS combo (SMRAME|DOPEN|DCLS = 0x68): Bochs
            // BX_PANICs; rusty_box must not crash the host on a guest
            // register write and instead treats it as disabled.
            dm.pci_conf_addr = conf_addr(0x00, 0x72);
            dm.pci_write(0xCFE, 0x68, 1);
            assert!(dm.smram_needs_update);
            dm.process_pci_deferred(&mut io, &mut mem);
            assert_eq!(mem.smram_state(), (false, false, false));
        });
    }

    // ─── Finding #20a: PAM config re-applied to memory after guest reset ─────
    //
    // Bochs pci.cc bx_pci_bridge_c::reset() zeroes the PAM config bytes AND
    // re-applies memory type for every PAM area directly
    // (DEV_mem_set_memory_type loop), so the shadow-RAM routing tracks the
    // reset PAM config immediately. rusty_box defers the memory-side effect
    // via `pam_needs_update`, drained by `process_pci_deferred`.

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
            dm.process_pci_deferred(&mut io, &mut mem);
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
            dm.process_pci_deferred(&mut io, &mut mem);
            assert!(!dm.pam_needs_update, "drain must clear the flag");
            assert_eq!(
                mem.memory_type(12, 1),
                false,
                "area 12 (F0000) write-shadow must be back to the reset default"
            );
        });
    }
}
