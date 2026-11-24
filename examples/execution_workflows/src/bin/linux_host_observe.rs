//! Route an observed Linux task and read its executable mappings on demand.

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use rusty_box::emulator::StopReason;
use rusty_box::events::{App, ExecutionRequest, FromEvent};
use rusty_box_execution_workflows::linux::console::{Console, ConsoleResult};
use rusty_box_execution_workflows::linux::host::{
    ExecutableSample, ObservationError, Observer, PausedProcess, SamplePoint, TaskIdentity,
};

struct ProcessFound {
    program: &'static str,
    task: TaskIdentity,
    mm_address: u32,
    cr3: u64,
    point: SamplePoint,
    user_ip: u32,
}

struct ImageSource<'guest> {
    paused: Option<PausedProcess<'guest, ()>>,
    image: Option<ExecutableSample>,
}

impl FromEvent<ProcessFound, ImageSource<'_>> for ExecutableSample {
    type Error = ObservationError;

    fn from_event<'a>(
        _: &'a ProcessFound,
        source: &'a mut ImageSource<'_>,
    ) -> Result<&'a Self, ObservationError> {
        if let Some(process) = source.paused.take() {
            source.image = Some(process.executable()?.into_observation().executable);
        }
        source.image.as_ref().ok_or(ObservationError::NoExecutable)
    }
}

fn print_process(event: &ProcessFound) {
    let task = event.task;
    println!(
        "Host process {} pid={} tid={} start={} mm={:#x} cr3={:#x} {:?} user_ip={:#x}",
        event.program,
        task.pid,
        task.tid,
        task.start_boottime_ns,
        event.mm_address,
        event.cr3,
        event.point,
        event.user_ip,
    );
}

fn print_entry(
    event: &ProcessFound,
    image: &ExecutableSample,
) -> Result<ExecutionRequest, ObservationError> {
    let entry = image
        .entry
        .filter(|entry| {
            image
                .mappings
                .iter()
                .any(|mapping| (mapping.start..mapping.end).contains(entry))
        })
        .ok_or(ObservationError::Invalid(
            "entry is outside the executable mappings",
        ))?;
    println!("Host image pid={} entry={entry:#x}", event.task.pid);
    Ok(ExecutionRequest::None)
}

fn print_mappings(_: &ProcessFound, image: &ExecutableSample) {
    for mapping in &image.mappings {
        println!(
            "Host executable VMA {:#x}..{:#x} file offset {:#x}",
            mapping.start, mapping.end, mapping.file_offset
        );
    }
}

struct DeliveredSample {
    task: TaskIdentity,
    prepared_image: bool,
}

fn deliver_sample(
    program: &'static str,
    process: PausedProcess<'_, ()>,
    with_image: bool,
) -> ConsoleResult<DeliveredSample> {
    let sample = process.sample();
    let event = ProcessFound {
        program,
        task: sample.task,
        mm_address: sample.mm_address,
        cr3: sample.cr3,
        point: sample.point,
        user_ip: sample.user_ip,
    };
    let source = ImageSource {
        paused: Some(process),
        image: None,
    };
    let routes = App::fallible().route(print_process);
    let (request, source) = if with_image {
        let mut app = routes
            .route(print_entry)
            .route(print_mappings)
            .with_state(source);
        (app.dispatch(&event)?, app.into_state())
    } else {
        let mut app = routes.with_state(source);
        (app.dispatch(&event)?, app.into_state())
    };
    if request == ExecutionRequest::Stop {
        return Err("application requested a stop at the sampled boundary".into());
    }
    Ok(DeliveredSample {
        task: event.task,
        prepared_image: source.image.is_some(),
    })
}

fn sample(
    console: &mut Console,
    observer: &Observer,
    program: &'static str,
    with_image: bool,
) -> ConsoleResult<DeliveredSample> {
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
                        let observation = deliver_sample(program, process, with_image)?;
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
    let (kernel, initramfs, with_image, timed) = match args.as_slice() {
        [kernel, initramfs] => (kernel, initramfs, true, false),
        [kernel, initramfs, flag] if flag.as_os_str() == "--process-only" => {
            (kernel, initramfs, false, false)
        }
        [kernel, initramfs, flag] if flag.as_os_str() == "--benchmark" => {
            (kernel, initramfs, true, true)
        }
        _ => {
            return Err(
                "usage: linux_host_observe BZIMAGE INITRAMFS [--process-only | --benchmark]".into(),
            );
        }
    };
    let kernel = fs::read(kernel)?;
    let observer = Observer::for_kernel(&kernel)?;
    let mut console = Console::boot(&kernel, &fs::read(initramfs)?)?;
    let mut tasks = Vec::new();
    let mut images = 0;
    for (program, output) in [
        ("program_sum", "sum=22"),
        ("program_text", "argv0=/bin/program_text"),
    ] {
        console.clear_output();
        console.command(&format!("/bin/{program}; echo RB_HOST_EXIT:$?"))?;
        let observation = sample(&mut console, &observer, program, with_image)?;
        tasks.push(observation.task);
        images += usize::from(observation.prepared_image);
        if timed {
            benchmark(&mut console, &observer)?;
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
    if !matches!(tasks.as_slice(), [first, second] if first != second) {
        return Err("expected two distinct host tasks".into());
    }
    println!("Observed two distinct host tasks. Prepared {images} executable images.");
    Ok(())
}
