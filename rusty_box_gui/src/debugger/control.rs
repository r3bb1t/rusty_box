//! The one channel between the shell and a VM's run thread for pausing,
//! resuming, stepping, reading and saving RAM. Every write of the run's stop
//! flag goes through here, under one lock (R5), so a stop the shell asks for
//! is never taken for a pause, and a resume never lowers a stop.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use rusty_box::cpu::{BreakpointRefusal, CodeBreakpoint, CodeBreakpointHit};
use rusty_box::emulator::PowerState;

use super::inspect::{CodeTarget, DebugSnapshot, MemoryTarget, ViewRequest};
use super::source_breakpoints::{admits, AppliedBreakpoints, BreakpointTarget, ScopedAddress};
use super::stepping::{StepCeiling, StepRequest};
use crate::symbols::hub::SymbolsView;
use super::kernel_layout::{same_layouts, KernelLayout};
use super::search::{FindOutcome, FindRequest};
use super::windows::WindowsDiscovery;
use super::frames::InspectRequest;
#[cfg(feature = "gui-egui")]
use super::frames::InspectAsk;
use super::xref::service::{CopyRequest, XrefService};
#[cfg(feature = "gui-egui")]
use super::xref::service::{CopyModule, IndexState};

/// What the shell asked of the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wanted {
    Run,
    Pause,
}

/// Where the run thread is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunPhase {
    Running,
    Paused,
}

/// Work a run thread does while its machine is between runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Task {
    /// Read the snapshot the Debugger page shows.
    Refresh,
    /// Step the processor `request` names.
    Step(StepRequest),
    /// Write the machine's RAM to this file.
    DumpRam(PathBuf),
    /// Search memory, and show the hit.
    Find(FindRequest),
    /// Give the machine this set of code breakpoints.
    ApplyBreakpoints(Vec<CodeBreakpoint>),
    /// Copy these modules' images for the cross-reference service.
    CopyImages(CopyRequest),
}

/// What a run thread does once its machine handed control back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AfterRun {
    End,
    Serve,
}

/// What a run thread does once its machine stopped at a breakpoint that is
/// not the owner's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Passing {
    /// Nothing was asked: the run goes on through it.
    RunOn,
    /// What [`RunControl::after_run`] says.
    Then(AfterRun),
}

/// A run thread's next move while it serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BetweenRuns {
    Perform(Task),
    /// The stop flag is lowered: run on.
    Resume,
    End,
}

/// What became of a RAM dump.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DumpOutcome {
    Saved { path: PathBuf, bytes: u64 },
    Failed { path: PathBuf, reason: String },
}

/// Whether a dump was taken on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DumpRequest {
    Queued,
    NotRunning,
}

/// Something the run thread refused, for the page to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DebugNote {
    /// A machine on the hypervisor retires instructions nothing counts, so it
    /// cannot stop after one.
    StepNeedsInterpreter,
    /// A machine on the hypervisor meets no instruction boundary this port
    /// sees, so it cannot stop before one.
    BreakpointsNeedInterpreter,
    /// The machine holds at most `MAX_LINEAR_BREAKPOINTS` code breakpoints of
    /// a kind.
    TooManyBreakpoints,
    /// A step stopped before it finished.
    StepCut(StepCut),
}

/// Why a step stopped before it finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepCut {
    Ceiling(StepCeiling),
    /// The owner stopped it.
    Cancelled,
    /// Step out: the unwinder did not find where the frame returns.
    NoCaller,
    /// The machine did not take the step's breakpoints beside the owner's.
    BreakpointsRefused(BreakpointRefusal),
}

/// How a run begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunStart {
    /// The shell asked for it and lowered the stop flag then; a stop raised
    /// since is honoured.
    PowerOn,
    /// The run before it ended on a restart; the stop that ended it is spent,
    /// and lowered, unless a stop was asked for since that run ended.
    Restart,
}

/// Why a dump still queued when its VM stopped was not written.
pub(crate) const NOT_SAVED_VM_STOPPED: &str = "the VM stopped before its RAM was saved";

struct ControlState {
    /// A run has begun and not finished.
    live: bool,
    wanted: Wanted,
    phase: RunPhase,
    /// The shell asked the run to end: power off, restart, quit.
    ending: bool,
    /// The flag stands raised for the shell's work (a pause or a dump), so
    /// the run serves when it comes back, even if that work was withdrawn
    /// in the meantime (a pause answered by Resume before the run saw it).
    serve_requested: bool,
    /// A dump the run thread has taken off the queue and is writing.
    dump_in_flight: bool,
    tasks: VecDeque<Task>,
    view: ViewRequest,
    snapshot: Option<Arc<DebugSnapshot>>,
    /// The snapshot before `snapshot`, which the page compares against to
    /// mark what a step changed.
    previous: Option<Arc<DebugSnapshot>>,
    /// The guest has executed since `snapshot` was read: a pause that
    /// followed a run, or a step. A refresh that only changes the view finds
    /// it clear, and then the guest stands exactly as `snapshot` shows it.
    guest_ran: bool,
    /// The last search's outcome, and whether one is queued or running.
    find: Option<FindOutcome>,
    searching: bool,
    note: Option<DebugNote>,
    dump_outcomes: Vec<DumpOutcome>,
    /// The kernel PDB's layouts, which the shell hands over once its tables
    /// load; the walks read the guest's structures under them.
    kernel_layout: Option<Arc<KernelLayout>>,
    /// The owner's breakpoints as named: addresses and source lines.
    targets: Vec<BreakpointTarget>,
    /// The shell's code breakpoints, derived from `targets`: given to the
    /// machine at every run's start, and again whenever the shell changes
    /// them.
    breakpoints: Vec<CodeBreakpoint>,
    /// The addresses of `breakpoints` that are the owner's only in some
    /// address spaces: a user program's lines.
    scopes: Vec<ScopedAddress>,
    /// The code breakpoint the run is paused at, until it runs on or steps.
    stopped_at: Option<CodeBreakpointHit>,
    /// The set the machine took last: what a step gives it back after its
    /// own breakpoints (a set it refused leaves the one before in place).
    applied: Vec<CodeBreakpoint>,
    /// The symbol engine's latest view, which a step reads line rows from.
    symbols: Option<Arc<SymbolsView>>,
    /// What the variables panes ask of each read: the picked frame, the
    /// watches, the expansions.
    inspect: InspectRequest,
}

/// One VM's run control, shared by the shell and the VM's run thread.
pub(crate) struct RunControl {
    stop_flag: Arc<AtomicBool>,
    /// The owner asked the step in progress to stop. A step runs with the
    /// stop flag raised (the run is paused), so it has its own.
    step_cancel: AtomicBool,
    /// A step was taken off the queue and has not finished.
    stepping: AtomicBool,
    state: Mutex<ControlState>,
    changed: Condvar,
    /// What the run's pauses have learned about the guest's structures. Only
    /// the run thread reads it, so the lock is never contended.
    discovery: Mutex<WindowsDiscovery>,
    /// The VM's cross-reference indexes: the run thread hands it images, the
    /// shell reads answers from it.
    xrefs: Arc<XrefService>,
}

impl RunControl {
    pub(crate) fn new(stop_flag: Arc<AtomicBool>) -> Self {
        Self {
            stop_flag,
            step_cancel: AtomicBool::new(false),
            stepping: AtomicBool::new(false),
            state: Mutex::new(ControlState {
                live: false,
                wanted: Wanted::Run,
                phase: RunPhase::Running,
                ending: false,
                serve_requested: false,
                dump_in_flight: false,
                tasks: VecDeque::new(),
                view: ViewRequest::default(),
                snapshot: None,
                previous: None,
                guest_ran: false,
                find: None,
                searching: false,
                note: None,
                dump_outcomes: Vec::new(),
                kernel_layout: None,
                targets: Vec::new(),
                breakpoints: Vec::new(),
                scopes: Vec::new(),
                stopped_at: None,
                applied: Vec::new(),
                symbols: None,
                inspect: InspectRequest::default(),
            }),
            changed: Condvar::new(),
            discovery: Mutex::new(WindowsDiscovery::default()),
            xrefs: Arc::new(XrefService::new()),
        }
    }

    /// The VM's cross-reference service.
    pub(crate) fn xrefs(&self) -> &Arc<XrefService> {
        &self.xrefs
    }

    /// Asks a paused run to copy the modules of `request` that have no index
    /// and are not being copied or indexed (a failed one is tried again).
    #[cfg(feature = "gui-egui")]
    pub(crate) fn find_references(&self, request: CopyRequest) {
        let wanted: Vec<CopyModule> = request
            .modules
            .into_iter()
            .filter(|module| matches!(self.xrefs.state(&module.key), None | Some(IndexState::Failed(_))))
            .collect();
        if wanted.is_empty() {
            return;
        }
        let mut state = self.lock();
        if state.phase != RunPhase::Paused || state.wanted != Wanted::Pause || state.ending {
            return;
        }
        let keys: Vec<_> = wanted.iter().map(|module| module.key.clone()).collect();
        self.xrefs.mark_copying(&keys);
        state.tasks.push_back(Task::CopyImages(CopyRequest { modules: wanted, ..request }));
        self.changed.notify_all();
    }

    /// Stops finding references: copies not yet made are dropped, and the
    /// index under way and those queued are stopped.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn cancel_references(&self) {
        self.lock().tasks.retain(|task| !matches!(task, Task::CopyImages(_)));
        self.xrefs.cancel();
    }

    /// A lock a panicking thread poisoned still holds a whole state: every
    /// change to it is made inside one critical section.
    fn lock(&self) -> MutexGuard<'_, ControlState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The flag the run's machine polls.
    pub(crate) fn stop_flag(&self) -> &Arc<AtomicBool> {
        &self.stop_flag
    }

    // ── the shell's side ──

    /// A power-on lowers the stop its VM's last run left raised. The run
    /// never lowers it at its start, so a stop raised from here on, before
    /// the run begins, is honoured.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn arm_for_power_on(&self) {
        let mut state = self.lock();
        state.ending = false;
        state.serve_requested = false;
        state.wanted = Wanted::Run;
        self.stop_flag.store(false, Ordering::Relaxed);
    }

    #[cfg(feature = "gui-egui")]
    pub(crate) fn pause(&self) {
        let mut state = self.lock();
        if !state.live || state.ending {
            return;
        }
        state.wanted = Wanted::Pause;
        state.serve_requested = true;
        self.stop_flag.store(true, Ordering::Relaxed);
        self.changed.notify_all();
    }

    #[cfg(feature = "gui-egui")]
    pub(crate) fn resume(&self) {
        let mut state = self.lock();
        state.wanted = Wanted::Run;
        state.note = None;
        state.stopped_at = None;
        self.changed.notify_all();
    }

    /// The shell's code breakpoints become `applied`. A live run is given a
    /// changed set at once: a paused one between its tasks, a running one
    /// stopped for them and run on after, as a dump is. Scopes alone are the
    /// run thread's to judge hits by, and ask nothing of the machine.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn set_breakpoints(&self, applied: AppliedBreakpoints) {
        let mut state = self.lock();
        let AppliedBreakpoints { set, scopes } = applied;
        state.scopes = scopes;
        if state.breakpoints == set {
            return;
        }
        state.breakpoints = set.clone();
        if !state.live || state.ending {
            return;
        }
        // Only the newest set is worth giving the machine.
        state.tasks.retain(|task| !matches!(task, Task::ApplyBreakpoints(_)));
        state.tasks.push_back(Task::ApplyBreakpoints(set));
        if state.phase == RunPhase::Running {
            state.serve_requested = true;
            self.stop_flag.store(true, Ordering::Relaxed);
        }
        self.changed.notify_all();
    }

    /// The owner's breakpoints become `targets`; the shell derives the code
    /// breakpoints from them ([`Self::set_breakpoints`]).
    #[cfg(feature = "gui-egui")]
    pub(crate) fn set_targets(&self, targets: Vec<BreakpointTarget>) {
        self.lock().targets = targets;
    }

    #[cfg(feature = "gui-egui")]
    pub(crate) fn targets(&self) -> Vec<BreakpointTarget> {
        self.lock().targets.clone()
    }

    /// The code breakpoint the run is paused at.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn stopped_at(&self) -> Option<CodeBreakpointHit> {
        self.lock().stopped_at
    }

    /// Asks for a step; only a paused run takes it.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn step(&self, request: StepRequest) {
        let mut state = self.lock();
        if state.phase == RunPhase::Paused && state.wanted == Wanted::Pause && !state.ending {
            state.tasks.push_back(Task::Step(request));
            self.changed.notify_all();
        }
    }

    /// Stops the step in progress where it is; it pauses there.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn cancel_step(&self) {
        self.step_cancel.store(true, Ordering::Relaxed);
    }

    /// The work queued for the run thread, oldest first: how a test sees
    /// what the shell asked without taking it.
    #[cfg(test)]
    pub(crate) fn queued(&self) -> Vec<Task> {
        self.lock().tasks.iter().cloned().collect()
    }

    /// Whether a step is under way.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn stepping(&self) -> bool {
        self.stepping.load(Ordering::Relaxed)
    }

    /// The symbol engine's view a step reads line rows from; the shell hands
    /// over each one it draws.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn set_symbols(&self, view: Arc<SymbolsView>) {
        let mut state = self.lock();
        if state.symbols.as_ref().is_some_and(|held| Arc::ptr_eq(held, &view)) {
            return;
        }
        // A paused read that waited for a module's tables or binary is read
        // again once they are here; any other new view asks nothing, so a
        // refresh never feeds itself.
        let arrived = state.snapshot.as_ref().is_some_and(|shown| {
            shown.frames.awaiting.iter().any(|identity| view.tables(identity).is_some())
                || shown.frames.awaiting_images.iter().any(|key| view.image(key).is_some())
        });
        state.symbols = Some(view);
        if arrived {
            self.refresh_paused(&mut state);
        }
    }

    /// Queues one refresh of a paused run.
    fn refresh_paused(&self, state: &mut ControlState) {
        if state.phase == RunPhase::Paused && !state.tasks.contains(&Task::Refresh) {
            state.tasks.push_back(Task::Refresh);
            self.changed.notify_all();
        }
    }

    /// Picks the frame the variables panes show; a paused run reads again.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn select_frame(&self, frame: usize) {
        let mut state = self.lock();
        if state.inspect.frame == frame {
            return;
        }
        state.inspect.frame = frame;
        self.refresh_paused(&mut state);
    }

    /// The watches and expansions the panes ask for; a paused run reads
    /// again when they changed.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn set_inspect(&self, ask: InspectAsk) {
        let mut state = self.lock();
        let InspectAsk { watches, expanded, pages } = ask;
        if state.inspect.watches == watches && state.inspect.expanded == expanded && state.inspect.pages == pages {
            return;
        }
        state.inspect.watches = watches;
        state.inspect.expanded = expanded;
        state.inspect.pages = pages;
        self.refresh_paused(&mut state);
    }

    /// The expression the value tip shows, or none; a paused run reads it.
    /// The tip closes when the guest runs.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn show_tip(&self, tip: Option<String>) {
        let mut state = self.lock();
        if state.inspect.tip == tip {
            return;
        }
        let asked = tip.is_some();
        state.inspect.tip = tip;
        if asked {
            self.refresh_paused(&mut state);
        }
    }

    /// What the run thread reads the variables for.
    pub(crate) fn inspect(&self) -> InspectRequest {
        self.lock().inspect.clone()
    }

    /// What the page wants to see. A paused run reads it again at once.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn set_view(&self, view: ViewRequest) {
        let mut state = self.lock();
        if state.view == view {
            return;
        }
        state.view = view;
        if state.phase == RunPhase::Paused && !state.tasks.contains(&Task::Refresh) {
            state.tasks.push_back(Task::Refresh);
            self.changed.notify_all();
        }
    }

    /// The kernel PDB's layouts for the walks, or `None` to walk by discovery
    /// alone. A paused run reads the guest's structures again at once under
    /// new layouts; the snapshot it shows was read under the old ones, so
    /// nothing of it is carried over.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn set_kernel_layout(&self, layout: Option<Arc<KernelLayout>>) {
        let mut state = self.lock();
        if same_layouts(state.kernel_layout.as_ref(), layout.as_ref()) {
            return;
        }
        state.kernel_layout = layout;
        if state.phase == RunPhase::Paused && !state.tasks.contains(&Task::Refresh) {
            state.tasks.push_back(Task::Refresh);
            self.changed.notify_all();
        }
    }

    /// Asks the run to write its RAM to `path`. A running machine is stopped
    /// for the dump and runs on after it, so the file holds one instant.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn request_dump(&self, path: PathBuf) -> DumpRequest {
        let mut state = self.lock();
        if !state.live || state.ending {
            return DumpRequest::NotRunning;
        }
        state.tasks.push_back(Task::DumpRam(path));
        if state.phase == RunPhase::Running {
            state.serve_requested = true;
            self.stop_flag.store(true, Ordering::Relaxed);
        }
        self.changed.notify_all();
        DumpRequest::Queued
    }

    /// Ends the run, whatever else was asked: power off, restart, quit.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn end_run(&self) {
        let mut state = self.lock();
        state.ending = true;
        self.stop_flag.store(true, Ordering::Relaxed);
        self.changed.notify_all();
    }

    #[cfg(feature = "gui-egui")]
    pub(crate) fn wanted(&self) -> Wanted {
        self.lock().wanted
    }

    #[cfg(feature = "gui-egui")]
    pub(crate) fn phase(&self) -> RunPhase {
        self.lock().phase
    }

    /// The last snapshot a paused run published; still shown, as the state
    /// at the last pause, once the run runs on.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn snapshot(&self) -> Option<Arc<DebugSnapshot>> {
        self.lock().snapshot.clone()
    }

    /// The snapshot before the current one: what a step is compared against.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn previous_snapshot(&self) -> Option<Arc<DebugSnapshot>> {
        self.lock().previous.clone()
    }

    #[cfg(feature = "gui-egui")]
    pub(crate) fn note(&self) -> Option<DebugNote> {
        self.lock().note
    }

    /// Asks a paused run to search memory; the hit, if any, is shown.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn find(&self, request: FindRequest) {
        let mut state = self.lock();
        if state.phase == RunPhase::Paused && state.wanted == Wanted::Pause && !state.ending {
            state.tasks.push_back(Task::Find(request));
            state.searching = true;
            self.changed.notify_all();
        }
    }

    #[cfg(feature = "gui-egui")]
    pub(crate) fn find_outcome(&self) -> Option<FindOutcome> {
        self.lock().find
    }

    #[cfg(feature = "gui-egui")]
    pub(crate) fn searching(&self) -> bool {
        self.lock().searching
    }

    #[cfg(feature = "gui-egui")]
    /// A dump is queued or being written.
    pub(crate) fn dump_pending(&self) -> bool {
        let state = self.lock();
        state.dump_in_flight
            || state
                .tasks
                .iter()
                .any(|task| matches!(task, Task::DumpRam(_)))
    }

    #[cfg(feature = "gui-egui")]
    pub(crate) fn take_dump_outcomes(&self) -> Vec<DumpOutcome> {
        core::mem::take(&mut self.lock().dump_outcomes)
    }

    // ── the run thread's side ──

    /// A run begins `how`: it runs, nothing is asked of it yet. A stop asked
    /// for since the last run ended (or since the power-on) stands, so the
    /// run ends as soon as it starts; a restart otherwise lowers the stop
    /// that ended the run before it, under this lock, so a stop that lands
    /// between the two runs can never be lowered by mistake.
    pub(crate) fn begin_run(&self, how: RunStart) {
        let mut state = self.lock();
        state.live = true;
        state.wanted = Wanted::Run;
        state.phase = RunPhase::Running;
        state.serve_requested = false;
        state.note = None;
        state.tasks.clear();
        // A new run: what the last one showed is no longer this guest.
        state.snapshot = None;
        state.previous = None;
        state.guest_ran = false;
        state.find = None;
        state.searching = false;
        state.stopped_at = None;
        *self
            .discovery
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = WindowsDiscovery::default();
        match how {
            RunStart::PowerOn => {}
            RunStart::Restart => {
                if !state.ending {
                    self.stop_flag.store(false, Ordering::Relaxed);
                }
            }
        }
    }

    /// What the run thread does once the machine handed control back. A
    /// guest that switched itself off, or a stop the shell asked for, ends
    /// the run; a flag raised for a pause or a dump is served, even when the
    /// pause was withdrawn since; a budget spent with nothing asked ends it.
    pub(crate) fn after_run(&self, power: PowerState) -> AfterRun {
        match power {
            PowerState::Running => {}
            PowerState::PoweredOff | PowerState::CpuShutdown => return AfterRun::End,
        }
        let state = self.lock();
        if state.ending {
            AfterRun::End
        } else if state.serve_requested
            || state.wanted == Wanted::Pause
            || !state.tasks.is_empty()
        {
            AfterRun::Serve
        } else {
            AfterRun::End
        }
    }

    /// What the run thread does once the machine stopped at a breakpoint that
    /// is not the owner's: it ran no budget out, so with nothing asked it
    /// runs on, the stop flag the hit raised lowered under this lock (a pause
    /// asked meanwhile is seen here first, so it is never lowered by
    /// mistake); anything asked, or a guest that switched off, is
    /// [`Self::after_run`]'s.
    pub(crate) fn after_passing(&self, power: PowerState) -> Passing {
        {
            let state = self.lock();
            let asked = state.ending || state.serve_requested || state.wanted == Wanted::Pause || !state.tasks.is_empty();
            if power == PowerState::Running && !asked {
                self.stop_flag.store(false, Ordering::Relaxed);
                return Passing::RunOn;
            }
        }
        Passing::Then(self.after_run(power))
    }

    /// The run thread's next move. A stop comes first, then queued work; a
    /// run the shell wants running resumes with its stop flag lowered under
    /// this lock; a run newly paused publishes first, then sleeps until the
    /// shell asks for something.
    pub(crate) fn next_between_runs(&self) -> BetweenRuns {
        let mut state = self.lock();
        loop {
            if state.ending {
                return BetweenRuns::End;
            }
            if let Some(task) = state.tasks.pop_front() {
                match task {
                    Task::DumpRam(_) => state.dump_in_flight = true,
                    // The step executes instructions: the snapshot it
                    // publishes follows the guest's move, off the breakpoint
                    // it may have been paused at, and its code is read at RIP
                    // again. A cancel asked before it began was for an
                    // earlier step.
                    Task::Step(_) => {
                        state.view.code = CodeTarget::Rip;
                        state.inspect.frame = 0;
                        state.inspect.tip = None;
                        state.guest_ran = true;
                        state.stopped_at = None;
                        state.note = None;
                        self.step_cancel.store(false, Ordering::Relaxed);
                        self.stepping.store(true, Ordering::Relaxed);
                    }
                    Task::Refresh | Task::Find(_) | Task::ApplyBreakpoints(_) | Task::CopyImages(_) => {}
                }
                return BetweenRuns::Perform(task);
            }
            match state.wanted {
                Wanted::Run => {
                    self.stop_flag.store(false, Ordering::Relaxed);
                    state.serve_requested = false;
                    state.phase = RunPhase::Running;
                    // The next pause's code is read where it stops, in its
                    // innermost frame.
                    state.view.code = CodeTarget::Rip;
                    state.inspect.frame = 0;
                    state.inspect.tip = None;
                    return BetweenRuns::Resume;
                }
                Wanted::Pause => {
                    if state.phase == RunPhase::Running {
                        state.phase = RunPhase::Paused;
                        // The guest ran up to this pause.
                        state.guest_ran = true;
                        return BetweenRuns::Perform(Task::Refresh);
                    }
                    state = self
                        .changed
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            }
        }
    }

    pub(crate) fn view(&self) -> ViewRequest {
        self.lock().view
    }

    /// The shell's code breakpoints, which a run gives its machine at its
    /// start.
    pub(crate) fn breakpoints(&self) -> Vec<CodeBreakpoint> {
        self.lock().breakpoints.clone()
    }

    /// The machine stopped at `hit` while running: the run pauses, showing
    /// the processor that stopped.
    pub(crate) fn stopped_at_breakpoint(&self, hit: CodeBreakpointHit) {
        let mut state = self.lock();
        if state.ending {
            return;
        }
        state.wanted = Wanted::Pause;
        state.serve_requested = true;
        self.record_stop(&mut state, hit);
        self.changed.notify_all();
    }

    /// A step stopped at `hit`: the paused run shows it.
    pub(crate) fn stepped_onto_breakpoint(&self, hit: CodeBreakpointHit) {
        let mut state = self.lock();
        self.record_stop(&mut state, hit);
    }

    /// Whether `hit`, taken where the running process's address space is
    /// `space` (its `DirectoryTableBase`), is one of the owner's stops: a
    /// user program's line stops only in the processes that run it.
    pub(crate) fn admits(&self, hit: &CodeBreakpointHit, space: u64) -> bool {
        match hit.breakpoint {
            CodeBreakpoint::Linear(linear) => admits(&self.lock().scopes, linear, space),
            CodeBreakpoint::Physical(_) => true,
        }
    }

    /// Whether any of the owner's breakpoints stops only in some address
    /// spaces, so a hit needs its address space read to be judged.
    pub(crate) fn has_scopes(&self) -> bool {
        !self.lock().scopes.is_empty()
    }

    /// The machine took `set`.
    pub(crate) fn record_applied_breakpoints(&self, set: &[CodeBreakpoint]) {
        set.clone_into(&mut self.lock().applied);
    }

    /// The set the machine holds.
    pub(crate) fn applied_breakpoints(&self) -> Vec<CodeBreakpoint> {
        self.lock().applied.clone()
    }

    /// The step under way ended.
    pub(crate) fn step_finished(&self) {
        self.stepping.store(false, Ordering::Relaxed);
    }

    /// Whether the owner asked the step in progress to stop.
    pub(crate) fn step_cancelled(&self) -> bool {
        self.step_cancel.load(Ordering::Relaxed)
    }

    /// The view a step reads line rows from.
    pub(crate) fn symbols(&self) -> Option<Arc<SymbolsView>> {
        self.lock().symbols.clone()
    }

    fn record_stop(&self, state: &mut ControlState, hit: CodeBreakpointHit) {
        state.stopped_at = Some(hit);
        state.view.cpu = hit.cpu;
    }

    /// The layouts the walks read under now.
    pub(crate) fn kernel_layout(&self) -> Option<Arc<KernelLayout>> {
        self.lock().kernel_layout.clone()
    }

    /// Shows `snapshot`, which follows the guest's move; the one it replaces
    /// is kept as the comparison that marks what changed.
    pub(crate) fn publish(&self, snapshot: DebugSnapshot) {
        let mut state = self.lock();
        state.previous = state.snapshot.take();
        state.snapshot = Some(Arc::new(snapshot));
        state.guest_ran = false;
    }

    /// Shows `snapshot`, a reading of the guest as it stands at this pause.
    /// When the guest ran since the last snapshot (a pause that followed a
    /// run) that snapshot becomes the comparison; when only the view changed
    /// (a page, a tab, a search) the comparison stays what the last step or
    /// pause made it, so what changed is still marked.
    pub(crate) fn publish_view(&self, snapshot: DebugSnapshot) {
        let mut state = self.lock();
        if state.guest_ran {
            state.previous = state.snapshot.take();
        }
        state.snapshot = Some(Arc::new(snapshot));
        state.guest_ran = false;
    }

    /// The snapshot shown, when the guest has not executed since it was
    /// read: what a refresh that only changes the view may carry over.
    pub(crate) fn unmoved_snapshot(&self) -> Option<Arc<DebugSnapshot>> {
        let state = self.lock();
        if state.guest_ran {
            None
        } else {
            state.snapshot.clone()
        }
    }

    /// What this run has learned about the guest's structure layouts.
    pub(crate) fn discovery(&self) -> MutexGuard<'_, WindowsDiscovery> {
        self.discovery
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Records a search's outcome; a hit becomes the memory pane's window.
    pub(crate) fn record_find(&self, outcome: FindOutcome) {
        let mut state = self.lock();
        state.searching = false;
        if let FindOutcome::Found(found) = outcome {
            state.view.memory = MemoryTarget::At(found.at);
        }
        state.find = Some(outcome);
    }

    pub(crate) fn note_refusal(&self, note: DebugNote) {
        self.lock().note = Some(note);
    }

    pub(crate) fn record_dump(&self, outcome: DumpOutcome) {
        let mut state = self.lock();
        state.dump_in_flight = false;
        state.dump_outcomes.push(outcome);
    }

    /// Whether the shell has asked the run to end, so a long task (a dump of
    /// several GiB) gives up rather than holding a power-off or a quit.
    pub(crate) fn ending(&self) -> bool {
        self.lock().ending
    }

    /// The run is over: the stop that ended it is spent, and a dump nobody
    /// served is reported unsaved.
    pub(crate) fn finish_run(&self) {
        let mut state = self.lock();
        state.live = false;
        state.ending = false;
        state.dump_in_flight = false;
        state.wanted = Wanted::Run;
        state.phase = RunPhase::Running;
        let unserved: Vec<Task> = state.tasks.drain(..).collect();
        for task in unserved {
            match task {
                Task::DumpRam(path) => state.dump_outcomes.push(DumpOutcome::Failed {
                    path,
                    reason: NOT_SAVED_VM_STOPPED.to_owned(),
                }),
                // Modules the run did not copy are no longer being copied: a
                // later find asks for them again.
                Task::CopyImages(request) => {
                    let keys: Vec<_> = request.modules.into_iter().map(|module| module.key).collect();
                    self.xrefs.forget(&keys);
                }
                Task::Refresh | Task::Step(_) | Task::Find(_) | Task::ApplyBreakpoints(_) => {}
            }
        }
        state.searching = false;
    }
}

#[cfg(all(test, feature = "gui-egui"))]
mod tests {
    use super::*;
    use crate::debugger::guest::Root;
    use crate::debugger::inspect::{AddressSpace, GuestAddress};
    use crate::debugger::module_identity::GuestIdentity;
    use crate::debugger::xref::service::{IndexSpace, ModuleKey};
    use crate::symbols::hub::{ImageEntry, ImageKey, ImageState, ModuleEntry, ModuleSymbols};
    use rusty_box_symbols::identity::{ModuleIdentity, PeImageId};
    use rusty_box_symbols::module_file::ModuleFile;
    use rusty_box_symbols::service::SymbolSource;
    use rusty_box_symbols::tables::SymbolTables;
    use rusty_box_symbols_fixtures::pdb::{build_pdb, PdbFixture};
    use rusty_box_symbols_fixtures::pe::{pe64_file, Pe64Spec, SpecSection, SECTION_CODE};
    use std::collections::{BTreeMap, BTreeSet};

    fn control() -> RunControl {
        RunControl::new(Arc::new(AtomicBool::new(false)))
    }

    fn raised(control: &RunControl) -> bool {
        control.stop_flag().load(Ordering::Relaxed)
    }

    fn paused_control() -> RunControl {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control
    }

    #[test]
    fn a_step_or_a_run_selects_the_innermost_frame() {
        let control = paused_control();
        control.select_frame(2);
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        assert_eq!(control.inspect().frame, 2);
        control.step(StepRequest::instruction_into(0));
        assert!(matches!(control.next_between_runs(), BetweenRuns::Perform(Task::Step(_))));
        assert_eq!(control.inspect().frame, 0);
        control.select_frame(1);
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.resume();
        assert_eq!(control.next_between_runs(), BetweenRuns::Resume);
        assert_eq!(control.inspect().frame, 0);
    }

    #[test]
    fn a_step_or_a_run_closes_the_tip() {
        let control = paused_control();
        control.show_tip(Some("count".into()));
        assert_eq!(control.queued(), [Task::Refresh], "a paused run reads the tip");
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        assert_eq!(control.inspect().tip.as_deref(), Some("count"));
        control.show_tip(Some("count".into()));
        assert_eq!(control.queued(), [], "the same tip again asks nothing");
        control.step(StepRequest::instruction_into(0));
        assert!(matches!(control.next_between_runs(), BetweenRuns::Perform(Task::Step(_))));
        assert_eq!(control.inspect().tip, None);
        control.show_tip(Some("count".into()));
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.resume();
        assert_eq!(control.next_between_runs(), BetweenRuns::Resume);
        assert_eq!(control.inspect().tip, None);
    }

    #[test]
    fn picking_a_frame_or_a_watch_refreshes_a_paused_run_once() {
        let control = paused_control();
        control.select_frame(1);
        assert_eq!(control.queued(), [Task::Refresh]);
        control.select_frame(1);
        assert_eq!(control.queued(), [Task::Refresh], "the same pick again asks nothing");
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        let ask = InspectAsk { watches: vec!["count".into()], expanded: BTreeSet::new(), pages: BTreeMap::new() };
        control.set_inspect(ask.clone());
        assert_eq!(control.queued(), [Task::Refresh]);
        assert_eq!(control.inspect().watches, ["count"]);
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.set_inspect(ask);
        assert_eq!(control.queued(), []);
    }

    #[test]
    fn new_symbols_refresh_only_a_read_that_waited_for_them() {
        let identity = ModuleIdentity::Pe {
            file_name: "flat.exe".into(),
            image: PeImageId { time_date_stamp: 1, size_of_image: 0x1_0000 },
            pdb: None,
        };
        let control = paused_control();
        let mut snapshot = empty_snapshot(0x1000);
        snapshot.frames.awaiting = vec![identity.clone()];
        control.publish(snapshot);
        control.set_symbols(Arc::new(SymbolsView::default()));
        assert_eq!(control.queued(), [], "the module's tables have not loaded");
        let tables = SymbolTables::parse(&build_pdb(&PdbFixture::new([1; 16], 1))).expect("a PDB");
        let mut view = SymbolsView::default();
        view.modules.insert(
            identity,
            ModuleEntry { state: ModuleSymbols::Loaded { from: SymbolSource::Store { path: "flat.pdb".into() } }, tables: Some(Arc::new(tables)) },
        );
        control.set_symbols(Arc::new(view));
        assert_eq!(control.queued(), [Task::Refresh]);
    }

    #[test]
    fn a_binary_refreshes_only_a_read_that_waited_for_it() {
        let key = ImageKey { file_name: "flat.exe".into(), image: PeImageId { time_date_stamp: 1, size_of_image: 0x2000 } };
        let control = paused_control();
        let mut snapshot = empty_snapshot(0x1000);
        snapshot.frames.awaiting_images = vec![key.clone()];
        control.publish(snapshot);
        let with = |entry: ImageEntry| {
            let mut view = SymbolsView::default();
            view.images.insert(key.clone(), entry);
            Arc::new(view)
        };
        control.set_symbols(with(ImageEntry { state: ImageState::Pending, file: None }));
        assert_eq!(control.queued(), [], "still being looked up");
        control.set_symbols(with(ImageEntry { state: ImageState::NotOnServer, file: None }));
        assert_eq!(control.queued(), [], "nothing new to read with");
        let binary = pe64_file(&Pe64Spec {
            time_date_stamp: 1,
            size_of_image: 0x2000,
            image_base: 0x40_0000,
            sections: vec![SpecSection { name: b".text", rva: 0x1000, bytes: vec![0xC3; 0x10], characteristics: SECTION_CODE }],
            exception: None,
            base_relocations: None,
            import_address_table: None,
        });
        let file = ModuleFile::parse(binary).expect("the binary parses");
        control.set_symbols(with(ImageEntry {
            state: ImageState::Loaded { from: SymbolSource::Store { path: "flat.exe".into() } },
            file: Some(Arc::new(file)),
        }));
        assert_eq!(control.queued(), [Task::Refresh]);
    }

    /// Code shown somewhere else (a Go to) is shown at RIP again once the
    /// guest moves: the reset happens before the run thread reads.
    #[test]
    fn a_step_or_a_run_returns_the_code_pane_to_rip() {
        let elsewhere = CodeTarget::At(GuestAddress { space: AddressSpace::Linear, value: 0x1000 });
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.set_view(ViewRequest { code: elsewhere, ..control.view() });
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        assert_eq!(control.view().code, elsewhere);
        control.step(StepRequest::instruction_into(0));
        assert!(matches!(control.next_between_runs(), BetweenRuns::Perform(Task::Step(_))));
        assert_eq!(control.view().code, CodeTarget::Rip);
        control.set_view(ViewRequest { code: elsewhere, ..control.view() });
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.resume();
        assert_eq!(control.next_between_runs(), BetweenRuns::Resume);
        assert_eq!(control.view().code, CodeTarget::Rip);
    }

    /// Modules `a.sys` (never asked), `b.sys` (being copied) and `c.sys` (its
    /// copy failed): a find copies `a` and `c` once.
    #[test]
    fn finding_references_queues_one_copy_for_modules_not_indexed() {
        let module = |name: &str, base: u64| CopyModule {
            key: ModuleKey { identity: GuestIdentity::Unnamed, name: name.to_owned(), base, size: 0x3000, space: IndexSpace::Kernel },
            root: Root::Live,
        };
        let modules = vec![module("a.sys", 0x1_0000), module("b.sys", 0x2_0000), module("c.sys", 0x3_0000)];
        let request = CopyRequest { modules: modules.clone(), spans: Vec::new(), taken_at: 9 };
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.xrefs().mark_copying(std::slice::from_ref(&modules[1].key));
        control.xrefs().fail(modules[2].key.clone(), "its headers do not read".into());
        control.find_references(request.clone());
        let copied = vec![modules[0].clone(), modules[2].clone()];
        assert_eq!(control.queued(), [Task::CopyImages(CopyRequest { modules: copied, spans: Vec::new(), taken_at: 9 })]);
        assert_eq!(control.xrefs().state(&modules[2].key), Some(IndexState::Copying));
        control.find_references(request);
        assert_eq!(control.queued().len(), 1, "nothing is asked twice");
        control.cancel_references();
        assert_eq!(control.queued(), []);
    }

    #[test]
    fn a_shell_stop_ends_the_run_whatever_else_was_asked() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.pause();
        control.end_run();
        assert_eq!(control.after_run(PowerState::Running), AfterRun::End);
        assert_eq!(control.next_between_runs(), BetweenRuns::End);
    }

    #[test]
    fn a_guest_that_switched_itself_off_ends_the_run_even_when_a_pause_was_asked() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.pause();
        assert_eq!(control.after_run(PowerState::PoweredOff), AfterRun::End);
        assert_eq!(control.after_run(PowerState::CpuShutdown), AfterRun::End);
    }

    #[test]
    fn a_pause_raises_the_stop_flag_and_the_first_look_publishes() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.pause();
        assert!(raised(&control));
        assert_eq!(control.after_run(PowerState::Running), AfterRun::Serve);
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        assert_eq!(control.phase(), RunPhase::Paused);
    }

    #[test]
    fn a_resume_lowers_the_stop_flag_unless_a_stop_was_asked() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.resume();
        assert_eq!(control.next_between_runs(), BetweenRuns::Resume);
        assert!(!raised(&control));
        assert_eq!(control.phase(), RunPhase::Running);

        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.end_run();
        control.resume();
        assert_eq!(control.next_between_runs(), BetweenRuns::End);
        assert!(raised(&control), "a resume never lowers a stop the shell asked for");
    }

    #[test]
    fn a_dump_asked_while_running_is_served_then_the_run_resumes() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        let path = PathBuf::from("memory.bin");
        assert_eq!(control.request_dump(path.clone()), DumpRequest::Queued);
        assert!(raised(&control));
        assert!(control.dump_pending());
        assert_eq!(control.after_run(PowerState::Running), AfterRun::Serve);
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::DumpRam(path)));
        assert_eq!(control.next_between_runs(), BetweenRuns::Resume);
        assert!(!raised(&control));
    }

    #[test]
    fn a_dump_asked_of_a_vm_that_is_off_is_refused() {
        assert_eq!(
            control().request_dump(PathBuf::from("memory.bin")),
            DumpRequest::NotRunning
        );
    }

    #[test]
    fn a_dump_still_queued_when_the_run_ends_is_reported_unsaved() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        let path = PathBuf::from("memory.bin");
        assert_eq!(control.request_dump(path.clone()), DumpRequest::Queued);
        control.end_run();
        assert_eq!(control.after_run(PowerState::Running), AfterRun::End);
        control.finish_run();
        assert!(!control.dump_pending());
        assert_eq!(
            control.take_dump_outcomes(),
            [DumpOutcome::Failed {
                path,
                reason: NOT_SAVED_VM_STOPPED.to_owned(),
            }]
        );
    }

    /// Pause then Resume before the run thread saw the pause: the raised
    /// flag still brings the run back, and it runs on rather than ending.
    #[test]
    fn a_pause_withdrawn_before_the_run_saw_it_resumes_the_run() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.pause();
        control.resume();
        assert_eq!(control.after_run(PowerState::Running), AfterRun::Serve);
        assert_eq!(control.next_between_runs(), BetweenRuns::Resume);
        assert!(!raised(&control));
        assert_eq!(control.after_run(PowerState::Running), AfterRun::End);
    }

    #[test]
    fn a_new_run_shows_nothing_of_the_last() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.publish(crate::debugger::inspect::DebugSnapshot {
            cpus: Vec::new(),
            cpu: 0,
            code: Vec::new(),
            code_at: CodeTarget::Rip,
            rip: 0,
            frames: crate::debugger::frames::FramesRead::default(),
            memory: crate::debugger::inspect::MemoryWindow {
                start: crate::debugger::inspect::GuestAddress {
                    space: crate::debugger::inspect::AddressSpace::Linear,
                    value: 0,
                },
                rows: Vec::new(),
            },
            stack: Vec::new(),
            process: None,
            asked: crate::debugger::windows::SystemView::Off,
            system: crate::debugger::windows::SystemListing::NotAsked,
            symbols: crate::debugger::inspect::SymbolIndex::default(),
            memory_space: None,
            context: crate::debugger::inspect::ViewContext::Live,
            viewing: crate::debugger::inspect::Viewing::Live,
            layout: None,
            layouts: None,
            through: crate::debugger::windows::ReadThrough::ProcessorTables,
            vmcs: None,
            elsewhere: crate::debugger::windows::ModulesElsewhere::default(),
        });
        control.finish_run();
        control.begin_run(RunStart::PowerOn);
        assert!(control.snapshot().is_none());
        assert!(control.previous_snapshot().is_none());
    }

    /// A restart lowers the stop that ended the run before it; a stop asked
    /// for after that run ended still stands when the restart begins, so the
    /// restarted run ends at once.
    #[test]
    fn a_restart_lowers_only_the_stop_that_ended_its_run() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.end_run(); // the restart's own stop
        control.finish_run();
        control.begin_run(RunStart::Restart);
        assert!(!raised(&control), "the stop that ended the run is spent");

        control.end_run(); // a restart again
        control.finish_run();
        control.end_run(); // a power-off between the two runs
        control.begin_run(RunStart::Restart);
        assert!(raised(&control), "a stop after the run ended still stands");
        assert_eq!(control.after_run(PowerState::Running), AfterRun::End);
    }

    /// A dump being written still counts as pending, so the page keeps
    /// saying so and offers no second dump until it is done.
    #[test]
    fn a_dump_being_written_is_still_pending() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        let path = PathBuf::from("memory.bin");
        assert_eq!(control.request_dump(path.clone()), DumpRequest::Queued);
        assert_eq!(control.after_run(PowerState::Running), AfterRun::Serve);
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::DumpRam(path.clone())));
        assert!(control.dump_pending(), "taken off the queue, but not written yet");
        control.record_dump(DumpOutcome::Saved { path, bytes: 1 });
        assert!(!control.dump_pending());
    }

    fn empty_snapshot(rip: u64) -> crate::debugger::inspect::DebugSnapshot {
        use crate::debugger::inspect::{
            AddressSpace, DebugSnapshot, GuestAddress, MemoryWindow, SymbolIndex,
        };
        DebugSnapshot {
            cpus: Vec::new(),
            cpu: 0,
            code: Vec::new(),
            code_at: CodeTarget::Rip,
            rip,
            frames: crate::debugger::frames::FramesRead::default(),
            memory: MemoryWindow {
                start: GuestAddress { space: AddressSpace::Linear, value: rip },
                rows: Vec::new(),
            },
            stack: Vec::new(),
            process: None,
            asked: crate::debugger::windows::SystemView::Off,
            system: crate::debugger::windows::SystemListing::NotAsked,
            symbols: SymbolIndex::default(),
            memory_space: None,
            context: crate::debugger::inspect::ViewContext::Live,
            viewing: crate::debugger::inspect::Viewing::Live,
            layout: None,
            layouts: None,
            through: crate::debugger::windows::ReadThrough::ProcessorTables,
            vmcs: None,
            elsewhere: crate::debugger::windows::ModulesElsewhere::default(),
        }
    }

    /// New layouts make a paused run read the guest again, once; the same
    /// layouts handed over again ask for nothing.
    #[test]
    fn new_kernel_layouts_make_a_paused_run_read_again() {
        use crate::debugger::kernel_layout::KernelLayout;
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        let layout = Arc::new(KernelLayout {
            kernel: crate::symbols::kernel::tests::kernel_identity(),
            source: crate::debugger::kernel_layout::LayoutSource::Matched,
            globals: Default::default(),
            process: Default::default(),
            thread: Default::default(),
            vad: None,
            object_era: None,
            failures: Vec::new(),
        });
        control.set_kernel_layout(Some(Arc::clone(&layout)));
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.set_kernel_layout(Some(Arc::clone(&layout)));
        assert!(control.lock().tasks.is_empty());
        assert!(control.kernel_layout().is_some_and(|held| Arc::ptr_eq(&held, &layout)));
        control.set_kernel_layout(None);
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
    }

    /// Each snapshot published after the guest moved keeps the one before
    /// it, which the page compares against to draw what a step changed in red.
    #[test]
    fn a_published_snapshot_keeps_the_one_before_it() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.publish(empty_snapshot(1));
        control.publish(empty_snapshot(2));
        let previous = control.previous_snapshot().expect("kept");
        assert_eq!(previous.memory.start.value, 1);
        assert_eq!(control.snapshot().expect("shown").memory.start.value, 2);
    }

    /// A step, then a refresh that only changes the view: what the step
    /// changed is still compared against the state before the step.
    #[test]
    fn a_view_change_keeps_the_comparison_a_step_made() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.publish_view(empty_snapshot(1));
        control.step(StepRequest::instruction_into(0));
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Step(StepRequest::instruction_into(0))));
        control.publish(empty_snapshot(2));
        assert_eq!(control.previous_snapshot().expect("kept").memory.start.value, 1);

        // Paging memory re-reads the same guest: the comparison stays.
        control.set_view(ViewRequest {
            memory: crate::debugger::inspect::MemoryTarget::Rsp,
            ..ViewRequest::default()
        });
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        assert!(control.unmoved_snapshot().is_some(), "nothing executed since");
        control.publish_view(empty_snapshot(3));
        assert_eq!(control.previous_snapshot().expect("kept").memory.start.value, 1);
        assert_eq!(control.snapshot().expect("shown").memory.start.value, 3);
    }

    /// A pause that follows a run is a new baseline, and its snapshot cannot
    /// carry the last pause's structure reads over.
    #[test]
    fn a_pause_after_a_run_is_a_new_comparison_and_carries_nothing() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.publish_view(empty_snapshot(1));
        assert!(control.unmoved_snapshot().is_some());
        control.resume();
        assert_eq!(control.next_between_runs(), BetweenRuns::Resume);
        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        assert!(control.unmoved_snapshot().is_none(), "the guest ran between the pauses");
        control.publish_view(empty_snapshot(2));
        assert_eq!(control.previous_snapshot().expect("kept").memory.start.value, 1);
    }


    /// A search is taken only while paused, and a hit becomes the window.
    #[test]
    fn a_search_hit_becomes_the_memory_window() {
        use crate::debugger::inspect::{AddressSpace, GuestAddress};
        use crate::debugger::search::{FindOutcome, FindRequest, FoundAt};
        let control = control();
        control.begin_run(RunStart::PowerOn);
        let request = FindRequest {
            from: GuestAddress { space: AddressSpace::Physical, value: 0 },
            pattern: b"XP".to_vec(),
        };
        control.find(request.clone());
        assert!(!control.searching(), "a running VM takes no search");
        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.find(request.clone());
        assert!(control.searching());
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Find(request)));
        let at = GuestAddress { space: AddressSpace::Physical, value: 0x1234 };
        control.record_find(FindOutcome::Found(FoundAt { at, len: 2 }));
        assert!(!control.searching());
        assert_eq!(control.view().memory, crate::debugger::inspect::MemoryTarget::At(at));
    }

    #[test]
    fn a_run_that_spent_its_budget_with_nothing_asked_ends() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        assert_eq!(control.after_run(PowerState::Running), AfterRun::End);
    }

    #[test]
    fn a_step_is_queued_only_while_paused() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.step(StepRequest::instruction_into(0));
        assert_eq!(control.after_run(PowerState::Running), AfterRun::End, "no step queued");
        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.step(StepRequest::instruction_into(0));
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Step(StepRequest::instruction_into(0))));
    }

    #[test]
    fn an_unchanged_view_queues_no_refresh() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.set_view(ViewRequest::default());
        assert!(control.lock().tasks.is_empty());
        control.set_view(ViewRequest {
            memory: crate::debugger::inspect::MemoryTarget::Rsp,
            ..ViewRequest::default()
        });
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
    }

    /// A paused run thread sleeps on the condition variable; a stop wakes it.
    #[test]
    fn a_paused_thread_wakes_for_a_stop() {
        let control = Arc::new(control());
        control.begin_run(RunStart::PowerOn);
        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        let waiter = {
            let control = Arc::clone(&control);
            std::thread::spawn(move || control.next_between_runs())
        };
        control.end_run();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !waiter.is_finished() {
            assert!(std::time::Instant::now() < deadline, "the paused thread never woke");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(waiter.join().expect("the waiter"), BetweenRuns::End);
    }

    /// Breakpoints changed while the run runs stop it for the change, which
    /// is served alone (only the newest set), and the run goes on after.
    #[test]
    fn breakpoints_changed_while_running_are_given_to_the_machine_and_it_runs_on() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        let first = vec![CodeBreakpoint::Linear(0x1000)];
        let newest = vec![CodeBreakpoint::Linear(0x2000)];
        control.set_breakpoints(global(first));
        control.set_breakpoints(global(newest.clone()));
        assert!(raised(&control), "the run stops for the change");
        assert_eq!(control.after_run(PowerState::Running), AfterRun::Serve);
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::ApplyBreakpoints(newest.clone())));
        assert_eq!(control.next_between_runs(), BetweenRuns::Resume);
        assert!(!raised(&control), "and runs on");
        assert_eq!(control.breakpoints(), newest, "kept for the next run");
        control.set_breakpoints(global(newest));
        assert!(!raised(&control), "an unchanged set asks nothing");
    }

    /// A stop at a breakpoint that was not the owner's runs on, unless the
    /// owner asked for something meanwhile or the guest switched off.
    #[test]
    fn a_breakpoint_passed_over_runs_on_unless_something_was_asked() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        // The machine's hit raised the run's stop flag; running on lowers it.
        control.stop_flag().store(true, Ordering::Relaxed);
        assert_eq!(control.after_passing(PowerState::Running), Passing::RunOn);
        assert!(!raised(&control), "the run goes on");
        assert_eq!(control.after_passing(PowerState::PoweredOff), Passing::Then(AfterRun::End));
        control.pause();
        assert_eq!(control.after_passing(PowerState::Running), Passing::Then(AfterRun::Serve));
        control.end_run();
        assert_eq!(control.after_passing(PowerState::Running), Passing::Then(AfterRun::End));
    }

    fn global(set: Vec<CodeBreakpoint>) -> AppliedBreakpoints {
        AppliedBreakpoints { set, scopes: Vec::new() }
    }

    /// A breakpoint on a user program's line is the owner's only in the
    /// processes that run it; a new scope alone asks nothing of the machine.
    #[test]
    fn a_hit_outside_its_breakpoints_address_spaces_is_not_the_owners() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        let set = vec![CodeBreakpoint::Linear(0x1000), CodeBreakpoint::Linear(0x2000)];
        control.set_breakpoints(global(set.clone()));
        assert_eq!(control.after_run(PowerState::Running), AfterRun::Serve);
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::ApplyBreakpoints(set.clone())));
        assert_eq!(control.next_between_runs(), BetweenRuns::Resume);
        let scopes = vec![ScopedAddress { linear: 0x1000, spaces: vec![0x5000] }];
        control.set_breakpoints(AppliedBreakpoints { set, scopes });
        assert!(!raised(&control), "the machine's set did not change");
        let hit = |linear| CodeBreakpointHit { cpu: 0, breakpoint: CodeBreakpoint::Linear(linear), linear };
        assert!(control.admits(&hit(0x1000), 0x5000));
        assert!(!control.admits(&hit(0x1000), 0x6000), "another program at the same address");
        assert!(control.admits(&hit(0x2000), 0x6000), "an address with no scope");
    }

    /// A breakpoint stop pauses the run on the processor that stopped; a
    /// resume runs on, and the hit is no longer shown.
    #[test]
    fn a_breakpoint_stop_pauses_the_run_on_its_processor() {
        let control = control();
        control.begin_run(RunStart::PowerOn);
        let hit = CodeBreakpointHit {
            cpu: 1,
            breakpoint: CodeBreakpoint::Linear(0x1000),
            linear: 0x1000,
        };
        control.stopped_at_breakpoint(hit);
        assert_eq!(control.after_run(PowerState::Running), AfterRun::Serve);
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        assert_eq!(control.phase(), RunPhase::Paused);
        assert_eq!(control.view().cpu, 1, "the stopped processor is shown");
        assert_eq!(control.stopped_at(), Some(hit));
        control.step(StepRequest::instruction_into(0));
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Step(StepRequest::instruction_into(0))));
        assert_eq!(control.stopped_at(), None, "a step moves off the breakpoint");
        control.stepped_onto_breakpoint(hit);
        control.resume();
        assert_eq!(control.stopped_at(), None);
        assert_eq!(control.next_between_runs(), BetweenRuns::Resume);
    }
}
