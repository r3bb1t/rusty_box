use crate::{
    args::{Args, BootDevice, DisplayBackend, LogLevel},
    config::{Engine, ResolvedConfig},
    error::RunError,
};
// Named only where the hypervisor engine forces it; the interpreter path
// reaches the same narrowing through a method call and needs no name.
#[cfg(all(not(feature = "guest-trace"), feature = "hv-whp", windows))]
use crate::config::CpuCapabilities;
use rusty_box::params::BxParams;
#[cfg(feature = "gui-egui")]
use rusty_box::gui::{shared_display::SharedDisplay, BridgeGui};
#[cfg(all(not(feature = "guest-trace"), feature = "hv-whp", windows))]
use rusty_box::emulator::RunBudget;
use rusty_box::emulator::{
    AtaSlot, BootDevice as GuestBootDevice, BootOrder, DeviceClock,
    DiskGeometry as GuestDiskGeometry, Emulator, EmulatorConfig, Ips, MachineBuilder, MemorySize,
    SliceEngine,
};
use rusty_box::gui::{BxGui, NoGui, TermGui};
#[cfg(all(not(feature = "guest-trace"), feature = "hv-whp", windows))]
use rusty_box_whp_engine::{FastMachine, FastMachineFault, StepStop, WhpEngine};
#[cfg(feature = "gui-egui")]
use std::sync::atomic::Ordering;
#[cfg(feature = "gui-egui")]
use std::sync::{mpsc, Mutex};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

/// What a run amounted to, once it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunSummary {
    /// Guest instructions the boot processor retired, when the engine that ran
    /// the guest counts them.
    ///
    /// `None` for a machine on the hypervisor: a processor there retires
    /// instructions the host never tallies, and an approximate count would be
    /// indistinguishable from a real one.
    pub instructions_executed: Option<u64>,
}

pub fn run(args: Args) -> Result<RunSummary, RunError> {
    let config = crate::config::load_config(&args)?;
    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    if config.display == DisplayBackend::Egui {
        return run_shell(Some(LaunchVm::from_args(&args, config)));
    }
    run_resolved(config)
}

pub fn run_resolved(config: ResolvedConfig) -> Result<RunSummary, RunError> {
    match config.display {
        DisplayBackend::Headless => run_with_gui(config, NoGui::new(), None, true),
        DisplayBackend::Terminal => run_with_gui(config, TermGui::new(), None, true),
        #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
        DisplayBackend::Egui => run_shell(Some(LaunchVm {
            name: "Command line".to_owned(),
            config,
            source: LaunchSource::Flags,
        })),
        #[cfg(all(feature = "gui-egui", target_os = "android"))]
        DisplayBackend::Egui => run_egui(config),
    }
}

pub(crate) fn create_configured_disk_images(config: &ResolvedConfig) -> Result<(), RunError> {
    if let Some(disk) = &config.disk {
        if let Some(creation) = &disk.creation {
            match crate::disk_images::provision_startup_disk(creation)? {
                crate::disk_images::DiskProvisionOutcome::Created(created) => {
                    tracing::info!(
                        "startup disk: created {} ({} bytes)",
                        created.path.display(),
                        created.bytes
                    );
                }
                crate::disk_images::DiskProvisionOutcome::Reused { path, bytes } => {
                    tracing::info!(
                        "startup disk: reusing existing image {} ({bytes} bytes)",
                        path.display()
                    );
                }
            }
        }
    }
    Ok(())
}

/// Human-readable summary of what launching this config will do to the startup
/// disk, or `None` when there is no startup image to provision. Used to warn the
/// user before a potentially slow flat-file allocation.
#[cfg(feature = "gui-egui")]
pub(crate) fn startup_disk_action(config: &ResolvedConfig) -> Option<String> {
    let creation = config.disk.as_ref()?.creation.as_ref()?;
    if creation.overwrite {
        Some(format!(
            "Recreating disk image {} ({}) — overwrite is on, so any existing data is discarded. \
             Allocating the full file can take a while; the window may look idle until it finishes.",
            creation.path.display(),
            creation.size.display()
        ))
    } else if creation.path.exists() {
        None // reusing an existing image is instant; no warning needed
    } else {
        Some(format!(
            "Creating disk image {} ({}). Allocating the full flat file can take a while — \
             the window may look idle until the guest starts.",
            creation.path.display(),
            creation.size.display()
        ))
    }
}

/// Checks the media files `config` names and, when `create_startup_disks`,
/// provisions its startup disk. Every file this call does not create — the
/// CD-ROM, and a disk that is not created here — is checked first, so a
/// power-on refused for a missing one leaves an `overwrite` disk as it was.
fn prepare_configured_media_files(
    config: &ResolvedConfig,
    create_startup_disks: bool,
) -> Result<(), RunError> {
    let creates_disk =
        create_startup_disks && config.disk.as_ref().is_some_and(|disk| disk.creation.is_some());
    if let Some(cdrom) = &config.cdrom {
        verify_media_file("CD-ROM", &cdrom.path)?;
    }
    if let (Some(disk), false) = (&config.disk, creates_disk) {
        verify_media_file("disk", &disk.path)?;
    }
    if create_startup_disks {
        create_configured_disk_images(config)?;
    }
    if let (Some(disk), true) = (&config.disk, creates_disk) {
        verify_media_file("disk", &disk.path)?;
    }
    Ok(())
}

/// Prepares `config` and runs it on `gui` in one step, provisioning its
/// startup disk when `create_startup_disks`: the headless and terminal runs,
/// which provision the disk at every run.
fn run_with_gui<G>(
    config: ResolvedConfig,
    gui: G,
    stop_flag: Option<Arc<AtomicBool>>,
    create_startup_disks: bool,
) -> Result<RunSummary, RunError>
where
    G: BxGui + 'static,
{
    let prepared = prepare_run(config, create_startup_disks)?;
    run_prepared(prepared, gui, stop_flag)
}

/// A run's inputs, read and checked, with its startup disk provisioned.
/// Everything a configuration is refused for before its disk is touched is
/// refused before one of these exists.
struct PreparedRun {
    config: ResolvedConfig,
    bios_data: Vec<u8>,
    vga_data: Option<Vec<u8>>,
    slots: MediaSlots,
}

/// Reads and checks `config`'s files and, when `create_startup_disks`,
/// provisions its startup disk — last, so a configuration refused here for
/// an engine this build lacks, a blank or unreadable BIOS or a media slot
/// that does not resolve has erased nothing.
fn prepare_run(
    config: ResolvedConfig,
    create_startup_disks: bool,
) -> Result<PreparedRun, RunError> {
    init_tracing(config.log_level);

    // A build without the hypervisor path cannot honour the `whp` engine,
    // wherever it was chosen — `--engine`, a VM file's `emulator.engine`, or
    // the shell's Engine setting. It refuses before reading or creating
    // anything, for the reason a host without the platform is refused in
    // `run_prepared`: a run that silently went to the interpreter under the
    // hypervisor's name is a measurement nobody can trust.
    #[cfg(not(all(not(feature = "guest-trace"), feature = "hv-whp", windows)))]
    if config.engine == Engine::Whp {
        return Err(RunError::NoHypervisorEngine);
    }

    // A blank BIOS path names no file: the blank "New VM" powered on as it
    // is. It is refused as the missing setting it is, before a read that
    // could only report an empty path.
    if config.bios.as_os_str().is_empty() {
        return Err(RunError::MissingBios);
    }
    let bios_data = read_required_file("BIOS", &config.bios)?;
    let vga_data = match &config.vga_bios {
        Some(path) => Some(read_vga_bios_file(path)?),
        None => None,
    };
    let slots = resolve_media_slots(&config)?;
    prepare_configured_media_files(&config, create_startup_disks)?;

    Ok(PreparedRun {
        config,
        bios_data,
        vga_data,
        slots,
    })
}

/// Builds the machine `prepared` describes and runs it on `gui` until it
/// stops or `stop_flag` is raised.
fn run_prepared<G>(
    prepared: PreparedRun,
    gui: G,
    stop_flag: Option<Arc<AtomicBool>>,
) -> Result<RunSummary, RunError>
where
    G: BxGui + 'static,
{
    let PreparedRun {
        config,
        bios_data,
        vga_data,
        slots,
    } = prepared;

    let emulator_config = EmulatorConfig {
        // The GUI exposes both sizes, so a configuration that asks for a
        // guest larger than its host backing gets the overflow file it asked
        // for rather than one it forgot to decline.
        memory: MemorySize::partially_resident(
            mib_to_bytes("memory_mib", config.memory_mib)?,
            mib_to_bytes("host_memory_mib", config.host_memory_mib)?,
        ),
        memory_block_size: kib_to_bytes("memory_block_kib", config.memory_block_kib)?,
        ips: Ips::new(config.ips),
        pci_enabled: config.pci,
        pci_vga: config.pci_vga,
        sync_slowdown: config.sync_slowdown,
        sync_realtime: config.sync_realtime,
        smp_quantum: config.smp_quantum,
        cpuid_freq: config.cpuid_freq,
        cpu_params: cpu_params_for_engine(&config),
        // The engine decides who turns the device wheel, so it decides which
        // clock the wheel runs on.
        device_clock: device_clock_for(config.engine),
        ..EmulatorConfig::default()
    };

    let mut builder = MachineBuilder::new(emulator_config)
        .gui(gui)
        .bios(&bios_data)
        .boot_order(boot_sequence(&config.boot_order));
    if let Some(data) = &vga_data {
        builder = builder.vga_bios(data);
    }
    if let (Some(disk), Some(slot)) = (&config.disk, slots.disk) {
        let geometry = disk.geometry;
        builder = builder.disk_file(
            slot,
            path_to_str(&disk.path)?,
            GuestDiskGeometry::new(geometry.cylinders, geometry.heads, geometry.sectors_per_track),
        );
    }
    if let (Some(cdrom), Some(slot)) = (&config.cdrom, slots.cdrom) {
        builder = builder.cdrom_file(slot, path_to_str(&cdrom.path)?);
    }

    // The hypervisor engine, when this build has it and the caller asked.
    //
    // Returns from here rather than falling through, because the two paths
    // differ in how the guest is advanced, not only in the machine's type: a
    // machine on the hypervisor is stepped by `FastMachine` from this thread,
    // while an interpreter machine runs `run_interactive`'s own loop. What
    // they share is the arranging done before either, in `arrange_for_boot`.
    #[cfg(all(not(feature = "guest-trace"), feature = "hv-whp", windows))]
    if config.engine == Engine::Whp {
        // Asked for and absent is a refusal, not a silent fall back to the
        // interpreter: a caller who chose an engine wants to know it did not
        // get it, and a boot that quietly ran somewhere else is a measurement
        // nobody can trust.
        if !rusty_box_whp_engine::hypervisor_present().unwrap_or(false) {
            return Err(RunError::NoHypervisor);
        }
        let emu = builder.build_on::<WhpEngine>()?;
        return drive_on_the_hypervisor(emu, &config, stop_flag);
    }

    #[cfg(not(feature = "guest-trace"))]
    let emu = builder.build()?;
    // Diagnostic build: run the CPU with the guest-death tracer installed.
    // Single-CPU only — one tracer instance cannot be shared between processors.
    #[cfg(feature = "guest-trace")]
    let mut emu = {
        let trace_log = crate::guest_trace::GuestTracer::default_log_path();
        let tracer = crate::guest_trace::GuestTracer::create(&trace_log).map_err(|source| {
            RunError::FileRead {
                kind: "guest-trace log",
                path: PathBuf::from(&trace_log),
                source,
            }
        })?;
        tracing::info!("guest-trace: recording guest evidence to {trace_log}");
        builder.tracer(tracer).build()?
    };

    drive(emu, &config, stop_flag)
}

/// Everything a built machine needs arranged before it runs, whichever engine
/// runs it.
///
/// One function for both drive paths (R5): the front end's stop control, its
/// pre-boot video mode and its boot keystroke are wired here or nowhere, so
/// none of them can work on one engine and quietly not on the other.
fn arrange_for_boot<E>(
    emu: &mut Emulator<(), E>,
    config: &ResolvedConfig,
    stop_flag: Option<Arc<AtomicBool>>,
) where
    E: SliceEngine<()>,
{
    if let Some(stop_flag) = stop_flag {
        emu.set_stop_flag(stop_flag);
    }
    // Apply the pre-boot VBE mode after reset (reset re-defaults the VGA, and
    // VgaCore::set_preferred_mode persists it across any later guest-triggered
    // reset). Raises the DISPI caps so the guest may select this resolution.
    if let Some(mode) = config.vga_mode {
        emu.display()
            .set_preferred_mode(mode.width, mode.height, mode.bpp);
    }
    // The last step of the machine's own bring-up: anchors the PIT, the ACPI
    // timer and the VGA retrace to the instruction rate the BIOS's calibration
    // loops read. Idempotent, so `run_interactive` repeating it changes
    // nothing.
    emu.prepare_run();
    if should_prequeue_boot_enter(&config.boot_order) {
        // The keystroke a CD-ROM boot loader's prompt waits for. A refused
        // keystroke is a boot that sits at that prompt, so it is said rather
        // than assumed.
        if emu.keyboard().type_text("\n") == 0 {
            tracing::warn!(
                "the boot keystroke was not accepted; the boot prompt may wait for a key"
            );
        }
    }
}

/// Bring a built machine up and run it on the interpreter's own loop.
///
/// Generic over the engine because `run_interactive` is: any engine that runs
/// the machine in slices is driven this way. A machine on the hypervisor is
/// not — see [`drive_on_the_hypervisor`].
fn drive<E>(
    mut emu: Box<Emulator<(), E>>,
    config: &ResolvedConfig,
    stop_flag: Option<Arc<AtomicBool>>,
) -> Result<RunSummary, RunError>
where
    E: SliceEngine<()>,
{
    arrange_for_boot(emu.as_mut(), config, stop_flag);
    let instructions_executed = emu.run_interactive(config.max_instructions)?;

    Ok(RunSummary {
        instructions_executed: Some(instructions_executed),
    })
}

/// How much guest time one hypervisor step covers, in milliseconds at the
/// machine's own instruction rate.
///
/// The step is the latency ceiling for every host action: "Power Off",
/// "Reset" and a keystroke are all seen between steps, because nothing inside
/// one looks at the stop flag or the input queue. Ten milliseconds is under
/// what a person notices on a key and a quarter of the 40 ms VGA refresh
/// period, so a frame is drawn from a machine that paused at most one step
/// ago. Shorter would buy nothing visible and cost a park and a resume per
/// step — measured at 0.06 to 0.24 ms each in `FastMachine::step` — which at
/// a hundred steps a second is one or two percent of the guest's time and at
/// a thousand would be a fifth of it.
#[cfg(all(not(feature = "guest-trace"), feature = "hv-whp", windows))]
const HYPERVISOR_STEP_MILLIS: u64 = 10;

/// How often the front end is redrawn: the 25 frames a second of Bochs's VGA
/// update timer (vga.cc `vga_update_interval`), which `run_interactive` also
/// keeps.
#[cfg(all(not(feature = "guest-trace"), feature = "hv-whp", windows))]
const FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(40);

/// Drive a machine that runs its guest on the hypervisor.
///
/// Separate from [`drive`] rather than a branch inside it: a hypervisor
/// machine is advanced by `FastMachine::step`, which owns the vCPU thread and
/// the device wheel, while an interpreter machine is advanced by
/// `run_interactive`'s own loop. The two share their setup — the stop flag,
/// the pre-boot video mode, the queued boot key, in [`arrange_for_boot`] — and
/// nothing else.
///
/// What `run_interactive` does inside its loop is done here between steps,
/// while the machine is paused: host input is pumped into the devices, the
/// guest's console output is handed on, and the front end is redrawn. The
/// stop flag is read between steps too, which is what makes the step length
/// the front end's latency — see [`HYPERVISOR_STEP_MILLIS`].
#[cfg(all(not(feature = "guest-trace"), feature = "hv-whp", windows))]
fn drive_on_the_hypervisor(
    mut emu: Box<Emulator<(), WhpEngine>>,
    config: &ResolvedConfig,
    stop_flag: Option<Arc<AtomicBool>>,
) -> Result<RunSummary, RunError> {
    // Refused, not approximated: nothing on the hypervisor counts the guest's
    // instructions, so a limit in them would end the run at a guess, and a
    // caller who asked for a limit wants to know it did not get one.
    if config.max_instructions != u64::MAX {
        return Err(RunError::InstructionBudgetOnHypervisor {
            max_instructions: config.max_instructions,
        });
    }
    // This loop is what reads the flag, so a run given none gets one nobody
    // will raise rather than a branch on every step.
    let stop_flag = stop_flag.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    arrange_for_boot(emu.as_mut(), config, Some(Arc::clone(&stop_flag)));
    let step = RunBudget::Ticks(
        (emu.config().ips.per_second_u64() * HYPERVISOR_STEP_MILLIS / 1_000).max(1),
    );

    // Adoption hands the processor to a thread of its own and the devices to a
    // thread that wakes at their deadlines; nothing runs until a step says so.
    let mut machine =
        FastMachine::adopt(emu).map_err(|source| RunError::Hypervisor { source })?;
    machine.with_machine(|m| {
        // The status bar shows an instruction rate, and this machine has none
        // to show: told zero it reads `---`, rather than the last rate of a run
        // on the other engine.
        if let Some(gui) = m.gui_mut() {
            gui.show_ips(0);
        }
        m.display().force_update();
        m.update_gui();
    });

    let mut last_frame = std::time::Instant::now();
    loop {
        if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let outcome = machine
            .step(step)
            .map_err(|source| RunError::Hypervisor { source })?;
        // Between steps the machine is paused, which is the only time its
        // shadow processor and its devices can be read coherently.
        machine.with_machine(|m| {
            m.pump_gui_input();
            m.present_console_output();
            if last_frame.elapsed() >= FRAME_INTERVAL {
                m.update_gui();
                last_frame = std::time::Instant::now();
            }
        });
        match outcome.stop {
            StepStop::BudgetSpent => {}
            // The guest asked to be off. Its last frame was drawn above.
            StepStop::GuestPowerOff => break,
            StepStop::Faulted(fault) => {
                return Err(RunError::Hypervisor {
                    source: FastMachineFault::Engine(fault),
                });
            }
        }
    }

    Ok(RunSummary {
        // Nothing here counted, so nothing is claimed — see `RunSummary`.
        instructions_executed: None,
    })
}

/// Which clock the machine's devices run on, decided by the engine (R2).
///
/// An interpreter machine turns its own wheel as it retires instructions. A
/// machine on the hypervisor cannot: its processor runs on the host's silicon
/// and nothing counts what it retires, so a wheel measured in instructions
/// would never turn. `FastMachine` drives that wheel from a host clock on a
/// thread of its own, and refuses a machine configured to keep it in ticks.
#[cfg(all(not(feature = "guest-trace"), feature = "hv-whp", windows))]
fn device_clock_for(engine: Engine) -> DeviceClock {
    match engine {
        Engine::Interpreter => DeviceClock::Ticks,
        Engine::Whp => DeviceClock::HostTime,
    }
}

/// Without the hypervisor engine in this build, every machine is an
/// interpreter machine whatever engine was asked for, and an interpreter
/// machine turns its own wheel in ticks.
#[cfg(not(all(not(feature = "guest-trace"), feature = "hv-whp", windows)))]
fn device_clock_for(_engine: Engine) -> DeviceClock {
    DeviceClock::Ticks
}

/// The processor the machine offers its guest, narrowed for the engine that
/// will run it.
///
/// A machine on the hypervisor must be narrowed to what the host's silicon can
/// hold, whatever profile it was given: `XSETBV` is not a trapped instruction
/// on the partition, so a guest that believes the preset enables a register
/// file the host lacks makes the platform refuse its first 64-bit context
/// write-back — measured as ISOLINUX handing off to the Alpine kernel, then a
/// `Host`-kind engine fault. `HostShared` is therefore forced for that engine
/// rather than left to the `--cpu-capabilities` flag, which defaults to the
/// full `Preset`. The interpreter offers whatever the profile asked for.
#[cfg(all(not(feature = "guest-trace"), feature = "hv-whp", windows))]
fn cpu_params_for_engine(config: &ResolvedConfig) -> BxParams {
    let capabilities = match config.engine {
        Engine::Whp => CpuCapabilities::HostShared,
        Engine::Interpreter => config.cpu_capabilities,
    };
    capabilities.narrow(config.cpu_params.clone())
}

/// Without the hypervisor engine in this build, every machine runs on the
/// interpreter, which offers exactly the profile's processor.
#[cfg(not(all(not(feature = "guest-trace"), feature = "hv-whp", windows)))]
fn cpu_params_for_engine(config: &ResolvedConfig) -> BxParams {
    config.cpu_capabilities.narrow(config.cpu_params.clone())
}

/// What the egui shell opens on: the VM library, the VM it shows first, and
/// the notice it opens with.
#[cfg(feature = "gui-egui")]
pub struct ShellStart {
    pub library: crate::library::VmLibrary,
    pub opening: ShellOpening,
    /// The message the shell shows when it opens, telling the user what the
    /// start could not do — a bundled VM it could not import, say. `None`
    /// when the start did everything it set out to.
    pub notice: Option<String>,
}

/// The VM the shell shows first.
#[cfg(feature = "gui-egui")]
#[derive(Debug)]
pub enum ShellOpening {
    /// The library VM the shell showed last, or a blank temporary "New VM"
    /// when the library holds none.
    LastShown,
    /// This library VM, selected: the one whose file the command line named
    /// on its own.
    LibraryVm(crate::library::VmStem),
    /// The machine the command line described, shown first in the VM list
    /// as a temporary VM, selected, and written to the library only when the
    /// user keeps it.
    Launch(LaunchVm),
}

/// The machine the command line described, as the shell lists it.
#[cfg(feature = "gui-egui")]
#[derive(Debug)]
pub struct LaunchVm {
    pub name: String,
    pub config: ResolvedConfig,
    pub source: LaunchSource,
}

/// Where the command line's machine came from.
#[cfg(feature = "gui-egui")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchSource {
    /// Exactly the file `--config` named: nothing else on the command line
    /// changes the machine it describes ([`Args::names_only_a_config_file`]).
    ConfigFile(PathBuf),
    /// Flags, alone or over a `--config` file.
    Flags,
}

#[cfg(feature = "gui-egui")]
impl LaunchVm {
    /// Named after the `--config` file it came from, or "Command line" when
    /// flags alone described it.
    pub fn from_args(args: &Args, config: ResolvedConfig) -> Self {
        let name = args
            .config
            .as_deref()
            .and_then(|path| path.file_stem())
            .and_then(|stem| stem.to_str())
            .map_or_else(|| "Command line".to_owned(), str::to_owned);
        let source = match &args.config {
            Some(path) if args.names_only_a_config_file() => {
                LaunchSource::ConfigFile(path.clone())
            }
            Some(_) | None => LaunchSource::Flags,
        };
        Self {
            name,
            config,
            source,
        }
    }
}

/// Opens the desktop shell on the per-user VM library, with `launch` — what
/// the command line described, if anything — as [`shell_start`] places it.
///
/// Only a library folder that cannot be found or created refuses the
/// launch: with no folder there is nothing to show. A library that cannot be
/// read opens as empty, with the error as a notice in the window, and a
/// bundled VM that cannot be imported is left out, with the reason as the
/// shell's opening notice: no subscriber is installed this early, so a log
/// line alone would reach nobody.
#[cfg(all(feature = "gui-egui", not(target_os = "android")))]
pub fn run_shell(launch: Option<LaunchVm>) -> Result<RunSummary, RunError> {
    let dir = crate::library::default_library_dir().ok_or(RunError::NoLibraryFolder)?;
    let library = crate::library::VmLibrary::open(dir)?;
    run_egui(shell_start(library, launch))
}

/// What the shell opens on over `library` for `launch`, and the notice it
/// opens with: for the library VM the command line named, why the next
/// launch will not open on it; for any other opening, what a first run's
/// import could not do.
#[cfg(all(feature = "gui-egui", not(target_os = "android")))]
fn shell_start(library: crate::library::VmLibrary, launch: Option<LaunchVm>) -> ShellStart {
    let opening = shell_opening(&library, launch);
    let notice = match &opening {
        ShellOpening::LibraryVm(stem) => remember_named_vm(&library, stem),
        ShellOpening::LastShown | ShellOpening::Launch(_) => first_run_notice(&library),
    };
    ShellStart {
        library,
        opening,
        notice,
    }
}

/// The rule for a `--config` file that is already in the library: when the
/// command line named that file and nothing else that changes the machine
/// (`LaunchSource::ConfigFile`), and the file is one of `library`'s own VM
/// files (`VmLibrary::stem_of_file`), the shell opens on that library VM
/// rather than on a temporary copy of it. Anything else the command line
/// described stays a temporary VM, the only shape that can carry an
/// override such as `--memory-mib 64` without writing it back to the file.
#[cfg(all(feature = "gui-egui", not(target_os = "android")))]
fn shell_opening(library: &crate::library::VmLibrary, launch: Option<LaunchVm>) -> ShellOpening {
    let Some(launch) = launch else {
        return ShellOpening::LastShown;
    };
    let stem = match &launch.source {
        LaunchSource::ConfigFile(path) => library.stem_of_file(path),
        LaunchSource::Flags => None,
    };
    match stem {
        Some(stem) => ShellOpening::LibraryVm(stem),
        None => ShellOpening::Launch(launch),
    }
}

/// Records `stem`, the library VM the command line named, as the VM the
/// next launch shows. The shell selects it either way, so a record that
/// cannot be written costs only the next launch, and the message says so.
#[cfg(all(feature = "gui-egui", not(target_os = "android")))]
fn remember_named_vm(
    library: &crate::library::VmLibrary,
    stem: &crate::library::VmStem,
) -> Option<String> {
    match library.remember_selected(stem) {
        Ok(()) => None,
        Err(error) => {
            let message = format!(
                "{} is shown, but the next launch will not open on it: {error}",
                library.path_of(stem).display()
            );
            tracing::warn!("{message}");
            Some(message)
        }
    }
}

/// A first run's empty library gets the bundled VM ([`import_bundled_vm`]);
/// returns what that could not do. A library that cannot be read is not
/// imported into: the shell reads the library itself and shows that error as
/// its notice, so nothing is said twice.
#[cfg(all(feature = "gui-egui", not(target_os = "android")))]
fn first_run_notice(library: &crate::library::VmLibrary) -> Option<String> {
    match library.is_empty() {
        Ok(true) => import_bundled_vm(library),
        Ok(false) => None,
        Err(error) => {
            tracing::warn!(
                "the VM library could not be read, so no bundled VM is imported: {error}"
            );
            None
        }
    }
}

/// A first run's empty library gets the VM a `rusty_box.toml` beside the
/// executable describes, and the shell opens on it. Returns the message the
/// shell shows when that could not be done in full; an executable whose
/// folder cannot be found has nothing bundled to import.
#[cfg(all(feature = "gui-egui", not(target_os = "android")))]
fn import_bundled_vm(library: &crate::library::VmLibrary) -> Option<String> {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            let message = format!(
                "The executable could not be located, so no bundled VM was imported: {error}"
            );
            tracing::warn!("{message}");
            return Some(message);
        }
    };
    let exe_dir = exe.parent()?;
    import_bundled_vm_from(library, exe_dir)
}

/// Imports the VM `exe_dir`'s `rusty_box.toml` describes and records it as
/// the VM the shell shows next. Nothing here refuses the launch: the message
/// returned says what could not be done — the file did not load or the
/// library could not take it, or it was imported but the record of it could
/// not be written, so the next launch will not open on it — and the shell
/// opens on the library as it stands. `None` when there was nothing to
/// import or everything was done.
#[cfg(all(feature = "gui-egui", not(target_os = "android")))]
fn import_bundled_vm_from(library: &crate::library::VmLibrary, exe_dir: &Path) -> Option<String> {
    let stem = match library.import_bundled_vm(exe_dir) {
        Ok(Some(stem)) => stem,
        Ok(None) => return None,
        Err(error) => {
            let message = format!(
                "The VM bundled in {} was not imported: {error}",
                exe_dir.display()
            );
            tracing::warn!("{message}");
            return Some(message);
        }
    };
    match library.remember_selected(&stem) {
        Ok(()) => None,
        Err(error) => {
            let message = format!(
                "The VM bundled in {} was imported, but the next launch will not open on it: \
                 {error}",
                exe_dir.display()
            );
            tracing::warn!("{message}");
            Some(message)
        }
    }
}

/// The desktop shell: a window of its own over the emulator thread.
#[cfg(all(feature = "gui-egui", not(target_os = "android")))]
fn run_egui(start: ShellStart) -> Result<RunSummary, RunError> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true)
            .with_title("Rusty Box Workstation"),
        ..Default::default()
    };
    run_egui_shell(start, native_options, crate::app::NativeShellApp::new)
}

/// On Android the shell needs the activity, which only `android_main` holds,
/// so the egui backend starts from [`crate::android::main`] and never here.
#[cfg(all(feature = "gui-egui", target_os = "android"))]
fn run_egui(_config: ResolvedConfig) -> Result<RunSummary, RunError> {
    Err(RunError::Gui {
        message: "on Android the egui shell starts from the NativeActivity entry point, \
                  rusty_box_gui::android::main"
            .to_owned(),
    })
}

/// The file eframe's storage is opened on, in the app's storage. Nothing is
/// ever put in the storage, so the file is never written: the storage exists
/// so that eframe calls `App::save` when the activity's window is taken away
/// (`Event::Suspended`, from `surfaceDestroyed`), the last call the shell
/// gets before Android may kill the process, and the shell writes the edits
/// still in memory then. Without a storage eframe makes no such call, and
/// without a path of its own it would look for a per-user folder, which
/// Android has none of.
#[cfg(all(feature = "gui-egui", target_os = "android"))]
const EFRAME_STATE_FILE: &str = "eframe.ron";

/// The shell a phone shows for `app`, the activity NativeActivity handed this
/// process; `storage` is the app's private storage.
#[cfg(all(feature = "gui-egui", target_os = "android"))]
pub(crate) fn run_android_shell(
    start: ShellStart,
    app: crate::android::AndroidApp,
    storage: &Path,
) -> Result<RunSummary, RunError> {
    let native_options = eframe::NativeOptions {
        android_app: Some(app.clone()),
        persistence_path: Some(storage.join(EFRAME_STATE_FILE)),
        persist_window: false,
        ..Default::default()
    };
    run_egui_shell(start, native_options, move |cc, shared, command_tx, start| {
        crate::android::AndroidShellApp::new(cc, shared, command_tx, start, app)
    })
}

/// Runs an egui shell over the emulator thread. `make_app` builds the
/// window's app from the display the two threads share, the channel the
/// shell starts machines through, and the VM list the shell opens on.
#[cfg(feature = "gui-egui")]
fn run_egui_shell<A, F>(
    start: ShellStart,
    native_options: eframe::NativeOptions,
    make_app: F,
) -> Result<RunSummary, RunError>
where
    A: eframe::App + 'static,
    F: FnOnce(
            &eframe::CreationContext<'_>,
            Arc<Mutex<SharedDisplay>>,
            mpsc::Sender<crate::app::NativeEmulatorCommand>,
            ShellStart,
        ) -> A
        + 'static,
{
    let shared = Arc::new(Mutex::new(SharedDisplay::new()));
    let (command_tx, command_rx) = mpsc::channel();
    let shared_for_emu = Arc::clone(&shared);
    let emulator_thread = std::thread::Builder::new()
        .name("rusty_box_gui_emulator".to_owned())
        .stack_size(1500 * 1024 * 1024)
        .spawn(move || run_egui_emulator_loop(command_rx, shared_for_emu))
        .map_err(|source| RunError::ThreadStart { source })?;

    let shared_for_gui = Arc::clone(&shared);
    let gui_result = eframe::run_native(
        "Rusty Box Workstation",
        native_options,
        Box::new(move |cc| Ok(Box::new(make_app(cc, shared_for_gui, command_tx, start)))),
    );

    signal_egui_stop(&shared);
    let emulator_result = emulator_thread
        .join()
        .map_err(|_| RunError::EmulatorThreadPanic)?;
    gui_result.map_err(|source| RunError::Gui {
        message: source.to_string(),
    })?;
    emulator_result
}

/// `config`'s startup-disk creation, when it has one.
#[cfg(feature = "gui-egui")]
fn startup_disk_creation(config: &ResolvedConfig) -> Option<&crate::config::ResolvedDiskCreation> {
    config.disk.as_ref()?.creation.as_ref()
}

#[cfg(feature = "gui-egui")]
fn run_egui_emulator_loop(
    command_rx: mpsc::Receiver<crate::app::NativeEmulatorCommand>,
    shared: Arc<Mutex<SharedDisplay>>,
) -> Result<RunSummary, RunError> {
    let mut instructions_executed = Some(0u64);
    // The overwrite creations provisioned this session, by path. An overwrite
    // creation erases its file at the first power-on of the session that
    // uses the path and at no later one, so a file the user agreed to have
    // erased is erased once; the shell asks about each such file once per
    // session by the same rule (`NativeShellApp::overwrite_confirmed`). A
    // plain creation is provisioned at every power-on and never recorded
    // here: it only creates a missing file and reuses a valid one
    // (`disk_images::provision_startup_disk`), so it erases nothing.
    let mut provisioned: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    while let Ok(command) = command_rx.recv() {
        match command {
            crate::app::NativeEmulatorCommand::Start(config) => loop {
                let stop_flag = prepare_egui_run(&shared);
                let creation = startup_disk_creation(&config);
                let create_startup_disks = creation
                    .is_some_and(|creation| !creation.overwrite || !provisioned.contains(&creation.path));
                // Warn in the window that a (possibly slow) disk allocation is
                // about to happen, before the run blocks on it. Reusing an
                // existing image returns None here (instant, no warning).
                if create_startup_disks {
                    if let Some(status) = startup_disk_action(&config) {
                        if let Ok(mut display) = shared.lock() {
                            display.startup_status = Some(status);
                        }
                    }
                }
                let bridge = BridgeGui::new(Arc::clone(&shared));
                let run = prepare_run(config.clone(), create_startup_disks).and_then(|prepared| {
                    // The disk is provisioned once `prepare_run` returns, and
                    // the run after it can still fail, so an overwrite path is
                    // recorded here and not after the run: a later power-on
                    // must not erase the file a second time.
                    provisioned.extend(
                        creation
                            .filter(|creation| creation.overwrite)
                            .map(|creation| creation.path.clone()),
                    );
                    run_prepared(prepared, bridge, Some(stop_flag))
                });
                let summary = match run {
                    Ok(summary) => summary,
                    Err(error) => {
                        record_egui_error(&shared, &error);
                        break;
                    }
                };
                // A total is a total only while every run counted. One run on
                // the hypervisor makes the sum a guess, and a guess is not
                // reported as a count.
                instructions_executed =
                    match (instructions_executed, summary.instructions_executed) {
                        (Some(total), Some(counted)) => Some(total.saturating_add(counted)),
                        (None, _) | (_, None) => None,
                    };

                let restart_requested = finish_egui_run(&shared);
                if !restart_requested {
                    break;
                }
            },
        }
    }

    Ok(RunSummary {
        instructions_executed,
    })
}

#[cfg(feature = "gui-egui")]
fn prepare_egui_run(shared: &Arc<Mutex<SharedDisplay>>) -> Arc<AtomicBool> {
    if let Ok(mut display) = shared.lock() {
        display.stop_flag.store(false, Ordering::Relaxed);
        display.emu_running = true;
        display.start_pending = false;
        display.reset_requested = false;
        display.runtime_error = None;
        display.startup_status = None;
        // Input queued while the machine was off belongs to no run: serial
        // text typed, or keys tapped on the phone's key pad, are not replayed
        // into the next boot.
        drop(display.drain_serial_input());
        display.pending_keys.clear();
        Arc::clone(&display.stop_flag)
    } else {
        Arc::new(AtomicBool::new(false))
    }
}

#[cfg(feature = "gui-egui")]
fn finish_egui_run(shared: &Arc<Mutex<SharedDisplay>>) -> bool {
    if let Ok(mut display) = shared.lock() {
        let restart_requested = display.reset_requested;
        display.emu_running = false;
        display.start_pending = false;
        display.startup_status = None;
        restart_requested
    } else {
        false
    }
}

#[cfg(feature = "gui-egui")]
fn record_egui_error(shared: &Arc<Mutex<SharedDisplay>>, error: &RunError) {
    if let Ok(mut display) = shared.lock() {
        display.emu_running = false;
        display.start_pending = false;
        display.reset_requested = false;
        display.runtime_error = Some(format!("Emulator startup failed: {error}"));
        display.startup_status = None;
    }
}

#[cfg(feature = "gui-egui")]
fn signal_egui_stop(shared: &Arc<Mutex<SharedDisplay>>) {
    if let Ok(mut display) = shared.lock() {
        display.emu_running = false;
        display.stop_flag.store(true, Ordering::Relaxed);
        display.start_pending = false;
    }
}

/// Install the log subscriber.
///
/// `--log-level` sets the global level, and `RUST_LOG` (when set) overrides it
/// with the usual per-module syntax so a noisy subsystem can be silenced
/// without dropping the level everywhere. Several messages are deliberately
/// `info!` because Bochs emits the same text at `BX_INFO` — SMI entry/exit
/// (`cpu/smm.cc`), `cpu N hardware reset` (`cpu/init.cc`) and
/// `allocate APIC id=` (`iodev/apic.cc`) — and release builds compile out
/// `debug!` via `release_max_level_info`, so demoting them would make them
/// unavailable exactly where boot problems get diagnosed. To quiet just those:
///
/// ```text
/// RUST_LOG=info,rusty_box::cpu::smm=warn
/// ```
fn init_tracing(log_level: LogLevel) {
    let level = match log_level {
        LogLevel::Trace => tracing::Level::TRACE,
        LogLevel::Debug => tracing::Level::DEBUG,
        LogLevel::Info => tracing::Level::INFO,
        LogLevel::Warn => tracing::Level::WARN,
        LogLevel::Error => tracing::Level::ERROR,
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level.to_string()));

    match tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_env_filter(filter)
        .try_init()
    {
        Ok(()) => {}
        Err(error) => {
            tracing::debug!(?error, "tracing subscriber already initialized");
        }
    }
}

fn read_required_file(kind: &'static str, path: &Path) -> Result<Vec<u8>, RunError> {
    let data = fs::read(path).map_err(|source| RunError::FileRead {
        kind,
        path: path.to_owned(),
        source,
    })?;
    if data.is_empty() {
        return Err(RunError::EmptyFile {
            kind,
            path: path.to_owned(),
        });
    }
    Ok(data)
}

fn read_vga_bios_file(path: &Path) -> Result<Vec<u8>, RunError> {
    let data = fs::read(path).map_err(|source| RunError::FileRead {
        kind: "VGA BIOS",
        path: path.to_owned(),
        source,
    })?;
    if data.is_empty() || data.len() % 512 != 0 {
        return Err(RunError::InvalidVgaBiosSize {
            path: path.to_owned(),
            len: data.len(),
        });
    }
    Ok(data)
}

fn verify_media_file(kind: &'static str, path: &Path) -> Result<(), RunError> {
    let metadata = fs::metadata(path).map_err(|source| RunError::FileRead {
        kind,
        path: path.to_owned(),
        source,
    })?;
    if metadata.len() == 0 {
        return Err(RunError::EmptyFile {
            kind,
            path: path.to_owned(),
        });
    }
    Ok(())
}

fn mib_to_bytes(field: &'static str, mib: u32) -> Result<usize, RunError> {
    bytes_from_units(field, mib, 1024 * 1024)
}

fn kib_to_bytes(field: &'static str, kib: u32) -> Result<usize, RunError> {
    bytes_from_units(field, kib, 1024)
}

fn bytes_from_units(field: &'static str, value: u32, scale: u64) -> Result<usize, RunError> {
    let bytes = u64::from(value)
        .checked_mul(scale)
        .ok_or(RunError::ValueOverflow { field })?;
    usize::try_from(bytes).map_err(|_| RunError::ValueOverflow { field })
}

fn boot_sequence(boot_order: &[BootDevice]) -> BootOrder {
    let mut positions = [GuestBootDevice::None; 3];
    for (index, device) in boot_order.iter().take(3).enumerate() {
        positions[index] = match device {
            BootDevice::Disk => GuestBootDevice::Disk,
            BootDevice::Cdrom => GuestBootDevice::Cdrom,
        };
    }
    BootOrder::new(positions[0], positions[1], positions[2])
}

/// Where the configured media hang off the ATA controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct MediaSlots {
    disk: Option<AtaSlot>,
    cdrom: Option<AtaSlot>,
}

/// The one place a configured channel/drive pair becomes an [`AtaSlot`], so
/// an out-of-range socket and a double-booked one are both rejected once.
fn resolve_media_slots(config: &ResolvedConfig) -> Result<MediaSlots, RunError> {
    let disk = match &config.disk {
        Some(disk) => Some(ata_slot(
            "disk.channel",
            "disk.drive",
            disk.channel,
            disk.drive,
        )?),
        None => None,
    };
    let cdrom = match &config.cdrom {
        Some(cdrom) => Some(ata_slot(
            "cdrom.channel",
            "cdrom.drive",
            cdrom.channel,
            cdrom.drive,
        )?),
        None => None,
    };
    if let (Some(disk_slot), Some(cdrom_slot), Some(cdrom)) = (disk, cdrom, &config.cdrom) {
        if disk_slot == cdrom_slot {
            return Err(RunError::MediaAttach {
                kind: "CD-ROM",
                path: cdrom.path.clone(),
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "ATA slot already used by disk",
                ),
            });
        }
    }
    Ok(MediaSlots { disk, cdrom })
}

fn ata_slot(
    channel_field: &'static str,
    drive_field: &'static str,
    channel: usize,
    drive: usize,
) -> Result<AtaSlot, RunError> {
    if channel > 1 {
        return Err(RunError::ValueOverflow {
            field: channel_field,
        });
    }
    AtaSlot::new(channel, drive).ok_or(RunError::ValueOverflow { field: drive_field })
}

fn should_prequeue_boot_enter(boot_order: &[BootDevice]) -> bool {
    boot_order.first() == Some(&BootDevice::Cdrom)
}

fn path_to_str(path: &PathBuf) -> Result<&str, RunError> {
    path.to_str()
        .ok_or_else(|| RunError::NonUtf8Path { path: path.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::DiskGeometry;
    use crate::config::{
        CpuCapabilities, Engine, ResolvedCdrom, ResolvedDisk, ResolvedDiskCreation,
    };
    use rusty_box::params::BxParams;
    use rusty_box_bximage::ImageSize;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn empty_vga_bios_returns_vga_size_error() {
        let dir = std::env::temp_dir();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let bios = dir.join(format!("rusty_box_gui_bios_{suffix}.bin"));
        let vga = dir.join(format!("rusty_box_gui_vga_{suffix}.bin"));
        fs::write(&bios, [0xEA]).unwrap();
        fs::write(&vga, []).unwrap();

        let error = run_resolved(ResolvedConfig {
            engine: Engine::Interpreter,
            cpu_capabilities: CpuCapabilities::Preset,
            memory_mib: 32,
            host_memory_mib: 32,
            memory_block_kib: 128,
            ips: 4_000_000,
            pci: true,
            sync_slowdown: false,
            sync_realtime: false,
            smp_quantum: 16,
            cpuid_freq: rusty_box::CpuidFreq::None,
            max_instructions: 0,
            cpu_params: BxParams::default(),
            display: DisplayBackend::Headless,
            bios: bios.clone(),
            vga_bios: Some(vga.clone()),
            boot_order: Vec::new(),
            disk: None::<ResolvedDisk>,
            cdrom: None::<ResolvedCdrom>,
            log_level: LogLevel::Warn,
            vga_mode: None,
            pci_vga: false,
        })
        .unwrap_err();

        remove_test_file(&bios);
        remove_test_file(&vga);

        assert!(matches!(
            error,
            RunError::InvalidVgaBiosSize { path, len: 0 } if path == vga
        ));
    }

    fn remove_test_file(path: &Path) {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove {}: {error}", path.display()),
        }
    }

    fn unique_temp_path(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{}-{suffix}.img", std::process::id()))
    }

    fn disk_creation_config(path: PathBuf, overwrite: bool) -> ResolvedConfig {
        ResolvedConfig {
            engine: Engine::Interpreter,
            cpu_capabilities: CpuCapabilities::Preset,
            memory_mib: 32,
            host_memory_mib: 32,
            memory_block_kib: 128,
            ips: 4_000_000,
            pci: true,
            sync_slowdown: false,
            sync_realtime: false,
            smp_quantum: 16,
            cpuid_freq: rusty_box::CpuidFreq::None,
            max_instructions: 0,
            cpu_params: BxParams::default(),
            display: DisplayBackend::Headless,
            bios: unique_temp_path("rusty-box-gui-bios"),
            vga_bios: None,
            boot_order: vec![BootDevice::Disk],
            disk: Some(ResolvedDisk {
                path: path.clone(),
                geometry: DiskGeometry {
                    cylinders: 20,
                    heads: 16,
                    sectors_per_track: 63,
                },
                channel: 0,
                drive: 0,
                creation: Some(ResolvedDiskCreation {
                    path,
                    size: ImageSize::mib(10),
                    overwrite,
                }),
            }),
            cdrom: None::<ResolvedCdrom>,
            log_level: LogLevel::Warn,
            vga_mode: None,
            pci_vga: false,
        }
    }

    #[test]
    fn startup_disk_creation_creates_file_before_verification() {
        let disk = unique_temp_path("rusty-box-gui-created-disk");
        let config = disk_creation_config(disk.clone(), false);
        fs::write(&config.bios, [0xEA]).unwrap();

        create_configured_disk_images(&config).unwrap();

        assert_eq!(fs::metadata(&disk).unwrap().len(), 10_321_920);
        remove_test_file(&disk);
        remove_test_file(&config.bios);
    }

    #[test]
    fn startup_disk_reuses_existing_valid_image() {
        let disk = unique_temp_path("rusty-box-gui-existing-disk");
        // A valid (non-empty, sector-aligned) image that differs in size from the
        // config: it must be kept as-is, not rewritten to the config size.
        fs::write(&disk, [0u8; 1024]).unwrap();
        let config = disk_creation_config(disk.clone(), false);

        create_configured_disk_images(&config).unwrap();

        assert_eq!(fs::metadata(&disk).unwrap().len(), 1024);
        remove_test_file(&disk);
    }

    #[test]
    fn startup_disk_rejects_invalid_existing_image() {
        let disk = unique_temp_path("rusty-box-gui-invalid-disk");
        // 1 byte is not a usable flat image (not 512-aligned).
        fs::write(&disk, [0x00]).unwrap();
        let config = disk_creation_config(disk.clone(), false);

        let error = create_configured_disk_images(&config).unwrap_err();

        assert!(matches!(
            error,
            RunError::InvalidExistingDiskImage { path, len: 1 } if path == disk
        ));
        remove_test_file(&disk);
    }

    #[test]
    fn startup_disk_overwrite_recreates_existing_image() {
        let disk = unique_temp_path("rusty-box-gui-overwrite-disk");
        fs::write(&disk, [0u8; 1024]).unwrap();
        let config = disk_creation_config(disk.clone(), true);

        create_configured_disk_images(&config).unwrap();

        // overwrite = true still forces a fresh full-size image.
        assert_eq!(fs::metadata(&disk).unwrap().len(), 10_321_920);
        remove_test_file(&disk);
    }

    /// The `whp` engine in a build that carries no hypervisor path is refused,
    /// and refused before the startup disk the configuration names is created.
    #[cfg(not(all(not(feature = "guest-trace"), feature = "hv-whp", windows)))]
    #[test]
    fn a_build_without_the_hypervisor_refuses_the_whp_engine_before_creating_media() {
        let disk = unique_temp_path("rusty-box-gui-no-engine-disk");
        let mut config = disk_creation_config(disk.clone(), false);
        config.engine = Engine::Whp;
        let bios = config.bios.clone();
        fs::write(&bios, [0xEA]).unwrap();

        let result = run_resolved(config);
        let disk_exists = fs::metadata(&disk).is_ok();
        if disk_exists {
            remove_test_file(&disk);
        }
        remove_test_file(&bios);

        assert!(
            matches!(result, Err(RunError::NoHypervisorEngine)),
            "expected the WHP engine to be refused, got {result:?}"
        );
        assert!(!disk_exists, "a refused run created its startup disk");
    }

    /// The blank "New VM" powered on as it is: refused as the missing setting
    /// it is, not as a failed read of an empty path.
    #[test]
    fn a_blank_bios_path_is_a_missing_bios_not_a_failed_read() {
        let mut config = crate::config::blank_config();
        config.display = DisplayBackend::Headless;

        let error = run_resolved(config).unwrap_err();

        assert!(
            matches!(error, RunError::MissingBios),
            "expected MissingBios, got {error:?}"
        );
    }

    /// A scratch folder, removed when the test ends.
    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    struct ScratchDir {
        path: PathBuf,
    }

    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    impl ScratchDir {
        fn new(name: &str) -> Self {
            let path = unique_temp_path(name).with_extension("");
            fs::create_dir_all(&path).expect("create scratch dir");
            Self { path }
        }
    }

    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    impl Drop for ScratchDir {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.path) {
                eprintln!("could not remove {}: {error}", self.path.display());
            }
        }
    }

    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    #[test]
    fn the_bundled_vm_is_imported_and_is_the_vm_shown_next() {
        let library_dir = ScratchDir::new("rusty-box-gui-bundled-library");
        let exe_dir = ScratchDir::new("rusty-box-gui-bundled-exe");
        let library = crate::library::VmLibrary::open(library_dir.path.clone()).expect("open");
        fs::write(
            exe_dir.path.join(crate::config::DEFAULT_CONFIG_FILE),
            "[rom]\nbios = \"bios.bin\"\n\n[cdrom]\npath = \"boot.iso\"\n",
        )
        .expect("write the bundled file");

        let message = import_bundled_vm_from(&library, &exe_dir.path);

        assert_eq!(message, None);
        let contents = library.load().expect("load");
        assert_eq!(contents.vms.len(), 1);
        assert_eq!(contents.vms[0].name, crate::library::DEFAULT_VM_NAME);
        assert_eq!(library.last_selected(), Some(contents.vms[0].stem.clone()));
    }

    /// A VM that was imported while only the record of it could not be
    /// written is in the library, and the message says which half failed.
    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    #[test]
    fn a_bundled_vm_whose_record_cannot_be_written_is_imported_and_says_so() {
        let library_dir = ScratchDir::new("rusty-box-gui-unrecorded-library");
        let exe_dir = ScratchDir::new("rusty-box-gui-unrecorded-exe");
        let library = crate::library::VmLibrary::open(library_dir.path.clone()).expect("open");
        fs::write(
            exe_dir.path.join(crate::config::DEFAULT_CONFIG_FILE),
            "[rom]\nbios = \"bios.bin\"\n\n[cdrom]\npath = \"boot.iso\"\n",
        )
        .expect("write the bundled file");
        // A folder where the record file goes: the rename over it fails.
        fs::create_dir(library_dir.path.join(".last")).expect("block the record");

        let message = import_bundled_vm_from(&library, &exe_dir.path).expect("a message");

        assert!(message.contains("was imported, but"), "{message:?}");
        assert!(message.contains("will not open on it"), "{message:?}");
        assert!(message.contains(&exe_dir.path.display().to_string()), "{message:?}");
        assert_eq!(library.load().expect("load").vms.len(), 1);
        assert_eq!(library.last_selected(), None);
    }

    /// One bad bundled file does not stop the shell from opening: the
    /// library stays empty and the launch goes on.
    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    #[test]
    fn a_bundled_file_that_does_not_load_leaves_the_library_empty() {
        let library_dir = ScratchDir::new("rusty-box-gui-bad-bundled-library");
        let exe_dir = ScratchDir::new("rusty-box-gui-bad-bundled-exe");
        let library = crate::library::VmLibrary::open(library_dir.path.clone()).expect("open");
        fs::write(
            exe_dir.path.join(crate::config::DEFAULT_CONFIG_FILE),
            "memory_mib = [",
        )
        .expect("write the bundled file");

        let message = import_bundled_vm_from(&library, &exe_dir.path).expect("a message");

        assert!(message.contains("was not imported"), "{message:?}");
        assert!(message.contains(&exe_dir.path.display().to_string()), "{message:?}");
        assert!(library.load().expect("load").is_empty());
        assert_eq!(library.last_selected(), None);
    }

    /// The launch VM is listed under its file's name, and under "Command
    /// line" when flags alone described it.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn the_launch_vm_is_named_after_its_config_file() {
        let from_file = Args {
            config: Some(PathBuf::from("machines").join("Alpine edge.toml")),
            ..Args::default()
        };
        let from_flags = Args {
            cdrom: Some(PathBuf::from("boot.iso")),
            ..Args::default()
        };

        let config = disk_creation_config(PathBuf::from("disk.img"), false);
        assert_eq!(
            LaunchVm::from_args(&from_file, config.clone()).name,
            "Alpine edge"
        );
        assert_eq!(LaunchVm::from_args(&from_flags, config).name, "Command line");
    }

    /// The launch VM the command line `line` describes, resolved the way
    /// `main` resolves it.
    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    fn launch_from_command_line(line: &[&str]) -> LaunchVm {
        use clap::Parser;
        let args = Args::try_parse_from(line).expect("the command line parses");
        let config = crate::config::load_config(&args).expect("the command line resolves");
        LaunchVm::from_args(&args, config)
    }

    /// A library in `dir` holding one VM, "Alpine", for a command line to
    /// name by its file.
    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    fn alpine_library(dir: &ScratchDir) -> crate::library::VmLibrary {
        let library = crate::library::VmLibrary::open(dir.path.clone()).expect("open");
        let mut config = disk_creation_config(PathBuf::from("disk.img"), false);
        config.display = DisplayBackend::Egui;
        library.create("Alpine", &config).expect("create");
        library
    }

    /// `--config` naming a file of the library, and nothing else: the shell
    /// opens on that library VM, remembered for the next launch, and there
    /// is no temporary VM to open.
    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    #[test]
    fn a_library_file_named_alone_opens_as_that_library_vm() {
        let dir = ScratchDir::new("rusty-box-gui-named-library");
        let library = alpine_library(&dir);
        let stem = crate::library::VmStem::parse("alpine").expect("a stem");
        let file = library.path_of(&stem);
        let launch = launch_from_command_line(&[
            "rusty_box_gui",
            "--config",
            file.to_str().expect("a UTF-8 scratch path"),
        ]);
        assert_eq!(launch.source, LaunchSource::ConfigFile(file.clone()));

        let start = shell_start(library.clone(), Some(launch));

        assert!(
            matches!(&start.opening, ShellOpening::LibraryVm(named) if named == &stem),
            "opening: {:?}",
            start.opening
        );
        assert_eq!(start.notice, None);
        assert_eq!(library.last_selected(), Some(stem.clone()));

        // A spelling that differs in case names the same file on Windows.
        if cfg!(windows) {
            let shouted = dir.path.join("ALPINE.TOML");
            let launch = launch_from_command_line(&[
                "rusty_box_gui",
                "--config",
                shouted.to_str().expect("a UTF-8 scratch path"),
            ]);
            let start = shell_start(library.clone(), Some(launch));
            assert!(
                matches!(&start.opening, ShellOpening::LibraryVm(named) if named == &stem),
                "opening: {:?}",
                start.opening
            );
        }
    }

    /// The same library file with an override beside it is a temporary VM:
    /// only a temporary VM carries the override without writing it back.
    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    #[test]
    fn a_library_file_with_an_override_opens_as_a_temporary_vm() {
        let dir = ScratchDir::new("rusty-box-gui-overridden-library");
        let library = alpine_library(&dir);
        let file = dir.path.join("alpine.toml");
        let file_arg = file.to_str().expect("a UTF-8 scratch path");
        let before = fs::read_to_string(&file).expect("read");

        let with_memory =
            launch_from_command_line(&["rusty_box_gui", "--config", file_arg, "--memory-mib", "64"]);
        let start = shell_start(library.clone(), Some(with_memory));
        assert!(
            matches!(&start.opening, ShellOpening::Launch(vm)
                if vm.config.memory_mib == 64 && vm.source == LaunchSource::Flags),
            "opening: {:?}",
            start.opening
        );

        let with_engine = launch_from_command_line(&[
            "rusty_box_gui",
            "--config",
            file_arg,
            "--engine",
            "interpreter",
        ]);
        let start = shell_start(library.clone(), Some(with_engine));
        assert!(
            matches!(&start.opening, ShellOpening::Launch(_)),
            "opening: {:?}",
            start.opening
        );

        assert_eq!(library.last_selected(), None);
        assert_eq!(fs::read_to_string(&file).expect("read"), before);
    }

    /// A file outside the library is a temporary VM, even one that is a copy
    /// of a library file under the same name.
    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    #[test]
    fn a_config_file_elsewhere_opens_as_a_temporary_vm() {
        let dir = ScratchDir::new("rusty-box-gui-library-beside-elsewhere");
        let elsewhere = ScratchDir::new("rusty-box-gui-elsewhere");
        let library = alpine_library(&dir);
        let copy = elsewhere.path.join("alpine.toml");
        fs::copy(dir.path.join("alpine.toml"), &copy).expect("copy the file out");

        let launch = launch_from_command_line(&[
            "rusty_box_gui",
            "--config",
            copy.to_str().expect("a UTF-8 scratch path"),
        ]);
        let start = shell_start(library.clone(), Some(launch));

        assert!(
            matches!(&start.opening, ShellOpening::Launch(vm)
                if vm.name == "alpine" && vm.source == LaunchSource::ConfigFile(copy.clone())),
            "opening: {:?}",
            start.opening
        );
        assert_eq!(library.last_selected(), None);
    }

    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    #[test]
    fn a_folder_with_nothing_bundled_imports_nothing() {
        let library_dir = ScratchDir::new("rusty-box-gui-unbundled-library");
        let exe_dir = ScratchDir::new("rusty-box-gui-unbundled-exe");
        let library = crate::library::VmLibrary::open(library_dir.path.clone()).expect("open");

        let message = import_bundled_vm_from(&library, &exe_dir.path);

        assert_eq!(message, None);
        assert!(library.load().expect("load").is_empty());
        assert_eq!(library.last_selected(), None);
    }

    #[test]
    fn startup_disk_creation_can_be_skipped_for_egui_restart() {
        let disk = unique_temp_path("rusty-box-gui-restart-disk");
        fs::write(&disk, [0x00]).unwrap();
        let config = disk_creation_config(disk.clone(), false);

        prepare_configured_media_files(&config, false).unwrap();

        assert_eq!(fs::metadata(&disk).unwrap().len(), 1);
        remove_test_file(&disk);
    }

    #[test]
    fn startup_disk_creation_waits_until_media_slots_are_valid() {
        let disk = unique_temp_path("rusty-box-gui-invalid-slot-disk");
        let cdrom = unique_temp_path("rusty-box-gui-invalid-slot-cdrom");
        let mut config = disk_creation_config(disk.clone(), false);
        fs::write(&config.bios, [0xEA]).unwrap();
        fs::write(&cdrom, [0x00]).unwrap();
        config.cdrom = Some(ResolvedCdrom {
            path: cdrom.clone(),
            channel: 0,
            drive: 0,
        });

        let error = run_resolved(config).unwrap_err();
        let disk_exists = fs::metadata(&disk).is_ok();
        if disk_exists {
            remove_test_file(&disk);
        }
        remove_test_file(&cdrom);

        assert!(matches!(
            error,
            RunError::MediaAttach { kind: "CD-ROM", path, source }
                if path == cdrom && source.kind() == std::io::ErrorKind::InvalidInput
        ));
        assert!(!disk_exists);
    }

    #[cfg(feature = "gui-egui")]
    #[test]
    fn prepare_egui_run_clears_pending_serial_input() {
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));
        shared.lock().unwrap().queue_serial_input_line("stale");

        drop(prepare_egui_run(&shared));

        assert_eq!(
            shared.lock().unwrap().drain_serial_input(),
            Vec::<u8>::new()
        );
    }

    /// Keys tapped on the phone's key pad while the machine is off are not
    /// replayed into the next boot.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn prepare_egui_run_drops_keys_queued_while_stopped() {
        use rusty_box::gui::{HostInputEvent, HostInputSink};
        use rusty_box::iodev::scancodes::BxKey;
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));
        assert!(shared
            .lock()
            .unwrap()
            .push(HostInputEvent::Key(BxKey::Enter, true)));
        assert_eq!(shared.lock().unwrap().pending_keys.len(), 1);

        drop(prepare_egui_run(&shared));

        assert!(shared.lock().unwrap().pending_keys.is_empty());
    }

    /// A power-on refused for a missing CD-ROM leaves an `overwrite` disk as
    /// it was: the CD-ROM is checked before the disk is recreated.
    #[test]
    fn a_missing_cdrom_is_refused_before_an_overwrite_disk_is_recreated() {
        let disk = unique_temp_path("rusty-box-gui-kept-overwrite-disk");
        let missing_cdrom = unique_temp_path("rusty-box-gui-missing-cdrom");
        fs::write(&disk, [0xABu8; 1024]).unwrap();
        let mut config = disk_creation_config(disk.clone(), true);
        config.cdrom = Some(ResolvedCdrom {
            path: missing_cdrom.clone(),
            channel: 1,
            drive: 0,
        });

        let error = prepare_configured_media_files(&config, true).unwrap_err();

        let after = fs::read(&disk);
        remove_test_file(&disk);
        assert!(
            matches!(&error, RunError::FileRead { kind: "CD-ROM", path, .. } if path == &missing_cdrom),
            "{error:?}"
        );
        assert_eq!(after.unwrap(), vec![0xABu8; 1024]);
    }

    #[cfg(feature = "gui-egui")]
    #[test]
    fn a_failed_start_reports_no_startup_step_in_progress() {
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));
        {
            let mut display = shared.lock().unwrap();
            display.startup_status = Some(String::from("Creating disk image…"));
            display.start_pending = true;
            display.emu_running = true;
        }

        record_egui_error(&shared, &RunError::MissingDiskCreatePath);

        let display = shared.lock().unwrap();
        assert!(display.startup_status.is_none());
        assert!(!display.emu_running);
        assert!(!display.start_pending);
        assert!(display.runtime_error.is_some());
    }

    #[cfg(feature = "gui-egui")]
    #[test]
    fn egui_emulator_loop_clears_lifecycle_on_startup_error() {
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));
        let (command_tx, command_rx) = mpsc::channel();
        let missing_bios = unique_temp_path("rusty-box-gui-missing-bios");

        command_tx
            .send(crate::app::NativeEmulatorCommand::Start(ResolvedConfig {
                engine: Engine::Interpreter,
                cpu_capabilities: CpuCapabilities::Preset,
                memory_mib: 32,
                host_memory_mib: 32,
                memory_block_kib: 128,
                ips: 4_000_000,
                pci: true,
                sync_slowdown: false,
                sync_realtime: false,
                smp_quantum: 16,
                cpuid_freq: rusty_box::CpuidFreq::None,
                max_instructions: 0,
                cpu_params: BxParams::default(),
                display: DisplayBackend::Egui,
                bios: missing_bios,
                vga_bios: None,
                boot_order: Vec::new(),
                disk: None::<ResolvedDisk>,
                cdrom: None::<ResolvedCdrom>,
                log_level: LogLevel::Warn,
                vga_mode: None,
                pci_vga: false,
            }))
            .unwrap();
        drop(command_tx);

        let result = run_egui_emulator_loop(command_rx, Arc::clone(&shared));

        assert!(result.is_ok());
        let display = shared.lock().unwrap();
        assert!(!display.emu_running);
        assert!(!display.start_pending);
        assert!(display
            .runtime_error
            .as_deref()
            .is_some_and(|message| message.contains("Emulator startup failed")));
    }

    /// A BIOS the loop's machine builds with: one byte at a fresh path.
    #[cfg(feature = "gui-egui")]
    fn loop_bios() -> PathBuf {
        let bios = unique_temp_path("rusty-box-gui-loop-bios");
        fs::write(&bios, [0xEA]).unwrap();
        bios
    }

    /// A VM the loop runs to the end: its BIOS is the one byte at `bios`
    /// and, `max_instructions` being 0, its run retires nothing.
    #[cfg(feature = "gui-egui")]
    fn runnable_config(disk: PathBuf, overwrite: bool, bios: &Path) -> ResolvedConfig {
        let mut config = disk_creation_config(disk, overwrite);
        config.display = DisplayBackend::Egui;
        config.bios = bios.to_path_buf();
        config
    }

    /// Runs the loop over `configs`, one Start each, until the channel
    /// closes, and returns the loop's result with what the display holds.
    #[cfg(feature = "gui-egui")]
    fn run_loop_over(configs: Vec<ResolvedConfig>) -> (Result<RunSummary, RunError>, Option<String>) {
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));
        let (command_tx, command_rx) = mpsc::channel();
        for config in configs {
            command_tx
                .send(crate::app::NativeEmulatorCommand::Start(config))
                .unwrap();
        }
        drop(command_tx);

        let result = run_egui_emulator_loop(command_rx, Arc::clone(&shared));

        let runtime_error = shared.lock().unwrap().runtime_error.clone();
        (result, runtime_error)
    }

    /// Polls `holds` every ten milliseconds until it does, or fails the test
    /// after thirty seconds.
    #[cfg(feature = "gui-egui")]
    fn wait_until(mut holds: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !holds() {
            assert!(
                std::time::Instant::now() < deadline,
                "the condition did not hold within 30 s"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Each VM's startup disk is created at its first power-on of the
    /// session, not only the disk of the VM that was powered on first.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn the_loop_creates_each_vms_startup_disk_at_its_first_start() {
        let bios = loop_bios();
        let first = unique_temp_path("rusty-box-gui-loop-first-disk");
        let second = unique_temp_path("rusty-box-gui-loop-second-disk");

        let (result, runtime_error) = run_loop_over(vec![
            runnable_config(first.clone(), false, &bios),
            runnable_config(second.clone(), false, &bios),
        ]);

        let first_len = fs::metadata(&first).map(|metadata| metadata.len());
        let second_len = fs::metadata(&second).map(|metadata| metadata.len());
        remove_test_file(&first);
        remove_test_file(&second);
        remove_test_file(&bios);
        assert!(result.is_ok());
        assert_eq!(runtime_error, None);
        assert_eq!(first_len.unwrap(), 10_321_920);
        assert_eq!(second_len.unwrap(), 10_321_920);
    }

    /// A second VM's `overwrite` disk is recreated at its first power-on:
    /// the erase the shell asked the user about happens.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn the_loop_recreates_a_second_vms_overwrite_disk_at_its_first_start() {
        let bios = loop_bios();
        let first = unique_temp_path("rusty-box-gui-loop-first-disk");
        let second = unique_temp_path("rusty-box-gui-loop-second-overwrite-disk");
        fs::write(&second, [0u8; 1024]).unwrap();

        let (result, runtime_error) = run_loop_over(vec![
            runnable_config(first.clone(), false, &bios),
            runnable_config(second.clone(), true, &bios),
        ]);

        let second_len = fs::metadata(&second).map(|metadata| metadata.len());
        remove_test_file(&first);
        remove_test_file(&second);
        remove_test_file(&bios);
        assert!(result.is_ok());
        assert_eq!(runtime_error, None);
        assert_eq!(second_len.unwrap(), 10_321_920);
    }

    /// The loop on a thread of its own, as the shell runs it, fed one Start
    /// at a time.
    #[cfg(feature = "gui-egui")]
    struct LoopThread {
        commands: mpsc::Sender<crate::app::NativeEmulatorCommand>,
        shared: Arc<Mutex<SharedDisplay>>,
        thread: std::thread::JoinHandle<Result<RunSummary, RunError>>,
    }

    #[cfg(feature = "gui-egui")]
    impl LoopThread {
        fn spawn() -> Self {
            let shared = Arc::new(Mutex::new(SharedDisplay::new()));
            let (commands, command_rx) = mpsc::channel();
            let loop_shared = Arc::clone(&shared);
            let thread = std::thread::Builder::new()
                .name("rusty_box_gui_test_emulator".to_owned())
                .stack_size(16 * 1024 * 1024)
                .spawn(move || run_egui_emulator_loop(command_rx, loop_shared))
                .expect("spawn the loop");
            Self {
                commands,
                shared,
                thread,
            }
        }

        /// Powers `config` on and waits for its run to end: the run is over
        /// once its startup disk `disk` exists and the machine no longer
        /// runs, and the loop then waits for the next command.
        fn start_and_wait(&self, config: ResolvedConfig, disk: &Path) {
            self.commands
                .send(crate::app::NativeEmulatorCommand::Start(config))
                .unwrap();
            wait_until(|| disk.exists() && !self.shared.lock().unwrap().emu_running);
        }

        /// Powers `config` on, lets the loop finish every command it was
        /// sent, and returns its result with the error the display holds.
        fn start_and_finish(self, config: ResolvedConfig) -> (Result<RunSummary, RunError>, Option<String>) {
            let Self {
                commands,
                shared,
                thread,
            } = self;
            commands
                .send(crate::app::NativeEmulatorCommand::Start(config))
                .unwrap();
            drop(commands);
            let result = thread.join().expect("the loop thread");
            let runtime_error = shared.lock().unwrap().runtime_error.clone();
            (result, runtime_error)
        }
    }

    /// What a guest would have left on the disk at `path`: its first byte
    /// set to `0xAB`.
    #[cfg(feature = "gui-egui")]
    fn mark_first_byte(path: &Path) {
        let mut contents = fs::read(path).unwrap();
        contents[0] = 0xAB;
        fs::write(path, &contents).unwrap();
    }

    /// An overwrite disk is erased once per session however often its VM is
    /// powered on: a second Start leaves what the first run left on it.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn the_loop_erases_an_overwrite_disk_once_however_often_its_vm_starts() {
        let bios = loop_bios();
        let disk = unique_temp_path("rusty-box-gui-loop-same-disk");
        let config = runnable_config(disk.clone(), true, &bios);
        let emulator = LoopThread::spawn();

        emulator.start_and_wait(config.clone(), &disk);
        mark_first_byte(&disk);
        let (result, runtime_error) = emulator.start_and_finish(config);

        let after = fs::read(&disk);
        remove_test_file(&disk);
        remove_test_file(&bios);
        assert!(result.is_ok());
        assert_eq!(runtime_error, None);
        let after = after.unwrap();
        assert_eq!(after.len(), 10_321_920);
        assert_eq!(after[0], 0xAB);
    }

    /// A plain creation is provisioned at every Start: a disk removed after
    /// one power-on is there again after the next, and that power-on runs.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn the_loop_provisions_a_plain_disk_at_every_start() {
        let bios = loop_bios();
        let disk = unique_temp_path("rusty-box-gui-loop-plain-disk");
        let config = runnable_config(disk.clone(), false, &bios);
        let emulator = LoopThread::spawn();

        emulator.start_and_wait(config.clone(), &disk);
        fs::remove_file(&disk).unwrap();
        let (result, runtime_error) = emulator.start_and_finish(config);

        let after_len = fs::metadata(&disk).map(|metadata| metadata.len());
        remove_test_file(&disk);
        remove_test_file(&bios);
        assert!(result.is_ok());
        assert_eq!(runtime_error, None);
        assert_eq!(after_len.unwrap(), 10_321_920);
    }

    /// A plain power-on settles nothing for a later overwrite of the same
    /// file: the overwrite Start that follows erases what the first run
    /// left, as the shell's confirmation said it would.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn an_overwrite_start_after_a_plain_start_of_the_same_disk_erases_it() {
        let bios = loop_bios();
        let disk = unique_temp_path("rusty-box-gui-loop-plain-then-overwrite");
        let emulator = LoopThread::spawn();

        emulator.start_and_wait(runnable_config(disk.clone(), false, &bios), &disk);
        mark_first_byte(&disk);
        let (result, runtime_error) =
            emulator.start_and_finish(runnable_config(disk.clone(), true, &bios));

        let after = fs::read(&disk);
        remove_test_file(&disk);
        remove_test_file(&bios);
        assert!(result.is_ok());
        assert_eq!(runtime_error, None);
        let after = after.unwrap();
        assert_eq!(after.len(), 10_321_920);
        assert_eq!(after[0], 0x00, "the overwrite Start left the first run's data");
    }

    #[test]
    fn boot_sequence_pads_missing_devices_with_nothing() {
        assert_eq!(
            boot_sequence(&[BootDevice::Disk]),
            BootOrder::just(GuestBootDevice::Disk)
        );
        assert_eq!(
            boot_sequence(&[BootDevice::Cdrom, BootDevice::Disk]),
            BootOrder::new(
                GuestBootDevice::Cdrom,
                GuestBootDevice::Disk,
                GuestBootDevice::None
            )
        );
    }

    #[test]
    fn prequeues_enter_for_cdrom_first_boot() {
        assert!(should_prequeue_boot_enter(&[BootDevice::Cdrom]));
        assert!(should_prequeue_boot_enter(&[
            BootDevice::Cdrom,
            BootDevice::Disk
        ]));
        assert!(!should_prequeue_boot_enter(&[
            BootDevice::Disk,
            BootDevice::Cdrom
        ]));
        assert!(!should_prequeue_boot_enter(&[]));
    }

    fn resolved_disk(channel: usize, drive: usize) -> ResolvedDisk {
        ResolvedDisk {
            path: PathBuf::from("disk.img"),
            geometry: DiskGeometry {
                cylinders: 306,
                heads: 4,
                sectors_per_track: 17,
            },
            channel,
            drive,
            creation: None,
        }
    }

    fn media_config(disk: Option<ResolvedDisk>, cdrom: Option<ResolvedCdrom>) -> ResolvedConfig {
        let mut config = disk_creation_config(PathBuf::from("disk.img"), false);
        config.disk = disk;
        config.cdrom = cdrom;
        config
    }

    #[test]
    fn rejects_out_of_range_ata_slots() {
        let config = media_config(Some(resolved_disk(2, 0)), None);
        assert!(matches!(
            resolve_media_slots(&config),
            Err(RunError::ValueOverflow {
                field: "disk.channel"
            })
        ));

        let config = media_config(
            None,
            Some(ResolvedCdrom {
                path: PathBuf::from("cdrom.iso"),
                channel: 0,
                drive: 2,
            }),
        );
        assert!(matches!(
            resolve_media_slots(&config),
            Err(RunError::ValueOverflow {
                field: "cdrom.drive"
            })
        ));
    }

    #[test]
    fn configured_channel_and_drive_name_an_ata_slot() {
        let config = media_config(
            Some(resolved_disk(0, 1)),
            Some(ResolvedCdrom {
                path: PathBuf::from("cdrom.iso"),
                channel: 1,
                drive: 0,
            }),
        );

        let slots = resolve_media_slots(&config).unwrap();

        assert_eq!(slots.disk, Some(AtaSlot::PRIMARY_SLAVE));
        assert_eq!(slots.cdrom, Some(AtaSlot::SECONDARY_MASTER));
    }

    #[test]
    fn rejects_disk_cdrom_same_ata_slot() {
        let config = media_config(
            Some(resolved_disk(0, 0)),
            Some(ResolvedCdrom {
                path: PathBuf::from("cdrom.iso"),
                channel: 0,
                drive: 0,
            }),
        );

        let error = resolve_media_slots(&config).unwrap_err();

        assert!(matches!(
            error,
            RunError::MediaAttach { kind: "CD-ROM", path, source }
                if path == PathBuf::from("cdrom.iso")
                    && source.kind() == std::io::ErrorKind::InvalidInput
        ));
    }
}
