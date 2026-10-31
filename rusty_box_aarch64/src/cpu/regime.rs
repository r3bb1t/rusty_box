//! Translation regimes (Arm `Regime`): which stage 1 tables translate an
//! address, and the controls that govern them.
//!
//! Provenance (R7): the shared pseudocode of the A64 ISA XML (release
//! 2026-09): `TranslationRegime`, `AArch64_GetS1TTWParams`,
//! `AArch64_S1TTWParamsEL10`, `AArch64_S1TTWParamsEL2`,
//! `AArch64_S1Enabled`, `AArch64_AddrTop`, `AArch64_PhysicalAddressSize`,
//! `AArch64_BranchAddr`, `HasUnprivileged`, `AArch64_PACEffectiveTxSZ`;
//! for the granules `TGxGranuleBits`, `TranslationSize`,
//! `AArch64_S1StartLevel`, `AArch64_MaxTxSZ`, `AArch64_S1MinTxSZ`,
//! `AArch64_BlockDescSupported`, `AArch64_S1DecodeTG0`,
//! `AArch64_S1DecodeTG1`, `AArch64_S2DecodeTG0`, `AArch64_HaveS1TG` and
//! `AArch64_HaveS2TG`; for where a walk's addresses are held and how wide
//! they are, `AArch64_PAMax`, `AArch64_OAOutOfRange`,
//! `AArch64_S1TTBaseAddress`, `AArch64_S2TTBaseAddress`,
//! `AArch64_NextTableBase`, `AArch64_S1LeafBase` and `AArch64_S2LeafBase`.
//!
//! The walk parameters are gathered here, one place per regime; the walk
//! itself, the TLB and the access paths are `mmu.rs`'s.

use super::dp::ones;
use super::el2::{HCR_DC, HCR_TGE, HCR_VM, TCR_EL2_HPD, TCR_EL2_TBI, TCR_EL2_TBID};
use super::exec::sext;
use super::features::{Feature, Features};
use super::{id, sctlr, Cpu, El, Levels};

/// A stage 1 translation regime.
///
/// Closed (R5): the regimes of a processor with no EL3 and no FEAT_VHE.
/// The discriminants are the bit a TLB key holds for each (`mmu.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum Regime {
    /// EL1&0: `TTBR0_EL1` and `TTBR1_EL1`, two privilege levels.
    El10 = 0,
    /// EL2 (FEAT_EL2, `HCR_EL2.E2H` = 0): `TTBR0_EL2`, one privilege level
    /// and one virtual address range.
    El2 = 1,
}

/// Arm `AArch64_S1Enabled(Regime_EL10)` with `SCTLR_EL1` `sctlr` and
/// `HCR_EL2` `hcr` (0 where `EL2Enabled()` is FALSE): `SCTLR_EL1.M`, unless
/// `HCR_EL2.DC` or `TGE` disables stage 1 whatever it says. The one test
/// of it, which the processor ([`Cpu::s1_enabled`]) and a TLB context
/// (`mmu.rs`'s `El10Context`) both make.
#[inline(always)]
pub(crate) const fn el10_stage1_enabled(sctlr: u64, hcr: u64) -> bool {
    sctlr & sctlr::M != 0 && hcr & (HCR_DC | HCR_TGE) == 0
}

/// Whether stage 2 translates the EL1&0 regime under `HCR_EL2` `hcr`
/// (`AArch64_S2Translate`'s `walkparams.vm`): `VM` or `DC` set.
#[inline(always)]
pub(crate) const fn el10_stage2_enabled(hcr: u64) -> bool {
    hcr & (HCR_VM | HCR_DC) != 0
}

/// Whether the EL1&0 regime's data accesses with stage 1 disabled are to
/// Normal memory under `HCR_EL2` `hcr` (`AArch64_S1DisabledOutput`): `DC`
/// set.
#[inline(always)]
pub(crate) const fn el10_s1_disabled_data_normal(hcr: u64) -> bool {
    hcr & HCR_DC != 0
}

impl Regime {
    /// Arm `HasUnprivileged(regime)`: whether the regime has EL0's
    /// permissions as well as its own, and two virtual address ranges.
    #[inline(always)]
    pub(crate) const fn has_unprivileged(self) -> bool {
        matches!(self, Self::El10)
    }

    /// Whether `va` is in the upper of the regime's ranges, which bit 55
    /// selects (`AArch64_AddrTop`); a regime with one range has no upper.
    #[inline(always)]
    pub(crate) const fn upper(self, va: u64) -> bool {
        self.has_unprivileged() && (va >> 55) & 1 == 1
    }

    /// `va` with its top byte replaced by its range's extension: bit 55's
    /// copies in a regime with two ranges, ones in the upper and zeros in
    /// the lower, and zeros in a regime with one range (`AArch64_BranchAddr`
    /// under top-byte-ignore).
    #[inline(always)]
    pub(crate) const fn canonical(self, va: u64) -> u64 {
        if self.has_unprivileged() {
            sext(va, 56)
        } else {
            va & 0x00FF_FFFF_FFFF_FFFF
        }
    }
}

/// A translation granule (Arm `TGx`): the size of a page and of a table of
/// eight-byte descriptors.
///
/// Closed (R5): the granules of VMSAv8-64.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Granule {
    K4,
    K16,
    K64,
}

impl Granule {
    /// Arm `TGxGranuleBits`: the page offset's bits.
    #[inline(always)]
    pub(crate) const fn bits(self) -> u32 {
        match self {
            Self::K4 => 12,
            Self::K16 => 14,
            Self::K64 => 16,
        }
    }

    /// The input address bits a level of eight-byte descriptors resolves:
    /// the granule's bits less the descriptor size's three.
    #[inline(always)]
    pub(crate) const fn stride(self) -> u32 {
        self.bits() - 3
    }

    /// Arm `TranslationSize`: the input address bits a leaf at `level`
    /// maps directly to the output address.
    #[inline(always)]
    pub(crate) const fn translation_size(self, level: u32) -> u32 {
        self.bits() + self.stride() * (3 - level)
    }

    /// Arm `AArch64_S1StartLevel` (without FEAT_D128): the level whose
    /// table resolves the top bit of an `ia_bits`-bit input address, `3 -
    /// (ia_bits - 1 - bits) / stride`, found as the highest level whose
    /// translation size the address exceeds (level 3 for an address no
    /// wider than a level 2 leaf's).
    #[inline(always)]
    pub(crate) const fn start_level(self, ia_bits: u32) -> u32 {
        if ia_bits > self.translation_size(0) {
            0
        } else if ia_bits > self.translation_size(1) {
            1
        } else if ia_bits > self.translation_size(2) {
            2
        } else {
            3
        }
    }

    /// The level a stage 1 walk with the granule for `TxSZ` `txsz` starts
    /// at ([`Granule::start_level`]) on a processor implementing
    /// `features`, or `None` where every translation takes a level 0
    /// Translation fault: a `TxSZ` outside [`Granule::s1_min_txsz`] and
    /// [`Granule::max_txsz`] (`AArch64_S1TxSZFaults`, with the
    /// IMPLEMENTATION DEFINED "Fault on TxSZ value above maximum" and "...
    /// below minimum" TRUE, as QEMU's `param.tsz_oob`; with FEAT_LVA the
    /// second is no choice). The one test the walk and a snapshot's TLB both
    /// make (R5).
    #[inline(always)]
    pub(crate) const fn s1_start(self, txsz: u32, features: Features) -> Option<u32> {
        if txsz >= self.s1_min_txsz(features) && txsz <= self.max_txsz(features) {
            Some(self.start_level(64 - txsz))
        } else {
            None
        }
    }

    /// Whether a walk with the granule starting at `start` can end at
    /// `level` on a processor implementing `features`, at either stage: at
    /// or below the start, at a level the granule has leaves at
    /// ([`Granule::leaf_at`]).
    #[inline(always)]
    pub(crate) const fn ends_at(self, start: u32, level: u32, features: Features) -> bool {
        start <= level && self.leaf_at(level, features)
    }

    /// Arm `AArch64_S1MinTxSZ` without FEAT_LPA2's `DS`, FEAT_LVA3 or
    /// FEAT_D128: the smallest `TxSZ` at stage 1 on a processor implementing
    /// `features`, 12 (a 52-bit input address) with FEAT_LVA and the 64 KiB
    /// granule, else 16 (48 bits).
    #[inline(always)]
    pub(crate) const fn s1_min_txsz(self, features: Features) -> u32 {
        match self {
            Self::K64 if features.has(Feature::Lva) => 12,
            Self::K4 | Self::K16 | Self::K64 => 16,
        }
    }

    /// Arm `AArch64_MaxTxSZ`, at either stage: the largest `TxSZ`, the
    /// smallest input address, on a processor implementing `features`;
    /// [`MAX_TXSZ`] without FEAT_TTST.
    #[inline(always)]
    pub(crate) const fn max_txsz(self, features: Features) -> u32 {
        if !features.has(Feature::Ttst) {
            MAX_TXSZ
        } else {
            match self {
                Self::K4 | Self::K16 => 48,
                Self::K64 => 47,
            }
        }
    }

    /// Whether a block descriptor is valid at `level` on a processor
    /// implementing `features` (Arm `AArch64_BlockDescSupported` without
    /// FEAT_LPA2's `DS` or FEAT_D128): at levels 1 and 2 with the 4 KiB
    /// granule, at level 2 with the 16 KiB one, and with the 64 KiB one at
    /// level 2, and at level 1 too where `PAMax` is 52 bits (FEAT_LPA).
    #[inline(always)]
    pub(crate) const fn block_at(self, level: u32, features: Features) -> bool {
        match self {
            Self::K4 => level == 1 || level == 2,
            Self::K16 => level == 2,
            Self::K64 => level == 2 || (level == 1 && id::pa_bits(features) >= 52),
        }
    }

    /// Whether a walk with the granule can end at `level` on a processor
    /// implementing `features` (Arm `AArch64_DecodeDescriptorType`): at a
    /// page at level 3, or a block where the granule has blocks
    /// ([`Granule::block_at`]).
    #[inline(always)]
    pub(crate) const fn leaf_at(self, level: u32, features: Features) -> bool {
        level == 3 || self.block_at(level, features)
    }

    /// The granule of the EL1&0 regime's range `upper` names under
    /// `TCR_EL1` `tcr`, on a processor implementing `features`: `TG1`'s
    /// or `TG0`'s ([`Granule::decoded`]).
    #[inline(always)]
    const fn el10(tcr: u64, upper: bool, features: Features) -> Self {
        let named = if upper {
            Self::of_tg1(tcr >> TCR_EL1_TG1)
        } else {
            Self::of_tg0(tcr >> TCR_TG0)
        };
        Self::decoded(named, GranuleStage::One, features)
    }

    /// The granule of the EL2 regime under `TCR_EL2` `tcr`, on a processor
    /// implementing `features`: `TG0`'s ([`Granule::decoded`]).
    #[inline(always)]
    const fn el2(tcr: u64, features: Features) -> Self {
        Self::decoded(Self::of_tg0(tcr >> TCR_TG0), GranuleStage::One, features)
    }

    /// The granule a `TG0` field (`TCR_EL1.TG0`, `TCR_EL2.TG0`,
    /// `VTCR_EL2.TG0`) names: 0b00 4 KiB, 0b01 64 KiB, 0b10 16 KiB (Arm
    /// `AArch64_S1DecodeTG0`, `AArch64_S2DecodeTG0`); `None` for the reserved
    /// 0b11.
    #[inline(always)]
    const fn of_tg0(tg0: u64) -> Option<Self> {
        match tg0 & 3 {
            0b00 => Some(Self::K4),
            0b01 => Some(Self::K64),
            0b10 => Some(Self::K16),
            _ => None,
        }
    }

    /// The granule `TCR_EL1.TG1` names: 0b01 16 KiB, 0b10 4 KiB, 0b11 64
    /// KiB (Arm `AArch64_S1DecodeTG1`); `None` for the reserved 0b00.
    #[inline(always)]
    const fn of_tg1(tg1: u64) -> Option<Self> {
        match tg1 & 3 {
            0b01 => Some(Self::K16),
            0b10 => Some(Self::K4),
            0b11 => Some(Self::K64),
            _ => None,
        }
    }

    /// Whether a processor implementing `features` has the granule at
    /// `stage` (Arm `AArch64_HaveS1TG`; `AArch64_HaveS2TG` with FEAT_GTG,
    /// which every processor here with EL2 has). Every processor here has
    /// the 4 KiB one at both.
    #[inline(always)]
    const fn at(self, stage: GranuleStage, features: Features) -> bool {
        let (k16, k64) = match stage {
            GranuleStage::One => (Feature::TGran16K, Feature::TGran64K),
            GranuleStage::Two => (Feature::S2TGran16K, Feature::S2TGran64K),
        };
        match self {
            Self::K4 => true,
            Self::K16 => features.has(k16),
            Self::K64 => features.has(k64),
        }
    }

    /// The granule an encoding names at `stage` on a processor implementing
    /// `features` (`AArch64_S1DecodeTG0`, `AArch64_S1DecodeTG1`,
    /// `AArch64_S2DecodeTG0`): a reserved encoding, or one naming a granule
    /// the processor lacks there, names the IMPLEMENTATION DEFINED "TG0
    /// encoded granule size" or "TG1 encoded granule size", here the 4 KiB
    /// granule at either stage, the smallest, as QEMU's
    /// (`sanitize_gran_size`).
    #[inline(always)]
    const fn decoded(named: Option<Self>, stage: GranuleStage, features: Features) -> Self {
        match named {
            Some(granule) if granule.at(stage, features) => granule,
            Some(_) | None => Self::K4,
        }
    }

    /// The stage 2 granule `VTCR_EL2.TG0` `tg0` names on a processor
    /// implementing `features` ([`Granule::decoded`]).
    #[inline(always)]
    pub(crate) const fn stage2(tg0: u64, features: Features) -> Self {
        Self::decoded(Self::of_tg0(tg0), GranuleStage::Two, features)
    }
}

/// A stage of translation, as it decides which granules a processor has.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GranuleStage {
    One,
    Two,
}

/// Arm `AArch64_MaxTxSZ` without FEAT_TTST, whatever the granule: 39, a
/// 25-bit input address. With FEAT_TTST every granule's is larger.
const MAX_TXSZ: u32 = 39;

/// `TCR_EL1` fields of the lower range (`TTBR0_EL1`'s); each upper-range
/// field sits at the offset its constant names from it.
const TCR_EL1_T0SZ: u32 = 0;
const TCR_EL1_EPD0: u32 = 7;
/// `TCR_EL1.TG1` (31:30), whose encodings differ from `TG0`'s
/// ([`TCR_TG0`]).
const TCR_EL1_TG1: u32 = 30;
const TCR_EL1_IPS: u32 = 32;
const TCR_EL1_TBI0: u32 = 37;
const TCR_EL1_HPD0: u32 = 41;
const TCR_EL1_TBID0: u32 = 51;
/// How far above `T0SZ` and `EPD0` their upper-range fields `T1SZ` and
/// `EPD1` are; `TBI1`, `HPD1` and `TBID1` are each the bit above their
/// lower-range field.
const TCR_EL1_UPPER_SZ_EPD: u32 = 16;
/// `TCR_EL2.PS`.
const TCR_EL2_PS: u32 = 16;
/// `TG0` (15:14) of `TCR_EL1`, `TCR_EL2` and `VTCR_EL2` alike.
pub(crate) const TCR_TG0: u32 = 14;

/// Whether top-byte-ignore applies to the instruction or data addresses of
/// the EL1&0 regime's range `upper` names under `TCR_EL1` `tcr`
/// ([`Cpu::regime_tbi`]): `TBI1` or `TBI0`, and with FEAT_PAuth (`pauth`)
/// not for an instruction address the matching `TBID` takes out of it.
#[inline(always)]
const fn el10_tbi(tcr: u64, upper: bool, instruction: bool, pauth: bool) -> bool {
    // Each upper-range bit chosen whole: a branch's tag check is then a
    // carry into one bit test, where an added `upper` builds a mask first.
    let (tbi_at, tbid_at) = if upper {
        (TCR_EL1_TBI0 + 1, TCR_EL1_TBID0 + 1)
    } else {
        (TCR_EL1_TBI0, TCR_EL1_TBID0)
    };
    let tbi = (tcr >> tbi_at) & 1 == 1;
    let tbid = (tcr >> tbid_at) & 1 == 1;
    tbi && !(instruction && tbid && pauth)
}

/// Whether top-byte-ignore applies to the instruction or data addresses of
/// the EL2 regime under `TCR_EL2` `tcr` ([`Cpu::regime_tbi`]): `TBI`, and
/// with FEAT_PAuth (`pauth`) not for an instruction address `TBID` takes
/// out of it.
#[inline(always)]
const fn el2_tbi(tcr: u64, instruction: bool, pauth: bool) -> bool {
    tcr & TCR_EL2_TBI != 0 && !(instruction && tcr & TCR_EL2_TBID != 0 && pauth)
}

/// Whether the EL1&0 regime's walks of the range `upper` names are disabled
/// under `TCR_EL1` `tcr` (`EPD1` or `EPD0`).
#[inline(always)]
const fn el10_epd(tcr: u64, upper: bool) -> bool {
    let epd_bit = TCR_EL1_EPD0 + if upper { TCR_EL1_UPPER_SZ_EPD } else { 0 };
    (tcr >> epd_bit) & 1 == 1
}

/// Whether `va` is in the range of its regime its bit 55 selects (`upper`,
/// [`Regime::upper`]) for an `ia_bits`-bit input address, top-byte-ignore
/// (`tbi`) leaving its top byte out (Arm `AArch64_VAIsOutOfRange`, negated):
/// its bits from `ia_bits` up all ones in the upper range and zeros in the
/// lower.
#[inline(always)]
const fn va_in_range(va: u64, upper: bool, ia_bits: u32, tbi: bool) -> bool {
    let top = if tbi { 56 } else { 64 };
    let check = ones(top - ia_bits) << ia_bits;
    va & check == if upper { check } else { 0 }
}

/// `TxSZ` of the EL1&0 regime's range `upper` names under `TCR_EL1` `tcr`
/// (`T1SZ` or `T0SZ`), as the register holds it.
#[inline(always)]
const fn el10_txsz(tcr: u64, upper: bool) -> u32 {
    let shift = TCR_EL1_T0SZ + if upper { TCR_EL1_UPPER_SZ_EPD } else { 0 };
    ((tcr >> shift) & 0x3F) as u32
}

/// `TxSZ` of the EL2 regime under `TCR_EL2` `tcr` (`T0SZ`).
#[inline(always)]
const fn el2_txsz(tcr: u64) -> u32 {
    (tcr & 0x3F) as u32
}

/// What decides which addresses the stage 1 walks of one range of a
/// regime translate and where they can end (Arm `S1TTWParams`' `tgx`,
/// `txsz`, `epd`, `tbi` and `tbid`): the one description the walk
/// ([`S1Params::range`]) and a snapshot's TLB entries both read (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct S1Range {
    /// The upper virtual address range (`TTBR1_EL1`), only in EL1&0.
    pub(crate) upper: bool,
    /// The range's granule, from `TG0` or `TG1` ([`Granule::el10`],
    /// [`Granule::el2`]).
    pub(crate) granule: Granule,
    /// `TxSZ` as its register holds it, which the walk faults outside
    /// [`Granule::s1_min_txsz`]`..=`[`Granule::max_txsz`]
    /// ([`Granule::s1_start`]).
    pub(crate) txsz: u32,
    /// The range's walks are disabled (`TCR_EL1.EPD0`/`EPD1`; never in EL2).
    epd: bool,
    /// Top-byte-ignore of data addresses, and of instruction addresses.
    tbi_data: bool,
    tbi_instruction: bool,
}

impl S1Range {
    /// The range of the EL1&0 regime `upper` names under `TCR_EL1` `tcr` on
    /// a processor implementing `features`.
    #[inline(always)]
    pub(crate) const fn el10(tcr: u64, upper: bool, features: Features) -> Self {
        let pauth = features.has(Feature::PAuth);
        Self {
            upper,
            granule: Granule::el10(tcr, upper, features),
            txsz: el10_txsz(tcr, upper),
            epd: el10_epd(tcr, upper),
            tbi_data: el10_tbi(tcr, upper, false, pauth),
            tbi_instruction: el10_tbi(tcr, upper, true, pauth),
        }
    }

    /// The one range of the EL2 regime under `TCR_EL2` `tcr` on a processor
    /// implementing `features`.
    #[inline(always)]
    pub(crate) const fn el2(tcr: u64, features: Features) -> Self {
        let pauth = features.has(Feature::PAuth);
        Self {
            upper: false,
            granule: Granule::el2(tcr, features),
            txsz: el2_txsz(tcr),
            epd: false,
            tbi_data: el2_tbi(tcr, false, pauth),
            tbi_instruction: el2_tbi(tcr, true, pauth),
        }
    }

    /// Whether top-byte-ignore applies to an instruction address or a data
    /// address of the range ([`Cpu::regime_tbi`]).
    #[inline(always)]
    pub(crate) const fn tbi(self, instruction: bool) -> bool {
        if instruction {
            self.tbi_instruction
        } else {
            self.tbi_data
        }
    }

    /// The level a walk of `va` starts at, under top-byte-ignore `tbi`, on a
    /// processor implementing `features` ([`Granule::s1_start`]); `None`
    /// where `AArch64_S1Translate` takes a level 0 Translation fault ahead
    /// of the walk: for a `TxSZ` out of range, for `va` outside the range
    /// ([`va_in_range`]), or for the range's walks disabled (`EPD`) — the one
    /// fault whichever of them it is.
    #[inline(always)]
    pub(crate) const fn start_for(self, va: u64, tbi: bool, features: Features) -> Option<u32> {
        match self.granule.s1_start(self.txsz, features) {
            Some(start) if !self.epd && va_in_range(va, self.upper, 64 - self.txsz, tbi) => {
                Some(start)
            }
            Some(_) | None => None,
        }
    }

    /// Whether a walk translates `va` with a leaf at `level` on a processor
    /// implementing `features` ([`S1Range::start_for`],
    /// [`Granule::ends_at`]), under the top-byte-ignore of instruction
    /// addresses, which every tagged address the TLB keeps a translation of
    /// has ([`Cpu::cache`]): what a snapshot's TLB entry the walk made must
    /// meet.
    pub(crate) const fn reaches(self, va: u64, features: Features, level: u32) -> bool {
        match self.start_for(va, self.tbi_instruction, features) {
            Some(start) => self.granule.ends_at(start, level, features),
            None => false,
        }
    }
}

/// `PS`'s encoding of 52 bits, with which FEAT_LPA's table base registers
/// hold a 52-bit base for the 64 KiB granule.
const PS_52: u64 = 0b110;

/// Where a table base register holds bits 51:48 of the start table's
/// address (Arm `AArch64_S1TTBaseAddress`, `AArch64_S2TTBaseAddress`).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseHigh {
    /// Nowhere: `BADDR` is bits 47:1.
    None,
    /// Bits 5:2, `BADDR` bits 47:6 and the table at least 64-byte aligned:
    /// with FEAT_LPA, the 64 KiB granule and `PS` 0b110 (the IMPLEMENTATION
    /// DEFINED "BADDR expresses 52 bits for 64KB granule" FALSE without).
    Bits5To2,
}

/// Where a table or leaf descriptor holds bits 51:48 of the address it
/// gives (Arm `AArch64_NextTableBase`, `AArch64_S1LeafBase`,
/// `AArch64_S2LeafBase`).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DescriptorHigh {
    /// Nowhere: the address is bits 47:0, those below the granule or the
    /// leaf's size cleared.
    None,
    /// Bits 15:12, below every address the 64 KiB granule gives: with that
    /// granule where `PAMax` is 52 bits (the IMPLEMENTATION DEFINED
    /// "descriptor[15:12] for 64KB granule are OA[51:48]" FALSE where it is
    /// less).
    Bits15To12,
}

/// How a walk's table base register and descriptors hold its addresses,
/// and the size they must fit: what `S1TTWParams` and `S2TTWParams`' `ps`
/// mean with the walk's granule on its processor. The one description
/// both stages' walks read (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OaLayout {
    /// The output address size (Arm `AArch64_PhysicalAddressSize` without
    /// FEAT_LPA2's `DS` or FEAT_D128): `PS`'s, at most 52 bits with FEAT_LPA
    /// and the 64 KiB granule and at most 48 otherwise, and at most
    /// `PAMax`. Every table address and output address must fit it
    /// ([`OaLayout::fits`]).
    bits: u32,
    base: BaseHigh,
    descriptor: DescriptorHigh,
}

impl OaLayout {
    /// The layout of a walk with `granule` under the encoded `PS` or `IPS`
    /// `ps` on a processor implementing `features`.
    #[inline(always)]
    pub(crate) const fn of(ps: u64, granule: Granule, features: Features) -> Self {
        let pa_max = id::pa_bits(features);
        let k64 = matches!(granule, Granule::K64);
        let lpa = features.has(Feature::Lpa);
        let format_max = if lpa && k64 { 52 } else { 48 };
        let max = if pa_max < format_max {
            pa_max
        } else {
            format_max
        };
        let size = id::ADDRESS_SIZES[(ps & 7) as usize];
        Self {
            bits: if size < max { size } else { max },
            base: if lpa && k64 && ps & 7 == PS_52 {
                BaseHigh::Bits5To2
            } else {
                BaseHigh::None
            },
            descriptor: if k64 && pa_max >= 52 {
                DescriptorHigh::Bits15To12
            } else {
                DescriptorHigh::None
            },
        }
    }

    /// Whether `address` fits the output address size (Arm
    /// `AArch64_OAOutOfRange`, negated).
    #[inline(always)]
    pub(crate) const fn fits(self, address: u64) -> bool {
        address >> self.bits == 0
    }

    /// The start table's address the table base register `ttbr` holds,
    /// aligned down to `2^align_bits` bytes, the start table's size: `BADDR`
    /// with bits 51:48 from where [`BaseHigh`] says. Bit 0 is `CnP`
    /// (FEAT_TTCNP), which has no effect on one processor.
    #[inline(always)]
    pub(crate) const fn table_base(self, ttbr: u64, align_bits: u32) -> u64 {
        match self.base {
            BaseHigh::None => (ttbr & TTB_BADDR) & !ones(align_bits),
            BaseHigh::Bits5To2 => {
                let align_bits = if align_bits < 6 { 6 } else { align_bits };
                let base = (ttbr & TTB_BADDR) | (((ttbr >> 2) & 0xF) << 48);
                base & !ones(align_bits)
            }
        }
    }

    /// The address a table or leaf descriptor `desc` gives, its bits below
    /// `low_bits` (the granule's for a table, the leaf's size for a leaf)
    /// not part of it: bits 47:`low_bits`, and bits 51:48 from where
    /// [`DescriptorHigh`] says.
    #[inline(always)]
    pub(crate) const fn descriptor_address(self, desc: u64, low_bits: u32) -> u64 {
        let low = desc & DESC_OA & !ones(low_bits);
        match self.descriptor {
            DescriptorHigh::None => low,
            DescriptorHigh::Bits15To12 => low | (((desc >> 12) & 0xF) << 48),
        }
    }
}

/// The output address bits 47:0 a table or leaf descriptor holds, of which
/// those below the granule or the leaf's size are not the address.
const DESC_OA: u64 = 0x0000_FFFF_FFFF_FFFF;

/// A translation table base register's `BADDR`, bits 47:1, whose bits 5:2
/// FEAT_LPA's 52-bit base makes bits 51:48 ([`OaLayout::table_base`]); its
/// bit 0 is `CnP` (FEAT_TTCNP), which has no effect on one processor.
pub(crate) const TTB_BADDR: u64 = 0x0000_FFFF_FFFF_FFFE;

/// What a stage 1 walk of one regime reads from its registers (Arm
/// `S1TTWParams`), for the virtual address range of the address walked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct S1Params {
    pub(crate) regime: Regime,
    /// Which addresses the range's walks translate and where they can end.
    pub(crate) range: S1Range,
    /// The range's table base register.
    pub(crate) ttbr: u64,
    /// Hierarchical permissions disabled (FEAT_HPDS).
    pub(crate) hpd: bool,
    /// How the walk's addresses are held, and the output address size, from
    /// `IPS` or `PS` with the range's granule.
    pub(crate) oa: OaLayout,
    pub(crate) mair: u64,
    pub(crate) wxn: bool,
}

impl<L: Levels> Cpu<L> {
    /// Arm `TranslationRegime(PSTATE.EL)`: the regime the current level's
    /// accesses are translated in. Without EL2 it is always EL1&0, a
    /// constant every regime test folds into.
    #[inline(always)]
    pub(crate) fn regime(&self) -> Regime {
        if L::EL2 && self.pstate.el == El::El2 {
            Regime::El2
        } else {
            Regime::El10
        }
    }

    /// Arm `SCTLR_ELx()` for `regime`: the system control register of the
    /// level that owns it.
    #[inline(always)]
    pub(crate) fn regime_sctlr(&self, regime: Regime) -> u64 {
        match regime {
            Regime::El10 => self.sys.sctlr_el1,
            Regime::El2 => self.el2().sctlr,
        }
    }

    /// Arm `AArch64_S1Enabled(regime)`: `SCTLR_ELx.M`, which for EL1&0
    /// `HCR_EL2.DC` and `HCR_EL2.TGE` override ([`el10_stage1_enabled`]).
    #[inline(always)]
    pub(crate) fn s1_enabled(&self, regime: Regime) -> bool {
        match regime {
            Regime::El10 => el10_stage1_enabled(self.sys.sctlr_el1, self.hcr_el2()),
            Regime::El2 => self.el2().sctlr & sctlr::M != 0,
        }
    }

    /// Whether data accesses of `regime` with stage 1 disabled are to
    /// Normal memory rather than Device-nGnRnE (Arm
    /// `AArch64_S1DisabledOutput`): in EL1&0 under `HCR_EL2.DC`
    /// ([`el10_s1_disabled_data_normal`]).
    #[inline(always)]
    pub(crate) fn s1_disabled_data_normal(&self, regime: Regime) -> bool {
        regime == Regime::El10 && el10_s1_disabled_data_normal(self.hcr_el2())
    }

    /// Whether top-byte-ignore applies to the instruction or data addresses
    /// of the range of `regime` that `upper` names (Arm `AArch64_AddrTop`
    /// with the regime's `TBI` and `TBID`): EL1&0's `TCR_EL1.TBI1` or
    /// `TBI0`, EL2's `TCR_EL2.TBI`, and with FEAT_PAuth not for an
    /// instruction address the matching `TBID` takes out of it.
    #[inline]
    pub(crate) fn regime_tbi(&self, regime: Regime, upper: bool, instruction: bool) -> bool {
        let pauth = self.implements(Feature::PAuth);
        match regime {
            Regime::El10 => el10_tbi(self.sys.tcr_el1, upper, instruction, pauth),
            Regime::El2 => el2_tbi(self.el2().tcr, instruction, pauth),
        }
    }

    /// The range of `regime` that `upper` names as its registers describe
    /// it: EL1&0's from `TCR_EL1` ([`S1Range::el10`]), EL2's from
    /// `TCR_EL2` ([`S1Range::el2`]).
    #[inline]
    fn s1_range(&self, regime: Regime, upper: bool) -> S1Range {
        let features = self.imp.features;
        match regime {
            Regime::El10 => S1Range::el10(self.sys.tcr_el1, upper, features),
            Regime::El2 => S1Range::el2(self.el2().tcr, features),
        }
    }

    /// Arm `AArch64_PACEffectiveTxSZ`: the `TxSZ` of the range of `regime`
    /// that `upper` names held within [`Granule::s1_min_txsz`]`..=`
    /// [`Granule::max_txsz`] of the range's granule (`AArch64_S1MinTxSZ`,
    /// `AArch64_MaxTxSZ`), a value outside them counting as the nearest, as
    /// it sets the bottom bit of a pointer authentication code: for the
    /// walk's faulting values the CONSTRAINED UNPREDICTABLE
    /// `Unpredictable_RESTnSZ` taken as `Constraint_FORCE`, the pseudocode's
    /// own `ConstrainUnpredictable` choice ("Map to the limit value"). The
    /// walk does not hold it so ([`S1Range::txsz`]).
    #[inline]
    pub(crate) fn pac_effective_txsz(&self, regime: Regime, upper: bool) -> u32 {
        let features = self.imp.features;
        let range = self.s1_range(regime, upper);
        let min = range.granule.s1_min_txsz(features);
        let max = range.granule.max_txsz(features);
        range.txsz.clamp(min, max)
    }

    /// Arm `AArch64_GetS1TTWParams(regime, ..., va)`: the walk parameters
    /// of the range of `regime` that `va`'s bit 55 selects (EL2's one range
    /// whatever the bit).
    pub(crate) fn s1_params(&self, regime: Regime, va: u64) -> S1Params {
        let hpds = self.implements(Feature::Hpds);
        match regime {
            Regime::El10 => {
                let tcr = self.sys.tcr_el1;
                let upper = regime.upper(va);
                let ttbr = if upper {
                    self.sys.ttbr1_el1
                } else {
                    self.sys.ttbr0_el1
                };
                let up = u32::from(upper);
                let range = self.s1_range(regime, upper);
                S1Params {
                    regime,
                    range,
                    ttbr,
                    hpd: hpds && (tcr >> (TCR_EL1_HPD0 + up)) & 1 == 1,
                    oa: OaLayout::of(tcr >> TCR_EL1_IPS, range.granule, self.imp.features),
                    mair: self.sys.mair_el1,
                    wxn: self.sys.sctlr_el1 & sctlr::WXN != 0,
                }
            }
            Regime::El2 => {
                let el2 = self.el2();
                let range = self.s1_range(regime, false);
                S1Params {
                    regime,
                    range,
                    ttbr: el2.ttbr0,
                    hpd: hpds && el2.tcr & TCR_EL2_HPD != 0,
                    oa: OaLayout::of(el2.tcr >> TCR_EL2_PS, range.granule, self.imp.features),
                    mair: el2.mair,
                    wxn: el2.sctlr & sctlr::WXN != 0,
                }
            }
        }
    }

    /// Arm `AArch64_BranchAddr(target, PSTATE.EL)`: the address a branch,
    /// an exception entry or an exception return writes to the PC. Under
    /// top-byte-ignore of instruction addresses the tag is dropped: bits
    /// 63:56 become copies of bit 55 at EL0 and EL1, whose regime has two
    /// ranges, and zeros at EL2, whose regime has one; a tag TBID keeps from
    /// being ignored stays, for the fetch to fault on.
    #[inline]
    pub(crate) fn branch_addr(&self, target: u64) -> u64 {
        let regime = self.regime();
        if self.regime_tbi(regime, regime.upper(target), true) {
            regime.canonical(target)
        } else {
            target
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `AArch64_PhysicalAddressSize`: the encoded size, at most `PAMax`, 44
    /// bits without FEAT_LPA and 52 with it, and at most 48 bits but with
    /// FEAT_LPA and the 64 KiB granule.
    #[test]
    fn output_sizes_stop_at_the_physical_address_size() {
        let a57 = Features::of(&[]);
        let lpa = Features::of(&[Feature::Lpa]);
        let bits = |ps, granule, features| OaLayout::of(ps, granule, features).bits;
        for granule in [Granule::K4, Granule::K16, Granule::K64] {
            assert_eq!(bits(0b000, granule, a57), 32);
            assert_eq!(bits(0b011, granule, a57), 42);
            assert_eq!(bits(0b100, granule, a57), 44);
            assert_eq!(bits(0b101, granule, a57), 44, "48 bits, beyond PAMax");
            assert_eq!(bits(0b111, granule, a57), 44, "56 bits");
            assert_eq!(bits(0b101, granule, lpa), 48);
        }
        assert_eq!(bits(0b110, Granule::K4, lpa), 48, "52 bits, beyond 4 KiB's");
        assert_eq!(bits(0b110, Granule::K16, lpa), 48, "beyond 16 KiB's");
        assert_eq!(bits(0b110, Granule::K64, lpa), 52);
        assert_eq!(bits(0b111, Granule::K64, lpa), 52, "56 bits, beyond PAMax");
    }
}
