//! Arm group `sme` (Arm ARM C4.1, "SME encodings"): the A64 space with
//! `op0 = 1, op1 = 0000`. Like `sve` it puts a level of groups between the
//! top and the classes; Arm names SME's groups and classes `mortlach_*`.
//!
//! The encodings here are those of FEAT_SME and FEAT_SME2, which are still
//! being built (`Feature::complete`). FEAT_SME's are the instructions on
//! the ZA array: the outer products into 32-bit tiles (FMOPA and FMOPS of
//! single-precision and of half-precision values, BFMOPA and BFMOPS, and
//! SMOPA to UMOPS of bytes), MOVA between a tile slice and a vector, ZERO,
//! ADDHA and ADDVA of words, the tile slice loads and stores (LD1B to LD1Q,
//! ST1B to ST1Q) and LDR and STR of a ZA array vector. FEAT_SME2's so far
//! are the contiguous loads and stores of two or four vectors (LD1B to
//! LD1D, LDNT1B to LDNT1D, ST1B to ST1D, STNT1B to STNT1D), ZERO {ZT0},
//! LDR and STR of ZT0, and MOVT between ZT0 and a general register. The
//! rest of FEAT_SME2's instructions, and the optional 64-bit forms of
//! FEAT_SME_F64F64 and FEAT_SME_I16I64, are not here, which this decoder
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

/// `op0 = 00`: the floating-point outer products, by bits 23:22 and 3:2.
/// Bits 23:22 = 10 with bits 3:2 = 10 is mortlach2_misc_prod (FEAT_SME2
/// and its extensions); 00 is mortlach2_prod4 (FEAT_SME_MOP4), 01
/// mortlach2_ss_prod (FEAT_SME_TMOP), 11 the 64-bit products
/// (FEAT_SME_F64F64, FEAT_SME_I16I64).
fn fp_outer_products(word: u32) -> Option<Encoding> {
    match (bits(word, 23, 22), bits(word, 3, 2)) {
        (0b10, 0b00) => mortlach_32bit_fp_prod(word),
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
/// with bit 24 clear. Bit 24 set holds FEAT_SME2's multi-vector groups,
/// and bits 21:19 = 011 and 110 its lookup table expansions and
/// FEAT_SME_F16F8F16's scaled products.
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
        // Bits 23:22 and 18: 00 and 1 is mortlach_multizero (FEAT_SME2p1),
        // 1x mortlach_zt_expand_ctg (FEAT_SME2 and later).
        (0b0, 0b001) => match (bits(word, 23, 22), bits(word, 18, 18)) {
            (0b00, 0b0) => mortlach_zero(word).map(Encoding::MortlachZero),
            (0b01, 0b0) => mortlach_zero_zt(word).map(Encoding::MortlachZeroZt),
            (0b01, 0b1) => mortlach_mov_zt(word),
            _ => None,
        },
        (0b0, 0b010) => match bits(word, 3, 3) {
            0b0 => mortlach_hvadd(word),
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
/// tiles. `op1 = 1` is mortlach_i16i32_prod (FEAT_SME2).
fn mortlach_32bit_int_prod(word: u32) -> Option<Encoding> {
    let op1 = bits(word, 3, 3);
    match op1 {
        0b0 => mortlach_i8i32_prod(word).map(Encoding::MortlachI8i32Prod),
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
        (0b000 | 0b001, 0b0, _) => mortlach_multi2_cld_cldnt_ss_ctg(word).map(Encoding::MortlachMulti2CldCldntSsCtg),
        (0b000 | 0b001, 0b1, 0b0) => mortlach_multi4_cld_cldnt_ss_ctg(word).map(Encoding::MortlachMulti4CldCldntSsCtg),
        (0b010 | 0b011, 0b0, _) => mortlach_multi2_cst_cstnt_ss_ctg(word).map(Encoding::MortlachMulti2CstCstntSsCtg),
        (0b010 | 0b011, 0b1, 0b0) => mortlach_multi4_cst_cstnt_ss_ctg(word).map(Encoding::MortlachMulti4CstCstntSsCtg),
        (0b100, 0b0, _) => mortlach_multi2_cld_cldnt_si_ctg(word).map(Encoding::MortlachMulti2CldCldntSiCtg),
        (0b100, 0b1, 0b0) => mortlach_multi4_cld_cldnt_si_ctg(word).map(Encoding::MortlachMulti4CldCldntSiCtg),
        (0b110, 0b0, _) => mortlach_multi2_cst_cstnt_si_ctg(word).map(Encoding::MortlachMulti2CstCstntSiCtg),
        (0b110, 0b1, 0b0) => mortlach_multi4_cst_cstnt_si_ctg(word).map(Encoding::MortlachMulti4CstCstntSiCtg),
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
        (0b000 | 0b001, 0b0, _) => mortlach_multi2_cld_cldnt_ss_nctg(word).map(Encoding::MortlachMulti2CldCldntSsNctg),
        (0b000 | 0b001, 0b1, 0b0) => mortlach_multi4_cld_cldnt_ss_nctg(word).map(Encoding::MortlachMulti4CldCldntSsNctg),
        (0b010 | 0b011, 0b0, _) => mortlach_multi2_cst_cstnt_ss_nctg(word).map(Encoding::MortlachMulti2CstCstntSsNctg),
        (0b010 | 0b011, 0b1, 0b0) => mortlach_multi4_cst_cstnt_ss_nctg(word).map(Encoding::MortlachMulti4CstCstntSsNctg),
        (0b100, 0b0, _) => mortlach_multi2_cld_cldnt_si_nctg(word).map(Encoding::MortlachMulti2CldCldntSiNctg),
        (0b100, 0b1, 0b0) => mortlach_multi4_cld_cldnt_si_nctg(word).map(Encoding::MortlachMulti4CldCldntSiNctg),
        (0b110, 0b0, _) => mortlach_multi2_cst_cstnt_si_nctg(word).map(Encoding::MortlachMulti2CstCstntSiNctg),
        (0b110, 0b1, 0b0) => mortlach_multi4_cst_cstnt_si_nctg(word).map(Encoding::MortlachMulti4CstCstntSiNctg),
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

/// Arm group `mortlach_ins`: SME move into ZA. `op1 = 1` is FEAT_SME2's
/// multi-vector moves.
fn mortlach_ins(word: u32) -> Option<Encoding> {
    let op1 = bits(word, 18, 18);
    match op1 {
        0b0 => mortlach_insert_pred(word).map(Encoding::MortlachInsertPred),
        _ => None,
    }
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

/// Arm group `mortlach_ext`: SME move from ZA. `op1 = 1` is FEAT_SME2's
/// multi-vector moves, and with `op1 = 0` bit 9 set FEAT_SME2p1's MOVAZ.
fn mortlach_ext(word: u32) -> Option<Encoding> {
    let op1 = bits(word, 18, 18);
    match (op1, bits(word, 9, 9)) {
        (0b0, 0b0) => mortlach_extract_pred(word).map(Encoding::MortlachExtractPred),
        _ => None,
    }
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
