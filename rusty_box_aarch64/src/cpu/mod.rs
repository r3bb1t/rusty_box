//! The processor: architectural state and the interpreter that advances it.
//!
//! One `Cpu` is one processing element. It executes against any
//! [`PhysBus`](crate::bus::PhysBus) through `Cpu::step`, and keeps the
//! machine's time: `icount` is the tick count every clock in the machine is
//! derived from (one tick per instruction, time skipped forward only while the
//! processor waits).

pub mod id;

#[cfg(test)]
#[path = "../../tests/unit/afp.rs"]
mod afp;
mod atomic;
mod bti;
#[cfg(test)]
#[path = "../../tests/unit/bti.rs"]
mod bti_tests;
mod crypto;
mod decode;
#[cfg(test)]
#[path = "../../tests/unit/difftest.rs"]
mod difftest;
mod dp;
mod exception;
mod exec;
mod features;
mod fp;
mod fp_estimate;
#[cfg(test)]
#[path = "../../tests/unit/fp_extensions.rs"]
mod fp_extensions;
mod isa;
mod ldst;
#[cfg(test)]
#[path = "../../tests/unit/memory_system.rs"]
mod memory_system;
mod mmu;
mod pauth;
#[cfg(test)]
#[path = "../../tests/unit/pointer_auth.rs"]
mod pointer_auth;
#[cfg(test)]
#[path = "../../tests/unit/pstate_features.rs"]
mod pstate_features;
mod regs;
mod sctlr;
mod simd;
mod simd_arith;
mod simd_bf16;
mod simd_fp;
mod sme;
mod state;
#[cfg(test)]
#[path = "../../tests/unit/step.rs"]
mod step;
mod sve;
mod sysreg;
mod system;
mod timer;
mod wait;
mod za;

use rusty_box_core::{ClockHz, RingBuffer, VmClock, VmInstant};

pub use exception::ExceptionClass;
pub use exec::{DebugStops, Step, StopCause, UndefinedInsn};
pub use mmu::{Tlb, TranslationFault};
pub use regs::{CpuState, Reg, SysReg, SysRegRefusal, VReg};
pub use timer::TimerLines;
pub use wait::Wait;

/// An exception level this processor implements.
///
/// Closed (R5): EL2 and EL3 do not exist on this core, and an `ERET` that
/// names them is an illegal exception return, not a third variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum El {
    El0,
    El1,
}

impl El {
    #[inline]
    pub const fn index(self) -> u64 {
        match self {
            Self::El0 => 0,
            Self::El1 => 1,
        }
    }
}

/// `PSTATE.BTYPE` (FEAT_BTI): the kind of indirect branch that led to the
/// instruction about to execute, which that instruction must accept when
/// it is in a guarded page (Arm `BranchTargetCheck`). Each variant is named
/// by the BTI that accepts it.
///
/// Closed (R5): the field's four values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Btype {
    /// 00: no indirect branch; every instruction but BR, BLR and their
    /// authenticating forms leaves this, and no check is made.
    None,
    /// 01: BR from X16 or X17, or from a page that is not guarded: a
    /// `BTI c`, `BTI j` or `BTI jc` accepts it.
    JumpOrCall,
    /// 10: BLR: a `BTI c` or `BTI jc` accepts it.
    Call,
    /// 11: BR from any other register in a guarded page: a `BTI j` or
    /// `BTI jc` accepts it.
    Jump,
}

impl Btype {
    /// The field's encoding.
    pub const fn bits(self) -> u64 {
        match self {
            Self::None => 0b00,
            Self::JumpOrCall => 0b01,
            Self::Call => 0b10,
            Self::Jump => 0b11,
        }
    }

    /// The value the low two bits of `bits` encode.
    pub const fn from_bits(bits: u64) -> Self {
        match bits & 0b11 {
            0b00 => Self::None,
            0b01 => Self::JumpOrCall,
            0b10 => Self::Call,
            _ => Self::Jump,
        }
    }
}

/// `PSTATE`, held as separate fields; [`Pstate::to_spsr`] and
/// [`Pstate::from_spsr`] are the only conversions to and from the packed
/// `SPSR` layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pstate {
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    pub d: bool,
    pub a: bool,
    pub i: bool,
    pub f: bool,
    pub el: El,
    /// `PSTATE.SP`: true selects `SP_ELx` for the current EL, false `SP_EL0`.
    pub sp_sel: bool,
    pub ss: bool,
    pub il: bool,
    /// [`Btype::None`] on a processor without FEAT_BTI.
    pub btype: Btype,
    /// `PSTATE.PAN` (FEAT_PAN): EL1 data accesses to memory EL0 can reach
    /// fault. False on a processor without the feature.
    pub pan: bool,
    /// `PSTATE.UAO` (FEAT_UAO): LDTR and STTR at EL1 are checked against
    /// EL1's permissions. False on a processor without the feature.
    pub uao: bool,
    /// `PSTATE.DIT` (FEAT_DIT): data-independent timing, which an
    /// interpreter's timing has whatever the bit says. False on a processor
    /// without the feature.
    pub dit: bool,
    /// `PSTATE.SSBS` (FEAT_SSBS): whether speculative loads may bypass
    /// stores, which an interpreter never speculates. False on a processor
    /// without the feature.
    pub ssbs: bool,
}

impl Pstate {
    /// The state out of reset (Arm `AArch64_TakeReset`): EL1 with `SP_EL1`,
    /// every interrupt masked, DIT clear. PAN and UAO, which reset leaves
    /// UNKNOWN, and SSBS, which it sets to an IMPLEMENTATION DEFINED value,
    /// are clear, as QEMU's `arm_cpu_reset_hold` leaves them.
    pub const RESET: Self = Self {
        n: false,
        z: false,
        c: false,
        v: false,
        d: true,
        a: true,
        i: true,
        f: true,
        el: El::El1,
        sp_sel: true,
        ss: false,
        il: false,
        btype: Btype::None,
        pan: false,
        uao: false,
        dit: false,
        ssbs: false,
    };

    /// `SPSR_ELx.SSBS`, and the bit the `SSBS` register reads it at.
    pub(crate) const SSBS: u64 = 1 << 12;
    /// `SPSR_ELx.PAN`, and the bit the `PAN` register reads it at.
    pub(crate) const PAN: u64 = 1 << 22;
    /// `SPSR_ELx.UAO`, and the bit the `UAO` register reads it at.
    pub(crate) const UAO: u64 = 1 << 23;
    /// `SPSR_ELx.DIT`, and the bit the `DIT` register reads it at.
    pub(crate) const DIT: u64 = 1 << 24;
    /// The `SPSR_ELx` bits of the fields every processor has: NZCV, SS, IL,
    /// DAIF and M.
    pub(crate) const BASE: u64 = 0xF030_03DF;

    /// The `SPSR_ELx` encoding of this state (AArch64 form, `M[4]` = 0).
    pub const fn to_spsr(self) -> u64 {
        let mut v = 0u64;
        v |= (self.n as u64) << 31;
        v |= (self.z as u64) << 30;
        v |= (self.c as u64) << 29;
        v |= (self.v as u64) << 28;
        if self.dit {
            v |= Self::DIT;
        }
        if self.uao {
            v |= Self::UAO;
        }
        if self.pan {
            v |= Self::PAN;
        }
        v |= (self.ss as u64) << 21;
        v |= (self.il as u64) << 20;
        if self.ssbs {
            v |= Self::SSBS;
        }
        v |= self.btype.bits() << 10;
        v |= (self.d as u64) << 9;
        v |= (self.a as u64) << 8;
        v |= (self.i as u64) << 7;
        v |= (self.f as u64) << 6;
        v |= self.el.index() << 2;
        v |= self.sp_sel as u64;
        v
    }

    /// The state an `SPSR_ELx` value describes, if it is one this processor
    /// can be in: AArch64 (`M[4]` clear), EL0 or EL1, and at EL0 the only stack
    /// pointer EL0 has.
    pub const fn from_spsr(spsr: u64) -> Option<Self> {
        let m = spsr & 0x1F;
        let el = match (m >> 2) & 3 {
            0 => El::El0,
            1 => El::El1,
            _ => return None,
        };
        if m & 0x10 != 0 || m & 0x2 != 0 || (matches!(el, El::El0) && m & 1 != 0) {
            return None;
        }
        Some(Self {
            n: spsr & (1 << 31) != 0,
            z: spsr & (1 << 30) != 0,
            c: spsr & (1 << 29) != 0,
            v: spsr & (1 << 28) != 0,
            d: spsr & (1 << 9) != 0,
            a: spsr & (1 << 8) != 0,
            i: spsr & (1 << 7) != 0,
            f: spsr & (1 << 6) != 0,
            el,
            sp_sel: m & 1 != 0,
            ss: spsr & (1 << 21) != 0,
            il: spsr & (1 << 20) != 0,
            btype: Btype::from_bits(spsr >> 10),
            pan: spsr & Self::PAN != 0,
            uao: spsr & Self::UAO != 0,
            dit: spsr & Self::DIT != 0,
            ssbs: spsr & Self::SSBS != 0,
        })
    }

    /// `NZCV` in its register layout (bits 31:28).
    #[inline]
    pub const fn nzcv(self) -> u64 {
        ((self.n as u64) << 31) | ((self.z as u64) << 30) | ((self.c as u64) << 29) | ((self.v as u64) << 28)
    }

    #[inline]
    pub fn set_nzcv(&mut self, bits: u64) {
        self.n = bits & (1 << 31) != 0;
        self.z = bits & (1 << 30) != 0;
        self.c = bits & (1 << 29) != 0;
        self.v = bits & (1 << 28) != 0;
    }

    /// `DAIF` in its register layout (bits 9:6).
    #[inline]
    pub const fn daif(self) -> u64 {
        ((self.d as u64) << 9) | ((self.a as u64) << 8) | ((self.i as u64) << 7) | ((self.f as u64) << 6)
    }

    #[inline]
    pub fn set_daif(&mut self, bits: u64) {
        self.d = bits & (1 << 9) != 0;
        self.a = bits & (1 << 8) != 0;
        self.i = bits & (1 << 7) != 0;
        self.f = bits & (1 << 6) != 0;
    }
}

/// Construction parameters for a processor.
#[derive(Debug, Clone, Copy)]
pub struct CpuConfig {
    /// Emulated instructions per second: the rate of the tick count, and so
    /// of every clock derived from it (Bochs `cpu: ips`).
    pub ips: ClockHz,
    /// The system counter's frequency, which is what `CNTPCT_EL0` counts at
    /// and what `CNTFRQ_EL0` reads at reset. QEMU's `virt` (machine types
    /// before 9.0) runs it at 62.5 MHz.
    pub counter_hz: ClockHz,
    /// The address execution starts at.
    pub reset_pc: u64,
}

impl CpuConfig {
    /// QEMU's `virt` system counter frequency (`GTIMER_SCALE` 16 ns).
    pub const QEMU_VIRT_COUNTER_HZ: ClockHz = match ClockHz::new(62_500_000) {
        Some(hz) => hz,
        None => ClockHz::BOCHS_DEFAULT,
    };
}

/// What the processor is doing between instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Activity {
    Running,
    /// Executed `WFI`, `WFE`, `WFIT`, `WFET` or a PSCI `CPU_SUSPEND`, and
    /// sleeps until what it waits for.
    Waiting(Wait),
}

/// The local exclusive monitor (`LDXR`/`STXR`), tracking one physical
/// granule. Single processor: there is no global monitor to model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExclusiveMonitor {
    pub(crate) armed: bool,
    pub(crate) addr: u64,
    pub(crate) bytes: u64,
}

/// System registers that are plain storage, plus the ones whose effects the
/// MMU, exception and timer code read. The access rules live in `sysreg.rs`.
#[derive(Debug, Clone)]
pub(crate) struct SysRegs {
    pub(crate) sctlr_el1: u64,
    pub(crate) actlr_el1: u64,
    pub(crate) cpacr_el1: u64,
    pub(crate) ttbr0_el1: u64,
    pub(crate) ttbr1_el1: u64,
    pub(crate) tcr_el1: u64,
    pub(crate) spsr_el1: u64,
    pub(crate) elr_el1: u64,
    pub(crate) esr_el1: u64,
    pub(crate) far_el1: u64,
    pub(crate) afsr0_el1: u64,
    pub(crate) afsr1_el1: u64,
    pub(crate) par_el1: u64,
    pub(crate) mair_el1: u64,
    pub(crate) amair_el1: u64,
    pub(crate) vbar_el1: u64,
    pub(crate) contextidr_el1: u64,
    pub(crate) tpidr_el0: u64,
    pub(crate) tpidrro_el0: u64,
    pub(crate) tpidr_el1: u64,
    /// `TPIDR2_EL0` (FEAT_SME).
    pub(crate) tpidr2_el0: u64,
    /// `SMCR_EL1` (FEAT_SME): the `LEN` field this core keeps.
    pub(crate) smcr_el1: u64,
    pub(crate) csselr_el1: u64,
    pub(crate) cntkctl_el1: u64,
    pub(crate) cntfrq_el0: u64,
    pub(crate) cntp_ctl_el0: u64,
    pub(crate) cntp_cval_el0: u64,
    pub(crate) cntv_ctl_el0: u64,
    pub(crate) cntv_cval_el0: u64,
    pub(crate) mdscr_el1: u64,
    pub(crate) mdccint_el1: u64,
    pub(crate) oslsr_el1: u64,
    pub(crate) osdlr_el1: u64,
    pub(crate) oseccr_el1: u64,
    pub(crate) dbgprcr_el1: u64,
    pub(crate) dbgclaim: u64,
    pub(crate) dbgbvr: [u64; 2],
    pub(crate) dbgbcr: [u64; 2],
    pub(crate) dbgwvr: [u64; 2],
    pub(crate) dbgwcr: [u64; 2],
    /// `DISR_EL1` (FEAT_RAS): with no source of SError interrupts, only an
    /// `MSR` writes it.
    pub(crate) disr_el1: u64,
}

impl SysRegs {
    fn reset(counter_hz: ClockHz) -> Self {
        Self {
            sctlr_el1: id::SCTLR_EL1_RESET,
            actlr_el1: 0,
            cpacr_el1: 0,
            ttbr0_el1: 0,
            ttbr1_el1: 0,
            tcr_el1: 0,
            spsr_el1: 0,
            elr_el1: 0,
            esr_el1: 0,
            far_el1: 0,
            afsr0_el1: 0,
            afsr1_el1: 0,
            par_el1: 0,
            mair_el1: 0,
            amair_el1: 0,
            vbar_el1: 0,
            contextidr_el1: 0,
            tpidr_el0: 0,
            tpidrro_el0: 0,
            tpidr_el1: 0,
            tpidr2_el0: 0,
            smcr_el1: 0,
            csselr_el1: 0,
            cntkctl_el1: 0,
            cntfrq_el0: counter_hz.get(),
            cntp_ctl_el0: 0,
            cntp_cval_el0: 0,
            cntv_ctl_el0: 0,
            cntv_cval_el0: 0,
            mdscr_el1: 0,
            mdccint_el1: 0,
            oslsr_el1: id::OSLSR_EL1_RESET,
            osdlr_el1: 0,
            oseccr_el1: 0,
            dbgprcr_el1: 0,
            dbgclaim: 0,
            dbgbvr: [0; 2],
            dbgbcr: [0; 2],
            dbgwvr: [0; 2],
            dbgwcr: [0; 2],
            disr_el1: 0,
        }
    }
}

/// How many recently executed instruction addresses a processor remembers for
/// post-mortem diagnostics.
pub const PC_HISTORY: usize = 64;

/// One AArch64 processing element.
pub struct Cpu {
    pub(crate) x: [u64; 31],
    pub(crate) sp_el0: u64,
    pub(crate) sp_el1: u64,
    pub(crate) pc: u64,
    pub(crate) pstate: Pstate,
    /// SIMD&FP registers V0..V31, little-endian lanes in a `u128`: the low
    /// 128 bits of Z0..Z31. Every write goes through `write_v`.
    pub(crate) v: [u128; 32],
    pub(crate) fpcr: u64,
    pub(crate) fpsr: u64,
    /// `PSTATE.SM` and `PSTATE.ZA` (FEAT_SME).
    pub(crate) svcr: sme::Svcr,
    /// Z above V, the predicates and ZA (FEAT_SME).
    pub(crate) scalable: sme::ScalableRegs,
    /// The pointer authentication keys (FEAT_PAuth), zero at reset as QEMU
    /// sets them, where the architecture leaves them UNKNOWN.
    pub(crate) pac_keys: pauth::PacKeys,
    /// What this processor implements: the decoder, the ID registers and
    /// every feature's own checks read it.
    pub(crate) imp: features::Implementation,
    pub(crate) sys: SysRegs,
    pub(crate) monitor: ExclusiveMonitor,
    pub(crate) event: wait::EventRegister,
    pub(crate) tlb: Tlb,
    pub(crate) decode_cache: decode::DecodeCache,
    pub(crate) activity: Activity,
    /// Ticks since the machine was built: one per retired (or faulting)
    /// instruction, plus the span skipped while waiting.
    pub(crate) icount: u64,
    /// Instructions retired (or that took an exception in their place) since
    /// the machine was built — `icount` without the waiting.
    pub(crate) retired: u64,
    pub(crate) ips: ClockHz,
    pub(crate) counter_hz: ClockHz,
    /// Set when a register write may have changed a timer's output line or
    /// next deadline; the machine re-evaluates and clears it.
    pub(crate) timer_dirty: bool,
    /// The interrupt controller's IRQ and FIQ outputs, as the machine last
    /// sampled them. `ISR_EL1` reads them and `WFI` wakes on them.
    pub(crate) irq_line: bool,
    pub(crate) fiq_line: bool,
    pub(crate) debug_stops: DebugStops,
    pub(crate) pc_history: RingBuffer<u64, PC_HISTORY>,
}

impl Cpu {
    /// A processor in its reset state, shaped like the Cortex-A57
    /// ([`id`]).
    pub fn new(config: CpuConfig) -> Self {
        Self::implementing(config, features::Implementation::CORTEX_A57)
    }

    /// A processor implementing `imp`, in its reset state.
    pub(crate) fn implementing(config: CpuConfig, imp: features::Implementation) -> Self {
        Self {
            x: [0; 31],
            sp_el0: 0,
            sp_el1: 0,
            pc: config.reset_pc,
            pstate: Pstate::RESET,
            v: [0; 32],
            fpcr: 0,
            fpsr: 0,
            svcr: sme::Svcr::RESET,
            scalable: sme::ScalableRegs::ZERO,
            pac_keys: pauth::PacKeys::ZERO,
            imp,
            sys: SysRegs::reset(config.counter_hz),
            monitor: ExclusiveMonitor {
                armed: false,
                addr: 0,
                bytes: 0,
            },
            event: wait::EventRegister::at(false, timer::Count::new(0)),
            tlb: Tlb::new(),
            decode_cache: decode::DecodeCache::new(imp.features),
            activity: Activity::Running,
            icount: 0,
            retired: 0,
            ips: config.ips,
            counter_hz: config.counter_hz,
            timer_dirty: true,
            irq_line: false,
            fiq_line: false,
            debug_stops: DebugStops {
                on_undefined: false,
                on_bus_fault: false,
            },
            pc_history: RingBuffer::new(),
        }
    }

    /// Drive the processor's interrupt inputs from the interrupt controller.
    #[inline]
    pub fn set_interrupt_lines(&mut self, irq: bool, fiq: bool) {
        self.irq_line = irq;
        self.fiq_line = fiq;
    }

    /// Whether an interrupt input is asserted and unmasked, so that it would
    /// be taken at the next instruction boundary.
    #[inline]
    pub fn interrupt_deliverable(&self) -> bool {
        (self.irq_line && !self.pstate.i) || (self.fiq_line && !self.pstate.f)
    }

    /// Take the highest-priority deliverable interrupt, if any (FIQ before
    /// IRQ); [`Cpu::interrupt_deliverable`] says beforehand whether one will
    /// be taken.
    pub fn deliver_interrupt(&mut self) -> Option<crate::instrumentation::ExceptionTaken> {
        if self.fiq_line && !self.pstate.f {
            Some(self.take_fiq())
        } else if self.irq_line && !self.pstate.i {
            Some(self.take_irq())
        } else {
            None
        }
    }

    /// Advance time while waiting, to `until` (never backwards).
    pub fn wait_until(&mut self, until: VmInstant) {
        if until.ticks() > self.icount {
            self.icount = until.ticks();
        }
    }

    /// The address of the next instruction to execute.
    #[inline]
    pub fn pc(&self) -> u64 {
        self.pc
    }

    /// General-purpose register `n` (0..=30); 31 reads as zero (XZR).
    #[inline]
    pub fn xreg(&self, n: u32) -> u64 {
        self.x_or_zr(n)
    }

    /// The stack pointer currently selected by `PSTATE.SP` and the EL.
    #[inline]
    pub fn sp(&self) -> u64 {
        self.sp_current()
    }

    #[inline]
    pub fn pstate(&self) -> Pstate {
        self.pstate
    }

    #[inline]
    pub fn el(&self) -> El {
        self.pstate.el
    }

    #[inline]
    pub fn activity(&self) -> Activity {
        self.activity
    }

    /// SIMD&FP register `n` as 128 bits.
    #[inline]
    pub fn vreg(&self, n: u32) -> u128 {
        self.v[(n & 31) as usize]
    }

    /// Emulated time since the processor was created; a reset keeps
    /// counting.
    #[inline]
    pub fn now(&self) -> VmInstant {
        VmInstant::from_ticks(self.icount)
    }

    /// The reading devices see for an access made now.
    #[inline]
    pub fn clock(&self) -> VmClock {
        VmClock::new(self.now(), self.ips)
    }

    /// The rate of the tick count.
    #[inline]
    pub fn ips(&self) -> ClockHz {
        self.ips
    }

    /// Recently executed instruction addresses, oldest first.
    pub fn recent_pcs(&self) -> rusty_box_core::ring_buffer::Iter<'_, u64, PC_HISTORY> {
        self.pc_history.iter()
    }

    // ----- register file -----

    #[inline]
    pub(crate) fn x_or_zr(&self, n: u32) -> u64 {
        if n == 31 {
            0
        } else {
            self.x[n as usize]
        }
    }

    #[inline]
    pub(crate) fn set_x_or_zr(&mut self, n: u32, value: u64) {
        if n != 31 {
            self.x[n as usize] = value;
        }
    }

    /// Register `n` where 31 names the stack pointer.
    #[inline]
    pub(crate) fn x_or_sp(&self, n: u32) -> u64 {
        if n == 31 {
            self.sp_current()
        } else {
            self.x[n as usize]
        }
    }

    #[inline]
    pub(crate) fn set_x_or_sp(&mut self, n: u32, value: u64) {
        if n == 31 {
            self.set_sp_current(value);
        } else {
            self.x[n as usize] = value;
        }
    }

    /// Write a result of the given width: a 32-bit result zero-extends.
    #[inline]
    pub(crate) fn set_x_or_zr_sized(&mut self, n: u32, value: u64, sf: bool) {
        self.set_x_or_zr(n, if sf { value } else { value & 0xFFFF_FFFF });
    }

    #[inline]
    pub(crate) fn set_x_or_sp_sized(&mut self, n: u32, value: u64, sf: bool) {
        self.set_x_or_sp(n, if sf { value } else { value & 0xFFFF_FFFF });
    }

    #[inline]
    pub(crate) fn sp_current(&self) -> u64 {
        if self.pstate.sp_sel && self.pstate.el == El::El1 {
            self.sp_el1
        } else {
            self.sp_el0
        }
    }

    #[inline]
    pub(crate) fn set_sp_current(&mut self, value: u64) {
        if self.pstate.sp_sel && self.pstate.el == El::El1 {
            self.sp_el1 = value;
        } else {
            self.sp_el0 = value;
        }
    }
}
