//! DLX Linux Boot Example — egui GUI
//!
//! Same as `dlxlinux.rs` but uses the egui/eframe graphical window instead of
//! the terminal GUI. The emulator runs on a background thread while eframe
//! owns the main thread event loop.
//!
//! ## Usage
//! ```bash
//! cargo build --release --example dlxlinux_egui --features "std,gui-egui"
//! MAX_INSTRUCTIONS=250000000 ./target/release/examples/dlxlinux_egui.exe
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

/// DLX Linux disk geometry (from bochsrc.bxrc)
const DLX_CYLINDERS: u16 = 306;
const DLX_HEADS: u8 = 4;
const DLX_SPT: u8 = 17;

fn main() {
    // Initialize tracing
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
    // Find required files
    // =========================================================================
    let workspace_root = find_workspace_root();

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

    let disk_path = find_path(
        &workspace_root,
        &["dlxlinux/hd10meg.img", "../dlxlinux/hd10meg.img"],
    )
    .expect("Could not find DLX Linux disk image (hd10meg.img)");
    println!("Disk image: {}", disk_path.display());

    // =========================================================================
    // Create shared display
    // =========================================================================
    let shared = Arc::new(Mutex::new(SharedDisplay::new()));
    let shared_for_emu = Arc::clone(&shared);

    // =========================================================================
    // Spawn emulator on background thread
    // =========================================================================
    let disk_path_str = disk_path.to_string_lossy().to_string();
    let max_instructions: u64 = std::env::var("MAX_INSTRUCTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500_000_000);

    let emu_thread = std::thread::Builder::new()
        .stack_size(1500 * 1024 * 1024) // 1.5 GB stack
        .name("Emulator".to_string())
        .spawn(move || {
            loop {
                // Clear stop flag and mark running before each (re)start
                {
                    let mut d = shared_for_emu.lock().unwrap();
                    d.stop_flag.store(false, Ordering::Relaxed);
                    d.emu_running = true;
                    d.reset_requested = false;
                }

                if let Err(e) = run_emulator(
                    &bios_data,
                    vga_bios.as_deref(),
                    &disk_path_str,
                    Arc::clone(&shared_for_emu),
                    max_instructions,
                ) {
                    eprintln!("Emulator error: {:?}", e);
                }

                // Only restart if the GUI explicitly requested a reset
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
    // Start at 1x VGA (720×400) plus 26px status bar + small padding, guaranteed to
    // fit on any screen including 125% DPI on 1080p. The egui scale logic auto-scales
    // to 2x when the user maximizes or resizes the window larger. Min size matches 1x.
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 450.0])
            .with_min_inner_size([720.0, 426.0])
            .with_title("Rusty Box - Starting..."),
        ..Default::default()
    };

    let shared_for_gui = Arc::clone(&shared);
    let _ = eframe::run_native(
        "Rusty Box",
        native_options,
        Box::new(move |cc| Ok(Box::new(RustyBoxApp::new(cc, shared_for_gui)))),
    );

    // Window closed — signal emulator to stop
    if let Ok(mut display) = shared.lock() {
        display.emu_running = false;
    }

    // Wait for emulator thread (don't block forever)
    let _ = emu_thread.join();
}

fn run_emulator(
    bios_data: &[u8],
    vga_bios: Option<&[u8]>,
    disk_path: &str,
    shared: Arc<Mutex<SharedDisplay>>,
    max_instructions: u64,
) -> Result<()> {
    let config = EmulatorConfig {
        guest_memory_size: 32 * 1024 * 1024,
        host_memory_size: 32 * 1024 * 1024,
        memory_block_size: 128 * 1024,
        ips: 15_000_000,
        pci_enabled: true,
        ..Default::default()
    };

    let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;

    // Wire the shared stop_flag so the GUI reset button can interrupt run_interactive
    emu.stop_flag = {
        let d = shared.lock().unwrap();
        Arc::clone(&d.stop_flag)
    };

    // Set BridgeGui as the GUI
    let bridge = BridgeGui::new(Arc::clone(&shared));
    emu.set_gui(bridge);

    // Initialize hardware
    emu.init_memory_and_pc_system()?;

    // Load BIOS
    let bios_size = bios_data.len() as u64;
    let bios_load_addr = !(bios_size - 1);
    emu.load_bios(bios_data, bios_load_addr)?;

    // Load VGA BIOS
    if let Some(vga_data) = vga_bios {
        emu.load_optional_rom(vga_data, 0xC0000)?;
    }

    // Initialize CPU and devices
    emu.init_cpu_and_devices()?;

    // Configure CMOS
    emu.configure_memory_in_cmos(640, 31 * 1024);
    emu.configure_disk_geometry_in_cmos(0, DLX_CYLINDERS, DLX_HEADS, DLX_SPT);
    emu.configure_boot_sequence(2, 0, 0);

    // Attach disk
    emu.attach_disk(0, 0, disk_path, DLX_CYLINDERS, DLX_HEADS, DLX_SPT)
        .expect("Failed to attach disk image");

    // Initialize GUI, reset, start
    emu.init_gui(0, &[])?;
    emu.reset(ResetReason::Hardware)?;
    emu.init_gui_signal_handlers();
    emu.start();

    println!("Emulator started (max {} instructions)", max_instructions);
    let start_time = Instant::now();

    // Run the emulator
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

    // Dump VGA text
    println!("\n===== VGA TEXT DUMP =====");
    println!("{}", emu.vga_text_dump());

    // Signal GUI that emulator has stopped
    if let Ok(mut display) = shared.lock() {
        display.emu_running = false;
    }

    if let Some(ref mut gui) = emu.gui_mut() {
        gui.exit();
    }

    Ok(())
}

// =========================================================================
// File finding helpers (same logic as dlxlinux.rs)
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
