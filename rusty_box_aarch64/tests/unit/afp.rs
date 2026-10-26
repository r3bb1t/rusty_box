//! FEAT_AFP and FEAT_RPRES as a guest sees them: FPCR.FIZ, FPCR.AH's
//! alternate behaviours and FPCR.NEP's merging, on a processor that
//! implements them ([`Implementation::EVERY_FEATURE`]) and on the
//! Cortex-A57, which does not.
//!
//! The values the instructions compute under random FPCR.FIZ, AH and NEP
//! are checked against QEMU (`tests/vectors/qemu_max.txt`, the `_afp`
//! families); this file pins what random inputs rarely reach, each case to
//! the Arm ARM's pseudocode: FPProcessNaNs, FPProcessNaNs3, FPDefaultNaN,
//! FPUnpackBase, FPRoundBase, FPProcessDenorms, FPMax, FPMaxNum, FPAbs,
//! FPNeg, FPRecipStepFused, FPRecipEstimate, FPConvertBF, BFDotAdd,
//! BFMulAddH and IsMerging.
//!
//! The words are GNU as's (`-march=armv8.7-a+fp16+bf16`, objdump -d).

use super::features::Implementation;
use super::step::{exec, processor};

/// `msr fpcr, x1`.
const MSR_FPCR: u32 = 0xD51B_4401;
/// `fadd s0, s1, s2`, `fmul s0, s1, s2`, `fdiv s0, s1, s2`, `fsqrt s0,
/// s1`.
const FADD_S: u32 = 0x1E22_2820;
const FDIV_S: u32 = 0x1E22_1820;
const FSQRT_S: u32 = 0x1E21_C020;
/// `fmax s0, s1, s2`, `fmin s0, s1, s2`, `fmaxnm s0, s1, s2`.
const FMAX_S: u32 = 0x1E22_4820;
const FMIN_S: u32 = 0x1E22_5820;
const FMAXNM_S: u32 = 0x1E22_6820;
/// `fmadd s0, s1, s2, s3`, `fmadd d0, d1, d2, d3`.
const FMADD_S: u32 = 0x1F02_0C20;
const FMADD_D: u32 = 0x1F42_0C20;
/// `fnmul s0, s1, s2`, `fabs s0, s1`, `fneg s0, s1`, `fneg v0.4s, v1.4s`,
/// `fabd s0, s1, s2`.
const FNMUL_S: u32 = 0x1E22_8820;
const FABS_S: u32 = 0x1E20_C020;
const FNEG_S: u32 = 0x1E21_4020;
const FNEG_4S: u32 = 0x6EA0_F820;
const FABD_S: u32 = 0x7EA2_D420;
/// `fcvt s0, d1`, `fcvt d0, s1`, `fcvtxn s0, d1`.
const FCVT_S_D: u32 = 0x1E62_4020;
const FCVT_D_S: u32 = 0x1E22_C020;
const FCVTXN_S: u32 = 0x7E61_6820;
/// `frecps s0, s1, s2`, `frecpe s0, s1`, `frsqrte s0, s1`.
const FRECPS_S: u32 = 0x5E22_FC20;
const FRECPE_S: u32 = 0x5EA1_D820;
const FRSQRTE_S: u32 = 0x7EA1_D820;
/// `fcmeq s0, s1, s2`, `fcmeq s0, s1, #0.0`, `facge s0, s1, s2`.
const FCMEQ_S: u32 = 0x5E22_E420;
const FCMEQ_ZERO_S: u32 = 0x5EA0_D820;
const FACGE_S: u32 = 0x7E22_EC20;
/// `scvtf s0, w1`, `fmov s0, s1`, `fmov s0, #1.0`.
const SCVTF_S_W: u32 = 0x1E22_0020;
const FMOV_S: u32 = 0x1E20_4020;
const FMOV_S_ONE: u32 = 0x1E2E_1000;
/// `fmla s0, s1, v2.s[1]`, `fmul s0, s1, v2.s[0]`, `faddp s0, v1.2s`.
const FMLA_S_ELEM: u32 = 0x5FA2_1020;
const FMUL_S_ELEM: u32 = 0x5F82_9020;
const FADDP_S: u32 = 0x7E30_D820;
/// `fmax h0, h1, h2`, `fmul h0, h1, h2`.
const FMAX_H: u32 = 0x1EE2_4820;
const FMUL_H: u32 = 0x1EE2_0820;
/// `bfcvt h0, s1`, `bfdot v0.4s, v1.8h, v2.8h`, `bfmlalb v0.4s, v1.8h,
/// v2.8h`.
const BFCVT: u32 = 0x1E63_4020;
const BFDOT: u32 = 0x6E42_FC20;
const BFMLALB: u32 = 0x2EC2_FC20;

/// FPCR.FIZ, AH and NEP (FEAT_AFP), FZ16, RMode toward zero, FZ and DN.
const FIZ: u64 = 1 << 0;
const AH: u64 = 1 << 1;
const NEP: u64 = 1 << 2;
const FZ16: u64 = 1 << 19;
const RZ: u64 = 3 << 22;
const FZ: u64 = 1 << 24;
const DN: u64 = 1 << 25;
/// The FPSR cumulative bits.
const IOC: u64 = 1 << 0;
const DZC: u64 = 1 << 1;
const OFC: u64 = 1 << 2;
const UFC: u64 = 1 << 3;
const IXC: u64 = 1 << 4;
const IDC: u64 = 1 << 7;

/// Single-precision values.
const ONE: u128 = 0x3F80_0000;
const TWO: u128 = 0x4000_0000;
const QNAN: u128 = 0x7FC0_0001;
const SNAN: u128 = 0x7F80_0001;
const INF: u128 = 0x7F80_0000;
const NEG_INF: u128 = 0xFF80_0000;
const NEG_ZERO: u128 = 0x8000_0000;
/// The smallest single-precision denormal, 2^-149.
const DENORMAL: u128 = 1;

/// Step `insn` with V0-V3 holding `v` under `fpcr` on a processor with
/// every feature: V0 after it and FPSR.
fn run(insn: u32, fpcr: u64, v: [u128; 4]) -> (u128, u64) {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.fpcr = fpcr;
    cpu.fpsr = 0;
    cpu.v[..4].copy_from_slice(&v);
    assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
    (cpu.v[0], cpu.fpsr)
}

/// `insn` on V1 and V2 (and V3), V0 zero.
fn op(insn: u32, fpcr: u64, a: u128, b: u128) -> (u128, u64) {
    run(insn, fpcr, [0, a, b, 0])
}

/// FPCR.FIZ, AH and NEP are FEAT_AFP's, RES0 without it.
#[test]
fn a_guest_sets_fiz_ah_and_nep_with_feat_afp_only() {
    for (imp, kept) in [(Implementation::EVERY_FEATURE, FIZ | AH | NEP | FZ), (Implementation::CORTEX_A57, FZ)] {
        let mut cpu = processor(imp);
        cpu.x[1] = FIZ | AH | NEP | FZ;
        assert_eq!(exec(&mut cpu, MSR_FPCR), None);
        assert_eq!(cpu.fpcr, kept, "{imp:?}");
    }
}

/// FPProcessNaNs: two NaN operands give the first signalling one under
/// FPCR.AH = 0, the first one, quietened, under FPCR.AH = 1; FPDefaultNaN
/// is negative under FPCR.AH = 1.
#[test]
fn alternate_handling_chooses_the_first_nan_and_a_negative_default_nan() {
    assert_eq!(op(FADD_S, 0, QNAN, 0x7F80_0002), (0x7FC0_0002, IOC));
    assert_eq!(op(FADD_S, AH, QNAN, 0x7F80_0002), (0x7FC0_0001, IOC));
    assert_eq!(op(FADD_S, DN, QNAN, 0x7F80_0002), (0x7FC0_0000, IOC));
    assert_eq!(op(FADD_S, AH | DN, QNAN, 0x7F80_0002), (0xFFC0_0000, IOC));
    // Infinities of opposite signs added: the default NaN.
    assert_eq!(op(FADD_S, 0, INF, NEG_INF), (0x7FC0_0000, IOC));
    assert_eq!(op(FADD_S, AH, INF, NEG_INF), (0xFFC0_0000, IOC));
}

/// FPProcessNaNs3 for `fmadd s0, s1, s2, s3` (FPMulAdd(Sa, Sn, Sm)): the
/// first signalling NaN in the order Sa, Sn, Sm under FPCR.AH = 0; under
/// FPCR.AH = 1 Sn when it is one of two or more NaNs, else Sm when Sa is
/// a NaN too. An infinity times a zero plus a quiet NaN is the default NaN
/// with Invalid Operation under FPCR.AH = 0, the quiet NaN under FPCR.AH =
/// 1.
#[test]
fn alternate_handling_chooses_fused_nans_by_register() {
    let fmadd = |fpcr, a: u128, n: u128, m: u128| run(FMADD_S, fpcr, [0, n, m, a]);
    let (a, n, m) = (0x7FC0_000A, 0x7FC0_000B, 0x7F80_000C);
    assert_eq!(fmadd(0, a, n, m), (0x7FC0_000C, IOC));
    assert_eq!(fmadd(AH, a, n, m), (0x7FC0_000B, IOC));
    let (a, m) = (0x7F80_000A, 0x7FC0_000C);
    assert_eq!(fmadd(0, a, ONE, m), (0x7FC0_000A, IOC));
    assert_eq!(fmadd(AH, a, ONE, m), (0x7FC0_000C, IOC));
    assert_eq!(fmadd(0, 0x7FC0_000A, INF, 0), (0x7FC0_0000, IOC));
    assert_eq!(fmadd(AH, 0x7FC0_000A, INF, 0), (0x7FC0_000A, 0));
}

/// FPUnpackBase: FPCR.FIZ flushes a denormal operand without Input
/// Denormal; FPCR.FZ does it with one under FPCR.AH = 0 and not at all
/// under FPCR.AH = 1, where a used denormal raises Input Denormal
/// (FPProcessDenorms) and FPCR.FZ flushes the result after rounding,
/// raising Underflow and Inexact.
#[test]
fn fiz_and_fz_flush_as_fpcr_ah_says() {
    let add = |fpcr| op(FADD_S, fpcr, DENORMAL, 0);
    assert_eq!(add(0), (DENORMAL, 0));
    assert_eq!(add(FZ), (0, IDC));
    assert_eq!(add(FIZ), (0, 0));
    assert_eq!(add(FIZ | FZ), (0, IDC));
    assert_eq!(add(AH | FIZ), (0, 0));
    assert_eq!(add(AH), (DENORMAL, IDC));
    assert_eq!(add(AH | FZ), (0, IDC | UFC | IXC));
}

/// FPRoundBase: tininess before rounding under FPCR.AH = 0, after rounding
/// with an unbounded exponent under FPCR.AH = 1, which is where FPCR.FZ
/// flushes. 2^-126 - 2^-156 rounds to 2^-126 either way, so it is tiny
/// only under FPCR.AH = 0; 2^-127 is tiny under both.
#[test]
fn alternate_handling_decides_tininess_after_rounding() {
    let fcvt = |fpcr, d: u128| op(FCVT_S_D, fpcr, d, 0);
    let almost_min_normal = 0x380F_FFFF_FF80_0000;
    assert_eq!(fcvt(0, almost_min_normal), (0x0080_0000, UFC | IXC));
    assert_eq!(fcvt(AH, almost_min_normal), (0x0080_0000, IXC));
    assert_eq!(fcvt(FZ, almost_min_normal), (0, UFC));
    assert_eq!(fcvt(AH | FZ, almost_min_normal), (0x0080_0000, IXC));
    let half_min_normal = 0x3800_0000_0000_0000;
    assert_eq!(fcvt(AH, half_min_normal), (0x0040_0000, 0));
    assert_eq!(fcvt(AH | FZ, half_min_normal), (0, UFC | IXC));
    // FCVTXN rounds to odd, but a flushed zero keeps its lowest bit clear.
    let tiny_inexact = 0x3730_0000_0040_0000;
    assert_eq!(op(FCVTXN_S, 0, tiny_inexact, 0), (0x0000_0201, UFC | IXC));
    assert_eq!(op(FCVTXN_S, FZ, tiny_inexact, 0), (0, UFC));
    assert_eq!(op(FCVTXN_S, AH | FZ, tiny_inexact, 0), (0, UFC | IXC));
}

/// FPProcessDenorms under FPCR.AH = 1 reports a used denormal only where
/// each operation calls it: not when a NaN, a zero divisor, a negative
/// square root or an invalid fused operation decided the result.
#[test]
fn a_used_denormal_is_reported_where_the_pseudocode_says() {
    assert_eq!(op(FDIV_S, AH, DENORMAL, ONE), (DENORMAL, IDC));
    assert_eq!(op(FDIV_S, AH, DENORMAL, 0), (INF, DZC));
    assert_eq!(op(FADD_S, AH, QNAN, DENORMAL), (QNAN, 0));
    assert_eq!(op(FSQRT_S, AH, 0x8000_0001, 0), (0xFFC0_0000, IOC));
    assert_eq!(op(FSQRT_S, AH, DENORMAL, 0), (0x1A35_04F3, IDC | IXC));
    let fmadd = |a: u128, n: u128, m: u128| run(FMADD_S, AH, [0, n, m, a]);
    assert_eq!(fmadd(INF, NEG_INF, DENORMAL), (0xFFC0_0000, IOC));
    assert_eq!(fmadd(0, ONE, DENORMAL), (DENORMAL, IDC));
    assert_eq!(op(FCVT_D_S, AH, DENORMAL, 0), (0x36A0_0000_0000_0000, IDC));
    assert_eq!(op(FCMEQ_S, AH, DENORMAL, DENORMAL), (0xFFFF_FFFF, IDC));
    assert_eq!(op(FCMEQ_S, 0, DENORMAL, DENORMAL), (0xFFFF_FFFF, 0));
}

/// FPMax and FPMin under FPCR.AH = 1 (FMAX, FMIN): a NaN operand, or two
/// zeros of opposite signs, give the second operand as it is — a
/// signalling NaN not quietened — a NaN raising Invalid Operation; the
/// result is not flushed. FPMaxNum calls FPMax with the IEEE rules, so
/// FMAXNM's result is flushed after rounding.
#[test]
fn alternate_fmax_and_fmin_give_the_second_operand() {
    assert_eq!(op(FMAX_S, 0, QNAN, ONE), (QNAN, 0));
    assert_eq!(op(FMAX_S, AH, QNAN, ONE), (ONE, IOC));
    assert_eq!(op(FMAX_S, 0, ONE, SNAN), (0x7FC0_0001, IOC));
    assert_eq!(op(FMAX_S, AH, ONE, SNAN), (SNAN, IOC));
    assert_eq!(op(FMAX_S, 0, 0, NEG_ZERO), (0, 0));
    assert_eq!(op(FMAX_S, AH, 0, NEG_ZERO), (NEG_ZERO, 0));
    assert_eq!(op(FMIN_S, 0, NEG_ZERO, 0), (NEG_ZERO, 0));
    assert_eq!(op(FMIN_S, AH, NEG_ZERO, 0), (0, 0));
    assert_eq!(op(FMAX_S, AH | FZ, DENORMAL, 0xBF80_0000), (DENORMAL, IDC));
    assert_eq!(op(FMAXNM_S, AH | FZ, DENORMAL, 0xBF80_0000), (0, IDC | UFC | IXC));
    // Two NaNs: FPMaxNum replaces the quiet one under FPCR.AH = 0; under
    // FPCR.AH = 1 FPProcessNaNs takes the first.
    assert_eq!(op(FMAXNM_S, 0, QNAN, 0x7F80_0002), (0x7FC0_0002, IOC));
    assert_eq!(op(FMAXNM_S, AH, QNAN, 0x7F80_0002), (QNAN, IOC));
    // Half precision returns the second operand's own bits as well.
    assert_eq!(op(FMAX_H, AH, 0x3C00, 0x7C01), (0x7C01, IOC));
    assert_eq!(op(FMAX_H, 0, 0x3C00, 0x7C01), (0x7E01, IOC));
}

/// FPAbs and FPNeg leave a NaN alone under FPCR.AH = 1: FABS, FNEG, FNMUL's
/// negated product and FABD's absolute difference.
#[test]
fn alternate_abs_and_neg_keep_a_nan_s_sign() {
    assert_eq!(op(FNEG_S, 0, QNAN, 0), (0xFFC0_0001, 0));
    assert_eq!(op(FNEG_S, AH, QNAN, 0), (QNAN, 0));
    assert_eq!(op(FNEG_S, AH, ONE, 0), (0xBF80_0000, 0));
    assert_eq!(op(FABS_S, 0, 0xFFC0_0001, 0), (QNAN, 0));
    assert_eq!(op(FABS_S, AH, 0xFFC0_0001, 0), (0xFFC0_0001, 0));
    let lanes = |l: [u32; 4]| l.iter().rev().fold(0u128, |acc, &x| (acc << 32) | u128::from(x));
    let v = lanes([0x7FC0_0001, 0x3F80_0000, 0xFF80_0001, 0]);
    assert_eq!(op(FNEG_4S, 0, v, 0).0, lanes([0xFFC0_0001, 0xBF80_0000, 0x7F80_0001, 0x8000_0000]));
    assert_eq!(op(FNEG_4S, AH, v, 0).0, lanes([0x7FC0_0001, 0xBF80_0000, 0xFF80_0001, 0x8000_0000]));
    assert_eq!(op(FNMUL_S, 0, SNAN, ONE), (0xFFC0_0001, IOC));
    assert_eq!(op(FNMUL_S, AH, SNAN, ONE), (0x7FC0_0001, IOC));
    assert_eq!(op(FABD_S, 0, 0xFFC0_0001, ONE), (QNAN, 0));
    assert_eq!(op(FABD_S, AH, 0xFFC0_0001, ONE), (0xFFC0_0001, 0));
    // FACGE compares FPAbs of each: unordered either way.
    assert_eq!(op(FACGE_S, AH, 0xFFC0_0001, ONE), (0, IOC));
}

/// FPRecipStepFused and the estimates under FPCR.AH = 1 run with `fpexc`
/// FALSE: no exception, and FPNeg leaves a NaN alone. With FEAT_RPRES the
/// single-precision estimates have 12 bits.
#[test]
fn alternate_steps_and_estimates_raise_nothing() {
    assert_eq!(op(FRECPS_S, 0, SNAN, ONE), (0xFFC0_0001, IOC));
    assert_eq!(op(FRECPS_S, AH, SNAN, ONE), (0x7FC0_0001, 0));
    // 2 + 2^100 × 2^100 overflows.
    let big = 0x7180_0000;
    assert_eq!(op(FRECPS_S, 0, big, 0xF180_0000), (INF, OFC | IXC));
    assert_eq!(op(FRECPS_S, AH, big, 0xF180_0000), (INF, 0));
    assert_eq!(op(FRECPE_S, 0, TWO, 0), (0x3EFF_8000, 0));
    assert_eq!(op(FRECPE_S, AH, TWO, 0), (0x3EFF_F000, 0));
    assert_eq!(op(FRSQRTE_S, AH, 0x4080_0000, 0), (0x3EFF_F000, 0));
    assert_eq!(op(FRECPE_S, AH, 0, 0), (INF, 0));
    assert_eq!(op(FRECPE_S, 0, 0, 0), (INF, DZC));
}

/// FPConvertBF under FPCR.AH = 1 rounds to nearest with no exception;
/// BFDotAdd's default NaN takes FPCR.AH's sign; BFMulAddH under FPCR.AH = 1
/// raises nothing.
#[test]
fn alternate_bfloat16_operations() {
    let rounds_up = 0x3F80_8001;
    assert_eq!(op(BFCVT, RZ, rounds_up, 0), (0x3F80, IXC));
    assert_eq!(op(BFCVT, AH | RZ, rounds_up, 0), (0x3F81, 0));
    assert_eq!(op(BFDOT, 0, 0x7FC0, 0x3F80).0 as u32, 0x7FC0_0000);
    assert_eq!(op(BFDOT, AH, 0x7FC0, 0x3F80).0 as u32, 0xFFC0_0000);
    let mlal = |fpcr| run(BFMLALB, fpcr, [ONE, 0x7F81, 0x3F80, 0]);
    let (d, fpsr) = mlal(0);
    assert_eq!((d as u32, fpsr), (0x7FC1_0000, IOC));
    let (d, fpsr) = mlal(AH);
    assert_eq!((d as u32, fpsr), (0x7FC1_0000, 0));
}

/// FPRoundBase with `N == 16` under FPCR.FZ16: a tiny result flushes
/// before rounding raising Underflow under FPCR.AH = 0, after rounding
/// raising Underflow and Inexact under FPCR.AH = 1.
#[test]
fn fz16_flushes_after_rounding_under_alternate_handling() {
    // 2^-10 × 2^-10 = 2^-20, an exact denormal.
    assert_eq!(op(FMUL_H, FZ16, 0x1400, 0x1400), (0, UFC));
    assert_eq!(op(FMUL_H, AH | FZ16, 0x1400, 0x1400), (0, UFC | IXC));
    assert_eq!(op(FMUL_H, AH, 0x1400, 0x1400), (0x0010, 0));
}

/// The upper bits of V0-V3 for the merging test: a different pattern per
/// register above its low 32 bits, which hold `low`.
fn with_upper(n: u32, low: u128) -> u128 {
    (u128::from(n + 1) * 0x1111_1111_1111_1111_1111_1111_0000_0000) | low
}

/// IsMerging: under FPCR.NEP a scalar result's upper bits come from the
/// source register each page names — Vn for FADD, FMUL by element, Va for
/// FMADD, Vm for the compares, Vd for the conversions, estimates and FMLA
/// by element — and are zeroed by FMOV, the compares with zero and the
/// pairwise forms, by everything without FPCR.NEP, and in Streaming SVE
/// mode, where FPCR.NEP reads as 0 without FEAT_SME_FA64.
#[test]
fn nep_merges_scalar_results_into_the_named_register() {
    let v = [with_upper(0, 0), with_upper(1, ONE), with_upper(2, ONE), with_upper(3, ONE)];
    let merged = |insn: u32, fpcr: u64| {
        let mut cpu = processor(Implementation::EVERY_FEATURE);
        cpu.fpcr = fpcr;
        cpu.v[..4].copy_from_slice(&v);
        cpu.x[1] = 3;
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
        cpu.v[0]
    };
    let cases: [(u32, u32, Option<u32>); 11] = [
        (FADD_S, 32, Some(1)),
        (FMADD_S, 32, Some(3)),
        (FMADD_D, 64, Some(3)),
        (FCMEQ_S, 32, Some(2)),
        (FCMEQ_ZERO_S, 32, None),
        (FMOV_S, 32, None),
        (FMOV_S_ONE, 32, None),
        (SCVTF_S_W, 32, Some(0)),
        (FRECPE_S, 32, Some(0)),
        (FMLA_S_ELEM, 32, Some(0)),
        (FMUL_S_ELEM, 32, Some(1)),
    ];
    for (insn, esize, source) in cases {
        let expected = source.map_or(0, |n| with_upper(n, 0) >> esize << esize);
        assert_eq!(merged(insn, NEP) >> esize << esize, expected, "{insn:#010x} under NEP");
        assert_eq!(merged(insn, 0) >> esize, 0, "{insn:#010x} without NEP");
    }
    assert_eq!(merged(FADD_S, NEP) as u32, 0x4000_0000, "1.0 + 1.0");
    assert_eq!(merged(FADDP_S, NEP) >> 32, 0);

    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.set_pstate_sm(true);
    cpu.fpcr = NEP;
    cpu.v[..4].copy_from_slice(&v);
    assert_eq!(exec(&mut cpu, FADD_S), None);
    assert_eq!(cpu.v[0] >> 32, 0, "streaming: NEP reads as 0");
}
