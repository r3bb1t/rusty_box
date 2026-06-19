use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    const EMULATOR_STACK_SIZE: usize = 1500 * 1024 * 1024;

    let args = rusty_box_gui::Args::parse();
    let thread = match std::thread::Builder::new()
        .name("rusty_box_gui".to_owned())
        .stack_size(EMULATOR_STACK_SIZE)
        .spawn(move || rusty_box_gui::run(args))
    {
        Ok(thread) => thread,
        Err(error) => {
            eprintln!("rusty_box_gui: failed to start emulator thread: {error}");
            return ExitCode::FAILURE;
        }
    };

    match thread.join() {
        Ok(Ok(summary)) => {
            println!(
                "rusty_box_gui: executed {} instructions",
                summary.instructions_executed
            );
            ExitCode::SUCCESS
        }
        Ok(Err(error)) => {
            eprintln!("rusty_box_gui: {error}");
            ExitCode::FAILURE
        }
        Err(_) => {
            eprintln!("rusty_box_gui: emulator thread panicked");
            ExitCode::FAILURE
        }
    }
}
