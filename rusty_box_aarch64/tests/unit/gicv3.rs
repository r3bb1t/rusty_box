//! The GICv3 model's registers against IHI 0069G: their reset values, the
//! fields that are RAZ/WI or read-only in a single Security state under
//! affinity routing, and what a CPU interface reads as interrupts become
//! pending, are acknowledged, preempt and end (R9: each test asserts what
//! software reads).

use std::vec;

use super::*;
use crate::bus::ApIndex;
use crate::devices::gicv2::PriorityBits;
use crate::identity::{GicDistFields, GicDistIidr, GicIdentity, MsiFrameIidr};

const GICD_CTLR: u64 = 0x0000;
const GICD_TYPER: u64 = 0x0004;
const GICD_IIDR: u64 = 0x0008;
const GICD_PIDR2: u64 = 0xFFE8;
const GICD_ITARGETSR: u64 = 0x0800;
const GICD_SGIR: u64 = 0x0F00;
const GICD_SPENDSGIR: u64 = 0x0F20;
const GICD_IGRPMODR: u64 = 0x0D00;
const GICR_IIDR: u64 = 0x0004;
const GICR_TYPER: u64 = 0x0008;
const GICR_WAKER: u64 = 0x0014;
const GICR_SETLPIR: u64 = 0x0040;
const GICR_INVALLR: u64 = 0x00B0;
const GICR_PIDR2: u64 = 0xFFE8;
const GICR_IGROUPR0: u64 = SGI_BASE + IGROUPR;
const GICR_ISENABLER0: u64 = SGI_BASE + ISENABLER;
const GICR_ISPENDR0: u64 = SGI_BASE + ISPENDR;
const GICR_ISACTIVER0: u64 = SGI_BASE + ISACTIVER;
const GICR_IPRIORITYR: u64 = SGI_BASE + IPRIORITYR;
const GICR_ICFGR0: u64 = SGI_BASE + ICFGR;
const GICR_ICFGR1: u64 = SGI_BASE + ICFGR + 4;
const GICR_IGRPMODR0: u64 = SGI_BASE + 0x0D00;
const GICR_NSACR: u64 = SGI_BASE + 0x0E00;

/// The SPI the tests drive, and its bank's word and bit.
const SPI: u32 = 40;

/// What `GICD_TYPER` reads of the default shape: 480 SPIs, fourteen INTID
/// bits, LPIs.
const TYPER: u32 = (13 << 19) | (1 << 17) | 15;

fn gic() -> GicV3 {
    GicV3::new(GicShape::DEFAULT, GicNames::NONE)
}

fn dist(g: &mut GicV3, off: u64, v: u32) {
    g.dist_write(RegOffset::new(off), AccessWidth::Word, u64::from(v));
}

fn dist_get(g: &mut GicV3, off: u64) -> u32 {
    g.dist_read(RegOffset::new(off), AccessWidth::Word) as u32
}

/// A write of a Redistributor register that reaches no table: no RAM.
fn redist(g: &mut GicV3, off: u64, v: u32) {
    let mut no_ram: [u8; 0] = [];
    let mut ram = GuestRam::new(&mut no_ram);
    g.redist_write(
        &mut ram,
        RegOffset::new(off),
        AccessWidth::Word,
        u64::from(v),
    );
}

fn redist_get(g: &mut GicV3, off: u64) -> u32 {
    g.redist_read(RegOffset::new(off), AccessWidth::Word) as u32
}

/// The bank register at `base` that holds interrupt `irq`, and its bit.
fn bank(base: u64, irq: u32) -> (u64, u32) {
    (base + u64::from(irq / 32) * 4, 1 << (irq % 32))
}

/// A GIC set up as an operating system sets one up: the Redistributor
/// awake, both groups enabled at the distributor and the CPU interface,
/// no priority masked.
fn running() -> GicV3 {
    let mut g = gic();
    redist(&mut g, GICR_WAKER, 0);
    dist(&mut g, GICD_CTLR, GICD_CTLR_EN_GRP0 | GICD_CTLR_EN_GRP1);
    g.icc_write(IccRegister::Igrpen0, 1);
    g.icc_write(IccRegister::Igrpen1, 1);
    g.icc_write(IccRegister::Pmr, 0xFF);
    g
}

/// SPI `irq` enabled in Group 1 (`group1`) or 0 at `priority`.
fn enable_spi(g: &mut GicV3, irq: u32, group1: bool, priority: u8) {
    let (igroupr, bit) = bank(IGROUPR, irq);
    let groups = dist_get(g, igroupr);
    dist(
        g,
        igroupr,
        if group1 { groups | bit } else { groups & !bit },
    );
    let (isenabler, bit) = bank(ISENABLER, irq);
    dist(g, isenabler, bit);
    g.dist_write(
        RegOffset::new(IPRIORITYR + u64::from(irq)),
        AccessWidth::Byte,
        u64::from(priority),
    );
}

fn make_pending(g: &mut GicV3, irq: u32) {
    let (ispendr, bit) = bank(ISPENDR, irq);
    dist(g, ispendr, bit);
}

/// The reset values IHI 0069G gives, and the board's where it leaves them
/// UNKNOWN: `GICD_CTLR` with `ARE` and `DS` set and the groups disabled,
/// `GICD_TYPER` of the shape's SPIs, fourteen INTID bits and LPIs, a
/// sleeping Redistributor, the last of its region, of processor 0, with
/// physical LPIs disabled, and the identification registers' `ArchRev` 3.
#[test]
fn registers_reset_to_their_values() {
    let mut g = gic();
    assert_eq!(dist_get(&mut g, GICD_CTLR), GICD_CTLR_ARE | GICD_CTLR_DS);
    assert_eq!(
        dist_get(&mut g, GICD_TYPER),
        TYPER,
        "480 SPIs, 14 INTID bits, LPIS"
    );
    assert_eq!(dist_get(&mut g, GICD_IIDR), 0);
    assert_eq!(dist_get(&mut g, GICD_PIDR2) >> 4, 3, "ArchRev: GICv3");
    assert_eq!(redist_get(&mut g, GICR_PIDR2) >> 4, 3);
    assert_eq!(
        [0xFFF0, 0xFFF4, 0xFFF8, 0xFFFC].map(|off| dist_get(&mut g, off)),
        [0x0D, 0xF0, 0x05, 0xB1],
        "the CoreSight preamble"
    );
    let typer = GICR_TYPER_LAST | GICR_TYPER_PLPIS;
    assert_eq!(redist_get(&mut g, GICR_TYPER), typer, "Last, PLPIS");
    assert_eq!(redist_get(&mut g, GICR_TYPER + 4), 0, "affinity 0.0.0.0");
    assert_eq!(
        g.redist_read(RegOffset::new(GICR_TYPER), AccessWidth::Dword),
        u64::from(typer)
    );
    assert_eq!(
        redist_get(&mut g, GICR_CTLR),
        0b10,
        "CES; EnableLPIs, RWP 0"
    );
    assert_eq!(
        redist_get(&mut g, GICR_WAKER),
        GICR_WAKER_PROCESSOR_SLEEP | GICR_WAKER_CHILDREN_ASLEEP
    );
    assert_eq!(
        redist_get(&mut g, GICR_ICFGR0),
        0xAAAA_AAAA,
        "every SGI edge-triggered"
    );
    assert_eq!(
        redist_get(&mut g, GICR_ICFGR1),
        0,
        "every PPI level-sensitive"
    );
    let five = PriorityBits::Five;
    assert_eq!(g.icc_read(IccRegister::Bpr0), u64::from(five.min_bpr()));
    assert_eq!(g.icc_read(IccRegister::Bpr1), u64::from(five.min_bpr() + 1));
    assert_eq!(g.icc_read(IccRegister::Rpr), 0xFF, "idle");
    assert_eq!(g.icc_read(IccRegister::Sre), 0b111, "SRE, DFB and DIB RAO");
    assert_eq!(g.icc_read(IccRegister::Ctlr), 4 << 8, "PRIbits: five bits");
    assert_eq!(g.icc_read(IccRegister::Iar1), 1023);
}

/// The identification registers name `GICD_IIDR`'s implementer and
/// product, by the CoreSight scheme; `GICR_IIDR` reads `GICD_IIDR`.
#[test]
fn identification_follows_the_distributors_iidr() {
    let iidr = GicDistIidr::new(GicDistFields {
        product: 0x52,
        variant: 0,
        revision: 1,
        jep106_continuation: 0x9,
        jep106_identity: 0x2B,
    })
    .expect("fields that fit");
    let names = GicNames {
        gic: GicIdentity {
            distributor: iidr,
            cpu_interface: GicIdentity::NONE.cpu_interface,
        },
        msi_frame: MsiFrameIidr::NONE,
    };
    let mut g = GicV3::new(GicShape::DEFAULT, names);
    assert_eq!(dist_get(&mut g, GICD_IIDR), iidr.get());
    assert_eq!(redist_get(&mut g, GICR_IIDR), iidr.get());
    let pidr = |g: &mut GicV3, n: u64| {
        let off = if n < 4 {
            0xFFE0 + 4 * n
        } else {
            0xFFD0 + 4 * (n - 4)
        };
        dist_get(g, off)
    };
    assert_eq!(pidr(&mut g, 0), 0x52, "PART_0");
    assert_eq!(
        pidr(&mut g, 1),
        0xB0,
        "DES_0, the identity code's low nibble"
    );
    assert_eq!(pidr(&mut g, 2), 0x3A, "ArchRev 3, JEDEC, DES_1");
    assert_eq!(pidr(&mut g, 4), 0x49, "SIZE 4, DES_2 the continuation code");
    assert_eq!(
        redist_get(&mut g, 0xFFD0),
        0x59,
        "the Redistributor's two frames"
    );
}

/// `GICD_CTLR` keeps its group enables alone, `ARE` and `DS` staying set;
/// the distributor's registers of INTIDs 0 to 31, the SGI and target
/// registers, the group modifiers and the direct LPI registers are RAZ/WI.
#[test]
fn affinity_routing_and_one_security_state_leave_registers_raz_wi() {
    let mut g = gic();
    dist(&mut g, GICD_CTLR, 0xFFFF_FFFF);
    assert_eq!(dist_get(&mut g, GICD_CTLR), 0x53);
    dist(&mut g, GICD_CTLR, 0);
    assert_eq!(dist_get(&mut g, GICD_CTLR), GICD_CTLR_ARE | GICD_CTLR_DS);
    dist(&mut g, GICD_TYPER, 0);
    assert_eq!(dist_get(&mut g, GICD_TYPER), TYPER, "read-only");
    for off in [
        IGROUPR,
        ISENABLER,
        ICENABLER,
        ISPENDR,
        ISACTIVER,
        IPRIORITYR,
        IPRIORITYR + 0x1C,
        ICFGR,
        ICFGR + 4,
        GICD_ITARGETSR + 0x20,
        GICD_IGRPMODR + 4,
        GICD_SPENDSGIR,
    ] {
        dist(&mut g, off, 0xFFFF_FFFF);
        assert_eq!(dist_get(&mut g, off), 0, "GICD {off:#x}");
    }
    dist(&mut g, GICD_SGIR, 0x0200_0001);
    assert_eq!(redist_get(&mut g, GICR_ISPENDR0), 0, "GICD_SGIR is WI");
    for off in [GICR_SETLPIR, GICR_INVALLR, GICR_IGRPMODR0, GICR_NSACR] {
        redist(&mut g, off, 0xFFFF_FFFF);
        assert_eq!(redist_get(&mut g, off), 0, "GICR {off:#x}");
    }
    redist(&mut g, GICR_ICFGR0, 0);
    redist(&mut g, GICR_ICFGR1, 0xFFFF_FFFF);
    assert_eq!(redist_get(&mut g, GICR_ICFGR0), 0xAAAA_AAAA);
    assert_eq!(redist_get(&mut g, GICR_ICFGR1), 0);
}

/// The PPIs and SPIs that do not exist are RAZ/WI, and a priority keeps
/// only its implemented bits, in either frame.
#[test]
fn absent_interrupts_and_priority_bits_are_raz_wi() {
    let mut g = gic();
    redist(&mut g, GICR_ISENABLER0, 0xFFFF_FFFF);
    assert_eq!(
        redist_get(&mut g, GICR_ISENABLER0),
        0xFE00_FFFF | (1 << 22) | (1 << 23),
        "the SGIs and the PPIs that exist"
    );
    let past_the_last = IGROUPR + 4 * 16;
    dist(&mut g, past_the_last, 0xFFFF_FFFF);
    assert_eq!(dist_get(&mut g, past_the_last), 0);
    g.dist_write(RegOffset::new(IPRIORITYR + 40), AccessWidth::Byte, 0xFF);
    assert_eq!(
        g.dist_read(RegOffset::new(IPRIORITYR + 40), AccessWidth::Byte),
        0xF8
    );
    redist(&mut g, GICR_IPRIORITYR + 4, 0x1234_5678);
    assert_eq!(redist_get(&mut g, GICR_IPRIORITYR + 4), 0x1030_5078);
    let route = IROUTER + 8 * u64::from(SPI);
    g.dist_write(RegOffset::new(route), AccessWidth::Dword, u64::MAX);
    assert_eq!(
        g.dist_read(RegOffset::new(route), AccessWidth::Dword),
        0xFF_80FF_FFFF,
        "IROUTER's RES0 bits"
    );
    dist(&mut g, IROUTER + 8 * 31, 0xFFFF_FFFF);
    assert_eq!(dist_get(&mut g, IROUTER + 8 * 31), 0, "reserved below 32");
}

/// An SPI pending, enabled and in Group 1 is signalled as IRQ once the
/// Redistributor wakes; `ICC_IAR1_EL1` acknowledges it, the running
/// priority becomes its priority, and `ICC_EOIR1_EL1` drops the priority
/// and deactivates it.
#[test]
fn a_group1_spi_is_signalled_acknowledged_and_ended() {
    let mut g = gic();
    dist(&mut g, GICD_CTLR, GICD_CTLR_EN_GRP1);
    g.icc_write(IccRegister::Igrpen1, 1);
    g.icc_write(IccRegister::Pmr, 0xFF);
    enable_spi(&mut g, SPI, true, 0x80);
    make_pending(&mut g, SPI);
    assert!(!g.irq_out(), "the Redistributor sleeps");
    assert_eq!(g.icc_read(IccRegister::Hppir1), 1023);
    redist(&mut g, GICR_WAKER, 0);
    assert_eq!(redist_get(&mut g, GICR_WAKER), 0);
    assert!(g.irq_out() && !g.fiq_out());
    assert_eq!(g.icc_read(IccRegister::Hppir1), u64::from(SPI));
    assert_eq!(g.icc_read(IccRegister::Hppir0), 1023, "of the other group");
    assert_eq!(g.icc_read(IccRegister::Iar0), 1023, "of the other group");
    assert_eq!(g.icc_read(IccRegister::Iar1), u64::from(SPI));
    assert!(!g.irq_out());
    assert_eq!(g.icc_read(IccRegister::Rpr), 0x80);
    let (isactiver, bit) = bank(ISACTIVER, SPI);
    assert_eq!(dist_get(&mut g, isactiver), bit);
    assert_eq!(
        g.icc_read(IccRegister::Ap1r(ApIndex::Zero)),
        1 << (0x80 >> 3)
    );
    g.icc_write(IccRegister::Eoir1, u64::from(SPI));
    assert_eq!(dist_get(&mut g, isactiver), 0);
    assert_eq!(g.icc_read(IccRegister::Rpr), 0xFF);
    assert_eq!(g.icc_read(IccRegister::Iar1), 1023, "nothing pending");
}

/// A Group 0 interrupt is signalled as FIQ and acknowledged through
/// `ICC_IAR0_EL1`, not `ICC_IAR1_EL1`; a group the CPU interface disables
/// is not signalled.
#[test]
fn group0_is_fiq_and_the_cpu_interface_enables_each_group() {
    let mut g = running();
    enable_spi(&mut g, SPI, false, 0x40);
    make_pending(&mut g, SPI);
    assert!(g.fiq_out() && !g.irq_out());
    assert_eq!(g.icc_read(IccRegister::Iar1), 1023);
    g.icc_write(IccRegister::Igrpen0, 0);
    assert!(!g.fiq_out());
    assert_eq!(g.icc_read(IccRegister::Hppir0), 1023);
    g.icc_write(IccRegister::Igrpen0, 1);
    assert_eq!(g.icc_read(IccRegister::Iar0), u64::from(SPI));
    g.icc_write(IccRegister::Eoir0, u64::from(SPI));
    assert_eq!(g.icc_read(IccRegister::Rpr), 0xFF);
}

/// `ICC_PMR_EL1` masks an interrupt of its priority or lower; one of a
/// higher group priority preempts an active interrupt, one of the same
/// group priority does not, under the binary point.
#[test]
fn the_priority_mask_and_preemption() {
    let mut g = running();
    enable_spi(&mut g, SPI, true, 0x80);
    enable_spi(&mut g, SPI + 1, true, 0x40);
    enable_spi(&mut g, SPI + 2, true, 0x48);
    g.icc_write(IccRegister::Pmr, 0x80);
    make_pending(&mut g, SPI);
    assert!(!g.irq_out(), "0x80 is not below the mask");
    assert_eq!(g.icc_read(IccRegister::Iar1), 1023);
    g.icc_write(IccRegister::Pmr, 0xF8);
    assert!(g.irq_out());
    assert_eq!(g.icc_read(IccRegister::Iar1), u64::from(SPI));
    make_pending(&mut g, SPI + 1);
    assert!(g.irq_out(), "0x40 preempts 0x80");
    assert_eq!(g.icc_read(IccRegister::Iar1), u64::from(SPI + 1));
    assert_eq!(g.icc_read(IccRegister::Rpr), 0x40);
    // With BPR1 at 5, group priority [7:5]: 0x48 and 0x40 are one group
    // priority, so 0x48 does not preempt.
    g.icc_write(IccRegister::Bpr1, 5);
    make_pending(&mut g, SPI + 2);
    assert!(!g.irq_out());
    assert_eq!(
        g.icc_read(IccRegister::Hppir1),
        u64::from(SPI + 2),
        "pending all the same"
    );
    g.icc_write(IccRegister::Eoir1, u64::from(SPI + 1));
    assert_eq!(g.icc_read(IccRegister::Rpr), 0x80);
    assert!(g.irq_out(), "0x48 preempts 0x80");
}

/// The running priority keeps an acknowledged interrupt's priority at the
/// minimum binary point (12.2.19 `ICC_RPR_EL1`, B.7 `PriorityGroup`), not
/// its group priority under the group's current binary point; preemption
/// still compares group priorities.
#[test]
fn the_running_priority_ignores_the_binary_point() {
    let mut g = running();
    g.icc_write(IccRegister::Pmr, 0xF8);
    g.icc_write(IccRegister::Bpr1, 5);
    enable_spi(&mut g, SPI, true, 0x38);
    enable_spi(&mut g, SPI + 1, true, 0x20);
    make_pending(&mut g, SPI);
    assert_eq!(g.icc_read(IccRegister::Iar1), u64::from(SPI));
    assert_eq!(g.icc_read(IccRegister::Rpr), 0x38);
    assert_eq!(
        g.icc_read(IccRegister::Ap1r(ApIndex::Zero)),
        1 << (0x38 >> 3)
    );
    make_pending(&mut g, SPI + 1);
    assert!(
        !g.irq_out(),
        "0x20 and 0x38 are one group priority under BPR1 5"
    );
}

/// With `EOImode` set an end of interrupt drops the priority only, and
/// `ICC_DIR_EL1` deactivates; with it clear `ICC_DIR_EL1` does nothing.
#[test]
fn eoimode_splits_priority_drop_and_deactivation() {
    let mut g = running();
    g.icc_write(IccRegister::Ctlr, 0b10);
    assert_eq!(g.icc_read(IccRegister::Ctlr), (4 << 8) | 0b10);
    enable_spi(&mut g, SPI, true, 0x80);
    make_pending(&mut g, SPI);
    assert_eq!(g.icc_read(IccRegister::Iar1), u64::from(SPI));
    g.icc_write(IccRegister::Eoir1, u64::from(SPI));
    let (isactiver, bit) = bank(ISACTIVER, SPI);
    assert_eq!(dist_get(&mut g, isactiver), bit, "still active");
    assert_eq!(g.icc_read(IccRegister::Rpr), 0xFF, "its priority dropped");
    g.icc_write(IccRegister::Ctlr, 0);
    g.icc_write(IccRegister::Dir, u64::from(SPI));
    assert_eq!(
        dist_get(&mut g, isactiver),
        bit,
        "DIR ignored without EOImode"
    );
    g.icc_write(IccRegister::Ctlr, 0b10);
    g.icc_write(IccRegister::Dir, u64::from(SPI));
    assert_eq!(dist_get(&mut g, isactiver), 0);
}

/// With `CBPR` set, `ICC_BPR1_EL1` reads `ICC_BPR0_EL1` plus one and
/// ignores writes; a binary point below the minimum sets the minimum.
#[test]
fn binary_points_and_cbpr() {
    let mut g = gic();
    g.icc_write(IccRegister::Bpr0, 0);
    assert_eq!(g.icc_read(IccRegister::Bpr0), 2, "five bits' minimum");
    g.icc_write(IccRegister::Bpr1, 0);
    assert_eq!(g.icc_read(IccRegister::Bpr1), 3);
    g.icc_write(IccRegister::Bpr0, 4);
    g.icc_write(IccRegister::Ctlr, 0b01);
    assert_eq!(g.icc_read(IccRegister::Bpr1), 5);
    g.icc_write(IccRegister::Bpr1, 7);
    g.icc_write(IccRegister::Ctlr, 0);
    assert_eq!(g.icc_read(IccRegister::Bpr1), 3, "the write was ignored");
    g.icc_write(IccRegister::Ap0r(ApIndex::Zero), u64::MAX);
    assert_eq!(
        g.icc_read(IccRegister::Ap0r(ApIndex::Zero)),
        0xFFFF_FFFF,
        "32 levels"
    );
}

/// An SGI generated through `ICC_SGI1R_EL1` to this processor becomes
/// pending; one naming another processor, or every other, does not;
/// `ICC_SGI0R_EL1` and `ICC_ASGI1R_EL1` reach a Group 0 SGI only.
#[test]
fn sgis_are_generated_to_this_processor_alone() {
    let mut g = running();
    redist(&mut g, GICR_ISENABLER0, 0xFFFF);
    redist(&mut g, GICR_IGROUPR0, 1 << 5);
    let sgi = |n: u64| n << 24 | 1;
    g.icc_write(IccRegister::Sgi1r, sgi(5) | 1 << 40);
    g.icc_write(IccRegister::Sgi1r, sgi(5) | 1 << 16);
    g.icc_write(IccRegister::Sgi1r, 5 << 24 | 0b10);
    assert_eq!(
        redist_get(&mut g, GICR_ISPENDR0),
        0,
        "no other processor exists"
    );
    g.icc_write(IccRegister::Sgi0r, sgi(5));
    g.icc_write(IccRegister::Asgi1r, sgi(5));
    assert_eq!(redist_get(&mut g, GICR_ISPENDR0), 0, "SGI 5 is Group 1");
    g.icc_write(IccRegister::Sgi1r, sgi(5));
    assert_eq!(redist_get(&mut g, GICR_ISPENDR0), 1 << 5);
    assert!(g.irq_out());
    g.icc_write(IccRegister::Sgi0r, sgi(3));
    assert_eq!(redist_get(&mut g, GICR_ISPENDR0), 1 << 5 | 1 << 3);
    assert!(
        g.fiq_out(),
        "SGI 3 is Group 0 and of the same priority but a lower ID"
    );
    assert_eq!(g.icc_read(IccRegister::Iar0), 3);
    assert_eq!(redist_get(&mut g, GICR_ISACTIVER0), 1 << 3);
}

/// A level-sensitive PPI is pending while its line is high, and an SPI
/// routed to another processor is not forwarded until it is routed here or
/// to any.
#[test]
fn lines_and_routes() {
    let mut g = running();
    redist(&mut g, GICR_ISENABLER0, 1 << 30);
    redist(&mut g, GICR_IGROUPR0, 1 << 30);
    g.set_line(30, true);
    assert!(g.irq_out());
    assert_eq!(redist_get(&mut g, GICR_ISPENDR0), 1 << 30);
    g.set_line(30, false);
    assert!(!g.irq_out());
    enable_spi(&mut g, SPI, true, 0);
    let route = IROUTER + 8 * u64::from(SPI);
    g.dist_write(RegOffset::new(route), AccessWidth::Dword, 0x1_0000_0000);
    make_pending(&mut g, SPI);
    assert!(!g.irq_out(), "routed to 1.0.0.0");
    g.dist_write(RegOffset::new(route), AccessWidth::Dword, IROUTER_IRM);
    assert!(g.irq_out(), "1 of N");
}

/// Where the LPI tests keep their tables, in RAM from `RAM_BASE`: the LPI
/// Configuration table 4 KiB aligned, the LPI Pending table 64 KiB aligned
/// (12.11.30, 12.11.31).
const PROP_TABLE: u64 = crate::board::RAM_BASE + 0x1_0000;
const PEND_TABLE: u64 = crate::board::RAM_BASE + 0x2_0000;
const TEST_RAM: usize = 0x4_0000;
/// The LPI Pending table's byte of LPI 8192's bit.
const LPI_PENDING_BYTE: usize = 0x2_0000 + 1024;
/// `GICR_PROPBASER.IDbits` of fourteen bits.
const ID_BITS_14: u64 = 13;

/// A 64-bit write of a Redistributor register, its tables in `ram`.
fn redist64(g: &mut GicV3, ram: &mut [u8], off: u64, v: u64) {
    let mut ram = GuestRam::new(ram);
    g.redist_write(&mut ram, RegOffset::new(off), AccessWidth::Dword, v);
}

fn redist64_get(g: &mut GicV3, off: u64) -> u64 {
    g.redist_read(RegOffset::new(off), AccessWidth::Dword)
}

/// LPI `intid`'s LPI Configuration table entry in `ram`.
fn config(ram: &mut [u8], intid: u32, entry: u8) {
    ram[0x1_0000 + (intid as usize - 8192)] = entry;
}

/// A running GIC whose Redistributor's tables are the tests', the LPI
/// Pending table read unless `zero` says it is zero, and LPIs enabled.
fn with_lpis(ram: &mut [u8], zero: bool) -> GicV3 {
    let mut g = running();
    redist64(&mut g, ram, GICR_PROPBASER, PROP_TABLE | ID_BITS_14);
    let ptz = if zero { 1 << 62 } else { 0 };
    redist64(&mut g, ram, GICR_PENDBASER, PEND_TABLE | ptz);
    redist64(&mut g, ram, GICR_CTLR, 1);
    g
}

/// As `EnableLPIs` goes from 0 to 1 the Redistributor reads the LPI
/// Configuration table and the pending bits of the LPI Pending table: the
/// pending, enabled LPI of the highest priority is signalled, acknowledged
/// (its pending state cleared, its priority the running one) and ended
/// with a priority drop alone; one the configuration disables is pending
/// but not signalled. As `EnableLPIs` goes back to 0 the pending state is
/// written to the table, and nothing is pending any longer.
#[test]
fn lpis_load_and_write_back_their_tables() {
    let mut ram = vec![0u8; TEST_RAM];
    config(&mut ram, 8192, 0xA0 | 0b11);
    config(&mut ram, 8193, 0x80 | 0b11);
    config(&mut ram, 8194, 0x40 | 0b10);
    ram[LPI_PENDING_BYTE] = 0b111;
    let mut g = with_lpis(&mut ram, false);
    assert_eq!(redist_get(&mut g, GICR_CTLR), 0b11, "EnableLPIs, CES");
    assert_eq!(
        redist64_get(&mut g, GICR_PROPBASER),
        PROP_TABLE | ID_BITS_14
    );
    assert!(g.irq_out() && !g.fiq_out(), "LPIs are Group 1");
    assert_eq!(g.icc_read(IccRegister::Hppir1), 8193, "8194 is disabled");
    assert_eq!(g.icc_read(IccRegister::Iar1), 8193);
    assert_eq!(g.icc_read(IccRegister::Rpr), 0x80);
    assert_eq!(
        g.icc_read(IccRegister::Iar1),
        1023,
        "8192, at 0xA0, does not preempt"
    );
    g.icc_write(IccRegister::Eoir1, 8193);
    assert_eq!(g.icc_read(IccRegister::Rpr), 0xFF, "the priority dropped");
    assert_eq!(g.icc_read(IccRegister::Hppir1), 8192);
    // A base register keeps its value while LPIs are enabled.
    redist64(&mut g, &mut ram, GICR_PROPBASER, 0);
    assert_eq!(
        redist64_get(&mut g, GICR_PROPBASER),
        PROP_TABLE | ID_BITS_14
    );
    redist64(&mut g, &mut ram, GICR_CTLR, 0);
    assert_eq!(redist_get(&mut g, GICR_CTLR), 0b10, "CES: cleared");
    assert_eq!(
        ram[LPI_PENDING_BYTE], 0b101,
        "8192 and the disabled 8194 still pending"
    );
    assert_eq!(g.icc_read(IccRegister::Hppir1), 1023);
    assert!(!g.irq_out());
    // Enabled again, the table's pending bits come back.
    redist64(&mut g, &mut ram, GICR_CTLR, 1);
    assert_eq!(g.icc_read(IccRegister::Iar1), 8192);
}

/// `GICR_PENDBASER.PTZ` reads 0 and tells the Redistributor the table is
/// zero, so it reads no pending bit from it; a `GICR_PROPBASER.IDbits`
/// below 13 leaves every LPI out of range.
#[test]
fn ptz_and_idbits_leave_lpis_unread() {
    let mut ram = vec![0u8; TEST_RAM];
    config(&mut ram, 8192, 0xA3);
    ram[LPI_PENDING_BYTE] = 1;
    let mut g = with_lpis(&mut ram, true);
    assert_eq!(
        redist64_get(&mut g, GICR_PENDBASER),
        PEND_TABLE,
        "PTZ reads 0"
    );
    assert_eq!(g.icc_read(IccRegister::Hppir1), 1023);
    let mut g = running();
    redist64(&mut g, &mut ram, GICR_PROPBASER, PROP_TABLE | 12);
    redist64(&mut g, &mut ram, GICR_PENDBASER, PEND_TABLE);
    redist64(&mut g, &mut ram, GICR_CTLR, 1);
    assert_eq!(g.icc_read(IccRegister::Hppir1), 1023, "out of range");
    redist64(&mut g, &mut ram, GICR_CTLR, 0);
    assert_eq!(ram[LPI_PENDING_BYTE], 1, "nothing written back");
}

/// The base registers keep their fields and drop their RES0 bits.
#[test]
fn lpi_base_registers_keep_their_fields() {
    let mut ram = vec![0u8; TEST_RAM];
    let mut g = running();
    redist64(&mut g, &mut ram, GICR_PROPBASER, u64::MAX);
    assert_eq!(redist64_get(&mut g, GICR_PROPBASER), 0x070F_FFFF_FFFF_FF9F);
    redist64(&mut g, &mut ram, GICR_PENDBASER, u64::MAX);
    assert_eq!(
        redist64_get(&mut g, GICR_PENDBASER),
        0x070F_FFFF_FFFF_0F80,
        "PTZ reads 0"
    );
    redist(&mut g, GICR_PROPBASER + 4, 0);
    assert_eq!(
        redist64_get(&mut g, GICR_PROPBASER),
        0xFFFF_FF9F,
        "the halves are separate"
    );
}

/// An LPI takes part in prioritization with the other interrupts: an SPI of
/// the same priority wins by its lower INTID, an LPI of a higher priority
/// preempts an active SPI, and a priority mask masks it.
#[test]
fn lpis_are_prioritized_with_the_other_interrupts() {
    let mut ram = vec![0u8; TEST_RAM];
    config(&mut ram, 8200, 0x60 | 0b11);
    ram[LPI_PENDING_BYTE + 1] = 1;
    let mut g = with_lpis(&mut ram, false);
    enable_spi(&mut g, SPI, true, 0x60);
    make_pending(&mut g, SPI);
    assert_eq!(
        g.icc_read(IccRegister::Iar1),
        u64::from(SPI),
        "the lower INTID"
    );
    assert_eq!(
        g.icc_read(IccRegister::Iar1),
        1023,
        "the same group priority"
    );
    g.icc_write(IccRegister::Eoir1, u64::from(SPI));
    enable_spi(&mut g, SPI + 1, true, 0x80);
    make_pending(&mut g, SPI + 1);
    g.icc_write(IccRegister::Pmr, 0x60);
    assert_eq!(g.icc_read(IccRegister::Hppir1), 8200);
    assert_eq!(g.icc_read(IccRegister::Iar1), 1023, "masked");
    g.icc_write(IccRegister::Pmr, 0xFF);
    assert_eq!(g.icc_read(IccRegister::Iar1), 8200);
    g.icc_write(IccRegister::Eoir1, 8200);
    assert_eq!(g.icc_read(IccRegister::Iar1), u64::from(SPI + 1));
}

/// A snapshot holds the LPIs' state: a pending LPI restores pending, and
/// an image with an LPI pending while LPIs are disabled is refused.
#[test]
fn snapshots_hold_the_lpis() {
    let mut ram = vec![0u8; TEST_RAM];
    config(&mut ram, 8192, 0x40 | 0b11);
    ram[LPI_PENDING_BYTE] = 1;
    let g = with_lpis(&mut ram, false);
    let mut image = std::vec::Vec::new();
    g.save(&mut image).expect("save");
    let mut other = gic();
    other.restore(&mut image.as_slice()).expect("restore");
    assert_eq!(redist_get(&mut other, GICR_CTLR), 0b11);
    assert_eq!(other.icc_read(IccRegister::Iar1), 8192);
    // The image ends with the LPIs' section (EnableLPIs, the two base
    // registers, the configuration cached and the pending words) and the
    // ITS's (its enable, its stall, CBASER, CWRITER, CREADR and two
    // BASERs), this GIC having no virtual interface.
    const LPI_SECTION: usize = 1 + 8 + 8 + 8192 + 8192 / 8;
    const ITS_SECTION: usize = 1 + 1 + 8 * 3 + 8 * 2;
    let enable_lpis = image.len() - ITS_SECTION - LPI_SECTION;
    let mut bad = image.clone();
    bad[enable_lpis] = 0;
    let mut quiet = running();
    assert!(
        quiet.restore(&mut bad.as_slice()).is_err(),
        "an LPI pending with LPIs disabled"
    );
    assert_eq!(redist_get(&mut quiet, GICR_CTLR), 0b10, "nothing changed");
}

/// A snapshot of a GIC in use restores to the same registers, and one
/// with a state its registers cannot hold is refused, changing nothing.
#[test]
fn snapshots_round_trip_and_refuse_impossible_state() {
    let mut g = running();
    enable_spi(&mut g, SPI, true, 0x80);
    make_pending(&mut g, SPI);
    assert_eq!(g.icc_read(IccRegister::Iar1), u64::from(SPI));
    let mut image = std::vec::Vec::new();
    g.save(&mut image).expect("save");
    let mut other = gic();
    other.restore(&mut image.as_slice()).expect("restore");
    assert_eq!(other.icc_read(IccRegister::Rpr), 0x80);
    assert_eq!(redist_get(&mut other, GICR_WAKER), 0);
    let (isactiver, bit) = bank(ISACTIVER, SPI);
    assert_eq!(dist_get(&mut other, isactiver), bit);
    // A priority mask with an unimplemented bit.
    let mut bad = image.clone();
    bad[5] = 0x01;
    let mut fresh = gic();
    assert!(fresh.restore(&mut bad.as_slice()).is_err());
    assert_eq!(fresh.icc_read(IccRegister::Pmr), 0, "nothing changed");
}
