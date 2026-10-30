//! The generic timer's EL1 physical and virtual timers and, with FEAT_EL2,
//! the EL2 physical timer (Arm ARM D11; pseudocode
//! `AArch64_CheckTimerConditions`, `IsTimerConditionMet`,
//! `VirtualCounterTimer`).
//!
//! The system counter is a pure function of the tick count:
//! `CNTPCT = floor(icount * counter_hz / ips)`, computed in `u128` so no
//! rounding is lost and no host clock is read. The virtual count is the
//! physical count less `CNTVOFF_EL2`, which a core without EL2 does not have,
//! so there the two are equal. With FEAT_ECV_POFF and `CNTHCTL_EL2.ECV` set,
//! EL0 and EL1 read the physical count less `CNTPOFF_EL2`, and the EL1
//! physical timer compares that count.
//!
//! A timer condition is met when `CNTx_CTL.ENABLE` is set and the timer's
//! count — the physical count less the timer's offset: `CNTVOFF_EL2` for the
//! virtual timer, the physical offset for the EL1 physical timer, none for
//! the EL2 timer — has reached `CNTx_CVAL`; its output line is the condition
//! with `IMASK` clear. The machine samples the lines whenever a timer
//! register changes and at the next tick a line can change, which this
//! module computes exactly: the first tick whose count reaches `CVAL`, or
//! whose offset count wraps to zero.

use rusty_box_core::{Cycles, VmInstant};

use super::el2::CNTHCTL_ECV_POFF;
use super::features::Feature;
use super::{Cpu, El, Levels};

/// A value of the physical count (Arm `PhysicalCountInt`): what `CNTPCT_EL0`
/// reads at EL2, and at EL0 and EL1 but under FEAT_ECV_POFF's offset
/// ([`PhysicalView`]). A count, never a tick (R4): [`Cpu::tick_reaching`] is
/// the one way from a physical count to the tick count.
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

/// A value of the physical count as a level reads it: what `CNTPCT_EL0` and
/// `CNTPCTSS_EL0` read, the physical count less the physical offset at EL0
/// and EL1 ([`Cpu::physical_offset`]), modulo 2^64, and the physical count
/// itself at EL2. A [`Count`] only as EL2 reads it, and never a tick (R4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PhysicalView(u64);

impl PhysicalView {
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

/// A value of the count one timer counts in, its `CVAL`'s unit: the
/// physical count less the timer's offset ([`Cpu::timer_offset`]). Only
/// that timer's `CVAL` meets it; a `TVAL` access meets a [`TvalCount`]
/// (R4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TimerCount(u64);

impl TimerCount {
    /// Arm `IsTimerConditionMet`: the count, unsigned, at or past `cval`.
    const fn reached(self, cval: u64) -> bool {
        self.0 >= cval
    }
}

/// The offset a timer's count is taken by from the physical count
/// (`AArch64_CheckTimerConditions`'s `offset`): a distance in counts, not
/// a count (R4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CountOffset(u64);

impl CountOffset {
    /// No offset: the count is the physical count.
    pub(crate) const NONE: Self = Self(0);

    /// The distance, in counts.
    #[inline]
    pub(crate) const fn get(self) -> u64 {
        self.0
    }

    /// The offset count while the physical count is `now`: `now` less the
    /// offset, modulo 2^64, for its caller to name the count it is.
    #[inline]
    const fn from(self, now: Count) -> u64 {
        now.0.wrapping_sub(self.0)
    }

    /// The timer's count while the physical count is `now`
    /// ([`CountOffset::from`]).
    #[inline]
    const fn count_at(self, now: Count) -> TimerCount {
        TimerCount(self.from(now))
    }

    /// The physical count at which the timer's count wraps to zero: the
    /// one the offset equals.
    const fn wrap(self) -> Count {
        Count(self.0)
    }
}

/// What the timers read of the EL2 registers held as written: the offsets
/// of the counts the EL1 timers compare (`CNTVOFF_EL2` the virtual
/// timer's, the physical offset, [`Cpu::physical_offset`], the physical
/// timer's) and the EL2 timer's `CNTHP_CVAL_EL2`. A write of one of those
/// registers that changes it may move a timer: an enabled one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct El2TimerInputs {
    virtual_offset: CountOffset,
    physical_offset: CountOffset,
    hypervisor_cval: TimerCount,
}

/// A value of the count a `TVAL` access is made against
/// ([`Cpu::tval_count`]): the timer's count, but for the EL1 physical
/// timer's `TVAL` at EL2, the physical count itself. Only that access's
/// `TVAL` and its timer's `CVAL` meet it (R4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TvalCount(u64);

impl TvalCount {
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

/// A timer's `TVAL` as an access reaches it. Each is read and written
/// against a count ([`Cpu::tval_count`]); only the EL1 physical timer's
/// depends on the level the access comes from, which its access carries.
///
/// Closed (R5): one per [`GenericTimer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tval {
    /// `CNTP_TVAL_EL0`, accessed at the level.
    Physical(El),
    /// `CNTV_TVAL_EL0`.
    Virtual,
    /// `CNTHP_TVAL_EL2`.
    Hypervisor,
}

impl Tval {
    /// The timer whose `TVAL` this is.
    const fn timer(self) -> GenericTimer {
        match self {
            Self::Physical(_) => GenericTimer::Physical,
            Self::Virtual => GenericTimer::Virtual,
            Self::Hypervisor => GenericTimer::Hypervisor,
        }
    }
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
    /// The system counter at the tick `icount`: non-decreasing in `icount`.
    #[inline]
    pub(crate) fn counter_at(&self, icount: u64) -> Count {
        let value =
            u128::from(icount) * u128::from(self.imp.counter_hz.get()) / u128::from(self.ips.get());
        Count(u64::try_from(value).unwrap_or(u64::MAX))
    }

    /// The physical count now (Arm `PhysicalCountInt`), which `CNTPCT_EL0`
    /// reads at EL2 ([`Cpu::physical_count_read`] at each level).
    #[inline]
    pub(crate) fn counter(&self) -> Count {
        self.counter_at(self.icount)
    }

    /// `CNTVOFF_EL2`: 0 on a processor without EL2
    /// ([`El2Regs::ABSENT`](super::el2::El2Regs::ABSENT)), where the
    /// virtual count is the physical count.
    #[inline]
    pub(crate) fn virtual_offset(&self) -> CountOffset {
        CountOffset(self.el2().cntvoff)
    }

    /// The physical count's offset in EL0's and EL1's view of it, which the
    /// EL1 physical timer compares too (Arm `PhysicalOffsetIsValid`, and
    /// `AArch64_CheckTimerConditions`'s offset, with no EL3 and no
    /// FEAT_VHE): `CNTPOFF_EL2` on a processor with FEAT_ECV_POFF, which
    /// has EL2, while `CNTHCTL_EL2.ECV` is set; otherwise 0.
    #[inline]
    pub(crate) fn physical_offset(&self) -> CountOffset {
        let el2 = self.el2();
        if self.implements(Feature::EcvPoff) && el2.cnthctl & CNTHCTL_ECV_POFF != 0 {
            CountOffset(el2.cntpoff)
        } else {
            CountOffset::NONE
        }
    }

    /// What the timers read of the EL2 registers now.
    pub(crate) fn el2_timer_inputs(&self) -> El2TimerInputs {
        El2TimerInputs {
            virtual_offset: self.virtual_offset(),
            physical_offset: self.physical_offset(),
            hypervisor_cval: TimerCount(self.el2().cnthp_cval),
        }
    }

    /// `CNTPCT_EL0` and `CNTPCTSS_EL0` as an access at `el` reads them at
    /// the tick `icount`: the physical count less [`Cpu::physical_offset`]
    /// at EL0 and EL1, modulo 2^64, and the physical count itself at EL2.
    /// Non-decreasing in `icount` but where an offset count wraps to 0.
    ///
    /// The EL1 physical timer's count ([`Cpu::timer_count_at`]) takes the
    /// same offset at every level instead: Arm's accessors decide by the
    /// accessing level whether to subtract `CNTPOFF_EL2`, and
    /// `AArch64_CheckTimerConditions` by `PhysicalOffsetIsValid` alone,
    /// which reads no level. So the two are kept apart, both taking
    /// [`Cpu::physical_offset`] as the one offset there is.
    #[inline]
    pub(crate) fn physical_count_at(&self, el: El, icount: u64) -> PhysicalView {
        let offset = match el {
            El::El0 | El::El1 => self.physical_offset(),
            El::El2 => CountOffset::NONE,
        };
        PhysicalView(offset.from(self.counter_at(icount)))
    }

    /// `CNTPCT_EL0` and `CNTPCTSS_EL0` as an access at `el` reads them now
    /// ([`Cpu::physical_count_at`]).
    #[inline]
    pub(crate) fn physical_count_read(&self, el: El) -> PhysicalView {
        self.physical_count_at(el, self.icount)
    }

    /// `CNTVCT_EL0` at the tick `icount` (Arm `VirtualCounterTimer`, at
    /// EL0, EL1 and EL2 of a processor without FEAT_VHE): the physical count
    /// less `CNTVOFF_EL2`, modulo 2^64. Non-decreasing in `icount` but where
    /// it wraps to 0.
    #[inline]
    pub(crate) fn virtual_count_at(&self, icount: u64) -> VirtualCount {
        VirtualCount(self.virtual_offset().from(self.counter_at(icount)))
    }

    /// `CNTVCT_EL0` now ([`Cpu::virtual_count_at`]).
    #[inline]
    pub(crate) fn virtual_count(&self) -> VirtualCount {
        self.virtual_count_at(self.icount)
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

    /// The first tick at which a timer's count, `count` while the physical
    /// count is `now`, reaches `target`, which it lies below: the physical
    /// count having come the same distance, short of the timer count's wrap,
    /// which `target` lies before. `None` when `count` is not below
    /// `target`, or the tick lies past [`Cpu::tick_reaching`]'s last.
    fn tick_reaching_count(
        &self,
        now: Count,
        count: TimerCount,
        target: TimerCount,
    ) -> Option<VmInstant> {
        let distance = target.0.checked_sub(count.0).filter(|&d| d != 0)?;
        self.tick_reaching(Count(now.0.checked_add(distance)?))
    }

    /// The first tick at which the virtual count, below `target` now,
    /// reaches it ([`Cpu::tick_reaching_count`]: the virtual count is the
    /// virtual timer's count).
    pub(super) fn tick_reaching_virtual(&self, target: VirtualCount) -> Option<VmInstant> {
        let now = self.counter();
        let count = self.timer_count_at(GenericTimer::Virtual, now);
        self.tick_reaching_count(now, count, TimerCount(target.0))
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

    /// The offset of the count `timer` compares with its `CVAL` from the
    /// physical count (the `offset` `AArch64_CheckTimerConditions` passes
    /// `IsTimerConditionMet`): `CNTVOFF_EL2` for the virtual timer, the
    /// physical offset for the EL1 physical timer, none for the EL2 timer.
    #[inline]
    fn timer_offset(&self, timer: GenericTimer) -> CountOffset {
        match timer {
            GenericTimer::Virtual => self.virtual_offset(),
            GenericTimer::Physical => self.physical_offset(),
            GenericTimer::Hypervisor => CountOffset::NONE,
        }
    }

    /// The value of the count `timer` compares with its `CVAL`, which holds
    /// a value of the same count, while the physical count is `now`: the
    /// physical count less the timer's offset, modulo 2^64.
    #[inline]
    fn timer_count_at(&self, timer: GenericTimer, now: Count) -> TimerCount {
        self.timer_offset(timer).count_at(now)
    }

    /// The count `timer` compares now ([`Cpu::timer_count_at`]).
    #[inline]
    fn timer_count(&self, timer: GenericTimer) -> TimerCount {
        self.timer_count_at(timer, self.counter())
    }

    /// The count `tval` is read and written against: the EL1 physical
    /// timer's is the physical count as the accessing level reads it
    /// ([`Cpu::physical_count_read`]), which at EL2 is not offset though the
    /// timer's condition is (the `CNTP_TVAL_EL0` accessors offset it at EL0
    /// and EL1 alone); the other timers' is the timer's count.
    #[inline]
    fn tval_count(&self, tval: Tval) -> TvalCount {
        TvalCount(match tval {
            Tval::Physical(el) => self.physical_count_read(el).get(),
            Tval::Virtual | Tval::Hypervisor => self.timer_count(tval.timer()).0,
        })
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
        let status = if self.condition_met(timer) {
            CTL_ISTATUS
        } else {
            0
        };
        (self.timer_regs(timer).ctl & (CTL_ENABLE | CTL_IMASK)) | status
    }

    /// `tval` as read: its timer's `CVAL` less the count it is read against
    /// ([`Cpu::tval_count`]), truncated to 32 bits (a signed value; the
    /// upper word reads as zero). With the timer disabled the value is
    /// ARBITRARY; this is the one it reads.
    pub(crate) fn timer_tval_read(&self, tval: Tval) -> u64 {
        self.tval_count(tval)
            .tval_to(self.timer_regs(tval.timer()).cval)
    }

    /// The `CVAL` a write of `value` to `tval` sets: the count it is written
    /// against ([`Cpu::tval_count`]) plus the low word of `value`,
    /// sign-extended.
    pub(crate) fn timer_cval_from_tval(&self, tval: Tval, value: u64) -> u64 {
        self.tval_count(tval).plus_tval(value)
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
    /// register writes, the physical count being `now`. Low, it rises where
    /// its count reaches `CVAL`, the physical count having come the same
    /// distance. High, it falls where its count wraps to zero: where the
    /// physical count reaches the timer's offset ([`Cpu::timer_offset`]),
    /// which takes the count below a nonzero `CVAL` again,
    /// `IsTimerConditionMet` comparing them unsigned. Only an offset count
    /// wraps, so the EL2 timer's line, high, stays high.
    fn timer_change(&self, timer: GenericTimer, now: Count) -> Option<VmInstant> {
        let regs = self.timer_regs(timer);
        if regs.ctl & CTL_ENABLE == 0 || regs.ctl & CTL_IMASK != 0 {
            return None;
        }
        let count = self.timer_count_at(timer, now);
        let cval = TimerCount(regs.cval);
        if count < cval {
            return self.tick_reaching_count(now, count, cval);
        }
        // The count wraps where the physical count reaches the offset.
        let wrap = self.timer_offset(timer).wrap();
        if now < wrap && regs.cval != 0 {
            self.tick_reaching(wrap)
        } else {
            None
        }
    }

    /// The earliest future tick at which a timer output changes, if one
    /// will without further register writes.
    pub fn next_timer_deadline(&self) -> Option<VmInstant> {
        let now = self.counter();
        GenericTimer::ALL
            .into_iter()
            .filter_map(|timer| self.timer_change(timer, now))
            .min()
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
    use crate::cpu::{CpuConfig, Levels, NoEl2, SysReg, SysRegRefusal, WithEl2};
    use rusty_box_core::ClockHz;

    /// A Cortex-A57 of the levels `L` running at `ips`, its counter at
    /// `counter_hz`.
    fn processor<L: Levels>(ips: u64, counter_hz: u64) -> Cpu<L> {
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

    fn cpu(ips: u64, counter_hz: u64) -> Cpu<NoEl2> {
        processor(ips, counter_hz)
    }

    fn cpu_el2(ips: u64, counter_hz: u64) -> Cpu<WithEl2> {
        processor(ips, counter_hz)
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
        assert_eq!(
            c.timer_ctl_read(GenericTimer::Virtual) & CTL_ISTATUS,
            CTL_ISTATUS
        );
        assert!(!c.timer_lines().virtual_);
    }

    /// TVAL is a signed 32-bit view of CVAL - count.
    #[test]
    fn tval_round_trips_through_cval() {
        let mut c = cpu(1_000, 1_000);
        c.icount = 100;
        let cval = c.timer_cval_from_tval(Tval::Physical(El::El1), u64::from((-10i32) as u32));
        assert_eq!(cval, 90);
        c.sys.cntp_cval_el0 = cval;
        assert_eq!(
            c.timer_tval_read(Tval::Physical(El::El1)),
            u64::from((-10i32) as u32)
        );
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
        assert_eq!(c.timer_tval_read(Tval::Virtual), 10);
        assert!(!c.timer_lines().virtual_);
        assert_eq!(c.next_timer_deadline().map(VmInstant::ticks), Some(50));
        c.icount = 50;
        assert!(c.timer_lines().virtual_);
        assert_eq!(
            c.timer_cval_from_tval(Tval::Virtual, 5),
            25,
            "the virtual count 20 plus 5"
        );
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
        for (reg, value) in [
            (SysReg::CNTHP_CVAL_EL2, 7),
            (SysReg::CNTHP_CTL_EL2, CTL_ENABLE),
        ] {
            assert_eq!(
                plain.host_sysreg_write(reg, value),
                Err(SysRegRefusal::Absent),
                "{reg:?}"
            );
        }
        plain.icount = 7;
        assert!(!plain.timer_lines().hypervisor);
        assert_eq!(plain.next_timer_deadline(), None);
    }
}
