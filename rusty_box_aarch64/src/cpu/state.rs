//! The processor as a host sees it: named register access, the architectural
//! state at one moment, reset, and saving and restoring the whole processor.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use super::features::{Feature, Implementation};
use super::regs::{CpuState, Reg, RegClass, VReg};
use super::{Activity, Btype, Cpu, CpuConfig, El, ExclusiveMonitor, Pstate, SysRegs, PC_HISTORY};

/// `FPCR` bits a processor implementing `imp` keeps: AHP, DN, FZ and RMode,
/// and FZ16 with FEAT_FP16 (RES0 without it). The trap enables read as zero
/// on a core without trapped floating-point exceptions.
pub(crate) const fn fpcr_mask(imp: Implementation) -> u64 {
    const BASE: u64 = 0x07C0_0000;
    if imp.has(Feature::Fp16) {
        BASE | crate::softfloat::FPCR_FZ16
    } else {
        BASE
    }
}

/// `FPSR` bits this core keeps: N, Z, C, V, QC and the cumulative exception
/// flags.
pub(crate) const FPSR_MASK: u64 =
    0xF000_0000 | super::simd_arith::FPSR_QC | crate::softfloat::FPSR_CUMULATIVE_MASK;

/// How many system register values a snapshot carries, in
/// [`SysRegs::to_array`] order.
const SYSREG_COUNT: usize = 44;

impl SysRegs {
    fn to_array(&self) -> [u64; SYSREG_COUNT] {
        [
            self.sctlr_el1,
            self.actlr_el1,
            self.cpacr_el1,
            self.ttbr0_el1,
            self.ttbr1_el1,
            self.tcr_el1,
            self.spsr_el1,
            self.elr_el1,
            self.esr_el1,
            self.far_el1,
            self.afsr0_el1,
            self.afsr1_el1,
            self.par_el1,
            self.mair_el1,
            self.amair_el1,
            self.vbar_el1,
            self.contextidr_el1,
            self.tpidr_el0,
            self.tpidrro_el0,
            self.tpidr_el1,
            self.tpidr2_el0,
            self.smcr_el1,
            self.csselr_el1,
            self.cntkctl_el1,
            self.cntfrq_el0,
            self.cntp_ctl_el0,
            self.cntp_cval_el0,
            self.cntv_ctl_el0,
            self.cntv_cval_el0,
            self.mdscr_el1,
            self.mdccint_el1,
            self.oslsr_el1,
            self.osdlr_el1,
            self.oseccr_el1,
            self.dbgprcr_el1,
            self.dbgclaim,
            self.dbgbvr[0],
            self.dbgbvr[1],
            self.dbgbcr[0],
            self.dbgbcr[1],
            self.dbgwvr[0],
            self.dbgwvr[1],
            self.dbgwcr[0],
            self.dbgwcr[1],
        ]
    }

    fn from_array(a: [u64; SYSREG_COUNT]) -> Self {
        let [sctlr_el1, actlr_el1, cpacr_el1, ttbr0_el1, ttbr1_el1, tcr_el1, spsr_el1, elr_el1, esr_el1, far_el1, afsr0_el1, afsr1_el1, par_el1, mair_el1, amair_el1, vbar_el1, contextidr_el1, tpidr_el0, tpidrro_el0, tpidr_el1, tpidr2_el0, smcr_el1, csselr_el1, cntkctl_el1, cntfrq_el0, cntp_ctl_el0, cntp_cval_el0, cntv_ctl_el0, cntv_cval_el0, mdscr_el1, mdccint_el1, oslsr_el1, osdlr_el1, oseccr_el1, dbgprcr_el1, dbgclaim, bvr0, bvr1, bcr0, bcr1, wvr0, wvr1, wcr0, wcr1] = a;
        Self {
            sctlr_el1,
            actlr_el1,
            cpacr_el1,
            ttbr0_el1,
            ttbr1_el1,
            tcr_el1,
            spsr_el1,
            elr_el1,
            esr_el1,
            far_el1,
            afsr0_el1,
            afsr1_el1,
            par_el1,
            mair_el1,
            amair_el1,
            vbar_el1,
            contextidr_el1,
            tpidr_el0,
            tpidrro_el0,
            tpidr_el1,
            tpidr2_el0,
            smcr_el1,
            csselr_el1,
            cntkctl_el1,
            cntfrq_el0,
            cntp_ctl_el0,
            cntp_cval_el0,
            cntv_ctl_el0,
            cntv_cval_el0,
            mdscr_el1,
            mdccint_el1,
            oslsr_el1,
            osdlr_el1,
            oseccr_el1,
            dbgprcr_el1,
            dbgclaim,
            dbgbvr: [bvr0, bvr1],
            dbgbcr: [bcr0, bcr1],
            dbgwvr: [wvr0, wvr1],
            dbgwcr: [wcr0, wcr1],
        }
    }
}

impl Cpu {
    /// Instructions retired since the machine was built.
    #[inline]
    pub fn instructions(&self) -> u64 {
        self.retired
    }

    /// QEMU's `arm_cpu_reset` for a core whose highest level is EL1: every
    /// register to its reset value and execution at `reset_pc` in EL1h with
    /// interrupts masked. Time is the machine's, not the processor's, so the
    /// tick and instruction counts carry on, as QEMU's virtual clock does
    /// across a system reset.
    pub(crate) fn reset_keeping_time(&mut self, reset_pc: u64) {
        let fresh = Cpu::implementing(
            CpuConfig {
                ips: self.ips,
                counter_hz: self.counter_hz,
                reset_pc,
            },
            self.imp,
        );
        let pc_history = core::mem::take(&mut self.pc_history);
        *self = Cpu {
            icount: self.icount,
            retired: self.retired,
            debug_stops: self.debug_stops,
            pc_history,
            ..fresh
        };
    }

    /// The value of `reg`.
    pub fn reg(&self, reg: Reg) -> u64 {
        match reg.class() {
            RegClass::General(n) => self.x[n],
            RegClass::Sp => self.sp_current(),
            RegClass::SpEl0 => self.sp_el0,
            RegClass::SpEl1 => self.sp_el1,
            RegClass::Pc => self.pc,
            RegClass::Nzcv => self.pstate.nzcv(),
            RegClass::Daif => self.pstate.daif(),
            RegClass::Fpcr => self.fpcr,
            RegClass::Fpsr => self.fpsr,
        }
    }

    /// Set `reg`. `Nzcv`, `Daif`, `Fpcr` and `Fpsr` keep only the bits an
    /// `MSR` would. Writing `Pc` moves execution, by no branch: PSTATE.BTYPE
    /// becomes 00, so no branch check applies at the new PC. A processor
    /// waiting for an interrupt keeps waiting.
    pub(crate) fn set_reg(&mut self, reg: Reg, value: u64) {
        match reg.class() {
            RegClass::General(n) => self.x[n] = value,
            RegClass::Sp => self.set_sp_current(value),
            RegClass::SpEl0 => self.sp_el0 = value,
            RegClass::SpEl1 => self.sp_el1 = value,
            RegClass::Pc => {
                self.pc = value;
                self.pstate.btype = Btype::None;
            }
            RegClass::Nzcv => self.pstate.set_nzcv(value),
            RegClass::Daif => self.pstate.set_daif(value),
            RegClass::Fpcr => self.fpcr = value & fpcr_mask(self.imp),
            RegClass::Fpsr => self.fpsr = value & FPSR_MASK,
        }
    }

    /// The SIMD&FP register `reg`.
    pub fn v(&self, reg: VReg) -> u128 {
        self.v[usize::from(reg.index())]
    }

    pub(crate) fn set_v(&mut self, reg: VReg, value: u128) {
        self.write_v(usize::from(reg.index()), value);
    }

    /// Replace `PSTATE` with what of `pstate` this processor can hold.
    pub(crate) fn set_pstate(&mut self, pstate: Pstate) {
        self.pstate = self.held(pstate);
    }

    /// `pstate` as this processor holds it: at EL0 the only stack pointer
    /// is `SP_EL0`, so `sp_sel` is cleared there, and a field of a feature
    /// the processor lacks is clear (`btype` 00).
    pub(crate) fn held(&self, pstate: Pstate) -> Pstate {
        let has = |feature| self.implements(feature);
        Pstate {
            sp_sel: pstate.sp_sel && pstate.el == El::El1,
            btype: if has(Feature::Bti) { pstate.btype } else { Btype::None },
            pan: pstate.pan && has(Feature::Pan),
            uao: pstate.uao && has(Feature::Uao),
            dit: pstate.dit && has(Feature::Dit),
            ssbs: pstate.ssbs && has(Feature::Ssbs),
            ..pstate
        }
    }

    /// The `SPSR_EL1` bits that hold a field of this processor's `PSTATE`:
    /// [`Pstate::BASE`], and those of the fields [`Cpu::held`] keeps. The
    /// rest are RES0.
    pub(crate) fn spsr_mask(&self) -> u64 {
        let every = Pstate {
            btype: Btype::Jump,
            pan: true,
            uao: true,
            dit: true,
            ssbs: true,
            ..Pstate::RESET
        };
        Pstate::BASE | self.held(every).to_spsr()
    }

    /// A copy of the architectural state now.
    pub fn state(&self) -> CpuState {
        CpuState {
            x: self.x,
            sp_el0: self.sp_el0,
            sp_el1: self.sp_el1,
            pc: self.pc,
            pstate: self.pstate,
            v: self.v,
            fpcr: self.fpcr,
            fpsr: self.fpsr,
            elr_el1: self.sys.elr_el1,
            spsr_el1: self.sys.spsr_el1,
            esr_el1: self.sys.esr_el1,
            far_el1: self.sys.far_el1,
            vbar_el1: self.sys.vbar_el1,
            sctlr_el1: self.sys.sctlr_el1,
            tcr_el1: self.sys.tcr_el1,
            ttbr0_el1: self.sys.ttbr0_el1,
            ttbr1_el1: self.sys.ttbr1_el1,
            mair_el1: self.sys.mair_el1,
            activity: self.activity,
            instructions: self.retired,
            ticks: self.icount,
        }
    }

    /// Everything that decides what the processor does next, TLB included:
    /// a translation cached before the save is cached after the restore, so
    /// a guest that changed a table without invalidating sees the same stale
    /// entry either way. The decode cache is not saved — it is keyed by the
    /// instruction word and cannot change an outcome.
    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_u64(self.imp.features.bits())?;
        w.write_u32(self.imp.max_svl.bits())?;
        for x in self.x {
            w.write_u64(x)?;
        }
        w.write_u64(self.sp_el0)?;
        w.write_u64(self.sp_el1)?;
        w.write_u64(self.pc)?;
        w.write_u64(self.pstate.to_spsr())?;
        for v in self.v {
            w.write_u64(v as u64)?;
            w.write_u64((v >> 64) as u64)?;
        }
        w.write_u64(self.fpcr)?;
        w.write_u64(self.fpsr)?;
        for value in self.sys.to_array() {
            w.write_u64(value)?;
        }
        w.write_bool(self.monitor.armed)?;
        w.write_u64(self.monitor.addr)?;
        w.write_u64(self.monitor.bytes)?;
        w.write_bool(self.activity == Activity::WaitingForInterrupt)?;
        w.write_u64(self.icount)?;
        w.write_u64(self.retired)?;
        self.tlb.save(w)?;
        w.write_u8(self.pc_history.len() as u8)?;
        for pc in self.pc_history.iter() {
            w.write_u64(pc)?;
        }
        self.save_sme(w)?;
        self.save_pauth(w)
    }

    /// The inverse of [`Cpu::save`]. The interrupt inputs are not part of
    /// the image: the machine drives them from its interrupt controller.
    /// An image of a processor implementing other features, or another
    /// streaming vector length, is refused before anything changes.
    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let features = r.read_u64()?;
        let max_svl = r.read_u32()?;
        if features != self.imp.features.bits() || max_svl != self.imp.max_svl.bits() {
            return Err(SnapError::Invalid("snapshot of a processor implementing other features"));
        }
        let mut x = [0u64; 31];
        for reg in x.iter_mut() {
            *reg = r.read_u64()?;
        }
        let sp_el0 = r.read_u64()?;
        let sp_el1 = r.read_u64()?;
        let pc = r.read_u64()?;
        let pstate = Pstate::from_spsr(r.read_u64()?)
            .ok_or(SnapError::Invalid("processor state names an exception level or mode this core lacks"))?;
        if self.held(pstate) != pstate {
            return Err(SnapError::Invalid("processor state holds a field of a feature this processor lacks"));
        }
        let mut v = [0u128; 32];
        for reg in v.iter_mut() {
            let lo = r.read_u64()?;
            let hi = r.read_u64()?;
            *reg = u128::from(lo) | (u128::from(hi) << 64);
        }
        let fpcr = r.read_u64()? & fpcr_mask(self.imp);
        let fpsr = r.read_u64()? & FPSR_MASK;
        let mut sys = [0u64; SYSREG_COUNT];
        for value in sys.iter_mut() {
            *value = r.read_u64()?;
        }
        let sys = SysRegs::from_array(sys);
        if !self.sme_registers_restorable(&sys) {
            return Err(SnapError::Invalid("snapshot holds SME register bits this processor cannot set"));
        }
        if sys.spsr_el1 & !self.spsr_mask() != 0 {
            return Err(SnapError::Invalid("snapshot's SPSR_EL1 holds bits this processor cannot set"));
        }
        let monitor = ExclusiveMonitor {
            armed: r.read_bool()?,
            addr: r.read_u64()?,
            bytes: r.read_u64()?,
        };
        let activity = if r.read_bool()? {
            Activity::WaitingForInterrupt
        } else {
            Activity::Running
        };
        let icount = r.read_u64()?;
        let retired = r.read_u64()?;
        if retired > icount {
            return Err(SnapError::Invalid("processor retired more instructions than ticks passed"));
        }
        self.tlb.restore(r, self.imp.features)?;
        let history_len = usize::from(r.read_u8()?);
        if history_len > PC_HISTORY {
            return Err(SnapError::Invalid("processor PC history is longer than it holds"));
        }
        self.pc_history.clear();
        for _ in 0..history_len {
            self.pc_history.push_back(r.read_u64()?);
        }
        self.x = x;
        self.sp_el0 = sp_el0;
        self.sp_el1 = sp_el1;
        self.pc = pc;
        self.pstate = pstate;
        self.v = v;
        self.fpcr = fpcr;
        self.fpsr = fpsr;
        self.sys = sys;
        self.monitor = monitor;
        self.activity = activity;
        self.icount = icount;
        self.retired = retired;
        self.timer_dirty = true;
        self.restore_sme(r)?;
        self.restore_pauth(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_core::ClockHz;

    fn cpu() -> Cpu {
        Cpu::new(CpuConfig {
            ips: ClockHz::BOCHS_DEFAULT,
            counter_hz: ClockHz::BOCHS_DEFAULT,
            reset_pc: 0x1000,
        })
    }

    #[test]
    fn spsr_and_pstate_round_trip_and_illegal_modes_are_refused() {
        let p = Pstate {
            n: true,
            c: true,
            el: El::El0,
            sp_sel: false,
            ..Pstate::RESET
        };
        assert_eq!(Pstate::from_spsr(p.to_spsr()), Some(p));
        assert_eq!(Pstate::from_spsr(0x10), None, "AArch32");
        assert_eq!(Pstate::from_spsr(0x09), None, "EL2h");
        assert_eq!(Pstate::from_spsr(0x01), None, "EL0 with SP_EL1 selected");
    }

    #[test]
    fn named_registers_reach_the_register_file() {
        let mut c = cpu();
        c.set_reg(Reg::X7, 0xDEAD);
        c.set_reg(Reg::Sp, 0x8000);
        c.set_reg(Reg::Nzcv, u64::MAX);
        assert_eq!(c.reg(Reg::X7), 0xDEAD);
        assert_eq!(c.reg(Reg::SpEl1), 0x8000, "EL1h selects SP_EL1");
        assert_eq!(c.reg(Reg::Nzcv), 0xF000_0000, "only NZCV's bits are kept");
        assert_eq!(Reg::x(31), None);
        assert_eq!(Reg::x(30), Some(Reg::X30));
    }

    #[test]
    fn a_reset_keeps_time_and_restores_reset_values() {
        let mut c = cpu();
        c.icount = 500;
        c.retired = 400;
        c.set_reg(Reg::X0, 1);
        c.sys.sctlr_el1 = 1;
        c.reset_keeping_time(0x2000);
        assert_eq!((c.icount, c.retired), (500, 400));
        assert_eq!(c.reg(Reg::X0), 0);
        assert_eq!(c.reg(Reg::Pc), 0x2000);
        assert_eq!(c.sys.sctlr_el1, super::super::id::SCTLR_EL1_RESET);
    }

    #[test]
    fn a_processor_snapshot_restores_the_same_state() {
        let mut c = cpu();
        c.set_reg(Reg::X3, 33);
        c.set_v(VReg::new(5).expect("V5 exists"), 0x1234_5678_9ABC_DEF0_1111_2222_3333_4444);
        c.sys.vbar_el1 = 0x8000;
        c.icount = 99;
        c.retired = 90;
        c.pc_history.push_back(0x40);
        let mut image = std::vec::Vec::new();
        c.save(&mut image).expect("save");
        let mut restored = cpu();
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restore");
        assert!(source.is_empty());
        assert_eq!(restored.state(), c.state());
        assert_eq!(restored.recent_pcs().collect::<std::vec::Vec<_>>(), [0x40]);
    }
}
