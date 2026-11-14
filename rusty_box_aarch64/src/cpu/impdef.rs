//! The IMPLEMENTATION DEFINED registers: the Cortex-A57's, and none on the
//! Armv9.2 model.
//!
//! Provenance (R7): the Cortex-A57 MPCore TRM, DDI 0488H — the AArch64
//! IMPLEMENTATION DEFINED register summary (4.2.15, Table 4-15) and each
//! register's description (4.3.31, 4.3.33, 4.3.48, 4.3.49, 4.3.52, 4.3.53,
//! 4.3.56 to 4.3.59 and 4.3.62 to 4.3.70), the implementation options its
//! configuration values are chosen from (1.5, Table 1-1) and the GIC's
//! PERIPHBASE input (8.2.1, 8.2.4); and the Arm ARM, DDI 0487M.d, for what
//! a processor without EL3 or EL2 makes of the controls those levels hold
//! (D1.1.3, RHRDVX) and for `HCR_EL2.TIDCP` (the `S3_<op1>_C11_*`,
//! `S3_<op1>_C15_*`, `S1_<op1>_C11_*` and `S1_<op1>_C15_*` accessors of
//! AARCHMRS `Registers.json`).
//!
//! The Cortex-A57's registers, at EL1 and EL2 (EL0's accesses are
//! UNDEFINED, Table 4-15's "-"):
//!
//! | Register | Encoding | Reset | Kept by a write |
//! |---|---|---|---|
//! | `L2CTLR_EL1` | `S3_1_C11_C0_2` | 0 ([`L2CTLR_CONFIG`]) | bits 21, 20, 9:5, 2:0 |
//! | `L2ECTLR_EL1` | `S3_1_C11_C0_3` | 0 | bits 2:0 |
//! | `L2ACTLR_EL1` | `S3_1_C15_C0_0` | 0x10 | [`L2ACTLR_FIELDS`] |
//! | `CPUACTLR_EL1` | `S3_1_C15_C2_0` | 0 | [`CPUACTLR_FIELDS`] |
//! | `CPUECTLR_EL1` | `S3_1_C15_C2_1` | 0x1B_0000_0000 | bits 38, 36:35, 33:32, 6, 2:0 |
//! | `CPUMERRSR_EL1` | `S3_1_C15_C2_2` | 0 | none: a write zeroes it |
//! | `L2MERRSR_EL1` | `S3_1_C15_C2_3` | 0 | none: a write zeroes it |
//! | `CBAR_EL1` | `S3_1_C15_C3_0` | PERIPHBASE | read-only |
//! | `IL1DATA0_EL1`..`IL1DATA3_EL1` | `S3_0_C15_C0_0`..`_3` | the board's fill | bits 31:0 |
//! | `DL1DATA0_EL1`..`DL1DATA4_EL1` | `S3_0_C15_C1_0`..`_4` | the board's fill | bits 31:0 |
//! | `RAMINDEX` | `SYS #0, C15, C4, #0` | | write-only |
//!
//! and the architectural registers Arm leaves IMPLEMENTATION DEFINED, as the
//! TRM gives them: `ACTLR_EL2` keeps its five access controls
//! ([`ACTLR_EL2_FIELDS`]); `ACTLR_EL1`, `AFSR0_EL1`, `AFSR1_EL1`,
//! `AFSR0_EL2`, `AFSR1_EL2`, `AMAIR_EL1`, `AMAIR_EL2` and `HACR_EL2` are
//! RES0 (4.3.31, 4.3.37, 4.3.48, 4.3.49, 4.3.52, 4.3.53, 4.3.56, 4.3.57):
//! they read as zero and ignore writes, their traps (`HCR_EL2.TACR`,
//! `TRVM`, `TVM`) as Arm has them.
//!
//! Access. `HCR_EL2.TIDCP` traps every EL1 access of the space to EL2 (EC
//! 0x18, the ISS naming the encoding), as the Arm accessor does. Reads are
//! open at EL1 and EL2. A write of `L2CTLR_EL1`, `L2ECTLR_EL1`,
//! `L2ACTLR_EL1`, `CPUACTLR_EL1` or `CPUECTLR_EL1` needs, at EL1, its
//! `ACTLR_EL3` bit and its `ACTLR_EL2` bit set, and at EL2 its `ACTLR_EL3`
//! bit (the TRM's footnotes to each register's accessibility; with no
//! Secure state `SCR_EL3.NS` is 1, so the Secure alternative never
//! applies). The TRM does not say what a write it does not permit does;
//! here it traps to the level whose control refused it, with the syndrome
//! `HCR_EL2.TIDCP` gives the same access. With no EL3, Arm's rule for a
//! control that would take an exception to an absent level (D1.1.3,
//! RHRDVX: its Effective value is the one that takes none) makes each
//! `ACTLR_EL3` bit 1, so EL2 writes them all; without EL2 the same rule
//! makes each `ACTLR_EL2` bit 1, so EL1 does.
//!
//! What the registers control. `CBAR_EL1` reads the PERIPHBASE input the
//! board ties to its GIC (`GicWindows::periphbase`), and `L2CTLR_EL1`'s
//! read-only fields the board's configuration ([`L2CTLR_CONFIG`]). No
//! memory error ever occurs, so the syndrome registers read zero and
//! `L2ECTLR_EL1`'s error flags stay clear. `CPUACTLR_EL1` bit 8 makes `WFI`
//! a NOP and bit 7 `WFE` (`Cpu::exec_wfi`, `Cpu::exec_wfe`). Every other
//! field — latencies, prefetchers, clock gating, retention, ECC, the
//! coherency and barrier optimisations, `SMPEN` — controls a part of the
//! processor the model does not have (caches, timing, a bus, other
//! processors), so it is kept and read back and changes nothing; bit 5,
//! `PLD` and `PLDW` as NOPs, is what `PRFM` already is here. `RAMINDEX`
//! reads the cache and TLB RAMs into `IL1DATAn_EL1` or `DL1DATAn_EL1`; the
//! model holds no RAM contents, so every entry is invalid and, in the
//! Non-secure EL1 and EL2 that are this processor's only states, an
//! invalid entry reads as zeros (4.3.64). A reserved `RAMID`, which the TRM
//! says corrupts the data registers, zeroes both sets.

use rusty_box_core::SnapError;
use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};

use super::el2::{El2Reg, HCR_TIDCP};
use super::regs::SysReg;
use super::sysreg::{SysRegError, SysRegKey, trap_to_el2};
use super::{Cpu, El, Levels, reset};
use crate::bus::PhysAddr;

/// What a processor gives the IMPLEMENTATION DEFINED register space
/// (`S3_<op1>_C11_*` and `S3_<op1>_C15_*`) and instruction space
/// (`S1_<op1>_C11_*` and `S1_<op1>_C15_*`), and the architectural
/// registers whose contents Arm leaves to the implementation.
///
/// Closed (R5): each variant is one processor model's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImpdefSpace {
    /// Nothing: every access does what the space's accessors make of it with
    /// nothing implemented ([`Cpu::impdef_space_access`]), and the
    /// architectural IMPLEMENTATION DEFINED registers hold all 64 bits and
    /// control nothing. The Armv9.2 model's: no document gives a software
    /// model such registers.
    Empty,
    /// The Cortex-A57's (the module's table), with its PERIPHBASE input
    /// tied to `periphbase`, which reset samples into `CBAR_EL1` (DDI
    /// 0488H, 8.2.1).
    CortexA57 { periphbase: PhysAddr },
}

impl ImpdefSpace {
    /// This space with the PERIPHBASE input tied to `periphbase`; a space
    /// without the input is unchanged.
    pub(crate) const fn with_periphbase(self, periphbase: PhysAddr) -> Self {
        match self {
            Self::Empty => Self::Empty,
            Self::CortexA57 { .. } => Self::CortexA57 { periphbase },
        }
    }

    /// Whether the processor gives `key` a register or an instruction of
    /// its own: an access to any other encoding of the space does what
    /// [`Cpu::impdef_space_access`] says. What the check of Arm's accessors
    /// (`sysreg_spec.rs`) leaves to the TRM's.
    #[cfg(test)]
    pub(crate) const fn implements(self, key: SysRegKey) -> bool {
        match self {
            Self::Empty => false,
            Self::CortexA57 { .. } => A57Reg::of(key).is_some() || is_ramindex(key),
        }
    }

    /// What `ACTLR_EL1`, `AFSR0_EL1`, `AFSR1_EL1` or `AMAIR_EL1` keeps of
    /// `value`: all of it, or on the Cortex-A57, where each is RES0,
    /// nothing.
    pub(crate) const fn auxiliary_kept(self, value: u64) -> u64 {
        match self {
            Self::Empty => value,
            Self::CortexA57 { .. } => 0,
        }
    }

    /// What the EL2 register `reg` keeps of `value`, which the
    /// architecture's rule ([`El2Reg::kept`]) has kept: on the Cortex-A57,
    /// `ACTLR_EL2` its access controls and `HACR_EL2`, `AMAIR_EL2`,
    /// `AFSR0_EL2` and `AFSR1_EL2`, RES0, nothing; every other register,
    /// and every register of a processor without the Cortex-A57's space,
    /// what the architecture keeps.
    pub(crate) const fn el2_kept(self, reg: El2Reg, value: u64) -> u64 {
        match self {
            Self::Empty => value,
            Self::CortexA57 { .. } => match reg {
                El2Reg::Actlr => value & ACTLR_EL2_FIELDS,
                El2Reg::Hacr | El2Reg::Amair | El2Reg::Afsr0 | El2Reg::Afsr1 => 0,
                El2Reg::Hcr
                | El2Reg::Sctlr
                | El2Reg::Cptr
                | El2Reg::Hstr
                | El2Reg::Mdcr
                | El2Reg::Tcr
                | El2Reg::Ttbr0
                | El2Reg::Vtcr
                | El2Reg::Vttbr
                | El2Reg::Mair
                | El2Reg::Vbar
                | El2Reg::Esr
                | El2Reg::Far
                | El2Reg::Hpfar
                | El2Reg::Elr
                | El2Reg::Spsr
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
                | El2Reg::Vsesr
                | El2Reg::Ttbr1
                | El2Reg::CnthvCtl
                | El2Reg::CnthvCval
                | El2Reg::Hfgrtr
                | El2Reg::Hfgwtr
                | El2Reg::Hfgitr
                | El2Reg::Hdfgrtr
                | El2Reg::Hdfgwtr
                | El2Reg::Hcrx
                | El2Reg::Vncr => value,
            },
        }
    }
}

// ----- ACTLR_EL2 -----

/// `ACTLR_EL2.CPUACTLR`, `CPUECTLR`, `L2CTLR`, `L2ECTLR` and `L2ACTLR`
/// (DDI 0488H, 4.3.33, Table 4-46): each set lets EL1 write its register.
pub(crate) const ACTLR_EL2_CPUACTLR: u64 = 1 << 0;
pub(crate) const ACTLR_EL2_CPUECTLR: u64 = 1 << 1;
pub(crate) const ACTLR_EL2_L2CTLR: u64 = 1 << 4;
pub(crate) const ACTLR_EL2_L2ECTLR: u64 = 1 << 5;
pub(crate) const ACTLR_EL2_L2ACTLR: u64 = 1 << 6;

/// The Cortex-A57's `ACTLR_EL2` fields; bits 31:7 and 3:2 are RES0.
pub(crate) const ACTLR_EL2_FIELDS: u64 = ACTLR_EL2_CPUACTLR
    | ACTLR_EL2_CPUECTLR
    | ACTLR_EL2_L2CTLR
    | ACTLR_EL2_L2ECTLR
    | ACTLR_EL2_L2ACTLR;

// ----- The registers' fields -----

/// The processors the board has: one (`MPIDR_EL1` affinity 0.0.0.0, the
/// GIC's one CPU interface).
const PROCESSORS: u64 = 1;

/// `L2CTLR_EL1`'s read-only fields as the board configures the processor
/// (DDI 0488H, 4.3.58, Table 4-69, from the options of 1.5, Table 1-1):
/// L2RSTDISABLE (31) 0, the L2's valid RAM reset by hardware; the number of
/// processors (25:24) `PROCESSORS - 1`, one; L1 cache ECC and parity
/// protection (22) 0, ECC and parity in the L2 only; and no L2 arbitration
/// (13), Tag RAM (12) or Data RAM (11:10) register slice, the first of
/// Table 1-2's valid slice combinations and the one the board's 512 KiB L2
/// (`Caches::CORTEX_A57`) needs no other for. The L2's size has no field
/// here; `CCSIDR_EL1` reports it.
pub(crate) const L2CTLR_CONFIG: u64 = (PROCESSORS - 1) << 24;

/// `L2CTLR_EL1`'s writable fields: ECC and parity enable (21), data inline
/// ECC enable (20), Tag RAM setup (9) and latency (8:6), Data RAM setup (5)
/// and latency (2:0).
pub(crate) const L2CTLR_FIELDS: u64 = (1 << 21) | (1 << 20) | (0b1_1111 << 5) | 0b111;

/// `L2ECTLR_EL1`'s writable field, L2 dynamic retention control (2:0)
/// (4.3.59, Table 4-70). Its error flags (30, 29), which a write of 0
/// clears and of 1 leaves, are never set: no error occurs.
pub(crate) const L2ECTLR_FIELDS: u64 = 0b111;

/// `L2ACTLR_EL1`'s fields (4.3.65, Table 4-76): bits 28:25, 23:13, 11:10,
/// 8:6 and 4:0; bits 31:29, 24, 12, 9 and 5 are RES0.
pub(crate) const L2ACTLR_FIELDS: u64 =
    (0b1111 << 25) | (0b111_1111_1111 << 13) | (0b11 << 10) | (0b111 << 6) | 0b1_1111;

/// `L2ACTLR_EL1` at reset with the ACE interface the board's processor
/// has (Table 4-15, footnote ag): "Disable WriteUnique and WriteLineUnique
/// transactions from master" (4) set.
pub(crate) const L2ACTLR_RESET: u64 = 0x10;

/// `CPUACTLR_EL1`'s RES0 bits (4.3.66, Tables 4-77 and 4-78): 62:60,
/// 46:45, 43:40, 35, 6 and 2. Every other bit is a field.
const CPUACTLR_RES0: u64 =
    (0b111 << 60) | (0b11 << 45) | (0b1111 << 40) | (1 << 35) | (1 << 6) | (1 << 2);
pub(crate) const CPUACTLR_FIELDS: u64 = !CPUACTLR_RES0;

/// `CPUACTLR_EL1` bit 8, "Execute WFI instruction as a NOP instruction",
/// and bit 7, the same of `WFE`.
pub(crate) const CPUACTLR_WFI_NOP: u64 = 1 << 8;
pub(crate) const CPUACTLR_WFE_NOP: u64 = 1 << 7;

/// `CPUECTLR_EL1`'s fields (4.3.67, Table 4-79): disable table walk
/// descriptor access prefetch (38), the L2 instruction fetch (36:35) and
/// load/store data (33:32) prefetch distances, `SMPEN` (6) and processor
/// dynamic retention control (2:0).
pub(crate) const CPUECTLR_FIELDS: u64 = (1 << 38) | (0b11 << 35) | (0b11 << 32) | (1 << 6) | 0b111;

/// `CPUECTLR_EL1` at reset: both prefetch distances 0b11.
pub(crate) const CPUECTLR_RESET: u64 = 0x1B_0000_0000;

/// `CBAR_EL1.PERIPHBASE`, bits 43:18 (4.3.70, Table 4-82).
pub(crate) const CBAR_PERIPHBASE: u64 = ((1 << 44) - 1) & !((1 << 18) - 1);

/// How many `IL1DATAn_EL1` and `DL1DATAn_EL1` registers there are (4.3.62,
/// 4.3.63).
pub(crate) const IL1DATA_COUNT: usize = 4;
pub(crate) const DL1DATA_COUNT: usize = 5;

/// `RAMINDEX`'s encoding, `SYS #0, C15, C4, #0` (4.3.64).
const RAMINDEX: SysRegKey = SysRegKey::new(1, 0, 15, 4, 0);

/// Whether `key` names `RAMINDEX`.
const fn is_ramindex(key: SysRegKey) -> bool {
    key.op0() == RAMINDEX.op0()
        && key.op1() == RAMINDEX.op1()
        && key.crn() == RAMINDEX.crn()
        && key.crm() == RAMINDEX.crm()
        && key.op2() == RAMINDEX.op2()
}

/// `IL1DATA<n>_EL1`'s encoding, `S3_0_C15_C0_<n>`.
pub(crate) const fn il1data(n: u8) -> SysReg {
    SysReg::encode(3, 0, 15, 0, n)
}

/// `DL1DATA<n>_EL1`'s encoding, `S3_0_C15_C1_<n>`.
pub(crate) const fn dl1data(n: u8) -> SysReg {
    SysReg::encode(3, 0, 15, 1, n)
}

/// A register of the Cortex-A57's IMPLEMENTATION DEFINED space, as its
/// encoding names it.
///
/// Closed (R5): Table 4-15's AArch64 registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum A57Reg {
    L2Ctlr,
    L2Ectlr,
    L2Actlr,
    CpuActlr,
    CpuEctlr,
    CpuMerrsr,
    L2Merrsr,
    Cbar,
    /// `IL1DATA<n>_EL1`, `n` below [`IL1DATA_COUNT`].
    Il1Data(usize),
    /// `DL1DATA<n>_EL1`, `n` below [`DL1DATA_COUNT`].
    Dl1Data(usize),
}

impl A57Reg {
    /// The register `key` names, if it names one.
    const fn of(key: SysRegKey) -> Option<Self> {
        Some(
            match (key.op0(), key.op1(), key.crn(), key.crm(), key.op2()) {
                (3, 1, 11, 0, 2) => Self::L2Ctlr,
                (3, 1, 11, 0, 3) => Self::L2Ectlr,
                (3, 1, 15, 0, 0) => Self::L2Actlr,
                (3, 1, 15, 2, 0) => Self::CpuActlr,
                (3, 1, 15, 2, 1) => Self::CpuEctlr,
                (3, 1, 15, 2, 2) => Self::CpuMerrsr,
                (3, 1, 15, 2, 3) => Self::L2Merrsr,
                (3, 1, 15, 3, 0) => Self::Cbar,
                (3, 0, 15, 0, n @ 0..=3) => Self::Il1Data(n as usize),
                (3, 0, 15, 1, n @ 0..=4) => Self::Dl1Data(n as usize),
                _ => return None,
            },
        )
    }

    /// The `ACTLR_EL2` bit (and `ACTLR_EL3` bit, of the same name) that
    /// lets a lower level write the register, for the five registers whose
    /// writes it controls.
    const fn write_control(self) -> Option<u64> {
        match self {
            Self::CpuActlr => Some(ACTLR_EL2_CPUACTLR),
            Self::CpuEctlr => Some(ACTLR_EL2_CPUECTLR),
            Self::L2Ctlr => Some(ACTLR_EL2_L2CTLR),
            Self::L2Ectlr => Some(ACTLR_EL2_L2ECTLR),
            Self::L2Actlr => Some(ACTLR_EL2_L2ACTLR),
            Self::CpuMerrsr | Self::L2Merrsr | Self::Cbar | Self::Il1Data(_) | Self::Dl1Data(_) => {
                None
            }
        }
    }
}

/// Which data registers a `RAMINDEX` operation's `RAMID` (bits 31:24)
/// fills (4.3.64, Table 4-75).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RamSide {
    /// The L1-I Tag, Data, BTB and GHB RAMs, TLB array and indirect
    /// predictor RAM (`RAMID` 0x00 to 0x05): `IL1DATAn_EL1`.
    Instruction,
    /// The L1-D Tag and Data RAMs and TLB array (0x08 to 0x0A) and the L2
    /// Tag, Data, Snoop Tag, Data ECC, Dirty and TLB RAMs (0x10 to 0x14,
    /// 0x18): `DL1DATAn_EL1`.
    Data,
    /// A reserved value, which corrupts the data registers.
    Reserved,
}

impl RamSide {
    const fn of(operand: u64) -> Self {
        match (operand >> 24) & 0xFF {
            0x00..=0x05 => Self::Instruction,
            0x08..=0x0A | 0x10..=0x14 | 0x18 => Self::Data,
            _ => Self::Reserved,
        }
    }
}

// ----- The state -----

/// What a processor holds of its IMPLEMENTATION DEFINED registers.
///
/// Closed (R5): each variant is one [`ImpdefSpace`]'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImpdefRegs {
    /// Nothing ([`ImpdefSpace::Empty`]).
    Empty,
    CortexA57(CortexA57Regs),
}

/// The Cortex-A57's registers that hold a value: each as a read returns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CortexA57Regs {
    pub(crate) l2ctlr: u64,
    pub(crate) l2ectlr: u64,
    pub(crate) l2actlr: u64,
    pub(crate) cpuactlr: u64,
    pub(crate) cpuectlr: u64,
    pub(crate) il1data: [u32; IL1DATA_COUNT],
    pub(crate) dl1data: [u32; DL1DATA_COUNT],
    /// `CBAR_EL1`: PERIPHBASE as reset sampled it.
    pub(crate) cbar: u64,
}

impl ImpdefRegs {
    /// The registers of `space` out of reset: the TRM's reset values
    /// (Table 4-15), `CBAR_EL1` PERIPHBASE, and the UNKNOWN data registers
    /// the board's fill, each naming its encoding in its low word
    /// ([`reset`]).
    pub(crate) const fn reset(space: ImpdefSpace) -> Self {
        match space {
            ImpdefSpace::Empty => Self::Empty,
            ImpdefSpace::CortexA57 { periphbase } => {
                let mut il1 = [0; IL1DATA_COUNT];
                let mut n = 0;
                while n < IL1DATA_COUNT {
                    il1[n] = reset::low_word_of(il1data(n as u8)) as u32;
                    n += 1;
                }
                let mut dl1 = [0; DL1DATA_COUNT];
                let mut n = 0;
                while n < DL1DATA_COUNT {
                    dl1[n] = reset::low_word_of(dl1data(n as u8)) as u32;
                    n += 1;
                }
                Self::CortexA57(CortexA57Regs {
                    l2ctlr: L2CTLR_CONFIG,
                    l2ectlr: 0,
                    l2actlr: L2ACTLR_RESET,
                    cpuactlr: 0,
                    cpuectlr: CPUECTLR_RESET,
                    il1data: il1,
                    dl1data: dl1,
                    cbar: periphbase.get() & CBAR_PERIPHBASE,
                })
            }
        }
    }

    /// Whether `CPUACTLR_EL1` makes `WFI` a NOP.
    pub(crate) const fn wfi_is_nop(&self) -> bool {
        match self {
            Self::Empty => false,
            Self::CortexA57(regs) => regs.cpuactlr & CPUACTLR_WFI_NOP != 0,
        }
    }

    /// Whether `CPUACTLR_EL1` makes `WFE` a NOP.
    pub(crate) const fn wfe_is_nop(&self) -> bool {
        match self {
            Self::Empty => false,
            Self::CortexA57(regs) => regs.cpuactlr & CPUACTLR_WFE_NOP != 0,
        }
    }
}

impl CortexA57Regs {
    /// What a read of `reg` returns.
    const fn read(&self, reg: A57Reg) -> u64 {
        match reg {
            A57Reg::L2Ctlr => self.l2ctlr,
            A57Reg::L2Ectlr => self.l2ectlr,
            A57Reg::L2Actlr => self.l2actlr,
            A57Reg::CpuActlr => self.cpuactlr,
            A57Reg::CpuEctlr => self.cpuectlr,
            // No memory error occurs: nothing is ever recorded.
            A57Reg::CpuMerrsr | A57Reg::L2Merrsr => 0,
            A57Reg::Cbar => self.cbar,
            A57Reg::Il1Data(n) => self.il1data[n] as u64,
            A57Reg::Dl1Data(n) => self.dl1data[n] as u64,
        }
    }

    /// A write of `value` to `reg`: what each keeps of it; `CBAR_EL1` is
    /// read-only, so its `MSR` is UNDEFINED.
    fn write(&mut self, reg: A57Reg, value: u64) -> Result<(), SysRegError> {
        match reg {
            A57Reg::L2Ctlr => self.l2ctlr = L2CTLR_CONFIG | (value & L2CTLR_FIELDS),
            A57Reg::L2Ectlr => self.l2ectlr = value & L2ECTLR_FIELDS,
            A57Reg::L2Actlr => self.l2actlr = value & L2ACTLR_FIELDS,
            A57Reg::CpuActlr => self.cpuactlr = value & CPUACTLR_FIELDS,
            A57Reg::CpuEctlr => self.cpuectlr = value & CPUECTLR_FIELDS,
            // "A write of any value to the register updates the register
            // to zero", which it is.
            A57Reg::CpuMerrsr | A57Reg::L2Merrsr => {}
            A57Reg::Cbar => return Err(SysRegError::Undefined),
            A57Reg::Il1Data(n) => self.il1data[n] = value as u32,
            A57Reg::Dl1Data(n) => self.dl1data[n] = value as u32,
        }
        Ok(())
    }

    /// `RAMINDEX` with `operand`: every RAM entry it can name is invalid,
    /// so it fills the data registers of the RAM's side with zeros, or,
    /// for a reserved `RAMID`, both sides'.
    fn ramindex(&mut self, operand: u64) {
        match RamSide::of(operand) {
            RamSide::Instruction => self.il1data = [0; IL1DATA_COUNT],
            RamSide::Data => self.dl1data = [0; DL1DATA_COUNT],
            RamSide::Reserved => {
                self.il1data = [0; IL1DATA_COUNT];
                self.dl1data = [0; DL1DATA_COUNT];
            }
        }
    }

    /// Whether every register holds a value a write could leave, the
    /// read-only fields the board's: what a restored image must hold.
    const fn writable_state(&self) -> bool {
        self.l2ctlr & !L2CTLR_FIELDS == L2CTLR_CONFIG
            && self.l2ectlr & !L2ECTLR_FIELDS == 0
            && self.l2actlr & !L2ACTLR_FIELDS == 0
            && self.cpuactlr & !CPUACTLR_FIELDS == 0
            && self.cpuectlr & !CPUECTLR_FIELDS == 0
    }
}

impl<L: Levels> Cpu<L> {
    /// An `MRS` at `el` (EL1 or EL2) of the IMPLEMENTATION DEFINED space's
    /// `key`: the register's value, or `HCR_EL2.TIDCP`'s trap of EL1's; an
    /// encoding the processor gives nothing does what
    /// [`Cpu::impdef_space_access`] says.
    pub(crate) fn impdef_register_read(&self, key: SysRegKey, el: El) -> Result<u64, SysRegError> {
        let (ImpdefRegs::CortexA57(regs), Some(reg)) = (&self.impdef, A57Reg::of(key)) else {
            return Err(self.impdef_space_access(el));
        };
        trap_to_el2(el == El::El1 && self.hcr_any(HCR_TIDCP))?;
        Ok(regs.read(reg))
    }

    /// An `MSR` of `value` at `el` (EL1 or EL2) to the IMPLEMENTATION
    /// DEFINED space's `key`: `HCR_EL2.TIDCP`'s trap of EL1's, then at EL1
    /// the register's `ACTLR_EL2` write control, which with no EL2 is 1
    /// (the module's "Access"), then the write; an encoding the processor
    /// gives nothing does what [`Cpu::impdef_space_access`] says.
    pub(crate) fn impdef_register_write(
        &mut self,
        key: SysRegKey,
        el: El,
        value: u64,
    ) -> Result<(), SysRegError> {
        let (ImpdefRegs::CortexA57(mut regs), Some(reg)) = (self.impdef, A57Reg::of(key)) else {
            return Err(self.impdef_space_access(el));
        };
        let el1 = el == El::El1;
        trap_to_el2(el1 && self.hcr_any(HCR_TIDCP))?;
        if let Some(control) = reg.write_control() {
            let refused = self.have_el2() && self.el2().get(El2Reg::Actlr) & control == 0;
            trap_to_el2(el1 && refused)?;
        }
        regs.write(reg, value)?;
        self.impdef = ImpdefRegs::CortexA57(regs);
        Ok(())
    }

    /// A `SYS` at `el` of the IMPLEMENTATION DEFINED space's `key`, with
    /// its register's value `operand`: `RAMINDEX` at EL1 and EL2, after
    /// `HCR_EL2.TIDCP`'s trap of EL1's; any other encoding, and any at
    /// EL0, does what [`Cpu::impdef_space_access`] says.
    pub(crate) fn impdef_instruction(
        &mut self,
        key: SysRegKey,
        el: El,
        operand: u64,
    ) -> Result<(), SysRegError> {
        let ImpdefRegs::CortexA57(mut regs) = self.impdef else {
            return Err(self.impdef_space_access(el));
        };
        if el == El::El0 || !is_ramindex(key) {
            return Err(self.impdef_space_access(el));
        }
        trap_to_el2(el == El::El1 && self.hcr_any(HCR_TIDCP))?;
        regs.ramindex(operand);
        self.impdef = ImpdefRegs::CortexA57(regs);
        Ok(())
    }

    /// The IMPLEMENTATION DEFINED registers' values, for a snapshot: on the
    /// Cortex-A57 `L2CTLR_EL1`, `L2ECTLR_EL1`, `L2ACTLR_EL1`,
    /// `CPUACTLR_EL1`, `CPUECTLR_EL1`, then `IL1DATA0_EL1` to
    /// `IL1DATA3_EL1` and `DL1DATA0_EL1` to `DL1DATA4_EL1`; nothing on a
    /// processor without them. `CBAR_EL1` is the machine's PERIPHBASE,
    /// which its shape records.
    pub(crate) fn save_impdef<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        let ImpdefRegs::CortexA57(regs) = &self.impdef else {
            return Ok(());
        };
        for value in [
            regs.l2ctlr,
            regs.l2ectlr,
            regs.l2actlr,
            regs.cpuactlr,
            regs.cpuectlr,
        ] {
            w.write_u64(value)?;
        }
        for value in regs.il1data.iter().chain(regs.dl1data.iter()) {
            w.write_u32(*value)?;
        }
        Ok(())
    }

    /// The inverse of [`Cpu::save_impdef`]: refuses a register holding a
    /// bit no write keeps, or read-only fields other than the board's,
    /// before changing anything.
    pub(crate) fn restore_impdef<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let ImpdefRegs::CortexA57(current) = self.impdef else {
            return Ok(());
        };
        let mut regs = CortexA57Regs {
            l2ctlr: r.read_u64()?,
            l2ectlr: r.read_u64()?,
            l2actlr: r.read_u64()?,
            cpuactlr: r.read_u64()?,
            cpuectlr: r.read_u64()?,
            ..current
        };
        for value in regs.il1data.iter_mut().chain(regs.dl1data.iter_mut()) {
            *value = r.read_u32()?;
        }
        if !regs.writable_state() {
            return Err(SnapError::Invalid(
                "snapshot's IMPLEMENTATION DEFINED register holds bits this processor cannot set",
            ));
        }
        self.impdef = ImpdefRegs::CortexA57(regs);
        Ok(())
    }

    /// Whether `ACTLR_EL1`, `AFSR0_EL1`, `AFSR1_EL1` and `AMAIR_EL1` in
    /// `sys` hold what this processor's writes can leave in them
    /// ([`ImpdefSpace::auxiliary_kept`]): what a restored image must hold.
    pub(crate) fn auxiliary_restorable(&self, sys: &super::SysRegs) -> bool {
        let space = self.imp.impdef;
        [sys.actlr_el1, sys.afsr0_el1, sys.afsr1_el1, sys.amair_el1]
            .into_iter()
            .all(|value| space.auxiliary_kept(value) == value)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/impdef.rs"]
mod tests;
