//! The generic timer's EL1 physical and virtual timers (Arm ARM D11).
//!
//! The system counter is a pure function of the tick count:
//! `CNTPCT = floor(icount * counter_hz / ips)`, computed in `u128` so no
//! rounding is lost and no host clock is read. `CNTVOFF_EL2` does not exist on
//! a core without EL2, so the virtual count equals the physical count.
//!
//! A timer condition is met when `CNTx_CTL.ENABLE` is set and the count has
//! reached `CNTx_CVAL`; its output line is the condition with `IMASK` clear.
//! The machine samples the lines whenever a timer register changes and at the
//! next tick a line can rise, which this module computes exactly: the first
//! tick whose count reaches `CVAL`.

use rusty_box_core::{Cycles, VmInstant};

use super::Cpu;

/// A value of the system counter: what `CNTPCT_EL0` reads, and with no EL2
/// `CNTVCT_EL0` too. A count, never a tick (R4): [`Cpu::tick_reaching`] is
/// the one way from a count to the tick count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Count(u64);

impl Count {
    #[inline]
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    #[inline]
    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

/// The timer outputs, as levels on their PPIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TimerLines {
    /// EL1 physical timer (`CNTP_*`), the non-secure physical PPI.
    pub physical: bool,
    /// Virtual timer (`CNTV_*`).
    pub virtual_: bool,
}

const CTL_ENABLE: u64 = 1 << 0;
const CTL_IMASK: u64 = 1 << 1;
const CTL_ISTATUS: u64 = 1 << 2;

impl Cpu {
    /// The system counter at the tick `icount`: non-decreasing in `icount`.
    #[inline]
    pub(crate) fn counter_at(&self, icount: u64) -> Count {
        let value = u128::from(icount) * u128::from(self.imp.counter_hz.get()) / u128::from(self.ips.get());
        Count(u64::try_from(value).unwrap_or(u64::MAX))
    }

    /// `CNTPCT_EL0` now.
    #[inline]
    pub(crate) fn counter(&self) -> Count {
        self.counter_at(self.icount)
    }

    /// The first tick at which the counter reaches `count`: the ticks that
    /// span `count` counter cycles, rounded up, which is exactly where
    /// [`Cpu::counter_at`]'s floor first reaches it. `None` when that is the
    /// last tick the count of ticks holds or past it (the span saturates
    /// there): the machine reads `u64::MAX` as "never", and a deadline is
    /// never there.
    pub(super) fn tick_reaching(&self, count: Count) -> Option<VmInstant> {
        let ticks = self
            .ips
            .ticks_from_cycles_ceil(Cycles::new(count.0), self.imp.counter_hz)
            .ticks();
        (ticks != u64::MAX).then(|| VmInstant::from_ticks(ticks))
    }

    #[inline]
    fn condition_met(&self, ctl: u64, cval: u64) -> bool {
        ctl & CTL_ENABLE != 0 && self.counter() >= Count(cval)
    }

    /// `CNTx_CTL_EL0` as read: `ISTATUS` reports the condition, whatever
    /// `IMASK` says.
    pub(crate) fn timer_ctl_read(&self, ctl: u64, cval: u64) -> u64 {
        let status = if self.condition_met(ctl, cval) { CTL_ISTATUS } else { 0 };
        (ctl & (CTL_ENABLE | CTL_IMASK)) | status
    }

    /// `CNTx_TVAL_EL0` as read: `CVAL - count`, truncated to 32 bits (a
    /// signed value; the upper word reads as zero).
    pub(crate) fn timer_tval_read(&self, cval: u64) -> u64 {
        cval.wrapping_sub(self.counter().0) & 0xFFFF_FFFF
    }

    /// The `CVAL` a write of `value` to `CNTx_TVAL_EL0` sets: the count plus
    /// the low word of `value`, sign-extended.
    pub(crate) fn timer_cval_from_tval(&self, value: u64) -> u64 {
        self.counter().0.wrapping_add(value as u32 as i32 as i64 as u64)
    }

    /// Both timer outputs now.
    pub fn timer_lines(&self) -> TimerLines {
        let line = |ctl: u64, cval: u64| ctl & CTL_IMASK == 0 && self.condition_met(ctl, cval);
        TimerLines {
            physical: line(self.sys.cntp_ctl_el0, self.sys.cntp_cval_el0),
            virtual_: line(self.sys.cntv_ctl_el0, self.sys.cntv_cval_el0),
        }
    }

    /// The earliest future tick at which a timer output rises, if one will
    /// without further register writes.
    pub fn next_timer_deadline(&self) -> Option<VmInstant> {
        let rise = |ctl: u64, cval: u64| {
            if ctl & CTL_ENABLE != 0 && ctl & CTL_IMASK == 0 && self.counter() < Count(cval) {
                self.tick_reaching(Count(cval))
            } else {
                None
            }
        };
        let p = rise(self.sys.cntp_ctl_el0, self.sys.cntp_cval_el0);
        let v = rise(self.sys.cntv_ctl_el0, self.sys.cntv_cval_el0);
        p.into_iter().chain(v).min()
    }

    /// Whether a timer register changed since the machine last sampled the
    /// outputs; clears the flag.
    pub fn take_timer_dirty(&mut self) -> bool {
        core::mem::replace(&mut self.timer_dirty, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::features::Implementation;
    use crate::cpu::CpuConfig;
    use rusty_box_core::ClockHz;

    fn cpu(ips: u64, counter_hz: u64) -> Cpu {
        Cpu::implementing(
            CpuConfig {
                ips: ClockHz::new(ips).unwrap_or(ClockHz::BOCHS_DEFAULT),
                reset_pc: 0,
            },
            Implementation {
                counter_hz: ClockHz::new(counter_hz).unwrap_or(ClockHz::BOCHS_DEFAULT),
                ..Implementation::CORTEX_A57
            },
        )
    }

    /// The deadline is the first tick at which the guest would read a count
    /// at or past CVAL, never one tick early or late — at a rate where the
    /// counter does not advance every tick.
    #[test]
    fn the_deadline_is_exactly_the_first_tick_reaching_cval() {
        let mut c = cpu(50_000_000, 62_500_000);
        c.sys.cntp_ctl_el0 = CTL_ENABLE;
        c.sys.cntp_cval_el0 = 1_000_003;
        let deadline = c.next_timer_deadline().map_or(0, VmInstant::ticks);
        c.icount = deadline - 1;
        assert!(c.counter() < Count(1_000_003) && !c.timer_lines().physical);
        c.icount = deadline;
        assert!(c.counter() >= Count(1_000_003) && c.timer_lines().physical);
    }

    /// IMASK hides the line but not ISTATUS, and a masked timer has no
    /// deadline because its line cannot rise.
    #[test]
    fn a_masked_timer_reports_status_without_a_line() {
        let mut c = cpu(1_000, 1_000);
        c.sys.cntv_ctl_el0 = CTL_ENABLE | CTL_IMASK;
        c.sys.cntv_cval_el0 = 5;
        assert_eq!(c.next_timer_deadline(), None);
        c.icount = 5;
        assert_eq!(c.timer_ctl_read(c.sys.cntv_ctl_el0, 5) & CTL_ISTATUS, CTL_ISTATUS);
        assert!(!c.timer_lines().virtual_);
    }

    /// TVAL is a signed 32-bit view of CVAL - count.
    #[test]
    fn tval_round_trips_through_cval() {
        let mut c = cpu(1_000, 1_000);
        c.icount = 100;
        let cval = c.timer_cval_from_tval(u64::from((-10i32) as u32));
        assert_eq!(cval, 90);
        assert_eq!(c.timer_tval_read(cval), u64::from((-10i32) as u32));
    }
}
