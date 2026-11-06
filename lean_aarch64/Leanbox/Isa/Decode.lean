import Leanbox.Isa.Op
import Leanbox.Isa.Pseudocode
/-!
# The A64 decoder

The A64 decoder: an instruction word to the op its handler runs, for a
Cortex-A57 (Armv8.0 with FP, Advanced SIMD, the Cryptographic Extension and
CRC32), as rusty_box_aarch64 decodes it (cpu/isa/mod.rs `decode`, the
per-group decoders in cpu/isa/{dpimm,dpreg,control,ldst}.rs, and
cpu/decode.rs `classify`, which turns an encoding of a feature the
processor lacks into its `class_without_feature`).

A word that sets a should-be bit otherwise is UNDEFINED, as rusty_box's
decoder fixes those bits. The register-31 meaning of each operand (the
zero register or SP) is resolved here into a register-file slot.
-/
namespace Leanbox

/-- Bits `lo .. lo+n-1` of `w`. -/
@[inline] def fld (w : UInt32) (lo n : UInt32) : UInt32 := (w >>> lo) &&& ((1 <<< n) - 1)
@[inline] def fld64 (w : UInt32) (lo n : UInt32) : UInt64 := (fld w lo n).toUInt64
@[inline] def r5 (w : UInt32) (lo : UInt32) : UInt8 := (fld w lo 5).toUInt8
/-- Register 31 as the zero register (for reads). -/
@[inline] def zr (r : UInt8) : UInt8 := if r == 31 then ZR else r
/-- Register 31 as the zero register (for writes: discarded). -/
@[inline] def zw (r : UInt8) : UInt8 := if r == 31 then SINK else r

-- ── data processing (immediate): cpu/isa/dpimm.rs ─────────────────────
def decodeDpImm (w : UInt32) : Op :=
  let sf := fld w 31 1 == 1
  let d := r5 w 0
  let n := r5 w 5
  let op1 := fld w 22 4
  if op1 < 4 then                                    -- pcreladdr
    let imm := (fld64 w 29 2) ||| (fld64 w 5 19 <<< 2)
    if sf then .adr true (zw d) (sext imm 21 <<< 12) else .adr false (zw d) (sext imm 21)
  else if op1 < 6 then                               -- addsub_imm (op1<1> = sh)
    let imm := fld64 w 10 12 <<< (if fld w 22 1 == 1 then 12 else 0)
    let sub := fld w 30 1 == 1
    let setf := fld w 29 1 == 1
    if sf && sub && setf then .subsI64 (zw d) n imm
    else if sf && !setf then .addI64 d n (if sub then (0 : UInt64) - imm else imm)
    else .addsubI sf sub setf (if setf then zw d else d) n imm
  else if op1 < 8 then .undef w                      -- addsub_immtags (FEAT_MTE)
  else if op1 < 10 then                              -- log_imm
    let nbit := fld64 w 22 1
    if !sf && nbit == 1 then .undef w else
    match decodeBitMasks nbit (fld64 w 10 6) (fld64 w 16 6) true (if sf then 64 else 32) with
    | some (wm, _) =>
      let opc := (fld w 29 2).toUInt8
      if sf && opc != 3 then .logI64 opc d (zr n) wm
      else .logI sf opc (if opc == 3 then zw d else d) (zr n) wm
    | none => .undef w
  else if op1 < 12 then                              -- movewide
    let opc := (fld w 29 2).toUInt8
    let hw := fld64 w 21 2
    if opc == 1 || (!sf && hw >= 2) then .undef w else
    let imm := fld64 w 5 16
    let pos := hw * 16
    match opc with
    | 0 => .movc (zw d) (mask32 sf (~~~(imm <<< pos)))
    | 2 => .movc (zw d) (imm <<< pos)
    | _ => .movk sf (zw d) imm pos
  else if op1 < 14 then                              -- bitfield
    let opc := (fld w 29 2).toUInt8
    let nbit := fld64 w 22 1
    if opc == 3 || (sf && nbit == 0) || (!sf && nbit == 1) then .undef w else
    let immr := fld64 w 16 6
    let imms := fld64 w 10 6
    -- Arm SBFM/BFM/UBFM decode: a 32-bit form with immr<5> or imms<5> set
    if !sf && (immr >= 32 || imms >= 32) then .undef w else
    match decodeBitMasks nbit imms immr false (if sf then 64 else 32) with
    | some (wm, tm) => .bfm sf opc (zw d) (zr n) immr imms wm tm
    | none => .undef w
  else                                               -- extract
    let op21 := fld w 29 2
    let nbit := fld w 22 1
    let o0 := fld w 21 1
    if op21 != 0 || o0 != 0 then .undef w
    else if !sf && nbit == 0 && fld w 15 1 == 0 then .extr false (zw d) (zr n) (zr (r5 w 16)) (fld64 w 10 6)
    else if sf && nbit == 1 then .extr true (zw d) (zr n) (zr (r5 w 16)) (fld64 w 10 6)
    else .undef w

-- ── data processing (register): cpu/isa/dpreg.rs ──────────────────────
def decodeDpReg (w : UInt32) : Op :=
  let sf := fld w 31 1 == 1
  let d := r5 w 0
  let n := r5 w 5
  let m := r5 w 16
  let op0 := fld w 30 1
  let op1 := fld w 28 1
  let op2 := fld w 21 4
  let op3 := fld w 10 6
  if op1 == 0 then
    if fld w 24 1 == 0 then                          -- log_shift
      let amt := fld64 w 10 6
      if !sf && amt >= 32 then .undef w else
      let opc := (fld w 29 2).toUInt8
      let inv := fld w 21 1 == 1
      let sh := (fld w 22 2).toUInt8
      if sf && !inv && opc != 3 then
        if opc == 1 && n == 31 && amt == 0 then .movR (zw d) (zr m)
        else .logR64 opc sh (zw d) (zr n) (zr m) amt
      else .logR sf opc inv (zw d) (zr n) (zr m) sh amt
    else if fld w 21 1 == 0 then                     -- addsub_shift
      let sh := (fld w 22 2).toUInt8
      let amt := fld64 w 10 6
      if sh == 3 || (!sf && amt >= 32) then .undef w else
      let sub := fld w 30 1 == 1
      let setf := fld w 29 1 == 1
      if !setf && sh == 0 && amt == 0 then
        if sf then (if sub then .subR64 (zw d) (zr n) (zr m) else .addR64 (zw d) (zr n) (zr m))
        else if !sub then .addR32 (zw d) (zr n) (zr m)
        else .alu sf sub setf (zw d) (zr n) (zr m) sh amt
      else .alu sf sub setf (zw d) (zr n) (zr m) sh amt
    else                                             -- addsub_ext
      if fld w 22 2 != 0 then .undef w else
      let amt := fld64 w 10 3
      if amt > 4 then .undef w else
      let sub := fld w 30 1 == 1
      let setf := fld w 29 1 == 1
      .aluX sf sub setf (if setf then zw d else d) n (zr m) (fld w 13 3).toUInt8 amt
  else if fld w 24 1 == 1 then                       -- dp_3src
    let op54 := fld w 29 2
    let op31 := fld w 21 3
    let o0 := fld w 15 1
    let ra := r5 w 10
    if op54 != 0 then .undef w
    else if op31 == 0 then
      if sf && o0 == 0 then .madd64 (zw d) (zr n) (zr m) (zr ra)
      else .madd sf (o0 == 1) (zw d) (zr n) (zr m) (zr ra)
    else if !sf then .undef w
    else if op31 == 1 then .mull false (o0 == 1) (zw d) (zr n) (zr m) (zr ra)
    else if op31 == 5 then .mull true (o0 == 1) (zw d) (zr n) (zr m) (zr ra)
    else if op31 == 2 && o0 == 0 && ra == 31 then .mulh false (zw d) (zr n) (zr m)
    else if op31 == 6 && o0 == 0 && ra == 31 then .mulh true (zw d) (zr n) (zr m)
    else .undef w
  else if op2 == 0 then
    if op3 == 0 then                                 -- addsub_carry
      .adc sf (fld w 30 1 == 1) (fld w 29 1 == 1) (zw d) (zr n) (zr m)
    else .undef w                                    -- rmif, setf: FEAT_FlagM
  else if op2 == 2 then                              -- condcmp
    let s := fld w 29 1
    if s == 0 || fld w 10 1 != 0 || fld w 4 1 != 0 then .undef w else
    let isImm := fld w 11 1 == 1
    .ccmp sf (fld w 30 1 == 1) isImm (zr n) (zr m) (fld64 w 16 5)
      (fld w 12 4).toUInt8 (fld w 0 4).toUInt8
  else if op2 == 4 then                              -- condsel
    let s := fld w 29 1
    let o2 := fld w 10 2
    if s != 0 || o2 >= 2 then .undef w else
    .csel sf (fld w 30 1 == 1) (o2 == 1) (zw d) (zr n) (zr m) (fld w 12 4).toUInt8
  else if op2 == 6 then
    if fld w 29 1 != 0 then .undef w
    else if op0 == 0 then                            -- dp_2src
      let opc := fld w 10 6
      if opc == 2 || opc == 3 || (opc >= 8 && opc <= 11) then
        .dp2 sf opc.toUInt8 (zw d) (zr n) (zr m)
      else if opc >= 16 && opc <= 23 then           -- CRC32 / CRC32C
        let sz := opc &&& 3
        if (sz == 3) != sf then .undef w
        else .crc (opc &&& 4 != 0) sz.toUInt8 (zw d) (zr n) (zr m)
      else .undef w
    else                                             -- dp_1src
      let opcode2 := fld w 16 5
      let opc := fld w 10 6
      if opcode2 != 0 then .undef w
      else if opc <= 2 || opc == 4 || opc == 5 then .dp1 sf opc.toUInt8 (zw d) (zr n)
      else if opc == 3 && sf then .dp1 sf opc.toUInt8 (zw d) (zr n)
      else .undef w
  else .undef w

-- ── branches, exception generation and system: cpu/isa/control.rs ────
def decodeControl (w : UInt32) : Op :=
  let op0 := fld w 29 3
  let rt := r5 w 0
  if op0 == 2 then                                   -- condbranch
    if fld w 25 1 == 0 && fld w 24 1 == 0 && fld w 4 1 == 0 then
      .bcond (fld w 0 4).toUInt8 (sext (fld64 w 5 19) 19 <<< 2)
    else .undef w
  else if op0 == 0 || op0 == 4 then                  -- branch_imm
    if fld w 26 3 == 0b101 then .b (op0 == 4) (sext (fld64 w 0 26) 26 <<< 2) else .undef w
  else if op0 == 1 || op0 == 5 then
    let sf := op0 == 5
    if fld w 25 1 == 0 then                          -- compbranch
      .cbz sf (fld w 24 1 == 1) (zr rt) (sext (fld64 w 5 19) 19 <<< 2)
    else                                             -- testbranch
      .tbz (fld w 24 1 == 1) (zr rt) ((fld64 w 31 1 <<< 5) ||| fld64 w 19 5) (sext (fld64 w 5 14) 14 <<< 2)
  else if op0 == 6 then
    let top := fld w 24 2
    if top == 0 then                                 -- exception
      let opc := fld w 21 3
      let op2 := fld w 2 3
      let ll := fld w 0 2
      if op2 != 0 then .undef w
      else if opc == 0 && ll != 0 then .sys w        -- SVC, HVC, SMC
      else if opc == 1 && ll == 0 then .sys w        -- BRK
      else if opc == 2 && ll == 0 then .undef w      -- HLT, halting debug disabled
      else if opc == 5 && ll != 0 then .undef w      -- DCPS outside Debug state
      else .undef w
    else if top == 1 then                            -- system
      if fld w 22 2 != 0 then .undef w else
      let l := fld w 21 1
      let op0s := fld w 19 2
      let op1 := fld w 16 3
      let crn := fld w 12 4
      let crm := fld w 8 4
      let op2 := fld w 5 3
      if l == 0 && op0s == 0 then
        if op1 == 3 && crn == 2 then                 -- hints
          if rt != 31 then .undef w
          else if crm == 0 && op2 >= 2 && op2 <= 5 then .sys w   -- WFE WFI SEV SEVL
          else .nop
        else if op1 == 3 && crn == 1 then .undef w   -- WFET/WFIT: FEAT_WFxT
        else if op1 == 3 && crn == 3 then            -- barriers
          if rt != 31 then .undef w
          else if op2 == 2 then .sys w               -- CLREX
          else if op2 >= 4 && op2 <= 6 then .nop     -- DSB DMB ISB
          else .undef w                              -- SB, DSB nXS
        else if crn == 4 then                        -- pstate
          if op1 == 0 && op2 <= 2 then .undef w      -- CFINV/XAFLAG/AXFLAG: FEAT_FlagM
          else if rt != 31 then .undef w
          else .sys w
        else .undef w
      else if op0s == 1 then .sys w                  -- SYS, SYSL
      else if l == 1 && op0s == 3 && op1 == 3 && crn == 14 && crm == 0 && (op2 == 1 || op2 == 2) then
        .readCounter (op2 == 2) (zw rt)
      else if l == 1 then .mrs w ((w >>> 5) &&& 0xFFFF) (zw rt)
      else .sys w                                    -- MSR
    else if top >= 2 then                            -- branch_reg
      if fld w 25 1 == 0 then .undef w else
      let opc := fld w 21 4
      let op2 := fld w 16 5
      let op3 := fld w 10 6
      let rn := r5 w 5
      let op4 := fld w 0 5
      if op2 != 31 || op3 != 0 then .undef w
      else if opc == 0 && op4 == 0 then .br 0 (zr rn)
      else if opc == 1 && op4 == 0 then .br 1 (zr rn)
      else if opc == 2 && op4 == 0 then .br 2 (zr rn)
      else if opc == 4 && rn == 31 && op4 == 0 then .sys w     -- ERET
      else .undef w                                  -- DRPS outside Debug state, PAuth forms
    else .undef w
  else .undef w

-- ── loads and stores: cpu/isa/ldst.rs ─────────────────────────────────
def decodeLdst (w : UInt32) : Op :=
  let v := fld w 26 1 == 1
  let rt := r5 w 0
  let rn := r5 w 5
  let rm := r5 w 16
  let rt2 := r5 w 10
  let size := (fld w 30 2).toUInt8
  match fld w 28 2 with
  | 0 =>
    let b2423 := fld w 23 2
    if v then
      -- asisdlse / asisdlsep / asisdlso / asisdlsop
      if fld w 31 1 != 0 then .undef w
      else if b2423 == 0 then (if fld w 16 6 == 0 then .vldst w else .undef w)
      else if b2423 == 1 then (if fld w 21 1 == 0 then .vldst w else .undef w)
      else if b2423 == 2 then (if fld w 17 4 == 0 then .vldst w else .undef w)
      else .vldst w
    else if b2423 == 0 then
      if fld w 21 1 == 0 then                        -- ldstexclr
        let l := fld w 22 1
        let o0 := fld w 15 1
        if rt2 != 31 || (l == 1 && rm != 31) then .undef w else
        let _ := o0
        if l == 1 then .excl 0 size (zw rt) rn SINK
        else .excl 1 size (zr rt) rn (zw rm)
      else if fld w 31 1 == 1 then                   -- ldstexclp
        let l := fld w 22 1
        let o0 := fld w 15 1
        let sf := fld w 30 1 == 1
        if l == 1 && rm != 31 then .undef w
        else if l == 1 then let _ := o0; .exclPair true sf (zw rt) (zw rt2) rn SINK
        else .exclPair false sf (zr rt) (zr rt2) rn (zw rm)
      else .undef w                                  -- CASP: FEAT_LSE
    else if b2423 == 1 then
      if fld w 21 1 == 0 then                        -- ldstord
        let l := fld w 22 1
        let o0 := fld w 15 1
        if rm != 31 || rt2 != 31 || o0 == 0 then .undef w   -- LDLAR/STLLR: FEAT_LOR
        else if l == 1 then .excl 2 size (zw rt) rn SINK else .excl 3 size (zr rt) rn SINK
      else .undef w                                  -- CAS: FEAT_LSE
    else .undef w
  | 1 =>
    if fld w 24 1 == 0 then                          -- loadlit
      let opc := fld w 30 2
      let off := sext (fld64 w 5 19) 19 <<< 2
      if v then
        if opc == 3 then .undef w else .fldLit (opc.toUInt8 + 2) rt off
      else match opc with
        | 0 => .ldLit 2 false (zw rt) off
        | 1 => .ldLit 3 false (zw rt) off
        | 2 => .ldLit 2 true (zw rt) off
        | _ => .nop                                  -- PRFM (literal)
    else .undef w                                    -- ldapstl_unscaled: FEAT_LRCPC2
  | 2 =>                                             -- load/store pairs
    let opc := fld w 30 2
    let idx := fld w 23 2
    let load := fld w 22 1 == 1
    let wb := idx == 1 || idx == 3
    let post := idx == 1
    let imm7 := sext (fld64 w 15 7) 7
    if v then
      if opc == 3 then .undef w else
      let sz := opc.toUInt8 + 2
      .fldstPair sz load wb post rt rt2 rn (imm7 <<< sz.toUInt64)
    else
      let (t, t2) := if load then (zw rt, zw rt2) else (zr rt, zr rt2)
      if opc == 0 then .ldstPair 0 load wb post t t2 rn (imm7 <<< 2)
      else if opc == 1 then
        if load && idx != 0 then .ldstPair 1 load wb post t t2 rn (imm7 <<< 2) else .undef w
      else if opc == 2 then
        if idx == 1 then
          if load then .ldpPost64 t t2 rn (imm7 <<< 3) else .stpPost64 t t2 rn (imm7 <<< 3)
        else .ldstPair 2 load wb post t t2 rn (imm7 <<< 3)
      else .undef w
  | _ =>                                             -- single register
    let opc := (fld w 22 2).toUInt8
    let scale := if v then ((opc >>> 1) <<< 2) ||| size else size
    let prfm := !v && size == 3 && opc == 2
    let gprBad := !v && opc == 3 && size >= 2
    if fld w 24 1 == 1 then                          -- ldst_pos
      if v && opc >= 2 && size != 0 then .undef w
      else if gprBad && !prfm then .undef w
      else
      let off := fld64 w 10 12 <<< scale.toUInt64
      if prfm then .nop
      else if v then .fldst scale (opc &&& 1 == 1) false false rt rn off
      else .ldst size opc false false 0 (if opc == 0 then zr rt else zw rt) rn off
    else if fld w 21 1 == 0 then
      let idx := fld w 10 2
      let imm9 := sext (fld64 w 12 9) 9
      if v && opc >= 2 && size != 0 then .undef w
      else if idx == 2 then                          -- ldst_unpriv
        if v || (gprBad && !(size == 2 && opc == 2)) || prfm then .undef w
        else .ldst size opc false false 1 (if opc == 0 then zr rt else zw rt) rn imm9
      else if prfm then
        if idx == 0 then .nop else .undef w          -- PRFUM; no indexed prefetch
      else if gprBad && !(size == 2 && opc == 2) then .undef w
      else if v then .fldst scale (opc &&& 1 == 1) (idx != 0) (idx == 1) rt rn imm9
      else if size == 0 && opc == 1 && idx == 1 then .ldrbPost (zw rt) rn imm9
      else .ldst size opc (idx != 0) (idx == 1) 0 (if opc == 0 then zr rt else zw rt) rn imm9
    else
      let o := fld w 10 2
      if o == 2 then                                 -- ldst_regoff
        let option := fld w 13 3
        let s := fld w 12 1
        if v && opc >= 2 && size != 0 then .undef w
        else if prfm then (if option == 2 || option == 3 || option == 6 || option == 7 then .nop else .undef w)
        else if gprBad && !(size == 2 && opc == 2) then .undef w
        else if option &&& 2 == 0 then .undef w
        else
        let shift : UInt64 := if s == 1 then scale.toUInt64 else 0
        if v then .fldstR scale (opc &&& 1 == 1) rt rn (zr rm) option.toUInt8 shift
        else .ldstR size opc (if opc == 0 then zr rt else zw rt) rn (zr rm) option.toUInt8 shift
      else .undef w                                  -- atomics (FEAT_LSE), LDRAA (FEAT_PAuth)

-- ── SIMD&FP data processing: decoded at execution (`simd`) ───────────
def decode (w : UInt32) : Op :=
  let op0 := fld w 31 1
  let op1 := fld w 25 4
  if op1 == 0 then .undef w                          -- reserved (UDF), SME
  else if op1 == 8 || op1 == 9 then decodeDpImm w
  else if op1 == 10 || op1 == 11 then decodeControl w
  else if op1 == 4 || op1 == 6 || op1 == 12 || op1 == 14 then decodeLdst w
  else if op1 == 5 || op1 == 13 then decodeDpReg w
  else if op1 == 7 || op1 == 15 then .simd w
  else let _ := op0; .undef w
end Leanbox
