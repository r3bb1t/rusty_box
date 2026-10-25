//! Arm PrimeCell PL031 real-time clock.
//!
//! Provenance (R7): QEMU `hw/rtc/pl031.c` — register map, `pl031_id`, the
//! match interrupt. One deliberate difference: QEMU starts the counter from
//! the host's wall clock; here it starts from a configured epoch and counts
//! emulated seconds, so the guest sees the same date on every run.

use rusty_box_core::{VmClock, VmInstant};

use crate::bus::{AccessWidth, RegOffset};

const PL031_ID: [u8; 8] = [0x31, 0x10, 0x14, 0x00, 0x0D, 0xF0, 0x05, 0xB1];

pub struct Pl031 {
    /// The counter value at emulated time zero.
    tick_offset: u64,
    mr: u32,
    lr: u32,
    im: u32,
    is: u32,
}

impl Pl031 {
    /// A clock that reads `epoch_seconds` at emulated time zero.
    pub fn new(epoch_seconds: u64) -> Self {
        Self {
            tick_offset: epoch_seconds,
            mr: 0,
            lr: 0,
            im: 0,
            is: 0,
        }
    }

    fn seconds(clock: VmClock) -> u64 {
        clock.micros() / 1_000_000
    }

    /// `RTCDR`: the 32-bit counter.
    fn count(&self, clock: VmClock) -> u32 {
        self.tick_offset.wrapping_add(Self::seconds(clock)) as u32
    }

    pub fn irq(&self) -> bool {
        self.is & self.im & 1 != 0
    }

    /// Raise the match interrupt if the counter has reached the match value.
    pub fn update(&mut self, clock: VmClock) {
        if self.count(clock) == self.mr {
            self.is |= 1;
        }
    }

    /// The next instant at which the counter will equal the match value,
    /// if the match interrupt is enabled.
    pub fn next_deadline(&self, clock: VmClock) -> Option<VmInstant> {
        if self.im & 1 == 0 {
            return None;
        }
        let now = self.count(clock);
        let delta = u64::from(self.mr.wrapping_sub(now));
        if delta == 0 {
            return None;
        }
        let target_seconds = Self::seconds(clock) + delta;
        Some(VmInstant::from_ticks(
            target_seconds.saturating_mul(clock.rate().get()),
        ))
    }

    pub fn read(&mut self, offset: RegOffset, _width: AccessWidth, clock: VmClock) -> u64 {
        let off = offset.get();
        u64::from(match off {
            0x00 => self.count(clock),
            0x04 => self.mr,
            0x08 => self.lr,
            // RTCCR: the counter always runs.
            0x0C => 1,
            0x10 => self.im,
            0x14 => self.is,
            0x18 => self.is & self.im,
            0xFE0..=0xFFF => u32::from(PL031_ID[((off - 0xFE0) >> 2) as usize & 7]),
            _ => 0,
        })
    }

    pub fn write(&mut self, offset: RegOffset, _width: AccessWidth, value: u64, clock: VmClock) {
        let v = value as u32;
        match offset.get() {
            0x04 => {
                self.mr = v;
                self.update(clock);
            }
            0x08 => {
                self.lr = v;
                self.tick_offset = u64::from(v).wrapping_sub(Self::seconds(clock));
                self.update(clock);
            }
            0x10 => self.im = v & 1,
            0x1C => self.is &= !v,
            _ => {}
        }
    }
}
