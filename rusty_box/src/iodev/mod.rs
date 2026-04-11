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
pub mod serial;
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
pub use serial::BxSerialC;
// BxVgaC is pub(crate) - not exported outside the crate

/// Number of I/O ports (0x0000 - 0xFFFF)
pub const IO_PORTS: usize = 0x10000;

/// Identifies which hardware device owns an I/O port registration.
///
/// Used for safe enum-based dispatch instead of C-style `fn ptr + *mut c_void`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceId {
    /// No device registered (unhandled port)
    None,
    /// 8259 PIC (Programmable Interrupt Controller)
    Pic,
    /// 8254 PIT (Programmable Interval Timer)
    Pit,
    /// CMOS/RTC
    Cmos,
    /// 8237 DMA Controller
    Dma,
    /// 8042 Keyboard/Mouse Controller
    Keyboard,
    /// ATA/IDE Hard Drive Controller
    HardDrive,
    /// 16550 UART Serial Port
    Serial,
    /// VGA Display Controller
    Vga,
    /// Port 92h System Control (A20/reset)
    Port92,
    /// PCI bus (config addr/data, PIIX3 ELCR, BM-DMA)
    Pci,
    /// PCI IDE Controller (BM-DMA ports)
    PciIde,
    /// PIIX4 ACPI Power Management
    Acpi,
    /// I/O APIC (MMIO-only, no port I/O)
    Ioapic,
}

/// I/O handler registration entry for a single port.
///
/// Each port maps to a `DeviceId` for safe dispatch through `DeviceManager`.
#[derive(Clone)]
pub struct IoHandlerEntry {
    /// Which device owns this port
    pub(crate) device_id: DeviceId,
    /// Handler name for debugging
    pub(crate) name: String,
    /// I/O length mask (bit 0 = 1 byte, bit 1 = 2 bytes, bit 2 = 4 bytes)
    pub(crate) mask: u8,
}

impl Default for IoHandlerEntry {
    fn default() -> Self {
        Self {
            device_id: DeviceId::None,
            name: String::new(),
            mask: 0x7, // All lengths supported by default
        }
    }
}

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
    ///
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
    /// Total I/O port reads (for progress diagnostics)
    pub(crate) diag_io_reads: u64,
    /// Total I/O port writes
    pub(crate) diag_io_writes: u64,
    /// Pointer to DeviceManager for enum-based I/O dispatch.
    /// Set by the emulator before CPU execution; single-threaded.
    device_manager: Option<core::ptr::NonNull<devices::DeviceManager>>,
    /// Pointer to BxMemC for immediate PAM updates during PCI writes.
    /// Set by the emulator before CPU execution; single-threaded.
    mem_ptr: Option<core::ptr::NonNull<crate::memory::BxMemC<'static>>>,
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
            diag_io_reads: 0,
            diag_io_writes: 0,
            device_manager: None,
            mem_ptr: None,
        }
    }

    /// Register a read handler for a specific I/O port
    pub fn register_io_read_handler(
        &mut self,
        device_id: DeviceId,
        port: u16,
        name: &str,
        mask: u8,
    ) {
        let entry = &mut self.read_handlers[port as usize];
        entry.device_id = device_id;
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
        device_id: DeviceId,
        port: u16,
        name: &str,
        mask: u8,
    ) {
        let entry = &mut self.write_handlers[port as usize];
        entry.device_id = device_id;
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
        device_id: DeviceId,
        port: u16,
        name: &str,
        mask: u8,
    ) {
        self.register_io_read_handler(device_id, port, name, mask);
        self.register_io_write_handler(device_id, port, name, mask);
    }

    /// Read from an I/O port
    ///
    /// # Arguments
    /// * `port` - I/O port address
    /// * `io_len` - Length of I/O operation (1, 2, or 4 bytes)
    ///
    /// # Returns
    /// The value read from the port
    pub fn inp(&mut self, port: u16, io_len: u8, icount: u64) -> u32 {
        self.diag_io_reads += 1;
        let entry = &self.read_handlers[port as usize];
        let len_mask = 1u8 << (io_len.trailing_zeros() as u8);

        let value = if entry.device_id != DeviceId::None && (entry.mask & len_mask) != 0 {
            if let Some(mut dm) = self.device_manager {
                // SAFETY: device_manager set by emulator for execution duration; single-threaded
                let dm = unsafe { dm.as_mut() };
                let result = Self::dispatch_read(dm, entry.device_id, port, io_len, icount);
                // Drain PIC IOAPIC forwarding queue after device handler
                #[cfg(feature = "bx_support_apic")]
                {
                    let (fwds, count) = dm.pic.take_ioapic_forwards();
                    for &(irq, level) in &fwds[..count] {
                        dm.ioapic.set_irq_level(
                            irq,
                            level,
                            None,
                            #[cfg(feature = "bx_support_apic")]
                            None,
                        );
                    }
                }
                result
            } else {
                self.default_read_handler(port, io_len)
            }
        } else {
            self.default_read_handler(port, io_len)
        };

        self.last_io_read_port = port;
        self.last_io_read_value = value;
        value
    }

    /// Write to an I/O port
    pub fn outp(&mut self, port: u16, value: u32, io_len: u8) {
        self.diag_io_writes += 1;
        let entry = &self.write_handlers[port as usize];
        let len_mask = 1u8 << (io_len.trailing_zeros() as u8);

        if entry.device_id != DeviceId::None && (entry.mask & len_mask) != 0 {
            if let Some(mut dm) = self.device_manager {
                // SAFETY: device_manager set by emulator for execution duration; single-threaded
                let dm = unsafe { dm.as_mut() };
                Self::dispatch_write(dm, entry.device_id, port, value, io_len);
                // Drain PIC IOAPIC forwarding queue after device handler
                #[cfg(feature = "bx_support_apic")]
                {
                    let (fwds, count) = dm.pic.take_ioapic_forwards();
                    for &(irq, level) in &fwds[..count] {
                        dm.ioapic.set_irq_level(
                            irq,
                            level,
                            None,
                            #[cfg(feature = "bx_support_apic")]
                            None,
                        );
                    }
                }
                // Apply PAM register changes immediately during PCI writes.
                // BIOS reads from shadowed regions right after writing PAM.
                #[cfg(feature = "bx_support_pci")]
                if dm.pam_needs_update {
                    if let Some(mut mem) = self.mem_ptr {
                        // SAFETY: mem_ptr set by emulator; single-threaded
                        dm.pam_needs_update = false;
                        dm.pci_bridge.apply_pam_to_memory(unsafe { mem.as_mut() });
                    }
                }
                return;
            }
        }

        self.default_write_handler(port, value, io_len);
    }

    /// Bulk-read from an I/O port.
    ///
    /// For IDE data ports (0x1F0, 0x170), this copies up to `buf.len()` bytes
    /// directly from the ATA controller buffer in one call, avoiding per-word
    /// handler dispatch overhead. Returns the number of bytes actually read.
    /// For other ports, returns 0 (caller should fall back to per-word I/O).
    pub fn inp_bulk(&mut self, port: u16, buf: &mut [u8]) -> usize {
        // Only optimize IDE data ports (base + 0 = data register)
        if port != 0x1F0 && port != 0x170 {
            return 0;
        }
        let entry = &self.read_handlers[port as usize];
        if entry.device_id != DeviceId::HardDrive {
            return 0;
        }
        if let Some(mut dm) = self.device_manager {
            // SAFETY: device_manager set by emulator for execution duration; single-threaded
            let dm = unsafe { dm.as_mut() };
            #[cfg(feature = "bx_support_pci")]
            {
                let devices::DeviceManager { ref mut harddrv, ref mut pic, ref mut pci_ide, .. } = *dm;
                harddrv.bulk_read_data(port, buf, pic, pci_ide)
            }
            #[cfg(not(feature = "bx_support_pci"))]
            {
                let devices::DeviceManager { ref mut harddrv, ref mut pic, .. } = *dm;
                let mut stub_pci_ide = super::pci_ide::BxPciIde::new();
                harddrv.bulk_read_data(port, buf, pic, &mut stub_pci_ide)
            }
        } else {
            0
        }
    }

    /// Default read handler - returns 0xFFFFFFFF for unhandled ports
    fn default_read_handler(&self, address: u16, io_len: u8) -> u32 {
        // Bochs port 0xE9 hack (mirrors `cpp_orig/bochs/iodev/unmapped.cc` behavior when enabled):
        // - reading returns 0xE9 (casted to io_len)
        let mut retval: u32 = 0xFFFF_FFFF;
        if address == 0x00E9 {
            retval = 0xE9;
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
            const PORT_E9_CAPACITY: usize = 65536;
            if self.port_e9_output.len() >= PORT_E9_CAPACITY {
                self.port_e9_output.pop_front();
            }
            self.port_e9_output.push_back(value as u8);
        }

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

    /// Set device_manager pointer for enum-based I/O dispatch.
    /// Called by emulator before CPU execution.
    pub fn set_device_manager(&mut self, dm: core::ptr::NonNull<devices::DeviceManager>) {
        self.device_manager = Some(dm);
    }

    /// Clear device_manager pointer after CPU execution.
    pub fn clear_device_manager(&mut self) {
        self.device_manager = None;
    }

    /// Set BxMemC pointer for immediate PAM updates during PCI writes.
    pub fn set_mem_ptr(&mut self, mem: core::ptr::NonNull<crate::memory::BxMemC<'static>>) {
        self.mem_ptr = Some(mem);
    }

    /// Clear mem pointer after CPU execution.
    pub fn clear_mem_ptr(&mut self) {
        self.mem_ptr = None;
    }

    /// Dispatch a port read to the device identified by `id`.
    fn dispatch_read(dm: &mut devices::DeviceManager, id: DeviceId, port: u16, io_len: u8, icount: u64) -> u32 {
        match id {
            DeviceId::Pic => dm.pic.read(port, io_len),
            DeviceId::Pit => dm.pit.read(port, io_len, icount),
            DeviceId::Cmos => dm.cmos.read(port, io_len),
            DeviceId::Dma => dm.dma.read(port, io_len),
            DeviceId::Keyboard => {
                let devices::DeviceManager { ref mut keyboard, ref mut pit, .. } = *dm;
                keyboard.read(port, io_len, icount, Some(pit))
            }
            #[cfg(feature = "bx_support_pci")]
            DeviceId::HardDrive => {
                let devices::DeviceManager { ref mut harddrv, ref mut pic, ref mut pci_ide, .. } = *dm;
                harddrv.read(port, io_len, pic, pci_ide)
            }
            #[cfg(not(feature = "bx_support_pci"))]
            DeviceId::HardDrive => {
                let devices::DeviceManager { ref mut harddrv, ref mut pic, .. } = *dm;
                let mut stub_pci_ide = super::pci_ide::BxPciIde::new();
                harddrv.read(port, io_len, pic, &mut stub_pci_ide)
            }
            DeviceId::Serial => dm.serial.read(port, io_len),
            DeviceId::Vga => dm.vga.read_port(port, io_len, icount),
            DeviceId::Port92 => dm.port92_read(port, io_len),
            #[cfg(feature = "bx_support_pci")]
            DeviceId::Pci => dm.pci_read(port, io_len),
            #[cfg(feature = "bx_support_pci")]
            DeviceId::Acpi => dm.acpi_read(port, io_len),
            #[cfg(feature = "bx_support_pci")]
            DeviceId::PciIde => dm.pci_ide_read(port, io_len),
            // Without PCI feature, these variants are unreachable but must compile
            #[cfg(not(feature = "bx_support_pci"))]
            DeviceId::Pci | DeviceId::Acpi | DeviceId::PciIde => 0xFFFF_FFFF,
            DeviceId::Ioapic => 0xFF, // IOAPIC uses MMIO, not port I/O
            DeviceId::None => 0xFFFF_FFFF,
        }
    }

    /// Dispatch a port write to the device identified by `id`.
    fn dispatch_write(dm: &mut devices::DeviceManager, id: DeviceId, port: u16, value: u32, io_len: u8) {
        match id {
            DeviceId::Pic => dm.pic.write(port, value, io_len),
            DeviceId::Pit => dm.pit.write(port, value, io_len),
            DeviceId::Cmos => dm.cmos.write(port, value, io_len),
            DeviceId::Dma => dm.dma.write(port, value, io_len),
            DeviceId::Keyboard => {
                let devices::DeviceManager { ref mut keyboard, ref mut pit, .. } = *dm;
                keyboard.write(port, value, io_len, Some(pit))
            }
            #[cfg(feature = "bx_support_pci")]
            DeviceId::HardDrive => {
                let devices::DeviceManager { ref mut harddrv, ref mut pic, ref mut pci_ide, .. } = *dm;
                harddrv.write(port, value, io_len, pic, pci_ide)
            }
            #[cfg(not(feature = "bx_support_pci"))]
            DeviceId::HardDrive => {
                let devices::DeviceManager { ref mut harddrv, ref mut pic, .. } = *dm;
                let mut stub_pci_ide = super::pci_ide::BxPciIde::new();
                harddrv.write(port, value, io_len, pic, &mut stub_pci_ide)
            }
            DeviceId::Serial => dm.serial.write(port, value, io_len),
            DeviceId::Vga => dm.vga.write_port(port, value, io_len),
            DeviceId::Port92 => dm.port92_write(port, value, io_len),
            #[cfg(feature = "bx_support_pci")]
            DeviceId::Pci => dm.pci_write(port, value, io_len),
            #[cfg(feature = "bx_support_pci")]
            DeviceId::Acpi => dm.acpi_write(port, value, io_len),
            #[cfg(feature = "bx_support_pci")]
            DeviceId::PciIde => dm.pci_ide_write(port, value, io_len),
            #[cfg(not(feature = "bx_support_pci"))]
            DeviceId::Pci | DeviceId::Acpi | DeviceId::PciIde => {},
            DeviceId::Ioapic | DeviceId::None => {},
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_handlers() {
        let mut devices = BxDevicesC::new();

        // Reading unhandled port should return 0xFF/0xFFFF/0xFFFFFFFF
        assert_eq!(devices.inp(0x1234, 1, 0), 0xFF);
        assert_eq!(devices.inp(0x1234, 2, 0), 0xFFFF);
        assert_eq!(devices.inp(0x1234, 4, 0), 0xFFFFFFFF);
    }

    #[test]
    fn test_multiple_instances() {
        let mut dev1 = BxDevicesC::new();
        let mut dev2 = BxDevicesC::new();

        // Register handler only on dev1
        dev1.register_io_read_handler(DeviceId::Pic, 0x100, "test", 0x1);

        // dev1 has a device registered, dev2 does not.
        // Without a device_manager, both return default.
        assert_eq!(dev1.inp(0x100, 1, 0), 0xFF);
        assert_eq!(dev2.inp(0x100, 1, 0), 0xFF);
    }
}
