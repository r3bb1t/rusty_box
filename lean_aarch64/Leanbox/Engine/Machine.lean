import Leanbox.Engine.Run
/-!
# The machine

The run loop around the hot loop (machine.rs `sync_events`, `run_slice`,
`after_device_access`, `wake_tick`): bursts between events, the devices
brought up to date after each, and the ticks of a waiting processor passing
without retiring (`waited`).
-/
namespace Leanbox

/-- machine.rs `sync_events`. -/
def syncEvents (sys : Sys) (icnt : UInt64) : Sys :=
  let sys := { sys with timerDirty := false }
  let count := counterAt icnt
  let phys := timerLine sys.cntpCtl sys.cntpCval count
  let virt := timerLine sys.cntvCtl sys.cntvCval count
  let board := sys.board
  let sys := { sys with board := default }
  let board := board.setTimerLines phys virt false
  let (board, dev) := board.advanceDevices icnt
  let next := match minDeadline (nextTimerDeadline sys icnt) dev with
    | some t => t | none => MISS
  { sys with board := board, nextEvent := next, irqLine := board.gic.irqOut, fiqLine := board.gic.fiqOut }

/-- machine.rs `after_device_access`. -/
def afterDeviceAccess (sys : Sys) (icnt : UInt64) : Sys :=
  let sys := { sys with board := { sys.board with touched := false } }
  if sys.timerDirty || sys.board.deadlineMoved then syncEvents sys icnt
  else { sys with irqLine := sys.board.gic.irqOut, fiqLine := sys.board.gic.fiqOut }

/-- The machine between bursts. -/
structure Mach where
  rf : ByteArray
  mem : ByteArray
  sys : Sys
  pc : UInt64
  nzcv : UInt64
  icnt : UInt64
  deriving Inhabited

def Mach.retired (m : Mach) : UInt64 := m.icnt - m.sys.waited

/-- wait.rs `wake_if_pending`. -/
def wakeIfPending (sys : Sys) (icnt : UInt64) : Sys × Bool :=
  if sys.activity == 0 then (sys, true)
  else
    let interrupt := sys.irqLine || sys.fiqLine
    let (sys, wake) :=
      if sys.activity == 1 then (sys, interrupt)
      else let (sys, ev) := takeEvent sys icnt; (sys, ev || interrupt)
    (if wake then { sys with activity := 0 } else sys, wake)

/-- wait.rs `wake_deadline`. -/
def wakeDeadline (sys : Sys) : Option UInt64 :=
  if sys.activity == 2 then
    match streamEdgeAfter sys.cntkctl sys.eventFolded with
    | some c => tickReaching c
    | none => none
  else none

/-- machine.rs `wake_tick`. -/
def wakeTick (sys : Sys) (icnt : UInt64) : Option UInt64 :=
  if sys.nextEvent > icnt then (if sys.nextEvent != MISS then some sys.nextEvent else none)
  else
    let timer := match nextTimerDeadline sys icnt with
      | some t => if t > icnt then some t else none
      | none => none
    minDeadline timer (sys.board.nextDeadlineAfter icnt)

/-- machine.rs `run_until` with an instruction budget: run until `retired`
reaches `instrEnd` or the machine stops (`power` != 0, or a halt: 5). -/
partial def drive (m : Mach) (instrEnd : UInt64) : Mach :=
  if m.sys.power != 0 then m else
  let sys := if m.icnt >= m.sys.nextEvent then syncEvents m.sys m.icnt else m.sys
  let m : Mach := { m with sys := sys }
  if m.sys.activity != (0 : UInt8) then
    let (sys, woke) := wakeIfPending m.sys m.icnt
    let m : Mach := { m with sys := sys }
    if woke then drive m instrEnd else
    match minDeadline (wakeTick m.sys m.icnt) (wakeDeadline m.sys) with
    | none => { m with sys := { m.sys with power := 5 } }
    | some untl =>
      if untl > m.icnt then
        drive { m with icnt := untl, sys := { m.sys with waited := m.sys.waited + (untl - m.icnt) } } instrEnd
      else drive m instrEnd
  else
  let retired := m.retired
  if retired >= instrEnd then m else
  let toEvent := if m.sys.nextEvent > m.icnt then m.sys.nextEvent - m.icnt else 0
  let burst := min (max toEvent 1) (instrEnd - retired)
  let r := run m.rf m.mem m.sys DISPATCH 0 m.pc m.nzcv m.icnt (m.icnt + burst) MicroTlb.empty
  let sys := if r.why == 2 then afterDeviceAccess r.sys r.icnt else r.sys
  drive { rf := r.rf, mem := r.mem, sys := sys, pc := r.pc, nzcv := r.nzcv, icnt := r.icnt } instrEnd

end Leanbox
