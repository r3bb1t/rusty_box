/-
  Arm PrimeCell PL031 real-time clock: an exact port of
  rusty_box_aarch64/src/devices/pl031.rs (`Pl031`), for lockstep runs
  against the Rust machine.

  The RTC counts the emulated seconds: a running RTC reads `offset` plus the
  machine's elapsed microseconds divided by a million, modulo 2^32. Power-on
  leaves it stopped at the configured epoch (machine.rs
  `VirtConfig::DEFAULT_RTC_EPOCH` on the oracle's default board). The match
  alarm is a deadline in machine ticks, armed for the first tick at or after
  the start of the second the RTC reaches `RTCMR`.

  Widths are `log2` (0 byte, 1 half, 2 word, 3 dword); offsets are relative
  to the device's base. The public `reset`/`update`/`read`/`write` take the
  machine tick at a 50 MHz machine rate; the `...C` forms take a `Clock`
  with any rate (the Rust unit tests run a 1 MHz clock).
-/

namespace Leanbox.Dev.Pl031

/-! ## Register offsets (pl031.rs `RTCDR` ... `RTCPCELLID3`) -/

def RTCDR : UInt64 := 0x000
def RTCMR : UInt64 := 0x004
def RTCLR : UInt64 := 0x008
def RTCCR : UInt64 := 0x00C
def RTCIMSC : UInt64 := 0x010
def RTCRIS : UInt64 := 0x014
def RTCMIS : UInt64 := 0x018
def RTCICR : UInt64 := 0x01C
def RTCPERIPHID0 : UInt64 := 0xFE0
def RTCPCELLID3 : UInt64 := 0xFFC

-- pl031.rs PL031_ID (DDI 0224C Table 3-1)
def PL031_ID : Array UInt32 := #[0x31, 0x10, 0x04, 0x00, 0x0D, 0xF0, 0x05, 0xB1]

-- pl031.rs MICROS_PER_SECOND
def MICROS_PER_SECOND : UInt64 := 1_000_000

-- machine.rs VirtConfig::DEFAULT_RTC_EPOCH (2026-01-01T00:00:00Z)
def DEFAULT_RTC_EPOCH : UInt64 := 1_767_225_600

-- rusty_box_core time.rs ClockHz::BOCHS_DEFAULT
def MACHINE_RATE : Nat := 50_000_000

/-! ## Time (rusty_box_core/src/time.rs) -/

-- time.rs `saturate` (u128 -> u64, clamping)
def saturate (n : Nat) : UInt64 :=
  if n > UInt64.size - 1 then UInt64.ofNat (UInt64.size - 1) else UInt64.ofNat n

-- time.rs VmClock { now, rate }
structure Clock where
  now : UInt64
  rate : Nat := MACHINE_RATE
deriving Inhabited

-- a machine clock at tick `now`, 50 MHz
def Clock.atTick (now : UInt64) : Clock := { now := now }

-- time.rs VmClock::micros (ClockHz::micros_at): now * 10^6 / rate, rounded
-- down, saturating
def Clock.micros (c : Clock) : UInt64 :=
  saturate (c.now.toNat * 1_000_000 / c.rate)

-- time.rs ClockHz::ticks_from_micros_ceil: ceil(micros * rate / 10^6),
-- never zero, saturating
def ticksFromMicrosCeil (rate : Nat) (micros : UInt64) : UInt64 :=
  let ticks := (micros.toNat * rate + 1_000_000 - 1) / 1_000_000
  saturate (if ticks == 0 then 1 else ticks)

-- time.rs VmClock::after_micros: now + ticks_from_micros_ceil, saturating
def Clock.afterMicros (c : Clock) (micros : UInt64) : UInt64 :=
  saturate (c.now.toNat + (ticksFromMicrosCeil c.rate micros).toNat)

/-! ## State (pl031.rs `struct Pl031`) -/

-- pl031.rs Counting
inductive Counting where
  /-- Stopped, reading `value`. -/
  | stopped (value : UInt32)
  /-- Counting: the RTC reads `offset` plus the emulated seconds, both
  modulo 2^32. -/
  | running (offset : UInt32)
deriving Inhabited, BEq, Repr

structure State where
  counting : Counting
  mr : UInt32
  lr : UInt32
  im : UInt32
  is : UInt32
  /-- `alarm`: the machine tick the RTC reaches the match value, while it
  counts towards it. -/
  alarm : Option UInt64
deriving Inhabited

-- pl031.rs Pl031::new: stopped, reading `epoch_seconds` modulo 2^32
def State.new (epochSeconds : UInt64) : State :=
  { counting := .stopped epochSeconds.toUInt32
    mr := 0, lr := 0, im := 0, is := 0
    alarm := none }

-- pl031.rs register_at: the interface decodes PADDR[11:2]
def registerAt (offset : UInt64) : UInt64 := offset &&& 0xFFC

namespace State

-- pl031.rs Pl031::seconds: the emulated seconds elapsed, modulo 2^32
def seconds (c : Clock) : UInt32 := (c.micros / MICROS_PER_SECOND).toUInt32

-- pl031.rs Pl031::value: what RTCDR reads
def value (s : State) (c : Clock) : UInt32 :=
  match s.counting with
  | .stopped v => v
  | .running o => o + seconds c

-- pl031.rs Pl031::set_value
def setValue (s : State) (v : UInt32) (c : Clock) : State :=
  match s.counting with
  | .stopped _ => { s with counting := .stopped v }
  | .running _ => { s with counting := .running (v - seconds c) }

-- pl031.rs Pl031::reset: the RTC stops at the value it reads; RTCLR,
-- RTCIMSC and RTCRIS clear; RTCMR is kept.
def resetC (s : State) (c : Clock) : State :=
  { s with counting := .stopped (s.value c), lr := 0, im := 0, is := 0, alarm := none }

-- pl031.rs Pl031::reset at machine tick `now` (board.rs reset_devices)
def reset (s : State) (now : UInt64) : State := s.resetC (Clock.atTick now)

-- pl031.rs Pl031::irq
def irq (s : State) : Bool := s.is &&& s.im &&& 1 != 0

-- pl031.rs Pl031::set_alarm. `(second_now + seconds) * MICROS_PER_SECOND`
-- and the subtraction after it are u64 arithmetic, wrapping as in the
-- release build; at 50 MHz neither can leave u64.
def setAlarm (s : State) (c : Clock) : State :=
  let s := { s with alarm := none }
  let secs := s.mr - s.value c
  if secs == 0 then { s with is := 1 }
  else match s.counting with
    | .running _ =>
      let micros := c.micros
      let secondNow := micros / MICROS_PER_SECOND
      let atMicros := (secondNow + secs.toUInt64) * MICROS_PER_SECOND
      { s with alarm := some (c.afterMicros (atMicros - micros)) }
    | .stopped _ => s

-- pl031.rs Pl031::update: raise the raw status if the RTC has reached the
-- match value
def updateC (s : State) (c : Clock) : State :=
  match s.alarm with
  | some at_ => if c.now ≥ at_ then { s with alarm := none, is := 1 } else s
  | none => s

-- pl031.rs Pl031::update at machine tick `now` (only `clock.now()` is read)
def update (s : State) (now : UInt64) : State := s.updateC (Clock.atTick now)

-- pl031.rs Pl031::next_deadline (machine ticks)
def nextDeadline (s : State) : Option UInt64 := s.alarm

-- pl031.rs Pl031::register
def register (s : State) (offset : UInt64) (c : Clock) : UInt64 :=
  let reg := registerAt offset
  (if reg == RTCDR then s.value c
  else if reg == RTCMR then s.mr
  else if reg == RTCLR then s.lr
  else if reg == RTCCR then
    (match s.counting with | .running _ => 1 | .stopped _ => 0)
  else if reg == RTCIMSC then s.im
  else if reg == RTCRIS then s.is
  else if reg == RTCMIS then s.is &&& s.im
  else if RTCPERIPHID0 ≤ reg && reg ≤ RTCPCELLID3 then
    PL031_ID[((reg - RTCPERIPHID0) >>> 2).toNat]!
  else 0).toUInt64

-- pl031.rs Pl031::read via bus.rs read_by_words: an 8-byte access is
-- `offset` then `offset + 4`, low word first; any other width is one
-- register, unshifted and unmasked.
def readC (s : State) (off : UInt64) (log2 : UInt8) (c : Clock) : State × UInt64 :=
  if log2 ≥ 3 then
    (s, s.register off c ||| (s.register (off + 4) c <<< 32))
  else (s, s.register off c)

-- pl031.rs Pl031::read at machine tick `now` (board.rs read_outside_ram)
def read (s : State) (off : UInt64) (log2 : UInt8) (now : UInt64) : State × UInt64 :=
  s.readC off log2 (Clock.atTick now)

-- pl031.rs Pl031::set_register: whether the write re-armed the alarm
def setRegister (s : State) (offset : UInt64) (v : UInt32) (c : Clock) : State × Bool :=
  let reg := registerAt offset
  if reg == RTCMR then
    (({ s with mr := v } : State).setAlarm c, true)
  else if reg == RTCLR then
    ((({ s with lr := v } : State).setValue v c).setAlarm c, true)
  else if reg == RTCCR then
    match s.counting with
    | .stopped value =>
      if v &&& 1 != 0 then
        (({ s with counting := .running (value - seconds c) } : State).setAlarm c, true)
      else (s, false)
    | .running _ => ((s.setValue 0 c).setAlarm c, true)
  else if reg == RTCIMSC then ({ s with im := v &&& 1 }, false)
  else if reg == RTCICR then ({ s with is := s.is &&& ~~~(v &&& 1) }, false)
  else (s, false)

-- pl031.rs Pl031::write via bus.rs write_by_words (`word as u32`). Returns
-- whether either word re-armed the alarm, which moves the device's next
-- deadline.
def writeC (s : State) (off : UInt64) (log2 : UInt8) (v : UInt64) (c : Clock) : State × Bool :=
  if log2 ≥ 3 then
    let (s, r1) := s.setRegister off (v &&& 0xFFFF_FFFF).toUInt32 c
    let (s, r2) := s.setRegister (off + 4) (v >>> 32).toUInt32 c
    (s, r1 || r2)
  else s.setRegister off v.toUInt32 c

-- pl031.rs Pl031::write at machine tick `now` (board.rs write_outside_ram)
def write (s : State) (off : UInt64) (log2 : UInt8) (v : UInt64) (now : UInt64) : State × Bool :=
  s.writeC off log2 v (Clock.atTick now)

end State

/-! ## Checks reproducing pl031.rs unit tests -/

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
