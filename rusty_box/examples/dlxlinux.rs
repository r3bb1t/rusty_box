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
    // Create and configure emulator
    // =========================================================================
    let config = EmulatorConfig {
        // Match bochsrc.bxrc: 32 MB RAM
        // Stack overflow fixed by Boxing icache.mpool and returning Box<Emulator>
        guest_memory_size: 32 * 1024 * 1024, // 32 MB
        host_memory_size: 32 * 1024 * 1024,  // 32 MB
        memory_block_size: 128 * 1024,
        ips: 15_000_000,   // IPS from bochsrc.bxrc
        pci_enabled: true, // Enable PCI for shadow RAM support
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
    let headless = std::env::var_os("RUSTY_BOX_HEADLESS").is_some();
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
    // Base: 640KB, Extended: 31 MB = 31 * 1024 KB
    emu.configure_memory_in_cmos(640, 31 * 1024);

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

    // Run with instruction limit to allow debugging
    let max_instructions: u64 = std::env::var("MAX_INSTRUCTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000_000); // 50M instructions default

    // Use interactive loop that handles GUI events
    let result = emu.run_interactive(max_instructions);

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

    // GDT diagnostic dump: show GDTR, segment bases, and raw GDT entries
    {
        let gdtr_base = emu.cpu.get_gdtr_base();
        let gdtr_limit = emu.cpu.get_gdtr_limit();
        let cs_base = emu.cpu.get_cs_base();
        let ds_base = emu.cpu.get_ds_base();
        let ss_base = emu.cpu.get_ss_base();
        let cs_sel = emu.cpu.get_cs_selector();
        let ds_sel = emu.cpu.get_ds_selector();
        let ss_sel = emu.cpu.get_ss_selector();
        let cr3 = emu.cpu.get_cr3_val();
        println!();
        println!("===== GDT DIAGNOSTIC =====");
        println!(
            "GDTR: base={:#010x} limit={:#06x}  CR3={:#010x}",
            gdtr_base, gdtr_limit, cr3
        );
        println!(
            "CS={:#06x} base={:#010x}  DS={:#06x} base={:#010x}  SS={:#06x} base={:#010x}",
            cs_sel, cs_base, ds_sel, ds_base, ss_sel, ss_base
        );

        // If GDTR.base looks like a high virtual address (e.g. 0xC0xxxxxx),
        // compute the expected physical address
        let gdt_phys_base = if gdtr_base >= 0xC0000000 {
            (gdtr_base - 0xC0000000) as usize
        } else {
            gdtr_base as usize
        };

        // Dump first 8 GDT entries (64 bytes) from physical RAM
        println!("GDT entries at physical {:#010x}:", gdt_phys_base);
        let num_entries = std::cmp::min((gdtr_limit as usize + 1) / 8, 32);
        for i in 0..num_entries {
            let entry_addr = gdt_phys_base + i * 8;
            let bytes = emu.peek_ram_at(entry_addr, 8);
            if bytes.len() == 8 {
                let dword1 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                let dword2 = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                // Decode base and limit
                let limit = (dword1 & 0xFFFF) | ((dword2 & 0x000F0000) as u32);
                let base = ((dword1 >> 16) as u64)
                    | (((dword2 & 0xFF) as u64) << 16)
                    | ((dword2 & 0xFF000000) as u64);
                let g = (dword2 & 0x00800000) != 0;
                let d_b = (dword2 & 0x00400000) != 0;
                let p = (dword2 >> 15) & 1;
                let dpl = (dword2 >> 13) & 3;
                let s = (dword2 >> 12) & 1;
                let ty = (dword2 >> 8) & 0xF;
                let limit_scaled = if g { (limit << 12) | 0xFFF } else { limit };
                println!("  GDT[{}] raw={:#010x}_{:#010x}  base={:#010x} limit={:#010x} P={} DPL={} S={} type={:#x} G={} D/B={}",
                    i, dword2, dword1, base, limit_scaled, p, dpl, s, ty, g as u8, d_b as u8);
            }
        }
        println!("===== END GDT DIAGNOSTIC =====");
    }

    // Cleanup: restore terminal if GUI was used
    if let Some(ref mut gui) = emu.gui_mut() {
        gui.exit();
    }

    // In headless mode, dump the current VGA text screen once at the end.
    // This avoids terminal repaint while still letting you see BIOS/VGABIOS output.
    if headless {
        // ATA diagnostic
        let (ata_reads, ata_writes) = emu.device_manager.ata_io_counts();
        println!("\n===== ATA DIAGNOSTIC =====");
        println!("ATA read_count={}, write_count={}", ata_reads, ata_writes);

        // IRQ delivery chain diagnostic
        println!("\n===== IRQ DELIVERY CHAIN =====");
        println!("tick() calls:          {}", emu.device_manager.diag_tick_count);
        println!("PIT fires (check_irq0): {}", emu.device_manager.diag_pit_fires);
        println!("IRQ0 latched (raise):   {}", emu.device_manager.diag_irq0_latched);
        println!("IRQ0 was already high:  {}", emu.device_manager.diag_irq0_already_high);
        println!("iac() calls:            {}", emu.device_manager.diag_iac_count);
        let pic_diag = emu.device_manager.pic_diag();
        println!("total usec:             {} ({:.2}s virtual)", emu.device_manager.diag_total_usec, emu.device_manager.diag_total_usec as f64 / 1_000_000.0);
        println!("avg usec/tick:          {:.1}", emu.device_manager.diag_total_usec as f64 / emu.device_manager.diag_tick_count as f64);
        println!("PIC state:              {}", pic_diag);
        // Print non-zero vector histogram entries
        print!("iac vectors:            ");
        for (v, &count) in emu.device_manager.diag_vector_hist.iter().enumerate() {
            if count > 0 {
                print!("0x{:02x}={} ", v, count);
            }
        }
        println!();
        println!("CPU state:              {}", emu.cpu.cpu_diag_string());
        println!("CPU RIP:                {:#x}", emu.cpu.rip());
        println!("CPU CS:                 {:#06x}", emu.cpu.get_cs_selector());
        println!("CPU ESP:                {:#x}", emu.cpu.esp());

        // Dump code at idle loop RIP
        let rip = emu.cpu.rip() as usize;
        let ram = emu.memory.peek_ram(0, 0); // Get empty slice to check if method works
        println!("\n===== IDLE LOOP CODE (RIP={:#x}) =====", rip);
        // Dump 128 bytes before and 64 bytes at RIP
        for off in (rip.saturating_sub(128)..rip + 64).step_by(16) {
            let bytes = emu.memory.peek_ram(off, 16);
            let hex: Vec<String> = bytes.iter().map(|b| format!("{:02x}", b)).collect();
            let marker = if off == rip { " <-- RIP" } else { "" };
            println!("  {:#010x}: {}{}", off, hex.join(" "), marker);
        }
        // Also show CPU state
        println!("  EAX={:#010x} EBX={:#010x} ECX={:#010x} EDX={:#010x}",
            emu.cpu.eax(), emu.cpu.ebx(), emu.cpu.ecx(), emu.cpu.edx());
        println!("  ESI={:#010x} EDI={:#010x} ESP={:#010x} EBP={:#010x}",
            emu.cpu.esi(), emu.cpu.edi(), emu.cpu.esp(), emu.cpu.ebp());
        println!("  CR0={:#010x} CS={:#06x}",
            emu.cpu.get_cr0_val(), emu.cpu.get_cs_selector());

        // Read IDT entry for vector 0x20 (timer interrupt)
        // Only do kernel diagnostics when CPU is in protected+paged mode (kernel loaded)
        'kernel_diag: {
        let cr0 = emu.cpu.get_cr0_val();
        if (cr0 & 0x80000001) != 0x80000001 {
            println!("\n  (CPU not in protected+paged mode, kernel diagnostics skipped)");
            break 'kernel_diag;
        }
        println!("\n===== IDT ENTRY FOR VECTOR 0x20 =====");
        let idtr_base = emu.cpu.get_idtr_base();
        let idtr_limit = emu.cpu.get_idtr_limit();
        println!("  IDTR.base={:#010x} IDTR.limit={:#06x}", idtr_base, idtr_limit);
        // IDT entry is at IDTR.base + 0x20*8
        // But IDTR.base is a linear address, we need physical.
        // For kernel, linear = physical + 0xC0000000, so physical = linear - 0xC0000000
        let idt_entry_laddr = idtr_base + 0x20 * 8;
        let idt_entry_paddr = (idt_entry_laddr as u32).wrapping_sub(0xC0000000) as usize;
        let idt_bytes = emu.memory.peek_ram(idt_entry_paddr, 8);
        if idt_bytes.len() < 8 {
            println!("  (IDT entry address {:#010x} is outside RAM)", idt_entry_paddr);
            break 'kernel_diag;
        }
        let dword1 = u32::from_le_bytes([idt_bytes[0], idt_bytes[1], idt_bytes[2], idt_bytes[3]]);
        let dword2 = u32::from_le_bytes([idt_bytes[4], idt_bytes[5], idt_bytes[6], idt_bytes[7]]);
        let handler_offset = (dword1 & 0xFFFF) | ((dword2 & 0xFFFF0000));
        let handler_selector = (dword1 >> 16) & 0xFFFF;
        let gate_type = (dword2 >> 8) & 0x1F;
        let gate_dpl = (dword2 >> 13) & 0x3;
        let gate_p = (dword2 >> 15) & 0x1;
        println!("  IDT[0x20] @ laddr={:#010x} paddr={:#010x}", idt_entry_laddr, idt_entry_paddr);
        println!("  dword1={:#010x} dword2={:#010x}", dword1, dword2);
        println!("  handler={:#010x} selector={:#06x} type={:#x} DPL={} P={}", handler_offset, handler_selector, gate_type, gate_dpl, gate_p);
        // Dump handler code — show 256 bytes to see the full handler
        // Handle both identity-mapped (low) and kernel virtual (0xC0xxxxxx) addresses
        let handler_phys = if handler_offset < 0x80000000 {
            handler_offset as usize
        } else {
            (handler_offset as u32).wrapping_sub(0xC0000000) as usize
        };
        println!("  handler_phys={:#010x} (< 0x2000000 = {})", handler_phys, handler_phys < 0x2000000);
        if handler_phys < 0x2000000 {
            println!("  --- TIMER HANDLER CODE (256 bytes from {:#010x}) ---", handler_offset);
            for chunk_start in (0..256).step_by(16) {
                let off = handler_phys + chunk_start;
                let handler_bytes = emu.memory.peek_ram(off, 16);
                let hex: Vec<String> = handler_bytes.iter().map(|b| format!("{:02x}", b)).collect();
                println!("  {:#010x}: {}", handler_offset as u32 + chunk_start as u32, hex.join(" "));
            }
        }

        // Check the ACTUAL address the timer handler increments
        // From disasm: ff 05 50 b6 19 00 = INC [0x0019b650]
        {
            // Read physical 0x19b650 directly (bypassing paging)
            let b = emu.memory.peek_ram(0x19b650, 4);
            let val = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            println!("  phys [0x0019b650] = {} ({:#x})", val, val);

            // Walk the page table manually to see where virtual 0x19b650 maps
            let cr3 = emu.cpu.get_cr3_val() as usize;
            println!("  CR3 = {:#010x}", cr3);

            // Check several interesting virtual addresses
            for &vaddr in &[0x0019b650u32, 0xC019b650u32, 0x0010b9c8u32, 0x0010ae7cu32] {
                let pde_idx = (vaddr >> 22) as usize;
                let pte_idx = ((vaddr >> 12) & 0x3FF) as usize;
                let page_off = (vaddr & 0xFFF) as usize;

                let pde_addr = cr3 + pde_idx * 4;
                let pde_b = emu.memory.peek_ram(pde_addr, 4);
                let pde = u32::from_le_bytes([pde_b[0], pde_b[1], pde_b[2], pde_b[3]]);

                if pde & 1 == 0 {
                    println!("  vaddr {:#010x}: PDE[{}]={:#010x} NOT PRESENT", vaddr, pde_idx, pde);
                    continue;
                }

                // Check for 4MB PSE page
                if pde & 0x80 != 0 {
                    let phys = ((pde & 0xFFC00000) as usize) | ((vaddr & 0x003FFFFF) as usize);
                    let v = emu.memory.peek_ram(phys, 4);
                    let val = u32::from_le_bytes([v[0], v[1], v[2], v[3]]);
                    println!("  vaddr {:#010x}: 4MB page PDE[{}]={:#010x} → phys {:#010x} = {} ({:#x})",
                        vaddr, pde_idx, pde, phys, val, val);
                    continue;
                }

                // 4KB page: read PTE
                let pt_base = (pde & 0xFFFFF000) as usize;
                let pte_addr = pt_base + pte_idx * 4;
                let pte_b = emu.memory.peek_ram(pte_addr, 4);
                let pte = u32::from_le_bytes([pte_b[0], pte_b[1], pte_b[2], pte_b[3]]);

                if pte & 1 == 0 {
                    println!("  vaddr {:#010x}: PDE[{}]={:#010x} PTE[{}]={:#010x} NOT PRESENT",
                        vaddr, pde_idx, pde, pte_idx, pte);
                    continue;
                }

                let phys = ((pte & 0xFFFFF000) as usize) | page_off;
                let v = emu.memory.peek_ram(phys, 4);
                let val = u32::from_le_bytes([v[0], v[1], v[2], v[3]]);
                println!("  vaddr {:#010x}: PDE[{}]={:#010x} PTE[{}]={:#010x} → phys {:#010x} = {} ({:#x})",
                    vaddr, pde_idx, pde, pte_idx, pte, phys, val, val);
            }
        }

        // Dump do_IRQ function at 0x10cb88 and timer_interrupt at 0x10f7b0
        {
            println!("\n===== do_IRQ CODE (0x10cb88) =====");
            for chunk in (0x10cb88u32..0x10cd00).step_by(16) {
                let b = emu.memory.peek_ram(chunk as usize, 16);
                let hex: Vec<String> = b.iter().map(|b| format!("{:02x}", b)).collect();
                println!("  {:#010x}: {}", chunk, hex.join(" "));
            }
            println!("\n===== timer_interrupt CODE (0x10f7b0) =====");
            for chunk in (0x10f7b0u32..0x10f830).step_by(16) {
                let b = emu.memory.peek_ram(chunk as usize, 16);
                let hex: Vec<String> = b.iter().map(|b| format!("{:02x}", b)).collect();
                println!("  {:#010x}: {}", chunk, hex.join(" "));
            }
            println!("\n===== do_timer CODE (0x111dd4) =====");
            for chunk in (0x111dd4u32..0x111f00).step_by(16) {
                let b = emu.memory.peek_ram(chunk as usize, 16);
                let hex: Vec<String> = b.iter().map(|b| format!("{:02x}", b)).collect();
                println!("  {:#010x}: {}", chunk, hex.join(" "));
            }
        }

        // Check critical kernel data structures
        {
            let b = emu.memory.peek_ram(0x19b650, 4);
            let intr_count = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            println!("\n  intr_count @ phys 0x19b650 = {}", intr_count);

            // irq_action[0] pointer (from do_IRQ: MOV EBX, [ESI*4 + 0x197878])
            // ESI=0 for IRQ0, so address is 0x197878 (phys, via DS.base=0xC0000000)
            let b = emu.memory.peek_ram(0x197878, 4);
            let irq_action_0 = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            println!("  irq_action[0] @ phys 0x197878 = {:#010x}", irq_action_0);

            // kstat.interrupts[0] (from do_IRQ: INC [ESI*4 + 0x19b454])
            let b = emu.memory.peek_ram(0x19b454, 4);
            let kstat_irq0 = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            println!("  kstat.interrupts[0] @ phys 0x19b454 = {}", kstat_irq0);

            // If irq_action_0 != 0, dump the struct irqaction
            if irq_action_0 != 0 {
                // The pointer is stored as a kernel virtual address.
                // If it looks like 0xC0XXXXXX, subtract PAGE_OFFSET.
                // If it looks like 0x00XXXXXX, the kernel might use identity mapping.
                let phys_action = if irq_action_0 >= 0xC0000000 {
                    (irq_action_0 - 0xC0000000) as usize
                } else {
                    irq_action_0 as usize
                };
                if phys_action < 0x2000000 {
                    println!("  irqaction struct @ phys {:#010x}:", phys_action);
                    let action = emu.memory.peek_ram(phys_action, 24);
                    for i in (0..24).step_by(4) {
                        let val = u32::from_le_bytes([action[i], action[i+1], action[i+2], action[i+3]]);
                        println!("    +{:#04x}: {:#010x}", i, val);
                    }
                    // Also show the handler function pointer (first dword)
                    let handler_ptr = u32::from_le_bytes([action[0], action[1], action[2], action[3]]);
                    println!("  handler = {:#010x} (phys={:#010x})",
                        handler_ptr,
                        if handler_ptr >= 0xC0000000 { handler_ptr - 0xC0000000 } else { handler_ptr });
                } else {
                    println!("  irqaction phys_addr {:#010x} out of range!", phys_action);
                }
            }

            // Check jiffies (REAL: 0x19abe0 from do_timer disasm) and related
            for &(addr, label) in &[
                (0x19abe0u32, "jiffies (from do_timer INC)"),
                (0x19b4d8, "lost_ticks"),
                (0x19b654, "timer_active bitmask"),
                (0x10ae7cu32, "old candidate (NOT jiffies)"),
                (0x19b454, "kstat.interrupts[0]"),
                (0x19b650, "intr_count"),
            ] {
                let b = emu.memory.peek_ram(addr as usize, 4);
                let val = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                println!("  phys {:#010x} = {:>8} ({:#x})  {}", addr, val, val, label);
            }
        }

        // ATA controller state
        {
            println!("\n===== ATA CONTROLLER STATE =====");
            println!("{}", emu.device_manager.ata_diag());
        }

        // Search for jiffies — look for a u32 that's a reasonable timer count
        // With IPS=4M, 220M instructions, PIT at 100Hz ≈ 5500 ticks expected
        for addr in (0x106000..0x10c000).step_by(4) {
            let b = emu.memory.peek_ram(addr, 4);
            let val = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            if val > 100 && val < 100000 {
                // Also check if addr+4 could be need_resched (typically 0 or 1)
                let b2 = emu.memory.peek_ram(addr + 4, 4);
                let next = u32::from_le_bytes([b2[0], b2[1], b2[2], b2[3]]);
                if next <= 1 {
                    println!("  Possible jiffies at {:#x} = {}, next_word={}", addr, val, next);
                }
            }
        }

        // Dump where the kernel is spinning — show RIP and surrounding code
        {
            let rip = emu.cpu.rip() as u32;
            let rip_phys = rip.wrapping_sub(0xC0000000) as usize;
            println!("\n===== KERNEL SPIN LOCATION =====");
            println!("  RIP={:#010x} (phys={:#010x})", rip, rip_phys);
            println!("  async_event={} activity={:?}", emu.cpu.get_async_event(), emu.cpu.get_activity_state());
            if rip_phys < 0x2000000 {
                let code = emu.memory.peek_ram(rip_phys.saturating_sub(16), 48);
                let hex: Vec<String> = code.iter().map(|b| format!("{:02x}", b)).collect();
                println!("  Code @ phys {:#010x}: {}", rip_phys.saturating_sub(16), hex.join(" "));
            }
            // Check PIC masks
            println!("  PIC: {}", emu.device_manager.pic_diag());
            // Show async event state
            println!("  async_event={} activity={:?}",
                emu.cpu.get_async_event(), emu.cpu.get_activity_state());
        }
        } // end 'kernel_diag block

        println!();
        println!("===== CPU HANDLE_ASYNC_EVENT INTR DELIVERY =====");
        let (delivered, if_blocked, no_pic, pic_empty) = emu.cpu.get_hae_intr_diag();
        println!("  delivered={} if_blocked={} no_pic={} pic_empty={}",
            delivered, if_blocked, no_pic, pic_empty);

        println!();
        println!("===== INTERRUPT CHAIN DIAGNOSTIC =====");
        println!("{}", emu.device_manager.interrupt_chain_diag());

        println!();
        println!("===== VGA TEXT DUMP (headless) =====");
        println!("{}", emu.vga_text_dump());

        // BIOS POST codes: show count and last 32 bytes only
        let post = emu.devices.take_port80_output();
        if !post.is_empty() {
            print!(
                "===== BIOS POST CODES (port 0x80/0x84): {} total =====\n",
                post.len()
            );
            let start = post.len().saturating_sub(32);
            for (i, b) in post[start..].iter().enumerate() {
                if i != 0 && (i % 16) == 0 {
                    print!("\n");
                }
                print!("{:02x} ", b);
            }
            print!("\n");
        }
    }

    Ok(())
}
