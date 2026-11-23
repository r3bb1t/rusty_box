//! Select a guest, run it, and resume it through typed routes.
//! Start with `main` and `routes`. Register functions or closures as handlers.
//! The example implements `Emit` only for the budget wrapper.

use std::cell::OnceCell;

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::memory_session::SessionStop;
use rusty_box::cpu::registers::{EAX, EIP};
use rusty_box::cpu::{CpuEmulator, ProcessorActivity};
use rusty_box::events::{App, AppHandlers, Emit, ExecutionRequest, Subscriptions};

const COUNTER: ProcessId = ProcessId {
    pid: 12,
    generation: 1,
};
const WORKER: ProcessId = ProcessId {
    pid: 21,
    generation: 1,
};
const MAX_BUDGET: u64 = 16;
const ENTRY: u32 = 0x1000;

fn main() -> Result<(), WorkflowError> {
    let mut app = routes().with_state(State::demo()?);
    let run = RunRequested {
        target: Target::Selected,
        budget: 1,
    };

    assert_eq!(
        app.dispatch(&SelectProcess(COUNTER))?,
        ExecutionRequest::None
    );
    assert_eq!(app.dispatch(&run)?, ExecutionRequest::None);
    assert_eq!(
        app.dispatch(&SelectProcess(WORKER))?,
        ExecutionRequest::None
    );
    assert_eq!(app.dispatch(&run)?, ExecutionRequest::None);

    // The counter resumes at HLT while the worker remains selected.
    let resume = RunRequested {
        target: Target::Exact(COUNTER),
        ..run
    };
    assert_eq!(app.dispatch(&resume)?, ExecutionRequest::Stop);
    let denied = RunRequested {
        budget: MAX_BUDGET + 1,
        ..resume
    };
    assert!(matches!(
        app.dispatch(&denied),
        Err(WorkflowError::Budget(_))
    ));

    let state = app.into_state();
    for record in &state.audit {
        if let Some(guest) = &record.observed {
            print!(
                "{} ({}@{}) eip={:#x}, eax={}, ",
                guest.name, guest.id.pid, guest.id.generation, guest.eip, guest.eax
            );
        }
        match &record.outcome {
            Ok(request) => println!("{request:?}"),
            Err(error) => println!("run failed: {error}"),
        }
    }
    println!(
        "{} target lookups for {} run requests",
        state.lookups,
        state.audit.len()
    );
    Ok(())
}

// AppHandlers describes the chain that App::on builds.
fn routes() -> App<State, WorkflowError, impl AppHandlers<State, WorkflowError>> {
    let handlers = Subscriptions::<RunRequested, WorkflowError>::new()
        .on_with(execute)
        .on_with(observe_registers);
    let mut guarded = BudgetLimit(handlers);

    let run_process = move |event: &RunRequested, state: &mut State| {
        let mut parts = RunParts::new(state, event.target);
        let outcome = guarded.emit(event, &mut parts);
        parts.state.audit.push(RunRecord {
            observed: parts.observed,
            outcome: outcome.as_ref().copied().map_err(ToString::to_string),
        });
        outcome
    };

    App::<State, WorkflowError>::new()
        .on(select_process)
        .on(run_process)
}

struct SelectProcess(ProcessId);

struct RunRequested {
    target: Target,
    budget: u64,
}

fn select_process(
    event: &SelectProcess,
    state: &mut State,
) -> Result<ExecutionRequest, WorkflowError> {
    state.find(event.0)?;
    state.selected = Some(event.0);
    Ok(ExecutionRequest::None)
}

fn execute(
    event: &RunRequested,
    parts: &mut RunParts<'_>,
) -> Result<ExecutionRequest, WorkflowError> {
    let process = parts.target_process()?;
    match process.cpu.run(event.budget).stop {
        SessionStop::Budget => Ok(ExecutionRequest::None),
        SessionStop::Inactive(ProcessorActivity::Halted) => Ok(ExecutionRequest::Stop),
        stop => Err(WorkflowError::UnexpectedStop(stop)),
    }
}

// Subscriptions still calls the observer when execute requests Stop.
fn observe_registers(
    _: &RunRequested,
    parts: &mut RunParts<'_>,
) -> Result<ExecutionRequest, WorkflowError> {
    let process = parts.target_process()?;
    let regs = process.cpu.regs();
    parts.observed = Some(Observation {
        id: process.id,
        name: process.name,
        eip: regs.read(EIP),
        eax: regs.read(EAX),
    });
    Ok(ExecutionRequest::None)
}

// This wrapper owns the next sink and rejects invalid budgets before forwarding.
struct BudgetLimit<S>(S);

// The event is RunRequested and the data lent for this call is RunParts.
// The inner sink accepts the same inputs and has the same error type.
impl<'a, S> Emit<RunRequested, RunParts<'a>> for BudgetLimit<S>
where
    S: Emit<RunRequested, RunParts<'a>, Error = WorkflowError>,
{
    type Error = WorkflowError;

    fn emit(
        &mut self,
        event: &RunRequested,
        parts: &mut RunParts<'a>,
    ) -> Result<ExecutionRequest, Self::Error> {
        if !(1..=MAX_BUDGET).contains(&event.budget) {
            return Err(WorkflowError::Budget(event.budget));
        }
        self.0.emit(event, parts)
    }
}

// A fresh delivery owns its lookup cache. Each handler gets a short guest loan.
struct RunParts<'a> {
    state: &'a mut State,
    target: Target,
    resolved: OnceCell<Result<usize, TargetError>>,
    observed: Option<Observation>,
}

impl<'a> RunParts<'a> {
    fn new(state: &'a mut State, target: Target) -> Self {
        Self {
            state,
            target,
            resolved: OnceCell::new(),
            observed: None,
        }
    }

    // A local FromRequestParts analogue. Only the slot or error is cached.
    fn target_process(&mut self) -> Result<&mut Process, TargetError> {
        let slot = *self
            .resolved
            .get_or_init(|| self.state.resolve(self.target));
        // The delivery owns the exclusive state loan and handlers can only edit the guest.
        Ok(&mut self.state.processes[slot?])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProcessId {
    pid: u32,
    generation: u32,
}

#[derive(Clone, Copy)]
enum Target {
    Selected,
    Exact(ProcessId),
}

struct State {
    processes: Vec<Process>,
    selected: Option<ProcessId>,
    audit: Vec<RunRecord>,
    lookups: usize,
}

impl State {
    fn demo() -> rusty_box::Result<Self> {
        Ok(Self {
            processes: vec![
                Process::new(COUNTER, "counter", 42)?,
                Process::new(WORKER, "worker", 7)?,
            ],
            selected: None,
            audit: Vec::new(),
            lookups: 0,
        })
    }

    fn find(&self, id: ProcessId) -> Result<usize, TargetError> {
        self.processes
            .iter()
            .position(|process| process.id == id)
            .ok_or(TargetError::Stale(id))
    }

    fn resolve(&mut self, target: Target) -> Result<usize, TargetError> {
        self.lookups += 1;
        let id = match target {
            Target::Selected => self.selected.ok_or(TargetError::NoSelection)?,
            Target::Exact(id) => id,
        };
        self.find(id)
    }
}

struct Process {
    id: ProcessId,
    name: &'static str,
    cpu: CpuEmulator,
}

impl Process {
    fn new(id: ProcessId, name: &'static str, eax: u32) -> rusty_box::Result<Self> {
        let mut cpu = CpuEmulator::builder().build()?;
        let [a, b, c, d] = eax.to_le_bytes();
        // mov eax, imm32; hlt
        cpu.memory().write_phys(
            BxPhyAddress::new(u64::from(ENTRY)),
            &[0xB8, a, b, c, d, 0xF4],
        )?;
        Ok(Self { id, name, cpu })
    }
}

struct Observation {
    id: ProcessId,
    name: &'static str,
    eip: u32,
    eax: u32,
}

struct RunRecord {
    observed: Option<Observation>,
    outcome: Result<ExecutionRequest, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
enum TargetError {
    #[error("select a process before running the current target")]
    NoSelection,
    #[error("process {0:?} is missing or has been replaced")]
    Stale(ProcessId),
}

#[derive(Debug, thiserror::Error)]
enum WorkflowError {
    #[error("could not initialize the guest: {0}")]
    Setup(#[from] rusty_box::Error),
    #[error(transparent)]
    Target(#[from] TargetError),
    #[error("dispatch budget {0} must be in 1..={MAX_BUDGET}")]
    Budget(u64),
    #[error("guest stopped at an unexpected boundary: {0:?}")]
    UnexpectedStop(SessionStop),
}

#[cfg(test)]
mod tests {
    use super::*;

    const SELECTED: RunRequested = RunRequested {
        target: Target::Selected,
        budget: 1,
    };

    #[test]
    fn selected_guest_changes_and_explicit_target_resumes() {
        let mut app = routes().with_state(State::demo().unwrap());
        assert_eq!(
            app.dispatch(&SelectProcess(COUNTER)).unwrap(),
            ExecutionRequest::None
        );
        assert_eq!(app.state().lookups, 0);
        assert_eq!(app.dispatch(&SELECTED).unwrap(), ExecutionRequest::None);
        assert_eq!(
            app.dispatch(&SelectProcess(WORKER)).unwrap(),
            ExecutionRequest::None
        );
        assert_eq!(app.dispatch(&SELECTED).unwrap(), ExecutionRequest::None);
        let resume = RunRequested {
            target: Target::Exact(COUNTER),
            ..SELECTED
        };
        assert_eq!(app.dispatch(&resume).unwrap(), ExecutionRequest::Stop);

        let state = app.into_state();
        assert_eq!(state.selected, Some(WORKER));
        assert_eq!(state.lookups, 3);
        let observations: Vec<_> = state
            .audit
            .iter()
            .map(|record| {
                let guest = record.observed.as_ref().unwrap();
                (guest.id, guest.eip, guest.eax)
            })
            .collect();
        assert_eq!(
            observations,
            [
                (COUNTER, ENTRY + 5, 42),
                (WORKER, ENTRY + 5, 7),
                (COUNTER, ENTRY + 6, 42)
            ]
        );
        assert_eq!(state.audit[2].outcome, Ok(ExecutionRequest::Stop));
    }

    #[test]
    fn guard_skips_resolution_and_old_process_identity_is_refused() {
        let mut state = State::demo().unwrap();
        let replacement = ProcessId {
            generation: 2,
            ..COUNTER
        };
        state.processes[0] = Process::new(replacement, "replacement", 99).unwrap();
        let mut app = routes().with_state(state);

        assert!(matches!(
            app.dispatch(&RunRequested {
                budget: 0,
                ..SELECTED
            }),
            Err(WorkflowError::Budget(0))
        ));
        assert_eq!(app.state().lookups, 0);

        let exact = |id| RunRequested {
            target: Target::Exact(id),
            ..SELECTED
        };
        assert!(matches!(
            app.dispatch(&exact(COUNTER)),
            Err(WorkflowError::Target(TargetError::Stale(id))) if id == COUNTER
        ));
        assert_eq!(
            app.dispatch(&exact(replacement)).unwrap(),
            ExecutionRequest::None
        );

        let state = app.into_state();
        assert_eq!(state.lookups, 2);
        assert!(
            state.audit[..2]
                .iter()
                .all(|record| record.outcome.is_err() && record.observed.is_none())
        );
        assert_eq!(state.audit[2].observed.as_ref().unwrap().eax, 99);
    }

    #[test]
    fn failed_lookup_is_shared_for_this_delivery() {
        let mut state = State::demo().unwrap();
        let mut parts = RunParts::new(&mut state, Target::Selected);
        assert_eq!(parts.target_process().err(), Some(TargetError::NoSelection));
        assert_eq!(parts.target_process().err(), Some(TargetError::NoSelection));
        assert_eq!(state.lookups, 1);
    }
}
