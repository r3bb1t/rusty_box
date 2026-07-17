//! TEMPORARY CPU-core microbenchmark harness (perf investigation only).
//!
//! Runs a canonical hot loop through the full `cpu_loop` execution path in
//! paging-on long mode (FlatLong64 identity map). No disk/BIOS required, so it
//! is reproducible on any checkout and is a clean, profileable target for
//! `samply` / flamegraph.
//!
//! The loop body mixes ALU, a memory write, a memory read, and a
//! `dec`/`jnz` pair — the canonical lazy-flag ZF hot pattern that dominates
//! real guest inner loops.
//!
//! Run:
//!   PERFBENCH_INSN=500000000 cargo run --release --example perfbench --features std
//!   samply record ./target/release/examples/perfbench

use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, CpuSetupMode, ResetReason, X86Reg},
    emulator::{Emulator, EmulatorConfig},
};
use std::time::Instant;

const GUEST_RAM: usize = 128 * 1024 * 1024;
const CODE_BASE: u64 = 0x0040_0000;
const DATA_ADDR: u64 = 0x0050_0000;
const STACK_TOP: u64 = 0x07FF_FF00;

// Several loop shapes, selected by PERFBENCH_MODE, so we can attribute the
// per-instruction cost by subtraction. Every loop is counted by RCX and ends
// in `dec rcx; jnz loop` unless noted. Counter in RCX, data ptr in RBX.

// MODE=mixed (default): 5 ALU + 1 store + 1 load + dec/jnz  (7 insns/iter)
#[rustfmt::skip]
static LOOP_MIXED: &[u8] = &[
    0x48, 0x01, 0xC8,       // add rax, rcx
    0x48, 0x31, 0xD2,       // xor rdx, rdx
    0x48, 0x89, 0x03,       // mov [rbx], rax
    0x48, 0x8B, 0x33,       // mov rsi, [rbx]
    0x48, 0x01, 0xF0,       // add rax, rsi
    0x48, 0xFF, 0xC9,       // dec rcx
    0x75, 0xEC,             // jnz loop  (-20)
];

// MODE=alu: same instruction count as mixed but NO memory ops (2 stores/loads
// replaced by ALU). Subtract from `mixed` to isolate memory-access cost.
#[rustfmt::skip]
static LOOP_ALU: &[u8] = &[
    0x48, 0x01, 0xC8,       // add rax, rcx
    0x48, 0x31, 0xD2,       // xor rdx, rdx
    0x48, 0x01, 0xC2,       // add rdx, rax
    0x48, 0x01, 0xD6,       // add rsi, rdx
    0x48, 0x01, 0xF0,       // add rax, rsi
    0x48, 0xFF, 0xC9,       // dec rcx
    0x75, 0xEC,             // jnz loop  (-20)
];

// MODE=branch: 1 ALU + dec/jnz (3 insns/iter). Trace re-lookup every 3 insns.
// vs `straight` isolates icache-lookup + taken-branch overhead per trace.
#[rustfmt::skip]
static LOOP_BRANCH: &[u8] = &[
    0x48, 0x01, 0xC8,       // add rax, rcx
    0x48, 0xFF, 0xC9,       // dec rcx
    0x75, 0xF8,             // jnz loop  (-8)
];

// MODE=straight: 30 ALU + dec/jnz (31 insns/iter). Long trace amortizes the
// icache lookup — best-case per-instruction dispatch/execute cost.
#[rustfmt::skip]
static LOOP_STRAIGHT: &[u8] = &[
    0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8,
    0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8,
    0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8,
    0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8,
    0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8,
    0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8, 0x48, 0x01, 0xC8,
    0x48, 0xFF, 0xC9,       // dec rcx
    0x75, 0xA1,             // jnz loop  (-95)
];

fn select_loop() -> (&'static str, &'static [u8], u64) {
    // returns (name, code, insns_per_iter)
    match std::env::var("PERFBENCH_MODE").as_deref() {
        Ok("alu") => ("alu", LOOP_ALU, 7),
        Ok("branch") => ("branch", LOOP_BRANCH, 3),
        Ok("straight") => ("straight", LOOP_STRAIGHT, 32),
        _ => ("mixed", LOOP_MIXED, 7),
    }
}

fn main() {
    // Big stack like the other examples (long-mode page tables + emulator).
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(run)
        .expect("spawn")
        .join()
        .expect("join");
}

fn run() {
    let insns: u64 = std::env::var("PERFBENCH_INSN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500_000_000);

    let cfg = EmulatorConfig {
        guest_memory_size: GUEST_RAM,
        host_memory_size: GUEST_RAM,
        ips: 1_000_000_000,
        pci_enabled: false,
        ..EmulatorConfig::default()
    };

    let mut emu = Emulator::<Corei7SkylakeX>::new(cfg.clone()).expect("new");
    emu.memory
        .init_memory(cfg.guest_memory_size, cfg.host_memory_size, cfg.memory_block_size)
        .expect("init_memory");
    emu.memory.set_a20_mask(emu.pc_system.a20_mask());
    emu.pc_system.initialize(cfg.ips);
    unsafe { emu.cpu_mut_unchecked() }.reset(ResetReason::Hardware);
    emu.setup_cpu_mode(CpuSetupMode::FlatLong64).expect("mode");

    let (mode, code, _ipi) = select_loop();
    emu.mem_write(CODE_BASE, code).expect("write code");
    emu.reg_write(X86Reg::Rsp, STACK_TOP);
    emu.reg_write(X86Reg::Rbx, DATA_ADDR);
    // Counter high enough it never reaches zero within the budget.
    emu.reg_write(X86Reg::Rcx, 0x0000_FFFF_FFFF_FFFF);

    println!("perfbench[{mode}]: running {insns} instructions (FlatLong64, paging on)...");
    let start = Instant::now();
    let reason = emu
        .emu_start(CODE_BASE, None, None, Some(insns))
        .expect("emu_start");
    let elapsed = start.elapsed();

    let mips = insns as f64 / elapsed.as_secs_f64() / 1_000_000.0;
    let ns_per = elapsed.as_secs_f64() * 1e9 / insns as f64;
    println!(
        "perfbench[{mode}]: {reason:?} | {insns} insns in {:.3}s = {mips:.1} MIPS ({ns_per:.2} ns/insn)",
        elapsed.as_secs_f64()
    );
}
