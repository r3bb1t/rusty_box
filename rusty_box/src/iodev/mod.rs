//! I/O Device Subsystem
//!
//! This module provides the I/O port handling infrastructure for the emulator.
//! It manages 65536 I/O ports (0x0000 - 0xFFFF) with support for custom handlers.
//!
//! Each `BxDevicesC` instance is fully independent, allowing multiple
//! emulator instances to run concurrently without conflicts.
//!
//! ## Device Modules
//!
//! The following hardware devices are emulated:
//! - **PIC (8259)**: Programmable Interrupt Controller - handles hardware interrupts
//! - **PIT (8254)**: Programmable Interval Timer - system timer, speaker control
//! - **CMOS/RTC**: CMOS RAM and Real Time Clock
//! - **DMA (8237)**: Direct Memory Access controller
//! - **Keyboard (8042)**: PS/2 keyboard and mouse controller
//! - **HardDrive (ATA/IDE)**: Hard disk controller

use alloc::{collections::VecDeque, string::String, vec::Vec};
use core::ffi::c_void;

pub mod acpi;
pub mod cmos;
pub mod devices;
pub mod dma;
pub mod harddrv;
pub mod ioapic;
pub mod keyboard;
pub mod pci;
pub mod pci2isa;
pub mod pci_ide;
pub mod pic;
pub mod pit;
pub mod vga;

// Re-export device types for convenience
pub use acpi::BxAcpiCtrl;
pub use cmos::BxCmosC;
pub use dma::BxDmaC;
pub use harddrv::BxHardDriveC;
pub use ioapic::BxIoApic;
pub use keyboard::BxKeyboardC;
pub use pci::BxPciBridge;
pub use pci2isa::BxPiix3;
pub use pci_ide::BxPciIde;
pub use pic::BxPicC;
pub use pit::BxPitC;
// BxVgaC is pub(crate) - not exported outside the crate

/// Number of I/O ports (0x0000 - 0xFFFF)
pub const IO_PORTS: usize = 0x10000;

/// I/O read handler function type
///
/// # Arguments
/// * `this_ptr` - Pointer to device instance
/// * `address` - I/O port address
/// * `io_len` - Length of I/O operation (1, 2, or 4 bytes)
///
/// # Returns
/// The value read from the port
pub type IoReadHandlerT = fn(this_ptr: *mut c_void, address: u16, io_len: u8) -> u32;

/// I/O write handler function type
///
/// # Arguments
/// * `this_ptr` - Pointer to device instance
/// * `address` - I/O port address
/// * `value` - Value to write
/// * `io_len` - Length of I/O operation (1, 2, or 4 bytes)
pub type IoWriteHandlerT = fn(this_ptr: *mut c_void, address: u16, value: u32, io_len: u8);

/// I/O handler registration structure
#[derive(Clone)]
pub struct IoHandlerEntry {
    /// Handler function
    pub(crate) handler: Option<IoReadHandlerT>,
    /// Write handler function
    pub(crate) write_handler: Option<IoWriteHandlerT>,
    /// Device instance pointer
    pub(crate) this_ptr: *mut c_void,
    /// Handler name for debugging
    pub(crate) name: String,
    /// I/O length mask (bit 0 = 1 byte, bit 1 = 2 bytes, bit 2 = 4 bytes)
    pub(crate) mask: u8,
}

impl Default for IoHandlerEntry {
    fn default() -> Self {
        Self {
            handler: None,
            write_handler: None,
            this_ptr: core::ptr::null_mut(),
            name: String::new(),
            mask: 0x7, // All lengths supported by default
        }
    }
}

// SAFETY: IoHandlerEntry's raw pointer is only dereferenced within single-threaded emulator context
unsafe impl Send for IoHandlerEntry {}
unsafe impl Sync for IoHandlerEntry {}

/// Device controller - manages all I/O devices and port handlers
///
/// This struct is fully instance-based with no global state, allowing multiple
/// independent emulator instances to run concurrently.
pub struct BxDevicesC {
    /// Read handlers indexed by port number
    read_handlers: Vec<IoHandlerEntry>,
    /// Write handlers indexed by port number
    write_handlers: Vec<IoHandlerEntry>,
    /// PCI enabled flag
    pci_enabled: bool,
    /// PCI configuration address register (port 0xCF8)
    #[cfg(feature = "bx_support_pci")]
    pci_conf_addr: u32,

    /// Bochs BIOS/debug output ports (always-on).
    ///
    /// Bochs' rombios uses:
    /// - `INFO_PORT`  0x402
    /// - `DEBUG_PORT` 0x403
    /// VGABIOS also supports an info port (0x500).
    ///
    /// We funnel these into a single byte stream buffer. Host code (examples/GUI)
    /// can drain and print it.
    port_e9_output: VecDeque<u8>,

    /// Bochs BIOS POST codes (port 0x80, sometimes 0x84).
    ///
    /// These are not ASCII; they are diagnostic progress codes used by many BIOSes.
    port80_output: VecDeque<u8>,

    /// Last I/O read port and value (for stuck-loop diagnostics)
    pub(crate) last_io_read_port: u16,
    pub(crate) last_io_read_value: u32,
}

impl Default for BxDevicesC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxDevicesC {
    /// Create a new device controller instance
    pub fn new() -> Self {
        // Create handler arrays with default entries
        let mut read_handlers = Vec::with_capacity(IO_PORTS);
        let mut write_handlers = Vec::with_capacity(IO_PORTS);

        for _ in 0..IO_PORTS {
            read_handlers.push(IoHandlerEntry::default());
            write_handlers.push(IoHandlerEntry::default());
        }

        Self {
            read_handlers,
            write_handlers,
            pci_enabled: false,
            #[cfg(feature = "bx_support_pci")]
            pci_conf_addr: 0,
            port_e9_output: VecDeque::new(),
            port80_output: VecDeque::new(),
            last_io_read_port: 0,
            last_io_read_value: 0,
        }
    }

    /// Register a read handler for a specific I/O port
    ///
    /// # Arguments
    /// * `this_ptr` - Pointer to device instance
    /// * `handler` - Handler function
    /// * `port` - I/O port address
    /// * `name` - Handler name for debugging
    /// * `mask` - I/O length mask
    pub fn register_io_read_handler(
        &mut self,
        this_ptr: *mut c_void,
        handler: IoReadHandlerT,
        port: u16,
        name: &str,
        mask: u8,
    ) {
        let entry = &mut self.read_handlers[port as usize];
        entry.handler = Some(handler);
        entry.this_ptr = this_ptr;
        entry.name = String::from(name);
        entry.mask = mask;
        tracing::debug!(
            "Registered I/O read handler for port {:#06x}: {}",
            port,
            name
        );
    }

    /// Register a write handler for a specific I/O port
    pub fn register_io_write_handler(
        &mut self,
        this_ptr: *mut c_void,
        handler: IoWriteHandlerT,
        port: u16,
        name: &str,
        mask: u8,
    ) {
        let entry = &mut self.write_handlers[port as usize];
        entry.write_handler = Some(handler);
        entry.this_ptr = this_ptr;
        entry.name = String::from(name);
        entry.mask = mask;
        tracing::debug!(
            "Registered I/O write handler for port {:#06x}: {}",
            port,
            name
        );
    }

    /// Register both read and write handlers for a port
    pub fn register_io_handler(
        &mut self,
        this_ptr: *mut c_void,
        read_handler: IoReadHandlerT,
        write_handler: IoWriteHandlerT,
        port: u16,
        name: &str,
        mask: u8,
    ) {
        self.register_io_read_handler(this_ptr, read_handler, port, name, mask);
        self.register_io_write_handler(this_ptr, write_handler, port, name, mask);
    }

    /// Read from an I/O port
    ///
    /// # Arguments
    /// * `port` - I/O port address
    /// * `io_len` - Length of I/O operation (1, 2, or 4 bytes)
    ///
    /// # Returns
    /// The value read from the port
    pub fn inp(&mut self, port: u16, io_len: u8) -> u32 {
        let entry = &self.read_handlers[port as usize];

        let value = if let Some(handler) = entry.handler {
            // Check if the requested I/O length is supported
            let len_mask = 1u8 << (io_len.trailing_zeros() as u8);
            if (entry.mask & len_mask) != 0 {
                handler(entry.this_ptr, port, io_len)
            } else {
                // Handler exists but mask doesn't match - log this
                tracing::debug!("I/O read port={:#06x}: handler '{}' exists but mask={:#x} doesn't support len={}",
                    port, entry.name, entry.mask, io_len);
                self.default_read_handler(port, io_len)
            }
        } else {
            // Default: return all 1s for unhandled reads
            self.default_read_handler(port, io_len)
        };

        self.last_io_read_port = port;
        self.last_io_read_value = value;
        value
    }

    /// Write to an I/O port
    ///
    /// # Arguments
    /// * `port` - I/O port address
    /// * `value` - Value to write
    /// * `io_len` - Length of I/O operation (1, 2, or 4 bytes)
    pub fn outp(&mut self, port: u16, value: u32, io_len: u8) {
        let entry = &self.write_handlers[port as usize];

        if let Some(handler) = entry.write_handler {
            // Check if the requested I/O length is supported
            let len_mask = 1u8 << (io_len.trailing_zeros() as u8);
            if (entry.mask & len_mask) != 0 {
                handler(entry.this_ptr, port, value, io_len);
                return;
            }
        }

        // Default: ignore unhandled writes
        self.default_write_handler(port, value, io_len);
    }

    /// Default read handler - returns 0xFFFFFFFF for unhandled ports
    fn default_read_handler(&self, address: u16, io_len: u8) -> u32 {
        // Bochs port 0xE9 hack (mirrors `cpp_orig/bochs/iodev/unmapped.cc` behavior when enabled):
        // - reading returns 0xE9 (casted to io_len)
        let mut retval: u32 = 0xFFFF_FFFF;
        if address == 0x00E9 {
            retval = 0xE9;
        } else {
            tracing::trace!(
                "Unhandled I/O read: port={:#06x}, len={} -> 0xFF..F",
                address,
                io_len
            );
        }

        match io_len {
            1 => retval & 0xFF,
            2 => retval & 0xFFFF,
            4 => retval,
            _ => retval,
        }
    }

    /// Default write handler - ignores writes to unhandled ports
    fn default_write_handler(&mut self, address: u16, value: u32, io_len: u8) {
        // Bochs-style BIOS POST code port (0x80). Some BIOSes also use 0x84.
        if io_len == 1 && matches!(address, 0x0080 | 0x0084) {
            tracing::debug!("BIOS POST code port {:#06x}: {:#04x}", address, value as u8);
            const PORT80_CAPACITY: usize = 4096;
            if self.port80_output.len() >= PORT80_CAPACITY {
                self.port80_output.pop_front();
            }
            self.port80_output.push_back(value as u8);
            return;
        }

        // Bochs-style debug output ports: capture bytes into a host-drainable buffer.
        //
        // - 0xE9: Bochs debug console (optional in upstream; always-on here)
        // - 0x402/0x403: Bochs rombios INFO/DEBUG ports (cpp_orig/bochs/bios/rombios.h)
        // - 0x500: VGABIOS info port (cpp_orig/bochs/bios/VGABIOS-lgpl-README)
        if io_len == 1 && matches!(address, 0x00E9 | 0x0402 | 0x0403 | 0x0500) {
            tracing::debug!(
                "BIOS output port {:#06x}: {:?}",
                address,
                value as u8 as char
            );
            const PORT_E9_CAPACITY: usize = 4096;
            if self.port_e9_output.len() >= PORT_E9_CAPACITY {
                self.port_e9_output.pop_front();
            }
            self.port_e9_output.push_back(value as u8);
            return;
        }

        tracing::trace!(
            "Unhandled I/O write: port={:#06x}, value={:#x}, len={}",
            address,
            value,
            io_len
        );
    }

    /// Check if PCI is enabled
    pub fn is_pci_enabled(&self) -> bool {
        self.pci_enabled
    }

    /// Set PCI enabled state
    pub fn set_pci_enabled(&mut self, enabled: bool) {
        self.pci_enabled = enabled;
    }

    /// Drain and return bytes written to port 0xE9.
    ///
    /// This is alloc-only; callers can print/interpret the bytes however they want.
    pub fn take_port_e9_output(&mut self) -> Vec<u8> {
        self.port_e9_output.drain(..).collect()
    }

    /// Drain and return BIOS POST codes written to port 0x80/0x84.
    pub fn take_port80_output(&mut self) -> Vec<u8> {
        self.port80_output.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_handlers() {
        let mut devices = BxDevicesC::new();

        // Reading unhandled port should return 0xFF/0xFFFF/0xFFFFFFFF
        assert_eq!(devices.inp(0x1234, 1), 0xFF);
        assert_eq!(devices.inp(0x1234, 2), 0xFFFF);
        assert_eq!(devices.inp(0x1234, 4), 0xFFFFFFFF);
    }

    #[test]
    fn test_multiple_instances() {
        let mut dev1 = BxDevicesC::new();
        let dev2 = BxDevicesC::new();

        // Custom handler that returns port number
        fn custom_read(_: *mut c_void, port: u16, _: u8) -> u32 {
            port as u32 * 2
        }

        // Register handler only on dev1
        dev1.register_io_read_handler(core::ptr::null_mut(), custom_read, 0x100, "test", 0x1);

        // dev1 should return custom value, dev2 should return default
        assert_eq!(dev1.inp(0x100, 1), 0x200);
        let mut dev2 = dev2;
        assert_eq!(dev2.inp(0x100, 1), 0xFF);
    }
}
