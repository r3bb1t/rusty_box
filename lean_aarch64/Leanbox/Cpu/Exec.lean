import Leanbox.Cpu.Mmu
/-
The reference executor: every op, every check, every fault, as rusty_box's
per-instruction path runs it (cpu/exec.rs `execute_at`, `finish`, `retire`;
the handlers of cpu/dp.rs, cpu/ldst.rs, cpu/system.rs; cpu/exception.rs;
cpu/mmu.rs's access paths). The hot loop (Run.lean) takes its fast paths only
where they give the same result, and comes here for everything else.
-/
namespace Leanbox

/-- A synchronous exception an instruction raises, or a stop. `ec` 0x100
and above is a stop: 0x100 power off, 0x101 processor off, 0x102 reset,
0x1FF a behaviour this machine does not model (`why`). -/
structure Fault where
  ec : UInt64
  iss : UInt64 := 0
  far : Option UInt64 := none
  /-- The exception returns to the next instruction (SVC, HVC). -/
  next : Bool := false
  why : String := ""

def EC_UNKNOWN : UInt64 := 0x00
def EC_WFX : UInt64 := 0x01
def EC_FP : UInt64 := 0x07
def EC_ILLEGAL : UInt64 := 0x0E
def EC_SVC : UInt64 := 0x15
def EC_SYSREG : UInt64 := 0x18
def EC_IABORT_LOWER : UInt64 := 0x20
def EC_IABORT : UInt64 := 0x21
def EC_PC_ALIGN : UInt64 := 0x22
def EC_DABORT_LOWER : UInt64 := 0x24
def EC_DABORT : UInt64 := 0x25
def EC_SP_ALIGN : UInt64 := 0x26
def EC_BRK : UInt64 := 0x3C
def STOP_POWER_OFF : UInt64 := 0x100
def STOP_CPU_OFF : UInt64 := 0x101
def STOP_RESET : UInt64 := 0x102
def STOP_UNMODELLED : UInt64 := 0x1FF

def undefinedF : Fault := { ec := EC_UNKNOWN }
def unmodelled (why : String) : Fault := { ec := STOP_UNMODELLED, why := why }

/-- A data abort's syndrome (mmu.rs `data_abort`): FAR is the address. -/
def dataAbort (sys : Sys) (va : UInt64) (a : Abort) (cm : Bool := false) : Fault :=
  { ec := if sys.el == 0 then EC_DABORT_LOWER else EC_DABORT
    iss := a.fsc.toUInt64 ||| (if a.write || cm then 0x40 else 0) ||| (if cm then 0x100 else 0)
    far := some va }

def instructionAbort (sys : Sys) (va : UInt64) (a : Abort) : Fault :=
  { ec := if sys.el == 0 then EC_IABORT_LOWER else EC_IABORT, iss := a.fsc.toUInt64, far := some va }

/-- system.rs `fp_trap` to EL1. -/
def fpTrapF : Fault := { ec := EC_FP, iss := ((1 : UInt64) <<< 24) ||| ((0xE : UInt64) <<< 20) }

-- ── PSTATE as SPSR (mod.rs `Pstate::to_spsr`) ─────────────────────────
def toSpsr (sys : Sys) (nzcv : UInt64) : UInt64 :=
  (nzcv <<< 28) ||| (if sys.ss then (1 : UInt64) <<< 21 else 0) ||| (if sys.il then (1 : UInt64) <<< 20 else 0)
  ||| sys.daif ||| (sys.el.toUInt64 <<< 2) ||| (if sys.spsel then 1 else 0)

/-- exception.rs `take_exception` to EL1 with vector `offset` (0 sync,
0x80 IRQ, 0x100 FIQ): the PC the processor goes on at. -/
def takeException (rf : ByteArray) (sys : Sys) (nzcv : UInt64) (offset retAddr : UInt64)
    (esr : Option UInt64) (far : Option UInt64) : ByteArray × Sys × UInt64 :=
  let quadrant : UInt64 := if sys.el == 0 then 0x400 else if sys.spsel then 0x200 else 0
  let spsr := toSpsr sys nzcv
  let sys := { sys with spsr := spsr, elr := retAddr }
  let sys := match esr with | some e => { sys with esr := e } | none => sys
  let sys := match far with | some f => { sys with far := f } | none => sys
  let (rf, sys) := switchSp rf sys 1 true
  let sys := { sys with daif := 0x3C0, ss := false, il := false, monArmed := false, activity := 0 }
  (rf, sys, branchAddr sys ((sys.vbar &&& ~~~0x7FF) + quadrant + offset))

/-- `finish`'s exception arm for the instruction at `pc` that raised `f`:
it retires (one tick), then the exception is taken (exception.rs
`take_sync_exception`). A stop retires nothing. -/
def raise (st : St) (pc : UInt64) (f : Fault) : St :=
  if f.ec >= 0x100 then
    let p : UInt8 := if f.ec == STOP_POWER_OFF then 1 else if f.ec == STOP_CPU_OFF then 2
      else if f.ec == STOP_RESET then 3 else 4
    { st with pc := pc, flow := 2, sys := { st.sys with power := p, stopWhy := f.why } }
  else
  let esr := (f.ec <<< 26) ||| ((1 : UInt64) <<< 25) ||| (f.iss &&& 0x1FF_FFFF)
  let ret := if f.next then pc + 4 else pc
  let (rf, sys, npc) := takeException st.rf st.sys st.nzcv 0 ret (some esr) f.far
  { st with rf := rf, sys := sys, pc := npc, icnt := st.icnt + 1, flow := 3 }

/-- Retire the instruction at `pc`, going on to the next. -/
@[inline] def nx (st : St) (pc : UInt64) : St := { st with pc := pc + 4, icnt := st.icnt + 1 }
/-- Retire a branch to `target` (exec.rs `retire`, `Flow::Jump`). -/
@[inline] def jmp (st : St) (target : UInt64) : St :=
  { st with pc := branchAddr st.sys target, icnt := st.icnt + 1, flow := 1 }

-- ── the trace cache's code marks ──────────────────────────────────────
/-- Forget every trace: a store reached code a trace holds. -/
def dropTraces (sys : Sys) : Sys :=
  { sys with ctag := Array.replicate CACHE MISS, ctr := Array.replicate CACHE #[],
             cmark := zeros (RAM_SIZE.toNat / 128) }

/-- Forget the traces built from flash (a write to a flash bank). -/
def dropFlashTraces (sys : Sys) : Sys := Id.run do
  let mut sys := sys
  let mut i := 0
  while i < CACHE do
    let t := sys.ctag[i]!
    if t != MISS && t < RAM_BASE then
      sys := { sys with ctag := sys.ctag.set! i MISS, ctr := sys.ctr.set! i #[] }
    i := i + 1
  return sys

-- ── the bus (board.rs `PhysBus`) ──────────────────────────────────────
@[inline] def inRam (pa bytes : UInt64) : Bool := pa >= RAM_BASE && pa - RAM_BASE + bytes <= RAM_SIZE

/-- mmu.rs `note_store`: a store of `bytes` at `pa` clears a monitor whose
granule it overlaps, and forgets the traces holding code it overwrites. -/
def noteStore (st : St) (pa bytes : UInt64) : St :=
  let sys := st.sys
  let st := if sys.monArmed && pa < sys.monAddr + sys.monBytes && sys.monAddr < pa + bytes then
      { st with sys := { sys with monArmed := false } } else st
  if inRam pa bytes then
    let o := pa - RAM_BASE
    let l0 := (o >>> 7).toUSize
    let l1 := ((o + bytes - 1) >>> 7).toUSize
    if gb st.sys.cmark l0 != 0 || gb st.sys.cmark l1 != 0 then
      { st with sys := dropTraces st.sys, flow := max st.flow 1 }
    else st
  else st

/-- A read of `1 << log2` bytes at `pa` (bus.rs `read`): `none` is
`BusFault::Unmapped`. -/
def busRead (st : St) (pa : UInt64) (log2 : UInt8) : St × Option UInt64 :=
  let bytes : UInt64 := 1 <<< log2.toUInt64
  if inRam pa bytes then (st, some (ldN st.mem (pa - RAM_BASE).toUSize log2))
  else
    let sys := st.sys
    let board := sys.board
    let sys := { sys with board := default }
    let (board, v) := board.read pa log2 st.icnt
    ({ st with sys := { sys with board := board } }, v)

/-- A write (bus.rs `write`), the value already in the width; `false` is
`BusFault::Unmapped`. -/
def busWrite (st : St) (pa : UInt64) (log2 : UInt8) (v : UInt64) : St × Bool :=
  let bytes : UInt64 := 1 <<< log2.toUInt64
  if inRam pa bytes then
    let st := noteStore st pa bytes
    ({ st with mem := stN st.mem (pa - RAM_BASE).toUSize log2 v }, true)
  else
    let st := noteStore st pa bytes
    let sys := st.sys
    let board := sys.board
    let sys := { sys with board := default }
    let (board, ok) := board.write pa log2 (v &&& valueMask log2) st.icnt
    let sys := { sys with board := board }
    let sys := if pa < FLASH1_BASE + FLASH_BANK_SIZE then dropFlashTraces sys else sys
    ({ st with sys := sys }, ok)

/-- An external abort (mmu.rs `external_abort`) of a data access. -/
def externalAbort (sys : Sys) (va : UInt64) (write : Bool) : Fault :=
  dataAbort sys va ⟨FSC_SYNC_EXTERNAL, write⟩

-- ── data accesses (mmu.rs `read_mem`, `write_mem` and their paths) ────
/-- The access's rules: `unpriv` LDTR/STTR (EL0's permissions at EL1),
`aligned` an exclusive or ordered access, which faults when misaligned
(mmu.rs `AlignmentRule::Atomic`, `Ordered`, without FEAT_LSE2). -/
structure Acc where
  unpriv : Bool := false
  aligned : Bool := false

@[inline] def accEl (sys : Sys) (acc : Acc) : UInt8 := if acc.unpriv then 0 else sys.el

/-- mmu.rs `check_alignment`. -/
def checkAlignment (sys : Sys) (va bytes : UInt64) (acc : Acc) (write : Bool) : Option Fault :=
  if va &&& (bytes - 1) != 0 && (sys.sctlr &&& SCTLR_A != 0 || acc.aligned) then
    some (dataAbort sys va ⟨FSC_ALIGNMENT, write⟩)
  else none

/-- `translate_data_checked` for an access of `bytes` within one page. -/
def dataXlate (st : St) (va bytes : UInt64) (write : Bool) (acc : Acc) (refuse : Bool) :
    St × Except Fault UInt64 :=
  match checkAlignment st.sys va bytes acc write with
  | some f => (st, .error f)
  | none =>
    let (sys, r) := translate st.sys st.mem va st.icnt (if write then AK_WRITE else AK_READ) (accEl st.sys acc) refuse
    let st := { st with sys := sys }
    match r with
    | .ok (pa, _) => (st, .ok pa)
    | .error a => (st, .error (dataAbort st.sys va a))

/-- mmu.rs `read_single`. -/
def readSingle (st : St) (va : UInt64) (log2 : UInt8) (acc : Acc) (refuse : Bool) : St × Except Fault UInt64 :=
  let bytes : UInt64 := 1 <<< log2.toUInt64
  if (va &&& 0xFFF) + bytes <= 0x1000 then
    let (st, r) := dataXlate st va bytes false acc refuse
    match r with
    | .error f => (st, .error f)
    | .ok pa =>
      let (st, v) := busRead st pa log2
      match v with
      | some v => (st, .ok v)
      | none => (st, .error (externalAbort st.sys va false))
  else
    -- read_mem_across_pages: byte by byte, each translated
    match checkAlignment st.sys va bytes acc false with
    | some f => (st, .error f)
    | none =>
    let el := accEl st.sys acc
    let rec go (st : St) (i : Nat) (v : UInt64) : St × Except Fault UInt64 :=
      if i ≥ bytes.toNat then (st, .ok v) else
      let bva := va + i.toUInt64
      let (sys, r) := translate st.sys st.mem bva st.icnt AK_READ el true
      let st := { st with sys := sys }
      match r with
      | .error a => (st, .error (dataAbort st.sys bva a))
      | .ok (pa, _) =>
        let (st, b) := busRead st pa 0
        match b with
        | none => (st, .error (externalAbort st.sys bva false))
        | some b => go st (i + 1) (v ||| (b <<< (8 * i.toUInt64)))
    termination_by bytes.toNat - i
    go st 0 0

/-- mmu.rs `read_mem`. -/
@[inline] def readMem (st : St) (va : UInt64) (log2 : UInt8) (acc : Acc) : St × Except Fault UInt64 :=
  readSingle st va log2 acc (va &&& ((1 <<< log2.toUInt64) - 1) != 0)

/-- mmu.rs `translate_write_pages`: both pages of a write of `bytes` at `va`
that crosses a page boundary, the first page's then the second's: the PA of
`va`, the PA of the second page, and the bytes on the first. -/
def xlatePages (st : St) (va bytes : UInt64) (acc : Acc) (refuse : Bool) :
    St × Except Fault (UInt64 × UInt64 × UInt64) :=
  let el := accEl st.sys acc
  let second := (va + bytes - 1) &&& ~~~0xFFF
  let (sys, r1) := translate st.sys st.mem va st.icnt AK_WRITE el refuse
  let st := { st with sys := sys }
  match r1 with
  | .error a => (st, .error (dataAbort st.sys va a))
  | .ok (pa1, _) =>
  let (sys, r2) := translate st.sys st.mem second st.icnt AK_WRITE el refuse
  let st := { st with sys := sys }
  match r2 with
  | .error a => (st, .error (dataAbort st.sys second a))
  | .ok (pa2, _) => (st, .ok (pa1, pa2, second - va))

/-- mmu.rs `write_element_on_pages`: `bytes` of `v` at offset `off` of a
write translated by `xlatePages`, whole where they lie on one page, else
byte by byte. -/
def writeOnPages (st : St) (va : UInt64) (pages : UInt64 × UInt64 × UInt64) (off bytes v : UInt64) :
    St × Option Fault :=
  let (pa1, pa2, firstBytes) := pages
  let pa (o : UInt64) : UInt64 := if o < firstBytes then pa1 + o else pa2 + (o - firstBytes)
  if off + bytes <= firstBytes || off >= firstBytes then
    let log2 : UInt8 := if bytes == 8 then 3 else if bytes == 4 then 2 else if bytes == 2 then 1 else 0
    let (st, ok) := busWrite st (pa off) log2 v
    if ok then (st, none) else (st, some (externalAbort st.sys (va + off) true))
  else
    let rec go (st : St) (i : Nat) : St × Option Fault :=
      if i ≥ bytes.toNat then (st, none) else
      let at_ := off + i.toUInt64
      let (st, ok) := busWrite st (pa at_) 0 ((v >>> (8 * i.toUInt64)) &&& 0xFF)
      if !ok then (st, some (externalAbort st.sys (va + at_) true)) else go st (i + 1)
    termination_by bytes.toNat - i
    go st 0

/-- mmu.rs `write_single`. -/
def writeSingle (st : St) (va : UInt64) (log2 : UInt8) (v : UInt64) (acc : Acc) (refuse : Bool) :
    St × Option Fault :=
  let bytes : UInt64 := 1 <<< log2.toUInt64
  if (va &&& 0xFFF) + bytes <= 0x1000 then
    let (st, r) := dataXlate st va bytes true acc refuse
    match r with
    | .error f => (st, some f)
    | .ok pa =>
      let (st, ok) := busWrite st pa log2 v
      if ok then (st, none) else (st, some (externalAbort st.sys va true))
  else
    -- write_mem_across_pages: both pages translated, then the bytes written
    match checkAlignment st.sys va bytes acc true with
    | some f => (st, some f)
    | none =>
    let (st, r) := xlatePages st va bytes acc true
    match r with
    | .error f => (st, some f)
    | .ok pages => writeOnPages st va pages 0 bytes v

@[inline] def writeMem (st : St) (va : UInt64) (log2 : UInt8) (v : UInt64) (acc : Acc) : St × Option Fault :=
  writeSingle st va log2 v acc (va &&& ((1 <<< log2.toUInt64) - 1) != 0)

/-- mmu.rs `read_pair` (and `read_mem_u128` with `log2` 3 after its
alignment check): two elements, one translation within a page, each on its
own across one. -/
def readPair (st : St) (va : UInt64) (log2 : UInt8) (acc : Acc) (quad : Bool := false) :
    St × Except Fault (UInt64 × UInt64) :=
  let bytes : UInt64 := 1 <<< log2.toUInt64
  -- `OnDevice::of_alignment`: by the element, or by all 16 bytes of a Q access
  let refuse := va &&& ((if quad then 2 * bytes else bytes) - 1) != 0
  if (va &&& 0xFFF) + 2 * bytes <= 0x1000 && !acc.aligned then
    let (st, r) := dataXlate st va (2 * bytes) false acc refuse
    match r with
    | .error f => (st, .error f)
    | .ok pa =>
      let (st, a) := busRead st pa log2
      match a with
      | none => (st, .error (externalAbort st.sys va false))
      | some a =>
      let (st, b) := busRead st (pa + bytes) log2
      match b with
      | none => (st, .error (externalAbort st.sys (va + bytes) false))
      | some b => (st, .ok (a, b))
  else
    let (st, a) := readSingle st va log2 acc refuse
    match a with
    | .error f => (st, .error f)
    | .ok a =>
    let (st, b) := readSingle st (va + bytes) log2 acc refuse
    match b with
    | .error f => (st, .error f)
    | .ok b => (st, .ok (a, b))

/-- mmu.rs `write_pair` (and `write_mem_u128`). -/
def writePair (st : St) (va : UInt64) (log2 : UInt8) (a b : UInt64) (acc : Acc) (quad : Bool := false) :
    St × Option Fault :=
  let bytes : UInt64 := 1 <<< log2.toUInt64
  let refuse := va &&& ((if quad then 2 * bytes else bytes) - 1) != 0
  if (va &&& 0xFFF) + 2 * bytes <= 0x1000 && !acc.aligned then
    let (st, r) := dataXlate st va (2 * bytes) true acc refuse
    match r with
    | .error f => (st, some f)
    | .ok pa =>
      let (st, ok) := busWrite st pa log2 a
      if !ok then (st, some (externalAbort st.sys va true)) else
      let (st, ok) := busWrite st (pa + bytes) log2 b
      if !ok then (st, some (externalAbort st.sys (va + bytes) true)) else (st, none)
  else
    let (st, f) := writeSingle st va log2 a acc refuse
    match f with
    | some f => (st, some f)
    | none => writeSingle st (va + bytes) log2 b acc refuse

-- ── data processing helpers ───────────────────────────────────────────
/-- Arm `ExtendReg(reg, type, shift, N)`. -/
def extendReg (x : UInt64) (opt : UInt8) (shift : UInt64) (n : UInt64) : UInt64 :=
  let unsigned := opt &&& 4 == 0
  let len0 : UInt64 := 8 <<< (opt &&& 3).toUInt64
  let len := min len0 (n - shift)
  let v := x &&& ones len
  let v := if unsigned then v else sext v len
  mask32 (n == 64) (v <<< shift)

/-- mmu.rs `check_sp_alignment` for a base register `n`. -/
@[inline] def spAlignFault (sys : Sys) (n : UInt8) (base : UInt64) : Bool :=
  n == SP && base &&& 0xF != 0 &&
    (if sys.el == 0 then sys.sctlr &&& SCTLR_SA0 != 0 else sys.sctlr &&& SCTLR_SA != 0)

def spAlignF : Fault := { ec := EC_SP_ALIGN }

/-- CRC-32 or CRC-32C of `bytes` bytes of `data` into `acc`, bit-reflected. -/
def crc32 (acc data bytes : UInt64) (castagnoli : Bool) : UInt64 := Id.run do
  let poly : UInt64 := if castagnoli then 0x82F6_3B78 else 0xEDB8_8320
  let mut c := acc &&& 0xFFFF_FFFF
  for i in [0:(8 * bytes).toNat] do
    let bit := (data >>> i.toUInt64) &&& 1
    let lsb := (c ^^^ bit) &&& 1
    c := c >>> 1
    if lsb == 1 then c := c ^^^ poly
  return c

/-- The high 64 bits of a 128-bit product. -/
def mulHigh (a b : UInt64) (signed : Bool) : UInt64 :=
  if signed then
    let p : Int := a.toInt64.toInt * b.toInt64.toInt
    (p >>> 64).toInt64.toUInt64
  else ((a.toNat * b.toNat) >>> 64).toUInt64

@[inline] def setR (st : St) (d : UInt8) (v : UInt64) : St := { st with rf := wr st.rf d v }

/-- The base register of a load or store (ldst.rs `base_register`). -/
@[inline] def baseReg (st : St) (n : UInt8) : Except Fault UInt64 :=
  let b := rr st.rf n
  if spAlignFault st.sys n b then .error spAlignF else .ok b

/-- Write a whole SIMD&FP register. -/
@[inline] def setV (st : St) (t : UInt8) (lo hi : UInt64) : St := { st with rf := wv st.rf t lo hi }

-- ── the exception return (exception.rs `exception_return`) ────────────
def exceptionReturn (st : St) : St :=
  let sys := st.sys
  let spsr := sys.spsr
  let target := sys.elr
  let m := spsr &&& 0x1F
  let tel := (m >>> 2) &&& 3
  let legal := m &&& 0x10 == 0 && m &&& 2 == 0 && tel <= sys.el.toUInt64 && tel != 2 && tel != 3
    && !(tel == 0 && m &&& 1 == 1)
  let nzcv := (spsr >>> 28) &&& 0xF
  let sys := { sys with monArmed := false, ss := false, daif := spsr &&& 0x3C0 }
  let (rf, sys) :=
    if legal then
      let (rf, sys) := switchSp st.rf sys tel.toUInt8 (m &&& 1 == 1)
      (rf, { sys with il := spsr &&& ((1 : UInt64) <<< 20) != 0 })
    else (st.rf, { sys with il := true })
  let sys := { sys with eventSet := true }
  { st with rf := rf, sys := sys, nzcv := nzcv, pc := branchAddr sys target, icnt := st.icnt + 1, flow := 3 }

-- ── PSCI (system.rs `psci_call`) ──────────────────────────────────────
def PSCI_SUCCESS : UInt64 := 0
def PSCI_NOT_SUPPORTED : UInt64 := MISS
def PSCI_INVALID_PARAMETERS : UInt64 := MISS - 1
def PSCI_ALREADY_ON : UInt64 := MISS - 3
def PSCI_INVALID_ADDRESS : UInt64 := MISS - 8
def PSCI_AFFINITY : UInt64 := MPIDR_EL1 &&& 0xFF_00FF_FFFF

def psciSupported (id : UInt64) : Bool :=
  id == 0x8400_0000 || id == 0x8400_000A || id == 0x8400_0001 || id == 0xC400_0001
  || id == 0x8400_0002 || id == 0x8400_0003 || id == 0xC400_0003 || id == 0x8400_0004
  || id == 0xC400_0004 || id == 0x8400_0006 || id == 0x8400_0008 || id == 0x8400_0009

def psciAffinityInfo (target level : UInt64) : UInt64 :=
  let ignored : Option UInt64 := match level with
    | 0 => some 0 | 1 => some 0xFF | 2 => some 0xFFFF | 3 => some 0xFF_FFFF | _ => none
  match ignored with
  | none => PSCI_INVALID_PARAMETERS
  | some ig => if target &&& ~~~ig == PSCI_AFFINITY &&& ~~~ig then 0 else PSCI_INVALID_PARAMETERS

def psciCall (st : St) (pc : UInt64) : St :=
  let rf := st.rf
  let fn_ := rr rf 0 &&& 0xFFFF_FFFF
  let smc64 := fn_ &&& 0x4000_0000 != 0
  let a1 := if smc64 then rr rf 1 else rr rf 1 &&& 0xFFFF_FFFF
  let a2 := if smc64 then rr rf 2 else rr rf 2 &&& 0xFFFF_FFFF
  let set (v : UInt64) : St := nx { st with rf := wr rf 0 v } pc
  if fn_ == 0x8400_0000 then set 0x0001_0001
  else if fn_ == 0x8400_000A then set (if psciSupported (a1 &&& 0xFFFF_FFFF) then 0 else PSCI_NOT_SUPPORTED)
  else if fn_ == 0x8400_0001 || fn_ == 0xC400_0001 then
    if a1 &&& 0xFFFE_0000 != 0 then set PSCI_INVALID_PARAMETERS
    else
      -- entered as standby: wait for an interrupt (`begin_wait`)
      let st := nx { st with rf := wr rf 0 0 } pc
      { st with sys := { st.sys with activity := 1 }, flow := 2 }
  else if fn_ == 0x8400_0002 then raise st pc { ec := STOP_CPU_OFF }
  else if fn_ == 0x8400_0003 || fn_ == 0xC400_0003 then
    set (if a1 != PSCI_AFFINITY then PSCI_INVALID_PARAMETERS
         else if a2 &&& 3 != 0 then PSCI_INVALID_ADDRESS else PSCI_ALREADY_ON)
  else if fn_ == 0x8400_0004 || fn_ == 0xC400_0004 then set (psciAffinityInfo a1 (a2 &&& 0xFFFF_FFFF))
  else if fn_ == 0x8400_0006 then set 2
  else if fn_ == 0x8400_0008 then raise st pc { ec := STOP_POWER_OFF }
  else if fn_ == 0x8400_0009 then raise st pc { ec := STOP_RESET }
  else set PSCI_NOT_SUPPORTED

-- ── system instructions (system.rs) ───────────────────────────────────
/-- `ESR_ELx.ISS` of a trapped MSR/MRS/SYS (sysreg.rs `trap_iss`). -/
def trapIss (w : UInt32) (read : Bool) : UInt64 :=
  let k := (w >>> 5) &&& 0xFFFF
  let op0 := (k >>> 14).toUInt64
  let op1 := ((k >>> 11) &&& 7).toUInt64
  let crn := ((k >>> 7) &&& 15).toUInt64
  let crm := ((k >>> 3) &&& 15).toUInt64
  let op2 := (k &&& 7).toUInt64
  let rt := (w &&& 31).toUInt64
  (op0 <<< 20) ||| (op2 <<< 17) ||| (op1 <<< 14) ||| (crn <<< 10) ||| (rt <<< 5) ||| (crm <<< 1)
  ||| (if read then 1 else 0)

def sysFault (w : UInt32) (read : Bool) (e : SysErr) : Fault :=
  match e with
  | .undefined => undefinedF
  | .trap => { ec := EC_SYSREG, iss := trapIss w read }
  | .fpTrap => fpTrapF

/-- A cache maintenance by VA's translation (mmu.rs
`maintenance_translation`): `need` is the access kind it checks (none: 3). -/
def maintenance (st : St) (va : UInt64) (need : UInt8) : St × Option Fault :=
  let sys := st.sys
  if !mmuOn sys then
    let upper := (va >>> 55) &&& 1 == 1
    let addr := if tbi sys upper then va &&& 0x00FF_FFFF_FFFF_FFFF else va
    if addr >>> PA_BITS != 0 then (st, some (dataAbort sys va ⟨fscAddressSize 0, true⟩ true)) else (st, none)
  else
    let (sys, r) := tlbEntry sys st.mem va st.icnt false
    let st := { st with sys := sys }
    match r with
    | .error f => (st, some (dataAbort sys va ⟨f, true⟩ true))
    | .ok e =>
      if need != 3 && e &&& neededPerm need sys.el == 0 then
        (st, some (dataAbort sys va ⟨fscPermission ((e >>> ATTR_LEVEL_SHIFT) &&& 3), true⟩ true))
      else (st, none)

/-- mmu.rs `dc_zva`: 64 bytes of zeros. -/
def dcZva (st : St) (va : UInt64) : St × Option Fault :=
  let base := va &&& ~~~63
  let (sys, r) := translate st.sys st.mem base st.icnt AK_WRITE st.sys.el true
  let st := { st with sys := sys }
  match r with
  | .error a => (st, some (dataAbort st.sys va a))
  | .ok (pa, _) =>
    let st := noteStore st pa 64
    let rec go (st : St) (i : Nat) : St × Option Fault :=
      if i ≥ 8 then (st, none) else
      let at_ := pa + 8 * i.toUInt64
      if inRam at_ 8 then go { st with mem := st64 st.mem (at_ - RAM_BASE).toUSize 0 } (i + 1)
      else
        let sys := st.sys
        let board := sys.board
        let sys := { sys with board := default }
        let (board, ok) := board.write at_ 3 0 st.icnt
        let st := { st with sys := { sys with board := board } }
        if ok then go st (i + 1) else (st, some (externalAbort st.sys va true))
    termination_by 8 - i
    go st 0

/-- system.rs `exec_system`, `exec_hint`, `exec_exception_gen`, ERET. -/
def execSys (st : St) (pc : UInt64) (w : UInt32) : St :=
  let sys := st.sys
  let el0 := sys.el == 0
  let rt := (w &&& 31).toUInt8
  let xt := if rt == 31 then 0 else rr st.rf rt
  if w &&& 0xFF00_0000 == 0xD400_0000 then                  -- exception generation
    let opc := (w >>> 21) &&& 7
    let ll := w &&& 3
    let imm := ((w >>> 5) &&& 0xFFFF).toUInt64
    if opc == 0 && ll == 1 then raise st pc { ec := EC_SVC, iss := imm, next := true }
    else if opc == 0 && ll == 2 then (if el0 then raise st pc undefinedF else psciCall st pc)
    else if opc == 0 && ll == 3 then raise st pc undefinedF   -- SMC: the conduit is HVC
    else raise st pc { ec := EC_BRK, iss := imm }
  else if w &&& 0xFFFF_FFFF == 0xD69F_03E0 then              -- ERET
    if el0 then raise st pc undefinedF else exceptionReturn st
  else
  let l := (w >>> 21) &&& 1
  let op0 := (w >>> 19) &&& 3
  let op1 := (w >>> 16) &&& 7
  let crn := (w >>> 12) &&& 15
  let crm := (w >>> 8) &&& 15
  let op2 := (w >>> 5) &&& 7
  if l == 0 && op0 == 0 then
    if crn == 2 then                                         -- WFE WFI SEV SEVL
      if op2 == 2 then
        let (sys, ev) := takeEvent sys st.icnt
        let st := { st with sys := sys }
        if ev then nx st pc
        else if el0 && sys.sctlr &&& SCTLR_NTWE == 0 then raise st pc { ec := EC_WFX, iss := ((1 : UInt64) <<< 24) ||| ((0xE : UInt64) <<< 20) ||| 1 }
        else if sys.monArmed then nx st pc
        else { nx st pc with sys := { sys with activity := 2 }, flow := 2 }
      else if op2 == 3 then
        if sys.irqLine || sys.fiqLine then nx st pc
        else if el0 && sys.sctlr &&& SCTLR_NTWI == 0 then raise st pc { ec := EC_WFX, iss := ((1 : UInt64) <<< 24) ||| ((0xE : UInt64) <<< 20) }
        else { nx st pc with sys := { sys with activity := 1 }, flow := 2 }
      else nx { st with sys := { sys with eventSet := true } } pc
    else if crn == 3 then                                    -- CLREX
      nx { st with sys := { sys with monArmed := false } } pc
    else                                                     -- MSR (immediate)
      let operand := crm &&& 1 != 0
      if op1 == 0 && op2 == 5 then
        if el0 then raise st pc undefinedF else
        let (rf, sys) := switchSp st.rf sys sys.el operand
        { nx { st with rf := rf, sys := sys } pc with flow := 3 }
      else if op1 == 3 && (op2 == 6 || op2 == 7) then
        if el0 && sys.sctlr &&& SCTLR_UMA == 0 then raise st pc { ec := EC_SYSREG, iss := trapIss w false }
        else
          let bits := crm.toUInt64 <<< 6
          let d := if op2 == 6 then sys.daif ||| bits else sys.daif &&& ~~~bits
          { nx { st with sys := { sys with daif := d } } pc with flow := 1 }
      else raise st pc undefinedF
  else if op0 == 1 then
    if l == 1 then raise st pc undefinedF                    -- SYSL
    else if crn == 11 || crn == 15 then raise st pc undefinedF   -- IMPLEMENTATION DEFINED space
    else if op1 == 0 && crn == 7 && (crm == 1 || crm == 5) && op2 == 0 then   -- IC IALLUIS, IALLU
      if el0 then raise st pc undefinedF else nx st pc
    else if op1 == 3 && crn == 7 && (crm == 5 || crm == 10 || crm == 11 || crm == 14) && op2 == 1 then
      -- IC IVAU, DC CVAC, DC CVAU, DC CIVAC
      if el0 && sys.sctlr &&& SCTLR_UCI == 0 then raise st pc { ec := EC_SYSREG, iss := trapIss w false } else
      let need : UInt8 := if crm == 5 then 3 else if sys.el == 0 then AK_READ else 3
      let (st, f) := maintenance st xt need
      match f with | some f => raise st pc f | none => nx st pc
    else if op1 == 3 && crn == 7 && (crm == 12 || crm == 13) && op2 == 1 then raise st pc undefinedF  -- DC CVAP, CVADP
    else if op1 == 3 && crn == 7 && crm == 3 then raise st pc undefinedF   -- prediction restrictions
    else if op1 == 3 && crn == 7 && crm == 4 && op2 == 1 then   -- DC ZVA
      if el0 && sys.sctlr &&& SCTLR_DZE == 0 then raise st pc { ec := EC_SYSREG, iss := trapIss w false } else
      let (st, f) := dcZva st xt
      match f with | some f => raise st pc f | none => nx st pc
    else if op1 == 0 && crn == 7 && crm == 6 && op2 == 1 then   -- DC IVAC
      if el0 then raise st pc undefinedF else
      let (st, f) := maintenance st xt AK_WRITE
      match f with | some f => raise st pc f | none => nx st pc
    else if op1 == 0 && crn == 7 && (crm == 6 || crm == 10 || crm == 14) && op2 == 2 then   -- DC ISW CSW CISW
      if el0 then raise st pc undefinedF else nx st pc
    else if op1 == 0 && crn == 7 && ((crm == 8 && op2 <= 3) || (crm == 9 && op2 <= 1)) then
      if el0 || crm == 9 then raise st pc undefinedF
      else raise st pc (unmodelled s!"AT at {hex pc}")
    else if op1 == 0 && crn == 8 then                        -- TLBI (EL1)
      let ok := (crm == 3 || crm == 7) && (op2 <= 3 || op2 == 5 || op2 == 7)
      if el0 || !ok then raise st pc undefinedF
      else { nx { st with sys := flushTlb sys } pc with flow := 3 }
    else raise st pc undefinedF
  else
    -- MSR / MRS (register)
    let k := (w >>> 5) &&& 0xFFFF
    if l == 1 then
      match sysregRead sys st.rf st.nzcv st.icnt k with
      | .ok (sys, v) =>
        let v := if k == K_SP_EL0 && !sys.spsel then v else v
        nx { st with sys := sys, rf := if rt == 31 then st.rf else wr st.rf rt v } pc
      | .error e => raise st pc (sysFault w true e)
    else
      match sysregWrite sys st.nzcv st.icnt k xt with
      | .ok (sys, nzcv, eff) =>
        let sys := if eff == 1 then { sys with timerDirty := true }
          else if eff == 2 then flushTlb sys else sys
        let st := nx { st with sys := sys, nzcv := nzcv } pc
        if eff == 3 && debugArmed st.sys then raise st pc (unmodelled s!"debug controls armed at {hex pc}")
        -- a timer write goes back to the run loop; a change of the translation
        -- context empties the micro-TLBs; anything else ends the trace
        else { st with flow := if eff == 1 then 2 else if eff == 2 then 3 else 1 }
      | .error e => raise st pc (sysFault w false e)

end Leanbox
