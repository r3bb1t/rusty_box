import Leanbox.Cpu.Exec
import Leanbox.Cpu.Simd
/-
`execOp`: one op at `pc`, completely, as rusty_box's handlers run it
(cpu/dp.rs, cpu/ldst.rs, cpu/isa/control.rs and the classes exec.rs
`exec_class` dispatches to). The hot loop's fast paths agree with this.
-/
namespace Leanbox


def execLdst (st : St) (pc : UInt64) (size opc : UInt8) (wb post : Bool) (kind t n : UInt8) (off : UInt64) : St :=
  match baseReg st n with
  | .error f => raise st pc f
  | .ok base =>
  let addr := if post then base else base + off
  let acc : Acc := { unpriv := kind == 1 }
  let fin (st : St) : St := nx (if wb then setR st n (base + off) else st) pc
  if opc == 0 then
    let (st, f) := writeMem st addr size (rr st.rf t) acc
    match f with | some f => raise st pc f | none => fin st
  else
    let (st, r) := readMem st addr size acc
    match r with
    | .error f => raise st pc f
    | .ok v =>
      let bits : UInt64 := 8 <<< size.toUInt64
      let v := if opc == 1 then v else if opc == 2 then sext v bits else sext v bits &&& 0xFFFF_FFFF
      fin (setR st t v)

def execPair (st : St) (pc : UInt64) (kind : UInt8) (load wb post : Bool) (t t2 n : UInt8) (off : UInt64) : St :=
  match baseReg st n with
  | .error f => raise st pc f
  | .ok base =>
  let addr := if post then base else base + off
  let log2 : UInt8 := if kind == 2 then 3 else 2
  let fin (st : St) : St := nx (if wb then setR st n (base + off) else st) pc
  if load then
    let (st, r) := readPair st addr log2 {}
    match r with
    | .error f => raise st pc f
    | .ok (a, b) =>
      let (a, b) := if kind == 1 then (sext a 32, sext b 32) else (a, b)
      let st := if log2 == 2 then setR (setR st t2 b) t a else setR (setR st t a) t2 b
      fin st
  else
    let (st, f) := writePair st addr log2 (rr st.rf t) (rr st.rf t2) {}
    match f with | some f => raise st pc f | none => fin st

/-- ldst.rs `exec_ldst_exclusive`. -/
def execExcl (st : St) (pc : UInt64) (kind size t n s : UInt8) : St :=
  match baseReg st n with
  | .error f => raise st pc f
  | .ok addr =>
  let acc : Acc := { aligned := true }
  if kind == 2 then                                          -- LDAR
    let (st, r) := readMem st addr size acc
    match r with | .error f => raise st pc f | .ok v => nx (setR st t v) pc
  else if kind == 3 then                                     -- STLR
    let (st, f) := writeMem st addr size (rr st.rf t) acc
    match f with | some f => raise st pc f | none => nx st pc
  else
  let total : UInt64 := 1 <<< size.toUInt64
  if kind == 0 then                                          -- LDXR
    let (st, r) := dataXlate st addr total false acc (addr &&& (total - 1) != 0)
    match r with
    | .error f => raise st pc f
    | .ok pa =>
      let (st, v) := busRead st pa size
      match v with
      | none => raise st pc (externalAbort st.sys addr false)
      | some v =>
        let st := setR st t v
        nx { st with sys := { st.sys with monArmed := true, monAddr := pa, monVa := addr, monBytes := total } } pc
  else                                                       -- STXR
    match checkAlignment st.sys addr total acc true with
    | some f => raise st pc f
    | none =>
    let m := st.sys
    let (st, passed) : St × Except Fault (Option UInt64) :=
      if m.monArmed && m.monVa == addr && m.monBytes == total then
        let (st, r) := dataXlate st addr total true acc (addr &&& (total - 1) != 0)
        match r with
        | .error f => (st, .error f)
        | .ok pa => (st, .ok (if m.monAddr == pa then some pa else none))
      else (st, .ok none)
    match passed with
    | .error f => raise st pc f
    | .ok passed =>
    let st := { st with sys := { st.sys with monArmed := false } }
    match passed with
    | some pa =>
      let (st, ok) := busWrite st pa size (rr st.rf t)
      if !ok then raise st pc (externalAbort st.sys addr true) else nx (setR st s 0) pc
    | none => nx (setR st s 1) pc

/-- LDXP/STXP (ldst.rs `exec_ldst_exclusive`, pair). -/
def execExclPair (st : St) (pc : UInt64) (load sf : Bool) (t t2 n s : UInt8) : St :=
  match baseReg st n with
  | .error f => raise st pc f
  | .ok addr =>
  let acc : Acc := { aligned := true }
  let totalLog2 : UInt8 := if sf then 4 else 3
  let total : UInt64 := 1 <<< totalLog2.toUInt64
  if load then
    let (st, r) := dataXlate st addr total false acc (addr &&& (total - 1) != 0)
    match r with
    | .error f => raise st pc f
    | .ok pa =>
      let (st, a) := busRead st pa 3
      match a with
      | none => raise st pc (externalAbort st.sys addr false)
      | some a =>
      if !sf then
        let st := setR (setR st t (a &&& 0xFFFF_FFFF)) t2 (a >>> 32)
        nx { st with sys := { st.sys with monArmed := true, monAddr := pa, monVa := addr, monBytes := total } } pc
      else
      let (st, b) := busRead st (pa + 8) 3
      match b with
      | none => raise st pc (externalAbort st.sys addr false)
      | some b =>
        let st := setR (setR st t a) t2 b
        nx { st with sys := { st.sys with monArmed := true, monAddr := pa, monVa := addr, monBytes := total } } pc
  else
    match checkAlignment st.sys addr total acc true with
    | some f => raise st pc f
    | none =>
    let m := st.sys
    let (st, passed) : St × Except Fault (Option UInt64) :=
      if m.monArmed && m.monVa == addr && m.monBytes == total then
        let (st, r) := dataXlate st addr total true acc (addr &&& (total - 1) != 0)
        match r with
        | .error f => (st, .error f)
        | .ok pa => (st, .ok (if m.monAddr == pa then some pa else none))
      else (st, .ok none)
    match passed with
    | .error f => raise st pc f
    | .ok passed =>
    let st := { st with sys := { st.sys with monArmed := false } }
    match passed with
    | some pa =>
      if !sf then
        let v := (rr st.rf t2 <<< 32) ||| (rr st.rf t &&& 0xFFFF_FFFF)
        let (st, ok) := busWrite st pa 3 v
        if !ok then raise st pc (externalAbort st.sys addr true) else nx (setR st s 0) pc
      else
        let (st, ok) := busWrite st pa 3 (rr st.rf t)
        if !ok then raise st pc (externalAbort st.sys addr true) else
        let (st, ok) := busWrite st (pa + 8) 3 (rr st.rf t2)
        if !ok then raise st pc (externalAbort st.sys addr true) else nx (setR st s 0) pc
    | none => nx (setR st s 1) pc

/-- A SIMD&FP register load or store of `size` 0..4 at `addr`. -/
def fpAccess (st : St) (addr : UInt64) (size : UInt8) (load : Bool) (t : UInt8) : St × Option Fault :=
  if size == 4 then
    if load then
      match checkAlignment st.sys addr 16 {} false with
      | some f => (st, some f)
      | none =>
        let (st, r) := readPair st addr 3 {}
        match r with
        | .error f => (st, some f)
        | .ok (a, b) => (setV st t a b, none)
    else
      match checkAlignment st.sys addr 16 {} true with
      | some f => (st, some f)
      | none => writePair st addr 3 (vlo st.rf t) (vhi st.rf t) {}
  else if load then
    let (st, r) := readMem st addr size {}
    match r with
    | .error f => (st, some f)
    | .ok v => (setV st t v 0, none)
  else writeMem st addr size (vlo st.rf t &&& valueMask size) {}

def execOp (st : St) (pc : UInt64) (op : Op) : St :=
  let rf := st.rf
  let r (s : UInt8) : UInt64 := rr rf s
  match op with
  | .addR64 d n m => nx (setR st d (r n + r m)) pc
  | .subR64 d n m => nx (setR st d (r n - r m)) pc
  | .addR32 d n m => nx (setR st d ((r n + r m) &&& 0xFFFF_FFFF)) pc
  | .logR64 opc sh d n m amt => nx (setR st d (logic opc (r n) (shift64 (r m) sh amt))) pc
  | .movR d m => nx (setR st d (r m)) pc
  | .subsI64 d n imm =>
    let x := r n
    nx { setR st d (x - imm) with nzcv := awcFlags true x (~~~imm) 1 } pc
  | .addI64 d n imm => nx (setR st d (r n + imm)) pc
  | .logI64 opc d n mask => nx (setR st d (logic opc (r n) mask)) pc
  | .movc d v => nx (setR st d v) pc
  | .madd64 d n m a => nx (setR st d (r a + r n * r m)) pc
  | .adr page d off =>
    nx (setR st d (if page then (pc &&& ~~~0xFFF) + off else pc + off)) pc
  | .addsubI sf sub setf d n imm =>
    let x := mask32 sf (r n)
    let y := if sub then mask32 sf (~~~imm) else imm
    let c : UInt64 := if sub then 1 else 0
    let st := setR st d (mask32 sf (x + y + c))
    nx (if setf then { st with nzcv := awcFlags sf x y c } else st) pc
  | .logI sf opc d n mask =>
    let res := mask32 sf (logic opc (mask32 sf (r n)) mask)
    let st := setR st d res
    nx (if opc == 3 then { st with nzcv := logicFlags sf res } else st) pc
  | .movk sf d imm pos =>
    nx (setR st d (mask32 sf ((r d &&& ~~~((0xFFFF : UInt64) <<< pos)) ||| (imm <<< pos)))) pc
  | .bfm sf opc d n rr_ s wmask tmask =>
    let src := r n
    let bot := ror sf src rr_ &&& wmask
    let res := match opc with
      | 0 => ((if (src >>> s) &&& 1 == 1 then MISS else 0) &&& ~~~tmask) ||| (bot &&& tmask)
      | 1 => let dst := r d
             (dst &&& ~~~tmask) ||| (((dst &&& ~~~wmask) ||| bot) &&& tmask)
      | _ => bot &&& tmask
    nx (setR st d (mask32 sf res)) pc
  | .extr sf d n m lsb =>
    let res := if sf then (if lsb == 0 then r m else (r m >>> lsb) ||| (r n <<< (64 - lsb)))
      else
        let x := r n &&& 0xFFFF_FFFF
        let y := r m &&& 0xFFFF_FFFF
        (((x <<< 32) ||| y) >>> lsb) &&& 0xFFFF_FFFF
    nx (setR st d res) pc
  | .alu sf sub setf d n m sh amt =>
    let x := mask32 sf (r n)
    let y := mask32 sf (shiftReg sf (r m) sh amt)
    let y := if sub then mask32 sf (~~~y) else y
    let c : UInt64 := if sub then 1 else 0
    let st := setR st d (mask32 sf (x + y + c))
    nx (if setf then { st with nzcv := awcFlags sf x y c } else st) pc
  | .aluX sf sub setf d n m opt amt =>
    let x := mask32 sf (r n)
    let y := extendReg (r m) opt amt (if sf then 64 else 32)
    let y := if sub then mask32 sf (~~~y) else y
    let c : UInt64 := if sub then 1 else 0
    let st := setR st d (mask32 sf (x + y + c))
    nx (if setf then { st with nzcv := awcFlags sf x y c } else st) pc
  | .adc sf sub setf d n m =>
    let x := mask32 sf (r n)
    let y := if sub then mask32 sf (~~~(r m)) else mask32 sf (r m)
    let c := (st.nzcv >>> 1) &&& 1
    let st := setR st d (mask32 sf (x + y + c))
    nx (if setf then { st with nzcv := awcFlags sf x y c } else st) pc
  | .logR sf opc inv d n m sh amt =>
    let y := shiftReg sf (r m) sh amt
    let y := mask32 sf (if inv then ~~~y else y)
    let res := mask32 sf (logic opc (r n) y)
    let st := setR st d res
    nx (if opc == 3 then { st with nzcv := logicFlags sf res } else st) pc
  | .ccmp sf sub isImm n m imm cond flags =>
    if conditionHolds cond st.nzcv then
      let x := mask32 sf (r n)
      let y := if isImm then imm else mask32 sf (r m)
      let y := if sub then mask32 sf (~~~y) else y
      nx { st with nzcv := awcFlags sf x y (if sub then 1 else 0) } pc
    else nx { st with nzcv := flags.toUInt64 } pc
  | .csel sf inv inc d n m cond =>
    let v := if conditionHolds cond st.nzcv then r n
      else
        let y := r m
        let y := if inv then ~~~y else y
        if inc then y + 1 else y
    nx (setR st d (mask32 sf v)) pc
  | .dp1 sf opc d n =>
    let w : UInt64 := if sf then 64 else 32
    let x := mask32 sf (r n)
    let v := match opc with
      | 0 => rbit x w
      | 1 => revBytes x w 2
      | 2 => if sf then revBytes x w 4 else revBytes x w 4
      | 3 => revBytes x w 8
      | 4 => clz64 x w
      | _ => -- CLS: leading bits equal to the sign bit, not counting it
        let s := (x >>> (w - 1)) &&& 1
        let y := mask32 sf (if s == 1 then ~~~x else x)
        clz64 y w - 1
    nx (setR st d v) pc
  | .dp2 sf opc d n m =>
    let w : UInt64 := if sf then 64 else 32
    let x := mask32 sf (r n)
    let y := mask32 sf (r m)
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
    nx (setR st d (mask32 sf v)) pc
  | .crc c sz d n m =>
    let bytes : UInt64 := 1 <<< sz.toUInt64
    nx (setR st d (crc32 (r n) (r m &&& ones (8 * bytes)) bytes c)) pc
  | .madd sf sub d n m a =>
    let p := r n * r m
    nx (setR st d (mask32 sf (if sub then r a - p else r a + p))) pc
  | .mull u sub d n m a =>
    let x := if u then r n &&& 0xFFFF_FFFF else sext (r n &&& 0xFFFF_FFFF) 32
    let y := if u then r m &&& 0xFFFF_FFFF else sext (r m &&& 0xFFFF_FFFF) 32
    let p := x * y
    nx (setR st d (if sub then r a - p else r a + p)) pc
  | .mulh u d n m => nx (setR st d (mulHigh (r n) (r m) (!u))) pc
  | .bcond cond off =>
    if conditionHolds cond st.nzcv then jmp st (pc + off) else nx st pc
  | .b link off =>
    let st := if link then setR st 30 (pc + 4) else st
    jmp st (pc + off)
  | .br kind n =>
    let target := r n
    let st := if kind == 1 then setR st 30 (pc + 4) else st
    jmp st target
  | .cbz sf nz t off =>
    let z := mask32 sf (r t) == 0
    if z != nz then jmp st (pc + off) else nx st pc
  | .tbz nz t bit off =>
    let set := (r t >>> bit) &&& 1 == 1
    if set == nz then jmp st (pc + off) else nx st pc
  | .ldst size opc wb post kind t n off => execLdst st pc size opc wb post kind t n off
  | .ldrbPost t n off => execLdst st pc 0 1 true true 0 t n off
  | .ldstR size opc t n m opt shift =>
    match baseReg st n with
    | .error f => raise st pc f
    | .ok base =>
    let addr := base + extendReg (r m) opt shift 64
    if opc == 0 then
      let (st, f) := writeMem st addr size (r t) {}
      match f with | some f => raise st pc f | none => nx st pc
    else
      let (st, rv) := readMem st addr size {}
      match rv with
      | .error f => raise st pc f
      | .ok v =>
        let bits : UInt64 := 8 <<< size.toUInt64
        let v := if opc == 1 then v else if opc == 2 then sext v bits else sext v bits &&& 0xFFFF_FFFF
        nx (setR st t v) pc
  | .ldLit size signed t off =>
    let (st, rv) := readMem st (pc + off) size {}
    match rv with
    | .error f => raise st pc f
    | .ok v => nx (setR st t (if signed then sext v 32 else v)) pc
  | .ldstPair kind load wb post t t2 n off => execPair st pc kind load wb post t t2 n off
  | .ldpPost64 t t2 n off => execPair st pc 2 true true true t t2 n off
  | .stpPost64 t t2 n off => execPair st pc 2 false true true t t2 n off
  | .excl kind size t n s => execExcl st pc kind size t n s
  | .exclPair load sf t t2 n s => execExclPair st pc load sf t t2 n s
  | .fldst size load wb post t n off =>
    if !fpEnabled st.sys then raise st pc fpTrapF else
    match baseReg st n with
    | .error f => raise st pc f
    | .ok base =>
    let addr := if post then base else base + off
    let (st, f) := fpAccess st addr size load t
    match f with
    | some f => raise st pc f
    | none => nx (if wb then setR st n (base + off) else st) pc
  | .fldstR size load t n m opt shift =>
    if !fpEnabled st.sys then raise st pc fpTrapF else
    match baseReg st n with
    | .error f => raise st pc f
    | .ok base =>
    let (st, f) := fpAccess st (base + extendReg (r m) opt shift 64) size load t
    match f with | some f => raise st pc f | none => nx st pc
  | .fldLit size t off =>
    if !fpEnabled st.sys then raise st pc fpTrapF else
    let (st, f) := fpAccess st (pc + off) size true t
    match f with | some f => raise st pc f | none => nx st pc
  | .fldstPair size load wb post t t2 n off =>
    if !fpEnabled st.sys then raise st pc fpTrapF else
    match baseReg st n with
    | .error f => raise st pc f
    | .ok base =>
    let addr := if post then base else base + off
    let bytes : UInt64 := 1 <<< size.toUInt64
    let fin (st : St) : St := nx (if wb then setR st n (base + off) else st) pc
    if size == 4 then
      if load then
        let (st, f) := fpAccess st addr 4 true t
        match f with
        | some f => raise st pc f
        | none =>
          let (st, f) := fpAccess st (addr + 16) 4 true t2
          match f with | some f => raise st pc f | none => fin st
      else
        let (st, f) := fpAccess st addr 4 false t
        match f with
        | some f => raise st pc f
        | none =>
          let (st, f) := fpAccess st (addr + 16) 4 false t2
          match f with | some f => raise st pc f | none => fin st
    else if load then
      let (st, rv) := readPair st addr size {}
      match rv with
      | .error f => raise st pc f
      | .ok (a, b) => fin (setV (setV st t a 0) t2 b 0)
    else
      let m := valueMask size
      let (st, f) := writePair st addr size (vlo st.rf t &&& m) (vlo st.rf t2 &&& m) {}
      let _ := bytes
      match f with | some f => raise st pc f | none => fin st
  | .vldst w => execVldst st pc w
  | .nop => nx st pc
  | .readCounter virt t =>
    let el0 := st.sys.el == 0
    let en := if virt then EL0VCTEN else EL0PCTEN
    if el0 && st.sys.cntkctl &&& en == 0 then
      raise st pc { ec := EC_SYSREG, iss := trapIss (((if virt then K_CNTVCT else K_CNTPCT) <<< 5) ||| (if t == SINK then 31 else t.toUInt32)) true }
    else nx (setR st t (counterAt st.icnt)) pc
  | .sys w => execSys st pc w
  | .simd w => execSimd st pc w
  | .undef _ => raise st pc undefinedF
  | .endTrace => st

end Leanbox
