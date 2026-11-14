//! Nested virtualization (FEAT_NV, FEAT_NV2): a guest hypervisor running at
//! EL1 under `HCR_EL2.NV`, which the hypervisor at EL2 runs as though it
//! were at EL2.
//!
//! Provenance (R7): Arm's machine-readable register data (AARCHMRS
//! `Registers.json`, release 2026-09), each System register's and System
//! instruction's accessor at EL1, whose `EffectiveHCR_EL2_NVx()` branches
//! say what a guest hypervisor's access does and in what order among its
//! other traps; and the shared pseudocode of the A64 ISA XML (release
//! 2026-09): `EffectiveHCR_EL2_NVx`, `NVMem`, `EffectiveBADDR`,
//! `CreateAccDescNV2`, `AArch64_DataAbort`, `SyncExternalAbortTarget`,
//! `AArch64_WatchpointException`, `AArch64_CheckDebug`,
//! `AArch64_ExceptionClass`, `AArch64_FaultSyndrome`,
//! `AArch64_CheckForERetTrap`, `AArch64_TakeException`,
//! `AArch64_IsUnprivAccessPriv`, `AArch64_S1TTWParamsEL10`,
//! `AArch64_S1ApplyOutputPerms`, `AArch64_S1ApplyTablePerms` and
//! `AArch64_S1DirectBasePermissions`.
//!
//! Under `NV` ([`Nested`]) a guest hypervisor at EL1:
//! - reads `CurrentEL` as EL2;
//! - traps to EL2 (EC 0x18) its accesses to EL2's registers, to the
//!   `*_EL12` and `*_EL02` names of EL1's and EL0's, and to EL2's System
//!   instructions (`AT S1E2*`, `AT S12E*`, the EL2 TLB maintenance), and
//!   with FEAT_SPECRES its `CFP`, `DVP` and `CPP RCTX`; its `ERET`,
//!   `ERETAA` and `ERETAB` trap too (EC 0x1A);
//! - under `NV1` too, a guest hypervisor without FEAT_VHE's `E2H`: its
//!   accesses to `SPSR_EL1`, `ELR_EL1` and `VBAR_EL1` trap, an exception it
//!   takes to EL1 reports EL2 in `SPSR_EL1.M[3:2]` (as under `NV` with
//!   `NV2`), its `LDTR` and `STTR` are privileged, and the EL1&0 regime's
//!   stage 1 takes the EL2 regime's permission layout: `AP[1]` and
//!   `APTable[0]` read as 0, bit 54 is the privileged execute-never, bit 53
//!   and `PXNTable`'s place are ignored, and PAN takes no part.
//!
//! Under `NV2` with `NV` (FEAT_NV2), in place of many of those traps, the
//! access is a load or store of 64 bits at `VNCR_EL2` plus the register's
//! offset ([`VncrOffset`], `NVMem`), made as EL2 makes a data access
//! (`CreateAccDescNV2`): in the EL2 or EL2&0 regime, with EL2's
//! permissions, no PAN, and the watchpoints debug exceptions from EL2
//! would raise whatever `PSTATE.D` says; a fault on it is taken to EL2
//! with EC 0x25 (a Data Abort) or 0x35 (a watchpoint) and `ISS.VNCR` set.
//! And `ELR_EL2`, `SPSR_EL2`, `ESR_EL2` and `FAR_EL2` reach their EL1
//! counterparts ([`El1Counterpart`]).
//!
//! The choices the pseudocode leaves to an implementation: `{NV, NV1}`
//! `{0, 1}` acts as `{0, 0}` (`Constraint_NVNV1_00` of
//! `Unpredictable_NVNV1`); "Programming HCR_EL2.<NV,NV2> to '10' behaves as
//! '11'" is FALSE, so `NV2` without `NV` does nothing; and a `VNCR_EL2`
//! whose bits above the virtual address size are not the sign extension of
//! the top one is taken as that extension when it is used
//! (`Constraint_RESS` of `Unpredictable_BADDR_RESS`) and read back as
//! written.

use super::el2::{El2Reg, HCR_NV, HCR_NV1, HCR_NV2};
use super::exec::Fault;
use super::features::{Feature, Features};
use super::mmu::DataAccess;
use super::{Cpu, El, Levels};
use crate::bus::{AccessWidth, PhysBus};

/// Arm `EffectiveHCR_EL2_NVx()`: `HCR_EL2.{NV2, NV1, NV}` as they act at
/// EL1.
///
/// Closed (R5): the values the function returns on a processor whose
/// `NV1` is writable (FEAT_E2H0), as every processor here with FEAT_NV is,
/// with `{NV, NV1}` `{0, 1}` acting as `{0, 0}` and `NV2` acting only with
/// `NV` (the module's choices).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Nested {
    /// `000`: no guest hypervisor.
    Off,
    /// `001`: `NV`, a guest hypervisor with `E2H`.
    Nv,
    /// `011`: `NV` and `NV1`, a guest hypervisor without `E2H`.
    NvNv1,
    /// `101`: `NV` and `NV2`.
    Nv2,
    /// `111`: `NV`, `NV1` and `NV2`.
    Nv2Nv1,
}

impl Nested {
    /// The value `HCR_EL2` `hcr` gives on a processor implementing
    /// `features`, with EL2 enabled.
    #[inline]
    pub(crate) const fn of(hcr: u64, features: Features) -> Self {
        if !features.has(Feature::Nv) || hcr & HCR_NV == 0 {
            return Self::Off;
        }
        let nv1 = hcr & HCR_NV1 != 0;
        let nv2 = features.has(Feature::Nv2) && hcr & HCR_NV2 != 0;
        match (nv2, nv1) {
            (false, false) => Self::Nv,
            (false, true) => Self::NvNv1,
            (true, false) => Self::Nv2,
            (true, true) => Self::Nv2Nv1,
        }
    }

    /// `xx1`: a guest hypervisor runs at EL1.
    #[inline]
    pub(crate) const fn nv(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// `1x1`: its accesses are transformed (FEAT_NV2).
    #[inline]
    pub(crate) const fn nv2(self) -> bool {
        matches!(self, Self::Nv2 | Self::Nv2Nv1)
    }

    /// `x11`: the guest hypervisor has no `E2H` (`walkparams.nv1`, and the
    /// rule of `AArch64_IsUnprivAccessPriv`).
    #[inline]
    pub(crate) const fn nv1(self) -> bool {
        matches!(self, Self::NvNv1 | Self::Nv2Nv1)
    }

    /// `{'x01', '111'}`: an exception taken from EL1 to EL1 reports EL2 in
    /// `SPSR_EL1.M[3:2]` (`AArch64_TakeException`).
    #[inline]
    pub(crate) const fn reports_el2(self) -> bool {
        matches!(self, Self::Nv | Self::Nv2 | Self::Nv2Nv1)
    }
}

/// An offset into the page `VNCR_EL2` names, at which `NVMem` loads and
/// stores a register's 64 bits: a multiple of 8 within the page, never 0
/// (`NVMem`'s `assert offset > 0 && offset MOD 8 == 0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VncrOffset(u16);

impl VncrOffset {
    /// The offset `offset`, the value `Registers.json` gives a register's
    /// `NVMem(offset)`.
    pub(crate) const fn new(offset: u16) -> Self {
        assert!(offset > 0 && offset.is_multiple_of(8) && offset < 0x1000);
        Self(offset)
    }

    /// The offset in bytes.
    pub(crate) const fn get(self) -> u64 {
        self.0 as u64
    }
}

/// When the own name of an EL1 or EL0 register that FEAT_NV2 transforms is
/// a load or store at EL1, once the accessor's other traps have passed.
///
/// Closed (R5): the two conditions `Registers.json` gives such a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Nv2Own {
    /// `111`: with `NV1` alone, the guest hypervisor without `E2H` keeping
    /// its guest's EL1 registers in memory while its own EL2 ones are the
    /// processor's EL1 registers.
    WithNv1,
    /// `1x1`: with `NV2` whatever `NV1` is (`ACTLR_EL1`, the
    /// IMPLEMENTATION DEFINED "IMPLEMENTED_ACTLR_ELx accessor behavior"
    /// being FALSE here, and `MDSCR_EL1`).
    Always,
}

impl Nv2Own {
    /// Whether the name is transformed under `nested`.
    #[inline]
    pub(crate) const fn applies(self, nested: Nested) -> bool {
        match self {
            Self::WithNv1 => matches!(nested, Nested::Nv2Nv1),
            Self::Always => nested.nv2(),
        }
    }
}

/// The EL1 register an EL2 register's name reaches at EL1 under `NV2`.
///
/// Closed (R5): the four `Registers.json` gives (`X(t) = ELR_EL1()` and its
/// siblings under `'1x1'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum El1Counterpart {
    Elr,
    Spsr,
    Esr,
    Far,
}

/// What a guest hypervisor's access at EL1 to the name of an EL2 register
/// does under `NV`: each is UNDEFINED there without it.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum El2NameAtEl1 {
    /// It traps to EL2 (`'xx1'`), with `NV2` too.
    Trapped,
    /// With `NV2` a load or store at the offset (`'1x1'`); it traps
    /// otherwise.
    Memory(VncrOffset),
    /// With `NV2` an access to the EL1 register (`'1x1'`); it traps
    /// otherwise.
    El1(El1Counterpart),
}

/// What a guest hypervisor's access to EL2 register `reg` by its own name
/// does at EL1 under `NV`, as its accessor in `Registers.json` gives it.
/// The one map of the EL2 registers to their outcome, exhaustive (R5): a
/// register added to [`El2Reg`] says what it does here.
pub(crate) const fn el2_register_at_el1(reg: El2Reg) -> El2NameAtEl1 {
    use El2NameAtEl1::{El1, Memory, Trapped};
    const fn at(offset: u16) -> El2NameAtEl1 {
        Memory(VncrOffset::new(offset))
    }
    match reg {
        El2Reg::Vttbr => at(0x020),
        El2Reg::Vtcr => at(0x040),
        El2Reg::Vmpidr => at(0x050),
        El2Reg::Cntvoff => at(0x060),
        El2Reg::Hcr => at(0x078),
        El2Reg::Hstr => at(0x080),
        El2Reg::Vpidr => at(0x088),
        El2Reg::Tpidr => at(0x090),
        El2Reg::Hcrx => at(0x0A0),
        El2Reg::Vncr => at(0x0B0),
        El2Reg::Cntpoff => at(0x1A8),
        El2Reg::Hfgrtr => at(0x1B8),
        El2Reg::Hfgwtr => at(0x1C0),
        El2Reg::Hfgitr => at(0x1C8),
        El2Reg::Hdfgrtr => at(0x1D0),
        El2Reg::Hdfgwtr => at(0x1D8),
        El2Reg::Vdisr => at(0x500),
        El2Reg::Vsesr => at(0x508),
        El2Reg::Elr => El1(El1Counterpart::Elr),
        El2Reg::Spsr => El1(El1Counterpart::Spsr),
        El2Reg::Esr => El1(El1Counterpart::Esr),
        El2Reg::Far => El1(El1Counterpart::Far),
        El2Reg::Sctlr
        | El2Reg::Actlr
        | El2Reg::Cptr
        | El2Reg::Hacr
        | El2Reg::Mdcr
        | El2Reg::Tcr
        | El2Reg::Ttbr0
        | El2Reg::Mair
        | El2Reg::Amair
        | El2Reg::Vbar
        | El2Reg::Hpfar
        | El2Reg::Afsr0
        | El2Reg::Afsr1
        | El2Reg::Smcr
        | El2Reg::Cnthctl
        | El2Reg::CnthpCtl
        | El2Reg::CnthpCval
        | El2Reg::Contextidr
        | El2Reg::Ttbr1
        | El2Reg::CnthvCtl
        | El2Reg::CnthvCval => Trapped,
    }
}

/// `SP_EL1`'s offset: its name is EL2's (`op1` 4).
pub(crate) const VNCR_SP_EL1: VncrOffset = VncrOffset::new(0x240);
/// `SMPRIMAP_EL2`'s offset (FEAT_SME).
pub(crate) const VNCR_SMPRIMAP_EL2: VncrOffset = VncrOffset::new(0x1F8);

/// `ESR_ELx.ISS.VNCR`, bit 13: the Data Abort or watchpoint is of a load or
/// store `VNCR_EL2` made (`AArch64_FaultSyndrome`).
pub(crate) const ISS_VNCR: u32 = 1 << 13;

impl<L: Levels> Cpu<L> {
    /// Arm `EffectiveHCR_EL2_NVx()`: [`Nested::Off`] where EL2 is not
    /// enabled, a constant on a processor without EL2.
    #[inline]
    pub(crate) fn nested(&self) -> Nested {
        if !L::EL2 {
            return Nested::Off;
        }
        Nested::of(self.el2().hcr, self.imp.features)
    }

    /// [`Cpu::nested`] as it acts on an access at `el`: a guest hypervisor
    /// runs at EL1 alone, so [`Nested::Off`] at EL0 and EL2.
    #[inline]
    pub(crate) fn nested_at(&self, el: El) -> Nested {
        match el {
            El::El1 => self.nested(),
            El::El0 | El::El2 => Nested::Off,
        }
    }

    /// The getter of Arm `NVMem(offset)` (FEAT_NV2): the 64 bits a guest
    /// hypervisor's transformed `MRS` at EL1 reads, loaded at `offset` in
    /// the `VNCR_EL2` page ([`Cpu::as_nv2_access`]).
    pub(crate) fn nv2_load<B: PhysBus>(
        &mut self,
        bus: &mut B,
        offset: VncrOffset,
    ) -> Result<u64, Fault> {
        let address = self.vncr_address(offset);
        self.as_nv2_access(|cpu| cpu.read_mem(bus, address, AccessWidth::Dword, DataAccess::NORMAL))
    }

    /// The setter of Arm `NVMem(offset)` (FEAT_NV2): `value`, which a guest
    /// hypervisor's transformed `MSR` at EL1 writes, stored at `offset` in
    /// the `VNCR_EL2` page ([`Cpu::as_nv2_access`]).
    pub(crate) fn nv2_store<B: PhysBus>(
        &mut self,
        bus: &mut B,
        offset: VncrOffset,
        value: u64,
    ) -> Result<(), Fault> {
        let address = self.vncr_address(offset);
        self.as_nv2_access(|cpu| {
            cpu.write_mem(bus, address, AccessWidth::Dword, value, DataAccess::NORMAL)
        })
    }

    /// The data access `access` makes, made as Arm's `CreateAccDescNV2`
    /// describes it: by EL2, with no PAN, and with the watchpoints debug
    /// exceptions from EL2 raise whatever `PSTATE.D` says
    /// (`AArch64_CheckDebug` passes `mask` '0' for it). The translation path
    /// reads the level, PAN and the debug mask an access is made with from
    /// `PSTATE`, so for the access they are EL2, clear and clear, and they
    /// are put back after it; a data access changes no `PSTATE`. Its faults
    /// are the NV2 Data Abort and NV2 watchpoint ([`Fault::nv2`]).
    fn as_nv2_access<T, F: FnOnce(&mut Self) -> Result<T, Fault>>(
        &mut self,
        access: F,
    ) -> Result<T, Fault> {
        let current = self.pstate;
        self.pstate.el = El::El2;
        self.pstate.pan = false;
        self.pstate.d = false;
        let done = access(self);
        self.pstate = current;
        done.map_err(Fault::nv2)
    }

    /// The address `NVMem(offset)` accesses: `offset` above Arm
    /// `EffectiveBADDR(VNCR_EL2.BADDR :: Zeros(12), FALSE)`, the base whose
    /// bits from the virtual address size's top bit up (bit 52 with
    /// FEAT_LVA, 48 without) are that bit's sign extension
    /// (`Constraint_RESS`).
    fn vncr_address(&self, offset: VncrOffset) -> u64 {
        let n = if self.implements(Feature::Lva) {
            53
        } else {
            49
        };
        let shift = 64 - n;
        let base = (((self.el2().vncr << shift) as i64) >> shift) as u64;
        base.wrapping_add(offset.get())
    }
}
