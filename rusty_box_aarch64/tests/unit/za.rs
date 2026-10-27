//! The SME instructions on the ZA array's tiles, on a processor that
//! implements FEAT_SME and FEAT_SME2 ([`Implementation::EVERY_FEATURE`]).
//!
//! tests/vectors/qemu_sve.txt checks every class against QEMU at a 512-bit
//! vector length with random operands (kinds Z, ZF, ZR and ZM). These are
//! the properties random operands rarely or never reach, each the
//! guest-visible one (R9), each against its Arm pseudocode: the enable
//! checks and their order (`CheckStreamingSVEAndZAEnabled`,
//! `CheckSMEAndZAEnabled`), the tile geometry (`ZAhslice`, `ZAvslice`), the
//! slice index modulo the slices, the `SMEZEROUPPER` choice, MOVA's
//! predication and the quadword's one predicate bit, ZERO's tile list,
//! ADDHA's and ADDVA's sources, the integer products' signedness, byte
//! predicates, subtraction and wrap, the single-precision product's one
//! rounding, default NaN and silent exceptions, the widening products' two
//! roundings and unnegated inactive elements, the tile loads' zeroed
//! inactive elements and their faults part-way, LDR and STR of an array
//! vector and their alignment, and the encodings of other features
//! UNDEFINED. Of FEAT_SME2's: BMOPA's count of agreeing bits, the halfword
//! products' pairs and their predicates, MOVA of a group's slices from an
//! index aligned to the group, as rows and as columns, four doubleword
//! slices UNDEFINED below 256 bits, and FEAT_SME2 itself.

use std::vec;
use std::vec::Vec;

use super::super::features::{Feature, Features, Implementation, VecLen};
use super::super::sctlr;
use super::super::step::{exec, exec_in, processor, Taken, CPACR_ALL, DATA, FP_TRAP, SME_DISABLED, UNDEFINED};
use super::super::sve::{Pv, Zv};
use super::super::{Cpu, NoEl2};
use crate::softfloat::FPCR_AH;

/// The SME trap with SMTC 2: outside Streaming SVE mode.
const NOT_STREAMING: Taken = Taken { ec: 0x1D, iss: 2 };
/// The SME trap with SMTC 3: ZA storage disabled.
const INACTIVE_ZA: Taken = Taken { ec: 0x1D, iss: 3 };

/// `mova za1h.s[w12, 2], p0/m, z3.s`.
const MOVA_ZA1H_S: u32 = 0xC080_0066;
/// `zero {za}`.
const ZERO_ALL: u32 = 0xC008_00FF;
/// `smopa za0.s, p0/m, p1/m, z1.b, z2.b`.
const SMOPA: u32 = 0xA082_2020;
/// `ldr za[w12, 3], [x0, #3, mul vl]`.
const LDR_ZA: u32 = 0xE100_0003;
/// `bmopa za0.s, p0/m, p1/m, z1.s, z2.s` and `bmops za3.s, p2/m, p3/m,
/// z4.s, z5.s`.
const BMOPA: u32 = 0x8082_2028;
const BMOPS: u32 = 0x8085_689B;
/// `smopa za0.s, p0/m, p1/m, z1.h, z2.h`, and `umopa` into `za1.s`,
/// `smops` into `za2.s` and `umops` into `za3.s` of the same operands.
const SMOPA_H: u32 = 0xA082_2028;
const UMOPA_H: u32 = 0xA182_2029;
const SMOPS_H: u32 = 0xA082_203A;
const UMOPS_H: u32 = 0xA182_203B;
/// `mova za0h.b[w12, 2:3], {z4.b-z5.b}`, `mova za1v.s[w13, 2:3],
/// {z4.s-z5.s}`, `mova za7h.d[w12, 0:1], {z4.d-z5.d}` and `mova {z6.h-z7.h},
/// za1v.h[w12, 6:7]`.
const MOVA_TO_2_B: u32 = 0xC004_0081;
const MOVA_TO_2_S: u32 = 0xC084_A083;
const MOVA_TO_2_D: u32 = 0xC0C4_0087;
const MOVA_FROM_2_H: u32 = 0xC046_80E6;
/// `mova za2v.s[w14, 0:3], {z8.s-z11.s}`, `mova {z4.s-z7.s}, za2v.s[w15,
/// 0:3]`, `mova za0h.b[w12, 4:7], {z4.b-z7.b}` and `mova {z12.b-z15.b},
/// za0h.b[w13, 8:11]`.
const MOVA_TO_4_S: u32 = 0xC084_C502;
const MOVA_FROM_4_S: u32 = 0xC086_E444;
const MOVA_TO_4_B: u32 = 0xC004_0481;
const MOVA_FROM_4_B: u32 = 0xC006_244C;
/// `mova za5h.d[w12, 0:3], {z8.d-z11.d}` and `mova {z8.d-z11.d},
/// za6v.d[w12, 0:3]`.
const MOVA_TO_4_D: u32 = 0xC0C4_0505;
const MOVA_FROM_4_D: u32 = 0xC0C6_84C8;

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

/// `size` bytes of RAM counting up from 0.
fn counting(size: usize) -> Vec<u8> {
    (0..size).map(|i| i as u8).collect()
}

/// A processor in Streaming SVE mode with ZA storage enabled and
/// `SMCR_EL1.LEN = len`.
fn with_za(len: u64) -> Cpu<NoEl2> {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.sys.smcr_el1 = len;
    cpu.set_pstate_sm(true);
    cpu.set_pstate_za(true);
    cpu
}

/// The vector whose words are `words`.
fn of_words(words: &[u32]) -> Zv {
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    Zv::from_bytes(&bytes)
}

/// The words of `bytes`.
fn words(bytes: &[u8]) -> Vec<u32> {
    bytes.chunks_exact(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect()
}

/// The words of ZA vector `n` at the current length.
fn za_words(cpu: &Cpu<NoEl2>, n: usize) -> Vec<u32> {
    words(cpu.za_row(n, cpu.current_svl()))
}

/// ZA vector `n`, at the current length, of `words`.
fn set_za_words(cpu: &mut Cpu<NoEl2>, n: usize, words: &[u32]) {
    let svl = cpu.current_svl();
    cpu.set_za_vector(n, svl, &of_words(words));
}

/// The words of Z register `n` at the current length.
fn z_words(cpu: &Cpu<NoEl2>, n: u32) -> Vec<u32> {
    let svl = cpu.current_svl();
    words(&cpu.z(n, svl).as_bytes()[..svl.bytes()])
}

/// Z register `n`, at the current length, of `words`.
fn set_z_words(cpu: &mut Cpu<NoEl2>, n: u32, words: &[u32]) {
    let svl = cpu.current_svl();
    cpu.set_z(n, svl, &of_words(words));
}

/// Predicate `n` of the bits of `bits`, bit `i` that of vector byte `i`.
fn set_p_bits(cpu: &mut Cpu<NoEl2>, n: u32, bits: u64) {
    let svl = cpu.current_svl();
    cpu.set_p(n, svl, &Pv::from_bytes(&bits.to_le_bytes()));
}

/// MOVA, the outer products and the tile loads take
/// `CheckStreamingSVEAndZAEnabled`: SME's enable and SIMD&FP's, then the
/// trap outside Streaming SVE mode, then the trap with ZA disabled. ZERO,
/// LDR and STR take `CheckSMEAndZAEnabled`, and execute outside Streaming
/// SVE mode.
#[test]
fn the_za_instructions_check_sme_then_streaming_mode_then_za() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    assert_eq!(exec(&mut cpu, MOVA_ZA1H_S), Some(NOT_STREAMING));
    assert_eq!(exec(&mut cpu, ZERO_ALL), Some(INACTIVE_ZA));
    cpu.set_pstate_sm(true);
    assert_eq!(exec(&mut cpu, MOVA_ZA1H_S), Some(INACTIVE_ZA));
    assert_eq!(exec(&mut cpu, SMOPA), Some(INACTIVE_ZA));
    cpu.set_pstate_sm(false);
    cpu.set_pstate_za(true);
    assert_eq!(exec(&mut cpu, MOVA_ZA1H_S), Some(NOT_STREAMING));
    assert_eq!(exec(&mut cpu, ZERO_ALL), None);
    let mut ram = counting(64);
    cpu.x[0] = DATA;
    assert_eq!(exec_in(&mut cpu, &mut ram, LDR_ZA), None);
    // SME's enable first, then SIMD&FP's, before either mode.
    cpu.set_pstate_za(false);
    cpu.sys.cpacr_el1 = 0;
    assert_eq!(exec(&mut cpu, MOVA_ZA1H_S), Some(SME_DISABLED));
    assert_eq!(exec(&mut cpu, ZERO_ALL), Some(SME_DISABLED));
    cpu.sys.cpacr_el1 = 3 << 24;
    assert_eq!(exec(&mut cpu, ZERO_ALL), Some(FP_TRAP));
    cpu.sys.cpacr_el1 = CPACR_ALL;
    assert_eq!(exec(&mut cpu, ZERO_ALL), Some(INACTIVE_ZA));
}

/// Horizontal slice `s` of 32-bit tile `t` is ZA vector `t + s * 4`
/// (`ZAhslice`), and its vertical slice `s` is word `s` of each of the
/// tile's vectors (`ZAvslice`); MOVA reads back what it wrote.
#[test]
fn tile_slices_are_interleaved_vectors_and_their_columns() {
    let mut cpu = with_za(0);
    set_z_words(&mut cpu, 3, &[1, 2, 3, 4]);
    set_p_bits(&mut cpu, 0, u64::MAX);
    set_p_bits(&mut cpu, 1, u64::MAX);
    cpu.x[12] = 0;
    assert_eq!(exec(&mut cpu, MOVA_ZA1H_S), None);
    for n in 0..16 {
        let want = if n == 9 { vec![1, 2, 3, 4] } else { vec![0; 4] };
        assert_eq!(za_words(&cpu, n), want, "ZA[{n}]");
    }
    // mova za1v.s[w13, 1], p1/m, z3.s with W13 = 2: word 3 of vectors 1,
    // 5, 9 and 13.
    cpu.x[13] = 2;
    assert_eq!(exec(&mut cpu, 0xC080_A465), None);
    assert_eq!(za_words(&cpu, 1), [0, 0, 0, 1]);
    assert_eq!(za_words(&cpu, 5), [0, 0, 0, 2]);
    assert_eq!(za_words(&cpu, 9), [1, 2, 3, 3]);
    assert_eq!(za_words(&cpu, 13), [0, 0, 0, 4]);
    // mova z5.s, p0/m, za1v.s[w13, 1] and mova z5.s, p0/m, za1h.s[w12, 2].
    assert_eq!(exec(&mut cpu, 0xC082_A0A5), None);
    assert_eq!(z_words(&cpu, 5), [1, 2, 3, 4]);
    assert_eq!(exec(&mut cpu, 0xC082_00C5), None);
    assert_eq!(z_words(&cpu, 5), [1, 2, 3, 3]);
}

/// The slice is `(UInt(Ws) + offset) MOD dim`, `Ws` the low word of one of
/// X12 to X15.
#[test]
fn the_slice_index_is_ws_plus_the_offset_modulo_the_slices() {
    let mut cpu = with_za(0);
    set_z_words(&mut cpu, 3, &[7; 4]);
    set_p_bits(&mut cpu, 0, u64::MAX);
    // mova za1h.s[w12, 2]: (6 + 2) MOD 4 is slice 0, ZA vector 1.
    cpu.x[12] = 0xFFFF_FFFF_0000_0006;
    assert_eq!(exec(&mut cpu, MOVA_ZA1H_S), None);
    assert_eq!(za_words(&cpu, 1), [7; 4]);
    // (0xFFFF_FFFF + 2) MOD 4 is slice 1, ZA vector 5.
    cpu.x[12] = 0xFFFF_FFFF;
    set_z_words(&mut cpu, 3, &[8; 4]);
    assert_eq!(exec(&mut cpu, MOVA_ZA1H_S), None);
    assert_eq!(za_words(&cpu, 5), [8; 4]);
    assert_eq!(za_words(&cpu, 9), [0; 4]);
}

/// A write of a ZA vector zeroes it above the Streaming SVE vector length
/// (`Unpredictable_SMEZEROUPPER` taken as TRUE), as a write of a Z
/// register does.
#[test]
fn a_za_vector_write_zeroes_it_above_the_vector_length() {
    let mut cpu = with_za(0);
    cpu.set_za_vector(9, VecLen::MAX, &Zv::from_bytes(&[0xFF; VecLen::MAX.bytes()]));
    set_z_words(&mut cpu, 3, &[1, 2, 3, 4]);
    set_p_bits(&mut cpu, 0, u64::MAX);
    cpu.x[12] = 0;
    assert_eq!(exec(&mut cpu, MOVA_ZA1H_S), None);
    let row = cpu.za_row(9, VecLen::MAX);
    assert_eq!(words(&row[..16]), [1, 2, 3, 4]);
    assert!(row[16..].iter().all(|&b| b == 0));
}

/// MOVA moves the active elements alone, into a slice and out of one, and
/// a quadword element is active by the predicate bit of its first byte
/// (`ActivePredicateElement` with `esize` 128).
#[test]
fn mova_moves_the_active_elements_and_a_quadword_reads_one_predicate_bit() {
    let mut cpu = with_za(1);
    let svl = cpu.current_svl();
    let bytes: Vec<u8> = (1..=32).collect();
    cpu.set_z(4, svl, &Zv::from_bytes(&bytes));
    // Bit 1, inside quadword 0 but not its first, and bit 16, quadword 1's
    // first.
    set_p_bits(&mut cpu, 2, (1 << 1) | (1 << 16));
    for n in [15, 31] {
        cpu.set_za_vector(n, svl, &Zv::from_bytes(&[0xEE; 32]));
    }
    // mova za15v.q[w14, 0], p2/m, z4.q with W14 = 1: quadword 1 of ZA
    // vectors 15 (element 0, inactive) and 31 (element 1).
    cpu.x[14] = 1;
    assert_eq!(exec(&mut cpu, 0xC0C1_C88F), None);
    assert_eq!(cpu.za_row(15, svl), [0xEE; 32]);
    assert_eq!(cpu.za_row(31, svl)[..16], [0xEE; 16]);
    assert_eq!(cpu.za_row(31, svl)[16..], bytes[16..]);
    // mova z6.q, p2/m, za15v.q[w14, 0]: quadword 0 of z6 kept.
    cpu.set_z(6, svl, &Zv::from_bytes(&[0x77; 32]));
    assert_eq!(exec(&mut cpu, 0xC0C3_C9E6), None);
    let z6 = cpu.z(6, svl);
    assert_eq!(z6.as_bytes()[..16], [0x77; 16]);
    assert_eq!(z6.as_bytes()[16..32], bytes[16..]);
}

/// ZERO clears the 64-bit tiles its mask lists, tile `i` being ZA vectors
/// `i`, `i + 8`, ...; the others keep their contents.
#[test]
fn zero_clears_the_listed_doubleword_tiles_alone() {
    let mut cpu = with_za(0);
    for n in 0..16 {
        set_za_words(&mut cpu, n, &[n as u32 + 1; 4]);
    }
    // zero {za0.d, za3.d}.
    assert_eq!(exec(&mut cpu, 0xC008_0009), None);
    for n in 0..16 {
        let want = if n % 8 == 0 || n % 8 == 3 { 0 } else { n as u32 + 1 };
        assert_eq!(za_words(&cpu, n), [want; 4], "ZA[{n}]");
    }
}

/// ADDHA adds element `col` of Zn to element `[row, col]` of the tile and
/// ADDVA element `row`, where Pn makes the row active and Pm the column,
/// modulo 2^32.
#[test]
fn addha_adds_by_column_and_addva_by_row_under_both_predicates() {
    let mut cpu = with_za(0);
    let m = u32::MAX;
    // Tile 2: ZA vectors 2, 6, 10, 14.
    for row in 0..4 {
        set_za_words(&mut cpu, 2 + 4 * row, &[m; 4]);
    }
    set_z_words(&mut cpu, 7, &[10, 20, 30, 40]);
    // Rows 0, 1 and 3; columns 1, 2 and 3.
    set_p_bits(&mut cpu, 0, 0x1011);
    set_p_bits(&mut cpu, 1, 0x1110);
    // addha za2.s, p0/m, p1/m, z7.s.
    assert_eq!(exec(&mut cpu, 0xC090_20E2), None);
    let added = [m, 19, 29, 39];
    assert_eq!(za_words(&cpu, 2), added);
    assert_eq!(za_words(&cpu, 6), added);
    assert_eq!(za_words(&cpu, 10), [m; 4]);
    assert_eq!(za_words(&cpu, 14), added);
    // addva za2.s, p0/m, p1/m, z7.s.
    assert_eq!(exec(&mut cpu, 0xC091_20E2), None);
    assert_eq!(za_words(&cpu, 2), [m, 29, 39, 49]);
    assert_eq!(za_words(&cpu, 6), [m, 39, 49, 59]);
    assert_eq!(za_words(&cpu, 10), [m; 4]);
    assert_eq!(za_words(&cpu, 14), [m, 59, 69, 79]);
}

/// The integer outer products sum the four byte products of row element
/// `row` of Zn and column element `col` of Zm, Zn's bytes unsigned for
/// USMOPA and UMOPA and Zm's for SUMOPA and UMOPA, each byte under its own
/// predicate bits, modulo 2^32; SMOPS subtracts.
#[test]
fn the_integer_outer_products_take_each_operands_signedness_and_byte_predicates() {
    let mut cpu = with_za(0);
    // Row 0: bytes 0xFF, 2, 3, 4; column 0: bytes 0x80, 1, 1, 1.
    set_z_words(&mut cpu, 1, &[0x0403_02FF, 0, 0, 0]);
    set_z_words(&mut cpu, 2, &[0x0101_0180, 0, 0, 0]);
    set_p_bits(&mut cpu, 0, u64::MAX);
    set_p_bits(&mut cpu, 1, u64::MAX);
    // SMOPA: (-1)(-128) + 9; SUMOPA: (-1)(128) + 9; USMOPA: 255(-128) + 9;
    // UMOPA: 255(128) + 9.
    for (insn, want) in [(SMOPA, 137), (0xA0A2_2020, -119), (0xA182_2020, -32631), (0xA1A2_2020, 32649)] {
        set_za_words(&mut cpu, 0, &[0; 4]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(za_words(&cpu, 0), [want as u32, 0, 0, 0], "{insn:#010x}");
    }
    // Wrapping, and column 0's fourth byte inactive: 137 - 4.
    set_za_words(&mut cpu, 0, &[0x7FFF_FFFF, 0, 0, 0]);
    set_p_bits(&mut cpu, 1, !(1 << 3));
    assert_eq!(exec(&mut cpu, SMOPA), None);
    assert_eq!(za_words(&cpu, 0)[0], 0x7FFF_FFFF_u32.wrapping_add(133));
    // smops za3.s: 100 - 133.
    set_za_words(&mut cpu, 3, &[100, 0, 0, 0]);
    assert_eq!(exec(&mut cpu, 0xA082_2033), None);
    assert_eq!(za_words(&cpu, 3)[0], (-33i32) as u32);
}

/// FMOPA of single-precision values is `FPMulAdd_ZA`: one rounding of
/// `acc + a*b`, the default NaN for a NaN whatever FPCR.DN, negative under
/// FPCR.AH, and no exception reaching FPSR; FMOPS negates the row element.
#[test]
fn fmopa_is_a_fused_multiply_add_with_the_default_nan_and_no_exceptions() {
    let mut cpu = with_za(0);
    set_p_bits(&mut cpu, 0, u64::MAX);
    set_p_bits(&mut cpu, 1, u64::MAX);
    // Rows: 1 + 2^-12, a signalling NaN, 1/3. Columns: 1 + 2^-12, 1, 1/3.
    set_z_words(&mut cpu, 1, &[0x3F80_0800, 0x7F80_0001, 0x3EAA_AAAB, 0]);
    set_z_words(&mut cpu, 2, &[0x3F80_0800, 0x3F80_0000, 0x3EAA_AAAB, 0]);
    // Tile 0's row 0 is ZA vector 0, row 1 vector 4, row 2 vector 8.
    set_za_words(&mut cpu, 0, &[0xBF80_0000, 0, 0, 0]);
    cpu.fpcr = 0;
    cpu.fpsr = 0;
    // fmopa za0.s, p0/m, p1/m, z1.s, z2.s.
    let fmopa = 0x8082_2020;
    assert_eq!(exec(&mut cpu, fmopa), None);
    // -1 + (1 + 2^-12)^2 is 2^-11 + 2^-24, which a rounded product loses.
    assert_eq!(za_words(&cpu, 0)[0], 0x3A00_0400);
    assert_eq!(za_words(&cpu, 4)[0], 0x7FC0_0000);
    assert_ne!(za_words(&cpu, 8)[2], 0);
    assert_eq!(cpu.fpsr, 0);
    cpu.fpcr = FPCR_AH;
    assert_eq!(exec(&mut cpu, fmopa), None);
    assert_eq!(za_words(&cpu, 4)[0], 0xFFC0_0000);
    // fmops za0.s: 0 - 2 * 3.
    cpu.fpcr = 0;
    set_z_words(&mut cpu, 1, &[0x4000_0000, 0, 0, 0]);
    set_z_words(&mut cpu, 2, &[0x4040_0000, 0, 0, 0]);
    set_za_words(&mut cpu, 0, &[0; 4]);
    assert_eq!(exec(&mut cpu, 0x8082_2030), None);
    assert_eq!(za_words(&cpu, 0)[0], 0xC0C0_0000);
}

/// FMOPA of half-precision pairs is `FPDotAdd_ZA`: the two products summed
/// with one rounding, then added to the accumulator with a second, so
/// `1 + 2^-28` is 1 before -1 is added. FMOPS negates the active row
/// elements alone, an inactive one being +0.
#[test]
fn the_half_precision_outer_product_rounds_the_dot_product_then_the_sum() {
    let mut cpu = with_za(0);
    set_p_bits(&mut cpu, 0, u64::MAX);
    set_p_bits(&mut cpu, 1, u64::MAX);
    // Row 0's pair and column 0's: 1.0 and 2^-14.
    set_z_words(&mut cpu, 1, &[0x0400_3C00, 0, 0, 0]);
    set_z_words(&mut cpu, 2, &[0x0400_3C00, 0, 0, 0]);
    // Tile 1's row 0 is ZA vector 1.
    set_za_words(&mut cpu, 1, &[0xBF80_0000, 0, 0, 0]);
    // fmopa za1.s, p0/m, p1/m, z1.h, z2.h.
    assert_eq!(exec(&mut cpu, 0x81A2_2021), None);
    assert_eq!(za_words(&cpu, 1)[0], 0);
    // Row 0's pair +0 and +0, its second element inactive; column 0's 1.0
    // and 1.0; the accumulator -0. Negated, the first product is -0 and
    // the second +0, whose sum is +0, and -0 + +0 is +0; negating the
    // inactive element too would make both -0, and the result -0.
    set_z_words(&mut cpu, 1, &[0, 0, 0, 0]);
    set_z_words(&mut cpu, 2, &[0x3C00_3C00, 0, 0, 0]);
    set_p_bits(&mut cpu, 0, !(1 << 2));
    set_za_words(&mut cpu, 1, &[0x8000_0000, 0, 0, 0]);
    // fmops za1.s, p0/m, p1/m, z1.h, z2.h.
    assert_eq!(exec(&mut cpu, 0x81A2_2031), None);
    assert_eq!(za_words(&cpu, 1)[0], 0);
}

/// BFMOPA adds the sum of the products of BFloat16 pairs to a
/// single-precision accumulator (`BFDotAdd`), BFMOPS that of the negated
/// row elements.
#[test]
fn bfmopa_adds_the_pairs_products_and_bfmops_subtracts_them() {
    let mut cpu = with_za(0);
    set_p_bits(&mut cpu, 0, u64::MAX);
    set_p_bits(&mut cpu, 1, u64::MAX);
    // Row 0: 1.0 and 2.0; column 0: 3.0 and 4.0.
    set_z_words(&mut cpu, 1, &[0x4000_3F80, 0, 0, 0]);
    set_z_words(&mut cpu, 2, &[0x4080_4040, 0, 0, 0]);
    // Tile 2's row 0 is ZA vector 2: 0.5 + 11 and 0.5 - 11.
    set_za_words(&mut cpu, 2, &[0x3F00_0000, 0, 0, 0]);
    assert_eq!(exec(&mut cpu, 0x8182_2022), None);
    assert_eq!(za_words(&cpu, 2)[0], 0x4138_0000);
    set_za_words(&mut cpu, 2, &[0x3F00_0000, 0, 0, 0]);
    assert_eq!(exec(&mut cpu, 0x8182_2032), None);
    assert_eq!(za_words(&cpu, 2)[0], 0xC128_0000);
}

/// LD1W to a vertical slice loads element `e` from `X[n] + (X[m] + e) * 4`
/// where it is active and zeroes it where not, the elements after the last
/// active one included (docs/qemu-upstream-bugs.md, entry 15); the rest of
/// each vector keeps its contents.
#[test]
fn ld1w_to_a_vertical_slice_zeroes_every_inactive_element() {
    let mut cpu = with_za(0);
    let mut ram = counting(64);
    let m = u32::MAX;
    for row in 0..4 {
        set_za_words(&mut cpu, 1 + 4 * row, &[m; 4]);
    }
    cpu.x[0] = DATA;
    cpu.x[1] = 2;
    cpu.x[12] = 0;
    // Elements 0 and 1.
    set_p_bits(&mut cpu, 0, 0x0011);
    // ld1w {za1v.s[w12, 1]}, p0/z, [x0, x1, lsl #2]: word 1 of ZA vectors
    // 1, 5, 9 and 13, from bytes 8 up.
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xE081_8005), None);
    assert_eq!(za_words(&cpu, 1), [m, 0x0B0A_0908, m, m]);
    assert_eq!(za_words(&cpu, 5), [m, 0x0F0E_0D0C, m, m]);
    assert_eq!(za_words(&cpu, 9), [m, 0, m, m]);
    assert_eq!(za_words(&cpu, 13), [m, 0, m, m]);
}

/// LD1Q loads 16-byte elements, each under the predicate bit of its first
/// byte, the inactive ones zero; ST1Q stores the active ones alone.
#[test]
fn ld1q_and_st1q_move_quadwords_under_one_predicate_bit_each() {
    let mut cpu = with_za(1);
    let svl = cpu.current_svl();
    let mut ram = counting(128);
    cpu.x[0] = DATA;
    cpu.x[1] = 1;
    cpu.x[12] = 0;
    // Element 0's first byte, and a byte of element 1 that is not its
    // first.
    set_p_bits(&mut cpu, 0, 1 | (1 << 17));
    // ld1q {za3h.q[w12, 0]}, p0/z, [x0, x1, lsl #4]: ZA vector 3, from
    // byte 16 up.
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xE1C1_0003), None);
    assert_eq!(cpu.za_row(3, svl)[..16], ram[16..32]);
    assert_eq!(cpu.za_row(3, svl)[16..], [0; 16]);
    // st1q {za3h.q[w12, 0]}, p0, [x0, x1, lsl #4].
    let mut out = vec![0xEE; 128];
    assert_eq!(exec_in(&mut cpu, &mut out, 0xE1E1_0003), None);
    assert_eq!(out[16..32], counting(128)[16..32]);
    assert!(out[..16].iter().chain(&out[32..]).all(|&b| b == 0xEE));
}

/// A tile slice load that faults part-way writes no slice; a store keeps
/// the elements it stored before the faulting one.
#[test]
fn a_fault_part_way_writes_no_slice_and_keeps_the_stores_before_it() {
    let mut cpu = with_za(0);
    let mut ram = counting(64);
    // Words from 8 bytes before the end of the RAM.
    cpu.x[0] = DATA + 56;
    cpu.x[1] = 0;
    cpu.x[12] = 0;
    set_p_bits(&mut cpu, 0, u64::MAX);
    set_za_words(&mut cpu, 5, &[7; 4]);
    // ld1w {za1h.s[w12, 1]}, p0/z, [x0, x1, lsl #2]: ZA vector 5.
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xE081_0005), Some(data_abort(EXTERNAL, false)));
    assert_eq!(cpu.sys.far_el1, DATA + 64);
    assert_eq!(za_words(&cpu, 5), [7; 4]);
    // st1d {za7h.d[w15, 1]}, p1, [x0, x1, lsl #3] with W15 = 0: ZA vector
    // 15, its doubleword 0 to bytes 56 to 63, its doubleword 1 faulting.
    cpu.x[15] = 0;
    set_p_bits(&mut cpu, 1, u64::MAX);
    let svl = cpu.current_svl();
    cpu.set_za_vector(15, svl, &Zv::from_bytes(&[0xA5; 16]));
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xE0E1_640F), Some(data_abort(EXTERNAL, true)));
    assert_eq!(cpu.sys.far_el1, DATA + 64);
    assert_eq!(ram[..56], counting(56)[..]);
    assert_eq!(ram[56..], [0xA5; 8]);
}

/// LDR and STR of ZA move array vector `(UInt(Wv) + off4) MOD dim` at
/// `X[n] + off4 * (SVL DIV 8)`, outside Streaming SVE mode too, each byte
/// an access of its own; from a base not aligned to 16 bytes they fault
/// where alignment is enforced.
#[test]
fn ldr_and_str_of_za_move_a_vector_at_the_offset_times_its_length() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.set_pstate_za(true);
    let svl = cpu.current_svl();
    let mut ram = counting(64);
    cpu.x[0] = DATA;
    cpu.x[12] = 14;
    // ldr za[w12, 3], [x0, #3, mul vl]: vector (14 + 3) MOD 16, from byte
    // 48 up.
    assert_eq!(exec_in(&mut cpu, &mut ram, LDR_ZA), None);
    assert_eq!(cpu.za_row(1, svl), &counting(64)[48..]);
    // str za[w13, 0], [x0] with W13 = 1.
    cpu.x[13] = 1;
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xE120_2000), None);
    assert_eq!(ram[..16], counting(64)[48..]);
    cpu.x[0] = DATA + 1;
    cpu.sys.sctlr_el1 |= sctlr::A;
    assert_eq!(exec_in(&mut cpu, &mut ram, LDR_ZA), Some(data_abort(ALIGNMENT, false)));
    assert_eq!(cpu.sys.far_el1, DATA + 49);
    // A base aligned to 8 bytes is misaligned (`IsAlignedSize(base, 16)`):
    // ldr za[w12, 0], [x0] faults at it.
    cpu.x[0] = DATA + 8;
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xE100_0000), Some(data_abort(ALIGNMENT, false)));
    assert_eq!(cpu.sys.far_el1, DATA + 8);
}

/// BMOPA adds to element `[row, col]` the number of bits in which row
/// element `row` and column element `col` agree, modulo 2^32, where both
/// are active, and leaves the rest of the tile as it was; BMOPS subtracts
/// it.
#[test]
fn bmopa_counts_the_bits_in_which_the_row_and_column_elements_agree() {
    let mut cpu = with_za(0);
    set_z_words(&mut cpu, 1, &[0xFFFF_0000, 0xAAAA_AAAA, 0, 0xFF]);
    set_z_words(&mut cpu, 2, &[0xFFFF_0000, 0xFFFF_FFFF, 0, 0x5555_5555]);
    // Rows 0, 2 and 3 active; columns 0, 1 and 2.
    set_p_bits(&mut cpu, 0, 0x1101);
    set_p_bits(&mut cpu, 1, 0x0111);
    set_za_words(&mut cpu, 0, &[0xFFFF_FFF0, 0, 0, 7]);
    set_za_words(&mut cpu, 4, &[9; 4]);
    assert_eq!(exec(&mut cpu, BMOPA), None);
    assert_eq!(za_words(&cpu, 0), [0x10, 16, 16, 7]);
    assert_eq!(za_words(&cpu, 4), [9; 4]);
    assert_eq!(za_words(&cpu, 8), [16, 0, 32, 0]);
    assert_eq!(za_words(&cpu, 12), [8, 8, 24, 0]);
    assert_eq!(za_words(&cpu, 1), [0; 4]);
    // bmops za3.s of zeros: all 32 bits agree.
    set_p_bits(&mut cpu, 2, u64::MAX);
    set_p_bits(&mut cpu, 3, u64::MAX);
    set_za_words(&mut cpu, 3, &[10, 100, 0, 0]);
    assert_eq!(exec(&mut cpu, BMOPS), None);
    assert_eq!(za_words(&cpu, 3), [-22i32 as u32, 68, -32i32 as u32, -32i32 as u32]);
    assert_eq!(za_words(&cpu, 15), [-32i32 as u32; 4]);
}

/// SMOPA, UMOPA, SMOPS and UMOPS of halfwords sum the products of the two
/// halfwords of a row element and of a column element in the same place,
/// both signed or both unsigned, each product only where both halfwords
/// are active, modulo 2^32.
#[test]
fn the_halfword_outer_products_multiply_halfwords_in_the_same_place() {
    let mut cpu = with_za(0);
    // Rows: halfwords 0xFFFF and 3, then 0xFFFF twice; columns: 0x8000
    // and 5, then 0xFFFF twice.
    set_z_words(&mut cpu, 1, &[0x0003_FFFF, 0xFFFF_FFFF, 0, 0]);
    set_z_words(&mut cpu, 2, &[0x0005_8000, 0xFFFF_FFFF, 0, 0]);
    set_p_bits(&mut cpu, 0, u64::MAX);
    set_p_bits(&mut cpu, 1, u64::MAX);
    for insn in [SMOPA_H, UMOPA_H, SMOPS_H, UMOPS_H] {
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
    }
    // SMOPA: (-1)(-32768) + 3 * 5 and (-1)(-1) + 3(-1); (-1)(-32768) +
    // (-1)5 and 2.
    assert_eq!(za_words(&cpu, 0), [32783, -2i32 as u32, 0, 0]);
    assert_eq!(za_words(&cpu, 4), [32763, 2, 0, 0]);
    // UMOPA: 65535 * 32768 + 15 and 65535 * 65538, then 65535 * 32773 and
    // 2 * 65535^2, modulo 2^32.
    assert_eq!(za_words(&cpu, 1), [0x7FFF_800F, 0xFFFE, 0, 0]);
    assert_eq!(za_words(&cpu, 5), [0x8004_7FFB, 0xFFFC_0002, 0, 0]);
    // SMOPS and UMOPS from zero.
    assert_eq!(za_words(&cpu, 2), [-32783i32 as u32, 2, 0, 0]);
    assert_eq!(za_words(&cpu, 6), [-32763i32 as u32, -2i32 as u32, 0, 0]);
    assert_eq!(za_words(&cpu, 3), [0x7FFF_800F_u32.wrapping_neg(), 0xFFFE_u32.wrapping_neg(), 0, 0]);
    assert_eq!(za_words(&cpu, 7), [0x8004_7FFB_u32.wrapping_neg(), 0xFFFC_0002_u32.wrapping_neg(), 0, 0]);
    // Row 0's halfword 1 and column 1's halfword 0 inactive: [0, 0] has
    // the first product alone, [0, 1] none, [1, 1] the second alone.
    set_p_bits(&mut cpu, 0, !(1 << 2));
    set_p_bits(&mut cpu, 1, !(1 << 4));
    set_za_words(&mut cpu, 0, &[0; 4]);
    set_za_words(&mut cpu, 4, &[0; 4]);
    assert_eq!(exec(&mut cpu, SMOPA_H), None);
    assert_eq!(za_words(&cpu, 0), [32768, 0, 0, 0]);
    assert_eq!(za_words(&cpu, 4), [32763, 1, 0, 0]);
}

/// MOVA of two registers moves register `r` to or from slice
/// `((UInt(Ws) - UInt(Ws) MOD 2) + offset) MOD dim + r`, as a row or as a
/// column; two doubleword slices fit 128 bits.
#[test]
fn mova_of_two_registers_moves_two_slices_from_an_even_index() {
    let mut cpu = with_za(0);
    set_z_words(&mut cpu, 4, &[1, 2, 3, 4]);
    set_z_words(&mut cpu, 5, &[5, 6, 7, 8]);
    // za0h.b[w12, 2:3] with W12 = 15: (14 + 2) MOD 16 is slice 0, so ZA
    // vectors 0 and 1.
    cpu.x[12] = 15;
    assert_eq!(exec(&mut cpu, MOVA_TO_2_B), None);
    assert_eq!(za_words(&cpu, 0), [1, 2, 3, 4]);
    assert_eq!(za_words(&cpu, 1), [5, 6, 7, 8]);
    // za7h.d[w12, 0:1]: 14 MOD 2 is slice 0, ZA vectors 7 and 15.
    assert_eq!(exec(&mut cpu, MOVA_TO_2_D), None);
    assert_eq!(za_words(&cpu, 7), [1, 2, 3, 4]);
    assert_eq!(za_words(&cpu, 15), [5, 6, 7, 8]);
    // za1v.s[w13, 2:3] with W13 = 1: (0 + 2) MOD 4, so columns 2 and 3 of
    // tile 1's vectors 1, 5, 9 and 13.
    cpu.x[13] = 1;
    assert_eq!(exec(&mut cpu, MOVA_TO_2_S), None);
    assert_eq!(za_words(&cpu, 1), [5, 6, 1, 5]);
    assert_eq!(za_words(&cpu, 5), [0, 0, 2, 6]);
    assert_eq!(za_words(&cpu, 9), [0, 0, 3, 7]);
    assert_eq!(za_words(&cpu, 13), [0, 0, 4, 8]);
    // {z6.h-z7.h} from za1v.h[w12, 6:7]: (14 + 6) MOD 8 is slice 4, so
    // halfwords 4 and 5 of tile 1's vectors 1, 3, ..., 15.
    for v in 0..8u32 {
        set_za_words(&mut cpu, 2 * v as usize + 1, &[0, 0, (v + 1) << 16 | (0x10 + v), 0]);
    }
    assert_eq!(exec(&mut cpu, MOVA_FROM_2_H), None);
    assert_eq!(z_words(&cpu, 6), [0x0011_0010, 0x0013_0012, 0x0015_0014, 0x0017_0016]);
    assert_eq!(z_words(&cpu, 7), [0x0002_0001, 0x0004_0003, 0x0006_0005, 0x0008_0007]);
}

/// MOVA of four registers moves four slices from a multiple of four: as
/// columns, the registers' transpose.
#[test]
fn mova_of_four_registers_moves_four_slices_from_a_multiple_of_four() {
    let mut cpu = with_za(0);
    for r in 0..4u32 {
        set_z_words(&mut cpu, 8 + r, &[4 * r + 1, 4 * r + 2, 4 * r + 3, 4 * r + 4]);
    }
    // za2v.s[w14, 0:3]: columns 0 to 3 of tile 2's vectors 2, 6, 10 and
    // 14, whatever W14.
    cpu.x[14] = 0xFFFF_FFFF_0000_0003;
    assert_eq!(exec(&mut cpu, MOVA_TO_4_S), None);
    assert_eq!(za_words(&cpu, 2), [1, 5, 9, 13]);
    assert_eq!(za_words(&cpu, 6), [2, 6, 10, 14]);
    assert_eq!(za_words(&cpu, 10), [3, 7, 11, 15]);
    assert_eq!(za_words(&cpu, 14), [4, 8, 12, 16]);
    cpu.x[15] = 0xFFFF_FFFF;
    assert_eq!(exec(&mut cpu, MOVA_FROM_4_S), None);
    for r in 0..4 {
        assert_eq!(z_words(&cpu, 4 + r), z_words(&cpu, 8 + r), "Z{}", 4 + r);
    }
    // za0h.b[w12, 4:7] with W12 = 9: (8 + 4) MOD 16 is slice 12, so ZA
    // vectors 12 to 15; {z12.b-z15.b} from za0h.b[w13, 8:11] with W13 = 6:
    // (4 + 8) MOD 16, the same.
    cpu.x[12] = 9;
    assert_eq!(exec(&mut cpu, MOVA_TO_4_B), None);
    for r in 0..4 {
        assert_eq!(za_words(&cpu, 12 + r as usize), z_words(&cpu, 4 + r), "ZA[{}]", 12 + r);
    }
    cpu.x[13] = 6;
    assert_eq!(exec(&mut cpu, MOVA_FROM_4_B), None);
    for r in 0..4 {
        assert_eq!(z_words(&cpu, 12 + r), z_words(&cpu, 4 + r), "Z{}", 12 + r);
    }
}

/// Four doubleword slices need a tile of 256 bits: there they move as the
/// others do, below that vector length they are UNDEFINED after the enable
/// checks, and on a processor whose longest vector length is below it,
/// before them.
#[test]
fn four_doubleword_slices_need_a_256_bit_vector_length() {
    let mut cpu = with_za(1);
    // Tile 6's vectors 6, 14, 22 and 30: word k of vector v is 100v + k.
    let vectors = [6u32, 14, 22, 30];
    for v in vectors {
        let words: Vec<u32> = (0..8).map(|k| 100 * v + k).collect();
        set_za_words(&mut cpu, v as usize, &words);
    }
    // Their columns: Z(8 + c) is doubleword c of each.
    assert_eq!(exec(&mut cpu, MOVA_FROM_4_D), None);
    for c in 0..4u32 {
        let want: Vec<u32> = vectors.iter().flat_map(|&v| [100 * v + 2 * c, 100 * v + 2 * c + 1]).collect();
        assert_eq!(z_words(&cpu, 8 + c), want, "Z{}", 8 + c);
    }
    // As the rows of tile 5, vectors 5, 13, 21 and 29, from W12 = 7: (4 + 0)
    // MOD 4 is slice 0.
    cpu.x[12] = 7;
    assert_eq!(exec(&mut cpu, MOVA_TO_4_D), None);
    for (c, v) in [5, 13, 21, 29].into_iter().enumerate() {
        assert_eq!(za_words(&cpu, v), z_words(&cpu, 8 + c as u32), "ZA[{v}]");
    }

    let mut cpu = processor(Implementation::EVERY_FEATURE);
    assert_eq!(exec(&mut cpu, MOVA_TO_4_D), Some(NOT_STREAMING));
    cpu.set_pstate_sm(true);
    cpu.set_pstate_za(true);
    for insn in [MOVA_TO_4_D, MOVA_FROM_4_D] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
    let mut cpu = processor(Implementation {
        features: Implementation::EVERY_FEATURE.features,
        max_svl: VecLen::MIN,
    });
    for insn in [MOVA_TO_4_D, MOVA_FROM_4_D] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}

/// FEAT_SME2's instructions on tiles take the SME trap outside Streaming
/// SVE mode, then with ZA storage disabled, and are UNDEFINED without
/// FEAT_SME2.
#[test]
fn the_sme2_tile_instructions_need_streaming_mode_za_and_sme2() {
    let all = [
        BMOPA,
        BMOPS,
        SMOPA_H,
        UMOPS_H,
        MOVA_TO_2_B,
        MOVA_FROM_2_H,
        MOVA_TO_4_S,
        MOVA_FROM_4_B,
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
    });
    cpu.set_pstate_sm(true);
    cpu.set_pstate_za(true);
    for insn in all {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}

/// The encodings of other features in the SME space are UNDEFINED, before
/// any SME check: MOVA's `Q = 1` with a `size` other than 11, FMOPA,
/// SMOPA and ADDHA into 64-bit tiles (FEAT_SME_F64F64, FEAT_SME_I16I64),
/// ZERO of ZA array vectors (FEAT_SME2p1) and MOVT from a vector to ZT0
/// (FEAT_SME_LUTv2).
#[test]
fn the_encodings_of_other_features_are_undefined() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for insn in [0xC001_0020, 0x80C0_0000, 0xA0C0_0000, 0xC0D0_0000, 0xC00C_0000, 0xC04F_03E0] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}
