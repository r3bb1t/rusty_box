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
    emulator::{AtaSlot, BootDevice, BootOrder, DiskGeometry, EmulatorConfig, Ips, MemorySize, MachineBuilder},
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
    // A full filter directive, not a bare level. Parsing `RUST_LOG` as a
    // `Level` silently swallows anything targeted — `RUST_LOG=irq=debug` fails
    // to parse and falls back to WARN, so a trace someone added to chase a bug
    // simply never appears and its absence reads as evidence. Targets are the
    // whole point of having them.
    tracing_subscriber::fmt()
        .without_time()
        .with_target(true)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
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
        workspace_root.join("cpp_orig/bochs/bochs/bios/BIOS-bochs-latest"),
        workspace_root.join("cpp_orig/bochs/bochs/bios/BIOS-bochs-legacy"),
        workspace_root.join("cpp_orig/bochs/bochs/bios/bios.bin-1.13.0"),
        // Fallbacks (if user has BIOS copied elsewhere)
        workspace_root.join("BIOS-bochs-latest"),
        workspace_root.join("BIOS-bochs-legacy"),
        workspace_root.join("bios.bin-1.13.0"),
        workspace_root.join("../cpp_orig/bochs/bochs/bios/BIOS-bochs-latest"),
        workspace_root.join("../cpp_orig/bochs/bochs/bios/BIOS-bochs-legacy"),
        workspace_root.join("../BIOS-bochs-latest"),
        std::path::PathBuf::from("BIOS-bochs-latest"),
        std::path::PathBuf::from("../cpp_orig/bochs/bochs/bios/BIOS-bochs-latest"),
    ];

    let vga_bios_paths = [
        // Prefer user-provided binaries folder (if present)
        workspace_root.join("binaries/bios/VGABIOS-lgpl-latest.bin"),
        workspace_root.join("binaries/bios/VGABIOS-lgpl-latest-cirrus.bin"),
        workspace_root.join("binaries/bios/VGABIOS-lgpl-latest-debug.bin"),
        // Mirrored Bochs BIOS directory (upstream keeps LGPL VGA BIOSes in a
        // VGABIOS-lgpl/ subdirectory; older snapshots had them alongside the
        // system BIOS, so both layouts are probed)
        workspace_root.join("cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin"),
        workspace_root.join("cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest-cirrus.bin"),
        workspace_root.join("cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest-debug.bin"),
        workspace_root.join("cpp_orig/bochs/bochs/bios/VGABIOS-lgpl-latest.bin"),
        workspace_root.join("cpp_orig/bochs/bochs/bios/VGABIOS-lgpl-latest-cirrus.bin"),
        workspace_root.join("cpp_orig/bochs/bochs/bios/VGABIOS-lgpl-latest-debug.bin"),
        // Fallbacks
        workspace_root.join("VGABIOS-lgpl-latest.bin"),
        workspace_root.join("../cpp_orig/bochs/bochs/bios/VGABIOS-lgpl-latest.bin"),
        workspace_root.join("../VGABIOS-lgpl-latest.bin"),
        std::path::PathBuf::from("VGABIOS-lgpl-latest.bin"),
        std::path::PathBuf::from("../cpp_orig/bochs/bochs/bios/VGABIOS-lgpl-latest.bin"),
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
        memory: MemorySize::bytes(32 * 1024 * 1024), // 32 MB
        memory_block_size: 128 * 1024,
        ips: Ips::new(300_000_000),
        pci_enabled: true,
        ..Default::default()
    };

    tracing::info!(
        "Creating emulator: {} MB RAM, {} MIPS",
        config.memory.guest_bytes() / (1024 * 1024),
        config.ips.per_second() / 1_000_000,
    );

    // =========================================================================
    // Assemble the machine
    // =========================================================================
    // The builder performs the whole Bochs main.cc bring-up: memory, BIOS,
    // CPUs, devices, CMOS, media, GUI, reset, timers — in that order.
    tracing::info!("Assembling machine...");
    let disk_path_str = disk_path.to_string_lossy().to_string();
    let mut builder = MachineBuilder::new(config)
        .bios(&bios_data)
        .boot_order(BootOrder::just(BootDevice::Disk))
        .disk_file(
            AtaSlot::PRIMARY_MASTER,
            &disk_path_str,
            DiskGeometry::new(DLX_CYLINDERS.into(), DLX_HEADS, DLX_SPT),
        );

    if headless {
        builder = builder.gui(NoGui::new());
        println!("(headless) RUSTY_BOX_HEADLESS=1: terminal repaint disabled");
    } else {
        builder = builder.gui(TermGui::new());
    }

    if let Some((_vga_path, ref vga_data)) = vga_bios {
        builder = builder.vga_bios(vga_data);
    }

    let mut emu = builder.build()?;
    tracing::info!(
        "✓ Machine ready: BIOS {} KB, disk CHS={}/{}/{}",
        bios_data.len() / 1024,
        DLX_CYLINDERS,
        DLX_HEADS,
        DLX_SPT
    );

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
        if emu.get_enable_a20() {
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
        println!("║           BIOS OUTPUT (port 0xE9 debug console)            ║");
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
    // The default is the budget xtask's DLX boot gate runs this example with
    // (`DLX_BOOT_BUDGET` in xtask/src/ci.rs); headless, the gate requires it to
    // reach `dlx login:` and print `*** LOGIN DETECTED ***`.
    // Override with MAX_INSTRUCTIONS env var:
    //   MAX_INSTRUCTIONS=132865710   → stop at first kernel HLT (ATA/IRQ diagnostics)
    let max_instructions: u64 = std::env::var("MAX_INSTRUCTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(450_000_000);

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
    // Enter make+break: boots the default image at the `LILO boot:` prompt.
    // The DLX image's LILO is configured with `prompt` and no timeout, so it
    // waits for a keypress indefinitely.
    const LILO_ENTER_SCANCODES: &[u8] = &[0x5A, 0xF0, 0x5A];

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
        let mut lilo_boot_entered = false;
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
                    // The whole aperture, not just the displayed page: this
                    // watches for prompts the CRTC start address may have
                    // scrolled away from.
                    let rows = emu.display().dump_text_aperture();
                    let has_login = rows.iter().any(|r| r.contains("login:"));

                    // Boot the default image at the LILO prompt (waits forever
                    // otherwise — DLX's LILO has `prompt` with no timeout).
                    let at_lilo = rows.iter().any(|r| r.contains("LILO boot:"));
                    if at_lilo && !lilo_boot_entered {
                        println!(
                            "(headless) LILO prompt detected — injecting Enter at {}M instructions",
                            total_executed / 1_000_000
                        );
                        let _sent = emu.keyboard().scancodes(LILO_ENTER_SCANCODES);
                        lilo_boot_entered = true;
                    }

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
                        if has_login && !logged_in {
                            " *** LOGIN DETECTED ***"
                        } else {
                            ""
                        }
                    );

                    if has_login && !logged_in {
                        println!(
                            "(headless) Injecting 'root\\n' at {}M instructions",
                            total_executed / 1_000_000
                        );
                        let _sent = emu.keyboard().scancodes(LOGIN_SCANCODES);
                        logged_in = true;
                    }
                }

                // Keep-alive: reset console blank timer
                let _sent = emu.keyboard().scancodes(KEEP_ALIVE_SCANCODE);
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
    let e9: Vec<u8> = emu.debug_port().take_output().collect();
    if !e9.is_empty() {
        println!();
        println!("===== BOCHS DEBUG PORT OUTPUT (0xE9) =====");
        print!("{}", String::from_utf8_lossy(&e9));
    }

    println!("╠════════════════════════════════════════════════════════════╣");
    println!(
        "║  Final RIP:   {:#018x}  CS={:04x} mode={}          ║",
        emu.cpu().rip(),
        emu.cpu().get_cs_selector(),
        emu.cpu().get_cpu_mode()
    );
    println!(
        "║  EAX={:08x} EBX={:08x} ECX={:08x} EDX={:08x}  ║",
        emu.cpu().eax(),
        emu.cpu().ebx(),
        emu.cpu().ecx(),
        emu.cpu().edx()
    );
    println!(
        "║  ESP={:08x} EBP={:08x} ESI={:08x} EDI={:08x}  ║",
        emu.cpu().esp(),
        emu.cpu().ebp(),
        emu.cpu().esi(),
        emu.cpu().edi()
    );
    println!("╚════════════════════════════════════════════════════════════╝");

    // Cleanup: restore terminal if GUI was used
    if let Some(ref mut gui) = emu.gui_mut() {
        gui.exit();
    }

    // In headless mode, dump the current VGA text screen
    if headless {
        println!("\n===== VGA TEXT DUMP =====");
        if let Some(text) = emu.display().text() {
            println!("{}", text.to_text());
        }
    }

    Ok(())
}
