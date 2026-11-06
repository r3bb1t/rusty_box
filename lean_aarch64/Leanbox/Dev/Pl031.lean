/-!
# PL031 real-time clock

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

end Leanbox.Dev.Pl031
