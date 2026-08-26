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
use crate::snapshot::{checked_snapshot_len_add, SnapError, SnapRead, SnapResult, SnapWrite};


/// Bounded retention for the port-0xE9 debug console when no host consumer
/// has drained it yet.
const DEBUGCON_CAPACITY: usize = 65536;

/// Bounded retention for BIOS POST codes (ports 0x80/0x84).
const PORT80_CAPACITY: usize = 4096;

/// Draining iterator over the port-0xE9 debug console.
///
/// Named rather than `impl Iterator` (doctrine R0) so the machine's debug-port
/// role handle can forward it, and so the capacity constant stays out of the
/// public signature. Yields bytes in write order and empties the buffer.
pub struct DebugconDrain<'a>(crate::ring_buffer::Drain<'a, u8, DEBUGCON_CAPACITY>);

impl Iterator for DebugconDrain<'_> {
    type Item = u8;

    #[inline]
    fn next(&mut self) -> Option<u8> {
        self.0.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for DebugconDrain<'_> {}

/// Draining iterator over BIOS POST codes (ports 0x80/0x84). Same R0 rationale
/// as [`DebugconDrain`].
pub struct Port80Drain<'a>(crate::ring_buffer::Drain<'a, u8, PORT80_CAPACITY>);

impl Iterator for Port80Drain<'_> {
    type Item = u8;

    #[inline]
    fn next(&mut self) -> Option<u8> {
        self.0.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for Port80Drain<'_> {}

pub mod acpi;
#[cfg(feature = "alloc")]
pub mod acpi_tables;
pub mod cmos;
pub mod devices;
pub use crate::dma;
pub mod fw_cfg;
pub mod harddrv;
pub mod hpet;
pub mod ioapic;
pub mod keyboard;
pub mod scancodes;
pub mod pci;
pub mod pci2isa;
pub mod pci_ide;
pub use crate::pic;
pub mod pit;
pub mod ide;
pub mod serial;
pub(crate) mod wiring;

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
// VgaCore is pub(crate) - not exported outside the crate
#[cfg(feature = "alloc")]
/// The NV card, named beside its siblings in this namespace even though
/// nothing in this crate wires it yet — being ported ahead of its wiring is
/// what it is for, and dropping it from the list because of that is how a
/// ported-ahead model quietly stops existing.
#[cfg(feature = "alloc")]
pub use rusty_box_devices::display::geforce::BxGeForceC;

/// The port tables span the whole port space twice, so this struct's size is
/// multiplied by 131072. Pinned here so a future field addition is a
/// deliberate 128 KiB-per-table decision rather than an accident.
const _: () = assert!(core::mem::size_of::<IoHandlerEntry>() == 2);
/// Number of I/O ports (0x0000 - 0xFFFF)
pub const IO_PORTS: usize = 0x10000;
/// Maximum number of UARTs the machine can carry (Bochs serial.h
/// `BX_N_SERIAL_PORTS`). Bounds the scheduler's `TimerOwner::Serial*` port
/// index when validating a restored snapshot, which is the only thing that
/// reads it — hence the `std` gate that matches the snapshot format's own.
#[cfg(feature = "std")]
pub(crate) const BX_FIXED_SERIAL_TIMER_OWNERS: usize = 4;

/// Number of fixed device timer owners carried across the raw I/O boundary.
///
/// LAPIC requests use their CPU-local transport. Every device request below
/// has exactly one stable slot, so a producer can overwrite its own pending
/// work without allocating or scanning a timer list. The final four slots are
/// the ATA/ATAPI seek timers (Bochs harddrv.cc "HD/CD seek", one per drive).
///
/// This table is transitional: a device converted to the device API arms its
/// timers directly and gives up its slots here. The UART already has.
pub(crate) const BX_FIXED_TIMER_OWNER_COUNT: usize = 6 + 2 + 4;

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
    PciIdeCh0,
    PciIdeCh1,
    /// ATA/ATAPI seek timer — Bochs harddrv.cc "HD/CD seek". The argument is
    /// Bochs's `setTimerParam` value `(channel << 1) | device`.
    HdSeek(usize),
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
            Self::PciIdeCh0 => Some(6),
            Self::PciIdeCh1 => Some(7),
            Self::HdSeek(param) if param < 4 => Some(8 + param),
            Self::HdSeek(_) => None,
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

/// Identifies the device that owns an I/O port registration.
///
/// An opaque index, not a variant per device. Bochs registers a port to a
/// `(handler, this_ptr)` pair, so the bus there carries no knowledge of the
/// device set; this is the safe equivalent — the port tables carry a number the
/// bus assigns, and only [`DeviceManager`](devices::DeviceManager) knows which
/// device a number names. Adding a device is a new constant plus a routing arm,
/// not an edit to a type every port-table consumer must match exhaustively.
///
/// Slots are compile-time constants rather than runtime-allocated because the
/// PC machine's device set is fixed at build time. They are never serialized:
/// the port tables are rebuilt by device registration on restore, so the
/// numbering below is free to change.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct DevSlot(u8);

impl DevSlot {
    /// No device registered — an unclaimed port.
    pub const NONE: Self = Self(0);
    /// 8259 PIC (Programmable Interrupt Controller).
    pub const PIC: Self = Self(1);
    /// 8254 PIT (Programmable Interval Timer).
    pub const PIT: Self = Self(2);
    /// CMOS/RTC.
    pub const CMOS: Self = Self(3);
    /// 8237 DMA controller.
    pub const DMA: Self = Self(4);
    /// 8042 keyboard/mouse controller.
    pub const KEYBOARD: Self = Self(5);
    /// ATA/ATAPI task-file registers.
    pub const IDE: Self = Self(6);
    /// 16550 UART serial port.
    pub const SERIAL: Self = Self(7);
    /// VGA display controller.
    pub const VGA: Self = Self(8);
    /// Port 92h system control (A20/reset).
    pub const PORT92: Self = Self(9);
    /// PCI bus — config address/data, PIIX3 ELCR and APM ports.
    pub const PCI: Self = Self(10);
    /// PIIX4 ACPI power management.
    pub const ACPI: Self = Self(11);
    /// QEMU fw_cfg firmware configuration device.
    pub const FW_CFG: Self = Self(12);
    /// 82093AA I/O APIC. Memory-mapped only — it occupies no I/O port.
    pub const IOAPIC: Self = Self(13);
    /// High Precision Event Timer. Memory-mapped only.
    pub const HPET: Self = Self(14);

    /// True for the unclaimed-port slot.
    #[inline]
    pub const fn is_none(self) -> bool {
        self.0 == Self::NONE.0
    }

    /// This slot as the token a memory-map registration carries.
    ///
    /// Device identity is one namespace: a device that answers both port I/O
    /// and a physical range — the VGA — is the same slot on both buses. The
    /// memory subsystem stores the token without interpreting it and hands it
    /// back on an access, which is the whole of what it knows about devices.
    #[inline]
    pub(crate) const fn mmio_token(self) -> crate::memory::mmio_map::MmioToken {
        self.mmio_window_token(rusty_box_devices::api::WindowId::FIRST)
    }

    /// The token for one of this device's windows.
    ///
    /// A device with several disjoint ranges — the VGA's legacy aperture,
    /// framebuffer and register block — mints one token per range, so the
    /// access that comes back names the window as well as the device. The two
    /// halves live in one `u16` because the memory subsystem stores the token
    /// verbatim and interprets neither: which byte means what is the platform's
    /// business, and this pair of functions is where it is decided.
    #[inline]
    pub(crate) const fn mmio_window_token(
        self,
        window: rusty_box_devices::api::WindowId,
    ) -> crate::memory::mmio_map::MmioToken {
        crate::memory::mmio_map::MmioToken((self.0 as u16) | ((window.0 as u16) << 8))
    }

    /// Recover the slot a memory access reported. Inverse of [`Self::mmio_token`].
    #[inline]
    pub(crate) const fn from_mmio_token(token: crate::memory::mmio_map::MmioToken) -> Self {
        Self(token.0 as u8)
    }

    /// Recover the window a memory access reported. Inverse of
    /// [`Self::mmio_window_token`].
    #[inline]
    pub(crate) const fn window_from_mmio_token(
        token: crate::memory::mmio_map::MmioToken,
    ) -> rusty_box_devices::api::WindowId {
        rusty_box_devices::api::WindowId((token.0 >> 8) as u8)
    }

    /// Human-readable name, for diagnostics and registration logging.
    pub const fn name(self) -> &'static str {
        match self {
            Self::NONE => "unclaimed",
            Self::PIC => "8259 PIC",
            Self::PIT => "8254 PIT",
            Self::CMOS => "CMOS/RTC",
            Self::DMA => "8237 DMA",
            Self::KEYBOARD => "8042 keyboard controller",
            Self::IDE => "ATA/ATAPI",
            Self::SERIAL => "16550 UART",
            Self::VGA => "VGA",
            Self::PORT92 => "port 92h",
            Self::PCI => "PCI bus",
            Self::ACPI => "PIIX4 ACPI",
            Self::FW_CFG => "fw_cfg",
            Self::IOAPIC => "82093AA I/O APIC",
            Self::HPET => "HPET",
            _ => "unknown device slot",
        }
    }
}

impl core::fmt::Debug for DevSlot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "DevSlot({}, {})", self.0, self.name())
    }
}

/// I/O handler registration entry for a single port.
///
/// Each port maps to a [`DevSlot`] for safe dispatch through `DeviceManager`.
#[derive(Clone, Copy)]
pub struct IoHandlerEntry {
    /// Which device owns this port
    pub(crate) slot: DevSlot,
    /// I/O length mask (bit 0 = 1 byte, bit 1 = 2 bytes, bit 2 = 4 bytes)
    pub(crate) mask: u8,
}

impl Default for IoHandlerEntry {
    fn default() -> Self {
        Self {
            slot: DevSlot::NONE,
            // NOTE: keep this struct at two bytes — it is instantiated 131072
            // times (a read and a write table over the whole port space), so
            // every added byte costs 128 KiB per table.
            mask: 0x7, // All lengths supported by default
        }
    }
}

/// Bochs biosdev.h BX_BIOS_MESSAGE_SIZE — rombios/vgabios message-port line
/// buffer capacity.
const BX_BIOS_MESSAGE_SIZE: usize = 80;

/// Best-effort text view of a BIOS message-port buffer for log emission.
fn bios_message_text(bytes: &[u8]) -> &str {
    core::str::from_utf8(bytes).unwrap_or("<non-utf8 BIOS message>")
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

    /// Bochs port-0xE9 debug console byte stream (unmapped.cc port_e9_hack;
    /// optional in upstream, always-on here). Host code (examples/GUI) can
    /// drain and print it. BIOS/VGABIOS message ports (0x400-0x403,
    /// 0x500-0x503) do NOT land here — Bochs biosdev.cc routes those to the
    /// log, never the guest-visible console (see `bios_message_byte`).
    port_e9_output: RingBuffer<u8, DEBUGCON_CAPACITY>,

    /// Bochs BIOS POST codes (port 0x80, sometimes 0x84).
    ///
    /// These are not ASCII; they are diagnostic progress codes used by many BIOSes.
    port80_output: RingBuffer<u8, PORT80_CAPACITY>,

    /// Bochs biosdev.cc rombios message accumulator ("biosdev" logger).
    bios_message: [u8; BX_BIOS_MESSAGE_SIZE],
    bios_message_i: usize,
    bios_panic_flag: bool,
    /// Bochs biosdev.cc vgabios message accumulator ("vgabios" logger).
    vgabios_message: [u8; BX_BIOS_MESSAGE_SIZE],
    vgabios_message_i: usize,
    vgabios_panic_flag: bool,

    /// Last I/O read port and value (for stuck-loop diagnostics)
    pub(crate) last_io_read_port: u16,
    pub(crate) last_io_read_value: u32,
    /// Total I/O port reads (for progress diagnostics)
    pub(crate) diag_io_reads: u64,
    /// Total I/O port writes
    pub(crate) diag_io_writes: u64,
    /// Pointer to DeviceManager for enum-based I/O dispatch.
    /// Set by the emulator before CPU execution; single-threaded.
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
    /// Bochs unmapped.cc port-0x8900 "Shutdown" protocol progress (0..=8).
    /// Writing the bytes of "Shutdown" in order advances this; completing it
    /// requests emulator termination. Bochs's unmapped device has no
    /// register_state(), so this is deliberately not snapshotted (defaults to
    /// 0 on restore, matching upstream).
    shutdown_state: u8,
    /// Set when the port-0x8900 protocol completes (Bochs `bx_user_quit = 1`).
    /// Drained by the emulator at the next scheduler boundary into its stop
    /// flag. Transient — never snapshotted.
    shutdown_requested: bool,
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
            bios_message: [0; BX_BIOS_MESSAGE_SIZE],
            bios_message_i: 0,
            bios_panic_flag: false,
            vgabios_message: [0; BX_BIOS_MESSAGE_SIZE],
            vgabios_message_i: 0,
            vgabios_panic_flag: false,
            last_io_read_port: 0,
            last_io_read_value: 0,
            diag_io_reads: 0,
            diag_io_writes: 0,
            pic_intr_level: None,
            hrq_level: None,
            scheduler_boundary_requested: false,
            timer_requests: TimerRequestTable::default(),
            timer_ips: 1,
            shutdown_state: 0,
            shutdown_requested: false,
        }
    }

    /// Take (and clear) a pending port-0x8900 shutdown request. Bochs sets
    /// `bx_user_quit = 1` and BX_FATALs; the emulator instead translates this
    /// into its stop flag at the next scheduler boundary (a graceful stop in
    /// place of Bochs's immediate abort). Transient — never snapshotted.
    pub(crate) fn take_shutdown_request(&mut self) -> bool {
        core::mem::take(&mut self.shutdown_requested)
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
        slot: DevSlot,
        port: u16,
        name: &'static str,
        mask: u8,
    ) {
        let entry = &mut self.read_handlers[port as usize];
        entry.slot = slot;
        entry.mask = mask;
        // `name` is not retained: the port tables span the whole 64 Ki port
        // space twice, so a stored `&'static str` costs 2 MiB to carry a
        // string nothing reads back. It is logged here instead.
        tracing::trace!(
            "Registered I/O read handler for port {:#06x}: {}",
            port,
            name
        );
    }

    /// Register a write handler for a specific I/O port
    pub fn register_io_write_handler(
        &mut self,
        slot: DevSlot,
        port: u16,
        name: &'static str,
        mask: u8,
    ) {
        let entry = &mut self.write_handlers[port as usize];
        entry.slot = slot;
        entry.mask = mask;
        // See `register_io_read_handler`: the name is logged, not stored.
        tracing::trace!(
            "Registered I/O write handler for port {:#06x}: {}",
            port,
            name
        );
    }

    /// Register both read and write handlers for a port
    pub fn register_io_handler(&mut self, slot: DevSlot, port: u16, name: &'static str, mask: u8) {
        self.register_io_read_handler(slot, port, name, mask);
        self.register_io_write_handler(slot, port, name, mask);
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
    pub fn inp(
        &mut self,
        port: u16,
        io_len: u8,
        current_ticks: u64,
        pc_system: &mut crate::pc_system::BxPcSystemC,
        dm: &mut devices::DeviceManager,
    ) -> u32 {
        self.diag_io_reads += 1;
        let entry = &self.read_handlers[port as usize];
        let slot = entry.slot;
        let len_mask = 1u8 << (io_len.trailing_zeros() as u8);
        // Present only for an access a device model can actually service: a
        // claimed port, a width the registration allows, and a width with an
        // architectural encoding — one outside {1,2,4} can never be issued, so
        // the default handler answers it.
        let handler_width = rusty_box_devices::api::IoLen::from_bytes(io_len)
            .filter(|_| !slot.is_none() && (entry.mask & len_mask) != 0);

        let value = if let Some(width) = handler_width {
            {
                // Devices on the device API route through one context; the
                // rest still go through per-device dispatch. The two sets are
                // disjoint by construction — `bind_pio` answers for exactly the
                // converted slots — so neither path can shadow the other.
                let mut routed = None;
                if let Some(bound) = dm.bind_pio(slot, port) {
                    routed = Some(wiring::with_device_ctx(
                        bound.pic,
                        pc_system,
                        bound.handles,
                        current_ticks,
                        |ctx| bound.device.pio_read(port, width, ctx),
                    ));
                }
                let result = match routed {
                    Some(value) => value,
                    None => Self::dispatch_read(dm, slot, port, io_len),
                };
                let (fwds, count) = dm.pic.take_ioapic_forwards();
                if let Some(level) = dm.dma.take_hrq_request() {
                    self.hrq_level = Some(level);
                }
                let devices::DeviceManager {
                    ref mut pic,
                    ref mut ioapic,
                    ..
                } = *dm;
                for &(irq, level) in &fwds[..count] {
                    ioapic.set_irq_level(irq, level, Some(&mut *pic), None);
                }
                if let Some(level) = Self::take_pic_level_after_dispatch(dm) {
                    self.pic_intr_level = Some(level);
                }
                result
            }
        } else {
            self.default_read_handler(port, io_len)
        };
        self.last_io_read_port = port;
        self.last_io_read_value = value;
        value
    }

    /// Write to an I/O port.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn outp(
        &mut self,
        port: u16,
        value: u32,
        io_len: u8,
        current_ticks: u64,
        pc_system: &mut crate::pc_system::BxPcSystemC,
        dm: &mut devices::DeviceManager,
        mem: &mut crate::memory::BxMemC,
    ) {
        self.diag_io_writes += 1;
        let entry = &self.write_handlers[port as usize];
        let slot = entry.slot;
        let len_mask = 1u8 << (io_len.trailing_zeros() as u8);
        // See `inp`.
        let handler_width = rusty_box_devices::api::IoLen::from_bytes(io_len)
            .filter(|_| !slot.is_none() && (entry.mask & len_mask) != 0);

        if let Some(width) = handler_width {
            {
                // See `inp` on why the two paths cannot overlap.
                let mut routed = false;
                if let Some(bound) = dm.bind_pio(slot, port) {
                    wiring::with_device_ctx(
                        bound.pic,
                        pc_system,
                        bound.handles,
                        current_ticks,
                        |ctx| bound.device.pio_write(port, value, width, ctx),
                    );
                    routed = true;
                }
                if !routed {
                    Self::dispatch_write(dm, slot, port, value, io_len, mem);
                }
                dm.apply_dispatch_effects();
                // The IDE controller arms its own seek and bus-master
                // deadlines, still anchored to this OUT.
                Self::drain_ide_timers(dm, pc_system, current_ticks);
                let (fwds, count) = dm.pic.take_ioapic_forwards();
                if let Some(level) = dm.dma.take_hrq_request() {
                    self.hrq_level = Some(level);
                }
                let devices::DeviceManager { pic, ioapic, .. } = dm;
                for &(irq, level) in &fwds[..count] {
                    ioapic.set_irq_level(irq, level, Some(&mut *pic), None);
                }
                if let Some(level) = Self::take_pic_level_after_dispatch(dm) {
                    self.pic_intr_level = Some(level);
                }
                if dm.has_pending_machine_boundary() {
                    self.scheduler_boundary_requested = true;
                }
            }
            return;
        }

        self.default_write_handler(port, value, io_len);
    }

    /// Service a memory-mapped read that the memory map attributed to `token`.
    ///
    /// The counterpart of [`Self::inp`] for physical addresses. Memory decides
    /// *that* an address is claimed and by whom; this decides what that means,
    /// which is the split that lets the memory subsystem hold no device
    /// references at all.
    ///
    /// Returns whether a device serviced the access. `false` means the token
    /// named no memory-mapped device — the caller leaves the buffer as it found
    /// it, which reads as an unclaimed region rather than as stale data.
    pub fn mmio_read(
        &mut self,
        hit: crate::memory::mmio_map::MmioHit,
        len: u32,
        data: &mut [u8],
        now_ticks: u64,
        pc_system: &mut crate::pc_system::BxPcSystemC,
        dm: &mut devices::DeviceManager,
    ) -> bool {
        let slot = DevSlot::from_mmio_token(hit.token);
        let window = DevSlot::window_from_mmio_token(hit.token);
        let at = rusty_box_devices::api::WindowOffset(hit.offset);
        match dm.bind_mmio(slot) {
            Some(bound) => {
                wiring::with_device_ctx(
                    bound.pic,
                    pc_system,
                    bound.handles,
                    now_ticks,
                    |ctx| bound.device.mmio_read(window, at, len, data, ctx),
                );
                true
            }
            None => {
                tracing::warn!(
                    "MMIO read at {window:?}+{:#x} routed to {slot:?}, which maps no device",
                    hit.offset
                );
                false
            }
        }
    }

    /// Service a memory-mapped write attributed to `hit`. See [`Self::mmio_read`].
    pub fn mmio_write(
        &mut self,
        hit: crate::memory::mmio_map::MmioHit,
        len: u32,
        data: &[u8],
        now_ticks: u64,
        pc_system: &mut crate::pc_system::BxPcSystemC,
        dm: &mut devices::DeviceManager,
    ) -> bool {
        let slot = DevSlot::from_mmio_token(hit.token);
        let window = DevSlot::window_from_mmio_token(hit.token);
        let at = rusty_box_devices::api::WindowOffset(hit.offset);
        match dm.bind_mmio(slot) {
            Some(bound) => {
                wiring::with_device_ctx(
                    bound.pic,
                    pc_system,
                    bound.handles,
                    now_ticks,
                    |ctx| bound.device.mmio_write(window, at, len, data, ctx),
                );
                true
            }
            None => {
                tracing::warn!(
                    "MMIO write at {window:?}+{:#x} routed to {slot:?}, which maps no device",
                    hit.offset
                );
                false
            }
        }
    }

    /// Bulk-read from an I/O port.
    ///
    /// For IDE data ports (0x1F0, 0x170), this copies up to `buf.len()` bytes
    /// directly from the ATA controller buffer in one call, avoiding per-word
    /// handler dispatch overhead. Returns the number of bytes actually read.
    /// For other ports, returns 0 (caller should fall back to per-word I/O).
    ///
    /// `_current_ticks` is the issuing instruction's epoch. The bulk IDE path
    /// has no clocked register transition beyond the transfer itself, so
    /// nothing here consumes it; it stays in the signature because a
    /// device-owned timer producer on this path would have to anchor to the
    /// same boundary the other dispatch entry points use.
    pub fn inp_bulk(
        &mut self,
        port: u16,
        io_len: u8,
        buf: &mut [u8],
        _current_ticks: u64,
        dm: &mut devices::DeviceManager,
    ) -> usize {
        // Only optimize IDE data ports (base + 0 = data register).
        if (port != 0x1F0 && port != 0x170) || (io_len != 2 && io_len != 4) {
            return 0;
        }
        let entry = &self.read_handlers[port as usize];
        if entry.slot != DevSlot::IDE {
            return 0;
        }

        let bytes_read = {
            let result = {
                let devices::DeviceManager {
                    ref mut ide,
                    ref mut pic,
                    ..
                } = *dm;
                ide.bulk_read_data(port, io_len, buf, pic)
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
            if let Some(level) = Self::take_pic_level_after_dispatch(dm) {
                self.pic_intr_level = Some(level);
            }
            result
        };
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

        // Bochs port-0xE9 debug console (unmapped.cc port_e9_hack; optional
        // in upstream, always-on here): bytes go to the host-drainable stream.
        if io_len == 1 && address == 0x00E9 {
            self.port_e9_output.push_back(value as u8);
            return;
        }

        // Bochs biosdev.cc — rombios/vgabios message and panic ports. Text
        // accumulates into per-BIOS line buffers and flushes to the host log
        // only (info/debug/error level), never to a guest-visible stream.
        // Bochs registers the panic ports for 1- and 2-byte writes and the
        // message ports byte-wide.
        match address {
            // Bochs biosdev.cc port 0x0401: zero latches the panic flag; a
            // buffered message flushes as the panic text; otherwise fall
            // through to the line-number panic exactly like the C switch.
            0x0401 if io_len <= 2 => {
                if value == 0 {
                    self.bios_panic_flag = true;
                } else if self.bios_message_i > 0 {
                    let end = self.bios_message_i.min(BX_BIOS_MESSAGE_SIZE - 1);
                    self.bios_message_i = 0;
                    tracing::error!(
                        "[BIOS] panic: {}",
                        bios_message_text(&self.bios_message[..end])
                    );
                } else {
                    tracing::error!("[BIOS] panic at rombios.c, line {}", value);
                }
            }
            0x0400 if io_len <= 2 => {
                if value > 0 {
                    tracing::error!("[BIOS] panic at rombios.c, line {}", value);
                }
            }
            0x0402 | 0x0403 if io_len == 1 => {
                Self::bios_message_byte(
                    &mut self.bios_message,
                    &mut self.bios_message_i,
                    &mut self.bios_panic_flag,
                    "BIOS",
                    address == 0x0403,
                    value as u8,
                );
            }
            // Bochs biosdev.cc port 0x0502: vgabios twin of 0x0401.
            0x0502 if io_len <= 2 => {
                if value == 0 {
                    self.vgabios_panic_flag = true;
                } else if self.vgabios_message_i > 0 {
                    let end = self.vgabios_message_i.min(BX_BIOS_MESSAGE_SIZE - 1);
                    self.vgabios_message_i = 0;
                    tracing::error!(
                        "[VBIOS] panic: {}",
                        bios_message_text(&self.vgabios_message[..end])
                    );
                } else {
                    tracing::error!("[VBIOS] panic at vgabios.c, line {}", value);
                }
            }
            0x0501 if io_len <= 2 => {
                if value > 0 {
                    tracing::error!("[VBIOS] panic at vgabios.c, line {}", value);
                }
            }
            0x0500 | 0x0503 if io_len == 1 => {
                Self::bios_message_byte(
                    &mut self.vgabios_message,
                    &mut self.vgabios_message_i,
                    &mut self.vgabios_panic_flag,
                    "VBIOS",
                    address == 0x0503,
                    value as u8,
                );
            }
            // Bochs unmapped.cc port 0x8900: the "Shutdown" host<->guest
            // shutdown protocol. Writing the ASCII bytes of "Shutdown" in
            // sequence advances a state machine; the 8th ('n') requests
            // emulator termination (Bochs `bx_user_quit = 1` + BX_FATAL).
            // As in Bochs, an out-of-sequence *"Shutdown" letter* leaves the
            // state unchanged, while any other byte resets it. The write path
            // sees the full value (Bochs switches on `value`, not `value & 0xff`).
            0x8900 => {
                match value {
                    0x53 if self.shutdown_state == 0 => self.shutdown_state = 1, // 'S'
                    0x68 if self.shutdown_state == 1 => self.shutdown_state = 2, // 'h'
                    0x75 if self.shutdown_state == 2 => self.shutdown_state = 3, // 'u'
                    0x74 if self.shutdown_state == 3 => self.shutdown_state = 4, // 't'
                    0x64 if self.shutdown_state == 4 => self.shutdown_state = 5, // 'd'
                    0x6F if self.shutdown_state == 5 => self.shutdown_state = 6, // 'o'
                    0x77 if self.shutdown_state == 6 => self.shutdown_state = 7, // 'w'
                    0x6E if self.shutdown_state == 7 => self.shutdown_state = 8, // 'n'
                    // A "Shutdown" letter that does not match the current step:
                    // Bochs's `if (s.shutdown == N)` simply fails, leaving the
                    // state unchanged (it is NOT reset).
                    0x53 | 0x68 | 0x75 | 0x74 | 0x64 | 0x6F | 0x77 | 0x6E => {}
                    // Bochs `default: s.shutdown = 0`.
                    _ => self.shutdown_state = 0,
                }
                if self.shutdown_state == 8 {
                    tracing::info!("Shutdown port (0x8900): shutdown requested");
                    self.shutdown_requested = true;
                }
            }
            _ => {}
        }
    }

    /// Bochs biosdev.cc write() message-port body: accumulate a byte, flush
    /// the line to the log on overflow or newline; a pending panic flag
    /// promotes the newline flush to error level (and only that flush
    /// clears the flag, exactly like Bochs).
    fn bios_message_byte(
        message: &mut [u8; BX_BIOS_MESSAGE_SIZE],
        index: &mut usize,
        panic_flag: &mut bool,
        source: &str,
        is_debug: bool,
        value: u8,
    ) {
        message[*index] = value;
        *index += 1;
        if *index >= BX_BIOS_MESSAGE_SIZE {
            *index = 0;
            let text = bios_message_text(&message[..BX_BIOS_MESSAGE_SIZE - 1]);
            if is_debug {
                tracing::debug!("[{}] {}", source, text);
            } else {
                tracing::info!("[{}] {}", source, text);
            }
        } else if value == b'\n' {
            let end = *index - 1;
            *index = 0;
            let text = bios_message_text(&message[..end]);
            if *panic_flag {
                tracing::error!("[{}] panic: {}", source, text);
            } else if is_debug {
                tracing::debug!("[{}] {}", source, text);
            } else {
                tracing::info!("[{}] {}", source, text);
            }
            *panic_flag = false;
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
    pub fn drain_port_e9_output(&mut self) -> DebugconDrain<'_> {
        DebugconDrain(self.port_e9_output.drain())
    }

    /// Drain BIOS POST codes (port 0x80/0x84) as an iterator (no-alloc).
    pub fn drain_port80_output(&mut self) -> Port80Drain<'_> {
        Port80Drain(self.port80_output.drain())
    }

    /// Arm the IDE controller's own deadlines, anchored to the access that
    /// produced them.
    fn drain_ide_timers(
        dm: &mut devices::DeviceManager,
        pc_system: &mut crate::pc_system::BxPcSystemC,
        current_ticks: u64,
    ) {
        let mut handles = wiring::TimerHandles::default();
        for channel in 0..2usize {
            for device in 0..2usize {
                handles.set(
                    harddrv::BxHardDriveC::seek_timer_local(channel, device),
                    dm.ide.drives.seek_timer_handles[channel][device],
                );
            }
            handles.set(
                pci_ide::BxPciIde::bmdma_timer_local(channel),
                dm.ide.bus_master.bmdma[channel].timer_index,
            );
        }
        let devices::DeviceManager {
            ref mut ide,
            ref mut pic,
            ..
        } = *dm;
        let (harddrv, pci_ide, _scratch) = ide.split();
        wiring::with_device_ctx(pic, pc_system, handles, current_ticks, |ctx| {
            harddrv.drain_seek_timers(ctx);
            pci_ide.drain_bmdma_timers(ctx);
        });
    }

    /// Dispatch a port read to a device not yet on the device API.
    ///
    /// Disjoint from [`devices::DeviceManager::bind_pio`] by construction: a
    /// slot is answered by exactly one of the two, so a device converted to the
    /// device API loses its arm here in the same change.
    ///
    /// Takes no clock: every device still on this path answers from its own
    /// registers alone. The VGA was the last one that needed emulated time,
    /// and it reads it off its context now.
    #[inline]
    fn dispatch_read(
        dm: &mut devices::DeviceManager,
        slot: DevSlot,
        port: u16,
        io_len: u8,
    ) -> u32 {
        match slot {
            DevSlot::PIC => dm.pic.read(port, io_len),
            DevSlot::DMA => dm.dma.read(port, io_len),
            DevSlot::IDE => {
                let devices::DeviceManager { ide, pic, .. } = dm;
                ide.read(port, io_len, pic)
            }
            DevSlot::PORT92 => dm.port92_read(port, io_len),
            DevSlot::PCI => dm.pci_read(port, io_len),
            DevSlot::FW_CFG => dm.fw_cfg.read_port_mut(port, io_len),
            // A registered slot that neither path claims is a wiring mistake,
            // and it presents to the guest as a dead port rather than a
            // plausible value — the failure mode a routing table should have.
            _ => {
                tracing::warn!(
                    "I/O read of port {port:#06x} routed to {slot:?}, which has no read handler"
                );
                0xFFFF_FFFF
            }
        }
    }

    /// Dispatch a port write to a device not yet on the device API. See
    /// [`Self::dispatch_read`].
    #[inline]
    fn dispatch_write(
        dm: &mut devices::DeviceManager,
        slot: DevSlot,
        port: u16,
        value: u32,
        io_len: u8,
        mem: &mut crate::memory::BxMemC,
    ) {
        match slot {
            DevSlot::PIC => dm.pic.write(port, value, io_len),
            DevSlot::DMA => dm.dma.write(port, value, io_len),
            DevSlot::IDE => {
                let devices::DeviceManager { ide, pic, .. } = dm;
                ide.write(port, value, io_len, pic)
            }
            DevSlot::PORT92 => dm.port92_write(port, value, io_len),
            DevSlot::PCI => dm.pci_write(port, value, io_len),
            DevSlot::FW_CFG => dm.fw_cfg_write(port, value, io_len, mem),
            // See `dispatch_read`.
            _ => tracing::warn!(
                "I/O write of port {port:#06x} routed to {slot:?}, which has no write handler"
            ),
        }
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
fn invalid_bx_devices_snapshot(message: &'static str) -> SnapError {
    SnapError::Invalid(message)
}

#[cfg(feature = "std")]
fn timer_request_snapshot_len(request: TimerRequest) -> SnapResult<u64> {
    match request {
        TimerRequest::Unchanged | TimerRequest::Deactivate => Ok(1),
        TimerRequest::Activate { .. } => checked_snapshot_len_add(1, 17),
    }
}

#[cfg(feature = "std")]
fn write_timer_request_snapshot<W: SnapWrite>(
    writer: &mut W,
    request: TimerRequest,
) -> SnapResult<()> {
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
fn read_timer_request_snapshot<R: SnapRead>(
    reader: &mut R,
) -> SnapResult<TimerRequest> {
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
    pub(crate) fn snapshot_v3_body_len(&self) -> SnapResult<u64> {
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
    pub(crate) fn save_snapshot_v3_body<W: SnapWrite>(&self, writer: &mut W) -> SnapResult<()> {
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
    pub(crate) fn restore_snapshot_v3_body<R: SnapRead>(
        &mut self,
        reader: &mut R,
    ) -> SnapResult<BxDevicesSnapshotRestore> {
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

    fn validate_snapshot_v3_state(&self) -> SnapResult<()> {
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
    use rusty_box_devices::pci::PciDevice;

    /// BxDevicesC is ~1.5MB due to [IoHandlerEntry; 65536] x2.
    /// Allocate on heap to avoid test stack overflow.
    fn boxed_devices() -> alloc::boxed::Box<BxDevicesC> {
        // Use alloc_zeroed + field writes to avoid stack intermediary.
        let layout = alloc::alloc::Layout::new::<BxDevicesC>();
        unsafe {
            let ptr = alloc::alloc::alloc_zeroed(layout) as *mut BxDevicesC;
            assert!(!ptr.is_null());
            // Zero bits give DevSlot::NONE, which is the right slot, but a
            // zero width mask is not the default (0x7 = all widths), so the
            // entries still have to be written rather than left zeroed.
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

    /// A chipset device must describe an effect, never perform it.
    ///
    /// The producers are pure functions of committed configuration, so asking
    /// twice must give the same answer — the boundary drain calls them from a
    /// path that can run any number of times, and a producer that mutated
    /// would make the second drain disagree with the first.
    #[test]
    fn chipset_effects_are_derived_from_configuration_not_performed() {
        on_big_stack(|| {
            let mut dm = alloc::boxed::Box::new(devices::DeviceManager::new());
            dm.pci_bridge.reset();
            dm.pci2isa.reset();

            // SMRAME|DOPEN: open and unrestricted. SMRAM is a memory mapping,
            // not a BAR, so the write asks for a PAM/SMRAM re-derive and no
            // port re-registration.
            let effects = dm.pci_bridge.pci_write(0x72, 0x48, 1);
            assert!(effects.smram_changed && !effects.pam_changed);
            let first = dm.pci_bridge.smram_effect();
            assert_eq!(
                first,
                rusty_box_devices::api::ChipsetEffect::Smram(rusty_box_devices::api::SmramControl::Enable {
                    dopen: true,
                    dcls: false
                })
            );
            assert_eq!(first, dm.pci_bridge.smram_effect(), "producer must be pure");

            // Every PAM area is described, not just the ones that changed.
            let rusty_box_devices::api::ChipsetEffect::ShadowRam(areas) = dm.pci_bridge.shadow_ram_effect()
            else {
                panic!("the bridge must describe shadow RAM as such")
            };
            assert_eq!(areas.len(), rusty_box_devices::api::PAM_AREAS);

            // The ACPI suspend-to-ram store is a request, taken once.
            dm.acpi.suspend_to_ram_pending = true;
            assert_eq!(
                dm.acpi.take_pending_effect(),
                Some(rusty_box_devices::api::ChipsetEffect::CmosByte {
                    index: 0x0F,
                    value: 0xFE
                })
            );
            assert_eq!(
                dm.acpi.take_pending_effect(),
                None,
                "a taken effect must not be raised twice"
            );
        });
    }

    /// A memory-mapped access must reach the device the map named.
    ///
    /// The port and memory buses now share one slot namespace, so this pins the
    /// half memory cannot check for itself: the token a physical access reports
    /// has to bind to a real device and that device has to observe the write.
    /// The VGA text buffer is the case the whole boot depends on.
    #[test]
    fn a_reported_mmio_token_reaches_the_device_that_owns_it() {
        on_big_stack(|| {
            let mut dm = alloc::boxed::Box::new(devices::DeviceManager::new());
            let mut pc_system = crate::pc_system::BxPcSystemC::new();
            pc_system.initialize(1_000_000);

            // Writing through the slot the map would report must land in the
            // device, not merely be accepted. IOREGSEL is the cleanest witness:
            // an unconditional register that reads back what was written,
            // needing no mode programming first.
            const IOREGSEL: rusty_box_devices::api::WindowOffset = rusty_box_devices::api::WindowOffset(0);
            let window = rusty_box_devices::api::WindowId::FIRST;
            let bound = dm
                .bind_mmio(DevSlot::IOAPIC)
                .expect("the I/O APIC slot must map a device");
            wiring::with_device_ctx(bound.pic, &mut pc_system, bound.handles, 0, |ctx| {
                bound
                    .device
                    .mmio_write(window, IOREGSEL, 4, &0x12u32.to_ne_bytes(), ctx)
            });

            let mut readback = [0u8; 4];
            let bound = dm.bind_mmio(DevSlot::IOAPIC).expect("still bound");
            wiring::with_device_ctx(bound.pic, &mut pc_system, bound.handles, 0, |ctx| {
                bound.device.mmio_read(window, IOREGSEL, 4, &mut readback, ctx)
            });
            assert_eq!(
                u32::from_ne_bytes(readback),
                0x12,
                "the value must come back from the device that took it"
            );

            // The other memory-mapped slots bind too, and a port-only one does not.
            assert!(dm.bind_mmio(DevSlot::VGA).is_some());
            assert!(dm.bind_mmio(DevSlot::HPET).is_some());
            assert!(
                dm.bind_mmio(DevSlot::SERIAL).is_none(),
                "a slot with no physical range must not bind a memory device"
            );
        });
    }

    /// Every slot must be answered by exactly one of the two dispatch paths.
    ///
    /// The bus routes a claimed port either through `bind_pio` (device API) or
    /// through `dispatch_read`/`dispatch_write`, and picks between them on the
    /// slot alone. A slot claimed by both would make the legacy arm dead code
    /// that still looks live; a slot claimed by neither would present a
    /// registered port as unclaimed. Neither is visible at a call site, so it
    /// is pinned here — a device conversion that forgets to delete its legacy
    /// arm fails this test rather than leaving a plausible-looking duplicate.
    #[test]
    fn every_slot_is_claimed_by_exactly_one_dispatch_path() {
        const ALL: &[DevSlot] = &[
            DevSlot::PIC,
            DevSlot::PIT,
            DevSlot::CMOS,
            DevSlot::DMA,
            DevSlot::KEYBOARD,
            DevSlot::IDE,
            DevSlot::SERIAL,
            DevSlot::VGA,
            DevSlot::PORT92,
            DevSlot::PCI,
            DevSlot::ACPI,
            DevSlot::FW_CFG,
            // Memory-mapped only: they claim no port, so the port bus must
            // answer for neither of them.
            DevSlot::IOAPIC,
            DevSlot::HPET,
        ];
        /// Slots whose device is on the device API. Kept as data so the two
        /// sets are compared, not merely asserted about one at a time.
        const ON_DEVICE_API: &[DevSlot] = &[
            DevSlot::SERIAL,
            DevSlot::ACPI,
            DevSlot::CMOS,
            DevSlot::PIT,
            DevSlot::KEYBOARD,
            DevSlot::VGA,
        ];

        on_big_stack(|| {
            let mut dm = alloc::boxed::Box::new(devices::DeviceManager::new());
            for &slot in ALL {
                let bound = dm.bind_pio(slot, 0).is_some();
                assert_eq!(
                    bound,
                    ON_DEVICE_API.contains(&slot),
                    "{slot:?} is routed by the wrong dispatch path"
                );
            }
            assert!(
                dm.bind_pio(DevSlot::NONE, 0).is_none(),
                "the unclaimed slot must never bind to a device"
            );
        });
    }

    #[test]
    fn test_default_handlers() {
        let mut devices = boxed_devices();
        let mut pc_system = crate::pc_system::BxPcSystemC::new();
        let mut dm = devices::DeviceManager::new();

        // Reading unhandled port should return 0xFF/0xFFFF/0xFFFFFFFF
        assert_eq!(devices.inp(0x1234, 1, 0, &mut pc_system, &mut dm), 0xFF);
        assert_eq!(devices.inp(0x1234, 2, 0, &mut pc_system, &mut dm), 0xFFFF);
        assert_eq!(devices.inp(0x1234, 4, 0, &mut pc_system, &mut dm), 0xFFFFFFFF);
    }

    // Bochs unmapped.cc port 0x8900 "Shutdown" protocol: the ASCII bytes of
    // "Shutdown" in order request emulator termination; an out-of-sequence
    // "Shutdown" letter leaves the state unchanged, any other byte resets it.
    #[test]
    fn port_8900_shutdown_protocol_matches_bochs() {
        let mut dev = boxed_devices();

        // Partial progress, then a non-"Shutdown" byte resets (Bochs default:).
        dev.default_write_handler(0x8900, u32::from(b'S'), 1);
        dev.default_write_handler(0x8900, u32::from(b'h'), 1);
        assert_eq!(dev.shutdown_state, 2);
        assert!(!dev.shutdown_requested);
        dev.default_write_handler(0x8900, u32::from(b'X'), 1);
        assert_eq!(dev.shutdown_state, 0);

        // An out-of-sequence *"Shutdown" letter* leaves the state unchanged
        // (Bochs: the `if (s.shutdown == N)` guard fails without resetting).
        dev.default_write_handler(0x8900, u32::from(b'S'), 1); // -> 1
        dev.default_write_handler(0x8900, u32::from(b'S'), 1); // 'S' at state 1: unchanged
        assert_eq!(dev.shutdown_state, 1);

        // Completing the full "Shutdown" sequence requests termination.
        let mut dev = boxed_devices();
        for byte in b"Shutdown" {
            dev.default_write_handler(0x8900, u32::from(*byte), 1);
        }
        assert_eq!(dev.shutdown_state, 8);
        assert!(dev.shutdown_requested);
        assert!(dev.take_shutdown_request());
        assert!(!dev.take_shutdown_request(), "cleared after taking");
    }

    #[test]
    fn bios_message_ports_stay_out_of_the_e9_console_stream() {
        let mut devices = boxed_devices();
        let mut pc_system = crate::pc_system::BxPcSystemC::new();
        let mut dm = devices::DeviceManager::new();
        let mut mem = crate::memory::test_ram();

        // Bochs biosdev.cc: rombios/vgabios message ports flush to the log on
        // newline and must never reach the guest-visible 0xE9 console stream
        // (this leaked BIOS text onto the COM1 stdout mirror).
        for byte in b"PIIX3/PIIX4 init: elcr=60 70\n" {
            devices.outp(0x0402, u32::from(*byte), 1, 0, &mut pc_system, &mut dm, &mut mem);
        }
        for byte in b"VBE present\n" {
            devices.outp(0x0500, u32::from(*byte), 1, 0, &mut pc_system, &mut dm, &mut mem);
        }
        assert!(devices.port_e9_output.is_empty());
        assert_eq!(devices.bios_message_i, 0, "newline must flush the rombios buffer");
        assert_eq!(devices.vgabios_message_i, 0, "newline must flush the vgabios buffer");

        // A line longer than the Bochs 80-byte buffer flushes on overflow and
        // keeps accumulating the remainder.
        for _ in 0..BX_BIOS_MESSAGE_SIZE + 5 {
            devices.outp(0x0403, u32::from(b'x'), 1, 0, &mut pc_system, &mut dm, &mut mem);
        }
        assert_eq!(devices.bios_message_i, 5);
        assert!(devices.port_e9_output.is_empty());

        // The genuine port-0xE9 debug console still lands in the stream.
        devices.outp(0x00E9, u32::from(b'X'), 1, 0, &mut pc_system, &mut dm, &mut mem);
        assert_eq!(devices.port_e9_output.len(), 1);
    }

    /// Port registration is per-instance state, not a global table: two
    /// device buses in one process must not see each other's handlers.
    /// Asserted on the routing tables themselves — an unclaimed port and a
    /// claimed one can return the same value by coincidence.
    #[test]
    fn test_multiple_instances() {
        let mut dev1 = boxed_devices();
        let dev2 = boxed_devices();
        let mut pc_system = crate::pc_system::BxPcSystemC::new();
        let mut dm = devices::DeviceManager::new();

        dev1.register_io_read_handler(DevSlot::PIC, 0x100, "test", 0x1);

        assert_eq!(dev1.read_handlers[0x100].slot, DevSlot::PIC);
        assert!(
            dev2.read_handlers[0x100].slot.is_none(),
            "registering on one bus must not claim the port on another"
        );

        // The unclaimed bus still answers with the default handler.
        assert_eq!(dev2.read_handlers[0x100].slot.is_none(), true);
        let _ = dev1.inp(0x100, 1, 0, &mut pc_system, &mut dm);
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
            let mut pc_system = crate::pc_system::BxPcSystemC::new();
            let mut dm = devices::DeviceManager::new();
            // A clear notification followed by a later assertion can coexist
            // before the raw I/O borrow is released. The transport must
            // publish the final physical pin, not replay those edges in order.
            dm.pic.irq_cleared = true;
            dm.pic.irq_pending = true;
            dm.pic.master.int_pin = true;

            io.register_io_read_handler(DevSlot::PIC, 0x20, "PIC", 0x1);
            let _ = io.inp(0x20, 1, 91, &mut pc_system, &mut dm);

            assert_eq!(io.take_pic_intr_level(), Some(true));
            assert_eq!(io.take_pic_intr_level(), None);
        });
    }
    #[test]
    fn keyboard_port60_read_lowers_irq_and_arms_no_owner_request() {
        on_big_stack(|| {
            let mut io = boxed_devices();
            let mut pc_system = crate::pc_system::BxPcSystemC::new();
            let mut dm = devices::DeviceManager::new();
            dm.keyboard.send_scancode(0x1E);
            // Bochs keyboard.cc periodic(): the transfer fire makes the byte
            // readable (OBF) and latches IRQ1, but the IRQ is collected on the
            // NEXT continuous fire (one serial-delay period later).
            assert_eq!(dm.keyboard.timer_callback() & 0x01, 0);
            assert!(dm.keyboard.kbd_controller.outb);
            let irq_mask = dm.keyboard.timer_callback();
            assert_eq!(irq_mask & 0x01, 0x01);
            let delivered = u32::from(dm.keyboard.kbd_controller.kbd_output_buffer);
            dm.pic.raise_irq(1);
            assert_ne!(dm.pic.master.irq_in[1], 0);

            io.set_timer_ips(1_000_000);
            io.register_io_read_handler(DevSlot::KEYBOARD, keyboard::KBD_DATA_PORT, "Keyboard", 0x1);
            assert_eq!(io.inp(keyboard::KBD_DATA_PORT, 1, 77, &mut pc_system, &mut dm), delivered);

            assert_eq!(dm.pic.master.irq_in[1], 0);
            // The 8042 timer is continuous (Bochs keyboard.cc): keyboard port
            // I/O must not produce one-shot owner timer requests.
            assert_eq!(
                io.take_timer_requests().get(DeviceTimerOwner::Keyboard),
                TimerRequest::Unchanged
            );
        });
    }

}
