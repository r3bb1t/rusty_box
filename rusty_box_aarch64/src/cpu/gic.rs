//! The GIC CPU interface's System registers as the processor decodes them
//! (FEAT_GICv3): which `MRS` and `MSR` encodings name an `ICC_*_EL1`
//! register, and whether the processor has the interface at all, which
//! `ID_AA64PFR0_EL1.GIC` reports.
//!
//! Provenance (R7): Arm's machine-readable register data (AARCHMRS
//! `Registers.json`, release 2026-09), each `ICC_*_EL1` register's
//! encodings and `MRS`/`MSR` accessors, read for a processor with no EL2
//! and no EL3: UNDEFINED at EL0; at EL1 a trap to EL1 while
//! `ICC_SRE_EL1.SRE` is 0, which never happens here, the CPU interface
//! having no memory-mapped form (`SRE` RAO/WI, IHI 0069G 12.2.22); and
//! otherwise the access, but for `ICC_AP0R<n>_EL1` and `ICC_AP1R<n>_EL1`
//! of a preemption level the priority bits do not give (`n` 1 with fewer
//! than six bits, 2 and 3 with fewer than seven), which are UNDEFINED.
//! An encoding with no accessor in a direction — an `MSR` of a read-only
//! register, an `MRS` of a write-only one — is UNDEFINED in it. The
//! registers' state is the board's GIC's ([`crate::devices::gicv3`]), which
//! the processor reaches through its bus.

use super::sysreg::{Direction, SysRegKey};
use crate::bus::{ApIndex, IccRegister};
use crate::devices::gicv2::PriorityBits;

/// Whether the processor has the System register interface to a GIC CPU
/// interface (FEAT_GICv3), which follows the board's GIC: a board with a
/// GICv3 gives its processor one, a board with a GICv2 does not.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GicCpuInterface {
    /// None: `ID_AA64PFR0_EL1.GIC` 0, every `ICC_*` encoding UNDEFINED.
    Absent,
    /// The interface to versions 3.0 and 4.0 of the GIC CPU interface
    /// (`ID_AA64PFR0_EL1.GIC` 1), whose CPU interface implements these
    /// priority bits (`ICC_CTLR_EL1.PRIbits`), which decide the active
    /// priority registers it has.
    SystemRegisters(PriorityBits),
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

    /// The register an access in direction `dir` by encoding `key`
    /// reaches, if `key` names one of the interface's registers in that
    /// direction and the interface has it.
    pub(crate) fn register(self, key: SysRegKey, dir: Direction) -> Option<IccRegister> {
        match self {
            Self::Absent => None,
            Self::SystemRegisters(bits) => system_register(bits, key, dir),
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
            return None
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
    match (readable, dir) {
        (Access::ReadOnly, Direction::Write) | (Access::WriteOnly, Direction::Read) => None,
        (Access::ReadOnly | Access::ReadWrite, Direction::Read)
        | (Access::WriteOnly | Access::ReadWrite, Direction::Write) => Some(reg),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const FIVE: GicCpuInterface = GicCpuInterface::SystemRegisters(PriorityBits::Five);

    fn key(crn: u32, crm: u32, op2: u32) -> SysRegKey {
        SysRegKey::new(3, 0, crn, crm, op2)
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
        // registers are none of this interface's.
        for k in [
            key(12, 9, 5),
            SysRegKey::new(3, 4, 12, 9, 5),
            SysRegKey::new(3, 6, 12, 12, 4),
        ] {
            assert_eq!(FIVE.register(k, Read), None);
        }
    }

    /// The active priority registers past the first exist with six
    /// priority bits or more, the third and fourth with seven or more.
    #[test]
    fn the_active_priority_registers_follow_the_priority_bits() {
        for bits in PriorityBits::ALL {
            let interface = GicCpuInterface::SystemRegisters(bits);
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
