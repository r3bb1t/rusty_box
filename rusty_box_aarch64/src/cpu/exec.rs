//! Fetch, decode and dispatch: one call of [`Cpu::step`] executes one
//! instruction or takes one exception.
//!
//! Decoding is split in two. `disarm64` classifies the word into its
//! canonical A64 encoding (never a disassembly alias) and names the feature
//! that encoding belongs to (`decode.rs`); the handler for that encoding
//! *class* then reads every operand field from the word itself, following
//! the field layout and decode pseudocode of the Arm ARM. A class handler
//! therefore covers its whole encoding group, and a field the classifier does
//! not expose cannot be misread through it.

use disarm64_defn::InsnClass;

use rusty_box_core::VmClock;

use super::decode::Decoded;
use super::exception::{Exception, ExceptionClass};
use super::{Activity, Cpu};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::{Direction, Instrumentation, MemoryAccess};

/// The bus one step executes against: the machine's own, with the observer
/// told of every data access that completes. Fetches and table walks pass
/// through unreported — the observer hears of the instruction itself, and a
/// walk is not an access the program made.
pub(crate) struct Observed<'a, B, I> {
    bus: &'a mut B,
    pub(crate) observer: &'a mut I,
}

impl<B: PhysBus, I: Instrumentation> PhysBus for Observed<'_, B, I> {
    #[inline(always)]
    fn read(&mut self, addr: PhysAddr, width: AccessWidth, clock: VmClock) -> Result<u64, BusFault> {
        let value = self.bus.read(addr, width, clock)?;
        self.observer.memory_access(MemoryAccess {
            address: addr,
            width,
            direction: Direction::Read,
            value,
        });
        Ok(value)
    }

    #[inline(always)]
    fn write(&mut self, addr: PhysAddr, width: AccessWidth, value: u64, clock: VmClock) -> Result<(), BusFault> {
        self.bus.write(addr, width, value, clock)?;
        self.observer.memory_access(MemoryAccess {
            address: addr,
            width,
            direction: Direction::Write,
            value,
        });
        Ok(())
    }

    #[inline(always)]
    fn fetch(&mut self, addr: PhysAddr, clock: VmClock) -> Result<u32, BusFault> {
        self.bus.fetch(addr, clock)
    }

    #[inline(always)]
    fn read_descriptor(&mut self, addr: PhysAddr, clock: VmClock) -> Result<u64, BusFault> {
        self.bus.read_descriptor(addr, clock)
    }
}

/// What one [`Cpu::step`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// An instruction retired, or an exception was taken in its place.
    Retired,
    /// `WFI` (or PSCI `CPU_SUSPEND`) retired and the processor now waits for
    /// an interrupt; [`Cpu::activity`] says so until one arrives.
    Waiting,
    /// The machine must stop before this instruction; the processor state is
    /// exactly as it was before it.
    Stop(StopCause),
}

/// The address and word of an instruction that stopped the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UndefinedInsn {
    pub pc: u64,
    pub insn: u32,
}

/// Why a step stopped the machine instead of executing.
///
/// Closed (R5): the machine maps each variant onto its own `StopReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopCause {
    /// A defined ARMv8.0 encoding inside the advertised feature set that this
    /// interpreter does not implement. Raising UNDEFINED for it would be a
    /// silent divergence from the architecture, so the machine stops instead.
    Unimplemented(UndefinedInsn),
    /// The instruction is UNDEFINED and the machine was configured to stop on
    /// UNDEFINED instructions rather than deliver the exception.
    Undefined(UndefinedInsn),
    /// A data or instruction access reached no device and the machine was
    /// configured to stop rather than raise a synchronous external abort.
    BusFault { pc: u64, addr: u64 },
    /// PSCI `SYSTEM_OFF`.
    PowerOff,
    /// PSCI `SYSTEM_RESET`.
    Reset,
    /// PSCI `CPU_OFF` on the only processor: nothing can ever run again.
    CpuOff,
}

/// How control leaves an instruction that completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    /// Continue at the next sequential instruction.
    Next,
    /// Continue at this address (a taken branch, an exception return).
    Jump(u64),
    /// The instruction retired and the processor waits for an interrupt.
    Wait,
}

/// Why an instruction did not complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    /// Take this synchronous exception.
    Exception(Exception),
    /// Stop the machine before this instruction.
    Stop(StopCause),
}

impl From<Exception> for Fault {
    #[inline]
    fn from(e: Exception) -> Self {
        Self::Exception(e)
    }
}

pub(crate) type ExecResult = Result<Flow, Fault>;

/// Which faults stop the machine instead of being delivered to the guest.
/// Both default to delivering, which is what hardware does; stopping is a
/// bring-up aid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DebugStops {
    pub on_undefined: bool,
    pub on_bus_fault: bool,
}

// ----- field extraction -----

/// Bits `hi..=lo` of `insn`, right-aligned.
#[inline(always)]
pub(crate) const fn bits(insn: u32, hi: u32, lo: u32) -> u32 {
    (insn >> lo) & ((1u32 << (hi - lo + 1)) - 1)
}

#[inline(always)]
pub(crate) const fn bit(insn: u32, n: u32) -> bool {
    (insn >> n) & 1 != 0
}

/// Sign-extend the low `width` bits of `value`.
#[inline(always)]
pub(crate) const fn sext(value: u64, width: u32) -> u64 {
    let shift = 64 - width;
    (((value << shift) as i64) >> shift) as u64
}

impl Cpu {
    /// Configure which faults stop the machine instead of being delivered.
    pub fn set_debug_stops(&mut self, stops: DebugStops) {
        self.debug_stops = stops;
    }

    /// Execute one instruction (or take the exception it raises) against
    /// `bus`, advancing time by one tick unless the step stops the machine,
    /// and tell `observer` what happened.
    pub fn step<B: PhysBus, I: Instrumentation>(&mut self, bus: &mut B, observer: &mut I) -> Step {
        let pc = self.pc;
        let mut observed = Observed { bus, observer };
        let mut fetched = None;
        let result = self.execute(&mut observed, pc, &mut fetched);
        let observer = observed.observer;
        match result {
            Ok(flow) => {
                self.icount += 1;
                self.retired += 1;
                self.pc_history.push_back(pc);
                if let Some(insn) = fetched {
                    observer.after_execution(pc, insn);
                }
                match flow {
                    Flow::Next => {
                        self.pc = pc.wrapping_add(4);
                        Step::Retired
                    }
                    Flow::Jump(target) => {
                        // Every branch, ERET included, writes the PC through
                        // `AArch64.BranchAddr` (Arm ARM `BranchTo`).
                        let target = self.branch_addr(target);
                        self.pc = target;
                        observer.branch(pc, target);
                        Step::Retired
                    }
                    Flow::Wait => {
                        self.pc = pc.wrapping_add(4);
                        self.activity = Activity::WaitingForInterrupt;
                        observer.wait_for_interrupt(pc);
                        Step::Waiting
                    }
                }
            }
            Err(Fault::Exception(e)) => {
                self.icount += 1;
                self.retired += 1;
                self.pc_history.push_back(pc);
                let taken = self.take_sync_exception(e, pc);
                observer.exception(taken);
                Step::Retired
            }
            Err(Fault::Stop(cause)) => Step::Stop(cause),
        }
    }

    fn execute<B: PhysBus, I: Instrumentation>(
        &mut self,
        bus: &mut Observed<'_, B, I>,
        pc: u64,
        fetched_word: &mut Option<u32>,
    ) -> ExecResult {
        // An illegal exception return left PSTATE.IL set: the next
        // instruction raises Illegal Execution State instead of executing.
        if self.pstate.il {
            return Err(Exception::new(ExceptionClass::IllegalState, 0).into());
        }
        if pc & 3 != 0 {
            return Err(Exception::new(ExceptionClass::PcAlignment, 0).with_far(pc).into());
        }
        let fetched = self.fetch(bus, pc)?;
        let insn = fetched.word;
        *fetched_word = Some(insn);
        bus.observer.before_execution(pc, insn);
        let class = match self.decode_cache.decode(fetched.pa, insn) {
            Decoded::Hint => return self.exec_hint(insn, pc),
            Decoded::Undefined => return self.undefined(pc, insn),
            Decoded::Class(class) => class,
        };
        match class {
            InsnClass::ADDSUB_IMM => self.exec_addsub_imm(insn),
            InsnClass::ADDSUB_SHIFT => self.exec_addsub_shift(pc, insn),
            InsnClass::ADDSUB_EXT => self.exec_addsub_ext(pc, insn),
            InsnClass::ADDSUB_CARRY => self.exec_addsub_carry(pc, insn),
            InsnClass::LOG_IMM => self.exec_logic_imm(pc, insn),
            InsnClass::LOG_SHIFT => self.exec_logic_shift(pc, insn),
            InsnClass::MOVEWIDE => self.exec_movewide(pc, insn),
            InsnClass::PCRELADDR => self.exec_pcrel(pc, insn),
            InsnClass::BITFIELD => self.exec_bitfield(pc, insn),
            InsnClass::EXTRACT => self.exec_extract(pc, insn),
            InsnClass::CONDBRANCH => self.exec_condbranch(pc, insn),
            InsnClass::COMPBRANCH => self.exec_compbranch(pc, insn),
            InsnClass::TESTBRANCH => self.exec_testbranch(pc, insn),
            InsnClass::BRANCH_IMM => self.exec_branch_imm(pc, insn),
            InsnClass::BRANCH_REG => self.exec_branch_reg(pc, insn),
            InsnClass::CONDCMP_IMM | InsnClass::CONDCMP_REG => self.exec_condcmp(pc, insn),
            InsnClass::CONDSEL => self.exec_condsel(pc, insn),
            InsnClass::DP_1SRC => self.exec_dp1(pc, insn),
            InsnClass::DP_2SRC => self.exec_dp2(pc, insn),
            InsnClass::DP_3SRC => self.exec_dp3(pc, insn),
            InsnClass::EXCEPTION => self.exec_exception_gen(pc, insn),
            InsnClass::IC_SYSTEM => self.exec_system(bus, pc, insn),
            InsnClass::LDST_POS
            | InsnClass::LDST_IMM9
            | InsnClass::LDST_UNSCALED
            | InsnClass::LDST_REGOFF
            | InsnClass::LDST_UNPRIV => self.exec_ldst_single(bus, pc, insn, class),
            InsnClass::LOADLIT => self.exec_load_literal(bus, pc, insn),
            InsnClass::LDSTPAIR_OFF | InsnClass::LDSTPAIR_INDEXED | InsnClass::LDSTNAPAIR_OFFS => {
                self.exec_ldst_pair(bus, pc, insn)
            }
            InsnClass::LDSTEXCL => self.exec_ldst_exclusive(bus, pc, insn),
            InsnClass::ASISDLSE | InsnClass::ASISDLSEP => self.exec_ldst_multiple(bus, pc, insn),
            InsnClass::ASISDLSO | InsnClass::ASISDLSOP => self.exec_ldst_single_structure(bus, pc, insn),
            InsnClass::ASIMDIMM => self.exec_simd_imm(pc, insn),
            InsnClass::ASIMDINS => self.exec_simd_ins(pc, insn),
            InsnClass::ASIMDSAME => self.exec_simd_same(pc, insn, false),
            InsnClass::ASISDSAME => self.exec_simd_same(pc, insn, true),
            InsnClass::ASIMDMISC => self.exec_simd_misc(pc, insn, false),
            InsnClass::ASISDMISC => self.exec_simd_misc(pc, insn, true),
            InsnClass::ASIMDSHF => self.exec_simd_shift_imm(pc, insn, false),
            InsnClass::ASISDSHF => self.exec_simd_shift_imm(pc, insn, true),
            InsnClass::ASIMDDIFF => self.exec_simd_diff(pc, insn, false),
            InsnClass::ASISDDIFF => self.exec_simd_diff(pc, insn, true),
            InsnClass::ASIMDELEM => self.exec_simd_elem(pc, insn, false),
            InsnClass::ASISDELEM => self.exec_simd_elem(pc, insn, true),
            InsnClass::ASIMDALL => self.exec_simd_across(pc, insn),
            InsnClass::ASISDPAIR => self.exec_simd_scalar_pair(pc, insn),
            InsnClass::ASISDONE => self.exec_simd_scalar_dup(pc, insn),
            InsnClass::ASIMDEXT => self.exec_simd_ext(pc, insn),
            InsnClass::ASIMDPERM => self.exec_simd_perm(pc, insn),
            InsnClass::ASIMDTBL => self.exec_simd_tbl(pc, insn),
            InsnClass::FLOATIMM => self.exec_fmov_imm(pc, insn),
            InsnClass::FLOATDP1 => self.exec_fp_dp1(pc, insn),
            InsnClass::FLOATDP2 => self.exec_fp_dp2(pc, insn),
            InsnClass::FLOATDP3 => self.exec_fp_dp3(pc, insn),
            InsnClass::FLOATCMP => self.exec_fp_compare(pc, insn),
            InsnClass::FLOATCCMP => self.exec_fp_cond_compare(pc, insn),
            InsnClass::FLOATSEL => self.exec_fcsel(pc, insn),
            InsnClass::FLOAT2INT => self.exec_fp_int(pc, insn),
            InsnClass::FLOAT2FIX => self.exec_fp_fixed(pc, insn),
            _ => self.unimplemented(pc, insn),
        }
    }

    /// The instruction is UNDEFINED.
    #[inline]
    pub(crate) fn undefined(&self, pc: u64, insn: u32) -> ExecResult {
        if self.debug_stops.on_undefined {
            Err(Fault::Stop(StopCause::Undefined(UndefinedInsn { pc, insn })))
        } else {
            Err(Exception::new(ExceptionClass::Unknown, 0).into())
        }
    }

    /// A defined encoding this interpreter does not implement.
    #[inline]
    pub(crate) fn unimplemented(&self, pc: u64, insn: u32) -> ExecResult {
        Err(Fault::Stop(StopCause::Unimplemented(UndefinedInsn { pc, insn })))
    }

    /// The condition `cond` (Arm ARM `ConditionHolds`) against `PSTATE`.
    #[inline]
    pub(crate) fn condition_holds(&self, cond: u32) -> bool {
        let p = &self.pstate;
        let result = match cond >> 1 {
            0 => p.z,
            1 => p.c,
            2 => p.n,
            3 => p.v,
            4 => p.c && !p.z,
            5 => p.n == p.v,
            6 => p.n == p.v && !p.z,
            _ => true,
        };
        if cond & 1 == 1 && cond != 0b1111 {
            !result
        } else {
            result
        }
    }
}
