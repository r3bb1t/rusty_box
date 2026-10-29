//! The processor as a host sees it: named register access, the architectural
//! state at one moment, reset, and saving and restoring the whole processor.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use super::el2::{El2State, El2Storage};
use super::features::{Feature, Implementation};
use super::regs::{CpuState, CpuStateEl2, PstateRefusal, Reg, RegClass, VReg};
use super::sysreg::{MDSCR_FIELDS, OSDLR_DLK, RMR_RR};
use super::timer::VirtualCount;
use super::wait::{EventRegister, Wait};
use super::{
    pauth, sme, Activity, Btype, Cpu, El, ExclusiveMonitor, Levels, Pstate, SysRegs, PC_HISTORY,
};

/// How a snapshot names the processor's activity.
mod activity_tag {
    pub(super) const RUNNING: u8 = 0;
    pub(super) const INTERRUPT: u8 = 1;
    pub(super) const INTERRUPT_UNTIL: u8 = 2;
    pub(super) const EVENT: u8 = 3;
    pub(super) const EVENT_UNTIL: u8 = 4;
}

/// `FPCR` bits a processor implementing `imp` keeps: AHP, DN, FZ and RMode,
/// FZ16 with FEAT_FP16, and FIZ, AH and NEP with FEAT_AFP (RES0 without
/// them). The trap enables read as zero on a core without trapped
/// floating-point exceptions.
pub(crate) const fn fpcr_mask(imp: Implementation) -> u64 {
    use crate::softfloat::{FPCR_AH, FPCR_FIZ, FPCR_FZ16, FPCR_NEP};
    const BASE: u64 = 0x07C0_0000;
    let fp16 = if imp.has(Feature::Fp16) { FPCR_FZ16 } else { 0 };
    let afp = if imp.has(Feature::Afp) {
        FPCR_FIZ | FPCR_AH | FPCR_NEP
    } else {
        0
    };
    BASE | fp16 | afp
}

/// `FPSR` bits this core keeps: N, Z, C, V, QC and the cumulative exception
/// flags.
pub(crate) const FPSR_MASK: u64 =
    0xF000_0000 | super::simd_arith::FPSR_QC | crate::softfloat::FPSR_CUMULATIVE_MASK;

/// How many system register values a snapshot carries, in
/// [`SysRegs::to_array`] order.
const SYSREG_COUNT: usize = 49;

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
            self.disr_el1,
            u64::from(self.dtrrx),
            u64::from(self.dtrtx),
            self.rvbar,
            self.rmr,
        ]
    }

    /// The inverse of [`SysRegs::to_array`]. Refuses a Debug Communications
    /// Channel data register wider than its 32 bits, and an `MDSCR_EL1` or
    /// `RMR_ELx` with a bit set that no write stores.
    fn from_array(a: [u64; SYSREG_COUNT]) -> SnapResult<Self> {
        let [sctlr_el1, actlr_el1, cpacr_el1, ttbr0_el1, ttbr1_el1, tcr_el1, spsr_el1, elr_el1, esr_el1, far_el1, afsr0_el1, afsr1_el1, par_el1, mair_el1, amair_el1, vbar_el1, contextidr_el1, tpidr_el0, tpidrro_el0, tpidr_el1, tpidr2_el0, smcr_el1, csselr_el1, cntkctl_el1, cntfrq_el0, cntp_ctl_el0, cntp_cval_el0, cntv_ctl_el0, cntv_cval_el0, mdscr_el1, mdccint_el1, oslsr_el1, osdlr_el1, oseccr_el1, dbgprcr_el1, dbgclaim, bvr0, bvr1, bcr0, bcr1, wvr0, wvr1, wcr0, wcr1, disr_el1, dtrrx, dtrtx, rvbar, rmr] =
            a;
        if rmr & !RMR_RR != 0 {
            return Err(SnapError::Invalid("RMR_ELx with a bit other than RR set"));
        }
        let data_register = |value: u64| {
            u32::try_from(value)
                .map_err(|_| SnapError::Invalid("debug channel data register wider than 32 bits"))
        };
        let dtrrx = data_register(dtrrx)?;
        let dtrtx = data_register(dtrtx)?;
        if mdscr_el1 & !MDSCR_FIELDS != 0 {
            return Err(SnapError::Invalid("MDSCR_EL1 with a RES0 bit set"));
        }
        Ok(Self {
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
            dtrrx,
            dtrtx,
            disr_el1,
            rvbar,
            rmr,
        })
    }
}

impl<L: Levels> Cpu<L> {
    /// Instructions retired since the machine was built.
    #[inline]
    pub fn instructions(&self) -> u64 {
        self.retired
    }

    /// Arm `AArch64_TakeReset`: every register to its reset value and
    /// execution at `reset_pc` at the highest implemented level, EL2h with
    /// FEAT_EL2 and EL1h without, with interrupts masked. Time is the
    /// machine's, not the processor's, so the tick and instruction counts
    /// carry on across a system reset, as the board's counter does.
    /// `RVBAR_ELx` reads `reset_pc` afterwards, the address this reset
    /// started execution from.
    ///
    /// The processor is reset where it is, rather than replaced by a new
    /// one: it holds its trace cache, which is far larger than the rest of
    /// it and would be a second copy on the stack. Every field is named, so
    /// a field added to [`Cpu`] has to be decided here; the values are
    /// [`Cpu::implementing`]'s, which
    /// `a_reset_keeps_time_and_restores_reset_values` checks.
    pub(crate) fn reset_keeping_time(&mut self, reset_pc: u64) {
        let count = self.counter();
        let Cpu {
            x,
            sp_el0,
            sp_el1,
            el2_state,
            pc,
            pstate,
            v,
            fpcr,
            fpsr,
            svcr,
            scalable,
            pac_keys,
            imp,
            sys,
            monitor,
            event,
            tlb,
            traces,
            trace_break,
            in_guarded_page,
            activity,
            icount: _,
            retired,
            ips: _,
            timer_dirty,
            irq_line,
            fiq_line,
            debug_stops: _,
            history,
        } = self;
        *x = [0; 31];
        *sp_el0 = 0;
        *sp_el1 = 0;
        *el2_state = L::State::keep(El2State::reset(*imp));
        *pc = reset_pc;
        *pstate = Pstate::reset::<L>();
        *v = [0; 32];
        *fpcr = 0;
        *fpsr = 0;
        *svcr = sme::Svcr::RESET;
        *scalable = sme::ScalableRegs::ZERO;
        *pac_keys = pauth::PacKeys::ZERO;
        *sys = SysRegs::reset(imp.counter_hz, reset_pc);
        *monitor = ExclusiveMonitor::CLEAR;
        *event = EventRegister::at(false, count);
        tlb.flush();
        traces.flush();
        *trace_break = true;
        *in_guarded_page = false;
        *activity = Activity::Running;
        *timer_dirty = true;
        *irq_line = false;
        *fiq_line = false;
        history.jump(reset_pc, *retired);
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
    /// becomes 00, so no branch check applies at the new PC. A waiting
    /// processor keeps waiting.
    pub(crate) fn set_reg(&mut self, reg: Reg, value: u64) {
        match reg.class() {
            RegClass::General(n) => self.x[n] = value,
            RegClass::Sp => self.set_sp_current(value),
            RegClass::SpEl0 => self.sp_el0 = value,
            RegClass::SpEl1 => self.sp_el1 = value,
            RegClass::Pc => {
                self.pc = value;
                self.pstate.btype = Btype::None;
                self.history.jump(value, self.retired);
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

    /// Replace `PSTATE` with what of `pstate` this processor can hold, unless
    /// it names a level the processor lacks: a field of a feature it lacks
    /// is a bit it holds as zero, but no level stands in for an absent one.
    pub(crate) fn set_pstate(&mut self, pstate: Pstate) -> Result<(), PstateRefusal> {
        if pstate.el > self.highest_el() {
            return Err(PstateRefusal::AbsentLevel);
        }
        self.pstate = self.held(pstate);
        Ok(())
    }

    /// `pstate`, at a level this processor implements (its callers refuse
    /// any other), as the processor holds it: at EL0 the only stack pointer
    /// is `SP_EL0`, so `sp_sel` is cleared there, and a field of a feature
    /// the processor lacks is clear (`btype` 00).
    pub(crate) fn held(&self, pstate: Pstate) -> Pstate {
        let has = |feature| self.implements(feature);
        Pstate {
            sp_sel: pstate.sp_sel && pstate.el != El::El0,
            btype: if has(Feature::Bti) {
                pstate.btype
            } else {
                Btype::None
            },
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
            el2: self.el2_state.get().map(|state| CpuStateEl2 {
                sp_el2: state.sp_el2,
                elr_el2: state.regs.elr,
                spsr_el2: state.regs.spsr,
                esr_el2: state.regs.esr,
                far_el2: state.regs.far,
                vbar_el2: state.regs.vbar,
                sctlr_el2: state.regs.sctlr,
                hcr_el2: state.regs.hcr,
            }),
            activity: self.activity,
            instructions: self.retired,
            ticks: self.icount,
        }
    }

    /// Everything that decides what the processor does next, TLB included:
    /// a translation cached before the save is cached after the restore, so
    /// a guest that changed a table without invalidating sees the same stale
    /// entry either way. The trace cache is not saved: it is kept coherent
    /// with memory, so it cannot change an outcome, and a restore empties
    /// it.
    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        let features = self.imp.features.bits();
        w.write_u64(features as u64)?;
        w.write_u64((features >> 64) as u64)?;
        w.write_u32(self.imp.max_svl.bits())?;
        w.write_u64(self.imp.midr)?;
        w.write_u64(self.imp.counter_hz.get())?;
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
        self.save_el2(w)?;
        w.write_bool(self.monitor.armed)?;
        w.write_u64(self.monitor.addr)?;
        w.write_u64(self.monitor.va)?;
        w.write_u64(self.monitor.bytes)?;
        let (tag, timeout) = match self.activity {
            Activity::Running => (activity_tag::RUNNING, None),
            Activity::Waiting(Wait::Interrupt) => (activity_tag::INTERRUPT, None),
            Activity::Waiting(Wait::InterruptUntil(t)) => (activity_tag::INTERRUPT_UNTIL, Some(t)),
            Activity::Waiting(Wait::Event) => (activity_tag::EVENT, None),
            Activity::Waiting(Wait::EventUntil(t)) => (activity_tag::EVENT_UNTIL, Some(t)),
        };
        w.write_u8(tag)?;
        if let Some(t) = timeout {
            w.write_u64(t.get())?;
        }
        w.write_bool(self.event_register())?;
        w.write_u64(self.icount)?;
        w.write_u64(self.retired)?;
        self.tlb.save(w)?;
        let recent = self.recent_pcs();
        w.write_u8(recent.len() as u8)?;
        for pc in recent {
            w.write_u64(pc)?;
        }
        self.save_sme(w)?;
        self.save_pauth(w)
    }

    /// The inverse of [`Cpu::save`]. The interrupt inputs are not part of
    /// the image: the machine drives them from its interrupt controller.
    /// An image of another implementation — other features, another
    /// streaming vector length, another `MIDR_EL1` or another system
    /// counter — is refused before anything changes.
    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let features = u128::from(r.read_u64()?) | (u128::from(r.read_u64()?) << 64);
        let max_svl = r.read_u32()?;
        let midr = r.read_u64()?;
        let counter_hz = r.read_u64()?;
        if features != self.imp.features.bits() || max_svl != self.imp.max_svl.bits() {
            return Err(SnapError::Invalid(
                "snapshot of a processor implementing other features",
            ));
        }
        if midr != self.imp.midr || counter_hz != self.imp.counter_hz.get() {
            return Err(SnapError::Invalid("snapshot of another processor"));
        }
        let mut x = [0u64; 31];
        for reg in x.iter_mut() {
            *reg = r.read_u64()?;
        }
        let sp_el0 = r.read_u64()?;
        let sp_el1 = r.read_u64()?;
        let pc = r.read_u64()?;
        let pstate = Pstate::from_spsr(r.read_u64()?).ok_or(SnapError::Invalid(
            "processor state names an exception level or mode this core lacks",
        ))?;
        if pstate.el > self.highest_el() {
            return Err(SnapError::Invalid(
                "processor state names an exception level this processor lacks",
            ));
        }
        if self.held(pstate) != pstate {
            return Err(SnapError::Invalid(
                "processor state holds a field of a feature this processor lacks",
            ));
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
        let el2 = self.read_el2(r)?;
        let sys = SysRegs::from_array(sys)?;
        if !self.sme_registers_restorable(&sys) {
            return Err(SnapError::Invalid(
                "snapshot holds SME register bits this processor cannot set",
            ));
        }
        if sys.spsr_el1 & !self.spsr_mask() != 0 {
            return Err(SnapError::Invalid(
                "snapshot's SPSR_EL1 holds bits this processor cannot set",
            ));
        }
        if sys.cntkctl_el1 & !self.cntkctl_mask() != 0 {
            return Err(SnapError::Invalid(
                "snapshot's CNTKCTL_EL1 holds bits this processor cannot set",
            ));
        }
        if sys.disr_el1 & !self.disr_mask() != 0 {
            return Err(SnapError::Invalid(
                "snapshot's DISR_EL1 holds bits this processor cannot set",
            ));
        }
        let osdlr_mask = if self.implements(Feature::DoubleLock) {
            OSDLR_DLK
        } else {
            0
        };
        if sys.osdlr_el1 & !osdlr_mask != 0 {
            return Err(SnapError::Invalid(
                "snapshot's OSDLR_EL1 holds bits this processor cannot set",
            ));
        }
        let monitor = ExclusiveMonitor {
            armed: r.read_bool()?,
            addr: r.read_u64()?,
            va: r.read_u64()?,
            bytes: r.read_u64()?,
        };
        let activity = match r.read_u8()? {
            activity_tag::RUNNING => Activity::Running,
            activity_tag::INTERRUPT => Activity::Waiting(Wait::Interrupt),
            activity_tag::INTERRUPT_UNTIL => {
                Activity::Waiting(Wait::InterruptUntil(VirtualCount::new(r.read_u64()?)))
            }
            activity_tag::EVENT => Activity::Waiting(Wait::Event),
            activity_tag::EVENT_UNTIL => {
                Activity::Waiting(Wait::EventUntil(VirtualCount::new(r.read_u64()?)))
            }
            _ => {
                return Err(SnapError::Invalid(
                    "processor activity names a wait this core lacks",
                ))
            }
        };
        let timed = matches!(
            activity,
            Activity::Waiting(Wait::InterruptUntil(_) | Wait::EventUntil(_))
        );
        if timed && !self.implements(Feature::WfxT) {
            return Err(SnapError::Invalid(
                "processor waits with a timeout on a processor without FEAT_WFxT",
            ));
        }
        let event = r.read_bool()?;
        let icount = r.read_u64()?;
        let retired = r.read_u64()?;
        if retired > icount {
            return Err(SnapError::Invalid(
                "processor retired more instructions than ticks passed",
            ));
        }
        self.tlb.restore(r, self.imp.features, sys.tcr_el1)?;
        let history_len = usize::from(r.read_u8()?);
        if history_len > PC_HISTORY {
            return Err(SnapError::Invalid(
                "processor PC history is longer than it holds",
            ));
        }
        if history_len as u64 > retired {
            return Err(SnapError::Invalid(
                "processor PC history is longer than the instructions it retired",
            ));
        }
        let mut recent = [0u64; PC_HISTORY];
        for slot in recent.iter_mut().take(history_len) {
            *slot = r.read_u64()?;
        }
        self.history =
            super::history::PcHistory::from_addresses(&recent[..history_len], retired, pc);
        self.x = x;
        self.sp_el0 = sp_el0;
        self.sp_el1 = sp_el1;
        self.el2_state = el2;
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
        self.event = EventRegister::at(event, self.counter());
        self.timer_dirty = true;
        // The image's memory replaces the memory the traces were read from.
        self.flush_traces();
        self.restore_sme(r)?;
        self.restore_pauth(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::{CpuConfig, NoEl2};
    use rusty_box_core::ClockHz;

    fn cpu() -> Cpu<NoEl2> {
        Cpu::implementing(
            CpuConfig {
                ips: ClockHz::BOCHS_DEFAULT,
                reset_pc: 0x1000,
            },
            Implementation {
                counter_hz: ClockHz::BOCHS_DEFAULT,
                ..Implementation::CORTEX_A57
            },
        )
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
        assert_eq!(Pstate::from_spsr(0x0D), None, "EL3h");
        assert_eq!(Pstate::from_spsr(0x01), None, "EL0 with SP_EL1 selected");
        // EL2h is a state an SPSR encodes; a processor without EL2 refuses a
        // host's write of it.
        let el2h = Pstate::from_spsr(0x09).expect("EL2h");
        assert_eq!((el2h.el, el2h.sp_sel), (El::El2, true));
        let mut c = cpu();
        let before = c.pstate;
        assert_eq!(c.set_pstate(el2h), Err(PstateRefusal::AbsentLevel));
        assert_eq!(c.pstate, before, "nothing changed");
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
        c.set_reg(Reg::Sp, 0x8000);
        c.set_v(VReg::new(3).expect("V3 exists"), 7);
        c.sys.sctlr_el1 = 1;
        c.sys.vbar_el1 = 0x4000;
        c.sys.cntv_cval_el0 = 99;
        c.fpcr = fpcr_mask(c.imp);
        c.monitor.armed = true;
        c.send_event();
        c.activity = Activity::Waiting(Wait::Interrupt);
        c.set_interrupt_lines(true, true);
        c.reset_keeping_time(0x2000);
        // The Event Register is clear, with the event stream counted from now.
        assert_eq!(c.event, EventRegister::at(false, c.counter()));
        assert_eq!((c.icount, c.retired), (500, 400));
        assert_eq!(c.reg(Reg::Pc), 0x2000);
        // Everything else is as a processor built to start there has it.
        let mut fresh = Cpu::<NoEl2>::implementing(
            CpuConfig {
                ips: ClockHz::BOCHS_DEFAULT,
                reset_pc: 0x2000,
            },
            c.imp,
        );
        fresh.icount = 500;
        fresh.retired = 400;
        assert_eq!(c.state(), fresh.state());
        assert_eq!(c.sys.to_array(), fresh.sys.to_array());
        assert_eq!(c.monitor, fresh.monitor);
        assert_eq!(
            (c.irq_line, c.fiq_line, c.timer_dirty),
            (fresh.irq_line, fresh.fiq_line, fresh.timer_dirty)
        );
        assert_eq!(c.sys.sctlr_el1, super::super::id::SCTLR_EL1_RESET);
    }

    #[test]
    fn a_processor_snapshot_restores_the_same_state() {
        let mut c = cpu();
        c.set_reg(Reg::X3, 33);
        c.set_v(
            VReg::new(5).expect("V5 exists"),
            0x1234_5678_9ABC_DEF0_1111_2222_3333_4444,
        );
        c.sys.vbar_el1 = 0x8000;
        c.icount = 99;
        c.retired = 90;
        c.history = crate::cpu::history::PcHistory::from_addresses(&[0x40], 90, 0x1000);
        let mut image = std::vec::Vec::new();
        c.save(&mut image).expect("save");
        let mut restored = cpu();
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restore");
        assert!(source.is_empty());
        assert_eq!(restored.state(), c.state());
        assert_eq!(restored.recent_pcs().collect::<std::vec::Vec<_>>(), [0x40]);
    }

    /// Where an image's system registers start: after the implementation's
    /// identity (its features, two words; its streaming vector length,
    /// `MIDR_EL1` and counter frequency), X0-X30, SP_EL0, SP_EL1, PC,
    /// PSTATE, V0-V31, FPCR and FPSR.
    const SYSREGS_AT: usize = 16 + 4 + 8 + 8 + (31 + 4) * 8 + 32 * 16 + 2 * 8;

    #[test]
    fn a_processor_snapshot_with_a_debug_data_register_wider_than_32_bits_is_refused() {
        let mut c = cpu();
        c.sys.dtrrx = 0x1234_5678;
        c.sys.dtrtx = 0x9ABC_DEF0;
        let mut image = std::vec::Vec::new();
        c.save(&mut image).expect("save");
        // DTRRX and DTRTX are the two system registers before the last two,
        // RVBAR_ELx and RMR_ELx.
        let dtrrx = SYSREGS_AT + (SYSREG_COUNT - 4) * 8;
        assert_eq!(
            &image[dtrrx..dtrrx + 16],
            &[0x78, 0x56, 0x34, 0x12, 0, 0, 0, 0, 0xF0, 0xDE, 0xBC, 0x9A, 0, 0, 0, 0]
        );
        for slot in [dtrrx, dtrrx + 8] {
            let mut wide = image.clone();
            wide[slot + 4] = 1;
            let mut restored = cpu();
            assert!(
                restored.restore(&mut wide.as_slice()).is_err(),
                "bit 32 set at byte {slot}"
            );
            assert_eq!(
                restored.sys.to_array(),
                cpu().sys.to_array(),
                "a refused image changes nothing"
            );
        }
        let mut restored = cpu();
        restored.restore(&mut image.as_slice()).expect("restore");
        assert_eq!(
            (restored.sys.dtrrx, restored.sys.dtrtx),
            (0x1234_5678, 0x9ABC_DEF0)
        );
    }

    #[test]
    fn a_processor_snapshot_with_an_mdscr_el1_res0_bit_set_is_refused() {
        let mut c = cpu();
        c.sys.mdscr_el1 = MDSCR_FIELDS;
        let mut image = std::vec::Vec::new();
        c.save(&mut image).expect("save");
        // MDSCR_EL1 is the 30th system register.
        let mdscr = SYSREGS_AT + 29 * 8;
        assert_eq!(&image[mdscr..mdscr + 8], &MDSCR_FIELDS.to_le_bytes());
        for bit in (0..64).filter(|bit| MDSCR_FIELDS & (1 << bit) == 0) {
            let mut res0 = image.clone();
            res0[mdscr..mdscr + 8].copy_from_slice(&(MDSCR_FIELDS | 1 << bit).to_le_bytes());
            let mut restored = cpu();
            assert!(
                restored.restore(&mut res0.as_slice()).is_err(),
                "RES0 bit {bit} set"
            );
            assert_eq!(
                restored.sys.to_array(),
                cpu().sys.to_array(),
                "a refused image changes nothing"
            );
        }
        let mut restored = cpu();
        restored.restore(&mut image.as_slice()).expect("restore");
        assert_eq!(restored.sys.mdscr_el1, MDSCR_FIELDS);
    }

    /// `RMR_ELx` stores RR alone, so an image holding any other of its
    /// bits is refused; one holding RR restores it.
    #[test]
    fn a_processor_snapshot_with_an_rmr_bit_other_than_rr_is_refused() {
        let mut c = cpu();
        c.sys.rmr = RMR_RR;
        let mut image = std::vec::Vec::new();
        c.save(&mut image).expect("save");
        // RMR_ELx is the last system register.
        let rmr = SYSREGS_AT + (SYSREG_COUNT - 1) * 8;
        assert_eq!(&image[rmr..rmr + 8], &RMR_RR.to_le_bytes());
        for bit in (0..64).filter(|bit| RMR_RR & (1 << bit) == 0) {
            let mut other = image.clone();
            other[rmr..rmr + 8].copy_from_slice(&(RMR_RR | 1 << bit).to_le_bytes());
            let mut restored = cpu();
            assert!(
                restored.restore(&mut other.as_slice()).is_err(),
                "bit {bit} set"
            );
            assert_eq!(
                restored.sys.to_array(),
                cpu().sys.to_array(),
                "a refused image changes nothing"
            );
        }
        let mut restored = cpu();
        restored.restore(&mut image.as_slice()).expect("restore");
        assert_eq!(restored.sys.rmr, RMR_RR);
    }

    /// `OSDLR_EL1.DLK` is the Cortex-A57's to set (FEAT_DoubleLock) and
    /// RES0 on the Armv9.2 model, which Armv9.0 forbids the feature: an
    /// image holding it restores on the one and is refused by the other.
    #[test]
    fn a_processor_snapshot_with_osdlr_dlk_needs_the_os_double_lock() {
        // OSDLR_EL1 is the 33rd system register.
        let osdlr = SYSREGS_AT + 32 * 8;
        let mut c = cpu();
        c.sys.osdlr_el1 = OSDLR_DLK;
        let mut image = std::vec::Vec::new();
        c.save(&mut image).expect("save");
        assert_eq!(&image[osdlr..osdlr + 8], &OSDLR_DLK.to_le_bytes());
        let mut restored = cpu();
        restored.restore(&mut image.as_slice()).expect("restore");
        assert_eq!(restored.sys.osdlr_el1, OSDLR_DLK);

        let armv9 = || {
            Cpu::<NoEl2>::implementing(
                CpuConfig {
                    ips: ClockHz::BOCHS_DEFAULT,
                    reset_pc: 0x1000,
                },
                Implementation::ARMV9P2,
            )
        };
        let mut image = std::vec::Vec::new();
        armv9().save(&mut image).expect("save");
        assert_eq!(&image[osdlr..osdlr + 8], &[0; 8]);
        image[osdlr] = 1;
        let mut restored = armv9();
        assert_eq!(
            restored.restore(&mut image.as_slice()),
            Err(SnapError::Invalid(
                "snapshot's OSDLR_EL1 holds bits this processor cannot set"
            ))
        );
        assert_eq!(
            restored.sys.to_array(),
            armv9().sys.to_array(),
            "a refused image changes nothing"
        );
    }

    /// An image is refused by a processor of another identity or another
    /// system counter, with the same features, before anything changes.
    #[test]
    fn an_image_of_another_processor_is_refused() {
        let mut c = cpu();
        c.set_reg(Reg::X3, 33);
        let mut image = std::vec::Vec::new();
        c.save(&mut image).expect("save");
        let others = [
            Implementation {
                midr: super::super::id::MIDR_SOFTWARE_MODEL,
                ..c.imp
            },
            Implementation {
                counter_hz: super::super::ARMV8P6_COUNTER_HZ,
                ..c.imp
            },
        ];
        for imp in others {
            let mut other = Cpu::<NoEl2>::implementing(
                CpuConfig {
                    ips: ClockHz::BOCHS_DEFAULT,
                    reset_pc: 0x1000,
                },
                imp,
            );
            let before = other.state();
            let mut source: &[u8] = &image;
            assert_eq!(
                other.restore(&mut source),
                Err(SnapError::Invalid("snapshot of another processor"))
            );
            assert_eq!(other.state(), before);
        }
    }
}
