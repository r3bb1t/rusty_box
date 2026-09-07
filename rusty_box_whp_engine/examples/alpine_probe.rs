//! Alpine Linux booting on either engine, driven a slice at a time, with the
//! engine's own census beside the guest's own clock.
//!
//! The measurement harness for a boot's economy on both engines. It assembles
//! the machine `rusty_box_gui`'s runner assembles for
//! `--cpu-capabilities host-shared --ips 300000000 --cdrom <iso> --boot cdrom
//! --memory-mib 256 --host-memory-mib 256`, runs it on the engine
//! `ALPINE_ENGINE` names, drives it in a tight loop with no display rendering
//! and no input pump, and reports:
//!
//! - `ticks`: the arm's own progress counter summed over every slice, the
//!   seconds it amounts to at 300 M per second, and that figure per
//!   wall-second. Which quantity that counter holds is the arm's to decide,
//!   and the `progress_unit=` field of the `RESULT wall_s` line names it:
//!   guest ticks on the hypervisor engine, retired instructions on the
//!   interpreter. `ticks`, `guest_s`, `guest_per_wall` and every `guest=`
//!   derived from them are therefore guest seconds on the hypervisor arm
//!   alone; the two units track each other along the executing path and
//!   nowhere else, because the idle a HLT fast-forwards over falls outside the
//!   counter on both engines. Compare the arms by `wall_s`, never by these;
//! - on the hypervisor engine, the fast machine's `engine_census()` — the
//!   exits by class, what was put in the pending-event slot, and the
//!   hypervisor's own intercept and runtime counters as of the last park, the
//!   independent account that audits this port's. An arm that keeps no exit
//!   census prints `RESULT exits=none` in their place;
//! - `RESULT in_run_share`, one line per adjacent pair of milestones: the
//!   fraction of that phase's wall clock the processor spent INSIDE
//!   `WHvRunVirtualProcessor`, the hypervisor's own guest-versus-overhead
//!   split over the same window as a cross-check, and the exits the phase
//!   cost. This is the number the whole hypervisor design is judged by;
//! - the wall and engine time at which each screen milestone first appears,
//!   and the newest kernel `[ t ]` stamp on screen with the time it was seen
//!   at (the kernel's stamps are hardware-TSC time scaled by a boot-time
//!   calibration, NOT the engine's clock — reported so the two are never
//!   confused);
//! - `RESULT fatal_signatures`, counted over every distinct screen line the
//!   run ever showed rather than over the screen it ended on. The 80×25 plane
//!   is a snapshot: an oops printed between two scrapes has scrolled off by
//!   the next one, and a scan of the final screen would report a clean boot.
//!
//! Driven a slice at a time rather than by a display loop on purpose: the
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
//! `ALPINE_ENGINE=interpreter` runs this port's own interpreter; any other
//! value, unset included, runs the hypervisor engine. Both arms go through the
//! same loop, the same slice shape, the same stopping conditions and the same
//! engine-independent `RESULT` lines, which is what makes their wall times
//! comparable.
//!
//! `ALPINE_PROBE_PATIENCE_SECS` bounds the run in host time (default 300).
//! The hypervisor arm exits 77 when the host has no hypervisor; the
//! interpreter arm opens no partition and asks for none.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rusty_box::cpu::decoder::features::X86Feature;
use rusty_box::emulator::{
    AtaSlot, BootDevice, BootOrder, DeviceClock, Emulator, EmulatorConfig, Ips, MachineBuilder,
    MemorySize, ProgressUnit, RunBudget, SliceEngine, StopReason,
};
use rusty_box::params::BxParams;
use rusty_box::CpuidFreq;
use rusty_box_whp_engine::{FastMachine, FastMachineFault, StepStop, WhpEngine};

const MIB: usize = 1024 * 1024;
const IPS: u32 = 300_000_000;
/// The same outer slice the DLX example uses: on the interpreter the inner
/// batch is capped at 100 000 ticks by `run_until_budget_spent`, the same cap
/// the interactive loop's `INSTRUCTION_BATCH_SIZE` imposes, so the engine sees
/// the slice shape the GUI gives it; on hardware it is 66 ms of guest time
/// between one pause and the next.
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

/// A line carrying one of these is a boot that went wrong, whatever else it
/// reached: a kernel oops, a fault the kernel printed rather than handled, or
/// this port's own name for a vector it could not place.
const FATAL: &[&str] = &["Oops", "general protection", "unexpected_intr"];

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

/// Both accounts of what a machine on hardware has been doing, at one instant.
///
/// This port's own — how long its processors have been inside the partition,
/// and how often they left it — beside the hypervisor's, which is the only
/// independent check on it: an account cannot audit itself.
#[derive(Clone, Copy)]
struct CensusSample {
    /// Host nanoseconds spent inside `WHvRunVirtualProcessor`, summed over
    /// every processor.
    in_run_nanos: u64,
    /// The hypervisor's own total processor runtime, in 100 ns units.
    total_100ns: u64,
    /// The part of that runtime the hypervisor charged to itself rather than
    /// to the guest.
    hypervisor_100ns: u64,
    /// Exits taken, whatever their class.
    exits: u64,
}

/// When something was first seen, in every clock the probe keeps.
#[derive(Clone, Copy)]
struct Sighting {
    /// Host seconds since the run began.
    wall: f64,
    /// The machine's own clock at that moment.
    ticks: u64,
    /// What the hardware had done by then, from an arm that keeps a census.
    census: Option<CensusSample>,
}

/// The newest kernel `[ seconds.micros]` stamp on screen, with the moment the
/// probe read it. The stamp is hardware-TSC time scaled by the kernel's
/// boot-time calibration, so it is never the same clock as the sighting
/// beside it.
#[derive(Clone, Copy)]
struct Stamp {
    stamp: f64,
    seen: Sighting,
}

/// What one slice of either arm amounts to.
struct Advance {
    /// How much of the arm's own progress unit passed.
    progress: u64,
    /// Why the run is over, when this slice ended it.
    ending: Option<Ending>,
}

/// How a run ended.
enum Ending {
    /// The machine said so, in its own words.
    Said { why: String },
    /// A processor did not come back within the driver's bound. The run FAILS
    /// on this, where every other ending is still a measurement.
    Wedged { waited: f64 },
}

/// The verbs a boot needs, in whichever shape the arm's machine has them.
///
/// A trait rather than two loops because the comparison this instrument exists
/// for is only sound when both arms are driven the same way: the same slice
/// shape, the same screen scrape, the same stopping conditions, the same
/// `RESULT` lines. Two loops that merely looked alike would be comparing
/// themselves.
trait Arm {
    /// What this arm's progress counter is denominated in.
    const PROGRESS_UNIT: ProgressUnit;

    /// Arm the machine's timers and queue the runner's Enter, and say how many
    /// keys went in.
    fn prepare(&mut self) -> usize;

    /// Run the guest for one slice.
    fn advance(&mut self) -> Advance;

    /// The guest's text screen as it stands between two slices.
    fn screen(&mut self) -> Option<String>;

    /// Where the processor is.
    fn rip(&mut self) -> u64;

    /// This arm's own part of the ten-second progress line. Empty from an arm
    /// that keeps no census, so the line closes up rather than leaving a gap.
    fn progress_line(&self) -> String;

    /// This arm's own `RESULT` lines.
    fn results(&self);

    /// Both accounts of the hardware as they stand, or `None` from an arm that
    /// has no hardware to account for.
    fn sample(&self) -> Option<CensusSample>;
}

/// The interpreter's arm: a machine advanced by `Emulator::step`.
struct Stepped<E: SliceEngine<()>>(Box<Emulator<(), E>>);

impl<E: SliceEngine<()>> Arm for Stepped<E> {
    const PROGRESS_UNIT: ProgressUnit = E::PROGRESS_UNIT;

    fn prepare(&mut self) -> usize {
        self.0.prepare_run();
        self.0.keyboard().type_text("\n")
    }

    fn advance(&mut self) -> Advance {
        let outcome = match self.0.step(RunBudget::Ticks(SLICE_TICKS)) {
            Ok(outcome) => outcome,
            Err(error) => {
                return Advance {
                    progress: 0,
                    ending: Some(Ending::Said { why: format!("run error: {error}") }),
                }
            }
        };
        let progress = outcome.progress.count();
        let ending = if outcome.is_terminal() {
            Some(Ending::Said { why: format!("guest stopped: {:?}", outcome.stop) })
        } else if matches!(outcome.stop, StopReason::Halted) && outcome.progress.stalled() {
            Some(Ending::Said { why: "halted with nothing to wake it".into() })
        } else {
            None
        };
        Advance { progress, ending }
    }

    fn screen(&mut self) -> Option<String> {
        self.0.display().text().map(|text| text.to_text())
    }

    fn rip(&mut self) -> u64 {
        self.0.rip()
    }

    /// An interpreter never leaves the guest, so it has no census to report.
    fn progress_line(&self) -> String {
        String::new()
    }

    fn results(&self) {
        println!("RESULT exits=none");
    }

    fn sample(&self) -> Option<CensusSample> {
        None
    }
}

/// The hypervisor's arm: a machine adopted onto hardware, advanced by
/// `FastMachine::step`.
struct Adopted(FastMachine<()>);

impl Arm for Adopted {
    /// A hardware processor retires instructions nothing counts, so this arm's
    /// progress is the guest's own time.
    const PROGRESS_UNIT: ProgressUnit = ProgressUnit::Ticks;

    fn prepare(&mut self) -> usize {
        self.0.with_machine(|machine| {
            machine.prepare_run();
            machine.keyboard().type_text("\n")
        })
    }

    fn advance(&mut self) -> Advance {
        let outcome = match self.0.step(RunBudget::Ticks(SLICE_TICKS)) {
            Ok(outcome) => outcome,
            Err(FastMachineFault::Wedged { waited }) => {
                return Advance {
                    progress: 0,
                    ending: Some(Ending::Wedged { waited: waited.as_secs_f64() }),
                }
            }
            Err(error) => {
                return Advance {
                    progress: 0,
                    ending: Some(Ending::Said { why: format!("run error: {error}") }),
                }
            }
        };
        let ending = match outcome.stop {
            StepStop::BudgetSpent => None,
            StepStop::GuestPowerOff => {
                Some(Ending::Said { why: "guest stopped: GuestPowerOff".into() })
            }
            StepStop::Faulted(fault) => {
                Some(Ending::Said { why: format!("guest stopped: {fault}") })
            }
        };
        Advance { progress: outcome.ticks, ending }
    }

    fn screen(&mut self) -> Option<String> {
        self.0.with_machine(|machine| machine.display().text().map(|text| text.to_text()))
    }

    fn rip(&mut self) -> u64 {
        self.0.with_machine(|machine| machine.rip())
    }

    fn progress_line(&self) -> String {
        let census = self.0.engine_census();
        format!(
            "runs={} injected={} windows_armed={} exits={:?}",
            census.vcpus.first().map_or(0, |vcpu| vcpu.runs),
            census.injections.injected,
            census.injections.windows_armed,
            census.exits
        )
    }

    fn results(&self) {
        let census = self.0.engine_census();
        println!("RESULT exits={:?}", census.exits);
        println!(
            "RESULT inject injected={} windows_armed={}",
            census.injections.injected, census.injections.windows_armed
        );
        // The hypervisor's own counters come from the processor's own thread,
        // recorded as it parked — the engine cannot ask for them, because the
        // processor it would ask about belongs to a thread now. The first
        // processor is the whole machine here: this probe fixes the topology
        // at one socket, one core, one thread.
        match census.vcpus.first().and_then(|vcpu| vcpu.platform_at_last_park) {
            Some(counters) => {
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
            None => println!("RESULT platform counters unavailable: no processor has parked"),
        }
    }

    /// Summed over every processor, in both accounts.
    ///
    /// The two shares are a ratio each, and a ratio whose numerator covered
    /// every processor while its denominator covered one would be a number
    /// with no meaning on an SMP machine.
    fn sample(&self) -> Option<CensusSample> {
        let census = self.0.engine_census();
        let runtimes = census
            .vcpus
            .iter()
            .filter_map(|vcpu| vcpu.platform_at_last_park)
            .map(|counters| counters.runtime);
        let mut total_100ns = 0u64;
        let mut hypervisor_100ns = 0u64;
        for runtime in runtimes {
            total_100ns = total_100ns.saturating_add(runtime.total_100ns);
            hypervisor_100ns = hypervisor_100ns.saturating_add(runtime.hypervisor_100ns);
        }
        Some(CensusSample {
            in_run_nanos: census.vcpus.iter().map(|vcpu| vcpu.in_run_nanos).sum(),
            total_100ns,
            hypervisor_100ns,
            exits: census.exits.total(),
        })
    }
}

/// What one run amounts to in terms no engine defines, so that both arms
/// report it the same way.
struct Outcome {
    /// Why the loop stopped.
    ending: String,
    /// Host seconds the whole run took.
    wall: f64,
    /// The arm's own progress counter, summed over every slice, in its own
    /// [`ProgressUnit`]: guest ticks from a machine on hardware, retired
    /// instructions from an interpreter. Every `guest`-prefixed figure the
    /// report derives from it inherits that unit.
    ticks: u64,
    /// The first sighting of each [`MILESTONES`] needle, in that order.
    seen: Vec<Option<Sighting>>,
    /// The newest kernel stamp seen anywhere in the run.
    newest_stamp: Option<Stamp>,
    /// Every distinct non-blank line the screen ever showed.
    lines: BTreeSet<String>,
    /// Set when a processor did not come back. The run fails on it.
    wedged: Option<f64>,
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

    // Which engine runs the guest is the only difference between the two arms:
    // the same builder, the same loop, the same report. Read before the gate
    // below, because an interpreter run needs no partition to open.
    let interpreter = std::env::var("ALPINE_ENGINE").unwrap_or_default() == "interpreter";
    if !interpreter {
        match rusty_box_whp::hypervisor_present() {
            Ok(true) => {}
            other => {
                println!("skipped: no hypervisor ({other:?})");
                return SKIPPED;
            }
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
    // The device clock is the one field the two arms disagree about, and it is
    // not a preference: a machine on hardware runs its devices on host time
    // because nothing counts its processor's instructions, and
    // `FastMachine::adopt` refuses a machine that keeps device time in ticks
    // rather than adopting it with its devices silently frozen.
    let device_clock = if interpreter { DeviceClock::Ticks } else { DeviceClock::HostTime };
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
        device_clock,
        ..EmulatorConfig::default()
    };
    let builder = MachineBuilder::new(config)
        .bios(&bios)
        .vga_bios(&vga)
        .boot_order(BootOrder::just(BootDevice::Cdrom))
        .cdrom_file(AtaSlot::SECONDARY_MASTER, &iso);
    if interpreter {
        let machine = match builder.build() {
            Ok(machine) => machine,
            Err(error) => {
                eprintln!("could not assemble the machine: {error}");
                return 1;
            }
        };
        let mut arm = Stepped(machine);
        measure(&mut arm, &iso)
    } else {
        let assembled = match builder.build_on::<WhpEngine>() {
            Ok(machine) => machine,
            Err(error) => {
                eprintln!("could not assemble the machine: {error}");
                return 1;
            }
        };
        // Adoption hands the processor to a thread that enters the partition
        // and stays there, and gives the devices a thread that wakes at their
        // deadlines. Nothing runs until the first slice asks.
        let machine = match FastMachine::adopt(assembled) {
            Ok(machine) => machine,
            Err(error) => {
                eprintln!("could not adopt the machine onto hardware: {error}");
                return 1;
            }
        };
        let mut arm = Adopted(machine);
        measure(&mut arm, &iso)
    }
}

/// Boot one arm and print the whole report for it.
///
/// Returns the process's exit code: zero for a run that produced a
/// measurement, whatever it measured, and non-zero only for a processor that
/// did not come back — the one outcome that is not a measurement at all.
fn measure<A: Arm>(arm: &mut A, iso: &str) -> i32 {
    // The runner queues one Enter for a CD-first boot order; so does this.
    let typed = arm.prepare();
    println!("probe: iso={iso} patience={:?} prequeued_keys={typed}", patience());

    let outcome = drive(arm, patience());

    let guest = outcome.ticks as f64 / IPS as f64;
    // What `ticks` counts is the arm's choice, so the line that prints it says
    // which quantity a reader is holding. Only a ticks-reporting arm makes
    // `guest_s` and `guest_per_wall` guest seconds.
    let progress_unit = match A::PROGRESS_UNIT {
        ProgressUnit::Instructions => "instructions",
        ProgressUnit::Ticks => "ticks",
    };
    println!();
    println!("RESULT ending={:?}", outcome.ending);
    println!(
        "RESULT wall_s={:.2} ticks={} guest_s={guest:.3} guest_per_wall={:.3} progress_unit={progress_unit}",
        outcome.wall,
        outcome.ticks,
        guest / outcome.wall
    );
    for (index, needle) in MILESTONES.iter().enumerate() {
        match outcome.seen[index] {
            Some(at) => println!(
                "RESULT milestone {needle:?} wall={:.1} guest={:.2}",
                at.wall,
                at.ticks as f64 / IPS as f64
            ),
            None => println!("RESULT milestone {needle:?} NOT reached"),
        }
    }
    report_phases(&outcome.seen);
    match outcome.newest_stamp {
        Some(newest) => println!(
            "RESULT dmesg_max={:.3} seen_at_wall={:.1} seen_at_guest={:.2}",
            newest.stamp,
            newest.seen.wall,
            newest.seen.ticks as f64 / IPS as f64
        ),
        None => println!("RESULT dmesg_max=none"),
    }
    report_fatal(&outcome.lines);
    if let Some(waited) = outcome.wedged {
        println!("RESULT wedged waited={waited:.1}");
    }
    arm.results();
    println!("RIP = {:#x}", arm.rip());
    if let Some(screen) = arm.screen() {
        println!("screen:");
        for line in screen.lines() {
            let line = line.trim_end();
            if !line.is_empty() {
                println!("  | {line}");
            }
        }
    }
    if outcome.wedged.is_some() {
        1
    } else {
        0
    }
}

/// What each phase between two milestones cost, in the two independent
/// accounts of it.
///
/// `share` is this port's own measurement — host time inside
/// `WHvRunVirtualProcessor` over the phase's wall clock — and is the number
/// the hypervisor design is judged by. `guest_share` is the hypervisor's own
/// split of the same window, guest time over total processor runtime, and it
/// audits the first: two accounts that disagree mean one of them is measuring
/// something other than what it names.
///
/// A phase whose endpoints were not both reached, or whose arm keeps no
/// census, still gets its line, saying so. Omitting it would leave a reader
/// counting lines to work out which phase was missing.
fn report_phases(seen: &[Option<Sighting>]) {
    for (index, pair) in MILESTONES.windows(2).enumerate() {
        let (from, to) = (pair[0], pair[1]);
        let phase = format!("phase={from:?}..{to:?}");
        let opened = seen[index];
        let closed = seen[index + 1];
        let (Some(opened), Some(closed)) = (opened, closed) else {
            println!("RESULT in_run_share {phase} share=none guest_share=none exits=none");
            continue;
        };
        let (Some(before), Some(after)) = (opened.census, closed.census) else {
            println!("RESULT in_run_share {phase} share=none guest_share=none exits=none");
            continue;
        };
        let wall_nanos = (closed.wall - opened.wall) * 1e9;
        let share = if wall_nanos > 0.0 {
            let ran = after.in_run_nanos.saturating_sub(before.in_run_nanos);
            format!("{:.3}", ran as f64 / wall_nanos)
        } else {
            "none".into()
        };
        let charged = after.total_100ns.saturating_sub(before.total_100ns);
        let guest_share = if charged > 0 {
            let guest = after.total_100ns.saturating_sub(after.hypervisor_100ns);
            let was = before.total_100ns.saturating_sub(before.hypervisor_100ns);
            format!("{:.3}", guest.saturating_sub(was) as f64 / charged as f64)
        } else {
            "none".into()
        };
        println!(
            "RESULT in_run_share {phase} share={share} guest_share={guest_share} exits={}",
            after.exits.saturating_sub(before.exits)
        );
    }
}

/// Count the run's fatal signatures over every line it ever showed.
fn report_fatal(lines: &BTreeSet<String>) {
    let fatal: Vec<&String> = lines
        .iter()
        .filter(|line| FATAL.iter().any(|needle| line.contains(needle)))
        .collect();
    println!("RESULT fatal_signatures={}", fatal.len());
    for line in fatal {
        println!("  fatal | {line}");
    }
}

/// Drive one arm a slice at a time until the last milestone, a stop, or
/// `limit`.
fn drive<A: Arm>(arm: &mut A, limit: Duration) -> Outcome {
    let began = Instant::now();
    let mut ticks = 0u64;
    let mut seen: Vec<Option<Sighting>> = vec![None; MILESTONES.len()];
    let mut newest_stamp: Option<Stamp> = None;
    let mut lines: BTreeSet<String> = BTreeSet::new();
    let mut next_report = 10u64;
    let mut ending = String::from("login reached");
    let mut wedged = None;
    loop {
        let advance = arm.advance();
        ticks = ticks.saturating_add(advance.progress);
        let wall = began.elapsed().as_secs_f64();

        if let Some(screen) = arm.screen() {
            // Both accounts of the hardware are read at most once a slice, and
            // only when a milestone was actually hit: the read locks the
            // machine, and a boot takes tens of thousands of slices.
            let mut sampled: Option<Option<CensusSample>> = None;
            for (index, needle) in MILESTONES.iter().enumerate() {
                if seen[index].is_none() && screen.contains(needle) {
                    let census = *sampled.get_or_insert_with(|| arm.sample());
                    seen[index] = Some(Sighting { wall, ticks, census });
                    println!(
                        "milestone {needle:?}: wall {wall:.1}s ticks {ticks} guest {:.2}s",
                        ticks as f64 / IPS as f64
                    );
                }
            }
            let now = Sighting { wall, ticks, census: sampled.flatten() };
            for line in screen.lines() {
                if let Some(stamp) = dmesg_stamp(line) {
                    if newest_stamp.is_none_or(|newest| stamp > newest.stamp) {
                        newest_stamp = Some(Stamp { stamp, seen: now });
                    }
                }
                let line = line.trim_end();
                // The membership test is what makes the answer to the insert
                // below already known, and it is what keeps the allocation
                // rare: a boot shows a few hundred distinct lines across tens
                // of thousands of scrapes of the same screen.
                if !line.is_empty() && !lines.contains(line) {
                    lines.insert(line.to_owned());
                }
            }
        }

        if began.elapsed().as_secs() >= next_report {
            next_report += 10;
            let arm_line = arm.progress_line();
            // An arm that reports nothing leaves no gap in the line.
            let spacer = if arm_line.is_empty() { "" } else { " " };
            println!(
                "t={wall:.1}s ticks={ticks} guest={:.2}s ratio={:.2}{spacer}{arm_line} dmesg_max={:?}",
                ticks as f64 / IPS as f64,
                (ticks as f64 / IPS as f64) / wall,
                newest_stamp.map(|newest| newest.stamp),
            );
        }

        if seen.last().is_some_and(Option::is_some) {
            break;
        }
        match advance.ending {
            Some(Ending::Said { why }) => {
                ending = why;
                break;
            }
            Some(Ending::Wedged { waited }) => {
                wedged = Some(waited);
                ending = format!("a processor did not come back within {waited:.1}s");
                break;
            }
            None => {}
        }
        if began.elapsed() > limit {
            ending = format!("patience {limit:?} exhausted");
            break;
        }
    }

    Outcome {
        ending,
        wall: began.elapsed().as_secs_f64(),
        ticks,
        seen,
        newest_stamp,
        lines,
        wedged,
    }
}
