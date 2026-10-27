//! Differential check against QEMU's TCG: replays `tests/vectors/unicorn.txt`,
//! produced by `tests/vectors/gen_unicorn.py` with Unicorn, on the
//! Cortex-A57, and `tests/vectors/qemu_max.txt`, the encodings of later
//! features, produced by `tests/vectors/gen_qemu.py` with QEMU's user-mode
//! `max` processor, on a processor with every feature this core
//! implements. The two files share one format and one initial state.
//! `tests/vectors/qemu_sve.txt`, from `tests/vectors/gen_qemu_sve.py`, holds
//! the SVE instructions FEAT_SME makes legal, run in Streaming SVE mode at
//! a 512-bit vector length: the same initial state with Z0-Z31 and P0-P15
//! in place of the SIMD&FP registers, and Z and P changes in place of V
//! ones ([`replay_sve`]).
//!
//! Each vector is an instruction word and a seed. The seed rebuilds the exact
//! initial state the generator gave the reference (registers, flags, vector
//! registers, a 64 KiB data region); the vector lists what the reference
//! changed. The property asserted is the guest-visible one (R9): after one
//! step, PC, NZCV, FPSR, every general and vector register, SP and every
//! data byte equal the reference's.
//!
//! A word the reference executed may be UNDEFINED here. Such a vector is
//! skipped only for the reason [`should_be_field_set`] names; any other
//! UNDEFINED word, and any step that stops the machine, fails the test.
//! Failure messages show disarm64's disassembly of the word.
//!
//! A second check here, `against_disarm64`, steps the words the decoder
//! hands to a handler but binutils' opcode table allocates to nothing this
//! core implements: each must be UNDEFINED.

use std::collections::BTreeMap;
use std::format;
use std::string::String;
use std::vec::Vec;

use rusty_box_core::{ClockHz, VmClock};

use super::features::Implementation;
use super::sve::{Pv, Zv};
use super::{Cpu, CpuConfig, El, Levels, NoEl2, Step, WithEl2};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

const VECTORS: &str = include_str!("../vectors/unicorn.txt");
const MAX_VECTORS: &str = include_str!("../vectors/qemu_max.txt");
const SVE_VECTORS: &str = include_str!("../vectors/qemu_sve.txt");

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

/// Mirrors `F16_SPECIAL` in gen_unicorn.py.
const F16_SPECIAL: [u64; 32] = [
    0x0000, 0x8000, 0x7C00, 0xFC00, 0x7E00, 0xFE12, 0x7C01, 0xFD00, 0x0001, 0x83FF, 0x0400, 0x8400,
    0x7BFF, 0xFBFF, 0x3C00, 0xBC00, 0x3800, 0xB800, 0x3E00, 0x4100, 0xC100, 0x4300, 0x6400, 0x67FF,
    0x7800, 0xF800, 0x7BFE, 0x0200, 0x3BFF, 0x3C01, 0x0401, 0x03FF,
];

/// Mirrors `ordinary_f16` in gen_unicorn.py.
fn ordinary_f16(w: u64) -> u64 {
    ((w >> 63) << 15) | ((9 + ((w >> 32) % 13)) << 10) | (w & 0x3FF)
}

/// Mirrors `BF16_SPECIAL` in gen_unicorn.py.
const BF16_SPECIAL: [u64; 24] = [
    0x0000, 0x8000, 0x7F80, 0xFF80, 0x7FC0, 0xFFC1, 0x7F81, 0xFFA0, 0x0001, 0x807F, 0x0080, 0x8080, 0x7F7F,
    0xFF7F, 0x3F80, 0xBF80, 0x3F00, 0x3FC0, 0x5F80, 0x1F80, 0x7F00, 0x0100, 0x3F81, 0xC040,
];

/// Mirrors `ordinary_bf16` in gen_unicorn.py.
fn ordinary_bf16(w: u64) -> u64 {
    ((w >> 63) << 15) | ((117 + ((w >> 32) % 21)) << 7) | (w & 0x7F)
}

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
    if kind == "F" || kind == "FA" {
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
    if kind == "H" || kind == "HA" {
        for slot in v.iter_mut() {
            let sel = r.next() & 7;
            if sel >= 2 {
                let half = sel <= 5;
                let mixed = matches!(sel, 2 | 3 | 6);
                let (count, width) = if half { (8, 16) } else { (4, 32) };
                let mut value = 0u128;
                for k in 0..count {
                    let w = r.next();
                    let lane = if mixed && w & 1 == 1 {
                        let table: &[u64] = if half { &F16_SPECIAL } else { &F32_SPECIAL };
                        table[((w >> 1) % table.len() as u64) as usize]
                    } else if half {
                        ordinary_f16(w)
                    } else {
                        ordinary_f32(w)
                    };
                    value |= u128::from(lane) << (width * k);
                }
                *slot = value;
            }
        }
        fpcr = ((r.next() & 0x1F) << 22) | ((r.next() & 1) << 19);
    }
    if kind == "B" || kind == "BA" {
        for slot in v.iter_mut() {
            let sel = r.next() & 7;
            if sel >= 2 {
                let bf16 = sel <= 5;
                let mixed = matches!(sel, 2 | 3 | 6);
                let (count, width) = if bf16 { (8, 16) } else { (4, 32) };
                let mut value = 0u128;
                for k in 0..count {
                    let w = r.next();
                    let lane = if mixed && w & 1 == 1 {
                        let table: &[u64] = if bf16 { &BF16_SPECIAL } else { &F32_SPECIAL };
                        table[((w >> 1) % table.len() as u64) as usize]
                    } else if bf16 {
                        ordinary_bf16(w)
                    } else {
                        ordinary_f32(w)
                    };
                    value |= u128::from(lane) << (width * k);
                }
                *slot = value;
            }
        }
        fpcr = (r.next() & 0x1F) << 22;
    }
    if matches!(kind, "FA" | "HA" | "BA") {
        // FEAT_AFP's FPCR.FIZ, AH and NEP at random too.
        fpcr |= r.next() & 0x7;
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

/// The Streaming SVE vector length the SVE vectors run at, in bytes.
const SVE_VLB: usize = 64;

/// The predicate length at [`SVE_VLB`], in bytes.
const SVE_PLB: usize = SVE_VLB / 8;

/// Mirrors `Z_SPECIAL` in gen_qemu_sve.py.
const Z_SPECIAL: [u64; 14] = [
    0x0000_0000_0000_0000,
    0xFFFF_FFFF_FFFF_FFFF,
    0x8000_0000_0000_0000,
    0x7FFF_FFFF_FFFF_FFFF,
    0x8000_0000_8000_0000,
    0x7FFF_FFFF_7FFF_FFFF,
    0x8000_8000_8000_8000,
    0x7FFF_7FFF_7FFF_7FFF,
    0x8080_8080_8080_8080,
    0x7F7F_7F7F_7F7F_7F7F,
    0x0000_0000_0000_0001,
    0x0101_0101_0101_0101,
    0xFFFF_FFFF_0000_0000,
    0x0000_0000_FFFF_FFFF,
];

/// Mirrors `P_ELEMENTS` in gen_qemu_sve.py.
const P_ELEMENTS: [u64; 4] = [0xFFFF_FFFF_FFFF_FFFF, 0x5555_5555_5555_5555, 0x1111_1111_1111_1111, 0x0101_0101_0101_0101];

/// A format a kind SF register's lanes may take.
struct FpFormat {
    /// A lane's width in bytes.
    width: usize,
    special: &'static [u64],
    ordinary: fn(u64) -> u64,
}

/// The formats by the `size` field that names them (00 standing for
/// BFloat16). Mirrors `FP_FORMATS` in gen_qemu_sve.py.
const FP_FORMATS: [FpFormat; 4] = [
    FpFormat { width: 2, special: &BF16_SPECIAL, ordinary: ordinary_bf16 },
    FpFormat { width: 2, special: &F16_SPECIAL, ordinary: ordinary_f16 },
    FpFormat { width: 4, special: &F32_SPECIAL, ordinary: ordinary_f32 },
    FpFormat { width: 8, special: &F64_SPECIAL, ordinary: ordinary_f64 },
];

/// Mirrors `fp_register` in gen_qemu_sve.py.
fn fp_register(r: &mut SplitMix64, size: usize) -> [u8; SVE_VLB] {
    let format = &FP_FORMATS[size];
    let mut reg = [0u8; SVE_VLB];
    for lane in reg.chunks_exact_mut(format.width) {
        let w = r.next();
        let value = if w & 1 == 1 {
            format.special[((w >> 1) % format.special.len() as u64) as usize]
        } else {
            (format.ordinary)(w)
        };
        lane.copy_from_slice(&value.to_le_bytes()[..format.width]);
    }
    reg
}

/// The scalable registers of an SVE vector's initial state.
struct SveState {
    z: [[u8; SVE_VLB]; 32],
    p: [[u8; SVE_PLB]; 16],
    /// FPCR bits the kind adds to [`State::fpcr`].
    fpcr: u64,
}

/// Must match `sve_state` in gen_qemu_sve.py exactly.
fn sve_state(seed: u64, kind: &str, insn: u32) -> SveState {
    let mut r = SplitMix64(seed ^ 0x5356_455F_5354_4154);
    let mut z = [[0u8; SVE_VLB]; 32];
    for reg in z.iter_mut() {
        let sel = r.next() & 7;
        for lane in reg.chunks_exact_mut(8) {
            let w = r.next();
            let value = match sel {
                0 => {
                    let signs = r.next();
                    (w & 0x0F0F_0F0F_0F0F_0F0F) | ((signs & 0x0101_0101_0101_0101) * 0xF0)
                }
                1 => w & 0x0707_0707_0707_0707,
                2 => Z_SPECIAL[(w % Z_SPECIAL.len() as u64) as usize],
                3 => 0,
                _ => w,
            };
            lane.copy_from_slice(&value.to_le_bytes());
        }
    }
    let mut p = [[0u8; SVE_PLB]; 16];
    for reg in p.iter_mut() {
        let sel = r.next() & 7;
        let w = r.next();
        let value = match sel {
            0 => w,
            1 => u64::MAX,
            2 => 0,
            3..=6 => P_ELEMENTS[sel as usize - 3],
            _ => w & P_ELEMENTS[(r.next() & 3) as usize],
        };
        reg.copy_from_slice(&value.to_le_bytes());
    }
    let mut fpcr = if kind == "SA" { (r.next() & 1) << 1 } else { 0 };
    if kind == "SF" {
        let own = ((insn >> 22) & 3) as usize;
        for reg in z.iter_mut() {
            let sel = (r.next() & 7) as usize;
            if sel != 0 {
                *reg = fp_register(&mut r, if sel <= 4 { own } else { sel - 4 });
            }
        }
        fpcr = ((r.next() & 0x1F) << 22) | ((r.next() & 1) << 19) | (r.next() & 7);
    }
    SveState { z, p, fpcr }
}

/// `key:value` pairs of an SVE vector's `z=` or `p=` field: the register
/// number in decimal, its value as one big-endian hex number `len` bytes
/// wide, returned little-endian like the register. A malformed pair fails
/// the test: dropping it would leave the register's expected value its
/// initial one.
fn parse_registers(field: &str, prefix: &str, len: usize) -> Vec<(usize, Vec<u8>)> {
    let Some(body) = field.strip_prefix(prefix) else { panic!("`{field}` is no `{prefix}` field") };
    if body == "-" {
        return Vec::new();
    }
    body.split(',')
        .map(|pair| {
            let Some((k, hex)) = pair.split_once(':') else { panic!("`{pair}` in `{prefix}` is no `n:value` pair") };
            assert_eq!(hex.len(), 2 * len, "`{pair}` in `{prefix}` is not {len} bytes");
            let bytes = (0..len)
                .map(|i| {
                    let at = 2 * (len - 1 - i);
                    match u8::from_str_radix(&hex[at..at + 2], 16) {
                        Ok(byte) => byte,
                        Err(e) => panic!("`{pair}` in `{prefix}`: {e}"),
                    }
                })
                .collect();
            match k.parse() {
                Ok(n) => (n, bytes),
                Err(e) => panic!("`{pair}` in `{prefix}`: {e}"),
            }
        })
        .collect()
}

/// Step each SVE vector once at EL1 on a processor implementing every
/// feature and the levels `L`, in Streaming SVE mode at a 512-bit vector
/// length (`SMCR_EL1.LEN = 3`), and fail unless every one ends in the state
/// the reference left: PC, NZCV, FPSR, the general registers, SP, Z0-Z31,
/// P0-P15 and the data region.
fn replay_sve<L: Levels>(vectors: &str) {
    let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();

    for line in vectors.lines().filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split(' ').collect();
        let (name, kind) = (f[0], f[1]);
        let insn = u32::from_str_radix(f[2], 16).unwrap_or(0);
        let seed = u64::from_str_radix(f[3], 16).unwrap_or(0);
        let want_pc = u64::from_str_radix(f[4], 16).unwrap_or(0);
        let want_nzcv = u64::from_str_radix(f[5], 16).unwrap_or(0);
        let reg_changes = parse_pairs(f[6], "r=", 10);
        let z_changes = parse_registers(f[7], "z=", SVE_VLB);
        let p_changes = parse_registers(f[8], "p=", SVE_PLB);
        let mem_changes = parse_pairs(f[9], "m=", 16);
        let want_fpsr = f
            .get(10)
            .and_then(|field| field.strip_prefix("f="))
            .and_then(|hex| u64::from_str_radix(hex, 16).ok())
            .unwrap_or(0);
        let tally = tallies.entry(String::from(name)).or_default();

        let st = initial_state(seed, kind, insn);
        let sve = sve_state(seed, kind, insn);
        let mut cpu = Cpu::<L>::implementing(
            CpuConfig {
                ips,
                counter_hz: ips,
                reset_pc: CODE,
            },
            Implementation::EVERY_FEATURE,
        );
        enter_el1(&mut cpu);
        cpu.x = st.x;
        cpu.sp_el1 = st.sp;
        cpu.pstate.set_nzcv(st.nzcv);
        cpu.sys.cpacr_el1 = (3 << 24) | (3 << 20);
        cpu.sys.smcr_el1 = 3;
        // With EL2, `SMCR_EL2.LEN` caps the length EL1 runs at (Arm
        // `CurrentSVL`): a hypervisor grants its guest the same.
        if let Some(el2) = cpu.el2_mut() {
            el2.smcr = 3;
        }
        cpu.set_pstate_sm(true);
        cpu.fpcr = st.fpcr | sve.fpcr;
        cpu.fpsr = 0;
        let vl = cpu.current_svl();
        for (n, bytes) in sve.z.iter().enumerate() {
            cpu.set_z(n as u32, vl, &Zv::from_parts(u128::from_le_bytes(bytes[..16].try_into().unwrap_or([0; 16])), &bytes[16..]));
        }
        for (n, bytes) in sve.p.iter().enumerate() {
            cpu.set_p(n as u32, vl, &Pv::from_bytes(bytes));
        }
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
            tally.mismatched += 1;
            failures.push(format!("{name} {insn:08x} `{}`: UNDEFINED here, executed by the reference", disassemble(insn)));
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
        let mut want_z: Vec<Vec<u8>> = sve.z.iter().map(|b| b.to_vec()).collect();
        for (i, bytes) in &z_changes {
            want_z[*i] = bytes.clone();
        }
        let mut want_p: Vec<Vec<u8>> = sve.p.iter().map(|b| b.to_vec()).collect();
        for (i, bytes) in &p_changes {
            want_p[*i] = bytes.clone();
        }
        let mut want_data = st.data.clone();
        for (off, b) in &mem_changes {
            want_data[*off] = *b as u8;
        }

        let mut diffs: Vec<String> = Vec::new();
        if cpu.pc != want_pc {
            diffs.push(format!("pc {:#x} want {:#x} (esr {:#x})", cpu.pc, want_pc, cpu.sys.esr_el1));
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
        for (i, want) in want_z.iter().enumerate() {
            let got = cpu.z(i as u32, vl);
            let got: Vec<u8> = (0..SVE_VLB).map(|b| got.byte(b)).collect();
            if &got != want {
                let at = got.iter().zip(want).position(|(a, b)| a != b).unwrap_or(0);
                diffs.push(format!("z{i} byte {at} {:#x} want {:#x}", got[at], want[at]));
            }
        }
        for (i, want) in want_p.iter().enumerate() {
            let got = cpu.p(i as u32, vl);
            let got: Vec<u8> = (0..SVE_PLB).map(|b| got.byte(b)).collect();
            if &got != want {
                diffs.push(format!("p{i} {got:02x?} want {want:02x?}"));
            }
        }
        if bus.data != want_data {
            let first = bus.data.iter().zip(&want_data).position(|(a, b)| a != b).unwrap_or(0);
            diffs.push(format!("data[{first:#x}] {:#x} want {:#x}", bus.data[first], want_data[first]));
        }
        if diffs.is_empty() {
            tally.passed += 1;
        } else {
            tally.mismatched += 1;
            failures.push(format!("{name} {insn:08x} `{}`: {}", disassemble(insn), diffs.join("; ")));
        }
    }

    std::eprintln!("family                       pass  MISMATCH");
    for (name, t) in &tallies {
        std::eprintln!("{name:<28} {:>5} {:>9}", t.passed, t.mismatched);
    }
    for f in failures.iter().take(60) {
        std::eprintln!("MISMATCH {f}");
    }
    assert!(failures.is_empty(), "{} SVE vectors disagree with the reference", failures.len());
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
        let mut cpu = Cpu::<NoEl2>::new(CpuConfig {
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
        let mut cpu = Cpu::<NoEl2>::new(CpuConfig {
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
        let mut cpu = Cpu::<NoEl2>::new(CpuConfig {
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
    cpu: Cpu<NoEl2>,
    reset: Vec<u8>,
}

impl Stepper {
    fn new() -> Self {
        let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
        let mut cpu = Cpu::<NoEl2>::new(CpuConfig {
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
    replay::<NoEl2>(VECTORS, Implementation::CORTEX_A57);
}

#[test]
fn every_qemu_max_vector_matches_after_one_step_with_every_feature() {
    replay::<NoEl2>(MAX_VECTORS, Implementation::EVERY_FEATURE);
}

#[test]
fn every_qemu_sve_vector_matches_after_one_step_in_streaming_mode() {
    replay_sve::<NoEl2>(SVE_VECTORS);
}

/// At EL1 on a processor with EL2, whose controls out of reset trap nothing
/// EL1 executes and route nothing it raises to EL2, every vector ends as it
/// does without EL2 (the SVE ones with `SMCR_EL2.LEN` granting EL1 the
/// length they run at).
#[test]
fn every_vector_matches_at_el1_under_el2_out_of_reset() {
    replay::<WithEl2>(VECTORS, Implementation::CORTEX_A57);
    replay::<WithEl2>(MAX_VECTORS, Implementation::EVERY_FEATURE);
    replay_sve::<WithEl2>(SVE_VECTORS);
}

/// `cpu` at EL1 with `SP_EL1` selected, where every vector starts: a
/// processor with EL2 comes out of reset at EL2, one without at EL1.
fn enter_el1<L: Levels>(cpu: &mut Cpu<L>) {
    cpu.pstate.el = El::El1;
    cpu.pstate.sp_sel = true;
}

/// Step each vector of `vectors` once at EL1 on a processor implementing
/// `imp` and the levels `L`, and fail unless every one ends in the state
/// the reference left.
fn replay<L: Levels>(vectors: &str, imp: Implementation) {
    let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();

    for line in vectors.lines().filter(|l| !l.is_empty()) {
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
        let mut cpu = Cpu::<L>::implementing(
            CpuConfig {
                ips,
                counter_hz: ips,
                reset_pc: CODE,
            },
            imp,
        );
        enter_el1(&mut cpu);
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
                failures.push(format!("{name} {insn:08x} `{}`: UNDEFINED here, executed by the reference", disassemble(insn)));
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
    assert!(failures.is_empty(), "{} vectors disagree with the reference", failures.len());
}
