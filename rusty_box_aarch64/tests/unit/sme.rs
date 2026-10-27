//! FEAT_SME's state, registers and traps, on a processor that implements it
//! ([`Implementation::EVERY_FEATURE`]) and on one that does not.
//!
//! Each property is the guest-visible one (R9): what a register reads, the
//! exception a word takes (`ESR_EL1`), and what an instruction leaves in
//! the registers. The rules are the Arm ARM's shared pseudocode
//! (`CheckSMEAccess`, `CheckSMEEnabled`, `SetPSTATE_SM`, `SetPSTATE_ZA`,
//! `ResetSVEState`, `AArch64_CheckFPAdvSIMDEnabled`, `MaybeZeroSVEUppers`)
//! and the instruction pages of RDSVL, ADDSVL, ADDSPL and MSR (immediate).

use std::vec::Vec;

use super::super::features::{Implementation, VecLen};
use super::super::step::{exec, processor, Taken, CPACR_ALL, DATA, FP_TRAP, ILLEGAL_WHEN_STREAMING, SME_DISABLED, UNDEFINED};
use super::super::{Cpu, El, NoEl2, Pstate};
use super::Svcr;

const SMSTART: u32 = 0xD503_477F;
const SMSTOP: u32 = 0xD503_467F;
const SMSTART_SM: u32 = 0xD503_437F;
const SMSTOP_SM: u32 = 0xD503_427F;
const SMSTART_ZA: u32 = 0xD503_457F;
const SMSTOP_ZA: u32 = 0xD503_447F;
/// `mrs x0, svcr`.
const MRS_SVCR: u32 = 0xD53B_4240;
/// `msr svcr, x0`.
const MSR_SVCR: u32 = 0xD51B_4240;
/// `mrs x0, smcr_el1`.
const MRS_SMCR: u32 = 0xD538_12C0;
/// `msr smcr_el1, x0`.
const MSR_SMCR: u32 = 0xD518_12C0;
/// `mrs x0, tpidr2_el0`.
const MRS_TPIDR2: u32 = 0xD53B_D0A0;
/// `svc #0`.
const SVC: u32 = 0xD400_0001;

/// `rdsvl x0, #imm`.
const fn rdsvl(imm: i32) -> u32 {
    0x04BF_5800 | (((imm as u32) & 0x3F) << 5)
}

/// `addsvl xd|sp, xn|sp, #imm` (`addspl` with `predicate`).
const fn addsvl(rd: u32, rn: u32, imm: i32, predicate: bool) -> u32 {
    0x0420_5800 | ((predicate as u32) << 22) | (rn << 16) | (((imm as u32) & 0x3F) << 5) | rd
}

fn sme() -> Cpu<NoEl2> {
    processor(Implementation::EVERY_FEATURE)
}

fn at_el0(cpu: &mut Cpu<NoEl2>) {
    cpu.pstate = Pstate {
        el: El::El0,
        sp_sel: false,
        ..Pstate::RESET
    };
}

/// The bytes of Z<n> above V<n> up to `MaxImplementedSVL`, the most any
/// write reaches, as the register file holds them.
fn z_upper(cpu: &Cpu<NoEl2>, n: usize) -> Vec<u8> {
    let len = cpu.imp.max_svl.bytes() - 16;
    if cpu.scalable.z_upper_live & (1 << n) == 0 {
        std::vec![0; len]
    } else {
        cpu.scalable.z_upper[n][..len].to_vec()
    }
}

/// Fill Z<n> above V<n> with `byte`, up to `MaxImplementedSVL`.
fn set_z_upper(cpu: &mut Cpu<NoEl2>, n: usize, byte: u8) {
    let len = cpu.imp.max_svl.bytes() - 16;
    cpu.scalable.z_upper[n][..len].fill(byte);
    cpu.scalable.z_upper_live |= 1 << n;
}

#[test]
fn without_sme_its_instructions_and_registers_are_undefined() {
    let mut cpu = processor(Implementation::CORTEX_A57);
    for insn in [SMSTART, SMSTOP, SMSTART_SM, SMSTART_ZA, MRS_SVCR, MSR_SVCR, MRS_SMCR, MSR_SMCR, MRS_TPIDR2, rdsvl(1), addsvl(0, 1, 1, false)] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}

/// MSR SVCR*, #imm with `CRm<3:1>` = 000, 100, 101, 110, 111 names no
/// field: UNDEFINED with FEAT_SME too (Arm ARM MSR (immediate), decode).
#[test]
fn an_svcr_immediate_with_no_field_is_undefined() {
    let mut cpu = sme();
    for crm in [0b0000, 0b0001, 0b1000, 0b1011, 0b1101, 0b1111] {
        let insn = 0xD503_407F | (crm << 8);
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "CRm {crm:#06b}");
    }
    assert_eq!(cpu.svcr, Svcr::RESET);
}

/// `CheckSMEAccess`: `SMEN` = `x0` traps at EL0 and EL1, `01` at EL0 only.
#[test]
fn smstart_and_smstop_trap_until_cpacr_smen_enables_them() {
    let mut cpu = sme();
    for (smen, el1, el0) in [(0b00, false, false), (0b10, false, false), (0b01, true, false), (0b11, true, true)] {
        cpu.sys.cpacr_el1 = (3 << 20) | (smen << 24);
        for (el, enabled) in [(El::El1, el1), (El::El0, el0)] {
            cpu.svcr = Svcr::RESET;
            if el == El::El0 {
                at_el0(&mut cpu);
            } else {
                cpu.pstate = Pstate::RESET;
            }
            let want = if enabled { None } else { Some(SME_DISABLED) };
            assert_eq!(exec(&mut cpu, SMSTART), want, "SMEN {smen:#04b} at {el:?}");
            assert_eq!(cpu.svcr == (Svcr { sm: true, za: true }), enabled, "SMEN {smen:#04b} at {el:?}");
        }
    }
}

/// `SetPSTATE_SM`: a change of `PSTATE.SM` is `ResetSVEState`, which zeroes
/// every vector, predicate and FFR bit and sets `FPSR` to 0x0800_009F; an
/// SMSTART that finds the processor streaming already changes nothing.
#[test]
fn entering_or_leaving_streaming_mode_resets_the_sve_state() {
    let mut cpu = sme();
    let fill = |cpu: &mut Cpu<NoEl2>| {
        for n in 0..32 {
            cpu.v[n] = 0x1111_2222_3333_4444_5555_6666_7777_8888 + n as u128;
            set_z_upper(cpu, n, 0xEE);
        }
        cpu.scalable.p[3][0] = 0xFF;
        cpu.scalable.ffr[1] = 0xFF;
        cpu.fpsr = 0;
    };
    fill(&mut cpu);
    assert_eq!(exec(&mut cpu, SMSTART_SM), None);
    assert_eq!(cpu.svcr, Svcr { sm: true, za: false });
    assert!(cpu.v.iter().all(|&v| v == 0), "V cleared");
    assert!((0..32).all(|n| z_upper(&cpu, n).iter().all(|&b| b == 0)), "Z cleared");
    assert!(cpu.scalable.p.iter().flatten().all(|&b| b == 0) && cpu.scalable.ffr.iter().all(|&b| b == 0));
    assert_eq!(cpu.fpsr, 0x0800_009F);

    fill(&mut cpu);
    assert_eq!(exec(&mut cpu, SMSTART_SM), None);
    assert_eq!(cpu.v[5], 0x1111_2222_3333_4444_5555_6666_7777_888D, "no change of SM, no reset");
    assert_eq!(cpu.fpsr, 0);

    assert_eq!(exec(&mut cpu, SMSTOP_SM), None);
    assert_eq!(cpu.svcr, Svcr::RESET);
    assert_eq!(cpu.v[5], 0);
    assert_eq!(cpu.fpsr, 0x0800_009F);
}

/// `SetPSTATE_ZA`: enabling ZA zeroes it; the vectors are left alone.
#[test]
fn enabling_za_zeroes_it_and_leaves_the_vectors() {
    let mut cpu = sme();
    cpu.scalable.za[0][0] = 0xAB;
    cpu.scalable.za[63][63] = 0xCD;
    cpu.v[1] = 7;
    assert_eq!(exec(&mut cpu, SMSTART_ZA), None);
    assert_eq!(cpu.svcr, Svcr { sm: false, za: true });
    assert_eq!((cpu.scalable.za[0][0], cpu.scalable.za[63][63]), (0, 0));
    assert_eq!(cpu.v[1], 7, "ZA's enable is not streaming mode's");
    assert_eq!(exec(&mut cpu, SMSTOP_ZA), None);
    assert_eq!(cpu.svcr, Svcr::RESET);
    // SMSTART and SMSTOP set and clear both.
    assert_eq!(exec(&mut cpu, SMSTART), None);
    assert_eq!(cpu.svcr, Svcr { sm: true, za: true });
    assert_eq!(exec(&mut cpu, SMSTOP), None);
    assert_eq!(cpu.svcr, Svcr::RESET);
}

/// `SVCR` reads `PSTATE.SM` and `PSTATE.ZA`, and a write is
/// `SetPSTATE_SM(value<0>)` then `SetPSTATE_ZA(value<1>)`. At EL0 it needs
/// `SMEN` = 11; `SMCR_EL1` is EL1's, and keeps only `LEN`.
#[test]
fn svcr_and_smcr_el1_are_reached_through_cpacr_smen() {
    let mut cpu = sme();
    cpu.x[0] = 0b11;
    cpu.v[2] = 9;
    assert_eq!(exec(&mut cpu, MSR_SVCR), None);
    assert_eq!(cpu.svcr, Svcr { sm: true, za: true });
    assert_eq!(cpu.v[2], 0, "SetPSTATE_SM reset the SVE state");
    cpu.x[0] = 0;
    assert_eq!(exec(&mut cpu, MRS_SVCR), None);
    assert_eq!(cpu.x[0], 0b11);

    cpu.sys.cpacr_el1 = (3 << 20) | (0b01 << 24);
    at_el0(&mut cpu);
    assert_eq!(exec(&mut cpu, MRS_SVCR), Some(SME_DISABLED), "SMEN 01 at EL0");
    assert_eq!(exec(&mut cpu, MRS_SMCR), Some(UNDEFINED), "SMCR_EL1 at EL0");
    cpu.pstate = Pstate::RESET;
    assert_eq!(exec(&mut cpu, MRS_SVCR), None, "SMEN 01 at EL1");

    cpu.x[0] = u64::MAX;
    assert_eq!(exec(&mut cpu, MSR_SMCR), None);
    assert_eq!(cpu.sys.smcr_el1, 0xF, "LEN only: EZT0 and FA64 are RES0 here, bits 8:4 RAZ/WI");
    cpu.sys.cpacr_el1 = 3 << 20;
    assert_eq!(exec(&mut cpu, MRS_SMCR), Some(SME_DISABLED), "SMEN x0 at EL1");
    assert_eq!(exec(&mut cpu, MSR_SVCR), Some(SME_DISABLED));
}

/// `TPIDR2_EL0` is EL0's register, which `SCTLR_EL1.EnTP2` lets EL0 reach
/// (EC 0x18 otherwise).
#[test]
fn tpidr2_el0_needs_sctlr_entp2_at_el0() {
    let mut cpu = sme();
    cpu.sys.tpidr2_el0 = 0x1234;
    at_el0(&mut cpu);
    // ISS: op0 3, op2 5, op1 3, CRn 13, Rt 0, CRm 0, a read.
    assert_eq!(exec(&mut cpu, MRS_TPIDR2), Some(Taken { ec: 0x18, iss: 0x003A_F401 }));
    cpu.sys.sctlr_el1 |= 1 << 60;
    assert_eq!(exec(&mut cpu, MRS_TPIDR2), None);
    assert_eq!(cpu.x[0], 0x1234);
}

/// RDSVL, ADDSVL and ADDSPL count in the Streaming SVE vector length,
/// `SMCR_EL1.LEN` rounded down to a supported power of two, outside
/// Streaming SVE mode as well as in it.
#[test]
fn rdsvl_addsvl_and_addspl_count_in_the_streaming_vector_length() {
    let mut cpu = sme();
    for (len, bytes) in [(0, 16u64), (1, 32), (2, 32), (3, 64), (15, 64)] {
        cpu.sys.smcr_el1 = len;
        assert_eq!(exec(&mut cpu, rdsvl(1)), None);
        assert_eq!(cpu.x[0], bytes, "LEN {len}");
        assert_eq!(exec(&mut cpu, rdsvl(-2)), None);
        assert_eq!(cpu.x[0], (-2i64 * bytes as i64) as u64, "LEN {len}");
    }
    cpu.sys.smcr_el1 = 3;
    cpu.sp_el1 = 0x8000;
    assert_eq!(exec(&mut cpu, addsvl(31, 31, -1, false)), None);
    assert_eq!(cpu.sp_el1, 0x8000 - 64, "addsvl sp, sp, #-1");
    cpu.x[2] = 0x100;
    assert_eq!(exec(&mut cpu, addsvl(1, 2, 3, true)), None);
    assert_eq!(cpu.x[1], 0x100 + 3 * 8, "addspl x1, x2, #3");
    assert_eq!(exec(&mut cpu, rdsvl(31) | 31), None, "rdsvl xzr is a NOP");
}

/// `CheckSMEEnabled`: SME's enable is checked before SIMD&FP's.
#[test]
fn rdsvl_checks_sme_then_simd_and_fp() {
    let mut cpu = sme();
    cpu.sys.cpacr_el1 = 0;
    assert_eq!(exec(&mut cpu, rdsvl(1)), Some(SME_DISABLED));
    cpu.sys.cpacr_el1 = 3 << 24;
    assert_eq!(exec(&mut cpu, rdsvl(1)), Some(FP_TRAP));
}

/// `AArch64_CheckFPAdvSIMDEnabled`: in Streaming SVE mode an Advanced SIMD
/// instruction takes the SME trap with SMTC 1, after decode and after the
/// SIMD&FP enable. The scalar floating-point instructions, the SIMD&FP
/// register loads and stores, and the forms whose pages call
/// `AArch64_CheckFPEnabled` instead execute.
#[test]
fn advanced_simd_is_illegal_in_streaming_mode() {
    let illegal = [
        (0x4EA2_8420, "add v0.4s, v1.4s, v2.4s"),
        (0x0E0C_3C20, "umov w0, v1.s[1]"),
        (0x4E22_DC20, "fmulx v0.4s, v1.4s, v2.4s"),
        (0x4EA1_D820, "frecpe v0.4s, v1.4s"),
        (0x5E21_D820, "scvtf s0, s1"),
        (0x5E08_0420, "mov d0, v1.d[0]"),
        (0x4C40_7020, "ld1 {v0.16b}, [x1]"),
        (0x4D40_0020, "ld1 {v0.b}[8], [x1]"),
        (0x4E28_4820, "aese v0.16b, v1.16b"),
        (0x5E28_0820, "sha1h s0, s1"),
    ];
    let legal = [
        (0x1E22_2820, "fadd s0, s1, s2"),
        (0x9E67_0020, "fmov d0, x1"),
        (0x9EAE_0020, "fmov x0, v1.d[1]"),
        (0x0E04_3C20, "umov w0, v1.s[0]"),
        (0x0E01_2C20, "smov w0, v1.b[0]"),
        (0x5E22_DC20, "fmulx s0, s1, s2"),
        (0x5E22_FC20, "frecps s0, s1, s2"),
        (0x5EA1_D820, "frecpe s0, s1"),
        (0x7EA1_D820, "frsqrte s0, s1"),
        (0x5EA1_F820, "frecpx s0, s1"),
        (0x3DC0_0020, "ldr q0, [x1]"),
        (0x6D40_0420, "ldp d0, d1, [x1]"),
    ];
    let mut cpu = sme();
    cpu.x[1] = DATA;
    for (insn, text) in illegal.iter().chain(legal.iter()) {
        assert_eq!(exec(&mut cpu, *insn), None, "{text} outside streaming mode");
    }
    assert_eq!(exec(&mut cpu, SMSTART_SM), None);
    for (insn, text) in illegal {
        assert_eq!(exec(&mut cpu, insn), Some(ILLEGAL_WHEN_STREAMING), "{text}");
    }
    for (insn, text) in legal {
        assert_eq!(exec(&mut cpu, insn), None, "{text}");
    }
    // A reserved encoding is UNDEFINED before any enable check.
    assert_eq!(exec(&mut cpu, 0x0E08_0420), Some(UNDEFINED), "dup v0.1d, v1.d[0]");
    // The SIMD&FP enable comes before the streaming check.
    cpu.sys.cpacr_el1 = 3 << 24;
    assert_eq!(exec(&mut cpu, 0x4EA2_8420), Some(FP_TRAP), "add v0.4s with FPEN 00");
}

/// `V{128}(n) = value` zeroes Z<n> above bit 127, and leaves the other Z
/// registers alone.
#[test]
fn a_simd_and_fp_write_zeroes_the_rest_of_the_z_register() {
    let mut cpu = sme();
    assert_eq!(exec(&mut cpu, SMSTART_SM), None);
    set_z_upper(&mut cpu, 0, 0xAA);
    set_z_upper(&mut cpu, 1, 0xBB);
    cpu.x[1] = 0x4000_0000;
    assert_eq!(exec(&mut cpu, 0x1E27_0020), None, "fmov s0, w1");
    assert_eq!(cpu.v[0], 0x4000_0000);
    assert!(z_upper(&cpu, 0).iter().all(|&b| b == 0));
    assert!(z_upper(&cpu, 1).iter().all(|&b| b == 0xBB));
}

/// `MaybeZeroSVEUppers`: an exception from EL0 to EL1 in Streaming SVE
/// mode zeroes every Z, P and FFR bit above the streaming vector length,
/// and with ZA enabled every byte of ZA outside the SVL square; outside
/// Streaming SVE mode, or from EL1, nothing changes.
#[test]
fn an_exception_from_el0_zeroes_what_lies_beyond_the_vector_length() {
    let svl = VecLen::from_len_field(1, VecLen::MAX).bytes(); // 32 bytes
    let prepare = |cpu: &mut Cpu<NoEl2>| {
        set_z_upper(cpu, 7, 0xAA);
        cpu.scalable.p[2].fill(0xFF);
        cpu.scalable.ffr.fill(0xFF);
        for row in cpu.scalable.za[..64].iter_mut() {
            row[..64].fill(0xCC);
        }
    };
    let mut cpu = sme();
    assert_eq!(exec(&mut cpu, SMSTART), None);
    cpu.sys.smcr_el1 = 1;
    prepare(&mut cpu);
    at_el0(&mut cpu);
    assert_eq!(exec(&mut cpu, SVC), Some(Taken { ec: 0x15, iss: 0 }));
    let z = z_upper(&cpu, 7);
    assert!(z[..svl - 16].iter().all(|&b| b == 0xAA) && z[svl - 16..].iter().all(|&b| b == 0));
    let pl = svl / 8;
    assert!(cpu.scalable.p[2][..pl].iter().all(|&b| b == 0xFF) && cpu.scalable.p[2][pl..].iter().all(|&b| b == 0));
    assert!(cpu.scalable.ffr[..pl].iter().all(|&b| b == 0xFF) && cpu.scalable.ffr[pl..].iter().all(|&b| b == 0));
    for (n, row) in cpu.scalable.za[..64].iter().enumerate() {
        let kept = if n < svl { svl } else { 0 };
        assert!(row[..kept].iter().all(|&b| b == 0xCC) && row[kept..].iter().all(|&b| b == 0), "ZA row {n}");
    }
    assert_eq!(cpu.svcr, Svcr { sm: true, za: true }, "SM and ZA survive the exception");

    // From EL1, and outside streaming mode, the uppers stay.
    prepare(&mut cpu);
    cpu.pstate = Pstate::RESET;
    assert_eq!(exec(&mut cpu, SVC), Some(Taken { ec: 0x15, iss: 0 }));
    assert!(z_upper(&cpu, 7).iter().all(|&b| b == 0xAA), "from EL1");
    let mut cpu = sme();
    assert_eq!(exec(&mut cpu, SMSTART_ZA), None);
    prepare(&mut cpu);
    at_el0(&mut cpu);
    assert_eq!(exec(&mut cpu, SVC), Some(Taken { ec: 0x15, iss: 0 }));
    assert!(z_upper(&cpu, 7).iter().all(|&b| b == 0xAA), "not streaming");
    assert_eq!(cpu.scalable.za[63][63], 0xCC, "not streaming");
}

/// A snapshot carries the SME state, and a processor implementing other
/// features refuses it.
#[test]
fn a_snapshot_carries_the_sme_state() {
    let mut cpu = sme();
    assert_eq!(exec(&mut cpu, SMSTART), None);
    cpu.sys.smcr_el1 = 2;
    cpu.sys.tpidr2_el0 = 0x77;
    set_z_upper(&mut cpu, 31, 0x31);
    cpu.scalable.p[15][3] = 0x0F;
    cpu.scalable.ffr[0] = 1;
    cpu.scalable.za[5][9] = 0x59;
    let mut image = Vec::new();
    cpu.save(&mut image).expect("save");

    let mut restored = sme();
    let mut source: &[u8] = &image;
    restored.restore(&mut source).expect("restore");
    assert!(source.is_empty());
    assert_eq!(restored.svcr, Svcr { sm: true, za: true });
    assert_eq!((restored.sys.smcr_el1, restored.sys.tpidr2_el0), (2, 0x77));
    assert_eq!(z_upper(&restored, 31), z_upper(&cpu, 31));
    assert_eq!(restored.scalable.p, cpu.scalable.p);
    assert_eq!(restored.scalable.ffr, cpu.scalable.ffr);
    assert_eq!(restored.scalable.za[5][9], 0x59);

    let mut a57 = processor(Implementation::CORTEX_A57);
    assert!(a57.restore(&mut image.as_slice()).is_err(), "an SME image on a Cortex-A57");
}

/// An image whose SME registers hold bits no `MSR` can leave there is
/// refused: `SMCR_EL1` outside `LEN`, and any SME register on a processor
/// without FEAT_SME.
#[test]
fn a_snapshot_with_sme_bits_no_msr_can_set_is_refused() {
    let refused = |cpu: &Cpu<NoEl2>, imp: Implementation| {
        let mut image = Vec::new();
        cpu.save(&mut image).expect("save");
        processor(imp).restore(&mut image.as_slice()).is_err()
    };
    let mut cpu = sme();
    assert!(!refused(&cpu, Implementation::EVERY_FEATURE));
    cpu.sys.smcr_el1 = 0x10;
    assert!(refused(&cpu, Implementation::EVERY_FEATURE), "SMCR_EL1 bit 4");

    let a57 = processor(Implementation::CORTEX_A57);
    assert!(!refused(&a57, Implementation::CORTEX_A57));
    let mut a57 = processor(Implementation::CORTEX_A57);
    a57.sys.tpidr2_el0 = 1;
    assert!(refused(&a57, Implementation::CORTEX_A57), "TPIDR2_EL0 without FEAT_SME");
    let mut a57 = processor(Implementation::CORTEX_A57);
    a57.sys.smcr_el1 = 1;
    assert!(refused(&a57, Implementation::CORTEX_A57), "SMCR_EL1 without FEAT_SME");
    let mut a57 = processor(Implementation::CORTEX_A57);
    a57.sys.cpacr_el1 = CPACR_ALL;
    assert!(refused(&a57, Implementation::CORTEX_A57), "CPACR_EL1.SMEN without FEAT_SME");
}
