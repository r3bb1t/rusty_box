//! Stage 2 translation (VMSAv8-64, FEAT_EL2): the walk of the tables
//! `VTTBR_EL2` and `VTCR_EL2` describe, which translates the EL1&0 regime's
//! intermediate physical addresses (IPAs) while `HCR_EL2.VM` or `DC` is set,
//! with the 4 KiB granule, the one the ID registers offer at stage 2; and
//! the permissions it gives.
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

use super::el2::{HCR_PTW, HCR_TGE};
use super::features::Feature;
use super::memattr::{self, MemAttrs};
use super::mmu::{
    Ipa, TranslationFault, WalkStart, WalkStop, PERM_EL0_R, PERM_EL0_W, PERM_EL0_X, PERM_EL1_R,
    PERM_EL1_W, PERM_EL1_X,
};
use super::regime::{el10_stage2_enabled, output_address_bits, Regime};
use super::{id, Cpu, Levels};
use crate::bus::{PhysAddr, PhysBus};

/// `VTCR_EL2.T0SZ`, bits 5:0.
const VTCR_T0SZ: u64 = 0x3F;
/// `VTCR_EL2.SL0`, bits 7:6.
const VTCR_SL0: u32 = 6;
/// `VTCR_EL2.PS`, bits 18:16.
const VTCR_PS: u32 = 16;

/// `AArch64_S2MinTxSZ` with the 4 KiB granule and no FEAT_LPA2 or FEAT_D128:
/// `64 - PAMax`, the input address being at most as wide as a physical one.
const S2_MIN_TXSZ: u32 = 64 - id::PA_BITS;
/// `AArch64_MaxTxSZ` without FEAT_TTST: 39, a 25-bit input address.
const MAX_TXSZ: u32 = 39;

// `AArch64_S2InvalidSL` makes `SL0` = 0b10, a start at level 0, invalid
// where `PAMax` is under 44 bits; on this processor it is valid.
const _: () = assert!(id::PA_BITS >= 44);

/// What a stage 2 walk reads from its registers (Arm `S2TTWParams`, from
/// `AArch64_NSS2TTWParams`): `VTCR_EL2`'s `T0SZ`, `SL0` and `PS`, and
/// `VTTBR_EL2`'s base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct S2Params {
    txsz: u32,
    sl0: u32,
    /// The output address size `PS` gives, at most `PAMax`.
    oa_bits: u32,
    vttbr: u64,
}

/// A completed stage 2 walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct S2Walked {
    pub(crate) pa: PhysAddr,
    /// The permissions stage 2 gives, as the stage 1 permission bits they
    /// leave an access (`AArch64_S2DirectBasePermissions`): read and write
    /// at EL1 and EL0 alike, and execute as `XN` gives each.
    pub(crate) perms: u8,
    pub(crate) level: u8,
    /// The descriptor's `MemAttr`, `SH` and `FnXS` (false without FEAT_XS),
    /// as they are: an access reads only whether they make Device memory,
    /// and only address translation decodes them ([`S2Walked::attrs`]).
    memattr: u8,
    sh: u8,
    fnxs: bool,
}

impl S2Walked {
    /// Whether stage 2 maps Device memory ([`memattr::stage2_device`]).
    #[inline]
    pub(crate) const fn device(&self) -> bool {
        memattr::stage2_device(self.memattr)
    }

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
            txsz: (el2.vtcr & VTCR_T0SZ) as u32,
            sl0: ((el2.vtcr >> VTCR_SL0) & 3) as u32,
            oa_bits: output_address_bits(el2.vtcr >> VTCR_PS),
            vttbr: el2.vttbr,
        }
    }

    /// Whether `HCR_EL2.PTW` refuses a stage 1 walk the descriptors stage
    /// 2 maps to Device memory (`walkparams.ptw`, clear under `TGE`).
    pub(crate) fn s2_ptw(&self) -> bool {
        L::EL2 && self.el2().hcr & (HCR_PTW | HCR_TGE) == HCR_PTW
    }

    /// The stage 2 translation of `ipa`, as far as the walk: the checks
    /// `AArch64_S2Translate` makes of the configuration and the address
    /// before it walks, all level 0 Translation faults, then
    /// `AArch64_S2Walk`. The start level is `VTCR_EL2.SL0`'s, and it may
    /// index up to 16 concatenated tables; every descriptor address and the
    /// output address must fit the output size `PS` gives, else an address
    /// size fault, at level 0 for the first descriptor and at the level of
    /// the descriptor holding the address for the others
    /// ([`Cpu::walk_tables`], whose descriptors' addresses are physical).
    pub(crate) fn s2_walk<B: PhysBus>(
        &self,
        bus: &mut B,
        ipa: Ipa,
    ) -> Result<S2Walked, TranslationFault> {
        let params = self.s2_params();
        let ipa = ipa.get();
        let level0 = TranslationFault::Translation { level: 0 };
        if !(S2_MIN_TXSZ..=MAX_TXSZ).contains(&params.txsz) {
            return Err(level0);
        }
        // `AArch64_S2StartLevel` and `AArch64_S2InvalidSL` with the 4 KiB
        // granule and no FEAT_LPA2 (`SL2` = 0): 0b11, a start at level 3,
        // needs FEAT_TTST.
        let start_level: u32 = match params.sl0 {
            0b00 => 2,
            0b01 => 1,
            0b10 => 0,
            _ => return Err(level0),
        };
        let ia_bits = 64 - params.txsz;
        // `AArch64_S2InconsistentSL`: the start level must resolve at least
        // one bit, and at most a full level's nine and four more, the
        // index into 16 concatenated tables.
        let lsb = 12 + 9 * (3 - start_level);
        if ia_bits <= lsb || ia_bits > lsb + 9 + 4 {
            return Err(level0);
        }
        if ipa >> ia_bits != 0 {
            return Err(level0);
        }
        let start =
            WalkStart::<PhysAddr>::of_base(params.vttbr, start_level, ia_bits, params.oa_bits)?;
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
            level: leaf.level as u8,
            memattr: ((desc >> 2) & 0xF) as u8,
            sh: ((desc >> 8) & 3) as u8,
            fnxs: self.implements(Feature::Xs) && (desc >> 11) & 1 == 1,
        })
    }
}
