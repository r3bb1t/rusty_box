//! The one channel between the shell and a VM's run thread for pausing,
//! resuming, stepping, reading and saving RAM. Every write of the run's stop
//! flag goes through here, under one lock (R5), so a stop the shell asks for
//! is never taken for a pause, and a resume never lowers a stop.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use rusty_box::emulator::PowerState;

use super::inspect::{DebugSnapshot, ViewRequest};

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
    /// Retire one instruction.
    Step,
    /// Write the machine's RAM to this file.
    DumpRam(PathBuf),
}

/// What a run thread does once its machine handed control back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AfterRun {
    End,
    Serve,
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
    note: Option<DebugNote>,
    dump_outcomes: Vec<DumpOutcome>,
}

/// One VM's run control, shared by the shell and the VM's run thread.
pub(crate) struct RunControl {
    stop_flag: Arc<AtomicBool>,
    state: Mutex<ControlState>,
    changed: Condvar,
}

impl RunControl {
    pub(crate) fn new(stop_flag: Arc<AtomicBool>) -> Self {
        Self {
            stop_flag,
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
                note: None,
                dump_outcomes: Vec::new(),
            }),
            changed: Condvar::new(),
        }
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
        self.changed.notify_all();
    }

    /// Asks for one instruction; only a paused run takes it.
    #[cfg(feature = "gui-egui")]
    pub(crate) fn step(&self) {
        let mut state = self.lock();
        if state.phase == RunPhase::Paused && state.wanted == Wanted::Pause && !state.ending {
            state.tasks.push_back(Task::Step);
            self.changed.notify_all();
        }
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

    #[cfg(feature = "gui-egui")]
    pub(crate) fn note(&self) -> Option<DebugNote> {
        self.lock().note
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
                if matches!(task, Task::DumpRam(_)) {
                    state.dump_in_flight = true;
                }
                return BetweenRuns::Perform(task);
            }
            match state.wanted {
                Wanted::Run => {
                    self.stop_flag.store(false, Ordering::Relaxed);
                    state.serve_requested = false;
                    state.phase = RunPhase::Running;
                    return BetweenRuns::Resume;
                }
                Wanted::Pause => {
                    if state.phase == RunPhase::Running {
                        state.phase = RunPhase::Paused;
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

    pub(crate) fn publish(&self, snapshot: DebugSnapshot) {
        self.lock().snapshot = Some(Arc::new(snapshot));
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
                Task::Refresh | Task::Step => {}
            }
        }
    }
}

#[cfg(all(test, feature = "gui-egui"))]
mod tests {
    use super::*;

    fn control() -> RunControl {
        RunControl::new(Arc::new(AtomicBool::new(false)))
    }

    fn raised(control: &RunControl) -> bool {
        control.stop_flag().load(Ordering::Relaxed)
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
            memory: crate::debugger::inspect::MemoryWindow {
                start: crate::debugger::inspect::GuestAddress {
                    space: crate::debugger::inspect::AddressSpace::Linear,
                    value: 0,
                },
                rows: Vec::new(),
            },
            stack: Vec::new(),
        });
        control.finish_run();
        control.begin_run(RunStart::PowerOn);
        assert!(control.snapshot().is_none());
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
        control.step();
        assert_eq!(control.after_run(PowerState::Running), AfterRun::End, "no step queued");
        control.pause();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Refresh));
        control.step();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Step));
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
            cpu: 0,
            memory: crate::debugger::inspect::MemoryTarget::Rsp,
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
}
