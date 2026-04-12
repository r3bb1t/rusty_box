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
}

/// Maximum length for timer ID strings
const BX_MAX_TIMER_ID_LEN: usize = 32;

/// Maximum number of timers per PC system instance
const BX_MAX_TIMERS: usize = 64;

/// Default null timer interval (in ticks).
/// Bochs  — `const Bit64u NullTimerInterval = 0xffffffff;`
/// This ensures the countdown always fits in a u32 (Bochs uses Bit32u for countdown).
const NULL_TIMER_INTERVAL: u64 = 0xFFFF_FFFF;

/// Minimum allowable timer period in ticks.
/// Bochs  — prevents ridiculously low timer frequencies
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
    /// Local APIC timer.
    Lapic,
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
    /// Instructions per second (in millions)
    m_ips: f64,
    /// Hardware Request (DMA)
    hrq: bool,
    /// HRQ pending flag — set by set_hrq(true), checked by emulator loop.
    /// Bochs : set_HRQ sets HRQ and signals async_event.
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
    /// Number of entries in `fired_owners`.
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
            m_ips: 1.0,
            hrq: false,
            hrq_pending: false,
            async_event_pending: false,
            intr_raised: false,
            intr_cleared: false,
            kill_bochs_request: false,
            fired_owners: [TimerOwner::NullTimer; BX_MAX_TIMERS],
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
    /// Corresponds to `bx_pc_system_c::initialize()` in Bochs ().
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
        self.m_ips = f64::from(ips) / 1_000_000.0;

        tracing::debug!("PC system initialized with ips = {}", ips);
    }

    // ========================================================================
    // Timer tick mechanism — matches Bochs 
    // ========================================================================

    /// Advance virtual time by `n` ticks, firing any expired timers.
    /// This is the core timing primitive — matches Bochs .
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
    /// Matches Bochs .
    #[inline]
    pub fn tick1(&mut self) {
        self.curr_countdown -= 1;
        if self.curr_countdown == 0 {
            self.countdown_event();
        }
    }

    /// Handle countdown reaching zero. Checks all timers, fires expired ones,
    /// and recalculates next countdown period.
    /// Matches Bochs  exactly.
    #[inline]
    fn countdown_event(&mut self) {
        let mut first = self.num_timers;
        let mut last = 0usize;
        let mut min_time_to_fire: u64 = u64::MAX;
        let mut triggered = [false; BX_MAX_TIMERS];

        // Step 1: Advance total ticks by the countdown period
        // Bochs 
        self.ticks_total += self.curr_countdown_period as u64;

        // Step 2: Scan all timers for fires and find next event
        // Bochs  uses `==` (ticksTotal == timeToFire).
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

        // Step 3: Calculate next countdown period BEFORE recording fires.
        // Timer reactivation during dispatch needs the new countdown.
        // Bochs 
        let next_period = (min_time_to_fire - self.ticks_total) as u32;
        self.curr_countdown = next_period;
        self.curr_countdown_period = next_period;

        // Step 4: Record all triggered timers for dispatch by the emulator.
        // Bochs  called handlers here; we defer to the
        // emulator so pc_system doesn't need device pointers.
        if first <= last {
            for (offset, &triggered_flag) in triggered[first..=last].iter().enumerate() {
                let i = first + offset;
                if triggered_flag {
                    self.triggered_timer = i;
                    let owner = self.timers[i].owner;
                    if owner != TimerOwner::NullTimer {
                        self.fired_owners[self.num_fired] = owner;
                        self.num_fired += 1;
                    }
                    self.triggered_timer = 0;
                }
            }
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

        tracing::debug!("A20: set() = {}", self.enable_a20);

        // If there has been a transition, TLB flush may be needed
        if old_enable_a20 != self.enable_a20 {
            // Note: TLB flush is handled by the caller (CPU)
            tracing::debug!("A20 line changed, memory mapping affected");
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
    // Time queries — matches Bochs  and 
    // ========================================================================

    /// Get precise current time in ticks, including partial countdown.
    /// Matches Bochs :
    /// `ticksTotal + (currCountdownPeriod - currCountdown)`
    #[inline]
    pub fn time_ticks(&self) -> u64 {
        self.ticks_total + (self.curr_countdown_period - self.curr_countdown) as u64
    }

    /// Convert ticks to microseconds using IPS setting.
    /// Matches Bochs .
    pub fn time_usec(&self) -> u64 {
        ((self.time_ticks() as f64) / self.m_ips) as u64
    }

    /// Convert ticks to nanoseconds using IPS setting.
    /// Matches Bochs .
    pub fn time_nsec(&self) -> u64 {
        ((self.time_ticks() as f64) / self.m_ips * 1000.0) as u64
    }

    // ========================================================================
    // DMA and system control
    // ========================================================================

    /// Set the Hardware Request (DMA) line.
    /// Matches Bochs : sets HRQ flag and signals async_event.
    pub fn set_hrq(&mut self, value: bool) {
        self.hrq = value;
        if value {
            self.hrq_pending = true;
            // Bochs : BX_CPU(0)->async_event = 1
            self.async_event_pending = true;
        }
    }


    /// Get the Hardware Request (DMA) line state
    pub fn get_hrq(&self) -> bool {
        self.hrq
    }

    /// Signal external interrupt to bootstrap CPU (Bochs ).
    ///
    /// Sets `intr_raised` so the emulator applies BX_EVENT_PENDING_INTR
    /// and async_event=1 to the CPU.
    pub fn raise_intr(&mut self) {
        self.intr_raised = true;
    }

    /// Clear external interrupt signal (Bochs ).
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
    pub fn reset(&mut self, reset_type: ResetReason) -> crate::Result<()> {
        tracing::info!("BxPcSystemC::reset({:?}) called", reset_type);

        // A20 line is ENABLED at hardware reset on 386+ CPUs
        // (Only 286 systems start with A20 disabled)
        self.set_enable_a20(true);

        // Clear DMA pending flag
        self.hrq_pending = false;

        Ok(())
    }

    /// Register state for save/restore functionality.
    /// Bochs uses parameter tree nodes. Our snapshot uses snapshot.rs instead.
    pub fn register_state(&self) {
        tracing::debug!("PC system state registered");
    }

    /// Start all registered timers. No-op — matches Bochs .
    /// Timer time_to_fire is set correctly during register_timer/activate_timer.
    pub fn start_timers(&mut self) {
        tracing::debug!("start_timers: no-op (timers started during registration)");
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

    /// Register a new timer with period in ticks.
    ///
    /// Corresponds to `bx_pc_system_c::register_timer_ticks()` in Bochs ().
    /// Returns the timer index on success, or `PcSystemError::NoFreeTimerSlots` if full.
    pub fn register_timer(
        &mut self,
        owner: TimerOwner,
        period: u64,
        continuous: bool,
        active: bool,
        id: &str,
    ) -> Result<usize, PcSystemError> {
        // Enforce minimum timer period (Bochs )
        let period = period.max(MIN_ALLOWABLE_TIMER_PERIOD);

        // Search for free timer slot (i = 0 is reserved for NullTimer)
        // Bochs 
        for i in 1..BX_MAX_TIMERS {
            if !self.timers[i].flags.contains(TimerFlags::IN_USE) {
                self.timers[i].flags = TimerFlags::IN_USE;
                self.timers[i].flags.set(TimerFlags::ACTIVE, active);
                self.timers[i].flags.set(TimerFlags::CONTINUOUS, continuous);
                self.timers[i].period = period;
                // Bochs :
                // timeToFire = (ticksTotal + Bit64u(currCountdownPeriod-currCountdown)) + ticks
                self.timers[i].time_to_fire = if active {
                    self.time_ticks() + period
                } else {
                    0
                };
                self.timers[i].owner = owner;

                // Copy ID string
                let id_bytes = id.as_bytes();
                let copy_len = id_bytes.len().min(BX_MAX_TIMER_ID_LEN - 1);
                self.timers[i].id[..copy_len].copy_from_slice(&id_bytes[..copy_len]);
                self.timers[i].id[copy_len] = 0;

                // Adjust countdown if this timer fires sooner than current countdown
                // Bochs 
                if active && period < self.curr_countdown as u64 {
                    self.curr_countdown_period -= self.curr_countdown - period as u32;
                    self.curr_countdown = period as u32;
                }

                if i >= self.num_timers {
                    self.num_timers = i + 1;
                }

                tracing::debug!("Registered timer {} with id '{}'", i, id);
                return Ok(i);
            }
        }

        Err(PcSystemError::NoFreeTimerSlots)
    }

    /// Register a new timer with period in microseconds.
    ///
    /// Corresponds to `bx_pc_system_c::register_timer()` in Bochs ().
    /// Converts microseconds to ticks using IPS setting, then delegates to register_timer.
    pub fn register_timer_usec(
        &mut self,
        owner: TimerOwner,
        useconds: u32,
        continuous: bool,
        active: bool,
        id: &str,
    ) -> Result<usize, PcSystemError> {
        // Convert useconds to number of ticks (Bochs )
        let ticks = (f64::from(useconds) * self.m_ips) as u64;
        self.register_timer(owner, ticks, continuous, active, id)
    }

    /// Activate a timer with period in ticks.
    ///
    /// Corresponds to `bx_pc_system_c::activate_timer_ticks()` in Bochs ().
    pub fn activate_timer(
        &mut self,
        timer_index: usize,
        period: u64,
        continuous: bool,
    ) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        // Enforce minimum timer period (Bochs )
        let period = period.max(MIN_ALLOWABLE_TIMER_PERIOD);
        self.timers[timer_index].period = period;
        // Bochs :
        // timeToFire = (ticksTotal + Bit64u(currCountdownPeriod-currCountdown)) + ticks
        self.timers[timer_index].time_to_fire = self.time_ticks() + period;
        self.timers[timer_index].flags.insert(TimerFlags::ACTIVE);
        self.timers[timer_index].flags.set(TimerFlags::CONTINUOUS, continuous);

        // Adjust countdown if this timer fires sooner than current countdown
        // Bochs 
        if period < self.curr_countdown as u64 {
            self.curr_countdown_period -= self.curr_countdown - period as u32;
            self.curr_countdown = period as u32;
        }
        Ok(())
    }

    /// Activate a timer with period in microseconds.
    ///
    /// Corresponds to `bx_pc_system_c::activate_timer()` in Bochs ().
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
            // Convert useconds to ticks (Bochs )
            let t = (f64::from(useconds) * self.m_ips) as u64;
            t.max(MIN_ALLOWABLE_TIMER_PERIOD)
        };
        self.activate_timer(timer_index, ticks, continuous)
    }

    /// Activate a timer with period in nanoseconds.
    ///
    /// Corresponds to `bx_pc_system_c::activate_timer_nsec()` in Bochs ().
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
            // Convert nseconds to ticks (Bochs )
            let t = ((nseconds as f64) * self.m_ips / 1000.0) as u64;
            t.max(MIN_ALLOWABLE_TIMER_PERIOD)
        };
        self.activate_timer(timer_index, ticks, continuous)
    }

    /// Reactivate a periodic timer relative to its previous fire time.
    ///
    /// Unlike `activate_timer` which sets `time_to_fire = time_ticks() + period`,
    /// this adds `period` to the existing `time_to_fire`. Used for LAPIC catch-up:
    /// after processing a timer fire, re-arm relative to the previous fire point.
    pub fn reactivate_timer_relative(
        &mut self,
        timer_index: usize,
        period: u64,
    ) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        let period = period.max(MIN_ALLOWABLE_TIMER_PERIOD);
        self.timers[timer_index].period = period;
        self.timers[timer_index].time_to_fire += period;
        self.timers[timer_index].flags.insert(TimerFlags::ACTIVE);
        self.timers[timer_index].flags.remove(TimerFlags::CONTINUOUS);

        // Adjust countdown if this timer fires sooner
        let ticks_until_fire = self.timers[timer_index]
            .time_to_fire
            .saturating_sub(self.time_ticks());
        if ticks_until_fire < self.curr_countdown as u64 {
            let ticks_u32 = ticks_until_fire as u32;
            self.curr_countdown_period -= self.curr_countdown - ticks_u32;
            self.curr_countdown = ticks_u32;
        }
        Ok(())
    }

    /// Deactivate a timer.
    ///
    /// Corresponds to `bx_pc_system_c::deactivate_timer()` in Bochs ().
    pub fn deactivate_timer(&mut self, timer_index: usize) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        self.timers[timer_index].flags.remove(TimerFlags::ACTIVE);
        Ok(())
    }

    /// Unregister a timer, freeing its slot for reuse.
    ///
    /// Corresponds to `bx_pc_system_c::unregisterTimer()` in Bochs ().
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
    /// Matches Bochs  `getNumCpuTicksLeftNextEvent()`.
    #[inline]
    pub fn get_num_cpu_ticks_left_next_event(&self) -> u32 {
        self.curr_countdown
    }

    /// Decrement countdown without firing events.
    /// Used by FastRep (CPU instruction handlers) to track tick consumption
    /// matching Bochs `BX_TICKN()` inside `faststring.cc`.
    /// Returns true if countdown expired (caller should set async_event).
    /// The actual `countdown_event()` fires later via the outer `tickn()` call.
    #[inline]
    pub fn sub_countdown(&mut self, n: u32) -> bool {
        if self.curr_countdown > n {
            self.curr_countdown -= n;
            false
        } else {
            self.curr_countdown = 0;
            true
        }
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

    /// Emulate ISA bus timing delay (Bochs ).
    /// ISA bus runs at ~8 MHz. Each ISA cycle takes ~125ns.
    /// At typical IPS rates, this advances the tick counter to simulate bus delay.
    /// Emulate ISA bus timing delay (Bochs ).
    /// ISA bus runs at ~8 MHz. Each ISA cycle consumes CPU ticks
    /// proportional to IPS. Bochs: `tickn((Bit32u)(m_ips * 2.0))`
    pub fn isa_bus_delay(&mut self) {
        let m_ips = self.m_ips;
        if m_ips > 4.0 {
            let ticks = (m_ips * 2.0) as u32;
            self.tickn(ticks);
        }
    }

    /// Drain the buffer of timer owners that fired since the last drain.
    /// Returns `(owners, count)` — iterate `owners[..count]` and dispatch.
    pub fn take_fired_timers(&mut self) -> ([TimerOwner; BX_MAX_TIMERS], usize) {
        let result = (self.fired_owners, self.num_fired);
        self.num_fired = 0;
        result
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
            .register_timer(
                TimerOwner::PciIdeCh0,
                1000,
                true,
                true,
                "test_timer",
            )
            .unwrap();
        assert_eq!(idx, 1);
        assert_eq!(pc.num_timers, 2);

        // Countdown should be adjusted to 1000 (since 1000 < NULL_TIMER_INTERVAL)
        assert_eq!(pc.curr_countdown, 1000);
    }

    #[test]
    fn test_timer_usec_conversion() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(15_000_000); // 15 MIPS → m_ips = 15.0

        // 1000 usec at 15 MIPS = 15000 ticks
        let idx = pc
            .register_timer_usec(
                TimerOwner::PciIdeCh0,
                1000,
                true,
                true,
                "usec_timer",
            )
            .unwrap();
        assert_eq!(idx, 1);
        assert_eq!(pc.timers[1].period, 15000);
    }

    #[test]
    fn test_time_ticks_partial() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(10_000_000); // 10 MIPS

        // Register a timer with period 100
        let _idx = pc
            .register_timer(
                TimerOwner::PciIdeCh0,
                100,
                true,
                true,
                "partial_test",
            )
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
        pc.initialize(10_000_000); // 10 MIPS → m_ips = 10.0

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
        let (owners, count) = pc.take_fired_timers();
        assert_eq!(count, 5);
        for &owner in &owners[..count] {
            assert_eq!(owner, TimerOwner::PciIdeCh0);
        }
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
            .register_timer(
                TimerOwner::PciIdeCh0,
                1000,
                true,
                true,
                "t1",
            )
            .unwrap();
        assert_eq!(pc.curr_countdown, 1000);

        // Advance 200 ticks
        pc.tickn(200);
        assert_eq!(pc.curr_countdown, 800);

        // Now activate a second timer with period 500 — should adjust countdown
        let t2 = pc
            .register_timer(
                TimerOwner::PciIdeCh1,
                500,
                true,
                true,
                "t2",
            )
            .unwrap();
        // curr_countdown was 800, new timer needs 500 < 800
        // So countdown adjusted to 500
        assert_eq!(pc.curr_countdown, 500);
        assert!(pc.timers[t2].flags.contains(TimerFlags::ACTIVE));
    }
}
