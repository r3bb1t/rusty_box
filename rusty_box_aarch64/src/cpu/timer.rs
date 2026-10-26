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

use super::Cpu;

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

/// `CNTKCTL_EL1` event stream controls.
const EVNTEN: u64 = 1 << 2;
const EVNTDIR: u64 = 1 << 3;
const EVNTI_SHIFT: u64 = 4;

/// The Event Register, which `WFE` consumes (Arm ARM `EventRegister`).
/// `SEV`, `SEVL` and every exception return set it (`SendEventLocal`), and
/// so does each event the virtual event stream sends (`TestEventCNTV`).
/// The stream's events are worked out when the register is read, from the
/// counter values it passed since they were last accounted for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EventRegister {
    /// Set by an event already accounted for.
    pub(crate) set: bool,
    /// The count after which the event stream's events are not yet
    /// accounted for.
    pub(crate) stream_from: u64,
}

impl EventRegister {
    /// Clear, with the stream's events accounted for up to `count`.
    pub(crate) const fn clear_at(count: u64) -> Self {
        Self {
            set: false,
            stream_from: count,
        }
    }
}

/// Whether the virtual event stream `cntkctl` configures sends an event as
/// the counter runs from `from` to `to`: whether bit `EVNTI` of some count
/// in `from + 1..=to` makes the transition `EVNTDIR` selects from the count
/// before it (Arm ARM `TestEventCNTV`, for every count in between; this core
/// has no EL2, so no `CNTVOFF_EL2` offset and no `HCR_EL2` override).
pub(crate) fn event_stream_fired(cntkctl: u64, from: u64, to: u64) -> bool {
    if cntkctl & EVNTEN == 0 || to <= from {
        return false;
    }
    let n = (cntkctl >> EVNTI_SHIFT) & 0xF;
    let period = 1u128 << (n + 1);
    // Bit n rises at the counts `2^n` past a multiple of `2^(n+1)`, and
    // falls at the multiples themselves.
    let edge = if cntkctl & EVNTDIR == 0 { 1u128 << n } else { 0 };
    let from = u128::from(from);
    let mut first = from - from % period + edge;
    if first <= from {
        first += period;
    }
    first <= u128::from(to)
}

impl Cpu {
    /// Whether the Event Register is set.
    pub(crate) fn event_register_set(&self) -> bool {
        self.event.set || event_stream_fired(self.sys.cntkctl_el1, self.event.stream_from, self.counter())
    }

    /// `SendEventLocal`: set the Event Register.
    pub(crate) fn send_event_local(&mut self) {
        self.event.set = true;
    }

    /// `WFE`'s use of the Event Register: if it is set, clear it and say so.
    pub(crate) fn take_event(&mut self) -> bool {
        let set = self.event_register_set();
        if set {
            self.event = EventRegister::clear_at(self.counter());
        }
        set
    }

    /// Account for the event stream's events so far, ahead of a
    /// `CNTKCTL_EL1` write that may change what the stream sends.
    pub(crate) fn settle_event_stream(&mut self) {
        self.event = EventRegister {
            set: self.event_register_set(),
            stream_from: self.counter(),
        };
    }
    /// The system counter at the tick `icount`: non-decreasing in `icount`.
    #[inline]
    pub(crate) fn counter_at(&self, icount: u64) -> u64 {
        let value = u128::from(icount) * u128::from(self.counter_hz.get()) / u128::from(self.ips.get());
        u64::try_from(value).unwrap_or(u64::MAX)
    }

    /// `CNTPCT_EL0` now.
    #[inline]
    pub(crate) fn counter(&self) -> u64 {
        self.counter_at(self.icount)
    }

    /// The first tick at which the counter reaches `cval`.
    fn tick_reaching(&self, cval: u64) -> u64 {
        let needed = u128::from(cval) * u128::from(self.ips.get());
        let ticks = needed.div_ceil(u128::from(self.counter_hz.get()));
        u64::try_from(ticks).unwrap_or(u64::MAX)
    }

    #[inline]
    fn condition_met(&self, ctl: u64, cval: u64) -> bool {
        ctl & CTL_ENABLE != 0 && self.counter() >= cval
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
        cval.wrapping_sub(self.counter()) & 0xFFFF_FFFF
    }

    /// The `CVAL` a write of `value` to `CNTx_TVAL_EL0` sets: the count plus
    /// the low word of `value`, sign-extended.
    pub(crate) fn timer_cval_from_tval(&self, value: u64) -> u64 {
        self.counter().wrapping_add(value as u32 as i32 as i64 as u64)
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
    pub fn next_timer_deadline(&self) -> Option<u64> {
        let rise = |ctl: u64, cval: u64| {
            if ctl & CTL_ENABLE != 0 && ctl & CTL_IMASK == 0 && self.counter() < cval {
                Some(self.tick_reaching(cval))
            } else {
                None
            }
        };
        let p = rise(self.sys.cntp_ctl_el0, self.sys.cntp_cval_el0);
        let v = rise(self.sys.cntv_ctl_el0, self.sys.cntv_cval_el0);
        match (p, v) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, None) => a,
            (None, b) => b,
        }
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
    use crate::cpu::CpuConfig;
    use rusty_box_core::ClockHz;

    fn cpu(ips: u64, counter_hz: u64) -> Cpu {
        Cpu::new(CpuConfig {
            ips: ClockHz::new(ips).unwrap_or(ClockHz::BOCHS_DEFAULT),
            counter_hz: ClockHz::new(counter_hz).unwrap_or(ClockHz::BOCHS_DEFAULT),
            reset_pc: 0,
        })
    }

    /// The deadline is the first tick at which the guest would read a count
    /// at or past CVAL, never one tick early or late — at a rate where the
    /// counter does not advance every tick.
    #[test]
    fn the_deadline_is_exactly_the_first_tick_reaching_cval() {
        let mut c = cpu(50_000_000, 62_500_000);
        c.sys.cntp_ctl_el0 = CTL_ENABLE;
        c.sys.cntp_cval_el0 = 1_000_003;
        let deadline = c.next_timer_deadline().unwrap_or(0);
        c.icount = deadline - 1;
        assert!(c.counter() < 1_000_003 && !c.timer_lines().physical);
        c.icount = deadline;
        assert!(c.counter() >= 1_000_003 && c.timer_lines().physical);
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

    /// The stream fires on the chosen edge of bit EVNTI and on no other
    /// count, however far the counter runs at once.
    #[test]
    fn the_event_stream_fires_on_its_chosen_edge_only() {
        // EVNTI 2: bit 2 rises at counts 4, 12, 20, ... and falls at 8, 16, ...
        let rising = EVNTEN | (2 << EVNTI_SHIFT);
        let falling = rising | EVNTDIR;
        assert!(event_stream_fired(rising, 3, 4));
        assert!(!event_stream_fired(rising, 4, 11));
        assert!(event_stream_fired(rising, 4, 12));
        assert!(!event_stream_fired(falling, 0, 7));
        assert!(event_stream_fired(falling, 7, 8));
        assert!(!event_stream_fired(falling, 8, 15));
        assert!(event_stream_fired(falling, 8, 1 << 40));
        assert!(!event_stream_fired(rising & !EVNTEN, 0, 1 << 40), "EVNTEN clear sends nothing");
        assert!(!event_stream_fired(rising, 12, 12), "no count passed");
        // EVNTI 15, from near the top of the counter.
        let top = EVNTEN | (15 << EVNTI_SHIFT);
        assert!(event_stream_fired(top, u64::MAX - (1 << 16), u64::MAX - (1 << 15) + 1));
    }

    /// WFE consumes the register, which the stream sets again only at its
    /// next event; SEVL sets it whatever the stream does.
    #[test]
    fn wfe_consumes_the_event_register() {
        let mut c = cpu(1_000, 1_000);
        c.sys.cntkctl_el1 = EVNTEN | (3 << EVNTI_SHIFT);
        assert!(!c.take_event(), "no event before the count reaches 8");
        c.icount = 8;
        assert!(c.take_event(), "bit 3 rose at count 8");
        assert!(!c.take_event(), "consumed");
        c.icount = 23;
        assert!(!c.take_event(), "the next rise is at 24");
        c.send_event_local();
        assert!(c.take_event());
        c.icount = 24;
        assert!(c.take_event());
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
