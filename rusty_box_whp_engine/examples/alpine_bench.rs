//! Alpine Linux booting on both engines, timed the same way.
//!
//! DLX is the workload least suited to a hypervisor: a 1995 guest that reads
//! its disk a sector at a time through a port and clears its screen a byte at a
//! time through the planar VGA aperture, so almost every instruction that
//! matters leaves the hardware. Alpine is the other end of the same axis — it
//! drives the disk with bus-master DMA and spends its time in kernel code
//! rather than in device registers.
//!
//! The point of measuring both is that neither number alone says what this
//! engine costs. This one runs the same ISO on the interpreter and on the
//! hypervisor, from the same BIOS through the same devices, and reports the
//! host time each took to reach each milestone.
//!
//! ```text
//! cargo run --release -p rusty_box_whp_engine --example alpine_bench
//! ALPINE_ISO=/path/to/alpine.iso ALPINE_PATIENCE_SECS=300 …
//! ```

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusty_box::emulator::{
    AtaSlot, BootDevice, BootOrder, Emulator, EmulatorConfig, Ips, MachineBuilder, MemorySize,
    RunBudget, SliceEngine, StopReason,
};
use rusty_box_whp_engine::WhpEngine;

/// The exit code a harness reads as "this host cannot run this".
const SKIPPED: i32 = 77;

/// How much guest time one step is allowed to cover.
const SLICE_TICKS: u64 = 20_000_000;

/// Stack for the thread that builds the machines. An `Emulator` is assembled
/// before it is boxed, and the main thread's stack is not large enough.
const STACK: usize = 64 * 1024 * 1024;

/// What the guest must put on its own screen, in order.
const MILESTONES: &[(&str, &str)] = &[
    ("the BIOS is alive", "BIOS"),
    ("the boot loader has the disk", "ISOLINUX"),
    ("the kernel is up", "login:"),
];

fn patience() -> Duration {
    match std::env::var("ALPINE_PATIENCE_SECS").ok().and_then(|secs| secs.parse().ok()) {
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

/// How far a boot got, and how long each step took.
struct Reached {
    milestones: usize,
    took: Duration,
    at: Vec<Duration>,
}

/// Run one machine to its last milestone, or until patience runs out.
fn boot<E>(machine: &mut Emulator<(), E>, label: &str) -> Reached
where
    E: SliceEngine<()>,
{
    let began = Instant::now();
    let mut reached = 0;
    let mut panicked = false;
    let mut at = Vec::new();
    let limit = patience();
    loop {
        let outcome = match machine.step(RunBudget::Ticks(SLICE_TICKS)) {
            Ok(outcome) => outcome,
            Err(error) => {
                eprintln!("  {label}: the run ended with an error: {error}");
                break;
            }
        };
        if let Some(text) = machine.display().text() {
            let screen = text.to_text();
            // Into the same log stream as everything else the machine
            // narrates, so a guest that gave up can be placed in the order of
            // events rather than only at the end of them.
            if !panicked && screen.contains("Kernel panic") {
                panicked = true;
                tracing::info!(target: "vec", "GUEST PANICKED");
            }
            while reached < MILESTONES.len() {
                let (name, needle) = MILESTONES[reached];
                if !screen.contains(needle) {
                    break;
                }
                let elapsed = began.elapsed();
                println!("  {label}: {name} ({:.1}s)", elapsed.as_secs_f64());
                at.push(elapsed);
                reached += 1;
            }
        }
        if reached == MILESTONES.len() {
            break;
        }
        if outcome.is_terminal() {
            eprintln!("  {label}: the guest stopped: {:?}", outcome.stop);
            break;
        }
        if matches!(outcome.stop, StopReason::Halted) && outcome.progress.stalled() {
            eprintln!("  {label}: the guest halted with nothing left to wake it");
            break;
        }
        if began.elapsed() > limit {
            eprintln!("  {label}: gave up after {limit:?}");
            break;
        }
    }
    if reached < MILESTONES.len() {
        // The screen of a boot that did not finish is the whole reason to have
        // run it. Printed on every path that gives up, because the run worth
        // looking at is exactly the one that failed.
        dump(machine, label);
    }
    Reached { milestones: reached, took: began.elapsed(), at }
}

/// What the guest had reached when it stopped.
fn dump<E>(machine: &mut Emulator<(), E>, label: &str)
where
    E: SliceEngine<()>,
{
    println!("\n  {label}: RIP = {:#x}", machine.rip());
    match machine.display().text() {
        Some(text) => {
            let screen = text.to_text();
            if screen.trim().is_empty() {
                println!("  {label}: the screen is blank");
            }
            for line in screen.lines() {
                let line = line.trim_end();
                if !line.is_empty() {
                    println!("  | {line}");
                }
            }
        }
        None => println!("  {label}: the display is not in a text mode"),
    }
}

fn config() -> EmulatorConfig {
    EmulatorConfig {
        memory: MemorySize::bytes(512 * 1024 * 1024),
        memory_block_size: 128 * 1024,
        ips: Ips::new(300_000_000),
        pci_enabled: true,
        ..EmulatorConfig::default()
    }
}

fn builder<'a>(bios: &'a [u8], vga: Option<&'a [u8]>, iso: &str) -> MachineBuilder<'a, ()> {
    let mut builder = MachineBuilder::new(config())
        .bios(bios)
        .boot_order(BootOrder::just(BootDevice::Cdrom))
        .cdrom_file(AtaSlot::SECONDARY_MASTER, iso);
    if let Some(vga) = vga {
        builder = builder.vga_bios(vga);
    }
    builder
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
    // Without this nothing the machine narrates is printed, and an
    // investigation reads every count as zero — which looks exactly like a
    // finding and is not one. `RUST_LOG` decides what is listened to; note
    // that this workspace compiles `trace!` and `debug!` out of release
    // builds, so a probe meant to be seen here has to be `info!`.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(true)
        .init();

    let Some(root) = workspace_root() else {
        eprintln!("not inside the workspace");
        return std::process::ExitCode::FAILURE;
    };
    let iso = std::env::var("ALPINE_ISO")
        .unwrap_or_else(|_| root.join("alpine-virt-3.24.1-x86_64.iso").to_string_lossy().into());
    if !Path::new(&iso).exists() {
        eprintln!("no Alpine ISO at {iso}; set ALPINE_ISO");
        return std::process::ExitCode::FAILURE;
    }
    let bios = match std::fs::read(root.join("cpp_orig/bochs/bochs/bios/BIOS-bochs-latest")) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("no BIOS image: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let vga = std::fs::read(
        root.join("cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin"),
    )
    .ok();

    println!("Alpine: {iso}\n");

    // Either engine alone, because a failure on one is investigated by running
    // that one repeatedly and the other's boot is a minute of waiting each time.
    let only = std::env::var("ALPINE_ENGINE").unwrap_or_default();
    let want_software = only != "whp";
    let want_hardware = only != "interpreter";

    println!("interpreter:");
    let software = if !want_software {
        Reached { milestones: 0, took: Duration::ZERO, at: Vec::new() }
    } else {
        let mut machine = match builder(&bios, vga.as_deref(), &iso).build() {
            Ok(machine) => machine,
            Err(error) => {
                eprintln!("could not assemble the interpreter machine: {error}");
                return std::process::ExitCode::FAILURE;
            }
        };
        boot(&mut machine, "interpreter")
    };

    if !rusty_box_whp::hypervisor_present().unwrap_or(false) {
        println!("\nskipped: this host has no Windows Hypervisor Platform");
        return std::process::ExitCode::from(SKIPPED as u8);
    }

    println!("\nhypervisor:");
    let hardware = if !want_hardware {
        Reached { milestones: 0, took: Duration::ZERO, at: Vec::new() }
    } else {
        let mut machine =
            match builder(&bios, vga.as_deref(), &iso).build_on::<WhpEngine>() {
                Ok(machine) => machine,
                Err(error) => {
                    eprintln!("could not assemble the hypervisor machine: {error}");
                    return std::process::ExitCode::FAILURE;
                }
            };
        boot(&mut machine, "hypervisor")
    };

    println!("\n{:<14} {:>10} {:>12}", "milestone", "interpreter", "hypervisor");
    for (index, (name, _)) in MILESTONES.iter().enumerate() {
        let one = software.at.get(index).map(|at| format!("{:.1}s", at.as_secs_f64()));
        let two = hardware.at.get(index).map(|at| format!("{:.1}s", at.as_secs_f64()));
        println!(
            "{name:<14} {:>10} {:>12}",
            one.as_deref().unwrap_or("—"),
            two.as_deref().unwrap_or("—")
        );
    }
    println!(
        "\ninterpreter reached {} of {} in {:.1}s; hypervisor {} of {} in {:.1}s",
        software.milestones,
        MILESTONES.len(),
        software.took.as_secs_f64(),
        hardware.milestones,
        MILESTONES.len(),
        hardware.took.as_secs_f64(),
    );
    if software.milestones == MILESTONES.len() && hardware.milestones == MILESTONES.len() {
        println!(
            "the hypervisor is {:.2}x the interpreter on this boot",
            software.took.as_secs_f64() / hardware.took.as_secs_f64()
        );
    }
    std::process::ExitCode::SUCCESS
}
