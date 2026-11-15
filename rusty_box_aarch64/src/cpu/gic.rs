//! The GIC CPU interface's System registers as the processor decodes them
//! (FEAT_GICv3): which `MRS` and `MSR` encodings name an `ICC_*_EL1`,
//! `ICH_*_EL2` or `ICC_SRE_EL2` register, what each access reaches at each
//! exception level — the physical CPU interface, the virtual one, a trap to
//! EL2, a load or store of the `VNCR_EL2` page, or nothing (UNDEFINED) —
//! and whether the processor has the interface at all, which
//! `ID_AA64PFR0_EL1.GIC` reports.
//!
//! Provenance (R7): Arm's machine-readable register data (AARCHMRS
//! `Registers.json`, release 2026-09), each `ICC_*_EL1`, `ICH_*_EL2` and
//! `ICC_SRE_EL2` register's encodings and `MRS`/`MSR` accessors, read for a
//! processor with no EL3 and without FEAT_GCIE: every access UNDEFINED at
//! EL0. An `ICC_*_EL1` encoding at EL1 traps to EL1 while `ICC_SRE_EL1.SRE`
//! is 0, which never happens here, the CPU interface having no
//! memory-mapped form (`SRE` RAO/WI, IHI 0069G 12.2.22); with EL2, in the
//! order each accessor makes them, `HFGRTR_EL2`/`HFGWTR_EL2.ICC_IGRPENn_EL1`
//! (FEAT_FGT) traps `ICC_IGRPEN0_EL1` and `ICC_IGRPEN1_EL1`, `ICH_HCR_EL2.TDIR`
//! `ICC_DIR_EL1`, `TC` the registers common to both groups, `TALL0` Group
//! 0's and `TALL1` Group 1's, and `ICC_SRE_EL2.Enable` clear `ICC_SRE_EL1`;
//! then `HCR_EL2.FMO` makes a Group 0 register's access, and `HCR_EL2.IMO` a
//! Group 1 register's, one to the virtual CPU interface's `ICV_*_EL1`, either
//! a common register's, and either traps the SGI generation registers. At
//! EL2 the access reaches the physical interface. An `ICH_*_EL2` or
//! `ICC_SRE_EL2` encoding exists only with EL2 and is UNDEFINED at EL1 but
//! under `HCR_EL2.NV` (FEAT_NV), where it traps to EL2 or, under `NV2` too
//! (FEAT_NV2), for `ICH_LR<n>_EL2`, `ICH_AP0R<n>_EL2`, `ICH_AP1R<n>_EL2`,
//! `ICH_HCR_EL2` and `ICH_VMCR_EL2`, is a load or store at the register's
//! offset of the `VNCR_EL2` page. `ICC_AP0R<n>_EL1` and `ICC_AP1R<n>_EL1` of
//! a preemption level the priority bits do not give (`n` 1 with fewer than
//! six bits, 2 and 3 with fewer than seven), `ICH_AP0R<n>_EL2` and
//! `ICH_AP1R<n>_EL2` of one the virtual preemption bits do not give, and an
//! `ICH_LR<n>_EL2` past the List registers are UNDEFINED. An encoding with no
//! accessor in a direction — an `MSR` of a read-only register, an `MRS` of a
//! write-only one — is UNDEFINED in it. The registers' state is the board's
//! GIC's ([`crate::devices::gicv3`]), which the processor reaches through its
//! bus.

use super::fgt::{FGT_ICC_IGRPEN, FgtReg, FineGrainedTrap};
use super::nv::VncrOffset;
use super::sysreg::{Direction, SysRegKey};
use super::{Cpu, El, Levels};
use crate::bus::{
    ApIndex, GicRegister, GicTrap, IccRegister, IchRegister, IcvRegister, LrIndex, PhysBus,
};
use crate::devices::gicv2::PriorityBits;
use crate::devices::gicv3_virt::VirtualInterfaceShape;

/// Whether the processor has the System register interface to a GIC CPU
/// interface (FEAT_GICv3), which follows the board's GIC: a board with a
/// GICv3 gives its processor one, a board with a GICv2 does not.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GicCpuInterface {
    /// None: `ID_AA64PFR0_EL1.GIC` 0, every `ICC_*` and `ICH_*` encoding
    /// UNDEFINED.
    Absent,
    /// The interface to versions 3.0 and 4.0 of the GIC CPU interface
    /// (`ID_AA64PFR0_EL1.GIC` 1).
    SystemRegisters(SystemRegisterInterface),
}

/// The GIC CPU interface a processor with FEAT_GICv3 reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SystemRegisterInterface {
    /// The physical interface's priority bits (`ICC_CTLR_EL1.PRIbits`),
    /// which decide the active priority registers it has.
    pub(crate) priority_bits: PriorityBits,
    /// The virtual interface a processor with EL2 has (`ICH_VTR_EL2`),
    /// which decides the List registers and the virtual active priority
    /// registers there are.
    pub(crate) virtual_interface: VirtualInterfaceShape,
}

impl GicCpuInterface {
    /// `ID_AA64PFR0_EL1.GIC` (bits 27:24): 1 with the System register
    /// interface, 0 without (Arm ARM D24.2, `ID_AA64PFR0_EL1`).
    pub(crate) const fn id_field(self) -> u64 {
        match self {
            Self::Absent => 0,
            Self::SystemRegisters(_) => 1 << 24,
        }
    }

    /// The `ICC_*_EL1` register an access in direction `dir` by encoding
    /// `key` reaches, if `key` names one of the interface's registers in
    /// that direction and the interface has it.
    pub(crate) fn register(self, key: SysRegKey, dir: Direction) -> Option<IccRegister> {
        match self {
            Self::Absent => None,
            Self::SystemRegisters(interface) => system_register(interface.priority_bits, key, dir),
        }
    }

    /// The EL2 register (`ICH_*_EL2` or `ICC_SRE_EL2`) an access in
    /// direction `dir` by encoding `key` reaches on a processor with EL2,
    /// if `key` names one in that direction and the interface has it.
    pub(crate) fn el2_register(self, key: SysRegKey, dir: Direction) -> Option<El2GicRegister> {
        match self {
            Self::Absent => None,
            Self::SystemRegisters(interface) => el2_register(interface.virtual_interface, key, dir),
        }
    }
}

/// The register of a CPU interface of `bits` priority bits an access in
/// direction `dir` by encoding `key` reaches ([`GicCpuInterface::register`]).
fn system_register(bits: PriorityBits, key: SysRegKey, dir: Direction) -> Option<IccRegister> {
    let reg = icc_register(key)?;
    let readable = match reg {
        IccRegister::Eoir0
        | IccRegister::Eoir1
        | IccRegister::Dir
        | IccRegister::Sgi0r
        | IccRegister::Sgi1r
        | IccRegister::Asgi1r => Access::WriteOnly,
        IccRegister::Iar0
        | IccRegister::Iar1
        | IccRegister::Hppir0
        | IccRegister::Hppir1
        | IccRegister::Rpr => Access::ReadOnly,
        IccRegister::Ap0r(n) | IccRegister::Ap1r(n) if !has_active_priorities(bits, n) => {
            return None;
        }
        IccRegister::Pmr
        | IccRegister::Bpr0
        | IccRegister::Bpr1
        | IccRegister::Ap0r(_)
        | IccRegister::Ap1r(_)
        | IccRegister::Ctlr
        | IccRegister::Sre
        | IccRegister::Igrpen0
        | IccRegister::Igrpen1 => Access::ReadWrite,
    };
    in_direction(readable, dir).then_some(reg)
}

/// Which accessors a register's encoding has.
///
/// Closed (R5).
#[derive(Clone, Copy)]
enum Access {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

/// Whether a register of `access` has an accessor in direction `dir`.
const fn in_direction(access: Access, dir: Direction) -> bool {
    match (access, dir) {
        (Access::ReadOnly, Direction::Write) | (Access::WriteOnly, Direction::Read) => false,
        (Access::ReadOnly | Access::ReadWrite, Direction::Read)
        | (Access::WriteOnly | Access::ReadWrite, Direction::Write) => true,
    }
}

/// Whether `ICC_AP0R<n>_EL1` and `ICC_AP1R<n>_EL1` exist for a CPU
/// interface of `bits` priority bits: the first always, the second with six
/// or more, the third and fourth with seven or more (IHI 0069G 4.8.3).
const fn has_active_priorities(bits: PriorityBits, n: ApIndex) -> bool {
    match n {
        ApIndex::Zero => true,
        ApIndex::One => bits.count() >= 6,
        ApIndex::Two | ApIndex::Three => bits.count() >= 7,
    }
}

/// The `ICC_*_EL1` register `key` encodes, of the interface's EL1
/// registers (`op0` 3, `op1` 0; `CRn` 4 for `ICC_PMR_EL1`, 12 for the
/// rest).
const fn icc_register(key: SysRegKey) -> Option<IccRegister> {
    if key.op0() != 3 || key.op1() != 0 {
        return None;
    }
    let (crn, crm, op2) = (key.crn(), key.crm(), key.op2());
    Some(match (crn, crm, op2) {
        (4, 6, 0) => IccRegister::Pmr,
        (12, 8, 0) => IccRegister::Iar0,
        (12, 8, 1) => IccRegister::Eoir0,
        (12, 8, 2) => IccRegister::Hppir0,
        (12, 8, 3) => IccRegister::Bpr0,
        (12, 8, 4..=7) => IccRegister::Ap0r(ApIndex::of(op2)),
        (12, 9, 0..=3) => IccRegister::Ap1r(ApIndex::of(op2)),
        (12, 11, 1) => IccRegister::Dir,
        (12, 11, 3) => IccRegister::Rpr,
        (12, 11, 5) => IccRegister::Sgi1r,
        (12, 11, 6) => IccRegister::Asgi1r,
        (12, 11, 7) => IccRegister::Sgi0r,
        (12, 12, 0) => IccRegister::Iar1,
        (12, 12, 1) => IccRegister::Eoir1,
        (12, 12, 2) => IccRegister::Hppir1,
        (12, 12, 3) => IccRegister::Bpr1,
        (12, 12, 4) => IccRegister::Ctlr,
        (12, 12, 5) => IccRegister::Sre,
        (12, 12, 6) => IccRegister::Igrpen0,
        (12, 12, 7) => IccRegister::Igrpen1,
        _ => return None,
    })
}

/// How the controls of EL2 treat an EL1 access to an `ICC_*_EL1` register
/// (each register's accessor).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AtEl1 {
    /// `ICC_SRE_EL1`: `ICC_SRE_EL2.Enable` clear traps it, and it has no
    /// virtual form.
    SreEl1,
    /// An SGI generation register: `TC` traps it, and so do `HCR_EL2.FMO`
    /// and `IMO`.
    Sgi,
    /// A register common to both groups: `TC` traps it (and `ICC_DIR_EL1`,
    /// `TDIR` before), and `HCR_EL2.FMO` or `IMO` make it the virtual one.
    Common(IcvRegister),
    /// A Group 0 register: `TALL0` traps it, `HCR_EL2.FMO` makes it the
    /// virtual one.
    Group0(IcvRegister),
    /// A Group 1 register: `TALL1` traps it, `HCR_EL2.IMO` makes it the
    /// virtual one.
    Group1(IcvRegister),
}

/// How the controls of EL2 treat an EL1 access to `reg`.
const fn at_el1(reg: IccRegister) -> AtEl1 {
    match reg {
        IccRegister::Sre => AtEl1::SreEl1,
        IccRegister::Sgi0r | IccRegister::Sgi1r | IccRegister::Asgi1r => AtEl1::Sgi,
        IccRegister::Pmr => AtEl1::Common(IcvRegister::Pmr),
        IccRegister::Ctlr => AtEl1::Common(IcvRegister::Ctlr),
        IccRegister::Dir => AtEl1::Common(IcvRegister::Dir),
        IccRegister::Rpr => AtEl1::Common(IcvRegister::Rpr),
        IccRegister::Iar0 => AtEl1::Group0(IcvRegister::Iar0),
        IccRegister::Eoir0 => AtEl1::Group0(IcvRegister::Eoir0),
        IccRegister::Hppir0 => AtEl1::Group0(IcvRegister::Hppir0),
        IccRegister::Bpr0 => AtEl1::Group0(IcvRegister::Bpr0),
        IccRegister::Ap0r(n) => AtEl1::Group0(IcvRegister::Ap0r(n)),
        IccRegister::Igrpen0 => AtEl1::Group0(IcvRegister::Igrpen0),
        IccRegister::Iar1 => AtEl1::Group1(IcvRegister::Iar1),
        IccRegister::Eoir1 => AtEl1::Group1(IcvRegister::Eoir1),
        IccRegister::Hppir1 => AtEl1::Group1(IcvRegister::Hppir1),
        IccRegister::Bpr1 => AtEl1::Group1(IcvRegister::Bpr1),
        IccRegister::Ap1r(n) => AtEl1::Group1(IcvRegister::Ap1r(n)),
        IccRegister::Igrpen1 => AtEl1::Group1(IcvRegister::Igrpen1),
    }
}

/// One of the CPU interface's EL2 registers, with where a guest
/// hypervisor's access to it goes under FEAT_NV2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct El2GicRegister {
    pub(crate) reg: GicRegister,
    /// Its offset in the `VNCR_EL2` page, if `NV2` transforms an access to
    /// it (`ICH_LR<n>_EL2` at 0x400 + 8n, `ICH_AP0R<n>_EL2` at 0x480 + 8n,
    /// `ICH_AP1R<n>_EL2` at 0x4A0 + 8n, `ICH_HCR_EL2` at 0x4C0 and
    /// `ICH_VMCR_EL2` at 0x4C8); `None` where `NV` traps it whatever `NV2`
    /// says.
    pub(crate) vncr: Option<VncrOffset>,
}

/// The EL2 register of a virtual interface of `shape` an access in
/// direction `dir` by encoding `key` reaches
/// ([`GicCpuInterface::el2_register`]).
fn el2_register(
    shape: VirtualInterfaceShape,
    key: SysRegKey,
    dir: Direction,
) -> Option<El2GicRegister> {
    if key.op0() != 3 || key.op1() != 4 || key.crn() != 12 {
        return None;
    }
    let (crm, op2) = (key.crm(), key.op2());
    let control = |reg, offset: u16, access| {
        let vncr = if offset == 0 {
            None
        } else {
            Some(VncrOffset::new(offset))
        };
        in_direction(access, dir).then_some(El2GicRegister {
            reg: GicRegister::Control(reg),
            vncr,
        })
    };
    match (crm, op2) {
        (8, 0..=3) | (9, 0..=3) => {
            let n = ApIndex::of(op2);
            if !shape.has_active_priorities(n) {
                return None;
            }
            let at = 8 * n.get() as u16;
            if crm == 8 {
                control(IchRegister::Ap0r(n), 0x480 + at, Access::ReadWrite)
            } else {
                control(IchRegister::Ap1r(n), 0x4A0 + at, Access::ReadWrite)
            }
        }
        (9, 5) => Some(El2GicRegister {
            reg: GicRegister::SreEl2,
            vncr: None,
        }),
        (11, 0) => control(IchRegister::Hcr, 0x4C0, Access::ReadWrite),
        (11, 1) => control(IchRegister::Vtr, 0, Access::ReadOnly),
        (11, 2) => control(IchRegister::Misr, 0, Access::ReadOnly),
        (11, 3) => control(IchRegister::Eisr, 0, Access::ReadOnly),
        (11, 5) => control(IchRegister::Elrsr, 0, Access::ReadOnly),
        (11, 7) => control(IchRegister::Vmcr, 0x4C8, Access::ReadWrite),
        (12 | 13, _) => {
            let n = LrIndex::of(((crm & 1) << 3) | op2);
            if !shape.has_list_register(n) {
                return None;
            }
            control(
                IchRegister::Lr(n),
                0x400 + 8 * n.get() as u16,
                Access::ReadWrite,
            )
        }
        _ => None,
    }
}

/// What an `MRS` or `MSR` of an encoding the processor's own register
/// decode does not hold does to the GIC CPU interface
/// ([`Cpu::gic_access`]).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GicAccess {
    /// The access to the register, which the bus completes.
    Register(GicRegister),
    /// A trap to EL2 (`AArch64_SystemAccessTrap(EL2, 0x18)`).
    TrapToEl2,
    /// FEAT_NV2's load or store at this offset of the `VNCR_EL2` page.
    Vncr(VncrOffset),
    /// No access: the encoding is UNDEFINED.
    Undefined,
}

impl<L: Levels> Cpu<L> {
    /// What an `MRS` (`dir` [`Direction::Read`]) or `MSR` by `key` does at
    /// the current EL to the GIC CPU interface, the controls of EL2 that
    /// trap it read from `bus` (the module's rules). The processor's own
    /// register decode finds these encodings UNDEFINED, so the decision is
    /// made only on that cold path.
    pub(crate) fn gic_access<B: PhysBus>(
        &self,
        bus: &B,
        key: SysRegKey,
        dir: Direction,
    ) -> GicAccess {
        let el = self.pstate.el;
        if let Some(reg) = self.imp.gic.register(key, dir) {
            return match el {
                El::El0 => GicAccess::Undefined,
                El::El1 if L::EL2 => self.el1_icc_access(bus, reg, dir),
                El::El1 | El::El2 => GicAccess::Register(GicRegister::Physical(reg)),
            };
        }
        if !L::EL2 {
            return GicAccess::Undefined;
        }
        let Some(target) = self.imp.gic.el2_register(key, dir) else {
            return GicAccess::Undefined;
        };
        match el {
            El::El0 => GicAccess::Undefined,
            El::El2 => GicAccess::Register(target.reg),
            El::El1 => {
                let nested = self.nested();
                match target.vncr {
                    Some(offset) if nested.nv2() => GicAccess::Vncr(offset),
                    Some(_) | None if nested.nv() => GicAccess::TrapToEl2,
                    Some(_) | None => GicAccess::Undefined,
                }
            }
        }
    }

    /// An EL1 access to `reg` on a processor with EL2, through the controls
    /// of EL2 in the order its accessor makes them ([`AtEl1`]).
    fn el1_icc_access<B: PhysBus>(&self, bus: &B, reg: IccRegister, dir: Direction) -> GicAccess {
        let Ok(traps) = bus.gic_el1_traps() else {
            return GicAccess::Undefined;
        };
        let hcr = self.hcr_el2();
        let fmo = hcr & super::el2::HCR_FMO != 0;
        let imo = hcr & super::el2::HCR_IMO != 0;
        let physical = GicAccess::Register(GicRegister::Physical(reg));
        let virtual_or_physical = |icv, virtualized: bool| {
            if virtualized {
                GicAccess::Register(GicRegister::Virtual(icv))
            } else {
                physical
            }
        };
        if matches!(reg, IccRegister::Igrpen0 | IccRegister::Igrpen1) {
            let fgt = match dir {
                Direction::Read => FgtReg::Hfgrtr,
                Direction::Write => FgtReg::Hfgwtr,
            };
            if self.fine_grained_traps(FineGrainedTrap::set(fgt, FGT_ICC_IGRPEN), El::El1) {
                return GicAccess::TrapToEl2;
            }
        }
        match at_el1(reg) {
            AtEl1::SreEl1 if traps.traps(GicTrap::SreEl1) => GicAccess::TrapToEl2,
            AtEl1::SreEl1 => physical,
            AtEl1::Sgi if traps.traps(GicTrap::Common) || fmo || imo => GicAccess::TrapToEl2,
            AtEl1::Sgi => physical,
            AtEl1::Common(IcvRegister::Dir) if traps.traps(GicTrap::Deactivate) => {
                GicAccess::TrapToEl2
            }
            AtEl1::Common(_) if traps.traps(GicTrap::Common) => GicAccess::TrapToEl2,
            AtEl1::Common(icv) => virtual_or_physical(icv, fmo || imo),
            AtEl1::Group0(_) if traps.traps(GicTrap::Group0) => GicAccess::TrapToEl2,
            AtEl1::Group0(icv) => virtual_or_physical(icv, fmo),
            AtEl1::Group1(_) if traps.traps(GicTrap::Group1) => GicAccess::TrapToEl2,
            AtEl1::Group1(icv) => virtual_or_physical(icv, imo),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIVE: GicCpuInterface = GicCpuInterface::SystemRegisters(SystemRegisterInterface {
        priority_bits: PriorityBits::Five,
        virtual_interface: VirtualInterfaceShape::CORTEX_A57,
    });

    fn key(crn: u32, crm: u32, op2: u32) -> SysRegKey {
        SysRegKey::new(3, 0, crn, crm, op2)
    }

    fn el2_key(crm: u32, op2: u32) -> SysRegKey {
        SysRegKey::new(3, 4, 12, crm, op2)
    }

    /// Each register at the encoding Arm gives it, in the directions its
    /// accessors have.
    #[test]
    fn each_register_is_at_its_encoding_in_its_directions() {
        use Direction::{Read, Write};
        let cases = [
            (key(4, 6, 0), IccRegister::Pmr, [true, true]),
            (key(12, 8, 0), IccRegister::Iar0, [true, false]),
            (key(12, 8, 1), IccRegister::Eoir0, [false, true]),
            (key(12, 8, 2), IccRegister::Hppir0, [true, false]),
            (key(12, 8, 3), IccRegister::Bpr0, [true, true]),
            (
                key(12, 8, 4),
                IccRegister::Ap0r(ApIndex::Zero),
                [true, true],
            ),
            (
                key(12, 9, 0),
                IccRegister::Ap1r(ApIndex::Zero),
                [true, true],
            ),
            (key(12, 11, 1), IccRegister::Dir, [false, true]),
            (key(12, 11, 3), IccRegister::Rpr, [true, false]),
            (key(12, 11, 5), IccRegister::Sgi1r, [false, true]),
            (key(12, 11, 6), IccRegister::Asgi1r, [false, true]),
            (key(12, 11, 7), IccRegister::Sgi0r, [false, true]),
            (key(12, 12, 0), IccRegister::Iar1, [true, false]),
            (key(12, 12, 1), IccRegister::Eoir1, [false, true]),
            (key(12, 12, 2), IccRegister::Hppir1, [true, false]),
            (key(12, 12, 3), IccRegister::Bpr1, [true, true]),
            (key(12, 12, 4), IccRegister::Ctlr, [true, true]),
            (key(12, 12, 5), IccRegister::Sre, [true, true]),
            (key(12, 12, 6), IccRegister::Igrpen0, [true, true]),
            (key(12, 12, 7), IccRegister::Igrpen1, [true, true]),
        ];
        for (k, reg, [read, write]) in cases {
            assert_eq!(FIVE.register(k, Read), read.then_some(reg), "{reg:?} read");
            assert_eq!(
                FIVE.register(k, Write),
                write.then_some(reg),
                "{reg:?} write"
            );
            assert_eq!(GicCpuInterface::Absent.register(k, Read), None);
            assert_eq!(GicCpuInterface::Absent.register(k, Write), None);
        }
        // ICC_NMIAR1_EL1 (FEAT_GICv3_NMI), ICC_SRE_EL2 and the EL3
        // registers are none of the EL1 interface's.
        for k in [
            key(12, 9, 5),
            SysRegKey::new(3, 4, 12, 9, 5),
            SysRegKey::new(3, 6, 12, 12, 4),
        ] {
            assert_eq!(FIVE.register(k, Read), None);
        }
    }

    /// The EL2 registers at their encodings, in the directions their
    /// accessors have, each with its `VNCR_EL2` offset: the List registers
    /// and active priority registers only as many as the shape has.
    #[test]
    fn each_el2_register_is_at_its_encoding_with_its_vncr_offset() {
        use Direction::{Read, Write};
        let reg = |k: SysRegKey, dir| FIVE.el2_register(k, dir);
        let control = |r: IchRegister, offset: Option<u64>| Some((GicRegister::Control(r), offset));
        let got = |k: SysRegKey, dir| reg(k, dir).map(|r| (r.reg, r.vncr.map(VncrOffset::get)));
        assert_eq!(
            got(el2_key(11, 0), Write),
            control(IchRegister::Hcr, Some(0x4C0))
        );
        assert_eq!(
            got(el2_key(11, 7), Read),
            control(IchRegister::Vmcr, Some(0x4C8))
        );
        assert_eq!(got(el2_key(11, 1), Read), control(IchRegister::Vtr, None));
        assert_eq!(got(el2_key(11, 1), Write), None);
        for (op2, r) in [
            (2, IchRegister::Misr),
            (3, IchRegister::Eisr),
            (5, IchRegister::Elrsr),
        ] {
            assert_eq!(got(el2_key(11, op2), Read), control(r, None));
            assert_eq!(got(el2_key(11, op2), Write), None);
        }
        assert_eq!(
            got(el2_key(8, 0), Read),
            control(IchRegister::Ap0r(ApIndex::Zero), Some(0x480))
        );
        assert_eq!(
            got(el2_key(9, 0), Write),
            control(IchRegister::Ap1r(ApIndex::Zero), Some(0x4A0))
        );
        // Five preemption bits: one active priority register a group.
        assert_eq!(got(el2_key(8, 1), Read), None);
        assert_eq!(got(el2_key(9, 3), Read), None);
        assert_eq!(got(el2_key(9, 5), Read), Some((GicRegister::SreEl2, None)));
        for n in 0..16u32 {
            let k = el2_key(12 | (n >> 3), n & 7);
            let want = (n < 4).then(|| {
                (
                    GicRegister::Control(IchRegister::Lr(LrIndex::of(n))),
                    Some(0x400 + 8 * u64::from(n)),
                )
            });
            assert_eq!(got(k, Read), want, "ICH_LR{n}_EL2");
        }
        let nine = GicCpuInterface::SystemRegisters(SystemRegisterInterface {
            priority_bits: PriorityBits::Eight,
            virtual_interface: VirtualInterfaceShape::armv9p2(PriorityBits::Eight),
        });
        for n in 0..4u32 {
            let ap0 = nine
                .el2_register(el2_key(8, n), Read)
                .map(|r| r.vncr.map(VncrOffset::get));
            let ap1 = nine
                .el2_register(el2_key(9, n), Write)
                .map(|r| r.vncr.map(VncrOffset::get));
            assert_eq!(ap0, Some(Some(0x480 + 8 * u64::from(n))));
            assert_eq!(ap1, Some(Some(0x4A0 + 8 * u64::from(n))));
        }
        let lr15 = nine
            .el2_register(el2_key(13, 7), Read)
            .map(|r| r.vncr.map(VncrOffset::get));
        assert_eq!(lr15, Some(Some(0x478)));
        assert_eq!(
            GicCpuInterface::Absent.el2_register(el2_key(11, 0), Read),
            None
        );
    }

    /// The active priority registers past the first exist with six
    /// priority bits or more, the third and fourth with seven or more.
    #[test]
    fn the_active_priority_registers_follow_the_priority_bits() {
        for bits in PriorityBits::ALL {
            let interface = GicCpuInterface::SystemRegisters(SystemRegisterInterface {
                priority_bits: bits,
                virtual_interface: VirtualInterfaceShape::CORTEX_A57,
            });
            for n in ApIndex::ALL {
                let present = match n.get() {
                    0 => true,
                    1 => bits.count() >= 6,
                    _ => bits.count() >= 7,
                };
                let op2 = n.get() as u32;
                assert_eq!(
                    interface
                        .register(key(12, 8, 4 + op2), Direction::Read)
                        .is_some(),
                    present,
                    "{bits:?} AP0R{op2}"
                );
                assert_eq!(
                    interface
                        .register(key(12, 9, op2), Direction::Write)
                        .is_some(),
                    present,
                    "{bits:?} AP1R{op2}"
                );
            }
        }
    }
}
