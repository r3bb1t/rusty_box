//! What each engine costs when the guest touches no device at all.
//!
//! The DLX boot measures the opposite case: a guest that spends almost all of
//! its time in video memory and disk ports, where every access leaves the
//! hardware and the interpreter wins because it never leaves anything. That
//! number says nothing about the case this port's precise API is FOR — a
//! snapshot fuzzer, a lifter's oracle, a stretch of shellcode — where the guest
//! runs code and nothing else.
//!
//! So this runs the same guest on both engines: a counted loop with no memory
//! traffic, no port access and no interrupt, ending in one debug-port write and
//! a halt. Identical instruction counts, so wall time is directly comparable
//! and the ratio is the answer.
//!
//! ```text
//! cargo run --release -p rusty_box_whp_engine --example compute_bench
//! ```

use std::time::{Duration, Instant};

use rusty_box::cpu::instrumentation::{CpuSetupMode, X86Reg};
use rusty_box::emulator::{
    Emulator, EmulatorConfig, MemorySize, RunBudget, SliceEngine, StopReason,
};
use rusty_box_whp_engine::WhpEngine;

/// Where the guest's code goes.
const CODE: u64 = 0x1000;

/// Iterations of the counted loop.
///
/// Two instructions an iteration, so the guest retires `2 * ITERATIONS + 3`
/// instructions whichever engine runs it. Large enough that neither engine's
/// start-up cost shows in the ratio, small enough that the interpreter finishes
/// in seconds rather than minutes.
const ITERATIONS: u32 = 50_000_000;

/// Instructions the guest retires: the counter load, the loop body, the port
/// write and the halt.
const INSTRUCTIONS: u64 = 2 * ITERATIONS as u64 + 3;

/// Ticks a single `step` may cover. Large, because this guest has nothing to
/// stop for — the engines decide their own slices underneath.
const SLICE_TICKS: u64 = 50_000_000;

/// How long a run may take before it is called a failure rather than a
/// measurement.
const PATIENCE: Duration = Duration::from_secs(300);

/// The guest: count down and halt, touching nothing.
///
/// Real mode with 32-bit operands, so one register holds the whole count:
///
/// ```text
///   66 B9 imm32   mov  ecx, ITERATIONS
///   66 49         dec  ecx
///   75 FC         jnz  -4
///   E6 E9         out  0xE9, al
///   F4            hlt
/// ```
///
/// The `out` is the only instruction that leaves the processor, and it runs
/// once — its job is to prove the guest reached the end rather than stopping
/// somewhere quiet.
fn guest() -> Vec<u8> {
    let n = ITERATIONS.to_le_bytes();
    vec![
        0x66, 0xB9, n[0], n[1], n[2], n[3], // mov ecx, ITERATIONS
        0x66, 0x49, // dec ecx
        0x75, 0xFC, // jnz -4
        0xE6, 0xE9, // out 0xE9, al
        0xF4,       // hlt
    ]
}

fn config() -> EmulatorConfig {
    EmulatorConfig {
        memory: MemorySize::bytes(8 * 1024 * 1024),
        ..EmulatorConfig::default()
    }
}

/// Run until the guest halts, and report how long that took.
///
/// A halt is the end: this guest halts once, at the end, and an engine that
/// fast-forwards past it would be measuring the machine's idle path rather
/// than the guest's work.
fn time_until_halt<E>(machine: &mut Emulator<(), E>) -> Option<Duration>
where
    E: SliceEngine<()>,
{
    let began = Instant::now();
    loop {
        let outcome = match machine.step(RunBudget::Ticks(SLICE_TICKS)) {
            Ok(outcome) => outcome,
            Err(error) => {
                eprintln!("the run ended with an error: {error}");
                return None;
            }
        };
        if matches!(outcome.stop, StopReason::Halted) {
            return Some(began.elapsed());
        }
        if outcome.is_terminal() {
            eprintln!("the guest stopped early: {:?}", outcome.stop);
            return None;
        }
        if began.elapsed() > PATIENCE {
            eprintln!("gave up after {PATIENCE:?}");
            return None;
        }
    }
}

/// Whether the guest actually reached its final port write.
///
/// Without this the benchmark would happily time a guest that faulted into a
/// loop and never finished — fast, and meaningless.
fn reached_the_end<E>(machine: &mut Emulator<(), E>) -> bool
where
    E: SliceEngine<()>,
{
    machine.debug_port().take_output().count() > 0
}

fn rate(instructions: u64, took: Duration) -> f64 {
    instructions as f64 / took.as_secs_f64() / 1_000_000.0
}

/// Stack for the thread that builds the machines.
///
/// An `Emulator` is ~1.4 MB and is assembled before it is boxed, so it is built
/// on the stack of whichever thread builds it. The main thread's is not large
/// enough, and the failure is a silent overflow rather than an error.
const STACK: usize = 64 * 1024 * 1024;

fn main() -> std::process::ExitCode {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(bench)
        .expect("spawn")
        .join()
        .unwrap_or(std::process::ExitCode::FAILURE)
}

fn bench() -> std::process::ExitCode {
    let code = guest();
    println!(
        "guest: {ITERATIONS} iterations, {INSTRUCTIONS} instructions, no device access\n"
    );

    let interpreter = {
        let mut machine = match Emulator::<()>::new_with_mode(config(), CpuSetupMode::RealMode) {
            Ok(machine) => machine,
            Err(error) => {
                eprintln!("could not build the interpreter machine: {error}");
                return std::process::ExitCode::FAILURE;
            }
        };
        if machine.mem_write(CODE, &code).is_err() {
            eprintln!("could not load the guest");
            return std::process::ExitCode::FAILURE;
        }
        machine.reg_write(X86Reg::Rip, CODE);
        let took = time_until_halt(&mut machine);
        match (took, reached_the_end(&mut machine)) {
            (Some(took), true) => took,
            (_, false) => {
                eprintln!("the interpreter's guest never reached its port write");
                return std::process::ExitCode::FAILURE;
            }
            (None, _) => return std::process::ExitCode::FAILURE,
        }
    };
    println!(
        "interpreter: {:>8.2}s   {:>8.1} M instructions/s",
        interpreter.as_secs_f64(),
        rate(INSTRUCTIONS, interpreter)
    );

    if !rusty_box_whp::hypervisor_present().unwrap_or(false) {
        println!("\nskipped: this host has no Windows Hypervisor Platform");
        return std::process::ExitCode::SUCCESS;
    }

    let hardware = {
        let mut machine =
            match Emulator::<(), WhpEngine>::with_engine(config(), CpuSetupMode::RealMode) {
                Ok(machine) => machine,
                Err(error) => {
                    eprintln!("could not build the hypervisor machine: {error}");
                    return std::process::ExitCode::FAILURE;
                }
            };
        if machine.mem_write(CODE, &code).is_err() {
            eprintln!("could not load the guest");
            return std::process::ExitCode::FAILURE;
        }
        machine.reg_write(X86Reg::Rip, CODE);
        let took = time_until_halt(&mut machine);
        match (took, reached_the_end(&mut machine)) {
            (Some(took), true) => took,
            (_, false) => {
                eprintln!("the hypervisor's guest never reached its port write");
                return std::process::ExitCode::FAILURE;
            }
            (None, _) => return std::process::ExitCode::FAILURE,
        }
    };
    println!(
        "hypervisor:  {:>8.2}s   {:>8.1} M instructions/s",
        hardware.as_secs_f64(),
        rate(INSTRUCTIONS, hardware)
    );

    println!(
        "\nthe hypervisor is {:.1}x the interpreter on a guest that touches no device",
        interpreter.as_secs_f64() / hardware.as_secs_f64()
    );
    std::process::ExitCode::SUCCESS
}
