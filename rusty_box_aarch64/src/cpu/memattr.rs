//! Memory attributes (Arm `MemoryAttributes`): stage 1's, decoded from a
//! `MAIR_ELx` attribute and a descriptor's `SH`; stage 2's, from a stage 2
//! descriptor's `MemAttr`, `SH` and `FnXS`; the attributes of an access both
//! stages translate, combined; and how `PAR_EL1` reports them.
//!
//! Provenance (R7): the shared pseudocode of the A64 ISA XML (release
//! 2026-09): `S1DecodeMemAttrs`, `S1ConstrainUnpredictableRESMAIR`,
//! `DecodeLDFAttr`, `DecodeDevice`, `DecodeShareability`,
//! `S2DecodeMemAttrs`, `S2ResMemAttr`, `S2DecodeCacheability`,
//! `S2CombineS1MemAttrs`, `S2CombineS1AttrHints`, `S2CombineS1Device`,
//! `S2CombineS1Shareability`, `NormalNCMemAttr`, `EffectiveShareability`,
//! `EncodePARAttrs` and `PAREncodeShareability`, and the choices the
//! pseudocode's own `ConstrainUnpredictable` makes for
//! `Unpredictable_RESMAIR` (an UNKNOWN allocated value, zeros: Device-nGnRnE),
//! `Unpredictable_S2RESMEMATTR` (Normal Non-cacheable) and
//! `Unpredictable_Shareability` (Outer Shareable).
//!
//! No allocation tag is modelled: this processor has no FEAT_MTE2, so
//! `MAIR_ELx`'s Tagged encoding is reserved and every attribute is
//! Untagged. Nothing caches data, so the attributes decide no access's
//! effect but Device memory's alignment rules
//! ([`MemAttrs::is_device`]) and what `PAR_EL1` reports.

use super::features::{Feature, Features};

/// Arm `MemAttr`: a level of cache's policy for Normal memory.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cacheability {
    NonCacheable,
    WriteThrough,
    WriteBack,
}

/// Arm `MemAttrHints`: one level of Normal memory's cacheability, with the
/// allocation hints and transient hint only cacheable memory has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AttrHints {
    pub(crate) attrs: Cacheability,
    /// `MemHint`, as `MAIR_ELx` encodes it: bit 1 read-allocate, bit 0
    /// write-allocate. Zero for Non-cacheable memory.
    pub(crate) hints: u8,
    /// Whether the hints are transient. False for Non-cacheable memory.
    pub(crate) transient: bool,
}

/// `MemHint_RA`: read-allocate, no write-allocate.
const HINT_RA: u8 = 0b10;
/// `MemHint_RWA`: read- and write-allocate.
const HINT_RWA: u8 = 0b11;

/// Non-cacheable, which has no hints.
const NON_CACHEABLE: AttrHints = AttrHints {
    attrs: Cacheability::NonCacheable,
    hints: 0,
    transient: false,
};

/// Arm `DeviceType`, from the most restrictive to the least, the order
/// `S2CombineS1Device` takes the more restrictive of two by. The variants
/// are Arm's names (Device-nGnRnE … Device-GRE), whose capitals the
/// acronym lint would lower.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(clippy::upper_case_acronyms)]
pub(crate) enum DeviceType {
    NGnRnE = 0,
    NGnRE = 1,
    NGRE = 2,
    GRE = 3,
}

impl DeviceType {
    /// Arm `DecodeDevice` of two bits.
    const fn decode(bits: u8) -> Self {
        match bits & 3 {
            0 => Self::NGnRnE,
            1 => Self::NGnRE,
            2 => Self::NGRE,
            _ => Self::GRE,
        }
    }

    /// The more restrictive of `self` and `other` (`S2CombineS1Device`).
    const fn stronger(self, other: Self) -> Self {
        if (self as u8) <= (other as u8) {
            self
        } else {
            other
        }
    }
}

/// Arm `MemType` with what each type carries.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemType {
    Device(DeviceType),
    Normal { inner: AttrHints, outer: AttrHints },
}

impl MemType {
    /// Whether this is Normal memory whose inner and outer levels are both
    /// `attrs`.
    const fn normal_both(self, attrs: Cacheability) -> bool {
        match self {
            Self::Normal { inner, outer } => {
                inner.attrs as u8 == attrs as u8 && outer.attrs as u8 == attrs as u8
            }
            Self::Device(_) => false,
        }
    }

    /// Normal Inner and Outer Write-Back memory, the one type whose `XS`
    /// is 0 without FEAT_XS.
    const fn is_write_back(self) -> bool {
        self.normal_both(Cacheability::WriteBack)
    }

    /// Normal Inner and Outer Non-cacheable memory.
    const fn is_non_cacheable(self) -> bool {
        self.normal_both(Cacheability::NonCacheable)
    }
}

/// Arm `Shareability`.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shareability {
    Non,
    Inner,
    Outer,
}

impl Shareability {
    /// Arm `DecodeShareability` of a descriptor's or `VTCR_EL2`'s two-bit
    /// `SH`: 0b01 is reserved, and taken as Outer Shareable, the CONSTRAINED
    /// UNPREDICTABLE `Unpredictable_Shareability`'s `Constraint_OSH` of the
    /// three it permits (see the README's "Where the references differ").
    pub(crate) const fn decode(sh: u8) -> Self {
        match sh & 3 {
            0b00 => Self::Non,
            0b11 => Self::Inner,
            _ => Self::Outer,
        }
    }

    /// Arm `S2CombineS1Shareability`: the wider of two.
    const fn wider(self, other: Self) -> Self {
        match (self, other) {
            (Self::Outer, _) | (_, Self::Outer) => Self::Outer,
            (Self::Inner, _) | (_, Self::Inner) => Self::Inner,
            (Self::Non, Self::Non) => Self::Non,
        }
    }
}

/// Arm `MemoryAttributes`, as far as this processor has them: the memory
/// type, its shareability, and FEAT_XS's `XS` (true where an access may be
/// slow to complete, which with no FEAT_XS is every access not to Normal
/// Write-Back memory).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemAttrs {
    pub(crate) memtype: MemType,
    pub(crate) shareability: Shareability,
    pub(crate) xs: bool,
}

/// Arm `DecodeLDFAttr`: half of a `MAIR_ELx` attribute of Normal memory.
const fn decode_ldf(half: u8) -> AttrHints {
    let attrs = if half & 0b0100 == 0 {
        Cacheability::WriteThrough
    } else if half == 0b0100 {
        return NON_CACHEABLE;
    } else {
        Cacheability::WriteBack
    };
    let hints = half & 3;
    AttrHints {
        attrs,
        hints,
        transient: hints != 0 && half & 0b1000 == 0,
    }
}

/// Arm `S1ConstrainUnpredictableRESMAIR`: whether `attr` is a reserved
/// `MAIR_ELx` encoding on a processor whose FEAT_XS is `xs` (and which has
/// no FEAT_MTE2, so the Tagged encoding 0xF0 is reserved).
const fn reserved_mair(attr: u8, xs: bool) -> bool {
    if attr & 0xF0 == 0 {
        if attr & 3 == 0b01 {
            !xs
        } else {
            attr & 3 != 0
        }
    } else if attr == 0x40 || attr == 0xA0 {
        !xs
    } else {
        attr & 0x0F == 0
    }
}

/// The `MAIR_ELx` attribute `attr` stands for on a processor implementing
/// `features`: itself, or for a reserved encoding the allocated value zero,
/// Device-nGnRnE (`Unpredictable_RESMAIR`'s UNKNOWN value as the
/// pseudocode chooses it).
const fn stage1_allocated(attr: u8, features: Features) -> u8 {
    if reserved_mair(attr, features.has(Feature::Xs)) {
        0
    } else {
        attr
    }
}

/// Whether the `MAIR_ELx` attribute `attr` is Device memory on a processor
/// implementing `features`, as [`MemAttrs::stage1`] decodes it: what an
/// access reads of a walk's attributes, without the rest of the decode.
#[inline]
pub(crate) const fn stage1_device(attr: u8, features: Features) -> bool {
    stage1_allocated(attr, features) & 0xF0 == 0
}

/// The stage 2 `MemAttr` `memattr` stands for: itself, or for the reserved
/// encodings 0b0100 (no FEAT_MTE_PERM) and 0b1x00 Normal Non-cacheable,
/// 0b0101 (`Unpredictable_S2RESMEMATTR`'s `Constraint_NC`).
const fn stage2_allocated(memattr: u8) -> u8 {
    let memattr = memattr & 0xF;
    if memattr == 0b0100 || memattr & 0b1011 == 0b1000 {
        0b0101
    } else {
        memattr
    }
}

/// Whether the stage 2 `MemAttr` `memattr` is Device memory, as
/// [`MemAttrs::stage2`] decodes it.
#[inline]
pub(crate) const fn stage2_device(memattr: u8) -> bool {
    stage2_allocated(memattr) & 0b1100 == 0
}

/// Arm `S2DecodeCacheability` of a non-zero two-bit field. Stage 2 assigns
/// no hints: the combination takes stage 1's.
const fn s2_cacheability(bits: u8) -> AttrHints {
    let attrs = match bits & 3 {
        0b01 => return NON_CACHEABLE,
        0b10 => Cacheability::WriteThrough,
        _ => Cacheability::WriteBack,
    };
    AttrHints {
        attrs,
        hints: 0,
        transient: false,
    }
}

/// Arm `S2CombineS1AttrHints`: the weaker cacheability of the two, with
/// stage 1's hints where the result is cacheable.
const fn combine_hints(s1: AttrHints, s2: AttrHints) -> AttrHints {
    let attrs = match (s1.attrs, s2.attrs) {
        (Cacheability::NonCacheable, _) | (_, Cacheability::NonCacheable) => return NON_CACHEABLE,
        (Cacheability::WriteThrough, _) | (_, Cacheability::WriteThrough) => {
            Cacheability::WriteThrough
        }
        (Cacheability::WriteBack, Cacheability::WriteBack) => Cacheability::WriteBack,
    };
    AttrHints {
        attrs,
        hints: s1.hints,
        transient: s1.transient,
    }
}

/// `PAR_EL1.ATTR`'s encoding of one level of Normal memory
/// (`EncodePARAttrs`).
const fn encode_half(level: AttrHints) -> u8 {
    match level.attrs {
        Cacheability::NonCacheable => 0b0100,
        Cacheability::WriteThrough => {
            (if level.transient { 0b00 } else { 0b10 } << 2) | level.hints
        }
        Cacheability::WriteBack => (if level.transient { 0b01 } else { 0b11 } << 2) | level.hints,
    }
}

impl MemAttrs {
    /// Device-nGnRnE, Outer Shareable: a data access's attributes with
    /// stage 1 disabled (`AArch64_S1DisabledOutput`).
    pub(crate) const DEVICE_NGNRNE: Self = Self {
        memtype: MemType::Device(DeviceType::NGnRnE),
        shareability: Shareability::Outer,
        xs: true,
    };

    /// Normal Inner and Outer Write-Back, read- and write-allocate,
    /// Non-shareable: a data access's attributes with the EL1&0 regime's
    /// stage 1 disabled by `HCR_EL2.DC` (`AArch64_S1DisabledOutput`).
    pub(crate) const NORMAL_WRITE_BACK: Self = {
        let wb = AttrHints {
            attrs: Cacheability::WriteBack,
            hints: HINT_RWA,
            transient: false,
        };
        Self {
            memtype: MemType::Normal {
                inner: wb,
                outer: wb,
            },
            shareability: Shareability::Non,
            xs: false,
        }
    };

    /// Arm `NormalNCMemAttr`, with `xs` as the attributes it replaces had
    /// it.
    pub(crate) const fn normal_non_cacheable(xs: bool) -> Self {
        Self {
            memtype: MemType::Normal {
                inner: NON_CACHEABLE,
                outer: NON_CACHEABLE,
            },
            shareability: Shareability::Outer,
            xs,
        }
    }

    /// Arm `S1DecodeMemAttrs` of the `MAIR_ELx` attribute `attr` and a
    /// descriptor's `SH`, on a processor implementing `features`, a
    /// reserved encoding taken as [`stage1_allocated`] gives it.
    pub(crate) const fn stage1(attr: u8, sh: u8, features: Features) -> Self {
        let attr = stage1_allocated(attr, features);
        let (memtype, xs) = if attr & 0xF0 == 0 {
            // `XS` is the encoding's bit 0 inverted: 0b0000dd01 is FEAT_XS's
            // Device with XS = 0.
            (
                MemType::Device(DeviceType::decode(attr >> 2)),
                attr & 1 == 0,
            )
        } else if attr == 0x40 {
            // FEAT_XS: Normal Non-cacheable with XS = 0.
            (
                MemType::Normal {
                    inner: NON_CACHEABLE,
                    outer: NON_CACHEABLE,
                },
                false,
            )
        } else if attr == 0xA0 {
            // FEAT_XS: Normal Write-Through, read-allocate, non-transient,
            // with XS = 0.
            let wt = AttrHints {
                attrs: Cacheability::WriteThrough,
                hints: HINT_RA,
                transient: false,
            };
            (
                MemType::Normal {
                    inner: wt,
                    outer: wt,
                },
                false,
            )
        } else {
            let memtype = MemType::Normal {
                inner: decode_ldf(attr & 0xF),
                outer: decode_ldf(attr >> 4),
            };
            (memtype, !memtype.is_write_back())
        };
        Self {
            memtype,
            shareability: Shareability::decode(sh),
            xs,
        }
    }

    /// Arm `S2DecodeMemAttrs` of a stage 2 descriptor's `MemAttr`
    /// (bits 5:2) and `SH`, with `XS` the inverse of its `FnXS` (bit 11,
    /// RES0 without FEAT_XS), a reserved encoding taken as
    /// [`stage2_allocated`] gives it.
    pub(crate) const fn stage2(memattr: u8, sh: u8, fnxs: bool) -> Self {
        let attr = stage2_allocated(memattr);
        let memtype = if attr & 0b1100 == 0 {
            MemType::Device(DeviceType::decode(attr))
        } else {
            MemType::Normal {
                inner: s2_cacheability(attr),
                outer: s2_cacheability(attr >> 2),
            }
        };
        Self {
            memtype,
            shareability: Shareability::decode(sh),
            xs: !fnxs,
        }
    }

    /// Whether the memory is Device memory, whose alignment rules
    /// (`AArch64_S1HasAlignmentFaultDueToMemType` and its stage 2
    /// counterpart) refuse a misaligned data access.
    #[inline]
    pub(crate) const fn is_device(&self) -> bool {
        matches!(self.memtype, MemType::Device(_))
    }

    /// `self`, Normal memory taken as Non-cacheable where `treat` holds:
    /// `AArch64_S1TreatAsNormalNC` for a data access with the regime's
    /// `SCTLR_ELx.C` clear, `AArch64_S2TreatAsNormalNC` with
    /// `HCR_EL2.CD` set. Device memory stays as it is.
    pub(crate) const fn normal_as_non_cacheable(self, treat: bool) -> Self {
        match self.memtype {
            MemType::Normal { .. } if treat => Self::normal_non_cacheable(self.xs),
            MemType::Normal { .. } | MemType::Device(_) => self,
        }
    }

    /// Arm `EffectiveShareability`: Device and Normal Inner and Outer
    /// Non-cacheable memory are Outer Shareable.
    pub(crate) const fn effective_shareability(self) -> Self {
        if self.is_device() || self.memtype.is_non_cacheable() {
            Self {
                shareability: Shareability::Outer,
                ..self
            }
        } else {
            self
        }
    }

    /// Arm `S2CombineS1MemAttrs` with `HCR_EL2.FWB` clear: the attributes of
    /// an access stage 1 gives `s1` and stage 2 `s2`. Device memory at
    /// either stage makes the result Device, of the more restrictive type
    /// where both are; Normal memory at both takes the weaker cacheability
    /// of each level, with stage 1's hints.
    pub(crate) const fn combined(s1: Self, s2: Self) -> Self {
        let memtype = match (s1.memtype, s2.memtype) {
            (MemType::Device(a), MemType::Device(b)) => MemType::Device(a.stronger(b)),
            (MemType::Device(_), MemType::Normal { .. }) => s1.memtype,
            (MemType::Normal { .. }, MemType::Device(_)) => s2.memtype,
            (
                MemType::Normal {
                    inner: i1,
                    outer: o1,
                },
                MemType::Normal {
                    inner: i2,
                    outer: o2,
                },
            ) => MemType::Normal {
                inner: combine_hints(i1, i2),
                outer: combine_hints(o1, o2),
            },
        };
        Self {
            memtype,
            shareability: s1.shareability.wider(s2.shareability),
            xs: !memtype.is_write_back() && s1.xs && s2.xs,
        }
        .effective_shareability()
    }

    /// `PAR_EL1.ATTR` (Arm `EncodePARAttrs`, with no FEAT_MTE) on a
    /// processor implementing `features`: the `MAIR_ELx` encoding of the
    /// attributes. `PAR_EL1.ATTR` takes `MAIR_ELx`'s encodings, among which
    /// those of memory with XS = 0 — Device with bit 0 set, Normal
    /// Non-cacheable 0x40 and Write-Through 0xA0 — exist only with FEAT_XS.
    /// `EncodePARAttrs` reaches them without testing the feature, as Normal
    /// memory Write-Back at stage 1 has XS = 0 whatever the processor, and
    /// keeps it when `SCTLR_ELx.C`, `HCR_EL2.CD` or stage 2 makes it
    /// Non-cacheable or Device; without FEAT_XS such memory is reported
    /// with the encoding the processor has, as if XS were 1.
    pub(crate) const fn par_attr(&self, features: Features) -> u8 {
        let xs_reported = !self.xs && features.has(Feature::Xs);
        match self.memtype {
            MemType::Device(device) => ((device as u8) << 2) | if xs_reported { 1 } else { 0 },
            MemType::Normal { inner, outer } => {
                if xs_reported {
                    let wt_ra = matches!(outer.attrs, Cacheability::WriteThrough)
                        && matches!(inner.attrs, Cacheability::WriteThrough)
                        && !outer.transient
                        && outer.hints == HINT_RA;
                    if wt_ra {
                        return 0xA0;
                    }
                    if self.memtype.is_non_cacheable() {
                        return 0x40;
                    }
                }
                (encode_half(outer) << 4) | encode_half(inner)
            }
        }
    }

    /// `PAR_EL1.SH` (Arm `PAREncodeShareability`): the effective
    /// shareability ([`MemAttrs::effective_shareability`]).
    pub(crate) const fn par_shareability(&self) -> u8 {
        match self.effective_shareability().shareability {
            Shareability::Non => 0b00,
            Shareability::Inner => 0b11,
            Shareability::Outer => 0b10,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A57: Features = Features::of(&[]);
    const XS: Features = Features::of(&[Feature::Xs]);

    /// Every allocated `MAIR_ELx` encoding without FEAT_XS is reported in
    /// `PAR_EL1` as written; every reserved one as Device-nGnRnE, the
    /// value the pseudocode's `ConstrainUnpredictableBits` gives it.
    #[test]
    fn par_reports_each_allocated_mair_encoding_as_written() {
        for attr in 0..=255u8 {
            let decoded = MemAttrs::stage1(attr, 0b11, A57);
            if reserved_mair(attr, false) {
                assert_eq!(decoded, MemAttrs::stage1(0, 0b11, A57), "{attr:#04x}");
                assert_eq!(decoded.par_attr(A57), 0, "{attr:#04x}");
            } else {
                assert_eq!(decoded.par_attr(A57), attr, "{attr:#04x}");
            }
        }
    }

    /// With FEAT_XS, its encodings are allocated and round-trip, and
    /// Write-Back memory made Non-cacheable reports XS = 0 (0x40); without
    /// it, the encoding the processor has (0x44).
    #[test]
    fn feat_xs_allocates_its_encodings() {
        for attr in [0x40u8, 0xA0, 0x01, 0x05, 0x09, 0x0D] {
            assert!(reserved_mair(attr, false), "{attr:#04x} without FEAT_XS");
            assert_eq!(MemAttrs::stage1(attr, 0, XS).par_attr(XS), attr);
        }
        assert!(!MemAttrs::stage1(0x40, 0, XS).xs);
        assert!(MemAttrs::stage1(0x44, 0, XS).xs);
        let made_non_cacheable = MemAttrs::stage1(0xFF, 0b11, XS).normal_as_non_cacheable(true);
        assert_eq!(made_non_cacheable.par_attr(XS), 0x40);
        assert_eq!(made_non_cacheable.par_attr(A57), 0x44);
    }

    /// `S2CombineS1MemAttrs`: Device at either stage wins, the more
    /// restrictive Device type of two, the weaker cacheability with stage
    /// 1's hints, and the wider shareability.
    #[test]
    fn stage_2_combines_as_arm_gives() {
        let s1_wb = MemAttrs::stage1(0xFF, 0b11, A57);
        let s1_wt = MemAttrs::stage1(0xBB, 0b11, A57);
        let s2_wb = MemAttrs::stage2(0b1111, 0b11, false);
        let s2_nc = MemAttrs::stage2(0b0101, 0b11, false);
        let s2_dev_ngre = MemAttrs::stage2(0b0010, 0b10, false);
        assert_eq!(MemAttrs::combined(s1_wb, s2_wb).par_attr(A57), 0xFF);
        assert_eq!(MemAttrs::combined(s1_wb, s2_nc).par_attr(A57), 0x44);
        assert_eq!(MemAttrs::combined(s1_wt, s2_wb).par_attr(A57), 0xBB);
        let s2_wt = MemAttrs::stage2(0b1010, 0b11, false);
        assert_eq!(
            MemAttrs::combined(s1_wb, s2_wt).par_attr(A57),
            0xBB,
            "Write-Through with stage 1's read- and write-allocate"
        );
        assert_eq!(MemAttrs::combined(s1_wb, s2_dev_ngre).par_attr(A57), 0x08);
        let s1_dev_ngnre = MemAttrs::stage1(0x04, 0b10, A57);
        assert_eq!(
            MemAttrs::combined(s1_dev_ngnre, s2_dev_ngre).par_attr(A57),
            0x04
        );
        let s1_nsh = MemAttrs::stage1(0xFF, 0b00, A57);
        assert_eq!(MemAttrs::combined(s1_nsh, s2_wb).par_shareability(), 0b11);
        assert_eq!(
            MemAttrs::combined(s1_nsh, MemAttrs::stage2(0b1111, 0b10, false)).par_shareability(),
            0b10
        );
        assert_eq!(
            MemAttrs::combined(s1_wb, s2_dev_ngre).par_shareability(),
            0b10
        );
    }

    /// What an access reads of a walk's attributes, whether they make
    /// Device memory, is what the whole decode says, for every encoding at
    /// either stage, with FEAT_XS and without.
    #[test]
    fn the_device_test_agrees_with_the_decode() {
        for features in [A57, XS] {
            for attr in 0..=255u8 {
                assert_eq!(
                    stage1_device(attr, features),
                    MemAttrs::stage1(attr, 0b11, features).is_device(),
                    "{attr:#04x}"
                );
            }
        }
        for memattr in 0..16u8 {
            assert_eq!(
                stage2_device(memattr),
                MemAttrs::stage2(memattr, 0b11, false).is_device(),
                "{memattr:#06b}"
            );
        }
    }

    /// The reserved stage 2 encodings are Normal Non-cacheable.
    #[test]
    fn reserved_stage_2_encodings_are_non_cacheable() {
        let nc = MemAttrs::stage2(0b0101, 0b11, false);
        for memattr in [0b0100u8, 0b1000, 0b1100] {
            assert_eq!(MemAttrs::stage2(memattr, 0b11, false), nc, "{memattr:#06b}");
        }
    }
}
