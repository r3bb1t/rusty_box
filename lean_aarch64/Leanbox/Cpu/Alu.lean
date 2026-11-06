import Leanbox.Cpu.Exec
import Std.Tactic.BVDecide
/-
The values of the pure data-processing ops (cpu/dp.rs), shared by the
reference executor (`execOp`) and the hot loop's handlers, so both compute
each op the one way.
-/
namespace Leanbox

/-- ADD/SUB of `x` and `y`, both already in the operation's width (Arm
`AddWithCarry(x, y or NOT(y), carry)`): the result. -/
@[inline] def asRes (sf sub : Bool) (x y : UInt64) : UInt64 :=
  if sub then mask32 sf (x + mask32 sf (~~~y) + 1) else mask32 sf (x + y)
/-- Its NZCV. -/
@[inline] def asFlags (sf sub : Bool) (x y : UInt64) : UInt64 :=
  if sub then awcFlags sf x (mask32 sf (~~~y)) 1 else awcFlags sf x y 0

/-- ADC/SBC with carry `c`: the result and NZCV. -/
@[inline] def adcRes (sf sub : Bool) (x y c : UInt64) : UInt64 :=
  mask32 sf (x + (if sub then mask32 sf (~~~y) else y) + c)
@[inline] def adcFlags (sf sub : Bool) (x y c : UInt64) : UInt64 :=
  awcFlags sf x (if sub then mask32 sf (~~~y) else y) c

/-- SBFM/BFM/UBFM (`opc` 0, 1, 2) of `src` into `dst`. -/
@[inline] def bfmVal (sf : Bool) (opc : UInt8) (src dst r s wmask tmask : UInt64) : UInt64 :=
  let bot := ror sf src r &&& wmask
  let res := match opc with
    | 0 => ((if (src >>> s) &&& 1 == 1 then MISS else 0) &&& ~~~tmask) ||| (bot &&& tmask)
    | 1 => (dst &&& ~~~tmask) ||| (((dst &&& ~~~wmask) ||| bot) &&& tmask)
    | _ => bot &&& tmask
  mask32 sf res

@[inline] def extrVal (sf : Bool) (xn xm lsb : UInt64) : UInt64 :=
  if sf then (if lsb == 0 then xm else (xm >>> lsb) ||| (xn <<< (64 - lsb)))
  else
    let x := xn &&& 0xFFFF_FFFF
    let y := xm &&& 0xFFFF_FFFF
    (((x <<< 32) ||| y) >>> lsb) &&& 0xFFFF_FFFF

/-- Logical (shifted register) with Rm already shifted. -/
@[inline] def logVal (sf : Bool) (opc : UInt8) (inv : Bool) (xn ym : UInt64) : UInt64 :=
  mask32 sf (logic opc xn (mask32 sf (if inv then ~~~ym else ym)))

@[inline] def cselVal (sf inv inc : Bool) (cond : UInt8) (nzcv xn xm : UInt64) : UInt64 :=
  mask32 sf (if conditionHolds cond nzcv then xn
    else
      let y := if inv then ~~~xm else xm
      if inc then y + 1 else y)

@[inline] def ccmpFlags (sf sub : Bool) (cond flags : UInt8) (nzcv x y : UInt64) : UInt64 :=
  if conditionHolds cond nzcv then asFlags sf sub (mask32 sf x) (mask32 sf y) else flags.toUInt64

/-! ### CLZ, RBIT, REV16, REV32, REV

Each is defined as the Arm ARM's pseudocode states it, bit by bit or byte by
byte over the operation's width (`CountLeadingZeroBits`, RBIT's and REV's
`for` loops), by recursion, so the definition is what a proof reasons about.
Each is computed by a loop-free form (cpu/dp.rs `exec_dp1` uses the host's
`leading_zeros`, `reverse_bits`, `swap_bytes`): the `@[csimp]` theorem,
proved by `bv_decide` for every input, has the compiler run that form
wherever the definition is called. -/

/-- The leading zero bits among the low `n` bits of `x`. -/
def clzBits (x : UInt64) : Nat → UInt64
  | 0 => 0
  | n + 1 => if (x >>> n.toUInt64) &&& 1 == 1 then 0 else clzBits x n + 1

/-- CLZ of the operation's width (`CountLeadingZeroBits`). -/
def clz (sf : Bool) (x : UInt64) : UInt64 := if sf then clzBits x 64 else clzBits x 32

/-- `CountLeadingZeroBits` of 64 bits by binary search. -/
def clz64Fast (x0 : UInt64) : UInt64 :=
  if x0 == 0 then 64 else
  let a := x0 &&& 0xFFFFFFFF00000000 == 0
  let n1 : UInt64 := if a then 32 else 0
  let x1 := if a then x0 <<< 32 else x0
  let b := x1 &&& 0xFFFF000000000000 == 0
  let n2 := if b then n1 + 16 else n1
  let x2 := if b then x1 <<< 16 else x1
  let c := x2 &&& 0xFF00000000000000 == 0
  let n3 := if c then n2 + 8 else n2
  let x3 := if c then x2 <<< 8 else x2
  let d := x3 &&& 0xF000000000000000 == 0
  let n4 := if d then n3 + 4 else n3
  let x4 := if d then x3 <<< 4 else x3
  let e := x4 &&& 0xC000000000000000 == 0
  let n5 := if e then n4 + 2 else n4
  let x5 := if e then x4 <<< 2 else x4
  if x5 &&& 0x8000000000000000 == 0 then n5 + 1 else n5

/-- The low 32 bits counted as the top half of 64, bit 31 set below them so
that a zero word counts 32. -/
def clzFast (sf : Bool) (x : UInt64) : UInt64 :=
  if sf then clz64Fast x else clz64Fast (x <<< 32 ||| 0x8000_0000)

@[csimp] theorem clz_eq_clzFast : @clz = @clzFast := by
  funext sf x
  cases sf <;> simp only [clz, clzFast, clzBits, clz64Fast, Nat.toUInt64, UInt64.ofNat] <;> bv_decide

/-- Bit `i` of `x` at bit `w - 1 - i`, for the low `n` bits. -/
def rbitBits (x w : UInt64) : Nat → UInt64
  | 0 => 0
  | i + 1 => rbitBits x w i ||| ((x >>> i.toUInt64) &&& 1) <<< (w - 1 - i.toUInt64)

/-- RBIT of the operation's width. -/
def rbit (sf : Bool) (x : UInt64) : UInt64 := if sf then rbitBits x 64 64 else rbitBits x 32 32

/-- The bit fields `m` selects swapped with the fields `s` bits above them. -/
@[inline] def swapBits (x s m : UInt64) : UInt64 := (x >>> s &&& m) ||| ((x &&& m) <<< s)

/-- 64 bits reversed: adjacent bits, pairs, nibbles, bytes, halfwords and
words swapped in turn. -/
def rbit64Fast (x : UInt64) : UInt64 :=
  let x := swapBits x 1 0x5555555555555555
  let x := swapBits x 2 0x3333333333333333
  let x := swapBits x 4 0x0F0F0F0F0F0F0F0F
  let x := swapBits x 8 0x00FF00FF00FF00FF
  let x := swapBits x 16 0x0000FFFF0000FFFF
  swapBits x 32 0x00000000FFFFFFFF

def rbitFast (sf : Bool) (x : UInt64) : UInt64 := if sf then rbit64Fast x else rbit64Fast x >>> 32

@[csimp] theorem rbit_eq_rbitFast : @rbit = @rbitFast := by
  funext sf x
  cases sf <;> simp only [rbit, rbitFast, rbitBits, rbit64Fast, swapBits, Nat.toUInt64,
    UInt64.ofNat] <;> bv_decide

/-- Byte `j` of `x` at its mirror position in its `c`-byte container, for the
low `n` bytes. -/
def revBytes (x : UInt64) (c : Nat) : Nat → UInt64
  | 0 => 0
  | j + 1 => revBytes x c j |||
      ((x >>> (8 * j).toUInt64) &&& 0xFF) <<< (8 * (j / c * c + (c - 1 - j % c))).toUInt64

/-- REV16, REV32 and REV of the operation's width: the bytes of each 2-, 4-
or 8-byte container reversed. -/
def rev16 (sf : Bool) (x : UInt64) : UInt64 := if sf then revBytes x 2 8 else revBytes x 2 4
def rev32 (sf : Bool) (x : UInt64) : UInt64 := if sf then revBytes x 4 8 else revBytes x 4 4
def rev64 (sf : Bool) (x : UInt64) : UInt64 := if sf then revBytes x 8 8 else revBytes x 8 4

def rev16Fast (sf : Bool) (x : UInt64) : UInt64 :=
  let y := swapBits x 8 0x00FF00FF00FF00FF
  if sf then y else y &&& 0xFFFF_FFFF
def rev32Fast (sf : Bool) (x : UInt64) : UInt64 :=
  let y := swapBits (swapBits x 8 0x00FF00FF00FF00FF) 16 0x0000FFFF0000FFFF
  if sf then y else y &&& 0xFFFF_FFFF
/-- The 32-bit form has no 8-byte container: its four bytes land in the top
half (an encoding the decoder leaves unallocated). -/
def rev64Fast (sf : Bool) (x : UInt64) : UInt64 :=
  let x := if sf then x else x &&& 0xFFFF_FFFF
  swapBits (swapBits (swapBits x 8 0x00FF00FF00FF00FF) 16 0x0000FFFF0000FFFF) 32 0x00000000FFFFFFFF

@[csimp] theorem rev16_eq_rev16Fast : @rev16 = @rev16Fast := by
  funext sf x
  cases sf <;> simp only [rev16, rev16Fast, revBytes, swapBits, Nat.toUInt64, UInt64.ofNat]
    <;> bv_decide
@[csimp] theorem rev32_eq_rev32Fast : @rev32 = @rev32Fast := by
  funext sf x
  cases sf <;> simp only [rev32, rev32Fast, revBytes, swapBits, Nat.toUInt64, UInt64.ofNat]
    <;> bv_decide
@[csimp] theorem rev64_eq_rev64Fast : @rev64 = @rev64Fast := by
  funext sf x
  cases sf <;> simp only [rev64, rev64Fast, revBytes, swapBits, Nat.toUInt64, UInt64.ofNat]
    <;> bv_decide

def dp1Val (sf : Bool) (opc : UInt8) (xn : UInt64) : UInt64 :=
  let x := mask32 sf xn
  match opc with
  | 0 => rbit sf x
  | 1 => rev16 sf x
  | 2 => rev32 sf x
  | 3 => rev64 sf x
  | 4 => clz sf x
  | _ => -- CLS: leading bits equal to the sign bit, not counting it
    let s := (x >>> ((if sf then 64 else 32) - 1)) &&& 1
    clz sf (mask32 sf (if s == 1 then ~~~x else x)) - 1

def dp2Val (sf : Bool) (opc : UInt8) (xn xm : UInt64) : UInt64 :=
  let w : UInt64 := if sf then 64 else 32
  let x := mask32 sf xn
  let y := mask32 sf xm
  let v := match opc with
    | 2 => if y == 0 then 0 else x / y
    | 3 =>
      if y == 0 then 0 else
      if sf then (x.toInt64 / y.toInt64).toUInt64
      else ((x.toUInt32.toInt32 / y.toUInt32.toInt32).toUInt32).toUInt64
    | 8 => mask32 sf (x <<< (y % w))
    | 9 => x >>> (y % w)
    | 10 => shiftReg sf x 2 (y % w)
    | _ => ror sf x (y % w)
  mask32 sf v

@[inline] def mullVal (u sub : Bool) (xn xm xa : UInt64) : UInt64 :=
  let x := if u then xn &&& 0xFFFF_FFFF else sext (xn &&& 0xFFFF_FFFF) 32
  let y := if u then xm &&& 0xFFFF_FFFF else sext (xm &&& 0xFFFF_FFFF) 32
  let p := x * y
  if sub then xa - p else xa + p

end Leanbox
