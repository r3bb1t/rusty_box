//! Select a guest, run it, and resume it through typed routes.
//! Start with `main` and `routes`. Register functions or closures as handlers.
//! The example implements `Emit` only for the budget wrapper.

use std::cell::OnceCell;

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::memory_session::SessionStop;
use rusty_box::cpu::registers::{EAX, EIP};
use rusty_box::cpu::{CpuEmulator, ProcessorActivity};
use rusty_box::events::{App, AppHandlers, Emit, ExecutionRequest};

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
    let run_selected = RunProcess {
        target: Target::Selected,
        budget: 1,
    };
    let resume_counter = RunProcess {
        target: Target::Exact(COUNTER),
        ..run_selected
    };

    let deliveries = [
        app.dispatch(&SelectProcess(COUNTER))?,
        app.dispatch(&run_selected)?,
        app.dispatch(&SelectProcess(WORKER))?,
        app.dispatch(&run_selected)?,
        // Resume the counter while the worker remains selected.
        app.dispatch(&resume_counter)?,
    ];
    println!("Execution requests: {deliveries:?}");

    let over_budget = RunProcess {
        budget: MAX_BUDGET + 1,
        ..resume_counter
    };
    match app.dispatch(&over_budget) {
        Err(WorkflowError::Budget(_)) => {} // The audit log prints this rejection.
        Err(error) => return Err(error),
        Ok(request) => println!("Over-budget delivery: {request:?}"),
    }

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
    let mut guarded = BudgetLimit { inner: run_guest };

    let run_process = move |event: &RunProcess, state: &mut State| {
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

struct RunProcess {
    target: Target,
    budget: u64,
}

fn run_guest(
    event: &RunProcess,
    parts: &mut RunParts<'_>,
) -> Result<ExecutionRequest, WorkflowError> {
    let request = execute(event, parts)?;
    // A stop still leaves the registers available to observe.
    observe_registers(parts)?;
    Ok(request)
}

fn select_process(
    &SelectProcess(id): &SelectProcess,
    state: &mut State,
) -> Result<ExecutionRequest, WorkflowError> {
    state.find(id)?;
    state.selected = Some(id);
    Ok(ExecutionRequest::None)
}

fn execute(
    &RunProcess { budget, .. }: &RunProcess,
    parts: &mut RunParts<'_>,
) -> Result<ExecutionRequest, WorkflowError> {
    let Process { cpu, .. } = parts.target_process()?;
    match cpu.run(budget).stop {
        SessionStop::Budget => Ok(ExecutionRequest::None),
        SessionStop::Inactive(ProcessorActivity::Halted) => Ok(ExecutionRequest::Stop),
        stop => Err(WorkflowError::UnexpectedStop(stop)),
    }
}

fn observe_registers(parts: &mut RunParts<'_>) -> Result<(), TargetError> {
    let process = parts.target_process()?;
    let regs = process.cpu.regs();
    parts.observed = Some(Observation {
        id: process.id,
        name: process.name,
        eip: regs.read(EIP),
        eax: regs.read(EAX),
    });
    Ok(())
}

// This wrapper owns the handler and checks the budget before calling it.
struct BudgetLimit<F> {
    inner: F,
}

// The event is RunProcess and the data lent for this call is RunParts.
impl<'a, F> Emit<RunProcess, RunParts<'a>> for BudgetLimit<F>
where
    F: FnMut(&RunProcess, &mut RunParts<'a>) -> Result<ExecutionRequest, WorkflowError>,
{
    type Error = WorkflowError;

    fn emit(
        &mut self,
        event: &RunProcess,
        parts: &mut RunParts<'a>,
    ) -> Result<ExecutionRequest, Self::Error> {
        if !(1..=MAX_BUDGET).contains(&event.budget) {
            return Err(WorkflowError::Budget(event.budget));
        }
        (self.inner)(event, parts)
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
#[path = "../tests/examples/typed_event_workflow.rs"]
mod tests;
