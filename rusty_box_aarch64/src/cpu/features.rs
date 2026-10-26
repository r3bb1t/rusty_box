//! What a processor implements: the architectural features, by the Arm
//! ARM's `FEAT_` names, that decide which encodings execute and what the ID
//! registers report.
//!
//! Provenance (R7): the Arm A-profile Architecture Machine Readable
//! Specification, `Features.json` and `Instructions.json` (release
//! 2026-09): each feature's dependencies, the ID register fields that report
//! it, and the condition each encoding carries.
//!
//! A feature is implemented by a processor when its [`Features`] set holds
//! it; the decoder executes an encoding only when the processor implements
//! the feature the encoding belongs to. Every processor built implements
//! the Cortex-A57's features, which the constants of `id` report, and the
//! ID register field of each feature beyond them is read from the set
//! (`sysreg.rs`); `isa::tests` checks, for every implementation built, that
//! the ID registers and the set agree.

/// An architectural feature, by the Arm ARM's `FEAT_` name; the instruction
/// set features are what an encoding belongs to.
///
/// Closed (R5): the features this processor can implement, each one a bit of
/// [`Features`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Feature {
    /// The ARMv8.0 base instruction set: an encoding without a feature
    /// condition.
    Base,
    /// FEAT_FP: scalar floating point, and the loads and stores of SIMD&FP
    /// registers.
    Fp,
    /// FEAT_AdvSIMD: Advanced SIMD.
    AdvSimd,
    /// FEAT_AES: AESE, AESD, AESMC, AESIMC. FEAT_PMULL, the 64-bit PMULL
    /// the same ID field advertises, is no encoding of its own: PMULL's
    /// decode pseudocode checks it.
    Aes,
    /// FEAT_SHA1: SHA1C, SHA1P, SHA1M, SHA1H, SHA1SU0, SHA1SU1.
    Sha1,
    /// FEAT_SHA256: SHA256H, SHA256H2, SHA256SU0, SHA256SU1.
    Sha256,
    /// FEAT_CRC32: CRC32B/H/W/X, CRC32CB/CH/CW/CX.
    Crc32,
    /// FEAT_SME: Streaming SVE mode, the ZA array and the instructions that
    /// use them.
    Sme,
    /// FEAT_LSE: the atomic memory operations, SWP, CAS and CASP.
    Lse,
    /// FEAT_LSE2: unaligned exclusive and atomic accesses within a 16-byte
    /// granule, and `SCTLR_EL1.nAA` for the ordered ones. No encoding of its
    /// own.
    Lse2,
    /// FEAT_LRCPC: LDAPR, LDAPRB, LDAPRH.
    Lrcpc,
    /// FEAT_LRCPC2: LDAPUR*, STLUR*.
    Lrcpc2,
    /// FEAT_RDM: SQRDMLAH, SQRDMLSH.
    Rdm,
    /// FEAT_DotProd: SDOT, UDOT.
    DotProd,
    /// FEAT_I8MM: USDOT, SUDOT, SMMLA, USMMLA, UMMLA.
    I8mm,
    /// FEAT_FP16: half-precision arithmetic, scalar and Advanced SIMD.
    Fp16,
    /// FEAT_FHM: FMLAL, FMLSL, FMLAL2, FMLSL2.
    Fhm,
    /// FEAT_FCMA: FCMLA and FCADD.
    Fcma,
    /// FEAT_JSCVT: FJCVTZS.
    Jscvt,
    /// FEAT_FRINTTS: FRINT32Z, FRINT32X, FRINT64Z, FRINT64X.
    Frintts,
    /// FEAT_BF16: BFCVT, BFCVTN, BFDOT, BFMLALB, BFMLALT, BFMMLA.
    Bf16,
    /// FEAT_PAuth: the pointer authentication instructions, keys and
    /// enables.
    PAuth,
    /// FEAT_PAuth2: a code XORed into the pointer, and no error code in a
    /// failed one. No encoding of its own.
    PAuth2,
    /// FEAT_FPAC: a failed authentication alone faults. No encoding of its
    /// own.
    Fpac,
    /// FEAT_FPACCOMBINE: a failed authentication in a branch, return or load
    /// faults too. No encoding of its own.
    FpacCombine,
    /// FEAT_PACQARMA5: the code computed with QARMA5. No encoding of its
    /// own.
    PacQarma5,
}

/// What the conformance tests read of a feature: `tests/unit/spec.rs`
/// checks the decoder against Arm's data feature by feature.
#[cfg(test)]
impl Feature {
    /// Every feature, in declaration order.
    pub(crate) const ALL: [Feature; 26] = [
        Feature::Base,
        Feature::Fp,
        Feature::AdvSimd,
        Feature::Aes,
        Feature::Sha1,
        Feature::Sha256,
        Feature::Crc32,
        Feature::Sme,
        Feature::Lse,
        Feature::Lse2,
        Feature::Lrcpc,
        Feature::Lrcpc2,
        Feature::Rdm,
        Feature::DotProd,
        Feature::I8mm,
        Feature::Fp16,
        Feature::Fhm,
        Feature::Fcma,
        Feature::Jscvt,
        Feature::Frintts,
        Feature::Bf16,
        Feature::PAuth,
        Feature::PAuth2,
        Feature::Fpac,
        Feature::FpacCombine,
        Feature::PacQarma5,
    ];

    /// The name Arm's data gives the feature, `None` for the base set.
    pub(crate) const fn arm_name(self) -> Option<&'static str> {
        match self {
            Self::Base => None,
            Self::Fp => Some("FEAT_FP"),
            Self::AdvSimd => Some("FEAT_AdvSIMD"),
            Self::Aes => Some("FEAT_AES"),
            Self::Sha1 => Some("FEAT_SHA1"),
            Self::Sha256 => Some("FEAT_SHA256"),
            Self::Crc32 => Some("FEAT_CRC32"),
            Self::Sme => Some("FEAT_SME"),
            Self::Lse => Some("FEAT_LSE"),
            Self::Lse2 => Some("FEAT_LSE2"),
            Self::Lrcpc => Some("FEAT_LRCPC"),
            Self::Lrcpc2 => Some("FEAT_LRCPC2"),
            Self::Rdm => Some("FEAT_RDM"),
            Self::DotProd => Some("FEAT_DotProd"),
            Self::I8mm => Some("FEAT_I8MM"),
            Self::Fp16 => Some("FEAT_FP16"),
            Self::Fhm => Some("FEAT_FHM"),
            Self::Fcma => Some("FEAT_FCMA"),
            Self::Jscvt => Some("FEAT_JSCVT"),
            Self::Frintts => Some("FEAT_FRINTTS"),
            Self::Bf16 => Some("FEAT_BF16"),
            Self::PAuth => Some("FEAT_PAuth"),
            Self::PAuth2 => Some("FEAT_PAuth2"),
            Self::Fpac => Some("FEAT_FPAC"),
            Self::FpacCombine => Some("FEAT_FPACCOMBINE"),
            Self::PacQarma5 => Some("FEAT_PACQARMA5"),
        }
    }

    /// Whether the feature has encodings of its own; one that has none
    /// changes how other instructions behave.
    pub(crate) const fn encodes(self) -> bool {
        !matches!(self, Self::Lse2 | Self::PAuth2 | Self::Fpac | Self::FpacCombine | Self::PacQarma5)
    }

    /// Whether the decoder names every encoding of the feature: a feature
    /// still being built has some of its encodings, each of them Arm's, and
    /// no processor this crate builds implements it yet.
    pub(crate) const fn complete(self) -> bool {
        !matches!(self, Self::Sme)
    }
}

impl Feature {
    const fn bit(self) -> u64 {
        1 << self as u32
    }
}

/// A set of [`Feature`]s: what one processor implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Features(u64);

impl Features {
    /// What the Cortex-A57 this crate's processor is shaped like implements,
    /// as QEMU's `cortex-a57` has it: ARMv8.0 with FP, Advanced SIMD and
    /// the whole Cryptographic Extension and CRC32.
    pub(crate) const CORTEX_A57: Self = Self::of(&[
        Feature::Base,
        Feature::Fp,
        Feature::AdvSimd,
        Feature::Aes,
        Feature::Sha1,
        Feature::Sha256,
        Feature::Crc32,
    ]);

    /// The set of `features`.
    pub(crate) const fn of(features: &[Feature]) -> Self {
        let mut bits = 0;
        let mut i = 0;
        while i < features.len() {
            bits |= features[i].bit();
            i += 1;
        }
        Self(bits)
    }

    #[inline(always)]
    pub(crate) const fn has(self, feature: Feature) -> bool {
        self.0 & feature.bit() != 0
    }

    /// The set as a bit per [`Feature`], in declaration order: what a
    /// snapshot records of the processor it was taken from.
    pub(crate) const fn bits(self) -> u64 {
        self.0
    }
}

/// A scalable vector length (the Arm ARM's `VecLen`): a power of two from
/// 128 to 2048 bits, which is every length this processor supports (Arm
/// `SupportedPowerTwoSVL`), and the only unit an SVE or SME length is
/// counted in here (R4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct VecLen(u16);

impl VecLen {
    /// The shortest length: 128 bits.
    pub(crate) const MIN: Self = Self(128);
    /// `MAX_VL`, the architecture's longest vector: 2048 bits.
    pub(crate) const MAX: Self = Self(2048);

    #[inline]
    pub(crate) const fn bits(self) -> u32 {
        self.0 as u32
    }

    /// The length in bytes: a vector register's, and the number of
    /// horizontal vectors of the ZA array at this length.
    #[inline]
    pub(crate) const fn bytes(self) -> usize {
        self.0 as usize / 8
    }

    /// The length of a predicate register for vectors of this length, in
    /// bytes: one bit per vector byte.
    #[inline]
    pub(crate) const fn predicate_bytes(self) -> usize {
        self.0 as usize / 64
    }

    /// Arm `ImplementedSMEVectorLength((len + 1) * 128)`: the longest
    /// supported length not above the one a `LEN` field of `SMCR_ELx` asks
    /// for, capped at `max`. Every power of two up to `max` is supported,
    /// so that is the largest power of two at most `min((len + 1) * 128,
    /// max)`.
    pub(crate) const fn from_len_field(len: u64, max: Self) -> Self {
        let requested = ((len & 0xF) as u32 + 1) * 128;
        let capped = if requested < max.0 as u32 { requested } else { max.0 as u32 };
        Self((1u32 << (31 - capped.leading_zeros())) as u16)
    }
}

/// What one processor implements, and the IMPLEMENTATION DEFINED choices
/// the features it implements leave open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Implementation {
    pub(crate) features: Features,
    /// `MaxImplementedSVL()`: the longest Streaming SVE vector length, which
    /// [`Feature::Sme`] reads and nothing else does.
    pub(crate) max_svl: VecLen,
}

impl Implementation {
    /// The Cortex-A57 shape: [`Features::CORTEX_A57`], with no streaming
    /// vector length to choose.
    pub(crate) const CORTEX_A57: Self = Self {
        features: Features::CORTEX_A57,
        max_svl: VecLen::MIN,
    };

    #[inline(always)]
    pub(crate) const fn has(self, feature: Feature) -> bool {
        self.features.has(feature)
    }
}

/// What the tests of a feature run on while no processor model implements
/// it.
#[cfg(test)]
impl Implementation {
    /// Every feature, with Streaming SVE vectors of up to 512 bits. It is
    /// no processor Arm permits while the rest of Armv9.2, which FEAT_SME
    /// requires (`Features.json`: FEAT_SME implies Armv9.2), is still being
    /// added; each feature's tests exercise that feature's own rules.
    pub(crate) const EVERY_FEATURE: Self = Self {
        features: Features::of(&Feature::ALL),
        max_svl: VecLen(512),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A feature still being built is in no processor this crate builds:
    /// only the tests' own implementation has it. Every implementation has
    /// the Cortex-A57's features, which `id`'s constants report.
    #[test]
    fn the_processors_built_implement_only_complete_features() {
        for feature in Feature::ALL {
            assert!(
                feature.complete() || !Implementation::CORTEX_A57.has(feature),
                "{feature:?} is incomplete"
            );
            if Implementation::CORTEX_A57.has(feature) {
                assert!(Implementation::EVERY_FEATURE.has(feature), "{feature:?}");
            }
        }
    }

    /// QARMA5 is the one algorithm the pointer authentication instructions
    /// compute, so a processor with FEAT_PAuth implements FEAT_PACQARMA5,
    /// and its ID registers name the algorithm (Arm requires one:
    /// `FEAT_PAuth` with none of `FEAT_PACQARMA5`, `FEAT_PACQARMA3` and
    /// `FEAT_PACIMP` advertises no pointer authentication).
    #[test]
    fn a_processor_with_pauth_computes_with_qarma5() {
        for imp in [Implementation::CORTEX_A57, Implementation::EVERY_FEATURE] {
            assert!(!imp.has(Feature::PAuth) || imp.has(Feature::PacQarma5), "{imp:?}");
        }
    }

    /// `ImplementedSMEVectorLength`: `LEN` asks for `(LEN + 1) * 128`
    /// bits, and gets the longest power of two not above that or the
    /// implemented maximum.
    #[test]
    fn a_requested_length_rounds_down_to_a_supported_one() {
        let max = VecLen(512);
        let svl = |len: u64| VecLen::from_len_field(len, max).bits();
        assert_eq!(svl(0), 128);
        assert_eq!(svl(1), 256);
        assert_eq!(svl(2), 256, "384 is not a power of two");
        assert_eq!(svl(3), 512);
        assert_eq!(svl(7), 512, "1024 is above the maximum");
        assert_eq!(svl(15), 512);
        assert_eq!(VecLen::from_len_field(15, VecLen::MAX).bits(), 2048);
        assert_eq!(VecLen::from_len_field(0x1F, VecLen::MAX).bits(), 2048, "LEN is four bits");
    }
}
