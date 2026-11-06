import Leanbox.Cpu.Step
import Leanbox.Cpu.Poll
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
or the instruction abort (or PC alignment fault) it raises. Inline, as
`translate` is. -/
@[inline] def fetchTranslate (sys : Sys) (mem : ByteArray) (pc icnt : UInt64) : Sys × Except Fault UInt64 :=
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

/-- The data micro-TLB's refill: the RAM page `va` reaches for an access of
`kind` at the current EL, when it is Normal memory wholly in RAM that the
access may use (`MISS` otherwise, which leaves the access to `execOp`). -/
def microFill (sys : Sys) (mem : ByteArray) (va icnt : UInt64) (kind : UInt8) : Sys × UInt64 :=
  let (sys, r) := translate sys mem va icnt kind sys.el false
  match r with
  | .ok (pa, dev) =>
    if !dev && inRam (pa &&& ~~~0xFFF) 0x1000 then (sys, (pa - RAM_BASE) &&& ~~~0xFFF) else (sys, MISS)
  | .error _ => (sys, MISS)

/-- `microFill` from a TLB hit alone, with nothing allocated: the RAM page,
or `MISS` when the TLB does not hold `va`'s page (or the MMU is off) or the
page cannot be used (then `microFill` decides). -/
@[inline] def microProbe (sys : Sys) (va : UInt64) (kind : UInt8) : UInt64 :=
  if !mmuOn sys then MISS else
  let vp := va >>> 12
  let i := ((vp &&& (TLB_SETS.toUInt64 - 1)) * 2).toUSize
  if agu sys.tlb i MISS != vp then MISS else
  let e := agu sys.tlb (i + 1) 0
  let pa := e &&& ~~~0xFFF
  if e &&& ATTR_DEVICE == 0 && e &&& neededPerm kind sys.el != 0 && inRam pa 0x1000 then pa - RAM_BASE
  else MISS

/-- Whether a store to RAM offset `o` of `bytes` may skip `noteStore`: no
monitor armed, no trace holding code on its lines. -/
@[inline] def plainStore (sys : Sys) (o bytes : UInt64) : Bool :=
  !sys.monArmed && gb sys.cmark (o >>> 7).toUSize == 0 && gb sys.cmark ((o + bytes - 1) >>> 7).toUSize == 0

/-- Whether an access of `bytes` at `va` within a page may take a fast path:
aligned, or misaligned with SCTLR_EL1.A clear (micro-TLB pages are Normal
memory). -/
@[inline] def fastAligned (sys : Sys) (va bytes : UInt64) : Bool :=
  (va &&& 0xFFF) + bytes <= 0x1000 && (va &&& (bytes - 1) == 0 || sys.sctlr &&& SCTLR_A == 0)

@[inline] def extendLoad (v : UInt64) (size opc : UInt8) : UInt64 :=
  if opc == 1 then v else
  let bits : UInt64 := 8 <<< size.toUInt64
  if opc == 2 then sext v bits else sext v bits &&& 0xFFFF_FFFF

/-- The micro-TLBs `run` keeps from op to op: the code page (`cv` its VA
page, `cp` its PA page), two read pages and one write page (`dv*` the VA
page, `dp*` the RAM offset of the page). One object, so the loop carries one
value for all eight, updated in place while `run` holds the only reference. -/
structure MicroTlb where
  cv : UInt64
  cp : UInt64
  dvr : UInt64
  dpr : UInt64
  dvw : UInt64
  dpw : UInt64
  dvr2 : UInt64
  dpr2 : UInt64

/-- Every entry empty. -/
def MicroTlb.empty : MicroTlb := ⟨MISS, MISS, MISS, MISS, MISS, MISS, MISS, MISS⟩

/-- A read page refilled: the new page first, the one it displaces second. -/
@[inline] def MicroTlb.refillRead (u : MicroTlb) (vp pg : UInt64) : MicroTlb :=
  { u with dvr := vp, dpr := pg, dvr2 := u.dvr, dpr2 := u.dpr }

set_option hygiene false in
/-- Run the op at `i` through the reference executor and go on as it says. -/
macro "slowpath!" : term => `(
    let st := execOp { rf, mem, sys, pc, nzcv, icnt, flow := 0 } pc op
    if st.flow == 2 || st.sys.board.touched || st.sys.timerDirty then
      { rf := st.rf, mem := st.mem, sys := st.sys, pc := st.pc, nzcv := st.nzcv, icnt := st.icnt, why := 2 }
    else if st.flow == 0 then run st.rf st.mem st.sys tr (i+1) tpc st.nzcv st.icnt lim mt
    else if st.flow == 1 then run st.rf st.mem st.sys DISPATCH 0 st.pc st.nzcv st.icnt lim mt
    else run st.rf st.mem st.sys DISPATCH 0 st.pc st.nzcv st.icnt lim MicroTlb.empty)

set_option hygiene false in
/-- A load of `bytes` at `va` through the read micro-TLB: on a hit, `hit o`
with the RAM offset; on a miss, a refill and the op again; else the slow
path. -/
macro "loadAt!" va:term "," bytes:term "," hit:term : term => `(
    if fastAligned sys $va $bytes then
      if $va >>> 12 == mt.dvr then $hit (mt.dpr ||| ($va &&& 0xFFF))
      else if $va >>> 12 == mt.dvr2 then $hit (mt.dpr2 ||| ($va &&& 0xFFF))
      else
        let pg := microProbe sys $va AK_READ
        if pg != MISS then run rf mem sys tr i tpc nzcv icnt lim (mt.refillRead ($va >>> 12) pg)
        else
        let (sys', pg) := microFill sys mem $va icnt AK_READ
        if pg != MISS then run rf mem sys' tr i tpc nzcv icnt lim (mt.refillRead ($va >>> 12) pg)
        else let sys := sys'; slowpath!
    else slowpath!)

set_option hygiene false in
macro "storeAt!" va:term "," bytes:term "," hit:term : term => `(
    if fastAligned sys $va $bytes then
      if $va >>> 12 == mt.dvw then
        let o := mt.dpw ||| ($va &&& 0xFFF)
        if plainStore sys o $bytes then $hit o else slowpath!
      else
        let pg := microProbe sys $va AK_WRITE
        if pg != MISS then run rf mem sys tr i tpc nzcv icnt lim { mt with dvw := $va >>> 12, dpw := pg }
        else
        let (sys', pg) := microFill sys mem $va icnt AK_WRITE
        if pg != MISS then run rf mem sys' tr i tpc nzcv icnt lim { mt with dvw := $va >>> 12, dpw := pg }
        else let sys := sys'; slowpath!
    else slowpath!)

/-- `tr[i]` is the next op; `tpc` the trace's first PC; `lim` the burst's
end; (`cv`,`cp`) the code micro-TLB (VA page, PA page); (`dvr`,`dpr`) and
(`dvw`,`dpw`) the data micro-TLBs for reads and writes (VA page, RAM offset
page). Every handler ends in a tail call of `run`, compiled to a jump; the
helpers it calls return. -/
partial def run (rf mem : ByteArray) (sys : Sys) (tr : Array Op) (i : USize)
    (tpc nzcv icnt lim : UInt64) (mt : MicroTlb) : Res :=
  let op := agu tr i .endTrace
  let pc := tpc + 4 * i.toUInt64
  match op with
  | .addR64 d n m =>
    run (wr rf d (rr rf n + rr rf m)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .subR64 d n m =>
    run (wr rf d (rr rf n - rr rf m)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .addR32 d n m =>
    run (wr rf d ((rr rf n + rr rf m) &&& 0xFFFFFFFF)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .logR64 opc sh d n m amt =>
    run (wr rf d (logic opc (rr rf n) (shift64 (rr rf m) sh amt))) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .movR d m =>
    run (wr rf d (rr rf m)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .subsI64 d n imm =>
    let x := rr rf n
    run (wr rf d (x - imm)) mem sys tr (i+1) tpc (awcFlags true x (~~~imm) 1) (icnt+1) lim mt
  | .addI64 d n imm =>
    run (wr rf d (rr rf n + imm)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .logI64 opc d n mask =>
    run (wr rf d (logic opc (rr rf n) mask)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .movc d v =>
    run (wr rf d v) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .madd64 d n m a =>
    run (wr rf d (rr rf a + rr rf n * rr rf m)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .bcond cond off =>
    if conditionHolds cond nzcv then
      run rf mem sys DISPATCH 0 (branchAddr sys (pc + off)) nzcv (icnt+1) lim mt
    else run rf mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .b link off =>
    let rf := if link then wr rf 30 (pc + 4) else rf
    run rf mem sys DISPATCH 0 (branchAddr sys (pc + off)) nzcv (icnt+1) lim mt
  | .cbz sf nz t off =>
    if (mask32 sf (rr rf t) == 0) != nz then
      run rf mem sys DISPATCH 0 (branchAddr sys (pc + off)) nzcv (icnt+1) lim mt
    else run rf mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .tbz nz t bit off =>
    if ((rr rf t >>> bit) &&& 1 == 1) == nz then
      run rf mem sys DISPATCH 0 (branchAddr sys (pc + off)) nzcv (icnt+1) lim mt
    else run rf mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .br kind n =>
    let target := rr rf n
    let rf := if kind == 1 then wr rf 30 (pc + 4) else rf
    run rf mem sys DISPATCH 0 (branchAddr sys target) nzcv (icnt+1) lim mt
  | .adr page d off =>
    run (wr rf d (if page then (pc &&& ~~~0xFFF) + off else pc + off)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .addsubI sf sub setf d n imm =>
    let x := mask32 sf (rr rf n)
    run (wr rf d (asRes sf sub x imm)) mem sys tr (i+1) tpc (if setf then asFlags sf sub x imm else nzcv) (icnt+1) lim mt
  | .logI sf opc d n mask =>
    let res := mask32 sf (logic opc (mask32 sf (rr rf n)) mask)
    run (wr rf d res) mem sys tr (i+1) tpc (if opc == 3 then logicFlags sf res else nzcv) (icnt+1) lim mt
  | .movk sf d imm pos =>
    run (wr rf d (mask32 sf ((rr rf d &&& ~~~((0xFFFF : UInt64) <<< pos)) ||| (imm <<< pos)))) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .bfm sf opc d n r s wmask tmask =>
    run (wr rf d (bfmVal sf opc (rr rf n) (rr rf d) r s wmask tmask)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .extr sf d n m lsb =>
    run (wr rf d (extrVal sf (rr rf n) (rr rf m) lsb)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .alu sf sub setf d n m sh amt =>
    let x := mask32 sf (rr rf n)
    let y := mask32 sf (shiftReg sf (rr rf m) sh amt)
    run (wr rf d (asRes sf sub x y)) mem sys tr (i+1) tpc (if setf then asFlags sf sub x y else nzcv) (icnt+1) lim mt
  | .aluX sf sub setf d n m opt amt =>
    let x := mask32 sf (rr rf n)
    let y := extendReg (rr rf m) opt amt (if sf then 64 else 32)
    run (wr rf d (asRes sf sub x y)) mem sys tr (i+1) tpc (if setf then asFlags sf sub x y else nzcv) (icnt+1) lim mt
  | .adc sf sub setf d n m =>
    let x := mask32 sf (rr rf n)
    let y := mask32 sf (rr rf m)
    let c := (nzcv >>> 1) &&& 1
    run (wr rf d (adcRes sf sub x y c)) mem sys tr (i+1) tpc (if setf then adcFlags sf sub x y c else nzcv) (icnt+1) lim mt
  | .logR sf opc inv d n m sh amt =>
    let res := logVal sf opc inv (rr rf n) (shiftReg sf (rr rf m) sh amt)
    run (wr rf d res) mem sys tr (i+1) tpc (if opc == 3 then logicFlags sf res else nzcv) (icnt+1) lim mt
  | .ccmp sf sub isImm n m imm cond flags =>
    run rf mem sys tr (i+1) tpc (ccmpFlags sf sub cond flags nzcv (rr rf n) (if isImm then imm else rr rf m)) (icnt+1) lim mt
  | .csel sf inv inc d n m cond =>
    run (wr rf d (cselVal sf inv inc cond nzcv (rr rf n) (rr rf m))) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .dp1 sf opc d n => run (wr rf d (dp1Val sf opc (rr rf n))) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .dp2 sf opc d n m => run (wr rf d (dp2Val sf opc (rr rf n) (rr rf m))) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .madd sf sub d n m a =>
    let p := rr rf n * rr rf m
    run (wr rf d (mask32 sf (if sub then rr rf a - p else rr rf a + p))) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .mull u sub d n m a => run (wr rf d (mullVal u sub (rr rf n) (rr rf m) (rr rf a))) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .mulh u d n m => run (wr rf d (mulHigh (rr rf n) (rr rf m) (!u))) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .mrs _ k t =>
    -- the reads at EL1 with no side effect and nothing to check
    if sys.el != 1 then slowpath! else
    if k == K_SP_EL0 && sys.spsel then run (wr rf t sys.sp0) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
    else if k == K_TPIDR_EL1 then run (wr rf t sys.tpidr1) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
    else if k == K_DAIF then run (wr rf t sys.daif) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
    else if k == K_NZCV then run (wr rf t (nzcv <<< 28)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
    else if k == K_TPIDR_EL0 then run (wr rf t sys.tpidr0) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
    else if k == K_DCZID then run (wr rf t DCZID_BLOCK_LOG2_WORDS) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
    else if k == K_CURRENT_EL then run (wr rf t 4) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
    else slowpath!
  | .nop => run rf mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  | .readCounter virt t =>
    if sys.el == 0 then slowpath! else
    -- a counter poll's whole iterations, fast-forwarded (Cpu/Poll.lean)
    let key := noPollKey pc (rr rf 30)
    match (if i == 0 && pollShape (agu tr 1 .endTrace) && !knownNoPoll sys key then pollSkip sys rf mem pc t virt icnt (lim - icnt) else .notNow) with
    | .skip ticks count flags =>
      let sys := { sys with noPoll := clearNoPoll sys.noPoll key }
      run (wr rf t count) mem sys DISPATCH 0 pc flags (icnt + ticks) lim mt
    | .notPoll =>
      let sys := { sys with noPoll := noteNoPoll sys.noPoll key }
      run (wr rf t (counterAt icnt)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
    | .notNow =>
      run (wr rf t (counterAt icnt)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt
  -- loads and stores of general-purpose registers through the micro-TLBs
  | .ldst size opc wb post kind t n off =>
    let base := rr rf n
    if kind != 0 || spAlignFault sys n base then slowpath! else
    let addr := if post then base else base + off
    let bytes : UInt64 := 1 <<< size.toUInt64
    if opc == 0 then
      storeAt! addr, bytes, (fun o =>
        let mem := stN mem o.toUSize size (rr rf t)
        let rf := if wb then wr rf n (base + off) else rf
        run rf mem sys tr (i+1) tpc nzcv (icnt+1) lim mt)
    else
      loadAt! addr, bytes, (fun o =>
        let rf := wr rf t (extendLoad (ldN mem o.toUSize size) size opc)
        let rf := if wb then wr rf n (base + off) else rf
        run rf mem sys tr (i+1) tpc nzcv (icnt+1) lim mt)
  | .ldrbPost t n off =>
    let base := rr rf n
    if spAlignFault sys n base then slowpath! else
    loadAt! base, 1, (fun o =>
      run (wr (wr rf t (gb mem o.toUSize)) n (base + off)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt)
  | .ldstR size opc t n m opt shift =>
    let base := rr rf n
    if spAlignFault sys n base then slowpath! else
    let addr := base + extendReg (rr rf m) opt shift 64
    let bytes : UInt64 := 1 <<< size.toUInt64
    if opc == 0 then
      storeAt! addr, bytes, (fun o =>
        run rf (stN mem o.toUSize size (rr rf t)) sys tr (i+1) tpc nzcv (icnt+1) lim mt)
    else
      loadAt! addr, bytes, (fun o =>
        run (wr rf t (extendLoad (ldN mem o.toUSize size) size opc)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt)
  | .ldstPair kind load wb post t t2 n off =>
    let base := rr rf n
    if spAlignFault sys n base then slowpath! else
    let addr := if post then base else base + off
    let log2 : UInt8 := if kind == 2 then 3 else 2
    let eb : UInt64 := 1 <<< log2.toUInt64
    -- an element misaligned to its size is one access per element in execOp
    if addr &&& (eb - 1) != 0 then slowpath! else
    if load then
      loadAt! addr, 2 * eb, (fun o =>
        let a := ldN mem o.toUSize log2
        let b2 := ldN mem (o + eb).toUSize log2
        let (a, b2) := if kind == 1 then (sext a 32, sext b2 32) else (a, b2)
        let rf := if log2 == 2 then wr (wr rf t2 b2) t a else wr (wr rf t a) t2 b2
        let rf := if wb then wr rf n (base + off) else rf
        run rf mem sys tr (i+1) tpc nzcv (icnt+1) lim mt)
    else
      storeAt! addr, 2 * eb, (fun o =>
        let mem := stN (stN mem o.toUSize log2 (rr rf t)) (o + eb).toUSize log2 (rr rf t2)
        let rf := if wb then wr rf n (base + off) else rf
        run rf mem sys tr (i+1) tpc nzcv (icnt+1) lim mt)
  | .ldpPost64 t t2 n off =>
    let base := rr rf n
    if spAlignFault sys n base || base &&& 7 != 0 then slowpath! else
    loadAt! base, 16, (fun o =>
      let rf := wr (wr rf t (ld64 mem o.toUSize)) t2 (ld64 mem (o + 8).toUSize)
      run (wr rf n (base + off)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt)
  | .stpPost64 t t2 n off =>
    let base := rr rf n
    if spAlignFault sys n base || base &&& 7 != 0 then slowpath! else
    storeAt! base, 16, (fun o =>
      let mem := st64 (st64 mem o.toUSize (rr rf t)) (o + 8).toUSize (rr rf t2)
      run (wr rf n (base + off)) mem sys tr (i+1) tpc nzcv (icnt+1) lim mt)
  | .endTrace =>
    if icnt >= lim then { rf, mem, sys, pc, nzcv, icnt, why := 0 } else
    -- the common case inline: no interrupt, the same code page, a cached
    -- trace in RAM that fits in the burst
    let pa := mt.cp ||| (pc &&& 0xFFF)
    let idx := ((pa >>> 2) &&& (CACHE.toUInt64 - 1)).toUSize
    let t := agu sys.ctr idx DISPATCH
    if pc >>> 12 == mt.cv && deliverable sys == 0 && !sys.il && agu sys.ctag idx MISS == pa
        && inRam pa 4 && (t.size - 1).toUInt64 <= lim - icnt then
      run rf mem sys t 0 pc nzcv icnt lim mt
    else
    let n := lookup rf mem sys pc nzcv icnt lim mt.cv mt.cp
    if n.cv == MISS then
      run n.rf n.mem n.sys n.tr 0 n.pc n.nzcv n.icnt lim { MicroTlb.empty with cv := n.cv, cp := n.cp }
    else run n.rf n.mem n.sys n.tr 0 n.pc n.nzcv n.icnt lim { mt with cv := n.cv, cp := n.cp }
  | _ => slowpath!

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
  let r := run m.rf m.mem m.sys DISPATCH 0 m.pc m.nzcv m.icnt (m.icnt + burst) MicroTlb.empty
  let sys := if r.why == 2 then afterDeviceAccess r.sys r.icnt else r.sys
  drive { rf := r.rf, mem := r.mem, sys := sys, pc := r.pc, nzcv := r.nzcv, icnt := r.icnt } instrEnd

end Leanbox
