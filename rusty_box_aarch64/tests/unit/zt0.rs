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
//! snapshot.

use std::vec::Vec;

use super::super::features::{Feature, Features, Implementation, VecLen};
use super::super::sctlr;
use super::super::step::{exec, exec_in, processor, Taken, DATA, FP_TRAP, SME_DISABLED, UNDEFINED};
use super::super::Cpu;
use super::super::sme::ZT0_BYTES;

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

/// A processor with every feature but FEAT_SME2.
fn sme_alone() -> Cpu {
    let features: Vec<Feature> = Feature::ALL.into_iter().filter(|&f| f != Feature::Sme2).collect();
    processor(Implementation {
        features: Features::of(&features),
        max_svl: VecLen::MAX,
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
