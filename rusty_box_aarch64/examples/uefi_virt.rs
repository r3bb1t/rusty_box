//! Boot UEFI firmware built for QEMU's `virt` (EDK2 `ArmVirtQemu`) on the
//! rusty_box AArch64 interpreter, printing the firmware's serial console.
//!
//! ```text
//! cargo run --release -p rusty_box_aarch64 --example uefi_virt --features std -- \
//!     --code /usr/share/AAVMF/AAVMF_CODE.no-secboot.fd \
//!     --vars /usr/share/AAVMF/AAVMF_VARS.fd \
//!     --until 'Shell>' --type 'ver\r' --until 'Shell>'
//! ```
//!
//! `--until` and `--type` form a script, run in order: wait for text on the
//! console, then type keys (`\r` is Enter, `\e` Escape). The run succeeds when
//! the whole script completes.
//!
//! The variable store is copied, never written back, unless `--persist-vars`
//! names a file to save it to. The device tree defaults to the one in
//! `assets/`, dumped from QEMU 8.2 for `-M virt -cpu cortex-a57 -m 256`.

use std::io::Write;
use std::process::ExitCode;
use std::time::Instant;

use rusty_box_aarch64::cpu::DebugStops;
use rusty_box_aarch64::{RunBudget, StopReason, VirtConfig, VirtMachine, VirtMemory};
use rusty_box_core::ClockHz;

const DEFAULT_DTB: &[u8] = include_bytes!("../assets/qemu-virt-a57-256m.dtb");
const DEFAULT_RAM_MIB: usize = 256;
const FLASH_BANK: usize = 64 << 20;
const SLICE_TICKS: u64 = 4_000_000;

struct Args {
    code: String,
    vars: String,
    dtb: Option<String>,
    ram_mib: usize,
    ips: u64,
    max_ticks: u64,
    script: Vec<Action>,
    persist_vars: Option<String>,
    stop_on_undefined: bool,
    stop_on_bus_fault: bool,
    slice_ticks: u64,
}

/// One step of the console script.
enum Action {
    /// Wait until this text has appeared on the console.
    Until(Vec<u8>),
    /// Type these bytes into the UART.
    Type(Vec<u8>),
}

/// `\r` (Enter), `\n` (also Enter: a UEFI console expects CR), `\e`
/// (Escape) and `\\`; everything else as written.
fn unescape(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chars = text.bytes();
    while let Some(c) = chars.next() {
        if c != b'\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(b'r') | Some(b'n') => out.push(b'\r'),
            Some(b'e') => out.push(0x1B),
            Some(other) => out.push(other),
            None => out.push(b'\\'),
        }
    }
    out
}

fn usage() -> &'static str {
    "usage: uefi_virt --code <CODE.fd> --vars <VARS.fd> [--dtb <file>] [--ram-mib N] [--ips N]\n\
     \x20                [--max-ticks N] [--until <console text>]... [--type <keys>]...\n\
     \x20                [--persist-vars <file>] [--slice-ticks N]\n\
     \x20                [--stop-on-undefined] [--stop-on-bus-fault]"
}

/// FNV-1a over the machine's guest-visible state: RAM, the variable store,
/// and the processor registers. Equal hashes after equal tick counts are
/// what determinism means here.
fn state_hash(machine: &VirtMachine<'_>) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
    };
    eat(machine.board().ram());
    eat(machine.board().flash1_contents());
    let cpu = machine.cpu();
    for r in 0..31 {
        eat(&cpu.xreg(r).to_le_bytes());
    }
    for r in 0..32 {
        eat(&cpu.vreg(r).to_le_bytes());
    }
    eat(&cpu.sp().to_le_bytes());
    eat(&cpu.pc().to_le_bytes());
    eat(&cpu.pstate().to_spsr().to_le_bytes());
    eat(&machine.now().ticks().to_le_bytes());
    h
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        code: String::new(),
        vars: String::new(),
        dtb: None,
        ram_mib: DEFAULT_RAM_MIB,
        ips: ClockHz::BOCHS_DEFAULT.get(),
        max_ticks: u64::MAX,
        script: Vec::new(),
        persist_vars: None,
        stop_on_undefined: false,
        stop_on_bus_fault: false,
        slice_ticks: SLICE_TICKS,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--code" => a.code = value("--code")?,
            "--vars" => a.vars = value("--vars")?,
            "--dtb" => a.dtb = Some(value("--dtb")?),
            "--ram-mib" => a.ram_mib = value("--ram-mib")?.parse().map_err(|e| format!("--ram-mib: {e}"))?,
            "--ips" => a.ips = value("--ips")?.parse().map_err(|e| format!("--ips: {e}"))?,
            "--max-ticks" => a.max_ticks = value("--max-ticks")?.parse().map_err(|e| format!("--max-ticks: {e}"))?,
            "--until" => a.script.push(Action::Until(unescape(&value("--until")?))),
            "--type" => a.script.push(Action::Type(unescape(&value("--type")?))),
            "--persist-vars" => a.persist_vars = Some(value("--persist-vars")?),
            "--slice-ticks" => {
                a.slice_ticks = value("--slice-ticks")?.parse().map_err(|e| format!("--slice-ticks: {e}"))?;
                if a.slice_ticks == 0 {
                    return Err(String::from("--slice-ticks must be non-zero"));
                }
            }
            "--stop-on-undefined" => a.stop_on_undefined = true,
            "--stop-on-bus-fault" => a.stop_on_bus_fault = true,
            "-h" | "--help" => return Err(usage().to_string()),
            other => return Err(format!("unknown argument {other}\n{}", usage())),
        }
    }
    if a.code.is_empty() || a.vars.is_empty() {
        return Err(usage().to_string());
    }
    Ok(a)
}

/// A flash image padded with erased bytes to a whole bank, as QEMU requires
/// of a `pflash` drive.
fn load_flash(path: &str) -> Result<Vec<u8>, String> {
    let mut image = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if image.len() > FLASH_BANK {
        return Err(format!("{path}: {} bytes is larger than a 64 MiB flash bank", image.len()));
    }
    image.resize(FLASH_BANK, 0xFF);
    Ok(image)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let code = match load_flash(&args.code) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let mut vars = match load_flash(&args.vars) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let dtb = match &args.dtb {
        Some(p) => match std::fs::read(p) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{p}: {e}");
                return ExitCode::from(2);
            }
        },
        None => DEFAULT_DTB.to_vec(),
    };
    let Some(ips) = ClockHz::new(args.ips) else {
        eprintln!("--ips must be non-zero");
        return ExitCode::from(2);
    };
    let mut ram = vec![0u8; args.ram_mib << 20];

    let config = VirtConfig {
        ips,
        debug_stops: DebugStops {
            on_undefined: args.stop_on_undefined,
            on_bus_fault: args.stop_on_bus_fault,
        },
        ..VirtConfig::DEFAULT
    };
    let memory = VirtMemory {
        ram: &mut ram,
        flash0: &code,
        flash1: &mut vars,
        dtb: &dtb,
    };
    let mut machine = match VirtMachine::new(config, memory) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("cannot build the machine: {e:?}");
            return ExitCode::from(2);
        }
    };

    let started = Instant::now();
    let mut stdout = std::io::stdout().lock();
    // The script: wait for console text, type keys, in the order given.
    let mut script = args.script.iter();
    let mut current = script.next();
    let mut window: Vec<u8> = Vec::new();
    let mut pending_keys: std::collections::VecDeque<u8> = std::collections::VecDeque::new();
    let found = loop {
        // Run every Type step that is now current; stop at the next Until.
        while let Some(Action::Type(keys)) = current {
            pending_keys.extend(keys.iter().copied());
            current = script.next();
        }
        while let Some(&k) = pending_keys.front() {
            if !machine.send_uart(k) {
                break;
            }
            pending_keys.pop_front();
        }
        let remaining = args.max_ticks.saturating_sub(machine.now().ticks());
        if remaining == 0 {
            break (StopReason::BudgetExhausted, current.is_none());
        }
        let reason = machine.run(RunBudget::ticks(remaining.min(args.slice_ticks)));
        let out: Vec<u8> = machine.drain_uart().collect();
        if !out.is_empty() {
            if stdout.write_all(&out).and_then(|()| stdout.flush()).is_err() {
                break (reason, false);
            }
            window.extend_from_slice(&out);
        }
        if let Some(Action::Until(text)) = current {
            if window.windows(text.len()).any(|w| w == text.as_slice()) {
                window.clear();
                current = script.next();
                if current.is_none() {
                    break (reason, true);
                }
                continue;
            }
            let keep = text.len().max(1) - 1;
            if window.len() > keep {
                window.drain(..window.len() - keep);
            }
        }
        match reason {
            StopReason::BudgetExhausted | StopReason::UartOutputPending => continue,
            other => break (other, false),
        }
    };
    let (reason, found) = found;
    drop(stdout);

    let elapsed = started.elapsed().as_secs_f64();
    let ticks = machine.now().ticks();
    let cpu = machine.cpu();
    eprintln!();
    eprintln!("--- rusty_box_aarch64 ---");
    if found {
        eprintln!("console script completed");
    }
    eprintln!("stop: {reason:?}");
    eprintln!(
        "ticks: {ticks} ({:.3} s emulated) in {elapsed:.2} s host, {:.1} M ticks/s",
        ticks as f64 / args.ips as f64,
        ticks as f64 / elapsed.max(1e-9) / 1e6
    );
    eprintln!("pc: {:#x}  el: {:?}  sp: {:#x}", cpu.pc(), cpu.el(), cpu.sp());
    eprintln!("state hash: {:016x}", state_hash(&machine));
    if !matches!(reason, StopReason::BudgetExhausted | StopReason::UartOutputPending) || !found {
        let recent: Vec<String> = cpu.recent_pcs().map(|pc| format!("{pc:#x}")).collect();
        eprintln!("recent pcs: {}", recent.join(" "));
        for r in 0..31 {
            eprint!("x{r}={:#x}{}", cpu.xreg(r), if r % 4 == 3 { "\n" } else { " " });
        }
        eprintln!();
        for (name, enc) in [
            ("ESR_EL1", (3, 0, 5, 2, 0)),
            ("FAR_EL1", (3, 0, 6, 0, 0)),
            ("ELR_EL1", (3, 0, 4, 0, 1)),
            ("SCTLR_EL1", (3, 0, 1, 0, 0)),
            ("VBAR_EL1", (3, 0, 12, 0, 0)),
        ] {
            let v = cpu.read_sysreg(enc.0, enc.1, enc.2, enc.3, enc.4).unwrap_or(0);
            eprintln!("{name}={v:#x}");
        }
    }
    if let Some(path) = &args.persist_vars {
        if let Err(e) = std::fs::write(path, machine.board().flash1_contents()) {
            eprintln!("{path}: {e}");
        }
    }
    match (!args.script.is_empty(), found) {
        (true, true) => ExitCode::SUCCESS,
        (true, false) => ExitCode::from(1),
        (false, _) => match reason {
            StopReason::PoweredOff | StopReason::BudgetExhausted | StopReason::Halted => ExitCode::SUCCESS,
            _ => ExitCode::from(1),
        },
    }
}
