//! Guest time that runs only while the guest is allowed to run.
//!
//! A machine whose guest is on hardware no longer counts instructions, so the
//! tick its devices read has to come from somewhere else. It comes from here:
//! the host's monotonic clock, converted at the machine's nominal rate, and
//! gated on whether the guest is allowed to run at all. That gate is the whole
//! point — a paused machine's devices must observe no elapsed guest time, or
//! every timer they hold fires at once when it resumes.
//!
//! The reading is a [`VmInstant`] at a [`ClockHz`], the same pair a device
//! reads under the interpreter, so nothing downstream can tell which engine
//! produced it.

use rusty_box_core::time::{ClockHz, HostClock, HostInstant, VmDuration, VmInstant};
use std::time::Duration;

/// A host clock over [`std::time::Instant`] — the [`HostClock`] the engine runs
/// on.
///
/// Readings are nanoseconds since the clock was created, so a `HostInstant`
/// from one `StdClock` is meaningless to another.
pub struct StdClock {
    epoch: std::time::Instant,
}

impl StdClock {
    /// A clock whose zero is now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            epoch: std::time::Instant::now(),
        }
    }
}

impl Default for StdClock {
    fn default() -> Self {
        Self::new()
    }
}

impl HostClock for StdClock {
    fn now(&self) -> HostInstant {
        HostInstant::from_nanos(
            u64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(u64::MAX),
        )
    }
}

/// Whether the clock is running, and what each answer needs to be read (R2).
///
/// `Running` carries the host anchor the interval in flight is measured from;
/// `Stopped` carries nothing at all, because a stopped clock has no anchor and
/// nothing may be extrapolated from one.
enum State {
    Running { host_at: HostInstant },
    Stopped,
}

/// Guest time that advances only while the guest may run, answered in the
/// tree's own unit.
///
/// A reading is the clock's origin plus the ticks its accumulated *running*
/// host time has earned at `rate`. A paused interval is never accumulated, so a
/// pause is invisible to the guest; and because the sub-tick remainder lives in
/// those accumulated nanoseconds rather than in a floored reading, any number of
/// pauses costs the machine nothing. The one remainder ever set aside is the one
/// standing at the instant of a read, and the next read recovers it — which is
/// the rule [`ClockHz::ticks_from_nanos_floor`] states: reported time never runs
/// ahead of the counter.
pub struct VmClockSource<H: HostClock> {
    state: State,
    rate: ClockHz,
    /// The reading this clock counts up from — its zero, not a moving anchor.
    epoch: VmInstant,
    /// Host nanoseconds already run, excluding the interval in flight. Whole
    /// nanoseconds rather than whole ticks, which is what makes a pause free.
    run_nanos: u64,
    host: H,
}

impl<H: HostClock> VmClockSource<H> {
    /// A clock frozen at `vm_at`, which will earn ticks at `rate` from `host`
    /// once started.
    pub const fn stopped_at(vm_at: VmInstant, rate: ClockHz, host: H) -> Self {
        Self {
            state: State::Stopped,
            rate,
            epoch: vm_at,
            run_nanos: 0,
            host,
        }
    }

    /// The guest time now: the origin plus what running has earned since it.
    #[must_use]
    pub fn now(&self) -> VmInstant {
        self.epoch
            .add(self.rate.ticks_from_nanos_floor(self.running_nanos()))
    }

    /// Let time run, from the reading it stands at.
    ///
    /// Idempotent: starting a running clock leaves its anchor alone, so the
    /// interval in flight is neither restarted nor counted a second time.
    pub fn start(&mut self) {
        if matches!(self.state, State::Stopped) {
            self.state = State::Running {
                host_at: self.host.now(),
            };
        }
    }

    /// Freeze time at the current reading and answer it.
    ///
    /// The interval that was running is folded into the running total whole,
    /// sub-tick remainder included, so the pause that follows costs nothing.
    ///
    /// Idempotent: stopping a stopped clock answers the same value again.
    pub fn stop(&mut self) -> VmInstant {
        if self.is_running() {
            self.run_nanos = self.running_nanos();
            self.state = State::Stopped;
        }
        self.now()
    }

    /// Whether the guest this clock times is allowed to run.
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self.state, State::Running { .. })
    }

    /// The rate ticks are earned at — the machine's nominal instruction rate.
    #[must_use]
    pub const fn rate(&self) -> ClockHz {
        self.rate
    }

    /// How long from now until `at` becomes [`now`](Self::now), zero once it
    /// already has, and `None` while stopped, because a stopped clock will never
    /// reach it.
    ///
    /// Rounded up: the answer is the shortest host span after which
    /// [`now`](Self::now) is at least `at`, so a thread that waits it out never
    /// wakes before the tick it waited for exists. A span rather than a host
    /// instant, because a span is what [`std::sync::Condvar::wait_timeout`] and
    /// [`std::thread::park_timeout`] take and cannot overflow the host's own
    /// instant representation.
    #[must_use]
    pub fn time_until(&self, at: VmInstant) -> Option<Duration> {
        match self.state {
            State::Running { .. } => {
                let earn_at = self.nanos_to_earn(at.since(self.epoch));
                Some(Duration::from_nanos(
                    earn_at.saturating_sub(self.running_nanos()),
                ))
            }
            State::Stopped => None,
        }
    }

    /// Host nanoseconds this clock has run, the interval in flight included.
    fn running_nanos(&self) -> u64 {
        match self.state {
            State::Running { host_at } => self
                .run_nanos
                .saturating_add(self.host.now().nanos_since(host_at)),
            State::Stopped => self.run_nanos,
        }
    }

    /// The shortest running span in which `span` ticks are earned at this rate.
    ///
    /// [`ClockHz::nanos_at`] floors, which is the reporting rule; arming a wait
    /// takes the "not before" rule [`ClockHz::ticks_from_micros_ceil`] keeps, so
    /// a floor that has not yet earned the last tick is one nanosecond short of
    /// the span that has.
    fn nanos_to_earn(&self, span: VmDuration) -> u64 {
        let floor = self.rate.nanos_at(VmInstant::from_ticks(span.ticks()));
        if self.rate.ticks_from_nanos_floor(floor).ticks() >= span.ticks() {
            floor
        } else {
            floor.saturating_add(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::SharedClock;

    fn mhz(hz: u64) -> ClockHz {
        ClockHz::new(hz).expect("a rate")
    }

    #[test]
    fn time_advances_only_while_running() {
        let host = SharedClock::default();
        host.advance_nanos(1_000);
        let mut clock =
            VmClockSource::stopped_at(VmInstant::from_ticks(0), mhz(1_000_000_000), host.clone()); // 1 tick = 1 ns
        host.advance_nanos(500);
        assert_eq!(
            clock.now().ticks(),
            0,
            "stopped: the host moved, the guest did not"
        );
        clock.start();
        host.advance_nanos(700);
        assert_eq!(clock.now().ticks(), 700);
        let frozen = clock.stop();
        host.advance_nanos(10_000);
        assert_eq!(clock.now(), frozen);
        clock.start();
        host.advance_nanos(300);
        assert_eq!(
            clock.now().ticks(),
            1_000,
            "a pause is invisible to the guest"
        );
        clock.start();
        assert_eq!(clock.now().ticks(), 1_000, "start is idempotent");
    }

    #[test]
    fn ticks_are_earned_at_the_machines_rate() {
        let host = SharedClock::default();
        let mut clock =
            VmClockSource::stopped_at(VmInstant::from_ticks(0), mhz(50_000_000), host.clone()); // Ips::BOCHS_DEFAULT
        clock.start();
        host.advance_nanos(1_000_000_000);
        assert_eq!(clock.now().ticks(), 50_000_000, "one second is ips ticks");
        host.advance_nanos(10);
        assert_eq!(
            clock.now().ticks(),
            50_000_000,
            "10 ns is less than a 20 ns tick — floor, not round"
        );
        host.advance_nanos(10);
        assert_eq!(clock.now().ticks(), 50_000_001);
    }

    #[test]
    fn a_wait_exists_for_a_future_vm_time_only_while_running() {
        let host = SharedClock::default();
        let mut clock =
            VmClockSource::stopped_at(VmInstant::from_ticks(0), mhz(50_000_000), host.clone());
        assert!(clock.time_until(VmInstant::from_ticks(5)).is_none());
        clock.start();
        assert_eq!(
            clock.time_until(VmInstant::from_ticks(5)),
            Some(Duration::from_nanos(100)),
            "5 ticks at 50 MHz is 100 ns away"
        );
    }

    /// The rate the harnesses actually run at does not divide a second, and
    /// rounding down there wakes a waiter before its tick exists.
    #[test]
    fn a_wait_never_ends_before_the_tick_it_waits_for() {
        let host = SharedClock::default();
        let mut clock =
            VmClockSource::stopped_at(VmInstant::from_ticks(0), mhz(300_000_000), host.clone()); // alpine_probe's rate: 10/3 ns per tick
        clock.start();
        let wait = clock.time_until(VmInstant::from_ticks(1)).expect("running");
        assert_eq!(
            wait,
            Duration::from_nanos(4),
            "one tick is 3.33 ns, so the wait rounds UP to 4 — waking at 3 would \
             arrive before the tick"
        );
        host.advance_nanos(u64::try_from(wait.as_nanos()).expect("a wait of 4 ns"));
        assert!(
            clock.now().ticks() >= 1,
            "after waiting, the tick has genuinely arrived"
        );
    }

    /// A pause must not cost the machine time. Sub-tick remainders carry, so
    /// many pauses lose no more than one tick in total — not one each.
    #[test]
    fn sub_tick_remainders_survive_a_pause() {
        let host = SharedClock::default();
        let mut clock =
            VmClockSource::stopped_at(VmInstant::from_ticks(0), mhz(1_000_000_000), host.clone()); // 1 tick = 1 ns
        for _ in 0..10 {
            clock.start();
            host.advance_nanos(3);
            clock.stop();
            host.advance_nanos(1_000); // paused: invisible
        }
        assert_eq!(clock.now().ticks(), 30, "ten running intervals of 3 ns each");
    }

    /// The same property at a rate where each running interval genuinely leaves
    /// a remainder: ten 5 ns intervals at 300 MHz are 1.5 ticks each, and the
    /// halves must add up rather than be floored away one pause at a time.
    #[test]
    fn a_remainder_left_by_every_interval_still_adds_up() {
        let host = SharedClock::default();
        let mut clock =
            VmClockSource::stopped_at(VmInstant::from_ticks(0), mhz(300_000_000), host.clone());
        for _ in 0..10 {
            clock.start();
            host.advance_nanos(5);
            clock.stop();
            host.advance_nanos(1_000); // paused: invisible
        }
        assert_eq!(
            clock.now().ticks(),
            15,
            "50 ns of running at 300 MHz is 15 ticks; flooring each 5 ns interval \
             on its own would answer 10"
        );
    }
}
