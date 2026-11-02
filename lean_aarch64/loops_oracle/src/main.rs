#![allow(dead_code, unused_imports)]

use rusty_box_aarch64::board::{GIC_DIST_BASE, RAM_BASE};
use rusty_box_aarch64::boot::{LinuxImage, SERIAL_CONSOLE_COMMAND_LINE};
use rusty_box_aarch64::{
    GuestStorage, Instrumentation, MachineBuilder, NoEl2, NoInstrumentation, RamSize, Reg,
    RunBudget, StopReason, SysReg, VReg, VirtConfig, VirtMachine,
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
) -> Result<VirtMachine<rusty_box_aarch64::OwnedStorage, NoEl2, I>, String> {
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


fn fnv64(b: &[u8]) -> u64 { let mut h = 0xcbf29ce484222325u64; for &x in b { h ^= x as u64; h = h.wrapping_mul(0x100000001b3); } h }
fn main() {
    let which = std::env::args().nth(1).unwrap();
    let scale: u64 = std::env::args().nth(2).map(|s| s.parse().unwrap()).unwrap_or(1);
    let work = workloads().into_iter().find(|w| w.name == which).unwrap();
    let mut m = loop_machine(&work, NoInstrumentation, scale).unwrap();
    let t = std::time::Instant::now();
    let out = m.step(RunBudget::Instructions(u64::MAX));
    let s = t.elapsed().as_secs_f64();
    let n = out.progress.instructions().unwrap_or(0);
    let mut dst = vec![0u8; BLOCK]; m.mem_read(DESTINATION, &mut dst).unwrap();
    let mut res = [0u8; 8]; m.mem_read(RESULT, &mut res).unwrap();
    let regs: Vec<String> = (0..13).map(|i| format!("{:x}", m.reg_read(Reg::x(i).unwrap()))).collect();
    println!("rusty_box {which}: {n} instrs {s:.3}s {:.1} MIPS", n as f64 / s / 1e6);
    println!("  regs {}", regs.join(" "));
    println!("  dst {:x} result {:x}", fnv64(&dst), u64::from_le_bytes(res));
}
