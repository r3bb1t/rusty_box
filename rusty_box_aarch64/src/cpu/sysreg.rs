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
//!
//! With FEAT_EL2 the registers of `op1` = 4 are EL2's, UNDEFINED below it,
//! and EL2's controls trap EL0's and EL1's accesses to EL2 in the order each
//! register's accessor checks them. Provenance (R7): Arm's machine-readable
//! register data (AARCHMRS `Registers.json`, release 2026-09), each
//! register's `MRS` and `MSR` accessor at each level, read for a processor
//! with no EL3, `HCR_EL2.E2H` = 0 (no FEAT_VHE) and no FEAT_NV or FEAT_FGT.
//! A trap to EL1 is [`Route::Class`] (`AArch64_SystemAccessTraptoEL1orEL2`
//! from EL0, which `HCR_EL2.TGE` takes to EL2), a trap an EL2 control raises
//! [`Route::El2`] (`AArch64_SystemAccessTrap(EL2, ..)`).

use super::el2::{
    has_contextidr_el2, El2Reg, CNTHCTL_EL1PCEN, CNTHCTL_EL1PCTEN, CNTHCTL_EL1TVCT, CNTHCTL_EL1TVT,
    CPTR_TCPAC, HCR_AMO, HCR_APK, HCR_FIEN, HCR_TACR, HCR_TERR, HCR_TID1, HCR_TID2, HCR_TID3,
    HCR_TIDCP, HCR_TLOR, HCR_TRVM, HCR_TVM, MDCR_TDA, MDCR_TDE, MDCR_TDOSA, MDCR_TDRA,
};
use super::exception::Route;
use super::features::{Feature, Features};
use super::id;
use super::mmu::{El10Context, El2RegimeContext};
use super::pauth::KeyHalf;
use super::regs::{SysReg, SysRegRefusal};
use super::timer::{GenericTimer, Tval};
use super::{sctlr, Cpu, El, Levels, Pstate, SysRegs};

/// Who is accessing a system register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Accessor {
    /// An `MRS`/`MSR` at the current exception level, with every trap and
    /// enable control applying.
    Guest,
    /// A host (a debugger, an automation script): the highest implemented
    /// level's privilege, and none of the controls that exist to trap a
    /// guest — `CPACR_EL1.FPEN`, `CPTR_EL2`, the EL0 enables, and the rule
    /// that `SP_EL0` cannot be named while it is the stack pointer in use.
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
    /// reads of: `op0=3, op1=0, 1 or 3, CRn=0, CRm=0..7`.
    const fn in_id_space(self) -> bool {
        self.op0() == 3 && matches!(self.op1(), 0 | 1 | 3) && self.crn() == 0 && self.crm() < 8
    }

    /// Whether the encoding is in ID group 3, which `HCR_EL2.TID3` traps
    /// EL1's reads of: `op0=3, op1=0, CRn=0, CRm=1..7`, the reserved
    /// encodings included (for those reading zero, trapping is the
    /// IMPLEMENTATION DEFINED choice made here).
    const fn in_id_group3(self) -> bool {
        self.op0() == 3 && self.op1() == 0 && self.crn() == 0 && self.crm() >= 1 && self.crm() <= 7
    }

    /// Whether the encoding is in the IMPLEMENTATION DEFINED space: the
    /// instructions `S1_<op1>_C11_*` and `S1_<op1>_C15_*` (`SYS`, `SYSL`)
    /// and the registers `S3_<op1>_C11_*` and `S3_<op1>_C15_*` (`MSR`,
    /// `MRS`), none of which exists on this processor
    /// ([`Cpu::impdef_space_access`] says what an access does).
    pub(crate) const fn in_impdef_space(self) -> bool {
        (self.op0() == 1 || self.op0() == 3) && (self.crn() == 11 || self.crn() == 15)
    }

    /// Whether the encoding names an EL2 register (`op1` = 4).
    const fn is_el2(self) -> bool {
        self.op1() == 4
    }

    /// The encoding's slot in [`EL2_STORAGE`], its `CRn`, `CRm` and `op2`,
    /// where it is a register's of `op1` 4 (`op0` 3).
    const fn el2_slot(self) -> Option<usize> {
        if self.op0() == 3 && self.is_el2() {
            Some((self.0 & ((1 << 11) - 1)) as usize)
        } else {
            None
        }
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

/// The key register encoding `k` names, if it names one: the one map from
/// the ten encodings to the key halves they hold.
const fn key_half(k: u32) -> Option<KeyHalf> {
    Some(match k {
        APIAKEYLO_EL1 => KeyHalf::IaLo,
        APIAKEYHI_EL1 => KeyHalf::IaHi,
        APIBKEYLO_EL1 => KeyHalf::IbLo,
        APIBKEYHI_EL1 => KeyHalf::IbHi,
        APDAKEYLO_EL1 => KeyHalf::DaLo,
        APDAKEYHI_EL1 => KeyHalf::DaHi,
        APDBKEYLO_EL1 => KeyHalf::DbLo,
        APDBKEYHI_EL1 => KeyHalf::DbHi,
        APGAKEYLO_EL1 => KeyHalf::GaLo,
        APGAKEYHI_EL1 => KeyHalf::GaHi,
        _ => return None,
    })
}

/// A breakpoint or watchpoint register, of the two of each that
/// `ID_AA64DFR0_EL1` advertises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DebugPoint {
    reg: DebugPointReg,
    /// Which breakpoint or watchpoint: 0 or 1.
    n: usize,
}

/// A breakpoint's value or control register (`DBGBVR<n>_EL1`,
/// `DBGBCR<n>_EL1`), or a watchpoint's (`DBGWVR<n>_EL1`, `DBGWCR<n>_EL1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DebugPointReg {
    Bvr,
    Bcr,
    Wvr,
    Wcr,
}

impl DebugPoint {
    /// The register `key` names, if it names one (`op0=2, op1=0, CRn=0,
    /// CRm=n, op2=4..7`): the one decoding of these encodings.
    fn of(key: SysRegKey) -> Option<Self> {
        if key.op0() != 2 || key.op1() != 0 || key.crn() != 0 || key.crm() > 1 {
            return None;
        }
        let reg = match key.op2() {
            4 => DebugPointReg::Bvr,
            5 => DebugPointReg::Bcr,
            6 => DebugPointReg::Wvr,
            7 => DebugPointReg::Wcr,
            _ => return None,
        };
        Some(Self {
            reg,
            n: key.crm() as usize,
        })
    }
}
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
const ERRSELR_EL1: u32 = key!(3, 0, 5, 3, 1);
const ERXFR_EL1: u32 = key!(3, 0, 5, 4, 0);
const ERXCTLR_EL1: u32 = key!(3, 0, 5, 4, 1);
const ERXSTATUS_EL1: u32 = key!(3, 0, 5, 4, 2);
const ERXADDR_EL1: u32 = key!(3, 0, 5, 4, 3);
const ERXPFGF_EL1: u32 = key!(3, 0, 5, 4, 4);
const ERXPFGCTL_EL1: u32 = key!(3, 0, 5, 4, 5);
const ERXPFGCDN_EL1: u32 = key!(3, 0, 5, 4, 6);
const ERXMISC0_EL1: u32 = key!(3, 0, 5, 5, 0);
const ERXMISC1_EL1: u32 = key!(3, 0, 5, 5, 1);
const ERXMISC2_EL1: u32 = key!(3, 0, 5, 5, 2);
const ERXMISC3_EL1: u32 = key!(3, 0, 5, 5, 3);
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
// The reset management registers of EL1 where it is the highest EL (Arm's
// data: `IsHighestEL(EL1) && FEAT_AA64`); with FEAT_EL2, EL2's
// (`RVBAR_EL2`, `RMR_EL2`) are.
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

// The EL2 registers (FEAT_EL2).
const VPIDR_EL2: u32 = key!(3, 4, 0, 0, 0);
const VMPIDR_EL2: u32 = key!(3, 4, 0, 0, 5);
const SCTLR_EL2: u32 = key!(3, 4, 1, 0, 0);
const ACTLR_EL2: u32 = key!(3, 4, 1, 0, 1);
const HCR_EL2: u32 = key!(3, 4, 1, 1, 0);
const MDCR_EL2: u32 = key!(3, 4, 1, 1, 1);
const CPTR_EL2: u32 = key!(3, 4, 1, 1, 2);
const HSTR_EL2: u32 = key!(3, 4, 1, 1, 3);
const HACR_EL2: u32 = key!(3, 4, 1, 1, 7);
const SMPRIMAP_EL2: u32 = key!(3, 4, 1, 2, 5);
const SMCR_EL2: u32 = key!(3, 4, 1, 2, 6);
const TTBR0_EL2: u32 = key!(3, 4, 2, 0, 0);
const TCR_EL2: u32 = key!(3, 4, 2, 0, 2);
const VTTBR_EL2: u32 = key!(3, 4, 2, 1, 0);
const VTCR_EL2: u32 = key!(3, 4, 2, 1, 2);
const SPSR_EL2: u32 = key!(3, 4, 4, 0, 0);
const ELR_EL2: u32 = key!(3, 4, 4, 0, 1);
const SP_EL1: u32 = key!(3, 4, 4, 1, 0);
const SPSR_IRQ: u32 = key!(3, 4, 4, 3, 0);
const SPSR_ABT: u32 = key!(3, 4, 4, 3, 1);
const SPSR_UND: u32 = key!(3, 4, 4, 3, 2);
const SPSR_FIQ: u32 = key!(3, 4, 4, 3, 3);
const AFSR0_EL2: u32 = key!(3, 4, 5, 1, 0);
const AFSR1_EL2: u32 = key!(3, 4, 5, 1, 1);
const ESR_EL2: u32 = key!(3, 4, 5, 2, 0);
const VSESR_EL2: u32 = key!(3, 4, 5, 2, 3);
const FAR_EL2: u32 = key!(3, 4, 6, 0, 0);
const HPFAR_EL2: u32 = key!(3, 4, 6, 0, 4);
const MAIR_EL2: u32 = key!(3, 4, 10, 2, 0);
const AMAIR_EL2: u32 = key!(3, 4, 10, 3, 0);
const VBAR_EL2: u32 = key!(3, 4, 12, 0, 0);
const RVBAR_EL2: u32 = key!(3, 4, 12, 0, 1);
const RMR_EL2: u32 = key!(3, 4, 12, 0, 2);
const VDISR_EL2: u32 = key!(3, 4, 12, 1, 1);
const CONTEXTIDR_EL2: u32 = key!(3, 4, 13, 0, 1);
const TPIDR_EL2: u32 = key!(3, 4, 13, 0, 2);
const CNTVOFF_EL2: u32 = key!(3, 4, 14, 0, 3);
const CNTPOFF_EL2: u32 = key!(3, 4, 14, 0, 6);
const CNTHCTL_EL2: u32 = key!(3, 4, 14, 1, 0);
const CNTHP_TVAL_EL2: u32 = key!(3, 4, 14, 2, 0);
const CNTHP_CTL_EL2: u32 = key!(3, 4, 14, 2, 1);
const CNTHP_CVAL_EL2: u32 = key!(3, 4, 14, 2, 2);

/// The encoding an `MRS` and an `MSR` reach EL2 register `reg` by, held as
/// written, on a processor implementing `features`: `None` where the
/// processor lacks the register, and for `CNTHP_CTL_EL2`, whose `ISTATUS`
/// the timer computes (its own arms read and write it). The one map from
/// [`El2Reg`] to encodings, exhaustive (R5): a register added to the list
/// is given its encoding here, or does not compile. A register has one
/// encoding whatever the features; they decide only whether it is there,
/// which [`EL2_STORAGE`], built for every feature, relies on.
const fn el2_encoding(reg: El2Reg, features: Features) -> Option<u32> {
    const fn present(encoding: u32, has: bool) -> Option<u32> {
        if has {
            Some(encoding)
        } else {
            None
        }
    }
    match reg {
        El2Reg::Hcr => Some(HCR_EL2),
        El2Reg::Sctlr => Some(SCTLR_EL2),
        El2Reg::Actlr => Some(ACTLR_EL2),
        El2Reg::Cptr => Some(CPTR_EL2),
        El2Reg::Hstr => Some(HSTR_EL2),
        El2Reg::Hacr => Some(HACR_EL2),
        El2Reg::Mdcr => Some(MDCR_EL2),
        El2Reg::Tcr => Some(TCR_EL2),
        El2Reg::Ttbr0 => Some(TTBR0_EL2),
        El2Reg::Vtcr => Some(VTCR_EL2),
        El2Reg::Vttbr => Some(VTTBR_EL2),
        El2Reg::Mair => Some(MAIR_EL2),
        El2Reg::Amair => Some(AMAIR_EL2),
        El2Reg::Vbar => Some(VBAR_EL2),
        El2Reg::Esr => Some(ESR_EL2),
        El2Reg::Far => Some(FAR_EL2),
        El2Reg::Hpfar => Some(HPFAR_EL2),
        El2Reg::Elr => Some(ELR_EL2),
        El2Reg::Spsr => Some(SPSR_EL2),
        El2Reg::Afsr0 => Some(AFSR0_EL2),
        El2Reg::Afsr1 => Some(AFSR1_EL2),
        El2Reg::Tpidr => Some(TPIDR_EL2),
        El2Reg::Vpidr => Some(VPIDR_EL2),
        El2Reg::Vmpidr => Some(VMPIDR_EL2),
        El2Reg::Smcr => present(SMCR_EL2, features.has(Feature::Sme)),
        El2Reg::Cnthctl => Some(CNTHCTL_EL2),
        El2Reg::Cntvoff => Some(CNTVOFF_EL2),
        El2Reg::Cntpoff => present(CNTPOFF_EL2, features.has(Feature::EcvPoff)),
        El2Reg::CnthpCtl => None,
        El2Reg::CnthpCval => Some(CNTHP_CVAL_EL2),
        El2Reg::Contextidr => present(CONTEXTIDR_EL2, has_contextidr_el2(features)),
        El2Reg::Vdisr => present(VDISR_EL2, features.has(Feature::Ras)),
        El2Reg::Vsesr => present(VSESR_EL2, features.has(Feature::Ras)),
    }
}

/// The EL2 register held as written that each encoding of `op1` 4 names,
/// by its slot ([`SysRegKey::el2_slot`]), on a processor implementing every
/// feature: [`el2_encoding`] turned around, once, so finding a register is
/// one load. Each register's encoding is one of `op0` 3 and `op1` 4, no two
/// share one, and none has another with no feature or any one feature, or
/// this does not compile.
static EL2_STORAGE: [Option<El2Reg>; 1 << 11] = {
    let mut table = [None; 1 << 11];
    let mut i = 0;
    while i < El2Reg::ALL.len() {
        let reg = El2Reg::ALL[i];
        if let Some(encoding) = el2_encoding(reg, Features::EVERY) {
            let Some(slot) = SysRegKey(encoding).el2_slot() else {
                panic!("an EL2 register's encoding is of op0 3 and op1 4");
            };
            assert!(table[slot].is_none(), "one EL2 register per encoding");
            table[slot] = Some(reg);
            /// Whether `reg` has no encoding but `encoding` with `features`.
            const fn only(reg: El2Reg, features: Features, encoding: u32) -> bool {
                match el2_encoding(reg, features) {
                    Some(other) => other == encoding,
                    None => true,
                }
            }
            assert!(
                only(reg, Features::of(&[]), encoding),
                "one encoding per EL2 register"
            );
            let mut f = 0;
            while f < Feature::ALL.len() {
                assert!(
                    only(reg, Features::of(&[Feature::ALL[f]]), encoding),
                    "one encoding per EL2 register"
                );
                f += 1;
            }
        } else {
            assert!(
                el2_encoding(reg, Features::of(&[])).is_none(),
                "an EL2 register no feature set gives an encoding to has none"
            );
        }
        i += 1;
    }
    table
};

/// Why an `MRS`/`MSR` did not complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SysRegError {
    /// The encoding does not exist, or not at this EL, or not in this
    /// direction: UNDEFINED.
    Undefined,
    /// Trapped with EC 0x18 (`AArch64_SystemAccessTrap`): [`Route::Class`]
    /// for a trap to EL1, which `HCR_EL2.TGE` takes from EL0 to EL2
    /// (`AArch64_SystemAccessTraptoEL1orEL2`), or with FEAT_IDST to the
    /// current level (`UnimplementedIDRegister`); [`Route::El2`] for the
    /// trap an EL2 control raises.
    Trap(Route),
    /// SIMD&FP access disabled: EC 0x07, by `CPACR_EL1.FPEN`
    /// ([`Route::Class`]) or `CPTR_EL2.TFP` ([`Route::El2`]).
    FpTrap(Route),
    /// SME register access disabled: EC 0x1D, SMTC 0, by `CPACR_EL1.SMEN`
    /// ([`Route::Class`]) or `CPTR_EL2.TSM` ([`Route::El2`]).
    SmeTrap(Route),
}

/// What a register write invalidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteEffect {
    None,
    /// What the EL2 regime's table walk reads of its registers changed, or
    /// a host wrote a translation register: flush the TLB.
    FlushTlb,
    /// One of the EL1&0 regime's translation registers changed: the TLB's
    /// entries of its context stay apart ([`Cpu::el10_context_changed`]).
    El10Context,
    /// A write that may move a timer: of a timer's control or `TVAL`
    /// register, or of an EL1 timer's compare register, whatever the
    /// value; or of an EL2 register the timers read
    /// ([`El2Reg::feeds_timers`]) that changed what they read of the EL2
    /// registers ([`Cpu::el2_timer_inputs`]). Re-evaluate the timer outputs.
    Timer,
}

/// The regime whose translations a register is part of: the one list of
/// the translation registers, read by every write of one.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranslationRegister {
    /// The EL1&0 regime's ([`El10Context`]): its stage 1 registers
    /// (`SCTLR_EL1`, `TCR_EL1`, `TTBR0_EL1`, `TTBR1_EL1`, `MAIR_EL1`),
    /// stage 2's (`VTTBR_EL2`, `VTCR_EL2`), and `HCR_EL2`, whose `VM`, `DC`,
    /// `PTW` and `TGE` decide which walks are made.
    El10,
    /// The EL2 regime's ([`El2RegimeContext`]): `SCTLR_EL2`, `TCR_EL2`,
    /// `TTBR0_EL2` and `MAIR_EL2`.
    El2,
}

impl TranslationRegister {
    /// The regime the register below EL2 encoded `key` belongs to, if it is
    /// a translation register.
    const fn of_el1(key: u32) -> Option<Self> {
        match key {
            SCTLR_EL1 | TCR_EL1 | TTBR0_EL1 | TTBR1_EL1 | MAIR_EL1 => Some(Self::El10),
            _ => None,
        }
    }

    /// The regime the EL2 register `reg` belongs to, if it is a
    /// translation register.
    const fn of_el2(reg: El2Reg) -> Option<Self> {
        match reg {
            El2Reg::Sctlr | El2Reg::Tcr | El2Reg::Ttbr0 | El2Reg::Mair => Some(Self::El2),
            El2Reg::Hcr | El2Reg::Vtcr | El2Reg::Vttbr => Some(Self::El10),
            El2Reg::Actlr
            | El2Reg::Cptr
            | El2Reg::Hstr
            | El2Reg::Hacr
            | El2Reg::Mdcr
            | El2Reg::Amair
            | El2Reg::Vbar
            | El2Reg::Esr
            | El2Reg::Far
            | El2Reg::Hpfar
            | El2Reg::Elr
            | El2Reg::Spsr
            | El2Reg::Afsr0
            | El2Reg::Afsr1
            | El2Reg::Tpidr
            | El2Reg::Vpidr
            | El2Reg::Vmpidr
            | El2Reg::Smcr
            | El2Reg::Cnthctl
            | El2Reg::Cntvoff
            | El2Reg::Cntpoff
            | El2Reg::CnthpCtl
            | El2Reg::CnthpCval
            | El2Reg::Contextidr
            | El2Reg::Vdisr
            | El2Reg::Vsesr => None,
        }
    }
}

/// What a translation reads of one regime's registers, taken before a
/// write of one of them to compare with after it.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranslationContext {
    El10(El10Context),
    El2(El2RegimeContext),
}

/// The trap to EL1 an EL0 control raises (`AArch64_SystemAccessTraptoEL1orEL2`)
/// when `trapped`.
#[inline]
const fn trap_to_el1(trapped: bool) -> Result<(), SysRegError> {
    if trapped {
        Err(SysRegError::Trap(Route::Class))
    } else {
        Ok(())
    }
}

/// The trap to EL2 an EL2 control raises (`AArch64_SystemAccessTrap(EL2,
/// 0x18)`) when `trapped`.
#[inline]
const fn trap_to_el2(trapped: bool) -> Result<(), SysRegError> {
    if trapped {
        Err(SysRegError::Trap(Route::El2))
    } else {
        Ok(())
    }
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
/// whatever IDS holds read as zero. `VDISR_EL2` with EL1 in AArch64 has
/// the same layout.
pub(crate) const DISR_MASK: u64 = (1 << 31) | (1 << 24) | 0x00FF_FFFF;

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

/// `RMR_ELx.AA64`: the processor resets into AArch64. RAO/WI, as there is
/// no AArch32 to reset into.
const RMR_AA64: u64 = 1;

/// `RMR_ELx.RR`: a Warm reset is requested.
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
pub(crate) const CPACR_FPEN: u64 = 3 << 20;
/// `CPACR_EL1.SMEN` (FEAT_SME).
pub(crate) const CPACR_SMEN: u64 = 3 << 24;

/// `SMCR_EL1.LEN`, and `SMCR_EL2.LEN`. Bits 8:4 are RAZ/WI, and `FA64`
/// RES0 without FEAT_SME_FA64, which this core lacks.
pub(crate) const SMCR_LEN: u64 = 0xF;
/// `SMCR_EL1.EZT0` and `SMCR_EL2.EZT0` (FEAT_SME2): ZT0 is accessible at
/// EL0 and EL1, and at EL0, EL1 and EL2; RES0 without FEAT_SME2.
pub(crate) const SMCR_EZT0: u64 = 1 << 30;

/// The `SMCR_EL1` and `SMCR_EL2` bits a processor implementing `features`
/// (with FEAT_SME) keeps: `LEN`, and `EZT0` with FEAT_SME2.
pub(crate) const fn smcr_mask(features: Features) -> u64 {
    if features.has(Feature::Sme2) {
        SMCR_LEN | SMCR_EZT0
    } else {
        SMCR_LEN
    }
}

/// `DCZID_EL0.DZP`: `DC ZVA` and its kind are prohibited.
const DCZID_DZP: u64 = 1 << 4;

impl<L: Levels> Cpu<L> {
    /// The SME System register enable (`SVCR`, `SMCR_EL1`, `SMCR_EL2`), as
    /// their accessors check it ([`Cpu::sme_access_trap_route`]). A host is
    /// not trapped.
    fn sme_register_check(&self, accessor: Accessor) -> Result<(), SysRegError> {
        match self.sme_access_trap_route() {
            Some(route) if accessor != Accessor::Host => Err(SysRegError::SmeTrap(route)),
            Some(_) | None => Ok(()),
        }
    }

    /// The SIMD&FP register enable (`FPCR`, `FPSR`), as their accessors
    /// check it ([`Cpu::fp_access_trap_route`]; the trap to EL1 is an
    /// uncategorized exception to EL2 under `HCR_EL2.TGE`). A host is not
    /// trapped.
    fn fp_register_check(&self, accessor: Accessor) -> Result<(), SysRegError> {
        match self.fp_access_trap_route() {
            Some(route) if accessor != Accessor::Host => Err(SysRegError::FpTrap(route)),
            Some(_) | None => Ok(()),
        }
    }

    /// Whether `sys`, read from a snapshot, holds SME register values an
    /// `MSR` on this processor can leave: `CPACR_EL1.SMEN`, `SMCR_EL1` and
    /// `TPIDR2_EL0` only with FEAT_SME, and `SMCR_EL1` with nothing outside
    /// the bits it keeps.
    pub(crate) fn sme_registers_restorable(&self, sys: &SysRegs) -> bool {
        let sme = self.implements(Feature::Sme);
        sys.cpacr_el1 & !self.cpacr_mask() == 0
            && sys.smcr_el1 & !smcr_mask(self.imp.features) == 0
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

    /// An EL0 access to a counter or timer register whose `CNTKCTL_EL1`
    /// enable (`enable_bits`, any of them) is clear traps to EL1.
    fn el0_timer_check(&self, el: El, enable_bits: u64) -> Result<(), SysRegError> {
        trap_to_el1(el == El::El0 && self.sys.cntkctl_el1 & enable_bits == 0)
    }

    /// The accessor checks of `CNTPCT_EL0` and `CNTPCTSS_EL0`:
    /// `CNTKCTL_EL1.EL0PCTEN` at EL0, then `CNTHCTL_EL2.EL1PCTEN` at EL0 and
    /// EL1.
    fn physical_count_check(&self, el: El) -> Result<(), SysRegError> {
        self.el0_timer_check(el, EL0PCTEN)?;
        trap_to_el2(el != El::El2 && self.cnthctl_lacks(CNTHCTL_EL1PCTEN))
    }

    /// The accessor checks of `CNTVCT_EL0` and `CNTVCTSS_EL0`:
    /// `CNTKCTL_EL1.EL0VCTEN` at EL0, then `CNTHCTL_EL2.EL1TVCT` (FEAT_ECV) at
    /// EL0 and EL1.
    fn virtual_count_check(&self, el: El) -> Result<(), SysRegError> {
        self.el0_timer_check(el, EL0VCTEN)?;
        trap_to_el2(el != El::El2 && self.cnthctl_any(CNTHCTL_EL1TVCT))
    }

    /// The accessor checks of `CNTP_CTL_EL0`, `CNTP_CVAL_EL0` and
    /// `CNTP_TVAL_EL0`: `CNTKCTL_EL1.EL0PTEN` at EL0, then
    /// `CNTHCTL_EL2.EL1PCEN` at EL0 and EL1.
    fn physical_timer_check(&self, el: El) -> Result<(), SysRegError> {
        self.el0_timer_check(el, EL0PTEN)?;
        trap_to_el2(el != El::El2 && self.cnthctl_lacks(CNTHCTL_EL1PCEN))
    }

    /// The accessor checks of `CNTV_CTL_EL0`, `CNTV_CVAL_EL0` and
    /// `CNTV_TVAL_EL0`: `CNTKCTL_EL1.EL0VTEN` at EL0, then
    /// `CNTHCTL_EL2.EL1TVT` (FEAT_ECV) at EL0 and EL1.
    fn virtual_timer_check(&self, el: El) -> Result<(), SysRegError> {
        self.el0_timer_check(el, EL0VTEN)?;
        trap_to_el2(el != El::El2 && self.cnthctl_any(CNTHCTL_EL1TVT))
    }

    /// The accessor checks of `MDCCSR_EL0`, `DBGDTR_EL0`, `DBGDTRRX_EL0` and
    /// `DBGDTRTX_EL0`, made before the access has any effect (Arm ARM, DDI
    /// 0487, their accessibility pseudocode; QEMU target/arm/debug_helper.c
    /// `access_tdcc`): `MDSCR_EL1.TDCC` traps EL0's to EL1; then
    /// `MDCR_EL2.TDE` or `TDA` traps EL0's and EL1's to EL2, as `HCR_EL2.TGE`
    /// does EL0's.
    fn dcc_check(&self, el: El) -> Result<(), SysRegError> {
        trap_to_el1(el == El::El0 && self.sys.mdscr_el1 & MDSCR_TDCC != 0)?;
        let tge = el == El::El0 && self.tge();
        trap_to_el2(el != El::El2 && (tge || self.mdcr_any(MDCR_TDE | MDCR_TDA)))
    }

    /// The check of an EL1 debug register access: `MDCR_EL2.TDE`, or the
    /// control `bit` (`TDA`, `TDOSA`, `TDRA`) that names the register's
    /// group, traps it to EL2.
    fn debug_check(&self, el: El, bit: u64) -> Result<(), SysRegError> {
        trap_to_el2(el == El::El1 && self.mdcr_any(MDCR_TDE | bit))
    }

    /// The check of an EL1 read of a virtual memory control: `HCR_EL2.TRVM`.
    fn trvm_check(&self, el: El) -> Result<(), SysRegError> {
        trap_to_el2(el == El::El1 && self.hcr_any(HCR_TRVM))
    }

    /// The check of an EL1 write of a virtual memory control: `HCR_EL2.TVM`.
    fn tvm_check(&self, el: El) -> Result<(), SysRegError> {
        trap_to_el2(el == El::El1 && self.hcr_any(HCR_TVM))
    }

    /// Whether `key` names `ERRSELR_EL1` or an `ERX*` register this
    /// processor has: FEAT_RAS's, and with FEAT_RASv1p1 `ERXMISC2_EL1`,
    /// `ERXMISC3_EL1` and the fault injection registers. No error record
    /// exists (`ERRIDR_EL1.NUM` = 0), so no `ERRSELR_EL1.SEL` selects one:
    /// the registers read as zero and ignore writes, as Linux's KVM presents
    /// them to a guest with no error records.
    fn error_record_register(&self, key: SysRegKey) -> bool {
        match key.0 {
            ERRSELR_EL1 | ERXFR_EL1 | ERXCTLR_EL1 | ERXSTATUS_EL1 | ERXADDR_EL1 | ERXMISC0_EL1
            | ERXMISC1_EL1 => self.implements(Feature::Ras),
            ERXMISC2_EL1 | ERXMISC3_EL1 | ERXPFGF_EL1 | ERXPFGCTL_EL1 | ERXPFGCDN_EL1 => {
                self.implements(Feature::RasV1p1)
            }
            _ => false,
        }
    }

    /// The check of an EL1 access to an error record register, as each
    /// register's accessor makes it: `HCR_EL2.FIEN` clear traps the fault
    /// injection registers to EL2, `HCR_EL2.TERR` the others.
    fn error_record_check(&self, el: El, key: SysRegKey) -> Result<(), SysRegError> {
        let trapped = match key.0 {
            ERXPFGF_EL1 | ERXPFGCTL_EL1 | ERXPFGCDN_EL1 => self.hcr_lacks(HCR_FIEN),
            _ => self.hcr_any(HCR_TERR),
        };
        trap_to_el2(el == El::El1 && trapped)
    }

    /// Whether an access at `el` to a pointer authentication key register
    /// traps to EL2: at EL1, with `HCR_EL2.APK` clear.
    fn apk_traps(&self, el: El) -> bool {
        el == El::El1 && self.hcr_lacks(HCR_APK)
    }

    /// The outcome of an access to the IMPLEMENTATION DEFINED space
    /// ([`SysRegKey::in_impdef_space`]), as the `S1_<op1>_<Cn>_<Cm>_<op2>`
    /// and `S3_<op1>_<Cn>_<Cm>_<op2>` accessors of Arm's `Registers.json`
    /// make it with nothing implemented there, for an access at `el`: EL1's
    /// traps to EL2 under `HCR_EL2.TIDCP`, and every other is UNDEFINED.
    pub(crate) fn impdef_space_access(&self, el: El) -> SysRegError {
        if el == El::El1 && self.hcr_any(HCR_TIDCP) {
            SysRegError::Trap(Route::El2)
        } else {
            SysRegError::Undefined
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
                Err(SysRegError::Trap(Route::Class))
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
            Accessor::Host => self.highest_el(),
        }
    }

    /// The value of the register `key` names, as `accessor` reads it. This
    /// read has no side effect; an `MRS`'s side effect is
    /// [`Cpu::sysreg_mrs`]'s.
    fn sysreg_read_as(&self, key: SysRegKey, accessor: Accessor) -> Result<u64, SysRegError> {
        let el = self.access_level(accessor);
        let el0 = el == El::El0;
        let below_el2 = el != El::El2;
        let k = key.0;

        // Registers EL0 may reach, each with its own control.
        match k {
            NZCV => return Ok(self.pstate.nzcv()),
            DAIF => {
                trap_to_el1(el0 && self.sys.sctlr_el1 & sctlr::UMA == 0)?;
                return Ok(self.pstate.daif());
            }
            FPCR | FPSR => {
                self.fp_register_check(accessor)?;
                return Ok(if k == FPCR { self.fpcr } else { self.fpsr });
            }
            CTR_EL0 => {
                trap_to_el1(el0 && self.sys.sctlr_el1 & sctlr::UCT == 0)?;
                trap_to_el2(below_el2 && self.hcr_any(HCR_TID2))?;
                return Ok(id::CTR_EL0);
            }
            // `DZP` reads 1 where `DC ZVA` would trap: at EL0 under
            // `SCTLR_EL1.DZE` clear, at EL0 and EL1 under `HCR_EL2.TDZ`.
            DCZID_EL0 => {
                let prohibited = (el0 && self.sys.sctlr_el1 & sctlr::DZE == 0)
                    || (below_el2 && self.hcr_any(super::el2::HCR_TDZ));
                return Ok(id::DCZID_BLOCK_LOG2_WORDS | if prohibited { DCZID_DZP } else { 0 });
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
                self.sme_register_check(accessor)?;
                return Ok(self.svcr.bits());
            }
            TPIDR2_EL0 if self.implements(Feature::Sme) => {
                trap_to_el1(el0 && self.sys.sctlr_el1 & sctlr::ENTP2 == 0)?;
                return Ok(self.sys.tpidr2_el0);
            }
            CNTFRQ_EL0 => {
                self.el0_timer_check(el, EL0PCTEN | EL0VCTEN)?;
                return Ok(self.sys.cntfrq_el0);
            }
            CNTPCT_EL0 => {
                self.physical_count_check(el)?;
                return Ok(self.physical_count_read(el).get());
            }
            CNTVCT_EL0 => {
                self.virtual_count_check(el)?;
                return Ok(self.virtual_count().get());
            }
            // The self-synchronized views of the counters (FEAT_ECV): every
            // read here is in program order already.
            CNTPCTSS_EL0 if self.implements(Feature::Ecv) => {
                self.physical_count_check(el)?;
                return Ok(self.physical_count_read(el).get());
            }
            CNTVCTSS_EL0 if self.implements(Feature::Ecv) => {
                self.virtual_count_check(el)?;
                return Ok(self.virtual_count().get());
            }
            CNTP_CTL_EL0 | CNTP_CVAL_EL0 | CNTP_TVAL_EL0 => {
                self.physical_timer_check(el)?;
                return Ok(match k {
                    CNTP_CTL_EL0 => self.timer_ctl_read(GenericTimer::Physical),
                    CNTP_CVAL_EL0 => self.sys.cntp_cval_el0,
                    _ => self.timer_tval_read(Tval::Physical(el)),
                });
            }
            CNTV_CTL_EL0 | CNTV_CVAL_EL0 | CNTV_TVAL_EL0 => {
                self.virtual_timer_check(el)?;
                return Ok(match k {
                    CNTV_CTL_EL0 => self.timer_ctl_read(GenericTimer::Virtual),
                    CNTV_CVAL_EL0 => self.sys.cntv_cval_el0,
                    _ => self.timer_tval_read(Tval::Virtual),
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

        if key.in_impdef_space() {
            return Err(self.impdef_space_access(el));
        }
        if key.is_el2() {
            return if el == El::El2 {
                self.el2_register_read(key, accessor)
            } else {
                Err(SysRegError::Undefined)
            };
        }
        if key.in_id_group3() {
            trap_to_el2(el == El::El1 && self.hcr_any(HCR_TID3))?;
        }

        // A key register keeps all 64 bits.
        if let Some(half) = key_half(k).filter(|_| self.implements(Feature::PAuth)) {
            trap_to_el2(self.apk_traps(el))?;
            return Ok(self.pac_keys.half(half));
        }

        let el1 = el == El::El1;
        let value = match k {
            // EL1 reads `VPIDR_EL2` and `VMPIDR_EL2` in their place.
            MIDR_EL1 => {
                if el1 && self.el2_enabled() {
                    self.el2().vpidr
                } else {
                    self.imp.midr
                }
            }
            MPIDR_EL1 => {
                if el1 && self.el2_enabled() {
                    self.el2().vmpidr
                } else {
                    id::MPIDR_EL1
                }
            }
            REVIDR_EL1 => {
                trap_to_el2(el1 && self.hcr_any(HCR_TID1))?;
                id::REVIDR_EL1
            }
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
            CCSIDR_EL1 => {
                trap_to_el2(el1 && self.hcr_any(HCR_TID2))?;
                self.ccsidr()
            }
            CLIDR_EL1 => {
                trap_to_el2(el1 && self.hcr_any(HCR_TID2))?;
                id::CLIDR_EL1
            }
            SMIDR_EL1 if self.implements(Feature::Sme) => {
                trap_to_el2(el1 && self.hcr_any(HCR_TID1))?;
                id::SMIDR_EL1
            }
            AIDR_EL1 => {
                trap_to_el2(el1 && self.hcr_any(HCR_TID1))?;
                0
            }
            CSSELR_EL1 => {
                trap_to_el2(el1 && self.hcr_any(HCR_TID2))?;
                self.sys.csselr_el1
            }
            SCTLR_EL1 => {
                self.trvm_check(el)?;
                self.sys.sctlr_el1
            }
            ACTLR_EL1 => {
                trap_to_el2(el1 && self.hcr_any(HCR_TACR))?;
                self.sys.actlr_el1
            }
            CPACR_EL1 => {
                trap_to_el2(el1 && self.cptr_any(CPTR_TCPAC))?;
                self.sys.cpacr_el1
            }
            SMCR_EL1 if self.implements(Feature::Sme) => {
                self.sme_register_check(accessor)?;
                self.sys.smcr_el1
            }
            // RES0: SMIDR_EL1.SMPS is 0, there are no streaming execution
            // priorities.
            SMPRI_EL1 if self.implements(Feature::Sme) => 0,
            TTBR0_EL1 => {
                self.trvm_check(el)?;
                self.sys.ttbr0_el1
            }
            TTBR1_EL1 => {
                self.trvm_check(el)?;
                self.sys.ttbr1_el1
            }
            TCR_EL1 => {
                self.trvm_check(el)?;
                self.sys.tcr_el1
            }
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
            AFSR0_EL1 => {
                self.trvm_check(el)?;
                self.sys.afsr0_el1
            }
            AFSR1_EL1 => {
                self.trvm_check(el)?;
                self.sys.afsr1_el1
            }
            ESR_EL1 => {
                self.trvm_check(el)?;
                self.sys.esr_el1
            }
            FAR_EL1 => {
                self.trvm_check(el)?;
                self.sys.far_el1
            }
            PAR_EL1 => self.sys.par_el1,
            MAIR_EL1 => {
                self.trvm_check(el)?;
                self.sys.mair_el1
            }
            AMAIR_EL1 => {
                self.trvm_check(el)?;
                self.sys.amair_el1
            }
            // With no LORegions (`LORID_EL1`), the LORegion registers hold
            // nothing: RAZ/WI.
            LORSA_EL1 | LOREA_EL1 | LORN_EL1 | LORC_EL1 if self.implements(Feature::Lor) => {
                trap_to_el2(el1 && self.hcr_any(HCR_TLOR))?;
                0
            }
            LORID_EL1 if self.implements(Feature::Lor) => {
                trap_to_el2(el1 && self.hcr_any(HCR_TLOR))?;
                id::LORID_EL1
            }
            VBAR_EL1 => self.sys.vbar_el1,
            // Only where EL1 is the highest level. The IMPLEMENTATION
            // DEFINED address execution starts from after reset: the one the
            // machine's last reset started it from.
            RVBAR_EL1 if !self.have_el2() => self.sys.rvbar,
            RMR_EL1 if !self.have_el2() => RMR_AA64 | self.sys.rmr,
            ISR_EL1 => self.interrupt_status(el),
            CONTEXTIDR_EL1 => {
                self.trvm_check(el)?;
                self.sys.contextidr_el1
            }
            TPIDR_EL1 => self.sys.tpidr_el1,
            CNTKCTL_EL1 => self.sys.cntkctl_el1,
            MDCCINT_EL1 => {
                self.debug_check(el, MDCR_TDA)?;
                self.sys.mdccint_el1
            }
            MDSCR_EL1 => {
                self.debug_check(el, MDCR_TDA)?;
                self.sys.mdscr_el1
            }
            // The save/restore views of DTRRX and DTRTX: a read returns the
            // data register and leaves RXfull and TXfull as they are, at
            // either state of the OS Lock (Arm ARM, DDI 0487, the
            // `OSDTRRX_EL1` and `OSDTRTX_EL1` descriptions, which deprecate
            // their use while the OS Lock is unlocked). Bits [63:32] are
            // RES0.
            OSDTRRX_EL1 | OSDTRTX_EL1 => {
                self.debug_check(el, MDCR_TDA)?;
                u64::from(if k == OSDTRRX_EL1 {
                    self.sys.dtrrx
                } else {
                    self.sys.dtrtx
                })
            }
            // With the OS lock unlocked the value read is UNKNOWN; this is
            // the one it reads.
            OSECCR_EL1 => {
                self.debug_check(el, MDCR_TDA)?;
                self.sys.oseccr_el1
            }
            OSLSR_EL1 => {
                self.debug_check(el, MDCR_TDOSA)?;
                self.sys.oslsr_el1
            }
            // `MDCR_EL2.TDOSA` traps `OSDLR_EL1` on a processor with
            // FEAT_DoubleLock (`ID_AA64DFR0_EL1.DoubleLock` = 0) and, Arm
            // leaving it IMPLEMENTATION DEFINED ("Trapped by
            // MDCR_EL2.TDOSA"), on one without, where DLK is RES0.
            OSDLR_EL1 => {
                self.debug_check(el, MDCR_TDOSA)?;
                self.sys.osdlr_el1
            }
            DBGPRCR_EL1 => {
                self.debug_check(el, MDCR_TDOSA)?;
                self.sys.dbgprcr_el1
            }
            // No ROM table for an external debugger to find: `Valid` 0b00,
            // `ROMADDR` not valid, as QEMU reads it (`debug_cp_reginfo`).
            MDRAR_EL1 => {
                self.debug_check(el, MDCR_TDRA)?;
                0
            }
            // Every one of the eight claim tags is implemented: their
            // `DBGCLAIMSET_EL1` bits read as one ("Access to this field is
            // RAO/W1S", DDI 0601's `DBGCLAIMSET_EL1`), and
            // `DBGCLAIMCLR_EL1` reads the tags themselves, as QEMU's
            // `dbgclaimset_read` and `DBGCLAIMCLR_EL1` have them.
            DBGCLAIMSET_EL1 => {
                self.debug_check(el, MDCR_TDA)?;
                0xFF
            }
            DBGCLAIMCLR_EL1 => {
                self.debug_check(el, MDCR_TDA)?;
                self.sys.dbgclaim
            }
            DBGAUTHSTATUS_EL1 => {
                self.debug_check(el, MDCR_TDA)?;
                id::dbgauthstatus_el1(self.imp.features)
            }
            ERRIDR_EL1 if self.implements(Feature::Ras) => {
                trap_to_el2(el1 && self.hcr_any(HCR_TERR))?;
                id::ERRIDR_EL1
            }
            _ if self.error_record_register(key) => {
                self.error_record_check(el, key)?;
                0
            }
            // At EL1 under `HCR_EL2.AMO`, `VDISR_EL2` in its place.
            DISR_EL1 if self.implements(Feature::Ras) => {
                if el1 && self.hcr_any(HCR_AMO) {
                    self.el2().vdisr
                } else {
                    self.sys.disr_el1
                }
            }
            _ => {
                if let Some(point) = DebugPoint::of(key) {
                    self.debug_check(el, MDCR_TDA)?;
                    self.debug_point(point)
                } else if key.in_id_group3() {
                    0
                } else {
                    return Err(SysRegError::Undefined);
                }
            }
        };
        Ok(value)
    }

    /// The EL2 register `key` names, as `accessor` reads it at EL2.
    fn el2_register_read(&self, key: SysRegKey, accessor: Accessor) -> Result<u64, SysRegError> {
        if let Some(reg) = self.el2_storage(key) {
            if reg == El2Reg::Smcr {
                self.sme_register_check(accessor)?;
            }
            return Ok(self.el2().get(reg));
        }
        Ok(match key.0 {
            SP_EL1 => self.sp_el1,
            // RES0 with no AArch32 at EL1.
            SPSR_IRQ | SPSR_ABT | SPSR_UND | SPSR_FIQ => 0,
            RVBAR_EL2 => self.sys.rvbar,
            RMR_EL2 => RMR_AA64 | self.sys.rmr,
            CNTHP_CTL_EL2 => self.timer_ctl_read(GenericTimer::Hypervisor),
            CNTHP_TVAL_EL2 => self.timer_tval_read(Tval::Hypervisor),
            // RES0, as `SMPRI_EL1` is: no streaming execution priorities.
            SMPRIMAP_EL2 if self.implements(Feature::Sme) => 0,
            _ => return Err(SysRegError::Undefined),
        })
    }

    /// The EL2 register held as written (through its [`El2Reg::kept`]
    /// mask) that `key` names, if this processor implements it
    /// ([`el2_encoding`]).
    fn el2_storage(&self, key: SysRegKey) -> Option<El2Reg> {
        let reg = EL2_STORAGE[key.el2_slot()?]?;
        (el2_encoding(reg, self.imp.features) == Some(key.0)).then_some(reg)
    }

    /// The value of breakpoint or watchpoint register `point`.
    fn debug_point(&self, point: DebugPoint) -> u64 {
        let regs = match point.reg {
            DebugPointReg::Bvr => &self.sys.dbgbvr,
            DebugPointReg::Bcr => &self.sys.dbgbcr,
            DebugPointReg::Wvr => &self.sys.dbgwvr,
            DebugPointReg::Wcr => &self.sys.dbgwcr,
        };
        regs[point.n]
    }

    /// The storage of breakpoint or watchpoint register `point`.
    fn debug_point_mut(&mut self, point: DebugPoint) -> &mut u64 {
        let regs = match point.reg {
            DebugPointReg::Bvr => &mut self.sys.dbgbvr,
            DebugPointReg::Bcr => &mut self.sys.dbgbcr,
            DebugPointReg::Wvr => &mut self.sys.dbgwvr,
            DebugPointReg::Wcr => &mut self.sys.dbgwcr,
        };
        &mut regs[point.n]
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
                trap_to_el1(el0 && self.sys.sctlr_el1 & sctlr::UMA == 0)?;
                self.pstate.set_daif(value);
                return Ok(WriteEffect::None);
            }
            FPCR | FPSR => {
                self.fp_register_check(accessor)?;
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
                self.sme_register_check(accessor)?;
                self.set_pstate_sm(value & 1 != 0);
                self.set_pstate_za(value & 2 != 0);
                return Ok(WriteEffect::None);
            }
            TPIDR2_EL0 if self.implements(Feature::Sme) => {
                trap_to_el1(el0 && self.sys.sctlr_el1 & sctlr::ENTP2 == 0)?;
                self.sys.tpidr2_el0 = value;
                return Ok(WriteEffect::None);
            }
            CNTP_CTL_EL0 | CNTP_CVAL_EL0 | CNTP_TVAL_EL0 => {
                self.physical_timer_check(el)?;
                match k {
                    CNTP_CTL_EL0 => self.sys.cntp_ctl_el0 = value & 3,
                    CNTP_CVAL_EL0 => self.sys.cntp_cval_el0 = value,
                    _ => {
                        self.sys.cntp_cval_el0 =
                            self.timer_cval_from_tval(Tval::Physical(el), value)
                    }
                }
                return Ok(WriteEffect::Timer);
            }
            CNTV_CTL_EL0 | CNTV_CVAL_EL0 | CNTV_TVAL_EL0 => {
                self.virtual_timer_check(el)?;
                match k {
                    CNTV_CTL_EL0 => self.sys.cntv_ctl_el0 = value & 3,
                    CNTV_CVAL_EL0 => self.sys.cntv_cval_el0 = value,
                    _ => self.sys.cntv_cval_el0 = self.timer_cval_from_tval(Tval::Virtual, value),
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

        if key.in_impdef_space() {
            return Err(self.impdef_space_access(el));
        }
        if key.is_el2() {
            return if el == El::El2 {
                self.el2_register_write(key, value, accessor)
            } else {
                Err(SysRegError::Undefined)
            };
        }

        // A key register keeps all 64 bits, with nothing to invalidate.
        if let Some(half) = key_half(k).filter(|_| self.implements(Feature::PAuth)) {
            trap_to_el2(self.apk_traps(el))?;
            *self.pac_keys.half_mut(half) = value;
            return Ok(WriteEffect::None);
        }

        let el1 = el == El::El1;
        let before =
            TranslationRegister::of_el1(k).map(|register| self.translation_context(register));
        let effect = match k {
            CSSELR_EL1 => {
                trap_to_el2(el1 && self.hcr_any(HCR_TID2))?;
                self.sys.csselr_el1 = value & 0xF;
                WriteEffect::None
            }
            // The memory-tagging controls read as zero on a core without
            // FEAT_MTE.
            SCTLR_EL1 => {
                self.tvm_check(el)?;
                self.sys.sctlr_el1 = value & !SCTLR_MTE;
                WriteEffect::None
            }
            ACTLR_EL1 => {
                trap_to_el2(el1 && self.hcr_any(HCR_TACR))?;
                self.sys.actlr_el1 = value;
                WriteEffect::None
            }
            CPACR_EL1 => {
                trap_to_el2(el1 && self.cptr_any(CPTR_TCPAC))?;
                self.sys.cpacr_el1 = value & self.cpacr_mask();
                WriteEffect::None
            }
            SMCR_EL1 if self.implements(Feature::Sme) => {
                self.sme_register_check(accessor)?;
                self.sys.smcr_el1 = value & smcr_mask(self.imp.features);
                WriteEffect::None
            }
            SMPRI_EL1 if self.implements(Feature::Sme) => WriteEffect::None,
            TTBR0_EL1 => {
                self.tvm_check(el)?;
                self.sys.ttbr0_el1 = value;
                WriteEffect::None
            }
            TTBR1_EL1 => {
                self.tvm_check(el)?;
                self.sys.ttbr1_el1 = value;
                WriteEffect::None
            }
            TCR_EL1 => {
                self.tvm_check(el)?;
                self.sys.tcr_el1 = value;
                WriteEffect::None
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
                self.pstate.sp_sel = value & 1 != 0 && self.pstate.el != El::El0;
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
                self.tvm_check(el)?;
                self.sys.afsr0_el1 = value;
                WriteEffect::None
            }
            AFSR1_EL1 => {
                self.tvm_check(el)?;
                self.sys.afsr1_el1 = value;
                WriteEffect::None
            }
            ESR_EL1 => {
                self.tvm_check(el)?;
                self.sys.esr_el1 = value & 0xFFFF_FFFF;
                WriteEffect::None
            }
            FAR_EL1 => {
                self.tvm_check(el)?;
                self.sys.far_el1 = value;
                WriteEffect::None
            }
            PAR_EL1 => {
                self.sys.par_el1 = value;
                WriteEffect::None
            }
            MAIR_EL1 => {
                self.tvm_check(el)?;
                self.sys.mair_el1 = value;
                WriteEffect::None
            }
            AMAIR_EL1 => {
                self.tvm_check(el)?;
                self.sys.amair_el1 = value;
                WriteEffect::None
            }
            LORSA_EL1 | LOREA_EL1 | LORN_EL1 | LORC_EL1 if self.implements(Feature::Lor) => {
                trap_to_el2(el1 && self.hcr_any(HCR_TLOR))?;
                WriteEffect::None
            }
            // `ERXFR_EL1` and `ERXPFGF_EL1` are read-only.
            _ if self.error_record_register(key) && !matches!(k, ERXFR_EL1 | ERXPFGF_EL1) => {
                self.error_record_check(el, key)?;
                WriteEffect::None
            }
            DISR_EL1 if self.implements(Feature::Ras) => {
                if el1 && self.hcr_any(HCR_AMO) {
                    self.el2_write(El2Reg::Vdisr, value);
                } else {
                    self.sys.disr_el1 = value & self.disr_mask();
                }
                WriteEffect::None
            }
            VBAR_EL1 => {
                self.sys.vbar_el1 = value & !0x7FF;
                WriteEffect::None
            }
            // Only where EL1 is the highest level. RR asks the system for a
            // Warm reset of the processor. The board has no reset controller
            // that acts on the request (as QEMU's `virt` has none), so it is
            // held and nothing resets; AA64 is RAO/WI.
            RMR_EL1 if !self.have_el2() => {
                self.sys.rmr = value & RMR_RR;
                WriteEffect::None
            }
            CONTEXTIDR_EL1 => {
                self.tvm_check(el)?;
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
            // Writable only at the highest implemented EL. It reports a
            // frequency; it does not change the counter's rate.
            CNTFRQ_EL0 => {
                if el != self.highest_el() {
                    return Err(SysRegError::Undefined);
                }
                self.sys.cntfrq_el0 = value & 0xFFFF_FFFF;
                WriteEffect::None
            }
            MDCCINT_EL1 => {
                self.debug_check(el, MDCR_TDA)?;
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
                self.debug_check(el, MDCR_TDA)?;
                let kept = if self.sys.oslsr_el1 & OSLSR_OSLK != 0 {
                    0
                } else {
                    MDSCR_SAVE_RESTORE
                };
                self.sys.mdscr_el1 = (value & MDSCR_FIELDS & !kept) | (self.sys.mdscr_el1 & kept);
                WriteEffect::None
            }
            // A write to a save/restore view sets the data register from bits
            // [31:0] and leaves its flag as it is, at either state of the OS
            // Lock (the `OSDTRRX_EL1` and `OSDTRTX_EL1` descriptions).
            OSDTRRX_EL1 | OSDTRTX_EL1 => {
                self.debug_check(el, MDCR_TDA)?;
                if k == OSDTRRX_EL1 {
                    self.sys.dtrrx = value as u32;
                } else {
                    self.sys.dtrtx = value as u32;
                }
                WriteEffect::None
            }
            // Ignored while the OS lock is unlocked (Arm `OSECCR_EL1`'s
            // accessor returns before the write).
            OSECCR_EL1 => {
                self.debug_check(el, MDCR_TDA)?;
                if self.sys.oslsr_el1 & OSLSR_OSLK != 0 {
                    self.sys.oseccr_el1 = value & 0xFFFF_FFFF;
                }
                WriteEffect::None
            }
            OSLAR_EL1 => {
                self.debug_check(el, MDCR_TDOSA)?;
                // OSLAR_EL1.OSLK sets OSLSR_EL1.OSLK (bit 1).
                self.sys.oslsr_el1 = (self.sys.oslsr_el1 & !OSLSR_OSLK) | ((value & 1) << 1);
                WriteEffect::None
            }
            // DLK is RES0 without FEAT_DoubleLock.
            OSDLR_EL1 => {
                self.debug_check(el, MDCR_TDOSA)?;
                self.sys.osdlr_el1 = if self.implements(Feature::DoubleLock) {
                    value & OSDLR_DLK
                } else {
                    0
                };
                WriteEffect::None
            }
            DBGPRCR_EL1 => {
                self.debug_check(el, MDCR_TDOSA)?;
                self.sys.dbgprcr_el1 = value & DBGPRCR_CORENPDRQ;
                WriteEffect::None
            }
            DBGCLAIMSET_EL1 => {
                self.debug_check(el, MDCR_TDA)?;
                self.sys.dbgclaim |= value & 0xFF;
                WriteEffect::None
            }
            DBGCLAIMCLR_EL1 => {
                self.debug_check(el, MDCR_TDA)?;
                self.sys.dbgclaim &= !(value & 0xFF);
                WriteEffect::None
            }
            _ => {
                let Some(point) = DebugPoint::of(key) else {
                    return Err(SysRegError::Undefined);
                };
                self.debug_check(el, MDCR_TDA)?;
                *self.debug_point_mut(point) = value;
                WriteEffect::None
            }
        };
        Ok(self.translation_effect(before, effect))
    }

    /// What a translation reads now of the registers of `register`'s
    /// regime.
    fn translation_context(&self, register: TranslationRegister) -> TranslationContext {
        match register {
            TranslationRegister::El10 => TranslationContext::El10(self.el10_context()),
            TranslationRegister::El2 => TranslationContext::El2(El2RegimeContext::of(self.el2())),
        }
    }

    /// The effect of a write whose own is `effect`, of a translation
    /// register whose regime's context was `before`, if it is one: a write
    /// that changes the EL1&0 regime's context changes the context
    /// ([`WriteEffect::El10Context`]), one that changes what the EL2
    /// regime's walk reads empties the TLB ([`WriteEffect::FlushTlb`]), and
    /// one that leaves the context as it was changes nothing of the TLB,
    /// whose entries depend on the context alone.
    fn translation_effect(
        &self,
        before: Option<TranslationContext>,
        effect: WriteEffect,
    ) -> WriteEffect {
        match before {
            Some(TranslationContext::El10(context)) if context != self.el10_context() => {
                WriteEffect::El10Context
            }
            Some(TranslationContext::El2(context))
                if context != El2RegimeContext::of(self.el2()) =>
            {
                WriteEffect::FlushTlb
            }
            Some(TranslationContext::El10(_) | TranslationContext::El2(_)) | None => effect,
        }
    }

    /// Write `value` to the EL2 register `key` names, as `accessor` writes
    /// it at EL2.
    fn el2_register_write(
        &mut self,
        key: SysRegKey,
        value: u64,
        accessor: Accessor,
    ) -> Result<WriteEffect, SysRegError> {
        if let Some(reg) = self.el2_storage(key) {
            match reg {
                El2Reg::Smcr => self.sme_register_check(accessor)?,
                // The event streams' events so far are judged by the
                // controls and the offset that generated them.
                El2Reg::Cnthctl | El2Reg::Cntvoff => self.fold_event_stream(),
                _ => {}
            }
            // A translation register's write is judged by the context of
            // its regime ([`Cpu::translation_effect`]); `HCR_EL2`'s routing
            // and traps are read afresh before every instruction.
            let before =
                TranslationRegister::of_el2(reg).map(|register| self.translation_context(register));
            // What the timers read of the EL2 registers
            // ([`Cpu::el2_timer_inputs`]) is compared across a write of a
            // register they read ([`El2Reg::feeds_timers`]): one that
            // changes it may move a timer, and has the timer lines sampled
            // again.
            let inputs = reg.feeds_timers().then(|| self.el2_timer_inputs());
            self.el2_write(reg, value);
            let timers = inputs.is_some_and(|inputs| inputs != self.el2_timer_inputs());
            let effect = self.translation_effect(before, WriteEffect::None);
            return Ok(match (timers, effect) {
                (true, WriteEffect::None) => WriteEffect::Timer,
                // A write has one effect to return: the timers' sampling is
                // asked for here, the translation's returned.
                (true, WriteEffect::FlushTlb | WriteEffect::El10Context | WriteEffect::Timer) => {
                    self.apply_write_effect(WriteEffect::Timer);
                    effect
                }
                (false, _) => effect,
            });
        }
        Ok(match key.0 {
            SP_EL1 => {
                self.sp_el1 = value;
                WriteEffect::None
            }
            SPSR_IRQ | SPSR_ABT | SPSR_UND | SPSR_FIQ => WriteEffect::None,
            // As `RMR_EL1`'s where EL1 is the highest level.
            RMR_EL2 => {
                self.sys.rmr = value & RMR_RR;
                WriteEffect::None
            }
            CNTHP_CTL_EL2 => {
                self.el2_write(El2Reg::CnthpCtl, value);
                WriteEffect::Timer
            }
            CNTHP_TVAL_EL2 => {
                let cval = self.timer_cval_from_tval(Tval::Hypervisor, value);
                self.el2_write(El2Reg::CnthpCval, cval);
                WriteEffect::Timer
            }
            SMPRIMAP_EL2 if self.implements(Feature::Sme) => WriteEffect::None,
            _ => return Err(SysRegError::Undefined),
        })
    }

    /// An `MSR` of `value` to the EL2 register `reg`, keeping what
    /// [`El2Reg::kept`] keeps. Only a processor with EL2 reaches one: an
    /// access below EL2 to an EL2 register is UNDEFINED or trapped first,
    /// and a processor without EL2, which has no EL2 registers, never runs
    /// at EL2, so there the write changes nothing.
    fn el2_write(&mut self, reg: El2Reg, value: u64) {
        let (features, spsr_mask) = (self.imp.features, self.spsr_mask());
        if let Some(el2) = self.el2_mut() {
            el2.write(reg, features, spsr_mask, value);
        }
    }
}

impl<L: Levels> Cpu<L> {
    /// A host's read of `reg`: what an `MRS` at the highest implemented
    /// level returns, with no control that traps a guest in the way. Reading
    /// has no side effects.
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
            // A host reads at the highest level past the FP and SME enables
            // and the EL0 and EL2 controls, so only UNDEFINED is left to
            // come back.
            Err(
                SysRegError::Undefined
                | SysRegError::Trap(_)
                | SysRegError::FpTrap(_)
                | SysRegError::SmeTrap(_),
            ) => Err(SysRegRefusal::Absent),
        }
    }

    /// A host's write of `reg`, with the effect an `MSR` at the highest
    /// implemented level has, applied and returned: a timer write's has the
    /// machine re-evaluate its lines. A write of a translation register
    /// ([`Cpu::translation_register`]) empties the TLB whatever the value
    /// ([`WriteEffect::FlushTlb`]), so that a host that has changed a
    /// translation table makes the change seen by writing one.
    pub(crate) fn host_sysreg_write(
        &mut self,
        reg: SysReg,
        value: u64,
    ) -> Result<WriteEffect, SysRegRefusal> {
        let key = SysRegKey::from_sysreg(reg);
        match self.sysreg_write_as(key, value, Accessor::Host) {
            Ok(effect) => {
                let applied = match (effect, self.translation_register(key)) {
                    // The flush is all an `MSR`'s effect on the TLB would
                    // be, and more.
                    (
                        WriteEffect::None | WriteEffect::FlushTlb | WriteEffect::El10Context,
                        Some(TranslationRegister::El10 | TranslationRegister::El2),
                    ) => WriteEffect::FlushTlb,
                    (WriteEffect::None, None) => WriteEffect::None,
                    (WriteEffect::FlushTlb, None) => WriteEffect::FlushTlb,
                    (WriteEffect::El10Context, None) => WriteEffect::El10Context,
                    (WriteEffect::Timer, None | Some(_)) => WriteEffect::Timer,
                };
                self.apply_write_effect(applied);
                Ok(applied)
            }
            // As for reads, a host write can only be UNDEFINED: either the
            // register is read-only, or there is no such register.
            Err(
                SysRegError::Undefined
                | SysRegError::Trap(_)
                | SysRegError::FpTrap(_)
                | SysRegError::SmeTrap(_),
            ) => {
                if self.sysreg_read_as(key, Accessor::Host).is_ok() {
                    Err(SysRegRefusal::ReadOnly)
                } else {
                    Err(SysRegRefusal::Absent)
                }
            }
        }
    }

    /// The regime `key`'s register is a translation register of, if it is
    /// one ([`TranslationRegister`]); the EL2 registers only with EL2.
    fn translation_register(&self, key: SysRegKey) -> Option<TranslationRegister> {
        TranslationRegister::of_el1(key.0)
            .or_else(|| self.el2_storage(key).and_then(TranslationRegister::of_el2))
    }

    /// What the processor itself does after a write with `effect`: a TLB
    /// flush for an EL2 regime translation control, a change of context for
    /// an EL1&0 one, and for a timer register the flag that has the machine
    /// re-evaluate the timer lines.
    pub(crate) fn apply_write_effect(&mut self, effect: WriteEffect) {
        match effect {
            WriteEffect::None => {}
            WriteEffect::FlushTlb => self.flush_tlb(),
            WriteEffect::El10Context => self.el10_context_changed(),
            WriteEffect::Timer => self.timer_dirty = true,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/sysreg.rs"]
mod tests;
