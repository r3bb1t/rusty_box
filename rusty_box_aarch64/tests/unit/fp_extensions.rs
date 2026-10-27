//! FEAT_FCMA, FEAT_JSCVT, FEAT_FRINTTS and FEAT_BF16 as a guest sees them,
//! on a processor that implements them ([`Implementation::EVERY_FEATURE`])
//! and on the Cortex-A57, which does not.
//!
//! The values the instructions compute are checked against QEMU
//! (`tests/vectors/qemu_max.txt`); this file holds what those vectors cannot
//! show (R9): the sizes the decode pseudocode refuses, the half-precision
//! forms of FCMA without FEAT_FP16, the cases QEMU 8.2.2 gets wrong
//! (docs/qemu-upstream-bugs.md) and the ones random inputs rarely reach,
//! each pinned to the Arm ARM's pseudocode (FPToFixedJS, FPRoundIntN,
//! FPConvertBF, BFDotAdd, BFMulAddH), and the enable checks after decode.
//!
//! The words are GNU as's (`-march=armv8.6-a+fp16+bf16`, objdump -d).

use super::features::{Feature, Features, Implementation, VecLen};
use super::step::{exec, processor, CPACR_ALL, FP_TRAP, ILLEGAL_WHEN_STREAMING, UNDEFINED};

/// `fcmla v0.4s, v1.4s, v2.4s, #90`, `fcmla v0.4h, v1.4h, v2.4h, #0`.
const FCMLA_4S: u32 = 0x6E82_CC20;
const FCMLA_4H: u32 = 0x2E42_C420;
/// `fcadd v0.2d, v1.2d, v2.2d, #270`, `fcadd v0.8h, v1.8h, v2.8h, #90`.
const FCADD_2D: u32 = 0x6EC2_F420;
const FCADD_8H: u32 = 0x6E42_E420;
/// `fcmla v0.4s, v1.4s, v2.s[1], #180`, `fcmla v0.4h, v1.4h, v2.h[1],
/// #270`, `fcmla v0.8h, v1.8h, v2.h[3], #0`.
const FCMLA_ELEM_4S: u32 = 0x6F82_5820;
const FCMLA_ELEM_4H: u32 = 0x2F62_7020;
const FCMLA_ELEM_8H: u32 = 0x6F62_1820;
/// `fjcvtzs w0, d1`.
const FJCVTZS: u32 = 0x1E7E_0020;
/// `frint32z s0, s1`, `frint32x d0, d1`, `frint64z s0, s1`, `frint64x d0,
/// d1`.
const FRINT32Z_S: u32 = 0x1E28_4020;
const FRINT32X_D: u32 = 0x1E68_C020;
const FRINT64Z_S: u32 = 0x1E29_4020;
const FRINT64X_D: u32 = 0x1E69_C020;
/// `frint32x v0.4s, v1.4s`, `frint64z v0.2d, v1.2d`.
const FRINT32X_4S: u32 = 0x6E21_E820;
const FRINT64Z_2D: u32 = 0x4E61_F820;
/// `bfcvt h0, s1`, `bfcvtn v0.4h, v1.4s`, `bfcvtn2 v0.8h, v1.4s`.
const BFCVT: u32 = 0x1E63_4020;
const BFCVTN: u32 = 0x0EA1_6820;
const BFCVTN2: u32 = 0x4EA1_6820;
/// `bfdot v0.4s, v1.8h, v2.8h`, `bfdot v0.2s, v1.4h, v2.2h[3]`.
const BFDOT: u32 = 0x6E42_FC20;
const BFDOT_ELEM: u32 = 0x0F62_F820;
/// `bfmlalb v0.4s, v1.8h, v2.8h`, `bfmlalt v0.4s, v1.8h, v2.8h`, `bfmlalt
/// v0.4s, v1.8h, v2.h[7]`.
const BFMLALB: u32 = 0x2EC2_FC20;
const BFMLALT: u32 = 0x6EC2_FC20;
const BFMLALT_ELEM: u32 = 0x4FF2_F820;
/// `bfmmla v0.4s, v1.8h, v2.8h`.
const BFMMLA: u32 = 0x6E42_EC20;

/// The scalar floating-point forms, which `AArch64_CheckFPEnabled` checks.
const SCALAR_FP: [u32; 5] = [FRINT32Z_S, FRINT32X_D, FRINT64Z_S, FRINT64X_D, BFCVT];
/// The forms `AArch64_CheckFPAdvSIMDEnabled` checks.
const ADVSIMD: [u32; 19] = [
    FCMLA_4S,
    FCMLA_4H,
    FCADD_2D,
    FCADD_8H,
    FCMLA_ELEM_4S,
    FCMLA_ELEM_4H,
    FCMLA_ELEM_8H,
    FJCVTZS,
    FRINT32X_4S,
    FRINT64Z_2D,
    BFCVTN,
    BFCVTN2,
    BFDOT,
    BFDOT_ELEM,
    BFMLALB,
    BFMLALT,
    BFMLALT_ELEM,
    BFMMLA,
    // fcmla v0.2d, v1.2d, v2.2d, #0
    0x6EC2_C420,
];

/// FPCR.FZ, FPCR.DN and FPCR.RMode's round toward +infinity, -infinity
/// and zero.
const FZ: u64 = 1 << 24;
const DN: u64 = 1 << 25;
const RP: u64 = 0b01 << 22;
const RM: u64 = 0b10 << 22;
const RZ: u64 = 0b11 << 22;
/// FPSR's cumulative flags.
const IOC: u64 = 1 << 0;
const OFC: u64 = 1 << 2;
const UFC: u64 = 1 << 3;
const IXC: u64 = 1 << 4;
const IDC: u64 = 1 << 7;

fn every_form() -> impl Iterator<Item = u32> {
    SCALAR_FP.into_iter().chain(ADVSIMD)
}

#[test]
fn without_the_features_their_encodings_are_undefined() {
    let mut cpu = processor(Implementation::CORTEX_A57);
    for insn in every_form() {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}

#[test]
fn with_the_features_every_form_executes() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for insn in every_form() {
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
    }
}

/// FCMLA and FCADD: `if size == '00'`, `if Q == '0' && size == '11'`, and
/// by element `if size == '11'`, `if size == '10' && (L == '1' || Q ==
/// '0')`, `if size == '01' && H == '1' && Q == '0'`, each `EndOfDecode
/// (Decode_UNDEF)`.
#[test]
fn sizes_the_decode_pseudocode_refuses_are_undefined() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    let size = |insn: u32, size: u32| insn & !(3 << 22) | (size << 22);
    let refused = [
        size(FCMLA_4S, 0b00),
        size(FCADD_2D, 0b00),
        size(FCMLA_4S, 0b11) & !(1 << 30),
        size(FCADD_2D, 0b11) & !(1 << 30),
        size(FCMLA_ELEM_4S, 0b00),
        size(FCMLA_ELEM_4S, 0b11),
        // L = 1, and Q = 0, with single precision.
        FCMLA_ELEM_4S | (1 << 21),
        FCMLA_ELEM_4S & !(1 << 30),
        // H = 1 with Q = 0, half precision.
        FCMLA_ELEM_4H | (1 << 11),
    ];
    for insn in refused {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}

/// FCMLA and FCADD of half precision: `if size == '01' &&
/// !IsFeatureImplemented(FEAT_FP16)`, on a processor with FEAT_FCMA alone,
/// which Arm's feature model permits.
#[test]
fn the_half_precision_complex_forms_need_fp16() {
    let imp = Implementation {
        features: Features::of(&[Feature::Base, Feature::Fp, Feature::AdvSimd, Feature::Fcma]),
        max_svl: VecLen::MIN,
        ..Implementation::CORTEX_A57
    };
    let mut cpu = processor(imp);
    for insn in [FCMLA_4H, FCADD_8H, FCMLA_ELEM_4H, FCMLA_ELEM_8H] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
    for insn in [FCMLA_4S, FCADD_2D, FCMLA_ELEM_4S] {
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
    }
}

/// FCMLA (by element) with `Q = 0` computes the two complex numbers of the
/// 64-bit vector and nothing above it: signalling NaNs in the upper halves
/// raise nothing. QEMU 8.2.2 raises their invalid operation; random inputs
/// rarely put a signalling NaN in every upper element.
#[test]
fn a_64_bit_fcmla_by_element_reads_only_its_elements() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    // 1.0 in the low four halves, a signalling NaN in each upper one.
    let lanes = 0x7D00_7D00_7D00_7D00_3C00_3C00_3C00_3C00u128;
    cpu.v[0] = lanes;
    cpu.v[1] = lanes;
    cpu.v[2] = lanes;
    cpu.fpsr = 0;
    assert_eq!(exec(&mut cpu, FCMLA_ELEM_4H), None);
    // #270: real = 1 + 1×1, imaginary = 1 + 1×-1.
    assert_eq!(cpu.v[0], 0x0000_4000_0000_4000);
    assert_eq!(cpu.fpsr, 0);
}

/// `fjcvtzs w0, d1` of `d` under `fpcr`: X0, NZCV and FPSR.
fn jscvt(d: u64, fpcr: u64) -> (u64, u64, u64) {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.v[1] = u128::from(d) | (u128::MAX << 64);
    cpu.x[0] = u64::MAX;
    cpu.fpcr = fpcr;
    cpu.fpsr = 0;
    assert_eq!(exec(&mut cpu, FJCVTZS), None);
    (cpu.x[0], cpu.pstate.nzcv() >> 28, cpu.fpsr)
}

/// FPToFixedJS: rounded toward zero, wrapped to 32 bits, Z for an exact
/// result. With FPCR.FZ a denormal is a zero of its sign with Z clear,
/// where QEMU 8.2.2 sets Z.
#[test]
fn fjcvtzs_follows_fptofixedjs() {
    const Z: u64 = 0b0100;
    assert_eq!(jscvt(0x3FF0_0000_0000_0000, 0), (1, Z, 0), "1.0");
    assert_eq!(jscvt(0x0000_0000_0000_0000, 0), (0, Z, 0), "+0.0");
    assert_eq!(jscvt(0x8000_0000_0000_0000, 0), (0, 0, 0), "-0.0");
    assert_eq!(jscvt(0xC00E_0000_0000_0000, 0), (0xFFFF_FFFD, 0, IXC), "-3.75");
    assert_eq!(jscvt(0xC1E0_0000_0000_0000, 0), (0x8000_0000, Z, 0), "-2^31");
    assert_eq!(jscvt(0x41E0_0000_0000_0000, 0), (0x8000_0000, 0, IOC), "2^31");
    assert_eq!(jscvt(0x41F0_0000_0050_0000, 0), (5, 0, IOC), "2^32 + 5");
    assert_eq!(jscvt(0x44F0_0000_0000_0000, 0), (0, 0, IOC), "2^80");
    assert_eq!(jscvt(0x7FF8_0000_0000_0000, 0), (0, 0, IOC), "NaN");
    assert_eq!(jscvt(0xFFF0_0000_0000_0000, 0), (0, 0, IOC), "-infinity");
    assert_eq!(jscvt(0x0000_0000_0000_0001, 0), (0, 0, IXC), "a denormal");
    assert_eq!(jscvt(0x0000_0000_0000_0001, FZ), (0, 0, IDC), "a denormal, flushed");
    assert_eq!(jscvt(0x800F_FFFF_FFFF_FFFF, FZ), (0, 0, IDC), "a negative denormal, flushed");
}

/// `insn` on S1 or D1 (`value`) under `fpcr`: V0 and FPSR.
fn scalar(insn: u32, value: u64, fpcr: u64) -> (u128, u64) {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.v[0] = u128::MAX;
    cpu.v[1] = u128::from(value);
    cpu.fpcr = fpcr;
    cpu.fpsr = 0;
    assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
    (cpu.v[0], cpu.fpsr)
}

/// FPRoundIntN: an integral value within the size's signed range, else
/// (an infinity and a NaN too) its most negative integer with invalid
/// alone.
#[test]
fn frint32_and_frint64_saturate_to_the_most_negative_integer() {
    assert_eq!(scalar(FRINT32Z_S, 0x4EFF_FFFF, 0), (0x4EFF_FFFF, 0), "2^31 - 128");
    assert_eq!(scalar(FRINT32Z_S, 0x4F00_0000, 0), (0xCF00_0000, IOC), "2^31");
    assert_eq!(scalar(FRINT32Z_S, 0xCF00_0000, 0), (0xCF00_0000, 0), "-2^31");
    assert_eq!(scalar(FRINT32Z_S, 0x7FC0_0000, 0), (0xCF00_0000, IOC), "NaN");
    assert_eq!(scalar(FRINT32Z_S, 0xFF80_0000, 0), (0xCF00_0000, IOC), "-infinity");
    assert_eq!(scalar(FRINT32Z_S, 0xBF40_0000, 0), (0x8000_0000, IXC), "-0.75 is -0.0");
    assert_eq!(scalar(FRINT32Z_S, 0x8000_0001, FZ), (0x8000_0000, IDC), "a flushed denormal");
    assert_eq!(scalar(FRINT64Z_S, 0x5F00_0000, 0), (0xDF00_0000, IOC), "2^63");
    assert_eq!(scalar(FRINT64Z_S, 0x5E80_0000, 0), (0x5E80_0000, 0), "2^62");
    // 2^31 - 0.5 rounds to 2^31 to nearest: out of range, without inexact.
    assert_eq!(scalar(FRINT32X_D, 0x41DF_FFFF_FFE0_0000, 0), (0xC1E0_0000_0000_0000, IOC));
    assert_eq!(scalar(FRINT32X_D, 0x41DF_FFFF_FFE0_0000, RZ), (0x41DF_FFFF_FFC0_0000, IXC));
    assert_eq!(scalar(FRINT64X_D, 0x3FF4_0000_0000_0000, RP), (0x4000_0000_0000_0000, IXC), "1.25 up");
    assert_eq!(scalar(FRINT64X_D, 0xC3E0_0000_0000_0000, 0), (0xC3E0_0000_0000_0000, 0), "-2^63");
}

/// FPConvertBF: FPCR's rounding, with ties to even, and FPRoundBase's
/// exceptions; random inputs almost never hold an exact tie.
#[test]
fn bfcvt_rounds_by_fpcr() {
    assert_eq!(scalar(BFCVT, 0x3F80_0000, 0), (0x3F80, 0), "1.0");
    assert_eq!(scalar(BFCVT, 0x3F80_8000, 0), (0x3F80, IXC), "a tie, to even");
    assert_eq!(scalar(BFCVT, 0x3F81_8000, 0), (0x3F82, IXC), "a tie, to even");
    assert_eq!(scalar(BFCVT, 0x3F81_FFFF, RZ), (0x3F81, IXC));
    assert_eq!(scalar(BFCVT, 0x3F80_0001, RP), (0x3F81, IXC));
    assert_eq!(scalar(BFCVT, 0xBF80_0001, RP), (0xBF80, IXC));
    assert_eq!(scalar(BFCVT, 0xBF80_0001, RM), (0xBF81, IXC));
    assert_eq!(scalar(BFCVT, 0x7F7F_FFFF, 0), (0x7F80, OFC | IXC), "to infinity");
    assert_eq!(scalar(BFCVT, 0x7F7F_FFFF, RZ), (0x7F7F, IXC), "to the largest normal");
    assert_eq!(scalar(BFCVT, 0x0000_0001, 0), (0x0000, UFC | IXC), "tiny and inexact");
    assert_eq!(scalar(BFCVT, 0x0001_8000, 0), (0x0002, UFC | IXC), "a denormal tie");
    assert_eq!(scalar(BFCVT, 0x0001_0000, 0), (0x0001, 0), "an exact denormal");
    assert_eq!(scalar(BFCVT, 0x807F_FFFF, FZ), (0x8000, IDC), "a flushed denormal");
    assert_eq!(scalar(BFCVT, 0x7F80_0001, 0), (0x7FC0, IOC), "a signalling NaN, quieted");
    assert_eq!(scalar(BFCVT, 0xFFA0_0000, DN), (0x7FC0, IOC), "the default NaN");
    assert_eq!(scalar(BFCVT, 0xFFC1_2345, 0), (0xFFC1, 0), "a quiet NaN's payload, truncated");
}

/// BFCVTN writes the low half of Vd and zeroes the rest, BFCVTN2 the high
/// half and keeps the low (`Vpart`).
#[test]
fn bfcvtn_fills_one_half_of_vd() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    // 1.0, 2.0, -0.5, 3.0.
    cpu.v[1] = 0x4040_0000_BF00_0000_4000_0000_3F80_0000;
    cpu.v[0] = u128::MAX;
    assert_eq!(exec(&mut cpu, BFCVTN), None);
    assert_eq!(cpu.v[0], 0x4040_BF00_4000_3F80);
    assert_eq!(exec(&mut cpu, BFCVTN2), None);
    assert_eq!(cpu.v[0], 0x4040_BF00_4000_3F80_4040_BF00_4000_3F80);
}

/// `insn` on V0 = `d`, V1 = `n`, V2 = `m` under `fpcr`: V0 and FPSR.
fn vector(insn: u32, d: u128, n: u128, m: u128, fpcr: u64) -> (u128, u64) {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.v[0] = d;
    cpu.v[1] = n;
    cpu.v[2] = m;
    cpu.fpcr = fpcr;
    cpu.fpsr = 0;
    assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
    (cpu.v[0], cpu.fpsr)
}

/// BFDotAdd without FEAT_EBF16: BFloat16's own behaviours whatever FPCR
/// says, rounding to odd, flushing denormals, the default NaN, and no
/// exception.
#[test]
fn bfdot_follows_bfloat16_behaviours_not_fpcr() {
    let lane0 = |n: [u16; 2], m: [u16; 2], addend: u32, fpcr: u64| {
        let n = u128::from(n[0]) | (u128::from(n[1]) << 16);
        let m = u128::from(m[0]) | (u128::from(m[1]) << 16);
        let (d, fpsr) = vector(BFDOT, u128::from(addend), n, m, fpcr);
        assert_eq!(fpsr, 0, "no exception");
        d as u32
    };
    // 1 + 1×3 + 2×4.
    assert_eq!(lane0([0x3F80, 0x4000], [0x4040, 0x4080], 0x3F80_0000, 0), 0x4140_0000);
    // 1 + 2^-12 × 2^-12 rounds to odd, whatever FPCR.RMode.
    for fpcr in [0, RP, RM, RZ] {
        assert_eq!(lane0([0x3980, 0], [0x3980, 0], 0x3F80_0000, fpcr), 0x3F80_0001);
    }
    // A signalling NaN gives the default NaN, FPCR.DN or not.
    assert_eq!(lane0([0x7F81, 0], [0x3F80, 0], 0x3F80_0000, 0), 0x7FC0_0000);
    // A denormal operand and a denormal addend are zeros, FPCR.FZ or not.
    assert_eq!(lane0([0x0001, 0], [0x3F80, 0], 0x0000_0001, 0), 0);
    // The largest normal squared overflows to an infinity.
    assert_eq!(lane0([0x7F7F, 0], [0x7F7F, 0], 0, RZ), 0x7F80_0000);
}

/// BFDOT by element takes the pair `H:L` of all of Vm.
#[test]
fn bfdot_by_element_reads_the_indexed_pair() {
    // n = (1, 1) in each lane; m's pair 3 = (2, 3), the others NaN.
    let n = 0x3F80_3F80_3F80_3F80u128;
    let m = (0x4040_4000u128 << 96) | 0x7FC0_7FC0_7FC0_7FC0_7FC0_7FC0;
    let (d, fpsr) = vector(BFDOT_ELEM, u128::MAX, n, m, 0);
    // Each lane: 1×2 + 1×3 added to the NaN pattern of all ones, a NaN:
    // the default NaN.
    assert_eq!((d, fpsr), (0x7FC0_0000_7FC0_0000, 0));
    let (d, _) = vector(BFDOT_ELEM, 0, n, m, 0);
    assert_eq!(d, 0x40A0_0000_40A0_0000, "5.0 in each of the two lanes");
}

/// BFMulAddH: the even (`B`) or odd (`T`) elements, a single-precision
/// fused multiply-add under FPCR, BFloat16 denormals included.
#[test]
fn bfmlal_multiplies_the_bottom_or_top_elements_under_fpcr() {
    // n = (1, 2) and m = (3, 4) in every pair.
    let n = 0x4000_3F80_4000_3F80_4000_3F80_4000_3F80u128;
    let m = 0x4080_4040_4080_4040_4080_4040_4080_4040u128;
    assert_eq!(vector(BFMLALB, 0, n, m, 0), (0x4040_0000_4040_0000_4040_0000_4040_0000, 0), "1×3");
    assert_eq!(vector(BFMLALT, 0, n, m, 0), (0x4100_0000_4100_0000_4100_0000_4100_0000, 0), "2×4");
    // By element: the element `H:L:M` = 7 of V2.
    let m7 = 0x4080u128 << 112;
    assert_eq!(vector(BFMLALT_ELEM, 0, n, m7, 0).0, 0x4100_0000_4100_0000_4100_0000_4100_0000);
    // The smallest BFloat16 denormal, a single-precision denormal: kept
    // without FPCR.FZ, flushed with input denormal under it.
    let tiny = 0x0001_0001_0001_0001_0001_0001_0001_0001u128;
    let one = 0x3F80_3F80_3F80_3F80_3F80_3F80_3F80_3F80u128;
    assert_eq!(vector(BFMLALB, 0, tiny, one, 0), (0x0001_0000_0001_0000_0001_0000_0001_0000, 0));
    assert_eq!(vector(BFMLALB, 0, tiny, one, FZ), (0, IDC));
}

/// BFMatMulAddH: a 2×2 accumulator plus the product of the 2×4 matrix Vn
/// and the transpose of the 2×4 matrix Vm.
#[test]
fn bfmmla_multiplies_two_by_four_matrices() {
    // Vn's rows (1, 1, 1, 1) and (2, 2, 2, 2); Vm's (1, 1, 1, 1) and (0.5,
    // 0.5, 0.5, 0.5).
    let n = 0x4000_4000_4000_4000_3F80_3F80_3F80_3F80u128;
    let m = 0x3F00_3F00_3F00_3F00_3F80_3F80_3F80_3F80u128;
    // (4, 2; 8, 4), row by row.
    assert_eq!(vector(BFMMLA, 0, n, m, 0), (0x4080_0000_4100_0000_4000_0000_4080_0000, 0));
}

/// `AArch64_CheckFPEnabled` and `AArch64_CheckFPAdvSIMDEnabled` run after
/// decode: with SIMD&FP disabled every form takes the access trap; in
/// Streaming SVE mode the scalar FRINT32*, FRINT64* and BFCVT execute, and
/// the Advanced SIMD forms and FJCVTZS take the SME trap (SMTC 1).
#[test]
fn the_fp_checks_follow_decode() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.sys.cpacr_el1 = 3 << 24;
    for insn in every_form() {
        assert_eq!(exec(&mut cpu, insn), Some(FP_TRAP), "{insn:#010x}");
    }
    cpu.sys.cpacr_el1 = CPACR_ALL;
    // smstart sm
    assert_eq!(exec(&mut cpu, 0xD503_437F), None);
    for insn in SCALAR_FP {
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
    }
    for insn in ADVSIMD {
        assert_eq!(exec(&mut cpu, insn), Some(ILLEGAL_WHEN_STREAMING), "{insn:#010x}");
    }
}
