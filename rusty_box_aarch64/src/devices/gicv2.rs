//! GICv2 distributor and CPU interface for one processor, without the
//! Security Extensions (QEMU's `virt` without EL3), plus the GICv2m MSI
//! frame, and on a board with EL2 the Virtualization Extensions
//! ([`super::gicv2_virt`]).
//!
//! Provenance (R7): the ARM Generic Interrupt Controller Architecture
//! Specification, GIC architecture version 2.0 (IHI 0048B.b), first, with
//! the GIC-400's choices where it leaves them to the implementation (DDI
//! 0471B: PPIs 25 to 31, level-sensitive; and PPI 22, COMMIRQ's, and 23,
//! PMUIRQ's, which BSA's PPI assignments add), and its SPIs and priority bits
//! the board's to choose ([`GicShape`]: by default the GIC-400's 480 SPIs
//! and five bits), the minimum binary points and active priority registers
//! following from the priority bits (IHI 0048B Table 4-47), and its GICv2m
//! frame's SPIs the board's too ([`MsiFrame`], BSA DEN0094C I.5);
//! QEMU 11.1.2 `hw/intc/arm_gic.c` for a revision-2, single-CPU GIC second,
//! whose structure this model keeps —
//! `gic_update_internal` and `gic_get_best_irq` (the signalled interrupt),
//! `gic_get_current_pending_irq`, `gic_acknowledge_irq`, `gic_activate_irq`,
//! `gic_get_group_priority`, `gic_drop_prio`, `gic_get_prio_from_apr_bits`,
//! `gic_complete_irq`, `gic_deactivate_irq`, `gic_dist_readb`,
//! `gic_dist_writeb`/`writew`/`writel`, `gic_cpu_read` and `gic_cpu_write`
//! — with `hw/intc/gic_internal.h` (`gic_test_pending`,
//! `GICC_CTLR_V2_MASK`), `hw/intc/arm_gic_common.c`
//! (`arm_gic_common_reset_hold`, the `gic_dist` and `gic_cpu` regions) and
//! `hw/intc/arm_gicv2m.c` (`gicv2m_read`, `gicv2m_write`, `gicv2m_set_irq`).
//!
//! The highest-priority pending interrupt is chosen as IHI 0048B's
//! pseudocode chooses it (§3.7.2 `UpdateExceptionState`, §4.4.4
//! `ReadGICC_IAR`, §4.4.7 `ReadGICC_HPPIR`), where the specification leaves
//! two choices to the implementation (§3.7.1):
//! - A group the distributor does not forward (§4.3.1 GICD_CTLR.EnableGrpN)
//!   takes no part in the choice, so its interrupts are neither signalled
//!   nor read and do not hide the other group's: `IGNORE_GROUP_ENABLE`
//!   FALSE, the distributor's enables masking before prioritisation. QEMU
//!   11.1.2 (`gic_update_internal`) masks after it, the other choice.
//! - The CPU interface's group enables (§4.4.1 GICC_CTLR.EnableGrpN) take
//!   no part in the choice: they decide whether the interrupt chosen is
//!   signalled and acknowledged, so a higher-priority interrupt of a group
//!   the CPU interface does not signal hides the other group's, as in QEMU.
//!   §4.1.5 makes one exception, which QEMU does not: with AckCtl set,
//!   EnableGrp0 clear and EnableGrp1 set, "Group 0 interrupts are ignored
//!   and GICC_IAR behaves as GICC_AIAR". The pseudocode does not make it
//!   (`ReadGICC_IAR` returns 1023 for a highest pending Group 0 interrupt
//!   whatever AckCtl); the prose, which names the case, is taken over the
//!   pseudocode, which does not. Arm deprecates AckCtl set (§4.1.5).
//!   GICC_HPPIR and GICC_AHPPIR still report it: `GICC_MASK_HPPIR` FALSE,
//!   as in the GIC-400 ("even when the appropriate GICC_CTLR.EnableGrp bit
//!   is set to 0", DDI 0471B Table 3-6, note c).
//!
//! Where IHI 0048B and QEMU differ, this model follows IHI 0048B:
//! - A Group 1 interrupt reads as 1022 at GICC_IAR only when it is of
//!   "sufficient priority to be signaled" (Table 4-35, note a), where
//!   QEMU's `gic_acknowledge_irq` returns 1022 before comparing priorities.
//! - A pending interrupt preempts when its group priority is higher than
//!   the running priority's, both under the binary point of the pending
//!   interrupt's group (§3.3.3; §3.7.2 `UpdateExceptionState`:
//!   `PriorityIsHigher(priority, GICC_RPR AND mask)`), where QEMU compares
//!   the whole priority with the running priority.
//! - The Group 1 aliases GICC_AIAR, GICC_AEOIR and GICC_AHPPIR (Table 4-2,
//!   GICv2 only; §4.4.1 AckCtl) acknowledge, complete and report Group 1
//!   interrupts, as QEMU's Non-secure accesses to GICC_IAR, GICC_EOIR and
//!   GICC_HPPIR do.
//! - This is a uniprocessor implementation (GICD_TYPER.CPUNumber = 0): an SGI
//!   has one possible source, processor 0, so GICD_CPENDSGIRn and
//!   GICD_SPENDSGIRn hold a bit for that source only (a bit per supported
//!   SGI and source processor combination, a write of 0 having no effect),
//!   and GICC_IAR.CPUID is RAZ (§4.4.4: it identifies an SGI's source only
//!   in a multiprocessor implementation).
//!
//! With one processor every per-CPU mask QEMU keeps reduces to its bit 0,
//! and every interrupt targets that processor, so each per-interrupt state
//! is one bit, and an SGI's single source is its pending bit.
//!
//! The regions take any access size (`gic_ops` sets no `impl` limits): a
//! distributor access is composed of byte accesses, a CPU-interface access
//! reads or writes the register at its exact offset, and an 8-byte access
//! is split by QEMU's `access_with_adjusted_size` into two 4-byte ones, at
//! `offset` and then `offset + 4`.
//!
//! Interrupt state is recomputed after every register access and every
//! input line change; the machine reads `irq_out`/`fiq_out`, and with the
//! Virtualization Extensions `virtual_lines`, between instructions.

use rusty_box_core::SnapError;
use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};

use super::gicv2_virt::{HwDeactivation, MAINTENANCE_INTID, ReadEffect, VirtInterface};
use crate::bus::{AccessWidth, RegOffset, read_by_words, write_by_words};
use crate::cpu::VirtualLines;
use crate::identity::{GicIdentity, MsiFrameIidr};
use crate::interrupt_map::{InterruptMap, MsiCount, Spi};
use crate::layout::DeviceLayout;

/// SGIs, PPIs and the most SPIs a GIC has, [`SpiCount::MAX`]: the size of
/// every per-interrupt bank, whatever the GIC's [`GicShape`] implements.
pub const MAX_IRQ: usize = 32 + SpiCount::MAX.get() as usize;
pub(super) const WORDS: usize = MAX_IRQ / 32;
/// QEMU `GIC_NR_SGIS`.
pub(super) const NR_SGIS: usize = 16;
/// The PPIs the GIC implements, a bit each in the word of IDs 0 to 31:
/// 25 to 31, the GIC-400's ("all interrupts are implemented except for the
/// unused PPIs, IDs 16-24", DDI 0471B §3.3); 22, the PPI of the DCC's
/// COMMIRQ, which BSA recommends (DEN0094C RB_PPI_01) and V-BSA requires
/// to be a PPI (DEN0150 RV_L1PP_00); and 23, the PMU's PMUIRQ, which BSA
/// recommends (RB_PPI_01) for the PMU V-BSA requires (RV_L1PE_02) — the
/// GIC-400 has neither. Every field of an interrupt that does not exist is
/// RAZ/WI (IHI 0048B §4.3.13 and the other registers').
const IMPLEMENTED_PPIS: u32 = 0xFE00_0000 | (1 << 22) | (1 << 23);

/// How many SPIs a GIC has: a multiple of 32 (`GICD_TYPER.ITLinesNumber`
/// counts them in 32s, IHI 0048B §4.3.2), at most 480, the GIC-400's
/// largest configuration ("permitted values are 0-480, in steps of 32",
/// DDI 0471B §2.3.3), and at least [`SpiCount::MIN`], room for the board's
/// interrupt sources; whether a particular map fits is
/// [`InterruptMap::check`]'s to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpiCount(u16);

impl SpiCount {
    /// The count a board's GIC has unless its host chooses another: the
    /// GIC-400's largest.
    pub const DEFAULT: Self = Self::MAX;
    /// The GIC-400's largest configuration.
    pub const MAX: Self = Self(480);
    /// The fewest any interrupt map fits: an SPI for each of the board's
    /// device lines on the layout of the fewest virtio transports
    /// ([`InterruptMap::device_lines`]) and one for its GICv2m frame,
    /// rounded up to the next multiple of 32. The board's own map
    /// ([`InterruptMap::DEFAULT`]) starts its frame at SPI 32, so it needs
    /// 64.
    pub const MIN: Self = Self(
        ((InterruptMap::device_lines(DeviceLayout::fewest_transports()) + 1).div_ceil(32) * 32)
            as u16,
    );

    /// `spis`, if it is a multiple of 32 from [`SpiCount::MIN`] to
    /// [`SpiCount::MAX`].
    pub const fn new(spis: u32) -> Option<Self> {
        if spis.is_multiple_of(32) && spis >= Self::MIN.0 as u32 && spis <= Self::MAX.0 as u32 {
            Some(Self(spis as u16))
        } else {
            None
        }
    }

    /// The count.
    pub const fn get(self) -> u32 {
        self.0 as u32
    }

    /// The interrupt IDs below the first that does not exist: the SGIs and
    /// PPIs, and these SPIs.
    pub(super) const fn irq_end(self) -> usize {
        32 + self.0 as usize
    }
}

impl Default for SpiCount {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How many priority bits a GIC implements, the top ones of each priority
/// field, the others RAZ/WI (IHI 0048B §3.3, Table 3-1: "IMPLEMENTATION
/// DEFINED in the range 4-8"). They decide the minimum binary points and
/// how many preemption levels the active priority registers hold (Table
/// 4-47). The physical GIC's are this choice; the virtual interface's five
/// ([`super::gicv2_virt`]) are the architecture's own (§5.5.2, §5.5.3).
///
/// Closed (R5): the five counts Table 3-1 allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PriorityBits {
    Four,
    /// The GIC-400's, where it has no Non-secure view of them (DDI 0471B
    /// §2.6, "32 priority levels in Secure state").
    Five,
    Six,
    Seven,
    Eight,
}

impl PriorityBits {
    /// Every count, fewest first.
    pub const ALL: [Self; 5] = [Self::Four, Self::Five, Self::Six, Self::Seven, Self::Eight];
    /// The count a board's GIC has unless its host chooses another: the
    /// GIC-400's.
    pub const DEFAULT: Self = Self::Five;

    /// The count, 4 to 8.
    pub const fn count(self) -> u8 {
        match self {
            Self::Four => 4,
            Self::Five => 5,
            Self::Six => 6,
            Self::Seven => 7,
            Self::Eight => 8,
        }
    }

    /// The count `bits` names, if Table 3-1 allows it.
    pub const fn new(bits: u8) -> Option<Self> {
        match bits {
            4 => Some(Self::Four),
            5 => Some(Self::Five),
            6 => Some(Self::Six),
            7 => Some(Self::Seven),
            8 => Some(Self::Eight),
            _ => None,
        }
    }

    /// The priority bits that exist (Table 3-1, Table 4-32).
    pub(super) const fn mask(self) -> u8 {
        (0xFF00_u16 >> self.count()) as u8
    }

    /// The bits of a group priority: the priority bits, but for the lowest
    /// of eight, which the least binary point, 0, leaves to the
    /// subpriority (§3.3.3, Table 3-2).
    const fn group_bits(self) -> u32 {
        let count = self.count() as u32;
        if count < 7 { count } else { 7 }
    }

    /// `GICC_BPR`'s minimum binary point: the one whose group priority is
    /// every group bit (Table 4-47: 3 with four bits, 2 with five, 1 with
    /// six, 0 with seven), and 0 with eight, the least there is (§4.4.3).
    /// A write below it sets it (§4.4.3), and a reset sets it.
    pub(super) const fn min_bpr(self) -> u8 {
        (7 - self.group_bits()) as u8
    }

    /// `GICC_ABPR`'s minimum binary point, one above `GICC_BPR`'s (§3.3.3).
    pub(super) const fn min_abpr(self) -> u8 {
        self.min_bpr() + 1
    }

    /// The preemption levels, a bit each of the active priority registers
    /// (Table 4-47: 16, 32, 64, 128).
    const fn preemption_levels(self) -> u32 {
        1 << self.group_bits()
    }

    /// The bits of `GICC_APRn` and `GICC_NSAPRn` word `n` that hold a
    /// preemption level, in Table 4-47's layout (`GICC_APR0[15:0]` with
    /// four bits, `GICC_APR0` with five, `GICC_APR0` and `GICC_APR1` with
    /// six, all four from seven); the other bits and words are reserved.
    pub(super) const fn apr_bits(self, n: usize) -> u32 {
        let levels = self.preemption_levels() as usize;
        if levels >= 32 * (n + 1) {
            u32::MAX
        } else if levels > 32 * n {
            (1 << (levels - 32 * n)) - 1
        } else {
            0
        }
    }
}

impl Default for PriorityBits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The shape of a GIC that IHI 0048B leaves to the implementation and a
/// board chooses: how many SPIs it has and how many priority bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GicShape {
    pub spis: SpiCount,
    pub priority_bits: PriorityBits,
}

impl GicShape {
    /// The GIC-400's: 480 SPIs and five priority bits.
    pub const DEFAULT: Self = Self {
        spis: SpiCount::DEFAULT,
        priority_bits: PriorityBits::DEFAULT,
    };

    /// Whether interrupt `irq` exists: an SGI, a PPI of
    /// [`IMPLEMENTED_PPIS`], or one of the GIC's SPIs.
    pub(super) const fn implemented(self, irq: usize) -> bool {
        if irq < 32 {
            irq < NR_SGIS || IMPLEMENTED_PPIS & (1 << irq) != 0
        } else {
            irq < self.spis.irq_end()
        }
    }

    /// Interrupt ID `intid` as an index into the GIC's state, if the
    /// interrupt exists: the one gate of every path that takes an ID from a
    /// register (`GICC_EOIR`, `GICC_DIR`, a hardware List register) or an
    /// input line, so none reaches the state of an interrupt that does not
    /// exist.
    pub(super) const fn existing(self, intid: u32) -> Option<usize> {
        let i = intid as usize;
        if self.implemented(i) { Some(i) } else { None }
    }

    /// The interrupts of word `w` of a bank that exist, a bit each
    /// ([`GicShape::implemented`]).
    pub(super) const fn implemented_word(self, w: usize) -> u32 {
        let mut word = 0;
        let mut k = 0;
        while k < 32 {
            if self.implemented(w * 32 + k) {
                word |= 1 << k;
            }
            k += 1;
        }
        word
    }

    /// The words of a bank that hold an interrupt that exists.
    pub(super) const fn words(self) -> usize {
        self.spis.irq_end() / 32
    }
}

impl Default for GicShape {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The SGIs' bits in word 0 of a bank.
pub(super) const SGI_BITS: u32 = (1 << NR_SGIS) - 1;
/// QEMU `GIC_MAXIRQ`: interrupt IDs from here up are special.
pub(super) const MAXIRQ: u32 = 1020;
pub(super) const SPURIOUS: u32 = 1023;
pub(super) const SPURIOUS_GROUP1: u32 = 1022;
/// QEMU `running_priority` while nothing is active: above every priority.
pub(super) const IDLE_PRIORITY: u16 = 0x100;
/// The most active priority registers of a group: four, of 128 preemption
/// levels (IHI 0048B §4.4.12, Table 4-47).
pub(super) const MAX_APRS: usize = 4;

/// `GICD_CTLR`'s and `GICC_CTLR`'s group enables, the same bits in both.
pub(super) const GICD_CTLR_EN_GRP0: u32 = 1 << 0;
pub(super) const GICD_CTLR_EN_GRP1: u32 = 1 << 1;
pub(super) const GICC_CTLR_ACK_CTL: u32 = 1 << 2;
pub(super) const GICC_CTLR_FIQ_EN: u32 = 1 << 3;
pub(super) const GICC_CTLR_CBPR: u32 = 1 << 4;
pub(super) const GICC_CTLR_EOIMODE: u32 = 1 << 9;
/// QEMU `GICC_CTLR_V2_MASK`: the bits a GICv2 without the Security
/// Extensions keeps.
pub(super) const GICC_CTLR_V2_MASK: u32 = 0x21F;
/// The SGI source bit of processor 0, the one processor there is, in a
/// `GICD_CPENDSGIRn`/`GICD_SPENDSGIRn` byte.
const SGI_SOURCE_CPU0: u8 = 1;

/// The two register sets of the CPU interface (IHI 0048B §4.4.1 AckCtl,
/// Table 4-2).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CpuView {
    /// `GICC_IAR`, `GICC_EOIR` and `GICC_HPPIR`: Group 0, and Group 1 while
    /// `AckCtl` is set — as a Secure access in QEMU.
    Primary,
    /// `GICC_AIAR`, `GICC_AEOIR` and `GICC_AHPPIR`: Group 1 only — as a
    /// Non-secure access to the primary registers in QEMU.
    Group1Alias,
}

/// How `view` reads interrupt `id` of Group 1 (`group1`) or Group 0, on a
/// CPU interface physical or virtual (IHI 0048B §5.4: a virtual machine
/// cannot tell the two apart) whose `AckCtl` is `ack_ctl`: through the
/// primary registers a Group 1 interrupt reads as 1022 unless `AckCtl` is
/// set (§4.4.1 AckCtl; Tables 4-35, 5-12 and 5-14); through the Group 1
/// aliases a Group 0 interrupt reads as 1023 (Tables 5-15 and 5-17), as for
/// QEMU's Non-secure access (`gic_get_current_pending_irq`).
pub(super) const fn read_through(view: CpuView, group1: bool, ack_ctl: bool, id: u32) -> u32 {
    match view {
        CpuView::Primary if group1 && !ack_ctl => SPURIOUS_GROUP1,
        CpuView::Group1Alias if !group1 => SPURIOUS,
        CpuView::Primary | CpuView::Group1Alias => id,
    }
}

/// The enable bit of Group 1 (`group1`) or Group 0 in `GICD_CTLR`,
/// `GICC_CTLR` and `GICV_CTLR`, the same bit in all three.
pub(super) const fn group_enable(group1: bool) -> u32 {
    if group1 {
        GICD_CTLR_EN_GRP1
    } else {
        GICD_CTLR_EN_GRP0
    }
}

/// The mask of a group priority's bits for an interrupt of Group 1
/// (`group1`) or Group 0, on a CPU interface physical or virtual: the bits
/// above the binary point of its group, `abpr` minus one for Group 1 unless
/// `cbpr` is set, `bpr` otherwise (IHI 0048B §3.3 "Priority grouping",
/// §3.7.2 `GIC_PriorityMask`, §5.5.3; QEMU `gic_get_group_priority`).
pub(super) const fn group_mask(group1: bool, cbpr: bool, bpr: u8, abpr: u8) -> u8 {
    let point = if group1 && !cbpr { abpr - 1 } else { bpr };
    (0xFFu32 << ((point & 7) + 1)) as u8
}

/// The group priority of an interrupt of `priority` under `mask`
/// ([`group_mask`]).
pub(super) const fn group_priority(priority: u8, mask: u8) -> u8 {
    priority & mask
}

/// Whether a pending interrupt of `priority`, whose group's mask is `mask`,
/// preempts the running priority `running`: always while nothing is
/// active, and otherwise if its group priority is higher than the running
/// priority's under the same mask (IHI 0048B §3.3.3: "the value of the
/// group priority field for the new interrupt must be less than the value
/// of the group priority field of the Running priority"; §3.7.2
/// `UpdateExceptionState`: `PriorityIsHigher(priority, GICC_RPR AND
/// mask)`).
pub(super) const fn preempts(priority: u8, mask: u8, running: u16) -> bool {
    running >= IDLE_PRIORITY || priority & mask < running as u8 & mask
}

/// Whether the GIC implements the Virtualization Extensions (IHI 0048B
/// chapter 5): on a board whose processor has EL2, as QEMU's `virt` gives
/// its GICv2 them under `virtualization=on`
/// (`has-virtualization-extensions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GicExtensions {
    /// None of them: the GIC of a board whose processor has no EL2.
    Absent,
    Virtualization,
}

/// QEMU `gic_id_gicv2`, for offsets 0xFD0..=0xFFC.
const PERIPHERAL_ID: [u8; 12] = [
    0x04, 0x00, 0x00, 0x00, 0x90, 0xB4, 0x2B, 0x00, 0x0D, 0xF0, 0x05, 0xB1,
];

/// The SPIs a GICv2m frame is given (BSA DEN0094C I.5): `count`
/// contiguous ones from `first`, which `MSI_TYPER` reports (I.9.1) and an
/// `MSI_SETSPI_NS` write must name (I.9.2). The board's [`InterruptMap`]
/// decides them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MsiFrame {
    pub first: Spi,
    pub count: MsiCount,
}

impl MsiFrame {
    /// `MSI_TYPER`: the first SPI's interrupt ID in bits [25:16], the count
    /// in [9:0], both of which fit ([`Spi::LAST`], [`MsiCount::MAX`]).
    pub(super) const fn typer(self) -> u32 {
        (self.first.intid() << 16) | self.count.get()
    }

    /// Whether interrupt `intid` is one of the frame's.
    pub(super) const fn holds(self, intid: u32) -> bool {
        let first = self.first.intid();
        intid >= first && intid - first < self.count.get()
    }
}

/// A GICv2m frame whose SPIs are not all a GIC's
/// ([`GicV2::with_msi_frame`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MsiFrameOutsideGic {
    pub frame: MsiFrame,
    pub spis: SpiCount,
}

impl core::fmt::Display for MsiFrameOutsideGic {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "a GICv2m frame of {} SPIs from {} runs past the last of a GIC of {} SPIs",
            self.frame.count.get(),
            self.frame.first,
            self.spis.get()
        )
    }
}

impl core::error::Error for MsiFrameOutsideGic {}

const V2M_MSI_TYPER: u64 = 0x008;
pub(super) const V2M_MSI_SETSPI_NS: u64 = 0x040;
const V2M_MSI_IIDR: u64 = 0xFCC;

/// One bit per interrupt, interrupt `i` bit `i % 32` of word `i / 32`.
#[derive(Clone, Copy)]
pub(super) struct Bits(pub(super) [u32; WORDS]);

impl Bits {
    pub(super) const fn new() -> Self {
        Self([0; WORDS])
    }

    #[inline]
    pub(super) fn get(&self, i: usize) -> bool {
        self.0[i / 32] & (1 << (i % 32)) != 0
    }

    #[inline]
    pub(super) fn set(&mut self, i: usize, v: bool) {
        if v {
            self.0[i / 32] |= 1 << (i % 32);
        } else {
            self.0[i / 32] &= !(1 << (i % 32));
        }
    }

    /// Bits `first..first + 8` as a byte, where `first` is a multiple of 8
    /// (a byte of a one-bit-per-interrupt bank); a byte past the last
    /// interrupt reads as zero.
    #[inline]
    pub(super) fn byte(&self, first: usize) -> u8 {
        self.0
            .get(first / 32)
            .map_or(0, |word| (word >> (first % 32)) as u8)
    }
}

/// The interrupt controller.
pub struct GicV2 {
    /// `GICD_CTLR`.
    dist_ctlr: u32,
    enabled: Bits,
    /// QEMU `irq_state[].pending`: set by an edge, by software or by an SGI,
    /// cleared by acknowledgement or software. A level-sensitive interrupt is
    /// also pending while its line is high (`gic_test_pending`).
    pending: Bits,
    active: Bits,
    group1: Bits,
    /// The level of each input line.
    level: Bits,
    /// Edge-triggered (`GICD_ICFGR` bit 2n+1).
    edge: Bits,
    priority: [u8; MAX_IRQ],
    /// `GICC_CTLR`.
    cpu_ctlr: u32,
    pmr: u8,
    bpr: u8,
    abpr: u8,
    /// QEMU `running_priority`: the group priority of the highest-priority
    /// active interrupt, or [`IDLE_PRIORITY`].
    running_priority: u16,
    /// QEMU `apr`: one bit per active Group 0 preemption level, in the
    /// words and bits the shape's priority bits have
    /// ([`PriorityBits::apr_bits`]).
    apr: [u32; MAX_APRS],
    /// QEMU `nsapr`: one bit per active Group 1 preemption level.
    nsapr: [u32; MAX_APRS],
    /// QEMU `current_pending`, recomputed by [`GicV2::update`].
    current_pending: u32,
    /// The interrupt the CPU interface signals and `GICC_IAR` would
    /// acknowledge, recomputed by [`GicV2::update`]; 1023 while none is.
    signalled: u32,
    irq_out: bool,
    fiq_out: bool,
    /// The virtual interface, with the Virtualization Extensions.
    virt: Option<VirtInterface>,
    /// Who the GIC says made it, and its GICv2m frame: the board's
    /// identity, kept across a reset and a restore.
    names: GicNames,
    /// The SPIs and priority bits it has, the board's choice, kept across
    /// a reset and a restore.
    shape: GicShape,
    /// The GICv2m frame's SPIs, the board's choice, kept across a reset, a
    /// change of shape and a restore; none until a board gives it some
    /// ([`GicV2::with_msi_frame`]), so `MSI_TYPER` reads zero and every
    /// `MSI_SETSPI_NS` write is ignored.
    msi: Option<MsiFrame>,
}

/// What the GIC's identification registers read: `GICD_IIDR`, `GICC_IIDR`
/// and `GICV_IIDR` ([`GicIdentity`]), and the GICv2m frame's `MSI_IIDR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GicNames {
    pub gic: GicIdentity,
    pub msi_frame: MsiFrameIidr,
}

impl GicNames {
    /// No product and no implementer anywhere.
    pub const NONE: Self = Self {
        gic: GicIdentity::NONE,
        msi_frame: MsiFrameIidr::NONE,
    };
}

impl Default for GicV2 {
    fn default() -> Self {
        Self::new()
    }
}

impl GicV2 {
    /// A GIC out of reset with `extensions`, of the GIC-400's shape.
    pub fn with_extensions(extensions: GicExtensions) -> Self {
        Self::out_of_reset(extensions, GicShape::DEFAULT, GicNames::NONE, None)
    }

    /// This GIC with its identification registers reading as `names` says.
    pub fn identified_as(self, names: GicNames) -> Self {
        Self { names, ..self }
    }

    /// This GIC with `msi` its GICv2m frame's SPIs, refused when they are
    /// not all the GIC's.
    pub fn with_msi_frame(self, msi: MsiFrame) -> Result<Self, MsiFrameOutsideGic> {
        let last = msi.first.intid() + msi.count.get() - 1;
        if (last as usize) < self.shape.spis.irq_end() {
            Ok(self.with_checked_msi_frame(msi))
        } else {
            Err(MsiFrameOutsideGic {
                frame: msi,
                spis: self.shape.spis,
            })
        }
    }

    /// This GIC with `msi` its GICv2m frame's SPIs, which the caller has
    /// found all the GIC's: the board, whose interrupt map was checked
    /// against the GIC's SPIs (`InterruptMap::wire`).
    pub(crate) fn with_checked_msi_frame(self, msi: MsiFrame) -> Self {
        Self {
            msi: Some(msi),
            ..self
        }
    }

    /// The GICv2m frame's SPIs, if it has any.
    pub fn msi_frame(&self) -> Option<MsiFrame> {
        self.msi
    }

    /// A GIC out of reset with this one's extensions and identity and the
    /// SPIs and priority bits `shape` gives it. It is a new GIC, not this
    /// one changed: nothing of this one's state is kept, so no state of
    /// another shape can outlive it — its GICv2m frame, chosen for the
    /// shape it had, among it ([`GicV2::with_msi_frame`] gives it one).
    pub fn shaped(self, shape: GicShape) -> Self {
        Self::out_of_reset(self.extensions(), shape, self.names, None)
    }

    /// The SPIs and priority bits this GIC has.
    pub fn shape(&self) -> GicShape {
        self.shape
    }

    /// A reset: the GIC out of reset with the extensions, the shape, the
    /// identity and the GICv2m frame it has.
    pub fn reset(&mut self) {
        *self = Self::out_of_reset(self.extensions(), self.shape, self.names, self.msi);
    }

    /// The extensions this GIC implements.
    pub fn extensions(&self) -> GicExtensions {
        if self.virt.is_some() {
            GicExtensions::Virtualization
        } else {
            GicExtensions::Absent
        }
    }

    /// A GIC out of reset with no Virtualization Extensions
    /// ([`GicV2::with_extensions`]), CPU interfaces of no product or
    /// implementer ([`GicV2::identified_as`]), and the GIC-400's shape
    /// ([`GicV2::shaped`]).
    pub fn new() -> Self {
        Self::with_extensions(GicExtensions::Absent)
    }

    /// The one constructor of a GIC out of reset, QEMU
    /// `arm_gic_common_reset_hold`: SGIs enabled and edge-triggered, every
    /// other interrupt disabled, level-sensitive, Group 0 and at priority 0,
    /// with `extensions`, of `shape`, named `names`, its GICv2m frame's
    /// SPIs `msi`, its binary points at their reset values, the minimum the
    /// priority bits allow (IHI 0048B §4.4.3).
    fn out_of_reset(
        extensions: GicExtensions,
        shape: GicShape,
        names: GicNames,
        msi: Option<MsiFrame>,
    ) -> Self {
        let virt = match extensions {
            GicExtensions::Absent => None,
            GicExtensions::Virtualization => Some(VirtInterface::new()),
        };
        let mut enabled = Bits::new();
        let mut edge = Bits::new();
        for sgi in 0..NR_SGIS {
            enabled.set(sgi, true);
            edge.set(sgi, true);
        }
        Self {
            dist_ctlr: 0,
            enabled,
            pending: Bits::new(),
            active: Bits::new(),
            group1: Bits::new(),
            level: Bits::new(),
            edge,
            priority: [0; MAX_IRQ],
            cpu_ctlr: 0,
            pmr: 0,
            bpr: shape.priority_bits.min_bpr(),
            abpr: shape.priority_bits.min_abpr(),
            running_priority: IDLE_PRIORITY,
            apr: [0; MAX_APRS],
            nsapr: [0; MAX_APRS],
            current_pending: SPURIOUS,
            signalled: SPURIOUS,
            irq_out: false,
            fiq_out: false,
            virt,
            names,
            shape,
            msi,
        }
    }

    /// The virtual IRQ and FIQ outputs to the processor (Arm
    /// `VirtualIRQPending()`, `VirtualFIQPending()`), which only the
    /// Virtualization Extensions drive. They follow from the virtual
    /// interface's registers alone, so they are computed where they are
    /// read.
    #[inline]
    pub fn virtual_lines(&self) -> VirtualLines {
        self.virt
            .as_ref()
            .map_or(VirtualLines::NONE, VirtInterface::outputs)
    }

    /// The IRQ output to the processor.
    #[inline]
    pub fn irq_out(&self) -> bool {
        self.irq_out
    }

    /// The FIQ output to the processor.
    #[inline]
    pub fn fiq_out(&self) -> bool {
        self.fiq_out
    }

    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_u32(self.dist_ctlr)?;
        w.write_u32(self.cpu_ctlr)?;
        w.write_bytes(&[self.pmr, self.bpr, self.abpr])?;
        w.write_u16(self.running_priority)?;
        for word in self.apr.iter().chain(self.nsapr.iter()) {
            w.write_u32(*word)?;
        }
        for bits in [
            &self.enabled,
            &self.pending,
            &self.active,
            &self.group1,
            &self.level,
            &self.edge,
        ] {
            for word in bits.0 {
                w.write_u32(word)?;
            }
        }
        w.write_bytes(&self.priority)?;
        // The per-SGI source bytes repeat what `pending` holds (processor
        // 0's bit while the SGI is pending). They are part of the GIC's
        // section of the image, and `restore` refuses bytes that disagree
        // with `pending`.
        let sources: [u8; NR_SGIS] = core::array::from_fn(|sgi| self.sgi_sources(sgi));
        w.write_bytes(&sources)?;
        match &self.virt {
            None => w.write_u8(0),
            Some(virt) => {
                w.write_u8(1)?;
                virt.save(w)
            }
        }
    }

    /// SGI `sgi`'s `GICD_CPENDSGIRn`/`GICD_SPENDSGIRn` byte: the source bit
    /// of processor 0 while the SGI is pending.
    fn sgi_sources(&self, sgi: usize) -> u8 {
        if self.pending.get(sgi) {
            SGI_SOURCE_CPU0
        } else {
            0
        }
    }

    /// Reads the GIC's whole section of the image, refuses it if
    /// [`GicV2::check_restored`] does, and only then replaces this GIC's
    /// state: a refused or truncated section changes nothing here.
    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let dist_ctlr = r.read_u32()?;
        let cpu_ctlr = r.read_u32()?;
        let mut points = [0u8; 3];
        r.read_bytes(&mut points)?;
        let [pmr, bpr, abpr] = points;
        let running_priority = r.read_u16()?;
        let mut apr = [0u32; MAX_APRS];
        let mut nsapr = [0u32; MAX_APRS];
        for word in apr.iter_mut().chain(nsapr.iter_mut()) {
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
        let mut sgi_sources = [0u8; NR_SGIS];
        r.read_bytes(&mut sgi_sources)?;
        let virt = match r.read_u8()? {
            0 => None,
            1 => Some(VirtInterface::restore(r)?),
            _ => return Err(SnapError::Invalid("GIC extensions tag out of range")),
        };
        if virt.is_some() != self.virt.is_some() {
            return Err(SnapError::Invalid(
                "GIC image's Virtualization Extensions differ from the board's",
            ));
        }
        let restored = Self {
            dist_ctlr,
            enabled,
            pending,
            active,
            group1,
            level,
            edge,
            priority,
            cpu_ctlr,
            pmr,
            bpr,
            abpr,
            running_priority,
            apr,
            nsapr,
            current_pending: SPURIOUS,
            signalled: SPURIOUS,
            irq_out: false,
            fiq_out: false,
            virt,
            names: self.names,
            shape: self.shape,
            msi: self.msi,
        };
        restored.check_restored(&sgi_sources)?;
        *self = restored;
        self.update();
        self.sync_virtual();
        Ok(())
    }

    /// The one check of a restored image, made before it replaces anything:
    /// refuses binary points the registers cannot hold, a priority or a
    /// priority mask with an unimplemented bit set, an active priority of a
    /// preemption level the priority bits do not give, state of an interrupt
    /// that does not exist, an SGI that is disabled, not edge-triggered or
    /// with its line high, a PPI that is not level-sensitive, a running
    /// priority other than the one its active priority registers give (the
    /// highest active preemption level, or idle), an SGI source bit of a
    /// processor other than processor 0, SGI source bytes that disagree with
    /// the SGI's pending state (a source with the SGI not pending, or the
    /// SGI pending with no source), and with the Virtualization Extensions a
    /// maintenance interrupt line whose level is not the one the virtual
    /// interface gives.
    fn check_restored(&self, sgi_sources: &[u8; NR_SGIS]) -> SnapResult {
        let shape = self.shape;
        let bits = shape.priority_bits;
        if !(bits.min_bpr()..=7).contains(&self.bpr) || !(bits.min_abpr()..=7).contains(&self.abpr)
        {
            return Err(SnapError::Invalid(
                "GIC binary point outside its register's range",
            ));
        }
        let mask = bits.mask();
        if self.pmr & !mask != 0 || self.priority.iter().any(|p| p & !mask != 0) {
            return Err(SnapError::Invalid("GIC priority with an unimplemented bit"));
        }
        if (0..MAX_APRS).any(|n| (self.apr[n] | self.nsapr[n]) & !bits.apr_bits(n) != 0) {
            return Err(SnapError::Invalid(
                "GIC active priority of a preemption level that does not exist",
            ));
        }
        let absent = |bits: &Bits| (0..WORDS).any(|w| bits.0[w] & !shape.implemented_word(w) != 0);
        if [
            &self.enabled,
            &self.pending,
            &self.active,
            &self.group1,
            &self.level,
            &self.edge,
        ]
        .into_iter()
        .any(absent)
            || (0..MAX_IRQ).any(|i| !shape.implemented(i) && self.priority[i] != 0)
        {
            return Err(SnapError::Invalid(
                "GIC state of an interrupt that does not exist",
            ));
        }
        // SGIs are always enabled and edge-triggered and have no input line;
        // PPIs are level-sensitive.
        if self.edge.0[0] != SGI_BITS
            || self.enabled.0[0] & SGI_BITS != SGI_BITS
            || self.level.0[0] & SGI_BITS != 0
        {
            return Err(SnapError::Invalid(
                "GIC SGI or PPI configuration its registers cannot hold",
            ));
        }
        if self.running_priority != self.prio_from_apr_bits() {
            return Err(SnapError::Invalid(
                "GIC running priority disagrees with its active priorities",
            ));
        }
        for (sgi, sources) in sgi_sources.iter().enumerate() {
            if sources & !SGI_SOURCE_CPU0 != 0 {
                return Err(SnapError::Invalid(
                    "GIC SGI pending from a processor that does not exist",
                ));
            }
            if *sources != self.sgi_sources(sgi) {
                return Err(SnapError::Invalid(
                    "GIC SGI source disagrees with its pending state",
                ));
            }
        }
        // The virtual interface alone drives the maintenance interrupt's
        // line, so its level is the one the interface's state gives.
        if let Some(virt) = &self.virt {
            if self.level.get(MAINTENANCE_INTID as usize) != virt.maintenance() {
                return Err(SnapError::Invalid(
                    "GIC maintenance interrupt line disagrees with the virtual interface",
                ));
            }
        }
        Ok(())
    }

    /// Drive input line `intid` (a PPI or SPI that exists) to `level`: QEMU
    /// `gic_set_irq` with `gic_set_irq_generic`.
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

    /// QEMU `gic_test_pending`.
    #[inline]
    fn test_pending(&self, i: usize) -> bool {
        self.pending.get(i) || (!self.edge.get(i) && self.level.get(i))
    }

    /// QEMU `gic_get_best_irq`: the enabled, pending, inactive interrupt with
    /// the numerically lowest priority, the lowest ID among equals — among
    /// the interrupts of `groups` (`GICD_CTLR_EN_GRP0`/`GICD_CTLR_EN_GRP1`
    /// bits) only.
    fn best_irq(&self, groups: u32) -> Option<usize> {
        let group0 = if groups & GICD_CTLR_EN_GRP0 != 0 {
            u32::MAX
        } else {
            0
        };
        let group1 = if groups & GICD_CTLR_EN_GRP1 != 0 {
            u32::MAX
        } else {
            0
        };
        let mut best: Option<usize> = None;
        let mut best_prio = IDLE_PRIORITY;
        for w in 0..self.shape.words() {
            let pending = self.pending.0[w] | (self.level.0[w] & !self.edge.0[w]);
            let in_groups = (self.group1.0[w] & group1) | (!self.group1.0[w] & group0);
            let mut candidates = self.enabled.0[w] & pending & !self.active.0[w] & in_groups;
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

    /// Whether the CPU interface signals interrupt `irq`'s group (IHI 0048B
    /// §4.4.1 GICC_CTLR.EnableGrp0/EnableGrp1).
    #[inline]
    fn cpu_signals(&self, irq: usize) -> bool {
        self.cpu_ctlr & group_enable(self.group1.get(irq)) != 0
    }

    /// Whether interrupt `irq` preempts the running priority, under its own
    /// group's binary point ([`preempts`]).
    #[inline]
    fn preempts(&self, irq: usize) -> bool {
        preempts(
            self.priority[irq],
            self.group_mask(irq),
            self.running_priority,
        )
    }

    /// The groups the CPU interface chooses its interrupt among (IHI 0048B
    /// §4.1.5): both, its group enables deciding only whether the interrupt
    /// chosen is signalled, but for `AckCtl` set with Group 0 disabled and
    /// Group 1 enabled, where "Group 0 interrupts are ignored and GICC_IAR
    /// behaves as GICC_AIAR". `ctlr` is `GICC_CTLR`, or `GICV_CTLR`, whose
    /// bits are the same (§5.4: a virtual machine cannot tell its CPU
    /// interface from a physical one).
    pub(super) const fn considered_groups(ctlr: u32) -> u32 {
        let group1_only = GICD_CTLR_EN_GRP1 | GICC_CTLR_ACK_CTL;
        if ctlr & (GICD_CTLR_EN_GRP0 | group1_only) == group1_only {
            GICD_CTLR_EN_GRP1
        } else {
            GICD_CTLR_EN_GRP0 | GICD_CTLR_EN_GRP1
        }
    }

    /// IHI 0048B §3.7.2 `UpdateExceptionState` and `GIC_GenerateExceptions`
    /// (QEMU `gic_update_internal`): the highest-priority pending interrupt
    /// of the groups the distributor forwards becomes the current pending
    /// interrupt, which `GICC_HPPIR` reports, if it beats the priority mask
    /// (its whole priority, §3.3 "Priority masking"). The distributor's
    /// group enables mask before prioritisation, the IMPLEMENTATION DEFINED
    /// `IGNORE_GROUP_ENABLE` FALSE (§3.7.1); the CPU interface's take no
    /// part in the choice ([`Self::considered_groups`] gives its one
    /// exception), so a higher-priority interrupt of a group it does not
    /// signal hides the other group's. The interrupt chosen is signalled,
    /// and is the one `GICC_IAR` acknowledges, if its group is enabled at
    /// the CPU interface and it preempts the running priority: as FIQ if it
    /// is Group 0 and `FIQEn` is set (§4.4.1 GICC_CTLR.FIQEn), as IRQ
    /// otherwise.
    fn update(&mut self) {
        self.current_pending = SPURIOUS;
        self.signalled = SPURIOUS;
        self.irq_out = false;
        self.fiq_out = false;
        let forwarded = self.dist_ctlr & (GICD_CTLR_EN_GRP0 | GICD_CTLR_EN_GRP1);
        let below_mask = |g: &Self, irq: usize| g.priority[irq] < g.pmr;
        let highest = self.best_irq(forwarded).filter(|&i| below_mask(self, i));
        if let Some(best) = highest {
            self.current_pending = best as u32;
        }
        let considered = forwarded & Self::considered_groups(self.cpu_ctlr);
        // The highest pending interrupt is the choice unless it is of a group
        // left out, the one case that takes a second look.
        let chosen = match highest {
            Some(i) if considered & group_enable(self.group1.get(i)) == 0 => {
                self.best_irq(considered).filter(|&i| below_mask(self, i))
            }
            Some(_) | None => highest,
        };
        let Some(best) = chosen else {
            return;
        };
        if !self.cpu_signals(best) || !self.preempts(best) {
            return;
        }
        self.signalled = best as u32;
        if !self.group1.get(best) && self.cpu_ctlr & GICC_CTLR_FIQ_EN != 0 {
            self.fiq_out = true;
        } else {
            self.irq_out = true;
        }
    }

    // ----- distributor -----

    pub fn dist_read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        let off = offset.get();
        match width {
            AccessWidth::Byte => u64::from(self.dist_readb(off)),
            AccessWidth::Half => {
                u64::from(self.dist_readb(off)) | u64::from(self.dist_readb(off + 1)) << 8
            }
            AccessWidth::Word => u64::from(self.dist_readl(off)),
            AccessWidth::Dword => {
                u64::from(self.dist_readl(off)) | u64::from(self.dist_readl(off + 4)) << 32
            }
        }
    }

    fn dist_readl(&self, off: u64) -> u32 {
        (0..4).fold(0, |v, b| v | u32::from(self.dist_readb(off + b)) << (8 * b))
    }

    /// The first interrupt a byte of a one-bit-per-interrupt bank describes,
    /// if it exists.
    fn bank_irq(off: u64, base: u64) -> Option<usize> {
        let irq = ((off - base) * 8) as usize;
        (irq < MAX_IRQ).then_some(irq)
    }

    /// Byte `n` of `GICD_IIDR`, the board's identity's.
    #[inline(always)]
    fn iidr_byte(&self, n: usize) -> u8 {
        self.names.gic.distributor.get().to_le_bytes()[n]
    }

    /// QEMU `gic_dist_readb`.
    fn dist_readb(&self, off: u64) -> u8 {
        match off {
            0x000 => self.dist_ctlr as u8,
            0x004 => (self.shape.words() - 1) as u8,
            // GICD_IIDR, the board's identity's where QEMU's reads Arm's
            // JEP106 code, 0x43B. An arm per byte keeps the offsets below
            // 0x00C one jump table, which every other register's read skips
            // in two instructions (`cargo xtask ci`'s hot-loop step).
            0x008 => self.iidr_byte(0),
            0x009 => self.iidr_byte(1),
            0x00A => self.iidr_byte(2),
            0x00B => self.iidr_byte(3),
            0x080..=0x0FF => Self::bank_irq(off, 0x080).map_or(0, |irq| self.group1.byte(irq)),
            0x100..=0x17F => Self::bank_irq(off, 0x100).map_or(0, |irq| self.enabled.byte(irq)),
            0x180..=0x1FF => Self::bank_irq(off, 0x180).map_or(0, |irq| self.enabled.byte(irq)),
            0x200..=0x2FF => {
                let base = if off < 0x280 { 0x200 } else { 0x280 };
                Self::bank_irq(off, base).map_or(0, |irq| {
                    let mut b = 0u8;
                    for k in 0..8 {
                        if irq + k < MAX_IRQ && self.test_pending(irq + k) {
                            b |= 1 << k;
                        }
                    }
                    b
                })
            }
            0x300..=0x37F => Self::bank_irq(off, 0x300).map_or(0, |irq| self.active.byte(irq)),
            0x380..=0x3FF => Self::bank_irq(off, 0x380).map_or(0, |irq| self.active.byte(irq)),
            0x400..=0x7FF => self
                .priority
                .get((off - 0x400) as usize)
                .copied()
                .unwrap_or(0),
            // GICD_ITARGETSR: RAZ/WI on a uniprocessor GIC.
            0x800..=0xBFF => 0,
            0xC00..=0xEFF => {
                // Two bits per interrupt: bit 1 of each pair is edge, bit 0
                // (the 11MPCore's N-N model) reads as zero on a GICv2.
                let first = ((off - 0xC00) * 4) as usize;
                let mut b = 0u8;
                for k in 0..4 {
                    if first + k < MAX_IRQ && self.edge.get(first + k) {
                        b |= 2 << (2 * k);
                    }
                }
                b
            }
            0xF10..=0xF1F => self.sgi_sources((off - 0xF10) as usize),
            0xF20..=0xF2F => self.sgi_sources((off - 0xF20) as usize),
            0xFD0..=0xFFF if off.is_multiple_of(4) => PERIPHERAL_ID[((off - 0xFD0) >> 2) as usize],
            _ => 0,
        }
    }

    pub fn dist_write(&mut self, offset: RegOffset, width: AccessWidth, value: u64) {
        let off = offset.get();
        match width {
            AccessWidth::Byte => self.dist_writeb(off, value as u8),
            AccessWidth::Half => self.dist_writew(off, value as u16),
            AccessWidth::Word => self.dist_writel(off, value as u32),
            AccessWidth::Dword => {
                self.dist_writel(off, value as u32);
                self.dist_writel(off + 4, (value >> 32) as u32);
            }
        }
        self.update();
    }

    /// QEMU `gic_dist_writel`: `GICD_SGIR` is written only whole.
    fn dist_writel(&mut self, off: u64, v: u32) {
        if off == 0xF00 {
            self.write_sgir(v);
        } else {
            self.dist_writew(off, v as u16);
            self.dist_writew(off + 2, (v >> 16) as u16);
        }
    }

    fn dist_writew(&mut self, off: u64, v: u16) {
        self.dist_writeb(off, v as u8);
        self.dist_writeb(off + 1, (v >> 8) as u8);
    }

    /// For each bit set in `v`, set or clear interrupt `first + k`, if it
    /// exists in `shape`.
    fn set_each(shape: GicShape, bits: &mut Bits, first: usize, v: u8, value: bool) {
        for k in 0..8 {
            if v & (1 << k) != 0 && shape.implemented(first + k) {
                bits.set(first + k, value);
            }
        }
    }

    /// QEMU `gic_dist_writeb`.
    fn dist_writeb(&mut self, off: u64, v: u8) {
        match off {
            0x000 => self.dist_ctlr = u32::from(v) & (GICD_CTLR_EN_GRP0 | GICD_CTLR_EN_GRP1),
            0x080..=0x0FF => {
                if let Some(irq) = Self::bank_irq(off, 0x080) {
                    for k in 0..8 {
                        if self.shape.implemented(irq + k) {
                            self.group1.set(irq + k, v & (1 << k) != 0);
                        }
                    }
                }
            }
            0x100..=0x17F => {
                if let Some(irq) = Self::bank_irq(off, 0x100) {
                    // SGIs cannot be disabled: any write enables them.
                    let v = if irq < NR_SGIS { 0xFF } else { v };
                    Self::set_each(self.shape, &mut self.enabled, irq, v, true);
                }
            }
            0x180..=0x1FF => {
                if let Some(irq) = Self::bank_irq(off, 0x180) {
                    let v = if irq < NR_SGIS { 0 } else { v };
                    Self::set_each(self.shape, &mut self.enabled, irq, v, false);
                }
            }
            0x200..=0x27F => {
                // SGIs are made pending only through GICD_SGIR and
                // GICD_SPENDSGIR.
                if let Some(irq) = Self::bank_irq(off, 0x200) {
                    let v = if irq < NR_SGIS { 0 } else { v };
                    Self::set_each(self.shape, &mut self.pending, irq, v, true);
                }
            }
            0x280..=0x2FF => {
                if let Some(irq) = Self::bank_irq(off, 0x280) {
                    let v = if irq < NR_SGIS { 0 } else { v };
                    Self::set_each(self.shape, &mut self.pending, irq, v, false);
                }
            }
            0x300..=0x37F => {
                if let Some(irq) = Self::bank_irq(off, 0x300) {
                    Self::set_each(self.shape, &mut self.active, irq, v, true);
                }
            }
            0x380..=0x3FF => {
                if let Some(irq) = Self::bank_irq(off, 0x380) {
                    Self::set_each(self.shape, &mut self.active, irq, v, false);
                }
            }
            // QEMU `gic_dist_set_priority`: the unimplemented priority bits
            // and an interrupt that does not exist are WI.
            0x400..=0x7FF => {
                let irq = (off - 0x400) as usize;
                if self.shape.implemented(irq) {
                    self.priority[irq] = v & self.shape.priority_bits.mask();
                }
            }
            // Only an SPI's Int_config[1] is programmable: an SGI's is RAO/WI
            // and a PPI's reads as level-sensitive, not programmable, as in
            // the GIC-400 (IHI 0048B Table 4-18; DDI 0471B Table 3-2,
            // `GICD_ICFGR1` RO). Int_config[0] is reserved.
            0xC08..=0xEFF => {
                let first = ((off - 0xC00) * 4) as usize;
                for k in 0..4 {
                    if self.shape.implemented(first + k) {
                        self.edge.set(first + k, v & (2 << (2 * k)) != 0);
                    }
                }
            }
            // GICD_CPENDSGIRn and GICD_SPENDSGIRn (IHI 0048B): a 1 removes or
            // adds the SGI's pending state from that source processor, a 0
            // has no effect; only processor 0's bit exists.
            0xF10..=0xF1F if v & SGI_SOURCE_CPU0 != 0 => {
                self.pending.set((off - 0xF10) as usize, false)
            }
            0xF20..=0xF2F if v & SGI_SOURCE_CPU0 != 0 => {
                self.pending.set((off - 0xF20) as usize, true)
            }
            // GICD_CTLR's upper bytes, GICD_ITARGETSR (RAZ/WI on a
            // uniprocessor GIC), the SGI bits of processors that do not
            // exist, and the rest: ignored.
            _ => {}
        }
    }

    /// `GICD_SGIR` (QEMU `gic_dist_writel` at 0xF00), from processor 0. The
    /// target filter "all others" names only processors that do not exist;
    /// the reserved filter 3 names all eight, this one included.
    fn write_sgir(&mut self, value: u32) {
        let sgi = (value & 0xF) as usize;
        let targets = match (value >> 24) & 3 {
            0 => (value >> 16) & 0xFF,
            1 => 0xFF ^ 1,
            2 => 1,
            _ => 0xFF,
        };
        if targets & 1 != 0 {
            self.pending.set(sgi, true);
        }
    }

    // ----- CPU interface -----

    /// IHI 0048B §4.4.7 (`ReadGICC_HPPIR`): `GICC_HPPIR` or `GICC_AHPPIR`,
    /// the current pending interrupt read through `view`: one "of
    /// sufficient priority" (§4.1.5), past the priority mask, whether or not
    /// the CPU interface signals its group, the IMPLEMENTATION DEFINED
    /// `GICC_MASK_HPPIR` (§4.4.1) FALSE, as in the GIC-400 ("even when the
    /// appropriate `GICC_CTLR.EnableGrp` bit is set to 0", DDI 0471B Table
    /// 3-6, note c).
    fn highest_pending_irq(&self, view: CpuView) -> u32 {
        self.reported(self.current_pending, view)
    }

    /// QEMU `gic_get_current_pending_irq`: interrupt `irq` as `view` reads
    /// it ([`read_through`]). Its CPUID field is RAZ, as on any
    /// uniprocessor.
    fn reported(&self, irq: u32, view: CpuView) -> u32 {
        if irq >= MAXIRQ {
            return irq;
        }
        read_through(
            view,
            self.group1.get(irq as usize),
            self.cpu_ctlr & GICC_CTLR_ACK_CTL != 0,
            irq,
        )
    }

    /// Interrupt `irq`'s group mask under `GICC_BPR` and `GICC_ABPR`
    /// ([`group_mask`]).
    fn group_mask(&self, irq: usize) -> u8 {
        group_mask(
            self.group1.get(irq),
            self.cpu_ctlr & GICC_CTLR_CBPR != 0,
            self.bpr,
            self.abpr,
        )
    }

    /// Interrupt `irq`'s group priority ([`group_priority`]).
    fn group_priority(&self, irq: usize) -> u8 {
        group_priority(self.priority[irq], self.group_mask(irq))
    }

    /// The shift from a group priority to its preemption level: the bits at
    /// and below the least binary point, which no group priority has
    /// (IHI 0048B Table 4-47).
    fn level_shift(&self) -> u32 {
        u32::from(self.shape.priority_bits.min_bpr()) + 1
    }

    /// QEMU `gic_activate_irq`.
    fn activate(&mut self, irq: usize) {
        let prio = self.group_priority(irq);
        let level = usize::from(prio >> self.level_shift());
        let bank = if self.group1.get(irq) {
            &mut self.nsapr
        } else {
            &mut self.apr
        };
        bank[level / 32] |= 1 << (level % 32);
        self.running_priority = u16::from(prio);
        self.active.set(irq, true);
    }

    /// QEMU `gic_get_prio_from_apr_bits`.
    fn prio_from_apr_bits(&self) -> u16 {
        for i in 0..MAX_APRS {
            let apr = self.apr[i] | self.nsapr[i];
            if apr != 0 {
                return ((i as u32 * 32 + apr.trailing_zeros()) << self.level_shift()) as u16;
            }
        }
        IDLE_PRIORITY
    }

    /// QEMU `gic_drop_prio`: clear the lowest set bit of the group's active
    /// priority registers, which the nesting of acknowledgements and EOIs
    /// makes the running interrupt's.
    fn drop_prio(&mut self, group1: bool) {
        let bank = if group1 {
            &mut self.nsapr
        } else {
            &mut self.apr
        };
        if let Some(word) = bank.iter_mut().find(|w| **w != 0) {
            *word &= *word - 1;
        }
        self.running_priority = self.prio_from_apr_bits();
    }

    /// QEMU `gic_acknowledge_irq`: the `GICC_IAR` or `GICC_AIAR` value. An
    /// interrupt that does not preempt the running priority reads as 1023
    /// and stays pending (IHI 0048B §4.4.4). Acknowledging an SGI clears its
    /// one source, processor 0 (QEMU `gic_clear_pending_sgi`), and its CPUID
    /// field is RAZ (§4.4.4 GICC_IAR.CPUID: an SGI's source processor is
    /// reported only in a multiprocessor implementation).
    fn acknowledge(&mut self, view: CpuView) -> u32 {
        // `ReadGICC_IAR`: only the interrupt [`GicV2::update`] signals, one
        // whose group the CPU interface enables and "of sufficient priority
        // to be signaled", reads as its ID or as 1022 (IHI 0048B Table 4-35,
        // note a).
        if self.signalled >= MAXIRQ {
            return SPURIOUS;
        }
        let irq = self.reported(self.signalled, view);
        if irq >= MAXIRQ {
            return irq;
        }
        let i = irq as usize;
        self.activate(i);
        self.pending.set(i, false);
        self.update();
        irq
    }

    fn eoi_split(&self) -> bool {
        self.cpu_ctlr & GICC_CTLR_EOIMODE != 0
    }

    /// QEMU `gic_complete_irq` (`GICC_EOIR`, `GICC_AEOIR`): ignored for an
    /// interrupt that does not exist, while nothing is active, and for a
    /// Group 0 interrupt through the Group 1 alias (QEMU's Non-secure EOI);
    /// otherwise the priority drops and, unless `EOImode` splits the two, the
    /// interrupt deactivates. A Group 1 interrupt through `GICC_EOIR` while
    /// `AckCtl` is clear is UNPREDICTABLE, and completes as if it were set.
    fn complete(&mut self, irq: u32, view: CpuView) {
        let Some(i) = self.shape.existing(irq) else {
            return;
        };
        if self.running_priority == IDLE_PRIORITY {
            return;
        }
        let group1 = self.group1.get(i);
        if view == CpuView::Group1Alias && !group1 {
            return;
        }
        self.drop_prio(group1);
        if !self.eoi_split() {
            self.active.set(i, false);
        }
    }

    /// QEMU `gic_deactivate_irq` (`GICC_DIR`): ignored unless `EOImode` is
    /// set, and for an interrupt that does not exist.
    fn deactivate(&mut self, irq: u32) {
        match self.shape.existing(irq) {
            Some(i) if self.eoi_split() => self.active.set(i, false),
            Some(_) | None => {}
        }
    }

    pub fn cpu_read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        read_by_words(offset, width, |offset| {
            u64::from(self.cpu_read_register(offset.get()))
        })
    }

    /// QEMU `gic_cpu_read`, with the Group 1 aliases `GICC_AIAR` (0x20) and
    /// `GICC_AHPPIR` (0x28) of IHI 0048B Table 4-2, which QEMU reads as zero.
    /// `GICC_AEOIR` (0x24) is write-only.
    fn cpu_read_register(&mut self, off: u64) -> u32 {
        match off {
            0x00 => self.cpu_ctlr,
            0x04 => u32::from(self.pmr),
            0x08 => u32::from(self.bpr),
            0x0C => self.acknowledge(CpuView::Primary),
            0x14 => u32::from(self.running_priority.min(0xFF)),
            0x18 => self.highest_pending_irq(CpuView::Primary),
            0x1C => u32::from(self.abpr),
            0x20 => self.acknowledge(CpuView::Group1Alias),
            0x28 => self.highest_pending_irq(CpuView::Group1Alias),
            0xD0 | 0xD4 | 0xD8 | 0xDC => self.apr[((off - 0xD0) / 4) as usize],
            0xE0 | 0xE4 | 0xE8 | 0xEC => self.nsapr[((off - 0xE0) / 4) as usize],
            0xFC => self.names.gic.cpu_interface.get(),
            _ => 0,
        }
    }

    pub fn cpu_write(&mut self, offset: RegOffset, width: AccessWidth, value: u64) {
        write_by_words(offset, width, value, |offset, word| {
            self.cpu_write_register(offset.get(), word as u32);
        });
        self.update();
    }

    /// QEMU `gic_cpu_write`, with the Group 1 alias `GICC_AEOIR` (0x24) of
    /// IHI 0048B Table 4-2, which QEMU ignores.
    fn cpu_write_register(&mut self, off: u64, v: u32) {
        match off {
            0x00 => self.cpu_ctlr = v & GICC_CTLR_V2_MASK,
            0x04 => self.pmr = v as u8 & self.shape.priority_bits.mask(),
            0x08 => self.bpr = (v as u8 & 7).max(self.shape.priority_bits.min_bpr()),
            0x10 => self.complete(v & 0x3FF, CpuView::Primary),
            0x1C => self.abpr = (v as u8 & 7).max(self.shape.priority_bits.min_abpr()),
            0x24 => self.complete(v & 0x3FF, CpuView::Group1Alias),
            // The bits of the preemption levels the priority bits give; the
            // other bits, and the registers past them, are reserved (Table
            // 4-47).
            0xD0 | 0xD4 | 0xD8 | 0xDC => {
                let n = ((off - 0xD0) / 4) as usize;
                self.apr[n] = v & self.shape.priority_bits.apr_bits(n);
                self.running_priority = self.prio_from_apr_bits();
            }
            0xE0 | 0xE4 | 0xE8 | 0xEC => {
                let n = ((off - 0xE0) / 4) as usize;
                self.nsapr[n] = v & self.shape.priority_bits.apr_bits(n);
                self.running_priority = self.prio_from_apr_bits();
            }
            0x1000 => self.deactivate(v & 0x3FF),
            _ => {}
        }
    }

    // ----- Virtualization Extensions -----

    /// The register of this processor's virtual interface control block at
    /// `offset` in the `GICH` frame, as the GIC-400 lays the frame out (DDI
    /// 0471B §3.7): the common base, which an access redirects to its own
    /// processor's block, at 0x0000, and from 0x1000 the processor-specific
    /// bases, 0x200 apart by CPU ID (IHI 0048B §5.2.7). Only processor 0's
    /// exists; the others' are reserved.
    fn hyp_register(offset: RegOffset) -> Option<u64> {
        match offset.get() {
            off @ 0x0000..=0x0FFF => Some(off),
            off @ 0x1000..=0x11FF => Some(off - 0x1000),
            _ => None,
        }
    }

    /// A read of the virtual interface control registers (`GICH_*`, IHI
    /// 0048B §5.3). Without the extensions the board maps no such frame,
    /// and every register reads as zero.
    pub fn hyp_read(&self, offset: RegOffset, width: AccessWidth) -> u64 {
        read_by_words(offset, width, |offset| {
            let value = match (&self.virt, Self::hyp_register(offset)) {
                (Some(virt), Some(off)) => virt.hyp_read(off),
                (Some(_), None) | (None, _) => 0,
            };
            u64::from(value)
        })
    }

    pub fn hyp_write(&mut self, offset: RegOffset, width: AccessWidth, value: u64) {
        write_by_words(offset, width, value, |offset, word| {
            if let (Some(virt), Some(off)) = (self.virt.as_mut(), Self::hyp_register(offset)) {
                virt.hyp_write(off, word as u32);
            }
        });
        self.sync_virtual();
    }

    /// A read of the virtual CPU interface (`GICV_*`, §5.5).
    pub fn vcpu_read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        let mut effect = ReadEffect::Nothing;
        let iidr = self.names.gic.cpu_interface.get();
        let value = read_by_words(offset, width, |offset| {
            let Some(virt) = self.virt.as_mut() else {
                return 0;
            };
            let read = virt.vcpu_read(offset.get(), iidr);
            if read.effect == ReadEffect::Acknowledged {
                effect = ReadEffect::Acknowledged;
            }
            u64::from(read.value)
        });
        // An acknowledge changes the List registers, and with them the
        // outputs; no other read changes anything.
        match effect {
            ReadEffect::Acknowledged => self.sync_virtual(),
            ReadEffect::Nothing => {}
        }
        value
    }

    pub fn vcpu_write(&mut self, offset: RegOffset, width: AccessWidth, value: u64) {
        write_by_words(offset, width, value, |offset, word| {
            let retired = match self.virt.as_mut() {
                Some(virt) => virt.vcpu_write(offset.get(), word as u32),
                None => HwDeactivation(None),
            };
            self.retire_physical(retired);
        });
        self.sync_virtual();
    }

    /// A hardware List register's deactivation reaching the distributor:
    /// its physical interrupt is no longer active (IHI 0048B §5.5.5), and
    /// the distributor chooses again. A PPI is this processor's, the one
    /// there is.
    fn retire_physical(&mut self, retired: HwDeactivation) {
        // `deactivate_lr` gives a PPI's or an SPI's ID; one this GIC does
        // not implement names none of its interrupts.
        if let HwDeactivation(Some(id)) = retired {
            if let Some(i) = self.shape.existing(id) {
                self.active.set(i, false);
                self.update();
            }
        }
    }

    /// Drive the maintenance interrupt's line from the virtual interface
    /// ([`GicV2::set_line`] has the distributor choose again when the level
    /// changes).
    fn sync_virtual(&mut self) {
        let Some(virt) = &self.virt else {
            return;
        };
        let level = virt.maintenance();
        self.set_line(MAINTENANCE_INTID, level);
    }

    // ----- GICv2m MSI frame -----

    /// A read of the frame, whose `MSI_TYPER` its SPIs give and whose
    /// `MSI_IIDR` the GIC's names give ([`GicNames::msi_frame`]).
    pub fn v2m_read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        let off = offset.get();
        let (msi, iidr) = (self.msi, self.names.msi_frame);
        match width {
            AccessWidth::Byte | AccessWidth::Half => 0,
            AccessWidth::Word => u64::from(Self::v2m_register(off, msi, iidr)),
            AccessWidth::Dword => {
                u64::from(Self::v2m_register(off, msi, iidr))
                    | u64::from(Self::v2m_register(off + 4, msi, iidr)) << 32
            }
        }
    }

    /// QEMU `gicv2m_read` for a 4-byte access (any other size reads as
    /// zero), but for `MSI_TYPER`, which reports the board's choice of SPIs
    /// ([`MsiFrame`]), and `MSI_IIDR`, which is the board's identity's
    /// where QEMU's names QEMU (`PRODUCT_ID_QEMU`, 'Q').
    pub(super) fn v2m_register(off: u64, msi: Option<MsiFrame>, iidr: MsiFrameIidr) -> u32 {
        match off {
            V2M_MSI_TYPER => msi.map_or(0, MsiFrame::typer),
            V2M_MSI_IIDR => iidr.get(),
            _ => 0,
        }
    }

    pub fn v2m_write(&mut self, offset: RegOffset, width: AccessWidth, value: u64) {
        let off = offset.get();
        match width {
            AccessWidth::Byte => {}
            AccessWidth::Half | AccessWidth::Word => self.v2m_write_register(off, value as u32),
            AccessWidth::Dword => {
                self.v2m_write_register(off, value as u32);
                self.v2m_write_register(off + 4, (value >> 32) as u32);
            }
        }
    }

    /// QEMU `gicv2m_write` for a 2- or 4-byte access: `MSI_SETSPI_NS`
    /// pulses the SPI's input line (`gicv2m_set_irq`), which latches it
    /// pending only if it is configured edge-triggered; one that is not the
    /// frame's has no effect (BSA DEN0094C I.9.2).
    fn v2m_write_register(&mut self, off: u64, v: u32) {
        if off == V2M_MSI_SETSPI_NS {
            let spi = v & 0x3FF;
            if self.msi.is_some_and(|msi| msi.holds(spi)) {
                self.set_line(spi, true);
                self.set_line(spi, false);
            }
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    // Word accesses to the distributor and the CPU interface, for this
    // module's tests and the virtual interface's (`super::gicv2_virt`).

    pub(in crate::devices) fn dist(g: &mut GicV2, off: u64, v: u32) {
        g.dist_write(RegOffset::new(off), AccessWidth::Word, u64::from(v));
    }

    pub(in crate::devices) fn cpu(g: &mut GicV2, off: u64, v: u32) {
        g.cpu_write(RegOffset::new(off), AccessWidth::Word, u64::from(v));
    }

    pub(in crate::devices) fn cpu_get(g: &mut GicV2, off: u64) -> u32 {
        g.cpu_read(RegOffset::new(off), AccessWidth::Word) as u32
    }

    pub(in crate::devices) fn dist_get(g: &mut GicV2, off: u64) -> u32 {
        g.dist_read(RegOffset::new(off), AccessWidth::Word) as u32
    }

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
    const NSAPR0: u64 = 0xE0;
    const DIR: u64 = 0x1000;

    /// Both groups enabled at the distributor and the CPU interface, no
    /// priority masked.
    fn enabled() -> GicV2 {
        enabled_shaped(GicShape::DEFAULT)
    }

    /// [`enabled`], of `shape`.
    fn enabled_shaped(shape: GicShape) -> GicV2 {
        let mut g = GicV2::new().shaped(shape);
        dist(&mut g, 0x000, 3);
        cpu(&mut g, CTLR, 3);
        cpu(&mut g, PMR, 0xFF);
        g
    }

    /// Make interrupt `irq` edge-triggered (its `GICD_ICFGR` byte).
    fn edge_triggered(g: &mut GicV2, irq: u32) {
        let byte = 2u8 << (2 * (irq % 4));
        g.dist_write(
            RegOffset::new(0xC00 + u64::from(irq / 4)),
            AccessWidth::Byte,
            u64::from(byte),
        );
    }

    /// Enable SPI `irq` at `priority`, and raise its line.
    fn raise(g: &mut GicV2, irq: u32, priority: u8) {
        g.dist_write(
            RegOffset::new(0x400 + u64::from(irq)),
            AccessWidth::Byte,
            u64::from(priority),
        );
        dist(g, 0x100 + u64::from(irq / 32) * 4, 1 << (irq % 32));
        g.set_line(irq, true);
    }

    /// The timer path EDK2 depends on: a level PPI raises IRQ once enabled,
    /// IAR acknowledges it, and after EOI it is signalled again only while
    /// the line is still high.
    #[test]
    fn a_level_ppi_is_acknowledged_and_completed() {
        let mut g = GicV2::new();
        dist(&mut g, 0x000, 1);
        cpu(&mut g, CTLR, 1);
        cpu(&mut g, PMR, 0xFF);
        dist(&mut g, 0x100, 1 << 30);
        g.set_line(30, true);
        assert!(g.irq_out());
        assert_eq!(cpu_get(&mut g, IAR), 30);
        assert!(!g.irq_out(), "active, so not signalled again");
        g.set_line(30, false);
        cpu(&mut g, EOIR, 30);
        assert!(!g.irq_out());
        assert_eq!(cpu_get(&mut g, IAR), 1023);
    }

    /// The priority mask hides an interrupt whose priority is not higher:
    /// it is neither signalled, acknowledged nor reported, being of no
    /// "sufficient priority" (IHI 0048B §4.1.5; Table 5-17 for the virtual
    /// `GICV_AHPPIR`).
    ///
    /// Of five priority bits, `GICC_PMR`'s [2:0] are RAZ/WI (Table 4-32), so
    /// the next mask up is 0x88.
    #[test]
    fn pmr_masks_lower_priority_interrupts() {
        let mut g = enabled();
        cpu(&mut g, PMR, 0x80);
        raise(&mut g, 32, 0x80);
        assert!(!g.irq_out());
        assert_eq!(cpu_get(&mut g, HPPIR), 1023);
        assert_eq!(cpu_get(&mut g, IAR), 1023);
        cpu(&mut g, PMR, 0x87);
        assert_eq!(cpu_get(&mut g, PMR), 0x80, "bits [2:0] are RAZ/WI");
        assert!(!g.irq_out());
        cpu(&mut g, PMR, 0x88);
        assert!(g.irq_out());
    }

    #[test]
    fn sgis_are_enabled_from_reset_and_stay_enabled() {
        let mut g = GicV2::new();
        assert_eq!(dist_get(&mut g, 0x100) & 0xFFFF, 0xFFFF);
        dist(&mut g, 0x180, 0xFFFF_FFFF);
        assert_eq!(
            dist_get(&mut g, 0x100),
            0xFFFF,
            "ICENABLER cannot disable an SGI"
        );
        dist(&mut g, 0xC00, 0);
        assert_eq!(
            dist_get(&mut g, 0xC00),
            0xAAAA_AAAA,
            "nor make one level-sensitive"
        );
    }

    /// IHI 0048B §4.4.4: GICC_IAR.CPUID names an SGI's source only in a
    /// multiprocessor implementation, so here it reads as zero; and the
    /// SGI registers hold a source bit for processor 0 alone.
    #[test]
    fn an_sgi_on_a_uniprocessor_has_one_source_and_no_cpuid() {
        let mut g = enabled();
        dist(&mut g, 0xF00, (2 << 24) | 5);
        assert!(g.irq_out());
        assert_eq!(cpu_get(&mut g, IAR), 5, "from processor 0");
        cpu(&mut g, EOIR, 5);
        // Bits for processors 3 and 6, which do not exist, are RAZ/WI.
        g.dist_write(RegOffset::new(0xF20 + 7), AccessWidth::Byte, 0x48);
        assert_eq!(g.dist_read(RegOffset::new(0xF20 + 7), AccessWidth::Byte), 0);
        assert!(!g.irq_out(), "no SGI from a processor that does not exist");
        assert_eq!(cpu_get(&mut g, HPPIR), 1023);
        // With processor 0's bit too, the SGI is pending once, from it.
        g.dist_write(RegOffset::new(0xF20 + 7), AccessWidth::Byte, 0x49);
        assert_eq!(g.dist_read(RegOffset::new(0xF10 + 7), AccessWidth::Byte), 1);
        assert_eq!(cpu_get(&mut g, HPPIR), 7);
        assert_eq!(cpu_get(&mut g, IAR), 7, "CPUID is RAZ");
        cpu(&mut g, EOIR, 7);
        assert_eq!(
            cpu_get(&mut g, IAR),
            1023,
            "acknowledged once: no other source"
        );
        assert_eq!(dist_get(&mut g, 0xF24), 0);
        // GICD_CPENDSGIRn clears it only through processor 0's bit.
        g.dist_write(RegOffset::new(0xF20 + 2), AccessWidth::Byte, 1);
        g.dist_write(RegOffset::new(0xF10 + 2), AccessWidth::Byte, 0xFE);
        assert_eq!(
            cpu_get(&mut g, HPPIR),
            2,
            "other processors' bits clear nothing"
        );
        g.dist_write(RegOffset::new(0xF10 + 2), AccessWidth::Byte, 1);
        assert_eq!(cpu_get(&mut g, HPPIR), 1023);
    }

    /// IHI 0048B GICD_SPENDSGIRn: a 0 bit has no effect, so a word write
    /// pends only the SGIs whose byte sets a bit.
    #[test]
    fn a_zero_spendsgir_byte_pends_nothing() {
        let mut g = enabled();
        g.dist_write(RegOffset::new(0xF20 + 4), AccessWidth::Byte, 0);
        assert!(!g.irq_out());
        assert_eq!(cpu_get(&mut g, HPPIR), 1023);
        dist(&mut g, 0xF20, 0x0000_0001);
        assert_eq!(dist_get(&mut g, 0x200) & 0xF, 1, "SGI 0 only");
        assert_eq!(dist_get(&mut g, 0xF20), 1);
        assert_eq!(cpu_get(&mut g, IAR), 0);
        cpu(&mut g, EOIR, 0);
        assert_eq!(
            cpu_get(&mut g, IAR),
            1023,
            "SGIs 1 to 3 were not made pending"
        );
    }

    #[test]
    fn sgir_target_filters_on_a_uniprocessor() {
        let mut g = enabled();
        dist(&mut g, 0xF00, (1 << 24) | 1);
        assert!(!g.irq_out(), "all others: nobody");
        dist(&mut g, 0xF00, 1 << 16 | 2);
        assert_eq!(cpu_get(&mut g, IAR), 2, "a target list naming processor 0");
        dist(&mut g, 0xF00, (3 << 24) | 3);
        assert_eq!(
            cpu_get(&mut g, HPPIR),
            3,
            "the reserved filter names every processor"
        );
        g.dist_write(RegOffset::new(0xF00), AccessWidth::Byte, 4);
        assert_eq!(
            dist_get(&mut g, 0x200) & (1 << 4),
            0,
            "GICD_SGIR is written only whole"
        );
    }

    /// The running priority is the active interrupt's group priority, and
    /// its preemption level is one bit of `GICC_APR0`, the one active
    /// priority register five priority bits need (IHI 0048B Table 4-47).
    #[test]
    fn the_running_priority_is_the_group_priority() {
        let mut g = enabled();
        assert_eq!(cpu_get(&mut g, RPR), 0xFF, "idle");
        cpu(&mut g, BPR, 3);
        raise(&mut g, 40, 0x58);
        assert_eq!(cpu_get(&mut g, IAR), 40);
        assert_eq!(cpu_get(&mut g, RPR), 0x50, "bits [7:4] at BPR 3");
        // Preemption level 0x50 >> 3 = 10.
        assert_eq!(cpu_get(&mut g, APR0), 1 << 10);
        assert_eq!(cpu_get(&mut g, APR0 + 4), 0, "GICC_APR1 is reserved");
        // Preemption compares group priorities: 0x50 is 0x50 at BPR 3.
        raise(&mut g, 41, 0x50);
        assert!(!g.irq_out(), "0x50 does not preempt the running 0x50");
        raise(&mut g, 42, 0x48);
        assert!(g.irq_out(), "0x48 does");
        assert_eq!(cpu_get(&mut g, IAR), 42);
        assert_eq!(cpu_get(&mut g, RPR), 0x40);
        cpu(&mut g, EOIR, 42);
        assert_eq!(cpu_get(&mut g, RPR), 0x50);
        cpu(&mut g, EOIR, 40);
        assert_eq!(cpu_get(&mut g, RPR), 0xFF);
    }

    /// Of five priority bits, each priority field's [2:0] are RAZ/WI (IHI
    /// 0048B Table 3-1), and a binary point below its minimum, 2 for
    /// `GICC_BPR` and 3 for `GICC_ABPR`, is set to the minimum (§4.4.3),
    /// which is also where a reset leaves it.
    #[test]
    fn five_priority_bits_bound_the_priorities_and_binary_points() {
        let mut g = GicV2::new();
        assert_eq!(cpu_get(&mut g, BPR), 2, "reset");
        assert_eq!(cpu_get(&mut g, ABPR), 3, "reset");
        cpu(&mut g, BPR, 0);
        cpu(&mut g, ABPR, 1);
        assert_eq!(cpu_get(&mut g, BPR), 2);
        assert_eq!(cpu_get(&mut g, ABPR), 3);
        cpu(&mut g, BPR, 5);
        cpu(&mut g, ABPR, 6);
        assert_eq!(cpu_get(&mut g, BPR), 5);
        assert_eq!(cpu_get(&mut g, ABPR), 6);
        dist(&mut g, 0x400 + 32, 0xFFFF_FFFF);
        assert_eq!(dist_get(&mut g, 0x400 + 32), 0xF8F8_F8F8);
        cpu(&mut g, PMR, 0xFF);
        assert_eq!(cpu_get(&mut g, PMR), 0xF8);
    }

    /// `GICD_TYPER.ITLinesNumber` reads 15: 480 SPIs, the GIC-400's largest
    /// configuration, the last of which (INTID 511) is signalled.
    #[test]
    fn the_distributor_has_480_spis() {
        let mut g = enabled();
        assert_eq!(dist_get(&mut g, 0x004) & 0x1F, 15);
        raise(&mut g, 511, 0x88);
        assert_eq!(dist_get(&mut g, 0x400 + 508) >> 24, 0x88, "its priority");
        assert_eq!(
            dist_get(&mut g, 0x100 + 511 / 32 * 4) >> 31,
            1,
            "its enable"
        );
        assert_eq!(cpu_get(&mut g, IAR), 511);
        g.set_line(511, false);
        cpu(&mut g, EOIR, 511);
        assert_eq!(cpu_get(&mut g, RPR), 0xFF);
        // An input line past the last SPI raises nothing.
        g.set_line(512, true);
        assert!(!g.irq_out());
        assert_eq!(cpu_get(&mut g, HPPIR), 1023);
    }

    /// Each count of priority bits has IHI 0048B Table 4-47's binary points
    /// and active priority registers: `GICC_BPR` resets to, and is raised
    /// to, 3, 2, 1, 0 and 0, `GICC_ABPR` to one more; `GICC_PMR` keeps the
    /// bits that exist; an interrupt at priority 0x80 activates the
    /// preemption level 0x80 has, its bit in the word that holds it; and the
    /// bits and words past the levels that exist are reserved.
    #[test]
    fn each_count_of_priority_bits_has_its_binary_points_and_active_priorities() {
        for (bits, min_bpr, levels, mask) in [
            (PriorityBits::Four, 3, 16, 0xF0),
            (PriorityBits::Five, 2, 32, 0xF8),
            (PriorityBits::Six, 1, 64, 0xFC),
            (PriorityBits::Seven, 0, 128, 0xFE),
            (PriorityBits::Eight, 0, 128, 0xFF),
        ] {
            let shape = GicShape {
                priority_bits: bits,
                ..GicShape::DEFAULT
            };
            let mut g = enabled_shaped(shape);
            assert_eq!(cpu_get(&mut g, BPR), min_bpr, "{bits:?}: reset");
            assert_eq!(cpu_get(&mut g, ABPR), min_bpr + 1, "{bits:?}: reset");
            cpu(&mut g, BPR, 0);
            cpu(&mut g, ABPR, 0);
            assert_eq!(cpu_get(&mut g, BPR), min_bpr, "{bits:?}: raised");
            assert_eq!(cpu_get(&mut g, ABPR), min_bpr + 1, "{bits:?}: raised");
            assert_eq!(cpu_get(&mut g, PMR), mask, "{bits:?}: the bits that exist");

            raise(&mut g, 40, 0x80);
            assert_eq!(cpu_get(&mut g, IAR), 40, "{bits:?}");
            let level = 0x80 * levels / 256;
            for word in 0..4 {
                let want = if word == level / 32 {
                    1 << (level % 32)
                } else {
                    0
                };
                assert_eq!(
                    cpu_get(&mut g, APR0 + 4 * word as u64),
                    want,
                    "{bits:?}: GICC_APR{word}"
                );
            }
            assert_eq!(cpu_get(&mut g, RPR), 0x80, "{bits:?}");
            g.set_line(40, false);
            cpu(&mut g, EOIR, 40);

            for word in 0..4u32 {
                cpu(&mut g, NSAPR0 + 4 * u64::from(word), u32::MAX);
                let held = (levels as u32).saturating_sub(32 * word).min(32);
                let want = if held == 32 {
                    u32::MAX
                } else {
                    (1u32 << held) - 1
                };
                assert_eq!(
                    cpu_get(&mut g, NSAPR0 + 4 * u64::from(word)),
                    want,
                    "{bits:?}: GICC_NSAPR{word} keeps its levels' bits"
                );
            }
        }
        assert_eq!(PriorityBits::new(3), None);
        assert_eq!(PriorityBits::new(9), None);
        for bits in PriorityBits::ALL {
            assert_eq!(PriorityBits::new(bits.count()), Some(bits));
        }
    }

    /// A GIC of fewer SPIs reports them in `GICD_TYPER.ITLinesNumber` and has
    /// none past them: the last one is signalled, the next one's line raises
    /// nothing and its enable is RAZ/WI. The fewest it may have hold an SPI
    /// for each of the board's device lines and one for its GICv2m frame.
    #[test]
    fn a_gic_of_fewer_spis_has_none_past_them() {
        for count in [32, 64, 96, 160, 256, 480] {
            let spis = SpiCount::new(count).expect("a count the GIC-400 allows");
            let mut g = enabled_shaped(GicShape {
                spis,
                ..GicShape::DEFAULT
            });
            assert_eq!(dist_get(&mut g, 0x004) & 0x1F, count / 32, "{count}");
            let last = 32 + count - 1;
            raise(&mut g, last, 0x80);
            assert_eq!(cpu_get(&mut g, IAR), last, "{count}");
            g.set_line(last, false);
            cpu(&mut g, EOIR, last);
            if count < SpiCount::MAX.get() {
                let past = last + 1;
                raise(&mut g, past, 0x80);
                assert!(!g.irq_out(), "{count}: INTID {past} does not exist");
                assert_eq!(
                    dist_get(&mut g, 0x100 + u64::from(past / 32) * 4) & (1 << (past % 32)),
                    0,
                    "{count}: its enable is RAZ/WI"
                );
            }
        }
        assert_eq!(SpiCount::MIN.get(), 32);
        assert!(
            SpiCount::MIN.get()
                > InterruptMap::device_lines(DeviceLayout::Board.windows().virtio.count)
        );
        for refused in [0, 16, 100, 130, 512] {
            assert_eq!(SpiCount::new(refused), None, "{refused}");
        }
    }

    /// An EOI or a deactivation naming an interrupt that does not exist, PPI
    /// 20, changes nothing: the active interrupt keeps its running priority
    /// and stays active.
    #[test]
    fn an_eoi_of_an_interrupt_that_does_not_exist_is_ignored() {
        let mut g = enabled();
        raise(&mut g, 40, 0x50);
        assert_eq!(cpu_get(&mut g, IAR), 40);
        cpu(&mut g, EOIR, 20);
        assert_eq!(cpu_get(&mut g, RPR), 0x50, "the running priority stays");
        assert_eq!(cpu_get(&mut g, APR0), 1 << 10);
        cpu(&mut g, CTLR, 3 | GICC_CTLR_EOIMODE);
        cpu(&mut g, DIR, 20);
        assert_eq!(dist_get(&mut g, 0x304) & (1 << 8), 1 << 8, "40 is active");
        cpu(&mut g, EOIR, 40);
        assert_eq!(cpu_get(&mut g, RPR), 0xFF);
    }

    /// PPIs 16 to 21 and 24 do not exist, as PPIs 16 to 24 do not in the
    /// GIC-400 (DDI 0471B §3.3): their fields are RAZ/WI and their lines
    /// raise nothing. PPIs 25 to 31 do, and PPIs 22 and 23, COMMIRQ's and
    /// PMUIRQ's (BSA DEN0094C RB_PPI_01), and their trigger mode reads
    /// level-sensitive and cannot be changed (IHI 0048B Table 4-18), while
    /// an SPI's can.
    #[test]
    fn only_ppis_22_23_and_25_to_31_exist_and_ppis_stay_level_sensitive() {
        let mut g = enabled();
        let absent = 0x013F_0000;
        let present = 0xFEC0_0000;
        dist(&mut g, 0x080, 0xFFFF_FFFF);
        dist(&mut g, 0x100, 0xFFFF_FFFF);
        dist(&mut g, 0x200, 0xFFFF_FFFF);
        dist(&mut g, 0x300, 0xFFFF_FFFF);
        for bank in [0x080, 0x100, 0x200, 0x300] {
            assert_eq!(dist_get(&mut g, bank) & absent, 0, "bank {bank:#x}");
            assert_eq!(
                dist_get(&mut g, bank) & present,
                present,
                "bank {bank:#x}: PPIs 22, 23 and 25 to 31"
            );
        }
        dist(&mut g, 0x400 + 16, 0x8080_8080);
        assert_eq!(dist_get(&mut g, 0x400 + 16), 0, "PPIs 16 to 19");
        dist(&mut g, 0x400 + 20, 0x8080_8080);
        assert_eq!(
            dist_get(&mut g, 0x400 + 20),
            0x8080_0000,
            "PPIs 22 and 23 alone of 20 to 23"
        );
        assert_eq!(dist_get(&mut g, 0x400 + 24) & 0xFF, 0, "PPI 24");

        let mut g = enabled();
        dist(&mut g, 0x100, 1 << 20);
        g.set_line(20, true);
        assert!(!g.irq_out(), "PPI 20's line raises nothing");
        assert_eq!(cpu_get(&mut g, HPPIR), 1023);
        dist(&mut g, 0x100, 1 << 22);
        g.set_line(22, true);
        assert!(g.irq_out(), "PPI 22's line raises it");
        assert_eq!(cpu_get(&mut g, HPPIR), 22);
        g.set_line(22, false);
        assert!(!g.irq_out(), "and lowering the line withdraws it");
        dist(&mut g, 0x100, 1 << 23);
        g.set_line(23, true);
        assert!(g.irq_out(), "PPI 23's line raises it");
        assert_eq!(cpu_get(&mut g, HPPIR), 23);
        g.set_line(23, false);
        assert!(!g.irq_out(), "and lowering the line withdraws it");
        dist(&mut g, 0xC04, 0xFFFF_FFFF);
        assert_eq!(dist_get(&mut g, 0xC04), 0, "PPIs stay level-sensitive");
        dist(&mut g, 0xC08, 0xAAAA_AAAA);
        assert_eq!(dist_get(&mut g, 0xC08), 0xAAAA_AAAA, "SPIs 32 to 47");
    }

    /// IHI 0048B §3.3.3 and §3.7.2 `UpdateExceptionState`: with CBPR clear
    /// each group's binary point sets its own mask, and a pending interrupt
    /// preempts when its group priority is higher than the running
    /// priority's under that same mask (`PriorityIsHigher(priority,
    /// GICC_RPR AND mask)`). QEMU 11.1.2 compares whole priorities, so it
    /// would take 0x40 here.
    #[test]
    fn preemption_compares_group_priorities_under_each_groups_binary_point() {
        let mut g = enabled();
        cpu(&mut g, BPR, 3);
        cpu(&mut g, ABPR, 3);
        dist(&mut g, 0x084, 1 << 8);
        raise(&mut g, 40, 0x48);
        assert_eq!(cpu_get(&mut g, AIAR), 40);
        assert_eq!(cpu_get(&mut g, RPR), 0x48, "ABPR 3: Group 1 bits [7:3]");
        raise(&mut g, 41, 0x40);
        assert!(
            !g.irq_out(),
            "0x40 at BPR 3 is group priority 0x40, and the running 0x48 is 0x40 under BPR 3"
        );
        assert_eq!(cpu_get(&mut g, IAR), 1023);
        raise(&mut g, 43, 0x38);
        assert!(
            g.irq_out(),
            "0x38 at BPR 3 is group priority 0x30, higher than 0x40"
        );
        assert_eq!(cpu_get(&mut g, HPPIR), 43);
        assert_eq!(cpu_get(&mut g, IAR), 43);
        assert_eq!(cpu_get(&mut g, RPR), 0x30);
        g.set_line(43, false);
        cpu(&mut g, EOIR, 43);
        assert_eq!(cpu_get(&mut g, RPR), 0x48);
        cpu(&mut g, AEOIR, 40);
        assert_eq!(cpu_get(&mut g, RPR), 0xFF);
    }

    #[test]
    fn group_1_uses_the_aliased_binary_point_unless_cbpr() {
        let mut g = enabled();
        cpu(&mut g, CTLR, 3 | GICC_CTLR_ACK_CTL);
        dist(&mut g, 0x084, 1 << 8);
        cpu(&mut g, ABPR, 4);
        raise(&mut g, 40, 0x58);
        assert_eq!(cpu_get(&mut g, IAR), 40);
        assert_eq!(
            cpu_get(&mut g, RPR),
            0x50,
            "ABPR 4 means binary point 3: bits [7:4]"
        );
        // Preemption level 0x50 >> 3 = 10.
        assert_eq!(cpu_get(&mut g, NSAPR0), 1 << 10);
        assert_eq!(
            cpu_get(&mut g, APR0),
            0,
            "Group 1 levels are in GICC_NSAPRn"
        );
        cpu(&mut g, EOIR, 40);
        cpu(&mut g, CTLR, 3 | GICC_CTLR_ACK_CTL | GICC_CTLR_CBPR);
        g.set_line(40, false);
        g.set_line(40, true);
        assert_eq!(cpu_get(&mut g, IAR), 40);
        assert_eq!(cpu_get(&mut g, RPR), 0x58, "BPR 2 with CBPR: bits [7:3]");
    }

    #[test]
    fn a_group_1_interrupt_reads_1022_without_ackctl() {
        let mut g = enabled();
        dist(&mut g, 0x084, 1 << 8);
        raise(&mut g, 40, 0x10);
        assert!(g.irq_out());
        assert_eq!(cpu_get(&mut g, HPPIR), 1022);
        assert_eq!(cpu_get(&mut g, IAR), 1022);
        assert!(g.irq_out(), "not acknowledged");
    }

    /// Only a Group 1 interrupt "of sufficient priority to be signaled"
    /// reads 1022 at `GICC_IAR` (IHI 0048B Table 4-35, note a): one behind
    /// a higher running priority reads 1023, while `GICC_HPPIR` names it.
    #[test]
    fn a_group_1_interrupt_that_cannot_preempt_reads_1023() {
        let mut g = enabled();
        raise(&mut g, 41, 0x10);
        assert_eq!(cpu_get(&mut g, IAR), 41);
        dist(&mut g, 0x084, 1 << 8);
        raise(&mut g, 40, 0x20);
        assert_eq!(cpu_get(&mut g, HPPIR), 1022);
        assert_eq!(cpu_get(&mut g, IAR), 1023);
        g.set_line(41, false);
        cpu(&mut g, EOIR, 41);
        assert_eq!(cpu_get(&mut g, IAR), 1022);
    }

    /// IHI 0048B §4.3.1: GICD_CTLR.EnableGrp1 clear stops the distributor
    /// forwarding Group 1, and its enables mask before prioritisation (the
    /// IMPLEMENTATION DEFINED `IGNORE_GROUP_ENABLE` FALSE, §3.7.1), so its
    /// interrupt is neither signalled nor read, and the Group 0 interrupt
    /// behind it is.
    #[test]
    fn a_group_the_distributor_does_not_forward_hides_nothing() {
        let mut g = enabled();
        cpu(&mut g, CTLR, 3 | GICC_CTLR_ACK_CTL);
        dist(&mut g, 0x000, 1);
        dist(&mut g, 0x084, 1 << 8);
        raise(&mut g, 40, 0x10);
        raise(&mut g, 41, 0x20);
        assert!(g.irq_out(), "the Group 0 interrupt is signalled");
        assert_eq!(cpu_get(&mut g, HPPIR), 41);
        assert_eq!(
            cpu_get(&mut g, AHPPIR),
            1023,
            "no Group 1 interrupt is forwarded"
        );
        assert_eq!(cpu_get(&mut g, AIAR), 1023);
        assert_eq!(cpu_get(&mut g, IAR), 41);
        cpu(&mut g, EOIR, 41);
        g.set_line(41, false);
        assert!(!g.irq_out());
        assert_eq!(
            cpu_get(&mut g, IAR),
            1023,
            "AckCtl does not reach an unforwarded group"
        );
        assert_eq!(
            dist_get(&mut g, 0x304),
            0,
            "interrupt 40 never became active"
        );
        dist(&mut g, 0x000, 3);
        assert_eq!(
            cpu_get(&mut g, IAR),
            40,
            "forwarded once its group is enabled"
        );
    }

    /// IHI 0048B §4.1.5: with AckCtl set, EnableGrp0 clear and EnableGrp1
    /// set, "Group 0 interrupts are ignored and GICC_IAR behaves as
    /// GICC_AIAR": the higher-priority Group 0 interrupt hides nothing, and
    /// the Group 1 one is signalled and acknowledged. With AckCtl clear the
    /// Group 0 interrupt hides it again.
    #[test]
    fn with_ackctl_a_disabled_group_0_is_ignored() {
        let mut g = enabled();
        cpu(&mut g, CTLR, 2 | GICC_CTLR_ACK_CTL);
        dist(&mut g, 0x084, 1 << 9);
        raise(&mut g, 40, 0x10);
        raise(&mut g, 41, 0x20);
        assert!(g.irq_out(), "the Group 1 interrupt is signalled");
        assert_eq!(cpu_get(&mut g, IAR), 41);
        // 41's line stays high: completed, it is pending again.
        cpu(&mut g, EOIR, 41);
        assert!(g.irq_out(), "pending again, and signalled");
        cpu(&mut g, CTLR, 2);
        assert!(!g.irq_out(), "without AckCtl Group 0 hides it again");
        assert_eq!(cpu_get(&mut g, AIAR), 1023);
    }

    /// IHI 0048B §3.7.2 `GIC_GenerateExceptions` and §4.4.4 `ReadGICC_IAR`:
    /// the CPU interface's group enables take no part in choosing the
    /// highest-priority pending interrupt, so a Group 0 interrupt the CPU
    /// interface does not signal hides a lower-priority Group 1 one: neither
    /// is signalled, and `GICC_AIAR` reads 1023. `GICC_HPPIR` names it, as
    /// the GIC-400's does whatever `GICC_CTLR` enables (DDI 0471B Table 3-6,
    /// note c). Once it is no longer pending, the Group 1 interrupt is
    /// signalled and acknowledged.
    #[test]
    fn a_group_the_cpu_interface_does_not_signal_hides_the_other() {
        let mut g = enabled();
        cpu(&mut g, CTLR, 2);
        dist(&mut g, 0x084, 1 << 9);
        raise(&mut g, 40, 0x10);
        raise(&mut g, 41, 0x20);
        assert!(
            !g.irq_out() && !g.fiq_out(),
            "Group 0's interrupt is highest"
        );
        assert_eq!(
            cpu_get(&mut g, HPPIR),
            40,
            "Group 0's, though not signalled"
        );
        assert_eq!(
            cpu_get(&mut g, AHPPIR),
            1023,
            "a Group 0 interrupt is highest"
        );
        assert_eq!(cpu_get(&mut g, AIAR), 1023);
        assert_eq!(cpu_get(&mut g, IAR), 1023, "Group 0 is not signalled");
        assert_eq!(dist_get(&mut g, 0x304), 0, "nothing acknowledged");
        g.set_line(40, false);
        assert!(g.irq_out(), "the Group 1 interrupt is signalled");
        assert_eq!(cpu_get(&mut g, AIAR), 41);
    }

    #[test]
    fn eoi_while_idle_and_dir_without_eoimode_are_ignored() {
        let mut g = enabled();
        dist(&mut g, 0x304, 1 << 8);
        cpu(&mut g, EOIR, 40);
        assert_eq!(
            dist_get(&mut g, 0x304),
            1 << 8,
            "nothing was running, so EOI does nothing"
        );
        cpu(&mut g, DIR, 40);
        assert_eq!(dist_get(&mut g, 0x304), 1 << 8, "DIR needs EOImode");
        cpu(&mut g, CTLR, 3 | GICC_CTLR_EOIMODE);
        cpu(&mut g, DIR, 40);
        assert_eq!(dist_get(&mut g, 0x304), 0);
    }

    #[test]
    fn eoimode_splits_priority_drop_from_deactivation() {
        let mut g = enabled();
        cpu(&mut g, CTLR, 3 | GICC_CTLR_EOIMODE);
        raise(&mut g, 40, 0x20);
        assert_eq!(cpu_get(&mut g, IAR), 40);
        g.set_line(40, false);
        cpu(&mut g, EOIR, 40);
        assert_eq!(cpu_get(&mut g, RPR), 0xFF);
        assert_eq!(dist_get(&mut g, 0x304), 1 << 8, "still active");
        cpu(&mut g, DIR, 40);
        assert_eq!(dist_get(&mut g, 0x304), 0);
    }

    #[test]
    fn ctlr_keeps_qemus_bits() {
        let mut g = enabled();
        cpu(&mut g, CTLR, 0xFFFF_FFFF);
        assert_eq!(cpu_get(&mut g, CTLR), GICC_CTLR_V2_MASK);
    }

    /// IHI 0048B Table 4-2 and §4.4.1 AckCtl: with AckCtl clear (Arm's
    /// recommended setting) Group 1 is acknowledged, completed and reported
    /// through GICC_AIAR, GICC_AEOIR and GICC_AHPPIR, which see no Group 0
    /// interrupt.
    #[test]
    fn the_aliased_registers_handle_group_1() {
        let mut g = enabled();
        dist(&mut g, 0x084, 1 << 8);
        raise(&mut g, 40, 0x10);
        assert_eq!(cpu_get(&mut g, IAR), 1022, "GICC_IAR does not take Group 1");
        assert_eq!(cpu_get(&mut g, AHPPIR), 40);
        assert_eq!(cpu_get(&mut g, AIAR), 40);
        assert_eq!(dist_get(&mut g, 0x304), 1 << 8, "active");
        assert_eq!(cpu_get(&mut g, RPR), 0x10);
        assert_eq!(
            cpu_get(&mut g, NSAPR0),
            1 << 2,
            "Group 1 preemption level 0x10 >> 3 = 2"
        );
        assert_eq!(cpu_get(&mut g, AHPPIR), 1023, "nothing else pending");
        g.set_line(40, false);
        cpu(&mut g, AEOIR, 40);
        assert_eq!(cpu_get(&mut g, RPR), 0xFF);
        assert_eq!(dist_get(&mut g, 0x304), 0, "GICC_AEOIR completes it");
        // A Group 0 interrupt is invisible to the aliases.
        raise(&mut g, 41, 0x20);
        assert_eq!(cpu_get(&mut g, AHPPIR), 1023);
        assert_eq!(cpu_get(&mut g, AIAR), 1023);
        assert_eq!(dist_get(&mut g, 0x304), 0, "not acknowledged");
        assert_eq!(cpu_get(&mut g, IAR), 41);
        cpu(&mut g, AEOIR, 41);
        assert_eq!(
            cpu_get(&mut g, RPR),
            0x20,
            "GICC_AEOIR ignores a Group 0 interrupt"
        );
        cpu(&mut g, EOIR, 41);
        assert_eq!(cpu_get(&mut g, RPR), 0xFF);
    }

    #[test]
    fn an_eight_byte_access_is_two_registers() {
        let mut g = enabled();
        raise(&mut g, 40, 0x10);
        let pair = g.cpu_read(RegOffset::new(BPR), AccessWidth::Dword);
        assert_eq!(pair, (40 << 32) | 2, "GICC_BPR, then GICC_IAR acknowledges");
        g.cpu_write(RegOffset::new(EOIR), AccessWidth::Dword, 40);
        assert_eq!(cpu_get(&mut g, RPR), 0xFF);
        g.dist_write(
            RegOffset::new(0x400 + 40),
            AccessWidth::Dword,
            0x0000_0098_0000_0078,
        );
        assert_eq!(
            g.dist_read(RegOffset::new(0x400 + 40), AccessWidth::Byte),
            0x78
        );
        assert_eq!(
            g.dist_read(RegOffset::new(0x400 + 44), AccessWidth::Byte),
            0x98
        );
    }

    #[test]
    fn an_msi_latches_only_an_edge_triggered_spi() {
        let mut g = enabled()
            .with_msi_frame(frame(80, 64))
            .expect("the GIC's SPIs");
        dist(&mut g, 0x100 + 80 / 32 * 4, 1 << (80 % 32));
        g.v2m_write(RegOffset::new(V2M_MSI_SETSPI_NS), AccessWidth::Word, 80);
        assert!(!g.irq_out(), "a level-sensitive SPI forgets the pulse");
        edge_triggered(&mut g, 80);
        g.v2m_write(RegOffset::new(V2M_MSI_SETSPI_NS), AccessWidth::Byte, 80);
        assert!(!g.irq_out(), "a byte write is ignored");
        g.v2m_write(RegOffset::new(V2M_MSI_SETSPI_NS), AccessWidth::Half, 80);
        assert_eq!(cpu_get(&mut g, IAR), 80);
        assert_eq!(
            g.v2m_read(RegOffset::new(V2M_MSI_TYPER), AccessWidth::Byte),
            0
        );
    }

    /// `MSI_TYPER` reports the frame's SPIs, and only a write naming one of
    /// them raises it (BSA DEN0094C I.9.1, I.9.2); a GIC whose frame has
    /// none reports none and raises nothing.
    #[test]
    fn the_msi_frame_reports_and_raises_only_its_own_spis() {
        let mut g = enabled()
            .with_msi_frame(frame(71, 441))
            .expect("the GIC's SPIs");
        assert_eq!(g.msi_frame(), Some(frame(71, 441)));
        assert_eq!(
            g.v2m_read(RegOffset::new(V2M_MSI_TYPER), AccessWidth::Word),
            u64::from((71u32 << 16) | 441)
        );
        for spi in [70, 71, 511, 512] {
            edge_triggered(&mut g, spi);
            raise_msi_enabled(&mut g, spi);
            let ours = (71..512).contains(&spi);
            assert_eq!(g.irq_out(), ours, "INTID {spi}");
            if ours {
                assert_eq!(cpu_get(&mut g, IAR), spi);
                cpu(&mut g, EOIR, spi);
            }
        }
        g.reset();
        assert_eq!(
            g.v2m_read(RegOffset::new(V2M_MSI_TYPER), AccessWidth::Word),
            u64::from((71u32 << 16) | 441),
            "a reset keeps the frame"
        );
        let mut none = enabled();
        assert_eq!(none.msi_frame(), None);
        assert_eq!(
            none.v2m_read(RegOffset::new(V2M_MSI_TYPER), AccessWidth::Word),
            0
        );
        edge_triggered(&mut none, 80);
        raise_msi_enabled(&mut none, 80);
        assert!(!none.irq_out());
    }

    /// A frame whose SPIs are not all the GIC's is refused, and a GIC
    /// given another shape keeps no frame chosen for the one it had.
    #[test]
    fn a_frame_is_the_gics_own_spis() {
        let small = GicShape {
            spis: SpiCount::new(256).expect("a count"),
            ..GicShape::DEFAULT
        };
        let gic = GicV2::new().shaped(small);
        assert_eq!(
            gic.with_msi_frame(frame(272, 17)).map(|g| g.msi_frame()),
            Err(MsiFrameOutsideGic {
                frame: frame(272, 17),
                spis: small.spis
            }),
            "INTID 288 is past the last, 287"
        );
        let fits = GicV2::new()
            .shaped(small)
            .with_msi_frame(frame(272, 16))
            .expect("INTIDs 272 to 287");
        assert_eq!(fits.msi_frame(), Some(frame(272, 16)));
        assert_eq!(fits.shaped(GicShape::DEFAULT).msi_frame(), None);
    }

    /// A frame of `count` SPIs from INTID `first`.
    fn frame(first: u32, count: u32) -> MsiFrame {
        MsiFrame {
            first: Spi::new(first - 32).expect("an SPI"),
            count: MsiCount::new(count).expect("a count"),
        }
    }

    /// Enable interrupt `spi` and write it to `MSI_SETSPI_NS`.
    fn raise_msi_enabled(g: &mut GicV2, spi: u32) {
        dist(g, 0x100 + u64::from(spi / 32) * 4, 1 << (spi % 32));
        g.v2m_write(
            RegOffset::new(V2M_MSI_SETSPI_NS),
            AccessWidth::Word,
            u64::from(spi),
        );
    }

    #[test]
    fn a_snapshot_round_trips_the_priority_state() {
        let mut g = enabled();
        cpu(&mut g, BPR, 3);
        raise(&mut g, 40, 0x58);
        dist(&mut g, 0xF00, (2 << 24) | 3);
        assert_eq!(cpu_get(&mut g, IAR), 3);
        let mut image = std::vec::Vec::new();
        g.save(&mut image).expect("save");
        let mut restored = GicV2::new();
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restore");
        assert!(
            source.is_empty(),
            "the restore consumed exactly what the save wrote"
        );
        assert_eq!(cpu_get(&mut restored, RPR), 0);
        cpu(&mut restored, EOIR, 3);
        assert_eq!(cpu_get(&mut restored, BPR), 3);
        assert_eq!(cpu_get(&mut restored, IAR), 40);
        assert_eq!(cpu_get(&mut restored, RPR), 0x50);
    }

    /// An image whose SGI source bytes are not the ones this GIC writes is
    /// refused, and the refused restore leaves the GIC as it was: the same
    /// saved state, the same highest pending interrupt, the same output.
    #[test]
    fn a_snapshot_with_sgi_sources_that_disagree_is_refused_and_changes_nothing() {
        let mut g = enabled();
        dist(&mut g, 0xF00, (2 << 24) | 3);
        let mut image = std::vec::Vec::new();
        g.save(&mut image).expect("save");
        // The image of a GIC without the Virtualization Extensions ends with
        // the SGIs' source bytes and then the extensions' tag; SGI 3 is
        // pending.
        let source_byte = |sgi: usize| image.len() - 1 - NR_SGIS + sgi;
        let refused = [
            (source_byte(3), 0x08, "SGI 3 from processor 3"),
            (source_byte(3), 0x09, "SGI 3 from processors 0 and 3"),
            (source_byte(3), 0x00, "SGI 3 pending with no source"),
            (source_byte(4), 0x01, "SGI 4 sourced but not pending"),
        ];

        let mut target = enabled();
        raise(&mut target, 50, 0x30);
        let mut before = std::vec::Vec::new();
        target.save(&mut before).expect("save");
        for (at, byte, why) in refused {
            let mut bad = image.clone();
            bad[at] = byte;
            let mut source: &[u8] = &bad;
            assert!(target.restore(&mut source).is_err(), "{why}");
            let mut after = std::vec::Vec::new();
            target.save(&mut after).expect("save");
            assert!(after == before, "{why}: the refused image changed the GIC");
            assert!(target.irq_out(), "{why}");
            assert_eq!(cpu_get(&mut target, HPPIR), 50, "{why}");
        }
        let mut source: &[u8] = &image;
        target
            .restore(&mut source)
            .expect("the image as saved restores");
        assert_eq!(cpu_get(&mut target, HPPIR), 3);
    }

    /// An image holding what this GIC cannot hold is refused, and the
    /// refused restore leaves the GIC as it was: an unimplemented priority
    /// bit, a binary point below its minimum, a running priority its active
    /// priority registers do not give, an SGI disabled, level-sensitive or
    /// with its line high, an edge-triggered PPI, and any state of a PPI
    /// that does not exist.
    #[test]
    fn a_snapshot_of_state_this_gic_cannot_hold_is_refused_and_changes_nothing() {
        let g = enabled();
        let mut image = std::vec::Vec::new();
        g.save(&mut image).expect("save");
        // `save`'s layout: GICD_CTLR, GICC_CTLR, GICC_PMR, GICC_BPR,
        // GICC_ABPR, the running priority, GICC_APR0, GICC_NSAPR0, then the
        // enabled, pending, active, group, level and edge words, then the
        // priorities.
        const PMR_AT: usize = 8;
        const BPR_AT: usize = 9;
        const ABPR_AT: usize = 10;
        const RUNNING_AT: usize = 11;
        const APR_AT: usize = RUNNING_AT + 2;
        const BANK: usize = WORDS * 4;
        const ENABLED_AT: usize = APR_AT + 2 * MAX_APRS * 4;
        const PENDING_AT: usize = ENABLED_AT + BANK;
        const ACTIVE_AT: usize = ENABLED_AT + 2 * BANK;
        const GROUP_AT: usize = ENABLED_AT + 3 * BANK;
        const LEVEL_AT: usize = ENABLED_AT + 4 * BANK;
        const EDGE_AT: usize = ENABLED_AT + 5 * BANK;
        const PRIORITY_AT: usize = ENABLED_AT + 6 * BANK;
        let refused: [(&[(usize, u8)], &str); 16] = [
            (&[(PMR_AT, 0x81)], "a priority mask with bit 0"),
            (&[(APR_AT + 4, 0x01)], "GICC_APR1, reserved with five bits"),
            (&[(PRIORITY_AT + 40, 0x44)], "a priority with bit 2"),
            (&[(BPR_AT, 1)], "a binary point below 2"),
            (&[(ABPR_AT, 2)], "an aliased binary point below 3"),
            (&[(RUNNING_AT, 0x40), (RUNNING_AT + 1, 0)], "nothing active"),
            (&[(ENABLED_AT, 0xFE)], "SGI 0 disabled"),
            (&[(LEVEL_AT, 0x08)], "SGI 3's line high"),
            (&[(EDGE_AT, 0xFE)], "SGI 0 level-sensitive"),
            (&[(EDGE_AT + 3, 0x80)], "PPI 31 edge-triggered"),
            (&[(ENABLED_AT + 2, 0x10)], "PPI 20 enabled"),
            (&[(PENDING_AT + 2, 0x10)], "PPI 20 pending"),
            (&[(ACTIVE_AT + 3, 0x01)], "PPI 24 active"),
            (&[(GROUP_AT + 2, 0x01)], "PPI 16 in Group 1"),
            (&[(LEVEL_AT + 2, 0x20)], "PPI 21's line high"),
            (&[(PRIORITY_AT + 20, 0x80)], "PPI 20 with a priority"),
        ];

        let mut target = enabled();
        raise(&mut target, 50, 0x30);
        let mut before = std::vec::Vec::new();
        target.save(&mut before).expect("save");
        for (bytes, why) in refused {
            let mut bad = image.clone();
            for &(at, byte) in bytes {
                bad[at] = byte;
            }
            let mut source: &[u8] = &bad;
            assert!(target.restore(&mut source).is_err(), "{why}");
            let mut after = std::vec::Vec::new();
            target.save(&mut after).expect("save");
            assert!(after == before, "{why}: the refused image changed the GIC");
        }
        let mut source: &[u8] = &image;
        target
            .restore(&mut source)
            .expect("the image as saved restores");
    }

    /// IHI 0048B Table 4-2 and §4.4.1 AckCtl: AckCtl widens only the primary
    /// registers to Group 1. The aliases stay Group 1 only: a Group 0
    /// interrupt reads as 1023 through GICC_AHPPIR and GICC_AIAR, is not
    /// acknowledged and is not completed through GICC_AEOIR, while a Group 1
    /// interrupt is still taken through them.
    #[test]
    fn the_aliased_registers_stay_group_1_with_ackctl() {
        let mut g = enabled();
        cpu(&mut g, CTLR, 3 | GICC_CTLR_ACK_CTL);
        dist(&mut g, 0x084, 1 << 8);
        raise(&mut g, 40, 0x20);
        raise(&mut g, 41, 0x10);
        assert_eq!(
            cpu_get(&mut g, AHPPIR),
            1023,
            "the highest pending is Group 0"
        );
        assert_eq!(cpu_get(&mut g, AIAR), 1023);
        assert_eq!(dist_get(&mut g, 0x304), 0, "nothing acknowledged");
        assert_eq!(cpu_get(&mut g, HPPIR), 41);
        assert_eq!(cpu_get(&mut g, IAR), 41);
        cpu(&mut g, AEOIR, 41);
        assert_eq!(
            cpu_get(&mut g, RPR),
            0x10,
            "GICC_AEOIR ignores a Group 0 interrupt"
        );
        assert_eq!(dist_get(&mut g, 0x304), 1 << 9);
        g.set_line(41, false);
        cpu(&mut g, EOIR, 41);
        assert_eq!(cpu_get(&mut g, AHPPIR), 40);
        assert_eq!(cpu_get(&mut g, AIAR), 40, "the alias still takes Group 1");
        assert_eq!(cpu_get(&mut g, RPR), 0x20);
        cpu(&mut g, AEOIR, 40);
        assert_eq!(cpu_get(&mut g, RPR), 0xFF);
        assert_eq!(dist_get(&mut g, 0x304), 0, "GICC_AEOIR completes it");
    }
}
