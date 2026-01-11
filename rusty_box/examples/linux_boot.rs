//! Linux Boot Example
//!
//! This example demonstrates booting DLX Linux from a hard disk image.
//! DLX Linux is a minimal Linux distribution that fits on a small disk.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example linux_boot --features std --release
//! ```
//!
//! # Requirements
//!
//! - BIOS-bochs-latest (128KB BIOS ROM)
//! - hd10meg.img (DLX Linux disk image)
//!
//! Both files should be in the parent directory or dlxlinux subdirectory.

use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, ResetReason},
    emulator::{Emulator, EmulatorConfig},
};
use std::fs;

fn main() {
    // Initialize tracing for logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    println!("==============================================");
    println!("     DLX Linux Boot Demo - RustyBox");
    println!("==============================================");
    println!();

    if let Err(e) = run_linux() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn find_file(name: &str, search_paths: &[&str]) -> Option<String> {
    for base in search_paths {
        let path = format!("{}/{}", base, name);
        if std::path::Path::new(&path).exists() {
            return Some(path);
        }
    }
    None
}

fn run_linux() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Search paths for required files
    let search_paths = [
        ".",
        "..",
        "../dlxlinux",
        "dlxlinux",
    ];

    // Find BIOS
    let bios_path = find_file("BIOS-bochs-latest", &search_paths)
        .ok_or("Could not find BIOS-bochs-latest")?;
    
    println!("Found BIOS: {}", bios_path);

    // Find disk image
    let disk_path = find_file("hd10meg.img", &search_paths)
        .ok_or("Could not find hd10meg.img")?;
    
    println!("Found disk image: {}", disk_path);
    println!();

    // Load BIOS
    let bios_data = fs::read(&bios_path)?;
    
    println!("BIOS size: {} bytes", bios_data.len());

    // Create emulator configuration (32MB RAM like Bochs config)
    let config = EmulatorConfig {
        guest_memory_size: 32 * 1024 * 1024,  // 32 MB
        host_memory_size: 32 * 1024 * 1024,
        memory_block_size: 128 * 1024,
        ips: 15_000_000,  // 15 MIPS (matching Bochs config)
        pci_enabled: false,
        cpu_params: Default::default(),
    };

    println!("Creating emulator with {} MB RAM...", config.guest_memory_size / (1024 * 1024));
    let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;

    // Initialize emulator
    println!("Initializing emulator...");
    emu.initialize()?;

    // Configure CMOS memory
    // Base: 640KB, Extended: 31744KB (32MB - 1MB)
    println!("Configuring CMOS...");
    emu.configure_memory_in_cmos(640, 31744);
    
    // Configure hard drive in CMOS (type 47 = user-defined)
    emu.configure_disk_in_cmos(0, 47);

    // Attach hard disk image
    // DLX Linux uses CHS: 306 cylinders, 4 heads, 17 sectors per track
    println!("Attaching disk image...");
    emu.attach_disk(0, 0, &disk_path, 306, 4, 17)?;

    // Load BIOS at 0xFFFE0000 (128KB BIOS)
    let bios_address = 0x10000_0000u64 - bios_data.len() as u64;
    println!("Loading BIOS at {:#x}...", bios_address);
    emu.load_bios(&bios_data, bios_address)?;

    // Perform hardware reset
    println!("Performing hardware reset...");
    emu.reset(ResetReason::Hardware)?;

    println!();
    println!("==============================================");
    println!("     Emulator Configuration Complete");
    println!("==============================================");
    println!();
    println!("Memory: 32 MB");
    println!("IPS: 15,000,000");
    println!("Disk: {} (CHS: 306/4/17)", disk_path);
    println!();
    println!("Starting CPU at RIP = {:#x}", emu.rip());
    println!();
    println!("Note: Full Linux boot requires complete CPU instruction");
    println!("      implementation (protected mode, interrupts, etc.)");
    println!();

    // Start timers
    emu.prepare_run();

    // Run a limited number of instructions to demonstrate
    println!("Starting CPU execution...");
    println!();

    // Execute CPU loop
    // Note: This will run until an error occurs (e.g., unimplemented instruction)
    // In a full implementation, this would run until HLT or shutdown
    let result = emu.cpu.cpu_loop(&mut emu.memory, &[]);
    
    let final_rip = emu.cpu.rip();
    
    match result {
        Ok(()) => {
            println!("CPU loop completed normally");
        }
        Err(e) => {
            println!();
            println!("CPU stopped: {:?}", e);
        }
    }

    println!();
    println!("Demo complete.");
    println!("Final RIP = {:#x}", final_rip);
    println!();
    println!("To boot Linux fully, additional CPU features are needed:");
    println!("  - Complete protected mode support");
    println!("  - Interrupt handling (INT, IRET)");
    println!("  - I/O instructions (IN, OUT)");
    println!("  - All x86 instructions used by BIOS and Linux kernel");

    Ok(())
}

