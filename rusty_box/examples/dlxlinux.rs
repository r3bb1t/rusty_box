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

// Note: this example requires the `std` feature (terminal GUI + disk access).

/// DLX Linux disk geometry (from bochsrc.bxrc)
const DLX_CYLINDERS: u16 = 306;
const DLX_HEADS: u8 = 4;
const DLX_SPT: u8 = 17;

fn main() {
    // Use a larger stack size for debug builds
    const THREAD_STACK_SIZE: usize = if cfg!(debug_assertions) {
        1500 * 1024 * 1024
    } else {
        1500 * 1024 * 1024 // Increased to 1.5 GB for 1GB memory config
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
    // Check for BIOS output configuration
    let bios_output_file = std::env::var("BIOS_OUTPUT_FILE").ok();
    let bios_quiet_mode = std::env::var("BIOS_QUIET_MODE").is_ok();

    // Initialize tracing - respect RUST_LOG env var, with WARN as default
    // (set RUST_LOG=debug or RUST_LOG=info to see more detail)
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
        // Prefer BIOS-bochs-latest (128KB) - the modern BIOS
        workspace_root.join("cpp_orig/bochs/bios/BIOS-bochs-latest"),
        workspace_root.join("cpp_orig/bochs/bios/BIOS-bochs-legacy"),
        workspace_root.join("cpp_orig/bochs/bios/bios.bin-1.13.0"),
        // Fallbacks (if user has BIOS copied elsewhere)
        workspace_root.join("BIOS-bochs-latest"),
        workspace_root.join("BIOS-bochs-legacy"),
        workspace_root.join("bios.bin-1.13.0"),
        workspace_root.join("../cpp_orig/bochs/bios/BIOS-bochs-latest"),
        workspace_root.join("../cpp_orig/bochs/bios/BIOS-bochs-legacy"),
        workspace_root.join("../BIOS-bochs-latest"),
        std::path::PathBuf::from("BIOS-bochs-latest"),
        std::path::PathBuf::from("../cpp_orig/bochs/bios/BIOS-bochs-latest"),
    ];

    let vga_bios_paths = [
        // Prefer user-provided binaries folder (if present)
        workspace_root.join("binaries/bios/VGABIOS-lgpl-latest.bin"),
        workspace_root.join("binaries/bios/VGABIOS-lgpl-latest-cirrus.bin"),
        workspace_root.join("binaries/bios/VGABIOS-lgpl-latest-debug.bin"),
        // Mirrored Bochs BIOS directory
        workspace_root.join("cpp_orig/bochs/bios/VGABIOS-lgpl-latest.bin"),
        workspace_root.join("cpp_orig/bochs/bios/VGABIOS-lgpl-latest-cirrus.bin"),
        workspace_root.join("cpp_orig/bochs/bios/VGABIOS-lgpl-latest-debug.bin"),
        // Fallbacks
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
        .expect("Could not find BIOS file (BIOS-bochs-legacy or BIOS-bochs-latest)");
    println!("✓ BIOS loaded: {} bytes", bios_data.len());

    // Option ROMs (like VGABIOS) must be sized in 512-byte blocks (Bochs behavior).
    // Skip any readable file that doesn't satisfy this, and keep searching.
    let vga_bios = vga_bios_paths.iter().find_map(|path| {
        println!("  Trying VGA BIOS: {}", path.display());
        let data = std::fs::read(path).ok()?;
        if data.len() % 512 != 0 {
            println!(
                "    Skipping VGA BIOS (invalid option ROM size: {} bytes, not multiple of 512)",
                data.len()
            );
            return None;
        }
        Some((path.clone(), data))
    });

    if let Some((ref vga_path, ref vga)) = vga_bios {
        println!(
            "✓ VGA BIOS loaded: {} bytes ({})",
            vga.len(),
            vga_path.display()
        );
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
    // Detect headless mode early (needed for emulator config)
    // =========================================================================
    let headless = std::env::var_os("RUSTY_BOX_HEADLESS").is_some();

    // =========================================================================
    // Create and configure emulator
    // =========================================================================
    let config = EmulatorConfig {
        // Match bochsrc.bxrc: 32 MB RAM
        // Stack overflow fixed by Boxing icache.mpool and returning Box<Emulator>
        guest_memory_size: 32 * 1024 * 1024, // 32 MB
        host_memory_size: 32 * 1024 * 1024,  // 32 MB
        memory_block_size: 128 * 1024,
        ips: 15_000_000,
        pci_enabled: true,
        ..Default::default()
    };

    tracing::info!(
        "Creating emulator: {} MB RAM, {} MIPS",
        config.guest_memory_size / (1024 * 1024),
        config.ips / 1_000_000,
    );

    let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;

    // =========================================================================
    // Set up GUI (must be done BEFORE initialize() to match original Bochs)
    // =========================================================================
    if headless {
        emu.set_gui(NoGui::new());
        tracing::info!("✓ GUI set (NoGui / headless)");
        println!("(headless) RUSTY_BOX_HEADLESS=1: terminal repaint disabled");
    } else {
        let term_gui = TermGui::new();
        emu.set_gui(term_gui);
        tracing::info!("✓ GUI set (TermGui)");
    }

    // =========================================================================
    // Initialize hardware - Part 1: Memory and PC system
    // =========================================================================
    // Following original Bochs sequence from main.cc:1312-1353:
    // 1. Memory init (line 1312)
    // 2. Load BIOS (line 1315)
    // 3. CPU init (line 1337)
    // 4. Device init (line 1353)
    tracing::info!("Initializing hardware...");
    emu.init_memory_and_pc_system()?;

    // =========================================================================
    // Load BIOS ROMs (AFTER memory init, BEFORE CPU init)
    // =========================================================================
    // BIOS must be loaded at 0xF0000 (matching original Bochs)
    // This makes it accessible in the F000 segment (0xF0000-0xFFFFF)
    // The memory system maps 0xE0000-0xFFFFF to BIOS ROM via bios_map_last128k()
    // At CPU reset, CS.base is specially set to 0xFFFF0000, allowing the first
    // instruction fetch from 0xFFFFFFF0 to access the same ROM data
    let bios_size = bios_data.len() as u64;
    // Calculate BIOS load address following original Bochs logic:
    // romaddress = ~(size - 1) for reset vector support (misc_mem.cc:363)
    // 64KB BIOS:  ~0xFFFF = 0xFFFF0000 (ends at 4GB, wraps in u32)
    // 128KB BIOS: ~0x1FFFF = 0xFFFE0000
    // Validation: (romaddress + size) should wrap to 0 OR equal 0x100000
    let bios_load_addr = !(bios_size - 1);
    tracing::info!(
        "BIOS size: {} bytes ({} KB), load address: {:#x}",
        bios_size,
        bios_size / 1024,
        bios_load_addr
    );
    emu.load_bios(&bios_data, bios_load_addr)?;
    tracing::info!("✓ Loaded system BIOS at {:#x}", bios_load_addr);

    // Load VGA BIOS at 0xC0000 (optional)
    if let Some((_vga_path, vga_data)) = vga_bios {
        emu.load_optional_rom(&vga_data, 0xC0000)?;
        tracing::info!("✓ Loaded VGA BIOS at 0xC0000");
    }

    // =========================================================================
    // Initialize hardware - Part 2: CPU and devices
    // =========================================================================
    emu.init_cpu_and_devices()?;

    // =========================================================================
    // Configure CMOS (AFTER device initialization)
    // =========================================================================
    // Configure CMOS for 32 MB memory (matches bochsrc.bxrc)
    // Uses guest_memory_size from config (32 MB) — avoids double-counting base_kb
    emu.configure_memory_in_cmos_from_config();

    // Configure hard drive geometry in CMOS (matching Bochs harddrv.cc:448-474)
    // Sets type=0xF (extended) + registers 0x19, 0x1B-0x23 for drive 0
    emu.configure_disk_geometry_in_cmos(0, DLX_CYLINDERS, DLX_HEADS, DLX_SPT);

    // Configure boot sequence: boot from hard disk first (matching Bochs floppy.cc:332-337)
    // ELTORITO boot device codes: 0=none, 1=floppy, 2=hard disk, 3=cdrom
    emu.configure_boot_sequence(2, 0, 0);

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

    // Open BIOS output file if specified
    let bios_file_handle = if let Some(ref path) = bios_output_file {
        match std::fs::File::create(path) {
            Ok(file) => {
                println!("BIOS output will be written to: {}", path);
                Some(file)
            }
            Err(e) => {
                eprintln!("Failed to create BIOS output file '{}': {}", path, e);
                None
            }
        }
    } else {
        None
    };

    // Set BIOS output file in emulator
    if let Some(file) = bios_file_handle {
        emu.set_bios_output_file(file);
    }

    // Show BIOS output section header
    if bios_quiet_mode || bios_output_file.is_some() {
        println!();
        println!("╔════════════════════════════════════════════════════════════╗");
        println!("║           BIOS OUTPUT (ports 0x402/0x403/0xE9)             ║");
        println!("╚════════════════════════════════════════════════════════════╝");
        println!();
    }

    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║  Starting emulation - keyboard input enabled               ║");
    println!("║  Type 'root' and press Enter to login                     ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();

    let start_time = Instant::now();

    // Run with instruction limit. The kernel first enters HLT at ~132M instructions.
    // After that, timer ISRs wake the scheduler. Init mounts rootfs, starts getty,
    // which shows "dlx login:". In interactive mode the HLT sync keeps virtual
    // time close to real time, so the console blank timer fires correctly at ~600s.
    // Override with MAX_INSTRUCTIONS env var:
    //   MAX_INSTRUCTIONS=132865710   → stop at first kernel HLT (ATA/IRQ diagnostics)
    //   MAX_INSTRUCTIONS=200000000   → headless diagnostic run after HLT
    //   MAX_INSTRUCTIONS=500000000   → headless run trying to reach login (default)
    let max_instructions: u64 = std::env::var("MAX_INSTRUCTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500_000_000);

    // PS/2 Set 2 scancodes for "root\n". Break code = 0xF0 prefix + make code.
    // 'r'=0x2D, 'o'=0x44, 't'=0x2C, Enter=0x5A
    const LOGIN_SCANCODES: &[u8] = &[
        0x2D, 0xF0, 0x2D, // 'r' make + break
        0x44, 0xF0, 0x44, // 'o' make + break
        0x44, 0xF0, 0x44, // 'o' make + break
        0x2C, 0xF0, 0x2C, // 't' make + break
        0x5A, 0xF0, 0x5A, // Enter make + break
    ];
    // Harmless Left-Shift make+break: resets Linux console blank timer without
    // typing any character (the kernel discards modifier-only keypresses from
    // the TTY input buffer, but do_keyboard_interrupt() still calls unblank_screen()
    // and resets the inactivity timer).
    const KEEP_ALIVE_SCANCODE: &[u8] = &[0x12, 0xF0, 0x12]; // Left Shift

    // In headless mode: run in 20M-instruction phases. After the kernel HLTs (~132M),
    // inject a Shift keep-alive every phase to prevent the console blank timer from
    // firing (~25M-instruction interval at our virtual-time rate). Once VGA text
    // contains "login:", inject "root\n" scancodes to log in.
    //
    // In interactive mode: HLT sync keeps virtual≈real time, blank fires at ~600s,
    // so no keep-alive injection needed — the user can type normally.
    let result = if headless {
        let mut total_executed: u64 = 0;
        let mut run_result: Result<u64> = Ok(0);
        let mut logged_in = false;
        let phase_size: u64 = 1_000_000;

        'phases: loop {
            if total_executed >= max_instructions {
                break 'phases;
            }
            let run_for = phase_size.min(max_instructions - total_executed);
            match emu.run_interactive(run_for) {
                Ok(n) => total_executed += n,
                Err(e) => {
                    run_result = Err(e);
                    break 'phases;
                }
            }
            run_result = Ok(total_executed);

            // After kernel HLT starts (~130M), check VGA for login prompt
            if total_executed >= 130_000_000 {
                // Print VGA preview: every 1M before login, every 50M after
                let print_interval = if logged_in { 50_000_000 } else { 1_000_000 };
                if total_executed % print_interval == 0 || !logged_in {
                    let rows = emu.vga_all_text_rows();
                    let has_login = rows.iter().any(|r| r.contains("login:"));

                    let non_empty: Vec<&str> = rows
                        .iter()
                        .map(|r| r.trim())
                        .filter(|l| !l.is_empty())
                        .collect();
                    let preview_str = if non_empty.is_empty() {
                        "(blank/empty)".to_string()
                    } else {
                        non_empty[non_empty.len().saturating_sub(3)..].join(" | ")
                    };
                    println!(
                        "[{}M] VGA: {}{}",
                        total_executed / 1_000_000,
                        preview_str,
                        if has_login && !logged_in { " *** LOGIN DETECTED ***" } else { "" }
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
                    }
                }

                // Keep-alive: reset console blank timer
                for &sc in KEEP_ALIVE_SCANCODE {
                    emu.send_scancode(sc);
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

    // In headless mode (and even with GUI), also print any remaining Bochs-style
    // debug-port output that might not have been drained during execution.
    let e9 = emu.devices.take_port_e9_output();
    if !e9.is_empty() {
        println!();
        println!("===== BOCHS DEBUG PORT OUTPUT (0xE9/0x402/0x403/0x500) =====");
        print!("{}", String::from_utf8_lossy(&e9));
    }

    println!("╠════════════════════════════════════════════════════════════╣");
    println!(
        "║  Final RIP:   {:#018x}  CS={:04x} mode={}          ║",
        emu.cpu.rip(),
        emu.cpu.get_cs_selector(),
        emu.cpu.get_cpu_mode()
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

    // In headless mode, dump the current VGA text screen
    if headless {
        println!("\n===== VGA TEXT DUMP =====");
        println!("{}", emu.vga_text_dump());
    }

    Ok(())
}
