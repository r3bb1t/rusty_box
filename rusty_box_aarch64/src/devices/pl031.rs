//! Arm PrimeCell PL031 real-time clock.
//!
//! Provenance (R7): QEMU `hw/rtc/pl031.c` — register map, `pl031_id`, and
//! the match interrupt: `pl031_set_alarm` arms a one-shot alarm for the
//! moment the counter reaches the match value whenever the match or load
//! register is written, and `pl031_interrupt` latches the raw status when it
//! fires, masked or not. One deliberate difference: QEMU starts the counter
//! from the host's wall clock; here it starts from a configured epoch and
//! counts emulated seconds, so the guest sees the same date on every run.

use rusty_box_core::snap::{SnapError, SnapRead, SnapResult, SnapWrite};
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
    /// When the armed match alarm fires: QEMU's `PL031State::timer`.
    alarm: Option<VmInstant>,
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
            alarm: None,
        }
    }

    fn seconds(clock: VmClock) -> u64 {
        clock.micros() / 1_000_000
    }

    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_u64(self.tick_offset)?;
        for reg in [self.mr, self.lr, self.im, self.is] {
            w.write_u32(reg)?;
        }
        match self.alarm {
            Some(at) => {
                w.write_u8(1)?;
                w.write_u64(at.ticks())
            }
            None => w.write_u8(0),
        }
    }

    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        self.tick_offset = r.read_u64()?;
        self.mr = r.read_u32()?;
        self.lr = r.read_u32()?;
        self.im = r.read_u32()? & 1;
        self.is = r.read_u32()? & 1;
        self.alarm = match r.read_u8()? {
            0 => None,
            1 => Some(VmInstant::from_ticks(r.read_u64()?)),
            _ => return Err(SnapError::Invalid("RTC alarm is neither armed nor idle")),
        };
        Ok(())
    }

    /// `RTCDR`: the 32-bit counter.
    fn count(&self, clock: VmClock) -> u32 {
        self.tick_offset.wrapping_add(Self::seconds(clock)) as u32
    }

    pub fn irq(&self) -> bool {
        self.is & self.im & 1 != 0
    }

    /// QEMU `pl031_set_alarm`: arm the alarm for the moment the counter
    /// reaches the match value — at once when it already reads it, and up to
    /// 2^32 - 1 seconds ahead otherwise, the counter wrapping as it does.
    fn set_alarm(&mut self, clock: VmClock) {
        let seconds = self.mr.wrapping_sub(self.count(clock));
        if seconds == 0 {
            self.alarm = None;
            self.is = 1;
        } else {
            self.alarm = Some(clock.after_micros(u64::from(seconds) * 1_000_000));
        }
    }

    /// Fire the alarm if its moment has come (QEMU `pl031_interrupt`).
    pub fn update(&mut self, clock: VmClock) {
        if self.alarm.is_some_and(|at| clock.now() >= at) {
            self.alarm = None;
            self.is = 1;
        }
    }

    /// When the armed alarm fires, if one is armed. It fires whether or not
    /// the interrupt is masked, as QEMU's does: the raw status records it.
    pub fn next_deadline(&self) -> Option<VmInstant> {
        self.alarm
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

    /// A register write. Returns whether it re-armed the alarm, which moves
    /// the device's next deadline.
    #[must_use = "a re-armed alarm moves the board's next deadline"]
    pub fn write(&mut self, offset: RegOffset, _width: AccessWidth, value: u64, clock: VmClock) -> bool {
        let v = value as u32;
        match offset.get() {
            0x04 => {
                self.mr = v;
                self.set_alarm(clock);
                true
            }
            0x08 => {
                self.lr = v;
                self.tick_offset = u64::from(v).wrapping_sub(Self::seconds(clock));
                self.set_alarm(clock);
                true
            }
            0x10 => {
                self.im = v & 1;
                false
            }
            0x1C => {
                self.is &= !v;
                false
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_core::ClockHz;

    const HZ: u64 = 1_000_000;

    fn at_second(seconds: u64) -> VmClock {
        let rate = ClockHz::new(HZ).expect("a non-zero rate");
        VmClock::new(VmInstant::from_ticks(seconds * HZ), rate)
    }

    fn write(rtc: &mut Pl031, offset: u64, value: u32, clock: VmClock) -> bool {
        rtc.write(RegOffset::new(offset), AccessWidth::Word, u64::from(value), clock)
    }

    fn read(rtc: &mut Pl031, offset: u64, clock: VmClock) -> u64 {
        rtc.read(RegOffset::new(offset), AccessWidth::Word, clock)
    }

    /// A match value ahead of the counter arms an alarm for the second the
    /// counter reaches it, which latches the raw status there whether or
    /// not the interrupt is unmasked.
    #[test]
    fn the_alarm_fires_when_the_counter_reaches_the_match_value() {
        let mut rtc = Pl031::new(100);
        assert!(write(&mut rtc, 0x04, 110, at_second(5)), "writing the match value re-arms the alarm");
        assert_eq!(rtc.next_deadline(), Some(VmInstant::from_ticks(10 * HZ)));

        rtc.update(at_second(9));
        assert_eq!(read(&mut rtc, 0x14, at_second(9)), 0);
        rtc.update(at_second(10));
        assert_eq!(read(&mut rtc, 0x00, at_second(10)), 110);
        assert_eq!(read(&mut rtc, 0x14, at_second(10)), 1);
        assert!(!rtc.irq(), "masked: only the raw status records it");
        assert_eq!(rtc.next_deadline(), None, "the alarm is one-shot");

        assert!(!write(&mut rtc, 0x10, 1, at_second(10)));
        assert!(rtc.irq());
        assert!(!write(&mut rtc, 0x1C, 1, at_second(10)));
        assert!(!rtc.irq());
    }

    /// A match value the counter already reads raises the status at once;
    /// loading the counter re-arms against the new count.
    #[test]
    fn matching_now_fires_at_once_and_loading_re_arms() {
        let mut rtc = Pl031::new(0);
        assert!(write(&mut rtc, 0x04, 7, at_second(7)));
        assert_eq!(read(&mut rtc, 0x14, at_second(7)), 1);
        assert_eq!(rtc.next_deadline(), None);

        assert!(write(&mut rtc, 0x08, 4, at_second(7)), "loading the counter re-arms the alarm");
        assert_eq!(read(&mut rtc, 0x00, at_second(7)), 4);
        assert_eq!(rtc.next_deadline(), Some(VmInstant::from_ticks(10 * HZ)));
    }
}
