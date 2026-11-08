//! TLB maintenance (Arm ARM D8.17): what each `TLBI` instruction names, as
//! its encoding and its operand `Xt` give it (Arm's `TLBI` pages,
//! `AArch64_TLBI_VMALL`, `AArch64_TLBI_ASID`, `AArch64_TLBI_VA`,
//! `AArch64_TLBI_VAA`, `AArch64_TLBI_RVA`, `AArch64_TLBI_RVAA`,
//! `AArch64_TLBI_VMALLS12`, `AArch64_TLBI_ALL`, `AArch64_TLBI_IPAS2`,
//! `AArch64_TLBI_RIPAS2`, `TLBIRange`, `ResTLBITTL`, `ResTLBIRTTL`), and
//! which cached translations it removes (`TLBIMatch`): the TLB
//! (`mmu.rs`) describes each of its entries as a [`TlbRecord`] and removes
//! the ones the [`Invalidation`] matches.
//!
//! The board has one processor, so the Inner and Outer Shareable forms
//! (`IS`, `OS`) invalidate what the non-shareable ones do
//! (`BroadcastTLBI` reaches no other TLB). The TLB caches leaves alone —
//! no table descriptor of a stage 1 walk, and of stage 2 only its leaves'
//! translations of IPAs — so the Last-level forms (`VALE1`, `IPAS2LE1`,
//! …), which `TLBIMatch` lets leave table entries (`istable`), invalidate
//! what the others do. The `nXS` forms (FEAT_XS) invalidate what the
//! others do as well, entries of memory with the XS attribute included,
//! which `TLBIMatch` lets them leave: an over-invalidation the
//! architecture permits.

use super::el2::{TCR_EL2_DS, VTCR_EL2_DS, VTTBR_VMID8};
use super::exec::sext;
use super::features::Feature;
use super::mmu::LeafLevel;
use super::regime::{Granule, Regime};
use super::{Cpu, Levels};

/// `TCR_EL1.A1`: the ASID of the EL1&0 regime's translations is
/// `TTBR1_EL1`'s, not `TTBR0_EL1`'s.
const TCR_A1: u64 = 1 << 22;
/// `TCR_EL1.AS`: ASIDs are 16 bits; clear, their top eight bits are zero
/// (`AArch64_TLBContextEL10`).
const TCR_AS: u64 = 1 << 36;
/// `TCR_EL1.DS` (FEAT_LPA2), which with `TCR_EL2.DS` and `VTCR_EL2.DS`
/// makes a range's base address FEAT_LPA2's form (`HasLargeAddress`).
const TCR_EL1_DS: u64 = 1 << 59;
/// The input address bits a TLBI by VA compares: 55:0 (`TLBIMatch`).
const VA_BITS: u32 = 56;

/// A virtual machine identifier (Arm `VMID()`): `VTTBR_EL2.VMID`'s low
/// eight bits, the processors here having no FEAT_VMID16.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Vmid(u8);

impl Vmid {
    /// The VMID `VTTBR_EL2` `vttbr` gives.
    pub(crate) const fn of(vttbr: u64) -> Self {
        Self(((vttbr & VTTBR_VMID8) >> 48) as u8)
    }
}

/// An address space identifier (Arm `TLBContext.asid`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Asid(u16);

impl Asid {
    /// The ASID the EL1&0 regime's translations carry under `TCR_EL1`
    /// `tcr`, `TTBR0_EL1` `ttbr0` and `TTBR1_EL1` `ttbr1`
    /// (`AArch64_TLBContextEL10` without FEAT_ASID2): `TTBR1_EL1`'s under
    /// `A1`, else `TTBR0_EL1`'s, its top eight bits zero without `AS`.
    pub(crate) const fn el10(tcr: u64, ttbr0: u64, ttbr1: u64) -> Self {
        let ttbr = if tcr & TCR_A1 != 0 { ttbr1 } else { ttbr0 };
        let asid = (ttbr >> 48) as u16;
        Self(if tcr & TCR_AS != 0 { asid } else { asid & 0xFF })
    }

    /// The ASID a `TLBI` names: `Xt[63:48]`.
    const fn named(xt: u64) -> Self {
        Self((xt >> 48) as u16)
    }
}

/// Arm's `nG` of a stage 1 translation: whether it serves every ASID, or
/// only that of the context it was made in.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TranslationScope {
    /// `nG` clear, and every translation of a regime without ASIDs.
    Global,
    /// `nG` set in the EL1&0 regime.
    NonGlobal,
}

/// Which stages' information a cached translation holds, as `TLBIMatch`
/// reads `includes_s1` and `includes_s2`.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Holds {
    /// Stage 1's, alone or combined with stage 2's.
    Stage1,
    /// Stage 2's alone: a translation of an IPA.
    Stage2,
}

/// A cached translation as a TLB maintenance instruction matches it (Arm
/// `TLBRecord` as `TLBIMatch` reads it): its regime and, in the EL1&0
/// regime, its VMID where the processor has EL2 (`use_vmid`) and the ASID
/// of a translation holding stage 1's information (`UseASID`); its `nG`;
/// the stages it holds; its input address — a VA, or an IPA for a
/// translation of stage 2 alone; and the granule and level of the leaf
/// whose information it holds, stage 1's where it holds stage 1's, which
/// give its block size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TlbRecord {
    pub(crate) regime: Regime,
    pub(crate) vmid: Option<Vmid>,
    pub(crate) asid: Option<Asid>,
    pub(crate) scope: TranslationScope,
    pub(crate) holds: Holds,
    pub(crate) input: u64,
    pub(crate) granule: Granule,
    pub(crate) level: LeafLevel,
}

impl TlbRecord {
    /// Arm `TLBRecord.blocksize`: the input address bits the leaf maps
    /// directly, its block or page.
    const fn block_bits(&self) -> u32 {
        self.granule.translation_size(self.level.level())
    }
}

/// A TLBI's `TTL` hint (FEAT_TTL): the granule and level of the leaf that
/// translates the address it names (`DecodeTLBITG(ttl[3:2])`,
/// `ttl[1:0]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LevelHint {
    granule: Granule,
    level: LeafLevel,
}

/// A range a FEAT_TLBIRANGE TLBI names (`TLBIRange`): its granule (`TG`),
/// the addresses from `start` up to `end`, and the level its `TTL` names
/// where it names one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AddressRange {
    granule: Granule,
    start: u64,
    end: u64,
    level: Option<LeafLevel>,
}

/// The addresses a TLBI by address names.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Addresses {
    /// One address, with its `TTL` hint where it gives a valid one.
    At {
        address: u64,
        hint: Option<LevelHint>,
    },
    /// A range of them.
    Range(AddressRange),
}

impl Addresses {
    /// Whether the input address of `record`, as far as its bits below
    /// `top`, is one of these (`TLBIMatch`): with one address, the
    /// address's bits from the record's block size up equal the record's;
    /// with a range, the record's block lies partly in it. A hint or a
    /// range naming a granule or level the record's leaf is not at leaves
    /// it.
    fn match_input(&self, record: &TlbRecord, top: u32) -> bool {
        let block = (1u64 << record.block_bits()) - 1;
        let below_top = (1u64 << top) - 1;
        match *self {
            Self::At { address, hint } => {
                let hinted = hint.is_none_or(|hint| {
                    hint.granule == record.granule && hint.level == record.level
                });
                hinted && (address ^ record.input) & below_top & !block == 0
            }
            Self::Range(range) => {
                let leveled = range.level.is_none_or(|level| level == record.level);
                let start = record.input & !block & below_top;
                let end = (record.input | block) & below_top;
                range.granule == record.granule
                    && leveled
                    && range.start & below_top <= end
                    && range.end & below_top > start
            }
        }
    }
}

/// Whether a TLBI by VA reaches every ASID's translations, or one ASID's
/// and the global ones.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Asids {
    /// `VAE1`, `RVAE1` and their siblings: the ASID `Xt` names.
    Named(Asid),
    /// `VAAE1`, `RVAAE1` and their siblings.
    All,
}

/// What one TLBI invalidates: the `TLBIRecord` its operation makes, as
/// `TLBIMatch` reads it. Every operation of the EL1&0 regime carries the
/// current VMID where the processor has EL2 (`UseVMID`).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Invalidation {
    /// `VMALLE1` (`TLBIOp_VMALL`): every translation of the EL1&0 regime
    /// holding stage 1's information, of the VMID.
    El10Stage1 { vmid: Option<Vmid> },
    /// `ASIDE1` (`TLBIOp_ASID`): the non-global ones of the ASID.
    El10Asid { vmid: Option<Vmid>, asid: Asid },
    /// `VAE1`, `VAAE1`, `RVAE1`, `RVAAE1` and their Last-level forms
    /// (`TLBIOp_VA`, `TLBIOp_VAA`, `TLBIOp_RVA`, `TLBIOp_RVAA`): those of
    /// the addresses, of the ASIDs.
    El10Va {
        vmid: Option<Vmid>,
        asids: Asids,
        addresses: Addresses,
    },
    /// `VMALLS12E1` (`TLBIOp_VMALLS12`): every translation of the EL1&0
    /// regime of the VMID, of stage 2 alone too.
    El10Stage12 { vmid: Option<Vmid> },
    /// `ALLE1` (`TLBIOp_ALL`): every translation of the EL1&0 regime.
    El10Every,
    /// `IPAS2E1`, `RIPAS2E1` and their Last-level forms (`TLBIOp_IPAS2`,
    /// `TLBIOp_RIPAS2`): the translations of stage 2 alone of the IPAs, of
    /// the VMID; not those combining both stages.
    Ipa {
        vmid: Option<Vmid>,
        addresses: Addresses,
    },
    /// `ALLE2` (`TLBIOp_ALL`): every translation of the EL2 regime.
    El2Every,
    /// `VAE2`, `RVAE2` and their Last-level forms: the EL2 regime's of the
    /// addresses, which carry no ASID.
    El2Va { addresses: Addresses },
}

impl Invalidation {
    /// Arm `TLBIMatch`: whether this removes `record`, of a processor
    /// whose physical addresses are `pa_bits` wide (`NUM_PABITS`, the bits
    /// of an IPA it compares). VMIDs agree as `use_vmid` and `vmid` are
    /// compared: both absent without EL2, else equal.
    pub(crate) fn matches(&self, record: &TlbRecord, pa_bits: u32) -> bool {
        let el10 = record.regime == Regime::El10;
        let stage1 = record.holds == Holds::Stage1;
        match *self {
            Self::El10Stage1 { vmid } => el10 && stage1 && vmid == record.vmid,
            Self::El10Asid { vmid, asid } => {
                el10 && stage1
                    && vmid == record.vmid
                    && record.scope == TranslationScope::NonGlobal
                    && record.asid == Some(asid)
            }
            Self::El10Va {
                vmid,
                asids,
                addresses,
            } => {
                let asid_matches = match asids {
                    Asids::Named(asid) => {
                        record.scope == TranslationScope::Global || record.asid == Some(asid)
                    }
                    Asids::All => true,
                };
                el10 && stage1
                    && vmid == record.vmid
                    && asid_matches
                    && addresses.match_input(record, VA_BITS)
            }
            Self::El10Stage12 { vmid } => el10 && vmid == record.vmid,
            Self::El10Every => el10,
            Self::Ipa { vmid, addresses } => {
                el10 && !stage1 && vmid == record.vmid && addresses.match_input(record, pa_bits)
            }
            Self::El2Every => !el10,
            Self::El2Va { addresses } => !el10 && stage1 && addresses.match_input(record, VA_BITS),
        }
    }
}

/// The operation a TLBI's encoding names (Arm's `TLBI` pages), whatever its
/// shareability, Last-level and `nXS` forms, which invalidate alike here
/// (see the module's documentation).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TlbiForm {
    VmallE1,
    AsidE1,
    VaE1,
    VaaE1,
    RvaE1,
    RvaaE1,
    VmallS12E1,
    AllE1,
    Ipas2E1,
    Ripas2E1,
    AllE2,
    VaE2,
    RvaE2,
}

impl TlbiForm {
    /// The operation of the EL1 TLBI at `CRm` and `op2` (`op1` 0, `CRn` 8
    /// or 9): `CRm` 1, 3 and 7 the Outer Shareable, Inner Shareable and
    /// local forms of `VMALLE1`, `VAE1`, `ASIDE1`, `VAAE1`, `VALE1` and
    /// `VAALE1` (`op2` 0, 1, 2, 3, 5, 7); `CRm` 2, 5 and 6 the Inner
    /// Shareable, Outer Shareable and local range forms (`op2` 1, 3, 5, 7).
    pub(crate) const fn el1(crm: u32, op2: u32) -> Option<Self> {
        match (crm, op2) {
            (1 | 3 | 7, 0) => Some(Self::VmallE1),
            (1 | 3 | 7, 1 | 5) => Some(Self::VaE1),
            (1 | 3 | 7, 2) => Some(Self::AsidE1),
            (1 | 3 | 7, 3 | 7) => Some(Self::VaaE1),
            (2 | 5 | 6, 1 | 5) => Some(Self::RvaE1),
            (2 | 5 | 6, 3 | 7) => Some(Self::RvaaE1),
            _ => None,
        }
    }

    /// The operation of the EL2 TLBI at `CRm` and `op2` (`op1` 4, `CRn` 8
    /// or 9): `CRm` 1, 3 and 7 `ALLE2`, `VAE2`, `ALLE1`, `VALE2` and
    /// `VMALLS12E1` (`op2` 0, 1, 4, 5, 6); `CRm` 0 and 4 the stage 2 forms
    /// by IPA (`IPAS2E1` and `IPAS2LE1`, `RIPAS2E1` and `RIPAS2LE1` in
    /// each shareability); `CRm` 2, 5 and 6 `RVAE2` and `RVALE2`.
    pub(crate) const fn el2(crm: u32, op2: u32) -> Option<Self> {
        match (crm, op2) {
            (1 | 3 | 7, 0) => Some(Self::AllE2),
            (1 | 3 | 7, 1 | 5) => Some(Self::VaE2),
            (1 | 3 | 7, 4) => Some(Self::AllE1),
            (1 | 3 | 7, 6) => Some(Self::VmallS12E1),
            (0, 1 | 5) | (4, 0 | 1 | 4 | 5) => Some(Self::Ipas2E1),
            (0, 2 | 6) | (4, 2 | 3 | 6 | 7) => Some(Self::Ripas2E1),
            (2 | 5 | 6, 1 | 5) => Some(Self::RvaE2),
            _ => None,
        }
    }
}

/// The granule a TLBI's `TG` field names (`DecodeTLBITG`), none for 0b00.
const fn tlbi_granule(tg: u64) -> Option<Granule> {
    match tg & 3 {
        0b01 => Some(Granule::K4),
        0b10 => Some(Granule::K16),
        0b11 => Some(Granule::K64),
        _ => None,
    }
}

impl<L: Levels> Cpu<L> {
    /// What a TLBI of `form` with operand `xt` invalidates, at the current
    /// EL; `None` for a range form whose `TG` names no granule, which
    /// invalidates nothing (`TLBIRange`'s `valid`).
    pub(crate) fn tlbi_invalidation(&self, form: TlbiForm, xt: u64) -> Option<Invalidation> {
        let vmid = self.current_vmid();
        let el1_large = self.sys.tcr_el1 & TCR_EL1_DS != 0;
        let el2_large = self.el2().tcr & TCR_EL2_DS != 0;
        let ipa_large = self.el2().vtcr & VTCR_EL2_DS != 0;
        let va = Addresses::At {
            address: (xt & ((1 << 44) - 1)) << 12,
            hint: self.level_hint(xt),
        };
        let invalidation = match form {
            TlbiForm::VmallE1 => Invalidation::El10Stage1 { vmid },
            TlbiForm::AsidE1 => Invalidation::El10Asid {
                vmid,
                asid: Asid::named(xt),
            },
            TlbiForm::VaE1 => Invalidation::El10Va {
                vmid,
                asids: Asids::Named(Asid::named(xt)),
                addresses: va,
            },
            TlbiForm::VaaE1 => Invalidation::El10Va {
                vmid,
                asids: Asids::All,
                addresses: va,
            },
            TlbiForm::RvaE1 => Invalidation::El10Va {
                vmid,
                asids: Asids::Named(Asid::named(xt)),
                addresses: Addresses::Range(self.tlbi_range(xt, el1_large)?),
            },
            TlbiForm::RvaaE1 => Invalidation::El10Va {
                vmid,
                asids: Asids::All,
                addresses: Addresses::Range(self.tlbi_range(xt, el1_large)?),
            },
            TlbiForm::VmallS12E1 => Invalidation::El10Stage12 { vmid },
            TlbiForm::AllE1 => Invalidation::El10Every,
            TlbiForm::Ipas2E1 => Invalidation::Ipa {
                vmid,
                addresses: Addresses::At {
                    address: (xt & ((1 << 40) - 1)) << 12,
                    hint: self.level_hint(xt),
                },
            },
            // An IPA range's base address takes FEAT_LPA2's form under
            // `VTCR_EL2.DS`, as the `TLBI RIPAS2E1` page of Arm's
            // `Registers.json` gives it, where `AArch64_TLBI_RIPAS2` hands
            // `TLBIRange` the EL1&0 regime, whose `HasLargeAddress` reads
            // `TCR_EL1.DS`: the register that describes the IPA is the
            // stage 2 walk's.
            TlbiForm::Ripas2E1 => Invalidation::Ipa {
                vmid,
                addresses: Addresses::Range(self.tlbi_range(xt, ipa_large)?),
            },
            TlbiForm::AllE2 => Invalidation::El2Every,
            TlbiForm::VaE2 => Invalidation::El2Va { addresses: va },
            TlbiForm::RvaE2 => Invalidation::El2Va {
                addresses: Addresses::Range(self.tlbi_range(xt, el2_large)?),
            },
        };
        Some(invalidation)
    }

    /// The VMID the EL1&0 regime's translations carry now (Arm `VMID()`),
    /// where the processor has EL2; none without, whose translations carry
    /// none (`UseVMID`).
    pub(crate) fn current_vmid(&self) -> Option<Vmid> {
        L::EL2.then(|| Vmid::of(self.el2().vttbr))
    }

    /// The `TTL` hint of a TLBI by address (`Xt[47:44]`, FEAT_TTL): none
    /// without FEAT_TTL, whose processors hold the field RES0, nor for an
    /// encoding `ResTLBITTL` reserves — no granule (0b00xx), level 0 of the
    /// 16 KiB and 64 KiB granules, and without FEAT_LPA2 level 0 of the 4
    /// KiB granule and level 1 of the 16 KiB one, which only its blocks
    /// are at.
    fn level_hint(&self, xt: u64) -> Option<LevelHint> {
        if !self.implements(Feature::Ttl) {
            return None;
        }
        let ttl = (xt >> 44) & 0xF;
        let granule = tlbi_granule(ttl >> 2)?;
        let level = LeafLevel::of((ttl & 3) as u8)?;
        let lpa2 = self.implements(Feature::Lpa2);
        let reserved = match (granule, level) {
            (Granule::K16 | Granule::K64, LeafLevel::Zero) => true,
            (Granule::K4, LeafLevel::Zero) | (Granule::K16, LeafLevel::One) => !lpa2,
            (Granule::K4 | Granule::K16 | Granule::K64, _) => false,
        };
        (!reserved).then_some(LevelHint { granule, level })
    }

    /// Arm `TLBIRange` of a range form's operand `xt`, its regime's `DS`
    /// set or not (`large`, which counts only with FEAT_LPA2,
    /// `HasLargeAddress`): `TG` (`Xt[47:46]`), the base address
    /// (`Xt[36:0]`) at bit 12, 14 or 16 by the granule — at 16 under
    /// FEAT_LPA2's `DS` — and sign-extended above, the range `(NUM + 1) <<
    /// (5 * SCALE + 1 + granule bits)` long, saturating where it would
    /// cross bit 52; and the level the `TTL` (`Xt[38:37]`) names, none for
    /// an encoding `ResTLBIRTTL` reserves (0b00, and 0b01 with the 16 KiB
    /// granule without FEAT_LPA2). `None` where `TG` is 0b00.
    fn tlbi_range(&self, xt: u64, large: bool) -> Option<AddressRange> {
        let granule = tlbi_granule(xt >> 46)?;
        let large = large && self.implements(Feature::Lpa2);
        let low = match granule {
            Granule::K4 | Granule::K16 if !large => granule.bits(),
            Granule::K4 | Granule::K16 | Granule::K64 => 16,
        };
        let start = sext((xt & ((1 << 37) - 1)) << low, low + 37);
        let scale = ((xt >> 44) & 3) as u32;
        let num = (xt >> 39) & 0x1F;
        let length = (num + 1) << (5 * scale + 1 + granule.bits());
        let mut end = start.wrapping_add(length);
        if (end >> 52) & 1 != (start >> 52) & 1 {
            end = if (start >> 52) & 1 == 1 {
                u64::MAX
            } else {
                (1 << 52) - 1
            };
        }
        let level = match ((xt >> 37) & 3, granule) {
            (0, _) => None,
            (1, Granule::K16) if !self.implements(Feature::Lpa2) => None,
            (ttl, _) => LeafLevel::of(ttl as u8),
        };
        Some(AddressRange {
            granule,
            start,
            end,
            level,
        })
    }
}
