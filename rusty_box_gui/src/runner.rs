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
    run_resolved(config)
}

pub fn run_resolved(config: ResolvedConfig) -> Result<RunSummary, RunError> {
    match config.display {
        DisplayBackend::Headless => run_with_gui(config, NoGui::new(), None, true),
        DisplayBackend::Terminal => run_with_gui(config, TermGui::new(), None, true),
        #[cfg(feature = "gui-egui")]
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

fn prepare_configured_media_files(
    config: &ResolvedConfig,
    create_startup_disks: bool,
) -> Result<(), RunError> {
    if create_startup_disks {
        create_configured_disk_images(config)?;
    }
    if let Some(disk) = &config.disk {
        verify_media_file("disk", &disk.path)?;
    }
    if let Some(cdrom) = &config.cdrom {
        verify_media_file("CD-ROM", &cdrom.path)?;
    }
    Ok(())
}

fn run_with_gui<G>(
    config: ResolvedConfig,
    gui: G,
    stop_flag: Option<Arc<AtomicBool>>,
    create_startup_disks: bool,
) -> Result<RunSummary, RunError>
where
    G: BxGui + 'static,
{
    init_tracing(config.log_level);

    // A build without the hypervisor path cannot honour `--engine whp`. It
    // refuses before reading or creating anything, for the reason a host
    // without the platform is refused below: a run that silently went to the
    // interpreter under the hypervisor's name is a measurement nobody can
    // trust.
    #[cfg(not(all(not(feature = "guest-trace"), feature = "hv-whp", windows)))]
    if config.engine == Engine::Whp {
        return Err(RunError::NoHypervisorEngine);
    }

    let bios_data = read_required_file("BIOS", &config.bios)?;
    let vga_data = match &config.vga_bios {
        Some(path) => Some(read_vga_bios_file(path)?),
        None => None,
    };
    let slots = resolve_media_slots(&config)?;
    prepare_configured_media_files(&config, create_startup_disks)?;

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

/// The desktop shell: a window of its own over the emulator thread.
#[cfg(all(feature = "gui-egui", not(target_os = "android")))]
fn run_egui(config: ResolvedConfig) -> Result<RunSummary, RunError> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true)
            .with_title("Rusty Box Workstation"),
        ..Default::default()
    };
    run_egui_shell(config, native_options, crate::app::NativeShellApp::new)
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

/// The shell a phone shows for `app`, the activity NativeActivity handed this
/// process.
#[cfg(all(feature = "gui-egui", target_os = "android"))]
pub(crate) fn run_android_shell(
    config: ResolvedConfig,
    app: crate::android::AndroidApp,
) -> Result<RunSummary, RunError> {
    let native_options = eframe::NativeOptions {
        android_app: Some(app.clone()),
        ..Default::default()
    };
    run_egui_shell(config, native_options, move |cc, shared, command_tx, config| {
        crate::android::AndroidShellApp::new(cc, shared, command_tx, config, app)
    })
}

/// Runs an egui shell over the emulator thread. `make_app` builds the
/// window's app from the display the two threads share and the channel the
/// shell starts machines through.
#[cfg(feature = "gui-egui")]
fn run_egui_shell<A, F>(
    config: ResolvedConfig,
    native_options: eframe::NativeOptions,
    make_app: F,
) -> Result<RunSummary, RunError>
where
    A: eframe::App + 'static,
    F: FnOnce(
            &eframe::CreationContext<'_>,
            Arc<Mutex<SharedDisplay>>,
            mpsc::Sender<crate::app::NativeEmulatorCommand>,
            ResolvedConfig,
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
        Box::new(move |cc| Ok(Box::new(make_app(cc, shared_for_gui, command_tx, config)))),
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

#[cfg(feature = "gui-egui")]
fn run_egui_emulator_loop(
    command_rx: mpsc::Receiver<crate::app::NativeEmulatorCommand>,
    shared: Arc<Mutex<SharedDisplay>>,
) -> Result<RunSummary, RunError> {
    let mut instructions_executed = Some(0u64);
    let mut create_startup_disks = true;

    while let Ok(command) = command_rx.recv() {
        match command {
            crate::app::NativeEmulatorCommand::Start(config) => loop {
                let stop_flag = prepare_egui_run(&shared);
                // Warn in the window that a (possibly slow) disk allocation is
                // about to happen, before run_with_gui blocks on it. Reusing an
                // existing image returns None here (instant, no warning).
                if create_startup_disks {
                    if let Some(status) = startup_disk_action(&config) {
                        if let Ok(mut display) = shared.lock() {
                            display.startup_status = Some(status);
                        }
                    }
                }
                let bridge = BridgeGui::new(Arc::clone(&shared));
                let summary = match run_with_gui(
                    config.clone(),
                    bridge,
                    Some(stop_flag),
                    create_startup_disks,
                ) {
                    Ok(summary) => summary,
                    Err(error) => {
                        record_egui_error(&shared, &error);
                        break;
                    }
                };
                create_startup_disks = false;
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
        drop(display.drain_serial_input());
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
            config_path: None,
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
            config_path: None,
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

    /// `--engine whp` in a build that carries no hypervisor path is refused,
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
                config_path: None,
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
