//! The A64 encodings this processor decodes, and what each one is to it.
//!
//! An instruction word is classified by its *encoding*: the bits the Arm ARM
//! fixes for that instruction form, every other bit being an operand field.
//! `encodings.txt`, beside this file, lists the encodings this interpreter
//! knows, grouped under the encoding class of the Arm ARM's A64 encoding
//! index (DDI 0487, chapter C4) each belongs to, with that class's bit
//! diagram. `cargo xtask a64-decode` checks every row against its class's
//! diagram and compiles the list into `table.rs`: the same rows as a static
//! table, and a decision tree that finds, for any word, the matching encoding
//! with the most fixed bits. `cargo xtask ci` fails while `table.rs` is not
//! what the list compiles to.
//!
//! The classification stops at the encoding class. The handler for a class
//! reads every operand field from the word itself, following the field
//! layout and decode pseudocode of the Arm ARM, and raises UNDEFINED for the
//! reserved values its pseudocode names. A class is the unit of dispatch.

mod table;

pub(crate) use table::lookup;
#[cfg(test)]
pub(crate) use table::ENCODINGS;

/// An architectural feature, by the Arm ARM's `FEAT_` name: what an encoding
/// belongs to, and what the ID registers advertise.
///
/// Closed (R5): [`Feature::advertised`] answers for every variant, and the ID
/// registers in [`super::id`] are checked against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Feature {
    /// The ARMv8.0 base instruction set.
    Base,
    /// Scalar floating point (ARMv8.0).
    Fp,
    /// Advanced SIMD (ARMv8.0).
    AdvSimd,
    /// FEAT_AES: AESE, AESD, AESMC, AESIMC.
    Aes,
    /// FEAT_PMULL: PMULL and PMULL2 on 64-bit elements.
    Pmull,
    /// FEAT_SHA1: SHA1C, SHA1P, SHA1M, SHA1H, SHA1SU0, SHA1SU1.
    Sha1,
    /// FEAT_SHA256: SHA256H, SHA256H2, SHA256SU0, SHA256SU1.
    Sha256,
    /// FEAT_CRC32: CRC32 and CRC32C.
    Crc32,
    /// FEAT_FP16 (Armv8.2): half-precision data processing.
    Fp16,
    /// FEAT_BF16 (Armv8.6): BFCVT.
    Bf16,
    /// FEAT_MTE (Armv8.5): ADDG, SUBG.
    Mte,
    /// FEAT_CSSC (Armv8.9): SMAX, SMIN, UMAX, UMIN (immediate).
    Cssc,
    /// FEAT_GCS (Armv9.4): the guarded control stack instructions.
    Gcs,
    /// FEAT_LSUI (Armv9.6): unprivileged load/store pair.
    Lsui,
    /// FEAT_FlagM (Armv8.4): CFINV.
    FlagM,
    /// FEAT_FlagM2 (Armv8.5): XAFLAG, AXFLAG.
    FlagM2,
    /// FEAT_XS (Armv8.7): DSB with the nXS qualifier.
    Xs,
    /// FEAT_SB (Armv8.5): SB.
    Sb,
    /// FEAT_SME (Armv9.2): SMSTART, SMSTOP.
    Sme,
    /// FEAT_TME (Armv9.0): TSTART, TCOMMIT, TTEST.
    Tme,
    /// FEAT_WFxT (Armv8.7): WFET, WFIT.
    Wfxt,
}

impl Feature {
    /// Whether this processor advertises the feature in its ID registers.
    /// Every [`Class`] belongs to an advertised feature; an encoding of any
    /// other feature is UNDEFINED.
    pub(crate) const fn advertised(self) -> bool {
        match self {
            Self::Base | Self::Fp | Self::AdvSimd => true,
            Self::Aes
            | Self::Pmull
            | Self::Sha1
            | Self::Sha256
            | Self::Crc32
            | Self::Fp16
            | Self::Bf16
            | Self::Mte
            | Self::Cssc
            | Self::Gcs
            | Self::Lsui
            | Self::FlagM
            | Self::FlagM2
            | Self::Xs
            | Self::Sb
            | Self::Sme
            | Self::Tme
            | Self::Wfxt => false,
        }
    }
}

/// An encoding class of the Arm ARM's A64 encoding index (DDI 0487 C4.1): a
/// group of encodings sharing one bit diagram, executed by one handler.
///
/// Closed (R5): dispatch matches every variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Class {
    // Reserved
    /// UDF: permanently undefined.
    PermanentlyUndefined,
    // Data processing -- immediate
    PcRelAddressing,
    AddSubImmediate,
    LogicalImmediate,
    MoveWide,
    Bitfield,
    Extract,
    // Branches, exception generating and system instructions
    ConditionalBranch,
    ExceptionGeneration,
    System,
    BranchRegister,
    BranchImmediate,
    CompareAndBranch,
    TestAndBranch,
    // Loads and stores
    SimdLoadStoreMultiple,
    SimdLoadStoreMultiplePostIndexed,
    SimdLoadStoreSingle,
    SimdLoadStoreSinglePostIndexed,
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
    // Data processing -- register
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
    // Data processing -- scalar floating-point and Advanced SIMD
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
}

impl Class {
    /// The feature this class's encodings belong to. The load/store classes
    /// hold their SIMD&FP-register forms too, as the Arm ARM groups them:
    /// those check `CPACR_EL1.FPEN` when they execute.
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::PermanentlyUndefined
            | Self::PcRelAddressing
            | Self::AddSubImmediate
            | Self::LogicalImmediate
            | Self::MoveWide
            | Self::Bitfield
            | Self::Extract
            | Self::ConditionalBranch
            | Self::ExceptionGeneration
            | Self::System
            | Self::BranchRegister
            | Self::BranchImmediate
            | Self::CompareAndBranch
            | Self::TestAndBranch
            | Self::LoadStoreExclusive
            | Self::LoadLiteral
            | Self::LoadStoreNoAllocatePair
            | Self::LoadStorePairIndexed
            | Self::LoadStorePairOffset
            | Self::LoadStoreUnscaled
            | Self::LoadStoreIndexed
            | Self::LoadStoreUnprivileged
            | Self::LoadStoreRegisterOffset
            | Self::LoadStoreUnsignedImmediate
            | Self::DataProcessing2Source
            | Self::DataProcessing1Source
            | Self::LogicalShiftedRegister
            | Self::AddSubShiftedRegister
            | Self::AddSubExtendedRegister
            | Self::AddSubWithCarry
            | Self::ConditionalCompareRegister
            | Self::ConditionalCompareImmediate
            | Self::ConditionalSelect
            | Self::DataProcessing3Source => Feature::Base,
            Self::SimdLoadStoreMultiple
            | Self::SimdLoadStoreMultiplePostIndexed
            | Self::SimdLoadStoreSingle
            | Self::SimdLoadStoreSinglePostIndexed
            | Self::SimdScalarCopy
            | Self::SimdScalarThreeSame
            | Self::SimdScalarTwoRegisterMisc
            | Self::SimdScalarPairwise
            | Self::SimdScalarThreeDifferent
            | Self::SimdScalarShiftImmediate
            | Self::SimdScalarIndexedElement
            | Self::SimdTableLookup
            | Self::SimdPermute
            | Self::SimdExtract
            | Self::SimdCopy
            | Self::SimdThreeSame
            | Self::SimdTwoRegisterMisc
            | Self::SimdAcrossLanes
            | Self::SimdThreeDifferent
            | Self::SimdModifiedImmediate
            | Self::SimdShiftImmediate
            | Self::SimdIndexedElement => Feature::AdvSimd,
            Self::FpFixedPointConversion
            | Self::FpIntegerConversion
            | Self::FpDataProcessing1Source
            | Self::FpCompare
            | Self::FpImmediate
            | Self::FpConditionalCompare
            | Self::FpDataProcessing2Source
            | Self::FpConditionalSelect
            | Self::FpDataProcessing3Source => Feature::Fp,
        }
    }
}

/// What an encoding is to this interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    /// The HINT space (`D503201F | CRm:op2 << 5`): every encoding in it
    /// executes, as a NOP when the hint is not one this core implements —
    /// including the Armv8.3 pointer authentication and Armv8.5 BTI hints,
    /// which is what lets code built with branch protection run on an
    /// ARMv8.0 core.
    Hint,
    /// Executes through this class's handler, when the class's feature is
    /// advertised.
    Class(Class),
    /// An encoding of a feature this interpreter does not implement: always
    /// UNDEFINED. Listed where it takes precedence over an ARMv8.0 encoding
    /// that shares its fixed bits, or where a feature this processor may
    /// advertise is named before it has a handler.
    Absent(Feature),
}

/// One encoding: a word is an instance of it when `word & mask == opcode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Encoding {
    pub(crate) opcode: u32,
    pub(crate) mask: u32,
    /// The Arm ARM mnemonic, for diagnostics.
    pub(crate) mnemonic: &'static str,
    pub(crate) kind: Kind,
}

impl Encoding {
    pub(crate) const fn new(opcode: u32, mask: u32, mnemonic: &'static str, kind: Kind) -> Self {
        Self {
            opcode,
            mask,
            mnemonic,
            kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::{Cpu, CpuConfig, SysReg};
    use std::vec::Vec;

    /// Every feature an encoding in the list belongs to.
    fn features_in_the_list() -> Vec<Feature> {
        let mut features: Vec<Feature> = Vec::new();
        for e in ENCODINGS.iter() {
            let feature = match e.kind {
                Kind::Class(class) => class.feature(),
                Kind::Absent(feature) => feature,
                Kind::Hint => Feature::Base,
            };
            if !features.contains(&feature) {
                features.push(feature);
            }
        }
        features
    }

    /// Whether the ID registers, read the way an EL1 guest reads them,
    /// advertise `feature` (Arm ARM D19.2, the `ID_AA64*_EL1` field
    /// descriptions).
    fn id_registers_advertise(cpu: &Cpu, feature: Feature) -> bool {
        let read = |crm: u8, op2: u8| {
            SysReg::new(3, 0, 0, crm, op2)
                .map(|reg| cpu.sysreg(reg).unwrap_or(0))
                .unwrap_or(0)
        };
        let field = |value: u64, lsb: u32| (value >> lsb) & 0xF;
        let (pfr0, pfr1) = (read(4, 0), read(4, 1));
        let (isar0, isar1, isar2, isar3) = (read(6, 0), read(6, 1), read(6, 2), read(6, 3));
        match feature {
            Feature::Base => true,
            Feature::Fp => field(pfr0, 16) != 0xF,
            Feature::AdvSimd => field(pfr0, 20) != 0xF,
            Feature::Fp16 => field(pfr0, 16) == 1 && field(pfr0, 20) == 1,
            Feature::Aes => field(isar0, 4) >= 1,
            Feature::Pmull => field(isar0, 4) >= 2,
            Feature::Sha1 => field(isar0, 8) >= 1,
            Feature::Sha256 => field(isar0, 12) >= 1,
            Feature::Crc32 => field(isar0, 16) >= 1,
            Feature::Tme => field(isar0, 24) >= 1,
            Feature::FlagM => field(isar0, 52) >= 1,
            Feature::FlagM2 => field(isar0, 52) >= 2,
            Feature::Sb => field(isar1, 36) >= 1,
            Feature::Bf16 => field(isar1, 44) >= 1,
            Feature::Xs => field(isar1, 56) >= 1,
            Feature::Wfxt => field(isar2, 0) >= 2,
            Feature::Cssc => field(isar2, 52) >= 1,
            Feature::Mte => field(pfr1, 8) >= 1,
            Feature::Sme => field(pfr1, 24) >= 1,
            Feature::Gcs => field(pfr1, 44) >= 1,
            // ID_AA64ISAR3_EL1 holds only Armv9.4+ fields: any nonzero
            // value would advertise something this core lacks.
            Feature::Lsui => isar3 != 0,
        }
    }

    /// The decoder executes exactly the features the ID registers advertise,
    /// for every feature the encoding list names: a capability and the
    /// register that reports it change together.
    #[test]
    fn the_id_registers_advertise_exactly_what_the_decoder_executes() {
        let cpu = Cpu::new(CpuConfig {
            ips: rusty_box_core::ClockHz::BOCHS_DEFAULT,
            counter_hz: CpuConfig::QEMU_VIRT_COUNTER_HZ,
            reset_pc: 0,
        });
        let features = features_in_the_list();
        assert!(features.len() > 3, "the list names its absent features too");
        for feature in features {
            assert_eq!(
                feature.advertised(),
                id_registers_advertise(&cpu, feature),
                "{feature:?}"
            );
        }
    }

    /// Every class belongs to an advertised feature; an encoding of a
    /// feature not advertised is listed as absent, never as a class.
    #[test]
    fn every_class_in_the_list_belongs_to_an_advertised_feature() {
        for e in ENCODINGS.iter() {
            match e.kind {
                Kind::Class(class) => assert!(class.feature().advertised(), "{} {:?}", e.mnemonic, class),
                Kind::Absent(feature) => assert!(!feature.advertised(), "{} {:?}", e.mnemonic, feature),
                Kind::Hint => {}
            }
        }
    }
}
