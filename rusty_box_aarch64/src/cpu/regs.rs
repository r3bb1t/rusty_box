//! Naming the processor's registers from the host: the general-purpose and
//! special registers ([`Reg`]), the SIMD&FP registers ([`VReg`]), system
//! registers by encoding ([`SysReg`]), and the whole architectural state at
//! one moment ([`CpuState`]).

use super::{Activity, Pstate};

/// A register a host reads or writes by name.
///
/// Closed (R5). `Sp` is the stack pointer `PSTATE.SP` and the exception
/// level currently select; `SpEl0` and `SpEl1` name the banked registers
/// whichever is selected (`SP_EL2`, which only a machine with EL2 has, is
/// its own: `VirtMachine::sp_el2`). `Nzcv` and `Daif` are the `PSTATE`
/// fields in their `MRS` layout, and a write keeps only those bits, as
/// `MSR` would.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reg {
    X0,
    X1,
    X2,
    X3,
    X4,
    X5,
    X6,
    X7,
    X8,
    X9,
    X10,
    X11,
    X12,
    X13,
    X14,
    X15,
    X16,
    X17,
    X18,
    X19,
    X20,
    X21,
    X22,
    X23,
    X24,
    X25,
    X26,
    X27,
    X28,
    /// The frame pointer by convention.
    X29,
    /// The link register.
    X30,
    Sp,
    SpEl0,
    SpEl1,
    Pc,
    Nzcv,
    Daif,
    Fpcr,
    Fpsr,
}

impl Reg {
    /// `Xn` for `n` in `0..=30`. 31 is not a register: it encodes `XZR` or
    /// `SP` depending on the instruction, and each has its own name here.
    pub const fn x(n: u8) -> Option<Self> {
        Some(match n {
            0 => Self::X0,
            1 => Self::X1,
            2 => Self::X2,
            3 => Self::X3,
            4 => Self::X4,
            5 => Self::X5,
            6 => Self::X6,
            7 => Self::X7,
            8 => Self::X8,
            9 => Self::X9,
            10 => Self::X10,
            11 => Self::X11,
            12 => Self::X12,
            13 => Self::X13,
            14 => Self::X14,
            15 => Self::X15,
            16 => Self::X16,
            17 => Self::X17,
            18 => Self::X18,
            19 => Self::X19,
            20 => Self::X20,
            21 => Self::X21,
            22 => Self::X22,
            23 => Self::X23,
            24 => Self::X24,
            25 => Self::X25,
            26 => Self::X26,
            27 => Self::X27,
            28 => Self::X28,
            29 => Self::X29,
            30 => Self::X30,
            _ => return None,
        })
    }

    /// Which storage the register names: the one choke point (R5) every
    /// access goes through, so a new register is decided here once.
    pub(crate) const fn class(self) -> RegClass {
        match self {
            Self::X0 => RegClass::General(0),
            Self::X1 => RegClass::General(1),
            Self::X2 => RegClass::General(2),
            Self::X3 => RegClass::General(3),
            Self::X4 => RegClass::General(4),
            Self::X5 => RegClass::General(5),
            Self::X6 => RegClass::General(6),
            Self::X7 => RegClass::General(7),
            Self::X8 => RegClass::General(8),
            Self::X9 => RegClass::General(9),
            Self::X10 => RegClass::General(10),
            Self::X11 => RegClass::General(11),
            Self::X12 => RegClass::General(12),
            Self::X13 => RegClass::General(13),
            Self::X14 => RegClass::General(14),
            Self::X15 => RegClass::General(15),
            Self::X16 => RegClass::General(16),
            Self::X17 => RegClass::General(17),
            Self::X18 => RegClass::General(18),
            Self::X19 => RegClass::General(19),
            Self::X20 => RegClass::General(20),
            Self::X21 => RegClass::General(21),
            Self::X22 => RegClass::General(22),
            Self::X23 => RegClass::General(23),
            Self::X24 => RegClass::General(24),
            Self::X25 => RegClass::General(25),
            Self::X26 => RegClass::General(26),
            Self::X27 => RegClass::General(27),
            Self::X28 => RegClass::General(28),
            Self::X29 => RegClass::General(29),
            Self::X30 => RegClass::General(30),
            Self::Sp => RegClass::Sp,
            Self::SpEl0 => RegClass::SpEl0,
            Self::SpEl1 => RegClass::SpEl1,
            Self::Pc => RegClass::Pc,
            Self::Nzcv => RegClass::Nzcv,
            Self::Daif => RegClass::Daif,
            Self::Fpcr => RegClass::Fpcr,
            Self::Fpsr => RegClass::Fpsr,
        }
    }
}

/// What a [`Reg`] reaches in the processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegClass {
    /// `X<n>`, `n` in `0..=30`.
    General(usize),
    Sp,
    SpEl0,
    SpEl1,
    Pc,
    Nzcv,
    Daif,
    Fpcr,
    Fpsr,
}

/// A SIMD&FP register, `V0..=V31`, as its full 128 bits.
///
/// A witness (D-style): holding one proves the number is in range, so no
/// access through it can name a register that does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VReg(u8);

impl VReg {
    /// `Vn`, for `n` in `0..=31`.
    pub const fn new(n: u8) -> Option<Self> {
        if n < 32 {
            Some(Self(n))
        } else {
            None
        }
    }

    pub const fn index(self) -> u8 {
        self.0
    }
}

/// A system register, named by its `MRS`/`MSR` encoding.
///
/// Any encoding in the system register space (`op0` 2 or 3) can be named;
/// whether this processor implements it is the access's answer, not the
/// name's. The constants are the registers a host most often wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SysReg(u16);

macro_rules! sysregs {
    ($($(#[$doc:meta])* $name:ident = ($op0:expr, $op1:expr, $crn:expr, $crm:expr, $op2:expr);)*) => {
        impl SysReg {
            $($(#[$doc])* pub const $name: Self = Self::encode($op0, $op1, $crn, $crm, $op2);)*
        }
    };
}

sysregs! {
    MIDR_EL1 = (3, 0, 0, 0, 0);
    MPIDR_EL1 = (3, 0, 0, 0, 5);
    ID_AA64PFR0_EL1 = (3, 0, 0, 4, 0);
    ID_AA64ISAR0_EL1 = (3, 0, 0, 6, 0);
    ID_AA64MMFR0_EL1 = (3, 0, 0, 7, 0);
    SCTLR_EL1 = (3, 0, 1, 0, 0);
    CPACR_EL1 = (3, 0, 1, 0, 2);
    TTBR0_EL1 = (3, 0, 2, 0, 0);
    TTBR1_EL1 = (3, 0, 2, 0, 1);
    TCR_EL1 = (3, 0, 2, 0, 2);
    SPSR_EL1 = (3, 0, 4, 0, 0);
    ELR_EL1 = (3, 0, 4, 0, 1);
    SP_EL0 = (3, 0, 4, 1, 0);
    SPSEL = (3, 0, 4, 2, 0);
    CURRENT_EL = (3, 0, 4, 2, 2);
    NZCV = (3, 3, 4, 2, 0);
    DAIF = (3, 3, 4, 2, 1);
    FPCR = (3, 3, 4, 4, 0);
    FPSR = (3, 3, 4, 4, 1);
    ESR_EL1 = (3, 0, 5, 2, 0);
    FAR_EL1 = (3, 0, 6, 0, 0);
    PAR_EL1 = (3, 0, 7, 4, 0);
    MAIR_EL1 = (3, 0, 10, 2, 0);
    VBAR_EL1 = (3, 0, 12, 0, 0);
    ISR_EL1 = (3, 0, 12, 1, 0);
    CONTEXTIDR_EL1 = (3, 0, 13, 0, 1);
    TPIDR_EL1 = (3, 0, 13, 0, 4);
    TPIDR_EL0 = (3, 3, 13, 0, 2);
    TPIDRRO_EL0 = (3, 3, 13, 0, 3);
    CNTKCTL_EL1 = (3, 0, 14, 1, 0);
    CNTFRQ_EL0 = (3, 3, 14, 0, 0);
    CNTPCT_EL0 = (3, 3, 14, 0, 1);
    CNTVCT_EL0 = (3, 3, 14, 0, 2);
    CNTP_CTL_EL0 = (3, 3, 14, 2, 1);
    CNTP_CVAL_EL0 = (3, 3, 14, 2, 2);
    CNTV_CTL_EL0 = (3, 3, 14, 3, 1);
    CNTV_CVAL_EL0 = (3, 3, 14, 3, 2);
    /// Write-only: reading it is UNDEFINED.
    OSLAR_EL1 = (2, 0, 1, 0, 4);
    OSLSR_EL1 = (2, 0, 1, 1, 4);
    // The EL2 registers (FEAT_EL2), absent without it.
    SCTLR_EL2 = (3, 4, 1, 0, 0);
    HCR_EL2 = (3, 4, 1, 1, 0);
    MDCR_EL2 = (3, 4, 1, 1, 1);
    CPTR_EL2 = (3, 4, 1, 1, 2);
    TTBR0_EL2 = (3, 4, 2, 0, 0);
    TCR_EL2 = (3, 4, 2, 0, 2);
    VTTBR_EL2 = (3, 4, 2, 1, 0);
    VTCR_EL2 = (3, 4, 2, 1, 2);
    SPSR_EL2 = (3, 4, 4, 0, 0);
    ELR_EL2 = (3, 4, 4, 0, 1);
    SP_EL1 = (3, 4, 4, 1, 0);
    ESR_EL2 = (3, 4, 5, 2, 0);
    FAR_EL2 = (3, 4, 6, 0, 0);
    HPFAR_EL2 = (3, 4, 6, 0, 4);
    MAIR_EL2 = (3, 4, 10, 2, 0);
    VBAR_EL2 = (3, 4, 12, 0, 0);
    TPIDR_EL2 = (3, 4, 13, 0, 2);
    CNTVOFF_EL2 = (3, 4, 14, 0, 3);
    CNTPOFF_EL2 = (3, 4, 14, 0, 6);
    CNTHCTL_EL2 = (3, 4, 14, 1, 0);
    CNTHP_CTL_EL2 = (3, 4, 14, 2, 1);
    CNTHP_CVAL_EL2 = (3, 4, 14, 2, 2);
}

impl SysReg {
    const fn encode(op0: u8, op1: u8, crn: u8, crm: u8, op2: u8) -> Self {
        Self(
            ((op0 as u16 & 3) << 14)
                | ((op1 as u16 & 7) << 11)
                | ((crn as u16 & 15) << 7)
                | ((crm as u16 & 15) << 3)
                | (op2 as u16 & 7),
        )
    }

    /// The register `S<op0>_<op1>_C<crn>_C<crm>_<op2>`, if the fields name
    /// one in the system register space: `op0` 2 or 3, `op1` and `op2` below
    /// 8, `crn` and `crm` below 16.
    pub const fn new(op0: u8, op1: u8, crn: u8, crm: u8, op2: u8) -> Option<Self> {
        if (op0 == 2 || op0 == 3) && op1 < 8 && crn < 16 && crm < 16 && op2 < 8 {
            Some(Self::encode(op0, op1, crn, crm, op2))
        } else {
            None
        }
    }

    /// The fields packed as `MRS` packs them in its bits 20:5.
    pub const fn encoding(self) -> u16 {
        self.0
    }

    pub const fn op0(self) -> u8 {
        (self.0 >> 14) as u8
    }
    pub const fn op1(self) -> u8 {
        ((self.0 >> 11) & 7) as u8
    }
    pub const fn crn(self) -> u8 {
        ((self.0 >> 7) & 15) as u8
    }
    pub const fn crm(self) -> u8 {
        ((self.0 >> 3) & 15) as u8
    }
    pub const fn op2(self) -> u8 {
        (self.0 & 7) as u8
    }
}

/// Why a host's system register access was refused.
///
/// A host accesses with the highest implemented level's privilege (EL2
/// with FEAT_EL2, EL1 without) and without the controls that trap a guest
/// (`CPACR_EL1.FPEN`, `CPTR_EL2`, the EL0 enables), so what remains is what
/// the register itself allows. Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SysRegRefusal {
    /// This processor does not implement the register: a guest's access
    /// would be UNDEFINED.
    Absent,
    /// The register exists and cannot be written (an ID register, a
    /// counter, `CurrentEL`).
    ReadOnly,
    /// The register exists and cannot be read (`OSLAR_EL1`).
    WriteOnly,
}

impl core::fmt::Display for SysRegRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Absent => "this processor does not implement the register",
            Self::ReadOnly => "the register is read-only",
            Self::WriteOnly => "the register is write-only",
        })
    }
}

impl core::error::Error for SysRegRefusal {}

/// Why a host's `PSTATE` write is refused. Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PstateRefusal {
    /// The state names an exception level this processor does not
    /// implement: EL2 on one without FEAT_EL2.
    AbsentLevel,
}

impl core::fmt::Display for PstateRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::AbsentLevel => "this processor does not implement the exception level",
        })
    }
}

impl core::error::Error for PstateRefusal {}

/// The processor's architectural state at one moment.
///
/// A copy, not a view: it does not change as the machine runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuState {
    /// `X0..=X30`.
    pub x: [u64; 31],
    pub sp_el0: u64,
    pub sp_el1: u64,
    pub pc: u64,
    pub pstate: Pstate,
    /// `V0..=V31`, little-endian lanes.
    pub v: [u128; 32],
    pub fpcr: u64,
    pub fpsr: u64,
    pub elr_el1: u64,
    pub spsr_el1: u64,
    pub esr_el1: u64,
    pub far_el1: u64,
    pub vbar_el1: u64,
    pub sctlr_el1: u64,
    pub tcr_el1: u64,
    pub ttbr0_el1: u64,
    pub ttbr1_el1: u64,
    pub mair_el1: u64,
    /// The EL2 state, on a processor with EL2 ([`WithEl2`](super::WithEl2));
    /// a processor without has none.
    pub el2: Option<CpuStateEl2>,
    pub activity: Activity,
    /// Instructions retired since the machine was built.
    pub instructions: u64,
    /// Guest time since the machine was built, in ticks.
    pub ticks: u64,
}

/// The EL2 state of a processor with EL2 at one moment: `SP_EL2`, the
/// registers an exception to EL2 and its return use, and EL2's controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuStateEl2 {
    pub sp_el2: u64,
    pub elr_el2: u64,
    pub spsr_el2: u64,
    pub esr_el2: u64,
    pub far_el2: u64,
    pub vbar_el2: u64,
    pub sctlr_el2: u64,
    pub hcr_el2: u64,
}
