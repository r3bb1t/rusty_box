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

use rusty_box::cpu::arch_state::{ArchGroups, VcpuArchState};
use rusty_box::cpu::instrumentation::{Instrumentation, X86Reg};
use rusty_box::emulator::{DeviceClock, Emulator, RunBudget};
use rusty_box_core::time::{ClockHz, VmDuration, VmInstant};
use rusty_box_core::EngineFault;
use rusty_box_whp::WhpError;

use crate::device_thread::{self, DeviceThreadControl};
use crate::engine::{bring_up, ExitCounts, InjectCensus, WhpEngine};
use crate::vcpu_thread::{Parked, RegistersAtPark, VcpuCensus, VcpuControl, VcpuThread};
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
    /// The machine was not paused, so its processors' registers are the
    /// partition's and not the machine's to read or write.
    ///
    /// A guest that was resumed and not paused again, or a pause whose
    /// processor did not come back. Refused before the closure runs.
    NotPaused,
    /// A processor parked with its registers still in the partition: the
    /// platform would not give them up, so the machine's copy of that
    /// processor is not the guest's. Refused before the closure runs.
    RegistersNotInTheMachine {
        /// Which processor.
        processor: usize,
    },
    /// The closure wrote a register this engine does not carry into the
    /// partition, and the write was undone.
    ///
    /// Everything else the closure did stands — its writes to registers this
    /// engine does carry still reach the guest at the next entry. Only the
    /// named register was put back to the value the guest holds.
    Uncarried {
        /// The register whose write was undone.
        register: X86Reg,
        /// The processor it belongs to.
        processor: usize,
    },
    /// The closure wrote a model-specific register this engine does not carry
    /// into the partition, and the write was undone — as
    /// [`Self::Uncarried`], for a register named by its MSR index rather
    /// than by an [`X86Reg`].
    UncarriedMsr {
        /// The MSR index whose write was undone.
        msr: u32,
        /// The processor it belongs to.
        processor: usize,
    },
    /// The closure cleared `IF` while an external interrupt was already
    /// placed for delivery, and `IF` was set again.
    ///
    /// The vector was acknowledged at the 8259 on positive evidence that the
    /// guest could take it, and the platform refuses an entry that would carry
    /// it into a guest that cannot. The guest takes it at its next entry; `IF`
    /// can be cleared at a pause after that. Everything else the closure did
    /// stands, the rest of the flags included.
    InterruptAlreadyPlaced {
        /// The vector the guest takes at its next entry.
        vector: u8,
        /// The processor it is placed for.
        processor: usize,
    },
    /// The closure changed a processor's registers from the running path,
    /// and the change was undone.
    ///
    /// A running processor's registers are the partition's. A write to the
    /// machine's copy would be overwritten when the processor next parks, and
    /// until then an exit's errand would run against it. Registers are written
    /// through [`FastMachine::with_machine`], on a paused machine.
    WrittenWhileRunning {
        /// Which processor.
        processor: usize,
    },
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
            Self::NotPaused => write!(
                f,
                "the machine is not paused, so its processors' registers are the partition's"
            ),
            Self::RegistersNotInTheMachine { processor } => write!(
                f,
                "processor {processor} parked without its registers: the platform would not \
                 give them up"
            ),
            Self::Uncarried { register, processor } => write!(
                f,
                "processor {processor}'s {register:?} is not carried into the partition on this \
                 engine; the write was undone"
            ),
            Self::UncarriedMsr { msr, processor } => write!(
                f,
                "processor {processor}'s MSR {msr:#x} is not carried into the partition on this \
                 engine; the write was undone"
            ),
            Self::InterruptAlreadyPlaced { vector, processor } => write!(
                f,
                "vector {vector:#04x} is already placed for processor {processor}, which must \
                 take it at its next entry; IF was set again"
            ),
            Self::WrittenWhileRunning { processor } => write!(
                f,
                "processor {processor}'s registers were written from the running path; the \
                 write was undone — write them through `with_machine` on a paused machine"
            ),
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
    /// A processor is in the shutdown state: it triple-faulted on a machine
    /// set to shut down on one (`OnTripleFault::ShutDown`). Only a reset
    /// brings it back.
    CpuShutdown,
    /// A processor or the device thread could not carry on.
    Faulted(EngineFault),
}

/// The census of a fast machine — one struct, named fields (R0).
#[derive(Clone, Debug)]
pub struct EngineCensus {
    /// What left the partition, and why, summed over every processor.
    ///
    /// Built from the per-processor tallies below rather than read off the
    /// engine: each processor's thread counts its own exits, and there is no
    /// longer a single loop through which they all pass. Reading the engine's
    /// own field here would report zero forever — which it did, until a
    /// migrated test asserted a memory exit it could not see.
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
    /// lets a caller finish arranging the guest after adoption. Each
    /// processor's thread is born parked and has parked by the time this
    /// returns, so the pause is a fact rather than a label.
    ///
    /// The guest starts on the machine's own processor: whatever the machine
    /// was reset, set up or restored to, and whatever a caller writes through
    /// [`with_machine`](Self::with_machine) before the first step, is
    /// installed in the partition before the processor first enters it.
    ///
    /// # Errors
    /// [`FastMachineFault::DeviceClockIsTicks`] for a machine that keeps its
    /// device time in ticks — such a machine has no host-time clock for these
    /// threads to run against, and adopting it would silently freeze its
    /// devices. [`FastMachineFault::Wedged`] for a processor that did not park.
    /// Otherwise whatever starting the partition or a thread refused.
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

        // Assembled before the rest can fail, so a refusal below drops a
        // machine whose threads [`Drop`] stops and joins, rather than leaving
        // a born-parked processor holding the partition for the life of the
        // process.
        let mut adopted = Self {
            shared,
            clock,
            vcpus,
            devices: None,
            state: RunState::Paused,
        };
        let controls = adopted.vcpus.iter().map(|(_, control)| control.clone()).collect();
        adopted.devices = Some(
            device_thread::spawn(Arc::clone(&adopted.shared), Arc::clone(&adopted.clock), controls)
                .map_err(FastMachineFault::Engine)?,
        );
        adopted.wait_for_every_processor_to_park()?;
        Ok(adopted)
    }

    /// Let the guest run.
    ///
    /// Idempotent. The partition's own clock resumes with the machine's, so a
    /// guest reading its TSC across a pause sees the pause it did not live
    /// through as no time at all. Whatever registers were written through
    /// [`with_machine`](Self::with_machine) while paused, each processor's
    /// thread installs in the partition before it next enters it.
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
    /// Idempotent. Each processor reads its registers out of the partition as
    /// it parks, so once this returns, [`with_machine`](Self::with_machine)
    /// reads the guest's registers rather than a copy the guest stopped using.
    ///
    /// No lock is held while waiting for a processor: the thread being waited
    /// for has to take the machine's lock to finish its exit and to read its
    /// registers back, and a waiter holding it would deadlock the pause it
    /// asked for.
    ///
    /// # Errors
    /// [`FastMachineFault::Wedged`] if a processor did not come back within
    /// [`PARK_BOUND`] — the machine then stays running, and every verb that
    /// needs it paused refuses — or whatever the platform said about
    /// suspending its clock.
    pub fn pause(&mut self) -> Result<(), FastMachineFault> {
        if self.state == RunState::Paused {
            return Ok(());
        }
        self.ask_every_processor_to_park();
        self.wait_for_every_processor_to_park()?;
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
    /// For what a driver and a test do between steps — the display, the debug
    /// port, guest memory, a register. Every processor read its registers out
    /// of the partition when it parked, so a register read here is the
    /// guest's, and a register written here is installed in the partition
    /// before the processor next enters it. That holds for every group the
    /// engine carries: the general registers, `RIP` and the flags, the
    /// segments with their bases, the control, debug and descriptor-table
    /// registers, `EFER` and the model-specific registers carried beside it,
    /// and the x87 and vector file.
    ///
    /// A closure that resets the machine replaces every processor whole: the
    /// reset processor, its local APIC and the three registers below go into
    /// the partition before it next runs, and nothing it changed is refused.
    /// Otherwise three writes cannot be applied, and are undone and refused
    /// rather than kept in a copy the guest never reads:
    /// - the time-stamp counter, which reads as the guest's but which the
    ///   hardware owns on this engine;
    /// - `IA32_TSC_AUX` and `IA32_TSC_DEADLINE`, which the platform answers
    ///   itself and the exchange does not carry;
    /// - clearing `IF` while an external interrupt is already placed for
    ///   delivery, which the platform would refuse to enter with.
    ///
    /// # Errors
    /// Refused before the closure runs:
    /// - [`FastMachineFault::NotPaused`] for a machine whose guest is running,
    ///   or whose last pause did not bring every processor back;
    /// - [`FastMachineFault::RegistersNotInTheMachine`] for a processor that
    ///   parked without its registers.
    ///
    /// Refused after it ran: [`FastMachineFault::Uncarried`],
    /// [`FastMachineFault::UncarriedMsr`] or
    /// [`FastMachineFault::InterruptAlreadyPlaced`] for a closure that made
    /// one of the writes above. Every such write, on every processor, is put
    /// back; the closure's answer is dropped, and everything else it did
    /// stands. The first refusal is returned and any others are logged.
    pub fn with_machine<R>(
        &mut self,
        f: impl FnOnce(&mut Emulator<T, WhpEngine>) -> R,
    ) -> Result<R, FastMachineFault> {
        let placed = self.every_processor_is_in_the_machine()?;
        let mut machine = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        let held: Vec<ProcessorAsItStood> = (0..self.vcpus.len())
            .map(|processor| ProcessorAsItStood::of(&mut machine, processor))
            .collect();
        let answer = f(&mut machine);
        let mut verdict = Ok(answer);
        for (processor, (stood, placed)) in held.iter().zip(placed).enumerate() {
            // A reset the closure made replaces the whole processor at its
            // next entry — the counter, `IA32_TSC_AUX`, `IA32_TSC_DEADLINE`
            // and the pending event with it — so nothing it changed is a
            // write the partition cannot take.
            if self.processor_reset_is_owed(processor) {
                continue;
            }
            for refusal in stood.put_back_what_the_partition_cannot_take(&mut machine, processor, placed)
            {
                match verdict {
                    Ok(_) => verdict = Err(refusal),
                    Err(_) => tracing::error!("and besides: {refusal}"),
                }
            }
        }
        verdict
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
    /// with nothing else armed at all — never. The poke is made whether or not
    /// the closure was refused: the input it queued is real either way.
    ///
    /// **Registers are not this closure's business.** A running processor's
    /// registers are the partition's, so a register read here answers from a
    /// copy the guest is not using, and a register written here would be
    /// overwritten when the processor next parks — and until then, an exit's
    /// errand would run against it. The closure's effect on every processor's
    /// architectural state is therefore checked, and a change is undone and
    /// refused, whether or not the guest happens to be running. Registers are
    /// read and written through [`with_machine`](Self::with_machine). A reset
    /// of the machine is the exception: it replaces each processor whole,
    /// fetched out of its run and installed before it runs again.
    ///
    /// # Errors
    /// [`FastMachineFault::WrittenWhileRunning`] for a closure that changed a
    /// processor's registers. The change is undone and the closure's answer
    /// dropped; everything else it did stands.
    pub fn with_machine_while_running<R>(
        &mut self,
        f: impl FnOnce(&mut Emulator<T, WhpEngine>) -> R,
    ) -> Result<R, FastMachineFault> {
        let verdict = {
            let mut machine = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            let held: Vec<ProcessorAsItStood> = (0..self.vcpus.len())
                .map(|processor| ProcessorAsItStood::of(&mut machine, processor))
                .collect();
            let answer = f(&mut machine);
            // Every changed processor is put back, not only the first: the
            // refusal names one, and the undoing is owed to all of them.
            let mut verdict = Ok(answer);
            for (processor, stood) in held.iter().enumerate() {
                // A reset is the machine's, not a register write: the
                // processor it produced is installed at the next entry, and
                // undoing its registers would install half of one.
                if stood.still_stands(&mut machine, processor)
                    || self.processor_reset_is_owed(processor)
                {
                    continue;
                }
                let refusal = match stood.stand_again(&mut machine, processor) {
                    Ok(()) => FastMachineFault::WrittenWhileRunning { processor },
                    Err(fault) => fault,
                };
                match verdict {
                    Ok(_) => verdict = Err(refusal),
                    Err(_) => tracing::error!("and besides: {refusal}"),
                }
            }
            verdict
        };
        if let Some((_, devices)) = self.devices.as_ref() {
            devices.deadline_moved_earlier(self.now());
        }
        verdict
    }

    /// What the threads and the partition have done. Touches no processor.
    ///
    /// Safe to call while the guest runs: every number comes from a shared
    /// counter or from the machine's own engine under a brief lock, and none
    /// of it is a platform call against a processor another thread is inside.
    #[must_use]
    pub fn engine_census(&self) -> EngineCensus {
        let vcpus: Vec<VcpuCensus> = self
            .vcpus
            .iter()
            .map(|(_, control)| control.census())
            .collect();
        let mut exits = ExitCounts::default();
        for vcpu in &vcpus {
            exits.absorb(vcpu.exits);
        }
        let machine = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        EngineCensus {
            exits,
            vcpus,
            injections: machine.engine().inject_census().clone(),
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
    /// Whether the machine was reset since `processor` last entered the
    /// partition, so its thread installs the reset processor before the next.
    fn processor_reset_is_owed(&self, processor: usize) -> bool {
        self.vcpus
            .get(processor)
            .is_some_and(|(_, control)| control.reset_is_owed())
    }

    fn a_processor_gave_up(&self) -> Option<StepStop> {
        self.vcpus.iter().find_map(|(_, control)| {
            match control.wait_parked_by(Duration::ZERO) {
                Some(Parked::GuestPowerOff) => Some(StepStop::GuestPowerOff),
                Some(Parked::CpuShutdown) => Some(StepStop::CpuShutdown),
                Some(Parked::Fault(fault)) => Some(StepStop::Faulted(fault)),
                // A pause is this machine's own doing, and a stop is its
                // teardown. Neither ends a step.
                Some(Parked::Paused | Parked::Stopped) | None => None,
            }
        })
    }

    /// Ask every processor to park.
    ///
    /// Every processor is asked BEFORE any is waited for, so the waits in
    /// [`Self::wait_for_every_processor_to_park`] overlap rather than
    /// serialise: a machine with four processors pays one bound, not four.
    fn ask_every_processor_to_park(&self) {
        for (index, (_, control)) in self.vcpus.iter().enumerate() {
            match control.request_park(Parked::Paused) {
                Ok(()) => {}
                Err(error) => tracing::error!("vCPU {index} would not take a cancel: {error}"),
            }
        }
    }

    /// Wait until every processor has parked — and so has read its registers
    /// out of the partition — within one shared [`PARK_BOUND`].
    ///
    /// # Errors
    /// [`FastMachineFault::Wedged`] for a processor that did not park.
    fn wait_for_every_processor_to_park(&self) -> Result<(), FastMachineFault> {
        let began = Instant::now();
        for (_, control) in &self.vcpus {
            let left = PARK_BOUND.saturating_sub(began.elapsed());
            if control.wait_parked_by(left).is_none() {
                return Err(FastMachineFault::Wedged {
                    waited: began.elapsed(),
                });
            }
        }
        Ok(())
    }

    /// Whether the machine is paused with every processor's registers in it —
    /// the condition [`Self::with_machine`] refuses without.
    ///
    /// A processor that parked without them says so itself, which is the one
    /// case a paused machine still cannot be read: the platform would not
    /// hand the registers over, so the machine's copy is not the guest's.
    ///
    /// Answers, per processor in processor order, the vector its park found
    /// placed and not yet taken — see [`RegistersAtPark::InTheMachine`].
    fn every_processor_is_in_the_machine(&self) -> Result<Vec<Option<u8>>, FastMachineFault> {
        match self.state {
            RunState::Paused => {}
            RunState::Running => return Err(FastMachineFault::NotPaused),
        }
        let mut placed = Vec::with_capacity(self.vcpus.len());
        for (processor, (_, control)) in self.vcpus.iter().enumerate() {
            match control.registers_at_park() {
                Some(RegistersAtPark::InTheMachine { placed: vector }) => placed.push(vector),
                Some(RegistersAtPark::WithThePartition) => {
                    return Err(FastMachineFault::RegistersNotInTheMachine { processor })
                }
                // Paused, and yet this processor has not parked: nothing
                // brought its registers home, whatever the state says.
                None => return Err(FastMachineFault::NotPaused),
            }
        }
        Ok(placed)
    }
}

/// The model-specific registers the machine's API can write that the exchange
/// does not carry into the partition: the platform answers both itself.
///
/// Audited against `BxCpuC::write_msr_for_api`, arm by arm. The TSC is the
/// third such register and is checked on its own, because the machine's API
/// names it as a register too. Every other MSR that API writes is carried:
/// the APIC base, the SYSENTER trio, STAR/LSTAR/CSTAR/FMASK, KERNEL_GS_BASE
/// and EFER travel in `ArchGroups::MSRS`, and the FS and GS bases in
/// `ArchGroups::SEGMENTS`. `IA32_PLATFORM_ID`, `IA32_APERF` and `IA32_MPERF`
/// refuse every write already.
pub(crate) const UNCARRIED_MSRS: [u32; 2] = [
    // IA32_TSC_AUX — what `RDTSCP` and `RDPID` return.
    0xC000_0103,
    // IA32_TSC_DEADLINE — the local APIC timer's deadline, and the APIC the
    // guest programs on this engine is the partition's.
    0x6E0,
];

/// One processor's state as it stood before a closure ran, to tell what the
/// closure changed and to put back what may not stay changed.
///
/// The whole [`VcpuArchState`], plus what that state does not carry as the
/// machine's API reads it: the time-stamp counter and [`UNCARRIED_MSRS`].
/// Taken and compared under the machine's lock, where no processor's thread
/// can move any of it.
struct ProcessorAsItStood {
    state: std::boxed::Box<VcpuArchState>,
    tsc: u64,
    /// [`UNCARRIED_MSRS`], in order. `None` for one this processor will not
    /// read — which it will not write through the API either.
    msrs: [Option<u64>; UNCARRIED_MSRS.len()],
}

impl ProcessorAsItStood {
    fn of<T: Instrumentation>(machine: &mut Emulator<T, WhpEngine>, processor: usize) -> Self {
        let cpu = machine.processor(processor).into_cpu();
        let mut state = std::boxed::Box::new(VcpuArchState::default());
        cpu.export_arch_state(&mut state);
        Self {
            state,
            tsc: cpu.time_stamp_counter(),
            msrs: UNCARRIED_MSRS.map(|msr| cpu.read_msr_for_api(msr).ok()),
        }
    }

    /// Put back every write to this processor that cannot reach the
    /// partition, and answer one refusal per write put back.
    ///
    /// The counter and [`UNCARRIED_MSRS`] are compared with how they stood.
    /// `IF` is compared with nothing: `placed` says an interrupt stands in the
    /// partition, which was placed only into a guest with `IF` set, so a
    /// clear `IF` now is the closure's write. Every other register the closure
    /// changed stays changed and reaches the partition at the next entry.
    fn put_back_what_the_partition_cannot_take<T: Instrumentation>(
        &self,
        machine: &mut Emulator<T, WhpEngine>,
        processor: usize,
        placed: Option<u8>,
    ) -> Vec<FastMachineFault> {
        let cpu = machine.processor(processor).into_cpu();
        let mut refusals = Vec::new();
        if cpu.time_stamp_counter() != self.tsc {
            cpu.take_time_stamp_counter(self.tsc);
            refusals.push(FastMachineFault::Uncarried { register: X86Reg::Tsc, processor });
        }
        for (msr, held) in UNCARRIED_MSRS.iter().zip(self.msrs) {
            let Some(held) = held else { continue };
            if cpu.read_msr_for_api(*msr).ok() == Some(held) {
                continue;
            }
            refusals.push(match cpu.write_msr_for_api(*msr, held) {
                Ok(()) => FastMachineFault::UncarriedMsr { msr: *msr, processor },
                Err(refused) => {
                    tracing::error!(
                        "processor {processor} would not take back MSR {msr:#x} as it stood: \
                         {refused}"
                    );
                    FastMachineFault::Engine(EngineFault::new(
                        rusty_box_core::EngineFaultKind::Host,
                        "restoring a model-specific register the partition cannot take",
                    ))
                }
            });
        }
        if let Some(vector) = placed {
            if !cpu.interrupts_enabled() {
                refusals.push(match set_if_again(cpu) {
                    Ok(()) => FastMachineFault::InterruptAlreadyPlaced { vector, processor },
                    Err(fault) => fault,
                });
            }
        }
        refusals
    }

    fn still_stands<T: Instrumentation>(
        &self,
        machine: &mut Emulator<T, WhpEngine>,
        processor: usize,
    ) -> bool {
        let cpu = machine.processor(processor).into_cpu();
        let mut now = VcpuArchState::default();
        cpu.export_arch_state(&mut now);
        now == *self.state
            && cpu.time_stamp_counter() == self.tsc
            && UNCARRIED_MSRS.map(|msr| cpu.read_msr_for_api(msr).ok()) == self.msrs
    }

    /// Put the whole processor back as it stood.
    ///
    /// The counter and [`UNCARRIED_MSRS`] are put back whatever else happens.
    ///
    /// # Errors
    /// A refusal means the processor was NOT fully restored. The architectural
    /// state is validated before any of it is written, so if the processor
    /// will not take back the state it produced itself a moment ago — a defect
    /// in the import — every register that state carries is left as the
    /// closure left it. An MSR the processor will not take back is likewise
    /// left as the closure wrote it. Each is logged.
    fn stand_again<T: Instrumentation>(
        &self,
        machine: &mut Emulator<T, WhpEngine>,
        processor: usize,
    ) -> Result<(), FastMachineFault> {
        let cpu = machine.processor(processor).into_cpu();
        cpu.take_time_stamp_counter(self.tsc);
        let mut outcome = Ok(());
        for (msr, held) in UNCARRIED_MSRS.iter().zip(self.msrs) {
            let Some(held) = held else { continue };
            if cpu.read_msr_for_api(*msr).ok() == Some(held) {
                continue;
            }
            if let Err(refused) = cpu.write_msr_for_api(*msr, held) {
                tracing::error!(
                    "processor {processor} would not take back MSR {msr:#x} as it stood: {refused}"
                );
                outcome = Err(not_fully_restored());
            }
        }
        if let Err(refused) = cpu.import_arch_state(&self.state) {
            tracing::error!(
                "processor {processor} would not take back the state it stood in: {refused}"
            );
            outcome = Err(not_fully_restored());
        }
        outcome
    }
}

/// The refusal a processor that could not be put back as it stood answers
/// with — see [`ProcessorAsItStood::stand_again`].
fn not_fully_restored() -> FastMachineFault {
    FastMachineFault::Engine(EngineFault::new(
        rusty_box_core::EngineFaultKind::Host,
        "restoring a processor written from the running path",
    ))
}

/// Set `IF` again on a processor whose flags a closure cleared it in, leaving
/// every other flag as the closure wrote it.
///
/// Through the import path, which re-evaluates interrupt masking exactly as a
/// flags write through the machine's API does.
///
/// # Errors
/// None the flags alone can raise; a refusal would be a defect in the import,
/// and is reported as the processor left as the closure left it.
fn set_if_again<T: Instrumentation>(
    cpu: &mut rusty_box::cpu::cpu::BxCpuC<T>,
) -> Result<(), FastMachineFault> {
    /// `RFLAGS.IF`.
    const IF: u64 = 1 << 9;
    let mut flags = VcpuArchState::default();
    cpu.export_arch_groups(&mut flags, ArchGroups::RIP_RFLAGS);
    flags.rflags |= IF;
    cpu.import_arch_groups(&flags, ArchGroups::RIP_RFLAGS).map_err(|refused| {
        tracing::error!("the processor would not take its flags back with IF set: {refused}");
        FastMachineFault::Engine(EngineFault::new(
            rusty_box_core::EngineFaultKind::Host,
            "setting IF again under a placed interrupt",
        ))
    })
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
    use crate::fixtures::{
        a_turn_on_the_hardware, fast_machine_running, hypervisor_here, machine_running_on,
        machine_with_devices_on, CODE, DEBUG_PORT, MARK,
    };
    use rusty_box::cpu::instrumentation::X86Reg;

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
        let ips = machine
            .with_machine(|m| m.config().ips.per_second_u64())
            .expect("an adopted machine is paused");
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
            machine.with_machine(|m| m.ticks()).expect("a stepped machine is paused") >= ten_ms,
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
        let ips = machine
            .with_machine(|m| m.config().ips.per_second_u64())
            .expect("an adopted machine is paused");

        // A host-time machine arms nothing for the 8042 until something is
        // owed — that is the divergence, and it is the premise of the rest.
        // A millisecond of guest time is nearly seven serial-delay periods, so
        // a machine that armed the Bochs continuous timer would tick here.
        machine
            .step(RunBudget::Ticks(ips / 1_000))
            .expect("a millisecond of guest time");
        assert_eq!(
            machine
                .with_machine(|m| m.keyboard_serial_ticks())
                .expect("a stepped machine is paused"),
            0,
            "with nothing latched, the 8042 is not ticking at all"
        );

        let took = machine
            .with_machine_while_running(|m| m.keyboard().tap(rusty_box::iodev::scancodes::BxKey::A))
            .expect("a keystroke touches no register");
        assert!(took, "the 8042 took the keystroke");
        machine
            .step(RunBudget::Ticks(ips / 1_000))
            .expect("a second millisecond of guest time");

        assert!(
            machine
                .with_machine(|m| m.keyboard_serial_ticks())
                .expect("a stepped machine is paused")
                > 0,
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

    /// Ten milliseconds of guest time at the machine's own rate.
    fn ten_milliseconds(machine: &mut FastMachine) -> RunBudget {
        let ips = machine
            .with_machine(|m| m.config().ips.per_second_u64())
            .expect("the machine is paused");
        RunBudget::Ticks(ips / 100)
    }

    /// Everything the guest has written to the debug port since the last look.
    fn debug_output(machine: &mut FastMachine) -> std::vec::Vec<u8> {
        machine
            .with_machine(|m| m.debug_port().take_output().collect())
            .expect("the machine is paused")
    }

    /// A machine whose guest is running is not the machine's to read.
    ///
    /// Its processor's registers are the partition's while it runs, so the
    /// closure would read a copy the guest is not using. The refusal has to
    /// hold in the release build this project ships, not only under a
    /// `debug_assert` — and it has to come BEFORE the closure runs.
    #[test]
    fn reaching_into_a_running_machine_is_refused() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine = fast_machine_running(&SPIN);
        machine.resume().expect("the guest runs");

        let mut ran = false;
        match machine.with_machine(|_| ran = true) {
            Err(FastMachineFault::NotPaused) => {}
            Err(other) => panic!("refused for the wrong reason: {other}"),
            Ok(()) => panic!("the closure was let into a machine whose guest was running"),
        }
        assert!(!ran, "the closure ran against a machine whose guest was running");

        machine.pause().expect("the guest pauses");
        assert!(
            machine.with_machine(|_| ()).is_ok(),
            "and the same machine, paused, is let in"
        );
    }

    /// Adoption hands back a machine that is really paused: its guest runs
    /// nothing until a step or a resume says so.
    #[test]
    fn an_adopted_machine_runs_nothing_until_it_is_stepped() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        // mov al, MARK ; out 0xE9, al ; jmp $
        let mut machine = fast_machine_running(&[0xB0, MARK, 0xE6, DEBUG_PORT, 0xEB, 0xFE]);
        std::thread::sleep(std::time::Duration::from_millis(100));

        assert_eq!(
            debug_output(&mut machine),
            std::vec::Vec::<u8>::new(),
            "the guest wrote to the debug port before anything stepped it"
        );
        let budget = ten_milliseconds(&mut machine);
        machine.step(budget).expect("a step");
        assert_eq!(debug_output(&mut machine), std::vec![MARK], "and does once stepped");
    }

    /// The processor a machine was built with is the one its guest starts on.
    ///
    /// The machine points its processor at [`CODE`] before adoption. The
    /// partition's own reset state is `F000:FFF0`, and the vector the reset
    /// path would reach is aimed at a spin of its own here — so the guest
    /// reaches its port write only if adoption installed the machine's
    /// processor rather than the platform's.
    #[test]
    fn the_processor_a_machine_was_built_with_is_the_one_its_guest_starts_on() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut built =
            machine_running_on(DeviceClock::HostTime, &[0xB0, MARK, 0xE6, DEBUG_PORT, 0xEB, 0xFE]);
        // #UD's real-mode vector (IVT entry 6) at 0000:2000, which spins. The
        // ROM's 0xFF bytes raise #UD at the platform's reset vector.
        built.mem_write(6 * 4, &[0x00, 0x20, 0x00, 0x00]).expect("the IVT is writable");
        built.mem_write(0x2000, &SPIN).expect("the spin is writable");
        let mut machine = FastMachine::adopt(built).expect("a machine on hardware");

        let budget = ten_milliseconds(&mut machine);
        machine.step(budget).expect("a step");
        assert_eq!(
            debug_output(&mut machine),
            std::vec![MARK],
            "the guest must start where the machine's processor stood at adoption"
        );
    }

    /// A register read while the machine is paused is the one the guest left.
    ///
    /// The guest changes a register of every class the exchange carries and
    /// then spins. None of it takes an exit, so the machine is told of none of
    /// it until it pauses — which is the moment the reads below have to be
    /// current by.
    ///
    /// ```text
    /// 66 BB DF 9B 57 13   mov ebx, 0x13579BDF
    /// B8 00 02 / 8E E0    mov ax, 0x0200 ; mov fs, ax     ; base 0x2000
    /// 66 B8 C2 00 00 00   mov eax, 0xC2
    /// 0F 22 D0            mov cr2, eax
    /// 66 B8 D0 00 00 00   mov eax, 0xD0
    /// 0F 23 C0            mov dr0, eax
    /// 0F 20 E0            mov eax, cr4
    /// 66 0D 00 02 00 00   or eax, 0x200                  ; CR4.OSFXSR
    /// 0F 22 E0            mov cr4, eax
    /// 66 0F 6E C3         movd xmm0, ebx
    /// F9                  stc
    /// EB FE               jmp $                          ; at CODE + 0x2E
    /// ```
    #[test]
    fn a_register_read_while_paused_is_the_one_the_guest_left() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine = fast_machine_running(&[
            0x66, 0xBB, 0xDF, 0x9B, 0x57, 0x13, //
            0xB8, 0x00, 0x02, 0x8E, 0xE0, //
            0x66, 0xB8, 0xC2, 0x00, 0x00, 0x00, //
            0x0F, 0x22, 0xD0, //
            0x66, 0xB8, 0xD0, 0x00, 0x00, 0x00, //
            0x0F, 0x23, 0xC0, //
            0x0F, 0x20, 0xE0, //
            0x66, 0x0D, 0x00, 0x02, 0x00, 0x00, //
            0x0F, 0x22, 0xE0, //
            0x66, 0x0F, 0x6E, 0xC3, //
            0xF9, //
            0xEB, 0xFE, //
        ]);
        let budget = ten_milliseconds(&mut machine);
        machine.step(budget).expect("a step");

        machine.with_machine(|m| {
            assert_eq!(m.reg_read(X86Reg::Ebx), 0x1357_9BDF, "a general register");
            assert_eq!(m.reg_read(X86Reg::Fs), 0x0200, "a segment selector");
            assert_eq!(m.reg_read(X86Reg::FsBase), 0x2000, "and its base");
            assert_eq!(m.reg_read(X86Reg::Cr2), 0xC2, "a control register");
            assert_eq!(m.reg_read(X86Reg::Cr4) & 0x200, 0x200, "CR4.OSFXSR");
            assert_eq!(m.reg_read(X86Reg::Dr0), 0xD0, "a debug register");
            assert_eq!(
                m.reg_read_xmm(X86Reg::Xmm0)[..4],
                [0xDF, 0x9B, 0x57, 0x13],
                "the vector file"
            );
            assert_eq!(m.reg_read(X86Reg::Rflags) & 1, 1, "CF, from the flags");
            assert_eq!(m.reg_read(X86Reg::Rip), CODE + 0x2E, "the spin the guest stands on");
        })
        .expect("a stepped machine is paused");
    }

    /// The time-stamp counter read while paused is the guest's own.
    ///
    /// The guest stores what `RDTSC` gave it and spins; the counter at the
    /// pause is at least that and not wildly past it. The machine's own model
    /// of the counter, derived from the instructions its interpreter retired,
    /// is neither.
    ///
    /// ```text
    /// 0F 31            rdtsc
    /// 66 A3 00 05      mov [0x500], eax
    /// 66 89 16 04 05   mov [0x504], edx
    /// EB FE            jmp $
    /// ```
    #[test]
    fn the_time_stamp_counter_read_while_paused_is_the_guests_own() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine = fast_machine_running(&[
            0x0F, 0x31, //
            0x66, 0xA3, 0x00, 0x05, //
            0x66, 0x89, 0x16, 0x04, 0x05, //
            0xEB, 0xFE, //
        ]);
        let budget = ten_milliseconds(&mut machine);
        machine.step(budget).expect("a step");

        let (stored, read) = machine
            .with_machine(|m| {
                let mut bytes = [0u8; 8];
                m.mem_read(0x500, &mut bytes).expect("the guest's store is readable");
                (u64::from_le_bytes(bytes), m.reg_read(X86Reg::Tsc))
            })
            .expect("a stepped machine is paused");
        assert_ne!(stored, 0, "the guest ran its RDTSC");
        assert!(
            read >= stored,
            "the counter at the pause ({read}) is behind what the guest already read ({stored})"
        );
        assert!(
            read - stored < 10_000_000_000,
            "the counter at the pause ({read}) is seconds past the guest's ({stored})"
        );
    }

    /// A write to the time-stamp counter is refused and undone, and the rest
    /// of the closure stands.
    ///
    /// The hardware owns the counter on this engine, so a write kept in the
    /// machine's copy would read back as written while the guest went on
    /// reading its own. The guest waits for `BL`, then stores what `RDTSC`
    /// gives it:
    ///
    /// ```text
    /// 80 FB 00 / 74 FB    wait: cmp bl, 0 ; je wait
    /// 0F 31               rdtsc
    /// 66 A3 00 05         mov [0x500], eax
    /// 66 89 16 04 05      mov [0x504], edx
    /// EB FE               jmp $
    /// ```
    #[test]
    fn a_write_to_the_time_stamp_counter_is_refused_and_undone() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine = fast_machine_running(&[
            0x80, 0xFB, 0x00, 0x74, 0xFB, //
            0x0F, 0x31, //
            0x66, 0xA3, 0x00, 0x05, //
            0x66, 0x89, 0x16, 0x04, 0x05, //
            0xEB, 0xFE, //
        ]);
        let budget = ten_milliseconds(&mut machine);
        machine.step(budget).expect("a step");
        let guests = machine
            .with_machine(|m| m.reg_read(X86Reg::Tsc))
            .expect("a stepped machine is paused");

        const WRITTEN: u64 = 1 << 50;
        match machine.with_machine(|m| {
            m.reg_write(X86Reg::Rbx, 1);
            m.reg_write(X86Reg::Tsc, WRITTEN);
        }) {
            Err(FastMachineFault::Uncarried { register: X86Reg::Tsc, processor: 0 }) => {}
            Err(other) => panic!("refused for the wrong reason: {other}"),
            Ok(()) => panic!("a write to the counter the hardware owns was kept"),
        }
        assert_eq!(
            machine
                .with_machine(|m| m.reg_read(X86Reg::Tsc))
                .expect("still paused"),
            guests,
            "the counter reads the guest's value again, not the one written"
        );

        machine.step(budget).expect("a second step");
        let stored = machine
            .with_machine(|m| {
                let mut bytes = [0u8; 8];
                m.mem_read(0x500, &mut bytes).expect("the guest's store is readable");
                u64::from_le_bytes(bytes)
            })
            .expect("a stepped machine is paused");
        assert_ne!(stored, 0, "the RBX write beside the refused one stood and reached the guest");
        assert!(
            stored < WRITTEN,
            "the guest read its own counter ({stored}), not the refused one"
        );
    }

    /// Clearing IF while a vector is already placed for delivery is refused,
    /// and the vector still arrives.
    ///
    /// A vector this engine placed was acknowledged at the 8259 on positive
    /// evidence that the guest could take it; the platform refuses the entry
    /// that would carry it into a guest that cannot (`WHV_E_INVALID_VP_STATE`).
    ///
    /// The placed-but-untaken state is reached deterministically. A keystroke
    /// raises IRQ1 while the machine is paused, so the vector is owed before
    /// the processor's thread runs. The thread is then let go while the test
    /// holds the machine, so it waits on the machine's lock at its hand-back.
    /// The pause asked for meanwhile is a cancel issued between runs, which
    /// the platform latches for the next entry — the one right after the
    /// placement — and that entry comes back having run nothing.
    ///
    /// ```text
    /// 31 C0                xor ax, ax
    /// 8E D8 / 8E D0        mov ds, ax ; mov ss, ax
    /// BC 00 70             mov sp, 0x7000
    /// C7 06 24 00 1D 10    mov word [0x24], isr   ; IVT[9]: IRQ1 at the 8259's power-on base
    /// C7 06 26 00 00 00    mov word [0x26], 0
    /// B0 FD / E6 21        mov al, 0xFD ; out 0x21, al   ; unmask IRQ1 alone
    /// FB                   sti
    /// 40 / EB FD           busy: inc ax ; jmp busy       ; exit-free
    /// isr (CODE + 0x1D):
    /// E4 60                in al, 0x60                   ; take the scancode
    /// B0 5A / E6 E9        mov al, MARK ; out 0xE9, al
    /// B0 20 / E6 20        mov al, 0x20 ; out 0x20, al   ; EOI
    /// CF                   iret
    /// ```
    #[test]
    fn clearing_if_under_a_placed_interrupt_is_refused_and_the_vector_still_arrives() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine = FastMachine::adopt(machine_with_devices_on(
            DeviceClock::HostTime,
            &[
                0x31, 0xC0, //
                0x8E, 0xD8, 0x8E, 0xD0, //
                0xBC, 0x00, 0x70, //
                0xC7, 0x06, 0x24, 0x00, 0x1D, 0x10, //
                0xC7, 0x06, 0x26, 0x00, 0x00, 0x00, //
                0xB0, 0xFD, 0xE6, 0x21, //
                0xFB, //
                0x40, 0xEB, 0xFD, //
                0xE4, 0x60, //
                0xB0, MARK, 0xE6, DEBUG_PORT, //
                0xB0, 0x20, 0xE6, 0x20, //
                0xCF, //
            ],
        ))
        .expect("a machine on hardware");
        let budget = ten_milliseconds(&mut machine);
        machine.step(budget).expect("a step");
        let ips = machine
            .with_machine(|m| m.config().ips.per_second_u64())
            .expect("a stepped machine is paused");

        let raised = machine
            .with_machine(|m| {
                assert!(
                    m.keyboard().tap(rusty_box::iodev::scancodes::BxKey::A),
                    "the 8042 took the keystroke"
                );
                for _ in 0..200 {
                    m.service_device_time(ips / 10_000).expect("the wheel turns");
                    if m.processor(0).io().device_manager().has_interrupt() {
                        return true;
                    }
                }
                false
            })
            .expect("a stepped machine is paused");
        assert!(raised, "the keystroke raised the 8259's pin");

        let control = machine.vcpus[0].1.clone();
        {
            let held = machine.shared.lock().unwrap_or_else(PoisonError::into_inner);
            control.resume();
            std::thread::sleep(Duration::from_millis(200));
            control.request_park(Parked::Paused).expect("the park request reaches the platform");
            drop(held);
        }
        assert_eq!(
            control.wait_parked_by(Duration::from_secs(5)),
            Some(Parked::Paused),
            "the processor parked on the latched cancel"
        );
        assert_eq!(
            machine.engine_census().injections.injected,
            1,
            "the vector was placed for the partition"
        );
        assert_eq!(
            debug_output(&mut machine),
            std::vec::Vec::<u8>::new(),
            "and the guest has not taken it: the entry after the placement ran nothing"
        );

        let cleared = machine.with_machine(|m| {
            let flags = m.reg_read(X86Reg::Rflags);
            // CF beside IF, so the refusal is seen to undo IF alone.
            m.reg_write(X86Reg::Rflags, (flags & !0x200) | 1);
        });
        let outcome = machine.step(budget).expect("a step");
        assert_eq!(
            outcome.stop,
            StepStop::BudgetSpent,
            "the entry after the IF write carried the placed vector into a guest that could \
             not take it: {outcome:?}"
        );
        match cleared {
            Err(FastMachineFault::InterruptAlreadyPlaced { vector: 9, processor: 0 }) => {}
            Err(other) => panic!("refused for the wrong reason: {other}"),
            Ok(()) => panic!("clearing IF under a placed vector looked like it succeeded"),
        }
        assert!(
            debug_output(&mut machine).contains(&MARK),
            "the placed vector reached the guest's handler"
        );
        assert_eq!(
            machine
                .with_machine(|m| m.reg_read(X86Reg::Rflags) & 1)
                .expect("a stepped machine is paused"),
            1,
            "the CF written beside the refused IF stood"
        );
    }

    /// A write to `IA32_TSC_AUX` is refused and undone, and the rest of the
    /// closure stands.
    ///
    /// The platform answers the register itself and the exchange does not
    /// carry it, so a write kept in the machine's copy would read back as
    /// written while the guest's `RDTSCP` went on answering its own. The guest
    /// waits for `BL`, then reports `RDTSCP`'s `ECX`:
    ///
    /// ```text
    /// 80 FB 00 / 74 FB    wait: cmp bl, 0 ; je wait
    /// 0F 01 F9            rdtscp
    /// 88 C8 / E6 E9       mov al, cl ; out 0xE9, al
    /// EB FE               jmp $
    /// ```
    #[test]
    fn a_write_to_tsc_aux_is_refused_and_undone() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine = fast_machine_running(&[
            0x80, 0xFB, 0x00, 0x74, 0xFB, //
            0x0F, 0x01, 0xF9, //
            0x88, 0xC8, 0xE6, DEBUG_PORT, //
            0xEB, 0xFE, //
        ]);
        let budget = ten_milliseconds(&mut machine);
        machine.step(budget).expect("a step");
        const TSC_AUX: u32 = 0xC000_0103;
        let held = machine
            .with_machine(|m| m.msr_read(TSC_AUX).expect("TSC_AUX is readable"))
            .expect("a stepped machine is paused");

        match machine.with_machine(|m| {
            m.reg_write(X86Reg::Rbx, 1);
            m.msr_write(TSC_AUX, 0x77).expect("the machine takes the write");
        }) {
            Err(FastMachineFault::UncarriedMsr { msr: TSC_AUX, processor: 0 }) => {}
            Err(other) => panic!("refused for the wrong reason: {other}"),
            Ok(()) => panic!("a TSC_AUX write the guest never sees looked like it succeeded"),
        }
        assert_eq!(
            machine
                .with_machine(|m| m.msr_read(TSC_AUX).expect("TSC_AUX is readable"))
                .expect("still paused"),
            held,
            "the refused write was undone"
        );

        machine.step(budget).expect("a second step");
        assert_eq!(
            debug_output(&mut machine),
            std::vec![0],
            "the RBX write beside the refused one reached the guest, and its RDTSCP read the \
             partition's TSC_AUX — zero since reset — not the refused 0x77"
        );
    }

    /// A register written from the running path is refused and undone.
    ///
    /// A running processor's registers are the partition's: the write would
    /// be overwritten at the next park, and meanwhile an exit's errand would
    /// run against it. The guest waits for `BL` exactly as in
    /// `registers_written_while_paused_reach_the_guest`, and must still be
    /// waiting afterwards.
    ///
    /// The undo is read back on the running path itself, before any park: a
    /// park re-reads the processor from the partition, and would hide a write
    /// the undo had missed.
    #[test]
    fn a_register_written_while_the_guest_runs_is_refused_and_undone() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine = fast_machine_running(&[
            0xB0, 0x11, 0xE6, DEBUG_PORT, //
            0x80, 0xFB, 0x00, 0x74, 0xFB, //
            0x88, 0xD8, 0xE6, DEBUG_PORT, //
            0xEB, 0xFE, //
        ]);
        let budget = ten_milliseconds(&mut machine);
        machine.step(budget).expect("a step");
        assert_eq!(debug_output(&mut machine), std::vec![0x11], "the guest is waiting");

        machine.resume().expect("the guest runs");
        match machine.with_machine_while_running(|m| m.reg_write(X86Reg::Rbx, 0x5A)) {
            Err(FastMachineFault::WrittenWhileRunning { processor: 0 }) => {}
            Err(other) => panic!("refused for the wrong reason: {other}"),
            Ok(()) => panic!("a register write on the running path looked like it succeeded"),
        }
        assert_eq!(
            machine
                .with_machine_while_running(|m| m.reg_read(X86Reg::Rbx))
                .expect("a read changes nothing"),
            0,
            "the refused write was undone in the machine's copy, before any park re-read it"
        );
        machine.pause().expect("the guest pauses");
        assert_eq!(
            machine
                .with_machine(|m| m.reg_read(X86Reg::Rbx))
                .expect("a paused machine"),
            0,
            "RBX reads as the guest holds it"
        );
        machine.step(budget).expect("a second step");
        assert_eq!(
            debug_output(&mut machine),
            std::vec::Vec::<u8>::new(),
            "the guest never saw the refused write: it is still waiting"
        );
    }

    /// A pause nobody writes through costs the partition nothing, and a write
    /// costs exactly the group it touched.
    ///
    /// Asserted on the processor's own census, because the property is what
    /// the platform is told, which the guest cannot see.
    #[test]
    fn a_pause_nobody_writes_through_costs_the_partition_nothing() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine = fast_machine_running(&SPIN);
        let budget = ten_milliseconds(&mut machine);
        machine.step(budget).expect("a step");
        let installed = machine.engine_census().vcpus[0].handback_calls;
        assert!(installed >= 1, "adoption installed the machine's processor");

        machine
            .with_machine(|m| m.reg_read(X86Reg::Rip))
            .expect("a stepped machine is paused");
        machine.step(budget).expect("a second step");
        machine.step(budget).expect("a third step");
        assert_eq!(
            machine.engine_census().vcpus[0].handback_calls,
            installed,
            "two pauses and a read wrote nothing back"
        );

        machine
            .with_machine(|m| m.reg_write(X86Reg::Rbx, 1))
            .expect("a stepped machine is paused");
        machine.step(budget).expect("a fourth step");
        assert_eq!(
            machine.engine_census().vcpus[0].handback_calls,
            installed + 1,
            "one general register written is one batched register write"
        );
    }

    /// Registers written while paused reach the guest: a general register, a
    /// control register, a segment base and a debug register.
    ///
    /// The guest announces itself, then waits for `BL` to become nonzero —
    /// which only the host can make happen — and reports each of the others
    /// on the debug port.
    ///
    /// ```text
    /// B0 11 / E6 E9       mov al, 0x11 ; out 0xE9, al      ; running
    /// 80 FB 00 / 74 FB    wait: cmp bl, 0 ; je wait
    /// 88 D8 / E6 E9       mov al, bl ; out 0xE9, al
    /// 0F 20 D0 / E6 E9    mov eax, cr2 ; out 0xE9, al
    /// 64 A0 00 00 / E6 E9 mov al, fs:[0] ; out 0xE9, al
    /// 0F 21 C0 / E6 E9    mov eax, dr0 ; out 0xE9, al
    /// EB FE               jmp $
    /// ```
    #[test]
    fn registers_written_while_paused_reach_the_guest() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine = fast_machine_running(&[
            0xB0, 0x11, 0xE6, DEBUG_PORT, //
            0x80, 0xFB, 0x00, 0x74, 0xFB, //
            0x88, 0xD8, 0xE6, DEBUG_PORT, //
            0x0F, 0x20, 0xD0, 0xE6, DEBUG_PORT, //
            0x64, 0xA0, 0x00, 0x00, 0xE6, DEBUG_PORT, //
            0x0F, 0x21, 0xC0, 0xE6, DEBUG_PORT, //
            0xEB, 0xFE, //
        ]);
        let budget = ten_milliseconds(&mut machine);
        machine.step(budget).expect("a step");
        assert_eq!(debug_output(&mut machine), std::vec![0x11], "the guest is waiting");

        machine.with_machine(|m| {
            m.mem_write(0x3000, &[0xF5]).expect("the byte FS will point at");
            m.reg_write(X86Reg::Rbx, 0x5A);
            m.reg_write(X86Reg::Cr2, 0xC2);
            m.reg_write(X86Reg::FsBase, 0x3000);
            m.reg_write(X86Reg::Dr0, 0xD0);
        })
        .expect("a stepped machine is paused");
        machine.step(budget).expect("a second step");
        assert_eq!(
            debug_output(&mut machine),
            std::vec![0x5A, 0xC2, 0xF5, 0xD0],
            "the guest saw BL, CR2, FS's base and DR0 as the host wrote them"
        );
    }

    /// The instruction pointer, the flags, the vector file and a
    /// model-specific register written while paused reach the guest.
    ///
    /// The guest spins at `CODE + 4`; the host moves it to `CODE + 0x100`,
    /// where it reports the flags, `XMM0` and `IA32_KERNEL_GS_BASE`. `XMM0` is
    /// readable only under `CR4.OSFXSR`, which the host sets too.
    ///
    /// ```text
    /// B0 11 / E6 E9 / EB FE   mov al, 0x11 ; out 0xE9, al ; jmp $
    /// ...
    /// +0x100: B0 77 / E6 E9   mov al, 0x77 ; out 0xE9, al     ; moved
    ///         9C / 58 / E6 E9 pushf ; pop ax ; out 0xE9, al
    ///         66 0F 7E C0     movd eax, xmm0
    ///         E6 E9           out 0xE9, al
    ///         66 B9 02 01 00 C0  mov ecx, 0xC0000102
    ///         0F 32 / E6 E9   rdmsr ; out 0xE9, al
    ///         EB FE           jmp $
    /// ```
    #[test]
    fn the_instruction_pointer_flags_vector_file_and_msrs_written_while_paused_reach_the_guest() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut code = std::vec![0xB0, 0x11, 0xE6, DEBUG_PORT, 0xEB, 0xFE];
        code.resize(0x100, 0x90);
        code.extend_from_slice(&[
            0xB0, 0x77, 0xE6, DEBUG_PORT, //
            0x9C, 0x58, 0xE6, DEBUG_PORT, //
            0x66, 0x0F, 0x7E, 0xC0, 0xE6, DEBUG_PORT, //
            0x66, 0xB9, 0x02, 0x01, 0x00, 0xC0, //
            0x0F, 0x32, 0xE6, DEBUG_PORT, //
            0xEB, 0xFE, //
        ]);
        let mut machine = fast_machine_running(&code);
        let budget = ten_milliseconds(&mut machine);
        machine.step(budget).expect("a step");
        assert_eq!(debug_output(&mut machine), std::vec![0x11], "the guest is spinning");

        machine.with_machine(|m| {
            m.reg_write(X86Reg::Rip, CODE + 0x100);
            // Bit 1 is the reserved one; CF, ZF and SF beside it.
            m.reg_write(X86Reg::Rflags, 0xC3);
            m.reg_write(X86Reg::Rsp, 0x7000);
            let cr4 = m.reg_read(X86Reg::Cr4);
            m.reg_write(X86Reg::Cr4, cr4 | 0x200);
            let mut xmm0 = [0u8; 16];
            xmm0[0] = 0x99;
            m.reg_write_xmm(X86Reg::Xmm0, xmm0);
            m.msr_write(0xC000_0102, 0x6B).expect("IA32_KERNEL_GS_BASE is writable");
        })
        .expect("a stepped machine is paused");
        machine.step(budget).expect("a second step");
        assert_eq!(
            debug_output(&mut machine),
            std::vec![0x77, 0xC3, 0x99, 0x6B],
            "the guest ran from the written RIP, with the written flags, XMM0 and \
             IA32_KERNEL_GS_BASE"
        );
    }
}
