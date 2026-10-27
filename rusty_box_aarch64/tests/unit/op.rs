//! Every pre-decoded op executes exactly as its class handler executes the
//! word it came from. The property is the guest-visible one (R9): from the
//! same processor and memory, the handler and the op end in the same
//! result — the same next PC or exception, or the same stop — with every
//! register, flag, system register, TLB entry and memory byte the same.
//!
//! Words come from the encoding templates of the classes `op.rs`
//! specialises (the fixed bits of each Arm encoding class, every other bit
//! random), so that each specialised form, the reserved values that keep a
//! word on its class handler, and the boundaries between classes are all
//! reached; and from wholly random words. States are random: general and
//! SP registers (half of them pointing into a small RAM, aligned or not),
//! SIMD&FP registers (mostly numbers of moderate size, sometimes a zero,
//! an infinity, a NaN or a denormal), FPCR's controls, FPSR's cumulative
//! bits, the SIMD&FP enable, NZCV, the exception level and stack pointer
//! selection, the SP and strict alignment checks, and the MMU off (Device
//! memory, so misaligned accesses fault) or on over an identity map whose
//! permissions vary.

use std::collections::BTreeMap;
use std::vec::Vec;

use rusty_box_core::{ClockHz, VmClock};

use super::{predecode, Op};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::cpu::decode::classify;
use crate::cpu::exec::{cond_holds, Nzcv};
use crate::cpu::{Cpu, CpuConfig, El, Pstate};

/// The RAM: one page at the base of the 1 GiB the identity map covers.
const RAM: u64 = 0x4000_0000;
const RAM_LEN: usize = 4096;
/// Where the walk finds the level-1 table, outside RAM.
const TABLE: u64 = 0x1000_0000;

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// A page of RAM, and a translation table with one level-1 block
/// descriptor.
struct TestBus {
    ram: Vec<u8>,
    block: u64,
}

impl PhysBus for TestBus {
    fn read(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        _clock: VmClock,
    ) -> Result<u64, BusFault> {
        let a = addr.get();
        let n = width.bytes();
        if a >= RAM && a + n <= RAM + RAM_LEN as u64 {
            let off = (a - RAM) as usize;
            let mut v = 0u64;
            for i in 0..n as usize {
                v |= u64::from(self.ram[off + i]) << (8 * i);
            }
            return Ok(v);
        }
        if a == TABLE + 8 * (RAM >> 30) && width == AccessWidth::Dword {
            return Ok(self.block);
        }
        if (TABLE..TABLE + 4096).contains(&a) {
            return Ok(0);
        }
        Err(BusFault::Unmapped)
    }

    fn write(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        value: u64,
        _clock: VmClock,
    ) -> Result<(), BusFault> {
        let a = addr.get();
        let n = width.bytes();
        if a >= RAM && a + n <= RAM + RAM_LEN as u64 {
            let off = (a - RAM) as usize;
            for i in 0..n as usize {
                self.ram[off + i] = (value >> (8 * i)) as u8;
            }
            return Ok(());
        }
        Err(BusFault::Unmapped)
    }
}

/// A random register value: an address in or around the RAM, aligned or
/// not, half the time.
fn register(rng: &mut SplitMix64) -> u64 {
    let r = rng.next();
    match r % 4 {
        0 | 1 => (RAM + rng.next() % (RAM_LEN as u64 + 64)) & if r & 4 == 0 { !7 } else { !0 },
        2 => rng.next() % 64,
        _ => rng.next(),
    }
}

/// A random floating-point value: a zero, an infinity, a NaN, a denormal
/// or the largest number now and then, any bits now and then, and most of
/// the time a number of moderate magnitude, so that sums and products are
/// mostly normal and inexact; double precision in the low 64 bits, or
/// single precision in the low 32.
fn fp_register(rng: &mut SplitMix64) -> u128 {
    const DOUBLE_SPECIALS: [u64; 8] = [
        0,
        1 << 63,
        0x7FF0_0000_0000_0000,
        0xFFF0_0000_0000_0000,
        0x7FF8_0000_0000_0001,
        0x7FF0_0000_0000_0001,
        0x0000_0000_0000_0001,
        0x7FEF_FFFF_FFFF_FFFF,
    ];
    const SINGLE_SPECIALS: [u32; 8] = [
        0,
        1 << 31,
        0x7F80_0000,
        0xFF80_0000,
        0x7FC0_0001,
        0x7F80_0001,
        1,
        0x7F7F_FFFF,
    ];
    let r = rng.next();
    let bits = rng.next();
    let upper = u128::from(rng.next()) << 64;
    let single = r & 8 != 0;
    let low = match (r % 8, single) {
        (0, false) => DOUBLE_SPECIALS[(bits % 8) as usize],
        (0, true) => u64::from(SINGLE_SPECIALS[(bits % 8) as usize]) | (rng.next() << 32),
        (1, _) => bits,
        (_, false) => (bits & 0x800F_FFFF_FFFF_FFFF) | ((1023 - 40 + (r >> 8) % 80) << 52),
        (_, true) => {
            u64::from((bits as u32 & 0x807F_FFFF) | ((127 - 20 + ((r >> 8) % 40) as u32) << 23))
                | (bits & !0xFFFF_FFFF)
        }
    };
    upper | u128::from(low)
}

/// Put `cpu` and `bus` in the state `seed` names.
fn set_up(cpu: &mut Cpu, bus: &mut TestBus, seed: u64) {
    let mut rng = SplitMix64(seed);
    for x in cpu.x.iter_mut() {
        *x = register(&mut rng);
    }
    for v in cpu.v.iter_mut() {
        *v = fp_register(&mut rng);
    }
    // FPCR.RMode, FZ, DN and AHP, to nearest half the time; FPSR's
    // cumulative bits, IXC set half the time; the SIMD&FP enable, mostly
    // on.
    let fp = rng.next();
    let rmode = if fp & 1 == 0 { 0 } else { (fp >> 1) & 3 };
    cpu.fpcr = (rmode << 22) | (((fp >> 3) & 0b111) << 24);
    cpu.fpsr = (fp >> 8) & 0b1001_1111;
    cpu.sys.cpacr_el1 = if fp & (3 << 16) == 0 {
        ((fp >> 18) & 3) << 20
    } else {
        3 << 20
    };
    cpu.sp_el0 = register(&mut rng);
    cpu.sp_el1 = register(&mut rng);
    let flags = rng.next();
    let el = if flags & 1 == 0 { El::El1 } else { El::El0 };
    cpu.pstate = Pstate {
        n: flags & 2 != 0,
        z: flags & 4 != 0,
        c: flags & 8 != 0,
        v: flags & 16 != 0,
        el,
        sp_sel: el == El::El1 && flags & 32 != 0,
        ..Pstate::RESET
    };
    cpu.pc = RAM + ((rng.next() % RAM_LEN as u64) & !3);
    // SCTLR_EL1.M, .A, .SA and .SA0 from the seed; the rest at reset.
    let mmu = flags & 64 != 0;
    let sctlr = crate::cpu::id::SCTLR_EL1_RESET & !0b1_1011;
    let bit = |n: u32| (flags >> n) & 1;
    cpu.sys.sctlr_el1 = sctlr | u64::from(mmu) | (bit(7) << 1) | (bit(8) << 3) | (bit(9) << 4);
    cpu.sys.tcr_el1 = 25 | (1 << 23);
    cpu.sys.ttbr0_el1 = TABLE;
    cpu.sys.mair_el1 = 0xFF;
    cpu.tlb.flush();
    cpu.monitor.armed = false;
    // An identity block for the RAM's gigabyte, with random AP and
    // execute-never bits.
    let ap = (flags >> 10) & 3;
    let execute_never = (flags >> 12) & 3;
    bus.block = RAM | (1 << 10) | (3 << 8) | (ap << 6) | (execute_never << 53) | 0b01;
    for b in bus.ram.iter_mut() {
        *b = rng.next() as u8;
    }
}

fn snapshot(cpu: &Cpu) -> Vec<u8> {
    let mut image = Vec::new();
    cpu.save(&mut image).expect("a processor saves into a Vec");
    image
}

/// `(mask, value)`: the fixed bits of an encoding class.
const TEMPLATES: [(u32, u32, &str); 22] = [
    (0x1F80_0000, 0x1100_0000, "add/sub (immediate)"),
    (0x1F20_0000, 0x0B00_0000, "add/sub (shifted register)"),
    (0x1F80_0000, 0x1200_0000, "logical (immediate)"),
    (0x1F00_0000, 0x0A00_0000, "logical (shifted register)"),
    (0x1F80_0000, 0x1280_0000, "move wide"),
    (0x1F00_0000, 0x1000_0000, "PC-relative addressing"),
    (0x1F80_0000, 0x1300_0000, "bitfield"),
    (0x1FE0_0000, 0x1A80_0000, "conditional select"),
    (0x1F00_0000, 0x1B00_0000, "data processing (3 source)"),
    (0xFF00_0000, 0x5400_0000, "conditional branch"),
    (0x7E00_0000, 0x3400_0000, "compare and branch"),
    (0x7E00_0000, 0x3600_0000, "test and branch"),
    (0x7C00_0000, 0x1400_0000, "unconditional branch (immediate)"),
    (0xFF9F_FC1F, 0xD61F_0000, "unconditional branch (register)"),
    (0xFFFF_F01F, 0xD503_201F, "hints"),
    (0x3B00_0000, 0x3900_0000, "load/store (unsigned immediate)"),
    (
        0x3B20_0000,
        0x3800_0000,
        "load/store (immediate, unscaled and indexed)",
    ),
    (0x3B20_0C00, 0x3820_0800, "load/store (register offset)"),
    (0x3800_0000, 0x2800_0000, "load/store pair"),
    (
        0x1F20_0C00,
        0x1E20_0800,
        "floating-point data processing (2 source)",
    ),
    (
        0x1F00_0000,
        0x1F00_0000,
        "floating-point data processing (3 source)",
    ),
    (
        0xFFFF_FF00,
        0xD53B_E000,
        "MRS of a generic timer counter register",
    ),
];

#[test]
fn every_pre_decoded_op_executes_as_its_class_handler_does() {
    let config = CpuConfig {
        ips: ClockHz::BOCHS_DEFAULT,
        counter_hz: ClockHz::BOCHS_DEFAULT,
        reset_pc: RAM,
    };
    let mut by_class = Cpu::new(config);
    let mut by_op = Cpu::new(config);
    let mut by_pool = Cpu::new(config);
    let mut bus_class = TestBus {
        ram: std::vec![0; RAM_LEN],
        block: 0,
    };
    let mut bus_op = TestBus {
        ram: std::vec![0; RAM_LEN],
        block: 0,
    };
    let mut bus_pool = TestBus {
        ram: std::vec![0; RAM_LEN],
        block: 0,
    };
    let mut rng = SplitMix64(0x0DDB_A11C_0FFE_E000);
    let mut hits: BTreeMap<&'static str, u32> = BTreeMap::new();
    for i in 0..400_000u32 {
        let random = rng.next();
        let word = match TEMPLATES.get(i as usize % (TEMPLATES.len() + 2)) {
            Some(&(mask, value, _)) => (random as u32 & !mask) | value,
            None => random as u32,
        };
        let class = classify(word);
        let op = predecode(class, word);
        let name = variant(op);
        *hits.entry(name).or_insert(0) += 1;
        if matches!(op, Op::Class { .. }) {
            continue;
        }
        let seed = rng.next();
        set_up(&mut by_class, &mut bus_class, seed);
        set_up(&mut by_op, &mut bus_op, seed);
        set_up(&mut by_pool, &mut bus_pool, seed);
        let pc = by_class.pc;
        let want = by_class.exec_class(&mut bus_class, pc, class, word);
        // The op as the per-instruction path holds it, and where a trace's
        // op lies.
        let mut raised = None;
        let got = by_op
            .exec_op(&mut bus_op, pc, op, &mut raised)
            .map_err(|fault| *fault);
        let mut raised = None;
        let pooled = by_pool
            .exec_pooled(&mut bus_pool, pc, op, word, &mut raised)
            .map_err(|fault| *fault);
        assert_eq!(got, want, "{word:#010x} ({name}) seed {seed:#x}: result");
        assert_eq!(
            pooled, want,
            "{word:#010x} ({name}) seed {seed:#x}: result from the pool"
        );
        assert!(
            snapshot(&by_op) == snapshot(&by_class),
            "{word:#010x} ({name}) seed {seed:#x}: processor state"
        );
        assert!(
            snapshot(&by_pool) == snapshot(&by_class),
            "{word:#010x} ({name}) seed {seed:#x}: processor state from the pool"
        );
        assert!(
            bus_op.ram == bus_class.ram,
            "{word:#010x} ({name}) seed {seed:#x}: memory"
        );
        assert!(
            bus_pool.ram == bus_class.ram,
            "{word:#010x} ({name}) seed {seed:#x}: memory from the pool"
        );
    }
    // Every specialised form ran many times.
    for name in VARIANTS {
        let n = hits.get(name).copied().unwrap_or(0);
        assert!(n >= 200, "{name} ran {n} times: {hits:?}");
    }
}

#[test]
fn counter_reads_predecode_to_their_own_op() {
    for word in [0xD53B_E040, 0xD53B_E05F, 0xD53B_E020] {
        assert_eq!(
            predecode(classify(word), word),
            Op::ReadCounter { word },
            "{word:#010x}"
        );
    }
    // MRS CNTFRQ_EL0 and MSR CNTV_TVAL_EL0 stay with their class.
    for word in [0xD53B_E000, 0xD51B_E304] {
        assert!(
            matches!(predecode(classify(word), word), Op::Class { .. }),
            "{word:#010x}"
        );
    }
}

#[test]
fn a_condition_holds_on_a_flag_value_as_it_does_on_pstate() {
    let mut cpu = Cpu::new(CpuConfig {
        ips: ClockHz::BOCHS_DEFAULT,
        counter_hz: ClockHz::BOCHS_DEFAULT,
        reset_pc: RAM,
    });
    for bits in 0..16u32 {
        let flags = Nzcv {
            n: bits & 8 != 0,
            z: bits & 4 != 0,
            c: bits & 2 != 0,
            v: bits & 1 != 0,
        };
        cpu.pstate.set_flags(flags);
        assert_eq!(
            (cpu.pstate.n, cpu.pstate.z, cpu.pstate.c, cpu.pstate.v),
            (flags.n, flags.z, flags.c, flags.v)
        );
        for cond in 0..16 {
            assert_eq!(
                cond_holds(flags, cond),
                cpu.condition_holds(cond),
                "flags {bits:#06b} cond {cond:#06b}"
            );
        }
    }
}

/// The name of every `Op` variant but `Class`.
const VARIANTS: [&str; 31] = [
    "AddImm",
    "AddsImm",
    "AddSubShift",
    "LogImm",
    "LogShift",
    "MovImm",
    "Movk",
    "Adr",
    "Adrp",
    "Ubfm",
    "Sbfm",
    "Bfm",
    "CondSel",
    "Madd",
    "BCond",
    "Cbz",
    "Tbz",
    "B",
    "Bl",
    "Br",
    "Blr",
    "Nop",
    "LoadImm",
    "StoreImm",
    "LoadReg",
    "StoreReg",
    "LoadPair",
    "StorePair",
    "FpDp2",
    "FpDp3",
    "ReadCounter",
];

fn variant(op: Op) -> &'static str {
    match op {
        Op::Class { .. } => "Class",
        Op::AddImm { .. } => "AddImm",
        Op::AddsImm { .. } => "AddsImm",
        Op::AddSubShift { .. } => "AddSubShift",
        Op::LogImm { .. } => "LogImm",
        Op::LogShift { .. } => "LogShift",
        Op::MovImm { .. } => "MovImm",
        Op::Movk { .. } => "Movk",
        Op::Adr { .. } => "Adr",
        Op::Adrp { .. } => "Adrp",
        Op::Ubfm { .. } => "Ubfm",
        Op::Sbfm { .. } => "Sbfm",
        Op::Bfm { .. } => "Bfm",
        Op::CondSel { .. } => "CondSel",
        Op::Madd { .. } => "Madd",
        Op::BCond { .. } => "BCond",
        Op::Cbz { .. } => "Cbz",
        Op::Tbz { .. } => "Tbz",
        Op::B { .. } => "B",
        Op::Bl { .. } => "Bl",
        Op::Br { .. } => "Br",
        Op::Blr { .. } => "Blr",
        Op::Nop => "Nop",
        Op::LoadImm { .. } => "LoadImm",
        Op::StoreImm { .. } => "StoreImm",
        Op::LoadReg { .. } => "LoadReg",
        Op::StoreReg { .. } => "StoreReg",
        Op::LoadPair { .. } => "LoadPair",
        Op::StorePair { .. } => "StorePair",
        Op::FpDp2 { .. } => "FpDp2",
        Op::FpDp3 { .. } => "FpDp3",
        Op::ReadCounter { .. } => "ReadCounter",
    }
}
