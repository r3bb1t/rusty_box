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
//! encoding the interpreter stops on as unimplemented, fails the test.

use std::collections::BTreeMap;
use std::format;
use std::string::String;
use std::vec::Vec;

use disarm64::InsnOpcode;
use disarm64_defn::InsnFeatureSet;
use rusty_box_core::{ClockHz, VmClock};

use super::decode::{classify, Decoded};
use super::{Cpu, CpuConfig, Step, StopCause};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

const VECTORS: &str = include_str!("../../tests/vectors/unicorn.txt");

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
    let cases: [(u32, &str, u64); 8] = [
        (0x0E08_0420, "dup v0.1d, v1.d[0] (Q = 0)", 0x00),
        (0x0E04_2C20, "smov w0, v1.s[0]", 0x00),
        (0x4E01_3C20, "umov x0, v1.b[0]", 0x00),
        (0x0E08_3C20, "umov w0, v1.d[0]", 0x00),
        (0x1EA0_2820, "fadd with ftype 10", 0x00),
        (0x4E08_0420, "dup v0.2d, v1.d[0]", 0x07),
        (0x4F3F_E420, "scvtf v0.4s, v1.4s, #1", 0x07),
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

#[derive(Default)]
struct Tally {
    passed: usize,
    skipped: usize,
    unimplemented: usize,
    mismatched: usize,
}

/// Why a word Unicorn executed may be UNDEFINED on this core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpectedUndefined {
    /// An optional ARMv8.0 extension QEMU's cortex-a57 implements and this
    /// core's ID_AA64ISAR0_EL1 does not advertise: AES and PMULL, SHA-1 and
    /// SHA-256, CRC32.
    AbsentExtension,
    /// SMULH/UMULH with `Ra != 0b11111`: Ra is should-be-one, and UNDEFINED
    /// is one of the CONSTRAINED UNPREDICTABLE behaviours the Arm ARM permits
    /// (QEMU executes the word, ignoring Ra).
    MulHighRaNotOnes,
}

/// The reason a word this core classifies as UNDEFINED may be, or `None`
/// when it should have executed.
fn expected_undefined(insn: u32) -> Option<ExpectedUndefined> {
    if let Some(op) = disarm64::decoder::decode(insn) {
        // The vectors come from a Cortex-A57 model, so a word in any other
        // absent feature set means disarm64 mis-tagged an ARMv8.0 encoding.
        return match op.definition().feature_set {
            InsnFeatureSet::AES | InsnFeatureSet::SHA2 | InsnFeatureSet::CRC => {
                Some(ExpectedUndefined::AbsentExtension)
            }
            _ => None,
        };
    }
    let mul_high = insn & 0xFF60_8000 == 0x9B40_0000;
    if mul_high && (insn >> 10) & 0x1F != 0x1F {
        Some(ExpectedUndefined::MulHighRaNotOnes)
    } else {
        None
    }
}

#[test]
fn every_unicorn_vector_matches_after_one_step() {
    let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();
    let mut unimplemented: BTreeMap<u32, String> = BTreeMap::new();

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

        if classify(insn) == Decoded::Undefined {
            match expected_undefined(insn) {
                Some(_) => tally.skipped += 1,
                None => {
                    tally.mismatched += 1;
                    failures.push(format!("{name} {insn:08x}: UNDEFINED here, executed by Unicorn"));
                }
            }
            continue;
        }

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
        let mut bus = TestBus {
            insn,
            data: st.data.clone(),
        };

        match cpu.step(&mut bus, &mut NoInstrumentation) {
            Step::Stop(StopCause::Unimplemented(_)) => {
                tally.unimplemented += 1;
                unimplemented.entry(insn).or_insert_with(|| String::from(name));
                continue;
            }
            Step::Stop(other) => {
                tally.mismatched += 1;
                failures.push(format!("{name} {insn:08x}: stopped with {other:?}"));
                continue;
            }
            Step::Retired | Step::Waiting => {}
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
            let text = disarm64::decoder::decode(insn).map(|op| format!("{op}")).unwrap_or_default();
            failures.push(format!("{name} {insn:08x} `{text}`: {}", diffs.join("; ")));
        }
    }

    std::eprintln!("family                 pass   skip  unimpl  MISMATCH");
    for (name, t) in &tallies {
        std::eprintln!(
            "{name:<22} {:>5} {:>6} {:>7} {:>9}",
            t.passed, t.skipped, t.unimplemented, t.mismatched
        );
    }
    if !unimplemented.is_empty() {
        std::eprintln!("unimplemented encodings ({}):", unimplemented.len());
        for (insn, name) in unimplemented.iter() {
            let text = disarm64::decoder::decode(*insn).map(|op| format!("{op}")).unwrap_or_default();
            std::eprintln!("  {name} {insn:08x} {text}");
        }
    }
    for f in failures.iter().take(60) {
        std::eprintln!("MISMATCH {f}");
    }
    assert!(failures.is_empty(), "{} vectors disagree with Unicorn", failures.len());
    assert!(unimplemented.is_empty(), "{} encodings are not implemented", unimplemented.len());
}
