//! Alpine Linux booting on the hypervisor engine, driven through `step()`,
//! with the engine's own census beside the guest's own clock.
//!
//! The measurement harness for this engine's exit economy. It assembles the
//! machine `rusty_box_gui`'s runner assembles for
//! `--engine whp --cpu-capabilities host-shared --ips 300000000 --cdrom <iso>
//! --boot cdrom --memory-mib 256 --host-memory-mib 256`, steps it in a tight
//! loop with no display rendering and no input pump, and reports:
//!
//! - `ticks`: the machine's own clock (the sum of each step's progress), the
//!   guest-seconds it amounts to at 300 M ticks per second, and that figure
//!   per wall-second;
//! - the engine's `exits()`, `census()` and `inject_census()`, and the
//!   platform's own intercept and runtime counters — the independent account
//!   that audits the engine's;
//! - the wall and engine time at which each screen milestone first appears,
//!   and the newest kernel `[ t ]` stamp on screen with the time it was seen
//!   at (the kernel's stamps are hardware-TSC time scaled by a boot-time
//!   calibration, NOT the engine's clock — reported so the two are never
//!   confused).
//!
//! Driven through `step()` rather than a display loop on purpose: the
//! interactive path is measured 2–4x slower than this and erratic within a
//! single build, so a regression gate that reads it is noisy. This is the
//! instrument for a before/after comparison; run the arms interleaved, from
//! separate binaries, with the same ISO.
//!
//! ```text
//! ALPINE_ISO=alpine-virt-3.24.1-x86_64.iso \
//!   cargo run --release -p rusty_box_whp_engine --example alpine_probe
//! ```
//!
//! `ALPINE_PROBE_PATIENCE_SECS` bounds the run in host time (default 300).
//! Exits 77 when the host has no hypervisor.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rusty_box::cpu::decoder::features::X86Feature;
use rusty_box::emulator::{
    AtaSlot, BootDevice, BootOrder, EmulatorConfig, Ips, MachineBuilder, MemorySize, RunBudget,
    StopReason,
};
use rusty_box::params::BxParams;
use rusty_box::CpuidFreq;
use rusty_box_whp_engine::WhpEngine;

const MIB: usize = 1024 * 1024;
const IPS: u32 = 300_000_000;
/// The same outer step the DLX example uses; the inner batch is capped at
/// 100 000 ticks by `run_until_budget_spent`, the same cap the interactive
/// loop's `INSTRUCTION_BATCH_SIZE` imposes, so the engine sees the same slice
/// shape the GUI gives it.
const SLICE_TICKS: u64 = 20_000_000;

/// The exit code a harness reads as "this host cannot run this".
const SKIPPED: i32 = 77;

/// Screen needles, each recorded the first time it is seen. The last one
/// ends the run.
const MILESTONES: &[&str] = &[
    "ISOLINUX",
    "Linux version",
    "Freeing unused kernel",
    "OpenRC",
    "Starting",
    "Welcome to Alpine",
    "login:",
];

fn patience() -> Duration {
    match std::env::var("ALPINE_PROBE_PATIENCE_SECS").ok().and_then(|s| s.parse().ok()) {
        Some(secs) => Duration::from_secs(secs),
        None => Duration::from_secs(300),
    }
}

fn workspace_root() -> Option<PathBuf> {
    let mut at: PathBuf = std::env::current_dir().ok()?;
    loop {
        if at.join("Cargo.toml").exists() && at.join("rusty_box").is_dir() {
            return Some(at);
        }
        if !at.pop() {
            return None;
        }
    }
}

/// `CpuCapabilities::HostShared::narrow` from `rusty_box_gui/src/config.rs`,
/// in effect: drop AVX-512 / AVX when the host cannot carry their XSAVE
/// state, and MONITOR/MWAIT unconditionally.
fn host_shared(params: BxParams) -> BxParams {
    let reported = core::arch::x86_64::__cpuid_count(0xD, 0);
    let carried = (u64::from(reported.edx) << 32) | u64::from(reported.eax);
    const AVX512_STATE: u64 = (1 << 5) | (1 << 6) | (1 << 7);
    const AVX_STATE: u64 = 1 << 2;
    let mut narrowed = params;
    if carried & AVX512_STATE != AVX512_STATE {
        narrowed = narrowed.excluding(X86Feature::IsaAvx512);
    }
    if carried & AVX_STATE == 0 {
        narrowed = narrowed.excluding(X86Feature::IsaAvx);
    }
    narrowed.excluding(X86Feature::IsaMonitorMwait)
}

/// A kernel console line's `[ seconds.micros]` prefix, if it has one.
fn dmesg_stamp(line: &str) -> Option<f64> {
    let rest = line.trim_start().strip_prefix('[')?;
    let end = rest.find(']')?;
    let inner = rest[..end].trim();
    if !inner.contains('.') {
        return None;
    }
    inner.parse::<f64>().ok()
}

fn main() {
    // The machine is ~1.4 MiB and the boot walks deep call chains; the default
    // main thread is not sized for it.
    let run = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .name("alpine probe".into())
        .spawn(probe)
        .expect("spawn");
    std::process::exit(run.join().unwrap_or(1));
}

fn probe() -> i32 {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(true)
        .init();

    match rusty_box_whp::hypervisor_present() {
        Ok(true) => {}
        other => {
            println!("skipped: no hypervisor ({other:?})");
            return SKIPPED;
        }
    }
    let Some(root) = workspace_root() else {
        eprintln!("not inside a workspace");
        return 1;
    };
    let Ok(iso) = std::env::var("ALPINE_ISO") else {
        eprintln!("set ALPINE_ISO");
        return 1;
    };
    let bios = match std::fs::read(root.join("cpp_orig/bochs/bochs/bios/BIOS-bochs-latest")) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("no BIOS image: {error}");
            return 1;
        }
    };
    let vga = match std::fs::read(
        root.join("cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin"),
    ) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("no VGA BIOS image: {error}");
            return 1;
        }
    };

    let topology = match BxParams::default().with_topology(1, 1, 1) {
        Ok(params) => params,
        Err(error) => {
            eprintln!("topology: {error:?}");
            return 1;
        }
    };
    let config = EmulatorConfig {
        memory: MemorySize::partially_resident(256 * MIB, 256 * MIB),
        memory_block_size: 128 * 1024,
        ips: Ips::new(IPS),
        pci_enabled: true,
        pci_vga: false,
        sync_slowdown: false,
        sync_realtime: false,
        smp_quantum: 16,
        cpuid_freq: CpuidFreq::None,
        cpu_params: host_shared(topology),
        ..EmulatorConfig::default()
    };
    let builder = MachineBuilder::new(config)
        .bios(&bios)
        .vga_bios(&vga)
        .boot_order(BootOrder::just(BootDevice::Cdrom))
        .cdrom_file(AtaSlot::SECONDARY_MASTER, &iso);
    let mut machine = match builder.build_on::<WhpEngine>() {
        Ok(machine) => machine,
        Err(error) => {
            eprintln!("could not assemble the machine: {error}");
            return 1;
        }
    };
    // The runner queues one Enter for a CD-first boot order; so does this.
    machine.prepare_run();
    let typed = machine.keyboard().type_text("\n");
    println!("probe: iso={iso} patience={:?} prequeued_keys={typed}", patience());

    let began = Instant::now();
    let limit = patience();
    let mut ticks = 0u64;
    let mut seen: Vec<Option<(f64, u64)>> = vec![None; MILESTONES.len()];
    // (stamp, wall seconds, ticks) of the newest kernel timestamp seen on screen.
    let mut newest_stamp: Option<(f64, f64, u64)> = None;
    let mut next_report = 10u64;
    let mut ending = String::from("login reached");
    loop {
        let outcome = match machine.step(RunBudget::Ticks(SLICE_TICKS)) {
            Ok(outcome) => outcome,
            Err(error) => {
                ending = format!("run error: {error}");
                break;
            }
        };
        ticks = ticks.saturating_add(outcome.progress.count());
        let wall = began.elapsed().as_secs_f64();

        if let Some(screen) = machine.display().text().map(|text| text.to_text()) {
            for (index, needle) in MILESTONES.iter().enumerate() {
                if seen[index].is_none() && screen.contains(needle) {
                    seen[index] = Some((wall, ticks));
                    println!(
                        "milestone {needle:?}: wall {wall:.1}s ticks {ticks} guest {:.2}s",
                        ticks as f64 / IPS as f64
                    );
                }
            }
            for line in screen.lines() {
                if let Some(stamp) = dmesg_stamp(line) {
                    if newest_stamp.is_none_or(|(newest, _, _)| stamp > newest) {
                        newest_stamp = Some((stamp, wall, ticks));
                    }
                }
            }
        }

        if began.elapsed().as_secs() >= next_report {
            next_report += 10;
            let exits = machine.engine().exits();
            let census = machine.engine().census();
            let inject = machine.engine().inject_census();
            println!(
                "t={wall:.1}s ticks={ticks} guest={:.2}s ratio={:.2} slices={} injected={} windows_armed={} exits={exits:?} dmesg_max={:?}",
                ticks as f64 / IPS as f64,
                (ticks as f64 / IPS as f64) / wall,
                census.slices,
                inject.injected,
                inject.windows_armed,
                newest_stamp.map(|(s, _, _)| s),
            );
        }

        if seen.last().is_some_and(Option::is_some) {
            break;
        }
        if outcome.is_terminal() {
            ending = format!("guest stopped: {:?}", outcome.stop);
            break;
        }
        if matches!(outcome.stop, StopReason::Halted) && outcome.progress.stalled() {
            ending = "halted with nothing to wake it".into();
            break;
        }
        if began.elapsed() > limit {
            ending = format!("patience {limit:?} exhausted");
            break;
        }
    }

    let wall = began.elapsed().as_secs_f64();
    let guest = ticks as f64 / IPS as f64;
    let exits = machine.engine().exits();
    let census = machine.engine().census();
    let inject = machine.engine().inject_census();
    println!();
    println!("RESULT ending={ending:?}");
    println!(
        "RESULT wall_s={wall:.2} ticks={ticks} guest_s={guest:.3} guest_per_wall={:.3}",
        guest / wall
    );
    for (index, needle) in MILESTONES.iter().enumerate() {
        match seen[index] {
            Some((at, at_ticks)) => println!(
                "RESULT milestone {needle:?} wall={at:.1} guest={:.2}",
                at_ticks as f64 / IPS as f64
            ),
            None => println!("RESULT milestone {needle:?} NOT reached"),
        }
    }
    match newest_stamp {
        Some((stamp, at, at_ticks)) => println!(
            "RESULT dmesg_max={stamp:.3} seen_at_wall={at:.1} seen_at_guest={:.2}",
            at_ticks as f64 / IPS as f64
        ),
        None => println!("RESULT dmesg_max=none"),
    }
    println!("RESULT exits={exits:?}");
    println!("RESULT census={census:?}");
    println!("RESULT inject injected={} windows_armed={}", inject.injected, inject.windows_armed);
    match machine.engine().platform_counters() {
        Ok(counters) => {
            let intercepts = &counters.intercepts;
            println!(
                "RESULT platform io={} npf={} other={} cpuid={} msr={} runtime_total_ms={} hypervisor_ms={}",
                intercepts.io_instructions.count,
                intercepts.nested_page_fault_intercepts.count,
                intercepts.other_intercepts.count,
                intercepts.cpuid_instructions.count,
                intercepts.msr_accesses.count,
                counters.runtime.total_100ns / 10_000,
                counters.runtime.hypervisor_100ns / 10_000,
            );
        }
        Err(error) => println!("RESULT platform counters unavailable: {error}"),
    }
    println!("RIP = {:#x}", machine.rip());
    if let Some(text) = machine.display().text() {
        println!("screen:");
        for line in text.to_text().lines() {
            let line = line.trim_end();
            if !line.is_empty() {
                println!("  | {line}");
            }
        }
    }
    0
}
