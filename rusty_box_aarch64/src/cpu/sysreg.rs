//! `MRS`/`MSR` (register): which system registers exist, who may access
//! them, and what an access does.
//!
//! A register is named by its encoding `(op0, op1, CRn, CRm, op2)`, packed as
//! `MRS` packs it in bits 20:5. An encoding not listed is UNDEFINED, except the
//! unallocated part of the ID register space (`op0=3, op1=0, CRn=0,
//! CRm=1..7`), which reads as zero (Arm ARM D18.2, "reserved ID registers are
//! RAZ"). With FEAT_IDST, a read of the feature ID space (`op0=3, op1=0, 1
//! or 3, CRn=0, CRm=0..7`) that would be UNDEFINED traps with EC 0x18
//! instead: EL0's reads of the ID registers, and `UnimplementedIDRegister`.

use super::features::Feature;
use super::id;
use super::regs::{SysReg, SysRegRefusal};
use super::{sctlr, Cpu, El, Pstate, SysRegs};

/// Who is accessing a system register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Accessor {
    /// An `MRS`/`MSR` at the current exception level, with every trap and
    /// enable control applying.
    Guest,
    /// A host (a debugger, an automation script): EL1's privilege, and none
    /// of the controls that exist to trap a guest — `CPACR_EL1.FPEN`, the EL0
    /// enables, and the rule that `SP_EL0` cannot be named while it is the
    /// stack pointer in use.
    Host,
}

/// A system register encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SysRegKey(u32);

impl SysRegKey {
    pub(crate) const fn new(op0: u32, op1: u32, crn: u32, crm: u32, op2: u32) -> Self {
        Self(
            ((op0 & 3) << 14)
                | ((op1 & 7) << 11)
                | ((crn & 15) << 7)
                | ((crm & 15) << 3)
                | (op2 & 7),
        )
    }

    /// From an `MRS`/`MSR`/`SYS` instruction word.
    pub(crate) const fn from_insn(insn: u32) -> Self {
        Self((insn >> 5) & 0xFFFF)
    }

    /// From a host's name for the register: the same packing.
    pub(crate) const fn from_sysreg(reg: SysReg) -> Self {
        Self(reg.encoding() as u32)
    }

    const fn op0(self) -> u32 {
        self.0 >> 14
    }
    const fn op1(self) -> u32 {
        (self.0 >> 11) & 7
    }
    const fn crn(self) -> u32 {
        (self.0 >> 7) & 15
    }
    const fn crm(self) -> u32 {
        (self.0 >> 3) & 15
    }
    const fn op2(self) -> u32 {
        self.0 & 7
    }

    /// Whether the encoding is in the feature ID space FEAT_IDST traps
    /// reads of: `op0=3, op1=0, 1 or 3, CRn=0, CRm=0..7`, as QEMU's
    /// `arm_cpreg_encoding_in_idspace` has it.
    const fn in_id_space(self) -> bool {
        self.op0() == 3 && matches!(self.op1(), 0 | 1 | 3) && self.crn() == 0 && self.crm() < 8
    }

    /// `ESR_ELx.ISS` for a trapped `MSR`/`MRS` (EC 0x18).
    pub(crate) const fn trap_iss(self, rt: u32, read: bool) -> u32 {
        (self.op0() << 20)
            | (self.op2() << 17)
            | (self.op1() << 14)
            | (self.crn() << 10)
            | ((rt & 31) << 5)
            | (self.crm() << 1)
            | read as u32
    }
}

macro_rules! key {
    ($op0:expr, $op1:expr, $crn:expr, $crm:expr, $op2:expr) => {
        SysRegKey::new($op0, $op1, $crn, $crm, $op2).0
    };
}

const MIDR_EL1: u32 = key!(3, 0, 0, 0, 0);
const MPIDR_EL1: u32 = key!(3, 0, 0, 0, 5);
const REVIDR_EL1: u32 = key!(3, 0, 0, 0, 6);
const ID_AA64PFR0_EL1: u32 = key!(3, 0, 0, 4, 0);
const ID_AA64PFR1_EL1: u32 = key!(3, 0, 0, 4, 1);
const ID_AA64ZFR0_EL1: u32 = key!(3, 0, 0, 4, 4);
const ID_AA64SMFR0_EL1: u32 = key!(3, 0, 0, 4, 5);
const ID_AA64DFR0_EL1: u32 = key!(3, 0, 0, 5, 0);
const ID_AA64ISAR0_EL1: u32 = key!(3, 0, 0, 6, 0);
const ID_AA64ISAR1_EL1: u32 = key!(3, 0, 0, 6, 1);
const ID_AA64ISAR2_EL1: u32 = key!(3, 0, 0, 6, 2);
const ID_AA64MMFR0_EL1: u32 = key!(3, 0, 0, 7, 0);
const ID_AA64MMFR1_EL1: u32 = key!(3, 0, 0, 7, 1);
const ID_AA64MMFR2_EL1: u32 = key!(3, 0, 0, 7, 2);
const CCSIDR_EL1: u32 = key!(3, 1, 0, 0, 0);
const CLIDR_EL1: u32 = key!(3, 1, 0, 0, 1);
const SMIDR_EL1: u32 = key!(3, 1, 0, 0, 6);
const AIDR_EL1: u32 = key!(3, 1, 0, 0, 7);
const CSSELR_EL1: u32 = key!(3, 2, 0, 0, 0);
const CTR_EL0: u32 = key!(3, 3, 0, 0, 1);
const DCZID_EL0: u32 = key!(3, 3, 0, 0, 7);
const SCTLR_EL1: u32 = key!(3, 0, 1, 0, 0);
const ACTLR_EL1: u32 = key!(3, 0, 1, 0, 1);
const CPACR_EL1: u32 = key!(3, 0, 1, 0, 2);
const SMPRI_EL1: u32 = key!(3, 0, 1, 2, 4);
const SMCR_EL1: u32 = key!(3, 0, 1, 2, 6);
const TTBR0_EL1: u32 = key!(3, 0, 2, 0, 0);
const TTBR1_EL1: u32 = key!(3, 0, 2, 0, 1);
const TCR_EL1: u32 = key!(3, 0, 2, 0, 2);
const APIAKEYLO_EL1: u32 = key!(3, 0, 2, 1, 0);
const APIAKEYHI_EL1: u32 = key!(3, 0, 2, 1, 1);
const APIBKEYLO_EL1: u32 = key!(3, 0, 2, 1, 2);
const APIBKEYHI_EL1: u32 = key!(3, 0, 2, 1, 3);
const APDAKEYLO_EL1: u32 = key!(3, 0, 2, 2, 0);
const APDAKEYHI_EL1: u32 = key!(3, 0, 2, 2, 1);
const APDBKEYLO_EL1: u32 = key!(3, 0, 2, 2, 2);
const APDBKEYHI_EL1: u32 = key!(3, 0, 2, 2, 3);
const APGAKEYLO_EL1: u32 = key!(3, 0, 2, 3, 0);
const APGAKEYHI_EL1: u32 = key!(3, 0, 2, 3, 1);
const SPSR_EL1: u32 = key!(3, 0, 4, 0, 0);
const ELR_EL1: u32 = key!(3, 0, 4, 0, 1);
const SP_EL0: u32 = key!(3, 0, 4, 1, 0);
const SPSEL: u32 = key!(3, 0, 4, 2, 0);
const CURRENT_EL: u32 = key!(3, 0, 4, 2, 2);
const PAN: u32 = key!(3, 0, 4, 2, 3);
const UAO: u32 = key!(3, 0, 4, 2, 4);
const NZCV: u32 = key!(3, 3, 4, 2, 0);
const DAIF: u32 = key!(3, 3, 4, 2, 1);
const SVCR: u32 = key!(3, 3, 4, 2, 2);
const DIT: u32 = key!(3, 3, 4, 2, 5);
const SSBS: u32 = key!(3, 3, 4, 2, 6);
const FPCR: u32 = key!(3, 3, 4, 4, 0);
const FPSR: u32 = key!(3, 3, 4, 4, 1);
const AFSR0_EL1: u32 = key!(3, 0, 5, 1, 0);
const AFSR1_EL1: u32 = key!(3, 0, 5, 1, 1);
const ESR_EL1: u32 = key!(3, 0, 5, 2, 0);
const ERRIDR_EL1: u32 = key!(3, 0, 5, 3, 0);
const FAR_EL1: u32 = key!(3, 0, 6, 0, 0);
const PAR_EL1: u32 = key!(3, 0, 7, 4, 0);
const MAIR_EL1: u32 = key!(3, 0, 10, 2, 0);
const AMAIR_EL1: u32 = key!(3, 0, 10, 3, 0);
const LORSA_EL1: u32 = key!(3, 0, 10, 4, 0);
const LOREA_EL1: u32 = key!(3, 0, 10, 4, 1);
const LORN_EL1: u32 = key!(3, 0, 10, 4, 2);
const LORC_EL1: u32 = key!(3, 0, 10, 4, 3);
const LORID_EL1: u32 = key!(3, 0, 10, 4, 7);
const VBAR_EL1: u32 = key!(3, 0, 12, 0, 0);
// EL1 is the highest EL, so it has the reset management registers (Arm's
// data: `IsHighestEL(EL1) && FEAT_AA64`).
const RVBAR_EL1: u32 = key!(3, 0, 12, 0, 1);
const RMR_EL1: u32 = key!(3, 0, 12, 0, 2);
const ISR_EL1: u32 = key!(3, 0, 12, 1, 0);
const DISR_EL1: u32 = key!(3, 0, 12, 1, 1);
const CONTEXTIDR_EL1: u32 = key!(3, 0, 13, 0, 1);
const TPIDR_EL1: u32 = key!(3, 0, 13, 0, 4);
const TPIDR_EL0: u32 = key!(3, 3, 13, 0, 2);
const TPIDRRO_EL0: u32 = key!(3, 3, 13, 0, 3);
const TPIDR2_EL0: u32 = key!(3, 3, 13, 0, 5);
const CNTKCTL_EL1: u32 = key!(3, 0, 14, 1, 0);
const CNTFRQ_EL0: u32 = key!(3, 3, 14, 0, 0);
const CNTPCT_EL0: u32 = key!(3, 3, 14, 0, 1);
const CNTVCT_EL0: u32 = key!(3, 3, 14, 0, 2);
const CNTPCTSS_EL0: u32 = key!(3, 3, 14, 0, 5);
const CNTVCTSS_EL0: u32 = key!(3, 3, 14, 0, 6);
const CNTP_TVAL_EL0: u32 = key!(3, 3, 14, 2, 0);
const CNTP_CTL_EL0: u32 = key!(3, 3, 14, 2, 1);
const CNTP_CVAL_EL0: u32 = key!(3, 3, 14, 2, 2);
const CNTV_TVAL_EL0: u32 = key!(3, 3, 14, 3, 0);
const CNTV_CTL_EL0: u32 = key!(3, 3, 14, 3, 1);
const CNTV_CVAL_EL0: u32 = key!(3, 3, 14, 3, 2);
const MDCCINT_EL1: u32 = key!(2, 0, 0, 2, 0);
const MDSCR_EL1: u32 = key!(2, 0, 0, 2, 2);
const OSECCR_EL1: u32 = key!(2, 0, 0, 6, 2);
const MDCCSR_EL0: u32 = key!(2, 3, 0, 1, 0);
// The Debug Communications Channel data registers. `DBGDTRTX_EL0` is named
// `DBGDTRRX_EL0` when read.
const OSDTRRX_EL1: u32 = key!(2, 0, 0, 0, 2);
const OSDTRTX_EL1: u32 = key!(2, 0, 0, 3, 2);
const DBGDTR_EL0: u32 = key!(2, 3, 0, 4, 0);
const DBGDTRTX_EL0: u32 = key!(2, 3, 0, 5, 0);
const MDRAR_EL1: u32 = key!(2, 0, 1, 0, 0);
const OSLAR_EL1: u32 = key!(2, 0, 1, 0, 4);
const OSLSR_EL1: u32 = key!(2, 0, 1, 1, 4);
const OSDLR_EL1: u32 = key!(2, 0, 1, 3, 4);
const DBGPRCR_EL1: u32 = key!(2, 0, 1, 4, 4);
const DBGCLAIMSET_EL1: u32 = key!(2, 0, 7, 8, 6);
const DBGCLAIMCLR_EL1: u32 = key!(2, 0, 7, 9, 6);
const DBGAUTHSTATUS_EL1: u32 = key!(2, 0, 7, 14, 6);

/// A write to a pointer authentication key register: all 64 bits, with
/// nothing to invalidate.
fn set_key(half: &mut u64, value: u64) -> WriteEffect {
    *half = value;
    WriteEffect::None
}

/// Why an `MRS`/`MSR` did not complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SysRegError {
    /// The encoding does not exist, or not at this EL, or not in this
    /// direction: UNDEFINED.
    Undefined,
    /// Trapped to EL1 with EC 0x18: an EL0 access a control bit disables,
    /// or with FEAT_IDST a read of the feature ID space that would be
    /// UNDEFINED.
    TrapToEl1,
    /// SIMD&FP access disabled by `CPACR_EL1.FPEN`: EC 0x07.
    FpTrap,
    /// SME register access disabled by `CPACR_EL1.SMEN`: EC 0x1D, SMTC 0.
    SmeTrap,
}

/// What a register write invalidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteEffect {
    None,
    /// A register the table walk reads changed: flush the TLB.
    FlushTlb,
    /// A timer register changed: re-evaluate the timer outputs.
    Timer,
}

/// `CNTKCTL_EL1` EL0 access controls.
const EL0PCTEN: u64 = 1 << 0;
const EL0VCTEN: u64 = 1 << 1;
const EL0VTEN: u64 = 1 << 8;
const EL0PTEN: u64 = 1 << 9;
/// `CNTKCTL_EL1` bits 9:0: the EL0 access controls and the event stream's
/// `EVNTEN`, `EVNTDIR` and `EVNTI`.
const CNTKCTL_BASE: u64 = 0x3FF;

/// The `DISR_EL1` bits a processor with FEAT_RAS keeps: A (31), IDS (24),
/// and bits 23:0, an IMPLEMENTATION DEFINED syndrome with IDS set and AET,
/// EA and DFSC with it clear. Bits 23:0 outside those three are RES0 only
/// while IDS is clear, so they keep what was written; the bits RES0
/// whatever IDS holds read as zero.
const DISR_MASK: u64 = (1 << 31) | (1 << 24) | 0x00FF_FFFF;

/// `MDSCR_EL1.SS`: software step is enabled.
pub(super) const MDSCR_SS: u64 = 1;

/// `MDSCR_EL1.TDCC`: EL0's accesses to the Debug Communications Channel
/// registers trap to EL1.
const MDSCR_TDCC: u64 = 1 << 12;

/// `MDSCR_EL1.KDE`: debug exceptions are enabled at the EL they target.
pub(super) const MDSCR_KDE: u64 = 1 << 13;

/// `OSDLR_EL1.DLK`: the OS Double Lock is locked (FEAT_DoubleLock; RES0
/// without it).
pub(super) const OSDLR_DLK: u64 = 1;

/// `DBGPRCR_EL1.CORENPDRQ`: the core is asked not to power down, which
/// overrides the OS Double Lock.
pub(super) const DBGPRCR_CORENPDRQ: u64 = 1;

/// `RMR_EL1.AA64`: the processor resets into AArch64. RAO/WI, as there is
/// no AArch32 to reset into.
const RMR_AA64: u64 = 1;

/// `RMR_EL1.RR`: a Warm reset is requested.
pub(super) const RMR_RR: u64 = 1 << 1;

/// The Debug Communications Channel's flags, `EDSCR.TXfull` (DTRTX full)
/// and `EDSCR.RXfull` (DTRRX full). They are kept at their places in
/// `MDSCR_EL1`, bits 29 and 30, which views them as `MDCCSR_EL0` does at the
/// same bits (Arm ARM, DDI 0487, the `MDSCR_EL1`, `MDCCSR_EL0` and `EDSCR`
/// register descriptions).
const DCC_TXFULL: u64 = 1 << 29;
const DCC_RXFULL: u64 = 1 << 30;
const DCC_FLAGS: u64 = DCC_TXFULL | DCC_RXFULL;

/// The `MDSCR_EL1` fields that are the OS Lock save/restore view of `EDSCR`:
/// RXfull (30), TXfull (29), RXO (27), TXU (26), INTdis (23:22), TDA (21),
/// HDE (14) and ERR (6). An `MSR` writes them only while `OSLSR_EL1.OSLK` is
/// 1, and leaves them as they are while it is 0 (Arm ARM, DDI 0487, the
/// `MDSCR_EL1` description; Arm's ASL as the Sail arm-v9.3-a model carries
/// it, `__set_MDSCR_EL1`). That ASL gates TFO (31) and SC2 (19) the same way;
/// they belong to FEAT_TRF and FEAT_PCSRv8p2, which no processor model
/// here implements, and are RES0.
const MDSCR_SAVE_RESTORE: u64 =
    DCC_FLAGS | (1 << 27) | (1 << 26) | (0b11 << 22) | (1 << 21) | (1 << 14) | (1 << 6);

/// The `MDSCR_EL1` fields every processor model here has: the save/restore
/// fields, SS (0), TDCC (12), KDE (13) and MDE (15). Every other bit is
/// RES0 here (the bits later features define among them: TFO, SC2, EMBWE,
/// TTA, EnSPM, EHBWE, of features no model implements), and reads as zero
/// (Arm ARM, DDI 0487, the `MDSCR_EL1` description).
pub(super) const MDSCR_FIELDS: u64 =
    MDSCR_SAVE_RESTORE | MDSCR_SS | MDSCR_TDCC | MDSCR_KDE | (1 << 15);

/// `OSLSR_EL1.OSLK`: the OS Lock is locked.
pub(super) const OSLSR_OSLK: u64 = 1 << 1;

/// `SCTLR_EL1` FEAT_MTE fields: ITFSB, TCF0, TCF, ATA0, ATA, TCSO0, TCSO,
/// RES0 as no processor model here implements FEAT_MTE.
const SCTLR_MTE: u64 =
    (1 << 37) | (3 << 38) | (3 << 40) | (1 << 42) | (1 << 43) | (1 << 58) | (1 << 59);

/// `CPACR_EL1.FPEN`.
const CPACR_FPEN: u64 = 3 << 20;
/// `CPACR_EL1.SMEN` (FEAT_SME).
const CPACR_SMEN: u64 = 3 << 24;

/// `SMCR_EL1.LEN`. Bits 8:4 are RAZ/WI, and `FA64` RES0 without
/// FEAT_SME_FA64, which this core lacks.
const SMCR_LEN: u64 = 0xF;
/// `SMCR_EL1.EZT0` (FEAT_SME2): ZT0 is accessible at EL0 and EL1; RES0
/// without FEAT_SME2.
pub(crate) const SMCR_EZT0: u64 = 1 << 30;

impl Cpu {
    /// The `SMCR_EL1` bits this core keeps: `LEN`, and `EZT0` with
    /// FEAT_SME2.
    fn smcr_mask(&self) -> u64 {
        if self.implements(Feature::Sme2) {
            SMCR_LEN | SMCR_EZT0
        } else {
            SMCR_LEN
        }
    }

    /// The SME System register enable (`SVCR`, `SMCR_EL1`): what their
    /// accessors' `CPACR_EL1.SMEN` checks come to, at EL0 `SMEN != '11'`
    /// and at EL1 `SMEN IN {'x0'}`, which is `IsSMEEnabled` at either. A
    /// host is not trapped.
    fn sme_register_check(&self, el: El, accessor: Accessor) -> Result<(), SysRegError> {
        if accessor == Accessor::Guest && !self.sme_enabled_at(el) {
            Err(SysRegError::SmeTrap)
        } else {
            Ok(())
        }
    }

    /// Whether `sys`, read from a snapshot, holds SME register values an
    /// `MSR` on this processor can leave: `CPACR_EL1.SMEN`, `SMCR_EL1` and
    /// `TPIDR2_EL0` only with FEAT_SME, and `SMCR_EL1` with nothing outside
    /// the bits it keeps.
    pub(crate) fn sme_registers_restorable(&self, sys: &SysRegs) -> bool {
        let sme = self.implements(Feature::Sme);
        sys.cpacr_el1 & !self.cpacr_mask() == 0
            && sys.smcr_el1 & !self.smcr_mask() == 0
            && (sme || (sys.smcr_el1 == 0 && sys.tpidr2_el0 == 0))
    }

    /// The `CPACR_EL1` bits this core keeps: `FPEN`, and `SMEN` with
    /// FEAT_SME.
    fn cpacr_mask(&self) -> u64 {
        if self.implements(Feature::Sme) {
            CPACR_FPEN | CPACR_SMEN
        } else {
            CPACR_FPEN
        }
    }

    /// The `CCSIDR_EL1` value for the cache `CSSELR_EL1` selects.
    fn ccsidr(&self) -> u64 {
        match self.sys.csselr_el1 & 0xF {
            0 => id::CCSIDR_L1D,
            1 => id::CCSIDR_L1I,
            2 => id::CCSIDR_L2,
            _ => 0,
        }
    }

    /// The Debug Communications Channel registers EL0 may reach trap to EL1
    /// when `MDSCR_EL1.TDCC` is set, before the access has any effect (Arm
    /// ARM, DDI 0487, the `MDCCSR_EL0`, `DBGDTR_EL0`, `DBGDTRRX_EL0` and
    /// `DBGDTRTX_EL0` accessibility pseudocode with no EL2; QEMU
    /// target/arm/debug_helper.c `access_tdcc`).
    fn dcc_check(&self, el: El) -> Result<(), SysRegError> {
        if el == El::El0 && self.sys.mdscr_el1 & MDSCR_TDCC != 0 {
            Err(SysRegError::TrapToEl1)
        } else {
            Ok(())
        }
    }

    /// The `CNTKCTL_EL1` bits this core keeps: bits 9:0, and `EVNTIS` with
    /// FEAT_ECV.
    pub(crate) fn cntkctl_mask(&self) -> u64 {
        if self.implements(Feature::Ecv) {
            CNTKCTL_BASE | super::wait::EVNTIS
        } else {
            CNTKCTL_BASE
        }
    }

    /// The `DISR_EL1` bits this core keeps: [`DISR_MASK`] with FEAT_RAS,
    /// none without (there is no register).
    pub(crate) fn disr_mask(&self) -> u64 {
        if self.implements(Feature::Ras) {
            DISR_MASK
        } else {
            0
        }
    }

    fn el0_timer_check(&self, el: El, enable_bits: u64) -> Result<(), SysRegError> {
        if el == El::El0 && self.sys.cntkctl_el1 & enable_bits == 0 {
            Err(SysRegError::TrapToEl1)
        } else {
            Ok(())
        }
    }

    /// The value an `MRS` at the current EL reads from the register `key`
    /// names, without the read's side effect (see [`Cpu::sysreg_mrs`]).
    /// With FEAT_IDST, a read of the feature ID space that would be
    /// UNDEFINED traps instead (each ID register's accessor at EL0, and Arm
    /// `UnimplementedIDRegister` at any EL).
    #[inline]
    pub(crate) fn sysreg_read(&self, key: SysRegKey) -> Result<u64, SysRegError> {
        match self.sysreg_read_as(key, Accessor::Guest) {
            Err(SysRegError::Undefined) if key.in_id_space() && self.implements(Feature::Idst) => {
                Err(SysRegError::TrapToEl1)
            }
            result => result,
        }
    }

    /// `MRS`: read the register `key` names at the current EL, with the
    /// read's side effect. A read of `DBGDTRRX_EL0` or `DBGDTR_EL0` empties
    /// DTRRX: RXfull is cleared to 0 after the read (Arm ARM, DDI 0487, the
    /// `DBGDTRRX_EL0` and `DBGDTR_EL0` register descriptions). A read that
    /// traps has no effect.
    #[inline]
    pub(crate) fn sysreg_mrs(&mut self, key: SysRegKey) -> Result<u64, SysRegError> {
        let value = self.sysreg_read(key)?;
        if let DBGDTR_EL0 | DBGDTRTX_EL0 = key.0 {
            self.sys.mdscr_el1 &= !DCC_RXFULL;
        }
        Ok(value)
    }

    /// The exception level an accessor reads and writes at.
    fn access_level(&self, accessor: Accessor) -> El {
        match accessor {
            Accessor::Guest => self.pstate.el,
            Accessor::Host => El::El1,
        }
    }

    /// The value of the register `key` names, as `accessor` reads it. This
    /// read has no side effect; an `MRS`'s side effect is
    /// [`Cpu::sysreg_mrs`]'s.
    fn sysreg_read_as(&self, key: SysRegKey, accessor: Accessor) -> Result<u64, SysRegError> {
        let el = self.access_level(accessor);
        let el0 = el == El::El0;
        let k = key.0;

        // Registers EL0 may reach, each with its own control.
        match k {
            NZCV => return Ok(self.pstate.nzcv()),
            DAIF => {
                if el0 && self.sys.sctlr_el1 & sctlr::UMA == 0 {
                    return Err(SysRegError::TrapToEl1);
                }
                return Ok(self.pstate.daif());
            }
            FPCR | FPSR => {
                if accessor == Accessor::Guest && !self.fp_enabled() {
                    return Err(SysRegError::FpTrap);
                }
                return Ok(if k == FPCR { self.fpcr } else { self.fpsr });
            }
            CTR_EL0 => {
                if el0 && self.sys.sctlr_el1 & sctlr::UCT == 0 {
                    return Err(SysRegError::TrapToEl1);
                }
                return Ok(id::CTR_EL0);
            }
            DCZID_EL0 => {
                let prohibited = el0 && self.sys.sctlr_el1 & sctlr::DZE == 0;
                return Ok(id::DCZID_BLOCK_LOG2_WORDS | (u64::from(prohibited) << 4));
            }
            TPIDR_EL0 => return Ok(self.sys.tpidr_el0),
            TPIDRRO_EL0 => return Ok(self.sys.tpidrro_el0),
            DIT if self.implements(Feature::Dit) => {
                return Ok(if self.pstate.dit { Pstate::DIT } else { 0 })
            }
            SSBS if self.implements(Feature::Ssbs2) => {
                return Ok(if self.pstate.ssbs { Pstate::SSBS } else { 0 })
            }
            SVCR if self.implements(Feature::Sme) => {
                self.sme_register_check(el, accessor)?;
                return Ok(self.svcr.bits());
            }
            TPIDR2_EL0 if self.implements(Feature::Sme) => {
                if el0 && self.sys.sctlr_el1 & sctlr::ENTP2 == 0 {
                    return Err(SysRegError::TrapToEl1);
                }
                return Ok(self.sys.tpidr2_el0);
            }
            CNTFRQ_EL0 => {
                self.el0_timer_check(el, EL0PCTEN | EL0VCTEN)?;
                return Ok(self.sys.cntfrq_el0);
            }
            CNTPCT_EL0 => {
                self.el0_timer_check(el, EL0PCTEN)?;
                return Ok(self.counter().get());
            }
            CNTVCT_EL0 => {
                self.el0_timer_check(el, EL0VCTEN)?;
                return Ok(self.counter().get());
            }
            // The self-synchronized views of the counters (FEAT_ECV): every
            // read here is in program order already.
            CNTPCTSS_EL0 if self.implements(Feature::Ecv) => {
                self.el0_timer_check(el, EL0PCTEN)?;
                return Ok(self.counter().get());
            }
            CNTVCTSS_EL0 if self.implements(Feature::Ecv) => {
                self.el0_timer_check(el, EL0VCTEN)?;
                return Ok(self.counter().get());
            }
            CNTP_CTL_EL0 | CNTP_CVAL_EL0 | CNTP_TVAL_EL0 => {
                self.el0_timer_check(el, EL0PTEN)?;
                let ctl = self.sys.cntp_ctl_el0;
                let cval = self.sys.cntp_cval_el0;
                return Ok(match k {
                    CNTP_CTL_EL0 => self.timer_ctl_read(ctl, cval),
                    CNTP_CVAL_EL0 => cval,
                    _ => self.timer_tval_read(cval),
                });
            }
            CNTV_CTL_EL0 | CNTV_CVAL_EL0 | CNTV_TVAL_EL0 => {
                self.el0_timer_check(el, EL0VTEN)?;
                let ctl = self.sys.cntv_ctl_el0;
                let cval = self.sys.cntv_cval_el0;
                return Ok(match k {
                    CNTV_CTL_EL0 => self.timer_ctl_read(ctl, cval),
                    CNTV_CVAL_EL0 => cval,
                    _ => self.timer_tval_read(cval),
                });
            }
            // The Debug Communications Channel with no external debugger on
            // its far end (Arm ARM, DDI 0487, chapter H4, "The Debug
            // Communication Channel and Instruction Transfer Register").
            // `MDCCSR_EL0` reports the two flags; nothing ever empties DTRTX
            // or fills DTRRX, so TXfull stays set from the first write on
            // and RXfull is set only by a restore through `MDSCR_EL1`.
            MDCCSR_EL0 => {
                self.dcc_check(el)?;
                return Ok(self.sys.mdscr_el1 & DCC_FLAGS);
            }
            // `DBGDTRRX_EL0` (`DBGDTRTX_EL0`'s encoding, read) returns DTRRX
            // in bits [31:0], and `DBGDTR_EL0` returns DTRTX in bits [63:32]
            // and DTRRX in [31:0] (the `DBGDTRRX_EL0` and `DBGDTR_EL0`
            // descriptions; Arm's ASL `DBGDTR_EL0_read`). While RXfull is
            // clear the whole value is UNKNOWN; this model reads zero.
            // Emptying DTRRX is `sysreg_mrs`'s side effect.
            DBGDTR_EL0 | DBGDTRTX_EL0 => {
                self.dcc_check(el)?;
                if self.sys.mdscr_el1 & DCC_RXFULL == 0 {
                    return Ok(0);
                }
                let rx = u64::from(self.sys.dtrrx);
                return Ok(if k == DBGDTR_EL0 {
                    (u64::from(self.sys.dtrtx) << 32) | rx
                } else {
                    rx
                });
            }
            _ => {}
        }

        if el0 {
            return Err(SysRegError::Undefined);
        }

        let pauth = self.implements(Feature::PAuth);
        let keys = &self.pac_keys;
        let value = match k {
            // The save/restore views of DTRRX and DTRTX: a read returns the
            // data register and leaves RXfull and TXfull as they are, at
            // either state of the OS Lock (Arm ARM, DDI 0487, the
            // `OSDTRRX_EL1` and `OSDTRTX_EL1` descriptions, which deprecate
            // their use while the OS Lock is unlocked). Bits [63:32] are
            // RES0.
            OSDTRRX_EL1 => u64::from(self.sys.dtrrx),
            OSDTRTX_EL1 => u64::from(self.sys.dtrtx),
            MIDR_EL1 => self.imp.midr,
            MPIDR_EL1 => id::MPIDR_EL1,
            REVIDR_EL1 => id::REVIDR_EL1,
            ID_AA64PFR0_EL1 => id::id_aa64pfr0(self.imp.features),
            ID_AA64PFR1_EL1 => id::id_aa64pfr1(self.imp.features),
            ID_AA64ZFR0_EL1 => id::id_aa64zfr0(self.imp.features),
            ID_AA64SMFR0_EL1 => id::id_aa64smfr0(self.imp.features),
            ID_AA64DFR0_EL1 => id::id_aa64dfr0(self.imp.features),
            ID_AA64ISAR0_EL1 => id::id_aa64isar0(self.imp.features),
            ID_AA64ISAR1_EL1 => id::id_aa64isar1(self.imp.features),
            ID_AA64ISAR2_EL1 => id::id_aa64isar2(self.imp.features),
            ID_AA64MMFR0_EL1 => id::id_aa64mmfr0(self.imp.features),
            ID_AA64MMFR1_EL1 => id::id_aa64mmfr1(self.imp.features),
            ID_AA64MMFR2_EL1 => id::id_aa64mmfr2(self.imp.features),
            CCSIDR_EL1 => self.ccsidr(),
            CLIDR_EL1 => id::CLIDR_EL1,
            SMIDR_EL1 if self.implements(Feature::Sme) => id::SMIDR_EL1,
            AIDR_EL1 => 0,
            CSSELR_EL1 => self.sys.csselr_el1,
            SCTLR_EL1 => self.sys.sctlr_el1,
            ACTLR_EL1 => self.sys.actlr_el1,
            CPACR_EL1 => self.sys.cpacr_el1,
            SMCR_EL1 if self.implements(Feature::Sme) => {
                self.sme_register_check(el, accessor)?;
                self.sys.smcr_el1
            }
            // RES0: SMIDR_EL1.SMPS is 0, there are no streaming execution
            // priorities.
            SMPRI_EL1 if self.implements(Feature::Sme) => 0,
            TTBR0_EL1 => self.sys.ttbr0_el1,
            TTBR1_EL1 => self.sys.ttbr1_el1,
            TCR_EL1 => self.sys.tcr_el1,
            APIAKEYLO_EL1 if pauth => keys.ia.lo,
            APIAKEYHI_EL1 if pauth => keys.ia.hi,
            APIBKEYLO_EL1 if pauth => keys.ib.lo,
            APIBKEYHI_EL1 if pauth => keys.ib.hi,
            APDAKEYLO_EL1 if pauth => keys.da.lo,
            APDAKEYHI_EL1 if pauth => keys.da.hi,
            APDBKEYLO_EL1 if pauth => keys.db.lo,
            APDBKEYHI_EL1 if pauth => keys.db.hi,
            APGAKEYLO_EL1 if pauth => keys.ga.lo,
            APGAKEYHI_EL1 if pauth => keys.ga.hi,
            SPSR_EL1 => self.sys.spsr_el1,
            ELR_EL1 => self.sys.elr_el1,
            SP_EL0 => {
                // SP_EL0 is not accessible by name while it is the stack
                // pointer in use.
                if accessor == Accessor::Guest && !self.pstate.sp_sel {
                    return Err(SysRegError::Undefined);
                }
                self.sp_el0
            }
            SPSEL => u64::from(self.pstate.sp_sel),
            CURRENT_EL => self.pstate.el.index() << 2,
            PAN if self.implements(Feature::Pan) => {
                if self.pstate.pan {
                    Pstate::PAN
                } else {
                    0
                }
            }
            UAO if self.implements(Feature::Uao) => {
                if self.pstate.uao {
                    Pstate::UAO
                } else {
                    0
                }
            }
            AFSR0_EL1 => self.sys.afsr0_el1,
            AFSR1_EL1 => self.sys.afsr1_el1,
            ESR_EL1 => self.sys.esr_el1,
            FAR_EL1 => self.sys.far_el1,
            PAR_EL1 => self.sys.par_el1,
            MAIR_EL1 => self.sys.mair_el1,
            AMAIR_EL1 => self.sys.amair_el1,
            // With no LORegions (`LORID_EL1`), the LORegion registers hold
            // nothing: RAZ/WI, as QEMU's `lor_reginfo` has them.
            LORSA_EL1 | LOREA_EL1 | LORN_EL1 | LORC_EL1 if self.implements(Feature::Lor) => 0,
            LORID_EL1 if self.implements(Feature::Lor) => id::LORID_EL1,
            VBAR_EL1 => self.sys.vbar_el1,
            // The IMPLEMENTATION DEFINED address execution starts from after
            // reset: the one the machine's last reset started it from.
            RVBAR_EL1 => self.sys.rvbar_el1,
            RMR_EL1 => RMR_AA64 | self.sys.rmr_el1,
            ISR_EL1 => (u64::from(self.irq_line) << 7) | (u64::from(self.fiq_line) << 6),
            CONTEXTIDR_EL1 => self.sys.contextidr_el1,
            TPIDR_EL1 => self.sys.tpidr_el1,
            CNTKCTL_EL1 => self.sys.cntkctl_el1,
            MDCCINT_EL1 => self.sys.mdccint_el1,
            MDSCR_EL1 => self.sys.mdscr_el1,
            OSECCR_EL1 => self.sys.oseccr_el1,
            // No ROM table for an external debugger to find: `Valid` 0b00,
            // `ROMADDR` not valid, as QEMU reads it (`debug_cp_reginfo`).
            MDRAR_EL1 => 0,
            OSLSR_EL1 => self.sys.oslsr_el1,
            OSDLR_EL1 => self.sys.osdlr_el1,
            DBGPRCR_EL1 => self.sys.dbgprcr_el1,
            // Every one of the eight claim tags is implemented: their
            // `DBGCLAIMSET_EL1` bits read as one ("Access to this field is
            // RAO/W1S", DDI 0601's `DBGCLAIMSET_EL1`), and
            // `DBGCLAIMCLR_EL1` reads the tags themselves, as QEMU's
            // `dbgclaimset_read` and `DBGCLAIMCLR_EL1` have them.
            DBGCLAIMSET_EL1 => 0xFF,
            DBGCLAIMCLR_EL1 => self.sys.dbgclaim,
            DBGAUTHSTATUS_EL1 => id::dbgauthstatus_el1(self.imp.features),
            // With no error records (`ERRIDR_EL1.NUM` = 0), `ERRSELR_EL1`
            // and the `ERX*` registers are UNDEFINED, one of the outcomes
            // DDI 0601's pages of those registers permit then, as QEMU's
            // `minimal_ras_reginfo` has them.
            ERRIDR_EL1 if self.implements(Feature::Ras) => id::ERRIDR_EL1,
            DISR_EL1 if self.implements(Feature::Ras) => self.sys.disr_el1,
            _ => {
                if let Some(v) = self.debug_breakpoint_read(key) {
                    v
                } else if key.op0() == 3
                    && key.op1() == 0
                    && key.crn() == 0
                    && (1..=7).contains(&key.crm())
                {
                    0
                } else {
                    return Err(SysRegError::Undefined);
                }
            }
        };
        Ok(value)
    }

    /// The breakpoint and watchpoint registers for the two of each that
    /// `ID_AA64DFR0_EL1` advertises: `DBGBVR<n>`, `DBGBCR<n>`, `DBGWVR<n>`,
    /// `DBGWCR<n>` at `op0=2, op1=0, CRn=0, CRm=n, op2=4..7`.
    fn debug_breakpoint_read(&self, key: SysRegKey) -> Option<u64> {
        if key.op0() != 2 || key.op1() != 0 || key.crn() != 0 || key.crm() > 1 {
            return None;
        }
        let n = key.crm() as usize;
        match key.op2() {
            4 => Some(self.sys.dbgbvr[n]),
            5 => Some(self.sys.dbgbcr[n]),
            6 => Some(self.sys.dbgwvr[n]),
            7 => Some(self.sys.dbgwcr[n]),
            _ => None,
        }
    }

    fn debug_breakpoint_write(&mut self, key: SysRegKey, value: u64) -> bool {
        if key.op0() != 2 || key.op1() != 0 || key.crn() != 0 || key.crm() > 1 {
            return false;
        }
        let n = key.crm() as usize;
        match key.op2() {
            4 => self.sys.dbgbvr[n] = value,
            5 => self.sys.dbgbcr[n] = value,
            6 => self.sys.dbgwvr[n] = value,
            7 => self.sys.dbgwcr[n] = value,
            _ => return false,
        }
        true
    }

    /// `MSR`: write `value` to the register `key` names at the current EL.
    #[inline]
    pub(crate) fn sysreg_write(
        &mut self,
        key: SysRegKey,
        value: u64,
    ) -> Result<WriteEffect, SysRegError> {
        self.sysreg_write_as(key, value, Accessor::Guest)
    }

    /// Write `value` to the register `key` names, as `accessor` writes it.
    fn sysreg_write_as(
        &mut self,
        key: SysRegKey,
        value: u64,
        accessor: Accessor,
    ) -> Result<WriteEffect, SysRegError> {
        let el = self.access_level(accessor);
        let el0 = el == El::El0;
        let k = key.0;

        match k {
            NZCV => {
                self.pstate.set_nzcv(value);
                return Ok(WriteEffect::None);
            }
            DAIF => {
                if el0 && self.sys.sctlr_el1 & sctlr::UMA == 0 {
                    return Err(SysRegError::TrapToEl1);
                }
                self.pstate.set_daif(value);
                return Ok(WriteEffect::None);
            }
            FPCR | FPSR => {
                if accessor == Accessor::Guest && !self.fp_enabled() {
                    return Err(SysRegError::FpTrap);
                }
                if k == FPCR {
                    self.fpcr = value & super::state::fpcr_mask(self.imp);
                } else {
                    // N, Z, C, V (bits 31:28), QC and the cumulative
                    // exception bits.
                    self.fpsr = value & super::state::FPSR_MASK;
                }
                return Ok(WriteEffect::None);
            }
            TPIDR_EL0 => {
                self.sys.tpidr_el0 = value;
                return Ok(WriteEffect::None);
            }
            DIT if self.implements(Feature::Dit) => {
                self.pstate.dit = value & Pstate::DIT != 0;
                return Ok(WriteEffect::None);
            }
            SSBS if self.implements(Feature::Ssbs2) => {
                self.pstate.ssbs = value & Pstate::SSBS != 0;
                return Ok(WriteEffect::None);
            }
            // Arm `SVCR` setter: `SetPSTATE_SM(value<0>)`, then
            // `SetPSTATE_ZA(value<1>)`.
            SVCR if self.implements(Feature::Sme) => {
                self.sme_register_check(el, accessor)?;
                self.set_pstate_sm(value & 1 != 0);
                self.set_pstate_za(value & 2 != 0);
                return Ok(WriteEffect::None);
            }
            TPIDR2_EL0 if self.implements(Feature::Sme) => {
                if el0 && self.sys.sctlr_el1 & sctlr::ENTP2 == 0 {
                    return Err(SysRegError::TrapToEl1);
                }
                self.sys.tpidr2_el0 = value;
                return Ok(WriteEffect::None);
            }
            CNTP_CTL_EL0 | CNTP_CVAL_EL0 | CNTP_TVAL_EL0 => {
                self.el0_timer_check(el, EL0PTEN)?;
                match k {
                    CNTP_CTL_EL0 => self.sys.cntp_ctl_el0 = value & 3,
                    CNTP_CVAL_EL0 => self.sys.cntp_cval_el0 = value,
                    _ => self.sys.cntp_cval_el0 = self.timer_cval_from_tval(value),
                }
                return Ok(WriteEffect::Timer);
            }
            CNTV_CTL_EL0 | CNTV_CVAL_EL0 | CNTV_TVAL_EL0 => {
                self.el0_timer_check(el, EL0VTEN)?;
                match k {
                    CNTV_CTL_EL0 => self.sys.cntv_ctl_el0 = value & 3,
                    CNTV_CVAL_EL0 => self.sys.cntv_cval_el0 = value,
                    _ => self.sys.cntv_cval_el0 = self.timer_cval_from_tval(value),
                }
                return Ok(WriteEffect::Timer);
            }
            // A write fills DTRTX with bits [31:0]: TXfull is set to 1 after
            // the write (Arm ARM, DDI 0487, the `DBGDTRTX_EL0` and
            // `DBGDTR_EL0` register descriptions; Arm's ASL
            // `DBGDTR_EL0_set`), and only an external debugger's read of
            // DTRTX clears it again. `DBGDTR_EL0`'s bits [63:32] go to DTRRX
            // and leave RXfull as it is. A write while TXfull is already set
            // writes UNKNOWN values; this model leaves both data registers
            // as they were.
            DBGDTR_EL0 | DBGDTRTX_EL0 => {
                self.dcc_check(el)?;
                if self.sys.mdscr_el1 & DCC_TXFULL == 0 {
                    self.sys.dtrtx = value as u32;
                    if k == DBGDTR_EL0 {
                        self.sys.dtrrx = (value >> 32) as u32;
                    }
                }
                self.sys.mdscr_el1 |= DCC_TXFULL;
                return Ok(WriteEffect::None);
            }
            _ => {}
        }

        if el0 {
            return Err(SysRegError::Undefined);
        }

        let pauth = self.implements(Feature::PAuth);
        let keys = &mut self.pac_keys;
        let effect = match k {
            // A write to a save/restore view sets the data register from bits
            // [31:0] and leaves its flag as it is, at either state of the OS
            // Lock (the `OSDTRRX_EL1` and `OSDTRTX_EL1` descriptions).
            OSDTRRX_EL1 => {
                self.sys.dtrrx = value as u32;
                WriteEffect::None
            }
            OSDTRTX_EL1 => {
                self.sys.dtrtx = value as u32;
                WriteEffect::None
            }
            APIAKEYLO_EL1 if pauth => set_key(&mut keys.ia.lo, value),
            APIAKEYHI_EL1 if pauth => set_key(&mut keys.ia.hi, value),
            APIBKEYLO_EL1 if pauth => set_key(&mut keys.ib.lo, value),
            APIBKEYHI_EL1 if pauth => set_key(&mut keys.ib.hi, value),
            APDAKEYLO_EL1 if pauth => set_key(&mut keys.da.lo, value),
            APDAKEYHI_EL1 if pauth => set_key(&mut keys.da.hi, value),
            APDBKEYLO_EL1 if pauth => set_key(&mut keys.db.lo, value),
            APDBKEYHI_EL1 if pauth => set_key(&mut keys.db.hi, value),
            APGAKEYLO_EL1 if pauth => set_key(&mut keys.ga.lo, value),
            APGAKEYHI_EL1 if pauth => set_key(&mut keys.ga.hi, value),
            CSSELR_EL1 => {
                self.sys.csselr_el1 = value & 0xF;
                WriteEffect::None
            }
            // The memory-tagging controls read as zero on a core without
            // FEAT_MTE, and a write that changes nothing keeps the TLB
            // (QEMU target/arm/helper.c `sctlr_write`).
            SCTLR_EL1 => {
                let value = value & !SCTLR_MTE;
                if value == self.sys.sctlr_el1 {
                    WriteEffect::None
                } else {
                    self.sys.sctlr_el1 = value;
                    WriteEffect::FlushTlb
                }
            }
            ACTLR_EL1 => {
                self.sys.actlr_el1 = value;
                WriteEffect::None
            }
            CPACR_EL1 => {
                self.sys.cpacr_el1 = value & self.cpacr_mask();
                WriteEffect::None
            }
            SMCR_EL1 if self.implements(Feature::Sme) => {
                self.sme_register_check(el, accessor)?;
                self.sys.smcr_el1 = value & self.smcr_mask();
                WriteEffect::None
            }
            SMPRI_EL1 if self.implements(Feature::Sme) => WriteEffect::None,
            TTBR0_EL1 => {
                self.sys.ttbr0_el1 = value;
                WriteEffect::FlushTlb
            }
            TTBR1_EL1 => {
                self.sys.ttbr1_el1 = value;
                WriteEffect::FlushTlb
            }
            TCR_EL1 => {
                self.sys.tcr_el1 = value;
                WriteEffect::FlushTlb
            }
            SPSR_EL1 => {
                self.sys.spsr_el1 = value & self.spsr_mask();
                WriteEffect::None
            }
            ELR_EL1 => {
                self.sys.elr_el1 = value;
                WriteEffect::None
            }
            SP_EL0 => {
                if accessor == Accessor::Guest && !self.pstate.sp_sel {
                    return Err(SysRegError::Undefined);
                }
                self.sp_el0 = value;
                WriteEffect::None
            }
            SPSEL => {
                // EL0 has only SP_EL0. A guest at EL0 never gets here (its
                // MSR is UNDEFINED above); a host write made while the
                // processor is at EL0 leaves SP_EL0 selected, as
                // `set_pstate` does, so the state stays one SPSR can encode.
                self.pstate.sp_sel = value & 1 != 0 && self.pstate.el == El::El1;
                WriteEffect::None
            }
            PAN if self.implements(Feature::Pan) => {
                self.pstate.pan = value & Pstate::PAN != 0;
                WriteEffect::None
            }
            UAO if self.implements(Feature::Uao) => {
                self.pstate.uao = value & Pstate::UAO != 0;
                WriteEffect::None
            }
            AFSR0_EL1 => {
                self.sys.afsr0_el1 = value;
                WriteEffect::None
            }
            AFSR1_EL1 => {
                self.sys.afsr1_el1 = value;
                WriteEffect::None
            }
            ESR_EL1 => {
                self.sys.esr_el1 = value & 0xFFFF_FFFF;
                WriteEffect::None
            }
            FAR_EL1 => {
                self.sys.far_el1 = value;
                WriteEffect::None
            }
            PAR_EL1 => {
                self.sys.par_el1 = value;
                WriteEffect::None
            }
            MAIR_EL1 => {
                self.sys.mair_el1 = value;
                WriteEffect::FlushTlb
            }
            AMAIR_EL1 => {
                self.sys.amair_el1 = value;
                WriteEffect::None
            }
            LORSA_EL1 | LOREA_EL1 | LORN_EL1 | LORC_EL1 if self.implements(Feature::Lor) => {
                WriteEffect::None
            }
            DISR_EL1 if self.implements(Feature::Ras) => {
                self.sys.disr_el1 = value & self.disr_mask();
                WriteEffect::None
            }
            VBAR_EL1 => {
                self.sys.vbar_el1 = value & !0x7FF;
                WriteEffect::None
            }
            CONTEXTIDR_EL1 => {
                self.sys.contextidr_el1 = value & 0xFFFF_FFFF;
                WriteEffect::None
            }
            TPIDR_EL1 => {
                self.sys.tpidr_el1 = value;
                WriteEffect::None
            }
            TPIDRRO_EL0 => {
                self.sys.tpidrro_el0 = value;
                WriteEffect::None
            }
            CNTKCTL_EL1 => {
                // The event stream's events so far are judged by the
                // controls that generated them.
                self.fold_event_stream();
                self.sys.cntkctl_el1 = value & self.cntkctl_mask();
                WriteEffect::None
            }
            // Writable only at the highest implemented EL, which is EL1 here.
            // It reports a frequency; it does not change the counter's rate.
            CNTFRQ_EL0 => {
                self.sys.cntfrq_el0 = value & 0xFFFF_FFFF;
                WriteEffect::None
            }
            MDCCINT_EL1 => {
                self.sys.mdccint_el1 = value;
                WriteEffect::None
            }
            // The save/restore fields, the channel's TXfull and RXfull among
            // them, are read-only while the OS Lock is unlocked and written
            // through to `EDSCR` while it is locked (Arm ARM, DDI 0487,
            // `MDSCR_EL1` and `EDSCR`). Unlocked, a read of them is UNKNOWN;
            // this model's reads return what they hold at every OS Lock
            // state. A RES0 bit is not stored.
            MDSCR_EL1 => {
                let kept = if self.sys.oslsr_el1 & OSLSR_OSLK != 0 {
                    0
                } else {
                    MDSCR_SAVE_RESTORE
                };
                self.sys.mdscr_el1 = (value & MDSCR_FIELDS & !kept) | (self.sys.mdscr_el1 & kept);
                WriteEffect::None
            }
            OSECCR_EL1 => {
                self.sys.oseccr_el1 = value & 0xFFFF_FFFF;
                WriteEffect::None
            }
            OSLAR_EL1 => {
                // OSLAR_EL1.OSLK sets OSLSR_EL1.OSLK (bit 1).
                self.sys.oslsr_el1 = (self.sys.oslsr_el1 & !2) | ((value & 1) << 1);
                WriteEffect::None
            }
            // DLK is RES0 without FEAT_DoubleLock.
            OSDLR_EL1 => {
                self.sys.osdlr_el1 = if self.implements(Feature::DoubleLock) {
                    value & OSDLR_DLK
                } else {
                    0
                };
                WriteEffect::None
            }
            DBGPRCR_EL1 => {
                self.sys.dbgprcr_el1 = value & DBGPRCR_CORENPDRQ;
                WriteEffect::None
            }
            // RR asks the system for a Warm reset of the processor. The
            // board has no reset controller that acts on the request (as
            // QEMU's `virt` has none), so it is held and nothing resets;
            // AA64 is RAO/WI.
            RMR_EL1 => {
                self.sys.rmr_el1 = value & RMR_RR;
                WriteEffect::None
            }
            DBGCLAIMSET_EL1 => {
                self.sys.dbgclaim |= value & 0xFF;
                WriteEffect::None
            }
            DBGCLAIMCLR_EL1 => {
                self.sys.dbgclaim &= !(value & 0xFF);
                WriteEffect::None
            }
            _ => {
                if self.debug_breakpoint_write(key, value) {
                    WriteEffect::None
                } else {
                    return Err(SysRegError::Undefined);
                }
            }
        };
        Ok(effect)
    }
}

impl Cpu {
    /// A host's read of `reg`: what an EL1 `MRS` returns, with no control
    /// that traps a guest in the way. Reading has no side effects.
    ///
    /// # Errors
    ///
    /// [`SysRegRefusal::Absent`] for a register this processor does not
    /// implement, [`SysRegRefusal::WriteOnly`] for one that cannot be read.
    pub fn sysreg(&self, reg: SysReg) -> Result<u64, SysRegRefusal> {
        let key = SysRegKey::from_sysreg(reg);
        match self.sysreg_read_as(key, Accessor::Host) {
            Ok(value) => Ok(value),
            Err(_) if key.0 == OSLAR_EL1 => Err(SysRegRefusal::WriteOnly),
            // A host reads at EL1 past the FP and SME enables and the EL0
            // controls, so only UNDEFINED is left to come back.
            Err(
                SysRegError::Undefined
                | SysRegError::TrapToEl1
                | SysRegError::FpTrap
                | SysRegError::SmeTrap,
            ) => Err(SysRegRefusal::Absent),
        }
    }

    /// A host's write of `reg`, with the effect an EL1 `MSR` has: a write to
    /// a register the table walk reads flushes the TLB, and a timer write is
    /// reported for the machine to re-evaluate its lines.
    pub(crate) fn host_sysreg_write(
        &mut self,
        reg: SysReg,
        value: u64,
    ) -> Result<WriteEffect, SysRegRefusal> {
        let key = SysRegKey::from_sysreg(reg);
        match self.sysreg_write_as(key, value, Accessor::Host) {
            Ok(WriteEffect::FlushTlb) => {
                self.flush_tlb();
                Ok(WriteEffect::FlushTlb)
            }
            Ok(WriteEffect::Timer) => {
                self.timer_dirty = true;
                Ok(WriteEffect::Timer)
            }
            Ok(WriteEffect::None) => Ok(WriteEffect::None),
            // As for reads, a host write can only be UNDEFINED: either the
            // register is read-only, or there is no such register.
            Err(
                SysRegError::Undefined
                | SysRegError::TrapToEl1
                | SysRegError::FpTrap
                | SysRegError::SmeTrap,
            ) => {
                if self.sysreg_read_as(key, Accessor::Host).is_ok() {
                    Err(SysRegRefusal::ReadOnly)
                } else {
                    Err(SysRegRefusal::Absent)
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/sysreg.rs"]
mod tests;
