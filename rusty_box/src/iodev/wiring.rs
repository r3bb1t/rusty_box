//! Machine-side implementations of the device-facing capability traits.
//!
//! [`device_api`](super::device_api) states what a device may ask for; this
//! module supplies it from the machine's own parts. The split matters: the
//! device models depend only on the traits, so they never name the PIC or the
//! timer wheel, while these adapters are the single place that knows how a
//! request reaches real hardware state.

use super::device_api::{IrqLine, IrqSink, TimerKey, TimerService};
use crate::pc_system::BxPcSystemC;
use crate::pic::BxPicC;

/// Routes device interrupts to the 8259 pair — Bochs `DEV_pic_raise_irq` /
/// `DEV_pic_lower_irq`.
pub(crate) struct PicIrqSink<'a> {
    pub(crate) pic: &'a mut BxPicC,
}

impl IrqSink for PicIrqSink<'_> {
    #[inline]
    fn set_level(&mut self, line: IrqLine, level: bool) {
        match self.pic.set_irq_level(line.0, level) {
            // The PIC has already enqueued this forward for the scheduler
            // boundary to replay into the I/O APIC (`take_ioapic_forwards`).
            // The returned copy is a convenience for callers that drive the
            // APIC themselves; consuming it here would deliver it twice.
            Some(_queued_ioapic_forward) => {}
            None => {}
        }
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

/// Arms and cancels scheduler timers on behalf of a converted device.
pub(crate) struct WheelTimerService<'a> {
    pub(crate) pc_system: &'a mut BxPcSystemC,
    pub(crate) handles: TimerHandles,
}

impl WheelTimerService<'_> {
    /// The wheel takes a `u32` microsecond delay. Saturating is safe rather
    /// than merely convenient: a delay past `u32::MAX` microseconds (~71
    /// minutes) is far beyond any device's re-arm interval, so clamping can
    /// only ever fire a stale one-shot late, which every `*_timer_fired`
    /// handler already treats as a cancelled callback.
    #[inline]
    fn clamp_usec(delay_usec: u64) -> u32 {
        delay_usec.min(u32::MAX as u64) as u32
    }
}

impl WheelTimerService<'_> {
    fn arm(&mut self, key: TimerKey, delay_usec: u64, continuous: bool) {
        let Some(handle) = self.handles.get(key.local) else {
            return;
        };
        match self
            .pc_system
            .activate_timer_usec(handle, Self::clamp_usec(delay_usec), continuous)
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
}

impl TimerService for WheelTimerService<'_> {
    fn arm_oneshot_usec(&mut self, key: TimerKey, delay_usec: u64) {
        self.arm(key, delay_usec, false);
    }

    fn arm_periodic_usec(&mut self, key: TimerKey, period_usec: u64) {
        self.arm(key, period_usec, true);
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
