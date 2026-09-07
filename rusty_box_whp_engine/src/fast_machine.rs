//! What a machine on the hypervisor IS, once it stops being driven in slices.
//!
//! ## The shape
//!
//! A [`FastMachine`] owns three things and coordinates them: the machine
//! itself behind a lock, a clock that runs only while the guest may run, and
//! the threads. One thread per processor, each of which enters the partition
//! and STAYS there; one device thread, which sleeps until the machine's next
//! device deadline and services it without disturbing any of them.
//!
//! Nothing here steps. `Emulator::step` and the scheduler beneath it belong to
//! the interpreter's machine; a machine adopted here is driven by
//! [`FastMachine::step`], which is a span of guest TIME rather than a count of
//! instructions — the unit a processor running on hardware can actually hold
//! to, since nothing counts its instructions.
//!
//! ## Why the driver is a separate type
//!
//! `MachineBuilder::build_on::<WhpEngine>()` still answers a plain
//! `Box<Emulator<_, WhpEngine>>`, and adoption is the one step after it. That
//! keeps the machine crate free of every thread type: an `Emulator` is a
//! machine, not a scheduler, and the two engines disagree only about who turns
//! it.
//!
//! ## The ordering obligation
//!
//! A `Vcpu` and a `Canceller` name a partition the machine destroys when it
//! drops. Every thread holding one must therefore be joined BEFORE the machine
//! is released — and Rust's field order drops `shared` first, so the obligation
//! is discharged by [`Drop`]'s body, not by how the fields are written.

use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusty_box::cpu::instrumentation::Instrumentation;
use rusty_box::emulator::{DeviceClock, Emulator, RunBudget};
use rusty_box_core::time::{ClockHz, VmDuration, VmInstant};
use rusty_box_core::EngineFault;
use rusty_box_whp::WhpError;

use crate::device_thread::{self, DeviceThreadControl};
use crate::engine::{bring_up, ExitCounts, InjectCensus, WhpEngine};
use crate::vcpu_thread::{Parked, VcpuCensus, VcpuControl, VcpuThread};
use crate::vm_clock::{StdClock, VmClockSource};

/// How long a verb waits for a processor to come back before giving up on it.
///
/// A processor inside `WHvRunVirtualProcessor` answers a cancel in
/// microseconds. Five seconds is not a timing budget — it is the line past
/// which the platform is not going to answer at all, and the machine must say
/// so rather than block its caller forever.
const PARK_BOUND: Duration = Duration::from_secs(5);

/// The shortest wall-clock allowance any step gets.
///
/// A step for a handful of ticks still has to pay for a thread wake, a park
/// and a resume, and none of that scales with the budget.
const MINIMUM_STEP_ALLOWANCE: Duration = Duration::from_secs(1);

/// How far past its honest wall-clock cost a step may run before it is called
/// wedged.
///
/// Ten times, because a guest on hardware runs FASTER than its nominal rate,
/// not slower: the only way to overshoot by this much is for something to have
/// stopped answering.
const WEDGE_FACTOR: u64 = 10;

/// Why a fast-machine verb refused (R0).
#[derive(Debug)]
pub enum FastMachineFault {
    /// The machine or its engine refused work.
    Engine(EngineFault),
    /// A processor did not come back within [`PARK_BOUND`].
    ///
    /// Terminal for the step that saw it: the machine has a thread inside a
    /// platform call that is not returning, and nothing this side of the
    /// platform can end it.
    Wedged {
        /// How long was actually waited before giving up.
        waited: Duration,
    },
    /// A budget in instructions, which nothing here counts.
    ///
    /// A processor on hardware retires instructions the host does not tally,
    /// so the request cannot be honoured and is refused rather than
    /// approximated — an approximate instruction count is worse than none,
    /// because a caller cannot tell it from a real one.
    NoInstructionCount,
    /// The machine keeps its device time in ticks, so it has no host-time
    /// clock for these threads to run against.
    DeviceClockIsTicks,
    /// The platform refused.
    Platform(WhpError),
}

impl core::fmt::Display for FastMachineFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Engine(fault) => write!(f, "{fault}"),
            Self::Wedged { waited } => write!(
                f,
                "a processor did not come back within {:.1}s",
                waited.as_secs_f32()
            ),
            Self::NoInstructionCount => {
                write!(f, "a machine on the hypervisor cannot count instructions; budget it in ticks")
            }
            Self::DeviceClockIsTicks => write!(
                f,
                "a fast machine needs `DeviceClock::HostTime`; this one keeps device time in ticks"
            ),
            Self::Platform(error) => write!(f, "the platform refused: {error}"),
        }
    }
}

impl std::error::Error for FastMachineFault {}

impl From<WhpError> for FastMachineFault {
    fn from(error: WhpError) -> Self {
        Self::Platform(error)
    }
}

/// How far a step got and why it stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StepOutcome {
    /// Guest ticks that passed, measured on the machine's own clock.
    pub ticks: u64,
    /// What ended the step.
    pub stop: StepStop,
}

/// What ended a step (R0/R2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepStop {
    /// The requested span of guest time passed.
    BudgetSpent,
    /// The guest turned the machine off. Stepping again runs a machine that
    /// asked to be off.
    GuestPowerOff,
    /// A processor or the device thread could not carry on.
    Faulted(EngineFault),
}

/// The census of a fast machine — one struct, named fields (R0).
#[derive(Clone, Debug)]
pub struct EngineCensus {
    /// What left the partition, and why.
    pub exits: ExitCounts,
    /// One entry per processor, in processor order.
    pub vcpus: Vec<VcpuCensus>,
    /// What was placed in the partition's pending-event slot.
    pub injections: InjectCensus,
}

/// Whether the guest's threads are running (R2).
///
/// A state rather than a bool, so `resume` on a running machine and `pause` on
/// a paused one are both plainly idempotent and neither can double-suspend the
/// partition's clock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RunState {
    Paused,
    Running,
}

/// A machine whose guest runs on hardware, unbounded, on threads of its own.
pub struct FastMachine<T: Instrumentation + Send = ()> {
    shared: Arc<Mutex<Box<Emulator<T, WhpEngine>>>>,
    clock: Arc<Mutex<VmClockSource<StdClock>>>,
    vcpus: Vec<(JoinHandle<()>, VcpuControl)>,
    devices: Option<(JoinHandle<()>, DeviceThreadControl)>,
    state: RunState,
}

impl<T: Instrumentation + Send + 'static> FastMachine<T> {
    /// Take a built machine and start running it on hardware.
    ///
    /// Starts the partition, spawns a thread per processor and the device
    /// thread, and hands back a machine that is PAUSED: nothing runs until
    /// [`resume`](Self::resume) or [`step`](Self::step) says so, which is what
    /// lets a caller finish arranging the guest after adoption.
    ///
    /// # Errors
    /// [`FastMachineFault::DeviceClockIsTicks`] for a machine that keeps its
    /// device time in ticks — such a machine has no host-time clock for these
    /// threads to run against, and adopting it would silently freeze its
    /// devices. Otherwise whatever starting the partition or a thread refused.
    pub fn adopt(machine: Box<Emulator<T, WhpEngine>>) -> Result<Self, FastMachineFault> {
        if machine.device_clock() != DeviceClock::HostTime {
            return Err(FastMachineFault::DeviceClockIsTicks);
        }
        let rate = ClockHz::new(machine.config().ips.per_second_u64())
            .ok_or_else(|| {
                FastMachineFault::Engine(EngineFault::new(
                    rusty_box_core::EngineFaultKind::Host,
                    "a machine with no instruction rate has no clock",
                ))
            })?;
        let clock = Arc::new(Mutex::new(VmClockSource::stopped_at(
            VmInstant::from_ticks(machine.ticks()),
            rate,
            StdClock::new(),
        )));

        let shared = Arc::new(Mutex::new(machine));
        let mut vcpus = Vec::new();
        {
            let mut machine = shared.lock().unwrap_or_else(PoisonError::into_inner);
            let vcpu = bring_up(&mut machine).map_err(engine_refused)?;
            drop(machine);
            let (join, control) =
                VcpuThread::spawn(vcpu, 0, Arc::clone(&shared), Arc::clone(&clock))
                    .map_err(engine_refused)?;
            let mut machine = shared.lock().unwrap_or_else(PoisonError::into_inner);
            // The engine is how the machine's own boundary reaches the thread:
            // an 8259 edge becomes a cancel through `pic_pin_changed`, which
            // does nothing at all without a control installed here.
            machine.engine_mut().install_control(control.clone());
            vcpus.push((join, control));
        }

        let controls = vcpus.iter().map(|(_, control)| control.clone()).collect();
        let (join, devices) =
            device_thread::spawn(Arc::clone(&shared), Arc::clone(&clock), controls)
                .map_err(FastMachineFault::Engine)?;

        Ok(Self {
            shared,
            clock,
            vcpus,
            devices: Some((join, devices)),
            state: RunState::Paused,
        })
    }

    /// Let the guest run.
    ///
    /// Idempotent. The partition's own clock resumes with the machine's, so a
    /// guest reading its TSC across a pause sees the pause it did not live
    /// through as no time at all.
    ///
    /// # Errors
    /// Whatever the platform said about resuming its clock.
    pub fn resume(&mut self) -> Result<(), FastMachineFault> {
        if self.state == RunState::Running {
            return Ok(());
        }
        {
            let machine = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(partition) = machine.engine().partition() {
                partition.resume_time()?;
            }
            let mut clock = self.clock.lock().unwrap_or_else(PoisonError::into_inner);
            clock.start();
        }
        if let Some((_, devices)) = self.devices.as_ref() {
            devices.resume();
        }
        for (_, control) in &self.vcpus {
            control.resume();
        }
        self.state = RunState::Running;
        Ok(())
    }

    /// Bring every processor back and stop the clock.
    ///
    /// Idempotent. No lock is held while waiting for a processor: the thread
    /// being waited for has to take the machine's lock to finish its exit, and
    /// a waiter holding it would deadlock the pause it asked for.
    ///
    /// # Errors
    /// [`FastMachineFault::Wedged`] if a processor did not come back within
    /// [`PARK_BOUND`], or whatever the platform said about suspending its
    /// clock.
    pub fn pause(&mut self) -> Result<(), FastMachineFault> {
        if self.state == RunState::Paused {
            return Ok(());
        }
        let waited = self.park_every_processor()?;
        if let Some((_, devices)) = self.devices.as_ref() {
            devices.pause();
        }
        {
            let machine = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            let mut clock = self.clock.lock().unwrap_or_else(PoisonError::into_inner);
            clock.stop();
            if let Some(partition) = machine.engine().partition() {
                partition.suspend_time()?;
            }
        }
        self.state = RunState::Paused;
        let _ = waited;
        Ok(())
    }

    /// Run the guest for a span of its own time, then pause.
    ///
    /// The step's end is published to the device thread as a deadline, so the
    /// thread wakes at the target and bumps its service generation there
    /// rather than at whatever device deadline happens to be next. That is
    /// what makes a step end on time for a guest with nothing else due.
    ///
    /// No machine lock is held across any wait.
    ///
    /// # Errors
    /// [`FastMachineFault::NoInstructionCount`] for an instruction budget, and
    /// [`FastMachineFault::Wedged`] for a step whose wall clock ran ten times
    /// past its honest cost — see [`WEDGE_FACTOR`].
    pub fn step(&mut self, budget: RunBudget) -> Result<StepOutcome, FastMachineFault> {
        let RunBudget::Ticks(ticks) = budget else {
            return Err(FastMachineFault::NoInstructionCount);
        };
        let ips = {
            let machine = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            machine.config().ips.per_second_u64().max(1)
        };
        self.resume()?;

        let start = self.now();
        let target = start.add(VmDuration::from_ticks(ticks));
        let mut served = match self.devices.as_ref() {
            Some((_, devices)) => {
                devices.deadline_moved_earlier(target);
                devices.served()
            }
            None => 0,
        };
        let began = Instant::now();
        let allowance = MINIMUM_STEP_ALLOWANCE.max(Duration::from_nanos(
            ticks
                .saturating_mul(WEDGE_FACTOR)
                .saturating_mul(1_000_000_000)
                / ips,
        ));

        let stop = loop {
            if let Some((_, devices)) = self.devices.as_ref() {
                if let Some(now) = devices.wait_served_by(served, Duration::from_millis(10)) {
                    served = now;
                }
            }
            if self.now().ticks() >= target.ticks() {
                break StepStop::BudgetSpent;
            }
            if let Some(stop) = self.a_processor_gave_up() {
                break stop;
            }
            if began.elapsed() > allowance {
                // Best-effort: the machine is already not answering, and a
                // pause that also fails must not mask the wedge that caused
                // it. Reported either way.
                match self.pause() {
                    Ok(()) | Err(_) => {}
                }
                return Err(FastMachineFault::Wedged {
                    waited: began.elapsed(),
                });
            }
        };

        // The span is measured after the pause, not at the break: the clock
        // stops inside `pause`, and the guest really did run for the few
        // microseconds its processor took to come back. Measured at 3,000 to
        // 12,000 ticks on a 500,000-tick step — 0.06 to 0.24 ms.
        self.pause()?;
        Ok(StepOutcome {
            ticks: self.now().since(start).ticks(),
            stop,
        })
    }

    /// Reach into the machine while it is paused.
    ///
    /// For the reads a driver and a test do between steps — the display, the
    /// debug port, guest memory, a register. **Paused only:** a read taken
    /// while a processor is inside the partition answers from a shadow that
    /// processor is not using, which is not a lie the machine can detect.
    pub fn with_machine<R>(&mut self, f: impl FnOnce(&mut Emulator<T, WhpEngine>) -> R) -> R {
        debug_assert_eq!(
            self.state,
            RunState::Paused,
            "the machine must be paused before it is read: a running processor's \
             state is the partition's, not this shadow's"
        );
        let mut machine = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut machine)
    }

    /// Reach into the machine while its guest is RUNNING, then tell the device
    /// thread to look again.
    ///
    /// For host input — a keystroke, a mouse packet — which arrives on the
    /// front end's thread whenever the person at the keyboard presses a key,
    /// not at a moment the guest chose. The machine's lock is taken and
    /// released here; the processors are not disturbed.
    ///
    /// **The poke is the point.** Latching a byte in the 8042 arms nothing by
    /// itself: the one-shot is evaluated when device time is serviced
    /// (`Emulator::service_device_time`, divergence H6), and a device thread
    /// asleep until a distant deadline would not evaluate it until then. At an
    /// idle shell prompt that is a keystroke that arrives seconds late, or —
    /// with nothing else armed at all — never.
    pub fn with_machine_while_running<R>(
        &mut self,
        f: impl FnOnce(&mut Emulator<T, WhpEngine>) -> R,
    ) -> R {
        let answer = {
            let mut machine = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            f(&mut machine)
        };
        if let Some((_, devices)) = self.devices.as_ref() {
            devices.deadline_moved_earlier(self.now());
        }
        answer
    }

    /// What the threads and the partition have done. Touches no processor.
    ///
    /// Safe to call while the guest runs: every number comes from a shared
    /// counter or from the machine's own engine under a brief lock, and none
    /// of it is a platform call against a processor another thread is inside.
    #[must_use]
    pub fn engine_census(&self) -> EngineCensus {
        let vcpus = self
            .vcpus
            .iter()
            .map(|(_, control)| control.census())
            .collect();
        let machine = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        let engine = machine.engine();
        EngineCensus {
            exits: engine.exits(),
            vcpus,
            injections: engine.inject_census().clone(),
        }
    }

    /// The guest's time, now.
    fn now(&self) -> VmInstant {
        self.clock
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .now()
    }

    /// Whether any processor has stopped for a reason a step must report.
    ///
    /// Asked without blocking — a zero wait reads the slot and returns — so a
    /// step's loop can ask it every turn.
    fn a_processor_gave_up(&self) -> Option<StepStop> {
        self.vcpus.iter().find_map(|(_, control)| {
            match control.wait_parked_by(Duration::ZERO) {
                Some(Parked::GuestPowerOff) => Some(StepStop::GuestPowerOff),
                Some(Parked::Fault(fault)) => Some(StepStop::Faulted(fault)),
                // A pause is this machine's own doing, and a stop is its
                // teardown. Neither ends a step.
                Some(Parked::Paused | Parked::Stopped) | None => None,
            }
        })
    }

    /// Ask every processor to park and wait for all of them.
    ///
    /// Every processor is asked BEFORE any is waited for, so the waits overlap
    /// rather than serialise: a machine with four processors pays one bound,
    /// not four.
    fn park_every_processor(&self) -> Result<Duration, FastMachineFault> {
        for (index, (_, control)) in self.vcpus.iter().enumerate() {
            match control.request_park(Parked::Paused) {
                Ok(()) => {}
                Err(error) => tracing::error!("vCPU {index} would not take a cancel: {error}"),
            }
        }
        let began = Instant::now();
        for (_, control) in &self.vcpus {
            let left = PARK_BOUND.saturating_sub(began.elapsed());
            if control.wait_parked_by(left).is_none() {
                return Err(FastMachineFault::Wedged {
                    waited: began.elapsed(),
                });
            }
        }
        Ok(began.elapsed())
    }
}

impl<T: Instrumentation + Send> Drop for FastMachine<T> {
    /// Stop the threads, join them, and only then let the machine go.
    ///
    /// **The body discharges the ordering obligation, not the field order.**
    /// `shared` is declared first, so dropping by field order would release
    /// the machine — and with it the partition — before the threads that hold
    /// a `Vcpu` and a `Canceller` naming it.
    fn drop(&mut self) {
        // Ask everyone to stop before joining anyone: the device thread and
        // the processors wake each other, and a join taken before the ask
        // would wait on a thread still sleeping until a deadline.
        if let Some((_, devices)) = self.devices.as_ref() {
            devices.stop();
        }
        for (index, (_, control)) in self.vcpus.iter().enumerate() {
            match control.stop() {
                Ok(()) => {}
                Err(error) => {
                    tracing::error!("vCPU {index} would not take the stop cancel: {error}");
                }
            }
        }

        if let Some((join, _)) = self.devices.take() {
            match join.join() {
                Ok(()) => {}
                Err(_) => tracing::error!("the device thread panicked"),
            }
        }

        // A processor that answered its stop is joined. One that did not is
        // DETACHED rather than joined: it is inside a platform call that is
        // not returning, and joining it would hang the drop forever. Its next
        // platform call fails with a handle the partition has reclaimed, and
        // it returns then.
        let began = Instant::now();
        for (join, control) in std::mem::take(&mut self.vcpus) {
            let left = PARK_BOUND.saturating_sub(began.elapsed());
            if control.wait_parked_by(left).is_none() {
                tracing::error!(
                    "a vCPU thread did not leave its run within {:.1}s and is detached",
                    PARK_BOUND.as_secs_f32()
                );
                continue;
            }
            match join.join() {
                Ok(()) => {}
                Err(_) => tracing::error!("a vCPU thread panicked"),
            }
        }
    }
}

/// A machine or engine refusal, in this driver's vocabulary.
fn engine_refused(error: rusty_box::cpu::CpuError) -> FastMachineFault {
    match error {
        rusty_box::cpu::CpuError::EngineFault(fault) => FastMachineFault::Engine(fault),
        other => FastMachineFault::Engine(EngineFault::new(
            rusty_box_core::EngineFaultKind::Host,
            "the machine refused to start on hardware",
        ))
        .tagged(other),
    }
}

impl FastMachineFault {
    /// Keep the refusal's own words in the log when it had no [`EngineFault`]
    /// of its own to carry them.
    fn tagged(self, cause: rusty_box::cpu::CpuError) -> Self {
        tracing::error!("the machine refused to start on hardware: {cause:?}");
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{a_turn_on_the_hardware, hypervisor_here, machine_with_devices_on};

    /// `jmp $` — a guest that asks the hardware for nothing at all.
    ///
    /// The whole point: it takes no exit of its own, so every number below is
    /// this machine's own cost rather than the guest's.
    const SPIN: [u8; 2] = [0xEB, 0xFE];

    /// A step of guest time runs the guest for that much of it, and the wheel
    /// follows.
    ///
    /// Three properties in one, because they are only meaningful together: the
    /// step lasts as long as it was asked to, the machine's device wheel is
    /// where the clock is when it ends, and the processor spent that time
    /// INSIDE the partition rather than bouncing in and out of it. The last is
    /// what the whole design is for — under the slice model this guest cost
    /// one entry and one exit per slice and got under 1% of the wall clock.
    #[test]
    fn step_in_ticks_runs_the_guest_for_that_much_vm_time_and_pauses() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine =
            FastMachine::adopt(machine_with_devices_on(DeviceClock::HostTime, &SPIN))
                .expect("a machine on hardware");
        let ips = machine.with_machine(|m| m.config().ips.per_second_u64());
        let ten_ms = ips / 100;

        let outcome = machine
            .step(RunBudget::Ticks(ten_ms))
            .expect("a step of ten milliseconds");

        assert!(
            outcome.ticks >= ten_ms,
            "a step reports at least the time it was asked for: {outcome:?}"
        );
        assert!(
            outcome.ticks < ten_ms + ips / 500,
            "and overshoots by at most one device-thread wake (2 ms): {outcome:?}"
        );
        assert_eq!(outcome.stop, StepStop::BudgetSpent);
        assert!(
            machine.with_machine(|m| m.ticks()) >= ten_ms,
            "the wheel followed the clock rather than standing still"
        );

        let census = machine.engine_census();
        let vcpu = census.vcpus[0];
        // Half the step, which is not the number this design achieves — it
        // measures 9.48 to 10.10 ms of a 10 ms step, 95% and over — but the
        // number that DISTINGUISHES it. The slice model this replaced left
        // under 1% of the wall clock inside the run, so half is two orders of
        // magnitude above the behaviour under test and falsifies it outright,
        // while a 90% threshold only measured how busy the host was: it flaked
        // when the step ran on a machine still finishing a compile.
        assert!(
            vcpu.in_run_nanos >= 5_000_000,
            "the processor stayed inside WHvRunVirtualProcessor for most of the 10 ms \
             rather than bouncing in and out of it: {vcpu:?}"
        );
        let platform = vcpu
            .platform_at_last_park
            .expect("the platform's counters are refreshed at every park");
        let guest_100ns = platform
            .runtime
            .total_100ns
            .saturating_sub(platform.runtime.hypervisor_100ns);
        assert!(
            guest_100ns > platform.runtime.hypervisor_100ns,
            "cross-check against the hypervisor's own accounting: guest time exceeds \
             its overhead: {platform:?}"
        );
        assert!(
            census.exits.canceled <= 1 && census.exits.total() <= 2,
            "a spinning guest leaves the partition only for the pause's own cancel: {:?}",
            census.exits
        );
    }

    /// A machine that keeps device time in ticks has no host-time clock for
    /// these threads to run against, and is refused rather than adopted with
    /// its devices silently frozen.
    ///
    /// Needs no hypervisor: the check is made before the partition is started,
    /// which is itself the property — a caller learns it configured the wrong
    /// machine without first paying for hardware.
    #[test]
    fn a_machine_on_tick_time_is_refused_rather_than_adopted() {
        let machine = machine_with_devices_on(DeviceClock::Ticks, &SPIN);
        match FastMachine::adopt(machine) {
            Err(FastMachineFault::DeviceClockIsTicks) => {}
            Err(other) => panic!("refused for the wrong reason: {other}"),
            Ok(_) => panic!("a tick-time machine must not be adopted"),
        }
    }

    /// A keystroke the HOST queues arms the 8042's one-shot, although the
    /// guest touched no port.
    ///
    /// This is the whole of why the arming lives in `service_device_time` and
    /// not in the port-dispatch tail (divergence H6). Two of `activate_timer`'s
    /// six callers in `keyboard.rs` — `kbd_enQ` and `mouse_enQ` — are reached
    /// when the host queues input, which no guest port write passes through.
    /// A machine that armed only where the guest touched a port would leave
    /// this byte latched and IRQ1 unraised until some unrelated port access
    /// happened along: at an idle prompt, a dead keyboard.
    ///
    /// The guest here is `jmp $` — it touches nothing at all, which is what
    /// makes the arming attributable to the service rather than to it.
    #[test]
    fn a_keystroke_the_host_queues_arms_the_8042_although_the_guest_touched_no_port() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine =
            FastMachine::adopt(machine_with_devices_on(DeviceClock::HostTime, &SPIN))
                .expect("a machine on hardware");
        let ips = machine.with_machine(|m| m.config().ips.per_second_u64());

        // A host-time machine arms nothing for the 8042 until something is
        // owed — that is the divergence, and it is the premise of the rest.
        // A millisecond of guest time is nearly seven serial-delay periods, so
        // a machine that armed the Bochs continuous timer would tick here.
        machine
            .step(RunBudget::Ticks(ips / 1_000))
            .expect("a millisecond of guest time");
        assert_eq!(
            machine.with_machine(|m| m.keyboard_serial_ticks()),
            0,
            "with nothing latched, the 8042 is not ticking at all"
        );

        machine.with_machine_while_running(|m| {
            assert!(
                m.keyboard().tap(rusty_box::iodev::scancodes::BxKey::A),
                "the 8042 took the keystroke"
            );
        });
        machine
            .step(RunBudget::Ticks(ips / 1_000))
            .expect("a second millisecond of guest time");

        assert!(
            machine.with_machine(|m| m.keyboard_serial_ticks()) > 0,
            "the service armed the one-shot for the latched byte and a tick \
             carried it — although the guest executed nothing but a jump to \
             itself, so no port write could have armed anything"
        );
    }

    /// An instruction budget is refused, not approximated.
    ///
    /// Nothing counts a hardware processor's instructions, and an approximate
    /// count is worse than none: a caller cannot tell it from a real one.
    #[test]
    fn an_instruction_budget_is_refused() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine =
            FastMachine::adopt(machine_with_devices_on(DeviceClock::HostTime, &SPIN))
                .expect("a machine on hardware");
        match machine.step(RunBudget::Instructions(1_000)) {
            Err(FastMachineFault::NoInstructionCount) => {}
            Err(other) => panic!("refused for the wrong reason: {other}"),
            Ok(outcome) => panic!("an instruction budget must not be honoured: {outcome:?}"),
        }
    }
}
