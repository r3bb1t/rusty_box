use std::error::Error;
use std::io::{self, Write};
use std::path::PathBuf;

use rusty_box::events::{App, Emit, ExecutionRequest};
use rusty_box_linux_user::{BufferedIo, Image, Launch, ProgramExited, ProgramStarted, Stop};

#[derive(Default)]
struct State {
    last_started: Option<ProgramStarted>,
    exit: Option<ProgramExited>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [compute, filter, input] = args.as_slice() else {
        return Err("usage: run COMPUTE_ELF FILTER_ELF INPUT_FILE".into());
    };
    let input = std::fs::read(input)?;
    let handlers = App::fallible()
        .route(|start: &ProgramStarted, state: &mut State| {
            state.last_started = Some(*start);
            state.exit = None;
        })
        .route(|exit: &ProgramExited, state: &mut State| {
            if state.last_started.take().is_none() {
                return Err("exit without a matching program start");
            }
            state.exit = Some(*exit);
            Ok(ExecutionRequest::None)
        });
    let mut app = handlers.with_state(State::default());
    for (path, args, stdin) in [
        (compute, [b"compute".as_slice(), b"7"], &[][..]),
        (filter, [b"filter".as_slice(), b""], input.as_slice()),
    ] {
        let bytes = std::fs::read(path)?;
        let image = Image::parse(&bytes)?;
        let mut process = image.launch(
            Launch {
                args: &args,
                env: &[b"MODE=fixture"],
            },
            BufferedIo::new(stdin),
        )?;
        if app.emit(&process.started(), &mut ())? == ExecutionRequest::Stop {
            return Err("application requested a stop before execution".into());
        }
        let run = process.run(10_000_000)?;
        let Stop::Exited(exit) = run.stop else {
            return Err(format!("process stopped at {:?}", run.stop).into());
        };
        if exit.status != 0 {
            return Err(format!("{} exited with {}", path.display(), exit.status).into());
        }
        if app.emit(&exit, &mut ())? == ExecutionRequest::Stop {
            break;
        }
        println!(
            "{} exited with {} after {} ABI calls",
            path.display(),
            exit.status,
            run.syscalls
        );
        let streams = process.into_io();
        io::stdout().write_all(streams.stdout())?;
        io::stderr().write_all(streams.stderr())?;
    }
    let state = app.into_state();
    if state.last_started.is_some() || state.exit.is_none() {
        return Err("incomplete application lifecycle".into());
    }
    Ok(())
}
