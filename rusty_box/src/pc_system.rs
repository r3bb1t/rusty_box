use core::ffi::c_void;
#[cfg(feature = "std")]
use std::sync::OnceLock;

use crate::config::BxPhyAddress;
#[cfg(not(feature = "std"))]
use spin::once::Once;

#[cfg(feature = "std")]
static BX_PC_SYSTEM_LOCK: OnceLock<BxPcSystemC> = OnceLock::new();

#[cfg(not(feature = "std"))]
static BX_PC_SYSTEM_LOCK: Once<BxPcSystemC> = Once::new();

pub fn bx_pc_system() -> &'static BxPcSystemC {
    #[cfg(feature = "std")]
    return BX_PC_SYSTEM_LOCK.get_or_init(BxPcSystemC::new);
    #[cfg(not(feature = "std"))]
    BX_PC_SYSTEM_LOCK.call_once(BxPcSystemC::new)
}

const BxMaxTimerIDLen: usize = 32;
type BxTimerHandlerT = fn(arg: *mut c_void);

#[derive(Debug, Default)]
struct Timer {
    in_use: bool,                   // Timer slot is in-use (currently registered).
    period: u64,                    // Timer periodocity in cpu ticks.
    time_to_fire: u64,              // Time to fire next (in absolute ticks).
    active: bool,                   // 0=inactive, 1=active.
    continuous: bool,               // 0=one-shot timer, 1=continuous periodicity.
    funct: Option<BxTimerHandlerT>, // A callback function for when the
    // this_ptr: *mut c_void,            // The this-> pointer for C++ callbacks
    id: [u8; BxMaxTimerIDLen], // String ID of timer.
    param: u32,                // Device-specific value assigned to timer (optional)
}

#[derive(Debug, Default)]
pub struct BxPcSystemC {
    timer: Timer,
    num_timers: u32,
    triggered_timer: u32,
    curr_countdown: u32, // Current countdown ticks value (decrements to 0).
    curr_countdown_period: u32, // Length of current countdown period.
    ticks_total: u64,    // Num ticks total since start of emulator execution.
    last_time_usec: u64, // Last sequentially read time in usec.
    usec_since_last: u32, // Number of useconds claimed since then.
    a20_mask: BxPhyAddress,

    // A special null timer is always inserted in the timer[0] slot.  This
    // make sure that at least one timer is always active, and that the
    // duration is always less than a maximum 32-bit integer, so a 32-bit
    // counter can be used for the current countdown.
    NullTimerInterval: u64,
}

impl BxPcSystemC {
    fn new() -> Self {
        let mut sys = Self::default();
        // A20 line enabled: all bits can be used (no masking of bit 20)
        sys.a20_mask = 0xFFFFFFFFFFFFFFFFu64; // All bits enabled initially
        sys
    }

    /// Return current tick count used as a monotonic TSC source.
    pub fn time_ticks(&self) -> u64 {
        self.ticks_total
    }
}

pub fn a20_addr(x: BxPhyAddress) -> BxPhyAddress {
    x & bx_pc_system().a20_mask
}
