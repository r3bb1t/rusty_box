//! GICv2 distributor and CPU interface for one processor, without the
//! Security Extensions (QEMU's `virt` without EL3), plus the GICv2m MSI
//! frame.
//!
//! Provenance (R7): the ARM Generic Interrupt Controller Architecture
//! Specification, GIC architecture version 2.0 (IHI 0048B.b), first; QEMU
//! 11.1.2 `hw/intc/arm_gic.c` for a revision-2, single-CPU GIC with 288
//! interrupts and 8 priority bits second, whose structure this model keeps —
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
//! Where IHI 0048B and QEMU differ, this model follows IHI 0048B:
//! - A group the distributor does not forward (§4.3.1 GICD_CTLR.EnableGrpN)
//!   takes no part in choosing the highest-priority pending interrupt, so its
//!   interrupts are neither signalled nor returned by an acknowledge or
//!   highest-pending read, and do not hide the other group's.
//! - An interrupt of a group the CPU interface does not signal (§4.4.1
//!   GICC_CTLR.EnableGrpN) is never acknowledged: GICC_IAR returns 1023
//!   while signalling is disabled (§4.4.4).
//! - A pending interrupt preempts when its group priority, under the binary
//!   point of its own group, is higher than the running priority (§3.3
//!   "Interrupt prioritization": "Preemption", and "Priority grouping" with
//!   Table 3-3).
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
//! Where IHI 0048B is silent, this model reads it as follows. Whether a
//! group the CPU interface does not signal still takes part in choosing the
//! highest-priority pending interrupt, and so can hide the other group's, the
//! specification does not say: §4.4.1 makes GICC_CTLR.EnableGrpN the enable
//! for the signalling of Group N interrupts, and §4.4.4 has GICC_IAR return
//! 1023 while signalling is disabled, and nothing more. This model leaves
//! such a group out of the choice, as it does a group the distributor does
//! not forward, so its interrupts hide nothing. That is the rule the GICv3
//! architecture (IHI 0069) states: ICC_HPPIR0_EL1/ICC_HPPIR1_EL1 and
//! ICC_IAR0_EL1/ICC_IAR1_EL1 consider only interrupts of a group enabled by
//! ICC_IGRPEN0_EL1/ICC_IGRPEN1_EL1. QEMU 11.1.2 (`gic_update_internal`,
//! `gic_acknowledge_irq`) reads it the other way: it chooses the
//! highest-priority interrupt across both groups and only then checks the
//! CPU interface's enables, so a higher-priority interrupt of a group the
//! CPU interface does not signal blocks the enabled group's, and GICC_IAR
//! and GICC_HPPIR return it.
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
//! input line change; the machine reads `irq_out`/`fiq_out` between
//! instructions.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use crate::bus::{AccessWidth, RegOffset};

/// SGIs, PPIs and QEMU `virt`'s 256 SPIs (`num-irq` = `NUM_IRQS + 32`).
pub const NUM_IRQ: usize = 288;
const WORDS: usize = NUM_IRQ / 32;
/// QEMU `GIC_NR_SGIS`.
const NR_SGIS: usize = 16;
/// QEMU `GIC_MAXIRQ`: interrupt IDs from here up are special.
const MAXIRQ: u32 = 1020;
const SPURIOUS: u32 = 1023;
const SPURIOUS_GROUP1: u32 = 1022;
/// QEMU `running_priority` while nothing is active: above every priority.
const IDLE_PRIORITY: u16 = 0x100;
/// QEMU `GIC_NR_APRS`: 128 preemption levels at the minimum binary point.
const NR_APRS: usize = 4;
/// QEMU `GIC_MIN_BPR` and `GIC_MIN_ABPR`.
const MIN_BPR: u8 = 0;
const MIN_ABPR: u8 = MIN_BPR + 1;

const GICD_CTLR_EN_GRP0: u32 = 1 << 0;
const GICD_CTLR_EN_GRP1: u32 = 1 << 1;
const GICC_CTLR_ACK_CTL: u32 = 1 << 2;
const GICC_CTLR_FIQ_EN: u32 = 1 << 3;
const GICC_CTLR_CBPR: u32 = 1 << 4;
const GICC_CTLR_EOIMODE: u32 = 1 << 9;
/// QEMU `GICC_CTLR_V2_MASK`: the bits a GICv2 without the Security
/// Extensions keeps.
const GICC_CTLR_V2_MASK: u32 = 0x21F;
/// The SGI source bit of processor 0, the one processor there is, in a
/// `GICD_CPENDSGIRn`/`GICD_SPENDSGIRn` byte.
const SGI_SOURCE_CPU0: u8 = 1;

/// The two register sets of the CPU interface (IHI 0048B §4.4.1 AckCtl,
/// Table 4-2).
#[derive(Clone, Copy, PartialEq, Eq)]
enum CpuView {
    /// `GICC_IAR`, `GICC_EOIR` and `GICC_HPPIR`: Group 0, and Group 1 while
    /// `AckCtl` is set — as a Secure access in QEMU.
    Primary,
    /// `GICC_AIAR`, `GICC_AEOIR` and `GICC_AHPPIR`: Group 1 only — as a
    /// Non-secure access to the primary registers in QEMU.
    Group1Alias,
}

/// QEMU `gic_id_gicv2`, for offsets 0xFD0..=0xFFC.
const PERIPHERAL_ID: [u8; 12] = [0x04, 0x00, 0x00, 0x00, 0x90, 0xB4, 0x2B, 0x00, 0x0D, 0xF0, 0x05, 0xB1];

/// The GICv2m frame QEMU's `virt` places at 0x08020000: 64 SPIs from 80.
pub const V2M_BASE_SPI: u32 = 80;
pub const V2M_NUM_SPI: u32 = 64;
const V2M_MSI_TYPER: u64 = 0x008;
const V2M_MSI_SETSPI_NS: u64 = 0x040;
const V2M_MSI_IIDR: u64 = 0xFCC;
/// QEMU `PRODUCT_ID_QEMU`.
const PRODUCT_ID_QEMU: u32 = 0x51;

#[derive(Clone, Copy)]
struct Bits([u32; WORDS]);

impl Bits {
    const fn new() -> Self {
        Self([0; WORDS])
    }
    #[inline]
    fn get(&self, i: usize) -> bool {
        self.0[i / 32] & (1 << (i % 32)) != 0
    }
    #[inline]
    fn set(&mut self, i: usize, v: bool) {
        if v {
            self.0[i / 32] |= 1 << (i % 32);
        } else {
            self.0[i / 32] &= !(1 << (i % 32));
        }
    }
    /// Bits `first..first + 8` as a byte, the ones past the last interrupt
    /// reading as zero.
    fn byte(&self, first: usize) -> u8 {
        let mut b = 0u8;
        for k in 0..8 {
            if first + k < NUM_IRQ && self.get(first + k) {
                b |= 1 << k;
            }
        }
        b
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
    priority: [u8; NUM_IRQ],
    /// `GICC_CTLR`.
    cpu_ctlr: u32,
    pmr: u8,
    bpr: u8,
    abpr: u8,
    /// QEMU `running_priority`: the group priority of the highest-priority
    /// active interrupt, or [`IDLE_PRIORITY`].
    running_priority: u16,
    /// QEMU `apr`: one bit per active Group 0 preemption level.
    apr: [u32; NR_APRS],
    /// QEMU `nsapr`: one bit per active Group 1 preemption level.
    nsapr: [u32; NR_APRS],
    /// QEMU `current_pending`, recomputed by [`GicV2::update`].
    current_pending: u32,
    irq_out: bool,
    fiq_out: bool,
}

impl Default for GicV2 {
    fn default() -> Self {
        Self::new()
    }
}

impl GicV2 {
    /// QEMU `arm_gic_common_reset_hold`: SGIs enabled and edge-triggered,
    /// every other interrupt disabled, level-sensitive, Group 0 and at
    /// priority 0.
    pub fn new() -> Self {
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
            priority: [0; NUM_IRQ],
            cpu_ctlr: 0,
            pmr: 0,
            bpr: MIN_BPR,
            abpr: MIN_ABPR,
            running_priority: IDLE_PRIORITY,
            apr: [0; NR_APRS],
            nsapr: [0; NR_APRS],
            current_pending: SPURIOUS,
            irq_out: false,
            fiq_out: false,
        }
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
        for bits in [&self.enabled, &self.pending, &self.active, &self.group1, &self.level, &self.edge] {
            for word in bits.0 {
                w.write_u32(word)?;
            }
        }
        w.write_bytes(&self.priority)?;
        // The per-SGI source bytes repeat what `pending` holds (processor
        // 0's bit while the SGI is pending). They are written so the image
        // keeps the layout the machine's `SNAPSHOT_VERSION` 5 names, and
        // `restore` refuses bytes that disagree with `pending`.
        let sources: [u8; NR_SGIS] = core::array::from_fn(|sgi| self.sgi_sources(sgi));
        w.write_bytes(&sources)
    }

    /// SGI `sgi`'s `GICD_CPENDSGIRn`/`GICD_SPENDSGIRn` byte: the source bit
    /// of processor 0 while the SGI is pending.
    fn sgi_sources(&self, sgi: usize) -> u8 {
        if self.pending.get(sgi) { SGI_SOURCE_CPU0 } else { 0 }
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
        let mut apr = [0u32; NR_APRS];
        let mut nsapr = [0u32; NR_APRS];
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
        let mut priority = [0u8; NUM_IRQ];
        r.read_bytes(&mut priority)?;
        let mut sgi_sources = [0u8; NR_SGIS];
        r.read_bytes(&mut sgi_sources)?;
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
            irq_out: false,
            fiq_out: false,
        };
        restored.check_restored(&sgi_sources)?;
        *self = restored;
        self.update();
        Ok(())
    }

    /// The one check of a restored image, made before it replaces anything:
    /// refuses binary points the registers cannot hold, a running priority
    /// above idle, an SGI source bit of a processor other than processor 0,
    /// and SGI source bytes that disagree with the SGI's pending state (a
    /// source with the SGI not pending, or the SGI pending with no source).
    fn check_restored(&self, sgi_sources: &[u8; NR_SGIS]) -> SnapResult {
        if self.bpr > 7 || !(MIN_ABPR..=7).contains(&self.abpr) {
            return Err(SnapError::Invalid("GIC binary point outside its register's range"));
        }
        if self.running_priority > IDLE_PRIORITY {
            return Err(SnapError::Invalid("GIC running priority above idle"));
        }
        for (sgi, sources) in sgi_sources.iter().enumerate() {
            if sources & !SGI_SOURCE_CPU0 != 0 {
                return Err(SnapError::Invalid("GIC SGI pending from a processor that does not exist"));
            }
            if *sources != self.sgi_sources(sgi) {
                return Err(SnapError::Invalid("GIC SGI source disagrees with its pending state"));
            }
        }
        Ok(())
    }

    /// Drive input line `intid` (a PPI or SPI) to `level`: QEMU `gic_set_irq`
    /// with `gic_set_irq_generic`.
    pub fn set_line(&mut self, intid: u32, level: bool) {
        let i = intid as usize;
        if !(NR_SGIS..NUM_IRQ).contains(&i) || self.level.get(i) == level {
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
        let group0 = if groups & GICD_CTLR_EN_GRP0 != 0 { u32::MAX } else { 0 };
        let group1 = if groups & GICD_CTLR_EN_GRP1 != 0 { u32::MAX } else { 0 };
        let mut best: Option<usize> = None;
        let mut best_prio = IDLE_PRIORITY;
        for w in 0..WORDS {
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

    /// The groups the CPU interface considers: those the distributor forwards
    /// (IHI 0048B §4.3.1 GICD_CTLR.EnableGrp0/EnableGrp1) and the CPU
    /// interface signals (§4.4.1 GICC_CTLR.EnableGrp0/EnableGrp1). Leaving
    /// out a group the CPU interface does not signal is this model's reading
    /// where IHI 0048B is silent, the rule GICv3 states (IHI 0069
    /// ICC_HPPIR0_EL1/ICC_HPPIR1_EL1, ICC_IAR0_EL1/ICC_IAR1_EL1); QEMU 11.1.2
    /// lets such a group's interrupt block the other's (module docs).
    #[inline]
    fn signalled_groups(&self) -> u32 {
        self.dist_ctlr & self.cpu_ctlr & (GICD_CTLR_EN_GRP0 | GICD_CTLR_EN_GRP1)
    }

    /// IHI 0048B §3.3 "Preemption": a pending interrupt is signalled only if
    /// its group priority — under its own group's binary point ("Priority
    /// grouping", Table 3-3) — is higher than the running priority.
    #[inline]
    fn preempts(&self, irq: usize) -> bool {
        u16::from(self.group_priority(irq)) < self.running_priority
    }

    /// QEMU `gic_update_internal`, with the highest-priority pending
    /// interrupt chosen from the signalled groups only: it becomes the
    /// current pending interrupt if it beats the priority mask (its whole
    /// priority, IHI 0048B §3.3 "Priority masking"), and is signalled if it
    /// also preempts the running priority — as FIQ if it is Group 0 and
    /// `FIQEn` is set (§4.4.1 GICC_CTLR.FIQEn), as IRQ otherwise.
    fn update(&mut self) {
        self.current_pending = SPURIOUS;
        self.irq_out = false;
        self.fiq_out = false;
        let groups = self.signalled_groups();
        if groups == 0 {
            return;
        }
        let Some(best) = self.best_irq(groups) else {
            return;
        };
        if self.priority[best] >= self.pmr {
            return;
        }
        self.current_pending = best as u32;
        if !self.preempts(best) {
            return;
        }
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
            AccessWidth::Half => u64::from(self.dist_readb(off)) | u64::from(self.dist_readb(off + 1)) << 8,
            AccessWidth::Word => u64::from(self.dist_readl(off)),
            AccessWidth::Dword => u64::from(self.dist_readl(off)) | u64::from(self.dist_readl(off + 4)) << 32,
        }
    }

    fn dist_readl(&self, off: u64) -> u32 {
        (0..4).fold(0, |v, b| v | u32::from(self.dist_readb(off + b)) << (8 * b))
    }

    /// The first interrupt a byte of a one-bit-per-interrupt bank describes,
    /// if it exists.
    fn bank_irq(off: u64, base: u64) -> Option<usize> {
        let irq = ((off - base) * 8) as usize;
        (irq < NUM_IRQ).then_some(irq)
    }

    /// QEMU `gic_dist_readb`.
    fn dist_readb(&self, off: u64) -> u8 {
        match off {
            0x000 => self.dist_ctlr as u8,
            0x004 => ((NUM_IRQ / 32) - 1) as u8,
            0x008 => 0x3B,
            0x009 => 0x04,
            0x080..=0x0FF => Self::bank_irq(off, 0x080).map_or(0, |irq| self.group1.byte(irq)),
            0x100..=0x17F => Self::bank_irq(off, 0x100).map_or(0, |irq| self.enabled.byte(irq)),
            0x180..=0x1FF => Self::bank_irq(off, 0x180).map_or(0, |irq| self.enabled.byte(irq)),
            0x200..=0x2FF => {
                let base = if off < 0x280 { 0x200 } else { 0x280 };
                Self::bank_irq(off, base).map_or(0, |irq| {
                    let mut b = 0u8;
                    for k in 0..8 {
                        if irq + k < NUM_IRQ && self.test_pending(irq + k) {
                            b |= 1 << k;
                        }
                    }
                    b
                })
            }
            0x300..=0x37F => Self::bank_irq(off, 0x300).map_or(0, |irq| self.active.byte(irq)),
            0x380..=0x3FF => Self::bank_irq(off, 0x380).map_or(0, |irq| self.active.byte(irq)),
            0x400..=0x7FF => self.priority.get((off - 0x400) as usize).copied().unwrap_or(0),
            // GICD_ITARGETSR: RAZ/WI on a uniprocessor GIC.
            0x800..=0xBFF => 0,
            0xC00..=0xEFF => {
                // Two bits per interrupt: bit 1 of each pair is edge, bit 0
                // (the 11MPCore's N-N model) reads as zero on a GICv2.
                let first = ((off - 0xC00) * 4) as usize;
                let mut b = 0u8;
                for k in 0..4 {
                    if first + k < NUM_IRQ && self.edge.get(first + k) {
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

    /// For each bit set in `v`, set or clear interrupt `first + k`.
    fn set_each(bits: &mut Bits, first: usize, v: u8, value: bool) {
        for k in 0..8 {
            if v & (1 << k) != 0 && first + k < NUM_IRQ {
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
                        if irq + k < NUM_IRQ {
                            self.group1.set(irq + k, v & (1 << k) != 0);
                        }
                    }
                }
            }
            0x100..=0x17F => {
                if let Some(irq) = Self::bank_irq(off, 0x100) {
                    // SGIs cannot be disabled: any write enables them.
                    let v = if irq < NR_SGIS { 0xFF } else { v };
                    Self::set_each(&mut self.enabled, irq, v, true);
                }
            }
            0x180..=0x1FF => {
                if let Some(irq) = Self::bank_irq(off, 0x180) {
                    let v = if irq < NR_SGIS { 0 } else { v };
                    Self::set_each(&mut self.enabled, irq, v, false);
                }
            }
            0x200..=0x27F => {
                // SGIs are made pending only through GICD_SGIR and
                // GICD_SPENDSGIR.
                if let Some(irq) = Self::bank_irq(off, 0x200) {
                    let v = if irq < NR_SGIS { 0 } else { v };
                    Self::set_each(&mut self.pending, irq, v, true);
                }
            }
            0x280..=0x2FF => {
                if let Some(irq) = Self::bank_irq(off, 0x280) {
                    let v = if irq < NR_SGIS { 0 } else { v };
                    Self::set_each(&mut self.pending, irq, v, false);
                }
            }
            0x300..=0x37F => {
                if let Some(irq) = Self::bank_irq(off, 0x300) {
                    Self::set_each(&mut self.active, irq, v, true);
                }
            }
            0x380..=0x3FF => {
                if let Some(irq) = Self::bank_irq(off, 0x380) {
                    Self::set_each(&mut self.active, irq, v, false);
                }
            }
            // QEMU `gic_dist_set_priority`: with 8 priority bits nothing is
            // masked off.
            0x400..=0x7FF => {
                if let Some(p) = self.priority.get_mut((off - 0x400) as usize) {
                    *p = v;
                }
            }
            0xC00..=0xEFF => {
                let first = ((off - 0xC00) * 4) as usize;
                if first < NUM_IRQ {
                    // SGIs stay edge-triggered.
                    let v = if first < NR_SGIS { v | 0xAA } else { v };
                    for k in 0..4 {
                        if first + k < NUM_IRQ {
                            self.edge.set(first + k, v & (2 << (2 * k)) != 0);
                        }
                    }
                }
            }
            // GICD_CPENDSGIRn and GICD_SPENDSGIRn (IHI 0048B): a 1 removes or
            // adds the SGI's pending state from that source processor, a 0
            // has no effect; only processor 0's bit exists.
            0xF10..=0xF1F if v & SGI_SOURCE_CPU0 != 0 => self.pending.set((off - 0xF10) as usize, false),
            0xF20..=0xF2F if v & SGI_SOURCE_CPU0 != 0 => self.pending.set((off - 0xF20) as usize, true),
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

    /// QEMU `gic_get_current_pending_irq`: the `GICC_HPPIR` or `GICC_AHPPIR`
    /// value. Through the primary registers a Group 1 interrupt reads as
    /// 1022 unless `AckCtl` is set (IHI 0048B §4.4.1 AckCtl); through the
    /// Group 1 aliases a Group 0 interrupt reads as 1023, as for QEMU's
    /// Non-secure access. Its CPUID field is RAZ, as on any uniprocessor.
    fn current_pending_irq(&self, view: CpuView) -> u32 {
        let irq = self.current_pending;
        if irq >= MAXIRQ {
            return irq;
        }
        let group1 = self.group1.get(irq as usize);
        match view {
            CpuView::Primary if group1 && self.cpu_ctlr & GICC_CTLR_ACK_CTL == 0 => SPURIOUS_GROUP1,
            CpuView::Group1Alias if !group1 => SPURIOUS,
            CpuView::Primary | CpuView::Group1Alias => irq,
        }
    }

    /// QEMU `gic_get_group_priority`: the priority's bits above the binary
    /// point — `GICC_ABPR` minus one for a Group 1 interrupt unless `CBPR`
    /// is set, `GICC_BPR` otherwise.
    fn group_priority(&self, irq: usize) -> u8 {
        let bpr = if self.cpu_ctlr & GICC_CTLR_CBPR == 0 && self.group1.get(irq) {
            self.abpr - 1
        } else {
            self.bpr
        };
        self.priority[irq] & (0xFFu32 << ((bpr & 7) + 1)) as u8
    }

    /// QEMU `gic_activate_irq`.
    fn activate(&mut self, irq: usize) {
        let prio = self.group_priority(irq);
        let level = usize::from(prio >> (MIN_BPR + 1));
        let bank = if self.group1.get(irq) { &mut self.nsapr } else { &mut self.apr };
        bank[level / 32] |= 1 << (level % 32);
        self.running_priority = u16::from(prio);
        self.active.set(irq, true);
    }

    /// QEMU `gic_get_prio_from_apr_bits`.
    fn prio_from_apr_bits(&self) -> u16 {
        for i in 0..NR_APRS {
            let apr = self.apr[i] | self.nsapr[i];
            if apr != 0 {
                return ((i as u32 * 32 + apr.trailing_zeros()) << (MIN_BPR + 1)) as u16;
            }
        }
        IDLE_PRIORITY
    }

    /// QEMU `gic_drop_prio`: clear the lowest set bit of the group's active
    /// priority registers, which the nesting of acknowledgements and EOIs
    /// makes the running interrupt's.
    fn drop_prio(&mut self, group1: bool) {
        let bank = if group1 { &mut self.nsapr } else { &mut self.apr };
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
        let irq = self.current_pending_irq(view);
        if irq >= MAXIRQ {
            return irq;
        }
        let i = irq as usize;
        if !self.preempts(i) {
            return SPURIOUS;
        }
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
        let i = irq as usize;
        if i >= NUM_IRQ || self.running_priority == IDLE_PRIORITY {
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
    /// set.
    fn deactivate(&mut self, irq: u32) {
        let i = irq as usize;
        if irq >= MAXIRQ || i >= NUM_IRQ || !self.eoi_split() {
            return;
        }
        self.active.set(i, false);
    }

    pub fn cpu_read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        let off = offset.get();
        match width {
            AccessWidth::Dword => {
                let low = self.cpu_read_register(off);
                let high = self.cpu_read_register(off + 4);
                u64::from(low) | u64::from(high) << 32
            }
            AccessWidth::Byte | AccessWidth::Half | AccessWidth::Word => u64::from(self.cpu_read_register(off)),
        }
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
            0x18 => self.current_pending_irq(CpuView::Primary),
            0x1C => u32::from(self.abpr),
            0x20 => self.acknowledge(CpuView::Group1Alias),
            0x28 => self.current_pending_irq(CpuView::Group1Alias),
            0xD0 | 0xD4 | 0xD8 | 0xDC => self.apr[((off - 0xD0) / 4) as usize],
            0xE0 | 0xE4 | 0xE8 | 0xEC => self.nsapr[((off - 0xE0) / 4) as usize],
            0xFC => (2 << 16) | 0x43B,
            _ => 0,
        }
    }

    pub fn cpu_write(&mut self, offset: RegOffset, width: AccessWidth, value: u64) {
        let off = offset.get();
        match width {
            AccessWidth::Dword => {
                self.cpu_write_register(off, value as u32);
                self.cpu_write_register(off + 4, (value >> 32) as u32);
            }
            AccessWidth::Byte | AccessWidth::Half | AccessWidth::Word => self.cpu_write_register(off, value as u32),
        }
        self.update();
    }

    /// QEMU `gic_cpu_write`, with the Group 1 alias `GICC_AEOIR` (0x24) of
    /// IHI 0048B Table 4-2, which QEMU ignores.
    fn cpu_write_register(&mut self, off: u64, v: u32) {
        match off {
            0x00 => self.cpu_ctlr = v & GICC_CTLR_V2_MASK,
            0x04 => self.pmr = v as u8,
            // QEMU raises a written binary point to `GIC_MIN_BPR`, which is
            // 0, so every value stands.
            0x08 => self.bpr = v as u8 & 7,
            0x10 => self.complete(v & 0x3FF, CpuView::Primary),
            0x1C => self.abpr = (v as u8 & 7).max(MIN_ABPR),
            0x24 => self.complete(v & 0x3FF, CpuView::Group1Alias),
            0xD0 | 0xD4 | 0xD8 | 0xDC => {
                self.apr[((off - 0xD0) / 4) as usize] = v;
                self.running_priority = self.prio_from_apr_bits();
            }
            0xE0 | 0xE4 | 0xE8 | 0xEC => {
                self.nsapr[((off - 0xE0) / 4) as usize] = v;
                self.running_priority = self.prio_from_apr_bits();
            }
            0x1000 => self.deactivate(v & 0x3FF),
            _ => {}
        }
    }

    // ----- GICv2m MSI frame -----

    pub fn v2m_read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        let off = offset.get();
        match width {
            AccessWidth::Byte | AccessWidth::Half => 0,
            AccessWidth::Word => u64::from(Self::v2m_register(off)),
            AccessWidth::Dword => u64::from(Self::v2m_register(off)) | u64::from(Self::v2m_register(off + 4)) << 32,
        }
    }

    /// QEMU `gicv2m_read` for a 4-byte access (any other size reads as zero).
    fn v2m_register(off: u64) -> u32 {
        match off {
            V2M_MSI_TYPER => (V2M_BASE_SPI << 16) | V2M_NUM_SPI,
            V2M_MSI_IIDR => PRODUCT_ID_QEMU << 20,
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
    /// pending only if it is configured edge-triggered.
    fn v2m_write_register(&mut self, off: u64, v: u32) {
        if off == V2M_MSI_SETSPI_NS {
            let spi = v & 0x3FF;
            if (V2M_BASE_SPI..V2M_BASE_SPI + V2M_NUM_SPI).contains(&spi) {
                self.set_line(spi, true);
                self.set_line(spi, false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(g: &mut GicV2, off: u64, v: u32) {
        g.dist_write(RegOffset::new(off), AccessWidth::Word, u64::from(v));
    }

    fn cpu(g: &mut GicV2, off: u64, v: u32) {
        g.cpu_write(RegOffset::new(off), AccessWidth::Word, u64::from(v));
    }

    fn cpu_get(g: &mut GicV2, off: u64) -> u32 {
        g.cpu_read(RegOffset::new(off), AccessWidth::Word) as u32
    }

    fn dist_get(g: &mut GicV2, off: u64) -> u32 {
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
        let mut g = GicV2::new();
        dist(&mut g, 0x000, 3);
        cpu(&mut g, CTLR, 3);
        cpu(&mut g, PMR, 0xFF);
        g
    }

    /// Make interrupt `irq` edge-triggered (its `GICD_ICFGR` byte).
    fn edge_triggered(g: &mut GicV2, irq: u32) {
        let byte = 2u8 << (2 * (irq % 4));
        g.dist_write(RegOffset::new(0xC00 + u64::from(irq / 4)), AccessWidth::Byte, u64::from(byte));
    }

    /// Enable SPI `irq` at `priority`, and raise its line.
    fn raise(g: &mut GicV2, irq: u32, priority: u8) {
        g.dist_write(RegOffset::new(0x400 + u64::from(irq)), AccessWidth::Byte, u64::from(priority));
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

    /// The priority mask hides an interrupt whose priority is not higher.
    #[test]
    fn pmr_masks_lower_priority_interrupts() {
        let mut g = enabled();
        cpu(&mut g, PMR, 0x80);
        raise(&mut g, 32, 0x80);
        assert!(!g.irq_out());
        assert_eq!(cpu_get(&mut g, HPPIR), 1023);
        cpu(&mut g, PMR, 0x81);
        assert!(g.irq_out());
    }

    #[test]
    fn sgis_are_enabled_from_reset_and_stay_enabled() {
        let mut g = GicV2::new();
        assert_eq!(dist_get(&mut g, 0x100) & 0xFFFF, 0xFFFF);
        dist(&mut g, 0x180, 0xFFFF_FFFF);
        assert_eq!(dist_get(&mut g, 0x100), 0xFFFF, "ICENABLER cannot disable an SGI");
        dist(&mut g, 0xC00, 0);
        assert_eq!(dist_get(&mut g, 0xC00), 0xAAAA_AAAA, "nor make one level-sensitive");
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
        assert_eq!(cpu_get(&mut g, IAR), 1023, "acknowledged once: no other source");
        assert_eq!(dist_get(&mut g, 0xF24), 0);
        // GICD_CPENDSGIRn clears it only through processor 0's bit.
        g.dist_write(RegOffset::new(0xF20 + 2), AccessWidth::Byte, 1);
        g.dist_write(RegOffset::new(0xF10 + 2), AccessWidth::Byte, 0xFE);
        assert_eq!(cpu_get(&mut g, HPPIR), 2, "other processors' bits clear nothing");
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
        assert_eq!(cpu_get(&mut g, IAR), 1023, "SGIs 1 to 3 were not made pending");
    }

    #[test]
    fn sgir_target_filters_on_a_uniprocessor() {
        let mut g = enabled();
        dist(&mut g, 0xF00, (1 << 24) | 1);
        assert!(!g.irq_out(), "all others: nobody");
        dist(&mut g, 0xF00, 1 << 16 | 2);
        assert_eq!(cpu_get(&mut g, IAR), 2, "a target list naming processor 0");
        dist(&mut g, 0xF00, (3 << 24) | 3);
        assert_eq!(cpu_get(&mut g, HPPIR), 3, "the reserved filter names every processor");
        g.dist_write(RegOffset::new(0xF00), AccessWidth::Byte, 4);
        assert_eq!(dist_get(&mut g, 0x200) & (1 << 4), 0, "GICD_SGIR is written only whole");
    }

    #[test]
    fn the_running_priority_is_the_group_priority() {
        let mut g = enabled();
        assert_eq!(cpu_get(&mut g, RPR), 0xFF, "idle");
        cpu(&mut g, BPR, 2);
        raise(&mut g, 40, 0x45);
        assert_eq!(cpu_get(&mut g, IAR), 40);
        assert_eq!(cpu_get(&mut g, RPR), 0x40, "bits [7:3] at BPR 2");
        // Preemption level 0x40 >> 1 = 32: bit 0 of the second register.
        assert_eq!(cpu_get(&mut g, APR0 + 4), 1);
        assert_eq!(cpu_get(&mut g, APR0), 0);
        // Preemption compares group priorities: 0x41 is 0x40 at BPR 2.
        raise(&mut g, 41, 0x41);
        assert!(!g.irq_out(), "0x41 does not preempt 0x40");
        raise(&mut g, 42, 0x3F);
        assert!(g.irq_out(), "0x3F does");
        assert_eq!(cpu_get(&mut g, IAR), 42);
        assert_eq!(cpu_get(&mut g, RPR), 0x38);
        cpu(&mut g, EOIR, 42);
        assert_eq!(cpu_get(&mut g, RPR), 0x40);
        cpu(&mut g, EOIR, 40);
        assert_eq!(cpu_get(&mut g, RPR), 0xFF);
    }

    /// IHI 0048B §3.3 "Preemption" and "Priority grouping": with CBPR clear
    /// each group's binary point sets its own group priority, and a pending
    /// interrupt preempts when that is higher than the running priority.
    #[test]
    fn preemption_compares_group_priorities_under_each_groups_binary_point() {
        let mut g = enabled();
        cpu(&mut g, BPR, 2);
        cpu(&mut g, ABPR, 1);
        dist(&mut g, 0x084, 1 << 8);
        raise(&mut g, 40, 0x42);
        assert_eq!(cpu_get(&mut g, AIAR), 40);
        assert_eq!(cpu_get(&mut g, RPR), 0x42, "ABPR 1: Group 1 bits [7:1]");
        raise(&mut g, 42, 0x48);
        assert!(!g.irq_out(), "0x48 at BPR 2 is group priority 0x48, not higher than 0x42");
        raise(&mut g, 41, 0x43);
        assert!(g.irq_out(), "0x43 at BPR 2 is group priority 0x40, higher than 0x42");
        assert_eq!(cpu_get(&mut g, HPPIR), 41);
        assert_eq!(cpu_get(&mut g, IAR), 41);
        assert_eq!(cpu_get(&mut g, RPR), 0x40);
        cpu(&mut g, EOIR, 41);
        assert_eq!(cpu_get(&mut g, RPR), 0x42);
        cpu(&mut g, AEOIR, 40);
        assert_eq!(cpu_get(&mut g, RPR), 0xFF);
    }

    #[test]
    fn group_1_uses_the_aliased_binary_point_unless_cbpr() {
        let mut g = enabled();
        cpu(&mut g, CTLR, 3 | GICC_CTLR_ACK_CTL);
        dist(&mut g, 0x084, 1 << 8);
        cpu(&mut g, ABPR, 4);
        raise(&mut g, 40, 0x5F);
        assert_eq!(cpu_get(&mut g, IAR), 40);
        assert_eq!(cpu_get(&mut g, RPR), 0x50, "ABPR 4 means binary point 3: bits [7:4]");
        // Preemption level 0x50 >> 1 = 40: bit 8 of the second register.
        assert_eq!(cpu_get(&mut g, NSAPR0 + 4), 1 << 8);
        assert_eq!(cpu_get(&mut g, APR0 + 4), 0, "Group 1 levels are in GICC_NSAPRn");
        cpu(&mut g, EOIR, 40);
        cpu(&mut g, CTLR, 3 | GICC_CTLR_ACK_CTL | GICC_CTLR_CBPR);
        g.set_line(40, false);
        g.set_line(40, true);
        assert_eq!(cpu_get(&mut g, IAR), 40);
        assert_eq!(cpu_get(&mut g, RPR), 0x5E, "BPR 0 with CBPR: bits [7:1]");
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

    /// IHI 0048B §4.3.1: GICD_CTLR.EnableGrp1 clear stops the distributor
    /// forwarding Group 1, so its interrupt is neither signalled nor read, and
    /// the Group 0 interrupt behind it is.
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
        assert_eq!(cpu_get(&mut g, AHPPIR), 1023, "no Group 1 interrupt is forwarded");
        assert_eq!(cpu_get(&mut g, AIAR), 1023);
        assert_eq!(cpu_get(&mut g, IAR), 41);
        cpu(&mut g, EOIR, 41);
        g.set_line(41, false);
        assert!(!g.irq_out());
        assert_eq!(cpu_get(&mut g, IAR), 1023, "AckCtl does not reach an unforwarded group");
        assert_eq!(dist_get(&mut g, 0x304), 0, "interrupt 40 never became active");
        dist(&mut g, 0x000, 3);
        assert_eq!(cpu_get(&mut g, IAR), 40, "forwarded once its group is enabled");
    }

    /// IHI 0048B §4.4.1: GICC_CTLR.EnableGrp0 clear stops the CPU interface
    /// signalling Group 0, and §4.4.4: GICC_IAR then returns 1023 rather than
    /// a Group 0 interrupt. That the Group 0 interrupt does not hide the
    /// Group 1 one is this model's reading where IHI 0048B is silent, the
    /// rule GICv3 states (IHI 0069 ICC_HPPIR1_EL1, ICC_IAR1_EL1); QEMU 11.1.2
    /// lets it block the Group 1 interrupt.
    #[test]
    fn a_group_the_cpu_interface_does_not_signal_hides_nothing() {
        let mut g = enabled();
        cpu(&mut g, CTLR, 2);
        dist(&mut g, 0x084, 1 << 9);
        raise(&mut g, 40, 0x10);
        raise(&mut g, 41, 0x20);
        assert!(g.irq_out(), "the Group 1 interrupt is signalled");
        assert_eq!(cpu_get(&mut g, HPPIR), 1022, "Group 1, AckCtl clear");
        assert_eq!(cpu_get(&mut g, AHPPIR), 41);
        assert_eq!(cpu_get(&mut g, AIAR), 41);
        cpu(&mut g, AEOIR, 41);
        g.set_line(41, false);
        assert!(!g.irq_out());
        assert_eq!(cpu_get(&mut g, IAR), 1023, "Group 0 is not signalled, so not acknowledged");
        assert_eq!(dist_get(&mut g, 0x304), 0);
    }

    #[test]
    fn eoi_while_idle_and_dir_without_eoimode_are_ignored() {
        let mut g = enabled();
        dist(&mut g, 0x304, 1 << 8);
        cpu(&mut g, EOIR, 40);
        assert_eq!(dist_get(&mut g, 0x304), 1 << 8, "nothing was running, so EOI does nothing");
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
        assert_eq!(cpu_get(&mut g, NSAPR0), 1 << 8, "Group 1 preemption level 8");
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
        assert_eq!(cpu_get(&mut g, RPR), 0x20, "GICC_AEOIR ignores a Group 0 interrupt");
        cpu(&mut g, EOIR, 41);
        assert_eq!(cpu_get(&mut g, RPR), 0xFF);
    }

    #[test]
    fn an_eight_byte_access_is_two_registers() {
        let mut g = enabled();
        raise(&mut g, 40, 0x10);
        let pair = g.cpu_read(RegOffset::new(BPR), AccessWidth::Dword);
        assert_eq!(pair, 40 << 32, "GICC_BPR, then GICC_IAR acknowledges");
        g.cpu_write(RegOffset::new(EOIR), AccessWidth::Dword, 40);
        assert_eq!(cpu_get(&mut g, RPR), 0xFF);
        g.dist_write(RegOffset::new(0x400 + 40), AccessWidth::Dword, 0x0000_0099_0000_0077);
        assert_eq!(g.dist_read(RegOffset::new(0x400 + 40), AccessWidth::Byte), 0x77);
        assert_eq!(g.dist_read(RegOffset::new(0x400 + 44), AccessWidth::Byte), 0x99);
    }

    #[test]
    fn an_msi_latches_only_an_edge_triggered_spi() {
        let mut g = enabled();
        dist(&mut g, 0x100 + 80 / 32 * 4, 1 << (80 % 32));
        g.v2m_write(RegOffset::new(V2M_MSI_SETSPI_NS), AccessWidth::Word, 80);
        assert!(!g.irq_out(), "a level-sensitive SPI forgets the pulse");
        edge_triggered(&mut g, 80);
        g.v2m_write(RegOffset::new(V2M_MSI_SETSPI_NS), AccessWidth::Byte, 80);
        assert!(!g.irq_out(), "a byte write is ignored");
        g.v2m_write(RegOffset::new(V2M_MSI_SETSPI_NS), AccessWidth::Half, 80);
        assert_eq!(cpu_get(&mut g, IAR), 80);
        assert_eq!(g.v2m_read(RegOffset::new(V2M_MSI_TYPER), AccessWidth::Byte), 0);
        assert_eq!(g.v2m_read(RegOffset::new(V2M_MSI_TYPER), AccessWidth::Word), u64::from((80u32 << 16) | 64));
    }

    #[test]
    fn a_snapshot_round_trips_the_priority_state() {
        let mut g = enabled();
        cpu(&mut g, BPR, 1);
        raise(&mut g, 40, 0x45);
        dist(&mut g, 0xF00, (2 << 24) | 3);
        assert_eq!(cpu_get(&mut g, IAR), 3);
        let mut image = std::vec::Vec::new();
        g.save(&mut image).expect("save");
        let mut restored = GicV2::new();
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restore");
        assert!(source.is_empty(), "the restore consumed exactly what the save wrote");
        assert_eq!(cpu_get(&mut restored, RPR), 0);
        cpu(&mut restored, EOIR, 3);
        assert_eq!(cpu_get(&mut restored, IAR), 40);
        assert_eq!(cpu_get(&mut restored, RPR), 0x44);
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
        // The image ends with the SGIs' source bytes; SGI 3 is pending.
        let source_byte = |sgi: usize| image.len() - NR_SGIS + sgi;
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
        target.restore(&mut source).expect("the image as saved restores");
        assert_eq!(cpu_get(&mut target, HPPIR), 3);
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
        assert_eq!(cpu_get(&mut g, AHPPIR), 1023, "the highest pending is Group 0");
        assert_eq!(cpu_get(&mut g, AIAR), 1023);
        assert_eq!(dist_get(&mut g, 0x304), 0, "nothing acknowledged");
        assert_eq!(cpu_get(&mut g, HPPIR), 41);
        assert_eq!(cpu_get(&mut g, IAR), 41);
        cpu(&mut g, AEOIR, 41);
        assert_eq!(cpu_get(&mut g, RPR), 0x10, "GICC_AEOIR ignores a Group 0 interrupt");
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
