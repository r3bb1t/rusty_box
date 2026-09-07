//! The thread that runs a machine's devices on host time.
//!
//! ## What it replaces
//!
//! Under the slice model the machine's own timer wheel ran on the thread that
//! was inside the run: it could not fire while the guest was executing, so
//! every pulse a device owed arrived at a slice boundary instead of at its
//! deadline. Measured, the mean slice overran its budget 271-fold, and a guest
//! setting up its interrupts sees that as a timer that bursts rather than
//! ticks.
//!
//! So the wheel gets a thread of its own. It sleeps until the machine's next
//! device deadline, takes the machine's lock long enough to catch the wheel up
//! to the clock and run the boundary, and goes back to sleep. The guest is not
//! interrupted for any of it — the vCPU thread stays inside
//! `WHvRunVirtualProcessor`, and a device that raises a line reaches the guest
//! through the interrupt path rather than by ending its run.
//!
//! ## The two-phase wait
//!
//! Inherited whole from the slice alarm this module replaces, because the
//! measurement behind it has not changed: a timed wait on a condition variable
//! cannot return sooner than the host's 15.6 ms timer tick. Asked for 50
//! microseconds it returned after 15,467 on average — 309 times the request —
//! and asking for a millisecond still overshot 34-fold. Spinning the same 50
//! microseconds landed in 59.9.
//!
//! So everything beyond [`SPIN_MARGIN`] is slept and the margin itself is
//! spun. A device deadline nearer than the margin — which most are, since a
//! machine's next deadline is usually one 8042 or PIT period away — is spun
//! whole and fires when it was due.
//!
//! ## Lock order
//!
//! **machine, then clock.** [`service_once`] takes the clock inside the
//! machine's lock, and the vCPU thread's own per-exit catch-up does the same.
//! The wait in step 2 below takes the clock ALONE and releases it before
//! sleeping, because a thread that slept holding the clock would stop every
//! other thread from reading the time.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusty_box::cpu::instrumentation::Instrumentation;
use rusty_box::cpu::Result as CpuResult;
use rusty_box::emulator::{DeviceTime, Emulator, SliceEngine};
use rusty_box_core::time::{HostClock, VmInstant};
use rusty_box_core::{EngineFault, EngineFaultKind};

use crate::vm_clock::VmClockSource;

/// How much of a wait is spun rather than slept.
///
/// **Measured on this platform, not chosen.** `Condvar::wait_timeout` returns
/// after the host's timer tick whatever it is asked for — asked for 1 ms it
/// returned in 15.313 ms on average, for 5 ms in 15.819, for 10 ms in 15.640,
/// with a worst case of 26.842. The request does not appear in the answer at
/// all; the tick does.
///
/// So a slept wait cannot end within 15 ms of when it was asked to, and every
/// device deadline on a PC is nearer than that: the PIT at 1 kHz is 1 ms away,
/// the 8042's serial delay 150 µs. A device thread that slept for them would
/// deliver their pulses in bursts one host tick apart — which is precisely the
/// disease the slice model had and this design exists to cure.
///
/// Hence the margin covers the measured worst case with room over it: anything
/// due sooner than this is spun, and only a genuinely distant deadline is
/// slept, where one tick of error is a small fraction of the wait.
///
/// **The cost is a core, and it is real.** A guest with a 1 ms timer keeps
/// this thread spinning for as long as it runs. That is what every
/// hypervisor-backed VMM on Windows pays for timing fidelity, and the way out
/// is a high-resolution waitable timer
/// (`CreateWaitableTimerExW` with `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`)
/// rather than a shorter margin — a shorter margin does not buy back the tick,
/// it just misses the deadline.
const SPIN_MARGIN: Duration = Duration::from_millis(30);

/// Catch the machine's wheel up to the clock and run one device boundary.
///
/// The whole of what the thread does to a machine, factored out so it can be
/// tested without one: the arithmetic is the same on an interpreter machine as
/// on a hypervisor one, and an interpreter machine needs no hardware to build.
///
/// The elapsed span is the gap between where the clock has reached and where
/// the wheel stands, so a service that finds no host time passed earns no
/// ticks and fires nothing. Saturating, because the wheel is never behind by a
/// negative amount: a reset moves it forward under a clock that has not moved.
///
/// # Errors
/// Whatever the machine's own boundary raised.
pub(crate) fn service_once<T: Instrumentation, E: SliceEngine<T>, H: HostClock>(
    machine: &mut Emulator<T, E>,
    clock: &VmClockSource<H>,
) -> CpuResult<DeviceTime> {
    let elapsed = clock.now().ticks().saturating_sub(machine.ticks());
    machine.service_device_time(elapsed)
}

/// What the device thread is waiting for.
///
/// A state and two flags rather than three flags (R2): `run` and `stop` are
/// the thread's lifecycle and cannot be folded together — a paused thread is
/// still alive and resumable, a stopped one is not — while
/// `earlier_deadline` is a request that may arrive in either state.
#[derive(Debug, Default)]
struct DeviceWake {
    /// A deadline someone wants serviced sooner than the wheel's own.
    ///
    /// Set by [`DeviceThreadControl::deadline_moved_earlier`], and taken by
    /// the thread when it next computes a wait. A step's end is one of these:
    /// the caller wants `served` bumped at its target rather than at whatever
    /// the machine's next device deadline happens to be.
    earlier_deadline: Option<VmInstant>,
    /// The thread should exit.
    stop: bool,
    /// The thread should service. Down while the machine is paused.
    run: bool,
}

/// The handle a machine drives its device thread by.
///
/// Cloneable and `Send`: the thread holds one and the machine holds another,
/// and both only ever touch the two condition variables inside.
#[derive(Clone, Debug)]
pub struct DeviceThreadControl {
    wake: Arc<(Mutex<DeviceWake>, Condvar)>,
    /// How many times the thread has serviced, and a way to wait for the next.
    ///
    /// A generation rather than a flag, because a waiter must be able to tell
    /// "serviced since I last looked" from "serviced at some point": a flag
    /// races with the service that clears it, and a waiter that missed the
    /// clear waits for a service that has already happened.
    served: Arc<(Mutex<u64>, Condvar)>,
    /// Read without the lock by the spinning phase of a wait, which cannot
    /// hold one — see [`SPIN_MARGIN`]. Bumped by every change to `wake`, so a
    /// spin ends as soon as the state it was spinning on changes.
    generation: Arc<AtomicU64>,
}

impl Default for DeviceThreadControl {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceThreadControl {
    #[must_use]
    pub fn new() -> Self {
        Self {
            wake: Arc::new((Mutex::new(DeviceWake::default()), Condvar::new())),
            served: Arc::new((Mutex::new(0), Condvar::new())),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Service at `at` rather than at the wheel's own next deadline.
    ///
    /// Only ever moves the wake earlier — a request for a later instant than
    /// one already pending is dropped, so two callers asking for different
    /// deadlines both get theirs.
    pub fn deadline_moved_earlier(&self, at: VmInstant) {
        self.change(|wake| {
            wake.earlier_deadline = Some(match wake.earlier_deadline {
                Some(pending) if pending.ticks() <= at.ticks() => pending,
                _ => at,
            });
        });
    }

    /// Stop servicing, but stay alive.
    pub fn pause(&self) {
        self.change(|wake| wake.run = false);
    }

    /// Service again.
    pub fn resume(&self) {
        self.change(|wake| wake.run = true);
    }

    /// Leave the loop. The thread is joinable once this returns.
    pub fn stop(&self) {
        self.change(|wake| {
            wake.stop = true;
            wake.run = false;
        });
    }

    /// How many times the thread has serviced.
    #[must_use]
    pub fn served(&self) -> u64 {
        *self
            .served
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Block until the thread has serviced at least once more than `since`, or
    /// `within` passes.
    ///
    /// Answers the new generation, or `None` on the timeout. The sentinel is
    /// one that actually occurs: `served` is bumped after EVERY service, not
    /// only after one that did work, so a waiter cannot be left waiting on a
    /// condition the producer never writes.
    #[must_use]
    pub fn wait_served_by(&self, since: u64, within: Duration) -> Option<u64> {
        let (lock, changed) = &*self.served;
        let mut served = lock.lock().unwrap_or_else(PoisonError::into_inner);
        let deadline = Instant::now() + within;
        while *served <= since {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return None;
            };
            let (guard, timed_out) = changed
                .wait_timeout(served, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            served = guard;
            if timed_out.timed_out() && *served <= since {
                return None;
            }
        }
        Some(*served)
    }

    /// Apply a change and publish it.
    ///
    /// The one place the state is written (R5), so the generation bump and the
    /// notify cannot be forgotten by a new verb. A poisoned lock is recovered
    /// rather than propagated: the state behind it is three plain fields that
    /// no panic can leave half-written, and a panic raised into a machine
    /// driving a guest is strictly worse.
    fn change(&self, edit: impl FnOnce(&mut DeviceWake)) {
        {
            let mut wake = self.wake.0.lock().unwrap_or_else(PoisonError::into_inner);
            edit(&mut wake);
        }
        // Published after the state it describes, and read by a spinning
        // waiter before it acts, so a wait that has been superseded ends.
        self.generation.fetch_add(1, Ordering::Release);
        self.wake.1.notify_all();
    }

    /// Record a service and wake everyone waiting for one.
    fn record_service(&self) {
        let (lock, changed) = &*self.served;
        {
            let mut served = lock.lock().unwrap_or_else(PoisonError::into_inner);
            *served = served.saturating_add(1);
        }
        changed.notify_all();
    }
}

/// What the wait ended for (R0/R2).
///
/// Named rather than a bool, because the two are not degrees of the same
/// thing: one sends the thread to a machine that is still there, the other to
/// a return.
enum NextAction {
    /// Leave the loop; the machine is going away.
    Stop,
    /// Something is due. Service it.
    Service,
}

/// Start a device thread for `machine`.
///
/// Returns the handle to join and the control to drive it by. The thread
/// starts PAUSED: a machine that has not been resumed has no business earning
/// device time.
///
/// # Errors
/// Whatever the host said about starting a thread. Reported rather than
/// asserted: a machine whose devices have no thread runs a guest whose timers
/// never fire, and a library that aborted its caller's process over it would
/// take a front end down with it.
pub(crate) fn spawn<T: Instrumentation + Send + 'static>(
    machine: Arc<Mutex<Box<Emulator<T, crate::WhpEngine>>>>,
    clock: Arc<Mutex<VmClockSource<crate::StdClock>>>,
    vcpus: Vec<crate::vcpu_thread::VcpuControl>,
) -> Result<(JoinHandle<()>, DeviceThreadControl), EngineFault> {
    let control = DeviceThreadControl::new();
    let mine = control.clone();
    let thread = std::thread::Builder::new()
        .name("rusty_box devices".into())
        .spawn(move || run_loop(&machine, &clock, &vcpus, &mine))
        .map_err(|error| {
            tracing::error!("a device thread could not be started: {error}");
            EngineFault::new(EngineFaultKind::Host, "the host would not start a device thread")
        })?;
    Ok((thread, control))
}

/// Sleep until the machine's next deadline, service, repeat.
fn run_loop<T: Instrumentation + Send>(
    machine: &Mutex<Box<Emulator<T, crate::WhpEngine>>>,
    clock: &Mutex<VmClockSource<crate::StdClock>>,
    vcpus: &[crate::vcpu_thread::VcpuControl],
    control: &DeviceThreadControl,
) {
    loop {
        // SERVICE FIRST, then wait. The order is load-bearing: a thread that
        // waited first would take its longest wait of all with `next_deadline`
        // still `None` — it does not yet know the machine has an 8042 due in
        // 150 µs — and a condvar wait that long rides the host's 15.6 ms timer
        // tick. Measured before this order was fixed: a step asked for 10 ms
        // of guest time returned after 17 to 22, because the FIRST service of
        // every step landed a whole host tick late. Servicing on entry costs
        // one boundary against a wheel with nothing due, which the boundary's
        // own no-work fast path answers without touching a timer.
        match wait_until_runnable(control) {
            NextAction::Stop => return,
            NextAction::Service => {}
        }

        // machine, then clock — the order every path in this crate takes.
        let outcome = {
            let mut machine = machine.lock().unwrap_or_else(PoisonError::into_inner);
            let clock = clock.lock().unwrap_or_else(PoisonError::into_inner);
            service_once(&mut machine, &clock)
        };

        // Learned from the service that just ran and consumed by the wait
        // below, so the wait never has to guess: every wait of a run already
        // knows what the machine has due.
        let next_deadline = match outcome {
            Ok(time) => {
                if let Some(reason) = time.stop {
                    park_every_vcpu(vcpus, crate::vcpu_thread::Parked::from_stop(reason));
                    control.pause();
                }
                time.next_deadline.map(VmInstant::from_ticks)
            }
            Err(error) => {
                tracing::error!("the machine's device time could not be serviced: {error}");
                park_every_vcpu(
                    vcpus,
                    crate::vcpu_thread::Parked::Fault(rusty_box_core::EngineFault::new(
                        rusty_box_core::EngineFaultKind::Host,
                        "device time",
                    )),
                );
                control.pause();
                None
            }
        };
        control.record_service();

        // Now sleep until whatever is due next. A stop that arrives during it
        // ends the loop here rather than buying one more service against a
        // partition being torn down.
        match wait_for_deadline(control, clock, next_deadline) {
            NextAction::Stop => return,
            NextAction::Service => {}
        }
    }
}

/// Ask every processor to come back, whatever it is doing.
///
/// A cancel the platform refuses is reported and not retried: `request_park`
/// records the reason BEFORE it cancels, so the thread is already bound to
/// park and will do so at its next exit. What is lost is only the promptness,
/// which is worth a line in the log and nothing else — and every other
/// processor must still be asked, so one refusal cannot end the loop.
fn park_every_vcpu(vcpus: &[crate::vcpu_thread::VcpuControl], why: crate::vcpu_thread::Parked) {
    for (index, vcpu) in vcpus.iter().enumerate() {
        match vcpu.request_park(why) {
            Ok(()) => {}
            Err(error) => tracing::error!(
                "vCPU {index} could not be fetched out of its run to park with {why:?}: {error}"
            ),
        }
    }
}

/// Block while the machine is paused.
///
/// The only wait that has no deadline: a paused machine earns no guest time,
/// so there is nothing to be late for.
fn wait_until_runnable(control: &DeviceThreadControl) -> NextAction {
    let (lock, changed) = &*control.wake;
    let mut wake = lock.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        if wake.stop {
            return NextAction::Stop;
        }
        if wake.run {
            return NextAction::Service;
        }
        wake = changed.wait(wake).unwrap_or_else(PoisonError::into_inner);
    }
}

/// Block until `next_deadline`, or until someone asks for something sooner.
///
/// The two-phase wait: everything beyond [`SPIN_MARGIN`] is slept on the
/// condition variable, and the margin is spun with no lock held. Answers what
/// the caller should do next rather than a duration, so "the machine stopped"
/// and "wait zero" cannot be confused.
fn wait_for_deadline<H: HostClock>(
    control: &DeviceThreadControl,
    clock: &Mutex<VmClockSource<H>>,
    next_deadline: Option<VmInstant>,
) -> NextAction {
    let (lock, changed) = &*control.wake;
    let mut wake = lock.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        if wake.stop {
            return NextAction::Stop;
        }
        if !wake.run {
            // Paused mid-wait. The caller's next act is to block on the run
            // flag, which is where a paused machine belongs.
            return NextAction::Service;
        }

        // The soonest of the wheel's own deadline and anything asked for.
        let target = match (next_deadline, wake.earlier_deadline) {
            (Some(wheel), Some(asked)) if asked.ticks() < wheel.ticks() => Some(asked),
            (Some(wheel), _) => Some(wheel),
            (None, asked) => asked,
        };

        // The clock alone, and released before any wait: a thread sleeping
        // with the clock held would stop every other thread reading the time.
        let remaining = match target {
            Some(at) => clock
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .time_until(at),
            // Nothing armed anywhere. Wait for someone to arm something.
            None => None,
        };
        let Some(remaining) = remaining else {
            wake = changed
                .wait(wake)
                .unwrap_or_else(PoisonError::into_inner);
            continue;
        };

        if let Some(sleepable) = remaining.checked_sub(SPIN_MARGIN) {
            let (guard, _timed_out) = changed
                .wait_timeout(wake, sleepable)
                .unwrap_or_else(PoisonError::into_inner);
            wake = guard;
            // Re-read rather than assume the wait timed out: an earlier
            // deadline may have arrived while this thread slept.
            continue;
        }

        // Inside the margin, so the rest is spun with the lock released — a
        // thread arming the next deadline must not queue behind a thread that
        // is deliberately busy. The spin ends on the deadline OR on any change
        // to the state, whichever comes first.
        wake.earlier_deadline = None;
        let generation = control.generation.load(Ordering::Acquire);
        drop(wake);
        let until = Instant::now() + remaining;
        while Instant::now() < until
            && control.generation.load(Ordering::Acquire) == generation
        {
            std::hint::spin_loop();
        }
        return NextAction::Service;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::SharedClock;
    use rusty_box::emulator::{EmulatorConfig, MachineBuilder};
    use rusty_box_core::time::ClockHz;

    /// The service catches the wheel up to exactly where the clock stands, and
    /// names the deadline a thread should sleep until.
    ///
    /// Driven on an INTERPRETER machine: the arithmetic is the engine's
    /// business in neither direction, and an interpreter machine needs no
    /// hypervisor to build — so this half of the device thread is testable on
    /// any host.
    ///
    /// The second service is the one that matters. No host time passed between
    /// them, so a service that credited the machine anything would be earning
    /// ticks the clock has not run.
    #[test]
    fn service_once_catches_the_wheel_up_to_the_clock_and_names_the_next_deadline() {
        let host = SharedClock::default();
        let mut machine = MachineBuilder::new(EmulatorConfig::default())
            .build()
            .expect("an interpreter machine");
        let ips = machine.config().ips.per_second_u64();
        let rate = ClockHz::new(ips).expect("a positive instruction rate");
        let start = machine.ticks();
        let mut clock = VmClockSource::stopped_at(VmInstant::from_ticks(start), rate, host.clone());
        clock.start();

        // 400 µs. At the default 50 MHz that is 20,000 ticks, and the 8042's
        // continuous timer has a 150 µs period, so two periods are due.
        host.advance_nanos(400_000);
        let earned = rate.ticks_from_nanos_floor(400_000).ticks();
        // A bound the test owns rather than a device constant it would have to
        // widen an API to read: every continuous timer a furnished machine
        // arms has a sub-millisecond period, so a deadline further out than
        // this means nothing is armed and the wheel is idling.
        let within = rate.ticks_from_micros_ceil(1_000).ticks();

        let outcome = service_once(&mut machine, &clock).expect("a boundary");
        assert_eq!(
            machine.ticks(),
            start + earned,
            "the wheel is exactly where the clock is"
        );
        let next = outcome
            .next_deadline
            .expect("a continuous device timer is armed");
        assert!(
            next > start + earned && next <= start + earned + within,
            "the next deadline is ahead of now and within {within} ticks of it: {next}"
        );
        assert!(outcome.stop.is_none() && !outcome.reset_applied);

        let again = service_once(&mut machine, &clock).expect("a second boundary");
        assert_eq!(
            machine.ticks(),
            start + earned,
            "no host time passed, no ticks earned"
        );
        assert_eq!(again.next_deadline, Some(next));
    }
}
