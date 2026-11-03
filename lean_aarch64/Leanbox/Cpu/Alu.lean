import Leanbox.Cpu.Exec
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

def dp1Val (sf : Bool) (opc : UInt8) (xn : UInt64) : UInt64 :=
  let w : UInt64 := if sf then 64 else 32
  let x := mask32 sf xn
  match opc with
  | 0 => rbit x w
  | 1 => revBytes x w 2
  | 2 => revBytes x w 4
  | 3 => revBytes x w 8
  | 4 => clz64 x w
  | _ => -- CLS: leading bits equal to the sign bit, not counting it
    let s := (x >>> (w - 1)) &&& 1
    let y := mask32 sf (if s == 1 then ~~~x else x)
    clz64 y w - 1

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
