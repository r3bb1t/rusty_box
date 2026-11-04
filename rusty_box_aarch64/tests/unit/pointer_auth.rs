//! FEAT_PAuth, FEAT_PAuth2, FEAT_FPAC and FEAT_FPACCOMBINE with QARMA5, as
//! a guest sees them.
//!
//! [`every_qemu_pauth_vector_matches_after_one_step`] replays
//! `tests/vectors/qemu_pauth.txt`, which `tests/vectors/gen_qemu_pauth.py`
//! made with QEMU's system emulator under keys, key enables and TCR_EL1
//! fields of its own choosing, on a processor with every feature this core
//! implements: the codes QARMA5 computes, where they go in a pointer, what
//! an authentication restores and when it faults.
//!
//! The rest holds what QEMU's `max` cannot show (R9), each pinned to the
//! Arm ARM's pseudocode (`InsertPAC`, `Authenticate`, `EffectiveTBI`,
//! `AArch64.BranchAddr`, the instruction pages): the processors without
//! FEAT_PAuth2, FEAT_FPAC or FEAT_FPACCOMBINE and without FEAT_PAuth at
//! all, the pointer QEMU signs for the wrong half of the address space
//! (to 11.1.2 at least), the SP alignment check QEMU does not make, the
//! keys as registers and in a snapshot, and a tag TCR_EL1.TBID keeps for
//! the fetch to fault on.
//!
//! The words are GNU as's (`-march=armv8.3-a`, objdump -d).

use std::format;
use std::string::String;
use std::vec::Vec;

use rusty_box_core::{ClockHz, VmClock};

use super::features::{Feature, Features, Implementation};
use super::pauth::PacKeys;
use super::step::{exec, processor, Taken, CODE, UNDEFINED};
use super::{id, Cpu, CpuConfig, El, NoEl2, Step};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

const QEMU_VECTORS: &str = include_str!("../vectors/qemu_pauth.txt");

// tests/vectors/qemu_pauth.ld's layout.
const VBAR: u64 = 0x4018_0000;
const TEST_PC: u64 = 0x4020_0010;
const LAND: u64 = 0x4030_0000;
const DATA: u64 = 0x4040_0000;
const DATA_WORDS: u64 = 0x400;

/// The `SPSR_EL1` the runner gives every case: EL1h, every interrupt
/// masked.
const SPSR: u64 = 0x3C5;
/// The vector of a synchronous exception taken from EL1 with `SP_EL1`.
const SYNC_CURRENT_SPX: u64 = VBAR + 0x200;

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

/// What a case's seed gives: gen_qemu_pauth.py's `seeded`.
struct Seeded {
    keys: [u64; 10],
    x: [u64; 31],
    elr: u64,
    sp: u64,
}

impl Seeded {
    fn new(seed: u64) -> Self {
        let mut r = SplitMix64(seed);
        let keys = core::array::from_fn(|_| r.next());
        let x = core::array::from_fn(|_| r.next());
        let elr = r.next();
        let sp = r.next() & !0xF;
        Self { keys, x, elr, sp }
    }
}

/// The data region's doubleword `i`: gen_qemu_pauth.py's `data_word`.
fn data_word(i: u64) -> u64 {
    (i + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// The word under test at [`TEST_PC`], the data region, and every other
/// access a bus fault.
struct RunnerBus {
    insn: u32,
}

impl PhysBus for RunnerBus {
    fn read(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        _clock: VmClock,
    ) -> Result<u64, BusFault> {
        let a = addr.get();
        let n = width.bytes();
        if a >= TEST_PC && a + n <= TEST_PC + 4 {
            return Ok(u64::from(self.insn >> (8 * (a - TEST_PC))) & width.value_mask());
        }
        if a >= DATA && a + n <= DATA + 8 * DATA_WORDS && a.is_multiple_of(8) && n == 8 {
            return Ok(data_word((a - DATA) / 8));
        }
        Err(BusFault::Unmapped)
    }

    fn write(
        &mut self,
        _addr: PhysAddr,
        _width: AccessWidth,
        _value: u64,
        _clock: VmClock,
    ) -> Result<(), BusFault> {
        Err(BusFault::Unmapped)
    }
}

/// The value after `prefix` among `fields`.
fn field<'a>(fields: &[&'a str], prefix: &str) -> &'a str {
    fields
        .iter()
        .find_map(|f| f.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("no `{prefix}` in {fields:?}"))
}

fn hex(text: &str) -> u64 {
    u64::from_str_radix(text, 16).unwrap_or_else(|e| panic!("`{text}`: {e}"))
}

/// A register number (decimal) and its value (hex), as `n:value` pairs.
struct RegValue {
    reg: usize,
    value: u64,
}

fn reg_values(text: &str) -> Vec<RegValue> {
    if text == "-" {
        return Vec::new();
    }
    text.split(',')
        .map(|pair| {
            let (reg, value) = pair.split_once(':').unwrap_or_else(|| panic!("`{pair}`"));
            RegValue {
                reg: reg.parse().unwrap_or_else(|e| panic!("`{reg}`: {e}")),
                value: hex(value),
            }
        })
        .collect()
}

/// X0-X30, SP (31) and ELR_EL1 (32) of `cpu`.
fn registers(cpu: &Cpu<NoEl2>) -> [u64; 33] {
    core::array::from_fn(|i| match i {
        0..=30 => cpu.x[i],
        31 => cpu.sp_el1,
        _ => cpu.sys.elr_el1,
    })
}

#[test]
fn every_qemu_pauth_vector_matches_after_one_step() {
    let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
    let mut failures: Vec<String> = Vec::new();
    let mut count = 0;
    for line in QEMU_VECTORS.lines().filter(|l| !l.is_empty()) {
        count += 1;
        let fields: Vec<&str> = line.split(' ').collect();
        let (name, insn) = (fields[0], hex(fields[1]) as u32);
        let seeded = Seeded::new(hex(fields[2]));

        let mut cpu = Cpu::implementing(
            CpuConfig {
                ips,
                reset_pc: TEST_PC,
            },
            Implementation {
                counter_hz: ips,
                ..Implementation::EVERY_FEATURE
            },
        );
        cpu.pac_keys = PacKeys::from_halves(seeded.keys);
        cpu.sys.sctlr_el1 = hex(field(&fields, "s="));
        cpu.sys.tcr_el1 = hex(field(&fields, "t="));
        cpu.sys.spsr_el1 = SPSR;
        cpu.sys.vbar_el1 = VBAR;
        cpu.sys.esr_el1 = 0x3F << 26;
        cpu.pstate.set_nzcv(hex(field(&fields, "n=")));
        cpu.x = seeded.x;
        cpu.sp_el1 = seeded.sp;
        cpu.sys.elr_el1 = seeded.elr;
        for RegValue { reg, value } in reg_values(field(&fields, "i=")) {
            match reg {
                0..=30 => cpu.x[reg] = value,
                31 => cpu.sp_el1 = value,
                _ => cpu.sys.elr_el1 = value,
            }
        }
        let before = registers(&cpu);

        let step = cpu.step(&mut RunnerBus { insn }, &mut NoInstrumentation);
        let mut diffs: Vec<String> = Vec::new();
        if step != Step::Retired {
            diffs.push(format!("stepped {step:?}"));
        }
        let (want_pc, want_esr) = match field(&fields, "o=") {
            "N" => (TEST_PC + 4, None),
            "L" => (LAND, None),
            _ => (SYNC_CURRENT_SPX, Some(hex(field(&fields, "c=")))),
        };
        if cpu.pc != want_pc {
            diffs.push(format!(
                "pc {:#x} want {want_pc:#x} (esr {:#x})",
                cpu.pc, cpu.sys.esr_el1
            ));
        }
        match want_esr {
            Some(esr) => {
                if cpu.sys.esr_el1 != esr {
                    diffs.push(format!("esr {:#x} want {esr:#x}", cpu.sys.esr_el1));
                }
                if cpu.sys.elr_el1 != TEST_PC {
                    diffs.push(format!("elr {:#x} want {TEST_PC:#x}", cpu.sys.elr_el1));
                }
            }
            None => {
                let mut want = before;
                for RegValue { reg, value } in reg_values(field(&fields, "r=")) {
                    want[reg] = value;
                }
                let got = registers(&cpu);
                // ELR_EL1 is compared above for an exception, which writes it.
                for (reg, (g, w)) in got.iter().zip(want.iter()).enumerate().take(32) {
                    if g != w {
                        diffs.push(format!("r{reg} {g:#x} want {w:#x}"));
                    }
                }
                let nzcv = hex(field(&fields, "f="));
                if cpu.pstate.nzcv() != nzcv {
                    diffs.push(format!("nzcv {:#x} want {nzcv:#x}", cpu.pstate.nzcv()));
                }
            }
        }
        if !diffs.is_empty() {
            failures.push(format!("{name} {insn:08x}: {}", diffs.join("; ")));
        }
    }
    assert!(count > 2000, "{count} vectors");
    assert!(
        failures.is_empty(),
        "{} of {count} vectors differ:\n{}",
        failures.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ----- what the vectors cannot show -----

/// `pacia x0, x1`, `pacib x0, x1`, `pacda x0, x1`, `pacdb x0, x1`.
const PACIA: u32 = 0xDAC1_0020;
const PACIB: u32 = 0xDAC1_0420;
const PACDA: u32 = 0xDAC1_0820;
const PACDB: u32 = 0xDAC1_0C20;
/// `autia x0, x1`, `autib x0, x1`, `autda x0, x1`, `autdb x0, x1`.
const AUTIA: u32 = 0xDAC1_1020;
const AUTIB: u32 = 0xDAC1_1420;
const AUTDA: u32 = 0xDAC1_1820;
const AUTDB: u32 = 0xDAC1_1C20;
/// `paciza x0`, `pacdza x0`, `pacdzb x0`, `xpaci x0`, `pacga x0, x1, x2`.
const PACIZA: u32 = 0xDAC1_23E0;
const PACDZA: u32 = 0xDAC1_2BE0;
const PACDZB: u32 = 0xDAC1_2FE0;
const XPACI: u32 = 0xDAC1_43E0;
const PACGA: u32 = 0x9AC2_3020;
/// `paciasp`, `autiasp`, `xpaclri`, `pacia1716`.
const PACIASP: u32 = 0xD503_233F;
const AUTIASP: u32 = 0xD503_23BF;
const XPACLRI: u32 = 0xD503_20FF;
const PACIA1716: u32 = 0xD503_211F;
/// `braa x0, x1`, `blraa x0, x1`, `braaz x0`, `retaa`, `eretaa`.
const BRAA: u32 = 0xD71F_0801;
const BLRAA: u32 = 0xD73F_0801;
const BRAAZ: u32 = 0xD61F_081F;
const RETAA: u32 = 0xD65F_0BFF;
const ERETAA: u32 = 0xD69F_0BFF;
/// `ldraa x0, [x1, #8]`, `ldraa x0, [sp]`, `ldrab x0, [x1, #-8]!`.
const LDRAA: u32 = 0xF820_1420;
const LDRAA_SP: u32 = 0xF820_07E0;
const LDRAB_PRE: u32 = 0xF8FF_FC20;
/// `mrs x0, apiakeylo_el1`, `msr apiakeylo_el1, x0`, `msr apgakeyhi_el1,
/// x0`.
const MRS_APIAKEYLO: u32 = 0xD538_2100;
const MSR_APIAKEYLO: u32 = 0xD518_2100;
const MSR_APGAKEYHI: u32 = 0xD518_2320;
/// `ldr x0, [x1]`, `br x1`, `nop`.
const LDR: u32 = 0xF940_0020;
const BR_X1: u32 = 0xD61F_0020;
const NOP: u32 = 0xD503_201F;

/// `SCTLR_EL1.EnIA`, `EnIB`, `EnDA`, `EnDB`, and `SA`.
const EN_IA: u64 = 1 << 31;
const EN_IB: u64 = 1 << 30;
const EN_DA: u64 = 1 << 27;
const EN_DB: u64 = 1 << 13;
const SA: u64 = 1 << 3;

/// `TCR_EL1.TBI0`, `TBID0`, and `EPD1`.
const TBI0: u64 = 1 << 37;
const TBID0: u64 = 1 << 51;
const EPD1: u64 = 1 << 23;
/// T0SZ = 25 and T1SZ = 20: the lower half's code starts at bit 39, the
/// upper half's at bit 44.
const TXSZ: u64 = 25 | (20 << 16);

/// The PAC Fail exception: data key in `ISS[1]`, key B in `ISS[0]`.
fn pac_fail(data: bool, key_b: bool) -> Option<Taken> {
    Some(Taken {
        ec: 0x1C,
        iss: (u64::from(data) << 1) | u64::from(key_b),
    })
}

/// Bits `hi:lo` as a mask.
const fn mask(hi: u32, lo: u32) -> u64 {
    (u64::MAX >> (63 - hi)) & !((1u64 << lo) - 1)
}

/// The bits of a pointer of the lower half (T0SZ = 25) that hold a code
/// without top-byte-ignore.
const LOWER_CODE: u64 = mask(63, 56) | mask(54, 39);

/// A processor implementing `features`, the keys set to distinct values,
/// every key enabled, TCR_EL1 = [`TXSZ`].
fn keyed(features: &[Feature]) -> Cpu<NoEl2> {
    let mut cpu = processor(Implementation {
        features: Features::of(features),
        ..Implementation::CORTEX_A57
    });
    cpu.pac_keys = PacKeys::from_halves(core::array::from_fn(|i| {
        0x0123_4567_89AB_CDEF_u64
            .wrapping_mul(2 * i as u64 + 1)
            .rotate_left(i as u32 * 7)
    }));
    cpu.sys.sctlr_el1 |= EN_IA | EN_IB | EN_DA | EN_DB;
    cpu.sys.tcr_el1 = TXSZ;
    cpu
}

const PAUTH: [Feature; 3] = [Feature::Base, Feature::PAuth, Feature::PacQarma5];
const PAUTH2: [Feature; 4] = [
    Feature::Base,
    Feature::PAuth,
    Feature::PAuth2,
    Feature::PacQarma5,
];
const FPAC: [Feature; 5] = [
    Feature::Base,
    Feature::PAuth,
    Feature::PAuth2,
    Feature::Fpac,
    Feature::PacQarma5,
];
const FPACCOMBINE: [Feature; 6] = [
    Feature::Base,
    Feature::PAuth,
    Feature::PAuth2,
    Feature::Fpac,
    Feature::FpacCombine,
    Feature::PacQarma5,
];

/// X0 after `insn` with X0 = `ptr` and X1 = `modifier`, which must take no
/// exception.
fn apply(cpu: &mut Cpu<NoEl2>, insn: u32, ptr: u64, modifier: u64) -> u64 {
    cpu.x[0] = ptr;
    cpu.x[1] = modifier;
    assert_eq!(exec(cpu, insn), None, "{insn:#010x} of {ptr:#x}");
    cpu.x[0]
}

/// A pointer whose bit 55 and bit 63 differ is signed for the half bit 63
/// names unless a top-byte-ignore applies to the access, in either half,
/// and then for the half bit 55 names (`InsertPAC`'s `selbit`): the code
/// is the one of the pointer of that half with the same low bits, the
/// pointer's own bits XORed over it, and bit 55 that half's. QEMU, to
/// 11.1.2 at least, takes bit 55's half always (README, "Where the
/// references differ").
#[test]
fn a_pointer_whose_bits_55_and_63_differ_is_signed_for_insertpacs_half() {
    let mut cpu = keyed(&FPACCOMBINE);
    let awkward = 0x7F80_0000_0000_1000;
    let lower = 0x1000;
    for (insn, tcr) in [(PACDA, TXSZ), (PACIA, TXSZ), (PACIA, TXSZ | TBI0 | TBID0)] {
        cpu.sys.tcr_el1 = tcr;
        let twin = apply(&mut cpu, insn, lower, 0x55);
        let signed = apply(&mut cpu, insn, awkward, 0x55);
        assert_eq!(
            signed,
            twin ^ ((awkward ^ lower) & LOWER_CODE),
            "{insn:#010x} TCR {tcr:#x}"
        );
        assert_eq!(signed & (1 << 55), 0, "bit 55 names the lower half");
    }
    // TBI0 makes a data pointer's half bit 55's, though the upper half
    // has no top-byte-ignore: the code of the upper half's pointer.
    cpu.sys.tcr_el1 = TXSZ | TBI0;
    let awkward = 0x0080_0000_0000_1000;
    let upper = 0xFFFF_F000_0000_1000;
    let upper_code = mask(63, 56) | mask(54, 44);
    let twin = apply(&mut cpu, PACDA, upper, 0x55);
    assert_eq!(
        apply(&mut cpu, PACDA, awkward, 0x55),
        twin ^ ((awkward ^ upper) & upper_code)
    );
}

/// Without FEAT_PAuth2 the code replaces the pointer's bits rather than
/// being XORed over them, so a pointer of the lower half signs alike and
/// one of the upper half to the complement; either way authenticating
/// restores the pointer.
#[test]
fn without_pauth2_the_code_replaces_the_pointers_bits() {
    let mut pauth = keyed(&PAUTH);
    let mut pauth2 = keyed(&PAUTH2);
    let upper_code = mask(63, 56) | mask(54, 44);
    for (ptr, code) in [
        (0x0000_0012_3456_7000, 0),
        (0xFFFF_F012_3456_7000, upper_code),
    ] {
        for (sign, auth) in [
            (PACIA, AUTIA),
            (PACIB, AUTIB),
            (PACDA, AUTDA),
            (PACDB, AUTDB),
        ] {
            let replaced = apply(&mut pauth, sign, ptr, 0x77);
            assert_eq!(
                replaced,
                apply(&mut pauth2, sign, ptr, 0x77) ^ code,
                "{sign:#010x} of {ptr:#x}"
            );
            assert_eq!(apply(&mut pauth, auth, replaced, 0x77), ptr, "{auth:#010x}");
        }
    }
}

/// Without FEAT_PAuth2, signing a pointer whose extension is not all
/// zeros or all ones flips the code's bit below the top of the extension
/// (`InsertPAC`): bit 62, or bit 54 under top-byte-ignore.
#[test]
fn without_pauth2_a_noncanonical_pointer_gets_a_corrupted_code() {
    let mut cpu = keyed(&PAUTH);
    let noncanonical = 0x0000_4000_0000_1000;
    let canonical = 0x1000;
    assert_eq!(
        apply(&mut cpu, PACIA, noncanonical, 9),
        apply(&mut cpu, PACIA, canonical, 9) ^ (1 << 62)
    );
    cpu.sys.tcr_el1 = TXSZ | TBI0;
    let (tagged, tagged_canonical) = (0x5A00_4000_0000_1000, 0x5A00_0000_0000_1000);
    assert_eq!(
        apply(&mut cpu, PACDA, tagged, 9),
        apply(&mut cpu, PACDA, tagged_canonical, 9) ^ (1 << 54)
    );
}

/// Without FEAT_PAuth2 a failed authentication restores the extension and
/// writes the error code `key_number:NOT(key_number)` below its top bit:
/// bits 62:61, or 54:53 under top-byte-ignore. It takes no exception.
#[test]
fn without_pauth2_a_failed_authentication_writes_the_error_code() {
    let mut cpu = keyed(&PAUTH);
    for (sign, auth, error) in [
        (PACIA, AUTIA, 1 << 61),
        (PACIB, AUTIB, 1 << 62),
        (PACDB, AUTDB, 1 << 62),
    ] {
        let broken = apply(&mut cpu, sign, 0x1000, 3) ^ (1 << 50);
        assert_eq!(
            apply(&mut cpu, auth, broken, 3),
            0x1000 | error,
            "{auth:#010x}"
        );
    }
    cpu.sys.tcr_el1 = TXSZ | TBI0;
    let broken = apply(&mut cpu, PACDA, 0x5A00_0000_0000_1000, 3) ^ (1 << 50);
    assert_eq!(apply(&mut cpu, AUTDA, broken, 3), 0x5A20_0000_0000_1000);
}

/// FEAT_FPAC faults an authentication alone (AUT*) that fails, naming the
/// key; a branch that authenticates goes on to the corrupted address,
/// whose fetch faults, unless FEAT_FPACCOMBINE faults it too. With
/// FEAT_PAuth2 alone nothing faults: the pointer comes back with its code
/// XORed away, an extension that is not one.
#[test]
fn fpac_faults_a_failed_authentication_and_fpaccombine_a_combined_one_too() {
    let mut fpac = keyed(&FPAC);
    for (auth, data, key_b) in [
        (AUTIA, false, false),
        (AUTIB, false, true),
        (AUTDA, true, false),
        (AUTDB, true, true),
    ] {
        fpac.x[0] = 0x1000;
        assert_eq!(exec(&mut fpac, auth), pac_fail(data, key_b), "{auth:#010x}");
        assert_eq!(fpac.x[0], 0x1000, "an exception writes no register");
    }
    fpac.x[0] = 0x1000;
    assert_eq!(exec(&mut fpac, BRAA), None);
    let corrupted = fpac.pc;
    assert_ne!(corrupted & mask(63, 39), 0, "{corrupted:#x}");
    let mut ram = Ram::new();
    assert_eq!(
        ram.step(&mut fpac),
        Some(Abort {
            ec: 0x21,
            fsc: 0,
            far: corrupted
        }),
        "the fetch from the corrupted address: an address size fault, the MMU off"
    );

    let mut combined = keyed(&FPACCOMBINE);
    combined.x[0] = 0x1000;
    assert_eq!(exec(&mut combined, BRAA), pac_fail(false, false));

    let mut pauth2 = keyed(&PAUTH2);
    let restored = apply(&mut pauth2, AUTIA, 0x1000, 0x55);
    assert_ne!(restored & mask(54, 39), 0, "{restored:#x}");
}

/// A disabled key leaves the pointer as it is: signing and authenticating
/// with it change nothing and fault on nothing, and a branch goes to the
/// register's own address. PACGA has no enable.
#[test]
fn a_disabled_key_leaves_the_pointer_alone() {
    let mut cpu = keyed(&FPACCOMBINE);
    let ga = {
        cpu.x[1] = 0x1234;
        cpu.x[2] = 0x5678;
        assert_eq!(exec(&mut cpu, PACGA), None);
        cpu.x[0]
    };
    assert_eq!(ga & 0xFFFF_FFFF, 0, "the code in the top 32 bits");
    cpu.sys.sctlr_el1 &= !(EN_IA | EN_IB | EN_DA | EN_DB);
    for insn in [PACIA, PACIB, PACDA, PACDB, AUTIA, AUTIB, AUTDA, AUTDB] {
        assert_eq!(
            apply(&mut cpu, insn, 0x00AB_0000_0000_1000, 3),
            0x00AB_0000_0000_1000,
            "{insn:#010x}"
        );
    }
    cpu.x[0] = 0x2000;
    assert_eq!(exec(&mut cpu, BRAA), None);
    assert_eq!(cpu.pc, 0x2000);
    cpu.x[1] = 0x1234;
    cpu.x[2] = 0x5678;
    assert_eq!(exec(&mut cpu, PACGA), None);
    assert_eq!(cpu.x[0], ga);
}

/// Without FEAT_PAuth the hints are NOPs (`EndOfDecode(Decode_NOP)`), and
/// every other encoding and the key registers UNDEFINED.
#[test]
fn without_pauth_the_hints_are_nops_and_the_rest_undefined() {
    let mut cpu = processor(Implementation::CORTEX_A57);
    cpu.x[16] = 0x16;
    cpu.x[17] = 0x0000_0000_0000_1717;
    cpu.x[30] = 0x0000_0000_0000_3030;
    for hint in [PACIASP, AUTIASP, XPACLRI, PACIA1716] {
        assert_eq!(exec(&mut cpu, hint), None, "{hint:#010x}");
        assert_eq!(cpu.pc, CODE + 4);
        assert_eq!((cpu.x[17], cpu.x[30]), (0x1717, 0x3030), "{hint:#010x}");
    }
    for insn in [
        PACIA,
        AUTDB,
        PACIZA,
        XPACI,
        PACGA,
        BRAA,
        BLRAA,
        BRAAZ,
        RETAA,
        ERETAA,
        LDRAA,
        MRS_APIAKEYLO,
        MSR_APIAKEYLO,
    ] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}

/// With FEAT_PAuth the hints sign and authenticate X30 under SP, X17
/// under X16, and XPACLRI strips X30.
#[test]
fn the_hints_use_x30_and_sp_or_x17_and_x16() {
    let mut cpu = keyed(&FPACCOMBINE);
    cpu.sp_el1 = 0xABC0;
    cpu.x[30] = 0x1000;
    assert_eq!(exec(&mut cpu, PACIASP), None);
    let signed = cpu.x[30];
    assert_eq!(signed, apply(&mut cpu, PACIA, 0x1000, 0xABC0));
    cpu.x[30] = signed;
    assert_eq!(exec(&mut cpu, XPACLRI), None);
    assert_eq!(cpu.x[30], 0x1000);
    cpu.x[30] = signed;
    assert_eq!(exec(&mut cpu, AUTIASP), None);
    assert_eq!(cpu.x[30], 0x1000);
    cpu.x[16] = 0x1616;
    cpu.x[17] = 0x1000;
    assert_eq!(exec(&mut cpu, PACIA1716), None);
    let signed = cpu.x[17];
    assert_eq!(signed, apply(&mut cpu, PACIA, 0x1000, 0x1616));
}

/// The keys are EL1 registers: an `MSR` sets one, an `MRS` reads it back,
/// the instructions use it, and EL0 can do neither.
#[test]
fn the_keys_are_el1_registers() {
    let mut cpu = keyed(&FPACCOMBINE);
    cpu.x[0] = 0xDEAD_BEEF;
    assert_eq!(exec(&mut cpu, MSR_APIAKEYLO), None);
    cpu.x[0] = 0;
    assert_eq!(exec(&mut cpu, MRS_APIAKEYLO), None);
    assert_eq!(cpu.x[0], 0xDEAD_BEEF);

    cpu.x[1] = 1;
    cpu.x[2] = 2;
    assert_eq!(exec(&mut cpu, PACGA), None);
    let before = cpu.x[0];
    cpu.x[0] = 0x5555;
    assert_eq!(exec(&mut cpu, MSR_APGAKEYHI), None);
    assert_eq!(exec(&mut cpu, PACGA), None);
    assert_ne!(cpu.x[0], before, "PACGA under the new APGAKeyHi_EL1");

    cpu.pstate.el = El::El0;
    cpu.pstate.sp_sel = false;
    for insn in [MRS_APIAKEYLO, MSR_APIAKEYLO, ERETAA] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x} at EL0");
    }
}

/// LDRAA with SP as the base authenticates first and checks SP's
/// alignment after (the instruction page; QEMU makes no SP alignment
/// check): an unsigned SP fails the authentication, a signed misaligned
/// one the alignment check. LDRAB with writeback leaves the authenticated
/// address in the base.
#[test]
fn ldraa_authenticates_before_it_checks_sp_alignment() {
    use super::step::DATA as ONE_WORD_DATA;
    let mut cpu = keyed(&FPACCOMBINE);
    cpu.sys.sctlr_el1 |= SA;
    cpu.sp_el1 = ONE_WORD_DATA + 8;
    assert_eq!(exec(&mut cpu, LDRAA_SP), pac_fail(true, false));
    cpu.sp_el1 = apply(&mut cpu, PACDZA, ONE_WORD_DATA + 8, 0);
    assert_eq!(exec(&mut cpu, LDRAA_SP), Some(Taken { ec: 0x26, iss: 0 }));
    cpu.sp_el1 = apply(&mut cpu, PACDZA, ONE_WORD_DATA, 0);
    assert_eq!(exec(&mut cpu, LDRAA_SP), None);
    assert_eq!(cpu.x[0], 0x5A5A_5A5A_5A5A_5A5A);

    cpu.sys.sctlr_el1 &= !EN_DA;
    cpu.sp_el1 = ONE_WORD_DATA + 8;
    assert_eq!(
        exec(&mut cpu, LDRAA_SP),
        Some(Taken { ec: 0x26, iss: 0 }),
        "a disabled key, then the check"
    );

    cpu.x[1] = apply(&mut cpu, PACDZB, ONE_WORD_DATA + 16, 0);
    assert_eq!(exec(&mut cpu, LDRAB_PRE), None);
    assert_eq!(
        (cpu.x[0], cpu.x[1]),
        (0x5A5A_5A5A_5A5A_5A5A, ONE_WORD_DATA + 8)
    );
}

/// A snapshot carries the keys of a processor implementing FEAT_PAuth.
#[test]
fn a_snapshot_carries_the_keys() {
    let cpu = keyed(&FPACCOMBINE);
    let mut image = Vec::new();
    assert!(cpu.save(&mut image).is_ok());
    let mut restored = processor(Implementation {
        features: Features::of(&FPACCOMBINE),
        ..Implementation::CORTEX_A57
    });
    let mut source: &[u8] = &image;
    assert!(restored.restore(&mut source).is_ok());
    assert!(source.is_empty());
    assert_eq!(restored.pac_keys, cpu.pac_keys);
}

// ----- the tag TBID keeps -----

/// An exception a fetch took, as `ESR_EL1` and `FAR_EL1` report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Abort {
    ec: u64,
    fsc: u64,
    far: u64,
}

/// 1 MiB of RAM from address 0, every access beyond it a bus fault.
struct Ram(Vec<u8>);

/// The level-1 table, mapping the first GiB as one Normal block.
const L1: u64 = 0x1000;
const BRANCH_AT: u64 = 0x1_0000;
const TARGET: u64 = 0x4_0000;
const RAM_VBAR: u64 = 0x2_0000;

impl Ram {
    fn new() -> Self {
        let mut ram = Self(std::vec![0; 0x10_0000]);
        const AF: u64 = 1 << 10;
        const INNER_SHAREABLE: u64 = 3 << 8;
        ram.poke(L1, AF | INNER_SHAREABLE | 0b01);
        ram
    }

    fn poke(&mut self, at: u64, value: u64) {
        let at = at as usize;
        self.0[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn put(&mut self, at: u64, insn: u32) {
        let at = at as usize;
        self.0[at..at + 4].copy_from_slice(&insn.to_le_bytes());
    }

    /// Step the word at the processor's PC: the instruction abort its fetch
    /// took, if it took one.
    fn step(&mut self, cpu: &mut Cpu<NoEl2>) -> Option<Abort> {
        cpu.sys.vbar_el1 = RAM_VBAR;
        cpu.sys.esr_el1 = 0x3F << 26;
        assert_eq!(cpu.step(self, &mut NoInstrumentation), Step::Retired);
        let esr = cpu.sys.esr_el1;
        (esr >> 26 != 0x3F).then_some(Abort {
            ec: esr >> 26,
            fsc: esr & 0x3F,
            far: cpu.sys.far_el1,
        })
    }
}

impl PhysBus for Ram {
    fn read(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        _clock: VmClock,
    ) -> Result<u64, BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get(a..a + n).ok_or(BusFault::Unmapped)?;
        Ok(bytes.iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b)))
    }

    fn write(
        &mut self,
        _addr: PhysAddr,
        _width: AccessWidth,
        _value: u64,
        _clock: VmClock,
    ) -> Result<(), BusFault> {
        Err(BusFault::Unmapped)
    }
}

/// With TCR_EL1.TBID0 set, top-byte-ignore covers data addresses of the
/// lower half and not instruction addresses (`EffectiveTBI`): a branch
/// keeps its target's tag (`AArch64.BranchAddr`) and the fetch from the
/// tagged address faults, a translation fault at level 0 with the MMU
/// on, though a load from the same address filled the TLB, and an
/// address size fault with it off. Without TBID the branch drops the tag.
#[test]
fn a_tag_tbid_keeps_faults_the_fetch() {
    let tagged = 0x5A00_0000_0000_0000 | TARGET;
    for mmu in [false, true] {
        let mut ram = Ram::new();
        ram.put(BRANCH_AT, BR_X1);
        ram.put(BRANCH_AT + 4, LDR);
        ram.put(TARGET, NOP);
        for tbid in [false, true] {
            let mut cpu = keyed(&FPACCOMBINE);
            cpu.sys.tcr_el1 = 25 | EPD1 | TBI0 | if tbid { TBID0 } else { 0 };
            if mmu {
                cpu.sys.mair_el1 = 0xFF;
                cpu.sys.ttbr0_el1 = L1;
                cpu.sys.sctlr_el1 = id::SCTLR_EL1_RESET | 1;
            }
            cpu.x[1] = tagged;
            cpu.pc = BRANCH_AT + 4;
            assert_eq!(ram.step(&mut cpu), None, "the load from the tagged address");
            cpu.pc = BRANCH_AT;
            assert_eq!(ram.step(&mut cpu), None, "the branch");
            let fetch = ram.step(&mut cpu);
            if tbid {
                let fsc = if mmu { 0b00_0100 } else { 0 };
                assert_eq!(
                    fetch,
                    Some(Abort {
                        ec: 0x21,
                        fsc,
                        far: tagged
                    }),
                    "MMU on: {mmu}"
                );
            } else {
                assert_eq!(fetch, None, "MMU on: {mmu}");
                assert_eq!(cpu.pc, TARGET + 4);
            }
        }
    }
}
