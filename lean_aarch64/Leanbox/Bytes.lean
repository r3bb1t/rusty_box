/-!
# Bytes

Memory, the register file and the devices' buffers are `ByteArray`s. This
module reads and writes them with `USize` indices, so no access carries a
`Nat` (bignum) fallback, and every access is written `m[i]` or `m.uset i v`:
the bounds proofs come from `get_elem_tactic`, extended below with the facts
this module's checks put in context. A 2-, 4- or 8-byte word is checked once
(`ByteArray.WordIn`); each of its bytes is proven in bounds from that one
check, so the byte reads are straight-line and the C compiler joins them
into one load (the byte stores stay separate, each after the runtime's
exclusivity check).
-/

theorem ByteArray.usize_toNat_le (m : ByteArray) : m.usize.toNat ≤ m.size := by
  simp only [ByteArray.usize, USize.toNat_ofNat']
  exact Nat.mod_le _ _

theorem ByteArray.lt_size_of_lt_usize {m : ByteArray} {i : USize} (h : i < m.usize) :
    i.toNat < m.size :=
  Nat.lt_of_lt_of_le (USize.lt_iff_toNat_lt.mp h) m.usize_toNat_le

theorem Array.lt_size_of_lt_usize {α} {a : Array α} {i : USize} (h : i < a.usize) :
    i.toNat < a.size := by
  have h1 : i.toNat < a.usize.toNat := USize.lt_iff_toNat_lt.mp h
  have h2 : a.usize.toNat ≤ a.size := by
    simp only [Array.usize, USize.toNat_ofNat']
    exact Nat.mod_le _ _
  omega

/-- A `w`-byte word at `i` lies inside `m`: one check, which cannot overflow
(`i + w` could). -/
abbrev ByteArray.WordIn (m : ByteArray) (i w : USize) : Prop := w ≤ m.usize ∧ i ≤ m.usize - w

/-- Byte `k` of a word inside `m` is inside `m`. -/
theorem ByteArray.WordIn.lt {m : ByteArray} {i k w : USize} (h : m.WordIn i w)
    (hk : k.toNat < w.toNat) : (i + k).toNat < m.size := by
  have h1 := USize.le_iff_toNat_le.mp h.1
  have h2 := USize.le_iff_toNat_le.mp h.2
  rw [USize.toNat_sub_of_le _ _ h.1] at h2
  have := m.usize_toNat_le
  have := USize.toNat_lt_size m.usize
  have := USize.size_eq_two_pow
  rw [USize.toNat_add, Nat.mod_eq_of_lt (by omega)]
  omega

theorem ByteArray.WordIn.lt_zero {m : ByteArray} {i w : USize} (h : m.WordIn i w)
    (hw : 0 < w.toNat) : i.toNat < m.size := by
  simpa using h.lt (k := 0) (by simpa using hw)

@[simp] theorem ByteArray.size_uset (m : ByteArray) (i : USize) (v : UInt8)
    (h : i.toNat < m.size) : (m.uset i v h).size = m.size := by
  cases m; exact Array.size_uset ..

/- The bounds of `m[i]`, `m[i + k]` and `m.uset i v` from what is in context:
`i < m.usize`, or a `WordIn` that holds `i` (a store leaves the size alone,
so a `WordIn` of the array before a chain of stores serves every store in
it). -/
macro_rules
  | `(tactic| get_elem_tactic_extensible) =>
    `(tactic| first
      | exact ByteArray.lt_size_of_lt_usize ‹_›
      | exact Array.lt_size_of_lt_usize ‹_›
      | ((try simp only [ByteArray.size_uset]);
         first
         | exact ByteArray.WordIn.lt ‹_› (by simp [USize.toNat_ofNat_of_lt_32])
         | exact ByteArray.WordIn.lt_zero ‹_› (by simp [USize.toNat_ofNat_of_lt_32])))

namespace Leanbox

/-- Byte `i`, or 0 past the end. -/
@[inline] def gb (m : ByteArray) (i : USize) : UInt64 :=
  if _ : i < m.usize then m[i].toUInt64 else 0

/-- Byte `i` set to `v`'s low byte, or `m` unchanged past the end. -/
@[inline] def sb (m : ByteArray) (i : USize) (v : UInt64) : ByteArray :=
  if _ : i < m.usize then m.uset i v.toUInt8 else m

/- Little-endian words. A word inside `m` is read (written) under its one
`WordIn` check; a word reaching past the end byte by byte, each byte outside
reading 0 (left unwritten). -/
@[inline] def ld16 (m : ByteArray) (i : USize) : UInt64 :=
  if _ : m.WordIn i 2 then m[i].toUInt64 ||| m[i+1].toUInt64 <<< 8
  else gb m i ||| gb m (i+1) <<< 8

@[inline] def ld32 (m : ByteArray) (i : USize) : UInt64 :=
  if _ : m.WordIn i 4 then
    m[i].toUInt64 ||| m[i+1].toUInt64 <<< 8 ||| m[i+2].toUInt64 <<< 16 ||| m[i+3].toUInt64 <<< 24
  else gb m i ||| gb m (i+1) <<< 8 ||| gb m (i+2) <<< 16 ||| gb m (i+3) <<< 24

@[inline] def ld64 (m : ByteArray) (i : USize) : UInt64 :=
  if _ : m.WordIn i 8 then
    m[i].toUInt64 ||| m[i+1].toUInt64 <<< 8 ||| m[i+2].toUInt64 <<< 16 ||| m[i+3].toUInt64 <<< 24
      ||| m[i+4].toUInt64 <<< 32 ||| m[i+5].toUInt64 <<< 40 ||| m[i+6].toUInt64 <<< 48
      ||| m[i+7].toUInt64 <<< 56
  else
    gb m i ||| gb m (i+1) <<< 8 ||| gb m (i+2) <<< 16 ||| gb m (i+3) <<< 24
      ||| gb m (i+4) <<< 32 ||| gb m (i+5) <<< 40 ||| gb m (i+6) <<< 48 ||| gb m (i+7) <<< 56

@[inline] def st16 (m : ByteArray) (i : USize) (v : UInt64) : ByteArray :=
  if _ : m.WordIn i 2 then m |>.uset i v.toUInt8 |>.uset (i+1) (v >>> 8).toUInt8
  else m |> (sb · i v) |> (sb · (i+1) (v >>> 8))

@[inline] def st32 (m : ByteArray) (i : USize) (v : UInt64) : ByteArray :=
  if _ : m.WordIn i 4 then
    m |>.uset i v.toUInt8 |>.uset (i+1) (v >>> 8).toUInt8
      |>.uset (i+2) (v >>> 16).toUInt8 |>.uset (i+3) (v >>> 24).toUInt8
  else m |> (sb · i v) |> (sb · (i+1) (v >>> 8)) |> (sb · (i+2) (v >>> 16)) |> (sb · (i+3) (v >>> 24))

@[inline] def st64 (m : ByteArray) (i : USize) (v : UInt64) : ByteArray :=
  if _ : m.WordIn i 8 then
    m |>.uset i v.toUInt8 |>.uset (i+1) (v >>> 8).toUInt8
      |>.uset (i+2) (v >>> 16).toUInt8 |>.uset (i+3) (v >>> 24).toUInt8
      |>.uset (i+4) (v >>> 32).toUInt8 |>.uset (i+5) (v >>> 40).toUInt8
      |>.uset (i+6) (v >>> 48).toUInt8 |>.uset (i+7) (v >>> 56).toUInt8
  else
    m |> (sb · i v) |> (sb · (i+1) (v >>> 8)) |> (sb · (i+2) (v >>> 16)) |> (sb · (i+3) (v >>> 24))
      |> (sb · (i+4) (v >>> 32)) |> (sb · (i+5) (v >>> 40)) |> (sb · (i+6) (v >>> 48))
      |> (sb · (i+7) (v >>> 56))
/-- Load `1 << size` bytes, little-endian. -/
@[inline] def ldN (m : ByteArray) (i : USize) (size : UInt8) : UInt64 :=
  match size with | 0 => gb m i | 1 => ld16 m i | 2 => ld32 m i | _ => ld64 m i
@[inline] def stN (m : ByteArray) (i : USize) (size : UInt8) (v : UInt64) : ByteArray :=
  match size with | 0 => sb m i v | 1 => st16 m i v | 2 => st32 m i v | _ => st64 m i v

@[inline] def agu {α} (a : Array α) (i : USize) (dflt : α) : α :=
  if _ : i < a.usize then a[i] else dflt
@[inline] def asu {α} (a : Array α) (i : USize) (v : α) : Array α :=
  if _ : i < a.usize then a.uset i v (by get_elem_tactic) else a

/-- `n` zero bytes, built by doubling (each step one copy). -/
def zeros (n : Nat) : ByteArray := Id.run do
  let mut b := ByteArray.mk (Array.replicate (min n 4096) 0)
  while b.size < n do
    b := b ++ b
  return if b.size == n then b else b.extract 0 n

def MISS : UInt64 := 0xFFFFFFFFFFFFFFFF

end Leanbox
