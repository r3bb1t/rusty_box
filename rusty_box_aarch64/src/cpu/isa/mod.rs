//! The A64 decoder: which encoding of the Arm architecture an instruction
//! word is, and what executes it here.
//!
//! The decoder is written against Arm's A64 encoding tree as the Arm
//! A-profile Architecture Machine Readable Specification (AARCHMRS) gives
//! it, `Instructions.json`, which is the A64 encoding index of the Arm ARM
//! (DDI 0487, chapter C4) in data form. The tree has three levels: the
//! top-level groups (`dpimm`, `control`, `ldst`, `dpreg`, `simd_dp`,
//! `reserved`, `sve`), each split into the encoding classes of C4
//! (`addsub_imm`, `asimdsame`, ...), each holding encodings
//! (`ADD_32_addsub_imm`, ...); `sve` and `sme` put a level of groups
//! (`sve_alloca`, ...) between the top and the classes. An encoding is the
//! bits Arm fixes for one instruction form, the should-be (`(0)`/`(1)`)
//! bits among them, so a word that sets a should-be bit otherwise is
//! UNDEFINED here, one of the CONSTRAINED UNPREDICTABLE behaviours the Arm
//! ARM permits for it.
//!
//! The code follows the tree. [`decode`] dispatches on the A64 root's
//! fields to one function per top-level group, which dispatches on that
//! group's fields to one function per group or class, named as Arm names
//! it; the class function matches its fields to the encoding. Every field
//! is read by `bits(word, hi, lo)` under Arm's field name, every arm of
//! every `match` is a literal, a range, an or-pattern or `_`, and each
//! class returns a variant of its own enum, named for Arm's encoding id
//! without the class's name and without `only` (Arm's mark for the one
//! encoding of an instruction in its class), in UpperCamelCase (the SVE
//! and SME ids hold no class name, and keep all of it):
//! `ADD_32_addsub_imm` is `AddsubImm::Add32`,
//! `ADD_asimdsame_only` is `Asimdsame::Add`, `LD1_asisdlse_R1_1v` is
//! `Asisdlse::Ld1R11v`, `rdsvl_r_i_` is `SveIntReadSvlA::RdsvlRI`. That
//! shape is what lets `tests/unit/spec.rs` read this source and check it
//! against Arm's data (see there): every encoding of a feature the decoder
//! knows ([`Feature`]) is here, under its own name and class, with exactly
//! Arm's fixed bits, and nothing else is — but for the encodings of a
//! feature still being built (`Feature::complete`), of which only some
//! are here yet.
//!
//! What is not here is UNDEFINED: unallocated encodings, and encodings of
//! the features the decoder does not know, every later architecture
//! extension among them. What is here executes only on a processor that
//! implements its feature (`decode::classify`). The one place this decoder
//! departs from the allocation Arm's data gives is PRFM (register): see
//! `ldst::ldst_regoff`.
//!
//! The decode stops at the encoding. Execution is by [`Class`]: the
//! handler for a class reads every operand field from the word itself,
//! following the field layout and decode pseudocode of the Arm ARM, and
//! raises UNDEFINED for the reserved values that pseudocode names.

mod asimd;
mod asisd;
mod atomic;
mod control;
mod crypto;
mod dpimm;
mod dpreg;
mod float;
mod ldst;
mod reserved;
mod simd_dp;
mod sve;
#[cfg(test)]
#[path = "../../../tests/unit/spec.rs"]
pub(crate) mod spec;

use crate::cpu::exec::bits;
pub(crate) use crate::cpu::features::Feature;

use asimd::{
    Asimdall, Asimddiff, Asimdelem, Asimdext, Asimdimm, Asimdins, Asimdmisc, Asimdmiscfp16, Asimdperm, Asimdsame,
    Asimdsame2, Asimdsamefp16, Asimdshf, Asimdtbl,
};
use asisd::{
    Asisddiff, Asisdelem, Asisdmisc, Asisdmiscfp16, Asisdone, Asisdpair, Asisdsame, Asisdsame2, Asisdsamefp16, Asisdshf,
};
use atomic::{Comswap, Comswappr, LdapstlUnscaled, Memop};
use control::{
    Barriers, BranchImm, BranchReg, Compbranch, Condbranch, Exception, Hints, Pstate, Systeminstrs, Systeminstrswithreg, Systemmove,
    Testbranch,
};
use crypto::{Cryptoaes, Cryptosha2, Cryptosha3};
use dpimm::{AddsubImm, Bitfield, Extract, LogImm, Movewide, Pcreladdr};
use dpreg::{AddsubCarry, AddsubExt, AddsubShift, CondcmpImm, CondcmpReg, Condsel, Dp1src, Dp2src, Dp3src, LogShift, Rmif, Setf};
use float::{Float2fix, Float2int, Floatccmp, Floatcmp, Floatdp1, Floatdp2, Floatdp3, Floatimm, Floatsel};
use ldst::{
    Asisdlse, Asisdlsep, Asisdlso, Asisdlsop, LdstImmpost, LdstImmpre, LdstPac, LdstPos, LdstRegoff, LdstUnpriv,
    LdstUnscaled, Ldstexclp, Ldstexclr, LdstnapairOffs, LdstpairOff, LdstpairPost, LdstpairPre, Ldstord, Loadlit,
};
use reserved::PermUndef;
use sve::{
    SveIntArithImm0, SveIntArithImm1, SveIntArithImm2, SveIntArithSvl, SveIntArithVl, SveIntBinConsArit0, SveIntBinConsLog,
    SveIntBinConsMisc0D, SveIntBinConsShiftA, SveIntBinConsShiftB, SveIntBinPredArit0, SveIntBinPredArit1,
    SveIntBinPredArit2, SveIntBinPredDiv, SveIntBinPredLog, SveIntBinPredShift0, SveIntBinPredShift1,
    SveIntBinPredShift2, SveIntBreak, SveIntBrkn, SveIntBrkp, SveIntCmp0, SveIntCmp1, SveIntCount, SveIntCountR,
    SveIntCountRSat, SveIntCountV, SveIntCountVSat, SveIntCountvlv0, SveIntCountvlv1, SveIntCterm, SveIntDupFpimm,
    SveIntDupFpimmPred, SveIntDupImm, SveIntDupImmPred, SveIntDupMaskImm, SveIntIndexIi, SveIntIndexIr, SveIntIndexRi,
    SveIntIndexRr, SveIntLogImm, SveIntMladdsubVvvPred, SveIntMlasVvvPred, SveIntMovprfxPred, SveIntMulB,
    SveIntPcountPred, SveIntPermBinPermPp, SveIntPermBinPermZz, SveIntPermClastRz, SveIntPermClastVz,
    SveIntPermClastZz, SveIntPermCpyR, SveIntPermCpyV, SveIntPermDupI, SveIntPermDupR, SveIntPermExtractI,
    SveIntPermInsrs, SveIntPermInsrv, SveIntPermLastR, SveIntPermLastV, SveIntPermPunpk, SveIntPermRev,
    SveIntPermRevd, SveIntPermReverseP, SveIntPermReverseZ, SveIntPermSplice, SveIntPermTbl, SveIntPermTbl3src,
    SveIntPermUnpk, SveIntPfalse, SveIntPfirst, SveIntPnext, SveIntPredDup, SveIntPredLog, SveIntPredPatternA,
    SveIntPredPatternB, SveIntPtest, SveIntPtrue, SveIntReadSvlA, SveIntReadVlA, SveIntReduce0, SveIntReduce1,
    SveIntReduce2, SveIntRotateImm, SveIntScmpVi, SveIntSelVvv, SveIntSqdmulh, SveIntTernLog, SveIntUcmpVi,
    SveIntUnPredArit0, SveIntUnPredArit1, SveIntWhileRr, SveIntWhilenc, SveIntxPermExtractI, SveIntxPermSplice,
    SveMem32bFill, SveMem32bPfill, SveMemCldSi, SveMemCldSs, SveMemCldntSi, SveMemCldntSs, SveMemCstSi, SveMemCstSs,
    SveMemCstntSi, SveMemCstntSs, SveMemEldSi, SveMemEldSs, SveMemEstSi, SveMemEstSs, SveMemLdDup, SveMemLdqrSi,
    SveMemLdqrSs, SveMemPrfmSi, SveMemPrfmSs, SveMemPspill, SveMemSpill,
};

/// An A64 encoding this processor implements, by its Arm class and Arm
/// encoding id (the module documentation gives the naming).
///
/// Closed (R5): [`Encoding::class`] answers for every variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Encoding {
    // reserved
    PermUndef(PermUndef),
    // dpimm
    Pcreladdr(Pcreladdr),
    AddsubImm(AddsubImm),
    LogImm(LogImm),
    Movewide(Movewide),
    Bitfield(Bitfield),
    Extract(Extract),
    // control
    Condbranch(Condbranch),
    Exception(Exception),
    Hints(Hints),
    Barriers(Barriers),
    Pstate(Pstate),
    Systeminstrswithreg(Systeminstrswithreg),
    Systeminstrs(Systeminstrs),
    Systemmove(Systemmove),
    BranchReg(BranchReg),
    BranchImm(BranchImm),
    Compbranch(Compbranch),
    Testbranch(Testbranch),
    // ldst
    Asisdlse(Asisdlse),
    Asisdlsep(Asisdlsep),
    Asisdlso(Asisdlso),
    Asisdlsop(Asisdlsop),
    Ldstexclp(Ldstexclp),
    Ldstexclr(Ldstexclr),
    Ldstord(Ldstord),
    Loadlit(Loadlit),
    LdstnapairOffs(LdstnapairOffs),
    LdstpairPost(LdstpairPost),
    LdstpairOff(LdstpairOff),
    LdstpairPre(LdstpairPre),
    LdstUnscaled(LdstUnscaled),
    LdstImmpost(LdstImmpost),
    LdstUnpriv(LdstUnpriv),
    LdstImmpre(LdstImmpre),
    LdstRegoff(LdstRegoff),
    LdstPos(LdstPos),
    LdstPac(LdstPac),
    Memop(Memop),
    Comswap(Comswap),
    Comswappr(Comswappr),
    LdapstlUnscaled(LdapstlUnscaled),
    // dpreg
    Dp2src(Dp2src),
    Dp1src(Dp1src),
    LogShift(LogShift),
    AddsubShift(AddsubShift),
    AddsubExt(AddsubExt),
    AddsubCarry(AddsubCarry),
    Rmif(Rmif),
    Setf(Setf),
    CondcmpReg(CondcmpReg),
    CondcmpImm(CondcmpImm),
    Condsel(Condsel),
    Dp3src(Dp3src),
    // simd_dp
    Cryptoaes(Cryptoaes),
    Cryptosha3(Cryptosha3),
    Cryptosha2(Cryptosha2),
    Asisdone(Asisdone),
    Asisdmisc(Asisdmisc),
    Asisdpair(Asisdpair),
    Asisddiff(Asisddiff),
    Asisdsame(Asisdsame),
    Asisdsame2(Asisdsame2),
    Asisdsamefp16(Asisdsamefp16),
    Asisdmiscfp16(Asisdmiscfp16),
    Asisdshf(Asisdshf),
    Asisdelem(Asisdelem),
    Asimdtbl(Asimdtbl),
    Asimdperm(Asimdperm),
    Asimdext(Asimdext),
    Asimdins(Asimdins),
    Asimdmisc(Asimdmisc),
    Asimdall(Asimdall),
    Asimddiff(Asimddiff),
    Asimdsame(Asimdsame),
    Asimdsame2(Asimdsame2),
    Asimdsamefp16(Asimdsamefp16),
    Asimdmiscfp16(Asimdmiscfp16),
    Asimdimm(Asimdimm),
    Asimdshf(Asimdshf),
    Asimdelem(Asimdelem),
    Float2fix(Float2fix),
    Float2int(Float2int),
    Floatdp1(Floatdp1),
    Floatcmp(Floatcmp),
    Floatimm(Floatimm),
    Floatccmp(Floatccmp),
    Floatdp2(Floatdp2),
    Floatsel(Floatsel),
    Floatdp3(Floatdp3),
    // sve
    SveIntBinPredArit0(SveIntBinPredArit0),
    SveIntBinPredArit1(SveIntBinPredArit1),
    SveIntBinPredArit2(SveIntBinPredArit2),
    SveIntBinPredDiv(SveIntBinPredDiv),
    SveIntBinPredLog(SveIntBinPredLog),
    SveIntReduce0(SveIntReduce0),
    SveIntReduce1(SveIntReduce1),
    SveIntMovprfxPred(SveIntMovprfxPred),
    SveIntReduce2(SveIntReduce2),
    SveIntBinPredShift0(SveIntBinPredShift0),
    SveIntBinPredShift1(SveIntBinPredShift1),
    SveIntBinPredShift2(SveIntBinPredShift2),
    SveIntUnPredArit0(SveIntUnPredArit0),
    SveIntUnPredArit1(SveIntUnPredArit1),
    SveIntMlasVvvPred(SveIntMlasVvvPred),
    SveIntMladdsubVvvPred(SveIntMladdsubVvvPred),
    SveIntBinConsArit0(SveIntBinConsArit0),
    SveIntBinConsLog(SveIntBinConsLog),
    SveIntRotateImm(SveIntRotateImm),
    SveIntTernLog(SveIntTernLog),
    SveIntBinConsShiftA(SveIntBinConsShiftA),
    SveIntBinConsShiftB(SveIntBinConsShiftB),
    SveIntBinConsMisc0D(SveIntBinConsMisc0D),
    SveIntIndexIi(SveIntIndexIi),
    SveIntIndexRi(SveIntIndexRi),
    SveIntIndexIr(SveIntIndexIr),
    SveIntIndexRr(SveIntIndexRr),
    SveIntArithVl(SveIntArithVl),
    SveIntArithSvl(SveIntArithSvl),
    SveIntReadVlA(SveIntReadVlA),
    SveIntReadSvlA(SveIntReadSvlA),
    SveIntMulB(SveIntMulB),
    SveIntSqdmulh(SveIntSqdmulh),
    SveIntCountvlv0(SveIntCountvlv0),
    SveIntCountvlv1(SveIntCountvlv1),
    SveIntCount(SveIntCount),
    SveIntPredPatternA(SveIntPredPatternA),
    SveIntPredPatternB(SveIntPredPatternB),
    SveIntLogImm(SveIntLogImm),
    SveIntDupMaskImm(SveIntDupMaskImm),
    SveIntDupImmPred(SveIntDupImmPred),
    SveIntDupFpimmPred(SveIntDupFpimmPred),
    SveIntPermExtractI(SveIntPermExtractI),
    SveIntxPermExtractI(SveIntxPermExtractI),
    SveIntPermDupI(SveIntPermDupI),
    SveIntPermTbl3src(SveIntPermTbl3src),
    SveIntPermTbl(SveIntPermTbl),
    SveIntPermDupR(SveIntPermDupR),
    SveIntPermInsrs(SveIntPermInsrs),
    SveIntPermUnpk(SveIntPermUnpk),
    SveIntPermInsrv(SveIntPermInsrv),
    SveIntPermReverseZ(SveIntPermReverseZ),
    SveIntPermBinPermPp(SveIntPermBinPermPp),
    SveIntPermPunpk(SveIntPermPunpk),
    SveIntPermReverseP(SveIntPermReverseP),
    SveIntPermBinPermZz(SveIntPermBinPermZz),
    SveIntPermCpyV(SveIntPermCpyV),
    SveIntPermLastV(SveIntPermLastV),
    SveIntPermRev(SveIntPermRev),
    SveIntPermClastZz(SveIntPermClastZz),
    SveIntPermClastVz(SveIntPermClastVz),
    SveIntPermSplice(SveIntPermSplice),
    SveIntxPermSplice(SveIntxPermSplice),
    SveIntPermLastR(SveIntPermLastR),
    SveIntPermCpyR(SveIntPermCpyR),
    SveIntPermClastRz(SveIntPermClastRz),
    SveIntPermRevd(SveIntPermRevd),
    SveIntSelVvv(SveIntSelVvv),
    SveIntCmp0(SveIntCmp0),
    SveIntCmp1(SveIntCmp1),
    SveIntUcmpVi(SveIntUcmpVi),
    SveIntScmpVi(SveIntScmpVi),
    SveIntPredLog(SveIntPredLog),
    SveIntBrkp(SveIntBrkp),
    SveIntBreak(SveIntBreak),
    SveIntBrkn(SveIntBrkn),
    SveIntPtest(SveIntPtest),
    SveIntPfirst(SveIntPfirst),
    SveIntPnext(SveIntPnext),
    SveIntPtrue(SveIntPtrue),
    SveIntPfalse(SveIntPfalse),
    SveIntPredDup(SveIntPredDup),
    SveIntWhileRr(SveIntWhileRr),
    SveIntCterm(SveIntCterm),
    SveIntWhilenc(SveIntWhilenc),
    SveIntPcountPred(SveIntPcountPred),
    SveIntCountVSat(SveIntCountVSat),
    SveIntCountV(SveIntCountV),
    SveIntCountRSat(SveIntCountRSat),
    SveIntCountR(SveIntCountR),
    SveIntArithImm0(SveIntArithImm0),
    SveIntArithImm1(SveIntArithImm1),
    SveIntArithImm2(SveIntArithImm2),
    SveIntDupImm(SveIntDupImm),
    SveIntDupFpimm(SveIntDupFpimm),
    SveMem32bPfill(SveMem32bPfill),
    SveMem32bFill(SveMem32bFill),
    SveMemPrfmSi(SveMemPrfmSi),
    SveMemPrfmSs(SveMemPrfmSs),
    SveMemLdDup(SveMemLdDup),
    SveMemLdqrSs(SveMemLdqrSs),
    SveMemLdqrSi(SveMemLdqrSi),
    SveMemCldSs(SveMemCldSs),
    SveMemCldSi(SveMemCldSi),
    SveMemCldntSs(SveMemCldntSs),
    SveMemEldSs(SveMemEldSs),
    SveMemCldntSi(SveMemCldntSi),
    SveMemEldSi(SveMemEldSi),
    SveMemPspill(SveMemPspill),
    SveMemCstSs(SveMemCstSs),
    SveMemSpill(SveMemSpill),
    SveMemCstntSs(SveMemCstntSs),
    SveMemEstSs(SveMemEstSs),
    SveMemCstSi(SveMemCstSi),
    SveMemCstntSi(SveMemCstntSi),
    SveMemEstSi(SveMemEstSi),
}

/// The encoding `word` is, if it is one of a feature the decoder knows.
/// Which of those a processor executes is its own feature set's choice
/// (`decode::classify`).
pub(crate) fn decode(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 31, 31);
    let op1 = bits(word, 28, 25);
    match (op0, op1) {
        (0b0, 0b0000) => reserved::reserved(word),
        // op0 = 1, op1 = 0000 is sme (FEAT_SME); op1 = 0001 and 0011 are
        // unallocated.
        (_, 0b0010) => sve::sve(word),
        (_, 0b1000 | 0b1001) => dpimm::dpimm(word),
        (_, 0b1010 | 0b1011) => control::control(word),
        (_, 0b0100 | 0b0110 | 0b1100 | 0b1110) => ldst::ldst(word),
        (_, 0b0101 | 0b1101) => dpreg::dpreg(word),
        (_, 0b0111 | 0b1111) => simd_dp::simd_dp(word),
        _ => None,
    }
}

impl Encoding {
    /// The handler this encoding executes through.
    pub(crate) const fn class(self) -> Class {
        match self {
            Self::PermUndef(_) => Class::Undefined,
            Self::Pcreladdr(_) => Class::PcRelAddressing,
            Self::AddsubImm(_) => Class::AddSubImmediate,
            Self::LogImm(_) => Class::LogicalImmediate,
            Self::Movewide(_) => Class::MoveWide,
            Self::Bitfield(_) => Class::Bitfield,
            Self::Extract(_) => Class::Extract,
            Self::Condbranch(_) => Class::ConditionalBranch,
            Self::Exception(_) => Class::ExceptionGeneration,
            Self::Hints(e) => e.class(),
            Self::Pstate(e) => e.class(),
            Self::Systeminstrswithreg(_) => Class::WaitWithTimeout,
            Self::Barriers(_) | Self::Systeminstrs(_) | Self::Systemmove(_) => Class::System,
            Self::BranchReg(e) => e.class(),
            Self::BranchImm(_) => Class::BranchImmediate,
            Self::Compbranch(_) => Class::CompareAndBranch,
            Self::Testbranch(_) => Class::TestAndBranch,
            Self::Asisdlse(_) => Class::SimdLoadStoreMultiple,
            Self::Asisdlsep(_) => Class::SimdLoadStoreMultiplePostIndexed,
            Self::Asisdlso(_) => Class::SimdLoadStoreSingle,
            Self::Asisdlsop(_) => Class::SimdLoadStoreSinglePostIndexed,
            Self::Ldstexclp(_) | Self::Ldstexclr(_) | Self::Ldstord(_) => Class::LoadStoreExclusive,
            Self::Loadlit(_) => Class::LoadLiteral,
            Self::LdstnapairOffs(_) => Class::LoadStoreNoAllocatePair,
            Self::LdstpairPost(_) | Self::LdstpairPre(_) => Class::LoadStorePairIndexed,
            Self::LdstpairOff(_) => Class::LoadStorePairOffset,
            Self::LdstUnscaled(_) => Class::LoadStoreUnscaled,
            Self::LdstImmpost(_) | Self::LdstImmpre(_) => Class::LoadStoreIndexed,
            Self::LdstUnpriv(_) => Class::LoadStoreUnprivileged,
            Self::LdstRegoff(_) => Class::LoadStoreRegisterOffset,
            Self::LdstPos(_) => Class::LoadStoreUnsignedImmediate,
            Self::LdstPac(_) => Class::LoadAuthenticated,
            Self::Memop(_) => Class::AtomicMemoryOperation,
            Self::Comswap(_) => Class::CompareAndSwap,
            Self::Comswappr(_) => Class::CompareAndSwapPair,
            Self::LdapstlUnscaled(_) => Class::LoadStoreOrderedUnscaled,
            Self::Dp2src(e) => e.class(),
            Self::Dp1src(e) => e.class(),
            Self::LogShift(_) => Class::LogicalShiftedRegister,
            Self::AddsubShift(_) => Class::AddSubShiftedRegister,
            Self::AddsubExt(_) => Class::AddSubExtendedRegister,
            Self::AddsubCarry(_) => Class::AddSubWithCarry,
            Self::Rmif(_) => Class::RotateMaskInsertFlags,
            Self::Setf(_) => Class::EvaluateIntoFlags,
            Self::CondcmpReg(_) => Class::ConditionalCompareRegister,
            Self::CondcmpImm(_) => Class::ConditionalCompareImmediate,
            Self::Condsel(_) => Class::ConditionalSelect,
            Self::Dp3src(_) => Class::DataProcessing3Source,
            Self::Cryptoaes(_) => Class::CryptoAes,
            Self::Cryptosha3(_) => Class::CryptoShaThreeRegister,
            Self::Cryptosha2(_) => Class::CryptoShaTwoRegister,
            Self::Asisdone(_) => Class::SimdScalarCopy,
            Self::Asisdmisc(_) => Class::SimdScalarTwoRegisterMisc,
            Self::Asisdpair(_) => Class::SimdScalarPairwise,
            Self::Asisddiff(_) => Class::SimdScalarThreeDifferent,
            Self::Asisdsame(_) => Class::SimdScalarThreeSame,
            Self::Asisdsame2(_) => Class::SimdScalarRoundingDoublingMultiplyAccumulate,
            Self::Asisdsamefp16(_) => Class::SimdScalarThreeSameHalf,
            Self::Asisdmiscfp16(_) => Class::SimdScalarTwoRegisterMiscHalf,
            Self::Asisdshf(_) => Class::SimdScalarShiftImmediate,
            Self::Asisdelem(_) => Class::SimdScalarIndexedElement,
            Self::Asimdtbl(_) => Class::SimdTableLookup,
            Self::Asimdperm(_) => Class::SimdPermute,
            Self::Asimdext(_) => Class::SimdExtract,
            Self::Asimdins(_) => Class::SimdCopy,
            Self::Asimdmisc(_) => Class::SimdTwoRegisterMisc,
            Self::Asimdall(_) => Class::SimdAcrossLanes,
            Self::Asimddiff(_) => Class::SimdThreeDifferent,
            Self::Asimdsame(e) => e.class(),
            Self::Asimdsame2(e) => e.class(),
            Self::Asimdsamefp16(_) => Class::SimdThreeSameHalf,
            Self::Asimdmiscfp16(_) => Class::SimdTwoRegisterMiscHalf,
            Self::Asimdimm(_) => Class::SimdModifiedImmediate,
            Self::Asimdshf(_) => Class::SimdShiftImmediate,
            Self::Asimdelem(e) => e.class(),
            Self::Float2fix(_) => Class::FpFixedPointConversion,
            Self::Float2int(_) => Class::FpIntegerConversion,
            Self::Floatdp1(_) => Class::FpDataProcessing1Source,
            Self::Floatcmp(_) => Class::FpCompare,
            Self::Floatimm(_) => Class::FpImmediate,
            Self::Floatccmp(_) => Class::FpConditionalCompare,
            Self::Floatdp2(_) => Class::FpDataProcessing2Source,
            Self::Floatsel(_) => Class::FpConditionalSelect,
            Self::Floatdp3(_) => Class::FpDataProcessing3Source,
            Self::SveIntArithSvl(_) => Class::AddStreamingVectorLength,
            Self::SveIntReadSvlA(_) => Class::ReadStreamingVectorLength,
            Self::SveIntBinPredArit0(_)
            | Self::SveIntBinPredArit1(_)
            | Self::SveIntBinPredArit2(_)
            | Self::SveIntBinPredDiv(_)
            | Self::SveIntBinPredLog(_) => Class::SveIntBinaryPredicated,
            Self::SveIntReduce0(_) | Self::SveIntReduce1(_) | Self::SveIntReduce2(_) => Class::SveIntReduction,
            Self::SveIntMovprfxPred(_) | Self::SveIntBinConsMisc0D(_) => Class::SveMovprfx,
            Self::SveIntBinPredShift0(_) | Self::SveIntBinPredShift1(_) | Self::SveIntBinPredShift2(_) => {
                Class::SveShiftPredicated
            }
            Self::SveIntUnPredArit0(_) | Self::SveIntUnPredArit1(_) => Class::SveIntUnaryPredicated,
            Self::SveIntMlasVvvPred(_) | Self::SveIntMladdsubVvvPred(_) => Class::SveIntMulAddPredicated,
            Self::SveIntBinConsArit0(_) => Class::SveIntAddSubUnpredicated,
            Self::SveIntBinConsLog(_) | Self::SveIntRotateImm(_) | Self::SveIntTernLog(_) => Class::SveBitwiseUnpredicated,
            Self::SveIntBinConsShiftA(_) | Self::SveIntBinConsShiftB(_) => Class::SveShiftUnpredicated,
            Self::SveIntIndexIi(_) | Self::SveIntIndexRi(_) | Self::SveIntIndexIr(_) | Self::SveIntIndexRr(_) => {
                Class::SveIndex
            }
            Self::SveIntArithVl(_) => Class::AddVectorLength,
            Self::SveIntReadVlA(_) => Class::ReadVectorLength,
            Self::SveIntMulB(_) | Self::SveIntSqdmulh(_) => Class::SveIntMulUnpredicated,
            Self::SveIntCountvlv0(_)
            | Self::SveIntCountvlv1(_)
            | Self::SveIntCount(_)
            | Self::SveIntPredPatternA(_)
            | Self::SveIntPredPatternB(_) => Class::SveElementCount,
            Self::SveIntLogImm(_) | Self::SveIntDupMaskImm(_) => Class::SveBitmaskImmediate,
            Self::SveIntDupImmPred(_) | Self::SveIntDupFpimmPred(_) => Class::SveCopyImmediate,
            Self::SveIntPermExtractI(_) | Self::SveIntxPermExtractI(_) => Class::SveExtract,
            Self::SveIntPermDupI(_)
            | Self::SveIntPermTbl3src(_)
            | Self::SveIntPermTbl(_)
            | Self::SveIntPermDupR(_)
            | Self::SveIntPermInsrs(_)
            | Self::SveIntPermUnpk(_)
            | Self::SveIntPermInsrv(_)
            | Self::SveIntPermReverseZ(_) => Class::SvePermuteUnpredicated,
            Self::SveIntPermBinPermPp(_) | Self::SveIntPermPunpk(_) | Self::SveIntPermReverseP(_) => {
                Class::SvePermutePredicate
            }
            Self::SveIntPermBinPermZz(_) => Class::SvePermuteElements,
            Self::SveIntPermCpyV(_)
            | Self::SveIntPermLastV(_)
            | Self::SveIntPermRev(_)
            | Self::SveIntPermClastZz(_)
            | Self::SveIntPermClastVz(_)
            | Self::SveIntPermSplice(_)
            | Self::SveIntxPermSplice(_)
            | Self::SveIntPermLastR(_)
            | Self::SveIntPermCpyR(_)
            | Self::SveIntPermClastRz(_)
            | Self::SveIntPermRevd(_) => Class::SvePermutePredicated,
            Self::SveIntSelVvv(_) => Class::SveSelect,
            Self::SveIntCmp0(_) | Self::SveIntCmp1(_) | Self::SveIntUcmpVi(_) | Self::SveIntScmpVi(_) => {
                Class::SveIntCompare
            }
            Self::SveIntPredLog(_) => Class::SvePredicateLogical,
            Self::SveIntBrkp(_) | Self::SveIntBreak(_) | Self::SveIntBrkn(_) => Class::SveBreak,
            Self::SveIntPtest(_)
            | Self::SveIntPfirst(_)
            | Self::SveIntPnext(_)
            | Self::SveIntPtrue(_)
            | Self::SveIntPfalse(_)
            | Self::SveIntPredDup(_) => Class::SvePredicateMisc,
            Self::SveIntWhileRr(_) | Self::SveIntWhilenc(_) => Class::SveWhile,
            Self::SveIntCterm(_) => Class::SveConditionallyTerminate,
            Self::SveIntPcountPred(_)
            | Self::SveIntCountVSat(_)
            | Self::SveIntCountV(_)
            | Self::SveIntCountRSat(_)
            | Self::SveIntCountR(_) => Class::SvePredicateCount,
            Self::SveIntArithImm0(_)
            | Self::SveIntArithImm1(_)
            | Self::SveIntArithImm2(_)
            | Self::SveIntDupImm(_)
            | Self::SveIntDupFpimm(_) => Class::SveWideImmediate,
            Self::SveMem32bPfill(_) | Self::SveMem32bFill(_) | Self::SveMemPspill(_) | Self::SveMemSpill(_) => {
                Class::SveFillSpill
            }
            Self::SveMemPrfmSi(_) | Self::SveMemPrfmSs(_) => Class::SvePrefetch,
            Self::SveMemLdDup(_) => Class::SveLoadBroadcast,
            Self::SveMemLdqrSs(_) | Self::SveMemLdqrSi(_) => Class::SveLoadBroadcastQuadword,
            Self::SveMemCldSs(_) | Self::SveMemCldSi(_) => Class::SveLoadContiguous,
            Self::SveMemCldntSs(_) | Self::SveMemEldSs(_) | Self::SveMemCldntSi(_) | Self::SveMemEldSi(_) => {
                Class::SveLoadStructures
            }
            Self::SveMemCstSs(_) | Self::SveMemCstSi(_) => Class::SveStoreContiguous,
            Self::SveMemCstntSs(_) | Self::SveMemEstSs(_) | Self::SveMemCstntSi(_) | Self::SveMemEstSi(_) => {
                Class::SveStoreStructures
            }
        }
    }
}

/// What executes a word: the unit of dispatch, one handler each.
///
/// One flat enum, a byte, so that dispatch on a cached class is a single
/// jump table. Closed (R5): dispatch matches every variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Class {
    /// The Undefined Instruction exception: UDF, which is permanently
    /// undefined, and every word [`decode`] finds no encoding for.
    Undefined,
    PcRelAddressing,
    AddSubImmediate,
    LogicalImmediate,
    MoveWide,
    Bitfield,
    Extract,
    ConditionalBranch,
    ExceptionGeneration,
    /// The hint space: every hint executes, as a NOP when it is not one
    /// this core implements, including the pointer authentication and BTI
    /// hints of a processor without FEAT_PAuth or FEAT_BTI, which is what
    /// lets code built with branch protection run on an ARMv8.0 core.
    Hint,
    /// PACIASP, AUTIASP and the other pointer authentication hints
    /// (FEAT_PAuth): the handler reads which from `CRm` and `op2`.
    PointerAuthHint,
    /// WFET and WFIT (FEAT_WFxT): the handler reads which from `op2`.
    WaitWithTimeout,
    /// Barriers, MSR (immediate), the system instructions and the system
    /// register moves.
    System,
    /// CFINV (FEAT_FlagM), XAFLAG and AXFLAG (FEAT_FlagM2): the handler
    /// reads which from `op2`.
    FlagManipulation,
    BranchRegister,
    /// BRAA, BRAAZ, BLRAA, BLRAAZ, RETAA, ERETAA and their key B forms
    /// (FEAT_PAuth): the handler reads which from `opc`, `op3` and `op4`.
    AuthenticatedBranch,
    BranchImmediate,
    CompareAndBranch,
    TestAndBranch,
    SimdLoadStoreMultiple,
    SimdLoadStoreMultiplePostIndexed,
    SimdLoadStoreSingle,
    SimdLoadStoreSinglePostIndexed,
    /// The exclusive, exclusive pair and ordered loads and stores.
    LoadStoreExclusive,
    LoadLiteral,
    LoadStoreNoAllocatePair,
    LoadStorePairIndexed,
    LoadStorePairOffset,
    LoadStoreUnscaled,
    LoadStoreIndexed,
    LoadStoreUnprivileged,
    LoadStoreRegisterOffset,
    LoadStoreUnsignedImmediate,
    /// LDRAA, LDRAB (FEAT_PAuth): the handler reads which from `M` and
    /// `W`.
    LoadAuthenticated,
    DataProcessing2Source,
    DataProcessing1Source,
    /// PACIA, AUTDZB, XPACI and the other one-source pointer
    /// authentication instructions (FEAT_PAuth): the handler reads which
    /// from `opcode`.
    PointerAuth,
    /// PACGA (FEAT_PAuth).
    PointerAuthGeneric,
    LogicalShiftedRegister,
    AddSubShiftedRegister,
    AddSubExtendedRegister,
    AddSubWithCarry,
    /// RMIF (FEAT_FlagM).
    RotateMaskInsertFlags,
    /// SETF8, SETF16 (FEAT_FlagM): the handler reads which from `sz`.
    EvaluateIntoFlags,
    ConditionalCompareRegister,
    ConditionalCompareImmediate,
    ConditionalSelect,
    DataProcessing3Source,
    SimdScalarCopy,
    SimdScalarThreeSame,
    SimdScalarTwoRegisterMisc,
    SimdScalarPairwise,
    SimdScalarThreeDifferent,
    SimdScalarShiftImmediate,
    SimdScalarIndexedElement,
    SimdTableLookup,
    SimdPermute,
    SimdExtract,
    SimdCopy,
    SimdThreeSame,
    SimdTwoRegisterMisc,
    SimdAcrossLanes,
    SimdThreeDifferent,
    SimdModifiedImmediate,
    SimdShiftImmediate,
    SimdIndexedElement,
    /// SQRDMLAH, SQRDMLSH (vector): the handler reads which from `S`.
    SimdRoundingDoublingMultiplyAccumulate,
    /// SQRDMLAH, SQRDMLSH (scalar).
    SimdScalarRoundingDoublingMultiplyAccumulate,
    /// SDOT, UDOT, USDOT (vector): the handler reads which from `U` and
    /// `opcode<0>`.
    SimdDotProduct,
    /// SDOT, UDOT, SUDOT, USDOT (by element): the handler reads which from
    /// `U`, `opcode<0>` and `US`.
    SimdDotProductIndexedElement,
    /// SMMLA, USMMLA, UMMLA: the handler reads which from `U` and `B`.
    SimdMatrixMultiply,
    /// The three-same floating point of half precision (FEAT_FP16), vector
    /// and scalar: the handler reads which from `U`, `a` and `opcode`, as
    /// for single and double precision.
    SimdThreeSameHalf,
    SimdScalarThreeSameHalf,
    /// The two-register miscellaneous floating point of half precision
    /// (FEAT_FP16), vector and scalar.
    SimdTwoRegisterMiscHalf,
    SimdScalarTwoRegisterMiscHalf,
    /// FMLAL, FMLSL, FMLAL2, FMLSL2 (FEAT_FHM), vector and by element.
    SimdFpMulAddLong,
    SimdFpMulAddLongIndexedElement,
    /// FCMLA (FEAT_FCMA), vector and by element: the handler reads the
    /// rotation from `rot`.
    SimdComplexMultiplyAccumulate,
    SimdComplexMultiplyAccumulateIndexedElement,
    /// FCADD (FEAT_FCMA).
    SimdComplexAdd,
    /// BFDOT (FEAT_BF16), vector and by element.
    SimdBfDotProduct,
    SimdBfDotProductIndexedElement,
    /// BFMLALB, BFMLALT (FEAT_BF16), vector and by element: the handler
    /// reads which from `Q`.
    SimdBfMulAddLong,
    SimdBfMulAddLongIndexedElement,
    /// BFMMLA (FEAT_BF16).
    SimdBfMatrixMultiply,
    FpFixedPointConversion,
    FpIntegerConversion,
    FpDataProcessing1Source,
    FpCompare,
    FpImmediate,
    FpConditionalCompare,
    FpDataProcessing2Source,
    FpConditionalSelect,
    FpDataProcessing3Source,
    /// AESE, AESD, AESMC, AESIMC.
    CryptoAes,
    /// SHA1C, SHA1P, SHA1M, SHA1SU0, SHA256H, SHA256H2, SHA256SU1.
    CryptoShaThreeRegister,
    /// SHA1H, SHA1SU1, SHA256SU0.
    CryptoShaTwoRegister,
    /// CRC32B/H/W/X and CRC32CB/CH/CW/CX: the handler reads the size and
    /// the polynomial from `sz` and `C`.
    Crc32,
    /// LDADD, LDCLR, LDEOR, LDSET, LD{S,U}{MAX,MIN} and SWP, every size
    /// and ordering, and LDAPR: the handler reads which from `o3` and
    /// `opc`, which tell them apart once the decoder has fixed LDAPR's `A`,
    /// `R` and `Rs`. With one processor executing one instruction at a
    /// time, `A` and `R` order nothing.
    AtomicMemoryOperation,
    /// CAS of every size and ordering.
    CompareAndSwap,
    /// CASP, 32- and 64-bit pairs.
    CompareAndSwapPair,
    /// LDAPUR*, STLUR*: the handler reads which from `size` and `opc`.
    LoadStoreOrderedUnscaled,
    /// ADDSVL, ADDSPL: the handler reads which from `op`.
    AddStreamingVectorLength,
    /// RDSVL.
    ReadStreamingVectorLength,
    /// ADDVL, ADDPL: the handler reads which from `op`.
    AddVectorLength,
    /// RDVL.
    ReadVectorLength,
    // The SVE classes: each handler reads which instruction from the
    // fields its Arm group tells its classes apart by.
    /// `sve_int_pred_bin`: ADD, SUB, SUBR, the min/max/difference, multiply
    /// and divide forms, and the bitwise operations, predicated.
    SveIntBinaryPredicated,
    /// `sve_int_reduce_0`, `_1`, `_2`: SADDV, UADDV, SMAXV, SMINV, UMAXV,
    /// UMINV, ORV, EORV, ANDV.
    SveIntReduction,
    /// MOVPRFX, predicated and unpredicated.
    SveMovprfx,
    /// `sve_int_pred_shift`: shifts by immediate, by vector and by wide
    /// elements, predicated, ASRD and the SVE2 rounding and saturating
    /// shifts.
    SveShiftPredicated,
    /// `sve_int_pred_un`: the integer and bitwise unary operations,
    /// predicated.
    SveIntUnaryPredicated,
    /// MLA, MLS, MAD, MSB.
    SveIntMulAddPredicated,
    /// ADD, SUB and the saturating forms, unpredicated.
    SveIntAddSubUnpredicated,
    /// AND, ORR, EOR, BIC (vectors), XAR, EOR3, BCAX and the bitwise
    /// selects.
    SveBitwiseUnpredicated,
    /// Shifts by immediate and by wide elements, unpredicated.
    SveShiftUnpredicated,
    /// INDEX, its four forms.
    SveIndex,
    /// MUL, PMUL, SMULH, UMULH, SQDMULH, SQRDMULH (vectors, unpredicated).
    SveIntMulUnpredicated,
    /// `sve_countelt`: CNTx, INCx and DECx and their saturating forms.
    SveElementCount,
    /// AND, ORR, EOR (immediate) and DUPM.
    SveBitmaskImmediate,
    /// CPY (immediate) and FCPY.
    SveCopyImmediate,
    /// EXT, destructive and constructive.
    SveExtract,
    /// DUP (indexed), TBL, TBX, DUP (scalar), INSR, the unpacks and REV
    /// (vector).
    SvePermuteUnpredicated,
    /// ZIP, UZP, TRN, PUNPK and REV of predicates.
    SvePermutePredicate,
    /// ZIP, UZP, TRN of vectors.
    SvePermuteElements,
    /// `sve_perm_pred`: CPY (scalar), LASTA, LASTB, CLASTA, CLASTB, REVB,
    /// REVH, REVW, RBIT, REVD and SPLICE.
    SvePermutePredicated,
    /// SEL (vectors).
    SveSelect,
    /// CMP<cc>, by vector, wide element and immediate.
    SveIntCompare,
    /// AND, BIC, ORR, ORN, EOR, NOR, NAND, SEL of predicates, and the
    /// flag-setting forms.
    SvePredicateLogical,
    /// BRKA, BRKB, BRKN, BRKPA, BRKPB and their flag-setting forms.
    SveBreak,
    /// PTEST, PFIRST, PNEXT, PTRUE, PTRUES, PFALSE, PSEL.
    SvePredicateMisc,
    /// WHILE<cc>, WHILEWR, WHILERW.
    SveWhile,
    /// CTERMEQ, CTERMNE.
    SveConditionallyTerminate,
    /// CNTP, INCP, DECP and their saturating forms.
    SvePredicateCount,
    /// `sve_wideimm_unpred`: the add/subtract, min/max and multiply
    /// immediates, DUP (immediate), FDUP.
    SveWideImmediate,
    /// LDR and STR of a vector or a predicate register: the handler reads
    /// the direction from bit 30 and the register file from bit 14.
    SveFillSpill,
    /// PRFB, PRFH, PRFW, PRFD, scalar plus immediate and scalar plus
    /// scalar.
    SvePrefetch,
    /// LD1RB, LD1RH, LD1RW, LD1RD and the sign-extending LD1RSB, LD1RSH,
    /// LD1RSW.
    SveLoadBroadcast,
    /// LD1RQB, LD1RQH, LD1RQW, LD1RQD.
    SveLoadBroadcastQuadword,
    /// `sve_mem_cld_ss`, `_cld_si`: LD1B, LD1H, LD1W, LD1D and the
    /// sign-extending LD1SB, LD1SH, LD1SW, by `dtype`.
    SveLoadContiguous,
    /// LDNT1 and LD2, LD3, LD4 of bytes to doublewords: the handler reads
    /// the registers from `opc`, which is 00 for LDNT1.
    SveLoadStructures,
    /// ST1B, ST1H, ST1W, ST1D.
    SveStoreContiguous,
    /// STNT1 and ST2, ST3, ST4, as [`Class::SveLoadStructures`].
    SveStoreStructures,
}

/// What an encoding's decode does about the branch that led to it into a
/// guarded page (FEAT_BTI): which `BTypeCompatible` it sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BranchTarget {
    /// BTI accepts the branches `op2<2:1>` names (`BTypeCompatible_BTI`).
    Bti,
    /// PACIASP and PACIBSP accept a branch unless SCTLR_EL1's BT0 or BT1
    /// says otherwise (`BTypeCompatible_PAC`).
    PacSp,
    /// BRK accepts any branch.
    Any,
    /// Every other encoding's decode leaves `BTypeCompatible` FALSE.
    Refused,
}

impl Encoding {
    /// The handler of this encoding on a processor without its feature: a
    /// hint executes as a NOP (its decode's `EndOfDecode(Decode_NOP)`),
    /// any other encoding is UNDEFINED (`EndOfDecode(Decode_UNDEF)`).
    pub(crate) const fn class_without_feature(self) -> Class {
        if matches!(self, Self::Hints(_)) {
            Class::Hint
        } else {
            Class::Undefined
        }
    }

    /// What this encoding does about a branch into a guarded page, on a
    /// processor implementing its feature.
    pub(crate) const fn branch_target(self) -> BranchTarget {
        match self {
            Self::Hints(Hints::BtiHb) => BranchTarget::Bti,
            Self::Hints(Hints::PaciaspHi | Hints::PacibspHi) => BranchTarget::PacSp,
            Self::Exception(Exception::BrkEx) => BranchTarget::Any,
            _ => BranchTarget::Refused,
        }
    }

    /// The feature this encoding belongs to.
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::PermUndef(_)
            | Self::Pcreladdr(_)
            | Self::AddsubImm(_)
            | Self::LogImm(_)
            | Self::Movewide(_)
            | Self::Bitfield(_)
            | Self::Extract(_)
            | Self::Condbranch(_)
            | Self::Exception(_)
            | Self::Systeminstrs(_)
            | Self::Systemmove(_)
            | Self::BranchImm(_)
            | Self::Compbranch(_)
            | Self::Testbranch(_)
            | Self::Ldstexclp(_)
            | Self::Ldstexclr(_)
            | Self::LdstUnpriv(_)
            | Self::LogShift(_)
            | Self::AddsubShift(_)
            | Self::AddsubExt(_)
            | Self::AddsubCarry(_)
            | Self::CondcmpReg(_)
            | Self::CondcmpImm(_)
            | Self::Condsel(_)
            | Self::Dp3src(_) => Feature::Base,
            Self::Rmif(_) | Self::Setf(_) => Feature::FlagM,
            Self::Barriers(e) => e.feature(),
            Self::Ldstord(e) => e.feature(),
            Self::Pstate(e) => e.feature(),
            Self::Systeminstrswithreg(_) => Feature::WfxT,
            Self::Hints(e) => e.feature(),
            Self::BranchReg(e) => e.feature(),
            Self::Dp1src(e) => e.feature(),
            Self::Dp2src(e) => e.feature(),
            Self::Cryptoaes(_) => Feature::Aes,
            Self::Cryptosha3(e) => e.feature(),
            Self::Cryptosha2(e) => e.feature(),
            Self::Loadlit(e) => e.feature(),
            Self::LdstnapairOffs(e) => e.feature(),
            Self::LdstpairPost(e) => e.feature(),
            Self::LdstpairOff(e) => e.feature(),
            Self::LdstpairPre(e) => e.feature(),
            Self::LdstUnscaled(e) => e.feature(),
            Self::LdstImmpost(e) => e.feature(),
            Self::LdstImmpre(e) => e.feature(),
            Self::LdstRegoff(e) => e.feature(),
            Self::LdstPos(e) => e.feature(),
            Self::LdstPac(e) => e.feature(),
            Self::Memop(e) => e.feature(),
            Self::Comswap(e) => e.feature(),
            Self::Comswappr(e) => e.feature(),
            Self::LdapstlUnscaled(e) => e.feature(),
            Self::Asisdlse(_)
            | Self::Asisdlsep(_)
            | Self::Asisdlso(_)
            | Self::Asisdlsop(_)
            | Self::Asisdone(_)
            | Self::Asisdmisc(_)
            | Self::Asisddiff(_)
            | Self::Asisdsame(_)
            | Self::Asisdshf(_)
            | Self::Asimdtbl(_)
            | Self::Asimdperm(_)
            | Self::Asimdext(_)
            | Self::Asimdins(_)
            | Self::Asimddiff(_)
            | Self::Asimdshf(_) => Feature::AdvSimd,
            Self::Asimdmisc(e) => e.feature(),
            Self::Asisdpair(e) => e.feature(),
            Self::Asimdall(e) => e.feature(),
            Self::Asimdsame(e) => e.feature(),
            Self::Asimdimm(e) => e.feature(),
            Self::Asisdsamefp16(e) => e.feature(),
            Self::Asisdmiscfp16(e) => e.feature(),
            Self::Asimdsamefp16(e) => e.feature(),
            Self::Asimdmiscfp16(e) => e.feature(),
            Self::Asisdsame2(e) => e.feature(),
            Self::Asisdelem(e) => e.feature(),
            Self::Asimdsame2(e) => e.feature(),
            Self::Asimdelem(e) => e.feature(),
            Self::Float2fix(e) => e.feature(),
            Self::Float2int(e) => e.feature(),
            Self::Floatdp1(e) => e.feature(),
            Self::Floatcmp(e) => e.feature(),
            Self::Floatimm(e) => e.feature(),
            Self::Floatccmp(e) => e.feature(),
            Self::Floatdp2(e) => e.feature(),
            Self::Floatsel(e) => e.feature(),
            Self::Floatdp3(e) => e.feature(),
            // FEAT_SVE or FEAT_SME (or FEAT_SVE2 or FEAT_SME): of the two,
            // the decoder knows FEAT_SME alone.
            Self::SveIntBinPredArit0(_)
            | Self::SveIntBinPredArit1(_)
            | Self::SveIntBinPredArit2(_)
            | Self::SveIntBinPredDiv(_)
            | Self::SveIntBinPredLog(_)
            | Self::SveIntReduce0(_)
            | Self::SveIntReduce1(_)
            | Self::SveIntMovprfxPred(_)
            | Self::SveIntReduce2(_)
            | Self::SveIntBinPredShift0(_)
            | Self::SveIntBinPredShift1(_)
            | Self::SveIntBinPredShift2(_)
            | Self::SveIntUnPredArit0(_)
            | Self::SveIntUnPredArit1(_)
            | Self::SveIntMlasVvvPred(_)
            | Self::SveIntMladdsubVvvPred(_)
            | Self::SveIntBinConsArit0(_)
            | Self::SveIntBinConsLog(_)
            | Self::SveIntRotateImm(_)
            | Self::SveIntTernLog(_)
            | Self::SveIntBinConsShiftA(_)
            | Self::SveIntBinConsShiftB(_)
            | Self::SveIntBinConsMisc0D(_)
            | Self::SveIntIndexIi(_)
            | Self::SveIntIndexRi(_)
            | Self::SveIntIndexIr(_)
            | Self::SveIntIndexRr(_)
            | Self::SveIntArithVl(_)
            | Self::SveIntArithSvl(_)
            | Self::SveIntReadVlA(_)
            | Self::SveIntReadSvlA(_)
            | Self::SveIntMulB(_)
            | Self::SveIntSqdmulh(_)
            | Self::SveIntCountvlv0(_)
            | Self::SveIntCountvlv1(_)
            | Self::SveIntCount(_)
            | Self::SveIntPredPatternA(_)
            | Self::SveIntPredPatternB(_)
            | Self::SveIntLogImm(_)
            | Self::SveIntDupMaskImm(_)
            | Self::SveIntDupImmPred(_)
            | Self::SveIntDupFpimmPred(_)
            | Self::SveIntPermExtractI(_)
            | Self::SveIntxPermExtractI(_)
            | Self::SveIntPermDupI(_)
            | Self::SveIntPermTbl3src(_)
            | Self::SveIntPermTbl(_)
            | Self::SveIntPermDupR(_)
            | Self::SveIntPermInsrs(_)
            | Self::SveIntPermUnpk(_)
            | Self::SveIntPermInsrv(_)
            | Self::SveIntPermReverseZ(_)
            | Self::SveIntPermBinPermPp(_)
            | Self::SveIntPermPunpk(_)
            | Self::SveIntPermReverseP(_)
            | Self::SveIntPermBinPermZz(_)
            | Self::SveIntPermCpyV(_)
            | Self::SveIntPermLastV(_)
            | Self::SveIntPermRev(_)
            | Self::SveIntPermClastZz(_)
            | Self::SveIntPermClastVz(_)
            | Self::SveIntPermSplice(_)
            | Self::SveIntxPermSplice(_)
            | Self::SveIntPermLastR(_)
            | Self::SveIntPermCpyR(_)
            | Self::SveIntPermClastRz(_)
            | Self::SveIntPermRevd(_)
            | Self::SveIntSelVvv(_)
            | Self::SveIntCmp0(_)
            | Self::SveIntCmp1(_)
            | Self::SveIntUcmpVi(_)
            | Self::SveIntScmpVi(_)
            | Self::SveIntPredLog(_)
            | Self::SveIntBrkp(_)
            | Self::SveIntBreak(_)
            | Self::SveIntBrkn(_)
            | Self::SveIntPtest(_)
            | Self::SveIntPfirst(_)
            | Self::SveIntPnext(_)
            | Self::SveIntPtrue(_)
            | Self::SveIntPfalse(_)
            | Self::SveIntPredDup(_)
            | Self::SveIntWhileRr(_)
            | Self::SveIntCterm(_)
            | Self::SveIntWhilenc(_)
            | Self::SveIntPcountPred(_)
            | Self::SveIntCountVSat(_)
            | Self::SveIntCountV(_)
            | Self::SveIntCountRSat(_)
            | Self::SveIntCountR(_)
            | Self::SveIntArithImm0(_)
            | Self::SveIntArithImm1(_)
            | Self::SveIntArithImm2(_)
            | Self::SveIntDupImm(_)
            | Self::SveIntDupFpimm(_)
            | Self::SveMem32bPfill(_)
            | Self::SveMem32bFill(_)
            | Self::SveMemPrfmSi(_)
            | Self::SveMemPrfmSs(_)
            | Self::SveMemLdDup(_)
            | Self::SveMemLdqrSs(_)
            | Self::SveMemLdqrSi(_)
            | Self::SveMemCldSs(_)
            | Self::SveMemCldSi(_)
            | Self::SveMemCldntSs(_)
            | Self::SveMemEldSs(_)
            | Self::SveMemCldntSi(_)
            | Self::SveMemEldSi(_)
            | Self::SveMemPspill(_)
            | Self::SveMemCstSs(_)
            | Self::SveMemSpill(_)
            | Self::SveMemCstntSs(_)
            | Self::SveMemEstSs(_)
            | Self::SveMemCstSi(_)
            | Self::SveMemCstntSi(_)
            | Self::SveMemEstSi(_) => Feature::Sme,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::features::Implementation;
    use crate::cpu::{Cpu, CpuConfig, SysReg};
    use std::vec::Vec;

    /// The ID registers, read the way an EL1 guest reads them, advertise
    /// exactly the features the processor implements, each of them one an
    /// encoding of the decoder belongs to, and no other instruction-set
    /// feature (Arm ARM D19.2, the `ID_AA64*_EL1` field descriptions): a
    /// capability and the register that reports it change together.
    #[test]
    fn the_id_registers_advertise_exactly_the_features_the_decoder_executes() {
        let source = spec::source();
        assert!(source.errors.is_empty(), "{:?}", source.errors);
        let mut carried: Vec<Feature> = Vec::new();
        for o in source.occurrences.iter() {
            if let Some(feature) = decode(o.cube.opcode).map(Encoding::feature) {
                if !carried.contains(&feature) {
                    carried.push(feature);
                }
            }
        }
        for feature in Feature::ALL {
            assert_eq!(carried.contains(&feature), feature.encodes(), "{feature:?}");
        }

        for imp in [Implementation::CORTEX_A57, Implementation::EVERY_FEATURE] {
            let cpu = Cpu::implementing(
                CpuConfig {
                    ips: rusty_box_core::ClockHz::BOCHS_DEFAULT,
                    counter_hz: CpuConfig::QEMU_VIRT_COUNTER_HZ,
                    reset_pc: 0,
                },
                imp,
            );
            let read = |crm: u8, op2: u8| {
                SysReg::new(3, 0, 0, crm, op2)
                    .map(|reg| cpu.sysreg(reg).unwrap_or(u64::MAX))
                    .unwrap_or(u64::MAX)
            };
            let field = |value: u64, lsb: u32| (value >> lsb) & 0xF;
            let (pfr0, pfr1) = (read(4, 0), read(4, 1));
            let (zfr0, smfr0) = (read(4, 4), read(4, 5));
            let (isar0, isar1, isar2, isar3) = (read(6, 0), read(6, 1), read(6, 2), read(6, 3));
            let (mmfr0, mmfr1, mmfr2) = (read(7, 0), read(7, 1), read(7, 2));
            let dfr0 = read(5, 0);
            for feature in Feature::ALL {
                let advertised = match feature {
                    Feature::Base => true,
                    Feature::Fp => field(pfr0, 16) != 0xF,
                    Feature::AdvSimd => field(pfr0, 20) != 0xF,
                    Feature::Aes => field(isar0, 4) != 0,
                    Feature::Sha1 => field(isar0, 8) != 0,
                    Feature::Sha256 => field(isar0, 12) != 0,
                    Feature::Crc32 => field(isar0, 16) != 0,
                    Feature::Sme => field(pfr1, 24) != 0,
                    Feature::Lse => field(isar0, 20) != 0,
                    Feature::Lse2 => field(mmfr2, 32) != 0,
                    Feature::Lrcpc => field(isar1, 20) >= 1,
                    Feature::Lrcpc2 => field(isar1, 20) >= 2,
                    Feature::Rdm => field(isar0, 28) != 0,
                    Feature::DotProd => field(isar0, 44) != 0,
                    Feature::I8mm => field(isar1, 52) != 0,
                    Feature::Fp16 => field(pfr0, 16) == 1 && field(pfr0, 20) == 1,
                    Feature::Fhm => field(isar0, 48) != 0,
                    Feature::Fcma => field(isar1, 16) != 0,
                    Feature::Jscvt => field(isar1, 12) != 0,
                    Feature::Frintts => field(isar1, 32) != 0,
                    Feature::Bf16 => field(isar1, 44) != 0,
                    Feature::PAuth => field(isar1, 4) >= 1,
                    Feature::PAuth2 => field(isar1, 4) >= 3,
                    Feature::Fpac => field(isar1, 4) >= 4,
                    Feature::FpacCombine => field(isar1, 4) >= 5,
                    Feature::PacQarma5 => field(isar1, 24) != 0,
                    Feature::Bti => field(pfr1, 0) != 0,
                    Feature::FlagM => field(isar0, 52) >= 1,
                    Feature::FlagM2 => field(isar0, 52) >= 2,
                    Feature::Sb => field(isar1, 36) != 0,
                    Feature::Pan => field(mmfr1, 20) >= 1,
                    Feature::Pan2 => field(mmfr1, 20) >= 2,
                    Feature::Pan3 => field(mmfr1, 20) >= 3,
                    Feature::Uao => field(mmfr2, 4) != 0,
                    Feature::Dit => field(pfr0, 48) != 0,
                    Feature::Ssbs => field(pfr1, 4) >= 1,
                    Feature::Ssbs2 => field(pfr1, 4) >= 2,
                    Feature::Hpds => field(mmfr1, 12) >= 1,
                    Feature::Ttcnp => field(mmfr2, 0) != 0,
                    Feature::E0pd => field(mmfr2, 60) != 0,
                    Feature::Csv3 => field(pfr0, 60) != 0,
                    Feature::TlbiOs => field(isar0, 56) >= 1,
                    Feature::TlbiRange => field(isar0, 56) >= 2,
                    Feature::Ttl => field(mmfr2, 48) != 0,
                    Feature::Xs => field(isar1, 56) != 0,
                    Feature::Dpb => field(isar1, 0) >= 1,
                    Feature::Dpb2 => field(isar1, 0) >= 2,
                    Feature::Lor => field(mmfr1, 16) != 0,
                    Feature::Idst => field(mmfr2, 36) != 0,
                    Feature::SpecRes => field(isar1, 40) != 0,
                    Feature::Csv2 => field(pfr0, 56) != 0,
                    Feature::Ecv => field(mmfr0, 60) != 0,
                    Feature::WfxT => field(isar2, 0) >= 2,
                    Feature::Ras => field(pfr0, 28) >= 1,
                    // `Features.json`: `FEAT_RASv1p1 <=> RAS >= 2 || (RAS == 1
                    // && RAS_frac >= 1)`, and FEAT_RASv1p1 implies
                    // FEAT_RASSAv1p1, which has no field of its own.
                    Feature::RasV1p1 | Feature::RasSaV1p1 => field(pfr0, 28) >= 2 || (field(pfr0, 28) == 1 && field(pfr1, 12) >= 1),
                    Feature::Debugv8p1 => field(dfr0, 0) >= 7,
                    Feature::Debugv8p2 => field(dfr0, 0) >= 8,
                    Feature::Debugv8p4 => field(dfr0, 0) >= 9,
                    Feature::Afp => field(mmfr1, 44) >= 1,
                    Feature::Rpres => field(isar2, 4) >= 1,
                };
                assert_eq!(advertised, imp.has(feature), "{feature:?} on {imp:?}");
            }
            // FP = AdvSIMD = 1 with half precision, 0 without; every other
            // instruction-set feature is a nonzero field of these registers.
            let fp16 = u64::from(imp.has(Feature::Fp16));
            assert_eq!((field(pfr0, 16), field(pfr0, 20)), (fp16, fp16), "FP and AdvSIMD");
            // PFR0: EL0 and EL1 (AArch64 only), FP, AdvSIMD, RAS, DIT, CSV2 =
            // 1 (no FEAT_CSV2_2) and CSV3; nothing else.
            assert_eq!(pfr0 & !((0xFF << 16) | (0xF << 28) | (0xF << 48) | (0xF << 56) | (0xF << 60)), 0x11);
            // DFR0: DebugVer, two breakpoints and two watchpoints, no PMU.
            assert_eq!(dfr0 & !0xF, 0x0010_1000);
            assert!((6..=9).contains(&field(dfr0, 0)), "DebugVer: Armv8.0 to FEAT_Debugv8p4");
            assert!(field(pfr0, 56) <= 1, "CSV2 = 1: FEAT_CSV2, not FEAT_CSV2_2");
            // ID_AA64ISAR0_EL1: AES = 2 (AES and the 64-bit PMULL), SHA1 =
            // 1, SHA2 = 1 (SHA-256, not SHA-512), CRC32 = 1, Atomic = 2 with
            // FEAT_LSE, RDM = 1, DP = 1, FHM = 1, TS and TLB; nothing else.
            // ISAR1 holds DPB, APA, JSCVT, FCMA, LRCPC, GPA, FRINTTS, SB,
            // SPECRES = 1 (no FEAT_SPECRES2), BF16 = 1 (no FEAT_EBF16), I8MM
            // and XS alone (no API or GPI: no IMPLEMENTATION DEFINED
            // algorithm, and APA is never 2, FEAT_EPAC), ISAR2 WFxT = 2
            // alone, MMFR0 the 4 KiB granule's fields and ECV = 1 (no
            // FEAT_ECV_POFF), MMFR1 HPDS = 1 (no FEAT_HPDS2), LO and PAN
            // alone, MMFR2 CnP, UAO, AT, IDS, TTL and E0PD alone. MMFR1
            // AFP and ISAR2 RPRES are each 1 or 0.
            let at = |feature: Feature, lsb: u32, value: u64| if imp.has(feature) { value << lsb } else { 0 };
            let isar0_later =
                at(Feature::Lse, 20, 2) | at(Feature::Rdm, 28, 1) | at(Feature::DotProd, 44, 1) | at(Feature::Fhm, 48, 1);
            assert_eq!(isar0 & !((0xF << 52) | (0xF << 56)), 0x0000_0000_0001_1120 | isar0_later);
            let isar1_fields =
                [0, 4, 12, 16, 20, 24, 32, 36, 40, 44, 52, 56].into_iter().fold(0, |acc, lsb| acc | (0xF << lsb));
            assert_eq!(isar1 & !isar1_fields, 0);
            assert_ne!(field(isar1, 4), 2, "APA = 2 is FEAT_EPAC, which FEAT_PAuth2 excludes");
            assert!(field(isar1, 40) <= 1, "SPECRES = 1: FEAT_SPECRES, not FEAT_SPECRES2");
            assert!(field(isar1, 44) <= 1, "BF16 = 1: FEAT_BF16, not FEAT_EBF16");
            assert_eq!(mmfr0 & !(0xF << 60), 0x0F00_0024, "PARange 44 bits, 16-bit ASIDs, the 4 KiB granule alone");
            assert!(field(mmfr0, 60) <= 1, "ECV = 1: FEAT_ECV, not FEAT_ECV_POFF");
            assert!(field(mmfr1, 12) <= 1, "HPDS = 1: FEAT_HPDS, not FEAT_HPDS2");
            assert_eq!(mmfr1 & !((0xF << 12) | (0xF << 16) | (0xF << 20)), at(Feature::Afp, 44, 1));
            assert_eq!(mmfr2 & !(0xF | (0xF << 4) | (0xF << 32) | (0xF << 36) | (0xF << 48) | (0xF << 60)), 0);
            assert_eq!((isar2, isar3), (at(Feature::WfxT, 0, 2) | at(Feature::Rpres, 4, 1), 0));
            // No SVE2 (SVEver = 0, with FEAT_SME as without); with FEAT_SME,
            // BF16 and I8MM report FEAT_BF16 and FEAT_I8MM as ISAR1 does.
            let zfr0_later = if imp.has(Feature::Sme) { (field(isar1, 44) << 20) | (field(isar1, 52) << 44) } else { 0 };
            assert_eq!(zfr0, zfr0_later);
            // PFR1: BT = 1 (FEAT_BTI, not FEAT_BTIE), SSBS and SME alone.
            assert_eq!(pfr1 & !((0xF << 24) | (0xF << 4) | 0xF), 0);
            assert!(field(pfr1, 0) <= 1, "BT = 1: FEAT_BTI, not FEAT_BTIE");
            if imp.has(Feature::Sme) {
                assert_eq!(field(pfr1, 24), 1, "SME = 1: FEAT_SME, not FEAT_SME2");
                assert_eq!(smfr0, 0x0000_00FD_0000_0000, "F32F32, B16F32, F16F32, I8I32; SMEver 0");
            } else {
                assert_eq!((field(pfr1, 24), smfr0), (0, 0));
            }
        }
    }
}
