//! DLX Linux Boot Example
//!
//! This example boots DLX Linux, a minimal Linux distribution designed for Bochs.
//! It demonstrates the full hardware emulation including:
//! - PC system initialization
//! - BIOS loading and execution
//! - Hard disk image loading (hd10meg.img)
//! - Device initialization (PIC, PIT, CMOS, Keyboard, IDE)
//!
//! ## DLX Linux Configuration (from bochsrc.bxrc)
//! - Memory: 32 MB
//! - Boot device: Hard disk
//! - Disk geometry: 306 cylinders, 4 heads, 17 sectors per track
//! - IPS: 15000000

use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, ResetReason},
    emulator::{Emulator, EmulatorConfig},
    gui::{NoGui, TermGui},
    Result,
};
use std::time::Instant;
use tracing::Level;

/// DLX Linux disk geometry (from bochsrc.bxrc)
const DLX_CYLINDERS: u16 = 306;
const DLX_HEADS: u8 = 4;
const DLX_SPT: u8 = 17;

fn main() {
    // Use a larger stack size for debug builds
    const THREAD_STACK_SIZE: usize = if cfg!(debug_assertions) {
        1500 * 1024 * 1024
    } else {
        500 * 1024 * 1024
    };

    std::thread::Builder::new()
        .stack_size(THREAD_STACK_SIZE)
        .name("DLX Linux".to_string())
        .spawn(|| {
            if let Err(e) = run_dlxlinux() {
                eprintln!("Emulator error: {:?}", e);
                std::process::exit(1);
            }
        })
        .expect("Failed to spawn emulator thread")
        .join()
        .expect("Emulator thread panicked");
}

fn run_dlxlinux() -> Result<()> {
    // Initialize tracing (INFO level for cleaner output)
    // tracing_subscriber::fmt()
    //     .without_time()
    //     .with_target(false)
    //     // .with_max_level(Level::INFO)
    //     // .with_max_level(Level::TRACE)
    //     .with_max_level(Level::INFO)
    //     .init();

    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║              DLX Linux Boot - Rusty Box Emulator           ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║  DLX is a minimal Linux for Bochs demonstration            ║");
    println!("║  Login: root (no password)                                 ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();

    // =========================================================================
    // Find required files
    // =========================================================================
    // Try to find workspace root by looking for Cargo.toml or dlxlinux directory
    let workspace_root = std::env::current_dir()
        .ok()
        .and_then(|mut dir| {
            // Walk up directories looking for Cargo.toml or dlxlinux directory
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
        .or_else(|| {
            // Try from executable location (when running from cargo run)
            std::env::current_exe().ok().and_then(|exe| {
                let mut path = exe.parent()?;
                // If running from target/release/examples or target/debug/examples, go up 3 levels
                if path.ends_with("examples") {
                    path = path.parent()?.parent()?.parent()?;
                }
                if path.join("Cargo.toml").exists() || path.join("dlxlinux").exists() {
                    Some(path.to_path_buf())
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let bios_paths = [
        workspace_root.join("cpp_orig/bochs/bios/BIOS-bochs-latest"),
        workspace_root.join("BIOS-bochs-latest"),
        workspace_root.join("../cpp_orig/bochs/bios/BIOS-bochs-latest"),
        workspace_root.join("../BIOS-bochs-latest"),
        std::path::PathBuf::from("BIOS-bochs-latest"),
        std::path::PathBuf::from("../cpp_orig/bochs/bios/BIOS-bochs-latest"),
        std::path::PathBuf::from("../BIOS-bochs-latest"),
    ];

    let vga_bios_paths = [
        workspace_root.join("cpp_orig/bochs/bios/VGABIOS-lgpl-latest.bin"),
        workspace_root.join("VGABIOS-lgpl-latest.bin"),
        workspace_root.join("../cpp_orig/bochs/bios/VGABIOS-lgpl-latest.bin"),
        workspace_root.join("../VGABIOS-lgpl-latest.bin"),
        std::path::PathBuf::from("VGABIOS-lgpl-latest.bin"),
        std::path::PathBuf::from("../cpp_orig/bochs/bios/VGABIOS-lgpl-latest.bin"),
        std::path::PathBuf::from("../VGABIOS-lgpl-latest.bin"),
    ];

    let disk_paths = [
        workspace_root.join("dlxlinux/hd10meg.img"),
        workspace_root.join("../dlxlinux/hd10meg.img"),
        std::path::PathBuf::from("dlxlinux/hd10meg.img"),
        std::path::PathBuf::from("../dlxlinux/hd10meg.img"),
        std::path::PathBuf::from("hd10meg.img"),
    ];

    let bios_data = bios_paths
        .iter()
        .find_map(|path| {
            println!("  Trying BIOS: {}", path.display());
            std::fs::read(path).ok()
        })
        .expect("Could not find BIOS file (BIOS-bochs-latest)");
    println!("✓ BIOS loaded: {} bytes", bios_data.len());

    let vga_bios_data = vga_bios_paths.iter().find_map(|path| {
        println!("  Trying VGA BIOS: {}", path.display());
        std::fs::read(path).ok()
    });

    if let Some(ref vga) = vga_bios_data {
        println!("✓ VGA BIOS loaded: {} bytes", vga.len());
    } else {
        println!("⚠ VGA BIOS not found (optional)");
    }

    let disk_path = disk_paths
        .iter()
        .find(|path| {
            println!("  Trying disk: {}", path.display());
            path.exists()
        })
        .expect("Could not find DLX Linux disk image (hd10meg.img)");
    println!("✓ Disk image found: {}", disk_path.display());
    println!();

    // =========================================================================
    // Create and configure emulator
    // =========================================================================
    let config = EmulatorConfig {
        guest_memory_size: 32 * 1024 * 1024, // 32 MB (from bochsrc.bxrc)
        // host_memory_size: 32 * 1024 * 1024,
        host_memory_size: 32 * 1024 * 1024,
        memory_block_size: 128 * 1024,
        ips: 15_000_000, // IPS from bochsrc.bxrc
        pci_enabled: false,
        ..Default::default()
    };

    tracing::info!(
        "Creating emulator: {} MB RAM, {} MIPS",
        config.guest_memory_size / (1024 * 1024),
        config.ips / 1_000_000
    );

    let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;

    // =========================================================================
    // Set up GUI (must be done BEFORE initialize() to match original Bochs)
    // =========================================================================
    let term_gui = TermGui::new();
    emu.set_gui(term_gui);
    tracing::info!("✓ GUI set (TermGui)");

    // =========================================================================
    // Initialize hardware
    // =========================================================================
    tracing::info!("Initializing hardware...");
    emu.initialize()?;

    // Configure CMOS for 32MB memory
    // Base: 640KB, Extended: 31MB (32MB - 1MB)
    emu.configure_memory_in_cmos(640, 31 * 1024);

    // Configure hard drive in CMOS (drive type 47 = user-defined)
    emu.configure_disk_in_cmos(0, 47);

    // =========================================================================
    // Load BIOS ROMs
    // =========================================================================
    // Load main BIOS at 0xFFFE0000 (128KB BIOS)
    emu.load_bios(&bios_data, 0xfffe0000)?;
    tracing::info!("✓ Loaded system BIOS at 0xFFFE0000");

    // Load VGA BIOS at 0xC0000 (optional)
    if let Some(vga_data) = vga_bios_data {
        emu.load_optional_rom(&vga_data, 0xC0000)?;
        tracing::info!("✓ Loaded VGA BIOS at 0xC0000");
    }

    // =========================================================================
    // Attach disk image
    // =========================================================================
    tracing::info!("Attaching disk image: {}", disk_path.display());
    let disk_path_str = disk_path.to_string_lossy().to_string();
    emu.attach_disk(0, 0, &disk_path_str, DLX_CYLINDERS, DLX_HEADS, DLX_SPT)
        .expect("Failed to attach disk image");
    tracing::info!(
        "✓ Disk attached: CHS={}/{}/{}",
        DLX_CYLINDERS,
        DLX_HEADS,
        DLX_SPT
    );

    // =========================================================================
    // Initialize GUI (sets up terminal, but signal handlers after reset)
    // =========================================================================
    emu.init_gui(0, &[])?;
    tracing::info!("✓ Terminal GUI initialized");

    // =========================================================================
    // Hardware reset (enables A20, resets CPU and devices)
    // =========================================================================
    emu.reset(ResetReason::Hardware)?;
    tracing::info!("✓ Hardware reset complete");

    // =========================================================================
    // Initialize GUI signal handlers (after reset, before start_timers)
    // =========================================================================
    emu.init_gui_signal_handlers();
    tracing::info!("✓ GUI signal handlers initialized");

    // =========================================================================
    // Start timers (after signal handlers, matching original Bochs line 1384)
    // =========================================================================
    emu.start();
    tracing::info!("✓ Timers started");

    // =========================================================================
    // Show boot state
    // =========================================================================
    println!();
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║                      SYSTEM STATE                          ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!(
        "║  CS:IP  = F000:{:04X}                                       ║",
        emu.rip()
    );
    println!(
        "║  A20    = {}                                          ║",
        if emu.pc_system.get_enable_a20() {
            "enabled "
        } else {
            "disabled"
        }
    );
    println!(
        "║  Memory = {} MB                                           ║",
        32
    );
    println!(
        "║  Disk   = {} cylinders × {} heads × {} spt               ║",
        DLX_CYLINDERS, DLX_HEADS, DLX_SPT
    );
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();

    // Show BIOS reset vector
    let reset_vector_offset = 0x1FFF0usize;
    if reset_vector_offset + 16 <= bios_data.len() {
        let first_bytes = &bios_data[reset_vector_offset..reset_vector_offset + 16];
        tracing::debug!("BIOS reset vector bytes: {:02X?}", first_bytes);

        if first_bytes[0] == 0xEA {
            let offset = u16::from_le_bytes([first_bytes[1], first_bytes[2]]);
            let segment = u16::from_le_bytes([first_bytes[3], first_bytes[4]]);
            tracing::info!("Reset vector: JMP FAR {:04X}:{:04X}", segment, offset);
        }
    }

    // =========================================================================
    // Start execution
    // =========================================================================
    tracing::info!("Starting BIOS execution...");
    println!();
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║  Starting emulation - keyboard input enabled               ║");
    println!("║  Type 'root' and press Enter to login                     ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();

    let start_time = Instant::now();

    // Run with instruction limit to allow debugging
    const MAX_INSTRUCTIONS: u64 = 1_000_000_000; // 1M instructions - reasonable limit for testing
                                                 // const MAX_INSTRUCTIONS: u64 = 100; // 100k instructions - reasonable limit for testing

    // Use interactive loop that handles GUI events
    let result = emu.run_interactive(MAX_INSTRUCTIONS);

    let elapsed = start_time.elapsed();

    // =========================================================================
    // Show execution results
    // =========================================================================
    println!();
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║                    EXECUTION RESULTS                       ║");
    println!("╠════════════════════════════════════════════════════════════╣");

    match result {
        Ok(executed) => {
            println!("║  Instructions: {:>15}                        ║", executed);
            println!(
                "║  Time:         {:>12.3} sec                       ║",
                elapsed.as_secs_f64()
            );
            if elapsed.as_secs_f64() > 0.001 {
                let mips = executed as f64 / elapsed.as_secs_f64() / 1_000_000.0;
                println!(
                    "║  Speed:        {:>12.2} MIPS                      ║",
                    mips
                );
            }
        }
        Err(ref e) => {
            println!("║  Error: {:?}", e);
        }
    }

    println!("╠════════════════════════════════════════════════════════════╣");
    println!(
        "║  Final RIP:   {:#018x}                          ║",
        emu.cpu.rip()
    );
    println!(
        "║  EAX={:08x} EBX={:08x} ECX={:08x} EDX={:08x}  ║",
        emu.cpu.eax(),
        emu.cpu.ebx(),
        emu.cpu.ecx(),
        emu.cpu.edx()
    );
    println!(
        "║  ESP={:08x} EBP={:08x} ESI={:08x} EDI={:08x}  ║",
        emu.cpu.esp(),
        emu.cpu.ebp(),
        emu.cpu.esi(),
        emu.cpu.edi()
    );
    println!("╚════════════════════════════════════════════════════════════╝");

    // Cleanup: restore terminal if GUI was used
    if let Some(ref mut gui) = emu.gui_mut() {
        gui.exit();
    }

    Ok(())
}
