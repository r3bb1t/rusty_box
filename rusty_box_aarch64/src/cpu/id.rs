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
/// memory-mapped v2).
pub(crate) const fn id_aa64pfr0(features: Features) -> u64 {
    0x11 | field(features, Feature::Fp16, 16, 1) | field(features, Feature::Fp16, 20, 1)
}

/// `ID_AA64PFR1_EL1` of a processor implementing `features`: `SME` (bits
/// 27:24) = 1 with FEAT_SME, without FEAT_SME2 (`Features.json`: `FEAT_SME
/// <=> UInt(ID_AA64PFR1_EL1.SME) >= 1`); nothing else from v8.1+.
pub(crate) const fn id_aa64pfr1(features: Features) -> u64 {
    field(features, Feature::Sme, 24, 1)
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

/// `ID_AA64DFR0_EL1`: DebugVer = 6 (v8 debug), BRPs = 1 and WRPs = 1 (two of
/// each, the architectural minimum), CTX_CMPs = 0, PMUVer = 0 (no PMU).
pub const ID_AA64DFR0_EL1: u64 = 0x0000_0000_0010_1006;

/// `ID_AA64ISAR0_EL1` of a processor implementing `features`: AES (7:4) = 2
/// with FEAT_AES (AESE, AESD, AESMC, AESIMC and the 64-bit PMULL), SHA1
/// (11:8) = 1, SHA2 (15:12) = 1 (SHA-256 only), CRC32 (19:16) = 1, Atomic
/// (23:20) = 2 with FEAT_LSE, RDM (31:28) = 1, DP (47:44) = 1 with
/// FEAT_DotProd and FHM (51:48) = 1; no SHA-512, SHA3, SM3, SM4 or other
/// field. The Cortex-A57's is 0x11120, as QEMU sets it in
/// `aarch64_aa32_a57_init` (target/arm/cpu-max.c), which
/// `aarch64_a57_initfn` calls.
pub(crate) const fn id_aa64isar0(features: Features) -> u64 {
    field(features, Feature::Aes, 4, 2)
        | field(features, Feature::Sha1, 8, 1)
        | field(features, Feature::Sha256, 12, 1)
        | field(features, Feature::Crc32, 16, 1)
        | field(features, Feature::Lse, 20, 2)
        | field(features, Feature::Rdm, 28, 1)
        | field(features, Feature::DotProd, 44, 1)
        | field(features, Feature::Fhm, 48, 1)
}

/// `ID_AA64ISAR1_EL1` of a processor implementing `features`: LRCPC
/// (23:20) = 1 with FEAT_LRCPC, 2 with FEAT_LRCPC2 as well (which Arm's
/// feature model makes imply FEAT_LRCPC, so the field never promises LDAPR
/// to a processor without it), and I8MM (55:52) = 1 with FEAT_I8MM;
/// nothing else from v8.2+.
pub(crate) const fn id_aa64isar1(features: Features) -> u64 {
    let lrcpc = if !features.has(Feature::Lrcpc) {
        0
    } else if features.has(Feature::Lrcpc2) {
        2
    } else {
        1
    };
    (lrcpc << 20) | field(features, Feature::I8mm, 52, 1)
}

/// `ID_AA64ZFR0_EL1` of a processor implementing `features`: SVEver = 0
/// (no SVE2), and I8MM (47:44) = 1 when FEAT_I8MM comes with FEAT_SME
/// (`Features.json`: `(FEAT_SME || FEAT_SVE) ==> (FEAT_I8MM <=>
/// UInt(ID_AA64ZFR0_EL1.I8MM) >= 1)`); zero without FEAT_SME, as a
/// processor without SVE or SME reports it. The field promises the SVE
/// USDOT and SUDOT in Streaming SVE mode, which are among the encodings
/// of FEAT_SME still being built (`Feature::complete`), as is every SVE
/// instruction that mode allows.
pub(crate) const fn id_aa64zfr0(features: Features) -> u64 {
    if features.has(Feature::Sme) {
        field(features, Feature::I8mm, 44, 1)
    } else {
        0
    }
}

/// `ID_AA64MMFR2_EL1` of a processor implementing `features`: AT (35:32) =
/// 1 with FEAT_LSE2; nothing else from v8.1+.
pub(crate) const fn id_aa64mmfr2(features: Features) -> u64 {
    field(features, Feature::Lse2, 32, 1)
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

/// `ID_AA64MMFR0_EL1`: PARange = 4 (44-bit physical addresses, as the
/// Cortex-A57), ASIDBits = 2 (16-bit ASIDs), mixed endianness not supported,
/// TGran16 = 0 (not supported), TGran64 = 0xF (not supported), TGran4 = 0
/// (supported). Only the 4 KiB granule is implemented, so only it is offered.
pub const ID_AA64MMFR0_EL1: u64 = 0x0000_0000_0F00_0024;

/// `ID_AA64MMFR1_EL1`: nothing from v8.1+.
pub const ID_AA64MMFR1_EL1: u64 = 0;

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

/// `CCSIDR_EL1` for each `CSSELR_EL1` selection QEMU's Cortex-A57 answers:
/// L1 data (32 KiB, 2-way, 64-byte lines), L1 instruction (48 KiB, 3-way),
/// L2 unified (2 MiB, 16-way). Any other selection reads as zero.
pub const CCSIDR_L1D: u64 = 0x701F_E00A;
pub const CCSIDR_L1I: u64 = 0x201F_E012;
pub const CCSIDR_L2: u64 = 0x70FF_E07A;

/// `SCTLR_EL1` at reset, as QEMU's Cortex-A57 (`reset_sctlr`): MMU and caches
/// off, SP alignment checks on, `WFI`/`WFE` not trapped from EL0.
pub const SCTLR_EL1_RESET: u64 = 0x00C5_0838;

/// `OSLSR_EL1` at reset: the OS lock is implemented (OSLM = 0b10) and locked.
pub const OSLSR_EL1_RESET: u64 = 0b1010;
