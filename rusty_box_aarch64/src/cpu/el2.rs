//! Exception level 2 (FEAT_EL2): the hypervisor's registers, and the
//! controls they hold over EL1 and EL0.
//!
//! Provenance (R7): Arm's machine-readable register data (AARCHMRS
//! `Registers.json`, release 2026-09), each register's fields and the
//! features that make them exist, and the shared pseudocode of the A64 ISA
//! XML (release 2026-09): `EL2Enabled`, `HaveEL`, and the routing and trap
//! functions named where they are used. With no EL3 and no Secure state,
//! `EL2Enabled()` is `HaveEL(EL2)`.
//!
//! Every EL2 register here is in its `HCR_EL2.E2H == 0` layout: this
//! processor has no FEAT_VHE, so `E2H` is RES0 and the layout never
//! changes. A bit that is RES0 or RES1 for the features a processor
//! implements is hardwired to that value, reads returning it and writes
//! ignoring it, one of the two behaviours Arm permits for a reserved bit
//! (the other keeps what is written); the
//! IMPLEMENTATION DEFINED registers (`ACTLR_EL2`, `HACR_EL2`, `AFSR0_EL2`,
//! `AFSR1_EL2`, `AMAIR_EL2`) hold all 64 bits and control nothing, as their
//! EL1 counterparts do here.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use super::features::{Feature, Features, Implementation};
use super::sysreg::{smcr_mask, DISR_MASK};
use super::{id, Cpu, El};

// ----- The levels a processor implements -----

/// The exception levels a processor implements above EL1, as a type (R2):
/// whether it has EL2 is decided when its machine is built, as QEMU decides
/// it with `-M virt,virtualization=on|off`, and never changes. A processor
/// without EL2 holds no EL2 state, and every EL2 rule tests
/// [`Levels::EL2`], a constant, so on that processor the rules compile
/// away.
///
/// Closed (R5) and sealed: [`NoEl2`] and [`WithEl2`] are the level sets of
/// a processor with no EL3.
pub trait Levels: sealed::Sealed + Sized + 'static {
    /// Arm `HaveEL(EL2)`, which with no EL3 and no Secure state is
    /// `EL2Enabled()`.
    const EL2: bool;
}

/// EL0 and EL1 (QEMU `-M virt,virtualization=off`): the processor resets to
/// EL1, and its firmware takes PSCI calls on `HVC`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoEl2 {}

/// EL0, EL1 and EL2, in AArch64 state only (FEAT_EL2 and FEAT_AA64EL2,
/// QEMU `-M virt,virtualization=on`): the processor resets to EL2, `HVC`
/// is its hypervisor's, and its firmware takes PSCI calls on `SMC`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WithEl2 {}

impl Levels for NoEl2 {
    const EL2: bool = false;
}

impl Levels for WithEl2 {
    const EL2: bool = true;
}

mod sealed {
    /// What a level set stores of EL2.
    pub trait Sealed {
        type State: super::El2Storage;
    }
}

impl sealed::Sealed for NoEl2 {
    type State = NoEl2State;
}

impl sealed::Sealed for WithEl2 {
    type State = El2State;
}

/// Where a processor keeps its EL2 state: [`El2State`] with EL2, nothing
/// without. Public only as [`Levels`]'s sealed supertrait needs it, in a
/// module no other crate can name.
pub trait El2Storage: Clone + core::fmt::Debug + PartialEq + Eq + Send + Sync + 'static {
    /// A proof that the processor has EL2, which only
    /// [`El2Storage::proof`] makes: uninhabited on a processor without, so
    /// no value that carries one exists there.
    type Proof: Copy + core::fmt::Debug + Eq;
    /// The storage of `state`: kept on a processor with EL2, dropped on one
    /// without.
    fn keep(state: El2State) -> Self;
    /// The state, if the processor has EL2: a constant `None` without.
    fn get(&self) -> Option<&El2State>;
    fn get_mut(&mut self) -> Option<&mut El2State>;
    /// The proof, if the processor has EL2: a constant `None` without.
    fn proof() -> Option<Self::Proof>;
    /// The state `proof` shows the processor has.
    fn proven(&mut self, proof: Self::Proof) -> &mut El2State;
    fn proven_ref(&self, proof: Self::Proof) -> &El2State;
}

/// The EL2 state of a processor without EL2: none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoEl2State;

impl El2Storage for NoEl2State {
    type Proof = core::convert::Infallible;

    #[inline(always)]
    fn keep(_state: El2State) -> Self {
        Self
    }

    #[inline(always)]
    fn get(&self) -> Option<&El2State> {
        None
    }

    #[inline(always)]
    fn get_mut(&mut self) -> Option<&mut El2State> {
        None
    }

    #[inline(always)]
    fn proof() -> Option<Self::Proof> {
        None
    }

    #[inline(always)]
    fn proven(&mut self, proof: Self::Proof) -> &mut El2State {
        match proof {}
    }

    #[inline(always)]
    fn proven_ref(&self, proof: Self::Proof) -> &El2State {
        match proof {}
    }
}

impl El2Storage for El2State {
    type Proof = ();

    #[inline(always)]
    fn keep(state: El2State) -> Self {
        state
    }

    #[inline(always)]
    fn get(&self) -> Option<&El2State> {
        Some(self)
    }

    #[inline(always)]
    fn get_mut(&mut self) -> Option<&mut El2State> {
        Some(self)
    }

    #[inline(always)]
    fn proof() -> Option<Self::Proof> {
        Some(())
    }

    #[inline(always)]
    fn proven(&mut self, (): Self::Proof) -> &mut El2State {
        self
    }

    #[inline(always)]
    fn proven_ref(&self, (): Self::Proof) -> &El2State {
        self
    }
}

impl El2Regs {
    /// The EL2 registers out of reset. Arm `AArch64_TakeReset` leaves every
    /// register its pseudocode does not reset UNKNOWN, and the fields the
    /// Arm ARM gives a reset value when the PE resets into EL2 (such as
    /// `SCTLR_EL2.M`, `C` and `I`) reset to zero; this processor's choice for
    /// the UNKNOWN rest is zero too, but for the RES1 bits each register
    /// hardwires, `VPIDR_EL2` and `VMPIDR_EL2`, which reset to the
    /// `MIDR_EL1` and `MPIDR_EL1` they stand in for, so EL1 reads the
    /// processor's own identity until EL2 programs another, and
    /// `CNTHCTL_EL2`, whose `EL1PCTEN` and `EL1PCEN` reset to 1, so EL1's
    /// accesses to the physical counter and timer do not trap until EL2
    /// asks. Which bits are RES1 depends on the features `imp` implements:
    /// the RES1 bits of a feature the processor implements are that
    /// feature's field, zero out of reset.
    pub(crate) const fn reset(imp: Implementation) -> Self {
        let features = imp.features;
        Self {
            hcr: HCR_RES1,
            sctlr: SCTLR_EL2_RES1,
            actlr: 0,
            cptr: cptr_el2_res1(features),
            hstr: 0,
            hacr: 0,
            mdcr: 0,
            tcr: TCR_EL2_RES1,
            ttbr0: 0,
            vtcr: VTCR_EL2_RES1,
            vttbr: 0,
            mair: 0,
            amair: 0,
            vbar: 0,
            esr: 0,
            far: 0,
            hpfar: 0,
            elr: 0,
            spsr: 0,
            afsr0: 0,
            afsr1: 0,
            tpidr: 0,
            vpidr: imp.midr,
            vmpidr: id::MPIDR_EL1,
            smcr: 0,
            cnthctl: CNTHCTL_EL1PCTEN | CNTHCTL_EL1PCEN,
            cntvoff: 0,
            cnthp_ctl: 0,
            cnthp_cval: 0,
            contextidr: 0,
            vdisr: 0,
            vsesr: 0,
        }
    }
}

// ----- HCR_EL2 -----

/// `HCR_EL2.FMO`, `IMO`, `AMO`: physical FIQ, IRQ and SError routing to EL2.
pub(crate) const HCR_FMO: u64 = 1 << 3;
pub(crate) const HCR_IMO: u64 = 1 << 4;
pub(crate) const HCR_AMO: u64 = 1 << 5;
/// `HCR_EL2.VF`, `VI`, `VSE`: a virtual FIQ, IRQ and SError pending for EL1
/// and EL0, whose physical ones `FMO`, `IMO` and `AMO` route to EL2.
pub(crate) const HCR_VF: u64 = 1 << 6;
pub(crate) const HCR_VI: u64 = 1 << 7;
pub(crate) const HCR_VSE: u64 = 1 << 8;
/// `HCR_EL2.DC`: the EL1&0 regime's stage 1 is disabled, and its data
/// accesses are to Normal Write-Back memory (`AArch64_S1Enabled`,
/// `AArch64_S1DisabledOutput`).
pub(crate) const HCR_DC: u64 = 1 << 12;
/// `HCR_EL2.TWI`, `TWE`: WFI and WFE (and WFIT, WFET) at EL0 and EL1 trap
/// to EL2.
pub(crate) const HCR_TWI: u64 = 1 << 13;
pub(crate) const HCR_TWE: u64 = 1 << 14;
/// `HCR_EL2.TID1`: EL1's reads of `REVIDR_EL1`, `AIDR_EL1` and
/// `SMIDR_EL1` trap to EL2.
pub(crate) const HCR_TID1: u64 = 1 << 16;
/// `HCR_EL2.TID2`: accesses to the cache identification registers trap.
pub(crate) const HCR_TID2: u64 = 1 << 17;
/// `HCR_EL2.TID3`: EL1's reads of the ID group 3 registers trap.
pub(crate) const HCR_TID3: u64 = 1 << 18;
/// `HCR_EL2.TSC`: SMC at EL1 traps to EL2.
pub(crate) const HCR_TSC: u64 = 1 << 19;
/// `HCR_EL2.TIDCP`: EL1's accesses to the IMPLEMENTATION DEFINED space
/// trap. Whether EL0's trap too is IMPLEMENTATION DEFINED; here they are
/// UNDEFINED ([`Cpu::impdef_space_access`]).
pub(crate) const HCR_TIDCP: u64 = 1 << 20;
/// `HCR_EL2.TACR`: EL1's accesses to `ACTLR_EL1` trap.
pub(crate) const HCR_TACR: u64 = 1 << 21;
/// `HCR_EL2.TSW`: data cache maintenance by set/way traps.
pub(crate) const HCR_TSW: u64 = 1 << 22;
/// `HCR_EL2.TPCP`: data cache maintenance to the Point of Coherency or
/// Persistence traps.
pub(crate) const HCR_TPCP: u64 = 1 << 23;
/// `HCR_EL2.TPU`: cache maintenance to the Point of Unification traps.
pub(crate) const HCR_TPU: u64 = 1 << 24;
/// `HCR_EL2.TTLB`: EL1's TLB maintenance instructions trap.
pub(crate) const HCR_TTLB: u64 = 1 << 25;
/// `HCR_EL2.TVM`: EL1's writes to the virtual memory controls trap.
pub(crate) const HCR_TVM: u64 = 1 << 26;
/// `HCR_EL2.TGE`: EL0 runs under EL2, which takes the exceptions EL1 would.
pub(crate) const HCR_TGE: u64 = 1 << 27;
/// `HCR_EL2.TDZ`: `DC ZVA` traps, and `DCZID_EL0.DZP` reads 1.
pub(crate) const HCR_TDZ: u64 = 1 << 28;
/// `HCR_EL2.HCD`: with no EL3, `HVC` is UNDEFINED.
pub(crate) const HCR_HCD: u64 = 1 << 29;
/// `HCR_EL2.TRVM`: EL1's reads of the virtual memory controls trap.
pub(crate) const HCR_TRVM: u64 = 1 << 30;
/// `HCR_EL2.RW`: EL1 is AArch64, the only state it has (RAO/WI).
pub(crate) const HCR_RW: u64 = 1 << 31;
/// `HCR_EL2.TLOR` (FEAT_LOR): EL1's accesses to the LORegion registers trap.
pub(crate) const HCR_TLOR: u64 = 1 << 35;
/// `HCR_EL2.TERR` (FEAT_RAS): EL1's accesses to the error record registers
/// trap.
pub(crate) const HCR_TERR: u64 = 1 << 36;
/// `HCR_EL2.TEA` (FEAT_RAS): synchronous External aborts at EL0 and EL1
/// are taken to EL2.
pub(crate) const HCR_TEA: u64 = 1 << 37;
/// `HCR_EL2.APK` (FEAT_PAuth): 0 traps EL1's accesses to the key registers.
pub(crate) const HCR_APK: u64 = 1 << 40;
/// `HCR_EL2.API` (FEAT_PAuth): 0 traps the pointer authentication
/// instructions at EL0 and EL1.
pub(crate) const HCR_API: u64 = 1 << 41;
/// `HCR_EL2.FIEN` (FEAT_RASv1p1): EL1's accesses to the error injection
/// registers do not trap.
pub(crate) const HCR_FIEN: u64 = 1 << 47;

/// The `HCR_EL2` bits every processor with EL2 keeps (ARMv8.0: bits 33:0
/// but `TID0`, which is RES0 with no AArch32, and `RW`, RAO/WI): `VM`,
/// `SWIO`, `PTW`, `FMO`, `IMO`, `AMO`, `VF`, `VI`, `VSE`, `FB`, `BSU`,
/// `DC`, `TWI`, `TWE`, `TID1`-`TID3`, `TSC`, `TIDCP`, `TACR`, `TSW`, `TPCP`,
/// `TPU`, `TTLB`, `TVM`, `TGE`, `TDZ`, `HCD` (no EL3), `TRVM`, `CD`, `ID`.
const HCR_V8: u64 = 0x0000_0003_7FFF_7FFF;
/// `HCR_EL2`'s RES1 bits: `RW`.
const HCR_RES1: u64 = HCR_RW;

/// The `HCR_EL2` bits a processor implementing `features` keeps.
pub(crate) const fn hcr_mask(features: Features) -> u64 {
    let mut mask = HCR_V8;
    if features.has(Feature::Lor) {
        mask |= HCR_TLOR;
    }
    if features.has(Feature::Ras) {
        mask |= HCR_TERR | HCR_TEA;
    }
    if features.has(Feature::RasV1p1) {
        mask |= HCR_FIEN;
    }
    if features.has(Feature::PAuth) {
        mask |= HCR_APK | HCR_API;
    }
    mask
}

// ----- SCTLR_EL2 -----

/// `SCTLR_EL2.nAA` (FEAT_LSE2). `M`, `A`, `C`, `SA`, `I` and `WXN` sit at
/// `SCTLR_EL1`'s bits, which the `sctlr` module names.
pub(crate) const SCTLR_EL2_NAA: u64 = 1 << 6;
/// `SCTLR_EL2.BT` (FEAT_BTI): `PACIASP` and `PACIBSP` are not a branch
/// target for `BR` from a guarded page.
pub(crate) const SCTLR_EL2_BT: u64 = 1 << 36;
/// `SCTLR_EL2.DSSBS` (FEAT_SSBS): the `PSTATE.SSBS` an exception to EL2
/// sets.
pub(crate) const SCTLR_EL2_DSSBS: u64 = 1 << 44;
/// The pointer authentication enables (FEAT_PAuth), at `SCTLR_EL1`'s bits.
const SCTLR_EL2_PAUTH: u64 = (1 << 31) | (1 << 30) | (1 << 27) | (1 << 13);

/// The `SCTLR_EL2` bits every processor with EL2 keeps: `M`, `A`, `C`,
/// `SA`, `I`, `WXN`.
const SCTLR_EL2_BASE: u64 = 0x0008_100F;
/// `SCTLR_EL2`'s RES1 bits with no FEAT_ExS and `E2H` = 0: 4 (`SA0`), 5
/// (`CP15BEN`), 11 (`EOS`), 16 (`nTWI`), 18 (`nTWE`), 22 (`EIS`), 23
/// (`SPAN`), 28 (`nTLSMD`), 29 (`LSMAOE`).
pub(crate) const SCTLR_EL2_RES1: u64 = 0x30C5_0830;

/// The `SCTLR_EL2` bits a processor implementing `features` keeps.
pub(crate) const fn sctlr_el2_mask(features: Features) -> u64 {
    let mut mask = SCTLR_EL2_BASE;
    if features.has(Feature::Lse2) {
        mask |= SCTLR_EL2_NAA;
    }
    if features.has(Feature::PAuth) {
        mask |= SCTLR_EL2_PAUTH;
    }
    if features.has(Feature::Bti) {
        mask |= SCTLR_EL2_BT;
    }
    if features.has(Feature::Ssbs) {
        mask |= SCTLR_EL2_DSSBS;
    }
    mask
}

// ----- CPTR_EL2 -----

/// `CPTR_EL2.TFP`: SIMD&FP at EL0, EL1 and EL2 traps to EL2.
pub(crate) const CPTR_TFP: u64 = 1 << 10;
/// `CPTR_EL2.TSM` (FEAT_SME; RES1 without): SME at EL0, EL1 and EL2 traps
/// to EL2.
pub(crate) const CPTR_TSM: u64 = 1 << 12;
/// `CPTR_EL2.TCPAC`: EL1's accesses to `CPACR_EL1` trap to EL2.
pub(crate) const CPTR_TCPAC: u64 = 1 << 31;
/// `CPTR_EL2`'s RES1 bits every processor has: 7:0, 9 and 13, and 8
/// (`TZ`, RES1 without FEAT_SVE, which this processor never has).
const CPTR_EL2_RES1_BASE: u64 = 0x23FF;

/// The `CPTR_EL2` bits a processor implementing `features` keeps.
pub(crate) const fn cptr_el2_mask(features: Features) -> u64 {
    if features.has(Feature::Sme) {
        CPTR_TCPAC | CPTR_TFP | CPTR_TSM
    } else {
        CPTR_TCPAC | CPTR_TFP
    }
}

/// `CPTR_EL2`'s RES1 bits on a processor implementing `features`: the bits
/// every processor has, and `TSM` without FEAT_SME.
pub(crate) const fn cptr_el2_res1(features: Features) -> u64 {
    CPTR_EL2_RES1_BASE
        | if features.has(Feature::Sme) {
            0
        } else {
            CPTR_TSM
        }
}

// ----- MDCR_EL2 -----

/// `MDCR_EL2.TDE`: debug exceptions at EL0 and EL1 are taken to EL2.
pub(crate) const MDCR_TDE: u64 = 1 << 8;
/// `MDCR_EL2.TDA`: EL0's and EL1's accesses to the debug registers trap.
pub(crate) const MDCR_TDA: u64 = 1 << 9;
/// `MDCR_EL2.TDOSA`: EL1's accesses to the OS lock and power-down
/// registers trap.
pub(crate) const MDCR_TDOSA: u64 = 1 << 10;
/// `MDCR_EL2.TDRA`: EL1's accesses to the debug ROM address register,
/// `MDRAR_EL1`, trap (EL0 has no access to it).
pub(crate) const MDCR_TDRA: u64 = 1 << 11;
/// The `MDCR_EL2` bits a processor without a PMU, SPE, trace or FEAT_FGT
/// keeps: the four debug controls.
pub(crate) const MDCR_EL2_MASK: u64 = MDCR_TDE | MDCR_TDA | MDCR_TDOSA | MDCR_TDRA;

// ----- the EL2 translation regime and stage 2 -----

/// `TCR_EL2`'s bits for `E2H` = 0 every processor with EL2 keeps: `T0SZ`,
/// `IRGN0`, `ORGN0`, `SH0`, `TG0`, `PS`, `TBI`. `DS` (bit 32) is FEAT_LPA2's.
const TCR_EL2_BASE: u64 = 0x0017_FF3F;
/// `TCR_EL2.HPD` (FEAT_HPDS): hierarchical permissions off.
pub(crate) const TCR_EL2_HPD: u64 = 1 << 24;
/// `TCR_EL2.TBI`: top byte ignored.
pub(crate) const TCR_EL2_TBI: u64 = 1 << 20;
/// `TCR_EL2.TBID` (FEAT_PAuth): `TBI` for data addresses only.
pub(crate) const TCR_EL2_TBID: u64 = 1 << 29;
/// `TCR_EL2`'s RES1 bits: 23 and 31.
pub(crate) const TCR_EL2_RES1: u64 = 0x8080_0000;

/// The `TCR_EL2` bits a processor implementing `features` keeps.
pub(crate) const fn tcr_el2_mask(features: Features) -> u64 {
    let mut mask = TCR_EL2_BASE;
    if features.has(Feature::Hpds) {
        mask |= TCR_EL2_HPD;
    }
    if features.has(Feature::PAuth) {
        mask |= TCR_EL2_TBID;
    }
    mask
}

/// A translation table base register's `BADDR` (bits 47:1) and, with
/// FEAT_TTCNP, `CnP` (bit 0).
pub(crate) const fn ttbr_baddr_mask(features: Features) -> u64 {
    0x0000_FFFF_FFFF_FFFE | if features.has(Feature::Ttcnp) { 1 } else { 0 }
}

/// `VTCR_EL2`'s bits every processor with EL2 keeps: `T0SZ`, `SL0`,
/// `IRGN0`, `ORGN0`, `SH0`, `TG0`, `PS`.
pub(crate) const VTCR_EL2_MASK: u64 = 0x0007_FFFF;
/// `VTCR_EL2`'s RES1 bit: 31.
pub(crate) const VTCR_EL2_RES1: u64 = 1 << 31;
/// `VTTBR_EL2.VMID` with 8-bit VMIDs (no FEAT_VMID16): bits 55:48.
pub(crate) const VTTBR_VMID8: u64 = 0x00FF_0000_0000_0000;

// ----- the other EL2 registers -----

/// `HPFAR_EL2.FIPA` without FEAT_LPA: bits 39:4, IPA bits 47:12 (bits
/// 47:40 RES0).
pub(crate) const HPFAR_FIPA: u64 = 0x0000_00FF_FFFF_FFF0;
/// `VMPIDR_EL2`'s fields: `Aff3`, `U`, `MT`, `Aff2`, `Aff1`, `Aff0`.
pub(crate) const VMPIDR_MASK: u64 = 0x0000_00FF_41FF_FFFF;
/// `VMPIDR_EL2`'s RES1 bit: 31.
pub(crate) const VMPIDR_RES1: u64 = 1 << 31;
/// `VPIDR_EL2`: the `MIDR_EL1` value EL1 reads, bits 31:0.
const VPIDR_MASK: u64 = 0xFFFF_FFFF;
/// `VBAR_EL2`: the vector base, bits 63:11.
const VBAR_MASK: u64 = !0x7FF;
/// `ESR_EL2`: `EC`, `IL` and `ISS`, bits 31:0; `ISS2` is RES0 without the
/// features that report in it.
const ESR_MASK: u64 = 0xFFFF_FFFF;

// ----- the EL2 view of the generic timer -----

/// `CNTHCTL_EL2.EL1PCTEN` (`E2H` = 0): 0 traps EL0's and EL1's reads of the
/// physical count to EL2.
pub(crate) const CNTHCTL_EL1PCTEN: u64 = 1 << 0;
/// `CNTHCTL_EL2.EL1PCEN` (`E2H` = 0): 0 traps EL0's and EL1's accesses to
/// the EL1 physical timer to EL2.
pub(crate) const CNTHCTL_EL1PCEN: u64 = 1 << 1;
/// `CNTHCTL_EL2.EL1TVT` (FEAT_ECV): EL0's and EL1's accesses to the
/// virtual timer trap to EL2.
pub(crate) const CNTHCTL_EL1TVT: u64 = 1 << 13;
/// `CNTHCTL_EL2.EL1TVCT` (FEAT_ECV): EL0's and EL1's reads of the virtual
/// count trap to EL2.
pub(crate) const CNTHCTL_EL1TVCT: u64 = 1 << 14;
/// `CNTHCTL_EL2`'s fields every processor with EL2 has (`E2H` = 0):
/// `EL1PCTEN`, `EL1PCEN`, and the physical count's event stream `EVNTEN`,
/// `EVNTDIR` and `EVNTI` (Arm `TestEventCNTP`), at `CNTKCTL_EL1`'s bits.
const CNTHCTL_BASE: u64 = 0xFF;
/// `CNTHCTL_EL2`'s FEAT_ECV fields: `EL1TVT`, `EL1TVCT`, `EL1NVPCT`,
/// `EL1NVVCT` (whose traps act only with FEAT_NV, which this processor
/// lacks), `EVNTIS`. `ECV` (bit 12) is FEAT_ECV_POFF's.
const CNTHCTL_ECV: u64 = 0x3_E000;
/// `CNTHP_CTL_EL2.ENABLE` and `IMASK`, the fields a write keeps.
const CNTHP_CTL_MASK: u64 = 0b11;
/// `CONTEXTIDR_EL2.PROCID`.
const CONTEXTIDR_MASK: u64 = 0xFFFF_FFFF;
/// `VSESR_EL2` with EL1 in AArch64: `IDS`, `ISS`, the syndrome a virtual
/// SError reports.
pub(crate) const VSESR_MASK: u64 = (1 << 24) | 0x00FF_FFFF;

/// The `CNTHCTL_EL2` bits a processor implementing `features` keeps.
pub(crate) const fn cnthctl_mask(features: Features) -> u64 {
    if features.has(Feature::Ecv) {
        CNTHCTL_BASE | CNTHCTL_ECV
    } else {
        CNTHCTL_BASE
    }
}

/// Whether a processor implementing `features` has `CONTEXTIDR_EL2`:
/// FEAT_VHE or FEAT_Debugv8p2 brings it.
pub(crate) const fn has_contextidr_el2(features: Features) -> bool {
    features.has(Feature::Debugv8p2)
}

/// Declares [`El2Regs`] with [`El2Regs::ABSENT`], [`El2Regs::get`] and
/// `slot`, and [`El2Reg`] with [`El2Reg::ALL`], from one list of each
/// register's variant and field, so that no register can be left out of
/// the registers or a snapshot. The encodings that reach them are
/// `sysreg.rs`'s `el2_encoding`, an exhaustive match on [`El2Reg`] that a
/// register added here does not compile without (`CNTHP_CTL_EL2`, whose
/// `ISTATUS` is computed, is the timer's), and which
/// `tests/unit/sysreg_spec.rs` holds to Arm's accessors.
macro_rules! el2_registers {
    ($($(#[$doc:meta])* $variant:ident => $field:ident,)*) => {
        /// The EL2 registers, as `MSR` leaves them (each through its mask).
        /// Only a processor with EL2 has them ([`El2State`]); without, every
        /// rule reads [`El2Regs::ABSENT`] in their place.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub(crate) struct El2Regs {
            $($(#[$doc])* pub(crate) $field: u64,)*
        }

        /// One EL2 register, named once for what an `MSR` to it keeps and for
        /// its place in a snapshot.
        ///
        /// Closed (R5): every register [`El2Regs`] holds.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum El2Reg {
            $($variant,)*
        }

        impl El2Reg {
            /// Every EL2 register, in snapshot order: the order of the list.
            pub(crate) const ALL: [Self; [$(El2Reg::$variant),*].len()] = [$(Self::$variant),*];
        }

        impl El2Regs {
            /// What every rule reads of the EL2 registers on a processor
            /// without EL2 ([`Cpu::el2`]): zero, so `CNTVOFF_EL2` is 0 (the
            /// virtual count is the physical count), the EL2 timer is off
            /// and no virtual interrupt is raised. A rule that tests a
            /// control of EL1 and EL0 tests [`Levels::EL2`] first
            /// ([`Cpu::hcr_any`] and its siblings), and the rules that act at
            /// EL2 or take an exception there are reached only at a level a
            /// processor without EL2 never enters.
            pub(crate) const ABSENT: Self = Self {
                $($field: 0,)*
            };

            /// The register `reg`'s value.
            pub(crate) const fn get(&self, reg: El2Reg) -> u64 {
                match reg {
                    $(El2Reg::$variant => self.$field,)*
                }
            }

            /// The storage of the register `reg`.
            fn slot(&mut self, reg: El2Reg) -> &mut u64 {
                match reg {
                    $(El2Reg::$variant => &mut self.$field,)*
                }
            }
        }
    };
}

el2_registers! {
    Hcr => hcr,
    Sctlr => sctlr,
    Actlr => actlr,
    Cptr => cptr,
    Hstr => hstr,
    Hacr => hacr,
    Mdcr => mdcr,
    Tcr => tcr,
    Ttbr0 => ttbr0,
    Vtcr => vtcr,
    Vttbr => vttbr,
    Mair => mair,
    Amair => amair,
    Vbar => vbar,
    Esr => esr,
    Far => far,
    Hpfar => hpfar,
    Elr => elr,
    Spsr => spsr,
    Afsr0 => afsr0,
    Afsr1 => afsr1,
    Tpidr => tpidr,
    Vpidr => vpidr,
    Vmpidr => vmpidr,
    /// `SMCR_EL2` (FEAT_SME): the `LEN` field, and `EZT0` with FEAT_SME2.
    Smcr => smcr,
    /// `CNTHCTL_EL2`: EL0's and EL1's access to the counters and timers,
    /// and the EL2 event stream.
    Cnthctl => cnthctl,
    /// `CNTVOFF_EL2`: the virtual count's offset from the physical count.
    Cntvoff => cntvoff,
    /// `CNTHP_CTL_EL2` (`ENABLE` and `IMASK`; `ISTATUS` is computed) and
    /// `CNTHP_CVAL_EL2`: the EL2 physical timer.
    CnthpCtl => cnthp_ctl,
    CnthpCval => cnthp_cval,
    /// `CONTEXTIDR_EL2` (FEAT_Debugv8p2 here, where FEAT_VHE is absent).
    Contextidr => contextidr,
    /// `VDISR_EL2` and `VSESR_EL2` (FEAT_RAS): the virtual SError's deferred
    /// syndrome and the syndrome it is taken with.
    Vdisr => vdisr,
    Vsesr => vsesr,
}

impl El2Reg {
    /// What of `value` an `MSR` to the register keeps on a processor
    /// implementing `features`, whose `SPSR_ELx` bits are `spsr_mask`: the
    /// bits of the fields the register has there, its RES1 bits set and its
    /// RES0 bits clear.
    pub(crate) const fn kept(self, features: Features, spsr_mask: u64, value: u64) -> u64 {
        match self {
            Self::Hcr => (value & hcr_mask(features)) | HCR_RES1,
            Self::Sctlr => (value & sctlr_el2_mask(features)) | SCTLR_EL2_RES1,
            Self::Cptr => (value & cptr_el2_mask(features)) | cptr_el2_res1(features),
            // Every field of `HSTR_EL2` traps an AArch32 access, so with no
            // AArch32 the register is RES0.
            Self::Hstr => 0,
            Self::Mdcr => value & MDCR_EL2_MASK,
            Self::Tcr => (value & tcr_el2_mask(features)) | TCR_EL2_RES1,
            Self::Ttbr0 => value & ttbr_baddr_mask(features),
            Self::Vtcr => (value & VTCR_EL2_MASK) | VTCR_EL2_RES1,
            Self::Vttbr => value & (ttbr_baddr_mask(features) | VTTBR_VMID8),
            Self::Vbar => value & VBAR_MASK,
            Self::Esr => value & ESR_MASK,
            Self::Hpfar => value & HPFAR_FIPA,
            Self::Spsr => value & spsr_mask,
            Self::Vpidr => value & VPIDR_MASK,
            Self::Vmpidr => (value & VMPIDR_MASK) | VMPIDR_RES1,
            Self::Smcr => {
                if features.has(Feature::Sme) {
                    value & smcr_mask(features)
                } else {
                    0
                }
            }
            Self::Cnthctl => value & cnthctl_mask(features),
            Self::CnthpCtl => value & CNTHP_CTL_MASK,
            Self::Contextidr => {
                if has_contextidr_el2(features) {
                    value & CONTEXTIDR_MASK
                } else {
                    0
                }
            }
            Self::Vdisr => {
                if features.has(Feature::Ras) {
                    value & DISR_MASK
                } else {
                    0
                }
            }
            Self::Vsesr => {
                if features.has(Feature::Ras) {
                    value & VSESR_MASK
                } else {
                    0
                }
            }
            // The IMPLEMENTATION DEFINED registers, and those whose every bit
            // is a field: all 64 bits.
            Self::Actlr
            | Self::Hacr
            | Self::Mair
            | Self::Amair
            | Self::Far
            | Self::Elr
            | Self::Afsr0
            | Self::Afsr1
            | Self::Tpidr
            | Self::Cntvoff
            | Self::CnthpCval => value,
        }
    }
}

impl El2Regs {
    /// An `MSR` of `value` to `reg` on a processor implementing `features`
    /// whose `SPSR_ELx` bits are `spsr_mask`: what [`El2Reg::kept`] keeps.
    pub(crate) fn write(&mut self, reg: El2Reg, features: Features, spsr_mask: u64, value: u64) {
        *self.slot(reg) = reg.kept(features, spsr_mask, value);
    }
}

/// What a processor with FEAT_EL2 holds of EL2: `SP_EL2` and the EL2
/// registers, which a snapshot of it carries too. Public as
/// [`El2Storage`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct El2State {
    pub(crate) sp_el2: u64,
    pub(crate) regs: El2Regs,
}

impl El2State {
    /// The state out of reset on a processor implementing `imp`: `SP_EL2`
    /// zero and the registers as [`El2Regs::reset`] leaves them.
    pub(crate) const fn reset(imp: Implementation) -> Self {
        Self {
            sp_el2: 0,
            regs: El2Regs::reset(imp),
        }
    }
}

impl<L: Levels> Cpu<L> {
    /// The EL2 state of a processor implementing FEAT_EL2, for a snapshot:
    /// `SP_EL2`, then each register in [`El2Reg::ALL`] order. A processor
    /// without it writes nothing.
    pub(crate) fn save_el2<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        let Some(state) = self.el2_state.get() else {
            return Ok(());
        };
        w.write_u64(state.sp_el2)?;
        for reg in El2Reg::ALL {
            w.write_u64(state.regs.get(reg))?;
        }
        Ok(())
    }

    /// The inverse of [`Cpu::save_el2`], changing nothing: the state read,
    /// or for a processor without FEAT_EL2 the nothing it holds. A register
    /// holding a value no `MSR` on this processor leaves is refused.
    pub(crate) fn read_el2<R: SnapRead>(&self, r: &mut R) -> Result<L::State, SnapError> {
        let mut state = L::State::keep(El2State::reset(self.imp));
        let Some(read) = state.get_mut() else {
            return Ok(state);
        };
        read.sp_el2 = r.read_u64()?;
        let spsr_mask = self.spsr_mask();
        for reg in El2Reg::ALL {
            let value = r.read_u64()?;
            if reg.kept(self.imp.features, spsr_mask, value) != value {
                return Err(SnapError::Invalid(
                    "snapshot's EL2 register holds bits this processor cannot set",
                ));
            }
            *read.regs.slot(reg) = value;
        }
        Ok(state)
    }
}

impl<L: Levels> Cpu<L> {
    /// Arm `HaveEL(EL2)`: [`Levels::EL2`], a constant.
    #[inline(always)]
    pub(crate) const fn have_el2(&self) -> bool {
        L::EL2
    }

    /// Arm `EL2Enabled()`: with no EL3 and no Secure state, whether the
    /// processor implements EL2, [`Cpu::have_el2`]. The two stay apart so
    /// that each rule names the function its pseudocode calls; they part
    /// only with EL3 or Secure EL2, which this processor lacks.
    #[inline(always)]
    pub(crate) const fn el2_enabled(&self) -> bool {
        self.have_el2()
    }

    /// The EL2 registers as the rules read them: the processor's own with
    /// EL2, and without, [`El2Regs::ABSENT`], a constant the reads fold
    /// into.
    #[inline(always)]
    pub(crate) fn el2(&self) -> &El2Regs {
        match self.el2_state.get() {
            Some(state) => &state.regs,
            None => &El2Regs::ABSENT,
        }
    }

    /// The EL2 registers to write: `None` on a processor without EL2, which
    /// has none.
    #[inline(always)]
    pub(crate) fn el2_mut(&mut self) -> Option<&mut El2Regs> {
        self.el2_state.get_mut().map(|state| &mut state.regs)
    }

    /// `SP_EL2`: 0 on a processor without EL2, which never selects it.
    #[inline(always)]
    pub(crate) fn sp_el2(&self) -> u64 {
        self.el2_state.get().map_or(0, |state| state.sp_el2)
    }

    /// Write `SP_EL2`, which a processor without EL2 does not have: there
    /// the write goes nowhere.
    #[inline(always)]
    pub(crate) fn set_sp_el2(&mut self, value: u64) {
        if let Some(state) = self.el2_state.get_mut() {
            state.sp_el2 = value;
        }
    }

    /// Whether `EL2Enabled()` and any bit of `bits` is set in `HCR_EL2`.
    #[inline(always)]
    pub(crate) fn hcr_any(&self, bits: u64) -> bool {
        L::EL2 && self.el2().hcr & bits != 0
    }

    /// Whether `EL2Enabled()` and a bit of `bits` is clear in `HCR_EL2`:
    /// the controls that trap when 0 (`API`, `APK`, `FIEN`).
    #[inline]
    pub(crate) fn hcr_lacks(&self, bits: u64) -> bool {
        L::EL2 && self.el2().hcr & bits != bits
    }

    /// Whether `EL2Enabled()` and `HCR_EL2.TGE` is set.
    #[inline(always)]
    pub(crate) fn tge(&self) -> bool {
        self.hcr_any(HCR_TGE)
    }

    /// Arm `HighestEL()`: EL2 with FEAT_EL2, EL1 without (no EL3 here).
    #[inline]
    pub(crate) fn highest_el(&self) -> El {
        if self.have_el2() {
            El::El2
        } else {
            El::El1
        }
    }

    /// Whether `EL2Enabled()` and any bit of `bits` is set in `MDCR_EL2`.
    #[inline]
    pub(crate) fn mdcr_any(&self, bits: u64) -> bool {
        L::EL2 && self.el2().mdcr & bits != 0
    }

    /// Whether `EL2Enabled()` and any bit of `bits` is set in `CPTR_EL2`.
    #[inline]
    pub(crate) fn cptr_any(&self, bits: u64) -> bool {
        L::EL2 && self.el2().cptr & bits != 0
    }

    /// Whether `EL2Enabled()` and any bit of `bits` is set in
    /// `CNTHCTL_EL2`.
    #[inline]
    pub(crate) fn cnthctl_any(&self, bits: u64) -> bool {
        L::EL2 && self.el2().cnthctl & bits != 0
    }

    /// Whether `EL2Enabled()` and a bit of `bits` is clear in `CNTHCTL_EL2`:
    /// the controls that trap when 0 (`EL1PCTEN`, `EL1PCEN`).
    #[inline]
    pub(crate) fn cnthctl_lacks(&self, bits: u64) -> bool {
        L::EL2 && self.el2().cnthctl & bits != bits
    }
}

#[cfg(test)]
mod tests {
    use super::super::features::Implementation;
    use super::*;

    /// Each mask names the fields Arm gives the features, and only those,
    /// and the RES1 constants are disjoint from them.
    #[test]
    fn the_masks_keep_exactly_their_features_fields() {
        let a57 = Implementation::CORTEX_A57.with_levels::<WithEl2>().features;
        assert_eq!(hcr_mask(a57), 0x0000_0003_7FFF_7FFF);
        assert_eq!(
            hcr_mask(a57) & (HCR_RES1 | (1 << 15)),
            0,
            "RW and TID0 are not writable"
        );
        assert_eq!(sctlr_el2_mask(a57) & SCTLR_EL2_RES1, 0);
        assert_eq!(cptr_el2_mask(a57) & cptr_el2_res1(a57), 0);
        assert_eq!(cptr_el2_res1(a57), 0x33FF, "TSM is RES1 without FEAT_SME");
        assert_eq!(tcr_el2_mask(a57) & TCR_EL2_RES1, 0);
        assert_eq!(VTCR_EL2_MASK & VTCR_EL2_RES1, 0);
        assert_eq!(VMPIDR_MASK & VMPIDR_RES1, 0);
    }
}
