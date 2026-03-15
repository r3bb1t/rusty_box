//! Alpine Linux Direct Kernel Boot
//!
//! Boots Alpine Linux by loading the kernel and initramfs directly from the ISO,
//! bypassing ISOLINUX/SYSLINUX entirely. This is equivalent to QEMU's `-kernel`
//! and `-initrd` options.
//!
//! ## Usage
//!
//! ```bash
//! cargo run --release --example alpine_direct --features std
//!
//! # Custom ISO path (default: alpine-virt-3.23.3-x86_64.iso)
//! ALPINE_ISO=/path/to/alpine.iso cargo run --release --example alpine_direct --features std
//!
//! # Custom RAM size (default: 128 MB)
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
        .name("Alpine Direct Boot".to_string())
        .spawn(|| {
            if let Err(e) = run_alpine_direct() {
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
    if pvd_offset + 190 > iso_data.len() {
        return None;
    }

    // Root directory record is at PVD offset 156
    let root_lba = u32::from_le_bytes([
        iso_data[pvd_offset + 158],
        iso_data[pvd_offset + 159],
        iso_data[pvd_offset + 160],
        iso_data[pvd_offset + 161],
    ]) as usize;
    let root_size = u32::from_le_bytes([
        iso_data[pvd_offset + 166],
        iso_data[pvd_offset + 167],
        iso_data[pvd_offset + 168],
        iso_data[pvd_offset + 169],
    ]) as usize;

    fn find_entry(iso: &[u8], dir_lba: usize, dir_size: usize, name: &str) -> Option<(usize, usize, bool)> {
        let dir_data_start = dir_lba * 2048;
        let dir_data_end = dir_data_start + dir_size;
        if dir_data_end > iso.len() {
            return None;
        }
        let dir_data = &iso[dir_data_start..dir_data_end];
        let mut offset = 0;
        while offset < dir_data.len() {
            let rec_len = dir_data[offset] as usize;
            if rec_len == 0 {
                let next_sect = ((offset / 2048) + 1) * 2048;
                if next_sect >= dir_data.len() {
                    break;
                }
                offset = next_sect;
                continue;
            }
            if offset + 33 > dir_data.len() {
                break;
            }
            let name_len = dir_data[offset + 32] as usize;
            if offset + 33 + name_len > dir_data.len() {
                break;
            }
            let entry_name = std::str::from_utf8(&dir_data[offset + 33..offset + 33 + name_len])
                .unwrap_or("");
            let entry_lba = u32::from_le_bytes([
                dir_data[offset + 2], dir_data[offset + 3],
                dir_data[offset + 4], dir_data[offset + 5],
            ]) as usize;
            let entry_size = u32::from_le_bytes([
                dir_data[offset + 10], dir_data[offset + 11],
                dir_data[offset + 12], dir_data[offset + 13],
            ]) as usize;
            let is_dir = (dir_data[offset + 25] & 2) != 0;

            // ISO 9660 names may have ";1" version suffix
            let clean_name = entry_name.split(';').next().unwrap_or(entry_name);
            if clean_name.eq_ignore_ascii_case(name) {
                return Some((entry_lba, entry_size, is_dir));
            }
            offset += rec_len;
        }
        None
    }

    // Navigate path components
    let mut cur_lba = root_lba;
    let mut cur_size = root_size;
    for (i, component) in target_path.iter().enumerate() {
        let is_last = i == target_path.len() - 1;
        match find_entry(iso_data, cur_lba, cur_size, component) {
            Some((lba, size, is_dir)) => {
                if is_last {
                    // Extract file
                    let start = lba * 2048;
                    let end = start + size;
                    if end <= iso_data.len() {
                        return Some(iso_data[start..end].to_vec());
                    }
                    return None;
                }
                if !is_dir {
                    return None; // Expected directory but found file
                }
                cur_lba = lba;
                cur_size = size;
            }
            None => return None,
        }
    }
    None
}

fn run_alpine_direct() -> Result<()> {
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
        .unwrap_or(2_000_000_000);

    // Default command line: serial console + Alpine init
    let cmdline = std::env::var("CMDLINE").unwrap_or_else(|_|
        "console=ttyS0,115200 earlycon=uart8250,io,0x3f8,115200n8 earlyprintk=serial,ttyS0,115200 nomodeset nokaslr kfence.sample_interval=0 modules=cdrom,sr_mod,isofs".to_string()
    );

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
    // Read ISO and extract kernel + initramfs
    // =========================================================================
    println!("Reading ISO: {}", iso_path);
    let iso_data = std::fs::read(&iso_path)
        .unwrap_or_else(|e| {
            eprintln!("Failed to read ISO file '{}': {}", iso_path, e);
            eprintln!("Set ALPINE_ISO=/path/to/alpine-virt-*.iso");
            std::process::exit(1);
        });
    println!("  ISO size: {} MB", iso_data.len() / 1024 / 1024);

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

    // =========================================================================
    // Create and initialize emulator
    // =========================================================================
    let ram_bytes = ram_mb * 1024 * 1024;
    let config = EmulatorConfig {
        guest_memory_size: ram_bytes,
        host_memory_size: ram_bytes,
        ips: 15_000_000,
        ..EmulatorConfig::default()
    };

    println!("Creating emulator with {} MB RAM...", ram_mb);
    let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;

    // Initialize memory + PC system
    emu.init_memory_and_pc_system()?;

    // Initialize CPU + devices (needed for PIC, PIT, serial, etc.)
    emu.init_cpu_and_devices()?;

    // Configure CMOS memory
    emu.configure_memory_in_cmos_from_config();

    // Attach ISO as CD-ROM so Alpine Init can mount the squashfs root filesystem
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

    // Do a normal reset first (initializes all device state)
    emu.reset(ResetReason::Hardware)?;

    // =========================================================================
    // Set up direct kernel boot (overrides CPU state from reset)
    // =========================================================================
    println!("Setting up direct kernel boot...");
    println!("  Command line: {}", cmdline);
    emu.setup_direct_linux_boot(&vmlinuz, Some(&initramfs), &cmdline)?;

    emu.init_gui_signal_handlers();
    emu.start();

    println!();
    println!("Direct boot: EIP={:#010x} ESI={:#010x}", emu.cpu.rip(), emu.cpu.rsi());
    println!("Starting Alpine Linux kernel...");
    println!();

    // =========================================================================
    // Execution loop
    // =========================================================================
    let start_time = Instant::now();
    let mut total_executed: u64 = 0;

    const PHASE_SIZE: u64 = 500_000;
    let mut last_serial_drain = Instant::now();

    loop {
        if total_executed >= max_instructions {
            break;
        }
        let run_for = PHASE_SIZE.min(max_instructions - total_executed);
        match emu.run_interactive(run_for) {
            Ok(n) => {
                total_executed += n;
                if n == 0 {
                    let (act, ae) = emu.cpu_diag_state();
                    eprintln!("[EXIT] run_interactive returned 0 at total={} RIP={:#x} activity={} async_event={} IF={}",
                        total_executed, emu.cpu.rip(), act, ae, emu.cpu.interrupts_enabled());
                    break;
                }
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
            eprintln!(
                "[{:>4}M instr, {:.1}s, {:.1} MIPS] RIP={:#010x} mode={} IO[r={} w={}] ATA[ch0={} ch1={}]",
                total_executed / 1_000_000,
                elapsed,
                mips,
                emu.cpu.rip(),
                emu.get_cpu_mode_str(),
                io_r, io_w,
                ata_ch0,
                ata_ch1,
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
