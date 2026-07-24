//! PC System - Instance-based timer and system control
//!
//! This module provides the PC system infrastructure including:
//! - Timer management for scheduling events (Bochs-exact `tickn`/`countdownEvent` mechanism)
//! - A20 line control for memory addressing
//! - System reset coordination
//!
//! Each `BxPcSystemC` instance is fully independent, allowing multiple
//! emulator instances to run concurrently without conflicts.
//!
//! ## Timer Architecture (matching Bochs pc_system.cc)
//!
//! The timer system uses a countdown mechanism:
//! - `curr_countdown` decrements toward 0 as ticks are consumed by `tickn()`
//! - When it reaches 0, `countdown_event()` fires all expired timers
//! - `countdown_event()` recalculates the next countdown period
//! - `time_ticks()` returns precise current time including partial countdown

use bitflags::bitflags;
use thiserror::Error;

use crate::config::BxPhyAddress;
use crate::cpu::ResetReason;

#[cfg(feature = "std")]
use std::io::{self, Error, ErrorKind, Read, Write};

#[cfg(feature = "std")]
use crate::snapshot::{
    bounds, checked_snapshot_len_add, checked_snapshot_len_mul, SnapshotReader, SnapshotWriteExt,
    SNAPSHOT_SECTION_VERSION,
};

bitflags! {
    /// Timer state flags (replaces individual `in_use`, `active`, `continuous` bools).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TimerFlags: u8 {
        /// Timer slot is allocated
        const IN_USE     = 0x01;
        /// Timer is counting down and will fire
        const ACTIVE     = 0x02;
        /// Timer repeats after firing (vs one-shot)
        const CONTINUOUS = 0x04;
    }
}

/// Errors from PC system timer operations.
///
/// These correspond to `BX_PANIC()` calls in Bochs pc_system.cc.
#[derive(Error, Debug)]
pub enum PcSystemError {
    #[error("timer index {0} out of bounds (max {BX_MAX_TIMERS})")]
    TimerIndexOutOfBounds(usize),
    #[error("timer {0} is not in use")]
    TimerNotInUse(usize),
    #[error("cannot modify null timer (index 0)")]
    NullTimerModification,
    #[error("no free timer slots available (max {BX_MAX_TIMERS})")]
    NoFreeTimerSlots,
    #[error("cannot unregister active timer {0} — deactivate first")]
    TimerStillActive(usize),
    #[error("timer deadline cannot be represented")]
    TimerDeadlineOverflow,
    #[error("timer time conversion cannot be represented")]
    TimeConversionOverflow,
}

/// Maximum length for timer ID strings
const BX_MAX_TIMER_ID_LEN: usize = 32;

/// Fixed-capacity owner slots: null, PIT, keyboard, three CMOS timers,
/// ACPI overflow, four UART FIFO timers, two PCI-IDE channels, slowdown, and
/// four ATA/ATAPI seek timers (one per drive slot — Bochs harddrv.cc
/// registers "HD/CD seek" per configured drive).
pub const BX_FIXED_TIMER_OWNER_COUNT: usize = 21;
/// One LAPIC timer per supported CPU plus every fixed device owner.
pub const BX_MAX_TIMERS: usize =
    crate::params::BX_MAX_SMP_THREADS_SUPPORTED as usize + BX_FIXED_TIMER_OWNER_COUNT;

/// Default null timer interval (in ticks).
/// Bochs pc_system.cc — `const Bit64u NullTimerInterval = 0xffffffff;`
/// This ensures the countdown always fits in a u32 (Bochs uses Bit32u for countdown).
const NULL_TIMER_INTERVAL: u64 = 0xFFFF_FFFF;

/// Minimum allowable timer period in ticks.
/// Bochs pc_system.cc — prevents ridiculously low timer frequencies
/// when IPS is set too low.
const MIN_ALLOWABLE_TIMER_PERIOD: u64 = 1;

/// Identifies which device owns a timer, used for dispatch after firing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerOwner {
    /// The null timer (index 0) — keeps the timing system alive.
    NullTimer,
    /// PCI IDE BM-DMA channel 0.
    PciIdeCh0,
    /// PCI IDE BM-DMA channel 1.
    PciIdeCh1,
    /// Programmable interval timer.
    Pit,
    /// 8042 keyboard serial-transfer timer.
    Keyboard,
    /// CMOS periodic interrupt timer.
    CmosPeriodic,
    /// CMOS one-second clock timer.
    CmosOneSecond,
    /// CMOS update-in-progress completion timer.
    CmosUip,
    /// ACPI PM-timer overflow.
    AcpiPmOverflow,
    /// Receive FIFO timeout for the UART index.
    SerialFifo(usize),
    /// TX shift-register pacing timer for the UART index — Bochs serial.cc
    /// `tx_timer`, one byte emitted per `databyte_usec`.
    SerialTx(usize),
    /// Local APIC timer for the CPU index.
    Lapic(usize),
    /// Bochs host/virtual-time slowdown pacing timer.
    #[cfg(feature = "std")]
    Slowdown,
    /// ATA/ATAPI head-seek timer — Bochs harddrv.cc "HD/CD seek".
    /// The argument is Bochs's `setTimerParam` value `(channel << 1) | device`.
    HdSeek(usize),
    /// HPET comparator one-shot — Bochs hpet.cc "hpet". The argument is the
    /// comparator index (Bochs `setTimerParam(timer_id, i)`).
    Hpet(usize),
}

/// Individual timer structure
#[derive(Debug, Clone, Copy)]
pub struct Timer {
    /// Timer state flags (in_use, active, continuous)
    pub(crate) flags: TimerFlags,
    /// Timer period in ticks
    pub(crate) period: u64,
    /// Absolute tick count when timer should fire
    pub(crate) time_to_fire: u64,
    /// Which device owns this timer (used for dispatch)
    pub(crate) owner: TimerOwner,
    /// Timer identifier string
    pub(crate) id: [u8; BX_MAX_TIMER_ID_LEN],
}

impl Default for Timer {
    fn default() -> Self {
        Self {
            flags: TimerFlags::empty(),
            period: 0,
            time_to_fire: 0,
            owner: TimerOwner::NullTimer,
            id: [0; BX_MAX_TIMER_ID_LEN],
        }
    }
}

#[cfg(feature = "std")]
const TIMER_OWNER_NULL: u8 = 0;
#[cfg(feature = "std")]
const TIMER_OWNER_PCI_IDE_CH0: u8 = 1;
#[cfg(feature = "std")]
const TIMER_OWNER_PCI_IDE_CH1: u8 = 2;
#[cfg(feature = "std")]
const TIMER_OWNER_PIT: u8 = 3;
#[cfg(feature = "std")]
const TIMER_OWNER_KEYBOARD: u8 = 4;
#[cfg(feature = "std")]
const TIMER_OWNER_CMOS_PERIODIC: u8 = 5;
#[cfg(feature = "std")]
const TIMER_OWNER_CMOS_ONE_SECOND: u8 = 6;
#[cfg(feature = "std")]
const TIMER_OWNER_CMOS_UIP: u8 = 7;
#[cfg(feature = "std")]
const TIMER_OWNER_ACPI_PM_OVERFLOW: u8 = 8;
#[cfg(feature = "std")]
const TIMER_OWNER_SERIAL_FIFO: u8 = 9;
#[cfg(feature = "std")]
const TIMER_OWNER_LAPIC: u8 = 10;
#[cfg(feature = "std")]
const TIMER_OWNER_SLOWDOWN: u8 = 11;
#[cfg(feature = "std")]
const TIMER_OWNER_HD_SEEK: u8 = 12;
#[cfg(feature = "std")]
const TIMER_OWNER_HPET: u8 = 13;
#[cfg(feature = "std")]
const TIMER_OWNER_SERIAL_TX: u8 = 14;

#[cfg(feature = "std")]
const TIMER_OWNER_WIRE_LEN: u64 = 5;
#[cfg(feature = "std")]
const TIMER_WIRE_FIXED_LEN: u64 = 17;
#[cfg(feature = "std")]
const FIRED_OWNER_WIRE_LEN: u64 = TIMER_OWNER_WIRE_LEN + 4;

#[cfg(feature = "std")]
fn snapshot_invalid_data(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

#[cfg(feature = "std")]
fn snapshot_usize_to_u32(value: usize) -> io::Result<u32> {
    u32::try_from(value).map_err(|_| snapshot_invalid_data("snapshot value does not fit in u32"))
}

#[cfg(feature = "std")]
fn snapshot_usize_to_u64(value: usize) -> io::Result<u64> {
    u64::try_from(value).map_err(|_| snapshot_invalid_data("snapshot value does not fit in u64"))
}

#[cfg(feature = "std")]
fn max_lapic_timer_owners() -> io::Result<usize> {
    usize::try_from(crate::params::BX_MAX_SMP_THREADS_SUPPORTED)
        .map_err(|_| snapshot_invalid_data("LAPIC timer capacity does not fit in usize"))
}

#[cfg(feature = "std")]
fn timer_owner_wire_parts(owner: TimerOwner) -> io::Result<(u8, u32)> {
    let fixed = |tag| Ok((tag, 0));

    match owner {
        TimerOwner::NullTimer => fixed(TIMER_OWNER_NULL),
        TimerOwner::PciIdeCh0 => fixed(TIMER_OWNER_PCI_IDE_CH0),
        TimerOwner::PciIdeCh1 => fixed(TIMER_OWNER_PCI_IDE_CH1),
        TimerOwner::Pit => fixed(TIMER_OWNER_PIT),
        TimerOwner::Keyboard => fixed(TIMER_OWNER_KEYBOARD),
        TimerOwner::CmosPeriodic => fixed(TIMER_OWNER_CMOS_PERIODIC),
        TimerOwner::CmosOneSecond => fixed(TIMER_OWNER_CMOS_ONE_SECOND),
        TimerOwner::CmosUip => fixed(TIMER_OWNER_CMOS_UIP),
        TimerOwner::AcpiPmOverflow => fixed(TIMER_OWNER_ACPI_PM_OVERFLOW),
        TimerOwner::SerialFifo(port) => {
            if port >= crate::iodev::BX_FIXED_SERIAL_TIMER_OWNERS {
                return Err(snapshot_invalid_data("serial timer owner is out of range"));
            }
            Ok((TIMER_OWNER_SERIAL_FIFO, snapshot_usize_to_u32(port)?))
        }
        TimerOwner::SerialTx(port) => {
            if port >= crate::iodev::BX_FIXED_SERIAL_TIMER_OWNERS {
                return Err(snapshot_invalid_data("serial timer owner is out of range"));
            }
            Ok((TIMER_OWNER_SERIAL_TX, snapshot_usize_to_u32(port)?))
        }
        TimerOwner::Lapic(cpu) => {
            if cpu >= max_lapic_timer_owners()? {
                return Err(snapshot_invalid_data("LAPIC timer owner is out of range"));
            }
            Ok((TIMER_OWNER_LAPIC, snapshot_usize_to_u32(cpu)?))
        }
        TimerOwner::Slowdown => fixed(TIMER_OWNER_SLOWDOWN),
        TimerOwner::HdSeek(param) => {
            if param >= 4 {
                return Err(snapshot_invalid_data("HD seek timer owner is out of range"));
            }
            Ok((TIMER_OWNER_HD_SEEK, snapshot_usize_to_u32(param)?))
        }
        TimerOwner::Hpet(index) => {
            if index >= crate::iodev::hpet::HPET_NUM_TIMERS {
                return Err(snapshot_invalid_data("HPET timer owner is out of range"));
            }
            Ok((TIMER_OWNER_HPET, snapshot_usize_to_u32(index)?))
        }
    }
}

#[cfg(feature = "std")]
fn write_timer_owner<W: Write>(writer: &mut W, owner: TimerOwner) -> io::Result<()> {
    let (tag, argument) = timer_owner_wire_parts(owner)?;
    writer.write_u8(tag)?;
    writer.write_u32(argument)
}

#[cfg(feature = "std")]
fn read_timer_owner<R: Read>(reader: &mut SnapshotReader<R>) -> io::Result<TimerOwner> {
    let tag = reader.read_u8()?;
    let argument = reader.read_u32()?;

    let fixed = |owner| {
        if argument == 0 {
            Ok(owner)
        } else {
            Err(snapshot_invalid_data("fixed timer owner has an argument"))
        }
    };

    match tag {
        TIMER_OWNER_NULL => fixed(TimerOwner::NullTimer),
        TIMER_OWNER_PCI_IDE_CH0 => fixed(TimerOwner::PciIdeCh0),
        TIMER_OWNER_PCI_IDE_CH1 => fixed(TimerOwner::PciIdeCh1),
        TIMER_OWNER_PIT => fixed(TimerOwner::Pit),
        TIMER_OWNER_KEYBOARD => fixed(TimerOwner::Keyboard),
        TIMER_OWNER_CMOS_PERIODIC => fixed(TimerOwner::CmosPeriodic),
        TIMER_OWNER_CMOS_ONE_SECOND => fixed(TimerOwner::CmosOneSecond),
        TIMER_OWNER_CMOS_UIP => fixed(TimerOwner::CmosUip),
        TIMER_OWNER_ACPI_PM_OVERFLOW => fixed(TimerOwner::AcpiPmOverflow),
        TIMER_OWNER_SERIAL_FIFO => {
            let port = usize::try_from(argument)
                .map_err(|_| snapshot_invalid_data("serial timer owner argument is too large"))?;
            if port >= crate::iodev::BX_FIXED_SERIAL_TIMER_OWNERS {
                return Err(snapshot_invalid_data("serial timer owner is out of range"));
            }
            Ok(TimerOwner::SerialFifo(port))
        }
        TIMER_OWNER_SERIAL_TX => {
            let port = usize::try_from(argument)
                .map_err(|_| snapshot_invalid_data("serial timer owner argument is too large"))?;
            if port >= crate::iodev::BX_FIXED_SERIAL_TIMER_OWNERS {
                return Err(snapshot_invalid_data("serial timer owner is out of range"));
            }
            Ok(TimerOwner::SerialTx(port))
        }
        TIMER_OWNER_LAPIC => {
            let cpu = usize::try_from(argument)
                .map_err(|_| snapshot_invalid_data("LAPIC timer owner argument is too large"))?;
            if cpu >= max_lapic_timer_owners()? {
                return Err(snapshot_invalid_data("LAPIC timer owner is out of range"));
            }
            Ok(TimerOwner::Lapic(cpu))
        }
        TIMER_OWNER_SLOWDOWN => fixed(TimerOwner::Slowdown),
        TIMER_OWNER_HD_SEEK => {
            let param = usize::try_from(argument)
                .map_err(|_| snapshot_invalid_data("HD seek timer owner argument is too large"))?;
            if param >= 4 {
                return Err(snapshot_invalid_data("HD seek timer owner is out of range"));
            }
            Ok(TimerOwner::HdSeek(param))
        }
        TIMER_OWNER_HPET => {
            let index = usize::try_from(argument)
                .map_err(|_| snapshot_invalid_data("HPET timer owner argument is too large"))?;
            if index >= crate::iodev::hpet::HPET_NUM_TIMERS {
                return Err(snapshot_invalid_data("HPET timer owner is out of range"));
            }
            Ok(TimerOwner::Hpet(index))
        }
        _ => Err(snapshot_invalid_data("unknown timer owner tag")),
    }
}

#[cfg(feature = "std")]
fn validate_timer_id(id: &[u8; BX_MAX_TIMER_ID_LEN]) -> io::Result<()> {
    let mut terminated = false;
    for &byte in id {
        if byte == 0 {
            terminated = true;
        } else if terminated {
            return Err(snapshot_invalid_data(
                "timer identifier has nonzero bytes after its terminator",
            ));
        }
    }
    if !terminated {
        return Err(snapshot_invalid_data("timer identifier is not terminated"));
    }
    Ok(())
}

#[cfg(feature = "std")]
fn timer_owner_is_registered(timers: &[Timer; BX_MAX_TIMERS], owner: TimerOwner) -> bool {
    timers
        .iter()
        .any(|timer| timer.flags.contains(TimerFlags::IN_USE) && timer.owner == owner)
}

#[cfg(feature = "std")]
#[allow(clippy::too_many_arguments)]
fn validate_snapshot_state_fields(
    ips: u64,
    curr_countdown: u32,
    curr_countdown_period: u32,
    ticks_total: u64,
    timers: &[Timer; BX_MAX_TIMERS],
    num_timers: usize,
    triggered_timer: usize,
    fired_owners: &[TimerOwner; BX_MAX_TIMERS],
    fired_owner_counts: &[u32; BX_MAX_TIMERS],
    num_fired: usize,
    enable_a20: bool,
    a20_mask: BxPhyAddress,
) -> io::Result<()> {
    if ips == 0 {
        return Err(snapshot_invalid_data("configured IPS is zero"));
    }
    if num_timers == 0
        || num_timers > BX_MAX_TIMERS
        || num_timers > bounds::MAX_SNAPSHOT_COUNT
    {
        return Err(snapshot_invalid_data("timer count is out of range"));
    }
    if num_fired > BX_MAX_TIMERS || num_fired > bounds::MAX_SNAPSHOT_QUEUE_LEN {
        return Err(snapshot_invalid_data("fired timer queue count is out of range"));
    }
    if curr_countdown == 0
        || curr_countdown_period == 0
        || curr_countdown > curr_countdown_period
    {
        return Err(snapshot_invalid_data("countdown state is inconsistent"));
    }

    let elapsed = u64::from(curr_countdown_period - curr_countdown);
    let now = ticks_total
        .checked_add(elapsed)
        .ok_or_else(|| snapshot_invalid_data("timer epoch overflows"))?;
    let next_countdown_event = ticks_total
        .checked_add(u64::from(curr_countdown_period))
        .ok_or_else(|| snapshot_invalid_data("next countdown event overflows"))?;

    let expected_a20_mask = if enable_a20 {
        0xFFFF_FFFF_FFFF_FFFFu64
    } else {
        0xFFFF_FFFF_FFEF_FFFFu64
    };
    if a20_mask != expected_a20_mask {
        return Err(snapshot_invalid_data("A20 state and mask disagree"));
    }

    let null_timer_flags = TimerFlags::IN_USE | TimerFlags::ACTIVE | TimerFlags::CONTINUOUS;
    for (index, timer) in timers.iter().enumerate() {
        if index >= num_timers {
            if !timer.flags.is_empty()
                || timer.owner != TimerOwner::NullTimer
                || timer.id != [0; BX_MAX_TIMER_ID_LEN]
            {
                return Err(snapshot_invalid_data(
                    "timer outside registered range carries live state",
                ));
            }
            continue;
        }

        if index == 0 {
            if timer.flags != null_timer_flags
                || timer.period != NULL_TIMER_INTERVAL
                || timer.owner != TimerOwner::NullTimer
                || timer.id != [0; BX_MAX_TIMER_ID_LEN]
            {
                return Err(snapshot_invalid_data("null timer state is inconsistent"));
            }
        }

        let in_use = timer.flags.contains(TimerFlags::IN_USE);
        if !in_use {
            if !timer.flags.is_empty()
                || timer.owner != TimerOwner::NullTimer
                || timer.id != [0; BX_MAX_TIMER_ID_LEN]
            {
                return Err(snapshot_invalid_data("unused timer has live state"));
            }
            continue;
        }

        validate_timer_id(&timer.id)?;
        timer_owner_wire_parts(timer.owner)?;
        if index != 0 && timer.owner == TimerOwner::NullTimer {
            return Err(snapshot_invalid_data("non-null timer uses null owner"));
        }
        if index != 0
            && timers
                .iter()
                .take(index)
                .any(|previous| {
                    previous.flags.contains(TimerFlags::IN_USE) && previous.owner == timer.owner
                })
        {
            return Err(snapshot_invalid_data("timer owner is registered more than once"));
        }
        if timer.period < MIN_ALLOWABLE_TIMER_PERIOD {
            return Err(snapshot_invalid_data("timer period is zero"));
        }
        if timer.flags.contains(TimerFlags::ACTIVE) {
            if timer.time_to_fire <= now {
                return Err(snapshot_invalid_data("active timer deadline is not in the future"));
            }
            // The countdown never overshoots the earliest active deadline: it
            // is only ever shortened toward a timer, so no active timer may
            // sit strictly before it.
            if timer.time_to_fire < next_countdown_event {
                return Err(snapshot_invalid_data(
                    "active timer precedes the next countdown event",
                ));
            }
        }
    }

    // The countdown may point exactly at the earliest active timer, or before
    // it: `deactivate_timer` clears a timer's ACTIVE flag without re-narrowing
    // the countdown (matching Bochs pc_system.cc), so after the timer that set
    // the current countdown is deactivated, the countdown legitimately points
    // at a since-departed deadline until the next `countdown_event` recomputes
    // it. Requiring an active timer exactly at `next_countdown_event` would
    // reject that valid, reachable running state — as a real Windows boot hits.

    let triggered = timers
        .get(triggered_timer)
        .ok_or_else(|| snapshot_invalid_data("triggered timer is out of range"))?;
    if triggered_timer >= num_timers || !triggered.flags.contains(TimerFlags::IN_USE) {
        return Err(snapshot_invalid_data("triggered timer is not registered"));
    }

    for (index, (&owner, &count)) in fired_owners
        .iter()
        .zip(fired_owner_counts.iter())
        .take(num_fired)
        .enumerate()
    {
        timer_owner_wire_parts(owner)?;
        if owner == TimerOwner::NullTimer {
            return Err(snapshot_invalid_data("null timer cannot be pending for dispatch"));
        }
        if count == 0 {
            return Err(snapshot_invalid_data("fired timer count is zero"));
        }
        if fired_owners.iter().take(index).any(|previous| *previous == owner) {
            return Err(snapshot_invalid_data("fired timer owner is duplicated"));
        }
        if !timer_owner_is_registered(timers, owner) {
            return Err(snapshot_invalid_data(
                "fired timer owner does not map to a registered timer",
            ));
        }
    }

    Ok(())
}

/// PC System controller - manages timers, A20 line, and system-level operations
///
/// This struct is fully instance-based with no global state, allowing multiple
/// independent emulator instances to run concurrently.
#[derive(Debug)]
pub struct BxPcSystemC {
    /// Array of timers
    pub(crate) timers: [Timer; BX_MAX_TIMERS],
    /// Number of registered timers
    num_timers: usize,
    /// Index of most recently triggered timer
    triggered_timer: usize,
    /// Current countdown value (Bochs: Bit32u currCountdown)
    curr_countdown: u32,
    /// Period for current countdown (Bochs: Bit32u currCountdownPeriod)
    curr_countdown_period: u32,
    /// Total ticks since emulator started (Bochs: Bit64u ticksTotal)
    ticks_total: u64,
    /// Last time in microseconds
    last_time_usec: u64,
    /// Microseconds since last sync
    usec_since_last: u64,
    /// A20 address mask (controls A20 line gating)
    pub(crate) a20_mask: BxPhyAddress,
    /// Whether A20 line is enabled
    pub(crate) enable_a20: bool,
    /// Instructions per second (raw count, e.g. 300_000_000)
    ips: u64,
    /// Hardware Request (DMA)
    hrq: bool,
    /// HRQ pending flag — set by set_hrq(true), checked by emulator loop.
    /// Bochs pc_system.cc: set_HRQ sets HRQ and signals async_event.
    pub(crate) hrq_pending: bool,
    /// Flag: set_hrq(true) wants async_event=1 on the CPU.
    /// The emulator reads and clears this.
    pub(crate) async_event_pending: bool,
    /// Flag: raise_intr() wants BX_EVENT_PENDING_INTR set on the CPU.
    /// The emulator reads and clears this.
    pub(crate) intr_raised: bool,
    /// Flag: clear_intr() wants BX_EVENT_PENDING_INTR cleared on the CPU.
    /// The emulator reads and clears this.
    pub(crate) intr_cleared: bool,
    /// Request to terminate emulation
    pub(crate) kill_bochs_request: bool,
    /// Buffer of timer owners whose timers fired during the last tickn/tick1.
    /// Drained by the emulator via `take_fired_timers()`.
    fired_owners: [TimerOwner; BX_MAX_TIMERS],
    /// Fire counts for each distinct buffered owner in `fired_owners`.
    fired_owner_counts: [u32; BX_MAX_TIMERS],
    /// Number of distinct entries in `fired_owners`.
    num_fired: usize,
}

impl Default for BxPcSystemC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxPcSystemC {
    /// Create a new PC system instance with default settings
    pub fn new() -> Self {
        // Create default timer array
        let timers: [Timer; BX_MAX_TIMERS] = core::array::from_fn(|_| Timer::default());

        let mut sys = Self {
            timers,
            num_timers: 0,
            triggered_timer: 0,
            curr_countdown: NULL_TIMER_INTERVAL as u32,
            curr_countdown_period: NULL_TIMER_INTERVAL as u32,
            ticks_total: 0,
            last_time_usec: 0,
            usec_since_last: 0,
            // A20 line starts DISABLED at boot (bit 20 masked off)
            // This causes addresses like 0xFFFFFFF0 to wrap to 0x000FFFF0
            a20_mask: 0xFFFF_FFFF_FFEF_FFFFu64,
            enable_a20: false,
            ips: 1_000_000,
            hrq: false,
            hrq_pending: false,
            async_event_pending: false,
            intr_raised: false,
            intr_cleared: false,
            kill_bochs_request: false,
            fired_owners: [TimerOwner::NullTimer; BX_MAX_TIMERS],
            fired_owner_counts: [0; BX_MAX_TIMERS],
            num_fired: 0,
        };

        // Register the null timer as timer 0
        sys.timers[0].flags = TimerFlags::IN_USE | TimerFlags::ACTIVE | TimerFlags::CONTINUOUS;
        sys.timers[0].period = NULL_TIMER_INTERVAL;
        sys.timers[0].time_to_fire = NULL_TIMER_INTERVAL;
        sys.timers[0].owner = TimerOwner::NullTimer;
        sys.num_timers = 1;

        sys
    }

    /// Initialize the PC system with the given instructions-per-second value
    ///
    /// This sets up timer infrastructure and IPS-based timing.
    /// Corresponds to `bx_pc_system_c::initialize()` in Bochs (pc_system.cc).
    pub fn initialize(&mut self, ips: u32) {
        self.ticks_total = 0;
        self.timers[0].time_to_fire = NULL_TIMER_INTERVAL;
        self.curr_countdown = NULL_TIMER_INTERVAL as u32;
        self.curr_countdown_period = NULL_TIMER_INTERVAL as u32;
        self.last_time_usec = 0;
        self.usec_since_last = 0;
        self.triggered_timer = 0;
        self.hrq = false;
        self.hrq_pending = false;
        self.kill_bochs_request = false;

        // Convert IPS to millions for timing calculations
        self.ips = u64::from(ips.max(1));

        tracing::trace!("PC system initialized with ips = {}", ips);
    }

    /// Configured instructions-per-second rate (ticks per emulated second).
    #[inline]
    pub fn ips(&self) -> u64 {
        self.ips
    }

    // ========================================================================
    // Timer tick mechanism — matches Bochs pc_system.h
    // ========================================================================

    /// Advance virtual time by `n` ticks, firing any expired timers.
    /// This is the core timing primitive — matches Bochs pc_system.h.
    ///
    /// Replaces the old `tick()` + `check_timers()` pair with exact Bochs logic:
    /// decrements `curr_countdown`, triggers `countdown_event()` at 0.
    #[inline]
    pub fn tickn(&mut self, n: u32) {
        let mut remaining = n;
        while remaining >= self.curr_countdown {
            remaining -= self.curr_countdown;
            self.curr_countdown = 0;
            self.countdown_event();
            // curr_countdown is reset by countdown_event()
        }
        // remaining < curr_countdown — just decrement
        self.curr_countdown -= remaining;
    }

    /// Advance by exactly 1 tick (hot path optimization).
    /// Matches Bochs pc_system.h.
    #[inline]
    pub fn tick1(&mut self) {
        self.curr_countdown -= 1;
        if self.curr_countdown == 0 {
            self.countdown_event();
        }
    }

    /// Handle countdown reaching zero. Checks all timers, fires expired ones,
    /// and recalculates next countdown period.
    /// Matches Bochs pc_system.cc exactly.
    #[inline]
    fn countdown_event(&mut self) {
        let mut first = self.num_timers;
        let mut last = 0usize;
        let mut min_time_to_fire: u64 = u64::MAX;
        let mut triggered = [false; BX_MAX_TIMERS];

        // Step 1: Advance total ticks by the countdown period
        // Bochs pc_system.cc
        self.ticks_total += self.curr_countdown_period as u64;

        // Step 2: Scan all timers for fires and find next event
        // Bochs pc_system.cc uses `==` (ticksTotal == timeToFire).
        // We use `>=` to catch overdue timers when countdown period overshoots
        // the timer period. This was the root cause of LAPIC timer interrupts
        // never firing during HLT (session 53 fix).
        for (i, triggered_flag) in triggered.iter_mut().enumerate().take(self.num_timers) {
            *triggered_flag = false;
            if self.timers[i].flags.contains(TimerFlags::ACTIVE) {
                if self.ticks_total >= self.timers[i].time_to_fire {
                    // Timer is ready to fire (may be overdue)
                    *triggered_flag = true;
                    if !self.timers[i].flags.contains(TimerFlags::CONTINUOUS) {
                        // One-shot: deactivate
                        self.timers[i].flags.remove(TimerFlags::ACTIVE);
                    } else {
                        // Continuous: advance time_to_fire past ticks_total
                        while self.timers[i].time_to_fire <= self.ticks_total {
                            self.timers[i].time_to_fire += self.timers[i].period;
                        }
                        if self.timers[i].time_to_fire < min_time_to_fire {
                            min_time_to_fire = self.timers[i].time_to_fire;
                        }
                    }
                    if i < first {
                        first = i;
                    }
                    last = i;
                } else {
                    // Not ready yet — track for next countdown calculation
                    if self.timers[i].time_to_fire < min_time_to_fire {
                        min_time_to_fire = self.timers[i].time_to_fire;
                    }
                }
            }
        }
        // Step 3: Calculate the next countdown period before recording fires.
        // A timer farther than the u32 countdown horizon is revisited at the
        // horizon; an active timer must never leave a zero countdown behind.
        // Bochs pc_system.cc performs the same countdown narrowing.
        let next_period = min_time_to_fire
            .checked_sub(self.ticks_total)
            .unwrap_or(MIN_ALLOWABLE_TIMER_PERIOD)
            .clamp(MIN_ALLOWABLE_TIMER_PERIOD, NULL_TIMER_INTERVAL) as u32;
        self.curr_countdown = next_period;
        self.curr_countdown_period = next_period;

        // Step 4: Record all triggered timers for dispatch by the emulator.
        // Bochs pc_system.cc called handlers here; we defer to the
        // emulator so pc_system doesn't need device pointers.
        if first <= last {
            for (offset, &triggered_flag) in triggered[first..=last].iter().enumerate() {
                let i = first + offset;
                if triggered_flag {
                    self.triggered_timer = i;
                    let owner = self.timers[i].owner;
                    self.record_fired_owner(owner);
                    self.triggered_timer = 0;
                }
            }
        }
    }

    fn record_fired_owner(&mut self, owner: TimerOwner) {
        if owner == TimerOwner::NullTimer {
            return;
        }

        for entry in 0..self.num_fired {
            if self.fired_owners[entry] == owner {
                self.fired_owner_counts[entry] = self.fired_owner_counts[entry].saturating_add(1);
                return;
            }
        }

        if self.num_fired < BX_MAX_TIMERS {
            self.fired_owners[self.num_fired] = owner;
            self.fired_owner_counts[self.num_fired] = 1;
            self.num_fired += 1;
        } else {
            tracing::error!("timer fire owner buffer full; dropping owner {:?}", owner);
        }
    }

    // ========================================================================
    // A20 line control
    // ========================================================================

    /// Enable or disable the A20 address line
    ///
    /// When A20 is disabled, address bit 20 is masked off, limiting memory
    /// access to the first 1MB (for 8086 compatibility).
    pub fn set_enable_a20(&mut self, value: bool) {
        let old_enable_a20 = self.enable_a20;

        if value {
            self.enable_a20 = true;
            // Full 64-bit address space when A20 is enabled
            self.a20_mask = 0xFFFF_FFFF_FFFF_FFFFu64;
        } else {
            self.enable_a20 = false;
            // Mask off A20 line (bit 20)
            self.a20_mask = 0xFFFF_FFFF_FFEF_FFFFu64;
        }

        tracing::trace!("A20: set() = {}", self.enable_a20);

        // If there has been a transition, TLB flush may be needed
        if old_enable_a20 != self.enable_a20 {
            // Note: TLB flush is handled by the caller (CPU)
            tracing::trace!("A20 line changed, memory mapping affected");
        }
    }

    /// Get the current A20 line state
    pub fn get_enable_a20(&self) -> bool {
        self.enable_a20
    }

    /// Get the A20 address mask
    ///
    /// Apply this mask to physical addresses to implement A20 gating.
    #[inline]
    pub fn a20_mask(&self) -> BxPhyAddress {
        self.a20_mask
    }

    /// Apply A20 masking to an address
    #[inline]
    pub fn a20_addr(&self, addr: BxPhyAddress) -> BxPhyAddress {
        addr & self.a20_mask
    }

    // ========================================================================
    // Time queries — matches Bochs pc_system.h and pc_system.cc
    // ========================================================================

    /// Get precise current time in ticks, including partial countdown.
    /// Matches Bochs pc_system.h:
    /// `ticksTotal + (currCountdownPeriod - currCountdown)`
    #[inline]
    pub fn time_ticks(&self) -> u64 {
        self.ticks_total + (self.curr_countdown_period - self.curr_countdown) as u64
    }

    /// Convert an absolute tick epoch to microseconds without intermediate
    /// u64 multiplication overflow.
    pub fn time_usec_at_ticks(&self, ticks: u64) -> Result<u64, PcSystemError> {
        let usec = (u128::from(ticks) * 1_000_000u128) / u128::from(self.ips);
        u64::try_from(usec).map_err(|_| PcSystemError::TimeConversionOverflow)
    }

    /// Convert microseconds to ticks without intermediate u64 multiplication
    /// overflow.
    pub fn usec_to_ticks(&self, useconds: u64) -> Result<u64, PcSystemError> {
        let ticks = (u128::from(useconds) * u128::from(self.ips)) / 1_000_000u128;
        u64::try_from(ticks).map_err(|_| PcSystemError::TimeConversionOverflow)
    }

    /// Convert the current tick epoch to microseconds using IPS setting.
    /// The legacy non-fallible view saturates rather than wrapping; callers
    /// that need error reporting use `time_usec_at_ticks`.
    pub fn time_usec(&self) -> u64 {
        self.time_usec_at_ticks(self.time_ticks())
            .unwrap_or(u64::MAX)
    }

    /// Convert ticks to nanoseconds using IPS setting.
    /// Matches Bochs pc_system.cc for representable values.
    pub fn time_nsec(&self) -> u64 {
        let nsec = (u128::from(self.time_ticks()) * 1_000_000_000u128) / u128::from(self.ips);
        u64::try_from(nsec).unwrap_or(u64::MAX)
    }

    // ========================================================================
    // DMA and system control
    // ========================================================================

    /// Set the Hardware Request (DMA) line.
    /// Matches Bochs pc_system.cc: sets HRQ flag and signals async_event.
    pub fn set_hrq(&mut self, value: bool) {
        self.hrq = value;
        if value {
            self.hrq_pending = true;
            // Bochs pc_system.cc: BX_CPU(0)->async_event = 1
            self.async_event_pending = true;
        }
    }

    /// Get the Hardware Request (DMA) line state
    pub fn get_hrq(&self) -> bool {
        self.hrq
    }

    /// Signal external interrupt to bootstrap CPU (Bochs pc_system.cc).
    ///
    /// Sets `intr_raised` so the emulator applies BX_EVENT_PENDING_INTR
    /// and async_event=1 to the CPU.
    pub fn raise_intr(&mut self) {
        self.intr_raised = true;
    }

    /// Clear external interrupt signal (Bochs pc_system.cc).
    ///
    /// Sets `intr_cleared` so the emulator clears BX_EVENT_PENDING_INTR
    /// from the CPU.
    pub fn clear_intr(&mut self) {
        self.intr_cleared = true;
    }

    /// Perform a system reset
    ///
    /// For hardware reset: enables A20, resets CPU and all devices
    /// For software reset: just resets CPU
    pub fn reset(&mut self, reset_type: ResetReason) {
        tracing::debug!("BxPcSystemC::reset({:?}) called", reset_type);

        // A20 line is ENABLED at hardware reset on 386+ CPUs
        // (Only 286 systems start with A20 disabled)
        self.set_enable_a20(true);

        // Clear DMA pending flag
        self.hrq_pending = false;

        // Discard fired-but-undispatched timer callbacks. Bochs pc_system.cc
        // dispatches timer handlers synchronously inside tickn, so a queued
        // pre-reset callback can never be carried across Reset into the
        // fresh machine.
        self.num_fired = 0;
        self.fired_owner_counts = [0; BX_MAX_TIMERS];
        self.fired_owners = [TimerOwner::NullTimer; BX_MAX_TIMERS];
    }

    /// Register state for save/restore functionality.
    /// Bochs uses parameter tree nodes. Our snapshot uses snapshot.rs instead.
    pub fn register_state(&self) {
        tracing::trace!("PC system state registered");
    }

    /// Start all registered timers. No-op — matches Bochs pc_system.cc.
    /// Timer time_to_fire is set correctly during register_timer/activate_timer.
    pub fn start_timers(&mut self) {
        tracing::trace!("start_timers: no-op (timers started during registration)");
    }

    // ========================================================================
    // Timer registration and management
    // ========================================================================

    /// Validate a timer index (must be in range, in use, and not null timer).
    fn validate_timer_index(&self, timer_index: usize) -> Result<(), PcSystemError> {
        if timer_index >= BX_MAX_TIMERS {
            return Err(PcSystemError::TimerIndexOutOfBounds(timer_index));
        }
        if timer_index == 0 {
            return Err(PcSystemError::NullTimerModification);
        }
        if !self.timers[timer_index].flags.contains(TimerFlags::IN_USE) {
            return Err(PcSystemError::TimerNotInUse(timer_index));
        }
        Ok(())
    }
    #[inline]
    fn deadline_from_now(&self, ticks: u64) -> Result<u64, PcSystemError> {
        self.time_ticks()
            .checked_add(ticks)
            .ok_or(PcSystemError::TimerDeadlineOverflow)
    }

    /// Move the countdown only when `deadline_ticks` is earlier than the
    /// already scheduled countdown event. This preserves the current epoch.
    fn shorten_countdown_to(&mut self, deadline_ticks: u64) {
        let now = self.time_ticks();
        let Some(ticks_until_fire) = deadline_ticks.checked_sub(now) else {
            return;
        };
        if ticks_until_fire == 0 || ticks_until_fire >= u64::from(self.curr_countdown) {
            return;
        }

        let ticks_until_fire = ticks_until_fire as u32;
        let elapsed = self.curr_countdown_period - self.curr_countdown;
        self.curr_countdown = ticks_until_fire;
        self.curr_countdown_period = elapsed + ticks_until_fire;
    }

    /// Register a new timer with period in ticks.
    ///
    /// Corresponds to `bx_pc_system_c::register_timer_ticks()` in Bochs (pc_system.cc).
    /// Returns the timer index on success, or `PcSystemError::NoFreeTimerSlots` if full.
    pub fn register_timer(
        &mut self,
        owner: TimerOwner,
        period: u64,
        continuous: bool,
        active: bool,
        id: &str,
    ) -> Result<usize, PcSystemError> {
        let period = period.max(MIN_ALLOWABLE_TIMER_PERIOD);

        for i in 1..BX_MAX_TIMERS {
            if self.timers[i].flags.contains(TimerFlags::IN_USE) {
                continue;
            }

            let deadline_ticks = self.deadline_from_now(period)?;
            self.timers[i].flags = TimerFlags::IN_USE;
            self.timers[i]
                .flags
                .set(TimerFlags::CONTINUOUS, continuous);
            self.timers[i].period = period;
            self.timers[i].time_to_fire = deadline_ticks;
            self.timers[i].owner = owner;
            self.timers[i].id = [0; BX_MAX_TIMER_ID_LEN];

            let id_bytes = id.as_bytes();
            let copy_len = id_bytes.len().min(BX_MAX_TIMER_ID_LEN - 1);
            self.timers[i].id[..copy_len].copy_from_slice(&id_bytes[..copy_len]);

            if i >= self.num_timers {
                self.num_timers = i + 1;
            }
            if active {
                self.activate_timer_at_ticks(i, deadline_ticks, continuous)?;
            }

            tracing::trace!("Registered timer {} with id '{}'", i, id);
            return Ok(i);
        }

        Err(PcSystemError::NoFreeTimerSlots)
    }

    /// Register a new timer with period in microseconds.
    ///
    /// Corresponds to `bx_pc_system_c::register_timer()` in Bochs (pc_system.cc).
    /// Converts microseconds to ticks using IPS setting, then delegates to register_timer.
    pub fn register_timer_usec(
        &mut self,
        owner: TimerOwner,
        useconds: u32,
        continuous: bool,
        active: bool,
        id: &str,
    ) -> Result<usize, PcSystemError> {
        self.register_timer(
            owner,
            self.usec_to_ticks(u64::from(useconds))?,
            continuous,
            active,
            id,
        )
    }

    /// Activate a timer at an absolute tick epoch.
    ///
    /// Deadlines that are already due are made observable on the next tick,
    /// never as a zero-countdown re-entry. Absolute activation preserves the
    /// timer's registered repeat period.
    pub fn activate_timer_at_ticks(
        &mut self,
        timer_index: usize,
        deadline_ticks: u64,
        continuous: bool,
    ) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        let now = self.time_ticks();
        let deadline_ticks = if deadline_ticks <= now {
            now.checked_add(MIN_ALLOWABLE_TIMER_PERIOD)
                .ok_or(PcSystemError::TimerDeadlineOverflow)?
        } else {
            deadline_ticks
        };

        self.timers[timer_index].time_to_fire = deadline_ticks;
        self.timers[timer_index].flags.insert(TimerFlags::ACTIVE);
        self.timers[timer_index]
            .flags
            .set(TimerFlags::CONTINUOUS, continuous);
        self.shorten_countdown_to(deadline_ticks);
        Ok(())
    }

    /// Activate at an absolute first deadline while replacing the repeat period.
    ///
    /// Deferred device requests carry both values because the PC-system clock
    /// can still reflect the start of the CPU batch in which the request arose.
    pub(crate) fn activate_timer_at_ticks_with_period(
        &mut self,
        timer_index: usize,
        deadline_ticks: u64,
        period: u64,
        continuous: bool,
    ) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        self.timers[timer_index].period = period.max(MIN_ALLOWABLE_TIMER_PERIOD);
        self.activate_timer_at_ticks(timer_index, deadline_ticks, continuous)
    }

    /// Activate a timer with a period relative to the current tick epoch.
    ///
    /// Corresponds to `bx_pc_system_c::activate_timer_ticks()` in Bochs
    /// (pc_system.cc).
    pub fn activate_timer(
        &mut self,
        timer_index: usize,
        period: u64,
        continuous: bool,
    ) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        let period = period.max(MIN_ALLOWABLE_TIMER_PERIOD);
        let deadline_ticks = self.deadline_from_now(period)?;
        self.timers[timer_index].period = period;
        self.activate_timer_at_ticks(timer_index, deadline_ticks, continuous)
    }

    /// Activate a timer with period in microseconds.
    ///
    /// Corresponds to `bx_pc_system_c::activate_timer()` in Bochs (pc_system.cc).
    /// If `useconds == 0`, reuses the timer's existing period.
    pub fn activate_timer_usec(
        &mut self,
        timer_index: usize,
        useconds: u32,
        continuous: bool,
    ) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        let ticks = if useconds == 0 {
            self.timers[timer_index].period
        } else {
            self.usec_to_ticks(u64::from(useconds))?
                .max(MIN_ALLOWABLE_TIMER_PERIOD)
        };
        self.activate_timer(timer_index, ticks, continuous)
    }

    /// Activate a timer with period in nanoseconds.
    ///
    /// Corresponds to `bx_pc_system_c::activate_timer_nsec()` in Bochs (pc_system.cc).
    /// If `nseconds == 0`, reuses the timer's existing period.
    pub fn activate_timer_nsec(
        &mut self,
        timer_index: usize,
        nseconds: u64,
        continuous: bool,
    ) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        let ticks = if nseconds == 0 {
            self.timers[timer_index].period
        } else {
            let ticks = (u128::from(nseconds) * u128::from(self.ips)) / 1_000_000_000u128;
            u64::try_from(ticks)
                .map_err(|_| PcSystemError::TimeConversionOverflow)?
                .max(MIN_ALLOWABLE_TIMER_PERIOD)
        };
        self.activate_timer(timer_index, ticks, continuous)
    }

    /// Reactivate a timer relative to its previous fire epoch.
    ///
    /// This retains the prior-deadline phase for catch-up while using the
    /// absolute primitive to keep a late reactivation nonzero.
    pub fn reactivate_timer_relative(
        &mut self,
        timer_index: usize,
        period: u64,
    ) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        let period = period.max(MIN_ALLOWABLE_TIMER_PERIOD);
        let deadline_ticks = self.timers[timer_index]
            .time_to_fire
            .checked_add(period)
            .ok_or(PcSystemError::TimerDeadlineOverflow)?;
        self.timers[timer_index].period = period;
        self.activate_timer_at_ticks(timer_index, deadline_ticks, false)
    }

    /// Deactivate a timer.
    ///
    /// Corresponds to `bx_pc_system_c::deactivate_timer()` in Bochs (pc_system.cc).
    pub fn deactivate_timer(&mut self, timer_index: usize) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        self.timers[timer_index].flags.remove(TimerFlags::ACTIVE);
        Ok(())
    }

    /// Unregister a timer, freeing its slot for reuse.
    ///
    /// Corresponds to `bx_pc_system_c::unregisterTimer()` in Bochs (pc_system.cc).
    /// The timer must be deactivated first.
    pub fn unregister_timer(&mut self, timer_index: usize) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        if self.timers[timer_index].flags.contains(TimerFlags::ACTIVE) {
            return Err(PcSystemError::TimerStillActive(timer_index));
        }
        self.timers[timer_index].flags = TimerFlags::empty();
        self.timers[timer_index].period = u64::MAX;
        self.timers[timer_index].time_to_fire = u64::MAX;
        self.timers[timer_index].owner = TimerOwner::NullTimer;
        self.timers[timer_index].id = [0; BX_MAX_TIMER_ID_LEN];

        if timer_index == self.num_timers - 1 {
            self.num_timers -= 1;
        }
        Ok(())
    }

    /// Get the number of ticks until next timer event.
    /// Matches Bochs pc_system.h `getNumCpuTicksLeftNextEvent()`.
    #[inline]
    pub fn get_num_cpu_ticks_left_next_event(&self) -> u32 {
        self.curr_countdown
    }

    /// Probe whether a FastRep batch would reach the next countdown event.
    ///
    /// This does not mutate `curr_countdown`; the outer emulator loop advances
    /// pc_system time exactly once with `tickn(executed)`.
    #[inline]
    pub fn countdown_would_expire_after(&self, n: u32) -> bool {
        n >= self.curr_countdown
    }

    /// Get the number of registered timers.
    pub fn num_timers(&self) -> usize {
        self.num_timers
    }

    /// Check if a timer is active (for diagnostics).
    pub fn is_timer_active(&self, timer_index: usize) -> bool {
        if timer_index >= self.num_timers {
            return false;
        }
        self.timers[timer_index].flags.contains(TimerFlags::ACTIVE)
    }

    /// Get ticks remaining until a timer fires (for diagnostics).
    /// Returns 0 if timer is inactive or index is out of bounds.
    pub fn timer_countdown(&self, timer_index: usize) -> u64 {
        if timer_index >= self.num_timers {
            return 0;
        }
        if !self.timers[timer_index].flags.contains(TimerFlags::ACTIVE) {
            return 0;
        }
        let now = self.time_ticks();
        self.timers[timer_index].time_to_fire.saturating_sub(now)
    }

    /// Return ticks until next countdown event (Bochs getNumCpuTicksLeftNextEvent).
    #[inline]
    pub fn get_num_ticks_left_next_event(&self) -> u32 {
        self.curr_countdown
    }

    /// Return minimum ticks until any active timer fires.
    /// Returns u64::MAX if no timers are active.
    pub fn min_ticks_to_fire(&self) -> u64 {
        let now = self.time_ticks();
        let mut min = u64::MAX;
        for i in 0..self.num_timers {
            if self.timers[i].flags.contains(TimerFlags::ACTIVE) {
                let remaining = self.timers[i].time_to_fire.saturating_sub(now);
                if remaining < min {
                    min = remaining;
                }
            }
        }
        min
    }

    /// Return the earliest active non-null timer deadline in absolute ticks.
    ///
    /// The central scheduler uses this fixed-storage query to cap an elapsed
    /// step before a device callback can rearm another owner.
    pub fn next_timer_deadline_ticks(&self) -> Option<u64> {
        let mut deadline: Option<u64> = None;
        for timer in self.timers[..self.num_timers].iter() {
            if !timer.flags.contains(TimerFlags::ACTIVE) || timer.owner == TimerOwner::NullTimer {
                continue;
            }
            deadline = Some(match deadline {
                Some(current) => current.min(timer.time_to_fire),
                None => timer.time_to_fire,
            });
        }
        deadline
    }


    /// Emulate ISA bus timing delay (Bochs pc_system.cc).
    /// ISA bus runs at ~8 MHz. Each ISA cycle takes ~125ns.
    /// At typical IPS rates, this advances the tick counter to simulate bus delay.
    /// Emulate ISA bus timing delay (Bochs pc_system.cc).
    /// ISA bus runs at ~8 MHz. Each ISA cycle consumes CPU ticks
    /// proportional to IPS. Bochs: `tickn((Bit32u)(m_ips * 2.0))`
    pub fn isa_bus_delay(&mut self) {
        let ips = self.ips;
        if ips > 4_000_000 {
            let ticks = (ips * 2 / 1_000_000) as u32;
            self.tickn(ticks);
        }
    }

    /// Drain the buffer of timer owners that fired since the last drain.
    /// Returns `(owners, counts, count)` — iterate entries `0..count`.
    /// True when `tickn` recorded timer fires that have not been dispatched.
    /// Cheap read for the SMP round loop's servicing gate.
    #[inline]
    pub fn has_fired_timers(&self) -> bool {
        self.num_fired > 0
    }

    pub fn take_fired_timers(
        &mut self,
    ) -> ([TimerOwner; BX_MAX_TIMERS], [u32; BX_MAX_TIMERS], usize) {
        let owners = self.fired_owners;
        let counts = self.fired_owner_counts;
        let count = self.num_fired;
        for entry in 0..count {
            self.fired_owner_counts[entry] = 0;
        }
        self.num_fired = 0;
        (owners, counts, count)
    }

    /// Return the exact v3 payload length for this PC-system object.
    ///
    /// The payload owns its section-version prefix and streams every timer
    /// slot, so the section writer never needs a staging buffer.
    #[cfg(feature = "std")]
    pub(crate) fn snapshot_v3_len(&self) -> io::Result<u64> {
        self.validate_snapshot_v3_state()?;

        let mut len = 0u64;
        for field_len in [
            4u64, // section version
            8,    // configured IPS
            4,    // current countdown
            4,    // current countdown period
            8,    // total ticks
            8,    // last usec clock
            8,    // usec since last clock
            1,    // A20 enabled
            8,    // A20 mask
            1,    // HRQ
            1,    // HRQ pending
            1,    // async event pending
            1,    // interrupt raised
            1,    // interrupt cleared
            1,    // kill request
            4,    // timer capacity
            4,    // timer high-water count
            4,    // triggered slot
        ] {
            len = checked_snapshot_len_add(len, field_len)?;
        }

        let timer_wire_len =
            checked_snapshot_len_add(TIMER_WIRE_FIXED_LEN, TIMER_OWNER_WIRE_LEN)?;
        let timer_wire_len = checked_snapshot_len_add(
            timer_wire_len,
            snapshot_usize_to_u64(BX_MAX_TIMER_ID_LEN)?,
        )?;
        len = checked_snapshot_len_add(
            len,
            checked_snapshot_len_mul(timer_wire_len, snapshot_usize_to_u64(BX_MAX_TIMERS)?)?,
        )?;
        len = checked_snapshot_len_add(len, 4)?; // fired-owner queue count
        len = checked_snapshot_len_add(
            len,
            checked_snapshot_len_mul(
                FIRED_OWNER_WIRE_LEN,
                snapshot_usize_to_u64(self.num_fired)?,
            )?,
        )?;
        Ok(len)
    }

    /// Stream the complete v3 PC-system state, including all timer ownership
    /// and pending timer-dispatch work.
    #[cfg(feature = "std")]
    pub(crate) fn save_snapshot_v3<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.validate_snapshot_v3_state()?;

        writer.write_u32(SNAPSHOT_SECTION_VERSION)?;
        writer.write_u64(self.ips)?;
        writer.write_u32(self.curr_countdown)?;
        writer.write_u32(self.curr_countdown_period)?;
        writer.write_u64(self.ticks_total)?;
        writer.write_u64(self.last_time_usec)?;
        writer.write_u64(self.usec_since_last)?;
        writer.write_bool(self.enable_a20)?;
        writer.write_u64(self.a20_mask)?;
        writer.write_bool(self.hrq)?;
        writer.write_bool(self.hrq_pending)?;
        writer.write_bool(self.async_event_pending)?;
        writer.write_bool(self.intr_raised)?;
        writer.write_bool(self.intr_cleared)?;
        writer.write_bool(self.kill_bochs_request)?;
        writer.write_u32(snapshot_usize_to_u32(BX_MAX_TIMERS)?)?;
        writer.write_u32(snapshot_usize_to_u32(self.num_timers)?)?;
        writer.write_u32(snapshot_usize_to_u32(self.triggered_timer)?)?;

        for timer in &self.timers {
            writer.write_u8(timer.flags.bits())?;
            writer.write_u64(timer.period)?;
            writer.write_u64(timer.time_to_fire)?;
            write_timer_owner(writer, timer.owner)?;
            writer.write_bytes(&timer.id)?;
        }

        writer.write_u32(snapshot_usize_to_u32(self.num_fired)?)?;
        for (&owner, &count) in self
            .fired_owners
            .iter()
            .zip(self.fired_owner_counts.iter())
            .take(self.num_fired)
        {
            write_timer_owner(writer, owner)?;
            writer.write_u32(count)?;
        }
        Ok(())
    }

    /// Restore the complete v3 PC-system payload without changing live state
    /// until its bounds, topology, and clock invariants have all validated.
    ///
    /// Callback topology is intentionally not represented here: device codecs
    /// retain their host anchors and validate their saved timer handles through
    /// `validate_timer_handle_owner` after this object has restored.
    #[cfg(feature = "std")]
    pub(crate) fn restore_snapshot_v3<R: Read>(
        &mut self,
        reader: &mut SnapshotReader<R>,
    ) -> io::Result<()> {
        let section_version = reader.read_u32()?;
        if section_version != SNAPSHOT_SECTION_VERSION {
            return Err(snapshot_invalid_data("unsupported PC-system section version"));
        }

        let ips = reader.read_u64()?;
        if ips == 0 || ips != self.ips {
            return Err(snapshot_invalid_data("configured IPS does not match"));
        }
        let curr_countdown = reader.read_u32()?;
        let curr_countdown_period = reader.read_u32()?;
        let ticks_total = reader.read_u64()?;
        let last_time_usec = reader.read_u64()?;
        let usec_since_last = reader.read_u64()?;
        let enable_a20 = reader.read_bool()?;
        let a20_mask = reader.read_u64()?;
        let hrq = reader.read_bool()?;
        let hrq_pending = reader.read_bool()?;
        let async_event_pending = reader.read_bool()?;
        let intr_raised = reader.read_bool()?;
        let intr_cleared = reader.read_bool()?;
        let kill_bochs_request = reader.read_bool()?;

        let saved_timer_capacity = reader.read_count(BX_MAX_TIMERS)?;
        if saved_timer_capacity != BX_MAX_TIMERS {
            return Err(snapshot_invalid_data("timer capacity does not match"));
        }
        let num_timers = reader.read_count(BX_MAX_TIMERS)?;
        let triggered_timer = reader.read_count(BX_MAX_TIMERS - 1)?;

        let mut timers = core::array::from_fn(|_| Timer::default());
        for timer in &mut timers {
            let flags = reader.read_u8()?;
            timer.flags = TimerFlags::from_bits(flags)
                .ok_or_else(|| snapshot_invalid_data("timer flags contain unknown bits"))?;
            timer.period = reader.read_u64()?;
            timer.time_to_fire = reader.read_u64()?;
            timer.owner = read_timer_owner(reader)?;
            reader.read_bytes(&mut timer.id)?;
        }

        let num_fired = reader.read_count(BX_MAX_TIMERS.min(bounds::MAX_SNAPSHOT_QUEUE_LEN))?;
        let mut fired_owners = [TimerOwner::NullTimer; BX_MAX_TIMERS];
        let mut fired_owner_counts = [0u32; BX_MAX_TIMERS];
        for (owner, count) in fired_owners
            .iter_mut()
            .zip(fired_owner_counts.iter_mut())
            .take(num_fired)
        {
            *owner = read_timer_owner(reader)?;
            *count = reader.read_u32()?;
        }
        reader.finish_exact()?;

        validate_snapshot_state_fields(
            ips,
            curr_countdown,
            curr_countdown_period,
            ticks_total,
            &timers,
            num_timers,
            triggered_timer,
            &fired_owners,
            &fired_owner_counts,
            num_fired,
            enable_a20,
            a20_mask,
        )?;

        self.timers = timers;
        self.num_timers = num_timers;
        self.triggered_timer = triggered_timer;
        self.curr_countdown = curr_countdown;
        self.curr_countdown_period = curr_countdown_period;
        self.ticks_total = ticks_total;
        self.last_time_usec = last_time_usec;
        self.usec_since_last = usec_since_last;
        self.enable_a20 = enable_a20;
        self.a20_mask = a20_mask;
        self.hrq = hrq;
        self.hrq_pending = hrq_pending;
        self.async_event_pending = async_event_pending;
        self.intr_raised = intr_raised;
        self.intr_cleared = intr_cleared;
        self.kill_bochs_request = kill_bochs_request;
        self.fired_owners = fired_owners;
        self.fired_owner_counts = fired_owner_counts;
        self.num_fired = num_fired;
        Ok(())
    }

    /// Locate the registered slot owned by `owner`, if any.
    #[cfg(feature = "std")]
    pub(crate) fn find_timer_slot_by_owner(&self, owner: TimerOwner) -> Option<usize> {
        (0..self.num_timers).find(|&index| {
            self.timers[index].flags.contains(TimerFlags::IN_USE)
                && self.timers[index].owner == owner
        })
    }

    /// Whether a fired-but-undispatched callback for `owner` is queued.
    #[cfg(feature = "std")]
    pub(crate) fn has_fired_owner(&self, owner: TimerOwner) -> bool {
        self.fired_owners[..self.num_fired].contains(&owner)
    }

    /// Validate that a decoded device timer handle still names the fixed
    /// scheduler owner it expects. Device callbacks and host resources remain
    /// live; this verifies the restored data did not redirect either one.
    #[cfg(feature = "std")]
    pub(crate) fn validate_timer_handle_owner(
        &self,
        handle: usize,
        expected: TimerOwner,
    ) -> io::Result<()> {
        timer_owner_wire_parts(expected)?;
        if handle >= self.num_timers {
            return Err(snapshot_invalid_data("timer handle is outside the registered range"));
        }
        let timer = self
            .timers
            .get(handle)
            .ok_or_else(|| snapshot_invalid_data("timer handle is out of range"))?;
        if !timer.flags.contains(TimerFlags::IN_USE) {
            return Err(snapshot_invalid_data("timer handle names an unused slot"));
        }
        if timer.owner != expected {
            return Err(snapshot_invalid_data("timer handle owner does not match"));
        }
        Ok(())
    }

    #[cfg(feature = "std")]
    fn validate_snapshot_v3_state(&self) -> io::Result<()> {
        validate_snapshot_state_fields(
            self.ips,
            self.curr_countdown,
            self.curr_countdown_period,
            self.ticks_total,
            &self.timers,
            self.num_timers,
            self.triggered_timer,
            &self.fired_owners,
            &self.fired_owner_counts,
            self.num_fired,
            self.enable_a20,
            self.a20_mask,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_pc_system() {
        let pc = BxPcSystemC::new();
        // A20 starts DISABLED at boot (8086 compat)
        assert!(!pc.enable_a20);
        assert_eq!(pc.a20_mask, 0xFFFF_FFFF_FFEF_FFFFu64);
        assert_eq!(pc.num_timers, 1); // null timer
                                      // Countdown should be NULL_TIMER_INTERVAL (u32::MAX)
        assert_eq!(pc.curr_countdown, 0xFFFF_FFFF);
        assert_eq!(pc.curr_countdown_period, 0xFFFF_FFFF);
    }

    #[test]
    fn test_a20_control() {
        let mut pc = BxPcSystemC::new();

        // Initially disabled
        assert!(!pc.get_enable_a20());

        // Test address masking with A20 disabled (default)
        let addr: u64 = 0x0010_0000; // 1MB mark (bit 20 set)
        let masked = pc.a20_addr(addr);
        assert_eq!(masked, 0x0000_0000); // Bit 20 should be masked off

        // Enable A20
        pc.set_enable_a20(true);
        assert!(pc.get_enable_a20());
        assert_eq!(pc.a20_mask, 0xFFFF_FFFF_FFFF_FFFFu64);
        let masked = pc.a20_addr(addr);
        assert_eq!(masked, 0x0010_0000); // No masking

        // Disable A20 again
        pc.set_enable_a20(false);
        assert!(!pc.get_enable_a20());
        assert_eq!(pc.a20_mask, 0xFFFF_FFFF_FFEF_FFFFu64);
    }

    #[test]
    fn test_multiple_instances() {
        let mut pc1 = BxPcSystemC::new();
        let pc2 = BxPcSystemC::new();

        // Modify pc1
        pc1.set_enable_a20(false);
        pc1.tickn(1000);

        // pc2 should be unaffected — A20 starts disabled for both
        assert!(!pc2.get_enable_a20());
        assert_eq!(pc2.time_ticks(), 0);
    }

    #[test]
    fn test_timer_registration() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(15_000_000); // 15 MIPS

        // Register a timer — should get slot 1 (slot 0 is null timer)
        let idx = pc
            .register_timer(TimerOwner::PciIdeCh0, 1000, true, true, "test_timer")
            .unwrap();
        assert_eq!(idx, 1);
        assert_eq!(pc.num_timers, 2);

        // Countdown should be adjusted to 1000 (since 1000 < NULL_TIMER_INTERVAL)
        assert_eq!(pc.curr_countdown, 1000);
    }

    #[test]
    fn test_timer_usec_conversion() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(15_000_000); // 15 MIPS → ips = 15_000_000

        // 1000 usec at 15 MIPS = 15000 ticks
        let idx = pc
            .register_timer_usec(TimerOwner::PciIdeCh0, 1000, true, true, "usec_timer")
            .unwrap();
        assert_eq!(idx, 1);
        assert_eq!(pc.timers[1].period, 15000);
    }

    #[test]
    fn inactive_timer_registration_records_future_deadline() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(1_000_000);
        pc.tickn(25);

        let idx = pc
            .register_timer(TimerOwner::PciIdeCh0, 1000, false, false, "inactive")
            .unwrap();

        assert!(!pc.timers[idx].flags.contains(TimerFlags::ACTIVE));
        assert_eq!(pc.timers[idx].time_to_fire, 1025);
    }

    #[test]
    fn test_time_ticks_partial() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(10_000_000); // 10 MIPS

        // Register a timer with period 100
        let _idx = pc
            .register_timer(TimerOwner::PciIdeCh0, 100, true, true, "partial_test")
            .unwrap();

        // Advance 50 ticks — should NOT fire yet
        pc.tickn(50);
        // time_ticks() should be 50 (partial countdown)
        assert_eq!(pc.time_ticks(), 50);
        // ticks_total should still be 0 (no countdown_event yet)
        assert_eq!(pc.ticks_total, 0);
    }

    #[test]
    fn test_time_usec_nsec() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(10_000_000); // 10 MIPS → ips = 10_000_000

        pc.tickn(10_000_000); // 10M ticks = 1 second at 10 MIPS
        assert_eq!(pc.time_usec(), 1_000_000); // 1 second in microseconds
        assert_eq!(pc.time_nsec(), 1_000_000_000); // 1 second in nanoseconds
    }

    #[test]
    fn test_timer_fire_and_deactivate() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(1_000_000); // 1 MIPS

        let idx = pc
            .register_timer(
                TimerOwner::PciIdeCh0,
                100,
                false, // one-shot
                true,
                "oneshot",
            )
            .unwrap();

        // Advance 50 ticks — not yet fired
        pc.tickn(50);
        assert!(pc.timers[idx].flags.contains(TimerFlags::ACTIVE));

        // Advance 50 more — fires at 100
        pc.tickn(50);
        assert!(!pc.timers[idx].flags.contains(TimerFlags::ACTIVE)); // one-shot deactivated
    }

    #[test]
    fn test_continuous_timer_fires_multiple() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(1_000_000);

        let _idx = pc
            .register_timer(
                TimerOwner::PciIdeCh0,
                100,
                true, // continuous
                true,
                "continuous",
            )
            .unwrap();

        // Advance 500 ticks — should fire 5 times (at 100, 200, 300, 400, 500)
        pc.tickn(500);
        let (owners, counts, count) = pc.take_fired_timers();
        assert_eq!(count, 1);
        assert_eq!(owners[0], TimerOwner::PciIdeCh0);
        assert_eq!(counts[0], 5);
    }

    #[test]
    fn continuous_timer_many_fires_coalesces_without_overflow() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(1_000_000);

        pc.register_timer(TimerOwner::PciIdeCh0, 1, true, true, "continuous")
            .unwrap();

        pc.tickn(65);
        let (owners, counts, count) = pc.take_fired_timers();
        assert_eq!(count, 1);
        assert_eq!(owners[0], TimerOwner::PciIdeCh0);
        assert_eq!(counts[0], 65);
    }

    #[test]
    fn fastrep_countdown_probe_does_not_mutate_before_outer_tick() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(1_000_000);

        pc.register_timer(TimerOwner::PciIdeCh0, 1000, false, true, "oneshot")
            .unwrap();

        assert!(!pc.countdown_would_expire_after(500));
        assert_eq!(pc.curr_countdown, 1000);

        pc.tickn(500);
        let (_, _, count) = pc.take_fired_timers();
        assert_eq!(count, 0);

        pc.tickn(500);
        let (owners, counts, count) = pc.take_fired_timers();
        assert_eq!(count, 1);
        assert_eq!(owners[0], TimerOwner::PciIdeCh0);
        assert_eq!(counts[0], 1);
    }

    #[test]
    fn test_unregister_timer() {
        let mut pc = BxPcSystemC::new();
        let idx = pc
            .register_timer(
                TimerOwner::PciIdeCh0,
                1000,
                true,
                false, // inactive
                "unreg",
            )
            .unwrap();

        pc.unregister_timer(idx).unwrap();
        assert!(!pc.timers[idx].flags.contains(TimerFlags::IN_USE));

        // Can't unregister null timer
        assert!(pc.unregister_timer(0).is_err());
    }

    #[test]
    fn test_countdown_adjustment() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(15_000_000);

        // Register timer with period 1000
        let _t1 = pc
            .register_timer(TimerOwner::PciIdeCh0, 1000, true, true, "t1")
            .unwrap();
        assert_eq!(pc.curr_countdown, 1000);

        // Advance 200 ticks
        pc.tickn(200);
        assert_eq!(pc.curr_countdown, 800);

        // Now activate a second timer with period 500 — should adjust countdown
        let t2 = pc
            .register_timer(TimerOwner::PciIdeCh1, 500, true, true, "t2")
            .unwrap();
        // curr_countdown was 800, new timer needs 500 < 800
        // So countdown adjusted to 500
        assert_eq!(pc.curr_countdown, 500);
        assert!(pc.timers[t2].flags.contains(TimerFlags::ACTIVE));
    }

    #[test]
    fn absolute_activation_clamps_due_deadline_to_next_tick() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(1_000_000);
        pc.tickn(7);
        let timer = pc
            .register_timer(TimerOwner::PciIdeCh0, 10, false, false, "clamped")
            .unwrap();

        let now = pc.time_ticks();
        pc.activate_timer_at_ticks(timer, now - 1, false).unwrap();
        assert_eq!(pc.timers[timer].time_to_fire, now + 1);
        assert_eq!(pc.get_num_ticks_left_next_event(), 1);
        assert_eq!(pc.time_ticks(), now);

        pc.tick1();
        let (owners, counts, count) = pc.take_fired_timers();
        assert_eq!(count, 1);
        assert_eq!(owners[0], TimerOwner::PciIdeCh0);
        assert_eq!(counts[0], 1);
    }


    #[test]
    fn continuous_absolute_activation_preserves_registered_period() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(1_000_000);
        pc.tickn(7);
        let timer = pc
            .register_timer(TimerOwner::CmosOneSecond, 50, true, false, "second")
            .unwrap();

        let now = pc.time_ticks();
        pc.activate_timer_at_ticks(timer, now + 10, true).unwrap();
        assert_eq!(pc.timers[timer].period, 50);

        pc.tickn(10);
        let (_, counts, count) = pc.take_fired_timers();
        assert_eq!(count, 1);
        assert_eq!(counts[0], 1);

        pc.tickn(49);
        assert!(!pc.has_fired_timers());
        pc.tick1();
        let (owners, counts, count) = pc.take_fired_timers();
        assert_eq!(count, 1);
        assert_eq!(owners[0], TimerOwner::CmosOneSecond);
        assert_eq!(counts[0], 1);
    }
    #[test]
    fn absolute_time_conversions_report_overflow_without_wrapping() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(u32::MAX);

        assert!(matches!(
            pc.usec_to_ticks(u64::MAX),
            Err(PcSystemError::TimeConversionOverflow)
        ));

        pc.initialize(1);
        assert!(matches!(
            pc.time_usec_at_ticks(u64::MAX),
            Err(PcSystemError::TimeConversionOverflow)
        ));
    }

    #[test]
    fn tied_absolute_deadlines_fire_in_registration_order() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(1_000_000);
        let first = pc
            .register_timer(TimerOwner::PciIdeCh0, 100, false, false, "first")
            .unwrap();
        let second = pc
            .register_timer(TimerOwner::PciIdeCh1, 100, false, false, "second")
            .unwrap();
        let deadline = pc.time_ticks() + 25;

        pc.activate_timer_at_ticks(first, deadline, false).unwrap();
        pc.activate_timer_at_ticks(second, deadline, false).unwrap();
        pc.tickn(25);

        let (owners, counts, count) = pc.take_fired_timers();
        assert_eq!(count, 2);
        assert_eq!(owners[..count], [TimerOwner::PciIdeCh0, TimerOwner::PciIdeCh1]);
        assert_eq!(counts[..count], [1, 1]);
    }

    #[test]
    fn absolute_activation_updates_countdown_and_deadline_query() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(1_000_000);
        pc.tickn(10);
        let now = pc.time_ticks();
        let later = pc
            .register_timer(TimerOwner::PciIdeCh0, 100, false, false, "later")
            .unwrap();
        let earlier = pc
            .register_timer(TimerOwner::PciIdeCh1, 50, false, false, "earlier")
            .unwrap();

        pc.activate_timer_at_ticks(later, now + 100, false).unwrap();
        assert_eq!(pc.get_num_ticks_left_next_event(), 100);
        pc.activate_timer_at_ticks(earlier, now + 50, false).unwrap();
        assert_eq!(pc.get_num_ticks_left_next_event(), 50);
        assert_eq!(pc.next_timer_deadline_ticks(), Some(now + 50));
        assert_eq!(pc.time_ticks(), now);

        pc.tickn(50);
        assert_eq!(pc.time_ticks(), now + 50);
        assert_eq!(pc.get_num_ticks_left_next_event(), 50);
    }
    #[test]
    fn maximum_topology_registers_every_lapic_and_fixed_owner() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(1_000_000);
        let fixed = [
            TimerOwner::Pit,
            TimerOwner::Keyboard,
            TimerOwner::CmosPeriodic,
            TimerOwner::CmosOneSecond,
            TimerOwner::CmosUip,
            TimerOwner::AcpiPmOverflow,
            TimerOwner::SerialFifo(0),
            TimerOwner::SerialFifo(1),
            TimerOwner::SerialFifo(2),
            TimerOwner::SerialFifo(3),
            TimerOwner::PciIdeCh0,
            TimerOwner::PciIdeCh1,
            #[cfg(feature = "std")]
            TimerOwner::Slowdown,
        ];
        for owner in fixed {
            pc.register_timer(owner, 0, false, false, "fixed")
                .unwrap();
        }
        for cpu_index in 0..crate::params::BX_MAX_SMP_THREADS_SUPPORTED as usize {
            pc.register_timer(
                TimerOwner::Lapic(cpu_index),
                0,
                false,
                false,
                "lapic",
            )
            .unwrap();
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn snapshot_timer_owner_phase_roundtrip_rejects_owner_mismatch() {
        use std::io::Cursor;

        let mut source = BxPcSystemC::new();
        source.initialize(1_000_000);
        source.tickn(15);
        let handle = source
            .register_timer(TimerOwner::CmosPeriodic, 29, true, false, "CMOS periodic")
            .unwrap();
        let deadline = source.time_ticks() + 41;
        source
            .activate_timer_at_ticks(handle, deadline, true)
            .unwrap();

        let mut payload = Vec::new();
        source.save_snapshot_v3(&mut payload).unwrap();

        let mut restored = BxPcSystemC::new();
        restored.initialize(1_000_000);
        let mut reader =
            SnapshotReader::new(Cursor::new(payload.as_slice()), payload.len() as u64).unwrap();
        restored.restore_snapshot_v3(&mut reader).unwrap();

        assert_eq!(restored.time_ticks(), source.time_ticks());
        assert_eq!(restored.next_timer_deadline_ticks(), Some(deadline));
        assert_eq!(restored.timer_countdown(handle), 41);
        restored
            .validate_timer_handle_owner(handle, TimerOwner::CmosPeriodic)
            .unwrap();
        let mismatch = restored
            .validate_timer_handle_owner(handle, TimerOwner::CmosOneSecond)
            .unwrap_err();
        assert_eq!(mismatch.kind(), ErrorKind::InvalidData);

        restored.tickn(40);
        assert!(!restored.has_fired_timers());
        restored.tickn(1);
        let (owners, counts, count) = restored.take_fired_timers();
        assert_eq!(count, 1);
        assert_eq!(owners[0], TimerOwner::CmosPeriodic);
        assert_eq!(counts[0], 1);
    }

    #[cfg(feature = "std")]
    #[test]
    fn snapshot_saves_after_countdown_timer_is_deactivated() {
        // `deactivate_timer` clears the ACTIVE flag without re-narrowing the
        // countdown (matching Bochs), so the countdown legitimately points at
        // a since-departed deadline until the next `countdown_event`. Saving
        // in that state — reached by every real long boot — must succeed and
        // round-trip, and the machine must still recompute correctly.
        use std::io::Cursor;

        let mut source = BxPcSystemC::new();
        source.initialize(1_000_000);

        // A near timer sets the countdown, a farther timer stays active.
        let near = source
            .register_timer(TimerOwner::Keyboard, 0, false, false, "kbd")
            .unwrap();
        let far = source
            .register_timer(TimerOwner::CmosPeriodic, 0, false, false, "cmos")
            .unwrap();
        source.activate_timer_at_ticks(far, source.time_ticks() + 500, true).unwrap();
        source.activate_timer_at_ticks(near, source.time_ticks() + 40, false).unwrap();
        assert_eq!(source.next_timer_deadline_ticks(), Some(source.time_ticks() + 40));

        // Deactivate the timer the countdown points at; the countdown is NOT
        // re-narrowed, so it now precedes the earliest active deadline (500).
        source.deactivate_timer(near).unwrap();

        let mut payload = Vec::new();
        source.save_snapshot_v3(&mut payload).unwrap();

        let mut restored = BxPcSystemC::new();
        restored.initialize(1_000_000);
        let mut reader =
            SnapshotReader::new(Cursor::new(payload.as_slice()), payload.len() as u64).unwrap();
        restored.restore_snapshot_v3(&mut reader).unwrap();
        assert_eq!(restored.time_ticks(), source.time_ticks());

        // The stale countdown wakes early, fires nothing, and recomputes to
        // the true remaining deadline; the far timer fires exactly on time.
        restored.tickn(40);
        assert!(!restored.has_fired_timers());
        restored.tickn(459);
        assert!(!restored.has_fired_timers());
        restored.tickn(1);
        let (owners, _counts, count) = restored.take_fired_timers();
        assert_eq!(count, 1);
        assert_eq!(owners[0], TimerOwner::CmosPeriodic);
    }
}
