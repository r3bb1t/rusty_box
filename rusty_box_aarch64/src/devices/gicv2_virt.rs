//! The GIC Virtualization Extensions of the board's GICv2, for its one
//! processor: the virtual interface control registers (`GICH_*`) a
//! hypervisor programs, and the virtual CPU interface (`GICV_*`) its guest
//! acknowledges and completes virtual interrupts through.
//!
//! Provenance (R7): IHI 0048B.b chapter 5 first; the CoreLink GIC-400 TRM
//! (DDI 0471B, a GICv2 with the extensions) for what IHI 0048B leaves to the
//! implementation, `GICH_VTR` 0x90000003: four List registers, five
//! priority and five preemption bits; QEMU 11.1.2 `hw/intc/arm_gic.c` and
//! `gic_internal.h` (`gic_get_best_virq`, `gic_complete_irq` and
//! `gic_deactivate_irq` for a vCPU, `gic_hyp_read`/`gic_hyp_write`,
//! `gic_update_virt`) for the rest.
//!
//! The highest-priority pending List register is chosen as the physical CPU
//! interface chooses its interrupt (`super::gicv2`'s module docs, IHI
//! 0048B §3.7.2): among the List registers in the pending state, not those
//! active and pending (§5.5.7: GICV_HPPIR "never returns the Interrupt ID of
//! an interrupt that is active and pending"), whatever `GICV_CTLR`
//! enables. The enables decide whether it is signalled and acknowledged
//! (§5.4.1), so a higher-priority interrupt of a disabled group hides the
//! other group's, but in §4.1.5's one case: with `AckCtl` set, Group 0
//! disabled and Group 1 enabled, Group 0 is ignored. `GICV_HPPIR` and
//! `GICV_AHPPIR` report the highest pending interrupt past the priority
//! mask whatever the enables, as the GIC-400's do (DDI 0471B Table 3-10,
//! note b). With `GICH_HCR.En` clear the interface does not operate
//! (Table 5-2) and nothing is chosen.
//!
//! Where IHI 0048B leaves a choice, this model reads it so:
//! - Among equal priorities the lowest-numbered List register wins (QEMU
//!   `gic_get_best_virq`).
//! - Every EOI drops the running priority, through `GICV_EOIR` or
//!   `GICV_AEOIR` alike: which register suits the highest-priority active
//!   interrupt's group is a rule whose breach is UNPREDICTABLE (§5.2.3,
//!   Tables 5-13 and 5-16), and QEMU completes either way.
//! - A deactivation, by EOI with `EOImode` 0 or by `GICV_DIR`, whose ID no
//!   valid List register holds counts in `GICH_HCR.EOICount`: an EOI only
//!   when it dropped a priority (Table 5-2: "EOIs that do not clear a bit in
//!   the Active Priorities register, GICH_APR do not cause an increment"),
//!   `GICV_DIR` always (§5.5.14: "If the specified Interrupt does not exist
//!   in the List registers, the GICH_HCR.EOIcount field is incremented").
//!   One whose List register is only pending counts nothing and changes
//!   nothing, the effect §5.5.14 leaves UNPREDICTABLE.
//! - `GICV_AHPPIR`'s "No pending interrupts of sufficient priority to be
//!   signaled" (Table 5-17) is read as "none past the priority mask", the
//!   condition `GICV_HPPIR` reports under (Table 5-14 sets none of its
//!   own), so the two report the same interrupt; a running priority it
//!   cannot preempt does not hide it, as in QEMU's `current_pending`.
//! - A hardware interrupt's deactivation reaches the distributor whatever
//!   the physical interrupt's group. §5.5.5 likens it to a Non-secure
//!   `GICC_DIR` write, ignored for Group 0, but §5.1's note says the
//!   chapter describes only a GIC with the Security Extensions, "for which
//!   Group 0 physical interrupts are Secure interrupts". This GIC has no
//!   Security Extensions, so Group 0 is not Secure and its CPU interface
//!   controls both groups (§4.4.1), as in QEMU, whose `gic_cpu_ns_access` is
//!   false without them. A `PhysicalID` naming an SGI is UNPREDICTABLE
//!   (Table 5-9) and deactivates nothing.
//! - `GICV_RPR` reads 0xFF while nothing is active (§5.5.6 permits 0xFF or
//!   0xF8); `GICV_APR0` is an alias of `GICH_APR` and the other `GICV_APRn`
//!   are RAZ/WI, as §5.5.12 suggests.

use rusty_box_core::SnapError;
use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};

// GICV_CTLR has GICC_CTLR's bits (Table 5-11), and GICH_VMCR aliases them
// (Table 5-4); the virtual CPU interface's two register sets are the
// physical one's (Table 5-10): `GICV_IAR` and `GICV_HPPIR` for Group 0, and
// for Group 1 while `AckCtl` is set, `GICV_AIAR` and `GICV_AHPPIR` for
// Group 1 only.
use super::gicv2::{
    CpuView as View, GICC_CTLR_ACK_CTL as CTLR_ACK_CTL, GICC_CTLR_CBPR as CTLR_CBPR,
    GICC_CTLR_EOIMODE as CTLR_EOIMODE, GICC_CTLR_FIQ_EN as CTLR_FIQ_EN,
    GICC_CTLR_V2_MASK as CTLR_MASK, GICD_CTLR_EN_GRP0 as CTLR_EN_GRP0,
    GICD_CTLR_EN_GRP1 as CTLR_EN_GRP1, GicV2, IDLE_PRIORITY, MAXIRQ, SPURIOUS, group_enable,
    group_mask, preempts, read_through,
};
use crate::cpu::VirtualLines;

/// The List registers: four, as the GIC-400 implements (DDI 0471B
/// `GICH_VTR.ListRegs` 3), and QEMU (`GIC_NR_LR`).
pub const NUM_LR: usize = 4;

/// `GICH_VTR` (IHI 0048B §5.3.2): `PRIbits` and `PREbits` 0b100, five bits
/// each, the only values GICv2 allows; `ListRegs` the count less one. The
/// GIC-400's reset value (DDI 0471B Table 3-8).
pub const GICH_VTR: u32 = (0b100 << 29) | (0b100 << 26) | (NUM_LR as u32 - 1);

/// INTID 25, the maintenance interrupt: the GIC's own, "an internal PPI,
/// known as the virtual maintenance interrupt, PPI ID 25" that the
/// GIC-400 drives itself (DDI 0471B §2.3.2), which Arm's BSA 1.0C assigns
/// it too and QEMU names `ARCH_GIC_MAINT_IRQ`. So the device, not the
/// board, wires it.
pub const MAINTENANCE_INTID: u32 = 25;

/// `GICV_BPR`'s and `GICV_ABPR`'s minimum binary points: 32 preemption
/// levels (§5.5.3; footnote a of Table 5-4).
const MIN_BPR: u8 = 2;
const MIN_ABPR: u8 = 3;

// GICH_HCR (Table 5-2).
const HCR_EN: u32 = 1 << 0;
const HCR_UIE: u32 = 1 << 1;
const HCR_LRENPIE: u32 = 1 << 2;
const HCR_NPIE: u32 = 1 << 3;
const HCR_VGRP0EIE: u32 = 1 << 4;
const HCR_VGRP0DIE: u32 = 1 << 5;
const HCR_VGRP1EIE: u32 = 1 << 6;
const HCR_VGRP1DIE: u32 = 1 << 7;
const HCR_EOICOUNT_SHIFT: u32 = 27;
const HCR_EOICOUNT: u32 = 0x1F << HCR_EOICOUNT_SHIFT;
const HCR_MASK: u32 = HCR_EOICOUNT | 0xFF;

// GICH_MISR (Table 5-5).
const MISR_EOI: u32 = 1 << 0;
const MISR_U: u32 = 1 << 1;
const MISR_LRENP: u32 = 1 << 2;
const MISR_NP: u32 = 1 << 3;
const MISR_VGRP0E: u32 = 1 << 4;
const MISR_VGRP0D: u32 = 1 << 5;
const MISR_VGRP1E: u32 = 1 << 6;
const MISR_VGRP1D: u32 = 1 << 7;

/// `GICV_PMR`: bits [7:3]; [2:0] are reserved (§5.5.2).
const PMR_MASK: u8 = 0xF8;

// GICH_LRn (Table 5-9).
const LR_HW: u32 = 1 << 31;
const LR_GRP1: u32 = 1 << 30;
const LR_STATE_SHIFT: u32 = 28;
const LR_PRIORITY_SHIFT: u32 = 23;
const LR_PHYSICAL_SHIFT: u32 = 10;
/// With `HW` 0, bit 19 of `PhysicalID` is `EOI`.
const LR_EOI: u32 = 1 << 19;
/// Bits [22:20] are reserved, RAZ/WI.
const LR_MASK: u32 = !(0b111 << 20);

/// A List register's `State` (Table 5-9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LrState {
    Invalid,
    Pending,
    Active,
    PendingActive,
}

impl LrState {
    const fn of(lr: u32) -> Self {
        match (lr >> LR_STATE_SHIFT) & 0b11 {
            0b00 => Self::Invalid,
            0b01 => Self::Pending,
            0b10 => Self::Active,
            _ => Self::PendingActive,
        }
    }

    const fn bits(self) -> u32 {
        (match self {
            Self::Invalid => 0b00,
            Self::Pending => 0b01,
            Self::Active => 0b10,
            Self::PendingActive => 0b11,
        }) << LR_STATE_SHIFT
    }

    const fn is_active(self) -> bool {
        matches!(self, Self::Active | Self::PendingActive)
    }
}

/// Whether a deactivation that finds no List register counts in
/// `GICH_HCR.EOICount`.
#[derive(Clone, Copy)]
enum CountsUnmatched {
    Yes,
    No,
}

/// What a read of the virtual CPU interface did besides return `value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadEffect {
    /// Nothing: the interface is as it was.
    Nothing,
    /// An acknowledge: a List register became active, so the outputs and
    /// the maintenance interrupt are to be recomputed.
    Acknowledged,
}

/// A read of the virtual CPU interface: its value and its effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub(crate) struct VcpuRead {
    pub(crate) value: u32,
    pub(crate) effect: ReadEffect,
}

/// A physical interrupt a hardware List register's deactivation retires in
/// the distributor (Table 5-9 `HW`): its `PhysicalID`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub(crate) struct HwDeactivation(pub(crate) Option<u32>);

/// The virtual interface of the one processor.
pub(crate) struct VirtInterface {
    /// `GICH_HCR`, `EOICount` included.
    hcr: u32,
    /// `GICV_CTLR` (`GICH_VMCR`'s `VEM`, `VMCBPR`, `VMFIQEn`, `VMAckCtl`,
    /// `VMGrp1En`, `VMGrp0En`).
    ctlr: u32,
    /// `GICV_PMR` (`GICH_VMCR.VMPriMask`), bits [7:3].
    pmr: u8,
    /// `GICV_BPR` (`GICH_VMCR.VMBP`).
    bpr: u8,
    /// `GICV_ABPR` (`GICH_VMCR.VMABP`).
    abpr: u8,
    /// `GICH_APR` (and `GICV_APR0`): one bit per active preemption level.
    apr: u32,
    lr: [u32; NUM_LR],
}

impl VirtInterface {
    /// The reset values of Tables 5-1 and 5-10: everything 0 but the binary
    /// points, which reset to their minimums.
    pub(crate) const fn new() -> Self {
        Self {
            hcr: 0,
            ctlr: 0,
            pmr: 0,
            bpr: MIN_BPR,
            abpr: MIN_ABPR,
            apr: 0,
            lr: [0; NUM_LR],
        }
    }

    fn enabled(&self) -> bool {
        self.hcr & HCR_EN != 0
    }

    fn eoi_split(&self) -> bool {
        self.ctlr & CTLR_EOIMODE != 0
    }

    /// The `GICH_HCR.EOICount` increment of an EOI or deactivation that
    /// finds no List register, wrapping at 31 (Table 5-2).
    fn count_unmatched_eoi(&mut self) {
        let count = ((self.hcr >> HCR_EOICOUNT_SHIFT) + 1) & 0x1F;
        self.hcr = (self.hcr & !HCR_EOICOUNT) | (count << HCR_EOICOUNT_SHIFT);
    }

    /// List register `n`'s priority, as eight bits: its five implemented
    /// bits are [7:3] (§5.3.2 `PRIbits`).
    fn priority(&self, n: usize) -> u8 {
        (((self.lr[n] >> LR_PRIORITY_SHIFT) & 0x1F) << 3) as u8
    }

    fn group1(&self, n: usize) -> bool {
        self.lr[n] & LR_GRP1 != 0
    }

    fn virtual_id(&self, n: usize) -> u32 {
        self.lr[n] & 0x3FF
    }

    /// List register `n`'s group mask under `GICV_BPR` and `GICV_ABPR`
    /// ([`group_mask`]; §5.5.3, §5.5.8).
    fn group_mask(&self, n: usize) -> u8 {
        group_mask(
            self.group1(n),
            self.ctlr & CTLR_CBPR != 0,
            self.bpr,
            self.abpr,
        )
    }

    /// The running priority: the highest active priority in `GICH_APR`,
    /// whose bit `n` stands for priority `n << 3`, a List register's five
    /// priority bits (§5.3.7).
    fn running_priority(&self) -> u16 {
        if self.apr == 0 {
            IDLE_PRIORITY
        } else {
            (self.apr.trailing_zeros() << 3) as u16
        }
    }

    /// The groups the virtual CPU interface signals (§5.4.1).
    fn group_enabled(&self, n: usize) -> bool {
        self.ctlr & group_enable(self.group1(n)) != 0
    }

    /// The highest-priority pending List register of the groups `groups`
    /// (`GICD_CTLR_EN_GRP0`/`GICD_CTLR_EN_GRP1` bits) that beats `GICV_PMR`
    /// (§5.5.2), while `GICH_HCR.En` has the virtual CPU interface operate
    /// (Table 5-2) (QEMU `gic_get_best_virq`).
    fn best_pending(&self, groups: u32) -> Option<usize> {
        if !self.enabled() {
            return None;
        }
        let mut best: Option<usize> = None;
        for n in 0..NUM_LR {
            if LrState::of(self.lr[n]) != LrState::Pending
                || groups & group_enable(self.group1(n)) == 0
            {
                continue;
            }
            if best.is_none_or(|b| self.priority(n) < self.priority(b)) {
                best = Some(n);
            }
        }
        best.filter(|&n| self.priority(n) < self.pmr)
    }

    /// The highest-priority pending interrupt `GICV_HPPIR` reports: of
    /// either group, whatever `GICV_CTLR` enables, as the GIC-400's
    /// ("even when the appropriate `GICV_CTLR.EnableGrp` bit is set to 0",
    /// DDI 0471B Table 3-10, note b).
    fn highest_pending(&self) -> Option<usize> {
        self.best_pending(CTLR_EN_GRP0 | CTLR_EN_GRP1)
    }

    /// Whether List register `n` preempts the running priority, under its
    /// own group's binary point ([`preempts`]).
    fn preempts(&self, n: usize) -> bool {
        preempts(
            self.priority(n),
            self.group_mask(n),
            self.running_priority(),
        )
    }

    /// The interrupt the virtual CPU interface signals, if any: the highest
    /// pending one of the groups it chooses among, both but in IHI 0048B
    /// §4.1.5's one case ([`GicV2::considered_groups`]), if `GICV_CTLR`
    /// enables its group (§5.4.1) and it preempts the running priority.
    /// `GICV_CTLR`'s group enables take no further part in the choice, as
    /// the physical CPU interface's take none (§3.7.2).
    fn signalled(&self) -> Option<usize> {
        self.best_pending(GicV2::considered_groups(self.ctlr))
            .filter(|&n| self.group_enabled(n) && self.preempts(n))
    }

    /// The virtual IRQ and FIQ to the processor (Arm `VirtualIRQPending()`,
    /// `VirtualFIQPending()`), from the one interrupt signalled: a Group 0
    /// interrupt with `FIQEn` set is a FIQ, any other an IRQ (Table 5-9
    /// `Grp1`, Table 5-11 `FIQEn`).
    pub(crate) fn outputs(&self) -> VirtualLines {
        match self.signalled() {
            None => VirtualLines::NONE,
            Some(n) => {
                let fiq = !self.group1(n) && self.ctlr & CTLR_FIQ_EN != 0;
                VirtualLines { irq: !fiq, fiq }
            }
        }
    }

    /// `GICH_EISR0`: the List registers with an EOI maintenance interrupt,
    /// invalid with `HW` 0 and `EOI` 1 (Table 5-6, footnote a).
    fn eisr(&self) -> u32 {
        (0..NUM_LR)
            .filter(|&n| {
                let lr = self.lr[n];
                LrState::of(lr) == LrState::Invalid && lr & LR_HW == 0 && lr & LR_EOI != 0
            })
            .fold(0, |bits, n| bits | 1 << n)
    }

    /// `GICH_ELRSR0`: the empty List registers, invalid with `HW` 1 or `EOI`
    /// 0 (Table 5-7, footnote a).
    fn elrsr(&self) -> u32 {
        (0..NUM_LR)
            .filter(|&n| {
                let lr = self.lr[n];
                LrState::of(lr) == LrState::Invalid && (lr & LR_HW != 0 || lr & LR_EOI == 0)
            })
            .fold(0, |bits, n| bits | 1 << n)
    }

    /// `GICH_MISR` (Table 5-5).
    fn misr(&self) -> u32 {
        let hcr = self.hcr;
        let valid = self
            .lr
            .iter()
            .filter(|&&lr| LrState::of(lr) != LrState::Invalid)
            .count();
        // "No List register is in pending state": active and pending is a
        // state of its own (Table 5-9 `State`), as in QEMU's
        // `gic_extract_lr_info`.
        let none_pending = !self
            .lr
            .iter()
            .any(|&lr| LrState::of(lr) == LrState::Pending);
        let grp0 = self.ctlr & CTLR_EN_GRP0 != 0;
        let grp1 = self.ctlr & CTLR_EN_GRP1 != 0;
        let mut misr = 0;
        if self.eisr() != 0 {
            misr |= MISR_EOI;
        }
        if hcr & HCR_UIE != 0 && valid <= 1 {
            misr |= MISR_U;
        }
        if hcr & HCR_LRENPIE != 0 && hcr & HCR_EOICOUNT != 0 {
            misr |= MISR_LRENP;
        }
        if hcr & HCR_NPIE != 0 && none_pending {
            misr |= MISR_NP;
        }
        if hcr & HCR_VGRP0EIE != 0 && grp0 {
            misr |= MISR_VGRP0E;
        }
        if hcr & HCR_VGRP0DIE != 0 && !grp0 {
            misr |= MISR_VGRP0D;
        }
        if hcr & HCR_VGRP1EIE != 0 && grp1 {
            misr |= MISR_VGRP1E;
        }
        if hcr & HCR_VGRP1DIE != 0 && !grp1 {
            misr |= MISR_VGRP1D;
        }
        misr
    }

    /// The maintenance interrupt's level: a `GICH_MISR` bit set while
    /// `GICH_HCR.En` is (§5.3.4, §5.2.5: level-sensitive).
    pub(crate) fn maintenance(&self) -> bool {
        self.enabled() && self.misr() != 0
    }

    /// `GICH_VMCR` (Table 5-4).
    fn vmcr(&self) -> u32 {
        (u32::from(self.pmr >> 3) << 27)
            | (u32::from(self.bpr) << 21)
            | (u32::from(self.abpr) << 18)
            | self.ctlr
    }

    fn set_vmcr(&mut self, v: u32) {
        self.pmr = (((v >> 27) & 0x1F) << 3) as u8;
        self.bpr = (((v >> 21) & 7) as u8).max(MIN_BPR);
        self.abpr = (((v >> 18) & 7) as u8).max(MIN_ABPR);
        self.ctlr = v & CTLR_MASK;
    }

    /// The List register at `off`, a word of 0x100..=0x1FC (Table 5-1), if
    /// it is one of those implemented.
    fn lr_at(off: u64) -> Option<usize> {
        match off {
            0x100..=0x1FC if off.is_multiple_of(4) => {
                Some(((off - 0x100) / 4) as usize).filter(|&n| n < NUM_LR)
            }
            _ => None,
        }
    }

    /// A read of the virtual interface control registers (Table 5-1),
    /// reserved and unimplemented ones, and an offset inside a register,
    /// reading as zero.
    pub(crate) fn hyp_read(&self, off: u64) -> u32 {
        match off {
            0x00 => self.hcr,
            0x04 => GICH_VTR,
            0x08 => self.vmcr(),
            0x10 => self.misr(),
            0x20 => self.eisr(),
            0x30 => self.elrsr(),
            0xF0 => self.apr,
            _ => Self::lr_at(off).map_or(0, |n| self.lr[n]),
        }
    }

    /// A write of the virtual interface control registers; read-only and
    /// reserved ones, and an offset inside a register, ignore it.
    pub(crate) fn hyp_write(&mut self, off: u64, v: u32) {
        match off {
            0x00 => self.hcr = v & HCR_MASK,
            0x08 => self.set_vmcr(v),
            0xF0 => self.apr = v,
            _ => {
                if let Some(n) = Self::lr_at(off) {
                    self.lr[n] = v & LR_MASK;
                }
            }
        }
    }

    /// The ID `GICV_IAR`, `GICV_AIAR`, `GICV_HPPIR` and `GICV_AHPPIR`
    /// return for List register `n`: its `VirtualID`, with `CPUID` from bits
    /// [12:10] for a software interrupt and zero for a hardware one
    /// (§5.5.4).
    fn reported_id(&self, n: usize) -> u32 {
        let lr = self.lr[n];
        let cpuid = if lr & LR_HW == 0 {
            (lr >> LR_PHYSICAL_SHIFT) & 0b111
        } else {
            0
        };
        self.virtual_id(n) | (cpuid << 10)
    }

    /// List register `n`'s interrupt as `view` reads it ([`read_through`]).
    fn read_through(&self, view: View, n: usize) -> u32 {
        read_through(
            view,
            self.group1(n),
            self.ctlr & CTLR_ACK_CTL != 0,
            self.reported_id(n),
        )
    }

    /// `GICV_HPPIR` and `GICV_AHPPIR` (Tables 5-14 and 5-17): the highest
    /// pending interrupt whatever `GICV_CTLR` enables, as the GIC-400's
    /// ("even when the appropriate `GICV_CTLR.EnableGrp` bit is set to 0",
    /// DDI 0471B Table 3-10, note b), if it is "of sufficient priority"
    /// past `GICV_PMR` (§4.1.5; Table 5-17); 1023 while `GICH_HCR.En` is
    /// clear.
    fn hppir(&self, view: View) -> u32 {
        match self.highest_pending() {
            Some(n) => self.read_through(view, n),
            None => SPURIOUS,
        }
    }

    /// `GICV_IAR` and `GICV_AIAR` (§5.5.4, Tables 5-12 and 5-15): the
    /// signalled interrupt becomes active and its preemption level is set
    /// in `GICH_APR`. Only an interrupt "of sufficient priority to be
    /// signaled" reads as its ID or as 1022 (Table 5-12, note a); without
    /// one the read is 1023. The read says whether it acknowledged.
    fn acknowledge(&mut self, view: View) -> VcpuRead {
        let spurious = |value| VcpuRead {
            value,
            effect: ReadEffect::Nothing,
        };
        let Some(n) = self.signalled() else {
            return spurious(SPURIOUS);
        };
        // The ID field alone: a software interrupt's CPUID is above it.
        let id = self.read_through(view, n);
        if id & 0x3FF >= MAXIRQ {
            return spurious(id);
        }
        // §5.3.7: the bit set is the List register's five priority bits,
        // whatever the binary points hold.
        self.apr |= 1 << (self.priority(n) >> 3);
        self.lr[n] = (self.lr[n] & !LrState::PendingActive.bits()) | LrState::Active.bits();
        VcpuRead {
            value: id,
            effect: ReadEffect::Acknowledged,
        }
    }

    /// The valid List register an EOI or a deactivation writing `written`
    /// names, if any: the one of its Interrupt ID, which no other valid List
    /// register holds (Table 5-9: "Each valid interrupt stored in the List
    /// registers must have a unique VirtualID"), as QEMU's
    /// `gic_get_lr_entry` finds it, in whatever state it is.
    fn named_lr(&self, written: u32) -> Option<usize> {
        let id = written & 0x3FF;
        (0..NUM_LR)
            .find(|&n| LrState::of(self.lr[n]) != LrState::Invalid && self.virtual_id(n) == id)
    }

    /// The deactivation of the List register `written` names: of an active
    /// one, its deactivation; of one only pending, nothing, which EOIs and
    /// `GICV_DIR` leave UNPREDICTABLE (§5.5.14) and which counts in no
    /// `EOICount`, there being a corresponding entry (Table 5-2); of none, an
    /// `EOICount` increment where `counts`.
    fn deactivate_named(&mut self, written: u32, counts: CountsUnmatched) -> HwDeactivation {
        match self.named_lr(written) {
            Some(n) if LrState::of(self.lr[n]).is_active() => self.deactivate_lr(n),
            Some(_) => HwDeactivation(None),
            None => {
                match counts {
                    CountsUnmatched::Yes => self.count_unmatched_eoi(),
                    CountsUnmatched::No => {}
                }
                HwDeactivation(None)
            }
        }
    }

    /// Deactivate List register `n` (it stays pending if it was active and
    /// pending), and forward a hardware interrupt's deactivation to the
    /// distributor (Table 5-9 `HW`).
    fn deactivate_lr(&mut self, n: usize) -> HwDeactivation {
        let state = LrState::of(self.lr[n]);
        let rest = if state == LrState::PendingActive {
            LrState::Pending
        } else {
            LrState::Invalid
        };
        self.lr[n] = (self.lr[n] & !LrState::PendingActive.bits()) | rest.bits();
        if self.lr[n] & LR_HW == 0 {
            return HwDeactivation(None);
        }
        let physical = (self.lr[n] >> LR_PHYSICAL_SHIFT) & 0x3FF;
        // A PhysicalID of an SGI, or of a special ID, is UNPREDICTABLE
        // (Table 5-9) and retires nothing.
        HwDeactivation(Some(physical).filter(|&id| (16..MAXIRQ).contains(&id)))
    }

    /// `GICV_EOIR` and `GICV_AEOIR` (§5.5.5, §5.5.10), written `written`:
    /// the running priority drops and, unless `EOImode` splits the two, the
    /// List register it names deactivates.
    fn end_of_interrupt(&mut self, written: u32) -> HwDeactivation {
        if written & 0x3FF >= MAXIRQ {
            return HwDeactivation(None);
        }
        // Table 5-2: "EOIs that do not clear a bit in the Active Priorities
        // register, GICH_APR do not cause an increment".
        let counts = if self.apr != 0 {
            CountsUnmatched::Yes
        } else {
            CountsUnmatched::No
        };
        self.apr &= self.apr.wrapping_sub(1);
        if self.eoi_split() {
            return HwDeactivation(None);
        }
        self.deactivate_named(written, counts)
    }

    /// `GICV_DIR` (§5.5.14), written `written`: valid only while `EOImode`
    /// is set, otherwise UNPREDICTABLE and ignored.
    fn deactivate(&mut self, written: u32) -> HwDeactivation {
        if !self.eoi_split() || written & 0x3FF >= MAXIRQ {
            return HwDeactivation(None);
        }
        // §5.5.14: "If the specified Interrupt does not exist in the List
        // registers, the GICH_HCR.EOIcount field is incremented".
        self.deactivate_named(written, CountsUnmatched::Yes)
    }

    /// A read of the virtual CPU interface (Table 5-10). `iidr` is what
    /// `GICC_IIDR` reads, which `GICV_IIDR` repeats.
    pub(crate) fn vcpu_read(&mut self, off: u64, iidr: u32) -> VcpuRead {
        let plain = ReadEffect::Nothing;
        let (value, effect) = match off {
            0x0C => return self.acknowledge(View::Primary),
            0x20 => return self.acknowledge(View::Group1Alias),
            0x00 => (self.ctlr, plain),
            0x04 => (u32::from(self.pmr), plain),
            0x08 => (u32::from(self.bpr), plain),
            0x14 => (u32::from(self.running_priority().min(0xFF)), plain),
            0x18 => (self.hppir(View::Primary), plain),
            0x1C => (u32::from(self.abpr), plain),
            0x28 => (self.hppir(View::Group1Alias), plain),
            0xD0 => (self.apr, plain),
            0xFC => (iidr, plain),
            _ => (0, plain),
        };
        VcpuRead { value, effect }
    }

    /// A write of the virtual CPU interface.
    pub(crate) fn vcpu_write(&mut self, off: u64, v: u32) -> HwDeactivation {
        match off {
            0x00 => self.ctlr = v & CTLR_MASK,
            0x04 => self.pmr = v as u8 & PMR_MASK,
            0x08 => self.bpr = (v as u8 & 7).max(MIN_BPR),
            0x10 | 0x24 => return self.end_of_interrupt(v),
            0x1C => self.abpr = (v as u8 & 7).max(MIN_ABPR),
            0xD0 => self.apr = v,
            0x1000 => return self.deactivate(v),
            _ => {}
        }
        HwDeactivation(None)
    }

    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_u32(self.hcr)?;
        w.write_u32(self.ctlr)?;
        w.write_bytes(&[self.pmr, self.bpr, self.abpr])?;
        w.write_u32(self.apr)?;
        for lr in self.lr {
            w.write_u32(lr)?;
        }
        Ok(())
    }

    /// Reads the virtual interface's section of the image and refuses one
    /// whose registers hold bits they cannot: nothing changes before the
    /// caller accepts the result.
    pub(crate) fn restore<R: SnapRead>(r: &mut R) -> Result<Self, SnapError> {
        let hcr = r.read_u32()?;
        let ctlr = r.read_u32()?;
        let mut points = [0u8; 3];
        r.read_bytes(&mut points)?;
        let [pmr, bpr, abpr] = points;
        let apr = r.read_u32()?;
        let mut lr = [0u32; NUM_LR];
        for word in lr.iter_mut() {
            *word = r.read_u32()?;
        }
        if hcr & !HCR_MASK != 0 || ctlr & !CTLR_MASK != 0 || pmr & !PMR_MASK != 0 {
            return Err(SnapError::Invalid(
                "GIC virtual interface register with bits it does not keep",
            ));
        }
        if !(MIN_BPR..=7).contains(&bpr) || !(MIN_ABPR..=7).contains(&abpr) {
            return Err(SnapError::Invalid(
                "GIC virtual binary point outside its register's range",
            ));
        }
        if lr.iter().any(|&lr| lr & !LR_MASK != 0) {
            return Err(SnapError::Invalid(
                "GIC List register with reserved bits set",
            ));
        }
        Ok(Self {
            hcr,
            ctlr,
            pmr,
            bpr,
            abpr,
            apr,
            lr,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{GICH_VTR, MAINTENANCE_INTID};
    use crate::bus::{AccessWidth, RegOffset};
    use crate::devices::gicv2::tests::{cpu, cpu_get, dist, dist_get};
    use crate::devices::gicv2::{GicExtensions, GicV2};

    // GICH_* offsets (IHI 0048B Table 5-1).
    const HCR: u64 = 0x00;
    const VTR: u64 = 0x04;
    const VMCR: u64 = 0x08;
    const MISR: u64 = 0x10;
    const EISR0: u64 = 0x20;
    const ELRSR0: u64 = 0x30;
    const ELRSR1: u64 = 0x34;
    const APR: u64 = 0xF0;
    const LR0: u64 = 0x100;
    // GICV_* offsets (Table 5-10).
    const CTLR: u64 = 0x00;
    const PMR: u64 = 0x04;
    const BPR: u64 = 0x08;
    const IAR: u64 = 0x0C;
    const EOIR: u64 = 0x10;
    const RPR: u64 = 0x14;
    const HPPIR: u64 = 0x18;
    const ABPR: u64 = 0x1C;
    const AIAR: u64 = 0x20;
    const AEOIR: u64 = 0x24;
    const AHPPIR: u64 = 0x28;
    const APR0: u64 = 0xD0;
    const IIDR: u64 = 0xFC;
    const DIR: u64 = 0x1000;

    const EN: u32 = 1;
    const GRP0: u32 = 1 << 0;
    const GRP1: u32 = 1 << 1;
    const ACK_CTL: u32 = 1 << 2;
    const FIQ_EN: u32 = 1 << 3;
    const EOIMODE: u32 = 1 << 9;

    const HW: u32 = 1 << 31;
    const LR_GRP1: u32 = 1 << 30;
    const PENDING: u32 = 0b01 << 28;
    const ACTIVE: u32 = 0b10 << 28;
    const PENDING_ACTIVE: u32 = 0b11 << 28;
    const LR_EOI: u32 = 1 << 19;

    /// A List register's `Priority` field for eight-bit priority `p`.
    const fn prio(p: u32) -> u32 {
        (p >> 3) << 23
    }

    /// A hardware List register for physical interrupt `phys`.
    const fn physical(phys: u32) -> u32 {
        HW | (phys << 10)
    }

    fn hyp(g: &mut GicV2, off: u64, v: u32) {
        g.hyp_write(RegOffset::new(off), AccessWidth::Word, u64::from(v));
    }

    fn hyp_get(g: &mut GicV2, off: u64) -> u32 {
        g.hyp_read(RegOffset::new(off), AccessWidth::Word) as u32
    }

    fn vcpu(g: &mut GicV2, off: u64, v: u32) {
        g.vcpu_write(RegOffset::new(off), AccessWidth::Word, u64::from(v));
    }

    fn vcpu_get(g: &mut GicV2, off: u64) -> u32 {
        g.vcpu_read(RegOffset::new(off), AccessWidth::Word) as u32
    }

    /// A GIC with the extensions, its virtual CPU interface enabled for
    /// both groups with no priority masked, and `GICH_HCR.En` set.
    fn virtual_gic() -> GicV2 {
        let mut g = GicV2::with_extensions(GicExtensions::Virtualization);
        hyp(&mut g, HCR, EN);
        vcpu(&mut g, CTLR, GRP0 | GRP1);
        vcpu(&mut g, PMR, 0xF8);
        g
    }

    /// Whether interrupt `intid` is pending at the distributor
    /// (`GICD_ISPENDRn`).
    fn pending_at_distributor(g: &mut GicV2, intid: u32) -> bool {
        dist_get(g, 0x200 + u64::from(intid / 32) * 4) & (1 << (intid % 32)) != 0
    }

    fn active_at_distributor(g: &mut GicV2, intid: u32) -> bool {
        dist_get(g, 0x300 + u64::from(intid / 32) * 4) & (1 << (intid % 32)) != 0
    }

    /// The reset values of Tables 5-1 and 5-10, `GICH_VTR` the GIC-400's,
    /// and List registers past the fourth RAZ/WI (§5.3.8).
    #[test]
    fn the_virtual_interface_resets_as_ihi_0048b_gives_it() {
        let mut g = GicV2::with_extensions(GicExtensions::Virtualization);
        assert_eq!(hyp_get(&mut g, VTR), 0x9000_0003);
        assert_eq!(GICH_VTR, 0x9000_0003);
        assert_eq!(hyp_get(&mut g, HCR), 0);
        assert_eq!(hyp_get(&mut g, MISR), 0);
        assert_eq!(hyp_get(&mut g, EISR0), 0);
        assert_eq!(hyp_get(&mut g, ELRSR0), 0xF, "four empty List registers");
        assert_eq!(hyp_get(&mut g, ELRSR1), 0, "GICH_ELRSR1: none implemented");
        assert_eq!(vcpu_get(&mut g, BPR), 2);
        assert_eq!(vcpu_get(&mut g, ABPR), 3);
        assert_eq!(vcpu_get(&mut g, IAR), 1023);
        assert_eq!(vcpu_get(&mut g, RPR), 0xFF);
        assert_eq!(vcpu_get(&mut g, HPPIR), 1023);
        assert_eq!(vcpu_get(&mut g, IIDR), cpu_get(&mut g, IIDR));
        hyp(&mut g, LR0 + 4 * 4, PENDING | 40);
        assert_eq!(hyp_get(&mut g, LR0 + 4 * 4), 0, "GICH_LR4 is RAZ/WI");
        assert_eq!(hyp_get(&mut g, ELRSR0), 0xF);
        assert!(!g.virtual_lines().irq && !g.virtual_lines().fiq);
    }

    /// A GIC without the extensions drives no virtual output whatever its
    /// physical state.
    #[test]
    fn a_gic_without_the_extensions_raises_no_virtual_interrupt() {
        let mut g = GicV2::new();
        dist(&mut g, 0x000, 3);
        cpu(&mut g, 0x00, 3);
        cpu(&mut g, 0x04, 0xFF);
        dist(&mut g, 0x100, 1 << 27);
        g.set_line(27, true);
        assert!(g.irq_out());
        assert!(!g.virtual_lines().irq && !g.virtual_lines().fiq);
        assert_eq!(g.extensions(), GicExtensions::Absent);
    }

    /// A pending Group 1 List register raises the virtual IRQ; `GICV_AIAR`
    /// acknowledges it, making it active and setting its preemption level
    /// in `GICH_APR` (§5.5.4, §5.5.9), and the running priority hides it.
    #[test]
    fn a_pending_list_register_is_signalled_and_acknowledged() {
        let mut g = virtual_gic();
        hyp(&mut g, LR0, LR_GRP1 | PENDING | prio(0x48) | 27);
        assert!(g.virtual_lines().irq);
        assert!(!g.virtual_lines().fiq);
        assert_eq!(vcpu_get(&mut g, AHPPIR), 27);
        assert_eq!(vcpu_get(&mut g, AIAR), 27);
        assert_eq!(hyp_get(&mut g, LR0) & PENDING_ACTIVE, ACTIVE);
        // Group 1 under GICV_ABPR (3), so BPR 2 applies: group priority
        // 0x48 & 0xF8 = 0x48, level 9.
        assert_eq!(hyp_get(&mut g, APR), 1 << (0x48 >> 3));
        assert_eq!(
            vcpu_get(&mut g, APR0),
            hyp_get(&mut g, APR),
            "GICV_APR0 aliases it"
        );
        assert_eq!(vcpu_get(&mut g, RPR), 0x48);
        assert!(!g.virtual_lines().irq, "active, so not signalled");
        assert_eq!(vcpu_get(&mut g, AIAR), 1023);
    }

    /// The highest priority wins whatever its List register, and of equal
    /// priorities the lowest-numbered List register (QEMU
    /// `gic_get_best_virq`, where IHI 0048B §3.2.3 leaves the choice open).
    #[test]
    fn the_highest_priority_list_register_is_offered_first() {
        let mut g = virtual_gic();
        hyp(&mut g, LR0, PENDING | prio(0x60) | 50);
        hyp(&mut g, LR0 + 4, PENDING | prio(0x40) | 51);
        hyp(&mut g, LR0 + 8, PENDING | prio(0x40) | 52);
        hyp(&mut g, LR0 + 12, PENDING | prio(0x50) | 53);
        assert_eq!(vcpu_get(&mut g, HPPIR), 51);
        assert_eq!(vcpu_get(&mut g, IAR), 51);
        vcpu(&mut g, EOIR, 51);
        assert_eq!(vcpu_get(&mut g, IAR), 52);
        vcpu(&mut g, EOIR, 52);
        assert_eq!(vcpu_get(&mut g, IAR), 53);
        vcpu(&mut g, EOIR, 53);
        assert_eq!(vcpu_get(&mut g, IAR), 50);
    }

    /// `GICV_IAR` on a Group 1 interrupt reads 1022 and acknowledges nothing
    /// while `AckCtl` is clear, and acknowledges it when set (Table 5-12);
    /// `GICV_AIAR` and `GICV_AHPPIR` on Group 0 read 1023 (Tables 5-15,
    /// 5-17).
    #[test]
    fn the_acknowledge_registers_take_their_own_groups() {
        let mut g = virtual_gic();
        hyp(&mut g, LR0, LR_GRP1 | PENDING | prio(0x40) | 30);
        assert_eq!(vcpu_get(&mut g, IAR), 1022);
        assert_eq!(vcpu_get(&mut g, HPPIR), 1022);
        assert_eq!(
            hyp_get(&mut g, LR0) & PENDING_ACTIVE,
            PENDING,
            "not acknowledged"
        );
        vcpu(&mut g, CTLR, GRP0 | GRP1 | ACK_CTL);
        assert_eq!(vcpu_get(&mut g, HPPIR), 30);
        assert_eq!(vcpu_get(&mut g, IAR), 30);

        let mut g = virtual_gic();
        hyp(&mut g, LR0, PENDING | prio(0x40) | 31);
        assert_eq!(vcpu_get(&mut g, AHPPIR), 1023);
        assert_eq!(vcpu_get(&mut g, AIAR), 1023);
        assert_eq!(vcpu_get(&mut g, IAR), 31);
    }

    /// A Group 1 interrupt that cannot preempt the running priority reads
    /// 1023 at `GICV_IAR`, not 1022: only one "of sufficient priority to be
    /// signaled" reads 1022 (Table 5-12, note a). `GICV_HPPIR` still names
    /// it (Table 5-14).
    #[test]
    fn only_a_signalled_group_1_interrupt_reads_1022() {
        let mut g = virtual_gic();
        hyp(&mut g, APR, 1 << (0x40 >> 3));
        hyp(&mut g, LR0, LR_GRP1 | PENDING | prio(0x60) | 30);
        assert_eq!(vcpu_get(&mut g, IAR), 1023);
        assert_eq!(vcpu_get(&mut g, HPPIR), 1022);
        hyp(&mut g, APR, 0);
        assert_eq!(vcpu_get(&mut g, IAR), 1022);
    }

    /// `GICV_CTLR`'s group enables take no part in choosing the highest
    /// pending List register, as the physical CPU interface's take none
    /// (IHI 0048B §3.7.2): with `AckCtl` clear, a higher-priority interrupt
    /// of a disabled group hides the enabled group's, which is neither
    /// signalled nor acknowledged. `GICV_HPPIR` names it, as the GIC-400's
    /// does whatever the enables (DDI 0471B Table 3-10, note b), and reads
    /// 1023 while `GICH_HCR.En` is clear.
    #[test]
    fn a_disabled_groups_interrupt_hides_the_other() {
        let mut g = virtual_gic();
        vcpu(&mut g, CTLR, GRP1);
        hyp(&mut g, LR0, PENDING | prio(0x20) | 50);
        hyp(&mut g, LR0 + 4, LR_GRP1 | PENDING | prio(0x40) | 51);
        assert!(
            !g.virtual_lines().irq && !g.virtual_lines().fiq,
            "Group 0's interrupt is highest"
        );
        assert_eq!(vcpu_get(&mut g, HPPIR), 50, "Group 0's, though disabled");
        assert_eq!(
            vcpu_get(&mut g, AHPPIR),
            1023,
            "a Group 0 interrupt is highest"
        );
        assert_eq!(vcpu_get(&mut g, AIAR), 1023, "Group 0 is not signalled");
        assert_eq!(hyp_get(&mut g, LR0 + 4) & PENDING_ACTIVE, PENDING);
        hyp(&mut g, HCR, 0);
        assert_eq!(
            vcpu_get(&mut g, HPPIR),
            1023,
            "the interface does not operate"
        );
        hyp(&mut g, HCR, EN);
        hyp(&mut g, LR0, 0);
        assert!(g.virtual_lines().irq, "the Group 1 interrupt is signalled");
        assert_eq!(vcpu_get(&mut g, AIAR), 51);
    }

    /// A pending List register preempts only if its group priority is
    /// higher than the running priority's under its own group's mask (IHI
    /// 0048B §3.3.3, §3.7.2 `UpdateExceptionState`): at `GICV_ABPR` 7 a
    /// Group 1 interrupt at 0x10 and a running 0x48 are both 0x00 under
    /// Group 1's mask 0x80, so it does not preempt, though 0x10 & 0x80 is
    /// below 0x48 itself.
    #[test]
    fn preemption_masks_the_running_priority_too() {
        let mut g = virtual_gic();
        vcpu(&mut g, ABPR, 7);
        hyp(&mut g, LR0, PENDING | prio(0x48) | 40);
        assert_eq!(vcpu_get(&mut g, IAR), 40);
        hyp(&mut g, LR0 + 4, LR_GRP1 | PENDING | prio(0x10) | 41);
        assert!(!g.virtual_lines().irq);
        assert_eq!(vcpu_get(&mut g, AIAR), 1023);
        vcpu(&mut g, EOIR, 40);
        assert!(g.virtual_lines().irq, "nothing active: it is signalled");
        assert_eq!(vcpu_get(&mut g, AIAR), 41);
    }

    /// With `AckCtl` set, Group 0 disabled and Group 1 enabled, "Group 0
    /// interrupts are ignored and GICC_IAR behaves as GICC_AIAR" (IHI 0048B
    /// §4.1.5, which a virtual machine sees in its virtual CPU interface,
    /// §5.4): the higher-priority Group 0 List register hides nothing.
    #[test]
    fn with_ackctl_a_disabled_group_0_is_ignored() {
        let mut g = virtual_gic();
        vcpu(&mut g, CTLR, GRP1 | ACK_CTL);
        hyp(&mut g, LR0, PENDING | prio(0x20) | 50);
        hyp(&mut g, LR0 + 4, LR_GRP1 | PENDING | prio(0x40) | 51);
        assert!(g.virtual_lines().irq, "the Group 1 interrupt is signalled");
        assert_eq!(vcpu_get(&mut g, IAR), 51);
    }

    /// The priority mask holds back an interrupt whose priority is not
    /// higher: it is neither signalled, acknowledged nor reported, being of
    /// no "sufficient priority" (IHI 0048B §4.1.5, Table 5-17; §5.5.2).
    #[test]
    fn the_priority_mask_holds_an_interrupt_back() {
        let mut g = virtual_gic();
        vcpu(&mut g, CTLR, GRP0 | GRP1 | ACK_CTL);
        vcpu(&mut g, PMR, 0x40);
        hyp(&mut g, LR0, LR_GRP1 | PENDING | prio(0x40) | 35);
        assert!(!g.virtual_lines().irq);
        assert_eq!(vcpu_get(&mut g, HPPIR), 1023);
        assert_eq!(vcpu_get(&mut g, AHPPIR), 1023);
        assert_eq!(vcpu_get(&mut g, IAR), 1023);
        vcpu(&mut g, PMR, 0x48);
        assert_eq!(vcpu_get(&mut g, AHPPIR), 35);
    }

    /// An EOI whose ID a pending List register holds has "a corresponding
    /// entry in the List registers" (Table 5-2), so it counts in no
    /// `EOICount` and deactivates nothing.
    #[test]
    fn an_eoi_naming_a_pending_list_register_is_not_counted() {
        let mut g = virtual_gic();
        hyp(&mut g, HCR, EN | (1 << 2));
        hyp(&mut g, APR, 1 << 4);
        hyp(&mut g, LR0, PENDING | prio(0x40) | 50);
        vcpu(&mut g, EOIR, 50);
        assert_eq!(hyp_get(&mut g, HCR) >> 27, 0, "not counted");
        assert_eq!(hyp_get(&mut g, APR), 0, "the priority dropped");
        assert_eq!(hyp_get(&mut g, LR0) & PENDING_ACTIVE, PENDING);
    }

    /// An EOI names its List register by Interrupt ID, which no other valid
    /// List register holds (Table 5-9): a software interrupt's EOI that
    /// writes back the CPUID its acknowledge read deactivates it.
    #[test]
    fn an_eoi_names_its_interrupt_by_id() {
        let mut g = virtual_gic();
        hyp(&mut g, LR0, PENDING | prio(0x10) | (2 << 10) | 3);
        assert_eq!(vcpu_get(&mut g, IAR), (2 << 10) | 3);
        vcpu(&mut g, EOIR, (2 << 10) | 3);
        assert_eq!(hyp_get(&mut g, LR0) & PENDING_ACTIVE, 0);
        assert_eq!(hyp_get(&mut g, APR), 0);
    }

    /// With `EOImode` set, as KVM runs its guests, a hardware List
    /// register's `GICV_DIR` deactivates the physical interrupt in the
    /// distributor (§5.5.14).
    #[test]
    fn a_hardware_interrupts_dir_deactivates_it_in_the_distributor() {
        let mut g = virtual_gic();
        dist(&mut g, 0x000, 1);
        cpu(&mut g, 0x00, 1 | (1 << 9));
        cpu(&mut g, 0x04, 0xFF);
        dist(&mut g, 0x100, 1 << 27);
        g.set_line(27, true);
        assert_eq!(cpu_get(&mut g, 0x0C), 27);
        cpu(&mut g, 0x10, 27);
        g.set_line(27, false);
        assert!(active_at_distributor(&mut g, 27));
        vcpu(&mut g, CTLR, GRP0 | GRP1 | EOIMODE);
        hyp(&mut g, LR0, physical(27) | PENDING | prio(0x40) | 27);
        assert_eq!(vcpu_get(&mut g, IAR), 27);
        vcpu(&mut g, EOIR, 27);
        assert!(
            active_at_distributor(&mut g, 27),
            "the EOI only dropped the priority"
        );
        vcpu(&mut g, DIR, 27);
        assert_eq!(hyp_get(&mut g, LR0) & PENDING_ACTIVE, 0);
        assert!(
            !active_at_distributor(&mut g, 27),
            "retired in the distributor"
        );
    }

    /// A List register is a word: an access at an offset inside one reaches
    /// nothing (Table 5-1 gives each at a word offset).
    #[test]
    fn an_offset_inside_a_list_register_reaches_nothing() {
        let mut g = virtual_gic();
        hyp(&mut g, LR0, PENDING | prio(0x40) | 60);
        g.hyp_write(RegOffset::new(LR0 + 3), AccessWidth::Byte, 0x10);
        assert_eq!(hyp_get(&mut g, LR0), PENDING | prio(0x40) | 60);
        assert_eq!(g.hyp_read(RegOffset::new(LR0 + 1), AccessWidth::Byte), 0);
        assert!(g.virtual_lines().irq);
    }

    /// The processor-specific base of processor 0's virtual interface
    /// control registers, `GICH` + 0x1000 as in the GIC-400, reaches the
    /// same registers as the common base (IHI 0048B §5.2.7); the other
    /// processors' bases are reserved.
    #[test]
    fn the_processor_specific_base_aliases_the_common_one() {
        let mut g = virtual_gic();
        hyp(&mut g, 0x1000 + LR0, PENDING | prio(0x40) | 60);
        assert_eq!(hyp_get(&mut g, LR0), PENDING | prio(0x40) | 60);
        assert_eq!(hyp_get(&mut g, 0x1000 + VTR), GICH_VTR);
        assert_eq!(hyp_get(&mut g, 0x1000 + HCR), EN);
        assert!(g.virtual_lines().irq);
        assert_eq!(hyp_get(&mut g, 0x1200 + VTR), 0, "processor 1's");
        hyp(&mut g, 0x1200 + HCR, 0);
        assert_eq!(hyp_get(&mut g, HCR), EN);
    }

    /// A Group 0 List register is a virtual IRQ, or a virtual FIQ under
    /// `FIQEn` (Table 5-9 `Grp1`, Table 5-11 `FIQEn`).
    #[test]
    fn group_0_is_a_virtual_fiq_under_fiqen() {
        let mut g = virtual_gic();
        hyp(&mut g, LR0, PENDING | prio(0x40) | 33);
        assert!(g.virtual_lines().irq && !g.virtual_lines().fiq);
        vcpu(&mut g, CTLR, GRP0 | GRP1 | FIQ_EN);
        assert!(!g.virtual_lines().irq && g.virtual_lines().fiq);
    }

    /// With `GICH_HCR.En` clear nothing is signalled and `GICV_IAR` reads
    /// 1023 (Table 5-2); a disabled group is neither signalled nor
    /// acknowledged (§5.4.1); a priority no higher than `GICV_PMR` is masked
    /// (§5.5.2).
    #[test]
    fn enables_and_the_priority_mask_hold_interrupts_back() {
        let mut g = virtual_gic();
        hyp(&mut g, LR0, PENDING | prio(0x40) | 34);
        hyp(&mut g, HCR, 0);
        assert!(!g.virtual_lines().irq);
        assert_eq!(vcpu_get(&mut g, IAR), 1023);
        hyp(&mut g, HCR, EN);
        vcpu(&mut g, CTLR, GRP1);
        assert!(!g.virtual_lines().irq);
        assert_eq!(vcpu_get(&mut g, IAR), 1023);
        vcpu(&mut g, CTLR, GRP0 | GRP1);
        vcpu(&mut g, PMR, 0x40);
        assert!(
            !g.virtual_lines().irq,
            "priority 0x40 is not higher than the mask"
        );
        vcpu(&mut g, PMR, 0x48);
        assert!(g.virtual_lines().irq);
        assert_eq!(vcpu_get(&mut g, PMR), 0x48);
        vcpu(&mut g, PMR, 0xFF);
        assert_eq!(vcpu_get(&mut g, PMR), 0xF8, "bits [2:0] are reserved");
    }

    /// A pending interrupt preempts an active one only with a higher group
    /// priority, under the binary point (§5.5.3; IHI 0048B §3.3).
    #[test]
    fn preemption_follows_the_virtual_binary_point() {
        // Changing a binary point while an interrupt is active is
        // UNPREDICTABLE (IHI 0048B §3.3.3), so each case starts afresh.
        for (bpr, preempts) in [(2, true), (7, false)] {
            let mut g = virtual_gic();
            vcpu(&mut g, BPR, bpr);
            assert_eq!(vcpu_get(&mut g, BPR), bpr);
            hyp(&mut g, LR0, PENDING | prio(0x80) | 40);
            assert_eq!(vcpu_get(&mut g, IAR), 40);
            hyp(&mut g, LR0 + 4, PENDING | prio(0x70) | 41);
            assert_eq!(
                g.virtual_lines().irq,
                preempts,
                "0x70 over active 0x80 at binary point {bpr}"
            );
            assert_eq!(vcpu_get(&mut g, IAR), if preempts { 41 } else { 1023 });
        }
        // Group 1 takes GICV_ABPR minus one: at ABPR 7, binary point 6
        // splits 0x00 from 0x80, so 0x70 preempts 0x80; at ABPR 3 too.
        for abpr in [3, 7] {
            let mut g = virtual_gic();
            vcpu(&mut g, ABPR, abpr);
            hyp(&mut g, LR0, LR_GRP1 | PENDING | prio(0x80) | 40);
            assert_eq!(vcpu_get(&mut g, AIAR), 40);
            hyp(&mut g, LR0 + 4, LR_GRP1 | PENDING | prio(0x70) | 41);
            assert!(
                g.virtual_lines().irq,
                "Group 1 0x70 over 0x80 at ABPR {abpr}"
            );
        }
        let mut g = virtual_gic();
        vcpu(&mut g, BPR, 0);
        assert_eq!(vcpu_get(&mut g, BPR), 2, "raised to the minimum");
        vcpu(&mut g, ABPR, 0);
        assert_eq!(vcpu_get(&mut g, ABPR), 3, "raised to the minimum");
    }

    /// Acknowledging List register n sets the `GICH_APR` bit of its five
    /// priority bits whatever the binary point (§5.3.7), and `GICV_RPR`
    /// reads that priority (§5.5.6).
    #[test]
    fn the_active_priority_bit_ignores_the_binary_point() {
        let mut g = virtual_gic();
        vcpu(&mut g, BPR, 4);
        hyp(&mut g, LR0, PENDING | prio(0x38) | 40);
        assert_eq!(vcpu_get(&mut g, IAR), 40);
        assert_eq!(hyp_get(&mut g, APR), 1 << (0x38 >> 3));
        assert_eq!(vcpu_get(&mut g, RPR), 0x38);
    }

    /// `GICV_EOIR` drops the priority and deactivates the List register;
    /// with `HW` set it also deactivates the physical interrupt in the
    /// distributor, of either group (§5.5.5).
    #[test]
    fn a_hardware_interrupts_eoi_deactivates_it_in_the_distributor() {
        let mut g = virtual_gic();
        // The hypervisor's side: PPI 27 acknowledged and its priority
        // dropped, but left active (GICC_CTLR.EOImode set, §5.2.2).
        dist(&mut g, 0x000, 1);
        cpu(&mut g, 0x00, 1 | (1 << 9));
        cpu(&mut g, 0x04, 0xFF);
        dist(&mut g, 0x100, 1 << 27);
        g.set_line(27, true);
        assert_eq!(cpu_get(&mut g, 0x0C), 27);
        cpu(&mut g, 0x10, 27);
        assert!(active_at_distributor(&mut g, 27));
        g.set_line(27, false);
        // The guest's side.
        hyp(&mut g, LR0, physical(27) | PENDING | prio(0x40) | 27);
        assert_eq!(vcpu_get(&mut g, IAR), 27);
        assert_eq!(hyp_get(&mut g, LR0) & PENDING_ACTIVE, ACTIVE);
        vcpu(&mut g, EOIR, 27);
        assert_eq!(hyp_get(&mut g, LR0) & PENDING_ACTIVE, 0, "invalid");
        assert_eq!(hyp_get(&mut g, APR), 0);
        assert!(
            !active_at_distributor(&mut g, 27),
            "retired in the distributor"
        );
        assert_eq!(hyp_get(&mut g, ELRSR0) & 1, 1, "an empty List register");
    }

    /// Under `EOImode` an EOI only drops the priority, and `GICV_DIR`
    /// deactivates (Table 5-11); without `EOImode` `GICV_DIR` is ignored.
    #[test]
    fn eoimode_splits_priority_drop_from_deactivation() {
        let mut g = virtual_gic();
        vcpu(&mut g, CTLR, GRP0 | GRP1 | EOIMODE);
        hyp(&mut g, LR0, PENDING | prio(0x40) | 42);
        assert_eq!(vcpu_get(&mut g, IAR), 42);
        vcpu(&mut g, EOIR, 42);
        assert_eq!(hyp_get(&mut g, APR), 0, "the priority dropped");
        assert_eq!(
            hyp_get(&mut g, LR0) & PENDING_ACTIVE,
            ACTIVE,
            "still active"
        );
        vcpu(&mut g, DIR, 42);
        assert_eq!(hyp_get(&mut g, LR0) & PENDING_ACTIVE, 0);

        let mut g = virtual_gic();
        hyp(&mut g, LR0, ACTIVE | prio(0x40) | 43);
        vcpu(&mut g, DIR, 43);
        assert_eq!(hyp_get(&mut g, LR0) & PENDING_ACTIVE, ACTIVE, "DIR ignored");
    }

    /// An interrupt both active and pending is not offered (§5.5.7), and
    /// deactivating it leaves it pending, to be signalled again.
    #[test]
    fn an_active_and_pending_interrupt_stays_pending_once_deactivated() {
        let mut g = virtual_gic();
        hyp(&mut g, LR0, PENDING_ACTIVE | prio(0x40) | 44);
        assert_eq!(vcpu_get(&mut g, HPPIR), 1023);
        assert!(!g.virtual_lines().irq);
        hyp(&mut g, APR, 1 << (0x40 >> 3));
        vcpu(&mut g, EOIR, 44);
        assert_eq!(hyp_get(&mut g, LR0) & PENDING_ACTIVE, PENDING);
        assert!(g.virtual_lines().irq);
    }

    /// A software interrupt's acknowledge reports `CPUID` from bits [12:10]
    /// (§5.5.4); a hardware one's reads zero there.
    #[test]
    fn a_software_sgi_reports_its_cpuid() {
        let mut g = virtual_gic();
        hyp(&mut g, LR0, PENDING | prio(0x40) | (5 << 10) | 3);
        assert_eq!(vcpu_get(&mut g, HPPIR), (5 << 10) | 3);
        assert_eq!(vcpu_get(&mut g, IAR), (5 << 10) | 3);
        let mut g = virtual_gic();
        hyp(&mut g, LR0, physical(40) | PENDING | prio(0x40) | 40);
        assert_eq!(vcpu_get(&mut g, IAR), 40);
    }

    /// A software interrupt with `EOI` set raises the EOI maintenance
    /// interrupt once deactivated: `GICH_EISR0`, `GICH_MISR.EOI`, and the
    /// level of INTID 25 at the distributor (Tables 5-5, 5-6; §5.2.5).
    #[test]
    fn an_eoi_maintenance_interrupt_is_raised_on_intid_25() {
        let mut g = virtual_gic();
        hyp(&mut g, LR0, LR_EOI | PENDING | prio(0x40) | 45);
        assert_eq!(vcpu_get(&mut g, IAR), 45);
        assert!(!pending_at_distributor(&mut g, MAINTENANCE_INTID));
        vcpu(&mut g, EOIR, 45);
        assert_eq!(hyp_get(&mut g, EISR0), 1);
        assert_eq!(hyp_get(&mut g, ELRSR0) & 1, 0, "not empty: its EOI waits");
        assert_eq!(hyp_get(&mut g, MISR), 1);
        assert!(pending_at_distributor(&mut g, MAINTENANCE_INTID));
        hyp(&mut g, LR0, 0);
        assert_eq!(hyp_get(&mut g, MISR), 0);
        assert!(
            !pending_at_distributor(&mut g, MAINTENANCE_INTID),
            "level-sensitive"
        );
        hyp(&mut g, LR0, LR_EOI);
        hyp(&mut g, HCR, 0);
        assert!(
            !pending_at_distributor(&mut g, MAINTENANCE_INTID),
            "none while GICH_HCR.En is clear"
        );
    }

    /// An EOI that drops a priority but finds no List register counts in
    /// `GICH_HCR.EOICount`, which with `LRENPIE` raises `MISR.LRENP`; one
    /// that drops nothing does not count; `GICV_DIR` without a List register
    /// counts (Table 5-2).
    #[test]
    fn an_eoi_without_a_list_register_is_counted() {
        let mut g = virtual_gic();
        hyp(&mut g, HCR, EN | (1 << 2));
        vcpu(&mut g, EOIR, 50);
        assert_eq!(
            hyp_get(&mut g, HCR) >> 27,
            0,
            "nothing dropped, nothing counted"
        );
        hyp(&mut g, APR, 1 << 4);
        vcpu(&mut g, EOIR, 50);
        assert_eq!(hyp_get(&mut g, HCR) >> 27, 1);
        assert_eq!(hyp_get(&mut g, MISR), 1 << 2);
        assert!(pending_at_distributor(&mut g, MAINTENANCE_INTID));
        vcpu(&mut g, CTLR, GRP0 | GRP1 | EOIMODE);
        vcpu(&mut g, DIR, 51);
        assert_eq!(hyp_get(&mut g, HCR) >> 27, 2);
        hyp(&mut g, HCR, EN | (1 << 2) | (31 << 27));
        hyp(&mut g, APR, 1);
        vcpu(&mut g, CTLR, GRP0 | GRP1);
        vcpu(&mut g, AEOIR, 52);
        assert_eq!(hyp_get(&mut g, HCR) >> 27, 0, "wraps at 31");
    }

    /// The underflow, no-pending and group maintenance interrupts (Tables
    /// 5-2 and 5-5).
    #[test]
    fn the_other_maintenance_interrupts_follow_their_conditions() {
        let mut g = virtual_gic();
        hyp(&mut g, HCR, EN | (1 << 1));
        assert_eq!(hyp_get(&mut g, MISR), 1 << 1, "underflow: no valid entry");
        hyp(&mut g, LR0, PENDING | 60);
        assert_eq!(hyp_get(&mut g, MISR), 1 << 1, "underflow: one valid entry");
        hyp(&mut g, LR0 + 4, PENDING | 61);
        assert_eq!(hyp_get(&mut g, MISR), 0);
        hyp(&mut g, HCR, EN | (1 << 3));
        assert_eq!(hyp_get(&mut g, MISR), 0, "entries pending");
        hyp(&mut g, LR0, ACTIVE | 60);
        hyp(&mut g, LR0 + 4, 0);
        assert_eq!(hyp_get(&mut g, MISR), 1 << 3, "none pending");
        hyp(&mut g, LR0, PENDING_ACTIVE | 60);
        assert_eq!(
            hyp_get(&mut g, MISR),
            1 << 3,
            "active and pending is not the pending state"
        );
        hyp(&mut g, LR0, ACTIVE | 60);
        for (hcr, ctlr, misr) in [
            (1 << 4, GRP0, 1 << 4),
            (1 << 5, GRP1, 1 << 5),
            (1 << 6, GRP1, 1 << 6),
            (1 << 7, GRP0, 1 << 7),
            (1 << 4, GRP1, 0),
        ] {
            hyp(&mut g, HCR, EN | hcr);
            vcpu(&mut g, CTLR, ctlr);
            assert_eq!(hyp_get(&mut g, MISR), misr, "HCR {hcr:#x} CTLR {ctlr:#x}");
        }
    }

    /// `GICH_VMCR` reads and writes the virtual CPU interface's state, and
    /// its binary points keep their minimums (Table 5-4).
    #[test]
    fn vmcr_is_the_virtual_cpu_interfaces_state() {
        let mut g = GicV2::with_extensions(GicExtensions::Virtualization);
        let vmcr = (0x19 << 27) | (5 << 21) | (6 << 18) | EOIMODE | (1 << 4) | FIQ_EN | GRP1;
        hyp(&mut g, VMCR, vmcr);
        assert_eq!(hyp_get(&mut g, VMCR), vmcr);
        assert_eq!(vcpu_get(&mut g, PMR), 0x19 << 3);
        assert_eq!(vcpu_get(&mut g, BPR), 5);
        assert_eq!(vcpu_get(&mut g, ABPR), 6);
        assert_eq!(vcpu_get(&mut g, CTLR), EOIMODE | (1 << 4) | FIQ_EN | GRP1);
        hyp(&mut g, VMCR, !(0x3F << 18));
        assert_eq!(vcpu_get(&mut g, BPR), 2);
        assert_eq!(vcpu_get(&mut g, ABPR), 3);
        assert_eq!(vcpu_get(&mut g, CTLR), 0x21F, "only the GICV_CTLR bits");
        assert_eq!(hyp_get(&mut g, HCR), 0);
        hyp(&mut g, HCR, 0xFFFF_FFFF);
        assert_eq!(hyp_get(&mut g, HCR), 0xF800_00FF, "reserved bits RAZ/WI");
        hyp(&mut g, LR0, 0xFFFF_FFFF);
        assert_eq!(hyp_get(&mut g, LR0), 0xFF8F_FFFF, "bits [22:20] RAZ/WI");
    }

    /// A snapshot keeps the virtual interface, and is refused by a GIC
    /// whose extensions differ, or with bits its registers cannot hold,
    /// changing nothing.
    #[test]
    fn a_snapshot_keeps_the_virtual_interface() {
        let mut g = virtual_gic();
        hyp(&mut g, LR0, LR_GRP1 | PENDING | prio(0x40) | 27);
        hyp(&mut g, LR0 + 4, ACTIVE | prio(0x50) | 28);
        hyp(&mut g, APR, 1 << (0x50 >> 3));
        assert!(g.virtual_lines().irq);
        let mut image = std::vec::Vec::new();
        g.save(&mut image).expect("save");

        let mut restored = GicV2::with_extensions(GicExtensions::Virtualization);
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restores");
        assert!(source.is_empty());
        let mut again = std::vec::Vec::new();
        restored.save(&mut again).expect("save");
        assert!(again == image);
        assert!(
            restored.virtual_lines().irq,
            "the restored interface signals"
        );
        assert_eq!(vcpu_get(&mut restored, AHPPIR), 27);

        // `GICH_HCR` opens the virtual section, 31 bytes from the end: an
        // image whose `UIE` asks for the underflow maintenance interrupt
        // its one valid List register raises, while the line it holds is
        // low, contradicts itself.
        let mut g = virtual_gic();
        hyp(&mut g, LR0, PENDING | prio(0x40) | 27);
        let mut quiet = std::vec::Vec::new();
        g.save(&mut quiet).expect("save");
        let hcr = quiet.len() - 31;
        assert_eq!(quiet[hcr], 1, "GICH_HCR.En, little-endian");
        quiet[hcr] |= 1 << 1;
        let mut source: &[u8] = &quiet;
        assert!(
            GicV2::with_extensions(GicExtensions::Virtualization)
                .restore(&mut source)
                .is_err(),
            "the maintenance line disagrees with GICH_MISR"
        );

        let mut plain = GicV2::new();
        let mut before = std::vec::Vec::new();
        plain.save(&mut before).expect("save");
        let mut source: &[u8] = &image;
        assert!(plain.restore(&mut source).is_err());
        let mut after = std::vec::Vec::new();
        plain.save(&mut after).expect("save");
        assert!(after == before, "the refused image changed nothing");
        let mut source: &[u8] = &before;
        assert!(
            GicV2::with_extensions(GicExtensions::Virtualization)
                .restore(&mut source)
                .is_err(),
            "nor does a GIC with the extensions take one without"
        );

        // The last List register's word ends the image: reserved bits set.
        let mut bad = image.clone();
        let at = bad.len() - 4;
        bad[at + 2] |= 0x70;
        let mut source: &[u8] = &bad;
        assert!(
            GicV2::with_extensions(GicExtensions::Virtualization)
                .restore(&mut source)
                .is_err()
        );
    }
}
