//! Warm CPU-only timing and checked code-patch/refetch timing.

use std::error::Error;
use std::time::Instant;

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::Instruction;
use rusty_box::cpu::instrumentation::{HookMask, Instrumentation};
use rusty_box::cpu::memory_session::{CpuEmulator, SessionStop};
use rusty_box::cpu::registers::EAX;

const CODE: BxPhyAddress = BxPhyAddress::new(0x1000);
const LOOP: &[u8] = &[0x40, 0x31, 0xDB, 0x43, 0xEB, 0xFA]; // inc eax; xor ebx,ebx; inc ebx; jmp
const PATCHABLE: &[u8] = &[0xB8, 0, 0, 0, 0, 0xEB, 0xF9]; // mov eax,0; jmp
const WARMUP: u64 = 4096;
type BenchResult<T> = Result<T, Box<dyn Error>>;

#[derive(Default)]
struct Counter {
    enabled: bool,
    seen: u64,
}

impl Instrumentation for Counter {
    fn active_hooks(&self) -> HookMask {
        if self.enabled {
            HookMask::EXEC
        } else {
            HookMask::empty()
        }
    }

    fn before_execution(&mut self, _rip: u64, _instruction: &Instruction) {
        self.seen += 1;
    }
}

fn run<T: Instrumentation>(cpu: &mut CpuEmulator<T>, counts: u64) {
    let result = cpu.run(counts);
    assert!(matches!(result.stop, SessionStop::Budget));
    assert_eq!(result.instructions, counts);
}

fn alu(counts: u64, enabled: bool) -> BenchResult<f64> {
    let mut cpu = CpuEmulator::builder().build_with_tracer(Counter { enabled, seen: 0 })?;
    cpu.memory().write_phys(CODE, LOOP)?;
    cpu.regs().write(EAX, 0);
    run(&mut cpu, WARMUP);
    let start = Instant::now();
    run(&mut cpu, counts);
    let elapsed = start.elapsed();
    assert_eq!(cpu.regs().read(EAX), ((WARMUP + counts) / 4) as u32);
    assert_eq!(
        cpu.instrumentation().seen,
        if enabled { WARMUP + counts } else { 0 }
    );
    Ok(elapsed.as_nanos() as f64 / counts as f64)
}

fn refetch(batches: u64, patch: bool) -> BenchResult<f64> {
    let mut cpu = CpuEmulator::builder().build()?;
    cpu.memory().write_phys(CODE, PATCHABLE)?;
    run(&mut cpu, WARMUP);
    let start = Instant::now();
    for index in 0..batches {
        let expected = if patch { (index as u8) ^ 0xA5 } else { 0 };
        if patch {
            cpu.memory().write_phys(CODE.wrapping_add(1), &[expected])?;
        }
        run(&mut cpu, 2);
        assert_eq!(cpu.regs().read(EAX), u32::from(expected));
    }
    Ok(start.elapsed().as_nanos() as f64 / batches as f64)
}

fn main() -> BenchResult<()> {
    let counts = std::env::var("RB_CPU_COUNTS")
        .ok()
        .map_or(Ok(40_000_000), |value| value.parse::<u64>())?;
    let batches = std::env::var("RB_CACHE_BATCHES")
        .ok()
        .map_or(Ok(50_000), |value| value.parse::<u64>())?;
    if counts == 0 || counts % 4 != 0 || counts > (u64::from(u32::MAX) * 4 - WARMUP) || batches == 0
    {
        return Err("use a positive multiple of four within the EAX counter range and a positive batch count".into());
    }

    let mut samples = [[0.0; 7]; 4];
    for round in 0..7 {
        for position in 0..samples.len() {
            let mode = (round + position) % samples.len();
            samples[mode][round] = match mode {
                0 => alu(counts, false)?,
                1 => alu(counts, true)?,
                2 => refetch(batches, false)?,
                _ => refetch(batches, true)?,
            };
        }
    }
    for ((name, unit), values) in [
        ("ALU, hooks disabled", "count"),
        ("ALU, counting hook", "count"),
        ("warm fetch", "two-count batch"),
        ("checked patch and refetch", "two-count batch"),
    ]
    .into_iter()
    .zip(&mut samples)
    {
        values.sort_unstable_by(f64::total_cmp);
        println!(
            "{name}: {:.2} ns/{unit} [{:.2}..{:.2}] {values:?}",
            values[3], values[0], values[6]
        );
    }
    Ok(())
}
