//! What per-instruction observation costs on each engine, with no devices.
//!
//! The case a lifter or a tracer actually has: a stretch of code — an `.exe`,
//! a function, a basic block — and a need to see every instruction as it
//! retires. No disk, no video, no timer; just the processor.
//!
//! Three ways to get that, timed on the same guest so the numbers compare:
//!
//! - the interpreter running free, which is the ceiling for an in-line hook,
//!   because a hook on that path is a call on a walk it was already taking;
//! - the interpreter driven one instruction per call, which is what an
//!   external stepping API costs;
//! - the hypervisor single-stepping, which on this platform means setting the
//!   trap flag and taking one VM exit per instruction.
//!
//! ```text
//! cargo run --release -p rusty_box_whp_engine --example step_bench
//! ```

use std::time::{Duration, Instant};

use rusty_box::cpu::instrumentation::{CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig, MemorySize, RunBudget, StopReason};
use rusty_box_whp::{
    GpaPerms, HostPages, LocalApicMode, PartitionConfig, Reg, ExitReason, WhpResult,
};

/// Where the guest's code goes, in both machines.
const CODE: u64 = 0x1000;

/// Iterations for the engines that run at full speed.
///
/// Sized so the run lasts over a second: at ~80 M instructions a second a
/// shorter one lands inside the clock's own resolution and reports whatever it
/// likes.
const FAST_ITERATIONS: u32 = 50_000_000;

/// Iterations for single-stepping, which is three orders of magnitude slower.
///
/// Deliberately a different number: comparing a RATE is what makes the engines
/// comparable, and giving both the same instruction count would mean either a
/// stepping run of an hour or a free run too short to time.
const STEP_ITERATIONS: u32 = 1_000_000;

/// Instructions a counted loop of `iterations` retires: the counter load, the
/// loop body twice an iteration, and the halt.
const fn instructions(iterations: u32) -> u64 {
    2 * iterations as u64 + 2
}

/// Stack for the thread that builds the interpreter machine, which is ~1.4 MB
/// and is assembled before it is boxed.
const STACK: usize = 64 * 1024 * 1024;

/// `TF`, bit 8 of the flags register: the processor takes a `#DB` after every
/// instruction while it is set.
const TRAP_FLAG: u64 = 1 << 8;

/// Bit 1 of the exception bitmap — `#DB`, the debug exception a set `TF`
/// raises. Without it the trap is delivered to the guest instead of exiting.
const DEBUG_EXCEPTION: u64 = 1 << 1;

/// The reset code segment's base, where the guest page is mapped.
const RESET_CS_BASE: u64 = 0xF_0000;

/// The guest: count down and halt, touching nothing.
///
/// ```text
///   66 B9 imm32   mov  ecx, ITERATIONS
///   66 49         dec  ecx
///   75 FC         jnz  -4
///   F4            hlt
/// ```
fn guest(iterations: u32) -> Vec<u8> {
    let n = iterations.to_le_bytes();
    vec![0x66, 0xB9, n[0], n[1], n[2], n[3], 0x66, 0x49, 0x75, 0xFC, 0xF4]
}

fn config() -> EmulatorConfig {
    EmulatorConfig {
        memory: MemorySize::bytes(8 * 1024 * 1024),
        ..EmulatorConfig::default()
    }
}

fn rate(iterations: u32, took: Duration) -> f64 {
    instructions(iterations) as f64 / took.as_secs_f64() / 1_000_000.0
}

/// A per-instruction hook that does what a lifter's would: look at the
/// instruction and record something about it.
///
/// This is the shape that matters — the hook rides the interpreter's own
/// decode loop, so it is a call on a walk already being taken rather than a
/// round trip that stops the processor.
#[derive(Default)]
struct Tracer {
    seen: u64,
    last_rip: u64,
}

impl rusty_box::cpu::instrumentation::Instrumentation for Tracer {
    fn active_hooks(&self) -> rusty_box::cpu::instrumentation::HookMask {
        rusty_box::cpu::instrumentation::HookMask::EXEC
    }

    fn before_execution(&mut self, rip: u64, _instr: &rusty_box::cpu::decoder::Instruction) {
        self.seen += 1;
        self.last_rip = rip;
    }
}

/// The interpreter, running free with a per-instruction hook installed.
fn interpreter_hooked(code: &[u8]) -> Option<Duration> {
    let mut machine = Emulator::<Tracer>::new_with_mode_and_instrumentation(
        config(),
        CpuSetupMode::RealMode,
        Tracer::default(),
    )
    .ok()?;
    machine.mem_write(CODE, code).ok()?;
    machine.reg_write(X86Reg::Rip, CODE);
    let began = Instant::now();
    loop {
        let outcome = machine.step(RunBudget::Ticks(50_000_000)).ok()?;
        if matches!(outcome.stop, StopReason::Halted) {
            return Some(began.elapsed());
        }
        if began.elapsed() > Duration::from_secs(120) {
            return None;
        }
    }
}

/// The interpreter, running free with no hook — the ceiling.
fn interpreter_free(code: &[u8]) -> Option<Duration> {
    let mut machine = Emulator::<()>::new_with_mode(config(), CpuSetupMode::RealMode).ok()?;
    machine.mem_write(CODE, code).ok()?;
    machine.reg_write(X86Reg::Rip, CODE);
    let began = Instant::now();
    loop {
        let outcome = machine.step(RunBudget::Ticks(50_000_000)).ok()?;
        if matches!(outcome.stop, StopReason::Halted) {
            return Some(began.elapsed());
        }
        if began.elapsed() > Duration::from_secs(120) {
            return None;
        }
    }
}

/// The interpreter, one instruction per call — what an external stepping API
/// costs, since each call rebuilds the execution context the free path holds.
fn interpreter_stepped(code: &[u8]) -> Option<Duration> {
    let mut machine = Emulator::<()>::new_with_mode(config(), CpuSetupMode::RealMode).ok()?;
    machine.mem_write(CODE, code).ok()?;
    machine.reg_write(X86Reg::Rip, CODE);
    let began = Instant::now();
    for _ in 0..instructions(STEP_ITERATIONS) {
        let outcome = machine.step(RunBudget::Instructions(1)).ok()?;
        if matches!(outcome.stop, StopReason::Halted) {
            break;
        }
        if began.elapsed() > Duration::from_secs(300) {
            return None;
        }
    }
    Some(began.elapsed())
}

/// The hypervisor, single-stepping: `TF` set, `#DB` exiting, one VM exit and
/// one flags rewrite per instruction.
fn hypervisor_stepped(code: &[u8]) -> WhpResult<Option<Duration>> {
    let mut config = PartitionConfig::new()?;
    config
        .processor_count(1)?
        .local_apic(LocalApicMode::None)?
        .extended_vm_exits(rusty_box_whp::ExtendedVmExits {
            exception: true,
            ..rusty_box_whp::ExtendedVmExits::default()
        })?;
    config.exception_exits(DEBUG_EXCEPTION)?;
    let mut partition = config.setup()?;

    let mut pages = HostPages::new(2)?;
    pages.bytes_mut()[CODE as usize..CODE as usize + code.len()].copy_from_slice(code);
    partition.map(RESET_CS_BASE, pages, GpaPerms::RWX)?;
    partition.create_processor(0)?;
    partition.write_reg(0, Reg::Rip, CODE)?;

    // Arm the trap flag. `#DB` clears it on delivery, so it is re-armed at
    // every stop — which is the second write this technique pays for.
    let flags = partition.read_reg(0, Reg::Rflags)?;
    partition.write_reg(0, Reg::Rflags, flags | TRAP_FLAG)?;

    let began = Instant::now();
    let mut steps = 0u64;
    loop {
        match partition.run(0)?.reason {
            ExitReason::Halt => break,
            ExitReason::Exception => {
                steps += 1;
                let flags = partition.read_reg(0, Reg::Rflags)?;
                partition.write_reg(0, Reg::Rflags, flags | TRAP_FLAG)?;
            }
            other => {
                eprintln!("unexpected exit while stepping: {other:?}");
                return Ok(None);
            }
        }
        if began.elapsed() > Duration::from_secs(300) {
            eprintln!("gave up single-stepping after {steps} steps");
            return Ok(None);
        }
    }
    Ok(Some(began.elapsed()))
}

fn main() -> std::process::ExitCode {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(bench)
        .expect("spawn")
        .join()
        .unwrap_or(std::process::ExitCode::FAILURE)
}

fn bench() -> std::process::ExitCode {
    let fast = guest(FAST_ITERATIONS);
    let slow = guest(STEP_ITERATIONS);
    println!(
        "free-running: {} instructions;  stepping: {} instructions",
        instructions(FAST_ITERATIONS),
        instructions(STEP_ITERATIONS)
    );
    println!("(different lengths on purpose — the comparable quantity is the rate)\n");

    let report = |name: &str, iterations: u32, took: Option<Duration>| match took {
        Some(took) => println!(
            "{name:<30} {:>8.2}s   {:>9.2} M instr/s",
            took.as_secs_f64(),
            rate(iterations, took)
        ),
        None => println!("{name:<30} did not finish"),
    };

    report("interpreter, free running", FAST_ITERATIONS, interpreter_free(&fast));
    report("interpreter, hook per instr", FAST_ITERATIONS, interpreter_hooked(&fast));
    report("interpreter, one call per instr", STEP_ITERATIONS, interpreter_stepped(&slow));

    if !rusty_box_whp::hypervisor_present().unwrap_or(false) {
        println!("\nskipped: this host has no Windows Hypervisor Platform");
        return std::process::ExitCode::SUCCESS;
    }

    match hypervisor_stepped(&slow) {
        Ok(took) => report("hypervisor, single-stepped", STEP_ITERATIONS, took),
        Err(error) => println!("hypervisor, single-stepped:  refused: {error}"),
    }

    std::process::ExitCode::SUCCESS
}
