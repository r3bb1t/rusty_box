//! Boot Debian Linux and handle process reports while the guest writes COM1.

use std::cell::OnceCell;
use std::fs;
use std::path::Path;

use rusty_box::cpu::Instrumentation;
use rusty_box::cpu::instrumentation::{HookMask, IoHookEvent};
use rusty_box::emulator::{EmulatorConfig, Ips, MachineBuilder, MemorySize, RunBudget, StopReason};
use rusty_box::events::{App, Emit, ExecutionRequest, Resolve, Resolved};
use rusty_box::gui::NoGui;

fn main() -> Result<(), DemoError> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let app = App::fallible().route(print_exec);
    match args.as_slice() {
        [kernel, initramfs] => {
            let app = app
                .route(print_process)
                .route(print_entry)
                .with_state(ProcessResolver);
            run_linux(Path::new(kernel), Path::new(initramfs), app)
        }
        [kernel, initramfs, flag] if flag == "--events-only" => run_linux(
            Path::new(kernel),
            Path::new(initramfs),
            app.with_state(ProcessResolver),
        ),
        _ => Err(DemoError::Usage),
    }
}

fn print_exec(report: &ExecReported) {
    println!("Guest reported exec of {}", report.path);
}

fn print_process(_: &ExecReported, Resolved(process): Resolved<'_, LoadedProcess>) {
    println!("  Linux PID {}", process.pid);
}

fn print_entry(_: &ExecReported, Resolved(process): Resolved<'_, LoadedProcess>) {
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

struct ProcessResolver;

impl Resolve<ExecReported, LoadedProcess> for ProcessResolver {
    type Error = ReportError;

    fn resolve<'a>(
        &'a mut self,
        report: &'a ExecReported,
    ) -> Result<&'a LoadedProcess, ReportError> {
        report
            .process
            .get_or_init(|| {
                let mut words = report.header.split_ascii_whitespace();
                let pid = read_word(words.next())?;
                let entry = read_word(words.next())?;
                if pid == 0 || pid > i32::MAX as u32 || entry == 0 {
                    return Err(ReportError::InvalidExec);
                }
                Ok(LoadedProcess { pid, entry })
            })
            .as_ref()
            .map_err(|error| *error)
    }
}

fn read_word(word: Option<&str>) -> Result<u32, ReportError> {
    let word = word
        .filter(|word| word.len() == 8)
        .ok_or(ReportError::InvalidExec)?;
    u32::from_str_radix(word, 16).map_err(|_| ReportError::InvalidExec)
}

fn run_linux(
    kernel: &Path,
    initramfs: &Path,
    app: impl Emit<ExecReported, Error = ReportError> + 'static,
) -> Result<(), DemoError> {
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
        "console=ttyS0,115200 earlycon=uart8250,io,0x3f8,115200n8 nokaslr nomodeset quiet rdinit=/init",
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
                "Both Linux programs exited successfully. {} process resolutions.",
                reports.lookups
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
const WORKLOAD: &[u8] = b"printf 'r\\n' | /bin/rb-observe /bin/program_sum && printf 'r\\n' | /bin/rb-observe /bin/program_text && echo RB_TYPED_DONE\n";
const MAX_LINE: usize = 512;

struct SerialReports<A> {
    app: Option<A>,
    line: Vec<u8>,
    header: Option<String>,
    delivery: Result<ExecutionRequest, ReportError>,
    ready: bool,
    finished: bool,
    execs: usize,
    exits: usize,
    lookups: usize,
}

impl<A> SerialReports<A> {
    fn new(app: A) -> Self {
        Self {
            app: Some(app),
            ..Self::default()
        }
    }
}

impl<A> Default for SerialReports<A> {
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
            lookups: 0,
        }
    }
}

impl<A: Emit<ExecReported, Error = ReportError> + 'static> Instrumentation for SerialReports<A> {
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

impl<A: Emit<ExecReported, Error = ReportError>> SerialReports<A> {
    fn read_line(&mut self, line: &str) {
        if let Some(header) = line.strip_prefix("RB_EXEC ") {
            self.header = Some(header.to_owned());
        } else if let Some(path) = line.strip_prefix("RB_EXE ") {
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
            self.lookups += usize::from(report.process.get().is_some());
        } else if line.starts_with("RB_STATUS ") {
            if line.split_ascii_whitespace().nth(2) == Some("00000000") {
                self.exits += 1;
            } else {
                self.delivery = Err(ReportError::GuestFailed);
            }
        } else if line.starts_with("RB_ERROR ") {
            eprintln!("{line}");
            self.delivery = Err(ReportError::GuestFailed);
        } else if line == "RB_TYPED_DONE" {
            self.finished = true;
        } else if line.starts_with("sum=") || line.starts_with("argv0=") {
            println!("Guest output {line}");
        }
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
