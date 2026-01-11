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

use core::ffi::c_void;

use crate::{
    cpu::ResetReason,
    memory::BxMemC,
    pc_system::BxPcSystemC,
    Result,
};

use super::BxDevicesC;
use super::pic::{self, BxPicC, PIC_MASTER_CMD, PIC_MASTER_DATA, PIC_SLAVE_CMD, PIC_SLAVE_DATA, PIC_ELCR1, PIC_ELCR2};
use super::pit::{self, BxPitC, PIT_COUNTER0, PIT_COUNTER1, PIT_COUNTER2, PIT_CONTROL};
use super::cmos::{self, BxCmosC, CMOS_ADDR, CMOS_DATA};
use super::dma::{self, BxDmaC};
use super::keyboard::{self, BxKeyboardC, KBD_DATA_PORT, KBD_STATUS_PORT, SYSTEM_CONTROL_B};
use super::harddrv::{self, BxHardDriveC};

/// Port 0x92 - System Control Port
/// Bit 0: Fast A20 gate control (1 = A20 enabled)
/// Bit 1: Fast reset (writing 1 triggers CPU reset)
const PORT_92H: u16 = 0x0092;

/// Port 92h state storage
#[derive(Debug, Default, Clone)]
pub struct Port92State {
    /// Current value of port 92h
    pub value: u8,
}

/// Unified Device Manager
/// 
/// Holds all hardware devices and manages their initialization,
/// reset, and I/O port registration. This mirrors Bochs' `bx_devices_c`.
#[derive(Debug)]
pub struct DeviceManager {
    /// 8259 PIC (Programmable Interrupt Controller)
    pub pic: BxPicC,
    /// 8254 PIT (Programmable Interval Timer)
    pub pit: BxPitC,
    /// CMOS/RTC
    pub cmos: BxCmosC,
    /// 8237 DMA Controller
    pub dma: BxDmaC,
    /// 8042 Keyboard Controller
    pub keyboard: BxKeyboardC,
    /// ATA/IDE Hard Drive Controller
    pub harddrv: BxHardDriveC,
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
        }
    }

    /// Initialize all devices and register I/O handlers
    pub fn init(&mut self, io: &mut BxDevicesC) -> Result<()> {
        tracing::info!("Initializing device manager");

        // Initialize each device
        self.pic.init();
        self.pit.init();
        self.cmos.init();
        self.dma.init();
        self.keyboard.init();
        self.harddrv.init();

        // Register I/O handlers for each device
        self.register_pic_handlers(io);
        self.register_pit_handlers(io);
        self.register_cmos_handlers(io);
        self.register_dma_handlers(io);
        self.register_keyboard_handlers(io);
        self.register_harddrv_handlers(io);

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

        Ok(())
    }

    /// Register PIC I/O handlers
    fn register_pic_handlers(&mut self, io: &mut BxDevicesC) {
        let pic_ptr = &mut self.pic as *mut BxPicC as *mut c_void;
        
        for port in [PIC_MASTER_CMD, PIC_MASTER_DATA, PIC_SLAVE_CMD, PIC_SLAVE_DATA, PIC_ELCR1, PIC_ELCR2] {
            io.register_io_handler(
                pic_ptr,
                pic::pic_read_handler,
                pic::pic_write_handler,
                port,
                "8259 PIC",
                0x1,
            );
        }
    }

    /// Register PIT I/O handlers
    fn register_pit_handlers(&mut self, io: &mut BxDevicesC) {
        let pit_ptr = &mut self.pit as *mut BxPitC as *mut c_void;
        
        for port in [PIT_COUNTER0, PIT_COUNTER1, PIT_COUNTER2, PIT_CONTROL] {
            io.register_io_handler(
                pit_ptr,
                pit::pit_read_handler,
                pit::pit_write_handler,
                port,
                "8254 PIT",
                0x1,
            );
        }
    }

    /// Register CMOS I/O handlers
    fn register_cmos_handlers(&mut self, io: &mut BxDevicesC) {
        let cmos_ptr = &mut self.cmos as *mut BxCmosC as *mut c_void;
        
        io.register_io_handler(
            cmos_ptr,
            cmos::cmos_read_handler,
            cmos::cmos_write_handler,
            CMOS_ADDR,
            "CMOS Address",
            0x1,
        );
        io.register_io_handler(
            cmos_ptr,
            cmos::cmos_read_handler,
            cmos::cmos_write_handler,
            CMOS_DATA,
            "CMOS Data",
            0x1,
        );
    }

    /// Register DMA I/O handlers
    fn register_dma_handlers(&mut self, io: &mut BxDevicesC) {
        let dma_ptr = &mut self.dma as *mut BxDmaC as *mut c_void;
        
        // DMA1 ports (0x00-0x0F)
        for port in 0x00..=0x0F_u16 {
            io.register_io_handler(
                dma_ptr,
                dma::dma_read_handler,
                dma::dma_write_handler,
                port,
                "DMA1",
                0x1,
            );
        }
        
        // DMA2 ports (0xC0-0xDF)
        for port in 0xC0..=0xDF_u16 {
            io.register_io_handler(
                dma_ptr,
                dma::dma_read_handler,
                dma::dma_write_handler,
                port,
                "DMA2",
                0x1,
            );
        }
        
        // DMA page registers
        for port in [0x81_u16, 0x82, 0x83, 0x87, 0x89, 0x8A, 0x8B, 0x8F] {
            io.register_io_handler(
                dma_ptr,
                dma::dma_read_handler,
                dma::dma_write_handler,
                port,
                "DMA Page",
                0x1,
            );
        }
    }

    /// Register Keyboard I/O handlers
    fn register_keyboard_handlers(&mut self, io: &mut BxDevicesC) {
        let kbd_ptr = &mut self.keyboard as *mut BxKeyboardC as *mut c_void;
        
        io.register_io_handler(
            kbd_ptr,
            keyboard::keyboard_read_handler,
            keyboard::keyboard_write_handler,
            KBD_DATA_PORT,
            "Keyboard Data",
            0x1,
        );
        io.register_io_handler(
            kbd_ptr,
            keyboard::keyboard_read_handler,
            keyboard::keyboard_write_handler,
            KBD_STATUS_PORT,
            "Keyboard Status/Command",
            0x1,
        );
        io.register_io_handler(
            kbd_ptr,
            keyboard::keyboard_read_handler,
            keyboard::keyboard_write_handler,
            SYSTEM_CONTROL_B,
            "System Control B",
            0x1,
        );
    }

    /// Register Hard Drive I/O handlers
    fn register_harddrv_handlers(&mut self, io: &mut BxDevicesC) {
        let hd_ptr = &mut self.harddrv as *mut BxHardDriveC as *mut c_void;
        
        // Primary ATA (0x1F0-0x1F7, 0x3F6)
        for port in 0x1F0..=0x1F7_u16 {
            io.register_io_handler(
                hd_ptr,
                harddrv::harddrv_read_handler,
                harddrv::harddrv_write_handler,
                port,
                "ATA Primary",
                0x7, // 1, 2, 4 byte access
            );
        }
        io.register_io_handler(
            hd_ptr,
            harddrv::harddrv_read_handler,
            harddrv::harddrv_write_handler,
            0x3F6,
            "ATA Primary Control",
            0x1,
        );
        
        // Secondary ATA (0x170-0x177, 0x376)
        for port in 0x170..=0x177_u16 {
            io.register_io_handler(
                hd_ptr,
                harddrv::harddrv_read_handler,
                harddrv::harddrv_write_handler,
                port,
                "ATA Secondary",
                0x7,
            );
        }
        io.register_io_handler(
            hd_ptr,
            harddrv::harddrv_read_handler,
            harddrv::harddrv_write_handler,
            0x376,
            "ATA Secondary Control",
            0x1,
        );
    }

    /// Simulate time passing for timer-based devices
    /// Returns true if any interrupt is pending
    pub fn tick(&mut self, usec: u64) -> bool {
        // Tick PIT and check for IRQ0
        if self.pit.check_irq0() {
            self.pic.raise_irq(0);
        }
        
        // Tick CMOS/RTC and check for IRQ8
        if self.cmos.check_irq8() {
            self.pic.raise_irq(8);
        }
        
        // Check keyboard IRQ1
        if self.keyboard.check_irq1() {
            self.pic.raise_irq(1);
        }
        
        // Check mouse IRQ12
        if self.keyboard.check_irq12() {
            self.pic.raise_irq(12);
        }
        
        // Check hard drive IRQ14/15
        if self.harddrv.check_irq14() {
            self.pic.raise_irq(14);
        }
        if self.harddrv.check_irq15() {
            self.pic.raise_irq(15);
        }
        
        self.pic.has_interrupt()
    }

    /// Check if an interrupt is pending
    pub fn has_interrupt(&self) -> bool {
        self.pic.has_interrupt()
    }

    /// Acknowledge interrupt and get vector
    pub fn iac(&mut self) -> u8 {
        self.pic.iac()
    }

    /// Get A20 state from keyboard controller
    pub fn get_a20_from_keyboard(&self) -> bool {
        self.keyboard.get_a20_enabled()
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
    pub fn init(&mut self, _mem: &mut BxMemC) -> Result<()> {
        tracing::info!("Initializing device subsystem");

        // Register Port 92h - System Control Port (A20 gate, fast reset)
        // Note: We use a static handler function; the actual state is managed
        // externally by the Emulator struct
        self.register_io_handler(
            core::ptr::null_mut(),
            port92_read_handler,
            port92_write_handler,
            PORT_92H,
            "Port 92h System Control",
            0x1, // 1-byte I/O only
        );

        // TODO: Initialize other core devices as they are implemented:
        // - CMOS RTC (ports 0x70-0x71)
        // - DMA controller (ports 0x00-0x0F, 0x80-0x8F, 0xC0-0xDF)
        // - PIC - Interrupt controller (ports 0x20-0x3F, 0xA0-0xBF)
        // - PIT - Timer (ports 0x40-0x5F)
        // - Keyboard controller (ports 0x60, 0x64)
        // - Floppy controller (ports 0x3F0-0x3F7)

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
    ) -> Result<()> {
        // For now, delegate to the basic init
        // In the future, we could pass pc_system pointer to handlers
        self.init(_mem)
    }

    /// Reset all devices
    /// 
    /// # Arguments
    /// * `reset_type` - Type of reset (Hardware or Software)
    pub fn reset(&mut self, reset_type: ResetReason) -> Result<()> {
        match reset_type {
            ResetReason::Hardware => {
                tracing::info!("Device hardware reset");
                #[cfg(feature = "bx_support_pci")]
                {
                    // Clear PCI configuration address
                    self.pci_conf_addr = 0;
                }
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

/// Port 92h read handler
/// 
/// Returns the current state of the System Control Port
fn port92_read_handler(_this_ptr: *mut c_void, _port: u16, _io_len: u8) -> u32 {
    // In a full implementation, this would read from stored state
    // For now, return A20 enabled (bit 0 = 1)
    tracing::trace!("Port 92h read");
    0x02 // A20 enabled, no reset pending
}

/// Port 92h write handler
/// 
/// Handles A20 gate control and fast reset
fn port92_write_handler(_this_ptr: *mut c_void, _port: u16, value: u32, _io_len: u8) {
    let value = value as u8;
    tracing::debug!("Port 92h write: value={:#04x}", value);

    // Bit 0: A20 gate (directly controls A20 line)
    let a20_enabled = (value & 0x01) != 0;
    if a20_enabled {
        tracing::debug!("Port 92h: A20 line enabled via fast gate");
    } else {
        tracing::debug!("Port 92h: A20 line disabled via fast gate");
    }
    // Note: In a full implementation, this would call pc_system.set_enable_a20()
    // The Emulator struct coordinates this by monitoring port 92h state

    // Bit 1: Fast reset (pulse triggers CPU reset)
    if (value & 0x02) != 0 {
        tracing::warn!("Port 92h: Fast reset requested (bit 1 set)");
        // Note: In a full implementation, this would trigger a CPU reset
        // The Emulator struct handles this by checking the reset flag
    }

    // Other bits are typically undefined/reserved
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
        self.a20_gate = (value & 0x01) != 0;
        self.reset_request = (value & 0x02) != 0;

        // Return true if A20 state changed
        old_a20 != self.a20_gate
    }

    /// Read current port 92h value
    pub fn read(&self) -> u8 {
        let mut value = 0u8;
        if self.a20_gate {
            value |= 0x01;
        }
        // Bit 1 is write-only (reset trigger), reads as 0
        value
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

        // Disable A20
        let changed = port.write(0x00);
        assert!(changed); // State changed
        assert!(!port.a20_gate);

        // Enable A20 again
        let changed = port.write(0x01);
        assert!(changed);
        assert!(port.a20_gate);

        // Write same value (no change)
        let changed = port.write(0x01);
        assert!(!changed);

        // Trigger reset
        port.write(0x02);
        assert!(port.reset_request);
    }
}
