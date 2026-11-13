//! Arm PrimeCell PL031 real-time clock.
//!
//! Provenance (R7): the PrimeCell Real Time Clock (PL031) Technical
//! Reference Manual, ARM DDI 0224C (r1p3), first; QEMU 11.1.2
//! `hw/rtc/pl031.c` second.
//!
//! From DDI 0224C:
//!
//! - The register map, widths and reset values (Table 3-1). The
//!   identification registers read Table 3-1's values; `RTCPeriphID2` is
//!   0x04, revision field 0, which Table 3-13 gives r1p3.
//! - `RTCCR` (§2.2.4, Table 3-5): the RTC counts only once a 1 is written
//!   to bit 0, and a read returns whether it counts. Once it counts, any
//!   further write to `RTCCR` resets the RTC value to zero, and it counts on
//!   from there; Table B-2 makes this hold for every revision. A guest that
//!   writes `RTCCR` again loses the time: Linux's `pl031_probe` does, on
//!   every boot, where EDK2's `InitializePL031` writes it only while the RTC
//!   is stopped.
//! - `RTCLR` (§2.2.4, §3.3.3): a write makes the RTC value the written one,
//!   counting or not; a read returns the last value written.
//! - The match interrupt (§2.1, §2.2.6, §3.6): a comparator asserts it when
//!   the RTC value and `RTCMR` are equal, and the raw status stays set
//!   until a 1 is written to bit 0 of `RTCICR` (Table 3-9); `RTCIMSC` bit 0
//!   unmasks it. Only equality matches, so a match value already passed
//!   when `RTCMR` or `RTCLR` is written, or when the RTC is zeroed or
//!   started, is reached only once the 32-bit counter wraps (§2.2.6), and
//!   one the RTC reads then is equal at once and raises the raw status
//!   there. Each match raises it once: the RTC reaches the same value again
//!   2^32 seconds later.
//! - Reset (§2.3.1, §3.1, Table A-2): a system reset (`PRESETn`) clears
//!   `RTCCR`, `RTCLR`, `RTCIMSC` and `RTCRIS`. The RTC stops and keeps its
//!   value, which only the power-on `nRTCRST` loses; `RTCMR` and the offset
//!   are reset only at power-on (`nPOR`).
//! - The APB interface decodes `PADDR[11:2]` (Table A-1), so an access
//!   narrower than a word reaches the register that holds it. Which byte
//!   lane it returns is the bridge's: this board's hands over the register
//!   value unshifted, as for the PL011.
//!
//! Where DDI 0224C leaves the choice to the system: reserved and test
//! locations read as zero and ignore writes, as in QEMU. QEMU starts the
//! RTC from the host's wall clock, counting, and
//! never stops or zeroes it; README's "Where the references differ" has
//! this model's differences. Here the RTC reads a configured value at power-on,
//! stopped until the guest starts it, so the guest sees the same date on
//! every run; the configured value stands for a counter some earlier
//! power-on set, where DDI 0224C's own counter starts from one (§2.2.6).

use rusty_box_core::snap::{SnapError, SnapRead, SnapResult, SnapWrite};
use rusty_box_core::{VmClock, VmInstant};

use crate::bus::{AccessWidth, RegOffset, read_by_words, write_by_words};

/// The registers' offsets in the region (DDI 0224C Table 3-1).
const RTCDR: u64 = 0x000;
const RTCMR: u64 = 0x004;
const RTCLR: u64 = 0x008;
const RTCCR: u64 = 0x00C;
const RTCIMSC: u64 = 0x010;
const RTCRIS: u64 = 0x014;
const RTCMIS: u64 = 0x018;
const RTCICR: u64 = 0x01C;
/// `RTCPeriphID0` to `RTCPCellID3`: the eight identification registers
/// that end the region, one byte of [`PL031_ID`] each.
const RTCPERIPHID0: u64 = 0xFE0;
const RTCPCELLID3: u64 = 0xFFC;

/// DDI 0224C Table 3-1.
const PL031_ID: [u8; 8] = [0x31, 0x10, 0x04, 0x00, 0x0D, 0xF0, 0x05, 0xB1];

const MICROS_PER_SECOND: u64 = 1_000_000;

/// The register an access at `offset` reaches: the interface decodes
/// `PADDR[11:2]` (DDI 0224C Table A-1).
const fn register_at(offset: RegOffset) -> u64 {
    offset.get() & 0xFFC
}

/// Whether the RTC counts (`RTCCR` bit 0, DDI 0224C §3.3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Counting {
    /// Stopped, reading `value`.
    Stopped { value: u32 },
    /// Counting: the RTC reads `offset` plus the emulated seconds, both
    /// modulo 2^32.
    Running { offset: u32 },
}

pub struct Pl031 {
    counting: Counting,
    mr: u32,
    lr: u32,
    im: u32,
    is: u32,
    /// When the RTC reaches the match value, while it counts towards it.
    alarm: Option<VmInstant>,
}

impl Pl031 {
    /// An RTC as power-on leaves it: stopped, reading `epoch_seconds`
    /// modulo 2^32.
    pub fn new(epoch_seconds: u64) -> Self {
        Self {
            counting: Counting::Stopped {
                value: epoch_seconds as u32,
            },
            mr: 0,
            lr: 0,
            im: 0,
            is: 0,
            alarm: None,
        }
    }

    /// The emulated seconds elapsed, modulo 2^32: the edges of the 1 Hz
    /// clock the RTC counts.
    fn seconds(clock: VmClock) -> u32 {
        (clock.micros() / MICROS_PER_SECOND) as u32
    }

    /// The RTC value, which `RTCDR` reads.
    fn value(&self, clock: VmClock) -> u32 {
        match self.counting {
            Counting::Stopped { value } => value,
            Counting::Running { offset } => offset.wrapping_add(Self::seconds(clock)),
        }
    }

    /// Make the RTC read `value` now, counting on from it if it counts.
    fn set_value(&mut self, value: u32, clock: VmClock) {
        self.counting = match self.counting {
            Counting::Stopped { .. } => Counting::Stopped { value },
            Counting::Running { .. } => Counting::Running {
                offset: value.wrapping_sub(Self::seconds(clock)),
            },
        };
    }

    /// A system reset (`PRESETn`, DDI 0224C §2.3.1): the RTC stops at the
    /// value it reads, and the registers of the bus clock's domain clear;
    /// the match value is kept.
    pub fn reset(&mut self, clock: VmClock) {
        self.counting = Counting::Stopped {
            value: self.value(clock),
        };
        self.lr = 0;
        self.im = 0;
        self.is = 0;
        self.alarm = None;
    }

    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        match self.counting {
            Counting::Stopped { value } => {
                w.write_u8(0)?;
                w.write_u32(value)?;
            }
            Counting::Running { offset } => {
                w.write_u8(1)?;
                w.write_u32(offset)?;
            }
        }
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
        self.counting = match r.read_u8()? {
            0 => Counting::Stopped {
                value: r.read_u32()?,
            },
            1 => Counting::Running {
                offset: r.read_u32()?,
            },
            _ => return Err(SnapError::Invalid("RTC is neither stopped nor counting")),
        };
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

    pub fn irq(&self) -> bool {
        self.is & self.im & 1 != 0
    }

    /// Compare the RTC value with the match value after a write changed
    /// either: the comparator asserts the interrupt when they are equal
    /// (DDI 0224C §2.1, §2.2.6), so equal now raises the raw status at once.
    /// Then arm the alarm for the next time they are equal.
    fn set_alarm(&mut self, clock: VmClock) {
        if self.mr == self.value(clock) {
            self.is = 1;
        }
        self.arm(clock);
    }

    /// Arm the alarm for the start of the next second, after the one
    /// running now, at which the counting RTC reads the match value: only
    /// equality matches (DDI 0224C §2.1: "When both values are equal"), so
    /// a match value the RTC has already passed is reached only once the
    /// 32-bit counter wraps (§2.2.6), up to 2^32 - 1 seconds ahead, and the
    /// value it reads now 2^32 seconds ahead. A stopped RTC reaches none.
    /// The alarm is armed only while the raw status is clear: a match while
    /// it is set changes nothing a guest can read, and `RTCICR` clearing it
    /// arms the alarm again.
    fn arm(&mut self, clock: VmClock) {
        self.alarm = None;
        if self.is != 0 {
            return;
        }
        if let Counting::Running { .. } = self.counting {
            let seconds = match self.mr.wrapping_sub(self.value(clock)) {
                0 => 1u64 << 32,
                ahead => u64::from(ahead),
            };
            let second_now = clock.micros() / MICROS_PER_SECOND;
            let at_micros = (second_now + seconds) * MICROS_PER_SECOND;
            self.alarm = Some(clock.after_micros(at_micros - clock.micros()));
        }
    }

    /// Raise the raw status if the RTC has reached the match value. The
    /// alarm stays unarmed until `RTCICR` clears the status.
    pub fn update(&mut self, clock: VmClock) {
        if self.alarm.is_some_and(|at| clock.now() >= at) {
            self.is = 1;
            self.alarm = None;
        }
    }

    /// When the RTC next reaches the match value, if it counts and the raw
    /// status is clear. The raw status records the match whether or not the
    /// interrupt is masked.
    pub fn next_deadline(&self) -> Option<VmInstant> {
        self.alarm
    }

    pub fn read(&mut self, offset: RegOffset, width: AccessWidth, clock: VmClock) -> u64 {
        read_by_words(offset, width, |offset| self.register(offset, clock))
    }

    fn register(&mut self, offset: RegOffset, clock: VmClock) -> u64 {
        let reg = register_at(offset);
        u64::from(match reg {
            RTCDR => self.value(clock),
            RTCMR => self.mr,
            RTCLR => self.lr,
            RTCCR => u32::from(matches!(self.counting, Counting::Running { .. })),
            RTCIMSC => self.im,
            RTCRIS => self.is,
            RTCMIS => self.is & self.im,
            RTCPERIPHID0..=RTCPCELLID3 => u32::from(PL031_ID[((reg - RTCPERIPHID0) >> 2) as usize]),
            _ => 0,
        })
    }

    /// A register write. Returns whether it re-armed the alarm, which moves
    /// the device's next deadline.
    #[must_use = "a re-armed alarm moves the board's next deadline"]
    pub fn write(
        &mut self,
        offset: RegOffset,
        width: AccessWidth,
        value: u64,
        clock: VmClock,
    ) -> bool {
        let mut rearmed = false;
        write_by_words(offset, width, value, |offset, word| {
            rearmed |= self.set_register(offset, word as u32, clock);
        });
        rearmed
    }

    /// Write the register at `offset`. Returns whether that re-armed the
    /// alarm.
    fn set_register(&mut self, offset: RegOffset, v: u32, clock: VmClock) -> bool {
        match register_at(offset) {
            RTCMR => {
                self.mr = v;
                self.set_alarm(clock);
                true
            }
            RTCLR => {
                self.lr = v;
                self.set_value(v, clock);
                self.set_alarm(clock);
                true
            }
            RTCCR => match self.counting {
                Counting::Stopped { value } if v & 1 != 0 => {
                    self.counting = Counting::Running {
                        offset: value.wrapping_sub(Self::seconds(clock)),
                    };
                    self.set_alarm(clock);
                    true
                }
                Counting::Stopped { .. } => false,
                Counting::Running { .. } => {
                    self.set_value(0, clock);
                    self.set_alarm(clock);
                    true
                }
            },
            RTCIMSC => {
                self.im = v & 1;
                false
            }
            RTCICR if v & 1 != 0 && self.is != 0 => {
                self.is = 0;
                self.arm(clock);
                true
            }
            RTCICR => false,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use rusty_box_core::ClockHz;

    use super::*;

    const HZ: u64 = 1_000_000;

    fn at_micros(micros: u64) -> VmClock {
        let rate = ClockHz::new(HZ).expect("a non-zero rate");
        let ticks = u128::from(micros) * u128::from(HZ) / 1_000_000;
        let ticks = u64::try_from(ticks).expect("an instant the tick count holds");
        VmClock::new(VmInstant::from_ticks(ticks), rate)
    }

    fn at_second(seconds: u64) -> VmClock {
        at_micros(seconds * 1_000_000)
    }

    fn write(rtc: &mut Pl031, offset: u64, value: u32, clock: VmClock) -> bool {
        rtc.write(
            RegOffset::new(offset),
            AccessWidth::Word,
            u64::from(value),
            clock,
        )
    }

    fn read(rtc: &mut Pl031, offset: u64, clock: VmClock) -> u64 {
        rtc.read(RegOffset::new(offset), AccessWidth::Word, clock)
    }

    /// An RTC that power-on left reading `epoch`, started at `start`.
    fn started(epoch: u64, start: VmClock) -> Pl031 {
        let mut rtc = Pl031::new(epoch);
        assert!(
            write(&mut rtc, RTCCR, 1, start),
            "starting re-arms the alarm"
        );
        rtc
    }

    /// An 8-byte access is two word accesses, the low one first, which DDI
    /// 0224C's 32-bit APB interface leaves to the bridge and QEMU's memory
    /// core does (`access_with_adjusted_size`).
    #[test]
    fn an_eight_byte_access_is_two_word_accesses() {
        let mut rtc = Pl031::new(100);
        assert!(write(&mut rtc, RTCMR, 0x1234, at_second(0)));
        let value = rtc.read(RegOffset::new(RTCDR), AccessWidth::Dword, at_second(0));
        assert_eq!(value, 100 | (0x1234 << 32));

        let lr_then_cr = 7 | (1 << 32);
        assert!(rtc.write(
            RegOffset::new(RTCLR),
            AccessWidth::Dword,
            lr_then_cr,
            at_second(0)
        ));
        assert_eq!(read(&mut rtc, RTCLR, at_second(0)), 7);
        assert_eq!(read(&mut rtc, RTCCR, at_second(0)), 1, "RTCCR started it");
        assert_eq!(read(&mut rtc, RTCDR, at_second(2)), 9);
    }

    /// Power-on leaves the RTC stopped: it reads its value whatever the
    /// time, and `RTCCR` reads 0, until a 1 written to `RTCCR` starts it.
    #[test]
    fn the_rtc_counts_only_once_started() {
        let mut rtc = Pl031::new(100);
        assert_eq!(read(&mut rtc, RTCCR, at_second(0)), 0);
        assert_eq!(read(&mut rtc, RTCDR, at_second(7)), 100);
        assert!(
            !write(&mut rtc, RTCCR, 0, at_second(7)),
            "a 0 starts nothing"
        );
        assert_eq!(read(&mut rtc, RTCDR, at_second(8)), 100);

        assert!(write(&mut rtc, RTCCR, 1, at_second(8)));
        assert_eq!(read(&mut rtc, RTCCR, at_second(8)), 1);
        assert_eq!(read(&mut rtc, RTCDR, at_second(8)), 100);
        assert_eq!(read(&mut rtc, RTCDR, at_second(11)), 103);
    }

    /// DDI 0224C Table 3-5: a write to `RTCCR` once the RTC counts resets
    /// its value to zero, and it counts on from there.
    #[test]
    fn writing_rtccr_again_zeroes_the_running_rtc() {
        let mut rtc = started(1000, at_second(0));
        assert_eq!(read(&mut rtc, RTCDR, at_second(5)), 1005);
        assert!(write(&mut rtc, RTCCR, 1, at_second(5)));
        assert_eq!(read(&mut rtc, RTCDR, at_second(5)), 0);
        assert_eq!(read(&mut rtc, RTCDR, at_second(9)), 4);
        assert_eq!(read(&mut rtc, RTCCR, at_second(9)), 1, "it still counts");
    }

    /// Loading the RTC sets its value, counting or not, and `RTCLR` reads
    /// back what was written.
    #[test]
    fn loading_sets_the_value_counting_or_not() {
        let mut rtc = Pl031::new(0);
        assert!(write(&mut rtc, RTCLR, 50, at_second(3)));
        assert_eq!(read(&mut rtc, RTCDR, at_second(9)), 50, "stopped");
        assert!(write(&mut rtc, RTCCR, 1, at_second(9)));
        assert!(write(&mut rtc, RTCLR, 500, at_second(10)));
        assert_eq!(read(&mut rtc, RTCDR, at_second(12)), 502);
        assert_eq!(read(&mut rtc, RTCLR, at_second(12)), 500);
    }

    /// A match value ahead of the counting RTC arms an alarm for the moment
    /// the RTC reaches it — the start of that second, not a whole number of
    /// seconds after the write — which latches the raw status there whether
    /// or not the interrupt is unmasked.
    #[test]
    fn the_alarm_fires_when_the_rtc_reaches_the_match_value() {
        let mut rtc = started(100, at_second(0));
        assert!(
            write(&mut rtc, RTCMR, 110, at_micros(5_400_000)),
            "writing the match value re-arms the alarm"
        );
        assert_eq!(rtc.next_deadline(), Some(VmInstant::from_ticks(10 * HZ)));

        rtc.update(at_micros(9_999_999));
        assert_eq!(read(&mut rtc, RTCRIS, at_micros(9_999_999)), 0);
        rtc.update(at_second(10));
        assert_eq!(read(&mut rtc, RTCDR, at_second(10)), 110);
        assert_eq!(read(&mut rtc, RTCRIS, at_second(10)), 1);
        assert!(!rtc.irq(), "masked: only the raw status records it");
        assert_eq!(
            rtc.next_deadline(),
            None,
            "nothing to raise while it is set"
        );

        assert!(!write(&mut rtc, RTCIMSC, 1, at_second(10)));
        assert!(rtc.irq());
        assert!(
            !write(&mut rtc, RTCICR, 0, at_second(10)),
            "a 0 clears nothing"
        );
        assert!(rtc.irq());
        assert!(
            write(&mut rtc, RTCICR, 1, at_micros(10_500_000)),
            "clearing it re-arms the alarm"
        );
        assert!(!rtc.irq());
        assert_eq!(read(&mut rtc, RTCRIS, at_micros(10_500_000)), 0);
        assert_eq!(
            rtc.next_deadline(),
            Some(VmInstant::from_ticks((10 + (1 << 32)) * HZ)),
            "the RTC reads 110 again once the counter has wrapped"
        );
        assert!(!write(&mut rtc, RTCICR, 1, at_second(11)), "already clear");
    }

    /// Only equality matches (DDI 0224C §2.1): a match value the counting
    /// RTC has already passed raises nothing when written, and is reached
    /// once the 32-bit counter wraps (§2.2.6), where the raw status rises.
    #[test]
    fn a_match_value_already_passed_is_reached_when_the_counter_wraps() {
        let mut rtc = started(100, at_second(0));
        assert!(write(&mut rtc, RTCMR, 90, at_micros(5_250_000)));
        assert_eq!(read(&mut rtc, RTCDR, at_micros(5_250_000)), 105);
        assert_eq!(
            read(&mut rtc, RTCRIS, at_micros(5_250_000)),
            0,
            "not at once"
        );
        // 90 - 105 modulo 2^32 seconds after second 5.
        let wrap = 5 + (1u64 << 32) - 15;
        assert_eq!(rtc.next_deadline(), Some(VmInstant::from_ticks(wrap * HZ)));
        rtc.update(at_micros(wrap * 1_000_000 - 1));
        assert_eq!(read(&mut rtc, RTCRIS, at_micros(wrap * 1_000_000 - 1)), 0);
        assert_eq!(
            read(&mut rtc, RTCDR, at_micros(wrap * 1_000_000 - 1)),
            89,
            "the counter has wrapped"
        );
        rtc.update(at_second(wrap));
        assert_eq!(read(&mut rtc, RTCDR, at_second(wrap)), 90);
        assert_eq!(read(&mut rtc, RTCRIS, at_second(wrap)), 1);
    }

    /// A match value the counting RTC reads when it is written is equal at
    /// once, and the comparator raises the raw status there (DDI 0224C
    /// §2.2.6); the next match is 2^32 seconds on.
    #[test]
    fn a_match_value_the_rtc_reads_raises_the_status_at_once() {
        let mut rtc = started(100, at_second(0));
        assert!(write(&mut rtc, RTCMR, 103, at_micros(3_700_000)));
        assert_eq!(read(&mut rtc, RTCRIS, at_micros(3_700_000)), 1);
        assert_eq!(rtc.next_deadline(), None);
        assert!(write(&mut rtc, RTCICR, 1, at_micros(3_800_000)));
        assert_eq!(read(&mut rtc, RTCRIS, at_micros(3_800_000)), 0);
        assert_eq!(
            rtc.next_deadline(),
            Some(VmInstant::from_ticks((3 + (1 << 32)) * HZ))
        );
    }

    /// A stopped RTC never reaches a match value ahead of it, and one it
    /// already reads raises the status at once.
    #[test]
    fn a_stopped_rtc_arms_no_alarm() {
        let mut rtc = Pl031::new(7);
        assert!(write(&mut rtc, RTCMR, 9, at_second(1)));
        assert_eq!(rtc.next_deadline(), None);
        assert!(write(&mut rtc, RTCMR, 7, at_second(1)));
        assert_eq!(read(&mut rtc, RTCRIS, at_second(1)), 1);
    }

    /// Starting the RTC arms the alarm against its value; loading it re-arms
    /// against the new value.
    #[test]
    fn starting_and_loading_re_arm() {
        let mut rtc = Pl031::new(0);
        assert!(write(&mut rtc, RTCMR, 3, at_second(1)));
        assert!(write(&mut rtc, RTCCR, 1, at_second(1)));
        assert_eq!(rtc.next_deadline(), Some(VmInstant::from_ticks(4 * HZ)));
        assert!(write(&mut rtc, RTCLR, 1, at_second(2)));
        assert_eq!(rtc.next_deadline(), Some(VmInstant::from_ticks(4 * HZ)));
    }

    /// A system reset stops the RTC at the value it reads and clears the
    /// bus clock's registers; the match value survives.
    #[test]
    fn a_system_reset_stops_the_rtc_and_keeps_its_value() {
        let mut rtc = started(40, at_second(0));
        assert!(write(&mut rtc, RTCMR, 45, at_second(1)));
        assert!(write(&mut rtc, RTCLR, 41, at_second(1)));
        assert!(!write(&mut rtc, RTCIMSC, 1, at_second(1)));
        rtc.reset(at_second(3));
        assert_eq!(read(&mut rtc, RTCCR, at_second(9)), 0);
        assert_eq!(read(&mut rtc, RTCDR, at_second(9)), 43);
        assert_eq!(read(&mut rtc, RTCMR, at_second(9)), 45);
        assert_eq!(read(&mut rtc, RTCLR, at_second(9)), 0);
        assert_eq!(read(&mut rtc, RTCIMSC, at_second(9)), 0);
        assert_eq!(rtc.next_deadline(), None);
    }

    /// The interface decodes `PADDR[11:2]`: a byte read anywhere in a
    /// register reaches it, the identification bytes included.
    #[test]
    fn a_narrow_access_reaches_the_register_that_holds_it() {
        let mut rtc = Pl031::new(0x1234_5678);
        let byte =
            |rtc: &mut Pl031, off| rtc.read(RegOffset::new(off), AccessWidth::Byte, at_second(0));
        assert_eq!(byte(&mut rtc, 0x001), 0x1234_5678);
        assert_eq!(byte(&mut rtc, 0xFE9), 0x04);
        assert_eq!(byte(&mut rtc, 0xFFE), 0xB1);
        assert_eq!(byte(&mut rtc, 0xFD0), 0, "reserved for ID expansion");
    }
}
