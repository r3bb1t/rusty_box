//! The device-facing contracts a device model may use *during* dispatch.
//!
//! Bochs devices call `DEV_pic_raise_irq` and `bx_pc_system.activate_timer`
//! synchronously from inside their port handlers. This port could not: I/O
//! dispatch reached devices through a raw bus pointer that could not also
//! reach the PIC or the timer wheel, so every such call had to be latched into
//! a fixed side table and replayed at the next scheduler boundary.
//!
//! These traits restore the Bochs shape. A converted device receives a
//! [`DeviceCtx`] carrying the capabilities it may exercise, and calls them
//! directly. The latch tables exist only for devices not yet converted.
//!
//! Everything here is `no_std` and allocation-free: the context borrows, it
//! never owns, and the trait objects are `&mut dyn` rather than boxed.

/// An ISA interrupt line (IRQ0-15) — Bochs `DEV_pic_raise_irq` line numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrqLine(pub u8);

/// Width of a port access. Bochs passes `io_len` as a raw byte count; this
/// makes the three legal widths explicit at the trait boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoLen {
    Byte,
    Word,
    Dword,
}

impl IoLen {
    /// Bochs `io_len` byte count.
    #[inline]
    pub const fn bytes(self) -> u8 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
            Self::Dword => 4,
        }
    }

    /// Widths other than 1/2/4 are not architecturally reachable: the CPU's
    /// port paths only ever issue those three.
    #[inline]
    pub const fn from_bytes(bytes: u8) -> Option<Self> {
        match bytes {
            1 => Some(Self::Byte),
            2 => Some(Self::Word),
            4 => Some(Self::Dword),
            _ => None,
        }
    }
}

/// Identifies one timer owned by one device.
///
/// `local` is the device's own index for the timer, so a device owning several
/// (a UART owns an RX FIFO-timeout and a TX shift timer) distinguishes them
/// without the scheduler knowing what they mean. This is the data-carrying
/// replacement for the closed `DeviceTimerOwner` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerKey {
    pub device: DeviceKind,
    pub local: u16,
}

/// Which device model a [`TimerKey`] belongs to.
///
/// Converted devices only. Unconverted devices keep their `DeviceTimerOwner`
/// slot until they are moved across, at which point their variant is added
/// here and removed there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Serial,
    Acpi,
    Cmos,
}

/// Interrupt delivery, as seen by a device.
///
/// Bochs devices call `DEV_pic_raise_irq`/`DEV_pic_lower_irq` synchronously;
/// `set_level` is the level-oriented primitive both reduce to (Bochs
/// pic.cc `set_irq_level`).
pub trait IrqSink {
    fn set_level(&mut self, line: IrqLine, level: bool);

    #[inline]
    fn raise(&mut self, line: IrqLine) {
        self.set_level(line, true);
    }

    #[inline]
    fn lower(&mut self, line: IrqLine) {
        self.set_level(line, false);
    }
}

/// Timer arming, as seen by a device — Bochs `bx_pc_system.activate_timer`
/// and `deactivate_timer`, reachable from inside a port handler.
pub trait TimerService {
    /// Arm `key` as a one-shot, `delay_usec` microseconds of emulated time
    /// from now. Re-arming an already-armed timer replaces its deadline.
    fn arm_oneshot_usec(&mut self, key: TimerKey, delay_usec: u64);

    /// Arm `key` to fire every `period_usec` microseconds until cancelled —
    /// Bochs `activate_timer(..., continuous = 1)`.
    fn arm_periodic_usec(&mut self, key: TimerKey, period_usec: u64);

    /// Disarm `key`. Disarming an already-idle timer is a no-op.
    fn cancel(&mut self, key: TimerKey);
}

/// The capabilities a device may exercise while handling an access.
///
/// Borrowed, never owned: the dispatcher builds one from disjoint machine
/// fields for the duration of a single device call.
pub struct DeviceCtx<'a> {
    /// Emulated time at the access, in scheduler ticks. Bochs devices read
    /// `bx_pc_system.time_ticks()` for the same purpose.
    pub now_ticks: u64,
    pub irq: &'a mut dyn IrqSink,
    pub timers: &'a mut dyn TimerService,
}

/// A device that occupies I/O ports.
///
/// Mirrors the Bochs `read_handler`/`write_handler` pair, minus the
/// `void *this_ptr` trampoline: the device is `self`.
pub trait PioDevice {
    fn pio_read(&mut self, port: u16, len: IoLen, ctx: &mut DeviceCtx<'_>) -> u32;
    fn pio_write(&mut self, port: u16, value: u32, len: IoLen, ctx: &mut DeviceCtx<'_>);
}

/// A device that owns scheduler timers.
pub trait TimedDevice {
    /// One or more expirations of the device's `local` timer have come due.
    ///
    /// `fires` is the coalesced count, matching the existing scheduler
    /// transport, which reports how many periods elapsed rather than calling
    /// once per period.
    fn timer_fired(&mut self, local: u16, fires: u32, ctx: &mut DeviceCtx<'_>);
}
