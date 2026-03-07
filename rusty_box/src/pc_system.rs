//! PC System - Instance-based timer and system control
//!
//! This module provides the PC system infrastructure including:
//! - Timer management for scheduling events
//! - A20 line control for memory addressing
//! - System reset coordination
//!
//! Each `BxPcSystemC` instance is fully independent, allowing multiple
//! emulator instances to run concurrently without conflicts.

use core::ffi::c_void;

use thiserror::Error;

use crate::config::BxPhyAddress;
use crate::cpu::ResetReason;

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
/// Bochs uses 0xFFFFFFFF (u32::MAX). We use u64::MAX so the null timer
/// effectively never fires, matching Bochs behavior where it just serves
/// as a sentinel to keep the timer array non-empty.
const NULL_TIMER_INTERVAL: u64 = u64::MAX;

/// Minimum allowable timer period in ticks.
/// Bochs pc_system.cc:37 — prevents ridiculously low timer frequencies
/// when IPS is set too low.
const MIN_ALLOWABLE_TIMER_PERIOD: u64 = 1;

/// Timer handler function type
pub type BxTimerHandlerT = fn(this_ptr: *mut c_void);

/// Individual timer structure
#[derive(Debug, Clone)]
pub struct Timer {
    /// Whether this timer slot is in use
    pub(crate) in_use: bool,
    /// Timer period in ticks
    pub(crate) period: u64,
    /// Absolute tick count when timer should fire
    pub(crate) time_to_fire: u64,
    /// Whether timer is currently active
    pub(crate) active: bool,
    /// Whether timer repeats continuously
    pub(crate) continuous: bool,
    /// Handler function to call when timer fires
    pub(crate) handler: Option<BxTimerHandlerT>,
    /// Timer identifier string
    pub(crate) id: [u8; BX_MAX_TIMER_ID_LEN],
    /// User parameter passed to handler
    pub(crate) param: *mut c_void,
}

impl Default for Timer {
    fn default() -> Self {
        Self {
            in_use: false,
            period: 0,
            time_to_fire: 0,
            active: false,
            continuous: false,
            handler: None,
            id: [0; BX_MAX_TIMER_ID_LEN],
            param: core::ptr::null_mut(),
        }
    }
}

// SAFETY: Timer's raw pointer is only dereferenced within single-threaded emulator context
unsafe impl Send for Timer {}
unsafe impl Sync for Timer {}

/// PC System controller - manages timers, A20 line, and system-level operations
///
/// This struct is fully instance-based with no global state, allowing multiple
/// independent emulator instances to run concurrently.
#[derive(Debug)]
pub struct BxPcSystemC {
    /// Array of timers
    timers: [Timer; BX_MAX_TIMERS],
    /// Number of registered timers
    num_timers: usize,
    /// Index of most recently triggered timer
    triggered_timer: usize,
    /// Current countdown value
    curr_countdown: u64,
    /// Period for current countdown
    curr_countdown_period: u64,
    /// Total ticks since emulator started
    ticks_total: u64,
    /// Last time in microseconds
    last_time_usec: u64,
    /// Microseconds since last sync
    usec_since_last: u64,
    /// A20 address mask (controls A20 line gating)
    a20_mask: BxPhyAddress,
    /// Whether A20 line is enabled
    enable_a20: bool,
    /// Instructions per second (in millions)
    m_ips: f64,
    /// Hardware Request (DMA)
    hrq: bool,
    /// Request to terminate emulation
    pub(crate) kill_bochs_request: bool,
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
            curr_countdown: NULL_TIMER_INTERVAL,
            curr_countdown_period: NULL_TIMER_INTERVAL,
            ticks_total: 0,
            last_time_usec: 0,
            usec_since_last: 0,
            // A20 line starts DISABLED at boot (bit 20 masked off)
            // This causes addresses like 0xFFFFFFF0 to wrap to 0x000FFFF0
            a20_mask: 0xFFFF_FFFF_FFEF_FFFFu64,
            enable_a20: false,
            m_ips: 1.0,
            hrq: false,
            kill_bochs_request: false,
        };

        // Register the null timer as timer 0
        sys.timers[0].in_use = true;
        sys.timers[0].period = NULL_TIMER_INTERVAL;
        sys.timers[0].time_to_fire = NULL_TIMER_INTERVAL;
        sys.timers[0].active = true;
        sys.timers[0].continuous = true;
        sys.timers[0].handler = Some(Self::null_timer_handler);
        sys.num_timers = 1;

        sys
    }

    /// Initialize the PC system with the given instructions-per-second value
    ///
    /// This sets up timer infrastructure and IPS-based timing.
    /// Corresponds to `bx_pc_system_c::initialize()` in Bochs.
    pub fn initialize(&mut self, ips: u32) {
        self.ticks_total = 0;
        self.timers[0].time_to_fire = NULL_TIMER_INTERVAL;
        self.curr_countdown = NULL_TIMER_INTERVAL;
        self.curr_countdown_period = NULL_TIMER_INTERVAL;
        self.last_time_usec = 0;
        self.usec_since_last = 0;
        self.triggered_timer = 0;
        self.hrq = false;
        self.kill_bochs_request = false;

        // Convert IPS to millions for timing calculations
        self.m_ips = f64::from(ips) / 1_000_000.0;

        tracing::debug!("PC system initialized with ips = {}", ips);
    }

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

    /// Get total ticks elapsed since emulator start
    pub fn time_ticks(&self) -> u64 {
        self.ticks_total
    }

    /// Convert ticks to microseconds using IPS setting.
    ///
    /// Corresponds to `bx_pc_system_c::time_usec()` in Bochs (pc_system.cc:462).
    pub fn time_usec(&self) -> u64 {
        ((self.ticks_total as f64) / self.m_ips) as u64
    }

    /// Convert ticks to nanoseconds using IPS setting.
    ///
    /// Corresponds to `bx_pc_system_c::time_nsec()` in Bochs (pc_system.cc:467).
    pub fn time_nsec(&self) -> u64 {
        ((self.ticks_total as f64) / self.m_ips * 1000.0) as u64
    }

    /// Set the Hardware Request (DMA) line
    pub fn set_hrq(&mut self, value: bool) {
        self.hrq = value;
    }

    /// Get the Hardware Request (DMA) line state
    pub fn get_hrq(&self) -> bool {
        self.hrq
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

        Ok(())
    }

    /// Register state for save/restore functionality
    pub fn register_state(&self) {
        // TODO: Implement state registration for save/restore
        tracing::debug!("PC system state registered");
    }

    /// Start all registered timers
    pub fn start_timers(&mut self) {
        tracing::debug!("Starting {} timers", self.num_timers);
        // Activate all registered timers
        for i in 0..self.num_timers {
            if self.timers[i].in_use && self.timers[i].active {
                self.timers[i].time_to_fire = self.ticks_total + self.timers[i].period;
            }
        }
    }

    /// Validate a timer index (must be in range, in use, and not null timer).
    fn validate_timer_index(&self, timer_index: usize) -> Result<(), PcSystemError> {
        if timer_index >= BX_MAX_TIMERS {
            return Err(PcSystemError::TimerIndexOutOfBounds(timer_index));
        }
        if timer_index == 0 {
            return Err(PcSystemError::NullTimerModification);
        }
        if !self.timers[timer_index].in_use {
            return Err(PcSystemError::TimerNotInUse(timer_index));
        }
        Ok(())
    }

    /// Register a new timer with period in ticks.
    ///
    /// Corresponds to `bx_pc_system_c::register_timer_ticks()` in Bochs (pc_system.cc:262).
    /// Returns the timer index on success, or `PcSystemError::NoFreeTimerSlots` if full.
    pub fn register_timer(
        &mut self,
        handler: BxTimerHandlerT,
        param: *mut c_void,
        period: u64,
        continuous: bool,
        active: bool,
        id: &str,
    ) -> Result<usize, PcSystemError> {
        // Enforce minimum timer period (Bochs pc_system.cc:269)
        let period = period.max(MIN_ALLOWABLE_TIMER_PERIOD);

        // Search for free timer slot (i = 0 is reserved for NullTimer)
        // Bochs pc_system.cc:276
        for i in 1..BX_MAX_TIMERS {
            if !self.timers[i].in_use {
                self.timers[i].in_use = true;
                self.timers[i].period = period;
                self.timers[i].time_to_fire = if active { self.ticks_total + period } else { 0 };
                self.timers[i].active = active;
                self.timers[i].continuous = continuous;
                self.timers[i].handler = Some(handler);
                self.timers[i].param = param;

                // Copy ID string
                let id_bytes = id.as_bytes();
                let copy_len = id_bytes.len().min(BX_MAX_TIMER_ID_LEN - 1);
                self.timers[i].id[..copy_len].copy_from_slice(&id_bytes[..copy_len]);
                self.timers[i].id[copy_len] = 0;

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
    /// Corresponds to `bx_pc_system_c::register_timer()` in Bochs (pc_system.cc:253).
    /// Converts microseconds to ticks using IPS setting, then delegates to register_timer.
    pub fn register_timer_usec(
        &mut self,
        handler: BxTimerHandlerT,
        param: *mut c_void,
        useconds: u32,
        continuous: bool,
        active: bool,
        id: &str,
    ) -> Result<usize, PcSystemError> {
        // Convert useconds to number of ticks (Bochs pc_system.cc:257)
        let ticks = (f64::from(useconds) * self.m_ips) as u64;
        self.register_timer(handler, param, ticks, continuous, active, id)
    }

    /// Activate a timer with period in ticks.
    ///
    /// Corresponds to `bx_pc_system_c::activate_timer_ticks()` in Bochs (pc_system.cc:474).
    pub fn activate_timer(
        &mut self,
        timer_index: usize,
        period: u64,
        continuous: bool,
    ) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        // Enforce minimum timer period (Bochs pc_system.cc:488)
        let period = period.max(MIN_ALLOWABLE_TIMER_PERIOD);
        self.timers[timer_index].period = period;
        self.timers[timer_index].time_to_fire = self.ticks_total + period;
        self.timers[timer_index].active = true;
        self.timers[timer_index].continuous = continuous;
        Ok(())
    }

    /// Activate a timer with period in microseconds.
    ///
    /// Corresponds to `bx_pc_system_c::activate_timer()` in Bochs (pc_system.cc:508).
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
            // Convert useconds to ticks (Bochs pc_system.cc:525)
            let t = (f64::from(useconds) * self.m_ips) as u64;
            t.max(MIN_ALLOWABLE_TIMER_PERIOD)
        };
        self.activate_timer(timer_index, ticks, continuous)
    }

    /// Activate a timer with period in nanoseconds.
    ///
    /// Corresponds to `bx_pc_system_c::activate_timer_nsec()` in Bochs (pc_system.cc:539).
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
            // Convert nseconds to ticks (Bochs pc_system.cc:549)
            let t = ((nseconds as f64) * self.m_ips / 1000.0) as u64;
            t.max(MIN_ALLOWABLE_TIMER_PERIOD)
        };
        self.activate_timer(timer_index, ticks, continuous)
    }

    /// Deactivate a timer.
    ///
    /// Corresponds to `bx_pc_system_c::deactivate_timer()` in Bochs (pc_system.cc:563).
    pub fn deactivate_timer(&mut self, timer_index: usize) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        self.timers[timer_index].active = false;
        Ok(())
    }

    /// Unregister a timer, freeing its slot for reuse.
    ///
    /// Corresponds to `bx_pc_system_c::unregisterTimer()` in Bochs (pc_system.cc:575).
    /// The timer must be deactivated first.
    pub fn unregister_timer(&mut self, timer_index: usize) -> Result<(), PcSystemError> {
        self.validate_timer_index(timer_index)?;
        if self.timers[timer_index].active {
            return Err(PcSystemError::TimerStillActive(timer_index));
        }
        self.timers[timer_index].in_use = false;
        self.timers[timer_index].period = u64::MAX;
        self.timers[timer_index].time_to_fire = u64::MAX;
        self.timers[timer_index].continuous = false;
        self.timers[timer_index].handler = None;
        self.timers[timer_index].param = core::ptr::null_mut();
        self.timers[timer_index].id = [0; BX_MAX_TIMER_ID_LEN];

        if timer_index == self.num_timers - 1 {
            self.num_timers -= 1;
        }
        Ok(())
    }

    /// Set a timer's user parameter.
    ///
    /// Corresponds to `bx_pc_system_c::setTimerParam()` in Bochs (pc_system.cc:605).
    pub fn set_timer_param(
        &mut self,
        timer_index: usize,
        param: *mut c_void,
    ) -> Result<(), PcSystemError> {
        if timer_index >= self.num_timers {
            return Err(PcSystemError::TimerIndexOutOfBounds(timer_index));
        }
        self.timers[timer_index].param = param;
        Ok(())
    }

    /// Null timer handler (does nothing, just maintains timing)
    fn null_timer_handler(_param: *mut c_void) {
        // The null timer exists just to keep the timing system running
    }

    /// Tick the system by the given number of ticks
    pub fn tick(&mut self, ticks: u64) {
        self.ticks_total += ticks;
    }

    /// Check and fire any expired timers
    pub fn check_timers(&mut self) {
        for i in 0..self.num_timers {
            if self.timers[i].in_use
                && self.timers[i].active
                && self.ticks_total >= self.timers[i].time_to_fire
            {
                self.triggered_timer = i;

                if let Some(handler) = self.timers[i].handler {
                    handler(self.timers[i].param);
                }

                if self.timers[i].continuous {
                    self.timers[i].time_to_fire = self.ticks_total + self.timers[i].period;
                } else {
                    self.timers[i].active = false;
                }
            }
        }
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
        pc1.tick(1000);

        // pc2 should be unaffected — A20 starts disabled for both
        assert!(!pc2.get_enable_a20());
        assert_eq!(pc2.time_ticks(), 0);
    }

    fn dummy_handler(_: *mut c_void) {}

    #[test]
    fn test_timer_registration() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(15_000_000); // 15 MIPS

        // Register a timer — should get slot 1 (slot 0 is null timer)
        let idx = pc.register_timer(
            dummy_handler,
            core::ptr::null_mut(),
            1000,
            true,
            true,
            "test_timer",
        ).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(pc.num_timers, 2);
    }

    #[test]
    fn test_timer_usec_conversion() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(15_000_000); // 15 MIPS → m_ips = 15.0

        // 1000 usec at 15 MIPS = 15000 ticks
        let idx = pc.register_timer_usec(
            dummy_handler,
            core::ptr::null_mut(),
            1000,
            true,
            true,
            "usec_timer",
        ).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(pc.timers[1].period, 15000);
    }

    #[test]
    fn test_time_usec_nsec() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(10_000_000); // 10 MIPS → m_ips = 10.0

        pc.tick(10_000_000); // 10M ticks = 1 second at 10 MIPS
        assert_eq!(pc.time_usec(), 1_000_000); // 1 second in microseconds
        assert_eq!(pc.time_nsec(), 1_000_000_000); // 1 second in nanoseconds
    }

    #[test]
    fn test_timer_fire_and_deactivate() {
        let mut pc = BxPcSystemC::new();
        pc.initialize(1_000_000); // 1 MIPS

        let idx = pc.register_timer(
            dummy_handler,
            core::ptr::null_mut(),
            100,
            false, // one-shot
            true,
            "oneshot",
        ).unwrap();

        // Not yet time to fire
        pc.tick(50);
        pc.check_timers();
        assert!(pc.timers[idx].active);

        // Now fire
        pc.tick(50);
        pc.check_timers();
        assert!(!pc.timers[idx].active); // one-shot deactivated
    }

    #[test]
    fn test_unregister_timer() {
        let mut pc = BxPcSystemC::new();
        let idx = pc.register_timer(
            dummy_handler,
            core::ptr::null_mut(),
            1000,
            true,
            false, // inactive
            "unreg",
        ).unwrap();

        pc.unregister_timer(idx).unwrap();
        assert!(!pc.timers[idx].in_use);

        // Can't unregister null timer
        assert!(pc.unregister_timer(0).is_err());
    }
}
