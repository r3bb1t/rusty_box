//! The pre-decoded form of an instruction: what a trace holds.
//!
//! [`predecode`] turns an instruction word and the handler class the
//! decoder gave it into an [`Op`]. For the instructions that dominate
//! guest code — the arithmetic and logical immediates, the shifted
//! register forms, moves, bitfields, conditional selects, multiply-add,
//! branches, the single-register and pair loads and stores of general
//! registers, and scalar floating-point arithmetic — the op carries the
//! operand fields its handler reads, extracted once, with what depends on
//! the word alone worked out ahead (a logical immediate's bit mask, a
//! branch's offset, a load's scaled offset). Every other instruction, and
//! every word of those classes that sets a reserved value, is
//! [`Op::Class`]: its class handler reads its operands from the word, as
//! the Arm ARM lays them out.
//!
//! An op depends on nothing but the word: not the address it was fetched
//! from (a trace is keyed by physical address, and one physical page may be
//! executed at several virtual addresses), not the exception level, not a
//! system register. Whatever does depend on them — the PC a branch is
//! relative to, the SP alignment check, the SIMD&FP enable — is read when
//! the op executes, exactly where its class handler reads it.
//!
//! Each specialised op executes exactly as its class handler executes the
//! word it came from; `tests/unit/op.rs` checks that over random words and
//! register states for every one.

use super::dp::{add_with_carry, datasize, decode_bit_masks, ones, ror, shift_reg};
use super::exec::{bit, bits, keep_fault, sext, Fault, Flow, OpResult};
use super::fp::{dp2_operation, dp3_form, scalar_type, FpBinary, FpDp3Regs, FpType};
use super::isa::Class;
use super::mmu::{DataAccess, ElementPair};
use super::Cpu;
use crate::bus::{AccessWidth, PhysBus};
use crate::softfloat::FusedMulAdd;

/// How a load extends what it read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Extend {
    /// LDR, LDRB, LDRH: zero-extended.
    Zero,
    /// LDRSB, LDRSH, LDRSW to a 64-bit register.
    Signed64,
    /// LDRSB, LDRSH to a 32-bit register.
    Signed32,
}

/// How a load or store with an immediate offset forms its address and
/// updates its base (Arm ARM "Load/store register" and "Load/store register
/// pair" classes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Index {
    /// `[Xn, #imm]`: the base is not written.
    Offset,
    /// `[Xn], #imm`: the access is at the base, then the base moves.
    Post,
    /// `[Xn, #imm]!`: the base moves, and the access is at its new value.
    Pre,
}

/// One pre-decoded instruction. Register fields are register numbers (31
/// names XZR or SP as the class handler reads it); `sf` is the 64-bit
/// form.
///
/// Closed (R5): `Cpu::exec_op` matches every variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    /// Executed by its class handler, which reads the operands from `word`.
    Class { class: Class, word: u32 },
    /// ADD/SUB (immediate) without flags: `Rd|SP = Rn|SP + imm`, `imm`
    /// negated for SUB.
    AddImm { sf: bool, rd: u8, rn: u8, imm: u64 },
    /// ADDS/SUBS (immediate): `AddWithCarry(Rn|SP, y, carry)`, with `y` the
    /// immediate (ADDS) or its complement (SUBS, carry in set).
    AddsImm {
        sf: bool,
        rd: u8,
        rn: u8,
        carry: bool,
        y: u64,
    },
    /// ADD/ADDS/SUB/SUBS (shifted register).
    AddSubShift {
        sf: bool,
        sub: bool,
        flags: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        shift: u8,
        amount: u8,
    },
    /// AND/ORR/EOR/ANDS (immediate), `opc` as encoded, with the
    /// `DecodeBitMasks` immediate.
    LogImm {
        sf: bool,
        opc: u8,
        rd: u8,
        rn: u8,
        imm: u64,
    },
    /// AND/BIC/ORR/ORN/EOR/EON/ANDS/BICS (shifted register), `opc` as
    /// encoded, `invert` its N bit.
    LogShift {
        sf: bool,
        opc: u8,
        invert: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        shift: u8,
        amount: u8,
    },
    /// MOVZ/MOVN: `Rd = value`, already sized.
    MovImm { rd: u8, value: u64 },
    /// MOVK: the 16 bits at `pos` replaced by `imm` (already shifted).
    Movk { sf: bool, rd: u8, pos: u8, imm: u64 },
    /// ADR: `Rd = PC + offset`.
    Adr { rd: u8, offset: u64 },
    /// ADRP: `Rd = (PC with bits 11:0 clear) + offset`.
    Adrp { rd: u8, offset: u64 },
    /// UBFM: `Rd = ROR(Rn, immr) & mask`, `mask` being `wmask & tmask`.
    Ubfm {
        sf: bool,
        rd: u8,
        rn: u8,
        immr: u8,
        mask: u64,
    },
    /// SBFM: the UBFM bits, with the bits above `tmask` copies of bit
    /// `imms` of Rn; `tmask` is `Ones(tmask_bits)`.
    Sbfm {
        sf: bool,
        rd: u8,
        rn: u8,
        immr: u8,
        imms: u8,
        tmask_bits: u8,
        wmask: u64,
    },
    /// BFM: the UBFM bits inserted into Rd.
    Bfm {
        sf: bool,
        rd: u8,
        rn: u8,
        immr: u8,
        tmask_bits: u8,
        wmask: u64,
    },
    /// CSEL/CSINC/CSINV/CSNEG: `invert` is bit 30, `increment` bit 10.
    CondSel {
        sf: bool,
        invert: bool,
        increment: bool,
        cond: u8,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    /// MADD/MSUB.
    Madd {
        sf: bool,
        sub: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    },
    /// B.cond.
    BCond { cond: u8, offset: u64 },
    /// CBZ/CBNZ.
    Cbz {
        sf: bool,
        nonzero: bool,
        rt: u8,
        offset: u64,
    },
    /// TBZ/TBNZ on bit `pos`.
    Tbz {
        nonzero: bool,
        rt: u8,
        pos: u8,
        offset: u64,
    },
    /// B.
    B { offset: u64 },
    /// BL.
    Bl { offset: u64 },
    /// BR, which sets PSTATE.BTYPE (FEAT_BTI).
    Br { rn: u8 },
    /// RET, which leaves PSTATE.BTYPE clear.
    Ret { rn: u8 },
    /// BLR, which sets PSTATE.BTYPE (FEAT_BTI).
    Blr { rn: u8 },
    /// Every hint this core executes as a NOP.
    Nop,
    /// LDR/LDRB/LDRH/LDRSB/LDRSH/LDRSW (immediate, any index) of a general
    /// register, `size` the log2 of the access.
    LoadImm {
        size: u8,
        extend: Extend,
        index: Index,
        rt: u8,
        rn: u8,
        imm: u64,
    },
    /// STR/STRB/STRH (immediate, any index) of a general register.
    StoreImm {
        size: u8,
        index: Index,
        rt: u8,
        rn: u8,
        imm: u64,
    },
    /// LDR... (register offset): the offset is `ExtendReg(Rm, option,
    /// shift)`.
    LoadReg {
        size: u8,
        extend: Extend,
        option: u8,
        shift: u8,
        rt: u8,
        rn: u8,
        rm: u8,
    },
    /// STR... (register offset).
    StoreReg {
        size: u8,
        option: u8,
        shift: u8,
        rt: u8,
        rn: u8,
        rm: u8,
    },
    /// LDP/LDNP of 32- or 64-bit general registers (`size` 2 or 3).
    LoadPair {
        size: u8,
        index: Index,
        rt: u8,
        rt2: u8,
        rn: u8,
        imm: u64,
    },
    /// STP/STNP of 32- or 64-bit general registers.
    StorePair {
        size: u8,
        index: Index,
        rt: u8,
        rt2: u8,
        rn: u8,
        imm: u64,
    },
    /// FMUL/FDIV/FADD/FSUB/FMAX/FMIN/FMAXNM/FMINNM/FNMUL (scalar), of
    /// SIMD&FP registers.
    FpDp2 {
        op: FpBinary,
        ty: FpType,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    /// FMADD/FMSUB/FNMADD/FNMSUB, of SIMD&FP registers.
    FpDp3 {
        form: FusedMulAdd,
        ty: FpType,
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    },
    /// MRS Xt, CNTPCT_EL0 and MRS Xt, CNTVCT_EL0: the system counter read,
    /// executed by the System class handler from `word`.
    ReadCounter { word: u32 },
}

const _: () = assert!(core::mem::size_of::<Op>() == 16);

/// A register field, as a byte.
#[inline(always)]
const fn reg(word: u32, lo: u32) -> u8 {
    bits(word, lo + 4, lo) as u8
}

/// The pre-decoded form of `word`, whose handler class is `class`.
pub(crate) fn predecode(class: Class, word: u32) -> Op {
    let generic = Op::Class { class, word };
    let sf = bit(word, 31);
    let rd = reg(word, 0);
    let rn = reg(word, 5);
    let rm = reg(word, 16);
    match class {
        Class::AddSubImmediate => {
            let imm = u64::from(bits(word, 21, 10)) << if bit(word, 22) { 12 } else { 0 };
            let sub = bit(word, 30);
            if bit(word, 29) {
                Op::AddsImm {
                    sf,
                    rd,
                    rn,
                    carry: sub,
                    y: if sub { !imm } else { imm },
                }
            } else {
                Op::AddImm {
                    sf,
                    rd,
                    rn,
                    imm: if sub { imm.wrapping_neg() } else { imm },
                }
            }
        }
        Class::AddSubShiftedRegister => {
            let shift = bits(word, 23, 22);
            let amount = bits(word, 15, 10);
            if shift == 3 || (!sf && amount >= 32) {
                return generic;
            }
            Op::AddSubShift {
                sf,
                sub: bit(word, 30),
                flags: bit(word, 29),
                rd,
                rn,
                rm,
                shift: shift as u8,
                amount: amount as u8,
            }
        }
        Class::LogicalImmediate => {
            let n = bit(word, 22);
            if !sf && n {
                return generic;
            }
            match decode_bit_masks(
                n,
                bits(word, 15, 10),
                bits(word, 21, 16),
                true,
                datasize(sf),
            ) {
                Some(masks) => Op::LogImm {
                    sf,
                    opc: bits(word, 30, 29) as u8,
                    rd,
                    rn,
                    imm: masks.wmask,
                },
                None => generic,
            }
        }
        Class::LogicalShiftedRegister => {
            let amount = bits(word, 15, 10);
            if !sf && amount >= 32 {
                return generic;
            }
            Op::LogShift {
                sf,
                opc: bits(word, 30, 29) as u8,
                invert: bit(word, 21),
                rd,
                rn,
                rm,
                shift: bits(word, 23, 22) as u8,
                amount: amount as u8,
            }
        }
        Class::MoveWide => {
            let opc = bits(word, 30, 29);
            let hw = bits(word, 22, 21);
            if opc == 1 || (!sf && hw >= 2) {
                return generic;
            }
            let pos = hw * 16;
            let imm = u64::from(bits(word, 20, 5)) << pos;
            match opc {
                0 => Op::MovImm {
                    rd,
                    value: !imm & ones(datasize(sf)),
                },
                2 => Op::MovImm { rd, value: imm },
                _ => Op::Movk {
                    sf,
                    rd,
                    pos: pos as u8,
                    imm,
                },
            }
        }
        Class::PcRelAddressing => {
            let imm = (u64::from(bits(word, 23, 5)) << 2) | u64::from(bits(word, 30, 29));
            if bit(word, 31) {
                Op::Adrp {
                    rd,
                    offset: sext(imm << 12, 33),
                }
            } else {
                Op::Adr {
                    rd,
                    offset: sext(imm, 21),
                }
            }
        }
        Class::Bitfield => {
            let opc = bits(word, 30, 29);
            let n = bit(word, 22);
            let immr = bits(word, 21, 16);
            let imms = bits(word, 15, 10);
            if opc == 3 || sf != n || (!sf && (immr >= 32 || imms >= 32)) {
                return generic;
            }
            let size = datasize(sf);
            let Some(masks) = decode_bit_masks(n, imms, immr, false, size) else {
                return generic;
            };
            // With N equal to sf and both immediates below the datasize,
            // DecodeBitMasks's element is the whole register, so its tmask
            // is Ones(((imms - immr) mod size) + 1).
            let tmask_bits = ((imms.wrapping_sub(immr) & (size - 1)) + 1) as u8;
            match opc {
                0 => Op::Sbfm {
                    sf,
                    rd,
                    rn,
                    immr: immr as u8,
                    imms: imms as u8,
                    tmask_bits,
                    wmask: masks.wmask,
                },
                1 => Op::Bfm {
                    sf,
                    rd,
                    rn,
                    immr: immr as u8,
                    tmask_bits,
                    wmask: masks.wmask,
                },
                _ => Op::Ubfm {
                    sf,
                    rd,
                    rn,
                    immr: immr as u8,
                    mask: masks.wmask & masks.tmask,
                },
            }
        }
        Class::ConditionalSelect => {
            if bit(word, 29) || bit(word, 11) {
                return generic;
            }
            Op::CondSel {
                sf,
                invert: bit(word, 30),
                increment: bit(word, 10),
                cond: bits(word, 15, 12) as u8,
                rd,
                rn,
                rm,
            }
        }
        Class::DataProcessing3Source => {
            if bits(word, 30, 29) != 0 || bits(word, 23, 21) != 0 {
                return generic;
            }
            Op::Madd {
                sf,
                sub: bit(word, 15),
                rd,
                rn,
                rm,
                ra: reg(word, 10),
            }
        }
        Class::ConditionalBranch => {
            if bit(word, 4) {
                return generic;
            }
            Op::BCond {
                cond: bits(word, 3, 0) as u8,
                offset: sext(u64::from(bits(word, 23, 5)) << 2, 21),
            }
        }
        Class::CompareAndBranch => Op::Cbz {
            sf,
            nonzero: bit(word, 24),
            rt: rd,
            offset: sext(u64::from(bits(word, 23, 5)) << 2, 21),
        },
        Class::TestAndBranch => Op::Tbz {
            nonzero: bit(word, 24),
            rt: rd,
            pos: ((u32::from(sf) << 5) | bits(word, 23, 19)) as u8,
            offset: sext(u64::from(bits(word, 18, 5)) << 2, 16),
        },
        Class::BranchImmediate => {
            let offset = sext(u64::from(bits(word, 25, 0)) << 2, 28);
            if sf {
                Op::Bl { offset }
            } else {
                Op::B { offset }
            }
        }
        Class::BranchRegister => {
            if bits(word, 20, 16) != 0x1F || bits(word, 15, 10) != 0 || bits(word, 4, 0) != 0 {
                return generic;
            }
            match bits(word, 24, 21) {
                0 => Op::Br { rn },
                1 => Op::Blr { rn },
                2 => Op::Ret { rn },
                _ => generic,
            }
        }
        Class::Hint => match bits(word, 11, 5) {
            // WFE, WFI, SEV and SEVL have their own effects.
            2..=5 => generic,
            _ => Op::Nop,
        },
        Class::LoadStoreUnsignedImmediate => {
            let size = bits(word, 31, 30);
            if bit(word, 26) {
                return generic;
            }
            single_immediate(
                word,
                size,
                Index::Offset,
                u64::from(bits(word, 21, 10)) << size,
            )
            .unwrap_or(generic)
        }
        Class::LoadStoreUnscaled | Class::LoadStoreIndexed => {
            let size = bits(word, 31, 30);
            if bit(word, 26) {
                return generic;
            }
            let index = match bits(word, 11, 10) {
                0b00 => Index::Offset,
                0b01 => Index::Post,
                0b11 => Index::Pre,
                // LDTR/STTR: EL0 permissions.
                _ => return generic,
            };
            single_immediate(word, size, index, sext(u64::from(bits(word, 20, 12)), 9))
                .unwrap_or(generic)
        }
        Class::LoadStoreRegisterOffset => {
            let size = bits(word, 31, 30);
            let option = bits(word, 15, 13);
            if bit(word, 26) || option & 2 == 0 {
                return generic;
            }
            let shift = if bit(word, 12) { size } else { 0 } as u8;
            let (rt, option) = (rd, option as u8);
            match bits(word, 23, 22) {
                0 => Op::StoreReg {
                    size: size as u8,
                    option,
                    shift,
                    rt,
                    rn,
                    rm,
                },
                1 => Op::LoadReg {
                    size: size as u8,
                    extend: Extend::Zero,
                    option,
                    shift,
                    rt,
                    rn,
                    rm,
                },
                2 if size < 3 => Op::LoadReg {
                    size: size as u8,
                    extend: Extend::Signed64,
                    option,
                    shift,
                    rt,
                    rn,
                    rm,
                },
                3 if size < 2 => Op::LoadReg {
                    size: size as u8,
                    extend: Extend::Signed32,
                    option,
                    shift,
                    rt,
                    rn,
                    rm,
                },
                // PRFM, and LDRS* with no 32-bit form.
                _ => generic,
            }
        }
        Class::LoadStorePairOffset
        | Class::LoadStorePairIndexed
        | Class::LoadStoreNoAllocatePair => {
            let size = match (bit(word, 26), bits(word, 31, 30)) {
                (false, 0) => 2,
                (false, 2) => 3,
                // SIMD&FP pairs, LDPSW and the reserved opc.
                _ => return generic,
            };
            let index = match bits(word, 24, 23) {
                0b01 => Index::Post,
                0b11 => Index::Pre,
                _ => Index::Offset,
            };
            let imm = sext(u64::from(bits(word, 21, 15)), 7) << size;
            let (rt, rt2) = (rd, reg(word, 10));
            if bit(word, 22) {
                Op::LoadPair {
                    size,
                    index,
                    rt,
                    rt2,
                    rn,
                    imm,
                }
            } else {
                Op::StorePair {
                    size,
                    index,
                    rt,
                    rt2,
                    rn,
                    imm,
                }
            }
        }
        Class::FpDataProcessing2Source => match (scalar_type(word), dp2_operation(word)) {
            (Some(ty), Some(op)) => Op::FpDp2 { op, ty, rd, rn, rm },
            _ => generic,
        },
        Class::FpDataProcessing3Source => match scalar_type(word) {
            Some(ty) => Op::FpDp3 {
                form: dp3_form(word),
                ty,
                rd,
                rn,
                rm,
                ra: reg(word, 10),
            },
            None => generic,
        },
        Class::System => {
            // MRS Xt, CNTPCT_EL0 (op2 1) and MRS Xt, CNTVCT_EL0 (op2 2):
            // the system register encoding with Rt masked off.
            if matches!(word & !0x1F, 0xD53B_E020 | 0xD53B_E040) {
                Op::ReadCounter { word }
            } else {
                generic
            }
        }
        Class::Undefined
        | Class::Extract
        | Class::ExceptionGeneration
        | Class::SimdLoadStoreMultiple
        | Class::SimdLoadStoreMultiplePostIndexed
        | Class::SimdLoadStoreSingle
        | Class::SimdLoadStoreSinglePostIndexed
        | Class::LoadStoreExclusive
        | Class::LoadLiteral
        | Class::LoadStoreUnprivileged
        | Class::DataProcessing2Source
        | Class::DataProcessing1Source
        | Class::AddSubExtendedRegister
        | Class::AddSubWithCarry
        | Class::ConditionalCompareRegister
        | Class::ConditionalCompareImmediate
        | Class::SimdScalarCopy
        | Class::SimdScalarThreeSame
        | Class::SimdScalarTwoRegisterMisc
        | Class::SimdScalarPairwise
        | Class::SimdScalarThreeDifferent
        | Class::SimdScalarShiftImmediate
        | Class::SimdScalarIndexedElement
        | Class::SimdTableLookup
        | Class::SimdPermute
        | Class::SimdExtract
        | Class::SimdCopy
        | Class::SimdThreeSame
        | Class::SimdTwoRegisterMisc
        | Class::SimdAcrossLanes
        | Class::SimdThreeDifferent
        | Class::SimdModifiedImmediate
        | Class::SimdShiftImmediate
        | Class::SimdIndexedElement
        | Class::FpFixedPointConversion
        | Class::FpIntegerConversion
        | Class::FpDataProcessing1Source
        | Class::FpCompare
        | Class::FpImmediate
        | Class::FpConditionalCompare
        | Class::FpConditionalSelect
        | Class::CryptoAes
        | Class::CryptoShaThreeRegister
        | Class::CryptoShaTwoRegister
        | Class::Crc32
        | Class::PointerAuthHint
        | Class::WaitWithTimeout
        | Class::FlagManipulation
        | Class::AuthenticatedBranch
        | Class::LoadAuthenticated
        | Class::PointerAuth
        | Class::PointerAuthGeneric
        | Class::RotateMaskInsertFlags
        | Class::EvaluateIntoFlags
        | Class::SimdRoundingDoublingMultiplyAccumulate
        | Class::SimdScalarRoundingDoublingMultiplyAccumulate
        | Class::SimdDotProduct
        | Class::SimdDotProductIndexedElement
        | Class::SimdMatrixMultiply
        | Class::SimdThreeSameHalf
        | Class::SimdScalarThreeSameHalf
        | Class::SimdTwoRegisterMiscHalf
        | Class::SimdScalarTwoRegisterMiscHalf
        | Class::SimdFpMulAddLong
        | Class::SimdFpMulAddLongIndexedElement
        | Class::SimdComplexMultiplyAccumulate
        | Class::SimdComplexMultiplyAccumulateIndexedElement
        | Class::SimdComplexAdd
        | Class::SimdBfDotProduct
        | Class::SimdBfDotProductIndexedElement
        | Class::SimdBfMulAddLong
        | Class::SimdBfMulAddLongIndexedElement
        | Class::SimdBfMatrixMultiply
        | Class::AtomicMemoryOperation
        | Class::CompareAndSwap
        | Class::CompareAndSwapPair
        | Class::LoadStoreOrderedUnscaled
        | Class::AddStreamingVectorLength
        | Class::ReadStreamingVectorLength
        | Class::AddVectorLength
        | Class::ReadVectorLength
        | Class::SveIntBinaryPredicated
        | Class::SveIntReduction
        | Class::SveMovprfx
        | Class::SveShiftPredicated
        | Class::SveIntUnaryPredicated
        | Class::SveIntMulAddPredicated
        | Class::SveIntAddSubUnpredicated
        | Class::SveBitwiseUnpredicated
        | Class::SveShiftUnpredicated
        | Class::SveIndex
        | Class::SveIntMulUnpredicated
        | Class::SveElementCount
        | Class::SveBitmaskImmediate
        | Class::SveCopyImmediate
        | Class::SveExtract
        | Class::SvePermuteUnpredicated
        | Class::SvePermutePredicate
        | Class::SvePermuteElements
        | Class::SvePermutePredicated
        | Class::SveSelect
        | Class::SveIntCompare
        | Class::SvePredicateLogical
        | Class::SveBreak
        | Class::SvePredicateMisc
        | Class::SveWhile
        | Class::SveWhileCounter
        | Class::SveWhilePair
        | Class::SvePext
        | Class::SvePtrueCounter
        | Class::SveCntpCounter
        | Class::SveConditionallyTerminate
        | Class::SvePredicateCount
        | Class::SveWideImmediate
        | Class::SveIntxDot
        | Class::SveIntxComplex
        | Class::SveIntxMulLong
        | Class::SveIntxLongArithmetic
        | Class::SveIntxEorInterleaved
        | Class::SveIntxShiftLong
        | Class::SveIntxAbsDiffAccumulate
        | Class::SveIntxCarryLong
        | Class::SveIntxComplexAdd
        | Class::SveIntxShiftAccumulate
        | Class::SveIntxNarrow
        | Class::SveIntxNarrowPair
        | Class::SveIntxMul
        | Class::SveIntxBinaryPredicated
        | Class::SveIntxUnaryPredicated
        | Class::SveIntxClamp
        | Class::SveFpArithmeticPredicated
        | Class::SveFpArithmeticUnpredicated
        | Class::SveFpReduction
        | Class::SveFpCompare
        | Class::SveFpUnaryPredicated
        | Class::SveFpEstimate
        | Class::SveFpMulAdd
        | Class::SveFpIndexed
        | Class::SveFpClamp
        | Class::SveFpComplex
        | Class::SveFpPairwise
        | Class::SveFpConvertOdd
        | Class::SveFpWidening
        | Class::SveFillSpill
        | Class::SvePrefetch
        | Class::SveLoadBroadcast
        | Class::SveLoadBroadcastQuadword
        | Class::SveLoadContiguous
        | Class::SveLoadStructures
        | Class::SveStoreContiguous
        | Class::SveStoreStructures
        | Class::SveLoadMulti
        | Class::SveStoreMulti
        | Class::SveMultiSelect
        | Class::SveMultiIntBinary
        | Class::SveMultiFpBinary
        | Class::SveMultiClamp
        | Class::SveMultiZip
        | Class::SveMultiNarrowShift
        | Class::SveMultiNarrowInt
        | Class::SveMultiNarrowFp
        | Class::SveMultiFpUnary
        | Class::SveMultiUnpack
        | Class::SmeArrayIntLongLong
        | Class::SmeArrayIntLong
        | Class::SmeArrayFpLong
        | Class::SmeArrayFpDot
        | Class::SmeArrayIntDot
        | Class::SmeArrayFpMulAdd
        | Class::SmeArrayAddVectors
        | Class::SmeArrayAccumulate
        | Class::SmeIndexedIntDot
        | Class::SmeIndexedFpMulAdd
        | Class::SmeIndexedVerticalDot
        | Class::SmeBinaryOuterProduct
        | Class::SmeIntOuterProductHalfwords
        | Class::SmeMoveGroupToSlices
        | Class::SmeMoveGroupFromSlices
        | Class::SmeMoveGroupToArray
        | Class::SmeMoveGroupFromArray
        | Class::SmeLookupTable
        | Class::SmeFpOuterProduct
        | Class::SmeIntOuterProduct
        | Class::SmeMoveToSlice
        | Class::SmeMoveFromSlice
        | Class::SmeZero
        | Class::SmeAddVector
        | Class::SmeLoadSlice
        | Class::SmeStoreSlice
        | Class::SmeFillSpill
        | Class::SmeZeroZt0
        | Class::SmeFillSpillZt0 => generic,
    }
}

/// A general-register load or store with an immediate offset, `opc` read
/// from the word: `None` for a prefetch and for the encodings the handler
/// rejects (LDRS* to a 32-bit register of a word or doubleword).
fn single_immediate(word: u32, size: u32, index: Index, imm: u64) -> Option<Op> {
    let (rt, rn, size8) = (reg(word, 0), reg(word, 5), size as u8);
    match bits(word, 23, 22) {
        0 => Some(Op::StoreImm {
            size: size8,
            index,
            rt,
            rn,
            imm,
        }),
        1 => Some(Op::LoadImm {
            size: size8,
            extend: Extend::Zero,
            index,
            rt,
            rn,
            imm,
        }),
        2 if size < 3 => Some(Op::LoadImm {
            size: size8,
            extend: Extend::Signed64,
            index,
            rt,
            rn,
            imm,
        }),
        3 if size < 2 => Some(Op::LoadImm {
            size: size8,
            extend: Extend::Signed32,
            index,
            rt,
            rn,
            imm,
        }),
        _ => None,
    }
}

impl Op {
    /// Whether a trace ends after this instruction: control always leaves
    /// sequential execution, or the instruction can change what the
    /// processor does between instructions — the translation regime, the
    /// interrupt masks, the exception vectors, the instruction caches, or
    /// whether it sleeps. A conditional branch does not end a trace: when it
    /// is not taken, execution goes on in sequence (Bochs `BX_TRACE_END`
    /// marks unconditional control transfers, and a taken branch leaves the
    /// trace when it executes).
    pub(crate) const fn ends_trace(self) -> bool {
        match self {
            Self::B { .. }
            | Self::Bl { .. }
            | Self::Br { .. }
            | Self::Ret { .. }
            | Self::Blr { .. } => true,
            Self::Class { class, word } => class_ends_trace(class, word),
            Self::AddImm { .. }
            | Self::AddsImm { .. }
            | Self::AddSubShift { .. }
            | Self::LogImm { .. }
            | Self::LogShift { .. }
            | Self::MovImm { .. }
            | Self::Movk { .. }
            | Self::Adr { .. }
            | Self::Adrp { .. }
            | Self::Ubfm { .. }
            | Self::Sbfm { .. }
            | Self::Bfm { .. }
            | Self::CondSel { .. }
            | Self::Madd { .. }
            | Self::BCond { .. }
            | Self::Cbz { .. }
            | Self::Tbz { .. }
            | Self::Nop
            | Self::LoadImm { .. }
            | Self::StoreImm { .. }
            | Self::LoadReg { .. }
            | Self::StoreReg { .. }
            | Self::LoadPair { .. }
            | Self::StorePair { .. }
            | Self::FpDp2 { .. }
            | Self::FpDp3 { .. }
            | Self::ReadCounter { .. } => false,
        }
    }
}

/// [`Op::ends_trace`] for an instruction its class handler executes.
const fn class_ends_trace(class: Class, word: u32) -> bool {
    match class {
        // UNDEFINED, SVC/HVC/BRK, and BR/BLR/RET/ERET forms with any field
        // the handler rejects: control leaves, or the instruction faults.
        Class::Undefined | Class::ExceptionGeneration | Class::BranchImmediate | Class::BranchRegister => true,
        // BRAA, BLRAA, RETAA, ERETAA and their other forms (FEAT_PAuth):
        // control leaves, or the instruction faults.
        Class::AuthenticatedBranch => true,
        // WFE, WFI, SEV and SEVL; and WFET and WFIT (FEAT_WFxT), which may
        // wait.
        Class::Hint | Class::WaitWithTimeout => true,
        Class::System => {
            let read = bit(word, 21);
            let op0 = bits(word, 20, 19);
            let crn = bits(word, 15, 12);
            let op2 = bits(word, 7, 5);
            // MRS reads; CLREX, DSB and DMB order nothing an interpreter
            // reorders. ISB, the PSTATE writes (DAIFSet, DAIFClr, SPSel),
            // SYS (cache, TLB and address-translation maintenance) and
            // every MSR end the trace, as each ends a translation block in
            // QEMU (target/arm/tcg/translate-a64.c `trans_ISB`, the
            // `trans_MSR_i_*` functions, and `handle_sys`, which by default
            // ends the block after any system register write).
            !(read || (op0 == 0 && crn == 3 && matches!(op2, 2 | 4 | 5)))
        }
        Class::PcRelAddressing
        | Class::AddSubImmediate
        | Class::LogicalImmediate
        | Class::MoveWide
        | Class::Bitfield
        | Class::Extract
        | Class::ConditionalBranch
        | Class::CompareAndBranch
        | Class::TestAndBranch
        | Class::SimdLoadStoreMultiple
        | Class::SimdLoadStoreMultiplePostIndexed
        | Class::SimdLoadStoreSingle
        | Class::SimdLoadStoreSinglePostIndexed
        | Class::LoadStoreExclusive
        | Class::LoadLiteral
        | Class::LoadStoreNoAllocatePair
        | Class::LoadStorePairIndexed
        | Class::LoadStorePairOffset
        | Class::LoadStoreUnscaled
        | Class::LoadStoreIndexed
        | Class::LoadStoreUnprivileged
        | Class::LoadStoreRegisterOffset
        | Class::LoadStoreUnsignedImmediate
        | Class::DataProcessing2Source
        | Class::DataProcessing1Source
        | Class::LogicalShiftedRegister
        | Class::AddSubShiftedRegister
        | Class::AddSubExtendedRegister
        | Class::AddSubWithCarry
        | Class::ConditionalCompareRegister
        | Class::ConditionalCompareImmediate
        | Class::ConditionalSelect
        | Class::DataProcessing3Source
        | Class::SimdScalarCopy
        | Class::SimdScalarThreeSame
        | Class::SimdScalarTwoRegisterMisc
        | Class::SimdScalarPairwise
        | Class::SimdScalarThreeDifferent
        | Class::SimdScalarShiftImmediate
        | Class::SimdScalarIndexedElement
        | Class::SimdTableLookup
        | Class::SimdPermute
        | Class::SimdExtract
        | Class::SimdCopy
        | Class::SimdThreeSame
        | Class::SimdTwoRegisterMisc
        | Class::SimdAcrossLanes
        | Class::SimdThreeDifferent
        | Class::SimdModifiedImmediate
        | Class::SimdShiftImmediate
        | Class::SimdIndexedElement
        | Class::FpFixedPointConversion
        | Class::FpIntegerConversion
        | Class::FpDataProcessing1Source
        | Class::FpCompare
        | Class::FpImmediate
        | Class::FpConditionalCompare
        | Class::FpDataProcessing2Source
        | Class::FpConditionalSelect
        | Class::FpDataProcessing3Source
        | Class::CryptoAes
        | Class::CryptoShaThreeRegister
        | Class::CryptoShaTwoRegister
        | Class::Crc32
        // The pointer authentication hints and data-processing forms: they
        // write registers, and a failed authentication with FEAT_FPAC
        // faults as any instruction may.
        | Class::PointerAuthHint
        | Class::PointerAuth
        | Class::PointerAuthGeneric
        | Class::LoadAuthenticated
        // NZCV only.
        | Class::FlagManipulation
        | Class::RotateMaskInsertFlags
        | Class::EvaluateIntoFlags
        | Class::SimdRoundingDoublingMultiplyAccumulate
        | Class::SimdScalarRoundingDoublingMultiplyAccumulate
        | Class::SimdDotProduct
        | Class::SimdDotProductIndexedElement
        | Class::SimdMatrixMultiply
        | Class::SimdThreeSameHalf
        | Class::SimdScalarThreeSameHalf
        | Class::SimdTwoRegisterMiscHalf
        | Class::SimdScalarTwoRegisterMiscHalf
        | Class::SimdFpMulAddLong
        | Class::SimdFpMulAddLongIndexedElement
        | Class::SimdComplexMultiplyAccumulate
        | Class::SimdComplexMultiplyAccumulateIndexedElement
        | Class::SimdComplexAdd
        | Class::SimdBfDotProduct
        | Class::SimdBfDotProductIndexedElement
        | Class::SimdBfMulAddLong
        | Class::SimdBfMulAddLongIndexedElement
        | Class::SimdBfMatrixMultiply
        | Class::AtomicMemoryOperation
        | Class::CompareAndSwap
        | Class::CompareAndSwapPair
        | Class::LoadStoreOrderedUnscaled
        // The SVE and SME data-processing, load and store instructions:
        // what they may execute depends on PSTATE.SM and PSTATE.ZA, which
        // only an MSR changes (SMSTART, SMSTOP, MSR SVCR*), and an MSR ends
        // the trace.
        | Class::AddStreamingVectorLength
        | Class::ReadStreamingVectorLength
        | Class::AddVectorLength
        | Class::ReadVectorLength
        | Class::SveIntBinaryPredicated
        | Class::SveIntReduction
        | Class::SveMovprfx
        | Class::SveShiftPredicated
        | Class::SveIntUnaryPredicated
        | Class::SveIntMulAddPredicated
        | Class::SveIntAddSubUnpredicated
        | Class::SveBitwiseUnpredicated
        | Class::SveShiftUnpredicated
        | Class::SveIndex
        | Class::SveIntMulUnpredicated
        | Class::SveElementCount
        | Class::SveBitmaskImmediate
        | Class::SveCopyImmediate
        | Class::SveExtract
        | Class::SvePermuteUnpredicated
        | Class::SvePermutePredicate
        | Class::SvePermuteElements
        | Class::SvePermutePredicated
        | Class::SveSelect
        | Class::SveIntCompare
        | Class::SvePredicateLogical
        | Class::SveBreak
        | Class::SvePredicateMisc
        | Class::SveWhile
        | Class::SveWhileCounter
        | Class::SveWhilePair
        | Class::SvePext
        | Class::SvePtrueCounter
        | Class::SveCntpCounter
        | Class::SveConditionallyTerminate
        | Class::SvePredicateCount
        | Class::SveWideImmediate
        | Class::SveIntxDot
        | Class::SveIntxComplex
        | Class::SveIntxMulLong
        | Class::SveIntxLongArithmetic
        | Class::SveIntxEorInterleaved
        | Class::SveIntxShiftLong
        | Class::SveIntxAbsDiffAccumulate
        | Class::SveIntxCarryLong
        | Class::SveIntxComplexAdd
        | Class::SveIntxShiftAccumulate
        | Class::SveIntxNarrow
        | Class::SveIntxNarrowPair
        | Class::SveIntxMul
        | Class::SveIntxBinaryPredicated
        | Class::SveIntxUnaryPredicated
        | Class::SveIntxClamp
        | Class::SveFpArithmeticPredicated
        | Class::SveFpArithmeticUnpredicated
        | Class::SveFpReduction
        | Class::SveFpCompare
        | Class::SveFpUnaryPredicated
        | Class::SveFpEstimate
        | Class::SveFpMulAdd
        | Class::SveFpIndexed
        | Class::SveFpClamp
        | Class::SveFpComplex
        | Class::SveFpPairwise
        | Class::SveFpConvertOdd
        | Class::SveFpWidening
        | Class::SveFillSpill
        | Class::SvePrefetch
        | Class::SveLoadBroadcast
        | Class::SveLoadBroadcastQuadword
        | Class::SveLoadContiguous
        | Class::SveLoadStructures
        | Class::SveStoreContiguous
        | Class::SveStoreStructures
        | Class::SveLoadMulti
        | Class::SveStoreMulti
        | Class::SveMultiSelect
        | Class::SveMultiIntBinary
        | Class::SveMultiFpBinary
        | Class::SveMultiClamp
        | Class::SveMultiZip
        | Class::SveMultiNarrowShift
        | Class::SveMultiNarrowInt
        | Class::SveMultiNarrowFp
        | Class::SveMultiFpUnary
        | Class::SveMultiUnpack
        | Class::SmeArrayIntLongLong
        | Class::SmeArrayIntLong
        | Class::SmeArrayFpLong
        | Class::SmeArrayFpDot
        | Class::SmeArrayIntDot
        | Class::SmeArrayFpMulAdd
        | Class::SmeArrayAddVectors
        | Class::SmeArrayAccumulate
        | Class::SmeIndexedIntDot
        | Class::SmeIndexedFpMulAdd
        | Class::SmeIndexedVerticalDot
        | Class::SmeBinaryOuterProduct
        | Class::SmeIntOuterProductHalfwords
        | Class::SmeMoveGroupToSlices
        | Class::SmeMoveGroupFromSlices
        | Class::SmeMoveGroupToArray
        | Class::SmeMoveGroupFromArray
        | Class::SmeLookupTable
        | Class::SmeFpOuterProduct
        | Class::SmeIntOuterProduct
        | Class::SmeMoveToSlice
        | Class::SmeMoveFromSlice
        | Class::SmeZero
        | Class::SmeAddVector
        | Class::SmeLoadSlice
        | Class::SmeStoreSlice
        | Class::SmeFillSpill
        | Class::SmeZeroZt0
        | Class::SmeFillSpillZt0 => false,
    }
}

/// Where [`Cpu::exec_op`] reads the op it executes: an op of its own, or
/// one that lies in the processor (a trace's, in the pool).
pub(crate) trait OpSource {
    /// The op, borrowed from `self` or from `cpu`.
    fn op<'c>(&'c self, cpu: &'c Cpu) -> &'c Op;
}

impl OpSource for Op {
    #[inline(always)]
    fn op<'c>(&'c self, _: &'c Cpu) -> &'c Op {
        self
    }
}

impl Cpu {
    /// Execute the op `source` gives, the instruction at `pc`. A fault it
    /// raises is kept in `raised`, which the result's `Err` refers to (see
    /// [`OpResult`]).
    ///
    /// The match reads the op where it lies: the dispatch loads only the
    /// variant, and each arm loads its own fields as it binds them, so the
    /// trace loop's register allocation holds no field of any other arm.
    #[inline(always)]
    pub(crate) fn exec_op<'f, B: PhysBus, S: OpSource>(
        &mut self,
        bus: &mut B,
        pc: u64,
        source: S,
        raised: &'f mut Option<Fault>,
    ) -> OpResult<'f> {
        // The value of a step that completed; a fault ends the instruction,
        // kept in `raised`.
        macro_rules! completed {
            ($step:expr) => {
                match $step {
                    Ok(value) => value,
                    Err(fault) => return Err(keep_fault(raised, fault)),
                }
            };
        }
        match *source.op(self) {
            Op::Class { class, word } => Ok(completed!(self.exec_class(bus, pc, class, word))),
            Op::ReadCounter { word } => {
                Ok(completed!(self.exec_class(bus, pc, Class::System, word)))
            }
            Op::AddImm { sf, rd, rn, imm } => {
                let value = self.x_or_sp(u32::from(rn)).wrapping_add(imm);
                self.set_x_or_sp_sized(u32::from(rd), value, sf);
                Ok(Flow::Next)
            }
            Op::AddsImm {
                sf,
                rd,
                rn,
                carry,
                y,
            } => {
                let r = add_with_carry(self.x_or_sp(u32::from(rn)), y, carry, sf);
                self.set_flags(r);
                self.set_x_or_zr_sized(u32::from(rd), r.value, sf);
                Ok(Flow::Next)
            }
            Op::AddSubShift {
                sf,
                sub,
                flags,
                rd,
                rn,
                rm,
                shift,
                amount,
            } => {
                let op2 = shift_reg(
                    self.x_or_zr(u32::from(rm)),
                    u32::from(shift),
                    u32::from(amount),
                    datasize(sf),
                );
                let op1 = self.x_or_zr(u32::from(rn));
                let r = if sub {
                    add_with_carry(op1, !op2, true, sf)
                } else {
                    add_with_carry(op1, op2, false, sf)
                };
                if flags {
                    self.set_flags(r);
                }
                self.set_x_or_zr_sized(u32::from(rd), r.value, sf);
                Ok(Flow::Next)
            }
            Op::LogImm {
                sf,
                opc,
                rd,
                rn,
                imm,
            } => {
                let op1 = self.x_or_zr(u32::from(rn));
                let value = match opc {
                    0 | 3 => op1 & imm,
                    1 => op1 | imm,
                    _ => op1 ^ imm,
                } & ones(datasize(sf));
                if opc == 3 {
                    self.set_logic_flags(value, sf);
                    self.set_x_or_zr(u32::from(rd), value);
                } else {
                    self.set_x_or_sp(u32::from(rd), value);
                }
                Ok(Flow::Next)
            }
            Op::LogShift {
                sf,
                opc,
                invert,
                rd,
                rn,
                rm,
                shift,
                amount,
            } => {
                let size = datasize(sf);
                let mut op2 = shift_reg(
                    self.x_or_zr(u32::from(rm)),
                    u32::from(shift),
                    u32::from(amount),
                    size,
                );
                if invert {
                    op2 = !op2 & ones(size);
                }
                let op1 = self.x_or_zr(u32::from(rn)) & ones(size);
                let value = match opc {
                    0 | 3 => op1 & op2,
                    1 => op1 | op2,
                    _ => op1 ^ op2,
                };
                if opc == 3 {
                    self.set_logic_flags(value, sf);
                }
                self.set_x_or_zr(u32::from(rd), value);
                Ok(Flow::Next)
            }
            Op::MovImm { rd, value } => {
                self.set_x_or_zr(u32::from(rd), value);
                Ok(Flow::Next)
            }
            Op::Movk { sf, rd, pos, imm } => {
                let value = (self.x_or_zr(u32::from(rd)) & !(0xFFFFu64 << pos)) | imm;
                self.set_x_or_zr_sized(u32::from(rd), value, sf);
                Ok(Flow::Next)
            }
            Op::Adr { rd, offset } => {
                self.set_x_or_zr(u32::from(rd), pc.wrapping_add(offset));
                Ok(Flow::Next)
            }
            Op::Adrp { rd, offset } => {
                self.set_x_or_zr(u32::from(rd), (pc & !0xFFF).wrapping_add(offset));
                Ok(Flow::Next)
            }
            Op::Ubfm {
                sf,
                rd,
                rn,
                immr,
                mask,
            } => {
                let size = datasize(sf);
                let src = self.x_or_zr(u32::from(rn)) & ones(size);
                self.set_x_or_zr(u32::from(rd), ror(src, u32::from(immr), size) & mask);
                Ok(Flow::Next)
            }
            Op::Sbfm {
                sf,
                rd,
                rn,
                immr,
                imms,
                tmask_bits,
                wmask,
            } => {
                let size = datasize(sf);
                let src = self.x_or_zr(u32::from(rn)) & ones(size);
                let tmask = ones(u32::from(tmask_bits));
                let bot = ror(src, u32::from(immr), size) & wmask;
                let top = if (src >> imms) & 1 != 0 {
                    ones(size)
                } else {
                    0
                };
                self.set_x_or_zr(u32::from(rd), ((top & !tmask) | (bot & tmask)) & ones(size));
                Ok(Flow::Next)
            }
            Op::Bfm {
                sf,
                rd,
                rn,
                immr,
                tmask_bits,
                wmask,
            } => {
                let size = datasize(sf);
                let src = self.x_or_zr(u32::from(rn)) & ones(size);
                let dst = self.x_or_zr(u32::from(rd)) & ones(size);
                let tmask = ones(u32::from(tmask_bits));
                let bot = (dst & !wmask) | (ror(src, u32::from(immr), size) & wmask);
                self.set_x_or_zr(u32::from(rd), ((dst & !tmask) | (bot & tmask)) & ones(size));
                Ok(Flow::Next)
            }
            Op::CondSel {
                sf,
                invert,
                increment,
                cond,
                rd,
                rn,
                rm,
            } => {
                let value = if self.condition_holds(u32::from(cond)) {
                    self.x_or_zr(u32::from(rn))
                } else {
                    let m = self.x_or_zr(u32::from(rm));
                    match (invert, increment) {
                        (false, false) => m,
                        (false, true) => m.wrapping_add(1),
                        (true, false) => !m,
                        (true, true) => (!m).wrapping_add(1),
                    }
                };
                self.set_x_or_zr_sized(u32::from(rd), value, sf);
                Ok(Flow::Next)
            }
            Op::Madd {
                sf,
                sub,
                rd,
                rn,
                rm,
                ra,
            } => {
                let product = self
                    .x_or_zr(u32::from(rn))
                    .wrapping_mul(self.x_or_zr(u32::from(rm)));
                let a = self.x_or_zr(u32::from(ra));
                let value = if sub {
                    a.wrapping_sub(product)
                } else {
                    a.wrapping_add(product)
                };
                self.set_x_or_zr_sized(u32::from(rd), value, sf);
                Ok(Flow::Next)
            }
            Op::BCond { cond, offset } => {
                if self.condition_holds(u32::from(cond)) {
                    Ok(Flow::Jump(pc.wrapping_add(offset)))
                } else {
                    Ok(Flow::Next)
                }
            }
            Op::Cbz {
                sf,
                nonzero,
                rt,
                offset,
            } => {
                let value = self.x_or_zr(u32::from(rt)) & ones(datasize(sf));
                if (value == 0) != nonzero {
                    Ok(Flow::Jump(pc.wrapping_add(offset)))
                } else {
                    Ok(Flow::Next)
                }
            }
            Op::Tbz {
                nonzero,
                rt,
                pos,
                offset,
            } => {
                let set = (self.x_or_zr(u32::from(rt)) >> pos) & 1 != 0;
                if set == nonzero {
                    Ok(Flow::Jump(pc.wrapping_add(offset)))
                } else {
                    Ok(Flow::Next)
                }
            }
            Op::B { offset } => Ok(Flow::Jump(pc.wrapping_add(offset))),
            Op::Bl { offset } => {
                self.x[30] = pc.wrapping_add(4);
                Ok(Flow::Jump(pc.wrapping_add(offset)))
            }
            Op::Br { rn } => {
                self.set_btype_jump(self.in_guarded_page, u32::from(rn));
                Ok(Flow::Jump(self.x_or_zr(u32::from(rn))))
            }
            Op::Ret { rn } => Ok(Flow::Jump(self.x_or_zr(u32::from(rn)))),
            Op::Blr { rn } => {
                let target = self.x_or_zr(u32::from(rn));
                self.x[30] = pc.wrapping_add(4);
                self.set_btype_call();
                Ok(Flow::Jump(target))
            }
            Op::Nop => Ok(Flow::Next),
            Op::LoadImm {
                size,
                extend,
                index,
                rt,
                rn,
                imm,
            } => {
                let base = completed!(self.base_register(u32::from(rn)));
                let (address, writeback) = indexed(base, imm, index);
                let width = AccessWidth::from_size_field(u32::from(size));
                let v = completed!(self.read_mem(bus, address, width, DataAccess::NORMAL));
                self.set_x_or_zr(u32::from(rt), extended(v, size, extend));
                if let Some(wb) = writeback {
                    self.set_x_or_sp(u32::from(rn), wb);
                }
                Ok(Flow::Next)
            }
            Op::StoreImm {
                size,
                index,
                rt,
                rn,
                imm,
            } => {
                let base = completed!(self.base_register(u32::from(rn)));
                let (address, writeback) = indexed(base, imm, index);
                let v = self.x_or_zr(u32::from(rt));
                let width = AccessWidth::from_size_field(u32::from(size));
                completed!(self.write_mem(bus, address, width, v, DataAccess::NORMAL));
                if let Some(wb) = writeback {
                    self.set_x_or_sp(u32::from(rn), wb);
                }
                Ok(Flow::Next)
            }
            Op::LoadReg {
                size,
                extend,
                option,
                shift,
                rt,
                rn,
                rm,
            } => {
                let offset = super::dp::extend_reg(
                    self.x_or_zr(u32::from(rm)),
                    u32::from(option),
                    u32::from(shift),
                    64,
                );
                let address = completed!(self.base_register(u32::from(rn))).wrapping_add(offset);
                let width = AccessWidth::from_size_field(u32::from(size));
                let v = completed!(self.read_mem(bus, address, width, DataAccess::NORMAL));
                self.set_x_or_zr(u32::from(rt), extended(v, size, extend));
                Ok(Flow::Next)
            }
            Op::StoreReg {
                size,
                option,
                shift,
                rt,
                rn,
                rm,
            } => {
                let offset = super::dp::extend_reg(
                    self.x_or_zr(u32::from(rm)),
                    u32::from(option),
                    u32::from(shift),
                    64,
                );
                let address = completed!(self.base_register(u32::from(rn))).wrapping_add(offset);
                let v = self.x_or_zr(u32::from(rt));
                let width = AccessWidth::from_size_field(u32::from(size));
                completed!(self.write_mem(bus, address, width, v, DataAccess::NORMAL));
                Ok(Flow::Next)
            }
            Op::LoadPair {
                size,
                index,
                rt,
                rt2,
                rn,
                imm,
            } => {
                let base = completed!(self.base_register(u32::from(rn)));
                let (address, writeback) = indexed(base, imm, index);
                let width = AccessWidth::from_size_field(u32::from(size));
                let ElementPair {
                    first: a,
                    second: b,
                } = completed!(self.read_pair(bus, address, width, DataAccess::NORMAL));
                // The order `exec_ldst_pair` writes them in, which decides
                // the CONSTRAINED UNPREDICTABLE Rt == Rt2 case.
                if width == AccessWidth::Word {
                    self.set_x_or_zr(u32::from(rt2), b);
                    self.set_x_or_zr(u32::from(rt), a);
                } else {
                    self.set_x_or_zr(u32::from(rt), a);
                    self.set_x_or_zr(u32::from(rt2), b);
                }
                if let Some(wb) = writeback {
                    self.set_x_or_sp(u32::from(rn), wb);
                }
                Ok(Flow::Next)
            }
            Op::StorePair {
                size,
                index,
                rt,
                rt2,
                rn,
                imm,
            } => {
                let base = completed!(self.base_register(u32::from(rn)));
                let (address, writeback) = indexed(base, imm, index);
                let width = AccessWidth::from_size_field(u32::from(size));
                let value = ElementPair {
                    first: self.x_or_zr(u32::from(rt)),
                    second: self.x_or_zr(u32::from(rt2)),
                };
                completed!(self.write_pair(bus, address, width, value, DataAccess::NORMAL));
                if let Some(wb) = writeback {
                    self.set_x_or_sp(u32::from(rn), wb);
                }
                Ok(Flow::Next)
            }
            Op::FpDp2 { op, ty, rd, rn, rm } => Ok(completed!(self.fp_dp2(
                op,
                ty,
                u32::from(rd),
                u32::from(rn),
                u32::from(rm)
            ))),
            Op::FpDp3 {
                form,
                ty,
                rd,
                rn,
                rm,
                ra,
            } => {
                let regs = FpDp3Regs {
                    d: u32::from(rd),
                    n: u32::from(rn),
                    m: u32::from(rm),
                    a: u32::from(ra),
                };
                Ok(completed!(self.fp_dp3(form, ty, regs)))
            }
        }
    }
}

/// The address an access with an immediate offset reaches, and the base's
/// new value if it is written back.
#[inline(always)]
fn indexed(base: u64, imm: u64, index: Index) -> (u64, Option<u64>) {
    match index {
        Index::Offset => (base.wrapping_add(imm), None),
        Index::Post => (base, Some(base.wrapping_add(imm))),
        Index::Pre => {
            let a = base.wrapping_add(imm);
            (a, Some(a))
        }
    }
}

/// A loaded value of `1 << size` bytes, extended into its register.
#[inline(always)]
fn extended(v: u64, size: u8, extend: Extend) -> u64 {
    match extend {
        Extend::Zero => v,
        Extend::Signed64 => sext(v, 8 << size),
        Extend::Signed32 => sext(v, 8 << size) & 0xFFFF_FFFF,
    }
}

#[cfg(test)]
#[path = "../../tests/unit/op.rs"]
mod tests;
