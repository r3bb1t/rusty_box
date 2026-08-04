//! High Precision Event Timer.
//!
//! Port of Bochs iodev/hpet.cc / hpet.h (itself derived from QEMU). The
//! guest-visible model is byte-for-byte Bochs: a 3-timer HPET at physical
//! 0xFED00000 with a 10 ns main counter, legacy-replacement routing, level
//! and edge interrupt modes, FSB (MSI-style) delivery, and the 0x8086
//! vendor id the rombios32 ACPI builder probes for.
//!
//! Structural difference from Bochs (documented, behavior-preserving):
//! Bochs mutates `bx_pc_system` and the PIC synchronously from inside the
//! MMIO handlers. rusty_box memory handlers run inside a CPU batch where
//! neither is borrowable, so every side effect is queued in [`HpetPending`]
//! with deadlines pre-anchored to the access instant (`now_ticks`), and the
//! emulator drains the queue at the scheduler boundary the access requests.
//! Fired timers are serviced in emulator context and drain immediately, so
//! all guest-visible times and IRQ edges match Bochs exactly.

use crate::config::BxPhyAddress;
#[cfg(feature = "std")]
use crate::snapshot::{checked_snapshot_len_add, SnapshotReader, SnapshotWriteExt};
#[cfg(feature = "std")]
use std::io::{self, Read, Write};

/// Bochs hpet.cc `HPET_BASE`.
pub(crate) const HPET_BASE: BxPhyAddress = 0xFED0_0000;
/// Bochs hpet.cc `HPET_LEN`.
pub(crate) const HPET_LEN: BxPhyAddress = 0x400;
/// Bochs hpet.h `HPET_MIN_TIMERS` — the number of timers Bochs instantiates.
pub(crate) const HPET_NUM_TIMERS: usize = 3;

/// Bochs hpet.cc `HPET_CLK_PERIOD` (nanoseconds per HPET tick).
const HPET_CLK_PERIOD: u64 = 10;
/// Bochs hpet.cc `FS_PER_NS`.
const FS_PER_NS: u64 = 1_000_000;
/// Bochs hpet.cc `HPET_ROUTING_CAP`.
const HPET_ROUTING_CAP: u64 = 0xffffff;
/// Bochs hpet.cc `RTC_ISA_IRQ`.
const RTC_ISA_IRQ: u8 = 8;
/// Bochs hpet.cc clamp bounds for computed fire deltas (HPET ticks).
const HPET_MAX_ALLOWED_PERIOD: u64 = 0x0400_0000_0000_0000;
const HPET_MIN_ALLOWED_PERIOD: u64 = 1;

// Bochs hpet.h register offsets.
const HPET_ID: u16 = 0x000;
const HPET_PERIOD: u16 = 0x004;
const HPET_CFG: u16 = 0x010;
const HPET_STATUS: u16 = 0x020;
const HPET_COUNTER: u16 = 0x0F0;
const HPET_TN_CFG: u16 = 0x000;
const HPET_TN_CMP: u16 = 0x008;
const HPET_TN_ROUTE: u16 = 0x010;
const HPET_ID_HI: u16 = HPET_ID + 4;
const HPET_CFG_HI: u16 = HPET_CFG + 4;
const HPET_STATUS_HI: u16 = HPET_STATUS + 4;
const HPET_COUNTER_HI: u16 = HPET_COUNTER + 4;
const HPET_TN_CFG_HI: u16 = HPET_TN_CFG + 4;
const HPET_TN_CMP_HI: u16 = HPET_TN_CMP + 4;
const HPET_TN_ROUTE_HI: u16 = HPET_TN_ROUTE + 4;

// Bochs hpet.h configuration bits.
const HPET_CFG_ENABLE: u64 = 0x001;
const HPET_CFG_LEGACY: u64 = 0x002;
const HPET_CFG_WRITE_MASK: u64 = 0x3;
const HPET_TN_TYPE_LEVEL: u64 = 0x002;
const HPET_TN_ENABLE: u64 = 0x004;
const HPET_TN_PERIODIC: u64 = 0x008;
const HPET_TN_PERIODIC_CAP: u64 = 0x010;
const HPET_TN_SIZE_CAP: u64 = 0x020;
const HPET_TN_SETVAL: u64 = 0x040;
const HPET_TN_32BIT: u64 = 0x100;
const HPET_TN_INT_ROUTE_MASK: u64 = 0x3e00;
const HPET_TN_FSB_ENABLE: u64 = 0x4000;
const HPET_TN_CFG_WRITE_MASK: u64 = 0x7f4e;
const HPET_TN_INT_ROUTE_SHIFT: u64 = 9;

/// Queue capacities for side effects accumulated between drains. A single
/// register write produces at most a handful of entries; the bound covers a
/// full batch of back-to-back MMIO writes. Overflow is logged and dropped —
/// see `HpetPending::push_irq`.
const PENDING_IRQ_CAPACITY: usize = 32;
const PENDING_FSB_CAPACITY: usize = 8;

/// Bochs hpet.h `HPETTimer`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HpetTimer {
    pub(crate) config: u64,
    pub(crate) cmp: u64,
    pub(crate) fsb: u64,
    pub(crate) period: u64,
    pub(crate) last_checked: u64,
}

impl HpetTimer {
    const fn new() -> Self {
        Self {
            config: 0,
            cmp: 0,
            fsb: 0,
            period: 0,
            last_checked: 0,
        }
    }
}

/// One deferred pc-system timer operation for an HPET comparator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HpetTimerOp {
    /// Activate one-shot at this absolute pc-system tick deadline
    /// (pre-anchored at the MMIO access instant — Bochs
    /// `activate_timer_nsec` called synchronously).
    ArmAtTicks(u64),
    /// Bochs `deactivate_timer`.
    Deactivate,
}

/// Side effects queued by MMIO handlers for the emulator to apply.
#[derive(Debug)]
pub(crate) struct HpetPending {
    pub(crate) irq_ops: [(u8, bool); PENDING_IRQ_CAPACITY],
    pub(crate) irq_op_count: usize,
    pub(crate) fsb_writes: [(BxPhyAddress, u32); PENDING_FSB_CAPACITY],
    pub(crate) fsb_write_count: usize,
    pub(crate) timer_ops: [Option<HpetTimerOp>; HPET_NUM_TIMERS],
    /// Bochs `DEV_pit_enable_irq` / `DEV_cmos_enable_irq` calls (last wins).
    pub(crate) pit_irq_gate: Option<bool>,
    pub(crate) cmos_irq_gate: Option<bool>,
}

impl HpetPending {
    const fn new() -> Self {
        Self {
            irq_ops: [(0, false); PENDING_IRQ_CAPACITY],
            irq_op_count: 0,
            fsb_writes: [(0, 0); PENDING_FSB_CAPACITY],
            fsb_write_count: 0,
            timer_ops: [None; HPET_NUM_TIMERS],
            pit_irq_gate: None,
            cmos_irq_gate: None,
        }
    }

    fn is_empty(&self) -> bool {
        self.irq_op_count == 0
            && self.fsb_write_count == 0
            && self.timer_ops.iter().all(Option::is_none)
            && self.pit_irq_gate.is_none()
            && self.cmos_irq_gate.is_none()
    }
}

/// Bochs `bx_hpet_c`.
#[derive(Debug)]
pub struct BxHpetC {
    pub(crate) capability: u64,
    pub(crate) config: u64,
    pub(crate) isr: u64,
    pub(crate) hpet_counter: u64,
    pub(crate) hpet_reference_value: u64,
    /// Reference epoch in NANOSECONDS (Bochs `hpet_reference_time`).
    pub(crate) hpet_reference_time: u64,
    pub(crate) timers: [HpetTimer; HPET_NUM_TIMERS],
    /// PC-system one-shot handles, one per comparator (Bochs `timer_id`).
    pub(crate) timer_handles: [Option<usize>; HPET_NUM_TIMERS],
    /// Emulated-tick cursor of the current access — stamped by the CPU MMIO
    /// slow path (`system_ticks()`) or the emulator before servicing a fire.
    /// Bochs reads `bx_pc_system.time_nsec()` directly; this cursor carries
    /// the identical clock into handler context.
    now_ticks: u64,
    /// Tick rate for the nsec conversions (pc_system `ips`).
    ips: u64,
    pending: HpetPending,
}

impl Default for BxHpetC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxHpetC {
    pub fn new() -> Self {
        // Bochs hpet.cc init(): 3 timers, vendor 0x8086, revision 1,
        // 64-bit counter, legacy-replacement capable, 10 ns period.
        let mut capability: u64 = 0x8086_a001 | (((HPET_NUM_TIMERS as u64) - 1) << 8);
        capability |= (HPET_CLK_PERIOD * FS_PER_NS) << 32;
        Self {
            capability,
            config: 0,
            isr: 0,
            hpet_counter: 0,
            hpet_reference_value: 0,
            hpet_reference_time: 0,
            timers: [HpetTimer::new(); HPET_NUM_TIMERS],
            timer_handles: [None; HPET_NUM_TIMERS],
            now_ticks: 0,
            ips: 0,
            pending: HpetPending::new(),
        }
    }

    /// Bochs hpet.cc reset(): every comparator is stopped and re-armed to
    /// capability defaults, the main counter clears, and the PIT/RTC output
    /// pins are re-enabled.
    pub fn reset(&mut self) {
        for index in 0..HPET_NUM_TIMERS {
            self.hpet_del_timer(index);
            let timer = &mut self.timers[index];
            timer.cmp = u64::MAX;
            timer.period = u64::MAX;
            timer.config = HPET_TN_PERIODIC_CAP | HPET_TN_SIZE_CAP | (HPET_ROUTING_CAP << 32);
            timer.last_checked = 0;
        }
        self.hpet_counter = 0;
        self.hpet_reference_value = 0;
        self.hpet_reference_time = 0;
        self.config = 0;
        self.isr = 0;
        // Bochs: DEV_pit_enable_irq(1); DEV_cmos_enable_irq(1);
        self.pending.pit_irq_gate = Some(true);
        self.pending.cmos_irq_gate = Some(true);
    }

    /// Stamp the emulated-time cursor for the next handler call.
    pub(crate) fn set_now(&mut self, now_ticks: u64, ips: u64) {
        self.now_ticks = now_ticks;
        self.ips = ips;
    }

    /// Whether queued side effects await an emulator drain.
    pub(crate) fn has_pending_work(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Hand the queued side effects to the emulator and clear the queue.
    pub(crate) fn take_pending(&mut self) -> HpetPending {
        core::mem::replace(&mut self.pending, HpetPending::new())
    }

    // ── Bochs hpet.cc static helpers ─────────────────────────────────────

    /// Bochs `hpet_time_between` — start not later than end; wrap-aware.
    fn time_between(start: u64, end: u64, value: u64) -> bool {
        if start <= end {
            (start <= value) && (value <= end)
        } else {
            (start <= value) || (value <= end)
        }
    }

    /// Bochs `hpet_cmp32_to_cmp64`: earliest 64-bit tick after `reference`
    /// with the same low 32 bits as `value`.
    fn cmp32_to_cmp64(reference: u64, value: u32) -> u64 {
        if (reference as u32) <= value {
            (reference & 0xFFFF_FFFF_0000_0000) | u64::from(value)
        } else {
            (reference.wrapping_add(0x1_0000_0000) & 0xFFFF_FFFF_0000_0000) | u64::from(value)
        }
    }

    fn ticks_to_ns(value: u64) -> u64 {
        value.wrapping_mul(HPET_CLK_PERIOD)
    }

    fn ns_to_ticks(value: u64) -> u64 {
        value / HPET_CLK_PERIOD
    }

    /// Bochs `hpet_fixup_reg`.
    fn fixup_reg(new: u64, old: u64, mask: u64) -> u64 {
        (new & mask) | (old & !mask)
    }

    fn activating_bit(old: u64, new: u64, mask: u64) -> bool {
        (old & mask) == 0 && (new & mask) != 0
    }

    fn deactivating_bit(old: u64, new: u64, mask: u64) -> bool {
        (old & mask) != 0 && (new & mask) == 0
    }

    // ── Clock plumbing ───────────────────────────────────────────────────

    /// Emulated nanoseconds at the stamped cursor — the value Bochs reads
    /// from `bx_pc_system.time_nsec()` (pc_system.cc conversion).
    fn time_nsec(&self) -> u64 {
        if self.ips == 0 {
            return 0;
        }
        let nsec = (u128::from(self.now_ticks) * 1_000_000_000u128) / u128::from(self.ips);
        u64::try_from(nsec).unwrap_or(u64::MAX)
    }

    /// Emulated pc-system ticks for a nanosecond delta. Bochs pc_system.cc
    /// `activate_timer_nsec` computes `(Bit64u)(double(nsec) * m_ips / 1000.0)`
    /// — a truncating (floor) conversion. Matched with integer floor division:
    /// a floored deadline may fire before the comparator crossing, and
    /// `timer_fired` finds `time_between` false and re-arms, exactly like Bochs
    /// (the IRQ lands on the same tick either way).
    fn nsec_to_pc_ticks(&self, nsec: u64) -> u64 {
        let ticks = (u128::from(nsec) * u128::from(self.ips)) / 1_000_000_000u128;
        u64::try_from(ticks).unwrap_or(u64::MAX)
    }

    /// Bochs `hpet_get_ticks`.
    fn hpet_get_ticks(&self) -> u64 {
        Self::ns_to_ticks(self.time_nsec().wrapping_sub(self.hpet_reference_time))
            .wrapping_add(self.hpet_reference_value)
    }

    /// Bochs `hpet_calculate_diff` — distance from `current` to the
    /// comparator in HPET ticks, with 32-bit wrap semantics when configured.
    fn calculate_diff(timer: &HpetTimer, current: u64) -> u64 {
        if timer.config & HPET_TN_32BIT != 0 {
            u64::from((timer.cmp as u32).wrapping_sub(current as u32))
        } else {
            timer.cmp.wrapping_sub(current)
        }
    }

    fn in_legacy_mode(&self) -> bool {
        self.config & HPET_CFG_LEGACY != 0
    }

    fn enabled(&self) -> bool {
        self.config & HPET_CFG_ENABLE != 0
    }

    fn timer_int_route(timer: &HpetTimer) -> u8 {
        ((timer.config & HPET_TN_INT_ROUTE_MASK) >> HPET_TN_INT_ROUTE_SHIFT) as u8
    }

    fn timer_fsb_route(timer: &HpetTimer) -> bool {
        timer.config & HPET_TN_FSB_ENABLE != 0
    }

    fn timer_is_periodic(timer: &HpetTimer) -> bool {
        timer.config & HPET_TN_PERIODIC != 0
    }

    fn timer_enabled(timer: &HpetTimer) -> bool {
        timer.config & HPET_TN_ENABLE != 0
    }

    // ── Queued side effects ──────────────────────────────────────────────

    fn push_irq(&mut self, route: u8, level: bool) {
        if self.pending.irq_op_count >= PENDING_IRQ_CAPACITY {
            tracing::error!("HPET: pending IRQ queue overflow, dropping edge");
            return;
        }
        self.pending.irq_ops[self.pending.irq_op_count] = (route, level);
        self.pending.irq_op_count += 1;
    }

    fn push_fsb(&mut self, address: BxPhyAddress, value: u32) {
        if self.pending.fsb_write_count >= PENDING_FSB_CAPACITY {
            tracing::error!("HPET: pending FSB queue overflow, dropping message");
            return;
        }
        self.pending.fsb_writes[self.pending.fsb_write_count] = (address, value);
        self.pending.fsb_write_count += 1;
    }

    /// Bochs `update_irq`.
    fn update_irq(&mut self, index: usize, set: bool) {
        let timer = self.timers[index];
        let route = if index <= 1 && self.in_legacy_mode() {
            // Bochs hpet.cc: legacy replacement routes timer 0 to IRQ0/pin2
            // and timer 1 to the RTC IRQ.
            if index == 0 { 0 } else { RTC_ISA_IRQ }
        } else {
            Self::timer_int_route(&timer)
        };
        let mask = 1u64 << index;
        if !set || !self.enabled() {
            self.push_irq(route, false);
        } else {
            if timer.config & HPET_TN_TYPE_LEVEL != 0 {
                // Bochs: level timers latch status even with interrupts
                // disabled at the comparator.
                self.isr |= mask;
            }
            if Self::timer_enabled(&timer) {
                if Self::timer_fsb_route(&timer) {
                    self.push_fsb((timer.fsb >> 32) as BxPhyAddress, timer.fsb as u32);
                } else if timer.config & HPET_TN_TYPE_LEVEL != 0 {
                    self.push_irq(route, true);
                } else {
                    self.push_irq(route, false);
                    self.push_irq(route, true);
                }
            }
        }
    }

    /// Bochs `hpet_set_timer` — compute the next fire distance and queue the
    /// one-shot activation, pre-anchored at the stamped access instant.
    fn hpet_set_timer(&mut self, index: usize) {
        let cur_tick = self.hpet_get_ticks();
        let timer = self.timers[index];
        let mut diff = Self::calculate_diff(&timer, cur_tick);
        if diff == 0 {
            diff = if timer.config & HPET_TN_32BIT != 0 {
                0x1_0000_0000
            } else {
                HPET_MAX_ALLOWED_PERIOD
            };
        }
        // Bochs: one-shot 32-bit mode also interrupts on counter wrap.
        if !Self::timer_is_periodic(&timer) && (timer.config & HPET_TN_32BIT != 0) {
            let wrap_diff = 0x1_0000_0000u64 - u64::from(cur_tick as u32);
            diff = diff.min(wrap_diff);
        }
        diff = diff.clamp(HPET_MIN_ALLOWED_PERIOD, HPET_MAX_ALLOWED_PERIOD);
        let deadline =
            self.now_ticks.saturating_add(self.nsec_to_pc_ticks(Self::ticks_to_ns(diff)));
        self.pending.timer_ops[index] = Some(HpetTimerOp::ArmAtTicks(deadline));
    }

    /// Bochs `hpet_del_timer`.
    fn hpet_del_timer(&mut self, index: usize) {
        self.pending.timer_ops[index] = Some(HpetTimerOp::Deactivate);
        self.update_irq(index, false);
    }

    /// Bochs `hpet_timer` — the pc-system one-shot for comparator `index`
    /// fired. The caller must stamp `set_now` first and drain afterwards.
    pub(crate) fn timer_fired(&mut self, index: usize) {
        let cur_time = self.time_nsec();
        let cur_tick = self.hpet_get_ticks();
        let timer = self.timers[index];

        if Self::timer_is_periodic(&timer) {
            if timer.config & HPET_TN_32BIT != 0 {
                let mut cmp64 = Self::cmp32_to_cmp64(timer.last_checked, timer.cmp as u32);
                if Self::time_between(timer.last_checked, cur_tick, cmp64) {
                    self.update_irq(index, true);
                    let period32 = self.timers[index].period as u32;
                    if period32 != 0 {
                        loop {
                            cmp64 = cmp64.wrapping_add(u64::from(period32));
                            if !Self::time_between(
                                self.timers[index].last_checked,
                                cur_tick,
                                cmp64,
                            ) {
                                break;
                            }
                        }
                        self.timers[index].cmp = u64::from(cmp64 as u32);
                    }
                }
            } else if Self::time_between(timer.last_checked, cur_tick, timer.cmp) {
                self.update_irq(index, true);
                if self.timers[index].period != 0 {
                    loop {
                        self.timers[index].cmp = self.timers[index]
                            .cmp
                            .wrapping_add(self.timers[index].period);
                        if !Self::time_between(
                            self.timers[index].last_checked,
                            cur_tick,
                            self.timers[index].cmp,
                        ) {
                            break;
                        }
                    }
                }
            }
        } else if timer.config & HPET_TN_32BIT != 0 {
            let cmp64 = Self::cmp32_to_cmp64(timer.last_checked, timer.cmp as u32);
            let wrap = Self::cmp32_to_cmp64(timer.last_checked, 0);
            if Self::time_between(timer.last_checked, cur_tick, cmp64)
                || Self::time_between(timer.last_checked, cur_tick, wrap)
            {
                self.update_irq(index, true);
            }
        } else if Self::time_between(timer.last_checked, cur_tick, timer.cmp) {
            self.update_irq(index, true);
        }
        self.hpet_set_timer(index);
        self.timers[index].last_checked = cur_tick;

        // Bochs hpet.cc: fold whole elapsed HPET ticks into the reference
        // pair so the ns remainder keeps accumulating precisely.
        let ticks_passed = Self::ns_to_ticks(cur_time.wrapping_sub(self.hpet_reference_time));
        if ticks_passed != 0 {
            self.hpet_reference_time = self
                .hpet_reference_time
                .wrapping_add(Self::ticks_to_ns(ticks_passed));
            self.hpet_reference_value = self.hpet_reference_value.wrapping_add(ticks_passed);
        }
    }

    // ── MMIO ─────────────────────────────────────────────────────────────

    /// Memory-system dispatch entry (Bochs static `hpet_read`).
    pub(crate) fn mem_read(&mut self, addr: BxPhyAddress, len: u32, data: &mut [u8]) {
        match len {
            4 if addr & 0x3 == 0 => {
                let value = self.read_aligned(addr);
                data[..4].copy_from_slice(&value.to_le_bytes());
            }
            8 if addr & 0x7 == 0 => {
                let low = self.read_aligned(addr);
                let high = self.read_aligned(addr + 4);
                let value = u64::from(low) | (u64::from(high) << 32);
                data[..8].copy_from_slice(&value.to_le_bytes());
            }
            2 => {
                tracing::error!("HPET: unsupported read at {:#x} with len=2", addr);
                data[..2].fill(0);
            }
            1 => {
                tracing::error!("HPET: unsupported read at {:#x} with len=1", addr);
                data[0] = 0;
            }
            _ => {
                tracing::error!("HPET: unaligned read at {:#x} len={}", addr, len);
                data[..len as usize].fill(0);
            }
        }
    }

    /// Memory-system dispatch entry (Bochs static `hpet_write`).
    pub(crate) fn mem_write(&mut self, addr: BxPhyAddress, len: u32, data: &[u8]) {
        match len {
            4 if addr & 0x3 == 0 => {
                let value = u32::from_le_bytes(data[..4].try_into().expect("len checked"));
                self.write_aligned(addr, value, true);
            }
            8 if addr & 0x7 == 0 => {
                let value = u64::from_le_bytes(data[..8].try_into().expect("len checked"));
                self.write_aligned(addr, value as u32, false);
                self.write_aligned(addr + 4, (value >> 32) as u32, true);
            }
            _ => {
                tracing::error!("HPET: unsupported write at {:#x} len={}", addr, len);
            }
        }
    }

    /// Bochs `read_aligned`.
    pub(crate) fn read_aligned(&self, address: BxPhyAddress) -> u32 {
        let index = (address & 0x3ff) as u16;
        if index < 0x100 {
            match index {
                HPET_ID => self.capability as u32,
                HPET_PERIOD => (self.capability >> 32) as u32,
                HPET_CFG => self.config as u32,
                HPET_CFG_HI => (self.config >> 32) as u32,
                HPET_STATUS => self.isr as u32,
                HPET_STATUS_HI => (self.isr >> 32) as u32,
                HPET_COUNTER => {
                    if self.enabled() {
                        self.hpet_get_ticks() as u32
                    } else {
                        self.hpet_counter as u32
                    }
                }
                HPET_COUNTER_HI => {
                    if self.enabled() {
                        (self.hpet_get_ticks() >> 32) as u32
                    } else {
                        (self.hpet_counter >> 32) as u32
                    }
                }
                _ => {
                    tracing::error!("HPET: read from reserved offset {:#06x}", index);
                    0
                }
            }
        } else {
            let id = ((index - 0x100) / 0x20) as usize;
            if id >= HPET_NUM_TIMERS {
                tracing::error!("HPET: read: timer id out of range");
                return 0;
            }
            let timer = &self.timers[id];
            match index & 0x1f {
                HPET_TN_CFG => timer.config as u32,
                HPET_TN_CFG_HI => (timer.config >> 32) as u32,
                HPET_TN_CMP => timer.cmp as u32,
                HPET_TN_CMP_HI => (timer.cmp >> 32) as u32,
                HPET_TN_ROUTE => timer.fsb as u32,
                HPET_TN_ROUTE_HI => (timer.fsb >> 32) as u32,
                _ => {
                    tracing::error!("HPET: read from reserved offset {:#06x}", index);
                    0
                }
            }
        }
    }

    /// Bochs `write_aligned`.
    pub(crate) fn write_aligned(&mut self, address: BxPhyAddress, value: u32, trailing_write: bool) {
        let index = (address & 0x3ff) as u16;
        let new_val = u64::from(value);
        let old_val = u64::from(self.read_aligned(address));

        if index < 0x100 {
            match index {
                HPET_ID | HPET_ID_HI => {}
                HPET_CFG => {
                    let val = Self::fixup_reg(new_val, old_val, HPET_CFG_WRITE_MASK);
                    self.config = (self.config & 0xffff_ffff_0000_0000) | val;
                    if Self::activating_bit(old_val, new_val, HPET_CFG_ENABLE) {
                        // Enable main counter and interrupt generation.
                        self.hpet_reference_value = self.hpet_counter;
                        self.hpet_reference_time = self.time_nsec();
                        for i in 0..HPET_NUM_TIMERS {
                            if Self::timer_enabled(&self.timers[i]) && (self.isr & (1 << i)) != 0
                            {
                                self.update_irq(i, true);
                            }
                            self.hpet_set_timer(i);
                        }
                    } else if Self::deactivating_bit(old_val, new_val, HPET_CFG_ENABLE) {
                        // Halt main counter and disable interrupt generation.
                        self.hpet_counter = self.hpet_get_ticks();
                        for i in 0..HPET_NUM_TIMERS {
                            self.hpet_del_timer(i);
                        }
                    }
                    // Bochs: i8254 and RTC output pins are disabled in
                    // legacy-replacement mode.
                    if Self::activating_bit(old_val, new_val, HPET_CFG_LEGACY) {
                        tracing::info!("HPET: entering legacy mode");
                        self.pending.pit_irq_gate = Some(false);
                        self.pending.cmos_irq_gate = Some(false);
                    } else if Self::deactivating_bit(old_val, new_val, HPET_CFG_LEGACY) {
                        tracing::info!("HPET: leaving legacy mode");
                        self.pending.pit_irq_gate = Some(true);
                        self.pending.cmos_irq_gate = Some(true);
                    }
                }
                HPET_CFG_HI => {}
                HPET_STATUS => {
                    let val = new_val & self.isr;
                    for i in 0..HPET_NUM_TIMERS {
                        if val & (1 << i) != 0 {
                            self.update_irq(i, false);
                            self.isr &= !(1u64 << i);
                        }
                    }
                }
                HPET_STATUS_HI => {}
                HPET_COUNTER => {
                    if self.enabled() {
                        tracing::error!("HPET: writing counter while enabled!");
                    } else {
                        self.hpet_counter =
                            (self.hpet_counter & 0xffff_ffff_0000_0000) | new_val;
                        for timer in &mut self.timers {
                            timer.last_checked = self.hpet_counter;
                        }
                    }
                }
                HPET_COUNTER_HI => {
                    if self.enabled() {
                        tracing::error!("HPET: writing counter while enabled!");
                    } else {
                        self.hpet_counter =
                            (self.hpet_counter & 0xffff_ffff) | (new_val << 32);
                        for timer in &mut self.timers {
                            timer.last_checked = self.hpet_counter;
                        }
                    }
                }
                _ => {
                    tracing::error!("HPET: write to reserved offset {:#06x}", index);
                }
            }
        } else {
            let id = ((index - 0x100) / 0x20) as usize;
            if id >= HPET_NUM_TIMERS {
                tracing::error!("HPET: write: timer id out of range");
                return;
            }
            match index & 0x1f {
                HPET_TN_CFG => {
                    let val = Self::fixup_reg(new_val, old_val, HPET_TN_CFG_WRITE_MASK);
                    let timer = &mut self.timers[id];
                    timer.config = (timer.config & 0xffff_ffff_0000_0000) | val;
                    if timer.config & HPET_TN_32BIT != 0 {
                        timer.cmp = u64::from(timer.cmp as u32);
                        timer.period = u64::from(timer.period as u32);
                    }
                    if Self::timer_fsb_route(timer) || (timer.config & HPET_TN_TYPE_LEVEL) == 0 {
                        self.isr &= !(1u64 << id);
                    }
                    if Self::timer_enabled(&self.timers[id]) && self.enabled() {
                        let latched = self.isr & (1 << id) != 0;
                        self.update_irq(id, latched);
                    }
                    if self.enabled() {
                        self.hpet_set_timer(id);
                    }
                }
                HPET_TN_CFG_HI => {}
                HPET_TN_CMP => {
                    let timer = &mut self.timers[id];
                    if !Self::timer_is_periodic(timer) || (timer.config & HPET_TN_SETVAL) != 0 {
                        timer.cmp = (timer.cmp & 0xffff_ffff_0000_0000) | new_val;
                    }
                    timer.period = (timer.period & 0xffff_ffff_0000_0000) | new_val;
                    if trailing_write {
                        timer.config &= !HPET_TN_SETVAL;
                    }
                    if self.enabled() {
                        self.hpet_set_timer(id);
                    }
                }
                HPET_TN_CMP_HI => {
                    let timer = &mut self.timers[id];
                    if timer.config & HPET_TN_32BIT != 0 {
                        return;
                    }
                    if !Self::timer_is_periodic(timer) || (timer.config & HPET_TN_SETVAL) != 0 {
                        timer.cmp = (timer.cmp & 0xffff_ffff) | (new_val << 32);
                    }
                    timer.period = (timer.period & 0xffff_ffff) | (new_val << 32);
                    if trailing_write {
                        timer.config &= !HPET_TN_SETVAL;
                    }
                    if self.enabled() {
                        self.hpet_set_timer(id);
                    }
                }
                HPET_TN_ROUTE => {
                    let timer = &mut self.timers[id];
                    timer.fsb = (timer.fsb & 0xffff_ffff_0000_0000) | new_val;
                }
                HPET_TN_ROUTE_HI => {
                    let timer = &mut self.timers[id];
                    timer.fsb = (new_val << 32) | (timer.fsb & 0xffff_ffff);
                }
                _ => {
                    tracing::error!("HPET: write to reserved offset {:#06x}", index);
                }
            }
        }
    }
}

// ── Snapshot ─────────────────────────────────────────────────────────────

#[cfg(feature = "std")]
const HPET_SNAPSHOT_VERSION: u32 = 1;

#[cfg(feature = "std")]
impl BxHpetC {
    /// Byte length of the HPET snapshot section (fixed layout).
    pub(crate) fn snapshot_v3_len(&self) -> io::Result<u64> {
        // Bochs hpet.cc register_state(): config, isr, hpet_counter, plus
        // per-timer {config, cmp, fsb, period}. version u32 + 3 × u64 +
        // HPET_NUM_TIMERS × 4 × u64.
        let mut len = checked_snapshot_len_add(0, 4)?;
        len = checked_snapshot_len_add(len, 3 * 8)?;
        len = checked_snapshot_len_add(len, (HPET_NUM_TIMERS as u64) * 4 * 8)?;
        Ok(len)
    }

    /// Stream the exact field set Bochs `register_state()` serializes:
    /// config, isr, hpet_counter, and per-timer {config, cmp, fsb, period}.
    /// The derived reference/last_checked fields are deliberately NOT saved —
    /// Bochs omits them, so a restore reconstructs the counter from a zero
    /// reference exactly as Bochs does. The comparator pc-system timers are
    /// re-registered by `register_timer_owners`, so their handles are not part
    /// of the format; the pending queue is always empty at a snapshot boundary.
    pub(crate) fn save_snapshot_v3<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        debug_assert!(
            !self.has_pending_work(),
            "HPET snapshot taken with side effects still queued"
        );
        writer.write_u32(HPET_SNAPSHOT_VERSION)?;
        writer.write_u64(self.config)?;
        writer.write_u64(self.isr)?;
        writer.write_u64(self.hpet_counter)?;
        for timer in &self.timers {
            writer.write_u64(timer.config)?;
            writer.write_u64(timer.cmp)?;
            writer.write_u64(timer.fsb)?;
            writer.write_u64(timer.period)?;
        }
        Ok(())
    }

    /// Restore the Bochs `register_state()` field set. The pc-system timer
    /// table is restored separately, so this neither re-arms comparators nor
    /// emits IRQ edges. Bochs restores onto a freshly constructed device, so
    /// the omitted reference/last_checked fields start at zero; rusty_box
    /// restores in place, so it zeroes them explicitly to reproduce Bochs's
    /// counter-restore behavior exactly.
    pub(crate) fn restore_snapshot_v3<R: Read>(
        &mut self,
        reader: &mut SnapshotReader<R>,
    ) -> io::Result<()> {
        if reader.read_u32()? != HPET_SNAPSHOT_VERSION {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unsupported HPET snapshot section version",
            ));
        }
        self.config = reader.read_u64()?;
        self.isr = reader.read_u64()?;
        self.hpet_counter = reader.read_u64()?;
        for timer in &mut self.timers {
            timer.config = reader.read_u64()?;
            timer.cmp = reader.read_u64()?;
            timer.fsb = reader.read_u64()?;
            timer.period = reader.read_u64()?;
            // Bochs fresh-construct default (memset): the reference cursor is
            // rebuilt lazily on the next counter read / enable transition.
            timer.last_checked = 0;
        }
        self.hpet_reference_value = 0;
        self.hpet_reference_time = 0;
        // A restore must not leave stale queued work; the reset that precedes
        // it may have enqueued the PIT/RTC re-enable.
        self.pending = HpetPending::new();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IPS: u64 = 1_000_000_000; // 1 tick per ns keeps conversions exact

    fn enabled_hpet() -> BxHpetC {
        let mut hpet = BxHpetC::new();
        hpet.reset();
        let _ = hpet.take_pending();
        hpet.set_now(0, IPS);
        hpet.write_aligned(HPET_BASE + u64::from(HPET_CFG), HPET_CFG_ENABLE as u32, true);
        hpet
    }

    #[test]
    fn capability_register_carries_the_bios_probe_vendor_id() {
        let hpet = BxHpetC::new();
        // rombios32.c: hpet_enabled = (readl(ACPI_HPET_ADDRESS) >> 16) == 0x8086.
        assert_eq!(hpet.read_aligned(HPET_BASE + u64::from(HPET_ID)) >> 16, 0x8086);
        // Bochs hpet.cc: 3 timers, rev 1, 10 ns (100 MHz) period.
        assert_eq!((hpet.capability >> 8) & 0x1f, (HPET_NUM_TIMERS as u64) - 1);
        assert_eq!(hpet.capability >> 32, HPET_CLK_PERIOD * FS_PER_NS);
    }

    #[test]
    fn main_counter_follows_the_stamped_clock_only_while_enabled() {
        let mut hpet = enabled_hpet();
        let _ = hpet.take_pending();

        // 1000 ns at 10 ns per HPET tick = 100 counter ticks.
        hpet.set_now(1_000, IPS);
        assert_eq!(hpet.read_aligned(HPET_BASE + u64::from(HPET_COUNTER)), 100);

        // Disabling latches the counter; time passing no longer moves it.
        hpet.write_aligned(HPET_BASE + u64::from(HPET_CFG), 0, true);
        hpet.set_now(5_000, IPS);
        assert_eq!(hpet.read_aligned(HPET_BASE + u64::from(HPET_COUNTER)), 100);
    }

    #[test]
    fn oneshot_comparator_queues_an_exactly_anchored_deadline() {
        let mut hpet = enabled_hpet();
        let _ = hpet.take_pending();
        hpet.set_now(1_000, IPS);

        // Timer 0: edge, enabled, 32-bit, comparator 150 HPET ticks. The
        // 32-bit CFG write truncates the ~0 reset comparator to 0 so the
        // subsequent low-dword write fully programs it (Bochs write_aligned).
        let t0 = HPET_BASE + 0x100;
        hpet.write_aligned(
            t0 + u64::from(HPET_TN_CFG),
            (HPET_TN_ENABLE | HPET_TN_32BIT) as u32,
            true,
        );
        hpet.write_aligned(t0 + u64::from(HPET_TN_CMP), 150, true);

        let pending = hpet.take_pending();
        // Counter is at 100; 50 HPET ticks away = 500 ns after now (1000).
        assert_eq!(
            pending.timer_ops[0],
            Some(HpetTimerOp::ArmAtTicks(1_500)),
            "deadline must be anchored at the write instant"
        );

        // The fire at that instant raises the routed IRQ as an edge pair.
        hpet.set_now(1_500, IPS);
        hpet.timer_fired(0);
        let fired = hpet.take_pending();
        assert!(fired.irq_op_count >= 2);
        let route = fired.irq_ops[0].0;
        assert_eq!(fired.irq_ops[..fired.irq_op_count], [(route, false), (route, true)]);
    }

    #[test]
    fn legacy_mode_gates_pit_and_rtc_and_reroutes_timer0() {
        let mut hpet = enabled_hpet();
        let _ = hpet.take_pending();
        hpet.set_now(0, IPS);

        hpet.write_aligned(
            HPET_BASE + u64::from(HPET_CFG),
            (HPET_CFG_ENABLE | HPET_CFG_LEGACY) as u32,
            true,
        );
        let pending = hpet.take_pending();
        assert_eq!(pending.pit_irq_gate, Some(false));
        assert_eq!(pending.cmos_irq_gate, Some(false));

        // Timer 0 in legacy mode routes to IRQ0 regardless of TN_ROUTE.
        let t0 = HPET_BASE + 0x100;
        hpet.write_aligned(
            t0 + u64::from(HPET_TN_CFG),
            (HPET_TN_ENABLE | HPET_TN_32BIT | (7 << HPET_TN_INT_ROUTE_SHIFT)) as u32,
            true,
        );
        hpet.write_aligned(t0 + u64::from(HPET_TN_CMP), 10, true);
        let _ = hpet.take_pending();
        hpet.set_now(100, IPS);
        hpet.timer_fired(0);
        let fired = hpet.take_pending();
        assert_eq!(fired.irq_ops[..fired.irq_op_count], [(0, false), (0, true)]);

        // Leaving legacy mode re-enables the PIT/RTC pins (Bochs reset too).
        hpet.set_now(100, IPS);
        hpet.write_aligned(HPET_BASE + u64::from(HPET_CFG), HPET_CFG_ENABLE as u32, true);
        let pending = hpet.take_pending();
        assert_eq!(pending.pit_irq_gate, Some(true));
        assert_eq!(pending.cmos_irq_gate, Some(true));
    }

    #[test]
    fn periodic_timer_advances_its_comparator_past_the_current_tick() {
        let mut hpet = enabled_hpet();
        let _ = hpet.take_pending();
        hpet.set_now(0, IPS);

        // Timer 0 periodic (32-bit): SETVAL, comparator 100, period 100.
        let t0 = HPET_BASE + 0x100;
        hpet.write_aligned(
            t0 + u64::from(HPET_TN_CFG),
            (HPET_TN_ENABLE | HPET_TN_PERIODIC | HPET_TN_SETVAL | HPET_TN_32BIT) as u32,
            true,
        );
        hpet.write_aligned(t0 + u64::from(HPET_TN_CMP), 100, true);
        let _ = hpet.take_pending();

        // Fire at HPET tick 100 (= 1000 ns): comparator steps to 200.
        hpet.set_now(1_000, IPS);
        hpet.timer_fired(0);
        assert_eq!(hpet.timers[0].cmp, 200);
        let pending = hpet.take_pending();
        assert_eq!(pending.timer_ops[0], Some(HpetTimerOp::ArmAtTicks(2_000)));
    }

    #[test]
    fn level_timer_latches_isr_and_status_write_clears_it() {
        let mut hpet = enabled_hpet();
        let _ = hpet.take_pending();
        hpet.set_now(0, IPS);

        let t0 = HPET_BASE + 0x100;
        hpet.write_aligned(
            t0 + u64::from(HPET_TN_CFG),
            (HPET_TN_ENABLE | HPET_TN_TYPE_LEVEL | HPET_TN_32BIT) as u32,
            true,
        );
        hpet.write_aligned(t0 + u64::from(HPET_TN_CMP), 10, true);
        let _ = hpet.take_pending();

        hpet.set_now(200, IPS);
        hpet.timer_fired(0);
        assert_eq!(hpet.isr & 1, 1, "level mode must latch the status bit");
        let fired = hpet.take_pending();
        assert_eq!(fired.irq_ops[..fired.irq_op_count].last(), Some(&(0u8, true)));

        // Writing 1 to the status bit lowers the line and clears the latch.
        hpet.write_aligned(HPET_BASE + u64::from(HPET_STATUS), 1, true);
        assert_eq!(hpet.isr & 1, 0);
        let cleared = hpet.take_pending();
        assert_eq!(cleared.irq_ops[..cleared.irq_op_count], [(0, false)]);
    }

    #[test]
    fn snapshot_round_trips_bochs_register_state_and_zeroes_reference_fields() {
        let mut hpet = enabled_hpet();
        let _ = hpet.take_pending();
        hpet.set_now(0, IPS);

        // Program timer 0 (32-bit periodic) and timer 1 (edge), latch some
        // status, and let the reference cursor advance via a fire so the
        // omitted fields are genuinely non-zero at save time.
        let t0 = HPET_BASE + 0x100;
        hpet.write_aligned(
            t0 + u64::from(HPET_TN_CFG),
            (HPET_TN_ENABLE | HPET_TN_PERIODIC | HPET_TN_SETVAL | HPET_TN_32BIT) as u32,
            true,
        );
        hpet.write_aligned(t0 + u64::from(HPET_TN_CMP), 100, true);
        let t1 = HPET_BASE + 0x120;
        hpet.write_aligned(t1 + u64::from(HPET_TN_ROUTE), 0xdead_beef, true);
        hpet.set_now(1_000, IPS);
        hpet.timer_fired(0);
        let _ = hpet.take_pending();

        assert_ne!(hpet.hpet_reference_time, 0, "reference cursor should have advanced");

        let saved_config = hpet.config;
        let saved_isr = hpet.isr;
        let saved_timers: Vec<_> = hpet
            .timers
            .iter()
            .map(|t| (t.config, t.cmp, t.fsb, t.period))
            .collect();

        let mut blob = Vec::new();
        hpet.save_snapshot_v3(&mut blob).unwrap();
        assert_eq!(blob.len() as u64, hpet.snapshot_v3_len().unwrap());

        let mut restored = BxHpetC::new();
        restored.reset();
        let _ = restored.take_pending();
        // Dirty the reference fields so the restore must actively zero them.
        restored.hpet_reference_time = 0x1234;
        restored.hpet_reference_value = 0x5678;
        restored.timers[0].last_checked = 0x9abc;

        let mut reader =
            crate::snapshot::SnapshotReader::new(blob.as_slice(), blob.len() as u64).unwrap();
        restored.restore_snapshot_v3(&mut reader).unwrap();
        reader.finish_exact().unwrap();

        // Bochs register_state fields round-trip exactly.
        assert_eq!(restored.config, saved_config);
        assert_eq!(restored.isr, saved_isr);
        for (index, expected) in saved_timers.iter().enumerate() {
            let t = &restored.timers[index];
            assert_eq!((t.config, t.cmp, t.fsb, t.period), *expected, "timer {index}");
        }
        // Omitted derived fields are zeroed, matching Bochs's fresh-construct
        // restore (hpet.cc register_state does not serialize them).
        assert_eq!(restored.hpet_reference_time, 0);
        assert_eq!(restored.hpet_reference_value, 0);
        assert!(restored.timers.iter().all(|t| t.last_checked == 0));
    }
}
