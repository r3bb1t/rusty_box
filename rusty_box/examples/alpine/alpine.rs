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

fn read_guest_u32(emu: &mut Emulator<'_, Corei7SkylakeX>, addr: u64) -> Option<u32> {
    let mut bytes = [0; 4];
    emu.mem_read(addr, &mut bytes).ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn read_guest_u16(emu: &mut Emulator<'_, Corei7SkylakeX>, addr: u64) -> Option<u16> {
    let mut bytes = [0; 2];
    emu.mem_read(addr, &mut bytes).ok()?;
    Some(u16::from_le_bytes(bytes))
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
        workspace_root.join("cpp_orig/bochs/bochs/bios/BIOS-bochs-latest"),
        workspace_root.join("cpp_orig/bochs/bochs/bios/BIOS-bochs-legacy"),
        workspace_root.join("BIOS-bochs-latest"),
        std::path::PathBuf::from("BIOS-bochs-latest"),
    ];

    let vga_bios_paths = [
        workspace_root.join("binaries/bios/VGABIOS-lgpl-latest.bin"),
        workspace_root.join("cpp_orig/bochs/bochs/bios/VGABIOS-lgpl-latest.bin"),
        workspace_root.join("VGABIOS-lgpl-latest.bin"),
        std::path::PathBuf::from("VGABIOS-lgpl-latest.bin"),
    ];

    // Alpine disk/ISO image — from env var or search common locations
    let disk_path = if let Ok(path) = std::env::var("ALPINE_DISK") {
        let p = std::path::PathBuf::from(&path);
        if !p.exists() {
            eprintln!("ERROR: ALPINE_DISK={} does not exist", path);
            std::process::exit(1);
        }
        p
    } else {
        // Search common locations (raw images and ISOs)
        let search_paths = [
            workspace_root.join("alpine/alpine.img"),
            workspace_root.join("alpine/alpine-virt.img"),
            workspace_root.join("alpine.img"),
            std::path::PathBuf::from("alpine.img"),
            std::path::PathBuf::from("alpine/alpine.img"),
        ];
        let iso_search_paths: Vec<_> = std::fs::read_dir(&workspace_root)
            .ok()
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.extension()
                            .map(|ext| ext.to_str() == Some("iso"))
                            .unwrap_or(false)
                            && p.file_name()
                                .map(|n| {
                                    n.to_str()
                                        .map(|s| s.to_lowercase().contains("alpine"))
                                        .unwrap_or(false)
                                })
                                .unwrap_or(false)
                    })
                    .collect()
            })
            .unwrap_or_default();

        if let Some(p) = search_paths.iter().find(|p| p.exists()) {
            p.clone()
        } else if let Some(p) = iso_search_paths.first() {
            p.clone()
        } else {
            eprintln!("ERROR: No Alpine disk/ISO image found.");
            eprintln!();
            eprintln!("Options:");
            eprintln!("  1. Download Alpine ISO: https://alpinelinux.org/downloads/");
            eprintln!("     Place the .iso file in the project root.");
            eprintln!("  2. Set ALPINE_DISK=/path/to/alpine.iso");
            eprintln!();
            std::process::exit(1);
        }
    };

    // Detect if the image is an ISO (CD-ROM) or raw disk
    let is_iso = disk_path
        .extension()
        .map(|ext| ext.to_str() == Some("iso"))
        .unwrap_or(false);

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
        println!(
            "  VGA BIOS loaded: {} bytes ({})",
            vga.len(),
            vga_path.display()
        );
    } else {
        println!("  VGA BIOS not found (optional)");
    }

    // Disk image info
    let disk_meta = std::fs::metadata(&disk_path).expect("Cannot read disk image metadata");
    let disk_size = disk_meta.len();
    println!(
        "  {} image: {} ({} MB)",
        if is_iso { "ISO" } else { "Disk" },
        disk_path.display(),
        disk_size / (1024 * 1024)
    );

    // Determine disk geometry (only needed for raw disk images)
    let (cylinders, heads, spt) = if is_iso {
        (0u16, 0u8, 0u8) // CD-ROM doesn't use CHS
    } else if let Some(chs_str) = std::env::var("ALPINE_CHS").ok() {
        parse_chs(&chs_str).unwrap_or_else(|| {
            eprintln!("ERROR: Invalid ALPINE_CHS format. Use C,H,S (e.g., 1024,16,63)");
            std::process::exit(1);
        })
    } else {
        auto_detect_geometry(disk_size)
    };
    if is_iso {
        println!("  Boot mode: CD-ROM (El Torito)");
    } else {
        println!("  Disk geometry: CHS={}/{}/{}", cylinders, heads, spt);
    }
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
        ips: 300_000_000,
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

    let disk_path_str = disk_path.to_string_lossy().to_string();

    if is_iso {
        // CD-ROM boot: attach as ATAPI device on channel 1, master (drive 0)
        // Matches Bochs config: ata1-master: type=cdrom (secondary channel, 0x170, IRQ 15)
        emu.configure_boot_sequence(3, 0, 0); // 3 = cdrom first
        emu.attach_cdrom(1, 0, &disk_path_str)
            .expect("Failed to attach CD-ROM image");
        println!("  CD-ROM attached on ata1-master: {}", disk_path_str);
    } else {
        // Hard disk boot: configure CHS geometry and attach
        emu.configure_disk_geometry_in_cmos(0, cylinders, heads, spt);
        emu.configure_boot_sequence(2, 0, 0); // 2 = hard disk first
        emu.attach_disk(0, 0, &disk_path_str, cylinders.into(), heads, spt)
            .expect("Failed to attach disk image");
        println!("  Disk attached: CHS={}/{}/{}", cylinders, heads, spt);
    }

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
    println!(
        "║  CS:IP  = F000:{:04X}                                      ║",
        emu.rip()
    );
    println!(
        "║  A20    = {}                                         ║",
        if emu.pc_system.get_enable_a20() {
            "enabled "
        } else {
            "disabled"
        }
    );
    println!(
        "║  Memory = {} MB                                         ║",
        ram_mb
    );
    if is_iso {
        println!("║  Boot   = CD-ROM (El Torito)                             ║");
    } else {
        println!(
            "║  Disk   = {} cyl x {} heads x {} spt                    ║",
            cylinders, heads, spt
        );
    }
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
                                                            // PS/2 Set 2 scancode for Enter key
    const ENTER_SCANCODE: &[u8] = &[0x5A, 0xF0, 0x5A]; // Enter make + break

    let result = if headless {
        let mut total_executed: u64 = 0;
        let mut run_result: Result<u64> = Ok(0);
        let mut logged_in = false;
        let mut enter_injected = false;

        const PHASE_SIZE: u64 = 100_000;
        let mut last_rip: u64 = 0;
        let mut same_rip_count: u32 = 0;
        let mut phase_num: u64 = 0;

        // Set address hit watches for __intcall debugging
        // 0x100006 = __intcall RM entry (INC [0x3AD8], allocates stack frame)
        // 0x106AE3 = __intcall PM wrapper (PIC call via GOT)
        // 0x106AFB = CALL [EBX+8] inside __intcall wrapper
        // 0x89A0   = PM→RM trampoline entry
        // 0x84A7   = __intcall RM cleanup handler
        // 0x8662   = __farcall RM entry
        // 0x100C40 = idle loop (timer check)
        // 0x89C8   = PM→RM switch (JMP FAR to 16-bit CS)
        // Fine-grained watches inside __intcall to find where execution diverges
        // 0x100006: MOVZX EAX, byte [ESP+4]     (5 bytes)
        // 0x10000B: MOV EAX, [EAX*4]             (7 bytes)
        // 0x100012: PUSHFD                        (1 byte)
        // 0x100013: INC dword [0x3AD8]            (6 bytes)
        // 0x100019: PUSH EBX                      (1 byte)
        unsafe { emu.cpu_mut_unchecked() }.set_addr_hit_watches(&[
            (0x00100006, 0), // __intcall: MOVZX EAX, byte [ESP+4]
            (0x0010000B, 0), // __intcall: MOV EAX, [EAX*4]
            (0x00100012, 0), // __intcall: PUSHFD
            (0x00100013, 0), // __intcall: INC dword [0x3AD8]
            (0x00100019, 0), // __intcall: PUSH EBX
            (0x0000E82E, 0), // BIOS INT 16h handler
            (0x000084B4, 0), // RM→PM transition
            (0x000089C8, 0), // PM→RM switch JMP FAR
        ]);

        'phases: loop {
            if total_executed >= max_instructions {
                break 'phases;
            }
            let run_for = PHASE_SIZE.min(max_instructions - total_executed);
            let phase_start = Instant::now();
            match emu.run_interactive(run_for) {
                Ok(n) => {
                    let phase_elapsed = phase_start.elapsed();
                    if phase_elapsed.as_secs() >= 5 || n == 0 {
                        eprintln!(
                            "[PHASE {}] returned {} instr in {:?}, total={}",
                            phase_num,
                            n,
                            phase_elapsed,
                            total_executed + n,
                        );
                    }
                    total_executed += n;
                }
                Err(e) => {
                    run_result = Err(e);
                    break 'phases;
                }
            }
            run_result = Ok(total_executed);
            phase_num += 1;

            // Check if CPU entered shutdown (triple fault)
            if emu.cpu().is_in_shutdown() {
                println!(
                    "CPU triple-fault shutdown at {}M instructions",
                    total_executed / 1_000_000
                );
                break 'phases;
            }

            // No per-phase diagnostics — use CPU's built-in INT tracking instead

            // Progress and stuck-loop detection
            let rip = emu.cpu().rip();
            let mode = emu.get_cpu_mode_str();
            let cs = emu.cpu().get_cs_selector();
            let (ata_reads, _) = emu.device_manager.ata_io_counts();
            println!(
                "[{:>4}M] RIP={:#010x} CS={:04x} mode={:<11} ATA_rd={} EAX={:08x} ECX={:08x}",
                total_executed / 1_000_000,
                rip,
                cs,
                mode,
                ata_reads,
                emu.cpu().eax(),
                emu.cpu().ecx()
            );

            // Optional diagnostics read only the small guest ranges they print;
            // swapped guest RAM is never exposed as a flat host slice.
            #[cfg(debug_assertions)]
            if std::env::var("RUSTY_BOX_DEBUG").is_ok()
                && (2_800_000..3_100_000).contains(&total_executed)
            {
                let conv_mem = read_guest_u16(&mut emu, 0x0413).unwrap_or(0xDEAD);
                let bda_ticks = read_guest_u32(&mut emu, 0x046C).unwrap_or(0xDEAD_DEAD);
                let boot_info = emu.peek_ram_at(0x7C00, 32);
                println!(
                    "\n===== BOOT DIAGNOSTICS =====\n  BDA conv_mem_kb={conv_mem} BDA_ticks={bda_ticks:#010x}\n  boot bytes: {boot_info:02x?}"
                );
            }

            // Detect stuck at same RIP
            if rip == last_rip {
                same_rip_count += 1;
                if same_rip_count >= 3 {
                    println!(
                        "*** STUCK at RIP={:#010x} for {}x phases! ***",
                        rip, same_rip_count
                    );
                    println!(
                        "    EBX={:08x} EDX={:08x} ESP={:08x} EBP={:08x} ESI={:08x} EDI={:08x}",
                        emu.cpu().ebx(),
                        emu.cpu().edx(),
                        emu.cpu().esp(),
                        emu.cpu().ebp(),
                        emu.cpu().esi(),
                        emu.cpu().edi()
                    );
                    // Read a requested diagnostic window at RIP. This is only
                    // an approximate physical address outside paged modes.
                    let instr_bytes = emu.peek_ram_at(rip as usize, 16);
                    if !instr_bytes.is_empty() {
                        println!(
                            "    Instruction bytes at phys {:#x}: {:02x?}",
                            rip, instr_bytes
                        );
                    }
                }
            } else {
                same_rip_count = 0;
            }
            last_rip = rip;

            // Inject Enter at boot prompt to unblock ISOLINUX idle loop
            // ISOLINUX boot prompt appears at ~17M instructions; inject after that
            if total_executed >= 18_000_000 && !enter_injected {
                println!(
                    "[{}M] Injecting Enter key to boot prompt",
                    total_executed / 1_000_000
                );
                for &sc in ENTER_SCANCODE {
                    emu.send_scancode(sc);
                }
                enter_injected = true;
            }

            // Dump debug port output periodically to see ISOLINUX messages
            if phase_num % 10 == 0 {
                let e9 = emu.devices.take_port_e9_output();
                if !e9.is_empty() {
                    let text = String::from_utf8_lossy(&e9);
                    for line in text.lines().take(5) {
                        println!("[PORT] {}", line);
                    }
                    if text.lines().count() > 5 {
                        println!("[PORT] ... ({} total bytes)", e9.len());
                    }
                }
            }

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
                    if has_login {
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
            println!(
                "║  Time:         {:>12.3} sec                      ║",
                elapsed.as_secs_f64()
            );
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
        println!("===== BOCHS DEBUG PORT OUTPUT (0xE9) =====");
        print!("{}", String::from_utf8_lossy(&e9));
    }

    println!("╠════════════════════════════════════════════════════════════╣");
    println!(
        "║  Final RIP:   {:#018x}  CS={:04x} mode={:<11} ║",
        emu.cpu().rip(),
        emu.cpu().get_cs_selector(),
        emu.get_cpu_mode_str()
    );
    println!(
        "║  EAX={:08x} EBX={:08x} ECX={:08x} EDX={:08x} ║",
        emu.cpu().eax(),
        emu.cpu().ebx(),
        emu.cpu().ecx(),
        emu.cpu().edx()
    );
    println!(
        "║  ESP={:08x} EBP={:08x} ESI={:08x} EDI={:08x} ║",
        emu.cpu().esp(),
        emu.cpu().ebp(),
        emu.cpu().esi(),
        emu.cpu().edi()
    );
    println!("╚════════════════════════════════════════════════════════════╝");

    // Cleanup
    if let Some(ref mut gui) = emu.gui_mut() {
        gui.exit();
    }

    // Headless diagnostics
    if headless {
        // Keep headless output useful without depending on a contiguous host
        // RAM mapping. Every guest inspection below is a requested-size copy.
        println!("\n===== ADDRESS HIT COUNTERS =====");
        let labels = [
            "MOVZX EAX,[ESP+4]",
            "MOV EAX,[EAX*4]",
            "PUSHFD",
            "INC [0x3AD8]",
            "PUSH EBX",
            "BIOS INT 16h",
            "RM→PM transition",
            "PM→RM switch JMP FAR",
        ];
        for (i, (addr, count)) in emu.cpu().get_addr_hits().iter().enumerate() {
            if *addr != 0 {
                println!(
                    "  {addr:#010x} {:30} = {count} hits",
                    labels.get(i).unwrap_or(&"???")
                );
            }
        }

        let timer_cnt = read_guest_u32(&mut emu, 0x3AD8).unwrap_or(0);
        let bda_ticks = read_guest_u32(&mut emu, 0x046C).unwrap_or(0);
        let boot_info = emu.peek_ram_at(0x7C00, 32);
        let rip_bytes = emu.peek_ram_at(emu.cpu().rip() as usize, 16);
        println!(
            "\n===== BLOCK-AWARE GUEST DIAGNOSTICS =====\n  timer_cnt[0x3AD8]={timer_cnt}\n  BDA ticks[0x046C]={bda_ticks}\n  boot bytes[0x7c00]={boot_info:02x?}\n  RIP bytes={rip_bytes:02x?}"
        );

        println!("\n===== SERIAL (COM1) OUTPUT =====");
        let serial_bytes: Vec<u8> = emu.device_manager.drain_serial_tx(0).collect();
        if serial_bytes.is_empty() {
            println!("  (no serial output)");
        } else {
            let text = String::from_utf8_lossy(&serial_bytes);
            for line in text.lines().take(40) {
                println!("  {line}");
            }
        }
        println!("===== END SERIAL OUTPUT =====");
    }

    Ok(())
}
