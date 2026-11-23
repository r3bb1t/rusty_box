use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rusty_box::cpu::registers::EAX;

use super::*;

#[test]
fn one_run_resolves_both_loaded_code_ranges_and_shares_each_lookup() {
    let observed = Rc::new(RefCell::new(Vec::new()));
    let observations = observed.clone();
    let app = App::<Catalog, UnknownProcess>::new()
        .on(
            move |event: &InstructionCompleted, Resolved(process): Resolved<'_, LoadedProcess>| {
                observations.borrow_mut().push((event.ip, process.id));
            },
        )
        .on(|_: &InstructionCompleted, Resolved(_): Resolved<'_, LoadedProcess>| {})
        .with_state(Catalog::new());
    let mut cpu = load_cpu(app).unwrap();

    let run = cpu.run(16);

    assert!(matches!(
        run.stop,
        SessionStop::Inactive(ProcessorActivity::Halted)
    ));
    assert_eq!(cpu.regs().read(EAX), 42);
    assert_eq!(
        *observed.borrow(),
        [(0x1000, 12), (0x1005, 12), (0x2000, 21), (0x2001, 21)]
    );
    let tracer = cpu.instrumentation();
    assert_eq!(tracer.delivery, Ok(ExecutionRequest::None));
    assert_eq!(tracer.app.as_ref().unwrap().state().lookups, 4);
}

#[test]
fn event_only_handler_runs_without_resolving_processes() {
    let count = Rc::new(Cell::new(0));
    let calls = count.clone();
    let app = App::<Catalog, UnknownProcess>::new()
        .on(move |_: &InstructionCompleted| calls.set(calls.get() + 1))
        .with_state(Catalog::new());
    let mut cpu = load_cpu(app).unwrap();

    let run = cpu.run(16);

    assert_eq!(count.get(), 4);
    assert_eq!(run.instructions, 4);
    assert_eq!(
        cpu.instrumentation().app.as_ref().unwrap().state().lookups,
        0
    );
}

#[test]
fn missing_target_ends_publication_but_guest_can_finish() {
    let count = Rc::new(Cell::new(0));
    let calls = count.clone();
    let app = App::<Catalog, UnknownProcess>::new()
        .on(move |_: &InstructionCompleted| calls.set(calls.get() + 1))
        .on(print_process)
        .with_state(Catalog {
            processes: &[],
            lookups: 0,
        });
    let mut cpu = load_cpu(app).unwrap();

    let run = cpu.run(16);

    assert!(matches!(
        run.stop,
        SessionStop::Inactive(ProcessorActivity::Halted)
    ));
    assert_eq!(run.instructions, 4);
    assert_eq!(count.get(), 1);
    assert_eq!(cpu.instrumentation().delivery, Err(UnknownProcess(0x1000)));
}
