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

use alloc::format;
use alloc::string::String;

use crate::{cpu::ResetReason, memory::BxMemC, pc_system::BxPcSystemC, Result};

#[cfg(feature = "bx_support_pci")]
use super::acpi::BxAcpiCtrl;
use super::cmos::{BxCmosC, CMOS_ADDR, CMOS_DATA};
use super::dma::BxDmaC;
use super::harddrv::BxHardDriveC;
use super::ioapic::BxIoApic;
use super::keyboard::{BxKeyboardC, KBD_DATA_PORT, KBD_STATUS_PORT, SYSTEM_CONTROL_B};
#[cfg(feature = "bx_support_pci")]
use super::pci::BxPciBridge;
#[cfg(feature = "bx_support_pci")]
use super::pci2isa::BxPiix3;
#[cfg(feature = "bx_support_pci")]
use super::pci_ide::BxPciIde;
use super::pic::{
    BxPicC, PIC_ELCR1, PIC_ELCR2, PIC_MASTER_CMD, PIC_MASTER_DATA, PIC_SLAVE_CMD, PIC_SLAVE_DATA,
};
use super::pit::{BxPitC, PIT_CONTROL, PIT_COUNTER0, PIT_COUNTER1, PIT_COUNTER2};
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
    #[cfg(feature = "bx_support_apic")]
    pub(crate) ioapic: BxIoApic,
    /// PIIX4 ACPI Power Management controller
    /// Bochs: `bx_acpi_ctrl_c *pluginACPIController` (iodev/iodev.h)
    #[cfg(feature = "bx_support_pci")]
    pub(crate) acpi: BxAcpiCtrl,
    /// i440FX PCI Host Bridge (bus 0, dev 0, func 0)
    /// Bochs: `bx_pci_bridge_c *pluginPciBridge` (iodev/iodev.h)
    #[cfg(feature = "bx_support_pci")]
    pub(crate) pci_bridge: BxPciBridge,
    /// PIIX3 PCI-to-ISA Bridge (bus 0, dev 1, func 0)
    /// Bochs: `bx_piix3_c *pluginPci2IsaBridge` (iodev/iodev.h)
    #[cfg(feature = "bx_support_pci")]
    pub(crate) pci2isa: BxPiix3,
    /// PIIX3 PCI IDE Controller (bus 0, dev 1, func 1)
    /// Bochs: `bx_pci_ide_c *pluginPciIdeController` (iodev/iodev.h)
    #[cfg(feature = "bx_support_pci")]
    pub(crate) pci_ide: BxPciIde,
    /// 16550 UART Serial Port Controller (COM1-COM4)
    /// Bochs: `bx_serial_c *pluginSerial` (iodev/iodev.h)
    pub(crate) serial: BxSerialC,
    /// PCI configuration address register (shadow copy for handler dispatch)
    /// Bochs: bx_devices_c::pci_conf_addr (devices.cc)
    #[cfg(feature = "bx_support_pci")]
    pub(crate) pci_conf_addr: u32,
    /// Deferred: PCI IDE BAR4 changed, needs BM-DMA port re-registration
    #[cfg(feature = "bx_support_pci")]
    pub(crate) pci_ide_bar4_needs_reregister: bool,
    /// Deferred: ACPI PM base changed, needs port re-registration
    #[cfg(feature = "bx_support_pci")]
    pub(crate) acpi_pm_needs_reregister: bool,
    /// Deferred: ACPI SMBus base changed, needs port re-registration
    #[cfg(feature = "bx_support_pci")]
    pub(crate) acpi_sm_needs_reregister: bool,
    /// Deferred: PAM registers changed, needs memory type update
    #[cfg(feature = "bx_support_pci")]
    pub(crate) pam_needs_update: bool,
    /// Diagnostic: PIT fire count (check_irq0 returned true)
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
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceManager {
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
            #[cfg(feature = "bx_support_apic")]
            ioapic: BxIoApic::new(),
            #[cfg(feature = "bx_support_pci")]
            acpi: BxAcpiCtrl::new(),
            #[cfg(feature = "bx_support_pci")]
            pci_bridge: BxPciBridge::new(),
            #[cfg(feature = "bx_support_pci")]
            pci2isa: BxPiix3::new(),
            #[cfg(feature = "bx_support_pci")]
            pci_ide: BxPciIde::new(),
            serial: BxSerialC::new(1), // COM1 only
            #[cfg(feature = "bx_support_pci")]
            pci_conf_addr: 0,
            #[cfg(feature = "bx_support_pci")]
            pci_ide_bar4_needs_reregister: false,
            #[cfg(feature = "bx_support_pci")]
            acpi_pm_needs_reregister: false,
            #[cfg(feature = "bx_support_pci")]
            acpi_sm_needs_reregister: false,
            #[cfg(feature = "bx_support_pci")]
            pam_needs_update: false,
            diag_pit_fires: 0,
            diag_irq0_latched: 0,
            diag_irq0_already_high: 0,
            diag_iac_count: 0,
            diag_tick_count: 0,
            diag_total_usec: 0,
            diag_vector_hist: [0; 256],
        }
    }

    /// Initialize all devices and register I/O handlers
    ///
    /// Matches device loading order from cpp_orig/bochs/iodev/devices.cc:250-277:
    /// 1. CMOS (line 250)
    /// 2. DMA (line 251)
    /// 3. PIC (line 252)
    /// 4. PIT (line 253)
    /// 5. VGA (line 254-256)
    /// 6. Keyboard (line 262)
    /// 7. Hard drive (line 275-277)
    pub fn init(&mut self, io: &mut BxDevicesC, mem: &mut BxMemC) -> Result<()> {
        tracing::info!("Initializing device manager");

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
        #[cfg(feature = "bx_support_apic")]
        self.ioapic.init(mem)?;
        // 9. ACPI Power Management (Bochs: pluginACPIController->init() in devices.cc)
        #[cfg(feature = "bx_support_pci")]
        self.acpi.reset();
        // 10. PCI bus devices (Bochs: pluginPciBridge->init(), pluginPci2IsaBridge->init(), etc.)
        #[cfg(feature = "bx_support_pci")]
        {
            self.pci_bridge.reset();
            self.pci2isa.reset();
            self.pci_ide.reset();
        }

        // Wire up PIT pointer for port 0x61 integration (keyboard reads PIT C2 output)
        let pit_ptr = &mut self.pit as *mut BxPitC;
        unsafe { self.keyboard.set_pit_ptr(pit_ptr); }

        // Register I/O handlers for each device (order doesn't matter for handlers)
        self.register_cmos_handlers(io);
        self.register_dma_handlers(io);
        self.register_pic_handlers(io);
        self.register_pit_handlers(io);
        self.register_keyboard_handlers(io);
        self.register_harddrv_handlers(io);
        self.register_serial_handlers(io);
        #[cfg(feature = "bx_support_pci")]
        self.register_acpi_handlers(io);
        #[cfg(feature = "bx_support_pci")]
        self.register_pci_handlers(io);
        // Register BM-DMA ports if BAR4 is pre-configured (for direct boot without BIOS)
        #[cfg(feature = "bx_support_pci")]
        if self.pci_ide.bmdma_base > 0 {
            self.register_pci_ide_bmdma_ports(io);
        }

        tracing::info!("Device manager initialization complete");
        Ok(())
    }

    /// Reset all devices
    pub fn reset(&mut self, reset_type: ResetReason) -> Result<()> {
        tracing::info!("Device manager reset: {:?}", reset_type);

        self.pic.reset();
        self.pit.reset();
        self.cmos.reset();
        self.dma.reset();
        self.keyboard.reset();
        self.harddrv.reset();
        self.vga.reset();
        self.serial.reset();
        #[cfg(feature = "bx_support_apic")]
        self.ioapic.reset();
        #[cfg(feature = "bx_support_pci")]
        self.acpi.reset();
        #[cfg(feature = "bx_support_pci")]
        {
            self.pci_bridge.reset();
            self.pci2isa.reset();
            self.pci_ide.reset();
            self.pci_conf_addr = 0;
            self.pci_ide_bar4_needs_reregister = false;
            self.acpi_pm_needs_reregister = false;
            self.acpi_sm_needs_reregister = false;
        }

        Ok(())
    }

    /// Register PIC I/O handlers
    fn register_pic_handlers(&mut self, io: &mut BxDevicesC) {
        for port in [
            PIC_MASTER_CMD,
            PIC_MASTER_DATA,
            PIC_SLAVE_CMD,
            PIC_SLAVE_DATA,
            PIC_ELCR1,
            PIC_ELCR2,
        ] {
            io.register_io_handler(DeviceId::Pic, port, "8259 PIC", 0x1);
        }
    }

    /// Register PIT I/O handlers
    fn register_pit_handlers(&mut self, io: &mut BxDevicesC) {
        for port in [PIT_COUNTER0, PIT_COUNTER1, PIT_COUNTER2, PIT_CONTROL] {
            io.register_io_handler(DeviceId::Pit, port, "8254 PIT", 0x1);
        }
    }

    /// Register CMOS I/O handlers
    fn register_cmos_handlers(&mut self, io: &mut BxDevicesC) {
        io.register_io_handler(DeviceId::Cmos, CMOS_ADDR, "CMOS Address", 0x1);
        io.register_io_handler(DeviceId::Cmos, CMOS_DATA, "CMOS Data", 0x1);
        // Bochs cmos.cc:225-228 — extended CMOS RAM ports (addresses 0x80-0xFF)
        io.register_io_handler(DeviceId::Cmos, 0x0072, "Ext CMOS RAM", 0x1);
        io.register_io_handler(DeviceId::Cmos, 0x0073, "Ext CMOS RAM", 0x1);
    }

    /// Register DMA I/O handlers (Bochs dma.cc:138-154)
    fn register_dma_handlers(&mut self, io: &mut BxDevicesC) {
        // DMA1 ports 0x0000-0x000F (Bochs dma.cc:139-142)
        for port in 0x0000..=0x000F_u16 {
            io.register_io_handler(DeviceId::Dma, port, "DMA controller", 0x1);
        }

        // Page registers 0x0080-0x008F (Bochs dma.cc:145-148)
        for port in 0x0080..=0x008F_u16 {
            io.register_io_handler(DeviceId::Dma, port, "DMA controller", 0x1);
        }

        // DMA2 ports 0x00C0-0x00DE, step 2 (Bochs dma.cc:151-154)
        let mut port = 0x00C0_u16;
        while port <= 0x00DE {
            io.register_io_handler(DeviceId::Dma, port, "DMA controller", 0x1);
            port += 2;
        }
    }

    /// Register Keyboard I/O handlers
    fn register_keyboard_handlers(&mut self, io: &mut BxDevicesC) {
        io.register_io_handler(DeviceId::Keyboard, KBD_DATA_PORT, "Keyboard Data", 0x1);
        io.register_io_handler(DeviceId::Keyboard, KBD_STATUS_PORT, "Keyboard Status/Command", 0x1);
        io.register_io_handler(DeviceId::Keyboard, SYSTEM_CONTROL_B, "System Control B", 0x1);
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
    #[cfg(feature = "bx_support_pci")]
    fn register_acpi_handlers(&mut self, io: &mut BxDevicesC) {
        // SMI command port (0xB2) — Bochs acpi.cc:137-138
        io.register_io_write_handler(DeviceId::Acpi, 0x00B2, "ACPI SMI Command", 0x1);

        // ACPI debug port (0xB044) — Bochs acpi.cc:145-148
        io.register_io_handler(DeviceId::Acpi, 0xB044, "ACPI Debug", 0x7);
    }

    /// Register ACPI PM I/O port range (called when PM base changes via PCI config).
    #[cfg(feature = "bx_support_pci")]
    pub fn register_acpi_pm_ports(&mut self, io: &mut BxDevicesC) {
        let base = self.acpi.pm_base as u16;
        if base == 0 {
            return;
        }
        // Register 64 ports at PM base — Bochs acpi.cc:563-571
        for offset in 0..64u16 {
            let mask = self.acpi.pm_io_mask(offset as u8);
            if mask != 0 {
                io.register_io_handler(DeviceId::Acpi, base + offset, "ACPI PM", mask);
            }
        }
        self.acpi.pm_ports_registered = true;
    }

    /// Register ACPI SMBus I/O port range (called when SM base changes via PCI config).
    #[cfg(feature = "bx_support_pci")]
    pub fn register_acpi_sm_ports(&mut self, io: &mut BxDevicesC) {
        let base = self.acpi.sm_base as u16;
        if base == 0 {
            return;
        }
        // Register 16 ports at SM base — Bochs acpi.cc:572-580
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
    /// Bochs: devices.cc:264-270 (PCI bridge init order)
    #[cfg(feature = "bx_support_pci")]
    fn register_pci_handlers(&mut self, io: &mut BxDevicesC) {
        // PCI config address register (0xCF8) — 4-byte write only
        io.register_io_handler(DeviceId::Pci, super::pci::PCI_CONFIG_ADDR, "PCI Config Addr", 0x4);

        // PCI config data register (0xCFC-0xCFF) — 1/2/4-byte
        for port in 0x0CFC..=0x0CFF_u16 {
            io.register_io_handler(DeviceId::Pci, port, "PCI Config Data", 0x7);
        }

        // PIIX3 I/O ports: APM (0xB2-0xB3), ELCR (0x4D0-0x4D1)
        for port in [
            super::pci2isa::APM_CMD_PORT,
            super::pci2isa::APM_STS_PORT,
            super::pci2isa::ELCR1_PORT,
            super::pci2isa::ELCR2_PORT,
        ] {
            io.register_io_handler(DeviceId::Pci, port, "PIIX3", 0x1);
        }
    }

    /// Route a PCI config space read to the correct device.
    /// Bochs: devices.cc bx_devices_c::pci_read_handler() (inline in read_handler)
    #[cfg(feature = "bx_support_pci")]
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
            // APM + ELCR ports → PIIX3
            0x00B2 | 0x00B3 | 0x04D0 | 0x04D1 => self.pci2isa.read(address),
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
    #[cfg(feature = "bx_support_pci")]
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
            // Unrecognized device
            _ => {
                0xFFFF_FFFF
            }
        }
    }

    /// Register PCI IDE BM-DMA I/O ports when BAR4 changes.
    #[cfg(feature = "bx_support_pci")]
    fn register_pci_ide_bmdma_ports(&mut self, io: &mut BxDevicesC) {
        let base = self.pci_ide.bmdma_base as u16;
        if base == 0 {
            return;
        }
        for offset in 0..16u16 {
            let mask = self.pci_ide.bmdma_io_mask(offset as u8);
            if mask != 0 {
                io.register_io_handler(DeviceId::Pci, base + offset, "PCI IDE BM-DMA", mask);
            }
        }
        tracing::info!("PCI IDE BM-DMA ports registered at base {:#06x}", base);
    }

    /// Process deferred PCI port re-registrations.
    /// Called from the emulator loop when both DeviceManager and BxDevicesC are available.
    #[cfg(feature = "bx_support_pci")]
    pub fn process_pci_deferred<'c, I: crate::cpu::BxCpuIdTrait>(
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
            self.pci_bridge.apply_pam_to_memory::<I>(mem);
        }
        // Sync pci_conf_addr to BxDevicesC
        io.pci_conf_addr = self.pci_conf_addr;
    }

    /// Simulate time passing for timer-based devices
    /// Returns true if any interrupt is pending
    pub fn tick(&mut self, usec: u64) -> bool {
        self.diag_tick_count += 1;
        self.diag_total_usec += usec;
        // Tick PIT/RTC first to generate periodic interrupts (Bochs-like behavior).
        // PIT drives IRQ0, CMOS/RTC drives IRQ8 when enabled.
        let _pit_fired = self.pit.tick(usec);
        if self.pit.check_irq0() {
            self.diag_pit_fires += 1;
            // Track whether raise_irq will actually latch
            let was_high = self.pic.master.irq_in[0] != 0;
            if was_high {
                self.diag_irq0_already_high += 1;
            }
            // PIT pulses the IRQ line: lower first to reset edge-detect state,
            // then raise.  Without this, raise_irq(0) is a no-op when irq_in[0]
            // is still high from a previous fire that the CPU hasn't yet
            // acknowledged via INTA (common when our coarse batching delays
            // interrupt delivery).
            self.pic.lower_irq(0);
            self.pic.raise_irq(0);
            self.diag_irq0_latched += 1;
        }

        // CMOS: process IRQ8 lower BEFORE raise (from REG_STAT_C read)
        if self.cmos.check_irq8_lower() {
            self.pic.lower_irq(8);
        }
        self.cmos.tick(usec);
        if self.cmos.check_irq8() {
            self.pic.raise_irq(8);
        }

        // Keyboard: process IRQ lower requests BEFORE raises (matching Bochs
        // DEV_pic_lower_irq() calls in port 0x60 read handler, keyboard.cc:315/340)
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
        #[cfg(feature = "bx_support_pci")]
        {
            self.acpi.tick(usec);
            if self.acpi.irq9_level {
                self.pic.raise_irq(9);
            } else {
                self.pic.lower_irq(9);
            }
        }

        // Serial port: forward pending IRQ raise/lower to PIC
        // (Bochs: serial.cc raise_interrupt/lower_interrupt call DEV_pic_raise/lower_irq)
        // PIC now forwards to IOAPIC synchronously (Bochs pic.cc:499-500).
        for (irq, raise) in self.serial.take_pending_irqs() {
            if raise {
                self.pic.raise_irq(irq);
            } else {
                self.pic.lower_irq(irq);
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

    /// Get ATA controller diagnostic string
    pub fn ata_diag(&self) -> String {
        self.harddrv.diag_string()
    }

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
        // Reads current A20 gate state (bit 1)
        // TODO: When DeviceManager owns SystemControlPort, read from it directly.
        // For now, return default A20-enabled.
        0x02
    }

    /// Port 92h write dispatch (System Control Port)
    pub(crate) fn port92_write(&mut self, _port: u16, value: u32, _io_len: u8) {
        let _value = value as u8;
        // Port92 state is owned by Emulator (SystemControlPort), not DeviceManager.
        // The emulator polls system_control after each batch for A20/reset changes.
        // Writes here are a no-op until we move SystemControlPort into DeviceManager.
        tracing::debug!("Port 92h write: value={:#04x} (dispatch stub)", _value);
    }

    /// PCI I/O read dispatch
    #[cfg(feature = "bx_support_pci")]
    pub(crate) fn pci_read(&self, address: u16, io_len: u8) -> u32 {
        self.pci_io_read(address, io_len)
    }

    /// PCI I/O write dispatch
    #[cfg(feature = "bx_support_pci")]
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
                match devfunc {
                    0x00 => {
                        let pam_changed = self.pci_bridge.pci_write(reg_addr, value, io_len);
                        if pam_changed {
                            self.pam_needs_update = true;
                        }
                    }
                    0x08 => self.pci2isa.pci_write(reg_addr, value, io_len),
                    0x09 => {
                        let bar4_changed = self.pci_ide.pci_write(reg_addr, value, io_len);
                        if bar4_changed {
                            self.pci_ide_bar4_needs_reregister = true;
                        }
                    }
                    0x0B => {
                        let (pm_changed, sm_changed) = self.acpi.pci_write(reg_addr, value, io_len);
                        if pm_changed {
                            self.acpi_pm_needs_reregister = true;
                        }
                        if sm_changed {
                            self.acpi_sm_needs_reregister = true;
                        }
                    }
                    _ => {}
                }
            }
            0x00B2 | 0x00B3 | 0x04D0 | 0x04D1 => {
                self.pci2isa.write(address, value, io_len);
                if address == 0x00B2 {
                    self.acpi.generate_smi(value as u8);
                    self.pci2isa.apms = 0;
                    tracing::debug!(
                        "APM command {:#04x}: forwarded to ACPI, apms cleared (no SMM)",
                        value
                    );
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
    #[cfg(feature = "bx_support_pci")]
    pub(crate) fn acpi_read(&mut self, address: u16, io_len: u8) -> u32 {
        self.acpi.read(address, io_len)
    }

    /// ACPI I/O write dispatch
    #[cfg(feature = "bx_support_pci")]
    pub(crate) fn acpi_write(&mut self, address: u16, value: u32, io_len: u8) {
        if address == 0x00B2 {
            self.acpi.generate_smi(value as u8);
        } else {
            self.acpi.write(address, value, io_len);
        }
    }

    /// PCI IDE I/O read dispatch (BM-DMA ports)
    #[cfg(feature = "bx_support_pci")]
    pub(crate) fn pci_ide_read(&self, address: u16, io_len: u8) -> u32 {
        self.pci_ide.bmdma_read(address, io_len)
    }

    /// PCI IDE I/O write dispatch (BM-DMA ports)
    #[cfg(feature = "bx_support_pci")]
    pub(crate) fn pci_ide_write(&mut self, address: u16, value: u32, io_len: u8) {
        self.pci_ide.bmdma_write(address, value, io_len);
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
    pub fn init(
        &mut self,
        _mem: &mut BxMemC,
        _port92_state: Option<*mut SystemControlPort>,
    ) -> Result<()> {
        tracing::info!("Initializing device subsystem");

        // Register Port 92h - System Control Port (A20 gate, fast reset)
        self.register_io_handler(DeviceId::Port92, PORT_92H, "Port 92h System Control", 0x1);

        tracing::info!("Device initialization complete");
        Ok(())
    }

    /// Initialize devices with PC system reference for A20 control
    ///
    /// This variant allows devices to control the A20 line during operation.
    pub fn init_with_pc_system(
        &mut self,
        _mem: &mut BxMemC,
        _pc_system: &mut BxPcSystemC,
        port92_state: Option<*mut SystemControlPort>,
    ) -> Result<()> {
        self.init(_mem, port92_state)
    }

    /// Reset all devices
    ///
    /// Matches bx_devices_c::reset() from cpp_orig/bochs/iodev/devices.cc:398-411
    ///
    /// # Arguments
    /// * `reset_type` - Type of reset (Hardware or Software)
    pub fn reset(&mut self, reset_type: ResetReason) -> Result<()> {
        match reset_type {
            ResetReason::Hardware => {
                tracing::info!("Device hardware reset");
                #[cfg(feature = "bx_support_pci")]
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
                tracing::info!("Device software reset");
            }
        }
        Ok(())
    }

    /// Register device state for save/restore functionality
    pub fn register_state(&mut self) -> Result<()> {
        tracing::debug!("Device state registered");
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
    /// Reset request flag
    pub reset_request: bool,
}

impl SystemControlPort {
    /// Create a new System Control Port state
    pub fn new() -> Self {
        Self {
            value: 0,
            a20_gate: true, // A20 enabled by default on modern systems
            reset_request: false,
        }
    }

    /// Process a write to port 92h
    pub fn write(&mut self, value: u8) -> bool {
        let old_a20 = self.a20_gate;

        self.value = value;
        // Bochs devices.cc:559-561: bit 1 = A20 gate, bit 0 = fast reset
        self.a20_gate = (value & 0x02) != 0;
        self.reset_request = (value & 0x01) != 0;

        // Return true if A20 state changed
        old_a20 != self.a20_gate
    }

    /// Read current port 92h value
    /// Bochs devices.cc:505: return(BX_GET_ENABLE_A20() << 1)
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
        assert!(!port.reset_request);

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
        assert!(port.reset_request);
    }
}
