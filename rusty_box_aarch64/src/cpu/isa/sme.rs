//! Arm group `sme` (Arm ARM C4.1, "SME encodings"): the A64 space with
//! `op0 = 1, op1 = 0000`. Like `sve` it puts a level of groups between the
//! top and the classes; Arm names SME's groups and classes `mortlach_*`.
//!
//! The encodings here are those of FEAT_SME, which is still being built
//! (`Feature::complete`): the instructions on the ZA array. They are the
//! outer products into 32-bit tiles (FMOPA and FMOPS of single-precision
//! and of half-precision values, BFMOPA and BFMOPS, and SMOPA to UMOPS of
//! bytes), MOVA between a tile slice and a vector, ZERO, ADDHA and ADDVA of
//! words, the tile slice loads and stores (LD1B to LD1Q, ST1B to ST1Q) and
//! LDR and STR of a ZA array vector. FEAT_SME2's multi-vector instructions
//! and ZT0, and the optional 64-bit forms of FEAT_SME_F64F64 and
//! FEAT_SME_I16I64, are not here, which this decoder does not know.
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
        0b01 => int_outer_products(word),
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

/// `op0 = 01`: the integer outer products, by bits 23:22 and 2. Bits 24:23
/// = 00 and 10 are mortlach_multi_mem_ctg and _nctg (FEAT_SME2), bits
/// 23:22 = 11 the 64-bit products (FEAT_SME_F64F64, FEAT_SME_I16I64).
fn int_outer_products(word: u32) -> Option<Encoding> {
    match (bits(word, 23, 22), bits(word, 2, 2)) {
        (0b10, 0b0) => mortlach_32bit_int_prod(word),
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
        // 01 mortlach_zero_zt and mortlach_mov_zt (FEAT_SME2), 1x
        // mortlach_zt_expand_ctg (FEAT_SME2 and later).
        (0b0, 0b001) => match (bits(word, 23, 22), bits(word, 18, 18)) {
            (0b00, 0b0) => mortlach_zero(word).map(Encoding::MortlachZero),
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
/// and for LDR and STR by op1, op2 and bits 12:10. `op0 = 100x` with `op2
/// = 1` is mortlach_zt_ldst (FEAT_SME2).
fn mortlach_mem(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 24, 21);
    let op1 = bits(word, 20, 16);
    let op2 = bits(word, 15, 15);
    match op0 {
        0b0000 | 0b0010 | 0b0100 | 0b0110 => match bits(word, 4, 4) {
            0b0 => mortlach_contig_load(word).map(Encoding::MortlachContigLoad),
            _ => None,
        },
        0b0001 | 0b0011 | 0b0101 | 0b0111 => match bits(word, 4, 4) {
            0b0 => mortlach_contig_store(word).map(Encoding::MortlachContigStore),
            _ => None,
        },
        0b1000 | 0b1001 => match (op1, op2, bits(word, 12, 10), bits(word, 4, 4)) {
            (0b00000, 0b0, 0b000, 0b0) => mortlach_ctxt_ldst(word).map(Encoding::MortlachCtxtLdst),
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
