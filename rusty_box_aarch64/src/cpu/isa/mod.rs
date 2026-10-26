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

use asimd::{Asimdall, Asimddiff, Asimdelem, Asimdext, Asimdimm, Asimdins, Asimdmisc, Asimdperm, Asimdsame, Asimdshf, Asimdtbl};
use asisd::{Asisddiff, Asisdelem, Asisdmisc, Asisdone, Asisdpair, Asisdsame, Asisdshf};
use atomic::{Comswap, Comswappr, LdapstlUnscaled, Memop};
use control::{Barriers, BranchImm, BranchReg, Compbranch, Condbranch, Exception, Hints, Pstate, Systeminstrs, Systemmove, Testbranch};
use crypto::{Cryptoaes, Cryptosha2, Cryptosha3};
use dpimm::{AddsubImm, Bitfield, Extract, LogImm, Movewide, Pcreladdr};
use dpreg::{AddsubCarry, AddsubExt, AddsubShift, CondcmpImm, CondcmpReg, Condsel, Dp1src, Dp2src, Dp3src, LogShift};
use float::{Float2fix, Float2int, Floatccmp, Floatcmp, Floatdp1, Floatdp2, Floatdp3, Floatimm, Floatsel};
use ldst::{
    Asisdlse, Asisdlsep, Asisdlso, Asisdlsop, LdstImmpost, LdstImmpre, LdstPos, LdstRegoff, LdstUnpriv, LdstUnscaled,
    Ldstexclp, Ldstexclr, LdstnapairOffs, LdstpairOff, LdstpairPost, LdstpairPre, Ldstord, Loadlit,
};
use reserved::PermUndef;
use sve::{SveIntArithSvl, SveIntReadSvlA};

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
    SveIntArithSvl(SveIntArithSvl),
    SveIntReadSvlA(SveIntReadSvlA),
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
            Self::Hints(_) => Class::Hint,
            Self::Barriers(_) | Self::Pstate(_) | Self::Systeminstrs(_) | Self::Systemmove(_) => Class::System,
            Self::BranchReg(_) => Class::BranchRegister,
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
            Self::Memop(_) => Class::AtomicMemoryOperation,
            Self::Comswap(_) => Class::CompareAndSwap,
            Self::Comswappr(_) => Class::CompareAndSwapPair,
            Self::LdapstlUnscaled(_) => Class::LoadStoreOrderedUnscaled,
            Self::Dp2src(e) => e.class(),
            Self::Dp1src(_) => Class::DataProcessing1Source,
            Self::LogShift(_) => Class::LogicalShiftedRegister,
            Self::AddsubShift(_) => Class::AddSubShiftedRegister,
            Self::AddsubExt(_) => Class::AddSubExtendedRegister,
            Self::AddsubCarry(_) => Class::AddSubWithCarry,
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
            Self::Asisdshf(_) => Class::SimdScalarShiftImmediate,
            Self::Asisdelem(_) => Class::SimdScalarIndexedElement,
            Self::Asimdtbl(_) => Class::SimdTableLookup,
            Self::Asimdperm(_) => Class::SimdPermute,
            Self::Asimdext(_) => Class::SimdExtract,
            Self::Asimdins(_) => Class::SimdCopy,
            Self::Asimdmisc(_) => Class::SimdTwoRegisterMisc,
            Self::Asimdall(_) => Class::SimdAcrossLanes,
            Self::Asimddiff(_) => Class::SimdThreeDifferent,
            Self::Asimdsame(_) => Class::SimdThreeSame,
            Self::Asimdimm(_) => Class::SimdModifiedImmediate,
            Self::Asimdshf(_) => Class::SimdShiftImmediate,
            Self::Asimdelem(_) => Class::SimdIndexedElement,
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
    /// this core implements, including the Armv8.3 pointer authentication
    /// and Armv8.5 BTI hints, which is what lets code built with branch
    /// protection run on an ARMv8.0 core.
    Hint,
    /// Barriers, PSTATE, the system instructions and the system register
    /// moves.
    System,
    BranchRegister,
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
    DataProcessing2Source,
    DataProcessing1Source,
    LogicalShiftedRegister,
    AddSubShiftedRegister,
    AddSubExtendedRegister,
    AddSubWithCarry,
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
}

impl Encoding {
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
            | Self::Hints(_)
            | Self::Barriers(_)
            | Self::Pstate(_)
            | Self::Systeminstrs(_)
            | Self::Systemmove(_)
            | Self::BranchReg(_)
            | Self::BranchImm(_)
            | Self::Compbranch(_)
            | Self::Testbranch(_)
            | Self::Ldstexclp(_)
            | Self::Ldstexclr(_)
            | Self::Ldstord(_)
            | Self::LdstUnpriv(_)
            | Self::Dp1src(_)
            | Self::LogShift(_)
            | Self::AddsubShift(_)
            | Self::AddsubExt(_)
            | Self::AddsubCarry(_)
            | Self::CondcmpReg(_)
            | Self::CondcmpImm(_)
            | Self::Condsel(_)
            | Self::Dp3src(_) => Feature::Base,
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
            | Self::Asisdpair(_)
            | Self::Asisddiff(_)
            | Self::Asisdsame(_)
            | Self::Asisdshf(_)
            | Self::Asisdelem(_)
            | Self::Asimdtbl(_)
            | Self::Asimdperm(_)
            | Self::Asimdext(_)
            | Self::Asimdins(_)
            | Self::Asimdmisc(_)
            | Self::Asimdall(_)
            | Self::Asimddiff(_)
            | Self::Asimdsame(_)
            | Self::Asimdimm(_)
            | Self::Asimdshf(_)
            | Self::Asimdelem(_) => Feature::AdvSimd,
            Self::Float2fix(_)
            | Self::Float2int(_)
            | Self::Floatdp1(_)
            | Self::Floatcmp(_)
            | Self::Floatimm(_)
            | Self::Floatccmp(_)
            | Self::Floatdp2(_)
            | Self::Floatsel(_)
            | Self::Floatdp3(_) => Feature::Fp,
            Self::SveIntArithSvl(_) | Self::SveIntReadSvlA(_) => Feature::Sme,
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
            let mmfr2 = read(7, 2);
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
                };
                assert_eq!(advertised, imp.has(feature), "{feature:?} on {imp:?}");
            }
            // Half precision would be FP = AdvSIMD = 1; every other
            // instruction-set feature is a nonzero field of these registers.
            assert_eq!((field(pfr0, 16), field(pfr0, 20)), (0, 0), "FP and AdvSIMD, without half precision");
            // ID_AA64ISAR0_EL1: AES = 2 (AES and the 64-bit PMULL), SHA1 =
            // 1, SHA2 = 1 (SHA-256, not SHA-512), CRC32 = 1, Atomic = 2 with
            // FEAT_LSE; nothing else. ISAR1 holds LRCPC alone, MMFR2 AT alone.
            let lse = if imp.has(Feature::Lse) { 2 << 20 } else { 0 };
            assert_eq!(isar0, 0x0000_0000_0001_1120 | lse);
            assert_eq!(isar1 & !(0xF << 20), 0);
            assert_eq!(mmfr2 & !(0xF << 32), 0);
            assert_eq!((isar2, isar3), (0, 0));
            // No SVE2 (SVEver = 0, with FEAT_SME as without), and SME's
            // required fields only with FEAT_SME.
            assert_eq!(zfr0, 0);
            if imp.has(Feature::Sme) {
                assert_eq!(pfr1, 1 << 24, "SME = 1: FEAT_SME, not FEAT_SME2");
                assert_eq!(smfr0, 0x0000_00FD_0000_0000, "F32F32, B16F32, F16F32, I8I32; SMEver 0");
            } else {
                assert_eq!((pfr1, smfr0), (0, 0));
            }
        }
    }
}
