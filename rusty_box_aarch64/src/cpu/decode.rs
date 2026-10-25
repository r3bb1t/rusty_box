//! Instruction classification, cached by instruction word.
//!
//! What an instruction word decodes to depends on nothing but the word and
//! the features the processor implements, which never change, so an entry
//! is validated by the word itself: a hit can never be stale, and code that
//! rewrites itself needs no invalidation. Entries are *placed* by the
//! instruction's physical address, so straight-line code walks the table
//! sequentially and stays in cache. The cache only spares the decoder's
//! walk; every handler still reads its operands from the word.

use super::features::Features;
use super::isa::{self, Class};

/// Classify `word` on a processor implementing `features`: the class of the
/// encoding [`isa::decode`] finds, or [`Class::Undefined`] where it finds
/// none or finds one of a feature the processor does not implement (the
/// `IsFeatureImplemented` check every encoding's decode makes).
pub(crate) fn classify(word: u32, features: Features) -> Class {
    match isa::decode(word) {
        Some(encoding) if features.has(encoding.feature()) => encoding.class(),
        Some(_) | None => Class::Undefined,
    }
}

/// 2^16 slots, 64K instructions of code, in 320 KiB: Alpine's kernel and
/// user space together execute about 94,000 distinct words, and a
/// direct-mapped table an eighth this size misses on about one fetch in ten.
const INDEX_BITS: u32 = 16;
const ENTRIES: usize = 1 << INDEX_BITS;

/// The class of word 0, a UDF: what every slot starts out holding, with
/// word 0, so that an empty slot is a correct entry and a hit needs no
/// validity flag. Word 0's class is pinned to it by a test.
const EMPTY_CLASS: Class = Class::Undefined;

/// A direct-mapped table of classified words, kept as two arrays indexed
/// alike: five bytes a slot rather than an eight-byte padded pair.
pub struct DecodeCache {
    words: [u32; ENTRIES],
    classes: [Class; ENTRIES],
    /// What the processor implements, which every miss classifies against.
    features: Features,
}

impl DecodeCache {
    pub(crate) const fn new(features: Features) -> Self {
        Self {
            words: [0; ENTRIES],
            classes: [EMPTY_CLASS; ENTRIES],
            features,
        }
    }

    /// Classify `word`, fetched from physical address `pa`.
    #[inline]
    pub(crate) fn decode(&mut self, pa: u64, word: u32) -> Class {
        let slot = ((pa >> 2) as usize) & (ENTRIES - 1);
        if self.words[slot] == word {
            return self.classes[slot];
        }
        self.fill(slot, word)
    }

    /// The miss path, kept out of the dispatch loop: the decoder is large,
    /// and almost every fetch hits.
    #[cold]
    #[inline(never)]
    fn fill(&mut self, slot: usize, word: u32) -> Class {
        let class = classify(word, self.features);
        self.words[slot] = word;
        self.classes[slot] = class;
        class
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

    /// A slot reused by a different word — another address, or code that was
    /// rewritten in place — classifies the new word, never the old one.
    #[test]
    fn a_rewritten_or_aliasing_slot_classifies_the_new_word() {
        let mut c = DecodeCache::new(Features::CORTEX_A57);
        let add = 0x9100_1020; // add x0, x1, #4
        let ret = 0xD65F_03C0; // ret
        assert_eq!(c.decode(0x1000, add), Class::AddSubImmediate);
        assert_eq!(c.decode(0x1000, ret), Class::BranchRegister, "same address, new code");
        let aliasing = 0x1000 + (ENTRIES as u64) * 4;
        assert_eq!(c.decode(aliasing, add), Class::AddSubImmediate, "same slot, other address");
        assert_eq!(c.decode(0x2000, 0xD503_203F), Class::Hint, "YIELD");
        assert_eq!(c.decode(0x2004, 0xD503_233F), Class::Hint, "PACIASP is a hint");
        assert_eq!(
            c.decode(0x2008, 0x0420_0000),
            Class::Undefined,
            "an SVE encoding: the feature is absent from ID_AA64PFR0_EL1"
        );
    }

    /// An empty slot holds word 0 with [`EMPTY_CLASS`], which must be what
    /// word 0 classifies as, or a fetch of word 0 from a slot never filled
    /// would execute as the wrong class.
    #[test]
    fn an_empty_slot_is_a_correct_entry_for_word_zero() {
        assert_eq!(a57(0), EMPTY_CLASS);
        let mut c = DecodeCache::new(Features::CORTEX_A57);
        for pa in [0, 0x4000_0000, 0xFFFF_FFFC] {
            assert_eq!(c.decode(pa, 0), Class::Undefined, "udf #0 at {pa:#x}");
        }
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
