//! Identification values a guest can read, and the feature set they promise.
//!
//! Each processor model ([`CpuModel`](super::CpuModel)) is cut down to what
//! this interpreter implements: AArch64 at EL0 and EL1, and at EL2 where the
//! machine asks for it (FEAT_EL2), no PMU. Of the translation granules, the
//! interpreter walks all three; the Armv9.2 model offers them all, and the
//! Cortex-A57 the 4 KiB one alone, the others its TRM's to say
//! (`Features::CORTEX_A57`). The Armv9.2 model's physical addresses are 52
//! bits (FEAT_LPA), the Cortex-A57's 44
//! ([`CpuModel::physical_address_bits`](super::CpuModel::physical_address_bits));
//! its virtual addresses are 52 bits with the 64 KiB granule (FEAT_LVA), 48
//! otherwise and on the Cortex-A57.
//! The Cortex-A57 r1p3 (its TRM's revision, DDI 0488H, in `MIDR_EL1`) has
//! FP and Advanced SIMD, the
//! Cryptographic Extension's AES, PMULL, SHA-1 and SHA-256 and CRC32 (the
//! Cortex-A57's optional extensions, all of them, as QEMU's `cortex-a57`
//! has them); the Armv9.2 model has those and the rest of its feature set
//! (`Features::ARMV9P2`), and reports itself as a software model. Each
//! describes its own caches ([`Caches`]), both with 64-byte lines in an L1
//! data and instruction cache and a unified L2. Every value here is consistent with
//! that cut — an ID register that advertised a feature the executor rejects
//! would make the feature unreachable (or worse, reachable and wrong), so
//! the two change together. The registers that report instruction-set
//! features are functions of the processor's feature set, which the
//! decoder reads too.

use super::features::{Feature, Features};

/// The Cortex-A57's `MIDR_EL1`: Arm Ltd, variant 1, architecture 0xF, part
/// 0xD07 (Cortex-A57), revision 3 — r1p3, the revision the TRM describes
/// (DDI 0488H, Table 4-1 and 4.3.1). Of the registers the TRM's product
/// revisions (1.8) change from r1p0 to r1p3, `MIDR_EL1` is the only one
/// this model has: `TRCIDR1`, the Peripheral ID registers and
/// `CPUACTLR_EL1` are the trace unit's, the external debug interface's and
/// the IMPLEMENTATION DEFINED space's, none of which it implements.
pub const MIDR_CORTEX_A57: u64 = 0x411F_D073;

/// A software model's `MIDR_EL1`: implementer 0 (reserved for software
/// use), variant 0, architecture 0xF (the ID registers identify each
/// feature), part 0x052 (`'R'`, telling this model apart from QEMU's `max`,
/// which is part `'Q'`), revision 0. QEMU `aarch64_max_tcg_initfn` builds
/// its own the same way, so that no guest takes the processor for an Arm
/// part and applies that part's errata workarounds.
pub const MIDR_SOFTWARE_MODEL: u64 = 0x000F_0520;

/// `MPIDR_EL1`: bit 31 RES1, uniprocessor bit clear, affinity 0.0.0.0.
pub const MPIDR_EL1: u64 = 0x8000_0000;

/// `REVIDR_EL1`: no revision-specific fixes to report, the Cortex-A57's
/// reset value (DDI 0488H, Table 4-1), which the TRM leaves to each
/// implementation of the core (4.3.3); the Armv9.2 model's the same.
pub const REVIDR_EL1: u64 = 0;

/// `AIDR_EL1`: zero, the Cortex-A57's, which does not implement it (DDI
/// 0488H, 4.3.24: RES0); the Armv9.2 model, a software model, has no
/// auxiliary identification to give either.
pub(crate) const AIDR_EL1: u64 = 0;

/// `ID_AA64PFR0_EL1` of a processor implementing `features`: EL0 = 1 and
/// EL1 = 1 (AArch64 only), EL2 (11:8) = 1 with FEAT_EL2 (AArch64 only) and
/// 0 without, EL3 = 0 (not implemented), FP (19:16) and
/// AdvSIMD (23:20) 1 with FEAT_FP16 (half-precision arithmetic) and 0
/// without, GIC = 0 (no system-register interface: the GIC is a
/// memory-mapped v2), RAS (31:28) = 1 with FEAT_RAS and 2 with FEAT_RASv1p1
/// as well (which `Features.json` makes imply FEAT_RAS; with no EL3, 2
/// promises no FEAT_DoubleFault), DIT (51:48) = 1 with FEAT_DIT, CSV2
/// (59:56) = 1 with FEAT_CSV2 (2 would be FEAT_CSV2_2, which has the
/// `SCXTNUM_ELx` registers), CSV3 (63:60) = 1 with FEAT_CSV3.
pub(crate) const fn id_aa64pfr0(features: Features) -> u64 {
    let ras = chain_level(features, &[Feature::Ras, Feature::RasV1p1]);
    0x11 | field(features, Feature::El2, 8, 1)
        | field(features, Feature::Fp16, 16, 1)
        | field(features, Feature::Fp16, 20, 1)
        | (ras << 28)
        | field(features, Feature::Dit, 48, 1)
        | field(features, Feature::Csv2, 56, 1)
        | field(features, Feature::Csv3, 60, 1)
}

/// `ID_AA64PFR1_EL1` of a processor implementing `features`: `BT` (bits
/// 3:0) = 1 with FEAT_BTI (2 would be FEAT_BTIE), `SSBS` (7:4) = 1 with
/// FEAT_SSBS and 2 with FEAT_SSBS2 as well (which `Features.json` makes
/// imply FEAT_SSBS), `SME` (bits 27:24) = 1 with FEAT_SME and 2 with
/// FEAT_SME2 as well (`Features.json`: `FEAT_SME <=>
/// UInt(ID_AA64PFR1_EL1.SME) >= 1`, `FEAT_SME2 <=> ... >= 2`, and FEAT_SME2
/// implies FEAT_SME); nothing else from v8.1+.
pub(crate) const fn id_aa64pfr1(features: Features) -> u64 {
    let ssbs = chain_level(features, &[Feature::Ssbs, Feature::Ssbs2]);
    let sme = chain_level(features, &[Feature::Sme, Feature::Sme2]);
    field(features, Feature::Bti, 0, 1) | (ssbs << 4) | (sme << 24)
}

/// `ID_AA64SMFR0_EL1` of a processor implementing `features`: with
/// FEAT_SME, the fields `Features.json` requires of it, F32F32 (bit 32),
/// B16F32 (34), F16F32 (35) and I8I32 (39:36, 0b1111); with FEAT_SME2 as
/// well, those it requires of FEAT_SME2, SMEver (59:56) = 1, BI32I32 (33)
/// and I16I32 (47:44, 0b0101); no FA64, F64F64 or I16I64; zero without
/// FEAT_SME.
pub(crate) const fn id_aa64smfr0(features: Features) -> u64 {
    let sme = (1 << 32) | (1 << 34) | (1 << 35) | (0xF << 36);
    let sme2 = (1 << 56) | (1 << 33) | (0b0101 << 44);
    match (features.has(Feature::Sme), features.has(Feature::Sme2)) {
        (true, true) => sme | sme2,
        (true, false) => sme,
        (false, _) => 0,
    }
}

/// `SMIDR_EL1` (FEAT_SME), each field the board's own choice of those the
/// register's description (Arm ARM D24.2.186) leaves IMPLEMENTATION
/// DEFINED: `SH` (14:13) 0b10, the processor's Streaming SVE mode not
/// shared with another — said in its own field, the processor executing
/// streaming instructions itself, where 0b00 would leave `Affinity` to say
/// it; `Affinity` and `Affinity2` 0 and `NSMC` 0, as an unshared
/// implementation has them; `Implementer` (31:24) 0, reserved for software
/// use, and `Revision` (23:16) 0, those of the model's `MIDR_EL1`
/// ([`MIDR_SOFTWARE_MODEL`]); `SMPS` (15) 0, no streaming execution
/// priority, so `SMPRI_EL1` is RES0 and `HIP` RES0.
pub(crate) const SMIDR_EL1: u64 = 0b10 << 13;

/// `ID_AA64DFR0_EL1` of a processor implementing `features`: DebugVer
/// (3:0) = 6 (Armv8.0 debug), 7 with FEAT_Debugv8p1, 8 with FEAT_Debugv8p2
/// and 9 with FEAT_Debugv8p4 (each of which `Features.json` makes imply the
/// one before), BRPs = 1 and WRPs = 1 (two of each, the architectural
/// minimum), CTX_CMPs = 0, PMUVer = 0 (no PMU), and DoubleLock (39:36) 0
/// with FEAT_DoubleLock and 0b1111 without (`FEAT_DoubleLock <=>
/// SInt(ID_AA64DFR0_EL1.DoubleLock) >= 0`).
pub(crate) const fn id_aa64dfr0(features: Features) -> u64 {
    let debug = chain_level(
        features,
        &[Feature::Debugv8p1, Feature::Debugv8p2, Feature::Debugv8p4],
    );
    let double_lock = if features.has(Feature::DoubleLock) {
        0
    } else {
        0xF << 36
    };
    0x0000_0000_0010_1000 | double_lock | (6 + debug)
}

/// `DBGAUTHSTATUS_EL1` of a processor implementing `features`, as Arm's
/// `UpdateDbgAuthStatus` sets it for a processor with no Secure state and
/// no EL3, whose security state is always Non-secure: NSID (1:0) = 0b10,
/// Non-secure invasive debug implemented and disabled, the external
/// debugger's `DBGEN` being low with no debugger to raise it; NSNID (3:2) =
/// 0b10 likewise (`DBGEN` and `NIDEN` low), or 0b11 with FEAT_Debugv8p4,
/// under which `ExternalNoninvasiveDebugEnabled` is always TRUE; SID, SNID,
/// RLID, RLNID, RTID and RTNID 0, not implemented.
pub(crate) const fn dbgauthstatus_el1(features: Features) -> u64 {
    0b10_10 | field(features, Feature::Debugv8p4, 2, 1)
}

/// `ERRIDR_EL1` with FEAT_RAS: no error records (NUM = 0), as QEMU's
/// `minimal_ras_reginfo` has it.
pub(crate) const ERRIDR_EL1: u64 = 0;

/// `ID_AA64ISAR0_EL1` of a processor implementing `features`: AES (7:4) = 1
/// with FEAT_AES (AESE, AESD, AESMC, AESIMC) and 2 with FEAT_PMULL (the
/// 64-bit PMULL) as well (`Features.json`: `FEAT_PMULL <=>
/// UInt(ID_AA64ISAR0_EL1.AES) >= 2`, and FEAT_PMULL implies FEAT_AES), SHA1
/// (11:8) = 1, SHA2 (15:12) = 1 (SHA-256 only), CRC32 (19:16) = 1, Atomic
/// (23:20) = 2 with FEAT_LSE, RDM (31:28) = 1, DP (47:44) = 1 with
/// FEAT_DotProd, FHM (51:48) = 1, TS (55:52) = 1 with FEAT_FlagM and 2
/// with FEAT_FlagM2 as well (which `Features.json` makes imply FEAT_FlagM),
/// and TLB (59:56) = 1 with FEAT_TLBIOS and 2 with FEAT_TLBIRANGE as well
/// (which `Features.json` makes imply FEAT_TLBIOS); no SHA-512, SHA3, SM3,
/// SM4 or other field. The Cortex-A57's is 0x11120, as QEMU sets it in
/// `aarch64_aa32_a57_init` (target/arm/cpu-max.c), which
/// `aarch64_a57_initfn` calls.
pub(crate) const fn id_aa64isar0(features: Features) -> u64 {
    let aes = chain_level(features, &[Feature::Aes, Feature::Pmull]);
    let ts = chain_level(features, &[Feature::FlagM, Feature::FlagM2]);
    let tlb = chain_level(features, &[Feature::TlbiOs, Feature::TlbiRange]);
    (aes << 4)
        | field(features, Feature::Sha1, 8, 1)
        | field(features, Feature::Sha256, 12, 1)
        | field(features, Feature::Crc32, 16, 1)
        | field(features, Feature::Lse, 20, 2)
        | field(features, Feature::Rdm, 28, 1)
        | field(features, Feature::DotProd, 44, 1)
        | field(features, Feature::Fhm, 48, 1)
        | (ts << 52)
        | (tlb << 56)
}

/// `ID_AA64ISAR1_EL1` of a processor implementing `features`: JSCVT
/// (15:12) = 1 with FEAT_JSCVT, FCMA (19:16) = 1 with FEAT_FCMA, LRCPC
/// (23:20) = 1 with FEAT_LRCPC, 2 with FEAT_LRCPC2 as well (which Arm's
/// feature model makes imply FEAT_LRCPC, so the field never promises LDAPR
/// to a processor without it), FRINTTS (35:32) = 1 with FEAT_FRINTTS, BF16
/// (47:44) = 1 with FEAT_BF16 (2 would be FEAT_EBF16) and I8MM (55:52) = 1
/// with FEAT_I8MM. With FEAT_PAuth computed by QARMA5, APA (7:4) names the
/// newest of FEAT_PAuth (1), FEAT_PAuth2 (3), FEAT_FPAC (4) and
/// FEAT_FPACCOMBINE (5) the pointer authentication follows, the last two
/// acting only with FEAT_PAuth2 (which `Features.json` makes them imply),
/// and GPA (27:24) = 1 (`FEAT_PACQARMA5 <=> UInt(GPA) >= 1`); API and GPI
/// stay 0, as no IMPLEMENTATION DEFINED algorithm is implemented. SB
/// (39:36) = 1 with FEAT_SB, DPB (3:0) = 1 with FEAT_DPB and 2 with
/// FEAT_DPB2 as well (which `Features.json` makes imply FEAT_DPB), SPECRES
/// (43:40) = 1 with FEAT_SPECRES (2 would be FEAT_SPECRES2, which has COSP
/// RCTX), XS (59:56) = 1 with FEAT_XS. Nothing else from v8.2+.
pub(crate) const fn id_aa64isar1(features: Features) -> u64 {
    let dpb = chain_level(features, &[Feature::Dpb, Feature::Dpb2]);
    let lrcpc = chain_level(features, &[Feature::Lrcpc, Feature::Lrcpc2]);
    let apa = if !(features.has(Feature::PAuth) && features.has(Feature::PacQarma5)) {
        0
    } else if !features.has(Feature::PAuth2) {
        1
    } else if features.has(Feature::FpacCombine) {
        5
    } else if features.has(Feature::Fpac) {
        4
    } else {
        3
    };
    dpb | (apa << 4)
        | field(features, Feature::Jscvt, 12, 1)
        | field(features, Feature::Fcma, 16, 1)
        | (lrcpc << 20)
        | field(features, Feature::PacQarma5, 24, 1)
        | field(features, Feature::Frintts, 32, 1)
        | field(features, Feature::Sb, 36, 1)
        | field(features, Feature::SpecRes, 40, 1)
        | field(features, Feature::Bf16, 44, 1)
        | field(features, Feature::I8mm, 52, 1)
        | field(features, Feature::Xs, 56, 1)
}

/// `ID_AA64ISAR2_EL1` of a processor implementing `features`: WFxT (3:0) =
/// 2 with FEAT_WFxT (`Features.json`: `FEAT_WFxT <=> UInt(WFxT) >= 2`; 1 is
/// reserved), RPRES (7:4) = 1 with FEAT_RPRES; nothing else from v8.7+.
pub(crate) const fn id_aa64isar2(features: Features) -> u64 {
    field(features, Feature::WfxT, 0, 2) | field(features, Feature::Rpres, 4, 1)
}

/// `ID_AA64ZFR0_EL1` of a processor implementing `features`: SVEver = 0
/// (no SVE2), BF16 (23:20) = 1 and I8MM (47:44) = 1 when FEAT_BF16 and
/// FEAT_I8MM come with FEAT_SME (`Features.json`: `(FEAT_SME || FEAT_SVE)
/// ==> (FEAT_BF16 <=> UInt(ID_AA64ZFR0_EL1.BF16) >= 1)`, and the same of
/// I8MM); zero without FEAT_SME, as a processor without SVE or SME reports
/// it. The fields promise the SVE BFloat16 instructions (BFDOT, BFMLALB,
/// BFMLALT, BFCVT, BFCVTNT) and USDOT and SUDOT in Streaming SVE mode,
/// all implemented.
pub(crate) const fn id_aa64zfr0(features: Features) -> u64 {
    if features.has(Feature::Sme) {
        field(features, Feature::Bf16, 20, 1) | field(features, Feature::I8mm, 44, 1)
    } else {
        0
    }
}

/// `ID_AA64MMFR1_EL1` of a processor implementing `features`: HPDS (15:12)
/// = 1 with FEAT_HPDS (2 would be FEAT_HPDS2), LO (19:16) = 1 with
/// FEAT_LOR, PAN (23:20) names the newest of FEAT_PAN (1), FEAT_PAN2 (2)
/// and FEAT_PAN3 (3) the processor implements, each of which
/// `Features.json` makes imply the one before, and AFP (47:44) = 1 with
/// FEAT_AFP; nothing else from v8.1+.
pub(crate) const fn id_aa64mmfr1(features: Features) -> u64 {
    field(features, Feature::Hpds, 12, 1)
        | field(features, Feature::Lor, 16, 1)
        | (chain_level(features, &[Feature::Pan, Feature::Pan2, Feature::Pan3]) << 20)
        | field(features, Feature::Afp, 44, 1)
}

/// `ID_AA64MMFR2_EL1` of a processor implementing `features`: CnP (3:0) = 1
/// with FEAT_TTCNP, UAO (7:4) = 1 with FEAT_UAO, AT (35:32) = 1 with
/// FEAT_LSE2, IDS (39:36) = 1 with FEAT_IDST, TTL (51:48) = 1 with
/// FEAT_TTL, E0PD (63:60) = 1 with FEAT_E0PD, ST (31:28) = 1 with
/// FEAT_TTST, VARange (19:16) = 1 with FEAT_LVA (`FEAT_LVA <=>
/// UInt(VARange) >= 1`); nothing else from v8.1+.
pub(crate) const fn id_aa64mmfr2(features: Features) -> u64 {
    field(features, Feature::Ttcnp, 0, 1)
        | field(features, Feature::Uao, 4, 1)
        | field(features, Feature::Lva, 16, 1)
        | field(features, Feature::Ttst, 28, 1)
        | field(features, Feature::Lse2, 32, 1)
        | field(features, Feature::Idst, 36, 1)
        | field(features, Feature::Ttl, 48, 1)
        | field(features, Feature::E0pd, 60, 1)
}

/// `LORID_EL1` with FEAT_LOR: no LORegions (LR = 0) and no LORegion
/// descriptors (LD = 0), the board's choice of the counts the register's
/// description (Arm ARM D24.2.120) leaves IMPLEMENTATION DEFINED, so
/// `LORSA_EL1`, `LOREA_EL1`, `LORN_EL1` and `LORC_EL1` are RES0. A
/// LORegion narrows the accesses a `LDLAR` or `STLLR` orders to those in
/// its region (B2.4.3); this board's one processor completes each access
/// before the next, in program order for every observer, the devices' DMA
/// included, which no narrowing could weaken, so a region would promise a
/// guest nothing it could observe, where with none the
/// instructions are the acquire and release accesses the Arm ARM makes
/// them (D24.2.120's note).
pub(crate) const LORID_EL1: u64 = 0;

/// The ID register field of a chain of features each of which
/// `Features.json` makes imply the one before (FEAT_PAN, FEAT_PAN2,
/// FEAT_PAN3): how many of `chain`, from its first, `features` holds.
const fn chain_level(features: Features, chain: &[Feature]) -> u64 {
    let mut n = 0;
    while n < chain.len() && features.has(chain[n]) {
        n += 1;
    }
    n as u64
}

/// `value` in the ID register field at `lsb` when `features` holds
/// `feature`, zero (not implemented) otherwise.
const fn field(features: Features, feature: Feature, lsb: u32, value: u64) -> u64 {
    if features.has(feature) {
        value << lsb
    } else {
        0
    }
}

/// `ID_AA64MMFR0_EL1` of a processor implementing `features`: PARange
/// (3:0) the encoding of its physical address size ([`pa_range`]), 6 for 52
/// bits with FEAT_LPA and 4 for 44 bits without, ASIDBits = 2 (16-bit
/// ASIDs), mixed endianness not supported. The stage 1 granules: TGran4
/// (31:28) = 0, the 4 KiB one every processor here has, 1 with FEAT_LPA2
/// (the granule's 52-bit addresses); TGran16 (23:20) = 1 with
/// FEAT_TGran16K, 2 with FEAT_LPA2 too, else 0; TGran64 (27:24) = 0 with
/// FEAT_TGran64K, else 0xF (each field's own encoding of "supported").
/// With FEAT_EL2, the stage 2 granules said explicitly: TGran4_2 (43:40) =
/// 2, the 4 KiB granule (FEAT_S2TGran4K), and TGran16_2 (35:32) and
/// TGran64_2 (39:36) = 2 with FEAT_S2TGran16K and FEAT_S2TGran64K, else 1,
/// not supported. That is
/// FEAT_GTG, which `Features.json` makes hold exactly when all three fields
/// are non-zero, and permits a processor before ARMv8.5;
/// `AArch64_HaveS2TG` then reads the stage 2 fields. With FEAT_LPA2,
/// TGran4_2 and, where the granule is there, TGran16_2 are 3, the
/// granule's 52-bit addresses at stage 2. Each field's FEAT_LPA2 value
/// implies the feature (`(SInt(TGran4) >= 1) ==> FEAT_LPA2`,
/// `(UInt(TGran16) >= 2) ==> FEAT_LPA2`, and `>= 3` for the stage 2
/// fields), the fields being how software finds it. ECV (63:60) = 1 with
/// FEAT_ECV, 2 with FEAT_ECV_POFF too (`FEAT_ECV_POFF <=>
/// UInt(ID_AA64MMFR0_EL1.ECV) >= 2`).
pub(crate) const fn id_aa64mmfr0(features: Features) -> u64 {
    // FEAT_LPA2's step up from each granule field's "supported".
    let lpa2 = field(features, Feature::Lpa2, 0, 1);
    let stage_2 = if features.has(Feature::El2) {
        ((2 + lpa2) << 40)
            | ((1 + field(features, Feature::S2TGran64K, 0, 1)) << 36)
            | ((1 + field(features, Feature::S2TGran16K, 0, 1 + lpa2)) << 32)
    } else {
        0
    };
    let tgran64 = if features.has(Feature::TGran64K) {
        0
    } else {
        0xF << 24
    };
    // FEAT_ECV_POFF requires FEAT_ECV (`FEAT_ECV_POFF ==> FEAT_ECV`).
    let ecv = field(features, Feature::Ecv, 60, 1) + field(features, Feature::EcvPoff, 60, 1);
    0x0000_0020
        | pa_range(features)
        | (lpa2 << 28)
        | tgran64
        | field(features, Feature::TGran16K, 20, 1 + lpa2)
        | stage_2
        | ecv
}

/// `ID_AA64MMFR0_EL1.PARange` of a processor implementing `features`, the
/// encoding ([`ADDRESS_SIZES`]) of its physical address size, Arm's
/// IMPLEMENTATION DEFINED `AArch64_PAMax`: 0b0110, 52 bits, with FEAT_LPA,
/// which a `PARange` of 52 bits or more is (`FEAT_LPA <=> UInt(PARange) >=
/// 6`), 56 bits needing FEAT_D128, which no processor here has; 0b0100, 44
/// bits, without, the Cortex-A57's.
#[inline(always)]
const fn pa_range(features: Features) -> u64 {
    if features.has(Feature::Lpa) {
        0b0110
    } else {
        0b0100
    }
}

/// Arm `AArch64_PAMax`: the physical address size of a processor
/// implementing `features`, which its `ID_AA64MMFR0_EL1.PARange` reports
/// ([`pa_range`]).
#[inline(always)]
pub(crate) const fn pa_bits(features: Features) -> u32 {
    ADDRESS_SIZES[pa_range(features) as usize]
}

/// The widest physical address of any processor here ([`pa_bits`]).
pub(crate) const MAX_PA_BITS: u32 = pa_bits(Features::EVERY);

/// The address sizes, in bits, that `ID_AA64MMFR0_EL1.PARange` and
/// `TCR_ELx.PS`/`IPS` encode, each at its encoding.
pub(crate) const ADDRESS_SIZES: [u32; 8] = [32, 36, 40, 42, 44, 48, 52, 56];

/// `CTR_EL0` of both models: the Cortex-A57's (TRM DDI 0488H, 4.3.26,
/// Table 4-40) — IminLine and DminLine 64 bytes, a PIPT L1 instruction
/// cache (L1Ip 0b11), ERG and CWG of 64 bytes, IDC and DIC 0, bit 31
/// RES1. Every cache of both models has 64-byte lines ([`Caches`]), so the
/// smallest line and the writeback granule are 64 bytes on each.
pub const CTR_EL0: u64 = 0x8444_C004;

/// The data cache line `CTR_EL0.DminLine` names, in bytes.
pub const DC_LINE_BYTES: u64 = 4 << ((CTR_EL0 >> 16) & 0xF);

/// `DCZID_EL0.BS`: `DC ZVA` zeroes 2^4 words, 64 bytes.
pub const DCZID_BLOCK_LOG2_WORDS: u64 = 4;

/// Bytes zeroed by one `DC ZVA`.
pub const DC_ZVA_BYTES: u64 = 4 << DCZID_BLOCK_LOG2_WORDS;

/// `CLIDR_EL1` of both models: the Cortex-A57's (TRM DDI 0488H, 4.3.23,
/// Table 4-38) — Ctype1 0b011, separate L1 instruction and data caches;
/// Ctype2 0b100, a unified L2; LoUIS 1, LoC 2, LoUU 1; bits 63:30 RES0.
/// The Armv9.2 model's hierarchy has the same shape
/// ([`Caches::ARMV9P2`]); on it bits 32:30 are ICB, 0b000 for an inner
/// cache boundary not disclosed (Arm ARM, DDI 0487M.d, D24.2.32), and it
/// has no FEAT_MTE2 to add the Ttype fields.
pub const CLIDR_EL1: u64 = 0x0A20_0023;

/// The caches a processor reports: `CCSIDR_EL1` for each `CSSELR_EL1`
/// selection `CLIDR_EL1` names, in the format without FEAT_CCIDX, which
/// neither model implements (`ID_AA64MMFR2_EL1.CCIDX` 0): LineSize, bits
/// 2:0, log2(bytes) - 4; Associativity, bits 12:3, ways - 1; NumSets, bits
/// 27:13, sets - 1. The board models no cache, so these values describe what
/// maintenance by set/way walks and nothing a guest can measure (Arm ARM,
/// DDI 0487M.d, D24.2.31: "You cannot make any inference about the actual
/// sizes of caches based on these parameters"); the board's device tree
/// describes the caches from them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caches {
    /// `CSSELR_EL1` 0 (Level 1, InD 0): the L1 data cache.
    pub l1_data: u64,
    /// `CSSELR_EL1` 1 (Level 1, InD 1): the L1 instruction cache.
    pub l1_instruction: u64,
    /// `CSSELR_EL1` 2 (Level 2, InD 0): the unified L2.
    pub l2: u64,
}

impl Caches {
    /// The Cortex-A57's, the CCSIDR_EL1 encodings of the TRM (DDI 0488H,
    /// 4.3.22, Table 4-37): a 32 KiB 2-way L1 data cache and a 48 KiB
    /// 3-way L1 instruction cache, the sizes the design fixes (6.1), and a
    /// 512 KiB 16-way L2. The L2's size is a configuration option of the
    /// design, 512 KiB, 1 MiB or 2 MiB (1.5, Table 1-1; 7.2), its 16 ways
    /// and 64-byte lines fixed (7.1). The board takes the smallest: with no
    /// cache modelled the size buys a guest nothing, and cleaning a whole
    /// cache by set/way, which walks every set and way of each level to the
    /// PoC (the Arm ARM's example, DDI 0487M.d, D7.5.9.16.1), costs one
    /// instruction a line, 8,192 at L2 here where 2 MiB would take 32,768.
    pub const CORTEX_A57: Self = Self {
        l1_data: 0x701F_E00A,
        l1_instruction: 0x201F_E012,
        l2: 0x703F_E07A,
    };

    /// The Armv9.2 model's: a 32 KiB 4-way L1 data cache, a 32 KiB 4-way
    /// L1 instruction cache and a 256 KiB 8-way unified L2, every line 64
    /// bytes. The model is no Arm part and no Arm document gives a generic
    /// Armv9.2 processor caches — the architecture leaves each cache's
    /// size, associativity and line IMPLEMENTATION DEFINED — so the
    /// geometry is the board's own: lines of 64 bytes, the size the Arm ARM
    /// gives generic software as `hardware_constructive_interference_size`
    /// (D24.2.31) and the one `CTR_EL0` reports; round sizes kept as small
    /// as the Cortex-A57's L2 for the same reason, the cost of a
    /// whole-cache clean by set/way (4,096 lines at L2). Bits 31:28,
    /// UNKNOWN without FEAT_CCIDX, read 0.
    pub const ARMV9P2: Self = Self {
        l1_data: 0x000F_E01A,
        l1_instruction: 0x000F_E01A,
        l2: 0x003F_E03A,
    };

    /// The cache `csselr` (`CSSELR_EL1`'s Level, bits 3:1, and InD, bit 0)
    /// selects, as a selection of a cache this processor has: the three
    /// `CLIDR_EL1` names (0, 1 and 2) are themselves, and any other — a
    /// level beyond L2, an instruction cache at the unified L2, or a
    /// reserved Level — maps onto the Level 1 cache of the kind InD names
    /// (Level 0b000, InD kept), which both models have. A read of
    /// `CSSELR_EL1` returns this selection and `CCSIDR_EL1` describes its
    /// cache.
    ///
    /// Arm leaves both reads open for a selection of no implemented cache:
    /// `CSSELR_EL1` returns UNKNOWN Level and InD (Arm ARM, DDI 0487M.d,
    /// D24.2.40 and K1.2.12; the Cortex-A57 TRM, DDI 0488H, 4.3.25, makes
    /// only Level UNKNOWN, which is why InD is kept), and `CCSIDR_EL1` is
    /// CONSTRAINED UNPREDICTABLE, a NOP, UNDEFINED or an UNKNOWN value
    /// (D24.2.31, "Accessing CCSIDR_EL1"; the Cortex-A57's TRM, 4.3.22,
    /// chooses an UNKNOWN value). The mapping is the board's own, the first
    /// of the ways the Arm ARM lets an unallocated field value behave
    /// (K1.2.19: it "maps onto any of the allocated values"): every read
    /// then names a cache the guest can find in `CLIDR_EL1`, and the two
    /// registers agree on which.
    pub const fn selection(self, csselr: u64) -> u64 {
        match csselr & 0xF {
            selected @ 0..=2 => selected,
            unimplemented => unimplemented & 1,
        }
    }

    /// `CCSIDR_EL1` for the cache `csselr` selects ([`Caches::selection`]).
    pub const fn ccsidr(self, csselr: u64) -> u64 {
        match self.selection(csselr) {
            0 => self.l1_data,
            1 => self.l1_instruction,
            _ => self.l2,
        }
    }
}

/// `SCTLR_EL1` at reset, the Cortex-A57's (TRM DDI 0488H, Table 4-3 and
/// 4.3.30): MMU and caches off, SP alignment checks on, `WFI`/`WFE` not
/// trapped from EL0. The Armv9.2 model, whose fields the Arm ARM leaves
/// UNKNOWN at reset, takes the same value.
pub const SCTLR_EL1_RESET: u64 = 0x00C5_0838;

/// `OSLSR_EL1` at reset: the OS lock is implemented (OSLM = 0b10) and locked.
pub const OSLSR_EL1_RESET: u64 = 0b1010;
