/-
Byte access with `USize` indices and the Arm ARM's shared bit pseudocode.

Every access to a `ByteArray` goes through `gb`/`sb` with a `USize` index
and the bounds lemma, so no access carries a `Nat` (bignum) fallback. A
2-, 4- or 8-byte word wholly in bounds is checked once: `ByteArray.word_lt`
proves each of its bytes in bounds from that check, so the bytes are read
with no check of their own and the C compiler joins the reads into one load
(the stores stay separate, each with the runtime's exclusivity check).
-/

theorem ByteArray.lt_size_of_lt_usize {m : ByteArray} {i : USize} (h : i < m.usize) :
    i.toNat < m.size := by
  have h1 : i.toNat < m.usize.toNat := USize.lt_iff_toNat_lt.mp h
  have h2 : m.usize.toNat ≤ m.size := by
    simp only [ByteArray.usize, USize.toNat_ofNat']
    exact Nat.mod_le _ _
  omega

theorem Array.lt_size_of_lt_usize {α} {a : Array α} {i : USize} (h : i < a.usize) :
    i.toNat < a.size := by
  have h1 : i.toNat < a.usize.toNat := USize.lt_iff_toNat_lt.mp h
  have h2 : a.usize.toNat ≤ a.size := by
    simp only [Array.usize, USize.toNat_ofNat']
    exact Nat.mod_le _ _
  omega

theorem ByteArray.usize_toNat_le (m : ByteArray) : m.usize.toNat ≤ m.size := by
  simp only [ByteArray.usize, USize.toNat_ofNat']
  exact Nat.mod_le _ _

/-- Byte `k` of a `w`-byte word at `i` is in bounds once the whole word is:
one check stands for all `w` bytes, so their reads carry no check of their
own. -/
theorem ByteArray.word_lt {m : ByteArray} {i k w : USize} (h : w ≤ m.usize ∧ i ≤ m.usize - w)
    (hk : k.toNat < w.toNat) : (i + k).toNat < m.size := by
  have h1 := USize.le_iff_toNat_le.mp h.1
  have h2 := USize.le_iff_toNat_le.mp h.2
  rw [USize.toNat_sub_of_le _ _ h.1] at h2
  have h3 := m.usize_toNat_le
  have h4 := USize.toNat_lt_size m.usize
  have h5 := USize.size_eq_two_pow
  rw [USize.toNat_add, Nat.mod_eq_of_lt (by omega)]
  omega

/-- Literal indices compare as their values (each below 2^32, so on every
platform). -/
theorem USize.lit_lt {a b : Nat} (hb : b < 4294967296) (h : a < b) :
    (USize.ofNat a).toNat < (USize.ofNat b).toNat := by
  rw [USize.toNat_ofNat_of_lt_32 (by omega), USize.toNat_ofNat_of_lt_32 hb]; exact h

theorem ByteArray.size_uset' (m : ByteArray) (i : USize) (v : UInt8) (h : i.toNat < m.size) :
    (m.uset i v h).size = m.size := by
  cases m; exact Array.size_uset ..

namespace Leanbox

@[inline] def gb (m : ByteArray) (i : USize) : UInt64 :=
  if h : i < m.usize then (m.uget i (ByteArray.lt_size_of_lt_usize h)).toUInt64 else 0

/-- Byte `k` of a word whose bounds `h` proved, read with no check. -/
@[inline] def gw (m : ByteArray) {i w : USize} (h : w ≤ m.usize ∧ i ≤ m.usize - w) (k : USize)
    (hk : k.toNat < w.toNat) : UInt64 :=
  (m.uget (i + k) (ByteArray.word_lt h hk)).toUInt64

/-- A store of one byte that keeps the array's size `n` in its type, so the
next byte's bounds proof carries over. -/
@[inline] def putw (m : ByteArray) (n : Nat) (hn : m.size = n) (j : USize) (hj : j.toNat < n)
    (v : UInt64) : { r : ByteArray // r.size = n } :=
  ⟨m.uset j v.toUInt8 (hn ▸ hj), by rw [ByteArray.size_uset']; exact hn⟩

/- A word wholly in bounds is read (written) under one check, its bytes
straight-line, which the C compiler joins into one load; a word reaching past
the end is read (written) byte by byte, each byte outside reading 0 (left
unwritten). -/
@[inline] def ld16 (m : ByteArray) (i : USize) : UInt64 :=
  if h : 2 ≤ m.usize ∧ i ≤ m.usize - 2 then
    gw m h 0 (USize.lit_lt (by decide) (by decide)) ||| (gw m h 1 (USize.lit_lt (by decide) (by decide)) <<< 8)
  else gb m i ||| (gb m (i+1) <<< 8)
@[inline] def ld32 (m : ByteArray) (i : USize) : UInt64 :=
  if h : 4 ≤ m.usize ∧ i ≤ m.usize - 4 then
    gw m h 0 (USize.lit_lt (by decide) (by decide)) ||| (gw m h 1 (USize.lit_lt (by decide) (by decide)) <<< 8) ||| (gw m h 2 (USize.lit_lt (by decide) (by decide)) <<< 16)
      ||| (gw m h 3 (USize.lit_lt (by decide) (by decide)) <<< 24)
  else gb m i ||| (gb m (i+1) <<< 8) ||| (gb m (i+2) <<< 16) ||| (gb m (i+3) <<< 24)
@[inline] def ld64 (m : ByteArray) (i : USize) : UInt64 :=
  if h : 8 ≤ m.usize ∧ i ≤ m.usize - 8 then
    gw m h 0 (USize.lit_lt (by decide) (by decide)) ||| (gw m h 1 (USize.lit_lt (by decide) (by decide)) <<< 8) ||| (gw m h 2 (USize.lit_lt (by decide) (by decide)) <<< 16)
      ||| (gw m h 3 (USize.lit_lt (by decide) (by decide)) <<< 24) ||| (gw m h 4 (USize.lit_lt (by decide) (by decide)) <<< 32)
      ||| (gw m h 5 (USize.lit_lt (by decide) (by decide)) <<< 40) ||| (gw m h 6 (USize.lit_lt (by decide) (by decide)) <<< 48)
      ||| (gw m h 7 (USize.lit_lt (by decide) (by decide)) <<< 56)
  else
    let lo := gb m i ||| (gb m (i+1) <<< 8) ||| (gb m (i+2) <<< 16) ||| (gb m (i+3) <<< 24)
    let hi := gb m (i+4) ||| (gb m (i+5) <<< 8) ||| (gb m (i+6) <<< 16) ||| (gb m (i+7) <<< 24)
    lo ||| (hi <<< 32)

@[inline] def sb (m : ByteArray) (i : USize) (v : UInt64) : ByteArray :=
  if h : i < m.usize then m.uset i v.toUInt8 (ByteArray.lt_size_of_lt_usize h) else m
@[inline] def st16 (m : ByteArray) (i : USize) (v : UInt64) : ByteArray :=
  if h : 2 ≤ m.usize ∧ i ≤ m.usize - 2 then
    let n := m.size
    have p (k : USize) (hk : k.toNat < (2 : USize).toNat) : (i + k).toNat < n := ByteArray.word_lt h hk
    let r := putw m n rfl (i+0) (p 0 (USize.lit_lt (by decide) (by decide))) v
    (putw r.1 n r.2 (i+1) (p 1 (USize.lit_lt (by decide) (by decide))) (v >>> 8)).1
  else sb (sb m i v) (i+1) (v >>> 8)
@[inline] def st32 (m : ByteArray) (i : USize) (v : UInt64) : ByteArray :=
  if h : 4 ≤ m.usize ∧ i ≤ m.usize - 4 then
    let n := m.size
    have p (k : USize) (hk : k.toNat < (4 : USize).toNat) : (i + k).toNat < n := ByteArray.word_lt h hk
    let r := putw m n rfl (i+0) (p 0 (USize.lit_lt (by decide) (by decide))) v
    let r := putw r.1 n r.2 (i+1) (p 1 (USize.lit_lt (by decide) (by decide))) (v >>> 8)
    let r := putw r.1 n r.2 (i+2) (p 2 (USize.lit_lt (by decide) (by decide))) (v >>> 16)
    (putw r.1 n r.2 (i+3) (p 3 (USize.lit_lt (by decide) (by decide))) (v >>> 24)).1
  else sb (sb (sb (sb m i v) (i+1) (v >>> 8)) (i+2) (v >>> 16)) (i+3) (v >>> 24)
@[inline] def st64 (m : ByteArray) (i : USize) (v : UInt64) : ByteArray :=
  if h : 8 ≤ m.usize ∧ i ≤ m.usize - 8 then
    let n := m.size
    have p (k : USize) (hk : k.toNat < (8 : USize).toNat) : (i + k).toNat < n := ByteArray.word_lt h hk
    let r := putw m n rfl (i+0) (p 0 (USize.lit_lt (by decide) (by decide))) v
    let r := putw r.1 n r.2 (i+1) (p 1 (USize.lit_lt (by decide) (by decide))) (v >>> 8)
    let r := putw r.1 n r.2 (i+2) (p 2 (USize.lit_lt (by decide) (by decide))) (v >>> 16)
    let r := putw r.1 n r.2 (i+3) (p 3 (USize.lit_lt (by decide) (by decide))) (v >>> 24)
    let r := putw r.1 n r.2 (i+4) (p 4 (USize.lit_lt (by decide) (by decide))) (v >>> 32)
    let r := putw r.1 n r.2 (i+5) (p 5 (USize.lit_lt (by decide) (by decide))) (v >>> 40)
    let r := putw r.1 n r.2 (i+6) (p 6 (USize.lit_lt (by decide) (by decide))) (v >>> 48)
    (putw r.1 n r.2 (i+7) (p 7 (USize.lit_lt (by decide) (by decide))) (v >>> 56)).1
  else
    let m := sb (sb (sb (sb m i v) (i+1) (v >>> 8)) (i+2) (v >>> 16)) (i+3) (v >>> 24)
    sb (sb (sb (sb m (i+4) (v >>> 32)) (i+5) (v >>> 40)) (i+6) (v >>> 48)) (i+7) (v >>> 56)
/-- Load `1 << size` bytes, little-endian. -/
@[inline] def ldN (m : ByteArray) (i : USize) (size : UInt8) : UInt64 :=
  match size with | 0 => gb m i | 1 => ld16 m i | 2 => ld32 m i | _ => ld64 m i
@[inline] def stN (m : ByteArray) (i : USize) (size : UInt8) (v : UInt64) : ByteArray :=
  match size with | 0 => sb m i v | 1 => st16 m i v | 2 => st32 m i v | _ => st64 m i v

@[inline] def agu {α} (a : Array α) (i : USize) (dflt : α) : α :=
  if h : i < a.usize then a.uget i (Array.lt_size_of_lt_usize h) else dflt
@[inline] def asu {α} (a : Array α) (i : USize) (v : α) : Array α :=
  if h : i < a.usize then a.uset i v (Array.lt_size_of_lt_usize h) else a

/-- `n` zero bytes, built by doubling (each step one copy). -/
def zeros (n : Nat) : ByteArray := Id.run do
  let mut b := ByteArray.mk (Array.replicate (min n 4096) 0)
  while b.size < n do
    b := b ++ b
  return if b.size == n then b else b.extract 0 n

def MISS : UInt64 := 0xFFFFFFFFFFFFFFFF

-- ── Arm ARM shared pseudocode ──────────────────────────────────────────
@[inline] def ones (n : UInt64) : UInt64 := if n >= 64 then MISS else (1 <<< n) - 1
@[inline] def mask32 (sf : Bool) (x : UInt64) : UInt64 := if sf then x else x &&& 0xFFFFFFFF
@[inline] def ror (sf : Bool) (x : UInt64) (r : UInt64) : UInt64 :=
  if sf then (if r == 0 then x else (x >>> r) ||| (x <<< (64 - r)))
  else let x := x &&& 0xFFFFFFFF
       if r == 0 then x else ((x >>> r) ||| (x <<< (32 - r))) &&& 0xFFFFFFFF
@[inline] def shift64 (x : UInt64) (sh : UInt8) (amt : UInt64) : UInt64 :=
  match sh with
  | 0 => x <<< amt
  | 1 => x >>> amt
  | 2 => (x.toInt64 >>> amt.toInt64).toUInt64
  | _ => ror true x amt
/-- `ShiftReg` for a 32- or 64-bit operand, `amt` below the width. -/
@[inline] def shiftReg (sf : Bool) (x : UInt64) (sh : UInt8) (amt : UInt64) : UInt64 :=
  if sf then shift64 x sh amt else
    let x := x &&& 0xFFFFFFFF
    match sh with
    | 0 => (x <<< amt) &&& 0xFFFFFFFF
    | 1 => x >>> amt
    | 2 => ((x.toUInt32.toInt32 >>> amt.toUInt32.toInt32).toUInt32).toUInt64
    | _ => ror false x amt

@[inline] def sext (x : UInt64) (bits : UInt64) : UInt64 :=
  if bits >= 64 then x else ((x <<< (64 - bits)).toInt64 >>> (64 - bits).toInt64).toUInt64

/-- AddWithCarry's NZCV, in bits 3..0. -/
@[inline] def awcFlags (sf : Bool) (x y carry : UInt64) : UInt64 :=
  if sf then
    let r := x + y + carry
    let z : UInt64 := if r == 0 then 1 else 0
    let cf : UInt64 := if carry == 1 then (if r <= x then 1 else 0) else (if r < x then 1 else 0)
    ((r >>> 63) <<< 3) ||| (z <<< 2) ||| (cf <<< 1) ||| (((x ^^^ r) &&& (y ^^^ r)) >>> 63)
  else
    let x := x &&& 0xFFFFFFFF
    let y := y &&& 0xFFFFFFFF
    let full := x + y + carry
    let r := full &&& 0xFFFFFFFF
    let z : UInt64 := if r == 0 then 1 else 0
    (((r >>> 31) &&& 1) <<< 3) ||| (z <<< 2) ||| ((full >>> 32) <<< 1) ||| ((((x ^^^ r) &&& (y ^^^ r)) >>> 31) &&& 1)

@[inline] def logicFlags (sf : Bool) (r : UInt64) : UInt64 :=
  let n := if sf then r >>> 63 else (r >>> 31) &&& 1
  (n <<< 3) ||| ((if r == 0 then (1 : UInt64) else 0) <<< 2)

@[inline] def logic (opc : UInt8) (x y : UInt64) : UInt64 :=
  match opc with | 1 => x ||| y | 2 => x ^^^ y | _ => x &&& y

/-- `ConditionHolds` over NZCV in bits 3..0. -/
@[inline] def conditionHolds (cond : UInt8) (nzcv : UInt64) : Bool :=
  let n := nzcv &&& 8 != 0
  let z := nzcv &&& 4 != 0
  let c := nzcv &&& 2 != 0
  let v := nzcv &&& 1 != 0
  let base := match cond >>> 1 with
    | 0 => z | 1 => c | 2 => n | 3 => v
    | 4 => c && !z | 5 => n == v | 6 => n == v && !z | _ => true
  if cond &&& 1 == 1 && cond != 15 then !base else base

def highestSetBit (x : UInt64) (width : Nat) : Int := Id.run do
  let mut r : Int := -1
  for i in [0:width] do
    if (x >>> i.toUInt64) &&& 1 == 1 then r := i
  return r

def replicate (x : UInt64) (esize : UInt64) : UInt64 := Id.run do
  let mut r : UInt64 := 0
  let mut s : UInt64 := 0
  while s < 64 do
    r := r ||| (x <<< s)
    s := s + esize
  return r

/-- DecodeBitMasks(immN, imms, immr, immediate, M) = (wmask, tmask). -/
def decodeBitMasks (immN imms immr : UInt64) (immediate : Bool) (m : UInt64) : Option (UInt64 × UInt64) :=
  let len := highestSetBit ((immN <<< 6) ||| ((~~~imms) &&& 0x3F)) 7
  if len < 1 then none else
  let len := len.toNat.toUInt64
  let esize := (1 : UInt64) <<< len
  if esize > m then none else
  let levels := ones len
  if immediate && (imms &&& levels) == levels then none else
  let s := imms &&& levels
  let r := immr &&& levels
  let d := (s - r) &&& levels
  let welem := ones (s + 1)
  let telem := ones (d + 1)
  let rotated := if r == 0 then welem else ((welem >>> r) ||| (welem <<< (esize - r))) &&& ones esize
  let full := if m == 64 then MISS else 0xFFFFFFFF
  some (replicate rotated esize &&& full, replicate telem esize &&& full)

def hex (x : UInt64) : String := String.ofList (Nat.toDigits 16 x.toNat)

end Leanbox
