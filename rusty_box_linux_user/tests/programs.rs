use std::fs;
use std::path::PathBuf;

use rusty_box::events::{App, Emit, ExecutionRequest};
use rusty_box_linux_user::{
    BufferedIo, Image, Io, IoError, Launch, Output, ProgramExited, ProgramStarted, RunError, Stop,
};

struct ShortTransfers<'a>(BufferedIo<'a>);

impl Io for ShortTransfers<'_> {
    fn read_stdin(&mut self, buffer: &mut [u8]) -> Result<usize, IoError> {
        let cap = buffer.len().min(2);
        self.0.read_stdin(&mut buffer[..cap])
    }

    fn write(&mut self, stream: Output, buffer: &[u8]) -> Result<usize, IoError> {
        self.0.write(stream, &buffer[..buffer.len().min(1)])
    }
}

#[test]
#[ignore = "requires ELF fixtures built with tests/guest/build.sh"]
fn compiled_programs_run_repeatedly_with_caller_io_and_typed_events() {
    let fixtures = PathBuf::from(std::env::var_os("RB_USER_FIXTURES").expect("fixture directory"));
    let compute = fs::read(fixtures.join("compute")).unwrap();
    let filter = fs::read(fixtures.join("filter")).unwrap();
    let compute = Image::parse(&compute).unwrap();
    let filter = Image::parse(&filter).unwrap();

    #[derive(Default)]
    struct State {
        pending: bool,
        statuses: Vec<u8>,
        total: usize,
    }
    let handlers = App::<State, &'static str>::new()
        .on(|event: &ProgramStarted, state: &mut State| {
            assert_eq!(event.initial_stack % 16, 0);
            assert!(!state.pending);
            state.pending = true;
        })
        .on(|exit: &ProgramExited, state: &mut State| {
            if !state.pending {
                return Err("duplicate exit");
            }
            state.statuses.push(exit.status);
            state.pending = false;
            Ok(ExecutionRequest::None)
        })
        .on(|_: &ProgramExited, state: &mut State| state.total += 1);
    let mut app = handlers.with_state(State::default());

    for _ in 0..2 {
        let mut process = compute
            .launch(
                Launch {
                    args: &[b"compute", b"7"],
                    env: &[b"MODE=fixture"],
                },
                BufferedIo::new(&[]),
            )
            .unwrap();
        assert_eq!(
            app.emit(&process.started(), &mut ()),
            Ok(ExecutionRequest::None)
        );
        let run = process.run(50_000).unwrap();
        assert_eq!(run.syscalls, 1);
        let Stop::Exited(exit) = run.stop else {
            panic!("{run:?}");
        };
        assert_eq!(app.emit(&exit, &mut ()), Ok(ExecutionRequest::None));
        assert_eq!(exit, ProgramExited { status: 0 });
        assert!(matches!(process.run(1), Err(RunError::Finished)));
    }

    let mut prior = None;
    for (input, output, budget) in [
        (&b"abCxyz!"[..], &b"ABCXYZ!"[..], 13),
        (&b"abCxyz!"[..], &b"ABCXYZ!"[..], 13),
        (&b"different input"[..], &b"DIFFERENT INPUT"[..], 1),
    ] {
        let mut process = filter
            .launch(Launch::default(), ShortTransfers(BufferedIo::new(input)))
            .unwrap();
        assert_eq!(
            app.emit(&process.started(), &mut ()),
            Ok(ExecutionRequest::None)
        );
        let mut counts = (0, 0);
        let exit = (0..20_000)
            .find_map(|_| {
                let run = process.run(budget).unwrap();
                counts.0 += run.engine_count;
                counts.1 += run.syscalls;
                match run.stop {
                    Stop::Budget => None,
                    Stop::Exited(exit) => Some(exit),
                    stop => panic!("{stop:?}"),
                }
            })
            .expect("process exit within bounded slices");
        assert_eq!(app.emit(&exit, &mut ()), Ok(ExecutionRequest::None));
        assert_eq!(exit.status, 0);
        let streams = process.into_io().0;
        assert_eq!(streams.input_consumed(), input.len());
        assert_eq!(streams.stdout(), output);
        assert!(streams.stderr().is_empty());
        assert_eq!(
            counts.1,
            input.len() as u64 + input.len().div_ceil(2) as u64 + 2
        );
        if budget == 13 {
            if let Some(prev) = prior.replace(counts) {
                assert_eq!(counts, prev);
            }
        }
    }
    let recovered = app.into_state();
    assert_eq!(recovered.statuses, [0, 0, 0, 0, 0]);
    assert_eq!(recovered.total, 5);
    assert!(!recovered.pending);
}
