/-!
# Instructions as data

`Op` is a decoded A64 instruction: what its handler needs, every operand
worked out as far as it can be without the machine state. A register operand
is a register-file slot, so the two meanings of register 31 (the zero
register or the stack pointer) are settled by the decoder, never by a
handler.
-/
namespace Leanbox

/-- Register-file slots: X0..X30, SP (the current one) = 31, the zero
register = 32 (never written), a discard slot = 33 for writes to XZR. -/
def SP : UInt8 := 31
def ZR : UInt8 := 32
def SINK : UInt8 := 33
/-- Where the SIMD&FP registers start in the register file, in bytes. -/
def VBASE : USize := 34 * 8
def RF_BYTES : Nat := 34 * 8 + 32 * 16

/-- A decoded instruction. Register operands are slots; immediates are
worked out as far as they can be without the machine state. -/
inductive Op where
  -- hot split forms
  | addR64  (d n m : UInt8)
  | subR64  (d n m : UInt8)
  | addR32  (d n m : UInt8)
  | logR64  (opc sh : UInt8) (d n m : UInt8) (amt : UInt64)
  | movR    (d m : UInt8)
  | subsI64 (d n : UInt8) (imm : UInt64)
  | addI64  (d n : UInt8) (imm : UInt64)        -- ADD/SUB Xd|SP, Xn|SP, #imm (SUB: imm negated)
  | logI64  (opc : UInt8) (d n : UInt8) (mask : UInt64)
  | movc    (d : UInt8) (v : UInt64)
  | madd64  (d n m a : UInt8)
  -- data processing (immediate)
  | adr     (page : Bool) (d : UInt8) (off : UInt64)
  | addsubI (sf sub setf : Bool) (d n : UInt8) (imm : UInt64)
  | logI    (sf : Bool) (opc : UInt8) (d n : UInt8) (mask : UInt64)
  | movk    (sf : Bool) (d : UInt8) (imm pos : UInt64)
  | bfm     (sf : Bool) (opc d n : UInt8) (r s wmask tmask : UInt64)
  | extr    (sf : Bool) (d n m : UInt8) (lsb : UInt64)
  -- data processing (register)
  | alu     (sf sub setf : Bool) (d n m sh : UInt8) (amt : UInt64)
  | aluX    (sf sub setf : Bool) (d n m opt : UInt8) (amt : UInt64)
  | adc     (sf sub setf : Bool) (d n m : UInt8)
  | logR    (sf : Bool) (opc : UInt8) (inv : Bool) (d n m sh : UInt8) (amt : UInt64)
  | ccmp    (sf sub isImm : Bool) (n m : UInt8) (imm : UInt64) (cond nzcv : UInt8)
  | csel    (sf inv inc : Bool) (d n m cond : UInt8)
  | dp1     (sf : Bool) (opc d n : UInt8)
  | dp2     (sf : Bool) (opc d n m : UInt8)
  | crc     (c : Bool) (sz d n m : UInt8)
  | madd    (sf sub : Bool) (d n m a : UInt8)
  | mull    (u sub : Bool) (d n m a : UInt8)
  | mulh    (u : Bool) (d n m : UInt8)
  -- branches
  | bcond   (cond : UInt8) (off : UInt64)
  | b       (link : Bool) (off : UInt64)
  | br      (kind n : UInt8)                    -- 0 BR, 1 BLR, 2 RET
  | cbz     (sf nz : Bool) (t : UInt8) (off : UInt64)
  | tbz     (nz : Bool) (t : UInt8) (bit : UInt64) (off : UInt64)
  -- loads and stores of general-purpose registers
  /-- `opc`: 0 store, 1 load, 2 load signed to 64, 3 load signed to 32;
  `kind`: 0 plain, 1 unprivileged (LDTR/STTR). -/
  | ldst    (size opc : UInt8) (wb post : Bool) (kind t n : UInt8) (off : UInt64)
  | ldstR   (size opc : UInt8) (t n m opt : UInt8) (shift : UInt64)
  | ldLit   (size : UInt8) (signed : Bool) (t : UInt8) (off : UInt64)
  /-- `kind`: 0 32-bit, 1 LDPSW, 2 64-bit. -/
  | ldstPair (kind : UInt8) (load wb post : Bool) (t t2 n : UInt8) (off : UInt64)
  | ldpPost64 (t t2 n : UInt8) (off : UInt64)
  | stpPost64 (t t2 n : UInt8) (off : UInt64)
  | ldrbPost  (t n : UInt8) (off : UInt64)
  /-- Exclusive and ordered single accesses: `kind` 0 LDXR/LDAXR, 1
  STXR/STLXR, 2 LDAR, 3 STLR; `s` the status register of a store exclusive. -/
  | excl    (kind size : UInt8) (t n s : UInt8)
  | exclPair (load : Bool) (sf : Bool) (t t2 n s : UInt8)
  -- loads and stores of SIMD&FP registers: `size` 0..4 (B H S D Q)
  | fldst   (size : UInt8) (load wb post : Bool) (t n : UInt8) (off : UInt64)
  | fldstR  (size : UInt8) (load : Bool) (t n m opt : UInt8) (shift : UInt64)
  | fldLit  (size : UInt8) (t : UInt8) (off : UInt64)
  | fldstPair (size : UInt8) (load wb post : Bool) (t t2 n : UInt8) (off : UInt64)
  /-- SIMD structure loads and stores (multiple and single), executed from
  the word. -/
  | vldst   (w : UInt32)
  -- system
  | nop
  | readCounter (virt : Bool) (t : UInt8)      -- MRS Xt, CNTPCT_EL0 / CNTVCT_EL0
  /-- Every other MRS: `k` the register's encoding (bits 20:5), `t` the
  destination slot, `w` the word (for the reference executor). -/
  | mrs     (w k : UInt32) (t : UInt8)
  | sys     (w : UInt32)                        -- every other system, exception and ERET word
  | simd    (w : UInt32)                        -- SIMD&FP data processing
  | undef   (w : UInt32)
  | endTrace
  deriving Inhabited, Repr

/-- Whether an op ends its trace: an unconditional branch and every op the
slow path may leave elsewhere from. A not-taken B.cond, CBZ or TBZ continues
the trace, as Bochs's traces do. -/
@[inline] def Op.endsTrace : Op → Bool
  | .b .. | .br .. | .sys _ | .undef _ => true
  | _ => false

end Leanbox
