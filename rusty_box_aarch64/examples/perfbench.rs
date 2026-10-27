//! The interpreter's speed, measured on fixed work.
//!
//! Five guest loops run on a bare machine with the MMU on (an identity map
//! of RAM as Normal memory, and for `mmio` of the devices too), their code
//! embedded below as encoded words so that nothing is assembled at run
//! time:
//!
//! - `alu`: an integer chain — shifted-register ALU operations, a logical
//!   immediate, a multiply-add, a 32-bit add and the loop's `subs`/`b.ne`;
//! - `memcpy`: 64 KiB copied with post-indexed `ldp`/`stp`, 32 bytes an
//!   iteration;
//! - `fnv`: FNV-1a over 64 KiB, a byte (`ldrb`, `eor`, `mul`) at a time;
//! - `fp`: a double-precision `fmadd`/`fmul`/`fadd`/`fsub` chain;
//! - `mmio`: the `alu` chain run from a device's registers (the GIC
//!   distributor's priority bytes), whose fetches cannot be cached: every
//!   instruction by the per-instruction path.
//!
//! Each ends with PSCI `SYSTEM_OFF`, and each runs twice: as is, and with an
//! observer that hooks every instruction and every data access, as a
//! Unicorn code hook and memory hook would.
//!
//! With the images in the environment it also boots UEFI firmware to the
//! UEFI Shell and a Linux kernel to its login prompt, as `uefi_virt` and
//! `linux_virt` do:
//!
//! | variable | what |
//! |---|---|
//! | `PERFBENCH_EDK2_CODE`, `PERFBENCH_EDK2_VARS` | `AAVMF_CODE.no-secboot.fd`, `AAVMF_VARS.fd` |
//! | `PERFBENCH_KERNEL`, `PERFBENCH_INITRD` | an arm64 `Image` and its initramfs |
//! | `PERFBENCH_WORKLOADS` | a comma-separated subset of `alu,memcpy,fnv,fp,mmio,edk2,alpine` |
//! | `PERFBENCH_HOOKS` | `plain`, `hooked` or `both` (the default) for the loops |
//! | `PERFBENCH_SCALE` | a multiplier on every loop's iteration count (default 1) |
//! | `PERFBENCH_HASH_AT` | instruction counts, comma-separated: instead of timing, print the state hashes at each |
//!
//! Every run prints one line after its clock stops — the workload, the
//! instructions it retired, the host seconds, the speed and the hash of the
//! state it ended in (RAM, the variable store and the registers, and
//! separately the whole snapshot image) — so that a candidate build is
//! compared with a baseline on the same work and the same result. The
//! console of a boot is scanned for its prompt, never printed, and nothing
//! prints while a clock runs.
//!
//! The example is `aarch64_perfbench`, apart from rusty_box's x86
//! `perfbench`, which would build to the same file; `cargo xtask
//! perf-baseline` archives both.
//!
//! ```text
//! cargo run --release -p rusty_box_aarch64 --example aarch64_perfbench --features std
//! PERFBENCH_EDK2_CODE=/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd \
//!   PERFBENCH_EDK2_VARS=/usr/share/AAVMF/AAVMF_VARS.fd \
//!   PERFBENCH_KERNEL=Image PERFBENCH_INITRD=initramfs.cpio.gz \
//!   cargo run --release -p rusty_box_aarch64 --example aarch64_perfbench --features std
//! ```

use std::process::ExitCode;
use std::time::Instant;

use rusty_box_aarch64::board::{GIC_DIST_BASE, RAM_BASE};
use rusty_box_aarch64::boot::{LinuxImage, SERIAL_CONSOLE_COMMAND_LINE};
use rusty_box_aarch64::instrumentation::MemoryAccess;
use rusty_box_aarch64::{
    GuestStorage, Instrumentation, MachineBuilder, NoInstrumentation, RamSize, Reg, RunBudget,
    StopReason, SysReg, VReg, VirtConfig, VirtMachine,
};

/// Where each loop's code is placed.
const CODE: u64 = RAM_BASE + 0x1000;
/// The level-1 translation table of the identity map.
const TABLES: u64 = RAM_BASE + 0x10_0000;
/// The copy's source and the hash's input.
const SOURCE: u64 = RAM_BASE + 0x20_0000;
/// The copy's destination.
const DESTINATION: u64 = RAM_BASE + 0x30_0000;
/// Where the hash leaves its result.
const RESULT: u64 = RAM_BASE + 0x40_0000;
/// Bytes a loop pass covers.
const BLOCK: usize = 64 * 1024;
/// RAM of a loop machine.
const LOOP_RAM_MIB: u64 = 64;

/// How many ticks a boot runs between looks at its console.
const BOOT_SLICE_TICKS: u64 = 4_000_000;
/// How long a boot may take, in ticks, before it counts as failed.
const BOOT_GIVE_UP_TICKS: u64 = 20_000_000_000;

/// `x0 = PSCI SYSTEM_OFF; hvc #0`: every loop's last three words.
const POWER_OFF: [u32; 3] = [0xD280_0100, 0xF2B0_8000, 0xD400_0002];

/// `loop: add x0, x0, x1; eor x1, x1, x0, lsl #3; sub x2, x2, x0;
/// orr x3, x3, x2, lsr #7; and x4, x3, #0xff00ff00ff00ff00;
/// madd x5, x0, x1, x5; lsl x6, x5, #5; add w7, w7, w0; subs x9, x9, #1;
/// b.ne loop` — ten instructions an iteration, `x9` iterations.
const ALU: &[u32] = &[
    0x8B01_0000,
    0xCA00_0C21,
    0xCB00_0042,
    0xAA42_1C63,
    0x9208_9C64,
    0x9B01_1405,
    0xD37B_E8A6,
    0x0B00_00E7,
    0xF100_0529,
    0x54FF_FEE1,
];

/// `outer: mov x0, x11; mov x1, x12; mov x2, #2048;
/// inner: ldp x3, x4, [x0], #16; ldp x5, x6, [x0], #16;
/// stp x3, x4, [x1], #16; stp x5, x6, [x1], #16; subs x2, x2, #1;
/// b.ne inner; subs x10, x10, #1; b.ne outer` — `x10` copies of 64 KiB
/// from `x11` to `x12`.
const MEMCPY: &[u32] = &[
    0xAA0B_03E0,
    0xAA0C_03E1,
    0xD281_0002,
    0xA8C1_1003,
    0xA8C1_1805,
    0xA881_1023,
    0xA881_1825,
    0xF100_0442,
    0x54FF_FF61,
    0xF100_054A,
    0x54FF_FEC1,
];

/// `x1 = 0xcbf29ce484222325; x4 = 0x100000001b3;
/// outer: mov x0, x11; mov x2, #65536;
/// inner: ldrb w3, [x0], #1; eor x1, x1, x3; mul x1, x1, x4;
/// subs x2, x2, #1; b.ne inner; str x1, [x12]; subs x10, x10, #1;
/// b.ne outer` — FNV-1a over 64 KiB at `x11`, `x10` times, the hash at
/// `x12`.
const FNV: &[u32] = &[
    0xD284_64A1,
    0xF2B0_8441,
    0xF2D3_9C81,
    0xF2F9_7E41,
    0xD280_3664,
    0xF2C0_2004,
    0xAA0B_03E0,
    0xD2A0_0022,
    0x3840_1403,
    0xCA03_0021,
    0x9B04_7C21,
    0xF100_0442,
    0x54FF_FF81,
    0xF900_0181,
    0xF100_054A,
    0x54FF_FEE1,
];

/// `loop: fmadd d0, d1, d2, d0; fmul d1, d1, d3; fadd d2, d2, d4;
/// fmadd d5, d0, d3, d5; fsub d6, d6, d1; subs x9, x9, #1; b.ne loop` —
/// seven instructions an iteration, `x9` iterations.
const FP: &[u32] = &[
    0x1F42_0020,
    0x1E63_0821,
    0x1E64_2842,
    0x1F43_1405,
    0x1E61_38C6,
    0xF100_0529,
    0x54FF_FF41,
];

/// Where `mmio` runs the `alu` chain: the GIC distributor's priority bytes
/// of interrupts 32 onwards (`GICD_IPRIORITYR8`), which read back what is
/// written to them and change nothing else.
const DEVICE_CODE: u64 = GIC_DIST_BASE + 0x420;

/// `mov x2, #13; 1: ldr w3, [x11], #4; str w3, [x12], #4; subs x2, x2, #1;
/// b.ne 1b; br x13` — copy the 13 words after it (the `alu` chain and the
/// power-off) from `x11` to `x12`, then run them at `x13`.
const COPY_TO_DEVICE: &[u32] = &[
    0xD280_01A2,
    0xB840_4563,
    0xB800_4583,
    0xF100_0442,
    0x54FF_FFA1,
    0xD61F_01A0,
];

/// What a run measured.
struct Measured {
    instructions: u64,
    seconds: f64,
    stop: StopReason,
    state: u64,
    snapshot: u64,
}

/// An observer with a code hook and a memory hook, each doing a little
/// work, so neither can be compiled away.
#[derive(Default)]
struct Hooks {
    executed: u64,
    pcs: u64,
    accesses: u64,
    addresses: u64,
}

impl Instrumentation for Hooks {
    fn before_execution(&mut self, pc: u64, _insn: u32) {
        self.executed += 1;
        self.pcs = self.pcs.wrapping_add(pc);
    }

    fn memory_access(&mut self, access: MemoryAccess) {
        self.accesses += 1;
        self.addresses = self.addresses.wrapping_add(access.address.get());
    }
}

/// FNV-1a, over everything the hashes cover.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xCBF2_9CE4_8422_2325)
    }

    fn eat(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
}

/// The guest-visible state: RAM, the variable store, and every register a
/// host can read.
fn state_hash<S: GuestStorage, I: Instrumentation>(machine: &VirtMachine<S, I>) -> u64 {
    let mut h = Fnv::new();
    h.eat(machine.ram());
    h.eat(machine.variable_store());
    let cpu = machine.cpu_snapshot();
    for x in cpu.x {
        h.eat(&x.to_le_bytes());
    }
    for v in cpu.v {
        h.eat(&v.to_le_bytes());
    }
    for value in [
        cpu.sp_el0,
        cpu.sp_el1,
        cpu.pc,
        cpu.pstate.to_spsr(),
        cpu.fpcr,
        cpu.fpsr,
        cpu.elr_el1,
        cpu.spsr_el1,
        cpu.esr_el1,
        cpu.far_el1,
        cpu.vbar_el1,
        cpu.sctlr_el1,
        cpu.tcr_el1,
        cpu.ttbr0_el1,
        cpu.ttbr1_el1,
        cpu.mair_el1,
        cpu.instructions,
        cpu.ticks,
    ] {
        h.eat(&value.to_le_bytes());
    }
    h.0
}

/// The whole snapshot image: processor (TLB included), devices, RAM.
fn snapshot_hash<S: GuestStorage, I: Instrumentation>(
    machine: &VirtMachine<S, I>,
) -> Result<u64, String> {
    let mut image = Vec::new();
    machine
        .save_snapshot(&mut image)
        .map_err(|e| format!("snapshot: {e:?}"))?;
    let mut h = Fnv::new();
    h.eat(&image);
    Ok(h.0)
}

/// The words [`COPY_TO_DEVICE`] copies.
const _: () = assert!(ALU.len() + POWER_OFF.len() == 13);

/// A loop: its code, the registers it starts with, and the data it reads.
struct Workload {
    name: &'static str,
    /// Code that runs before the loop's, at [`CODE`].
    prologue: &'static [u32],
    code: &'static [u32],
    /// Whether the first GiB of the address space, the board's devices,
    /// is mapped too (as Normal memory, so that code can run from a
    /// device's registers).
    maps_devices: bool,
    /// The registers it starts with, for a given scale.
    start: fn(u64) -> Start,
}

/// Initial register values: general-purpose, and double-precision `Dn`.
struct Start {
    x: Vec<(Reg, u64)>,
    d: Vec<(u8, f64)>,
}

fn workloads() -> [Workload; 5] {
    [
        Workload {
            name: "alu",
            prologue: &[],
            code: ALU,
            maps_devices: false,
            start: |scale| Start {
                x: vec![
                    (Reg::X0, 0x0123_4567_89AB_CDEF),
                    (Reg::X1, 0x0F1E_2D3C_4B5A_6978),
                    (Reg::X9, 10_000_000 * scale),
                ],
                d: vec![],
            },
        },
        Workload {
            name: "memcpy",
            prologue: &[],
            code: MEMCPY,
            maps_devices: false,
            start: |scale| Start {
                x: vec![
                    (Reg::X10, 8_000 * scale),
                    (Reg::X11, SOURCE),
                    (Reg::X12, DESTINATION),
                ],
                d: vec![],
            },
        },
        Workload {
            name: "fnv",
            prologue: &[],
            code: FNV,
            maps_devices: false,
            start: |scale| Start {
                x: vec![
                    (Reg::X10, 250 * scale),
                    (Reg::X11, SOURCE),
                    (Reg::X12, RESULT),
                ],
                d: vec![],
            },
        },
        Workload {
            name: "fp",
            prologue: &[],
            code: FP,
            maps_devices: false,
            start: |scale| Start {
                x: vec![(Reg::X9, 10_000_000 * scale)],
                d: vec![(1, 1.5), (2, 0.25), (3, 1.000_000_1), (4, 1e-6)],
            },
        },
        Workload {
            name: "mmio",
            prologue: COPY_TO_DEVICE,
            code: ALU,
            maps_devices: true,
            start: |scale| Start {
                x: vec![
                    (Reg::X0, 0x0123_4567_89AB_CDEF),
                    (Reg::X1, 0x0F1E_2D3C_4B5A_6978),
                    (Reg::X9, 100_000 * scale),
                    (Reg::X11, CODE + 4 * COPY_TO_DEVICE.len() as u64),
                    (Reg::X12, DEVICE_CODE),
                    (Reg::X13, DEVICE_CODE),
                ],
                d: vec![],
            },
        },
    ]
}

/// A bare machine with the loop's code at [`CODE`], RAM identity-mapped
/// as Normal memory with the MMU on, floating point enabled, and the
/// source block filled with a fixed pseudo-random pattern.
fn loop_machine<I: Instrumentation>(
    work: &Workload,
    observer: I,
    scale: u64,
) -> Result<VirtMachine<rusty_box_aarch64::OwnedStorage, I>, String> {
    let ram = RamSize::mib(LOOP_RAM_MIB).ok_or("loop RAM size")?;
    let mut machine = MachineBuilder::new(VirtConfig::default())
        .ram(ram)
        .bare()
        .instrumentation(observer)
        .build()
        .map_err(|e| format!("build: {e}"))?;
    let mut code: Vec<u8> = Vec::new();
    for word in work
        .prologue
        .iter()
        .chain(work.code)
        .chain(POWER_OFF.iter())
    {
        code.extend_from_slice(&word.to_le_bytes());
    }
    machine
        .mem_write(CODE, &code)
        .map_err(|e| format!("code: {e}"))?;
    let mut pattern = Vec::with_capacity(BLOCK);
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    for _ in 0..BLOCK / 8 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        pattern.extend_from_slice(&seed.to_le_bytes());
    }
    machine
        .mem_write(SOURCE, &pattern)
        .map_err(|e| format!("data: {e}"))?;
    // One level-1 block descriptor: 1 GiB at RAM_BASE, AttrIndx 0
    // (MAIR byte 0xFF, Normal write-back), inner shareable, AF set, EL1
    // read/write.
    let block = RAM_BASE | (1 << 10) | (3 << 8) | 0b01;
    machine
        .mem_write(TABLES + 8 * (RAM_BASE >> 30), &block.to_le_bytes())
        .map_err(|e| format!("tables: {e}"))?;
    if work.maps_devices {
        // The same for the first GiB, the devices', at VA 0.
        let devices = (1 << 10) | (3 << 8) | 0b01u64;
        machine
            .mem_write(TABLES, &devices.to_le_bytes())
            .map_err(|e| format!("tables: {e}"))?;
    }
    // T0SZ 25 (a 39-bit space from level 1), walks cacheable and inner
    // shareable, the 4 KiB granule; EPD1 keeps TTBR1 out of it.
    let tcr = 25 | (1 << 8) | (1 << 10) | (3 << 12) | (1 << 23);
    for (reg, value) in [
        (SysReg::MAIR_EL1, 0xFF),
        (SysReg::TCR_EL1, tcr),
        (SysReg::TTBR0_EL1, TABLES),
        (SysReg::CPACR_EL1, 3 << 20),
        (SysReg::SCTLR_EL1, 0x00C5_0838 | 1 | (1 << 2) | (1 << 12)),
    ] {
        machine
            .sysreg_write(reg, value)
            .map_err(|e| format!("{reg:?}: {e}"))?;
    }
    let start = (work.start)(scale);
    for (reg, value) in start.x {
        machine.reg_write(reg, value);
    }
    for (n, value) in start.d {
        let reg = VReg::new(n).ok_or("a V register")?;
        machine.vreg_write(reg, u128::from(value.to_bits()));
    }
    machine.reg_write(Reg::Pc, CODE);
    Ok(machine)
}

fn run_loop<I: Instrumentation>(
    work: &Workload,
    observer: I,
    scale: u64,
) -> Result<Measured, String> {
    let mut machine = loop_machine(work, observer, scale)?;
    let started = Instant::now();
    let outcome = machine.step(RunBudget::Instructions(u64::MAX));
    let seconds = started.elapsed().as_secs_f64();
    Ok(Measured {
        instructions: outcome.progress.instructions().unwrap_or(0),
        seconds,
        stop: outcome.stop,
        state: state_hash(&machine),
        snapshot: snapshot_hash(&machine)?,
    })
}

/// A boot image set, read from the environment.
enum Boot {
    Edk2 {
        code: Vec<u8>,
        vars: Option<Vec<u8>>,
    },
    Alpine {
        kernel: Vec<u8>,
        initrd: Option<Vec<u8>>,
    },
}

impl Boot {
    fn name(&self) -> &'static str {
        match self {
            Self::Edk2 { .. } => "edk2",
            Self::Alpine { .. } => "alpine",
        }
    }

    /// What the console shows when the boot is done.
    fn prompt(&self) -> &'static [u8] {
        match self {
            Self::Edk2 { .. } => b"Shell> ",
            Self::Alpine { .. } => b"login: ",
        }
    }

    fn machine(&self) -> Result<VirtMachine<rusty_box_aarch64::OwnedStorage>, String> {
        let builder = MachineBuilder::new(VirtConfig::default());
        match self {
            Self::Edk2 { code, vars } => {
                let mut b = builder.firmware(code);
                if let Some(vars) = vars {
                    b = b.variables(vars);
                }
                b.build().map_err(|e| format!("build: {e}"))
            }
            Self::Alpine { kernel, initrd } => {
                let image = LinuxImage::parse(kernel).map_err(|e| format!("kernel: {e}"))?;
                let mut b = builder
                    .linux(image)
                    .command_line(SERIAL_CONSOLE_COMMAND_LINE);
                if let Some(initrd) = initrd {
                    b = b.initrd(initrd);
                }
                b.build().map_err(|e| format!("build: {e}"))
            }
        }
    }
}

fn read(var: &str) -> Result<Option<Vec<u8>>, String> {
    match std::env::var_os(var) {
        Some(path) => std::fs::read(&path)
            .map(Some)
            .map_err(|e| format!("{var}={}: {e}", path.to_string_lossy())),
        None => Ok(None),
    }
}

fn boots() -> Result<Vec<Boot>, String> {
    let mut out = Vec::new();
    if let Some(code) = read("PERFBENCH_EDK2_CODE")? {
        out.push(Boot::Edk2 {
            code,
            vars: read("PERFBENCH_EDK2_VARS")?,
        });
    }
    if let Some(kernel) = read("PERFBENCH_KERNEL")? {
        out.push(Boot::Alpine {
            kernel,
            initrd: read("PERFBENCH_INITRD")?,
        });
    }
    Ok(out)
}

/// Boot until the prompt, the console scanned after every slice.
fn run_boot(boot: &Boot) -> Result<Measured, String> {
    let mut machine = boot.machine()?;
    let prompt = boot.prompt();
    let mut window: Vec<u8> = Vec::new();
    let started = Instant::now();
    let stop = loop {
        let outcome = machine.step(RunBudget::Ticks(BOOT_SLICE_TICKS));
        if let Some(mut uart) = machine.serial(0) {
            window.extend(uart.take_output());
        }
        if window.windows(prompt.len()).any(|w| w == prompt) {
            break outcome.stop;
        }
        let keep = window.len().saturating_sub(prompt.len());
        window.drain(..keep);
        if outcome.is_terminal() || machine.ticks() >= BOOT_GIVE_UP_TICKS {
            break outcome.stop;
        }
    };
    let seconds = started.elapsed().as_secs_f64();
    let reached = window.windows(prompt.len()).any(|w| w == prompt);
    if !reached {
        return Err(format!(
            "{} stopped ({stop:?}) before its prompt",
            boot.name()
        ));
    }
    Ok(Measured {
        instructions: machine.instructions(),
        seconds,
        stop,
        state: state_hash(&machine),
        snapshot: snapshot_hash(&machine)?,
    })
}

/// The state hashes after each of `counts` instructions of a boot.
fn hash_boot(boot: &Boot, counts: &[u64]) -> Result<(), String> {
    let mut machine = boot.machine()?;
    for &count in counts {
        while machine.instructions() < count {
            let outcome = machine.step(RunBudget::Instructions(count - machine.instructions()));
            if let Some(mut uart) = machine.serial(0) {
                uart.take_output().for_each(drop);
            }
            if outcome.is_terminal() {
                return Err(format!(
                    "{} stopped ({:?}) at {} instructions",
                    boot.name(),
                    outcome.stop,
                    machine.instructions()
                ));
            }
        }
        println!(
            "hash {} at {count}: state {:016x} snapshot {:016x} ticks {}",
            boot.name(),
            state_hash(&machine),
            snapshot_hash(&machine)?,
            machine.ticks()
        );
    }
    Ok(())
}

fn print(name: &str, variant: &str, m: &Measured) {
    println!(
        "perfbench {name:<7} {variant:<6} insns {:>11} secs {:>8.3} mips {:>8.2} stop {:?} state {:016x} snapshot {:016x}",
        m.instructions,
        m.seconds,
        m.instructions as f64 / m.seconds.max(1e-9) / 1e6,
        m.stop,
        m.state,
        m.snapshot
    );
}

fn env_u64(var: &str, default: u64) -> Result<u64, String> {
    match std::env::var(var) {
        Ok(text) => text.parse().map_err(|e| format!("{var}: {e}")),
        Err(_) => Ok(default),
    }
}

fn bench() -> Result<(), String> {
    let selected: Option<Vec<String>> = std::env::var("PERFBENCH_WORKLOADS")
        .ok()
        .map(|s| s.split(',').map(str::to_owned).collect());
    let wants = |name: &str| {
        selected
            .as_ref()
            .is_none_or(|list| list.iter().any(|s| s == name))
    };
    let hooks = std::env::var("PERFBENCH_HOOKS").unwrap_or_else(|_| String::from("both"));
    let (plain, hooked) = match hooks.as_str() {
        "plain" => (true, false),
        "hooked" => (false, true),
        "both" => (true, true),
        other => return Err(format!("PERFBENCH_HOOKS={other}: plain, hooked or both")),
    };
    let scale = env_u64("PERFBENCH_SCALE", 1)?;
    if scale == 0 {
        // A loop counter of 0 would count down through 2^64 iterations.
        return Err("PERFBENCH_SCALE=0: the loops need at least 1".to_string());
    }
    let hash_at: Option<Vec<u64>> = match std::env::var("PERFBENCH_HASH_AT") {
        Ok(text) => Some(
            text.split(',')
                .map(|n| {
                    n.trim()
                        .parse::<u64>()
                        .map_err(|e| format!("PERFBENCH_HASH_AT: {e}"))
                })
                .collect::<Result<_, _>>()?,
        ),
        Err(_) => None,
    };

    if let Some(counts) = &hash_at {
        for boot in boots()?.iter().filter(|b| wants(b.name())) {
            hash_boot(boot, counts)?;
        }
        return Ok(());
    }

    for work in workloads().iter().filter(|w| wants(w.name)) {
        if plain {
            let m = run_loop(work, NoInstrumentation, scale)?;
            print(work.name, "plain", &m);
        }
        if hooked {
            let m = run_loop(work, Hooks::default(), scale)?;
            print(work.name, "hooked", &m);
        }
    }
    for boot in boots()?.iter().filter(|b| wants(b.name())) {
        let m = run_boot(boot)?;
        print(boot.name(), "plain", &m);
    }
    Ok(())
}

fn main() -> ExitCode {
    // A machine is built and moved by value; see examples/common.
    let thread = std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(bench);
    match thread.map(std::thread::JoinHandle::join) {
        Ok(Ok(Ok(()))) => ExitCode::SUCCESS,
        Ok(Ok(Err(e))) => {
            eprintln!("perfbench: {e}");
            ExitCode::FAILURE
        }
        Ok(Err(_)) => {
            eprintln!("perfbench: the benchmark thread panicked");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("perfbench: cannot start the benchmark thread: {e}");
            ExitCode::FAILURE
        }
    }
}
