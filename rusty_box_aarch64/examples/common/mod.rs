//! What the example boot programs share: the options every run takes, a
//! console script (wait for text, type keys), the run loop that streams the
//! guest's UART to stdout while following the script, and the end-of-run
//! report.

use std::collections::VecDeque;
use std::io::Write;
use std::process::ExitCode;
use std::time::Instant;

use rusty_box_aarch64::cpu::DebugStops;
use rusty_box_aarch64::{
    CursorPos, Dimensions, Disk, DisplaySink, GuestStorage, KeyboardInput, MachineBuilder, Redraw, Reg, Refreshed,
    Resolution, Rgb, RunBudget, StopReason, SysReg, TilePos, Typed, VgaTextModeInfo, VirtMachine,
};
use rusty_box_core::ClockHz;

/// Ticks per step: how often the console is drained and typed keys are
/// delivered.
const SLICE_TICKS: u64 = 4_000_000;

/// The stack the thread that builds and runs a machine is given. A
/// `VirtMachine` is about 170 KiB (its trace cache is on the heap), and a
/// build moves it by value a few times, which comes close to the 1 MiB a
/// Windows main thread has; tests take the same floor from
/// `RUST_MIN_STACK` in `.cargo/config.toml`.
const MACHINE_STACK_BYTES: usize = 16 * 1024 * 1024;

/// Run `program` on a thread with [`MACHINE_STACK_BYTES`] of stack and
/// return its exit code; a panic or a thread that cannot start is a
/// failure.
pub fn on_machine_thread<F>(program: F) -> ExitCode
where
    F: FnOnce() -> ExitCode + Send + 'static,
{
    let thread = std::thread::Builder::new()
        .name("machine".to_owned())
        .stack_size(MACHINE_STACK_BYTES)
        .spawn(program);
    match thread.map(std::thread::JoinHandle::join) {
        Ok(Ok(code)) => code,
        Ok(Err(_)) => {
            eprintln!("the machine thread panicked");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("cannot start the machine thread: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Which keyboard a script types on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keys {
    /// The machine's keyboard: the virtio keyboard if one is plugged in,
    /// else the terminal's.
    Machine,
    /// The keyboard of the terminal on the UART.
    Terminal,
}

/// One step of the console script.
pub enum Action {
    /// Wait until this text has appeared on the console.
    Until(Vec<u8>),
    /// Type this text on a keyboard.
    Type(Keys, String),
}

/// Whether [`RunArgs::parse_flag`] consumed a flag.
pub enum Parsed {
    Consumed,
    NotMine,
}

/// The options every boot program takes.
pub struct RunArgs {
    pub ips: u64,
    pub max_ticks: u64,
    pub slice_ticks: u64,
    pub script: Vec<Action>,
    pub stop_on_undefined: bool,
    pub stop_on_bus_fault: bool,
    /// A virtio-gpu asking the guest for this size.
    pub display: Option<Resolution>,
    pub virtio_keyboard: bool,
    pub virtio_tablet: bool,
    /// A raw image file behind a virtio-blk, read and written in place.
    pub disk: Option<String>,
    /// Where to write the display's last frame, as a binary PPM.
    pub screenshot: Option<String>,
}

/// How a run ended.
pub struct Outcome {
    pub reason: StopReason,
    pub script_completed: bool,
}

/// The usage text for the shared options.
pub const RUN_USAGE: &str = "[--ips N] [--max-ticks N] [--slice-ticks N]\n\
     \x20      [--until <console text>]... [--type <keys>]... [--type-serial <keys>]...\n\
     \x20      [--stop-on-undefined] [--stop-on-bus-fault]\n\
     \x20      [--display WIDTHxHEIGHT] [--virtio-keyboard] [--virtio-tablet] [--disk <raw image>]\n\
     \x20      [--screenshot <file.ppm>]";

/// The display's frames, kept as the host would show them.
#[derive(Default)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    /// Four bytes a pixel, red first.
    pub rgba: Vec<u8>,
    pub frames: u64,
}

impl DisplaySink for Screen {
    fn dimension_update(&mut self, dims: Dimensions) {
        self.width = dims.width;
        self.height = dims.height;
        self.rgba = vec![0; dims.width as usize * dims.height as usize * 4];
    }

    fn text_update(&mut self, _previous: &[u8], _current: &[u8], _cursor: Option<CursorPos>, _info: &VgaTextModeInfo) {}

    fn graphics_tile_update(&mut self, rgba: &[u8], at: TilePos) {
        let stride = self.width as usize * 4;
        let row_bytes = at.width as usize * 4;
        for row in 0..at.height as usize {
            let dst = (at.y as usize + row) * stride + at.x as usize * 4;
            let src = row * row_bytes;
            if let (Some(d), Some(s)) = (self.rgba.get_mut(dst..dst + row_bytes), rgba.get(src..src + row_bytes)) {
                d.copy_from_slice(s);
            }
        }
    }

    fn palette_change(&mut self, _index: u8, _colour: Rgb) -> Redraw {
        Redraw::NotNeeded
    }

    fn set_text_charmap(&mut self, _map: usize, _glyphs: &[u8]) {}

    fn clear_screen(&mut self) {
        self.rgba.fill(0);
    }

    fn flush(&mut self) {
        self.frames += 1;
    }
}

impl Screen {
    /// The frame as a binary PPM.
    pub fn ppm(&self) -> Vec<u8> {
        let mut out = format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        for p in self.rgba.chunks_exact(4) {
            out.extend_from_slice(&p[..3]);
        }
        out
    }

    /// How many pixels are not black.
    pub fn lit_pixels(&self) -> usize {
        self.rgba.chunks_exact(4).filter(|p| p[..3] != [0, 0, 0]).count()
    }
}

/// Plug the devices `args` asks for into `builder`.
///
/// # Errors
///
/// The disk image cannot be opened for reading and writing.
pub fn plug_devices<'i, B, I>(mut builder: MachineBuilder<'i, B, I>, args: &RunArgs) -> Result<MachineBuilder<'i, B, I>, String> {
    if let Some(size) = args.display {
        builder = builder.virtio_display(size);
    }
    if args.virtio_keyboard {
        builder = builder.virtio_keyboard();
    }
    if args.virtio_tablet {
        builder = builder.virtio_tablet();
    }
    if let Some(path) = &args.disk {
        let file = std::fs::OpenOptions::new().read(true).write(true).open(path).map_err(|e| format!("{path}: {e}"))?;
        builder = builder.virtio_disk(Disk::file(file).map_err(|e| format!("{path}: {e}"))?);
    }
    Ok(builder)
}

/// Make what the guest wrote to the disk durable, as QEMU does when it
/// closes a drive, and say if a guest request met a host error.
///
/// # Errors
///
/// The host's error syncing the disk.
pub fn close_disk<S: GuestStorage>(machine: &mut VirtMachine<S>) -> Result<(), String> {
    let Some(disk) = machine.disk_mut() else {
        return Ok(());
    };
    if let Some(e) = disk.take_io_error() {
        eprintln!("the guest was told of a disk I/O error: {e}");
    }
    disk.flush().map_err(|e| format!("flushing the disk: {e}"))
}

/// `\r` and `\n` (Enter), `\e` (Escape), `\t` (Tab) and `\\`; everything
/// else as written.
fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('r') | Some('n') => out.push('\n'),
            Some('e') => out.push('\u{1B}'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

impl RunArgs {
    pub fn new() -> Self {
        Self {
            ips: ClockHz::BOCHS_DEFAULT.get(),
            max_ticks: u64::MAX,
            slice_ticks: SLICE_TICKS,
            script: Vec::new(),
            stop_on_undefined: false,
            stop_on_bus_fault: false,
            display: None,
            virtio_keyboard: false,
            virtio_tablet: false,
            disk: None,
            screenshot: None,
        }
    }

    /// Take `flag` if it is a shared option, reading its value with `value`.
    pub fn parse_flag<F: FnMut(&str) -> Result<String, String>>(
        &mut self,
        flag: &str,
        mut value: F,
    ) -> Result<Parsed, String> {
        let number = |name: &str, text: String| text.parse::<u64>().map_err(|e| format!("{name}: {e}"));
        match flag {
            "--ips" => self.ips = number(flag, value(flag)?)?,
            "--max-ticks" => self.max_ticks = number(flag, value(flag)?)?,
            "--slice-ticks" => {
                self.slice_ticks = number(flag, value(flag)?)?;
                if self.slice_ticks == 0 {
                    return Err(String::from("--slice-ticks must be non-zero"));
                }
            }
            "--until" => self.script.push(Action::Until(unescape(&value(flag)?).into_bytes())),
            "--type" => self.script.push(Action::Type(Keys::Machine, unescape(&value(flag)?))),
            "--type-serial" => self.script.push(Action::Type(Keys::Terminal, unescape(&value(flag)?))),
            "--stop-on-undefined" => self.stop_on_undefined = true,
            "--stop-on-bus-fault" => self.stop_on_bus_fault = true,
            "--display" => {
                let text = value(flag)?;
                let (w, h) = text.split_once('x').ok_or("--display takes WIDTHxHEIGHT")?;
                let width = w.parse::<u32>().map_err(|e| format!("--display width: {e}"))?;
                let height = h.parse::<u32>().map_err(|e| format!("--display height: {e}"))?;
                self.display = Some(Resolution::new(width, height));
            }
            "--virtio-keyboard" => self.virtio_keyboard = true,
            "--virtio-tablet" => self.virtio_tablet = true,
            "--disk" => self.disk = Some(value(flag)?),
            "--screenshot" => self.screenshot = Some(value(flag)?),
            _ => return Ok(Parsed::NotMine),
        }
        Ok(Parsed::Consumed)
    }

    pub fn debug_stops(&self) -> DebugStops {
        DebugStops {
            on_undefined: self.stop_on_undefined,
            on_bus_fault: self.stop_on_bus_fault,
        }
    }

    pub fn ips(&self) -> Result<ClockHz, String> {
        ClockHz::new(self.ips).ok_or_else(|| String::from("--ips must be non-zero"))
    }
}

/// Type as much of `pending`, in order, as its keyboards take now, leaving
/// the rest in it.
fn type_pending<S: GuestStorage>(machine: &mut VirtMachine<S>, pending: &mut VecDeque<(Keys, String)>) {
    while let Some((keys, text)) = pending.front_mut() {
        type_text(machine, *keys, text);
        if !text.is_empty() {
            return;
        }
        pending.pop_front();
    }
}

/// Type as much of `pending` as `keys` takes now, leaving the rest in it. A
/// character with no key on a US layout goes in as its UTF-8 bytes.
fn type_text<S: GuestStorage>(machine: &mut VirtMachine<S>, keys: Keys, pending: &mut String) {
    while !pending.is_empty() {
        let mut keyboard = match keys {
            Keys::Machine => machine.keyboard(),
            Keys::Terminal => machine.terminal_keyboard(),
        };
        match keyboard.type_text(pending) {
            Typed::All { .. } => pending.clear(),
            Typed::Refused { delivered } => {
                *pending = pending.chars().skip(delivered).collect();
                return;
            }
            Typed::Unmappable { delivered, ch } => {
                let mut utf8 = [0u8; 4];
                let bytes = ch.encode_utf8(&mut utf8).as_bytes();
                if keyboard.bytes(bytes) < bytes.len() {
                    *pending = pending.chars().skip(delivered).collect();
                    return;
                }
                *pending = pending.chars().skip(delivered + 1).collect();
            }
        }
    }
}

/// Run until the script completes, the tick budget is spent, or the machine
/// stops, streaming the console to stdout, typing the script's keys as the
/// guest takes them, and keeping the display's picture; then write the
/// screenshot, if one was asked for.
pub fn run<S: GuestStorage>(machine: &mut VirtMachine<S>, args: &RunArgs) -> Outcome {
    let mut screen = Screen::default();
    let outcome = run_with_screen(machine, args, &mut screen);
    if let Some(mut display) = machine.display() {
        match display.refresh(&mut screen) {
            Refreshed::Frame | Refreshed::Unchanged => {}
        }
        eprintln!(
            "display: {}x{} shown, {} frames, {} lit pixels",
            screen.width,
            screen.height,
            screen.frames,
            screen.lit_pixels()
        );
    }
    if let Some(path) = &args.screenshot {
        if let Err(e) = std::fs::write(path, screen.ppm()) {
            eprintln!("{path}: {e}");
        }
    }
    outcome
}

fn run_with_screen<S: GuestStorage>(machine: &mut VirtMachine<S>, args: &RunArgs, screen: &mut Screen) -> Outcome {
    let mut stdout = std::io::stdout().lock();
    let mut script = args.script.iter();
    let mut current = script.next();
    let mut window: Vec<u8> = Vec::new();
    let mut pending: VecDeque<(Keys, String)> = VecDeque::new();
    loop {
        // Run every Type step that is now current; stop at the next Until.
        while let Some(Action::Type(keys, text)) = current {
            pending.push_back((*keys, text.clone()));
            current = script.next();
        }
        type_pending(machine, &mut pending);
        let remaining = args.max_ticks.saturating_sub(machine.ticks());
        if remaining == 0 {
            return Outcome {
                reason: StopReason::BudgetExhausted,
                script_completed: current.is_none() && pending.is_empty() && !args.script.is_empty(),
            };
        }
        let outcome = machine.step(RunBudget::Ticks(remaining.min(args.slice_ticks)));
        if let Some(mut display) = machine.display() {
            // Whether a frame came is the screen's business: it counts them.
            match display.refresh(screen) {
                Refreshed::Frame | Refreshed::Unchanged => {}
            }
        }
        let out: Vec<u8> = machine.serial(0).map(|mut uart| uart.take_output().collect()).unwrap_or_default();
        if !out.is_empty() {
            if stdout.write_all(&out).and_then(|()| stdout.flush()).is_err() {
                return Outcome {
                    reason: outcome.stop,
                    script_completed: false,
                };
            }
            window.extend_from_slice(&out);
        }
        if let Some(Action::Until(text)) = current {
            if window.windows(text.len()).any(|w| w == text.as_slice()) {
                window.clear();
                current = script.next();
                if current.is_none() {
                    return Outcome {
                        reason: outcome.stop,
                        script_completed: true,
                    };
                }
                continue;
            }
            let keep = text.len().max(1) - 1;
            if window.len() > keep {
                window.drain(..window.len() - keep);
            }
        }
        if outcome.is_terminal() {
            // A script that ends by typing (`poweroff`, say) is complete once
            // its last keys are in, however the guest then stops.
            return Outcome {
                reason: outcome.stop,
                script_completed: current.is_none() && pending.is_empty() && !args.script.is_empty(),
            };
        }
    }
}

/// FNV-1a over the machine's guest-visible state: RAM, the variable store,
/// and the processor registers. Equal hashes after equal tick counts are
/// what determinism means here.
pub fn state_hash<S: GuestStorage>(machine: &VirtMachine<S>) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
    };
    eat(machine.ram());
    eat(machine.variable_store());
    let cpu = machine.cpu_snapshot();
    for x in cpu.x {
        eat(&x.to_le_bytes());
    }
    for v in cpu.v {
        eat(&v.to_le_bytes());
    }
    eat(&machine.reg_read(Reg::Sp).to_le_bytes());
    eat(&cpu.pc.to_le_bytes());
    eat(&cpu.pstate.to_spsr().to_le_bytes());
    eat(&machine.ticks().to_le_bytes());
    h
}

/// Print how the run ended, its speed and state hash, and — when it did not
/// end as intended — the processor state a debugger would show first.
pub fn report<S: GuestStorage>(machine: &VirtMachine<S>, args: &RunArgs, outcome: &Outcome, started: Instant) {
    let elapsed = started.elapsed().as_secs_f64();
    let ticks = machine.ticks();
    let cpu = machine.cpu_snapshot();
    eprintln!();
    eprintln!("--- rusty_box_aarch64 ---");
    if outcome.script_completed {
        eprintln!("console script completed");
    }
    eprintln!("stop: {:?}", outcome.reason);
    eprintln!(
        "ticks: {ticks} ({:.3} s emulated, {} instructions) in {elapsed:.2} s host, {:.1} M ticks/s",
        ticks as f64 / args.ips as f64,
        machine.instructions(),
        ticks as f64 / elapsed.max(1e-9) / 1e6
    );
    eprintln!("pc: {:#x}  el: {:?}  sp: {:#x}", cpu.pc, cpu.pstate.el, machine.reg_read(Reg::Sp));
    eprintln!("state hash: {:016x}", state_hash(machine));
    let ended_as_intended =
        outcome.reason == StopReason::BudgetExhausted && (outcome.script_completed || args.script.is_empty());
    if !ended_as_intended {
        let recent: Vec<String> = machine.recent_pcs().map(|pc| format!("{pc:#x}")).collect();
        eprintln!("recent pcs: {}", recent.join(" "));
        for (r, x) in cpu.x.iter().enumerate() {
            eprint!("x{r}={x:#x}{}", if r % 4 == 3 { "\n" } else { " " });
        }
        eprintln!();
        for (name, reg) in [
            ("ESR_EL1", SysReg::ESR_EL1),
            ("FAR_EL1", SysReg::FAR_EL1),
            ("ELR_EL1", SysReg::ELR_EL1),
            ("SCTLR_EL1", SysReg::SCTLR_EL1),
            ("VBAR_EL1", SysReg::VBAR_EL1),
        ] {
            match machine.sysreg_read(reg) {
                Ok(v) => eprintln!("{name}={v:#x}"),
                Err(refusal) => eprintln!("{name}: {refusal}"),
            }
        }
    }
}

/// Success when the script completed, or with no script when the machine
/// ran out its budget, powered off or halted.
pub fn exit_code(args: &RunArgs, outcome: &Outcome) -> ExitCode {
    if !args.script.is_empty() {
        return if outcome.script_completed {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        };
    }
    match outcome.reason {
        StopReason::PoweredOff | StopReason::BudgetExhausted | StopReason::Halted => ExitCode::SUCCESS,
        StopReason::StopRequested
        | StopReason::ExitReached
        | StopReason::ResetRequested
        | StopReason::ProcessorOff
        | StopReason::Undefined(_)
        | StopReason::BusFault { .. } => ExitCode::from(1),
    }
}
