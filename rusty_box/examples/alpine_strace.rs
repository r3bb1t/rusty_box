//! Alpine Linux Boot with strace-Style System Call Tracing
//!
//! Demonstrates the new instrumentation API: loads Alpine like `alpine_direct`,
//! then uses Unicorn-style `hook_add_*` closures and the BOCHS-style
//! [`Instrumentation`] trait to log:
//!
//! - every Linux syscall with decoded arguments (SYSCALL, SYSENTER, INT 0x80)
//! - every x86 exception (#PF, #GP, #UD, ...)
//! - hardware interrupt delivery
//! - I/O port reads/writes (coarse list, noisy ones muted)
//!
//! ## Usage
//!
//! ```bash
//! cargo run --release --example alpine_strace \
//!     --features "std,instrumentation" -- --headless
//! ```
//!
//! Environment variables (all optional):
//! - `ALPINE_ISO`          — path to Alpine virt ISO (default: alpine-virt-3.23.3-x86_64.iso)
//! - `ALPINE_RAM_MB`       — RAM size (default 256)
//! - `MAX_INSTRUCTIONS`    — cap (default 4_000_000_000)
//! - `STRACE_FROM_ICOUNT`  — only log events after this many icount (skip BIOS POST noise)
//! - `STRACE_LIMIT`        — max events to log (default unlimited)
//! - `STRACE_LOG`           — output file path (default: strace.log)
//! - `STRACE_PORTS`        — `1` to log port I/O (default off — very noisy)
//! - `STRACE_IRQS`         — `1` to log hardware interrupts (default off)
//! - `STRACE_EXCEPTIONS`   — `1` to log CPU exceptions (default on)
//! - `RUSTY_BOX_BOOT`      — `direct` bypasses BIOS/ISOLINUX
//! - `RUSTY_BOX_HEADLESS`  — any value → no GUI

#![cfg(all(feature = "std", feature = "instrumentation"))]

use rusty_box::{
    cpu::{
        core_i7_skylake::Corei7SkylakeX,
        decoder::Instruction,
        BranchType, Instrumentation, ResetReason,
    },
    emulator::{Emulator, EmulatorConfig},
    gui::{NoGui, TermGui},
    Result,
};
use std::time::Instant;

mod syscalls;

/// BOCHS-style tracer. All config and state are plain fields — no Arc,
/// no Mutex, no atomics. The outer loop accesses state via the zero-cost
/// `emu.instrumentation()` / `emu.instrumentation_mut()` field accessors.
pub struct StraceTracer {
    enabled_from: u64,
    max_events: u64,
    log_ports: bool,
    log_irqs: bool,
    log_exc: bool,
    events: u64,
    icount: u64,
    last_printed: u64,
    /// Set by `interrupt`/`far_branch` when a syscall fires; drained by
    /// `drain_syscalls` on the next batch boundary.
    pending: Option<Pending>,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    icount: u64,
    is_64: bool,
}

impl StraceTracer {
    fn from_env() -> Self {
        let enabled_from = std::env::var("STRACE_FROM_ICOUNT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let max_events = std::env::var("STRACE_LIMIT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(u64::MAX);
        let log_ports = std::env::var("STRACE_PORTS").ok().as_deref() == Some("1");
        let log_irqs = std::env::var("STRACE_IRQS").ok().as_deref() == Some("1");
        let log_exc = std::env::var("STRACE_EXCEPTIONS").ok().as_deref() != Some("0");
        Self {
            enabled_from,
            max_events,
            log_ports,
            log_irqs,
            log_exc,
            events: 0,
            icount: 0,
            last_printed: 0,
            pending: None,
        }
    }

    fn should_log(&mut self) -> bool {
        if self.icount >= self.enabled_from && self.events < self.max_events {
            self.events += 1;
            true
        } else {
            false
        }
    }
}

impl Instrumentation for StraceTracer {
    fn before_execution(&mut self, _rip: u64, _instr: &Instruction) {
        self.icount = self.icount.saturating_add(1);
    }

    fn hwinterrupt(&mut self, vector: u8, cs: u16, rip: u64) {
        if self.log_irqs && self.should_log() {
            let line = format!(
                "[{icount:>12}] HWIRQ  vec={vec:#04x} {name:>15} cs={cs:#06x} rip={rip:#018x}",
                icount = self.icount,
                vec = vector,
                name = irq_name(vector),
                cs = cs,
                rip = rip,
            );
            tracing::info!("{}", line);
        }
    }

    fn exception(&mut self, vector: u8, error_code: u32) {
        if self.log_exc && self.should_log() {
            let line = format!(
                "[{icount:>12}] EXC    vec={vec:#04x} {name:>6} err={err:#010x}",
                icount = self.icount,
                vec = vector,
                name = exception_name(vector),
                err = error_code,
            );
            tracing::info!("{}", line);
        }
    }

    fn interrupt(&mut self, vector: u8) {
        // INT 0x80 is the 32-bit Linux syscall gate. Register file is read
        // in `drain_syscalls` on the next batch boundary.
        if vector == 0x80 && self.should_log() {
            self.pending = Some(Pending { icount: self.icount, is_64: false });
        }
    }

    fn far_branch(
        &mut self,
        what: BranchType,
        _prev_cs: u16,
        _prev_rip: u64,
        _new_cs: u16,
        _new_rip: u64,
    ) {
        // SYSCALL (64-bit) / SYSENTER (32-bit) arrive here.
        if matches!(what, BranchType::Syscall | BranchType::Sysenter)
            && self.should_log()
        {
            let is_64 = matches!(what, BranchType::Syscall);
            self.pending = Some(Pending { icount: self.icount, is_64 });
        }
    }

    fn inp2(&mut self, port: u16, len: u8, val: u32) {
        if self.log_ports && !is_noisy_port(port) && self.should_log() {
            let line = format!(
                "[{icount:>12}] IN {len}  port={port:#06x} {name:>14} -> {val:#010x}",
                icount = self.icount,
                len = len,
                port = port,
                name = port_name(port),
                val = val,
            );
            tracing::info!("{}", line);
        }
    }

    fn outp(&mut self, port: u16, len: u8, val: u32) {
        if self.log_ports && !is_noisy_port(port) && self.should_log() {
            let line = format!(
                "[{icount:>12}] OUT{len}  port={port:#06x} {name:>14} <- {val:#010x}",
                icount = self.icount,
                len = len,
                port = port,
                name = port_name(port),
                val = val,
            );
            tracing::info!("{}", line);
        }
    }
}

/// Call between batches: pulls any pending syscall out of the tracer, reads
/// register state, decodes into a typed `Syscall` enum, and logs a
/// strace-style line.
fn drain_syscalls(emu: &mut Emulator<Corei7SkylakeX, StraceTracer>) {
    use rusty_box::cpu::X86Reg;
    let pending = emu.instrumentation_mut().pending.take();
    let Some(Pending { icount, is_64 }) = pending else { return; };

    // Read the register file now — first safe point after the syscall
    // fired. SYSCALL has swapped GS and loaded LSTAR into RIP, but the
    // kernel handler hasn't run yet so RAX/RDI/... still hold user values.
    let args: [u64; 6] = if is_64 {
        [
            emu.reg_read(X86Reg::Rdi),
            emu.reg_read(X86Reg::Rsi),
            emu.reg_read(X86Reg::Rdx),
            emu.reg_read(X86Reg::R10),
            emu.reg_read(X86Reg::R8),
            emu.reg_read(X86Reg::R9),
        ]
    } else {
        [
            emu.reg_read(X86Reg::Ebx),
            emu.reg_read(X86Reg::Ecx),
            emu.reg_read(X86Reg::Edx),
            emu.reg_read(X86Reg::Esi),
            emu.reg_read(X86Reg::Edi),
            emu.reg_read(X86Reg::Ebp),
        ]
    };
    let nr = if is_64 { emu.reg_read(X86Reg::Rax) } else { emu.reg_read(X86Reg::Eax) };

    // Decode into typed enum. String args read from guest memory.
    let mut read_str = |addr: u64| -> String {
        if addr == 0 { return "NULL".into(); }
        let mut buf = [0u8; 256];
        match emu.mem_read(addr, &mut buf) {
            Ok(()) => {
                let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                String::from_utf8_lossy(&buf[..end]).into_owned()
            }
            Err(_) => format!("{addr:#x}"),
        }
    };

    let decoded = if is_64 {
        syscalls::Syscall::decode_x86_64(nr, args, &mut read_str)
    } else {
        // 32-bit: use name table fallback (decode_x86_32 not yet implemented)
        let name = syscalls::name_x86_32(nr as u32);
        syscalls::Syscall::Other { nr, name, args }
    };

    tracing::info!(
        "[{icount:>12}] {kind:5} {decoded}",
        icount = icount,
        kind = if is_64 { "SYS64" } else { "SYS32" },
    );
    emu.instrumentation_mut().last_printed = icount;
}

// ─────────────────── Port, IRQ, Exception name tables ───────────────────

fn port_name(port: u16) -> &'static str {
    match port {
        0x20 | 0x21 => "PIC1",
        0xA0 | 0xA1 => "PIC2",
        0x40..=0x43 => "PIT",
        0x60 | 0x64 => "KBD",
        0x70 | 0x71 => "CMOS/RTC",
        0x80 => "POST",
        0x92 => "A20/reset",
        0xE9 => "BIOS-dbg",
        0x170..=0x177 | 0x376 | 0x377 => "ATA-1",
        0x1F0..=0x1F7 | 0x3F6 => "ATA-0",
        0x3F8..=0x3FF => "COM1",
        0x2F8..=0x2FF => "COM2",
        0x3D4 | 0x3D5 => "VGA-CRT",
        0x3BA | 0x3DA => "VGA-stat",
        0x3C0..=0x3CF => "VGA-seq",
        0xCF8 | 0xCFC..=0xCFF => "PCI-CONF",
        _ => "?",
    }
}

/// Ports that fire >1000 times per batch — muted by default to keep the
/// trace readable. Enable with `STRACE_PORTS=1 STRACE_ALL_PORTS=1`.
fn is_noisy_port(port: u16) -> bool {
    if std::env::var("STRACE_ALL_PORTS").ok().as_deref() == Some("1") {
        return false;
    }
    matches!(
        port,
        // PIT channel 0 ticks continuously
        0x40 | 0x43
        // Keyboard status polling
        | 0x60 | 0x64
        // VGA attribute index (polled during text-mode writes)
        | 0x3DA | 0x3BA
        // PIC EOI writes
        | 0x20 | 0xA0
    )
}

fn irq_name(vector: u8) -> &'static str {
    // After PIC remap, Linux uses 0x20+irq for IRQs.
    match vector {
        0x20 => "IRQ0-timer",
        0x21 => "IRQ1-kbd",
        0x22 => "IRQ2-casc",
        0x23 => "IRQ3-com2",
        0x24 => "IRQ4-com1",
        0x25 => "IRQ5",
        0x26 => "IRQ6-fdc",
        0x27 => "IRQ7",
        0x28 => "IRQ8-rtc",
        0x29 => "IRQ9",
        0x2A => "IRQ10",
        0x2B => "IRQ11",
        0x2C => "IRQ12-ps2",
        0x2D => "IRQ13-fpu",
        0x2E => "IRQ14-ata0",
        0x2F => "IRQ15-ata1",
        _ => "?",
    }
}

fn exception_name(vector: u8) -> &'static str {
    match vector {
        0 => "#DE",
        1 => "#DB",
        2 => "NMI",
        3 => "#BP",
        4 => "#OF",
        5 => "#BR",
        6 => "#UD",
        7 => "#NM",
        8 => "#DF",
        10 => "#TS",
        11 => "#NP",
        12 => "#SS",
        13 => "#GP",
        14 => "#PF",
        16 => "#MF",
        17 => "#AC",
        18 => "#MC",
        19 => "#XM",
        20 => "#VE",
        _ => "?",
    }
}

// ─────────────────────────── Boot wrapper (copied from alpine_direct) ───────────────────────────

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
                let entry_lba =
                    u32::from_le_bytes([entry[2], entry[3], entry[4], entry[5]]);
                let entry_len =
                    u32::from_le_bytes([entry[10], entry[11], entry[12], entry[13]]);
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
    let ram_mb: usize = std::env::var("ALPINE_RAM_MB")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(256);
    let headless = std::env::var("RUSTY_BOX_HEADLESS").is_ok();
    let max_instructions: u64 = std::env::var("MAX_INSTRUCTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4_000_000_000);
    let boot_mode =
        std::env::var("RUSTY_BOX_BOOT").unwrap_or_else(|_| "bios".to_string());
    let bios_boot = boot_mode != "direct";

    // Route strace output to a log file so it doesn't interleave with VGA terminal output.
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
        eprintln!("Failed to read ISO '{iso_path}': {e}");
        eprintln!("Set ALPINE_ISO=/path/to/alpine-virt-*.iso");
        std::process::exit(1);
    });
    tracing::info!("  ISO size: {} MB", iso_data.len() / 1024 / 1024);

    let ram_bytes = ram_mb * 1024 * 1024;
    let tracer = StraceTracer::from_env();
    tracing::info!(
        "Strace config: from_icount={} max={} ports={} irqs={} exc={}",
        tracer.enabled_from, tracer.max_events, tracer.log_ports, tracer.log_irqs, tracer.log_exc
    );

    let config = EmulatorConfig {
        guest_memory_size: ram_bytes,
        host_memory_size: ram_bytes,
        ips: 300_000_000,
        pci_enabled: true,
        ..EmulatorConfig::default()
    };
    let mut emu = Emulator::<Corei7SkylakeX, StraceTracer>::new_with_instrumentation(config, tracer)?;
    emu.init_memory_and_pc_system()?;

    if bios_boot {
        let ws = std::env::current_dir().unwrap_or_default();
        let ws = ws.to_string_lossy();
        let bios_candidates = [
            format!("{ws}/cpp_orig/bochs/bios/BIOS-bochs-latest"),
            format!("{ws}/../cpp_orig/bochs/bios/BIOS-bochs-latest"),
            "cpp_orig/bochs/bios/BIOS-bochs-latest".into(),
        ];
        let bios_data = bios_candidates
            .iter()
            .find_map(|p| std::fs::read(p).ok())
            .expect("Could not find BIOS-bochs-latest");
        let bios_size = bios_data.len() as u64;
        let bios_load_addr = !(bios_size - 1);
        emu.load_bios(&bios_data, bios_load_addr)?;

        let vga_candidates = [
            format!("{ws}/binaries/bios/VGABIOS-lgpl-latest.bin"),
            format!("{ws}/../binaries/bios/VGABIOS-lgpl-latest.bin"),
            "binaries/bios/VGABIOS-lgpl-latest.bin".into(),
        ];
        if let Some(vga_data) = vga_candidates.iter().find_map(|p| std::fs::read(p).ok()) {
            emu.load_optional_rom(&vga_data, 0xC0000)?;
            tracing::info!("  VGA BIOS loaded: {} bytes", vga_data.len());
        }
        emu.init_cpu_and_devices()?;
        emu.configure_memory_in_cmos_from_config();
        emu.configure_boot_sequence(3, 0, 0);
        emu.attach_cdrom(1, 0, &iso_path).expect("attach CDROM");
        if headless {
            emu.set_gui(NoGui::new());
        } else {
            emu.set_gui(TermGui::new());
        }
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
        if headless {
            emu.set_gui(NoGui::new());
        } else {
            emu.set_gui(TermGui::new());
        }
        emu.init_gui(0, &[])?;
        emu.reset(ResetReason::Hardware)?;
        emu.init_vga_text_mode3();
        emu.setup_direct_linux_boot(&vmlinuz, Some(&initramfs), &cmdline)?;
        emu.init_gui_signal_handlers();
        emu.start();
    }

    // Instrumentation already installed at construction time.

    // ═════════════════════════ Execute ═════════════════════════
    let start = Instant::now();
    let mut executed: u64 = 0;
    const PHASE: u64 = 500_000;
    let mut last_drain = Instant::now();
    let mut enter_injected = !bios_boot;

    while executed < max_instructions {
        let budget = PHASE.min(max_instructions - executed);
        match emu.run_interactive(budget) {
            Ok(n) => executed += n,
            Err(e) => {
                tracing::error!("CPU error at {executed}: {e:?}");
                break;
            }
        }

        // Drain any pending syscall entries captured by the tracer.
        drain_syscalls(&mut emu);

        if emu.cpu.is_in_shutdown() {
            tracing::info!("CPU shutdown at {executed}");
            break;
        }

        // BIOS boot: press Enter at ISOLINUX around 18M instructions.
        if !enter_injected && executed >= 18_000_000 {
            tracing::info!("[{}M] press Enter at ISOLINUX", executed / 1_000_000);
            emu.send_string("\n");
            enter_injected = true;
        }

        // Drain serial (kernel printk output).
        if last_drain.elapsed().as_millis() >= 100 {
            let out: Vec<u8> = emu.device_manager.drain_serial_tx(0).collect();
            if !out.is_empty() {
                use std::io::Write;
                std::io::stdout().write_all(&out).ok();
                std::io::stdout().flush().ok();
            }
            last_drain = Instant::now();
        }

        // Stop once we've logged enough events.
        {
            let t = emu.instrumentation();
            if t.events >= t.max_events {
                tracing::info!(
                    "Strace limit ({}) reached at icount~={}; stopping",
                    t.max_events,
                    t.last_printed,
                );
                break;
            }
        }
    }

    // Final drain.
    let out: Vec<u8> = emu.device_manager.drain_serial_tx(0).collect();
    if !out.is_empty() {
        use std::io::Write;
        std::io::stdout().write_all(&out).ok();
        std::io::stdout().flush().ok();
    }

    let elapsed = start.elapsed();
    tracing::info!(
        "Ran {executed} instructions in {:.2}s ({:.1} MIPS); {} strace events logged",
        elapsed.as_secs_f64(),
        executed as f64 / elapsed.as_secs_f64() / 1_000_000.0,
        emu.instrumentation().events.min(emu.instrumentation().max_events),
    );
    Ok(())
}
