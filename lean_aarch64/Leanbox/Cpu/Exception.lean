import Leanbox.Cpu.Mmu
/-!
# Exceptions and retiring

A synchronous exception as a value (`Fault`); taking one (exception.rs
`take_exception`) and returning from one (`exception_return`); and how an
instruction retires: to the next instruction (`nx`), by a branch (`jmp`), by
raising (`raise`), with the register writes it retires with (`setR`,
`setV`) and the stack-pointer alignment check of a base register
(`baseReg`).
-/
namespace Leanbox

/-- A synchronous exception an instruction raises, or a stop. `ec` 0x100
and above is a stop: 0x100 power off, 0x101 processor off, 0x102 reset,
0x1FF a behaviour this machine does not model (`why`). -/
structure Fault where
  ec : UInt64
  iss : UInt64 := 0
  far : Option UInt64 := none
  /-- The exception returns to the next instruction (SVC, HVC). -/
  next : Bool := false
  why : String := ""

def EC_UNKNOWN : UInt64 := 0x00
def EC_WFX : UInt64 := 0x01
def EC_FP : UInt64 := 0x07
def EC_ILLEGAL : UInt64 := 0x0E
def EC_SVC : UInt64 := 0x15
def EC_SYSREG : UInt64 := 0x18
def EC_IABORT_LOWER : UInt64 := 0x20
def EC_IABORT : UInt64 := 0x21
def EC_PC_ALIGN : UInt64 := 0x22
def EC_DABORT_LOWER : UInt64 := 0x24
def EC_DABORT : UInt64 := 0x25
def EC_SP_ALIGN : UInt64 := 0x26
def EC_BRK : UInt64 := 0x3C
def STOP_POWER_OFF : UInt64 := 0x100
def STOP_CPU_OFF : UInt64 := 0x101
def STOP_RESET : UInt64 := 0x102
def STOP_UNMODELLED : UInt64 := 0x1FF

def undefinedF : Fault := { ec := EC_UNKNOWN }
def unmodelled (why : String) : Fault := { ec := STOP_UNMODELLED, why := why }

/-- A data abort's syndrome (mmu.rs `data_abort`): FAR is the address. -/
def dataAbort (sys : Sys) (va : UInt64) (a : Abort) (cm : Bool := false) : Fault :=
  { ec := if sys.el == 0 then EC_DABORT_LOWER else EC_DABORT
    iss := a.fsc.toUInt64 ||| (if a.write || cm then 0x40 else 0) ||| (if cm then 0x100 else 0)
    far := some va }

def instructionAbort (sys : Sys) (va : UInt64) (a : Abort) : Fault :=
  { ec := if sys.el == 0 then EC_IABORT_LOWER else EC_IABORT, iss := a.fsc.toUInt64, far := some va }

/-- system.rs `fp_trap` to EL1. -/
def fpTrapF : Fault := { ec := EC_FP, iss := ((1 : UInt64) <<< 24) ||| ((0xE : UInt64) <<< 20) }

-- ── PSTATE as SPSR (mod.rs `Pstate::to_spsr`) ─────────────────────────
def toSpsr (sys : Sys) (nzcv : UInt64) : UInt64 :=
  (nzcv <<< 28) ||| (if sys.ss then (1 : UInt64) <<< 21 else 0) ||| (if sys.il then (1 : UInt64) <<< 20 else 0)
  ||| sys.daif ||| (sys.el.toUInt64 <<< 2) ||| (if sys.spsel then 1 else 0)

/-- exception.rs `take_exception` to EL1 with vector `offset` (0 sync,
0x80 IRQ, 0x100 FIQ): the PC the processor goes on at. -/
def takeException (rf : ByteArray) (sys : Sys) (nzcv : UInt64) (offset retAddr : UInt64)
    (esr : Option UInt64) (far : Option UInt64) : ByteArray × Sys × UInt64 :=
  let quadrant : UInt64 := if sys.el == 0 then 0x400 else if sys.spsel then 0x200 else 0
  let spsr := toSpsr sys nzcv
  let sys := { sys with spsr := spsr, elr := retAddr }
  let sys := match esr with | some e => { sys with esr := e } | none => sys
  let sys := match far with | some f => { sys with far := f } | none => sys
  let (rf, sys) := switchSp rf sys 1 true
  let sys := { sys with daif := 0x3C0, ss := false, il := false, monArmed := false, activity := 0 }
  (rf, sys, branchAddr sys ((sys.vbar &&& ~~~0x7FF) + quadrant + offset))

/-- `finish`'s exception arm for the instruction at `pc` that raised `f`:
it retires (one tick), then the exception is taken (exception.rs
`take_sync_exception`). A stop retires nothing. -/
def raise (st : St) (pc : UInt64) (f : Fault) : St :=
  if f.ec >= 0x100 then
    let p : UInt8 := if f.ec == STOP_POWER_OFF then 1 else if f.ec == STOP_CPU_OFF then 2
      else if f.ec == STOP_RESET then 3 else 4
    { st with pc := pc, flow := 2, sys := { st.sys with power := p, stopWhy := f.why } }
  else
  let esr := (f.ec <<< 26) ||| ((1 : UInt64) <<< 25) ||| (f.iss &&& 0x1FF_FFFF)
  let ret := if f.next then pc + 4 else pc
  let (rf, sys, npc) := takeException st.rf st.sys st.nzcv 0 ret (some esr) f.far
  { st with rf := rf, sys := sys, pc := npc, icnt := st.icnt + 1, flow := 3 }

/-- Retire the instruction at `pc`, going on to the next. -/
@[inline] def nx (st : St) (pc : UInt64) : St := { st with pc := pc + 4, icnt := st.icnt + 1 }
/-- Retire a branch to `target` (exec.rs `retire`, `Flow::Jump`). -/
@[inline] def jmp (st : St) (target : UInt64) : St :=
  { st with pc := branchAddr st.sys target, icnt := st.icnt + 1, flow := 1 }

/-- mmu.rs `check_sp_alignment` for a base register `n`. -/
@[inline] def spAlignFault (sys : Sys) (n : UInt8) (base : UInt64) : Bool :=
  n == SP && base &&& 0xF != 0 &&
    (if sys.el == 0 then sys.sctlr &&& SCTLR_SA0 != 0 else sys.sctlr &&& SCTLR_SA != 0)

def spAlignF : Fault := { ec := EC_SP_ALIGN }

@[inline] def setR (st : St) (d : UInt8) (v : UInt64) : St := { st with rf := wr st.rf d v }

/-- The base register of a load or store (ldst.rs `base_register`). -/
@[inline] def baseReg (st : St) (n : UInt8) : Except Fault UInt64 :=
  let b := rr st.rf n
  if spAlignFault st.sys n b then .error spAlignF else .ok b

/-- Write a whole SIMD&FP register. -/
@[inline] def setV (st : St) (t : UInt8) (lo hi : UInt64) : St := { st with rf := wv st.rf t lo hi }

-- ── the exception return (exception.rs `exception_return`) ────────────
def exceptionReturn (st : St) : St :=
  let sys := st.sys
  let spsr := sys.spsr
  let target := sys.elr
  let m := spsr &&& 0x1F
  let tel := (m >>> 2) &&& 3
  let legal := m &&& 0x10 == 0 && m &&& 2 == 0 && tel <= sys.el.toUInt64 && tel != 2 && tel != 3
    && !(tel == 0 && m &&& 1 == 1)
  let nzcv := (spsr >>> 28) &&& 0xF
  let sys := { sys with monArmed := false, ss := false, daif := spsr &&& 0x3C0 }
  let (rf, sys) :=
    if legal then
      let (rf, sys) := switchSp st.rf sys tel.toUInt8 (m &&& 1 == 1)
      (rf, { sys with il := spsr &&& ((1 : UInt64) <<< 20) != 0 })
    else (st.rf, { sys with il := true })
  let sys := { sys with eventSet := true }
  { st with rf := rf, sys := sys, nzcv := nzcv, pc := branchAddr sys target, icnt := st.icnt + 1, flow := 3 }

end Leanbox
