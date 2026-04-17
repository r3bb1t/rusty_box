//! Alpine Linux boot with strace-style syscall tracing.
//!
//! Boots Alpine from ISO (BIOS path by default, or `RUSTY_BOX_BOOT=direct` for
//! direct kernel+initramfs). Every SYSCALL is intercepted via the `pre_syscall`
//! hook, decoded with string argument resolution, and logged to `strace.log`
//! (keeps the VGA terminal clean). The SYSCALL executes architecturally so the
//! Linux kernel actually services it — the hook only observes.
//!
//! ```bash
//! cargo run --release --example alpine_strace --features "std,instrumentation"
//! ```
//!
//! Env:
//! - `ALPINE_ISO`         — path to Alpine virt ISO
//! - `ALPINE_RAM_MB`      — RAM (default 256)
//! - `MAX_INSTRUCTIONS`   — cap (default 4e9)
//! - `STRACE_LOG`         — output file (default `strace.log`)
//! - `RUSTY_BOX_BOOT`     — `direct` skips BIOS/ISOLINUX
//! - `RUSTY_BOX_HEADLESS` — set → no GUI

#![cfg(all(feature = "std", feature = "instrumentation"))]

use rusty_box::{
    cpu::{
        core_i7_skylake::Corei7SkylakeX,
        HookCtx, HookMask, InstrAction, Instrumentation, ResetReason, X86Reg,
    },
    emulator::{Emulator, EmulatorConfig},
    gui::{NoGui, TermGui},
    Result,
};
use std::time::Instant;

mod syscalls;

#[derive(Default)]
pub struct StraceTracer {
    icount: u64,
}

impl Instrumentation for StraceTracer {
    fn active_hooks(&self) -> HookMask {
        HookMask::EXEC
    }

    fn before_execution(&mut self, _rip: u64, _instr: &rusty_box::cpu::decoder::Instruction) {
        self.icount = self.icount.saturating_add(1);
    }

    fn pre_syscall(&mut self, ctx: &mut HookCtx) -> InstrAction {
        // Read raw registers. This example uses the SysV AMD64 convention
        // (RAX = nr, args in RDI/RSI/RDX/R10/R8/R9) because that's what Linux
        // userspace uses. Rusty_box itself assumes no OS.
        let nr = ctx.reg_read(X86Reg::Rax);
        let args = [
            ctx.reg_read(X86Reg::Rdi),
            ctx.reg_read(X86Reg::Rsi),
            ctx.reg_read(X86Reg::Rdx),
            ctx.reg_read(X86Reg::R10),
            ctx.reg_read(X86Reg::R8),
            ctx.reg_read(X86Reg::R9),
        ];

        // Read NUL-terminated strings from user memory. `ctx.cr3()` is the
        // user's CR3 because the hook runs BEFORE the CS/RIP transition.
        let user_cr3 = ctx.cr3();
        let strings: [Option<String>; 6] = core::array::from_fn(|i| {
            let addr = args[i];
            if addr == 0 || addr >= 0x8000_0000_0000_0000 {
                return None;
            }
            let mut buf = [0u8; 128];
            if !ctx.virt_read_with_cr3(addr, user_cr3, &mut buf) {
                return None;
            }
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let s = String::from_utf8_lossy(&buf[..end]).into_owned();
            Some(truncate_str(s, 64))
        });

        let mut no_fallback = |addr: u64| format!("{addr:#x}");
        let decoded = syscalls::Syscall::decode_x86_64(nr, args, &strings, &mut no_fallback);
        tracing::info!("[{icount:>12}] {decoded}", icount = self.icount);

        // Let the kernel actually service the syscall.
        InstrAction::Continue
    }
}

// ─────────────────────────── Boot helpers ───────────────────────────

/// Truncate a `String` to `max` **bytes** without splitting a UTF-8 codepoint.
/// Appends `...` when truncation occurred.
fn truncate_str(s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    format!("{}...", &s[..i])
}

fn extract_from_iso(iso_data: &[u8], target_path: &[&str]) -> Option<Vec<u8>> {
    let pvd_offset = 16 * 2048;
    if iso_data.len() < pvd_offset + 2048 {
        return None;
    }
    let pvd = &iso_data[pvd_offset..pvd_offset + 2048];
    if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
        return None;
    }
    let root_record = &pvd[156..156 + 34];
    let mut current_lba =
        u32::from_le_bytes([root_record[2], root_record[3], root_record[4], root_record[5]]);
    let mut current_len = u32::from_le_bytes([
        root_record[10], root_record[11], root_record[12], root_record[13],
    ]);
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
                let next = ((pos / 2048) + 1) * 2048;
                if next >= dir_data.len() {
                    break;
                }
                pos = next;
                continue;
            }
            if record_len < 33 {
                pos += 1;
                continue;
            }
            let entry = &dir_data[pos..pos + record_len];
            let name_len = entry[32] as usize;
            if entry.len() < 33 + name_len {
                pos += record_len;
                continue;
            }
            let entry_name = &entry[33..33 + name_len];
            let (candidate_name, _) = entry_name
                .iter()
                .position(|&b| b == b';')
                .map(|i| (&entry_name[..i], &entry_name[i..]))
                .unwrap_or((entry_name, &[][..]));
            if candidate_name.eq_ignore_ascii_case(name.as_bytes()) {
                let entry_lba = u32::from_le_bytes([entry[2], entry[3], entry[4], entry[5]]);
                let entry_len = u32::from_le_bytes([entry[10], entry[11], entry[12], entry[13]]);
                if is_file {
                    let file_offset = entry_lba as usize * 2048;
                    let end = file_offset + entry_len as usize;
                    if end > iso_data.len() {
                        return None;
                    }
                    return Some(iso_data[file_offset..end].to_vec());
                } else {
                    current_lba = entry_lba;
                    current_len = entry_len;
                    found = true;
                    break;
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

// ─────────────────────────── Main ───────────────────────────

fn main() {
    const STACK: usize = 1500 * 1024 * 1024;
    std::thread::Builder::new()
        .stack_size(STACK)
        .name("alpine-strace".into())
        .spawn(|| {
            if let Err(e) = run() {
                eprintln!("Error: {e:?}");
                std::process::exit(1);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

fn run() -> Result<()> {
    let iso_path = std::env::var("ALPINE_ISO")
        .unwrap_or_else(|_| "alpine-virt-3.23.3-x86_64.iso".to_string());
    let ram_mb: usize = std::env::var("ALPINE_RAM_MB").ok().and_then(|s| s.parse().ok()).unwrap_or(256);
    let max_instructions: u64 = std::env::var("MAX_INSTRUCTIONS").ok().and_then(|s| s.parse().ok()).unwrap_or(4_000_000_000);
    let headless = std::env::var("RUSTY_BOX_HEADLESS").is_ok();
    let bios_boot = std::env::var("RUSTY_BOX_BOOT").unwrap_or_default() != "direct";

    // Route strace output to a file so it doesn't interleave with VGA terminal output.
    let log_path = std::env::var("STRACE_LOG").unwrap_or_else(|_| "strace.log".into());
    let file_appender = tracing_appender::rolling::never(".", &log_path);
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .with_ansi(false)
        .with_writer(non_blocking)
        .init();
    eprintln!("Strace output -> {log_path}");

    tracing::info!("Reading ISO: {iso_path}");
    let iso_data = std::fs::read(&iso_path).unwrap_or_else(|e| {
        eprintln!("Failed to read '{iso_path}': {e}\nSet ALPINE_ISO=/path/to/alpine-virt-*.iso");
        std::process::exit(1);
    });
    tracing::info!("  ISO size: {} MB", iso_data.len() / 1024 / 1024);

    let ram_bytes = ram_mb * 1024 * 1024;
    let config = EmulatorConfig {
        guest_memory_size: ram_bytes,
        host_memory_size: ram_bytes,
        ips: 300_000_000,
        pci_enabled: true,
        ..EmulatorConfig::default()
    };
    let mut emu = Emulator::<Corei7SkylakeX, StraceTracer>::new_with_instrumentation(
        config,
        StraceTracer::default(),
    )?;
    emu.init_memory_and_pc_system()?;

    if bios_boot {
        let ws = std::env::current_dir().unwrap_or_default();
        let ws = ws.to_string_lossy();
        let bios = [
            format!("{ws}/cpp_orig/bochs/bios/BIOS-bochs-latest"),
            format!("{ws}/../cpp_orig/bochs/bios/BIOS-bochs-latest"),
            "cpp_orig/bochs/bios/BIOS-bochs-latest".into(),
        ]
        .iter()
        .find_map(|p| std::fs::read(p).ok())
        .expect("BIOS-bochs-latest not found");
        let bios_load_addr = !(bios.len() as u64 - 1);
        emu.load_bios(&bios, bios_load_addr)?;
        let vga = [
            format!("{ws}/binaries/bios/VGABIOS-lgpl-latest.bin"),
            format!("{ws}/../binaries/bios/VGABIOS-lgpl-latest.bin"),
            "binaries/bios/VGABIOS-lgpl-latest.bin".into(),
        ]
        .iter()
        .find_map(|p| std::fs::read(p).ok());
        if let Some(ref v) = vga {
            emu.load_optional_rom(v, 0xC0000)?;
        }
        emu.init_cpu_and_devices()?;
        emu.configure_memory_in_cmos_from_config();
        emu.configure_boot_sequence(3, 0, 0);
        emu.attach_cdrom(1, 0, &iso_path).expect("attach CDROM");
        if headless { emu.set_gui(NoGui::new()); } else { emu.set_gui(TermGui::new()); }
        emu.init_gui(0, &[])?;
        emu.reset(ResetReason::Hardware)?;
        emu.init_gui_signal_handlers();
        emu.start();
        emu.prepare_run();
    } else {
        let vmlinuz = extract_from_iso(&iso_data, &["BOOT", "VMLINUZ_VIRT."])
            .expect("VMLINUZ_VIRT not found");
        let initramfs = extract_from_iso(&iso_data, &["BOOT", "INITRAMFS_VIRT."])
            .expect("INITRAMFS_VIRT not found");
        let cmdline = std::env::var("CMDLINE").unwrap_or_else(|_|
            "console=ttyS0,115200 earlycon=uart8250,io,0x3f8,115200n8 earlyprintk=serial,ttyS0,115200 nomodeset nokaslr modules=loop,squashfs,cdrom,sr_mod,isofs modloop=/boot/modloop-virt".into()
        );
        emu.init_cpu_and_devices()?;
        emu.configure_memory_in_cmos_from_config();
        emu.attach_cdrom(1, 0, &iso_path).expect("attach CDROM");
        if headless { emu.set_gui(NoGui::new()); } else { emu.set_gui(TermGui::new()); }
        emu.init_gui(0, &[])?;
        emu.reset(ResetReason::Hardware)?;
        emu.init_vga_text_mode3();
        emu.setup_direct_linux_boot(&vmlinuz, Some(&initramfs), &cmdline)?;
        emu.init_gui_signal_handlers();
        emu.start();
    }

    // ─── Execute ───────────────────────────────────────────────────────────
    let start = Instant::now();
    let mut executed: u64 = 0;
    const BATCH: u64 = 500_000;
    let mut last_drain = Instant::now();
    let mut enter_injected = !bios_boot;

    while executed < max_instructions {
        let budget = BATCH.min(max_instructions - executed);
        match emu.run_interactive(budget) {
            Ok(n) => executed += n,
            Err(e) => {
                tracing::error!("CPU error at {executed}: {e:?}");
                break;
            }
        }
        if emu.cpu.is_in_shutdown() {
            tracing::info!("CPU shutdown at {executed}");
            break;
        }
        // BIOS boot: press Enter at ISOLINUX around 18M instructions.
        if !enter_injected && executed >= 18_000_000 {
            emu.send_string("\n");
            enter_injected = true;
        }
        // Drain serial (kernel printk output) to stdout every 100ms.
        if last_drain.elapsed().as_millis() >= 100 {
            let out: Vec<u8> = emu.device_manager.drain_serial_tx(0).collect();
            if !out.is_empty() {
                use std::io::Write;
                let _ = std::io::stdout().write_all(&out);
                let _ = std::io::stdout().flush();
            }
            last_drain = Instant::now();
        }
    }

    // Final drain.
    let out: Vec<u8> = emu.device_manager.drain_serial_tx(0).collect();
    if !out.is_empty() {
        use std::io::Write;
        let _ = std::io::stdout().write_all(&out);
        let _ = std::io::stdout().flush();
    }

    let elapsed = start.elapsed();
    tracing::info!(
        "Ran {executed} instructions in {:.2}s ({:.1} MIPS)",
        elapsed.as_secs_f64(),
        executed as f64 / elapsed.as_secs_f64() / 1_000_000.0,
    );
    Ok(())
}
