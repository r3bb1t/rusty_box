//! System instructions (Arm ARM C5): hints, barriers, PSTATE access, cache
//! and TLB maintenance, address translation, `MRS`/`MSR`, and the
//! exception-generating instructions — including the PSCI calls QEMU's
//! `virt` answers on `HVC` without EL2 and on `SMC` with it (its device
//! tree's `method`).

use super::el2::{CPTR_TFP, HCR_HCD, HCR_TDZ, HCR_TPCP, HCR_TPU, HCR_TSC, HCR_TSW, HCR_TTLB};
use super::exception::{Exception, ExceptionClass, Route};
use super::exec::{bit, bits, ExecResult, Fault, Flow, StopCause};
use super::features::Feature;
use super::mmu::{AtCheck, CacheMaintenance};
use super::sme::SmeTrap;
use super::sysreg::{SysRegError, SysRegKey, CPACR_FPEN};
use super::wait::Wait;
use super::{sctlr, Cpu, El, Levels};
use crate::bus::PhysBus;


/// PSCI function identifiers (Arm DEN 0022) and return codes.
mod psci {
    pub(super) const VERSION: u64 = 0x8400_0000;
    pub(super) const CPU_SUSPEND_32: u64 = 0x8400_0001;
    pub(super) const CPU_SUSPEND_64: u64 = 0xC400_0001;
    pub(super) const CPU_OFF: u64 = 0x8400_0002;
    pub(super) const CPU_ON_32: u64 = 0x8400_0003;
    pub(super) const CPU_ON_64: u64 = 0xC400_0003;
    pub(super) const AFFINITY_INFO_32: u64 = 0x8400_0004;
    pub(super) const AFFINITY_INFO_64: u64 = 0xC400_0004;
    pub(super) const MIGRATE_INFO_TYPE: u64 = 0x8400_0006;
    pub(super) const SYSTEM_OFF: u64 = 0x8400_0008;
    pub(super) const SYSTEM_RESET: u64 = 0x8400_0009;
    pub(super) const FEATURES: u64 = 0x8400_000A;

    pub(super) const SUCCESS: u64 = 0;
    pub(super) const NOT_SUPPORTED: u64 = (-1i64) as u64;
    pub(super) const INVALID_PARAMETERS: u64 = (-2i64) as u64;
    pub(super) const ALREADY_ON: u64 = (-4i64) as u64;

    /// PSCI 1.1, the version QEMU's `virt` implements for current machine
    /// types (`target/arm/psci.c`, `QEMU_PSCI_VERSION_1_1`).
    pub(super) const IMPLEMENTED_VERSION: u64 = 0x0001_0001;
    /// "Trusted OS is not present or does not require migration".
    pub(super) const MIGRATE_NOT_REQUIRED: u64 = 2;
}

impl<L: Levels> Cpu<L> {
    /// Whether the two-bit `CPACR_EL1` enable `field` ([`CPACR_FPEN`],
    /// [`CPACR_SMEN`](super::sysreg::CPACR_SMEN)) allows its instructions
    /// at `el`: `11` at EL0 and EL1, `01` at EL1 alone, `x0` at neither;
    /// EL2 is not subject to it.
    #[inline]
    pub(crate) fn cpacr_enabled_at(&self, field: u64, el: El) -> bool {
        match el {
            El::El2 => true,
            El::El0 | El::El1 => match (self.sys.cpacr_el1 & field) >> field.trailing_zeros() {
                0b11 => true,
                0b01 => el == El::El1,
                _ => false,
            },
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
            Route::Class if self.tge() => Exception::to_el2(ExceptionClass::Unknown, 0).into(),
            Route::Class | Route::El2 => {
                Exception::routed(ExceptionClass::FpAccess, (1 << 24) | (0xE << 20), route).into()
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
        self.sysreg_fault(pc, insn, SysRegError::Trap(route), SysRegKey::from_insn(insn), rt, false)
    }

    /// The exception an `MRS`, `MSR`, `SYS` or `SYSL` that `err` refuses
    /// takes, its syndrome from `key`, `rt` and the direction.
    fn sysreg_fault(&self, pc: u64, insn: u32, err: SysRegError, key: SysRegKey, rt: u32, read: bool) -> ExecResult {
        match err {
            SysRegError::Undefined => self.undefined(pc, insn),
            SysRegError::Trap(route) => {
                Err(Exception::routed(ExceptionClass::SysRegTrap, key.trap_iss(rt, read), route).into())
            }
            SysRegError::FpTrap(route) => Err(self.fp_trap(route)),
            SysRegError::SmeTrap(route) => Err(self.sme_trap(SmeTrap::AccessTrap, route)),
        }
    }

    /// The HINT space: NOP, YIELD, WFE, WFI, SEV, SEVL, ESB; every other
    /// hint is a NOP, the pointer authentication hints of a processor
    /// without FEAT_PAuth and the BTI hint included (with FEAT_PAuth those
    /// have a handler of their own, [`Cpu::exec_pauth_hint`]).
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
    pub(crate) fn exec_system<B: PhysBus>(&mut self, bus: &mut B, pc: u64, insn: u32) -> ExecResult {
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
                match self.sysreg_read(key) {
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
    fn exec_pstate_or_barrier(&mut self, pc: u64, insn: u32, op1: u32, crn: u32, crm: u32, op2: u32, rt: u32) -> ExecResult {
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
                    self.pstate.set_daif(if op2 == 6 { daif | bits } else { daif & !bits });
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
            return self.sysreg_fault(pc, insn, self.impdef_space_access(self.pstate.el), key, rt, true);
        }
        self.undefined(pc, insn)
    }

    /// `SYS`: cache maintenance, address translation and TLB maintenance,
    /// with each instruction's EL0 control (a trap to EL1, which
    /// `HCR_EL2.TGE` takes to EL2) checked before its `HCR_EL2` trap to EL2,
    /// as its accessor in Arm's `Registers.json` orders them.
    #[allow(clippy::too_many_arguments)]
    fn exec_sys<B: PhysBus>(&mut self, bus: &mut B, pc: u64, insn: u32, op1: u32, crn: u32, crm: u32, op2: u32, rt: u32) -> ExecResult {
        let el = self.pstate.el;
        let el0 = el == El::El0;
        let el1 = el == El::El1;
        let below_el2 = el != El::El2;
        let key = SysRegKey::from_insn(insn);
        if key.in_impdef_space() {
            return self.sysreg_fault(pc, insn, self.impdef_space_access(el), key, rt, false);
        }
        match (op1, crn, crm, op2) {
            // IC IALLUIS, IC IALLU: there is no instruction cache to
            // invalidate. `HCR_EL2.TPU` traps EL1's.
            (0, 7, 1, 0) | (0, 7, 5, 0) => {
                if el0 {
                    return self.undefined(pc, insn);
                }
                if el1 && self.hcr_any(HCR_TPU) {
                    return self.sys_trap(pc, insn, rt, Route::El2);
                }
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
                let el2_control = if crm == 5 || crm == 11 { HCR_TPU } else { HCR_TPCP };
                if below_el2 && self.hcr_any(el2_control) {
                    return self.sys_trap(pc, insn, rt, Route::El2);
                }
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
                if below_el2 && self.hcr_any(HCR_TDZ) {
                    return self.sys_trap(pc, insn, rt, Route::El2);
                }
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
                if el1 && self.hcr_any(HCR_TPCP) {
                    return self.sys_trap(pc, insn, rt, Route::El2);
                }
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
                if el1 && self.hcr_any(HCR_TSW) {
                    return self.sys_trap(pc, insn, rt, Route::El2);
                }
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
                self.address_translate(bus, va, op2 & 1 == 1, checked)?;
                Ok(Flow::Next)
            }
            // AT S1E2R, S1E2W: the EL2 regime, at EL2 only.
            (4, 7, 8, 0 | 1) => {
                if el != El::El2 {
                    return self.undefined(pc, insn);
                }
                let va = self.x_or_zr(rt);
                self.address_translate(bus, va, op2 & 1 == 1, AtCheck::El2)?;
                Ok(Flow::Next)
            }
            // AT S12E1R, S12E1W, S12E0R, S12E0W: the EL1&0 regime, at EL2
            // only (EL1's are UNDEFINED without FEAT_NV). Arm's accessor
            // translates stage 1 alone while `HCR_EL2.{DC,VM}` is {0,0} and
            // stages 1 and 2 otherwise; this processor applies no stage 2
            // translation to any access, so each is its stage 1 translation.
            (4, 7, 8, 4..=7) => {
                if el != El::El2 {
                    return self.undefined(pc, insn);
                }
                let va = self.x_or_zr(rt);
                let checked = if op2 < 6 { AtCheck::El1 } else { AtCheck::El0 };
                self.address_translate(bus, va, op2 & 1 == 1, checked)?;
                Ok(Flow::Next)
            }
            // The EL1 TLB maintenance instructions (CRn 8), and their nXS
            // forms (CRn 9, FEAT_XS), at EL1 and EL2; `HCR_EL2.TTLB` traps
            // EL1's. Each invalidates at least what it names, whatever its
            // TTL hint (FEAT_TTL) or range, so flushing everything satisfies
            // all of them.
            (0, 8 | 9, _, _) => {
                if el0 || !self.implements_tlbi(crn, crm, op2) {
                    return self.undefined(pc, insn);
                }
                if el1 && self.hcr_any(HCR_TTLB) {
                    return self.sys_trap(pc, insn, rt, Route::El2);
                }
                self.tlb.flush();
                Ok(Flow::Next)
            }
            // The EL2 TLB maintenance instructions, at EL2 only: of the EL2
            // regime, of the whole EL1&0 regime, and of stage 2 by IPA. Each
            // invalidates at least what it names, so flushing everything
            // satisfies all of them.
            (4, 8 | 9, _, _) => {
                if el != El::El2 || !self.implements_tlbi_el2(crn, crm, op2) {
                    return self.undefined(pc, insn);
                }
                self.tlb.flush();
                Ok(Flow::Next)
            }
            _ => self.undefined(pc, insn),
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
            (0, 1) => Err(Exception::new(ExceptionClass::Svc, imm16).into()),
            (0, 2) => self.exec_hvc(pc, insn, imm16),
            (0, 3) => self.exec_smc(pc, insn, imm16),
            (1, 0) => Err(Exception::new(ExceptionClass::Brk, imm16).into()),
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

    /// `HVC #imm16`: UNDEFINED at EL0; on a processor without EL2 a
    /// firmware call ([`Cpu::psci_conduit`]) whatever its immediate and
    /// function ID, the IDs the firmware does not know answered with
    /// NOT_SUPPORTED; with EL2, Arm's `HVC` (`AArch64_CallHypervisor`): an
    /// exception to EL2 from EL1 or EL2, returning to the next instruction,
    /// unless `HCR_EL2.HCD` makes it UNDEFINED.
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
                Err(Exception::to_el2(ExceptionClass::Hvc, imm16).into())
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
            return Err(Exception::to_el2(ExceptionClass::Smc, imm16).into());
        }
        match self.psci_conduit() {
            PsciConduit::Smc => self.psci_call(),
            PsciConduit::Hvc => self.undefined(pc, insn),
        }
    }

    /// The PSCI firmware interface QEMU's `virt` provides for a single
    /// processor (QEMU `target/arm/psci.c` `arm_handle_psci_call`).
    fn psci_call(&mut self) -> ExecResult {
        let function = self.x[0];
        let result = match function {
            psci::VERSION => psci::IMPLEMENTED_VERSION,
            psci::FEATURES => {
                if psci_supported(self.x[1]) {
                    psci::SUCCESS
                } else {
                    psci::NOT_SUPPORTED
                }
            }
            psci::CPU_SUSPEND_32 | psci::CPU_SUSPEND_64 => {
                if self.x[1] & 0xFFFE_0000 != 0 {
                    psci::INVALID_PARAMETERS
                } else {
                    // A standby request: wait for an interrupt, then return
                    // success, as QEMU does.
                    self.x[0] = psci::SUCCESS;
                    return Ok(self.begin_wait(Wait::Interrupt));
                }
            }
            psci::CPU_OFF => return Err(Fault::Stop(StopCause::CpuOff)),
            psci::CPU_ON_32 | psci::CPU_ON_64 => {
                if self.x[1] & 0xFF_00FF_FFFF == 0 {
                    psci::ALREADY_ON
                } else {
                    psci::INVALID_PARAMETERS
                }
            }
            psci::AFFINITY_INFO_32 | psci::AFFINITY_INFO_64 => {
                // Levels above 0 are always on; at level 0 only this
                // processor's affinity exists.
                if self.x[2] != 0 || self.x[1] & 0xFF_00FF_FFFF == 0 {
                    0 // ON
                } else {
                    psci::INVALID_PARAMETERS
                }
            }
            psci::MIGRATE_INFO_TYPE => psci::MIGRATE_NOT_REQUIRED,
            psci::SYSTEM_OFF => return Err(Fault::Stop(StopCause::PowerOff)),
            psci::SYSTEM_RESET => return Err(Fault::Stop(StopCause::Reset)),
            _ => psci::NOT_SUPPORTED,
        };
        self.x[0] = result;
        Ok(Flow::Next)
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

/// The PSCI functions `psci_call` implements, for `PSCI_FEATURES`.
fn psci_supported(id: u64) -> bool {
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
