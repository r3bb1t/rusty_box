//! Machine-side implementations of the device-facing capability traits.
//!
//! [`device_api`](super::device_api) states what a device may ask for; this
//! module supplies it from the machine's own parts. The split matters: the
//! device models depend only on the traits, so they never name the PIC or the
//! timer wheel, while these adapters are the single place that knows how a
//! request reaches real hardware state.

use super::device_api::{DeviceCtx, IrqLine, IrqSink, TimerKey, TimerService};
use crate::pc_system::BxPcSystemC;
use crate::pic::BxPicC;

/// Build a device context over the machine's parts and run `f` with it.
///
/// The single place a [`DeviceCtx`] is assembled. The parts arrive already
/// borrowed disjointly — the caller has to split them out of the device manager
/// anyway, since the device being dispatched lives in the same struct as the
/// PIC — so this owns only the assembly, not the split.
#[inline]
pub(crate) fn with_device_ctx<R>(
    pic: &mut BxPicC,
    pc_system: &mut BxPcSystemC,
    handles: TimerHandles,
    now_ticks: u64,
    f: impl FnOnce(&mut DeviceCtx<'_>) -> R,
) -> R {
    let ips = pc_system.ips();
    let mut irq = PicIrqSink { pic };
    let mut timers = WheelTimerService {
        pc_system,
        handles,
        now_ticks,
    };
    let mut ctx = DeviceCtx {
        now_ticks,
        ips,
        irq: &mut irq,
        timers: &mut timers,
    };
    f(&mut ctx)
}

/// Routes device interrupts to the 8259 pair — Bochs `DEV_pic_raise_irq` /
/// `DEV_pic_lower_irq`.
pub(crate) struct PicIrqSink<'a> {
    pub(crate) pic: &'a mut BxPicC,
}

impl IrqSink for PicIrqSink<'_> {
    #[inline]
    fn set_level(&mut self, line: IrqLine, level: bool) {
        // The PIC enqueues any I/O APIC forward this edge implies; the
        // scheduler boundary replays it with `take_ioapic_forwards`.
        self.pic.set_irq_level(line.0, level);
    }

    #[inline]
    fn level(&self, line: IrqLine) -> bool {
        self.pic.irq_line_level(line.0)
    }
}

/// The scheduler slots one converted device owns, snapshotted before dispatch.
///
/// A device's timer handles live on the device itself, which is mutably
/// borrowed while it runs. Rather than latching requests and replaying them
/// later — the very indirection this refactor removes — the dispatcher reads
/// the handles first and hands them to the timer service, so arming stays
/// synchronous inside the device call.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TimerHandles {
    slots: [Option<usize>; TimerHandles::CAPACITY],
}

impl TimerHandles {
    /// Two timers per UART across the four possible ports.
    pub(crate) const CAPACITY: usize = 8;

    #[inline]
    pub(crate) fn set(&mut self, local: u16, handle: Option<usize>) {
        if let Some(slot) = self.slots.get_mut(local as usize) {
            *slot = handle;
        }
    }

    #[inline]
    fn get(&self, local: u16) -> Option<usize> {
        self.slots.get(local as usize).copied().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iodev::device_api::DeviceKind;
    use crate::pc_system::TimerOwner;

    /// A device arming a timer mid-batch must anchor the deadline at the tick
    /// of the access that produced it, not at the wheel's position. The wheel
    /// only advances at batch boundaries, so it lags the CPU while a batch is
    /// in flight; anchoring there would fire every device timer early by that
    /// lag. Bochs arms from inside the handler with time already current.
    #[test]
    fn deadlines_anchor_at_the_issuing_tick_not_the_wheel() {
        let mut pc_system = BxPcSystemC::new();
        pc_system.initialize(1_000_000);
        let handle = pc_system
            .register_timer(TimerOwner::Pit, 0, false, false, "test")
            .unwrap();

        // The wheel sits at 0 while the issuing access is at tick 500.
        assert_eq!(pc_system.time_ticks(), 0);
        let mut handles = TimerHandles::default();
        handles.set(0, Some(handle));
        let mut service = WheelTimerService {
            pc_system: &mut pc_system,
            handles,
            now_ticks: 500,
        };
        service.arm_oneshot_ticks(
            TimerKey {
                device: DeviceKind::Pit,
                local: 0,
            },
            7,
        );

        assert!(pc_system.timer_is_active(handle));
        assert_eq!(
            pc_system.timer_time_to_fire(handle),
            507,
            "deadline must be issuing tick + delay, not wheel tick + delay"
        );
    }
}

/// A timer service that accepts and discards every request.
///
/// For contexts with no scheduler: unit tests that exercise a device's
/// interrupt behaviour in isolation, and — once it exists — CPU-only
/// emulation, which has no machine timer wheel.
#[cfg(test)]
pub(crate) struct NullTimerService;

#[cfg(test)]

impl TimerService for NullTimerService {
    fn arm_oneshot_usec(&mut self, _key: TimerKey, _delay_usec: u64) {}
    fn arm_periodic_usec(&mut self, _key: TimerKey, _period_usec: u64) {}
    fn arm_oneshot_ticks(&mut self, _key: TimerKey, _delay_ticks: u64) {}
    fn cancel(&mut self, _key: TimerKey) {}
}

/// Arms and cancels scheduler timers on behalf of a converted device.
///
/// Deadlines are anchored to `now_ticks` — the tick count of the access being
/// serviced — and not to the wheel's current position. The two differ: the
/// wheel only advances at batch boundaries, so while a CPU batch is in flight
/// it lags the CPU's live tick count. Arming relative to the wheel would make
/// every converted device's timer fire early by that lag. Bochs has no such
/// gap, because it arms from inside the handler with time already current, so
/// anchoring at the issuing tick is what reproduces its timing.
pub(crate) struct WheelTimerService<'a> {
    pub(crate) pc_system: &'a mut BxPcSystemC,
    pub(crate) handles: TimerHandles,
    /// Tick count of the access being serviced.
    pub(crate) now_ticks: u64,
}

impl WheelTimerService<'_> {
    /// Microseconds to scheduler ticks — the same conversion the deferred
    /// request table used, so converted and unconverted devices asking for the
    /// same delay land on the same deadline.
    #[inline]
    fn usec_to_ticks(&self, delay_usec: u64) -> u64 {
        (u128::from(delay_usec) * u128::from(self.pc_system.ips()))
            .div_ceil(1_000_000)
            .max(1)
            .min(u128::from(u64::MAX)) as u64
    }

    fn arm_ticks(&mut self, key: TimerKey, delay_ticks: u64, continuous: bool) {
        let Some(handle) = self.handles.get(key.local) else {
            // The binding that built this context did not carry the device's
            // handle for `local`, so the deadline has nowhere to go. Saying so
            // is the point: a device armed a timer and the wheel never learned
            // of it, which no later symptom names.
            tracing::error!("device timer {key:?} armed with no handle bound; deadline dropped");
            return;
        };
        let deadline = self.now_ticks.saturating_add(delay_ticks);
        match self
            .pc_system
            .activate_timer_at_ticks_with_period(handle, deadline, delay_ticks, continuous)
        {
            Ok(()) => {}
            Err(error) => {
                // A device can only reach a handle the machine registered for
                // it, so this means the wheel rejected an index it issued.
                tracing::error!(
                    "device timer {:?} (handle {handle}) failed to arm: {error:?}",
                    key
                );
            }
        }
    }

    fn arm(&mut self, key: TimerKey, delay_usec: u64, continuous: bool) {
        let ticks = self.usec_to_ticks(delay_usec);
        self.arm_ticks(key, ticks, continuous);
    }
}

impl TimerService for WheelTimerService<'_> {
    fn arm_oneshot_usec(&mut self, key: TimerKey, delay_usec: u64) {
        self.arm(key, delay_usec, false);
    }

    fn arm_periodic_usec(&mut self, key: TimerKey, period_usec: u64) {
        self.arm(key, period_usec, true);
    }

    fn arm_oneshot_ticks(&mut self, key: TimerKey, delay_ticks: u64) {
        self.arm_ticks(key, delay_ticks, false);
    }

    fn cancel(&mut self, key: TimerKey) {
        let Some(handle) = self.handles.get(key.local) else {
            return;
        };
        match self.pc_system.deactivate_timer(handle) {
            Ok(()) => {}
            Err(error) => {
                tracing::error!(
                    "device timer {:?} (handle {handle}) failed to cancel: {error:?}",
                    key
                );
            }
        }
    }
}
