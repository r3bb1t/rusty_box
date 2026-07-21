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

use crate::ring_buffer::RingBuffer;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
#[cfg(feature = "std")]
use std::io::{self, Error, ErrorKind, Read, Write};

#[cfg(feature = "std")]
use crate::snapshot::{checked_snapshot_len_add, SnapshotReader, SnapshotWriteExt};


pub mod acpi;
#[cfg(feature = "alloc")]
pub mod acpi_tables;
pub mod cmos;
pub mod ddc;
pub mod devices;
pub use crate::dma;
pub mod fw_cfg;
pub mod harddrv;
pub mod ioapic;
pub mod keyboard;
pub mod pci;
pub mod pci2isa;
pub mod pci_ide;
pub use crate::pic;
#[cfg(feature = "alloc")]
pub mod geforce;
pub mod pit;
pub mod serial;
pub mod vga;

// Re-export device types for convenience
pub use acpi::BxAcpiCtrl;
pub use cmos::BxCmosC;
pub use dma::BxDmaC;
pub use fw_cfg::BxFwCfg;
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
#[cfg(feature = "alloc")]
pub use geforce::BxGeForceC;

/// Number of I/O ports (0x0000 - 0xFFFF)
pub const IO_PORTS: usize = 0x10000;
/// Number of serial timer-owner slots reserved by the no-allocation I/O
/// scheduler transport. The current machine wires one UART; the remaining
/// slots make the transport independent of a later topology expansion.
pub(crate) const BX_FIXED_SERIAL_TIMER_OWNERS: usize = 4;

/// Number of fixed device timer owners carried across the raw I/O boundary.
///
/// LAPIC requests use their CPU-local transport. Every device request below
/// has exactly one stable slot, so a producer can overwrite its own pending
/// work without allocating or scanning a timer list.
pub(crate) const BX_FIXED_TIMER_OWNER_COUNT: usize = 6 + BX_FIXED_SERIAL_TIMER_OWNERS + 2;

/// A device-owned timer slot in the fixed scheduler transport.
#[allow(dead_code)] // Phase 3 device producers fill the reserved owner slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceTimerOwner {
    Pit,
    Keyboard,
    CmosPeriodic,
    CmosOneSecond,
    CmosUip,
    AcpiPmOverflow,
    SerialFifo(usize),
    PciIdeCh0,
    PciIdeCh1,
}

impl DeviceTimerOwner {
    #[inline]
    const fn slot(self) -> Option<usize> {
        match self {
            Self::Pit => Some(0),
            Self::Keyboard => Some(1),
            Self::CmosPeriodic => Some(2),
            Self::CmosOneSecond => Some(3),
            Self::CmosUip => Some(4),
            Self::AcpiPmOverflow => Some(5),
            Self::SerialFifo(index) if index < BX_FIXED_SERIAL_TIMER_OWNERS => Some(6 + index),
            Self::SerialFifo(_) => None,
            Self::PciIdeCh0 => Some(6 + BX_FIXED_SERIAL_TIMER_OWNERS),
            Self::PciIdeCh1 => Some(7 + BX_FIXED_SERIAL_TIMER_OWNERS),
        }
    }
}

/// Deferred timer operation captured at the guest instruction which requested
/// it. The emulator applies it only after raw I/O borrows have been cleared.
#[allow(dead_code)] // Phase 3 adds device-side deactivate producers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum TimerRequest {
    #[default]
    Unchanged,
    Deactivate,
    Activate {
        deadline_ticks: u64,
        period_ticks: u64,
        continuous: bool,
    },
}

/// Fixed, no-allocation timer request table shared by I/O producers and the
/// central scheduler boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TimerRequestTable {
    slots: [TimerRequest; BX_FIXED_TIMER_OWNER_COUNT],
}

impl Default for TimerRequestTable {
    fn default() -> Self {
        Self {
            slots: [TimerRequest::Unchanged; BX_FIXED_TIMER_OWNER_COUNT],
        }
    }
}

impl TimerRequestTable {
    #[inline]
    pub(crate) const fn get(&self, owner: DeviceTimerOwner) -> TimerRequest {
        match owner.slot() {
            Some(slot) => self.slots[slot],
            None => TimerRequest::Unchanged,
        }
    }

    #[inline]
    fn overwrite(&mut self, owner: DeviceTimerOwner, request: TimerRequest) -> bool {
        let Some(slot) = owner.slot() else {
            return false;
        };
        let changed = self.slots[slot] != request;
        self.slots[slot] = request;
        changed
    }

    /// True when any owner slot holds a pending Activate/Deactivate request.
    #[inline]
    pub(crate) fn has_any_request(&self) -> bool {
        self.slots
            .iter()
            .any(|slot| !matches!(slot, TimerRequest::Unchanged))
    }
}

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
    /// QEMU fw_cfg Firmware Configuration Device
    FwCfg,
}

/// I/O handler registration entry for a single port.
///
/// Each port maps to a `DeviceId` for safe dispatch through `DeviceManager`.
#[derive(Clone, Copy)]
pub struct IoHandlerEntry {
    /// Which device owns this port
    pub(crate) device_id: DeviceId,
    /// Handler name for debugging
    pub(crate) name: &'static str,
    /// I/O length mask (bit 0 = 1 byte, bit 1 = 2 bytes, bit 2 = 4 bytes)
    pub(crate) mask: u8,
}

impl Default for IoHandlerEntry {
    fn default() -> Self {
        Self {
            device_id: DeviceId::None,
            name: "",
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
    read_handlers: [IoHandlerEntry; IO_PORTS],
    /// Write handlers indexed by port number
    write_handlers: [IoHandlerEntry; IO_PORTS],
    /// PCI enabled flag
    pci_enabled: bool,
    /// PCI configuration address register (port 0xCF8)
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
    port_e9_output: RingBuffer<u8, 65536>,

    /// Bochs BIOS POST codes (port 0x80, sometimes 0x84).
    ///
    /// These are not ASCII; they are diagnostic progress codes used by many BIOSes.
    port80_output: RingBuffer<u8, 4096>,

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
    /// Final physical INT level after the latest I/O dispatch which changed
    /// the PIC. This overwrites edge history so a clear followed by a reassert
    /// is observed by the CPU as asserted.
    pic_intr_level: Option<bool>,
    /// Final desired HRQ level after the latest I/O dispatch touched the 8237
    /// (Bochs pc_system.cc set_HRQ). Last writer wins, exactly like
    /// `pic_intr_level`.
    hrq_level: Option<bool>,
    /// Set whenever I/O queues scheduler-owned work that must be committed
    /// after raw bus pointers are torn down.
    scheduler_boundary_requested: bool,
    /// Fixed device timer requests captured during I/O dispatch.
    timer_requests: TimerRequestTable,
    /// PC-system timer frequency used to convert device microseconds into
    /// absolute scheduler ticks without borrowing `BxPcSystemC` during I/O.
    timer_ips: u64,
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
        let read_handlers = [IoHandlerEntry::default(); IO_PORTS];
        let write_handlers = [IoHandlerEntry::default(); IO_PORTS];

        Self {
            read_handlers,
            write_handlers,
            pci_enabled: false,
            pci_conf_addr: 0,
            port_e9_output: RingBuffer::new(),
            port80_output: RingBuffer::new(),
            last_io_read_port: 0,
            last_io_read_value: 0,
            diag_io_reads: 0,
            diag_io_writes: 0,
            device_manager: None,
            pic_intr_level: None,
            hrq_level: None,
            scheduler_boundary_requested: false,
            timer_requests: TimerRequestTable::default(),
            timer_ips: 1,
        }
    }
    /// Configure the scheduler tick frequency before device registration.
    #[inline]
    pub(crate) fn set_timer_ips(&mut self, ips: u64) {
        self.timer_ips = ips.max(1);
    }

    /// Queue a one-shot fixed-owner timer relative to the issuing instruction.
    #[inline]
    pub(crate) fn request_timer_after_usec(
        &mut self,
        owner: DeviceTimerOwner,
        current_ticks: u64,
        delay_usec: Option<u64>,
    ) {
        self.request_timer_after_usec_with_mode(owner, current_ticks, delay_usec, false);
    }

    #[inline]
    pub(crate) fn request_timer_after_usec_with_mode(
        &mut self,
        owner: DeviceTimerOwner,
        current_ticks: u64,
        delay_usec: Option<u64>,
        continuous: bool,
    ) {
        let request = match delay_usec {
            Some(usec) => {
                let ticks = (u128::from(usec) * u128::from(self.timer_ips))
                    .div_ceil(1_000_000)
                    .max(1)
                    .min(u128::from(u64::MAX)) as u64;
                TimerRequest::Activate {
                    deadline_ticks: current_ticks.saturating_add(ticks),
                    period_ticks: ticks,
                    continuous,
                }
            }
            None => TimerRequest::Deactivate,
        };
        self.request_timer(owner, request);
    }
    /// Non-consuming test of every I/O-latched boundary slot, used by the
    /// scheduler's no-work fast path. Covers exactly the sources
    /// `service_scheduler_boundary` drains from this layer: the final PIC and
    /// HRQ levels, the explicit boundary request, and fixed timer requests.
    #[inline]
    pub(crate) fn has_pending_boundary_work(&self) -> bool {
        self.pic_intr_level.is_some()
            || self.hrq_level.is_some()
            || self.scheduler_boundary_requested
            || self.timer_requests.has_any_request()
    }

    /// Drain the final PIC INT level observed during I/O dispatch.
    #[inline]
    pub(crate) fn take_pic_intr_level(&mut self) -> Option<bool> {
        self.pic_intr_level.take()
    }

    /// Drain the final desired HRQ level observed during I/O dispatch.
    #[inline]
    pub(crate) fn take_hrq_level(&mut self) -> Option<bool> {
        self.hrq_level.take()
    }

    /// Drain the scheduler-boundary request raised by I/O dispatch.
    #[inline]
    pub(crate) fn take_scheduler_boundary_requested(&mut self) -> bool {
        core::mem::take(&mut self.scheduler_boundary_requested)
    }

    /// Drain every fixed device timer request after raw I/O borrows are gone.
    #[inline]
    pub(crate) fn take_timer_requests(&mut self) -> TimerRequestTable {
        core::mem::take(&mut self.timer_requests)
    }

    /// Discard work captured before a machine reset. Reset dominates the
    /// boundary: no pre-reset interrupt level or timer operation may be
    /// committed after the reset returns.
    #[inline]
    pub(crate) fn discard_scheduler_boundary_work(&mut self) {
        self.pic_intr_level = None;
        self.hrq_level = None;
        self.scheduler_boundary_requested = false;
        self.timer_requests = TimerRequestTable::default();
    }

    /// Queue a device timer operation. Each producer owns one table slot and
    /// therefore overwrites only its own pending operation.
    #[inline]
    pub(crate) fn request_timer(&mut self, owner: DeviceTimerOwner, request: TimerRequest) {
        if self.timer_requests.overwrite(owner, request) {
            self.scheduler_boundary_requested = true;
        }
    }

    /// Consume PIC edge bookkeeping after a dispatch and collapse it to the
    /// final physical interrupt level.
    #[inline]
    fn take_pic_level_after_dispatch(dm: &mut devices::DeviceManager) -> Option<bool> {
        let changed = dm.pic.irq_pending || dm.pic.irq_cleared;
        dm.pic.irq_pending = false;
        dm.pic.irq_cleared = false;
        changed.then(|| dm.pic.has_interrupt())
    }

    /// Register a read handler for a specific I/O port
    pub fn register_io_read_handler(
        &mut self,
        device_id: DeviceId,
        port: u16,
        name: &'static str,
        mask: u8,
    ) {
        let entry = &mut self.read_handlers[port as usize];
        entry.device_id = device_id;
        entry.name = name;
        entry.mask = mask;
        tracing::trace!(
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
        name: &'static str,
        mask: u8,
    ) {
        let entry = &mut self.write_handlers[port as usize];
        entry.device_id = device_id;
        entry.name = name;
        entry.mask = mask;
        tracing::trace!(
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
        name: &'static str,
        mask: u8,
    ) {
        self.register_io_read_handler(device_id, port, name, mask);
        self.register_io_write_handler(device_id, port, name, mask);
    }

    /// Unregister the read and write handlers for a port, restoring the
    /// unhandled default. Bochs devices.cc `unregister_io_read_handler` +
    /// `unregister_io_write_handler` (used when a PCI BAR moves).
    pub fn unregister_io_handler(&mut self, port: u16) {
        self.read_handlers[port as usize] = IoHandlerEntry::default();
        self.write_handlers[port as usize] = IoHandlerEntry::default();
        tracing::trace!("Unregistered I/O handlers for port {:#06x}", port);
    }

    pub(crate) fn apply_cmos_timer_sync(
        &mut self,
        current_ticks: u64,
        sync: cmos::CmosTimerSync,
    ) {
        for (owner, action, continuous) in [
            (DeviceTimerOwner::CmosPeriodic, sync.periodic, true),
            (DeviceTimerOwner::CmosOneSecond, sync.one_second, true),
            (DeviceTimerOwner::CmosUip, sync.uip, false),
        ] {
            match action {
                cmos::CmosTimerAction::Unchanged => {}
                cmos::CmosTimerAction::Restart(delay) => {
                    self.request_timer_after_usec_with_mode(
                        owner,
                        current_ticks,
                        Some(delay),
                        continuous,
                    );
                }
                cmos::CmosTimerAction::Deactivate => {
                    self.request_timer_after_usec(owner, current_ticks, None);
                }
            }
        }
    }

    /// Read from an I/O port.
    #[inline]
    pub fn inp(&mut self, port: u16, io_len: u8, current_ticks: u64) -> u32 {
        self.diag_io_reads += 1;
        let entry = &self.read_handlers[port as usize];
        let device_id = entry.device_id;
        let len_mask = 1u8 << (io_len.trailing_zeros() as u8);
        let has_handler = device_id != DeviceId::None && (entry.mask & len_mask) != 0;

        let mut pic_intr_level = None;
        let mut hrq_level = None;
        let mut timer_update = None;
        let value = if has_handler {
            if let Some(dm) = self.device_manager_mut() {
                let result = Self::dispatch_read(dm, device_id, port, io_len, current_ticks);
                timer_update =
                    Self::timer_update_after_dispatch(dm, device_id, port, current_ticks);
                if device_id == DeviceId::Acpi {
                    if dm.acpi.irq9_level {
                        dm.pic.raise_irq(9);
                    } else {
                        dm.pic.lower_irq(9);
                    }
                }
                let (fwds, count) = dm.pic.take_ioapic_forwards();
                hrq_level = dm.dma.take_hrq_request();
                let devices::DeviceManager {
                    ref mut pic,
                    ref mut ioapic,
                    ..
                } = *dm;
                for &(irq, level) in &fwds[..count] {
                    ioapic.set_irq_level(irq, level, Some(&mut *pic), None);
                }
                pic_intr_level = Self::take_pic_level_after_dispatch(dm);
                result
            } else {
                self.default_read_handler(port, io_len)
            }
        } else {
            self.default_read_handler(port, io_len)
        };

        if let Some((owner, delay)) = timer_update {
            self.request_timer_after_usec(owner, current_ticks, delay);
        }
        if let Some(level) = pic_intr_level {
            self.pic_intr_level = Some(level);
        }
        if let Some(level) = hrq_level {
            self.hrq_level = Some(level);
        }
        self.last_io_read_port = port;
        self.last_io_read_value = value;
        value
    }

    /// Write to an I/O port.
    #[inline]
    pub fn outp(&mut self, port: u16, value: u32, io_len: u8, current_ticks: u64) {
        self.diag_io_writes += 1;
        let entry = &self.write_handlers[port as usize];
        let device_id = entry.device_id;
        let len_mask = 1u8 << (io_len.trailing_zeros() as u8);
        let has_handler = device_id != DeviceId::None && (entry.mask & len_mask) != 0;

        if has_handler {
            let mut pic_intr_level = None;
            let mut hrq_level = None;
            let mut ide_timer_delays = [None; 2];
            let mut timer_update = None;
            let mut cmos_timer_sync = None;
            let mut deferred_device_work = false;
            let mut machine_boundary_pending = false;
            let dispatched = if let Some(dm) = self.device_manager_mut() {
                cmos_timer_sync =
                    Self::dispatch_write(dm, device_id, port, value, io_len, current_ticks);
                timer_update =
                    Self::timer_update_after_dispatch(dm, device_id, port, current_ticks);
                deferred_device_work = device_id == DeviceId::HardDrive
                    && dm.harddrv.seek_complete_pending.iter().any(|pending| *pending);
                if device_id == DeviceId::Acpi {
                    if dm.acpi.irq9_level {
                        dm.pic.raise_irq(9);
                    } else {
                        dm.pic.lower_irq(9);
                    }
                }
                let (fwds, count) = dm.pic.take_ioapic_forwards();
                hrq_level = dm.dma.take_hrq_request();
                let devices::DeviceManager { pic, ioapic, .. } = dm;
                for &(irq, level) in &fwds[..count] {
                    ioapic.set_irq_level(irq, level, Some(&mut *pic), None);
                }
                for (channel, delay_ticks) in ide_timer_delays.iter_mut().enumerate() {
                    *delay_ticks = dm.pci_ide.take_pending_timer_arm(channel);
                }
                pic_intr_level = Self::take_pic_level_after_dispatch(dm);
                machine_boundary_pending = dm.has_pending_machine_boundary();
                true
            } else {
                false
            };

            if deferred_device_work || machine_boundary_pending {
                self.scheduler_boundary_requested = true;
            }
            if let Some(sync) = cmos_timer_sync {
                self.apply_cmos_timer_sync(current_ticks, sync);
            }
            if let Some((owner, delay)) = timer_update {
                self.request_timer_after_usec(owner, current_ticks, delay);
            }
            if let Some(level) = pic_intr_level {
                self.pic_intr_level = Some(level);
            }
            if let Some(level) = hrq_level {
                self.hrq_level = Some(level);
            }
            for (channel, delay_ticks) in ide_timer_delays.into_iter().enumerate() {
                if let Some(delay_ticks) = delay_ticks {
                    let owner = match channel {
                        0 => DeviceTimerOwner::PciIdeCh0,
                        1 => DeviceTimerOwner::PciIdeCh1,
                        _ => unreachable!(),
                    };
                    self.request_timer(
                        owner,
                        TimerRequest::Activate {
                            deadline_ticks: current_ticks.saturating_add(u64::from(delay_ticks)),
                            period_ticks: u64::from(delay_ticks),
                            continuous: false,
                        },
                    );
                }
            }
            if dispatched {
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
    pub fn inp_bulk(
        &mut self,
        port: u16,
        io_len: u8,
        buf: &mut [u8],
        current_ticks: u64,
    ) -> usize {
        // Only optimize IDE data ports (base + 0 = data register).
        if (port != 0x1F0 && port != 0x170) || (io_len != 2 && io_len != 4) {
            return 0;
        }
        let entry = &self.read_handlers[port as usize];
        if entry.device_id != DeviceId::HardDrive {
            return 0;
        }

        // The current bulk IDE path has no clocked register transition beyond
        // the operation itself. Keep the captured issuing epoch in its API so
        // future device-owned timer producers preserve the same boundary.
        let _ = current_ticks;
        let mut pic_intr_level = None;
        let bytes_read = if let Some(dm) = self.device_manager_mut() {
            let result = {
                let devices::DeviceManager {
                    ref mut harddrv,
                    ref mut pic,
                    ref mut pci_ide,
                    ..
                } = *dm;
                harddrv.bulk_read_data(port, io_len, buf, pic, pci_ide)
            };
            {
                let (fwds, count) = dm.pic.take_ioapic_forwards();
                let devices::DeviceManager {
                    ref mut pic,
                    ref mut ioapic,
                    ..
                } = *dm;
                for &(irq, level) in &fwds[..count] {
                    ioapic.set_irq_level(irq, level, Some(&mut *pic), None);
                }
            }
            pic_intr_level = Self::take_pic_level_after_dispatch(dm);
            result
        } else {
            0
        };
        if let Some(level) = pic_intr_level {
            self.pic_intr_level = Some(level);
        }
        bytes_read
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
            tracing::trace!("BIOS POST code port {:#06x}: {:#04x}", address, value as u8);
            self.port80_output.push_back(value as u8);
            return;
        }

        // Bochs-style debug output ports: capture bytes into a host-drainable buffer.
        //
        // - 0xE9: Bochs debug console (optional in upstream; always-on here)
        // - 0x402/0x403: Bochs rombios INFO/DEBUG ports (cpp_orig/bochs/bios/rombios.h)
        // - 0x500: VGABIOS info port (cpp_orig/bochs/bios/VGABIOS-lgpl-README)
        if io_len == 1 && matches!(address, 0x00E9 | 0x0402 | 0x0403 | 0x0500) {
            tracing::trace!(
                "BIOS output port {:#06x}: {:?}",
                address,
                value as u8 as char
            );
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
    #[cfg(feature = "alloc")]
    pub fn take_port_e9_output(&mut self) -> Vec<u8> {
        self.port_e9_output.drain().collect()
    }

    /// Drain and return BIOS POST codes written to port 0x80/0x84.
    #[cfg(feature = "alloc")]
    pub fn take_port80_output(&mut self) -> Vec<u8> {
        self.port80_output.drain().collect()
    }

    /// Drain port 0xE9 output as an iterator (no-alloc).
    pub fn drain_port_e9_output(&mut self) -> impl Iterator<Item = u8> + '_ {
        self.port_e9_output.drain()
    }

    /// Drain BIOS POST codes (port 0x80/0x84) as an iterator (no-alloc).
    pub fn drain_port80_output(&mut self) -> impl Iterator<Item = u8> + '_ {
        self.port80_output.drain()
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

    /// Access the device manager only while the emulator has installed its
    /// single-threaded dispatch pointer for the current CPU slice.
    #[inline(always)]
    fn device_manager_mut(&mut self) -> Option<&mut devices::DeviceManager> {
        self.device_manager.map(|mut pointer| unsafe { pointer.as_mut() })
    }


    #[inline]
    fn timer_update_after_dispatch(
        dm: &mut devices::DeviceManager,
        id: DeviceId,
        port: u16,
        current_ticks: u64,
    ) -> Option<(DeviceTimerOwner, Option<u64>)> {
        match id {
            DeviceId::Pit => Some((DeviceTimerOwner::Pit, dm.pit.next_event_usec())),
            DeviceId::Keyboard => dm
                .keyboard
                .take_keyboard_timer_update()
                .map(|delay| (DeviceTimerOwner::Keyboard, delay)),
            DeviceId::Acpi => Some((
                DeviceTimerOwner::AcpiPmOverflow,
                dm.acpi.overflow_delay_usec(current_ticks),
            )),
            DeviceId::Serial => {
                let index = dm.serial.port_index_for_address(port)?;
                dm.serial
                    .take_fifo_timer_update(index)
                    .map(|delay| (DeviceTimerOwner::SerialFifo(index), delay))
            }
            _ => None,
        }
    }

    #[inline]
    fn forward_serial_irqs(dm: &mut devices::DeviceManager) {
        for (irq, raise) in dm.serial.take_pending_irqs() {
            if raise {
                dm.pic.raise_irq(irq);
            } else {
                dm.pic.lower_irq(irq);
            }
        }
    }

    /// Dispatch a port read to the device identified by `id`.
    #[inline]
    fn dispatch_read(
        dm: &mut devices::DeviceManager,
        id: DeviceId,
        port: u16,
        io_len: u8,
        current_ticks: u64,
    ) -> u32 {
        match id {
            DeviceId::Pic => dm.pic.read(port, io_len),
            DeviceId::Pit => {
                let result = dm.pit.read(port, io_len, current_ticks);
                // The pre-read sync can clock counter 0's OUT pin — replay
                // the transitions into the PIC (Bochs pit.cc irq_handler
                // fires synchronously from handle_timer inside read).
                dm.drain_pit_irq0();
                result
            }
            DeviceId::Cmos => {
                let result = dm.cmos.read(port, io_len);
                if dm.cmos.check_irq8_lower() {
                    dm.pic.lower_irq(8);
                }
                result
            }
            DeviceId::Dma => dm.dma.read(port, io_len),
            DeviceId::Keyboard => {
                if port == keyboard::KBD_DATA_PORT {
                    let result = dm.keyboard.read_data_port_for_device_manager();
                    if let Some(irq) = result.irq_to_lower {
                        dm.pic.lower_irq(irq);
                    }
                    result.value
                } else {
                    dm.keyboard.read(port, io_len)
                }
            }
            DeviceId::HardDrive => {
                let devices::DeviceManager {
                    harddrv,
                    pic,
                    pci_ide,
                    ..
                } = dm;
                harddrv.read(port, io_len, pic, pci_ide)
            }
            DeviceId::Serial => {
                let result = dm.serial.read(port, io_len);
                Self::forward_serial_irqs(dm);
                result
            }
            DeviceId::Vga => dm.vga.read_port(port, io_len, current_ticks),
            DeviceId::Port92 => dm.port92_read(port, io_len),
            DeviceId::Pci => dm.pci_read(port, io_len),
            DeviceId::Acpi => dm.acpi_read(port, io_len, current_ticks),
            DeviceId::PciIde => dm.pci_ide_read(port, io_len),
            DeviceId::FwCfg => dm.fw_cfg.read_port_mut(port, io_len),
            DeviceId::Ioapic => 0xFF, // IOAPIC uses MMIO, not port I/O
            DeviceId::None => 0xFFFF_FFFF,
        }
    }

    /// Dispatch a port write to the device identified by `id`.
    #[inline]
    fn dispatch_write(
        dm: &mut devices::DeviceManager,
        id: DeviceId,
        port: u16,
        value: u32,
        io_len: u8,
        current_ticks: u64,
    ) -> Option<cmos::CmosTimerSync> {
        if id == DeviceId::Cmos {
            return Some(dm.cmos.write(port, value, io_len));
        }
        match id {
            DeviceId::Pic => dm.pic.write(port, value, io_len),
            DeviceId::Pit => {
                dm.pit.write(port, value, io_len, current_ticks);
                dm.drain_pit_irq0();
            }
            DeviceId::Dma => dm.dma.write(port, value, io_len),
            DeviceId::Keyboard => dm.keyboard.write(port, value, io_len),
            DeviceId::HardDrive => {
                let devices::DeviceManager {
                    harddrv,
                    pic,
                    pci_ide,
                    ..
                } = dm;
                harddrv.write(port, value, io_len, pic, pci_ide)
            }
            DeviceId::Serial => {
                dm.serial.write(port, value, io_len);
                Self::forward_serial_irqs(dm);
            }
            DeviceId::Vga => dm.vga.write_port(port, value, io_len),
            DeviceId::Port92 => dm.port92_write(port, value, io_len),
            DeviceId::Pci => dm.pci_write(port, value, io_len),
            DeviceId::Acpi => dm.acpi_write(port, value, io_len, current_ticks),
            DeviceId::PciIde => dm.pci_ide_write(port, value, io_len),
            DeviceId::FwCfg => dm.fw_cfg_write(port, value, io_len),
            DeviceId::Cmos | DeviceId::Ioapic | DeviceId::None => {}
        }
        None
    }
}

/// Maximum logical bytes retained for the Bochs debug-console stream.
#[cfg(feature = "std")]
const PORT_E9_SNAPSHOT_CAPACITY: usize = 65_536;
/// Maximum logical bytes retained for the Bochs POST-code stream.
#[cfg(feature = "std")]
const PORT80_SNAPSHOT_CAPACITY: usize = 4_096;

/// PLATFORM-local continuation state decoded from [`BxDevicesC`].
///
/// The enclosing PLATFORM decoder cross-checks `pci_conf_addr` against the
/// DeviceManager latch before allowing execution to resume.
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BxDevicesSnapshotRestore {
    pub(crate) pci_enabled: bool,
    pub(crate) pci_conf_addr: u32,
    pub(crate) pic_intr_level: Option<bool>,
    pub(crate) scheduler_boundary_requested: bool,
}

#[cfg(feature = "std")]
fn invalid_bx_devices_snapshot(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

#[cfg(feature = "std")]
fn timer_request_snapshot_len(request: TimerRequest) -> io::Result<u64> {
    match request {
        TimerRequest::Unchanged | TimerRequest::Deactivate => Ok(1),
        TimerRequest::Activate { .. } => checked_snapshot_len_add(1, 17),
    }
}

#[cfg(feature = "std")]
fn write_timer_request_snapshot<W: Write>(
    writer: &mut W,
    request: TimerRequest,
) -> io::Result<()> {
    match request {
        TimerRequest::Unchanged => writer.write_u8(0),
        TimerRequest::Deactivate => writer.write_u8(1),
        TimerRequest::Activate {
            deadline_ticks,
            period_ticks,
            continuous,
        } => {
            writer.write_u8(2)?;
            writer.write_u64(deadline_ticks)?;
            writer.write_u64(period_ticks)?;
            writer.write_bool(continuous)
        }
    }
}

#[cfg(feature = "std")]
fn read_timer_request_snapshot<R: Read>(
    reader: &mut SnapshotReader<R>,
) -> io::Result<TimerRequest> {
    match reader.read_u8()? {
        0 => Ok(TimerRequest::Unchanged),
        1 => Ok(TimerRequest::Deactivate),
        2 => Ok(TimerRequest::Activate {
            deadline_ticks: reader.read_u64()?,
            period_ticks: reader.read_u64()?,
            continuous: reader.read_bool()?,
        }),
        _ => Err(invalid_bx_devices_snapshot(
            "snapshot device timer request tag is invalid",
        )),
    }
}

#[cfg(feature = "std")]
impl BxDevicesC {
    /// Number of bytes emitted by the PLATFORM controller body. Handler
    /// topology, raw pointers, immutable timer configuration, and diagnostics
    /// intentionally stay live and are never part of this representation.
    pub(crate) fn snapshot_v3_body_len(&self) -> io::Result<u64> {
        self.validate_snapshot_v3_state()?;

        let mut len = 1u64; // PCI enabled
        len = checked_snapshot_len_add(len, 4)?; // PCI config latch
        len = checked_snapshot_len_add(
            len,
            if self.pic_intr_level.is_some() { 2 } else { 1 },
        )?;
        len = checked_snapshot_len_add(
            len,
            if self.hrq_level.is_some() { 2 } else { 1 },
        )?;
        len = checked_snapshot_len_add(len, 1)?; // scheduler boundary latch
        for request in self.timer_requests.slots {
            len = checked_snapshot_len_add(len, timer_request_snapshot_len(request)?)?;
        }
        len = checked_snapshot_len_add(len, 8)?; // both queue counts
        len = checked_snapshot_len_add(
            len,
            u64::try_from(self.port_e9_output.len()).map_err(|_| {
                invalid_bx_devices_snapshot("snapshot debug-console queue length does not fit")
            })?,
        )?;
        checked_snapshot_len_add(
            len,
            u64::try_from(self.port80_output.len()).map_err(|_| {
                invalid_bx_devices_snapshot("snapshot POST-code queue length does not fit")
            })?,
        )
    }

    /// Stream guest-visible controller continuation state without draining
    /// queues or serializing live handler/pointer topology.
    pub(crate) fn save_snapshot_v3_body<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.validate_snapshot_v3_state()?;

        writer.write_bool(self.pci_enabled)?;
        writer.write_u32(self.pci_conf_addr)?;
        writer.write_bool(self.pic_intr_level.is_some())?;
        if let Some(level) = self.pic_intr_level {
            writer.write_bool(level)?;
        }
        writer.write_bool(self.hrq_level.is_some())?;
        if let Some(level) = self.hrq_level {
            writer.write_bool(level)?;
        }
        writer.write_bool(self.scheduler_boundary_requested)?;
        for request in self.timer_requests.slots {
            write_timer_request_snapshot(writer, request)?;
        }
        writer.write_u32(u32::try_from(self.port_e9_output.len()).map_err(|_| {
            invalid_bx_devices_snapshot("snapshot debug-console queue length does not fit u32")
        })?)?;
        writer.write_u32(u32::try_from(self.port80_output.len()).map_err(|_| {
            invalid_bx_devices_snapshot("snapshot POST-code queue length does not fit u32")
        })?)?;
        for byte in self.port_e9_output.iter() {
            writer.write_u8(byte)?;
        }
        for byte in self.port80_output.iter() {
            writer.write_u8(byte)?;
        }
        Ok(())
    }

    /// Decode controller state without touching handler registrations, raw
    /// pointers, timer frequency configuration, or diagnostics. Pending timer
    /// operations remain queued for the parent-owned scheduler boundary.
    pub(crate) fn restore_snapshot_v3_body<R: Read>(
        &mut self,
        reader: &mut SnapshotReader<R>,
    ) -> io::Result<BxDevicesSnapshotRestore> {
        let live_pci_enabled = self.pci_enabled;
        let pci_enabled = reader.read_bool()?;
        if pci_enabled != live_pci_enabled {
            return Err(invalid_bx_devices_snapshot(
                "snapshot PCI enablement does not match live configuration",
            ));
        }
        let pci_conf_addr = reader.read_u32()?;
        let pic_intr_level = if reader.read_bool()? {
            Some(reader.read_bool()?)
        } else {
            None
        };
        let hrq_level = if reader.read_bool()? {
            Some(reader.read_bool()?)
        } else {
            None
        };
        let scheduler_boundary_requested = reader.read_bool()?;
        let mut timer_requests =
            [TimerRequest::Unchanged; BX_FIXED_TIMER_OWNER_COUNT];
        let mut has_timer_request = false;
        for request in &mut timer_requests {
            *request = read_timer_request_snapshot(reader)?;
            has_timer_request |= *request != TimerRequest::Unchanged;
        }
        if has_timer_request && !scheduler_boundary_requested {
            return Err(invalid_bx_devices_snapshot(
                "snapshot timer request lacks scheduler-boundary latch",
            ));
        }

        let port_e9_len = reader.read_count(PORT_E9_SNAPSHOT_CAPACITY)?;
        let port80_len = reader.read_count(PORT80_SNAPSHOT_CAPACITY)?;

        // The queue counts have been bounded before either live queue changes.
        // Fixed storage avoids untrusted allocation; a later truncated stream
        // is an unrecoverable parent restore error by contract.
        self.port_e9_output.clear();
        for _ in 0..port_e9_len {
            self.port_e9_output.push_back(reader.read_u8()?);
        }
        self.port80_output.clear();
        for _ in 0..port80_len {
            self.port80_output.push_back(reader.read_u8()?);
        }

        self.pci_enabled = pci_enabled;
        self.pci_conf_addr = pci_conf_addr;
        self.pic_intr_level = pic_intr_level;
        self.hrq_level = hrq_level;
        self.scheduler_boundary_requested = scheduler_boundary_requested;
        self.timer_requests = TimerRequestTable {
            slots: timer_requests,
        };

        Ok(BxDevicesSnapshotRestore {
            pci_enabled,
            pci_conf_addr,
            pic_intr_level,
            scheduler_boundary_requested,
        })
    }

    fn validate_snapshot_v3_state(&self) -> io::Result<()> {
        if self.port_e9_output.len() > PORT_E9_SNAPSHOT_CAPACITY
            || self.port80_output.len() > PORT80_SNAPSHOT_CAPACITY
        {
            return Err(invalid_bx_devices_snapshot(
                "snapshot output queue exceeds fixed capacity",
            ));
        }
        let has_timer_request = self
            .timer_requests
            .slots
            .iter()
            .any(|request| *request != TimerRequest::Unchanged);
        if has_timer_request && !self.scheduler_boundary_requested {
            return Err(invalid_bx_devices_snapshot(
                "device timer request lacks scheduler-boundary latch",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BxDevicesC is ~1.5MB due to [IoHandlerEntry; 65536] x2.
    /// Allocate on heap to avoid test stack overflow.
    fn boxed_devices() -> alloc::boxed::Box<BxDevicesC> {
        // Use alloc_zeroed + field writes to avoid stack intermediary.
        let layout = alloc::alloc::Layout::new::<BxDevicesC>();
        unsafe {
            let ptr = alloc::alloc::alloc_zeroed(layout) as *mut BxDevicesC;
            assert!(!ptr.is_null());
            // IoHandlerEntry is all-zero-valid: device_id=None(0), name="" is ptr+len
            // but &'static str zero bits aren't valid. Write defaults properly:
            for i in 0..IO_PORTS {
                core::ptr::addr_of_mut!((*ptr).read_handlers[i]).write(IoHandlerEntry::default());
                core::ptr::addr_of_mut!((*ptr).write_handlers[i]).write(IoHandlerEntry::default());
            }
            core::ptr::addr_of_mut!((*ptr).port_e9_output).write(RingBuffer::new());
            core::ptr::addr_of_mut!((*ptr).port80_output).write(RingBuffer::new());
            alloc::boxed::Box::from_raw(ptr)
        }
    }

    fn on_big_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn test_default_handlers() {
        let mut devices = boxed_devices();

        // Reading unhandled port should return 0xFF/0xFFFF/0xFFFFFFFF
        assert_eq!(devices.inp(0x1234, 1, 0), 0xFF);
        assert_eq!(devices.inp(0x1234, 2, 0), 0xFFFF);
        assert_eq!(devices.inp(0x1234, 4, 0), 0xFFFFFFFF);
    }

    #[test]
    fn test_multiple_instances() {
        let mut dev1 = boxed_devices();
        let mut dev2 = boxed_devices();

        // Register handler only on dev1
        dev1.register_io_read_handler(DeviceId::Pic, 0x100, "test", 0x1);

        // dev1 has a device registered, dev2 does not.
        // Without a device_manager, both return default.
        assert_eq!(dev1.inp(0x100, 1, 0), 0xFF);
        assert_eq!(dev2.inp(0x100, 1, 0), 0xFF);
    }

    #[test]
    fn timer_request_table_overwrites_only_its_owner_and_latches_boundary() {
        let mut devices = boxed_devices();

        devices.request_timer(
            DeviceTimerOwner::PciIdeCh0,
            TimerRequest::Activate {
                deadline_ticks: 17,
                period_ticks: 17,
                continuous: false,
            },
        );
        devices.request_timer(DeviceTimerOwner::PciIdeCh0, TimerRequest::Deactivate);
        devices.request_timer(
            DeviceTimerOwner::PciIdeCh1,
            TimerRequest::Activate {
                deadline_ticks: 23,
                period_ticks: 23,
                continuous: true,
            },
        );

        assert!(devices.take_scheduler_boundary_requested());
        assert!(!devices.take_scheduler_boundary_requested());
        let requests = devices.take_timer_requests();
        assert_eq!(
            requests.get(DeviceTimerOwner::PciIdeCh0),
            TimerRequest::Deactivate
        );
        assert_eq!(
            requests.get(DeviceTimerOwner::PciIdeCh1),
            TimerRequest::Activate {
                deadline_ticks: 23,
                period_ticks: 23,
                continuous: true,
            }
        );
        assert_eq!(
            devices
                .take_timer_requests()
                .get(DeviceTimerOwner::PciIdeCh0),
            TimerRequest::Unchanged
        );
    }

    #[test]
    fn reset_discards_pre_reset_scheduler_transport() {
        let mut devices = boxed_devices();
        devices.pic_intr_level = Some(true);
        devices.request_timer(
            DeviceTimerOwner::PciIdeCh0,
            TimerRequest::Activate {
                deadline_ticks: 17,
                period_ticks: 17,
                continuous: false,
            },
        );

        devices.discard_scheduler_boundary_work();

        assert_eq!(devices.take_pic_intr_level(), None);
        assert!(!devices.take_scheduler_boundary_requested());
        assert_eq!(
            devices
                .take_timer_requests()
                .get(DeviceTimerOwner::PciIdeCh0),
            TimerRequest::Unchanged
        );
    }

    #[test]
    fn pic_clear_then_reassert_collapses_to_asserted_level() {
        on_big_stack(|| {
            let mut io = boxed_devices();
            let mut dm = devices::DeviceManager::new();
            // A clear notification followed by a later assertion can coexist
            // before the raw I/O borrow is released. The transport must
            // publish the final physical pin, not replay those edges in order.
            dm.pic.irq_cleared = true;
            dm.pic.irq_pending = true;
            dm.pic.master.int_pin = true;

            io.register_io_read_handler(DeviceId::Pic, 0x20, "PIC", 0x1);
            io.set_device_manager(core::ptr::NonNull::from(&mut dm));
            let _ = io.inp(0x20, 1, 91);
            io.clear_device_manager();

            assert_eq!(io.take_pic_intr_level(), Some(true));
            assert_eq!(io.take_pic_intr_level(), None);
        });
    }
    #[test]
    fn keyboard_port60_read_lowers_irq_before_replacement_timer() {
        on_big_stack(|| {
            let mut io = boxed_devices();
            let mut dm = devices::DeviceManager::new();
            dm.keyboard.send_scancode(0x1E);
            let callback = dm.keyboard.timer_callback(1);
            assert_eq!(callback.irq_mask & 0x01, 0x01);
            let delivered = u32::from(dm.keyboard.kbd_controller.kbd_output_buffer);
            dm.pic.raise_irq(1);
            assert_ne!(dm.pic.master.irq_in[1], 0);

            io.set_timer_ips(1_000_000);
            io.register_io_read_handler(DeviceId::Keyboard, keyboard::KBD_DATA_PORT, "Keyboard", 0x1);
            io.set_device_manager(core::ptr::NonNull::from(&mut dm));
            assert_eq!(io.inp(keyboard::KBD_DATA_PORT, 1, 77), delivered);
            io.clear_device_manager();

            assert_eq!(dm.pic.master.irq_in[1], 0);
            assert_eq!(
                io.take_timer_requests().get(DeviceTimerOwner::Keyboard),
                TimerRequest::Activate {
                    deadline_ticks: 78,
                    period_ticks: 1,
                    continuous: false,
                }
            );
        });
    }

    #[test]
    fn keyboard_status_polling_preserves_existing_one_shot_deadline() {
        on_big_stack(|| {
            let mut io = boxed_devices();
            let mut dm = devices::DeviceManager::new();

            io.set_timer_ips(120_000_000);
            io.register_io_write_handler(
                DeviceId::Keyboard,
                keyboard::KBD_DATA_PORT,
                "Keyboard",
                0x1,
            );
            io.register_io_read_handler(
                DeviceId::Keyboard,
                keyboard::KBD_STATUS_PORT,
                "Keyboard",
                0x1,
            );
            io.set_device_manager(core::ptr::NonNull::from(&mut dm));

            io.outp(keyboard::KBD_DATA_PORT, 0xff, 1, 100);
            assert_eq!(
                io.take_timer_requests().get(DeviceTimerOwner::Keyboard),
                TimerRequest::Activate {
                    deadline_ticks: 220,
                    period_ticks: 120,
                    continuous: false,
                }
            );
            assert!(io.take_scheduler_boundary_requested());

            for now in 101..220 {
                let _ = io.inp(keyboard::KBD_STATUS_PORT, 1, now);
            }
            io.clear_device_manager();

            assert_eq!(
                io.take_timer_requests().get(DeviceTimerOwner::Keyboard),
                TimerRequest::Unchanged
            );
            assert!(!io.take_scheduler_boundary_requested());
        });
    }

}
