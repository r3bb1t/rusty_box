//! GICv3 Distributor, Redistributor and CPU interface for one processor, in
//! a single Security state, with the CPU interface reached through its
//! System registers only, and an ITS.
//!
//! Provenance (R7): the Arm Generic Interrupt Controller Architecture
//! Specification, GIC architecture version 3 and version 4 (IHI 0069G),
//! for a GICv3.0 implementation of these choices:
//! - One Security state: `GICD_CTLR.DS` RAO/WI (12.9.4), so `GICD_CTLR` has
//!   its single-state layout, `GICD_TYPER.SecurityExtn` is 0, the group
//!   modifier and Non-secure access registers (`GICD_IGRPMODR<n>`,
//!   `GICD_NSACR<n>`, `GICR_IGRPMODR0`, `GICR_NSACR`) are RAZ/WI, Group 0
//!   interrupts are signalled as FIQ and Group 1 as IRQ, and every SGI
//!   generation register reaches a target as Table 12-14 gives a
//!   Non-secure access with `GICD_CTLR.DS` 1.
//! - Affinity routing only, no GICv2 legacy operation: `GICD_CTLR.ARE` RAO/WI
//!   (12.9.4), so the distributor's registers of INTIDs 0 to 31 are RAZ/WI
//!   and `GICD_SGIR`, `GICD_ITARGETSR<n>`, `GICD_CPENDSGIR<n>` and
//!   `GICD_SPENDSGIR<n>` are RAZ/WI; and the System register interface
//!   only (12.2.22: an implementation may support only it), so
//!   `ICC_SRE_EL1.SRE` is RAO/WI, there is no IRQ or FIQ bypass
//!   (`ICC_SRE_EL1.DIB` and `DFB` RAO/WI), and there is no memory-mapped
//!   `GICC_*` frame.
//! - LPIs (`GICD_TYPER.LPIS` 1, `GICR_TYPER.PLPIS` 1) with fourteen bits of
//!   INTID (`GICD_TYPER.IDbits` 13, `num_LPIs` 0), the Redistributor's side
//!   in [`super::gicv3_lpi`], and one ITS ([`super::gicv3_its`]) through
//!   which every MSI comes, as BSA DEN0094C RB_GIC_03 asks of a GICv3 system
//!   with PCI Express; no direct LPIs (`GICR_TYPER.DirectLPI` 0) and no
//!   message-based SPIs (`GICD_TYPER.MBIS` 0, `GICD_SETSPI_NSR` and the
//!   others reserved), and no GICv2m frame.
//! - One Redistributor, for processor 0 at affinity 0.0.0.0, the last
//!   (`GICR_TYPER.Last`), whose frames `RD_base` and `SGI_base` are the
//!   128 KiB of its region; 1 of N distribution of SPIs
//!   (`GICD_TYPER.No1N` 0), the one processor being every SPI's
//!   participating node; affinity level 3 and range selectors not
//!   supported (`GICD_TYPER.A3V`, `RSS`, `ICC_CTLR_EL1.A3V`, `RSS` 0), as
//!   the Cortex-A57's CPU interface has them (DDI 0488H Table 8-13); 16
//!   bits of INTID at the CPU interface (`ICC_CTLR_EL1.IDbits` 0); no SEIs,
//!   no extended ranges, no RAS (`GICD_STATUSR`, `GICR_STATUSR` RAZ/WI).
//! - The priority bits the board's [`GicShape`] gives, the same at the
//!   Distributor, the Redistributor and the CPU interface
//!   (`ICC_CTLR_EL1.PRIbits`), 4 to 8 with one Security state (4.8); the
//!   minimum binary points (Table 4-11) and the active priority registers
//!   (4.8.3, Table 4-12) follow from them, as in the GICv2 model
//!   ([`PriorityBits`](super::gicv2::PriorityBits)).
//!
//! Where the specification leaves a choice to the implementation:
//! - A Group the distributor (`GICD_CTLR.EnableGrpN`) or the CPU interface
//!   (`ICC_IGRPENn_EL1`) disables takes no part in choosing the highest
//!   priority pending interrupt (4.7.2: required for the distributor's and
//!   for 1 of N SPIs, IMPLEMENTATION DEFINED for a directly routed one).
//! - A Redistributor whose `GICR_WAKER.ProcessorSleep` is 1 holds every
//!   interrupt and forwards none (12.11.40); `ChildrenAsleep` follows it at
//!   once, there being no interface to quiesce. `ProcessorSleep` resets to
//!   1, as the specification requires, and the firmware that powers the
//!   processor on clears it ([`GicV3::power_on_processor`]).
//! - SGIs can be enabled and disabled (4.7.1); a PPI is level-sensitive, its
//!   `GICR_ICFGR1` field not programmable (12.11.8), as in the GIC-400.
//! - An SPI whose `GICD_IROUTER<n>` names no processor (IRM 0, an affinity
//!   other than 0.0.0.0) is forwarded to none, and reads back as written
//!   (12.9.20, the first CONSTRAINED UNPREDICTABLE choice).
//! - An end of interrupt that does not name the highest active priority's
//!   group is ignored, and one that does drops that priority; an
//!   `ICC_DIR_EL1` write while `EOImode` is 0 is ignored (both UNPREDICTABLE,
//!   12.2.6, 12.2.9, 12.2.10). An end of interrupt of a special INTID
//!   (1020 to 1023) does nothing (`InterruptIdentifierValid`, 2.2.3).
//! - Each reset value the specification leaves UNKNOWN is 0 — the groups'
//!   enables, the groups (`GICR_IGROUPR0`), the routes, the active
//!   priorities and the LPI tables' base registers — but for the binary
//!   points, which reset to their minimum.
//! - LPIs, always Group 1 (5.1), take part in choosing the highest priority
//!   pending interrupt as the other Group 1 interrupts do, under the same
//!   group enables, an SGI, PPI or SPI winning a tie of priority by its
//!   lower INTID; acknowledging one clears its pending state, and an end of
//!   interrupt naming one drops the running priority alone, an LPI having no
//!   active state.
//! - `GICD_PIDR0` to `GICD_PIDR4` and `GICR_PIDR0` to `GICR_PIDR4` follow
//!   the CoreLink and CoreSight identification scheme Arm recommends
//!   (12.1.13): `ArchRev` 3, the designer and part from `GICD_IIDR`'s
//!   implementer and product (the board's identity), the size the frames
//!   occupy; the component ID registers the scheme's preamble with class 0xF,
//!   a generic component.
//! - Registers decode by aligned word; `GICD_IPRIORITYR<n>` and
//!   `GICR_IPRIORITYR<n>` by byte too, and an 8-byte access is two word
//!   accesses, the low first, which covers the 64-bit `GICD_IROUTER<n>`,
//!   `GICR_TYPER`, `GICR_PROPBASER` and `GICR_PENDBASER` (12.1.3); any
//!   other access reads as zero and writes nothing (UNPREDICTABLE there).
//!
//! The highest-priority pending interrupt and its signalling follow the
//! specification's pseudocode (B.7.2 `CanSignalInterrupt`,
//! `HighestPriorityPendingInterrupt`, 4.8 `GroupBits`): an enabled,
//! pending, inactive interrupt of an enabled group, the lowest priority
//! value winning and the lowest INTID among equals, is signalled when its
//! priority is below `ICC_PMR_EL1` and its group priority, under its own
//! group's binary point, above the running priority's.
//!
//! For a processor with EL2 the CPU interface has its virtualization
//! support too ([`super::gicv3_virt`]): the virtual interface control
//! registers, the virtual CPU interface and `ICC_SRE_EL2`. Its maintenance
//! interrupt drives PPI 25 as a level-sensitive line of this
//! Redistributor's, and a hardware List register's deactivation
//! deactivates its physical interrupt here (IHI 0069G 12.4.6 `HW`), as an
//! `ICC_DIR_EL1` write would, whatever `EOImode` says.
//!
//! Interrupt state is recomputed after every register access and every
//! input line change; the machine reads [`GicV3::irq_out`],
//! [`GicV3::fiq_out`] and [`GicV3::virtual_lines`] between instructions.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use super::gicv2::{
    group_mask, preempts, Bits, GicNames, GicShape, SpiCount, GICD_CTLR_EN_GRP0,
    GICD_CTLR_EN_GRP1, IDLE_PRIORITY, MAX_APRS, MAX_IRQ, NR_SGIS, SGI_BITS, SPURIOUS, WORDS,
};
use super::gicv3_its::Its;
use super::gicv3_lpi::{Lpis, GICR_PENDBASER, GICR_PROPBASER, ID_BITS};
use super::gicv3_virt::{
    PhysicalDeactivation, VirtualInterface, VirtualInterfaceShape, MAINTENANCE_INTID,
};
use crate::bus::{AccessWidth, GicEl1Traps, GicRegister, IccRegister, RegOffset};
use crate::cpu::VirtualLines;
use crate::devices::virtio::queue::GuestRam;

/// `GICD_CTLR.ARE`, RAO/WI: affinity routing always.
const GICD_CTLR_ARE: u32 = 1 << 4;
/// `GICD_CTLR.DS`, RAO/WI: one Security state.
const GICD_CTLR_DS: u32 = 1 << 6;
/// `GICD_TYPER.IDbits`: the INTID bits, minus one; and `LPIS`.
const GICD_TYPER_IDBITS: u32 = (ID_BITS - 1) << 19;
const GICD_TYPER_LPIS: u32 = 1 << 17;
/// `GICR_TYPER.PLPIS`: physical LPIs; and `Last`, the last Redistributor of
/// its region.
const GICR_TYPER_PLPIS: u32 = 1 << 0;
const GICR_TYPER_LAST: u32 = 1 << 4;
/// `GICR_CTLR`, from `RD_base`.
const GICR_CTLR: u64 = 0x0000;
/// The INTIDs with a special meaning (2.2.1).
const SPECIAL_INTIDS: core::ops::RangeInclusive<u32> = 1020..=1023;
/// `GICR_WAKER.ProcessorSleep` and `ChildrenAsleep`.
const GICR_WAKER_PROCESSOR_SLEEP: u32 = 1 << 1;
const GICR_WAKER_CHILDREN_ASLEEP: u32 = 1 << 2;
/// `ICC_CTLR_EL1`'s writable fields: `CBPR` and `EOImode`.
const ICC_CTLR_CBPR: u32 = 1 << 0;
const ICC_CTLR_EOIMODE: u32 = 1 << 1;
/// `ICC_SRE_EL1`: `SRE`, `DFB` and `DIB`, each RAO/WI.
const ICC_SRE: u64 = 0b111;
/// The bits of a `GICD_IROUTER<n>` that hold something: `Aff3` [39:32],
/// `Interrupt_Routing_Mode` [31], `Aff2`, `Aff1` and `Aff0` [23:0].
const IROUTER_MASK: u64 = 0xFF_80FF_FFFF;
const IROUTER_IRM: u64 = 1 << 31;
/// Where the register banks start, in the distributor and in the
/// Redistributor's `SGI_base` frame alike.
const IGROUPR: u64 = 0x080;
const ISENABLER: u64 = 0x100;
const ICENABLER: u64 = 0x180;
const ISPENDR: u64 = 0x200;
const ICPENDR: u64 = 0x280;
const ISACTIVER: u64 = 0x300;
const ICACTIVER: u64 = 0x380;
const IPRIORITYR: u64 = 0x400;
const ICFGR: u64 = 0xC00;
/// `GICD_IROUTER<n>`, eight bytes apart from `GICD_IROUTER0`.
const IROUTER: u64 = 0x6000;
/// The Redistributor's `SGI_base` frame, 64 KiB after its `RD_base`.
const SGI_BASE: u64 = 0x1_0000;
/// The identification registers at the top of each 64 KiB frame.
const ID_REGISTERS: u64 = 0xFFD0;
/// `GICD_PIDR2.ArchRev` and `GICR_PIDR2.ArchRev`: GICv3.
const ARCH_REV_GICV3: u32 = 3;
/// What the frames occupy, as the identification scheme's `SIZE` counts
/// it: the log2 of the 4 KiB blocks, 64 KiB for the distributor and the
/// Redistributor's two frames of 64 KiB.
const DIST_SIZE_LOG2_BLOCKS: u32 = 4;
const REDIST_SIZE_LOG2_BLOCKS: u32 = 5;
/// The identification scheme's component ID registers: its preamble, with
/// class 0xF, a generic component.
const COMPONENT_ID: [u32; 4] = [0x0D, 0xF0, 0x05, 0xB1];
/// The SPIs a GIC can have, a `GICD_IROUTER<n>` each.
const MAX_SPIS: usize = SpiCount::MAX.get() as usize;

/// Whether the Redistributor's processor is asleep (`GICR_WAKER`).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessorSleep {
    /// Interrupts are forwarded to the CPU interface.
    Awake,
    /// Interrupts are held at the Redistributor, as out of reset.
    Asleep,
}

/// The interrupt controller.
#[derive(Clone)]
pub struct GicV3 {
    /// `GICD_CTLR`'s group enables.
    dist_ctlr: u32,
    enabled: Bits,
    /// Set by an edge, by software or by an SGI, cleared by acknowledgement
    /// or software. A level-sensitive interrupt is also pending while its
    /// line is high.
    pending: Bits,
    active: Bits,
    group1: Bits,
    /// The level of each input line.
    level: Bits,
    /// Edge-triggered: every SGI, and the SPIs `GICD_ICFGR<n>` makes so.
    edge: Bits,
    /// The SPIs whose `GICD_IROUTER<n>` reaches processor 0: 1 of N, or
    /// its affinity.
    routed: Bits,
    priority: [u8; MAX_IRQ],
    /// `GICD_IROUTER<n>` of each SPI, SPI `n` at `n - 32`.
    route: [u64; MAX_SPIS],
    sleep: ProcessorSleep,
    /// `ICC_PMR_EL1`.
    pmr: u8,
    /// `ICC_BPR0_EL1` and `ICC_BPR1_EL1`.
    bpr0: u8,
    bpr1: u8,
    /// `ICC_CTLR_EL1`'s `CBPR` and `EOImode`.
    icc_ctlr: u32,
    /// `ICC_IGRPEN0_EL1.Enable` and `ICC_IGRPEN1_EL1.Enable`, as the group
    /// enable bits of `GICD_CTLR`.
    igrpen: u32,
    /// `ICC_AP0R<n>_EL1` and `ICC_AP1R<n>_EL1`: one bit per active
    /// preemption level, in the layout the priority bits give (Table 4-12).
    ap0r: [u32; MAX_APRS],
    ap1r: [u32; MAX_APRS],
    /// The highest priority pending interrupt the Redistributor and the
    /// Distributor present to the CPU interface, recomputed by
    /// [`GicV3::update`].
    hppi: Option<usize>,
    irq_out: bool,
    fiq_out: bool,
    /// The CPU interface's EL2 state, for a processor with EL2: the virtual
    /// interface and `ICC_SRE_EL2`.
    el2: Option<VirtualInterface>,
    /// The virtual IRQ and FIQ the virtual CPU interface signals,
    /// recomputed by [`GicV3::refresh_virtual`].
    virtual_lines: VirtualLines,
    /// Who the GIC says made it: `GICD_IIDR`, `GICR_IIDR` and `GITS_IIDR`
    /// read the distributor's identification.
    names: GicNames,
    shape: GicShape,
    /// The Redistributor's LPIs.
    lpis: Lpis,
    its: Its,
}

impl GicV3 {
    /// A GIC out of reset of `shape`, named `names`, for a processor without
    /// EL2.
    pub fn new(shape: GicShape, names: GicNames) -> Self {
        Self::with_el2(shape, names, None)
    }

    /// [`GicV3::new`], its CPU interface with the virtualization support of
    /// `el2` for a processor with EL2, if it is one.
    pub(crate) fn with_el2(
        shape: GicShape,
        names: GicNames,
        el2: Option<VirtualInterfaceShape>,
    ) -> Self {
        let mut edge = Bits::new();
        for sgi in 0..NR_SGIS {
            edge.set(sgi, true);
        }
        let mut gic = Self {
            dist_ctlr: 0,
            enabled: Bits::new(),
            pending: Bits::new(),
            active: Bits::new(),
            group1: Bits::new(),
            level: Bits::new(),
            edge,
            routed: Bits::new(),
            priority: [0; MAX_IRQ],
            route: [0; MAX_SPIS],
            sleep: ProcessorSleep::Asleep,
            pmr: 0,
            bpr0: shape.priority_bits.min_bpr(),
            bpr1: shape.priority_bits.min_abpr(),
            icc_ctlr: 0,
            igrpen: 0,
            ap0r: [0; MAX_APRS],
            ap1r: [0; MAX_APRS],
            hppi: None,
            irq_out: false,
            fiq_out: false,
            el2: el2.map(VirtualInterface::new),
            virtual_lines: VirtualLines::NONE,
            names,
            shape,
            lpis: Lpis::new(),
            its: Its::new(),
        };
        gic.route_all();
        gic
    }

    /// A reset: the GIC out of reset with the shape, the names and the
    /// virtualization support it has.
    pub fn reset(&mut self) {
        let el2 = self.el2.as_ref().map(VirtualInterface::shape);
        *self = Self::with_el2(self.shape, self.names, el2);
    }

    /// What firmware does as it powers the processor on: bring its
    /// Redistributor out of sleep (`GICR_WAKER.ProcessorSleep` 0, then
    /// `ChildrenAsleep` 0, IHI 0069G 12.11.40 "After powering up a PE ...
    /// software must set this bit to 0"), so that it forwards interrupts.
    pub fn power_on_processor(&mut self) {
        self.sleep = ProcessorSleep::Awake;
        self.update();
    }

    /// The SPIs and priority bits this GIC has.
    pub fn shape(&self) -> GicShape {
        self.shape
    }

    /// The IRQ output to the processor: a Group 1 interrupt signalled.
    #[inline]
    pub fn irq_out(&self) -> bool {
        self.irq_out
    }

    /// The FIQ output to the processor: a Group 0 interrupt signalled.
    #[inline]
    pub fn fiq_out(&self) -> bool {
        self.fiq_out
    }

    /// The virtual IRQ and FIQ outputs to the processor, from the virtual
    /// CPU interface; none without one.
    #[inline]
    pub fn virtual_lines(&self) -> VirtualLines {
        self.virtual_lines
    }

    /// The controls of EL2 that trap an EL1 access to the CPU interface's
    /// System registers: `ICH_HCR_EL2`'s and `ICC_SRE_EL2.Enable`; with no
    /// virtualization support, none, which no processor asks for (one
    /// without EL2 meets no such trap).
    pub(crate) fn el1_traps(&self) -> GicEl1Traps {
        match &self.el2 {
            Some(el2) => GicEl1Traps::of(el2.hcr(), el2.sre_el2()),
            None => GicEl1Traps::of(0, u64::MAX),
        }
    }

    /// An `MRS` of `reg`, the physical CPU interface's or the virtual
    /// one's, with the read's side effect (an acknowledge). A register of
    /// the virtualization support of a GIC without it reads as zero, which
    /// no processor asks for.
    pub fn register_read(&mut self, reg: GicRegister) -> u64 {
        let value = match (reg, &mut self.el2) {
            (GicRegister::Physical(icc), _) => return self.icc_read(icc),
            (GicRegister::Virtual(icv), Some(el2)) => el2.virtual_read(icv),
            (GicRegister::Control(ich), Some(el2)) => el2.control_read(ich),
            (GicRegister::SreEl2, Some(el2)) => el2.sre_el2(),
            (GicRegister::Virtual(_) | GicRegister::Control(_) | GicRegister::SreEl2, None) => {
                return 0
            }
        };
        self.refresh_virtual();
        value
    }

    /// An `MSR` of `value` to `reg`: a hardware List register's
    /// deactivation deactivates its physical interrupt. A register of the
    /// virtualization support of a GIC without it ignores it, which no
    /// processor asks for.
    pub fn register_write(&mut self, reg: GicRegister, value: u64) {
        let deactivation = match (reg, &mut self.el2) {
            (GicRegister::Physical(icc), _) => return self.icc_write(icc, value),
            (GicRegister::Virtual(icv), Some(el2)) => el2.virtual_write(icv, value),
            (GicRegister::Control(ich), Some(el2)) => {
                el2.control_write(ich, value);
                PhysicalDeactivation(None)
            }
            (GicRegister::SreEl2, Some(el2)) => {
                el2.set_sre_el2(value);
                PhysicalDeactivation(None)
            }
            (GicRegister::Virtual(_) | GicRegister::Control(_) | GicRegister::SreEl2, None) => {
                return
            }
        };
        if let PhysicalDeactivation(Some(intid)) = deactivation {
            self.deactivate(intid);
        }
        self.refresh_virtual();
    }

    /// Bring the virtual CPU interface's outputs up to date: the virtual
    /// IRQ and FIQ, and the maintenance interrupt's line, PPI 25; then the
    /// physical outputs, which that line feeds.
    fn refresh_virtual(&mut self) {
        let Some(el2) = &self.el2 else {
            return;
        };
        self.virtual_lines = el2.outputs();
        self.level
            .set(MAINTENANCE_INTID as usize, el2.maintenance());
        self.update();
    }

    /// Drive input line `intid` (a PPI or SPI that exists) to `level`.
    pub fn set_line(&mut self, intid: u32, level: bool) {
        let Some(i) = self.shape.existing(intid) else {
            return;
        };
        if i < NR_SGIS || self.level.get(i) == level {
            return;
        }
        self.level.set(i, level);
        if level && self.edge.get(i) {
            self.pending.set(i, true);
        }
        self.update();
    }

    /// Work out [`GicV3::routed`] from every SPI's route.
    fn route_all(&mut self) {
        for spi in 32..self.shape.spis.irq_end() {
            self.route_spi(spi);
        }
    }

    /// Whether SPI `spi`'s route reaches processor 0, into
    /// [`GicV3::routed`].
    fn route_spi(&mut self, spi: usize) {
        let route = self.route[spi - 32];
        let reaches = route & IROUTER_IRM != 0 || route & IROUTER_MASK == 0;
        self.routed.set(spi, reaches);
    }

    // ----- prioritization -----

    /// The enabled, pending, inactive interrupt of `groups` (group enable
    /// bits) with the numerically lowest priority, the lowest INTID among
    /// equals; an SPI only where it is routed to processor 0; an LPI, of
    /// Group 1, where it is pending and its configuration enables it.
    fn best_irq(&self, groups: u32) -> Option<usize> {
        let line = self.best_line(groups);
        if groups & GICD_CTLR_EN_GRP1 == 0 {
            return line;
        }
        let Some(lpi) = self.lpis.best(self.shape.priority_bits.mask()) else {
            return line;
        };
        match line {
            Some(irq) if self.priority[irq] <= lpi.priority => line,
            Some(_) | None => Some(lpi.intid as usize),
        }
    }

    /// [`GicV3::best_irq`] of the SGIs, PPIs and SPIs alone.
    fn best_line(&self, groups: u32) -> Option<usize> {
        let mask_of = |bit: u32| if groups & bit != 0 { u32::MAX } else { 0 };
        let (group0, group1) = (mask_of(GICD_CTLR_EN_GRP0), mask_of(GICD_CTLR_EN_GRP1));
        let mut best: Option<usize> = None;
        let mut best_prio = IDLE_PRIORITY;
        for w in 0..self.shape.words() {
            let pending = self.pending.0[w] | (self.level.0[w] & !self.edge.0[w]);
            let in_groups = (self.group1.0[w] & group1) | (!self.group1.0[w] & group0);
            let reaches = if w == 0 { u32::MAX } else { self.routed.0[w] };
            let mut candidates =
                self.enabled.0[w] & pending & !self.active.0[w] & in_groups & reaches;
            while candidates != 0 {
                let i = w * 32 + candidates.trailing_zeros() as usize;
                candidates &= candidates - 1;
                let p = u16::from(self.priority[i]);
                if p < best_prio {
                    best_prio = p;
                    best = Some(i);
                }
            }
        }
        best
    }

    /// Whether interrupt `irq` is an LPI.
    fn is_lpi(irq: usize) -> bool {
        Lpis::is_lpi(irq as u32)
    }

    /// Interrupt `irq`'s priority: an LPI's from its cached configuration.
    fn priority_of(&self, irq: usize) -> u8 {
        if Self::is_lpi(irq) {
            self.lpis
                .priority(irq as u32, self.shape.priority_bits.mask())
        } else {
            self.priority[irq]
        }
    }

    /// Whether interrupt `irq` is of Group 1, as every LPI is.
    fn is_group1(&self, irq: usize) -> bool {
        Self::is_lpi(irq) || self.group1.get(irq)
    }

    /// The mask of interrupt `irq`'s group priority bits, under its group's
    /// binary point (4.8 `GroupBits`: Group 0's `ICC_BPR0_EL1`, Group 1's
    /// `ICC_BPR1_EL1` minus one, or with `CBPR` set `ICC_BPR0_EL1`).
    fn group_mask(&self, irq: usize) -> u8 {
        group_mask(
            self.is_group1(irq),
            self.icc_ctlr & ICC_CTLR_CBPR != 0,
            self.bpr0,
            self.bpr1,
        )
    }

    /// The shift from a group priority to its preemption level, a bit of
    /// the active priority registers: the bits at and below the least
    /// binary point, which no group priority has.
    fn level_shift(&self) -> u32 {
        u32::from(self.shape.priority_bits.min_bpr()) + 1
    }

    /// The running priority (B.7 `GetHighestActivePriority`): the
    /// highest active preemption level's priority, or [`IDLE_PRIORITY`]
    /// while none is active.
    fn running_priority(&self) -> u16 {
        for n in 0..MAX_APRS {
            let active = self.ap0r[n] | self.ap1r[n];
            if active != 0 {
                return ((n as u32 * 32 + active.trailing_zeros()) << self.level_shift()) as u16;
            }
        }
        IDLE_PRIORITY
    }

    /// The group of the highest active priority, if one is active
    /// (B.7 `GetHighestActiveGroup`): Group 0's where both have it.
    fn highest_active_group1(&self) -> Option<bool> {
        for n in 0..MAX_APRS {
            let (g0, g1) = (self.ap0r[n], self.ap1r[n]);
            if g0 | g1 != 0 {
                return Some(g1.trailing_zeros() < g0.trailing_zeros());
            }
        }
        None
    }

    /// B.7.2 `CanSignalInterrupt`: whether pending interrupt `irq` is of
    /// sufficient priority to be signalled — below the priority mask, and
    /// with nothing active or a group priority higher than the running
    /// priority's under its group's binary point.
    fn can_signal(&self, irq: usize) -> bool {
        let priority = self.priority_of(irq);
        priority < self.pmr && preempts(priority, self.group_mask(irq), self.running_priority())
    }

    /// The highest priority pending interrupt of the groups both the
    /// distributor and the CPU interface enable, and the IRQ or FIQ it
    /// raises if it can be signalled: Group 1 as IRQ, Group 0 as FIQ, one
    /// Security state having no other routing (4.6.1). A sleeping
    /// Redistributor forwards nothing.
    fn update(&mut self) {
        self.irq_out = false;
        self.fiq_out = false;
        self.hppi = match self.sleep {
            ProcessorSleep::Asleep => None,
            ProcessorSleep::Awake => self.best_irq(self.dist_ctlr & self.igrpen),
        };
        let Some(best) = self.hppi else {
            return;
        };
        if self.can_signal(best) {
            if self.is_group1(best) {
                self.irq_out = true;
            } else {
                self.fiq_out = true;
            }
        }
    }

    /// Acknowledge interrupt `irq`, its group's active priority register
    /// recording its group priority's preemption level: an SGI, PPI or SPI
    /// becomes active, an LPI is no longer pending.
    fn activate(&mut self, irq: usize) {
        let level = usize::from(self.priority_of(irq) & self.group_mask(irq)) >> self.level_shift();
        let bank = if self.is_group1(irq) {
            &mut self.ap1r
        } else {
            &mut self.ap0r
        };
        bank[level / 32] |= 1 << (level % 32);
        if Self::is_lpi(irq) {
            self.lpis.clear_pending(irq as u32);
        } else {
            self.active.set(irq, true);
            self.pending.set(irq, false);
        }
    }

    /// B.7.2 `PriorityDrop`: clear the highest active priority of the
    /// group.
    fn drop_priority(&mut self, group1: bool) {
        let bank = if group1 {
            &mut self.ap1r
        } else {
            &mut self.ap0r
        };
        if let Some(word) = bank.iter_mut().find(|w| **w != 0) {
            *word &= *word - 1;
        }
    }

    /// Deactivate interrupt `intid`, if it exists.
    fn deactivate(&mut self, intid: u32) {
        if let Some(i) = self.shape.existing(intid) {
            self.active.set(i, false);
        }
    }

    fn eoi_split(&self) -> bool {
        self.icc_ctlr & ICC_CTLR_EOIMODE != 0
    }

    // ----- the CPU interface -----

    /// An `MRS` of `reg`, with the read's side effect: an `ICC_IAR0_EL1`
    /// or `ICC_IAR1_EL1` acknowledges the interrupt it returns. A
    /// write-only register reads as zero, which no processor asks for (its
    /// `MRS` is UNDEFINED).
    pub fn icc_read(&mut self, reg: IccRegister) -> u64 {
        u64::from(match reg {
            IccRegister::Pmr => u32::from(self.pmr),
            IccRegister::Iar0 => self.acknowledge(false),
            IccRegister::Iar1 => self.acknowledge(true),
            IccRegister::Hppir0 => self.highest_pending(false),
            IccRegister::Hppir1 => self.highest_pending(true),
            IccRegister::Bpr0 => u32::from(self.bpr0),
            IccRegister::Bpr1 => u32::from(if self.icc_ctlr & ICC_CTLR_CBPR != 0 {
                (self.bpr0 + 1).min(7)
            } else {
                self.bpr1
            }),
            IccRegister::Ap0r(n) => self.ap0r[n.get()],
            IccRegister::Ap1r(n) => self.ap1r[n.get()],
            // `ICC_RPR_EL1`: 0xFF while nothing is active.
            IccRegister::Rpr => u32::from(self.running_priority().min(0xFF)),
            IccRegister::Ctlr => self.icc_ctlr_value(),
            IccRegister::Sre => ICC_SRE as u32,
            IccRegister::Igrpen0 => u32::from(self.igrpen & GICD_CTLR_EN_GRP0 != 0),
            IccRegister::Igrpen1 => u32::from(self.igrpen & GICD_CTLR_EN_GRP1 != 0),
            IccRegister::Eoir0
            | IccRegister::Eoir1
            | IccRegister::Dir
            | IccRegister::Sgi0r
            | IccRegister::Sgi1r
            | IccRegister::Asgi1r => 0,
        })
    }

    /// `ICC_CTLR_EL1`: `CBPR` and `EOImode` as written, `PRIbits` the
    /// priority bits minus one, and every other field 0 — 16 INTID bits,
    /// no SEIs, no affinity level 3, no range selectors, no extended
    /// INTIDs, `PMHE` RAZ/WI.
    fn icc_ctlr_value(&self) -> u32 {
        self.icc_ctlr | (u32::from(self.shape.priority_bits.count()) - 1) << 8
    }

    /// An `MSR` of `value` to `reg`. A read-only register ignores it,
    /// which no processor asks for (its `MSR` is UNDEFINED).
    pub fn icc_write(&mut self, reg: IccRegister, value: u64) {
        let bits = self.shape.priority_bits;
        let low = value as u32;
        match reg {
            IccRegister::Pmr => self.pmr = low as u8 & bits.mask(),
            IccRegister::Bpr0 => self.bpr0 = (low as u8 & 7).max(bits.min_bpr()),
            IccRegister::Bpr1 => {
                if self.icc_ctlr & ICC_CTLR_CBPR == 0 {
                    self.bpr1 = (low as u8 & 7).max(bits.min_abpr());
                }
            }
            IccRegister::Ap0r(n) => self.ap0r[n.get()] = low & bits.apr_bits(n.get()),
            IccRegister::Ap1r(n) => self.ap1r[n.get()] = low & bits.apr_bits(n.get()),
            IccRegister::Ctlr => self.icc_ctlr = low & (ICC_CTLR_CBPR | ICC_CTLR_EOIMODE),
            IccRegister::Igrpen0 => self.set_group_enable(GICD_CTLR_EN_GRP0, low),
            IccRegister::Igrpen1 => self.set_group_enable(GICD_CTLR_EN_GRP1, low),
            IccRegister::Eoir0 => self.end_of_interrupt(false, low),
            IccRegister::Eoir1 => self.end_of_interrupt(true, low),
            IccRegister::Dir => {
                if self.eoi_split() {
                    self.deactivate(low & 0xFF_FFFF);
                }
            }
            IccRegister::Sgi0r => self.generate_sgi(value, SgiRegister::Group0),
            IccRegister::Sgi1r => self.generate_sgi(value, SgiRegister::Group1),
            IccRegister::Asgi1r => self.generate_sgi(value, SgiRegister::AliasGroup1),
            IccRegister::Iar0
            | IccRegister::Iar1
            | IccRegister::Hppir0
            | IccRegister::Hppir1
            | IccRegister::Rpr
            | IccRegister::Sre => {}
        }
        self.update();
    }

    fn set_group_enable(&mut self, group: u32, value: u32) {
        if value & 1 != 0 {
            self.igrpen |= group;
        } else {
            self.igrpen &= !group;
        }
    }

    /// `ICC_IAR0_EL1` (`group1` false) or `ICC_IAR1_EL1`: the highest
    /// priority pending interrupt, acknowledged, if it is of the register's
    /// group and of sufficient priority to be signalled; 1023 otherwise
    /// (2.2.1).
    fn acknowledge(&mut self, group1: bool) -> u32 {
        match self.hppi {
            Some(irq) if self.is_group1(irq) == group1 && self.can_signal(irq) => {
                self.activate(irq);
                self.update();
                irq as u32
            }
            Some(_) | None => SPURIOUS,
        }
    }

    /// `ICC_HPPIR0_EL1` or `ICC_HPPIR1_EL1` (B.7.2
    /// `HighestPriorityPendingInterrupt`): the highest priority pending
    /// interrupt if it is of the register's group, whatever its priority;
    /// 1023 otherwise.
    fn highest_pending(&self, group1: bool) -> u32 {
        match self.hppi {
            Some(irq) if self.is_group1(irq) == group1 => irq as u32,
            Some(_) | None => SPURIOUS,
        }
    }

    /// `ICC_EOIR0_EL1` or `ICC_EOIR1_EL1` naming `value`'s INTID: a
    /// priority drop of the group's highest active priority, where it is
    /// the highest active priority, and unless `EOImode` splits the two, a
    /// deactivation, which an LPI, having no active state, does not need. A
    /// special INTID needs no end of interrupt (2.2.1).
    fn end_of_interrupt(&mut self, group1: bool, value: u32) {
        let intid = value & 0xFF_FFFF;
        if SPECIAL_INTIDS.contains(&intid) || self.highest_active_group1() != Some(group1) {
            return;
        }
        self.drop_priority(group1);
        if !self.eoi_split() {
            self.deactivate(intid);
        }
    }

    /// An SGI generation register's write, from processor 0 (12.2.3,
    /// 12.2.20, 12.2.21): the SGI `INTID` [27:24] made pending at processor
    /// 0 where the write names it — `IRM` [40] clear, `Aff3` [55:48],
    /// `Aff2` [39:32], `Aff1` [23:16] and `RS` [47:44] zero, and bit 0 of
    /// `TargetList` set — and the register forwards an SGI of its group
    /// (Table 12-14, a Non-secure access with `GICD_CTLR.DS` 1:
    /// `ICC_SGI1R_EL1` either group's, `ICC_SGI0R_EL1` and
    /// `ICC_ASGI1R_EL1` Group 0's). `IRM` set names every processor but
    /// this one, which are none.
    fn generate_sgi(&mut self, value: u64, register: SgiRegister) {
        let sgi = ((value >> 24) & 0xF) as usize;
        let irm = value & (1 << 40) != 0;
        let elsewhere = value & 0x00FF_F0FF_00FF_0000 != 0;
        if irm || elsewhere || value & 1 == 0 {
            return;
        }
        let forwarded = match register {
            SgiRegister::Group1 => true,
            SgiRegister::Group0 | SgiRegister::AliasGroup1 => !self.group1.get(sgi),
        };
        if forwarded {
            self.pending.set(sgi, true);
        }
    }

    // ----- the distributor -----

    pub fn dist_read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        let off = offset.get();
        match width {
            AccessWidth::Byte => u64::from(self.dist_priority_byte(off)),
            AccessWidth::Half => 0,
            AccessWidth::Word => u64::from(self.dist_word(off)),
            AccessWidth::Dword => {
                u64::from(self.dist_word(off)) | u64::from(self.dist_word(off + 4)) << 32
            }
        }
    }

    pub fn dist_write(&mut self, offset: RegOffset, width: AccessWidth, value: u64) {
        let off = offset.get();
        match width {
            AccessWidth::Byte => self.set_dist_priority_byte(off, value as u8),
            AccessWidth::Half => {}
            AccessWidth::Word => self.dist_write_word(off, value as u32),
            AccessWidth::Dword => {
                self.dist_write_word(off, value as u32);
                self.dist_write_word(off + 4, (value >> 32) as u32);
            }
        }
        self.update();
    }

    /// Word `n` of a bank of one bit per interrupt, in the distributor:
    /// the SPIs' words alone, word 0 (the SGIs' and PPIs') RAZ/WI under
    /// affinity routing.
    fn spi_word(&self, n: usize) -> Option<usize> {
        (n != 0 && n < self.shape.words()).then_some(n)
    }

    /// `GICD_TYPER`: `ITLinesNumber` from the SPIs, fourteen INTID bits,
    /// LPIs, and every other field 0 (one Security state, the LPIs the INTID
    /// bits number, no message-based SPIs, 1 of N supported, no affinity
    /// level 3, no range selectors).
    fn dist_typer(&self) -> u32 {
        GICD_TYPER_IDBITS | GICD_TYPER_LPIS | (self.shape.words() as u32 - 1)
    }

    fn dist_word(&self, off: u64) -> u32 {
        if !off.is_multiple_of(4) {
            return 0;
        }
        let bank = |base: u64| ((off - base) / 4) as usize;
        match off {
            0x0000 => self.dist_ctlr | GICD_CTLR_ARE | GICD_CTLR_DS,
            0x0004 => self.dist_typer(),
            0x0008 => self.names.gic.distributor.get(),
            0x0080..=0x00FC => self.dist_bank(&self.group1, bank(IGROUPR)),
            0x0100..=0x01FC => self.dist_bank(&self.enabled, bank(ISENABLER) % 32),
            0x0200..=0x02FC => self
                .spi_word(bank(ISPENDR) % 32)
                .map_or(0, |n| self.pending_word(n)),
            0x0300..=0x03FC => self.dist_bank(&self.active, bank(ISACTIVER) % 32),
            0x0400..=0x07FC => (0..4).fold(0, |v, b| {
                v | u32::from(self.dist_priority_byte(off + b)) << (8 * b)
            }),
            0x0C00..=0x0CFC => self.config_word(bank(ICFGR), Frame::Distributor),
            0x6000..=0x7FFC => self.route_half(off - IROUTER),
            ID_REGISTERS..=0xFFFC => {
                identification(self.names, off - ID_REGISTERS, DIST_SIZE_LOG2_BLOCKS)
            }
            _ => 0,
        }
    }

    /// Word `n` of `bits` as the distributor reads it.
    fn dist_bank(&self, bits: &Bits, n: usize) -> u32 {
        self.spi_word(n).map_or(0, |n| bits.0[n])
    }

    /// The pending state of word `n`'s interrupts: latched, or a
    /// level-sensitive line high.
    fn pending_word(&self, n: usize) -> u32 {
        self.pending.0[n] | (self.level.0[n] & !self.edge.0[n])
    }

    /// `GICD_ICFGR<n>` or `GICR_ICFGR0`/`GICR_ICFGR1`: two bits per
    /// interrupt, `Int_config[1]` set for an edge-triggered one; the
    /// distributor's of the SGIs and PPIs RAZ/WI.
    fn config_word(&self, n: usize, frame: Frame) -> u32 {
        let first = n * 16;
        let in_frame = match frame {
            Frame::Distributor => first >= 32,
            Frame::Redistributor => first < 32,
        };
        if !in_frame {
            return 0;
        }
        (0..16).fold(0, |v, k| {
            let i = first + k;
            if i < MAX_IRQ && self.edge.get(i) {
                v | 2 << (2 * k)
            } else {
                v
            }
        })
    }

    /// Half `half_offset / 4` of a `GICD_IROUTER<n>`, at `half_offset`
    /// bytes from `GICD_IROUTER0`; the registers of INTIDs 0 to 31 are
    /// reserved, and an SPI that does not exist has none.
    fn route_half(&self, half_offset: u64) -> u32 {
        match self.route_index(half_offset) {
            Some(index) => (self.route[index] >> (32 * ((half_offset / 4) % 2))) as u32,
            None => 0,
        }
    }

    /// The index into [`GicV3::route`] of the `GICD_IROUTER<n>` at
    /// `half_offset` from `GICD_IROUTER0`, if its SPI exists.
    fn route_index(&self, half_offset: u64) -> Option<usize> {
        let intid = (half_offset / 8) as usize;
        (intid >= 32 && self.shape.implemented(intid)).then_some(intid - 32)
    }

    /// Byte `off` of the distributor's `GICD_IPRIORITYR<n>`: an SPI's
    /// priority; those of INTIDs 0 to 31 RAZ/WI under affinity routing.
    fn dist_priority_byte(&self, off: u64) -> u8 {
        match off {
            0x0420..=0x07FF => self
                .priority
                .get((off - IPRIORITYR) as usize)
                .copied()
                .unwrap_or(0),
            _ => 0,
        }
    }

    fn set_dist_priority_byte(&mut self, off: u64, value: u8) {
        if let 0x0420..=0x07FF = off {
            self.set_priority((off - IPRIORITYR) as usize, value);
        }
    }

    /// Interrupt `irq`'s priority, if it exists: the implemented bits of
    /// `value`.
    fn set_priority(&mut self, irq: usize, value: u8) {
        if self.shape.implemented(irq) {
            self.priority[irq] = value & self.shape.priority_bits.mask();
        }
    }

    fn dist_write_word(&mut self, off: u64, v: u32) {
        if !off.is_multiple_of(4) {
            return;
        }
        let bank = |base: u64| ((off - base) / 4) as usize;
        match off {
            0x0000 => self.dist_ctlr = v & (GICD_CTLR_EN_GRP0 | GICD_CTLR_EN_GRP1),
            0x0080..=0x00FC => {
                if let Some(n) = self.spi_word(bank(IGROUPR)) {
                    self.group1.0[n] = v & self.shape.implemented_word(n);
                }
            }
            0x0100..=0x01FC => {
                if let Some(n) = self.spi_word(bank(ISENABLER) % 32) {
                    set_or_clear(
                        &mut self.enabled,
                        n,
                        v & self.shape.implemented_word(n),
                        off < ICENABLER,
                    );
                }
            }
            0x0200..=0x02FC => {
                if let Some(n) = self.spi_word(bank(ISPENDR) % 32) {
                    set_or_clear(
                        &mut self.pending,
                        n,
                        v & self.shape.implemented_word(n),
                        off < ICPENDR,
                    );
                }
            }
            0x0300..=0x03FC => {
                if let Some(n) = self.spi_word(bank(ISACTIVER) % 32) {
                    set_or_clear(
                        &mut self.active,
                        n,
                        v & self.shape.implemented_word(n),
                        off < ICACTIVER,
                    );
                }
            }
            0x0400..=0x07FC => {
                for b in 0..4 {
                    self.set_dist_priority_byte(off + b, (v >> (8 * b)) as u8);
                }
            }
            0x0C00..=0x0CFC => {
                let first = bank(ICFGR) * 16;
                if first >= 32 {
                    for k in 0..16 {
                        if self.shape.implemented(first + k) {
                            self.edge.set(first + k, v & (2 << (2 * k)) != 0);
                        }
                    }
                }
            }
            0x6000..=0x7FFC => {
                let half_offset = off - IROUTER;
                if let Some(index) = self.route_index(half_offset) {
                    let shift = 32 * ((half_offset / 4) % 2);
                    let route = self.route[index] & !(0xFFFF_FFFF << shift);
                    self.route[index] = (route | u64::from(v) << shift) & IROUTER_MASK;
                    self.route_spi(index + 32);
                }
            }
            // The group enables' other bits, the SGI and target registers
            // under affinity routing, the group modifiers and Non-secure
            // access controls of one Security state, and the rest: RAZ/WI.
            _ => {}
        }
    }

    // ----- the Redistributor -----

    pub fn redist_read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        let off = offset.get();
        match width {
            AccessWidth::Byte => u64::from(self.redist_priority_byte(off)),
            AccessWidth::Half => 0,
            AccessWidth::Word => u64::from(self.redist_word(off)),
            AccessWidth::Dword => {
                u64::from(self.redist_word(off)) | u64::from(self.redist_word(off + 4)) << 32
            }
        }
    }

    /// A write of the Redistributor's frames, which reach guest `ram` for
    /// its LPI tables.
    pub(crate) fn redist_write(
        &mut self,
        ram: &mut GuestRam<'_>,
        offset: RegOffset,
        width: AccessWidth,
        value: u64,
    ) {
        let off = offset.get();
        match width {
            AccessWidth::Byte => self.set_redist_priority_byte(off, value as u8),
            AccessWidth::Half => {}
            AccessWidth::Word => self.redist_write_word(ram, off, value as u32),
            AccessWidth::Dword => {
                self.redist_write_word(ram, off, value as u32);
                self.redist_write_word(ram, off + 4, (value >> 32) as u32);
            }
        }
        self.update();
    }

    fn redist_word(&self, off: u64) -> u32 {
        if !off.is_multiple_of(4) {
            return 0;
        }
        let word0 = |bits: &Bits| bits.0[0];
        match off {
            GICR_CTLR => self.lpis.ctlr(),
            // `GICR_IIDR`.
            0x0004 => self.names.gic.distributor.get(),
            // `GICR_TYPER`: the last Redistributor, of processor 0 (its
            // `Processor_Number`), at affinity 0.0.0.0 (the high word), with
            // physical LPIs, its LPI Configuration table shared with every
            // other Redistributor (`CommonLPIAff` 0), and no direct LPIs.
            0x0008 => GICR_TYPER_LAST | GICR_TYPER_PLPIS,
            o if o & !7 == GICR_PROPBASER || o & !7 == GICR_PENDBASER => {
                self.lpis.base_word(o & !7, (o >> 2) & 1)
            }
            0x0014 => match self.sleep {
                ProcessorSleep::Awake => 0,
                ProcessorSleep::Asleep => GICR_WAKER_PROCESSOR_SLEEP | GICR_WAKER_CHILDREN_ASLEEP,
            },
            ID_REGISTERS..=0xFFFC => {
                identification(self.names, off - ID_REGISTERS, REDIST_SIZE_LOG2_BLOCKS)
            }
            o if o == SGI_BASE + IGROUPR => word0(&self.group1),
            o if o == SGI_BASE + ISENABLER || o == SGI_BASE + ICENABLER => word0(&self.enabled),
            o if o == SGI_BASE + ISPENDR || o == SGI_BASE + ICPENDR => self.pending_word(0),
            o if o == SGI_BASE + ISACTIVER || o == SGI_BASE + ICACTIVER => word0(&self.active),
            o if (SGI_BASE + IPRIORITYR..SGI_BASE + IPRIORITYR + 32).contains(&o) => (0..4)
                .fold(0, |v, b| {
                    v | u32::from(self.redist_priority_byte(off + b)) << (8 * b)
                }),
            o if o == SGI_BASE + ICFGR => self.config_word(0, Frame::Redistributor),
            o if o == SGI_BASE + ICFGR + 4 => self.config_word(1, Frame::Redistributor),
            // `GICR_STATUSR`, the direct LPI registers, the group modifiers
            // and the Non-secure access control: RAZ/WI.
            _ => 0,
        }
    }

    /// Byte `off` of the Redistributor's frames: an SGI's or PPI's priority
    /// in `GICR_IPRIORITYR<n>`.
    fn redist_priority_byte(&self, off: u64) -> u8 {
        match off.checked_sub(SGI_BASE + IPRIORITYR) {
            Some(irq) if irq < 32 => self.priority[irq as usize],
            Some(_) | None => 0,
        }
    }

    fn set_redist_priority_byte(&mut self, off: u64, value: u8) {
        if let Some(irq) = off
            .checked_sub(SGI_BASE + IPRIORITYR)
            .filter(|irq| *irq < 32)
        {
            self.set_priority(irq as usize, value);
        }
    }

    fn redist_write_word(&mut self, ram: &mut GuestRam<'_>, off: u64, v: u32) {
        if !off.is_multiple_of(4) {
            return;
        }
        // The SGIs' and PPIs' bits that exist, of a bank's word.
        let ids = v & self.shape.implemented_word(0);
        match off {
            GICR_CTLR => self.lpis.write_ctlr(ram, v),
            o if o & !7 == GICR_PROPBASER || o & !7 == GICR_PENDBASER => {
                self.lpis.write_base_word(o & !7, (o >> 2) & 1, v);
            }
            0x0014 => {
                self.sleep = if v & GICR_WAKER_PROCESSOR_SLEEP != 0 {
                    ProcessorSleep::Asleep
                } else {
                    ProcessorSleep::Awake
                };
            }
            o if o == SGI_BASE + IGROUPR => self.group1.0[0] = ids,
            o if o == SGI_BASE + ISENABLER => set_or_clear(&mut self.enabled, 0, ids, true),
            o if o == SGI_BASE + ICENABLER => set_or_clear(&mut self.enabled, 0, ids, false),
            o if o == SGI_BASE + ISPENDR => set_or_clear(&mut self.pending, 0, ids, true),
            o if o == SGI_BASE + ICPENDR => set_or_clear(&mut self.pending, 0, ids, false),
            o if o == SGI_BASE + ISACTIVER => set_or_clear(&mut self.active, 0, ids, true),
            o if o == SGI_BASE + ICACTIVER => set_or_clear(&mut self.active, 0, ids, false),
            o if (SGI_BASE + IPRIORITYR..SGI_BASE + IPRIORITYR + 32).contains(&o) => {
                for b in 0..4 {
                    self.set_redist_priority_byte(off + b, (v >> (8 * b)) as u8);
                }
            }
            // `GICR_ICFGR0` (every SGI edge-triggered) and `GICR_ICFGR1`
            // (every PPI level-sensitive) are read-only; the rest RAZ/WI.
            _ => {}
        }
    }

    // ----- the ITS -----

    /// A read of the ITS's frames.
    pub fn its_read(&self, offset: RegOffset, width: AccessWidth) -> u64 {
        self.its.read(self.names, offset, width)
    }

    /// A write of the ITS's frames: the commands it starts and the
    /// translation it asks for, against the Redistributor and guest `ram`,
    /// and the interrupts they make pending signalled.
    pub(crate) fn its_write(
        &mut self,
        ram: &mut GuestRam<'_>,
        offset: RegOffset,
        width: AccessWidth,
        value: u64,
    ) {
        self.its.write(&mut self.lpis, ram, offset, width, value);
        self.update();
    }

    // ----- snapshots -----

    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_u32(self.dist_ctlr)?;
        w.write_u8(match self.sleep {
            ProcessorSleep::Awake => 0,
            ProcessorSleep::Asleep => 1,
        })?;
        w.write_bytes(&[self.pmr, self.bpr0, self.bpr1])?;
        w.write_u32(self.icc_ctlr)?;
        w.write_u32(self.igrpen)?;
        for word in self.ap0r.iter().chain(self.ap1r.iter()) {
            w.write_u32(*word)?;
        }
        for bits in self.banks() {
            for word in bits.0 {
                w.write_u32(word)?;
            }
        }
        w.write_bytes(&self.priority)?;
        for route in self.route {
            w.write_u64(route)?;
        }
        self.lpis.save(w)?;
        self.its.save(w)?;
        match &self.el2 {
            Some(el2) => el2.save(w),
            None => Ok(()),
        }
    }

    /// The banks a snapshot holds, in its order.
    fn banks(&self) -> [&Bits; 6] {
        [
            &self.enabled,
            &self.pending,
            &self.active,
            &self.group1,
            &self.level,
            &self.edge,
        ]
    }

    /// Reads the GIC's whole section of the image, refuses it if
    /// [`GicV3::check_restored`], [`Lpis::restore`] or [`Its::restore`]
    /// does, and only then replaces this GIC's state: a refused or
    /// truncated section changes nothing here.
    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let dist_ctlr = r.read_u32()?;
        let sleep = match r.read_u8()? {
            0 => ProcessorSleep::Awake,
            1 => ProcessorSleep::Asleep,
            _ => {
                return Err(SnapError::Invalid(
                    "GICv3 Redistributor sleep state out of range",
                ))
            }
        };
        let mut points = [0u8; 3];
        r.read_bytes(&mut points)?;
        let [pmr, bpr0, bpr1] = points;
        let icc_ctlr = r.read_u32()?;
        let igrpen = r.read_u32()?;
        let mut ap0r = [0u32; MAX_APRS];
        let mut ap1r = [0u32; MAX_APRS];
        for word in ap0r.iter_mut().chain(ap1r.iter_mut()) {
            *word = r.read_u32()?;
        }
        let mut sets = [Bits::new(); 6];
        for bits in sets.iter_mut() {
            for word in bits.0.iter_mut() {
                *word = r.read_u32()?;
            }
        }
        let [enabled, pending, active, group1, level, edge] = sets;
        let mut priority = [0u8; MAX_IRQ];
        r.read_bytes(&mut priority)?;
        let mut route = [0u64; MAX_SPIS];
        for entry in route.iter_mut() {
            *entry = r.read_u64()?;
        }
        let lpis = Lpis::restore(r)?;
        let its = Its::restore(r)?;
        let el2 = match &self.el2 {
            Some(el2) => Some(VirtualInterface::restore(el2.shape(), r)?),
            None => None,
        };
        let mut restored = Self {
            dist_ctlr,
            enabled,
            pending,
            active,
            group1,
            level,
            edge,
            routed: Bits::new(),
            priority,
            route,
            sleep,
            pmr,
            bpr0,
            bpr1,
            icc_ctlr,
            igrpen,
            ap0r,
            ap1r,
            hppi: None,
            irq_out: false,
            fiq_out: false,
            el2,
            virtual_lines: VirtualLines::NONE,
            names: self.names,
            shape: self.shape,
            lpis,
            its,
        };
        restored.check_restored()?;
        restored.route_all();
        restored.update();
        restored.refresh_virtual();
        *self = restored;
        Ok(())
    }

    /// The one check of a restored image, made before it replaces
    /// anything: refuses a control register with a bit no field holds,
    /// binary points the registers cannot hold, a priority or a priority
    /// mask with an unimplemented bit set, an active priority of a
    /// preemption level the priority bits do not give, state or a route of
    /// an interrupt that does not exist, a route with a bit no field holds,
    /// an SGI that is not edge-triggered or has a line, a PPI that is not
    /// level-sensitive, and a maintenance interrupt line other than what
    /// the virtual interface asserts. The virtual interface's own registers
    /// were checked as they were read ([`VirtualInterface::restore`]).
    fn check_restored(&self) -> SnapResult {
        let groups = GICD_CTLR_EN_GRP0 | GICD_CTLR_EN_GRP1;
        if self.dist_ctlr & !groups != 0
            || self.igrpen & !groups != 0
            || self.icc_ctlr & !(ICC_CTLR_CBPR | ICC_CTLR_EOIMODE) != 0
        {
            return Err(SnapError::Invalid(
                "GICv3 control register with a bit no field holds",
            ));
        }
        let shape = self.shape;
        let bits = shape.priority_bits;
        if !(bits.min_bpr()..=7).contains(&self.bpr0) || !(bits.min_abpr()..=7).contains(&self.bpr1)
        {
            return Err(SnapError::Invalid(
                "GICv3 binary point outside its register's range",
            ));
        }
        let mask = bits.mask();
        if self.pmr & !mask != 0 || self.priority.iter().any(|p| p & !mask != 0) {
            return Err(SnapError::Invalid(
                "GICv3 priority with an unimplemented bit",
            ));
        }
        if (0..MAX_APRS).any(|n| (self.ap0r[n] | self.ap1r[n]) & !bits.apr_bits(n) != 0) {
            return Err(SnapError::Invalid(
                "GICv3 active priority of a preemption level that does not exist",
            ));
        }
        let absent = |bits: &Bits| (0..WORDS).any(|w| bits.0[w] & !shape.implemented_word(w) != 0);
        let spi_absent = |index: usize| !shape.implemented(index + 32);
        if self.banks().into_iter().any(absent)
            || (0..MAX_IRQ).any(|i| !shape.implemented(i) && self.priority[i] != 0)
            || (0..MAX_SPIS).any(|n| spi_absent(n) && self.route[n] != 0)
        {
            return Err(SnapError::Invalid(
                "GICv3 state of an interrupt that does not exist",
            ));
        }
        if self.route.iter().any(|route| route & !IROUTER_MASK != 0) {
            return Err(SnapError::Invalid("GICv3 route with a bit no field holds"));
        }
        if self.edge.0[0] != SGI_BITS || self.level.0[0] & SGI_BITS != 0 {
            return Err(SnapError::Invalid(
                "GICv3 SGI or PPI configuration its registers cannot hold",
            ));
        }
        let maintenance = self.el2.as_ref().is_some_and(VirtualInterface::maintenance);
        if self.level.get(MAINTENANCE_INTID as usize) != maintenance {
            return Err(SnapError::Invalid(
                "GICv3 maintenance interrupt line disagrees with ICH_MISR_EL2",
            ));
        }
        Ok(())
    }
}

/// The three registers that generate an SGI.
///
/// Closed (R5).
#[derive(Clone, Copy)]
enum SgiRegister {
    /// `ICC_SGI0R_EL1`.
    Group0,
    /// `ICC_SGI1R_EL1`.
    Group1,
    /// `ICC_ASGI1R_EL1`.
    AliasGroup1,
}

/// The frame a configuration register is read in.
///
/// Closed (R5).
#[derive(Clone, Copy)]
enum Frame {
    Distributor,
    Redistributor,
}

/// Set (`set`) or clear the bits `v` of word `n` of `bits`.
fn set_or_clear(bits: &mut Bits, n: usize, v: u32, set: bool) {
    if set {
        bits.0[n] |= v;
    } else {
        bits.0[n] &= !v;
    }
}

/// The identification register at `index` bytes from 0xFFD0 of a frame
/// that occupies `2^size_log2` 4 KiB blocks, in the CoreLink and CoreSight
/// scheme (IHI 0069G 12.1.13): `PIDR4` at 0xFFD0 (the designer's JEP106
/// continuation code and `SIZE`), `PIDR5` to `PIDR7` RES0, `PIDR0` (the
/// part number's low byte) and `PIDR1` (its high nibble, 0, and the
/// identity code's low nibble), `PIDR2` (the identity code's high bits, the
/// JEDEC bit where the designer has a JEP106 code, and `ArchRev`), `PIDR3`
/// 0, then `CIDR0` to `CIDR3`. The designer and part are `GICD_IIDR`'s
/// implementer and product.
pub(super) fn identification(names: GicNames, index: u64, size_log2: u32) -> u32 {
    let iidr = names.gic.distributor.fields();
    let identity = u32::from(iidr.jep106_identity);
    let continuation = u32::from(iidr.jep106_continuation);
    let jedec = u32::from(identity != 0 || continuation != 0);
    match index / 4 {
        0 => continuation | size_log2 << 4,
        4 => u32::from(iidr.product),
        5 => (identity & 0xF) << 4,
        6 => identity >> 4 | jedec << 3 | ARCH_REV_GICV3 << 4,
        i @ 8..=11 => COMPONENT_ID[i as usize - 8],
        _ => 0,
    }
}

#[cfg(test)]
#[path = "../../tests/unit/gicv3.rs"]
mod tests;
