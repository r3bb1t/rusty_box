//! Route commands to application-owned x86 guests with scoped target lookup.
mod process;
mod run;

use process::{Catalog, Process, ProcessId, SelectProcess, Target};
use run::{Audit, BudgetLimit, RunParts, RunRecord, RunRequested, with_target};
use rusty_box::cpu::memory_session::SessionStop;
use rusty_box::events::{App, AppHandlers, Emit, ExecutionRequest, Subscriptions};

const COUNTER: ProcessId = ProcessId::new(12, 1);
const WORKER: ProcessId = ProcessId::new(21, 1);
const MAX_BUDGET: u64 = 16;

#[derive(Debug, thiserror::Error)]
enum WorkflowError {
    #[error("could not initialize the guest: {0}")]
    Setup(#[from] rusty_box::Error),
    #[error(transparent)]
    Target(#[from] process::TargetError),
    #[error("dispatch budget {budget} must be in 1..={maximum}")]
    Budget { budget: u64, maximum: u64 },
    #[error("guest stopped at an unexpected boundary: {0:?}")]
    UnexpectedStop(SessionStop),
}

struct State {
    catalog: Catalog,
    runs: Vec<RunRecord>,
}

fn compose() -> App<State, WorkflowError, impl AppHandlers<State, WorkflowError>> {
    let handlers = Subscriptions::<RunRequested, WorkflowError>::new()
        .on_with(with_target(run::observe_before))
        .on_with(with_target(run::execute))
        .on_with(with_target(run::observe_after));
    let mut run = Audit(BudgetLimit {
        maximum: MAX_BUDGET,
        inner: handlers,
    });

    let app = App::<State, WorkflowError>::new();
    process::routes(app, |state| &mut state.catalog).on(
        move |event: &RunRequested, state: &mut State| {
            run.emit(event, &mut RunParts::new(state, event))
        },
    )
}

fn demo_state() -> rusty_box::Result<State> {
    let mut catalog = Catalog::default();
    catalog.insert(Process::new(COUNTER, "counter", 42)?);
    catalog.insert(Process::new(WORKER, "worker", 7)?);
    Ok(State {
        catalog,
        runs: Vec::new(),
    })
}

fn main() -> Result<(), WorkflowError> {
    let mut app = compose().with_state(demo_state()?);
    assert_eq!(
        app.dispatch(&SelectProcess(COUNTER))?,
        ExecutionRequest::None
    );
    let run_selected = RunRequested {
        target: Target::Selected,
        budget: 1,
    };
    assert_eq!(app.dispatch(&run_selected)?, ExecutionRequest::None);
    // The same route resolves the new selection at delivery time.
    assert_eq!(
        app.dispatch(&SelectProcess(WORKER))?,
        ExecutionRequest::None
    );
    assert_eq!(app.dispatch(&run_selected)?, ExecutionRequest::None);
    // An explicit identity can run a different guest without changing the selection.
    let resume_counter = RunRequested {
        target: Target::Exact(COUNTER),
        budget: 1,
    };
    assert_eq!(app.dispatch(&resume_counter)?, ExecutionRequest::Stop);
    let denied = RunRequested {
        target: Target::Exact(COUNTER),
        budget: MAX_BUDGET + 1,
    };
    assert!(matches!(
        app.dispatch(&denied),
        Err(WorkflowError::Budget { .. })
    ));

    let state = app.into_state();
    for record in &state.runs {
        println!("{record}");
    }
    println!(
        "{} target resolutions for {} requests",
        state.catalog.resolutions(),
        state.runs.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use process::TargetError;
    use run::{GuestRun, Outcome};

    use super::*;

    fn run_selected(budget: u64) -> RunRequested {
        RunRequested {
            target: Target::Selected,
            budget,
        }
    }

    #[test]
    fn routes_select_resume_and_honor_stop_after_all_handlers() {
        let mut app = compose().with_state(demo_state().unwrap());
        assert_eq!(
            app.dispatch(&SelectProcess(COUNTER)).unwrap(),
            ExecutionRequest::None
        );
        assert_eq!(app.state().catalog.resolutions(), 0);
        assert_eq!(
            app.dispatch(&run_selected(1)).unwrap(),
            ExecutionRequest::None
        );
        assert_eq!(
            app.dispatch(&run_selected(1)).unwrap(),
            ExecutionRequest::Stop
        );
        assert_eq!(
            app.dispatch(&SelectProcess(WORKER)).unwrap(),
            ExecutionRequest::None
        );
        assert_eq!(
            app.dispatch(&run_selected(1)).unwrap(),
            ExecutionRequest::None
        );
        let state = app.into_state();
        assert_eq!(state.catalog.resolutions(), 3);
        let records = &state.runs;
        assert_eq!(records[0].target, Some(COUNTER));
        assert_eq!(
            records[0].guest,
            Some(GuestRun {
                name: "counter",
                ip_before: 0x1000,
                eax: 42,
                instructions: 1,
            })
        );
        assert_eq!(
            records[1].outcome,
            Outcome::Delivered(ExecutionRequest::Stop)
        );
        assert_eq!(
            records[1].guest,
            Some(GuestRun {
                name: "counter",
                ip_before: 0x1005,
                eax: 42,
                instructions: 1,
            })
        );
        assert_eq!(records[2].target, Some(WORKER));
        assert_eq!(records[2].guest.unwrap().eax, 7);
    }

    #[test]
    fn budget_rejection_precedes_lookup_and_stale_identity_is_refused() {
        let mut state = demo_state().unwrap();
        let replacement = ProcessId::new(12, 2);
        state
            .catalog
            .insert(Process::new(replacement, "replacement", 99).unwrap());
        let mut app = compose().with_state(state);
        assert!(matches!(
            app.dispatch(&run_selected(0)),
            Err(WorkflowError::Budget { .. })
        ));
        assert_eq!(app.state().catalog.resolutions(), 0);

        let exact = |target| RunRequested {
            target: Target::Exact(target),
            budget: 1,
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
        assert_eq!(state.catalog.resolutions(), 2);
        assert_eq!(state.runs[0].outcome, Outcome::Failed);
        assert_eq!(state.runs[0].target, None);
        assert_eq!(state.runs[1].outcome, Outcome::Failed);
        assert_eq!(state.runs[1].guest, None);
        assert_eq!(state.runs[2].guest.unwrap().eax, 99);
    }
}
