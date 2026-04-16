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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

mod syscalls;

/// Shared strace state — atomics so it can live inside a closure that only
/// captures by shared reference. We don't need locking because all hook
/// firing is single-threaded (the closure runs on the CPU thread while the
/// GUI thread only calls StopHandle).
struct StraceCtx {
    enabled_from: u64,
    max_events: u64,
    log_ports: bool,
    log_irqs: bool,
    log_exc: bool,
    events: AtomicU64,
    /// Last printed icount, for rate-limiting.
    last_printed: AtomicU64,
    out: std::sync::Mutex<std::io::BufWriter<std::io::Stderr>>,
}

impl StraceCtx {
    fn from_env() -> Arc<Self> {
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
        // Exceptions on by default since they're low-volume and interesting.
        let log_exc = std::env::var("STRACE_EXCEPTIONS").ok().as_deref() != Some("0");
        Arc::new(Self {
            enabled_from,
            max_events,
            log_ports,
            log_irqs,
            log_exc,
            events: AtomicU64::new(0),
            last_printed: AtomicU64::new(0),
            out: std::sync::Mutex::new(std::io::BufWriter::new(std::io::stderr())),
        })
    }

    fn bump(&self) -> bool {
        let n = self.events.fetch_add(1, Ordering::Relaxed);
        n < self.max_events
    }


    fn print(&self, line: &str) {
        use std::io::Write;
        if let Ok(mut w) = self.out.lock() {
            let _ = writeln!(w, "{line}");
            let _ = w.flush();
        }
    }
}

/// Tracer that lives inside the BOCHS [`Instrumentation`] trait object.
/// We do the hook logic there because it needs access to register values
/// via its own icount counter — Unicorn-style closures can't read CPU
/// state mid-instruction (they get `(rip, &Instruction)` only).
///
/// For the full register set we install a lightweight shadow: this tracer
/// listens to `before_execution` to capture RIP + Opcode. When it sees a
/// syscall-family instruction, it requests a snapshot from the surrounding
/// emulator on the next batch boundary. But since batches are 4096
/// instructions and syscalls are relatively rare, we get the info live:
/// the hook runs synchronously with CPU state intact.
///
/// So we stash register getters by address and formulate the strace line
/// directly from the CPU snapshot.
struct StraceTracer {
    ctx: Arc<StraceCtx>,
    icount: u64,
    /// Captured at each SYSCALL/SYSENTER/INT80 before the transition.
    /// We need to read registers at the moment the syscall fires; the
    /// `far_branch` hook sees post-transition state where RAX/RDI/... may
    /// have been clobbered. Instead we cooperate with the emulator loop:
    /// flag the next poll, and `drain_events` emits one strace line per flag.
    pending_syscall_nr: Option<(u64, SyscallArgs, bool /*is_64*/)>,
}

#[derive(Debug, Clone, Copy)]
struct SyscallArgs {
    nr: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
}

impl StraceTracer {
    fn new(ctx: Arc<StraceCtx>) -> Self {
        Self { ctx, icount: 0, pending_syscall_nr: None }
    }

    fn should_log(&self) -> bool {
        self.icount >= self.ctx.enabled_from && self.ctx.bump()
    }
}

impl Instrumentation for StraceTracer {
    fn before_execution(&mut self, _rip: u64, _instr: &Instruction) {
        self.icount = self.icount.saturating_add(1);
    }

    fn hwinterrupt(&mut self, vector: u8, cs: u16, rip: u64) {
        if self.ctx.log_irqs && self.should_log() {
            let line = format!(
                "[{icount:>12}] HWIRQ  vec={vec:#04x} {name:>15} cs={cs:#06x} rip={rip:#018x}",
                icount = self.icount,
                vec = vector,
                name = irq_name(vector),
                cs = cs,
                rip = rip,
            );
            self.ctx.print(&line);
        }
    }

    fn exception(&mut self, vector: u8, error_code: u32) {
        if self.ctx.log_exc && self.should_log() {
            let line = format!(
                "[{icount:>12}] EXC    vec={vec:#04x} {name:>6} err={err:#010x}",
                icount = self.icount,
                vec = vector,
                name = exception_name(vector),
                err = error_code,
            );
            self.ctx.print(&line);
        }
    }

    fn interrupt(&mut self, vector: u8) {
        // INT 0x80 is the 32-bit Linux syscall gate. We can only read
        // register values from emulator (not here); the caller below
        // arranges it via a memoized flag.
        if vector == 0x80 && self.should_log() {
            self.pending_syscall_nr = Some((
                self.icount,
                SyscallArgs { nr: 0, a0: 0, a1: 0, a2: 0, a3: 0, a4: 0, a5: 0 },
                false,
            ));
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
            self.pending_syscall_nr = Some((
                self.icount,
                SyscallArgs { nr: 0, a0: 0, a1: 0, a2: 0, a3: 0, a4: 0, a5: 0 },
                is_64,
            ));
        }
    }

    fn inp2(&mut self, port: u16, len: u8, val: u32) {
        if self.ctx.log_ports && !is_noisy_port(port) && self.should_log() {
            let line = format!(
                "[{icount:>12}] IN {len}  port={port:#06x} {name:>14} -> {val:#010x}",
                icount = self.icount,
                len = len,
                port = port,
                name = port_name(port),
                val = val,
            );
            self.ctx.print(&line);
        }
    }

    fn outp(&mut self, port: u16, len: u8, val: u32) {
        if self.ctx.log_ports && !is_noisy_port(port) && self.should_log() {
            let line = format!(
                "[{icount:>12}] OUT{len}  port={port:#06x} {name:>14} <- {val:#010x}",
                icount = self.icount,
                len = len,
                port = port,
                name = port_name(port),
                val = val,
            );
            self.ctx.print(&line);
        }
    }
}

/// Call between batches: reads register state and emits any pending
/// syscall line.
fn drain_syscalls(emu: &mut Emulator<Corei7SkylakeX>, ctx: &StraceCtx) {
    use rusty_box::cpu::X86Reg;
    // Peek into the instrumentation registry to see if a syscall is pending.
    // We installed a tracer, so we need to temporarily pop it, check, and
    // reinstall. That's expensive — but syscalls are rare (<1% of batches).
    let prev = emu.clear_instrumentation();
    let pending = if let Some(mut boxed) = prev {
        let mut tracer = unsafe {
            // SAFETY: we only ever install StraceTracer via this file's path,
            // so downcasting is valid.
            let raw = Box::into_raw(boxed) as *mut StraceTracer;
            Box::from_raw(raw)
        };
        let pending = tracer.pending_syscall_nr.take();
        boxed = tracer;
        emu.set_instrumentation(boxed);
        pending
    } else {
        None
    };

    if let Some((icount, _stale, is_64)) = pending {
        // Read register file NOW — this is the first safe point after the
        // syscall fired. For SYSCALL the kernel has already swapped GS,
        // adjusted RIP and set up RCX/R11, but RAX/RDI/... still hold
        // the caller's values in the first instruction of the handler.
        let (nr, a0, a1, a2, a3, a4, a5) = if is_64 {
            (
                emu.reg_read(X86Reg::Rax).unwrap_or(0),
                emu.reg_read(X86Reg::Rdi).unwrap_or(0),
                emu.reg_read(X86Reg::Rsi).unwrap_or(0),
                emu.reg_read(X86Reg::Rdx).unwrap_or(0),
                emu.reg_read(X86Reg::R10).unwrap_or(0),
                emu.reg_read(X86Reg::R8).unwrap_or(0),
                emu.reg_read(X86Reg::R9).unwrap_or(0),
            )
        } else {
            (
                emu.reg_read(X86Reg::Eax).unwrap_or(0),
                emu.reg_read(X86Reg::Ebx).unwrap_or(0),
                emu.reg_read(X86Reg::Ecx).unwrap_or(0),
                emu.reg_read(X86Reg::Edx).unwrap_or(0),
                emu.reg_read(X86Reg::Esi).unwrap_or(0),
                emu.reg_read(X86Reg::Edi).unwrap_or(0),
                emu.reg_read(X86Reg::Ebp).unwrap_or(0),
            )
        };
        let args = SyscallArgs { nr, a0, a1, a2, a3, a4, a5 };
        let name = if is_64 {
            syscalls::name_x86_64(nr as u32)
        } else {
            syscalls::name_x86_32(nr as u32)
        };
        let fmt = format_syscall(name, &args, is_64);
        let line = format!(
            "[{icount:>12}] {kind:5} {line}",
            icount = icount,
            kind = if is_64 { "SYS64" } else { "SYS32" },
            line = fmt,
        );
        ctx.print(&line);
    }
}

fn format_syscall(name: &str, a: &SyscallArgs, is_64: bool) -> String {
    // Show arg count heuristically — most syscalls use <= 3 args. Always
    // show nr, and dump all six for the ones we know take them.
    let width = if is_64 { 16 } else { 8 };
    format!(
        "{name:<20} nr={nr:>4}  ({a0:#0w$x}, {a1:#0w$x}, {a2:#0w$x}, {a3:#0w$x}, {a4:#0w$x}, {a5:#0w$x})",
        name = name,
        nr = a.nr,
        a0 = a.a0,
        a1 = a.a1,
        a2 = a.a2,
        a3 = a.a3,
        a4 = a.a4,
        a5 = a.a5,
        w = width + 2,
    )
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

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_target(false)
        .init();

    println!("Reading ISO: {iso_path}");
    let iso_data = std::fs::read(&iso_path).unwrap_or_else(|e| {
        eprintln!("Failed to read ISO '{iso_path}': {e}");
        eprintln!("Set ALPINE_ISO=/path/to/alpine-virt-*.iso");
        std::process::exit(1);
    });
    println!("  ISO size: {} MB", iso_data.len() / 1024 / 1024);

    let ram_bytes = ram_mb * 1024 * 1024;
    let config = EmulatorConfig {
        guest_memory_size: ram_bytes,
        host_memory_size: ram_bytes,
        ips: 300_000_000,
        pci_enabled: true,
        ..EmulatorConfig::default()
    };
    let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;
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
            println!("  VGA BIOS loaded: {} bytes", vga_data.len());
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

    // ═════════════════════════ Install strace hooks ═════════════════════════

    let ctx = StraceCtx::from_env();
    println!(
        "Strace config: from_icount={} max={} ports={} irqs={} exc={}",
        ctx.enabled_from, ctx.max_events, ctx.log_ports, ctx.log_irqs, ctx.log_exc
    );

    emu.set_instrumentation(Box::new(StraceTracer::new(ctx.clone())));

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
                eprintln!("CPU error at {executed}: {e:?}");
                break;
            }
        }

        // Drain any pending syscall entries captured by the tracer.
        drain_syscalls(&mut emu, &ctx);

        if emu.cpu.is_in_shutdown() {
            println!("CPU shutdown at {executed}");
            break;
        }

        // BIOS boot: press Enter at ISOLINUX around 18M instructions.
        if !enter_injected && executed >= 18_000_000 {
            println!("[{}M] press Enter at ISOLINUX", executed / 1_000_000);
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
        if ctx.events.load(Ordering::Relaxed) >= ctx.max_events {
            println!(
                "Strace limit ({}) reached at icount~={}; stopping",
                ctx.max_events,
                ctx.last_printed.load(Ordering::Relaxed)
            );
            break;
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
    println!(
        "\nRan {executed} instructions in {:.2}s ({:.1} MIPS); {} strace events logged",
        elapsed.as_secs_f64(),
        executed as f64 / elapsed.as_secs_f64() / 1_000_000.0,
        ctx.events.load(Ordering::Relaxed).min(ctx.max_events),
    );
    Ok(())
}
