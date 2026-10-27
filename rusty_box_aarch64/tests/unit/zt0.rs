//! FEAT_SME2's ZT0 and the instructions on it, on a processor that
//! implements FEAT_SME2 ([`Implementation::EVERY_FEATURE`]) and on one with
//! FEAT_SME alone.
//!
//! Each property is the guest-visible one (R9), against its Arm pseudocode:
//! the enable checks and their order (`CheckSMEEnabled`, then
//! `CheckSMEZT0Enabled`: ZA storage, then `SMCR_EL1.EZT0`), in or out of
//! Streaming SVE mode; ZERO, LDR and STR of ZT0's 64 bytes, with SP as the
//! base too; LDR's and STR's alignment to 16 bytes and a fault part-way;
//! MOVT UNDEFINED outside Debug state; ZT0 zeroed by enabling ZA storage
//! (`ResetSMEState`); `SMCR_EL1.EZT0` RES0 without FEAT_SME2; and ZT0 in a
//! snapshot. LUTI2 and LUTI4: each index of the segment the immediate
//! selects, modulo the segments, for one, two and four registers; the
//! indexes read before a register of the group is written; their checks
//! (`CheckStreamingSVEEnabled` first) and their UNDEFINED sizes.

use std::vec::Vec;

use super::super::features::{Feature, Features, Implementation, VecLen};
use super::super::sctlr;
use super::super::step::{exec, exec_in, processor, Taken, DATA, FP_TRAP, SME_DISABLED, UNDEFINED};
use super::super::sve::Zv;
use super::super::Cpu;
use super::super::sme::ZT0_BYTES;

/// The SME trap with SMTC 2: outside Streaming SVE mode.
const NOT_STREAMING: Taken = Taken { ec: 0x1D, iss: 2 };
/// The SME trap with SMTC 3: ZA storage disabled.
const INACTIVE_ZA: Taken = Taken { ec: 0x1D, iss: 3 };
/// The SME trap with SMTC 4: ZT0 disabled by `SMCR_EL1.EZT0`.
const INACCESSIBLE_ZT0: Taken = Taken { ec: 0x1D, iss: 4 };
/// `SMCR_EL1.EZT0`.
const EZT0: u64 = 1 << 30;

/// `zero {zt0}`.
const ZERO_ZT0: u32 = 0xC048_0001;
/// `ldr zt0, [x0]`.
const LDR_ZT0: u32 = 0xE11F_8000;
/// `str zt0, [x1]`.
const STR_ZT0: u32 = 0xE13F_8020;
/// `ldr zt0, [sp]`.
const LDR_ZT0_SP: u32 = 0xE11F_83E0;
/// `movt x0, zt0[0]` and `movt zt0[0], x0`.
const MOVT_FROM_ZT0: u32 = 0xC04C_03E0;
const MOVT_TO_ZT0: u32 = 0xC04E_03E0;
/// `smstart za`.
const SMSTART_ZA: u32 = 0xD503_457F;
/// `msr smcr_el1, x0`.
const MSR_SMCR: u32 = 0xD518_12C0;
/// `luti2 z0.b, zt0, z1[15]`, `luti2 z0.s, zt0, z1[3]`, `luti4 z0.h, zt0,
/// z1[3]` and `luti4 z0.s, zt0, z31[7]`.
const LUTI2_B: u32 = 0xC0CF_C020;
const LUTI2_S: u32 = 0xC0CC_E020;
const LUTI4_H: u32 = 0xC0CA_D020;
const LUTI4_S: u32 = 0xC0CB_E3E0;
/// `luti2 {z0.b-z1.b}, zt0, z2[7]` and `luti4 {z0.h-z1.h}, zt0, z2[3]`.
const LUTI2_2_B: u32 = 0xC08F_C040;
const LUTI4_2_H: u32 = 0xC08B_D040;
/// `luti2 {z0.h-z3.h}, zt0, z4[3]` and `luti4 {z4.s-z7.s}, zt0, z4[1]`.
const LUTI2_4_H: u32 = 0xC08F_9080;
const LUTI4_4_S: u32 = 0xC08B_A084;
/// `luti2 z0.b, zt0, z1[15]` with `size = 11`, and `luti4 {z0.h-z3.h},
/// zt0, z4[0]` with `size = 00`.
const LUTI2_SIZE_11: u32 = 0xC0CF_F020;
const LUTI4_4_B: u32 = 0xC08A_8080;

/// A data abort from the current level, with its fault status code.
const fn data_abort(dfsc: u64, write: bool) -> Taken {
    Taken {
        ec: 0x25,
        iss: dfsc | ((write as u64) << 6),
    }
}

/// The DFSC of an alignment fault and of a synchronous external abort.
const ALIGNMENT: u64 = 0x21;
const EXTERNAL: u64 = 0x10;

/// `size` bytes counting up from `from`.
fn counting(from: u8, size: usize) -> Vec<u8> {
    (0..size).map(|i| from.wrapping_add(i as u8)).collect()
}

/// A processor with FEAT_SME2, ZA storage enabled and ZT0 accessible, not
/// in Streaming SVE mode.
fn with_zt0() -> Cpu {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.sys.smcr_el1 = EZT0;
    cpu.set_pstate_za(true);
    cpu
}

/// A processor with FEAT_SME2 in Streaming SVE mode at 128 bits, ZA
/// storage enabled and ZT0 accessible, ZT0's entry `i` 0x1111_1111 * `i`.
fn with_table() -> Cpu {
    let mut cpu = with_zt0();
    cpu.set_pstate_sm(true);
    let mut table = [0; ZT0_BYTES];
    for (i, entry) in table.chunks_exact_mut(4).enumerate() {
        entry.copy_from_slice(&(0x1111_1111 * i as u32).to_le_bytes());
    }
    cpu.set_zt0(&table);
    cpu
}

/// The low `bytes` bytes of each of [`with_table`]'s entries `indexes`.
fn looked_up(bytes: usize, indexes: &[u32]) -> Vec<u8> {
    indexes.iter().flat_map(|&i| (0x1111_1111 * i).to_le_bytes()[..bytes].to_vec()).collect()
}

/// Z register `n` of the 16 bytes `bytes`.
fn set_z_bytes(cpu: &mut Cpu, n: u32, bytes: [u8; 16]) {
    let svl = cpu.current_svl();
    cpu.set_z(n, svl, &Zv::from_bytes(&bytes));
}

/// The bytes of Z register `n` at the current length.
fn z_bytes(cpu: &Cpu, n: u32) -> Vec<u8> {
    let svl = cpu.current_svl();
    cpu.z(n, svl).as_bytes()[..svl.bytes()].to_vec()
}

/// 16 bytes: `fill`, then `tail` at the end.
fn bytes_ending(fill: u8, tail: &[u8]) -> [u8; 16] {
    let mut bytes = [fill; 16];
    bytes[16 - tail.len()..].copy_from_slice(tail);
    bytes
}

/// A processor with every feature but FEAT_SME2.
fn sme_alone() -> Cpu {
    let features: Vec<Feature> = Feature::ALL.into_iter().filter(|&f| f != Feature::Sme2).collect();
    processor(Implementation {
        features: Features::of(&features),
        max_svl: VecLen::MAX,
        ..Implementation::EVERY_FEATURE
    })
}

/// The instructions on ZT0 take SME's enable and SIMD&FP's, then the trap
/// with ZA storage disabled, then the one with `SMCR_EL1.EZT0` clear; they
/// execute outside Streaming SVE mode.
#[test]
fn the_zt0_instructions_check_sme_then_za_then_ezt0() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.sys.cpacr_el1 = 0;
    assert_eq!(exec(&mut cpu, ZERO_ZT0), Some(SME_DISABLED));
    cpu.sys.cpacr_el1 = 3 << 24;
    assert_eq!(exec(&mut cpu, ZERO_ZT0), Some(FP_TRAP));
    cpu.sys.cpacr_el1 = (3 << 20) | (3 << 24);
    assert_eq!(exec(&mut cpu, ZERO_ZT0), Some(INACTIVE_ZA));
    cpu.set_pstate_za(true);
    assert_eq!(exec(&mut cpu, ZERO_ZT0), Some(INACCESSIBLE_ZT0));
    assert_eq!(exec(&mut cpu, LDR_ZT0), Some(INACCESSIBLE_ZT0));
    assert_eq!(exec(&mut cpu, STR_ZT0), Some(INACCESSIBLE_ZT0));
    cpu.sys.smcr_el1 = EZT0;
    cpu.set_zt0(&[0xFF; ZT0_BYTES]);
    assert_eq!(exec(&mut cpu, ZERO_ZT0), None);
    assert_eq!(cpu.zt0(), [0; ZT0_BYTES]);
    cpu.set_pstate_sm(true);
    cpu.set_zt0(&[0xFF; ZT0_BYTES]);
    assert_eq!(exec(&mut cpu, ZERO_ZT0), None, "in Streaming SVE mode too");
    assert_eq!(cpu.zt0(), [0; ZT0_BYTES]);
}

/// LDR and STR move ZT0's 64 bytes, byte 0 at the address, from `X[n]` or
/// SP.
#[test]
fn ldr_and_str_move_zt0s_64_bytes() {
    let mut cpu = with_zt0();
    let mut ram = counting(0, 160);
    cpu.x[0] = DATA + 16;
    assert_eq!(exec_in(&mut cpu, &mut ram, LDR_ZT0), None);
    assert_eq!(cpu.zt0()[..], counting(16, 64)[..]);
    cpu.x[1] = DATA + 96;
    assert_eq!(exec_in(&mut cpu, &mut ram, STR_ZT0), None);
    assert_eq!(ram[96..160], counting(16, 64)[..]);
    assert_eq!(ram[..96], counting(0, 96)[..]);
    cpu.sp_el1 = DATA + 32;
    assert_eq!(exec_in(&mut cpu, &mut ram, LDR_ZT0_SP), None);
    assert_eq!(cpu.zt0()[..], counting(32, 64)[..]);
}

/// From an address not aligned to 16 bytes, LDR and STR fault where
/// alignment is enforced, before any access; a fault part-way loads
/// nothing and keeps the bytes stored before it.
#[test]
fn ldr_and_str_of_zt0_align_to_16_bytes_and_fault_part_way() {
    let mut cpu = with_zt0();
    let mut ram = counting(0, 96);
    cpu.sys.sctlr_el1 |= sctlr::A;
    cpu.x[0] = DATA + 8;
    assert_eq!(exec_in(&mut cpu, &mut ram, LDR_ZT0), Some(data_abort(ALIGNMENT, false)));
    assert_eq!(cpu.sys.far_el1, DATA + 8);
    cpu.x[1] = DATA + 8;
    assert_eq!(exec_in(&mut cpu, &mut ram, STR_ZT0), Some(data_abort(ALIGNMENT, true)));
    assert_eq!(ram, counting(0, 96));
    cpu.sys.sctlr_el1 &= !sctlr::A;

    // 64 bytes from 48 before the end of the RAM.
    cpu.set_zt0(&[0xA5; ZT0_BYTES]);
    cpu.x[0] = DATA + 48;
    assert_eq!(exec_in(&mut cpu, &mut ram, LDR_ZT0), Some(data_abort(EXTERNAL, false)));
    assert_eq!(cpu.sys.far_el1, DATA + 96);
    assert_eq!(cpu.zt0(), [0xA5; ZT0_BYTES]);
    cpu.x[1] = DATA + 48;
    assert_eq!(exec_in(&mut cpu, &mut ram, STR_ZT0), Some(data_abort(EXTERNAL, true)));
    assert_eq!(cpu.sys.far_el1, DATA + 96);
    assert_eq!(ram[48..], [0xA5; 48]);
}

/// MOVT between ZT0 and a general register is defined only in Debug state,
/// so it is UNDEFINED here, before any SME check.
#[test]
fn movt_with_a_general_register_is_undefined_outside_debug_state() {
    let mut cpu = with_zt0();
    for insn in [MOVT_FROM_ZT0, MOVT_TO_ZT0] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
    cpu.sys.cpacr_el1 = 0;
    assert_eq!(exec(&mut cpu, MOVT_FROM_ZT0), Some(UNDEFINED));
}

/// Enabling ZA storage zeroes ZT0 with ZA (`ResetSMEState`); setting
/// `PSTATE.SM` does not touch it.
#[test]
fn enabling_za_zeroes_zt0() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.set_zt0(&[0x5A; ZT0_BYTES]);
    cpu.set_pstate_sm(true);
    assert_eq!(cpu.zt0(), [0x5A; ZT0_BYTES]);
    assert_eq!(exec(&mut cpu, SMSTART_ZA), None);
    assert_eq!(cpu.zt0(), [0; ZT0_BYTES]);
}

/// Without FEAT_SME2, `SMCR_EL1.EZT0` is RES0, the instructions on ZT0 are
/// UNDEFINED, and a snapshot of the same processor holding EZT0, which no
/// `MSR` can leave there, is refused.
#[test]
fn without_sme2_ezt0_is_res0_and_the_zt0_instructions_are_undefined() {
    let mut cpu = sme_alone();
    cpu.set_pstate_za(true);
    cpu.x[0] = u64::MAX;
    assert_eq!(exec(&mut cpu, MSR_SMCR), None);
    assert_eq!(cpu.sys.smcr_el1, 0xF);
    for insn in [ZERO_ZT0, LDR_ZT0, STR_ZT0] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
    let refused = |smcr: u64| {
        let mut image = Vec::new();
        let mut cpu = sme_alone();
        cpu.sys.smcr_el1 = smcr;
        cpu.save(&mut image).expect("save");
        sme_alone().restore(&mut image.as_slice()).is_err()
    };
    assert!(!refused(0xF));
    assert!(refused(EZT0 | 0xF), "EZT0 without FEAT_SME2");
}

/// A snapshot carries ZT0 on a processor with FEAT_SME2, and one with
/// `SMCR_EL1.EZT0` set restores there.
#[test]
fn a_snapshot_carries_zt0() {
    let mut cpu = with_zt0();
    cpu.set_zt0(&core::array::from_fn(|i| i as u8 ^ 0x3C));
    let mut image = Vec::new();
    cpu.save(&mut image).expect("save");
    let mut restored = processor(Implementation::EVERY_FEATURE);
    let mut source: &[u8] = &image;
    restored.restore(&mut source).expect("restore");
    assert!(source.is_empty());
    assert_eq!(restored.zt0(), cpu.zt0());
    assert_eq!(restored.sys.smcr_el1, EZT0);
}

/// LUTI2 and LUTI4 to one register: element `e` is the low `esize` bits of
/// ZT0's entry that index `segment * elements + e` of `Zn` selects, 2 or 4
/// bits each, the segment the immediate modulo `esize DIV isize`.
#[test]
fn luti_looks_up_each_index_of_the_selected_segment() {
    let mut cpu = with_table();
    // luti2 z0.b, zt0, z1[15]: 15 MOD 4 is segment 3, bytes 12 to 15.
    set_z_bytes(&mut cpu, 1, bytes_ending(0xFF, &[0xE4; 4]));
    assert_eq!(exec(&mut cpu, LUTI2_B), None);
    assert_eq!(z_bytes(&cpu, 0), looked_up(1, &[0, 1, 2, 3].repeat(4)));
    // luti2 z0.s, zt0, z1[3]: segment 3 of 16, byte 3.
    let mut indexes = [0xFF; 16];
    indexes[3] = 0x1B;
    set_z_bytes(&mut cpu, 1, indexes);
    assert_eq!(exec(&mut cpu, LUTI2_S), None);
    assert_eq!(z_bytes(&cpu, 0), looked_up(4, &[3, 2, 1, 0]));
    // luti4 z0.h, zt0, z1[3]: segment 3 of 4, bytes 12 to 15, the low
    // nibble first.
    set_z_bytes(&mut cpu, 1, bytes_ending(0xFF, &[0x10, 0x32, 0x54, 0xF6]));
    assert_eq!(exec(&mut cpu, LUTI4_H), None);
    assert_eq!(z_bytes(&cpu, 0), looked_up(2, &[0, 1, 2, 3, 4, 5, 6, 15]));
    // luti4 z0.s, zt0, z31[7]: segment 7 of 8, bytes 14 and 15.
    set_z_bytes(&mut cpu, 31, bytes_ending(0xFF, &[0x9A, 0xBC]));
    assert_eq!(exec(&mut cpu, LUTI4_S), None);
    assert_eq!(z_bytes(&cpu, 0), looked_up(4, &[0xA, 0x9, 0xC, 0xB]));
}

/// LUTI2 and LUTI4 to two or four registers: register `r` takes the
/// indexes from `(segment * nreg + r) * elements`, the segment the
/// immediate modulo `esize DIV (isize * nreg)`; `Zn` is read before the
/// first register is written, even when it is that register.
#[test]
fn luti_of_a_group_takes_consecutive_runs_of_indexes() {
    let mut cpu = with_table();
    // luti2 {z0.b-z1.b}, zt0, z2[7]: 7 MOD 2 is segment 1, so Z0 takes
    // bytes 8 to 11 and Z1 bytes 12 to 15.
    set_z_bytes(&mut cpu, 2, bytes_ending(0xFF, &[0xE4, 0xE4, 0xE4, 0xE4, 0x1B, 0x1B, 0x1B, 0x1B]));
    assert_eq!(exec(&mut cpu, LUTI2_2_B), None);
    assert_eq!(z_bytes(&cpu, 0), looked_up(1, &[0, 1, 2, 3].repeat(4)));
    assert_eq!(z_bytes(&cpu, 1), looked_up(1, &[3, 2, 1, 0].repeat(4)));
    // luti4 {z0.h-z1.h}, zt0, z2[3]: 3 MOD 2 is segment 1.
    let nibbles = [0x10, 0x32, 0x54, 0x76, 0x98, 0xBA, 0xDC, 0xFE];
    set_z_bytes(&mut cpu, 2, bytes_ending(0xFF, &nibbles));
    assert_eq!(exec(&mut cpu, LUTI4_2_H), None);
    assert_eq!(z_bytes(&cpu, 0), looked_up(2, &[0, 1, 2, 3, 4, 5, 6, 7]));
    assert_eq!(z_bytes(&cpu, 1), looked_up(2, &[8, 9, 10, 11, 12, 13, 14, 15]));
    // luti2 {z0.h-z3.h}, zt0, z4[3]: 3 MOD 2 is segment 1, two bytes to
    // each register.
    set_z_bytes(&mut cpu, 4, bytes_ending(0xE4, &[0x00, 0x00, 0x55, 0x55, 0xAA, 0xAA, 0xFF, 0xFF]));
    assert_eq!(exec(&mut cpu, LUTI2_4_H), None);
    for r in 0..4 {
        assert_eq!(z_bytes(&cpu, r), looked_up(2, &[r; 8]), "Z{r}");
    }
    // luti4 {z4.s-z7.s}, zt0, z4[1]: segment 1 of 2, every index read
    // from Z4 before Z4 is written.
    set_z_bytes(&mut cpu, 4, bytes_ending(0xFF, &nibbles));
    assert_eq!(exec(&mut cpu, LUTI4_4_S), None);
    for r in 0..4 {
        assert_eq!(z_bytes(&cpu, 4 + r), looked_up(4, &[4 * r, 4 * r + 1, 4 * r + 2, 4 * r + 3]), "Z{}", 4 + r);
    }
}

/// LUTI2 and LUTI4 take `CheckStreamingSVEEnabled`, then
/// `CheckSMEZT0Enabled`: the trap outside Streaming SVE mode whatever ZA
/// storage, then the one with ZA storage disabled, then the one with
/// `SMCR_EL1.EZT0` clear. `size = 11`, and LUTI4 of bytes to four
/// registers, are UNDEFINED before them, and every form without
/// FEAT_SME2.
#[test]
fn luti_checks_streaming_mode_then_za_then_ezt0() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for insn in [LUTI2_SIZE_11, LUTI4_4_B] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
    assert_eq!(exec(&mut cpu, LUTI2_B), Some(NOT_STREAMING));
    cpu.set_pstate_za(true);
    assert_eq!(exec(&mut cpu, LUTI2_B), Some(NOT_STREAMING));
    cpu.set_pstate_za(false);
    cpu.set_pstate_sm(true);
    assert_eq!(exec(&mut cpu, LUTI2_B), Some(INACTIVE_ZA));
    cpu.set_pstate_za(true);
    assert_eq!(exec(&mut cpu, LUTI2_B), Some(INACCESSIBLE_ZT0));
    cpu.sys.smcr_el1 = EZT0;
    assert_eq!(exec(&mut cpu, LUTI2_B), None);
    let mut cpu = sme_alone();
    cpu.set_pstate_sm(true);
    cpu.set_pstate_za(true);
    for insn in [LUTI2_B, LUTI4_H, LUTI2_2_B, LUTI4_4_S] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}
