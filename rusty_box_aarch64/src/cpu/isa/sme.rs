//! Arm group `sme` (Arm ARM C4.1, "SME encodings"): the A64 space with
//! `op0 = 1, op1 = 0000`. Like `sve` it puts a level of groups between the
//! top and the classes; Arm names SME's groups and classes `mortlach_*`.
//!
//! The encodings here are all of FEAT_SME's and FEAT_SME2's in this group
//! (`Feature::complete`). FEAT_SME's are the instructions on
//! the ZA array: the outer products into 32-bit tiles (FMOPA and FMOPS of
//! single-precision and of half-precision values, BFMOPA and BFMOPS, and
//! SMOPA to UMOPS of bytes), MOVA between a tile slice and a vector, ZERO,
//! ADDHA and ADDVA of words, the tile slice loads and stores (LD1B to LD1Q,
//! ST1B to ST1Q) and LDR and STR of a ZA array vector. FEAT_SME2's are the
//! contiguous loads and stores of two or four vectors (LD1B to LD1D,
//! LDNT1B to LDNT1D, ST1B to ST1D, STNT1B to STNT1D), ZERO {ZT0}, LDR and
//! STR of ZT0, MOVT between ZT0 and a general register, LUTI2 and LUTI4 of
//! ZT0 to one, two or four vectors, BMOPA and BMOPS, SMOPA to UMOPS of
//! halfwords into 32-bit tiles, MOVA between a group of two or four
//! vectors and as many tile slices or ZA array vectors, the SVE operations
//! on groups of two or four vectors (`mortlach_multi_sve_1` to `_4`: SEL,
//! the integer and floating-point maxima, minima, shifts, additions,
//! multiplications and clamps, ZIP and UZP, the narrowing shifts and
//! conversions, the conversions between words and single precision, FRINT
//! and the unpacks), and the instructions on groups of ZA array vectors
//! (`mortlach_multi_array_1a` to `_2b`: the long integer and floating-point
//! products, the dot products, FMLA and FMLS, and ADD, SUB, FADD and FSUB)
//! and by an indexed element (`mortlach_multi_indexed_1` to `_3`, with the
//! vertical dot products FVDOT, BFVDOT, SVDOT, UVDOT, USVDOT and SUVDOT).
//! The optional 64-bit forms of FEAT_SME_F64F64 and FEAT_SME_I16I64, and the
//! encodings of the later SME extensions, are not here, which this decoder
//! does not know.
//!
//! The variants are Arm's encoding ids, whole (the module documentation of
//! `isa` gives the naming, which `tests/unit/spec.rs` checks); the ids of
//! one class share their operand suffix, so the lint against a shared
//! postfix does not apply here.
#![allow(clippy::enum_variant_names)]

use super::Encoding;
use crate::cpu::exec::bits;

pub(super) fn sme(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 30, 29);
    match op0 {
        0b00 => fp_outer_products(word),
        0b01 => multi_mem_and_int_outer_products(word),
        0b10 => array_and_vectors(word),
        0b11 => mortlach_mem(word),
        _ => None,
    }
}

/// `op0 = 00`: the floating-point outer products, by bits 23:22 and 3:2,
/// and with bits 23:22 = 10 and 3:2 = 10 mortlach2_misc_prod. Bits 23:22
/// = 00 are mortlach2_prod4 (FEAT_SME_MOP4), 01 mortlach2_ss_prod
/// (FEAT_SME_TMOP), 11 the 64-bit products (FEAT_SME_F64F64,
/// FEAT_SME_I16I64).
fn fp_outer_products(word: u32) -> Option<Encoding> {
    match (bits(word, 23, 22), bits(word, 3, 2)) {
        (0b10, 0b00) => mortlach_32bit_fp_prod(word),
        (0b10, 0b10) => mortlach2_misc_prod(word),
        _ => None,
    }
}

/// Arm group `mortlach2_misc_prod`: SME2 binary and FP8 outer products,
/// 32-bit and 16-bit tiles. `op0 = 0, op1 = 1` holds FEAT_SME_F8F16's
/// mortlach_f8f16_prod and FEAT_SME_F16F8F16's mortlach_f16f8_prod; `op0 =
/// 1` FEAT_SME_F16F16's mortlach_f16f16_prod (`op1 = 0`) and
/// FEAT_SME_B16B16's mortlach_b16b16_prod (`op1 = 1`).
fn mortlach2_misc_prod(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 24, 24);
    let op1 = bits(word, 21, 21);
    match (op0, op1) {
        (0b0, 0b0) => mortlach_bini32_prod(word).map(Encoding::MortlachBini32Prod),
        _ => None,
    }
}

/// Arm class `mortlach_bini32_prod`: binary (XNOR and population count) sum
/// of outer products and accumulate or subtract (`S`), into 32-bit tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachBini32Prod {
    BmopaZaPpZz32,
    BmopsZaPpZz32,
}

fn mortlach_bini32_prod(word: u32) -> Option<MortlachBini32Prod> {
    let s = bits(word, 4, 4);
    match s {
        0b0 => Some(MortlachBini32Prod::BmopaZaPpZz32),
        0b1 => Some(MortlachBini32Prod::BmopsZaPpZz32),
        _ => None,
    }
}

/// `op0 = 01`: the multi-vector loads and stores by bits 24:23 (00 to
/// consecutive registers, 10 to strided ones), and the integer outer
/// products by bits 23:22 and 2. Bits 23:22 = 11 are the 64-bit products
/// (FEAT_SME_F64F64, FEAT_SME_I16I64).
fn multi_mem_and_int_outer_products(word: u32) -> Option<Encoding> {
    match (bits(word, 24, 22), bits(word, 2, 2)) {
        (0b000 | 0b001, _) => mortlach_multi_mem_ctg(word),
        (0b100 | 0b101, _) => mortlach_multi_mem_nctg(word),
        (0b010 | 0b110, 0b0) => mortlach_32bit_int_prod(word),
        _ => None,
    }
}

/// `op0 = 10`: the instructions on ZA's tiles and vectors, by bits 21:19
/// with bit 24 clear, and with bit 24 set FEAT_SME2's multi-vector groups:
/// by an indexed element with bit 21 clear (one vector for bit 20 clear,
/// else two or four by bit 15), on ZA array vectors with bit 21 set and
/// bit 15 clear, and the SVE operations with both set. With bit 24 clear,
/// bits 23:19 = 1x001 and 10011 are FEAT_SME2's lookup table expansions
/// (`mortlach_zt_expand_ctg`, `_nctg`), and bits 22:20 = 110
/// FEAT_SME_F16F8F16's scaled products (`mortlach_multivec_scaled_idx`).
fn array_and_vectors(word: u32) -> Option<Encoding> {
    match (bits(word, 24, 24), bits(word, 21, 19)) {
        // Bit 17.
        (0b0, 0b000) => match bits(word, 17, 17) {
            0b0 => match bits(word, 4, 4) {
                0b0 => mortlach_ins(word),
                _ => None,
            },
            0b1 => mortlach_ext(word),
            _ => None,
        },
        // Bits 23:22 and 18: 00 and 1 is mortlach_multizero (FEAT_SME2p1).
        (0b0, 0b001) => match (bits(word, 23, 22), bits(word, 18, 18)) {
            (0b00, 0b0) => mortlach_zero(word).map(Encoding::MortlachZero),
            (0b01, 0b0) => mortlach_zero_zt(word).map(Encoding::MortlachZeroZt),
            (0b01, 0b1) => mortlach_mov_zt(word),
            (0b10 | 0b11, _) => mortlach_zt_expand_ctg(word),
            _ => None,
        },
        (0b0, 0b010) => match bits(word, 3, 3) {
            0b0 => mortlach_hvadd(word),
            _ => None,
        },
        (0b1, 0b000 | 0b001) => mortlach_multi_indexed_1(word),
        (0b1, 0b010 | 0b011) => match bits(word, 15, 15) {
            0b0 => mortlach_multi_indexed_2(word),
            0b1 => mortlach_multi_indexed_3(word),
            _ => None,
        },
        (0b1, 0b100..=0b111) => match bits(word, 15, 15) {
            0b0 => multi_array(word),
            0b1 => multi_vector_sve(word),
            _ => None,
        },
        _ => None,
    }
}

/// Arm group `mortlach_32bit_fp_prod`: SME FP outer product, 32-bit
/// tiles. `op0 = 0, op1 = 1` is mortlach_f8f32_prod (FEAT_SME_F8F32).
fn mortlach_32bit_fp_prod(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 24, 24);
    let op1 = bits(word, 21, 21);
    match (op0, op1) {
        (0b0, 0b0) => mortlach_f32f32_prod(word).map(Encoding::MortlachF32f32Prod),
        (0b1, 0b0) => mortlach_b16f32_prod(word).map(Encoding::MortlachB16f32Prod),
        (0b1, 0b1) => mortlach_f16f32_prod(word).map(Encoding::MortlachF16f32Prod),
        _ => None,
    }
}

/// Arm class `mortlach_f32f32_prod`: single-precision outer product and
/// accumulate or subtract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachF32f32Prod {
    FmopaZaPpZz32,
    FmopsZaPpZz32,
}

fn mortlach_f32f32_prod(word: u32) -> Option<MortlachF32f32Prod> {
    let s = bits(word, 4, 4);
    match s {
        0b0 => Some(MortlachF32f32Prod::FmopaZaPpZz32),
        0b1 => Some(MortlachF32f32Prod::FmopsZaPpZz32),
        _ => None,
    }
}

/// Arm class `mortlach_b16f32_prod`: BFloat16 sum of outer products and
/// accumulate or subtract, into single precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachB16f32Prod {
    BfmopaZa32PpZz,
    BfmopsZa32PpZz,
}

fn mortlach_b16f32_prod(word: u32) -> Option<MortlachB16f32Prod> {
    let s = bits(word, 4, 4);
    match s {
        0b0 => Some(MortlachB16f32Prod::BfmopaZa32PpZz),
        0b1 => Some(MortlachB16f32Prod::BfmopsZa32PpZz),
        _ => None,
    }
}

/// Arm class `mortlach_f16f32_prod`: half-precision sum of outer products
/// and accumulate or subtract, into single precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachF16f32Prod {
    FmopaZa32PpZz16,
    FmopsZa32PpZz16,
}

fn mortlach_f16f32_prod(word: u32) -> Option<MortlachF16f32Prod> {
    let s = bits(word, 4, 4);
    match s {
        0b0 => Some(MortlachF16f32Prod::FmopaZa32PpZz16),
        0b1 => Some(MortlachF16f32Prod::FmopsZa32PpZz16),
        _ => None,
    }
}

/// Arm group `mortlach_32bit_int_prod`: SME integer outer product, 32-bit
/// tiles, of bytes (`op1 = 0`) or FEAT_SME2's of halfwords.
fn mortlach_32bit_int_prod(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 21, 21);
    let op1 = bits(word, 3, 3);
    match (op0, op1) {
        (_, 0b0) => mortlach_i8i32_prod(word).map(Encoding::MortlachI8i32Prod),
        (0b0, 0b1) => mortlach_i16i32_prod(word).map(Encoding::MortlachI16i32Prod),
        _ => None,
    }
}

/// Arm class `mortlach_i16i32_prod`: 16-bit integer sum of outer products
/// and accumulate or subtract (`S`), into 32-bit tiles, of signed or
/// unsigned (`u0`) halfwords.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachI16i32Prod {
    SmopaZa32PpZz16,
    UmopaZa32PpZz16,
    SmopsZa32PpZz16,
    UmopsZa32PpZz16,
}

fn mortlach_i16i32_prod(word: u32) -> Option<MortlachI16i32Prod> {
    let u0 = bits(word, 24, 24);
    let s = bits(word, 4, 4);
    match (u0, s) {
        (0b0, 0b0) => Some(MortlachI16i32Prod::SmopaZa32PpZz16),
        (0b1, 0b0) => Some(MortlachI16i32Prod::UmopaZa32PpZz16),
        (0b0, 0b1) => Some(MortlachI16i32Prod::SmopsZa32PpZz16),
        (0b1, 0b1) => Some(MortlachI16i32Prod::UmopsZa32PpZz16),
        _ => None,
    }
}

/// Arm class `mortlach_i8i32_prod`: 8-bit integer sum of outer products
/// and accumulate or subtract, into 32-bit tiles, of signed or unsigned
/// bytes (`u0` for Zn's, `u1` for Zm's).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachI8i32Prod {
    SmopaZaPpZz32,
    SumopaZaPpZz32,
    UsmopaZaPpZz32,
    UmopaZaPpZz32,
    SmopsZaPpZz32,
    SumopsZaPpZz32,
    UsmopsZaPpZz32,
    UmopsZaPpZz32,
}

fn mortlach_i8i32_prod(word: u32) -> Option<MortlachI8i32Prod> {
    let u0 = bits(word, 24, 24);
    let u1 = bits(word, 21, 21);
    let s = bits(word, 4, 4);
    match (u0, u1, s) {
        (0b0, 0b0, 0b0) => Some(MortlachI8i32Prod::SmopaZaPpZz32),
        (0b0, 0b1, 0b0) => Some(MortlachI8i32Prod::SumopaZaPpZz32),
        (0b1, 0b0, 0b0) => Some(MortlachI8i32Prod::UsmopaZaPpZz32),
        (0b1, 0b1, 0b0) => Some(MortlachI8i32Prod::UmopaZaPpZz32),
        (0b0, 0b0, 0b1) => Some(MortlachI8i32Prod::SmopsZaPpZz32),
        (0b0, 0b1, 0b1) => Some(MortlachI8i32Prod::SumopsZaPpZz32),
        (0b1, 0b0, 0b1) => Some(MortlachI8i32Prod::UsmopsZaPpZz32),
        (0b1, 0b1, 0b1) => Some(MortlachI8i32Prod::UmopsZaPpZz32),
        _ => None,
    }
}

/// Arm group `mortlach_multi_mem_ctg`: SME2 multi-vector contiguous loads
/// and stores to consecutive registers, by op0 (bits 22:20: a load or a
/// store by bit 21, scalar plus scalar or plus immediate by bit 22) and op1
/// (bit 15: two registers or four, which fix op2, bit 1).
fn mortlach_multi_mem_ctg(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 22, 20);
    let op1 = bits(word, 15, 15);
    let op2 = bits(word, 1, 1);
    match (op0, op1, op2) {
        (0b000 | 0b001, 0b0, _) => {
            mortlach_multi2_cld_cldnt_ss_ctg(word).map(Encoding::MortlachMulti2CldCldntSsCtg)
        }
        (0b000 | 0b001, 0b1, 0b0) => {
            mortlach_multi4_cld_cldnt_ss_ctg(word).map(Encoding::MortlachMulti4CldCldntSsCtg)
        }
        (0b010 | 0b011, 0b0, _) => {
            mortlach_multi2_cst_cstnt_ss_ctg(word).map(Encoding::MortlachMulti2CstCstntSsCtg)
        }
        (0b010 | 0b011, 0b1, 0b0) => {
            mortlach_multi4_cst_cstnt_ss_ctg(word).map(Encoding::MortlachMulti4CstCstntSsCtg)
        }
        (0b100, 0b0, _) => {
            mortlach_multi2_cld_cldnt_si_ctg(word).map(Encoding::MortlachMulti2CldCldntSiCtg)
        }
        (0b100, 0b1, 0b0) => {
            mortlach_multi4_cld_cldnt_si_ctg(word).map(Encoding::MortlachMulti4CldCldntSiCtg)
        }
        (0b110, 0b0, _) => {
            mortlach_multi2_cst_cstnt_si_ctg(word).map(Encoding::MortlachMulti2CstCstntSiCtg)
        }
        (0b110, 0b1, 0b0) => {
            mortlach_multi4_cst_cstnt_si_ctg(word).map(Encoding::MortlachMulti4CstCstntSiCtg)
        }
        _ => None,
    }
}

/// Arm group `mortlach_multi_mem_nctg`: SME2 multi-vector contiguous
/// loads and stores to strided registers, as `mortlach_multi_mem_ctg`
/// with four registers fixing op2 at bit 2.
fn mortlach_multi_mem_nctg(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 22, 20);
    let op1 = bits(word, 15, 15);
    let op2 = bits(word, 2, 2);
    match (op0, op1, op2) {
        (0b000 | 0b001, 0b0, _) => {
            mortlach_multi2_cld_cldnt_ss_nctg(word).map(Encoding::MortlachMulti2CldCldntSsNctg)
        }
        (0b000 | 0b001, 0b1, 0b0) => {
            mortlach_multi4_cld_cldnt_ss_nctg(word).map(Encoding::MortlachMulti4CldCldntSsNctg)
        }
        (0b010 | 0b011, 0b0, _) => {
            mortlach_multi2_cst_cstnt_ss_nctg(word).map(Encoding::MortlachMulti2CstCstntSsNctg)
        }
        (0b010 | 0b011, 0b1, 0b0) => {
            mortlach_multi4_cst_cstnt_ss_nctg(word).map(Encoding::MortlachMulti4CstCstntSsNctg)
        }
        (0b100, 0b0, _) => {
            mortlach_multi2_cld_cldnt_si_nctg(word).map(Encoding::MortlachMulti2CldCldntSiNctg)
        }
        (0b100, 0b1, 0b0) => {
            mortlach_multi4_cld_cldnt_si_nctg(word).map(Encoding::MortlachMulti4CldCldntSiNctg)
        }
        (0b110, 0b0, _) => {
            mortlach_multi2_cst_cstnt_si_nctg(word).map(Encoding::MortlachMulti2CstCstntSiNctg)
        }
        (0b110, 0b1, 0b0) => {
            mortlach_multi4_cst_cstnt_si_nctg(word).map(Encoding::MortlachMulti4CstCstntSiNctg)
        }
        _ => None,
    }
}

/// Arm class `mortlach_multi2_cld_cldnt_ss_ctg`: load two consecutive
/// vectors, scalar plus scalar, of the element size `msz`, non-temporal by
/// `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2CldCldntSsCtg {
    Ld1bMzPBr2,
    Ldnt1bMzPBr2,
    Ld1hMzPBr2,
    Ldnt1hMzPBr2,
    Ld1wMzPBr2,
    Ldnt1wMzPBr2,
    Ld1dMzPBr2,
    Ldnt1dMzPBr2,
}

fn mortlach_multi2_cld_cldnt_ss_ctg(word: u32) -> Option<MortlachMulti2CldCldntSsCtg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 0, 0);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti2CldCldntSsCtg::Ld1bMzPBr2),
        (0b00, 0b1) => Some(MortlachMulti2CldCldntSsCtg::Ldnt1bMzPBr2),
        (0b01, 0b0) => Some(MortlachMulti2CldCldntSsCtg::Ld1hMzPBr2),
        (0b01, 0b1) => Some(MortlachMulti2CldCldntSsCtg::Ldnt1hMzPBr2),
        (0b10, 0b0) => Some(MortlachMulti2CldCldntSsCtg::Ld1wMzPBr2),
        (0b10, 0b1) => Some(MortlachMulti2CldCldntSsCtg::Ldnt1wMzPBr2),
        (0b11, 0b0) => Some(MortlachMulti2CldCldntSsCtg::Ld1dMzPBr2),
        (0b11, 0b1) => Some(MortlachMulti2CldCldntSsCtg::Ldnt1dMzPBr2),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_cld_cldnt_ss_ctg`: load four consecutive
/// vectors, scalar plus scalar, of the element size `msz`, non-temporal by
/// `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4CldCldntSsCtg {
    Ld1bMzPBr4,
    Ldnt1bMzPBr4,
    Ld1hMzPBr4,
    Ldnt1hMzPBr4,
    Ld1wMzPBr4,
    Ldnt1wMzPBr4,
    Ld1dMzPBr4,
    Ldnt1dMzPBr4,
}

fn mortlach_multi4_cld_cldnt_ss_ctg(word: u32) -> Option<MortlachMulti4CldCldntSsCtg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 0, 0);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti4CldCldntSsCtg::Ld1bMzPBr4),
        (0b00, 0b1) => Some(MortlachMulti4CldCldntSsCtg::Ldnt1bMzPBr4),
        (0b01, 0b0) => Some(MortlachMulti4CldCldntSsCtg::Ld1hMzPBr4),
        (0b01, 0b1) => Some(MortlachMulti4CldCldntSsCtg::Ldnt1hMzPBr4),
        (0b10, 0b0) => Some(MortlachMulti4CldCldntSsCtg::Ld1wMzPBr4),
        (0b10, 0b1) => Some(MortlachMulti4CldCldntSsCtg::Ldnt1wMzPBr4),
        (0b11, 0b0) => Some(MortlachMulti4CldCldntSsCtg::Ld1dMzPBr4),
        (0b11, 0b1) => Some(MortlachMulti4CldCldntSsCtg::Ldnt1dMzPBr4),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_cst_cstnt_ss_ctg`: store two consecutive
/// vectors, scalar plus scalar, of the element size `msz`, non-temporal by
/// `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2CstCstntSsCtg {
    St1bMzPBr2,
    Stnt1bMzPBr2,
    St1hMzPBr2,
    Stnt1hMzPBr2,
    St1wMzPBr2,
    Stnt1wMzPBr2,
    St1dMzPBr2,
    Stnt1dMzPBr2,
}

fn mortlach_multi2_cst_cstnt_ss_ctg(word: u32) -> Option<MortlachMulti2CstCstntSsCtg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 0, 0);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti2CstCstntSsCtg::St1bMzPBr2),
        (0b00, 0b1) => Some(MortlachMulti2CstCstntSsCtg::Stnt1bMzPBr2),
        (0b01, 0b0) => Some(MortlachMulti2CstCstntSsCtg::St1hMzPBr2),
        (0b01, 0b1) => Some(MortlachMulti2CstCstntSsCtg::Stnt1hMzPBr2),
        (0b10, 0b0) => Some(MortlachMulti2CstCstntSsCtg::St1wMzPBr2),
        (0b10, 0b1) => Some(MortlachMulti2CstCstntSsCtg::Stnt1wMzPBr2),
        (0b11, 0b0) => Some(MortlachMulti2CstCstntSsCtg::St1dMzPBr2),
        (0b11, 0b1) => Some(MortlachMulti2CstCstntSsCtg::Stnt1dMzPBr2),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_cst_cstnt_ss_ctg`: store four consecutive
/// vectors, scalar plus scalar, of the element size `msz`, non-temporal by
/// `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4CstCstntSsCtg {
    St1bMzPBr4,
    Stnt1bMzPBr4,
    St1hMzPBr4,
    Stnt1hMzPBr4,
    St1wMzPBr4,
    Stnt1wMzPBr4,
    St1dMzPBr4,
    Stnt1dMzPBr4,
}

fn mortlach_multi4_cst_cstnt_ss_ctg(word: u32) -> Option<MortlachMulti4CstCstntSsCtg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 0, 0);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti4CstCstntSsCtg::St1bMzPBr4),
        (0b00, 0b1) => Some(MortlachMulti4CstCstntSsCtg::Stnt1bMzPBr4),
        (0b01, 0b0) => Some(MortlachMulti4CstCstntSsCtg::St1hMzPBr4),
        (0b01, 0b1) => Some(MortlachMulti4CstCstntSsCtg::Stnt1hMzPBr4),
        (0b10, 0b0) => Some(MortlachMulti4CstCstntSsCtg::St1wMzPBr4),
        (0b10, 0b1) => Some(MortlachMulti4CstCstntSsCtg::Stnt1wMzPBr4),
        (0b11, 0b0) => Some(MortlachMulti4CstCstntSsCtg::St1dMzPBr4),
        (0b11, 0b1) => Some(MortlachMulti4CstCstntSsCtg::Stnt1dMzPBr4),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_cld_cldnt_si_ctg`: load two consecutive
/// vectors, scalar plus immediate, of the element size `msz`, non-temporal
/// by `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2CldCldntSiCtg {
    Ld1bMzPBi2,
    Ldnt1bMzPBi2,
    Ld1hMzPBi2,
    Ldnt1hMzPBi2,
    Ld1wMzPBi2,
    Ldnt1wMzPBi2,
    Ld1dMzPBi2,
    Ldnt1dMzPBi2,
}

fn mortlach_multi2_cld_cldnt_si_ctg(word: u32) -> Option<MortlachMulti2CldCldntSiCtg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 0, 0);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti2CldCldntSiCtg::Ld1bMzPBi2),
        (0b00, 0b1) => Some(MortlachMulti2CldCldntSiCtg::Ldnt1bMzPBi2),
        (0b01, 0b0) => Some(MortlachMulti2CldCldntSiCtg::Ld1hMzPBi2),
        (0b01, 0b1) => Some(MortlachMulti2CldCldntSiCtg::Ldnt1hMzPBi2),
        (0b10, 0b0) => Some(MortlachMulti2CldCldntSiCtg::Ld1wMzPBi2),
        (0b10, 0b1) => Some(MortlachMulti2CldCldntSiCtg::Ldnt1wMzPBi2),
        (0b11, 0b0) => Some(MortlachMulti2CldCldntSiCtg::Ld1dMzPBi2),
        (0b11, 0b1) => Some(MortlachMulti2CldCldntSiCtg::Ldnt1dMzPBi2),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_cld_cldnt_si_ctg`: load four consecutive
/// vectors, scalar plus immediate, of the element size `msz`, non-temporal
/// by `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4CldCldntSiCtg {
    Ld1bMzPBi4,
    Ldnt1bMzPBi4,
    Ld1hMzPBi4,
    Ldnt1hMzPBi4,
    Ld1wMzPBi4,
    Ldnt1wMzPBi4,
    Ld1dMzPBi4,
    Ldnt1dMzPBi4,
}

fn mortlach_multi4_cld_cldnt_si_ctg(word: u32) -> Option<MortlachMulti4CldCldntSiCtg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 0, 0);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti4CldCldntSiCtg::Ld1bMzPBi4),
        (0b00, 0b1) => Some(MortlachMulti4CldCldntSiCtg::Ldnt1bMzPBi4),
        (0b01, 0b0) => Some(MortlachMulti4CldCldntSiCtg::Ld1hMzPBi4),
        (0b01, 0b1) => Some(MortlachMulti4CldCldntSiCtg::Ldnt1hMzPBi4),
        (0b10, 0b0) => Some(MortlachMulti4CldCldntSiCtg::Ld1wMzPBi4),
        (0b10, 0b1) => Some(MortlachMulti4CldCldntSiCtg::Ldnt1wMzPBi4),
        (0b11, 0b0) => Some(MortlachMulti4CldCldntSiCtg::Ld1dMzPBi4),
        (0b11, 0b1) => Some(MortlachMulti4CldCldntSiCtg::Ldnt1dMzPBi4),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_cst_cstnt_si_ctg`: store two consecutive
/// vectors, scalar plus immediate, of the element size `msz`, non-temporal
/// by `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2CstCstntSiCtg {
    St1bMzPBi2,
    Stnt1bMzPBi2,
    St1hMzPBi2,
    Stnt1hMzPBi2,
    St1wMzPBi2,
    Stnt1wMzPBi2,
    St1dMzPBi2,
    Stnt1dMzPBi2,
}

fn mortlach_multi2_cst_cstnt_si_ctg(word: u32) -> Option<MortlachMulti2CstCstntSiCtg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 0, 0);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti2CstCstntSiCtg::St1bMzPBi2),
        (0b00, 0b1) => Some(MortlachMulti2CstCstntSiCtg::Stnt1bMzPBi2),
        (0b01, 0b0) => Some(MortlachMulti2CstCstntSiCtg::St1hMzPBi2),
        (0b01, 0b1) => Some(MortlachMulti2CstCstntSiCtg::Stnt1hMzPBi2),
        (0b10, 0b0) => Some(MortlachMulti2CstCstntSiCtg::St1wMzPBi2),
        (0b10, 0b1) => Some(MortlachMulti2CstCstntSiCtg::Stnt1wMzPBi2),
        (0b11, 0b0) => Some(MortlachMulti2CstCstntSiCtg::St1dMzPBi2),
        (0b11, 0b1) => Some(MortlachMulti2CstCstntSiCtg::Stnt1dMzPBi2),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_cst_cstnt_si_ctg`: store four consecutive
/// vectors, scalar plus immediate, of the element size `msz`, non-temporal
/// by `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4CstCstntSiCtg {
    St1bMzPBi4,
    Stnt1bMzPBi4,
    St1hMzPBi4,
    Stnt1hMzPBi4,
    St1wMzPBi4,
    Stnt1wMzPBi4,
    St1dMzPBi4,
    Stnt1dMzPBi4,
}

fn mortlach_multi4_cst_cstnt_si_ctg(word: u32) -> Option<MortlachMulti4CstCstntSiCtg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 0, 0);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti4CstCstntSiCtg::St1bMzPBi4),
        (0b00, 0b1) => Some(MortlachMulti4CstCstntSiCtg::Stnt1bMzPBi4),
        (0b01, 0b0) => Some(MortlachMulti4CstCstntSiCtg::St1hMzPBi4),
        (0b01, 0b1) => Some(MortlachMulti4CstCstntSiCtg::Stnt1hMzPBi4),
        (0b10, 0b0) => Some(MortlachMulti4CstCstntSiCtg::St1wMzPBi4),
        (0b10, 0b1) => Some(MortlachMulti4CstCstntSiCtg::Stnt1wMzPBi4),
        (0b11, 0b0) => Some(MortlachMulti4CstCstntSiCtg::St1dMzPBi4),
        (0b11, 0b1) => Some(MortlachMulti4CstCstntSiCtg::Stnt1dMzPBi4),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_cld_cldnt_ss_nctg`: load two strided vectors,
/// scalar plus scalar, of the element size `msz`, non-temporal by `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2CldCldntSsNctg {
    Ld1bMzxPBr2x8,
    Ldnt1bMzxPBr2x8,
    Ld1hMzxPBr2x8,
    Ldnt1hMzxPBr2x8,
    Ld1wMzxPBr2x8,
    Ldnt1wMzxPBr2x8,
    Ld1dMzxPBr2x8,
    Ldnt1dMzxPBr2x8,
}

fn mortlach_multi2_cld_cldnt_ss_nctg(word: u32) -> Option<MortlachMulti2CldCldntSsNctg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 3, 3);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti2CldCldntSsNctg::Ld1bMzxPBr2x8),
        (0b00, 0b1) => Some(MortlachMulti2CldCldntSsNctg::Ldnt1bMzxPBr2x8),
        (0b01, 0b0) => Some(MortlachMulti2CldCldntSsNctg::Ld1hMzxPBr2x8),
        (0b01, 0b1) => Some(MortlachMulti2CldCldntSsNctg::Ldnt1hMzxPBr2x8),
        (0b10, 0b0) => Some(MortlachMulti2CldCldntSsNctg::Ld1wMzxPBr2x8),
        (0b10, 0b1) => Some(MortlachMulti2CldCldntSsNctg::Ldnt1wMzxPBr2x8),
        (0b11, 0b0) => Some(MortlachMulti2CldCldntSsNctg::Ld1dMzxPBr2x8),
        (0b11, 0b1) => Some(MortlachMulti2CldCldntSsNctg::Ldnt1dMzxPBr2x8),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_cld_cldnt_ss_nctg`: load four strided
/// vectors, scalar plus scalar, of the element size `msz`, non-temporal by
/// `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4CldCldntSsNctg {
    Ld1bMzxPBr4x4,
    Ldnt1bMzxPBr4x4,
    Ld1hMzxPBr4x4,
    Ldnt1hMzxPBr4x4,
    Ld1wMzxPBr4x4,
    Ldnt1wMzxPBr4x4,
    Ld1dMzxPBr4x4,
    Ldnt1dMzxPBr4x4,
}

fn mortlach_multi4_cld_cldnt_ss_nctg(word: u32) -> Option<MortlachMulti4CldCldntSsNctg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 3, 3);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti4CldCldntSsNctg::Ld1bMzxPBr4x4),
        (0b00, 0b1) => Some(MortlachMulti4CldCldntSsNctg::Ldnt1bMzxPBr4x4),
        (0b01, 0b0) => Some(MortlachMulti4CldCldntSsNctg::Ld1hMzxPBr4x4),
        (0b01, 0b1) => Some(MortlachMulti4CldCldntSsNctg::Ldnt1hMzxPBr4x4),
        (0b10, 0b0) => Some(MortlachMulti4CldCldntSsNctg::Ld1wMzxPBr4x4),
        (0b10, 0b1) => Some(MortlachMulti4CldCldntSsNctg::Ldnt1wMzxPBr4x4),
        (0b11, 0b0) => Some(MortlachMulti4CldCldntSsNctg::Ld1dMzxPBr4x4),
        (0b11, 0b1) => Some(MortlachMulti4CldCldntSsNctg::Ldnt1dMzxPBr4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_cst_cstnt_ss_nctg`: store two strided
/// vectors, scalar plus scalar, of the element size `msz`, non-temporal by
/// `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2CstCstntSsNctg {
    St1bMzxPBr2x8,
    Stnt1bMzxPBr2x8,
    St1hMzxPBr2x8,
    Stnt1hMzxPBr2x8,
    St1wMzxPBr2x8,
    Stnt1wMzxPBr2x8,
    St1dMzxPBr2x8,
    Stnt1dMzxPBr2x8,
}

fn mortlach_multi2_cst_cstnt_ss_nctg(word: u32) -> Option<MortlachMulti2CstCstntSsNctg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 3, 3);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti2CstCstntSsNctg::St1bMzxPBr2x8),
        (0b00, 0b1) => Some(MortlachMulti2CstCstntSsNctg::Stnt1bMzxPBr2x8),
        (0b01, 0b0) => Some(MortlachMulti2CstCstntSsNctg::St1hMzxPBr2x8),
        (0b01, 0b1) => Some(MortlachMulti2CstCstntSsNctg::Stnt1hMzxPBr2x8),
        (0b10, 0b0) => Some(MortlachMulti2CstCstntSsNctg::St1wMzxPBr2x8),
        (0b10, 0b1) => Some(MortlachMulti2CstCstntSsNctg::Stnt1wMzxPBr2x8),
        (0b11, 0b0) => Some(MortlachMulti2CstCstntSsNctg::St1dMzxPBr2x8),
        (0b11, 0b1) => Some(MortlachMulti2CstCstntSsNctg::Stnt1dMzxPBr2x8),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_cst_cstnt_ss_nctg`: store four strided
/// vectors, scalar plus scalar, of the element size `msz`, non-temporal by
/// `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4CstCstntSsNctg {
    St1bMzxPBr4x4,
    Stnt1bMzxPBr4x4,
    St1hMzxPBr4x4,
    Stnt1hMzxPBr4x4,
    St1wMzxPBr4x4,
    Stnt1wMzxPBr4x4,
    St1dMzxPBr4x4,
    Stnt1dMzxPBr4x4,
}

fn mortlach_multi4_cst_cstnt_ss_nctg(word: u32) -> Option<MortlachMulti4CstCstntSsNctg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 3, 3);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti4CstCstntSsNctg::St1bMzxPBr4x4),
        (0b00, 0b1) => Some(MortlachMulti4CstCstntSsNctg::Stnt1bMzxPBr4x4),
        (0b01, 0b0) => Some(MortlachMulti4CstCstntSsNctg::St1hMzxPBr4x4),
        (0b01, 0b1) => Some(MortlachMulti4CstCstntSsNctg::Stnt1hMzxPBr4x4),
        (0b10, 0b0) => Some(MortlachMulti4CstCstntSsNctg::St1wMzxPBr4x4),
        (0b10, 0b1) => Some(MortlachMulti4CstCstntSsNctg::Stnt1wMzxPBr4x4),
        (0b11, 0b0) => Some(MortlachMulti4CstCstntSsNctg::St1dMzxPBr4x4),
        (0b11, 0b1) => Some(MortlachMulti4CstCstntSsNctg::Stnt1dMzxPBr4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_cld_cldnt_si_nctg`: load two strided vectors,
/// scalar plus immediate, of the element size `msz`, non-temporal by `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2CldCldntSiNctg {
    Ld1bMzxPBi2x8,
    Ldnt1bMzxPBi2x8,
    Ld1hMzxPBi2x8,
    Ldnt1hMzxPBi2x8,
    Ld1wMzxPBi2x8,
    Ldnt1wMzxPBi2x8,
    Ld1dMzxPBi2x8,
    Ldnt1dMzxPBi2x8,
}

fn mortlach_multi2_cld_cldnt_si_nctg(word: u32) -> Option<MortlachMulti2CldCldntSiNctg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 3, 3);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti2CldCldntSiNctg::Ld1bMzxPBi2x8),
        (0b00, 0b1) => Some(MortlachMulti2CldCldntSiNctg::Ldnt1bMzxPBi2x8),
        (0b01, 0b0) => Some(MortlachMulti2CldCldntSiNctg::Ld1hMzxPBi2x8),
        (0b01, 0b1) => Some(MortlachMulti2CldCldntSiNctg::Ldnt1hMzxPBi2x8),
        (0b10, 0b0) => Some(MortlachMulti2CldCldntSiNctg::Ld1wMzxPBi2x8),
        (0b10, 0b1) => Some(MortlachMulti2CldCldntSiNctg::Ldnt1wMzxPBi2x8),
        (0b11, 0b0) => Some(MortlachMulti2CldCldntSiNctg::Ld1dMzxPBi2x8),
        (0b11, 0b1) => Some(MortlachMulti2CldCldntSiNctg::Ldnt1dMzxPBi2x8),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_cld_cldnt_si_nctg`: load four strided
/// vectors, scalar plus immediate, of the element size `msz`, non-temporal
/// by `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4CldCldntSiNctg {
    Ld1bMzxPBi4x4,
    Ldnt1bMzxPBi4x4,
    Ld1hMzxPBi4x4,
    Ldnt1hMzxPBi4x4,
    Ld1wMzxPBi4x4,
    Ldnt1wMzxPBi4x4,
    Ld1dMzxPBi4x4,
    Ldnt1dMzxPBi4x4,
}

fn mortlach_multi4_cld_cldnt_si_nctg(word: u32) -> Option<MortlachMulti4CldCldntSiNctg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 3, 3);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti4CldCldntSiNctg::Ld1bMzxPBi4x4),
        (0b00, 0b1) => Some(MortlachMulti4CldCldntSiNctg::Ldnt1bMzxPBi4x4),
        (0b01, 0b0) => Some(MortlachMulti4CldCldntSiNctg::Ld1hMzxPBi4x4),
        (0b01, 0b1) => Some(MortlachMulti4CldCldntSiNctg::Ldnt1hMzxPBi4x4),
        (0b10, 0b0) => Some(MortlachMulti4CldCldntSiNctg::Ld1wMzxPBi4x4),
        (0b10, 0b1) => Some(MortlachMulti4CldCldntSiNctg::Ldnt1wMzxPBi4x4),
        (0b11, 0b0) => Some(MortlachMulti4CldCldntSiNctg::Ld1dMzxPBi4x4),
        (0b11, 0b1) => Some(MortlachMulti4CldCldntSiNctg::Ldnt1dMzxPBi4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_cst_cstnt_si_nctg`: store two strided
/// vectors, scalar plus immediate, of the element size `msz`, non-temporal
/// by `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2CstCstntSiNctg {
    St1bMzxPBi2x8,
    Stnt1bMzxPBi2x8,
    St1hMzxPBi2x8,
    Stnt1hMzxPBi2x8,
    St1wMzxPBi2x8,
    Stnt1wMzxPBi2x8,
    St1dMzxPBi2x8,
    Stnt1dMzxPBi2x8,
}

fn mortlach_multi2_cst_cstnt_si_nctg(word: u32) -> Option<MortlachMulti2CstCstntSiNctg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 3, 3);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti2CstCstntSiNctg::St1bMzxPBi2x8),
        (0b00, 0b1) => Some(MortlachMulti2CstCstntSiNctg::Stnt1bMzxPBi2x8),
        (0b01, 0b0) => Some(MortlachMulti2CstCstntSiNctg::St1hMzxPBi2x8),
        (0b01, 0b1) => Some(MortlachMulti2CstCstntSiNctg::Stnt1hMzxPBi2x8),
        (0b10, 0b0) => Some(MortlachMulti2CstCstntSiNctg::St1wMzxPBi2x8),
        (0b10, 0b1) => Some(MortlachMulti2CstCstntSiNctg::Stnt1wMzxPBi2x8),
        (0b11, 0b0) => Some(MortlachMulti2CstCstntSiNctg::St1dMzxPBi2x8),
        (0b11, 0b1) => Some(MortlachMulti2CstCstntSiNctg::Stnt1dMzxPBi2x8),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_cst_cstnt_si_nctg`: store four strided
/// vectors, scalar plus immediate, of the element size `msz`, non-temporal
/// by `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4CstCstntSiNctg {
    St1bMzxPBi4x4,
    Stnt1bMzxPBi4x4,
    St1hMzxPBi4x4,
    Stnt1hMzxPBi4x4,
    St1wMzxPBi4x4,
    Stnt1wMzxPBi4x4,
    St1dMzxPBi4x4,
    Stnt1dMzxPBi4x4,
}

fn mortlach_multi4_cst_cstnt_si_nctg(word: u32) -> Option<MortlachMulti4CstCstntSiNctg> {
    let msz = bits(word, 14, 13);
    let n = bits(word, 3, 3);
    match (msz, n) {
        (0b00, 0b0) => Some(MortlachMulti4CstCstntSiNctg::St1bMzxPBi4x4),
        (0b00, 0b1) => Some(MortlachMulti4CstCstntSiNctg::Stnt1bMzxPBi4x4),
        (0b01, 0b0) => Some(MortlachMulti4CstCstntSiNctg::St1hMzxPBi4x4),
        (0b01, 0b1) => Some(MortlachMulti4CstCstntSiNctg::Stnt1hMzxPBi4x4),
        (0b10, 0b0) => Some(MortlachMulti4CstCstntSiNctg::St1wMzxPBi4x4),
        (0b10, 0b1) => Some(MortlachMulti4CstCstntSiNctg::Stnt1wMzxPBi4x4),
        (0b11, 0b0) => Some(MortlachMulti4CstCstntSiNctg::St1dMzxPBi4x4),
        (0b11, 0b1) => Some(MortlachMulti4CstCstntSiNctg::Stnt1dMzxPBi4x4),
        _ => None,
    }
}

/// Arm group `mortlach_ins`: SME move into ZA, of one vector predicated
/// (`op1 = 0`) or FEAT_SME2's of two or four vectors to tile slices or ZA
/// array vectors. Arm allocates nothing else in the group.
fn mortlach_ins(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 23, 22);
    let op1 = bits(word, 18, 18);
    let op2 = bits(word, 16, 15);
    let op3 = bits(word, 12, 10);
    let op4 = bits(word, 6, 5);
    let op5 = bits(word, 3, 3);
    match (op0, op1, op2, op3, op4, op5) {
        (_, 0b0, _, _, _, _) => mortlach_insert_pred(word).map(Encoding::MortlachInsertPred),
        (_, 0b1, 0b00 | 0b01, 0b000, 0b00 | 0b10, 0b0) => {
            mortlach_multi2_insert_ctg(word).map(Encoding::MortlachMulti2InsertCtg)
        }
        (_, 0b1, 0b00 | 0b01, 0b001, 0b00, 0b0) => {
            mortlach_multi4_insert_ctg(word).map(Encoding::MortlachMulti4InsertCtg)
        }
        (0b00, 0b1, 0b00, 0b010, 0b00 | 0b10, 0b0) => {
            mortlach_multi2_za_insert_ctg(word).map(Encoding::MortlachMulti2ZaInsertCtg)
        }
        (0b00, 0b1, 0b00, 0b011, 0b00, 0b0) => {
            mortlach_multi4_za_insert_ctg(word).map(Encoding::MortlachMulti4ZaInsertCtg)
        }
        _ => None,
    }
}

/// Arm class `mortlach_multi2_insert_ctg`: move two vectors into two
/// consecutive horizontal or vertical tile slices (`V`) of the element size
/// `size`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2InsertCtg {
    MovaZa2ZB1,
    MovaZa2ZH1,
    MovaZa2ZW1,
    MovaZa2ZD1,
}

fn mortlach_multi2_insert_ctg(word: u32) -> Option<MortlachMulti2InsertCtg> {
    let size = bits(word, 23, 22);
    match size {
        0b00 => Some(MortlachMulti2InsertCtg::MovaZa2ZB1),
        0b01 => Some(MortlachMulti2InsertCtg::MovaZa2ZH1),
        0b10 => Some(MortlachMulti2InsertCtg::MovaZa2ZW1),
        0b11 => Some(MortlachMulti2InsertCtg::MovaZa2ZD1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_insert_ctg`: move four vectors into four
/// consecutive horizontal or vertical tile slices (`V`) of the element size
/// `size`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4InsertCtg {
    MovaZa4ZB1,
    MovaZa4ZH1,
    MovaZa4ZW1,
    MovaZa4ZD1,
}

fn mortlach_multi4_insert_ctg(word: u32) -> Option<MortlachMulti4InsertCtg> {
    let size = bits(word, 23, 22);
    let opc = bits(word, 2, 0);
    match (size, opc) {
        (0b00, 0b000..=0b011) => Some(MortlachMulti4InsertCtg::MovaZa4ZB1),
        (0b01, 0b000..=0b011) => Some(MortlachMulti4InsertCtg::MovaZa4ZH1),
        (0b10, 0b000..=0b011) => Some(MortlachMulti4InsertCtg::MovaZa4ZW1),
        (0b11, _) => Some(MortlachMulti4InsertCtg::MovaZa4ZD1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_za_insert_ctg`: move two vectors into ZA
/// array vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZaInsertCtg {
    MovaZaMz21,
}

fn mortlach_multi2_za_insert_ctg(_word: u32) -> Option<MortlachMulti2ZaInsertCtg> {
    Some(MortlachMulti2ZaInsertCtg::MovaZaMz21)
}

/// Arm class `mortlach_multi4_za_insert_ctg`: move four vectors into ZA
/// array vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZaInsertCtg {
    MovaZaMz41,
}

fn mortlach_multi4_za_insert_ctg(_word: u32) -> Option<MortlachMulti4ZaInsertCtg> {
    Some(MortlachMulti4ZaInsertCtg::MovaZaMz41)
}

/// Arm class `mortlach_insert_pred`: move a vector into a horizontal or
/// vertical tile slice (`V`), predicated, of the element size `size` and
/// `Q` select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachInsertPred {
    MovaZaPRzB,
    MovaZaPRzH,
    MovaZaPRzW,
    MovaZaPRzD,
    MovaZaPRzQ,
}

fn mortlach_insert_pred(word: u32) -> Option<MortlachInsertPred> {
    let size = bits(word, 23, 22);
    let q = bits(word, 16, 16);
    match (size, q) {
        (0b00, 0b0) => Some(MortlachInsertPred::MovaZaPRzB),
        (0b01, 0b0) => Some(MortlachInsertPred::MovaZaPRzH),
        (0b10, 0b0) => Some(MortlachInsertPred::MovaZaPRzW),
        (0b11, 0b0) => Some(MortlachInsertPred::MovaZaPRzD),
        (0b11, 0b1) => Some(MortlachInsertPred::MovaZaPRzQ),
        _ => None,
    }
}

/// Arm group `mortlach_ext`: SME move from ZA, of one vector predicated
/// (`op1 = 0`) or FEAT_SME2's of two or four vectors from tile slices or
/// ZA array vectors. FEAT_SME2p1's MOVAZ is `op4 = 1x` with `op1 = 0`
/// (mortlach_extract_zero) and `op4 = 10` in the places of FEAT_SME2's
/// classes of groups (the `*_extract_zero` ones); Arm allocates nothing
/// else in the group.
fn mortlach_ext(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 23, 22);
    let op1 = bits(word, 18, 18);
    let op2 = bits(word, 16, 15);
    let op3 = bits(word, 12, 10);
    let op4 = bits(word, 9, 8);
    let op5 = bits(word, 1, 0);
    match (op0, op1, op2, op3, op4, op5) {
        (_, 0b0, _, _, 0b00 | 0b01, _) => {
            mortlach_extract_pred(word).map(Encoding::MortlachExtractPred)
        }
        (_, 0b1, 0b00 | 0b01, 0b000, 0b00, 0b00 | 0b10) => {
            mortlach_multi2_extract_ctg(word).map(Encoding::MortlachMulti2ExtractCtg)
        }
        (_, 0b1, 0b00 | 0b01, 0b001, 0b00, 0b00) => {
            mortlach_multi4_extract_ctg(word).map(Encoding::MortlachMulti4ExtractCtg)
        }
        (0b00, 0b1, 0b00, 0b010, 0b00, 0b00 | 0b10) => {
            mortlach_multi2_za_extract_ctg(word).map(Encoding::MortlachMulti2ZaExtractCtg)
        }
        (0b00, 0b1, 0b00, 0b011, 0b00, 0b00) => {
            mortlach_multi4_za_extract_ctg(word).map(Encoding::MortlachMulti4ZaExtractCtg)
        }
        _ => None,
    }
}

/// Arm class `mortlach_multi2_extract_ctg`: move two consecutive horizontal
/// or vertical tile slices (`V`) of the element size `size` into two
/// vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ExtractCtg {
    MovaMz2ZaB1,
    MovaMz2ZaH1,
    MovaMz2ZaW1,
    MovaMz2ZaD1,
}

fn mortlach_multi2_extract_ctg(word: u32) -> Option<MortlachMulti2ExtractCtg> {
    let size = bits(word, 23, 22);
    match size {
        0b00 => Some(MortlachMulti2ExtractCtg::MovaMz2ZaB1),
        0b01 => Some(MortlachMulti2ExtractCtg::MovaMz2ZaH1),
        0b10 => Some(MortlachMulti2ExtractCtg::MovaMz2ZaW1),
        0b11 => Some(MortlachMulti2ExtractCtg::MovaMz2ZaD1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_extract_ctg`: move four consecutive
/// horizontal or vertical tile slices (`V`) of the element size `size` into
/// four vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ExtractCtg {
    MovaMz4ZaB1,
    MovaMz4ZaH1,
    MovaMz4ZaW1,
    MovaMz4ZaD1,
}

fn mortlach_multi4_extract_ctg(word: u32) -> Option<MortlachMulti4ExtractCtg> {
    let size = bits(word, 23, 22);
    let opc = bits(word, 7, 5);
    match (size, opc) {
        (0b00, 0b000..=0b011) => Some(MortlachMulti4ExtractCtg::MovaMz4ZaB1),
        (0b01, 0b000..=0b011) => Some(MortlachMulti4ExtractCtg::MovaMz4ZaH1),
        (0b10, 0b000..=0b011) => Some(MortlachMulti4ExtractCtg::MovaMz4ZaW1),
        (0b11, _) => Some(MortlachMulti4ExtractCtg::MovaMz4ZaD1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_za_extract_ctg`: move ZA array vectors into
/// two vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZaExtractCtg {
    MovaMzZa21,
}

fn mortlach_multi2_za_extract_ctg(_word: u32) -> Option<MortlachMulti2ZaExtractCtg> {
    Some(MortlachMulti2ZaExtractCtg::MovaMzZa21)
}

/// Arm class `mortlach_multi4_za_extract_ctg`: move ZA array vectors into
/// four vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZaExtractCtg {
    MovaMzZa41,
}

fn mortlach_multi4_za_extract_ctg(_word: u32) -> Option<MortlachMulti4ZaExtractCtg> {
    Some(MortlachMulti4ZaExtractCtg::MovaMzZa41)
}

/// Arm class `mortlach_extract_pred`: move a horizontal or vertical tile
/// slice (`V`) into a vector, predicated, of the element size `size` and
/// `Q` select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachExtractPred {
    MovaZPRzaB,
    MovaZPRzaH,
    MovaZPRzaW,
    MovaZPRzaD,
    MovaZPRzaQ,
}

fn mortlach_extract_pred(word: u32) -> Option<MortlachExtractPred> {
    let size = bits(word, 23, 22);
    let q = bits(word, 16, 16);
    match (size, q) {
        (0b00, 0b0) => Some(MortlachExtractPred::MovaZPRzaB),
        (0b01, 0b0) => Some(MortlachExtractPred::MovaZPRzaH),
        (0b10, 0b0) => Some(MortlachExtractPred::MovaZPRzaW),
        (0b11, 0b0) => Some(MortlachExtractPred::MovaZPRzaD),
        (0b11, 0b1) => Some(MortlachExtractPred::MovaZPRzaQ),
        _ => None,
    }
}

/// Arm group `mortlach_zero`: SME zero tiles, the one class of its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachZero {
    ZeroZaI,
}

fn mortlach_zero(word: u32) -> Option<MortlachZero> {
    let op0 = bits(word, 17, 8);
    match op0 {
        0b0000000000 => Some(MortlachZero::ZeroZaI),
        _ => None,
    }
}

/// Arm group `mortlach_zero_zt`: SME2 zero ZT0, the one class of its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachZeroZt {
    ZeroZtI,
}

fn mortlach_zero_zt(word: u32) -> Option<MortlachZeroZt> {
    let op0 = bits(word, 17, 4);
    let opc = bits(word, 3, 0);
    match (op0, opc) {
        (0b00000000000000, 0b0001) => Some(MortlachZeroZt::ZeroZtI),
        _ => None,
    }
}

/// Arm group `mortlach_zt_expand_ctg`: SME2 lookup table expansions from
/// ZT0 into one vector (`op0 = 1`) or consecutive vectors: two for `op1 =
/// xxxx1` (bit 14), four for `xxx10` (bits 15:14). With `op0 = 0`, `op1 =
/// 01100` and `op2 = 0` is FEAT_SME_LUTv2's mortlach_expand_4dst2src_ctg,
/// and `op1 = 01000`, `op4 = 0` and `op2 = 0` FEAT_SME2p3's
/// mortlach_expand_4dst3src_ctg (AARCHMRS: the A64 ISA XML of the same
/// release has no FEAT_SME2p3 and leaves those words unallocated).
fn mortlach_zt_expand_ctg(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 22, 22);
    let op3 = bits(word, 1, 0);
    match (op0, bits(word, 15, 14), op3) {
        (0b1, _, _) => mortlach_expand_1dst(word).map(Encoding::MortlachExpand1dst),
        (0b0, 0b01 | 0b11, 0b00 | 0b10) => {
            mortlach_expand_2dst_ctg(word).map(Encoding::MortlachExpand2dstCtg)
        }
        (0b0, 0b10, 0b00) => mortlach_expand_4dst_ctg(word).map(Encoding::MortlachExpand4dstCtg),
        _ => None,
    }
}

/// Arm class `mortlach_expand_1dst`: LUTI2 and LUTI4 (`opc`): one vector of
/// the elements of ZT0 its packed indexes name. `opc = 00001` with `size =
/// 00` is FEAT_SME2p3's LUTI6 (in AARCHMRS, as the group's other
/// FEAT_SME2p3 class).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachExpand1dst {
    Luti2ZZtz,
    Luti4ZZtz,
}

fn mortlach_expand_1dst(word: u32) -> Option<MortlachExpand1dst> {
    let opc = bits(word, 18, 14);
    let opc2 = bits(word, 11, 10);
    match (opc, opc2) {
        (0b10000..=0b11111, 0b00) => Some(MortlachExpand1dst::Luti2ZZtz),
        (0b01000..=0b01111, 0b00) => Some(MortlachExpand1dst::Luti4ZZtz),
        _ => None,
    }
}

/// Arm class `mortlach_expand_2dst_ctg`: LUTI2 and LUTI4 (`opc`): two
/// consecutive vectors of the elements of ZT0 their packed indexes name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachExpand2dstCtg {
    Luti2Mz2Ztz1,
    Luti4Mz2Ztz1,
}

fn mortlach_expand_2dst_ctg(word: u32) -> Option<MortlachExpand2dstCtg> {
    let opc = bits(word, 18, 15);
    let opc2 = bits(word, 11, 10);
    match (opc, opc2) {
        (0b1000..=0b1111, 0b00) => Some(MortlachExpand2dstCtg::Luti2Mz2Ztz1),
        (0b0100..=0b0111, 0b00) => Some(MortlachExpand2dstCtg::Luti4Mz2Ztz1),
        _ => None,
    }
}

/// Arm class `mortlach_expand_4dst_ctg`: LUTI2 and LUTI4 (`opc`): four
/// consecutive vectors of the elements of ZT0 their packed indexes name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachExpand4dstCtg {
    Luti2Mz4Ztz1,
    Luti4Mz4Ztz1,
}

fn mortlach_expand_4dst_ctg(word: u32) -> Option<MortlachExpand4dstCtg> {
    let opc = bits(word, 18, 16);
    let opc2 = bits(word, 11, 10);
    match (opc, opc2) {
        (0b100..=0b111, 0b00) => Some(MortlachExpand4dstCtg::Luti2Mz4Ztz1),
        (0b010..=0b011, 0b00) => Some(MortlachExpand4dstCtg::Luti4Mz4Ztz1),
        _ => None,
    }
}

/// Arm group `mortlach_mov_zt`: SME2 move into or out of ZT0, by op0 (bit
/// 17) and op1 (bits 16:15). `op0 = 1, op1 = 10` is mortlach_move_to_zt
/// (FEAT_SME_LUTv2).
fn mortlach_mov_zt(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 17, 17);
    let op1 = bits(word, 16, 15);
    match (op0, op1) {
        (0b0, 0b00) => mortlach_extract_zt(word).map(Encoding::MortlachExtractZt),
        (0b1, 0b00) => mortlach_insert_zt(word).map(Encoding::MortlachInsertZt),
        _ => None,
    }
}

/// Arm class `mortlach_extract_zt`: move a doubleword of ZT0 to a general
/// register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachExtractZt {
    MovtRZt,
}

fn mortlach_extract_zt(word: u32) -> Option<MortlachExtractZt> {
    let opc = bits(word, 11, 5);
    match opc {
        0b0011111 => Some(MortlachExtractZt::MovtRZt),
        _ => None,
    }
}

/// Arm class `mortlach_insert_zt`: move a general register to a doubleword
/// of ZT0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachInsertZt {
    MovtZtR,
}

fn mortlach_insert_zt(word: u32) -> Option<MortlachInsertZt> {
    let opc = bits(word, 11, 5);
    match opc {
        0b0011111 => Some(MortlachInsertZt::MovtZtR),
        _ => None,
    }
}

/// Arm group `mortlach_hvadd`: SME add vector to array. `op0 = 1, op1 =
/// 00, op2 = 0` is its one class.
fn mortlach_hvadd(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 23, 23);
    let op1 = bits(word, 18, 17);
    let op2 = bits(word, 4, 4);
    match (op0, op1, op2) {
        (0b1, 0b00, 0b0) => mortlach_addhv(word).map(Encoding::MortlachAddhv),
        _ => None,
    }
}

/// Arm class `mortlach_addhv`: add a vector to each horizontal slice or
/// each vertical slice (`V`) of a 32-bit tile. `op = 1` is the 64-bit
/// forms (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachAddhv {
    AddhaZaPpZ32,
    AddvaZaPpZ32,
}

fn mortlach_addhv(word: u32) -> Option<MortlachAddhv> {
    let op = bits(word, 22, 22);
    let v = bits(word, 16, 16);
    match (op, v, bits(word, 2, 2)) {
        (0b0, 0b0, 0b0) => Some(MortlachAddhv::AddhaZaPpZ32),
        (0b0, 0b1, 0b0) => Some(MortlachAddhv::AddvaZaPpZ32),
        _ => None,
    }
}

/// Bits 24, 21 and 15 set: FEAT_SME2's multi-vector SVE groups, by bits
/// 15:13 and 12:10, and the bits each group fixes beyond them. Bits 15:10
/// = 111001 and 111010 are mortlach_multi_sve_5a and _5b, 1111xx
/// mortlach_multi_sve_6.
fn multi_vector_sve(word: u32) -> Option<Encoding> {
    match (bits(word, 15, 13), bits(word, 12, 10)) {
        (0b100, _) => mortlach_multi_sve_1(word),
        // Bit 20 clear, and for four vectors bit 1.
        (0b101, 0b000 | 0b001) => match bits(word, 20, 20) {
            0b0 => mortlach_multi_sve_2a(word),
            _ => None,
        },
        (0b101, 0b010 | 0b011) => match (bits(word, 20, 20), bits(word, 1, 1)) {
            (0b0, 0b0) => mortlach_multi_sve_2b(word),
            _ => None,
        },
        // Bit 16 clear, and for four vectors bits 17:16 and 1.
        (0b101, 0b100) => match bits(word, 16, 16) {
            0b0 => mortlach_multi_sve_2c0(word),
            _ => None,
        },
        (0b101, 0b101) => match bits(word, 16, 16) {
            0b0 => mortlach_multi_sve_2c1(word),
            _ => None,
        },
        (0b101, 0b110) => match (bits(word, 17, 16), bits(word, 1, 1)) {
            (0b00, 0b0) => mortlach_multi_sve_2d0(word),
            _ => None,
        },
        (0b101, 0b111) => match (bits(word, 17, 16), bits(word, 1, 1)) {
            (0b00, 0b0) => mortlach_multi_sve_2d1(word),
            _ => None,
        },
        (0b110, _) => mortlach_multi_sve_3(word),
        (0b111, 0b000) => mortlach_multi_sve_4(word),
        _ => None,
    }
}

/// Arm group `mortlach_multi_sve_1`: SME2 multi-vector select, of two
/// vectors or four.
fn mortlach_multi_sve_1(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 17, 16);
    let op1 = bits(word, 6, 5);
    let op2 = bits(word, 1, 0);
    match (op0, op1, op2) {
        (0b00 | 0b10, 0b00 | 0b10, 0b00 | 0b10) => {
            mortlach_multi2_select_int(word).map(Encoding::MortlachMulti2SelectInt)
        }
        (0b01, 0b00, 0b00) => {
            mortlach_multi4_select_int(word).map(Encoding::MortlachMulti4SelectInt)
        }
        _ => None,
    }
}

/// Arm group `mortlach_multi_sve_2a`: SME2 operations on two vectors and
/// one. `op1 = 01, op2 = 100` is mortlach_multi2_z_z_fscale_sm (FEAT_FP8,
/// FEAT_SVE_BFSCALE).
fn mortlach_multi_sve_2a(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 10, 10);
    let op1 = bits(word, 9, 8);
    let op2 = bits(word, 7, 5);
    match (op0, op1, op2) {
        (0b0, 0b00, 0b000 | 0b001) => {
            mortlach_multi2_z_z_minmax_sm(word).map(Encoding::MortlachMulti2ZZMinmaxSm)
        }
        (0b0, 0b01, 0b000 | 0b001) => {
            mortlach_multi2_z_z_fminmax_sm(word).map(Encoding::MortlachMulti2ZZFminmaxSm)
        }
        (0b0, 0b10, _) => mortlach_multi2_z_z_shift_sm(word).map(Encoding::MortlachMulti2ZZShiftSm),
        (0b0, 0b11, 0b000) => mortlach_multi2_z_z_add_sm(word).map(Encoding::MortlachMulti2ZZAddSm),
        (0b1, 0b00, 0b000) => {
            mortlach_multi2_z_z_sqdmulh_sm(word).map(Encoding::MortlachMulti2ZZSqdmulhSm)
        }
        _ => None,
    }
}

/// Arm group `mortlach_multi_sve_2b`: SME2 operations on four vectors and
/// one, as `mortlach_multi_sve_2a`.
fn mortlach_multi_sve_2b(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 10, 10);
    let op1 = bits(word, 9, 8);
    let op2 = bits(word, 7, 5);
    match (op0, op1, op2) {
        (0b0, 0b00, 0b000 | 0b001) => {
            mortlach_multi4_z_z_minmax_sm(word).map(Encoding::MortlachMulti4ZZMinmaxSm)
        }
        (0b0, 0b01, 0b000 | 0b001) => {
            mortlach_multi4_z_z_fminmax_sm(word).map(Encoding::MortlachMulti4ZZFminmaxSm)
        }
        (0b0, 0b10, _) => mortlach_multi4_z_z_shift_sm(word).map(Encoding::MortlachMulti4ZZShiftSm),
        (0b0, 0b11, 0b000) => mortlach_multi4_z_z_add_sm(word).map(Encoding::MortlachMulti4ZZAddSm),
        (0b1, 0b00, 0b000) => {
            mortlach_multi4_z_z_sqdmulh_sm(word).map(Encoding::MortlachMulti4ZZSqdmulhSm)
        }
        _ => None,
    }
}

/// Arm group `mortlach_multi_sve_2c0`: SME2 operations on two vectors and
/// two. `op0 = 011` is mortlach_multi2_z_z_fscale_mm (FEAT_FP8,
/// FEAT_SVE_BFSCALE).
fn mortlach_multi_sve_2c0(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 9, 7);
    match op0 {
        0b000 => mortlach_multi2_z_z_minmax_mm(word).map(Encoding::MortlachMulti2ZZMinmaxMm),
        0b010 => mortlach_multi2_z_z_fminmax_mm(word).map(Encoding::MortlachMulti2ZZFminmaxMm),
        0b100 | 0b101 => mortlach_multi2_z_z_shift_mm(word).map(Encoding::MortlachMulti2ZZShiftMm),
        _ => None,
    }
}

/// Arm group `mortlach_multi_sve_2c1`: SME2 saturating doubling multiply
/// high of two vectors by two.
fn mortlach_multi_sve_2c1(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 9, 5);
    match op0 {
        0b00000 => mortlach_multi2_z_z_sqdmulh_mm(word).map(Encoding::MortlachMulti2ZZSqdmulhMm),
        _ => None,
    }
}

/// Arm group `mortlach_multi_sve_2d0`: SME2 operations on four vectors and
/// four, as `mortlach_multi_sve_2c0`.
fn mortlach_multi_sve_2d0(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 9, 7);
    match op0 {
        0b000 => mortlach_multi4_z_z_minmax_mm(word).map(Encoding::MortlachMulti4ZZMinmaxMm),
        0b010 => mortlach_multi4_z_z_fminmax_mm(word).map(Encoding::MortlachMulti4ZZFminmaxMm),
        0b100 | 0b101 => mortlach_multi4_z_z_shift_mm(word).map(Encoding::MortlachMulti4ZZShiftMm),
        _ => None,
    }
}

/// Arm group `mortlach_multi_sve_2d1`: SME2 saturating doubling multiply
/// high of four vectors by four.
fn mortlach_multi_sve_2d1(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 9, 5);
    match op0 {
        0b00000 => mortlach_multi4_z_z_sqdmulh_mm(word).map(Encoding::MortlachMulti4ZZSqdmulhMm),
        _ => None,
    }
}

/// Arm group `mortlach_multi_sve_3`: SME2 clamps, two-vector permutes and
/// rounding shifts right and narrow, by op1 (bits 12:10), with op0 (the
/// element size) and op2 (bit 1, fixed by the four-vector forms).
fn mortlach_multi_sve_3(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 23, 22);
    let op1 = bits(word, 12, 10);
    let op2 = bits(word, 1, 1);
    match (op0, op1, op2) {
        (_, 0b000, _) => mortlach_multi2_fclamp(word).map(Encoding::MortlachMulti2Fclamp),
        (_, 0b001, _) => mortlach_multi2_clamp_int(word).map(Encoding::MortlachMulti2ClampInt),
        (_, 0b010, 0b0) => mortlach_multi4_fclamp(word).map(Encoding::MortlachMulti4Fclamp),
        (_, 0b011, 0b0) => mortlach_multi4_clamp_int(word).map(Encoding::MortlachMulti4ClampInt),
        (_, 0b100, _) => mortlach_multi2_z_z_zip(word).map(Encoding::MortlachMulti2ZZZip),
        (0b00, 0b101, _) => {
            mortlach_multi2_z_z_long_zip(word).map(Encoding::MortlachMulti2ZZLongZip)
        }
        (0b11, 0b101, _) => mortlach_multi2_qrshr(word).map(Encoding::MortlachMulti2Qrshr),
        (_, 0b110 | 0b111, _) => mortlach_multi4_qrshr(word).map(Encoding::MortlachMulti4Qrshr),
        _ => None,
    }
}

/// Arm group `mortlach_multi_sve_4`: SME2 conversions, roundings, unpacks
/// and four-vector permutes, by op1 (bits 20:18) and op2 (bits 17:16),
/// with op0 (bits 23:22) and the bits op3 (6:5) and op4 (1:0) the
/// register groups fix. `op1 = 001` with `op2 = 00` and `10` are
/// mortlach_multi2_narrow_fp8_cvrt and _wide_fp8_cvrt, and `op1 = 101,
/// op2 = 00` mortlach_multi4_narrow_fp8_cvrt (FEAT_FP8); `op0 = 10, op1 =
/// 000, op2 = 00` is mortlach_multi2_wide_fp_cvrt (FEAT_SME_F16F16).
fn mortlach_multi_sve_4(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 23, 22);
    let op1 = bits(word, 20, 18);
    let op2 = bits(word, 17, 16);
    let op3 = bits(word, 6, 5);
    let op4 = bits(word, 1, 0);
    match (op0, op1, op2, op3, op4) {
        (0b00 | 0b01, 0b000, 0b00, _, _) => {
            mortlach_multi2_narrow_fp_cvrt(word).map(Encoding::MortlachMulti2NarrowFpCvrt)
        }
        (0b00, 0b000, 0b01, _, 0b00 | 0b10) => {
            mortlach_multi2_fpint_cvrt(word).map(Encoding::MortlachMulti2FpintCvrt)
        }
        (0b00, 0b000, 0b10, _, 0b00 | 0b10) => {
            mortlach_multi2_intfp_cvrt(word).map(Encoding::MortlachMulti2IntfpCvrt)
        }
        (0b00 | 0b01, 0b000, 0b11, _, _) => {
            mortlach_multi2_narrow_int_cvrt(word).map(Encoding::MortlachMulti2NarrowIntCvrt)
        }
        (_, 0b001, 0b01, _, _) => {
            mortlach_multi2_wide_int(word).map(Encoding::MortlachMulti2WideInt)
        }
        (_, 0b010 | 0b011, _, 0b00 | 0b10, 0b00 | 0b10) => {
            mortlach_multi2_frint(word).map(Encoding::MortlachMulti2Frint)
        }
        (0b00, 0b100, 0b01, 0b00 | 0b01, 0b00) => {
            mortlach_multi4_fpint_cvrt(word).map(Encoding::MortlachMulti4FpintCvrt)
        }
        (0b00, 0b100, 0b10, 0b00 | 0b01, 0b00) => {
            mortlach_multi4_intfp_cvrt(word).map(Encoding::MortlachMulti4IntfpCvrt)
        }
        (_, 0b100, 0b11, _, _) => {
            mortlach_multi4_narrow_int_cvrt(word).map(Encoding::MortlachMulti4NarrowIntCvrt)
        }
        (_, 0b101, 0b01, 0b00 | 0b10, 0b00 | 0b01) => {
            mortlach_multi4_wide_int(word).map(Encoding::MortlachMulti4WideInt)
        }
        (_, 0b101, 0b10, 0b00, 0b00 | 0b10) => {
            mortlach_multi4_z_z_zip(word).map(Encoding::MortlachMulti4ZZZip)
        }
        (0b00, 0b101, 0b11, 0b00, 0b00 | 0b10) => {
            mortlach_multi4_z_z_long_zip(word).map(Encoding::MortlachMulti4ZZLongZip)
        }
        (_, 0b110 | 0b111, _, 0b00, 0b00) => {
            mortlach_multi4_frint(word).map(Encoding::MortlachMulti4Frint)
        }
        _ => None,
    }
}

/// Arm class `mortlach_multi2_select_int`: select two vectors' elements by
/// a predicate-as-counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2SelectInt {
    SelMzPZz2,
}

fn mortlach_multi2_select_int(_word: u32) -> Option<MortlachMulti2SelectInt> {
    Some(MortlachMulti2SelectInt::SelMzPZz2)
}

/// Arm class `mortlach_multi4_select_int`: select four vectors' elements by
/// a predicate-as-counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4SelectInt {
    SelMzPZz4,
}

fn mortlach_multi4_select_int(_word: u32) -> Option<MortlachMulti4SelectInt> {
    Some(MortlachMulti4SelectInt::SelMzPZz4)
}

/// Arm class `mortlach_multi2_z_z_minmax_sm`: signed or unsigned (`U`)
/// maximum or minimum (`op`) of two vectors and one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZMinmaxSm {
    SmaxMzZzv2x1,
    SminMzZzv2x1,
    UmaxMzZzv2x1,
    UminMzZzv2x1,
}

fn mortlach_multi2_z_z_minmax_sm(word: u32) -> Option<MortlachMulti2ZZMinmaxSm> {
    let op = bits(word, 5, 5);
    let u = bits(word, 0, 0);
    match (op, u) {
        (0b0, 0b0) => Some(MortlachMulti2ZZMinmaxSm::SmaxMzZzv2x1),
        (0b1, 0b0) => Some(MortlachMulti2ZZMinmaxSm::SminMzZzv2x1),
        (0b0, 0b1) => Some(MortlachMulti2ZZMinmaxSm::UmaxMzZzv2x1),
        (0b1, 0b1) => Some(MortlachMulti2ZZMinmaxSm::UminMzZzv2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_z_fminmax_sm`: floating-point maximum or
/// minimum (`o2`), number or not (`op`), of two vectors and one. `size =
/// 00` is the BFloat16 forms (FEAT_SVE_B16B16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZFminmaxSm {
    FmaxMzZzv2x1,
    FminMzZzv2x1,
    FmaxnmMzZzv2x1,
    FminnmMzZzv2x1,
}

fn mortlach_multi2_z_z_fminmax_sm(word: u32) -> Option<MortlachMulti2ZZFminmaxSm> {
    let size = bits(word, 23, 22);
    let op = bits(word, 5, 5);
    let o2 = bits(word, 0, 0);
    match (size, op, o2) {
        (0b00, _, _) => None,
        (_, 0b0, 0b0) => Some(MortlachMulti2ZZFminmaxSm::FmaxMzZzv2x1),
        (_, 0b0, 0b1) => Some(MortlachMulti2ZZFminmaxSm::FminMzZzv2x1),
        (_, 0b1, 0b0) => Some(MortlachMulti2ZZFminmaxSm::FmaxnmMzZzv2x1),
        (_, 0b1, 0b1) => Some(MortlachMulti2ZZFminmaxSm::FminnmMzZzv2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_z_shift_sm`: signed or unsigned (`U`)
/// rounding shift left of two vectors by one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZShiftSm {
    SrshlMzZzv2x1,
    UrshlMzZzv2x1,
}

fn mortlach_multi2_z_z_shift_sm(word: u32) -> Option<MortlachMulti2ZZShiftSm> {
    let opc = bits(word, 7, 5);
    let u = bits(word, 0, 0);
    match (opc, u) {
        (0b001, 0b0) => Some(MortlachMulti2ZZShiftSm::SrshlMzZzv2x1),
        (0b001, 0b1) => Some(MortlachMulti2ZZShiftSm::UrshlMzZzv2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_z_add_sm`: add one vector to two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZAddSm {
    AddMzZzv2x1,
}

fn mortlach_multi2_z_z_add_sm(word: u32) -> Option<MortlachMulti2ZZAddSm> {
    let op = bits(word, 0, 0);
    match op {
        0b0 => Some(MortlachMulti2ZZAddSm::AddMzZzv2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_z_sqdmulh_sm`: saturating doubling multiply
/// high of two vectors by one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZSqdmulhSm {
    SqdmulhMzZzv2x1,
}

fn mortlach_multi2_z_z_sqdmulh_sm(word: u32) -> Option<MortlachMulti2ZZSqdmulhSm> {
    let op = bits(word, 0, 0);
    match op {
        0b0 => Some(MortlachMulti2ZZSqdmulhSm::SqdmulhMzZzv2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_z_minmax_sm`: signed or unsigned (`U`)
/// maximum or minimum (`op`) of four vectors and one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZMinmaxSm {
    SmaxMzZzv4x1,
    SminMzZzv4x1,
    UmaxMzZzv4x1,
    UminMzZzv4x1,
}

fn mortlach_multi4_z_z_minmax_sm(word: u32) -> Option<MortlachMulti4ZZMinmaxSm> {
    let op = bits(word, 5, 5);
    let u = bits(word, 0, 0);
    match (op, u) {
        (0b0, 0b0) => Some(MortlachMulti4ZZMinmaxSm::SmaxMzZzv4x1),
        (0b1, 0b0) => Some(MortlachMulti4ZZMinmaxSm::SminMzZzv4x1),
        (0b0, 0b1) => Some(MortlachMulti4ZZMinmaxSm::UmaxMzZzv4x1),
        (0b1, 0b1) => Some(MortlachMulti4ZZMinmaxSm::UminMzZzv4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_z_fminmax_sm`: floating-point maximum or
/// minimum (`o2`), number or not (`op`), of four vectors and one. `size =
/// 00` is the BFloat16 forms (FEAT_SVE_B16B16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZFminmaxSm {
    FmaxMzZzv4x1,
    FminMzZzv4x1,
    FmaxnmMzZzv4x1,
    FminnmMzZzv4x1,
}

fn mortlach_multi4_z_z_fminmax_sm(word: u32) -> Option<MortlachMulti4ZZFminmaxSm> {
    let size = bits(word, 23, 22);
    let op = bits(word, 5, 5);
    let o2 = bits(word, 0, 0);
    match (size, op, o2) {
        (0b00, _, _) => None,
        (_, 0b0, 0b0) => Some(MortlachMulti4ZZFminmaxSm::FmaxMzZzv4x1),
        (_, 0b0, 0b1) => Some(MortlachMulti4ZZFminmaxSm::FminMzZzv4x1),
        (_, 0b1, 0b0) => Some(MortlachMulti4ZZFminmaxSm::FmaxnmMzZzv4x1),
        (_, 0b1, 0b1) => Some(MortlachMulti4ZZFminmaxSm::FminnmMzZzv4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_z_shift_sm`: signed or unsigned (`U`)
/// rounding shift left of four vectors by one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZShiftSm {
    SrshlMzZzv4x1,
    UrshlMzZzv4x1,
}

fn mortlach_multi4_z_z_shift_sm(word: u32) -> Option<MortlachMulti4ZZShiftSm> {
    let opc = bits(word, 7, 5);
    let u = bits(word, 0, 0);
    match (opc, u) {
        (0b001, 0b0) => Some(MortlachMulti4ZZShiftSm::SrshlMzZzv4x1),
        (0b001, 0b1) => Some(MortlachMulti4ZZShiftSm::UrshlMzZzv4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_z_add_sm`: add one vector to four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZAddSm {
    AddMzZzv4x1,
}

fn mortlach_multi4_z_z_add_sm(word: u32) -> Option<MortlachMulti4ZZAddSm> {
    let op = bits(word, 0, 0);
    match op {
        0b0 => Some(MortlachMulti4ZZAddSm::AddMzZzv4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_z_sqdmulh_sm`: saturating doubling multiply
/// high of four vectors by one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZSqdmulhSm {
    SqdmulhMzZzv4x1,
}

fn mortlach_multi4_z_z_sqdmulh_sm(word: u32) -> Option<MortlachMulti4ZZSqdmulhSm> {
    let op = bits(word, 0, 0);
    match op {
        0b0 => Some(MortlachMulti4ZZSqdmulhSm::SqdmulhMzZzv4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_z_minmax_mm`: signed or unsigned (`U`)
/// maximum or minimum (`opc`) of two vectors and two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZMinmaxMm {
    SmaxMzZzw2x2,
    SminMzZzw2x2,
    UmaxMzZzw2x2,
    UminMzZzw2x2,
}

fn mortlach_multi2_z_z_minmax_mm(word: u32) -> Option<MortlachMulti2ZZMinmaxMm> {
    let opc = bits(word, 6, 5);
    let u = bits(word, 0, 0);
    match (opc, u) {
        (0b00, 0b0) => Some(MortlachMulti2ZZMinmaxMm::SmaxMzZzw2x2),
        (0b01, 0b0) => Some(MortlachMulti2ZZMinmaxMm::SminMzZzw2x2),
        (0b00, 0b1) => Some(MortlachMulti2ZZMinmaxMm::UmaxMzZzw2x2),
        (0b01, 0b1) => Some(MortlachMulti2ZZMinmaxMm::UminMzZzw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_z_fminmax_mm`: floating-point maximum or
/// minimum (`o2`), number or not (`opc`), of two vectors and two. `size =
/// 00` is the BFloat16 forms (FEAT_SVE_B16B16), `opc = 10` FAMAX and FAMIN
/// (FEAT_FAMINMAX).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZFminmaxMm {
    FmaxMzZzw2x2,
    FminMzZzw2x2,
    FmaxnmMzZzw2x2,
    FminnmMzZzw2x2,
}

fn mortlach_multi2_z_z_fminmax_mm(word: u32) -> Option<MortlachMulti2ZZFminmaxMm> {
    let size = bits(word, 23, 22);
    let opc = bits(word, 6, 5);
    let o2 = bits(word, 0, 0);
    match (size, opc, o2) {
        (0b00, _, _) => None,
        (_, 0b00, 0b0) => Some(MortlachMulti2ZZFminmaxMm::FmaxMzZzw2x2),
        (_, 0b00, 0b1) => Some(MortlachMulti2ZZFminmaxMm::FminMzZzw2x2),
        (_, 0b01, 0b0) => Some(MortlachMulti2ZZFminmaxMm::FmaxnmMzZzw2x2),
        (_, 0b01, 0b1) => Some(MortlachMulti2ZZFminmaxMm::FminnmMzZzw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_z_shift_mm`: signed or unsigned (`U`)
/// rounding shift left of two vectors by two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZShiftMm {
    SrshlMzZzw2x2,
    UrshlMzZzw2x2,
}

fn mortlach_multi2_z_z_shift_mm(word: u32) -> Option<MortlachMulti2ZZShiftMm> {
    let opc = bits(word, 7, 5);
    let u = bits(word, 0, 0);
    match (opc, u) {
        (0b001, 0b0) => Some(MortlachMulti2ZZShiftMm::SrshlMzZzw2x2),
        (0b001, 0b1) => Some(MortlachMulti2ZZShiftMm::UrshlMzZzw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_z_sqdmulh_mm`: saturating doubling multiply
/// high of two vectors by two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZSqdmulhMm {
    SqdmulhMzZzw2x2,
}

fn mortlach_multi2_z_z_sqdmulh_mm(word: u32) -> Option<MortlachMulti2ZZSqdmulhMm> {
    let op = bits(word, 0, 0);
    match op {
        0b0 => Some(MortlachMulti2ZZSqdmulhMm::SqdmulhMzZzw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_z_minmax_mm`: signed or unsigned (`U`)
/// maximum or minimum (`opc`) of four vectors and four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZMinmaxMm {
    SmaxMzZzw4x4,
    SminMzZzw4x4,
    UmaxMzZzw4x4,
    UminMzZzw4x4,
}

fn mortlach_multi4_z_z_minmax_mm(word: u32) -> Option<MortlachMulti4ZZMinmaxMm> {
    let opc = bits(word, 6, 5);
    let u = bits(word, 0, 0);
    match (opc, u) {
        (0b00, 0b0) => Some(MortlachMulti4ZZMinmaxMm::SmaxMzZzw4x4),
        (0b01, 0b0) => Some(MortlachMulti4ZZMinmaxMm::SminMzZzw4x4),
        (0b00, 0b1) => Some(MortlachMulti4ZZMinmaxMm::UmaxMzZzw4x4),
        (0b01, 0b1) => Some(MortlachMulti4ZZMinmaxMm::UminMzZzw4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_z_fminmax_mm`: floating-point maximum or
/// minimum (`o2`), number or not (`opc`), of four vectors and four. `size
/// = 00` is the BFloat16 forms (FEAT_SVE_B16B16), `opc = 10` FAMAX and
/// FAMIN (FEAT_FAMINMAX).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZFminmaxMm {
    FmaxMzZzw4x4,
    FminMzZzw4x4,
    FmaxnmMzZzw4x4,
    FminnmMzZzw4x4,
}

fn mortlach_multi4_z_z_fminmax_mm(word: u32) -> Option<MortlachMulti4ZZFminmaxMm> {
    let size = bits(word, 23, 22);
    let opc = bits(word, 6, 5);
    let o2 = bits(word, 0, 0);
    match (size, opc, o2) {
        (0b00, _, _) => None,
        (_, 0b00, 0b0) => Some(MortlachMulti4ZZFminmaxMm::FmaxMzZzw4x4),
        (_, 0b00, 0b1) => Some(MortlachMulti4ZZFminmaxMm::FminMzZzw4x4),
        (_, 0b01, 0b0) => Some(MortlachMulti4ZZFminmaxMm::FmaxnmMzZzw4x4),
        (_, 0b01, 0b1) => Some(MortlachMulti4ZZFminmaxMm::FminnmMzZzw4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_z_shift_mm`: signed or unsigned (`U`)
/// rounding shift left of four vectors by four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZShiftMm {
    SrshlMzZzw4x4,
    UrshlMzZzw4x4,
}

fn mortlach_multi4_z_z_shift_mm(word: u32) -> Option<MortlachMulti4ZZShiftMm> {
    let opc = bits(word, 7, 5);
    let u = bits(word, 0, 0);
    match (opc, u) {
        (0b001, 0b0) => Some(MortlachMulti4ZZShiftMm::SrshlMzZzw4x4),
        (0b001, 0b1) => Some(MortlachMulti4ZZShiftMm::UrshlMzZzw4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_z_sqdmulh_mm`: saturating doubling multiply
/// high of four vectors by four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZSqdmulhMm {
    SqdmulhMzZzw4x4,
}

fn mortlach_multi4_z_z_sqdmulh_mm(word: u32) -> Option<MortlachMulti4ZZSqdmulhMm> {
    let op = bits(word, 0, 0);
    match op {
        0b0 => Some(MortlachMulti4ZZSqdmulhMm::SqdmulhMzZzw4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_fclamp`: floating-point clamp of two
/// vectors. `size = 00` is BFCLAMP (FEAT_SVE_B16B16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2Fclamp {
    FclampMzZz2,
}

fn mortlach_multi2_fclamp(word: u32) -> Option<MortlachMulti2Fclamp> {
    let size = bits(word, 23, 22);
    let op = bits(word, 0, 0);
    match (size, op) {
        (0b00, _) => None,
        (_, 0b0) => Some(MortlachMulti2Fclamp::FclampMzZz2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_clamp_int`: signed or unsigned (`U`) clamp of
/// two vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ClampInt {
    SclampMzZz2,
    UclampMzZz2,
}

fn mortlach_multi2_clamp_int(word: u32) -> Option<MortlachMulti2ClampInt> {
    let u = bits(word, 0, 0);
    match u {
        0b0 => Some(MortlachMulti2ClampInt::SclampMzZz2),
        0b1 => Some(MortlachMulti2ClampInt::UclampMzZz2),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_fclamp`: floating-point clamp of four
/// vectors. `size = 00` is BFCLAMP (FEAT_SVE_B16B16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4Fclamp {
    FclampMzZz4,
}

fn mortlach_multi4_fclamp(word: u32) -> Option<MortlachMulti4Fclamp> {
    let size = bits(word, 23, 22);
    let op = bits(word, 0, 0);
    match (size, op) {
        (0b00, _) => None,
        (_, 0b0) => Some(MortlachMulti4Fclamp::FclampMzZz4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_clamp_int`: signed or unsigned (`U`) clamp of
/// four vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ClampInt {
    SclampMzZz4,
    UclampMzZz4,
}

fn mortlach_multi4_clamp_int(word: u32) -> Option<MortlachMulti4ClampInt> {
    let u = bits(word, 0, 0);
    match u {
        0b0 => Some(MortlachMulti4ClampInt::SclampMzZz4),
        0b1 => Some(MortlachMulti4ClampInt::UclampMzZz4),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_z_zip`: interleave or concatenate (`op`)
/// the elements of two vectors into two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZZip {
    ZipMzZz2,
    UzpMzZz2,
}

fn mortlach_multi2_z_z_zip(word: u32) -> Option<MortlachMulti2ZZZip> {
    let op = bits(word, 0, 0);
    match op {
        0b0 => Some(MortlachMulti2ZZZip::ZipMzZz2),
        0b1 => Some(MortlachMulti2ZZZip::UzpMzZz2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_z_long_zip`: interleave or concatenate
/// (`op`) the quadwords of two vectors into two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZLongZip {
    ZipMzZz2q,
    UzpMzZz2q,
}

fn mortlach_multi2_z_z_long_zip(word: u32) -> Option<MortlachMulti2ZZLongZip> {
    let op = bits(word, 0, 0);
    match op {
        0b0 => Some(MortlachMulti2ZZLongZip::ZipMzZz2q),
        0b1 => Some(MortlachMulti2ZZLongZip::UzpMzZz2q),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_qrshr`: saturating rounding shift right and
/// narrow two vectors to one, signed or unsigned (`U`, `op` to an unsigned
/// result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2Qrshr {
    SqrshrZMz2,
    SqrshruZMz2,
    UqrshrZMz2,
}

fn mortlach_multi2_qrshr(word: u32) -> Option<MortlachMulti2Qrshr> {
    let op = bits(word, 20, 20);
    let u = bits(word, 5, 5);
    match (op, u) {
        (0b0, 0b0) => Some(MortlachMulti2Qrshr::SqrshrZMz2),
        (0b1, 0b0) => Some(MortlachMulti2Qrshr::SqrshruZMz2),
        (0b0, 0b1) => Some(MortlachMulti2Qrshr::UqrshrZMz2),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_qrshr`: saturating rounding shift right and
/// narrow four vectors to one, signed or unsigned (`U`, `op` to an
/// unsigned result), concatenated or interleaved (`N`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4Qrshr {
    SqrshrZMz4,
    SqrshruZMz4,
    SqrshrnZMz4,
    SqrshrunZMz4,
    UqrshrZMz4,
    UqrshrnZMz4,
}

fn mortlach_multi4_qrshr(word: u32) -> Option<MortlachMulti4Qrshr> {
    let n = bits(word, 10, 10);
    let op = bits(word, 6, 6);
    let u = bits(word, 5, 5);
    match (n, op, u) {
        (0b0, 0b0, 0b0) => Some(MortlachMulti4Qrshr::SqrshrZMz4),
        (0b0, 0b1, 0b0) => Some(MortlachMulti4Qrshr::SqrshruZMz4),
        (0b1, 0b0, 0b0) => Some(MortlachMulti4Qrshr::SqrshrnZMz4),
        (0b1, 0b1, 0b0) => Some(MortlachMulti4Qrshr::SqrshrunZMz4),
        (0b0, 0b0, 0b1) => Some(MortlachMulti4Qrshr::UqrshrZMz4),
        (0b1, 0b0, 0b1) => Some(MortlachMulti4Qrshr::UqrshrnZMz4),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_narrow_fp_cvrt`: convert two vectors of
/// single precision to one of half precision or BFloat16 (`op`),
/// concatenated or interleaved (`N`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2NarrowFpCvrt {
    BfcvtZMz2,
    FcvtZMz2,
    BfcvtnZMz2,
    FcvtnZMz2,
}

fn mortlach_multi2_narrow_fp_cvrt(word: u32) -> Option<MortlachMulti2NarrowFpCvrt> {
    let op = bits(word, 22, 22);
    let n = bits(word, 5, 5);
    match (op, n) {
        (0b1, 0b0) => Some(MortlachMulti2NarrowFpCvrt::BfcvtZMz2),
        (0b0, 0b0) => Some(MortlachMulti2NarrowFpCvrt::FcvtZMz2),
        (0b1, 0b1) => Some(MortlachMulti2NarrowFpCvrt::BfcvtnZMz2),
        (0b0, 0b1) => Some(MortlachMulti2NarrowFpCvrt::FcvtnZMz2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_fpint_cvrt`: convert two vectors of single
/// precision to signed or unsigned (`U`) words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2FpintCvrt {
    FcvtzsMzZ2,
    FcvtzuMzZ2,
}

fn mortlach_multi2_fpint_cvrt(word: u32) -> Option<MortlachMulti2FpintCvrt> {
    let u = bits(word, 5, 5);
    match u {
        0b0 => Some(MortlachMulti2FpintCvrt::FcvtzsMzZ2),
        0b1 => Some(MortlachMulti2FpintCvrt::FcvtzuMzZ2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_intfp_cvrt`: convert two vectors of signed or
/// unsigned (`U`) words to single precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2IntfpCvrt {
    ScvtfMzZ2,
    UcvtfMzZ2,
}

fn mortlach_multi2_intfp_cvrt(word: u32) -> Option<MortlachMulti2IntfpCvrt> {
    let u = bits(word, 5, 5);
    match u {
        0b0 => Some(MortlachMulti2IntfpCvrt::ScvtfMzZ2),
        0b1 => Some(MortlachMulti2IntfpCvrt::UcvtfMzZ2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_narrow_int_cvrt`: saturating narrow of two
/// vectors to one, signed or unsigned (`U`, `op` to an unsigned result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2NarrowIntCvrt {
    SqcvtZMz2,
    SqcvtuZMz2,
    UqcvtZMz2,
}

fn mortlach_multi2_narrow_int_cvrt(word: u32) -> Option<MortlachMulti2NarrowIntCvrt> {
    let op = bits(word, 22, 22);
    let u = bits(word, 5, 5);
    match (op, u) {
        (0b0, 0b0) => Some(MortlachMulti2NarrowIntCvrt::SqcvtZMz2),
        (0b1, 0b0) => Some(MortlachMulti2NarrowIntCvrt::SqcvtuZMz2),
        (0b0, 0b1) => Some(MortlachMulti2NarrowIntCvrt::UqcvtZMz2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_wide_int`: unpack one vector's elements to two
/// vectors of twice their size, signed or unsigned (`U`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2WideInt {
    SunpkMzZ2,
    UunpkMzZ2,
}

fn mortlach_multi2_wide_int(word: u32) -> Option<MortlachMulti2WideInt> {
    let u = bits(word, 0, 0);
    match u {
        0b0 => Some(MortlachMulti2WideInt::SunpkMzZ2),
        0b1 => Some(MortlachMulti2WideInt::UunpkMzZ2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_frint`: round two vectors of single precision
/// to integral values, by the rounding `opc` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2Frint {
    FrintnMzZ2,
    FrintpMzZ2,
    FrintmMzZ2,
    FrintaMzZ2,
}

fn mortlach_multi2_frint(word: u32) -> Option<MortlachMulti2Frint> {
    let size = bits(word, 23, 22);
    let opc = bits(word, 18, 16);
    match (size, opc) {
        (0b10, 0b000) => Some(MortlachMulti2Frint::FrintnMzZ2),
        (0b10, 0b001) => Some(MortlachMulti2Frint::FrintpMzZ2),
        (0b10, 0b010) => Some(MortlachMulti2Frint::FrintmMzZ2),
        (0b10, 0b100) => Some(MortlachMulti2Frint::FrintaMzZ2),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_fpint_cvrt`: convert four vectors of single
/// precision to signed or unsigned (`U`) words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4FpintCvrt {
    FcvtzsMzZ4,
    FcvtzuMzZ4,
}

fn mortlach_multi4_fpint_cvrt(word: u32) -> Option<MortlachMulti4FpintCvrt> {
    let u = bits(word, 5, 5);
    match u {
        0b0 => Some(MortlachMulti4FpintCvrt::FcvtzsMzZ4),
        0b1 => Some(MortlachMulti4FpintCvrt::FcvtzuMzZ4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_intfp_cvrt`: convert four vectors of signed or
/// unsigned (`U`) words to single precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4IntfpCvrt {
    ScvtfMzZ4,
    UcvtfMzZ4,
}

fn mortlach_multi4_intfp_cvrt(word: u32) -> Option<MortlachMulti4IntfpCvrt> {
    let u = bits(word, 5, 5);
    match u {
        0b0 => Some(MortlachMulti4IntfpCvrt::ScvtfMzZ4),
        0b1 => Some(MortlachMulti4IntfpCvrt::UcvtfMzZ4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_narrow_int_cvrt`: saturating narrow of four
/// vectors to one, signed or unsigned (`U`, `op` to an unsigned result),
/// concatenated or interleaved (`N`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4NarrowIntCvrt {
    SqcvtZMz4,
    SqcvtuZMz4,
    SqcvtnZMz4,
    SqcvtunZMz4,
    UqcvtZMz4,
    UqcvtnZMz4,
}

fn mortlach_multi4_narrow_int_cvrt(word: u32) -> Option<MortlachMulti4NarrowIntCvrt> {
    let op = bits(word, 22, 22);
    let n = bits(word, 6, 6);
    let u = bits(word, 5, 5);
    match (op, n, u) {
        (0b0, 0b0, 0b0) => Some(MortlachMulti4NarrowIntCvrt::SqcvtZMz4),
        (0b1, 0b0, 0b0) => Some(MortlachMulti4NarrowIntCvrt::SqcvtuZMz4),
        (0b0, 0b1, 0b0) => Some(MortlachMulti4NarrowIntCvrt::SqcvtnZMz4),
        (0b1, 0b1, 0b0) => Some(MortlachMulti4NarrowIntCvrt::SqcvtunZMz4),
        (0b0, 0b0, 0b1) => Some(MortlachMulti4NarrowIntCvrt::UqcvtZMz4),
        (0b0, 0b1, 0b1) => Some(MortlachMulti4NarrowIntCvrt::UqcvtnZMz4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_wide_int`: unpack two vectors' elements to
/// four vectors of twice their size, signed or unsigned (`U`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4WideInt {
    SunpkMzZ4,
    UunpkMzZ4,
}

fn mortlach_multi4_wide_int(word: u32) -> Option<MortlachMulti4WideInt> {
    let u = bits(word, 0, 0);
    match u {
        0b0 => Some(MortlachMulti4WideInt::SunpkMzZ4),
        0b1 => Some(MortlachMulti4WideInt::UunpkMzZ4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_z_zip`: interleave or concatenate (`op`)
/// the elements of four vectors into four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZZip {
    ZipMzZ4,
    UzpMzZ4,
}

fn mortlach_multi4_z_z_zip(word: u32) -> Option<MortlachMulti4ZZZip> {
    let op = bits(word, 1, 1);
    match op {
        0b0 => Some(MortlachMulti4ZZZip::ZipMzZ4),
        0b1 => Some(MortlachMulti4ZZZip::UzpMzZ4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_z_long_zip`: interleave or concatenate
/// (`op`) the quadwords of four vectors into four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZLongZip {
    ZipMzZ4q,
    UzpMzZ4q,
}

fn mortlach_multi4_z_z_long_zip(word: u32) -> Option<MortlachMulti4ZZLongZip> {
    let op = bits(word, 1, 1);
    match op {
        0b0 => Some(MortlachMulti4ZZLongZip::ZipMzZ4q),
        0b1 => Some(MortlachMulti4ZZLongZip::UzpMzZ4q),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_frint`: round four vectors of single
/// precision to integral values, by the rounding `opc` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4Frint {
    FrintnMzZ4,
    FrintpMzZ4,
    FrintmMzZ4,
    FrintaMzZ4,
}

fn mortlach_multi4_frint(word: u32) -> Option<MortlachMulti4Frint> {
    let size = bits(word, 23, 22);
    let opc = bits(word, 18, 16);
    match (size, opc) {
        (0b10, 0b000) => Some(MortlachMulti4Frint::FrintnMzZ4),
        (0b10, 0b001) => Some(MortlachMulti4Frint::FrintpMzZ4),
        (0b10, 0b010) => Some(MortlachMulti4Frint::FrintmMzZ4),
        (0b10, 0b100) => Some(MortlachMulti4Frint::FrintaMzZ4),
        _ => None,
    }
}

/// Bits 24 and 21 set, bit 15 clear: FEAT_SME2's multi-vector groups on ZA
/// array vectors, by bit 23, then bit 20 (`mortlach_multi_array_1a`, `_1b`)
/// or bit 16 (`_2a`, `_2b`).
fn multi_array(word: u32) -> Option<Encoding> {
    match (bits(word, 23, 23), bits(word, 20, 20), bits(word, 16, 16)) {
        (0b0, 0b0, _) => mortlach_multi_array_1a(word),
        (0b0, 0b1, _) => mortlach_multi_array_1b(word),
        (0b1, _, 0b0) => mortlach_multi_array_2a(word),
        (0b1, _, 0b1) => mortlach_multi_array_2b(word),
        _ => None,
    }
}

/// Arm group `mortlach_multi_array_1a`: SME2 multi-vector operations of
/// two vectors (or one) by one into ZA array vectors. `op0 = 0, op1 = 000,
/// op3 = 1` is mortlach_multi2_zz_za_fp8_fma_long_long_sm (FEAT_SME_F8F32),
/// `op1 = 111` mortlach_multi2_zz_za_f16_sm (FEAT_SME_F16F16).
fn mortlach_multi_array_1a(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 22, 22);
    let op1 = bits(word, 12, 10);
    let op2 = bits(word, 4, 2);
    let op3 = bits(word, 1, 1);
    match (op0, op1, op2, op3) {
        (_, 0b000, _, 0b0) => mortlach_multi2_zz_za_mla_long_long_sm(word)
            .map(Encoding::MortlachMulti2ZzZaMlaLongLongSm),
        (_, 0b001, _, _) => mortlach_multi1_zz_za_mla_long_long_sm(word)
            .map(Encoding::MortlachMulti1ZzZaMlaLongLongSm),
        (0b0, 0b010, _, _) => {
            mortlach_multi2_zz_za_fma_long_sm(word).map(Encoding::MortlachMulti2ZzZaFmaLongSm)
        }
        (0b1, 0b010, _, _) => {
            mortlach_multi2_zz_za_mla_long_sm(word).map(Encoding::MortlachMulti2ZzZaMlaLongSm)
        }
        (0b0, 0b011, _, _) => {
            mortlach_multi1_zz_za_fma_long_sm(word).map(Encoding::MortlachMulti1ZzZaFmaLongSm)
        }
        (0b1, 0b011, _, _) => {
            mortlach_multi1_zz_za_mla_long_sm(word).map(Encoding::MortlachMulti1ZzZaMlaLongSm)
        }
        (0b0, 0b100, _, _) => {
            mortlach_multi2_z_za_fpdot_sm(word).map(Encoding::MortlachMulti2ZZaFpdotSm)
        }
        (_, 0b101, 0b000 | 0b001 | 0b100 | 0b101, _) => {
            mortlach_multi2_z_za_4way_dot_sm(word).map(Encoding::MortlachMulti2ZZa4wayDotSm)
        }
        (0b0, 0b101, 0b010 | 0b011 | 0b110 | 0b111, _) => {
            mortlach_multi2_z_za_mixed_dot_sm(word).map(Encoding::MortlachMulti2ZZaMixedDotSm)
        }
        (0b1, 0b101, 0b010 | 0b011 | 0b110 | 0b111, _) => {
            mortlach_multi2_z_za_2way_dot_sm(word).map(Encoding::MortlachMulti2ZZa2wayDotSm)
        }
        (_, 0b110, 0b000..=0b011, _) => {
            mortlach_multi2_zz_za_float_sm(word).map(Encoding::MortlachMulti2ZzZaFloatSm)
        }
        (_, 0b110, 0b100..=0b111, _) => {
            mortlach_multi2_zz_za_int_sm(word).map(Encoding::MortlachMulti2ZzZaIntSm)
        }
        _ => None,
    }
}

/// Arm group `mortlach_multi_array_1b`: SME2 multi-vector operations of
/// four vectors by one into ZA array vectors. `op1 = 000, op3 = 1` and `op1
/// = 001` are the FP8 products of FEAT_SME_F8F32, `op1 = 011`
/// FEAT_SME_F8F16's, `op1 = 111` FEAT_SME_F16F16's.
fn mortlach_multi_array_1b(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 22, 22);
    let op1 = bits(word, 12, 10);
    let op2 = bits(word, 4, 2);
    let op3 = bits(word, 1, 1);
    match (op0, op1, op2, op3) {
        (_, 0b000, _, 0b0) => mortlach_multi4_zz_za_mla_long_long_sm(word)
            .map(Encoding::MortlachMulti4ZzZaMlaLongLongSm),
        (0b0, 0b010, _, _) => {
            mortlach_multi4_zz_za_fma_long_sm(word).map(Encoding::MortlachMulti4ZzZaFmaLongSm)
        }
        (0b1, 0b010, _, _) => {
            mortlach_multi4_zz_za_mla_long_sm(word).map(Encoding::MortlachMulti4ZzZaMlaLongSm)
        }
        (0b0, 0b100, _, _) => {
            mortlach_multi4_z_za_fpdot_sm(word).map(Encoding::MortlachMulti4ZZaFpdotSm)
        }
        (_, 0b101, 0b000 | 0b001 | 0b100 | 0b101, _) => {
            mortlach_multi4_z_za_4way_dot_sm(word).map(Encoding::MortlachMulti4ZZa4wayDotSm)
        }
        (0b0, 0b101, 0b010 | 0b011 | 0b110 | 0b111, _) => {
            mortlach_multi4_z_za_mixed_dot_sm(word).map(Encoding::MortlachMulti4ZZaMixedDotSm)
        }
        (0b1, 0b101, 0b010 | 0b011 | 0b110 | 0b111, _) => {
            mortlach_multi4_z_za_2way_dot_sm(word).map(Encoding::MortlachMulti4ZZa2wayDotSm)
        }
        (_, 0b110, 0b000..=0b011, _) => {
            mortlach_multi4_zz_za_float_sm(word).map(Encoding::MortlachMulti4ZzZaFloatSm)
        }
        (_, 0b110, 0b100..=0b111, _) => {
            mortlach_multi4_zz_za_int_sm(word).map(Encoding::MortlachMulti4ZzZaIntSm)
        }
        _ => None,
    }
}

/// Arm group `mortlach_multi_array_2a`: SME2 multi-vector operations of two
/// vectors by two into ZA array vectors, and of two vectors to them. `op4
/// = 1` with `op3 = 000` and `010` is the FP8 products of FEAT_SME_F8F32
/// and FEAT_SME_F8F16; `op3 = 100, op5 = x1x` and `op2 = 10, op3 = 111`
/// FEAT_SME_F16F16's.
fn mortlach_multi_array_2a(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 22, 22);
    let op1 = bits(word, 20, 19);
    let op2 = bits(word, 18, 17);
    let op3 = bits(word, 12, 10);
    let op4 = bits(word, 5, 5);
    let op5 = bits(word, 4, 2);
    let op6 = bits(word, 1, 1);
    match (op0, op1, op2, op3, op4, op5, op6) {
        (_, _, _, 0b000, 0b0, _, 0b0) => mortlach_multi2_zz_za_mla_long_long_mm(word)
            .map(Encoding::MortlachMulti2ZzZaMlaLongLongMm),
        (0b0, _, _, 0b010, 0b0, 0b000 | 0b010 | 0b100 | 0b110, _) => {
            mortlach_multi2_zz_za_fma_long_mm(word).map(Encoding::MortlachMulti2ZzZaFmaLongMm)
        }
        (0b1, _, _, 0b010, 0b0, 0b000 | 0b010 | 0b100 | 0b110, _) => {
            mortlach_multi2_zz_za_mla_long_mm(word).map(Encoding::MortlachMulti2ZzZaMlaLongMm)
        }
        (0b0, _, _, 0b100, _, 0b000 | 0b001 | 0b100 | 0b101, _) => {
            mortlach_multi2_z_za_fpdot_mm(word).map(Encoding::MortlachMulti2ZZaFpdotMm)
        }
        (_, _, _, 0b101, 0b0, 0b000 | 0b001 | 0b100 | 0b101, _) => {
            mortlach_multi2_z_za_4way_dot_mm(word).map(Encoding::MortlachMulti2ZZa4wayDotMm)
        }
        (0b0, _, _, 0b101, 0b0, 0b010 | 0b011, _) => {
            mortlach_multi2_z_za_mixed_dot_mm(word).map(Encoding::MortlachMulti2ZZaMixedDotMm)
        }
        (0b1, _, _, 0b101, 0b0, 0b010 | 0b011 | 0b110 | 0b111, _) => {
            mortlach_multi2_z_za_2way_dot_mm(word).map(Encoding::MortlachMulti2ZZa2wayDotMm)
        }
        (_, _, _, 0b110, 0b0, 0b000..=0b011, _) => {
            mortlach_multi2_zz_za_float_mm(word).map(Encoding::MortlachMulti2ZzZaFloatMm)
        }
        (_, _, _, 0b110, 0b0, 0b100..=0b111, _) => {
            mortlach_multi2_zz_za_int_mm(word).map(Encoding::MortlachMulti2ZzZaIntMm)
        }
        (_, 0b00, 0b00, 0b111, 0b0, 0b000..=0b011, _) => {
            mortlach_multi2_z_za_float_mm(word).map(Encoding::MortlachMulti2ZZaFloatMm)
        }
        (_, 0b00, 0b00, 0b111, 0b0, 0b100..=0b111, _) => {
            mortlach_multi2_z_za_int_mm(word).map(Encoding::MortlachMulti2ZZaIntMm)
        }
        _ => None,
    }
}

/// Arm group `mortlach_multi_array_2b`: SME2 multi-vector operations of
/// four vectors by four into ZA array vectors, and of four vectors to
/// them, as `mortlach_multi_array_2a` with `op4` two bits wide.
fn mortlach_multi_array_2b(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 22, 22);
    let op1 = bits(word, 20, 19);
    let op2 = bits(word, 18, 17);
    let op3 = bits(word, 12, 10);
    let op4 = bits(word, 6, 5);
    let op5 = bits(word, 4, 2);
    let op6 = bits(word, 1, 1);
    match (op0, op1, op2, op3, op4, op5, op6) {
        (_, _, 0b00 | 0b10, 0b000, 0b00, _, 0b0) => mortlach_multi4_zz_za_mla_long_long_mm(word)
            .map(Encoding::MortlachMulti4ZzZaMlaLongLongMm),
        (0b0, _, 0b00 | 0b10, 0b010, 0b00, 0b000 | 0b010 | 0b100 | 0b110, _) => {
            mortlach_multi4_zz_za_fma_long_mm(word).map(Encoding::MortlachMulti4ZzZaFmaLongMm)
        }
        (0b1, _, 0b00 | 0b10, 0b010, 0b00, 0b000 | 0b010 | 0b100 | 0b110, _) => {
            mortlach_multi4_zz_za_mla_long_mm(word).map(Encoding::MortlachMulti4ZzZaMlaLongMm)
        }
        (0b0, _, 0b00 | 0b10, 0b100, 0b00 | 0b01, 0b000 | 0b001 | 0b100 | 0b101, _) => {
            mortlach_multi4_z_za_fpdot_mm(word).map(Encoding::MortlachMulti4ZZaFpdotMm)
        }
        (_, _, 0b00 | 0b10, 0b101, 0b00, 0b000 | 0b001 | 0b100 | 0b101, _) => {
            mortlach_multi4_z_za_4way_dot_mm(word).map(Encoding::MortlachMulti4ZZa4wayDotMm)
        }
        (0b0, _, 0b00 | 0b10, 0b101, 0b00, 0b010 | 0b011, _) => {
            mortlach_multi4_z_za_mixed_dot_mm(word).map(Encoding::MortlachMulti4ZZaMixedDotMm)
        }
        (0b1, _, 0b00 | 0b10, 0b101, 0b00, 0b010 | 0b011 | 0b110 | 0b111, _) => {
            mortlach_multi4_z_za_2way_dot_mm(word).map(Encoding::MortlachMulti4ZZa2wayDotMm)
        }
        (_, _, 0b00 | 0b10, 0b110, 0b00, 0b000..=0b011, _) => {
            mortlach_multi4_zz_za_float_mm(word).map(Encoding::MortlachMulti4ZzZaFloatMm)
        }
        (_, _, 0b00 | 0b10, 0b110, 0b00, 0b100..=0b111, _) => {
            mortlach_multi4_zz_za_int_mm(word).map(Encoding::MortlachMulti4ZzZaIntMm)
        }
        (_, 0b00, 0b00, 0b111, 0b00, 0b000..=0b011, _) => {
            mortlach_multi4_z_za_float_mm(word).map(Encoding::MortlachMulti4ZZaFloatMm)
        }
        (_, 0b00, 0b00, 0b111, 0b00, 0b100..=0b111, _) => {
            mortlach_multi4_z_za_int_mm(word).map(Encoding::MortlachMulti4ZZaIntMm)
        }
        _ => None,
    }
}

/// Arm class `mortlach_multi2_zz_za_mla_long_long_sm`: signed, unsigned or
/// mixed-sign (`U`, `op`) four-way multiply-add or subtract (`S`) long of
/// two vectors by one into ZA. `sz = 1` is the 64-bit forms
/// (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZzZaMlaLongLongSm {
    SmlallZaZzv2x1,
    UsmlallZaZzvS2x1,
    SmlsllZaZzv2x1,
    UmlallZaZzv2x1,
    SumlallZaZzvS2x1,
    UmlsllZaZzv2x1,
}

fn mortlach_multi2_zz_za_mla_long_long_sm(word: u32) -> Option<MortlachMulti2ZzZaMlaLongLongSm> {
    let sz = bits(word, 22, 22);
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    let op = bits(word, 2, 2);
    match (sz, u, s, op) {
        (0b1, _, _, _) => None,
        (_, 0b0, 0b0, 0b0) => Some(MortlachMulti2ZzZaMlaLongLongSm::SmlallZaZzv2x1),
        (0b0, 0b0, 0b0, 0b1) => Some(MortlachMulti2ZzZaMlaLongLongSm::UsmlallZaZzvS2x1),
        (_, 0b0, 0b1, 0b0) => Some(MortlachMulti2ZzZaMlaLongLongSm::SmlsllZaZzv2x1),
        (_, 0b1, 0b0, 0b0) => Some(MortlachMulti2ZzZaMlaLongLongSm::UmlallZaZzv2x1),
        (0b0, 0b1, 0b0, 0b1) => Some(MortlachMulti2ZzZaMlaLongLongSm::SumlallZaZzvS2x1),
        (_, 0b1, 0b1, 0b0) => Some(MortlachMulti2ZzZaMlaLongLongSm::UmlsllZaZzv2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi1_zz_za_mla_long_long_sm`: signed, unsigned or
/// mixed-sign (`U`, `op`) four-way multiply-add or subtract (`S`) long of
/// one vector by one into ZA. `sz = 1` is the 64-bit forms
/// (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti1ZzZaMlaLongLongSm {
    SmlallZaZzv1,
    UsmlallZaZzvS,
    SmlsllZaZzv1,
    UmlallZaZzv1,
    UmlsllZaZzv1,
}

fn mortlach_multi1_zz_za_mla_long_long_sm(word: u32) -> Option<MortlachMulti1ZzZaMlaLongLongSm> {
    let sz = bits(word, 22, 22);
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    let op = bits(word, 2, 2);
    match (sz, u, s, op) {
        (0b1, _, _, _) => None,
        (_, 0b0, 0b0, 0b0) => Some(MortlachMulti1ZzZaMlaLongLongSm::SmlallZaZzv1),
        (0b0, 0b0, 0b0, 0b1) => Some(MortlachMulti1ZzZaMlaLongLongSm::UsmlallZaZzvS),
        (_, 0b0, 0b1, 0b0) => Some(MortlachMulti1ZzZaMlaLongLongSm::SmlsllZaZzv1),
        (_, 0b1, 0b0, 0b0) => Some(MortlachMulti1ZzZaMlaLongLongSm::UmlallZaZzv1),
        (_, 0b1, 0b1, 0b0) => Some(MortlachMulti1ZzZaMlaLongLongSm::UmlsllZaZzv1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_zz_za_fma_long_sm`: half-precision or
/// BFloat16 (`op`) multiply-add or subtract (`S`) long of two vectors by
/// one into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZzZaFmaLongSm {
    BfmlalZaZzv2x1,
    FmlalZaZzv2x1,
    BfmlslZaZzv2x1,
    FmlslZaZzv2x1,
}

fn mortlach_multi2_zz_za_fma_long_sm(word: u32) -> Option<MortlachMulti2ZzZaFmaLongSm> {
    let op = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    let o2 = bits(word, 2, 2);
    match (op, s, o2) {
        (0b1, 0b0, 0b0) => Some(MortlachMulti2ZzZaFmaLongSm::BfmlalZaZzv2x1),
        (0b0, 0b0, 0b0) => Some(MortlachMulti2ZzZaFmaLongSm::FmlalZaZzv2x1),
        (0b1, 0b1, 0b0) => Some(MortlachMulti2ZzZaFmaLongSm::BfmlslZaZzv2x1),
        (0b0, 0b1, 0b0) => Some(MortlachMulti2ZzZaFmaLongSm::FmlslZaZzv2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_zz_za_mla_long_sm`: signed or unsigned (`U`)
/// two-way multiply-add or subtract (`S`) long of two vectors by one into
/// ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZzZaMlaLongSm {
    SmlalZaZzv2x1,
    SmlslZaZzv2x1,
    UmlalZaZzv2x1,
    UmlslZaZzv2x1,
}

fn mortlach_multi2_zz_za_mla_long_sm(word: u32) -> Option<MortlachMulti2ZzZaMlaLongSm> {
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    let op = bits(word, 2, 2);
    match (u, s, op) {
        (0b0, 0b0, 0b0) => Some(MortlachMulti2ZzZaMlaLongSm::SmlalZaZzv2x1),
        (0b0, 0b1, 0b0) => Some(MortlachMulti2ZzZaMlaLongSm::SmlslZaZzv2x1),
        (0b1, 0b0, 0b0) => Some(MortlachMulti2ZzZaMlaLongSm::UmlalZaZzv2x1),
        (0b1, 0b1, 0b0) => Some(MortlachMulti2ZzZaMlaLongSm::UmlslZaZzv2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi1_zz_za_fma_long_sm`: half-precision or
/// BFloat16 (`op`) multiply-add or subtract (`S`) long of one vector by one
/// into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti1ZzZaFmaLongSm {
    BfmlalZaZzv1,
    FmlalZaZzv1,
    BfmlslZaZzv1,
    FmlslZaZzv1,
}

fn mortlach_multi1_zz_za_fma_long_sm(word: u32) -> Option<MortlachMulti1ZzZaFmaLongSm> {
    let op = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (op, s) {
        (0b1, 0b0) => Some(MortlachMulti1ZzZaFmaLongSm::BfmlalZaZzv1),
        (0b0, 0b0) => Some(MortlachMulti1ZzZaFmaLongSm::FmlalZaZzv1),
        (0b1, 0b1) => Some(MortlachMulti1ZzZaFmaLongSm::BfmlslZaZzv1),
        (0b0, 0b1) => Some(MortlachMulti1ZzZaFmaLongSm::FmlslZaZzv1),
        _ => None,
    }
}

/// Arm class `mortlach_multi1_zz_za_mla_long_sm`: signed or unsigned (`U`)
/// two-way multiply-add or subtract (`S`) long of one vector by one into
/// ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti1ZzZaMlaLongSm {
    SmlalZaZzv1,
    SmlslZaZzv1,
    UmlalZaZzv1,
    UmlslZaZzv1,
}

fn mortlach_multi1_zz_za_mla_long_sm(word: u32) -> Option<MortlachMulti1ZzZaMlaLongSm> {
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (u, s) {
        (0b0, 0b0) => Some(MortlachMulti1ZzZaMlaLongSm::SmlalZaZzv1),
        (0b0, 0b1) => Some(MortlachMulti1ZzZaMlaLongSm::SmlslZaZzv1),
        (0b1, 0b0) => Some(MortlachMulti1ZzZaMlaLongSm::UmlalZaZzv1),
        (0b1, 0b1) => Some(MortlachMulti1ZzZaMlaLongSm::UmlslZaZzv1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_za_fpdot_sm`: half-precision or BFloat16
/// (`opc`) dot product of two vectors by one into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZaFpdotSm {
    FdotZaZzv2x1,
    BfdotZaZzv2x1,
}

fn mortlach_multi2_z_za_fpdot_sm(word: u32) -> Option<MortlachMulti2ZZaFpdotSm> {
    let opc = bits(word, 4, 3);
    match opc {
        0b00 => Some(MortlachMulti2ZZaFpdotSm::FdotZaZzv2x1),
        0b10 => Some(MortlachMulti2ZZaFpdotSm::BfdotZaZzv2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_za_4way_dot_sm`: signed or unsigned (`U`)
/// four-way dot product of two vectors by one into ZA. `sz = 1` is the
/// 64-bit forms (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZa4wayDotSm {
    SdotZaZzv2x1,
    UdotZaZzv2x1,
}

fn mortlach_multi2_z_za_4way_dot_sm(word: u32) -> Option<MortlachMulti2ZZa4wayDotSm> {
    let sz = bits(word, 22, 22);
    let u = bits(word, 4, 4);
    match (sz, u) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti2ZZa4wayDotSm::SdotZaZzv2x1),
        (_, 0b1) => Some(MortlachMulti2ZZa4wayDotSm::UdotZaZzv2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_za_mixed_dot_sm`: mixed-sign (`U`) four-way
/// dot product of two vectors by one into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZaMixedDotSm {
    UsdotZaZzvS2x1,
    SudotZaZzvS2x1,
}

fn mortlach_multi2_z_za_mixed_dot_sm(word: u32) -> Option<MortlachMulti2ZZaMixedDotSm> {
    let u = bits(word, 4, 4);
    match u {
        0b0 => Some(MortlachMulti2ZZaMixedDotSm::UsdotZaZzvS2x1),
        0b1 => Some(MortlachMulti2ZZaMixedDotSm::SudotZaZzvS2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_za_2way_dot_sm`: signed or unsigned (`U`)
/// two-way dot product of two vectors of halfwords by one into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZa2wayDotSm {
    SdotZa32Zzv2x1,
    UdotZa32Zzv2x1,
}

fn mortlach_multi2_z_za_2way_dot_sm(word: u32) -> Option<MortlachMulti2ZZa2wayDotSm> {
    let u = bits(word, 4, 4);
    match u {
        0b0 => Some(MortlachMulti2ZZa2wayDotSm::SdotZa32Zzv2x1),
        0b1 => Some(MortlachMulti2ZZa2wayDotSm::UdotZa32Zzv2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_zz_za_float_sm`: floating-point multiply-add
/// or subtract (`S`) of two vectors by one into ZA. `sz = 1` is the
/// double-precision forms (FEAT_SME_F64F64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZzZaFloatSm {
    FmlaZaZzv2x1,
    FmlsZaZzv2x1,
}

fn mortlach_multi2_zz_za_float_sm(word: u32) -> Option<MortlachMulti2ZzZaFloatSm> {
    let sz = bits(word, 22, 22);
    let s = bits(word, 3, 3);
    match (sz, s) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti2ZzZaFloatSm::FmlaZaZzv2x1),
        (_, 0b1) => Some(MortlachMulti2ZzZaFloatSm::FmlsZaZzv2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_zz_za_int_sm`: the sum or difference (`S`) of
/// two vectors and one, to ZA. `sz = 1` is the 64-bit forms
/// (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZzZaIntSm {
    AddZaZzv2x1,
    SubZaZzv2x1,
}

fn mortlach_multi2_zz_za_int_sm(word: u32) -> Option<MortlachMulti2ZzZaIntSm> {
    let sz = bits(word, 22, 22);
    let s = bits(word, 3, 3);
    match (sz, s) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti2ZzZaIntSm::AddZaZzv2x1),
        (_, 0b1) => Some(MortlachMulti2ZzZaIntSm::SubZaZzv2x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_zz_za_mla_long_long_sm`: signed, unsigned or
/// mixed-sign (`U`, `op`) four-way multiply-add or subtract (`S`) long of
/// four vectors by one into ZA. `sz = 1` is the 64-bit forms
/// (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZzZaMlaLongLongSm {
    SmlallZaZzv4x1,
    UsmlallZaZzvS4x1,
    SmlsllZaZzv4x1,
    UmlallZaZzv4x1,
    SumlallZaZzvS4x1,
    UmlsllZaZzv4x1,
}

fn mortlach_multi4_zz_za_mla_long_long_sm(word: u32) -> Option<MortlachMulti4ZzZaMlaLongLongSm> {
    let sz = bits(word, 22, 22);
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    let op = bits(word, 2, 2);
    match (sz, u, s, op) {
        (0b1, _, _, _) => None,
        (_, 0b0, 0b0, 0b0) => Some(MortlachMulti4ZzZaMlaLongLongSm::SmlallZaZzv4x1),
        (0b0, 0b0, 0b0, 0b1) => Some(MortlachMulti4ZzZaMlaLongLongSm::UsmlallZaZzvS4x1),
        (_, 0b0, 0b1, 0b0) => Some(MortlachMulti4ZzZaMlaLongLongSm::SmlsllZaZzv4x1),
        (_, 0b1, 0b0, 0b0) => Some(MortlachMulti4ZzZaMlaLongLongSm::UmlallZaZzv4x1),
        (0b0, 0b1, 0b0, 0b1) => Some(MortlachMulti4ZzZaMlaLongLongSm::SumlallZaZzvS4x1),
        (_, 0b1, 0b1, 0b0) => Some(MortlachMulti4ZzZaMlaLongLongSm::UmlsllZaZzv4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_zz_za_fma_long_sm`: half-precision or
/// BFloat16 (`op`) multiply-add or subtract (`S`) long of four vectors by
/// one into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZzZaFmaLongSm {
    BfmlalZaZzv4x1,
    FmlalZaZzv4x1,
    BfmlslZaZzv4x1,
    FmlslZaZzv4x1,
}

fn mortlach_multi4_zz_za_fma_long_sm(word: u32) -> Option<MortlachMulti4ZzZaFmaLongSm> {
    let op = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    let o2 = bits(word, 2, 2);
    match (op, s, o2) {
        (0b1, 0b0, 0b0) => Some(MortlachMulti4ZzZaFmaLongSm::BfmlalZaZzv4x1),
        (0b0, 0b0, 0b0) => Some(MortlachMulti4ZzZaFmaLongSm::FmlalZaZzv4x1),
        (0b1, 0b1, 0b0) => Some(MortlachMulti4ZzZaFmaLongSm::BfmlslZaZzv4x1),
        (0b0, 0b1, 0b0) => Some(MortlachMulti4ZzZaFmaLongSm::FmlslZaZzv4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_zz_za_mla_long_sm`: signed or unsigned (`U`)
/// two-way multiply-add or subtract (`S`) long of four vectors by one into
/// ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZzZaMlaLongSm {
    SmlalZaZzv4x1,
    SmlslZaZzv4x1,
    UmlalZaZzv4x1,
    UmlslZaZzv4x1,
}

fn mortlach_multi4_zz_za_mla_long_sm(word: u32) -> Option<MortlachMulti4ZzZaMlaLongSm> {
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    let op = bits(word, 2, 2);
    match (u, s, op) {
        (0b0, 0b0, 0b0) => Some(MortlachMulti4ZzZaMlaLongSm::SmlalZaZzv4x1),
        (0b0, 0b1, 0b0) => Some(MortlachMulti4ZzZaMlaLongSm::SmlslZaZzv4x1),
        (0b1, 0b0, 0b0) => Some(MortlachMulti4ZzZaMlaLongSm::UmlalZaZzv4x1),
        (0b1, 0b1, 0b0) => Some(MortlachMulti4ZzZaMlaLongSm::UmlslZaZzv4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_za_fpdot_sm`: half-precision or BFloat16
/// (`opc`) dot product of four vectors by one into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZaFpdotSm {
    FdotZaZzv4x1,
    BfdotZaZzv4x1,
}

fn mortlach_multi4_z_za_fpdot_sm(word: u32) -> Option<MortlachMulti4ZZaFpdotSm> {
    let opc = bits(word, 4, 3);
    match opc {
        0b00 => Some(MortlachMulti4ZZaFpdotSm::FdotZaZzv4x1),
        0b10 => Some(MortlachMulti4ZZaFpdotSm::BfdotZaZzv4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_za_4way_dot_sm`: signed or unsigned (`U`)
/// four-way dot product of four vectors by one into ZA. `sz = 1` is the
/// 64-bit forms (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZa4wayDotSm {
    SdotZaZzv4x1,
    UdotZaZzv4x1,
}

fn mortlach_multi4_z_za_4way_dot_sm(word: u32) -> Option<MortlachMulti4ZZa4wayDotSm> {
    let sz = bits(word, 22, 22);
    let u = bits(word, 4, 4);
    match (sz, u) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti4ZZa4wayDotSm::SdotZaZzv4x1),
        (_, 0b1) => Some(MortlachMulti4ZZa4wayDotSm::UdotZaZzv4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_za_mixed_dot_sm`: mixed-sign (`U`) four-way
/// dot product of four vectors by one into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZaMixedDotSm {
    UsdotZaZzvS4x1,
    SudotZaZzvS4x1,
}

fn mortlach_multi4_z_za_mixed_dot_sm(word: u32) -> Option<MortlachMulti4ZZaMixedDotSm> {
    let u = bits(word, 4, 4);
    match u {
        0b0 => Some(MortlachMulti4ZZaMixedDotSm::UsdotZaZzvS4x1),
        0b1 => Some(MortlachMulti4ZZaMixedDotSm::SudotZaZzvS4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_za_2way_dot_sm`: signed or unsigned (`U`)
/// two-way dot product of four vectors of halfwords by one into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZa2wayDotSm {
    SdotZa32Zzv4x1,
    UdotZa32Zzv4x1,
}

fn mortlach_multi4_z_za_2way_dot_sm(word: u32) -> Option<MortlachMulti4ZZa2wayDotSm> {
    let u = bits(word, 4, 4);
    match u {
        0b0 => Some(MortlachMulti4ZZa2wayDotSm::SdotZa32Zzv4x1),
        0b1 => Some(MortlachMulti4ZZa2wayDotSm::UdotZa32Zzv4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_zz_za_float_sm`: floating-point multiply-add
/// or subtract (`S`) of four vectors by one into ZA. `sz = 1` is the
/// double-precision forms (FEAT_SME_F64F64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZzZaFloatSm {
    FmlaZaZzv4x1,
    FmlsZaZzv4x1,
}

fn mortlach_multi4_zz_za_float_sm(word: u32) -> Option<MortlachMulti4ZzZaFloatSm> {
    let sz = bits(word, 22, 22);
    let s = bits(word, 3, 3);
    match (sz, s) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti4ZzZaFloatSm::FmlaZaZzv4x1),
        (_, 0b1) => Some(MortlachMulti4ZzZaFloatSm::FmlsZaZzv4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_zz_za_int_sm`: the sum or difference (`S`) of
/// four vectors and one, to ZA. `sz = 1` is the 64-bit forms
/// (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZzZaIntSm {
    AddZaZzv4x1,
    SubZaZzv4x1,
}

fn mortlach_multi4_zz_za_int_sm(word: u32) -> Option<MortlachMulti4ZzZaIntSm> {
    let sz = bits(word, 22, 22);
    let s = bits(word, 3, 3);
    match (sz, s) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti4ZzZaIntSm::AddZaZzv4x1),
        (_, 0b1) => Some(MortlachMulti4ZzZaIntSm::SubZaZzv4x1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_zz_za_mla_long_long_mm`: signed, unsigned or
/// mixed-sign (`U`, `op`) four-way multiply-add or subtract (`S`) long of
/// two vectors by two into ZA. `sz = 1` is the 64-bit forms
/// (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZzZaMlaLongLongMm {
    SmlallZaZzw2x2,
    UsmlallZaZzwS2x2,
    SmlsllZaZzw2x2,
    UmlallZaZzw2x2,
    UmlsllZaZzw2x2,
}

fn mortlach_multi2_zz_za_mla_long_long_mm(word: u32) -> Option<MortlachMulti2ZzZaMlaLongLongMm> {
    let sz = bits(word, 22, 22);
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    let op = bits(word, 2, 2);
    match (sz, u, s, op) {
        (0b1, _, _, _) => None,
        (_, 0b0, 0b0, 0b0) => Some(MortlachMulti2ZzZaMlaLongLongMm::SmlallZaZzw2x2),
        (0b0, 0b0, 0b0, 0b1) => Some(MortlachMulti2ZzZaMlaLongLongMm::UsmlallZaZzwS2x2),
        (_, 0b0, 0b1, 0b0) => Some(MortlachMulti2ZzZaMlaLongLongMm::SmlsllZaZzw2x2),
        (_, 0b1, 0b0, 0b0) => Some(MortlachMulti2ZzZaMlaLongLongMm::UmlallZaZzw2x2),
        (_, 0b1, 0b1, 0b0) => Some(MortlachMulti2ZzZaMlaLongLongMm::UmlsllZaZzw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_zz_za_fma_long_mm`: half-precision or
/// BFloat16 (`op`) multiply-add or subtract (`S`) long of two vectors by
/// two into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZzZaFmaLongMm {
    BfmlalZaZzw2x2,
    FmlalZaZzw2x2,
    BfmlslZaZzw2x2,
    FmlslZaZzw2x2,
}

fn mortlach_multi2_zz_za_fma_long_mm(word: u32) -> Option<MortlachMulti2ZzZaFmaLongMm> {
    let op = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (op, s) {
        (0b1, 0b0) => Some(MortlachMulti2ZzZaFmaLongMm::BfmlalZaZzw2x2),
        (0b0, 0b0) => Some(MortlachMulti2ZzZaFmaLongMm::FmlalZaZzw2x2),
        (0b1, 0b1) => Some(MortlachMulti2ZzZaFmaLongMm::BfmlslZaZzw2x2),
        (0b0, 0b1) => Some(MortlachMulti2ZzZaFmaLongMm::FmlslZaZzw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_zz_za_mla_long_mm`: signed or unsigned (`U`)
/// two-way multiply-add or subtract (`S`) long of two vectors by two into
/// ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZzZaMlaLongMm {
    SmlalZaZzw2x2,
    SmlslZaZzw2x2,
    UmlalZaZzw2x2,
    UmlslZaZzw2x2,
}

fn mortlach_multi2_zz_za_mla_long_mm(word: u32) -> Option<MortlachMulti2ZzZaMlaLongMm> {
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (u, s) {
        (0b0, 0b0) => Some(MortlachMulti2ZzZaMlaLongMm::SmlalZaZzw2x2),
        (0b0, 0b1) => Some(MortlachMulti2ZzZaMlaLongMm::SmlslZaZzw2x2),
        (0b1, 0b0) => Some(MortlachMulti2ZzZaMlaLongMm::UmlalZaZzw2x2),
        (0b1, 0b1) => Some(MortlachMulti2ZzZaMlaLongMm::UmlslZaZzw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_za_fpdot_mm`: half-precision or BFloat16
/// (`opc`) dot product of two vectors by two into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZaFpdotMm {
    FdotZaZzw2x2,
    BfdotZaZzw2x2,
}

fn mortlach_multi2_z_za_fpdot_mm(word: u32) -> Option<MortlachMulti2ZZaFpdotMm> {
    let opc = bits(word, 5, 4);
    match opc {
        0b00 => Some(MortlachMulti2ZZaFpdotMm::FdotZaZzw2x2),
        0b01 => Some(MortlachMulti2ZZaFpdotMm::BfdotZaZzw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_za_4way_dot_mm`: signed or unsigned (`U`)
/// four-way dot product of two vectors by two into ZA. `sz = 1` is the
/// 64-bit forms (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZa4wayDotMm {
    SdotZaZzw2x2,
    UdotZaZzw2x2,
}

fn mortlach_multi2_z_za_4way_dot_mm(word: u32) -> Option<MortlachMulti2ZZa4wayDotMm> {
    let sz = bits(word, 22, 22);
    let u = bits(word, 4, 4);
    match (sz, u) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti2ZZa4wayDotMm::SdotZaZzw2x2),
        (_, 0b1) => Some(MortlachMulti2ZZa4wayDotMm::UdotZaZzw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_za_mixed_dot_mm`: mixed-sign four-way dot
/// product of two vectors by two into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZaMixedDotMm {
    UsdotZaZzwS2x2,
}

fn mortlach_multi2_z_za_mixed_dot_mm(_word: u32) -> Option<MortlachMulti2ZZaMixedDotMm> {
    Some(MortlachMulti2ZZaMixedDotMm::UsdotZaZzwS2x2)
}

/// Arm class `mortlach_multi2_z_za_2way_dot_mm`: signed or unsigned (`U`)
/// two-way dot product of two vectors of halfwords by two into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZa2wayDotMm {
    SdotZa32Zzw2x2,
    UdotZa32Zzw2x2,
}

fn mortlach_multi2_z_za_2way_dot_mm(word: u32) -> Option<MortlachMulti2ZZa2wayDotMm> {
    let u = bits(word, 4, 4);
    match u {
        0b0 => Some(MortlachMulti2ZZa2wayDotMm::SdotZa32Zzw2x2),
        0b1 => Some(MortlachMulti2ZZa2wayDotMm::UdotZa32Zzw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_zz_za_float_mm`: floating-point multiply-add
/// or subtract (`S`) of two vectors by two into ZA. `sz = 1` is the
/// double-precision forms (FEAT_SME_F64F64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZzZaFloatMm {
    FmlaZaZzw2x2,
    FmlsZaZzw2x2,
}

fn mortlach_multi2_zz_za_float_mm(word: u32) -> Option<MortlachMulti2ZzZaFloatMm> {
    let sz = bits(word, 22, 22);
    let s = bits(word, 3, 3);
    match (sz, s) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti2ZzZaFloatMm::FmlaZaZzw2x2),
        (_, 0b1) => Some(MortlachMulti2ZzZaFloatMm::FmlsZaZzw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_zz_za_int_mm`: the sums or differences (`S`)
/// of two vectors and two, to ZA. `sz = 1` is the 64-bit forms
/// (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZzZaIntMm {
    AddZaZzw2x2,
    SubZaZzw2x2,
}

fn mortlach_multi2_zz_za_int_mm(word: u32) -> Option<MortlachMulti2ZzZaIntMm> {
    let sz = bits(word, 22, 22);
    let s = bits(word, 3, 3);
    match (sz, s) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti2ZzZaIntMm::AddZaZzw2x2),
        (_, 0b1) => Some(MortlachMulti2ZzZaIntMm::SubZaZzw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_za_float_mm`: floating-point add or
/// subtract (`S`) of two vectors to ZA. `sz = 1` is the double-precision
/// forms (FEAT_SME_F64F64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZaFloatMm {
    FaddZaZw2x2,
    FsubZaZw2x2,
}

fn mortlach_multi2_z_za_float_mm(word: u32) -> Option<MortlachMulti2ZZaFloatMm> {
    let sz = bits(word, 22, 22);
    let s = bits(word, 3, 3);
    match (sz, s) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti2ZZaFloatMm::FaddZaZw2x2),
        (_, 0b1) => Some(MortlachMulti2ZZaFloatMm::FsubZaZw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_z_za_int_mm`: add or subtract (`S`) of two
/// vectors to ZA. `sz = 1` is the 64-bit forms (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZZaIntMm {
    AddZaZw2x2,
    SubZaZw2x2,
}

fn mortlach_multi2_z_za_int_mm(word: u32) -> Option<MortlachMulti2ZZaIntMm> {
    let sz = bits(word, 22, 22);
    let s = bits(word, 3, 3);
    match (sz, s) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti2ZZaIntMm::AddZaZw2x2),
        (_, 0b1) => Some(MortlachMulti2ZZaIntMm::SubZaZw2x2),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_zz_za_mla_long_long_mm`: signed, unsigned or
/// mixed-sign (`U`, `op`) four-way multiply-add or subtract (`S`) long of
/// four vectors by four into ZA. `sz = 1` is the 64-bit forms
/// (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZzZaMlaLongLongMm {
    SmlallZaZzw4x4,
    UsmlallZaZzwS4x4,
    SmlsllZaZzw4x4,
    UmlallZaZzw4x4,
    UmlsllZaZzw4x4,
}

fn mortlach_multi4_zz_za_mla_long_long_mm(word: u32) -> Option<MortlachMulti4ZzZaMlaLongLongMm> {
    let sz = bits(word, 22, 22);
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    let op = bits(word, 2, 2);
    match (sz, u, s, op) {
        (0b1, _, _, _) => None,
        (_, 0b0, 0b0, 0b0) => Some(MortlachMulti4ZzZaMlaLongLongMm::SmlallZaZzw4x4),
        (0b0, 0b0, 0b0, 0b1) => Some(MortlachMulti4ZzZaMlaLongLongMm::UsmlallZaZzwS4x4),
        (_, 0b0, 0b1, 0b0) => Some(MortlachMulti4ZzZaMlaLongLongMm::SmlsllZaZzw4x4),
        (_, 0b1, 0b0, 0b0) => Some(MortlachMulti4ZzZaMlaLongLongMm::UmlallZaZzw4x4),
        (_, 0b1, 0b1, 0b0) => Some(MortlachMulti4ZzZaMlaLongLongMm::UmlsllZaZzw4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_zz_za_fma_long_mm`: half-precision or
/// BFloat16 (`op`) multiply-add or subtract (`S`) long of four vectors by
/// four into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZzZaFmaLongMm {
    BfmlalZaZzw4x4,
    FmlalZaZzw4x4,
    BfmlslZaZzw4x4,
    FmlslZaZzw4x4,
}

fn mortlach_multi4_zz_za_fma_long_mm(word: u32) -> Option<MortlachMulti4ZzZaFmaLongMm> {
    let op = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (op, s) {
        (0b1, 0b0) => Some(MortlachMulti4ZzZaFmaLongMm::BfmlalZaZzw4x4),
        (0b0, 0b0) => Some(MortlachMulti4ZzZaFmaLongMm::FmlalZaZzw4x4),
        (0b1, 0b1) => Some(MortlachMulti4ZzZaFmaLongMm::BfmlslZaZzw4x4),
        (0b0, 0b1) => Some(MortlachMulti4ZzZaFmaLongMm::FmlslZaZzw4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_zz_za_mla_long_mm`: signed or unsigned (`U`)
/// two-way multiply-add or subtract (`S`) long of four vectors by four into
/// ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZzZaMlaLongMm {
    SmlalZaZzw4x4,
    SmlslZaZzw4x4,
    UmlalZaZzw4x4,
    UmlslZaZzw4x4,
}

fn mortlach_multi4_zz_za_mla_long_mm(word: u32) -> Option<MortlachMulti4ZzZaMlaLongMm> {
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (u, s) {
        (0b0, 0b0) => Some(MortlachMulti4ZzZaMlaLongMm::SmlalZaZzw4x4),
        (0b0, 0b1) => Some(MortlachMulti4ZzZaMlaLongMm::SmlslZaZzw4x4),
        (0b1, 0b0) => Some(MortlachMulti4ZzZaMlaLongMm::UmlalZaZzw4x4),
        (0b1, 0b1) => Some(MortlachMulti4ZzZaMlaLongMm::UmlslZaZzw4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_za_fpdot_mm`: half-precision or BFloat16
/// (`opc`) dot product of four vectors by four into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZaFpdotMm {
    FdotZaZzw4x4,
    BfdotZaZzw4x4,
}

fn mortlach_multi4_z_za_fpdot_mm(word: u32) -> Option<MortlachMulti4ZZaFpdotMm> {
    let opc = bits(word, 5, 4);
    match opc {
        0b00 => Some(MortlachMulti4ZZaFpdotMm::FdotZaZzw4x4),
        0b01 => Some(MortlachMulti4ZZaFpdotMm::BfdotZaZzw4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_za_4way_dot_mm`: signed or unsigned (`U`)
/// four-way dot product of four vectors by four into ZA. `sz = 1` is the
/// 64-bit forms (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZa4wayDotMm {
    SdotZaZzw4x4,
    UdotZaZzw4x4,
}

fn mortlach_multi4_z_za_4way_dot_mm(word: u32) -> Option<MortlachMulti4ZZa4wayDotMm> {
    let sz = bits(word, 22, 22);
    let u = bits(word, 4, 4);
    match (sz, u) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti4ZZa4wayDotMm::SdotZaZzw4x4),
        (_, 0b1) => Some(MortlachMulti4ZZa4wayDotMm::UdotZaZzw4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_za_mixed_dot_mm`: mixed-sign four-way dot
/// product of four vectors by four into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZaMixedDotMm {
    UsdotZaZzwS4x4,
}

fn mortlach_multi4_z_za_mixed_dot_mm(_word: u32) -> Option<MortlachMulti4ZZaMixedDotMm> {
    Some(MortlachMulti4ZZaMixedDotMm::UsdotZaZzwS4x4)
}

/// Arm class `mortlach_multi4_z_za_2way_dot_mm`: signed or unsigned (`U`)
/// two-way dot product of four vectors of halfwords by four into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZa2wayDotMm {
    SdotZa32Zzw4x4,
    UdotZa32Zzw4x4,
}

fn mortlach_multi4_z_za_2way_dot_mm(word: u32) -> Option<MortlachMulti4ZZa2wayDotMm> {
    let u = bits(word, 4, 4);
    match u {
        0b0 => Some(MortlachMulti4ZZa2wayDotMm::SdotZa32Zzw4x4),
        0b1 => Some(MortlachMulti4ZZa2wayDotMm::UdotZa32Zzw4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_zz_za_float_mm`: floating-point multiply-add
/// or subtract (`S`) of four vectors by four into ZA. `sz = 1` is the
/// double-precision forms (FEAT_SME_F64F64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZzZaFloatMm {
    FmlaZaZzw4x4,
    FmlsZaZzw4x4,
}

fn mortlach_multi4_zz_za_float_mm(word: u32) -> Option<MortlachMulti4ZzZaFloatMm> {
    let sz = bits(word, 22, 22);
    let s = bits(word, 3, 3);
    match (sz, s) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti4ZzZaFloatMm::FmlaZaZzw4x4),
        (_, 0b1) => Some(MortlachMulti4ZzZaFloatMm::FmlsZaZzw4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_zz_za_int_mm`: the sums or differences (`S`)
/// of four vectors and four, to ZA. `sz = 1` is the 64-bit forms
/// (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZzZaIntMm {
    AddZaZzw4x4,
    SubZaZzw4x4,
}

fn mortlach_multi4_zz_za_int_mm(word: u32) -> Option<MortlachMulti4ZzZaIntMm> {
    let sz = bits(word, 22, 22);
    let s = bits(word, 3, 3);
    match (sz, s) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti4ZzZaIntMm::AddZaZzw4x4),
        (_, 0b1) => Some(MortlachMulti4ZzZaIntMm::SubZaZzw4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_za_float_mm`: floating-point add or
/// subtract (`S`) of four vectors to ZA. `sz = 1` is the double-precision
/// forms (FEAT_SME_F64F64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZaFloatMm {
    FaddZaZw4x4,
    FsubZaZw4x4,
}

fn mortlach_multi4_z_za_float_mm(word: u32) -> Option<MortlachMulti4ZZaFloatMm> {
    let sz = bits(word, 22, 22);
    let s = bits(word, 3, 3);
    match (sz, s) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti4ZZaFloatMm::FaddZaZw4x4),
        (_, 0b1) => Some(MortlachMulti4ZZaFloatMm::FsubZaZw4x4),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_z_za_int_mm`: add or subtract (`S`) of four
/// vectors to ZA. `sz = 1` is the 64-bit forms (FEAT_SME_I16I64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZZaIntMm {
    AddZaZw4x4,
    SubZaZw4x4,
}

fn mortlach_multi4_z_za_int_mm(word: u32) -> Option<MortlachMulti4ZZaIntMm> {
    let sz = bits(word, 22, 22);
    let s = bits(word, 3, 3);
    match (sz, s) {
        (0b1, _) => None,
        (_, 0b0) => Some(MortlachMulti4ZZaIntMm::AddZaZw4x4),
        (_, 0b1) => Some(MortlachMulti4ZZaIntMm::SubZaZw4x4),
        _ => None,
    }
}

/// Arm group `mortlach_multi_indexed_1`: SME2 multi-vector operations of
/// one vector by an indexed element into ZA array vectors. `op0 = 01` and
/// `op0 = 11, op1 = 0` are the FP8 products of FEAT_SME_F8F32 and
/// FEAT_SME_F8F16, `op0 = 10, op1 = 0` FEAT_SME_I16I64's.
fn mortlach_multi_indexed_1(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 23, 22);
    let op1 = bits(word, 12, 12);
    match (op0, op1) {
        (0b00, _) => {
            mortlach_multi1_mla_long_long_idx_s(word).map(Encoding::MortlachMulti1MlaLongLongIdxS)
        }
        (0b10, 0b1) => mortlach_multi1_fma_long_idx(word).map(Encoding::MortlachMulti1FmaLongIdx),
        (0b11, 0b1) => mortlach_multi1_mla_long_idx(word).map(Encoding::MortlachMulti1MlaLongIdx),
        _ => None,
    }
}

/// Arm group `mortlach_multi_indexed_2`: SME2 multi-vector operations of
/// two vectors by an indexed element into ZA array vectors. `op0 = 00, op1
/// = 1x` is mortlach_multi2_zza_idx_h, whose encodings are FEAT_SME_F16F16's,
/// FEAT_SME_B16B16's and FEAT_SME_F8F16's; `op2 = 1` holds the FP8
/// products, and `op0 = 1x, op1 = 0x` with `op2 = 0` FEAT_SME_I16I64's,
/// FEAT_SME_F64F64's and FEAT_SME_F8F32's.
fn mortlach_multi_indexed_2(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 23, 22);
    let op1 = bits(word, 12, 11);
    let op2 = bits(word, 5, 5);
    match (op0, op1, op2) {
        (0b00, 0b00 | 0b01, _) => {
            mortlach_multi2_mla_long_long_idx_s(word).map(Encoding::MortlachMulti2MlaLongLongIdxS)
        }
        (0b01, _, _) => mortlach_multi2_zza_idx_s(word).map(Encoding::MortlachMulti2ZzaIdxS),
        (0b10, 0b10 | 0b11, 0b0) => {
            mortlach_multi2_fma_long_idx(word).map(Encoding::MortlachMulti2FmaLongIdx)
        }
        (0b11, 0b10 | 0b11, 0b0) => {
            mortlach_multi2_mla_long_idx(word).map(Encoding::MortlachMulti2MlaLongIdx)
        }
        _ => None,
    }
}

/// Arm group `mortlach_multi_indexed_3`: SME2 multi-vector operations of
/// four vectors by an indexed element into ZA array vectors, as
/// `mortlach_multi_indexed_2` with `op2 = 1xx` and `op2 = 01x` holding the
/// FP8 products.
fn mortlach_multi_indexed_3(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 23, 22);
    let op1 = bits(word, 12, 11);
    let op2 = bits(word, 6, 4);
    match (op0, op1, op2) {
        (0b00, 0b00 | 0b01, 0b000..=0b011) => {
            mortlach_multi4_mla_long_long_idx_s(word).map(Encoding::MortlachMulti4MlaLongLongIdxS)
        }
        (0b01, _, 0b000..=0b011) => {
            mortlach_multi4_zza_idx_s(word).map(Encoding::MortlachMulti4ZzaIdxS)
        }
        (0b10, 0b10 | 0b11, 0b000 | 0b001) => {
            mortlach_multi4_fma_long_idx(word).map(Encoding::MortlachMulti4FmaLongIdx)
        }
        (0b11, 0b10 | 0b11, 0b000 | 0b001) => {
            mortlach_multi4_mla_long_idx(word).map(Encoding::MortlachMulti4MlaLongIdx)
        }
        _ => None,
    }
}

/// Arm class `mortlach_multi1_mla_long_long_idx_s`: signed, unsigned or
/// mixed-sign (`U`, `op`) four-way multiply-add or subtract (`S`) long of
/// one vector by an indexed byte into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti1MlaLongLongIdxS {
    SmlallZaZziS,
    UsmlallZaZziS,
    SmlsllZaZziS,
    UmlallZaZziS,
    SumlallZaZziS,
    UmlsllZaZziS,
}

fn mortlach_multi1_mla_long_long_idx_s(word: u32) -> Option<MortlachMulti1MlaLongLongIdxS> {
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    let op = bits(word, 2, 2);
    match (u, s, op) {
        (0b0, 0b0, 0b0) => Some(MortlachMulti1MlaLongLongIdxS::SmlallZaZziS),
        (0b0, 0b0, 0b1) => Some(MortlachMulti1MlaLongLongIdxS::UsmlallZaZziS),
        (0b0, 0b1, 0b0) => Some(MortlachMulti1MlaLongLongIdxS::SmlsllZaZziS),
        (0b1, 0b0, 0b0) => Some(MortlachMulti1MlaLongLongIdxS::UmlallZaZziS),
        (0b1, 0b0, 0b1) => Some(MortlachMulti1MlaLongLongIdxS::SumlallZaZziS),
        (0b1, 0b1, 0b0) => Some(MortlachMulti1MlaLongLongIdxS::UmlsllZaZziS),
        _ => None,
    }
}

/// Arm class `mortlach_multi1_fma_long_idx`: half-precision or BFloat16
/// (`op`) multiply-add or subtract (`S`) long of one vector by an indexed
/// element into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti1FmaLongIdx {
    BfmlalZaZzi1,
    FmlalZaZzi1,
    BfmlslZaZzi1,
    FmlslZaZzi1,
}

fn mortlach_multi1_fma_long_idx(word: u32) -> Option<MortlachMulti1FmaLongIdx> {
    let op = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (op, s) {
        (0b1, 0b0) => Some(MortlachMulti1FmaLongIdx::BfmlalZaZzi1),
        (0b0, 0b0) => Some(MortlachMulti1FmaLongIdx::FmlalZaZzi1),
        (0b1, 0b1) => Some(MortlachMulti1FmaLongIdx::BfmlslZaZzi1),
        (0b0, 0b1) => Some(MortlachMulti1FmaLongIdx::FmlslZaZzi1),
        _ => None,
    }
}

/// Arm class `mortlach_multi1_mla_long_idx`: signed or unsigned (`U`)
/// two-way multiply-add or subtract (`S`) long of one vector by an indexed
/// halfword into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti1MlaLongIdx {
    SmlalZaZzi1,
    SmlslZaZzi1,
    UmlalZaZzi1,
    UmlslZaZzi1,
}

fn mortlach_multi1_mla_long_idx(word: u32) -> Option<MortlachMulti1MlaLongIdx> {
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (u, s) {
        (0b0, 0b0) => Some(MortlachMulti1MlaLongIdx::SmlalZaZzi1),
        (0b0, 0b1) => Some(MortlachMulti1MlaLongIdx::SmlslZaZzi1),
        (0b1, 0b0) => Some(MortlachMulti1MlaLongIdx::UmlalZaZzi1),
        (0b1, 0b1) => Some(MortlachMulti1MlaLongIdx::UmlslZaZzi1),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_mla_long_long_idx_s`: signed, unsigned or
/// mixed-sign (`U`, `op`) four-way multiply-add or subtract (`S`) long of
/// two vectors by an indexed byte into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2MlaLongLongIdxS {
    SmlallZaZziS2xi,
    UsmlallZaZziS2xi,
    SmlsllZaZziS2xi,
    UmlallZaZziS2xi,
    SumlallZaZziS2xi,
    UmlsllZaZziS2xi,
}

fn mortlach_multi2_mla_long_long_idx_s(word: u32) -> Option<MortlachMulti2MlaLongLongIdxS> {
    let op = bits(word, 5, 5);
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (op, u, s) {
        (0b0, 0b0, 0b0) => Some(MortlachMulti2MlaLongLongIdxS::SmlallZaZziS2xi),
        (0b1, 0b0, 0b0) => Some(MortlachMulti2MlaLongLongIdxS::UsmlallZaZziS2xi),
        (0b0, 0b0, 0b1) => Some(MortlachMulti2MlaLongLongIdxS::SmlsllZaZziS2xi),
        (0b0, 0b1, 0b0) => Some(MortlachMulti2MlaLongLongIdxS::UmlallZaZziS2xi),
        (0b1, 0b1, 0b0) => Some(MortlachMulti2MlaLongLongIdxS::SumlallZaZziS2xi),
        (0b0, 0b1, 0b1) => Some(MortlachMulti2MlaLongLongIdxS::UmlsllZaZziS2xi),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_zza_idx_s`: FMLA and FMLS of single-precision
/// values, the vertical dot products FVDOT, BFVDOT, SVDOT and UVDOT (`op =
/// 0`), and the dot products SDOT, UDOT, USDOT, SUDOT, FDOT and BFDOT (`op
/// = 1`), of two vectors by an indexed element or group (`opc2`) into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2ZzaIdxS {
    FmlaZaZziS2xi,
    FvdotZaZzi2xi,
    BfvdotZaZzi2xi,
    SvdotZa32Zzi2xi,
    SdotZa32Zzi2xi,
    FdotZaZzi2xi,
    BfdotZaZzi2xi,
    SdotZaZziS2xi,
    UsdotZaZziS2xi,
    FmlsZaZziS2xi,
    UvdotZa32Zzi2xi,
    UdotZa32Zzi2xi,
    UdotZaZziS2xi,
    SudotZaZziS2xi,
}

fn mortlach_multi2_zza_idx_s(word: u32) -> Option<MortlachMulti2ZzaIdxS> {
    let op = bits(word, 12, 12);
    let opc2 = bits(word, 5, 3);
    match (op, opc2) {
        (0b0, 0b000) => Some(MortlachMulti2ZzaIdxS::FmlaZaZziS2xi),
        (0b0, 0b001) => Some(MortlachMulti2ZzaIdxS::FvdotZaZzi2xi),
        (0b0, 0b011) => Some(MortlachMulti2ZzaIdxS::BfvdotZaZzi2xi),
        (0b0, 0b100) => Some(MortlachMulti2ZzaIdxS::SvdotZa32Zzi2xi),
        (0b1, 0b000) => Some(MortlachMulti2ZzaIdxS::SdotZa32Zzi2xi),
        (0b1, 0b001) => Some(MortlachMulti2ZzaIdxS::FdotZaZzi2xi),
        (0b1, 0b011) => Some(MortlachMulti2ZzaIdxS::BfdotZaZzi2xi),
        (0b1, 0b100) => Some(MortlachMulti2ZzaIdxS::SdotZaZziS2xi),
        (0b1, 0b101) => Some(MortlachMulti2ZzaIdxS::UsdotZaZziS2xi),
        (0b0, 0b010) => Some(MortlachMulti2ZzaIdxS::FmlsZaZziS2xi),
        (0b0, 0b110) => Some(MortlachMulti2ZzaIdxS::UvdotZa32Zzi2xi),
        (0b1, 0b010) => Some(MortlachMulti2ZzaIdxS::UdotZa32Zzi2xi),
        (0b1, 0b110) => Some(MortlachMulti2ZzaIdxS::UdotZaZziS2xi),
        (0b1, 0b111) => Some(MortlachMulti2ZzaIdxS::SudotZaZziS2xi),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_fma_long_idx`: half-precision or BFloat16
/// (`op`) multiply-add or subtract (`S`) long of two vectors by an indexed
/// element into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2FmaLongIdx {
    BfmlalZaZzi2xi,
    FmlalZaZzi2xi,
    BfmlslZaZzi2xi,
    FmlslZaZzi2xi,
}

fn mortlach_multi2_fma_long_idx(word: u32) -> Option<MortlachMulti2FmaLongIdx> {
    let op = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (op, s) {
        (0b1, 0b0) => Some(MortlachMulti2FmaLongIdx::BfmlalZaZzi2xi),
        (0b0, 0b0) => Some(MortlachMulti2FmaLongIdx::FmlalZaZzi2xi),
        (0b1, 0b1) => Some(MortlachMulti2FmaLongIdx::BfmlslZaZzi2xi),
        (0b0, 0b1) => Some(MortlachMulti2FmaLongIdx::FmlslZaZzi2xi),
        _ => None,
    }
}

/// Arm class `mortlach_multi2_mla_long_idx`: signed or unsigned (`U`)
/// two-way multiply-add or subtract (`S`) long of two vectors by an indexed
/// halfword into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti2MlaLongIdx {
    SmlalZaZzi2xi,
    SmlslZaZzi2xi,
    UmlalZaZzi2xi,
    UmlslZaZzi2xi,
}

fn mortlach_multi2_mla_long_idx(word: u32) -> Option<MortlachMulti2MlaLongIdx> {
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (u, s) {
        (0b0, 0b0) => Some(MortlachMulti2MlaLongIdx::SmlalZaZzi2xi),
        (0b0, 0b1) => Some(MortlachMulti2MlaLongIdx::SmlslZaZzi2xi),
        (0b1, 0b0) => Some(MortlachMulti2MlaLongIdx::UmlalZaZzi2xi),
        (0b1, 0b1) => Some(MortlachMulti2MlaLongIdx::UmlslZaZzi2xi),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_mla_long_long_idx_s`: signed, unsigned or
/// mixed-sign (`U`, `op`) four-way multiply-add or subtract (`S`) long of
/// four vectors by an indexed byte into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4MlaLongLongIdxS {
    SmlallZaZziS4xi,
    UsmlallZaZziS4xi,
    SmlsllZaZziS4xi,
    UmlallZaZziS4xi,
    SumlallZaZziS4xi,
    UmlsllZaZziS4xi,
}

fn mortlach_multi4_mla_long_long_idx_s(word: u32) -> Option<MortlachMulti4MlaLongLongIdxS> {
    let op = bits(word, 5, 5);
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (op, u, s) {
        (0b0, 0b0, 0b0) => Some(MortlachMulti4MlaLongLongIdxS::SmlallZaZziS4xi),
        (0b1, 0b0, 0b0) => Some(MortlachMulti4MlaLongLongIdxS::UsmlallZaZziS4xi),
        (0b0, 0b0, 0b1) => Some(MortlachMulti4MlaLongLongIdxS::SmlsllZaZziS4xi),
        (0b0, 0b1, 0b0) => Some(MortlachMulti4MlaLongLongIdxS::UmlallZaZziS4xi),
        (0b1, 0b1, 0b0) => Some(MortlachMulti4MlaLongLongIdxS::SumlallZaZziS4xi),
        (0b0, 0b1, 0b1) => Some(MortlachMulti4MlaLongLongIdxS::UmlsllZaZziS4xi),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_zza_idx_s`: FMLA and FMLS of single-precision
/// values, the vertical dot products SVDOT, UVDOT, USVDOT and SUVDOT (`op =
/// 0`), and the dot products SDOT, UDOT, USDOT, SUDOT, FDOT and BFDOT (`op
/// = 1`), of four vectors by an indexed element or group (`opc2`) into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4ZzaIdxS {
    FmlaZaZziS4xi,
    SvdotZaZziS4xi,
    UsvdotZaZziS4xi,
    SdotZa32Zzi4xi,
    FdotZaZzi4xi,
    BfdotZaZzi4xi,
    SdotZaZziS4xi,
    UsdotZaZziS4xi,
    FmlsZaZziS4xi,
    UvdotZaZziS4xi,
    SuvdotZaZziS4xi,
    UdotZa32Zzi4xi,
    UdotZaZziS4xi,
    SudotZaZziS4xi,
}

fn mortlach_multi4_zza_idx_s(word: u32) -> Option<MortlachMulti4ZzaIdxS> {
    let op = bits(word, 12, 12);
    let opc2 = bits(word, 5, 3);
    match (op, opc2) {
        (0b0, 0b000) => Some(MortlachMulti4ZzaIdxS::FmlaZaZziS4xi),
        (0b0, 0b100) => Some(MortlachMulti4ZzaIdxS::SvdotZaZziS4xi),
        (0b0, 0b101) => Some(MortlachMulti4ZzaIdxS::UsvdotZaZziS4xi),
        (0b1, 0b000) => Some(MortlachMulti4ZzaIdxS::SdotZa32Zzi4xi),
        (0b1, 0b001) => Some(MortlachMulti4ZzaIdxS::FdotZaZzi4xi),
        (0b1, 0b011) => Some(MortlachMulti4ZzaIdxS::BfdotZaZzi4xi),
        (0b1, 0b100) => Some(MortlachMulti4ZzaIdxS::SdotZaZziS4xi),
        (0b1, 0b101) => Some(MortlachMulti4ZzaIdxS::UsdotZaZziS4xi),
        (0b0, 0b010) => Some(MortlachMulti4ZzaIdxS::FmlsZaZziS4xi),
        (0b0, 0b110) => Some(MortlachMulti4ZzaIdxS::UvdotZaZziS4xi),
        (0b0, 0b111) => Some(MortlachMulti4ZzaIdxS::SuvdotZaZziS4xi),
        (0b1, 0b010) => Some(MortlachMulti4ZzaIdxS::UdotZa32Zzi4xi),
        (0b1, 0b110) => Some(MortlachMulti4ZzaIdxS::UdotZaZziS4xi),
        (0b1, 0b111) => Some(MortlachMulti4ZzaIdxS::SudotZaZziS4xi),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_fma_long_idx`: half-precision or BFloat16
/// (`op`) multiply-add or subtract (`S`) long of four vectors by an indexed
/// element into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4FmaLongIdx {
    BfmlalZaZzi4xi,
    FmlalZaZzi4xi,
    BfmlslZaZzi4xi,
    FmlslZaZzi4xi,
}

fn mortlach_multi4_fma_long_idx(word: u32) -> Option<MortlachMulti4FmaLongIdx> {
    let op = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (op, s) {
        (0b1, 0b0) => Some(MortlachMulti4FmaLongIdx::BfmlalZaZzi4xi),
        (0b0, 0b0) => Some(MortlachMulti4FmaLongIdx::FmlalZaZzi4xi),
        (0b1, 0b1) => Some(MortlachMulti4FmaLongIdx::BfmlslZaZzi4xi),
        (0b0, 0b1) => Some(MortlachMulti4FmaLongIdx::FmlslZaZzi4xi),
        _ => None,
    }
}

/// Arm class `mortlach_multi4_mla_long_idx`: signed or unsigned (`U`)
/// two-way multiply-add or subtract (`S`) long of four vectors by an indexed
/// halfword into ZA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachMulti4MlaLongIdx {
    SmlalZaZzi4xi,
    SmlslZaZzi4xi,
    UmlalZaZzi4xi,
    UmlslZaZzi4xi,
}

fn mortlach_multi4_mla_long_idx(word: u32) -> Option<MortlachMulti4MlaLongIdx> {
    let u = bits(word, 4, 4);
    let s = bits(word, 3, 3);
    match (u, s) {
        (0b0, 0b0) => Some(MortlachMulti4MlaLongIdx::SmlalZaZzi4xi),
        (0b0, 0b1) => Some(MortlachMulti4MlaLongIdx::SmlslZaZzi4xi),
        (0b1, 0b0) => Some(MortlachMulti4MlaLongIdx::UmlalZaZzi4xi),
        (0b1, 0b1) => Some(MortlachMulti4MlaLongIdx::UmlslZaZzi4xi),
        _ => None,
    }
}

/// Arm group `mortlach_mem`: SME memory, by op0 (bits 24:21) and bit 4,
/// and for LDR and STR by op2, then of a ZA array vector by op1 and bits
/// 12:10, of ZT0 by op3 and op4.
fn mortlach_mem(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 24, 21);
    let op1 = bits(word, 20, 16);
    let op2 = bits(word, 15, 15);
    let op3 = bits(word, 14, 10);
    let op4 = bits(word, 4, 2);
    match op0 {
        0b0000 | 0b0010 | 0b0100 | 0b0110 => match bits(word, 4, 4) {
            0b0 => mortlach_contig_load(word).map(Encoding::MortlachContigLoad),
            _ => None,
        },
        0b0001 | 0b0011 | 0b0101 | 0b0111 => match bits(word, 4, 4) {
            0b0 => mortlach_contig_store(word).map(Encoding::MortlachContigStore),
            _ => None,
        },
        0b1000 | 0b1001 => match op2 {
            0b0 => match (op1, bits(word, 12, 10), bits(word, 4, 4)) {
                (0b00000, 0b000, 0b0) => mortlach_ctxt_ldst(word).map(Encoding::MortlachCtxtLdst),
                _ => None,
            },
            0b1 => match (op3, op4) {
                (0b00000, 0b000) => mortlach_zt_ldst(word).map(Encoding::MortlachZtLdst),
                _ => None,
            },
            _ => None,
        },
        0b1110 => match bits(word, 4, 4) {
            0b0 => mortlach_contig_qload(word).map(Encoding::MortlachContigQload),
            _ => None,
        },
        0b1111 => match bits(word, 4, 4) {
            0b0 => mortlach_contig_qstore(word).map(Encoding::MortlachContigQstore),
            _ => None,
        },
        _ => None,
    }
}

/// Arm class `mortlach_contig_load`: load a tile slice of the element size
/// `msz`, scalar plus scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachContigLoad {
    Ld1bZaPRrr,
    Ld1hZaPRrr,
    Ld1wZaPRrr,
    Ld1dZaPRrr,
}

fn mortlach_contig_load(word: u32) -> Option<MortlachContigLoad> {
    let msz = bits(word, 23, 22);
    match msz {
        0b00 => Some(MortlachContigLoad::Ld1bZaPRrr),
        0b01 => Some(MortlachContigLoad::Ld1hZaPRrr),
        0b10 => Some(MortlachContigLoad::Ld1wZaPRrr),
        0b11 => Some(MortlachContigLoad::Ld1dZaPRrr),
        _ => None,
    }
}

/// Arm class `mortlach_contig_store`: store a tile slice of the element
/// size `msz`, scalar plus scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachContigStore {
    St1bZaPRrr,
    St1hZaPRrr,
    St1wZaPRrr,
    St1dZaPRrr,
}

fn mortlach_contig_store(word: u32) -> Option<MortlachContigStore> {
    let msz = bits(word, 23, 22);
    match msz {
        0b00 => Some(MortlachContigStore::St1bZaPRrr),
        0b01 => Some(MortlachContigStore::St1hZaPRrr),
        0b10 => Some(MortlachContigStore::St1wZaPRrr),
        0b11 => Some(MortlachContigStore::St1dZaPRrr),
        _ => None,
    }
}

/// Arm class `mortlach_ctxt_ldst`: load or store (`op`) a ZA array
/// vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachCtxtLdst {
    LdrZaRi,
    StrZaRi,
}

fn mortlach_ctxt_ldst(word: u32) -> Option<MortlachCtxtLdst> {
    let op = bits(word, 21, 21);
    match op {
        0b0 => Some(MortlachCtxtLdst::LdrZaRi),
        0b1 => Some(MortlachCtxtLdst::StrZaRi),
        _ => None,
    }
}

/// Arm class `mortlach_zt_ldst`: load or store (`opc`) ZT0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachZtLdst {
    LdrZtBr,
    StrZtBr,
}

fn mortlach_zt_ldst(word: u32) -> Option<MortlachZtLdst> {
    let opc = bits(word, 21, 16);
    let opc2 = bits(word, 1, 0);
    match (opc, opc2) {
        (0b011111, 0b00) => Some(MortlachZtLdst::LdrZtBr),
        (0b111111, 0b00) => Some(MortlachZtLdst::StrZtBr),
        _ => None,
    }
}

/// Arm class `mortlach_contig_qload`: load a tile slice of quadwords.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachContigQload {
    Ld1qZaPRrr,
}

fn mortlach_contig_qload(_word: u32) -> Option<MortlachContigQload> {
    Some(MortlachContigQload::Ld1qZaPRrr)
}

/// Arm class `mortlach_contig_qstore`: store a tile slice of quadwords.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MortlachContigQstore {
    St1qZaPRrr,
}

fn mortlach_contig_qstore(_word: u32) -> Option<MortlachContigQstore> {
    Some(MortlachContigQstore::St1qZaPRrr)
}
