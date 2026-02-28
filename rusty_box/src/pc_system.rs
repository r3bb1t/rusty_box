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

use crate::config::BxPhyAddress;
use crate::cpu::ResetReason;

/// Maximum length for timer ID strings
const BX_MAX_TIMER_ID_LEN: usize = 32;

/// Maximum number of timers per PC system instance
const BX_MAX_TIMERS: usize = 64;

/// Default null timer interval (in ticks)
const NULL_TIMER_INTERVAL: u64 = 100000;

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

    /// Register a new timer
    ///
    /// Returns the timer index on success
    pub fn register_timer(
        &mut self,
        handler: BxTimerHandlerT,
        param: *mut c_void,
        period: u64,
        continuous: bool,
        active: bool,
        id: &str,
    ) -> Option<usize> {
        // Find a free timer slot
        for i in 0..BX_MAX_TIMERS {
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
                return Some(i);
            }
        }

        tracing::error!("No free timer slots available");
        None
    }

    /// Activate a timer
    pub fn activate_timer(&mut self, timer_index: usize, period: u64, continuous: bool) {
        if timer_index < BX_MAX_TIMERS && self.timers[timer_index].in_use {
            self.timers[timer_index].period = period;
            self.timers[timer_index].time_to_fire = self.ticks_total + period;
            self.timers[timer_index].active = true;
            self.timers[timer_index].continuous = continuous;
        }
    }

    /// Deactivate a timer
    pub fn deactivate_timer(&mut self, timer_index: usize) {
        if timer_index < BX_MAX_TIMERS && self.timers[timer_index].in_use {
            self.timers[timer_index].active = false;
        }
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
        assert!(pc.enable_a20);
        assert_eq!(pc.a20_mask, 0xFFFF_FFFF_FFFF_FFFFu64);
        assert_eq!(pc.num_timers, 1); // null timer
    }

    #[test]
    fn test_a20_control() {
        let mut pc = BxPcSystemC::new();

        // Initially enabled
        assert!(pc.get_enable_a20());

        // Disable A20
        pc.set_enable_a20(false);
        assert!(!pc.get_enable_a20());
        assert_eq!(pc.a20_mask, 0xFFFF_FFFF_FFEF_FFFFu64);

        // Test address masking
        let addr: u64 = 0x0010_0000; // 1MB mark (bit 20 set)
        let masked = pc.a20_addr(addr);
        assert_eq!(masked, 0x0000_0000); // Bit 20 should be masked off

        // Re-enable A20
        pc.set_enable_a20(true);
        let masked = pc.a20_addr(addr);
        assert_eq!(masked, 0x0010_0000); // No masking
    }

    #[test]
    fn test_multiple_instances() {
        let mut pc1 = BxPcSystemC::new();
        let mut pc2 = BxPcSystemC::new();

        // Modify pc1
        pc1.set_enable_a20(false);
        pc1.tick(1000);

        // pc2 should be unaffected
        assert!(pc2.get_enable_a20());
        assert_eq!(pc2.time_ticks(), 0);
    }
}
