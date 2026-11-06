import Leanbox.Cpu.Exception
/-!
# Data accesses

The data accesses of loads and stores (mmu.rs `read_mem`, `write_mem` and
their paths: alignment, translation, an access that crosses a page, pairs),
the physical bus they reach (board.rs `PhysBus`), and the trace cache's code
marks a store to RAM checks.
-/
namespace Leanbox

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

end Leanbox
