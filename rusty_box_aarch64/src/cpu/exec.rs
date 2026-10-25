//! Fetch, decode and dispatch: one call of [`Cpu::step`] executes one
//! instruction or takes one exception.
//!
//! Decoding is split in two. The decoder (`isa/`, cached by `decode.rs`)
//! finds the word's A64 encoding, if it is one of a feature this core
//! implements, and the handler class that executes it; anything else is
//! UNDEFINED. The handler for that *class* then reads every operand field
//! from the word itself, following the field layout and decode pseudocode of
//! the Arm ARM. A class handler therefore covers its whole encoding group,
//! and a field the decoder does not expose cannot be misread through it.

use rusty_box_core::VmClock;

use super::exception::{Exception, ExceptionClass};
use super::isa::Class;
use super::ldst::SingleForm;
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
/// No variant says an encoding has no handler: dispatch matches every
/// class the decoder can return, so that cannot happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopCause {
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
        match self.decode_cache.decode(fetched.pa, insn) {
            Class::Undefined => self.undefined(pc, insn),
            Class::Hint => self.exec_hint(insn, pc),
            Class::AddSubImmediate => self.exec_addsub_imm(pc, insn),
            Class::AddSubShiftedRegister => self.exec_addsub_shift(pc, insn),
            Class::AddSubExtendedRegister => self.exec_addsub_ext(pc, insn),
            Class::AddSubWithCarry => self.exec_addsub_carry(pc, insn),
            Class::LogicalImmediate => self.exec_logic_imm(pc, insn),
            Class::LogicalShiftedRegister => self.exec_logic_shift(pc, insn),
            Class::MoveWide => self.exec_movewide(pc, insn),
            Class::PcRelAddressing => self.exec_pcrel(pc, insn),
            Class::Bitfield => self.exec_bitfield(pc, insn),
            Class::Extract => self.exec_extract(pc, insn),
            Class::ConditionalBranch => self.exec_condbranch(pc, insn),
            Class::CompareAndBranch => self.exec_compbranch(pc, insn),
            Class::TestAndBranch => self.exec_testbranch(pc, insn),
            Class::BranchImmediate => self.exec_branch_imm(pc, insn),
            Class::BranchRegister => self.exec_branch_reg(pc, insn),
            Class::ConditionalCompareImmediate | Class::ConditionalCompareRegister => self.exec_condcmp(pc, insn),
            Class::ConditionalSelect => self.exec_condsel(pc, insn),
            Class::DataProcessing1Source => self.exec_dp1(pc, insn),
            Class::DataProcessing2Source => self.exec_dp2(pc, insn),
            Class::DataProcessing3Source => self.exec_dp3(pc, insn),
            Class::ExceptionGeneration => self.exec_exception_gen(pc, insn),
            Class::System => self.exec_system(bus, pc, insn),
            Class::LoadStoreUnsignedImmediate => {
                self.exec_ldst_single(bus, pc, insn, SingleForm::UnsignedImmediate)
            }
            Class::LoadStoreUnscaled | Class::LoadStoreIndexed | Class::LoadStoreUnprivileged => {
                self.exec_ldst_single(bus, pc, insn, SingleForm::Immediate9)
            }
            Class::LoadStoreRegisterOffset => self.exec_ldst_single(bus, pc, insn, SingleForm::RegisterOffset),
            Class::LoadLiteral => self.exec_load_literal(bus, pc, insn),
            Class::LoadStorePairOffset | Class::LoadStorePairIndexed | Class::LoadStoreNoAllocatePair => {
                self.exec_ldst_pair(bus, pc, insn)
            }
            Class::LoadStoreExclusive => self.exec_ldst_exclusive(bus, pc, insn),
            Class::SimdLoadStoreMultiple | Class::SimdLoadStoreMultiplePostIndexed => {
                self.exec_ldst_multiple(bus, pc, insn)
            }
            Class::SimdLoadStoreSingle | Class::SimdLoadStoreSinglePostIndexed => {
                self.exec_ldst_single_structure(bus, pc, insn)
            }
            Class::SimdModifiedImmediate => self.exec_simd_imm(pc, insn),
            Class::SimdCopy => self.exec_simd_ins(pc, insn),
            Class::SimdThreeSame => self.exec_simd_same(pc, insn, false),
            Class::SimdScalarThreeSame => self.exec_simd_same(pc, insn, true),
            Class::SimdTwoRegisterMisc => self.exec_simd_misc(pc, insn, false),
            Class::SimdScalarTwoRegisterMisc => self.exec_simd_misc(pc, insn, true),
            Class::SimdShiftImmediate => self.exec_simd_shift_imm(pc, insn, false),
            Class::SimdScalarShiftImmediate => self.exec_simd_shift_imm(pc, insn, true),
            Class::SimdThreeDifferent => self.exec_simd_diff(pc, insn, false),
            Class::SimdScalarThreeDifferent => self.exec_simd_diff(pc, insn, true),
            Class::SimdIndexedElement => self.exec_simd_elem(pc, insn, false),
            Class::SimdScalarIndexedElement => self.exec_simd_elem(pc, insn, true),
            Class::SimdAcrossLanes => self.exec_simd_across(pc, insn),
            Class::SimdScalarPairwise => self.exec_simd_scalar_pair(pc, insn),
            Class::SimdScalarCopy => self.exec_simd_scalar_dup(pc, insn),
            Class::SimdExtract => self.exec_simd_ext(pc, insn),
            Class::SimdPermute => self.exec_simd_perm(pc, insn),
            Class::SimdTableLookup => self.exec_simd_tbl(pc, insn),
            Class::FpImmediate => self.exec_fmov_imm(pc, insn),
            Class::FpDataProcessing1Source => self.exec_fp_dp1(pc, insn),
            Class::FpDataProcessing2Source => self.exec_fp_dp2(pc, insn),
            Class::FpDataProcessing3Source => self.exec_fp_dp3(pc, insn),
            Class::FpCompare => self.exec_fp_compare(pc, insn),
            Class::FpConditionalCompare => self.exec_fp_cond_compare(pc, insn),
            Class::FpConditionalSelect => self.exec_fcsel(pc, insn),
            Class::FpIntegerConversion => self.exec_fp_int(pc, insn),
            Class::FpFixedPointConversion => self.exec_fp_fixed(pc, insn),
            Class::CryptoAes => self.exec_aes(insn),
            Class::CryptoShaThreeRegister => self.exec_sha_three(pc, insn),
            Class::CryptoShaTwoRegister => self.exec_sha_two(pc, insn),
            Class::Crc32 => self.exec_crc32(insn),
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
