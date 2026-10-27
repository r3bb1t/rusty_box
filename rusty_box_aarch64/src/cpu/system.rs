//! System instructions (Arm ARM C5): hints, barriers, PSTATE access, cache
//! and TLB maintenance, address translation, `MRS`/`MSR`, and the
//! exception-generating instructions — including the PSCI calls QEMU's
//! `virt` answers on `HVC` (its device tree says `method = "hvc"`).

use super::exception::{Exception, ExceptionClass};
use super::exec::{bit, bits, ExecResult, Fault, Flow, StopCause};
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

    /// The processor's affinity value, the form a PSCI call names it in:
    /// `MPIDR_EL1`'s Aff3 (bits 39:32) and Aff2..Aff0 (bits 23:0), as QEMU's
    /// `arm_cpu_mp_affinity` gives it.
    pub(super) const AFFINITY: u64 = crate::cpu::id::MPIDR_EL1 & 0xFF_00FF_FFFF;
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

    fn sysreg_fault(&self, pc: u64, insn: u32, err: SysRegError, key: SysRegKey, rt: u32, read: bool) -> ExecResult {
        match err {
            SysRegError::Undefined => self.undefined(pc, insn),
            SysRegError::TrapToEl1 => Err(Exception::new(ExceptionClass::SysRegTrap, key.trap_iss(rt, read)).into()),
            SysRegError::FpTrap => Err(self.fp_trap()),
        }
    }

    /// The HINT space: NOP, YIELD, WFE, WFI, SEV, SEVL; every other hint
    /// (including the pointer-authentication and BTI hints) is a NOP. A WFx
    /// traps from EL0 only when it would wait (Arm ARM `Hint_WFE`,
    /// `Hint_WFI`; QEMU target/arm/tcg/op_helper.c `HELPER(wfe)`,
    /// `HELPER(wfi)`).
    pub(crate) fn exec_hint(&mut self, insn: u32, _pc: u64) -> ExecResult {
        match bits(insn, 11, 5) {
            // WFE: a set Event Register is consumed and the instruction
            // completes. Otherwise it may wait; there is no other processor
            // to send an event, and WFE may complete at any time, so it
            // completes at once.
            2 => {
                if self.take_event() {
                    return Ok(Flow::Next);
                }
                if self.pstate.el == El::El0 && self.sys.sctlr_el1 & SCTLR_NTWE == 0 {
                    return Err(Exception::new(ExceptionClass::WfxTrap, (1 << 24) | (0xE << 20) | 1).into());
                }
                Ok(Flow::Next)
            }
            // WFI: a pending interrupt, masked or not (`InterruptPending`),
            // ends it before it would wait.
            3 => {
                if self.irq_line || self.fiq_line {
                    return Ok(Flow::Next);
                }
                if self.pstate.el == El::El0 && self.sys.sctlr_el1 & SCTLR_NTWI == 0 {
                    return Err(Exception::new(ExceptionClass::WfxTrap, (1 << 24) | (0xE << 20)).into());
                }
                Ok(Flow::Wait)
            }
            // SEV signals every processor, and this is the only one; SEVL
            // signals this one.
            4 | 5 => {
                self.send_event_local();
                Ok(Flow::Next)
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
                    Ok(WriteEffect::FlushTlb) => self.flush_tlb(),
                    Ok(WriteEffect::Timer) => self.timer_dirty = true,
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
            // IC IALLUIS, IC IALLU (and IC IVAU, below): the processor's
            // decoded traces are its instruction cache, and they are never
            // stale. Every write that can reach their memory forgets the
            // traces it covers when it is made (`note_store`, the board's
            // DMA log, `mem_write`), so there is nothing left to
            // invalidate, as QEMU's system emulation, whose translation
            // blocks are kept coherent the same way, makes them no-ops
            // (target/arm/helper.c `v8_cp_reginfo`: IC_IALLUIS, IC_IALLU
            // and, outside CONFIG_USER_ONLY, IC_IVAU are ARM_CP_NOP).
            (0, 7, 1, 0) | (0, 7, 5, 0) => {
                if el0 {
                    return self.undefined(pc, insn);
                }
                Ok(Flow::Next)
            }
            // IC IVAU, DC CVAC, DC CVAU, DC CIVAC: EL0 needs SCTLR_EL1.UCI.
            // With no data cache model, cleaning has nothing to act on, and
            // the instruction cache is never stale (above).
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
                self.address_translate(bus, va, op2 & 1 == 1, as_el)?;
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
                self.flush_tlb();
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
            // A misaligned entry point is refused before the target is
            // looked up, and the target is the processor whose affinity
            // value equals the argument exactly (QEMU target/arm/arm-powerctl.c
            // `arm_set_cpu_on`, `arm_get_cpu_by_id`). The only processor is on.
            psci::CPU_ON_32 | psci::CPU_ON_64 => {
                if self.x[2] & 3 != 0 || self.x[1] != psci::AFFINITY {
                    psci::INVALID_PARAMETERS
                } else {
                    psci::ALREADY_ON
                }
            }
            psci::AFFINITY_INFO_32 | psci::AFFINITY_INFO_64 => {
                // Levels above 0 are always on; at level 0 only this
                // processor's affinity exists.
                if self.x[2] != 0 || self.x[1] == psci::AFFINITY {
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
