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
use super::fp::FpType;
use super::isa::Class;
use super::ldst::SingleForm;
use super::{Activity, Btype, Cpu};
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
    /// A wait (`WFI`, `WFE`, `WFIT`, `WFET`, or a PSCI `CPU_SUSPEND`)
    /// retired and the processor now sleeps; [`Cpu::activity`] says what for
    /// until its wait ends.
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
    /// The instruction retired and the processor waits: its activity, which
    /// [`Cpu::begin_wait`] alone sets, names what for. The wait is not
    /// carried here, so that every instruction's result stays as small as a
    /// branch's.
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
        let btype = self.pstate.btype;
        let mut observed = Observed { bus, observer };
        let mut fetched = None;
        let result = self.execute(&mut observed, pc, &mut fetched);
        let observer = observed.observer;
        if result.is_err() {
            // An instruction that faults or stops completes nothing:
            // PSTATE.BTYPE stays what the branch to it left.
            self.pstate.btype = btype;
        }
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
                        if let Activity::Waiting(wait) = self.activity {
                            observer.wait(pc, wait);
                        }
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
        if self.pstate.btype != Btype::None {
            self.branch_target_check(fetched.guarded, insn)?;
        }
        match self.decode_cache.decode(fetched.pa, insn) {
            Class::Undefined => self.undefined(pc, insn),
            Class::Hint => self.exec_hint(insn, pc),
            Class::WaitWithTimeout => self.exec_wfxt(insn),
            Class::PointerAuthHint => self.exec_pauth_hint(insn),
            Class::AddSubImmediate => self.exec_addsub_imm(pc, insn),
            Class::AddSubShiftedRegister => self.exec_addsub_shift(pc, insn),
            Class::AddSubExtendedRegister => self.exec_addsub_ext(pc, insn),
            Class::AddSubWithCarry => self.exec_addsub_carry(pc, insn),
            Class::RotateMaskInsertFlags => self.exec_rmif(insn),
            Class::EvaluateIntoFlags => self.exec_setf(insn),
            Class::FlagManipulation => self.exec_flag_manipulation(pc, insn),
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
            Class::BranchRegister => self.exec_branch_reg(pc, insn, fetched.guarded),
            Class::AuthenticatedBranch => self.exec_branch_auth(pc, insn, fetched.guarded),
            Class::ConditionalCompareImmediate | Class::ConditionalCompareRegister => self.exec_condcmp(pc, insn),
            Class::ConditionalSelect => self.exec_condsel(pc, insn),
            Class::DataProcessing1Source => self.exec_dp1(pc, insn),
            Class::DataProcessing2Source => self.exec_dp2(pc, insn),
            Class::PointerAuth => self.exec_pauth(insn),
            Class::PointerAuthGeneric => self.exec_pacga(insn),
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
            Class::LoadAuthenticated => self.exec_load_auth(bus, insn),
            Class::LoadLiteral => self.exec_load_literal(bus, pc, insn),
            Class::LoadStorePairOffset | Class::LoadStorePairIndexed | Class::LoadStoreNoAllocatePair => {
                self.exec_ldst_pair(bus, pc, insn)
            }
            Class::LoadStoreExclusive => self.exec_ldst_exclusive(bus, pc, insn),
            Class::AtomicMemoryOperation => self.exec_memop(bus, pc, insn),
            Class::CompareAndSwap => self.exec_cas(bus, insn),
            Class::CompareAndSwapPair => self.exec_casp(bus, pc, insn),
            Class::LoadStoreOrderedUnscaled => self.exec_ldapstl_unscaled(bus, insn),
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
            Class::SimdRoundingDoublingMultiplyAccumulate => self.exec_simd_rdm(pc, insn, false),
            Class::SimdScalarRoundingDoublingMultiplyAccumulate => self.exec_simd_rdm(pc, insn, true),
            Class::SimdDotProduct => self.exec_simd_dot(pc, insn, false),
            Class::SimdDotProductIndexedElement => self.exec_simd_dot(pc, insn, true),
            Class::SimdMatrixMultiply => self.exec_simd_mmla(pc, insn),
            Class::SimdThreeSameHalf => self.exec_simd_same_fp(pc, insn, false, FpType::Half),
            Class::SimdScalarThreeSameHalf => self.exec_simd_same_fp(pc, insn, true, FpType::Half),
            Class::SimdTwoRegisterMiscHalf => self.exec_simd_misc_fp(pc, insn, false, FpType::Half),
            Class::SimdScalarTwoRegisterMiscHalf => {
                self.exec_simd_misc_fp(pc, insn, true, FpType::Half)
            }
            Class::SimdFpMulAddLong => self.exec_simd_fmlal(insn, false),
            Class::SimdFpMulAddLongIndexedElement => self.exec_simd_fmlal(insn, true),
            Class::SimdComplexMultiplyAccumulate => self.exec_simd_fcmla(pc, insn, false),
            Class::SimdComplexMultiplyAccumulateIndexedElement => self.exec_simd_fcmla(pc, insn, true),
            Class::SimdComplexAdd => self.exec_simd_fcadd(pc, insn),
            Class::SimdBfDotProduct => self.exec_simd_bfdot(insn, false),
            Class::SimdBfDotProductIndexedElement => self.exec_simd_bfdot(insn, true),
            Class::SimdBfMulAddLong => self.exec_simd_bfmlal(insn, false),
            Class::SimdBfMulAddLongIndexedElement => self.exec_simd_bfmlal(insn, true),
            Class::SimdBfMatrixMultiply => self.exec_simd_bfmmla(insn),
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
            Class::AddStreamingVectorLength => self.exec_addsvl(insn),
            Class::ReadStreamingVectorLength => self.exec_rdsvl(insn),
            Class::AddVectorLength => self.exec_addvl(insn),
            Class::ReadVectorLength => self.exec_rdvl(insn),
            Class::SveIntBinaryPredicated => self.exec_sve_int_bin_pred(pc, insn),
            Class::SveIntReduction => self.exec_sve_int_reduce(pc, insn),
            Class::SveMovprfx => self.exec_sve_movprfx(insn),
            Class::SveShiftPredicated => self.exec_sve_shift_pred(pc, insn),
            Class::SveIntUnaryPredicated => self.exec_sve_int_unary(pc, insn),
            Class::SveIntMulAddPredicated => self.exec_sve_mul_add(insn),
            Class::SveIntAddSubUnpredicated => self.exec_sve_add_sub(pc, insn),
            Class::SveBitwiseUnpredicated => self.exec_sve_bitwise(pc, insn),
            Class::SveShiftUnpredicated => self.exec_sve_shift_unpred(pc, insn),
            Class::SveIndex => self.exec_sve_index(insn),
            Class::SveIntMulUnpredicated => self.exec_sve_mul_unpred(pc, insn),
            Class::SveElementCount => self.exec_sve_element_count(pc, insn),
            Class::SveBitmaskImmediate => self.exec_sve_bitmask_imm(pc, insn),
            Class::SveCopyImmediate => self.exec_sve_copy_imm(pc, insn),
            Class::SveExtract => self.exec_sve_ext(insn),
            Class::SvePermuteUnpredicated => self.exec_sve_perm_unpred(pc, insn),
            Class::SvePermutePredicate => self.exec_sve_perm_predicate(pc, insn),
            Class::SvePermuteElements => self.exec_sve_perm_elements(pc, insn),
            Class::SvePermutePredicated => self.exec_sve_perm_predicated(pc, insn),
            Class::SveSelect => self.exec_sve_sel(insn),
            Class::SveIntCompare => self.exec_sve_compare(pc, insn),
            Class::SvePredicateLogical => self.exec_sve_pred_logical(insn),
            Class::SveBreak => self.exec_sve_break(insn),
            Class::SvePredicateMisc => self.exec_sve_pred_misc(pc, insn),
            Class::SveWhile => self.exec_sve_while(insn),
            Class::SveWhileCounter => self.exec_sve_while_counter(insn),
            Class::SveWhilePair => self.exec_sve_while_pair(insn),
            Class::SvePext => self.exec_sve_pext(insn),
            Class::SvePtrueCounter => self.exec_sve_ptrue_counter(insn),
            Class::SveCntpCounter => self.exec_sve_cntp_counter(insn),
            Class::SveConditionallyTerminate => self.exec_sve_cterm(insn),
            Class::SvePredicateCount => self.exec_sve_pred_count(pc, insn),
            Class::SveWideImmediate => self.exec_sve_wide_imm(pc, insn),
            Class::SveIntxDot => self.exec_sve_intx_dot(pc, insn),
            Class::SveIntxComplex => self.exec_sve_intx_complex(pc, insn),
            Class::SveIntxMulLong => self.exec_sve_intx_mul_long(pc, insn),
            Class::SveIntxMul => self.exec_sve_intx_mul(pc, insn),
            Class::SveIntxBinaryPredicated => self.exec_sve_intx_binary_pred(pc, insn),
            Class::SveIntxUnaryPredicated => self.exec_sve_intx_unary_pred(pc, insn),
            Class::SveIntxClamp => self.exec_sve_intx_clamp(insn),
            Class::SveIntxLongArithmetic => self.exec_sve_intx_long_arith(pc, insn),
            Class::SveIntxEorInterleaved => self.exec_sve_intx_eor_interleaved(insn),
            Class::SveIntxShiftLong => self.exec_sve_intx_shift_long(pc, insn),
            Class::SveIntxAbsDiffAccumulate => self.exec_sve_intx_abs_diff_acc(pc, insn),
            Class::SveIntxCarryLong => self.exec_sve_intx_carry_long(insn),
            Class::SveIntxComplexAdd => self.exec_sve_intx_complex_add(insn),
            Class::SveIntxShiftAccumulate => self.exec_sve_intx_shift_acc(pc, insn),
            Class::SveIntxNarrow => self.exec_sve_intx_narrow(pc, insn),
            Class::SveFpArithmeticPredicated => self.exec_sve_fp_arith_pred(pc, insn),
            Class::SveFpArithmeticUnpredicated => self.exec_sve_fp_arith_unpred(pc, insn),
            Class::SveFpReduction => self.exec_sve_fp_reduce(pc, insn),
            Class::SveFpCompare => self.exec_sve_fp_compare(pc, insn),
            Class::SveFpUnaryPredicated => self.exec_sve_fp_unary_pred(pc, insn),
            Class::SveFpEstimate => self.exec_sve_fp_estimate(pc, insn),
            Class::SveFpMulAdd => self.exec_sve_fp_mul_add(pc, insn),
            Class::SveFpIndexed => self.exec_sve_fp_indexed(insn),
            Class::SveFpComplex => self.exec_sve_fp_complex(pc, insn),
            Class::SveFpPairwise => self.exec_sve_fp_pairwise(pc, insn),
            Class::SveFpConvertOdd => self.exec_sve_fp_convert_odd(pc, insn),
            Class::SveFpWidening => self.exec_sve_fp_widening(insn),
            Class::SveFillSpill => self.exec_sve_fill_spill(bus, insn),
            Class::SvePrefetch => self.exec_sve_prefetch(),
            Class::SveLoadBroadcast => self.exec_sve_load_broadcast(bus, insn),
            Class::SveLoadBroadcastQuadword => self.exec_sve_load_broadcast_quadword(bus, insn),
            Class::SveLoadContiguous => self.exec_sve_load_contiguous(bus, insn),
            Class::SveLoadStructures => self.exec_sve_load_structures(bus, insn),
            Class::SveStoreContiguous => self.exec_sve_store_contiguous(bus, pc, insn),
            Class::SveStoreStructures => self.exec_sve_store_structures(bus, insn),
            Class::SveLoadMulti => self.exec_sve_load_multi(bus, insn),
            Class::SveStoreMulti => self.exec_sve_store_multi(bus, insn),
            Class::SveMultiSelect => self.exec_sve_multi_sel(insn),
            Class::SveMultiIntBinary => self.exec_sve_multi_int_binary(pc, insn),
            Class::SveMultiFpBinary => self.exec_sve_multi_fp_binary(pc, insn),
            Class::SveMultiClamp => self.exec_sve_multi_clamp(pc, insn),
            Class::SveMultiZip => self.exec_sve_multi_zip(pc, insn),
            Class::SveMultiNarrowShift => self.exec_sve_multi_narrow_shift(pc, insn),
            Class::SveMultiNarrowInt => self.exec_sve_multi_narrow_int(insn),
            Class::SveMultiNarrowFp => self.exec_sve_multi_narrow_fp(insn),
            Class::SveMultiFpUnary => self.exec_sve_multi_fp_unary(pc, insn),
            Class::SveMultiUnpack => self.exec_sve_multi_unpack(pc, insn),
            Class::SmeFpOuterProduct => self.exec_sme_fp_outer_product(insn),
            Class::SmeIntOuterProduct => self.exec_sme_int_outer_product(insn),
            Class::SmeMoveToSlice => self.exec_sme_move_to_slice(insn),
            Class::SmeMoveFromSlice => self.exec_sme_move_from_slice(insn),
            Class::SmeZero => self.exec_sme_zero(insn),
            Class::SmeAddVector => self.exec_sme_add_vector(insn),
            Class::SmeLoadSlice => self.exec_sme_load_slice(bus, insn),
            Class::SmeStoreSlice => self.exec_sme_store_slice(bus, insn),
            Class::SmeFillSpill => self.exec_sme_fill_spill(bus, insn),
            Class::SmeZeroZt0 => self.exec_sme_zero_zt0(),
            Class::SmeFillSpillZt0 => self.exec_sme_fill_spill_zt0(bus, insn),
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
