use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use rusty_box::emulator::StopReason;
use rusty_box::events::{App, ExecutionRequest};
use rusty_box_execution_workflows::linux::console::{Console, ConsoleResult};
use rusty_box_execution_workflows::linux::host::{
    ExecutableSample, Observation, ObservationError, Observer, ProcessSample, TaskIdentity,
};

#[derive(Default)]
struct State {
    processes: Vec<(TaskIdentity, u32)>,
    images: usize,
}

fn sample(console: &mut Console, observer: &Observer, program: &str) -> ConsoleResult<Observation> {
    let breakpoint = observer.return_breakpoint();
    console.machine().set_code_breakpoints(&[breakpoint])?;
    let mut last_partial = None;
    for _ in 0..1_000 {
        match console.pump()? {
            StopReason::Breakpoint(hit) if hit.cpu == 0 && hit.breakpoint == breakpoint => {
                match observer.at_pause(console.machine()) {
                    Ok(process)
                        if process.sample().executable_basename.as_deref() == Some(program) =>
                    {
                        let observation = process.executable()?.into_observation();
                        console.machine().set_code_breakpoints(&[])?;
                        console.machine().stop_handle().reset();
                        return Ok(observation);
                    }
                    Ok(_) => {}
                    Err(
                        error @ (ObservationError::NoAddressSpace
                        | ObservationError::UnsupportedBoundary),
                    ) => {
                        last_partial = Some(error);
                    }
                    Err(error) => return Err(error.into()),
                }
                console.machine().stop_handle().reset();
            }
            StopReason::BudgetExhausted | StopReason::Halted => {}
            stop => return Err(format!("unexpected host sampling stop {stop:?}").into()),
        }
        if console
            .lines()
            .any(|line| line.starts_with("RB_HOST_EXIT:"))
        {
            break;
        }
    }
    Err(format!(
        "no host sample for {program} ({last_partial:?}), COM1 {:?}",
        console.tail()
    )
    .into())
}

fn benchmark(console: &mut Console, observer: &Observer) -> ConsoleResult<()> {
    const SAMPLES: usize = 256;
    let mut process_ns = Vec::with_capacity(SAMPLES);
    let mut maps_ns = Vec::with_capacity(SAMPLES);
    let mut elf_ns = Vec::with_capacity(SAMPLES);
    let mut resident = 0;
    let mut last_error = None;
    for _ in 0..SAMPLES {
        let start = Instant::now();
        let process = observer.at_pause(console.machine())?;
        let captured = Instant::now();
        let mut executable = process.executable()?;
        let mapped = Instant::now();
        let analysis = executable.inspect_elf();
        let analyzed = Instant::now();
        match std::hint::black_box(analysis) {
            Ok(_) => resident += 1,
            Err(error) => last_error = Some(error),
        }
        process_ns.push((captured - start).as_nanos());
        maps_ns.push((mapped - captured).as_nanos());
        elf_ns.push((analyzed - mapped).as_nanos());
    }
    for samples in [&mut process_ns, &mut maps_ns, &mut elf_ns] {
        samples.sort_unstable();
    }
    let mid = SAMPLES / 2;
    println!(
        "Median paused-read latency over {SAMPLES} repetitions, process {} ns, mappings/auxv {} ns, optional ELF {} ns ({resident} successful)",
        process_ns[mid], maps_ns[mid], elf_ns[mid]
    );
    if let Some(error) = last_error {
        println!("Optional ELF unavailable: {error}");
    }
    Ok(())
}

fn main() -> ConsoleResult<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let (kernel, initramfs, timed) = match args.as_slice() {
        [kernel, initramfs] => (kernel, initramfs, false),
        [kernel, initramfs, flag] if flag.as_os_str() == "--benchmark" => (kernel, initramfs, true),
        _ => return Err("usage: linux_host_observe BZIMAGE INITRAMFS [--benchmark]".into()),
    };
    let handlers = App::<State, String>::new()
        .on(|event: &ProcessSample, state: &mut State| {
            state.processes.push((event.task, event.mm_address));
            println!(
                "Host process {} pid={} tid={} start={} mm={:#x} cr3={:#x} {:?} user_ip={:#x}",
                event.executable_basename.as_deref().unwrap_or("?"),
                event.task.pid,
                event.task.tid,
                event.task.start_boottime_ns,
                event.mm_address,
                event.cr3,
                event.point,
                event.user_ip,
            );
        })
        .on(|event: &ExecutableSample, state: &mut State| {
            if state.processes.len() != state.images + 1
                || state.processes.last() != Some(&(event.task, event.mm_address))
                || event.entry.is_none_or(|entry| {
                    !event
                        .mappings
                        .iter()
                        .any(|mapping| (mapping.start..mapping.end).contains(&entry))
                })
            {
                return Err(
                    "executable sample does not match preceding process or executable mappings"
                        .to_owned(),
                );
            }
            state.images += 1;
            println!(
                "Host image pid={} entry={:#x}",
                event.task.pid,
                event.entry.unwrap()
            );
            for mapping in &event.mappings {
                println!(
                    "Host executable VMA {:#x}..{:#x} file offset {:#x}",
                    mapping.start, mapping.end, mapping.file_offset
                );
            }
            Ok(ExecutionRequest::None)
        });
    let mut app = handlers.with_state(State::default());
    let kernel = fs::read(kernel)?;
    let observer = Observer::for_kernel(&kernel)?;
    let mut console = Console::boot(&kernel, &fs::read(initramfs)?)?;
    for (program, output) in [
        ("program_sum", "sum=22"),
        ("program_text", "argv0=/bin/program_text"),
    ] {
        console.clear_output();
        console.command(&format!("/bin/{program}; echo RB_HOST_EXIT:$?"))?;
        let observation = sample(&mut console, &observer, program)?;
        if timed {
            benchmark(&mut console, &observer)?;
        }
        if observation.deliver(&mut app)? == ExecutionRequest::Stop {
            return Err("application requested a stop at the sampled boundary".into());
        }
        let status = console.wait_for("ordinary command exit", 600, |line| {
            line.strip_prefix("RB_HOST_EXIT:").map(str::to_owned)
        })?;
        if status != "0" || !console.lines().any(|line| line == output) {
            return Err(format!(
                "ordinary command {program} failed, status {status}, COM1 {:?}",
                console.tail()
            )
            .into());
        }
        println!("Guest output {output}");
    }
    let state = app.into_state();
    if state.processes.len() != 2
        || state.images != 2
        || state.processes[0].0 == state.processes[1].0
    {
        return Err("unexpected recovered host observation state".into());
    }
    println!("Recovered state with two distinct host tasks and two images");
    Ok(())
}
