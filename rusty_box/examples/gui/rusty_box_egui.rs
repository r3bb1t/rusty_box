//! Unified Rusty Box GUI — boots DLX Linux or Alpine Linux via egui
//!
//! Selects boot profile based on `RUSTY_BOX_BOOT` env var:
//! - `dlx` (default): DLX Linux, 32MB RAM, hard disk
//! - `alpine`: Alpine Linux, 256MB RAM, ISO CD-ROM
//!
//! ## Usage
//! ```bash
//! # DLX Linux (default)
//! cargo run --release --example rusty_box_egui --features "std,gui-egui"
//!
//! # Alpine Linux
//! RUSTY_BOX_BOOT=alpine cargo run --release --example rusty_box_egui --features "std,gui-egui"
//!
//! # Alpine with custom RAM
//! RUSTY_BOX_BOOT=alpine ALPINE_RAM_MB=512 cargo run --release --example rusty_box_egui --features "std,gui-egui"
//! ```

use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, ResetReason},
    emulator::{Emulator, EmulatorConfig},
    gui::{shared_display::SharedDisplay, BridgeGui, RustyBoxApp},
    Result,
};
use std::sync::{
    atomic::Ordering,
    {Arc, Mutex},
};
use std::time::Instant;

// =========================================================================
// Boot profile
// =========================================================================

enum BootProfile {
    Dlx {
        disk_path: std::path::PathBuf,
    },
    Alpine {
        iso_path: std::path::PathBuf,
        ram_mb: usize,
    },
}

/// DLX Linux disk geometry (from bochsrc.bxrc)
const DLX_CYLINDERS: u16 = 306;
const DLX_HEADS: u8 = 4;
const DLX_SPT: u8 = 17;

fn main() {
    let log_level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| s.parse::<tracing::Level>().ok())
        .unwrap_or(tracing::Level::WARN);

    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_max_level(log_level)
        .init();

    // =========================================================================
    // Detect boot profile
    // =========================================================================
    let workspace_root = find_workspace_root();
    let profile = detect_boot_profile(&workspace_root);

    let window_title = match &profile {
        BootProfile::Dlx { .. } => "Rusty Box - DLX Linux",
        BootProfile::Alpine { .. } => "Rusty Box - Alpine Linux",
    };

    // =========================================================================
    // Find BIOS files (shared by both profiles)
    // =========================================================================
    let bios_data = find_file(
        &workspace_root,
        &[
            "cpp_orig/bochs/bios/BIOS-bochs-latest",
            "cpp_orig/bochs/bios/BIOS-bochs-legacy",
            "BIOS-bochs-latest",
            "../cpp_orig/bochs/bios/BIOS-bochs-latest",
        ],
    )
    .expect("Could not find BIOS file");
    println!("BIOS loaded: {} bytes", bios_data.len());

    let vga_bios = find_file_with_validation(
        &workspace_root,
        &[
            "binaries/bios/VGABIOS-lgpl-latest.bin",
            "cpp_orig/bochs/bios/VGABIOS-lgpl-latest.bin",
            "VGABIOS-lgpl-latest.bin",
            "../cpp_orig/bochs/bios/VGABIOS-lgpl-latest.bin",
        ],
        |data| data.len() % 512 == 0,
    );
    if let Some(ref vga) = vga_bios {
        println!("VGA BIOS loaded: {} bytes", vga.len());
    }

    // =========================================================================
    // Create shared display
    // =========================================================================
    let shared = Arc::new(Mutex::new(SharedDisplay::new()));
    let shared_for_emu = Arc::clone(&shared);

    let max_instructions: u64 = std::env::var("MAX_INSTRUCTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(u64::MAX);

    // =========================================================================
    // Spawn emulator on background thread
    // =========================================================================
    let emu_thread = std::thread::Builder::new()
        .stack_size(1500 * 1024 * 1024)
        .name("Emulator".to_string())
        .spawn(move || {
            loop {
                {
                    let mut d = shared_for_emu.lock().unwrap();
                    d.stop_flag.store(false, Ordering::Relaxed);
                    d.emu_running = true;
                    d.reset_requested = false;
                }

                if let Err(e) = run_emulator(
                    &profile,
                    &bios_data,
                    vga_bios.as_deref(),
                    Arc::clone(&shared_for_emu),
                    max_instructions,
                ) {
                    eprintln!("Emulator error: {:?}", e);
                }

                let restart = {
                    let d = shared_for_emu.lock().unwrap();
                    d.reset_requested
                };
                if !restart {
                    break;
                }
                println!("Restarting emulator (Reset requested)...");
            }
        })
        .expect("Failed to spawn emulator thread");

    // =========================================================================
    // Run eframe on main thread
    // =========================================================================
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 450.0])
            .with_min_inner_size([720.0, 426.0])
            .with_title(window_title),
        ..Default::default()
    };

    let shared_for_gui = Arc::clone(&shared);
    let _ = eframe::run_native(
        "Rusty Box",
        native_options,
        Box::new(move |cc| Ok(Box::new(RustyBoxApp::new(cc, shared_for_gui)))),
    );

    if let Ok(mut display) = shared.lock() {
        display.emu_running = false;
    }

    let _ = emu_thread.join();
}

// =========================================================================
// Emulator runner
// =========================================================================

fn run_emulator(
    profile: &BootProfile,
    bios_data: &[u8],
    vga_bios: Option<&[u8]>,
    shared: Arc<Mutex<SharedDisplay>>,
    max_instructions: u64,
) -> Result<()> {
    let ram_bytes = match profile {
        BootProfile::Dlx { .. } => 32 * 1024 * 1024,
        BootProfile::Alpine { ram_mb, .. } => ram_mb * 1024 * 1024,
    };

    let config = EmulatorConfig {
        guest_memory_size: ram_bytes,
        host_memory_size: ram_bytes,
        memory_block_size: 128 * 1024,
        ips: 15_000_000,
        pci_enabled: true,
        ..Default::default()
    };

    let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;

    emu.stop_flag = {
        let d = shared.lock().unwrap();
        Arc::clone(&d.stop_flag)
    };

    let bridge = BridgeGui::new(Arc::clone(&shared));
    emu.set_gui(bridge);

    emu.init_memory_and_pc_system()?;

    let bios_size = bios_data.len() as u64;
    let bios_load_addr = !(bios_size - 1);
    emu.load_bios(bios_data, bios_load_addr)?;

    if let Some(vga_data) = vga_bios {
        emu.load_optional_rom(vga_data, 0xC0000)?;
    }

    emu.init_cpu_and_devices()?;

    // Profile-specific configuration
    match profile {
        BootProfile::Dlx { disk_path } => {
            emu.configure_memory_in_cmos(640, 31 * 1024);
            emu.configure_disk_geometry_in_cmos(0, DLX_CYLINDERS, DLX_HEADS, DLX_SPT);
            emu.configure_boot_sequence(2, 0, 0);
            let disk_path_str = disk_path.to_string_lossy().to_string();
            emu.attach_disk(0, 0, &disk_path_str, DLX_CYLINDERS, DLX_HEADS, DLX_SPT)
                .expect("Failed to attach DLX disk image");
            println!("DLX disk attached: {}", disk_path.display());
        }
        BootProfile::Alpine { iso_path, .. } => {
            emu.configure_memory_in_cmos_from_config();
            emu.configure_boot_sequence(3, 0, 0); // CD-ROM first
            let iso_path_str = iso_path.to_string_lossy().to_string();
            emu.attach_cdrom(1, 0, &iso_path_str)
                .expect("Failed to attach Alpine CD-ROM image");
            println!("Alpine CD-ROM attached: {}", iso_path.display());
        }
    }

    emu.init_gui(0, &[])?;
    emu.reset(ResetReason::Hardware)?;
    emu.init_gui_signal_handlers();
    emu.start();

    println!("Emulator started (max {} instructions)", max_instructions);
    let start_time = Instant::now();

    let result = emu.run_interactive(max_instructions);
    let elapsed = start_time.elapsed();

    match result {
        Ok(executed) => {
            let mips = if elapsed.as_secs_f64() > 0.001 {
                executed as f64 / elapsed.as_secs_f64() / 1_000_000.0
            } else {
                0.0
            };
            println!(
                "Executed {} instructions in {:.3}s ({:.2} MIPS)",
                executed,
                elapsed.as_secs_f64(),
                mips
            );
        }
        Err(ref e) => {
            eprintln!("Execution error: {:?}", e);
        }
    }

    println!("\n===== VGA TEXT DUMP =====");
    println!("{}", emu.vga_text_dump());

    if let Ok(mut display) = shared.lock() {
        display.emu_running = false;
    }

    if let Some(ref mut gui) = emu.gui_mut() {
        gui.exit();
    }

    Ok(())
}

// =========================================================================
// Boot profile detection
// =========================================================================

fn detect_boot_profile(workspace_root: &std::path::Path) -> BootProfile {
    let boot_env = std::env::var("RUSTY_BOX_BOOT")
        .unwrap_or_else(|_| "dlx".to_string())
        .to_lowercase();

    match boot_env.as_str() {
        "alpine" => {
            let ram_mb: usize = std::env::var("ALPINE_RAM_MB")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(256);

            let iso_path = if let Ok(path) = std::env::var("ALPINE_DISK") {
                let p = std::path::PathBuf::from(&path);
                if !p.exists() {
                    eprintln!("ERROR: ALPINE_DISK={} does not exist", path);
                    std::process::exit(1);
                }
                p
            } else {
                // Auto-detect alpine*.iso in workspace root
                let iso = std::fs::read_dir(workspace_root)
                    .ok()
                    .and_then(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .map(|e| e.path())
                            .find(|p| {
                                p.extension().map(|ext| ext == "iso").unwrap_or(false)
                                    && p.file_name()
                                        .and_then(|n| n.to_str())
                                        .map(|s| s.to_lowercase().contains("alpine"))
                                        .unwrap_or(false)
                            })
                    });

                match iso {
                    Some(p) => p,
                    None => {
                        eprintln!("ERROR: No Alpine ISO found in workspace root.");
                        eprintln!("Set ALPINE_DISK=/path/to/alpine.iso or place an alpine*.iso in the project root.");
                        std::process::exit(1);
                    }
                }
            };

            println!("Boot profile: Alpine Linux ({} MB RAM)", ram_mb);
            println!("ISO: {}", iso_path.display());
            BootProfile::Alpine { iso_path, ram_mb }
        }
        _ => {
            let disk_path = find_path(
                workspace_root,
                &["dlxlinux/hd10meg.img", "../dlxlinux/hd10meg.img"],
            )
            .expect("Could not find DLX Linux disk image (hd10meg.img)");

            println!("Boot profile: DLX Linux (32 MB RAM)");
            println!("Disk: {}", disk_path.display());
            BootProfile::Dlx { disk_path }
        }
    }
}

// =========================================================================
// File finding helpers
// =========================================================================

fn find_workspace_root() -> std::path::PathBuf {
    std::env::current_dir()
        .ok()
        .and_then(|mut dir| {
            loop {
                if dir.join("Cargo.toml").exists() || dir.join("dlxlinux").exists() {
                    return Some(dir);
                }
                if let Some(parent) = dir.parent() {
                    dir = parent.to_path_buf();
                } else {
                    break;
                }
            }
            None
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

fn find_file(root: &std::path::Path, candidates: &[&str]) -> Option<Vec<u8>> {
    for candidate in candidates {
        let path = root.join(candidate);
        if let Ok(data) = std::fs::read(&path) {
            return Some(data);
        }
    }
    None
}

fn find_file_with_validation(
    root: &std::path::Path,
    candidates: &[&str],
    validate: impl Fn(&[u8]) -> bool,
) -> Option<Vec<u8>> {
    for candidate in candidates {
        let path = root.join(candidate);
        if let Ok(data) = std::fs::read(&path) {
            if validate(&data) {
                return Some(data);
            }
        }
    }
    None
}

fn find_path(root: &std::path::Path, candidates: &[&str]) -> Option<std::path::PathBuf> {
    for candidate in candidates {
        let path = root.join(candidate);
        if path.exists() {
            return Some(path);
        }
    }
    None
}
