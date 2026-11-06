import Leanbox.Cpu.Step
/-!
# Traces

The hot loop runs traces: decoded runs of up to 32 instructions in one page,
each ending in an `endTrace` op (Bochs's dummy end-of-trace instruction),
cached by physical address. This module builds them and finds the next one
(Bochs cpu/icache.cc, BxEndTrace).
-/
namespace Leanbox

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

end Leanbox
