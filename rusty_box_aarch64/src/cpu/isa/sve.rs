//! Arm group `sve` (Arm ARM C4.1, "SVE encodings"): the A64 space with
//! `op1 = 0010`. Its groups nest one level deeper than the other top-level
//! groups': `sve` holds groups (`sve_alloca`, ...), which hold the classes.
//!
//! The encodings here are those of FEAT_SME, which is still being built
//! (`Feature::complete`): the streaming vector length instructions, and the
//! SVE integer, permute and predicate instructions FEAT_SME makes legal in
//! Streaming SVE mode (`op0 = 000` and `001`). The SVE loads and stores,
//! the floating-point instructions and the rest of SVE2 FEAT_SME allows are
//! not here yet; neither is anything only FEAT_SVE allows, which this
//! decoder does not know.
//!
//! The variants are Arm's encoding ids, whole (the module documentation of
//! `isa` gives the naming, which `tests/unit/spec.rs` checks), and the ids
//! of one class often share their operand suffix (`add_z_p_zz_`,
//! `sub_z_p_zz_`): the names are Arm's, so the lint against a shared
//! postfix does not apply here.
#![allow(clippy::enum_variant_names)]

use super::Encoding;
use crate::cpu::exec::bits;

pub(super) fn sve(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 31, 29);
    match op0 {
        0b000 => integer_and_permute(word),
        0b001 => compare_and_predicate(word),
        _ => None,
    }
}

/// `op0 = 000`: integer arithmetic, element counts, immediates, permutes
/// and selects. By op1<7> and op1<4>.
fn integer_and_permute(word: u32) -> Option<Encoding> {
    let op2 = bits(word, 15, 10);
    match (bits(word, 24, 24), bits(word, 21, 21)) {
        // op2<5:3>.
        (0b0, 0b0) => match bits(word, 15, 13) {
            0b000 => sve_int_pred_bin(word),
            0b001 => sve_int_pred_red(word),
            0b100 => sve_int_pred_shift(word),
            0b101 => sve_int_pred_un(word),
            0b010 | 0b011 | 0b110 | 0b111 => sve_int_muladd_pred(word),
            _ => None,
        },
        // op2<5:2>; 1010 is sve_int_adr (ADR, FEAT_SVE alone).
        (0b0, 0b1) => match bits(word, 15, 12) {
            0b0000 | 0b0001 => sve_int_unpred_arit(word),
            0b0010 | 0b0011 => sve_int_unpred_logical(word),
            0b1000 | 0b1001 => sve_int_unpred_shift(word),
            0b1011 => sve_int_unpred_misc(word),
            0b0100 => sve_index(word),
            0b0101 => sve_alloca(word),
            0b0110 | 0b0111 => sve_int_unpred_arit_b(word),
            0b1100..=0b1111 => sve_countelt(word),
            _ => None,
        },
        // op1<3>.
        (0b1, 0b0) => match bits(word, 20, 20) {
            0b0 => sve_maskimm(word),
            0b1 => sve_wideimm_pred(word),
            _ => None,
        },
        // 000xxx with op1<6> = 1 is sve_perm_inter_long, 001001 and 001101
        // sve_perm_quads_a and _c (FEAT_SVE2p1 and FEAT_SME2p1).
        (0b1, 0b1) => match op2 {
            0b000000..=0b000111 => match bits(word, 23, 23) {
                0b0 => sve_perm_extract(word),
                _ => None,
            },
            0b001000 => sve_perm_unpred_a(word),
            0b001010 | 0b001011 => sve_perm_unpred_b(word),
            0b001100 => sve_perm_unpred_c(word),
            0b001110 => sve_perm_unpred_d(word),
            0b010000..=0b010111 => sve_perm_predicates(word),
            0b011000..=0b011111 => sve_perm_inter(word),
            0b100000..=0b101111 => sve_perm_pred(word),
            0b110000..=0b111111 => sve_int_select(word),
            _ => None,
        },
        _ => None,
    }
}

/// `op0 = 001`: integer compares, predicate operations, `WHILE`, predicate
/// counts and the unpredicated immediates. By op1<7> and op1<4>.
fn compare_and_predicate(word: u32) -> Option<Encoding> {
    let op3 = bits(word, 4, 4);
    match (bits(word, 24, 24), bits(word, 21, 21)) {
        (0b0, 0b0) => sve_cmpvec(word),
        (0b0, 0b1) => sve_cmpuimm(word),
        // op2<5:4>, then op1<3>.
        (0b1, 0b0) => match (bits(word, 15, 14), bits(word, 20, 20)) {
            (0b00 | 0b10, _) => sve_cmpsimm(word),
            (0b01, 0b0) => sve_pred_gen_a(word),
            (0b01, 0b1) => sve_pred_gen_c(word),
            (0b11, 0b0) => sve_pred_gen_b(word),
            (0b11, 0b1) => sve_pred_gen_d(word),
            _ => None,
        },
        // op2<5:4>; op3 = 1 under 01 is sve_while_pn (FEAT_SVE2p1 and
        // FEAT_SME2), and op2 = 1001xx under 10 is sve_pred_wrffr
        // (FEAT_SVE alone).
        (0b1, 0b1) => match bits(word, 15, 14) {
            0b00 => sve_cmpgpr(word),
            0b01 => match op3 {
                0b0 => sve_pred_dup(word),
                _ => None,
            },
            // op1<3:2>, then op2<3:2>.
            0b10 => match (bits(word, 20, 19), bits(word, 13, 12)) {
                (0b00, _) => sve_pred_count_a(word),
                (0b01, 0b00) => sve_pred_count_b(word),
                _ => None,
            },
            0b11 => sve_wideimm_unpred(word),
            _ => None,
        },
        _ => None,
    }
}

// ----- op0 = 000, op1<7> = 0, op1<4> = 0: predicated integer -----

/// Arm group `sve_int_pred_bin`: SVE integer binary operations (predicated).
fn sve_int_pred_bin(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 20, 18);
    match op0 {
        0b000 | 0b001 => sve_int_bin_pred_arit_0(word).map(Encoding::SveIntBinPredArit0),
        0b010 | 0b011 => sve_int_bin_pred_arit_1(word).map(Encoding::SveIntBinPredArit1),
        0b100 => sve_int_bin_pred_arit_2(word).map(Encoding::SveIntBinPredArit2),
        0b101 => sve_int_bin_pred_div(word).map(Encoding::SveIntBinPredDiv),
        0b110 | 0b111 => sve_int_bin_pred_log(word).map(Encoding::SveIntBinPredLog),
        _ => None,
    }
}

/// Arm group `sve_int_bin_pred_arit_0`: SVE integer add/subtract vectors
/// (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBinPredArit0 {
    AddZPZz,
    SubZPZz,
    SubrZPZz,
}

fn sve_int_bin_pred_arit_0(word: u32) -> Option<SveIntBinPredArit0> {
    let opc = bits(word, 18, 16);
    match opc {
        0b000 => Some(SveIntBinPredArit0::AddZPZz),
        0b001 => Some(SveIntBinPredArit0::SubZPZz),
        0b011 => Some(SveIntBinPredArit0::SubrZPZz),
        _ => None,
    }
}

/// Arm group `sve_int_bin_pred_arit_1`: SVE integer min/max/difference
/// (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBinPredArit1 {
    SmaxZPZz,
    SminZPZz,
    SabdZPZz,
    UmaxZPZz,
    UminZPZz,
    UabdZPZz,
}

fn sve_int_bin_pred_arit_1(word: u32) -> Option<SveIntBinPredArit1> {
    let opc = bits(word, 18, 17);
    let u = bits(word, 16, 16);
    match (opc, u) {
        (0b00, 0b0) => Some(SveIntBinPredArit1::SmaxZPZz),
        (0b01, 0b0) => Some(SveIntBinPredArit1::SminZPZz),
        (0b10, 0b0) => Some(SveIntBinPredArit1::SabdZPZz),
        (0b00, 0b1) => Some(SveIntBinPredArit1::UmaxZPZz),
        (0b01, 0b1) => Some(SveIntBinPredArit1::UminZPZz),
        (0b10, 0b1) => Some(SveIntBinPredArit1::UabdZPZz),
        _ => None,
    }
}

/// Arm group `sve_int_bin_pred_arit_2`: SVE integer multiply vectors
/// (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBinPredArit2 {
    MulZPZz,
    SmulhZPZz,
    UmulhZPZz,
}

fn sve_int_bin_pred_arit_2(word: u32) -> Option<SveIntBinPredArit2> {
    let h = bits(word, 17, 17);
    let u = bits(word, 16, 16);
    match (h, u) {
        (0b0, 0b0) => Some(SveIntBinPredArit2::MulZPZz),
        (0b1, 0b0) => Some(SveIntBinPredArit2::SmulhZPZz),
        (0b1, 0b1) => Some(SveIntBinPredArit2::UmulhZPZz),
        _ => None,
    }
}

/// Arm group `sve_int_bin_pred_div`: SVE integer divide vectors
/// (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBinPredDiv {
    SdivZPZz,
    SdivrZPZz,
    UdivZPZz,
    UdivrZPZz,
}

fn sve_int_bin_pred_div(word: u32) -> Option<SveIntBinPredDiv> {
    let r = bits(word, 17, 17);
    let u = bits(word, 16, 16);
    match (r, u) {
        (0b0, 0b0) => Some(SveIntBinPredDiv::SdivZPZz),
        (0b1, 0b0) => Some(SveIntBinPredDiv::SdivrZPZz),
        (0b0, 0b1) => Some(SveIntBinPredDiv::UdivZPZz),
        (0b1, 0b1) => Some(SveIntBinPredDiv::UdivrZPZz),
        _ => None,
    }
}

/// Arm group `sve_int_bin_pred_log`: SVE bitwise logical operations
/// (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBinPredLog {
    OrrZPZz,
    EorZPZz,
    AndZPZz,
    BicZPZz,
}

fn sve_int_bin_pred_log(word: u32) -> Option<SveIntBinPredLog> {
    let opc = bits(word, 18, 16);
    match opc {
        0b000 => Some(SveIntBinPredLog::OrrZPZz),
        0b001 => Some(SveIntBinPredLog::EorZPZz),
        0b010 => Some(SveIntBinPredLog::AndZPZz),
        0b011 => Some(SveIntBinPredLog::BicZPZz),
        _ => None,
    }
}

/// Arm group `sve_int_pred_red`: SVE integer reduction (predicated). The
/// `q` forms are FEAT_SVE2p1's and FEAT_SME2p1's.
fn sve_int_pred_red(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 20, 18);
    match op0 {
        0b000 => sve_int_reduce_0(word).map(Encoding::SveIntReduce0),
        0b010 => sve_int_reduce_1(word).map(Encoding::SveIntReduce1),
        0b100 | 0b101 => sve_int_movprfx_pred(word).map(Encoding::SveIntMovprfxPred),
        0b110 => sve_int_reduce_2(word).map(Encoding::SveIntReduce2),
        _ => None,
    }
}

/// Arm group `sve_int_reduce_0`: SVE integer add reduction (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntReduce0 {
    SaddvRPZ,
    UaddvRPZ,
}

fn sve_int_reduce_0(word: u32) -> Option<SveIntReduce0> {
    let op = bits(word, 17, 17);
    let u = bits(word, 16, 16);
    match (op, u) {
        (0b0, 0b0) => Some(SveIntReduce0::SaddvRPZ),
        (0b0, 0b1) => Some(SveIntReduce0::UaddvRPZ),
        _ => None,
    }
}

/// Arm group `sve_int_reduce_1`: SVE integer min/max reduction
/// (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntReduce1 {
    SmaxvRPZ,
    SminvRPZ,
    UmaxvRPZ,
    UminvRPZ,
}

fn sve_int_reduce_1(word: u32) -> Option<SveIntReduce1> {
    let op = bits(word, 17, 17);
    let u = bits(word, 16, 16);
    match (op, u) {
        (0b0, 0b0) => Some(SveIntReduce1::SmaxvRPZ),
        (0b1, 0b0) => Some(SveIntReduce1::SminvRPZ),
        (0b0, 0b1) => Some(SveIntReduce1::UmaxvRPZ),
        (0b1, 0b1) => Some(SveIntReduce1::UminvRPZ),
        _ => None,
    }
}

/// Arm group `sve_int_movprfx_pred`: SVE constructive prefix (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntMovprfxPred {
    MovprfxZPZ,
}

fn sve_int_movprfx_pred(word: u32) -> Option<SveIntMovprfxPred> {
    let opc = bits(word, 18, 17);
    match opc {
        0b00 => Some(SveIntMovprfxPred::MovprfxZPZ),
        _ => None,
    }
}

/// Arm group `sve_int_reduce_2`: SVE bitwise logical reduction
/// (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntReduce2 {
    OrvRPZ,
    EorvRPZ,
    AndvRPZ,
}

fn sve_int_reduce_2(word: u32) -> Option<SveIntReduce2> {
    let opc = bits(word, 17, 16);
    match opc {
        0b00 => Some(SveIntReduce2::OrvRPZ),
        0b01 => Some(SveIntReduce2::EorvRPZ),
        0b10 => Some(SveIntReduce2::AndvRPZ),
        _ => None,
    }
}

/// Arm group `sve_int_pred_shift`: SVE bitwise shift (predicated).
fn sve_int_pred_shift(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 20, 19);
    match op0 {
        0b00 | 0b01 => sve_int_bin_pred_shift_0(word).map(Encoding::SveIntBinPredShift0),
        0b10 => sve_int_bin_pred_shift_1(word).map(Encoding::SveIntBinPredShift1),
        0b11 => sve_int_bin_pred_shift_2(word).map(Encoding::SveIntBinPredShift2),
        _ => None,
    }
}

/// Arm group `sve_int_bin_pred_shift_0`: SVE bitwise shift by immediate
/// (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBinPredShift0 {
    AsrZPZi,
    LslZPZi,
    AsrdZPZi,
    SqshlZPZi,
    SrshrZPZi,
    SqshluZPZi,
    LsrZPZi,
    UqshlZPZi,
    UrshrZPZi,
}

fn sve_int_bin_pred_shift_0(word: u32) -> Option<SveIntBinPredShift0> {
    let opc = bits(word, 19, 18);
    let l = bits(word, 17, 17);
    let u = bits(word, 16, 16);
    match (opc, l, u) {
        (0b00, 0b0, 0b0) => Some(SveIntBinPredShift0::AsrZPZi),
        (0b00, 0b1, 0b1) => Some(SveIntBinPredShift0::LslZPZi),
        (0b01, 0b0, 0b0) => Some(SveIntBinPredShift0::AsrdZPZi),
        (0b01, 0b1, 0b0) => Some(SveIntBinPredShift0::SqshlZPZi),
        (0b11, 0b0, 0b0) => Some(SveIntBinPredShift0::SrshrZPZi),
        (0b11, 0b1, 0b1) => Some(SveIntBinPredShift0::SqshluZPZi),
        (0b00, 0b0, 0b1) => Some(SveIntBinPredShift0::LsrZPZi),
        (0b01, 0b1, 0b1) => Some(SveIntBinPredShift0::UqshlZPZi),
        (0b11, 0b0, 0b1) => Some(SveIntBinPredShift0::UrshrZPZi),
        _ => None,
    }
}

/// Arm group `sve_int_bin_pred_shift_1`: SVE bitwise shift by vector
/// (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBinPredShift1 {
    AsrZPZz,
    LslZPZz,
    AsrrZPZz,
    LslrZPZz,
    LsrZPZz,
    LsrrZPZz,
}

fn sve_int_bin_pred_shift_1(word: u32) -> Option<SveIntBinPredShift1> {
    let r = bits(word, 18, 18);
    let l = bits(word, 17, 17);
    let u = bits(word, 16, 16);
    match (r, l, u) {
        (0b0, 0b0, 0b0) => Some(SveIntBinPredShift1::AsrZPZz),
        (0b0, 0b1, 0b1) => Some(SveIntBinPredShift1::LslZPZz),
        (0b1, 0b0, 0b0) => Some(SveIntBinPredShift1::AsrrZPZz),
        (0b1, 0b1, 0b1) => Some(SveIntBinPredShift1::LslrZPZz),
        (0b0, 0b0, 0b1) => Some(SveIntBinPredShift1::LsrZPZz),
        (0b1, 0b0, 0b1) => Some(SveIntBinPredShift1::LsrrZPZz),
        _ => None,
    }
}

/// Arm group `sve_int_bin_pred_shift_2`: SVE bitwise shift by wide elements
/// (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBinPredShift2 {
    AsrZPZw,
    LslZPZw,
    LsrZPZw,
}

fn sve_int_bin_pred_shift_2(word: u32) -> Option<SveIntBinPredShift2> {
    let r = bits(word, 18, 18);
    let l = bits(word, 17, 17);
    let u = bits(word, 16, 16);
    match (r, l, u) {
        (0b0, 0b0, 0b0) => Some(SveIntBinPredShift2::AsrZPZw),
        (0b0, 0b1, 0b1) => Some(SveIntBinPredShift2::LslZPZw),
        (0b0, 0b0, 0b1) => Some(SveIntBinPredShift2::LsrZPZw),
        _ => None,
    }
}

/// Arm group `sve_int_pred_un`: SVE integer unary operations (predicated).
fn sve_int_pred_un(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 20, 19);
    match op0 {
        0b00 | 0b10 => sve_int_un_pred_arit_0(word).map(Encoding::SveIntUnPredArit0),
        0b01 | 0b11 => sve_int_un_pred_arit_1(word).map(Encoding::SveIntUnPredArit1),
        _ => None,
    }
}

/// Arm group `sve_int_un_pred_arit_0`: SVE integer unary operations
/// (predicated). `M = 0` is the zeroing forms of FEAT_SVE2p2 and
/// FEAT_SME2p2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntUnPredArit0 {
    AbsZPZM,
    NegZPZM,
    SxtwZPZM,
    SxthZPZM,
    SxtbZPZM,
    UxtwZPZM,
    UxthZPZM,
    UxtbZPZM,
}

fn sve_int_un_pred_arit_0(word: u32) -> Option<SveIntUnPredArit0> {
    let m = bits(word, 20, 20);
    let opc = bits(word, 18, 16);
    match (m, opc) {
        (0b1, 0b110) => Some(SveIntUnPredArit0::AbsZPZM),
        (0b1, 0b111) => Some(SveIntUnPredArit0::NegZPZM),
        (0b1, 0b100) => Some(SveIntUnPredArit0::SxtwZPZM),
        (0b1, 0b010) => Some(SveIntUnPredArit0::SxthZPZM),
        (0b1, 0b000) => Some(SveIntUnPredArit0::SxtbZPZM),
        (0b1, 0b101) => Some(SveIntUnPredArit0::UxtwZPZM),
        (0b1, 0b011) => Some(SveIntUnPredArit0::UxthZPZM),
        (0b1, 0b001) => Some(SveIntUnPredArit0::UxtbZPZM),
        _ => None,
    }
}

/// Arm group `sve_int_un_pred_arit_1`: SVE bitwise unary operations
/// (predicated). `M = 0` is the zeroing forms of FEAT_SVE2p2 and
/// FEAT_SME2p2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntUnPredArit1 {
    ClsZPZM,
    ClzZPZM,
    CntZPZM,
    CnotZPZM,
    FabsZPZM,
    FnegZPZM,
    NotZPZM,
}

fn sve_int_un_pred_arit_1(word: u32) -> Option<SveIntUnPredArit1> {
    let m = bits(word, 20, 20);
    let opc = bits(word, 18, 16);
    match (m, opc) {
        (0b1, 0b000) => Some(SveIntUnPredArit1::ClsZPZM),
        (0b1, 0b001) => Some(SveIntUnPredArit1::ClzZPZM),
        (0b1, 0b010) => Some(SveIntUnPredArit1::CntZPZM),
        (0b1, 0b011) => Some(SveIntUnPredArit1::CnotZPZM),
        (0b1, 0b100) => Some(SveIntUnPredArit1::FabsZPZM),
        (0b1, 0b101) => Some(SveIntUnPredArit1::FnegZPZM),
        (0b1, 0b110) => Some(SveIntUnPredArit1::NotZPZM),
        _ => None,
    }
}

/// Arm group `sve_int_muladd_pred`: SVE integer multiply-accumulate
/// (predicated).
fn sve_int_muladd_pred(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 15, 15);
    match op0 {
        0b0 => sve_int_mlas_vvv_pred(word).map(Encoding::SveIntMlasVvvPred),
        0b1 => sve_int_mladdsub_vvv_pred(word).map(Encoding::SveIntMladdsubVvvPred),
        _ => None,
    }
}

/// Arm group `sve_int_mlas_vvv_pred`: SVE integer multiply-accumulate
/// writing addend (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntMlasVvvPred {
    MlaZPZzz,
    MlsZPZzz,
}

fn sve_int_mlas_vvv_pred(word: u32) -> Option<SveIntMlasVvvPred> {
    let op = bits(word, 13, 13);
    match op {
        0b0 => Some(SveIntMlasVvvPred::MlaZPZzz),
        0b1 => Some(SveIntMlasVvvPred::MlsZPZzz),
        _ => None,
    }
}

/// Arm group `sve_int_mladdsub_vvv_pred`: SVE integer multiply-add writing
/// multiplicand (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntMladdsubVvvPred {
    MadZPZzz,
    MsbZPZzz,
}

fn sve_int_mladdsub_vvv_pred(word: u32) -> Option<SveIntMladdsubVvvPred> {
    let op = bits(word, 13, 13);
    match op {
        0b0 => Some(SveIntMladdsubVvvPred::MadZPZzz),
        0b1 => Some(SveIntMladdsubVvvPred::MsbZPZzz),
        _ => None,
    }
}

// ----- op0 = 000, op1<7> = 0, op1<4> = 1: unpredicated integer -----

/// Arm group `sve_int_unpred_arit`: SVE integer add/subtract vectors
/// (unpredicated).
fn sve_int_unpred_arit(word: u32) -> Option<Encoding> {
    sve_int_bin_cons_arit_0(word).map(Encoding::SveIntBinConsArit0)
}

/// Arm group `sve_int_bin_cons_arit_0`: SVE integer add/subtract vectors
/// (unpredicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBinConsArit0 {
    AddZZz,
    SubZZz,
    SqaddZZz,
    SqsubZZz,
    UqaddZZz,
    UqsubZZz,
}

fn sve_int_bin_cons_arit_0(word: u32) -> Option<SveIntBinConsArit0> {
    let opc = bits(word, 12, 10);
    match opc {
        0b000 => Some(SveIntBinConsArit0::AddZZz),
        0b001 => Some(SveIntBinConsArit0::SubZZz),
        0b100 => Some(SveIntBinConsArit0::SqaddZZz),
        0b110 => Some(SveIntBinConsArit0::SqsubZZz),
        0b101 => Some(SveIntBinConsArit0::UqaddZZz),
        0b111 => Some(SveIntBinConsArit0::UqsubZZz),
        _ => None,
    }
}

/// Arm group `sve_int_unpred_logical`: SVE bitwise logical operations
/// (unpredicated).
fn sve_int_unpred_logical(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 12, 10);
    match op0 {
        0b100 => sve_int_bin_cons_log(word).map(Encoding::SveIntBinConsLog),
        0b101 => sve_int_rotate_imm(word).map(Encoding::SveIntRotateImm),
        0b110 | 0b111 => sve_int_tern_log(word).map(Encoding::SveIntTernLog),
        _ => None,
    }
}

/// Arm group `sve_int_bin_cons_log`: SVE bitwise logical operations
/// (unpredicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBinConsLog {
    AndZZz,
    OrrZZz,
    EorZZz,
    BicZZz,
}

fn sve_int_bin_cons_log(word: u32) -> Option<SveIntBinConsLog> {
    let opc = bits(word, 23, 22);
    match opc {
        0b00 => Some(SveIntBinConsLog::AndZZz),
        0b01 => Some(SveIntBinConsLog::OrrZZz),
        0b10 => Some(SveIntBinConsLog::EorZZz),
        0b11 => Some(SveIntBinConsLog::BicZZz),
        _ => None,
    }
}

/// Arm group `sve_int_rotate_imm`: SVE2 bitwise exclusive-or and rotate
/// right by immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntRotateImm {
    XarZZzi,
}

fn sve_int_rotate_imm(_word: u32) -> Option<SveIntRotateImm> {
    Some(SveIntRotateImm::XarZZzi)
}

/// Arm group `sve_int_tern_log`: SVE2 bitwise ternary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntTernLog {
    Eor3ZZzz,
    BcaxZZzz,
    BslZZzz,
    Bsl1nZZzz,
    Bsl2nZZzz,
    NbslZZzz,
}

fn sve_int_tern_log(word: u32) -> Option<SveIntTernLog> {
    let opc = bits(word, 23, 22);
    let o2 = bits(word, 10, 10);
    match (opc, o2) {
        (0b00, 0b0) => Some(SveIntTernLog::Eor3ZZzz),
        (0b01, 0b0) => Some(SveIntTernLog::BcaxZZzz),
        (0b00, 0b1) => Some(SveIntTernLog::BslZZzz),
        (0b01, 0b1) => Some(SveIntTernLog::Bsl1nZZzz),
        (0b10, 0b1) => Some(SveIntTernLog::Bsl2nZZzz),
        (0b11, 0b1) => Some(SveIntTernLog::NbslZZzz),
        _ => None,
    }
}

/// Arm group `sve_int_unpred_shift`: SVE bitwise shift (unpredicated).
fn sve_int_unpred_shift(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 12, 12);
    match op0 {
        0b0 => sve_int_bin_cons_shift_a(word).map(Encoding::SveIntBinConsShiftA),
        0b1 => sve_int_bin_cons_shift_b(word).map(Encoding::SveIntBinConsShiftB),
        _ => None,
    }
}

/// Arm group `sve_int_bin_cons_shift_a`: SVE bitwise shift by wide elements
/// (unpredicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBinConsShiftA {
    AsrZZw,
    LslZZw,
    LsrZZw,
}

fn sve_int_bin_cons_shift_a(word: u32) -> Option<SveIntBinConsShiftA> {
    let opc = bits(word, 11, 10);
    match opc {
        0b00 => Some(SveIntBinConsShiftA::AsrZZw),
        0b11 => Some(SveIntBinConsShiftA::LslZZw),
        0b01 => Some(SveIntBinConsShiftA::LsrZZw),
        _ => None,
    }
}

/// Arm group `sve_int_bin_cons_shift_b`: SVE bitwise shift by immediate
/// (unpredicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBinConsShiftB {
    AsrZZi,
    LslZZi,
    LsrZZi,
}

fn sve_int_bin_cons_shift_b(word: u32) -> Option<SveIntBinConsShiftB> {
    let opc = bits(word, 11, 10);
    match opc {
        0b00 => Some(SveIntBinConsShiftB::AsrZZi),
        0b11 => Some(SveIntBinConsShiftB::LslZZi),
        0b01 => Some(SveIntBinConsShiftB::LsrZZi),
        _ => None,
    }
}

/// Arm group `sve_int_unpred_misc`: SVE integer misc (unpredicated). `op0 =
/// 0x` and `10` are FTSSEL and FEXPA (FEAT_SVE alone).
fn sve_int_unpred_misc(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 11, 10);
    match op0 {
        0b11 => sve_int_bin_cons_misc_0_d(word).map(Encoding::SveIntBinConsMisc0D),
        _ => None,
    }
}

/// Arm group `sve_int_bin_cons_misc_0_d`: SVE constructive prefix
/// (unpredicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBinConsMisc0D {
    MovprfxZZ,
}

fn sve_int_bin_cons_misc_0_d(word: u32) -> Option<SveIntBinConsMisc0D> {
    let opc = bits(word, 23, 22);
    let opc2 = bits(word, 20, 16);
    match (opc, opc2) {
        (0b00, 0b00000) => Some(SveIntBinConsMisc0D::MovprfxZZ),
        _ => None,
    }
}

/// Arm group `sve_index`: SVE index generation.
fn sve_index(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 11, 10);
    match op0 {
        0b00 => sve_int_index_ii(word).map(Encoding::SveIntIndexIi),
        0b01 => sve_int_index_ri(word).map(Encoding::SveIntIndexRi),
        0b10 => sve_int_index_ir(word).map(Encoding::SveIntIndexIr),
        0b11 => sve_int_index_rr(word).map(Encoding::SveIntIndexRr),
        _ => None,
    }
}

/// Arm group `sve_int_index_ii`: SVE index generation (immediate start,
/// immediate increment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntIndexIi {
    IndexZIi,
}

fn sve_int_index_ii(_word: u32) -> Option<SveIntIndexIi> {
    Some(SveIntIndexIi::IndexZIi)
}

/// Arm group `sve_int_index_ri`: SVE index generation (register start,
/// immediate increment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntIndexRi {
    IndexZRi,
}

fn sve_int_index_ri(_word: u32) -> Option<SveIntIndexRi> {
    Some(SveIntIndexRi::IndexZRi)
}

/// Arm group `sve_int_index_ir`: SVE index generation (immediate start,
/// register increment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntIndexIr {
    IndexZIr,
}

fn sve_int_index_ir(_word: u32) -> Option<SveIntIndexIr> {
    Some(SveIntIndexIr::IndexZIr)
}

/// Arm group `sve_int_index_rr`: SVE index generation (register start,
/// register increment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntIndexRr {
    IndexZRr,
}

fn sve_int_index_rr(_word: u32) -> Option<SveIntIndexRr> {
    Some(SveIntIndexRr::IndexZRr)
}

/// Arm group `sve_alloca`: SVE stack frame adjustment and size.
fn sve_alloca(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 23, 23);
    let op1 = bits(word, 11, 11);
    match (op0, op1) {
        (0b0, 0b0) => sve_int_arith_vl(word).map(Encoding::SveIntArithVl),
        (0b0, 0b1) => sve_int_arith_svl(word).map(Encoding::SveIntArithSvl),
        (0b1, 0b0) => sve_int_read_vl_a(word).map(Encoding::SveIntReadVlA),
        (0b1, 0b1) => sve_int_read_svl_a(word).map(Encoding::SveIntReadSvlA),
        _ => None,
    }
}

/// Arm group `sve_int_arith_vl`: SVE stack frame adjustment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntArithVl {
    AddvlRRi,
    AddplRRi,
}

fn sve_int_arith_vl(word: u32) -> Option<SveIntArithVl> {
    let op = bits(word, 22, 22);
    match op {
        0b0 => Some(SveIntArithVl::AddvlRRi),
        0b1 => Some(SveIntArithVl::AddplRRi),
        _ => None,
    }
}

/// Arm group `sve_int_arith_svl`: SVE stack frame adjustment (streaming).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntArithSvl {
    AddsvlRRi,
    AddsplRRi,
}

fn sve_int_arith_svl(word: u32) -> Option<SveIntArithSvl> {
    let op = bits(word, 22, 22);
    match op {
        0b0 => Some(SveIntArithSvl::AddsvlRRi),
        0b1 => Some(SveIntArithSvl::AddsplRRi),
        _ => None,
    }
}

/// Arm group `sve_int_read_vl_a`: SVE stack frame size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntReadVlA {
    RdvlRI,
}

fn sve_int_read_vl_a(word: u32) -> Option<SveIntReadVlA> {
    let op = bits(word, 22, 22);
    let opc2 = bits(word, 20, 16);
    match (op, opc2) {
        (0b0, 0b11111) => Some(SveIntReadVlA::RdvlRI),
        _ => None,
    }
}

/// Arm group `sve_int_read_svl_a`: Streaming SVE stack frame size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntReadSvlA {
    RdsvlRI,
}

fn sve_int_read_svl_a(word: u32) -> Option<SveIntReadSvlA> {
    let op = bits(word, 22, 22);
    let opc2 = bits(word, 20, 16);
    match (op, opc2) {
        (0b0, 0b11111) => Some(SveIntReadSvlA::RdsvlRI),
        _ => None,
    }
}

/// Arm group `sve_int_unpred_arit_b`: SVE2 integer multiply
/// (unpredicated). `op0 = 11x` is FEAT_SVE2p1's and FEAT_SME2p1's.
fn sve_int_unpred_arit_b(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 12, 10);
    match op0 {
        0b000..=0b011 => sve_int_mul_b(word).map(Encoding::SveIntMulB),
        0b100 | 0b101 => sve_int_sqdmulh(word).map(Encoding::SveIntSqdmulh),
        _ => None,
    }
}

/// Arm group `sve_int_mul_b`: SVE2 integer multiply vectors (unpredicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntMulB {
    MulZZz,
    PmulZZz,
    SmulhZZz,
    UmulhZZz,
}

fn sve_int_mul_b(word: u32) -> Option<SveIntMulB> {
    let size = bits(word, 23, 22);
    let opc = bits(word, 11, 10);
    match opc {
        0b00 => Some(SveIntMulB::MulZZz),
        0b01 => match size {
            0b00 => Some(SveIntMulB::PmulZZz),
            _ => None,
        },
        0b10 => Some(SveIntMulB::SmulhZZz),
        0b11 => Some(SveIntMulB::UmulhZZz),
        _ => None,
    }
}

/// Arm group `sve_int_sqdmulh`: SVE2 signed saturating doubling multiply
/// high (unpredicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntSqdmulh {
    SqdmulhZZz,
    SqrdmulhZZz,
}

fn sve_int_sqdmulh(word: u32) -> Option<SveIntSqdmulh> {
    let r = bits(word, 10, 10);
    match r {
        0b0 => Some(SveIntSqdmulh::SqdmulhZZz),
        0b1 => Some(SveIntSqdmulh::SqrdmulhZZz),
        _ => None,
    }
}

/// Arm group `sve_countelt`: SVE element count.
fn sve_countelt(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 20, 20);
    let op1 = bits(word, 13, 11);
    match (op0, op1) {
        (0b0, 0b000 | 0b001) => sve_int_countvlv0(word).map(Encoding::SveIntCountvlv0),
        (0b1, 0b000) => sve_int_countvlv1(word).map(Encoding::SveIntCountvlv1),
        (0b0, 0b100) => sve_int_count(word).map(Encoding::SveIntCount),
        (0b1, 0b100) => sve_int_pred_pattern_a(word).map(Encoding::SveIntPredPatternA),
        (_, 0b110 | 0b111) => sve_int_pred_pattern_b(word).map(Encoding::SveIntPredPatternB),
        _ => None,
    }
}

/// Arm group `sve_int_countvlv0`: SVE saturating inc/dec vector by element
/// count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntCountvlv0 {
    SqinchZZs,
    SqdechZZs,
    UqinchZZs,
    UqdechZZs,
    SqincwZZs,
    SqdecwZZs,
    UqincwZZs,
    UqdecwZZs,
    SqincdZZs,
    SqdecdZZs,
    UqincdZZs,
    UqdecdZZs,
}

fn sve_int_countvlv0(word: u32) -> Option<SveIntCountvlv0> {
    let size = bits(word, 23, 22);
    let d = bits(word, 11, 11);
    let u = bits(word, 10, 10);
    match (size, d, u) {
        (0b01, 0b0, 0b0) => Some(SveIntCountvlv0::SqinchZZs),
        (0b01, 0b1, 0b0) => Some(SveIntCountvlv0::SqdechZZs),
        (0b01, 0b0, 0b1) => Some(SveIntCountvlv0::UqinchZZs),
        (0b01, 0b1, 0b1) => Some(SveIntCountvlv0::UqdechZZs),
        (0b10, 0b0, 0b0) => Some(SveIntCountvlv0::SqincwZZs),
        (0b10, 0b1, 0b0) => Some(SveIntCountvlv0::SqdecwZZs),
        (0b10, 0b0, 0b1) => Some(SveIntCountvlv0::UqincwZZs),
        (0b10, 0b1, 0b1) => Some(SveIntCountvlv0::UqdecwZZs),
        (0b11, 0b0, 0b0) => Some(SveIntCountvlv0::SqincdZZs),
        (0b11, 0b1, 0b0) => Some(SveIntCountvlv0::SqdecdZZs),
        (0b11, 0b0, 0b1) => Some(SveIntCountvlv0::UqincdZZs),
        (0b11, 0b1, 0b1) => Some(SveIntCountvlv0::UqdecdZZs),
        _ => None,
    }
}

/// Arm group `sve_int_countvlv1`: SVE inc/dec vector by element count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntCountvlv1 {
    InchZZs,
    DechZZs,
    IncwZZs,
    DecwZZs,
    IncdZZs,
    DecdZZs,
}

fn sve_int_countvlv1(word: u32) -> Option<SveIntCountvlv1> {
    let size = bits(word, 23, 22);
    let d = bits(word, 10, 10);
    match (size, d) {
        (0b01, 0b0) => Some(SveIntCountvlv1::InchZZs),
        (0b01, 0b1) => Some(SveIntCountvlv1::DechZZs),
        (0b10, 0b0) => Some(SveIntCountvlv1::IncwZZs),
        (0b10, 0b1) => Some(SveIntCountvlv1::DecwZZs),
        (0b11, 0b0) => Some(SveIntCountvlv1::IncdZZs),
        (0b11, 0b1) => Some(SveIntCountvlv1::DecdZZs),
        _ => None,
    }
}

/// Arm group `sve_int_count`: SVE element count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntCount {
    CntbRS,
    CnthRS,
    CntwRS,
    CntdRS,
}

fn sve_int_count(word: u32) -> Option<SveIntCount> {
    let size = bits(word, 23, 22);
    let op = bits(word, 10, 10);
    match (size, op) {
        (0b00, 0b0) => Some(SveIntCount::CntbRS),
        (0b01, 0b0) => Some(SveIntCount::CnthRS),
        (0b10, 0b0) => Some(SveIntCount::CntwRS),
        (0b11, 0b0) => Some(SveIntCount::CntdRS),
        _ => None,
    }
}

/// Arm group `sve_int_pred_pattern_a`: SVE inc/dec register by element
/// count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPredPatternA {
    IncbRRs,
    DecbRRs,
    InchRRs,
    DechRRs,
    IncwRRs,
    DecwRRs,
    IncdRRs,
    DecdRRs,
}

fn sve_int_pred_pattern_a(word: u32) -> Option<SveIntPredPatternA> {
    let size = bits(word, 23, 22);
    let d = bits(word, 10, 10);
    match (size, d) {
        (0b00, 0b0) => Some(SveIntPredPatternA::IncbRRs),
        (0b00, 0b1) => Some(SveIntPredPatternA::DecbRRs),
        (0b01, 0b0) => Some(SveIntPredPatternA::InchRRs),
        (0b01, 0b1) => Some(SveIntPredPatternA::DechRRs),
        (0b10, 0b0) => Some(SveIntPredPatternA::IncwRRs),
        (0b10, 0b1) => Some(SveIntPredPatternA::DecwRRs),
        (0b11, 0b0) => Some(SveIntPredPatternA::IncdRRs),
        (0b11, 0b1) => Some(SveIntPredPatternA::DecdRRs),
        _ => None,
    }
}

/// Arm group `sve_int_pred_pattern_b`: SVE saturating inc/dec register by
/// element count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPredPatternB {
    SqincbRRsSx,
    UqincbRRsUw,
    SqdecbRRsSx,
    UqdecbRRsUw,
    SqincbRRsX,
    SqdecbRRsX,
    SqinchRRsSx,
    UqinchRRsUw,
    SqdechRRsSx,
    UqdechRRsUw,
    SqinchRRsX,
    SqdechRRsX,
    SqincwRRsSx,
    UqincwRRsUw,
    SqdecwRRsSx,
    UqdecwRRsUw,
    SqincwRRsX,
    SqdecwRRsX,
    SqincdRRsSx,
    UqincdRRsUw,
    SqdecdRRsSx,
    UqdecdRRsUw,
    SqincdRRsX,
    SqdecdRRsX,
    UqincbRRsX,
    UqdecbRRsX,
    UqinchRRsX,
    UqdechRRsX,
    UqincwRRsX,
    UqdecwRRsX,
    UqincdRRsX,
    UqdecdRRsX,
}

fn sve_int_pred_pattern_b(word: u32) -> Option<SveIntPredPatternB> {
    let size = bits(word, 23, 22);
    let sf = bits(word, 20, 20);
    let d = bits(word, 11, 11);
    let u = bits(word, 10, 10);
    match (size, sf, d, u) {
        (0b00, 0b0, 0b0, 0b0) => Some(SveIntPredPatternB::SqincbRRsSx),
        (0b00, 0b0, 0b0, 0b1) => Some(SveIntPredPatternB::UqincbRRsUw),
        (0b00, 0b0, 0b1, 0b0) => Some(SveIntPredPatternB::SqdecbRRsSx),
        (0b00, 0b0, 0b1, 0b1) => Some(SveIntPredPatternB::UqdecbRRsUw),
        (0b00, 0b1, 0b0, 0b0) => Some(SveIntPredPatternB::SqincbRRsX),
        (0b00, 0b1, 0b1, 0b0) => Some(SveIntPredPatternB::SqdecbRRsX),
        (0b01, 0b0, 0b0, 0b0) => Some(SveIntPredPatternB::SqinchRRsSx),
        (0b01, 0b0, 0b0, 0b1) => Some(SveIntPredPatternB::UqinchRRsUw),
        (0b01, 0b0, 0b1, 0b0) => Some(SveIntPredPatternB::SqdechRRsSx),
        (0b01, 0b0, 0b1, 0b1) => Some(SveIntPredPatternB::UqdechRRsUw),
        (0b01, 0b1, 0b0, 0b0) => Some(SveIntPredPatternB::SqinchRRsX),
        (0b01, 0b1, 0b1, 0b0) => Some(SveIntPredPatternB::SqdechRRsX),
        (0b10, 0b0, 0b0, 0b0) => Some(SveIntPredPatternB::SqincwRRsSx),
        (0b10, 0b0, 0b0, 0b1) => Some(SveIntPredPatternB::UqincwRRsUw),
        (0b10, 0b0, 0b1, 0b0) => Some(SveIntPredPatternB::SqdecwRRsSx),
        (0b10, 0b0, 0b1, 0b1) => Some(SveIntPredPatternB::UqdecwRRsUw),
        (0b10, 0b1, 0b0, 0b0) => Some(SveIntPredPatternB::SqincwRRsX),
        (0b10, 0b1, 0b1, 0b0) => Some(SveIntPredPatternB::SqdecwRRsX),
        (0b11, 0b0, 0b0, 0b0) => Some(SveIntPredPatternB::SqincdRRsSx),
        (0b11, 0b0, 0b0, 0b1) => Some(SveIntPredPatternB::UqincdRRsUw),
        (0b11, 0b0, 0b1, 0b0) => Some(SveIntPredPatternB::SqdecdRRsSx),
        (0b11, 0b0, 0b1, 0b1) => Some(SveIntPredPatternB::UqdecdRRsUw),
        (0b11, 0b1, 0b0, 0b0) => Some(SveIntPredPatternB::SqincdRRsX),
        (0b11, 0b1, 0b1, 0b0) => Some(SveIntPredPatternB::SqdecdRRsX),
        (0b00, 0b1, 0b0, 0b1) => Some(SveIntPredPatternB::UqincbRRsX),
        (0b00, 0b1, 0b1, 0b1) => Some(SveIntPredPatternB::UqdecbRRsX),
        (0b01, 0b1, 0b0, 0b1) => Some(SveIntPredPatternB::UqinchRRsX),
        (0b01, 0b1, 0b1, 0b1) => Some(SveIntPredPatternB::UqdechRRsX),
        (0b10, 0b1, 0b0, 0b1) => Some(SveIntPredPatternB::UqincwRRsX),
        (0b10, 0b1, 0b1, 0b1) => Some(SveIntPredPatternB::UqdecwRRsX),
        (0b11, 0b1, 0b0, 0b1) => Some(SveIntPredPatternB::UqincdRRsX),
        (0b11, 0b1, 0b1, 0b1) => Some(SveIntPredPatternB::UqdecdRRsX),
        _ => None,
    }
}

// ----- op0 = 000, op1<7> = 1, op1<4> = 0: wide immediates -----

/// Arm group `sve_maskimm`: SVE bitwise logical with immediate and
/// broadcast bitmask immediate.
fn sve_maskimm(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 23, 22);
    let op1 = bits(word, 19, 18);
    match (op0, op1) {
        (0b00..=0b10, 0b00) => sve_int_log_imm(word).map(Encoding::SveIntLogImm),
        (0b11, 0b00) => sve_int_dup_mask_imm(word).map(Encoding::SveIntDupMaskImm),
        _ => None,
    }
}

/// Arm group `sve_int_log_imm`: SVE bitwise logical with immediate
/// (unpredicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntLogImm {
    OrrZZi,
    EorZZi,
    AndZZi,
}

fn sve_int_log_imm(word: u32) -> Option<SveIntLogImm> {
    let opc = bits(word, 23, 22);
    match opc {
        0b00 => Some(SveIntLogImm::OrrZZi),
        0b01 => Some(SveIntLogImm::EorZZi),
        0b10 => Some(SveIntLogImm::AndZZi),
        _ => None,
    }
}

/// Arm group `sve_int_dup_mask_imm`: SVE broadcast bitmask immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntDupMaskImm {
    DupmZI,
}

fn sve_int_dup_mask_imm(_word: u32) -> Option<SveIntDupMaskImm> {
    Some(SveIntDupMaskImm::DupmZI)
}

/// Arm group `sve_wideimm_pred`: SVE integer and floating-point copy
/// immediate (predicated).
fn sve_wideimm_pred(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 15, 13);
    match op0 {
        0b000..=0b011 => sve_int_dup_imm_pred(word).map(Encoding::SveIntDupImmPred),
        0b110 => sve_int_dup_fpimm_pred(word).map(Encoding::SveIntDupFpimmPred),
        _ => None,
    }
}

/// Arm group `sve_int_dup_imm_pred`: SVE copy integer immediate
/// (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntDupImmPred {
    CpyZOI,
    CpyZPI,
}

fn sve_int_dup_imm_pred(word: u32) -> Option<SveIntDupImmPred> {
    let m = bits(word, 14, 14);
    match m {
        0b0 => Some(SveIntDupImmPred::CpyZOI),
        0b1 => Some(SveIntDupImmPred::CpyZPI),
        _ => None,
    }
}

/// Arm group `sve_int_dup_fpimm_pred`: SVE copy floating-point immediate
/// (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntDupFpimmPred {
    FcpyZPI,
}

fn sve_int_dup_fpimm_pred(_word: u32) -> Option<SveIntDupFpimmPred> {
    Some(SveIntDupFpimmPred::FcpyZPI)
}

// ----- op0 = 000, op1<7> = 1, op1<4> = 1: permutes and selects -----

/// Arm group `sve_perm_extract`: SVE permute vector extract.
fn sve_perm_extract(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 22, 22);
    match op0 {
        0b0 => sve_int_perm_extract_i(word).map(Encoding::SveIntPermExtractI),
        0b1 => sve_intx_perm_extract_i(word).map(Encoding::SveIntxPermExtractI),
        _ => None,
    }
}

/// Arm group `sve_int_perm_extract_i`: SVE extract vector (immediate
/// offset, destructive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermExtractI {
    ExtZZiDes,
}

fn sve_int_perm_extract_i(_word: u32) -> Option<SveIntPermExtractI> {
    Some(SveIntPermExtractI::ExtZZiDes)
}

/// Arm group `sve_intx_perm_extract_i`: SVE2 extract vector (immediate
/// offset, constructive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntxPermExtractI {
    ExtZZiCon,
}

fn sve_intx_perm_extract_i(_word: u32) -> Option<SveIntxPermExtractI> {
    Some(SveIntxPermExtractI::ExtZZiCon)
}

/// Arm group `sve_perm_unpred_a`: SVE permute vector segment (DUP
/// indexed).
fn sve_perm_unpred_a(word: u32) -> Option<Encoding> {
    sve_int_perm_dup_i(word).map(Encoding::SveIntPermDupI)
}

/// Arm group `sve_int_perm_dup_i`: SVE broadcast indexed element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermDupI {
    DupZZi,
}

fn sve_int_perm_dup_i(_word: u32) -> Option<SveIntPermDupI> {
    Some(SveIntPermDupI::DupZZi)
}

/// Arm group `sve_perm_unpred_b`: SVE2 table lookup (three sources).
fn sve_perm_unpred_b(word: u32) -> Option<Encoding> {
    sve_int_perm_tbl_3src(word).map(Encoding::SveIntPermTbl3src)
}

/// Arm group `sve_int_perm_tbl_3src`: SVE2 table lookup (three sources).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermTbl3src {
    TblZZz2,
    TbxZZz,
}

fn sve_int_perm_tbl_3src(word: u32) -> Option<SveIntPermTbl3src> {
    let op = bits(word, 10, 10);
    match op {
        0b0 => Some(SveIntPermTbl3src::TblZZz2),
        0b1 => Some(SveIntPermTbl3src::TbxZZz),
        _ => None,
    }
}

/// Arm group `sve_perm_unpred_c`: SVE table lookup.
fn sve_perm_unpred_c(word: u32) -> Option<Encoding> {
    sve_int_perm_tbl(word).map(Encoding::SveIntPermTbl)
}

/// Arm group `sve_int_perm_tbl`: SVE table lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermTbl {
    TblZZz1,
}

fn sve_int_perm_tbl(_word: u32) -> Option<SveIntPermTbl> {
    Some(SveIntPermTbl::TblZZz1)
}

/// Arm group `sve_perm_unpred_d`: SVE permute vector (unpredicated).
/// `op0 = 01` is FEAT_SVE2p1's and FEAT_SME2p1's predicate moves.
fn sve_perm_unpred_d(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 20, 19);
    let op1 = bits(word, 18, 16);
    match (op0, op1) {
        (0b00, 0b000) => sve_int_perm_dup_r(word).map(Encoding::SveIntPermDupR),
        (0b00, 0b100) => sve_int_perm_insrs(word).map(Encoding::SveIntPermInsrs),
        (0b10, 0b000..=0b011) => sve_int_perm_unpk(word).map(Encoding::SveIntPermUnpk),
        (0b10, 0b100) => sve_int_perm_insrv(word).map(Encoding::SveIntPermInsrv),
        (0b11, 0b000) => sve_int_perm_reverse_z(word).map(Encoding::SveIntPermReverseZ),
        _ => None,
    }
}

/// Arm group `sve_int_perm_dup_r`: SVE broadcast general register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermDupR {
    DupZRBhs,
    DupZRD,
}

fn sve_int_perm_dup_r(word: u32) -> Option<SveIntPermDupR> {
    let size = bits(word, 23, 22);
    match size {
        0b11 => Some(SveIntPermDupR::DupZRD),
        _ => Some(SveIntPermDupR::DupZRBhs),
    }
}

/// Arm group `sve_int_perm_insrs`: SVE insert general register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermInsrs {
    InsrZR,
}

fn sve_int_perm_insrs(_word: u32) -> Option<SveIntPermInsrs> {
    Some(SveIntPermInsrs::InsrZR)
}

/// Arm group `sve_int_perm_unpk`: SVE unpack vector elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermUnpk {
    SunpkloZZ,
    SunpkhiZZ,
    UunpkloZZ,
    UunpkhiZZ,
}

fn sve_int_perm_unpk(word: u32) -> Option<SveIntPermUnpk> {
    let u = bits(word, 17, 17);
    let h = bits(word, 16, 16);
    match (u, h) {
        (0b0, 0b0) => Some(SveIntPermUnpk::SunpkloZZ),
        (0b0, 0b1) => Some(SveIntPermUnpk::SunpkhiZZ),
        (0b1, 0b0) => Some(SveIntPermUnpk::UunpkloZZ),
        (0b1, 0b1) => Some(SveIntPermUnpk::UunpkhiZZ),
        _ => None,
    }
}

/// Arm group `sve_int_perm_insrv`: SVE insert SIMD&FP scalar register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermInsrv {
    InsrZV,
}

fn sve_int_perm_insrv(_word: u32) -> Option<SveIntPermInsrv> {
    Some(SveIntPermInsrv::InsrZV)
}

/// Arm group `sve_int_perm_reverse_z`: SVE reverse vector elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermReverseZ {
    RevZZ,
}

fn sve_int_perm_reverse_z(_word: u32) -> Option<SveIntPermReverseZ> {
    Some(SveIntPermReverseZ::RevZZ)
}

/// Arm group `sve_perm_predicates`: SVE permute predicate.
fn sve_perm_predicates(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 23, 22);
    let op1 = bits(word, 20, 16);
    let op2 = bits(word, 12, 9);
    let op3 = bits(word, 4, 4);
    match (op1, op3) {
        (0b00000..=0b01111, 0b0) => match bits(word, 9, 9) {
            0b0 => sve_int_perm_bin_perm_pp(word).map(Encoding::SveIntPermBinPermPp),
            _ => None,
        },
        (0b10000 | 0b10001, 0b0) => match (op0, op2) {
            (0b00, 0b0000) => sve_int_perm_punpk(word).map(Encoding::SveIntPermPunpk),
            _ => None,
        },
        (0b10100, 0b0) => match op2 {
            0b0000 => sve_int_perm_reverse_p(word).map(Encoding::SveIntPermReverseP),
            _ => None,
        },
        _ => None,
    }
}

/// Arm group `sve_int_perm_bin_perm_pp`: SVE permute predicate elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermBinPermPp {
    Zip1PPp,
    Uzp1PPp,
    Trn1PPp,
    Zip2PPp,
    Uzp2PPp,
    Trn2PPp,
}

fn sve_int_perm_bin_perm_pp(word: u32) -> Option<SveIntPermBinPermPp> {
    let opc = bits(word, 12, 11);
    let h = bits(word, 10, 10);
    match (opc, h) {
        (0b00, 0b0) => Some(SveIntPermBinPermPp::Zip1PPp),
        (0b01, 0b0) => Some(SveIntPermBinPermPp::Uzp1PPp),
        (0b10, 0b0) => Some(SveIntPermBinPermPp::Trn1PPp),
        (0b00, 0b1) => Some(SveIntPermBinPermPp::Zip2PPp),
        (0b01, 0b1) => Some(SveIntPermBinPermPp::Uzp2PPp),
        (0b10, 0b1) => Some(SveIntPermBinPermPp::Trn2PPp),
        _ => None,
    }
}

/// Arm group `sve_int_perm_punpk`: SVE unpack predicate elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermPunpk {
    PunpkloPP,
    PunpkhiPP,
}

fn sve_int_perm_punpk(word: u32) -> Option<SveIntPermPunpk> {
    let h = bits(word, 16, 16);
    match h {
        0b0 => Some(SveIntPermPunpk::PunpkloPP),
        0b1 => Some(SveIntPermPunpk::PunpkhiPP),
        _ => None,
    }
}

/// Arm group `sve_int_perm_reverse_p`: SVE reverse predicate elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermReverseP {
    RevPP,
}

fn sve_int_perm_reverse_p(_word: u32) -> Option<SveIntPermReverseP> {
    Some(SveIntPermReverseP::RevPP)
}

/// Arm group `sve_perm_inter`: SVE permute vector elements.
fn sve_perm_inter(word: u32) -> Option<Encoding> {
    sve_int_perm_bin_perm_zz(word).map(Encoding::SveIntPermBinPermZz)
}

/// Arm group `sve_int_perm_bin_perm_zz`: SVE permute vector elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermBinPermZz {
    Zip1ZZz,
    Uzp1ZZz,
    Trn1ZZz,
    Zip2ZZz,
    Uzp2ZZz,
    Trn2ZZz,
}

fn sve_int_perm_bin_perm_zz(word: u32) -> Option<SveIntPermBinPermZz> {
    let opc = bits(word, 12, 10);
    match opc {
        0b000 => Some(SveIntPermBinPermZz::Zip1ZZz),
        0b010 => Some(SveIntPermBinPermZz::Uzp1ZZz),
        0b100 => Some(SveIntPermBinPermZz::Trn1ZZz),
        0b001 => Some(SveIntPermBinPermZz::Zip2ZZz),
        0b011 => Some(SveIntPermBinPermZz::Uzp2ZZz),
        0b101 => Some(SveIntPermBinPermZz::Trn2ZZz),
        _ => None,
    }
}

/// Arm group `sve_perm_pred`: SVE permute vector (predicated). COMPACT and
/// EXPAND are FEAT_SVE's and FEAT_SVE2p2's alone.
fn sve_perm_pred(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 20, 20);
    let op1 = bits(word, 19, 17);
    let op2 = bits(word, 16, 16);
    let op3 = bits(word, 13, 13);
    match (op0, op1, op2, op3) {
        (0b0, 0b000, 0b0, 0b0) => sve_int_perm_cpy_v(word).map(Encoding::SveIntPermCpyV),
        (0b0, 0b001, _, 0b0) => sve_int_perm_last_v(word).map(Encoding::SveIntPermLastV),
        (0b0, 0b010 | 0b011, _, _) => sve_int_perm_rev(word).map(Encoding::SveIntPermRev),
        (0b0, 0b100, _, 0b0) => sve_int_perm_clast_zz(word).map(Encoding::SveIntPermClastZz),
        (0b0, 0b101, _, 0b0) => sve_int_perm_clast_vz(word).map(Encoding::SveIntPermClastVz),
        (0b0, 0b110, 0b0, 0b0) => sve_int_perm_splice(word).map(Encoding::SveIntPermSplice),
        (0b0, 0b110, 0b1, 0b0) => sve_intx_perm_splice(word).map(Encoding::SveIntxPermSplice),
        (0b0, 0b000, _, 0b1) => sve_int_perm_last_r(word).map(Encoding::SveIntPermLastR),
        (0b0, 0b100, 0b0, 0b1) => sve_int_perm_cpy_r(word).map(Encoding::SveIntPermCpyR),
        (0b1, 0b000, _, 0b1) => sve_int_perm_clast_rz(word).map(Encoding::SveIntPermClastRz),
        (0b0, 0b111, 0b0, _) => sve_int_perm_revd(word).map(Encoding::SveIntPermRevd),
        _ => None,
    }
}

/// Arm group `sve_int_perm_cpy_v`: SVE copy SIMD&FP scalar register to
/// vector (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermCpyV {
    CpyZPV,
}

fn sve_int_perm_cpy_v(_word: u32) -> Option<SveIntPermCpyV> {
    Some(SveIntPermCpyV::CpyZPV)
}

/// Arm group `sve_int_perm_last_v`: SVE extract element to SIMD&FP scalar
/// register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermLastV {
    LastaVPZ,
    LastbVPZ,
}

fn sve_int_perm_last_v(word: u32) -> Option<SveIntPermLastV> {
    let b = bits(word, 16, 16);
    match b {
        0b0 => Some(SveIntPermLastV::LastaVPZ),
        0b1 => Some(SveIntPermLastV::LastbVPZ),
        _ => None,
    }
}

/// Arm group `sve_int_perm_rev`: SVE reverse within elements. `Z = 1` is
/// the zeroing forms of FEAT_SVE2p2 and FEAT_SME2p2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermRev {
    RevbZZM,
    RevhZZM,
    RevwZZM,
    RbitZPZM,
}

fn sve_int_perm_rev(word: u32) -> Option<SveIntPermRev> {
    let opc = bits(word, 17, 16);
    let z = bits(word, 13, 13);
    match (opc, z) {
        (0b00, 0b0) => Some(SveIntPermRev::RevbZZM),
        (0b01, 0b0) => Some(SveIntPermRev::RevhZZM),
        (0b10, 0b0) => Some(SveIntPermRev::RevwZZM),
        (0b11, 0b0) => Some(SveIntPermRev::RbitZPZM),
        _ => None,
    }
}

/// Arm group `sve_int_perm_clast_zz`: SVE conditionally broadcast element
/// to vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermClastZz {
    ClastaZPZz,
    ClastbZPZz,
}

fn sve_int_perm_clast_zz(word: u32) -> Option<SveIntPermClastZz> {
    let b = bits(word, 16, 16);
    match b {
        0b0 => Some(SveIntPermClastZz::ClastaZPZz),
        0b1 => Some(SveIntPermClastZz::ClastbZPZz),
        _ => None,
    }
}

/// Arm group `sve_int_perm_clast_vz`: SVE conditionally extract element to
/// SIMD&FP scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermClastVz {
    ClastaVPZ,
    ClastbVPZ,
}

fn sve_int_perm_clast_vz(word: u32) -> Option<SveIntPermClastVz> {
    let b = bits(word, 16, 16);
    match b {
        0b0 => Some(SveIntPermClastVz::ClastaVPZ),
        0b1 => Some(SveIntPermClastVz::ClastbVPZ),
        _ => None,
    }
}

/// Arm group `sve_int_perm_splice`: SVE vector splice (destructive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermSplice {
    SpliceZPZzDes,
}

fn sve_int_perm_splice(_word: u32) -> Option<SveIntPermSplice> {
    Some(SveIntPermSplice::SpliceZPZzDes)
}

/// Arm group `sve_intx_perm_splice`: SVE2 vector splice (constructive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntxPermSplice {
    SpliceZPZzCon,
}

fn sve_intx_perm_splice(_word: u32) -> Option<SveIntxPermSplice> {
    Some(SveIntxPermSplice::SpliceZPZzCon)
}

/// Arm group `sve_int_perm_last_r`: SVE extract element to general
/// register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermLastR {
    LastaRPZ,
    LastbRPZ,
}

fn sve_int_perm_last_r(word: u32) -> Option<SveIntPermLastR> {
    let b = bits(word, 16, 16);
    match b {
        0b0 => Some(SveIntPermLastR::LastaRPZ),
        0b1 => Some(SveIntPermLastR::LastbRPZ),
        _ => None,
    }
}

/// Arm group `sve_int_perm_cpy_r`: SVE copy general register to vector
/// (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermCpyR {
    CpyZPRBhs,
    CpyZPRD,
}

fn sve_int_perm_cpy_r(word: u32) -> Option<SveIntPermCpyR> {
    let size = bits(word, 23, 22);
    match size {
        0b11 => Some(SveIntPermCpyR::CpyZPRD),
        _ => Some(SveIntPermCpyR::CpyZPRBhs),
    }
}

/// Arm group `sve_int_perm_clast_rz`: SVE conditionally extract element to
/// general register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermClastRz {
    ClastaRPZ,
    ClastbRPZ,
}

fn sve_int_perm_clast_rz(word: u32) -> Option<SveIntPermClastRz> {
    let b = bits(word, 16, 16);
    match b {
        0b0 => Some(SveIntPermClastRz::ClastaRPZ),
        0b1 => Some(SveIntPermClastRz::ClastbRPZ),
        _ => None,
    }
}

/// Arm group `sve_int_perm_revd`: SVE reverse doublewords. `Z = 1` is the
/// zeroing form of FEAT_SVE2p2 and FEAT_SME2p2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPermRevd {
    RevdZPZM,
}

fn sve_int_perm_revd(word: u32) -> Option<SveIntPermRevd> {
    let size = bits(word, 23, 22);
    let z = bits(word, 13, 13);
    match (size, z) {
        (0b00, 0b0) => Some(SveIntPermRevd::RevdZPZM),
        _ => None,
    }
}

/// Arm group `sve_int_select`: SVE select vector elements (predicated).
fn sve_int_select(word: u32) -> Option<Encoding> {
    sve_int_sel_vvv(word).map(Encoding::SveIntSelVvv)
}

/// Arm group `sve_int_sel_vvv`: SVE select vector elements (predicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntSelVvv {
    SelZPZz,
}

fn sve_int_sel_vvv(_word: u32) -> Option<SveIntSelVvv> {
    Some(SveIntSelVvv::SelZPZz)
}

// ----- op0 = 001: compares and predicates -----

/// Arm group `sve_cmpvec`: SVE integer compare vectors.
fn sve_cmpvec(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 14, 14);
    match op0 {
        0b0 => sve_int_cmp_0(word).map(Encoding::SveIntCmp0),
        0b1 => sve_int_cmp_1(word).map(Encoding::SveIntCmp1),
        _ => None,
    }
}

/// Arm group `sve_int_cmp_0`: SVE integer compare vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntCmp0 {
    CmphsPPZz,
    CmpgePPZz,
    CmpeqPPZw,
    CmpeqPPZz,
    CmphiPPZz,
    CmpgtPPZz,
    CmpnePPZw,
    CmpnePPZz,
}

fn sve_int_cmp_0(word: u32) -> Option<SveIntCmp0> {
    let op = bits(word, 15, 15);
    let o2 = bits(word, 13, 13);
    let ne = bits(word, 4, 4);
    match (op, o2, ne) {
        (0b0, 0b0, 0b0) => Some(SveIntCmp0::CmphsPPZz),
        (0b1, 0b0, 0b0) => Some(SveIntCmp0::CmpgePPZz),
        (0b0, 0b1, 0b0) => Some(SveIntCmp0::CmpeqPPZw),
        (0b1, 0b1, 0b0) => Some(SveIntCmp0::CmpeqPPZz),
        (0b0, 0b0, 0b1) => Some(SveIntCmp0::CmphiPPZz),
        (0b1, 0b0, 0b1) => Some(SveIntCmp0::CmpgtPPZz),
        (0b0, 0b1, 0b1) => Some(SveIntCmp0::CmpnePPZw),
        (0b1, 0b1, 0b1) => Some(SveIntCmp0::CmpnePPZz),
        _ => None,
    }
}

/// Arm group `sve_int_cmp_1`: SVE integer compare with wide elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntCmp1 {
    CmpgePPZw,
    CmphsPPZw,
    CmpgtPPZw,
    CmphiPPZw,
    CmpltPPZw,
    CmploPPZw,
    CmplePPZw,
    CmplsPPZw,
}

fn sve_int_cmp_1(word: u32) -> Option<SveIntCmp1> {
    let u = bits(word, 15, 15);
    let lt = bits(word, 13, 13);
    let ne = bits(word, 4, 4);
    match (u, lt, ne) {
        (0b0, 0b0, 0b0) => Some(SveIntCmp1::CmpgePPZw),
        (0b1, 0b0, 0b0) => Some(SveIntCmp1::CmphsPPZw),
        (0b0, 0b0, 0b1) => Some(SveIntCmp1::CmpgtPPZw),
        (0b1, 0b0, 0b1) => Some(SveIntCmp1::CmphiPPZw),
        (0b0, 0b1, 0b0) => Some(SveIntCmp1::CmpltPPZw),
        (0b1, 0b1, 0b0) => Some(SveIntCmp1::CmploPPZw),
        (0b0, 0b1, 0b1) => Some(SveIntCmp1::CmplePPZw),
        (0b1, 0b1, 0b1) => Some(SveIntCmp1::CmplsPPZw),
        _ => None,
    }
}

/// Arm group `sve_cmpuimm`: SVE integer compare with unsigned immediate.
fn sve_cmpuimm(word: u32) -> Option<Encoding> {
    sve_int_ucmp_vi(word).map(Encoding::SveIntUcmpVi)
}

/// Arm group `sve_int_ucmp_vi`: SVE integer compare with unsigned
/// immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntUcmpVi {
    CmphsPPZi,
    CmphiPPZi,
    CmploPPZi,
    CmplsPPZi,
}

fn sve_int_ucmp_vi(word: u32) -> Option<SveIntUcmpVi> {
    let lt = bits(word, 13, 13);
    let ne = bits(word, 4, 4);
    match (lt, ne) {
        (0b0, 0b0) => Some(SveIntUcmpVi::CmphsPPZi),
        (0b0, 0b1) => Some(SveIntUcmpVi::CmphiPPZi),
        (0b1, 0b0) => Some(SveIntUcmpVi::CmploPPZi),
        (0b1, 0b1) => Some(SveIntUcmpVi::CmplsPPZi),
        _ => None,
    }
}

/// Arm group `sve_cmpsimm`: SVE integer compare with signed immediate.
fn sve_cmpsimm(word: u32) -> Option<Encoding> {
    sve_int_scmp_vi(word).map(Encoding::SveIntScmpVi)
}

/// Arm group `sve_int_scmp_vi`: SVE integer compare with signed immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntScmpVi {
    CmpgePPZi,
    CmpeqPPZi,
    CmpltPPZi,
    CmpgtPPZi,
    CmpnePPZi,
    CmplePPZi,
}

fn sve_int_scmp_vi(word: u32) -> Option<SveIntScmpVi> {
    let op = bits(word, 15, 15);
    let o2 = bits(word, 13, 13);
    let ne = bits(word, 4, 4);
    match (op, o2, ne) {
        (0b0, 0b0, 0b0) => Some(SveIntScmpVi::CmpgePPZi),
        (0b1, 0b0, 0b0) => Some(SveIntScmpVi::CmpeqPPZi),
        (0b0, 0b1, 0b0) => Some(SveIntScmpVi::CmpltPPZi),
        (0b0, 0b0, 0b1) => Some(SveIntScmpVi::CmpgtPPZi),
        (0b1, 0b0, 0b1) => Some(SveIntScmpVi::CmpnePPZi),
        (0b0, 0b1, 0b1) => Some(SveIntScmpVi::CmplePPZi),
        _ => None,
    }
}

/// Arm group `sve_pred_gen_a`: SVE predicate logical operations.
fn sve_pred_gen_a(word: u32) -> Option<Encoding> {
    sve_int_pred_log(word).map(Encoding::SveIntPredLog)
}

/// Arm group `sve_int_pred_log`: SVE predicate logical operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPredLog {
    AndPPPpZ,
    BicPPPpZ,
    OrrPPPpZ,
    OrnPPPpZ,
    EorPPPpZ,
    SelPPPp,
    NorPPPpZ,
    NandPPPpZ,
    AndsPPPpZ,
    BicsPPPpZ,
    OrrsPPPpZ,
    OrnsPPPpZ,
    EorsPPPpZ,
    NorsPPPpZ,
    NandsPPPpZ,
}

fn sve_int_pred_log(word: u32) -> Option<SveIntPredLog> {
    let op = bits(word, 23, 23);
    let s = bits(word, 22, 22);
    let o2 = bits(word, 9, 9);
    let o3 = bits(word, 4, 4);
    match (op, s, o2, o3) {
        (0b0, 0b0, 0b0, 0b0) => Some(SveIntPredLog::AndPPPpZ),
        (0b0, 0b0, 0b0, 0b1) => Some(SveIntPredLog::BicPPPpZ),
        (0b1, 0b0, 0b0, 0b0) => Some(SveIntPredLog::OrrPPPpZ),
        (0b1, 0b0, 0b0, 0b1) => Some(SveIntPredLog::OrnPPPpZ),
        (0b0, 0b0, 0b1, 0b0) => Some(SveIntPredLog::EorPPPpZ),
        (0b0, 0b0, 0b1, 0b1) => Some(SveIntPredLog::SelPPPp),
        (0b1, 0b0, 0b1, 0b0) => Some(SveIntPredLog::NorPPPpZ),
        (0b1, 0b0, 0b1, 0b1) => Some(SveIntPredLog::NandPPPpZ),
        (0b0, 0b1, 0b0, 0b0) => Some(SveIntPredLog::AndsPPPpZ),
        (0b0, 0b1, 0b0, 0b1) => Some(SveIntPredLog::BicsPPPpZ),
        (0b1, 0b1, 0b0, 0b0) => Some(SveIntPredLog::OrrsPPPpZ),
        (0b1, 0b1, 0b0, 0b1) => Some(SveIntPredLog::OrnsPPPpZ),
        (0b0, 0b1, 0b1, 0b0) => Some(SveIntPredLog::EorsPPPpZ),
        (0b1, 0b1, 0b1, 0b0) => Some(SveIntPredLog::NorsPPPpZ),
        (0b1, 0b1, 0b1, 0b1) => Some(SveIntPredLog::NandsPPPpZ),
        _ => None,
    }
}

/// Arm group `sve_pred_gen_b`: SVE propagate break from previous partition.
fn sve_pred_gen_b(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 9, 9);
    match op0 {
        0b0 => sve_int_brkp(word).map(Encoding::SveIntBrkp),
        _ => None,
    }
}

/// Arm group `sve_int_brkp`: SVE propagate break from previous partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBrkp {
    BrkpaPPPp,
    BrkpasPPPp,
    BrkpbPPPp,
    BrkpbsPPPp,
}

fn sve_int_brkp(word: u32) -> Option<SveIntBrkp> {
    let op = bits(word, 23, 23);
    let s = bits(word, 22, 22);
    let b = bits(word, 4, 4);
    match (op, s, b) {
        (0b0, 0b0, 0b0) => Some(SveIntBrkp::BrkpaPPPp),
        (0b0, 0b1, 0b0) => Some(SveIntBrkp::BrkpasPPPp),
        (0b0, 0b0, 0b1) => Some(SveIntBrkp::BrkpbPPPp),
        (0b0, 0b1, 0b1) => Some(SveIntBrkp::BrkpbsPPPp),
        _ => None,
    }
}

/// Arm group `sve_pred_gen_c`: SVE partition break condition.
fn sve_pred_gen_c(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 23, 23);
    let op1 = bits(word, 19, 16);
    let op2 = bits(word, 9, 9);
    let op3 = bits(word, 4, 4);
    match (op1, op2) {
        (0b0000, 0b0) => sve_int_break(word).map(Encoding::SveIntBreak),
        (0b1000, 0b0) => match (op0, op3) {
            (0b0, 0b0) => sve_int_brkn(word).map(Encoding::SveIntBrkn),
            _ => None,
        },
        _ => None,
    }
}

/// Arm group `sve_int_break`: SVE partition break condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBreak {
    BrkaPPP,
    BrkasPPPZ,
    BrkbPPP,
    BrkbsPPPZ,
}

fn sve_int_break(word: u32) -> Option<SveIntBreak> {
    let b = bits(word, 23, 23);
    let s = bits(word, 22, 22);
    let m = bits(word, 4, 4);
    match (b, s) {
        (0b0, 0b0) => Some(SveIntBreak::BrkaPPP),
        (0b0, 0b1) => match m {
            0b0 => Some(SveIntBreak::BrkasPPPZ),
            _ => None,
        },
        (0b1, 0b0) => Some(SveIntBreak::BrkbPPP),
        (0b1, 0b1) => match m {
            0b0 => Some(SveIntBreak::BrkbsPPPZ),
            _ => None,
        },
        _ => None,
    }
}

/// Arm group `sve_int_brkn`: SVE propagate break to next partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntBrkn {
    BrknPPPp,
    BrknsPPPp,
}

fn sve_int_brkn(word: u32) -> Option<SveIntBrkn> {
    let s = bits(word, 22, 22);
    match s {
        0b0 => Some(SveIntBrkn::BrknPPPp),
        0b1 => Some(SveIntBrkn::BrknsPPPp),
        _ => None,
    }
}

/// Arm group `sve_pred_gen_d`: SVE predicate misc. RDFFR is FEAT_SVE's
/// alone.
fn sve_pred_gen_d(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 19, 16);
    let op1 = bits(word, 13, 11);
    let op2 = bits(word, 10, 9);
    let op3 = bits(word, 8, 5);
    let op4 = bits(word, 4, 4);
    match (op0, op1, op2, op4) {
        (0b0000, _, 0b00 | 0b10, 0b0) => sve_int_ptest(word).map(Encoding::SveIntPtest),
        (0b1000, 0b000, 0b00, 0b0) => sve_int_pfirst(word).map(Encoding::SveIntPfirst),
        (0b1001, 0b000, 0b10, 0b0) => sve_int_pnext(word).map(Encoding::SveIntPnext),
        (0b1000 | 0b1001, 0b100, 0b00 | 0b01, 0b0) => sve_int_ptrue(word).map(Encoding::SveIntPtrue),
        (0b1000, 0b100, 0b10, 0b0) => match op3 {
            0b0000 => sve_int_pfalse(word).map(Encoding::SveIntPfalse),
            _ => None,
        },
        _ => None,
    }
}

/// Arm group `sve_int_ptest`: SVE predicate test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPtest {
    PtestPP,
}

fn sve_int_ptest(word: u32) -> Option<SveIntPtest> {
    let op = bits(word, 23, 23);
    let s = bits(word, 22, 22);
    let opc2 = bits(word, 3, 0);
    match (op, s, opc2) {
        (0b0, 0b1, 0b0000) => Some(SveIntPtest::PtestPP),
        _ => None,
    }
}

/// Arm group `sve_int_pfirst`: SVE predicate first active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPfirst {
    PfirstPPP,
}

fn sve_int_pfirst(word: u32) -> Option<SveIntPfirst> {
    let op = bits(word, 23, 23);
    let s = bits(word, 22, 22);
    match (op, s) {
        (0b0, 0b1) => Some(SveIntPfirst::PfirstPPP),
        _ => None,
    }
}

/// Arm group `sve_int_pnext`: SVE predicate next active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPnext {
    PnextPPP,
}

fn sve_int_pnext(_word: u32) -> Option<SveIntPnext> {
    Some(SveIntPnext::PnextPPP)
}

/// Arm group `sve_int_ptrue`: SVE predicate initialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPtrue {
    PtruePS,
    PtruesPS,
}

fn sve_int_ptrue(word: u32) -> Option<SveIntPtrue> {
    let s = bits(word, 16, 16);
    match s {
        0b0 => Some(SveIntPtrue::PtruePS),
        0b1 => Some(SveIntPtrue::PtruesPS),
        _ => None,
    }
}

/// Arm group `sve_int_pfalse`: SVE predicate zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPfalse {
    PfalseP,
}

fn sve_int_pfalse(word: u32) -> Option<SveIntPfalse> {
    let op = bits(word, 23, 23);
    let s = bits(word, 22, 22);
    match (op, s) {
        (0b0, 0b0) => Some(SveIntPfalse::PfalseP),
        _ => None,
    }
}

/// Arm group `sve_pred_dup`: SVE predicate select (PSEL).
fn sve_pred_dup(word: u32) -> Option<Encoding> {
    sve_int_pred_dup(word).map(Encoding::SveIntPredDup)
}

/// Arm group `sve_int_pred_dup`: SVE predicate select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPredDup {
    PselPPpi,
}

fn sve_int_pred_dup(word: u32) -> Option<SveIntPredDup> {
    let s = bits(word, 9, 9);
    match s {
        0b0 => Some(SveIntPredDup::PselPPpi),
        _ => None,
    }
}

/// Arm group `sve_cmpgpr`: SVE integer compare scalar count and limit.
fn sve_cmpgpr(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 13, 12);
    let op1 = bits(word, 11, 10);
    let op2 = bits(word, 3, 0);
    match (op0, op1) {
        (0b00 | 0b01, _) => sve_int_while_rr(word).map(Encoding::SveIntWhileRr),
        (0b10, 0b00) => match op2 {
            0b0000 => sve_int_cterm(word).map(Encoding::SveIntCterm),
            _ => None,
        },
        (0b11, 0b00) => sve_int_whilenc(word).map(Encoding::SveIntWhilenc),
        _ => None,
    }
}

/// Arm group `sve_int_while_rr`: SVE integer compare scalar count and
/// limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntWhileRr {
    WhilegePPRr,
    WhilehsPPRr,
    WhilegtPPRr,
    WhilehiPPRr,
    WhileltPPRr,
    WhileloPPRr,
    WhilelePPRr,
    WhilelsPPRr,
}

fn sve_int_while_rr(word: u32) -> Option<SveIntWhileRr> {
    let u = bits(word, 11, 11);
    let lt = bits(word, 10, 10);
    let eq = bits(word, 4, 4);
    match (u, lt, eq) {
        (0b0, 0b0, 0b0) => Some(SveIntWhileRr::WhilegePPRr),
        (0b1, 0b0, 0b0) => Some(SveIntWhileRr::WhilehsPPRr),
        (0b0, 0b0, 0b1) => Some(SveIntWhileRr::WhilegtPPRr),
        (0b1, 0b0, 0b1) => Some(SveIntWhileRr::WhilehiPPRr),
        (0b0, 0b1, 0b0) => Some(SveIntWhileRr::WhileltPPRr),
        (0b1, 0b1, 0b0) => Some(SveIntWhileRr::WhileloPPRr),
        (0b0, 0b1, 0b1) => Some(SveIntWhileRr::WhilelePPRr),
        (0b1, 0b1, 0b1) => Some(SveIntWhileRr::WhilelsPPRr),
        _ => None,
    }
}

/// Arm group `sve_int_cterm`: SVE conditionally terminate scalars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntCterm {
    CtermeqRr,
    CtermneRr,
}

fn sve_int_cterm(word: u32) -> Option<SveIntCterm> {
    let op = bits(word, 23, 23);
    let ne = bits(word, 4, 4);
    match (op, ne) {
        (0b1, 0b0) => Some(SveIntCterm::CtermeqRr),
        (0b1, 0b1) => Some(SveIntCterm::CtermneRr),
        _ => None,
    }
}

/// Arm group `sve_int_whilenc`: SVE2 pointer conflict compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntWhilenc {
    WhilewrPRr,
    WhilerwPRr,
}

fn sve_int_whilenc(word: u32) -> Option<SveIntWhilenc> {
    let rw = bits(word, 4, 4);
    match rw {
        0b0 => Some(SveIntWhilenc::WhilewrPRr),
        0b1 => Some(SveIntWhilenc::WhilerwPRr),
        _ => None,
    }
}

/// Arm group `sve_pred_count_a`: SVE predicate count. `op1 = 1` is
/// FEAT_SVE2p1's and FEAT_SME2's predicate-as-counter form.
fn sve_pred_count_a(word: u32) -> Option<Encoding> {
    let op1 = bits(word, 9, 9);
    match op1 {
        0b0 => sve_int_pcount_pred(word).map(Encoding::SveIntPcountPred),
        _ => None,
    }
}

/// Arm group `sve_int_pcount_pred`: SVE predicate count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntPcountPred {
    CntpRPP,
}

fn sve_int_pcount_pred(word: u32) -> Option<SveIntPcountPred> {
    let opc = bits(word, 18, 16);
    match opc {
        0b000 => Some(SveIntPcountPred::CntpRPP),
        _ => None,
    }
}

/// Arm group `sve_pred_count_b`: SVE inc/dec by predicate count.
fn sve_pred_count_b(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 18, 18);
    let op1 = bits(word, 11, 11);
    match (op0, op1) {
        (0b0, 0b0) => sve_int_count_v_sat(word).map(Encoding::SveIntCountVSat),
        (0b1, 0b0) => sve_int_count_v(word).map(Encoding::SveIntCountV),
        (0b0, 0b1) => sve_int_count_r_sat(word).map(Encoding::SveIntCountRSat),
        (0b1, 0b1) => sve_int_count_r(word).map(Encoding::SveIntCountR),
        _ => None,
    }
}

/// Arm group `sve_int_count_v_sat`: SVE saturating inc/dec vector by
/// predicate count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntCountVSat {
    SqincpZPZ,
    SqdecpZPZ,
    UqincpZPZ,
    UqdecpZPZ,
}

fn sve_int_count_v_sat(word: u32) -> Option<SveIntCountVSat> {
    let d = bits(word, 17, 17);
    let u = bits(word, 16, 16);
    let opc = bits(word, 10, 9);
    match (d, u, opc) {
        (0b0, 0b0, 0b00) => Some(SveIntCountVSat::SqincpZPZ),
        (0b1, 0b0, 0b00) => Some(SveIntCountVSat::SqdecpZPZ),
        (0b0, 0b1, 0b00) => Some(SveIntCountVSat::UqincpZPZ),
        (0b1, 0b1, 0b00) => Some(SveIntCountVSat::UqdecpZPZ),
        _ => None,
    }
}

/// Arm group `sve_int_count_v`: SVE inc/dec vector by predicate count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntCountV {
    IncpZPZ,
    DecpZPZ,
}

fn sve_int_count_v(word: u32) -> Option<SveIntCountV> {
    let op = bits(word, 17, 17);
    let d = bits(word, 16, 16);
    let opc2 = bits(word, 10, 9);
    match (op, d, opc2) {
        (0b0, 0b0, 0b00) => Some(SveIntCountV::IncpZPZ),
        (0b0, 0b1, 0b00) => Some(SveIntCountV::DecpZPZ),
        _ => None,
    }
}

/// Arm group `sve_int_count_r_sat`: SVE saturating inc/dec register by
/// predicate count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntCountRSat {
    SqincpRPRSx,
    UqincpRPRUw,
    SqdecpRPRSx,
    UqdecpRPRUw,
    SqincpRPRX,
    SqdecpRPRX,
    UqincpRPRX,
    UqdecpRPRX,
}

fn sve_int_count_r_sat(word: u32) -> Option<SveIntCountRSat> {
    let d = bits(word, 17, 17);
    let u = bits(word, 16, 16);
    let sf = bits(word, 10, 10);
    let op = bits(word, 9, 9);
    match (d, u, sf, op) {
        (0b0, 0b0, 0b0, 0b0) => Some(SveIntCountRSat::SqincpRPRSx),
        (0b0, 0b1, 0b0, 0b0) => Some(SveIntCountRSat::UqincpRPRUw),
        (0b1, 0b0, 0b0, 0b0) => Some(SveIntCountRSat::SqdecpRPRSx),
        (0b1, 0b1, 0b0, 0b0) => Some(SveIntCountRSat::UqdecpRPRUw),
        (0b0, 0b0, 0b1, 0b0) => Some(SveIntCountRSat::SqincpRPRX),
        (0b1, 0b0, 0b1, 0b0) => Some(SveIntCountRSat::SqdecpRPRX),
        (0b0, 0b1, 0b1, 0b0) => Some(SveIntCountRSat::UqincpRPRX),
        (0b1, 0b1, 0b1, 0b0) => Some(SveIntCountRSat::UqdecpRPRX),
        _ => None,
    }
}

/// Arm group `sve_int_count_r`: SVE inc/dec register by predicate count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntCountR {
    IncpRPR,
    DecpRPR,
}

fn sve_int_count_r(word: u32) -> Option<SveIntCountR> {
    let op = bits(word, 17, 17);
    let d = bits(word, 16, 16);
    let opc2 = bits(word, 10, 9);
    match (op, d, opc2) {
        (0b0, 0b0, 0b00) => Some(SveIntCountR::IncpRPR),
        (0b0, 0b1, 0b00) => Some(SveIntCountR::DecpRPR),
        _ => None,
    }
}

/// Arm group `sve_wideimm_unpred`: SVE integer wide immediate
/// (unpredicated).
fn sve_wideimm_unpred(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 20, 19);
    let op1 = bits(word, 16, 16);
    match (op0, op1) {
        (0b00, _) => sve_int_arith_imm0(word).map(Encoding::SveIntArithImm0),
        (0b01, _) => sve_int_arith_imm1(word).map(Encoding::SveIntArithImm1),
        (0b10, _) => sve_int_arith_imm2(word).map(Encoding::SveIntArithImm2),
        (0b11, 0b0) => sve_int_dup_imm(word).map(Encoding::SveIntDupImm),
        (0b11, 0b1) => sve_int_dup_fpimm(word).map(Encoding::SveIntDupFpimm),
        _ => None,
    }
}

/// Arm group `sve_int_arith_imm0`: SVE integer add/subtract immediate
/// (unpredicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntArithImm0 {
    AddZZi,
    SubZZi,
    SubrZZi,
    SqaddZZi,
    SqsubZZi,
    UqaddZZi,
    UqsubZZi,
}

fn sve_int_arith_imm0(word: u32) -> Option<SveIntArithImm0> {
    let opc = bits(word, 18, 16);
    match opc {
        0b000 => Some(SveIntArithImm0::AddZZi),
        0b001 => Some(SveIntArithImm0::SubZZi),
        0b011 => Some(SveIntArithImm0::SubrZZi),
        0b100 => Some(SveIntArithImm0::SqaddZZi),
        0b110 => Some(SveIntArithImm0::SqsubZZi),
        0b101 => Some(SveIntArithImm0::UqaddZZi),
        0b111 => Some(SveIntArithImm0::UqsubZZi),
        _ => None,
    }
}

/// Arm group `sve_int_arith_imm1`: SVE integer min/max immediate
/// (unpredicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntArithImm1 {
    SmaxZZi,
    SminZZi,
    UmaxZZi,
    UminZZi,
}

fn sve_int_arith_imm1(word: u32) -> Option<SveIntArithImm1> {
    let opc = bits(word, 18, 16);
    let o2 = bits(word, 13, 13);
    match (opc, o2) {
        (0b000, 0b0) => Some(SveIntArithImm1::SmaxZZi),
        (0b010, 0b0) => Some(SveIntArithImm1::SminZZi),
        (0b001, 0b0) => Some(SveIntArithImm1::UmaxZZi),
        (0b011, 0b0) => Some(SveIntArithImm1::UminZZi),
        _ => None,
    }
}

/// Arm group `sve_int_arith_imm2`: SVE integer multiply immediate
/// (unpredicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntArithImm2 {
    MulZZi,
}

fn sve_int_arith_imm2(word: u32) -> Option<SveIntArithImm2> {
    let opc = bits(word, 18, 16);
    let o2 = bits(word, 13, 13);
    match (opc, o2) {
        (0b000, 0b0) => Some(SveIntArithImm2::MulZZi),
        _ => None,
    }
}

/// Arm group `sve_int_dup_imm`: SVE broadcast integer immediate
/// (unpredicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntDupImm {
    DupZI,
}

fn sve_int_dup_imm(word: u32) -> Option<SveIntDupImm> {
    let opc = bits(word, 18, 17);
    match opc {
        0b00 => Some(SveIntDupImm::DupZI),
        _ => None,
    }
}

/// Arm group `sve_int_dup_fpimm`: SVE broadcast floating-point immediate
/// (unpredicated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SveIntDupFpimm {
    FdupZI,
}

fn sve_int_dup_fpimm(word: u32) -> Option<SveIntDupFpimm> {
    let opc = bits(word, 18, 17);
    let o2 = bits(word, 13, 13);
    match (opc, o2) {
        (0b00, 0b0) => Some(SveIntDupFpimm::FdupZI),
        _ => None,
    }
}
