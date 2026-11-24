//! Boot Debian Linux and handle process reports while the guest writes COM1.
//! Start with `main` and the two handlers. `SerialReports` connects COM1 to the app.

use std::cell::OnceCell;
use std::fs;
use std::path::Path;

use rusty_box::cpu::Instrumentation;
use rusty_box::cpu::instrumentation::{HookMask, IoHookEvent};
use rusty_box::emulator::{EmulatorConfig, Ips, MachineBuilder, MemorySize, RunBudget, StopReason};
use rusty_box::events::{App, Emit, ExecutionRequest, FromEvent};
use rusty_box::gui::NoGui;

fn main() -> Result<(), DemoError> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let app = App::fallible().route(print_exec);
    match args.as_slice() {
        [kernel, initramfs] => {
            let app = app.route(print_process).with_state(ProcessCount::default());
            run_linux(Path::new(kernel), Path::new(initramfs), app)
        }
        [kernel, initramfs, flag] if flag == "--events-only" => run_linux(
            Path::new(kernel),
            Path::new(initramfs),
            app.with_state(ProcessCount::default()),
        ),
        _ => Err(DemoError::Usage),
    }
}

fn print_exec(report: &ExecReported) {
    println!("Guest reported exec of {}", report.path);
}

#[derive(Default)]
struct ProcessCount {
    observed: usize,
}

fn print_process(_: &ExecReported, process: &LoadedProcess, state: &mut ProcessCount) {
    state.observed += 1;
    println!("  Process {} has Linux PID {}", state.observed, process.pid);
    println!("  entry {:#x}", process.entry);
}

struct ExecReported {
    header: String,
    path: String,
    process: OnceCell<Result<LoadedProcess, ReportError>>,
}

struct LoadedProcess {
    pid: u32,
    entry: u32,
}

impl FromEvent<ExecReported> for LoadedProcess {
    type Error = ReportError;

    fn from_event<'a>(report: &'a ExecReported, _: &'a mut ()) -> Result<&'a Self, ReportError> {
        report
            .process
            .get_or_init(|| Self::from_header(&report.header))
            .as_ref()
            .map_err(|error| *error)
    }
}

impl LoadedProcess {
    fn from_header(header: &str) -> Result<Self, ReportError> {
        let mut words = header.split_ascii_whitespace();
        let pid = read_hex_word(words.next())?;
        let entry = read_hex_word(words.next())?;
        if pid == 0 || pid > i32::MAX as u32 || entry == 0 {
            return Err(ReportError::InvalidExec);
        }
        Ok(Self { pid, entry })
    }
}

fn read_hex_word(word: Option<&str>) -> Result<u32, ReportError> {
    let word = word
        .filter(|word| word.len() == 8)
        .ok_or(ReportError::InvalidExec)?;
    u32::from_str_radix(word, 16).map_err(|_| ReportError::InvalidExec)
}

fn run_linux<Sink>(kernel: &Path, initramfs: &Path, app: Sink) -> Result<(), DemoError>
where
    Sink: 'static
        + Emit<
            ExecReported,
            // The run loop propagates report errors after each guest step.
            Error = ReportError,
        >,
{
    let config = EmulatorConfig {
        memory: MemorySize::bytes(128 * 1024 * 1024),
        ips: Ips::new(300_000_000),
        ..Default::default()
    };
    let mut guest = MachineBuilder::new(config)
        .gui(NoGui::new())
        .tracer(SerialReports::new(app))
        .build()?;
    guest.setup_direct_linux_boot(
        &fs::read(kernel)?,
        Some(&fs::read(initramfs)?),
        KERNEL_COMMAND_LINE,
    )?;

    let mut command = WORKLOAD;
    let mut console = Vec::new();
    for _ in 0..6_000 {
        let outcome = guest.step(RunBudget::Ticks(2_000_000))?;
        match outcome.stop {
            StopReason::BudgetExhausted | StopReason::Halted => {}
            stop => return Err(DemoError::UnexpectedStop(stop)),
        }
        console.extend(guest.serial(0).ok_or(DemoError::NoConsole)?.take_output());
        if console.len() > 4096 {
            console.drain(..console.len() - 4096);
        }
        if !guest.instrumentation().ready {
            let transcript = String::from_utf8_lossy(&console);
            if let Some(version) = transcript
                .split_inclusive('\n')
                .filter(|line| line.ends_with('\n'))
                .find_map(|line| line.trim_end().strip_prefix("RB_READY:"))
            {
                println!("Booted Linux {version}");
                guest.instrumentation_mut().ready = true;
                guest.refresh_hook_mask();
            }
        }
        let reports = guest.instrumentation();
        if reports.delivery? == ExecutionRequest::Stop {
            return Ok(());
        }
        if reports.finished {
            if reports.execs != 2 || reports.exits != 2 {
                return Err(ReportError::IncompleteRun.into());
            }
            println!(
                "Both Linux programs exited successfully. {} process extractions.",
                reports.extractions
            );
            return Ok(());
        }
        if reports.ready && !command.is_empty() {
            let sent = guest.serial(0).ok_or(DemoError::NoConsole)?.send(command);
            command = &command[sent..];
        }
    }
    Err(DemoError::Timeout {
        ip: guest.rip(),
        transcript: String::from_utf8_lossy(&console).into_owned(),
    })
}

// The existing fixture's supervisor reports exec metadata and reads "r\n"
// before resuming its child. The guest shell connects the input via a pipe.
const WORKLOAD: &[u8] = b"printf 'r\\n' | /bin/rb-observe /bin/program_sum && \
    printf 'r\\n' | /bin/rb-observe /bin/program_text && echo RB_TYPED_DONE\n";
const KERNEL_COMMAND_LINE: &str = "console=ttyS0,115200 earlycon=uart8250,io,0x3f8,115200n8 \
    nokaslr nomodeset quiet rdinit=/init";
const MAX_LINE: usize = 512;

struct SerialReports<Sink> {
    app: Option<Sink>,
    line: Vec<u8>,
    header: Option<String>,
    delivery: Result<ExecutionRequest, ReportError>,
    ready: bool,
    finished: bool,
    execs: usize,
    exits: usize,
    extractions: usize,
}

impl<Sink> SerialReports<Sink> {
    fn new(app: Sink) -> Self {
        Self {
            app: Some(app),
            ..Self::default()
        }
    }
}

impl<Sink> Default for SerialReports<Sink> {
    fn default() -> Self {
        Self {
            app: None,
            line: Vec::new(),
            header: None,
            delivery: Ok(ExecutionRequest::None),
            ready: false,
            finished: false,
            execs: 0,
            exits: 0,
            extractions: 0,
        }
    }
}

impl<Sink> Instrumentation for SerialReports<Sink>
where
    Sink: 'static
        + Emit<
            ExecReported,
            // The I/O hook records delivery errors for the run loop.
            Error = ReportError,
        >,
{
    fn active_hooks(&self) -> HookMask {
        if self.ready {
            HookMask::IO
        } else {
            HookMask::empty()
        }
    }

    fn outp(&mut self, io: &IoHookEvent) {
        if io.port.get() != 0x3F8 || io.size != 1 || self.delivery != Ok(ExecutionRequest::None) {
            return;
        }
        if io.value as u8 != b'\n' {
            if self.line.len() < MAX_LINE {
                self.line.push(io.value as u8);
            }
            return;
        }
        let mut line = std::mem::take(&mut self.line);
        if line.len() < MAX_LINE {
            if let Ok(line) = std::str::from_utf8(&line) {
                self.read_line(line.trim_end());
            }
        }
        line.clear();
        self.line = line;
    }
}

impl<Sink> SerialReports<Sink>
where
    Sink: Emit<
            ExecReported,
            // Malformed reports and delivery errors use the same error type.
            Error = ReportError,
        >,
{
    fn read_line(&mut self, line: &str) {
        match line.split_once(' ') {
            Some(("RB_EXEC", header)) => self.header = Some(header.to_owned()),
            Some(("RB_EXE", path)) => self.publish_exec(path),
            Some(("RB_STATUS", fields))
                if fields.split_ascii_whitespace().nth(1) == Some("00000000") =>
            {
                self.exits += 1;
            }
            Some(("RB_STATUS", _)) => self.delivery = Err(ReportError::GuestFailed),
            Some(("RB_ERROR", _)) => {
                eprintln!("{line}");
                self.delivery = Err(ReportError::GuestFailed);
            }
            _ if line == "RB_TYPED_DONE" => self.finished = true,
            _ if line.starts_with("sum=") || line.starts_with("argv0=") => {
                println!("Guest output {line}");
            }
            _ => {}
        }
    }

    fn publish_exec(&mut self, path: &str) {
        let (Some(header), Some(app)) = (self.header.take(), &mut self.app) else {
            self.delivery = Err(ReportError::InvalidExec);
            return;
        };
        let report = ExecReported {
            header,
            path: path.to_owned(),
            process: OnceCell::new(),
        };
        self.execs += 1;
        self.delivery = app.emit(&report, &mut ());
        self.extractions += usize::from(report.process.get().is_some());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
enum ReportError {
    #[error("invalid guest exec report")]
    InvalidExec,
    #[error("guest supervisor or executable failed")]
    GuestFailed,
    #[error("expected two exec reports and two successful exits")]
    IncompleteRun,
}

#[derive(Debug, thiserror::Error)]
enum DemoError {
    #[error("usage: typed_event_workflow BZIMAGE INITRAMFS [--events-only]")]
    Usage,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Emulator(#[from] rusty_box::Error),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error("guest has no COM1")]
    NoConsole,
    #[error("Linux did not complete the workload at IP {ip:#x}. COM1 tail: {transcript:?}")]
    Timeout { ip: u64, transcript: String },
    #[error("unexpected emulator stop: {0:?}")]
    UnexpectedStop(StopReason),
}

#[cfg(test)]
#[path = "../tests/examples/typed_event_workflow.rs"]
mod tests;
