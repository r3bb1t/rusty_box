use std::cell::RefCell;
use std::rc::Rc;

use rusty_box::cpu::instrumentation::{IoPort, MemAccessRW};

use super::*;

fn write_com1(tracer: &mut impl Instrumentation, bytes: &[u8]) {
    for &value in bytes {
        tracer.outp(&IoHookEvent {
            port: IoPort::new(0x3F8),
            size: 1,
            value: u32::from(value),
            access: MemAccessRW::Write,
        });
    }
}

#[test]
fn serial_write_delivers_the_report_and_shares_extraction() {
    let observed = Rc::new(RefCell::new(Vec::new()));
    let observations = observed.clone();
    let app = App::fallible()
        .route(move |report: &ExecReported, process: &LoadedProcess| {
            observations
                .borrow_mut()
                .push((report.path.clone(), process.pid, process.entry));
        })
        .route(|_: &ExecReported, _: &LoadedProcess| {})
        .with_state(());
    let mut tracer = SerialReports::new(app);

    write_com1(
        &mut tracer,
        b"RB_EXEC 0000000c 08049000 00000000 00000000 00000000 00000000\n",
    );
    assert!(observed.borrow().is_empty());
    write_com1(&mut tracer, b"RB_EXE /bin/program_sum\n");

    assert_eq!(
        *observed.borrow(),
        [("/bin/program_sum".to_owned(), 12, 0x08049000)]
    );
    assert_eq!(tracer.extractions, 1);
    assert_eq!(tracer.delivery, Ok(ExecutionRequest::None));
}

#[test]
fn event_only_handler_skips_parsing_process_metadata() {
    let app = App::fallible().route(|_: &ExecReported| {}).with_state(());
    let mut tracer = SerialReports::new(app);

    write_com1(
        &mut tracer,
        b"RB_EXEC bad metadata\nRB_EXE /bin/program_sum\n",
    );

    assert_eq!(tracer.execs, 1);
    assert_eq!(tracer.extractions, 0);
    assert_eq!(tracer.delivery, Ok(ExecutionRequest::None));
}

#[test]
fn invalid_process_metadata_stops_delivery_when_requested() {
    fn unexpected_extraction(_: &ExecReported, _: &LoadedProcess) {
        panic!("handler received invalid process metadata");
    }

    let app = App::fallible().route(unexpected_extraction).with_state(());
    let mut tracer = SerialReports::new(app);

    write_com1(
        &mut tracer,
        b"RB_EXEC bad metadata\nRB_EXE /bin/program_sum\n",
    );

    assert_eq!(tracer.extractions, 1);
    assert_eq!(tracer.delivery, Err(ReportError::InvalidExec));
}
