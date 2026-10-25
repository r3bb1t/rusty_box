//! Identification values a guest can read, and the feature set they promise.
//!
//! The processor is shaped like a Cortex-A57 r1p0 (the value QEMU's
//! `cortex-a57` model reports in `MIDR_EL1`), cut down to what this
//! interpreter implements: AArch64 at EL0 and EL1 only, FP and Advanced SIMD,
//! the 4 KiB translation granule, no PMU, no crypto, no CRC32. Every value here
//! is consistent with that cut — an ID register that advertised a feature the
//! executor rejects would make the feature unreachable (or worse, reachable
//! and wrong), so the two change together.

/// `MIDR_EL1`: Arm Ltd, variant 1, architecture 0xF, part 0xD07 (Cortex-A57),
/// revision 0. QEMU `aarch64_a57_initfn` sets the same.
pub const MIDR_EL1: u64 = 0x411F_D070;

/// `MPIDR_EL1`: bit 31 RES1, uniprocessor bit clear, affinity 0.0.0.0.
pub const MPIDR_EL1: u64 = 0x8000_0000;

/// `REVIDR_EL1`: no revision-specific fixes to report.
pub const REVIDR_EL1: u64 = 0;

/// `ID_AA64PFR0_EL1`: EL0 = 1 and EL1 = 1 (AArch64 only), EL2 = EL3 = 0 (not
/// implemented), FP = 0 and AdvSIMD = 0 (implemented, without half precision),
/// GIC = 0 (no system-register interface: the GIC is a memory-mapped v2).
pub const ID_AA64PFR0_EL1: u64 = 0x0000_0000_0000_0011;

/// `ID_AA64PFR1_EL1`: nothing from v8.1+.
pub const ID_AA64PFR1_EL1: u64 = 0;

/// `ID_AA64DFR0_EL1`: DebugVer = 6 (v8 debug), BRPs = 1 and WRPs = 1 (two of
/// each, the architectural minimum), CTX_CMPs = 0, PMUVer = 0 (no PMU).
pub const ID_AA64DFR0_EL1: u64 = 0x0000_0000_0010_1006;

/// `ID_AA64ISAR0_EL1`: no AES, SHA, CRC32 or LSE atomics.
pub const ID_AA64ISAR0_EL1: u64 = 0;

/// `ID_AA64ISAR1_EL1`: no v8.2+ instructions.
pub const ID_AA64ISAR1_EL1: u64 = 0;

/// `ID_AA64MMFR0_EL1`: PARange = 4 (44-bit physical addresses, as the
/// Cortex-A57), ASIDBits = 2 (16-bit ASIDs), mixed endianness not supported,
/// TGran16 = 0 (not supported), TGran64 = 0xF (not supported), TGran4 = 0
/// (supported). Only the 4 KiB granule is implemented, so only it is offered.
pub const ID_AA64MMFR0_EL1: u64 = 0x0000_0000_0F00_0024;

/// `ID_AA64MMFR1_EL1` / `ID_AA64MMFR2_EL1`: nothing from v8.1+.
pub const ID_AA64MMFR1_EL1: u64 = 0;
pub const ID_AA64MMFR2_EL1: u64 = 0;

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
