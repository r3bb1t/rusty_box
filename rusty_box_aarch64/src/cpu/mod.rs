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
mod el2;
#[cfg(test)]
#[path = "../../tests/unit/el2.rs"]
mod el2_tests;
#[cfg(test)]
#[path = "../../tests/unit/el2_vectors.rs"]
mod el2_vectors;
mod exception;
mod exec;
mod features;
mod fp;
mod fp_estimate;
#[cfg(test)]
#[path = "../../tests/unit/fp_extensions.rs"]
mod fp_extensions;
mod history;
mod isa;
mod ldst;
#[cfg(test)]
#[path = "../../tests/unit/load_store.rs"]
mod load_store;
mod memattr;
#[cfg(test)]
#[path = "../../tests/unit/memory_system.rs"]
mod memory_system;
mod mmu;
mod op;
mod pauth;
#[cfg(test)]
#[path = "../../tests/unit/pointer_auth.rs"]
mod pointer_auth;
mod poll;
#[cfg(test)]
#[path = "../../tests/unit/pstate_features.rs"]
mod pstate_features;
mod regime;
mod regs;
mod sctlr;
mod simd;
mod simd_arith;
mod simd_bf16;
mod simd_fp;
mod sme;
mod stage2;
mod state;
#[cfg(test)]
#[path = "../../tests/unit/step.rs"]
mod step;
mod sve;
mod sysreg;
#[cfg(test)]
#[path = "../../tests/unit/sysreg_spec.rs"]
mod sysreg_spec;
mod system;
mod timer;
mod trace;
mod wait;
mod za;
mod za_array;
mod zt0;

use rusty_box_core::{ClockHz, VmClock, VmInstant};

use el2::El2Storage;

pub use el2::{Levels, NoEl2, WithEl2};
pub use exception::ExceptionClass;
pub use exec::{DebugStops, Step, StopCause, UndefinedInsn};
pub use features::{
    CpuModel, El2Unavailable, UnknownCpuModel, ARMV8P6_COUNTER_HZ, QEMU_BACKCOMPAT_COUNTER_HZ,
};
pub use history::RecentPcs;
pub use mmu::{Ipa, NoTranslation, Tlb, TranslationFault, TranslationStage};
pub use regs::{CpuState, CpuStateEl2, PstateRefusal, Reg, SysReg, SysRegRefusal, VReg};
pub(crate) use sysreg::WriteEffect;
pub(crate) use system::{PsciConduit, PsciNode};
pub use timer::{TimerLines, VirtualCount};
pub use wait::Wait;

use exec::Nzcv;

/// An exception level this architecture's AArch64 state names, short of
/// EL3.
///
/// Closed (R5): EL3 does not exist on this core, and an `ERET` that names it
/// is an illegal exception return, not a fourth variant. EL2 exists only on
/// a processor implementing FEAT_EL2; on one without, nothing puts the
/// processor at EL2: an `ERET` that names it is illegal, a snapshot that
/// does is refused, and so is a host's
/// [`VirtMachine::set_pstate`](crate::machine::VirtMachine::set_pstate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum El {
    El0,
    El1,
    El2,
}

impl El {
    #[inline]
    pub const fn index(self) -> u64 {
        match self {
            Self::El0 => 0,
            Self::El1 => 1,
            Self::El2 => 2,
        }
    }

    /// The level `SPSR.M[3:2]` names, if it is one of these.
    #[inline]
    pub const fn from_index(index: u64) -> Option<Self> {
        match index {
            0 => Some(Self::El0),
            1 => Some(Self::El1),
            2 => Some(Self::El2),
            _ => None,
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
    /// `PSTATE.SP`: true selects `SP_ELx` for the current EL (`SP_EL1` at
    /// EL1, `SP_EL2` at EL2), false `SP_EL0`.
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
    /// The state out of reset of a processor whose highest exception level
    /// is EL1 (Arm `AArch64_TakeReset`): EL1 with `SP_EL1`, every interrupt
    /// masked, DIT clear. PAN and UAO, which reset leaves UNKNOWN, and SSBS,
    /// which it sets to an IMPLEMENTATION DEFINED value, are clear: this
    /// processor's choice for each.
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

    /// The state out of reset of a processor implementing the levels `L`:
    /// Arm `AArch64_TakeReset` starts it at the highest implemented
    /// exception level, which is EL2 with FEAT_EL2 (no EL3 here) and EL1
    /// without, with that level's stack pointer (`PSTATE.SP` = 1).
    pub(crate) const fn reset<L: Levels>() -> Self {
        let el = if L::EL2 { El::El2 } else { El::El1 };
        Self { el, ..Self::RESET }
    }

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

    /// The state an `SPSR_ELx` value describes, if it is one an AArch64
    /// processor with no EL3 can be in: AArch64 (`M[4]` clear), EL0, EL1 or
    /// EL2, and at EL0 the only stack pointer EL0 has. Whether a processor
    /// has EL2 is the processor's question: on one without, an `ERET` to
    /// EL2 is an illegal return and
    /// [`VirtMachine::set_pstate`](crate::machine::VirtMachine::set_pstate)
    /// refuses EL2.
    pub const fn from_spsr(spsr: u64) -> Option<Self> {
        let m = spsr & 0x1F;
        let Some(el) = El::from_index((m >> 2) & 3) else {
            return None;
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
        ((self.n as u64) << 31)
            | ((self.z as u64) << 30)
            | ((self.c as u64) << 29)
            | ((self.v as u64) << 28)
    }

    /// Set N, Z, C and V to `flags`.
    #[inline]
    pub(crate) fn set_flags(&mut self, flags: Nzcv) {
        self.n = flags.n;
        self.z = flags.z;
        self.c = flags.c;
        self.v = flags.v;
    }

    /// Set N, Z, C and V from `NZCV` in its register layout (bits 31:28),
    /// as an `MSR NZCV` does; every other bit of `bits` is ignored.
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
        ((self.d as u64) << 9)
            | ((self.a as u64) << 8)
            | ((self.i as u64) << 7)
            | ((self.f as u64) << 6)
    }

    /// Set D, A, I and F from `DAIF` in its register layout (bits 9:6), as
    /// an `MSR DAIF` does; every other bit of `bits` is ignored.
    #[inline]
    pub fn set_daif(&mut self, bits: u64) {
        self.d = bits & (1 << 9) != 0;
        self.a = bits & (1 << 8) != 0;
        self.i = bits & (1 << 7) != 0;
        self.f = bits & (1 << 6) != 0;
    }
}

/// Construction parameters for a processor, beyond its model, which sets
/// the system counter's frequency ([`CpuModel::counter_hz`]).
#[derive(Debug, Clone, Copy)]
pub struct CpuConfig {
    /// Emulated instructions per second: the rate of the tick count, and so
    /// of every clock derived from it (Bochs `cpu: ips`).
    pub ips: ClockHz,
    /// The address execution starts at.
    pub reset_pc: u64,
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
    /// The physical address of the monitored granule.
    pub(crate) addr: u64,
    /// The virtual address the load-exclusive used, which a store-exclusive
    /// compares first (Arm ARM `AArch64.IsExclusiveVA`).
    pub(crate) va: u64,
    pub(crate) bytes: u64,
}

impl ExclusiveMonitor {
    /// Open Access: no granule monitored.
    pub(crate) const CLEAR: Self = Self {
        armed: false,
        addr: 0,
        va: 0,
        bytes: 0,
    };
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
    /// `SMCR_EL1` (FEAT_SME): the `LEN` field, and `EZT0` with FEAT_SME2.
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
    /// The Debug Communications Channel's data registers, DTRRX and DTRTX,
    /// 32 bits each; their flags, RXfull and TXfull, are in `mdscr_el1`.
    pub(crate) dtrrx: u32,
    pub(crate) dtrtx: u32,
    /// `DISR_EL1` (FEAT_RAS): with no source of SError interrupts, only an
    /// `MSR` writes it.
    pub(crate) disr_el1: u64,
    /// `RVBAR_EL1`, or with FEAT_EL2 `RVBAR_EL2`: the reset vector base
    /// address of the highest implemented level, the address execution
    /// started from at the last reset (Arm `AArch64_TakeReset` branches to
    /// it).
    pub(crate) rvbar: u64,
    /// `RMR_EL1`'s, or with FEAT_EL2 `RMR_EL2`'s, stored field: `RR` (bit
    /// 1), a Warm reset request, which this board's reset controller does
    /// not act on (Arm: "the bit is strictly a request"). `AA64` (bit 0)
    /// reads as one, the only state this processor resets into.
    pub(crate) rmr: u64,
}

impl SysRegs {
    fn reset(counter_hz: ClockHz, reset_pc: u64) -> Self {
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
            dtrrx: 0,
            dtrtx: 0,
            disr_el1: 0,
            rvbar: reset_pc,
            rmr: 0,
        }
    }
}

/// How many recently executed instruction addresses a processor remembers for
/// post-mortem diagnostics.
pub const PC_HISTORY: usize = 64;

/// One AArch64 processing element, implementing the exception levels `L`
/// ([`NoEl2`] or [`WithEl2`]).
pub struct Cpu<L: Levels> {
    pub(crate) x: [u64; 31],
    pub(crate) sp_el0: u64,
    pub(crate) sp_el1: u64,
    /// `SP_EL2` and the EL2 registers, which only a processor with EL2
    /// holds ([`Cpu::el2`], [`Cpu::el2_mut`], [`Cpu::sp_el2`]).
    pub(crate) el2_state: L::State,
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
    /// What this processor implements, as a processor of its level set:
    /// the decoder, the ID registers and every feature's own checks read
    /// it.
    pub(crate) imp: features::LeveledImplementation<L>,
    pub(crate) sys: SysRegs,
    pub(crate) monitor: ExclusiveMonitor,
    pub(crate) event: wait::EventRegister,
    pub(crate) tlb: Tlb<L>,
    pub(crate) traces: trace::TraceCache,
    /// Set by anything that may change how the next instruction is fetched
    /// or found — a TLB fill or flush, a write over cached code, a cache
    /// invalidation — so that the running trace stops after the current
    /// instruction. Cleared when a trace starts.
    pub(crate) trace_break: bool,
    /// Arm `InGuardedPage` (FEAT_BTI) of the instruction executing: whether
    /// the page its fetch translated is guarded, as that translation found
    /// it (`Cpu::fetch_translate`). Every fetch translates before its
    /// instruction runs, but those of a trace's later instructions, which
    /// are on its first one's page; so it is not saved, and the first fetch
    /// after a restore sets it.
    pub(crate) in_guarded_page: bool,
    pub(crate) activity: Activity,
    /// Ticks since the machine was built: one per retired (or faulting)
    /// instruction, plus the span skipped while waiting.
    pub(crate) icount: u64,
    /// Instructions retired (or that took an exception in their place) since
    /// the machine was built — `icount` without the waiting.
    pub(crate) retired: u64,
    pub(crate) ips: ClockHz,
    /// Set when a register write may have changed a timer's output line or
    /// next deadline; the machine re-evaluates and clears it.
    pub(crate) timer_dirty: bool,
    /// The interrupt controller's IRQ and FIQ outputs, as the machine last
    /// sampled them. `ISR_EL1` reads them and `WFI` wakes on them.
    pub(crate) irq_line: bool,
    pub(crate) fiq_line: bool,
    pub(crate) debug_stops: DebugStops,
    /// Where the last [`PC_HISTORY`] instructions ran. Every write of `pc`
    /// other than the step to the next sequential instruction tells it.
    pub(crate) history: history::PcHistory,
}

impl<L: Levels> Cpu<L> {
    /// A processor of `model` in its reset state, with EL2 where its level
    /// set has it; refused for a model that cannot have EL2 when the level
    /// set does ([`CpuModel::check_levels`]).
    pub fn new(model: CpuModel, config: CpuConfig) -> Result<Self, El2Unavailable> {
        model.check_levels::<L>()?;
        Ok(Self::implementing(config, model.implementation()))
    }

    /// A processor implementing `imp` and the levels `L`, in its reset
    /// state. `L` alone decides FEAT_EL2: whatever `imp` says of it is
    /// replaced, and without EL2 the features only EL2 brings, such as
    /// FEAT_ECV_POFF, are dropped
    /// ([`features::Implementation::with_levels`]).
    pub(crate) fn implementing(config: CpuConfig, imp: features::Implementation) -> Self {
        Self::leveled(config, imp.with_levels::<L>())
    }

    /// A processor implementing `imp`, leveled for `L` already, in its
    /// reset state.
    pub(crate) fn leveled(config: CpuConfig, imp: features::LeveledImplementation<L>) -> Self {
        Self {
            x: [0; 31],
            sp_el0: 0,
            sp_el1: 0,
            el2_state: L::State::keep(el2::El2State::reset(imp)),
            pc: config.reset_pc,
            pstate: Pstate::reset::<L>(),
            v: [0; 32],
            fpcr: 0,
            fpsr: 0,
            svcr: sme::Svcr::RESET,
            scalable: sme::ScalableRegs::ZERO,
            pac_keys: pauth::PacKeys::ZERO,
            imp,
            sys: SysRegs::reset(imp.counter_hz, config.reset_pc),
            monitor: ExclusiveMonitor::CLEAR,
            event: wait::EventRegister::at(false, timer::Count::new(0)),
            tlb: Tlb::new(),
            traces: trace::TraceCache::new(),
            trace_break: false,
            in_guarded_page: false,
            activity: Activity::Running,
            icount: 0,
            retired: 0,
            ips: config.ips,
            timer_dirty: true,
            irq_line: false,
            fiq_line: false,
            debug_stops: DebugStops {
                on_undefined: false,
                on_bus_fault: false,
            },
            history: history::PcHistory::new(config.reset_pc, 0),
        }
    }

    /// Drive the processor's interrupt inputs from the interrupt controller.
    #[inline]
    pub fn set_interrupt_lines(&mut self, irq: bool, fiq: bool) {
        self.irq_line = irq;
        self.fiq_line = fiq;
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

    /// Recently executed instruction addresses, oldest first: the last
    /// [`PC_HISTORY`] instructions that retired or took an exception in
    /// their place.
    pub fn recent_pcs(&self) -> RecentPcs {
        self.history.recent(self.retired)
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

    /// The stack pointer `PSTATE.SP` and `PSTATE.EL` select (Arm `SP[]`):
    /// `SP_EL0` with `PSTATE.SP` clear, and at EL0, which holds it clear. A
    /// processor without EL2 is never at EL2, so for it the EL1 arm covers
    /// both and the selection is the one test of a processor with EL1 alone.
    #[inline]
    pub(crate) fn sp_current(&self) -> u64 {
        match (self.pstate.sp_sel, self.pstate.el) {
            (true, El::El2) if L::EL2 => self.sp_el2(),
            (true, El::El1 | El::El2) => self.sp_el1,
            (false, _) | (true, El::El0) => self.sp_el0,
        }
    }

    /// The write of [`Cpu::sp_current`].
    #[inline]
    pub(crate) fn set_sp_current(&mut self, value: u64) {
        match (self.pstate.sp_sel, self.pstate.el) {
            (true, El::El2) if L::EL2 => self.set_sp_el2(value),
            (true, El::El1 | El::El2) => self.sp_el1 = value,
            (false, _) | (true, El::El0) => self.sp_el0 = value,
        }
    }
}
