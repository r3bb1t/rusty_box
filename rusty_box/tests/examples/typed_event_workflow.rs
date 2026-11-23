use super::*;

const SELECTED: RunProcess = RunProcess {
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
    let resume = RunProcess {
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
        app.dispatch(&RunProcess {
            budget: 0,
            ..SELECTED
        }),
        Err(WorkflowError::Budget(0))
    ));
    assert_eq!(app.state().lookups, 0);

    let exact = |id| RunProcess {
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
