//! One processor, the thread that runs it, and the exits that thread services.
//!
//! ## What changes here
//!
//! The slice loop beside this one enters the partition, runs the guest for a
//! measured stretch, and comes back so the machine can service its devices.
//! Measured, that costs 2.19 million entries in 300 seconds, 85.4% of which
//! produce no exit at all, and leaves under 1% of the wall clock actually
//! executing guest instructions. The entry is the cost, and the only way to
//! stop paying it is to stop leaving.
//!
//! So a thread here is BOUND to one processor for its life. It comes back when
//! the hardware has something to say, takes the machine's lock long enough to
//! answer, and re-enters immediately — with no state exchange and no budget in
//! between, which is the whole difference from a slice. What it does not do is
//! return to a scheduler that owns the machine; the machine is owned by nobody,
//! and its device timers, its interrupt fabric and the front end reading its
//! display all run on other threads against the same lock. That is what makes
//! the guest's time the guest's rather than a share of one thread's.
//!
//! Not one entry that never returns: `WHvRunVirtualProcessor` returns on every
//! exit and the loop below re-enters. QEMU's WHPX accelerator has the same
//! shape — `whpx_vcpu_run` is a loop around the run call, and `bql_unlock()`
//! before it is what lets QEMU's main loop advance device time meanwhile.
//!
//! ## The three rules the design rests on
//!
//! **The lock is released across the run.** `Vcpu::run` blocks for as long as
//! the guest keeps running, which is the whole point; holding the machine's
//! lock across it would serialise every device in the machine behind a guest
//! that never exits. The lock is taken to service an exit and dropped before
//! the next entry, and `in_run_nanos` measures the span between them.
//!
//! **A park request sets its flag before it cancels.** A cancel is a message to
//! a run that may not have started yet, and the platform makes it sticky for
//! exactly one entry; the flag is what makes it sticky for as many as it takes.
//! [`park_request`] is that ordering, written as a free function over
//! [`CancelRun`] so it can be tested without a partition — which is the only
//! way to test an ordering whose failure mode is a race. QEMU's
//! `cpus-common.c` keeps the same `exit_request` before its own kick.
//!
//! **A cancel goes only to a processor inside its run.** The platform latches
//! a cancel issued between runs and spends it on the next entry, which then
//! retires nothing; a guest that exits often would never execute its way to
//! the `STI` it needs. So the thread publishes whether it is inside
//! `WHvRunVirtualProcessor`, the legacy-interrupt raiser asks before it
//! cancels, and the entry re-reads the request under that flag —
//! [`VcpuControl::in_run`] is the ordering, [`ext_int_request`] the asking.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusty_box::cpu::{cpu::BxCpuC, instrumentation::Instrumentation, CpuError, Result};
use rusty_box::emulator::{Emulator, PcIo, Processor, StopReason};
use rusty_box_core::{EngineFault, EngineFaultKind};
use rusty_box_whp::{
    ApicWriteType, Canceller, Exit, ExitReason, InternalActivity, IoPortAccess,
    PendingExtIntEvent, Reg, Vcpu, WhpError, WhpResult,
};

use crate::engine::{
    describe_the_fault, history_mark, platform_failed, ports_on_the_shadow, reports_each_fault,
    report_the_state_the_platform_refused, run_the_shadow_out_of_smm, withhold_virtualisation_from,
    ExitCounts, InjectState, PlatformCounters, Trapped, WhpEngine, RFLAGS_TF,
};
use crate::exchange::{Exchange, ExitClass};
use crate::state::VpRegisters;
use crate::vm_clock::{StdClock, VmClockSource};
use crate::xsave::{self, XsaveArea};

/// The processor the machine boots on, and the only one the 8259's INTR is
/// wired to (`BxLocalApic::preset_lint0`, divergence D6).
const BOOT_PROCESSOR: usize = 0;

/// Why a vCPU thread left its run loop.
///
/// Named rather than a flag and a reason beside it (R0/R2): a parked thread is
/// one state with one cause, and the cause is what every waiter acts on — a
/// pause resumes, a power-off tears the machine down, a fault is reported.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Parked {
    /// The host asked for the processor back and means to give it back.
    Paused,
    /// The guest turned the machine off.
    ///
    /// It arrives as a stop on the machine's own device time, and no exit the
    /// processor takes says anything about it — so whoever turns the wheel is
    /// who sees it, whether that is the device thread or this thread routing an
    /// EOI through the machine's own boundary.
    GuestPowerOff,
    /// Something the thread could not service or could not survive.
    Fault(EngineFault),
    /// The thread has left its run loop for good and will not answer a resume.
    ///
    /// Distinct from [`Self::Paused`] because a caller waiting on a park has to
    /// tell a processor it can have back from one that is gone (R2), and
    /// `stop` reaches the thread by the same slot a pause does.
    Stopped,
}

impl Parked {
    /// Why a processor should come back, given what stopped the machine.
    ///
    /// The one place a machine's stop becomes a processor's park (R5), and
    /// exhaustive over the whole vocabulary rather than over the three
    /// [`StopCause`](rusty_box::emulator::StopReason) values a device boundary
    /// can produce today — a reason the machine grows breaks this instead of
    /// falling into a default, and the default that would be inherited is
    /// "keep running", which is the wrong answer for every one of them.
    pub(crate) fn from_stop(reason: StopReason) -> Self {
        match reason {
            // Terminal, and the only stop a caller acts on differently: the
            // machine is off and will not come back.
            StopReason::GuestPowerOff => Self::GuestPowerOff,
            // The host asked and means to give the machine back.
            StopReason::StopRequested => Self::Paused,
            // A guest that triple-faulted, an engine that refused work the
            // machine cannot do without, or a machine reporting a batch
            // vocabulary no fast machine runs — `Halted` and `BudgetExhausted`
            // describe a stepping loop this machine does not have, so a
            // boundary that reports one is describing a machine in a state its
            // driver cannot explain.
            StopReason::CpuShutdown
            | StopReason::EngineFault
            | StopReason::Halted
            | StopReason::BudgetExhausted => Self::Fault(EngineFault::new(
                EngineFaultKind::Host,
                "the machine stopped its own devices",
            )),
        }
    }
}

/// One thread's account of itself, readable from any thread.
///
/// Plain copies of the shared counters, plus the platform's own numbers as of
/// the thread's last park. Those come from the thread itself — the only holder
/// of the [`Vcpu`] — so reading this census never makes a platform call against
/// a processor another thread is running.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct VcpuCensus {
    /// Entries into the partition, counted as they are made — so a thread
    /// currently inside a run has already counted it.
    pub runs: u64,
    /// Host time spent inside `WHvRunVirtualProcessor`, and nowhere else.
    ///
    /// The number this whole design is judged by: against the wall clock of a
    /// stretch, it is the fraction of the host's time the guest actually had.
    pub in_run_nanos: u64,
    /// What the guest has been leaving the hardware for, by class.
    pub exits: ExitCounts,
    /// Platform register writes the exchange made putting serviced exits back.
    ///
    /// What the state exchange cost, which is the half of an exit's price this
    /// engine controls: an exit that imported nothing exports nothing, and a
    /// class whose count here climbs faster than its exit count is a class
    /// asking for more state than it reads.
    pub export_calls: u64,
    /// The hypervisor's own accounting as of the last park, or `None` if the
    /// thread has not parked yet.
    ///
    /// The independent check on the fields above: they are this port's account
    /// of its own behaviour, and an account cannot audit itself.
    pub platform_at_last_park: Option<PlatformCounters>,
}

/// The counters the thread writes and everyone else reads.
///
/// Atomics rather than a lock because the reader is a front end that may ask at
/// any moment and the writer is on the exit path: a lock here would put the
/// guest's own progress behind whoever last asked how it was doing. Each field
/// is written by one thread and read by any, so a relaxed increment paired with
/// an acquire load is the whole synchronisation — no two fields are read as a
/// pair whose consistency matters.
#[derive(Default)]
struct SharedCensus {
    runs: AtomicU64,
    in_run_nanos: AtomicU64,
    export_calls: AtomicU64,
    exits: SharedExits,
    /// Written only at a park, which is rare, and read only by a census. A lock
    /// is the honest shape for a value this size; the exit path never touches
    /// it.
    platform_at_last_park: Mutex<Option<PlatformCounters>>,
}

/// [`ExitCounts`], one atomic per bucket.
#[derive(Default)]
struct SharedExits {
    port: AtomicU64,
    memory: AtomicU64,
    cpuid: AtomicU64,
    msr: AtomicU64,
    exception: AtomicU64,
    halt: AtomicU64,
    canceled: AtomicU64,
    window: AtomicU64,
    apic_eoi: AtomicU64,
    apic_write: AtomicU64,
    other: AtomicU64,
}

impl SharedExits {
    /// Count one exit, by the class its reason belongs to.
    ///
    /// The one place a reason becomes a bucket (R5), and exhaustive on
    /// [`ExitReason`] — a reason the platform grows must be given a bucket here
    /// as well as an arm below, or the total stops adding up and no assertion
    /// about "no exit of class X" means anything.
    fn record(&self, reason: ExitReason) {
        let bucket = match reason {
            ExitReason::IoPortAccess(_) => &self.port,
            ExitReason::MemoryAccess(_) => &self.memory,
            ExitReason::Cpuid(_) => &self.cpuid,
            ExitReason::MsrAccess(_) => &self.msr,
            ExitReason::Exception => &self.exception,
            ExitReason::Halt => &self.halt,
            ExitReason::Canceled { .. } => &self.canceled,
            ExitReason::InterruptWindow => &self.window,
            ExitReason::ApicEoi { .. } => &self.apic_eoi,
            ExitReason::ApicWriteTrap { .. } => &self.apic_write,
            ExitReason::None
            | ExitReason::UnrecoverableException
            | ExitReason::InvalidVpRegisterValue
            | ExitReason::UnsupportedFeature { .. }
            | ExitReason::SynicSintDeliverable
            | ExitReason::Rdtsc
            | ExitReason::ApicSmiTrap
            | ExitReason::Hypercall
            | ExitReason::ApicInitSipiTrap
            | ExitReason::Unrecognized(_) => &self.other,
        };
        bucket.fetch_add(1, Ordering::Release);
    }

    fn snapshot(&self) -> ExitCounts {
        ExitCounts {
            port: self.port.load(Ordering::Acquire),
            memory: self.memory.load(Ordering::Acquire),
            cpuid: self.cpuid.load(Ordering::Acquire),
            msr: self.msr.load(Ordering::Acquire),
            exception: self.exception.load(Ordering::Acquire),
            halt: self.halt.load(Ordering::Acquire),
            canceled: self.canceled.load(Ordering::Acquire),
            // The thread never ends a slice, so it never counts a boundary.
            boundary: 0,
            window: self.window.load(Ordering::Acquire),
            apic_eoi: self.apic_eoi.load(Ordering::Acquire),
            apic_write: self.apic_write.load(Ordering::Acquire),
            other: self.other.load(Ordering::Acquire),
        }
    }
}

/// Where a park is asked for, published and waited on.
///
/// One mutex for the three, because they are three readings of one question —
/// is this thread running? — and a waiter that saw two of them from different
/// instants could conclude a thread had parked when it had only been asked to.
#[derive(Default)]
struct ParkSlot {
    /// What the next park should report. Set by [`VcpuControl::request_park`]
    /// BEFORE the flag the thread checks, so a thread that sees the flag always
    /// finds the reason that set it.
    requested: Option<Parked>,
    /// What the thread reported when it parked; `None` while it runs.
    parked: Option<Parked>,
    /// The thread should leave its run loop rather than wait for a resume.
    stop: bool,
}

/// The cross-thread controls for one vCPU thread.
///
/// Cloned to whoever needs to speak to the thread — the front end, the device
/// thread, the engine's own interrupt path — because every one of them holds
/// the same three shared things and none of them owns the thread.
#[derive(Clone)]
pub(crate) struct VcpuControl {
    /// Set before every cancel, cleared by a resume. The thread checks it
    /// before each entry, which is what keeps a cancel that landed between two
    /// runs from being lost.
    exit_requested: Arc<AtomicBool>,
    /// A PIC INT pin is high and the thread has not yet staged it.
    ///
    /// The dedup lives here rather than at the machine, so a pin reported at
    /// every boundary costs one cancel per VECTOR — see [`ext_int_request`].
    /// Cleared by the thread's own pre-run staging, and only there: the flag
    /// outlives a masked LINT0 and a guest that cannot yet take a delivery, so
    /// neither loses the interrupt it is owed.
    ext_int_pending: Arc<AtomicBool>,
    /// Whether this processor's thread is inside `WHvRunVirtualProcessor`
    /// right now.
    ///
    /// The one question a canceller must ask. `WHvCancelRunVirtualProcessor`
    /// issued to a processor that is NOT inside a run is latched by the
    /// platform and spent on the next entry, which returns having retired no
    /// instruction — undocumented behaviour, measured here. A guest that exits
    /// often is then never able to execute its way to the `STI` that would let
    /// it accept the vector.
    ///
    /// `SeqCst` on every access, on both sides: this and `ext_int_pending` are
    /// a Dekker pair, and a weaker ordering admits the interleaving in which
    /// the raiser reads this as false while the thread reads the request as
    /// unset, so neither acts and the vector is owed forever.
    pub(crate) in_run: Arc<AtomicBool>,
    /// Whether the last staging attempt found the guest unable to take the
    /// vector it is owed.
    ///
    /// The thread's answer to a question the platform will not answer: with the
    /// deliverability notification dead under an emulated APIC (probe P10),
    /// nothing reports that a blocked guest became ready, so while this stands
    /// every pin republication cancels the run again instead of trusting the
    /// dedup. Set only by a guest that WANTED the interrupt and could not take
    /// it — a masked LINT0 leaves it clear, because a guest that masked its own
    /// line is not waiting for anything and unmasking it traps anyway.
    ext_int_blocked: Arc<AtomicBool>,
    canceller: Canceller,
    park: Arc<(Mutex<ParkSlot>, Condvar)>,
    census: Arc<SharedCensus>,
}

/// Cancelling a run, as the park protocol needs it.
///
/// A trait for one implementor, so [`park_request`]'s ORDER can be observed
/// from inside the cancel: a test's canceller records what the flag said at the
/// instant it was called, which is the only way to assert that the flag was set
/// first without a partition and a race to lose.
pub(crate) trait CancelRun {
    /// Ask the processor to leave `WHvRunVirtualProcessor`.
    ///
    /// # Errors
    /// Whatever the platform said.
    fn cancel(&self) -> WhpResult<()>;
}

impl CancelRun for Canceller {
    fn cancel(&self) -> WhpResult<()> {
        Canceller::cancel(self)
    }
}

/// Ask a thread to leave its run: flag first, cancel second.
///
/// The order is the whole correctness argument. A cancel is sticky for exactly
/// one entry, so one that lands in the gap between a run returning and the next
/// beginning is consumed by an entry the requester never meant to end — and the
/// run it was aimed at proceeds. With the flag set first, that entry never
/// happens: the thread's own check at the head of its loop sees the flag and
/// parks, and the stray cancel is spent on nothing.
///
/// # Errors
/// Whatever the platform said about the cancel. The flag is set either way, and
/// deliberately left set: the thread parks at the next point it looks at the
/// flag, which is the head of its loop. A guest that never leaves the partition
/// — one that neither exits nor can be cancelled — reaches no such point, so a
/// refused cancel against it is a park that never happens rather than a slow
/// one. That is the wedge `FastMachine` bounds with a timeout and answers by
/// detaching the thread; clearing the flag here would turn every slow park into
/// a lost one to no benefit.
pub(crate) fn park_request(
    exit_requested: &AtomicBool,
    cancel: &impl CancelRun,
) -> WhpResult<()> {
    exit_requested.store(true, Ordering::Release);
    cancel.cancel()
}

/// One vector owed, one cancel — however many boundaries report the pin.
///
/// The machine publishes the 8259's INT pin at every boundary that finds it
/// asserted, because a level sampled at a boundary can miss the gap between two
/// interrupts entirely (`SliceEngine::pic_pin_changed`). That makes the dedup
/// this side's obligation, and this is where it is discharged: the flag moves
/// false→true exactly once per vector the thread has yet to stage, and only the
/// thread that moves it pays for a cancel. A pin held high across a thousand
/// boundaries therefore costs one exit, not a thousand.
///
/// A free function over [`CancelRun`] for the same reason [`park_request`] is
/// one: the property is about what the canceller sees, and a partition cannot
/// be asked what it was not told.
///
/// The dedup has one exception, and without it a vector can be owed forever.
/// A staging attempt that found the guest unable to take the interrupt sets
/// `blocked`, and while that stands every request cancels again rather than
/// returning early. The reason is measured: `WHvX64RegisterDeliverabilityNotifications`
/// never produces an interrupt-window exit under an emulated local APIC
/// (`docs/whp-interrupt-window-2026-09-06.md`, probe P10 — the same guest under
/// `LocalApicEmulationMode::None` takes the window exit on its own), so nothing
/// tells this engine that a blocked guest became ready. A guest that clears its
/// own `IF` for a handler and returns with `IRET` — which is no exit at all —
/// would otherwise never be asked again.
///
/// It stays cheap where it matters. The common case is one cancel per vector:
/// the first request cancels, the thread stages it into a guest that can take
/// it, and `blocked` is never set. Only a guest that was busy pays a second,
/// and it pays at its own interrupt rate rather than on a timer.
///
/// And a cancel goes only to a processor that is inside its run. One issued to
/// a processor between runs is latched by the platform and spent on the next
/// entry, which then retires nothing — [`VcpuControl::in_run`] is the account
/// of that, and of the ordering that keeps a request raised in the gap from
/// being lost. The flag is set here regardless, so the thread's own entry
/// stages the request whether or not it was cancelled for.
///
/// # Errors
/// Whatever the platform said about the cancel. The flag is left SET either
/// way: the interrupt is owed whether or not the processor could be fetched out
/// to take it, and the thread's next entry stages it regardless.
pub(crate) fn ext_int_request(
    pending: &AtomicBool,
    blocked: &AtomicBool,
    in_run: &AtomicBool,
    cancel: &impl CancelRun,
) -> WhpResult<()> {
    let first = pending
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok();
    if !(first || blocked.load(Ordering::SeqCst)) {
        return Ok(());
    }
    // Only a processor inside its run can be fetched out of one. A cancel to
    // one that is between runs is latched and spent on the next entry; the
    // thread stages this request before that entry anyway, so there is nothing
    // a cancel here could win.
    if in_run.load(Ordering::SeqCst) {
        return cancel.cancel();
    }
    Ok(())
}

impl VcpuControl {
    /// Ask the thread to leave its run and park with `why`.
    ///
    /// # Errors
    /// Whatever the platform said about the cancel; the thread is still bound
    /// to park.
    pub(crate) fn request_park(&self, why: Parked) -> WhpResult<()> {
        // The FIRST reason wins, as it does in `stop`. A second request
        // arriving before the thread has consumed the first is describing the
        // same park, and overwriting would let a later `Paused` rename a
        // `GuestPowerOff` the machine cannot rediscover — the same precedence
        // `StopCause::displaces` keeps on the machine's side.
        let mut slot = self.slot();
        let reason = *slot.requested.get_or_insert(why);
        drop(slot);
        let asked = park_request(&self.exit_requested, &self.canceller);
        debug_assert!(
            reason == why || matches!(reason, Parked::GuestPowerOff | Parked::Fault(_)),
            "a park already requested for {reason:?} was not replaced by {why:?}"
        );
        asked
    }

    /// The 8259's INT pin is asserted.
    ///
    /// Cancels the processor only for a vector the thread has not yet staged,
    /// and only while the processor is inside a run — [`ext_int_request`] is
    /// both rules. A pin reported at every boundary therefore costs one exit
    /// rather than one per boundary, and a processor between runs stages the
    /// vector at its own next entry. The vector itself is not acknowledged
    /// here: nothing is taken from the machine's controllers until the thread
    /// has positive evidence that the guest can take it.
    ///
    /// # Errors
    /// Whatever the platform said about the cancel.
    pub(crate) fn raise_ext_int(&self) -> WhpResult<()> {
        ext_int_request(
            &self.ext_int_pending,
            &self.ext_int_blocked,
            &self.in_run,
            &self.canceller,
        )
    }

    /// Wait for the thread to park, answering what it parked for.
    ///
    /// `None` if it has not within `within` — a bound rather than a wait,
    /// because the caller is the one that knows what a thread which did not
    /// stop means: a test fails, a pause reports a wedged processor.
    pub(crate) fn wait_parked_by(&self, within: Duration) -> Option<Parked> {
        let (slot, woken) = &*self.park;
        let guard = slot.lock().unwrap_or_else(PoisonError::into_inner);
        let (guard, _) = woken
            .wait_timeout_while(guard, within, |slot| slot.parked.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        guard.parked
    }

    /// Let a parked thread run again.
    ///
    /// Clears the flag before the park slot, in the mirror image of
    /// [`park_request`]: the thread wakes on the slot, and a thread that woke
    /// with the flag still set would park again immediately.
    pub(crate) fn resume(&self) {
        self.exit_requested.store(false, Ordering::Release);
        let mut slot = self.slot();
        slot.requested = None;
        slot.parked = None;
        drop(slot);
        self.park.1.notify_all();
    }

    /// Tell the thread to leave its run loop for good.
    ///
    /// Works on a running thread and on a parked one alike: the flag and the
    /// cancel bring a running thread out of the partition, and the notification
    /// wakes a parked one instead of leaving it waiting for a resume that will
    /// never come. Joining the thread afterwards is what releases the [`Vcpu`]
    /// before the partition it names is destroyed.
    ///
    /// # Errors
    /// Whatever the platform said about the cancel.
    pub(crate) fn stop(&self) -> WhpResult<()> {
        let mut slot = self.slot();
        slot.stop = true;
        slot.requested.get_or_insert(Parked::Paused);
        drop(slot);
        let asked = park_request(&self.exit_requested, &self.canceller);
        self.park.1.notify_all();
        asked
    }

    /// This thread's account of itself. Touches no processor.
    pub(crate) fn census(&self) -> VcpuCensus {
        VcpuCensus {
            runs: self.census.runs.load(Ordering::Acquire),
            in_run_nanos: self.census.in_run_nanos.load(Ordering::Acquire),
            exits: self.census.exits.snapshot(),
            export_calls: self.census.export_calls.load(Ordering::Acquire),
            platform_at_last_park: *self
                .census
                .platform_at_last_park
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        }
    }

    /// The park slot, whether or not a panicking thread poisoned it.
    ///
    /// The lock guards a plain description of what a thread is doing, not an
    /// invariant across several values: a thread that panicked while holding it
    /// left the description consistent, and refusing to read it afterwards
    /// would take away the one thing a survivor needs — the ability to stop the
    /// rest of the machine.
    fn slot(&self) -> std::sync::MutexGuard<'_, ParkSlot> {
        self.park.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Whether the thread goes back into the partition or leaves it.
///
/// Two states rather than an `Option<Parked>` (R2): "run again" is an answer,
/// not the absence of one.
enum Continue {
    /// Straight back in.
    Run,
    /// Out, for this reason.
    Park(Parked),
}

/// What the pre-run staging leaves for the entry that follows it.
///
/// Three states rather than [`Continue`] with a flag beside it (R2), because
/// the entry acts differently on each and the difference is the second half of
/// the Dekker argument in [`VcpuControl::in_run`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Staged {
    /// Nothing is owed as of the staging's last look at the flag: it was
    /// clear, or the vector it carried was placed or spent here. A request
    /// that lands after that look may have found `in_run` false and cancelled
    /// nothing, so the entry re-reads the flag under `in_run` before it goes
    /// in.
    Clear,
    /// A vector is owed and deliberately left standing — LINT0 is masked, or
    /// the guest cannot take a delivery yet. The entry proceeds: the guest has
    /// to execute to change either, and what asks again is the unmask trap or
    /// a republication of the pin that finds the processor inside its run.
    Owed,
    /// Out, for this reason.
    Park(Parked),
}

/// One processor, the machine it runs against, and everything servicing an exit
/// needs.
///
/// The [`Vcpu`] is owned outright: it is `Send` and not `Sync`, so the type
/// system already says one thread runs one processor, and moving the handle
/// here is what makes that thread this one.
pub(crate) struct VcpuThread<T: Instrumentation + Send> {
    vcpu: Vcpu,
    index: usize,
    machine: Arc<Mutex<std::boxed::Box<Emulator<T, WhpEngine>>>>,
    /// The guest's clock, for catching the machine's wheel up at an exit.
    ///
    /// A guest reading a timer device must be answered from the wheel as it
    /// stands AT THAT INSTANT, not as the device thread last left it: two
    /// reads with no device deadline between them would otherwise return the
    /// same count, and a guest calibrating against the PIT would measure its
    /// own loop as infinitely fast. Locked INSIDE the machine's lock, which is
    /// the order every path in this crate takes.
    clock: Arc<Mutex<VmClockSource<StdClock>>>,
    exchange: Exchange,
    xsave: XsaveArea,
    inject: InjectState,
    control: VcpuControl,
}

impl<T: Instrumentation + Send + 'static> VcpuThread<T> {
    /// Move `vcpu` onto a thread of its own and start running the guest.
    ///
    /// Returns the join handle beside the controls, and both are needed: the
    /// controls stop the thread, and the join is what discharges the obligation
    /// the processor handle carries — it names a partition the machine destroys
    /// when it drops, so the thread must be gone before the machine is.
    ///
    /// The thread starts RUNNING. A caller that wants it paused asks for a park
    /// straight away; nothing here can start it paused without leaving a window
    /// in which the processor is neither running nor parked.
    ///
    /// # Errors
    /// The processor's extended-state area, which is read once here so every
    /// later exchange patches the platform's own bytes rather than inventing
    /// them.
    pub(crate) fn spawn(
        vcpu: Vcpu,
        index: usize,
        machine: Arc<Mutex<std::boxed::Box<Emulator<T, WhpEngine>>>>,
        clock: Arc<Mutex<VmClockSource<StdClock>>>,
    ) -> Result<(JoinHandle<()>, VcpuControl)> {
        let xsave = XsaveArea::read_from(&vcpu, xsave::HostComponents::of_this_host())
            .map_err(platform_failed)?;
        let control = VcpuControl {
            exit_requested: Arc::new(AtomicBool::new(false)),
            ext_int_pending: Arc::new(AtomicBool::new(false)),
            ext_int_blocked: Arc::new(AtomicBool::new(false)),
            in_run: Arc::new(AtomicBool::new(false)),
            canceller: vcpu.canceller(),
            park: Arc::new((Mutex::new(ParkSlot::default()), Condvar::new())),
            census: Arc::new(SharedCensus::default()),
        };
        let thread = Self {
            vcpu,
            index,
            machine,
            clock,
            exchange: Exchange::at_reset(),
            xsave,
            inject: InjectState::at_reset(),
            control: control.clone(),
        };
        let join = std::thread::Builder::new()
            .name(std::format!("rusty_box vcpu {index}"))
            .spawn(move || thread.run_loop())
            .map_err(|error| {
                tracing::error!("a vCPU thread could not be started: {error}");
                CpuError::EngineFault(EngineFault::new(
                    EngineFaultKind::Host,
                    "spawning a vCPU thread",
                ))
            })?;
        Ok((join, control))
    }

    /// Enter the partition, service what comes back, and enter it again.
    ///
    /// The loop is unbounded on purpose: nothing here decides how long the
    /// guest may run. It leaves only because the hardware handed back something
    /// this thread cannot answer, or because someone asked for the processor.
    fn run_loop(mut self) {
        loop {
            // Before the entry, not after: a cancel that landed between two
            // runs is spent, and this is what makes the request behind it
            // stick. See `park_request`.
            if self.control.exit_requested.load(Ordering::Acquire) {
                let why = self.control.slot().requested.unwrap_or(Parked::Paused);
                if self.park(why) == Woke::ToStop {
                    return;
                }
                continue;
            }
            // The 8259's INTR, if one is owed and the guest can take it. Here
            // rather than after an exit because this is the only moment the
            // answer is about the processor the NEXT entry runs — which is the
            // processor that would take the vector.
            let staged = self.stage_the_legacy_interrupt();
            if let Staged::Park(why) = staged {
                if self.park(why) == Woke::ToStop {
                    return;
                }
                continue;
            }
            self.control.in_run.store(true, Ordering::SeqCst);
            // The raiser may have set the request in the window between the
            // staging above and this store, and read `in_run` as false, so it
            // issued no cancel and nothing else will report it. Only a request
            // the staging's last look found CLEAR can have gone unannounced
            // that way: one it left standing on purpose is asked for again by
            // the unmask trap or by a republication of the pin while the
            // processor is inside a run, and entering is what lets the guest
            // execute its way to either. Enter only with nothing newly owed;
            // otherwise go round and stage it.
            if staged == Staged::Clear && self.control.ext_int_pending.load(Ordering::SeqCst) {
                self.control.in_run.store(false, Ordering::SeqCst);
                continue;
            }
            self.control.census.runs.fetch_add(1, Ordering::Release);
            let entered = Instant::now();
            let exit = self.vcpu.run();
            self.control.in_run.store(false, Ordering::SeqCst);
            self.record_the_run(entered.elapsed());
            let outcome = match exit {
                Ok(exit) => self.service(&exit),
                Err(error) => Continue::Park(Parked::Fault(refused_run(&error))),
            };
            if let Continue::Park(why) = outcome {
                if self.park(why) == Woke::ToStop {
                    return;
                }
            }
        }
    }

    /// Place the 8259's pending vector in the partition, if the guest can take
    /// one.
    ///
    /// The legacy interrupt path in full, and the only one there is once the
    /// hypervisor owns the local APIC: the platform's `WHV_INTERRUPT_TYPE` has
    /// no ExtINT, so `WHvRequestInterrupt` cannot carry the 8259's wire and the
    /// pending-event slot is the whole surface. Measured: a vector placed there
    /// is delivered whether or not the guest masked LINT0, which is why the
    /// fabric's own copy of that entry is consulted first — it is the only
    /// thing that can honour the mask. Measured too: a placement into a guest
    /// that cannot take an interrupt is not held for later, it makes the next
    /// entry fail outright with `WHV_E_INVALID_VP_STATE`.
    ///
    /// Three gates, in this order, and the order is the freshness contract:
    /// the fabric's LINT0, then whether the processor the NEXT entry runs can
    /// take a delivery, and only then the acknowledge. The acknowledge IS the
    /// INTA moment and cannot be undone, so it happens with positive evidence
    /// in hand and never on the absence of evidence to the contrary. Nothing is
    /// taken from the machine's controllers on any other path out.
    ///
    /// Costs nothing at all until the pin raises the flag: the atomic is read
    /// before the machine's lock is taken, so an entry with no interrupt owed
    /// touches neither the lock nor the platform.
    ///
    /// Answers how it left the flag as well as whether to run, because the
    /// entry's re-check under `in_run` has to tell a request this staging left
    /// standing on purpose from one that arrived after its last look — see
    /// [`Staged`].
    fn stage_the_legacy_interrupt(&mut self) -> Staged {
        if !self.control.ext_int_pending.load(Ordering::SeqCst) {
            return Staged::Clear;
        }
        let Self { vcpu, index, machine, inject, control, .. } = self;
        let mut guard = match machine.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return Staged::Park(Parked::Fault(EngineFault::new(
                    EngineFaultKind::Host,
                    "machine lock poisoned by a panicking peer thread",
                )))
            }
        };
        let Processor { cpu, mut io, engine } = guard.processor(*index);
        if !io.device_manager().irq().lint0_admits_ext_int() {
            // The guest masked its own legacy line, or gave LINT0 a fixed
            // vector instead. Nothing is acknowledged, and the flag stays up so
            // the vector is still owed the moment the guest unmasks — the
            // ordinary case for firmware that opens its line late.
            return Staged::Owed;
        }
        if !inject.permits_ext_int() {
            // Blocked right now — `IF` clear, an interrupt shadow, or a
            // delivery the platform has begun. Nothing is acknowledged and the
            // flag stays up, so the vector is still owed.
            //
            // Recorded, because nothing else will report the guest becoming
            // ready. `WHvX64RegisterDeliverabilityNotifications` is accepted
            // and read back under an emulated APIC and never produces an
            // interrupt-window exit — measured against a working control in
            // `None` mode (probe P10, docs/whp-interrupt-window-2026-09-06.md).
            // With this set, every later republication of the pin cancels the
            // run again instead of being deduplicated away, which is what
            // catches the ordinary case: a guest that took one interrupt, is
            // inside its handler with `IF` clear, and returns with `IRET` —
            // not an exit, and so not otherwise a moment anyone asks again.
            control.ext_int_blocked.store(true, Ordering::Release);
            return Staged::Owed;
        }
        // Whatever happens below, this attempt was not blocked: the guest could
        // take a delivery. Cleared before the acknowledge rather than after, so
        // a fault on the way out cannot leave the flag set and turn every later
        // boundary into a cancel.
        control.ext_int_blocked.store(false, Ordering::Release);
        // THE INTA MOMENT — reached only with every gate passed. The same body
        // the interpreter acknowledges through, so the LAPIC-before-8259 order,
        // the fabric's counted acknowledge, the spurious vectors and the
        // deasserted-pin reconcile are one behaviour across both engines.
        let Some(vector) = io.pop_deliverable_vector(cpu) else {
            // Nothing was deliverable after all, and the acknowledge attempt
            // reconciled the stale pin. The edge is spent.
            control.ext_int_pending.store(false, Ordering::SeqCst);
            return Staged::Clear;
        };
        if let Err(error) =
            vcpu.write_words128(Reg::PendingEvent, PendingExtIntEvent { vector }.as_words())
        {
            return Staged::Park(Parked::Fault(refused_register(&error)));
        }
        // Load-bearing, not redundant: measured, a processor parked in `HLT`
        // under an emulated APIC sets `halt_suspend`, produces no halt exit,
        // and does NOT run the handler for a placed event until the suspend is
        // cleared. This processor is the boot processor and never awaits a
        // SIPI, so the whole register is written running rather than read back
        // and patched.
        const RUNNING: InternalActivity = InternalActivity {
            startup_suspend: false,
            halt_suspend: false,
            idle_suspend: false,
        };
        if let Err(error) = vcpu.set_internal_activity(RUNNING) {
            return Staged::Park(Parked::Fault(refused_register(&error)));
        }
        // The platform holds a delivery now; the next exit's header will say so
        // itself, and until one arrives this is the record.
        inject.note_placed_event();
        let census = engine.inject_census_mut();
        census.injected += 1;
        census.injected_per_vector[usize::from(vector)] =
            census.injected_per_vector[usize::from(vector)].saturating_add(1);
        control.ext_int_pending.store(false, Ordering::SeqCst);
        // The acknowledge changed the controllers' lines; the processor's
        // latched copy follows before anything asks.
        io.sync_io_events(cpu);
        tracing::debug!(target: "irq", "CPU: ExtINT vector {vector:#04x} placed for the partition");
        Staged::Clear
    }

    /// Answer one exit, with the machine's lock held for exactly as long as the
    /// answer takes.
    ///
    /// Every reason takes the lock, because every reason takes the exit
    /// header — the free half of a state exchange, and the fields most likely
    /// to have moved — and the header lands in the shadow processor, which the
    /// machine owns. What that costs even in the worst case is small beside a
    /// VM round trip: the lock is held for a header copy and released across
    /// the entry, so a guest that exits as fast as the hardware allows still
    /// leaves the machine free for the great majority of its time.
    fn service(&mut self, exit: &Exit) -> Continue {
        self.control.census.exits.record(exit.reason);
        // Destructured so the machine's lock borrows one field rather than the
        // whole thread: everything else the servicer needs is a sibling field,
        // and the guard lives across the call that uses them.
        let Self { vcpu, index, machine, clock, exchange, xsave, inject, control } = self;
        let mut guard = match machine.lock() {
            Ok(guard) => guard,
            // A peer thread panicked while holding the machine. Under the
            // release profile this cannot happen — a panic aborts the process —
            // so it is reachable only under the unwinding profile tests are
            // built with, and there the honest answer is to stop running a
            // guest against a machine nobody can vouch for.
            Err(_) => {
                return Continue::Park(Parked::Fault(EngineFault::new(
                    EngineFaultKind::Host,
                    "machine lock poisoned by a panicking peer thread",
                )))
            }
        };
        // The one exit whose answer is the MACHINE's own boundary rather than a
        // processor's, so it is answered here, where the machine is still
        // undivided. An EOI at a local APIC the hypervisor owns has to reach
        // the I/O APIC: a level entry whose line is still high owes its vector
        // again, and only the machine's boundary can route what the resample
        // queues back through `route_ioapic_delivery`.
        if let ExitReason::ApicEoi { vector } = exit.reason {
            let queued = {
                let Processor { mut io, .. } = guard.processor(*index);
                io.device_manager().irq_mut().resample_on_eoi(vector as u8)
            };
            if queued {
                match guard.service_device_time(0) {
                    // The guest asked to be powered off while this EOI was
                    // being routed. Nothing the processor does afterwards is
                    // wanted, so it leaves rather than running on.
                    Ok(time) if time.stop.is_some() => {
                        return Continue::Park(Parked::GuestPowerOff)
                    }
                    Ok(_) => {}
                    Err(error) => {
                        return Continue::Park(Parked::Fault(refused_service(&error)))
                    }
                }
            }
        }
        // The wheel is caught up to the clock BEFORE any arm answers, so a
        // device the guest is about to read answers from where time actually
        // stands (spec §3.2). Without it a guest that latches the PIT twice
        // with no device deadline between the reads gets the same count twice
        // and measures its own loop as infinitely fast — the device thread
        // moves the wheel on ITS schedule, which is deadlines, not exits.
        //
        // The clock is locked inside the machine's lock, which is the order
        // every path in this crate takes. Cheap when nothing is due: the
        // boundary's own no-work fast path answers an advance of zero without
        // touching a timer.
        {
            let clock = clock.lock().unwrap_or_else(PoisonError::into_inner);
            match crate::device_thread::service_once(&mut guard, &clock) {
                // A power-off found here is the machine's, not this exit's,
                // and the processor has no business running on after it.
                Ok(time) if time.stop.is_some() => {
                    return Continue::Park(Parked::GuestPowerOff)
                }
                Ok(_) => {}
                Err(error) => return Continue::Park(Parked::Fault(refused_service(&error))),
            }
        }
        let Processor { cpu, mut io, engine } = guard.processor(*index);
        engine.history.record(exit.vp.rip, history_mark(exit.reason));
        let mut servicer = Servicer { vcpu, index: *index, exchange, xsave, inject, control };
        match servicer.answer(exit, cpu, &mut io, engine) {
            Ok(carry_on) => carry_on,
            Err(error) => Continue::Park(Parked::Fault(refused_service(&error))),
        }
    }

    /// Publish `why`, then wait for a resume or a stop.
    ///
    /// The platform's own counters are refreshed here rather than by the
    /// waiter, because this thread is the only holder of the processor: a
    /// counter read from anywhere else would be a platform call against a
    /// processor another thread is inside.
    ///
    /// Called only with the machine's lock released — it is dropped when
    /// [`Self::service`] returns — because this blocks, and a thread blocking
    /// on a condition variable while holding the machine would stop the machine
    /// with it.
    fn park(&mut self, why: Parked) -> Woke {
        match self.vcpu.counters().intercept_counters().and_then(|intercepts| {
            self.vcpu
                .counters()
                .runtime_counters()
                .map(|runtime| PlatformCounters { intercepts, runtime })
        }) {
            Ok(counters) => {
                *self
                    .control
                    .census
                    .platform_at_last_park
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(counters);
            }
            Err(error) => {
                tracing::warn!("the platform would not report this processor's counters: {error}");
            }
        }
        tracing::debug!("vCPU {} parked: {why:?}", self.index);
        let (slot, woken) = &*self.control.park;
        let mut guard = slot.lock().unwrap_or_else(PoisonError::into_inner);
        // A `resume` that landed between the decision to park and this line
        // has already cleared `requested`, and parking now would wait for a
        // wake that has been and gone. Read under the slot lock, which
        // `resume` takes only after clearing the flag, so the two cannot
        // cross. Only a request can be withdrawn: a fault or a power-off is
        // this thread's own conclusion, and `stop` leaves `requested` set, so
        // neither is skipped here.
        if why == Parked::Paused && guard.requested.is_none() {
            return Woke::ToRun;
        }
        guard.parked = Some(why);
        woken.notify_all();
        while !guard.stop && guard.parked.is_some() {
            guard = woken.wait(guard).unwrap_or_else(PoisonError::into_inner);
        }
        if guard.stop {
            // Published before the thread leaves, so a caller that waits on the
            // slot is told the processor is gone rather than reading the
            // `Paused` this park was entered for and waiting for a resume that
            // nothing will answer.
            guard.parked = Some(Parked::Stopped);
            Woke::ToStop
        } else {
            Woke::ToRun
        }
    }

    /// Record what an entry cost in host time.
    ///
    /// The span is added here because here is the only moment it is known;
    /// the entry itself is counted before the run rather than after it, so a
    /// census read while the guest is running does not report the entry it is
    /// inside as never having happened. The two therefore differ by the run in
    /// progress, which is the honest reading of both.
    fn record_the_run(&self, took: Duration) {
        self.control.census.in_run_nanos.fetch_add(
            u64::try_from(took.as_nanos()).unwrap_or(u64::MAX),
            Ordering::Release,
        );
    }
}

/// Everything servicing an exit needs except the machine, whose lock the caller
/// holds for exactly as long as the call.
///
/// A borrow of the thread's own fields rather than a second owner of them: the
/// machine is behind a lock and the rest is not, so the two are separated here
/// and nowhere else.
struct Servicer<'a> {
    vcpu: &'a Vcpu,
    /// Which processor this exit came from. Most arms answer the same way for
    /// every processor; the ones that do not are the ones that touch machine
    /// state only the boot processor owns.
    index: usize,
    exchange: &'a mut Exchange,
    xsave: &'a mut XsaveArea,
    inject: &'a mut InjectState,
    control: &'a VcpuControl,
}

impl Servicer<'_> {
    /// The service match itself: one arm per exit reason, exhaustive (R5).
    ///
    /// A reason the platform grows breaks this rather than falling into a
    /// default, because the default that would be inherited is "run the guest
    /// on regardless", and a guest run on past an exit nobody answered is a
    /// guest that has silently diverged.
    ///
    /// # Errors
    /// Whatever the machine's own dispatch raised, or a register the platform
    /// would not take.
    fn answer<T: Instrumentation>(
        &mut self,
        exit: &Exit,
        cpu: &mut BxCpuC<T>,
        io: &mut PcIo<'_>,
        engine: &mut WhpEngine,
    ) -> Result<Continue> {
        // The free half of an exchange: `RIP`, `RFLAGS`, `CS` and `CR8` arrive
        // with the exit, so every arm below — and the injection gate after it —
        // runs against a shadow whose most volatile fields are already current.
        self.exchange.take_header(cpu, &exit.vp, self.inject)?;
        let carry_on = self.dispatch(exit, cpu, io, engine)?;
        // A device that answered this exit may have latched something on the
        // bus — an interrupt line, a hold request, a machine boundary to
        // service. The interpreter drains those at its own next instruction
        // boundary; this is that boundary here, and it is the one place it
        // happens (R5). Run for every exit rather than only for the ones that
        // reached a device, because the machine's other threads latch on the
        // same bus and a line nobody moved onto the processor is a line the
        // guest never sees.
        io.sync_io_events(cpu);
        Ok(carry_on)
    }

    /// One arm per exit reason.
    ///
    /// # Errors
    /// As [`Self::answer`].
    fn dispatch<T: Instrumentation>(
        &mut self,
        exit: &Exit,
        cpu: &mut BxCpuC<T>,
        io: &mut PcIo<'_>,
        engine: &mut WhpEngine,
    ) -> Result<Continue> {
        match exit.reason {
            // The one exit the platform pre-decodes: port, width, direction and
            // RAX all arrive in the exit, so no decoder is involved and the
            // machine's own device dispatch answers it directly.
            ExitReason::IoPortAccess(access) => {
                if access.string_op
                    || access.rep_prefix
                    || ports_on_the_shadow()
                    || exit.vp.rflags & RFLAGS_TF != 0
                {
                    // A string or repeated port access moves memory as well as
                    // a register, and the exit describes only the register.
                    // Finishing it from the exit alone would transfer one item
                    // and step over the rest — an IDE `REP INSW` reading a
                    // 512-byte sector would deliver two bytes and the guest
                    // would never know. The shadow runs the instruction whole,
                    // through the machine's own dispatch, exactly as the
                    // interpreter would.
                    //
                    // A guest single-stepping owes a `#DB` after this
                    // instruction as well, and finishing it from the exit —
                    // RIP arithmetic, no shadow instruction — leaves nobody
                    // to deliver it. The shadow retires it and its tail takes
                    // the trap. TF is read from the exit header, already
                    // here, so a guest that is not stepping pays nothing.
                    self.import(cpu, ExitClass::StringPort, NO_BYTES)?;
                    self.finish_the_errand(cpu, io, Trapped::Access)?;
                } else {
                    service_port_access(self.vcpu, io, exit, access)?;
                }
                Ok(Continue::Run)
            }
            // An access the partition's map does not answer: a device window,
            // a page the plan left out, or a write to a range mapped read-only.
            // The shadow finishes it, because nothing else can.
            ExitReason::MemoryAccess(access) => {
                tracing::trace!(
                    "servicing a {:?} at gpa {:#x} on the shadow processor",
                    access.access,
                    access.gpa
                );
                let bytes = usize::from(access.instruction_byte_count).min(16);
                self.import(cpu, ExitClass::Mmio, &access.instruction_bytes[..bytes])?;
                self.finish_the_errand(cpu, io, Trapped::Access)?;
                Ok(Continue::Run)
            }
            // What the guest asked about the processor. Answered by executing
            // the instruction on the shadow, so the answer is this port's
            // model rather than the host's silicon.
            ExitReason::Cpuid(access) => {
                let leaf = access.rax as u32;
                self.import(cpu, ExitClass::Cpuid, CPUID)?;
                self.finish_the_errand(cpu, io, Trapped::Cpuid { leaf })?;
                Ok(Continue::Run)
            }
            ExitReason::MsrAccess(access) => {
                self.import(cpu, ExitClass::Msr, if access.is_write { WRMSR } else { RDMSR })?;
                self.finish_the_errand(cpu, io, Trapped::Access)?;
                Ok(Continue::Run)
            }
            ExitReason::Exception => {
                // The platform hands a trapped fault back instead of
                // delivering it, so the processor still stands on the
                // instruction that raised it and the shadow can finish that
                // instruction itself. It either completes what the platform
                // refused — the firmware's write to `IA32_FEATURE_CONTROL` is
                // the one every boot reaches — or raises the same fault and
                // enters the guest's own handler. Either way the guest sees
                // what the interpreter would have shown it, which is the same
                // treatment an MMIO access, a `CPUID` and a port access get.
                self.import(cpu, ExitClass::Exception, NO_BYTES)?;
                if reports_each_fault() {
                    describe_the_fault(self.vcpu, cpu, io, exit, &engine.history);
                }
                self.finish_the_errand(cpu, io, Trapped::Access)?;
                Ok(Continue::Run)
            }
            // The platform's answer to a notification the injection path armed:
            // delivery has just become possible. No injection happens in this
            // arm — QEMU's whpx accelerator keeps the window exit a pure
            // notification too (`whpx-all.c` `whpx_vcpu_post_run` only clears
            // its `window_registered` flag) — because the staging runs with the
            // NEXT entry's own header and re-asks every gate rather than
            // trusting a stale answer.
            ExitReason::InterruptWindow => {
                self.inject.window = None;
                Ok(Continue::Run)
            }
            // A halt exit means the partition is not in the mode this thread
            // requires. Measured: under the hypervisor's own local APIC a
            // `cli; hlt` produces NO halt exit at all — the processor parks
            // inside the run with `halt_suspend` set, and the pre-run staging's
            // activity write is what starts it again. So a `Halt` arriving here
            // says the partition was configured with no APIC, and this thread's
            // whole interrupt path — the placed ExtINT, the requested I/O APIC
            // message, the EOI resample — has nothing to deliver through.
            // Running on would look like a working machine and deliver nothing.
            ExitReason::Halt => Ok(Continue::Park(Parked::Fault(EngineFault::new(
                EngineFaultKind::Unserviced,
                "X64Halt under the hypervisor APIC",
            )))),
            // Someone asked for the processor back. Which someone matters: a
            // cancel this thread's own controls asked for is a park, and any
            // other is the legacy interrupt path asking for a boundary at which
            // to stage a vector.
            ExitReason::Canceled { .. } => {
                if self.control.exit_requested.load(Ordering::Acquire) {
                    let why = self.control.slot().requested.unwrap_or(Parked::Paused);
                    return Ok(Continue::Park(why));
                }
                Ok(Continue::Run)
            }
            // The guest reprogrammed the pin its 8259 drives. The trap NOTIFIES
            // — the platform has already applied the value to its own register
            // — so what is owed here is the fabric's copy, which is the only
            // gate on the legacy path: a vector placed in the pending-event
            // slot is delivered whether or not the guest masked this entry.
            //
            // RIP is NOT advanced. Measured: an APIC write trap arrives with
            // RIP already past the store while reporting `instruction_length`
            // zero — the inverse of a memory exit — so a handler that
            // recomputed the end of the instruction would step over the one
            // after it.
            //
            // Only the boot processor's write is recorded. The legacy wire
            // exists on that processor alone (divergence D6), so an
            // application processor's LVT0 says nothing about the 8259 — and
            // the platform has already applied its value to the register that
            // processor reads.
            ExitReason::ApicWriteTrap { register: ApicWriteType::Lint0, value } => {
                if self.index == BOOT_PROCESSOR {
                    io.device_manager().irq_mut().set_bsp_lint0(value);
                }
                Ok(Continue::Run)
            }
            // The partition traps no other APIC register for this engine, so a
            // write to one is the platform's own business and the processor
            // goes back in.
            ExitReason::ApicWriteTrap { .. } => Ok(Continue::Run),
            // The platform refuses to run the processor it holds, and does not
            // say which register offends. The state it refused is read back and
            // reported whole — segments first, because the architecture's entry
            // checks put most of their rules on segments.
            ExitReason::InvalidVpRegisterValue => {
                match self.exchange.import_everything(self.vcpu, cpu, self.xsave) {
                    Ok(()) => {
                        let mut state = rusty_box::cpu::arch_state::VcpuArchState::default();
                        cpu.export_arch_state(&mut state);
                        report_the_state_the_platform_refused(&state);
                    }
                    Err(error) => tracing::error!(
                        "the platform refuses this processor and would not describe it: {error}"
                    ),
                }
                Ok(Continue::Park(unserviced(exit)))
            }
            // Already answered, before the machine was divided into a
            // processor and its parts: the resample an EOI owes the I/O APIC
            // and the boundary that routes it both need the machine whole. See
            // [`VcpuThread::service`]. The arm stands so this match remains
            // exhaustive on every reason the platform has (R5).
            ExitReason::ApicEoi { .. } => Ok(Continue::Run),
            // Exits this engine has not been asked to service. Each is real
            // guest behaviour a complete engine answers; refusing by name is
            // what keeps a half-serviced one from looking like a working
            // machine.
            // The guest asked its local APIC for a system-management
            // interrupt and the partition trapped it out rather than taking
            // it, because `apic_smi_trap` asked it to. That request is only
            // honest if the handler runs HERE: this machine models SMRAM's
            // access control in its own chipset, so a handler entered on the
            // hardware would read a memory view this machine never authorised.
            //
            // The whole processor crosses, not a class's groups: an SMI saves
            // and restores a state-save area covering registers no exit class
            // names, and the handler runs to completion before the guest is
            // handed back — a processor cannot be returned to the hardware
            // half-way into system-management mode, which has no equivalent
            // there.
            ExitReason::ApicSmiTrap => {
                self.exchange.import_everything(self.vcpu, cpu, self.xsave)?;
                io.deliver_smi(cpu);
                // Signalled, not yet taken: the event is processed when the
                // processor next runs, exactly as Bochs decides it.
                io.emulate_one(cpu)?;
                io.sync_io_events(cpu);
                run_the_shadow_out_of_smm(cpu, io)?;
                let calls = self.exchange.export_imported(self.vcpu, cpu, self.xsave)?;
                self.control
                    .census
                    .export_calls
                    .fetch_add(u64::try_from(calls).unwrap_or(u64::MAX), Ordering::Release);
                Ok(Continue::Run)
            }
            ExitReason::None
            | ExitReason::UnrecoverableException
            | ExitReason::UnsupportedFeature { .. }
            | ExitReason::SynicSintDeliverable
            | ExitReason::Rdtsc
            | ExitReason::Hypercall
            | ExitReason::ApicInitSipiTrap
            | ExitReason::Unrecognized(_) => Ok(Continue::Park(unserviced(exit))),
        }
    }

    /// Read what this exit's class needs that the partition still holds.
    ///
    /// # Errors
    /// A register the platform would not give up, or a state this port refuses.
    fn import<T: Instrumentation>(
        &mut self,
        cpu: &mut BxCpuC<T>,
        class: ExitClass,
        bytes: &[u8],
    ) -> Result<()> {
        self.exchange.import_for(self.vcpu, cpu, class, bytes, self.xsave)
    }

    /// The end every errand shares: retire the trapped instruction, deliver the
    /// trap it owes, republish deliverability from the shadow, and write back
    /// exactly what was read.
    ///
    /// One function rather than a tail repeated in every arm, because each of
    /// those obligations is a safety obligation (R5). The trap first: a
    /// single-step `#DB` is delivered at the head of the NEXT instruction, and
    /// that head belongs to the hardware, which arms a step for its own
    /// instruction and knows nothing of the shadow's. Then the republish: the
    /// errand has moved `IF` and may have armed an inhibit, and an injection
    /// gate reading the pre-errand header would acknowledge a vector it must
    /// not. Then the export, which makes the partition identical to the shadow
    /// for the groups this exit read and leaves every other group the
    /// partition's own.
    ///
    /// # Errors
    /// A fault the shadow could not take, or a register the platform would not
    /// take back.
    fn finish_the_errand<T: Instrumentation>(
        &mut self,
        cpu: &mut BxCpuC<T>,
        io: &mut PcIo<'_>,
        trapped: Trapped,
    ) -> Result<()> {
        // The trapped instruction, whole, on the machine's own dispatch path.
        // Whatever it does — completes the access, moves a sector, or raises a
        // fault and enters a handler — the processor it leaves behind is the
        // one the platform must continue from.
        io.finish_the_instruction(cpu)?;
        io.deliver_the_trap_owed(cpu)?;
        self.inject.refresh_from_shadow(cpu.interrupts_enabled(), cpu.in_interrupt_shadow());
        // Withheld on the shadow ITSELF, before the export, so that what the
        // partition receives and what the shadow keeps are one processor.
        if let Trapped::Cpuid { leaf } = trapped {
            withhold_virtualisation_from(cpu, leaf);
        }
        let calls = self.exchange.export_imported(self.vcpu, cpu, self.xsave)?;
        self.control
            .census
            .export_calls
            .fetch_add(u64::try_from(calls).unwrap_or(u64::MAX), Ordering::Release);
        Ok(())
    }
}

/// What ended a park.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Woke {
    /// A resume: back into the partition.
    ToRun,
    /// A stop: out of the run loop, so the thread can be joined.
    ToStop,
}

/// Instruction bytes for an exit that carries none.
///
/// The exchange reads them for one question — whether the instruction touches
/// the x87 or vector file — and answers "yes, import it" when there are none to
/// read. That is the right rule wherever the instruction is genuinely unknown,
/// which is a port exit (the platform pre-decoded it and reported no bytes) and
/// a trapped fault (the faulting instruction is arbitrary). The vector file
/// then crosses on those exits, which is a cost rather than a wrong answer, and
/// both are rare: a `REP INSW` moves a whole sector in one exit.
const NO_BYTES: &[u8] = &[];

/// `CPUID`. The exit names the instruction, so the exchange is told rather than
/// left to import a vector file this instruction cannot touch.
const CPUID: &[u8] = &[0x0F, 0xA2];

/// `RDMSR`, and `WRMSR` beside it. Named for the same reason as [`CPUID`].
const RDMSR: &[u8] = &[0x0F, 0x32];
const WRMSR: &[u8] = &[0x0F, 0x30];

/// Answer a port access out of the machine's own device set, then step the
/// processor past the instruction that caused it.
///
/// Generic over the register seam rather than taking a [`Vcpu`], because the
/// slice loop and the vCPU thread both call it and neither owns the other's
/// processor handle.
///
/// # Errors
/// A register the platform would not take.
pub(crate) fn service_port_access<V: VpRegisters>(
    vp: &V,
    io: &mut PcIo<'_>,
    exit: &Exit,
    access: IoPortAccess,
) -> Result<()> {
    let ticks = io.pc_system.time_ticks();
    let port = access.port;
    let width = access.access_size;

    // The platform hands back RAX WHOLE, whatever the access width, so both
    // directions have to narrow it themselves. An `OUT DX, AL` that handed a
    // device the other three bytes of RAX would be telling it something the
    // guest never wrote, and the interpreter — whose handler passes `AL`,
    // `AX` or `EAX` and nothing else — would tell it something different for
    // the same guest instruction.
    let mask: u64 = match width {
        1 => 0xFF,
        2 => 0xFFFF,
        _ => 0xFFFF_FFFF,
    };

    let rax = if access.is_write {
        io.devices.outp(
            port,
            (access.rax & mask) as u32,
            width,
            ticks,
            io.pc_system,
            io.device_manager,
            io.memory,
        );
        access.rax
    } else {
        let value = io.devices.inp(port, width, ticks, io.pc_system, io.device_manager);
        // A narrower `IN` leaves the bytes above its width as the guest had
        // them — the same rule the interpreter's own `port_in` follows.
        (access.rax & !mask) | (u64::from(value) & mask)
    };

    // Unlike a memory exit, a port exit DOES report its instruction length and
    // does not advance RIP itself, so finishing it is arithmetic rather than a
    // decode (probe finding 2).
    let resume = exit.vp.rip + u64::from(exit.vp.instruction_length);
    vp.write_words(&[Reg::Rip, Reg::Rax], &[resume, rax]).map_err(platform_failed)
}

/// The fault a register write the staging made parks with.
///
/// Separate from [`refused_run`] because the two say different things about the
/// same processor: one is a run the platform would not start, the other a
/// register it would not take, and the second is how a mode mismatch shows —
/// the pending-event slot and the activity register are both refused by a
/// partition that has no APIC to hold them.
fn refused_register(error: &WhpError) -> EngineFault {
    tracing::error!("the platform refused a register this staging wrote: {error}");
    EngineFault::with_code(EngineFaultKind::Vcpu, error.call(), error.hresult())
}

/// The fault a run the platform refused parks with.
fn refused_run(error: &WhpError) -> EngineFault {
    tracing::error!("the platform refused to run this processor: {error}");
    EngineFault::with_code(EngineFaultKind::Vcpu, error.call(), error.hresult())
}

/// The fault a failed service parks with.
///
/// A [`CpuError`] carries more than an [`EngineFault`] can, so what cannot
/// cross is logged here rather than lost: the fault names where, the log says
/// what.
fn refused_service(error: &CpuError) -> EngineFault {
    match error {
        CpuError::EngineFault(fault) => *fault,
        other => {
            tracing::error!("servicing an exit on the vCPU thread failed: {other}");
            EngineFault::new(EngineFaultKind::Host, "servicing an exit on the vCPU thread")
        }
    }
}

/// The fault an exit with no arm parks with.
///
/// Named by the platform's own reason, so a reader knows which arm is missing
/// rather than only that one is.
fn unserviced(exit: &Exit) -> Parked {
    tracing::error!(
        "no service for a {} exit at {:#x} (execution state {:#06x})",
        exit.reason.name(),
        exit.vp.rip,
        exit.vp.execution_state
    );
    Parked::Fault(EngineFault::new(EngineFaultKind::Unserviced, exit.reason.name()))
}

#[cfg(test)]
mod tests {
    use super::{ext_int_request, park_request, CancelRun};
    use crate::engine::ext_int_permitted;
    use rusty_box_whp::{SegmentRegister, VpContext, WhpResult};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// The three header bits the freshness contract rests on, and nothing else.
    ///
    /// The staging acknowledges at the machine's controllers on this answer,
    /// and an acknowledge cannot be undone — so it must be positive evidence
    /// that delivery is permitted, never the absence of evidence that it is
    /// blocked. A predicate that read two of the three would place a vector
    /// into a context the VM-entry checks reject, and the vector would be lost
    /// with the entry.
    #[test]
    fn the_ext_int_readiness_predicate_reads_the_three_header_bits() {
        let base = VpContext {
            rip: 0,
            rflags: 0x202,
            cs: SegmentRegister {
                base: 0,
                limit: 0xFFFF,
                selector: 0,
                attributes: 0x9B,
            },
            instruction_length: 0,
            cr8: 0,
            execution_state: 0,
        };
        assert!(ext_int_permitted(&base), "IF set, no shadow, nothing pending");
        assert!(!ext_int_permitted(&VpContext { rflags: 0x002, ..base }), "IF clear");
        assert!(
            !ext_int_permitted(&VpContext { execution_state: 1 << 12, ..base }),
            "interrupt shadow"
        );
        assert!(
            !ext_int_permitted(&VpContext { execution_state: 1 << 6, ..base }),
            "an interruption is already pending"
        );
    }

    /// A canceller that records what the flag said at the moment it was asked
    /// to cancel.
    struct RecordingCancel<'a> {
        flag: &'a AtomicBool,
        saw: Cell<Option<bool>>,
    }

    impl CancelRun for RecordingCancel<'_> {
        fn cancel(&self) -> WhpResult<()> {
            self.saw.set(Some(self.flag.load(Ordering::Acquire)));
            Ok(())
        }
    }

    /// A canceller that counts.
    struct CountingCancel(Cell<u32>);

    impl CancelRun for CountingCancel {
        fn cancel(&self) -> WhpResult<()> {
            self.0.set(self.0.get() + 1);
            Ok(())
        }
    }

    /// One cancel per vector owed, however many boundaries report the pin.
    ///
    /// The machine publishes the 8259's INT pin at every boundary that finds it
    /// asserted, so this dedup is the only thing between one interrupt and a
    /// processor fetched out of the hardware thousands of times a second. The
    /// second half matters as much as the first: once the thread has staged the
    /// vector and cleared the flag, the NEXT interrupt must cancel again, or a
    /// guest that takes one interrupt never takes another.
    #[test]
    fn a_pin_reported_at_every_boundary_costs_one_cancel_per_vector() {
        let owed = AtomicBool::new(false);
        let blocked = AtomicBool::new(false);
        let in_run = AtomicBool::new(true);
        let cancel = CountingCancel(Cell::new(0));
        for _ in 0..8 {
            ext_int_request(&owed, &blocked, &in_run, &cancel).unwrap();
        }
        assert_eq!(cancel.0.get(), 1, "eight boundaries, one interrupt, one cancel");
        assert!(owed.load(Ordering::Acquire), "and the vector is still owed");

        // The thread staged it and cleared the flag.
        owed.store(false, Ordering::Release);
        ext_int_request(&owed, &blocked, &in_run, &cancel).unwrap();
        assert_eq!(cancel.0.get(), 2, "the next interrupt fetches the processor out again");
    }

    /// A guest that could not take the vector is fetched out again at every
    /// boundary until it can.
    ///
    /// The dedup above is right for a guest that simply has not been reached
    /// yet, and wrong for one that was reached and refused: nothing reports a
    /// blocked guest becoming ready, because the platform's deliverability
    /// notification never fires under an emulated APIC (probe P10). The
    /// ordinary shape of that is a guest inside its own interrupt handler with
    /// `IF` clear, which returns with `IRET` — not an exit, and so not a moment
    /// anyone would otherwise ask again. Without this the second interrupt of a
    /// machine's life is delivered and no later one ever is.
    #[test]
    fn a_blocked_guest_is_fetched_out_again_at_every_boundary() {
        let owed = AtomicBool::new(true);
        let blocked = AtomicBool::new(true);
        let in_run = AtomicBool::new(true);
        let cancel = CountingCancel(Cell::new(0));
        for _ in 0..8 {
            ext_int_request(&owed, &blocked, &in_run, &cancel).unwrap();
        }
        assert_eq!(
            cancel.0.get(),
            8,
            "the vector was owed and the guest could not take it, so every boundary asks again"
        );

        // The staging attempt that finds the guest ready clears it, and the
        // dedup applies once more.
        blocked.store(false, Ordering::Release);
        for _ in 0..8 {
            ext_int_request(&owed, &blocked, &in_run, &cancel).unwrap();
        }
        assert_eq!(cancel.0.get(), 8, "a guest that can take it costs nothing further");
    }

    /// A request raised while the processor is between runs cancels nothing.
    ///
    /// `WHvCancelRunVirtualProcessor` is latched when the processor is not
    /// inside `WHvRunVirtualProcessor`: the next entry returns having retired
    /// no instruction. A guest that exits often is therefore never able to
    /// execute its way to the `STI` that would let it accept the vector, which
    /// is the livelock this flag exists to prevent. The request stays owed, and
    /// the thread's own next entry stages it.
    #[test]
    fn a_request_raised_between_runs_cancels_nothing() {
        let owed = AtomicBool::new(false);
        let blocked = AtomicBool::new(false);
        let in_run = AtomicBool::new(false);
        let cancel = CountingCancel(Cell::new(0));

        ext_int_request(&owed, &blocked, &in_run, &cancel).expect("a request is recordable");

        assert_eq!(cancel.0.get(), 0, "a processor that is not running is not cancelled");
        assert!(owed.load(Ordering::SeqCst), "and the vector is still owed");
    }

    /// A request raised while the processor is inside its run cancels once.
    ///
    /// This is the case the cancel exists for: a guest in a long run that takes
    /// no exits has no other moment at which the thread could stage a vector.
    #[test]
    fn a_request_raised_inside_a_run_cancels_once() {
        let owed = AtomicBool::new(false);
        let blocked = AtomicBool::new(false);
        let in_run = AtomicBool::new(true);
        let cancel = CountingCancel(Cell::new(0));

        ext_int_request(&owed, &blocked, &in_run, &cancel).expect("a request is recordable");

        assert_eq!(cancel.0.get(), 1, "a running processor is fetched out exactly once");
        assert!(owed.load(Ordering::SeqCst), "and the vector is owed until it is staged");
    }

    #[test]
    fn a_park_request_sets_the_flag_before_it_cancels() {
        let flag = AtomicBool::new(false);
        let cancel = RecordingCancel { flag: &flag, saw: Cell::new(None) };
        park_request(&flag, &cancel).unwrap();
        assert_eq!(
            cancel.saw.get(),
            Some(true),
            "the cancel saw the flag already set — a cancel landing between runs is consumed \
             by the flag check, never lost"
        );
        assert!(flag.load(Ordering::Acquire));
    }
}
