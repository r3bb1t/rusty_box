//! Boot DLX Linux with the guest running on the host's hypervisor.
//!
//! The counterpart to `rusty_box`'s `dlxlinux` example, on the other engine.
//! Everything about the machine is the same — same BIOS, same VGA BIOS, same
//! disk image, same 32 MiB and same instructions-per-second — because the
//! whole point is that only the engine differs.
//!
//! ## Why it reports sub-milestones
//!
//! A boot that stops at the SMM handshake and one that stops waiting for its
//! first timer tick look identical from outside: a machine that keeps running
//! and never prints a login prompt. So this walks the guest's own text screen
//! and says which milestone it reached and which it did not. A failure that
//! names its stage is worth more than a gate that only says no.
//!
//! Exits 77 when the host has no hypervisor, which is the convention a test
//! harness reads as "skipped" rather than "failed".

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusty_box::emulator::{
    AtaSlot, BootDevice, BootOrder, DiskGeometry, EmulatorConfig, Ips, MachineBuilder,
    MemorySize, RunBudget, StopReason,
};
use rusty_box_whp_engine::{SliceCensus, WhpEngine};

/// DLX Linux disk geometry, from `bochsrc.bxrc`.
const DLX_CYLINDERS: u16 = 306;
const DLX_HEADS: u8 = 4;
const DLX_SPT: u8 = 17;

/// The exit code a harness reads as "this host cannot run this".
const SKIPPED: i32 = 77;

/// How much guest time one step is allowed to cover.
///
/// Guest time rather than instructions: a hardware engine retires instructions
/// this port never sees and reports the time that passed instead, so an
/// instruction budget is the one thing this machine cannot be asked for.
const SLICE_TICKS: u64 = 20_000_000;

/// How long the whole boot may take in HOST time before it is called stuck.
///
/// `DLX_WHP_PATIENCE_SECS` overrides it, because measuring a timing change
/// means running this repeatedly and a full-length boot is six minutes of
/// waiting per experiment.
fn patience() -> Duration {
    match std::env::var("DLX_WHP_PATIENCE_SECS").ok().and_then(|secs| secs.parse().ok()) {
        Some(secs) => Duration::from_secs(secs),
        None => Duration::from_secs(360),
    }
}

/// What the guest must put on its own screen, in order.
///
/// Read off the text plane rather than a serial port, because the VGA console
/// is what a person would be looking at and because every one of these
/// characters reached the screen through an MMIO exit serviced by the shadow.
const MILESTONES: &[(&str, &str)] = &[
    ("the BIOS is alive", "BIOS"),
    ("the boot loader has the disk", "LILO"),
    ("the kernel is up", "login:"),
];

fn main() {
    // The machine is ~1.4 MiB and the boot walks deep call chains; the default
    // main thread is not sized for it.
    let run = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .name("dlx on whp".into())
        .spawn(boot)
        .expect("spawn");
    std::process::exit(run.join().unwrap_or(1));
}

fn boot() -> i32 {
    // The engine narrates what it services and what it refuses; `RUST_LOG`
    // decides whether anyone is listening. `RUST_LOG=rusty_box_whp_engine=trace`
    // is the setting that says which exits a stuck boot was taking.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(true)
        .init();

    match rusty_box_whp::hypervisor_present() {
        Ok(true) => {}
        Ok(false) => {
            println!("skipped: this host has no Windows Hypervisor Platform");
            return SKIPPED;
        }
        Err(error) => {
            println!("skipped: asking for the platform failed: {error}");
            return SKIPPED;
        }
    }

    let Some(root) = workspace_root() else {
        eprintln!("could not find the workspace root from the current directory");
        return 1;
    };
    let bios = match std::fs::read(root.join("cpp_orig/bochs/bochs/bios/BIOS-bochs-latest")) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("no BIOS image: {error}");
            return 1;
        }
    };
    let vga_bios = std::fs::read(
        root.join("cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin"),
    )
    .ok();
    let disk = root.join("dlxlinux/hd10meg.img");
    if !disk.exists() {
        eprintln!("no DLX disk image at {}", disk.display());
        return 1;
    }

    let config = EmulatorConfig {
        memory: MemorySize::bytes(32 * 1024 * 1024),
        memory_block_size: 128 * 1024,
        ips: Ips::new(300_000_000),
        pci_enabled: true,
        ..EmulatorConfig::default()
    };

    let disk_path = disk.to_string_lossy().to_string();
    let mut builder = MachineBuilder::new(config)
        .bios(&bios)
        .boot_order(BootOrder::just(BootDevice::Disk))
        .disk_file(
            AtaSlot::PRIMARY_MASTER,
            &disk_path,
            DiskGeometry::new(DLX_CYLINDERS.into(), DLX_HEADS, DLX_SPT),
        );
    if let Some(vga) = vga_bios.as_deref() {
        builder = builder.vga_bios(vga);
    }

    // The one line that differs from the software example.
    let mut machine = match builder.build_on::<WhpEngine>() {
        Ok(machine) => machine,
        Err(error) => {
            eprintln!("could not assemble the machine: {error}");
            return 1;
        }
    };
    println!("machine assembled on the hypervisor engine; running");

    // A watchdog on its own thread, because the failure worth catching is the
    // one where the machine does not come back: a progress line printed by the
    // loop says nothing when the loop is the thing that stopped. This counts
    // completed steps from outside, so "slow" and "wedged" stop looking alike.
    let steps = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let watched = std::sync::Arc::clone(&steps);
    std::thread::Builder::new()
        .name("watchdog".into())
        .spawn(move || {
            let mut previous = 0u64;
            for second in 1.. {
                std::thread::sleep(Duration::from_secs(1));
                let now = watched.load(std::sync::atomic::Ordering::Relaxed);
                println!(
                    "  {second:>3}s  {} steps (+{})",
                    now,
                    now.saturating_sub(previous)
                );
                previous = now;
            }
        })
        .expect("watchdog");

    let began = Instant::now();
    let patience = patience();
    let mut reached = 0usize;
    let mut ticks = 0u64;
    let mut last_said = u64::MAX;
    // The guest's screen as last reported, so only what changed is printed.
    let mut shown = String::new();
    loop {
        let outcome = match machine.step(RunBudget::Ticks(SLICE_TICKS)) {
            Ok(outcome) => outcome,
            Err(error) => {
                report(
                    reached,
                    began,
                    ticks,
                    &machine,
                );
                eprintln!("the run ended with an error: {error}");
                return 1;
            }
        };
        ticks = ticks.saturating_add(outcome.progress.count());
        steps.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Show the guest's own screen as it fills. Every character here got
        // there through an MMIO exit serviced on the shadow processor, so this
        // is not a convenience — it is the only account of what the guest
        // believes it is doing, and counters cannot substitute for it.
        if let Some(screen) = machine.display().text().map(|text| text.to_text()) {
            if screen != shown {
                for (line, was) in screen.lines().zip(shown.lines().chain(std::iter::repeat("")))
                {
                    let line = line.trim_end();
                    if line != was.trim_end() && !line.is_empty() {
                        println!("│ {line}");
                    }
                }
                shown = screen;
            }
        }
        // A boot that is merely slow and one that is stuck look the same from
        // outside, so say what the guest has been exiting for as it goes.
        //
        // And how those exits were divided into slices, which the counts alone
        // cannot say: the same ten thousand exits are one cost spread over a
        // hundred slices and quite another spread over ten thousand, because a
        // slice buys a VM entry, a VM exit and two architectural state
        // exchanges whatever it then does with them.
        if began.elapsed().as_secs() != last_said {
            last_said = began.elapsed().as_secs();
            let exits = machine.engine().exits();
            let census = machine.engine().census();
            println!(
                "       rip={:#x} exits: port {} mem {} cpuid {} msr {} halt {} boundary {}",
                machine.rip(),
                exits.port,
                exits.memory,
                exits.cpuid,
                exits.msr,
                exits.halt,
                exits.boundary,
            );
            println!("       {}", one_line_census(&census));
        }

        if let Some(screen) = machine.display().text().map(|text| text.to_text()) {
            while reached < MILESTONES.len() {
                let (name, needle) = MILESTONES[reached];
                if !screen.contains(needle) {
                    break;
                }
                println!(
                    "  reached: {name}  ({:.1}s, {} Mticks)",
                    began.elapsed().as_secs_f64(),
                    ticks / 1_000_000
                );
                reached += 1;
            }
        }
        if reached == MILESTONES.len() {
            println!("*** LOGIN DETECTED (WHP) ***");
            // Reported on the way OUT, not only on the way down. The census of
            // a boot that succeeded is the measurement this example exists to
            // produce; printing it only on the failure paths would mean the
            // one run worth measuring is the one that says nothing.
            report(reached, began, ticks, &machine);
            return 0;
        }

        if outcome.is_terminal() {
            report(
                reached,
                began,
                ticks,
                &machine,
            );
            eprintln!("the guest stopped: {:?}", outcome.stop);
            return 1;
        }
        if began.elapsed() > patience {
            report(
                reached,
                began,
                ticks,
                &machine,
            );
            eprintln!("gave up after {patience:?} of host time");
            return 1;
        }
        // A machine whose processor is halted with nothing able to wake it is
        // not going to become unstuck by being asked again.
        if matches!(outcome.stop, StopReason::Halted) && outcome.progress.stalled() {
            report(
                reached,
                began,
                ticks,
                &machine,
            );
            eprintln!("the guest halted with nothing left to wake it");
            dump(&mut machine);
            return 1;
        }
    }
}

/// Say how far the guest got and, more usefully, what it was doing.
fn report(
    reached: usize,
    began: Instant,
    ticks: u64,
    machine: &rusty_box::emulator::Emulator<(), WhpEngine>,
) {
    let exits = machine.engine().exits();
    let census = machine.engine().census();
    println!();
    println!(
        "got {} of {} milestones in {:.1}s of host time and {} Mticks of guest time",
        reached,
        MILESTONES.len(),
        began.elapsed().as_secs_f64(),
        ticks / 1_000_000
    );
    for (index, (name, needle)) in MILESTONES.iter().enumerate() {
        let mark = if index < reached { "reached" } else { "NOT reached" };
        println!("  {mark}: {name}  (looking for {needle:?})");
    }
    println!(
        "  exits: port {} mem {} cpuid {} msr {} halt {} canceled {} boundary {}",
        exits.port,
        exits.memory,
        exits.cpuid,
        exits.msr,
        exits.halt,
        exits.canceled,
        exits.boundary,
    );
    println!("  {}", one_line_census(&census));
    // The hypervisor's own account, beside this engine's. Printed even when it
    // is unavailable: "the platform would not say" is itself a result, and
    // silently omitting the independent check would leave a reader believing
    // the two agreed.
    match machine.engine().platform_counters() {
        Ok(counters) => {
            let intercepts = &counters.intercepts;
            println!(
                "  platform: io {} npf {} other {} cpuid {} msr {} halt_time_100ns {}",
                intercepts.io_instructions.count,
                intercepts.nested_page_fault_intercepts.count,
                intercepts.other_intercepts.count,
                intercepts.cpuid_instructions.count,
                intercepts.msr_accesses.count,
                intercepts.halt_instructions.time_100ns,
            );
            println!(
                "  runtime: total {}ms hypervisor {}ms",
                counters.runtime.total_100ns / 10_000,
                counters.runtime.hypervisor_100ns / 10_000,
            );
        }
        Err(error) => println!("  platform counters unavailable: {error}"),
    }
}

/// The census as one line: how many slices, how many exits each held, and what
/// ended them.
///
/// The histogram's buckets are named from [`SliceCensus::BUCKET_LABELS`] rather
/// than written out here, so a reader and the engine cannot describe different
/// ranges with the same numbers.
fn one_line_census(census: &SliceCensus) -> String {
    let histogram: Vec<String> = SliceCensus::BUCKET_LABELS
        .iter()
        .zip(census.exits_per_slice.iter())
        .map(|(label, slices)| format!("{label}={slices}"))
        .collect();
    format!(
        "slices: {} [exits/slice {}] ended: halted {} canceled {} budget {} \
         boundary(processor {} device {} event {})",
        census.slices,
        histogram.join(" "),
        census.ended_halted,
        census.ended_canceled,
        census.ended_budget,
        census.ended_wants_machine_boundary,
        census.ended_needs_boundary,
        census.ended_event_to_deliver,
    )
}

/// Show what the guest was doing when it stopped.
///
/// The screen first, because that is what a person would have been looking at,
/// and it says which part of the firmware got as far as printing. Then where
/// the processor is, which distinguishes "stopped in the BIOS" from "stopped
/// somewhere this port put it".
fn dump(machine: &mut rusty_box::emulator::Emulator<(), WhpEngine>) {
    println!();
    println!("RIP = {:#x}", machine.rip());
    match machine.display().text() {
        Some(text) => {
            let screen = text.to_text();
            println!("the guest's text screen:");
            for line in screen.lines() {
                let line = line.trim_end();
                if !line.is_empty() {
                    println!("  | {line}");
                }
            }
            if screen.trim().is_empty() {
                println!("  (blank — nothing has been printed)");
            }
        }
        None => println!("the display is not in a text mode"),
    }
    let debugcon: Vec<u8> = machine.debug_port().take_output().collect();
    if !debugcon.is_empty() {
        println!("port 0xE9 said: {}", String::from_utf8_lossy(&debugcon));
    }
}

/// Walk up from the current directory until a `Cargo.toml` with a workspace in
/// it turns up, so the example runs from anywhere in the tree.
fn workspace_root() -> Option<PathBuf> {
    let mut at: PathBuf = std::env::current_dir().ok()?;
    loop {
        if is_workspace_root(&at) {
            return Some(at);
        }
        if !at.pop() {
            return None;
        }
    }
}

fn is_workspace_root(at: &Path) -> bool {
    std::fs::read_to_string(at.join("Cargo.toml"))
        .is_ok_and(|manifest| manifest.contains("[workspace]"))
}
