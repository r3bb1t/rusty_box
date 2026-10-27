//! FEAT_SME2's multi-vector SVE operations, on a processor that implements
//! FEAT_SME2 ([`Implementation::EVERY_FEATURE`]) in Streaming SVE mode.
//!
//! Each property is the guest-visible one (R9), each expected value worked
//! from the instruction's Arm pseudocode: SEL's elements by the counter
//! across the registers; every source read before a destination is
//! written; SQDMULH's saturation and its exact 64-bit product; the
//! rounding shifts' clamped shift and wrapped result; the maxima and
//! minima signed and unsigned, by one vector and by a group; the
//! floating-point maxima and minima on NaNs and zeros; the clamps' order;
//! ZIP and UZP of two and four vectors, of quadwords, and the vector
//! lengths they need, in the decode and after the enable check; the
//! narrowing shifts' rounding, saturation, concatenation and interleaving,
//! and the reserved `tsize`; the saturating narrows; BFCVT and FCVT, the
//! latter in IEEE half precision under FPCR.AHP; the word conversions'
//! saturation and rounding; FRINT's own rounding with no inexact; the
//! unpacks and their reserved size. Of the SVE2.1 instructions FEAT_SME2
//! makes legal: the narrows of two vectors' interleaving, rounding and
//! saturation, and FCLAMP of one vector at each size, with BFCLAMP's size
//! UNDEFINED. The checks: Streaming SVE mode, and UNDEFINED without
//! FEAT_SME2.

use std::vec;
use std::vec::Vec;

use super::super::super::features::{Feature, Features, Implementation, VecLen};
use super::super::super::fp_estimate::fpsr::{IOC, IXC, OFC};
use super::super::super::step::{exec, processor, Taken, UNDEFINED};
use super::super::super::{Cpu, NoEl2};
use super::super::{Esize, Pv, Zv};

/// The SME trap with SMTC 2: outside Streaming SVE mode.
const NOT_STREAMING: Taken = Taken { ec: 0x1D, iss: 2 };

/// FPCR.AHP, and FPCR.RMode's round toward plus infinity and toward zero.
const FPCR_AHP: u64 = 1 << 26;
const FPCR_RP: u64 = 0b01 << 22;
const FPCR_RZ: u64 = 0b11 << 22;

/// `sel {z0.b-z1.b}, pn8, {z2.b-z3.b}, {z4.b-z5.b}`.
const SEL_2_B: u32 = 0xC124_8040;
/// `sel {z4.h-z7.h}, pn9, {z8.h-z11.h}, {z12.h-z15.h}`.
const SEL_4_H: u32 = 0xC16D_8504;
/// `add {z0.h-z1.h}, {z0.h-z1.h}, z0.h`.
const ADD_2X1_H_Z0: u32 = 0xC160_A300;
/// `sqdmulh {z0.b-z1.b}, {z0.b-z1.b}, z2.b`.
const SQDMULH_2X1_B: u32 = 0xC122_A400;
/// `sqdmulh {z0.d-z3.d}, {z0.d-z3.d}, {z4.d-z7.d}`.
const SQDMULH_4X4_D: u32 = 0xC1E4_BC00;
/// `srshl {z0.b-z1.b}, {z0.b-z1.b}, z2.b` and `srshl {z0.h-z1.h},
/// {z0.h-z1.h}, z2.h`.
const SRSHL_2X1_B: u32 = 0xC122_A220;
const SRSHL_2X1_H: u32 = 0xC162_A220;
/// `urshl {z0.b-z1.b}, {z0.b-z1.b}, {z2.b-z3.b}`.
const URSHL_2X2_B: u32 = 0xC122_B221;
/// `smax {z0.s-z3.s}, {z0.s-z3.s}, {z4.s-z7.s}`.
const SMAX_4X4_S: u32 = 0xC1A4_B800;
/// `umin {z0.s-z3.s}, {z0.s-z3.s}, {z4.s-z7.s}`.
const UMIN_4X4_S: u32 = 0xC1A4_B821;
/// `smin {z0.s-z3.s}, {z0.s-z3.s}, z4.s`.
const SMIN_4X1_S: u32 = 0xC1A4_A820;
/// `umax {z0.s-z1.s}, {z0.s-z1.s}, z4.s`.
const UMAX_2X1_S: u32 = 0xC1A4_A001;
/// `fmax {z0.s-z1.s}, {z0.s-z1.s}, z2.s`.
const FMAX_2X1_S: u32 = 0xC1A2_A100;
/// `fmaxnm {z0.s-z1.s}, {z0.s-z1.s}, {z2.s-z3.s}`.
const FMAXNM_2X2_S: u32 = 0xC1A2_B120;
/// `fmin {z0.d-z3.d}, {z0.d-z3.d}, {z4.d-z7.d}`.
const FMIN_4X4_D: u32 = 0xC1E4_B901;
/// `fminnm {z0.h-z3.h}, {z0.h-z3.h}, z4.h`.
const FMINNM_4X1_H: u32 = 0xC164_A921;
/// `fclamp {z0.s-z1.s}, z2.s, z3.s`.
const FCLAMP_2_S: u32 = 0xC1A3_C040;
/// SVE2.1's `fclamp z0.h, z1.h, z2.h`, `.s` and `.d`, and the word with
/// `size = 00`, BFCLAMP (FEAT_SVE_B16B16).
const FCLAMP_H: u32 = 0x6462_2420;
const FCLAMP_S: u32 = 0x64A2_2420;
const FCLAMP_D: u32 = 0x64E2_2420;
const BFCLAMP: u32 = 0x6422_2420;
/// SVE2.1's `sqrshrn z0.h, {z2.s-z3.s}, #16`, `sqrshrun ..., #1`,
/// `uqrshrn ..., #8`, `sqcvtn z0.h, {z2.s-z3.s}`, `sqcvtun` and `uqcvtn`.
const SQRSHRN_PAIR: u32 = 0x45B0_2840;
const SQRSHRUN_PAIR: u32 = 0x45BF_0840;
const UQRSHRN_PAIR: u32 = 0x45B8_3840;
const SQCVTN_PAIR: u32 = 0x4531_4040;
const SQCVTUN_PAIR: u32 = 0x4531_5040;
const UQCVTN_PAIR: u32 = 0x4531_4840;
/// `sclamp {z0.b-z3.b}, z4.b, z5.b` and `uclamp {z0.b-z3.b}, z4.b, z5.b`.
const SCLAMP_4_B: u32 = 0xC125_CC80;
const UCLAMP_4_B: u32 = 0xC125_CC81;
/// `zip {z0.b-z1.b}, z2.b, z3.b` and `uzp {z0.b-z1.b}, z2.b, z3.b`.
const ZIP_2_B: u32 = 0xC123_D040;
const UZP_2_B: u32 = 0xC123_D041;
/// `zip {z0.s-z3.s}, {z4.s-z7.s}` and `uzp {z0.s-z3.s}, {z4.s-z7.s}`.
const ZIP_4_S: u32 = 0xC1B6_E080;
const UZP_4_S: u32 = 0xC1B6_E082;
/// `zip {z0.q-z1.q}, z2.q, z3.q`.
const ZIP_2_Q: u32 = 0xC123_D440;
/// `uzp {z0.q-z3.q}, {z4.q-z7.q}`.
const UZP_4_Q: u32 = 0xC137_E082;
/// `zip {z0.d-z3.d}, {z4.d-z7.d}`.
const ZIP_4_D: u32 = 0xC1F6_E080;
/// `sqrshr z0.h, {z2.s-z3.s}, #16`, `sqrshru z0.h, {z2.s-z3.s}, #1` and
/// `uqrshr z0.h, {z2.s-z3.s}, #4`.
const SQRSHR_2_H: u32 = 0xC1E0_D440;
const SQRSHRU_2_H: u32 = 0xC1FF_D440;
const UQRSHR_2_H: u32 = 0xC1EC_D460;
/// `sqrshr z0.b, {z4.s-z7.s}, #8`.
const SQRSHR_4_B: u32 = 0xC178_D880;
/// `sqrshrn z0.h, {z4.d-z7.d}, #32`.
const SQRSHRN_4_H: u32 = 0xC1E0_DC80;
/// `uqrshrn z0.b, {z4.s-z7.s}, #1`.
const UQRSHRN_4_B: u32 = 0xC17F_DCA0;
/// `sqrshrun z0.h, {z4.d-z7.d}, #1`.
const SQRSHRUN_4_H: u32 = 0xC1FF_DCC0;
/// SQRSHR of four vectors with `tsize = 00`.
const QRSHR_4_TSIZE_00: u32 = 0xC120_DC80;
/// `sqcvt z0.h, {z2.s-z3.s}`, `sqcvtu z0.h, {z2.s-z3.s}` and `uqcvt z0.h,
/// {z2.s-z3.s}`.
const SQCVT_2_H: u32 = 0xC123_E040;
const SQCVTU_2_H: u32 = 0xC163_E040;
const UQCVT_2_H: u32 = 0xC123_E060;
/// `sqcvt z0.b, {z4.s-z7.s}`.
const SQCVT_4_B: u32 = 0xC133_E080;
/// `sqcvtn z0.h, {z4.d-z7.d}`.
const SQCVTN_4_H: u32 = 0xC1B3_E0C0;
/// `uqcvtn z0.b, {z4.s-z7.s}`.
const UQCVTN_4_B: u32 = 0xC133_E0E0;
/// `sqcvtun z0.h, {z4.d-z7.d}`.
const SQCVTUN_4_H: u32 = 0xC1F3_E0C0;
/// `bfcvt`, `fcvt`, `bfcvtn` and `fcvtn` `z0.h, {z2.s-z3.s}`.
const BFCVT_2: u32 = 0xC160_E040;
const FCVT_2: u32 = 0xC120_E040;
const BFCVTN_2: u32 = 0xC160_E060;
const FCVTN_2: u32 = 0xC120_E060;
/// `fcvtzs {z0.s-z1.s}, {z2.s-z3.s}` and `fcvtzu {z0.s-z3.s}, {z4.s-z7.s}`.
const FCVTZS_2: u32 = 0xC121_E040;
const FCVTZU_4: u32 = 0xC131_E0A0;
/// `scvtf {z0.s-z1.s}, {z2.s-z3.s}` and `ucvtf {z0.s-z3.s}, {z4.s-z7.s}`.
const SCVTF_2: u32 = 0xC122_E040;
const UCVTF_4: u32 = 0xC132_E0A0;
/// `frintn` and `frintp` `{z0.s-z1.s}, {z2.s-z3.s}`; `frintm` and `frinta`
/// `{z0.s-z3.s}, {z4.s-z7.s}`.
const FRINTN_2: u32 = 0xC1A8_E040;
const FRINTP_2: u32 = 0xC1A9_E040;
const FRINTM_4: u32 = 0xC1BA_E080;
const FRINTA_4: u32 = 0xC1BC_E080;
/// `sunpk {z0.h-z1.h}, z2.b`, `uunpk {z0.d-z3.d}, {z4.s-z5.s}` and `sunpk
/// {z0.s-z3.s}, {z4.h-z5.h}`.
const SUNPK_2_H: u32 = 0xC165_E040;
const UUNPK_4_D: u32 = 0xC1F5_E081;
const SUNPK_4_S: u32 = 0xC1B5_E080;
/// SUNPK of one vector with `size = 00`.
const UNPK_SIZE_00: u32 = 0xC125_E040;

/// Single-precision values.
const ONE: u64 = 0x3F80_0000;
const MINUS_ONE: u64 = 0xBF80_0000;
const TWO: u64 = 0x4000_0000;
const MINUS_TWO: u64 = 0xC000_0000;
const THREE: u64 = 0x4040_0000;
const MINUS_THREE: u64 = 0xC040_0000;
const FOUR: u64 = 0x4080_0000;
const FIVE: u64 = 0x40A0_0000;
const HALF: u64 = 0x3F00_0000;
const ONE_AND_A_HALF: u64 = 0x3FC0_0000;
const TWO_AND_A_HALF: u64 = 0x4020_0000;
const MINUS_TWO_AND_A_HALF: u64 = 0xC020_0000;
const THREE_AND_A_HALF: u64 = 0x4060_0000;
const POINT_FOUR: u64 = 0x3ECC_CCCD;
const MINUS_POINT_FOUR: u64 = 0xBECC_CCCD;
const ZERO: u64 = 0;
const MINUS_ZERO: u64 = 0x8000_0000;
const INF: u64 = 0x7F80_0000;
const MINUS_INF: u64 = 0xFF80_0000;
const QNAN: u64 = 0x7FC0_0000;
/// A signalling NaN, and the quiet NaN processing it gives.
const SNAN: u64 = 0x7F80_0001;
const SNAN_QUIETED: u64 = 0x7FC0_0001;
const TEN_BILLION: u64 = 0x5015_02F9;

/// A processor with FEAT_SME2 in Streaming SVE mode with `SMCR_EL1.LEN =
/// len`, FPCR `fpcr` and FPSR clear.
fn streaming(len: u64, fpcr: u64) -> Cpu<NoEl2> {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.sys.smcr_el1 = len;
    cpu.set_pstate_sm(true);
    cpu.fpcr = fpcr;
    cpu.fpsr = 0;
    cpu
}

/// Z register `n` holding `values` as elements of `es`, repeated from the
/// first element through the vector.
fn set_z(cpu: &mut Cpu<NoEl2>, n: u32, es: Esize, values: &[u64]) {
    let vl = cpu.current_svl();
    let mut z = Zv::ZERO;
    for e in 0..es.elements(vl) {
        z.set(e, es, values[e % values.len()]);
    }
    cpu.set_z(n, vl, &z);
}

/// The elements of `es` of Z register `n`.
fn z(cpu: &Cpu<NoEl2>, n: u32, es: Esize) -> Vec<u64> {
    let vl = cpu.current_svl();
    let z = cpu.z(n, vl);
    (0..es.elements(vl)).map(|e| z.get(e, es)).collect()
}

/// `values` repeated to `count` elements.
fn repeated(values: &[u64], count: usize) -> Vec<u64> {
    (0..count).map(|e| values[e % values.len()]).collect()
}

/// Predicate `n` holding `bits` in its low bits.
fn set_p_bits(cpu: &mut Cpu<NoEl2>, n: u32, bits: u64) {
    let svl = cpu.current_svl();
    cpu.set_p(n, svl, &Pv::from_bytes(&bits.to_le_bytes()[..svl.predicate_bytes()]));
}

/// SEL takes element `e` of register `r` from the first group where the
/// counter's element `r * elements + e` is active: at 128 bits, a count of
/// 20 bytes makes the first register all Zn's and four bytes of the second;
/// three halfwords inverted make all but the first three of the first
/// register's elements, and every later register, Zn's.
#[test]
fn sel_takes_each_element_from_the_first_group_where_the_counter_is_active() {
    let mut cpu = streaming(0, 0);
    for (n, v) in [(2, 0x22), (3, 0x33), (4, 0x44), (5, 0x55)] {
        set_z(&mut cpu, n, Esize::B, &[v]);
    }
    // EncodePredCount: bytes (bit 0), a count of 20 above it.
    set_p_bits(&mut cpu, 8, (20 << 1) | 1);
    assert_eq!(exec(&mut cpu, SEL_2_B), None);
    assert_eq!(z(&cpu, 0, Esize::B), vec![0x22; 16]);
    assert_eq!(z(&cpu, 1, Esize::B), [vec![0x33; 4], vec![0x55; 12]].concat());

    for r in 0..4 {
        set_z(&mut cpu, 8 + r, Esize::H, &[0x80 + u64::from(r)]);
        set_z(&mut cpu, 12 + r, Esize::H, &[0xC0 + u64::from(r)]);
    }
    // Halfwords (bits 1:0 = 10), a count of 3, inverted (bit 15).
    set_p_bits(&mut cpu, 9, 0x8000 | (3 << 2) | 0b10);
    assert_eq!(exec(&mut cpu, SEL_4_H), None);
    assert_eq!(z(&cpu, 4, Esize::H), vec![0xC0, 0xC0, 0xC0, 0x80, 0x80, 0x80, 0x80, 0x80]);
    for r in 1..4 {
        assert_eq!(z(&cpu, 4 + r, Esize::H), vec![0x80 + u64::from(r); 8]);
    }
}

/// The results are written once every one is computed: ADD of {Z0, Z1}
/// and Z0 adds Z0's value before the instruction to Z1.
#[test]
fn every_source_is_read_before_a_destination_is_written() {
    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 0, Esize::H, &[1]);
    set_z(&mut cpu, 1, Esize::H, &[10]);
    assert_eq!(exec(&mut cpu, ADD_2X1_H_Z0), None);
    assert_eq!(z(&cpu, 0, Esize::H), vec![2; 8]);
    assert_eq!(z(&cpu, 1, Esize::H), vec![11; 8]);
}

/// SQDMULH is `SignedSat((2 * a * b) >> esize)`: -128 * -128 saturates,
/// the shift rounds toward minus infinity, and the 64-bit product is
/// exact; by a group, register `r` multiplies by `Zm + r`.
#[test]
fn sqdmulh_saturates_and_is_exact_for_doublewords() {
    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 0, Esize::B, &[0x80, 0x80]);
    set_z(&mut cpu, 1, Esize::B, &[0x40, 0x01]);
    set_z(&mut cpu, 2, Esize::B, &[0x80, 0xFF]);
    assert_eq!(exec(&mut cpu, SQDMULH_2X1_B), None);
    assert_eq!(z(&cpu, 0, Esize::B), repeated(&[0x7F, 0x01], 16));
    assert_eq!(z(&cpu, 1, Esize::B), repeated(&[0xC0, 0xFF], 16));

    let mut cpu = streaming(0, 0);
    let operands = [
        (i64::MIN as u64, i64::MIN as u64),
        (i64::MAX as u64, i64::MAX as u64),
        (u64::MAX, 1),
        (1 << 62, 4),
    ];
    for (r, (a, b)) in (0..).zip(operands) {
        set_z(&mut cpu, r, Esize::D, &[a]);
        set_z(&mut cpu, 4 + r, Esize::D, &[b]);
    }
    assert_eq!(exec(&mut cpu, SQDMULH_4X4_D), None);
    let expected = [0x7FFF_FFFF_FFFF_FFFF, 0x7FFF_FFFF_FFFF_FFFE, u64::MAX, 2];
    for (r, want) in (0..).zip(expected) {
        assert_eq!(z(&cpu, r, Esize::D), vec![want; 2], "Z{r}");
    }
}

/// SRSHL shifts by `ShiftSat(SInt(b), esize)`, left or (rounding) right,
/// keeping the low `esize` bits: a left shift wraps, a shift by the whole
/// element or more gives zero, and a shift of -128 is one of -9. The shift
/// is the whole element's: a halfword of -256 is a shift of -17 and one of
/// 256 of 17, where their low bytes would shift by 0. URSHL reads its
/// element unsigned; by a group, register `r` shifts by `Zm + r`.
#[test]
fn the_rounding_shifts_clamp_the_shift_and_wrap_the_result() {
    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 0, Esize::B, &[0x40, 0x03, 0x06, 0x01, 0x80, 0x7F, 0x01, 0xFD]);
    set_z(&mut cpu, 1, Esize::B, &[0xFF]);
    set_z(&mut cpu, 2, Esize::B, &[0x01, 0xFF, 0xFE, 0x08, 0xF7, 0x80, 0x09, 0xFF]);
    assert_eq!(exec(&mut cpu, SRSHL_2X1_B), None);
    assert_eq!(z(&cpu, 0, Esize::B), repeated(&[0x80, 0x02, 0x02, 0x00, 0x00, 0x00, 0x00, 0xFF], 16));
    assert_eq!(z(&cpu, 1, Esize::B), repeated(&[0xFE, 0, 0, 0, 0, 0, 0, 0], 16));

    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 0, Esize::H, &[0x7FFF, 0x0001, 0x8000, 0x0001, 0x0001, 0x8000, 0x8000, 0x4000]);
    set_z(&mut cpu, 1, Esize::H, &[0xFFFF]);
    set_z(&mut cpu, 2, Esize::H, &[0xFF00, 0x0100, 0xFFEF, 0x0011, 0x0010, 0xFFF0, 0xFFF1, 0x0001]);
    assert_eq!(exec(&mut cpu, SRSHL_2X1_H), None);
    assert_eq!(z(&cpu, 0, Esize::H), vec![0, 0, 0, 0, 0, 0, 0xFFFF, 0x8000]);
    assert_eq!(z(&cpu, 1, Esize::H), vec![0, 0, 0, 0, 0, 0, 0, 0xFFFE]);

    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 0, Esize::B, &[0xFF]);
    set_z(&mut cpu, 1, Esize::B, &[0x81]);
    set_z(&mut cpu, 2, Esize::B, &[0xFF]);
    set_z(&mut cpu, 3, Esize::B, &[0x01]);
    assert_eq!(exec(&mut cpu, URSHL_2X2_B), None);
    assert_eq!(z(&cpu, 0, Esize::B), vec![0x80; 16]);
    assert_eq!(z(&cpu, 1, Esize::B), vec![0x02; 16]);
}

/// SMAX and SMIN compare signed, UMAX and UMIN unsigned; by a group,
/// register `r` against `Zm + r`, by one vector each against `Zm`.
#[test]
fn the_maxima_and_minima_are_signed_or_unsigned_by_one_vector_or_by_a_group() {
    let operands = [(0xFFFF_FFFF, 1), (0x8000_0000, 0x7FFF_FFFF), (5, 3), (0xFFFF_FFFE, 0xFFFF_FFFF)];
    let run = |insn: u32| {
        let mut cpu = streaming(0, 0);
        for (r, (a, b)) in (0..).zip(operands) {
            set_z(&mut cpu, r, Esize::S, &[a]);
            set_z(&mut cpu, 4 + r, Esize::S, &[b]);
        }
        assert_eq!(exec(&mut cpu, insn), None);
        (0..4).map(|r| z(&cpu, r, Esize::S)[0]).collect::<Vec<u64>>()
    };
    assert_eq!(run(SMAX_4X4_S), vec![1, 0x7FFF_FFFF, 5, 0xFFFF_FFFF]);
    assert_eq!(run(UMIN_4X4_S), vec![1, 0x7FFF_FFFF, 3, 0xFFFF_FFFE]);

    let mut cpu = streaming(0, 0);
    for (r, (a, _)) in (0..).zip(operands) {
        set_z(&mut cpu, r, Esize::S, &[a]);
    }
    set_z(&mut cpu, 4, Esize::S, &[0]);
    assert_eq!(exec(&mut cpu, SMIN_4X1_S), None);
    let got: Vec<u64> = (0..4).map(|r| z(&cpu, r, Esize::S)[0]).collect();
    assert_eq!(got, vec![0xFFFF_FFFF, 0x8000_0000, 0, 0xFFFF_FFFE]);

    set_z(&mut cpu, 0, Esize::S, &[0xFFFF_FFFF]);
    set_z(&mut cpu, 1, Esize::S, &[0x7FFF_FFFF]);
    set_z(&mut cpu, 4, Esize::S, &[0x8000_0000]);
    assert_eq!(exec(&mut cpu, UMAX_2X1_S), None);
    assert_eq!(z(&cpu, 0, Esize::S), vec![0xFFFF_FFFF; 4]);
    assert_eq!(z(&cpu, 1, Esize::S), vec![0x8000_0000; 4]);
}

/// FMAX and FMIN propagate a NaN, quieting a signalling one with Invalid
/// Operation, and order the zeros; FMAXNM and FMINNM take the number over
/// one quiet NaN, but not over a signalling NaN, and the first of two
/// quiet NaNs. Single, double and half precision.
#[test]
fn the_floating_point_maxima_and_minima_handle_nans_and_zeros() {
    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 0, Esize::S, &[ONE, 0x7FC0_0001, MINUS_ZERO, SNAN]);
    set_z(&mut cpu, 1, Esize::S, &[MINUS_ONE, TWO, ZERO, QNAN]);
    set_z(&mut cpu, 2, Esize::S, &[TWO, THREE, ZERO, THREE]);
    assert_eq!(exec(&mut cpu, FMAX_2X1_S), None);
    assert_eq!(z(&cpu, 0, Esize::S), vec![TWO, 0x7FC0_0001, ZERO, SNAN_QUIETED]);
    assert_eq!(z(&cpu, 1, Esize::S), vec![TWO, THREE, ZERO, QNAN]);
    assert_eq!(cpu.fpsr, IOC);

    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 0, Esize::S, &[QNAN, ONE, SNAN, MINUS_ZERO]);
    set_z(&mut cpu, 1, Esize::S, &[0x7FC0_0002, ONE, ONE, ONE]);
    set_z(&mut cpu, 2, Esize::S, &[FIVE, QNAN, FIVE, ZERO]);
    set_z(&mut cpu, 3, Esize::S, &[0x7FC0_0003, TWO, HALF, ONE]);
    assert_eq!(exec(&mut cpu, FMAXNM_2X2_S), None);
    assert_eq!(z(&cpu, 0, Esize::S), vec![FIVE, ONE, SNAN_QUIETED, ZERO]);
    assert_eq!(z(&cpu, 1, Esize::S), vec![0x7FC0_0002, TWO, ONE, ONE]);
    assert_eq!(cpu.fpsr, IOC);

    let mut cpu = streaming(0, 0);
    let one = 0x3FF0_0000_0000_0000;
    let operands = [
        ([one, 0x8000_0000_0000_0000], [0xC000_0000_0000_0000, 0]),
        ([0x7FF0_0000_0000_0000, 0xFFF0_0000_0000_0000], [0x4008_0000_0000_0000, 0]),
        ([0x7FF8_0000_0000_0000, one], [one, 0x7FF8_0000_0000_0001]),
        ([0x4014_0000_0000_0000, 0x401C_0000_0000_0000], [0x4018_0000_0000_0000, 0x4000_0000_0000_0000]),
    ];
    for (r, (a, b)) in (0..).zip(operands) {
        set_z(&mut cpu, r, Esize::D, &a);
        set_z(&mut cpu, 4 + r, Esize::D, &b);
    }
    assert_eq!(exec(&mut cpu, FMIN_4X4_D), None);
    assert_eq!(z(&cpu, 0, Esize::D), vec![0xC000_0000_0000_0000, 0x8000_0000_0000_0000]);
    assert_eq!(z(&cpu, 1, Esize::D), vec![0x4008_0000_0000_0000, 0xFFF0_0000_0000_0000]);
    assert_eq!(z(&cpu, 2, Esize::D), vec![0x7FF8_0000_0000_0000, 0x7FF8_0000_0000_0001]);
    assert_eq!(z(&cpu, 3, Esize::D), vec![0x4014_0000_0000_0000, 0x4000_0000_0000_0000]);
    assert_eq!(cpu.fpsr, 0);

    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 0, Esize::H, &[0x4000, 0x3800, 0x7E01, 0x0000]);
    set_z(&mut cpu, 1, Esize::H, &[0x7E01, 0xC000, 0x3C00, 0x3C00]);
    set_z(&mut cpu, 2, Esize::H, &[0x3C00]);
    set_z(&mut cpu, 3, Esize::H, &[0x3C00]);
    set_z(&mut cpu, 4, Esize::H, &[0x7E00, 0x3C00, 0x4000, 0x8000]);
    assert_eq!(exec(&mut cpu, FMINNM_4X1_H), None);
    assert_eq!(z(&cpu, 0, Esize::H), repeated(&[0x4000, 0x3800, 0x4000, 0x8000], 8));
    assert_eq!(z(&cpu, 1, Esize::H), repeated(&[0x7E01, 0xC000, 0x3C00, 0x8000], 8));
    for r in 2..4 {
        assert_eq!(z(&cpu, r, Esize::H), repeated(&[0x3C00, 0x3C00, 0x3C00, 0x8000], 8));
    }
    assert_eq!(cpu.fpsr, 0);
}

/// FCLAMP is `FPMinNum(FPMaxNum(Zn, Zd), Zm)`: a value above Zm is Zm's,
/// a quiet NaN Zn's bound and then Zm's, and where Zn exceeds Zm the
/// result is Zm's. With quiet NaNs in all three, each operation returns
/// its first operand's: Zn's NaN.
#[test]
fn fclamp_takes_the_maximum_with_zn_then_the_minimum_with_zm() {
    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 0, Esize::S, &[FIVE, TWO, QNAN, HALF]);
    set_z(&mut cpu, 1, Esize::S, &[ONE_AND_A_HALF, ZERO, MINUS_INF, INF]);
    set_z(&mut cpu, 2, Esize::S, &[ONE, THREE, ONE, ONE]);
    set_z(&mut cpu, 3, Esize::S, &[TWO, ONE, TWO, TWO]);
    assert_eq!(exec(&mut cpu, FCLAMP_2_S), None);
    assert_eq!(z(&cpu, 0, Esize::S), vec![TWO, ONE, ONE, ONE]);
    assert_eq!(z(&cpu, 1, Esize::S), vec![ONE_AND_A_HALF, ONE, ONE, TWO]);
    assert_eq!(cpu.fpsr, 0);

    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 0, Esize::S, &[0x7FC0_0001]);
    set_z(&mut cpu, 1, Esize::S, &[0x7FC0_0001]);
    set_z(&mut cpu, 2, Esize::S, &[0x7FC0_0002]);
    set_z(&mut cpu, 3, Esize::S, &[0x7FC0_0003]);
    assert_eq!(exec(&mut cpu, FCLAMP_2_S), None);
    assert_eq!(z(&cpu, 0, Esize::S), vec![0x7FC0_0002; 4]);
    assert_eq!(z(&cpu, 1, Esize::S), vec![0x7FC0_0002; 4]);
    assert_eq!(cpu.fpsr, 0);
}

/// SCLAMP and UCLAMP are `Min(Max(Zd, Zn), Zm)`, signed and unsigned: the
/// same bytes clamp differently.
#[test]
fn sclamp_and_uclamp_compare_signed_and_unsigned() {
    let run = |insn: u32, low: u64, high: u64| {
        let mut cpu = streaming(0, 0);
        for (r, v) in (0..).zip([0xFF, 0x80, 0x7F, 0x05]) {
            set_z(&mut cpu, r, Esize::B, &[v]);
        }
        set_z(&mut cpu, 4, Esize::B, &[low]);
        set_z(&mut cpu, 5, Esize::B, &[high]);
        assert_eq!(exec(&mut cpu, insn), None);
        (0..4).map(|r| z(&cpu, r, Esize::B)[0]).collect::<Vec<u64>>()
    };
    assert_eq!(run(SCLAMP_4_B, 0xF0, 0x10), vec![0xFF, 0xF0, 0x10, 0x05]);
    assert_eq!(run(UCLAMP_4_B, 0x10, 0xF0), vec![0xF0, 0x80, 0x7F, 0x10]);
}

/// ZIP of two vectors interleaves their elements, the first half of each
/// to the first register and the second half to the second; UZP takes the
/// even elements of the two to the first and the odd ones to the second.
#[test]
fn zip_and_uzp_of_two_vectors() {
    let low: Vec<u64> = (0..16).collect();
    let high: Vec<u64> = (16..32).collect();
    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 2, Esize::B, &low);
    set_z(&mut cpu, 3, Esize::B, &high);
    assert_eq!(exec(&mut cpu, ZIP_2_B), None);
    assert_eq!(z(&cpu, 0, Esize::B), vec![0, 16, 1, 17, 2, 18, 3, 19, 4, 20, 5, 21, 6, 22, 7, 23]);
    assert_eq!(z(&cpu, 1, Esize::B), vec![8, 24, 9, 25, 10, 26, 11, 27, 12, 28, 13, 29, 14, 30, 15, 31]);
    assert_eq!(exec(&mut cpu, UZP_2_B), None);
    assert_eq!(z(&cpu, 0, Esize::B), vec![0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30]);
    assert_eq!(z(&cpu, 1, Esize::B), vec![1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, 29, 31]);
}

/// ZIP of four vectors gives register `r` the `r`th quarter of each
/// source, interleaved four ways; UZP gives register `i` every fourth
/// element from the `i`th, of each source in turn. At 256 bits, two
/// elements per quarter.
#[test]
fn zip_and_uzp_of_four_vectors_interleave_by_quarters() {
    let mut cpu = streaming(1, 0);
    for r in 0..4u32 {
        let base = 0x100 * u64::from(4 + r);
        set_z(&mut cpu, 4 + r, Esize::S, &(base..base + 8).collect::<Vec<u64>>());
    }
    assert_eq!(exec(&mut cpu, ZIP_4_S), None);
    assert_eq!(z(&cpu, 0, Esize::S), vec![0x400, 0x500, 0x600, 0x700, 0x401, 0x501, 0x601, 0x701]);
    assert_eq!(z(&cpu, 1, Esize::S), vec![0x402, 0x502, 0x602, 0x702, 0x403, 0x503, 0x603, 0x703]);
    assert_eq!(z(&cpu, 2, Esize::S), vec![0x404, 0x504, 0x604, 0x704, 0x405, 0x505, 0x605, 0x705]);
    assert_eq!(z(&cpu, 3, Esize::S), vec![0x406, 0x506, 0x606, 0x706, 0x407, 0x507, 0x607, 0x707]);
    assert_eq!(exec(&mut cpu, UZP_4_S), None);
    assert_eq!(z(&cpu, 0, Esize::S), vec![0x400, 0x404, 0x500, 0x504, 0x600, 0x604, 0x700, 0x704]);
    assert_eq!(z(&cpu, 1, Esize::S), vec![0x401, 0x405, 0x501, 0x505, 0x601, 0x605, 0x701, 0x705]);
    assert_eq!(z(&cpu, 2, Esize::S), vec![0x402, 0x406, 0x502, 0x506, 0x602, 0x606, 0x702, 0x706]);
    assert_eq!(z(&cpu, 3, Esize::S), vec![0x403, 0x407, 0x503, 0x507, 0x603, 0x607, 0x703, 0x707]);
}

/// The permutes of quadwords move whole quadwords, and need the current
/// vector to hold `nreg` of the elements: ZIP of two vectors of quadwords
/// and four of doublewords are UNDEFINED at 128 bits, UZP of four of
/// quadwords at 256.
#[test]
fn the_permutes_need_a_vector_that_holds_their_elements() {
    let mut cpu = streaming(0, 0);
    for insn in [ZIP_2_Q, ZIP_4_D, UZP_4_Q] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }

    let mut cpu = streaming(1, 0);
    set_z(&mut cpu, 2, Esize::B, &(0x00..0x20).collect::<Vec<u64>>());
    set_z(&mut cpu, 3, Esize::B, &(0x20..0x40).collect::<Vec<u64>>());
    assert_eq!(exec(&mut cpu, ZIP_2_Q), None);
    assert_eq!(z(&cpu, 0, Esize::B), [(0x00..0x10).collect::<Vec<u64>>(), (0x20..0x30).collect()].concat());
    assert_eq!(z(&cpu, 1, Esize::B), [(0x10..0x20).collect::<Vec<u64>>(), (0x30..0x40).collect()].concat());
    assert_eq!(exec(&mut cpu, ZIP_4_D), None);
    assert_eq!(exec(&mut cpu, UZP_4_Q), Some(UNDEFINED));

    // At 512 bits, quadword `k` of source `r` holds the bytes `4 * r + k`.
    let mut cpu = streaming(3, 0);
    for r in 0..4u32 {
        let quads: Vec<u64> = (0..4).flat_map(|k| [u64::from(4 * r) + k; 16]).collect();
        set_z(&mut cpu, 4 + r, Esize::B, &quads);
    }
    assert_eq!(exec(&mut cpu, UZP_4_Q), None);
    for i in 0..4u32 {
        let want: Vec<u64> = (0..4).flat_map(|r| [4 * r + u64::from(i); 16]).collect();
        assert_eq!(z(&cpu, i, Esize::B), want, "Z{i}");
    }
}

/// Before the enable check, the decode makes ZIP and UZP of quadwords and
/// of four vectors of doublewords UNDEFINED where the longest Streaming
/// SVE vector cannot hold them: outside Streaming SVE mode they are
/// UNDEFINED, not the SME trap.
#[test]
fn the_decode_needs_a_longest_vector_that_holds_the_elements() {
    let short = |max_svl: VecLen| {
        processor(Implementation {
            features: Features::of(&Feature::ALL),
            max_svl,
        })
    };
    let mut cpu = short(VecLen::MIN);
    for insn in [ZIP_2_Q, ZIP_4_D, UZP_4_Q] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
    assert_eq!(exec(&mut cpu, ZIP_2_B), Some(NOT_STREAMING));

    let mut cpu = short(VecLen::from_len_field(1, VecLen::MAX));
    assert_eq!(exec(&mut cpu, ZIP_2_Q), Some(NOT_STREAMING));
    assert_eq!(exec(&mut cpu, ZIP_4_D), Some(NOT_STREAMING));
    assert_eq!(exec(&mut cpu, UZP_4_Q), Some(UNDEFINED));
}

/// SQRSHR, SQRSHRU and UQRSHR of two vectors: `(x + 2^(shift-1)) >>
/// shift`, rounding toward minus infinity after the addition, saturated
/// signed or unsigned; the first vector's results, then the second's.
#[test]
fn the_narrowing_shifts_of_two_vectors_round_and_saturate() {
    let run = |insn: u32, first: &[u64], second: &[u64]| {
        let mut cpu = streaming(0, 0);
        set_z(&mut cpu, 2, Esize::S, first);
        set_z(&mut cpu, 3, Esize::S, second);
        assert_eq!(exec(&mut cpu, insn), None);
        z(&cpu, 0, Esize::H)
    };
    assert_eq!(
        run(SQRSHR_2_H, &[0x0001_8000, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_7FFF], &[0x7FFF, 0x8000, 0, 0x0123_0000]),
        vec![2, 0x7FFF, 0x8000, 0xFFFF, 0, 1, 0, 0x123]
    );
    assert_eq!(
        run(SQRSHRU_2_H, &[5, 0xFFFF_FFFF, 0xFFFF_FFFD, 0x0002_0000], &[0x0001_FFFE, 1, 2, 3]),
        vec![3, 0, 0, 0xFFFF, 0xFFFF, 1, 1, 2]
    );
    assert_eq!(
        run(UQRSHR_2_H, &[0xFFFF_FFFF, 8, 7, 0x000F_FFF0], &[0x0010_0000, 0x18, 0x17, 0]),
        vec![0xFFFF, 1, 0, 0xFFFF, 0xFFFF, 2, 1, 0]
    );
}

/// The narrowing shifts of four vectors to a quarter of the element size:
/// SQRSHR concatenates the four vectors' results (a shift of 8 from
/// words), the N forms interleave them, element `e` of register `i` to
/// `4 * e + i` (shifts of 32 and 1 from doublewords, of 1 from words).
#[test]
fn the_narrowing_shifts_of_four_vectors_concatenate_or_interleave() {
    let run = |insn: u32, es: Esize, sources: [&[u64]; 4], result: Esize| {
        let mut cpu = streaming(0, 0);
        for (r, source) in (4..).zip(sources) {
            set_z(&mut cpu, r, es, source);
        }
        assert_eq!(exec(&mut cpu, insn), None);
        z(&cpu, 0, result)
    };
    assert_eq!(
        run(
            SQRSHR_4_B,
            Esize::S,
            [&[0x180, 0x7F80, 0xFFFF_8000, 0xFFFF_FF80], &[1, 0x80, 0x7F, 0xFFFF_FF7F], &[0x300], &[0xFFFF_FD00]],
            Esize::B
        ),
        vec![2, 0x7F, 0x80, 0, 0, 1, 0, 0xFF, 3, 3, 3, 3, 0xFD, 0xFD, 0xFD, 0xFD]
    );
    assert_eq!(
        run(
            SQRSHRN_4_H,
            Esize::D,
            [
                &[0x0000_0001_8000_0000, 0x7FFF_FFFF_FFFF_FFFF],
                &[0xFFFF_FFFF_8000_0000, 0x8000_0000_0000_0000],
                &[0x0000_7FFF_0000_0000, 0x0000_8000_0000_0000],
                &[0xFFFF_FFFF_7FFF_FFFF, 0x0000_0000_7FFF_FFFF],
            ],
            Esize::H
        ),
        vec![2, 0, 0x7FFF, 0xFFFF, 0x7FFF, 0x8000, 0x7FFF, 0]
    );
    assert_eq!(
        run(UQRSHRN_4_B, Esize::S, [&[0xFFFF_FFFF, 1, 2, 3], &[0x1FE, 0x1FF, 0x1FD, 0], &[10], &[0x80]], Esize::B),
        vec![0xFF, 0xFF, 5, 0x40, 1, 0xFF, 5, 0x40, 1, 0xFF, 5, 0x40, 2, 0, 5, 0x40]
    );
    assert_eq!(
        run(
            SQRSHRUN_4_H,
            Esize::D,
            [&[u64::MAX, 0xFFFF_FFFF_FFFF_FFFD], &[0x1_FFFF, 0x1_FFFD], &[7, 6], &[i64::MAX as u64, i64::MIN as u64]],
            Esize::H
        ),
        vec![0, 0xFFFF, 4, 0xFFFF, 0, 0xFFFF, 3, 0]
    );
}

/// `tsize = 00` of the four-vector shifts is UNDEFINED in the decode,
/// before the enable check.
#[test]
fn a_reserved_tsize_is_undefined_before_the_enable_check() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    assert_eq!(exec(&mut cpu, QRSHR_4_TSIZE_00), Some(UNDEFINED));
    assert_eq!(exec(&mut cpu, SQRSHRN_4_H), Some(NOT_STREAMING));
}

/// SQCVT, SQCVTU and UQCVT of two vectors: each element saturated, from a
/// signed or (UQCVT) an unsigned value to a signed or (SQCVTU, UQCVT) an
/// unsigned range; the first vector's results, then the second's.
#[test]
fn the_saturating_narrows_of_two_vectors() {
    let run = |insn: u32| {
        let mut cpu = streaming(0, 0);
        set_z(&mut cpu, 2, Esize::S, &[0x0000_7FFF, 0x0000_8000, 0xFFFF_8000, 0xFFFF_7FFF]);
        set_z(&mut cpu, 3, Esize::S, &[1, 0xFFFF_FFFF, 0x8000_0000, 0x7FFF_FFFF]);
        assert_eq!(exec(&mut cpu, insn), None);
        z(&cpu, 0, Esize::H)
    };
    assert_eq!(run(SQCVT_2_H), vec![0x7FFF, 0x7FFF, 0x8000, 0x8000, 1, 0xFFFF, 0x8000, 0x7FFF]);
    assert_eq!(run(SQCVTU_2_H), vec![0x7FFF, 0x8000, 0, 0, 1, 0, 0, 0xFFFF]);
    assert_eq!(run(UQCVT_2_H), vec![0x7FFF, 0x8000, 0xFFFF, 0xFFFF, 1, 0xFFFF, 0xFFFF, 0xFFFF]);
}

/// The saturating narrows of four vectors to a quarter of the element
/// size: SQCVT concatenates, the N forms interleave.
#[test]
fn the_saturating_narrows_of_four_vectors_concatenate_or_interleave() {
    let run = |insn: u32, es: Esize, sources: [&[u64]; 4], result: Esize| {
        let mut cpu = streaming(0, 0);
        for (r, source) in (4..).zip(sources) {
            set_z(&mut cpu, r, es, source);
        }
        assert_eq!(exec(&mut cpu, insn), None);
        z(&cpu, 0, result)
    };
    assert_eq!(
        run(SQCVT_4_B, Esize::S, [&[0x7F, 0x80, 0xFFFF_FF80, 0xFFFF_FF7F], &[5], &[0xFFFF_FFFB], &[0x1234]], Esize::B),
        vec![0x7F, 0x7F, 0x80, 0x80, 5, 5, 5, 5, 0xFB, 0xFB, 0xFB, 0xFB, 0x7F, 0x7F, 0x7F, 0x7F]
    );
    assert_eq!(
        run(
            SQCVTN_4_H,
            Esize::D,
            [
                &[0x7FFF, 0x8000],
                &[0xFFFF_FFFF_FFFF_8000, 0xFFFF_FFFF_FFFF_7FFF],
                &[i64::MAX as u64, i64::MIN as u64],
                &[1, u64::MAX]
            ],
            Esize::H
        ),
        vec![0x7FFF, 0x8000, 0x7FFF, 1, 0x7FFF, 0x8000, 0x8000, 0xFFFF]
    );
    assert_eq!(
        run(UQCVTN_4_B, Esize::S, [&[0xFF, 0x100, 0xFFFF_FFFF, 0], &[1], &[2], &[3]], Esize::B),
        vec![0xFF, 1, 2, 3, 0xFF, 1, 2, 3, 0xFF, 1, 2, 3, 0, 1, 2, 3]
    );
    assert_eq!(
        run(SQCVTUN_4_H, Esize::D, [&[u64::MAX, 0xFFFF], &[0x1_0000, i64::MIN as u64], &[7, 8], &[0x8000, 0x7FFF]], Esize::H),
        vec![0, 0xFFFF, 7, 0x8000, 0xFFFF, 0, 8, 0x7FFF]
    );
}

/// BFCVT rounds to BFloat16 by FPCR (to nearest, ties to even), quieting a
/// signalling NaN and overflowing to infinity; FCVT converts to IEEE half
/// precision although FPCR.AHP is set (`FPConvertSVE`), 65520 rounding to
/// infinity and a quiet NaN staying one. The first vector's results, then
/// the second's, or (N) interleaved.
#[test]
fn bfcvt_and_fcvt_concatenate_or_interleave_and_fcvt_ignores_ahp() {
    let bf16 = |insn: u32| {
        let mut cpu = streaming(0, 0);
        set_z(&mut cpu, 2, Esize::S, &[ONE, 0x3F80_8000, 0x3F81_8000, 0x3F80_8001]);
        set_z(&mut cpu, 3, Esize::S, &[SNAN, MINUS_INF, 0x7F7F_FFFF, ZERO]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(cpu.fpsr, IOC | OFC | IXC, "{insn:#010x}");
        z(&cpu, 0, Esize::H)
    };
    assert_eq!(bf16(BFCVT_2), vec![0x3F80, 0x3F80, 0x3F82, 0x3F81, 0x7FC0, 0xFF80, 0x7F80, 0]);
    assert_eq!(bf16(BFCVTN_2), vec![0x3F80, 0x7FC0, 0x3F80, 0xFF80, 0x3F82, 0x7F80, 0x3F81, 0]);

    let half = |insn: u32| {
        let mut cpu = streaming(0, FPCR_AHP);
        set_z(&mut cpu, 2, Esize::S, &[ONE, 0x477F_F000, QNAN, 0x3380_0000]);
        set_z(&mut cpu, 3, Esize::S, &[MINUS_TWO, HALF, 0x477F_E000, MINUS_ZERO]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(cpu.fpsr, OFC | IXC, "{insn:#010x}");
        z(&cpu, 0, Esize::H)
    };
    assert_eq!(half(FCVT_2), vec![0x3C00, 0x7C00, 0x7E00, 0x0001, 0xC000, 0x3800, 0x7BFF, 0x8000]);
    assert_eq!(half(FCVTN_2), vec![0x3C00, 0xC000, 0x7C00, 0x3800, 0x7E00, 0x7BFF, 0x0001, 0x8000]);
}

/// FCVTZS and FCVTZU round toward zero whatever FPCR.RMode says,
/// saturating with Invalid Operation (a NaN to zero); SCVTF and UCVTF
/// round by FPCR.RMode.
#[test]
fn the_word_conversions_saturate_and_round_toward_zero_or_by_fpcr() {
    let mut cpu = streaming(0, FPCR_RP);
    set_z(&mut cpu, 2, Esize::S, &[0xBFC0_0000, 0x4F32_D05E, QNAN, MINUS_ZERO]);
    set_z(&mut cpu, 3, Esize::S, &[0x4039_999A, 0xCF00_0000, 0xCF32_D05E, HALF]);
    assert_eq!(exec(&mut cpu, FCVTZS_2), None);
    assert_eq!(z(&cpu, 0, Esize::S), vec![0xFFFF_FFFF, 0x7FFF_FFFF, 0, 0]);
    assert_eq!(z(&cpu, 1, Esize::S), vec![2, 0x8000_0000, 0x8000_0000, 0]);
    assert_eq!(cpu.fpsr, IOC | IXC);

    let mut cpu = streaming(0, 0);
    for (r, v) in (4..).zip([MINUS_ONE, 0x4F7F_FFFF, 0x4F95_02F9, 0x3FE0_0000]) {
        set_z(&mut cpu, r, Esize::S, &[v]);
    }
    assert_eq!(exec(&mut cpu, FCVTZU_4), None);
    let got: Vec<u64> = (0..4).map(|r| z(&cpu, r, Esize::S)[0]).collect();
    assert_eq!(got, vec![0, 0xFFFF_FF00, 0xFFFF_FFFF, 1]);
    assert_eq!(cpu.fpsr, IOC | IXC);

    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 2, Esize::S, &[0xFFFF_FFFF, 0x7FFF_FFFF]);
    set_z(&mut cpu, 3, Esize::S, &[0x8000_0000, 0x0100_0001]);
    assert_eq!(exec(&mut cpu, SCVTF_2), None);
    assert_eq!(z(&cpu, 0, Esize::S), repeated(&[MINUS_ONE, 0x4F00_0000], 4));
    assert_eq!(z(&cpu, 1, Esize::S), repeated(&[0xCF00_0000, 0x4B80_0000], 4));
    assert_eq!(cpu.fpsr, IXC);

    let mut cpu = streaming(0, FPCR_RZ);
    for (r, v) in (4..).zip([0xFFFF_FFFF, 0x8000_0000, 1, 0x0100_0001]) {
        set_z(&mut cpu, r, Esize::S, &[v]);
    }
    assert_eq!(exec(&mut cpu, UCVTF_4), None);
    let got: Vec<u64> = (0..4).map(|r| z(&cpu, r, Esize::S)[0]).collect();
    assert_eq!(got, vec![0x4F7F_FFFF, 0x4F00_0000, ONE, 0x4B80_0000]);
    assert_eq!(cpu.fpsr, IXC);
}

/// FRINTN, FRINTP, FRINTM and FRINTA round by their own mode, not FPCR's,
/// keep the sign of a zero, quiet a signalling NaN with Invalid Operation,
/// and raise no Inexact (`exact` FALSE).
#[test]
fn frint_rounds_by_its_own_mode_without_inexact() {
    let two = |insn: u32| {
        let mut cpu = streaming(0, FPCR_RZ);
        set_z(&mut cpu, 2, Esize::S, &[TWO_AND_A_HALF, THREE_AND_A_HALF, MINUS_TWO_AND_A_HALF, POINT_FOUR]);
        set_z(&mut cpu, 3, Esize::S, &[MINUS_POINT_FOUR, SNAN, INF, TEN_BILLION]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(cpu.fpsr, IOC, "{insn:#010x}");
        assert_eq!(z(&cpu, 1, Esize::S), vec![MINUS_ZERO, SNAN_QUIETED, INF, TEN_BILLION], "{insn:#010x}");
        z(&cpu, 0, Esize::S)
    };
    assert_eq!(two(FRINTN_2), vec![TWO, FOUR, MINUS_TWO, ZERO]);
    assert_eq!(two(FRINTP_2), vec![THREE, FOUR, MINUS_TWO, ONE]);

    let four = |insn: u32| {
        let mut cpu = streaming(0, 0);
        set_z(&mut cpu, 4, Esize::S, &[TWO_AND_A_HALF, THREE_AND_A_HALF, MINUS_TWO_AND_A_HALF, POINT_FOUR]);
        set_z(&mut cpu, 5, Esize::S, &[MINUS_POINT_FOUR]);
        set_z(&mut cpu, 6, Esize::S, &[ONE]);
        set_z(&mut cpu, 7, Esize::S, &[MINUS_ZERO]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(cpu.fpsr, 0, "{insn:#010x}");
        assert_eq!(z(&cpu, 2, Esize::S), vec![ONE; 4], "{insn:#010x}");
        assert_eq!(z(&cpu, 3, Esize::S), vec![MINUS_ZERO; 4], "{insn:#010x}");
        (z(&cpu, 0, Esize::S), z(&cpu, 1, Esize::S)[0])
    };
    assert_eq!(four(FRINTM_4), (vec![TWO, THREE, MINUS_THREE, ZERO], MINUS_ONE));
    assert_eq!(four(FRINTA_4), (vec![THREE, FOUR, MINUS_THREE, ZERO], MINUS_ZERO));
}

/// SUNPK and UUNPK extend the low half of source `r` to destination `2 *
/// r` and its high half to `2 * r + 1`, signed or unsigned; `size = 00`
/// is UNDEFINED in the decode.
#[test]
fn the_unpacks_extend_each_half_to_its_own_register() {
    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 2, Esize::B, &[0x00, 0x7F, 0x80, 0xFF, 1, 2, 3, 4, 0x81, 0x82, 5, 6, 7, 8, 9, 0xF0]);
    assert_eq!(exec(&mut cpu, SUNPK_2_H), None);
    assert_eq!(z(&cpu, 0, Esize::H), vec![0, 0x7F, 0xFF80, 0xFFFF, 1, 2, 3, 4]);
    assert_eq!(z(&cpu, 1, Esize::H), vec![0xFF81, 0xFF82, 5, 6, 7, 8, 9, 0xFFF0]);

    set_z(&mut cpu, 4, Esize::S, &[0xFFFF_FFFF, 1, 0x8000_0000, 2]);
    set_z(&mut cpu, 5, Esize::S, &[3, 4, 5, 6]);
    assert_eq!(exec(&mut cpu, UUNPK_4_D), None);
    assert_eq!(z(&cpu, 0, Esize::D), vec![0xFFFF_FFFF, 1]);
    assert_eq!(z(&cpu, 1, Esize::D), vec![0x8000_0000, 2]);
    assert_eq!(z(&cpu, 2, Esize::D), vec![3, 4]);
    assert_eq!(z(&cpu, 3, Esize::D), vec![5, 6]);

    set_z(&mut cpu, 4, Esize::H, &[0xFFFF, 1, 0x8000, 2, 3, 4, 5, 6]);
    set_z(&mut cpu, 5, Esize::H, &[7, 8, 9, 10, 0x7FFF, 0x8001, 11, 12]);
    assert_eq!(exec(&mut cpu, SUNPK_4_S), None);
    assert_eq!(z(&cpu, 0, Esize::S), vec![0xFFFF_FFFF, 1, 0xFFFF_8000, 2]);
    assert_eq!(z(&cpu, 1, Esize::S), vec![3, 4, 5, 6]);
    assert_eq!(z(&cpu, 2, Esize::S), vec![7, 8, 9, 10]);
    assert_eq!(z(&cpu, 3, Esize::S), vec![0x7FFF, 0xFFFF_8001, 11, 12]);

    let mut cpu = processor(Implementation::EVERY_FEATURE);
    assert_eq!(exec(&mut cpu, UNPK_SIZE_00), Some(UNDEFINED));
}

/// SVE2.1's narrows of two vectors interleave: halfword `2 * e + i` from
/// word `e` of `Zn + i`, rounded and shifted right by `16 - imm4` or not,
/// then saturated, signed or unsigned, from a signed or an unsigned source;
/// no saturation sets FPSR.QC.
#[test]
fn the_sve2p1_narrows_of_two_vectors_interleave() {
    let cases = [
        (SQRSHRN_PAIR, [0x7FFF, 0xFFFF, 2, 2, 0x8000, 0x7FFF, 0, 0]),
        (SQRSHRUN_PAIR, [0xFFFF, 0, 0xC000, 0xFFFF, 0, 0xFFFF, 0x091A, 0]),
        (UQRSHRN_PAIR, [0xFFFF, 0xFFFF, 0x0180, 0x0200, 0xFFFF, 0xFFFF, 0x0012, 0xFFFF]),
        (SQCVTN_PAIR, [0x7FFF, 0x8000, 0x7FFF, 0x7FFF, 0x8000, 0x7FFF, 0x1234, 0x8000]),
        (SQCVTUN_PAIR, [0xFFFF, 0, 0xFFFF, 0xFFFF, 0, 0xFFFF, 0x1234, 0]),
        (UQCVTN_PAIR, [0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0x1234, 0xFFFF]),
    ];
    for (insn, want) in cases {
        let mut cpu = streaming(0, 0);
        set_z(&mut cpu, 2, Esize::S, &[0x7FFF_8000, 0x0001_8000, 0x8000_0000, 0x0000_1234]);
        set_z(&mut cpu, 3, Esize::S, &[0xFFFF_7FFF, 0x0002_0000, 0x7FFF_7FFF, 0xFFFF_8000]);
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
        assert_eq!(z(&cpu, 0, Esize::H), want.to_vec(), "{insn:#010x}");
        assert_eq!(cpu.fpsr, 0, "{insn:#010x}");
    }
}

/// SVE2.1's FCLAMP of one vector is the groups' `FPMinNum(FPMaxNum(Zn,
/// Zd), Zm)` at each element size. A signalling NaN in Zd raises Invalid
/// Operation in FPMaxNum, whose quieted NaN then loses to Zm in FPMinNum.
/// `size = 00`, BFCLAMP, is UNDEFINED.
#[test]
fn fclamp_of_one_vector_clamps_at_each_element_size() {
    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 0, Esize::S, &[FIVE, HALF, QNAN, SNAN]);
    set_z(&mut cpu, 1, Esize::S, &[ONE]);
    set_z(&mut cpu, 2, Esize::S, &[TWO]);
    assert_eq!(exec(&mut cpu, FCLAMP_S), None);
    assert_eq!(z(&cpu, 0, Esize::S), vec![TWO, ONE, ONE, TWO]);
    assert_eq!(cpu.fpsr, IOC);

    // 5.0 between 1.0 and 2.0, in half and double precision.
    let mut cpu = streaming(0, 0);
    set_z(&mut cpu, 0, Esize::H, &[0x4500]);
    set_z(&mut cpu, 1, Esize::H, &[0x3C00]);
    set_z(&mut cpu, 2, Esize::H, &[0x4000]);
    assert_eq!(exec(&mut cpu, FCLAMP_H), None);
    assert_eq!(z(&cpu, 0, Esize::H), vec![0x4000; 8]);
    set_z(&mut cpu, 0, Esize::D, &[0x4014_0000_0000_0000]);
    set_z(&mut cpu, 1, Esize::D, &[0x3FF0_0000_0000_0000]);
    set_z(&mut cpu, 2, Esize::D, &[0x4000_0000_0000_0000]);
    assert_eq!(exec(&mut cpu, FCLAMP_D), None);
    assert_eq!(z(&cpu, 0, Esize::D), vec![0x4000_0000_0000_0000; 2]);
    assert_eq!(cpu.fpsr, 0);

    assert_eq!(exec(&mut cpu, BFCLAMP), Some(UNDEFINED));
}

/// Every one of them takes the SME trap outside Streaming SVE mode, and is
/// UNDEFINED without FEAT_SME2.
#[test]
fn they_need_streaming_mode_and_sme2() {
    let all = [
        SEL_2_B,
        SQDMULH_4X4_D,
        SRSHL_2X1_B,
        UMIN_4X4_S,
        FMAX_2X1_S,
        FMIN_4X4_D,
        FCLAMP_2_S,
        UCLAMP_4_B,
        ZIP_2_B,
        UZP_4_S,
        ZIP_2_Q,
        SQRSHR_2_H,
        UQRSHRN_4_B,
        SQCVT_2_H,
        SQCVTN_4_H,
        BFCVT_2,
        FCVTN_2,
        FCVTZU_4,
        SCVTF_2,
        FRINTA_4,
        SUNPK_2_H,
        UUNPK_4_D,
        FCLAMP_H,
        SQRSHRN_PAIR,
        UQRSHRN_PAIR,
        SQCVTUN_PAIR,
    ];
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for insn in all {
        assert_eq!(exec(&mut cpu, insn), Some(NOT_STREAMING), "{insn:#010x}");
    }
    let features: Vec<Feature> = Feature::ALL.into_iter().filter(|&f| f != Feature::Sme2).collect();
    let mut cpu = processor(Implementation {
        features: Features::of(&features),
        max_svl: VecLen::MAX,
    });
    cpu.set_pstate_sm(true);
    for insn in all {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}
