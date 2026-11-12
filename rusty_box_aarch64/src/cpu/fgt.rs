//! Fine-grained traps (FEAT_FGT): the five registers EL2 traps single
//! System registers and System instructions of EL1 and EL0 with, and what
//! each of their fields traps.
//!
//! Provenance (R7): Arm's machine-readable register data (AARCHMRS
//! `Registers.json`, release 2026-09): the fieldsets of `HFGRTR_EL2`,
//! `HFGWTR_EL2`, `HFGITR_EL2`, `HDFGRTR_EL2` and `HDFGWTR_EL2`, each field
//! there for the feature its condition names, and the accessor of each
//! register and instruction, which tests the field (`HFGRTR_EL2.SCTLR_EL1
//! == '1'`, or for the fields Arm names with an `n`, `HFGRTR_EL2.nTPIDR2_EL0
//! == '0'`) in its EL1 branch and, for those EL0 reaches, in its EL0 branch
//! outside the host (`EffectivelyAtEL0NotInHost`), each where the accessor
//! orders it among the other traps; and the shared pseudocode of the A64
//! ISA XML (release 2026-09): `AArch64_CheckForSVCTrap` and
//! `AArch64_CheckForERetTrap` for `HFGITR_EL2.SVC_EL0`, `SVC_EL1` and
//! `ERET`. With no EL3 there is no `SCR_EL3.FGTEn`: the traps act wherever
//! `EL2Enabled()`. `tests/unit/sysreg_spec.rs` holds every register's and
//! System instruction's trap to Arm's accessors.

use super::el2::{El2Regs, HCRX_FGTNXS};
use super::exception::{Exception, SyndromeClass};
use super::exec::Fault;
use super::features::{Feature, Features, Implementation};
use super::gic::GicCpuInterface;
use super::{Cpu, El, Levels};

/// One of the fine-grained trap registers.
///
/// Closed (R5): the five FEAT_FGT gives EL2 (`HAFGRTR_EL2` is FEAT_AMUv1's,
/// which this processor lacks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FgtReg {
    /// `HFGRTR_EL2`: reads (`MRS`) of System registers.
    Hfgrtr,
    /// `HFGWTR_EL2`: writes (`MSR`) of System registers.
    Hfgwtr,
    /// `HFGITR_EL2`: System instructions, `SVC` and `ERET`.
    Hfgitr,
    /// `HDFGRTR_EL2`: reads of the debug and PMU registers.
    Hdfgrtr,
    /// `HDFGWTR_EL2`: writes of the debug and PMU registers.
    Hdfgwtr,
}

impl FgtReg {
    /// The register's value in `regs`.
    const fn value(self, regs: &El2Regs) -> u64 {
        match self {
            Self::Hfgrtr => regs.hfgrtr,
            Self::Hfgwtr => regs.hfgwtr,
            Self::Hfgitr => regs.hfgitr,
            Self::Hdfgrtr => regs.hdfgrtr,
            Self::Hdfgwtr => regs.hdfgwtr,
        }
    }

    /// The register's fields on a processor implementing `features`.
    pub(crate) const fn mask(self, features: Features) -> u64 {
        match self {
            Self::Hfgrtr => hfgrtr_mask(features),
            Self::Hfgwtr => hfgwtr_mask(features),
            Self::Hfgitr => hfgitr_mask(features),
            Self::Hdfgrtr => hdfgrtr_mask(features),
            Self::Hdfgwtr => hdfgwtr_mask(features),
        }
    }

    /// The fields the System register interface to a GIC CPU interface
    /// (FEAT_GICv3) adds to the register on a processor implementing `imp`:
    /// `HFGRTR_EL2.ICC_IGRPENn_EL1` and `HFGWTR_EL2.ICC_IGRPENn_EL1`
    /// (bit 39), with FEAT_FGT, which the board's GIC gives where it is a
    /// GICv3, the processor's features where they do not.
    pub(crate) const fn gic_fields(self, imp: Implementation) -> u64 {
        let interface = !matches!(imp.gic, GicCpuInterface::Absent);
        match self {
            Self::Hfgrtr | Self::Hfgwtr if interface && imp.features.has(Feature::Fgt) => {
                1 << FGT_ICC_IGRPEN
            }
            Self::Hfgrtr | Self::Hfgwtr | Self::Hfgitr | Self::Hdfgrtr | Self::Hdfgwtr => 0,
        }
    }

    /// The register's fields on a processor implementing `imp`: its
    /// features' and its GIC CPU interface's.
    const fn fields(self, imp: Implementation) -> u64 {
        self.mask(imp.features) | self.gic_fields(imp)
    }
}

/// `HFGRTR_EL2.ICC_IGRPENn_EL1` and `HFGWTR_EL2.ICC_IGRPENn_EL1`: EL1's
/// accesses to `ICC_IGRPEN0_EL1` and `ICC_IGRPEN1_EL1` trap to EL2.
pub(crate) const FGT_ICC_IGRPEN: u32 = 39;

/// When a fine-grained trap field traps.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrapsWhen {
    /// Set: every field but those Arm names with an `n`.
    Set,
    /// Clear: the fields Arm names with an `n` (`nTPIDR2_EL0`,
    /// `nSMPRI_EL1`), so that a hypervisor that writes zeros to the fields
    /// it does not know traps what they control.
    Clear,
}

/// One field of a fine-grained trap register: what traps an access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FineGrainedTrap {
    pub(crate) reg: FgtReg,
    pub(crate) bit: u32,
    pub(crate) traps_when: TrapsWhen,
}

impl FineGrainedTrap {
    /// The field at `bit` of `reg`, trapping when set.
    pub(crate) const fn set(reg: FgtReg, bit: u32) -> Self {
        Self {
            reg,
            bit,
            traps_when: TrapsWhen::Set,
        }
    }

    /// The field at `bit` of `reg`, trapping when clear.
    pub(crate) const fn clear(reg: FgtReg, bit: u32) -> Self {
        Self {
            reg,
            bit,
            traps_when: TrapsWhen::Clear,
        }
    }

    /// Whether the field traps on a processor implementing `imp` whose EL2
    /// registers are `regs`: a field the processor has ([`FgtReg::mask`],
    /// [`FgtReg::gic_fields`]) holding the value that traps. A field the
    /// processor lacks traps nothing, whichever way Arm names it.
    const fn traps(self, regs: &El2Regs, imp: Implementation) -> bool {
        let bit = 1u64 << self.bit;
        if self.reg.fields(imp) & bit == 0 {
            return false;
        }
        let set = self.reg.value(regs) & bit != 0;
        match self.traps_when {
            TrapsWhen::Set => set,
            TrapsWhen::Clear => !set,
        }
    }
}

/// `bits` where `features` holds `feature`, 0 otherwise.
const fn with(features: Features, feature: Feature, bits: u64) -> u64 {
    if features.has(feature) {
        bits
    } else {
        0
    }
}

/// A run of bits, `lo` to `hi` inclusive.
const fn bits(hi: u32, lo: u32) -> u64 {
    (u64::MAX >> (63 - hi)) & !((1u64 << lo) - 1)
}

/// The `HFGRTR_EL2` fields every processor with FEAT_FGT has: `AFSR0_EL1`,
/// `AFSR1_EL1`, `AIDR_EL1`, `AMAIR_EL1` (bits 3:0), `CCSIDR_EL1` to
/// `ISR_EL1` (18:9), `MAIR_EL1` to `SCTLR_EL1` (29:24), and `TCR_EL1` to
/// `VBAR_EL1` (38:32).
const HFGRTR_BASE: u64 = bits(3, 0) | bits(18, 9) | bits(29, 24) | bits(38, 32);
/// `HFGRTR_EL2`'s and `HFGWTR_EL2`'s pointer authentication key fields
/// (FEAT_PAuth): `APDAKey` to `APIBKey`, bits 8:4.
const FGT_PAUTH: u64 = bits(8, 4);
/// Their LORegion fields (FEAT_LOR): `LORC_EL1` to `LORSA_EL1`, bits 23:19
/// (`HFGWTR_EL2` without `LORID_EL1`'s bit 21).
const FGT_LOR: u64 = bits(23, 19);
/// Their error record fields of FEAT_RAS: `ERRIDR_EL1` to `ERXMISCn_EL1`
/// (bits 45:40) and `ERXADDR_EL1` (49).
const FGT_RAS: u64 = bits(45, 40) | (1 << 49);
/// Their fault injection fields (FEAT_RASv1p1): `ERXPFGF_EL1`,
/// `ERXPFGCTL_EL1`, `ERXPFGCDN_EL1`, bits 48:46.
const FGT_RAS_V1P1: u64 = bits(48, 46);
/// Their SME fields: `nSMPRI_EL1` (54) and `nTPIDR2_EL0` (55).
const FGT_SME: u64 = bits(55, 54);
/// The `HFGRTR_EL2` fields of registers no `MSR` writes, which
/// `HFGWTR_EL2` has as RES0: `AIDR_EL1` (2), `CCSIDR_EL1` (9),
/// `CLIDR_EL1` (10), `CTR_EL0` (14), `DCZID_EL0` (15), `ISR_EL1` (18),
/// `LORID_EL1` (21), `MIDR_EL1` (25), `MPIDR_EL1` (26), `REVIDR_EL1` (28),
/// `ERRIDR_EL1` (40), `ERXFR_EL1` (42) and `ERXPFGF_EL1` (46).
const READ_ONLY: u64 = (1 << 2)
    | (1 << 9)
    | (1 << 10)
    | (1 << 14)
    | (1 << 15)
    | (1 << 18)
    | (1 << 21)
    | (1 << 25)
    | (1 << 26)
    | (1 << 28)
    | (1 << 40)
    | (1 << 42)
    | (1 << 46);

/// The `HFGRTR_EL2` fields a processor implementing `features` has.
pub(crate) const fn hfgrtr_mask(features: Features) -> u64 {
    if !features.has(Feature::Fgt) {
        return 0;
    }
    HFGRTR_BASE
        | with(features, Feature::PAuth, FGT_PAUTH)
        | with(features, Feature::Lor, FGT_LOR)
        | with(features, Feature::Ras, FGT_RAS)
        | with(features, Feature::RasV1p1, FGT_RAS_V1P1)
        | with(features, Feature::Sme, FGT_SME)
}

/// The `HFGWTR_EL2` fields a processor implementing `features` has: those
/// of `HFGRTR_EL2` but the registers no `MSR` writes.
pub(crate) const fn hfgwtr_mask(features: Features) -> u64 {
    hfgrtr_mask(features) & !READ_ONLY
}

/// The `HFGITR_EL2` fields every processor with FEAT_FGT has: the cache
/// maintenance instructions `IC IALLUIS` to `DC CVAP` and `DC ZVA`, `DC
/// CCIVAC`'s (bits 8:0, 11:10), the address translations `AT S1E1R` to `AT
/// S1E0W` (15:12), the EL1 TLB maintenance instructions of FEAT_EL2,
/// Inner Shareable (33:28) and not (47:42), `ERET`, `SVC_EL0`, `SVC_EL1`
/// and `DC CVAC`'s (54:51).
const HFGITR_BASE: u64 = (bits(15, 0) & !(1 << 9)) | bits(33, 28) | bits(47, 42) | bits(54, 51);
/// `HFGITR_EL2.DCCVADP` (FEAT_DPB2).
const HFGITR_DPB2: u64 = 1 << 9;
/// `HFGITR_EL2.ATS1E1RP` and `ATS1E1WP` (FEAT_PAN2).
const HFGITR_PAN2: u64 = bits(17, 16);
/// `HFGITR_EL2`'s Outer Shareable TLB maintenance fields (FEAT_TLBIOS),
/// bits 23:18.
const HFGITR_TLBIOS: u64 = bits(23, 18);
/// Its range TLB maintenance fields (FEAT_TLBIRANGE), Inner Shareable and
/// not, bits 41:34.
const HFGITR_TLBIRANGE: u64 = bits(41, 34);
/// Its Outer Shareable range fields (FEAT_TLBIRANGE and FEAT_TLBIOS), bits
/// 27:24.
const HFGITR_TLBIRANGE_OS: u64 = bits(27, 24);
/// Its prediction restriction fields (FEAT_SPECRES): `CFPRCTX`,
/// `DVPRCTX` and `CPPRCTX`, bits 50:48.
const HFGITR_SPECRES: u64 = bits(50, 48);

/// The `HFGITR_EL2` fields a processor implementing `features` has.
pub(crate) const fn hfgitr_mask(features: Features) -> u64 {
    if !features.has(Feature::Fgt) {
        return 0;
    }
    let range_os = if features.has(Feature::TlbiRange) && features.has(Feature::TlbiOs) {
        HFGITR_TLBIRANGE_OS
    } else {
        0
    };
    HFGITR_BASE
        | with(features, Feature::Dpb2, HFGITR_DPB2)
        | with(features, Feature::Pan2, HFGITR_PAN2)
        | with(features, Feature::TlbiOs, HFGITR_TLBIOS)
        | with(features, Feature::TlbiRange, HFGITR_TLBIRANGE)
        | range_os
        | with(features, Feature::SpecRes, HFGITR_SPECRES)
}

/// The debug register fields `HDFGRTR_EL2` and `HDFGWTR_EL2` share, every
/// processor with FEAT_FGT having them: `DBGBCRn_EL1` to `MDSCR_EL1` (bits
/// 4:0), `DBGCLAIM` (5), `DBGPRCR_EL1` (7) and `OSECCR_EL1` (10).
const HDFG_DEBUG: u64 = bits(5, 0) | (1 << 7) | (1 << 10);
/// `HDFGRTR_EL2`'s debug fields of registers only read: `DBGAUTHSTATUS_EL1`
/// (6) and `OSLSR_EL1` (9).
const HDFGRTR_DEBUG_READ: u64 = (1 << 6) | (1 << 9);
/// `HDFGWTR_EL2.OSLAR_EL1`, bit 8: a register only written.
const HDFGWTR_OSLAR: u64 = 1 << 8;
/// The `OSDLR_EL1` field of both (FEAT_DoubleLock), bit 11.
const HDFG_OSDLR: u64 = 1 << 11;
/// The PMU fields both have (FEAT_PMUv3): `PMEVCNTRn_EL0` to `PMSELR_EL0`
/// (bits 19:12) and `PMUSERENR_EL0` (57).
const HDFG_PMU: u64 = bits(19, 12) | (1 << 57);
/// `HDFGRTR_EL2`'s PMU fields of registers only read: `PMMIR_EL1` (22) and
/// `PMCEIDn_EL0` (58).
const HDFGRTR_PMU_READ: u64 = (1 << 22) | (1 << 58);
/// `HDFGWTR_EL2`'s PMU fields of registers whose reads it does not trap:
/// `PMSWINC_EL0` (20), only written, and `PMCR_EL0` (21).
const HDFGWTR_PMU_WRITE: u64 = bits(21, 20);

/// The `HDFGRTR_EL2` fields a processor implementing `features` has.
pub(crate) const fn hdfgrtr_mask(features: Features) -> u64 {
    if !features.has(Feature::Fgt) {
        return 0;
    }
    HDFG_DEBUG
        | HDFGRTR_DEBUG_READ
        | with(features, Feature::DoubleLock, HDFG_OSDLR)
        | with(features, Feature::PmuV3, HDFG_PMU | HDFGRTR_PMU_READ)
}

/// The `HDFGWTR_EL2` fields a processor implementing `features` has.
pub(crate) const fn hdfgwtr_mask(features: Features) -> u64 {
    if !features.has(Feature::Fgt) {
        return 0;
    }
    HDFG_DEBUG
        | HDFGWTR_OSLAR
        | with(features, Feature::DoubleLock, HDFG_OSDLR)
        | with(features, Feature::PmuV3, HDFG_PMU | HDFGWTR_PMU_WRITE)
}

/// The `HFGITR_EL2` field that traps the System instruction (`SYS`) at
/// `op1`, `CRn`, `CRm` and `op2` at EL1, and at EL0 outside the host, if it
/// has one, by its accessor: the cache maintenance instructions, `DC CVAC`'s
/// field at bit 54; the address translations of the EL1&0 regime from EL1;
/// the prediction restrictions (FEAT_SPECRES); and the EL1 TLB maintenance
/// instructions, each of its own and the nXS form (`CRn` 9, FEAT_XS) by
/// its plain form's, which `HCRX_EL2.FGTnXS` can exempt
/// ([`Cpu::fine_grained_tlbi_nxs`]).
pub(crate) const fn hfgitr_field(
    op1: u32,
    crn: u32,
    crm: u32,
    op2: u32,
) -> Option<FineGrainedTrap> {
    let bit = match (op1, crn, crm, op2) {
        (0, 7, 1, 0) => 0,   // IC IALLUIS
        (0, 7, 5, 0) => 1,   // IC IALLU
        (3, 7, 5, 1) => 2,   // IC IVAU
        (0, 7, 6, 1) => 3,   // DC IVAC
        (0, 7, 6, 2) => 4,   // DC ISW
        (0, 7, 10, 2) => 5,  // DC CSW
        (0, 7, 14, 2) => 6,  // DC CISW
        (3, 7, 11, 1) => 7,  // DC CVAU
        (3, 7, 12, 1) => 8,  // DC CVAP
        (3, 7, 13, 1) => 9,  // DC CVADP
        (3, 7, 14, 1) => 10, // DC CIVAC
        (3, 7, 4, 1) => 11,  // DC ZVA
        (3, 7, 10, 1) => 54, // DC CVAC
        // AT S1E1R, S1E1W, S1E0R, S1E0W; S1E1RP, S1E1WP.
        (0, 7, 8, 0..=3) => 12 + op2,
        (0, 7, 9, 0 | 1) => 16 + op2,
        // CFP, DVP and CPP RCTX.
        (3, 7, 3, 4) => 48,
        (3, 7, 3, 5) => 49,
        (3, 7, 3, 7) => 50,
        (0, 8 | 9, _, _) => match tlbi_field(crm, op2) {
            Some(bit) => bit,
            None => return None,
        },
        _ => return None,
    };
    Some(FineGrainedTrap::set(FgtReg::Hfgitr, bit))
}

/// The `HFGITR_EL2` field of the EL1 TLB maintenance instruction at `CRm`
/// and `op2`: the Outer Shareable forms at bits 23:18 and their range forms
/// at 27:24, the Inner Shareable forms at 33:28 and range forms at 37:34,
/// the local range forms at 41:38 and forms at 47:42, each group in the
/// order `VMALLE1`, `VAE1`, `ASIDE1`, `VAAE1`, `VALE1`, `VAALE1` (`op2` 0,
/// 1, 2, 3, 5, 7), the range groups `RVAE1`, `RVAAE1`, `RVALE1`,
/// `RVAALE1` (`op2` 1, 3, 5, 7).
const fn tlbi_field(crm: u32, op2: u32) -> Option<u32> {
    let (base, range) = match crm {
        1 => (18, false),
        3 => (28, false),
        7 => (42, false),
        5 => (24, true),
        2 => (34, true),
        6 => (38, true),
        _ => return None,
    };
    let index = match (range, op2) {
        (false, 0..=3) => op2,
        (false, 5) => 4,
        (false, 7) => 5,
        (true, 1 | 3 | 5 | 7) => op2 / 2,
        _ => return None,
    };
    Some(base + index)
}

/// `HFGITR_EL2.ERET`: `ERET`, `ERETAA` and `ERETAB` at EL1 trap to EL2,
/// with EC 0x1A (`AArch64_CheckForERetTrap`).
pub(crate) const HFGITR_ERET: FineGrainedTrap = FineGrainedTrap::set(FgtReg::Hfgitr, 51);
/// The form of an exception return, as a trapped one reports it in its
/// syndrome (`AArch64_CheckForERetTrap`).
///
/// Closed (R5): `ERET`, and `ERETAA` and `ERETAB` (FEAT_PAuth).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EretForm {
    Plain,
    KeyA,
    KeyB,
}

impl EretForm {
    /// `ESR_ELx.ISS` of its trap: bit 1 set for an authenticated return,
    /// bit 0 then naming key B.
    const fn iss(self) -> u32 {
        match self {
            Self::Plain => 0,
            Self::KeyA => 0b10,
            Self::KeyB => 0b11,
        }
    }
}

/// `HFGITR_EL2.SVC_EL0` and `SVC_EL1`: `SVC` at EL0 (outside the host)
/// and at EL1 traps to EL2, returning to the `SVC` itself
/// (`AArch64_CheckForSVCTrap`).
pub(crate) const HFGITR_SVC_EL0: FineGrainedTrap = FineGrainedTrap::set(FgtReg::Hfgitr, 52);
pub(crate) const HFGITR_SVC_EL1: FineGrainedTrap = FineGrainedTrap::set(FgtReg::Hfgitr, 53);

impl<L: Levels> Cpu<L> {
    /// Whether `trap` traps an access at `el` (each accessor's fine-grained
    /// trap condition: `EL2Enabled()`, FEAT_FGT, at EL1, or at EL0 outside
    /// the host, `EffectivelyAtEL0NotInHost()`). A processor without EL2
    /// tests a constant.
    #[inline]
    pub(crate) fn fine_grained_traps(&self, trap: FineGrainedTrap, el: El) -> bool {
        let applies = match el {
            El::El1 => true,
            El::El0 => !self.el0_in_host(),
            El::El2 => false,
        };
        L::EL2 && applies && trap.traps(self.el2(), self.imp.get())
    }

    /// [`Cpu::fine_grained_traps`] of `trap`, where there is one.
    #[inline]
    pub(crate) fn fine_grained_traps_any(&self, trap: Option<FineGrainedTrap>, el: El) -> bool {
        match trap {
            Some(trap) => self.fine_grained_traps(trap, el),
            None => false,
        }
    }

    /// Whether `HFGITR_EL2` traps the System instruction at `op1`, `CRn`,
    /// `CRm` and `op2` at `el` ([`hfgitr_field`]): an nXS form of a TLB
    /// maintenance instruction (`CRn` 9) only where
    /// [`Cpu::fine_grained_tlbi_nxs`].
    pub(crate) fn fine_grained_instruction(
        &self,
        op1: u32,
        crn: u32,
        crm: u32,
        op2: u32,
        el: El,
    ) -> bool {
        let nxs = op1 == 0 && crn == 9;
        (!nxs || self.fine_grained_tlbi_nxs())
            && self.fine_grained_traps_any(hfgitr_field(op1, crn, crm, op2), el)
    }

    /// Arm `AArch64_CheckForERetTrap` of an exception return of `form`: at
    /// EL1 with `HFGITR_EL2.ERET` set, or under `HCR_EL2.NV` (FEAT_NV, a
    /// guest hypervisor's return, which with no FEAT_NV3 has no
    /// `NVHCR_EL2.TGE` to spare it), a trap to EL2 with EC 0x1A, ahead of
    /// reading `ELR_EL1` and of authenticating it.
    pub(crate) fn check_eret_trap(&self, form: EretForm) -> Result<(), Fault> {
        if self.pstate.el == El::El1
            && (self.fine_grained_traps(HFGITR_ERET, El::El1) || self.nested().nv())
        {
            return Err(Exception::to_el2(SyndromeClass::ERetTrap, form.iss()).into());
        }
        Ok(())
    }

    /// Whether the `HFGITR_EL2` fields of the EL1 TLB maintenance
    /// instructions trap their nXS forms too (each nXS form's accessor):
    /// only with FEAT_HCX, and there unless `HCRX_EL2.FGTnXS` is set.
    fn fine_grained_tlbi_nxs(&self) -> bool {
        self.implements(Feature::Hcx) && self.el2().hcrx & HCRX_FGTNXS == 0
    }
}
