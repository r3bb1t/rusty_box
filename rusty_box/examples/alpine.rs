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
        emu.attach_disk(0, 0, &disk_path_str, cylinders, heads, spt)
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
        println!(
            "║  Boot   = CD-ROM (El Torito)                             ║"
        );
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
        let mut bda_dumped = false;
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
        emu.cpu.set_addr_hit_watches(&[
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
                            phase_num, n, phase_elapsed, total_executed + n,
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
            if emu.cpu.is_in_shutdown() {
                println!("CPU triple-fault shutdown at {}M instructions", total_executed / 1_000_000);
                break 'phases;
            }

            // No per-phase diagnostics — use CPU's built-in INT tracking instead

            // Progress and stuck-loop detection
            let rip = emu.cpu.rip();
            let mode = emu.get_cpu_mode_str();
            let cs = emu.cpu.get_cs_selector();
            let (ata_reads, _) = emu.device_manager.ata_io_counts();
            println!(
                "[{:>4}M] RIP={:#010x} CS={:04x} mode={:<11} ATA_rd={} EAX={:08x} ECX={:08x}",
                total_executed / 1_000_000,
                rip, cs, mode, ata_reads,
                emu.cpu.eax(), emu.cpu.ecx()
            );

            // One-time dump: BDA and IVT at ~2M (before SYSLINUX init)
            if total_executed >= 2_800_000 && !bda_dumped {
                bda_dumped = true;
                let (rp, rl) = emu.memory.get_ram_base_ptr();
                let rd16 = |addr: usize| -> u16 {
                    if addr + 1 < rl { unsafe { (rp.add(addr) as *const u16).read_unaligned() } } else { 0xDEAD }
                };
                let rd32 = |addr: usize| -> u32 {
                    if addr + 3 < rl { unsafe { (rp.add(addr) as *const u32).read_unaligned() } } else { 0xDEAD }
                };
                println!("\n===== BDA / IVT CHECK =====");
                println!("  BDA[0x0413] conv_mem_kb = {} (0x{:04x})", rd16(0x0413), rd16(0x0413));
                println!("  BDA[0x0415] ext_mem_kb  = {} (0x{:04x})", rd16(0x0415), rd16(0x0415));
                println!("  IVT[0x12]  = {:04x}:{:04x}", rd16(0x004A), rd16(0x0048));
                println!("  IVT[0x13]  = {:04x}:{:04x}", rd16(0x004E), rd16(0x004C));
                println!("  IVT[0x10]  = {:04x}:{:04x}", rd16(0x0042), rd16(0x0040));
                println!("  IVT[0x15]  = {:04x}:{:04x}", rd16(0x0056), rd16(0x0054));
                println!("  IVT[0x16]  = {:04x}:{:04x}", rd16(0x005A), rd16(0x0058));
                // Check SYSLINUX memory at key addresses
                println!("  [0x100135] code = {:08x} {:08x}", rd32(0x100135), rd32(0x100139));
                // Check what's at EBDA
                let ebda_seg = rd16(0x040E);
                println!("  BDA[0x040E] EBDA seg = {:04x} (phys {:05x})", ebda_seg, (ebda_seg as u32) << 4);
                println!();
            }

            // One-time dump at ~3M instructions
            if total_executed >= 3_000_000 && total_executed < 3_100_000 {
                let (rp, rl) = emu.memory.get_ram_base_ptr();
                let rd = |addr: usize| -> u32 {
                    if addr + 3 < rl { unsafe { (rp.add(addr) as *const u32).read_unaligned() } } else { 0xDEAD }
                };

                // Dump boot info table from isolinux.bin stub at 0x7C00
                println!("\n===== BOOT INFO TABLE (isolinux.bin @ 0x7C00) =====");
                println!("  bi_pvd    @ 0x7C08 = {:08x} (should be 0x10 = sector 16)", rd(0x7C08));
                println!("  bi_file   @ 0x7C0C = {:08x} (boot file LBA)", rd(0x7C0C));
                println!("  bi_length @ 0x7C10 = {:08x} (boot file length)", rd(0x7C10));
                println!("  bi_csum   @ 0x7C14 = {:08x} (checksum)", rd(0x7C14));
                // Also dump raw bytes 0x7C00-0x7C20
                print!("  0x7C00:");
                for j in 0..32usize { print!(" {:02x}", unsafe { *rp.add(0x7C00 + j) }); }
                println!();
                // Also check what the STUB stores in its data area (0x3000+)
                println!("  BootInfoTable ptr (from stub data):");
                println!("  [0x3004] = {:08x}", rd(0x3004));
                println!("  [0x3008] = {:08x}", rd(0x3008));
                println!("  [0x300C] = {:08x}", rd(0x300C));
                println!("  [0x3010] = {:08x}", rd(0x3010));

                // Dump init_func table at 0x110DC0 (first 10 entries, stride 0x38)
                println!("\n===== INIT FUNC TABLE @ 0x110DC0 (stride=0x38) =====");
                for i in 0..10u32 {
                    let base = 0x110DC0 + (i * 0x38) as usize;
                    let dw0 = rd(base);
                    let dw1 = rd(base + 4);
                    let dw2 = rd(base + 8);
                    let dw3 = rd(base + 12);
                    if dw0 != 0 || dw1 != 0 || i < 3 {
                        println!("  [{:2}] @{:#08x}: {:08x} {:08x} {:08x} {:08x}", i, base, dw0, dw1, dw2, dw3);
                    }
                }

                // Search for constructor-like arrays in 0x100000-0x10A800
                // These would be arrays of 4-byte pointers in 0x100000-0x110000 range
                println!("\n===== SEARCHING FOR CONSTRUCTOR ARRAYS =====");
                let mut found = 0;
                for addr in (0x100000..0x10A800).step_by(4) {
                    let val = rd(addr);
                    // Look for sequences of 3+ consecutive function pointers
                    if val >= 0x100000 && val < 0x112000 {
                        let v1 = rd(addr + 4);
                        let v2 = rd(addr + 8);
                        if v1 >= 0x100000 && v1 < 0x112000 && v2 >= 0x100000 && v2 < 0x112000 {
                            if found < 10 {
                                println!("  Candidate @{:#08x}: {:08x} {:08x} {:08x} {:08x}",
                                    addr, val, v1, v2, rd(addr + 12));
                            }
                            found += 1;
                        }
                    }
                }
                println!("  Total candidate sequences: {}", found);

                // Dump malloc/allocator state at 0x10ECF0 (base address from trace)
                println!("\n===== MALLOC ALLOCATOR STATE =====");
                println!("  Allocator base area (0x10ECF0-0x10ED3F):");
                for row in 0..5usize {
                    let base = 0x10ECF0 + row * 16;
                    print!("  {:#08x}:", base);
                    for j in 0..4usize { print!(" {:08x}", rd(base + j * 4)); }
                    println!();
                }
                println!("  Free list area (0x10F2C0-0x10F31F):");
                for row in 0..4usize {
                    let base = 0x10F2C0 + row * 16;
                    print!("  {:#08x}:", base);
                    for j in 0..4usize { print!(" {:08x}", rd(base + j * 4)); }
                    println!();
                }
                // Dump __com32 structure (at known SYSLINUX addresses)
                println!("  __com32 struct candidate at 0x10F180:");
                for row in 0..4usize {
                    let base = 0x10F180 + row * 16;
                    print!("  {:#08x}:", base);
                    for j in 0..4usize { print!(" {:08x}", rd(base + j * 4)); }
                    println!();
                }

                // Dump the linked list node at 0x22060 (from init_func entry[1])
                println!("\n===== INIT_FUNC LIST NODE @ 0x22060 =====");
                for row in 0..4usize {
                    let base = 0x22060 + row * 16;
                    if base + 15 < rl {
                        print!("  {:#08x}:", base);
                        for j in 0..4usize {
                            let val = rd(base + j * 4);
                            print!(" {:08x}", val);
                        }
                        println!();
                    }
                }

                // Dump idle loop code at key addresses
                println!("\n===== IDLE LOOP CODE =====");
                for &(name, addr) in &[
                    ("0x100c40 timer_read", 0x100c40usize),
                    ("0x10418b cond_check", 0x10418busize),
                    ("0x104171 timer_thresh", 0x104171usize),
                    ("0x100065 poll_start", 0x100065usize),
                    ("0x100080 poll_mid", 0x100080usize),
                ] {
                    print!("  {} {:#08x}:", name, addr);
                    for j in 0..24usize {
                        if addr + j < rl {
                            print!(" {:02x}", unsafe { *rp.add(addr + j) });
                        }
                    }
                    println!();
                }

                // Dump pending_flag and nearby state
                println!("\n===== KEY STATE VARS =====");
                println!("  pending_flag  @ 0x10F774 = {:08x}", rd(0x10F774));
                println!("  timer_start   @ 0x10F778 = {:08x}", rd(0x10F778));
                println!("  callback_ptr  @ 0x8E84   = {:08x}", rd(0x8E84));
                println!("  poll_flag     @ 0x110D94 = {:08x}", rd(0x110D94));
                println!("  callback_ptr2 @ 0x111064 = {:08x}", rd(0x111064));

                // Dump init_func iterator code bytes
                println!("\n===== INIT_FUNC CODE @ 0x104340-0x104360 =====");
                for row in 0..2usize {
                    let base = 0x104340 + row * 16;
                    if base + 15 < rl {
                        print!("  {:#08x}:", base);
                        for j in 0..16usize {
                            print!(" {:02x}", unsafe { *rp.add(base + j) });
                        }
                        println!();
                    }
                }

                // Dump the constructor candidate array at 0x108CE0
                println!("\n===== CONSTRUCTOR TABLE @ 0x108CE0 =====");
                for row in 0..8usize {
                    let base = 0x108CE0 + row * 16;
                    if base + 15 < rl {
                        print!("  {:#08x}:", base);
                        for j in 0..4usize {
                            let val = rd(base + j * 4);
                            print!(" {:08x}", val);
                        }
                        println!();
                    }
                }

                // Search for references to constructor table in code
                // The constructor table starts at ~0x108CE4 with the first function pointer
                // But the count/header might be at 0x108CE0 (value 0x44=68)
                // A constructor-calling loop would have MOV/LEA reg, 0x108CE4 or 0x108CE0
                println!("\n===== SEARCHING FOR CTOR TABLE REFS IN CODE =====");
                let ctor_addr_bytes: [u8; 4] = (0x108CE4u32).to_le_bytes();
                let ctor_hdr_bytes: [u8; 4] = (0x108CE0u32).to_le_bytes();
                // Also search for the end-of-array marker: after last ptr 0x1054C0,
                // the array terminator pattern (0x00008EBC at 0x108D14)
                let ctor_end_bytes: [u8; 4] = (0x108D14u32).to_le_bytes();
                for (name, target_bytes) in &[
                    ("0x108CE4 (first ctor)", &ctor_addr_bytes),
                    ("0x108CE0 (ctor header)", &ctor_hdr_bytes),
                    ("0x108D14 (ctor end)", &ctor_end_bytes),
                ] {
                    let mut found = 0;
                    for addr in 0x100000..0x10A800usize {
                        if addr + 3 < rl {
                            let b = unsafe { core::slice::from_raw_parts(rp.add(addr), 4) };
                            if b == *target_bytes {
                                if found < 5 {
                                    // Show context: 8 bytes before and after
                                    print!("  {} ref at {:#08x}: ", name, addr);
                                    let start = if addr >= 8 { addr - 8 } else { 0 };
                                    for j in start..addr+8 {
                                        if j < rl { print!("{:02x}", unsafe { *rp.add(j) }); }
                                        if j == addr - 1 { print!("["); }
                                        if j == addr + 3 { print!("]"); }
                                    }
                                    println!();
                                }
                                found += 1;
                            }
                        }
                    }
                    println!("  {} total refs: {}", name, found);
                }

                // Also dump memory around the init_func table base to see what's there
                println!("\n===== MEMORY 0x110D80-0x110E40 =====");
                for row in 0..12usize {
                    let base = 0x110D80 + row * 16;
                    if base + 15 < rl {
                        print!("  {:#08x}:", base);
                        for j in 0..4usize {
                            print!(" {:08x}", rd(base + j * 4));
                        }
                        println!();
                    }
                }
            }

            // Detect stuck at same RIP
            if rip == last_rip {
                same_rip_count += 1;
                if same_rip_count >= 3 {
                    println!("*** STUCK at RIP={:#010x} for {}x phases! ***", rip, same_rip_count);
                    println!("    EBX={:08x} EDX={:08x} ESP={:08x} EBP={:08x} ESI={:08x} EDI={:08x}",
                        emu.cpu.ebx(), emu.cpu.edx(), emu.cpu.esp(),
                        emu.cpu.ebp(), emu.cpu.esi(), emu.cpu.edi());
                    // Read a few bytes at RIP to show instruction bytes
                    let (ptr, len) = emu.memory.get_raw_memory_ptr();
                    let rip_phys = rip as usize; // NOTE: approximate — works if paging identity-maps or we're in real/PM without paging offset
                    if rip_phys < len {
                        let end = (rip_phys + 16).min(len);
                        let instr_bytes = unsafe { core::slice::from_raw_parts(ptr.add(rip_phys), end - rip_phys) };
                        println!("    Instruction bytes at phys {:#x}: {:02x?}", rip_phys, instr_bytes);
                    }
                }
            } else {
                same_rip_count = 0;
            }
            last_rip = rip;

            // Inject Enter at boot prompt to unblock ISOLINUX idle loop
            // ISOLINUX boot prompt appears at ~17M instructions; inject after that
            if total_executed >= 18_000_000 && !enter_injected {
                println!("[{}M] Injecting Enter key to boot prompt", total_executed / 1_000_000);
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
        println!("===== BOCHS DEBUG PORT OUTPUT (0xE9/0x402/0x403/0x500) =====");
        print!("{}", String::from_utf8_lossy(&e9));
    }

    println!("╠════════════════════════════════════════════════════════════╣");
    println!(
        "║  Final RIP:   {:#018x}  CS={:04x} mode={:<11} ║",
        emu.cpu.rip(),
        emu.cpu.get_cs_selector(),
        emu.get_cpu_mode_str()
    );
    println!(
        "║  EAX={:08x} EBX={:08x} ECX={:08x} EDX={:08x} ║",
        emu.cpu.eax(),
        emu.cpu.ebx(),
        emu.cpu.ecx(),
        emu.cpu.edx()
    );
    println!(
        "║  ESP={:08x} EBP={:08x} ESI={:08x} EDI={:08x} ║",
        emu.cpu.esp(),
        emu.cpu.ebp(),
        emu.cpu.esi(),
        emu.cpu.edi()
    );
    println!("╚════════════════════════════════════════════════════════════╝");

    // Cleanup
    if let Some(ref mut gui) = emu.gui_mut() {
        gui.exit();
    }

    // Headless diagnostics
    if headless {
        // Address hit counters — shows which code paths were taken
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
        let hits = emu.cpu.get_addr_hits();
        for (i, (addr, count)) in hits.iter().enumerate() {
            if *addr != 0 {
                let label = labels.get(i).unwrap_or(&"???");
                println!("  {:#010x} {:30} = {} hits", addr, label, count);
            }
        }

        // Read timer_cnt at [0x3AD8] — incremented by __intcall INC instruction
        // Also dump 32 bytes around it to check for nearby writes
        {
            let (rp, rl) = emu.memory.get_ram_base_ptr();
            let timer_cnt = if 0x3ADC <= rl {
                unsafe { (rp.add(0x3AD8) as *const u32).read_unaligned() }
            } else { 0 };
            println!("\n===== TIMER_CNT [0x3AD8] = {} =====", timer_cnt);
            // Dump 64 bytes: 0x3AC0 - 0x3AFF
            print!("0x3AC0: ");
            for i in 0x3AC0usize..0x3B00 {
                if i < rl { print!("{:02x} ", unsafe { *rp.add(i) }); }
                if (i + 1) % 16 == 0 && i < 0x3AFF { print!("\n{:#06x}: ", i + 1); }
            }
            println!();
            // Also check via get_raw_memory_ptr (different accessor)
            let (rp2, rl2) = emu.memory.get_raw_memory_ptr();
            if 0x3ADC <= rl2 {
                let val2 = unsafe { (rp2.add(0x3AD8) as *const u32).read_unaligned() };
                println!("get_raw_memory_ptr[0x3AD8] = {} (ptrs same={})", val2, rp == rp2);
            }
        }

        let (ata_reads, ata_writes) = emu.device_manager.ata_io_counts();

        // IVT dump — identify which INT each handler address belongs to
        println!("\n===== IVT ENTRIES (Interrupt Vector Table) =====");
        {
            let (rp, rl) = emu.memory.get_ram_base_ptr();
            let read_ivt = |int_num: u16| -> (u16, u16) {
                let addr = (int_num as usize) * 4;
                if addr + 3 < rl {
                    unsafe {
                        let ip = u16::from_le_bytes([*rp.add(addr), *rp.add(addr + 1)]);
                        let cs = u16::from_le_bytes([*rp.add(addr + 2), *rp.add(addr + 3)]);
                        (cs, ip)
                    }
                } else {
                    (0xFFFF, 0xFFFF)
                }
            };
            let int_names = [
                (0x08, "Timer (IRQ0)"),
                (0x09, "Keyboard (IRQ1)"),
                (0x10, "Video (INT 10h)"),
                (0x11, "Equipment (INT 11h)"),
                (0x12, "Memory Size (INT 12h)"),
                (0x13, "Disk (INT 13h)"),
                (0x15, "System (INT 15h)"),
                (0x16, "Keyboard (INT 16h)"),
                (0x19, "Bootstrap (INT 19h)"),
                (0x1A, "Time (INT 1Ah)"),
                (0x1C, "User Timer (INT 1Ch)"),
            ];
            for (num, name) in &int_names {
                let (cs, ip) = read_ivt(*num);
                println!("  INT {:02x}h {:20} = {:04x}:{:04x}", num, name, cs, ip);
            }
        }

        // Keyboard port 0x60 read diagnostics
        println!("\n===== KEYBOARD PORT 0x60 READS =====");
        let (p60_count, p60_last, kbd_clk, scanning, xlat, outb) = emu.device_manager.kbd_diag();
        println!("port 0x60 read count: {}", p60_count);
        println!("port 0x60 last value: {:#04x}", p60_last);
        println!("kbd_clock_enabled: {}", kbd_clk);
        println!("scanning_enabled: {}", scanning);
        println!("scancodes_translate: {}", xlat);
        println!("outb: {}", outb);

        // Dump BIOS INT 9 handler at F000:E987 (phys 0xFE987)
        println!("\n===== BIOS INT 9 HANDLER (F000:E987) =====");
        {
            let (rp_bios, rl_bios) = emu.memory.get_ram_base_ptr();
            // The BIOS ROM is mapped at 0xF0000-0xFFFFF → last 64KB of 128KB BIOS
            // Physical address 0xFE987 → in the ROM shadow area
            // get_raw_memory_ptr includes vector_offset
            let bios_phys = 0xFE987usize;
            print!("0xFE987: ");
            for i in 0..48usize {
                let addr = bios_phys + i;
                let byte = if addr < rl_bios {
                    unsafe { *rp_bios.add(addr) }
                } else { 0xFF };
                print!("{:02x} ", byte);
                if (i + 1) % 16 == 0 && i < 47 { print!("\n{:#08x}: ", bios_phys + i + 1); }
            }
            println!();
        }

        // Software INT histogram (from CPU's built-in tracking)
        println!("\n===== SOFTWARE INT HISTOGRAM (late, after 2M icount) =====");
        let int_late = emu.cpu.get_soft_int_vectors_late();
        for (i, &c) in int_late.iter().enumerate() {
            if c > 0 { println!("  INT {:02x}h: {} calls", i, c); }
        }
        println!("\n===== SOFTWARE INT HISTOGRAM (all) =====");
        let int_all = emu.cpu.get_soft_int_vectors();
        for (i, &c) in int_all.iter().enumerate() {
            if c > 0 { println!("  INT {:02x}h: {} calls", i, c); }
        }

        // Exception counters
        let exc_counts = emu.cpu.get_exception_diag();
        let exc_names = ["#DE","#DB","NMI","#BP","#OF","#BR","#UD","#NM",
            "#DF","CSO","#TS","#NP","#SS","#GP","#PF","R15",
            "#MF","#AC","#MC","#XM"];
        println!("\n===== EXCEPTIONS =====");
        let mut any_exc = false;
        for (i, &c) in exc_counts.iter().enumerate().take(20) {
            if c > 0 { println!("  {}={}", exc_names[i], c); any_exc = true; }
        }
        if !any_exc { println!("  (none)"); }

        println!("\n===== ATA DIAGNOSTIC =====");
        println!("ATA read_count={}, write_count={}", ata_reads, ata_writes);

        println!("\n===== IRQ DELIVERY CHAIN =====");
        println!(
            "tick() calls:          {}",
            emu.device_manager.diag_tick_count
        );
        println!(
            "PIT fires (check_irq0): {}",
            emu.device_manager.diag_pit_fires
        );
        println!(
            "IRQ0 latched (raise):   {}",
            emu.device_manager.diag_irq0_latched
        );
        println!(
            "iac() calls:            {}",
            emu.device_manager.diag_iac_count
        );
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

        // Exception counters
        let exc_counts = emu.cpu.get_exception_diag();
        let exc_names = ["#DE","#DB","NMI","#BP","#OF","#BR","#UD","#NM",
            "#DF","CSO","#TS","#NP","#SS","#GP","#PF","R15",
            "#MF","#AC","#MC","#XM","#VE","#CP","R22","R23",
            "R24","R25","R26","R27","R28","#SX","R30","R31"];
        print!("Exceptions:             ");
        let mut any_exc = false;
        for (i, &c) in exc_counts.iter().enumerate() {
            if c > 0 {
                print!("{}={} ", exc_names[i], c);
                any_exc = true;
            }
        }
        if !any_exc { print!("(none)"); }
        println!();

        // IaError (decoder/unimplemented opcode) diagnostics
        let (ia_err_count, ia_err_rip) = emu.cpu.get_ia_error_diag();
        if ia_err_count > 0 {
            println!("IaError:                count={} last_rip={:#x}", ia_err_count, ia_err_rip);
        } else {
            println!("IaError:                (none)");
        }

        // IAC (interrupt acknowledge) vector histogram
        let iac_vecs = emu.cpu.get_iac_vectors();
        print!("CPU iac vectors:        ");
        for (v, &count) in iac_vecs.iter().enumerate() {
            if count > 0 {
                print!("0x{:02x}={} ", v, count);
            }
        }
        println!();

        // HAE (handleAsyncEvent) interrupt delivery diagnostics
        let (hae_delivered, hae_if_blocked, hae_no_pic, hae_pic_empty) = emu.cpu.get_hae_intr_diag();
        println!("HAE intr delivered:     {}", hae_delivered);
        println!("HAE intr IF-blocked:    {}", hae_if_blocked);
        println!("HAE intr no-pic:        {}", hae_no_pic);
        println!("HAE intr pic-empty:     {}", hae_pic_empty);

        // inject_external_interrupt diagnostics (emulator-path delivery)
        let (inject_count, inject_vecs) = emu.cpu.get_inject_ext_intr_diag();
        println!("inject_ext_intr calls:  {}", inject_count);
        print!("inject_ext_intr vecs:   ");
        for (v, &count) in inject_vecs.iter().enumerate() {
            if count > 0 {
                print!("0x{:02x}={} ", v, count);
            }
        }
        println!();

        // Software INT (INT nn) vector histogram — shows which BIOS calls ISOLINUX makes
        let soft_int_vecs = emu.cpu.get_soft_int_vectors();
        let soft_int_total: u64 = soft_int_vecs.iter().sum();
        println!("Software INT total:     {}", soft_int_total);
        print!("Software INT vectors:   ");
        let int_names: &[(u8, &str)] = &[
            (0x10, "vid"), (0x11, "equ"), (0x12, "mem"), (0x13, "dsk"),
            (0x14, "ser"), (0x15, "sys"), (0x16, "kbd"), (0x19, "boot"),
            (0x1A, "rtc"), (0x80, "lnx"),
        ];
        for (v, &count) in soft_int_vecs.iter().enumerate() {
            if count > 0 {
                let name = int_names.iter().find(|(n, _)| *n == v as u8).map(|(_, n)| *n).unwrap_or("");
                if !name.is_empty() {
                    print!("0x{:02x}({})={} ", v, name, count);
                } else {
                    print!("0x{:02x}={} ", v, count);
                }
            }
        }
        println!();

        // INT 10h AH subfunction histogram (late calls only)
        let int10h_ah = emu.cpu.get_int10h_ah_hist();
        let int10h_names: &[(u8, &str)] = &[
            (0x00, "SetMode"), (0x01, "SetCursor"), (0x02, "SetPos"),
            (0x03, "GetPos"), (0x05, "SetPage"), (0x06, "ScrollUp"),
            (0x07, "ScrollDn"), (0x08, "ReadCh"), (0x09, "WriteCh"),
            (0x0A, "WriteChN"), (0x0E, "TTY"), (0x0F, "GetMode"),
            (0x12, "AltFunc"), (0x13, "WriteStr"),
        ];
        print!("INT 10h AH (late):      ");
        let mut any_10h = false;
        for (ah, &count) in int10h_ah.iter().enumerate() {
            if count > 0 {
                any_10h = true;
                let name = int10h_names.iter().find(|(n, _)| *n == ah as u8).map(|(_, n)| *n).unwrap_or("?");
                print!("AH={:02x}({})={} ", ah, name, count);
            }
        }
        if !any_10h { print!("(none)"); }
        println!();

        // INT 10h timing
        let (i10_first, i10_last, tty_first, tty_last) = emu.cpu.get_int10h_icount_range();
        println!("INT10h icount range:    first={} last={} (TTY: first={} last={})",
            i10_first, i10_last, tty_first, tty_last);

        // TTY characters written via INT 10h AH=0Eh
        let tty_chars = emu.cpu.get_int10h_tty_chars();
        if !tty_chars.is_empty() {
            let printable: String = tty_chars.iter().map(|&b| {
                if b >= 0x20 && b < 0x7F { b as char } else { '.' }
            }).collect();
            let hex: Vec<String> = tty_chars.iter().take(64).map(|b| format!("{:02x}", b)).collect();
            println!("INT10h TTY chars:       \"{}\"", printable);
            println!("INT10h TTY hex:         {}", hex.join(" "));
        }

        // Late software INT calls (after BIOS POST, icount > 2M) — ISOLINUX BIOS calls
        let soft_int_late = emu.cpu.get_soft_int_vectors_late();
        let late_total: u64 = soft_int_late.iter().sum();
        println!("Late soft INT total:    {} (after BIOS POST)", late_total);
        print!("Late soft INT vectors:  ");
        for (v, &count) in soft_int_late.iter().enumerate() {
            if count > 0 {
                let name = int_names.iter().find(|(n, _)| *n == v as u8).map(|(_, n)| *n).unwrap_or("");
                if !name.is_empty() {
                    print!("0x{:02x}({})={} ", v, name, count);
                } else {
                    print!("0x{:02x}={} ", v, count);
                }
            }
        }
        println!();

        // Read timer value over time to see if it changes
        println!("\n===== TIMER TRACKING =====");
        {
            let (rp, rl) = emu.memory.get_ram_base_ptr();
            let rdw = |addr: usize| -> u32 {
                if addr + 3 < rl { unsafe { (rp.add(addr) as *const u32).read_unaligned() } } else { 0xDEAD }
            };
            let rdw16 = |addr: usize| -> u16 {
                if addr + 1 < rl { unsafe { (rp.add(addr) as *const u16).read_unaligned() } } else { 0xDEAD }
            };
            let timer_val = rdw(0x8EBC);
            let timer_start = rdw(0x0010F778);
            let bda_ticks = rdw(0x046C);
            let pending_flag = rdw(0x0010F774); // 16-bit: CMP WORD [0x10F774], 0
            let callback_ptr = rdw(0x00111064);
            println!("[0x8EBC] timer_val = {:#010x} ({})", timer_val, timer_val);
            println!("[0x10F778] timer_start = {:#010x} ({})", timer_start, timer_start);
            println!("timer diff = {} (need > 3 to exit)", timer_val.wrapping_sub(timer_start));
            println!("[0x046C] BDA ticks = {:#010x} ({})", bda_ticks, bda_ticks);
            println!("[0x10F774] pending_flag = {:#06x} (need != 0 to exit idle)", pending_flag & 0xFFFF);
            println!("[0x111064] callback_ptr = {:#010x}", callback_ptr);

            // BIOS keyboard buffer check
            let kbd_head = rdw16(0x041A) as usize;
            let kbd_tail = rdw16(0x041C) as usize;
            println!("\n===== BIOS KEYBOARD BUFFER =====");
            println!("kbd_head={:#06x} kbd_tail={:#06x} (equal=empty)", kbd_head, kbd_tail);
            if kbd_head != kbd_tail {
                // Buffer has data — dump it
                let mut pos = kbd_head;
                let buf_start = 0x001E; // relative to BDA segment 0x40
                let buf_end = 0x003E;
                while pos != kbd_tail {
                    let scan = rdw16(0x0400 + pos);
                    println!("  kbd[{:#06x}] = {:#06x} (char={:?} scan={:#04x})",
                        pos, scan, (scan & 0xFF) as u8 as char, (scan >> 8) as u8);
                    pos += 2;
                    if pos >= buf_end { pos = buf_start; }
                }
            } else {
                println!("  (empty — keyboard scancode may not have been processed by INT 9)");
            }

            // Dump RM handler at 0x8523 (ISOLINUX IRQ RM handler)
            println!("\n===== RM IRQ HANDLER at 0x8523 =====");
            print!("0x8523: ");
            for i in 0..64usize {
                print!("{:02x} ", unsafe { if 0x8523 + i < rl { *rp.add(0x8523 + i) } else { 0xFF } });
                if (i + 1) % 16 == 0 && i < 63 { print!("\n{:#08x}: ", 0x8523 + i + 1); }
            }
            println!();

            // Dump RM→PM transition code at 0x84EE-0x8540
            println!("\n===== RM/PM TRANSITION CODE at 0x84EE =====");
            print!("0x84EE: ");
            for i in 0..82usize {
                print!("{:02x} ", unsafe { if 0x84EE + i < rl { *rp.add(0x84EE + i) } else { 0xFF } });
                if (i + 1) % 16 == 0 && i < 81 { print!("\n{:#08x}: ", 0x84EE + i + 1); }
            }
            println!();

            // Dump PIC mask registers to check if IRQ0 is masked
            println!("\nPIC state: {}", emu.device_manager.pic_diag());

            // Dump IDT entries
            let idt_base = rdw(0x8E7E) as usize;
            println!("PM IDT base = {:#010x}", idt_base);
            for vec in [0x08, 0x09, 0x20, 0x21, 0x22] {
                let entry = idt_base + vec * 8;
                let w0 = rdw(entry);
                let w1 = rdw(entry + 4);
                let offset = (w0 & 0xFFFF) | (w1 & 0xFFFF0000);
                let sel = (w0 >> 16) & 0xFFFF;
                println!("IDT[{:#04x}]: handler={:#010x} sel={:#06x} dw0={:#010x} dw1={:#010x}",
                    vec, offset, sel, w0, w1);
                if (offset as usize) + 16 < rl {
                    let mut bytes = Vec::new();
                    for i in 0..16 {
                        bytes.push(format!("{:02x}", unsafe { *rp.add(offset as usize + i) }));
                    }
                    println!("  code: {}", bytes.join(" "));
                }
            }
            // Also dump IRQ0 vector for BIOS IVT (real-mode vector 8 and 9)
            let ivt8 = rdw(0x20);
            let ivt9 = rdw(0x24);
            println!("IVT[8] = {:#010x} (seg:off = {:04x}:{:04x})", ivt8,
                (ivt8 >> 16) & 0xFFFF, ivt8 & 0xFFFF);
            println!("IVT[9] = {:#010x} (seg:off = {:04x}:{:04x})", ivt9,
                (ivt9 >> 16) & 0xFFFF, ivt9 & 0xFFFF);
        }

        // Additional key addresses for ISOLINUX debugging
        {
            let (rp2, rl2) = emu.memory.get_ram_base_ptr();
            let rdw2 = |addr: usize| -> u32 {
                if addr + 3 < rl2 { unsafe { (rp2.add(addr) as *const u32).read_unaligned() } } else { 0xDEAD }
            };

            // [0x110D94] — polling loop flag (CMP WORD [0x110D94], 0)
            let poll_flag = rdw2(0x110D94);
            println!("[0x110D94] poll_flag = {:#010x}", poll_flag);

            // IVT entry for INT 22h (SYSLINUX API vector) — at 0x88
            let ivt22 = rdw2(0x88);
            println!("IVT[22h] = {:#010x} (seg:off = {:04x}:{:04x})",
                ivt22, (ivt22 >> 16) & 0xFFFF, ivt22 & 0xFFFF);

            // PM IDT entry for INT 22h (SYSLINUX API) — at IDT_base + 0x22*8
            let idt_base2 = rdw2(0x8E7E) as usize;
            if idt_base2 > 0 && idt_base2 + 0x22 * 8 + 8 < rl2 {
                let idt22_w0 = rdw2(idt_base2 + 0x22 * 8);
                let idt22_w1 = rdw2(idt_base2 + 0x22 * 8 + 4);
                let idt22_offset = (idt22_w0 & 0xFFFF) | (idt22_w1 & 0xFFFF0000);
                let idt22_sel = (idt22_w0 >> 16) & 0xFFFF;
                let idt22_type = (idt22_w1 >> 8) & 0x1F;
                println!("PM IDT[22h]: offset={:#010x} sel={:#06x} type={:#04x} dw0={:#010x} dw1={:#010x}",
                    idt22_offset, idt22_sel, idt22_type, idt22_w0, idt22_w1);
                // Dump first 16 bytes of code at the IDT handler
                if (idt22_offset as usize) + 16 < rl2 {
                    print!("  handler code: ");
                    for i in 0..16 {
                        print!("{:02x} ", unsafe { *rp2.add(idt22_offset as usize + i) });
                    }
                    println!();
                }
            }

            // Also dump IDT entries for vectors 0x13 (INT 13h), 0x15, 0x16, 0x20-0x23
            println!("PM IDT critical entries:");
            for vec in [0x13u32, 0x15, 0x16, 0x20, 0x21, 0x22, 0x30, 0x31] {
                if idt_base2 + (vec as usize) * 8 + 8 < rl2 {
                    let w0 = rdw2(idt_base2 + (vec as usize) * 8);
                    let w1 = rdw2(idt_base2 + (vec as usize) * 8 + 4);
                    let offset = (w0 & 0xFFFF) | (w1 & 0xFFFF0000);
                    let sel = (w0 >> 16) & 0xFFFF;
                    println!("  IDT[{:#04x}]: handler={:#010x} sel={:#06x} dw0={:#010x} dw1={:#010x}",
                        vec, offset, sel, w0, w1);
                }
            }

            // Dump the PM IDT common dispatch handler
            // IDT[22h] handler at 0x3B8B does: PUSH 0x22 (6a 22); JMP rel8 +0x72 (eb 72)
            // Target = 0x3B8D + 2 + 0x72 = 0x3C01
            println!("\n===== PM COMMON DISPATCH HANDLER (0x3C01) =====");
            let dispatch_addr = 0x3C01usize;
            if dispatch_addr + 128 < rl2 {
                for row in 0..8usize {
                    let base = dispatch_addr + row * 16;
                    print!("{:#06x}: ", base);
                    for i in 0..16usize {
                        print!("{:02x} ", unsafe { *rp2.add(base + i) });
                    }
                    // ASCII
                    print!(" |");
                    for i in 0..16usize {
                        let b = unsafe { *rp2.add(base + i) };
                        print!("{}", if b >= 0x20 && b < 0x7F { b as char } else { '.' });
                    }
                    println!("|");
                }
            }

            // Also dump 16 bytes BEFORE the dispatch target for context
            println!("Pre-dispatch (0x3BF0):");
            if 0x3BF0 + 32 < rl2 {
                print!("0x3bf0: ");
                for i in 0..32usize {
                    print!("{:02x} ", unsafe { *rp2.add(0x3BF0 + i) });
                }
                println!();
            }

            // Search for "isolinux" in first 16MB of RAM
            println!("\n===== ISOLINUX STRING SEARCH =====");
            {
                let search_len = rl2.min(16 * 1024 * 1024);
                let needle = b"isolinux";
                let mut found = 0;
                for addr in 0..search_len.saturating_sub(needle.len()) {
                    let matches = (0..needle.len()).all(|i| {
                        let b = unsafe { *rp2.add(addr + i) };
                        b.to_ascii_lowercase() == needle[i]
                    });
                    if matches && found < 10 {
                        // Show 80 bytes of context
                        let start = if addr >= 16 { addr - 16 } else { 0 };
                        let end = (addr + 64).min(search_len);
                        let context: Vec<u8> = (start..end).map(|a| unsafe { *rp2.add(a) }).collect();
                        let text: String = context.iter().map(|&b| {
                            if b >= 0x20 && b < 0x7F { b as char } else { '.' }
                        }).collect();
                        println!("  [{:#010x}] {:?}", addr, text);
                        found += 1;
                    }
                }
                if found == 0 {
                    println!("  'isolinux' NOT FOUND in first 16MB");
                } else {
                    println!("  ({} hits total)", found);
                }
            }

            // Check IRQ1 (keyboard) delivery in IAC vectors
            println!("\n===== KEYBOARD IRQ1 CHECK =====");
            let iac_vecs2 = emu.cpu.get_iac_vectors();
            println!("IRQ0 (vec 0x08): {} deliveries", iac_vecs2[0x08]);
            println!("IRQ1 (vec 0x09): {} deliveries", iac_vecs2[0x09]);
            println!("IRQ2 (vec 0x0A): {} deliveries", iac_vecs2[0x0A]);
            // Also check remapped PIC vectors (0x20+ for ISOLINUX)
            println!("PIC vec 0x20: {} deliveries", iac_vecs2[0x20]);
            println!("PIC vec 0x21: {} deliveries", iac_vecs2[0x21]);
            // Check inject path too
            let (_, inject_vecs2) = emu.cpu.get_inject_ext_intr_diag();
            println!("inject vec 0x08: {}", inject_vecs2[0x08]);
            println!("inject vec 0x09: {}", inject_vecs2[0x09]);
            println!("inject vec 0x20: {}", inject_vecs2[0x20]);
            println!("inject vec 0x21: {}", inject_vecs2[0x21]);

            // ISOLINUX key data addresses
            let callback = rdw2(0x8E84);
            let irq_chain = rdw2(0x8E88);
            println!("\n[0x8E84] callback_ptr = {:#010x}", callback);
            println!("[0x8E88] irq_chain    = {:#010x}", irq_chain);

            // Stack context at first HLT: [ESP+0x58] checked for bit 6
            if let Some((_, _, _, _, _, regs, _)) = emu.cpu.get_first_pm_hlt() {
                let esp = regs[4] as usize;
                if esp + 0x60 < rl2 {
                    let val_58 = unsafe { *rp2.add(esp + 0x58) };
                    let dw_58 = rdw2(esp + 0x58);
                    println!("[ESP+0x58] = byte:{:#04x} dword:{:#010x} (TEST 0x40 = {})",
                        val_58, dw_58, if val_58 & 0x40 != 0 { "SET" } else { "clear" });
                }
            }

            // Dump code at the idle function 0x100c40-0x100c80
            println!("\n===== IDLE FUNCTION (0x100C40) =====");
            print!("0x100c40: ");
            for i in 0..64usize {
                let addr = 0x100c40 + i;
                let b = if addr < rl2 { unsafe { *rp2.add(addr) } } else { 0xFF };
                print!("{:02x} ", b);
                if (i + 1) % 16 == 0 && i < 63 { print!("\n{:#08x}: ", 0x100c40 + i + 1); }
            }
            println!();

            // Dump code around the caller at 0x10415c-0x1041a0
            println!("\n===== CALLER LOOP (0x10415C) =====");
            print!("0x10415c: ");
            for i in 0..68usize {
                let addr = 0x10415c + i;
                let b = if addr < rl2 { unsafe { *rp2.add(addr) } } else { 0xFF };
                print!("{:02x} ", b);
                if (i + 1) % 16 == 0 && i < 67 { print!("\n{:#08x}: ", 0x10415c + i + 1); }
            }
            println!();
        }

        // Dump ISOLINUX timer function at 0x100c40-0x100c70
        println!("\n===== ISOLINUX TIMER FUNCTION DUMP =====");
        let (ram_ptr0, ram_len0) = emu.memory.get_ram_base_ptr();
        let rb = |addr: usize| -> u8 {
            if addr < ram_len0 { unsafe { *ram_ptr0.add(addr) } } else { 0xFF }
        };
        let rd = |addr: usize| -> u32 {
            if addr + 3 < ram_len0 {
                unsafe { (ram_ptr0.add(addr) as *const u32).read_unaligned() }
            } else { 0xDEADDEAD }
        };
        // Dump 0x100c40 to 0x100c70
        print!("0x100c40: ");
        for i in 0..48usize {
            print!("{:02x} ", rb(0x100c40 + i));
            if (i + 1) % 16 == 0 && i < 47 { print!("\n{:#08x}: ", 0x100c40 + i + 1); }
        }
        println!();
        // Decode MOV EAX, [moffs32] at 0x100c40 (if A1 opcode)
        let b0 = rb(0x100c40);
        if b0 == 0xA1 {
            let timer_addr = rd(0x100c41);
            let timer_val = rd(timer_addr as usize);
            println!("Timer: MOV EAX, [{:#010x}] = {:#010x}", timer_addr, timer_val);
        } else {
            println!("Function starts with opcode {:#04x} (not A1=MOV EAX,moffs)", b0);
        }
        // Dump the caller at 0x104160-0x104198
        print!("0x104160: ");
        for i in 0..56usize {
            print!("{:02x} ", rb(0x104160 + i));
            if (i + 1) % 16 == 0 && i < 55 { print!("\n{:#08x}: ", 0x104160 + i + 1); }
        }
        println!();

        // ISOLINUX PM code dump using raw memory pointer
        println!("\n===== ISOLINUX PM CODE DUMP =====");
        let (ram_ptr, ram_len) = emu.memory.get_ram_base_ptr();
        let read_byte = |addr: usize| -> u8 {
            if addr < ram_len { unsafe { *ram_ptr.add(addr) } } else { 0xFF }
        };
        let read_dword = |addr: usize| -> u32 {
            if addr + 3 < ram_len {
                unsafe { (ram_ptr.add(addr) as *const u32).read_unaligned() }
            } else { 0xDEADDEAD }
        };

        // Dump unknown RM handler at 0x8662 (dominant PM→RM bounce target)
        println!("\n===== RM HANDLER at 0x8662 (dominant bounce target) =====");
        for row in 0..4usize {
            let base = 0x8662 + row * 16;
            print!("{:#06x}: ", base);
            for i in 0..16usize {
                print!("{:02x} ", read_byte(base + i));
            }
            print!(" |");
            for i in 0..16usize {
                let b = read_byte(base + i);
                print!("{}", if b >= 0x20 && b < 0x7F { b as char } else { '.' });
            }
            println!("|");
        }
        // Full context 0x8640-0x86A0
        println!("0x8640-0x86A0 context:");
        for row in 0..6usize {
            let base = 0x8640 + row * 16;
            print!("{:#06x}: ", base);
            for i in 0..16usize {
                print!("{:02x} ", read_byte(base + i));
            }
            println!();
        }
        // RM stack pointer for __intcall
        let rm_sp_val = read_byte(0x38B8) as u16 | ((read_byte(0x38B9) as u16) << 8);
        let rm_ss_val = read_byte(0x38BA) as u16 | ((read_byte(0x38BB) as u16) << 8);
        println!("\nRM stack [0x38B8]: SS:SP = {:04x}:{:04x} (phys {:#x})",
            rm_ss_val, rm_sp_val, (rm_ss_val as u32) * 16 + rm_sp_val as u32);
        let rm_stack_phys = (rm_ss_val as usize) * 16 + rm_sp_val as usize;
        if rm_stack_phys + 64 < ram_len {
            println!("RM stack contents:");
            for row in 0..4usize {
                let base = rm_stack_phys + row * 16;
                print!("  {:#06x}: ", base);
                for i in 0..16usize {
                    print!("{:02x} ", read_byte(base + i));
                }
                println!();
            }
        }

        // Dump code at 0x84A0-0x84EF (pre-PM entry)
        print!("0x84A0: ");
        for i in 0..80usize {
            print!("{:02x} ", read_byte(0x84A0 + i));
            if (i + 1) % 16 == 0 && i < 79 { print!("\n{:#06x}: ", 0x84A0 + i + 1); }
        }
        println!();
        // Dump PM entry at 0x89A0-0x89FF
        print!("0x89A0: ");
        for i in 0..96usize {
            print!("{:02x} ", read_byte(0x89A0 + i));
            if (i + 1) % 16 == 0 && i < 95 { print!("\n{:#06x}: ", 0x89A0 + i + 1); }
        }
        println!();
        // Key data locations
        let saved_esp = read_dword(0x8E78);
        let timer_counter = read_dword(0x3AD8);
        let callback_ptr = read_dword(0x8E84);
        println!("[0x8E78] saved_esp = {:#010x}", saved_esp);
        println!("[0x3AD8] timer_cnt = {:#010x}", timer_counter);
        println!("[0x8E84] callback  = {:#010x}", callback_ptr);
        // GDT at 0x8E00
        print!("GDT[0x8E00]: ");
        for i in 0..48usize {
            print!("{:02x} ", read_byte(0x8E00 + i));
            if (i + 1) % 8 == 0 { print!("| "); }
        }
        println!();
        // Stack around saved_esp
        if saved_esp > 0 && saved_esp < 0x1000000 {
            println!("Stack around saved_esp ({:#x}):", saved_esp);
            let base = if saved_esp >= 64 { saved_esp - 64 } else { 0 };
            for i in 0..32u32 {
                let addr = base + i * 4;
                let val = read_dword(addr as usize);
                let marker = if addr == saved_esp { " <-- ESP" } else { "" };
                println!("  [{:#010x}] = {:#010x}{}", addr, val, marker);
            }
        }
        // IDT dump (first few entries from PM IDT)
        // ISOLINUX PM LIDT loads from [0x8E7C]
        let idt_limit = read_byte(0x8E7C) as u16 | ((read_byte(0x8E7D) as u16) << 8);
        let idt_base = read_dword(0x8E7E);
        println!("PM IDT: limit={:#x} base={:#010x}", idt_limit, idt_base);
        if idt_base > 0 && idt_base < 0x1000000 {
            for i in 0..8u32 {
                let entry_base = idt_base as usize + i as usize * 8;
                let w0 = read_dword(entry_base);
                let w1 = read_dword(entry_base + 4);
                let offset = (w0 & 0xFFFF) | ((w1 & 0xFFFF0000));
                let sel = (w0 >> 16) & 0xFFFF;
                let typ = (w1 >> 8) & 0x1F;
                println!("  IDT[{}]: offset={:#010x} sel={:#06x} type={:#04x} dw0={:#010x} dw1={:#010x}",
                    i, offset, sel, typ, w0, w1);
            }
        }

        // VGA text dump
        println!("\n===== VGA TEXT DUMP =====");
        let vga_text = emu.vga_scan_text_memory();
        for line in vga_text.lines() {
            if !line.trim().is_empty() {
                println!("  {}", line);
            }
        }
        println!("===== END VGA TEXT DUMP =====");

        // First HLT in PM — ISOLINUX idle loop entry state
        if let Some((icount, rip, cs, ss, eflags, regs, stack)) = emu.cpu.get_first_pm_hlt() {
            println!("\n===== FIRST HLT IN PROTECTED MODE =====");
            println!("icount:  {}", icount);
            println!("RIP:     {:#010x}", rip);
            println!("CS:      {:#06x}  SS: {:#06x}", cs, ss);
            println!("EFLAGS:  {:#010x}", eflags);
            println!("EAX={:08x} ECX={:08x} EDX={:08x} EBX={:08x}", regs[0], regs[1], regs[2], regs[3]);
            println!("ESP={:08x} EBP={:08x} ESI={:08x} EDI={:08x}", regs[4], regs[5], regs[6], regs[7]);
            println!("Stack (from ESP):");
            let esp = regs[4];
            for i in 0..16u32 {
                println!("  [{:#010x}] = {:#010x}", esp.wrapping_add(i * 4), stack[i as usize]);
            }
        }

        // Search for config file content in RAM
        println!("\n===== CONFIG FILE SEARCH =====");
        {
            let (rp3, rl3) = emu.memory.get_ram_base_ptr();
            // Search for "SERIAL" string (from syslinux.cfg)
            let needle = b"SERIAL";
            let mut found_count = 0;
            for addr in 0..rl3.saturating_sub(needle.len()) {
                let matches = (0..needle.len()).all(|i| unsafe { *rp3.add(addr + i) } == needle[i]);
                if matches && found_count < 5 {
                    // Show context around the match
                    let start = if addr >= 32 { addr - 32 } else { 0 };
                    let end = (addr + 64).min(rl3);
                    let context: Vec<u8> = (start..end).map(|a| unsafe { *rp3.add(a) }).collect();
                    let text = String::from_utf8_lossy(&context);
                    println!("  Found 'SERIAL' at {:#010x}: {:?}", addr, text.chars().take(80).collect::<String>());
                    found_count += 1;
                }
            }
            if found_count == 0 {
                println!("  'SERIAL' NOT FOUND in RAM — config file was never loaded!");
            }

            // Also search for "TIMEOUT" (another config keyword)
            let needle2 = b"TIMEOUT";
            let mut found_count2 = 0;
            for addr in 0..rl3.saturating_sub(needle2.len()) {
                let matches = (0..needle2.len()).all(|i| unsafe { *rp3.add(addr + i) } == needle2[i]);
                if matches && found_count2 < 3 {
                    let start = if addr >= 16 { addr - 16 } else { 0 };
                    let end = (addr + 48).min(rl3);
                    let context: Vec<u8> = (start..end).map(|a| unsafe { *rp3.add(a) }).collect();
                    let text = String::from_utf8_lossy(&context);
                    println!("  Found 'TIMEOUT' at {:#010x}: {:?}", addr, text.chars().take(60).collect::<String>());
                    found_count2 += 1;
                }
            }
            if found_count2 == 0 {
                println!("  'TIMEOUT' NOT FOUND in RAM");
            }

            // Search for "vmlinuz" (kernel name from config)
            let needle3 = b"vmlinuz";
            let mut found_count3 = 0;
            for addr in 0..rl3.saturating_sub(needle3.len()) {
                let matches = (0..needle3.len()).all(|i| unsafe { *rp3.add(addr + i) } == needle3[i]);
                if matches && found_count3 < 3 {
                    println!("  Found 'vmlinuz' at {:#010x}", addr);
                    found_count3 += 1;
                }
            }
            if found_count3 == 0 {
                println!("  'vmlinuz' NOT FOUND in RAM");
            }
        }

        // PM↔RM transition counts
        println!("\n===== PM/RM TRANSITIONS =====");
        let (pm_to_rm, rm_to_pm) = emu.cpu.get_pm_rm_transition_counts();
        println!("PM→RM transitions: {}", pm_to_rm);
        println!("RM→PM transitions: {}", rm_to_pm);
        // timer_cnt at [0x3AD8] counts hardware IRQ bounces (ISOLINUX increments it at 0x89D7)
        let timer_cnt_val = {
            let (rp_t, rl_t) = emu.memory.get_ram_base_ptr();
            if 0x3AD8 + 3 < rl_t {
                unsafe { (rp_t.add(0x3AD8) as *const u32).read_unaligned() }
            } else { 0 }
        };
        println!("[0x3AD8] timer_cnt (hw IRQ bounces): {}", timer_cnt_val);
        let sw_bounces = pm_to_rm.saturating_sub(timer_cnt_val as u64);
        // Also subtract BIOS PM→RM transitions (initial rombios32 PM return)
        println!("Estimated software INT bounces (__intcall): ~{}", sw_bounces);
        if sw_bounces == 0 {
            println!("*** WARNING: __intcall was NEVER called! ISOLINUX cannot make BIOS calls from PM! ***");
        }

        // SYSLINUX init function table dump
        // The PM core iterates through structures at 0x110DC0 + index*0x3B
        // Each entry should contain a function pointer for init functions like iso_init
        println!("\n===== SYSLINUX INIT FUNCTION TABLE (0x110DC0) =====");
        {
            let (rp_ft, rl_ft) = emu.memory.get_ram_base_ptr();
            let rd_ft = |addr: usize| -> u32 {
                if addr + 3 < rl_ft { unsafe { (rp_ft.add(addr) as *const u32).read_unaligned() } } else { 0xDEAD }
            };
            let rb_ft = |addr: usize| -> u8 {
                if addr < rl_ft { unsafe { *rp_ft.add(addr) } } else { 0xFF }
            };
            // Dump entries 0-31 (each 0x3B = 59 bytes)
            for i in 0..32u32 {
                let base = 0x110DC0 + (i * 0x3B) as usize;
                let dw0 = rd_ft(base);
                let dw1 = rd_ft(base + 4);
                let dw2 = rd_ft(base + 8);
                let b0 = rb_ft(base);
                // Only show non-zero entries or a few zero ones for context
                if dw0 != 0 || dw1 != 0 || i < 4 || i == 0x1C {
                    print!("  [{}] @{:#08x}: {:08x} {:08x} {:08x}", i, base, dw0, dw1, dw2);
                    // Show first 16 bytes as hex
                    print!("  bytes:");
                    for j in 0..16usize {
                        print!(" {:02x}", rb_ft(base + j));
                    }
                    println!();
                }
            }
            // Also dump the raw memory at the table base
            println!("Raw 0x110DC0-0x110E40:");
            for row in 0..8usize {
                let base = 0x110DC0 + row * 16;
                print!("  {:#08x}:", base);
                for i in 0..16usize {
                    print!(" {:02x}", rb_ft(base + i));
                }
                println!();
            }
        }

        // Boot info table check — verify the boot info table is accessible in RAM
        println!("\n===== BOOT INFO TABLE =====");
        {
            let (rp_bi, rl_bi) = emu.memory.get_ram_base_ptr();
            let rd_bi = |addr: usize| -> u32 {
                if addr + 3 < rl_bi { unsafe { (rp_bi.add(addr) as *const u32).read_unaligned() } } else { 0xDEAD }
            };
            // El Torito loads boot image at 0x7C00. Boot info table is at offset 8.
            let pvd_lba = rd_bi(0x7C08);
            let file_lba = rd_bi(0x7C0C);
            let file_len = rd_bi(0x7C10);
            let checksum = rd_bi(0x7C14);
            println!("At 0x7C08 (original El Torito load location):");
            println!("  PVD LBA:       {} ({:#x})", pvd_lba, pvd_lba);
            println!("  Boot file LBA: {} ({:#x})", file_lba, file_lba);
            println!("  Boot file len: {} ({:#x})", file_len, file_len);
            println!("  Checksum:      {:#010x}", checksum);
            if pvd_lba == 0 && file_lba == 0 {
                println!("  *** BOOT INFO TABLE NOT FOUND at 0x7C08! ***");
            }
            // Also check at 0x8808 (0x8800 = 0x880 << 4, which is where isolinux may relocate stage 1)
            let pvd2 = rd_bi(0x8808);
            let file2 = rd_bi(0x880C);
            if pvd2 > 0 && pvd2 < 100 {
                println!("  (Also found at 0x8808: PVD LBA={}, file LBA={})", pvd2, file2);
            }

            // Relocated image at 0x100000 — ISOLINUX relocates itself here.
            // Boot info table should be at 0x100008 (offset 8 from relocated base).
            // Expected: 10 00 00 00 (PVD=16), 38 00 00 00 (file=56),
            //           00 A8 00 00 (len=43008), EB 8A 81 22 (checksum)
            println!("\nRelocated image at 0x100000 (first 32 bytes):");
            print!("  0x100000: ");
            for i in 0..32usize {
                let addr = 0x100000 + i;
                let b = if addr < rl_bi { unsafe { *rp_bi.add(addr) } } else { 0xFF };
                print!("{:02x} ", b);
                if i == 15 { print!("\n  0x100010: "); }
            }
            println!();
            let rel_pvd = rd_bi(0x100008);
            let rel_file = rd_bi(0x10000C);
            let rel_len = rd_bi(0x100010);
            let rel_csum = rd_bi(0x100014);
            println!("  Relocated boot info table at 0x100008:");
            println!("    PVD LBA:       {} ({:#x})  expected: 16 (0x10)", rel_pvd, rel_pvd);
            println!("    Boot file LBA: {} ({:#x})  expected: 56 (0x38)", rel_file, rel_file);
            println!("    Boot file len: {} ({:#x})  expected: 43008 (0xa800)", rel_len, rel_len);
            println!("    Checksum:      {:#010x}  expected: 0x22818aeb", rel_csum);
            let match_ok = rel_pvd == 16 && rel_file == 56 && rel_len == 43008 && rel_csum == 0x22818AEB;
            if match_ok {
                println!("    MATCH: YES — boot info table correctly relocated");
            } else if rel_pvd == 0 && rel_file == 0 && rel_len == 0 {
                println!("    *** BOOT INFO TABLE ALL ZEROS at 0x100008! Relocation failed or wrong address! ***");
            } else {
                println!("    *** MISMATCH — boot info table at 0x100008 has unexpected values! ***");
            }

            // Also dump 0x100000-0x100020 as dwords for easier reading
            println!("  As dwords:");
            for i in 0..8u32 {
                let addr = 0x100000 + (i as usize) * 4;
                let val = rd_bi(addr);
                println!("    [{:#010x}] = {:#010x} ({})", addr, val, val);
            }
        }

        // IVT entries for key BIOS interrupts
        println!("\n===== IVT ENTRIES (real-mode INT vectors) =====");
        {
            let (rp_ivt, rl_ivt) = emu.memory.get_ram_base_ptr();
            let rd_ivt = |addr: usize| -> u32 {
                if addr + 3 < rl_ivt { unsafe { (rp_ivt.add(addr) as *const u32).read_unaligned() } } else { 0xDEAD }
            };
            for vec_num in [0x08u8, 0x09, 0x10, 0x13, 0x15, 0x16, 0x19, 0x1A, 0x1C] {
                let addr = vec_num as usize * 4;
                let entry = rd_ivt(addr);
                let seg = (entry >> 16) & 0xFFFF;
                let off = entry & 0xFFFF;
                println!("  IVT[{:#04x}] = {:04x}:{:04x} (phys {:#010x})",
                    vec_num, seg, off, seg * 16 + off);
            }
        }

        // Memory gap verification: 0x8400-0x847F
        // El Torito loads 0x7C00-0x83FF, stub loads 0x8800+ — gap must be filled by copy loop
        println!("\n===== MEMORY GAP CHECK (0x8400-0x847F) =====");
        {
            let (rp_gap, rl_gap) = emu.memory.get_ram_base_ptr();
            // Expected bytes from ISO at boot image offset 0x800
            let expected: [u8; 16] = [0xe8, 0x0f, 0x02, 0x66, 0x68, 0x50, 0x89, 0x00,
                                       0x00, 0xe8, 0x83, 0x00, 0x66, 0x3d, 0xf0, 0xf2];
            let mut actual = [0u8; 16];
            for i in 0..16 {
                actual[i] = if 0x8400 + i < rl_gap { unsafe { *rp_gap.add(0x8400 + i) } } else { 0xFF };
            }
            let match_ok = actual == expected;
            print!("  Expected: ");
            for b in &expected { print!("{:02x} ", b); }
            println!();
            print!("  Actual:   ");
            for b in &actual { print!("{:02x} ", b); }
            println!();
            println!("  Match: {}", if match_ok { "YES — gap filled correctly" } else { "NO — GAP DATA MISMATCH!" });

            // Dump full 128 bytes for comparison
            print!("  0x8400: ");
            for i in 0..128usize {
                let b = if 0x8400 + i < rl_gap { unsafe { *rp_gap.add(0x8400 + i) } } else { 0xFF };
                print!("{:02x} ", b);
                if (i + 1) % 16 == 0 && i < 127 { print!("\n  {:#06x}: ", 0x8400 + i + 1); }
            }
            println!();
            // Check if gap is all zeros (uninitialized)
            let all_zero = (0..0x400usize).all(|i| {
                if 0x8400 + i < rl_gap { unsafe { *rp_gap.add(0x8400 + i) == 0 } } else { false }
            });
            if all_zero {
                println!("  *** GAP IS ALL ZEROS — stub copy loop never filled 0x8400-0x87FF! ***");
            }
        }

        // Check PVD in RAM (search for CD001 signature)
        println!("\n===== PVD 'CD001' SEARCH =====");
        {
            let (rp_pvd, rl_pvd) = emu.memory.get_ram_base_ptr();
            let needle = b"CD001";
            let search_end = rl_pvd.min(16 * 1024 * 1024);
            let mut found = 0;
            for addr in 0..search_end.saturating_sub(5) {
                let ok = (0..5).all(|i| unsafe { *rp_pvd.add(addr + i) } == needle[i]);
                if ok && found < 5 {
                    println!("  Found CD001 at {:#010x}", addr);
                    found += 1;
                }
            }
            if found == 0 {
                println!("  CD001 NOT FOUND — PVD was NEVER read from disk!");
            }
        }

        // Dump __intcall PM code at 0x106AE3
        println!("\n===== PM __intcall CODE (0x106AE3) =====");
        {
            let (rp_ic, rl_ic) = emu.memory.get_ram_base_ptr();
            for row in 0..12usize {
                let base = 0x106AE3 + row * 16;
                if base + 16 < rl_ic {
                    print!("{:#08x}: ", base);
                    for i in 0..16usize {
                        print!("{:02x} ", unsafe { *rp_ic.add(base + i) });
                    }
                    println!();
                }
            }
            // Also dump the RM dispatch at 0x8490-0x84CF (entry) and the code
            // that the POPAD/RET4 returns to
            println!("\n===== RM __intcall ENTRY (0x8490-0x84D2) =====");
            for row in 0..5usize {
                let base = 0x8490 + row * 16;
                if base + 16 < rl_ic {
                    print!("{:#08x}: ", base);
                    for i in 0..16usize {
                        print!("{:02x} ", unsafe { *rp_ic.add(base + i) });
                    }
                    println!();
                }
            }
            // Check what's at the RM stack return address (where RET4 from 0x84A7 goes)
            // After POPAD: SP = start + 44 (4 segs * 2 + POPAD 32 + POPFD 4)
            // RET4 pops IP from [SP+44] then skips 4 bytes
            let rm_sp = 0x7B68usize; // from [0x38B8]
            let ret_ip_addr = rm_sp + 8 + 32 + 4; // GS+FS+ES+DS + POPAD + POPFD = 44 = 0x2C
            let ret_ip = if ret_ip_addr + 1 < rl_ic {
                unsafe { (rp_ic.add(ret_ip_addr) as *const u16).read_unaligned() }
            } else { 0xFFFF };
            println!("\nRM stack ret IP @ {:#06x} = {:#06x}", ret_ip_addr, ret_ip);
            // Dump code at that return address
            if (ret_ip as usize) + 32 < rl_ic {
                print!("Code at {:#06x}: ", ret_ip);
                for i in 0..32usize {
                    print!("{:02x} ", unsafe { *rp_ic.add(ret_ip as usize + i) });
                }
                println!();
            }
            // Also dump what INT vector the __intcall code stores
            // In SYSLINUX, the INT number is patched into a self-modifying INT instruction
            // Look for CD xx (INT nn) pattern near the dispatch area
            println!("\nSearching for INT (CD xx) in 0x8470-0x8530:");
            for addr in 0x8470usize..0x8530 {
                if addr + 1 < rl_ic {
                    let b0 = unsafe { *rp_ic.add(addr) };
                    let b1 = unsafe { *rp_ic.add(addr + 1) };
                    if b0 == 0xCD {
                        println!("  {:#06x}: CD {:02x} (INT {:#04x})", addr, b1, b1);
                    }
                }
            }
            // Also check in the __intcall PM code
            println!("Searching for INT (CD xx) in 0x106AE0-0x106B80:");
            for addr in 0x106AE0usize..0x106B80 {
                if addr + 1 < rl_ic {
                    let b0 = unsafe { *rp_ic.add(addr) };
                    let b1 = unsafe { *rp_ic.add(addr + 1) };
                    if b0 == 0xCD {
                        println!("  {:#06x}: CD {:02x} (INT {:#04x})", addr, b1, b1);
                    }
                }
            }
            // Dump GOT area 0x8E48-0x8E98 (function pointers + data)
            println!("\nGOT / data area 0x8E48-0x8E98:");
            for row in 0..5usize {
                let base = 0x8E48 + row * 16;
                if base + 16 < rl_ic {
                    print!("{:#06x}: ", base);
                    for i in 0..16usize {
                        print!("{:02x} ", unsafe { *rp_ic.add(base + i) });
                    }
                    // Also print as dwords
                    print!("  dw:");
                    for i in (0..16).step_by(4) {
                        let dw = unsafe { (rp_ic.add(base + i) as *const u32).read_unaligned() };
                        print!(" {:#010x}", dw);
                    }
                    println!();
                }
            }
            // The function pointer [0x8E58] should point to the actual __intcall impl
            let intcall_fn = if 0x8E5B < rl_ic {
                unsafe { (rp_ic.add(0x8E58) as *const u32).read_unaligned() }
            } else { 0 };
            println!("\n[0x8E58] __intcall func ptr = {:#010x}", intcall_fn);
            if (intcall_fn as usize) + 64 < rl_ic {
                println!("Code at __intcall impl ({:#010x}):", intcall_fn);
                for row in 0..4usize {
                    let base = intcall_fn as usize + row * 16;
                    print!("{:#08x}: ", base);
                    for i in 0..16usize {
                        print!("{:02x} ", unsafe { *rp_ic.add(base + i) });
                    }
                    println!();
                }
            }
            // Also dump CDROM drive number at [0x3017]
            let drv_num = if 0x3017 < rl_ic { unsafe { *rp_ic.add(0x3017) } } else { 0xFF };
            println!("\n[0x3017] CDROM drive number = {:#04x}", drv_num);
            // Spec packet at [0x3030]
            print!("[0x3030] Spec packet: ");
            for i in 0..19usize {
                if 0x3030 + i < rl_ic {
                    print!("{:02x} ", unsafe { *rp_ic.add(0x3030 + i) });
                }
            }
            println!();
            // Dump [0x3020-0x3050] for more context
            println!("Stub data 0x3000-0x3050:");
            for row in 0..5usize {
                let base = 0x3000 + row * 16;
                if base + 16 < rl_ic {
                    print!("{:#06x}: ", base);
                    for i in 0..16usize {
                        print!("{:02x} ", unsafe { *rp_ic.add(base + i) });
                    }
                    println!();
                }
            }
            // Search for "Boot failed" in RAM
            println!("\nSearching for 'Boot failed' in RAM:");
            let needle = b"Boot failed";
            let search_end = rl_ic.min(16 * 1024 * 1024);
            let mut bf_found = 0;
            for addr in 0..search_end.saturating_sub(needle.len()) {
                let ok = (0..needle.len()).all(|i| unsafe { *rp_ic.add(addr + i) } == needle[i]);
                if ok && bf_found < 5 {
                    println!("  Found at {:#010x}", addr);
                    bf_found += 1;
                }
            }
            if bf_found == 0 { println!("  NOT FOUND"); }
        }

        // Serial port TX output (COM1) — ISOLINUX uses SERIAL 0 115200
        println!("\n===== SERIAL (COM1) OUTPUT =====");
        let serial_bytes: Vec<u8> = emu.device_manager.drain_serial_tx(0).collect();
        if serial_bytes.is_empty() {
            println!("  (no serial output)");
        } else {
            println!("  {} bytes total", serial_bytes.len());
            let text = String::from_utf8_lossy(&serial_bytes);
            for line in text.lines().take(40) {
                println!("  {}", line);
            }
        }
        println!("===== END SERIAL OUTPUT =====");
    }

    Ok(())
}
