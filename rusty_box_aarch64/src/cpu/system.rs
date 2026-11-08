//! System instructions (Arm ARM C5): hints, barriers, PSTATE access, cache
//! and TLB maintenance, address translation, `MRS`/`MSR`, and the
//! exception-generating instructions — including the PSCI calls (Arm DEN
//! 0022E) and the SMC Calling Convention's Arm Architecture Calls (DEN
//! 0028F) that the board's firmware answers on `HVC` without EL2 and on
//! `SMC` with it, as QEMU's `virt` does (its device tree's `method`).

use super::el2::{CPTR_TFP, HCR_HCD, HCR_TDZ, HCR_TPCP, HCR_TPU, HCR_TSC, HCR_TSW, HCR_TTLB};
use super::exception::{Exception, Route, SyndromeClass};
use super::exec::{bit, bits, ExecResult, Fault, Flow, StopCause};
use super::features::Feature;
use super::mmu::{AtCheck, AtStages, CacheMaintenance};
use super::sme::SmeTrap;
use super::sysreg::{SysRegError, SysRegKey, CPACR_FPEN};
use super::tlbi::TlbiForm;
use super::wait::Wait;
use super::{sctlr, Cpu, El, Levels};
use crate::bus::PhysBus;

/// PSCI function identifiers and return codes (Arm DEN 0022D §5.1, Table 6).
pub(super) mod psci {
    pub(super) const VERSION: u32 = 0x8400_0000;
    pub(super) const CPU_SUSPEND_32: u32 = 0x8400_0001;
    pub(super) const CPU_SUSPEND_64: u32 = 0xC400_0001;
    pub(super) const CPU_OFF: u32 = 0x8400_0002;
    pub(super) const CPU_ON_32: u32 = 0x8400_0003;
    pub(super) const CPU_ON_64: u32 = 0xC400_0003;
    pub(super) const AFFINITY_INFO_32: u32 = 0x8400_0004;
    pub(super) const AFFINITY_INFO_64: u32 = 0xC400_0004;
    pub(super) const MIGRATE_INFO_TYPE: u32 = 0x8400_0006;
    pub(super) const SYSTEM_OFF: u32 = 0x8400_0008;
    pub(super) const SYSTEM_RESET: u32 = 0x8400_0009;
    pub(super) const FEATURES: u32 = 0x8400_000A;

    /// Function identifier bit 30: the SMC64/HVC64 calling convention, whose
    /// arguments are X registers; clear, SMC32/HVC32, whose arguments are W
    /// registers (Arm DEN 0028B Table 2-1, §2.6, §2.7).
    pub(super) const SMC64: u32 = 1 << 30;

    pub(super) const SUCCESS: u64 = 0;
    pub(super) const NOT_SUPPORTED: u64 = (-1i64) as u64;
    pub(super) const INVALID_PARAMETERS: u64 = (-2i64) as u64;
    pub(super) const ALREADY_ON: u64 = (-4i64) as u64;
    pub(super) const INVALID_ADDRESS: u64 = (-9i64) as u64;
    /// `AFFINITY_INFO`: at least one core of the instance is on.
    pub(super) const ON: u64 = 0;

    /// PSCI 1.2, the version DEN 0022E specifies: every function Table 17
    /// makes mandatory for it, and `MIGRATE_INFO_TYPE`, of the optional
    /// ones.
    pub(crate) const IMPLEMENTED_VERSION: u64 = 0x0001_0002;
    /// "Trusted OS is not present or does not require migration".
    pub(super) const MIGRATE_NOT_REQUIRED: u64 = 2;

    /// The processor's affinity value, the form a PSCI call names it in
    /// (DEN 0022D §5.1.4 `target_cpu`): `MPIDR_EL1`'s Aff3 (bits 39:32) and
    /// Aff2..Aff0 (bits 23:0), every other bit zero.
    pub(super) const AFFINITY: u64 = crate::cpu::id::MPIDR_EL1 & 0xFF_00FF_FFFF;

    /// `AFFINITY_INFO` for the instance `target_affinity` names at
    /// `lowest_affinity_level` (DEN 0022D §5.1.5, §5.7). Level `n` ignores
    /// the fields Aff0 to Aff(`n`-1); a level above 3, or an instance this
    /// processor is not in, is not present in the platform and gets
    /// INVALID_PARAMETERS (§5.7.2). The one processor is on, so every
    /// present instance is ON: §5.7.1 lets PSCI 1.0 and later either answer
    /// a level above 0 so or refuse it, and this answers it.
    pub(super) fn affinity_info(target_affinity: u64, lowest_affinity_level: u32) -> u64 {
        let ignored: u64 = match lowest_affinity_level {
            0 => 0,
            1 => 0xFF,
            2 => 0xFFFF,
            3 => 0xFF_FFFF,
            _ => return INVALID_PARAMETERS,
        };
        if target_affinity & !ignored == AFFINITY & !ignored {
            ON
        } else {
            INVALID_PARAMETERS
        }
    }
}

/// The SMC Calling Convention's Arm Architecture Calls (Arm DEN 0028F §7)
/// the board's firmware answers, beside PSCI.
mod smccc {
    pub(super) const VERSION: u32 = 0x8000_0000;
    pub(super) const ARCH_FEATURES: u32 = 0x8000_0001;
    pub(super) const ARCH_WORKAROUND_1: u32 = 0x8000_8000;
    pub(super) const ARCH_WORKAROUND_2: u32 = 0x8000_7FFF;
    pub(super) const ARCH_WORKAROUND_3: u32 = 0x8000_3FFF;

    /// SMCCC 1.5, the version DEN 0028F specifies (Table 13-1: major 1 in
    /// bits 30:16, minor 5 in bits 15:0).
    pub(super) const IMPLEMENTED_VERSION: u64 = 0x0001_0005;

    /// Function identifier bit 31: a Fast Call (DEN 0028F §2.5, Table 2-1).
    const FAST_CALL: u32 = 1 << 31;
    /// Function identifier bit 16 of a Fast Call, from SMCCC 1.3: the hint
    /// that the caller holds no SVE live state, "not part of the function
    /// identification" (§2.5.1).
    const SVE_HINT: u32 = 1 << 16;

    /// The function `fid` identifies: a Fast Call's with its SVE hint
    /// taken as 0, as "the SMCCC implementation must disregard this bit
    /// and consider it to be 0 for the purpose of function identification"
    /// (§2.5.1). The board keeps every register over a call, so the hint
    /// asks nothing of it (§2.8).
    pub(super) const fn identify(fid: u32) -> u32 {
        if fid & FAST_CALL != 0 {
            fid & !SVE_HINT
        } else {
            fid
        }
    }

    /// Table 7-1's return codes beyond PSCI's `SUCCESS` and
    /// `NOT_SUPPORTED`, which have the same values.
    pub(super) const NOT_REQUIRED: u64 = (-2i64) as u64;
    /// `SMCCC_ARCH_FEATURES` of a workaround: it "can be invoked safely on
    /// all PEs in the system", and the PE asking "does not require firmware
    /// mitigation" (§7.5.2, §7.6.2, §7.7.2).
    pub(super) const SAFE_AND_NOT_REQUIRED: u64 = 1;
}

/// What the board's firmware tells a guest about the speculative-execution
/// vulnerabilities the SMC Calling Convention has workarounds for:
/// CVE-2017-5715 (`SMCCC_ARCH_WORKAROUND_1`), CVE-2018-3639
/// (`SMCCC_ARCH_WORKAROUND_2`) and CVE-2022-23960 with CVE-2017-5715
/// (`SMCCC_ARCH_WORKAROUND_3`), through `SMCCC_ARCH_FEATURES` (Arm DEN
/// 0028F §7.5 to §7.7). The interpreter executes nothing speculatively, so
/// no workaround has anything to do; each answer DEN 0028F allows is
/// offered, the choice being the host's.
///
/// Closed (R5): the three answers the discovery sections give.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpectreReport {
    /// Each workaround "can be invoked safely" and the processor "does not
    /// require firmware mitigation" (discovery 1), and `WORKAROUND_2`'s
    /// mitigation is "not required" (`NOT_REQUIRED`): what is true of an
    /// interpreter. `WORKAROUND_1` and `WORKAROUND_3` are implemented as
    /// calls that have no effect (§7.5.4, §7.7.4); `WORKAROUND_2` is not,
    /// as `NOT_REQUIRED` says it "must not be invoked".
    NotRequired,
    /// The processor requires firmware mitigation and the firmware
    /// provides it (discovery 0 for all three), so a guest takes its
    /// firmware mitigation paths. Each workaround is implemented as a call
    /// that has no effect, there being nothing to mitigate.
    Mitigated,
    /// The firmware "does not provide any information" (`NOT_SUPPORTED`
    /// for all three), as firmware of SMCCC 1.0 does not; no workaround is
    /// implemented.
    Unreported,
}

impl SpectreReport {
    /// Every answer, in the order a list of them shows.
    pub const ALL: [SpectreReport; 3] = [Self::NotRequired, Self::Mitigated, Self::Unreported];

    /// The answer a board gives unless its host chooses another.
    pub const DEFAULT: Self = Self::NotRequired;

    /// The answer's name for a command line or a configuration file, which
    /// [`str::parse`] reads back.
    pub const fn name(self) -> &'static str {
        match self {
            Self::NotRequired => "not-required",
            Self::Mitigated => "mitigated",
            Self::Unreported => "unreported",
        }
    }

    /// The answer as a snapshot's machine shape records it, fixed per
    /// answer whatever order the variants are declared in.
    pub(crate) const fn tag(self) -> u8 {
        match self {
            Self::NotRequired => 0,
            Self::Mitigated => 1,
            Self::Unreported => 2,
        }
    }

    /// `SMCCC_ARCH_FEATURES` of `workaround`.
    const fn discovery(self, workaround: Workaround) -> u64 {
        match (self, workaround) {
            (Self::NotRequired, Workaround::Two) => smccc::NOT_REQUIRED,
            (Self::NotRequired, Workaround::One | Workaround::Three) => {
                smccc::SAFE_AND_NOT_REQUIRED
            }
            (Self::Mitigated, _) => psci::SUCCESS,
            (Self::Unreported, _) => psci::NOT_SUPPORTED,
        }
    }

    /// Whether `workaround` is implemented: its discovery is not negative
    /// (§7.3.4).
    const fn implements(self, workaround: Workaround) -> bool {
        (self.discovery(workaround) as i64) >= 0
    }
}

/// The SMC Calling Convention's speculation workarounds (DEN 0028F §7.5 to
/// §7.7).
///
/// Closed (R5): the three SMCCC 1.5 defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Workaround {
    /// `SMCCC_ARCH_WORKAROUND_1`, CVE-2017-5715.
    One,
    /// `SMCCC_ARCH_WORKAROUND_2`, CVE-2018-3639.
    Two,
    /// `SMCCC_ARCH_WORKAROUND_3`, CVE-2017-5715 and CVE-2022-23960.
    Three,
}

impl Workaround {
    /// The workaround the function identifier `function` names, if one
    /// does.
    const fn of(function: u32) -> Option<Self> {
        match function {
            smccc::ARCH_WORKAROUND_1 => Some(Self::One),
            smccc::ARCH_WORKAROUND_2 => Some(Self::Two),
            smccc::ARCH_WORKAROUND_3 => Some(Self::Three),
            _ => None,
        }
    }
}

impl Default for SpectreReport {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The answer's [`SpectreReport::name`].
impl core::fmt::Display for SpectreReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

impl core::str::FromStr for SpectreReport {
    type Err = UnknownSpectreReport;

    /// The answer whose [`SpectreReport::name`] is `name`.
    fn from_str(name: &str) -> Result<Self, UnknownSpectreReport> {
        Self::ALL
            .into_iter()
            .find(|report| report.name() == name)
            .ok_or(UnknownSpectreReport)
    }
}

/// A name no [`SpectreReport`] has. It prints the names there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnknownSpectreReport;

impl core::fmt::Display for UnknownSpectreReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("no speculation report has that name; the reports are")?;
        for (i, report) in SpectreReport::ALL.iter().enumerate() {
            let separator = if i == 0 { " " } else { ", " };
            write!(f, "{separator}\"{}\"", report.name())?;
        }
        Ok(())
    }
}

impl core::error::Error for UnknownSpectreReport {}

impl<L: Levels> Cpu<L> {
    /// Whether the two-bit `CPACR_EL1` enable `field` ([`CPACR_FPEN`],
    /// [`CPACR_SMEN`](super::sysreg::CPACR_SMEN)) allows its instructions
    /// at `el`: `11` at EL0 and EL1, `01` at EL1 alone, `x0` at neither;
    /// EL2 is not subject to it. A processor without EL2 is never at EL2,
    /// so for one the EL2 arm is not compiled and no level is tested first.
    #[inline]
    pub(crate) fn cpacr_enabled_at(&self, field: u64, el: El) -> bool {
        match el {
            El::El2 if L::EL2 => true,
            El::El0 | El::El1 | El::El2 => {
                match (self.sys.cpacr_el1 & field) >> field.trailing_zeros() {
                    0b11 => true,
                    0b01 => el == El::El1,
                    _ => false,
                }
            }
        }
    }

    /// Whether `CPACR_EL1.FPEN` allows SIMD&FP at `el`.
    #[inline]
    pub(crate) fn cpacr_fp_enabled_at(&self, el: El) -> bool {
        self.cpacr_enabled_at(CPACR_FPEN, el)
    }

    /// Whether `CPTR_EL2.TFP` (with `HCR_EL2.E2H` 0, the only layout here)
    /// disables SIMD&FP at EL0, EL1 and EL2.
    #[inline]
    pub(crate) fn cptr_el2_traps_fp(&self) -> bool {
        self.cptr_any(CPTR_TFP)
    }

    /// Arm `AArch64_IsFPEnabled(el)`: `CPACR_EL1.FPEN` at EL0 and EL1, then
    /// `CPTR_EL2.TFP` at EL0, EL1 and EL2.
    #[inline]
    pub(crate) fn fp_enabled_at(&self, el: El) -> bool {
        self.cpacr_fp_enabled_at(el) && !self.cptr_el2_traps_fp()
    }

    /// The trap a disabled SIMD&FP access raises (Arm
    /// `AArch64_AdvSIMDFPAccessTrap`): EC 0x07, with `CV` set and `COND`
    /// 0b1110 as for every AArch64 instruction, to EL1 for `CPACR_EL1`'s
    /// ([`Route::Class`]) and to EL2 for `CPTR_EL2`'s ([`Route::El2`]).
    /// `HCR_EL2.TGE` takes the one to EL1 to EL2 as an uncategorized
    /// exception.
    #[inline]
    pub(crate) fn fp_trap(&self, route: Route) -> Fault {
        match route {
            Route::Class if self.tge() => Exception::to_el2(SyndromeClass::Unknown, 0).into(),
            Route::Class | Route::El2 => {
                Exception::routed(SyndromeClass::FpAccess, (1 << 24) | (0xE << 20), route).into()
            }
        }
    }

    /// Where the SIMD&FP enables trap an access at the current level, if
    /// they do, in the order Arm's checks make them: `CPACR_EL1.FPEN` at EL0
    /// and EL1 to EL1 ([`Route::Class`]), then `CPTR_EL2.TFP` to EL2
    /// ([`Route::El2`]). The SIMD&FP instructions and the `FPCR` and `FPSR`
    /// accessors alike check it.
    #[inline]
    pub(crate) fn fp_access_trap_route(&self) -> Option<Route> {
        if !self.cpacr_fp_enabled_at(self.pstate.el) {
            Some(Route::Class)
        } else if self.cptr_el2_traps_fp() {
            Some(Route::El2)
        } else {
            None
        }
    }

    /// Arm `AArch64_CheckFPEnabled`: the check of the scalar floating-point
    /// instructions and the SIMD&FP register loads and stores, made after
    /// decode has found the encoding allocated
    /// ([`Cpu::fp_access_trap_route`], `AArch64_CheckFPAdvSIMDTrap`). These
    /// are legal in Streaming SVE mode.
    #[inline]
    pub(crate) fn fp_check(&self) -> Result<(), Fault> {
        match self.fp_access_trap_route() {
            Some(route) => Err(self.fp_trap(route)),
            None => Ok(()),
        }
    }

    /// Arm `AArch64_CheckFPAdvSIMDEnabled`: the check of the Advanced SIMD
    /// instructions, which is [`Cpu::fp_check`] and then, in Streaming SVE
    /// mode, the SME trap for an instruction illegal there
    /// (`IsFullA64Enabled` is FALSE without FEAT_SME_FA64).
    #[inline]
    pub(crate) fn simd_check(&self) -> Result<(), Fault> {
        self.fp_check()?;
        if self.svcr.sm {
            return Err(self.sme_trap(SmeTrap::Streaming, Route::Class));
        }
        Ok(())
    }

    /// The trap, routed as `route` says, of the System instruction `insn`
    /// (`SYS`, which reports no read): the path an `MRS` or `MSR` trap takes.
    fn sys_trap(&self, pc: u64, insn: u32, rt: u32, route: Route) -> ExecResult {
        self.sysreg_fault(
            pc,
            insn,
            SysRegError::Trap(route),
            SysRegKey::from_insn(insn),
            rt,
            false,
        )
    }

    /// The trap to EL2 of the System instruction `insn` when an `HCR_EL2`
    /// control traps it at the current level, `trapped`
    /// (`AArch64_SystemAccessTrap(EL2, 0x18)`): the one shape of
    /// [`Cpu::exec_sys`]'s EL2 checks, each made after the instruction's
    /// own.
    fn sys_el2_trap(trapped: bool, insn: u32, rt: u32) -> Result<(), Fault> {
        if trapped {
            Err(Self::sysreg_trap(
                SysRegKey::from_insn(insn),
                rt,
                false,
                Route::El2,
            ))
        } else {
            Ok(())
        }
    }

    /// The exception an access trapped to `route` takes (EC 0x18), its
    /// syndrome from `key`, `rt` and the direction.
    fn sysreg_trap(key: SysRegKey, rt: u32, read: bool, route: Route) -> Fault {
        Exception::routed(SyndromeClass::SysRegTrap, key.trap_iss(rt, read), route).into()
    }

    /// The exception an `MRS`, `MSR`, `SYS` or `SYSL` that `err` refuses
    /// takes, its syndrome from `key`, `rt` and the direction.
    fn sysreg_fault(
        &self,
        pc: u64,
        insn: u32,
        err: SysRegError,
        key: SysRegKey,
        rt: u32,
        read: bool,
    ) -> ExecResult {
        match err {
            SysRegError::Undefined => self.undefined(pc, insn),
            SysRegError::Trap(route) => Err(Self::sysreg_trap(key, rt, read, route)),
            SysRegError::FpTrap(route) => Err(self.fp_trap(route)),
            SysRegError::SmeTrap(route) => Err(self.sme_trap(SmeTrap::AccessTrap, route)),
        }
    }

    /// The HINT space: NOP, YIELD, WFE, WFI, SEV, SEVL, ESB; every other
    /// hint is a NOP, the pointer authentication hints of a processor
    /// without FEAT_PAuth and the BTI hint included (with FEAT_PAuth those
    /// have a handler of their own, [`Cpu::exec_pauth_hint`]). A WFx traps
    /// from EL0 only when it would wait (Arm ARM `Hint_WFE`, `Hint_WFI`).
    pub(crate) fn exec_hint(&mut self, insn: u32, _pc: u64) -> ExecResult {
        match bits(insn, 11, 5) {
            2 => self.exec_wfe(),
            3 => self.exec_wfi(),
            // SEV and SEVL: with one processor, `SendEvent` is
            // `SendEventLocal`.
            4 | 5 => {
                self.send_event();
                Ok(Flow::Next)
            }
            // ESB (FEAT_RAS): its `AArch64_ESBOperation` defers a masked
            // physical SError into DISR_EL1, and nothing on this machine
            // raises one; at EL0 and EL1 with EL2 enabled,
            // `AArch64_vESBOperation` defers a masked virtual one.
            16 if self.implements(Feature::Ras) => {
                self.virtual_esb();
                Ok(Flow::Next)
            }
            // Every other hint.
            _ => Ok(Flow::Next),
        }
    }

    /// `1101 0101 00 L op0 op1 CRn CRm op2 Rt`.
    pub(crate) fn exec_system<B: PhysBus>(
        &mut self,
        bus: &mut B,
        pc: u64,
        insn: u32,
    ) -> ExecResult {
        let l = bit(insn, 21);
        let op0 = bits(insn, 20, 19);
        let op1 = bits(insn, 18, 16);
        let crn = bits(insn, 15, 12);
        let crm = bits(insn, 11, 8);
        let op2 = bits(insn, 7, 5);
        let rt = bits(insn, 4, 0);
        match (l, op0) {
            (false, 0) => self.exec_pstate_or_barrier(pc, insn, op1, crn, crm, op2, rt),
            (false, 1) => self.exec_sys(bus, pc, insn, op1, crn, crm, op2, rt),
            (true, 1) => self.exec_sysl(pc, insn, rt),
            (true, 0) => self.undefined(pc, insn),
            (false, _) => {
                let key = SysRegKey::from_insn(insn);
                let value = self.x_or_zr(rt);
                match self.sysreg_write(key, value) {
                    Ok(effect) => self.apply_write_effect(effect),
                    Err(e) => return self.sysreg_fault(pc, insn, e, key, rt, false),
                }
                Ok(Flow::Next)
            }
            (true, _) => {
                let key = SysRegKey::from_insn(insn);
                match self.sysreg_mrs(key) {
                    Ok(v) => {
                        self.set_x_or_zr(rt, v);
                        Ok(Flow::Next)
                    }
                    Err(e) => self.sysreg_fault(pc, insn, e, key, rt, true),
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_pstate_or_barrier(
        &mut self,
        pc: u64,
        insn: u32,
        op1: u32,
        crn: u32,
        crm: u32,
        op2: u32,
        rt: u32,
    ) -> ExecResult {
        if rt != 31 {
            return self.undefined(pc, insn);
        }
        let el0 = self.pstate.el == El::El0;
        let operand = crm & 1 != 0;
        match crn {
            3 => match op2 {
                // CLREX
                2 => {
                    self.monitor.armed = false;
                    Ok(Flow::Next)
                }
                // DSB, DMB, ISB, SB (FEAT_SB, which the decoder admits with
                // CRm = 0 alone) and DSB nXS (op2 = 1, FEAT_XS, which the
                // decoder admits with CRm<1:0> = 10 alone): an interpreter
                // that retires each access in order before the next, and
                // never speculates, has nothing to wait for.
                1 | 4..=7 => Ok(Flow::Next),
                _ => self.undefined(pc, insn),
            },
            // MSR (immediate): op1 = 000 needs EL1, op1 = 011 is open to
            // EL0 but for the DAIF controls; a field of a feature this
            // processor lacks is UNDEFINED.
            4 => match (op1, op2) {
                // UAO (FEAT_UAO)
                (0, 3) if self.implements(Feature::Uao) && !el0 => {
                    self.pstate.uao = operand;
                    Ok(Flow::Next)
                }
                // PAN (FEAT_PAN)
                (0, 4) if self.implements(Feature::Pan) && !el0 => {
                    self.pstate.pan = operand;
                    Ok(Flow::Next)
                }
                // SPSel
                (0, 5) => {
                    if el0 {
                        return self.undefined(pc, insn);
                    }
                    self.pstate.sp_sel = operand;
                    Ok(Flow::Next)
                }
                // SSBS (FEAT_SSBS)
                (3, 1) if self.implements(Feature::Ssbs) => {
                    self.pstate.ssbs = operand;
                    Ok(Flow::Next)
                }
                // DIT (FEAT_DIT)
                (3, 2) if self.implements(Feature::Dit) => {
                    self.pstate.dit = operand;
                    Ok(Flow::Next)
                }
                // DAIFSet / DAIFClr
                (3, 6) | (3, 7) => {
                    if self.pstate.el == El::El0 && self.sys.sctlr_el1 & sctlr::UMA == 0 {
                        return self.sys_trap(pc, insn, rt, Route::Class);
                    }
                    let bits = u64::from(crm) << 6;
                    let daif = self.pstate.daif();
                    self.pstate
                        .set_daif(if op2 == 6 { daif | bits } else { daif & !bits });
                    Ok(Flow::Next)
                }
                // SVCRSM, SVCRZA, SVCRSMZA (FEAT_SME): SMSTART, SMSTOP.
                (3, 3) => self.exec_msr_svcr(pc, insn),
                // ALLINT and PM (op1 001), TCO (3, 4): features this core
                // lacks.
                _ => self.undefined(pc, insn),
            },
            _ => self.undefined(pc, insn),
        }
    }

    /// CFINV (FEAT_FlagM), XAFLAG and AXFLAG (FEAT_FlagM2), by `op2`. XAFLAG
    /// turns the flags a floating-point compare sets into the ones an
    /// integer compare would for the same outcome, AXFLAG the reverse,
    /// with an unordered result as Z and C both set.
    pub(crate) fn exec_flag_manipulation(&mut self, pc: u64, insn: u32) -> ExecResult {
        let p = &mut self.pstate;
        let (z, c, v) = (p.z, p.c, p.v);
        match bits(insn, 7, 5) {
            // CFINV
            0b000 => p.c = !c,
            // XAFLAG
            0b001 => {
                p.n = !c && !z;
                p.z = z && c;
                p.c = c || z;
                p.v = !c && z;
            }
            // AXFLAG
            0b010 => {
                p.n = false;
                p.z = z || v;
                p.c = c && !v;
                p.v = false;
            }
            // MSR (immediate), which the decoder does not send here.
            _ => return self.undefined(pc, insn),
        }
        Ok(Flow::Next)
    }

    /// `SYSL`: no System instruction with a result exists here, so an
    /// encoding in the IMPLEMENTATION DEFINED space does what
    /// [`Cpu::impdef_space_access`] says and every other is UNDEFINED.
    fn exec_sysl(&mut self, pc: u64, insn: u32, rt: u32) -> ExecResult {
        let key = SysRegKey::from_insn(insn);
        if key.in_impdef_space() {
            return self.sysreg_fault(
                pc,
                insn,
                self.impdef_space_access(self.pstate.el),
                key,
                rt,
                true,
            );
        }
        self.undefined(pc, insn)
    }

    /// `SYS`: cache maintenance, address translation and TLB maintenance,
    /// with each instruction's EL0 control (a trap to EL1, which
    /// `HCR_EL2.TGE` takes to EL2) checked before its `HCR_EL2` trap to EL2,
    /// as its accessor in Arm's `Registers.json` orders them; and in the
    /// IMPLEMENTATION DEFINED space the Cortex-A57's `RAMINDEX`
    /// ([`Cpu::impdef_instruction`]).
    #[allow(clippy::too_many_arguments)]
    fn exec_sys<B: PhysBus>(
        &mut self,
        bus: &mut B,
        pc: u64,
        insn: u32,
        op1: u32,
        crn: u32,
        crm: u32,
        op2: u32,
        rt: u32,
    ) -> ExecResult {
        let el = self.pstate.el;
        let el0 = el == El::El0;
        let el1 = el == El::El1;
        let below_el2 = el != El::El2;
        let key = SysRegKey::from_insn(insn);
        if key.in_impdef_space() {
            let operand = self.x_or_zr(rt);
            return match self.impdef_instruction(key, el, operand) {
                Ok(()) => Ok(Flow::Next),
                Err(err) => self.sysreg_fault(pc, insn, err, key, rt, false),
            };
        }
        match (op1, crn, crm, op2) {
            // IC IALLUIS, IC IALLU: the processor's decoded traces are its
            // instruction cache, and they are never stale. Every write that
            // can reach their memory forgets the traces it covers when it
            // is made (`note_store`, the board's DMA log, `mem_write`), so
            // there is nothing left to invalidate, as QEMU's system
            // emulation, whose translation blocks are kept coherent the
            // same way, makes them no-ops (target/arm/helper.c
            // `v8_cp_reginfo`: IC_IALLUIS and IC_IALLU are ARM_CP_NOP).
            // IC IVAU (below) invalidates nothing either, but translates
            // the address it names, as Arm `AArch64_IC` does. `HCR_EL2.TPU`
            // traps EL1's.
            (0, 7, 1, 0) | (0, 7, 5, 0) => {
                if el0 {
                    return self.undefined(pc, insn);
                }
                Self::sys_el2_trap(el1 && self.hcr_any(HCR_TPU), insn, rt)?;
                Ok(Flow::Next)
            }
            // IC IVAU, DC CVAC, DC CVAU, DC CIVAC, and DC CVAP (FEAT_DPB)
            // and DC CVADP (FEAT_DPB2): EL0 needs SCTLR_EL1.UCI. CLIDR_EL1
            // names caches to the PoU and the PoC and CTR_EL0.DIC is 0, so
            // none is executed as a NOP (`AArch64_TreatDCAsNOP`,
            // `AArch64_TreatICAsNOP`); a memory system with no PoP or PoDP
            // cleans DC CVAP and DC CVADP to the PoC (`AArch64_DC`).
            (3, 7, 5 | 10 | 11 | 12 | 13 | 14, 1) => {
                let implemented = match crm {
                    12 => self.implements(Feature::Dpb),
                    13 => self.implements(Feature::Dpb2),
                    _ => true,
                };
                if !implemented {
                    return self.undefined(pc, insn);
                }
                if el0 && self.sys.sctlr_el1 & sctlr::UCI == 0 {
                    return self.sys_trap(pc, insn, rt, Route::Class);
                }
                // To the PoU (`IC IVAU`, `DC CVAU`) `HCR_EL2.TPU`, the others
                // `TPCP`, trap EL0's and EL1's.
                let el2_control = if crm == 5 || crm == 11 {
                    HCR_TPU
                } else {
                    HCR_TPCP
                };
                Self::sys_el2_trap(below_el2 && self.hcr_any(el2_control), insn, rt)?;
                let op = if crm == 5 {
                    CacheMaintenance::InstructionInvalidate
                } else {
                    CacheMaintenance::Clean
                };
                let va = self.x_or_zr(rt);
                self.cache_maintenance(bus, va, op)?;
                Ok(Flow::Next)
            }
            // CFP, DVP and CPP RCTX (FEAT_SPECRES): EL0 needs
            // SCTLR_EL1.EnRCTX. Each restricts predictions
            // (`AArch64_RestrictPrediction`), which a processor that never
            // speculates does not make.
            (3, 7, 3, 4 | 5 | 7) => {
                if !self.implements(Feature::SpecRes) {
                    return self.undefined(pc, insn);
                }
                if el0 && self.sys.sctlr_el1 & sctlr::ENRCTX == 0 {
                    return self.sys_trap(pc, insn, rt, Route::Class);
                }
                Ok(Flow::Next)
            }
            // DC ZVA: EL0 needs SCTLR_EL1.DZE; `HCR_EL2.TDZ` traps EL0's and
            // EL1's.
            (3, 7, 4, 1) => {
                if el0 && self.sys.sctlr_el1 & sctlr::DZE == 0 {
                    return self.sys_trap(pc, insn, rt, Route::Class);
                }
                Self::sys_el2_trap(below_el2 && self.hcr_any(HCR_TDZ), insn, rt)?;
                let va = self.x_or_zr(rt);
                self.dc_zva(bus, va)?;
                Ok(Flow::Next)
            }
            // DC IVAC: EL1 and EL2; `HCR_EL2.TPCP` traps EL1's. Its
            // translation is an invalidate's, needing write permission where
            // stage 1 checks one: `AArch64_DC` translates before it makes
            // EL1's a clean and invalidate under `HCR_EL2.DC` or `VM`, which
            // with no data cache is the same operation.
            (0, 7, 6, 1) => {
                if el0 {
                    return self.undefined(pc, insn);
                }
                Self::sys_el2_trap(el1 && self.hcr_any(HCR_TPCP), insn, rt)?;
                let va = self.x_or_zr(rt);
                self.cache_maintenance(bus, va, CacheMaintenance::Invalidate)?;
                Ok(Flow::Next)
            }
            // DC ISW, DC CSW, DC CISW: EL1 and EL2; `HCR_EL2.TSW` traps
            // EL1's. With no data cache model, a set and way names nothing
            // to act on.
            (0, 7, 6, 2) | (0, 7, 10, 2) | (0, 7, 14, 2) => {
                if el0 {
                    return self.undefined(pc, insn);
                }
                Self::sys_el2_trap(el1 && self.hcr_any(HCR_TSW), insn, rt)?;
                Ok(Flow::Next)
            }
            // AT S1E1R, S1E1W, S1E0R, S1E0W, and with FEAT_PAN2 AT S1E1RP
            // and S1E1WP, which PSTATE.PAN takes part in: the EL1&0 regime,
            // from EL1 or EL2.
            (0, 7, 8, 0..=3) | (0, 7, 9, 0 | 1) => {
                if el0 || (crm == 9 && !self.implements(Feature::Pan2)) {
                    return self.undefined(pc, insn);
                }
                let va = self.x_or_zr(rt);
                let checked = match (crm, op2 >> 1) {
                    (9, _) => AtCheck::El1Pan,
                    (_, 0) => AtCheck::El1,
                    _ => AtCheck::El0,
                };
                self.address_translate(bus, va, op2 & 1 == 1, checked, AtStages::One)?;
                Ok(Flow::Next)
            }
            // AT S1E2R, S1E2W: the EL2 regime, at EL2 only.
            (4, 7, 8, 0 | 1) => {
                if el != El::El2 {
                    return self.undefined(pc, insn);
                }
                let va = self.x_or_zr(rt);
                self.address_translate(bus, va, op2 & 1 == 1, AtCheck::El2, AtStages::One)?;
                Ok(Flow::Next)
            }
            // AT S12E1R, S12E1W, S12E0R, S12E0W: the EL1&0 regime, at EL2
            // only (EL1's are UNDEFINED without FEAT_NV), by stage 1 and then
            // stage 2, which `AArch64_S2Translate` leaves out while
            // `HCR_EL2.{DC,VM}` is {0,0}.
            (4, 7, 8, 4..=7) => {
                if el != El::El2 {
                    return self.undefined(pc, insn);
                }
                let va = self.x_or_zr(rt);
                let checked = if op2 < 6 { AtCheck::El1 } else { AtCheck::El0 };
                self.address_translate(bus, va, op2 & 1 == 1, checked, AtStages::OneAndTwo)?;
                Ok(Flow::Next)
            }
            // The EL1 TLB maintenance instructions (CRn 8), and their nXS
            // forms (CRn 9, FEAT_XS), at EL1 and EL2; `HCR_EL2.TTLB` traps
            // EL1's. Each removes what it names (`tlbi.rs`).
            (0, 8 | 9, _, _) => {
                let form = TlbiForm::el1(crm, op2);
                let Some(form) = form.filter(|_| !el0 && self.implements_tlbi(crn, crm, op2))
                else {
                    return self.undefined(pc, insn);
                };
                Self::sys_el2_trap(el1 && self.hcr_any(HCR_TTLB), insn, rt)?;
                self.tlb_maintenance(form, self.x_or_zr(rt));
                Ok(Flow::Next)
            }
            // The EL2 TLB maintenance instructions, at EL2 only: of the EL2
            // regime, of the whole EL1&0 regime, and of stage 2 by IPA.
            (4, 8 | 9, _, _) => {
                let form = TlbiForm::el2(crm, op2);
                let Some(form) =
                    form.filter(|_| el == El::El2 && self.implements_tlbi_el2(crn, crm, op2))
                else {
                    return self.undefined(pc, insn);
                };
                self.tlb_maintenance(form, self.x_or_zr(rt));
                Ok(Flow::Next)
            }
            _ => self.undefined(pc, insn),
        }
    }

    /// A TLB maintenance instruction of `form` with operand `xt`: what it
    /// names removed ([`Cpu::tlbi_invalidation`], [`Cpu::invalidate_tlb`]),
    /// nothing for a range form whose `TG` names no granule, which
    /// `TLBIRange` makes invalid. The trace ends either way, as at every
    /// TLBI (`Op::ends_trace`).
    fn tlb_maintenance(&mut self, form: TlbiForm, xt: u64) {
        match self.tlbi_invalidation(form, xt) {
            Some(op) => self.invalidate_tlb(&op),
            None => self.trace_break = true,
        }
    }

    /// Whether this processor implements the EL2 TLB maintenance
    /// instruction at `CRm` and `op2` of `op1 = 4`, `CRn` 8, or `CRn` 9 for
    /// its nXS form (FEAT_XS): Arm's `TLBI` pages for FEAT_EL2, with
    /// FEAT_TLBIOS's Outer Shareable forms and FEAT_TLBIRANGE's range forms.
    fn implements_tlbi_el2(&self, crn: u32, crm: u32, op2: u32) -> bool {
        if !self.have_el2() || (crn == 9 && !self.implements(Feature::Xs)) {
            return false;
        }
        let os = self.implements(Feature::TlbiOs);
        let range = self.implements(Feature::TlbiRange);
        match (crm, op2) {
            // IPAS2E1IS, IPAS2LE1IS; IPAS2E1, IPAS2LE1.
            (0 | 4, 1 | 5) => true,
            // ALLE2IS, VAE2IS, ALLE1IS, VALE2IS, VMALLS12E1IS, and ALLE2,
            // VAE2, ALLE1, VALE2, VMALLS12E1.
            (3 | 7, 0 | 1 | 4 | 5 | 6) => true,
            // ALLE2OS, VAE2OS, ALLE1OS, VALE2OS, VMALLS12E1OS; IPAS2E1OS,
            // IPAS2LE1OS.
            (1, 0 | 1 | 4 | 5 | 6) | (4, 0 | 4) => os,
            // RIPAS2E1IS, RIPAS2LE1IS; RIPAS2E1, RIPAS2LE1; RVAE2IS,
            // RVALE2IS; RVAE2, RVALE2.
            (0 | 4, 2 | 6) | (2 | 6, 1 | 5) => range,
            // RIPAS2E1OS, RIPAS2LE1OS; RVAE2OS, RVALE2OS.
            (4, 3 | 7) | (5, 1 | 5) => range && os,
            _ => false,
        }
    }

    /// Whether this processor implements the EL1 TLB maintenance instruction
    /// at `CRm` and `op2` of `op1 = 0`, `CRn` 8, or `CRn` 9 for its nXS form,
    /// which FEAT_XS adds (Arm's `TLBI` pages, and QEMU
    /// target/arm/tcg/tlb-insns.c `tlbi_v8_cp_reginfo`, `tlbios_reginfo`
    /// and `tlbirange_reginfo`).
    fn implements_tlbi(&self, crn: u32, crm: u32, op2: u32) -> bool {
        if crn == 9 && !self.implements(Feature::Xs) {
            return false;
        }
        let os = self.implements(Feature::TlbiOs);
        let range = self.implements(Feature::TlbiRange);
        match (crm, op2) {
            // VMALLE1IS, VAE1IS, ASIDE1IS, VAAE1IS, VALE1IS, VAALE1IS, and
            // VMALLE1, VAE1, ASIDE1, VAAE1, VALE1, VAALE1.
            (3 | 7, 0 | 1 | 2 | 3 | 5 | 7) => true,
            // VMALLE1OS, VAE1OS, ASIDE1OS, VAAE1OS, VALE1OS, VAALE1OS.
            (1, 0 | 1 | 2 | 3 | 5 | 7) => os,
            // RVAE1IS, RVAAE1IS, RVALE1IS, RVAALE1IS, and RVAE1, RVAAE1,
            // RVALE1, RVAALE1.
            (2 | 6, 1 | 3 | 5 | 7) => range,
            // RVAE1OS, RVAAE1OS, RVALE1OS, RVAALE1OS.
            (5, 1 | 3 | 5 | 7) => range && os,
            _ => false,
        }
    }

    /// SVC, HVC, SMC, BRK, HLT, DCPS: `11010100 opc imm16 op2 LL`.
    pub(crate) fn exec_exception_gen(&mut self, pc: u64, insn: u32) -> ExecResult {
        let opc = bits(insn, 23, 21);
        let imm16 = bits(insn, 20, 5);
        if bits(insn, 4, 2) != 0 {
            return self.undefined(pc, insn);
        }
        match (opc, bits(insn, 1, 0)) {
            (0, 1) => Err(Exception::new(SyndromeClass::Svc, imm16).into()),
            (0, 2) => self.exec_hvc(pc, insn, imm16),
            (0, 3) => self.exec_smc(pc, insn, imm16),
            (1, 0) => Err(Exception::new(SyndromeClass::Brk, imm16).into()),
            // HLT with halting debug disabled, and DCPS outside Debug state,
            // are UNDEFINED.
            _ => self.undefined(pc, insn),
        }
    }

    /// The conduit the board's PSCI firmware answers this processor on
    /// ([`PsciConduit::of`]).
    fn psci_conduit(&self) -> PsciConduit {
        PsciConduit::of::<L>()
    }

    /// `HVC #imm16`: UNDEFINED at EL0. On a processor without EL2, where
    /// Arm makes `HVC` UNDEFINED, this board's firmware stands at EL1 where
    /// EL2 would, as QEMU's `virt` does (PSCI conduit "hvc";
    /// target/arm/tcg/psci.c `arm_is_psci_call`): every HVC is a firmware
    /// call ([`Cpu::psci_conduit`]), whatever its immediate (DEN 0028B §2.9
    /// leaves nonzero HVC immediates to the hypervisor) and function ID, and
    /// an unknown ID gets NOT_SUPPORTED; README "Where the references
    /// differ" registers the departure. With EL2, Arm's `HVC`
    /// (`AArch64_CallHypervisor`): an exception to EL2 from EL1 or EL2,
    /// returning to the next instruction, unless `HCR_EL2.HCD` makes it
    /// UNDEFINED.
    fn exec_hvc(&mut self, pc: u64, insn: u32, imm16: u32) -> ExecResult {
        if self.pstate.el == El::El0 {
            return self.undefined(pc, insn);
        }
        match self.psci_conduit() {
            PsciConduit::Hvc => self.psci_call(),
            PsciConduit::Smc => {
                if self.hcr_any(HCR_HCD) {
                    return self.undefined(pc, insn);
                }
                Err(Exception::to_el2(SyndromeClass::Hvc, imm16).into())
            }
        }
    }

    /// `SMC #imm16`: UNDEFINED at EL0; at EL1 under `HCR_EL2.TSC` trapped to
    /// EL2, returning to the `SMC` itself (`AArch64_CheckForSMCUndefOrTrap`
    /// with no EL3, whose IMPLEMENTATION DEFINED "Trap SMC execution at EL1
    /// to EL2" is TRUE here); otherwise a firmware call where the firmware's
    /// conduit is `SMC`, as for `HVC`, and UNDEFINED where it is not.
    fn exec_smc(&mut self, pc: u64, insn: u32, imm16: u32) -> ExecResult {
        if self.pstate.el == El::El0 {
            return self.undefined(pc, insn);
        }
        if self.pstate.el == El::El1 && self.hcr_any(HCR_TSC) {
            return Err(Exception::to_el2(SyndromeClass::Smc, imm16).into());
        }
        match self.psci_conduit() {
            PsciConduit::Smc => self.psci_call(),
            PsciConduit::Hvc => self.undefined(pc, insn),
        }
    }

    /// The PSCI 1.2 firmware interface (Arm DEN 0022E) for a single
    /// processor, with QEMU's `virt`'s function set (target/arm/tcg/psci.c
    /// `arm_handle_psci_call`): DEN 0022E Table 17's mandatory functions and
    /// `MIGRATE_INFO_TYPE`; and SMCCC 1.5's Arm Architecture Calls (Arm DEN
    /// 0028F §7): `SMCCC_VERSION`, `SMCCC_ARCH_FEATURES` and the
    /// workarounds the board's [`SpectreReport`] implements.
    ///
    /// The SMC Calling Convention passes the function identifier in W0 in
    /// both conventions, and an SMC32/HVC32 call's arguments in W1-W3 (Arm
    /// DEN 0028B §2.6, §2.7, and §3.1 Table 3-1, whose "Wn" is a register's
    /// low 32 bits, the upper ones ignored — §3.1 prints "Implementations
    /// must ignore the least significant bits" after "The least significant
    /// 32-bits are used, and the most significant 32-bits are zero", a slip
    /// for the most significant; DEN 0022D §5.2.1), so the upper
    /// halves of those X registers take no part, as in TF-A's
    /// `psci_smc_handler`. A parameter DEN 0022D declares `uint32`
    /// (`power_state`, `lowest_affinity_level`, `psci_func_id`) is its
    /// register's low 32 bits in either convention.
    fn psci_call(&mut self) -> ExecResult {
        let function = smccc::identify(self.x[0] as u32);
        let (arg1, arg2) = if function & psci::SMC64 != 0 {
            (self.x[1], self.x[2])
        } else {
            (u64::from(self.x[1] as u32), u64::from(self.x[2] as u32))
        };
        let result = match function {
            psci::VERSION => psci::IMPLEMENTED_VERSION,
            // DEN 0022E §5.15.2: `PSCI_FEATURES` "should report the presence
            // of SMCCC_VERSION", which DEN 0028F §7.2.2 discovers through it.
            psci::FEATURES => {
                let asked = smccc::identify(arg1 as u32);
                if psci_supported(asked) || asked == smccc::VERSION {
                    psci::SUCCESS
                } else {
                    psci::NOT_SUPPORTED
                }
            }
            smccc::VERSION => smccc::IMPLEMENTED_VERSION,
            smccc::ARCH_FEATURES => self.smccc_arch_features(smccc::identify(arg1 as u32)),
            // A workaround the firmware implements has no effect: no result,
            // every register as it was (DEN 0028F §7.5.4, §7.6.4, §7.7.4:
            // "an implementation that has no effect" on a PE that needs no
            // mitigation; X0 to X3 are result registers, which a call
            // without results leaves to the implementation).
            _ if Workaround::of(function).is_some_and(|w| self.spectre.implements(w)) => {
                return Ok(Flow::Next);
            }
            psci::CPU_SUSPEND_32 | psci::CPU_SUSPEND_64 => {
                // `power_state` is in the original format, as `PSCI_FEATURES`
                // reports (DEN 0022D §5.15.2, Table 11). Any of bits 31:17
                // set — its reserved bits, or a PowerLevel (25:24) above the
                // core's, the only power level here (§5.4.2.1, Table 7) — is
                // a power state this platform does not provide, which
                // §5.4.5 answers with INVALID_PARAMETERS. QEMU refuses the
                // same bits.
                if arg1 as u32 & 0xFFFE_0000 != 0 {
                    psci::INVALID_PARAMETERS
                } else {
                    // Every accepted request is entered as standby: wait for
                    // an interrupt, then return SUCCESS at the next
                    // instruction. A powerdown request (StateType, bit 16,
                    // set) is downgraded to standby so, which §5.4.5
                    // permits; QEMU does the same.
                    self.x[0] = psci::SUCCESS;
                    return Ok(self.begin_wait(Wait::Interrupt));
                }
            }
            psci::CPU_OFF => return Err(Fault::Stop(StopCause::CpuOff)),
            // DEN 0022D §5.6.2: a `target_cpu` that is no processor's
            // affinity value is INVALID_PARAMETERS, an entry point the
            // implementation knows to be invalid "because it is in a range
            // that is known not to be available to the caller" is
            // INVALID_ADDRESS, and a processor already on is ALREADY_ON. An
            // entry point that is not 4-byte aligned can never be executed
            // in AArch64, the only state here; it is not a range condition,
            // and INVALID_ADDRESS, the code for an invalid entry point, is
            // this implementation's choice for it. QEMU refuses it too
            // (target/arm/arm-powerctl.c `arm_set_cpu_on`), first and with
            // INVALID_PARAMETERS. The order is TF-A's
            // `psci_cpu_on_by_core_pos` (target, then entry point, then
            // state); the specification gives none. The only processor is
            // on.
            psci::CPU_ON_32 | psci::CPU_ON_64 => {
                if arg1 != psci::AFFINITY {
                    psci::INVALID_PARAMETERS
                } else if arg2 & 3 != 0 {
                    psci::INVALID_ADDRESS
                } else {
                    psci::ALREADY_ON
                }
            }
            psci::AFFINITY_INFO_32 | psci::AFFINITY_INFO_64 => {
                psci::affinity_info(arg1, arg2 as u32)
            }
            psci::MIGRATE_INFO_TYPE => psci::MIGRATE_NOT_REQUIRED,
            psci::SYSTEM_OFF => return Err(Fault::Stop(StopCause::PowerOff)),
            psci::SYSTEM_RESET => return Err(Fault::Stop(StopCause::Reset)),
            _ => psci::NOT_SUPPORTED,
        };
        self.x[0] = result;
        Ok(Flow::Next)
    }

    /// `SMCCC_ARCH_FEATURES` of `arch_func_id` (DEN 0028F §7.3): SUCCESS
    /// for itself and `SMCCC_VERSION` (§7.3.6), the workarounds as the
    /// board's [`SpectreReport`] answers them, and NOT_SUPPORTED for every
    /// other Arm Architecture or Standard Hypervisor Service function and
    /// for an ID outside those ranges (§7.3.3).
    fn smccc_arch_features(&self, arch_func_id: u32) -> u64 {
        match (arch_func_id, Workaround::of(arch_func_id)) {
            (smccc::VERSION | smccc::ARCH_FEATURES, _) => psci::SUCCESS,
            (_, Some(workaround)) => self.spectre.discovery(workaround),
            (_, None) => psci::NOT_SUPPORTED,
        }
    }
}

/// The instruction the board's PSCI firmware answers.
///
/// Closed (R5): the two conduits PSCI (Arm DEN 0022) defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PsciConduit {
    Hvc,
    Smc,
}

impl PsciConduit {
    /// The conduit the board's firmware answers a processor of the level
    /// set `L` on, the one decision both the processor and the device
    /// tree's `/psci` `method` follow (QEMU `virt`, `hw/arm/virt.c`
    /// `psci_conduit`): `HVC` on a processor without EL2, where nothing else
    /// takes an `HVC` from EL1, and `SMC` on one with EL2, whose `HVC` is the
    /// hypervisor's. The firmware stands in for the EL3 this processor does
    /// not implement, so where Arm makes the instruction UNDEFINED for want
    /// of EL2 or EL3 it is a firmware call instead.
    pub(crate) const fn of<L: Levels>() -> Self {
        if L::EL2 {
            Self::Smc
        } else {
            Self::Hvc
        }
    }

    /// The device tree's `/psci` `method` naming it (QEMU
    /// `fdt_add_psci_node`).
    pub(crate) const fn method(self) -> &'static str {
        match self {
            Self::Hvc => "hvc",
            Self::Smc => "smc",
        }
    }
}

/// The device tree's `/psci` `compatible`, its strings each NUL-terminated:
/// PSCI 1.0 and 0.2 (the `arm,psci-1.0` binding covers 1.0 and later, and
/// the firmware answers 1.2). A caller of PSCI 0.2 or later finds every
/// function by its standard identifier (DEN 0022), so the node gives
/// neither the 0.1 binding nor its function identifiers, which a firmware
/// of 0.1 alone would name.
pub(crate) const PSCI_COMPATIBLE: &[u8] = b"arm,psci-1.0\0arm,psci-0.2\0";

/// The PSCI functions `psci_call` implements, for `PSCI_FEATURES`.
fn psci_supported(id: u32) -> bool {
    matches!(
        id,
        psci::VERSION
            | psci::FEATURES
            | psci::CPU_SUSPEND_32
            | psci::CPU_SUSPEND_64
            | psci::CPU_OFF
            | psci::CPU_ON_32
            | psci::CPU_ON_64
            | psci::AFFINITY_INFO_32
            | psci::AFFINITY_INFO_64
            | psci::MIGRATE_INFO_TYPE
            | psci::SYSTEM_OFF
            | psci::SYSTEM_RESET
    )
}
