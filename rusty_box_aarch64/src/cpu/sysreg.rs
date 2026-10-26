//! `MRS`/`MSR` (register): which system registers exist, who may access
//! them, and what an access does.
//!
//! A register is named by its encoding `(op0, op1, CRn, CRm, op2)`, packed as
//! `MRS` packs it in bits 20:5. An encoding not listed is UNDEFINED, except the
//! unallocated part of the ID register space (`op0=3, op1=0, CRn=0,
//! CRm=1..7`), which reads as zero (Arm ARM D18.2, "reserved ID registers are
//! RAZ").

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
        Self(((op0 & 3) << 14) | ((op1 & 7) << 11) | ((crn & 15) << 7) | ((crm & 15) << 3) | (op2 & 7))
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
const FAR_EL1: u32 = key!(3, 0, 6, 0, 0);
const PAR_EL1: u32 = key!(3, 0, 7, 4, 0);
const MAIR_EL1: u32 = key!(3, 0, 10, 2, 0);
const AMAIR_EL1: u32 = key!(3, 0, 10, 3, 0);
const VBAR_EL1: u32 = key!(3, 0, 12, 0, 0);
const ISR_EL1: u32 = key!(3, 0, 12, 1, 0);
const CONTEXTIDR_EL1: u32 = key!(3, 0, 13, 0, 1);
const TPIDR_EL1: u32 = key!(3, 0, 13, 0, 4);
const TPIDR_EL0: u32 = key!(3, 3, 13, 0, 2);
const TPIDRRO_EL0: u32 = key!(3, 3, 13, 0, 3);
const TPIDR2_EL0: u32 = key!(3, 3, 13, 0, 5);
const CNTKCTL_EL1: u32 = key!(3, 0, 14, 1, 0);
const CNTFRQ_EL0: u32 = key!(3, 3, 14, 0, 0);
const CNTPCT_EL0: u32 = key!(3, 3, 14, 0, 1);
const CNTVCT_EL0: u32 = key!(3, 3, 14, 0, 2);
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
    /// EL0 access disabled by a control bit: trapped to EL1 with EC 0x18.
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

/// `CPACR_EL1.FPEN`.
const CPACR_FPEN: u64 = 3 << 20;
/// `CPACR_EL1.SMEN` (FEAT_SME).
const CPACR_SMEN: u64 = 3 << 24;

/// `SMCR_EL1.LEN`: the only field this core keeps. Bits 8:4 are RAZ/WI,
/// and `EZT0` and `FA64` RES0 without FEAT_SME2 and FEAT_SME_FA64.
const SMCR_LEN: u64 = 0xF;

impl Cpu {
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
    /// `LEN`.
    pub(crate) fn sme_registers_restorable(&self, sys: &SysRegs) -> bool {
        let sme = self.implements(Feature::Sme);
        sys.cpacr_el1 & !self.cpacr_mask() == 0
            && sys.smcr_el1 & !SMCR_LEN == 0
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

    fn el0_timer_check(&self, el: El, enable_bits: u64) -> Result<(), SysRegError> {
        if el == El::El0 && self.sys.cntkctl_el1 & enable_bits == 0 {
            Err(SysRegError::TrapToEl1)
        } else {
            Ok(())
        }
    }

    /// `MRS`: the value of the register `key` names, read at the current EL.
    #[inline]
    pub(crate) fn sysreg_read(&self, key: SysRegKey) -> Result<u64, SysRegError> {
        self.sysreg_read_as(key, Accessor::Guest)
    }

    /// The exception level an accessor reads and writes at.
    fn access_level(&self, accessor: Accessor) -> El {
        match accessor {
            Accessor::Guest => self.pstate.el,
            Accessor::Host => El::El1,
        }
    }

    /// The value of the register `key` names, as `accessor` reads it.
    /// Reading has no side effects on this core.
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
            DIT if self.implements(Feature::Dit) => return Ok(if self.pstate.dit { Pstate::DIT } else { 0 }),
            SSBS if self.implements(Feature::Ssbs2) => return Ok(if self.pstate.ssbs { Pstate::SSBS } else { 0 }),
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
                return Ok(self.counter());
            }
            CNTVCT_EL0 => {
                self.el0_timer_check(el, EL0VCTEN)?;
                return Ok(self.counter());
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
            MDCCSR_EL0 => return Ok(0),
            _ => {}
        }

        if el0 {
            return Err(SysRegError::Undefined);
        }

        let pauth = self.implements(Feature::PAuth);
        let keys = &self.pac_keys;
        let value = match k {
            MIDR_EL1 => id::MIDR_EL1,
            MPIDR_EL1 => id::MPIDR_EL1,
            REVIDR_EL1 => id::REVIDR_EL1,
            ID_AA64PFR0_EL1 => id::id_aa64pfr0(self.imp.features),
            ID_AA64PFR1_EL1 => id::id_aa64pfr1(self.imp.features),
            ID_AA64ZFR0_EL1 => id::id_aa64zfr0(self.imp.features),
            ID_AA64SMFR0_EL1 => id::id_aa64smfr0(self.imp.features),
            ID_AA64DFR0_EL1 => id::ID_AA64DFR0_EL1,
            ID_AA64ISAR0_EL1 => id::id_aa64isar0(self.imp.features),
            ID_AA64ISAR1_EL1 => id::id_aa64isar1(self.imp.features),
            ID_AA64MMFR0_EL1 => id::ID_AA64MMFR0_EL1,
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
            VBAR_EL1 => self.sys.vbar_el1,
            ISR_EL1 => (u64::from(self.irq_line) << 7) | (u64::from(self.fiq_line) << 6),
            CONTEXTIDR_EL1 => self.sys.contextidr_el1,
            TPIDR_EL1 => self.sys.tpidr_el1,
            CNTKCTL_EL1 => self.sys.cntkctl_el1,
            MDCCINT_EL1 => self.sys.mdccint_el1,
            MDSCR_EL1 => self.sys.mdscr_el1,
            OSECCR_EL1 => self.sys.oseccr_el1,
            OSLSR_EL1 => self.sys.oslsr_el1,
            OSDLR_EL1 => self.sys.osdlr_el1,
            DBGPRCR_EL1 => self.sys.dbgprcr_el1,
            DBGCLAIMSET_EL1 | DBGCLAIMCLR_EL1 => self.sys.dbgclaim,
            DBGAUTHSTATUS_EL1 => 0,
            _ => {
                if let Some(v) = self.debug_breakpoint_read(key) {
                    v
                } else if key.op0() == 3 && key.op1() == 0 && key.crn() == 0 && (1..=7).contains(&key.crm()) {
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
    pub(crate) fn sysreg_write(&mut self, key: SysRegKey, value: u64) -> Result<WriteEffect, SysRegError> {
        self.sysreg_write_as(key, value, Accessor::Guest)
    }

    /// Write `value` to the register `key` names, as `accessor` writes it.
    fn sysreg_write_as(&mut self, key: SysRegKey, value: u64, accessor: Accessor) -> Result<WriteEffect, SysRegError> {
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
            _ => {}
        }

        if el0 {
            return Err(SysRegError::Undefined);
        }

        let pauth = self.implements(Feature::PAuth);
        let keys = &mut self.pac_keys;
        let effect = match k {
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
            SCTLR_EL1 => {
                self.sys.sctlr_el1 = value;
                WriteEffect::FlushTlb
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
                self.sys.smcr_el1 = value & SMCR_LEN;
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
                self.sys.cntkctl_el1 = value & 0x3FF;
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
            MDSCR_EL1 => {
                self.sys.mdscr_el1 = value;
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
            OSDLR_EL1 => {
                self.sys.osdlr_el1 = value & 1;
                WriteEffect::None
            }
            DBGPRCR_EL1 => {
                self.sys.dbgprcr_el1 = value & 1;
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
            Err(SysRegError::Undefined | SysRegError::TrapToEl1 | SysRegError::FpTrap | SysRegError::SmeTrap) => {
                Err(SysRegRefusal::Absent)
            }
        }
    }

    /// A host's write of `reg`, with the effect an EL1 `MSR` has: a write to
    /// a register the table walk reads flushes the TLB, and a timer write is
    /// reported for the machine to re-evaluate its lines.
    pub(crate) fn host_sysreg_write(&mut self, reg: SysReg, value: u64) -> Result<WriteEffect, SysRegRefusal> {
        let key = SysRegKey::from_sysreg(reg);
        match self.sysreg_write_as(key, value, Accessor::Host) {
            Ok(WriteEffect::FlushTlb) => {
                self.tlb.flush();
                Ok(WriteEffect::FlushTlb)
            }
            Ok(WriteEffect::Timer) => {
                self.timer_dirty = true;
                Ok(WriteEffect::Timer)
            }
            Ok(WriteEffect::None) => Ok(WriteEffect::None),
            // As for reads, a host write can only be UNDEFINED: either the
            // register is read-only, or there is no such register.
            Err(SysRegError::Undefined | SysRegError::TrapToEl1 | SysRegError::FpTrap | SysRegError::SmeTrap) => {
                if self.sysreg_read_as(key, Accessor::Host).is_ok() {
                    Err(SysRegRefusal::ReadOnly)
                } else {
                    Err(SysRegRefusal::Absent)
                }
            }
        }
    }
}
