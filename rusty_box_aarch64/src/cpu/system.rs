//! System instructions (Arm ARM C5): hints, barriers, PSTATE access, cache
//! and TLB maintenance, address translation, `MRS`/`MSR`, and the
//! exception-generating instructions — including the PSCI calls QEMU's
//! `virt` answers on `HVC` (its device tree says `method = "hvc"`).

use super::exception::{Exception, ExceptionClass};
use super::exec::{bit, bits, ExecResult, Fault, Flow, StopCause};
use super::sme::SmeTrap;
use super::sysreg::{SysRegError, SysRegKey, WriteEffect};
use super::{Cpu, El};
use crate::bus::PhysBus;

/// `SCTLR_EL1` EL0 controls the instructions here consult.
const SCTLR_UMA: u64 = 1 << 9;
const SCTLR_DZE: u64 = 1 << 14;
const SCTLR_NTWI: u64 = 1 << 16;
const SCTLR_NTWE: u64 = 1 << 18;
const SCTLR_UCI: u64 = 1 << 26;

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

impl Cpu {
    /// SIMD&FP access enabled at the current EL by `CPACR_EL1.FPEN`.
    #[inline]
    pub(crate) fn fp_enabled(&self) -> bool {
        match (self.sys.cpacr_el1 >> 20) & 3 {
            0b11 => true,
            0b01 => self.pstate.el == El::El1,
            _ => false,
        }
    }

    /// The trap a disabled SIMD&FP access raises: EC 0x07, with `CV` set and
    /// `COND` 0b1110 as for every AArch64 instruction.
    #[inline]
    pub(crate) fn fp_trap(&self) -> Fault {
        Exception::new(ExceptionClass::FpAccess, (1 << 24) | (0xE << 20)).into()
    }

    /// Arm `AArch64_CheckFPEnabled`: the check of the scalar floating-point
    /// instructions and the SIMD&FP register loads and stores, made after
    /// decode has found the encoding allocated. These are legal in
    /// Streaming SVE mode.
    #[inline]
    pub(crate) fn fp_check(&self) -> Result<(), Fault> {
        if self.fp_enabled() {
            Ok(())
        } else {
            Err(self.fp_trap())
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
            return Err(self.sme_trap(SmeTrap::Streaming));
        }
        Ok(())
    }

    fn sysreg_fault(&self, pc: u64, insn: u32, err: SysRegError, key: SysRegKey, rt: u32, read: bool) -> ExecResult {
        match err {
            SysRegError::Undefined => self.undefined(pc, insn),
            SysRegError::TrapToEl1 => Err(Exception::new(ExceptionClass::SysRegTrap, key.trap_iss(rt, read)).into()),
            SysRegError::FpTrap => Err(self.fp_trap()),
            SysRegError::SmeTrap => Err(self.sme_trap(SmeTrap::AccessTrap)),
        }
    }

    /// The HINT space: NOP, YIELD, WFE, WFI, SEV, SEVL; every other hint is
    /// a NOP, the pointer authentication hints of a processor without
    /// FEAT_PAuth and the BTI hint included (with FEAT_PAuth those have a
    /// handler of their own, [`Cpu::exec_pauth_hint`]).
    pub(crate) fn exec_hint(&mut self, insn: u32, _pc: u64) -> ExecResult {
        match bits(insn, 11, 5) {
            // WFE: there is no other processor to send an event, and WFE may
            // complete at any time, so it completes at once.
            2 => {
                if self.pstate.el == El::El0 && self.sys.sctlr_el1 & SCTLR_NTWE == 0 {
                    return Err(Exception::new(ExceptionClass::WfxTrap, (1 << 24) | (0xE << 20) | 1).into());
                }
                Ok(Flow::Next)
            }
            3 => {
                if self.pstate.el == El::El0 && self.sys.sctlr_el1 & SCTLR_NTWI == 0 {
                    return Err(Exception::new(ExceptionClass::WfxTrap, (1 << 24) | (0xE << 20)).into());
                }
                Ok(Flow::Wait)
            }
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
            (true, 0) | (true, 1) => self.undefined(pc, insn),
            (false, _) => {
                let key = SysRegKey::from_insn(insn);
                let value = self.x_or_zr(rt);
                match self.sysreg_write(key, value) {
                    Ok(WriteEffect::None) => {}
                    Ok(WriteEffect::FlushTlb) => self.tlb.flush(),
                    Ok(WriteEffect::Timer) => self.timer_dirty = true,
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
        match crn {
            3 => match op2 {
                // CLREX
                2 => {
                    self.monitor.armed = false;
                    Ok(Flow::Next)
                }
                // DSB, DMB, ISB: an interpreter that retires each access in
                // order before the next has nothing to wait for.
                4..=6 => Ok(Flow::Next),
                _ => self.undefined(pc, insn),
            },
            4 => match (op1, op2) {
                // SPSel
                (0, 5) => {
                    if self.pstate.el == El::El0 {
                        return self.undefined(pc, insn);
                    }
                    self.pstate.sp_sel = crm & 1 != 0;
                    Ok(Flow::Next)
                }
                // DAIFSet / DAIFClr
                (3, 6) | (3, 7) => {
                    if self.pstate.el == El::El0 && self.sys.sctlr_el1 & SCTLR_UMA == 0 {
                        let key = SysRegKey::from_insn(insn);
                        return Err(Exception::new(ExceptionClass::SysRegTrap, key.trap_iss(rt, false)).into());
                    }
                    let bits = u64::from(crm) << 6;
                    let daif = self.pstate.daif();
                    self.pstate.set_daif(if op2 == 6 { daif | bits } else { daif & !bits });
                    Ok(Flow::Next)
                }
                // SVCRSM, SVCRZA, SVCRSMZA (FEAT_SME): SMSTART, SMSTOP.
                (3, 3) => self.exec_msr_svcr(pc, insn),
                // PAN, UAO, DIT, SSBS, TCO...: features this core lacks.
                _ => self.undefined(pc, insn),
            },
            _ => self.undefined(pc, insn),
        }
    }

    /// `SYS`: cache maintenance, address translation and TLB maintenance.
    #[allow(clippy::too_many_arguments)]
    fn exec_sys<B: PhysBus>(&mut self, bus: &mut B, pc: u64, insn: u32, op1: u32, crn: u32, crm: u32, op2: u32, rt: u32) -> ExecResult {
        let el0 = self.pstate.el == El::El0;
        let trap = Fault::from(Exception::new(
            ExceptionClass::SysRegTrap,
            SysRegKey::from_insn(insn).trap_iss(rt, false),
        ));
        match (op1, crn, crm, op2) {
            // IC IALLUIS, IC IALLU: there is no instruction cache to invalidate.
            (0, 7, 1, 0) | (0, 7, 5, 0) => {
                if el0 {
                    return self.undefined(pc, insn);
                }
                Ok(Flow::Next)
            }
            // IC IVAU, DC CVAC, DC CVAU, DC CIVAC: EL0 needs SCTLR_EL1.UCI.
            (3, 7, 5, 1) | (3, 7, 10, 1) | (3, 7, 11, 1) | (3, 7, 14, 1) => {
                if el0 && self.sys.sctlr_el1 & SCTLR_UCI == 0 {
                    return Err(trap);
                }
                Ok(Flow::Next)
            }
            // DC ZVA: EL0 needs SCTLR_EL1.DZE.
            (3, 7, 4, 1) => {
                if el0 && self.sys.sctlr_el1 & SCTLR_DZE == 0 {
                    return Err(trap);
                }
                let va = self.x_or_zr(rt);
                self.dc_zva(bus, va)?;
                Ok(Flow::Next)
            }
            // DC IVAC, DC ISW, DC CSW, DC CISW: EL1 only. With no data cache
            // model, invalidation and cleaning have nothing to act on.
            (0, 7, 6, 1) | (0, 7, 6, 2) | (0, 7, 10, 2) | (0, 7, 14, 2) => {
                if el0 {
                    return self.undefined(pc, insn);
                }
                Ok(Flow::Next)
            }
            // AT S1E1R, S1E1W, S1E0R, S1E0W.
            (0, 7, 8, 0..=3) => {
                if el0 {
                    return self.undefined(pc, insn);
                }
                let va = self.x_or_zr(rt);
                let as_el = if op2 >= 2 { El::El0 } else { El::El1 };
                self.address_translate(bus, va, op2 & 1 == 1, as_el);
                Ok(Flow::Next)
            }
            // TLBI VMALLE1IS, VAE1IS, ASIDE1IS, VAAE1IS, VALE1IS, VAALE1IS
            // (CRm 3) and VMALLE1, VAE1, ASIDE1, VAAE1, VALE1, VAALE1 (CRm 7),
            // the ARMv8.0 EL1 TLB maintenance instructions (Arm ARM TLBI
            // pages; QEMU target/arm/tcg/tlb-insns.c `tlbi_v8_cp_reginfo`).
            // Each invalidates at least what it names, so flushing everything
            // satisfies all of them. The rest of op1 0, CRn 8 belongs to
            // FEAT_TLBIOS and FEAT_TLBIRANGE.
            (0, 8, 3 | 7, 0 | 1 | 2 | 3 | 5 | 7) => {
                if el0 {
                    return self.undefined(pc, insn);
                }
                self.tlb.flush();
                Ok(Flow::Next)
            }
            _ => self.undefined(pc, insn),
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
            (0, 2) => {
                // HVC is UNDEFINED at EL0. At EL1, with no EL2 to take it,
                // QEMU's `virt` (PSCI conduit "hvc") services every HVC as a
                // firmware call whatever its immediate and function ID
                // (target/arm/tcg/psci.c `arm_is_psci_call`), answering the
                // IDs it does not know with NOT_SUPPORTED.
                if self.pstate.el == El::El1 {
                    return self.psci_call();
                }
                self.undefined(pc, insn)
            }
            // SMC: EL3 is not implemented.
            (0, 3) => self.undefined(pc, insn),
            (1, 0) => Err(Exception::new(ExceptionClass::Brk, imm16).into()),
            // HLT with halting debug disabled, and DCPS outside Debug state,
            // are UNDEFINED.
            _ => self.undefined(pc, insn),
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
                    return Ok(Flow::Wait);
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
