import Leanbox.Cpu.State
/-!
# Time

The generic timer (cpu/timer.rs) and the event register (cpu/wait.rs). Time
is counted in instructions: the system counter is a function of the tick
count `icnt` (`counterAt`).
-/
namespace Leanbox

/-- timer.rs `counter_at`: floor(icount * counter_hz / ips). -/
@[inline] def counterAt (icnt : UInt64) : UInt64 :=
  -- counter_hz / ips = 5/4 exactly, so this is floor(icnt * 5 / 4)
  (icnt >>> 2) * (5 : UInt64) + (((icnt &&& 3) * (5 : UInt64)) >>> 2)

/-- rusty_box_core time.rs `saturate`. -/
def saturate (n : Nat) : UInt64 := if n ≥ UInt64.size then MISS else n.toUInt64

/-- timer.rs `tick_reaching`: the first tick whose count reaches `count`
(`ticks_from_cycles_ceil`), `none` at u64::MAX. -/
def tickReaching (count : UInt64) : Option UInt64 :=
  let t := saturate ((count.toNat * IPS + COUNTER_HZ - 1) / COUNTER_HZ)
  if t == MISS then none else some t

/-- timer.rs `timer_change` for a timer with no offset. -/
def timerChange (ctl cval now : UInt64) : Option UInt64 :=
  if ctl &&& 1 == 0 || ctl &&& 2 != 0 then none
  else if now < cval then
    let target := now.toNat + (cval - now).toNat
    if target ≥ UInt64.size then none else tickReaching target.toUInt64
  else none

/-- timer.rs `next_timer_deadline`. -/
def nextTimerDeadline (sys : Sys) (icnt : UInt64) : Option UInt64 :=
  let now := counterAt icnt
  minDeadline (timerChange sys.cntpCtl sys.cntpCval now) (timerChange sys.cntvCtl sys.cntvCval now)

/-- timer.rs `condition_met`. -/
@[inline] def conditionMet (ctl cval count : UInt64) : Bool := ctl &&& 1 != 0 && count >= cval
/-- timer.rs `timer_line`. -/
@[inline] def timerLine (ctl cval count : UInt64) : Bool := ctl &&& 2 == 0 && conditionMet ctl cval count
/-- timer.rs `timer_ctl_read`. -/
def timerCtlRead (ctl cval count : UInt64) : UInt64 :=
  (ctl &&& 3) ||| (if conditionMet ctl cval count then 4 else 0)

-- ── the event register: wait.rs ───────────────────────────────────────
/-- wait.rs `stream_edge_after` for the virtual stream (no offset). -/
def streamEdgeAfter (ctl after : UInt64) : Option UInt64 :=
  if ctl &&& 4 == 0 then none else
  let n := (ctl >>> 4) &&& 0xF
  let half := (1 : UInt64) <<< n
  let period := half <<< 1
  let edge := if ctl &&& 8 == 0 then half else 0
  let phase := edge &&& (period - 1)
  let candidate := after - after % period + phase
  if candidate > after then some candidate
  else if candidate.toNat + period.toNat ≥ UInt64.size then none
  else some (candidate + period)

/-- wait.rs `fold_event_stream`. -/
def foldEventStream (sys : Sys) (icnt : UInt64) : Sys :=
  let now := counterAt icnt
  let sys := match streamEdgeAfter sys.cntkctl sys.eventFolded with
    | some c => if c <= now then { sys with eventSet := true } else sys
    | none => sys
  { sys with eventFolded := now }

/-- wait.rs `take_event`. -/
def takeEvent (sys : Sys) (icnt : UInt64) : Sys × Bool :=
  let sys := foldEventStream sys icnt
  ({ sys with eventSet := false }, sys.eventSet)

end Leanbox
