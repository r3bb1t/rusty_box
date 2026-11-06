import Leanbox.Dev.Pl031
/-!
# PL031 checks

The pl031.rs unit tests as `#guard`s, checked when this module is built (`lake test`).
-/
namespace Leanbox.Dev.Pl031

namespace Tests
open State

-- pl031.rs tests HZ, at_micros, at_second: a 1 MHz clock
def HZ : UInt64 := 1_000_000
def atMicros (m : UInt64) : Clock := { now := m * HZ / 1_000_000, rate := HZ.toNat }
def atSecond (n : UInt64) : Clock := atMicros (n * 1_000_000)

def wr (s : State) (off : UInt64) (v : UInt32) (c : Clock) : State × Bool :=
  s.writeC off 2 v.toUInt64 c
def wrS (s : State) (off : UInt64) (v : UInt32) (c : Clock) : State := (wr s off v c).1
def rd (s : State) (off : UInt64) (c : Clock) : UInt64 := (s.readC off 2 c).2

-- pl031.rs tests `started`
def started (epoch : UInt64) (start : Clock) : State × Bool := wr (State.new epoch) RTCCR 1 start

-- an_eight_byte_access_is_two_word_accesses
def tDword : Bool :=
  let (s, r1) := wr (State.new 100) RTCMR 0x1234 (atSecond 0)
  let v := (s.readC RTCDR 3 (atSecond 0)).2
  let (s, r2) := s.writeC RTCLR 3 ((7 : UInt64) ||| ((1 : UInt64) <<< 32)) (atSecond 0)
  r1 && v == (100 : UInt64) ||| ((0x1234 : UInt64) <<< 32) && r2
    && rd s RTCLR (atSecond 0) == 7 && rd s RTCCR (atSecond 0) == 1
    && rd s RTCDR (atSecond 2) == 9
#guard tDword

-- the_rtc_counts_only_once_started
def tStarted : Bool :=
  let s := State.new 100
  let ok := rd s RTCCR (atSecond 0) == 0 && rd s RTCDR (atSecond 7) == 100
  let (s, r0) := wr s RTCCR 0 (atSecond 7)
  let ok := ok && !r0 && rd s RTCDR (atSecond 8) == 100
  let (s, r1) := wr s RTCCR 1 (atSecond 8)
  ok && r1 && rd s RTCCR (atSecond 8) == 1 && rd s RTCDR (atSecond 8) == 100
    && rd s RTCDR (atSecond 11) == 103
#guard tStarted

-- writing_rtccr_again_zeroes_the_running_rtc
def tRezero : Bool :=
  let (s, r0) := started 1000 (atSecond 0)
  let ok := r0 && rd s RTCDR (atSecond 5) == 1005
  let (s, r1) := wr s RTCCR 1 (atSecond 5)
  ok && r1 && rd s RTCDR (atSecond 5) == 0 && rd s RTCDR (atSecond 9) == 4
    && rd s RTCCR (atSecond 9) == 1
#guard tRezero

-- loading_sets_the_value_counting_or_not
def tLoad : Bool :=
  let (s, r1) := wr (State.new 0) RTCLR 50 (atSecond 3)
  let ok := r1 && rd s RTCDR (atSecond 9) == 50
  let (s, r2) := wr s RTCCR 1 (atSecond 9)
  let (s, r3) := wr s RTCLR 500 (atSecond 10)
  ok && r2 && r3 && rd s RTCDR (atSecond 12) == 502 && rd s RTCLR (atSecond 12) == 500
#guard tLoad

-- the_alarm_fires_when_the_rtc_reaches_the_match_value
def tAlarm : Bool :=
  let (s, _) := started 100 (atSecond 0)
  let (s, r1) := wr s RTCMR 110 (atMicros 5_400_000)
  let ok := r1 && s.nextDeadline == some (10 * HZ)
  let s := s.updateC (atMicros 9_999_999)
  let ok := ok && rd s RTCRIS (atMicros 9_999_999) == 0
  let s := s.updateC (atSecond 10)
  let ok := ok && rd s RTCDR (atSecond 10) == 110 && rd s RTCRIS (atSecond 10) == 1
    && !s.irq && s.nextDeadline == none
  let (s, r2) := wr s RTCIMSC 1 (atSecond 10)
  let ok := ok && !r2 && s.irq
  let (s, r3) := wr s RTCICR 0 (atSecond 10)
  let ok := ok && !r3 && s.irq
  let (s, r4) := wr s RTCICR 1 (atSecond 10)
  ok && !r4 && !s.irq
#guard tAlarm

-- a_stopped_rtc_arms_no_alarm
def tStoppedNoAlarm : Bool :=
  let (s, r1) := wr (State.new 7) RTCMR 9 (atSecond 1)
  let ok := r1 && s.nextDeadline == none
  let (s, r2) := wr s RTCMR 7 (atSecond 1)
  ok && r2 && rd s RTCRIS (atSecond 1) == 1
#guard tStoppedNoAlarm

-- starting_and_loading_re_arm
def tRearm : Bool :=
  let (s, r1) := wr (State.new 0) RTCMR 3 (atSecond 1)
  let (s, r2) := wr s RTCCR 1 (atSecond 1)
  let ok := r1 && r2 && s.nextDeadline == some (4 * HZ)
  let (s, r3) := wr s RTCLR 1 (atSecond 2)
  ok && r3 && s.nextDeadline == some (4 * HZ)
#guard tRearm

-- a_system_reset_stops_the_rtc_and_keeps_its_value
def tReset : Bool :=
  let (s, _) := started 40 (atSecond 0)
  let (s, r1) := wr s RTCMR 45 (atSecond 1)
  let (s, r2) := wr s RTCLR 41 (atSecond 1)
  let (s, r3) := wr s RTCIMSC 1 (atSecond 1)
  let s := s.resetC (atSecond 3)
  r1 && r2 && !r3
    && rd s RTCCR (atSecond 9) == 0 && rd s RTCDR (atSecond 9) == 43
    && rd s RTCMR (atSecond 9) == 45 && rd s RTCLR (atSecond 9) == 0
    && rd s RTCIMSC (atSecond 9) == 0 && s.nextDeadline == none
#guard tReset

-- a_narrow_access_reaches_the_register_that_holds_it
def tNarrow : Bool :=
  let s := State.new 0x1234_5678
  let byte (off : UInt64) := (s.readC off 0 (atSecond 0)).2
  byte 0x001 == 0x1234_5678 && byte 0xFE9 == 0x04 && byte 0xFFE == 0xB1 && byte 0xFD0 == 0
#guard tNarrow

-- At the board's 50 MHz: the default epoch reads back until started; a match
-- 3 s ahead of a start at tick 7 is due 3 s of ticks after the write's own
-- tick (`after_micros` counts from `clock.now()`, the microseconds of the
-- start of second 3 minus the write's microseconds, 0 here), so the RTC reads
-- the match value 7 ticks before the raw status rises.
def tMachineRate : Bool :=
  let s := State.new DEFAULT_RTC_EPOCH
  let ok := (s.read RTCDR 2 123_456_789).2 == DEFAULT_RTC_EPOCH
  let (s, r1) := s.write RTCCR 2 1 7
  let (s, r2) := s.write RTCMR 2 (DEFAULT_RTC_EPOCH + 3) 7
  let ok := ok && r1 && r2 && s.nextDeadline == some 150_000_007
  let s := s.update 150_000_006
  let ok := ok && rd s RTCRIS (Clock.atTick 150_000_006) == 0
    && (s.read RTCDR 2 150_000_000).2 == DEFAULT_RTC_EPOCH + 3
  let s := s.update 150_000_007
  ok && rd s RTCRIS (Clock.atTick 150_000_007) == 1
#guard tMachineRate

end Tests

end Leanbox.Dev.Pl031
