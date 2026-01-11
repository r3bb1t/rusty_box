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
    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_max_level(Level::INFO)
        .init();

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
    let bios_paths = [
        "../cpp_orig/bochs/bios/BIOS-bochs-latest",
        "../BIOS-bochs-latest",
        "BIOS-bochs-latest",
    ];
    
    let vga_bios_paths = [
        "../cpp_orig/bochs/bios/VGABIOS-lgpl-latest.bin",
        "../VGABIOS-lgpl-latest.bin",
        "VGABIOS-lgpl-latest.bin",
    ];

    let disk_paths = [
        "../dlxlinux/hd10meg.img",
        "dlxlinux/hd10meg.img",
        "hd10meg.img",
    ];

    let bios_data = bios_paths
        .iter()
        .find_map(|path| {
            println!("  Trying BIOS: {}", path);
            std::fs::read(path).ok()
        })
        .expect("Could not find BIOS file (BIOS-bochs-latest)");
    println!("✓ BIOS loaded: {} bytes", bios_data.len());

    let vga_bios_data = vga_bios_paths
        .iter()
        .find_map(|path| {
            println!("  Trying VGA BIOS: {}", path);
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
            println!("  Trying disk: {}", path);
            std::path::Path::new(path).exists()
        })
        .expect("Could not find DLX Linux disk image (hd10meg.img)");
    println!("✓ Disk image found: {}", disk_path);
    println!();

    // =========================================================================
    // Create and configure emulator
    // =========================================================================
    let config = EmulatorConfig {
        guest_memory_size: 32 * 1024 * 1024,  // 32 MB (from bochsrc.bxrc)
        host_memory_size: 32 * 1024 * 1024,
        memory_block_size: 128 * 1024,
        ips: 15_000_000,  // IPS from bochsrc.bxrc
        pci_enabled: false,
        ..Default::default()
    };

    tracing::info!("Creating emulator: {} MB RAM, {} MIPS", 
        config.guest_memory_size / (1024 * 1024),
        config.ips / 1_000_000);

    let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;

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
    tracing::info!("Attaching disk image: {}", disk_path);
    emu.attach_disk(0, 0, disk_path, DLX_CYLINDERS, DLX_HEADS, DLX_SPT)
        .expect("Failed to attach disk image");
    tracing::info!("✓ Disk attached: CHS={}/{}/{}", DLX_CYLINDERS, DLX_HEADS, DLX_SPT);

    // =========================================================================
    // Hardware reset
    // =========================================================================
    emu.reset(ResetReason::Hardware)?;
    tracing::info!("✓ Hardware reset complete");

    // =========================================================================
    // Show boot state
    // =========================================================================
    println!();
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║                      SYSTEM STATE                          ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║  CS:IP  = F000:{:04X}                                       ║", emu.rip());
    println!("║  A20    = {}                                          ║", 
        if emu.pc_system.get_enable_a20() { "enabled " } else { "disabled" });
    println!("║  Memory = {} MB                                           ║", 32);
    println!("║  Disk   = {} cylinders × {} heads × {} spt               ║", 
        DLX_CYLINDERS, DLX_HEADS, DLX_SPT);
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
    emu.prepare_run();

    let start_time = Instant::now();
    
    // Run with instruction limit to allow debugging
    const MAX_INSTRUCTIONS: u64 = 100_000_000; // 100M instructions for boot
    
    let result = emu.cpu.cpu_loop_n(&mut emu.memory, &[], MAX_INSTRUCTIONS);
    
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
            println!("║  Time:         {:>12.3} sec                       ║", elapsed.as_secs_f64());
            if elapsed.as_secs_f64() > 0.001 {
                let mips = executed as f64 / elapsed.as_secs_f64() / 1_000_000.0;
                println!("║  Speed:        {:>12.2} MIPS                      ║", mips);
            }
        }
        Err(ref e) => {
            println!("║  Error: {:?}", e);
        }
    }
    
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║  Final RIP:   {:#018x}                          ║", emu.cpu.rip());
    println!("║  EAX={:08x} EBX={:08x} ECX={:08x} EDX={:08x}  ║",
        emu.cpu.eax(), emu.cpu.ebx(), emu.cpu.ecx(), emu.cpu.edx());
    println!("║  ESP={:08x} EBP={:08x} ESI={:08x} EDI={:08x}  ║",
        emu.cpu.esp(), emu.cpu.ebp(), emu.cpu.esi(), emu.cpu.edi());
    println!("╚════════════════════════════════════════════════════════════╝");

    Ok(())
}

