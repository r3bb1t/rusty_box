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
//! `HCR_EL2.E2H` (FEAT_VHE) redefines four registers, `SCTLR_EL2`,
//! `CPTR_EL2`, `TCR_EL2` and `CNTHCTL_EL2` (Arm ARM D8.13.1), each of
//! which has a layout for `E2H` 0 and one for `E2H` 1 ([`E2hLayout`]):
//! each keeps every bit that is a field in either layout as last written,
//! whatever `E2H` was then, as Arm requires of a bit reserved only in some
//! contexts, and every reader takes the fields of the layout `E2H` selects
//! now. A bit that is a field in neither layout reads as the current
//! layout's RES1 value, a write of `HCR_EL2` that changes `E2H` setting it
//! again. Without FEAT_VHE `E2H` is RES0 and the layout is always the
//! `E2H` 0 one. A bit that is RES0 or RES1 in every context, for the
//! features a processor implements, is hardwired to that value, reads
//! returning it and writes ignoring it, one of the two behaviours Arm
//! permits for a reserved bit (the other keeps what is written). The
//! IMPLEMENTATION DEFINED registers (`ACTLR_EL2`, `HACR_EL2`, `AFSR0_EL2`,
//! `AFSR1_EL2`, `AMAIR_EL2`) are the implementation's
//! ([`ImpdefSpace::el2_kept`](super::impdef::ImpdefSpace::el2_kept)): on
//! the Cortex-A57 its TRM's, `ACTLR_EL2`
//! holding the controls of EL1's writes of its own IMPLEMENTATION DEFINED
//! registers and the rest RES0; on the Armv9.2 model all 64 bits,
//! controlling nothing, as their EL1 counterparts do there.

use rusty_box_core::SnapError;
use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};

use super::features::{Feature, Features, Implementation, LeveledImplementation};
use super::fgt::FgtReg;
use super::mmu::{ContextStorage, NoContexts, TlbContexts};
use super::regime::TTB_BADDR;
use super::regs::SysReg;
use super::state::spsr_mask_of;
use super::sysreg::{DISR_MASK, smcr_mask};
use super::{Cpu, El, id, reset, sctlr};

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
    /// What a level set stores of EL2, and the tables of contexts its TLB
    /// tags what it holds with.
    pub trait Sealed {
        type State: super::El2Storage;
        type Contexts: super::ContextStorage;
    }
}

impl sealed::Sealed for NoEl2 {
    type Contexts = NoContexts;
    type State = NoEl2State;
}

impl sealed::Sealed for WithEl2 {
    type Contexts = TlbContexts;
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
    /// The EL2 registers out of reset ([`reset`] says why each takes its
    /// value). The Cortex-A57 TRM's (DDI 0488H, Table 4-13): `SCTLR_EL2`
    /// its RES1 bits and `SA`, `CPTR_EL2` its RES1 bits, `HCR_EL2`,
    /// `ACTLR_EL2`, `HSTR_EL2`, `HACR_EL2`, `ESR_EL2` and `HPFAR_EL2` zero
    /// (`HCR_EL2.RW` RES1, there being no AArch32), `VPIDR_EL2` and
    /// `VMPIDR_EL2` the `MIDR_EL1` and `MPIDR_EL1` they stand in for, so EL1
    /// reads the processor's own identity until EL2 programs another, and
    /// `CNTHCTL_EL2` with `EL1PCTEN` and `EL1PCEN` set (Table 9-2), so EL1's
    /// accesses to the physical counter and timer do not trap until EL2
    /// asks, and `MDCR_EL2` 0x6, `HPMN` the PMU's six event counters
    /// (Table 4-13), so EL1 reaches every one; with another PMU `HPMN` its
    /// count, and without one zero; the controls zero beside their RES1
    /// bits, the fine-grained trap registers (FEAT_FGT) and `HCRX_EL2`
    /// (FEAT_HCX) among them, whose every field Arm resets to 0 where EL2
    /// is the highest implemented level (Arm ARM D24.2, each field's reset
    /// behaviour); and the values the
    /// board's fill, bits 63:32 of `TTBR0_EL2`, `VTTBR_EL2`, `FAR_EL2` and
    /// `VBAR_EL2` zero as the TRM has them. Each is what an `MSR` keeps of
    /// it ([`El2Reg::kept`]): which bits are RES1 depends on the features
    /// `imp` implements, the RES1 bits of a feature the processor implements
    /// being that feature's field, zero out of reset.
    pub(crate) const fn reset<L: Levels>(imp: LeveledImplementation<L>) -> Self {
        let imp = imp.get();
        let features = imp.features;
        let spsr_mask = spsr_mask_of(features);
        // `HCR_EL2.E2H` resets to 0: the registers it redefines take their
        // `E2H` 0 layout.
        let layout = E2hLayout::E2h0;
        Self {
            hcr: HCR_RES1,
            sctlr: SCTLR_EL2_RES1 | sctlr::SA,
            actlr: 0,
            cptr: cptr_el2_res1(features, layout),
            hstr: 0,
            hacr: 0,
            mdcr: El2Reg::Mdcr.kept(features, spsr_mask, layout, imp.pmu.counters.get() as u64),
            tcr: TCR_EL2_RES1,
            ttbr0: El2Reg::Ttbr0.kept(
                features,
                spsr_mask,
                layout,
                reset::low_word_of(SysReg::TTBR0_EL2),
            ),
            vtcr: VTCR_EL2_RES1,
            vttbr: El2Reg::Vttbr.kept(
                features,
                spsr_mask,
                layout,
                reset::low_word_of(SysReg::VTTBR_EL2),
            ),
            mair: 0,
            amair: 0,
            vbar: El2Reg::Vbar.kept(
                features,
                spsr_mask,
                layout,
                reset::low_word_of(SysReg::VBAR_EL2),
            ),
            esr: 0,
            far: reset::low_word_of(SysReg::FAR_EL2),
            hpfar: 0,
            elr: reset::value_of(SysReg::ELR_EL2),
            spsr: El2Reg::Spsr.kept(
                features,
                spsr_mask,
                layout,
                reset::value_of(SysReg::SPSR_EL2),
            ),
            afsr0: 0,
            afsr1: 0,
            tpidr: reset::value_of(SysReg::TPIDR_EL2),
            vpidr: imp.midr,
            vmpidr: id::MPIDR_EL1,
            smcr: 0,
            cnthctl: CNTHCTL_EL1PCTEN | CNTHCTL_EL1PCEN,
            cntvoff: 0,
            cntpoff: 0,
            cnthp_ctl: 0,
            cnthp_cval: reset::value_of(SysReg::CNTHP_CVAL_EL2),
            contextidr: El2Reg::Contextidr.kept(
                features,
                spsr_mask,
                layout,
                reset::low_word_of(reset::CONTEXTIDR_EL2),
            ),
            vdisr: 0,
            vsesr: 0,
            ttbr1: El2Reg::Ttbr1.kept(
                features,
                spsr_mask,
                layout,
                reset::low_word_of(SysReg::TTBR1_EL2),
            ),
            cnthv_ctl: 0,
            cnthv_cval: El2Reg::CnthvCval.kept(
                features,
                spsr_mask,
                layout,
                reset::value_of(SysReg::CNTHV_CVAL_EL2),
            ),
            hfgrtr: 0,
            hfgwtr: 0,
            hfgitr: 0,
            hdfgrtr: 0,
            hdfgwtr: 0,
            hcrx: 0,
            vncr: El2Reg::Vncr.kept(
                features,
                spsr_mask,
                layout,
                reset::value_of(SysReg::VNCR_EL2),
            ),
        }
    }
}

// ----- HCR_EL2 -----

/// `HCR_EL2.VM`: stage 2 translation of the EL1&0 regime
/// (`AArch64_NSS2TTWParams`, `walkparams.vm`).
pub(crate) const HCR_VM: u64 = 1 << 0;
/// `HCR_EL2.PTW`: a stage 1 walk's descriptor read that stage 2 maps to
/// Device memory takes a stage 2 Permission fault (`walkparams.ptw`, which
/// `HCR_EL2.TGE` clears).
pub(crate) const HCR_PTW: u64 = 1 << 2;
/// `HCR_EL2.FMO`, `IMO`, `AMO`: physical FIQ, IRQ and SError routing to EL2.
pub(crate) const HCR_FMO: u64 = 1 << 3;
pub(crate) const HCR_IMO: u64 = 1 << 4;
pub(crate) const HCR_AMO: u64 = 1 << 5;
/// `HCR_EL2.VF`, `VI`, `VSE`: a virtual FIQ, IRQ and SError pending for EL1
/// and EL0, whose physical ones `FMO`, `IMO` and `AMO` route to EL2.
pub(crate) const HCR_VF: u64 = 1 << 6;
pub(crate) const HCR_VI: u64 = 1 << 7;
pub(crate) const HCR_VSE: u64 = 1 << 8;
/// `HCR_EL2.DC`: the EL1&0 regime's stage 1 is disabled, its data accesses
/// are to Normal Write-Back memory (`AArch64_S1Enabled`,
/// `AArch64_S1DisabledOutput`), and stage 2 translates them as under `VM`
/// (`AArch64_NSS2TTWParams`).
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
/// `HCR_EL2.CD`: stage 2 makes the EL1&0 regime's Normal data memory
/// Non-cacheable (`S2DCacheEnabled`, `AArch64_S2TreatAsNormalNC`).
pub(crate) const HCR_CD: u64 = 1 << 32;
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
/// `HCR_EL2.E2H` (FEAT_VHE): EL2 runs in the EL2&0 translation regime, its
/// registers take their `E2H` 1 layouts ([`E2hLayout`]), and the EL1
/// register names reach EL2's at EL2 (Arm `EffectiveHCR_EL2_E2H`, writable
/// as the processor has FEAT_E2H0).
pub(crate) const HCR_E2H: u64 = 1 << 34;
/// `HCR_EL2.NV` (FEAT_NV): EL1 runs a guest hypervisor, whose accesses to
/// EL2's registers and instructions trap to EL2 (`nv.rs`).
pub(crate) const HCR_NV: u64 = 1 << 42;
/// `HCR_EL2.NV1` (FEAT_NV): with `NV`, a guest hypervisor without FEAT_VHE's
/// `E2H` (`nv.rs`).
pub(crate) const HCR_NV1: u64 = 1 << 43;
/// `HCR_EL2.AT` (FEAT_NV): EL1's `AT S1E0*` and `AT S1E1*` trap to EL2.
pub(crate) const HCR_AT: u64 = 1 << 44;
/// `HCR_EL2.NV2` (FEAT_NV2): with `NV`, the guest hypervisor's accesses to
/// many System registers are loads and stores of the page `VNCR_EL2`
/// names (`nv.rs`).
pub(crate) const HCR_NV2: u64 = 1 << 45;
/// `HCR_EL2.FIEN` (FEAT_RASv1p1): EL1's accesses to the error injection
/// registers do not trap.
pub(crate) const HCR_FIEN: u64 = 1 << 47;
/// `HCR_EL2.FWB` (FEAT_S2FWB): stage 2's `MemAttr` combines with stage 1's
/// attributes by `AArch64_S2ApplyFWBMemAttrs`, where it can force
/// Write-Back, instead of `S2CombineS1MemAttrs`.
pub(crate) const HCR_FWB: u64 = 1 << 46;
/// `HCR_EL2.TID4` (FEAT_EVT): EL1's accesses to `CCSIDR_EL1`, `CLIDR_EL1`
/// and `CSSELR_EL1` trap (`CCSIDR2_EL1` too, which a processor without
/// FEAT_CCIDX does not have).
pub(crate) const HCR_TID4: u64 = 1 << 49;
/// `HCR_EL2.TICAB` (FEAT_EVT): EL1's `IC IALLUIS` traps.
pub(crate) const HCR_TICAB: u64 = 1 << 50;
/// `HCR_EL2.TOCU` (FEAT_EVT): EL0's and EL1's cache maintenance to the
/// Point of Unification (`IC IVAU`, `IC IALLU`, `DC CVAU`) traps.
pub(crate) const HCR_TOCU: u64 = 1 << 52;
/// `HCR_EL2.TTLBIS` (FEAT_EVT2): EL1's Inner Shareable TLB maintenance
/// instructions trap.
pub(crate) const HCR_TTLBIS: u64 = 1 << 54;
/// `HCR_EL2.TTLBOS` (FEAT_EVT2): EL1's Outer Shareable TLB maintenance
/// instructions trap.
pub(crate) const HCR_TTLBOS: u64 = 1 << 55;

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
    if features.has(Feature::Vhe) {
        mask |= HCR_E2H;
    }
    if features.has(Feature::S2Fwb) {
        mask |= HCR_FWB;
    }
    if features.has(Feature::Evt) {
        mask |= HCR_TID4 | HCR_TICAB | HCR_TOCU;
    }
    if features.has(Feature::Evt2) {
        mask |= HCR_TTLBIS | HCR_TTLBOS;
    }
    if features.has(Feature::Nv) {
        mask |= HCR_NV | HCR_NV1 | HCR_AT;
    }
    if features.has(Feature::Nv2) {
        mask |= HCR_NV2;
    }
    mask
}

// ----- HCRX_EL2 -----

/// `HCRX_EL2.FnXS` (FEAT_XS): EL1's TLB maintenance instructions without
/// the nXS qualifier behave as with it (Arm `ExcludeXS`), which removes
/// the translations of memory with the XS attribute too here, as the nXS
/// forms do (`tlbi.rs`).
pub(crate) const HCRX_FNXS: u64 = 1 << 3;
/// `HCRX_EL2.FGTnXS` (FEAT_XS): the `HFGITR_EL2` traps of the EL1 TLB
/// maintenance instructions do not trap their nXS forms.
pub(crate) const HCRX_FGTNXS: u64 = 1 << 4;
/// `HCRX_EL2.FNB`, bit 36, which Arm's data gives every processor with
/// FEAT_HCX and only `AArch64_EffectiveOpScope` and
/// `AArch64_EffectiveBroadcast` read, with FEAT_TLBID, which this
/// processor lacks: kept, and acting on nothing.
const HCRX_FNB: u64 = 1 << 36;

/// The `HCRX_EL2` bits a processor implementing `features` (with
/// FEAT_HCX) keeps: `FNB`, and with FEAT_XS `FnXS` and `FGTnXS`. Its other
/// fields belong to features this processor lacks, and `SMPME`, FEAT_SME's,
/// is RES0 where `SMIDR_EL1.SMPS` is 0, as here (no streaming execution
/// priorities).
pub(crate) const fn hcrx_mask(features: Features) -> u64 {
    if !features.has(Feature::Hcx) {
        return 0;
    }
    let xs = if features.has(Feature::Xs) {
        HCRX_FNXS | HCRX_FGTNXS
    } else {
        0
    };
    HCRX_FNB | xs
}

/// Which layout a register `HCR_EL2.E2H` redefines has (the `E2H0` and
/// `E2H1` fieldsets of Arm's register data): `SCTLR_EL2`, `CPTR_EL2`,
/// `TCR_EL2` and `CNTHCTL_EL2` (Arm ARM D8.13.1).
///
/// Closed (R5): `E2H`'s two values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum E2hLayout {
    /// `EffectiveHCR_EL2_E2H()` 0: the EL2 regime's, and every processor
    /// without FEAT_VHE.
    E2h0,
    /// `EffectiveHCR_EL2_E2H()` 1 (FEAT_VHE): the EL2&0 regime's, each
    /// register in the layout of its EL1 counterpart.
    E2h1,
}

impl E2hLayout {
    /// The layout `HCR_EL2` `hcr` selects: `E2H`, which only a processor
    /// with FEAT_VHE keeps.
    #[inline(always)]
    pub(crate) const fn of(hcr: u64) -> Self {
        if hcr & HCR_E2H != 0 {
            Self::E2h1
        } else {
            Self::E2h0
        }
    }
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
/// `SCTLR_EL2.BT0` (FEAT_BTI, `E2H` 1): `PACIASP` and `PACIBSP` are not a
/// branch target for `BR` from a guarded page at EL0 in the EL2&0 regime.
pub(crate) const SCTLR_EL2_BT0: u64 = 1 << 35;
/// The fields `E2H` 1 gives `SCTLR_EL2` on every processor with FEAT_VHE,
/// at `SCTLR_EL1`'s bits, which apply to EL0 under `HCR_EL2.TGE`: `SA0`,
/// `CP15BEN` and `SED` (fields Arm's data gives the layout without
/// AArch32 at EL0 too), `DZE`, `UCT`, `nTWI`, `nTWE`, `SPAN` and `UCI`.
const SCTLR_EL2_E2H1: u64 = sctlr::SA0
    | (1 << 5)
    | (1 << 8)
    | sctlr::DZE
    | sctlr::UCT
    | sctlr::NTWI
    | sctlr::NTWE
    | sctlr::SPAN
    | sctlr::UCI;
/// `SCTLR_EL2`'s RES1 bits with `E2H` 1, no FEAT_ExS, no AArch32 at EL0, no
/// FEAT_CSV2_2 and no FEAT_LSMAOC: 7 (`ITD`), 11 (`EOS`), 20 (`TSCXT`), 22
/// (`EIS`), 28 (`nTLSMD`), 29 (`LSMAOE`).
pub(crate) const SCTLR_EL2_E2H1_RES1: u64 = 0x3050_0880;

/// The `SCTLR_EL2` bits a processor implementing `features` keeps in
/// `layout`.
pub(crate) const fn sctlr_el2_mask(features: Features, layout: E2hLayout) -> u64 {
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
    if let E2hLayout::E2h1 = layout {
        mask |= SCTLR_EL2_E2H1;
        if features.has(Feature::SpecRes) {
            mask |= sctlr::ENRCTX;
        }
        if features.has(Feature::Bti) {
            mask |= SCTLR_EL2_BT0;
        }
        if features.has(Feature::Pan3) {
            mask |= sctlr::EPAN;
        }
        if features.has(Feature::Sme) {
            mask |= sctlr::ENTP2;
        }
    }
    mask
}

/// `SCTLR_EL2`'s RES1 bits in `layout`.
pub(crate) const fn sctlr_el2_res1(layout: E2hLayout) -> u64 {
    match layout {
        E2hLayout::E2h0 => SCTLR_EL2_RES1,
        E2hLayout::E2h1 => SCTLR_EL2_E2H1_RES1,
    }
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

/// `CPTR_EL2.FPEN` (`E2H` 1): SIMD&FP at EL0 in the EL2&0 regime and at EL2
/// traps to EL2 but where it is 0b11, at EL0 under `HCR_EL2.TGE` where it
/// is 0b01 too (Arm `AArch64_CheckFPAdvSIMDTrap`). `CPACR_EL1.FPEN`'s bits.
pub(crate) const CPTR_FPEN: u64 = 3 << 20;
/// `CPTR_EL2.SMEN` (FEAT_SME, `E2H` 1): SME, as `FPEN` SIMD&FP (Arm
/// `CheckSMEAccess`). `CPACR_EL1.SMEN`'s bits.
pub(crate) const CPTR_SMEN: u64 = 3 << 24;

/// The `CPTR_EL2` bits a processor implementing `features` keeps in
/// `layout`: `TCPAC` in both, with `E2H` 0 `TFP` and with FEAT_SME `TSM`,
/// with `E2H` 1 `FPEN` and with FEAT_SME `SMEN`.
pub(crate) const fn cptr_el2_mask(features: Features, layout: E2hLayout) -> u64 {
    let sme = features.has(Feature::Sme);
    match layout {
        E2hLayout::E2h0 if sme => CPTR_TCPAC | CPTR_TFP | CPTR_TSM,
        E2hLayout::E2h0 => CPTR_TCPAC | CPTR_TFP,
        E2hLayout::E2h1 if sme => CPTR_TCPAC | CPTR_FPEN | CPTR_SMEN,
        E2hLayout::E2h1 => CPTR_TCPAC | CPTR_FPEN,
    }
}

/// `CPTR_EL2`'s RES1 bits on a processor implementing `features` in
/// `layout`: with `E2H` 0 the bits every processor has, and `TSM` without
/// FEAT_SME; with `E2H` 1 none.
pub(crate) const fn cptr_el2_res1(features: Features, layout: E2hLayout) -> u64 {
    match layout {
        E2hLayout::E2h0 if features.has(Feature::Sme) => CPTR_EL2_RES1_BASE,
        E2hLayout::E2h0 => CPTR_EL2_RES1_BASE | CPTR_TSM,
        E2hLayout::E2h1 => 0,
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
/// `MDCR_EL2.HPMN` (FEAT_PMUv3): EL0 and EL1 reach the event counters
/// below it; the rest are reserved for EL2.
pub(crate) const MDCR_HPMN: u64 = 0x1F;
/// `MDCR_EL2.TPMCR` (FEAT_PMUv3): EL0's and EL1's accesses to `PMCR_EL0`
/// trap.
pub(crate) const MDCR_TPMCR: u64 = 1 << 5;
/// `MDCR_EL2.TPM` (FEAT_PMUv3): EL0's and EL1's accesses to the PMU
/// registers trap.
pub(crate) const MDCR_TPM: u64 = 1 << 6;
/// `MDCR_EL2.HPME` (FEAT_PMUv3): the global enable of the event counters
/// reserved for EL2.
pub(crate) const MDCR_HPME: u64 = 1 << 7;
/// `MDCR_EL2.HPMD` (FEAT_PMUv3p1): the event counters not reserved for
/// EL2 do not count at EL2.
pub(crate) const MDCR_HPMD: u64 = 1 << 17;
/// `MDCR_EL2.HCCD` (FEAT_PMUv3p5): the cycle counter does not count at
/// EL2.
pub(crate) const MDCR_HCCD: u64 = 1 << 23;
/// `MDCR_EL2.HLP` (FEAT_PMUv3p5): the event counters reserved for EL2
/// overflow at bit 63 rather than 31.
pub(crate) const MDCR_HLP: u64 = 1 << 26;
/// `MDCR_EL2.HPMFZO` (FEAT_PMUv3p7): the event counters reserved for EL2
/// stop while one of them has overflowed.
pub(crate) const MDCR_HPMFZO: u64 = 1 << 29;
/// `MDCR_EL2.TDCC` (FEAT_FGT): EL0's and EL1's accesses to the Debug
/// Communications Channel registers trap.
pub(crate) const MDCR_TDCC: u64 = 1 << 27;

/// The `MDCR_EL2` bits a processor implementing `features` keeps (Arm's
/// `MDCR_EL2` fieldset, with no SPE, trace, FEAT_MTPMU or FEAT_EBEP): the
/// four debug controls, with FEAT_FGT `TDCC`, and with FEAT_PMUv3 `HPMN`,
/// `TPMCR`, `TPM` and `HPME`, with FEAT_PMUv3p1 `HPMD`, with FEAT_PMUv3p5
/// `HCCD` and `HLP`, and with FEAT_PMUv3p7 `HPMFZO`.
pub(crate) const fn mdcr_el2_mask(features: Features) -> u64 {
    let mut mask = MDCR_TDE | MDCR_TDA | MDCR_TDOSA | MDCR_TDRA;
    if features.has(Feature::Fgt) {
        mask |= MDCR_TDCC;
    }
    if features.has(Feature::PmuV3) {
        mask |= MDCR_HPMN | MDCR_TPMCR | MDCR_TPM | MDCR_HPME;
    }
    if features.has(Feature::PmuV3p1) {
        mask |= MDCR_HPMD;
    }
    if features.has(Feature::PmuV3p5) {
        mask |= MDCR_HCCD | MDCR_HLP;
    }
    if features.has(Feature::PmuV3p7) {
        mask |= MDCR_HPMFZO;
    }
    mask
}

// ----- the EL2 translation regime and stage 2 -----

/// `TCR_EL2`'s bits for `E2H` = 0 every processor with EL2 keeps: `T0SZ`,
/// `IRGN0`, `ORGN0`, `SH0`, `TG0`, `PS`, `TBI`.
const TCR_EL2_BASE: u64 = 0x0017_FF3F;
/// `TCR_EL2.HPD` (FEAT_HPDS): hierarchical permissions off.
pub(crate) const TCR_EL2_HPD: u64 = 1 << 24;
/// `TCR_EL2.TBI`: top byte ignored.
pub(crate) const TCR_EL2_TBI: u64 = 1 << 20;
/// `TCR_EL2.TBID` (FEAT_PAuth): `TBI` for data addresses only.
pub(crate) const TCR_EL2_TBID: u64 = 1 << 29;
/// `TCR_EL2.DS` (FEAT_LPA2): 52-bit addresses with the 4 KiB and 16 KiB
/// granules.
pub(crate) const TCR_EL2_DS: u64 = 1 << 32;
/// `TCR_EL2.HA` (FEAT_HAF) and `TCR_EL2.HD` (FEAT_HAFDBS) with `E2H` = 0,
/// and `VTCR_EL2.HA` and `VTCR_EL2.HD` at the same bits: the hardware update
/// of the Access flag and of dirty state.
pub(crate) const TCR_EL2_HA: u64 = 1 << 21;
pub(crate) const TCR_EL2_HD: u64 = 1 << 22;
pub(crate) const VTCR_EL2_HA: u64 = TCR_EL2_HA;
pub(crate) const VTCR_EL2_HD: u64 = TCR_EL2_HD;
/// `TCR_EL2`'s RES1 bits: 23 and 31.
pub(crate) const TCR_EL2_RES1: u64 = 0x8080_0000;

/// `HA` with FEAT_HAF and `HD` with FEAT_HAFDBS, the bits of `TCR_EL2` and
/// `VTCR_EL2` alike that a processor implementing `features` keeps of them.
const fn hw_update_mask(features: Features) -> u64 {
    let ha = if features.has(Feature::Haf) {
        TCR_EL2_HA
    } else {
        0
    };
    let hd = if features.has(Feature::Hafdbs) {
        TCR_EL2_HD
    } else {
        0
    };
    ha | hd
}

/// `TCR_EL2`'s bits for `E2H` 1, `TCR_EL1`'s layout, every processor with
/// FEAT_VHE keeps: `T0SZ`, `EPD0`, `IRGN0`, `ORGN0`, `SH0`, `TG0`, `T1SZ`,
/// `A1`, `EPD1`, `IRGN1`, `ORGN1`, `SH1`, `TG1`, `IPS`, `AS` (FEAT_ASID16:
/// `ID_AA64MMFR0_EL1.ASIDBits` is 16 bits on every processor here),
/// `TBI0` and `TBI1`.
const TCR_EL2_E2H1_BASE: u64 = 0x0000_0077_FFFF_FFBF;
/// `TCR_EL2`'s `E2H` 1 fields of the features: `HA` (FEAT_HAF), `HD`
/// (FEAT_HAFDBS), `HPD0` and `HPD1` (FEAT_HPDS), `TBID0` and `TBID1`
/// (FEAT_PAuth), `E0PD0` and `E0PD1` (FEAT_E0PD), `DS` (FEAT_LPA2), each
/// at `TCR_EL1`'s bit.
const TCR_EL2_E2H1_HA: u64 = 1 << 39;
const TCR_EL2_E2H1_HD: u64 = 1 << 40;
const TCR_EL2_E2H1_HPD: u64 = (1 << 41) | (1 << 42);
const TCR_EL2_E2H1_TBID: u64 = (1 << 51) | (1 << 52);
const TCR_EL2_E2H1_E0PD: u64 = (1 << 55) | (1 << 56);
const TCR_EL2_E2H1_DS: u64 = 1 << 59;

/// The `TCR_EL2` bits a processor implementing `features` keeps in
/// `layout`.
pub(crate) const fn tcr_el2_mask(features: Features, layout: E2hLayout) -> u64 {
    /// `bits` where `features` holds `feature`.
    const fn with(features: Features, feature: Feature, bits: u64) -> u64 {
        if features.has(feature) { bits } else { 0 }
    }
    match layout {
        E2hLayout::E2h0 => {
            TCR_EL2_BASE
                | hw_update_mask(features)
                | with(features, Feature::Hpds, TCR_EL2_HPD)
                | with(features, Feature::PAuth, TCR_EL2_TBID)
                | with(features, Feature::Lpa2, TCR_EL2_DS)
        }
        E2hLayout::E2h1 => {
            TCR_EL2_E2H1_BASE
                | with(features, Feature::Haf, TCR_EL2_E2H1_HA)
                | with(features, Feature::Hafdbs, TCR_EL2_E2H1_HD)
                | with(features, Feature::Hpds, TCR_EL2_E2H1_HPD)
                | with(features, Feature::PAuth, TCR_EL2_E2H1_TBID)
                | with(features, Feature::E0pd, TCR_EL2_E2H1_E0PD)
                | with(features, Feature::Lpa2, TCR_EL2_E2H1_DS)
        }
    }
}

/// `TCR_EL2`'s RES1 bits in `layout`: 23 and 31 with `E2H` 0, none with
/// `E2H` 1.
pub(crate) const fn tcr_el2_res1(layout: E2hLayout) -> u64 {
    match layout {
        E2hLayout::E2h0 => TCR_EL2_RES1,
        E2hLayout::E2h1 => 0,
    }
}

/// A translation table base register's `BADDR` (bits 47:1) and, with
/// FEAT_TTCNP, `CnP` (bit 0).
pub(crate) const fn ttbr_baddr_mask(features: Features) -> u64 {
    TTB_BADDR | if features.has(Feature::Ttcnp) { 1 } else { 0 }
}

/// A translation table base register's `ASID`, bits 63:48: `TTBR0_EL2`'s
/// and `TTBR1_EL2`'s with FEAT_VHE.
pub(crate) const TTBR_ASID: u64 = 0xFFFF_0000_0000_0000;

/// The `TTBR0_EL2` and `TTBR1_EL2` bits a processor implementing
/// `features` keeps: [`ttbr_baddr_mask`], and with FEAT_VHE `ASID`, in
/// either layout of `HCR_EL2.E2H` (the register's fieldsets turn on the
/// feature alone).
pub(crate) const fn ttbr_el2_mask(features: Features) -> u64 {
    ttbr_baddr_mask(features)
        | if features.has(Feature::Vhe) {
            TTBR_ASID
        } else {
            0
        }
}

/// `VTCR_EL2`'s bits every processor with EL2 keeps: `T0SZ`, `SL0`,
/// `IRGN0`, `ORGN0`, `SH0`, `TG0`, `PS`.
const VTCR_EL2_BASE: u64 = 0x0007_FFFF;
/// `VTCR_EL2.DS` (FEAT_LPA2): 52-bit addresses with the 4 KiB and 16 KiB
/// granules.
pub(crate) const VTCR_EL2_DS: u64 = 1 << 32;
/// `VTCR_EL2.SL2` (FEAT_LPA2): with `DS` and the 4 KiB granule, the top bit
/// of the start level's encoding (`SL2:SL0`).
pub(crate) const VTCR_EL2_SL2: u64 = 1 << 33;

/// The `VTCR_EL2` bits a processor implementing `features` keeps.
const fn vtcr_el2_mask(features: Features) -> u64 {
    let lpa2 = if features.has(Feature::Lpa2) {
        VTCR_EL2_DS | VTCR_EL2_SL2
    } else {
        0
    };
    VTCR_EL2_BASE | lpa2 | hw_update_mask(features)
}

/// `VTCR_EL2`'s RES1 bit: 31.
pub(crate) const VTCR_EL2_RES1: u64 = 1 << 31;
/// `VTTBR_EL2.VMID` with 8-bit VMIDs (no FEAT_VMID16): bits 55:48.
pub(crate) const VTTBR_VMID8: u64 = 0x00FF_0000_0000_0000;

// ----- the other EL2 registers -----

/// `HPFAR_EL2.FIPA` on a processor implementing `features`: bits 39:4, IPA
/// bits 47:12, and with FEAT_LPA bits 43:4, IPA bits 51:12 (the bits above
/// RES0).
pub(crate) const fn hpfar_fipa(features: Features) -> u64 {
    if features.has(Feature::Lpa) {
        0x0000_0FFF_FFFF_FFF0
    } else {
        0x0000_00FF_FFFF_FFF0
    }
}
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
/// `CNTHCTL_EL2.EL1PCTEN` and `EL1PTEN` with `E2H` 1: as `EL1PCTEN` and
/// `EL1PCEN` with `E2H` 0, at bits 10 and 11.
pub(crate) const CNTHCTL_E2H1_EL1PCTEN: u64 = 1 << 10;
pub(crate) const CNTHCTL_E2H1_EL1PTEN: u64 = 1 << 11;
/// `CNTHCTL_EL2`'s fields every processor with FEAT_VHE has with `E2H` 1:
/// the event stream's, the EL1 enables, and `EL0PCTEN`, `EL0VCTEN`,
/// `EL0VTEN` and `EL0PTEN`, whose 0 traps EL0's accesses in the host to the
/// counters and timers to EL2, at `CNTKCTL_EL1`'s bits; bits 11:0.
const CNTHCTL_E2H1_BASE: u64 = 0xFFF;
/// `CNTHCTL_EL2`'s FEAT_ECV fields: `EL1TVT`, `EL1TVCT`, `EL1NVPCT`,
/// `EL1NVVCT` (whose traps act only with FEAT_NV2, on a guest hypervisor's
/// `*_EL02` timer accesses), `EVNTIS`.
const CNTHCTL_ECV_FIELDS: u64 = 0x3_E000;
/// `CNTHCTL_EL2.EL1NVPCT` and `EL1NVVCT` (FEAT_ECV): with `HCR_EL2.{NV2,
/// NV1, NV}` {1, 0, 1}, EL1's accesses to the EL1 physical and virtual
/// timers by their `*_EL02` names trap to EL2, where they would otherwise
/// be loads and stores of the `VNCR_EL2` page.
pub(crate) const CNTHCTL_EL1NVPCT: u64 = 1 << 15;
pub(crate) const CNTHCTL_EL1NVVCT: u64 = 1 << 16;
/// `CNTHCTL_EL2.ECV` (FEAT_ECV_POFF), bit 12: `CNTPOFF_EL2` offsets the
/// physical count EL0 and EL1 read and the EL1 physical timer compares
/// (Arm `PhysicalOffsetIsValid`, `AArch64_CheckTimerConditions`). Only a
/// processor with FEAT_ECV_POFF keeps it.
pub(crate) const CNTHCTL_ECV_POFF: u64 = 1 << 12;
/// `CNTHP_CTL_EL2.ENABLE` and `IMASK`, the fields a write keeps, and
/// `CNTHV_CTL_EL2`'s alike.
const CNTHP_CTL_MASK: u64 = 0b11;
/// `CONTEXTIDR_EL2.PROCID`.
const CONTEXTIDR_MASK: u64 = 0xFFFF_FFFF;
/// `VNCR_EL2.BADDR` (FEAT_NV2), bits 63:12, every bit above the virtual
/// address size held as written: a load or store through it sign-extends
/// the address from the size's top bit (`EffectiveBADDR`, `nv.rs`).
pub(crate) const VNCR_BADDR: u64 = !0xFFF;
/// `VSESR_EL2` with EL1 in AArch64: `IDS`, `ISS`, the syndrome a virtual
/// SError reports.
pub(crate) const VSESR_MASK: u64 = (1 << 24) | 0x00FF_FFFF;

/// The `CNTHCTL_EL2` bits a processor implementing `features` keeps in
/// `layout`: bits 7:0 with `E2H` 0, 11:0 with `E2H` 1, and FEAT_ECV's and
/// FEAT_ECV_POFF's in both.
pub(crate) const fn cnthctl_mask(features: Features, layout: E2hLayout) -> u64 {
    let ecv = if features.has(Feature::Ecv) {
        CNTHCTL_ECV_FIELDS
    } else {
        0
    };
    let poff = if features.has(Feature::EcvPoff) {
        CNTHCTL_ECV_POFF
    } else {
        0
    };
    let base = match layout {
        E2hLayout::E2h0 => CNTHCTL_BASE,
        E2hLayout::E2h1 => CNTHCTL_E2H1_BASE,
    };
    base | ecv | poff
}

/// Whether a processor implementing `features` has `CONTEXTIDR_EL2`: Arm's
/// data gives it to one with FEAT_Debugv8p1, which FEAT_VHE needs.
pub(crate) const fn has_contextidr_el2(features: Features) -> bool {
    features.has(Feature::Debugv8p1)
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
    /// `CNTPOFF_EL2` (FEAT_ECV_POFF): the physical count's offset at EL0
    /// and EL1 while `CNTHCTL_EL2.ECV` is set.
    Cntpoff => cntpoff,
    /// `CNTHP_CTL_EL2` (`ENABLE` and `IMASK`; `ISTATUS` is computed) and
    /// `CNTHP_CVAL_EL2`: the EL2 physical timer.
    CnthpCtl => cnthp_ctl,
    CnthpCval => cnthp_cval,
    /// `CONTEXTIDR_EL2` (FEAT_Debugv8p1, which FEAT_VHE needs).
    Contextidr => contextidr,
    /// `VDISR_EL2` and `VSESR_EL2` (FEAT_RAS): the virtual SError's deferred
    /// syndrome and the syndrome it is taken with.
    Vdisr => vdisr,
    Vsesr => vsesr,
    /// `TTBR1_EL2` (FEAT_VHE): the upper range's table base and ASID in the
    /// EL2&0 regime.
    Ttbr1 => ttbr1,
    /// `CNTHV_CTL_EL2` (`ENABLE` and `IMASK`; `ISTATUS` is computed) and
    /// `CNTHV_CVAL_EL2` (FEAT_VHE): the EL2 virtual timer.
    CnthvCtl => cnthv_ctl,
    CnthvCval => cnthv_cval,
    /// `HFGRTR_EL2`, `HFGWTR_EL2`, `HFGITR_EL2`, `HDFGRTR_EL2` and
    /// `HDFGWTR_EL2` (FEAT_FGT): the fine-grained traps (`fgt.rs`).
    Hfgrtr => hfgrtr,
    Hfgwtr => hfgwtr,
    Hfgitr => hfgitr,
    Hdfgrtr => hdfgrtr,
    Hdfgwtr => hdfgwtr,
    /// `HCRX_EL2` (FEAT_HCX): the extended hypervisor configuration.
    Hcrx => hcrx,
    /// `VNCR_EL2` (FEAT_NV2): the base of the page a guest hypervisor's
    /// transformed System register accesses load and store (`nv.rs`).
    Vncr => vncr,
}

/// What of `value` a register `HCR_EL2.E2H` redefines keeps, `e2h0_fields`
/// and `e2h1_fields` being its fields in each layout and `res1` the RES1
/// bits of the layout of the time: every bit that is a field in a layout
/// the processor has (the `E2H` 1 one only with FEAT_VHE) as written, a
/// read returning the value last written whatever the register's use when
/// it was written (Arm ARM Glossary, "If a bit is RES0 only in some
/// contexts" and its RES1 twin); a bit that is a field in neither, `res1`'s
/// value, as Arm permits for a bit reserved in every context (writes
/// ignored, and for one RES0 in a layout and RES1 in the other the read
/// UNKNOWN).
const fn redefined_kept(
    features: Features,
    e2h0_fields: u64,
    e2h1_fields: u64,
    res1: u64,
    value: u64,
) -> u64 {
    let fields = if features.has(Feature::Vhe) {
        e2h0_fields | e2h1_fields
    } else {
        e2h0_fields
    };
    (value & fields) | (res1 & !fields)
}

impl El2Reg {
    /// The registers `HCR_EL2.E2H` redefines, which take the layout it
    /// selects ([`E2hLayout`]).
    const REDEFINED: [Self; 4] = [Self::Sctlr, Self::Cptr, Self::Tcr, Self::Cnthctl];

    /// What of `value` an `MSR` to the register keeps on a processor
    /// implementing `features`, whose `SPSR_ELx` bits are `spsr_mask`, in
    /// `layout` (which only the registers `HCR_EL2.E2H` redefines read): the
    /// bits of the fields the register has there, its RES1 bits set and its
    /// RES0 bits clear.
    pub(crate) const fn kept(
        self,
        features: Features,
        spsr_mask: u64,
        layout: E2hLayout,
        value: u64,
    ) -> u64 {
        match self {
            Self::Hcr => (value & hcr_mask(features)) | HCR_RES1,
            Self::Sctlr => redefined_kept(
                features,
                sctlr_el2_mask(features, E2hLayout::E2h0),
                sctlr_el2_mask(features, E2hLayout::E2h1),
                sctlr_el2_res1(layout),
                value,
            ),
            Self::Cptr => redefined_kept(
                features,
                cptr_el2_mask(features, E2hLayout::E2h0),
                cptr_el2_mask(features, E2hLayout::E2h1),
                cptr_el2_res1(features, layout),
                value,
            ),
            // Every field of `HSTR_EL2` traps an AArch32 access, so with no
            // AArch32 the register is RES0.
            Self::Hstr => 0,
            Self::Mdcr => value & mdcr_el2_mask(features),
            Self::Tcr => redefined_kept(
                features,
                tcr_el2_mask(features, E2hLayout::E2h0),
                tcr_el2_mask(features, E2hLayout::E2h1),
                tcr_el2_res1(layout),
                value,
            ),
            Self::Ttbr0 => value & ttbr_el2_mask(features),
            Self::Ttbr1 => {
                if features.has(Feature::Vhe) {
                    value & ttbr_el2_mask(features)
                } else {
                    0
                }
            }
            Self::Vtcr => (value & vtcr_el2_mask(features)) | VTCR_EL2_RES1,
            Self::Vttbr => value & (ttbr_baddr_mask(features) | VTTBR_VMID8),
            Self::Vbar => value & VBAR_MASK,
            Self::Esr => value & ESR_MASK,
            Self::Hpfar => value & hpfar_fipa(features),
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
            Self::Cnthctl => redefined_kept(
                features,
                cnthctl_mask(features, E2hLayout::E2h0),
                cnthctl_mask(features, E2hLayout::E2h1),
                0,
                value,
            ),
            // `CNTPOFF_EL2.PO`, all 64 bits, with FEAT_ECV_POFF.
            Self::Cntpoff => {
                if features.has(Feature::EcvPoff) {
                    value
                } else {
                    0
                }
            }
            Self::CnthpCtl => value & CNTHP_CTL_MASK,
            // The EL2 virtual timer's, with FEAT_VHE.
            Self::CnthvCtl | Self::CnthvCval => {
                if !features.has(Feature::Vhe) {
                    0
                } else if let Self::CnthvCtl = self {
                    value & CNTHP_CTL_MASK
                } else {
                    value
                }
            }
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
            Self::Hfgrtr => value & FgtReg::Hfgrtr.mask(features),
            Self::Hfgwtr => value & FgtReg::Hfgwtr.mask(features),
            Self::Hfgitr => value & FgtReg::Hfgitr.mask(features),
            Self::Hdfgrtr => value & FgtReg::Hdfgrtr.mask(features),
            Self::Hdfgwtr => value & FgtReg::Hdfgwtr.mask(features),
            Self::Hcrx => value & hcrx_mask(features),
            Self::Vncr => {
                if features.has(Feature::Nv2) {
                    value & VNCR_BADDR
                } else {
                    0
                }
            }
            // The IMPLEMENTATION DEFINED registers, whose bits the
            // implementation decides (`El2Reg::held`), and those whose every
            // bit is a field: all 64 bits.
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

    /// Whether the timers read the register: what
    /// [`Cpu::el2_timer_inputs`] gathers, `HCR_EL2`, whose `E2H` and `TGE`
    /// decide whether the EL1 physical timer takes FEAT_ECV_POFF's offset
    /// (`!ELIsInHost(EL0)`), and `CNTHP_CTL_EL2` and `CNTHV_CTL_EL2`, which
    /// their own arms write. Only a write of one of these can move a timer.
    /// Exhaustive (R5): a register added to the list says whether it is
    /// one.
    pub(crate) const fn feeds_timers(self) -> bool {
        match self {
            Self::Hcr
            | Self::Cnthctl
            | Self::Cntvoff
            | Self::Cntpoff
            | Self::CnthpCtl
            | Self::CnthpCval
            | Self::CnthvCtl
            | Self::CnthvCval => true,
            Self::Sctlr
            | Self::Actlr
            | Self::Cptr
            | Self::Hstr
            | Self::Hacr
            | Self::Mdcr
            | Self::Tcr
            | Self::Ttbr0
            | Self::Vtcr
            | Self::Vttbr
            | Self::Mair
            | Self::Amair
            | Self::Vbar
            | Self::Esr
            | Self::Far
            | Self::Hpfar
            | Self::Elr
            | Self::Spsr
            | Self::Afsr0
            | Self::Afsr1
            | Self::Tpidr
            | Self::Vpidr
            | Self::Vmpidr
            | Self::Smcr
            | Self::Contextidr
            | Self::Vdisr
            | Self::Vsesr
            | Self::Ttbr1
            | Self::Hfgrtr
            | Self::Hfgwtr
            | Self::Hfgitr
            | Self::Hdfgrtr
            | Self::Hdfgwtr
            | Self::Hcrx
            | Self::Vncr => false,
        }
    }
}

// A snapshot reads `HCR_EL2` before the registers whose layout it selects
// ([`Cpu::read_el2`]).
const _: () = assert!(matches!(El2Reg::ALL[0], El2Reg::Hcr));

impl El2Reg {
    /// What of `value` an `MSR` to the register keeps on a processor
    /// implementing `imp`, whose `SPSR_ELx` bits are `spsr_mask`, in
    /// `layout`: what the architecture keeps ([`El2Reg::kept`]) and the
    /// fields the processor's GIC CPU interface adds
    /// ([`FgtReg::gic_fields`]), of an IMPLEMENTATION DEFINED register what
    /// the implementation keeps of that
    /// ([`ImpdefSpace::el2_kept`](super::impdef::ImpdefSpace::el2_kept)).
    pub(crate) const fn held(
        self,
        imp: Implementation,
        spsr_mask: u64,
        layout: E2hLayout,
        value: u64,
    ) -> u64 {
        let gic = match self.fine_grained() {
            Some(fgt) => fgt.gic_fields(imp),
            None => 0,
        };
        let kept = self.kept(imp.features, spsr_mask, layout, value) | (value & gic);
        imp.impdef.el2_kept(self, kept)
    }

    /// The fine-grained trap register this is, if it is one.
    const fn fine_grained(self) -> Option<FgtReg> {
        match self {
            Self::Hfgrtr => Some(FgtReg::Hfgrtr),
            Self::Hfgwtr => Some(FgtReg::Hfgwtr),
            Self::Hfgitr => Some(FgtReg::Hfgitr),
            Self::Hdfgrtr => Some(FgtReg::Hdfgrtr),
            Self::Hdfgwtr => Some(FgtReg::Hdfgwtr),
            Self::Hcr
            | Self::Sctlr
            | Self::Actlr
            | Self::Cptr
            | Self::Hstr
            | Self::Hacr
            | Self::Mdcr
            | Self::Tcr
            | Self::Ttbr0
            | Self::Vtcr
            | Self::Vttbr
            | Self::Mair
            | Self::Amair
            | Self::Vbar
            | Self::Esr
            | Self::Far
            | Self::Hpfar
            | Self::Elr
            | Self::Spsr
            | Self::Afsr0
            | Self::Afsr1
            | Self::Tpidr
            | Self::Vpidr
            | Self::Vmpidr
            | Self::Smcr
            | Self::Cnthctl
            | Self::Cntvoff
            | Self::Cntpoff
            | Self::CnthpCtl
            | Self::CnthpCval
            | Self::Contextidr
            | Self::Vdisr
            | Self::Vsesr
            | Self::Ttbr1
            | Self::CnthvCtl
            | Self::CnthvCval
            | Self::Hcrx
            | Self::Vncr => None,
        }
    }
}

impl El2Regs {
    /// The layout `HCR_EL2.E2H` gives the registers it redefines now.
    #[inline(always)]
    pub(crate) const fn layout(&self) -> E2hLayout {
        E2hLayout::of(self.hcr)
    }

    /// An `MSR` of `value` to `reg` on a processor implementing `imp` whose
    /// `SPSR_ELx` bits are `spsr_mask`: what [`El2Reg::held`] keeps in the
    /// layout of the time. A write of `HCR_EL2` that changes `E2H` sets the
    /// bits of each register it redefines that are a field in neither
    /// layout to the new layout's RES1 value, and keeps every field.
    pub(crate) fn write(&mut self, reg: El2Reg, imp: Implementation, spsr_mask: u64, value: u64) {
        let layout = self.layout();
        *self.slot(reg) = reg.held(imp, spsr_mask, layout, value);
        let now = self.layout();
        if now != layout {
            for redefined in El2Reg::REDEFINED {
                let held = redefined.held(imp, spsr_mask, now, self.get(redefined));
                *self.slot(redefined) = held;
            }
        }
    }
}

/// What a processor with FEAT_EL2 holds of EL2: `SP_EL2` and the EL2
/// registers, which a snapshot of it carries too, and the interrupt
/// controller's virtual interrupt inputs, which a snapshot leaves out: the
/// machine drives them again from its restored GIC. Public as
/// [`El2Storage`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct El2State {
    pub(crate) sp_el2: u64,
    pub(crate) regs: El2Regs,
    /// The interrupt controller's virtual interrupt inputs (Arm
    /// `VirtualIRQPending()` and `VirtualFIQPending()`, IMPLEMENTATION
    /// DEFINED there), which a GIC with the Virtualization Extensions
    /// drives. Inputs, as the physical lines are, so a snapshot holds
    /// neither: the machine drives them again.
    pub(crate) virtual_lines: VirtualLines,
}

/// The virtual IRQ and FIQ inputs from the interrupt controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualLines {
    pub irq: bool,
    pub fiq: bool,
}

impl VirtualLines {
    pub const NONE: Self = Self {
        irq: false,
        fiq: false,
    };
}

impl El2State {
    /// The state out of reset on a processor implementing `imp`: `SP_EL2`
    /// the board's fill ([`reset::SP_EL2`]), the registers as
    /// [`El2Regs::reset`] leaves them and neither virtual input asserted.
    pub(crate) const fn reset<L: Levels>(imp: LeveledImplementation<L>) -> Self {
        Self {
            sp_el2: reset::SP_EL2,
            regs: El2Regs::reset(imp),
            virtual_lines: VirtualLines::NONE,
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
    /// holding a value no `MSR` on this processor leaves is refused, each
    /// register `HCR_EL2.E2H` redefines in the layout of the `HCR_EL2` read
    /// before it ([`El2Reg::ALL`] holds `HCR_EL2` first).
    pub(crate) fn read_el2<R: SnapRead>(&self, r: &mut R) -> Result<L::State, SnapError> {
        let mut state = L::State::keep(El2State::reset(self.imp));
        let Some(read) = state.get_mut() else {
            return Ok(state);
        };
        read.sp_el2 = r.read_u64()?;
        let spsr_mask = self.spsr_mask();
        for reg in El2Reg::ALL {
            let value = r.read_u64()?;
            if reg.held(self.imp.get(), spsr_mask, read.regs.layout(), value) != value {
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

    /// `HCR_EL2` as the controls of the levels below EL2 read it: 0 where
    /// `EL2Enabled()` is FALSE, on a processor without EL2.
    #[inline(always)]
    pub(crate) fn hcr_el2(&self) -> u64 {
        if L::EL2 { self.el2().hcr } else { 0 }
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

    /// Arm `ELIsInHost(EL2)`: `EL2Enabled()` and `EffectiveHCR_EL2_E2H()`
    /// 1, which only a processor with FEAT_VHE keeps. A constant false
    /// without EL2.
    #[inline(always)]
    pub(crate) fn e2h(&self) -> bool {
        self.hcr_any(HCR_E2H)
    }

    /// Arm `ELIsInHost(EL0)`: `EL2Enabled()` and `HCR_EL2.{E2H, TGE}`
    /// {1, 1}, under which EL0 runs in the EL2&0 regime. A constant false
    /// without EL2.
    #[inline(always)]
    pub(crate) fn el0_in_host(&self) -> bool {
        L::EL2 && self.el2().hcr & (HCR_E2H | HCR_TGE) == HCR_E2H | HCR_TGE
    }

    /// Arm `ELIsInHost(el)`: EL2 under `E2H` ([`Cpu::e2h`]), EL0 under
    /// `E2H` and `TGE` ([`Cpu::el0_in_host`]), and EL1 never.
    #[inline(always)]
    pub(crate) fn el_is_in_host(&self, el: El) -> bool {
        match el {
            El::El2 => self.e2h(),
            El::El0 => self.el0_in_host(),
            El::El1 => false,
        }
    }

    /// Arm `IsInHost()`: [`Cpu::el_is_in_host`] of the current level.
    #[inline(always)]
    pub(crate) fn in_host(&self) -> bool {
        self.el_is_in_host(self.pstate.el)
    }

    /// Arm `HighestEL()`: EL2 with FEAT_EL2, EL1 without (no EL3 here).
    #[inline]
    pub(crate) fn highest_el(&self) -> El {
        if self.have_el2() { El::El2 } else { El::El1 }
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
    /// and the RES1 constants are disjoint from them, in each layout of
    /// `HCR_EL2.E2H`.
    #[test]
    fn the_masks_keep_exactly_their_features_fields() {
        use E2hLayout::{E2h0, E2h1};
        let a57 = Implementation::CORTEX_A57.with_levels::<WithEl2>().features;
        assert_eq!(hcr_mask(a57), 0x0000_0003_7FFF_7FFF);
        assert_eq!(
            hcr_mask(a57) & (HCR_RES1 | (1 << 15) | HCR_E2H),
            0,
            "RW, TID0 and, without FEAT_VHE, E2H are not writable"
        );
        assert_eq!(sctlr_el2_mask(a57, E2h0) & SCTLR_EL2_RES1, 0);
        assert_eq!(cptr_el2_mask(a57, E2h0) & cptr_el2_res1(a57, E2h0), 0);
        assert_eq!(
            cptr_el2_res1(a57, E2h0),
            0x33FF,
            "TSM is RES1 without FEAT_SME"
        );
        assert_eq!(tcr_el2_mask(a57, E2h0) & TCR_EL2_RES1, 0);
        let every = Implementation::EVERY_FEATURE
            .with_levels::<WithEl2>()
            .features;
        assert_eq!(hcr_mask(every) & HCR_E2H, HCR_E2H, "E2H with FEAT_VHE");
        for layout in [E2h0, E2h1] {
            assert_eq!(
                sctlr_el2_mask(every, layout) & sctlr_el2_res1(layout),
                0,
                "{layout:?}"
            );
            assert_eq!(
                cptr_el2_mask(every, layout) & cptr_el2_res1(every, layout),
                0,
                "{layout:?}"
            );
            assert_eq!(
                tcr_el2_mask(every, layout) & tcr_el2_res1(layout),
                0,
                "{layout:?}"
            );
        }
        // `E2H` 1 moves the controls of `CPTR_EL2` and `CNTHCTL_EL2` to
        // `CPACR_EL1`'s and `CNTKCTL_EL1`'s bits.
        assert_eq!(
            cptr_el2_mask(every, E2h1),
            CPTR_TCPAC | CPTR_FPEN | CPTR_SMEN
        );
        assert_eq!(cnthctl_mask(every, E2h1) & 0xFFF, 0xFFF);
        assert_eq!(cnthctl_mask(every, E2h0) & 0xF00, 0);
        assert_eq!(vtcr_el2_mask(every) & VTCR_EL2_RES1, 0);
        // HA and HD are RES0 on the Cortex-A57, which has neither FEAT_HAF
        // nor FEAT_HAFDBS, and kept where the processor has both.
        let hw = TCR_EL2_HA | TCR_EL2_HD;
        assert_eq!(tcr_el2_mask(a57, E2h0) & hw, 0);
        assert_eq!(vtcr_el2_mask(a57) & (VTCR_EL2_HA | VTCR_EL2_HD), 0);
        assert_eq!(tcr_el2_mask(every, E2h0) & hw, hw);
        assert_eq!(
            vtcr_el2_mask(every) & (VTCR_EL2_HA | VTCR_EL2_HD),
            VTCR_EL2_HA | VTCR_EL2_HD
        );
        let haf_alone = Implementation::every_feature_but(&[Feature::Hafdbs])
            .with_levels::<WithEl2>()
            .features;
        assert_eq!(tcr_el2_mask(haf_alone, E2h0) & hw, TCR_EL2_HA);
        assert_eq!(VMPIDR_MASK & VMPIDR_RES1, 0);
        assert_eq!(
            ttbr_el2_mask(a57) & TTBR_ASID,
            0,
            "no ASID without FEAT_VHE"
        );
        assert_eq!(ttbr_el2_mask(every) & TTBR_ASID, TTBR_ASID);
    }

    /// A write of `HCR_EL2` that changes `E2H` leaves each register it
    /// redefines holding what its new layout keeps: `CPTR_EL2`'s RES1 bits
    /// of the `E2H` 0 layout are clear with `E2H` 1, and set again with
    /// `E2H` 0; `TCR_EL2`'s `T1SZ`, a field of the `E2H` 1 layout, is the
    /// `E2H` 0 layout's `PS` and `TBI` where they overlap.
    #[test]
    fn a_change_of_e2h_keeps_the_fields_of_either_layout() {
        let imp = Implementation::EVERY_FEATURE.with_levels::<WithEl2>().get();
        let spsr_mask = spsr_mask_of(imp.features);
        let mut regs = El2Regs::reset(imp.with_levels::<WithEl2>());
        assert_eq!(regs.layout(), E2hLayout::E2h0);
        assert_eq!(regs.cptr & CPTR_EL2_RES1_BASE, CPTR_EL2_RES1_BASE);
        // FPEN, an E2H 1 field in RES0 bits of the E2H 0 layout, written
        // with E2H 0.
        regs.write(El2Reg::Cptr, imp, spsr_mask, CPTR_EL2_RES1_BASE | CPTR_FPEN);
        assert_eq!(regs.cptr & CPTR_FPEN, CPTR_FPEN, "read back as written");
        regs.write(El2Reg::Hcr, imp, spsr_mask, HCR_E2H);
        assert_eq!(regs.layout(), E2hLayout::E2h1);
        assert_eq!(
            regs.cptr, CPTR_FPEN,
            "FPEN kept; the bits that are a field in neither layout RES0 with E2H 1"
        );
        regs.write(El2Reg::Tcr, imp, spsr_mask, 0x3F << 16);
        assert_eq!(regs.tcr, 0x3F << 16, "T1SZ");
        // nTWI cleared with E2H 1, a RES1 bit of the E2H 0 layout.
        regs.write(El2Reg::Sctlr, imp, spsr_mask, regs.sctlr & !sctlr::NTWI);
        regs.write(El2Reg::Hcr, imp, spsr_mask, 0);
        assert_eq!(
            regs.cptr,
            cptr_el2_res1(imp.features, E2hLayout::E2h0) | CPTR_FPEN,
            "the E2H 0 RES1 bits again, FPEN still kept"
        );
        assert_eq!(
            regs.tcr & (0x3F << 16),
            0x3F << 16,
            "T1SZ kept, bit 19 with it"
        );
        regs.write(El2Reg::Hcr, imp, spsr_mask, HCR_E2H);
        assert_eq!(
            regs.sctlr & sctlr::NTWI,
            0,
            "nTWI as written, after the round trip"
        );
        assert_eq!(regs.cptr, CPTR_FPEN);
    }
}
