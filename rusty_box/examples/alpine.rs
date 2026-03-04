//! Alpine Linux Boot Example
//!
//! This example boots Alpine Linux, a modern lightweight Linux distribution.
//! It demonstrates the full hardware emulation with modern features:
//! - 256 MB RAM (configurable via ALPINE_RAM_MB env var)
//! - APIC, ACPI, PCI bus infrastructure
//! - SSE/SSE2 instruction support
//! - Serial port (16550 UART)
//!
//! ## Usage
//!
//! ```bash
//! # Set path to Alpine disk image
//! ALPINE_DISK=/path/to/alpine.img cargo run --release --example alpine --features std
//!
//! # Headless mode with debug output
//! RUSTY_BOX_HEADLESS=1 ALPINE_DISK=alpine.img cargo run --release --example alpine --features std
//!
//! # Custom RAM size (default: 256 MB)
//! ALPINE_RAM_MB=512 ALPINE_DISK=alpine.img cargo run --release --example alpine --features std
//!
//! # Custom disk geometry (auto-detected by default)
//! ALPINE_CHS=1024,16,63 ALPINE_DISK=alpine.img cargo run --release --example alpine --features std
//! ```

use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, ResetReason},
    emulator::{Emulator, EmulatorConfig},
    gui::{NoGui, TermGui},
    Result,
};
use std::time::Instant;

fn main() {
    const THREAD_STACK_SIZE: usize = 1500 * 1024 * 1024; // 1.5 GB

    std::thread::Builder::new()
        .stack_size(THREAD_STACK_SIZE)
        .name("Alpine Linux".to_string())
        .spawn(|| {
            if let Err(e) = run_alpine() {
                eprintln!("Emulator error: {:?}", e);
                std::process::exit(1);
            }
        })
        .expect("Failed to spawn emulator thread")
        .join()
        .expect("Emulator thread panicked");
}

/// Calculate CHS geometry from disk size in bytes.
/// Uses standard LBA-to-CHS translation for disks up to ~8GB.
fn auto_detect_geometry(disk_size: u64) -> (u16, u8, u8) {
    let total_sectors = disk_size / 512;

    if total_sectors == 0 {
        return (1, 1, 1);
    }

    // Standard geometry: 16 heads, 63 sectors per track
    let spt: u8 = 63;
    let heads: u8 = 16;
    let cylinders = (total_sectors / (heads as u64 * spt as u64)) as u16;

    // Cap at 16383 cylinders (CHS limit)
    let cylinders = cylinders.min(16383).max(1);

    (cylinders, heads, spt)
}

/// Parse CHS geometry from "C,H,S" string
fn parse_chs(s: &str) -> Option<(u16, u8, u8)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 3 {
        return None;
    }
    let c = parts[0].trim().parse::<u16>().ok()?;
    let h = parts[1].trim().parse::<u8>().ok()?;
    let s = parts[2].trim().parse::<u8>().ok()?;
    Some((c, h, s))
}

fn run_alpine() -> Result<()> {
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

    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║           Alpine Linux Boot - Rusty Box Emulator          ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║  Alpine Linux is a lightweight, security-oriented distro  ║");
    println!("║  Set ALPINE_DISK=/path/to/alpine.img to specify image     ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();

    // =========================================================================
    // Configuration from environment
    // =========================================================================
    let ram_mb: usize = std::env::var("ALPINE_RAM_MB")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(256);

    let ram_bytes = ram_mb * 1024 * 1024;
    println!("  RAM: {} MB", ram_mb);

    // =========================================================================
    // Find required files
    // =========================================================================
    let workspace_root = std::env::current_dir()
        .ok()
        .and_then(|mut dir| {
            loop {
                if dir.join("Cargo.toml").exists() {
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
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    // BIOS paths (same as dlxlinux)
    let bios_paths = [
        workspace_root.join("cpp_orig/bochs/bios/BIOS-bochs-latest"),
        workspace_root.join("cpp_orig/bochs/bios/BIOS-bochs-legacy"),
        workspace_root.join("BIOS-bochs-latest"),
        std::path::PathBuf::from("BIOS-bochs-latest"),
    ];

    let vga_bios_paths = [
        workspace_root.join("binaries/bios/VGABIOS-lgpl-latest.bin"),
        workspace_root.join("cpp_orig/bochs/bios/VGABIOS-lgpl-latest.bin"),
        workspace_root.join("VGABIOS-lgpl-latest.bin"),
        std::path::PathBuf::from("VGABIOS-lgpl-latest.bin"),
    ];

    // Alpine disk image — from env var or search common locations
    let disk_path = if let Ok(path) = std::env::var("ALPINE_DISK") {
        let p = std::path::PathBuf::from(&path);
        if !p.exists() {
            eprintln!("ERROR: ALPINE_DISK={} does not exist", path);
            std::process::exit(1);
        }
        p
    } else {
        // Search common locations
        let search_paths = [
            workspace_root.join("alpine/alpine.img"),
            workspace_root.join("alpine/alpine-virt.img"),
            workspace_root.join("alpine.img"),
            std::path::PathBuf::from("alpine.img"),
            std::path::PathBuf::from("alpine/alpine.img"),
        ];
        match search_paths.iter().find(|p| p.exists()) {
            Some(p) => p.clone(),
            None => {
                eprintln!("ERROR: No Alpine disk image found.");
                eprintln!();
                eprintln!("To create one, use qemu-img or download from Alpine:");
                eprintln!("  qemu-img create -f raw alpine.img 256M");
                eprintln!("Then install Alpine onto it, or use a pre-built image.");
                eprintln!();
                eprintln!("Set ALPINE_DISK=/path/to/alpine.img to specify the image.");
                std::process::exit(1);
            }
        }
    };

    // Load BIOS
    let bios_data = bios_paths
        .iter()
        .find_map(|path| {
            println!("  Trying BIOS: {}", path.display());
            std::fs::read(path).ok()
        })
        .expect("Could not find BIOS file (BIOS-bochs-latest)");
    println!("  BIOS loaded: {} bytes", bios_data.len());

    // Load VGA BIOS (optional)
    let vga_bios = vga_bios_paths.iter().find_map(|path| {
        let data = std::fs::read(path).ok()?;
        if data.len() % 512 != 0 {
            return None;
        }
        Some((path.clone(), data))
    });

    if let Some((ref vga_path, ref vga)) = vga_bios {
        println!("  VGA BIOS loaded: {} bytes ({})", vga.len(), vga_path.display());
    } else {
        println!("  VGA BIOS not found (optional)");
    }

    // Disk image info
    let disk_meta = std::fs::metadata(&disk_path).expect("Cannot read disk image metadata");
    let disk_size = disk_meta.len();
    println!("  Disk image: {} ({} MB)", disk_path.display(), disk_size / (1024 * 1024));

    // Determine disk geometry
    let (cylinders, heads, spt) = if let Some(chs_str) = std::env::var("ALPINE_CHS").ok() {
        parse_chs(&chs_str).unwrap_or_else(|| {
            eprintln!("ERROR: Invalid ALPINE_CHS format. Use C,H,S (e.g., 1024,16,63)");
            std::process::exit(1);
        })
    } else {
        auto_detect_geometry(disk_size)
    };
    println!("  Disk geometry: CHS={}/{}/{}", cylinders, heads, spt);
    println!();

    // =========================================================================
    // Detect headless mode
    // =========================================================================
    let headless = std::env::var_os("RUSTY_BOX_HEADLESS").is_some();

    // =========================================================================
    // Create and configure emulator
    // =========================================================================
    let config = EmulatorConfig {
        guest_memory_size: ram_bytes,
        host_memory_size: ram_bytes,
        memory_block_size: 128 * 1024,
        ips: 15_000_000,
        pci_enabled: true,
        ..Default::default()
    };

    let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;

    // =========================================================================
    // Set up GUI
    // =========================================================================
    if headless {
        emu.set_gui(NoGui::new());
        println!("(headless) RUSTY_BOX_HEADLESS=1: terminal repaint disabled");
    } else {
        let term_gui = TermGui::new();
        emu.set_gui(term_gui);
    }

    // =========================================================================
    // Initialize hardware
    // =========================================================================
    emu.init_memory_and_pc_system()?;

    // Load BIOS
    let bios_size = bios_data.len() as u64;
    let bios_load_addr = !(bios_size - 1);
    emu.load_bios(&bios_data, bios_load_addr)?;

    // Load VGA BIOS
    if let Some((_vga_path, vga_data)) = vga_bios {
        emu.load_optional_rom(&vga_data, 0xC0000)?;
    }

    // Initialize CPU and devices
    emu.init_cpu_and_devices()?;

    // =========================================================================
    // Configure CMOS
    // =========================================================================
    // Use the bytes-based API that correctly handles large RAM sizes
    emu.configure_memory_in_cmos_from_config();

    // Configure hard drive geometry in CMOS
    emu.configure_disk_geometry_in_cmos(0, cylinders, heads, spt);

    // Boot from hard disk
    emu.configure_boot_sequence(2, 0, 0);

    // =========================================================================
    // Attach disk image
    // =========================================================================
    let disk_path_str = disk_path.to_string_lossy().to_string();
    emu.attach_disk(0, 0, &disk_path_str, cylinders, heads, spt)
        .expect("Failed to attach disk image");
    println!("  Disk attached: CHS={}/{}/{}", cylinders, heads, spt);

    // =========================================================================
    // Initialize GUI and reset
    // =========================================================================
    emu.init_gui(0, &[])?;
    emu.reset(ResetReason::Hardware)?;
    emu.init_gui_signal_handlers();
    emu.start();

    // =========================================================================
    // Show boot state
    // =========================================================================
    println!();
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║                      SYSTEM STATE                         ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║  CS:IP  = F000:{:04X}                                      ║", emu.rip());
    println!("║  A20    = {}                                         ║",
        if emu.pc_system.get_enable_a20() { "enabled " } else { "disabled" });
    println!("║  Memory = {} MB                                         ║", ram_mb);
    println!("║  Disk   = {} cyl x {} heads x {} spt                    ║",
        cylinders, heads, spt);
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();

    // =========================================================================
    // Open BIOS output file if specified
    // =========================================================================
    if let Some(path) = std::env::var("BIOS_OUTPUT_FILE").ok() {
        if let Ok(file) = std::fs::File::create(&path) {
            println!("BIOS output will be written to: {}", path);
            emu.set_bios_output_file(file);
        }
    }

    // =========================================================================
    // Start execution
    // =========================================================================
    println!("Starting Alpine Linux boot...");
    println!();

    let start_time = Instant::now();

    let max_instructions: u64 = std::env::var("MAX_INSTRUCTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000_000); // 1B instructions default (more than DLX)

    // PS/2 Set 2 scancodes for "root\n"
    const LOGIN_SCANCODES: &[u8] = &[
        0x2D, 0xF0, 0x2D, // 'r' make + break
        0x44, 0xF0, 0x44, // 'o' make + break
        0x44, 0xF0, 0x44, // 'o' make + break
        0x2C, 0xF0, 0x2C, // 't' make + break
        0x5A, 0xF0, 0x5A, // Enter make + break
    ];
    const KEEP_ALIVE_SCANCODE: &[u8] = &[0x12, 0xF0, 0x12]; // Left Shift

    let result = if headless {
        let mut total_executed: u64 = 0;
        let mut run_result: Result<u64> = Ok(0);
        let mut logged_in = false;

        const PHASE_SIZE: u64 = 20_000_000;
        'phases: loop {
            if total_executed >= max_instructions {
                break 'phases;
            }
            let run_for = PHASE_SIZE.min(max_instructions - total_executed);
            match emu.run_interactive(run_for) {
                Ok(n) => total_executed += n,
                Err(e) => {
                    run_result = Err(e);
                    break 'phases;
                }
            }
            run_result = Ok(total_executed);

            // Check VGA text for boot progress
            if total_executed >= 100_000_000 {
                let vga_text = emu.vga_scan_text_memory();
                let has_login = vga_text.contains("login:");

                let preview: Vec<&str> = vga_text
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .take(3)
                    .collect();
                let preview_str = if preview.is_empty() {
                    "(blank/empty)".to_string()
                } else {
                    preview.join(" | ")
                };
                println!(
                    "[{}M] VGA: {}{}",
                    total_executed / 1_000_000,
                    preview_str,
                    if has_login { " *** LOGIN DETECTED ***" } else { "" }
                );

                if has_login && !logged_in {
                    println!(
                        "(headless) Injecting 'root\\n' at {}M instructions",
                        total_executed / 1_000_000
                    );
                    for &sc in LOGIN_SCANCODES {
                        emu.send_scancode(sc);
                    }
                    logged_in = true;
                } else {
                    for &sc in KEEP_ALIVE_SCANCODE {
                        emu.send_scancode(sc);
                    }
                }
            }
        }
        run_result
    } else {
        emu.run_interactive(max_instructions)
    };

    let elapsed = start_time.elapsed();

    // =========================================================================
    // Show execution results
    // =========================================================================
    println!();
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║                    EXECUTION RESULTS                      ║");
    println!("╠════════════════════════════════════════════════════════════╣");

    match result {
        Ok(executed) => {
            println!("║  Instructions: {:>15}                       ║", executed);
            println!("║  Time:         {:>12.3} sec                      ║", elapsed.as_secs_f64());
            if elapsed.as_secs_f64() > 0.001 {
                let mips = executed as f64 / elapsed.as_secs_f64() / 1_000_000.0;
                println!("║  Speed:        {:>12.2} MIPS                     ║", mips);
            }
        }
        Err(ref e) => {
            println!("║  Error: {:?}", e);
        }
    }

    // Debug port output
    let e9 = emu.devices.take_port_e9_output();
    if !e9.is_empty() {
        println!();
        println!("===== BOCHS DEBUG PORT OUTPUT (0xE9/0x402/0x403/0x500) =====");
        print!("{}", String::from_utf8_lossy(&e9));
    }

    println!("╠════════════════════════════════════════════════════════════╣");
    println!(
        "║  Final RIP:   {:#018x}  CS={:04x} mode={}         ║",
        emu.cpu.rip(),
        emu.cpu.get_cs_selector(),
        emu.cpu.get_cpu_mode()
    );
    println!(
        "║  EAX={:08x} EBX={:08x} ECX={:08x} EDX={:08x} ║",
        emu.cpu.eax(), emu.cpu.ebx(), emu.cpu.ecx(), emu.cpu.edx()
    );
    println!(
        "║  ESP={:08x} EBP={:08x} ESI={:08x} EDI={:08x} ║",
        emu.cpu.esp(), emu.cpu.ebp(), emu.cpu.esi(), emu.cpu.edi()
    );
    println!("╚════════════════════════════════════════════════════════════╝");

    // Cleanup
    if let Some(ref mut gui) = emu.gui_mut() {
        gui.exit();
    }

    // Headless diagnostics
    if headless {
        let (ata_reads, ata_writes) = emu.device_manager.ata_io_counts();
        println!("\n===== ATA DIAGNOSTIC =====");
        println!("ATA read_count={}, write_count={}", ata_reads, ata_writes);

        println!("\n===== IRQ DELIVERY CHAIN =====");
        println!("tick() calls:          {}", emu.device_manager.diag_tick_count);
        println!("PIT fires (check_irq0): {}", emu.device_manager.diag_pit_fires);
        println!("IRQ0 latched (raise):   {}", emu.device_manager.diag_irq0_latched);
        println!("iac() calls:            {}", emu.device_manager.diag_iac_count);
        let pic_diag = emu.device_manager.pic_diag();
        println!("PIC state:              {}", pic_diag);
        print!("iac vectors:            ");
        for (v, &count) in emu.device_manager.diag_vector_hist.iter().enumerate() {
            if count > 0 {
                print!("0x{:02x}={} ", v, count);
            }
        }
        println!();
        println!("CPU state:              {}", emu.cpu.cpu_diag_string());
        println!("CPU RIP:                {:#x}", emu.cpu.rip());

        // VGA text dump
        println!("\n===== VGA TEXT DUMP =====");
        let vga_text = emu.vga_scan_text_memory();
        for line in vga_text.lines() {
            if !line.trim().is_empty() {
                println!("  {}", line);
            }
        }
        println!("===== END VGA TEXT DUMP =====");
    }

    Ok(())
}
