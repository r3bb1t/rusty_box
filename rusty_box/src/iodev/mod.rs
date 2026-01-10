//! I/O Device Subsystem
//!
//! This module provides the I/O port handling infrastructure for the emulator.
//! It manages 65536 I/O ports (0x0000 - 0xFFFF) with support for custom handlers.
//!
//! Each `BxDevicesC` instance is fully independent, allowing multiple
//! emulator instances to run concurrently without conflicts.

use alloc::{boxed::Box, string::String, vec::Vec};
use core::ffi::c_void;

pub mod devices;

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
    pub handler: Option<IoReadHandlerT>,
    /// Write handler function
    pub write_handler: Option<IoWriteHandlerT>,
    /// Device instance pointer
    pub this_ptr: *mut c_void,
    /// Handler name for debugging
    pub name: String,
    /// I/O length mask (bit 0 = 1 byte, bit 1 = 2 bytes, bit 2 = 4 bytes)
    pub mask: u8,
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
        tracing::debug!("Registered I/O read handler for port {:#06x}: {}", port, name);
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
        tracing::debug!("Registered I/O write handler for port {:#06x}: {}", port, name);
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
    pub fn inp(&self, port: u16, io_len: u8) -> u32 {
        let entry = &self.read_handlers[port as usize];
        
        if let Some(handler) = entry.handler {
            // Check if the requested I/O length is supported
            let len_mask = 1u8 << (io_len.trailing_zeros() as u8);
            if (entry.mask & len_mask) != 0 {
                return handler(entry.this_ptr, port, io_len);
            }
        }
        
        // Default: return all 1s for unhandled reads
        self.default_read_handler(port, io_len)
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
        tracing::trace!("Unhandled I/O read: port={:#06x}, len={}", address, io_len);
        match io_len {
            1 => 0xFF,
            2 => 0xFFFF,
            4 => 0xFFFFFFFF,
            _ => 0xFFFFFFFF,
        }
    }

    /// Default write handler - ignores writes to unhandled ports
    fn default_write_handler(&self, address: u16, value: u32, io_len: u8) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_handlers() {
        let devices = BxDevicesC::new();
        
        // Reading unhandled port should return 0xFF/0xFFFF/0xFFFFFFFF
        assert_eq!(devices.inp(0x1234, 1), 0xFF);
        assert_eq!(devices.inp(0x1234, 2), 0xFFFF);
        assert_eq!(devices.inp(0x1234, 4), 0xFFFFFFFF);
    }

    #[test]
    fn test_multiple_instances() {
        let mut dev1 = BxDevicesC::new();
        let mut dev2 = BxDevicesC::new();

        // Custom handler that returns port number
        fn custom_read(_: *mut c_void, port: u16, _: u8) -> u32 {
            port as u32 * 2
        }

        // Register handler only on dev1
        dev1.register_io_read_handler(
            core::ptr::null_mut(),
            custom_read,
            0x100,
            "test",
            0x1,
        );

        // dev1 should return custom value, dev2 should return default
        assert_eq!(dev1.inp(0x100, 1), 0x200);
        assert_eq!(dev2.inp(0x100, 1), 0xFF);
    }
}
