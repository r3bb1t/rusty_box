import Leanbox.Cpu.Step
/-
The chained interpreter (bench/Chain.lean's design: one self-tail-recursive
`run`, each handler tail-calling it on the next op, every trace ending in an
`endTrace` op) and the machine's run loop around it (machine.rs `start`,
`sync_events`, `run_slice`, `after_device_access`, `wake_tick`).

Time is the tick count `icnt`: every instruction retires one tick; a
waiting processor's ticks pass without retiring (`waited`). The hot loop
runs to `lim`, the end of a burst (machine.rs: the next event, or the
instruction budget), and never past it: a trace longer than what is left is
run cut short. Interrupts are taken between traces, which end after
anything that may change what an interrupt sees.
-/
namespace Leanbox

/-- What the hot loop returns to the run loop with. `why`: 0 the burst is
spent, 2 a step asked for the run loop (a device access, a timer write, a
wait, a stop). -/
structure Res where
  rf : ByteArray
  mem : ByteArray
  sys : Sys
  pc : UInt64
  nzcv : UInt64
  icnt : UInt64
  why : UInt8
  deriving Inhabited

/-- The trace a branch goes to: one `endTrace` op, which looks the next
trace up by the PC it is given (Bochs BX_LINK_TRACE into
getTraceCacheEntry). -/
def DISPATCH : Array Op := #[.endTrace]

/-- Build the trace starting at physical address `pa` from `fetchWord`'s
words: up to 32 instructions in one page, ending after an op that ends a
trace (trace.rs `MAX_TRACE`). -/
def buildTrace (fetchWord : UInt64 → UInt32) (pa : UInt64) : Array Op := Id.run do
  let mut ops : Array Op := Array.emptyWithCapacity 33
  let mut a := pa
  for _ in [0:32] do
    let op := decode (fetchWord a)
    ops := ops.push op
    a := a + 4
    if op.endsTrace || a &&& 0xFFF == 0 then break
  return ops.push .endTrace

/-- Mark the 128-byte lines of RAM a trace at `pa` of `n` ops holds: at
most 32 instructions, so at most two lines. -/
def markCode (cmark : ByteArray) (pa : UInt64) (n : Nat) : ByteArray :=
  let o0 := pa - RAM_BASE
  let o1 := o0 + 4 * n.toUInt64 - 1
  sb (sb cmark (o0 >>> 7).toUSize 1) (o1 >>> 7).toUSize 1

/-- An interrupt deliverable now (exception.rs `deliverable_interrupt`
without EL2): its vector offset, FIQ first. -/
@[inline] def deliverable (sys : Sys) : UInt64 :=
  if sys.fiqLine && sys.daif &&& 0x40 == 0 then 0x100
  else if sys.irqLine && sys.daif &&& 0x80 == 0 then 0x80
  else 0

/-- The instruction fetch at `pc` for a trace lookup: its physical address,
or the instruction abort (or PC alignment fault) it raises. -/
def fetchTranslate (sys : Sys) (mem : ByteArray) (pc icnt : UInt64) : Sys × Except Fault UInt64 :=
  if pc &&& 3 != 0 then (sys, .error { ec := EC_PC_ALIGN, far := some pc }) else
  let (sys, r) := translate sys mem pc icnt AK_FETCH sys.el false
  match r with
  | .ok (pa, _) => (sys, .ok pa)
  | .error a => (sys, .error (instructionAbort sys pc a))

@[inline] def flashWord (b : Board) (pa : UInt64) : UInt32 :=
  if pa < FLASH1_BASE then (ld32 b.code (pa - FLASH0_BASE).toUSize).toUInt32
  else (ld32 b.vars (pa - FLASH1_BASE).toUSize).toUInt32

/-- `ops` cut to at most `n` instructions, ending in `endTrace`. -/
def cutTrace (ops : Array Op) (n : Nat) : Array Op :=
  (ops.extract 0 n).push .endTrace

-- ── the chained interpreter ────────────────────────────────────────────
/-- Where the hot loop goes on after a trace lookup. -/
structure Next where
  rf : ByteArray
  mem : ByteArray
  sys : Sys
  tr : Array Op
  pc : UInt64
  nzcv : UInt64
  icnt : UInt64
  cv : UInt64
  cp : UInt64

/-- BxEndTrace's lookup (not the burst check): an interrupt, the illegal
state check, the fetch's translation, then the next trace by the physical
address of `pc`, built on a miss and cut to what is left of the burst. -/
def lookup (rf mem : ByteArray) (sys : Sys) (pc nzcv icnt lim cv cp : UInt64) : Next :=
  let vec := deliverable sys
  if vec != 0 then
    let (rf, sys, npc) := takeException rf sys nzcv vec pc none none
    { rf, mem, sys, tr := DISPATCH, pc := npc, nzcv, icnt, cv := MISS, cp := MISS }
  else if sys.il then
    -- AArch64_CheckIllegalState, after the fetch's checks
    let (sys, r) := fetchTranslate sys mem pc icnt
    let st : St := { rf, mem, sys, pc, nzcv, icnt, flow := 0 }
    let st := match r with
      | .error f => raise st pc f
      | .ok _ => raise st pc { ec := EC_ILLEGAL }
    { rf := st.rf, mem := st.mem, sys := st.sys, tr := DISPATCH, pc := st.pc, nzcv := st.nzcv,
      icnt := st.icnt, cv := MISS, cp := MISS }
  else
  let (sys, cv, cp, pa?) :=
    if pc >>> 12 == cv then (sys, cv, cp, Except.ok (cp ||| (pc &&& 0xFFF)))
    else
      let (sys, r) := fetchTranslate sys mem pc icnt
      match r with
      | .ok pa => (sys, pc >>> 12, pa &&& ~~~0xFFF, Except.ok pa)
      | .error f => (sys, MISS, MISS, Except.error f)
  match pa? with
  | .error f =>
    let st := raise { rf, mem, sys, pc, nzcv, icnt, flow := 0 } pc f
    { rf := st.rf, mem := st.mem, sys := st.sys, tr := DISPATCH, pc := st.pc, nzcv := st.nzcv,
      icnt := st.icnt, cv := MISS, cp := MISS }
  | .ok pa =>
  let left := (lim - icnt).toNat
  if inRam pa 4 then
    let idx := ((pa >>> 2) &&& (CACHE.toUInt64 - 1)).toUSize
    let (sys, t) :=
      if agu sys.ctag idx MISS == pa then (sys, agu sys.ctr idx DISPATCH)
      else
        let t := buildTrace (fun a => (ld32 mem (a - RAM_BASE).toUSize).toUInt32) pa
        let n := t.size - 1
        ({ sys with ctag := asu sys.ctag idx pa, ctr := asu sys.ctr idx t,
                    cmark := markCode sys.cmark pa n }, t)
    let t := if t.size - 1 > left then cutTrace t left else t
    { rf, mem, sys, tr := t, pc, nzcv, icnt, cv, cp }
  else if sys.board.flashReadsArray pa then
    let idx := ((pa >>> 2) &&& (CACHE.toUInt64 - 1)).toUSize
    let (sys, t) :=
      if agu sys.ctag idx MISS == pa then (sys, agu sys.ctr idx DISPATCH)
      else
        let t := buildTrace (flashWord sys.board) pa
        ({ sys with ctag := asu sys.ctag idx pa, ctr := asu sys.ctr idx t }, t)
    let t := if t.size - 1 > left then cutTrace t left else t
    { rf, mem, sys, tr := t, pc, nzcv, icnt, cv, cp }
  else
    -- a fetch whose word cannot be kept: read through the bus, one at a time
    let st : St := { rf, mem, sys, pc, nzcv, icnt, flow := 0 }
    let (st, w) := busRead st pa 2
    match w with
    | none =>
      let st := raise st pc (instructionAbort st.sys pc ⟨FSC_SYNC_EXTERNAL, false⟩)
      { rf := st.rf, mem := st.mem, sys := st.sys, tr := DISPATCH, pc := st.pc, nzcv := st.nzcv,
        icnt := st.icnt, cv := MISS, cp := MISS }
    | some w =>
      { rf := st.rf, mem := st.mem, sys := st.sys, tr := #[decode w.toUInt32, .endTrace], pc,
        nzcv := st.nzcv, icnt := st.icnt, cv := MISS, cp := MISS }

/-- `tr[i]` is the next op; `tpc` the trace's first PC; `lim` the burst's
end; (`cv`,`cp`) the code micro-TLB (VA page, PA page). Every handler ends
in a tail call of `run`, compiled to a jump; the helpers it calls return. -/
partial def run (rf mem : ByteArray) (sys : Sys) (tr : Array Op) (i : USize)
    (tpc nzcv icnt lim cv cp : UInt64) : Res :=
  let op := agu tr i .endTrace
  let pc := tpc + 4 * i.toUInt64
  match op with
  | .addR64 d n m =>
    run (wr rf d (rr rf n + rr rf m)) mem sys tr (i+1) tpc nzcv (icnt+1) lim cv cp
  | .subR64 d n m =>
    run (wr rf d (rr rf n - rr rf m)) mem sys tr (i+1) tpc nzcv (icnt+1) lim cv cp
  | .addR32 d n m =>
    run (wr rf d ((rr rf n + rr rf m) &&& 0xFFFFFFFF)) mem sys tr (i+1) tpc nzcv (icnt+1) lim cv cp
  | .logR64 opc sh d n m amt =>
    run (wr rf d (logic opc (rr rf n) (shift64 (rr rf m) sh amt))) mem sys tr (i+1) tpc nzcv (icnt+1) lim cv cp
  | .movR d m =>
    run (wr rf d (rr rf m)) mem sys tr (i+1) tpc nzcv (icnt+1) lim cv cp
  | .subsI64 d n imm =>
    let x := rr rf n
    run (wr rf d (x - imm)) mem sys tr (i+1) tpc (awcFlags true x (~~~imm) 1) (icnt+1) lim cv cp
  | .addI64 d n imm =>
    run (wr rf d (rr rf n + imm)) mem sys tr (i+1) tpc nzcv (icnt+1) lim cv cp
  | .logI64 opc d n mask =>
    run (wr rf d (logic opc (rr rf n) mask)) mem sys tr (i+1) tpc nzcv (icnt+1) lim cv cp
  | .movc d v =>
    run (wr rf d v) mem sys tr (i+1) tpc nzcv (icnt+1) lim cv cp
  | .madd64 d n m a =>
    run (wr rf d (rr rf a + rr rf n * rr rf m)) mem sys tr (i+1) tpc nzcv (icnt+1) lim cv cp
  | .bcond cond off =>
    if conditionHolds cond nzcv then
      run rf mem sys DISPATCH 0 (branchAddr sys (pc + off)) nzcv (icnt+1) lim cv cp
    else run rf mem sys tr (i+1) tpc nzcv (icnt+1) lim cv cp
  | .b link off =>
    let rf := if link then wr rf 30 (pc + 4) else rf
    run rf mem sys DISPATCH 0 (branchAddr sys (pc + off)) nzcv (icnt+1) lim cv cp
  | .cbz sf nz t off =>
    if (mask32 sf (rr rf t) == 0) != nz then
      run rf mem sys DISPATCH 0 (branchAddr sys (pc + off)) nzcv (icnt+1) lim cv cp
    else run rf mem sys tr (i+1) tpc nzcv (icnt+1) lim cv cp
  | .tbz nz t bit off =>
    if ((rr rf t >>> bit) &&& 1 == 1) == nz then
      run rf mem sys DISPATCH 0 (branchAddr sys (pc + off)) nzcv (icnt+1) lim cv cp
    else run rf mem sys tr (i+1) tpc nzcv (icnt+1) lim cv cp
  | .br kind n =>
    let target := rr rf n
    let rf := if kind == 1 then wr rf 30 (pc + 4) else rf
    run rf mem sys DISPATCH 0 (branchAddr sys target) nzcv (icnt+1) lim cv cp
  | .nop => run rf mem sys tr (i+1) tpc nzcv (icnt+1) lim cv cp
  | .endTrace =>
    if icnt >= lim then { rf, mem, sys, pc, nzcv, icnt, why := 0 } else
    let n := lookup rf mem sys pc nzcv icnt lim cv cp
    run n.rf n.mem n.sys n.tr 0 n.pc n.nzcv n.icnt lim n.cv n.cp
  | _ =>
    -- every other op through the reference executor
    let st := execOp { rf, mem, sys, pc, nzcv, icnt, flow := 0 } pc op
    if st.flow == 2 || st.sys.board.touched || st.sys.timerDirty then
      { rf := st.rf, mem := st.mem, sys := st.sys, pc := st.pc, nzcv := st.nzcv, icnt := st.icnt, why := 2 }
    else if st.flow == 0 then run st.rf st.mem st.sys tr (i+1) tpc st.nzcv st.icnt lim cv cp
    else if st.flow == 1 then run st.rf st.mem st.sys DISPATCH 0 st.pc st.nzcv st.icnt lim cv cp
    else run st.rf st.mem st.sys DISPATCH 0 st.pc st.nzcv st.icnt lim MISS MISS

-- ── the run loop: machine.rs ───────────────────────────────────────────
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
  let r := run m.rf m.mem m.sys DISPATCH 0 m.pc m.nzcv m.icnt (m.icnt + burst) MISS MISS
  let sys := if r.why == 2 then afterDeviceAccess r.sys r.icnt else r.sys
  drive { rf := r.rf, mem := r.mem, sys := sys, pc := r.pc, nzcv := r.nzcv, icnt := r.icnt } instrEnd

end Leanbox
