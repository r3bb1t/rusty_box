//! Differential check against Unicorn (QEMU's TCG): replays
//! `tests/vectors/unicorn.txt`, produced by `tests/vectors/gen_unicorn.py`.
//!
//! Each vector is an instruction word and a seed. The seed rebuilds the exact
//! initial state the generator gave Unicorn (registers, flags, vector
//! registers, a 64 KiB data region); the vector lists what Unicorn changed.
//! The property asserted is the guest-visible one (R9): after one step, PC,
//! NZCV, FPSR, every general and vector register, SP and every data byte
//! equal Unicorn's.
//!
//! Unicorn's CPU model has more features than this core, so a word it
//! executed may be UNDEFINED here. Such a vector is skipped only for a
//! reason [`expected_undefined`] names; any other UNDEFINED word, and any
//! step that stops the machine, fails the test. Failure messages show
//! disarm64's disassembly of the word.
//!
//! A second check here, `against_disarm64`, steps the words the decoder
//! hands to a handler but binutils' opcode table allocates to nothing this
//! core implements: each must be UNDEFINED.

use std::collections::BTreeMap;
use std::format;
use std::string::String;
use std::vec::Vec;

use rusty_box_core::{ClockHz, VmClock};

use super::{Cpu, CpuConfig, Step};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

const VECTORS: &str = include_str!("../vectors/unicorn.txt");

const CODE: u64 = 0x10000;
const DATA: u64 = 0x10_0000;
const DATA_SIZE: usize = 0x1_0000;

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

struct State {
    x: [u64; 31],
    sp: u64,
    nzcv: u64,
    v: [u128; 32],
    data: Vec<u8>,
    fpcr: u64,
}

/// Mirrors `F32_SPECIAL` in gen_unicorn.py.
const F32_SPECIAL: [u64; 34] = [
    0x0000_0000, 0x8000_0000, 0x7F80_0000, 0xFF80_0000, 0x7FC0_0000, 0xFFC1_2345, 0x7F80_0001, 0xFFA0_0000,
    0x0000_0001, 0x807F_FFFF, 0x0080_0000, 0x8080_0000, 0x7F7F_FFFF, 0xFF7F_FFFF, 0x3F80_0000, 0xBF80_0000,
    0x3F00_0000, 0xBF00_0000, 0x3FC0_0000, 0x4020_0000, 0xC020_0000, 0x4060_0000, 0x4B00_0000, 0x4B7F_FFFF,
    0x4F00_0000, 0xCF00_0000, 0x4F80_0000, 0x5F00_0000, 0xDF00_0000, 0x5F80_0000, 0x4EFF_FFFF, 0x477F_E000,
    0x3380_0000, 0x387F_C000,
];

/// Mirrors `F64_SPECIAL` in gen_unicorn.py.
const F64_SPECIAL: [u64; 35] = [
    0x0000_0000_0000_0000,
    0x8000_0000_0000_0000,
    0x7FF0_0000_0000_0000,
    0xFFF0_0000_0000_0000,
    0x7FF8_0000_0000_0000,
    0xFFF8_0000_0001_2345,
    0x7FF0_0000_0000_0001,
    0xFFF4_0000_0000_0000,
    0x0000_0000_0000_0001,
    0x800F_FFFF_FFFF_FFFF,
    0x0010_0000_0000_0000,
    0x8010_0000_0000_0000,
    0x7FEF_FFFF_FFFF_FFFF,
    0xFFEF_FFFF_FFFF_FFFF,
    0x3FF0_0000_0000_0000,
    0xBFF0_0000_0000_0000,
    0x3FE0_0000_0000_0000,
    0xBFE0_0000_0000_0000,
    0x3FF8_0000_0000_0000,
    0x4004_0000_0000_0000,
    0xC004_0000_0000_0000,
    0x400C_0000_0000_0000,
    0x4330_0000_0000_0000,
    0x41E0_0000_0000_0000,
    0xC1E0_0000_0000_0000,
    0x41DF_FFFF_FFC0_0000,
    0x41EF_FFFF_FFE0_0000,
    0x41F0_0000_0000_0000,
    0x43E0_0000_0000_0000,
    0xC3E0_0000_0000_0000,
    0x43F0_0000_0000_0000,
    0x47EF_FFFF_E000_0000,
    0x36A0_0000_0000_0000,
    0x3810_0000_0000_0000,
    0x380F_FFFF_FFFF_FFFF,
];

/// Mirrors `ordinary_f32` in gen_unicorn.py.
fn ordinary_f32(w: u64) -> u64 {
    ((w >> 63) << 31) | ((117 + ((w >> 32) % 21)) << 23) | (w & 0x7F_FFFF)
}

/// Mirrors `ordinary_f64` in gen_unicorn.py.
fn ordinary_f64(w: u64) -> u64 {
    ((w >> 63) << 63) | ((1003 + ((w >> 52) & 0x3FF) % 41) << 52) | (w & ((1 << 52) - 1))
}

/// Mirrors `initial_state` in gen_unicorn.py exactly.
fn initial_state(seed: u64, kind: &str, insn: u32) -> State {
    let mut r = SplitMix64(seed);
    let mut x = [0u64; 31];
    for slot in x.iter_mut() {
        let sel = r.next() & 7;
        let v = r.next();
        *slot = match sel {
            0 => 0,
            1 => v & 0xFF,
            2 => (v & 0xFF).wrapping_neg(),
            3 => v & 0xFFFF_FFFF,
            4 => (v as u32 as i32 as i64) as u64,
            5 => 1u64 << (v & 63),
            _ => v,
        };
    }
    let mut sp = r.next();
    let nzcv = (r.next() & 0xF) << 28;
    let mut v = [0u128; 32];
    for slot in v.iter_mut() {
        let lo = r.next();
        let hi = r.next();
        *slot = u128::from(lo) | (u128::from(hi) << 64);
    }
    let mut data = Vec::with_capacity(DATA_SIZE);
    for _ in 0..DATA_SIZE / 8 {
        data.extend_from_slice(&r.next().to_le_bytes());
    }
    if kind == "V" {
        const LOW_NIBBLES: u128 = 0x0F0F_0F0F_0F0F_0F0F_0F0F_0F0F_0F0F_0F0F;
        const ONE_PER_BYTE: u128 = 0x0101_0101_0101_0101_0101_0101_0101_0101;
        for slot in v.iter_mut() {
            let sel = r.next() & 1;
            let signs = u128::from(r.next()) | (u128::from(r.next()) << 64);
            if sel == 1 {
                *slot = (*slot & LOW_NIBBLES) | ((signs & ONE_PER_BYTE) * 0xF0);
            }
        }
    }
    let mut fpcr = 0;
    if kind == "F" {
        for slot in v.iter_mut() {
            let sel = r.next() & 7;
            if sel >= 2 {
                let single = matches!(sel, 2 | 4 | 5);
                let mixed = matches!(sel, 2 | 3);
                let (count, width) = if single { (4, 32) } else { (2, 64) };
                let mut value = 0u128;
                for k in 0..count {
                    let w = r.next();
                    let lane = if mixed && w & 1 == 1 {
                        let table: &[u64] = if single { &F32_SPECIAL } else { &F64_SPECIAL };
                        table[((w >> 1) % table.len() as u64) as usize]
                    } else if single {
                        ordinary_f32(w)
                    } else {
                        ordinary_f64(w)
                    };
                    value |= u128::from(lane) << (width * k);
                }
                *slot = value;
            }
        }
        fpcr = (r.next() & 0x1F) << 22;
    }
    let rn = ((insn >> 5) & 31) as usize;
    if kind == "T" && rn != 31 {
        x[rn] = CODE + ((r.next() & 0x3FFF) << 2);
    }
    if kind == "M" || kind == "R" {
        let base = DATA + 0x4000 + (r.next() & 0x3F0);
        if rn == 31 {
            sp = base;
        } else {
            x[rn] = base;
        }
        if kind == "R" {
            let rm = ((insn >> 16) & 31) as usize;
            let off = (r.next() & 0xF) << 4;
            if rm != 31 && rm != rn {
                x[rm] = off;
            }
        }
    }
    State {
        x,
        sp,
        nzcv,
        v,
        data,
        fpcr,
    }
}

struct TestBus {
    insn: u32,
    data: Vec<u8>,
}

impl PhysBus for TestBus {
    fn read(&mut self, addr: PhysAddr, width: AccessWidth, _clock: VmClock) -> Result<u64, BusFault> {
        let a = addr.get();
        let n = width.bytes();
        if a >= CODE && a + n <= CODE + 4 {
            return Ok(u64::from(self.insn >> (8 * (a - CODE))) & width.value_mask());
        }
        if a + n <= DATA {
            return Ok(0);
        }
        if a >= DATA && a + n <= DATA + DATA_SIZE as u64 {
            let off = (a - DATA) as usize;
            let mut v = 0u64;
            for i in 0..n as usize {
                v |= u64::from(self.data[off + i]) << (8 * i);
            }
            return Ok(v);
        }
        Err(BusFault::Unmapped)
    }

    fn write(&mut self, addr: PhysAddr, width: AccessWidth, value: u64, _clock: VmClock) -> Result<(), BusFault> {
        let a = addr.get();
        let n = width.bytes();
        if a >= DATA && a + n <= DATA + DATA_SIZE as u64 {
            let off = (a - DATA) as usize;
            for i in 0..n as usize {
                self.data[off + i] = (value >> (8 * i)) as u8;
            }
            return Ok(());
        }
        Err(BusFault::Unmapped)
    }
}

/// `key:value` pairs; register numbers are decimal, data offsets hex, values
/// always hex — as gen_unicorn.py writes them.
fn parse_pairs(field: &str, prefix: &str, key_radix: u32) -> Vec<(usize, u128)> {
    let body = field.strip_prefix(prefix).unwrap_or("-");
    if body == "-" {
        return Vec::new();
    }
    body.split(',')
        .filter_map(|p| {
            let (k, v) = p.split_once(':')?;
            Some((usize::from_str_radix(k, key_radix).ok()?, u128::from_str_radix(v, 16).ok()?))
        })
        .collect()
}

/// With SIMD&FP disabled (`CPACR_EL1.FPEN = 0`), a reserved encoding is
/// still UNDEFINED (EC 0x00): decode comes before the enable check. An
/// allocated one takes the SIMD&FP access trap (EC 0x07).
#[test]
fn reserved_simd_and_fp_encodings_are_undefined_before_the_fp_trap() {
    let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
    let cases: [(u32, &str, u64); 18] = [
        (0x0E22_E020, "pmull v0.8h, v1.8b, v2.8b", 0x07),
        (0x0EE2_E020, "pmull v0.1q, v1.1d, v2.1d", 0x07),
        (0x0E62_E020, "pmull with size 01", 0x00),
        (0x0EA2_E020, "pmull with size 10", 0x00),
        (0x2EE2_E020, "opcode 1110 with U = 1 and size 11", 0x00),
        (0x4E68_4820, "aese with size 01", 0x00),
        (0x5E02_7020, "sha3 class opcode 111", 0x00),
        (0x0E08_0420, "dup v0.1d, v1.d[0] (Q = 0)", 0x00),
        (0x0E04_2C20, "smov w0, v1.s[0]", 0x00),
        (0x4E01_3C20, "umov x0, v1.b[0]", 0x00),
        (0x0E08_3C20, "umov w0, v1.d[0]", 0x00),
        (0x1EA0_2820, "fadd with ftype 10", 0x00),
        (0x4E08_0420, "dup v0.2d, v1.d[0]", 0x07),
        (0x4F3F_E420, "scvtf v0.4s, v1.4s, #1", 0x07),
        (0x5F3F_E420, "scvtf s0, s1, #1", 0x07),
        (0x5F7F_FC20, "fcvtzs d0, d1, #1", 0x07),
        (0x4F1F_E420, "scvtf v0.8h, v1.8h, #1 (half precision, FEAT_FP16)", 0x00),
        (0x1E20_2820, "fadd s0, s1, s0", 0x07),
    ];
    for (insn, text, want_ec) in cases {
        let mut cpu = Cpu::new(CpuConfig {
            ips,
            counter_hz: ips,
            reset_pc: CODE,
        });
        let mut bus = TestBus {
            insn,
            data: std::vec![0; DATA_SIZE],
        };
        let step = cpu.step(&mut bus, &mut NoInstrumentation);
        assert_eq!(step, Step::Retired, "{text}");
        assert_eq!(cpu.sys.esr_el1 >> 26, want_ec, "{text}");
    }
}

/// UDF raises the Undefined Instruction exception (EC 0x00) whatever its
/// immediate (Arm ARM C6.2 UDF): `udf #1` and `udf #2` end in the bits SVC
/// and HVC end in, but neither is an exception-generating instruction.
#[test]
fn udf_takes_the_undefined_instruction_exception_whatever_its_immediate() {
    let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
    for insn in [0x0000_0000, 0x0000_0001, 0x0000_0002, 0x0000_0003, 0x0000_ABCD] {
        let mut cpu = Cpu::new(CpuConfig {
            ips,
            counter_hz: ips,
            reset_pc: CODE,
        });
        cpu.x[0] = 0x8400_0008; // PSCI SYSTEM_OFF, had `udf #2` reached the PSCI conduit
        let mut bus = TestBus {
            insn,
            data: std::vec![0; DATA_SIZE],
        };
        assert_eq!(cpu.step(&mut bus, &mut NoInstrumentation), Step::Retired, "{insn:#010x}");
        assert_eq!(cpu.sys.esr_el1 >> 26, 0x00, "{insn:#010x}");
    }
}

/// ADD, ADDS, SUB and SUBS (immediate) with bit 23 set are UNDEFINED: the
/// ARMv8.0 decode makes `shift = 1x` a ReservedValue (Arm ARM ADD
/// (immediate)), and QEMU decodes no instruction there (`a64.decode`
/// `ADD_i` and its siblings fix bit 23 to 0). None of these is one of the
/// later encodings that took bit 23 (ADDG/SUBG need `sf = 1, S = 0`, the
/// min/max immediates `bits 21:18 = 00xx, S = 0`).
#[test]
fn add_and_sub_immediate_with_shift_1x_are_undefined() {
    let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
    let cases: [(u32, &str); 5] = [
        (0x1180_0420, "add w0, w1, #1 with shift 10"),
        (0x11C4_0420, "add w0, w1, #1 with shift 11"),
        (0xB1C0_0420, "adds x0, x1, #1 with shift 11"),
        (0x5180_0420, "sub w0, w1, #1 with shift 10"),
        (0xF1C0_0420, "subs x0, x1, #1 with shift 11"),
    ];
    for (insn, text) in cases {
        let mut cpu = Cpu::new(CpuConfig {
            ips,
            counter_hz: ips,
            reset_pc: CODE,
        });
        cpu.x[1] = 41;
        let mut bus = TestBus {
            insn,
            data: std::vec![0; DATA_SIZE],
        };
        assert_eq!(cpu.step(&mut bus, &mut NoInstrumentation), Step::Retired, "{text}");
        assert_eq!(cpu.sys.esr_el1 >> 26, 0x00, "{text}: UNDEFINED");
        assert_eq!(cpu.x[0], 0, "{text}: nothing written");
    }
}

/// A processor at EL1 with SIMD&FP enabled that steps one word at a time,
/// each from the same reset state. The state comes back from a snapshot
/// before every word, which costs far less than building a processor (its
/// decode cache alone is 320 KiB), and the decode cache, keyed by the word,
/// needs no reset.
struct Stepper {
    cpu: Cpu,
    reset: Vec<u8>,
}

impl Stepper {
    fn new() -> Self {
        let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
        let mut cpu = Cpu::new(CpuConfig {
            ips,
            counter_hz: ips,
            reset_pc: CODE,
        });
        cpu.sys.cpacr_el1 = 3 << 20;
        let mut reset = Vec::new();
        if let Err(e) = cpu.save(&mut reset) {
            panic!("the reset state does not save: {e:?}");
        }
        Self { cpu, reset }
    }

    /// One step of `insn`: the exception class it took, or `None` when it
    /// took none.
    fn exception_class(&mut self, insn: u32) -> Option<u64> {
        if let Err(e) = self.cpu.restore(&mut self.reset.as_slice()) {
            panic!("the reset state does not restore: {e:?}");
        }
        self.cpu.sys.esr_el1 = 0x3F << 26;
        let mut bus = TestBus {
            insn,
            data: std::vec![0; DATA_SIZE],
        };
        assert_eq!(self.cpu.step(&mut bus, &mut NoInstrumentation), Step::Retired, "{insn:#010x}");
        Some(self.cpu.sys.esr_el1 >> 26).filter(|&ec| ec != 0x3F)
    }
}

/// [`Stepper::exception_class`] on a processor of its own.
fn step_exception_class(insn: u32) -> Option<u64> {
    Stepper::new().exception_class(insn)
}

/// The barriers are CLREX, DSB, DMB and ISB with `op1 = 011` (Arm's
/// `barriers` group, `CLREX_BN_barriers` and its siblings); with any other
/// `op1` the word is unallocated, so UNDEFINED, not a barrier.
#[test]
fn barriers_need_op1_011() {
    for (insn, text) in [
        (0xD503_305F, "clrex"),
        (0xD503_3F9F, "dsb sy"),
        (0xD503_3BBF, "dmb ish"),
        (0xD503_3FDF, "isb"),
    ] {
        assert_eq!(step_exception_class(insn), None, "{text} executes");
        for op1 in [0b000, 0b001, 0b010, 0b100, 0b111] {
            let other = (insn & !(0b111 << 16)) | (op1 << 16);
            assert_eq!(step_exception_class(other), Some(0x00), "{text} with op1 = {op1:03b}: UNDEFINED");
        }
    }
}

/// The EL1 TLB maintenance instructions of ARMv8.0 are `SYS #0, C8, Cm,
/// #op2` with `Cm` 3 (Inner Shareable) or 7 and `op2` one of 0, 1, 2, 3, 5,
/// 7 (QEMU target/arm/tcg/tlb-insns.c `tlbi_v8_cp_reginfo`); the rest of
/// `CRn = 8` belongs to FEAT_TLBIOS and FEAT_TLBIRANGE, absent here, so is
/// UNDEFINED rather than a TLB flush.
#[test]
fn tlb_maintenance_is_the_armv8_0_set() {
    for crm in 0..16u32 {
        for op2 in 0..8u32 {
            let insn = 0xD508_8000 | (crm << 8) | (op2 << 5);
            let allocated = matches!(crm, 3 | 7) && matches!(op2, 0 | 1 | 2 | 3 | 5 | 7);
            let expected = if allocated { None } else { Some(0x00) };
            assert_eq!(step_exception_class(insn), expected, "sys #0, c8, c{crm}, #{op2}, x0");
        }
    }
}

/// FCMP and FCMPE with zero have a should-be-zero `Rm` (Arm's
/// `FCMP_SZ_floatcmp` fixes it to 00000): another value is UNDEFINED, one
/// of the CONSTRAINED UNPREDICTABLE behaviours the Arm ARM permits.
#[test]
fn compares_with_zero_need_rm_zero() {
    for (insn, text) in [
        (0x1E20_2028, "fcmp s1, #0.0"),
        (0x1E20_2038, "fcmpe s1, #0.0"),
        (0x1E60_2028, "fcmp d1, #0.0"),
        (0x1E60_2038, "fcmpe d1, #0.0"),
    ] {
        assert_eq!(step_exception_class(insn), None, "{text} executes");
        assert_eq!(step_exception_class(insn | (0b00011 << 16)), Some(0x00), "{text} with Rm = 3: UNDEFINED");
    }
}

/// A second differential check, against disarm64 0.2 (generated from
/// binutils' opcode table: independent of Arm's data, and needing none):
/// every word the decoder hands to a handler that disarm64 allocates to no
/// encoding of a feature this core implements is UNDEFINED when it
/// executes. Arm's encodings leave reserved field values (an element size,
/// a register type, a lane index) to the decode pseudocode, where binutils'
/// rows leave them out; the handlers raise UNDEFINED for them. The words
/// that execute anyway are exactly these, each a fault in disarm64's
/// tagging of an encoding rather than a choice:
///
/// - the HINT space executes whatever the hint, so CHKFEAT (FEAT_CHK,
///   Armv9.4, a hint) is a hint here;
/// - the fixed-point conversions SCVTF, UCVTF, FCVTZS and FCVTZU by
///   immediate are tagged FEAT_FP16 for every `immh<1>` set, but only
///   `immh = 001x` is half precision: `immh<3:2>` nonzero is the single- or
///   double-precision ARMv8.0 encoding.
mod against_disarm64 {
    use super::{step_exception_class, Stepper};
    use crate::cpu::isa;
    use disarm64::InsnOpcode;
    use disarm64_defn::{InsnClass, InsnFeatureSet};
    use std::sync::Mutex;
    use std::vec::Vec;

    /// The feature sets of disarm64's table this core implements: binutils'
    /// AES set holds the 64-bit PMULL too, and its SHA2 set SHA-1 and
    /// SHA-256.
    fn implemented(set: InsnFeatureSet) -> bool {
        matches!(
            set,
            InsnFeatureSet::V8
                | InsnFeatureSet::FP
                | InsnFeatureSet::SIMD
                | InsnFeatureSet::AES
                | InsnFeatureSet::SHA2
                | InsnFeatureSet::CRC
        )
    }

    /// Which reason, if any, has `word` execute here though disarm64
    /// allocates it to no implemented encoding.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Executes {
        Allocated,
        HintSpace,
        FixedPointConversion,
        No,
    }

    fn executes(word: u32) -> Executes {
        let op = disarm64::decoder::decode(word);
        let Some(d) = op.as_ref().map(|op| op.definition()) else {
            return if word & 0xFFFF_F01F == 0xD503_201F { Executes::HintSpace } else { Executes::No };
        };
        if implemented(d.feature_set) {
            return Executes::Allocated;
        }
        if word & 0xFFFF_F01F == 0xD503_201F {
            return Executes::HintSpace;
        }
        let immh = (word >> 19) & 0xF;
        let mistagged = d.feature_set == InsnFeatureSet::SIMD_F16
            && matches!(d.class, InsnClass::ASIMDSHF | InsnClass::ASISDSHF)
            && d.mask & 0x0060_0000 == 0;
        if mistagged && immh & 0b1100 != 0 {
            return Executes::FixedPointConversion;
        }
        Executes::No
    }

    #[derive(Default)]
    struct Tally {
        words: u64,
        /// Words handed to a handler that disarm64 does not allocate.
        executed_as_undefined: u64,
        disagreements: u64,
        hint_space: u64,
        fixed_point: u64,
    }

    impl Tally {
        fn add(&mut self, other: &Tally) {
            self.words += other.words;
            self.executed_as_undefined += other.executed_as_undefined;
            self.disagreements += other.disagreements;
            self.hint_space += other.hint_space;
            self.fixed_point += other.fixed_point;
        }
    }

    fn check(stepper: &mut Stepper, word: u32, tally: &mut Tally, first: &Mutex<Vec<u32>>) {
        tally.words += 1;
        if isa::decode(word).is_none() {
            return;
        }
        match executes(word) {
            Executes::Allocated => return,
            Executes::HintSpace => {
                tally.hint_space += 1;
                return;
            }
            Executes::FixedPointConversion => {
                tally.fixed_point += 1;
                return;
            }
            Executes::No => {}
        }
        tally.executed_as_undefined += 1;
        if stepper.exception_class(word) != Some(0x00) {
            tally.disagreements += 1;
            if let Ok(mut words) = first.lock() {
                if words.len() < 32 {
                    words.push(word);
                }
            }
        }
    }

    fn report(tally: &Tally, first: &Mutex<Vec<u32>>) {
        std::eprintln!(
            "{} words; {} handed to a handler that disarm64 does not allocate, {} of them not UNDEFINED; \
             deliberate differences from disarm64: {} HINT-space, {} fixed-point conversions",
            tally.words,
            tally.executed_as_undefined,
            tally.disagreements,
            tally.hint_space,
            tally.fixed_point,
        );
        if let Ok(words) = first.lock() {
            for &word in words.iter() {
                let theirs = disarm64::decoder::decode(word).map(|op| op.definition());
                std::eprintln!(
                    "  {word:08x}: ours {:?}, exception class {:?}, disarm64 {:?}",
                    isa::decode(word),
                    step_exception_class(word),
                    theirs.map(|d| (d.mnemonic, d.opcode, d.mask, d.feature_set)),
                );
            }
        }
    }

    /// Every encoding the decoder's source names, 64 random instances
    /// each, and a million random words: fast enough for every test run.
    #[test]
    fn words_disarm64_does_not_allocate_are_undefined_on_sampled_words() {
        let mut stepper = Stepper::new();
        let mut tally = Tally::default();
        let first = Mutex::new(Vec::new());
        let mut state = 0x5EED_u64;
        let mut next = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            (z ^ (z >> 31)) as u32
        };
        let source = isa::spec::source();
        assert!(source.errors.is_empty(), "{:?}", source.errors);
        for o in source.occurrences.iter() {
            for _ in 0..64 {
                check(&mut stepper, o.cube.opcode | (next() & !o.cube.mask), &mut tally, &first);
            }
        }
        for _ in 0..1_000_000 {
            check(&mut stepper, next(), &mut tally, &first);
        }
        report(&tally, &first);
        assert_eq!(tally.disagreements, 0);
    }

    /// The same, on all 2^32 words.
    #[test]
    #[ignore = "exhaustive: every 32-bit word, on every host thread"]
    fn words_disarm64_does_not_allocate_are_undefined_on_every_word() {
        let total = Mutex::new(Tally::default());
        let first = Mutex::new(Vec::new());
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()) as u64;
        let started = std::time::Instant::now();
        std::thread::scope(|scope| {
            for t in 0..threads {
                let (total, first) = (&total, &first);
                scope.spawn(move || {
                    let mut stepper = Stepper::new();
                    let mut tally = Tally::default();
                    let mut word = t;
                    while word < 1 << 32 {
                        check(&mut stepper, word as u32, &mut tally, first);
                        word += threads;
                    }
                    if let Ok(mut total) = total.lock() {
                        total.add(&tally);
                    }
                });
            }
        });
        std::eprintln!("{threads} threads, {:.1} s", started.elapsed().as_secs_f64());
        let tally = total.into_inner().unwrap_or_default();
        report(&tally, &first);
        assert_eq!(tally.words, 1 << 32, "every word checked once");
        assert_eq!(tally.disagreements, 0);
    }
}

#[derive(Default)]
struct Tally {
    passed: usize,
    skipped: usize,
    mismatched: usize,
}

/// disarm64's disassembly of `insn`, for a failure message.
fn disassemble(insn: u32) -> String {
    disarm64::decoder::decode(insn).map(|op| format!("{op}")).unwrap_or_default()
}

/// Whether `insn` sets a should-be field otherwise, the one reason a word
/// Unicorn executed may be UNDEFINED on this core: `Ra != 11111` in SMULH and
/// UMULH, `Rm != 00000` in FCMP and FCMPE with zero. UNDEFINED is one of the
/// CONSTRAINED UNPREDICTABLE behaviours the Arm ARM permits, and the one
/// Arm's encodings give (they fix the field); Unicorn's QEMU executes the
/// word, ignoring the field (README, "Where the references differ").
fn should_be_field_set(insn: u32) -> bool {
    let mul_high_ra_set = insn & 0xFF60_8000 == 0x9B40_0000 && (insn >> 10) & 0x1F != 0x1F;
    let compare_with_zero_rm_set =
        insn & 0xFF20_FC0F == 0x1E20_2008 && (insn >> 22) & 0b11 < 0b10 && (insn >> 16) & 0x1F != 0;
    mul_high_ra_set || compare_with_zero_rm_set
}

#[test]
fn every_unicorn_vector_matches_after_one_step() {
    let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();

    for line in VECTORS.lines().filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split(' ').collect();
        let (name, kind) = (f[0], f[1]);
        let insn = u32::from_str_radix(f[2], 16).unwrap_or(0);
        let seed = u64::from_str_radix(f[3], 16).unwrap_or(0);
        let want_pc = u64::from_str_radix(f[4], 16).unwrap_or(0);
        let want_nzcv = u64::from_str_radix(f[5], 16).unwrap_or(0);
        let reg_changes = parse_pairs(f[6], "r=", 10);
        let v_changes = parse_pairs(f[7], "v=", 10);
        let mem_changes = parse_pairs(f[8], "m=", 16);
        let want_fpsr = f
            .get(9)
            .and_then(|field| field.strip_prefix("f="))
            .and_then(|hex| u64::from_str_radix(hex, 16).ok())
            .unwrap_or(0);
        let tally = tallies.entry(String::from(name)).or_default();

        let st = initial_state(seed, kind, insn);
        let mut cpu = Cpu::new(CpuConfig {
            ips,
            counter_hz: ips,
            reset_pc: CODE,
        });
        cpu.x = st.x;
        cpu.sp_el1 = st.sp;
        cpu.pstate.set_nzcv(st.nzcv);
        cpu.v = st.v;
        cpu.fpcr = st.fpcr;
        cpu.sys.cpacr_el1 = 3 << 20;
        // An exception class no step leaves, to tell an UNDEFINED word
        // (EC 0x00) from one that took no exception.
        cpu.sys.esr_el1 = 0x3F << 26;
        let mut bus = TestBus {
            insn,
            data: st.data.clone(),
        };

        match cpu.step(&mut bus, &mut NoInstrumentation) {
            Step::Stop(other) => {
                tally.mismatched += 1;
                failures.push(format!("{name} {insn:08x}: stopped with {other:?}"));
                continue;
            }
            Step::Retired | Step::Waiting => {}
        }
        if cpu.sys.esr_el1 >> 26 == 0x00 {
            if should_be_field_set(insn) {
                tally.skipped += 1;
            } else {
                tally.mismatched += 1;
                failures.push(format!("{name} {insn:08x} `{}`: UNDEFINED here, executed by Unicorn", disassemble(insn)));
            }
            continue;
        }

        let mut want_x = st.x;
        let mut want_sp = st.sp;
        for (i, val) in &reg_changes {
            if *i == 31 {
                want_sp = *val as u64;
            } else {
                want_x[*i] = *val as u64;
            }
        }
        let mut want_v = st.v;
        for (i, val) in &v_changes {
            want_v[*i] = *val;
        }
        let mut want_data = st.data.clone();
        for (off, b) in &mem_changes {
            want_data[*off] = *b as u8;
        }

        let mut diffs: Vec<String> = Vec::new();
        if cpu.pc != want_pc {
            diffs.push(format!(
                "pc {:#x} want {:#x} (esr {:#x})",
                cpu.pc, want_pc, cpu.sys.esr_el1
            ));
        }
        if cpu.pstate.nzcv() != want_nzcv {
            diffs.push(format!("nzcv {:#x} want {:#x}", cpu.pstate.nzcv(), want_nzcv));
        }
        if cpu.fpsr != want_fpsr {
            diffs.push(format!("fpsr {:#x} want {:#x}", cpu.fpsr, want_fpsr));
        }
        for (i, (got, want)) in cpu.x.iter().zip(&want_x).enumerate() {
            if got != want {
                diffs.push(format!("x{i} {got:#x} want {want:#x}"));
            }
        }
        if cpu.sp_el1 != want_sp {
            diffs.push(format!("sp {:#x} want {:#x}", cpu.sp_el1, want_sp));
        }
        for (i, (got, want)) in cpu.v.iter().zip(&want_v).enumerate() {
            if got != want {
                diffs.push(format!("v{i} {got:#034x} want {want:#034x}"));
            }
        }
        if bus.data != want_data {
            let first = bus.data.iter().zip(&want_data).position(|(a, b)| a != b).unwrap_or(0);
            diffs.push(format!(
                "data[{first:#x}] {:#x} want {:#x}",
                bus.data[first], want_data[first]
            ));
        }
        if diffs.is_empty() {
            tally.passed += 1;
        } else {
            tally.mismatched += 1;
            failures.push(format!("{name} {insn:08x} `{}`: {}", disassemble(insn), diffs.join("; ")));
        }
    }

    std::eprintln!("family                 pass   skip  MISMATCH");
    for (name, t) in &tallies {
        std::eprintln!("{name:<22} {:>5} {:>6} {:>9}", t.passed, t.skipped, t.mismatched);
    }
    for f in failures.iter().take(60) {
        std::eprintln!("MISMATCH {f}");
    }
    assert!(failures.is_empty(), "{} vectors disagree with Unicorn", failures.len());
}
