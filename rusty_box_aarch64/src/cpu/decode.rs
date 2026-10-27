//! Instruction classification: which handler class executes a word.
//!
//! What an instruction word decodes to depends on nothing but the word and
//! the features the processor implements, which never change. The decoder
//! runs when a trace is built (`trace.rs`) and when an instruction is
//! fetched from memory whose words cannot be kept; the trace cache keeps
//! what it found, so straight-line code is decoded once.

use super::features::Features;
use super::isa::{self, Class};

/// Classify `word` on a processor implementing `features`: the class of the
/// encoding [`isa::decode`] finds; for one of a feature the processor does
/// not implement, the class the `IsFeatureImplemented` check of its decode
/// leaves ([`isa::Encoding::class_without_feature`]); and
/// [`Class::Undefined`] where it finds none.
pub(crate) fn classify(word: u32, features: Features) -> Class {
    match isa::decode(word) {
        Some(encoding) if features.has(encoding.feature()) => encoding.class(),
        Some(encoding) => encoding.class_without_feature(),
        None => Class::Undefined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::isa::Encoding;

    /// The class of `word` on a Cortex-A57.
    fn a57(word: u32) -> Class {
        classify(word, Features::CORTEX_A57)
    }

    /// Words of different classes, the hints among them, and a word of an
    /// absent feature.
    #[test]
    fn words_classify_by_their_encoding() {
        assert_eq!(a57(0x9100_1020), Class::AddSubImmediate, "add x0, x1, #4");
        assert_eq!(a57(0xD65F_03C0), Class::BranchRegister, "ret");
        assert_eq!(a57(0xD503_203F), Class::Hint, "YIELD");
        assert_eq!(a57(0xD503_233F), Class::Hint, "PACIASP is a hint");
        assert_eq!(a57(0), Class::Undefined, "udf #0");
        assert_eq!(
            a57(0x0420_0000),
            Class::Undefined,
            "an SVE encoding: the feature is absent from ID_AA64PFR0_EL1"
        );
    }

    /// An RPRFM word (FEAT_RPRFM, Armv8.8) is a PRFM (register) with a
    /// reserved prfop on a core without the feature, which the Arm ARM
    /// executes as a prefetch hint.
    #[test]
    fn range_prefetch_words_are_register_prefetches() {
        assert_eq!(a57(0xF8A0_C879), Class::LoadStoreRegisterOffset);
    }

    /// Fixed-point conversions are half precision only for `immh = 001x`
    /// (Arm ARM SCVTF (vector, fixed-point); QEMU `a64.decode` `@fcvtq_h`,
    /// `@fcvtq_s`, `@fcvtq_d`): the single- and double-precision forms, vector
    /// and scalar, are ARMv8.0 Advanced SIMD. The half-precision form shares
    /// their Arm encoding, and its handler raises UNDEFINED for it (the
    /// decode pseudocode's FEAT_FP16 check;
    /// `difftest::reserved_simd_and_fp_encodings_are_undefined_before_the_fp_trap`).
    #[test]
    fn fixed_point_conversions_are_advanced_simd() {
        // scvtf v0.4s, v1.4s, #1 (immh 0111) and scvtf s0, s1, #1.
        assert_eq!(a57(0x4F3F_E420), Class::SimdShiftImmediate);
        assert_eq!(a57(0x5F3F_E420), Class::SimdScalarShiftImmediate);
        // fcvtzs d0, d1, #1 (immh 1111).
        assert_eq!(a57(0x5F7F_FC20), Class::SimdScalarShiftImmediate);
    }

    /// UDF is its own encoding (Arm ARM C6.2 UDF, in the "Reserved" group of
    /// C4.1), whatever its immediate: the low bits of `udf #1` are those of
    /// an SVC, but the word is not an exception-generating instruction.
    #[test]
    fn udf_is_permanently_undefined_whatever_its_immediate() {
        for word in [0x0000_0000, 0x0000_0001, 0x0000_0002, 0x0000_FFFF] {
            assert!(matches!(isa::decode(word), Some(Encoding::PermUndef(_))), "{word:#010x}");
            assert_eq!(a57(word), Class::Undefined, "{word:#010x}");
        }
        assert_eq!(isa::decode(0x0001_0000), None, "outside UDF: unallocated");
    }
}
