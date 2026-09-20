//! Emulated time as a tick count with a stated rate, and the host clock that
//! can drive it.
//!
//! Bochs keeps emulated time as a tick counter (`bx_pc_system.ticks_total`)
//! whose rate is the configured instructions-per-second, and converts to
//! microseconds or nanoseconds at each use site. This port inherited the
//! counter but not the rate: a tick travelled as a bare `u64` and every
//! consumer that needed wall-clock units had to be handed `ips` separately,
//! which is how instruction count leaked into eight device files.
//!
//! Here the rate travels with the reading. That is what makes the same device
//! model work under an execution engine that does not count instructions: a
//! hypervisor advances the tick from a host monotonic clock at the same stated
//! rate, and nothing downstream can tell the difference — which is the point,
//! because the snapshot format checks that rate (`SEC_ACPI` rejects a snapshot
//! whose `ips` differs from the live machine's, and the PIT validates its
//! microsecond remainder against it).
//!
//! **Every conversion below is a named function that preserves the rounding of
//! the site it replaces**, and says which site that is. They are not
//! interchangeable: Bochs rounds up when arming a timer (so a zero-delay
//! request still lands in the future) and down when reporting a reading (so
//! reported time never runs ahead of the counter). Collapsing them onto one
//! rule changes guest-visible timing.

use core::num::NonZeroU64;

const MICROS_PER_SECOND: u128 = 1_000_000;
const NANOS_PER_SECOND: u128 = 1_000_000_000;

/// A conversion whose exact result does not fit the requested width.
///
/// Bochs computes in `double` and silently loses precision; this port computes
/// in `u128` and reports instead. Sites that inherited Bochs's saturating
/// behaviour use the saturating conversions and never see this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeOverflow;

/// Ticks per second of an emulated clock — Bochs `bx_pc_system.m_ips`.
///
/// Non-zero by construction because every conversion here divides by it. That
/// is not defensive: the rate reaches this crate from guest-influenced
/// configuration, and a zero reached the divisor once already in this tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClockHz(NonZeroU64);

impl ClockHz {
    /// Bochs `config.cc` default for `cpu.ips`.
    pub const BOCHS_DEFAULT: Self = match NonZeroU64::new(50_000_000) {
        Some(hz) => Self(hz),
        None => unreachable!(),
    };

    #[inline]
    pub const fn new(hz: u64) -> Option<Self> {
        match NonZeroU64::new(hz) {
            Some(hz) => Some(Self(hz)),
            None => None,
        }
    }

    #[inline]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// Microseconds elapsed at `at`, rounded down, saturating.
    ///
    /// Bochs pc_system.cc `time_usec`.
    #[inline]
    pub const fn micros_at(self, at: VmInstant) -> u64 {
        saturate(at.ticks as u128 * MICROS_PER_SECOND / self.get() as u128)
    }

    /// Microseconds elapsed at `at`, rounded down, reporting overflow.
    ///
    /// Bochs pc_system.cc `time_usec` again, at the call sites this port made
    /// fallible rather than saturating.
    #[inline]
    pub const fn try_micros_at(self, at: VmInstant) -> Result<u64, TimeOverflow> {
        narrow(at.ticks as u128 * MICROS_PER_SECOND / self.get() as u128)
    }

    /// Nanoseconds elapsed at `at`, rounded down, saturating.
    ///
    /// Bochs pc_system.cc `time_nsec` — what hpet.cc reads to drive its main
    /// counter.
    #[inline]
    pub const fn nanos_at(self, at: VmInstant) -> u64 {
        saturate(at.ticks as u128 * NANOS_PER_SECOND / self.get() as u128)
    }

    /// Microseconds elapsed at `at`, with the sub-microsecond phase that
    /// division discarded.
    ///
    /// The PIT keeps this remainder across syncs and snapshots it, because
    /// dropping it loses a fraction of a microsecond per access and the
    /// accumulated loss stalls IRQ0 — which is a guest-visible hang, not a
    /// rounding curiosity. Bochs pit.cc keeps the same carry.
    #[inline]
    pub const fn micros_phase_at(self, at: VmInstant) -> MicrosPhase {
        let scaled = at.ticks as u128 * MICROS_PER_SECOND;
        MicrosPhase {
            micros: saturate(scaled / self.get() as u128),
            remainder: scaled % self.get() as u128,
        }
    }

    /// Ticks spanning `micros`, rounded **up**, never zero.
    ///
    /// The arming rule: Bochs `activate_timer` treats a delay as "not before",
    /// so a sub-tick delay must still land on the next tick rather than on the
    /// current one, where it would fire immediately and re-arm forever.
    #[inline]
    pub const fn ticks_from_micros_ceil(self, micros: u64) -> VmDuration {
        let hz = self.get() as u128;
        let exact = micros as u128 * hz;
        let ticks = exact.div_ceil(MICROS_PER_SECOND);
        VmDuration {
            ticks: saturate(if ticks == 0 { 1 } else { ticks }),
        }
    }

    /// Ticks spanning `micros`, rounded down, saturating.
    ///
    /// Bochs pc_system.cc `usec_to_ticks`. Distinct from
    /// [`Self::ticks_from_micros_ceil`] and deliberately without its
    /// `max(1)`: this converts a *quantity* of time, so zero microseconds is
    /// zero ticks.
    #[inline]
    pub const fn ticks_from_micros_floor(self, micros: u64) -> VmDuration {
        VmDuration {
            ticks: saturate(micros as u128 * self.get() as u128 / MICROS_PER_SECOND),
        }
    }

    /// Ticks spanning `micros`, rounded down, reporting overflow.
    #[inline]
    pub const fn try_ticks_from_micros(self, micros: u64) -> Result<VmDuration, TimeOverflow> {
        match narrow(micros as u128 * self.get() as u128 / MICROS_PER_SECOND) {
            Ok(ticks) => Ok(VmDuration { ticks }),
            Err(overflow) => Err(overflow),
        }
    }

    /// Ticks spanning `nanos`, rounded down, saturating.
    ///
    /// Bochs pc_system.cc `activate_timer_nsec` computes
    /// `(Bit64u)(double(nsec) * m_ips / 1000.0)`, a truncating conversion. A
    /// floored deadline may fire just before the crossing it was armed for;
    /// the HPET then finds the comparator not yet reached and re-arms, exactly
    /// as upstream does.
    #[inline]
    pub const fn ticks_from_nanos_floor(self, nanos: u64) -> VmDuration {
        VmDuration {
            ticks: saturate(nanos as u128 * self.get() as u128 / NANOS_PER_SECOND),
        }
    }
}

/// A point on an emulated clock, counted in ticks from machine start.
///
/// Distinct from [`VmDuration`] because a point and a span are not the same
/// quantity (R4): subtracting two instants gives a duration, and adding two
/// instants is meaningless. This port has already had a bug from letting the
/// two share a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VmInstant {
    ticks: u64,
}

impl VmInstant {
    pub const ZERO: Self = Self { ticks: 0 };

    #[inline]
    pub const fn from_ticks(ticks: u64) -> Self {
        Self { ticks }
    }

    #[inline]
    pub const fn ticks(self) -> u64 {
        self.ticks
    }

    /// Time from `earlier` to this instant, or zero if this is not later.
    ///
    /// Saturating rather than signed: every consumer asks "how long until the
    /// deadline", and a passed deadline is due now, not overdue by a negative
    /// amount.
    #[inline]
    pub const fn since(self, earlier: Self) -> VmDuration {
        VmDuration {
            ticks: self.ticks.saturating_sub(earlier.ticks),
        }
    }

    #[inline]
    pub const fn add(self, span: VmDuration) -> Self {
        Self {
            ticks: self.ticks.saturating_add(span.ticks),
        }
    }
}

/// A span of emulated time, counted in ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VmDuration {
    ticks: u64,
}

impl VmDuration {
    pub const ZERO: Self = Self { ticks: 0 };
    /// The shortest span a timer can be armed for — Bochs's "not before now".
    pub const ONE_TICK: Self = Self { ticks: 1 };

    #[inline]
    pub const fn from_ticks(ticks: u64) -> Self {
        Self { ticks }
    }

    #[inline]
    pub const fn ticks(self) -> u64 {
        self.ticks
    }
}

/// Microseconds elapsed, plus the phase division discarded.
///
/// `remainder` is in units of ticks·microseconds-per-second and is always less
/// than the rate, which is the invariant the PIT's snapshot validates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MicrosPhase {
    pub micros: u64,
    pub remainder: u128,
}

/// A reading of an emulated clock: where it is, and how fast it runs.
///
/// Passed by value into a device access the way Bochs devices reach
/// `bx_pc_system` from inside a handler. Carrying the rate beside the reading
/// is what lets a device convert to wall-clock units without being told the
/// machine's instruction rate — and therefore without assuming there is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmClock {
    now: VmInstant,
    rate: ClockHz,
}

impl VmClock {
    #[inline]
    pub const fn new(now: VmInstant, rate: ClockHz) -> Self {
        Self { now, rate }
    }

    #[inline]
    pub const fn now(self) -> VmInstant {
        self.now
    }

    #[inline]
    pub const fn rate(self) -> ClockHz {
        self.rate
    }

    /// Microseconds elapsed, rounded down, saturating — Bochs `time_usec`.
    #[inline]
    pub const fn micros(self) -> u64 {
        self.rate.micros_at(self.now)
    }

    /// Nanoseconds elapsed, rounded down, saturating — Bochs `time_nsec`.
    #[inline]
    pub const fn nanos(self) -> u64 {
        self.rate.nanos_at(self.now)
    }

    /// Microseconds elapsed with the discarded phase — the PIT's carry.
    #[inline]
    pub const fn micros_phase(self) -> MicrosPhase {
        self.rate.micros_phase_at(self.now)
    }

    /// The instant `span` from now.
    #[inline]
    pub const fn after(self, span: VmDuration) -> VmInstant {
        self.now.add(span)
    }

    /// The instant `micros` from now under the arming rule — rounded up, never
    /// this same tick. The deadline a device gets when it arms a timer from
    /// inside an access.
    #[inline]
    pub const fn after_micros(self, micros: u64) -> VmInstant {
        self.now.add(self.rate.ticks_from_micros_ceil(micros))
    }

    /// The instant `micros` from now, rounded down — Bochs `usec_to_ticks`
    /// added to the current tick.
    #[inline]
    pub const fn after_micros_floor(self, micros: u64) -> VmInstant {
        self.now.add(self.rate.ticks_from_micros_floor(micros))
    }

    /// The instant `nanos` from now, rounded down — the HPET's comparator.
    #[inline]
    pub const fn after_nanos_floor(self, nanos: u64) -> VmInstant {
        self.now.add(self.rate.ticks_from_nanos_floor(nanos))
    }
}

/// A point on the host's monotonic clock.
///
/// Opaque and comparison-only: it exists so an engine can say "run until" and
/// a test can say "now is exactly this", without either naming `std::time`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostInstant {
    nanos: u64,
}

impl HostInstant {
    #[inline]
    pub const fn from_nanos(nanos: u64) -> Self {
        Self { nanos }
    }

    #[inline]
    pub const fn as_nanos(self) -> u64 {
        self.nanos
    }

    /// Elapsed host time from `earlier`, in nanoseconds, zero if not later.
    #[inline]
    pub const fn nanos_since(self, earlier: Self) -> u64 {
        self.nanos.saturating_sub(earlier.nanos)
    }
}

/// The single host-clock seam.
///
/// Bochs reads wall time through one free function (`bx_get_realtime64_usec`),
/// which makes its realtime behaviour untestable and unportable. One trait with
/// one method makes the same reading mockable, which is what deterministic
/// replay, wasm and UEFI all need — none of them has a monotonic clock on the
/// terms `std` assumes.
pub trait HostClock {
    fn now(&self) -> HostInstant;
}

/// A host clock a caller advances by hand — tests, replay, and targets with no
/// clock of their own.
#[derive(Debug, Clone, Copy, Default)]
pub struct ManualClock {
    nanos: u64,
}

impl ManualClock {
    #[inline]
    pub const fn new() -> Self {
        Self { nanos: 0 }
    }

    #[inline]
    pub const fn at(nanos: u64) -> Self {
        Self { nanos }
    }

    #[inline]
    pub fn advance_nanos(&mut self, nanos: u64) {
        self.nanos = self.nanos.saturating_add(nanos);
    }
}

impl HostClock for ManualClock {
    #[inline]
    fn now(&self) -> HostInstant {
        HostInstant::from_nanos(self.nanos)
    }
}

/// Clamp a `u128` result to `u64`, the way the reporting conversions do.
#[inline]
const fn saturate(value: u128) -> u64 {
    if value > u64::MAX as u128 {
        u64::MAX
    } else {
        value as u64
    }
}

/// Narrow a `u128` result to `u64`, reporting rather than clamping.
#[inline]
const fn narrow(value: u128) -> Result<u64, TimeOverflow> {
    if value > u64::MAX as u128 {
        Err(TimeOverflow)
    } else {
        Ok(value as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rates a machine actually runs at, plus the awkward ones: a rate that
    /// does not divide a second evenly is where a conversion routed through
    /// nanoseconds loses a tick, which is the reason this crate converts from
    /// ticks directly.
    const RATES: &[u64] = &[1, 2, 3, 7, 1_000, 1_000_000, 50_000_000, 300_000_000, u64::MAX];

    fn hz(rate: u64) -> ClockHz {
        ClockHz::new(rate).expect("rates under test are non-zero")
    }

    /// The formulas these replace, transcribed from their sites so the sweep
    /// compares against the tree as it was, not against a restatement of the
    /// new code.
    mod as_it_was {
        pub fn time_usec(ticks: u64, ips: u64) -> u64 {
            u64::try_from(u128::from(ticks) * 1_000_000u128 / u128::from(ips)).unwrap_or(u64::MAX)
        }

        pub fn time_nsec(ticks: u64, ips: u64) -> u64 {
            u64::try_from(u128::from(ticks) * 1_000_000_000u128 / u128::from(ips))
                .unwrap_or(u64::MAX)
        }

        /// `BxPcSystemC::usec_to_ticks` — floor, fallible.
        pub fn usec_to_ticks(useconds: u64, ips: u64) -> Option<u64> {
            u64::try_from(u128::from(useconds) * u128::from(ips) / 1_000_000u128).ok()
        }

        /// `WheelTimerService::usec_to_ticks` and
        /// `BxDevicesC::request_timer_after_usec_with_mode` — ceil, never zero.
        pub fn arm_usec_to_ticks(delay_usec: u64, ips: u64) -> u64 {
            (u128::from(delay_usec) * u128::from(ips))
                .div_ceil(1_000_000)
                .max(1)
                .min(u128::from(u64::MAX)) as u64
        }

        /// `BxHpetC::nsec_to_pc_ticks` — floor.
        pub fn nsec_to_pc_ticks(nsec: u64, ips: u64) -> u64 {
            u64::try_from(u128::from(nsec) * u128::from(ips) / 1_000_000_000u128)
                .unwrap_or(u64::MAX)
        }

        /// `BxPitC::sync_to_icount` — floor plus the retained phase.
        ///
        /// Note the `as u64`: alone among the conversions here this one
        /// **wraps** rather than saturating, because it never went through the
        /// `try_from(..).unwrap_or(u64::MAX)` its siblings use. See
        /// `the_phase_conversion_saturates_where_its_site_wrapped`.
        pub fn micros_phase(ticks: u64, ips: u64) -> (u64, u128) {
            let scaled = u128::from(ticks) * u128::from(1_000_000u64);
            ((scaled / u128::from(ips)) as u64, scaled % u128::from(ips))
        }
    }

    /// Every named conversion equals the site it replaces, across the rates a
    /// machine runs at and the tick magnitudes a boot reaches. This is the gate
    /// on the whole time unit: if it passes, moving a site onto `VmClock`
    /// cannot change what a guest observes.
    #[test]
    fn every_conversion_matches_the_formula_it_replaces() {
        let ticks = [
            0u64,
            1,
            2,
            999_999,
            1_000_000,
            50_000_000,
            300_000_000,
            1 << 40,
            u64::MAX,
        ];
        for &rate in RATES {
            let clock_rate = hz(rate);
            for &t in &ticks {
                let at = VmInstant::from_ticks(t);

                assert_eq!(
                    clock_rate.micros_at(at),
                    as_it_was::time_usec(t, rate),
                    "micros_at({t}, {rate})"
                );
                assert_eq!(
                    clock_rate.nanos_at(at),
                    as_it_was::time_nsec(t, rate),
                    "nanos_at({t}, {rate})"
                );

                let phase = clock_rate.micros_phase_at(at);
                let (micros, remainder) = as_it_was::micros_phase(t, rate);
                assert_eq!(phase.remainder, remainder, "micros_phase_at({t}, {rate})");
                assert!(
                    phase.remainder < u128::from(rate),
                    "the PIT snapshot validates remainder < ips"
                );
                // Equal wherever the exact value is representable, which is
                // every input any site can reach; the one case that is not is
                // its own test below.
                if u128::from(t) * u128::from(1_000_000u64) / u128::from(rate)
                    <= u128::from(u64::MAX)
                {
                    assert_eq!(phase.micros, micros, "micros_phase_at({t}, {rate})");
                }

                assert_eq!(
                    clock_rate.ticks_from_micros_ceil(t).ticks(),
                    as_it_was::arm_usec_to_ticks(t, rate),
                    "ticks_from_micros_ceil({t}, {rate})"
                );
                assert_eq!(
                    clock_rate.ticks_from_nanos_floor(t).ticks(),
                    as_it_was::nsec_to_pc_ticks(t, rate),
                    "ticks_from_nanos_floor({t}, {rate})"
                );
                assert_eq!(
                    clock_rate.try_ticks_from_micros(t).map(|d| d.ticks()).ok(),
                    as_it_was::usec_to_ticks(t, rate),
                    "try_ticks_from_micros({t}, {rate})"
                );
                assert_eq!(
                    clock_rate.try_micros_at(at).ok(),
                    u64::try_from(u128::from(t) * 1_000_000u128 / u128::from(rate)).ok(),
                    "try_micros_at({t}, {rate})"
                );
            }
        }
    }

    /// A tick is not the same quantity as a nanosecond, and this is why the
    /// clock stores ticks rather than an absolute nanosecond deadline: at a
    /// rate that does not divide a second, converting a tick count out to
    /// nanoseconds and back does not return it.
    #[test]
    fn a_deadline_kept_in_ticks_survives_a_rate_that_nanoseconds_cannot_express() {
        let rate = hz(3);
        let armed = VmDuration::from_ticks(4);
        let as_nanos = rate.nanos_at(VmInstant::from_ticks(armed.ticks()));
        assert_eq!(
            rate.ticks_from_nanos_floor(as_nanos).ticks(),
            3,
            "the round trip loses a tick, so deadlines must not take it"
        );
        assert_eq!(armed.ticks(), 4, "which the tick-denominated span keeps");
    }

    /// Arming rounds up and never returns the current tick; reporting rounds
    /// down. A timer armed for less than one tick of delay fires next tick, not
    /// this one — where it would re-fire forever.
    #[test]
    fn arming_rounds_up_and_reading_rounds_down() {
        let rate = hz(1_000_000);
        assert_eq!(rate.ticks_from_micros_ceil(0).ticks(), 1);
        assert_eq!(rate.ticks_from_micros_floor(0).ticks(), 0);

        let slow = hz(1);
        assert_eq!(
            slow.ticks_from_micros_ceil(1).ticks(),
            1,
            "a sub-tick delay still lands in the future"
        );
        assert_eq!(slow.ticks_from_micros_floor(1).ticks(), 0);

        let clock = VmClock::new(VmInstant::from_ticks(500), slow);
        assert_eq!(clock.after_micros(1).ticks(), 501);
        assert_eq!(clock.after_micros_floor(1).ticks(), 500);
    }

    /// A deadline anchors at the instant of the access that produced it, which
    /// is what `VmClock` carrying `now` is for: a device arming mid-batch must
    /// not anchor at the wheel's lagging position.
    #[test]
    fn deadlines_anchor_at_the_reading_they_were_armed_from() {
        let rate = hz(1_000_000);
        let at_access = VmClock::new(VmInstant::from_ticks(500), rate);
        let at_wheel = VmClock::new(VmInstant::ZERO, rate);
        assert_eq!(at_access.after_micros(10).ticks(), 510);
        assert_eq!(at_wheel.after_micros(10).ticks(), 10);
    }

    /// The one input where this crate does not reproduce its site exactly, and
    /// why that is deliberate.
    ///
    /// `BxPitC::sync_to_icount` narrows with `as u64`, so a microsecond count
    /// too large for the width wraps — and a wrapped `total_usec` runs
    /// backwards, which stalls the PIT permanently (the IRQ0 freeze its own
    /// comments describe). Every sibling conversion in this module saturates,
    /// because they narrow through `try_from(..).unwrap_or(u64::MAX)`. Rather
    /// than carry one wrapping outlier into a shared unit, the phase
    /// conversion saturates like the rest.
    ///
    /// This changes nothing a machine can observe: reaching it needs the tick
    /// counter within a factor of the rate of `u64::MAX`, which at the slowest
    /// rate this port accepts is more instructions than a host will retire.
    #[test]
    fn the_phase_conversion_saturates_where_its_site_wrapped() {
        let rate = hz(1);
        let phase = rate.micros_phase_at(VmInstant::from_ticks(u64::MAX));
        assert_eq!(phase.micros, u64::MAX);
        assert_ne!(
            phase.micros,
            as_it_was::micros_phase(u64::MAX, 1).0,
            "the site wrapped here; this is the sole intended difference"
        );
        assert_eq!(
            phase.micros,
            rate.micros_at(VmInstant::from_ticks(u64::MAX)),
            "and it now agrees with the sibling that reports the same quantity"
        );
    }

    /// Saturation, not wraparound, at the top of the range — a reported time
    /// that wrapped would run backwards.
    #[test]
    fn conversions_saturate_rather_than_wrap() {
        let rate = hz(1);
        let far = VmInstant::from_ticks(u64::MAX);
        assert_eq!(rate.micros_at(far), u64::MAX);
        assert_eq!(rate.try_micros_at(far), Err(TimeOverflow));
        assert_eq!(far.add(VmDuration::from_ticks(1)).ticks(), u64::MAX);
    }

    /// A duration is measured between instants, and a passed deadline is due
    /// now rather than overdue by a negative amount.
    #[test]
    fn a_passed_deadline_is_due_now() {
        let now = VmInstant::from_ticks(100);
        assert_eq!(now.since(VmInstant::from_ticks(40)).ticks(), 60);
        assert_eq!(now.since(VmInstant::from_ticks(140)).ticks(), 0);
    }

    #[test]
    fn a_rate_of_zero_is_not_a_rate() {
        assert!(ClockHz::new(0).is_none());
        assert_eq!(ClockHz::BOCHS_DEFAULT.get(), 50_000_000);
    }

    #[test]
    fn a_manual_clock_moves_only_when_advanced() {
        let mut clock = ManualClock::new();
        assert_eq!(clock.now().as_nanos(), 0);
        clock.advance_nanos(1_500);
        assert_eq!(clock.now().as_nanos(), 1_500);
        assert_eq!(clock.now().nanos_since(HostInstant::from_nanos(500)), 1_000);
        assert_eq!(clock.now().nanos_since(HostInstant::from_nanos(9_000)), 0);
    }
}
