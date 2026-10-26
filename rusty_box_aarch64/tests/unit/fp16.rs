//! FEAT_FP16 and FEAT_FHM as a guest sees them, on a processor that
//! implements them ([`Implementation::EVERY_FEATURE`]) and on the
//! Cortex-A57, which does not.
//!
//! The values the instructions compute are checked against QEMU
//! (`tests/vectors/qemu_max.txt`); this file holds what those vectors cannot
//! show (R9): the words Arm does not allocate, FPCR.FZ16 as a guest sets it
//! and the exceptions it changes, pinned to the Arm ARM's FPUnpack, FPRound
//! and FPMulAddH, the enable checks after decode, and the forms QEMU 8.2.2
//! gets wrong (docs/qemu-upstream-bugs.md), pinned to their pages.
//!
//! The words are GNU as's (`-march=armv8.4-a+fp16+fp16fml`, objdump -d).

use std::vec::Vec;

use super::super::features::Implementation;
use super::super::step::{exec, processor, FP_TRAP, ILLEGAL_WHEN_STREAMING, UNDEFINED};

/// `fadd h0, h1, h2`, `fmul h0, h1, h2`, `fdiv h0, h1, h2`.
const FADD_H: u32 = 0x1EE2_2820;
const FMUL_H: u32 = 0x1EE2_0820;
const FDIV_H: u32 = 0x1EE2_1820;
/// `fmov h0, #1.0`, `fmov h0, w1`, `fmov w0, h1`.
const FMOV_H_IMM: u32 = 0x1EEE_1000;
const FMOV_H_FROM_W: u32 = 0x1EE7_0020;
const FMOV_W_FROM_H: u32 = 0x1EE6_0020;
/// `fadd v0.8h, v1.8h, v2.8h`.
const FADD_8H: u32 = 0x4E42_1420;
/// `fcvtzs h0, h1`.
const FCVTZS_H: u32 = 0x5EF9_B820;
/// `fmul v0.8h, v1.8h, v2.h[0]`.
const FMUL_8H_ELEM: u32 = 0x4F02_9020;
/// `fmaxnmv h0, v1.8h`, `fmaxnmp h0, v1.2h`.
const FMAXNMV_H: u32 = 0x4E30_C820;
const FMAXNMP_H: u32 = 0x5E30_C820;
/// `fmov v0.8h, #1.0`.
const FMOV_8H_IMM: u32 = 0x4F03_FE00;
/// `scvtf v0.4h, v1.4h, #16`; `scvtf v0.2s, v1.2s, #32`, its single
/// precision form.
const SCVTF_4H_FIXED: u32 = 0x0F10_E420;
const SCVTF_2S_FIXED: u32 = 0x0F20_E420;
/// `fcvtzs h0, h1, #1`.
const FCVTZS_H_FIXED: u32 = 0x5F1F_FC20;
/// `fabs v0.8h, v1.8h`, `fneg v0.8h, v1.8h`.
const FABS_8H: u32 = 0x4EF8_F820;
const FNEG_8H: u32 = 0x6EF8_F820;
/// `fmlal v0.2s, v1.2h, v2.2h`, `fmlal v0.2s, v1.2h, v2.h[0]`.
const FMLAL: u32 = 0x0E22_EC20;
const FMLAL_ELEM: u32 = 0x0F82_0020;
/// The scalar two-register miscellaneous half-precision class's `a = 1`,
/// opcode 01111 words, `U` 0 and 1, with Rn 1 and Rd 0: where the vector
/// class has FABS and FNEG.
const SCALAR_FABS_H: u32 = 0x5EF8_F820;
const SCALAR_FNEG_H: u32 = 0x7EF8_F820;
/// `fcvt s0, h1`, `fcvt h0, s1`: FEAT_FP's conversions, on every processor.
const FCVT_S_FROM_H: u32 = 0x1EE2_4020;
const FCVT_H_FROM_S: u32 = 0x1E23_C020;
/// `msr fpcr, x1`.
const MSR_FPCR: u32 = 0xD51B_4401;

/// The scalar floating-point forms, which `AArch64_CheckFPEnabled` checks.
const SCALAR_FP: [u32; 6] = [FADD_H, FMUL_H, FDIV_H, FMOV_H_IMM, FMOV_H_FROM_W, FMOV_W_FROM_H];
/// The Advanced SIMD forms, which `AArch64_CheckFPAdvSIMDEnabled` checks.
const ADVSIMD: [u32; 13] = [
    FADD_8H,
    FCVTZS_H,
    FMUL_8H_ELEM,
    FMAXNMV_H,
    FMAXNMP_H,
    FMOV_8H_IMM,
    SCVTF_4H_FIXED,
    FCVTZS_H_FIXED,
    FABS_8H,
    FNEG_8H,
    FMLAL,
    FMLAL_ELEM,
    // fmlsl2 v0.4s, v1.4h, v2.h[7]
    0x6FB2_C820,
];

/// FPCR.FZ16 and FPCR.FZ.
const FZ16: u64 = 1 << 19;
const FZ: u64 = 1 << 24;
/// FPSR's cumulative underflow, inexact and input denormal flags.
const UFC: u64 = 1 << 3;
const IXC: u64 = 1 << 4;
const IDC: u64 = 1 << 7;

fn allocated() -> Vec<u32> {
    SCALAR_FP.iter().chain(ADVSIMD.iter()).copied().collect()
}

#[test]
fn without_the_features_their_encodings_are_undefined() {
    let mut cpu = processor(Implementation::CORTEX_A57);
    for insn in allocated() {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
    // FEAT_FP's conversions between half and single precision are not
    // FEAT_FP16's.
    assert_eq!(exec(&mut cpu, FCVT_S_FROM_H), None);
    assert_eq!(exec(&mut cpu, FCVT_H_FROM_S), None);
}

#[test]
fn with_the_features_every_form_executes() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for insn in allocated() {
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
    }
}

/// SCVTF (vector, fixed-point): `if immh == '000x' || (immh == '001x' &&
/// !IsFeatureImplemented(FEAT_FP16)) then EndOfDecode(Decode_UNDEF)`.
#[test]
fn a_half_precision_fixed_point_shift_needs_fp16() {
    let mut cpu = processor(Implementation::CORTEX_A57);
    assert_eq!(exec(&mut cpu, SCVTF_4H_FIXED), Some(UNDEFINED));
    assert_eq!(exec(&mut cpu, SCVTF_2S_FIXED), None);
}

/// The scalar two-register miscellaneous half-precision class has no FABS
/// or FNEG (`a = 1`, opcode 01111), which the vector class has. QEMU 8.2.2
/// executes FNEG and aborts on FABS.
#[test]
fn the_scalar_half_precision_fabs_and_fneg_are_unallocated() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for insn in [SCALAR_FABS_H, SCALAR_FNEG_H] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}

/// FCVTZS (scalar, fixed-point) writes `V[d]` at 128 bits, the 16-bit
/// result zero-extended. QEMU 8.2.2 sign-extends it into 32 bits.
#[test]
fn fcvtzs_to_a_half_precision_fixed_point_zeroes_the_bits_above_it() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    // -1.0, with the bits above the half-precision operand set.
    cpu.v[1] = (u128::MAX << 16) | 0xBC00;
    cpu.v[0] = u128::MAX;
    assert_eq!(exec(&mut cpu, FCVTZS_H_FIXED), None);
    assert_eq!(cpu.v[0], 0xFFFE, "-1.0 × 2^1");
    assert_eq!(exec(&mut cpu, FCVTZS_H), None);
    assert_eq!(cpu.v[0], 0xFFFF, "-1");
    assert_eq!(cpu.fpsr, 0);
}

/// FPCR.FZ16 is FEAT_FP16's, RES0 without it.
#[test]
fn a_guest_sets_fz16_with_fp16_only() {
    for (imp, kept) in [(Implementation::EVERY_FEATURE, FZ16 | FZ), (Implementation::CORTEX_A57, FZ)] {
        let mut cpu = processor(imp);
        cpu.x[1] = FZ16 | FZ;
        assert_eq!(exec(&mut cpu, MSR_FPCR), None);
        assert_eq!(cpu.fpcr, kept);
    }
}

/// `h0 = h1 <op> h2` under `fpcr`: the result register and FPSR.
fn half_op(insn: u32, fpcr: u64, h1: u16, h2: u16) -> (u128, u64) {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.fpcr = fpcr;
    cpu.fpsr = 0;
    cpu.v[1] = u128::from(h1);
    cpu.v[2] = u128::from(h2);
    assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
    (cpu.v[0], cpu.fpsr)
}

/// FPUnpack with `N == 16`: under FPCR.FZ16 a denormal operand is a zero
/// with no exception (half precision has no input denormal exception);
/// FPCR.FZ does not apply to half precision.
#[test]
fn fz16_flushes_a_denormal_operand_without_an_exception() {
    // The smallest denormal, plus zero.
    assert_eq!(half_op(FADD_H, 0, 0x0001, 0), (0x0001, 0));
    assert_eq!(half_op(FADD_H, FZ, 0x0001, 0), (0x0001, 0));
    assert_eq!(half_op(FADD_H, FZ16, 0x0001, 0), (0x0000, 0));
    assert_eq!(half_op(FADD_H, FZ16, 0x8001, 0x8000), (0x8000, 0));
}

/// FPRound with `N == 16`: under FPCR.FZ16 a result below 2^-14 before
/// rounding is a zero of its sign, raising underflow and nothing else, even
/// where rounding it would have been inexact; without FZ16 a tiny result
/// raises underflow only when it is inexact.
#[test]
fn fz16_flushes_a_tiny_result_raising_underflow_alone() {
    // 2^-10 × 2^-10 = 2^-20, an exact denormal.
    assert_eq!(half_op(FMUL_H, 0, 0x1400, 0x1400), (0x0010, 0));
    assert_eq!(half_op(FMUL_H, FZ16, 0x1400, 0x1400), (0x0000, UFC));
    // 2^-14 / 3 = 341.33 × 2^-24, an inexact denormal.
    assert_eq!(half_op(FDIV_H, 0, 0x0400, 0x4200), (0x0155, UFC | IXC));
    assert_eq!(half_op(FDIV_H, FZ16, 0x0400, 0x4200), (0x0000, UFC));
    assert_eq!(half_op(FDIV_H, FZ16, 0x8400, 0x4200), (0x8000, UFC));
    // FPCR.FZ is single and double precision's.
    assert_eq!(half_op(FDIV_H, FZ, 0x0400, 0x4200), (0x0155, UFC | IXC));
}

/// FPMulAddH: the half-precision operands are unpacked under FPCR.FZ16,
/// the single-precision addend under FPCR.FZ (which raises input denormal).
#[test]
fn fmlal_flushes_its_operands_by_fz16_and_its_addend_by_fz() {
    let fmlal = |fpcr: u64, addend: u32, n: u16, m: u16| {
        let mut cpu = processor(Implementation::EVERY_FEATURE);
        cpu.fpcr = fpcr;
        cpu.fpsr = 0;
        cpu.v[0] = u128::from(addend);
        cpu.v[1] = u128::from(n);
        cpu.v[2] = u128::from(m);
        assert_eq!(exec(&mut cpu, FMLAL), None);
        (cpu.v[0], cpu.fpsr)
    };
    // 0 + 2^-24 × 1.0: a half-precision denormal, a single-precision
    // normal.
    assert_eq!(fmlal(0, 0, 0x0001, 0x3C00), (0x3380_0000, 0));
    assert_eq!(fmlal(FZ, 0, 0x0001, 0x3C00), (0x3380_0000, 0));
    assert_eq!(fmlal(FZ16, 0, 0x0001, 0x3C00), (0, 0));
    // A single-precision denormal addend plus 0 × 0.
    assert_eq!(fmlal(FZ16, 1, 0, 0), (1, 0));
    assert_eq!(fmlal(FZ, 1, 0, 0), (0, IDC));
}

/// `AArch64_CheckFPEnabled` and `AArch64_CheckFPAdvSIMDEnabled` run after
/// decode: with SIMD&FP disabled every form takes the access trap; in
/// Streaming SVE mode the scalar floating-point forms execute and the
/// Advanced SIMD ones take the SME trap (SMTC 1).
#[test]
fn the_fp_checks_follow_decode() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.sys.cpacr_el1 = 3 << 24;
    for insn in allocated() {
        assert_eq!(exec(&mut cpu, insn), Some(FP_TRAP), "{insn:#010x}");
    }
    // Unallocated, whatever the enables.
    assert_eq!(exec(&mut cpu, SCALAR_FABS_H), Some(UNDEFINED));

    cpu.sys.cpacr_el1 = super::super::step::CPACR_ALL;
    // smstart sm
    assert_eq!(exec(&mut cpu, 0xD503_437F), None);
    for insn in SCALAR_FP {
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
    }
    for insn in ADVSIMD {
        assert_eq!(exec(&mut cpu, insn), Some(ILLEGAL_WHEN_STREAMING), "{insn:#010x}");
    }
}
