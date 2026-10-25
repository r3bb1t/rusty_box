//! GICv2 distributor and CPU interface for one processor, without the
//! Security Extensions (QEMU's `virt` without EL3), plus the GICv2m MSI
//! frame.
//!
//! Provenance (R7): register layout and semantics from the Arm GICv2
//! architecture specification (IHI 0048B); identification values, reset
//! values and the GICv2m frame from QEMU `hw/intc/arm_gic.c` and
//! `hw/intc/arm_gicv2m.c` (`gic_id_gicv2`, `GIC_MIN_BPR`, `V2M_MSI_TYPER`).
//!
//! Interrupt state is recomputed after every register access and every
//! input line change; the machine reads `irq_out`/`fiq_out` between
//! instructions.

use crate::bus::{AccessWidth, RegOffset};

/// SGIs, PPIs and QEMU `virt`'s 256 SPIs.
pub const NUM_IRQ: usize = 288;
const WORDS: usize = NUM_IRQ / 32;
const SPURIOUS: u32 = 1023;
const SPURIOUS_GROUP1: u32 = 1022;

/// QEMU `gic_id_gicv2`, for offsets 0xFD0..=0xFFC.
const PERIPHERAL_ID: [u8; 12] = [0x04, 0x00, 0x00, 0x00, 0x90, 0xB4, 0x2B, 0x00, 0x0D, 0xF0, 0x05, 0xB1];

/// The GICv2m frame QEMU's `virt` places at 0x08020000: 64 SPIs from 80.
pub const V2M_BASE_SPI: u32 = 80;
pub const V2M_NUM_SPI: u32 = 64;

/// The set/clear-enable, -pending and -active register semantics: for each
/// bit set in `v`, set (or clear) the state of interrupt `first + k`, for
/// interrupts at or above `lowest`.
fn set_each(bits: &mut Bits, v: u8, first: usize, lowest: usize, value: bool) {
    for k in 0..8 {
        let i = first + k;
        if v & (1 << k) != 0 && i >= lowest && i < NUM_IRQ {
            bits.set(i, value);
        }
    }
}

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
}

/// The interrupt controller.
pub struct GicV2 {
    dist_ctlr: u32,
    cpu_ctlr: u32,
    pmr: u8,
    bpr: u8,
    abpr: u8,
    enabled: Bits,
    /// Latched pending state: edge-triggered arrivals and software sets.
    pending_latch: Bits,
    active: Bits,
    group1: Bits,
    /// Current level of each input line.
    level: Bits,
    /// Edge-triggered (ICFGR bit 2n+1).
    edge: Bits,
    priority: [u8; NUM_IRQ],
    targets: [u8; NUM_IRQ],
    /// Acknowledged interrupts whose priority has not been dropped, oldest
    /// first: the running priority is the last one's.
    stack: [u16; 64],
    stack_len: usize,
    irq_out: bool,
    fiq_out: bool,
}

impl Default for GicV2 {
    fn default() -> Self {
        Self::new()
    }
}

impl GicV2 {
    pub fn new() -> Self {
        let mut edge = Bits::new();
        for sgi in 0..16 {
            edge.set(sgi, true);
        }
        let mut targets = [0u8; NUM_IRQ];
        for t in targets.iter_mut().take(32) {
            *t = 1;
        }
        Self {
            dist_ctlr: 0,
            cpu_ctlr: 0,
            pmr: 0,
            bpr: 0,
            abpr: 1,
            enabled: Bits::new(),
            pending_latch: Bits::new(),
            active: Bits::new(),
            group1: Bits::new(),
            level: Bits::new(),
            edge,
            priority: [0; NUM_IRQ],
            targets,
            stack: [0; 64],
            stack_len: 0,
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

    /// Drive input line `intid` (a PPI or SPI) to `level`.
    pub fn set_line(&mut self, intid: u32, level: bool) {
        let i = intid as usize;
        if !(16..NUM_IRQ).contains(&i) {
            return;
        }
        let was = self.level.get(i);
        if was == level {
            return;
        }
        self.level.set(i, level);
        if self.edge.get(i) && level && !was {
            self.pending_latch.set(i, true);
        }
        self.update();
    }

    /// Pulse an edge on SPI `intid` (the GICv2m `MSI_SETSPI_NS` path).
    fn pulse(&mut self, intid: u32) {
        let i = intid as usize;
        if (32..NUM_IRQ).contains(&i) {
            self.pending_latch.set(i, true);
            self.update();
        }
    }

    #[inline]
    fn pending(&self, i: usize) -> bool {
        self.pending_latch.get(i) || (!self.edge.get(i) && self.level.get(i))
    }

    #[inline]
    fn group_enabled(&self, i: usize) -> bool {
        if self.group1.get(i) {
            self.dist_ctlr & 2 != 0 && self.cpu_ctlr & 2 != 0
        } else {
            self.dist_ctlr & 1 != 0 && self.cpu_ctlr & 1 != 0
        }
    }

    /// The mask selecting the group priority bits for preemption: bits
    /// `[7:BPR+1]`. At BPR 7 no bit is a group priority bit, so nothing
    /// preempts.
    #[inline]
    fn group_priority_mask(&self) -> u8 {
        (0xFFu16 << (u16::from(self.bpr.min(7)) + 1)) as u8
    }

    fn running_priority(&self) -> u8 {
        match self.stack_len {
            0 => 0xFF,
            n => self.priority[self.stack[n - 1] as usize],
        }
    }

    /// The highest-priority interrupt that is pending, enabled, not active,
    /// in an enabled group, unmasked by PMR and able to preempt the running
    /// priority.
    fn best_pending(&self) -> Option<usize> {
        let mut best: Option<usize> = None;
        let mask = self.group_priority_mask();
        let running = self.running_priority();
        for i in 0..NUM_IRQ {
            if !self.enabled.get(i) || self.active.get(i) || !self.pending(i) || !self.group_enabled(i) {
                continue;
            }
            let p = self.priority[i];
            if p >= self.pmr || (running != 0xFF && (p & mask) >= (running & mask)) {
                continue;
            }
            match best {
                Some(b) if self.priority[b] <= p => {}
                _ => best = Some(i),
            }
        }
        best
    }

    fn update(&mut self) {
        let best = self.best_pending();
        let fiq_en = self.cpu_ctlr & (1 << 3) != 0;
        match best {
            Some(i) if !self.group1.get(i) && fiq_en => {
                self.fiq_out = true;
                self.irq_out = false;
            }
            Some(_) => {
                self.irq_out = true;
                self.fiq_out = false;
            }
            None => {
                self.irq_out = false;
                self.fiq_out = false;
            }
        }
    }

    // ----- distributor -----

    pub fn dist_read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        let off = offset.get();
        let bytes = width.bytes();
        let mut v = 0u64;
        for b in 0..bytes.min(4) {
            v |= u64::from(self.dist_read_byte(off + b)) << (8 * b);
        }
        v
    }

    fn dist_read_byte(&self, off: u64) -> u8 {
        let word_bit = |bits: &Bits, base: u64| -> u8 {
            let first = ((off - base) * 8) as usize;
            let mut b = 0u8;
            for k in 0..8 {
                if first + k < NUM_IRQ && bits.get(first + k) {
                    b |= 1 << k;
                }
            }
            b
        };
        match off {
            0x000 => self.dist_ctlr as u8,
            0x004 => ((NUM_IRQ / 32) - 1) as u8,
            0x008 => 0x3B,
            0x009 => 0x04,
            0x001..=0x003 | 0x005..=0x007 | 0x00A..=0x07F => 0,
            0x080..=0x0BF => word_bit(&self.group1, 0x080),
            0x100..=0x13F => word_bit(&self.enabled, 0x100),
            0x180..=0x1BF => word_bit(&self.enabled, 0x180),
            0x200..=0x23F | 0x280..=0x2BF => {
                let first = ((off & 0x3F) * 8) as usize;
                let mut b = 0u8;
                for k in 0..8 {
                    if first + k < NUM_IRQ && self.pending(first + k) {
                        b |= 1 << k;
                    }
                }
                b
            }
            0x300..=0x33F => word_bit(&self.active, 0x300),
            0x380..=0x3BF => word_bit(&self.active, 0x380),
            0x400..=0x7FF => self.priority.get((off - 0x400) as usize).copied().unwrap_or(0),
            0x800..=0xBFF => self.targets.get((off - 0x800) as usize).copied().unwrap_or(0),
            0xC00..=0xCFF => {
                // Two bits per interrupt, bit 1 of each pair = edge.
                let first = ((off - 0xC00) * 4) as usize;
                let mut b = 0u8;
                for k in 0..4 {
                    if first + k < NUM_IRQ && self.edge.get(first + k) {
                        b |= 2 << (2 * k);
                    }
                }
                b
            }
            0xF10..=0xF2F => {
                // CPENDSGIR/SPENDSGIR: bit 0 of each SGI's byte is CPU 0.
                let sgi = ((off - 0xF10) % 16) as usize;
                u8::from(self.pending_latch.get(sgi))
            }
            0xFD0..=0xFFF => {
                let idx = ((off - 0xFD0) / 4) as usize;
                if off.is_multiple_of(4) {
                    PERIPHERAL_ID.get(idx).copied().unwrap_or(0)
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    pub fn dist_write(&mut self, offset: RegOffset, width: AccessWidth, value: u64) {
        let off = offset.get();
        if off == 0xF00 {
            self.write_sgir(value as u32);
            self.update();
            return;
        }
        for b in 0..width.bytes().min(4) {
            self.dist_write_byte(off + b, (value >> (8 * b)) as u8);
        }
        self.update();
    }

    fn dist_write_byte(&mut self, off: u64, v: u8) {
        match off {
            0x000 => self.dist_ctlr = u32::from(v & 3),
            0x080..=0x0BF => {
                let first = ((off - 0x080) * 8) as usize;
                for k in 0..8 {
                    if first + k < NUM_IRQ {
                        self.group1.set(first + k, v & (1 << k) != 0);
                    }
                }
            }
            0x100..=0x13F => set_each(&mut self.enabled, v, ((off - 0x100) * 8) as usize, 0, true),
            0x180..=0x1BF => set_each(&mut self.enabled, v, ((off - 0x180) * 8) as usize, 0, false),
            // SGIs are made pending only through GICD_SGIR/SPENDSGIR.
            0x200..=0x23F => set_each(&mut self.pending_latch, v, ((off - 0x200) * 8) as usize, 16, true),
            0x280..=0x2BF => set_each(&mut self.pending_latch, v, ((off - 0x280) * 8) as usize, 16, false),
            0x300..=0x33F => set_each(&mut self.active, v, ((off - 0x300) * 8) as usize, 0, true),
            0x380..=0x3BF => set_each(&mut self.active, v, ((off - 0x380) * 8) as usize, 0, false),
            0x400..=0x7FF => {
                if let Some(p) = self.priority.get_mut((off - 0x400) as usize) {
                    *p = v;
                }
            }
            0x800..=0xBFF => {
                // SGI and PPI targets are read-only: this processor.
                let i = (off - 0x800) as usize;
                if i >= 32 {
                    if let Some(t) = self.targets.get_mut(i) {
                        *t = v & 1;
                    }
                }
            }
            0xC00..=0xCFF => {
                let first = ((off - 0xC00) * 4) as usize;
                for k in 0..4 {
                    let i = first + k;
                    // SGI configuration is fixed edge-triggered.
                    if (16..NUM_IRQ).contains(&i) {
                        self.edge.set(i, v & (2 << (2 * k)) != 0);
                    }
                }
            }
            0xF10..=0xF1F if v & 1 != 0 => self.pending_latch.set(((off - 0xF10) % 16) as usize, false),
            0xF20..=0xF2F if v & 1 != 0 => self.pending_latch.set(((off - 0xF20) % 16) as usize, true),
            _ => {}
        }
    }

    /// GICD_SGIR: with one processor, every filter that includes it targets
    /// it ("all others" targets nobody).
    fn write_sgir(&mut self, value: u32) {
        let sgi = (value & 0xF) as usize;
        let to_self = match (value >> 24) & 3 {
            0 => (value >> 16) & 1 != 0,
            2 => true,
            _ => false,
        };
        if to_self {
            self.pending_latch.set(sgi, true);
        }
    }

    // ----- CPU interface -----

    /// Acknowledge: the IAR value for the best pending interrupt.
    fn acknowledge(&mut self, group1_alias: bool) -> u32 {
        let Some(i) = self.best_pending() else {
            return SPURIOUS;
        };
        let g1 = self.group1.get(i);
        if g1 != group1_alias && !(g1 && self.cpu_ctlr & (1 << 2) != 0) {
            return if g1 { SPURIOUS_GROUP1 } else { SPURIOUS };
        }
        self.pending_latch.set(i, false);
        self.active.set(i, true);
        if self.stack_len < self.stack.len() {
            self.stack[self.stack_len] = i as u16;
            self.stack_len += 1;
        }
        self.update();
        i as u32
    }

    /// End of interrupt: drop the running priority, and deactivate unless
    /// EOImode separates the two.
    fn end_of_interrupt(&mut self, value: u32) {
        let id = (value & 0x3FF) as u16;
        if let Some(pos) = self.stack[..self.stack_len].iter().rposition(|&x| x == id) {
            self.stack.copy_within(pos + 1..self.stack_len, pos);
            self.stack_len -= 1;
        }
        if self.cpu_ctlr & (1 << 9) == 0 && (id as usize) < NUM_IRQ {
            self.active.set(id as usize, false);
        }
        self.update();
    }

    fn highest_pending_id(&self) -> u32 {
        match self.best_pending() {
            Some(i) => i as u32,
            None => SPURIOUS,
        }
    }

    /// GICC_APR0..3: one bit per active group priority (priority >> 1, the
    /// group priority at the minimum binary point).
    fn active_priorities(&self, word: usize) -> u32 {
        let mut v = 0u32;
        for &id in &self.stack[..self.stack_len] {
            let level = (self.priority[id as usize] >> 1) as usize;
            if level / 32 == word {
                v |= 1 << (level % 32);
            }
        }
        v
    }

    pub fn cpu_read(&mut self, offset: RegOffset, _width: AccessWidth) -> u64 {
        u64::from(match offset.get() {
            0x00 => self.cpu_ctlr,
            0x04 => u32::from(self.pmr),
            0x08 => u32::from(self.bpr),
            0x0C => self.acknowledge(false),
            0x14 => u32::from(self.running_priority()),
            0x18 => self.highest_pending_id(),
            0x1C => u32::from(self.abpr),
            0x20 => self.acknowledge(true),
            0x28 => self.highest_pending_id(),
            0xD0..=0xDC => self.active_priorities(((offset.get() - 0xD0) / 4) as usize),
            0xFC => 0x0002_043B,
            _ => 0,
        })
    }

    pub fn cpu_write(&mut self, offset: RegOffset, _width: AccessWidth, value: u64) {
        let v = value as u32;
        match offset.get() {
            0x00 => self.cpu_ctlr = v & 0x3FF,
            0x04 => self.pmr = v as u8,
            0x08 => self.bpr = (v & 7) as u8,
            0x10 | 0x24 => self.end_of_interrupt(v),
            0x1C => self.abpr = ((v & 7) as u8).max(1),
            0x1000 => {
                let id = (v & 0x3FF) as usize;
                if id < NUM_IRQ {
                    self.active.set(id, false);
                }
            }
            _ => {}
        }
        self.update();
    }

    // ----- GICv2m MSI frame -----

    pub fn v2m_read(&mut self, offset: RegOffset, _width: AccessWidth) -> u64 {
        match offset.get() {
            0x008 => u64::from((V2M_BASE_SPI << 16) | V2M_NUM_SPI),
            0xFCC => 0x51 << 20,
            _ => 0,
        }
    }

    pub fn v2m_write(&mut self, offset: RegOffset, _width: AccessWidth, value: u64) {
        if offset.get() == 0x040 {
            let spi = (value as u32) & 0x3FF;
            if (V2M_BASE_SPI..V2M_BASE_SPI + V2M_NUM_SPI).contains(&spi) {
                self.pulse(spi);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(g: &mut GicV2, dist: bool, off: u64, v: u64) {
        if dist {
            g.dist_write(RegOffset::new(off), AccessWidth::Word, v);
        } else {
            g.cpu_write(RegOffset::new(off), AccessWidth::Word, v);
        }
    }

    /// The timer path EDK2 depends on: a level PPI raises IRQ once enabled,
    /// IAR acknowledges it, and after EOI it is signalled again only while
    /// the line is still high.
    #[test]
    fn a_level_ppi_is_acknowledged_and_completed() {
        let mut g = GicV2::new();
        w(&mut g, true, 0x000, 1);
        w(&mut g, false, 0x000, 1);
        w(&mut g, false, 0x004, 0xFF);
        w(&mut g, true, 0x100, 1 << 30);
        g.set_line(30, true);
        assert!(g.irq_out());
        assert_eq!(g.cpu_read(RegOffset::new(0x0C), AccessWidth::Word), 30);
        assert!(!g.irq_out(), "active, so not signalled again");
        g.set_line(30, false);
        w(&mut g, false, 0x010, 30);
        assert!(!g.irq_out());
        assert_eq!(g.cpu_read(RegOffset::new(0x0C), AccessWidth::Word), 1023);
    }

    /// The priority mask hides an interrupt whose priority is not higher.
    #[test]
    fn pmr_masks_lower_priority_interrupts() {
        let mut g = GicV2::new();
        w(&mut g, true, 0x000, 1);
        w(&mut g, false, 0x000, 1);
        w(&mut g, true, 0x400 + 32, 0x80);
        w(&mut g, true, 0x104, 1);
        w(&mut g, false, 0x004, 0x80);
        g.set_line(32, true);
        assert!(!g.irq_out());
        w(&mut g, false, 0x004, 0x81);
        assert!(g.irq_out());
    }
}
