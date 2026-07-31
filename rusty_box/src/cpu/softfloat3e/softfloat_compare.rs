#![allow(dead_code, non_snake_case)]
//! The 32 VEX/EVEX floating-point compare predicates.
//!
//! Ported from Bochs softfloat3e/include/softfloat-compare.h together with
//! the `avx_compare32` / `avx_compare64` dispatch tables in
//! Bochs cpu/avx/avx_pfp.cc. Legacy SSE CMPPS/CMPPD use only predicates
//! 0..7, which are the first eight entries of the same table (Bochs
//! sse_pfp.cc `compare32` / `compare64`).

use super::f32_compare::{f32_compare, f32_compare_quiet};
use super::f64_compare::{f64_compare, f64_compare_quiet};
use super::softfloat::*;
use super::softfloat_types::*;

/// Whether predicates 0..15 use the quiet comparison (a QNaN operand does
/// not raise #I). Predicates 16..31 are the same relations with the
/// opposite signalling behaviour, which is why `f32_compare_predicate`
/// flips this bit for them.
///
/// Order follows Bochs `avx_compare32`:
/// EQ_OQ, LT_OS, LE_OS, UNORD_Q, NEQ_UQ, NLT_US, NLE_US, ORD_Q,
/// EQ_UQ, NGE_US, NGT_US, FALSE_OQ, NEQ_OQ, GE_OS, GT_OS, TRUE_UQ.
const PREDICATE_IS_QUIET: [bool; 16] = [
    true,  // 0  EQ_OQ    eq_ordered_quiet
    false, // 1  LT_OS    lt_ordered_signalling
    false, // 2  LE_OS    le_ordered_signalling
    true,  // 3  UNORD_Q  unordered_quiet
    true,  // 4  NEQ_UQ   neq_unordered_quiet
    false, // 5  NLT_US   nlt_unordered_signalling
    false, // 6  NLE_US   nle_unordered_signalling
    true,  // 7  ORD_Q    ordered_quiet
    true,  // 8  EQ_UQ    eq_unordered_quiet
    false, // 9  NGE_US   nge_unordered_signalling
    false, // 10 NGT_US   ngt_unordered_signalling
    true,  // 11 FALSE_OQ false_quiet
    true,  // 12 NEQ_OQ   neq_ordered_quiet
    false, // 13 GE_OS    ge_ordered_signalling
    false, // 14 GT_OS    gt_ordered_signalling
    true,  // 15 TRUE_UQ  true_quiet
];

/// Apply the relation test for predicate `base` (0..15) to an ordering
/// relation already produced by the appropriate quiet/signalling compare.
/// The comparison itself is always performed first, so FALSE/TRUE still
/// raise whatever exception their operands warrant — matching Bochs
/// `f32_false_quiet` / `f32_true_quiet`, which discard the relation but
/// not the status update.
#[inline]
fn relation_matches(base: u8, relation: i32) -> bool {
    match base {
        0 => relation == RELATION_EQUAL,
        1 => relation == RELATION_LESS,
        2 => relation == RELATION_LESS || relation == RELATION_EQUAL,
        3 => relation == RELATION_UNORDERED,
        4 => relation != RELATION_EQUAL,
        5 => relation != RELATION_LESS,
        6 => relation != RELATION_LESS && relation != RELATION_EQUAL,
        7 => relation != RELATION_UNORDERED,
        8 => relation == RELATION_EQUAL || relation == RELATION_UNORDERED,
        9 => relation == RELATION_LESS || relation == RELATION_UNORDERED,
        10 => relation != RELATION_GREATER,
        11 => false,
        12 => relation != RELATION_EQUAL && relation != RELATION_UNORDERED,
        13 => relation == RELATION_GREATER || relation == RELATION_EQUAL,
        14 => relation == RELATION_GREATER,
        15 => true,
        _ => unreachable!("compare predicate base is masked to 0..15"),
    }
}

/// Evaluate one of the 32 single-precision compare predicates.
/// Bochs `avx_compare32[predicate]`.
#[inline]
pub(crate) fn f32_compare_predicate(
    predicate: u8,
    a: float32,
    b: float32,
    status: &mut SoftFloatStatus,
) -> bool {
    let base = predicate & 0xF;
    // Predicates 16..31 repeat 0..15 with the opposite signalling rule.
    let quiet = PREDICATE_IS_QUIET[base as usize] ^ (predicate & 0x10 != 0);
    let relation = if quiet {
        f32_compare_quiet(a, b, status)
    } else {
        f32_compare(a, b, status)
    };
    relation_matches(base, relation)
}

/// Evaluate one of the 32 double-precision compare predicates.
/// Bochs `avx_compare64[predicate]`.
#[inline]
pub(crate) fn f64_compare_predicate(
    predicate: u8,
    a: float64,
    b: float64,
    status: &mut SoftFloatStatus,
) -> bool {
    let base = predicate & 0xF;
    let quiet = PREDICATE_IS_QUIET[base as usize] ^ (predicate & 0x10 != 0);
    let relation = if quiet {
        f64_compare_quiet(a, b, status)
    } else {
        f64_compare(a, b, status)
    };
    relation_matches(base, relation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmp32(pred: u8, a: f32, b: f32) -> bool {
        let mut st = SoftFloatStatus::default();
        f32_compare_predicate(pred, a.to_bits(), b.to_bits(), &mut st)
    }
    fn cmp64(pred: u8, a: f64, b: f64) -> bool {
        let mut st = SoftFloatStatus::default();
        f64_compare_predicate(pred, a.to_bits(), b.to_bits(), &mut st)
    }

    #[test]
    fn legacy_sse_predicates_match_their_definitions() {
        // Predicates 0..7 are the ones legacy CMPPS/CMPPD can encode.
        for (a, b) in [(1.0f32, 2.0f32), (2.0, 1.0), (1.0, 1.0)] {
            assert_eq!(cmp32(0, a, b), a == b, "EQ {a} {b}");
            assert_eq!(cmp32(1, a, b), a < b, "LT {a} {b}");
            assert_eq!(cmp32(2, a, b), a <= b, "LE {a} {b}");
            assert!(!cmp32(3, a, b), "UNORD {a} {b}");
            assert_eq!(cmp32(4, a, b), a != b, "NEQ {a} {b}");
            assert_eq!(cmp32(5, a, b), !(a < b), "NLT {a} {b}");
            assert_eq!(cmp32(6, a, b), !(a <= b), "NLE {a} {b}");
            assert!(cmp32(7, a, b), "ORD {a} {b}");
        }
        let nan = f32::NAN;
        assert!(cmp32(3, nan, 1.0)); // UNORD
        assert!(!cmp32(7, nan, 1.0)); // ORD
        assert!(cmp32(4, nan, 1.0)); // NEQ_UQ is true for unordered
        assert!(!cmp32(0, nan, 1.0)); // EQ_OQ is false for unordered
    }

    #[test]
    fn avx_only_predicates_cover_the_full_table() {
        assert!(!cmp64(11, 1.0, 1.0)); // FALSE_OQ
        assert!(cmp64(15, 1.0, 2.0)); // TRUE_UQ
        assert!(cmp64(13, 2.0, 1.0)); // GE_OS
        assert!(cmp64(14, 2.0, 1.0)); // GT_OS
        assert!(!cmp64(14, 1.0, 1.0));
        assert!(cmp64(8, f64::NAN, 1.0)); // EQ_UQ true when unordered
        assert!(!cmp64(12, f64::NAN, 1.0)); // NEQ_OQ false when unordered
        assert!(cmp64(9, 1.0, 2.0)); // NGE_US
        assert!(cmp64(10, 1.0, 2.0)); // NGT_US
    }

    // The quiet/signalling split is exactly what decides whether #I is
    // raised — the behaviour the old native-float port could not model.
    #[test]
    fn quiet_and_signalling_predicates_differ_on_qnan() {
        let qnan = f64::NAN.to_bits(); // Rust's NAN is quiet
        let one = 1.0f64.to_bits();

        let mut st = SoftFloatStatus::default();
        f64_compare_predicate(0, qnan, one, &mut st); // EQ_OQ — quiet
        assert_eq!(st.softfloat_exceptionFlags & FLAG_INVALID, 0);

        let mut st = SoftFloatStatus::default();
        f64_compare_predicate(16, qnan, one, &mut st); // EQ_OS — signalling
        assert_eq!(st.softfloat_exceptionFlags & FLAG_INVALID, FLAG_INVALID);

        // An SNaN raises #I for both.
        let snan = 0x7FF0_0000_0000_0001u64;
        let mut st = SoftFloatStatus::default();
        f64_compare_predicate(0, snan, one, &mut st);
        assert_eq!(st.softfloat_exceptionFlags & FLAG_INVALID, FLAG_INVALID);
    }
}
