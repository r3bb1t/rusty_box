//! Instruction classification, cached by instruction word.
//!
//! What an instruction word decodes to depends on nothing but the word, so
//! an entry is validated by the word itself: a hit can never be stale, and
//! code that rewrites itself needs no invalidation. Entries are *placed* by
//! the instruction's physical address, so straight-line code walks the table
//! sequentially and stays in cache. The cache only spares the classifier's
//! decision tree; every handler still reads its operands from the word.

use disarm64::InsnOpcode;
use disarm64_defn::{InsnClass, InsnFeatureSet};

/// What a word is, as far as dispatch is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decoded {
    /// In the HINT space: executes whatever the hint, as a NOP if this core
    /// does not implement it.
    Hint,
    /// Unallocated, or belonging to a feature this core lacks.
    Undefined,
    /// An encoding in the implemented feature set, of this class.
    Class(InsnClass),
}

/// The ARMv8.0 feature sets this core implements. Every other feature set is
/// absent from the ID registers, so its encodings are UNDEFINED.
#[inline]
fn feature_implemented(set: InsnFeatureSet) -> bool {
    matches!(set, InsnFeatureSet::V8 | InsnFeatureSet::FP | InsnFeatureSet::SIMD)
}

/// The HINT space (`D503201F | CRm:op2 << 5`): every encoding in it executes,
/// as a NOP if the hint is not implemented — including the v8.3 pointer
/// authentication and v8.5 BTI hints, which is what lets code built with
/// branch protection run on an ARMv8.0 core.
#[inline]
fn is_hint(insn: u32) -> bool {
    insn & 0xFFFF_F01F == 0xD503_201F
}

/// disarm64 0.2 tags SCVTF, UCVTF, FCVTZS and FCVTZU (fixed point, vector
/// and scalar shift-by-immediate) as the FEAT_FP16 form whenever `immh<1>`
/// is set, but only `immh = 001x` is half precision: with `immh<3:2>`
/// nonzero the word is the single- (01xx) or double-precision (1xxx) form,
/// which ARMv8.0 has (Arm ARM C7.2; QEMU `a64.decode` `@fcvtq_s`/`@fcvtq_d`).
fn is_mistagged_fixed_point_conversion(class: InsnClass, set: InsnFeatureSet, word: u32) -> bool {
    let immh = (word >> 19) & 0xF;
    set == InsnFeatureSet::SIMD_F16
        && matches!(class, InsnClass::ASIMDSHF | InsnClass::ASISDSHF)
        && immh & 0b1100 != 0
}

pub(crate) fn classify(word: u32) -> Decoded {
    if is_hint(word) {
        return Decoded::Hint;
    }
    match disarm64::decoder::decode(word) {
        Some(op) => {
            let def = op.definition();
            if is_mistagged_fixed_point_conversion(def.class, def.feature_set, word) {
                Decoded::Class(def.class)
            } else if !feature_implemented(def.feature_set) {
                Decoded::Undefined
            } else if def.class == InsnClass::SME_MISC {
                // disarm64 0.2 files RPRFM (FEAT_RPRFM, Armv8.8) as SME_MISC
                // in the V8 feature set — the only post-v8.0 encoding it tags
                // that way. Without FEAT_RPRFM these words are PRFM
                // (register) with a reserved prfop, which the Arm ARM
                // executes as a prefetch hint.
                Decoded::Class(InsnClass::LDST_REGOFF)
            } else {
                Decoded::Class(def.class)
            }
        }
        None => Decoded::Undefined,
    }
}

const INDEX_BITS: u32 = 13;
const ENTRIES: usize = 1 << INDEX_BITS;

#[derive(Debug, Clone, Copy)]
struct Entry {
    word: u32,
    valid: bool,
    decoded: Decoded,
}

const EMPTY: Entry = Entry {
    word: 0,
    valid: false,
    decoded: Decoded::Undefined,
};

/// A direct-mapped table of classified words.
pub struct DecodeCache {
    entries: [Entry; ENTRIES],
}

impl DecodeCache {
    pub(crate) const fn new() -> Self {
        Self {
            entries: [EMPTY; ENTRIES],
        }
    }

    /// Classify `word`, fetched from physical address `pa`.
    #[inline]
    pub(crate) fn decode(&mut self, pa: u64, word: u32) -> Decoded {
        let slot = ((pa >> 2) as usize) & (ENTRIES - 1);
        let e = self.entries[slot];
        if e.valid && e.word == word {
            return e.decoded;
        }
        let decoded = classify(word);
        self.entries[slot] = Entry {
            word,
            valid: true,
            decoded,
        };
        decoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slot reused by a different word — another address, or code that was
    /// rewritten in place — classifies the new word, never the old one.
    #[test]
    fn a_rewritten_or_aliasing_slot_classifies_the_new_word() {
        let mut c = DecodeCache::new();
        let add = 0x9100_1020; // add x0, x1, #4
        let ret = 0xD65F_03C0; // ret
        assert_eq!(c.decode(0x1000, add), Decoded::Class(InsnClass::ADDSUB_IMM));
        assert_eq!(c.decode(0x1000, ret), Decoded::Class(InsnClass::BRANCH_REG), "same address, new code");
        let aliasing = 0x1000 + (ENTRIES as u64) * 4;
        assert_eq!(c.decode(aliasing, add), Decoded::Class(InsnClass::ADDSUB_IMM), "same slot, other address");
        assert_eq!(c.decode(0x2000, 0xD503_203F), Decoded::Hint, "YIELD");
        assert_eq!(c.decode(0x2004, 0xD503_233F), Decoded::Hint, "PACIASP is a hint");
        assert_eq!(
            c.decode(0x2008, 0x0420_0000),
            Decoded::Undefined,
            "an SVE encoding: the feature is absent from ID_AA64PFR0_EL1"
        );
    }

    /// An RPRFM word is a PRFM (register) on a core without FEAT_RPRFM.
    #[test]
    fn range_prefetch_words_are_register_prefetches() {
        assert_eq!(classify(0xF8A0_C879), Decoded::Class(InsnClass::LDST_REGOFF));
    }

    /// Fixed-point conversions are half precision only for `immh = 001x`.
    #[test]
    fn fixed_point_conversions_need_fp16_only_for_half_precision() {
        // scvtf v0.4s, v1.4s, #1 (immh 0111) and scvtf s0, s1, #1.
        assert_eq!(classify(0x4F3F_E420), Decoded::Class(InsnClass::ASIMDSHF));
        assert_eq!(classify(0x5F3F_E420), Decoded::Class(InsnClass::ASISDSHF));
        // fcvtzs d0, d1, #1 (immh 1111).
        assert_eq!(classify(0x5F7F_FC20), Decoded::Class(InsnClass::ASISDSHF));
        // scvtf v0.8h, v1.8h, #1 (immh 0011): FEAT_FP16.
        assert_eq!(classify(0x4F1F_E420), Decoded::Undefined);
    }
}
