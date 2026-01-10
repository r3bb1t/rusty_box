//! Device Initialization and Management
//!
//! This module implements the device initialization sequence from Bochs,
//! including the Port 0x92 System Control handler for A20 line control.

use core::ffi::c_void;

use crate::{
    cpu::ResetReason,
    memory::BxMemC,
    pc_system::BxPcSystemC,
    Result,
};

use super::BxDevicesC;

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
