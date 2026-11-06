import Leanbox.Engine.Trace
import Leanbox.Engine.Poll
/-!
# The chained interpreter

One self-tail-recursive `run` (bench/Chain.lean's design), each handler
tail-calling it on the next op of its trace. Time is the tick count `icnt`:
every instruction retires one tick. The hot loop runs to `lim`, the end of
a burst (machine.rs: the next event, or the instruction budget), and never
past it: a trace longer than what is left is run cut short. Interrupts are
taken between traces, which end after anything that may change what an
interrupt sees.
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

/-- The micro-TLBs `run` keeps from op to op: two code pages (`cv*` the VA
page, `cp*` its PA page; the newest first, the one it displaced second), two
read pages and one write page (`dv*` the VA page, `dp*` the RAM offset of the
page). One object, so the loop carries one value for all ten, updated in
place while `run` holds the only reference. Every change of translation
regime or exception level empties all of them together (flow 3). -/
structure MicroTlb where
  cv : UInt64
  cp : UInt64
  dvr : UInt64
  dpr : UInt64
  dvw : UInt64
  dpw : UInt64
  dvr2 : UInt64
  dpr2 : UInt64
  cv2 : UInt64
  cp2 : UInt64

/-- Every entry empty. -/
def MicroTlb.empty : MicroTlb := ⟨MISS, MISS, MISS, MISS, MISS, MISS, MISS, MISS, MISS, MISS⟩

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
end; `mt` the code and data micro-TLBs (`MicroTlb`). Every handler ends in a tail call of `run`, compiled to a jump; the
helpers it calls return. -/
partial def run (rf mem : ByteArray) (sys : Sys) (tr : Array Op) (i : USize)
    (tpc nzcv icnt lim : UInt64) (mt : MicroTlb) : Res :=
  let pc := tpc + 4 * i.toUInt64
  -- Every trace ends in `endTrace`, so `i` is always in range; the index
  -- past the end goes to `DISPATCH`, which is `endTrace` at the same PC.
  -- The op is read in the proven branch and matched there, so it is a
  -- borrow of `tr` (no reference count taken per op).
  if h : i < tr.usize then
  let op := tr[i]
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
    -- a counter poll's whole iterations, fast-forwarded (Engine/Poll.lean)
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
    -- the common case inline: no interrupt, a code page in the micro-TLB, a
    -- cached trace in RAM that fits in the burst
    let vp := pc >>> 12
    let second := vp != mt.cv && vp == mt.cv2
    let cv := if second then mt.cv2 else mt.cv
    let cp := if second then mt.cp2 else mt.cp
    let pa := cp ||| (pc &&& 0xFFF)
    let idx := ((pa >>> 2) &&& (CACHE.toUInt64 - 1)).toUSize
    let t := agu sys.ctr idx DISPATCH
    if vp == cv && deliverable sys == 0 && !sys.il && agu sys.ctag idx MISS == pa
        && inRam pa 4 && (t.size - 1).toUInt64 <= lim - icnt then
      run rf mem sys t 0 pc nzcv icnt lim mt
    else
    let n := lookup rf mem sys pc nzcv icnt lim cv cp
    if n.cv == MISS then run n.rf n.mem n.sys n.tr 0 n.pc n.nzcv n.icnt lim MicroTlb.empty
    else if n.cv == cv then run n.rf n.mem n.sys n.tr 0 n.pc n.nzcv n.icnt lim mt
    else
      run n.rf n.mem n.sys n.tr 0 n.pc n.nzcv n.icnt lim
        { mt with cv := n.cv, cp := n.cp, cv2 := mt.cv, cp2 := mt.cp }
  | _ => slowpath!
  else run rf mem sys DISPATCH 0 pc nzcv icnt lim mt

end Leanbox
