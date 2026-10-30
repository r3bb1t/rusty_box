//! The A64 decoder: which encoding of the Arm architecture an instruction
//! word is, and what executes it here.
//!
//! The decoder is written against Arm's A64 encoding tree as the Arm
//! A-profile Architecture Machine Readable Specification (AARCHMRS) gives
//! it, `Instructions.json`, which is the A64 encoding index of the Arm ARM
//! (DDI 0487, chapter C4) in data form. The tree has three levels: the
//! top-level groups (`dpimm`, `control`, `ldst`, `dpreg`, `simd_dp`,
//! `reserved`, `sve`, `sme`), each split into the encoding classes of C4
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
//! would be here; no feature the decoder knows is being built.
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
mod sme;
#[cfg(test)]
#[path = "../../../tests/unit/spec.rs"]
pub(crate) mod spec;
mod sve;

use crate::cpu::exec::bits;
pub(crate) use crate::cpu::features::Feature;

use asimd::{
    Asimdall, Asimddiff, Asimdelem, Asimdext, Asimdimm, Asimdins, Asimdmisc, Asimdmiscfp16,
    Asimdperm, Asimdsame, Asimdsame2, Asimdsamefp16, Asimdshf, Asimdtbl,
};
use asisd::{
    Asisddiff, Asisdelem, Asisdmisc, Asisdmiscfp16, Asisdone, Asisdpair, Asisdsame, Asisdsame2,
    Asisdsamefp16, Asisdshf,
};
use atomic::{Comswap, Comswappr, LdapstlUnscaled, Memop};
use control::{
    Barriers, BranchImm, BranchReg, Compbranch, Condbranch, Exception, Hints, Pstate, Systeminstrs,
    Systeminstrswithreg, Systemmove, Testbranch,
};
use crypto::{Cryptoaes, Cryptosha2, Cryptosha3};
use dpimm::{AddsubImm, Bitfield, Extract, LogImm, Movewide, Pcreladdr};
use dpreg::{
    AddsubCarry, AddsubExt, AddsubShift, CondcmpImm, CondcmpReg, Condsel, Dp1src, Dp2src, Dp3src,
    LogShift, Rmif, Setf,
};
use float::{
    Float2fix, Float2int, Floatccmp, Floatcmp, Floatdp1, Floatdp2, Floatdp3, Floatimm, Floatsel,
};
use ldst::{
    Asisdlse, Asisdlsep, Asisdlso, Asisdlsop, LdstImmpost, LdstImmpre, LdstPac, LdstPos,
    LdstRegoff, LdstUnpriv, LdstUnscaled, Ldstexclp, Ldstexclr, LdstnapairOffs, Ldstord,
    LdstpairOff, LdstpairPost, LdstpairPre, Loadlit,
};
use reserved::PermUndef;
use sme::{
    MortlachAddhv, MortlachB16f32Prod, MortlachBini32Prod, MortlachContigLoad, MortlachContigQload,
    MortlachContigQstore, MortlachContigStore, MortlachCtxtLdst, MortlachExpand1dst,
    MortlachExpand2dstCtg, MortlachExpand4dstCtg, MortlachExtractPred, MortlachExtractZt,
    MortlachF16f32Prod, MortlachF32f32Prod, MortlachI16i32Prod, MortlachI8i32Prod,
    MortlachInsertPred, MortlachInsertZt, MortlachMulti1FmaLongIdx, MortlachMulti1MlaLongIdx,
    MortlachMulti1MlaLongLongIdxS, MortlachMulti1ZzZaFmaLongSm, MortlachMulti1ZzZaMlaLongLongSm,
    MortlachMulti1ZzZaMlaLongSm, MortlachMulti2ClampInt, MortlachMulti2CldCldntSiCtg,
    MortlachMulti2CldCldntSiNctg, MortlachMulti2CldCldntSsCtg, MortlachMulti2CldCldntSsNctg,
    MortlachMulti2CstCstntSiCtg, MortlachMulti2CstCstntSiNctg, MortlachMulti2CstCstntSsCtg,
    MortlachMulti2CstCstntSsNctg, MortlachMulti2ExtractCtg, MortlachMulti2Fclamp,
    MortlachMulti2FmaLongIdx, MortlachMulti2FpintCvrt, MortlachMulti2Frint,
    MortlachMulti2InsertCtg, MortlachMulti2IntfpCvrt, MortlachMulti2MlaLongIdx,
    MortlachMulti2MlaLongLongIdxS, MortlachMulti2NarrowFpCvrt, MortlachMulti2NarrowIntCvrt,
    MortlachMulti2Qrshr, MortlachMulti2SelectInt, MortlachMulti2WideInt, MortlachMulti2ZZAddSm,
    MortlachMulti2ZZFminmaxMm, MortlachMulti2ZZFminmaxSm, MortlachMulti2ZZLongZip,
    MortlachMulti2ZZMinmaxMm, MortlachMulti2ZZMinmaxSm, MortlachMulti2ZZShiftMm,
    MortlachMulti2ZZShiftSm, MortlachMulti2ZZSqdmulhMm, MortlachMulti2ZZSqdmulhSm,
    MortlachMulti2ZZZip, MortlachMulti2ZZa2wayDotMm, MortlachMulti2ZZa2wayDotSm,
    MortlachMulti2ZZa4wayDotMm, MortlachMulti2ZZa4wayDotSm, MortlachMulti2ZZaFloatMm,
    MortlachMulti2ZZaFpdotMm, MortlachMulti2ZZaFpdotSm, MortlachMulti2ZZaIntMm,
    MortlachMulti2ZZaMixedDotMm, MortlachMulti2ZZaMixedDotSm, MortlachMulti2ZaExtractCtg,
    MortlachMulti2ZaInsertCtg, MortlachMulti2ZzZaFloatMm, MortlachMulti2ZzZaFloatSm,
    MortlachMulti2ZzZaFmaLongMm, MortlachMulti2ZzZaFmaLongSm, MortlachMulti2ZzZaIntMm,
    MortlachMulti2ZzZaIntSm, MortlachMulti2ZzZaMlaLongLongMm, MortlachMulti2ZzZaMlaLongLongSm,
    MortlachMulti2ZzZaMlaLongMm, MortlachMulti2ZzZaMlaLongSm, MortlachMulti2ZzaIdxS,
    MortlachMulti4ClampInt, MortlachMulti4CldCldntSiCtg, MortlachMulti4CldCldntSiNctg,
    MortlachMulti4CldCldntSsCtg, MortlachMulti4CldCldntSsNctg, MortlachMulti4CstCstntSiCtg,
    MortlachMulti4CstCstntSiNctg, MortlachMulti4CstCstntSsCtg, MortlachMulti4CstCstntSsNctg,
    MortlachMulti4ExtractCtg, MortlachMulti4Fclamp, MortlachMulti4FmaLongIdx,
    MortlachMulti4FpintCvrt, MortlachMulti4Frint, MortlachMulti4InsertCtg, MortlachMulti4IntfpCvrt,
    MortlachMulti4MlaLongIdx, MortlachMulti4MlaLongLongIdxS, MortlachMulti4NarrowIntCvrt,
    MortlachMulti4Qrshr, MortlachMulti4SelectInt, MortlachMulti4WideInt, MortlachMulti4ZZAddSm,
    MortlachMulti4ZZFminmaxMm, MortlachMulti4ZZFminmaxSm, MortlachMulti4ZZLongZip,
    MortlachMulti4ZZMinmaxMm, MortlachMulti4ZZMinmaxSm, MortlachMulti4ZZShiftMm,
    MortlachMulti4ZZShiftSm, MortlachMulti4ZZSqdmulhMm, MortlachMulti4ZZSqdmulhSm,
    MortlachMulti4ZZZip, MortlachMulti4ZZa2wayDotMm, MortlachMulti4ZZa2wayDotSm,
    MortlachMulti4ZZa4wayDotMm, MortlachMulti4ZZa4wayDotSm, MortlachMulti4ZZaFloatMm,
    MortlachMulti4ZZaFpdotMm, MortlachMulti4ZZaFpdotSm, MortlachMulti4ZZaIntMm,
    MortlachMulti4ZZaMixedDotMm, MortlachMulti4ZZaMixedDotSm, MortlachMulti4ZaExtractCtg,
    MortlachMulti4ZaInsertCtg, MortlachMulti4ZzZaFloatMm, MortlachMulti4ZzZaFloatSm,
    MortlachMulti4ZzZaFmaLongMm, MortlachMulti4ZzZaFmaLongSm, MortlachMulti4ZzZaIntMm,
    MortlachMulti4ZzZaIntSm, MortlachMulti4ZzZaMlaLongLongMm, MortlachMulti4ZzZaMlaLongLongSm,
    MortlachMulti4ZzZaMlaLongMm, MortlachMulti4ZzZaMlaLongSm, MortlachMulti4ZzaIdxS, MortlachZero,
    MortlachZeroZt, MortlachZtLdst,
};
use sve::{
    SveFp2opIPZds, SveFp2opPPd, SveFp2opPZdA, SveFp2opPZdB0, SveFp2opPZdB1, SveFp2opPZdC,
    SveFp2opPZdD, SveFp2opPZds, SveFp2opUZd, SveFp3opPPd, SveFp3opPZdsA, SveFp3opPZdsB,
    SveFp3opUZd, SveFpClamp, SveFpFastRed, SveFpFcadd, SveFpFcmla, SveFpFcmlaByIndexedElem,
    SveFpFcvt2, SveFpFdot, SveFpFdotByIndexedElem, SveFpFmaByIndexedElem, SveFpFmaLong,
    SveFpFmaLongByIndexedElem, SveFpFmulByIndexedElem, SveFpPairwise, SveIntArithImm0,
    SveIntArithImm1, SveIntArithImm2, SveIntArithSvl, SveIntArithVl, SveIntBinConsArit0,
    SveIntBinConsLog, SveIntBinConsMisc0D, SveIntBinConsShiftA, SveIntBinConsShiftB,
    SveIntBinPredArit0, SveIntBinPredArit1, SveIntBinPredArit2, SveIntBinPredDiv, SveIntBinPredLog,
    SveIntBinPredShift0, SveIntBinPredShift1, SveIntBinPredShift2, SveIntBreak, SveIntBrkn,
    SveIntBrkp, SveIntCmp0, SveIntCmp1, SveIntCount, SveIntCountR, SveIntCountRSat, SveIntCountV,
    SveIntCountVSat, SveIntCountvlv0, SveIntCountvlv1, SveIntCterm, SveIntCtrToMask,
    SveIntDupFpimm, SveIntDupFpimmPred, SveIntDupImm, SveIntDupImmPred, SveIntDupMaskImm,
    SveIntIndexIi, SveIntIndexIr, SveIntIndexRi, SveIntIndexRr, SveIntLogImm,
    SveIntMladdsubVvvPred, SveIntMlasVvvPred, SveIntMovprfxPred, SveIntMulB, SveIntPcountPn,
    SveIntPcountPred, SveIntPermBinPermPp, SveIntPermBinPermZz, SveIntPermClastRz,
    SveIntPermClastVz, SveIntPermClastZz, SveIntPermCpyR, SveIntPermCpyV, SveIntPermDupI,
    SveIntPermDupR, SveIntPermExtractI, SveIntPermInsrs, SveIntPermInsrv, SveIntPermLastR,
    SveIntPermLastV, SveIntPermPunpk, SveIntPermRev, SveIntPermRevd, SveIntPermReverseP,
    SveIntPermReverseZ, SveIntPermSplice, SveIntPermTbl, SveIntPermTbl3src, SveIntPermUnpk,
    SveIntPfalse, SveIntPfirst, SveIntPnPtrue, SveIntPnext, SveIntPredDup, SveIntPredLog,
    SveIntPredPatternA, SveIntPredPatternB, SveIntPtest, SveIntPtrue, SveIntReadSvlA,
    SveIntReadVlA, SveIntReduce0, SveIntReduce1, SveIntReduce2, SveIntRotateImm, SveIntScmpVi,
    SveIntSelVvv, SveIntSqdmulh, SveIntTernLog, SveIntUcmpVi, SveIntUnPredArit0, SveIntUnPredArit1,
    SveIntWhileRr, SveIntWhileRrPair, SveIntWhileRrPn, SveIntWhilenc, SveIntxAba, SveIntxAbaLong,
    SveIntxAccumulateLongPairs, SveIntxAdcLong, SveIntxArithBinaryPairs, SveIntxArithNarrow,
    SveIntxBinPredShiftSatRound, SveIntxCadd, SveIntxCdot, SveIntxCdotByIndexedElem, SveIntxClamp,
    SveIntxClong, SveIntxCmla, SveIntxCmlaByIndexedElem, SveIntxConsArithLong,
    SveIntxConsArithWide, SveIntxConsMulLong, SveIntxDot, SveIntxDot2, SveIntxDot2ByIndexedElem,
    SveIntxDotByIndexedElem, SveIntxEorx, SveIntxExtractNarrow, SveIntxMixedDot,
    SveIntxMixedDotByIndexedElem, SveIntxMlaByIndexedElem, SveIntxMlaLongByIndexedElem,
    SveIntxMlalLong, SveIntxMulByIndexedElem, SveIntxMulLongByIndexedElem,
    SveIntxMultiExtractNarrow, SveIntxMultiShiftNarrow, SveIntxPermExtractI, SveIntxPermSplice,
    SveIntxPredArithBinary, SveIntxPredArithBinarySat, SveIntxPredArithUnary,
    SveIntxQdmlaLongByIndexedElem, SveIntxQdmlalLong, SveIntxQdmlalbt,
    SveIntxQdmulLongByIndexedElem, SveIntxQdmulhByIndexedElem, SveIntxQrdcmlaByIndexedElem,
    SveIntxQrdmlah, SveIntxQrdmlahByIndexedElem, SveIntxShiftInsert, SveIntxShiftLong,
    SveIntxShiftNarrow, SveIntxSra, SveMem32bFill, SveMem32bPfill, SveMemCldSi, SveMemCldSs,
    SveMemCldntSi, SveMemCldntSs, SveMemCstSi, SveMemCstSs, SveMemCstntSi, SveMemCstntSs,
    SveMemEldSi, SveMemEldSs, SveMemEstSi, SveMemEstSs, SveMemLdDup, SveMemLdqrSi, SveMemLdqrSs,
    SveMemPrfmSi, SveMemPrfmSs, SveMemPspill, SveMemSpill,
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
    SveIntWhileRrPn(SveIntWhileRrPn),
    SveIntWhileRrPair(SveIntWhileRrPair),
    SveIntCtrToMask(SveIntCtrToMask),
    SveIntPnPtrue(SveIntPnPtrue),
    SveIntPcountPred(SveIntPcountPred),
    SveIntPcountPn(SveIntPcountPn),
    SveIntCountVSat(SveIntCountVSat),
    SveIntCountV(SveIntCountV),
    SveIntCountRSat(SveIntCountRSat),
    SveIntCountR(SveIntCountR),
    SveIntArithImm0(SveIntArithImm0),
    SveIntArithImm1(SveIntArithImm1),
    SveIntArithImm2(SveIntArithImm2),
    SveIntDupImm(SveIntDupImm),
    SveIntDupFpimm(SveIntDupFpimm),
    SveIntxDot(SveIntxDot),
    SveIntxQdmlalbt(SveIntxQdmlalbt),
    SveIntxCdot(SveIntxCdot),
    SveIntxCmla(SveIntxCmla),
    SveIntxMlalLong(SveIntxMlalLong),
    SveIntxQdmlalLong(SveIntxQdmlalLong),
    SveIntxQrdmlah(SveIntxQrdmlah),
    SveIntxMixedDot(SveIntxMixedDot),
    SveIntxBinPredShiftSatRound(SveIntxBinPredShiftSatRound),
    SveIntxPredArithUnary(SveIntxPredArithUnary),
    SveIntxAccumulateLongPairs(SveIntxAccumulateLongPairs),
    SveIntxPredArithBinary(SveIntxPredArithBinary),
    SveIntxArithBinaryPairs(SveIntxArithBinaryPairs),
    SveIntxPredArithBinarySat(SveIntxPredArithBinarySat),
    SveIntxClamp(SveIntxClamp),
    SveIntxDot2(SveIntxDot2),
    SveIntxDot2ByIndexedElem(SveIntxDot2ByIndexedElem),
    SveIntxDotByIndexedElem(SveIntxDotByIndexedElem),
    SveIntxMlaByIndexedElem(SveIntxMlaByIndexedElem),
    SveIntxQrdmlahByIndexedElem(SveIntxQrdmlahByIndexedElem),
    SveIntxMixedDotByIndexedElem(SveIntxMixedDotByIndexedElem),
    SveIntxQdmlaLongByIndexedElem(SveIntxQdmlaLongByIndexedElem),
    SveIntxCdotByIndexedElem(SveIntxCdotByIndexedElem),
    SveIntxCmlaByIndexedElem(SveIntxCmlaByIndexedElem),
    SveIntxQrdcmlaByIndexedElem(SveIntxQrdcmlaByIndexedElem),
    SveIntxMlaLongByIndexedElem(SveIntxMlaLongByIndexedElem),
    SveIntxMulLongByIndexedElem(SveIntxMulLongByIndexedElem),
    SveIntxQdmulLongByIndexedElem(SveIntxQdmulLongByIndexedElem),
    SveIntxQdmulhByIndexedElem(SveIntxQdmulhByIndexedElem),
    SveIntxMulByIndexedElem(SveIntxMulByIndexedElem),
    SveIntxConsArithLong(SveIntxConsArithLong),
    SveIntxConsArithWide(SveIntxConsArithWide),
    SveIntxConsMulLong(SveIntxConsMulLong),
    SveIntxClong(SveIntxClong),
    SveIntxEorx(SveIntxEorx),
    SveIntxShiftLong(SveIntxShiftLong),
    SveIntxAbaLong(SveIntxAbaLong),
    SveIntxAdcLong(SveIntxAdcLong),
    SveIntxCadd(SveIntxCadd),
    SveIntxSra(SveIntxSra),
    SveIntxShiftInsert(SveIntxShiftInsert),
    SveIntxAba(SveIntxAba),
    SveIntxShiftNarrow(SveIntxShiftNarrow),
    SveIntxExtractNarrow(SveIntxExtractNarrow),
    SveIntxMultiShiftNarrow(SveIntxMultiShiftNarrow),
    SveIntxMultiExtractNarrow(SveIntxMultiExtractNarrow),
    SveIntxArithNarrow(SveIntxArithNarrow),
    SveFpFcmla(SveFpFcmla),
    SveFpFcadd(SveFpFcadd),
    SveFpFcvt2(SveFpFcvt2),
    SveFpPairwise(SveFpPairwise),
    SveFpFmaByIndexedElem(SveFpFmaByIndexedElem),
    SveFpFcmlaByIndexedElem(SveFpFcmlaByIndexedElem),
    SveFpFmulByIndexedElem(SveFpFmulByIndexedElem),
    SveFpClamp(SveFpClamp),
    SveFpFdotByIndexedElem(SveFpFdotByIndexedElem),
    SveFpFmaLongByIndexedElem(SveFpFmaLongByIndexedElem),
    SveFpFdot(SveFpFdot),
    SveFpFmaLong(SveFpFmaLong),
    SveFp3opUZd(SveFp3opUZd),
    SveFpFastRed(SveFpFastRed),
    SveFp3opPPd(SveFp3opPPd),
    SveFp2opPZds(SveFp2opPZds),
    SveFp2opIPZds(SveFp2opIPZds),
    SveFp2opPZdA(SveFp2opPZdA),
    SveFp2opPZdB0(SveFp2opPZdB0),
    SveFp2opPZdB1(SveFp2opPZdB1),
    SveFp2opPZdC(SveFp2opPZdC),
    SveFp2opPZdD(SveFp2opPZdD),
    SveFp2opUZd(SveFp2opUZd),
    SveFp2opPPd(SveFp2opPPd),
    SveFp3opPZdsA(SveFp3opPZdsA),
    SveFp3opPZdsB(SveFp3opPZdsB),
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
    // sme
    MortlachF32f32Prod(MortlachF32f32Prod),
    MortlachB16f32Prod(MortlachB16f32Prod),
    MortlachF16f32Prod(MortlachF16f32Prod),
    MortlachI8i32Prod(MortlachI8i32Prod),
    MortlachInsertPred(MortlachInsertPred),
    MortlachExtractPred(MortlachExtractPred),
    MortlachZero(MortlachZero),
    MortlachAddhv(MortlachAddhv),
    MortlachContigLoad(MortlachContigLoad),
    MortlachContigStore(MortlachContigStore),
    MortlachCtxtLdst(MortlachCtxtLdst),
    MortlachContigQload(MortlachContigQload),
    MortlachContigQstore(MortlachContigQstore),
    MortlachMulti2CldCldntSsCtg(MortlachMulti2CldCldntSsCtg),
    MortlachMulti4CldCldntSsCtg(MortlachMulti4CldCldntSsCtg),
    MortlachMulti2CstCstntSsCtg(MortlachMulti2CstCstntSsCtg),
    MortlachMulti4CstCstntSsCtg(MortlachMulti4CstCstntSsCtg),
    MortlachMulti2CldCldntSiCtg(MortlachMulti2CldCldntSiCtg),
    MortlachMulti4CldCldntSiCtg(MortlachMulti4CldCldntSiCtg),
    MortlachMulti2CstCstntSiCtg(MortlachMulti2CstCstntSiCtg),
    MortlachMulti4CstCstntSiCtg(MortlachMulti4CstCstntSiCtg),
    MortlachMulti2CldCldntSsNctg(MortlachMulti2CldCldntSsNctg),
    MortlachMulti4CldCldntSsNctg(MortlachMulti4CldCldntSsNctg),
    MortlachMulti2CstCstntSsNctg(MortlachMulti2CstCstntSsNctg),
    MortlachMulti4CstCstntSsNctg(MortlachMulti4CstCstntSsNctg),
    MortlachMulti2CldCldntSiNctg(MortlachMulti2CldCldntSiNctg),
    MortlachMulti4CldCldntSiNctg(MortlachMulti4CldCldntSiNctg),
    MortlachMulti2CstCstntSiNctg(MortlachMulti2CstCstntSiNctg),
    MortlachMulti4CstCstntSiNctg(MortlachMulti4CstCstntSiNctg),
    MortlachZeroZt(MortlachZeroZt),
    MortlachExtractZt(MortlachExtractZt),
    MortlachInsertZt(MortlachInsertZt),
    MortlachZtLdst(MortlachZtLdst),
    MortlachMulti2SelectInt(MortlachMulti2SelectInt),
    MortlachMulti4SelectInt(MortlachMulti4SelectInt),
    MortlachMulti2ZZMinmaxSm(MortlachMulti2ZZMinmaxSm),
    MortlachMulti2ZZFminmaxSm(MortlachMulti2ZZFminmaxSm),
    MortlachMulti2ZZShiftSm(MortlachMulti2ZZShiftSm),
    MortlachMulti2ZZAddSm(MortlachMulti2ZZAddSm),
    MortlachMulti2ZZSqdmulhSm(MortlachMulti2ZZSqdmulhSm),
    MortlachMulti4ZZMinmaxSm(MortlachMulti4ZZMinmaxSm),
    MortlachMulti4ZZFminmaxSm(MortlachMulti4ZZFminmaxSm),
    MortlachMulti4ZZShiftSm(MortlachMulti4ZZShiftSm),
    MortlachMulti4ZZAddSm(MortlachMulti4ZZAddSm),
    MortlachMulti4ZZSqdmulhSm(MortlachMulti4ZZSqdmulhSm),
    MortlachMulti2ZZMinmaxMm(MortlachMulti2ZZMinmaxMm),
    MortlachMulti2ZZFminmaxMm(MortlachMulti2ZZFminmaxMm),
    MortlachMulti2ZZShiftMm(MortlachMulti2ZZShiftMm),
    MortlachMulti2ZZSqdmulhMm(MortlachMulti2ZZSqdmulhMm),
    MortlachMulti4ZZMinmaxMm(MortlachMulti4ZZMinmaxMm),
    MortlachMulti4ZZFminmaxMm(MortlachMulti4ZZFminmaxMm),
    MortlachMulti4ZZShiftMm(MortlachMulti4ZZShiftMm),
    MortlachMulti4ZZSqdmulhMm(MortlachMulti4ZZSqdmulhMm),
    MortlachMulti2Fclamp(MortlachMulti2Fclamp),
    MortlachMulti2ClampInt(MortlachMulti2ClampInt),
    MortlachMulti4Fclamp(MortlachMulti4Fclamp),
    MortlachMulti4ClampInt(MortlachMulti4ClampInt),
    MortlachMulti2ZZZip(MortlachMulti2ZZZip),
    MortlachMulti2ZZLongZip(MortlachMulti2ZZLongZip),
    MortlachMulti2Qrshr(MortlachMulti2Qrshr),
    MortlachMulti4Qrshr(MortlachMulti4Qrshr),
    MortlachMulti2NarrowFpCvrt(MortlachMulti2NarrowFpCvrt),
    MortlachMulti2FpintCvrt(MortlachMulti2FpintCvrt),
    MortlachMulti2IntfpCvrt(MortlachMulti2IntfpCvrt),
    MortlachMulti2NarrowIntCvrt(MortlachMulti2NarrowIntCvrt),
    MortlachMulti2WideInt(MortlachMulti2WideInt),
    MortlachMulti2Frint(MortlachMulti2Frint),
    MortlachMulti4FpintCvrt(MortlachMulti4FpintCvrt),
    MortlachMulti4IntfpCvrt(MortlachMulti4IntfpCvrt),
    MortlachMulti4NarrowIntCvrt(MortlachMulti4NarrowIntCvrt),
    MortlachMulti4WideInt(MortlachMulti4WideInt),
    MortlachMulti4ZZZip(MortlachMulti4ZZZip),
    MortlachMulti4ZZLongZip(MortlachMulti4ZZLongZip),
    MortlachMulti4Frint(MortlachMulti4Frint),
    MortlachMulti2ZzZaMlaLongLongSm(MortlachMulti2ZzZaMlaLongLongSm),
    MortlachMulti1ZzZaMlaLongLongSm(MortlachMulti1ZzZaMlaLongLongSm),
    MortlachMulti2ZzZaFmaLongSm(MortlachMulti2ZzZaFmaLongSm),
    MortlachMulti2ZzZaMlaLongSm(MortlachMulti2ZzZaMlaLongSm),
    MortlachMulti1ZzZaFmaLongSm(MortlachMulti1ZzZaFmaLongSm),
    MortlachMulti1ZzZaMlaLongSm(MortlachMulti1ZzZaMlaLongSm),
    MortlachMulti2ZZaFpdotSm(MortlachMulti2ZZaFpdotSm),
    MortlachMulti2ZZa4wayDotSm(MortlachMulti2ZZa4wayDotSm),
    MortlachMulti2ZZaMixedDotSm(MortlachMulti2ZZaMixedDotSm),
    MortlachMulti2ZZa2wayDotSm(MortlachMulti2ZZa2wayDotSm),
    MortlachMulti2ZzZaFloatSm(MortlachMulti2ZzZaFloatSm),
    MortlachMulti2ZzZaIntSm(MortlachMulti2ZzZaIntSm),
    MortlachMulti4ZzZaMlaLongLongSm(MortlachMulti4ZzZaMlaLongLongSm),
    MortlachMulti4ZzZaFmaLongSm(MortlachMulti4ZzZaFmaLongSm),
    MortlachMulti4ZzZaMlaLongSm(MortlachMulti4ZzZaMlaLongSm),
    MortlachMulti4ZZaFpdotSm(MortlachMulti4ZZaFpdotSm),
    MortlachMulti4ZZa4wayDotSm(MortlachMulti4ZZa4wayDotSm),
    MortlachMulti4ZZaMixedDotSm(MortlachMulti4ZZaMixedDotSm),
    MortlachMulti4ZZa2wayDotSm(MortlachMulti4ZZa2wayDotSm),
    MortlachMulti4ZzZaFloatSm(MortlachMulti4ZzZaFloatSm),
    MortlachMulti4ZzZaIntSm(MortlachMulti4ZzZaIntSm),
    MortlachMulti2ZzZaMlaLongLongMm(MortlachMulti2ZzZaMlaLongLongMm),
    MortlachMulti2ZzZaFmaLongMm(MortlachMulti2ZzZaFmaLongMm),
    MortlachMulti2ZzZaMlaLongMm(MortlachMulti2ZzZaMlaLongMm),
    MortlachMulti2ZZaFpdotMm(MortlachMulti2ZZaFpdotMm),
    MortlachMulti2ZZa4wayDotMm(MortlachMulti2ZZa4wayDotMm),
    MortlachMulti2ZZaMixedDotMm(MortlachMulti2ZZaMixedDotMm),
    MortlachMulti2ZZa2wayDotMm(MortlachMulti2ZZa2wayDotMm),
    MortlachMulti2ZzZaFloatMm(MortlachMulti2ZzZaFloatMm),
    MortlachMulti2ZzZaIntMm(MortlachMulti2ZzZaIntMm),
    MortlachMulti2ZZaFloatMm(MortlachMulti2ZZaFloatMm),
    MortlachMulti2ZZaIntMm(MortlachMulti2ZZaIntMm),
    MortlachMulti4ZzZaMlaLongLongMm(MortlachMulti4ZzZaMlaLongLongMm),
    MortlachMulti4ZzZaFmaLongMm(MortlachMulti4ZzZaFmaLongMm),
    MortlachMulti4ZzZaMlaLongMm(MortlachMulti4ZzZaMlaLongMm),
    MortlachMulti4ZZaFpdotMm(MortlachMulti4ZZaFpdotMm),
    MortlachMulti4ZZa4wayDotMm(MortlachMulti4ZZa4wayDotMm),
    MortlachMulti4ZZaMixedDotMm(MortlachMulti4ZZaMixedDotMm),
    MortlachMulti4ZZa2wayDotMm(MortlachMulti4ZZa2wayDotMm),
    MortlachMulti4ZzZaFloatMm(MortlachMulti4ZzZaFloatMm),
    MortlachMulti4ZzZaIntMm(MortlachMulti4ZzZaIntMm),
    MortlachMulti4ZZaFloatMm(MortlachMulti4ZZaFloatMm),
    MortlachMulti4ZZaIntMm(MortlachMulti4ZZaIntMm),
    MortlachMulti1MlaLongLongIdxS(MortlachMulti1MlaLongLongIdxS),
    MortlachMulti1FmaLongIdx(MortlachMulti1FmaLongIdx),
    MortlachMulti1MlaLongIdx(MortlachMulti1MlaLongIdx),
    MortlachMulti2MlaLongLongIdxS(MortlachMulti2MlaLongLongIdxS),
    MortlachMulti2ZzaIdxS(MortlachMulti2ZzaIdxS),
    MortlachMulti2FmaLongIdx(MortlachMulti2FmaLongIdx),
    MortlachMulti2MlaLongIdx(MortlachMulti2MlaLongIdx),
    MortlachMulti4MlaLongLongIdxS(MortlachMulti4MlaLongLongIdxS),
    MortlachMulti4ZzaIdxS(MortlachMulti4ZzaIdxS),
    MortlachMulti4FmaLongIdx(MortlachMulti4FmaLongIdx),
    MortlachMulti4MlaLongIdx(MortlachMulti4MlaLongIdx),
    MortlachBini32Prod(MortlachBini32Prod),
    MortlachI16i32Prod(MortlachI16i32Prod),
    MortlachMulti2InsertCtg(MortlachMulti2InsertCtg),
    MortlachMulti4InsertCtg(MortlachMulti4InsertCtg),
    MortlachMulti2ZaInsertCtg(MortlachMulti2ZaInsertCtg),
    MortlachMulti4ZaInsertCtg(MortlachMulti4ZaInsertCtg),
    MortlachMulti2ExtractCtg(MortlachMulti2ExtractCtg),
    MortlachMulti4ExtractCtg(MortlachMulti4ExtractCtg),
    MortlachMulti2ZaExtractCtg(MortlachMulti2ZaExtractCtg),
    MortlachMulti4ZaExtractCtg(MortlachMulti4ZaExtractCtg),
    MortlachExpand1dst(MortlachExpand1dst),
    MortlachExpand2dstCtg(MortlachExpand2dstCtg),
    MortlachExpand4dstCtg(MortlachExpand4dstCtg),
}

/// The encoding `word` is, if it is one of a feature the decoder knows.
/// Which of those a processor executes is its own feature set's choice
/// (`decode::classify`).
pub(crate) fn decode(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 31, 31);
    let op1 = bits(word, 28, 25);
    match (op0, op1) {
        (0b0, 0b0000) => reserved::reserved(word),
        (0b1, 0b0000) => sme::sme(word),
        // op1 = 0001 and 0011 are unallocated.
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
            Self::SveIntReduce0(_) | Self::SveIntReduce1(_) | Self::SveIntReduce2(_) => {
                Class::SveIntReduction
            }
            Self::SveIntMovprfxPred(_) | Self::SveIntBinConsMisc0D(_) => Class::SveMovprfx,
            Self::SveIntBinPredShift0(_)
            | Self::SveIntBinPredShift1(_)
            | Self::SveIntBinPredShift2(_) => Class::SveShiftPredicated,
            Self::SveIntUnPredArit0(_) | Self::SveIntUnPredArit1(_) => Class::SveIntUnaryPredicated,
            Self::SveIntMlasVvvPred(_) | Self::SveIntMladdsubVvvPred(_) => {
                Class::SveIntMulAddPredicated
            }
            Self::SveIntBinConsArit0(_) => Class::SveIntAddSubUnpredicated,
            Self::SveIntBinConsLog(_) | Self::SveIntRotateImm(_) | Self::SveIntTernLog(_) => {
                Class::SveBitwiseUnpredicated
            }
            Self::SveIntBinConsShiftA(_) | Self::SveIntBinConsShiftB(_) => {
                Class::SveShiftUnpredicated
            }
            Self::SveIntIndexIi(_)
            | Self::SveIntIndexRi(_)
            | Self::SveIntIndexIr(_)
            | Self::SveIntIndexRr(_) => Class::SveIndex,
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
            Self::SveIntPermBinPermPp(_)
            | Self::SveIntPermPunpk(_)
            | Self::SveIntPermReverseP(_) => Class::SvePermutePredicate,
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
            Self::SveIntCmp0(_)
            | Self::SveIntCmp1(_)
            | Self::SveIntUcmpVi(_)
            | Self::SveIntScmpVi(_) => Class::SveIntCompare,
            Self::SveIntPredLog(_) => Class::SvePredicateLogical,
            Self::SveIntBrkp(_) | Self::SveIntBreak(_) | Self::SveIntBrkn(_) => Class::SveBreak,
            Self::SveIntPtest(_)
            | Self::SveIntPfirst(_)
            | Self::SveIntPnext(_)
            | Self::SveIntPtrue(_)
            | Self::SveIntPfalse(_)
            | Self::SveIntPredDup(_) => Class::SvePredicateMisc,
            Self::SveIntWhileRr(_) | Self::SveIntWhilenc(_) => Class::SveWhile,
            Self::SveIntWhileRrPn(_) => Class::SveWhileCounter,
            Self::SveIntWhileRrPair(_) => Class::SveWhilePair,
            Self::SveIntCtrToMask(_) => Class::SvePext,
            Self::SveIntPnPtrue(_) => Class::SvePtrueCounter,
            Self::SveIntPcountPn(_) => Class::SveCntpCounter,
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
            Self::SveIntxDot(_)
            | Self::SveIntxMixedDot(_)
            | Self::SveIntxDotByIndexedElem(_)
            | Self::SveIntxMixedDotByIndexedElem(_) => Class::SveIntxDot,
            Self::SveIntxCdot(_)
            | Self::SveIntxCmla(_)
            | Self::SveIntxCdotByIndexedElem(_)
            | Self::SveIntxCmlaByIndexedElem(_)
            | Self::SveIntxQrdcmlaByIndexedElem(_) => Class::SveIntxComplex,
            Self::SveIntxQdmlalbt(_)
            | Self::SveIntxMlalLong(_)
            | Self::SveIntxQdmlalLong(_)
            | Self::SveIntxMlaLongByIndexedElem(_)
            | Self::SveIntxQdmlaLongByIndexedElem(_)
            | Self::SveIntxMulLongByIndexedElem(_)
            | Self::SveIntxQdmulLongByIndexedElem(_)
            | Self::SveIntxConsMulLong(_) => Class::SveIntxMulLong,
            Self::SveIntxConsArithLong(_)
            | Self::SveIntxConsArithWide(_)
            | Self::SveIntxClong(_) => Class::SveIntxLongArithmetic,
            Self::SveIntxEorx(_) => Class::SveIntxEorInterleaved,
            Self::SveIntxShiftLong(_) => Class::SveIntxShiftLong,
            Self::SveIntxAbaLong(_) | Self::SveIntxAba(_) => Class::SveIntxAbsDiffAccumulate,
            Self::SveIntxAdcLong(_) => Class::SveIntxCarryLong,
            Self::SveIntxCadd(_) => Class::SveIntxComplexAdd,
            Self::SveIntxSra(_) | Self::SveIntxShiftInsert(_) => Class::SveIntxShiftAccumulate,
            Self::SveIntxShiftNarrow(_)
            | Self::SveIntxExtractNarrow(_)
            | Self::SveIntxArithNarrow(_) => Class::SveIntxNarrow,
            Self::SveIntxMultiShiftNarrow(_) | Self::SveIntxMultiExtractNarrow(_) => {
                Class::SveIntxNarrowPair
            }
            Self::SveIntxQrdmlah(_)
            | Self::SveIntxMlaByIndexedElem(_)
            | Self::SveIntxQrdmlahByIndexedElem(_)
            | Self::SveIntxQdmulhByIndexedElem(_)
            | Self::SveIntxMulByIndexedElem(_) => Class::SveIntxMul,
            Self::SveIntxBinPredShiftSatRound(_)
            | Self::SveIntxPredArithBinary(_)
            | Self::SveIntxArithBinaryPairs(_)
            | Self::SveIntxPredArithBinarySat(_) => Class::SveIntxBinaryPredicated,
            Self::SveIntxPredArithUnary(_) | Self::SveIntxAccumulateLongPairs(_) => {
                Class::SveIntxUnaryPredicated
            }
            Self::SveIntxClamp(_) => Class::SveIntxClamp,
            Self::SveIntxDot2(_) | Self::SveIntxDot2ByIndexedElem(_) => Class::SveIntxDot,
            Self::SveFp2opPZds(_) | Self::SveFp2opIPZds(_) => Class::SveFpArithmeticPredicated,
            Self::SveFp3opUZd(_) => Class::SveFpArithmeticUnpredicated,
            Self::SveFpFastRed(_) => Class::SveFpReduction,
            Self::SveFp3opPPd(_) | Self::SveFp2opPPd(_) => Class::SveFpCompare,
            Self::SveFp2opPZdA(_)
            | Self::SveFp2opPZdB0(_)
            | Self::SveFp2opPZdB1(_)
            | Self::SveFp2opPZdC(_)
            | Self::SveFp2opPZdD(_) => Class::SveFpUnaryPredicated,
            Self::SveFp2opUZd(_) => Class::SveFpEstimate,
            Self::SveFp3opPZdsA(_) | Self::SveFp3opPZdsB(_) => Class::SveFpMulAdd,
            Self::SveFpFmaByIndexedElem(_) | Self::SveFpFmulByIndexedElem(_) => Class::SveFpIndexed,
            Self::SveFpClamp(_) => Class::SveFpClamp,
            Self::SveFpFcmla(_) | Self::SveFpFcadd(_) | Self::SveFpFcmlaByIndexedElem(_) => {
                Class::SveFpComplex
            }
            Self::SveFpPairwise(_) => Class::SveFpPairwise,
            Self::SveFpFcvt2(_) => Class::SveFpConvertOdd,
            Self::SveFpFdotByIndexedElem(_)
            | Self::SveFpFmaLongByIndexedElem(_)
            | Self::SveFpFdot(_)
            | Self::SveFpFmaLong(_) => Class::SveFpWidening,
            Self::SveMem32bPfill(_)
            | Self::SveMem32bFill(_)
            | Self::SveMemPspill(_)
            | Self::SveMemSpill(_) => Class::SveFillSpill,
            Self::SveMemPrfmSi(_) | Self::SveMemPrfmSs(_) => Class::SvePrefetch,
            Self::SveMemLdDup(_) => Class::SveLoadBroadcast,
            Self::SveMemLdqrSs(_) | Self::SveMemLdqrSi(_) => Class::SveLoadBroadcastQuadword,
            Self::SveMemCldSs(_) | Self::SveMemCldSi(_) => Class::SveLoadContiguous,
            Self::SveMemCldntSs(_)
            | Self::SveMemEldSs(_)
            | Self::SveMemCldntSi(_)
            | Self::SveMemEldSi(_) => Class::SveLoadStructures,
            Self::SveMemCstSs(_) | Self::SveMemCstSi(_) => Class::SveStoreContiguous,
            Self::SveMemCstntSs(_)
            | Self::SveMemEstSs(_)
            | Self::SveMemCstntSi(_)
            | Self::SveMemEstSi(_) => Class::SveStoreStructures,
            Self::MortlachF32f32Prod(_)
            | Self::MortlachB16f32Prod(_)
            | Self::MortlachF16f32Prod(_) => Class::SmeFpOuterProduct,
            Self::MortlachI8i32Prod(_) => Class::SmeIntOuterProduct,
            Self::MortlachInsertPred(_) => Class::SmeMoveToSlice,
            Self::MortlachExtractPred(_) => Class::SmeMoveFromSlice,
            Self::MortlachZero(_) => Class::SmeZero,
            Self::MortlachAddhv(_) => Class::SmeAddVector,
            Self::MortlachContigLoad(_) | Self::MortlachContigQload(_) => Class::SmeLoadSlice,
            Self::MortlachContigStore(_) | Self::MortlachContigQstore(_) => Class::SmeStoreSlice,
            Self::MortlachCtxtLdst(_) => Class::SmeFillSpill,
            Self::MortlachMulti2CldCldntSsCtg(_)
            | Self::MortlachMulti4CldCldntSsCtg(_)
            | Self::MortlachMulti2CldCldntSiCtg(_)
            | Self::MortlachMulti4CldCldntSiCtg(_)
            | Self::MortlachMulti2CldCldntSsNctg(_)
            | Self::MortlachMulti4CldCldntSsNctg(_)
            | Self::MortlachMulti2CldCldntSiNctg(_)
            | Self::MortlachMulti4CldCldntSiNctg(_) => Class::SveLoadMulti,
            Self::MortlachMulti2CstCstntSsCtg(_)
            | Self::MortlachMulti4CstCstntSsCtg(_)
            | Self::MortlachMulti2CstCstntSiCtg(_)
            | Self::MortlachMulti4CstCstntSiCtg(_)
            | Self::MortlachMulti2CstCstntSsNctg(_)
            | Self::MortlachMulti4CstCstntSsNctg(_)
            | Self::MortlachMulti2CstCstntSiNctg(_)
            | Self::MortlachMulti4CstCstntSiNctg(_) => Class::SveStoreMulti,
            Self::MortlachZeroZt(_) => Class::SmeZeroZt0,
            // MOVT between ZT0 and a general register is defined only in
            // Debug state (its decode's `!Halted()`), which this processor
            // never enters.
            Self::MortlachExtractZt(_) | Self::MortlachInsertZt(_) => Class::Undefined,
            Self::MortlachZtLdst(_) => Class::SmeFillSpillZt0,
            Self::MortlachMulti2SelectInt(_) | Self::MortlachMulti4SelectInt(_) => {
                Class::SveMultiSelect
            }
            Self::MortlachMulti2ZZMinmaxSm(_)
            | Self::MortlachMulti2ZZShiftSm(_)
            | Self::MortlachMulti2ZZAddSm(_)
            | Self::MortlachMulti2ZZSqdmulhSm(_)
            | Self::MortlachMulti4ZZMinmaxSm(_)
            | Self::MortlachMulti4ZZShiftSm(_)
            | Self::MortlachMulti4ZZAddSm(_)
            | Self::MortlachMulti4ZZSqdmulhSm(_)
            | Self::MortlachMulti2ZZMinmaxMm(_)
            | Self::MortlachMulti2ZZShiftMm(_)
            | Self::MortlachMulti2ZZSqdmulhMm(_)
            | Self::MortlachMulti4ZZMinmaxMm(_)
            | Self::MortlachMulti4ZZShiftMm(_)
            | Self::MortlachMulti4ZZSqdmulhMm(_) => Class::SveMultiIntBinary,
            Self::MortlachMulti2ZZFminmaxSm(_)
            | Self::MortlachMulti4ZZFminmaxSm(_)
            | Self::MortlachMulti2ZZFminmaxMm(_)
            | Self::MortlachMulti4ZZFminmaxMm(_) => Class::SveMultiFpBinary,
            Self::MortlachMulti2Fclamp(_)
            | Self::MortlachMulti2ClampInt(_)
            | Self::MortlachMulti4Fclamp(_)
            | Self::MortlachMulti4ClampInt(_) => Class::SveMultiClamp,
            Self::MortlachMulti2ZZZip(_)
            | Self::MortlachMulti2ZZLongZip(_)
            | Self::MortlachMulti4ZZZip(_)
            | Self::MortlachMulti4ZZLongZip(_) => Class::SveMultiZip,
            Self::MortlachMulti2Qrshr(_) | Self::MortlachMulti4Qrshr(_) => {
                Class::SveMultiNarrowShift
            }
            Self::MortlachMulti2NarrowIntCvrt(_) | Self::MortlachMulti4NarrowIntCvrt(_) => {
                Class::SveMultiNarrowInt
            }
            Self::MortlachMulti2NarrowFpCvrt(_) => Class::SveMultiNarrowFp,
            Self::MortlachMulti2FpintCvrt(_)
            | Self::MortlachMulti2IntfpCvrt(_)
            | Self::MortlachMulti2Frint(_)
            | Self::MortlachMulti4FpintCvrt(_)
            | Self::MortlachMulti4IntfpCvrt(_)
            | Self::MortlachMulti4Frint(_) => Class::SveMultiFpUnary,
            Self::MortlachMulti2WideInt(_) | Self::MortlachMulti4WideInt(_) => {
                Class::SveMultiUnpack
            }
            Self::MortlachMulti2ZzZaMlaLongLongSm(_)
            | Self::MortlachMulti1ZzZaMlaLongLongSm(_)
            | Self::MortlachMulti4ZzZaMlaLongLongSm(_)
            | Self::MortlachMulti2ZzZaMlaLongLongMm(_)
            | Self::MortlachMulti4ZzZaMlaLongLongMm(_)
            | Self::MortlachMulti1MlaLongLongIdxS(_)
            | Self::MortlachMulti2MlaLongLongIdxS(_)
            | Self::MortlachMulti4MlaLongLongIdxS(_) => Class::SmeArrayIntLongLong,
            Self::MortlachMulti2ZzZaMlaLongSm(_)
            | Self::MortlachMulti1ZzZaMlaLongSm(_)
            | Self::MortlachMulti4ZzZaMlaLongSm(_)
            | Self::MortlachMulti2ZzZaMlaLongMm(_)
            | Self::MortlachMulti4ZzZaMlaLongMm(_) => Class::SmeArrayIntLong,
            Self::MortlachMulti2ZzZaFmaLongSm(_)
            | Self::MortlachMulti1ZzZaFmaLongSm(_)
            | Self::MortlachMulti4ZzZaFmaLongSm(_)
            | Self::MortlachMulti2ZzZaFmaLongMm(_)
            | Self::MortlachMulti4ZzZaFmaLongMm(_) => Class::SmeArrayFpLong,
            Self::MortlachMulti2ZZaFpdotSm(_)
            | Self::MortlachMulti4ZZaFpdotSm(_)
            | Self::MortlachMulti2ZZaFpdotMm(_)
            | Self::MortlachMulti4ZZaFpdotMm(_) => Class::SmeArrayFpDot,
            Self::MortlachMulti2ZZa4wayDotSm(_)
            | Self::MortlachMulti2ZZaMixedDotSm(_)
            | Self::MortlachMulti2ZZa2wayDotSm(_)
            | Self::MortlachMulti4ZZa4wayDotSm(_)
            | Self::MortlachMulti4ZZaMixedDotSm(_)
            | Self::MortlachMulti4ZZa2wayDotSm(_)
            | Self::MortlachMulti2ZZa4wayDotMm(_)
            | Self::MortlachMulti2ZZaMixedDotMm(_)
            | Self::MortlachMulti2ZZa2wayDotMm(_)
            | Self::MortlachMulti4ZZa4wayDotMm(_)
            | Self::MortlachMulti4ZZaMixedDotMm(_)
            | Self::MortlachMulti4ZZa2wayDotMm(_) => Class::SmeArrayIntDot,
            Self::MortlachMulti2ZzZaFloatSm(_)
            | Self::MortlachMulti4ZzZaFloatSm(_)
            | Self::MortlachMulti2ZzZaFloatMm(_)
            | Self::MortlachMulti4ZzZaFloatMm(_) => Class::SmeArrayFpMulAdd,
            Self::MortlachMulti2ZzZaIntSm(_)
            | Self::MortlachMulti4ZzZaIntSm(_)
            | Self::MortlachMulti2ZzZaIntMm(_)
            | Self::MortlachMulti4ZzZaIntMm(_) => Class::SmeArrayAddVectors,
            Self::MortlachMulti2ZZaFloatMm(_)
            | Self::MortlachMulti4ZZaFloatMm(_)
            | Self::MortlachMulti2ZZaIntMm(_)
            | Self::MortlachMulti4ZZaIntMm(_) => Class::SmeArrayAccumulate,
            Self::MortlachMulti1MlaLongIdx(_)
            | Self::MortlachMulti2MlaLongIdx(_)
            | Self::MortlachMulti4MlaLongIdx(_) => Class::SmeArrayIntLong,
            Self::MortlachMulti1FmaLongIdx(_)
            | Self::MortlachMulti2FmaLongIdx(_)
            | Self::MortlachMulti4FmaLongIdx(_) => Class::SmeArrayFpLong,
            Self::MortlachMulti2ZzaIdxS(encoding) => match encoding {
                MortlachMulti2ZzaIdxS::FmlaZaZziS2xi | MortlachMulti2ZzaIdxS::FmlsZaZziS2xi => {
                    Class::SmeIndexedFpMulAdd
                }
                MortlachMulti2ZzaIdxS::FdotZaZzi2xi | MortlachMulti2ZzaIdxS::BfdotZaZzi2xi => {
                    Class::SmeArrayFpDot
                }
                MortlachMulti2ZzaIdxS::SdotZa32Zzi2xi
                | MortlachMulti2ZzaIdxS::UdotZa32Zzi2xi
                | MortlachMulti2ZzaIdxS::SdotZaZziS2xi
                | MortlachMulti2ZzaIdxS::UdotZaZziS2xi
                | MortlachMulti2ZzaIdxS::UsdotZaZziS2xi
                | MortlachMulti2ZzaIdxS::SudotZaZziS2xi => Class::SmeIndexedIntDot,
                MortlachMulti2ZzaIdxS::FvdotZaZzi2xi
                | MortlachMulti2ZzaIdxS::BfvdotZaZzi2xi
                | MortlachMulti2ZzaIdxS::SvdotZa32Zzi2xi
                | MortlachMulti2ZzaIdxS::UvdotZa32Zzi2xi => Class::SmeIndexedVerticalDot,
            },
            Self::MortlachMulti4ZzaIdxS(encoding) => match encoding {
                MortlachMulti4ZzaIdxS::FmlaZaZziS4xi | MortlachMulti4ZzaIdxS::FmlsZaZziS4xi => {
                    Class::SmeIndexedFpMulAdd
                }
                MortlachMulti4ZzaIdxS::FdotZaZzi4xi | MortlachMulti4ZzaIdxS::BfdotZaZzi4xi => {
                    Class::SmeArrayFpDot
                }
                MortlachMulti4ZzaIdxS::SdotZa32Zzi4xi
                | MortlachMulti4ZzaIdxS::UdotZa32Zzi4xi
                | MortlachMulti4ZzaIdxS::SdotZaZziS4xi
                | MortlachMulti4ZzaIdxS::UdotZaZziS4xi
                | MortlachMulti4ZzaIdxS::UsdotZaZziS4xi
                | MortlachMulti4ZzaIdxS::SudotZaZziS4xi => Class::SmeIndexedIntDot,
                MortlachMulti4ZzaIdxS::SvdotZaZziS4xi
                | MortlachMulti4ZzaIdxS::UvdotZaZziS4xi
                | MortlachMulti4ZzaIdxS::UsvdotZaZziS4xi
                | MortlachMulti4ZzaIdxS::SuvdotZaZziS4xi => Class::SmeIndexedVerticalDot,
            },
            Self::MortlachBini32Prod(_) => Class::SmeBinaryOuterProduct,
            Self::MortlachI16i32Prod(_) => Class::SmeIntOuterProductHalfwords,
            Self::MortlachMulti2InsertCtg(_) | Self::MortlachMulti4InsertCtg(_) => {
                Class::SmeMoveGroupToSlices
            }
            Self::MortlachMulti2ExtractCtg(_) | Self::MortlachMulti4ExtractCtg(_) => {
                Class::SmeMoveGroupFromSlices
            }
            Self::MortlachMulti2ZaInsertCtg(_) | Self::MortlachMulti4ZaInsertCtg(_) => {
                Class::SmeMoveGroupToArray
            }
            Self::MortlachMulti2ZaExtractCtg(_) | Self::MortlachMulti4ZaExtractCtg(_) => {
                Class::SmeMoveGroupFromArray
            }
            Self::MortlachExpand1dst(_)
            | Self::MortlachExpand2dstCtg(_)
            | Self::MortlachExpand4dstCtg(_) => Class::SmeLookupTable,
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
    /// undefined, every word [`decode`] finds no encoding for, and the
    /// encodings defined only in Debug state, which this processor never
    /// enters.
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
    /// WHILE<cc> (predicate-as-counter).
    SveWhileCounter,
    /// WHILE<cc> (pair of predicates).
    SveWhilePair,
    /// PEXT of one predicate and of a pair.
    SvePext,
    /// PTRUE (predicate-as-counter).
    SvePtrueCounter,
    /// CNTP (predicate-as-counter).
    SveCntpCounter,
    /// CTERMEQ, CTERMNE.
    SveConditionallyTerminate,
    /// CNTP, INCP, DECP and their saturating forms.
    SvePredicateCount,
    /// `sve_wideimm_unpred`: the add/subtract, min/max and multiply
    /// immediates, DUP (immediate), FDUP.
    SveWideImmediate,
    /// SDOT, UDOT, USDOT, by vector and indexed (bit 21), and SUDOT
    /// (indexed); and SDOT and UDOT of halfword pairs, by vector and
    /// indexed (bit 23).
    SveIntxDot,
    /// CDOT, CMLA and SQRDCMLAH, by vector and indexed (bit 21).
    SveIntxComplex,
    /// The long multiplies and multiply-adds: SMLALB to UMLSLT, SQDMLALB to
    /// SQDMLSLT, SQDMLALBT and SQDMLSLBT by vector; SMLALB to UMLSLT,
    /// SQDMLALB to SQDMLSLT, SMULLB to UMULLT, SQDMULLB and SQDMULLT
    /// indexed (bit 21); SMULLB to UMULLT, SQDMULLB, SQDMULLT, PMULLB and
    /// PMULLT by vector (bit 24).
    SveIntxMulLong,
    /// SADDLB to UABDLT, SADDWB to USUBWT, SADDLBT, SSUBLBT, SSUBLTB: the
    /// sums, differences and absolute differences of half-size elements,
    /// or of a wide element and a half-size one.
    SveIntxLongArithmetic,
    /// EORBT, EORTB.
    SveIntxEorInterleaved,
    /// SSHLLB, SSHLLT, USHLLB, USHLLT.
    SveIntxShiftLong,
    /// SABA, UABA and SABALB to UABALT.
    SveIntxAbsDiffAccumulate,
    /// ADCLB, ADCLT, SBCLB, SBCLT.
    SveIntxCarryLong,
    /// CADD, SQCADD.
    SveIntxComplexAdd,
    /// SSRA, SRSRA, USRA, URSRA, SRI, SLI.
    SveIntxShiftAccumulate,
    /// The narrowing shifts (SHRNB to UQRSHRNT), the saturating extracts
    /// (SQXTNB to UQXTNT) and the high halves of sums and differences
    /// (ADDHNB to RSUBHNT).
    SveIntxNarrow,
    /// SQRSHRN, SQRSHRUN, UQRSHRN, SQCVTN, SQCVTUN, UQCVTN of two vectors.
    SveIntxNarrowPair,
    /// SQRDMLAH and SQRDMLSH by vector and indexed (bit 21); MLA, MLS,
    /// MUL, SQDMULH and SQRDMULH indexed.
    SveIntxMul,
    /// `sve_intx_predicated`'s binary operations: the saturating and
    /// rounding shifts, the halving adds and subtracts, the pairwise
    /// operations and the saturating adds and subtracts.
    SveIntxBinaryPredicated,
    /// URECPE, URSQRTE, SQABS, SQNEG, SADALP, UADALP.
    SveIntxUnaryPredicated,
    /// SCLAMP, UCLAMP.
    SveIntxClamp,
    /// `sve_fp_pred`: FADD, FSUB, FMUL, FSUBR, FMAXNM, FMINNM, FMAX, FMIN
    /// of vectors and of an immediate (bit 20), and FABD, FSCALE, FMULX,
    /// FDIVR, FDIV, predicated.
    SveFpArithmeticPredicated,
    /// FADD, FSUB, FMUL, FRECPS, FRSQRTS, unpredicated.
    SveFpArithmeticUnpredicated,
    /// FADDV, FMAXNMV, FMINNMV, FMAXV, FMINV.
    SveFpReduction,
    /// FCMEQ, FCMNE, FCMGE, FCMGT, FCMUO, FACGE, FACGT of vectors, and the
    /// compares with zero (bit 14 clear).
    SveFpCompare,
    /// `sve_fp_unary`: FRINT*, FCVT, FCVTX, BFCVT, FRECPX, FSQRT, SCVTF,
    /// UCVTF, FLOGB, FCVTZS, FCVTZU, merging.
    SveFpUnaryPredicated,
    /// FRECPE, FRSQRTE (vectors).
    SveFpEstimate,
    /// FMLA, FMLS, FNMLA, FNMLS and FMAD, FMSB, FNMAD, FNMSB (bit 15),
    /// predicated.
    SveFpMulAdd,
    /// FMLA, FMLS and FMUL (bit 13), indexed.
    SveFpIndexed,
    /// FCLAMP.
    SveFpClamp,
    /// FCMLA, predicated and indexed (bit 21), and FCADD.
    SveFpComplex,
    /// FADDP, FMAXNMP, FMINNMP, FMAXP, FMINP.
    SveFpPairwise,
    /// FCVTNT, FCVTXNT, BFCVTNT, FCVTLT.
    SveFpConvertOdd,
    /// FMLALB, FMLALT, FMLSLB, FMLSLT, BFMLALB, BFMLALT, BFMLSLB, BFMLSLT,
    /// FDOT and BFDOT, by vector and indexed.
    SveFpWidening,
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
    /// LD1B to LD1D and LDNT1B to LDNT1D of two or four vectors under a
    /// predicate-as-counter: the handler reads the registers, the
    /// addressing form and the element size from the word.
    SveLoadMulti,
    /// ST1B to ST1D and STNT1B to STNT1D of two or four vectors, as
    /// [`Class::SveLoadMulti`].
    SveStoreMulti,
    /// SEL of two or four vectors under a predicate-as-counter.
    SveMultiSelect,
    /// SMAX, SMIN, UMAX, UMIN, SRSHL, URSHL, ADD and SQDMULH of two or
    /// four vectors by one or by as many: the handler reads the operation
    /// and the registers from the word.
    SveMultiIntBinary,
    /// FMAX, FMIN, FMAXNM and FMINNM of two or four vectors by one or by as
    /// many.
    SveMultiFpBinary,
    /// SCLAMP, UCLAMP and FCLAMP of two or four vectors.
    SveMultiClamp,
    /// ZIP and UZP of elements or quadwords, two vectors into two and four
    /// into four.
    SveMultiZip,
    /// SQRSHR, SQRSHRU, UQRSHR of two or four vectors, and SQRSHRN,
    /// SQRSHRUN, UQRSHRN of four.
    SveMultiNarrowShift,
    /// SQCVT, SQCVTU, UQCVT of two or four vectors, and SQCVTN, SQCVTUN,
    /// UQCVTN of four.
    SveMultiNarrowInt,
    /// BFCVT, FCVT, BFCVTN and FCVTN of two vectors.
    SveMultiNarrowFp,
    /// FCVTZS, FCVTZU, SCVTF, UCVTF, FRINTN, FRINTP, FRINTM and FRINTA of
    /// two or four vectors of words.
    SveMultiFpUnary,
    /// SUNPK and UUNPK of one vector to two and of two to four.
    SveMultiUnpack,
    /// SMLALL, SMLSLL, UMLALL, UMLSLL, USMLALL and SUMLALL of bytes into
    /// ZA array vectors of words: one, two or four vectors by one, by as
    /// many or by an indexed byte.
    SmeArrayIntLongLong,
    /// SMLAL, SMLSL, UMLAL and UMLSL of halfwords into ZA array vectors of
    /// words, by vectors or by an indexed halfword.
    SmeArrayIntLong,
    /// FMLAL, FMLSL, BFMLAL and BFMLSL of half-precision and BFloat16
    /// values into ZA array vectors of single-precision values, by vectors
    /// or by an indexed element.
    SmeArrayFpLong,
    /// FDOT and BFDOT of pairs into ZA array vectors of single-precision
    /// values, by vectors or by an indexed pair.
    SmeArrayFpDot,
    /// SDOT, UDOT, USDOT and SUDOT of bytes, and SDOT and UDOT of
    /// halfwords, into ZA array vectors of words.
    SmeArrayIntDot,
    /// FMLA and FMLS of single-precision values into ZA array vectors.
    SmeArrayFpMulAdd,
    /// ADD and SUB of vectors into ZA array vectors of words: the sum or
    /// difference replaces them.
    SmeArrayAddVectors,
    /// FADD, FSUB, ADD and SUB of vectors to ZA array vectors of
    /// single-precision values or words.
    SmeArrayAccumulate,
    /// SDOT, UDOT, USDOT and SUDOT of bytes, and SDOT and UDOT of
    /// halfwords, by an indexed group into ZA array vectors of words.
    SmeIndexedIntDot,
    /// FMLA and FMLS of single-precision values by an indexed element into
    /// ZA array vectors.
    SmeIndexedFpMulAdd,
    /// FVDOT, BFVDOT, SVDOT, UVDOT, USVDOT and SUVDOT: the dot products of
    /// the elements in the same place of each register of the group, by an
    /// indexed group, into ZA array vectors.
    SmeIndexedVerticalDot,
    /// BMOPA and BMOPS: the population counts of the XNORs of words, into
    /// 32-bit tiles.
    SmeBinaryOuterProduct,
    /// SMOPA, UMOPA, SMOPS and UMOPS of halfwords into 32-bit tiles.
    SmeIntOuterProductHalfwords,
    /// MOVA of two or four vectors to consecutive tile slices.
    SmeMoveGroupToSlices,
    /// MOVA of two or four consecutive tile slices to vectors.
    SmeMoveGroupFromSlices,
    /// MOVA of two or four vectors to ZA array vectors.
    SmeMoveGroupToArray,
    /// MOVA of ZA array vectors to two or four vectors.
    SmeMoveGroupFromArray,
    /// LUTI2 and LUTI4 from ZT0 into one, two or four vectors.
    SmeLookupTable,
    /// FMOPA and FMOPS of single-precision and of half-precision values,
    /// BFMOPA and BFMOPS, into 32-bit tiles: the handler reads which from
    /// bits 24, 21 and 4.
    SmeFpOuterProduct,
    /// SMOPA, SUMOPA, USMOPA, UMOPA, SMOPS, SUMOPS, USMOPS, UMOPS of bytes
    /// into 32-bit tiles.
    SmeIntOuterProduct,
    /// MOVA from a vector to a tile slice.
    SmeMoveToSlice,
    /// MOVA from a tile slice to a vector.
    SmeMoveFromSlice,
    /// ZERO of a list of 64-bit tiles.
    SmeZero,
    /// ADDHA and ADDVA of words.
    SmeAddVector,
    /// LD1B, LD1H, LD1W, LD1D and LD1Q to a tile slice.
    SmeLoadSlice,
    /// ST1B, ST1H, ST1W, ST1D and ST1Q from a tile slice.
    SmeStoreSlice,
    /// LDR and STR of a ZA array vector: the handler reads the direction
    /// from bit 21.
    SmeFillSpill,
    /// ZERO {ZT0}.
    SmeZeroZt0,
    /// LDR and STR of ZT0: the handler reads the direction from bit 21.
    SmeFillSpillZt0,
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
            | Self::SveIntxDot(_)
            | Self::SveIntxQdmlalbt(_)
            | Self::SveIntxCdot(_)
            | Self::SveIntxCmla(_)
            | Self::SveIntxMlalLong(_)
            | Self::SveIntxQdmlalLong(_)
            | Self::SveIntxQrdmlah(_)
            | Self::SveIntxMixedDot(_)
            | Self::SveIntxBinPredShiftSatRound(_)
            | Self::SveIntxPredArithUnary(_)
            | Self::SveIntxAccumulateLongPairs(_)
            | Self::SveIntxPredArithBinary(_)
            | Self::SveIntxArithBinaryPairs(_)
            | Self::SveIntxPredArithBinarySat(_)
            | Self::SveIntxClamp(_)
            | Self::SveIntxDotByIndexedElem(_)
            | Self::SveIntxMlaByIndexedElem(_)
            | Self::SveIntxQrdmlahByIndexedElem(_)
            | Self::SveIntxMixedDotByIndexedElem(_)
            | Self::SveIntxQdmlaLongByIndexedElem(_)
            | Self::SveIntxCdotByIndexedElem(_)
            | Self::SveIntxCmlaByIndexedElem(_)
            | Self::SveIntxQrdcmlaByIndexedElem(_)
            | Self::SveIntxMlaLongByIndexedElem(_)
            | Self::SveIntxMulLongByIndexedElem(_)
            | Self::SveIntxQdmulLongByIndexedElem(_)
            | Self::SveIntxQdmulhByIndexedElem(_)
            | Self::SveIntxMulByIndexedElem(_)
            | Self::SveIntxConsArithLong(_)
            | Self::SveIntxConsArithWide(_)
            | Self::SveIntxConsMulLong(_)
            | Self::SveIntxClong(_)
            | Self::SveIntxEorx(_)
            | Self::SveIntxShiftLong(_)
            | Self::SveIntxAbaLong(_)
            | Self::SveIntxAdcLong(_)
            | Self::SveIntxCadd(_)
            | Self::SveIntxSra(_)
            | Self::SveIntxShiftInsert(_)
            | Self::SveIntxAba(_)
            | Self::SveIntxShiftNarrow(_)
            | Self::SveIntxExtractNarrow(_)
            | Self::SveIntxArithNarrow(_)
            | Self::SveFpFcmla(_)
            | Self::SveFpFcadd(_)
            | Self::SveFpFcvt2(_)
            | Self::SveFpPairwise(_)
            | Self::SveFpFmaByIndexedElem(_)
            | Self::SveFpFcmlaByIndexedElem(_)
            | Self::SveFpFmulByIndexedElem(_)
            | Self::SveFp3opUZd(_)
            | Self::SveFpFastRed(_)
            | Self::SveFp3opPPd(_)
            | Self::SveFp2opPZds(_)
            | Self::SveFp2opIPZds(_)
            | Self::SveFp2opPZdA(_)
            | Self::SveFp2opPZdB0(_)
            | Self::SveFp2opPZdB1(_)
            | Self::SveFp2opPZdC(_)
            | Self::SveFp2opPZdD(_)
            | Self::SveFp2opUZd(_)
            | Self::SveFp2opPPd(_)
            | Self::SveFp3opPZdsA(_)
            | Self::SveFp3opPZdsB(_)
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
            | Self::SveMemEstSi(_)
            | Self::MortlachF32f32Prod(_)
            | Self::MortlachB16f32Prod(_)
            | Self::MortlachF16f32Prod(_)
            | Self::MortlachI8i32Prod(_)
            | Self::MortlachInsertPred(_)
            | Self::MortlachExtractPred(_)
            | Self::MortlachZero(_)
            | Self::MortlachAddhv(_)
            | Self::MortlachContigLoad(_)
            | Self::MortlachContigStore(_)
            | Self::MortlachCtxtLdst(_)
            | Self::MortlachContigQload(_)
            | Self::MortlachContigQstore(_) => Feature::Sme,
            // FEAT_SVE2p1 has the predicate-as-counter instructions and the
            // consecutive multi-vector loads and stores as well, which this
            // decoder does not separate, implementing FEAT_SME2 alone.
            Self::SveIntWhileRrPn(_)
            | Self::SveIntWhileRrPair(_)
            | Self::SveIntCtrToMask(_)
            | Self::SveIntPnPtrue(_)
            | Self::SveIntPcountPn(_)
            | Self::MortlachMulti2CldCldntSsCtg(_)
            | Self::MortlachMulti4CldCldntSsCtg(_)
            | Self::MortlachMulti2CstCstntSsCtg(_)
            | Self::MortlachMulti4CstCstntSsCtg(_)
            | Self::MortlachMulti2CldCldntSiCtg(_)
            | Self::MortlachMulti4CldCldntSiCtg(_)
            | Self::MortlachMulti2CstCstntSiCtg(_)
            | Self::MortlachMulti4CstCstntSiCtg(_)
            | Self::MortlachMulti2CldCldntSsNctg(_)
            | Self::MortlachMulti4CldCldntSsNctg(_)
            | Self::MortlachMulti2CstCstntSsNctg(_)
            | Self::MortlachMulti4CstCstntSsNctg(_)
            | Self::MortlachMulti2CldCldntSiNctg(_)
            | Self::MortlachMulti4CldCldntSiNctg(_)
            | Self::MortlachMulti2CstCstntSiNctg(_)
            | Self::MortlachMulti4CstCstntSiNctg(_)
            | Self::MortlachZeroZt(_)
            | Self::MortlachExtractZt(_)
            | Self::MortlachInsertZt(_)
            | Self::MortlachZtLdst(_)
            | Self::MortlachMulti2SelectInt(_)
            | Self::MortlachMulti4SelectInt(_)
            | Self::MortlachMulti2ZZMinmaxSm(_)
            | Self::MortlachMulti2ZZFminmaxSm(_)
            | Self::MortlachMulti2ZZShiftSm(_)
            | Self::MortlachMulti2ZZAddSm(_)
            | Self::MortlachMulti2ZZSqdmulhSm(_)
            | Self::MortlachMulti4ZZMinmaxSm(_)
            | Self::MortlachMulti4ZZFminmaxSm(_)
            | Self::MortlachMulti4ZZShiftSm(_)
            | Self::MortlachMulti4ZZAddSm(_)
            | Self::MortlachMulti4ZZSqdmulhSm(_)
            | Self::MortlachMulti2ZZMinmaxMm(_)
            | Self::MortlachMulti2ZZFminmaxMm(_)
            | Self::MortlachMulti2ZZShiftMm(_)
            | Self::MortlachMulti2ZZSqdmulhMm(_)
            | Self::MortlachMulti4ZZMinmaxMm(_)
            | Self::MortlachMulti4ZZFminmaxMm(_)
            | Self::MortlachMulti4ZZShiftMm(_)
            | Self::MortlachMulti4ZZSqdmulhMm(_)
            | Self::MortlachMulti2Fclamp(_)
            | Self::MortlachMulti2ClampInt(_)
            | Self::MortlachMulti4Fclamp(_)
            | Self::MortlachMulti4ClampInt(_)
            | Self::MortlachMulti2ZZZip(_)
            | Self::MortlachMulti2ZZLongZip(_)
            | Self::MortlachMulti2Qrshr(_)
            | Self::MortlachMulti4Qrshr(_)
            | Self::MortlachMulti2NarrowFpCvrt(_)
            | Self::MortlachMulti2FpintCvrt(_)
            | Self::MortlachMulti2IntfpCvrt(_)
            | Self::MortlachMulti2NarrowIntCvrt(_)
            | Self::MortlachMulti2WideInt(_)
            | Self::MortlachMulti2Frint(_)
            | Self::MortlachMulti4FpintCvrt(_)
            | Self::MortlachMulti4IntfpCvrt(_)
            | Self::MortlachMulti4NarrowIntCvrt(_)
            | Self::MortlachMulti4WideInt(_)
            | Self::MortlachMulti4ZZZip(_)
            | Self::MortlachMulti4ZZLongZip(_)
            | Self::MortlachMulti4Frint(_)
            | Self::MortlachMulti2ZzZaMlaLongLongSm(_)
            | Self::MortlachMulti1ZzZaMlaLongLongSm(_)
            | Self::MortlachMulti2ZzZaFmaLongSm(_)
            | Self::MortlachMulti2ZzZaMlaLongSm(_)
            | Self::MortlachMulti1ZzZaFmaLongSm(_)
            | Self::MortlachMulti1ZzZaMlaLongSm(_)
            | Self::MortlachMulti2ZZaFpdotSm(_)
            | Self::MortlachMulti2ZZa4wayDotSm(_)
            | Self::MortlachMulti2ZZaMixedDotSm(_)
            | Self::MortlachMulti2ZZa2wayDotSm(_)
            | Self::MortlachMulti2ZzZaFloatSm(_)
            | Self::MortlachMulti2ZzZaIntSm(_)
            | Self::MortlachMulti4ZzZaMlaLongLongSm(_)
            | Self::MortlachMulti4ZzZaFmaLongSm(_)
            | Self::MortlachMulti4ZzZaMlaLongSm(_)
            | Self::MortlachMulti4ZZaFpdotSm(_)
            | Self::MortlachMulti4ZZa4wayDotSm(_)
            | Self::MortlachMulti4ZZaMixedDotSm(_)
            | Self::MortlachMulti4ZZa2wayDotSm(_)
            | Self::MortlachMulti4ZzZaFloatSm(_)
            | Self::MortlachMulti4ZzZaIntSm(_)
            | Self::MortlachMulti2ZzZaMlaLongLongMm(_)
            | Self::MortlachMulti2ZzZaFmaLongMm(_)
            | Self::MortlachMulti2ZzZaMlaLongMm(_)
            | Self::MortlachMulti2ZZaFpdotMm(_)
            | Self::MortlachMulti2ZZa4wayDotMm(_)
            | Self::MortlachMulti2ZZaMixedDotMm(_)
            | Self::MortlachMulti2ZZa2wayDotMm(_)
            | Self::MortlachMulti2ZzZaFloatMm(_)
            | Self::MortlachMulti2ZzZaIntMm(_)
            | Self::MortlachMulti2ZZaFloatMm(_)
            | Self::MortlachMulti2ZZaIntMm(_)
            | Self::MortlachMulti4ZzZaMlaLongLongMm(_)
            | Self::MortlachMulti4ZzZaFmaLongMm(_)
            | Self::MortlachMulti4ZzZaMlaLongMm(_)
            | Self::MortlachMulti4ZZaFpdotMm(_)
            | Self::MortlachMulti4ZZa4wayDotMm(_)
            | Self::MortlachMulti4ZZaMixedDotMm(_)
            | Self::MortlachMulti4ZZa2wayDotMm(_)
            | Self::MortlachMulti4ZzZaFloatMm(_)
            | Self::MortlachMulti4ZzZaIntMm(_)
            | Self::MortlachMulti4ZZaFloatMm(_)
            | Self::MortlachMulti4ZZaIntMm(_)
            | Self::MortlachMulti1MlaLongLongIdxS(_)
            | Self::MortlachMulti1FmaLongIdx(_)
            | Self::MortlachMulti1MlaLongIdx(_)
            | Self::MortlachMulti2MlaLongLongIdxS(_)
            | Self::MortlachMulti2ZzaIdxS(_)
            | Self::MortlachMulti2FmaLongIdx(_)
            | Self::MortlachMulti2MlaLongIdx(_)
            | Self::MortlachMulti4MlaLongLongIdxS(_)
            | Self::MortlachMulti4ZzaIdxS(_)
            | Self::MortlachMulti4FmaLongIdx(_)
            | Self::MortlachMulti4MlaLongIdx(_)
            | Self::MortlachBini32Prod(_)
            | Self::MortlachI16i32Prod(_)
            | Self::MortlachMulti2InsertCtg(_)
            | Self::MortlachMulti4InsertCtg(_)
            | Self::MortlachMulti2ZaInsertCtg(_)
            | Self::MortlachMulti4ZaInsertCtg(_)
            | Self::MortlachMulti2ExtractCtg(_)
            | Self::MortlachMulti4ExtractCtg(_)
            | Self::MortlachMulti2ZaExtractCtg(_)
            | Self::MortlachMulti4ZaExtractCtg(_)
            | Self::MortlachExpand1dst(_)
            | Self::MortlachExpand2dstCtg(_)
            | Self::MortlachExpand4dstCtg(_)
            | Self::SveIntxDot2(_)
            | Self::SveIntxDot2ByIndexedElem(_)
            | Self::SveIntxMultiShiftNarrow(_)
            | Self::SveIntxMultiExtractNarrow(_)
            | Self::SveFpClamp(_) => Feature::Sme2,
            Self::SveFpFdotByIndexedElem(e) => e.feature(),
            Self::SveFpFmaLongByIndexedElem(e) => e.feature(),
            Self::SveFpFdot(e) => e.feature(),
            Self::SveFpFmaLong(e) => e.feature(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::features::Implementation;
    use crate::cpu::{Cpu, CpuConfig, Levels, NoEl2, SysReg, WithEl2};
    use std::vec::Vec;

    /// A processor of the levels `L` implementing `imp`, as built (EL2 its
    /// levels' to decide), and its ID register space as a host reads it:
    /// `op0` 3, `op1` 0, `CRn` 0, indexed by `CRm` and `op2`, `u64::MAX`
    /// where a read is refused.
    fn id_registers<L: Levels>(imp: Implementation) -> (Implementation, [[u64; 8]; 8]) {
        let cpu = Cpu::<L>::implementing(
            CpuConfig {
                ips: rusty_box_core::ClockHz::BOCHS_DEFAULT,
                reset_pc: 0,
            },
            imp,
        );
        let mut ids = [[u64::MAX; 8]; 8];
        for (crm, row) in (0u8..).zip(ids.iter_mut()) {
            for (op2, id) in (0u8..).zip(row.iter_mut()) {
                if let Some(reg) = SysReg::new(3, 0, 0, crm, op2) {
                    *id = cpu.sysreg(reg).unwrap_or(u64::MAX);
                }
            }
        }
        (cpu.imp, ids)
    }

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

        let models = [
            Implementation::CORTEX_A57,
            Implementation::ARMV9P2,
            Implementation::EVERY_FEATURE,
        ];
        let built = models
            .iter()
            .flat_map(|&imp| [id_registers::<NoEl2>(imp), id_registers::<WithEl2>(imp)]);
        for (imp, ids) in built {
            let read = |crm: u8, op2: u8| ids[usize::from(crm)][usize::from(op2)];
            let field = |value: u64, lsb: u32| (value >> lsb) & 0xF;
            let (pfr0, pfr1) = (read(4, 0), read(4, 1));
            let (zfr0, smfr0) = (read(4, 4), read(4, 5));
            let (isar0, isar1, isar2, isar3) = (read(6, 0), read(6, 1), read(6, 2), read(6, 3));
            let (mmfr0, mmfr1, mmfr2) = (read(7, 0), read(7, 1), read(7, 2));
            let dfr0 = read(5, 0);
            assert_eq!(read(0, 0), imp.midr, "MIDR_EL1 is the implementation's");
            for feature in Feature::ALL {
                let advertised = match feature {
                    Feature::Base => true,
                    Feature::Fp => field(pfr0, 16) != 0xF,
                    Feature::AdvSimd => field(pfr0, 20) != 0xF,
                    Feature::Aes => field(isar0, 4) != 0,
                    Feature::Pmull => field(isar0, 4) >= 2,
                    // `SInt(DoubleLock) >= 0`: 0b0xxx.
                    Feature::DoubleLock => field(dfr0, 36) < 8,
                    Feature::Sha1 => field(isar0, 8) != 0,
                    Feature::Sha256 => field(isar0, 12) != 0,
                    Feature::Crc32 => field(isar0, 16) != 0,
                    Feature::Sme => field(pfr1, 24) >= 1,
                    Feature::Sme2 => field(pfr1, 24) >= 2,
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
                    Feature::RasV1p1 | Feature::RasSaV1p1 => {
                        field(pfr0, 28) >= 2 || (field(pfr0, 28) == 1 && field(pfr1, 12) >= 1)
                    }
                    Feature::Debugv8p1 => field(dfr0, 0) >= 7,
                    Feature::Debugv8p2 => field(dfr0, 0) >= 8,
                    Feature::Debugv8p4 => field(dfr0, 0) >= 9,
                    Feature::Afp => field(mmfr1, 44) >= 1,
                    Feature::Rpres => field(isar2, 4) >= 1,
                    // `Features.json`: `FEAT_EL2 <=> EL2 >= 1`, and
                    // `FEAT_AA64EL2` the same field's AArch64 support.
                    Feature::El2 | Feature::Aa64El2 => field(pfr0, 8) >= 1,
                };
                assert_eq!(advertised, imp.has(feature), "{feature:?} on {imp:?}");
            }
            // FP = AdvSIMD = 1 with half precision, 0 without; every other
            // instruction-set feature is a nonzero field of these registers.
            let fp16 = u64::from(imp.has(Feature::Fp16));
            assert_eq!(
                (field(pfr0, 16), field(pfr0, 20)),
                (fp16, fp16),
                "FP and AdvSIMD"
            );
            // PFR0: EL0 and EL1 (AArch64 only), EL2 (AArch64 only) with
            // FEAT_EL2, FP, AdvSIMD, RAS, DIT, CSV2 = 1 (no FEAT_CSV2_2) and
            // CSV3; nothing else.
            let el2 = u64::from(imp.has(Feature::El2));
            assert_eq!(field(pfr0, 8), el2, "EL2: AArch64 only, or absent");
            assert_eq!(
                pfr0 & !((0xF << 8)
                    | (0xFF << 16)
                    | (0xF << 28)
                    | (0xF << 48)
                    | (0xF << 56)
                    | (0xF << 60)),
                0x11
            );
            // DFR0: DebugVer, two breakpoints and two watchpoints, no PMU,
            // and DoubleLock 0 or 0b1111.
            assert_eq!(dfr0 & !0xF & !(0xF << 36), 0x0010_1000);
            assert!(matches!(field(dfr0, 36), 0 | 0xF), "DoubleLock");
            assert!(
                (6..=9).contains(&field(dfr0, 0)),
                "DebugVer: Armv8.0 to FEAT_Debugv8p4"
            );
            assert!(field(pfr0, 56) <= 1, "CSV2 = 1: FEAT_CSV2, not FEAT_CSV2_2");
            // ID_AA64ISAR0_EL1: AES = 2 (AES and the 64-bit PMULL), SHA1 =
            // 1, SHA2 = 1 (SHA-256, not SHA-512), CRC32 = 1, Atomic = 2 with
            // FEAT_LSE, RDM = 1, DP = 1, FHM = 1, TS and TLB; nothing else.
            // ISAR1 holds DPB, APA, JSCVT, FCMA, LRCPC, GPA, FRINTTS, SB,
            // SPECRES = 1 (no FEAT_SPECRES2), BF16 = 1 (no FEAT_EBF16), I8MM
            // and XS alone (no API or GPI: no IMPLEMENTATION DEFINED
            // algorithm, and APA is never 2, FEAT_EPAC), ISAR2 WFxT = 2
            // alone, MMFR0 the 4 KiB granule's fields (with EL2, TGranX_2 =
            // 1: none at stage 2) and ECV = 1 (no FEAT_ECV_POFF), MMFR1
            // HPDS = 1 (no FEAT_HPDS2), LO and PAN
            // alone, MMFR2 CnP, UAO, AT, IDS, TTL and E0PD alone. MMFR1
            // AFP and ISAR2 RPRES are each 1 or 0.
            let at = |feature: Feature, lsb: u32, value: u64| {
                if imp.has(feature) {
                    value << lsb
                } else {
                    0
                }
            };
            let isar0_later = at(Feature::Lse, 20, 2)
                | at(Feature::Rdm, 28, 1)
                | at(Feature::DotProd, 44, 1)
                | at(Feature::Fhm, 48, 1);
            assert_eq!(
                isar0 & !((0xF << 52) | (0xF << 56)),
                0x0000_0000_0001_1120 | isar0_later
            );
            let isar1_fields = [0, 4, 12, 16, 20, 24, 32, 36, 40, 44, 52, 56]
                .into_iter()
                .fold(0, |acc, lsb| acc | (0xF << lsb));
            assert_eq!(isar1 & !isar1_fields, 0);
            assert_ne!(
                field(isar1, 4),
                2,
                "APA = 2 is FEAT_EPAC, which FEAT_PAuth2 excludes"
            );
            assert!(
                field(isar1, 40) <= 1,
                "SPECRES = 1: FEAT_SPECRES, not FEAT_SPECRES2"
            );
            assert!(field(isar1, 44) <= 1, "BF16 = 1: FEAT_BF16, not FEAT_EBF16");
            let stage_2_granules =
                at(Feature::El2, 40, 2) | at(Feature::El2, 36, 1) | at(Feature::El2, 32, 1);
            assert_eq!(
                mmfr0 & !(0xF << 60),
                0x0F00_0024 | stage_2_granules,
                "PARange 44 bits, 16-bit ASIDs, the 4 KiB granule alone, at each stage"
            );
            // `Features.json`: FEAT_GTG holds exactly when each of TGran4_2,
            // TGran16_2 and TGran64_2 is non-zero, so they are all or none.
            let stage_2_fields = [field(mmfr0, 40), field(mmfr0, 32), field(mmfr0, 36)];
            assert!(
                stage_2_fields.iter().all(|&f| f >= 1) || stage_2_fields.iter().all(|&f| f == 0),
                "{stage_2_fields:?}"
            );
            assert!(
                field(mmfr0, 60) <= 1,
                "ECV = 1: FEAT_ECV, not FEAT_ECV_POFF"
            );
            assert!(field(mmfr1, 12) <= 1, "HPDS = 1: FEAT_HPDS, not FEAT_HPDS2");
            assert_eq!(
                mmfr1 & !((0xF << 12) | (0xF << 16) | (0xF << 20)),
                at(Feature::Afp, 44, 1)
            );
            assert_eq!(
                mmfr2 & !(0xF | (0xF << 4) | (0xF << 32) | (0xF << 36) | (0xF << 48) | (0xF << 60)),
                0
            );
            assert_eq!(
                (isar2, isar3),
                (at(Feature::WfxT, 0, 2) | at(Feature::Rpres, 4, 1), 0)
            );
            // No SVE2 (SVEver = 0, with FEAT_SME as without); with FEAT_SME,
            // BF16 and I8MM report FEAT_BF16 and FEAT_I8MM as ISAR1 does.
            let zfr0_later = if imp.has(Feature::Sme) {
                (field(isar1, 44) << 20) | (field(isar1, 52) << 44)
            } else {
                0
            };
            assert_eq!(zfr0, zfr0_later);
            // PFR1: BT = 1 (FEAT_BTI, not FEAT_BTIE), SSBS and SME alone.
            assert_eq!(pfr1 & !((0xF << 24) | (0xF << 4) | 0xF), 0);
            assert!(field(pfr1, 0) <= 1, "BT = 1: FEAT_BTI, not FEAT_BTIE");
            match (imp.has(Feature::Sme), imp.has(Feature::Sme2)) {
                (true, false) => {
                    assert_eq!(field(pfr1, 24), 1, "SME = 1: FEAT_SME, not FEAT_SME2");
                    assert_eq!(
                        smfr0, 0x0000_00FD_0000_0000,
                        "F32F32, B16F32, F16F32, I8I32; SMEver 0"
                    );
                }
                (true, true) => {
                    assert_eq!(field(pfr1, 24), 2, "SME = 2: FEAT_SME2");
                    assert_eq!(
                        smfr0, 0x0100_50FF_0000_0000,
                        "F32F32, BI32I32, B16F32, F16F32, I8I32, I16I32 = 0b0101; SMEver 1"
                    );
                }
                (false, _) => assert_eq!((field(pfr1, 24), smfr0), (0, 0)),
            }
        }
    }
}
