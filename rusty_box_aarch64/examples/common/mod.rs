//! What the example boot programs share: the options every run takes, a
//! console script (wait for text, type keys), the run loop that streams the
//! guest's UART to stdout while following the script, and the end-of-run
//! report.

use std::io::Write;
use std::process::ExitCode;
use std::time::Instant;

use rusty_box_aarch64::cpu::DebugStops;
use rusty_box_aarch64::{GuestStorage, KeyboardInput, Reg, RunBudget, StopReason, SysReg, Typed, VirtMachine};
use rusty_box_core::ClockHz;

/// Ticks per step: how often the console is drained and typed keys are
/// delivered.
const SLICE_TICKS: u64 = 4_000_000;

/// One step of the console script.
pub enum Action {
    /// Wait until this text has appeared on the console.
    Until(Vec<u8>),
    /// Type this text on the terminal's keyboard.
    Type(String),
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
}

/// How a run ended.
pub struct Outcome {
    pub reason: StopReason,
    pub script_completed: bool,
}

/// The usage text for the shared options.
pub const RUN_USAGE: &str = "[--ips N] [--max-ticks N] [--slice-ticks N]\n\
     \x20      [--until <console text>]... [--type <keys>]...\n\
     \x20      [--stop-on-undefined] [--stop-on-bus-fault]";

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
            "--type" => self.script.push(Action::Type(unescape(&value(flag)?))),
            "--stop-on-undefined" => self.stop_on_undefined = true,
            "--stop-on-bus-fault" => self.stop_on_bus_fault = true,
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

/// Type as much of `pending` as the terminal takes now, leaving the rest in
/// it. A character with no key on a US layout goes in as its UTF-8 bytes.
fn type_pending<S: GuestStorage>(machine: &mut VirtMachine<S>, pending: &mut String) {
    while !pending.is_empty() {
        let mut keyboard = machine.keyboard();
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
/// stops, streaming the console to stdout and typing the script's keys as
/// the terminal takes them.
pub fn run<S: GuestStorage>(machine: &mut VirtMachine<S>, args: &RunArgs) -> Outcome {
    let mut stdout = std::io::stdout().lock();
    let mut script = args.script.iter();
    let mut current = script.next();
    let mut window: Vec<u8> = Vec::new();
    let mut pending = String::new();
    loop {
        // Run every Type step that is now current; stop at the next Until.
        while let Some(Action::Type(text)) = current {
            pending.push_str(text);
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
        | StopReason::Unimplemented(_)
        | StopReason::Undefined(_)
        | StopReason::BusFault { .. } => ExitCode::from(1),
    }
}
