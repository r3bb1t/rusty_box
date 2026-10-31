//! The `SCTLR_EL1` fields the processor reads, as masks of the register
//! (Arm ARM D24.2, `SCTLR_EL1`). The pointer authentication enables and
//! BT0/BT1 are read where their key or EL selects the bit
//! (`pauth.rs`, `bti.rs`).

/// M: stage 1 translation for the EL1&0 regime.
pub(crate) const M: u64 = 1 << 0;
/// A: every misaligned data access faults.
pub(crate) const A: u64 = 1 << 1;
/// C: data accesses to Normal memory are cacheable at stage 1; clear, they
/// are Non-cacheable (`AArch64_S1DCacheEnabled`).
pub(crate) const C: u64 = 1 << 2;
/// SA: the SP alignment check at EL1.
pub(crate) const SA: u64 = 1 << 3;
/// SA0: the SP alignment check at EL0.
pub(crate) const SA0: u64 = 1 << 4;
/// nAA (FEAT_LSE2): an ordered access may leave its 16-byte granule.
pub(crate) const NAA: u64 = 1 << 6;
/// UMA: EL0 may access the DAIF bits.
pub(crate) const UMA: u64 = 1 << 9;
/// EnRCTX (FEAT_SPECRES): EL0 may execute `CFP`, `DVP` and `CPP RCTX`.
pub(crate) const ENRCTX: u64 = 1 << 10;
/// DZE: EL0 may execute `DC ZVA`.
pub(crate) const DZE: u64 = 1 << 14;
/// UCT: EL0 may read `CTR_EL0`.
pub(crate) const UCT: u64 = 1 << 15;
/// nTWI: `WFI` at EL0 is not trapped.
pub(crate) const NTWI: u64 = 1 << 16;
/// nTWE: `WFE` at EL0 is not trapped.
pub(crate) const NTWE: u64 = 1 << 18;
/// WXN: writable memory is execute-never.
pub(crate) const WXN: u64 = 1 << 19;
/// SPAN (FEAT_PAN): 0 sets PSTATE.PAN on taking an exception.
pub(crate) const SPAN: u64 = 1 << 23;
/// UCI: EL0 may execute the cache maintenance instructions by VA.
pub(crate) const UCI: u64 = 1 << 26;
/// DSSBS (FEAT_SSBS): the PSTATE.SSBS an exception sets.
pub(crate) const DSSBS: u64 = 1 << 44;
/// EPAN (FEAT_PAN3): memory EL0 can execute counts for PSTATE.PAN.
pub(crate) const EPAN: u64 = 1 << 57;
/// EnTP2 (FEAT_SME): EL0 may access `TPIDR2_EL0`.
pub(crate) const ENTP2: u64 = 1 << 60;
