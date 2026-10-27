//! FEAT_SME2's multi-vector instructions on ZA array vectors, on a
//! processor that implements FEAT_SME2 ([`Implementation::EVERY_FEATURE`])
//! in Streaming SVE mode with ZA storage enabled.
//!
//! Each property is the guest-visible one (R9), each expected value worked
//! from the instruction's Arm pseudocode: the vector `(Wv + offset) MOD
//! vstride` in each register's stride, the stride by the vector length and
//! the register count, the long forms' vectors from a multiple of two or
//! four and their element `width * e + i`, the single register group
//! wrapping from Z31 to Z0, and the second group stepping with the first;
//! the integer products' signs, mixed signs, subtraction and wrap; the dot
//! products of bytes and halfwords; FMLA's one rounding, default NaN,
//! negative under FPCR.AH, and silent exceptions; FDOT's rounding of the
//! pair before the sum; the long products' element pairs and FMLSL's and
//! BFMLSL's negation; ADD and SUB replacing the vectors, and FADD, FSUB,
//! ADD and SUB accumulating into them; MOVA between a group and the
//! vectors, its offset unscaled. By an indexed element: the element,
//! or group, of the segment holding each result element at 256 bits, each
//! class's index fields, a group's mixed-sign bit, and the vertical dot
//! products' elements across the registers. The checks: Streaming SVE
//! mode, ZA storage, FEAT_SME2, and the 64-bit, FP8 and half-precision
//! forms UNDEFINED.

use std::vec;
use std::vec::Vec;

use super::super::features::{Feature, Features, Implementation, VecLen};
use super::super::step::{exec, processor, Taken, UNDEFINED};
use super::super::sve::{Esize, Zv};
use super::super::Cpu;
use crate::softfloat::FPCR_AH;

/// The SME trap with SMTC 2: outside Streaming SVE mode.
const NOT_STREAMING: Taken = Taken { ec: 0x1D, iss: 2 };
/// The SME trap with SMTC 3: ZA storage disabled.
const INACTIVE_ZA: Taken = Taken { ec: 0x1D, iss: 3 };

/// `fmla za.s[w8, 1, vgx2], {z0.s-z1.s}, z2.s` and `fmls`.
const FMLA_2X1: u32 = 0xC122_1801;
const FMLS_2X1: u32 = 0xC122_1809;
/// `fmla za.s[w9, 7, vgx4], {z4.s-z7.s}, {z8.s-z11.s}`.
const FMLA_4X4: u32 = 0xC1A9_3887;
/// `smlall za.s[w9, 4:7], z0.b, z1.b`, `smlsll` and `umlall`.
const SMLALL_1: u32 = 0xC121_2401;
const SMLSLL_1: u32 = 0xC121_2409;
const UMLALL_1: u32 = 0xC121_2411;
/// `usmlall za.s[w9, 0:3, vgx2], {z0.b-z1.b}, z2.b` and `sumlall`.
const USMLALL_2X1: u32 = 0xC122_2004;
const SUMLALL_2X1: u32 = 0xC122_2014;
/// `smlal za.s[w10, 0:1, vgx4], {z30.h, z31.h, z0.h, z1.h}, z2.h`.
const SMLAL_4X1_Z30: u32 = 0xC172_4BC0;
/// `umlsl za.s[w10, 6:7], z0.h, z1.h`.
const UMLSL_1: u32 = 0xC161_4C1B;
/// `fmlal za.s[w11, 0:1], z0.h, z1.h`, `fmlsl`, `bfmlal` and `bfmlsl`.
const FMLAL_1: u32 = 0xC121_6C00;
const FMLSL_1: u32 = 0xC121_6C08;
const BFMLAL_1: u32 = 0xC121_6C10;
const BFMLSL_1: u32 = 0xC121_6C18;
/// `sdot za.s[w8, 0, vgx2], {z0.b-z1.b}, z2.b`, `udot`, `usdot` and
/// `sudot`.
const SDOT_2X1_B: u32 = 0xC122_1400;
const UDOT_2X1_B: u32 = 0xC122_1410;
const USDOT_2X1: u32 = 0xC122_1408;
const SUDOT_2X1: u32 = 0xC122_1418;
/// `sdot za.s[w8, 0, vgx2], {z0.h-z1.h}, z2.h`.
const SDOT_2X1_H: u32 = 0xC162_1408;
/// `udot za.s[w8, 0, vgx4], {z4.h-z7.h}, {z8.h-z11.h}`.
const UDOT_4X4_H: u32 = 0xC1E9_1498;
/// `fdot za.s[w8, 0, vgx2], {z0.h-z1.h}, z2.h` and `bfdot`.
const FDOT_2X1: u32 = 0xC122_1000;
const BFDOT_2X1: u32 = 0xC122_1010;
/// `bfdot za.s[w8, 0, vgx4], {z4.h-z7.h}, {z8.h-z11.h}`.
const BFDOT_4X4: u32 = 0xC1A9_1090;
/// `add za.s[w8, 3, vgx2], {z0.s-z1.s}, z2.s`.
const ADD_ZZV_2X1: u32 = 0xC122_1813;
/// `sub za.s[w8, 3, vgx4], {z4.s-z7.s}, {z8.s-z11.s}`.
const SUB_ZZW_4X4: u32 = 0xC1A9_189B;
/// `add za.s[w8, 3, vgx2], {z2.s-z3.s}` and `fadd`.
const ADD_ZW_2X2: u32 = 0xC1A0_1C53;
const FADD_ZW_2X2: u32 = 0xC1A0_1C43;
/// `sub za.s[w8, 3, vgx4], {z4.s-z7.s}` and `fsub`.
const SUB_ZW_4X4: u32 = 0xC1A1_1C9B;
const FSUB_ZW_4X4: u32 = 0xC1A1_1C8B;
/// The 64-bit forms: `smlall za.d[w9, 4:7], z0.h, z1.h`, `fmla za.d[w8, 1,
/// vgx2], {z0.d-z1.d}, z2.d`, `add za.d[w8, 3, vgx2], {z0.d-z1.d}, z2.d`,
/// `fadd za.d[w8, 3, vgx2], {z2.d-z3.d}` and `sdot za.d[w8, 0, vgx2],
/// {z0.h-z1.h}, z2.h`.
const SMLALL_D: u32 = 0xC161_2401;
const FMLA_D: u32 = 0xC162_1801;
const ADD_D: u32 = 0xC162_1813;
const FADD_D: u32 = 0xC1E0_1C43;
const SDOT_D: u32 = 0xC162_1400;

/// By an indexed element: `smlall za.s[w8, 4:7], z0.b, z1.b[13]`, `usmlall
/// za.s[w8, 0:3, vgx2], {z2.b-z3.b}, z1.b[6]` and `sumlall za.s[w8, 0:3,
/// vgx4], {z4.b-z7.b}, z1.b[9]`.
const SMLALL_IDX_1: u32 = 0xC101_9401;
const USMLALL_IDX_2: u32 = 0xC111_0464;
const SUMLALL_IDX_4: u32 = 0xC111_88B2;
/// `smlal za.s[w9, 2:3], z0.h, z1.h[5]` and `umlsl za.s[w9, 0:1, vgx4],
/// {z4.h-z7.h}, z1.h[3]`.
const SMLAL_IDX_1: u32 = 0xC1C1_B401;
const UMLSL_IDX_4: u32 = 0xC1D1_B49C;
/// `fmlal za.s[w10, 0:1], z0.h, z1.h[7]` and `bfmlsl za.s[w10, 0:1,
/// vgx2], {z2.h-z3.h}, z1.h[2]`.
const FMLAL_IDX_1: u32 = 0xC181_DC00;
const BFMLSL_IDX_2: u32 = 0xC191_5458;
/// `fmla za.s[w11, 1, vgx2], {z2.s-z3.s}, z1.s[3]` and `fmls za.s[w11, 1,
/// vgx4], {z4.s-z7.s}, z1.s[1]`.
const FMLA_IDX_2: u32 = 0xC151_6C41;
const FMLS_IDX_4: u32 = 0xC151_E491;
/// `sdot za.s[w8, 0, vgx2], {z2.b-z3.b}, z1.b[2]`, `usdot za.s[w8, 0,
/// vgx4], {z4.b-z7.b}, z1.b[1]` and `udot za.s[w8, 0, vgx2], {z2.h-z3.h},
/// z1.h[3]`.
const SDOT_IDX_2: u32 = 0xC151_1860;
const USDOT_IDX_4: u32 = 0xC151_94A8;
const UDOT_IDX_2_H: u32 = 0xC151_1C50;
/// `fdot za.s[w8, 0, vgx2], {z2.h-z3.h}, z1.h[1]`.
const FDOT_IDX_2: u32 = 0xC151_1448;
/// `fvdot za.s[w8, 0, vgx2], {z2.h-z3.h}, z1.h[1]` and `bfvdot ...,
/// z1.h[3]`.
const FVDOT_2: u32 = 0xC151_0448;
const BFVDOT_2: u32 = 0xC151_0C58;
/// `svdot za.s[w8, 0, vgx2], {z2.h-z3.h}, z1.h[2]` and `uvdot ..., z1.h[0]`.
const SVDOT_2_H: u32 = 0xC151_0860;
const UVDOT_2_H: u32 = 0xC151_0070;
/// `svdot za.s[w8, 0, vgx4], {z4.b-z7.b}, z1.b[1]`, `usvdot ..., z1.b[3]`,
/// `uvdot ..., z1.b[0]` and `suvdot ..., z1.b[2]`.
const SVDOT_4: u32 = 0xC151_84A0;
const USVDOT_4: u32 = 0xC151_8CA8;
const UVDOT_4: u32 = 0xC151_80B0;
const SUVDOT_4: u32 = 0xC151_88B8;
/// The indexed forms of other features: `smlall za.d[w8, 0:3], z0.h,
/// z1.h[1]` and `sdot za.d[w8, 0, vgx2], {z2.h-z3.h}, z1.h[1]`
/// (FEAT_SME_I16I64), `fmla za.d[w8, 0, vgx2], {z2.d-z3.d}, z1.d[1]`
/// (FEAT_SME_F64F64), `fdot za.s[w8, 0, vgx2], {z2.b-z3.b}, z1.b[1]`
/// (FEAT_SME_F8F32) and `fmla za.h[w8, 0, vgx2], {z2.h-z3.h}, z1.h[1]`
/// (FEAT_SME_F16F16).
const SMLALL_IDX_D: u32 = 0xC181_0400;
const SDOT_IDX_D: u32 = 0xC1D1_0448;
const FMLA_IDX_D: u32 = 0xC1D1_0440;
const FDOT_IDX_FP8: u32 = 0xC151_0478;
const FMLA_IDX_H: u32 = 0xC111_1048;
/// `mova za.d[w8, 3, vgx2], {z4.d-z5.d}`, `mova za.d[w9, 7, vgx4],
/// {z8.d-z11.d}`, `mova {z4.d-z5.d}, za.d[w10, 1, vgx2]` and `mova
/// {z12.d-z15.d}, za.d[w11, 5, vgx4]`.
const MOVA_TO_ARRAY_2: u32 = 0xC004_0883;
const MOVA_TO_ARRAY_4: u32 = 0xC004_2D07;
const MOVA_FROM_ARRAY_2: u32 = 0xC006_4824;
const MOVA_FROM_ARRAY_4: u32 = 0xC006_6CAC;

/// Single-precision values.
const ZERO: u64 = 0;
const MINUS_ZERO: u64 = 0x8000_0000;
const HALF: u64 = 0x3F00_0000;
const ONE: u64 = 0x3F80_0000;
const MINUS_ONE: u64 = 0xBF80_0000;
const ONE_AND_A_HALF: u64 = 0x3FC0_0000;
const TWO: u64 = 0x4000_0000;
const TWO_AND_A_HALF: u64 = 0x4020_0000;
const THREE: u64 = 0x4040_0000;
const MINUS_THREE: u64 = 0xC040_0000;
const THREE_AND_A_HALF: u64 = 0x4060_0000;
const FOUR: u64 = 0x4080_0000;
const FOUR_AND_A_HALF: u64 = 0x4090_0000;
const MINUS_FIVE: u64 = 0xC0A0_0000;
const SIX_AND_A_HALF: u64 = 0x40D0_0000;
const SEVEN: u64 = 0x40E0_0000;
const MINUS_SEVEN: u64 = 0xC0E0_0000;
const NINE: u64 = 0x4110_0000;
const INF: u64 = 0x7F80_0000;
const SEVEN_AND_A_HALF: u64 = 0x40F0_0000;
const EIGHT_AND_A_HALF: u64 = 0x4108_0000;
const ELEVEN_AND_A_HALF: u64 = 0x4138_0000;
/// A signalling NaN, and the default NaN, positive and under FPCR.AH.
const SNAN: u64 = 0x7F80_0001;
const DEFAULT_NAN: u64 = 0x7FC0_0000;
const DEFAULT_NAN_AH: u64 = 0xFFC0_0000;

/// A processor with FEAT_SME2 in Streaming SVE mode with ZA storage
/// enabled, `SMCR_EL1.LEN = len`, and FPCR and FPSR clear.
fn with_za(len: u64) -> Cpu {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.sys.smcr_el1 = len;
    cpu.set_pstate_sm(true);
    cpu.set_pstate_za(true);
    cpu.fpcr = 0;
    cpu.fpsr = 0;
    cpu
}

/// `values` as elements of `es`, repeated from the first element through
/// a vector of the current length.
fn vector(cpu: &Cpu, es: Esize, values: &[u64]) -> Zv {
    let mut z = Zv::ZERO;
    for e in 0..es.elements(cpu.current_svl()) {
        z.set(e, es, values[e % values.len()]);
    }
    z
}

/// Z register `n` holding `values` as elements of `es`, repeated.
fn set_z(cpu: &mut Cpu, n: u32, es: Esize, values: &[u64]) {
    let z = vector(cpu, es, values);
    let svl = cpu.current_svl();
    cpu.set_z(n, svl, &z);
}

/// Z register `n` with element `i` of `es` being `f(i)`.
fn set_z_with<F: Fn(usize) -> u64>(cpu: &mut Cpu, n: u32, es: Esize, f: F) {
    let svl = cpu.current_svl();
    let mut z = Zv::ZERO;
    for i in 0..es.elements(svl) {
        z.set(i, es, f(i));
    }
    cpu.set_z(n, svl, &z);
}

/// ZA vector `n` holding `words`, repeated.
fn set_za(cpu: &mut Cpu, n: usize, words: &[u64]) {
    let v = vector(cpu, Esize::S, words);
    let svl = cpu.current_svl();
    cpu.set_za_vector(n, svl, &v);
}

/// Every ZA vector holding `word` in each word.
fn fill_za(cpu: &mut Cpu, word: u64) {
    for n in 0..cpu.current_svl().bytes() {
        set_za(cpu, n, &[word]);
    }
}

/// The words of ZA vector `n`.
fn za_words(cpu: &Cpu, n: usize) -> Vec<u64> {
    let svl = cpu.current_svl();
    let v = cpu.za_vector(n, svl);
    (0..Esize::S.elements(svl)).map(|e| v.get(e, Esize::S)).collect()
}

/// The words of Z register `n`.
fn z_words(cpu: &Cpu, n: u32) -> Vec<u64> {
    let svl = cpu.current_svl();
    let z = cpu.z(n, svl);
    (0..Esize::S.elements(svl)).map(|e| z.get(e, Esize::S)).collect()
}

/// The ZA vectors holding a word other than `word`.
fn changed(cpu: &Cpu, word: u64) -> Vec<usize> {
    (0..cpu.current_svl().bytes()).filter(|&n| za_words(cpu, n).iter().any(|&w| w != word)).collect()
}

/// Each register `r` writes vector `(Wv + offset) MOD vstride` of stride
/// `r`: at 128 bits two registers divide ZA's 16 vectors into strides of 8,
/// so W8 = 9 and offset 1 select vectors 2 and 10; at 256 bits four
/// registers divide its 32 into strides of 8, so W9 = 2 and offset 7
/// select vectors 1, 9, 17 and 25, register `r` by `Zm + r`.
#[test]
fn each_register_writes_the_selected_vector_of_its_own_stride() {
    let mut cpu = with_za(0);
    fill_za(&mut cpu, HALF);
    set_z(&mut cpu, 0, Esize::S, &[ONE]);
    set_z(&mut cpu, 1, Esize::S, &[TWO]);
    set_z(&mut cpu, 2, Esize::S, &[THREE]);
    cpu.x[8] = 9;
    assert_eq!(exec(&mut cpu, FMLA_2X1), None);
    assert_eq!(changed(&cpu, HALF), [2, 10]);
    assert_eq!(za_words(&cpu, 2), [THREE_AND_A_HALF; 4]);
    assert_eq!(za_words(&cpu, 10), [SIX_AND_A_HALF; 4]);

    let mut cpu = with_za(1);
    fill_za(&mut cpu, HALF);
    for (r, value) in [ONE, TWO, THREE, FOUR].into_iter().enumerate() {
        set_z(&mut cpu, 4 + r as u32, Esize::S, &[value]);
        set_z(&mut cpu, 8 + r as u32, Esize::S, &[TWO]);
    }
    cpu.x[9] = 2;
    assert_eq!(exec(&mut cpu, FMLA_4X4), None);
    assert_eq!(changed(&cpu, HALF), [1, 9, 17, 25]);
    for (n, want) in [(1, TWO_AND_A_HALF), (9, FOUR_AND_A_HALF), (17, SIX_AND_A_HALF), (25, EIGHT_AND_A_HALF)] {
        assert_eq!(za_words(&cpu, n), [want; 8], "vector {n}");
    }
}

/// FMLA is `FPMulAdd_ZA`: one rounding of `acc + a*b`, so `-1 + (1 +
/// 2^-23)(1 - 2^-24)` keeps the `2^-24 - 2^-47` a rounded product loses;
/// the default NaN for a NaN whatever FPCR.DN, negative under FPCR.AH; no
/// exception reaching FPSR. FMLS negates the register's element.
#[test]
fn fmla_is_one_rounding_with_the_default_nan_and_no_exceptions() {
    let mut cpu = with_za(0);
    // (7 + 1) MOD 8: vectors 0 and 8.
    cpu.x[8] = 7;
    set_z(&mut cpu, 0, Esize::S, &[0x3F80_0001]);
    set_z(&mut cpu, 1, Esize::S, &[SNAN]);
    set_z(&mut cpu, 2, Esize::S, &[0x3F7F_FFFF]);
    set_za(&mut cpu, 0, &[MINUS_ONE]);
    assert_eq!(exec(&mut cpu, FMLA_2X1), None);
    assert_eq!(za_words(&cpu, 0), [0x337F_FFFE; 4]);
    assert_eq!(za_words(&cpu, 8), [DEFAULT_NAN; 4]);
    assert_eq!(cpu.fpsr, 0);
    cpu.fpcr = FPCR_AH;
    assert_eq!(exec(&mut cpu, FMLA_2X1), None);
    assert_eq!(za_words(&cpu, 8), [DEFAULT_NAN_AH; 4]);
    assert_eq!(cpu.fpsr, 0);

    cpu.fpcr = 0;
    set_za(&mut cpu, 0, &[ONE]);
    assert_eq!(exec(&mut cpu, FMLS_2X1), None);
    assert_eq!(za_words(&cpu, 0), [0xB37F_FFFE; 4]);
}

/// The four-way long products write four vectors from the selected one
/// rounded down to a multiple of four: `(6 + 4) MOD 16` is 10, so vectors 8
/// to 11, word `e` of vector `8 + i` from bytes `4 * e + i`. The products
/// are signed or unsigned, subtracted by SMLSLL, and wrap modulo 2^32.
#[test]
fn the_four_way_long_products_write_four_vectors_from_a_multiple_of_four() {
    let mut cpu = with_za(0);
    cpu.x[9] = 6;
    // Bytes -128, 127, -1, 2 by -128, -128, 3, -1.
    set_z(&mut cpu, 0, Esize::B, &[0x80, 0x7F, 0xFF, 0x02]);
    set_z(&mut cpu, 1, Esize::B, &[0x80, 0x80, 0x03, 0xFF]);
    for (insn, want) in [
        (SMLALL_1, [0x3FFF, 0xFFFF_C07F, 0xFFFF_FFFC, 0xFFFF_FFFD]),
        (SMLSLL_1, [0xFFFF_BFFF, 0x3F7F, 2, 1]),
        (UMLALL_1, [0x3FFF, 0x3F7F, 0x2FC, 0x1FD]),
    ] {
        fill_za(&mut cpu, 0xFFFF_FFFF);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(changed(&cpu, 0xFFFF_FFFF), [8, 9, 10, 11], "{insn:#010x}");
        for (i, want) in want.into_iter().enumerate() {
            assert_eq!(za_words(&cpu, 8 + i), [want; 4], "{insn:#010x} vector {}", 8 + i);
        }
    }
}

/// USMLALL multiplies the registers' unsigned bytes by Zm's signed ones,
/// SUMLALL their signed bytes by Zm's unsigned ones: 255 or -1 and 2 by
/// -128 or 128, into vectors 4 to 7 and 12 to 15 (`6 MOD 8` rounded down
/// to 4).
#[test]
fn the_mixed_sign_long_products_take_each_operand_s_own_sign() {
    let mut cpu = with_za(0);
    cpu.x[9] = 6;
    set_z(&mut cpu, 0, Esize::B, &[0xFF]);
    set_z(&mut cpu, 1, Esize::B, &[0x02]);
    set_z(&mut cpu, 2, Esize::B, &[0x80]);
    for (insn, first, second) in [(USMLALL_2X1, 0xFFFF_8080, 0xFFFF_FF00), (SUMLALL_2X1, 0xFFFF_FF80, 0x100)] {
        fill_za(&mut cpu, 0);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(changed(&cpu, 0), [4, 5, 6, 7, 12, 13, 14, 15], "{insn:#010x}");
        for i in 0..4 {
            assert_eq!(za_words(&cpu, 4 + i), [first; 4], "{insn:#010x}");
            assert_eq!(za_words(&cpu, 12 + i), [second; 4], "{insn:#010x}");
        }
    }
}

/// The two-way long products write two vectors, word `e` of vector `i`
/// from halfwords `2 * e + i`, register `r` into stride `r`; a group from
/// Z30 wraps to Z0 and Z1. UMLSL of one register takes the offset `off3 *
/// 2`: `(17 + 6) MOD 16` rounded down to 6, and subtracts unsigned
/// products, 65535 * 65535 wrapping.
#[test]
fn the_two_way_long_products_write_pairs_and_wrap_the_register_group() {
    let mut cpu = with_za(0);
    cpu.x[10] = 0;
    set_z(&mut cpu, 30, Esize::H, &[3, 4]);
    set_z(&mut cpu, 31, Esize::H, &[5, 6]);
    set_z(&mut cpu, 0, Esize::H, &[7, 8]);
    set_z(&mut cpu, 1, Esize::H, &[0xFFFF, 1]);
    set_z(&mut cpu, 2, Esize::H, &[0xFFFE]);
    fill_za(&mut cpu, 0);
    assert_eq!(exec(&mut cpu, SMLAL_4X1_Z30), None);
    assert_eq!(changed(&cpu, 0), [0, 1, 4, 5, 8, 9, 12, 13]);
    for (n, want) in [
        (0, 0xFFFF_FFFA),
        (1, 0xFFFF_FFF8),
        (4, 0xFFFF_FFF6),
        (5, 0xFFFF_FFF4),
        (8, 0xFFFF_FFF2),
        (9, 0xFFFF_FFF0),
        (12, 2),
        (13, 0xFFFF_FFFE),
    ] {
        assert_eq!(za_words(&cpu, n), [want; 4], "vector {n}");
    }

    cpu.x[10] = 17;
    set_z(&mut cpu, 0, Esize::H, &[0xFFFF, 2]);
    set_z(&mut cpu, 1, Esize::H, &[0xFFFF, 3]);
    fill_za(&mut cpu, 0);
    assert_eq!(exec(&mut cpu, UMLSL_1), None);
    assert_eq!(changed(&cpu, 0), [6, 7]);
    assert_eq!(za_words(&cpu, 6), [0x0001_FFFF; 4]);
    assert_eq!(za_words(&cpu, 7), [0xFFFF_FFFA; 4]);
}

/// The dot products add four byte products, each operand signed or
/// unsigned by the instruction (-1 or 255 by -128 or 128), or two halfword
/// products, (-32768)^2 twice wrapping to 2^31; a group by a group takes
/// register `r` by `Zm + r`.
#[test]
fn the_dot_products_sum_bytes_or_halfwords_by_each_operand_s_sign() {
    let mut cpu = with_za(0);
    cpu.x[8] = 0;
    set_z(&mut cpu, 0, Esize::B, &[0xFF]);
    set_z(&mut cpu, 1, Esize::B, &[1, 2, 3, 4]);
    set_z(&mut cpu, 2, Esize::B, &[0x80]);
    for (insn, first, second) in [
        (SDOT_2X1_B, 0x201, 0xFFFF_FB01),
        (UDOT_2X1_B, 0x1_FE01, 0x501),
        (USDOT_2X1, 0xFFFE_0201, 0xFFFF_FB01),
        (SUDOT_2X1, 0xFFFF_FE01, 0x501),
    ] {
        fill_za(&mut cpu, 1);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(changed(&cpu, 1), [0, 8], "{insn:#010x}");
        assert_eq!(za_words(&cpu, 0), [first; 4], "{insn:#010x}");
        assert_eq!(za_words(&cpu, 8), [second; 4], "{insn:#010x}");
    }

    set_z(&mut cpu, 0, Esize::H, &[0x8000]);
    set_z(&mut cpu, 1, Esize::H, &[1, 2]);
    set_z(&mut cpu, 2, Esize::H, &[0x8000]);
    fill_za(&mut cpu, 1);
    assert_eq!(exec(&mut cpu, SDOT_2X1_H), None);
    assert_eq!(za_words(&cpu, 0), [0x8000_0001; 4]);
    assert_eq!(za_words(&cpu, 8), [0xFFFE_8001; 4]);

    for r in 0..4 {
        set_z(&mut cpu, 4 + r, Esize::H, &[0xFFFF]);
        set_z(&mut cpu, 8 + r, Esize::H, &[u64::from(r) + 1]);
    }
    fill_za(&mut cpu, 1);
    assert_eq!(exec(&mut cpu, UDOT_4X4_H), None);
    assert_eq!(changed(&cpu, 1), [0, 4, 8, 12]);
    for (n, want) in [(0, 0x1_FFFF), (4, 0x3_FFFD), (8, 0x5_FFFB), (12, 0x7_FFF9)] {
        assert_eq!(za_words(&cpu, n), [want; 4], "vector {n}");
    }
}

/// FDOT is `FPDotAdd_ZA`: the pair's products summed with one rounding,
/// then added with a second, so `1 + 2^-28` is 1 before -1 is added, and
/// no exception reaches FPSR; BFDOT adds the products of the BFloat16
/// pairs.
#[test]
fn fdot_rounds_the_pair_before_adding_it() {
    let mut cpu = with_za(0);
    cpu.x[8] = 0;
    // 1.0 and 2^-14 by 1.0 and 2^-14, and 1.0 and 2.0 by them.
    set_z(&mut cpu, 0, Esize::H, &[0x3C00, 0x0400]);
    set_z(&mut cpu, 1, Esize::H, &[0x3C00, 0x4000]);
    set_z(&mut cpu, 2, Esize::H, &[0x3C00, 0x0400]);
    fill_za(&mut cpu, MINUS_ONE);
    assert_eq!(exec(&mut cpu, FDOT_2X1), None);
    assert_eq!(za_words(&cpu, 0), [ZERO; 4]);
    // 1 + 2^-13, less 1.
    assert_eq!(za_words(&cpu, 8), [0x3900_0000; 4]);
    assert_eq!(cpu.fpsr, 0);

    // 1.0 and 2.0 by 3.0 and 4.0, and 1.0 and 1.0 by them.
    set_z(&mut cpu, 0, Esize::H, &[0x3F80, 0x4000]);
    set_z(&mut cpu, 1, Esize::H, &[0x3F80, 0x3F80]);
    set_z(&mut cpu, 2, Esize::H, &[0x4040, 0x4080]);
    fill_za(&mut cpu, HALF);
    assert_eq!(exec(&mut cpu, BFDOT_2X1), None);
    assert_eq!(changed(&cpu, HALF), [0, 8]);
    assert_eq!(za_words(&cpu, 0), [ELEVEN_AND_A_HALF; 4]);
    assert_eq!(za_words(&cpu, 8), [SEVEN_AND_A_HALF; 4]);
}

/// The widening products write two vectors, word `e` of vector `i` from
/// the 16-bit elements `2 * e + i`: 1.0 * 3.0 and +0 * 1.0 added to -0.
/// FMLSL and BFMLSL negate the register's element, so `-0 + -(+0 * 1)` is
/// -0 where FMLAL's and BFMLAL's sum is +0.
#[test]
fn the_widening_products_pair_their_elements_and_negate_for_subtraction() {
    let mut cpu = with_za(0);
    cpu.x[11] = 0;
    for (insn, n, m, first, second) in [
        (FMLAL_1, [0x3C00, 0x0000], [0x4200, 0x3C00], THREE, ZERO),
        (FMLSL_1, [0x3C00, 0x0000], [0x4200, 0x3C00], MINUS_THREE, MINUS_ZERO),
        (BFMLAL_1, [0x3F80, 0x0000], [0x4040, 0x3F80], THREE, ZERO),
        (BFMLSL_1, [0x3F80, 0x0000], [0x4040, 0x3F80], MINUS_THREE, MINUS_ZERO),
    ] {
        set_z(&mut cpu, 0, Esize::H, &n);
        set_z(&mut cpu, 1, Esize::H, &m);
        fill_za(&mut cpu, MINUS_ZERO);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(za_words(&cpu, 0), [first; 4], "{insn:#010x}");
        assert_eq!(za_words(&cpu, 1), [second; 4], "{insn:#010x}");
        assert!(changed(&cpu, MINUS_ZERO).iter().all(|&v| v < 2), "{insn:#010x}");
    }
}

/// ADD and SUB of registers and Zm replace the selected vectors, whatever
/// they held, modulo 2^32: vectors 3 and 11 (`3 MOD 8`) of two registers,
/// 3, 7, 11 and 15 (`3 MOD 4`) of four.
#[test]
fn add_and_sub_of_vectors_replace_the_za_vectors() {
    let mut cpu = with_za(0);
    cpu.x[8] = 0;
    fill_za(&mut cpu, 1000);
    set_z(&mut cpu, 0, Esize::S, &[0xFFFF_FFFF]);
    set_z(&mut cpu, 1, Esize::S, &[5]);
    set_z(&mut cpu, 2, Esize::S, &[2]);
    assert_eq!(exec(&mut cpu, ADD_ZZV_2X1), None);
    assert_eq!(changed(&cpu, 1000), [3, 11]);
    assert_eq!(za_words(&cpu, 3), [1; 4]);
    assert_eq!(za_words(&cpu, 11), [7; 4]);

    fill_za(&mut cpu, 1000);
    for r in 0..4 {
        set_z(&mut cpu, 4 + r, Esize::S, &[u64::from(r)]);
        set_z(&mut cpu, 8 + r, Esize::S, &[1]);
    }
    assert_eq!(exec(&mut cpu, SUB_ZZW_4X4), None);
    assert_eq!(changed(&cpu, 1000), [3, 7, 11, 15]);
    for (n, want) in [(3, 0xFFFF_FFFF), (7, 0), (11, 1), (15, 2)] {
        assert_eq!(za_words(&cpu, n), [want; 4], "vector {n}");
    }
}

/// ADD, SUB, FADD and FSUB of a group accumulate into the selected
/// vectors, register `r` into stride `r`: the integer sums wrap, the
/// floating-point ones are `FPAdd_ZA` and `FPSub_ZA`, a signalling NaN
/// giving the default NaN with no exception reaching FPSR.
#[test]
fn the_group_accumulates_into_the_za_vectors() {
    let mut cpu = with_za(0);
    cpu.x[8] = 0;
    fill_za(&mut cpu, 0xFFFF_FFFF);
    set_z(&mut cpu, 2, Esize::S, &[5]);
    set_z(&mut cpu, 3, Esize::S, &[1]);
    assert_eq!(exec(&mut cpu, ADD_ZW_2X2), None);
    assert_eq!(changed(&cpu, 0xFFFF_FFFF), [3, 11]);
    assert_eq!(za_words(&cpu, 3), [4; 4]);
    assert_eq!(za_words(&cpu, 11), [0; 4]);

    fill_za(&mut cpu, ONE);
    set_za(&mut cpu, 3, &[SNAN, ONE, ONE, ONE]);
    set_z(&mut cpu, 2, Esize::S, &[TWO]);
    set_z(&mut cpu, 3, Esize::S, &[HALF]);
    assert_eq!(exec(&mut cpu, FADD_ZW_2X2), None);
    assert_eq!(changed(&cpu, ONE), [3, 11]);
    assert_eq!(za_words(&cpu, 3), [DEFAULT_NAN, THREE, THREE, THREE]);
    assert_eq!(za_words(&cpu, 11), [ONE_AND_A_HALF; 4]);
    assert_eq!(cpu.fpsr, 0);

    for (r, value) in [ONE, TWO, THREE, FOUR].into_iter().enumerate() {
        set_z(&mut cpu, 4 + r as u32, Esize::S, &[value]);
    }
    fill_za(&mut cpu, FOUR);
    assert_eq!(exec(&mut cpu, FSUB_ZW_4X4), None);
    for (n, want) in [(3, THREE), (7, TWO), (11, ONE), (15, ZERO)] {
        assert_eq!(za_words(&cpu, n), [want; 4], "vector {n}");
    }
    for r in 0..4 {
        set_z(&mut cpu, 4 + r, Esize::S, &[u64::from(r) + 1]);
    }
    fill_za(&mut cpu, 10);
    assert_eq!(exec(&mut cpu, SUB_ZW_4X4), None);
    assert_eq!(changed(&cpu, 10), [3, 7, 11, 15]);
    for (n, want) in [(3, 9), (7, 8), (11, 7), (15, 6)] {
        assert_eq!(za_words(&cpu, n), [want; 4], "vector {n}");
    }
}

/// SMLALL, SMLAL, FMLAL and FMLA by an indexed element take element
/// `index` of the 128-bit segment holding each result word: at 256 bits
/// the first four words from the first segment's, the last four from the
/// second's. SMLALL by byte 13 (`i4h:i4l`, bits 15 and 12:10) at offset
/// `off2 * 4`, of bytes 1 to 4 by 3 and -2 into vectors 4 to 7; SMLAL by
/// halfword 5 (`i3h:i3l`, bits 15 and 11:10) at offset `off3 * 2`; FMLAL
/// by element 7 into vectors 0 and 1; FMLA of two registers by word 3
/// (`i2`, bits 11:10) into vectors 1 and 17.
#[test]
fn the_indexed_element_comes_from_each_result_s_own_segment() {
    let mut cpu = with_za(1);
    cpu.x[8] = 0;
    set_z(&mut cpu, 0, Esize::B, &[1, 2, 3, 4]);
    set_z_with(&mut cpu, 1, Esize::B, |b| match b {
        13 => 3,
        29 => 0xFE,
        _ => 0x7F,
    });
    fill_za(&mut cpu, 0);
    assert_eq!(exec(&mut cpu, SMLALL_IDX_1), None);
    assert_eq!(changed(&cpu, 0), [4, 5, 6, 7]);
    for i in 0..4 {
        let byte = i as i64 + 1;
        let (first, second) = ((3 * byte) as u64, (-2 * byte) as u32 as u64);
        assert_eq!(za_words(&cpu, 4 + i), [vec![first; 4], vec![second; 4]].concat(), "vector {}", 4 + i);
    }

    cpu.x[9] = 0;
    set_z(&mut cpu, 0, Esize::H, &[1, 2]);
    set_z_with(&mut cpu, 1, Esize::H, |h| match h {
        5 => 7,
        13 => 0xFFFF,
        _ => 0x7FFF,
    });
    fill_za(&mut cpu, 0);
    assert_eq!(exec(&mut cpu, SMLAL_IDX_1), None);
    assert_eq!(changed(&cpu, 0), [2, 3]);
    assert_eq!(za_words(&cpu, 2), [vec![7; 4], vec![0xFFFF_FFFF; 4]].concat());
    assert_eq!(za_words(&cpu, 3), [vec![14; 4], vec![0xFFFF_FFFE; 4]].concat());

    // 1.0 and 2.0 by 3.0 and by 0.5, added to 1.0.
    cpu.x[10] = 0;
    set_z(&mut cpu, 0, Esize::H, &[0x3C00, 0x4000]);
    set_z_with(&mut cpu, 1, Esize::H, |h| match h {
        7 => 0x4200,
        15 => 0x3800,
        _ => 0x7C00,
    });
    fill_za(&mut cpu, ONE);
    assert_eq!(exec(&mut cpu, FMLAL_IDX_1), None);
    assert_eq!(changed(&cpu, ONE), [0, 1]);
    assert_eq!(za_words(&cpu, 0), [vec![FOUR; 4], vec![ONE_AND_A_HALF; 4]].concat());
    assert_eq!(za_words(&cpu, 1), [vec![SEVEN; 4], vec![TWO; 4]].concat());

    // 1.0 and 2.0 by 3.0 and by 0.5, added to 0.5; `vstride` 16.
    cpu.x[11] = 0;
    set_z(&mut cpu, 2, Esize::S, &[ONE]);
    set_z(&mut cpu, 3, Esize::S, &[TWO]);
    set_z_with(&mut cpu, 1, Esize::S, |s| match s {
        3 => THREE,
        7 => HALF,
        _ => INF,
    });
    fill_za(&mut cpu, HALF);
    assert_eq!(exec(&mut cpu, FMLA_IDX_2), None);
    assert_eq!(changed(&cpu, HALF), [1, 17]);
    assert_eq!(za_words(&cpu, 1), [vec![THREE_AND_A_HALF; 4], vec![ONE; 4]].concat());
    assert_eq!(za_words(&cpu, 17), [vec![SIX_AND_A_HALF; 4], vec![ONE_AND_A_HALF; 4]].concat());
}

/// Of a group by an indexed element, the four-way products' mixed signs
/// are `op` (bit 5) and the index `i4h:i4l` bits 11:10 and 2:1: USMLALL of
/// 255 and 2 by byte 6, -128, into vectors 0 to 3 and 8 to 11; SUMLALL of
/// -1, 1, -128 and 127 by byte 9, 255, into all sixteen. The two-way ones'
/// index is `i3h:i3l`, bits 11:10 and 2: UMLSL of four registers by
/// halfword 3, 65535. BFMLSL of two by element 2 and FMLS of four by word
/// 1 subtract their products.
#[test]
fn the_indexed_products_of_groups() {
    let mut cpu = with_za(0);
    cpu.x[8] = 0;
    set_z(&mut cpu, 2, Esize::B, &[0xFF]);
    set_z(&mut cpu, 3, Esize::B, &[0x02]);
    set_z_with(&mut cpu, 1, Esize::B, |b| if b == 6 { 0x80 } else { 1 });
    fill_za(&mut cpu, 0);
    assert_eq!(exec(&mut cpu, USMLALL_IDX_2), None);
    assert_eq!(changed(&cpu, 0), [0, 1, 2, 3, 8, 9, 10, 11]);
    for i in 0..4 {
        assert_eq!(za_words(&cpu, i), [0xFFFF_8080; 4]);
        assert_eq!(za_words(&cpu, 8 + i), [0xFFFF_FF00; 4]);
    }

    for (r, value) in [0xFF, 0x01, 0x80, 0x7F].into_iter().enumerate() {
        set_z(&mut cpu, 4 + r as u32, Esize::B, &[value]);
    }
    set_z_with(&mut cpu, 1, Esize::B, |b| if b == 9 { 0xFF } else { 0 });
    fill_za(&mut cpu, 0);
    assert_eq!(exec(&mut cpu, SUMLALL_IDX_4), None);
    for (r, want) in [0xFFFF_FF01, 0xFF, 0xFFFF_8080, 0x7E81].into_iter().enumerate() {
        for i in 0..4 {
            assert_eq!(za_words(&cpu, 4 * r + i), [want; 4], "register {r}");
        }
    }

    cpu.x[9] = 0;
    for r in 0..4 {
        set_z(&mut cpu, 4 + r, Esize::H, &[u64::from(r) + 1]);
    }
    set_z_with(&mut cpu, 1, Esize::H, |h| if h == 3 { 0xFFFF } else { 0 });
    fill_za(&mut cpu, 0);
    assert_eq!(exec(&mut cpu, UMLSL_IDX_4), None);
    assert_eq!(changed(&cpu, 0), [0, 1, 4, 5, 8, 9, 12, 13]);
    for (r, want) in [0xFFFF_0001, 0xFFFE_0002, 0xFFFD_0003, 0xFFFC_0004].into_iter().enumerate() {
        assert_eq!(za_words(&cpu, 4 * r), [want; 4], "register {r}");
        assert_eq!(za_words(&cpu, 4 * r + 1), [want; 4], "register {r}");
    }

    // 1.0 - 1.0 * 0.5 and 1.0 - 2.0 * 0.5, in BFloat16.
    cpu.x[10] = 0;
    set_z(&mut cpu, 2, Esize::H, &[0x3F80]);
    set_z(&mut cpu, 3, Esize::H, &[0x4000]);
    set_z_with(&mut cpu, 1, Esize::H, |h| if h == 2 { 0x3F00 } else { 0x7F80 });
    fill_za(&mut cpu, ONE);
    assert_eq!(exec(&mut cpu, BFMLSL_IDX_2), None);
    assert_eq!(changed(&cpu, ONE), [0, 1, 8, 9]);
    for n in [0, 1] {
        assert_eq!(za_words(&cpu, n), [HALF; 4]);
        assert_eq!(za_words(&cpu, 8 + n), [ZERO; 4]);
    }

    // 1.0 - (r + 1) * 2.0 into vectors 1, 5, 9 and 13.
    cpu.x[11] = 0;
    for (r, value) in [ONE, TWO, THREE, FOUR].into_iter().enumerate() {
        set_z(&mut cpu, 4 + r as u32, Esize::S, &[value]);
    }
    set_z_with(&mut cpu, 1, Esize::S, |s| if s == 1 { TWO } else { INF });
    fill_za(&mut cpu, ONE);
    assert_eq!(exec(&mut cpu, FMLS_IDX_4), None);
    assert_eq!(changed(&cpu, ONE), [1, 5, 9, 13]);
    for (n, want) in [(1, MINUS_ONE), (5, MINUS_THREE), (9, MINUS_FIVE), (13, MINUS_SEVEN)] {
        assert_eq!(za_words(&cpu, n), [want; 4], "vector {n}");
    }
}

/// The indexed dot products take group `index` of the segment, element
/// `k` of it with element `k` of each word's group: at 256 bits SDOT by
/// group 2 counts its first byte, 1, in the first segment and its last,
/// -128, in the second; USDOT of four registers by group 1; UDOT of
/// halfwords by group 3; FDOT by pair 1.
#[test]
fn the_indexed_dot_products_take_a_group_of_the_segment() {
    let mut cpu = with_za(1);
    cpu.x[8] = 0;
    set_z(&mut cpu, 2, Esize::B, &[1, 2, 3, 4]);
    set_z(&mut cpu, 3, Esize::B, &[0xFF]);
    set_z_with(&mut cpu, 1, Esize::B, |b| match b {
        8 => 1,
        9..=11 | 24..=26 => 0,
        27 => 0x80,
        _ => 0x7F,
    });
    fill_za(&mut cpu, 0);
    assert_eq!(exec(&mut cpu, SDOT_IDX_2), None);
    assert_eq!(changed(&cpu, 0), [0, 16]);
    assert_eq!(za_words(&cpu, 0), [vec![1; 4], vec![0xFFFF_FE00; 4]].concat());
    assert_eq!(za_words(&cpu, 16), [vec![0xFFFF_FFFF; 4], vec![128; 4]].concat());

    let mut cpu = with_za(0);
    cpu.x[8] = 0;
    for (r, value) in [0xFF, 0x01, 0x80, 0x02].into_iter().enumerate() {
        set_z(&mut cpu, 4 + r as u32, Esize::B, &[value]);
    }
    set_z_with(&mut cpu, 1, Esize::B, |b| if b / 4 == 1 { 0x80 } else { 0x7F });
    fill_za(&mut cpu, 0);
    assert_eq!(exec(&mut cpu, USDOT_IDX_4), None);
    assert_eq!(changed(&cpu, 0), [0, 4, 8, 12]);
    for (r, want) in [0xFFFE_0200, 0xFFFF_FE00, 0xFFFF_0000, 0xFFFF_FC00].into_iter().enumerate() {
        assert_eq!(za_words(&cpu, 4 * r), [want; 4], "register {r}");
    }

    set_z(&mut cpu, 2, Esize::H, &[0xFFFF]);
    set_z(&mut cpu, 3, Esize::H, &[1, 2]);
    set_z_with(&mut cpu, 1, Esize::H, |h| match h {
        6 => 2,
        7 => 3,
        _ => 0x7FFF,
    });
    fill_za(&mut cpu, 0);
    assert_eq!(exec(&mut cpu, UDOT_IDX_2_H), None);
    assert_eq!(changed(&cpu, 0), [0, 8]);
    assert_eq!(za_words(&cpu, 0), [0x4_FFFB; 4]);
    assert_eq!(za_words(&cpu, 8), [8; 4]);

    // 0.5 + 1.0 * 3.0 + 2.0 * 4.0, and 0.5 + 0.5 * 3.0 + 0.5 * 4.0.
    set_z(&mut cpu, 2, Esize::H, &[0x3C00, 0x4000]);
    set_z(&mut cpu, 3, Esize::H, &[0x3800]);
    set_z_with(&mut cpu, 1, Esize::H, |h| match h {
        2 => 0x4200,
        3 => 0x4400,
        _ => 0x7C00,
    });
    fill_za(&mut cpu, HALF);
    assert_eq!(exec(&mut cpu, FDOT_IDX_2), None);
    assert_eq!(changed(&cpu, HALF), [0, 8]);
    assert_eq!(za_words(&cpu, 0), [ELEVEN_AND_A_HALF; 4]);
    assert_eq!(za_words(&cpu, 8), [FOUR; 4]);
}

/// The vertical dot products sum, for register `r`'s word `e`, element
/// `nreg * e + r` of each register `k` of the group times element `k` of
/// the indexed group: FVDOT and BFVDOT of 1.0 and 3.0 (`r = 0`) and of 2.0
/// and 4.0 (`r = 1`) by 0.5 and 2.0; SVDOT and UVDOT of two registers of
/// halfwords; SVDOT, USVDOT, UVDOT and SUVDOT of four of bytes, register
/// `k`'s byte `4 * e + r` being `r + 1`, `-(r + 1)`, 2 and -128 in turn,
/// by 1, 2, 3 and -1.
#[test]
fn the_vertical_dot_products_sum_across_the_registers() {
    let mut cpu = with_za(0);
    cpu.x[8] = 0;
    for (insn, z2, z3, pair, inf) in [
        (FVDOT_2, [0x3C00, 0x4000], [0x4200, 0x4400], [(2, 0x3800), (3, 0x4000)], 0x7C00),
        (BFVDOT_2, [0x3F80, 0x4000], [0x4040, 0x4080], [(6, 0x3F00), (7, 0x4000)], 0x7F80),
    ] {
        set_z(&mut cpu, 2, Esize::H, &z2);
        set_z(&mut cpu, 3, Esize::H, &z3);
        set_z_with(&mut cpu, 1, Esize::H, |h| pair.iter().find(|p| p.0 == h).map_or(inf, |p| p.1));
        fill_za(&mut cpu, ZERO);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(changed(&cpu, ZERO), [0, 8], "{insn:#010x}");
        assert_eq!(za_words(&cpu, 0), [SIX_AND_A_HALF; 4], "{insn:#010x}");
        assert_eq!(za_words(&cpu, 8), [NINE; 4], "{insn:#010x}");
    }

    // FVDOT's sum is FPDotAdd_ZA's: a signalling NaN in `r = 0`'s pair gives
    // the default NaN whatever FPCR.DN, and no exception reaches FPSR.
    set_z(&mut cpu, 2, Esize::H, &[0x7C01, 0x4000]);
    set_z(&mut cpu, 3, Esize::H, &[0x4200, 0x4400]);
    set_z_with(&mut cpu, 1, Esize::H, |h| match h {
        2 => 0x3800,
        3 => 0x4000,
        _ => 0x7C00,
    });
    fill_za(&mut cpu, ZERO);
    assert_eq!(exec(&mut cpu, FVDOT_2), None);
    assert_eq!(za_words(&cpu, 0), [DEFAULT_NAN; 4]);
    assert_eq!(za_words(&cpu, 8), [NINE; 4]);
    assert_eq!(cpu.fpsr, 0);

    // -32768 and 2 (`r = 0`), 1 and -1 (`r = 1`), by 3 and -2.
    set_z(&mut cpu, 2, Esize::H, &[0x8000, 1]);
    set_z(&mut cpu, 3, Esize::H, &[2, 0xFFFF]);
    for (insn, first, want) in [(SVDOT_2_H, 4, [0xFFFE_7FFC, 5]), (UVDOT_2_H, 0, [0x3_7FFC, 0xFFFD_0005])] {
        set_z_with(&mut cpu, 1, Esize::H, |h| match h.checked_sub(first) {
            Some(0) => 3,
            Some(1) => 0xFFFE,
            _ => 0x7FFF,
        });
        fill_za(&mut cpu, 0);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(changed(&cpu, 0), [0, 8], "{insn:#010x}");
        assert_eq!(za_words(&cpu, 0), [want[0]; 4], "{insn:#010x}");
        assert_eq!(za_words(&cpu, 8), [want[1]; 4], "{insn:#010x}");
    }

    set_z_with(&mut cpu, 4, Esize::B, |b| (b % 4) as u64 + 1);
    set_z_with(&mut cpu, 5, Esize::B, |b| (-((b % 4) as i64 + 1)) as u8 as u64);
    set_z(&mut cpu, 6, Esize::B, &[2]);
    set_z(&mut cpu, 7, Esize::B, &[0x80]);
    for (insn, group, want) in [
        (SVDOT_4, 1, [133, 132, 131, 130]),
        (USVDOT_4, 3, [389, 388, 387, 386]),
        (UVDOT_4, 0, [33157, 33156, 33155, 33154]),
        (SUVDOT_4, 2, [0xFFFF_8085, 0xFFFF_8084, 0xFFFF_8083, 0xFFFF_8082]),
    ] {
        set_z_with(&mut cpu, 1, Esize::B, |b| if b / 4 == group { [1, 2, 3, 0xFF][b % 4] } else { 0x7F });
        fill_za(&mut cpu, 0);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(changed(&cpu, 0), [0, 4, 8, 12], "{insn:#010x}");
        for (r, want) in want.into_iter().enumerate() {
            assert_eq!(za_words(&cpu, 4 * r), [want; 4], "{insn:#010x} register {r}");
        }
    }
}

/// The 64-bit forms of FEAT_SME_I16I64 and FEAT_SME_F64F64, by vectors and
/// by an indexed element, and the indexed forms of FP8 (FEAT_SME_F8F32) and
/// of half-precision values (FEAT_SME_F16F16), are UNDEFINED in the decode,
/// before the enable checks.
#[test]
fn the_forms_of_other_features_are_undefined() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for insn in [
        SMLALL_D,
        FMLA_D,
        ADD_D,
        FADD_D,
        SDOT_D,
        SMLALL_IDX_D,
        SDOT_IDX_D,
        FMLA_IDX_D,
        FDOT_IDX_FP8,
        FMLA_IDX_H,
    ] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}

/// MOVA between a group and ZA array vectors moves register `r` to or from
/// vector `(UInt(Wv) + offset) MOD vstride` of stride `r`, its offset
/// unscaled, and touches no other vector.
#[test]
fn mova_moves_a_group_to_and_from_the_selected_vector_of_each_stride() {
    let mut cpu = with_za(0);
    fill_za(&mut cpu, 0xEE);
    set_z(&mut cpu, 4, Esize::S, &[1]);
    set_z(&mut cpu, 5, Esize::S, &[2]);
    // Two registers at 128 bits: strides of 8, and (6 + 3) MOD 8 is vector
    // 1; back from (0 + 1) MOD 8.
    cpu.x[8] = 6;
    assert_eq!(exec(&mut cpu, MOVA_TO_ARRAY_2), None);
    assert_eq!(changed(&cpu, 0xEE), [1, 9]);
    assert_eq!(za_words(&cpu, 1), [1; 4]);
    assert_eq!(za_words(&cpu, 9), [2; 4]);
    set_z(&mut cpu, 4, Esize::S, &[0]);
    set_z(&mut cpu, 5, Esize::S, &[0]);
    assert_eq!(exec(&mut cpu, MOVA_FROM_ARRAY_2), None);
    assert_eq!(z_words(&cpu, 4), [1; 4]);
    assert_eq!(z_words(&cpu, 5), [2; 4]);

    // Four registers at 256 bits: strides of 8, and (1 + 7) MOD 8 is
    // vector 0 from the low word of X9; back from (4 + 5) MOD 8, vector 1.
    let mut cpu = with_za(1);
    fill_za(&mut cpu, 0xEE);
    for r in 0..4 {
        set_z(&mut cpu, 8 + r, Esize::S, &[10 + u64::from(r)]);
    }
    cpu.x[9] = 0xFFFF_FFFF_0000_0001;
    assert_eq!(exec(&mut cpu, MOVA_TO_ARRAY_4), None);
    assert_eq!(changed(&cpu, 0xEE), [0, 8, 16, 24]);
    for r in 0..4 {
        assert_eq!(za_words(&cpu, 8 * r), [10 + r as u64; 8], "ZA[{}]", 8 * r);
        set_za(&mut cpu, 8 * r + 1, &[20 + r as u64]);
    }
    cpu.x[11] = 4;
    assert_eq!(exec(&mut cpu, MOVA_FROM_ARRAY_4), None);
    for r in 0..4 {
        assert_eq!(z_words(&cpu, 12 + r), [20 + u64::from(r); 8], "Z{}", 12 + r);
    }
}

/// Every one of them takes the SME trap outside Streaming SVE mode, then
/// with ZA storage disabled, and is UNDEFINED without FEAT_SME2.
#[test]
fn they_need_streaming_mode_za_and_sme2() {
    let all = [
        FMLA_2X1,
        FMLA_4X4,
        SMLALL_1,
        SUMLALL_2X1,
        SMLAL_4X1_Z30,
        UMLSL_1,
        BFMLSL_1,
        SUDOT_2X1,
        UDOT_4X4_H,
        FDOT_2X1,
        BFDOT_4X4,
        ADD_ZZV_2X1,
        SUB_ZZW_4X4,
        ADD_ZW_2X2,
        FSUB_ZW_4X4,
        SMLALL_IDX_1,
        USMLALL_IDX_2,
        UMLSL_IDX_4,
        FMLAL_IDX_1,
        FMLA_IDX_2,
        SDOT_IDX_2,
        FDOT_IDX_2,
        FVDOT_2,
        SVDOT_4,
        MOVA_TO_ARRAY_2,
        MOVA_TO_ARRAY_4,
        MOVA_FROM_ARRAY_2,
        MOVA_FROM_ARRAY_4,
    ];
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for insn in all {
        assert_eq!(exec(&mut cpu, insn), Some(NOT_STREAMING), "{insn:#010x}");
    }
    cpu.set_pstate_sm(true);
    for insn in all {
        assert_eq!(exec(&mut cpu, insn), Some(INACTIVE_ZA), "{insn:#010x}");
    }
    let features: Vec<Feature> = Feature::ALL.into_iter().filter(|&f| f != Feature::Sme2).collect();
    let mut cpu = processor(Implementation {
        features: Features::of(&features),
        max_svl: VecLen::MAX,
        ..Implementation::EVERY_FEATURE
    });
    cpu.set_pstate_sm(true);
    cpu.set_pstate_za(true);
    for insn in all {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}
