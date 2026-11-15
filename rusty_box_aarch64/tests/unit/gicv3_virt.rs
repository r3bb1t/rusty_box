//! The GICv3 virtual interface's registers against IHI 0069G: their reset
//! values and RES0 fields, what `ICH_VTR_EL2` reports of each processor,
//! and what a hypervisor reads of its List registers, maintenance status
//! and active priorities, and a guest of its virtual CPU interface, as
//! virtual interrupts are signalled, acknowledged, preempt and end (R9:
//! each test asserts what software reads).

use super::*;
use crate::bus::{AccessWidth, GicRegister, GicTrap, IccRegister, RegOffset};
use crate::devices::gicv2::{GicNames, GicShape};
use crate::devices::gicv3::GicV3;
use crate::devices::virtio::queue::GuestRam;

const A57: VirtualInterfaceShape = VirtualInterfaceShape::CORTEX_A57;
/// The Armv9.2 model's on a CPU interface of eight priority bits: seven
/// virtual ones.
const V92: VirtualInterfaceShape = VirtualInterfaceShape::armv9p2(PriorityBits::Eight);

const PENDING: u64 = 0b01 << 62;
const ACTIVE: u64 = 0b10 << 62;
const PENDING_ACTIVE: u64 = 0b11 << 62;
const HW: u64 = 1 << 61;
const GROUP1: u64 = 1 << 60;
const EOI: u64 = 1 << 41;

/// A List register's `Priority` field for priority `p`.
const fn prio(p: u8) -> u64 {
    (p as u64) << 48
}

/// A hardware List register's `pINTID` field for physical interrupt
/// `intid`.
const fn physical(intid: u32) -> u64 {
    HW | (intid as u64) << 32
}

fn lr(n: u32) -> IchRegister {
    IchRegister::Lr(LrIndex::of(n))
}

/// An interface of `shape` with the virtual CPU interface enabled
/// (`ICH_HCR_EL2.En`), both groups enabled and no priority masked.
fn running(shape: VirtualInterfaceShape) -> VirtualInterface {
    let mut v = VirtualInterface::new(shape);
    v.control_write(IchRegister::Hcr, HCR_EN);
    write(&mut v, IcvRegister::Igrpen0, 1);
    write(&mut v, IcvRegister::Igrpen1, 1);
    write(&mut v, IcvRegister::Pmr, 0xFF);
    v
}

fn write(v: &mut VirtualInterface, reg: IcvRegister, value: u64) {
    assert_eq!(
        v.virtual_write(reg, value),
        PhysicalDeactivation(None),
        "{reg:?} deactivates no physical interrupt"
    );
}

/// Each processor's `ICH_VTR_EL2`, and the shape it reports.
#[test]
fn ich_vtr_reports_each_processors_virtual_interface() {
    // DDI 0488H Table 8-13.
    assert_eq!(A57.vtr(), 0x9000_0003);
    assert_eq!(A57.list_registers(), 4);
    assert_eq!(A57.priority_bits(), PriorityBits::Five);
    // Seven priority and preemption bits, nV4, TDS and 16 List registers.
    assert_eq!(V92.vtr(), 0xD818_000F);
    assert_eq!(V92.list_registers(), 16);
    assert_eq!(V92.priority_bits(), PriorityBits::Seven);
    // The Armv9.2 model's virtual priority bits follow the CPU
    // interface's, from five to seven.
    for (physical, vtr) in [
        (PriorityBits::Four, 0x9018_000F),
        (PriorityBits::Five, 0x9018_000F),
        (PriorityBits::Six, 0xB418_000F),
        (PriorityBits::Seven, 0xD818_000F),
        (PriorityBits::Eight, 0xD818_000F),
    ] {
        assert_eq!(
            VirtualInterfaceShape::armv9p2(physical).vtr(),
            vtr,
            "{physical:?}"
        );
    }
    for shape in [A57, V92] {
        let v = VirtualInterface::new(shape);
        assert_eq!(v.control_read(IchRegister::Vtr), shape.vtr());
    }
}

/// The registers out of reset: `ICH_HCR_EL2` 0; `ICH_VMCR_EL2` with the
/// least binary points and `VFIQEn` (RES1); every List register empty, so
/// `ICH_ELRSR_EL2` names each; no EOI and no maintenance interrupt;
/// `ICC_SRE_EL2` with `SRE`, `DFB` and `DIB` set and `Enable` clear.
#[test]
fn the_registers_reset_to_the_values_ihi_0069g_gives() {
    let v = VirtualInterface::new(A57);
    assert_eq!(v.control_read(IchRegister::Hcr), 0);
    assert_eq!(v.control_read(IchRegister::Vmcr), 0x004C_0008);
    assert_eq!(v.control_read(IchRegister::Elrsr), 0xF);
    assert_eq!(v.control_read(IchRegister::Eisr), 0);
    assert_eq!(v.control_read(IchRegister::Misr), 0);
    assert!(!v.maintenance());
    assert_eq!(v.sre_el2(), 0b111);
    for n in 0..4 {
        assert_eq!(v.control_read(lr(n)), 0);
    }
    let v = VirtualInterface::new(V92);
    // Seven preemption bits: VBPR0's least is 0, VBPR1's 1.
    assert_eq!(v.control_read(IchRegister::Vmcr), 0x0004_0008);
    assert_eq!(v.control_read(IchRegister::Elrsr), 0xFFFF);
}

/// All ones and all zeros written to each register read back with its
/// RES0 bits clear, its RES1 bit set, its unimplemented priority bits
/// clear, and its binary points at least their least: `TDIR` with `TDS`
/// alone, `TSEI` never (no SEIs).
#[test]
fn each_register_keeps_its_fields_alone() {
    let cases: [(VirtualInterfaceShape, IchRegister, u64, u64); 10] = [
        (A57, IchRegister::Hcr, 0xF800_1CFF, 0),
        (V92, IchRegister::Hcr, 0xF800_5CFF, 0),
        (A57, IchRegister::Vmcr, 0xF8FC_021B, 0x004C_0008),
        (V92, IchRegister::Vmcr, 0xFEFC_021B, 0x0004_0008),
        (A57, lr(3), 0xF0F8_1FFF_0000_FFFF, 0),
        (V92, lr(15), 0xF0FE_1FFF_0000_FFFF, 0),
        (A57, IchRegister::Ap0r(ApIndex::Zero), 0xFFFF_FFFF, 0),
        (V92, IchRegister::Ap1r(ApIndex::Three), 0xFFFF_FFFF, 0),
        // Read-only: writes change nothing.
        (A57, IchRegister::Vtr, 0x9000_0003, 0x9000_0003),
        (A57, IchRegister::Elrsr, 0xF, 0xF),
    ];
    for (shape, reg, ones, zeros) in cases {
        let mut v = VirtualInterface::new(shape);
        v.control_write(reg, u64::MAX);
        assert_eq!(v.control_read(reg), ones, "{shape:?} {reg:?} after ones");
        v.control_write(reg, 0);
        assert_eq!(v.control_read(reg), zeros, "{shape:?} {reg:?} after zeros");
    }
    let mut v = VirtualInterface::new(A57);
    v.set_sre_el2(u64::MAX);
    assert_eq!(v.sre_el2(), 0xF);
    v.set_sre_el2(0);
    assert_eq!(v.sre_el2(), 0x7);
}

/// A Group 1 interrupt in a List register is signalled as a virtual IRQ,
/// read as the highest pending, acknowledged (active, its preemption level
/// in `ICH_AP1R0_EL2`, the running priority its priority), and ended
/// (inactive, the List register empty again).
#[test]
fn a_list_register_interrupt_is_signalled_acknowledged_and_ended() {
    let mut v = running(A57);
    v.control_write(lr(1), PENDING | GROUP1 | prio(0xA0) | 27);
    assert_eq!(
        v.outputs(),
        VirtualLines {
            irq: true,
            fiq: false
        }
    );
    assert_eq!(v.virtual_read(IcvRegister::Hppir1), 27);
    assert_eq!(v.virtual_read(IcvRegister::Hppir0), 1023);
    assert_eq!(v.virtual_read(IcvRegister::Iar0), 1023, "not Group 0's");
    assert_eq!(v.virtual_read(IcvRegister::Iar1), 27);
    assert_eq!(v.control_read(lr(1)) >> 62, 0b10, "active");
    assert_eq!(
        v.control_read(IchRegister::Ap1r(ApIndex::Zero)),
        1 << (0xA0 >> 3)
    );
    assert_eq!(v.virtual_read(IcvRegister::Rpr), 0xA0);
    assert_eq!(v.outputs(), VirtualLines::NONE);
    assert_eq!(v.virtual_read(IcvRegister::Iar1), 1023);
    write(&mut v, IcvRegister::Eoir1, 27);
    assert_eq!(v.control_read(lr(1)) >> 62, 0, "inactive");
    assert_eq!(v.control_read(IchRegister::Ap1r(ApIndex::Zero)), 0);
    assert_eq!(v.virtual_read(IcvRegister::Rpr), 0xFF);
    assert_eq!(v.control_read(IchRegister::Elrsr), 0xF);
}

/// A Group 0 interrupt is a virtual FIQ (`VFIQEn` RES1), and with Group 0
/// disabled it is neither signalled nor the highest pending.
#[test]
fn group_0_is_a_virtual_fiq_while_enabled() {
    let mut v = running(A57);
    v.control_write(lr(0), PENDING | prio(0x40) | 33);
    assert_eq!(
        v.outputs(),
        VirtualLines {
            irq: false,
            fiq: true
        }
    );
    assert_eq!(v.virtual_read(IcvRegister::Hppir0), 33);
    write(&mut v, IcvRegister::Igrpen0, 0);
    assert_eq!(v.outputs(), VirtualLines::NONE);
    assert_eq!(v.virtual_read(IcvRegister::Hppir0), 1023);
    assert_eq!(v.virtual_read(IcvRegister::Iar0), 1023);
}

/// With `ICH_HCR_EL2.En` clear nothing is signalled or acknowledged.
#[test]
fn a_disabled_interface_signals_nothing() {
    let mut v = running(A57);
    v.control_write(lr(0), PENDING | GROUP1 | prio(0x40) | 40);
    v.control_write(IchRegister::Hcr, 0);
    assert_eq!(v.outputs(), VirtualLines::NONE);
    assert_eq!(v.virtual_read(IcvRegister::Iar1), 1023);
}

/// The priority mask and the running priority decide what is signalled:
/// an interrupt at or below `ICV_PMR_EL1` is not, nor one whose group
/// priority does not preempt the active one's under the binary point; the
/// numerically lowest priority wins, the lowest List register among equals.
#[test]
fn priority_mask_running_priority_and_binary_point_decide_preemption() {
    let mut v = running(A57);
    v.control_write(lr(0), PENDING | GROUP1 | prio(0x80) | 40);
    v.control_write(lr(1), PENDING | GROUP1 | prio(0x60) | 41);
    v.control_write(lr(2), PENDING | GROUP1 | prio(0x60) | 42);
    write(&mut v, IcvRegister::Pmr, 0x60);
    assert_eq!(
        v.outputs(),
        VirtualLines::NONE,
        "0x60 is not above PMR 0x60"
    );
    assert_eq!(v.virtual_read(IcvRegister::Iar1), 1023);
    assert_eq!(
        v.virtual_read(IcvRegister::Hppir1),
        41,
        "HPPIR ignores the mask"
    );
    write(&mut v, IcvRegister::Pmr, 0xFF);
    assert_eq!(
        v.virtual_read(IcvRegister::Iar1),
        41,
        "the lower-numbered of two"
    );
    // 42 at the same priority cannot preempt 41.
    assert_eq!(v.outputs(), VirtualLines::NONE);
    write(&mut v, IcvRegister::Eoir1, 41);
    assert_eq!(v.virtual_read(IcvRegister::Iar1), 42);
    // 0x50 preempts 0x60 under the least binary point...
    v.control_write(lr(3), PENDING | GROUP1 | prio(0x50) | 43);
    assert_eq!(
        v.outputs(),
        VirtualLines {
            irq: true,
            fiq: false
        }
    );
    // ...but not where `ICV_BPR1_EL1` 6 leaves bits [7:6] the group: 0x40
    // both.
    write(&mut v, IcvRegister::Bpr1, 6);
    assert_eq!(v.virtual_read(IcvRegister::Bpr1), 6);
    assert_eq!(v.outputs(), VirtualLines::NONE);
    // A binary point below the least is the least.
    write(&mut v, IcvRegister::Bpr1, 0);
    assert_eq!(v.virtual_read(IcvRegister::Bpr1), 3);
    write(&mut v, IcvRegister::Bpr0, 0);
    assert_eq!(v.virtual_read(IcvRegister::Bpr0), 2);
    // With CBPR, BPR1 reads BPR0 plus one and ignores writes.
    write(&mut v, IcvRegister::Ctlr, 1);
    assert_eq!(v.virtual_read(IcvRegister::Bpr1), 3);
    write(&mut v, IcvRegister::Bpr1, 6);
    assert_eq!(v.virtual_read(IcvRegister::Bpr1), 3);
    assert_eq!(v.control_read(IchRegister::Vmcr) >> 18 & 7, 3);
}

/// `ICV_CTLR_EL1` reports the virtual priority bits, and its `EOImode`
/// and `CBPR` are `ICH_VMCR_EL2`'s `VEOIM` and `VCBPR`.
#[test]
fn icv_ctlr_aliases_the_vmcr_and_reports_the_vtr() {
    let mut v = running(V92);
    assert_eq!(v.virtual_read(IcvRegister::Ctlr), 6 << 8);
    write(&mut v, IcvRegister::Ctlr, u64::MAX);
    assert_eq!(v.virtual_read(IcvRegister::Ctlr), 6 << 8 | 0b11);
    assert_eq!(
        v.control_read(IchRegister::Vmcr) & (VMCR_VEOIM | VMCR_VCBPR),
        VMCR_VEOIM | VMCR_VCBPR
    );
    let mut a57 = running(A57);
    assert_eq!(a57.virtual_read(IcvRegister::Ctlr), 4 << 8);
    write(&mut a57, IcvRegister::Pmr, 0xFF);
    assert_eq!(a57.virtual_read(IcvRegister::Pmr), 0xF8, "VPRIMask");
    assert_eq!(a57.control_read(IchRegister::Vmcr) >> 24, 0xF8);
}

/// A List register with `HW` 0 and `EOI` 1 asserts the EOI maintenance
/// interrupt once its interrupt is deactivated: `ICH_EISR_EL2` names it,
/// `ICH_ELRSR_EL2` does not, `ICH_MISR_EL2.EOI` is set.
#[test]
fn an_eoi_list_register_asserts_maintenance_when_deactivated() {
    let mut v = running(A57);
    v.control_write(lr(2), PENDING | GROUP1 | EOI | prio(0x80) | 50);
    assert_eq!(v.virtual_read(IcvRegister::Iar1), 50);
    assert!(!v.maintenance());
    write(&mut v, IcvRegister::Eoir1, 50);
    assert_eq!(v.control_read(IchRegister::Eisr), 1 << 2);
    assert_eq!(v.control_read(IchRegister::Elrsr), 0xF & !(1 << 2));
    assert_eq!(v.control_read(IchRegister::Misr), MISR_EOI);
    assert!(v.maintenance());
    // The hypervisor empties it.
    v.control_write(lr(2), 0);
    assert!(!v.maintenance());
    // No maintenance while the interface is disabled.
    v.control_write(lr(2), EOI);
    v.control_write(IchRegister::Hcr, 0);
    assert_eq!(v.control_read(IchRegister::Misr), MISR_EOI);
    assert!(!v.maintenance());
}

/// The maintenance conditions `ICH_HCR_EL2` enables, each in
/// `ICH_MISR_EL2`: underflow (no more than one valid List register), no
/// pending List register, an `EOIcount` not zero, and each group enabled or
/// disabled.
#[test]
fn ich_misr_reports_each_enabled_condition() {
    let mut v = running(A57);
    v.control_write(IchRegister::Hcr, HCR_EN | HCR_UIE | HCR_NPIE);
    assert_eq!(v.control_read(IchRegister::Misr), MISR_U | MISR_NP);
    v.control_write(lr(0), PENDING | 30);
    v.control_write(lr(1), ACTIVE | 31);
    assert_eq!(v.control_read(IchRegister::Misr), 0);
    v.control_write(lr(0), PENDING_ACTIVE | 30);
    assert_eq!(
        v.control_read(IchRegister::Misr),
        MISR_NP,
        "active and pending is not pending"
    );
    v.control_write(
        IchRegister::Hcr,
        HCR_EN | HCR_VGRP0EIE | HCR_VGRP0DIE | HCR_VGRP1EIE | HCR_VGRP1DIE,
    );
    assert_eq!(v.control_read(IchRegister::Misr), MISR_VGRP0E | MISR_VGRP1E);
    write(&mut v, IcvRegister::Igrpen1, 0);
    assert_eq!(v.control_read(IchRegister::Misr), MISR_VGRP0E | MISR_VGRP1D);
    v.control_write(IchRegister::Hcr, HCR_EN | HCR_LRENPIE | 3 << 27);
    assert_eq!(v.control_read(IchRegister::Misr), MISR_LRENP);
    assert!(v.maintenance());
}

/// An EOI of an interrupt no List register holds drops the running
/// priority and counts in `EOIcount`; with `VEOIM` set it counts nothing,
/// and a `ICV_DIR_EL1` of such an interrupt counts instead. An EOI with no
/// active priority does nothing.
#[test]
fn an_eoi_with_no_list_register_counts_in_eoicount() {
    let mut v = running(A57);
    v.control_write(IchRegister::Ap1r(ApIndex::Zero), 1 << 4);
    write(&mut v, IcvRegister::Eoir1, 99);
    assert_eq!(v.control_read(IchRegister::Ap1r(ApIndex::Zero)), 0);
    assert_eq!(v.control_read(IchRegister::Hcr) >> 27, 1);
    write(&mut v, IcvRegister::Eoir1, 99);
    assert_eq!(
        v.control_read(IchRegister::Hcr) >> 27,
        1,
        "no priority to drop"
    );
    write(&mut v, IcvRegister::Ctlr, 2);
    v.control_write(IchRegister::Ap1r(ApIndex::Zero), 1 << 4);
    write(&mut v, IcvRegister::Eoir1, 99);
    assert_eq!(
        v.control_read(IchRegister::Hcr) >> 27,
        1,
        "VEOIM: the DIR counts"
    );
    write(&mut v, IcvRegister::Dir, 99);
    assert_eq!(v.control_read(IchRegister::Hcr) >> 27, 2);
    // A special INTID is no deactivation.
    write(&mut v, IcvRegister::Dir, 1023);
    assert_eq!(v.control_read(IchRegister::Hcr) >> 27, 2);
}

/// With `VEOIM` set, an EOI drops the priority and leaves the interrupt
/// active; `ICV_DIR_EL1` deactivates it, an active and pending one staying
/// pending.
#[test]
fn split_eoi_mode_deactivates_through_dir() {
    let mut v = running(A57);
    write(&mut v, IcvRegister::Ctlr, 2);
    v.control_write(lr(0), PENDING | GROUP1 | prio(0x20) | 60);
    assert_eq!(v.virtual_read(IcvRegister::Iar1), 60);
    write(&mut v, IcvRegister::Eoir1, 60);
    assert_eq!(v.virtual_read(IcvRegister::Rpr), 0xFF);
    assert_eq!(v.control_read(lr(0)) >> 62, 0b10, "still active");
    v.control_write(lr(0), v.control_read(lr(0)) | PENDING);
    write(&mut v, IcvRegister::Dir, 60);
    assert_eq!(v.control_read(lr(0)) >> 62, 0b01, "pending again");
}

/// A hardware List register's deactivation names its physical interrupt,
/// below 1020; a software one's names none.
#[test]
fn a_hardware_list_register_deactivates_its_physical_interrupt() {
    let mut v = running(A57);
    v.control_write(lr(0), PENDING | GROUP1 | physical(27) | prio(0x80) | 27);
    assert_eq!(v.virtual_read(IcvRegister::Iar1), 27);
    assert_eq!(
        v.virtual_write(IcvRegister::Eoir1, 27),
        PhysicalDeactivation(Some(27))
    );
    v.control_write(lr(0), PENDING | GROUP1 | physical(1021) | prio(0x80) | 28);
    assert_eq!(v.virtual_read(IcvRegister::Iar1), 28);
    assert_eq!(
        v.virtual_write(IcvRegister::Eoir1, 28),
        PhysicalDeactivation(None)
    );
    // Hardware List registers hold no EOI maintenance: ELRSR names them.
    assert_eq!(v.control_read(IchRegister::Eisr), 0);
    assert_eq!(v.control_read(IchRegister::Elrsr), 0xF);
}

/// The seven preemption bits of the Armv9.2 model's interface: priority
/// 0xFC's level is 126, bit 30 of `ICH_AP0R3_EL2`.
#[test]
fn seven_preemption_bits_use_every_active_priority_register() {
    let mut v = running(V92);
    v.control_write(lr(15), PENDING | prio(0xFC) | 70);
    assert_eq!(v.virtual_read(IcvRegister::Iar0), 70);
    assert_eq!(v.control_read(IchRegister::Ap0r(ApIndex::Three)), 1 << 30);
    assert_eq!(v.virtual_read(IcvRegister::Rpr), 0xFC);
    write(&mut v, IcvRegister::Eoir0, 70);
    assert_eq!(v.control_read(IchRegister::Ap0r(ApIndex::Three)), 0);
}

/// A snapshot of the interface restores to the same registers, and an
/// image with a bit no field holds is refused.
#[test]
fn a_snapshot_round_trips_and_refuses_bits_no_field_holds() {
    let mut v = running(V92);
    v.control_write(lr(7), PENDING | GROUP1 | EOI | prio(0x42) | 77);
    v.control_write(IchRegister::Ap0r(ApIndex::Two), 0x10);
    v.set_sre_el2(SRE_EL2_ENABLE);
    let mut image = std::vec::Vec::new();
    v.save(&mut image).expect("save");
    let mut source: &[u8] = &image;
    let restored = VirtualInterface::restore(V92, &mut source).expect("restore");
    for reg in [
        IchRegister::Hcr,
        IchRegister::Vmcr,
        lr(7),
        IchRegister::Ap0r(ApIndex::Two),
    ] {
        assert_eq!(restored.control_read(reg), v.control_read(reg), "{reg:?}");
    }
    assert_eq!(restored.sre_el2(), 0xF);
    // ICH_HCR_EL2 bit 13, TSEI, is RES0.
    let mut bad = image.clone();
    bad[1] |= 1 << 5;
    let mut source: &[u8] = &bad;
    assert!(VirtualInterface::restore(V92, &mut source).is_err());
    // VFIQEn clear.
    let mut bad = image.clone();
    bad[8] &= !(1 << 3);
    let mut source: &[u8] = &bad;
    assert!(VirtualInterface::restore(V92, &mut source).is_err());
    let mut source: &[u8] = &image[..image.len() - 1];
    assert!(
        VirtualInterface::restore(V92, &mut source).is_err(),
        "truncated"
    );
}

/// At the GIC: the maintenance interrupt is PPI 25, pending at the
/// Redistributor while asserted; a hardware List register's deactivation
/// deactivates its physical interrupt; the virtual lines and the EL1 trap
/// controls are what the registers say.
#[test]
fn the_gic_wires_maintenance_hardware_deactivation_and_traps() {
    let mut g = GicV3::with_el2(GicShape::DEFAULT, GicNames::NONE, Some(A57));
    g.power_on_processor();
    const SGI_BASE: u64 = 0x1_0000;
    let redist = |g: &mut GicV3, off: u64, v: u32| {
        let mut no_ram: [u8; 0] = [];
        let mut ram = GuestRam::new(&mut no_ram);
        g.redist_write(
            &mut ram,
            RegOffset::new(off),
            AccessWidth::Word,
            u64::from(v),
        );
    };
    let redist_get =
        |g: &mut GicV3, off: u64| g.redist_read(RegOffset::new(off), AccessWidth::Word) as u32;
    let ich = |g: &mut GicV3, reg: IchRegister, v: u64| {
        g.register_write(GicRegister::Control(reg), v);
    };
    // PPI 25 and 27 enabled, Group 1.
    redist(&mut g, SGI_BASE + 0x80, (1 << 25) | (1 << 27));
    redist(&mut g, SGI_BASE + 0x100, (1 << 25) | (1 << 27));
    g.dist_write(RegOffset::new(0), AccessWidth::Word, 2);
    g.register_write(GicRegister::Physical(IccRegister::Igrpen1), 1);
    g.register_write(GicRegister::Physical(IccRegister::Pmr), 0xFF);
    assert!(!g.irq_out());

    ich(&mut g, IchRegister::Hcr, HCR_EN | HCR_UIE);
    assert_ne!(
        redist_get(&mut g, SGI_BASE + 0x200) & (1 << 25),
        0,
        "PPI 25 pending"
    );
    assert!(g.irq_out());
    assert_eq!(
        g.register_read(GicRegister::Physical(IccRegister::Iar1)),
        25
    );
    g.register_write(GicRegister::Physical(IccRegister::Eoir1), 25);
    ich(&mut g, IchRegister::Hcr, HCR_EN);
    assert_eq!(redist_get(&mut g, SGI_BASE + 0x200) & (1 << 25), 0);

    // The timer's PPI 27, taken by the hypervisor in EOImode 1 and handed
    // to the guest as a hardware List register.
    g.register_write(GicRegister::Physical(IccRegister::Ctlr), 2);
    g.set_line(27, true);
    assert_eq!(
        g.register_read(GicRegister::Physical(IccRegister::Iar1)),
        27
    );
    g.register_write(GicRegister::Physical(IccRegister::Eoir1), 27);
    g.set_line(27, false);
    assert_ne!(
        redist_get(&mut g, SGI_BASE + 0x300) & (1 << 27),
        0,
        "active"
    );
    ich(
        &mut g,
        IchRegister::Lr(LrIndex::of(0)),
        PENDING | GROUP1 | physical(27) | prio(0xA0) | 27,
    );
    g.register_write(GicRegister::Virtual(IcvRegister::Igrpen1), 1);
    g.register_write(GicRegister::Virtual(IcvRegister::Pmr), 0xFF);
    assert_eq!(
        g.virtual_lines(),
        VirtualLines {
            irq: true,
            fiq: false
        }
    );
    assert_eq!(g.register_read(GicRegister::Virtual(IcvRegister::Iar1)), 27);
    assert_eq!(g.virtual_lines(), VirtualLines::NONE);
    g.register_write(GicRegister::Virtual(IcvRegister::Eoir1), 27);
    assert_eq!(
        redist_get(&mut g, SGI_BASE + 0x300) & (1 << 27),
        0,
        "deactivated"
    );

    assert!(g.el1_traps().traps(GicTrap::SreEl1), "Enable resets clear");
    g.register_write(GicRegister::SreEl2, 0xF);
    assert_eq!(g.register_read(GicRegister::SreEl2), 0xF);
    assert!(!g.el1_traps().traps(GicTrap::SreEl1));
    ich(&mut g, IchRegister::Hcr, HCR_TC | HCR_TALL1);
    let traps = g.el1_traps();
    assert!(traps.traps(GicTrap::Common) && traps.traps(GicTrap::Group1));
    assert!(!traps.traps(GicTrap::Group0) && !traps.traps(GicTrap::Deactivate));

    // The whole GIC's snapshot, its virtual interface in it.
    let mut image = std::vec::Vec::new();
    g.save(&mut image).expect("save");
    let mut other = GicV3::with_el2(GicShape::DEFAULT, GicNames::NONE, Some(A57));
    let mut source: &[u8] = &image;
    other.restore(&mut source).expect("restore");
    assert_eq!(
        other.register_read(GicRegister::Control(IchRegister::Hcr)),
        HCR_TC | HCR_TALL1
    );
    let plain = GicV3::new(GicShape::DEFAULT, GicNames::NONE);
    let mut shorter = std::vec::Vec::new();
    plain.save(&mut shorter).expect("save");
    let mut source: &[u8] = &shorter;
    assert!(
        GicV3::with_el2(GicShape::DEFAULT, GicNames::NONE, Some(A57))
            .restore(&mut source)
            .is_err(),
        "a GIC with the virtual interface takes no image without it"
    );
}
