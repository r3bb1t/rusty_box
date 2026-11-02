/-
A pure-Lean A64 interpreter after Bochs's handlers-chaining design
(cpu/cpu.h BX_NEXT_INSTR / BX_NEXT_TRACE / BX_LINK_TRACE, cpu/icache.cc
BxEndTrace, genDummyTraceCacheEntry), for rusty_box's `aarch64_perfbench`
loops (alu, memcpy, fnv).

  * The whole interpreter is ONE self-tail-recursive function, `run`. Every
    handler ends by calling `run` on the next op — Bochs's BX_NEXT_INSTR, a
    tail call into the next handler — and Lean compiles a self tail call to a
    jump, so a trace of any length, and any number of traces, run in one
    activation with no stack growth (Bochs bounds its link depth because
    C++ does not promise the tail call).
  * Every trace ends in an `endTrace` op (Bochs's dummy end-of-trace
    instruction), so no handler checks for the end of its trace. A trace
    continues over a not-taken B.cond, as Bochs's traces do.
  * Hot encodings get their own handler, chosen at decode (Bochs's
    split handlers); XZR and SP are separate register-file slots, chosen at
    decode, so no operand access branches on register 31.
  * Machine state the hot path touches is the loop's parameters, unboxed:
    the trace, the op index, the trace's PC, NZCV, the instruction count and
    a two-entry data micro-TLB plus a one-entry code micro-TLB in front of a
    64-entry TLB and the stage 1 walk. Registers and RAM are ByteArrays the
    loop owns, so writes are in place. Nothing is allocated per instruction.
-/

-- ── unboxed byte access with USize indices ─────────────────────────────
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

@[inline] def gb (m : ByteArray) (i : USize) : UInt64 :=
  if h : i < m.usize then (m.uget i (ByteArray.lt_size_of_lt_usize h)).toUInt64 else 0
@[inline] def ld16 (m : ByteArray) (i : USize) : UInt64 := gb m i ||| (gb m (i+1) <<< 8)
@[inline] def ld32 (m : ByteArray) (i : USize) : UInt64 :=
  gb m i ||| (gb m (i+1) <<< 8) ||| (gb m (i+2) <<< 16) ||| (gb m (i+3) <<< 24)
@[inline] def ld64 (m : ByteArray) (i : USize) : UInt64 := ld32 m i ||| (ld32 m (i+4) <<< 32)

@[inline] def sb (m : ByteArray) (i : USize) (v : UInt64) : ByteArray :=
  if h : i < m.usize then m.uset i v.toUInt8 (ByteArray.lt_size_of_lt_usize h) else m
@[inline] def st16 (m : ByteArray) (i : USize) (v : UInt64) : ByteArray := sb (sb m i v) (i+1) (v >>> 8)
@[inline] def st32 (m : ByteArray) (i : USize) (v : UInt64) : ByteArray :=
  sb (sb (sb (sb m i v) (i+1) (v >>> 8)) (i+2) (v >>> 16)) (i+3) (v >>> 24)
@[inline] def st64 (m : ByteArray) (i : USize) (v : UInt64) : ByteArray :=
  st32 (st32 m i v) (i+4) (v >>> 32)
@[inline] def ldN (m : ByteArray) (i : USize) (size : UInt8) : UInt64 :=
  match size with | 0 => gb m i | 1 => ld16 m i | 2 => ld32 m i | _ => ld64 m i
@[inline] def stN (m : ByteArray) (i : USize) (size : UInt8) (v : UInt64) : ByteArray :=
  match size with | 0 => sb m i v | 1 => st16 m i v | 2 => st32 m i v | _ => st64 m i v

@[inline] def agu {α} (a : Array α) (i : USize) (dflt : α) : α :=
  if h : i < a.usize then a.uget i (Array.lt_size_of_lt_usize h) else dflt

-- ── the register file: X0..X30, SP = 31, ZR (reads 0) = 32, sink = 33 ──
def SP : UInt8 := 31
def ZR : UInt8 := 32
def SINK : UInt8 := 33
@[inline] def rr (rf : ByteArray) (s : UInt8) : UInt64 := ld64 rf (s.toUSize * 8)
@[inline] def wr (rf : ByteArray) (s : UInt8) (v : UInt64) : ByteArray := st64 rf (s.toUSize * 8) v

def RAM_BASE : UInt64 := 0x40000000
def RAM_SIZE : Nat := 64 * 1024 * 1024
def MISS : UInt64 := 0xFFFFFFFFFFFFFFFF
def CACHE : Nat := 4096

-- ── decoded instructions ───────────────────────────────────────────────
inductive Op where
  -- split handlers for the hot forms
  | addR64  (d n m : UInt8)                    -- ADD Xd, Xn, Xm
  | subR64  (d n m : UInt8)                    -- SUB Xd, Xn, Xm
  | addR32  (d n m : UInt8)                    -- ADD Wd, Wn, Wm
  | logR64  (opc sh : UInt8) (d n m : UInt8) (amt : UInt64)   -- AND/ORR/EOR Xd, Xn, Xm, shift #amt
  | movR    (d m : UInt8)                      -- MOV Xd, Xm (ORR Xd, XZR, Xm)
  | subsI64 (d n : UInt8) (imm : UInt64)       -- SUBS Xd, Xn, #imm
  | logI64  (opc : UInt8) (d n : UInt8) (mask : UInt64)  -- AND/ORR/EOR Xd, Xn, #mask
  | movc    (d : UInt8) (v : UInt64)           -- MOVZ / MOVN, value worked out at decode
  | madd64  (d n m a : UInt8)                  -- MADD Xd, Xn, Xm, Xa (MUL: a = ZR)
  | ldpPost64 (t t2 n : UInt8) (off : UInt64)  -- LDP Xt, Xt2, [Xn], #off
  | stpPost64 (t t2 n : UInt8) (off : UInt64)  -- STP Xt, Xt2, [Xn], #off
  | ldrbPost  (t n : UInt8) (off : UInt64)     -- LDRB Wt, [Xn], #off
  -- general handlers
  | alu     (sf sub setf : Bool) (d n m sh : UInt8) (amt : UInt64)
  | logR    (sf : Bool) (opc : UInt8) (inv : Bool) (d n m sh : UInt8) (amt : UInt64)
  | addsubI (sf sub setf : Bool) (d n : UInt8) (imm : UInt64)
  | logI    (sf : Bool) (opc : UInt8) (d n : UInt8) (mask : UInt64)
  | movk    (sf : Bool) (d : UInt8) (imm pos : UInt64)
  | bfm     (sf : Bool) (opc d n : UInt8) (r s wmask tmask : UInt64)
  | madd    (sf sub : Bool) (d n m a : UInt8)
  | ldstPair (sf load wb post : Bool) (t t2 n : UInt8) (off : UInt64)
  | ldst    (size opc : UInt8) (wb post : Bool) (t n : UInt8) (off : UInt64)
  | bcond   (cond : UInt8) (off : UInt64)
  | b       (link : Bool) (off : UInt64)
  | hvc
  | undef   (w : UInt32)
  | endTrace                                   -- Bochs BxEndTrace
  deriving Inhabited

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
@[inline] def shiftReg (sf : Bool) (x : UInt64) (sh : UInt8) (amt : UInt64) : UInt64 :=
  if sf then shift64 x sh amt else
    let x := x &&& 0xFFFFFFFF
    match sh with
    | 0 => (x <<< amt) &&& 0xFFFFFFFF
    | 1 => x >>> amt
    | 2 => ((x.toUInt32.toInt32 >>> amt.toUInt32.toInt32).toUInt32).toUInt64
    | _ => ror false x amt

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

@[inline] def sext (x : UInt64) (bits : UInt64) : UInt64 :=
  ((x <<< (64 - bits)).toInt64 >>> (64 - bits).toInt64).toUInt64

-- ── decode: the A64 encoding index, with the register-31 meaning per field ─
def decode (w : UInt32) : Op :=
  let w64 := w.toUInt64
  let f (lo bits : UInt64) : UInt64 := (w64 >>> lo) &&& ones bits
  let r5 (lo : UInt64) : UInt8 := (f lo 5).toUInt8
  -- register 31 as the zero register (reads) / discarded (writes), or as SP
  let zr (r : UInt8) : UInt8 := if r == 31 then ZR else r
  let zw (r : UInt8) : UInt8 := if r == 31 then SINK else r
  let d := r5 0
  let n := r5 5
  let m := r5 16
  let sf := f 31 1 == 1
  if w &&& 0x1F200000 == 0x0B000000 then            -- add/sub (shifted register)
    let sh := (f 22 2).toUInt8
    let amt := f 10 6
    if sh == 3 || (!sf && amt >= 32) then .undef w else
    let sub := f 30 1 == 1
    let setf := f 29 1 == 1
    if !setf && sh == 0 && amt == 0 then
      if sf then (if sub then .subR64 (zw d) (zr n) (zr m) else .addR64 (zw d) (zr n) (zr m))
      else if !sub then .addR32 (zw d) (zr n) (zr m)
      else .alu sf sub setf (zw d) (zr n) (zr m) sh amt
    else .alu sf sub setf (zw d) (zr n) (zr m) sh amt
  else if w &&& 0x1F000000 == 0x0A000000 then       -- logical (shifted register)
    let amt := f 10 6
    if !sf && amt >= 32 then .undef w else
    let opc := (f 29 2).toUInt8
    let inv := f 21 1 == 1
    let sh := (f 22 2).toUInt8
    if sf && !inv && opc != 3 then
      if opc == 1 && n == 31 && amt == 0 then .movR (zw d) (zr m)
      else .logR64 opc sh (zw d) (zr n) (zr m) amt
    else .logR sf opc inv sh (zw d) (zr n) (zr m) amt
  else if w &&& 0x1F800000 == 0x11000000 then       -- add/sub (immediate)
    let imm := f 10 12 <<< (if f 22 1 == 1 then 12 else 0)
    let sub := f 30 1 == 1
    let setf := f 29 1 == 1
    if sf && sub && setf then .subsI64 (zw d) n imm   -- Rn is SP-capable: slot 31 is SP
    else .addsubI sf sub setf (if setf then zw d else d) n imm
  else if w &&& 0x1F800000 == 0x12000000 then       -- logical (immediate)
    if !sf && f 22 1 == 1 then .undef w else
    match decodeBitMasks (f 22 1) (f 10 6) (f 16 6) true (if sf then 64 else 32) with
    | some (wm, _) =>
      let opc := (f 29 2).toUInt8
      if sf && opc != 3 then .logI64 opc d (zr n) wm      -- Rd is SP-capable
      else .logI sf opc (if opc == 3 then zw d else d) (zr n) wm
    | none => .undef w
  else if w &&& 0x1F800000 == 0x12800000 then       -- move wide
    let opc := (f 29 2).toUInt8
    let hw := f 21 2
    if opc == 1 || (!sf && hw >= 2) then .undef w else
    let imm := f 5 16
    let pos := hw * 16
    match opc with
    | 0 => .movc (zw d) (mask32 sf (~~~(imm <<< pos)))
    | 2 => .movc (zw d) (imm <<< pos)
    | _ => .movk sf (zw d) imm pos
  else if w &&& 0x1F800000 == 0x13000000 then       -- bitfield
    let opc := (f 29 2).toUInt8
    let nbit := f 22 1
    if opc == 3 || (sf && nbit == 0) || (!sf && nbit == 1) then .undef w else
    match decodeBitMasks nbit (f 10 6) (f 16 6) false (if sf then 64 else 32) with
    | some (wm, tm) => .bfm sf opc (zw d) (zr n) (f 16 6) (f 10 6) wm tm
    | none => .undef w
  else if w &&& 0x1F000000 == 0x1B000000 then       -- data-processing (3 source)
    if f 21 3 != 0 || f 29 2 != 0 then .undef w else
    let sub := f 15 1 == 1
    let a := r5 10
    if sf && !sub then .madd64 (zw d) (zr n) (zr m) (zr a)
    else .madd sf sub (zw d) (zr n) (zr m) (zr a)
  else if w &&& 0x3A000000 == 0x28000000 then       -- load/store pair
    let opc := f 30 2
    let idx := f 23 2
    if f 26 1 == 1 || idx == 0 || opc == 1 || opc == 3 then .undef w else
    let sf := opc == 2
    let load := f 22 1 == 1
    let t := r5 0
    let t2 := r5 10
    let off := sext (f 15 7) 7 <<< (if sf then 3 else 2)
    -- Rt/Rt2: zero register for stores, discarded for loads; Rn is SP-capable
    let (t, t2) := if load then (zw t, zw t2) else (zr t, zr t2)
    if sf && idx == 1 then (if load then .ldpPost64 t t2 n off else .stpPost64 t t2 n off)
    else .ldstPair sf load (idx != 2) (idx == 1) t t2 n off
  else if w &&& 0x3B200000 == 0x38000000 && f 10 2 != 2 && f 26 1 == 0 then  -- ld/st (imm9)
    let idx := f 10 2
    let size := (f 30 2).toUInt8
    let opc := (f 22 2).toUInt8
    let t := if opc == 0 then zr d else zw d
    let off := sext (f 12 9) 9
    if size == 0 && opc == 1 && idx == 1 then .ldrbPost t n off
    else .ldst size opc (idx != 0) (idx == 1) t n off
  else if w &&& 0x3B000000 == 0x39000000 && f 26 1 == 0 then                 -- ld/st (uimm12)
    let size := f 30 2
    let opc := (f 22 2).toUInt8
    .ldst size.toUInt8 opc false false (if opc == 0 then zr d else zw d) n (f 10 12 <<< size)
  else if w &&& 0xFF000010 == 0x54000000 then       -- B.cond
    .bcond (f 0 4).toUInt8 (sext (f 5 19) 19 <<< 2)
  else if w &&& 0x7C000000 == 0x14000000 then       -- B, BL
    .b (f 31 1 == 1) (sext (f 0 26) 26 <<< 2)
  else if w &&& 0xFFE0001F == 0xD4000002 then .hvc
  else .undef w

/-- A trace ends after an unconditional branch, HVC or an undefined
instruction, at 32 instructions or at the page's end — but NOT after a
B.cond, which continues the trace when not taken, as in Bochs. -/
@[inline] def Op.endsTrace : Op → Bool
  | .b .. | .hvc | .undef _ => true
  | _ => false

-- ── the slowly-changing state: one object, rebuilt only on a miss ─────
structure Sys where
  ttbr0 : UInt64
  tlb   : Array UInt64          -- 64 × (VA page, PA page)
  ctag  : Array UInt64          -- trace cache tags (physical address)
  ctr   : Array (Array Op)      -- trace cache, direct-mapped
  cpg   : ByteArray             -- 1 per RAM page holding cached code

/-- TTBR0, T0SZ = 25, 4 KiB granule, level 1 block descriptors with AF. -/
def walk (sys : Sys) (mem : ByteArray) (va : UInt64) : UInt64 :=
  if va >>> 39 != 0 then MISS else
  let dpa := sys.ttbr0 + ((va >>> 30) &&& 0x1FF) * 8
  if dpa < RAM_BASE || dpa - RAM_BASE + 8 > RAM_SIZE.toUInt64 then MISS else
  let desc := ld64 mem (dpa - RAM_BASE).toUSize
  if desc &&& 3 == 1 && desc &&& 0x400 != 0 then
    (desc &&& 0x0000FFFFC0000000) ||| (va &&& 0x3FFFF000)
  else MISS

/-- The PA page of `va` through the 64-entry TLB, filling it from the walk;
`MISS` on a translation fault. Only reached when a micro-TLB misses. -/
def translate (sys : Sys) (mem : ByteArray) (va : UInt64) : Sys × UInt64 :=
  let vp := va >>> 12
  let i := ((vp &&& 63) * 2).toUSize
  if agu sys.tlb i MISS == vp then (sys, agu sys.tlb (i+1) MISS) else
  let pa := walk sys mem va
  if pa == MISS then (sys, MISS) else
  ({ sys with tlb := (sys.tlb.set! i.toNat vp).set! (i.toNat + 1) pa }, pa)

def buildTrace (mem : ByteArray) (pa : UInt64) : Array Op := Id.run do
  let mut ops : Array Op := Array.emptyWithCapacity 33
  let mut a := pa
  for _ in [0:32] do
    let op := decode (ld32 mem (a - RAM_BASE).toUSize).toUInt32
    ops := ops.push op
    a := a + 4
    if op.endsTrace || a &&& 0xFFF == 0 then break
  return ops.push .endTrace

def dropCache (sys : Sys) : Sys :=
  { sys with ctag := Array.replicate CACHE MISS, ctr := Array.replicate CACHE #[],
             cpg := ByteArray.mk (Array.replicate (RAM_SIZE / 4096) 0) }

/-- The trace a taken branch goes to: a one-op trace whose `endTrace`, at
index 0, dispatches on the trace PC it is given (Bochs BX_NEXT_TRACE /
BX_LINK_TRACE into getTraceCacheEntry). -/
def DISPATCH : Array Op := #[.endTrace]

structure Res where
  rf    : ByteArray
  mem   : ByteArray
  pc    : UInt64
  icnt  : UInt64
  halt  : UInt8      -- 1 powered off, 2 undefined, 3 fault
  deriving Inhabited

@[inline] def inRam (pa : UInt64) (bytes : UInt64) : Bool :=
  pa >= RAM_BASE && pa - RAM_BASE + bytes <= RAM_SIZE.toUInt64

-- ── the chained interpreter ────────────────────────────────────────────
/-- `tr[i]` is the next op; `tpc` is the trace's first PC (op `i` is at
`tpc + 4i`); (`dv0`,`dp0`), (`dv1`,`dp1`) are the data micro-TLB's VA and
PA pages and (`cv`,`cp`) the code micro-TLB's. -/
partial def run (rf mem : ByteArray) (sys : Sys) (tr : Array Op) (i : USize)
    (tpc nzcv icnt cv cp dv0 dp0 dv1 dp1 : UInt64) : Res :=
  let op := agu tr i .endTrace
  let pc := tpc + 4 * i.toUInt64
  let fault (h : UInt8) : Res := { rf, mem, pc, icnt, halt := h }
  -- the data micro-TLB: the PA of `va` if its page is cached, else MISS
  let dpa (va : UInt64) : UInt64 :=
    let vp := va >>> 12
    if vp == dv0 then dp0 ||| (va &&& 0xFFF)
    else if vp == dv1 then dp1 ||| (va &&& 0xFFF) else MISS
  match op with
  | .addR64 d n m =>
    run (wr rf d (rr rf n + rr rf m)) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .subR64 d n m =>
    run (wr rf d (rr rf n - rr rf m)) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .addR32 d n m =>
    run (wr rf d ((rr rf n + rr rf m) &&& 0xFFFFFFFF)) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .logR64 opc sh d n m amt =>
    run (wr rf d (logic opc (rr rf n) (shift64 (rr rf m) sh amt))) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .movR d m =>
    run (wr rf d (rr rf m)) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .subsI64 d n imm =>
    let x := rr rf n
    let y := ~~~imm
    run (wr rf d (x - imm)) mem sys tr (i+1) tpc (awcFlags true x y 1) (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .logI64 opc d n mask =>
    run (wr rf d (logic opc (rr rf n) mask)) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .movc d v =>
    run (wr rf d v) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .madd64 d n m a =>
    run (wr rf d (rr rf a + rr rf n * rr rf m)) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .alu sf sub setf d n m sh amt =>
    let x := mask32 sf (rr rf n)
    let y := mask32 sf (shiftReg sf (rr rf m) sh amt)
    let y := if sub then mask32 sf (~~~y) else y
    let carry : UInt64 := if sub then 1 else 0
    let r := mask32 sf (x + y + carry)
    let nzcv := if setf then awcFlags sf x y carry else nzcv
    run (wr rf d r) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .logR sf opc inv sh d n m amt =>
    let y := shiftReg sf (rr rf m) sh amt
    let y := mask32 sf (if inv then ~~~y else y)
    let r := mask32 sf (logic opc (rr rf n) y)
    let nzcv := if opc == 3 then logicFlags sf r else nzcv
    run (wr rf d r) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .addsubI sf sub setf d n imm =>
    let x := mask32 sf (rr rf n)
    let y := if sub then mask32 sf (~~~imm) else imm
    let carry : UInt64 := if sub then 1 else 0
    let r := mask32 sf (x + y + carry)
    let nzcv := if setf then awcFlags sf x y carry else nzcv
    run (wr rf d r) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .logI sf opc d n mask =>
    let r := mask32 sf (logic opc (mask32 sf (rr rf n)) mask)
    let nzcv := if opc == 3 then logicFlags sf r else nzcv
    run (wr rf d r) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .movk sf d imm pos =>
    let r := mask32 sf ((rr rf d &&& ~~~(0xFFFF <<< pos)) ||| (imm <<< pos))
    run (wr rf d r) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .bfm sf opc d n r s wmask tmask =>
    let src := rr rf n
    let bot := ror sf src r &&& wmask
    let res := match opc with
      | 0 => ((if (src >>> s) &&& 1 == 1 then MISS else 0) &&& ~~~tmask) ||| (bot &&& tmask)
      | 1 => let dst := rr rf d
             (dst &&& ~~~tmask) ||| (((dst &&& ~~~wmask) ||| bot) &&& tmask)
      | _ => bot &&& tmask
    run (wr rf d (mask32 sf res)) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .madd sf sub d n m a =>
    let p := rr rf n * rr rf m
    let r := mask32 sf (if sub then rr rf a - p else rr rf a + p)
    run (wr rf d r) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  -- memory: a micro-TLB miss refills it and runs the same op again
  | .ldpPost64 t t2 n off =>
    let base := rr rf n
    let pa := dpa base
    if pa == MISS then refill base else
    if (base &&& 0xFFF) > 0xFF0 || !inRam pa 16 then fault 3 else
    let o := (pa - RAM_BASE).toUSize
    let v1 := ld64 mem o
    let v2 := ld64 mem (o + 8)
    run (wr (wr (wr rf t v1) t2 v2) n (base + off)) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .stpPost64 t t2 n off =>
    let base := rr rf n
    let pa := dpa base
    if pa == MISS then refill base else
    if (base &&& 0xFFF) > 0xFF0 || !inRam pa 16 then fault 3 else
    let o := (pa - RAM_BASE).toUSize
    let mem := st64 (st64 mem o (rr rf t)) (o + 8) (rr rf t2)
    let rf := wr rf n (base + off)
    if gb sys.cpg (o >>> 12) != 0 then
      run rf mem (dropCache sys) DISPATCH 0 (pc + 4) nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
    else run rf mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .ldrbPost t n off =>
    let base := rr rf n
    let pa := dpa base
    if pa == MISS then refill base else
    if !inRam pa 1 then fault 3 else
    let v := gb mem (pa - RAM_BASE).toUSize
    run (wr (wr rf t v) n (base + off)) mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .ldstPair sf load wb post t t2 n off =>
    let base := rr rf n
    let addr := if post then base else base + off
    let bytes : UInt64 := if sf then 8 else 4
    let size : UInt8 := if sf then 3 else 2
    let pa := dpa addr
    if pa == MISS then refill addr else
    if (addr &&& 0xFFF) + 2 * bytes > 0x1000 || !inRam pa (2 * bytes) then fault 3 else
    let o := (pa - RAM_BASE).toUSize
    if load then
      let v1 := ldN mem o size
      let v2 := ldN mem (o + bytes.toUSize) size
      let rf := wr (wr rf t v1) t2 v2
      let rf := if wb then wr rf n (base + off) else rf
      run rf mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
    else
      let mem := stN (stN mem o size (rr rf t)) (o + bytes.toUSize) size (rr rf t2)
      let rf := if wb then wr rf n (base + off) else rf
      if gb sys.cpg (o >>> 12) != 0 then
        run rf mem (dropCache sys) DISPATCH 0 (pc + 4) nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
      else run rf mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .ldst size opc wb post t n off =>
    let base := rr rf n
    let addr := if post then base else base + off
    let bytes : UInt64 := 1 <<< size.toUInt64
    let pa := dpa addr
    if pa == MISS then refill addr else
    if (addr &&& 0xFFF) + bytes > 0x1000 || !inRam pa bytes then fault 3 else
    let o := (pa - RAM_BASE).toUSize
    if opc == 0 then
      let mem := stN mem o size (rr rf t)
      let rf := if wb then wr rf n (base + off) else rf
      if gb sys.cpg (o >>> 12) != 0 then
        run rf mem (dropCache sys) DISPATCH 0 (pc + 4) nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
      else run rf mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
    else
      let v := ldN mem o size
      let v := if opc == 1 then v else
        let v := sext v (8 * bytes)
        if opc == 3 then v &&& 0xFFFFFFFF else v
      let rf := wr rf t v
      let rf := if wb then wr rf n (base + off) else rf
      run rf mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  -- control: BX_NEXT_INSTR over a not-taken branch, BX_LINK_TRACE when taken
  | .bcond cond off =>
    if conditionHolds cond nzcv then
      run rf mem sys DISPATCH 0 (pc + off) nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
    else run rf mem sys tr (i+1) tpc nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .b link off =>
    let rf := if link then wr rf 30 (pc + 4) else rf
    run rf mem sys DISPATCH 0 (pc + off) nzcv (icnt+1) cv cp dv0 dp0 dv1 dp1
  | .hvc =>   -- PSCI over HVC: SYSTEM_OFF powers off
    fault (if rr rf 0 == 0x84000008 then 1 else 2)
  | .undef _ => fault 2
  -- BxEndTrace: look the next trace up by the physical address of `pc`
  | .endTrace =>
    if pc >>> 12 != cv then
      let (sys, pp) := translate sys mem pc
      if pp == MISS then fault 3 else
      run rf mem sys DISPATCH 0 pc nzcv icnt (pc >>> 12) pp dv0 dp0 dv1 dp1
    else
    let pa := cp ||| (pc &&& 0xFFF)
    if !inRam pa 4 then fault 3 else
    let idx := ((pa >>> 2) &&& (CACHE.toUInt64 - 1)).toUSize
    if agu sys.ctag idx MISS == pa then
      run rf mem sys (agu sys.ctr idx DISPATCH) 0 pc nzcv icnt cv cp dv0 dp0 dv1 dp1
    else
      let t := buildTrace mem pa
      let pg := ((pa - RAM_BASE) >>> 12).toNat
      let sys := { sys with ctag := sys.ctag.set! idx.toNat pa, ctr := sys.ctr.set! idx.toNat t,
                            cpg := sys.cpg.set! pg 1 }
      run rf mem sys t 0 pc nzcv icnt cv cp dv0 dp0 dv1 dp1
where
  /-- Bring `va`'s page into the data micro-TLB (the older entry is evicted)
  and run the same op again. -/
  refill (va : UInt64) : Res :=
    let (sys', pp) := translate sys mem va
    if pp == MISS then { rf, mem, pc := tpc + 4 * i.toUInt64, icnt, halt := 3 } else
    run rf mem sys' tr i tpc nzcv icnt cv cp (va >>> 12) pp dv0 dp0

-- ── the perfbench setup ────────────────────────────────────────────────
def CODE : UInt64 := RAM_BASE + 0x1000
def TABLES : UInt64 := RAM_BASE + 0x100000
def SOURCE : UInt64 := RAM_BASE + 0x200000
def DESTINATION : UInt64 := RAM_BASE + 0x300000
def RESULT : UInt64 := RAM_BASE + 0x400000
def BLOCK : Nat := 64 * 1024
def POWER_OFF : List UInt32 := [0xD2800100, 0xF2B08000, 0xD4000002]
def ALU : List UInt32 := [0x8B010000, 0xCA000C21, 0xCB000042, 0xAA421C63, 0x92089C64,
  0x9B011405, 0xD37BE8A6, 0x0B0000E7, 0xF1000529, 0x54FFFEE1]
def MEMCPY : List UInt32 := [0xAA0B03E0, 0xAA0C03E1, 0xD2810002, 0xA8C11003, 0xA8C11805,
  0xA8811023, 0xA8811825, 0xF1000442, 0x54FFFF61, 0xF100054A, 0x54FFFEC1]
def FNV : List UInt32 := [0xD28464A1, 0xF2B08441, 0xF2D39C81, 0xF2F97E41, 0xD2803664,
  0xF2C02004, 0xAA0B03E0, 0xD2A00022, 0x38401403, 0xCA030021, 0x9B047C21, 0xF1000442,
  0x54FFFF81, 0xF9000181, 0xF100054A, 0x54FFFEE1]

def zeros (n : Nat) : ByteArray := Id.run do
  let mut b := ByteArray.emptyWithCapacity n
  for _ in [0:n] do b := b.push 0
  return b

def setup (code : List UInt32) : ByteArray := Id.run do
  let mut mem := zeros RAM_SIZE
  let mut a := (CODE - RAM_BASE).toUSize
  for w in code ++ POWER_OFF do
    mem := st32 mem a w.toUInt64
    a := a + 4
  let mut seed : UInt64 := 0x9E3779B97F4A7C15
  for i in [0:BLOCK / 8] do
    seed := seed ^^^ (seed <<< 13)
    seed := seed ^^^ (seed >>> 7)
    seed := seed ^^^ (seed <<< 17)
    mem := st64 mem ((SOURCE - RAM_BASE).toUSize + (8 * i).toUSize) seed
  let block : UInt64 := RAM_BASE ||| ((1 : UInt64) <<< 10) ||| ((3 : UInt64) <<< 8) ||| 1
  mem := st64 mem ((TABLES - RAM_BASE) + (8 : UInt64) * (RAM_BASE >>> 30)).toUSize block
  return mem

def fnv64 (m : ByteArray) (off len : Nat) : UInt64 := Id.run do
  let mut h : UInt64 := 0xcbf29ce484222325
  for i in [off:off+len] do
    h := (h ^^^ (m[i]!).toUInt64) * 0x100000001b3
  return h

def hex (x : UInt64) : String := String.ofList (Nat.toDigits 16 x.toNat)

def main (args : List String) : IO UInt32 := do
  let wl := args.getD 0 "alu"
  let scale := ((args.getD 1 "1").toNat?.getD 1).toUInt64
  let (code, regs) : List UInt32 × List (UInt8 × UInt64) := match wl with
    | "memcpy" => (MEMCPY, [(10, if scale == 0 then 80 else 8000 * scale), (11, SOURCE), (12, DESTINATION)])
    | "fnv"    => (FNV, [(10, if scale == 0 then 3 else 250 * scale), (11, SOURCE), (12, RESULT)])
    | _        => (ALU, [(0, 0x0123456789ABCDEF), (1, 0x0F1E2D3C4B5A6978), (9, if scale == 0 then 100000 else 10000000 * scale)])
  let mem := setup code
  let mut rf := zeros (34 * 8)
  for (r, v) in regs do rf := wr rf r v
  let tlb := Array.replicate 128 MISS
  let ctag := Array.replicate CACHE MISS
  let ctr : Array (Array Op) := Array.replicate CACHE #[]
  let cpg := ByteArray.mk (Array.replicate (RAM_SIZE / 4096) 0)
  let sys : Sys := { ttbr0 := TABLES, tlb := tlb, ctag := ctag, ctr := ctr, cpg := cpg }
  -- force the setup before the clock starts
  if (ld64 mem 0x1000) == 1 then IO.println "?"
  let t0 ← IO.monoNanosNow
  let r := run rf mem sys DISPATCH 0 CODE 0 0 MISS MISS MISS MISS MISS MISS
  let t1 ← IO.monoNanosNow
  let s := (t1 - t0).toFloat / 1e9
  IO.println s!"lean chain {wl}: {r.icnt} instrs {s}s {r.icnt.toFloat / s / 1e6} MIPS halt={r.halt}"
  IO.println s!"  regs {String.intercalate " " ((List.range 13).map fun i => hex (rr r.rf i.toUInt8))}"
  IO.println s!"  dst {hex (fnv64 r.mem (DESTINATION - RAM_BASE).toNat BLOCK)} result {hex (ld64 r.mem (RESULT - RAM_BASE).toUSize)}"
  return 0
