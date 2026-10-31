//! Stage 2 translation (VMSAv8-64, FEAT_EL2): the walk of the tables
//! `VTTBR_EL2` and `VTCR_EL2` describe, which translates the EL1&0 regime's
//! intermediate physical addresses (IPAs) while `HCR_EL2.VM` or `DC` is set,
//! with each granule the processor has at stage 2; and the permissions it
//! gives.
//!
//! Provenance (R7): the shared pseudocode of the A64 ISA XML (release
//! 2026-09): `AArch64_S2Translate` (its checks before the walk),
//! `AArch64_NSS2TTWParams`, `AArch64_S2DecodeTG0`, `AArch64_HaveS2TG`,
//! `AArch64_S2TxSZFaults`, `AArch64_S2MinTxSZ`, `AArch64_MaxTxSZ`,
//! `AArch64_S2InvalidSL`, `AArch64_S2InconsistentSL`,
//! `AArch64_S2StartLevel`, `AArch64_IPAIsOutOfRange`,
//! `AArch64_S2TTBaseAddress`, `AArch64_S2SLTTEntryAddress`,
//! `AArch64_S2Walk`, `AArch64_S2NextWalkStateTable`,
//! `AArch64_S2NextWalkStateLeaf`, `AArch64_S2LeafBase`, `Stage2OA`,
//! `AArch64_S2OAOutOfRange`, `AArch64_S2ApplyOutputPerms`,
//! `AArch64_S2DirectBasePermissions` and `AArch64_S2CheckPermissions`.
//!
//! The IMPLEMENTATION DEFINED choices are stage 1's, the pseudocode asking
//! the same questions of both stages: a `T0SZ` outside the range takes a
//! level 0 Translation fault ("Fault on TxSZ value above maximum", "... below
//! minimum"), `TG0` naming a granule this processor lacks is the 4 KiB one
//! ("TG0 encoded granule size"), the Contiguous bit is treated as 0 ("Treat
//! Contiguous bit as 0"), and a descriptor with its access flag clear faults
//! for every access, cache maintenance included ("Generate access flag fault
//! on IC/DC operations").

use core::convert::Infallible;

use super::el2::{HCR_PTW, HCR_TGE, VTCR_EL2_DS, VTCR_EL2_SL2};
use super::features::{Feature, Features};
use super::memattr::MemAttrs;
use super::mmu::{
    Ipa, LeafLevel, Level, TranslationFault, WalkStart, WalkStop, PERM_EL0_R, PERM_EL0_W,
    PERM_EL0_X, PERM_EL1_R, PERM_EL1_W, PERM_EL1_X,
};
use super::regime::{el10_stage2_enabled, sh0, Ds, Granule, OaLayout, Regime, TCR_TG0};
use super::{id, Cpu, Levels};
use crate::bus::{PhysAddr, PhysBus};

/// `VTCR_EL2.T0SZ`, bits 5:0.
const VTCR_T0SZ: u64 = 0x3F;
/// `VTCR_EL2.SL0`, bits 7:6.
const VTCR_SL0: u32 = 6;
/// `VTCR_EL2.PS`, bits 18:16.
const VTCR_PS: u32 = 16;

/// Arm `AArch64_S2MinTxSZ` without FEAT_D128, for an EL1 in AArch64 state,
/// the only one here: the smallest `T0SZ` with `granule`, under FEAT_LPA2's
/// `ds` or not, on a processor implementing `features`, `64 -` the widest
/// input address. That is `PAMax` bits, or where `PAMax` is 52 bits or
/// more, 52 with the 64 KiB granule or `ds` and 48 otherwise.
const fn s2_min_txsz(granule: Granule, ds: Ds, features: Features) -> u32 {
    let pa_max = id::pa_bits(features);
    let ia_max = if pa_max < 52 {
        pa_max
    } else if granule.holds_52_bits(ds) {
        52
    } else {
        48
    };
    64 - ia_max
}

// `AArch64_S2InvalidSL` makes a start at level 0 with the 4 KiB granule
// (`SL2:SL0` = 0b010) invalid where `PAMax` is under 44 bits, at level 1
// with the 64 KiB granule (`SL0` = 0b10) under 44, and at level 1 with the
// 16 KiB granule (0b10) under 42; on every processor here, 44 bits without
// FEAT_LPA and 52 with it, all three are valid, and [`s2_start_level`]
// tests `PAMax` only for the starts that need 52 bits.
const _: () = assert!(
    id::pa_bits(Features::of(&[])) >= 44 && id::pa_bits(Features::of(&[Feature::Lpa])) >= 44
);

/// What a stage 2 walk reads from its registers (Arm `S2TTWParams`, from
/// `AArch64_NSS2TTWParams`): what `VTCR_EL2` says of it, and `VTTBR_EL2`'s
/// base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct S2Params {
    /// Where the walk starts and how its addresses are held ([`s2_start`]).
    start: Option<S2Start>,
    vttbr: u64,
}

/// Where a stage 2 walk starts: its granule (`VTCR_EL2.TG0`,
/// [`Granule::stage2`]), the size of its input address (`64 - T0SZ`) and
/// its start level (`SL0`'s, or `SL2:SL0`'s); and how its addresses are
/// held and bounded, `PS` with the granule, FEAT_LPA2's `DS` and under it
/// `SH0` ([`OaLayout::of`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct S2Start {
    pub(crate) granule: Granule,
    pub(crate) ia_bits: u32,
    pub(crate) level: Level,
    pub(crate) oa: OaLayout,
}

impl S2Start {
    /// Whether a walk from here can end at `level` on a processor
    /// implementing `features` ([`Granule::ends_at`]).
    pub(crate) const fn ends_at(self, level: LeafLevel, features: Features) -> bool {
        let ds = self.oa.ds();
        self.granule.ends_at(self.level, level, ds, features)
    }

    /// Whether the walk translates `ipa` (Arm `AArch64_IPAIsOutOfRange`,
    /// negated): whether it fits the input address.
    pub(crate) const fn translates(self, ipa: Ipa) -> bool {
        ipa.get() >> self.ia_bits == 0
    }
}

/// Arm `AArch64_S2StartLevel` without FEAT_D128: the level a stage 2 walk
/// with `granule` under `VTCR_EL2` `vtcr` starts at, FEAT_LPA2's `DS`
/// effective or not (`ds`, [`Granule::effective_ds`]), on a processor
/// implementing `features`; `None` where `AArch64_S2InvalidSL` makes the
/// encoding invalid. With the 4 KiB granule it is `SL2:SL0`'s, `SL2`
/// counting under `DS` alone (`AArch64_NSS2TTWParams`'s `sl2`): 0b011, a
/// start at level 3, needs FEAT_TTST, and 0b100, a start at level -1, a
/// 52-bit `PAMax`; 0b101, 0b110 and 0b111 name no level. With the others
/// it is `SL0`'s: with the 16 KiB granule 0b11, a start at level 0, needs
/// `DS` and a 52-bit `PAMax`; with the 64 KiB granule 0b11 names no level.
const fn s2_start_level(granule: Granule, vtcr: u64, ds: Ds, features: Features) -> Option<Level> {
    let pa_52 = id::pa_bits(features) >= 52;
    let sl2 = ds.on() && vtcr & VTCR_EL2_SL2 != 0;
    match (granule, sl2, (vtcr >> VTCR_SL0) & 3) {
        (Granule::K4, false, 0b00) => Some(Level::Two),
        (Granule::K4, false, 0b01) => Some(Level::One),
        (Granule::K4, false, 0b10) => Some(Level::Zero),
        (Granule::K4, false, 0b11) if features.has(Feature::Ttst) => Some(Level::Three),
        (Granule::K4, true, 0b00) if pa_52 => Some(Level::Minus1),
        (Granule::K16 | Granule::K64, _, 0b00) => Some(Level::Three),
        (Granule::K16 | Granule::K64, _, 0b01) => Some(Level::Two),
        (Granule::K16 | Granule::K64, _, 0b10) => Some(Level::One),
        (Granule::K16, _, 0b11) if ds.on() && pa_52 => Some(Level::Zero),
        (Granule::K4 | Granule::K16 | Granule::K64, _, _) => None,
    }
}

/// Where a stage 2 walk under `VTCR_EL2` `vtcr` starts on a processor
/// implementing `features`, or `None` where `AArch64_S2Translate` refuses
/// the configuration before any walk, every translation a level 0
/// Translation fault: a `T0SZ` outside [`s2_min_txsz`] and
/// `AArch64_MaxTxSZ` of the granule (`AArch64_S2TxSZFaults`, with the
/// IMPLEMENTATION DEFINED "Fault on TxSZ value above maximum" and "...
/// below minimum" TRUE, as stage 1's; with FEAT_LPA the second is no
/// choice), an `SL0` or `SL2:SL0` `AArch64_S2InvalidSL` refuses
/// ([`s2_start_level`]), or one whose start level does not resolve at least
/// one bit of the input address and at most a full level's stride and four
/// more, the index into 16 concatenated tables
/// (`AArch64_S2InconsistentSL`). The one test the walk and a snapshot's TLB
/// both make (R5).
pub(crate) const fn s2_start(vtcr: u64, features: Features) -> Option<S2Start> {
    let granule = Granule::stage2(vtcr >> TCR_TG0, features);
    let ds = granule.effective_ds(vtcr & VTCR_EL2_DS != 0, features);
    let txsz = (vtcr & VTCR_T0SZ) as u32;
    if txsz < s2_min_txsz(granule, ds, features) || txsz > granule.max_txsz(features) {
        return None;
    }
    let Some(level) = s2_start_level(granule, vtcr, ds, features) else {
        return None;
    };
    let ia_bits = 64 - txsz;
    let lsb = granule.translation_size(level);
    if ia_bits <= lsb || ia_bits > lsb + granule.stride() + 4 {
        return None;
    }
    Some(S2Start {
        granule,
        ia_bits,
        level,
        oa: OaLayout::of(vtcr >> VTCR_PS, granule, ds, sh0(vtcr), features),
    })
}

/// A completed stage 2 walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct S2Walked {
    pub(crate) pa: PhysAddr,
    /// The permissions stage 2 gives, as the stage 1 permission bits they
    /// leave an access (`AArch64_S2DirectBasePermissions`): read and write
    /// at EL1 and EL0 alike, and execute as `XN` gives each.
    pub(crate) perms: u8,
    pub(crate) level: LeafLevel,
    /// The descriptor's `MemAttr` and `FnXS` (false without FEAT_XS), and
    /// the leaf's shareability — the descriptor's `SH`, or under FEAT_LPA2's
    /// `DS` `VTCR_EL2.SH0` ([`OaLayout::shareability`]) — as they are,
    /// decoded by [`S2Walked::attrs`].
    memattr: u8,
    sh: u8,
    fnxs: bool,
}

impl S2Walked {
    /// The attributes stage 2 gives (`S2DecodeMemAttrs`).
    pub(crate) const fn attrs(&self) -> MemAttrs {
        MemAttrs::stage2(self.memattr, self.sh, self.fnxs)
    }
}

/// The stage 2 permissions a descriptor gives (`AArch64_S2ApplyOutputPerms`
/// and `AArch64_S2DirectBasePermissions` without FEAT_S2PIE, FEAT_HAFDBS or
/// FEAT_XNX): `S2AP[0]` (bit 6) read, `S2AP[1]` (bit 7) write, and `XN`
/// (bit 54) execute-never at EL1 and EL0 alike.
const fn s2_permissions(desc: u64) -> u8 {
    let mut perms = 0;
    if (desc >> 6) & 1 == 1 {
        perms |= PERM_EL1_R | PERM_EL0_R;
    }
    if (desc >> 7) & 1 == 1 {
        perms |= PERM_EL1_W | PERM_EL0_W;
    }
    if (desc >> 54) & 1 == 0 {
        perms |= PERM_EL1_X | PERM_EL0_X;
    }
    perms
}

impl<L: Levels> Cpu<L> {
    /// Whether accesses translated in `regime` are translated by stage 2
    /// as well (`AArch64_S2Translate`, reached for the EL1&0 regime with
    /// `EL2Enabled()`, and `walkparams.vm`): `HCR_EL2.VM` or `DC` set. A
    /// processor without EL2 tests a constant.
    #[inline(always)]
    pub(crate) fn stage2_enabled(&self, regime: Regime) -> bool {
        regime == Regime::El10 && el10_stage2_enabled(self.hcr_el2())
    }

    /// Arm `AArch64_NSS2TTWParams`, as far as this processor has them.
    fn s2_params(&self) -> S2Params {
        let el2 = self.el2();
        S2Params {
            start: s2_start(el2.vtcr, self.imp.features),
            vttbr: el2.vttbr,
        }
    }

    /// Whether `HCR_EL2.PTW` refuses a stage 1 walk the descriptors stage
    /// 2 maps to Device memory (`walkparams.ptw`, clear under `TGE`).
    pub(crate) fn s2_ptw(&self) -> bool {
        L::EL2 && self.el2().hcr & (HCR_PTW | HCR_TGE) == HCR_PTW
    }

    /// The stage 2 translation of `ipa`, as far as the walk: the checks
    /// `AArch64_S2Translate` makes of the configuration ([`s2_start`]) and
    /// the address (`AArch64_IPAIsOutOfRange`) before it walks, all level 0
    /// Translation faults, then `AArch64_S2Walk` from the start level
    /// `SL0` or `SL2:SL0` gives, which may index up to 16 concatenated
    /// tables; every descriptor address and the output address must fit the
    /// output size `PS` gives, else an address size fault, at level 0 for
    /// the first descriptor and at the level of the descriptor holding the
    /// address for the others ([`Cpu::walk_tables`], whose descriptors'
    /// addresses are physical).
    pub(crate) fn s2_walk<B: PhysBus>(
        &self,
        bus: &mut B,
        ipa: Ipa,
    ) -> Result<S2Walked, TranslationFault> {
        let params = self.s2_params();
        let level0 = TranslationFault::Translation { level: Level::Zero };
        let Some(s2_start) = params.start else {
            return Err(level0);
        };
        if !s2_start.translates(ipa) {
            return Err(level0);
        }
        let ipa = ipa.get();
        let S2Start {
            granule,
            ia_bits,
            level,
            oa,
        } = s2_start;
        let start = WalkStart::<PhysAddr>::of_base(params.vttbr, granule, level, ia_bits, oa)?;
        let physical = &mut |_: &mut B, address: PhysAddr| Ok::<PhysAddr, Infallible>(address);
        let leaf = match self.walk_tables(bus, ipa, start, physical) {
            Ok(leaf) => leaf,
            Err(WalkStop::Fault(fault)) => return Err(fault),
            Err(WalkStop::Descriptor(never)) => match never {},
        };
        let desc = leaf.desc;
        Ok(S2Walked {
            pa: leaf.out,
            perms: s2_permissions(desc),
            level: leaf.level,
            memattr: ((desc >> 2) & 0xF) as u8,
            sh: oa.shareability(desc),
            fnxs: self.implements(Feature::Xs) && (desc >> 11) & 1 == 1,
        })
    }
}
