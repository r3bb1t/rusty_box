import Leanbox.Cpu.Step
/-!
# Counter polls

Exact fast-forward of counter polls (cpu/poll.rs): a loop that only waits
for the system counter to pass a value is moved straight to the end of its
last iteration that goes the same way as the current one, leaving exactly
the state running every instruction would leave.

The rule is poll.rs's. One iteration runs from the counter read (`MRS Xt,
CNTPCT_EL0` or `CNTVCT_EL0`) back to it, within 32 instructions, each
fetched from a TLB hit that permits the fetch (or with stage 1 off) out of
RAM or a flash bank reading its array, and each one of: a NOP; the one
64-bit compare of Xt with another register or an immediate; after it, a
B.cond on an ordering (HS LO HI LS GE LT GT LE); B, BL; BR, BLR, RET
through a register other than Xt. It writes nothing but x30 (which must end
as it started), Xt and NZCV, reads no memory and raises nothing, so its path
is a function of the count it reads alone, and each ordering condition, as
the count grows, changes its outcome at most once: when the iteration
starting at the count of iteration 0 and the one at iteration k take the
same path, every iteration between does. The count is monotone in the tick
count, and stays below 2^62, where signed and unsigned orderings agree.
Skipped iterations fit inside the current burst, before which nothing
outside the processor changes, so an interrupt is taken at the same
instruction either way.

poll.rs stops a skip some iterations short to refill the PC history its
snapshots read; this machine keeps no such history, and the state after a
skip is the same either way.
-/
namespace Leanbox

/-- One iteration as the walk found it (poll.rs `PollCycle`). -/
structure Cycle where
  len : UInt64
  jumps : UInt64
  conds : UInt64
  outcomes : UInt64
  flags : UInt64

@[inline] def Cycle.samePath (a b : Cycle) : Bool :=
  a.len == b.len && a.jumps == b.jumps && a.conds == b.conds && a.outcomes == b.outcomes

/-- poll.rs `poll_op` without exits: the op at `pc` when its fetch is a
permitted TLB hit (or stage 1 is off) from memory whose fetches are stable
(board.rs `fetch_source`). -/
def pollOp (sys : Sys) (mem : ByteArray) (pc : UInt64) : Option Op :=
  if pc &&& 3 != 0 then none else
  let pa? : Option UInt64 :=
    if !mmuOn sys then
      let upper := (pc >>> 55) &&& 1 == 1
      let a := if tbi sys upper then pc &&& 0x00FF_FFFF_FFFF_FFFF else pc
      if a >>> PA_BITS != 0 then none else some a
    else
      let vp := pc >>> 12
      let i := ((vp &&& (TLB_SETS.toUInt64 - 1)) * 2).toUSize
      if agu sys.tlb i MISS != vp then none else
      let e := agu sys.tlb (i + 1) 0
      if e &&& neededPerm AK_FETCH sys.el == 0 then none
      else some ((e &&& ~~~0xFFF) ||| (pc &&& 0xFFF))
  match pa? with
  | none => none
  | some pa =>
    if inRam pa 4 then some (decode (ld32 mem (pa - RAM_BASE).toUSize).toUInt32)
    else if sys.board.flashReadsArray pa then
      let b := sys.board
      let w := if pa < FLASH1_BASE then ld32 b.code (pa - FLASH0_BASE).toUSize
        else ld32 b.vars (pa - FLASH1_BASE).toUSize
      some (decode w.toUInt32)
    else none

/-- poll.rs `ordering`. -/
@[inline] def ordering (cond : UInt8) : Bool := cond == 2 || cond == 3 || (cond >= 8 && cond <= 13)

/-- Why a walk found no iteration: the code is no poll (`static`), or a
fetch the walk needs is not yet a TLB hit, or x30 does not repeat yet
(`transient`). -/
inductive NoCycle where
  | static
  | transient

/-- poll.rs `walk_poll`: one iteration from the read at `start` of Xt
(slot `t`, `virt` the virtual count) given the count `value`. -/
def walkPoll (sys : Sys) (rf mem : ByteArray) (start : UInt64) (t : UInt8) (virt : Bool)
    (value : UInt64) : Except NoCycle Cycle :=
  match pollOp sys mem start with
  | none => .error .transient
  | some (.readCounter v t') => if v != virt || t' != t then .error .static else
    let x30₀ := rr rf 30
    let reg (n : UInt8) (x30 : UInt64) : UInt64 :=
      if n == t then value else if n == 30 then x30 else rr rf n
    let rec go (pc x30 len jumps conds outcomes : UInt64) (cmp : Option UInt64) (fuel : Nat) :
        Except NoCycle Cycle :=
      match fuel with
      | 0 => .error .static
      | fuel + 1 =>
      if len == 32 then .error .static else
      match pollOp sys mem pc with
      | none => .error .transient
      | some op =>
      let len := len + 1
      let step (next x30 jumps conds outcomes : UInt64) (cmp : Option UInt64) : Except NoCycle Cycle :=
        if next == start then
          match cmp with
          | some flags =>
            if jumps == 0 then .error .static
            else if x30 != x30₀ then .error .transient
            else .ok { len, jumps, conds, outcomes, flags }
          | none => .error .static
        else go next x30 len jumps conds outcomes cmp fuel
      match op with
      | .nop => step (pc + 4) x30 jumps conds outcomes cmp
      | .alu true true true d n m 0 0 =>
        if d != SINK || cmp.isSome || ((n == t) == (m == t)) then .error .static
        else step (pc + 4) x30 jumps conds outcomes (some (awcFlags true (reg n x30) (~~~(reg m x30)) 1))
      | .subsI64 d n imm =>
        if d != SINK || cmp.isSome || n != t then .error .static
        else step (pc + 4) x30 jumps conds outcomes (some (awcFlags true value (~~~imm) 1))
      | .bcond cond off =>
        if !ordering cond then .error .static else
        match cmp with
        | none => .error .static
        | some flags =>
          let taken := conditionHolds cond flags
          let outcomes := outcomes ||| ((if taken then 1 else 0) <<< conds)
          if taken then step (branchAddr sys (pc + off)) x30 (jumps + 1) (conds + 1) outcomes cmp
          else step (pc + 4) x30 jumps (conds + 1) outcomes cmp
      | .b link off =>
        let x30 := if link then pc + 4 else x30
        step (branchAddr sys (pc + off)) x30 (jumps + 1) conds outcomes cmp
      | .br kind n =>
        if n == t then .error .static else
        let target := reg n x30
        let x30 := if kind == 1 then pc + 4 else x30
        step (branchAddr sys target) x30 (jumps + 1) conds outcomes cmp
      | _ => .error .static
    go (start + 4) x30₀ 1 0 0 0 none 40
  | some _ => .error .static

/-- Whether `op` is of a shape a poll's iteration may hold (poll.rs
`PollOp::of`): the cheap test of the op after a read, before a walk. -/
@[inline] def pollShape (op : Op) : Bool :=
  match op with
  | .nop | .subsI64 .. | .bcond .. | .b .. | .br .. | .endTrace => true
  | .alu true true true _ _ _ 0 0 => true
  | _ => false

/-- What `pollSkip` found. -/
inductive PollFound where
  /-- Skip this many ticks; the count the last skipped iteration read, and
  the flags it left. -/
  | skip (ticks count flags : UInt64)
  /-- No iteration to skip now. -/
  | notNow
  /-- The read starts no poll: its walk failed. -/
  | notPoll

/-- poll.rs `fast_forward_poll` at the read at `pc` into slot `t`, with
`left` ticks before the burst's end. -/
def pollSkip (sys : Sys) (rf mem : ByteArray) (pc : UInt64) (t : UInt8) (virt : Bool)
    (icnt left : UInt64) : PollFound :=
  if t >= 30 || left < 4 then .notNow else
  match walkPoll sys rf mem pc t virt (counterAt icnt) with
  | .error .static => .notPoll
  | .error .transient => .notNow
  | .ok cycle =>
    let len := cycle.len
    let fit := left / len
    if fit < 2 then .notNow else
    let first := counterAt icnt
    let last := counterAt (icnt + fit * len)
    if first > last || last >= ((1 : UInt64) <<< 62) then .notNow else
    let same (i : UInt64) : Bool :=
      match walkPoll sys rf mem pc t virt (counterAt (icnt + i * len)) with
      | .ok c => c.samePath cycle
      | .error _ => false
    -- the first iteration in 1..=fit that goes another way, or fit
    let firstOther :=
      if same fit then fit
      else Id.run do
        let mut lo : UInt64 := 1
        let mut hi : UInt64 := fit
        while lo < hi do
          let mid := lo + (hi - lo) / 2
          if same mid then lo := mid + 1 else hi := mid
        return lo
    let skip := firstOther
    if skip == 0 then .notNow else
    match walkPoll sys rf mem pc t virt (counterAt (icnt + (skip - 1) * len)) with
    | .error _ => .notNow
    | .ok lastC => .skip (skip * len) (counterAt (icnt + (skip - 1) * len)) lastC.flags

/-- Entries of the table of reads whose walk failed. -/
def NO_POLL : Nat := 256
/-- The table's key for the read at `pc` with x30 holding `x30`: a walk
depends on both (a read in a shared function returns through x30). -/
@[inline] def noPollKey (pc x30 : UInt64) : UInt64 := pc ^^^ (x30 * 0x9E37_79B9_7F4A_7C15)
@[inline] def noPollIdx (key : UInt64) : USize := (((key >>> 2) &&& (NO_POLL.toUInt64 - 1)) * 2).toUSize
/-- Whether the read with this key failed its walk the last eight times
(only a speed matter: a poll's walk also fails once its deadline has passed,
so a read is given up on only after failing repeatedly). -/
@[inline] def knownNoPoll (sys : Sys) (key : UInt64) : Bool :=
  let i := noPollIdx key
  agu sys.noPoll i MISS == key && agu sys.noPoll (i + 1) 0 >= 8
/-- Count one more failed walk for the key. -/
def noteNoPoll (a : Array UInt64) (key : UInt64) : Array UInt64 :=
  let i := noPollIdx key
  if agu a i MISS == key then asu a (i + 1) (agu a (i + 1) 0 + 1)
  else asu (asu a i key) (i + 1) 1
/-- Forget the key's failures: its walk succeeded. -/
def clearNoPoll (a : Array UInt64) (key : UInt64) : Array UInt64 :=
  let i := noPollIdx key
  if agu a i MISS == key then asu a (i + 1) 0 else a

end Leanbox
