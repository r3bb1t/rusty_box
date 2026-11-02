import Leanbox.Cpu.Sysreg
/-
Stage 1 translation of the EL1&0 regime (cpu/mmu.rs, cpu/regime.rs,
cpu/memattr.rs) for a Cortex-A57 without EL2: the 4 KiB granule alone (a
TGx naming another reads as 4 KiB, regime.rs `Granule::decoded`), 44-bit
physical addresses, no FEAT_HPDS, PAN, LPA, LVA, TTST or E0PD.
-/
namespace Leanbox

/-- What a translation refuses an access with: the fault status code
(mmu.rs `Fsc::code`) and whether the access was a write (`ISS.WnR`). -/
structure Abort where
  fsc : UInt32
  write : Bool

def FSC_ALIGNMENT : UInt32 := 0x21
def FSC_SYNC_EXTERNAL : UInt32 := 0x10
@[inline] def fscAddressSize (level : UInt64) : UInt32 := level.toUInt32
@[inline] def fscTranslation (level : UInt64) : UInt32 := 4 ||| level.toUInt32
@[inline] def fscAccessFlag (level : UInt64) : UInt32 := 8 ||| level.toUInt32
@[inline] def fscPermission (level : UInt64) : UInt32 := 12 ||| level.toUInt32
@[inline] def fscExternalOnWalk (level : UInt64) : UInt32 := 0x14 ||| level.toUInt32

/-- An access's kind: 0 read, 1 write, 2 fetch. -/
def AK_READ : UInt8 := 0
def AK_WRITE : UInt8 := 1
def AK_FETCH : UInt8 := 2

/-- regime.rs `el10_tbi` (no FEAT_PAuth, so TBID takes no part). -/
@[inline] def tbi (sys : Sys) (upper : Bool) : Bool :=
  (sys.tcr >>> (if upper then 38 else 37)) &&& 1 == 1

/-- regime.rs `branch_addr` in the EL1&0 regime: a branch target's top
byte replaced by bit 55's copies under top-byte-ignore. -/
@[inline] def branchAddr (sys : Sys) (target : UInt64) : UInt64 :=
  let upper := (target >>> 55) &&& 1 == 1
  if tbi sys upper then sext target 56 else target

@[inline] def mmuOn (sys : Sys) : Bool := sys.sctlr &&& SCTLR_M != 0

/-- id.rs `ADDRESS_SIZES`. -/
def addressSize (ps : UInt64) : UInt64 :=
  match ps &&& 7 with
  | 0 => 32 | 1 => 36 | 2 => 40 | 3 => 42 | 4 => 44 | 5 => 48 | 6 => 52 | _ => 56

/-- regime.rs `Granule::translation_size` of the 4 KiB granule at `level`
(0..3). -/
@[inline] def translationSize (level : UInt64) : UInt64 := 12 + 9 * (3 - level)

/-- memattr.rs `stage1` / `reserved_mair`: whether `MAIR` attribute `attr`
is Device memory on a processor without FEAT_XS (a reserved encoding is
Device-nGnRnE). -/
@[inline] def attrDevice (attr : UInt64) : Bool := attr &&& 0xF0 == 0 || attr &&& 0x0F == 0

/-- A descriptor read of the walk (bus.rs `read_descriptor`, a doubleword
read): RAM directly, anything else through the board. -/
def readDescriptor (mem : ByteArray) (board : Board) (pa now : UInt64) : Board × Option UInt64 :=
  if pa >= RAM_BASE && pa - RAM_BASE + 8 <= RAM_SIZE then
    (board, some (ld64 mem (pa - RAM_BASE).toUSize))
  else board.read pa 3 now

/-- mmu.rs `walk` and `walk_tables`: the leaf's TLB entry value (PA page,
permissions, Device, level) or the fault's code. -/
def walk (sys : Sys) (mem : ByteArray) (board : Board) (va now : UInt64) (fetch : Bool) :
    Board × Except UInt32 UInt64 :=
  let upper := (va >>> 55) &&& 1 == 1
  let txsz := (sys.tcr >>> (if upper then 16 else 0)) &&& 0x3F
  let epd := (sys.tcr >>> (if upper then 23 else 7)) &&& 1 == 1
  let tb := tbi sys upper
  let _ := fetch
  let iaBits := 64 - txsz
  let top : UInt64 := if tb then 56 else 64
  let check := ones (top - iaBits) <<< iaBits
  let inRange := va &&& check == (if upper then check else 0)
  if txsz < 16 || txsz > 39 || epd || !inRange then (board, .error (fscTranslation 0)) else
  let oa := min (addressSize (sys.tcr >>> 32)) PA_BITS
  let fits (a : UInt64) : Bool := a >>> oa == 0
  let start : UInt64 :=
    if iaBits > 39 then 0 else if iaBits > 30 then 1 else if iaBits > 21 then 2 else 3
  let startBits := iaBits - translationSize start
  let ttbr := if upper then sys.ttbr1 else sys.ttbr0
  let table := (ttbr &&& 0x0000_FFFF_FFFF_FFFE) &&& ~~~(ones (startBits + 3))
  if !fits table then (board, .error (fscAddressSize 0)) else
  let rec go (board : Board) (table level tables : UInt64) (fuel : Nat) : Board × Except UInt32 UInt64 :=
    match fuel with
    | 0 => (board, .error (fscTranslation level))
    | fuel + 1 =>
    let shift := translationSize level
    let idxBits := if level == start then startBits else 9
    let index := (va >>> shift) &&& ones idxBits
    let (board, d) := readDescriptor mem board (table + index * 8) now
    match d with
    | none => (board, .error (fscExternalOnWalk level))
    | some desc =>
    if desc &&& 1 == 0 then (board, .error (fscTranslation level)) else
    if desc &&& 2 != 0 && level < 3 then
      let next := desc &&& 0x0000_FFFF_FFFF_F000
      if !fits next then (board, .error (fscAddressSize level))
      else go board next (level + 1) (tables ||| (desc >>> 59)) fuel
    else
    -- a page at level 3, a block at levels 1 and 2
    let leafOk := (level == 3 && desc &&& 2 != 0) || level == 1 || level == 2
    if !leafOk then (board, .error (fscTranslation level)) else
    let out := desc &&& 0x0000_FFFF_FFFF_FFFF &&& ~~~(ones shift)
    if !fits out then (board, .error (fscAddressSize level)) else
    if (desc >>> 10) &&& 1 == 0 then (board, .error (fscAccessFlag level)) else
    let apTable := (tables >>> 2) &&& 3
    let uxnTable := (tables >>> 1) &&& 1 == 1
    let pxnTable := tables &&& 1 == 1
    let ap := (desc >>> 6) &&& 3
    let el1w0 := ap &&& 2 == 0
    let el0r0 := ap &&& 1 == 1
    let el0w0 := el0r0 && el1w0
    let (el0r, el0w) := if apTable &&& 1 != 0 then (false, false) else (el0r0, el0w0)
    let (el1w, el0w) := if apTable &&& 2 != 0 then (false, false) else (el1w0, el0w)
    let uxn := (desc >>> 54) &&& 1 == 1 || uxnTable
    let pxn := (desc >>> 53) &&& 1 == 1 || pxnTable
    let wxn := sys.sctlr &&& SCTLR_WXN != 0
    let el1x := !pxn && !el0w && !(wxn && el1w)
    let el0x := !uxn && !(wxn && el0w)
    let perms : UInt64 := PERM_EL1_R ||| (if el1w then PERM_EL1_W else 0) ||| (if el1x then PERM_EL1_X else 0)
      ||| (if el0r then PERM_EL0_R else 0) ||| (if el0w then PERM_EL0_W else 0) ||| (if el0x then PERM_EL0_X else 0)
    let attr := (sys.mair >>> (((desc >>> 2) &&& 7) * 8)) &&& 0xFF
    let dev : UInt64 := if attrDevice attr then ATTR_DEVICE else 0
    let pa := (out ||| (va &&& ones shift)) &&& ~~~0xFFF
    (board, .ok (pa ||| perms ||| dev ||| (level <<< ATTR_LEVEL_SHIFT)))
  go board table start 0 5

/-- mmu.rs `permitted`: the permission bit an access of `kind` at `el`
needs. -/
@[inline] def neededPerm (kind el : UInt8) : UInt64 :=
  if el == 0 then (if kind == AK_READ then PERM_EL0_R else if kind == AK_WRITE then PERM_EL0_W else PERM_EL0_X)
  else (if kind == AK_READ then PERM_EL1_R else if kind == AK_WRITE then PERM_EL1_W else PERM_EL1_X)

def flushTlb (sys : Sys) : Sys := { sys with tlb := Array.replicate (2 * TLB_SETS) MISS }

/-- The TLB entry for `va`'s page, walking on a miss and keeping what the
walk found (mmu.rs `translate_needs`, `translate_miss`, `cache`). -/
def tlbEntry (sys : Sys) (mem : ByteArray) (va now : UInt64) (fetch : Bool) :
    Sys × Except UInt32 UInt64 :=
  let vp := va >>> 12
  let i := ((vp &&& (TLB_SETS.toUInt64 - 1)) * 2).toUSize
  if agu sys.tlb i MISS == vp then (sys, .ok (agu sys.tlb (i + 1) 0))
  else
    let board := sys.board
    let sys := { sys with board := default }
    let (board, r) := walk sys mem board va now fetch
    let sys := { sys with board := board }
    match r with
    | .ok e => ({ sys with tlb := asu (asu sys.tlb i vp) (i + 1) e }, .ok e)
    | .error f => (sys, .error f)

/-- mmu.rs `translate_needs` for an access of `kind` checked at `el`:
the physical address and whether it is Device memory, or the abort.
`refuseDevice` is `OnDevice::Refused` (a misaligned access, where Device
memory takes an alignment fault). -/
def translate (sys : Sys) (mem : ByteArray) (va now : UInt64) (kind el : UInt8) (refuseDevice : Bool) :
    Sys × Except Abort (UInt64 × Bool) :=
  let write := kind == AK_WRITE
  if !mmuOn sys then
    -- mmu.rs `untranslated`: stage 1 disabled, data accesses to Device memory
    let upper := (va >>> 55) &&& 1 == 1
    let addr := if tbi sys upper then va &&& 0x00FF_FFFF_FFFF_FFFF else va
    if addr >>> PA_BITS != 0 then (sys, .error ⟨fscAddressSize 0, write⟩)
    else if refuseDevice && kind != AK_FETCH then (sys, .error ⟨FSC_ALIGNMENT, write⟩)
    else (sys, .ok (addr, kind != AK_FETCH))
  else
  let (sys, r) := tlbEntry sys mem va now (kind == AK_FETCH)
  match r with
  | .error f => (sys, .error ⟨f, write⟩)
  | .ok e =>
    let dev := e &&& ATTR_DEVICE != 0
    if refuseDevice && dev then (sys, .error ⟨FSC_ALIGNMENT, write⟩)
    else if e &&& neededPerm kind el == 0 then
      (sys, .error ⟨fscPermission ((e >>> ATTR_LEVEL_SHIFT) &&& 3), write⟩)
    else (sys, .ok ((e &&& ~~~0xFFF) ||| (va &&& 0xFFF), dev))

end Leanbox
