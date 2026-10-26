//! Identification values a guest can read, and the feature set they promise.
//!
//! The processor is shaped like a Cortex-A57 r1p0 (the value QEMU's
//! `cortex-a57` model reports in `MIDR_EL1`), cut down to what this
//! interpreter implements: AArch64 at EL0 and EL1 only, FP and Advanced SIMD,
//! the Cryptographic Extension's AES, PMULL, SHA-1 and SHA-256 and CRC32 (the
//! Cortex-A57's optional extensions, all of them, as QEMU's `cortex-a57`
//! has them), the 4 KiB translation granule, no PMU. Every value here
//! is consistent with that cut — an ID register that advertised a feature the
//! executor rejects would make the feature unreachable (or worse, reachable
//! and wrong), so the two change together. The registers that report
//! instruction-set features are functions of the processor's feature set,
//! which the decoder reads too.

use super::features::{Feature, Features};

/// `MIDR_EL1`: Arm Ltd, variant 1, architecture 0xF, part 0xD07 (Cortex-A57),
/// revision 0. QEMU `aarch64_a57_initfn` sets the same.
pub const MIDR_EL1: u64 = 0x411F_D070;

/// `MPIDR_EL1`: bit 31 RES1, uniprocessor bit clear, affinity 0.0.0.0.
pub const MPIDR_EL1: u64 = 0x8000_0000;

/// `REVIDR_EL1`: no revision-specific fixes to report.
pub const REVIDR_EL1: u64 = 0;

/// `ID_AA64PFR0_EL1` of a processor implementing `features`: EL0 = 1 and
/// EL1 = 1 (AArch64 only), EL2 = EL3 = 0 (not implemented), FP (19:16) and
/// AdvSIMD (23:20) 1 with FEAT_FP16 (half-precision arithmetic) and 0
/// without, GIC = 0 (no system-register interface: the GIC is a
/// memory-mapped v2), RAS (31:28) = 1 with FEAT_RAS and 2 with FEAT_RASv1p1
/// as well (which `Features.json` makes imply FEAT_RAS; with no EL3, 2
/// promises no FEAT_DoubleFault), DIT (51:48) = 1 with FEAT_DIT, CSV2
/// (59:56) = 1 with FEAT_CSV2 (2 would be FEAT_CSV2_2, which has the
/// `SCXTNUM_ELx` registers), CSV3 (63:60) = 1 with FEAT_CSV3.
pub(crate) const fn id_aa64pfr0(features: Features) -> u64 {
    let ras = chain_level(features, &[Feature::Ras, Feature::RasV1p1]);
    0x11 | field(features, Feature::Fp16, 16, 1)
        | field(features, Feature::Fp16, 20, 1)
        | (ras << 28)
        | field(features, Feature::Dit, 48, 1)
        | field(features, Feature::Csv2, 56, 1)
        | field(features, Feature::Csv3, 60, 1)
}

/// `ID_AA64PFR1_EL1` of a processor implementing `features`: `BT` (bits
/// 3:0) = 1 with FEAT_BTI (2 would be FEAT_BTIE), `SSBS` (7:4) = 1 with
/// FEAT_SSBS and 2 with FEAT_SSBS2 as well (which `Features.json` makes
/// imply FEAT_SSBS), `SME` (bits 27:24) = 1 with FEAT_SME, without
/// FEAT_SME2 (`Features.json`: `FEAT_SME <=> UInt(ID_AA64PFR1_EL1.SME) >=
/// 1`); nothing else from v8.1+.
pub(crate) const fn id_aa64pfr1(features: Features) -> u64 {
    let ssbs = chain_level(features, &[Feature::Ssbs, Feature::Ssbs2]);
    field(features, Feature::Bti, 0, 1) | (ssbs << 4) | field(features, Feature::Sme, 24, 1)
}

/// `ID_AA64SMFR0_EL1` of a processor implementing `features`: with
/// FEAT_SME, the fields `Features.json` requires of it, F32F32 (bit 32),
/// B16F32 (34), F16F32 (35) and I8I32 (39:36, 0b1111), with SMEver 0 (no
/// FEAT_SME2), and no FA64, F64F64 or I16I64; zero without.
pub(crate) const fn id_aa64smfr0(features: Features) -> u64 {
    if features.has(Feature::Sme) {
        (1 << 32) | (1 << 34) | (1 << 35) | (0xF << 36)
    } else {
        0
    }
}

/// `SMIDR_EL1`, as QEMU reports it (`target/arm/helper.c`, `sme_reginfo`):
/// implementer 0 (software), revision 0, no streaming execution priority
/// (`SMPS` = 0, so `SMPRI_EL1` is RES0), no affinity with another
/// processor's Streaming SVE mode.
pub(crate) const SMIDR_EL1: u64 = 0;

/// `ID_AA64DFR0_EL1` of a processor implementing `features`: DebugVer
/// (3:0) = 6 (Armv8.0 debug), 7 with FEAT_Debugv8p1, 8 with FEAT_Debugv8p2
/// and 9 with FEAT_Debugv8p4 (each of which `Features.json` makes imply the
/// one before), BRPs = 1 and WRPs = 1 (two of each, the architectural
/// minimum), CTX_CMPs = 0, PMUVer = 0 (no PMU).
pub(crate) const fn id_aa64dfr0(features: Features) -> u64 {
    let debug = chain_level(features, &[Feature::Debugv8p1, Feature::Debugv8p2, Feature::Debugv8p4]);
    0x0000_0000_0010_1000 | (6 + debug)
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

/// `ID_AA64ISAR0_EL1` of a processor implementing `features`: AES (7:4) = 2
/// with FEAT_AES (AESE, AESD, AESMC, AESIMC and the 64-bit PMULL), SHA1
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
    let ts = chain_level(features, &[Feature::FlagM, Feature::FlagM2]);
    let tlb = chain_level(features, &[Feature::TlbiOs, Feature::TlbiRange]);
    field(features, Feature::Aes, 4, 2)
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
/// FEAT_TTL, E0PD (63:60) = 1 with FEAT_E0PD; nothing else from v8.1+.
pub(crate) const fn id_aa64mmfr2(features: Features) -> u64 {
    field(features, Feature::Ttcnp, 0, 1)
        | field(features, Feature::Uao, 4, 1)
        | field(features, Feature::Lse2, 32, 1)
        | field(features, Feature::Idst, 36, 1)
        | field(features, Feature::Ttl, 48, 1)
        | field(features, Feature::E0pd, 60, 1)
}

/// `LORID_EL1` with FEAT_LOR: no LORegions (LR = 0) and no LORegion
/// descriptors (LD = 0), as QEMU's `lor_reginfo` has it, so `LORSA_EL1`,
/// `LOREA_EL1`, `LORN_EL1` and `LORC_EL1` hold nothing.
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

/// `ID_AA64MMFR0_EL1` of a processor implementing `features`: PARange = 4
/// (44-bit physical addresses, as the Cortex-A57), ASIDBits = 2 (16-bit
/// ASIDs), mixed endianness not supported, TGran16 = 0 (not supported),
/// TGran64 = 0xF (not supported), TGran4 = 0 (supported): only the 4 KiB
/// granule is implemented, so only it is offered. ECV (63:60) = 1 with
/// FEAT_ECV (2 would be FEAT_ECV_POFF, which needs EL2).
pub(crate) const fn id_aa64mmfr0(features: Features) -> u64 {
    0x0F00_0024 | field(features, Feature::Ecv, 60, 1)
}

/// Physical address width implied by `ID_AA64MMFR0_EL1.PARange`.
pub const PA_BITS: u32 = 44;

/// `CTR_EL0`: as the Cortex-A57 — 64-byte minimum I and D lines, PIPT
/// instruction cache, ERG and CWG of 64 bytes, bit 31 RES1.
pub const CTR_EL0: u64 = 0x8444_C004;

/// `DCZID_EL0.BS`: `DC ZVA` zeroes 2^4 words, 64 bytes.
pub const DCZID_BLOCK_LOG2_WORDS: u64 = 4;

/// Bytes zeroed by one `DC ZVA`.
pub const DC_ZVA_BYTES: u64 = 4 << DCZID_BLOCK_LOG2_WORDS;

/// `CLIDR_EL1`: L1 separate I and D, L2 unified, LoUIS = 1, LoC = 2, LoUU = 1
/// — the Cortex-A57 hierarchy QEMU reports.
pub const CLIDR_EL1: u64 = 0x0A20_0023;

/// `CCSIDR_EL1` for each `CSSELR_EL1` selection the Cortex-A57 answers: L1
/// data (32 KiB, 2-way, 64-byte lines), L1 instruction (48 KiB, 3-way), L2
/// unified (2 MiB, 16-way). The L1 geometry is the TRM's (DDI 0488C, 6.1);
/// QEMU 9.2 and later report a 4-way L1 data cache (README, "Where the
/// references differ"). Any other selection reads as zero.
pub const CCSIDR_L1D: u64 = 0x701F_E00A;
pub const CCSIDR_L1I: u64 = 0x201F_E012;
pub const CCSIDR_L2: u64 = 0x70FF_E07A;

/// `SCTLR_EL1` at reset, as QEMU's Cortex-A57 (`reset_sctlr`): MMU and caches
/// off, SP alignment checks on, `WFI`/`WFE` not trapped from EL0.
pub const SCTLR_EL1_RESET: u64 = 0x00C5_0838;

/// `OSLSR_EL1` at reset: the OS lock is implemented (OSLM = 0b10) and locked.
pub const OSLSR_EL1_RESET: u64 = 0b1010;
