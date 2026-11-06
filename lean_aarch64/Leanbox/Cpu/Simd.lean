import Leanbox.Cpu.Memory
import Leanbox.Cpu.Sysreg
import Leanbox.Isa.Decode
/-!
# SIMD and floating point

SIMD&FP: the structure loads and stores (ldst.rs `exec_ldst_multiple`,
`exec_ldst_single_structure`) and the Advanced SIMD and FP data processing
this machine executes so far (cpu/simd.rs, cpu/simd_arith.rs, cpu/fp.rs).
A SIMD&FP data-processing word outside that set stops the machine as
unmodelled rather than being taken for UNDEFINED.
-/
namespace Leanbox

/-- A 128-bit SIMD&FP value. -/
structure V128 where
  lo : UInt64
  hi : UInt64
  deriving Inhabited, BEq

@[inline] def getV (rf : ByteArray) (t : UInt8) : V128 := ⟨vlo rf t, vhi rf t⟩
@[inline] def putV (rf : ByteArray) (t : UInt8) (v : V128) : ByteArray := wv rf t v.lo v.hi

/-- Element `i` of `1 << log2` bytes. -/
@[inline] def V128.elem (v : V128) (log2 : UInt64) (i : UInt64) : UInt64 :=
  let bits := 8 <<< log2
  let pos := i * bits
  let w := if pos < 64 then v.lo >>> pos else v.hi >>> (pos - 64)
  w &&& ones bits

@[inline] def V128.setElem (v : V128) (log2 : UInt64) (i : UInt64) (x : UInt64) : V128 :=
  let bits := 8 <<< log2
  let pos := i * bits
  let m := ones bits
  if pos < 64 then { v with lo := (v.lo &&& ~~~(m <<< pos)) ||| ((x &&& m) <<< pos) }
  else { v with hi := (v.hi &&& ~~~(m <<< (pos - 64))) ||| ((x &&& m) <<< (pos - 64)) }

/-- The elements of an operation of `q` (128 bits) or not (64). -/
@[inline] def elems (q : Bool) (log2 : UInt64) : UInt64 := (if q then 16 else 8) >>> log2

def mapV (q : Bool) (log2 : UInt64) (f : UInt64 → UInt64) (a : V128) : V128 := Id.run do
  let mut r : V128 := ⟨0, 0⟩
  for i in [0:(elems q log2).toNat] do
    r := r.setElem log2 i.toUInt64 (f (a.elem log2 i.toUInt64))
  return r

def zipV (q : Bool) (log2 : UInt64) (f : UInt64 → UInt64 → UInt64) (a b : V128) : V128 := Id.run do
  let mut r : V128 := ⟨0, 0⟩
  for i in [0:(elems q log2).toNat] do
    r := r.setElem log2 i.toUInt64 (f (a.elem log2 i.toUInt64) (b.elem log2 i.toUInt64))
  return r

-- ── structure loads and stores ────────────────────────────────────────
/-- ldst.rs `exec_ldst_multiple` and `exec_ldst_single_structure`. -/
def execVldst (st : St) (pc : UInt64) (w : UInt32) : St :=
  let q := fld w 30 1 == 1
  let post := fld w 23 1 == 1
  let load := fld w 22 1 == 1
  let rm := r5 w 16
  let rn := r5 w 5
  let rt := r5 w 0
  if !post && rm != 0 then raise st pc undefinedF else
  let fin (st : St) (offs : UInt64) (base : UInt64) : St :=
    if post then
      let step := if rm == 31 then offs else rr st.rf rm
      nx (setR st rn (base + step)) pc
    else nx st pc
  if fld w 24 1 == 0 then
    -- multiple structures
    let opcode := fld w 12 4
    let size := fld64 w 10 2
    let rs : Option (Nat × Nat) := match opcode with
      | 0 => some (1, 4) | 2 => some (4, 1) | 4 => some (1, 3) | 6 => some (3, 1)
      | 7 => some (1, 1) | 8 => some (1, 2) | 10 => some (2, 1) | _ => none
    match rs with
    | none => raise st pc undefinedF
    | some (rpt, selem) =>
    if size == 3 && !q && selem != 1 then raise st pc undefinedF else
    if !fpEnabled st.sys then raise st pc fpTrapF else
    match baseReg st rn with
    | .error f => raise st pc f
    | .ok base =>
    let ebytes : UInt64 := 1 <<< size
    let n := (elems q size).toNat
    -- the registers' values are worked on as copies, written at the end
    let rec go (st : St) (regs : Array V128) (r e s : Nat) (offs : UInt64) (fuel : Nat) :
        St × Except Fault (Array V128 × UInt64) :=
      match fuel with
      | 0 => (st, .ok (regs, offs))
      | fuel + 1 =>
      if r ≥ rpt then (st, .ok (regs, offs)) else
      if e ≥ n then go st regs (r + 1) 0 0 offs fuel else
      if s ≥ selem then go st regs r (e + 1) 0 offs fuel else
      let tt := (rt.toNat + r + s) % 32
      let ea := base + offs
      if load then
        let (st, v) := readMem st ea size.toUInt8 {}
        match v with
        | .error f => (st, .error f)
        | .ok v => go st (regs.set! tt ((regs[tt]!).setElem size e.toUInt64 v)) r e (s + 1) (offs + ebytes) fuel
      else
        let v := (regs[tt]!).elem size e.toUInt64
        let (st, f) := writeMem st ea size.toUInt8 v {}
        match f with
        | some f => (st, .error f)
        | none => go st regs r e (s + 1) (offs + ebytes) fuel
    let regs := (Array.range 32).map fun i => getV st.rf i.toUInt8
    let (st, res) := go st regs 0 0 0 0 (rpt * n * selem + rpt * n + rpt + 2)
    match res with
    | .error f => raise st pc f
    | .ok (regs, offs) =>
      let st := if load then Id.run do
          let mut st := st
          for k in [0:rpt * selem] do
            let t := (rt.toNat + k) % 32
            let v := regs[t]!
            st := { st with rf := putV st.rf t.toUInt8 (if q then v else ⟨v.lo, 0⟩) }
          return st
        else st
      fin st offs base
  else
    -- single structure
    let r := fld w 21 1
    let opcode := fld w 13 3
    let s := fld w 12 1
    let size := fld w 10 2
    let selem := (((opcode &&& 1) <<< 1) ||| r) + 1
    let qs := ((if q then (1 : UInt32) else 0) <<< 1) ||| s
    let sel : Option (UInt64 × UInt64 × Bool) := match opcode >>> 1 with
      | 0 => some (0, ((qs <<< 2) ||| size).toUInt64, false)
      | 1 => if size &&& 1 != 0 then none else some (1, ((qs <<< 1) ||| (size >>> 1)).toUInt64, false)
      | 2 => if size &&& 2 != 0 then none
             else if size &&& 1 == 0 then some (2, qs.toUInt64, false)
             else if s == 1 then none else some (3, (if q then 1 else 0), false)
      | _ => if !load || s == 1 then none else some (size.toUInt64, 0, true)
    match sel with
    | none => raise st pc undefinedF
    | some (scale, index, replicate) =>
    if !fpEnabled st.sys then raise st pc fpTrapF else
    match baseReg st rn with
    | .error f => raise st pc f
    | .ok base =>
    let ebytes : UInt64 := 1 <<< scale
    let rec go1 (st : St) (regs : Array V128) (k : Nat) (offs : UInt64) : St × Except Fault (Array V128 × UInt64) :=
      if k ≥ selem.toNat then (st, .ok (regs, offs)) else
      let t := (rt.toNat + k) % 32
      let ea := base + offs
      if replicate then
        let (st, v) := readMem st ea scale.toUInt8 {}
        match v with
        | .error f => (st, .error f)
        | .ok v =>
          let full := Id.run do
            let mut x : V128 := ⟨0, 0⟩
            for i in [0:(16 >>> scale.toNat)] do
              x := x.setElem scale i.toUInt64 v
            return x
          go1 st (regs.set! t (if q then full else ⟨full.lo, 0⟩)) (k + 1) (offs + ebytes)
      else if load then
        let (st, v) := readMem st ea scale.toUInt8 {}
        match v with
        | .error f => (st, .error f)
        | .ok v => go1 st (regs.set! t ((regs[t]!).setElem scale index v)) (k + 1) (offs + ebytes)
      else
        let (st, f) := writeMem st ea scale.toUInt8 ((regs[t]!).elem scale index) {}
        match f with
        | some f => (st, .error f)
        | none => go1 st regs (k + 1) (offs + ebytes)
    termination_by selem.toNat - k
    let regs := (Array.range 32).map fun i => getV st.rf i.toUInt8
    let (st, res) := go1 st regs 0 0
    match res with
    | .error f => raise st pc f
    | .ok (regs, offs) =>
      let st := if load then Id.run do
          let mut st := st
          for k in [0:selem.toNat] do
            let t := (rt.toNat + k) % 32
            st := { st with rf := putV st.rf t.toUInt8 regs[t]! }
          return st
        else st
      fin st offs base

-- ── Advanced SIMD data processing ─────────────────────────────────────
/-- Arm `AdvSIMDExpandImm(op, cmode, imm8)`. -/
def advSimdExpandImm (op : Bool) (cmode : UInt64) (imm8 : UInt64) : UInt64 :=
  let rep32 (x : UInt64) : UInt64 := (x <<< 32) ||| x
  let rep16 (x : UInt64) : UInt64 := let y := (x <<< 16) ||| x; (y <<< 32) ||| y
  match cmode >>> 1 with
  | 0 => rep32 imm8
  | 1 => rep32 (imm8 <<< 8)
  | 2 => rep32 (imm8 <<< 16)
  | 3 => rep32 (imm8 <<< 24)
  | 4 => rep16 imm8
  | 5 => rep16 (imm8 <<< 8)
  | 6 => if cmode &&& 1 == 0 then rep32 ((imm8 <<< 8) ||| 0xFF) else rep32 ((imm8 <<< 16) ||| 0xFFFF)
  | _ =>
    if cmode &&& 1 == 0 && !op then
      let b := imm8; let y := (b <<< 8) ||| b; let y := (y <<< 16) ||| y; (y <<< 32) ||| y
    else if cmode &&& 1 == 0 && op then Id.run do
      let mut r : UInt64 := 0
      for i in [0:8] do
        if (imm8 >>> i.toUInt64) &&& 1 == 1 then r := r ||| ((0xFF : UInt64) <<< (8 * i.toUInt64))
      return r
    else if !op then
      -- single-precision FMOV immediate, replicated
      let a := (imm8 >>> 7) &&& 1
      let b := (imm8 >>> 6) &&& 1
      let c := imm8 &&& 0x3F
      let s := (a <<< 31) ||| ((b ^^^ 1) <<< 30) ||| ((if b == 1 then (0x1F : UInt64) else 0) <<< 25) ||| (c <<< 19)
      rep32 s
    else
      let a := (imm8 >>> 7) &&& 1
      let b := (imm8 >>> 6) &&& 1
      let c := imm8 &&& 0x3F
      (a <<< 63) ||| ((b ^^^ 1) <<< 62) ||| ((if b == 1 then (0xFF : UInt64) else 0) <<< 54) ||| (c <<< 48)

/-- Signed comparison of `bits`-bit elements. -/
@[inline] def sgt (bits a b : UInt64) : Bool := (sext a bits).toInt64 > (sext b bits).toInt64
@[inline] def sge (bits a b : UInt64) : Bool := (sext a bits).toInt64 >= (sext b bits).toInt64

/-- The unsigned integer `x` as an IEEE value of `ebits` exponent and
`fbits` fraction bits, rounded by `rmode` (0 RN, 1 RP, 2 RM, 3 RZ): the bit
pattern and whether it was inexact. -/
def intToFloat (neg : Bool) (x : Nat) (ebits fbits : Nat) (rmode : UInt64) : UInt64 × Bool :=
  let sign : UInt64 := if neg then 1 else 0
  let signBit := sign <<< (ebits + fbits).toUInt64
  if x == 0 then (signBit, false) else
  let msb := Nat.log2 x
  let bias := (1 <<< (ebits - 1)) - 1
  if msb ≤ fbits then
    let mant := (x <<< (fbits - msb)) - (1 <<< fbits)
    (signBit ||| ((msb + bias).toUInt64 <<< fbits.toUInt64) ||| mant.toUInt64, false)
  else
    let sh := msb - fbits
    let q := x >>> sh
    let rem := x - (q <<< sh)
    let half := 1 <<< (sh - 1)
    let up := match rmode with
      | 0 => rem > half || (rem == half && q % 2 == 1)
      | 1 => rem != 0 && !neg
      | 2 => rem != 0 && neg
      | _ => false
    let q := if up then q + 1 else q
    let (q, e) := if q == (1 <<< (fbits + 1)) then (q >>> 1, msb + 1) else (q, msb)
    let mant := q - (1 <<< fbits)
    (signBit ||| ((e + bias).toUInt64 <<< fbits.toUInt64) ||| mant.toUInt64, rem != 0)

-- ── the Cryptographic Extension's SHA-256 (Arm ARM `SHA256hash`) ───────
@[inline] def ror32 (x : UInt64) (r : UInt64) : UInt64 :=
  ((x >>> r) ||| (x <<< (32 - r))) &&& 0xFFFF_FFFF
@[inline] def lane32 (v : V128) (e : UInt64) : UInt64 := v.elem 2 e

/-- Arm `SHA256hash(X, Y, W, part1)`. -/
def sha256hash (x y w : V128) (part1 : Bool) : V128 := Id.run do
  let mut x := x
  let mut y := y
  for e in [0:4] do
    let chs := (((lane32 y 1) ^^^ (lane32 y 2)) &&& (lane32 y 0)) ^^^ (lane32 y 2)
    let maj := ((lane32 x 0) &&& (lane32 x 1)) ||| (((lane32 x 0) ||| (lane32 x 1)) &&& (lane32 x 2))
    let y0 := lane32 y 0
    let sig1 := ror32 y0 6 ^^^ ror32 y0 11 ^^^ ror32 y0 25
    let t := (lane32 y 3 + sig1 + chs + lane32 w e.toUInt64) &&& 0xFFFF_FFFF
    x := x.setElem 2 3 (t + lane32 x 3)
    let x0 := lane32 x 0
    let sig0 := ror32 x0 2 ^^^ ror32 x0 13 ^^^ ror32 x0 22
    y := y.setElem 2 3 (t + sig0 + maj)
    -- <Y, X> = ROL(Y : X, 32)
    let xTop := lane32 x 3
    let yTop := lane32 y 3
    x := ⟨(x.lo <<< 32) ||| yTop, (x.hi <<< 32) ||| (x.lo >>> 32)⟩
    y := ⟨(y.lo <<< 32) ||| xTop, (y.hi <<< 32) ||| (y.lo >>> 32)⟩
  return if part1 then x else y

/-- Arm `SHA256SU0`. -/
def sha256su0 (d n : V128) : V128 := Id.run do
  -- T = operand2<31:0> : operand1<127:32>
  let t : V128 := ⟨(d.lo >>> 32) ||| (d.hi <<< 32), (d.hi >>> 32) ||| (n.lo <<< 32)⟩
  let mut r : V128 := ⟨0, 0⟩
  for e in [0:4] do
    let el := lane32 t e.toUInt64
    let el := ror32 el 7 ^^^ ror32 el 18 ^^^ (el >>> 3)
    r := r.setElem 2 e.toUInt64 (el + lane32 d e.toUInt64)
  return r

/-- Arm `SHA256SU1`. -/
def sha256su1 (d n m : V128) : V128 := Id.run do
  -- T0 = operand3<31:0> : operand2<127:32>; T1 = operand3<127:64>
  let t0 : V128 := ⟨(n.lo >>> 32) ||| (n.hi <<< 32), (n.hi >>> 32) ||| (m.lo <<< 32)⟩
  let mut r : V128 := ⟨0, 0⟩
  let mut t1 : UInt64 := m.hi
  for e in [0:2] do
    let el := (t1 >>> (32 * e.toUInt64)) &&& 0xFFFF_FFFF
    let el := ror32 el 17 ^^^ ror32 el 19 ^^^ (el >>> 10)
    r := r.setElem 2 e.toUInt64 (el + lane32 d e.toUInt64 + lane32 t0 e.toUInt64)
  t1 := r.lo
  for e in [2:4] do
    let el := (t1 >>> (32 * (e - 2).toUInt64)) &&& 0xFFFF_FFFF
    let el := ror32 el 17 ^^^ ror32 el 19 ^^^ (el >>> 10)
    r := r.setElem 2 e.toUInt64 (el + lane32 d e.toUInt64 + lane32 t0 e.toUInt64)
  return r

/-- Carry-less multiplication of `bits`-bit `a` and `b` (Arm
`PolynomialMult`): the low and high 64 bits of the product. -/
def clmul (a b : UInt64) (bits : Nat) : UInt64 × UInt64 := Id.run do
  let mut lo : UInt64 := 0
  let mut hi : UInt64 := 0
  for i in [0:bits] do
    if (b >>> i.toUInt64) &&& 1 == 1 then
      lo := lo ^^^ (a <<< i.toUInt64)
      if i != 0 then hi := hi ^^^ (a >>> (64 - i.toUInt64))
  return (lo, hi)

def stopSimd (st : St) (pc : UInt64) (w : UInt32) : St :=
  raise st pc (unmodelled s!"SIMD&FP word {hex w.toUInt64} at {hex pc}")

/-- The SIMD&FP data-processing words this machine executes. -/
def execSimd (st : St) (pc : UInt64) (w : UInt32) : St :=
  if !fpEnabled st.sys then raise st pc fpTrapF else
  let rd := r5 w 0
  let rn := r5 w 5
  let rm := r5 w 16
  let q := fld w 30 1 == 1
  let u := fld w 29 1 == 1
  let size := fld64 w 22 2
  let a := getV st.rf rn
  let b := getV st.rf rm
  let setD (v : V128) : St := nx { st with rf := putV st.rf rd (if q then v else ⟨v.lo, 0⟩) } pc
  if w &&& 0x9FF8_0400 == 0x0F00_0400 then                  -- modified immediate
    let op := fld w 29 1 == 1
    let cmode := fld64 w 12 4
    let o2 := fld w 11 1
    if o2 == 1 then raise st pc undefinedF
    else if op && cmode == 15 && !q then raise st pc undefinedF
    else
    let imm8 := (fld64 w 16 3 <<< 5) ||| fld64 w 5 5
    let imm := advSimdExpandImm op cmode imm8
    let d := getV st.rf rd
    let immv : V128 := ⟨imm, imm⟩
    -- cmode 0xx1 / 10x1: ORR (op 0) or BIC (op 1); otherwise MOVI/MVNI/FMOV
    let orrbic := cmode < 12 && cmode &&& 1 == 1
    let r : V128 :=
      if orrbic then
        if op then ⟨d.lo &&& ~~~imm, d.hi &&& ~~~imm⟩ else ⟨d.lo ||| imm, d.hi ||| imm⟩
      else if op && cmode != 14 && cmode != 15 then ⟨~~~imm, ~~~imm⟩
      else immv
    setD r
  else if w &&& 0x9FE0_8400 == 0x0E00_0400 then              -- copy
    let imm5 := fld64 w 16 5
    let imm4 := fld w 11 4
    let op := fld w 29 1
    let log2 : UInt64 := if imm5 &&& 1 == 1 then 0 else if imm5 &&& 2 != 0 then 1
      else if imm5 &&& 4 != 0 then 2 else if imm5 &&& 8 != 0 then 3 else 4
    if log2 == 4 then raise st pc undefinedF else
    let idx := imm5 >>> (log2 + 1)
    if op == 1 then                                          -- INS (element)
      if !q then raise st pc undefinedF else
      let idx2 := fld64 w 11 4 >>> log2
      let d := getV st.rf rd
      nx { st with rf := putV st.rf rd (d.setElem log2 idx (a.elem log2 idx2)) } pc
    else if imm4 == 0 then                                   -- DUP (element)
      if log2 == 3 && !q then raise st pc undefinedF else
      setD (mapV true log2 (fun _ => a.elem log2 idx) ⟨0, 0⟩)
    else if imm4 == 1 then                                   -- DUP (general)
      if log2 == 3 && !q then raise st pc undefinedF else
      let x := rr st.rf (zr rn)
      setD (mapV true log2 (fun _ => x) ⟨0, 0⟩)
    else if imm4 == 3 then                                   -- INS (general)
      if !q then raise st pc undefinedF else
      let d := getV st.rf rd
      nx { st with rf := putV st.rf rd (d.setElem log2 idx (rr st.rf (zr rn))) } pc
    else if imm4 == 7 then                                   -- UMOV
      if (q && log2 != 3) || (!q && log2 == 3) then raise st pc undefinedF
      else nx (setR st (zw rd) (a.elem log2 idx)) pc
    else if imm4 == 5 then                                   -- SMOV
      if log2 == 3 || (!q && log2 == 2) then raise st pc undefinedF
      else
        let v := sext (a.elem log2 idx) (8 <<< log2)
        nx (setR st (zw rd) (if q then v else v &&& 0xFFFF_FFFF)) pc
    else raise st pc undefinedF
  else if w &&& 0x9F20_0400 == 0x0E20_0400 then              -- three same
    let opcode := fld w 11 5
    let bits : UInt64 := 8 <<< size
    let all := ones bits
    let cmp (f : UInt64 → UInt64 → Bool) : St :=
      if size == 3 && !q then raise st pc undefinedF
      else setD (zipV q size (fun x y => if f x y then all else 0) a b)
    if opcode == 3 then                                      -- logical
      let d := getV st.rf rd
      let f (x y z : UInt64) : UInt64 :=
        if !u then match size with
          | 0 => x &&& y | 1 => x &&& ~~~y | 2 => x ||| y | _ => x ||| ~~~y
        else match size with
          | 0 => x ^^^ y
          | 1 => (x &&& z) ||| (y &&& ~~~z)    -- BSL: Vd selects
          | 2 => (x &&& y) ||| (z &&& ~~~y)    -- BIT
          | _ => (z &&& y) ||| (x &&& ~~~y)    -- BIF
      setD ⟨f a.lo b.lo d.lo, f a.hi b.hi d.hi⟩
    else if opcode == 16 then                                -- ADD / SUB
      if size == 3 && !q then raise st pc undefinedF
      else setD (zipV q size (fun x y => if u then x - y else x + y) a b)
    else if opcode == 17 then                                -- CMTST / CMEQ
      cmp (fun x y => if u then x == y else x &&& y != 0)
    else if opcode == 6 then cmp (fun x y => if u then x > y else sgt bits x y)     -- CMHI / CMGT
    else if opcode == 7 then cmp (fun x y => if u then x >= y else sge bits x y)    -- CMHS / CMGE
    else if opcode == 23 && !u then                          -- ADDP
      if size == 3 && !q then raise st pc undefinedF else
      let n := elems q size
      let half := n / 2
      let r := Id.run do
        let mut r : V128 := ⟨0, 0⟩
        for i in [0:half.toNat] do
          let i := i.toUInt64
          r := r.setElem size i (a.elem size (2 * i) + a.elem size (2 * i + 1))
          r := r.setElem size (i + half) (b.elem size (2 * i) + b.elem size (2 * i + 1))
        return r
      setD r
    else if opcode == 19 && !u then                          -- MUL
      if size == 3 then raise st pc undefinedF else setD (zipV q size (· * ·) a b)
    else stopSimd st pc w
  else if w &&& 0x9F3E_0C00 == 0x0E20_0800 then              -- two-register misc
    let opcode := fld w 12 5
    let bits : UInt64 := 8 <<< size
    let all := ones bits
    if opcode == 9 || opcode == 8 || opcode == 10 then       -- CMEQ/CMLE, CMGT/CMGE, CMLT (zero)
      if size == 3 && !q then raise st pc undefinedF else
      let f : UInt64 → Bool := match opcode, u with
        | 8, false => fun x => sgt bits x 0
        | 8, true => fun x => sge bits x 0
        | 9, false => fun x => x == 0
        | 9, true => fun x => !(sgt bits x 0)
        | _, _ => fun x => !(sge bits x 0)
      if opcode == 10 && u then raise st pc undefinedF
      else setD (mapV q size (fun x => if f x then all else 0) a)
    else if opcode == 5 && u && size == 0 then setD ⟨~~~a.lo, ~~~a.hi⟩   -- NOT
    else if opcode == 0 || (opcode == 1 && !u) then           -- REV64, REV32, REV16
      let container : UInt64 := if opcode == 0 && !u then 64 else if opcode == 0 then 32 else 16
      if bits >= container then raise st pc undefinedF else
      let per := container / bits
      let r := Id.run do
        let mut r : V128 := ⟨0, 0⟩
        for e in [0:(elems q size).toNat] do
          let e := e.toUInt64
          r := r.setElem size e (a.elem size (e - e % per + (per - 1 - e % per)))
        return r
      setD r
    else stopSimd st pc w
  else if w &&& 0xFF3E_0C00 == 0x5E30_0800 then              -- scalar pairwise
    let opcode := fld w 12 5
    if opcode == 27 && !u && size == 3 then
      nx { st with rf := putV st.rf rd ⟨a.lo + a.hi, 0⟩ } pc
    else stopSimd st pc w
  else if w &&& 0xFF20_8C00 == 0x5E00_0000 then              -- crypto three-register SHA
    if size != 0 then raise st pc undefinedF else
    let opcode := fld w 12 3
    let d := getV st.rf rd
    let r? : Option V128 := match opcode with
      | 4 => some (sha256hash d a b true)          -- SHA256H Qd, Qn, Vm
      | 5 => some (sha256hash a d b false)         -- SHA256H2 Qd, Qn, Vm
      | 6 => some (sha256su1 d a b)
      | _ => none
    match r? with
    | some r => nx { st with rf := putV st.rf rd r } pc
    | none => if opcode == 7 then raise st pc undefinedF else stopSimd st pc w
  else if w &&& 0xFF3E_0C00 == 0x5E28_0800 then              -- crypto two-register SHA
    if size != 0 then raise st pc undefinedF else
    let opcode := fld w 12 5
    if opcode == 2 then nx { st with rf := putV st.rf rd (sha256su0 (getV st.rf rd) a) } pc
    else if opcode > 2 then raise st pc undefinedF else stopSimd st pc w
  else if w &&& 0x9F20_FC00 == 0x0E20_E000 && !u then        -- PMULL, PMULL2
    let hiHalf := q
    if size == 3 then
      let x := if hiHalf then a.hi else a.lo
      let y := if hiHalf then b.hi else b.lo
      let (lo, hi) := clmul x y 64
      nx { st with rf := putV st.rf rd ⟨lo, hi⟩ } pc
    else if size == 0 then
      let r := Id.run do
        let mut r : V128 := ⟨0, 0⟩
        for e in [0:8] do
          let x := (if hiHalf then a.hi else a.lo) >>> (8 * e.toUInt64) &&& 0xFF
          let y := (if hiHalf then b.hi else b.lo) >>> (8 * e.toUInt64) &&& 0xFF
          r := r.setElem 1 e.toUInt64 (clmul x y 8).1
        return r
      nx { st with rf := putV st.rf rd r } pc
    else raise st pc undefinedF
  else if w &&& 0x9F80_FC00 == 0x0F00_A400 && fld w 19 4 != 0 then   -- SSHLL, USHLL
    let immh := fld64 w 19 4
    if immh &&& 8 != 0 then raise st pc undefinedF else
    let log2 : UInt64 := if immh &&& 4 != 0 then 2 else if immh &&& 2 != 0 then 1 else 0
    let esize : UInt64 := 8 <<< log2
    let shift := fld64 w 16 7 - esize
    let src := if q then a.hi else a.lo
    let r := Id.run do
      let mut r : V128 := ⟨0, 0⟩
      for e in [0:(64 / esize).toNat] do
        let x := (src >>> (esize * e.toUInt64)) &&& ones esize
        let x := if u then x else sext x esize &&& ones (2 * esize)
        r := r.setElem (log2 + 1) e.toUInt64 (x <<< shift)
      return r
    nx { st with rf := putV st.rf rd r } pc
  else if w &&& 0x7F20_FC00 == 0x1E20_0000 then              -- FP <-> integer
    let sf := fld w 31 1 == 1
    let ftype := fld w 22 2
    let rmode := fld w 19 2
    let opcode := fld w 16 3
    if fld w 29 1 != 0 then raise st pc undefinedF else
    if rmode == 0 && (opcode == 2 || opcode == 3) && (ftype == 0 || ftype == 1) then   -- SCVTF / UCVTF
      let x := rr st.rf (zr rn)
      let x := if sf then x else x &&& 0xFFFF_FFFF
      let neg := opcode == 2 && (if sf then x >>> 63 == 1 else x >>> 31 == 1)
      let mag : Nat := if !neg then x.toNat else if sf then (0 - x).toNat else ((0 - x) &&& 0xFFFF_FFFF).toNat
      let rmodeF := (st.sys.fpcr >>> 22) &&& 3
      let (bitsv, inexact) := if ftype == 1 then intToFloat neg mag 11 52 rmodeF else intToFloat neg mag 8 23 rmodeF
      let st := if inexact then { st with sys := { st.sys with fpsr := st.sys.fpsr ||| 0x10 } } else st
      nx { st with rf := putV st.rf rd ⟨bitsv, 0⟩ } pc
    else if rmode == 0 && opcode == 6 && ((sf && ftype == 1) || (!sf && ftype == 0)) then   -- FMOV to general
      nx (setR st (zw rd) (if sf then a.lo else a.lo &&& 0xFFFF_FFFF)) pc
    else if rmode == 0 && opcode == 7 && ((sf && ftype == 1) || (!sf && ftype == 0)) then   -- FMOV from general
      let x := rr st.rf (zr rn)
      nx { st with rf := putV st.rf rd ⟨if sf then x else x &&& 0xFFFF_FFFF, 0⟩ } pc
    else if sf && ftype == 2 && rmode == 1 && opcode == 6 then nx (setR st (zw rd) a.hi) pc
    else if sf && ftype == 2 && rmode == 1 && opcode == 7 then
      let d := getV st.rf rd
      nx { st with rf := putV st.rf rd ⟨d.lo, rr st.rf (zr rn)⟩ } pc
    else stopSimd st pc w
  else stopSimd st pc w

end Leanbox
