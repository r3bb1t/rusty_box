use crate::{
    args::{Args, BootDevice, DisplayBackend, LogLevel},
    config::{ResolvedCdrom, ResolvedConfig, ResolvedDisk},
    error::RunError,
};
#[cfg(feature = "gui-egui")]
use rusty_box::gui::{shared_display::SharedDisplay, BridgeGui};
use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, ResetReason},
    emulator::{Emulator, EmulatorConfig},
    gui::{BxGui, NoGui, TermGui},
};
#[cfg(feature = "gui-egui")]
use std::sync::atomic::Ordering;
#[cfg(feature = "gui-egui")]
use std::sync::{mpsc, Mutex};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunSummary {
    pub instructions_executed: u64,
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
            crate::disk_images::create_startup_disk(creation)?;
        }
    }
    Ok(())
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

    let bios_data = read_required_file("BIOS", &config.bios)?;
    let vga_data = match &config.vga_bios {
        Some(path) => Some(read_vga_bios_file(path)?),
        None => None,
    };
    validate_configured_media_slots(&config)?;
    prepare_configured_media_files(&config, create_startup_disks)?;

    let emulator_config = EmulatorConfig {
        guest_memory_size: mib_to_bytes("memory_mib", config.memory_mib)?,
        host_memory_size: mib_to_bytes("host_memory_mib", config.host_memory_mib)?,
        memory_block_size: kib_to_bytes("memory_block_kib", config.memory_block_kib)?,
        ips: config.ips,
        pci_enabled: config.pci,
        sync_slowdown: config.sync_slowdown,
        cpu_params: config.cpu_params.clone(),
        ..EmulatorConfig::default()
    };

    #[cfg(not(feature = "guest-trace"))]
    let mut emu = Emulator::<Corei7SkylakeX>::new(emulator_config)?;
    // Diagnostic build: run the CPU with the guest-death tracer installed.
    // Single-CPU only — new_with_instrumentation rejects SMP configs.
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
        eprintln!("guest-trace: recording guest evidence to {trace_log}");
        Emulator::<Corei7SkylakeX, crate::guest_trace::GuestTracer>::new_with_instrumentation(
            emulator_config,
            tracer,
        )?
    };
    if let Some(stop_flag) = stop_flag {
        emu.stop_flag = stop_flag;
    }
    emu.set_gui(gui);

    emu.init_memory_and_pc_system()?;
    let bios_load_addr = !(bios_data.len() as u64 - 1);
    emu.load_bios(&bios_data, bios_load_addr)?;
    if let Some(data) = &vga_data {
        emu.load_optional_rom(data, 0xC0000)?;
    }
    emu.init_cpu_and_devices()?;
    emu.configure_memory_in_cmos_from_config();
    let (first_boot, second_boot, third_boot) = boot_sequence(&config.boot_order);
    emu.configure_boot_sequence(first_boot, second_boot, third_boot);

    if let Some(disk) = &config.disk {
        let geometry = disk.geometry;
        emu.configure_disk_geometry_in_cmos(
            disk_cmos_drive(disk),
            geometry.cylinders,
            geometry.heads,
            geometry.sectors_per_track,
        );
        let disk_path = path_to_str(&disk.path)?;
        emu.attach_disk(
            disk.channel,
            disk.drive,
            disk_path,
            geometry.cylinders,
            geometry.heads,
            geometry.sectors_per_track,
        )
        .map_err(|source| RunError::MediaAttach {
            kind: "disk",
            path: disk.path.clone(),
            source,
        })?;
    }

    if let Some(cdrom) = &config.cdrom {
        let cdrom_path = path_to_str(&cdrom.path)?;
        emu.attach_cdrom(cdrom.channel, cdrom.drive, cdrom_path)
            .map_err(|source| RunError::MediaAttach {
                kind: "CD-ROM",
                path: cdrom.path.clone(),
                source,
            })?;
    }

    emu.init_gui(0, &[])?;
    emu.reset(ResetReason::Hardware)?;
    emu.init_gui_signal_handlers();
    emu.start();
    if should_prequeue_boot_enter(&config.boot_order) {
        emu.prepare_run();
        emu.send_string("\n");
    }
    let instructions_executed = emu.run_interactive(config.max_instructions)?;

    Ok(RunSummary {
        instructions_executed,
    })
}

#[cfg(feature = "gui-egui")]
fn run_egui(config: ResolvedConfig) -> Result<RunSummary, RunError> {
    let shared = Arc::new(Mutex::new(SharedDisplay::new()));
    let (command_tx, command_rx) = mpsc::channel();
    let shared_for_emu = Arc::clone(&shared);
    let emulator_thread = std::thread::Builder::new()
        .name("rusty_box_gui_emulator".to_owned())
        .stack_size(1500 * 1024 * 1024)
        .spawn(move || run_egui_emulator_loop(command_rx, shared_for_emu))
        .map_err(|source| RunError::ThreadStart { source })?;

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true)
            .with_title("Rusty Box Workstation"),
        ..Default::default()
    };
    let shared_for_gui = Arc::clone(&shared);
    let gui_result = eframe::run_native(
        "Rusty Box Workstation",
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(crate::app::NativeShellApp::new(
                cc,
                shared_for_gui,
                command_tx,
                config,
            )))
        }),
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
    let mut instructions_executed = 0u64;
    let mut create_startup_disks = true;

    while let Ok(command) = command_rx.recv() {
        match command {
            crate::app::NativeEmulatorCommand::Start(config) => loop {
                let stop_flag = prepare_egui_run(&shared);
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
                instructions_executed =
                    instructions_executed.saturating_add(summary.instructions_executed);

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

fn init_tracing(log_level: LogLevel) {
    match tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_max_level(match log_level {
            LogLevel::Trace => tracing::Level::TRACE,
            LogLevel::Debug => tracing::Level::DEBUG,
            LogLevel::Info => tracing::Level::INFO,
            LogLevel::Warn => tracing::Level::WARN,
            LogLevel::Error => tracing::Level::ERROR,
        })
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

fn boot_sequence(boot_order: &[BootDevice]) -> (u8, u8, u8) {
    let mut values = [0; 3];
    for (index, device) in boot_order.iter().take(3).enumerate() {
        values[index] = match device {
            BootDevice::Disk => 2,
            BootDevice::Cdrom => 3,
        };
    }
    (values[0], values[1], values[2])
}

fn validate_configured_media_slots(config: &ResolvedConfig) -> Result<(), RunError> {
    if let Some(disk) = &config.disk {
        validate_ata_slot("disk.channel", disk.channel)?;
        validate_ata_slot("disk.drive", disk.drive)?;
        if let Some(cdrom) = &config.cdrom {
            validate_distinct_media_slots(disk, cdrom)?;
        }
    }
    if let Some(cdrom) = &config.cdrom {
        validate_ata_slot("cdrom.channel", cdrom.channel)?;
        validate_ata_slot("cdrom.drive", cdrom.drive)?;
    }
    Ok(())
}

fn validate_distinct_media_slots(
    disk: &ResolvedDisk,
    cdrom: &ResolvedCdrom,
) -> Result<(), RunError> {
    if disk.channel == cdrom.channel && disk.drive == cdrom.drive {
        Err(RunError::MediaAttach {
            kind: "CD-ROM",
            path: cdrom.path.clone(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "ATA slot already used by disk"),
        })
    } else {
        Ok(())
    }
}

fn validate_ata_slot(field: &'static str, value: usize) -> Result<(), RunError> {
    if value > 1 {
        Err(RunError::ValueOverflow { field })
    } else {
        Ok(())
    }
}

fn disk_cmos_drive(disk: &ResolvedDisk) -> u8 {
    disk.drive as u8
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
    use crate::config::ResolvedDiskCreation;
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
            memory_mib: 32,
            host_memory_mib: 32,
            memory_block_kib: 128,
            ips: 4_000_000,
            pci: true,
            sync_slowdown: false,
            max_instructions: 0,
            cpu_params: BxParams::default(),
            display: DisplayBackend::Headless,
            bios: bios.clone(),
            vga_bios: Some(vga.clone()),
            boot_order: Vec::new(),
            disk: None::<ResolvedDisk>,
            cdrom: None::<ResolvedCdrom>,
            log_level: LogLevel::Warn,
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
            memory_mib: 32,
            host_memory_mib: 32,
            memory_block_kib: 128,
            ips: 4_000_000,
            pci: true,
            sync_slowdown: false,
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
    fn startup_disk_creation_respects_no_overwrite() {
        let disk = unique_temp_path("rusty-box-gui-existing-disk");
        fs::write(&disk, [0x00]).unwrap();
        let config = disk_creation_config(disk.clone(), false);
        fs::write(&config.bios, [0xEA]).unwrap();

        let error = create_configured_disk_images(&config).unwrap_err();

        assert!(matches!(
            error,
            RunError::DiskCreate {
                source: rusty_box_bximage::BxImageError::AlreadyExists { path }
            } if path == disk
        ));
        remove_test_file(&disk);
        remove_test_file(&config.bios);
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
    fn egui_emulator_loop_clears_lifecycle_on_startup_error() {
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));
        let (command_tx, command_rx) = mpsc::channel();
        let missing_bios = unique_temp_path("rusty-box-gui-missing-bios");

        command_tx
            .send(crate::app::NativeEmulatorCommand::Start(ResolvedConfig {
                memory_mib: 32,
                host_memory_mib: 32,
                memory_block_kib: 128,
                ips: 4_000_000,
                pci: true,
                sync_slowdown: false,
                max_instructions: 0,
                cpu_params: BxParams::default(),
                display: DisplayBackend::Egui,
                bios: missing_bios,
                vga_bios: None,
                boot_order: Vec::new(),
                disk: None::<ResolvedDisk>,
                cdrom: None::<ResolvedCdrom>,
                log_level: LogLevel::Warn,
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
    fn boot_sequence_pads_missing_devices_with_zero() {
        assert_eq!(boot_sequence(&[BootDevice::Disk]), (2, 0, 0));
        assert_eq!(
            boot_sequence(&[BootDevice::Cdrom, BootDevice::Disk]),
            (3, 2, 0)
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

    #[test]
    fn rejects_out_of_range_ata_slots() {
        assert!(matches!(
            validate_ata_slot("disk.channel", 2),
            Err(RunError::ValueOverflow {
                field: "disk.channel"
            })
        ));
        assert!(matches!(
            validate_ata_slot("cdrom.drive", 2),
            Err(RunError::ValueOverflow {
                field: "cdrom.drive"
            })
        ));
    }

    #[test]
    fn cmos_drive_uses_configured_disk_drive() {
        let disk = ResolvedDisk {
            path: PathBuf::from("disk.img"),
            geometry: DiskGeometry {
                cylinders: 306,
                heads: 4,
                sectors_per_track: 17,
            },
            channel: 0,
            drive: 1,
            creation: None,
        };

        assert_eq!(disk_cmos_drive(&disk), 1);
    }

    #[test]
    fn rejects_disk_cdrom_same_ata_slot() {
        let disk = ResolvedDisk {
            path: PathBuf::from("disk.img"),
            geometry: DiskGeometry {
                cylinders: 306,
                heads: 4,
                sectors_per_track: 17,
            },
            channel: 0,
            drive: 0,
            creation: None,
        };
        let cdrom = ResolvedCdrom {
            path: PathBuf::from("cdrom.iso"),
            channel: 0,
            drive: 0,
        };

        let error = validate_distinct_media_slots(&disk, &cdrom).unwrap_err();

        assert!(matches!(
            error,
            RunError::MediaAttach { kind: "CD-ROM", path, source }
                if path == PathBuf::from("cdrom.iso")
                    && source.kind() == std::io::ErrorKind::InvalidInput
        ));
    }
}
