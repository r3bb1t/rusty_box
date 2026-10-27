//! Translation regimes (Arm `Regime`): which stage 1 tables translate an
//! address, and the controls that govern them.
//!
//! Provenance (R7): the shared pseudocode of the A64 ISA XML (release
//! 2026-09): `TranslationRegime`, `AArch64_GetS1TTWParams`,
//! `AArch64_S1TTWParamsEL10`, `AArch64_S1TTWParamsEL2`,
//! `AArch64_S1Enabled`, `AArch64_AddrTop`, `AArch64_PhysicalAddressSize`,
//! `AArch64_BranchAddr`, `HasUnprivileged`.
//!
//! The walk parameters are gathered here, one place per regime; the walk
//! itself, the TLB and the access paths are `mmu.rs`'s.

use super::el2::{HCR_DC, HCR_TGE, TCR_EL2_HPD, TCR_EL2_TBI, TCR_EL2_TBID};
use super::features::Feature;
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
    /// copies in the upper range, zeros in the lower and in a regime with
    /// one range (`AArch64_BranchAddr` under top-byte-ignore).
    #[inline(always)]
    pub(crate) const fn canonical(self, va: u64) -> u64 {
        if self.upper(va) {
            va | 0xFF00_0000_0000_0000
        } else {
            va & 0x00FF_FFFF_FFFF_FFFF
        }
    }
}

/// `TCR_EL1` fields of the lower range (`TTBR0_EL1`'s); each upper-range
/// field sits at the offset its constant names from it.
const TCR_EL1_T0SZ: u32 = 0;
const TCR_EL1_EPD0: u32 = 7;
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

/// `TCR_ELx.PS`/`IPS` and `AArch64_PhysicalAddressSize`: the size in bits
/// each encoding names, which the walk's output addresses must fit.
const PS_BITS: [u32; 8] = [32, 36, 40, 42, 44, 48, 52, 56];

/// The output address size of an encoded `PS` or `IPS` field on this
/// processor (Arm `AArch64_PhysicalAddressSize` with no FEAT_LPA and no
/// FEAT_D128): the field's size, at most 48 bits and at most `PAMax`.
#[inline]
pub(crate) const fn output_address_bits(encoded: u64) -> u32 {
    let ps = PS_BITS[(encoded & 7) as usize];
    let max = if id::PA_BITS < 48 { id::PA_BITS } else { 48 };
    if ps < max {
        ps
    } else {
        max
    }
}

/// What a stage 1 walk of one regime reads from its registers (Arm
/// `S1TTWParams`), for the virtual address range of the address walked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct S1Params {
    pub(crate) regime: Regime,
    /// The upper virtual address range (`TTBR1_EL1`), only in EL1&0.
    pub(crate) upper: bool,
    /// The range's table base register.
    pub(crate) ttbr: u64,
    /// `TxSZ`, held within 16..=39 ([`Cpu::regime_txsz`]).
    pub(crate) txsz: u32,
    /// The range's walks are disabled (`TCR_EL1.EPD0`/`EPD1`).
    pub(crate) epd: bool,
    /// Hierarchical permissions disabled (FEAT_HPDS).
    pub(crate) hpd: bool,
    /// The output address size, from `IPS` or `PS`.
    pub(crate) oa_bits: u32,
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
    /// `HCR_EL2.DC` and `HCR_EL2.TGE` override (stage 1 of EL1&0 is then
    /// disabled whatever `SCTLR_EL1.M` says).
    #[inline(always)]
    pub(crate) fn s1_enabled(&self, regime: Regime) -> bool {
        match regime {
            Regime::El10 => self.sys.sctlr_el1 & sctlr::M != 0 && !self.hcr_any(HCR_DC | HCR_TGE),
            Regime::El2 => self.el2().sctlr & sctlr::M != 0,
        }
    }

    /// Whether data accesses of `regime` with stage 1 disabled are to
    /// Normal memory rather than Device-nGnRnE (Arm
    /// `AArch64_S1DisabledOutput`): in EL1&0 under `HCR_EL2.DC`.
    #[inline(always)]
    pub(crate) fn s1_disabled_data_normal(&self, regime: Regime) -> bool {
        regime == Regime::El10 && self.hcr_any(HCR_DC)
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
            Regime::El10 => {
                let tcr = self.sys.tcr_el1;
                let tbi = (tcr >> (TCR_EL1_TBI0 + u32::from(upper))) & 1 == 1;
                let tbid = (tcr >> (TCR_EL1_TBID0 + u32::from(upper))) & 1 == 1;
                tbi && !(instruction && tbid && pauth)
            }
            Regime::El2 => {
                let tcr = self.el2().tcr;
                tcr & TCR_EL2_TBI != 0 && !(instruction && tcr & TCR_EL2_TBID != 0 && pauth)
            }
        }
    }

    /// `TxSZ` of the range of `regime` that `upper` names, held within
    /// 16..=39: a value outside them behaves as the nearest (Arm
    /// `AArch64_S1TxSZFaults`, the IMPLEMENTATION DEFINED choice not to
    /// fault made here), for the walk and for the bottom bit of a pointer
    /// authentication code alike (`AArch64_PACEffectiveTxSZ`).
    #[inline]
    pub(crate) fn regime_txsz(&self, regime: Regime, upper: bool) -> u32 {
        let txsz = match regime {
            Regime::El10 => {
                let shift = TCR_EL1_T0SZ + if upper { TCR_EL1_UPPER_SZ_EPD } else { 0 };
                (self.sys.tcr_el1 >> shift) & 0x3F
            }
            Regime::El2 => self.el2().tcr & 0x3F,
        };
        txsz.clamp(16, 39) as u32
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
                let ttbr = if upper { self.sys.ttbr1_el1 } else { self.sys.ttbr0_el1 };
                let epd_bit = TCR_EL1_EPD0 + if upper { TCR_EL1_UPPER_SZ_EPD } else { 0 };
                let up = u32::from(upper);
                S1Params {
                    regime,
                    upper,
                    ttbr,
                    txsz: self.regime_txsz(regime, upper),
                    epd: (tcr >> epd_bit) & 1 == 1,
                    hpd: hpds && (tcr >> (TCR_EL1_HPD0 + up)) & 1 == 1,
                    oa_bits: output_address_bits(tcr >> TCR_EL1_IPS),
                    mair: self.sys.mair_el1,
                    wxn: self.sys.sctlr_el1 & sctlr::WXN != 0,
                }
            }
            Regime::El2 => {
                let el2 = self.el2();
                S1Params {
                    regime,
                    upper: false,
                    ttbr: el2.ttbr0,
                    txsz: self.regime_txsz(regime, false),
                    epd: false,
                    hpd: hpds && el2.tcr & TCR_EL2_HPD != 0,
                    oa_bits: output_address_bits(el2.tcr >> TCR_EL2_PS),
                    mair: el2.mair,
                    wxn: el2.sctlr & sctlr::WXN != 0,
                }
            }
        }
    }

    /// Arm `AArch64_BranchAddr(target, PSTATE.EL)`: the address a branch,
    /// an exception entry or an exception return writes to the PC. Under
    /// top-byte-ignore of instruction addresses the tag is dropped: bits
    /// [63:56] become copies of bit 55 at EL0 and EL1, whose regime has two
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

    /// `AArch64_PhysicalAddressSize` with no FEAT_LPA: the encoded size, at
    /// most 48 bits and at most this processor's 44.
    #[test]
    fn output_sizes_stop_at_the_physical_address_size() {
        assert_eq!(output_address_bits(0b000), 32);
        assert_eq!(output_address_bits(0b011), 42);
        assert_eq!(output_address_bits(0b100), 44);
        assert_eq!(output_address_bits(0b101), 44, "48 bits, beyond PAMax");
        assert_eq!(output_address_bits(0b111), 44, "56 bits");
    }
}
