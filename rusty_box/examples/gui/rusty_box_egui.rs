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
    AlpineDirect {
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
        BootProfile::Alpine { .. } => "Rusty Box - Alpine Linux (BIOS)",
        BootProfile::AlpineDirect { .. } => "Rusty Box - Alpine Linux (Direct Boot)",
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
        BootProfile::Alpine { ram_mb, .. } | BootProfile::AlpineDirect { ram_mb, .. } => {
            ram_mb * 1024 * 1024
        }
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
        BootProfile::AlpineDirect { iso_path, .. } => {
            // Direct kernel boot — extract vmlinuz + initramfs from ISO
            emu.configure_memory_in_cmos_from_config();
            // Attach ISO as CD-ROM so Alpine Init can mount the squashfs root filesystem
            let iso_path_str = iso_path.to_string_lossy().to_string();
            emu.attach_cdrom(1, 0, &iso_path_str)
                .expect("Failed to attach Alpine ISO as CD-ROM");
            println!("CD-ROM attached: {}", iso_path.display());
            let iso_data = std::fs::read(iso_path).expect("Failed to read ISO");
            let (kernel, initramfs) =
                extract_kernel_from_iso(&iso_data).expect("Failed to extract kernel from ISO");
            let cmdline = std::env::var("CMDLINE").unwrap_or_else(|_| {
                "console=ttyS0,115200 console=tty0 earlycon=uart8250,io,0x3f8,115200n8 nomodeset nokaslr kfence.sample_interval=0 modules=cdrom,sr_mod,isofs".to_string()
            });
            println!(
                "Direct boot: kernel={} bytes, initramfs={} bytes",
                kernel.len(),
                initramfs.len()
            );
            emu.init_gui(0, &[])?;
            emu.reset(ResetReason::Hardware)?;
            emu.init_gui_signal_handlers();
            emu.init_vga_text_mode3(); // No BIOS runs — initialize VGA for kernel vgacon
            emu.start();
            emu.setup_direct_linux_boot(&kernel, Some(&initramfs), &cmdline)?;
            // Skip the common init_gui/reset/start below
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
                Err(ref e) => eprintln!("Execution error: {:?}", e),
            }
            println!("\n===== VGA TEXT DUMP =====");
            println!("{}", emu.vga_text_dump());
            if let Ok(mut display) = shared.lock() {
                display.emu_running = false;
            }
            if let Some(ref mut gui) = emu.gui_mut() {
                gui.exit();
            }
            return Ok(());
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

            let iso_path = find_alpine_iso(workspace_root);
            println!("Boot profile: Alpine Linux BIOS ({} MB RAM)", ram_mb);
            println!("ISO: {}", iso_path.display());
            BootProfile::Alpine { iso_path, ram_mb }
        }
        "alpine-direct" => {
            let ram_mb: usize = std::env::var("ALPINE_RAM_MB")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(256);

            let iso_path = find_alpine_iso(workspace_root);
            println!("Boot profile: Alpine Linux Direct Boot ({} MB RAM)", ram_mb);
            println!("ISO: {}", iso_path.display());
            BootProfile::AlpineDirect { iso_path, ram_mb }
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

fn find_alpine_iso(workspace_root: &std::path::Path) -> std::path::PathBuf {
    if let Ok(path) = std::env::var("ALPINE_ISO") {
        let p = std::path::PathBuf::from(&path);
        if !p.exists() {
            eprintln!("ERROR: ALPINE_ISO={} does not exist", path);
            std::process::exit(1);
        }
        return p;
    }
    if let Ok(path) = std::env::var("ALPINE_DISK") {
        let p = std::path::PathBuf::from(&path);
        if !p.exists() {
            eprintln!("ERROR: ALPINE_DISK={} does not exist", path);
            std::process::exit(1);
        }
        return p;
    }
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
            eprintln!("ERROR: No Alpine ISO found. Set ALPINE_ISO=/path/to/alpine.iso");
            std::process::exit(1);
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

// =========================================================================
// ISO extraction for direct kernel boot
// =========================================================================

fn extract_kernel_from_iso(iso_data: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let kernel = extract_from_iso(iso_data, &["BOOT", "VMLINUZ_VIRT."])?;
    let initramfs = extract_from_iso(iso_data, &["BOOT", "INITRAMFS_VIRT."])?;
    Some((kernel, initramfs))
}

fn extract_from_iso(iso_data: &[u8], target_path: &[&str]) -> Option<Vec<u8>> {
    let pvd_offset = 16 * 2048;
    if pvd_offset + 190 > iso_data.len() {
        return None;
    }
    let root_lba = u32::from_le_bytes([
        iso_data[pvd_offset + 158], iso_data[pvd_offset + 159],
        iso_data[pvd_offset + 160], iso_data[pvd_offset + 161],
    ]) as usize;
    let root_size = u32::from_le_bytes([
        iso_data[pvd_offset + 166], iso_data[pvd_offset + 167],
        iso_data[pvd_offset + 168], iso_data[pvd_offset + 169],
    ]) as usize;

    fn find_entry(iso: &[u8], dir_lba: usize, dir_size: usize, name: &str) -> Option<(usize, usize, bool)> {
        let dir_data_start = dir_lba * 2048;
        let dir_data_end = dir_data_start + dir_size;
        if dir_data_end > iso.len() { return None; }
        let dir_data = &iso[dir_data_start..dir_data_end];
        let mut offset = 0;
        while offset < dir_data.len() {
            let rec_len = dir_data[offset] as usize;
            if rec_len == 0 {
                let next_sect = ((offset / 2048) + 1) * 2048;
                if next_sect >= dir_data.len() { break; }
                offset = next_sect;
                continue;
            }
            if offset + 33 > dir_data.len() { break; }
            let name_len = dir_data[offset + 32] as usize;
            if offset + 33 + name_len > dir_data.len() { break; }
            let entry_name = std::str::from_utf8(&dir_data[offset + 33..offset + 33 + name_len]).unwrap_or("");
            let entry_lba = u32::from_le_bytes([
                dir_data[offset + 2], dir_data[offset + 3],
                dir_data[offset + 4], dir_data[offset + 5],
            ]) as usize;
            let entry_size = u32::from_le_bytes([
                dir_data[offset + 10], dir_data[offset + 11],
                dir_data[offset + 12], dir_data[offset + 13],
            ]) as usize;
            let is_dir = (dir_data[offset + 25] & 2) != 0;
            let clean_name = entry_name.split(';').next().unwrap_or(entry_name);
            if clean_name.eq_ignore_ascii_case(name) {
                return Some((entry_lba, entry_size, is_dir));
            }
            offset += rec_len;
        }
        None
    }

    let mut cur_lba = root_lba;
    let mut cur_size = root_size;
    for (i, component) in target_path.iter().enumerate() {
        let is_last = i == target_path.len() - 1;
        match find_entry(iso_data, cur_lba, cur_size, component) {
            Some((lba, size, is_dir)) => {
                if is_last {
                    let start = lba * 2048;
                    let end = start + size;
                    if end <= iso_data.len() {
                        return Some(iso_data[start..end].to_vec());
                    }
                    return None;
                }
                if !is_dir { return None; }
                cur_lba = lba;
                cur_size = size;
            }
            None => return None,
        }
    }
    None
}
