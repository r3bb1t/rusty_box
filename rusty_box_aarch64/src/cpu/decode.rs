//! Instruction classification, cached by instruction word.
//!
//! What an instruction word decodes to depends on nothing but the word, so
//! an entry is validated by the word itself: a hit can never be stale, and
//! code that rewrites itself needs no invalidation. Entries are *placed* by
//! the instruction's physical address, so straight-line code walks the table
//! sequentially and stays in cache. The cache only spares the classifier's
//! decision tree; every handler still reads its operands from the word.

use super::isa::{self, Class, Kind};

/// What a word is, as far as dispatch is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decoded {
    /// In the HINT space: executes whatever the hint, as a NOP if this core
    /// does not implement it.
    Hint,
    /// Unallocated, or belonging to a feature this core lacks.
    Undefined,
    /// An encoding of this class, whose feature this core advertises.
    Class(Class),
}

/// Classify `word` by the encoding list (`isa/encodings.txt`): the matching
/// encoding with the most fixed bits, gated on its feature. This is the one
/// place an encoding of an absent feature becomes UNDEFINED (R5).
pub(crate) fn classify(word: u32) -> Decoded {
    match isa::lookup(word) {
        None => Decoded::Undefined,
        Some(encoding) => match encoding.kind {
            Kind::Hint => Decoded::Hint,
            Kind::Class(class) if class.feature().advertised() => Decoded::Class(class),
            Kind::Class(_) | Kind::Absent(_) => Decoded::Undefined,
        },
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
        self.fill(slot, word)
    }

    /// The miss path, kept out of the dispatch loop: the decision tree is
    /// large, and almost every fetch hits.
    #[cold]
    #[inline(never)]
    fn fill(&mut self, slot: usize, word: u32) -> Decoded {
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
        assert_eq!(c.decode(0x1000, add), Decoded::Class(Class::AddSubImmediate));
        assert_eq!(c.decode(0x1000, ret), Decoded::Class(Class::BranchRegister), "same address, new code");
        let aliasing = 0x1000 + (ENTRIES as u64) * 4;
        assert_eq!(c.decode(aliasing, add), Decoded::Class(Class::AddSubImmediate), "same slot, other address");
        assert_eq!(c.decode(0x2000, 0xD503_203F), Decoded::Hint, "YIELD");
        assert_eq!(c.decode(0x2004, 0xD503_233F), Decoded::Hint, "PACIASP is a hint");
        assert_eq!(
            c.decode(0x2008, 0x0420_0000),
            Decoded::Undefined,
            "an SVE encoding: the feature is absent from ID_AA64PFR0_EL1"
        );
    }

    /// An RPRFM word (FEAT_RPRFM, Armv8.8) is a PRFM (register) with a
    /// reserved prfop on a core without the feature, which the Arm ARM
    /// executes as a prefetch hint.
    #[test]
    fn range_prefetch_words_are_register_prefetches() {
        assert_eq!(classify(0xF8A0_C879), Decoded::Class(Class::LoadStoreRegisterOffset));
    }

    /// Fixed-point conversions are half precision only for `immh = 001x`
    /// (Arm ARM C7.2 SCVTF (vector, fixed-point); QEMU `a64.decode`
    /// `@fcvtq_h`, `@fcvtq_s`, `@fcvtq_d`).
    #[test]
    fn fixed_point_conversions_need_fp16_only_for_half_precision() {
        // scvtf v0.4s, v1.4s, #1 (immh 0111) and scvtf s0, s1, #1.
        assert_eq!(classify(0x4F3F_E420), Decoded::Class(Class::SimdShiftImmediate));
        assert_eq!(classify(0x5F3F_E420), Decoded::Class(Class::SimdScalarShiftImmediate));
        // fcvtzs d0, d1, #1 (immh 1111).
        assert_eq!(classify(0x5F7F_FC20), Decoded::Class(Class::SimdScalarShiftImmediate));
        // scvtf v0.8h, v1.8h, #1 (immh 0011): FEAT_FP16.
        assert_eq!(classify(0x4F1F_E420), Decoded::Undefined);
    }

    /// UDF is its own encoding (Arm ARM C6.2 UDF, in the "Reserved" group of
    /// C4.1), whatever its immediate: the low bits of `udf #1` are those of
    /// an SVC, but the word is not an exception-generating instruction.
    #[test]
    fn udf_is_permanently_undefined_whatever_its_immediate() {
        for word in [0x0000_0000, 0x0000_0001, 0x0000_0002, 0x0000_FFFF] {
            assert_eq!(classify(word), Decoded::Class(Class::PermanentlyUndefined), "{word:#010x}");
        }
        assert_eq!(classify(0x0001_0000), Decoded::Undefined, "outside UDF: unallocated");
    }

    /// The equivalence the encoding list is built to: on every word, the
    /// classifier names the encoding disarm64 0.2 names (by opcode and mask),
    /// for every encoding of a feature this core advertises, and says
    /// UNDEFINED wherever disarm64 names nothing or a feature this core
    /// lacks — with exactly these deliberate exceptions, each a fault in
    /// disarm64's tagging of an encoding rather than a choice:
    ///
    /// - the HINT space executes whatever the hint, so CHKFEAT (FEAT_CHK,
    ///   Armv9.4, a hint) is HINT here;
    /// - RPRFM (FEAT_RPRFM) is tagged as a base encoding there; without the
    ///   feature the word is PRFM (register);
    /// - the fixed-point conversions SCVTF, UCVTF, FCVTZS and FCVTZU by
    ///   immediate are tagged FEAT_FP16 for every `immh<1>` set, but only
    ///   `immh = 001x` is half precision: `immh<3:2>` nonzero is the
    ///   single- or double-precision ARMv8.0 encoding.
    mod equivalence {
        use super::super::classify;
        use super::super::Decoded;
        use crate::cpu::isa;
        use disarm64::InsnOpcode;
        use disarm64_defn::InsnFeatureSet;
        use std::sync::Mutex;
        use std::vec::Vec;

        /// The feature sets of disarm64's table this core advertises.
        fn advertised(set: InsnFeatureSet) -> bool {
            matches!(set, InsnFeatureSet::V8 | InsnFeatureSet::FP | InsnFeatureSet::SIMD)
        }

        /// What the classifier must say about a word.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Expected {
            Hint,
            Undefined,
            /// This encoding (opcode, mask), executing.
            Encoding(u32, u32),
        }

        /// Which of the deliberate exceptions, if any, decided a word.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Exception {
            None,
            HintSpace,
            RangePrefetch,
            FixedPointConversion,
        }

        /// disarm64's PRFM (register) row, which RPRFM words fall back to.
        const PRFM_REGISTER: (u32, u32) = (0xF8A0_0800, 0xFFE0_0C00);

        fn expected(word: u32) -> (Expected, Exception) {
            if word & 0xFFFF_F01F == 0xD503_201F {
                let tagged_hint = disarm64::decoder::decode(word).is_some_and(|op| op.definition().mnemonic == "hint");
                return (Expected::Hint, if tagged_hint { Exception::None } else { Exception::HintSpace });
            }
            let Some(op) = disarm64::decoder::decode(word) else {
                return (Expected::Undefined, Exception::None);
            };
            let d = op.definition();
            if d.mnemonic == "rprfm" {
                return (Expected::Encoding(PRFM_REGISTER.0, PRFM_REGISTER.1), Exception::RangePrefetch);
            }
            let immh = (word >> 19) & 0xF;
            let mistagged = d.feature_set == InsnFeatureSet::SIMD_F16
                && matches!(d.class, disarm64_defn::InsnClass::ASIMDSHF | disarm64_defn::InsnClass::ASISDSHF)
                && d.mask & 0x0060_0000 == 0;
            if mistagged && immh & 0b1100 != 0 {
                let single_or_double = Expected::Encoding(d.opcode & !(1 << 20), d.mask & !(1 << 20));
                return (single_or_double, Exception::FixedPointConversion);
            }
            if advertised(d.feature_set) {
                (Expected::Encoding(d.opcode, d.mask), Exception::None)
            } else {
                (Expected::Undefined, Exception::None)
            }
        }

        /// Whether the classifier says what `expected` says.
        fn agrees(word: u32, expected: Expected) -> bool {
            let encoding = isa::lookup(word);
            match expected {
                Expected::Hint => classify(word) == Decoded::Hint,
                Expected::Undefined => classify(word) == Decoded::Undefined,
                Expected::Encoding(opcode, mask) => {
                    encoding.is_some_and(|e| e.opcode == opcode && e.mask == mask)
                        && matches!(classify(word), Decoded::Class(_))
                }
            }
        }

        /// Counts over the words one thread checked.
        #[derive(Default)]
        struct Tally {
            words: u64,
            disagreements: u64,
            hint_space: u64,
            range_prefetch: u64,
            fixed_point: u64,
        }

        impl Tally {
            fn add(&mut self, other: &Tally) {
                self.words += other.words;
                self.disagreements += other.disagreements;
                self.hint_space += other.hint_space;
                self.range_prefetch += other.range_prefetch;
                self.fixed_point += other.fixed_point;
            }
        }

        fn check(word: u32, tally: &mut Tally, first: &Mutex<Vec<u32>>) {
            let (want, exception) = expected(word);
            tally.words += 1;
            match exception {
                Exception::None => {}
                Exception::HintSpace => tally.hint_space += 1,
                Exception::RangePrefetch => tally.range_prefetch += 1,
                Exception::FixedPointConversion => tally.fixed_point += 1,
            }
            if !agrees(word, want) {
                tally.disagreements += 1;
                if let Ok(mut words) = first.lock() {
                    if words.len() < 32 {
                        words.push(word);
                    }
                }
            }
        }

        fn report(tally: &Tally, first: &Mutex<Vec<u32>>) {
            std::eprintln!(
                "{} words: {} disagreements; deliberate differences from disarm64: {} HINT-space, {} RPRFM, {} fixed-point conversions",
                tally.words, tally.disagreements, tally.hint_space, tally.range_prefetch, tally.fixed_point,
            );
            if let Ok(words) = first.lock() {
                for &word in words.iter() {
                    let theirs = disarm64::decoder::decode(word).map(|op| op.definition());
                    std::eprintln!(
                        "  {word:08x}: ours {:?} = {:?}, disarm64 {:?}",
                        isa::lookup(word).map(|e| (e.mnemonic, e.opcode, e.mask)),
                        classify(word),
                        theirs.map(|d| (d.mnemonic, d.opcode, d.mask, d.feature_set)),
                    );
                }
            }
        }

        /// Every row of the encoding list, 64 random instances each, and a
        /// million random words: the equivalence on the words that matter,
        /// fast enough for every test run.
        #[test]
        fn the_classifier_matches_disarm64_on_sampled_words() {
            let mut tally = Tally::default();
            let first = Mutex::new(Vec::new());
            let mut state = 0x5EED_u64;
            let mut next = || {
                state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut z = state;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                (z ^ (z >> 31)) as u32
            };
            for e in isa::ENCODINGS.iter() {
                for _ in 0..64 {
                    check(e.opcode | (next() & !e.mask), &mut tally, &first);
                }
            }
            for _ in 0..1_000_000 {
                check(next(), &mut tally, &first);
            }
            report(&tally, &first);
            assert_eq!(tally.disagreements, 0);
        }

        /// The same, on all 2^32 words.
        #[test]
        #[ignore = "exhaustive: every 32-bit word, on every host thread"]
        fn the_classifier_matches_disarm64_on_every_word() {
            let total = Mutex::new(Tally::default());
            let first = Mutex::new(Vec::new());
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get()) as u64;
            let started = std::time::Instant::now();
            std::thread::scope(|scope| {
                for t in 0..threads {
                    let (total, first) = (&total, &first);
                    scope.spawn(move || {
                        let mut tally = Tally::default();
                        let mut word = t;
                        while word < 1 << 32 {
                            check(word as u32, &mut tally, first);
                            word += threads;
                        }
                        if let Ok(mut total) = total.lock() {
                            total.add(&tally);
                        }
                    });
                }
            });
            std::eprintln!("{threads} threads, {:.1} s", started.elapsed().as_secs_f64());
            let tally = total.into_inner().unwrap_or_default();
            report(&tally, &first);
            assert_eq!(tally.words, 1 << 32, "every word checked once");
            assert_eq!(tally.disagreements, 0);
        }
    }
}
