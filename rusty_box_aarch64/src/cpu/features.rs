//! What a processor implements: the architectural features, by the Arm
//! ARM's `FEAT_` names, that decide which encodings execute and what the ID
//! registers report.
//!
//! Provenance (R7): the Arm A-profile Architecture Machine Readable
//! Specification, `Features.json` and `Instructions.json` (release
//! 2026-09): each feature's dependencies, the ID register fields that report
//! it, and the condition each encoding carries.
//!
//! A feature is implemented by a processor when its [`Features`] set holds
//! it; the decoder executes an encoding only when the processor implements
//! the feature the encoding belongs to. Every processor built implements
//! the Cortex-A57's features but FEAT_DoubleLock, which Armv9.0 forbids;
//! the constants of `id` report the rest, and the ID register field of
//! every other feature is read from the set (`sysreg.rs`); `isa::tests`
//! checks, for every implementation built, that the ID registers and the
//! set agree.
//!
//! A machine's processor is one of the [`CpuModel`]s, each an
//! [`Implementation`]: a feature set, the processor's identity, and the
//! IMPLEMENTATION DEFINED choices its features leave open.

use super::id;
use rusty_box_core::ClockHz;

use super::el2::Levels;
use core::marker::PhantomData;

/// An architectural feature, by the Arm ARM's `FEAT_` name; the instruction
/// set features are what an encoding belongs to.
///
/// Closed (R5): the features this processor can implement, each one a bit of
/// [`Features`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Feature {
    /// The ARMv8.0 base instruction set: an encoding without a feature
    /// condition.
    Base,
    /// FEAT_FP: scalar floating point, and the loads and stores of SIMD&FP
    /// registers.
    Fp,
    /// FEAT_AdvSIMD: Advanced SIMD.
    AdvSimd,
    /// FEAT_AES: AESE, AESD, AESMC, AESIMC.
    Aes,
    /// FEAT_SHA1: SHA1C, SHA1P, SHA1M, SHA1H, SHA1SU0, SHA1SU1.
    Sha1,
    /// FEAT_SHA256: SHA256H, SHA256H2, SHA256SU0, SHA256SU1.
    Sha256,
    /// FEAT_CRC32: CRC32B/H/W/X, CRC32CB/CH/CW/CX.
    Crc32,
    /// FEAT_SME: Streaming SVE mode, the ZA array and the instructions that
    /// use them.
    Sme,
    /// FEAT_LSE: the atomic memory operations, SWP, CAS and CASP.
    Lse,
    /// FEAT_LSE2: unaligned exclusive and atomic accesses within a 16-byte
    /// granule, and `SCTLR_EL1.nAA` for the ordered ones. No encoding of its
    /// own.
    Lse2,
    /// FEAT_LRCPC: LDAPR, LDAPRB, LDAPRH.
    Lrcpc,
    /// FEAT_LRCPC2: LDAPUR*, STLUR*.
    Lrcpc2,
    /// FEAT_RDM: SQRDMLAH, SQRDMLSH.
    Rdm,
    /// FEAT_DotProd: SDOT, UDOT.
    DotProd,
    /// FEAT_I8MM: USDOT, SUDOT, SMMLA, USMMLA, UMMLA.
    I8mm,
    /// FEAT_FP16: half-precision arithmetic, scalar and Advanced SIMD.
    Fp16,
    /// FEAT_FHM: FMLAL, FMLSL, FMLAL2, FMLSL2.
    Fhm,
    /// FEAT_FCMA: FCMLA and FCADD.
    Fcma,
    /// FEAT_JSCVT: FJCVTZS.
    Jscvt,
    /// FEAT_FRINTTS: FRINT32Z, FRINT32X, FRINT64Z, FRINT64X.
    Frintts,
    /// FEAT_BF16: BFCVT, BFCVTN, BFDOT, BFMLALB, BFMLALT, BFMMLA.
    Bf16,
    /// FEAT_PAuth: the pointer authentication instructions, keys and
    /// enables.
    PAuth,
    /// FEAT_PAuth2: a code XORed into the pointer, and no error code in a
    /// failed one. No encoding of its own.
    PAuth2,
    /// FEAT_FPAC: a failed authentication alone faults. No encoding of its
    /// own.
    Fpac,
    /// FEAT_FPACCOMBINE: a failed authentication in a branch, return or load
    /// faults too. No encoding of its own.
    FpacCombine,
    /// FEAT_PACQARMA5: the code computed with QARMA5. No encoding of its
    /// own.
    PacQarma5,
    /// FEAT_BTI: BTI, PSTATE.BTYPE and guarded pages.
    Bti,
    /// FEAT_FlagM: CFINV, RMIF, SETF8, SETF16.
    FlagM,
    /// FEAT_FlagM2: AXFLAG, XAFLAG.
    FlagM2,
    /// FEAT_SB: SB.
    Sb,
    /// FEAT_PAN: PSTATE.PAN, which refuses EL1 data accesses to memory EL0
    /// can reach. No encoding of its own.
    Pan,
    /// FEAT_PAN2: AT S1E1RP and AT S1E1WP, which are SYS. No encoding of
    /// its own.
    Pan2,
    /// FEAT_PAN3: SCTLR_EL1.EPAN, under which memory EL0 can execute counts
    /// as memory EL0 can reach. No encoding of its own.
    Pan3,
    /// FEAT_UAO: PSTATE.UAO, which gives LDTR and STTR at EL1 EL1's
    /// permissions. No encoding of its own.
    Uao,
    /// FEAT_DIT: PSTATE.DIT. No encoding of its own.
    Dit,
    /// FEAT_SSBS: PSTATE.SSBS, written by MSR (immediate). No encoding of
    /// its own.
    Ssbs,
    /// FEAT_SSBS2: the SSBS register. No encoding of its own.
    Ssbs2,
    /// FEAT_HPDS: TCR_EL1.HPD0 and HPD1, which turn off the hierarchical
    /// permissions of table descriptors. No encoding of its own.
    Hpds,
    /// FEAT_TTCNP: TTBR0_EL1.CnP and TTBR1_EL1.CnP. No encoding of its own.
    Ttcnp,
    /// FEAT_E0PD: TCR_EL1.E0PD0 and E0PD1, under which an EL0 access to
    /// that half of the address space takes a level 0 translation fault.
    /// No encoding of its own.
    E0pd,
    /// FEAT_CSV3: no data loaded under speculation from a faulting access
    /// is used, which FEAT_E0PD requires. No encoding of its own.
    Csv3,
    /// FEAT_TLBIOS: the Outer Shareable TLBI forms, which are SYS. No
    /// encoding of its own.
    TlbiOs,
    /// FEAT_TLBIRANGE: the TLBI range forms, which are SYS. No encoding of
    /// its own.
    TlbiRange,
    /// FEAT_TTL: the TTL hint of a TLBI by address. No encoding of its own.
    Ttl,
    /// FEAT_XS: DSB nXS, and the TLBI nXS forms, which are SYS.
    Xs,
    /// FEAT_DPB: DC CVAP, which is SYS. No encoding of its own.
    Dpb,
    /// FEAT_DPB2: DC CVADP, which is SYS. No encoding of its own.
    Dpb2,
    /// FEAT_LOR: LDLAR, STLLR, their byte and halfword forms, and the
    /// LORegion registers.
    Lor,
    /// FEAT_IDST: a read of the feature ID register space that is not
    /// permitted traps with EC 0x18 instead of being UNDEFINED. No encoding
    /// of its own.
    Idst,
    /// FEAT_SPECRES: CFP, DVP and CPP RCTX, which are SYS. No encoding of
    /// its own.
    SpecRes,
    /// FEAT_CSV2: branch targets trained in one context do not steer
    /// speculation in another. No encoding of its own.
    Csv2,
    /// FEAT_ECV: CNTPCTSS_EL0, CNTVCTSS_EL0 and CNTKCTL_EL1.EVNTIS. No
    /// encoding of its own.
    Ecv,
    /// FEAT_ECV_POFF: `CNTPOFF_EL2` and `CNTHCTL_EL2.ECV`, the physical
    /// count's offset at EL0 and EL1. Only a processor with EL2 has it
    /// (`(FEAT_AA64 && FEAT_ECV_POFF) ==> FEAT_AA64EL2`). No encoding of its
    /// own.
    EcvPoff,
    /// FEAT_WFxT: WFET and WFIT.
    WfxT,
    /// FEAT_RAS: ESB, `ERRIDR_EL1` and `DISR_EL1`, and the error state a
    /// synchronous external abort reports.
    Ras,
    /// FEAT_RASv1p1: more error record registers (`ERXMISC2_EL1` and the
    /// rest), which a processor with no error records does not have. No
    /// encoding of its own.
    RasV1p1,
    /// FEAT_RASSAv1p1: the RAS System Architecture v1.1, which changes only
    /// the memory-mapped error record registers (`ERRIIDR`, `ERR<n>MISC2`,
    /// `ERR<n>MISC3` and `ERR<n>STATUS` in Arm's register data). This
    /// processor has no error records, so nothing reads it; a processor
    /// with FEAT_RASv1p1 declares it, as `Features.json` makes FEAT_RASv1p1
    /// imply it. No encoding of its own.
    #[allow(dead_code)]
    RasSaV1p1,
    /// FEAT_Debugv8p1: `ID_AA64DFR0_EL1.DebugVer` 7. No encoding of its own.
    Debugv8p1,
    /// FEAT_Debugv8p2: `DebugVer` 8. No encoding of its own.
    Debugv8p2,
    /// FEAT_Debugv8p4: `DebugVer` 9, and non-invasive debug always enabled
    /// (`DBGAUTHSTATUS_EL1.NSNID`). No encoding of its own.
    Debugv8p4,
    /// FEAT_AFP: FPCR.FIZ, FPCR.AH (Arm's alternate floating-point
    /// behaviours) and FPCR.NEP (scalar results merged into a source
    /// register). No encoding of its own.
    Afp,
    /// FEAT_RPRES: 12-bit single-precision FRECPE and FRSQRTE estimates
    /// under FPCR.AH = 1. No encoding of its own.
    Rpres,
    /// FEAT_EL2: Exception level 2, the hypervisor's, with its registers,
    /// its translation regime and the traps and routing controls it holds
    /// over EL1 and EL0. No encoding of its own: `HVC`, `ERET` and the
    /// `SYS` forms that name EL2 are base encodings whose decode or
    /// execution checks `HaveEL(EL2)`.
    El2,
    /// FEAT_AA64EL2: EL2 in AArch64 state, the only state this processor
    /// has. No encoding of its own.
    Aa64El2,
    /// FEAT_SME2: the ZT0 register, predicate-as-counter, and the
    /// multi-vector instructions.
    Sme2,
    /// FEAT_PMULL: the 64-bit PMULL and PMULL2 (`size = 11`), which PMULL's
    /// decode pseudocode checks. No encoding of its own.
    Pmull,
    /// FEAT_DoubleLock: the OS Double Lock, `OSDLR_EL1.DLK`, which Armv9.0
    /// forbids (`v9Ap0 ==> !FEAT_DoubleLock`). No encoding of its own.
    DoubleLock,
    /// FEAT_TGran16K: the 16 KiB translation granule at stage 1. No
    /// encoding of its own.
    TGran16K,
    /// FEAT_TGran64K: the 64 KiB translation granule at stage 1. No
    /// encoding of its own.
    TGran64K,
    /// FEAT_S2TGran16K: the 16 KiB granule at stage 2, which needs EL2 and
    /// the granule at stage 1 (`FEAT_S2TGran16K ==> (FEAT_AA64EL2 &&
    /// FEAT_TGran16K)`). No encoding of its own.
    S2TGran16K,
    /// FEAT_S2TGran64K: the 64 KiB granule at stage 2, as FEAT_S2TGran16K.
    /// No encoding of its own.
    S2TGran64K,
    /// FEAT_TTST: small translation tables, an input address down to 16
    /// bits at either stage (`AArch64_MaxTxSZ`: `TxSZ` up to 48, 47 with the
    /// 64 KiB granule), and stage 2 starting at level 3 with the 4 KiB
    /// granule. No encoding of its own.
    Ttst,
    /// FEAT_LPA: 52-bit physical addresses, `PAMax` 52
    /// (`ID_AA64MMFR0_EL1.PARange` 0b0110, `FEAT_LPA <=> UInt(PARange) >=
    /// 6`), which the 64 KiB granule's descriptors and table base registers
    /// can hold. No encoding of its own.
    Lpa,
    /// FEAT_LVA: 52-bit virtual addresses with the 64 KiB granule
    /// (`AArch64_S1MinTxSZ` 12). No encoding of its own.
    Lva,
    /// FEAT_LPA2: 52-bit addresses with the 4 KiB and 16 KiB granules where
    /// `TCR_EL1.DS`, `TCR_EL2.DS` or `VTCR_EL2.DS` is set: input addresses
    /// down to a `TxSZ` of 12, walks from level -1 with the 4 KiB granule,
    /// blocks at level 0 (4 KiB) and level 1 (16 KiB), a descriptor's bits
    /// 9:8 holding address bits 51:50 and its shareability taken from the
    /// `TCR`, and a table base register's bits 5:2 address bits 51:48. It
    /// needs FEAT_LVA (`FEAT_LPA2 ==> FEAT_LVA`). No encoding of its own.
    Lpa2,
}

// `Feature::ALL` is in declaration order, and each feature has a bit of a
// `Features` set. That it lists every feature, which `Features::EVERY` and
// the EL2 register table built from it rely on, the source-reading test
// `the_decoder_source_has_the_shape_the_spec_test_reads` checks, against
// the enum as written, in every test run.
const _: () = {
    let mut i = 0;
    while i < Feature::ALL.len() {
        assert!(
            Feature::ALL[i] as usize == i,
            "Feature::ALL is out of declaration order"
        );
        i += 1;
    }
    assert!(
        Feature::ALL.len() <= u128::BITS as usize,
        "more features than a Features set has bits"
    );
};

impl Feature {
    /// Every feature, in declaration order.
    pub(crate) const ALL: [Feature; 75] = [
        Feature::Base,
        Feature::Fp,
        Feature::AdvSimd,
        Feature::Aes,
        Feature::Sha1,
        Feature::Sha256,
        Feature::Crc32,
        Feature::Sme,
        Feature::Lse,
        Feature::Lse2,
        Feature::Lrcpc,
        Feature::Lrcpc2,
        Feature::Rdm,
        Feature::DotProd,
        Feature::I8mm,
        Feature::Fp16,
        Feature::Fhm,
        Feature::Fcma,
        Feature::Jscvt,
        Feature::Frintts,
        Feature::Bf16,
        Feature::PAuth,
        Feature::PAuth2,
        Feature::Fpac,
        Feature::FpacCombine,
        Feature::PacQarma5,
        Feature::Bti,
        Feature::FlagM,
        Feature::FlagM2,
        Feature::Sb,
        Feature::Pan,
        Feature::Pan2,
        Feature::Pan3,
        Feature::Uao,
        Feature::Dit,
        Feature::Ssbs,
        Feature::Ssbs2,
        Feature::Hpds,
        Feature::Ttcnp,
        Feature::E0pd,
        Feature::Csv3,
        Feature::TlbiOs,
        Feature::TlbiRange,
        Feature::Ttl,
        Feature::Xs,
        Feature::Dpb,
        Feature::Dpb2,
        Feature::Lor,
        Feature::Idst,
        Feature::SpecRes,
        Feature::Csv2,
        Feature::Ecv,
        Feature::EcvPoff,
        Feature::WfxT,
        Feature::Ras,
        Feature::RasV1p1,
        Feature::RasSaV1p1,
        Feature::Debugv8p1,
        Feature::Debugv8p2,
        Feature::Debugv8p4,
        Feature::Afp,
        Feature::Rpres,
        Feature::El2,
        Feature::Aa64El2,
        Feature::Sme2,
        Feature::Pmull,
        Feature::DoubleLock,
        Feature::TGran16K,
        Feature::TGran64K,
        Feature::S2TGran16K,
        Feature::S2TGran64K,
        Feature::Ttst,
        Feature::Lpa,
        Feature::Lva,
        Feature::Lpa2,
    ];
}

/// What the conformance tests read of a feature: `tests/unit/spec.rs`
/// checks the decoder against Arm's data feature by feature.
#[cfg(test)]
impl Feature {
    /// The name Arm's data gives the feature, `None` for the base set.
    pub(crate) const fn arm_name(self) -> Option<&'static str> {
        match self {
            Self::Base => None,
            Self::Fp => Some("FEAT_FP"),
            Self::AdvSimd => Some("FEAT_AdvSIMD"),
            Self::Aes => Some("FEAT_AES"),
            Self::Sha1 => Some("FEAT_SHA1"),
            Self::Sha256 => Some("FEAT_SHA256"),
            Self::Crc32 => Some("FEAT_CRC32"),
            Self::Sme => Some("FEAT_SME"),
            Self::Lse => Some("FEAT_LSE"),
            Self::Lse2 => Some("FEAT_LSE2"),
            Self::Lrcpc => Some("FEAT_LRCPC"),
            Self::Lrcpc2 => Some("FEAT_LRCPC2"),
            Self::Rdm => Some("FEAT_RDM"),
            Self::DotProd => Some("FEAT_DotProd"),
            Self::I8mm => Some("FEAT_I8MM"),
            Self::Fp16 => Some("FEAT_FP16"),
            Self::Fhm => Some("FEAT_FHM"),
            Self::Fcma => Some("FEAT_FCMA"),
            Self::Jscvt => Some("FEAT_JSCVT"),
            Self::Frintts => Some("FEAT_FRINTTS"),
            Self::Bf16 => Some("FEAT_BF16"),
            Self::PAuth => Some("FEAT_PAuth"),
            Self::PAuth2 => Some("FEAT_PAuth2"),
            Self::Fpac => Some("FEAT_FPAC"),
            Self::FpacCombine => Some("FEAT_FPACCOMBINE"),
            Self::PacQarma5 => Some("FEAT_PACQARMA5"),
            Self::Bti => Some("FEAT_BTI"),
            Self::FlagM => Some("FEAT_FlagM"),
            Self::FlagM2 => Some("FEAT_FlagM2"),
            Self::Sb => Some("FEAT_SB"),
            Self::Pan => Some("FEAT_PAN"),
            Self::Pan2 => Some("FEAT_PAN2"),
            Self::Pan3 => Some("FEAT_PAN3"),
            Self::Uao => Some("FEAT_UAO"),
            Self::Dit => Some("FEAT_DIT"),
            Self::Ssbs => Some("FEAT_SSBS"),
            Self::Ssbs2 => Some("FEAT_SSBS2"),
            Self::Hpds => Some("FEAT_HPDS"),
            Self::Ttcnp => Some("FEAT_TTCNP"),
            Self::E0pd => Some("FEAT_E0PD"),
            Self::Csv3 => Some("FEAT_CSV3"),
            Self::TlbiOs => Some("FEAT_TLBIOS"),
            Self::TlbiRange => Some("FEAT_TLBIRANGE"),
            Self::Ttl => Some("FEAT_TTL"),
            Self::Xs => Some("FEAT_XS"),
            Self::Dpb => Some("FEAT_DPB"),
            Self::Dpb2 => Some("FEAT_DPB2"),
            Self::Lor => Some("FEAT_LOR"),
            Self::Idst => Some("FEAT_IDST"),
            Self::SpecRes => Some("FEAT_SPECRES"),
            Self::Csv2 => Some("FEAT_CSV2"),
            Self::Ecv => Some("FEAT_ECV"),
            Self::EcvPoff => Some("FEAT_ECV_POFF"),
            Self::WfxT => Some("FEAT_WFxT"),
            Self::Ras => Some("FEAT_RAS"),
            Self::RasV1p1 => Some("FEAT_RASv1p1"),
            Self::RasSaV1p1 => Some("FEAT_RASSAv1p1"),
            Self::Debugv8p1 => Some("FEAT_Debugv8p1"),
            Self::Debugv8p2 => Some("FEAT_Debugv8p2"),
            Self::Debugv8p4 => Some("FEAT_Debugv8p4"),
            Self::Afp => Some("FEAT_AFP"),
            Self::Rpres => Some("FEAT_RPRES"),
            Self::El2 => Some("FEAT_EL2"),
            Self::Aa64El2 => Some("FEAT_AA64EL2"),
            Self::Sme2 => Some("FEAT_SME2"),
            Self::Pmull => Some("FEAT_PMULL"),
            Self::DoubleLock => Some("FEAT_DoubleLock"),
            Self::TGran16K => Some("FEAT_TGran16K"),
            Self::TGran64K => Some("FEAT_TGran64K"),
            Self::S2TGran16K => Some("FEAT_S2TGran16K"),
            Self::S2TGran64K => Some("FEAT_S2TGran64K"),
            Self::Ttst => Some("FEAT_TTST"),
            Self::Lpa => Some("FEAT_LPA"),
            Self::Lva => Some("FEAT_LVA"),
            Self::Lpa2 => Some("FEAT_LPA2"),
        }
    }

    /// Whether the feature has encodings of its own; one that has none
    /// changes how other instructions behave.
    pub(crate) const fn encodes(self) -> bool {
        !matches!(
            self,
            Self::Lse2
                | Self::PAuth2
                | Self::Fpac
                | Self::FpacCombine
                | Self::PacQarma5
                | Self::Pan
                | Self::Pan2
                | Self::Pan3
                | Self::Uao
                | Self::Dit
                | Self::Ssbs
                | Self::Ssbs2
                | Self::Hpds
                | Self::Ttcnp
                | Self::E0pd
                | Self::Csv3
                | Self::TlbiOs
                | Self::TlbiRange
                | Self::Ttl
                | Self::Dpb
                | Self::Dpb2
                | Self::Idst
                | Self::SpecRes
                | Self::Csv2
                | Self::Ecv
                | Self::EcvPoff
                | Self::RasV1p1
                | Self::RasSaV1p1
                | Self::Debugv8p1
                | Self::Debugv8p2
                | Self::Debugv8p4
                | Self::Afp
                | Self::Rpres
                | Self::El2
                | Self::Aa64El2
                | Self::Pmull
                | Self::DoubleLock
                | Self::TGran16K
                | Self::TGran64K
                | Self::S2TGran16K
                | Self::S2TGran64K
                | Self::Ttst
                | Self::Lpa
                | Self::Lva
                | Self::Lpa2
        )
    }

    /// Whether the decoder names every encoding of the feature: a feature
    /// still being built has some of its encodings, each of them Arm's, and
    /// no processor this crate builds implements it yet. No feature the
    /// decoder knows is being built.
    pub(crate) const fn complete(self) -> bool {
        true
    }
}

impl Feature {
    /// The feature's bit of a [`Features`] set, whose 128 bits hold every
    /// feature (checked where [`Feature::ALL`] is built).
    #[inline(always)]
    const fn bit(self) -> u128 {
        1 << self as u32
    }
}

/// A set of [`Feature`]s: what one processor implements, one bit per
/// feature, in declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Features(u128);

impl Features {
    /// What the Cortex-A57 this crate's processor is shaped like implements,
    /// as QEMU's `cortex-a57` has it: ARMv8.0 with FP, Advanced SIMD, the
    /// whole Cryptographic Extension (AES with the 64-bit PMULL, SHA-1 and
    /// SHA-256), CRC32 and the OS Double Lock (`ID_AA64DFR0_EL1.DoubleLock`
    /// 0, as the Cortex-A57 and QEMU report it); and of the translation
    /// granules, the 4 KiB one every processor has alone. Which others the
    /// core has is the Cortex-A57 TRM's (DDI 0488) to say, which is not on
    /// disk, and Arm's documents decide over QEMU, whose `cortex-a57`
    /// reports the 64 KiB one too (`ID_AA64MMFR0_EL1` 0x1124). The identity
    /// and extensions above are QEMU's report of the core.
    pub(crate) const CORTEX_A57: Self = Self::of(&[
        Feature::Base,
        Feature::Fp,
        Feature::AdvSimd,
        Feature::Aes,
        Feature::Pmull,
        Feature::Sha1,
        Feature::Sha256,
        Feature::Crc32,
        Feature::DoubleLock,
    ]);

    /// Every feature: a set no processor implements (FEAT_DoubleLock with
    /// FEAT_SME, for one), for what is decided once for whichever features
    /// a processor has, such as where each register a feature brings is
    /// encoded.
    pub(crate) const EVERY: Self = Self::of(&Feature::ALL);

    /// FEAT_EL2 in AArch64 state only (FEAT_AA64EL2 without FEAT_AA32EL2):
    /// what a processor of the level set [`WithEl2`](super::WithEl2) adds.
    const EL2: Self = Self(Feature::El2.bit() | Feature::Aa64El2.bit());

    /// The features only a processor with EL2 has, which one of the level
    /// set [`NoEl2`](super::NoEl2) drops with EL2: FEAT_ECV_POFF
    /// (`(FEAT_AA64 && FEAT_ECV_POFF) ==> FEAT_AA64EL2`), and the stage 2
    /// granules (`FEAT_S2TGran16K ==> (FEAT_AA64EL2 && FEAT_TGran16K)`).
    const NEED_EL2: Self =
        Self(Feature::EcvPoff.bit() | Feature::S2TGran16K.bit() | Feature::S2TGran64K.bit());

    /// What the Armv9.2 processor model implements: every feature Arm's
    /// `Features.json` requires of an Armv9.2 processor that has FP and
    /// Advanced SIMD and AArch64 at EL0 and EL1 alone (`v9Ap2 ==> (v9Ap1 &&
    /// v8Ap7)` down the versions, with what a version requires of a
    /// processor with FP or Advanced SIMD, such as `(v8Ap6 && FEAT_FP) ==>
    /// FEAT_BF16`, and what those features imply); FEAT_SME and FEAT_SME2;
    /// and these options: the Cryptographic Extension's AES, PMULL, SHA-1
    /// and SHA-256, QARMA5 as the pointer authentication algorithm,
    /// FEAT_FPAC and FEAT_FPACCOMBINE, FEAT_SSBS and FEAT_SSBS2,
    /// FEAT_RPRES, every translation granule at both stages, FEAT_TTST, and
    /// 52-bit physical and virtual addresses with every granule (FEAT_LPA,
    /// FEAT_LVA, FEAT_LPA2). Not FEAT_SVE: Streaming SVE mode is its only
    /// SVE. Not FEAT_DoubleLock, the one Cortex-A57 feature Armv9.0
    /// forbids.
    pub(crate) const ARMV9P2: Self = Self::of(&[
        Feature::Base,
        Feature::Fp,
        Feature::AdvSimd,
        // Required from Armv8.1
        Feature::Crc32,
        Feature::Lse,
        Feature::Rdm,
        Feature::Pan,
        Feature::Lor,
        Feature::Hpds,
        Feature::Debugv8p1,
        // Required from Armv8.2
        Feature::Fp16,
        Feature::Pan2,
        Feature::Uao,
        Feature::Dpb,
        Feature::Ttcnp,
        Feature::Ras,
        Feature::Debugv8p2,
        // Required from Armv8.3
        Feature::Lrcpc,
        Feature::Fcma,
        Feature::Jscvt,
        Feature::PAuth,
        // Required from Armv8.4
        Feature::Lse2,
        Feature::Lrcpc2,
        Feature::DotProd,
        Feature::Fhm,
        Feature::FlagM,
        Feature::Dit,
        Feature::Idst,
        Feature::TlbiOs,
        Feature::TlbiRange,
        Feature::Ttl,
        Feature::Debugv8p4,
        Feature::RasV1p1,
        Feature::RasSaV1p1,
        // Required from Armv8.5
        Feature::FlagM2,
        Feature::Frintts,
        Feature::Sb,
        Feature::SpecRes,
        Feature::Bti,
        Feature::E0pd,
        Feature::Csv2,
        Feature::Csv3,
        Feature::Dpb2,
        // Required from Armv8.6
        Feature::I8mm,
        Feature::Bf16,
        Feature::Ecv,
        Feature::PAuth2,
        // Required from Armv8.7
        Feature::Pan3,
        Feature::WfxT,
        Feature::Xs,
        Feature::Afp,
        // Options
        Feature::Sme,
        Feature::Sme2,
        Feature::Aes,
        Feature::Pmull,
        Feature::Sha1,
        Feature::Sha256,
        Feature::PacQarma5,
        Feature::Fpac,
        Feature::FpacCombine,
        Feature::Ssbs,
        Feature::Ssbs2,
        Feature::Rpres,
        Feature::TGran16K,
        Feature::TGran64K,
        Feature::S2TGran16K,
        Feature::S2TGran64K,
        Feature::Ttst,
        Feature::Lpa,
        Feature::Lva,
        Feature::Lpa2,
    ]);

    /// The set of `features`.
    pub(crate) const fn of(features: &[Feature]) -> Self {
        let mut bits = 0;
        let mut i = 0;
        while i < features.len() {
            bits |= features[i].bit();
            i += 1;
        }
        Self(bits)
    }

    /// Tested in the 64-bit half that holds `feature`'s bit: a variable
    /// test of the whole 128 bits costs the decoder's per-instruction
    /// feature check a double-word shift, which choosing the half first
    /// leaves out.
    #[inline(always)]
    pub(crate) const fn has(self, feature: Feature) -> bool {
        let bit = feature as u32;
        let half = if bit < 64 {
            self.0 as u64
        } else {
            (self.0 >> 64) as u64
        };
        half & (1 << (bit % 64)) != 0
    }

    /// The set as a bit per [`Feature`], in declaration order: what a
    /// snapshot records of the processor it was taken from.
    pub(crate) const fn bits(self) -> u128 {
        self.0
    }
}

/// A scalable vector length (the Arm ARM's `VecLen`): a power of two from
/// 128 to 2048 bits, which is every length this processor supports (Arm
/// `SupportedPowerTwoSVL`), and the only unit an SVE or SME length is
/// counted in here (R4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct VecLen(u16);

impl VecLen {
    /// The shortest length: 128 bits.
    pub(crate) const MIN: Self = Self(128);
    /// `MAX_VL`, the architecture's longest vector: 2048 bits.
    pub(crate) const MAX: Self = Self(2048);

    #[inline]
    pub(crate) const fn bits(self) -> u32 {
        self.0 as u32
    }

    /// The length in bytes: a vector register's, and the number of
    /// horizontal vectors of the ZA array at this length.
    #[inline]
    pub(crate) const fn bytes(self) -> usize {
        self.0 as usize / 8
    }

    /// The length of a predicate register for vectors of this length, in
    /// bytes: one bit per vector byte.
    #[inline]
    pub(crate) const fn predicate_bytes(self) -> usize {
        self.0 as usize / 64
    }

    /// Arm `ImplementedSMEVectorLength((len + 1) * 128)`: the longest
    /// supported length not above the one a `LEN` field of `SMCR_ELx` asks
    /// for, capped at `max`. Every power of two up to `max` is supported,
    /// so that is the largest power of two at most `min((len + 1) * 128,
    /// max)`.
    pub(crate) const fn from_len_field(len: u64, max: Self) -> Self {
        let requested = ((len & 0xF) as u32 + 1) * 128;
        let capped = if requested < max.0 as u32 {
            requested
        } else {
            max.0 as u32
        };
        Self((1u32 << (31 - capped.leading_zeros())) as u16)
    }
}

/// What one processor implements, what it calls itself, the IMPLEMENTATION
/// DEFINED choices the features it implements leave open, and the system
/// counter it runs with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Implementation {
    pub(crate) features: Features,
    /// `MIDR_EL1`: who made the processor and which part it is.
    pub(crate) midr: u64,
    /// `MaxImplementedSVL()`: the longest Streaming SVE vector length, which
    /// [`Feature::Sme`] reads and nothing else does.
    pub(crate) max_svl: VecLen,
    /// The system counter's frequency: what `CNTPCT_EL0` counts at and
    /// `CNTFRQ_EL0` reads at reset. The system's choice before Armv8.6,
    /// [`ARMV8P6_COUNTER_HZ`] from it.
    pub(crate) counter_hz: ClockHz,
}

impl Implementation {
    /// The Cortex-A57 shape: [`Features::CORTEX_A57`] and the Cortex-A57's
    /// `MIDR_EL1`, with no streaming vector length to choose, and QEMU's
    /// counter for it.
    pub(crate) const CORTEX_A57: Self = Self {
        features: Features::CORTEX_A57,
        midr: id::MIDR_CORTEX_A57,
        max_svl: VecLen::MIN,
        counter_hz: QEMU_BACKCOMPAT_COUNTER_HZ,
    };

    /// The Armv9.2 processor model: [`Features::ARMV9P2`], a software
    /// model's `MIDR_EL1`, Streaming SVE vectors of up to 512 bits, and the
    /// counter Armv8.6 and later require.
    pub(crate) const ARMV9P2: Self = Self {
        features: Features::ARMV9P2,
        midr: id::MIDR_SOFTWARE_MODEL,
        max_svl: VecLen(512),
        counter_hz: ARMV8P6_COUNTER_HZ,
    };

    /// This implementation as a processor of the level set `L` has it, with
    /// FEAT_EL2 and FEAT_AA64EL2 exactly when `L` has EL2: the processor's
    /// type is what decides them
    /// ([`Cpu::implementing`](super::Cpu::implementing)). Without EL2 it
    /// also drops the features only EL2 brings ([`Features::NEED_EL2`]);
    /// with it, it keeps those the implementation lists.
    pub(crate) const fn with_levels<L: Levels>(self) -> LeveledImplementation<L> {
        LeveledImplementation {
            imp: Self {
                features: Features(if L::EL2 {
                    self.features.0 | Features::EL2.0
                } else {
                    self.features.0 & !(Features::EL2.0 | Features::NEED_EL2.0)
                }),
                ..self
            },
            levels: PhantomData,
        }
    }

    #[inline(always)]
    pub(crate) const fn has(self, feature: Feature) -> bool {
        self.features.has(feature)
    }
}

/// An implementation as a processor of the level set `L` has it
/// ([`Implementation::with_levels`]): what a processor of `L` holds, and
/// all a processor of `L` is built from
/// ([`Cpu::leveled`](super::Cpu::leveled)), so its level set is the one it
/// was leveled for (R2). One leveled without EL2 has dropped the features
/// only EL2 brings, which leveling it again with EL2 could not bring back;
/// a plain [`Implementation`] comes out of it only by name ([`get`] or a
/// deref), never in its place.
///
/// [`get`]: LeveledImplementation::get
pub(crate) struct LeveledImplementation<L: Levels> {
    imp: Implementation,
    levels: PhantomData<L>,
}

// By hand: derived, each would ask of `L` what it asks of the struct, and
// a level set is a marker type that holds nothing.
impl<L: Levels> Clone for LeveledImplementation<L> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}

impl<L: Levels> Copy for LeveledImplementation<L> {}

impl<L: Levels> PartialEq for LeveledImplementation<L> {
    fn eq(&self, other: &Self) -> bool {
        self.imp == other.imp
    }
}

impl<L: Levels> Eq for LeveledImplementation<L> {}

impl<L: Levels> core::fmt::Debug for LeveledImplementation<L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.imp.fmt(f)
    }
}

impl<L: Levels> LeveledImplementation<L> {
    /// The implementation, leveled: what a `const fn` reads, where
    /// [`Deref`](core::ops::Deref) cannot be called.
    #[inline(always)]
    pub(crate) const fn get(self) -> Implementation {
        self.imp
    }
}

impl<L: Levels> core::ops::Deref for LeveledImplementation<L> {
    type Target = Implementation;

    #[inline(always)]
    fn deref(&self) -> &Implementation {
        &self.imp
    }
}

/// What the tests of a feature run on, whichever processor models
/// implement it.
#[cfg(test)]
impl Implementation {
    /// The Armv9.2 model with every feature the decoder knows, which is
    /// today the model's own set, FEAT_DoubleLock (a set no processor may
    /// have, Armv9.0 forbidding FEAT_DoubleLock, and none is built with) and
    /// FEAT_ECV_POFF, which it keeps only with EL2 and no model lists yet:
    /// a feature's tests run on it before any
    /// model implements the feature, and each exercises that feature's own
    /// rules. EL2 comes with the processor's level set, as it does for
    /// every implementation: on [`NoEl2`](super::NoEl2) it starts at EL1,
    /// which the instruction tests and the QEMU `max` vectors (taken on
    /// `virt` without virtualization) run at; on [`WithEl2`](super::WithEl2)
    /// it covers the rules where EL2 meets another feature (`CPTR_EL2`'s
    /// traps of SIMD&FP and SME, `SMCR_EL2`, `HCR_EL2.API` and `APK`, the
    /// EL2 regime's TBI, PAN and BTI).
    pub(crate) const EVERY_FEATURE: Self = Self {
        features: Features::EVERY,
        ..Self::ARMV9P2
    };

    /// [`Implementation::EVERY_FEATURE`] without the features `left_out`:
    /// what the tests of a feature's absence run on.
    pub(crate) const fn every_feature_but(left_out: &[Feature]) -> Self {
        Self {
            features: Features(Features::EVERY.0 & !Features::of(left_out).0),
            ..Self::EVERY_FEATURE
        }
    }
}

/// The system counter frequency QEMU gives the processor types it had
/// before it standardized on 1 GHz, the Cortex-A57 among them
/// (`GTIMER_BACKCOMPAT_HZ`, a 16 ns tick).
pub const QEMU_BACKCOMPAT_COUNTER_HZ: ClockHz = match ClockHz::new(62_500_000) {
    Some(hz) => hz,
    None => panic!("62.5 MHz is a rate"),
};

/// The system counter frequency of Armv8.6 and later, which fix it at 1 GHz
/// (Arm ARM, "The system counter"; QEMU `GTIMER_DEFAULT_HZ`).
pub const ARMV8P6_COUNTER_HZ: ClockHz = match ClockHz::new(1_000_000_000) {
    Some(hz) => hz,
    None => panic!("1 GHz is a rate"),
};

/// A processor a machine can be built with.
///
/// Closed (R5): each variant is one processor, its features, identity and
/// IMPLEMENTATION DEFINED choices, and the system counter a board pairs it
/// with. The discriminant is the model's number in a snapshot, so a model
/// is only ever added last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum CpuModel {
    /// A Cortex-A57 r1p0 as QEMU's `cortex-a57` is one: Armv8.0 with FP,
    /// Advanced SIMD, the Cryptographic Extension and CRC32, cut down to
    /// AArch64 at EL0 and EL1, with a 62.5 MHz system counter.
    #[default]
    CortexA57,
    /// An Armv9.2 processor with FEAT_SME and FEAT_SME2 and no FEAT_SVE:
    /// AArch64 at EL0 and EL1, every feature Armv9.2 requires of it, the
    /// options listed for its feature set, and Streaming SVE vectors of up
    /// to 512 bits. It reports itself as a software model (`MIDR_EL1`
    /// implementer 0), as no Arm part has its features, and runs with a
    /// 1 GHz system counter, as Armv8.6 and later require.
    Armv9p2,
}

impl CpuModel {
    /// Every model, each at the index of its discriminant (checked below,
    /// and in `each_model_is_named_and_identified`, whose exhaustive match
    /// does not compile once the enum gains a model until it is listed).
    pub const ALL: [CpuModel; 2] = [Self::CortexA57, Self::Armv9p2];

    /// The model's name for a command line or a configuration file, which
    /// [`str::parse`] reads back.
    pub const fn name(self) -> &'static str {
        match self {
            Self::CortexA57 => "cortex-a57",
            Self::Armv9p2 => "armv9.2",
        }
    }

    /// The frequency of the system counter a board pairs with this
    /// processor: what `CNTPCT_EL0` counts at and `CNTFRQ_EL0` reads at
    /// reset. QEMU runs the Cortex-A57's at [`QEMU_BACKCOMPAT_COUNTER_HZ`]
    /// (`ARM_FEATURE_BACKCOMPAT_CNTFRQ`); the Armv9.2 model's is
    /// [`ARMV8P6_COUNTER_HZ`].
    pub const fn counter_hz(self) -> ClockHz {
        self.implementation().counter_hz
    }

    /// The size of a physical address on this model, in bits (Arm
    /// `AArch64_PAMax`), which `ID_AA64MMFR0_EL1.PARange` reports: 44 on
    /// the Cortex-A57, 52 on the Armv9.2 model, which has FEAT_LPA.
    pub const fn physical_address_bits(self) -> u32 {
        id::pa_bits(self.implementation().features)
    }

    /// Whether a machine of this model can be built with EL2
    /// ([`MachineBuilder::with_el2`](crate::machine::MachineBuilder::with_el2)).
    /// The Cortex-A57 can, as QEMU's `-M virt,virtualization=on` gives its
    /// `cortex-a57` EL2. The Armv9.2 model cannot yet: Arm's feature model
    /// (`Features.json`) requires of an Armv9.2 processor with EL2 FEAT_VHE,
    /// FEAT_FGT, FEAT_HCX, FEAT_EVT, FEAT_EVT2, FEAT_S2FWB and FEAT_XNX,
    /// which this core does not implement, and a processor without them is
    /// not one Arm permits.
    pub const fn el2_available(self) -> bool {
        match self {
            Self::CortexA57 => true,
            Self::Armv9p2 => false,
        }
    }

    /// Whether a processor of this model may have the levels of `L`: every
    /// model without EL2, and with it one that [`CpuModel::el2_available`].
    /// The one check of a model's levels, which [`Cpu::new`](super::Cpu::new)
    /// and a machine's build both make.
    pub const fn check_levels<L: Levels>(self) -> Result<(), El2Unavailable> {
        if L::EL2 && !self.el2_available() {
            Err(El2Unavailable { model: self })
        } else {
            Ok(())
        }
    }

    /// The number a snapshot records the model by: its discriminant.
    pub(crate) const fn tag(self) -> u8 {
        self as u8
    }

    pub(crate) const fn implementation(self) -> Implementation {
        match self {
            Self::CortexA57 => Implementation::CORTEX_A57,
            Self::Armv9p2 => Implementation::ARMV9P2,
        }
    }

    /// The `compatible` of the processor's device tree node (Linux
    /// `Documentation/devicetree/bindings/arm/cpus.yaml`): the part's own,
    /// or `arm,armv8`, which the binding gives software models.
    pub(crate) const fn compatible(self) -> &'static str {
        match self {
            Self::CortexA57 => "arm,cortex-a57",
            Self::Armv9p2 => "arm,armv8",
        }
    }
}

const _: () = {
    let mut i = 0;
    while i < CpuModel::ALL.len() {
        assert!(
            CpuModel::ALL[i] as usize == i,
            "CpuModel::ALL lists each model at its discriminant"
        );
        i += 1;
    }
};

impl core::str::FromStr for CpuModel {
    type Err = UnknownCpuModel;

    /// The model whose [`CpuModel::name`] is `name`.
    fn from_str(name: &str) -> Result<Self, UnknownCpuModel> {
        Self::ALL
            .into_iter()
            .find(|model| model.name() == name)
            .ok_or(UnknownCpuModel)
    }
}

/// A name no [`CpuModel`] has. It prints the names there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnknownCpuModel;

impl core::fmt::Display for UnknownCpuModel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("no processor model has that name; the models are")?;
        for (i, model) in CpuModel::ALL.iter().enumerate() {
            let separator = if i == 0 { " " } else { ", " };
            write!(f, "{separator}\"{}\"", model.name())?;
        }
        Ok(())
    }
}

impl core::error::Error for UnknownCpuModel {}

/// A processor of a model asked for with EL2, which that model cannot have
/// ([`CpuModel::el2_available`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct El2Unavailable {
    /// The model refused EL2.
    pub model: CpuModel,
}

impl core::fmt::Display for El2Unavailable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "the {} processor model cannot have EL2: Arm requires features of it with EL2 that this core does not implement",
            self.model.name()
        )
    }
}

impl core::error::Error for El2Unavailable {}

#[cfg(test)]
mod tests {
    use super::super::{NoEl2, WithEl2};
    use super::*;

    /// A feature still being built is in no processor model: only the
    /// tests' own implementation has it. Every implementation has the
    /// features the Cortex-A57 has at the same exception levels, which
    /// `id`'s constants report, but FEAT_DoubleLock, which a processor with
    /// FEAT_SME (an Armv9.2 one) may not have.
    #[test]
    fn the_processors_built_implement_only_complete_features() {
        let every = Implementation::EVERY_FEATURE.with_levels::<WithEl2>();
        let a57 = [
            Implementation::CORTEX_A57.with_levels::<NoEl2>().get(),
            Implementation::CORTEX_A57.with_levels::<WithEl2>().get(),
        ];
        for model in CpuModel::ALL {
            let without = model.implementation().with_levels::<NoEl2>();
            let with = model.implementation().with_levels::<WithEl2>();
            for feature in Feature::ALL {
                for (imp, a57) in [without.get(), with.get()].into_iter().zip(a57) {
                    assert!(
                        feature.complete() || !imp.has(feature),
                        "{model:?}: {feature:?} is incomplete"
                    );
                    assert!(
                        !imp.has(feature) || every.has(feature),
                        "{model:?}: {feature:?}"
                    );
                    if feature == Feature::DoubleLock {
                        assert_ne!(
                            imp.has(feature),
                            imp.has(Feature::Sme),
                            "{model:?}: v9Ap0 ==> !FEAT_DoubleLock"
                        );
                    } else if a57.has(feature) {
                        assert!(imp.has(feature), "{model:?}: {feature:?}");
                    }
                }
                // The level set adds EL2 and keeps the features only EL2
                // brings that the model lists; it decides those alone.
                let el2 = matches!(feature, Feature::El2 | Feature::Aa64El2);
                let needs_el2 =
                    Features::NEED_EL2.has(feature) && model.implementation().has(feature);
                assert_eq!(
                    with.has(feature),
                    without.has(feature) || el2 || needs_el2,
                    "{model:?}: {feature:?}"
                );
                assert!(
                    !(el2 || Features::NEED_EL2.has(feature)) || !without.has(feature),
                    "{model:?}: {feature:?}"
                );
            }
            // Beside those features, the level set changes nothing.
            assert_eq!(
                Implementation {
                    features: without.features,
                    ..*with
                },
                *without
            );
        }
    }

    /// A processor without EL2 has no feature only EL2 brings
    /// (`(FEAT_AA64 && FEAT_ECV_POFF) ==> FEAT_AA64EL2`), and its ID
    /// registers say so: the implementation with every feature has
    /// FEAT_ECV_POFF with EL2 alone, and `ID_AA64MMFR0_EL1.ECV` reads 2 with
    /// it and 1, FEAT_ECV's, without.
    #[test]
    fn a_processor_without_el2_has_no_feature_only_el2_brings() {
        let with = Implementation::EVERY_FEATURE.with_levels::<WithEl2>();
        let without = Implementation::EVERY_FEATURE.with_levels::<NoEl2>();
        for feature in Feature::ALL {
            if Features::NEED_EL2.has(feature) {
                assert!(with.has(feature) && !without.has(feature), "{feature:?}");
            }
        }
        let ecv = |features: Features| (id::id_aa64mmfr0(features) >> 60) & 0xF;
        assert_eq!((ecv(with.features), ecv(without.features)), (2, 1));
    }

    /// QARMA5 is the one algorithm the pointer authentication instructions
    /// compute, so a processor with FEAT_PAuth implements FEAT_PACQARMA5,
    /// and its ID registers name the algorithm (Arm requires one:
    /// `FEAT_PAuth` with none of `FEAT_PACQARMA5`, `FEAT_PACQARMA3` and
    /// `FEAT_PACIMP` advertises no pointer authentication).
    #[test]
    fn a_processor_with_pauth_computes_with_qarma5() {
        let models = CpuModel::ALL.map(CpuModel::implementation);
        for imp in models.into_iter().chain([Implementation::EVERY_FEATURE]) {
            assert!(
                !imp.has(Feature::PAuth) || imp.has(Feature::PacQarma5),
                "{imp:?}"
            );
        }
    }

    /// Each model has a name of its own, which names it back; its
    /// identity, counter and device tree `compatible` are its part's, or a
    /// software model's: the Cortex-A57 is Arm's part 0xD07 with QEMU's
    /// 62.5 MHz counter, the Armv9.2 model implementer 0 with Armv8.6's
    /// 1 GHz.
    #[test]
    fn each_model_is_named_and_identified() {
        let mut listed = 0;
        for model in CpuModel::ALL {
            // Exhaustive: a model the enum gains fails to compile here
            // until `ALL` lists it and this counts it.
            listed += match model {
                CpuModel::CortexA57 | CpuModel::Armv9p2 => 1,
            };
            assert_eq!(model.name().parse::<CpuModel>(), Ok(model));
            assert_eq!(
                CpuModel::ALL
                    .iter()
                    .filter(|m| m.tag() == model.tag())
                    .count(),
                1,
                "{model:?}"
            );
        }
        assert_eq!(listed, 2);
        assert_eq!("cortex-a53".parse::<CpuModel>(), Err(UnknownCpuModel));
        assert_eq!(
            std::format!("{UnknownCpuModel}"),
            "no processor model has that name; the models are \"cortex-a57\", \"armv9.2\""
        );
        assert_eq!(CpuModel::default(), CpuModel::CortexA57);

        let a57 = CpuModel::CortexA57;
        assert_eq!(a57.implementation(), Implementation::CORTEX_A57);
        assert_eq!(
            (
                a57.implementation().midr >> 24,
                (a57.implementation().midr >> 4) & 0xFFF
            ),
            (0x41, 0xD07)
        );
        assert_eq!(a57.counter_hz().get(), 62_500_000);
        assert_eq!(a57.compatible(), "arm,cortex-a57");

        let v92 = CpuModel::Armv9p2;
        assert_eq!(v92.implementation(), Implementation::ARMV9P2);
        let midr = v92.implementation().midr;
        assert_eq!(
            (midr >> 24, (midr >> 16) & 0xF),
            (0, 0xF),
            "software use, features in the ID registers"
        );
        assert_ne!((midr >> 4) & 0xFFF, u64::from(b'Q'), "not QEMU's `max`");
        assert_eq!(v92.counter_hz().get(), 1_000_000_000);
        assert_eq!(v92.compatible(), "arm,armv8");
        assert!(
            v92.implementation().has(Feature::Sme2) && v92.implementation().max_svl == VecLen(512)
        );
    }

    /// `ImplementedSMEVectorLength`: `LEN` asks for `(LEN + 1) * 128`
    /// bits, and gets the longest power of two not above that or the
    /// implemented maximum.
    #[test]
    fn a_requested_length_rounds_down_to_a_supported_one() {
        let max = VecLen(512);
        let svl = |len: u64| VecLen::from_len_field(len, max).bits();
        assert_eq!(svl(0), 128);
        assert_eq!(svl(1), 256);
        assert_eq!(svl(2), 256, "384 is not a power of two");
        assert_eq!(svl(3), 512);
        assert_eq!(svl(7), 512, "1024 is above the maximum");
        assert_eq!(svl(15), 512);
        assert_eq!(VecLen::from_len_field(15, VecLen::MAX).bits(), 2048);
        assert_eq!(
            VecLen::from_len_field(0x1F, VecLen::MAX).bits(),
            2048,
            "LEN is four bits"
        );
    }
}
