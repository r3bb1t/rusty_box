//! Host-driven ELF substitution on a directly booted Debian i386 kernel.

use std::convert::Infallible;
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use rusty_box::cpu::CodeBreakpoint;
use rusty_box::emulator::StopReason;
use rusty_box_core::events::{Emit, ExecutionRequest, Subscriptions};
use rusty_box_execution_workflows::linux::console::Console;
use rusty_box_execution_workflows::linux::{
    I386ImageProbe, ImageObservers, ProcessCreated, ProcessExited,
};

type HostResult<T> = Result<T, Box<dyn Error>>;
const STAGE: &str = "/bin/cat.rb-stage";
const ORIGINAL: &str = "/bin/cat.rb-original";
const START: &str = "/tmp/rb-cat-start";
const MARKER: &str = "RB_REPLACEMENT_OK";

struct Replacement(Console);

impl Replacement {
    fn check_install_paths(&mut self) -> HostResult<()> {
        self.0.command(&format!("test -x /bin/cat && test ! -e {ORIGINAL} && test ! -e {STAGE} && test ! -e {START} && echo RB_CLEAR"))?;
        self.0.wait_line("RB_CLEAR")
    }

    fn install(&mut self, elf: &[u8]) -> HostResult<()> {
        for (i, chunk) in elf.chunks(64).enumerate() {
            let mut bytes = String::new();
            for byte in chunk {
                write!(bytes, "\\0{byte:03o}")?;
            }
            let redirect = if i == 0 { ">" } else { ">>" };
            self.0
                .command(&format!("printf '%b' '{bytes}' {redirect} {STAGE}"))?;
        }
        self.0.command(&format!("test \"$(wc -c < {STAGE})\" -eq {} && chmod 755 {STAGE} && mv /bin/cat {ORIGINAL} && mv {STAGE} /bin/cat && echo RB_INSTALLED", elf.len()))?;
        self.0.wait_line("RB_INSTALLED")
    }

    fn spawn(&mut self) -> HostResult<ProcessCreated> {
        self.0.command(&format!("mkfifo {START} && {{ (read gate < {START} && exec /bin/cat) & p=$!; echo RB_FORK:$p; }}"))?;
        self.0.wait_for("child PID", 350, |line| {
            let pid = line.strip_prefix("RB_FORK:")?.parse().ok()?;
            (pid != 0).then_some(ProcessCreated { pid })
        })
    }

    fn run_replacement(
        &mut self,
        child: ProcessCreated,
        probe: &I386ImageProbe,
        observers: &mut ImageObservers<'_>,
    ) -> HostResult<ProcessExited> {
        self.0
            .machine()
            .set_code_breakpoints(&[CodeBreakpoint::Linear(probe.checkpoint())])?;
        self.0.command(&format!(
            "echo go > {START}; wait \"$p\"; echo RB_EXIT:$p:$?; p="
        ))?;
        let mut mapped_pid = None;
        for _ in 0..600 {
            match self.0.pump()? {
                StopReason::Breakpoint(hit)
                    if hit.linear == probe.checkpoint() && mapped_pid.is_none() =>
                {
                    let started = Instant::now();
                    let image = observers.prepare(self.0.machine(), probe, hit)?;
                    let first_use = started.elapsed();
                    if image.event().pid != child.pid {
                        return Err(format!(
                            "mapped PID {} differs from the launched PID {}",
                            image.event().pid,
                            child.pid
                        )
                        .into());
                    }
                    let started = Instant::now();
                    for _ in 0..100_000 {
                        std::hint::black_box(std::hint::black_box(&image).binary());
                    }
                    let cached_ns = started.elapsed().as_nanos() as f64 / 100_000.0;
                    mapped_pid = Some(image.event().pid);
                    let request = observers.deliver(&image);
                    match image.binary() {
                        Some(Err(error)) => {
                            return Err(format!("guest ELF analysis: {error}").into());
                        }
                        Some(Ok(binary))
                            if binary.loaded_code_sha256 != probe.supplied_code_sha256() =>
                        {
                            return Err(
                                "mapped ELF code differs from the host-supplied executable".into(),
                            );
                        }
                        _ => {}
                    }
                    if request == ExecutionRequest::Stop {
                        return Err("image subscriber requested a stop at the checkpoint".into());
                    }
                    println!(
                        "Live ELF observation and analysis {:?}, cached access {:.1} ns",
                        first_use, cached_ns
                    );
                    self.0.machine().set_code_breakpoints(&[])?;
                    self.0.machine().stop_handle().reset();
                }
                StopReason::Halted | StopReason::BudgetExhausted => {}
                stop => return Err(format!("unexpected child stop: {stop:?}").into()),
            }
            if let Some(exit) = self.0.lines().find_map(parse_exit) {
                if mapped_pid != Some(exit.pid)
                    || exit.wait_status != 0
                    || !self.0.lines().any(|line| line == MARKER)
                {
                    return Err(format!(
                        "guest output or identity mismatch: {mapped_pid:?}, {exit:?}, {:?}",
                        self.0.tail()
                    )
                    .into());
                }
                println!("Guest output: {MARKER}");
                return Ok(exit);
            }
        }
        Err(format!("child did not finish: {:?}", self.0.tail()).into())
    }

    fn restore(&mut self) -> HostResult<()> {
        self.0.machine().set_code_breakpoints(&[])?;
        self.0.machine().stop_handle().reset();
        self.0.command(&format!("if test -n \"$p\"; then kill \"$p\" 2>/dev/null; wait \"$p\" 2>/dev/null; p=; fi; if test -e {ORIGINAL}; then mv -f {ORIGINAL} /bin/cat; fi; rm -f {STAGE} {START}; echo RB_ORIGINAL > /tmp/rb-cat-input; /bin/cat /tmp/rb-cat-input && echo RB_RESTORED; rm -f /tmp/rb-cat-input"))?;
        self.0.wait_line("RB_RESTORED")?;
        self.0.wait_line("RB_ORIGINAL")
    }
}

fn parse_exit(line: &str) -> Option<ProcessExited> {
    let (pid, status) = line.strip_prefix("RB_EXIT:")?.split_once(':')?;
    Some(ProcessExited {
        pid: pid.parse().ok()?,
        wait_status: status.parse().ok()?,
    })
}

fn main() -> HostResult<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [kernel, initramfs, replacement] = args.as_slice() else {
        return Err("usage: linux_cat BZIMAGE INITRAMFS REPLACEMENT_ELF".into());
    };
    let mut guest = Replacement(Console::boot(&fs::read(kernel)?, &fs::read(initramfs)?)?);
    // The replacement first reaches the host workflow after the guest is ready.
    let elf = fs::read(replacement)?;
    if elf.is_empty() || elf.len() > 4096 {
        return Err("replacement must be between 1 and 4096 bytes".into());
    }
    let probe = I386ImageProbe::for_replacement(&elf)?;
    let mut created = Subscriptions::<ProcessCreated, Infallible>::new()
        .on(|event| println!("ProcessCreated {event:?}"));
    let mut exited = Subscriptions::<ProcessExited, Infallible>::new()
        .on(|event| println!("ProcessExited {event:?}"));
    let mut observers = ImageObservers::default();
    observers.on(|image| println!("ProcessImageMapped {image:?}"));
    observers.on_binary(|image, binary| {
        println!(
            "ELF {:?}, entry {:#x}, base {:#x}, bias {:#x}, SHA-256 {:02x?}",
            binary.format,
            binary.entry,
            binary.guest_base,
            binary.load_bias,
            binary.loaded_code_sha256
        );
        println!(
            "Image-relative checkpoint {:?}",
            binary.code_offset(image.space, image.checkpoint)
        );
        ExecutionRequest::None
    });
    observers.on_binary(|_, binary| {
        println!(
            "Shared result with {} program headers",
            binary.program_headers.len()
        );
        ExecutionRequest::None
    });
    guest.check_install_paths()?;
    let result = (|| {
        guest.install(&elf)?;
        let child = guest.spawn()?;
        if created.emit(&child, &mut ())? == ExecutionRequest::Stop {
            return Err("process subscriber requested a stop before exec".into());
        }
        guest.run_replacement(child, &probe, &mut observers)
    })();
    let restoration = guest.restore();
    match (result, restoration) {
        (Ok(exit), Ok(())) => {
            match exited.emit(&exit, &mut ())? {
                ExecutionRequest::None | ExecutionRequest::Stop => {}
            }
            println!("Original /bin/cat restored and read the verification file");
            Ok(())
        }
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(restore)) => {
            Err(format!("replacement: {error}; restore: {restore}").into())
        }
    }
}
