import Leanbox.Bytes
/-!
# The Arm ARM's shared pseudocode

The bit-level functions the Arm ARM writes its instruction descriptions in
(its shared pseudocode: `Ones`, `ROR`, `ShiftReg`, `SignExtend`, the flags
of `AddWithCarry`, `ConditionHolds`, `HighestSetBit`, `Replicate`,
`DecodeBitMasks`), over `UInt64` words.
-/
namespace Leanbox

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
