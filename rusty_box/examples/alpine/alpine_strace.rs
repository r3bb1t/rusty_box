//! Alpine Linux boot with strace-style syscall tracing, in a graphical egui window.
//!
//! Opens a native window (VGA display via `BridgeGui`, emulator on a background
//! thread, `eframe` on the main thread — same architecture as `rusty_box_egui`).
//! Every SYSCALL is intercepted via the `pre_syscall` hook, decoded with string
//! argument resolution, and logged to `strace.log` (keeps the display clean).
//! The SYSCALL executes architecturally so the Linux kernel actually services it
//! — the hook only observes.
//!
//! Boots Alpine from ISO. Default is direct kernel+initramfs boot with
//! `console=tty0`, which renders to the VGA console shown in the window.
//! `RUSTY_BOX_BOOT=bios` uses full BIOS/ISOLINUX boot instead — but the ISO's
//! default is serial-only, so that path only fills the serial console panel.
//!
//! ```bash
//! cargo run --release --example alpine_strace --features "std,instrumentation,gui-egui"
//! ```
//!
//! Env:
//! - `ALPINE_ISO`       — path to Alpine virt ISO
//! - `ALPINE_RAM_MB`    — RAM (default 256)
//! - `MAX_INSTRUCTIONS` — cap (default: run until window closed)
//! - `STRACE_LOG`       — output file (default `strace.log`)
//! - `RUSTY_BOX_BOOT`   — `bios` for full BIOS/ISOLINUX boot (serial-only)
//! - `RUSTY_BOX_NOSYNC` — set to `1` to disable wall-clock slowdown

#![cfg(all(
    feature = "std",
    feature = "instrumentation",
    feature = "gui-egui"
))]

use rusty_box::{
    cpu::{
        core_i7_skylake::Corei7SkylakeX, HookCtx, HookMask, InstrAction, Instrumentation,
        ResetReason, X86Reg,
    },
    emulator::{Emulator, EmulatorConfig},
    gui::{shared_display::SharedDisplay, BridgeGui, RustyBoxApp},
    Result,
};
use std::sync::{atomic::Ordering, Arc, Mutex};

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

// ─────────────────────────── Boot config ───────────────────────────

enum BootMode {
    /// Full BIOS + ISOLINUX boot from the ISO.
    Bios,
    /// Direct kernel + initramfs boot (skips BIOS/ISOLINUX).
    Direct,
}

struct BootConfig {
    iso_path: String,
    ram_mb: usize,
    max_instructions: u64,
    mode: BootMode,
    sync_slowdown: bool,
}

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
    let mut current_lba = u32::from_le_bytes([
        root_record[2],
        root_record[3],
        root_record[4],
        root_record[5],
    ]);
    let mut current_len = u32::from_le_bytes([
        root_record[10],
        root_record[11],
        root_record[12],
        root_record[13],
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

/// Read the first candidate path that exists, relative to the current dir and
/// its parent (covers both `rusty_box/` and the workspace root as CWD).
fn find_file(candidates: &[&str]) -> Option<Vec<u8>> {
    let ws = std::env::current_dir().unwrap_or_default();
    let ws = ws.to_string_lossy();
    candidates
        .iter()
        .flat_map(|c| [format!("{ws}/{c}"), format!("{ws}/../{c}"), c.to_string()])
        .find_map(|p| std::fs::read(&p).ok())
}

// ─────────────────────────── Main ───────────────────────────

fn main() {
    // Route strace output to a file so it doesn't compete with the GUI. The
    // guard must stay alive for the whole program, so it lives in `main`.
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

    let iso_path =
        std::env::var("ALPINE_ISO").unwrap_or_else(|_| "alpine-virt-3.24.1-x86_64.iso".to_string());
    let ram_mb: usize = std::env::var("ALPINE_RAM_MB")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(256);
    let max_instructions: u64 = std::env::var("MAX_INSTRUCTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(u64::MAX);
    // Default to direct kernel boot with `console=tty0`, which renders to the
    // VGA text console shown in the window. BIOS/ISOLINUX boot (RUSTY_BOX_BOOT=bios)
    // uses the ISO's serial-only default, so it only fills the serial panel.
    let mode = if std::env::var("RUSTY_BOX_BOOT").unwrap_or_default() == "bios" {
        BootMode::Bios
    } else {
        BootMode::Direct
    };
    // sync=slowdown enabled by default — throttles active execution to match
    // wall-clock time. Override with RUSTY_BOX_NOSYNC=1.
    let sync_slowdown = std::env::var("RUSTY_BOX_NOSYNC").map_or(true, |v| v != "1");

    // Fail fast if the ISO is missing (before we open a window).
    if std::fs::metadata(&iso_path).is_err() {
        eprintln!(
            "Failed to read '{iso_path}'\nSet ALPINE_ISO=/path/to/alpine-virt-*.iso"
        );
        std::process::exit(1);
    }

    let boot = BootConfig {
        iso_path,
        ram_mb,
        max_instructions,
        mode,
        sync_slowdown,
    };

    // Shared VGA display state bridged between the emulator thread and the GUI.
    let shared = Arc::new(Mutex::new(SharedDisplay::new()));
    let shared_for_emu = Arc::clone(&shared);

    // Emulator runs on a background thread with a large stack (deep call chains).
    let emu_thread = std::thread::Builder::new()
        .stack_size(1500 * 1024 * 1024)
        .name("alpine-strace".into())
        .spawn(move || loop {
            {
                let mut d = shared_for_emu.lock().unwrap();
                d.stop_flag.store(false, Ordering::Relaxed);
                d.emu_running = true;
                d.reset_requested = false;
            }

            if let Err(e) = run_emulator(&boot, Arc::clone(&shared_for_emu)) {
                eprintln!("Emulator error: {e:?}");
            }

            let restart = shared_for_emu.lock().unwrap().reset_requested;
            if !restart {
                break;
            }
            eprintln!("Restarting emulator (Reset requested)...");
        })
        .expect("Failed to spawn emulator thread");

    // eframe must run on the main thread.
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 450.0])
            .with_min_inner_size([720.0, 426.0])
            .with_title("Rusty Box — Alpine Linux (strace)"),
        ..Default::default()
    };
    let shared_for_gui = Arc::clone(&shared);
    let _ = eframe::run_native(
        "Rusty Box strace",
        native_options,
        Box::new(move |cc| Ok(Box::new(RustyBoxApp::new(cc, shared_for_gui)))),
    );

    // Window closed → tell the emulator to stop, then join.
    {
        let mut d = shared.lock().unwrap();
        d.stop_flag.store(true, Ordering::Relaxed);
        d.emu_running = false;
    }
    let _ = emu_thread.join();
}

fn run_emulator(boot: &BootConfig, shared: Arc<Mutex<SharedDisplay>>) -> Result<()> {
    let ram_bytes = boot.ram_mb * 1024 * 1024;
    let config = EmulatorConfig {
        guest_memory_size: ram_bytes,
        host_memory_size: ram_bytes,
        ips: 300_000_000,
        pci_enabled: true,
        sync_slowdown: boot.sync_slowdown,
        ..EmulatorConfig::default()
    };

    let mut emu = Emulator::<Corei7SkylakeX, StraceTracer>::new_with_instrumentation(
        config,
        StraceTracer::default(),
    )?;

    // Wire the GUI stop flag so closing the window stops execution.
    emu.stop_flag = Arc::clone(&shared.lock().unwrap().stop_flag);
    emu.set_gui(BridgeGui::new(Arc::clone(&shared)));
    emu.init_memory_and_pc_system()?;

    match boot.mode {
        BootMode::Bios => {
            let bios = find_file(&["cpp_orig/bochs/bochs/bios/BIOS-bochs-latest"])
                .expect("BIOS-bochs-latest not found");
            let bios_load_addr = !(bios.len() as u64 - 1);
            emu.load_bios(&bios, bios_load_addr)?;
            if let Some(vga) = find_file(&[
                "binaries/bios/VGABIOS-lgpl-latest.bin",
                "cpp_orig/bochs/bochs/bios/VGABIOS-lgpl-latest.bin",
            ]) {
                emu.load_optional_rom(&vga, 0xC0000)?;
            }
            emu.init_cpu_and_devices()?;
            emu.configure_memory_in_cmos_from_config();
            emu.configure_boot_sequence(3, 0, 0);
            emu.attach_cdrom(1, 0, &boot.iso_path).expect("attach CDROM");
            emu.init_gui(0, &[])?;
            emu.reset(ResetReason::Hardware)?;
            emu.init_gui_signal_handlers();
            emu.start();
            // Pre-queue Enter at the ISOLINUX prompt to accept the ISO default.
            emu.prepare_run();
            emu.send_string("\n");
        }
        BootMode::Direct => {
            let iso_data = std::fs::read(&boot.iso_path).expect("read ISO");
            let vmlinuz = extract_from_iso(&iso_data, &["BOOT", "VMLINUZ_VIRT."])
                .expect("VMLINUZ_VIRT not found");
            let initramfs = extract_from_iso(&iso_data, &["BOOT", "INITRAMFS_VIRT."])
                .expect("INITRAMFS_VIRT not found");
            let cmdline = std::env::var("CMDLINE").unwrap_or_else(|_|
                "console=tty0 console=ttyS0,115200 earlycon=uart8250,io,0x3f8,115200n8 nomodeset nokaslr modules=loop,squashfs,cdrom,sr_mod,isofs modloop=/boot/modloop-virt".into()
            );
            emu.init_cpu_and_devices()?;
            emu.configure_memory_in_cmos_from_config();
            emu.attach_cdrom(1, 0, &boot.iso_path).expect("attach CDROM");
            emu.init_gui(0, &[])?;
            emu.reset(ResetReason::Hardware)?;
            emu.init_gui_signal_handlers();
            emu.init_vga_text_mode3();
            emu.start();
            emu.setup_direct_linux_boot(&vmlinuz, Some(&initramfs), &cmdline)?;
        }
    }

    // run_interactive drives the GUI updates and honors the stop flag internally.
    let result = emu.run_interactive(boot.max_instructions);
    match result {
        Ok(executed) => tracing::info!("Ran {executed} instructions"),
        Err(ref e) => tracing::error!("Execution error: {e:?}"),
    }

    if let Ok(mut display) = shared.lock() {
        display.emu_running = false;
    }
    if let Some(gui) = emu.gui_mut() {
        gui.exit();
    }
    result.map(|_| ())
}
