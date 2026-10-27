//! The generic timer's EL1 physical and virtual timers and, with FEAT_EL2,
//! the EL2 physical timer (Arm ARM D11; pseudocode
//! `AArch64_CheckTimerConditions`, `IsTimerConditionMet`,
//! `VirtualCounterTimer`).
//!
//! The system counter is a pure function of the tick count:
//! `CNTPCT = floor(icount * counter_hz / ips)`, computed in `u128` so no
//! rounding is lost and no host clock is read. The virtual count is the
//! physical count less `CNTVOFF_EL2`, which a core without EL2 does not have,
//! so there the two are equal.
//!
//! A timer condition is met when `CNTx_CTL.ENABLE` is set and the timer's
//! count (the virtual count for the virtual timer, the physical count for
//! the others) has reached `CNTx_CVAL`; its output line is the condition with
//! `IMASK` clear. The machine samples the lines whenever a timer register
//! changes and at the next tick a line can rise, which this module computes
//! exactly: the first tick whose count reaches `CVAL`.

use rusty_box_core::VmInstant;

use super::{Cpu, Levels};

/// A value of the physical count: what `CNTPCT_EL0` reads. A count, never a
/// tick (R4): [`Cpu::tick_reaching`] is the one way from a physical count to
/// the tick count.
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

/// A value of the virtual count: what `CNTVCT_EL0` reads, the physical
/// count less `CNTVOFF_EL2` modulo 2^64. Not a physical count, nor a tick
/// (R4): the processor's `tick_reaching_virtual` is the one way from a
/// virtual count to the tick count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VirtualCount(u64);

impl VirtualCount {
    #[inline]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A value of the count one timer counts in, its `CVAL`'s unit: the virtual
/// count for the virtual timer, the physical count for the others. Only
/// that timer's `CVAL` and `TVAL` meet it (R4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TimerCount(u64);

impl TimerCount {
    /// Arm `IsTimerConditionMet`: the count, unsigned, at or past `cval`.
    const fn reached(self, cval: u64) -> bool {
        self.0 >= cval
    }

    /// `TVAL` as read against `cval`: `cval` less the count, truncated to
    /// 32 bits.
    const fn tval_to(self, cval: u64) -> u64 {
        cval.wrapping_sub(self.0) & 0xFFFF_FFFF
    }

    /// The `CVAL` a write of `tval` to `TVAL` sets: the count plus the low
    /// word of `tval`, sign-extended.
    const fn plus_tval(self, tval: u64) -> u64 {
        self.0.wrapping_add(tval as u32 as i32 as i64 as u64)
    }
}

/// The timer outputs, as levels on their PPIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TimerLines {
    /// EL1 physical timer (`CNTP_*`), the non-secure physical PPI.
    pub physical: bool,
    /// Virtual timer (`CNTV_*`).
    pub virtual_: bool,
    /// EL2 physical timer (`CNTHP_*`, FEAT_EL2), the hypervisor PPI. Low on
    /// a processor without EL2.
    pub hypervisor: bool,
}

/// One of this processor's generic timers.
///
/// Closed (R5): the timers of a processor with no EL3, no FEAT_SEL2 and no
/// FEAT_VHE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GenericTimer {
    /// The EL1 physical timer, `CNTP_*_EL0`.
    Physical,
    /// The virtual timer, `CNTV_*_EL0`.
    Virtual,
    /// The EL2 physical timer, `CNTHP_*_EL2` (FEAT_EL2).
    Hypervisor,
}

impl GenericTimer {
    /// Every timer, in the order [`Cpu::timer_lines`] reports them.
    pub(crate) const ALL: [Self; 3] = [Self::Physical, Self::Virtual, Self::Hypervisor];
}

/// A timer's control and compare registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TimerRegs {
    ctl: u64,
    cval: u64,
}

pub(crate) const CTL_ENABLE: u64 = 1 << 0;
pub(crate) const CTL_IMASK: u64 = 1 << 1;
const CTL_ISTATUS: u64 = 1 << 2;

impl<L: Levels> Cpu<L> {
    /// The system counter at the tick `icount`.
    #[inline]
    fn counter_at(&self, icount: u64) -> Count {
        let value = u128::from(icount) * u128::from(self.counter_hz.get()) / u128::from(self.ips.get());
        Count(u64::try_from(value).unwrap_or(u64::MAX))
    }

    /// `CNTPCT_EL0` now (Arm `PhysicalCountInt`).
    #[inline]
    pub(crate) fn counter(&self) -> Count {
        self.counter_at(self.icount)
    }

    /// `CNTVOFF_EL2`: 0 on a processor without EL2
    /// ([`El2Regs::ABSENT`](super::el2::El2Regs::ABSENT)), where the
    /// virtual count is the physical count.
    #[inline]
    pub(crate) fn virtual_offset(&self) -> u64 {
        self.el2().cntvoff
    }

    /// `CNTVCT_EL0` now (Arm `VirtualCounterTimer`, at EL0, EL1 and EL2 of a
    /// processor without FEAT_VHE): the physical count less `CNTVOFF_EL2`,
    /// modulo 2^64.
    #[inline]
    pub(crate) fn virtual_count(&self) -> VirtualCount {
        VirtualCount(self.counter().0.wrapping_sub(self.virtual_offset()))
    }

    /// The first tick at which the counter reaches `count`. `None` when that
    /// is the last tick the count of ticks holds or past it: the machine
    /// reads `u64::MAX` as "never", and a deadline is never there.
    pub(super) fn tick_reaching(&self, count: Count) -> Option<VmInstant> {
        let needed = u128::from(count.0) * u128::from(self.ips.get());
        let ticks = needed.div_ceil(u128::from(self.counter_hz.get()));
        u64::try_from(ticks).ok().filter(|&t| t != u64::MAX).map(VmInstant::from_ticks)
    }

    /// The first tick at which the virtual count, below `target` now,
    /// reaches it: the physical count having come the same distance, short
    /// of the wrap of the virtual count, which `target` lies before. `None`
    /// when the virtual count is not below `target`, or the tick lies past
    /// [`Cpu::tick_reaching`]'s last.
    pub(super) fn tick_reaching_virtual(&self, target: VirtualCount) -> Option<VmInstant> {
        let distance = target.0.checked_sub(self.virtual_count().0).filter(|&d| d != 0)?;
        self.tick_reaching(Count(self.counter().0.checked_add(distance)?))
    }

    fn timer_regs(&self, timer: GenericTimer) -> TimerRegs {
        match timer {
            GenericTimer::Physical => TimerRegs {
                ctl: self.sys.cntp_ctl_el0,
                cval: self.sys.cntp_cval_el0,
            },
            GenericTimer::Virtual => TimerRegs {
                ctl: self.sys.cntv_ctl_el0,
                cval: self.sys.cntv_cval_el0,
            },
            GenericTimer::Hypervisor => TimerRegs {
                ctl: self.el2().cnthp_ctl,
                cval: self.el2().cnthp_cval,
            },
        }
    }

    /// The value of the count `timer` compares with its `CVAL`, which holds
    /// a value of the same count: the virtual count for the virtual timer,
    /// the physical count for the others (the `offset`
    /// `AArch64_CheckTimerConditions` passes `IsTimerConditionMet`).
    #[inline]
    fn timer_count(&self, timer: GenericTimer) -> TimerCount {
        match timer {
            GenericTimer::Virtual => TimerCount(self.virtual_count().0),
            GenericTimer::Physical | GenericTimer::Hypervisor => TimerCount(self.counter().0),
        }
    }

    /// Arm `IsTimerConditionMet` for an enabled `timer`: its count, as an
    /// unsigned 64-bit value, at or past `CVAL`.
    #[inline]
    fn condition_met(&self, timer: GenericTimer) -> bool {
        let regs = self.timer_regs(timer);
        regs.ctl & CTL_ENABLE != 0 && self.timer_count(timer).reached(regs.cval)
    }

    /// `CNTx_CTL` of `timer` as read: `ISTATUS` reports the condition,
    /// whatever `IMASK` says.
    pub(crate) fn timer_ctl_read(&self, timer: GenericTimer) -> u64 {
        let status = if self.condition_met(timer) { CTL_ISTATUS } else { 0 };
        (self.timer_regs(timer).ctl & (CTL_ENABLE | CTL_IMASK)) | status
    }

    /// `CNTx_TVAL` of `timer` as read: `CVAL` less the timer's count,
    /// truncated to 32 bits (a signed value; the upper word reads as zero).
    /// With the timer disabled the value is ARBITRARY; this is the one it
    /// reads.
    pub(crate) fn timer_tval_read(&self, timer: GenericTimer) -> u64 {
        self.timer_count(timer).tval_to(self.timer_regs(timer).cval)
    }

    /// The `CVAL` a write of `value` to `CNTx_TVAL` of `timer` sets: the
    /// timer's count plus the low word of `value`, sign-extended.
    pub(crate) fn timer_cval_from_tval(&self, timer: GenericTimer, value: u64) -> u64 {
        self.timer_count(timer).plus_tval(value)
    }

    /// Whether `timer`'s output line is high: its condition met with
    /// `IMASK` clear. The EL2 timer's is low for good on a processor
    /// without EL2, its `CTL` 0 ([`El2Regs`](super::el2::El2Regs)).
    fn timer_line(&self, timer: GenericTimer) -> bool {
        self.timer_regs(timer).ctl & CTL_IMASK == 0 && self.condition_met(timer)
    }

    /// Every timer output now.
    pub fn timer_lines(&self) -> TimerLines {
        TimerLines {
            physical: self.timer_line(GenericTimer::Physical),
            virtual_: self.timer_line(GenericTimer::Virtual),
            hypervisor: self.timer_line(GenericTimer::Hypervisor),
        }
    }

    /// The first tick at which `timer`'s output changes without further
    /// register writes. Low, it rises where its count reaches `CVAL`. High,
    /// only the virtual timer's can fall: the virtual count wraps to zero
    /// where the physical count reaches `CNTVOFF_EL2`, which takes it below
    /// a nonzero `CVAL` again, `IsTimerConditionMet` comparing them
    /// unsigned.
    fn timer_change(&self, timer: GenericTimer) -> Option<VmInstant> {
        let regs = self.timer_regs(timer);
        if regs.ctl & CTL_ENABLE == 0 || regs.ctl & CTL_IMASK != 0 {
            return None;
        }
        match timer {
            GenericTimer::Physical | GenericTimer::Hypervisor => {
                let cval = Count(regs.cval);
                if self.counter() < cval {
                    self.tick_reaching(cval)
                } else {
                    None
                }
            }
            GenericTimer::Virtual => {
                let cval = VirtualCount(regs.cval);
                if self.virtual_count() < cval {
                    return self.tick_reaching_virtual(cval);
                }
                let offset = Count(self.virtual_offset());
                if self.counter() < offset && regs.cval != 0 {
                    self.tick_reaching(offset)
                } else {
                    None
                }
            }
        }
    }

    /// The earliest future tick at which a timer output changes, if one
    /// will without further register writes.
    pub fn next_timer_deadline(&self) -> Option<VmInstant> {
        GenericTimer::ALL.into_iter().filter_map(|timer| self.timer_change(timer)).min()
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
    use crate::cpu::{CpuConfig, NoEl2, SysReg, SysRegRefusal, WithEl2};
    use rusty_box_core::ClockHz;

    fn config(ips: u64, counter_hz: u64) -> CpuConfig {
        CpuConfig {
            ips: ClockHz::new(ips).unwrap_or(ClockHz::BOCHS_DEFAULT),
            counter_hz: ClockHz::new(counter_hz).unwrap_or(ClockHz::BOCHS_DEFAULT),
            reset_pc: 0,
        }
    }

    fn cpu(ips: u64, counter_hz: u64) -> Cpu<NoEl2> {
        Cpu::new(config(ips, counter_hz))
    }

    fn cpu_el2(ips: u64, counter_hz: u64) -> Cpu<WithEl2> {
        Cpu::new(config(ips, counter_hz))
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
        assert_eq!(c.timer_ctl_read(GenericTimer::Virtual) & CTL_ISTATUS, CTL_ISTATUS);
        assert!(!c.timer_lines().virtual_);
    }

    /// TVAL is a signed 32-bit view of CVAL - count.
    #[test]
    fn tval_round_trips_through_cval() {
        let mut c = cpu(1_000, 1_000);
        c.icount = 100;
        let cval = c.timer_cval_from_tval(GenericTimer::Physical, u64::from((-10i32) as u32));
        assert_eq!(cval, 90);
        c.sys.cntp_cval_el0 = cval;
        assert_eq!(c.timer_tval_read(GenericTimer::Physical), u64::from((-10i32) as u32));
    }

    /// The virtual timer compares the physical count less `CNTVOFF_EL2`
    /// (`IsTimerConditionMet(CNTVOFF_EL2, ..)`): an offset of 30 makes a
    /// CVAL of 20 fire at physical count 50, and TVAL reads the virtual
    /// distance.
    #[test]
    fn the_virtual_timer_counts_from_the_virtual_offset() {
        let mut c = cpu_el2(1_000, 1_000);
        c.el2_state.regs.cntvoff = 30;
        c.sys.cntv_ctl_el0 = CTL_ENABLE;
        c.sys.cntv_cval_el0 = 20;
        c.icount = 40;
        assert_eq!(c.virtual_count(), VirtualCount(10));
        assert_eq!(c.timer_tval_read(GenericTimer::Virtual), 10);
        assert!(!c.timer_lines().virtual_);
        assert_eq!(c.next_timer_deadline().map(VmInstant::ticks), Some(50));
        c.icount = 50;
        assert!(c.timer_lines().virtual_);
        assert_eq!(c.timer_cval_from_tval(GenericTimer::Virtual, 5), 25, "the virtual count 20 plus 5");
    }

    /// An offset past the physical count makes the virtual count wrap:
    /// near 2^64 it is at or past a small CVAL, so the line is high; where
    /// the physical count reaches the offset the virtual count wraps to
    /// zero and the line falls, and it rises again where the virtual count
    /// reaches CVAL. A CVAL of zero is met at every count, so nothing falls.
    #[test]
    fn the_virtual_line_falls_where_the_virtual_count_wraps() {
        let mut c = cpu_el2(1_000, 1_000);
        c.el2_state.regs.cntvoff = 1_000;
        c.sys.cntv_ctl_el0 = CTL_ENABLE;
        c.sys.cntv_cval_el0 = 100;
        c.icount = 500;
        assert_eq!(c.virtual_count(), VirtualCount(0u64.wrapping_sub(500)));
        assert!(c.timer_lines().virtual_);
        assert_eq!(c.next_timer_deadline().map(VmInstant::ticks), Some(1_000));
        c.icount = 1_000;
        assert!(!c.timer_lines().virtual_);
        assert_eq!(c.next_timer_deadline().map(VmInstant::ticks), Some(1_100));
        c.icount = 1_100;
        assert!(c.timer_lines().virtual_);
        assert_eq!(c.next_timer_deadline(), None);

        c.icount = 500;
        c.sys.cntv_cval_el0 = 0;
        assert!(c.timer_lines().virtual_);
        assert_eq!(c.next_timer_deadline(), None);
    }

    /// The EL2 physical timer drives its own line from the physical count.
    /// A processor without EL2 has no such timer: its registers refuse a
    /// host's write, and the line stays low.
    #[test]
    fn the_el2_timer_has_its_own_line() {
        let mut c = cpu_el2(1_000, 1_000);
        c.el2_state.regs.cnthp_ctl = CTL_ENABLE;
        c.el2_state.regs.cnthp_cval = 7;
        assert_eq!(c.next_timer_deadline().map(VmInstant::ticks), Some(7));
        c.icount = 7;
        assert_eq!(
            c.timer_lines(),
            TimerLines {
                physical: false,
                virtual_: false,
                hypervisor: true,
            }
        );
        let mut plain = cpu(1_000, 1_000);
        for (reg, value) in [(SysReg::CNTHP_CVAL_EL2, 7), (SysReg::CNTHP_CTL_EL2, CTL_ENABLE)] {
            assert_eq!(plain.host_sysreg_write(reg, value), Err(SysRegRefusal::Absent), "{reg:?}");
        }
        plain.icount = 7;
        assert!(!plain.timer_lines().hypervisor);
        assert_eq!(plain.next_timer_deadline(), None);
    }
}
