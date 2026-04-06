//! Alpine Linux Boot (Direct Kernel or BIOS)
//!
//! Boots Alpine Linux either by loading the kernel and initramfs directly from
//! the ISO (direct boot), or by running the full BIOS POST → ISOLINUX → kernel
//! chain (BIOS boot). Default is BIOS boot.
//!
//! ## Usage
//!
//! ```bash
//! # BIOS boot (default) — full BIOS POST, ISOLINUX, kernel boot
//! cargo run --release --example alpine_direct --features std
//!
//! # Direct kernel boot — bypass BIOS/ISOLINUX, load kernel directly
//! RUSTY_BOX_BOOT=direct cargo run --release --example alpine_direct --features std
//!
//! # Custom ISO path (default: alpine-virt-3.23.3-x86_64.iso)
//! ALPINE_ISO=/path/to/alpine.iso cargo run --release --example alpine_direct --features std
//!
//! # Custom RAM size (default: 256 MB)
//! ALPINE_RAM_MB=256 cargo run --release --example alpine_direct --features std
//!
//! # Headless mode
//! RUSTY_BOX_HEADLESS=1 cargo run --release --example alpine_direct --features std
//! ```

use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, ResetReason},
    emulator::{Emulator, EmulatorConfig},
    gui::{NoGui, TermGui},
    Result,
};
use std::time::Instant;

fn main() {
    const THREAD_STACK_SIZE: usize = 1500 * 1024 * 1024;

    std::thread::Builder::new()
        .stack_size(THREAD_STACK_SIZE)
        .name("Alpine Boot".to_string())
        .spawn(|| {
            if let Err(e) = run_alpine() {
                eprintln!("Error: {:?}", e);
                std::process::exit(1);
            }
        })
        .expect("Failed to spawn thread")
        .join()
        .expect("Thread panicked");
}

/// Extract a file from an ISO 9660 filesystem by name.
/// Returns the file contents or None if not found.
fn extract_from_iso(iso_data: &[u8], target_path: &[&str]) -> Option<Vec<u8>> {
    // Parse Primary Volume Descriptor at sector 16
    let pvd_offset = 16 * 2048;
    if iso_data.len() < pvd_offset + 2048 {
        return None;
    }
    let pvd = &iso_data[pvd_offset..pvd_offset + 2048];

    // Check PVD signature: \x01CD001
    if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
        return None;
    }

    // Root directory record at offset 156 (34 bytes)
    let root_record = &pvd[156..156 + 34];
    let root_lba = u32::from_le_bytes([
        root_record[2],
        root_record[3],
        root_record[4],
        root_record[5],
    ]);
    let root_len = u32::from_le_bytes([
        root_record[10],
        root_record[11],
        root_record[12],
        root_record[13],
    ]);

    // Navigate directory tree
    let mut current_lba = root_lba;
    let mut current_len = root_len;

    for (depth, &name) in target_path.iter().enumerate() {
        let is_file = depth == target_path.len() - 1;
        let dir_offset = current_lba as usize * 2048;
        if dir_offset + current_len as usize > iso_data.len() {
            return None;
        }
        let dir_data = &iso_data[dir_offset..dir_offset + current_len as usize];

        let mut pos = 0;
        let mut found = false;

        while pos < dir_data.len() {
            let record_len = dir_data[pos] as usize;
            if record_len == 0 {
                // Move to next sector boundary
                let next_sector = ((pos / 2048) + 1) * 2048;
                if next_sector >= dir_data.len() {
                    break;
                }
                pos = next_sector;
                continue;
            }
            if pos + record_len > dir_data.len() {
                break;
            }

            let name_len = dir_data[pos + 32] as usize;
            if name_len > 0 && pos + 33 + name_len <= dir_data.len() {
                let entry_name =
                    String::from_utf8_lossy(&dir_data[pos + 33..pos + 33 + name_len]);
                let entry_name_upper = entry_name.to_uppercase();

                if entry_name_upper.starts_with(name) {
                    let entry_lba = u32::from_le_bytes([
                        dir_data[pos + 2],
                        dir_data[pos + 3],
                        dir_data[pos + 4],
                        dir_data[pos + 5],
                    ]);
                    let entry_len = u32::from_le_bytes([
                        dir_data[pos + 10],
                        dir_data[pos + 11],
                        dir_data[pos + 12],
                        dir_data[pos + 13],
                    ]);

                    if is_file {
                        // Extract file
                        let file_offset = entry_lba as usize * 2048;
                        if file_offset + entry_len as usize <= iso_data.len() {
                            return Some(
                                iso_data[file_offset..file_offset + entry_len as usize].to_vec(),
                            );
                        }
                        return None;
                    } else {
                        // Enter directory
                        current_lba = entry_lba;
                        current_len = entry_len;
                        found = true;
                        break;
                    }
                }
            }
            pos += record_len;
        }
        if !found && !is_file {
            return None;
        }
    }
    None
}

/// Find a file by checking multiple candidate paths.
fn find_file(candidates: &[&str]) -> Option<(String, Vec<u8>)> {
    for path in candidates {
        if let Ok(data) = std::fs::read(path) {
            return Some((path.to_string(), data));
        }
    }
    None
}

fn run_alpine() -> Result<()> {
    // =========================================================================
    // Configuration
    // =========================================================================
    let iso_path = std::env::var("ALPINE_ISO")
        .unwrap_or_else(|_| "alpine-virt-3.23.3-x86_64.iso".to_string());

    let ram_mb: usize = std::env::var("ALPINE_RAM_MB")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(256);

    let headless = std::env::var("RUSTY_BOX_HEADLESS").is_ok();

    let max_instructions: u64 = std::env::var("MAX_INSTRUCTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4_000_000_000);

    // Boot mode: "bios" (default) or "direct"
    let boot_mode = std::env::var("RUSTY_BOX_BOOT")
        .unwrap_or_else(|_| "bios".to_string());
    let bios_boot = boot_mode != "direct";

    // =========================================================================
    // Setup logging
    // =========================================================================
    let log_level: tracing::Level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(tracing::Level::WARN);

    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_target(false)
        .init();

    // =========================================================================
    // Read ISO
    // =========================================================================
    println!("Reading ISO: {}", iso_path);
    let iso_data = std::fs::read(&iso_path)
        .unwrap_or_else(|e| {
            eprintln!("Failed to read ISO file '{}': {}", iso_path, e);
            eprintln!("Set ALPINE_ISO=/path/to/alpine-virt-*.iso");
            std::process::exit(1);
        });
    println!("  ISO size: {} MB", iso_data.len() / 1024 / 1024);

    // =========================================================================
    // Create and initialize emulator
    // =========================================================================
    let ram_bytes = ram_mb * 1024 * 1024;
    let config = EmulatorConfig {
        guest_memory_size: ram_bytes,
        host_memory_size: ram_bytes,
        ips: 300_000_000,
        pci_enabled: true,
        ..EmulatorConfig::default()
    };

    println!("Creating emulator with {} MB RAM (boot mode: {})...", ram_mb, boot_mode);
    let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;

    // Initialize memory + PC system
    emu.init_memory_and_pc_system()?;

    if bios_boot {
        // =====================================================================
        // BIOS Boot Path
        // =====================================================================
        // Find and load BIOS
        let workspace_root = std::env::current_dir().unwrap_or_default();
        let ws = workspace_root.to_string_lossy();
        let bios_candidates = [
            format!("{}/cpp_orig/bochs/bios/BIOS-bochs-latest", ws),
            format!("{}/../cpp_orig/bochs/bios/BIOS-bochs-latest", ws),
            "cpp_orig/bochs/bios/BIOS-bochs-latest".to_string(),
        ];
        let bios_strs: Vec<&str> = bios_candidates.iter().map(|s| s.as_str()).collect();
        let (bios_path, bios_data) = find_file(&bios_strs)
            .expect("Could not find BIOS-bochs-latest");
        println!("  BIOS loaded: {} bytes ({})", bios_data.len(), bios_path);

        let bios_size = bios_data.len() as u64;
        let bios_load_addr = !(bios_size - 1);
        emu.load_bios(&bios_data, bios_load_addr)?;

        // Find and load VGA BIOS
        let vga_candidates = [
            format!("{}/binaries/bios/VGABIOS-lgpl-latest.bin", ws),
            format!("{}/../binaries/bios/VGABIOS-lgpl-latest.bin", ws),
            "binaries/bios/VGABIOS-lgpl-latest.bin".to_string(),
        ];
        let vga_strs: Vec<&str> = vga_candidates.iter().map(|s| s.as_str()).collect();
        if let Some((vga_path, vga_data)) = find_file(&vga_strs) {
            emu.load_optional_rom(&vga_data, 0xC0000)?;
            println!("  VGA BIOS loaded: {} bytes ({})", vga_data.len(), vga_path);
        }

        // Initialize CPU + devices
        emu.init_cpu_and_devices()?;

        // Configure for CD-ROM boot
        emu.configure_memory_in_cmos_from_config();
        emu.configure_boot_sequence(3, 0, 0); // CD-ROM first

        // Attach ISO as CD-ROM
        emu.attach_cdrom(1, 0, &iso_path)
            .expect("Failed to attach Alpine ISO as CD-ROM");
        println!("  CD-ROM attached: {}", iso_path);

        // Initialize GUI
        if headless {
            emu.set_gui(NoGui::new());
        } else {
            emu.set_gui(TermGui::new());
        }
        emu.init_gui(0, &[])?;

        // Reset and start
        emu.reset(ResetReason::Hardware)?;
        emu.init_gui_signal_handlers();
        emu.start();
        emu.prepare_run();

        println!("  Boot: BIOS POST → ISOLINUX → kernel");
    } else {
        // =====================================================================
        // Direct Kernel Boot Path
        // =====================================================================
        let vmlinuz = extract_from_iso(&iso_data, &["BOOT", "VMLINUZ_VIRT."])
            .unwrap_or_else(|| {
                eprintln!("Failed to find BOOT/VMLINUZ_VIRT in ISO");
                std::process::exit(1);
            });
        println!("  Kernel: {} bytes", vmlinuz.len());

        let initramfs = extract_from_iso(&iso_data, &["BOOT", "INITRAMFS_VIRT."])
            .unwrap_or_else(|| {
                eprintln!("Failed to find BOOT/INITRAMFS_VIRT in ISO");
                std::process::exit(1);
            });
        println!("  Initramfs: {} bytes ({} MB)", initramfs.len(), initramfs.len() / 1024 / 1024);

        let cmdline = std::env::var("CMDLINE").unwrap_or_else(|_|
            "console=ttyS0,115200 earlycon=uart8250,io,0x3f8,115200n8 earlyprintk=serial,ttyS0,115200 nomodeset nokaslr kfence.sample_interval=0 modules=loop,squashfs,cdrom,sr_mod,isofs modloop=/boot/modloop-virt".to_string()
        );

        // Initialize CPU + devices
        emu.init_cpu_and_devices()?;
        emu.configure_memory_in_cmos_from_config();

        // Attach ISO as CD-ROM
        emu.attach_cdrom(1, 0, &iso_path)
            .expect("Failed to attach Alpine ISO as CD-ROM");
        println!("  CD-ROM attached: {}", iso_path);

        // Initialize GUI
        if headless {
            emu.set_gui(NoGui::new());
        } else {
            emu.set_gui(TermGui::new());
        }
        emu.init_gui(0, &[])?;

        // Reset and set up direct boot
        emu.reset(ResetReason::Hardware)?;
        emu.init_vga_text_mode3();

        println!("  Command line: {}", cmdline);
        emu.setup_direct_linux_boot(&vmlinuz, Some(&initramfs), &cmdline)?;

        emu.init_gui_signal_handlers();
        emu.start();

        println!("  Boot: direct kernel (EIP={:#010x})", emu.cpu.rip());
    }

    println!("Starting Alpine Linux (max {} instructions)...\n", max_instructions);

    // =========================================================================
    // =========================================================================
    // Instrumentation: awk field-splitting debug hook
    // =========================================================================
    #[cfg(feature = "bx_instrumentation")]
    {
        use rusty_box::cpu::{CpuSnapshot, Instrumentation};

        /// Traces awk_split FS dispatch to find why field splitting fails.
        /// Watches for CmpEbIb (FS==space check) and TestAlib (Phase 2 whitespace)
        /// by matching decoded opcode + register values.
        struct AwkFieldSplitTracer {
            hits: u32,
        }
        impl Instrumentation for AwkFieldSplitTracer {
            fn before_execution(&mut self, rip: u64, opcode: u16, _ilen: u8, snap: &CpuSnapshot) {
                if snap.icount < 3_000_000_000 || rip < 0x400000 { return; }
                if self.hits >= 100 { return; }

                let al = (snap.rax & 0xFF) as u8;
                let bpl = (snap.rbp & 0xFF) as u8;

                // Print actual opcode values on first call for verification
                if self.hits == 0 {
                    let test_alib_val = rusty_box_decoder::opcode::Opcode::TestAlib as u16;
                    let cmp_ebib_val = rusty_box_decoder::opcode::Opcode::CmpEbIb as u16;
                    let cmp_alib_val = rusty_box_decoder::opcode::Opcode::CmpAlib as u16;
                    eprintln!("[INSTR-VALS] TestAlib={} CmpEbIb={} CmpAlib={} cur={}",
                        test_alib_val, cmp_ebib_val, cmp_alib_val, opcode);
                    self.hits = 1;
                }
                // Match opcodes
                if (opcode == 42 || opcode == 70 || opcode == 38) && self.hits < 30 {
                    let ch = if al >= 0x20 && al < 0x7f { al as char } else { '.' };
                    eprintln!("[INSTR] op={} RIP={:#x} AL={:#04x} '{}' BPL={:#04x} i={}",
                        opcode, rip, al, ch, bpl, snap.icount);
                    self.hits += 1;
                }
            }
        }
        emu.cpu_mut().set_instrumentation(Box::new(AwkFieldSplitTracer { hits: 0 }));
        println!("Instrumentation: AwkFieldSplitTracer installed");
    }

    // Execution loop
    // =========================================================================
    let start_time = Instant::now();
    let mut total_executed: u64 = 0;

    const PHASE_SIZE: u64 = 500_000;
    let mut last_serial_drain = Instant::now();

    // BIOS boot: inject Enter key at ISOLINUX prompt (~18M instructions)
    // Enter scancode: PS/2 set 2 — make=0x5A, break=0xF0 0x5A
    let mut enter_injected = !bios_boot; // skip for direct boot


    loop {
        if total_executed >= max_instructions {
            break;
        }
        let run_for = PHASE_SIZE.min(max_instructions - total_executed);
        match emu.run_interactive(run_for) {
            Ok(n) => {
                total_executed += n;
            }
            Err(e) => {
                eprintln!("CPU error at {} instructions: {:?}", total_executed, e);
                break;
            }
        }

        // Check for shutdown
        if emu.cpu.is_in_shutdown() {
            println!("CPU shutdown at {} instructions", total_executed);
            break;
        }

        // BIOS boot: type the full kernel cmdline at ISOLINUX prompt.
        // Typing at the prompt REPLACES the syslinux.cfg APPEND line,
        // so we must include everything from the original APPEND plus
        // console= for serial output. The original APPEND is:
        //   modules=loop,squashfs,sd-mod,usb-storage quiet
        // We add console=ttyS0,115200 and drop quiet for visibility.
        if !enter_injected && total_executed >= 18_000_000 {
            println!("[{}M] Typing cmdline at ISOLINUX boot prompt", total_executed / 1_000_000);
            emu.send_string("virt modules=loop,squashfs,sd-mod,usb-storage console=ttyS0,115200\n");
            enter_injected = true;
        }



        // Drain serial port output periodically
        if last_serial_drain.elapsed().as_millis() >= 100 {
            let output: Vec<u8> = emu.device_manager.drain_serial_tx(0).collect();
            if !output.is_empty() {
                use std::io::Write;
                let mut stdout = std::io::stdout();
                stdout.write_all(&output).ok();
                stdout.flush().ok();
            }
            last_serial_drain = Instant::now();
        }

        // Progress report every 50M instructions
        if total_executed % 50_000_000 < PHASE_SIZE {
            let elapsed = start_time.elapsed().as_secs_f64();
            let mips = total_executed as f64 / elapsed / 1_000_000.0;
            let (ata_ch0, ata_ch1) = emu.ata_diag_reads();
            let (io_r, io_w) = emu.io_diag_counts();
            let ata1_info = emu.ata_ch1_diag();
            eprintln!(
                "[{:>4}M instr, {:.1}s, {:.1} MIPS] RIP={:#010x} mode={} IO[r={} w={}] ATA[ch0={} ch1={}] {}",
                total_executed / 1_000_000,
                elapsed,
                mips,
                emu.cpu.rip(),
                emu.get_cpu_mode_str(),
                io_r, io_w,
                ata_ch0,
                ata_ch1,
                ata1_info,
            );
        }
    }

    // Final serial drain
    let output: Vec<u8> = emu.device_manager.drain_serial_tx(0).collect();
    if !output.is_empty() {
        use std::io::Write;
        std::io::stdout().write_all(&output).ok();
        std::io::stdout().flush().ok();
    }

    // Drain port 0xE9 output (Bochs debug port — used by kernel decompressor __putstr)
    let e9 = emu.devices.take_port_e9_output();
    if !e9.is_empty() {
        println!("\n--- Port 0xE9 (kernel decompressor) ---");
        let s = String::from_utf8_lossy(&e9);
        println!("{}", s);
    }

    let elapsed = start_time.elapsed();
    println!();
    println!(
        "Executed {} instructions in {:.2}s ({:.1} MIPS)",
        total_executed,
        elapsed.as_secs_f64(),
        total_executed as f64 / elapsed.as_secs_f64() / 1_000_000.0,
    );

    emu.dump_alpine_diag();

    // VGA text dump
    let vga = emu.vga_text_dump();
    if !vga.trim().is_empty() {
        println!("\n--- VGA Text ---");
        println!("{}", vga);
    }

    Ok(())
}
