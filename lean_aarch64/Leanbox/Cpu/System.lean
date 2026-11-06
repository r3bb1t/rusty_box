import Leanbox.Cpu.Memory
import Leanbox.Cpu.Sysreg
/-!
# System instructions

PSCI (system.rs `psci_call`) and the system instructions (system.rs
`exec_system`, `exec_hint`, `exec_exception_gen`, ERET): exception
generation, waits and hints, `MSR` (immediate), cache and TLB maintenance,
`DC ZVA`, and `MRS`/`MSR` of the system registers.
-/
namespace Leanbox

-- ── PSCI (system.rs `psci_call`) ──────────────────────────────────────
def PSCI_SUCCESS : UInt64 := 0
def PSCI_NOT_SUPPORTED : UInt64 := MISS
def PSCI_INVALID_PARAMETERS : UInt64 := MISS - 1
def PSCI_ALREADY_ON : UInt64 := MISS - 3
def PSCI_INVALID_ADDRESS : UInt64 := MISS - 8
def PSCI_AFFINITY : UInt64 := MPIDR_EL1 &&& 0xFF_00FF_FFFF

def psciSupported (id : UInt64) : Bool :=
  id == 0x8400_0000 || id == 0x8400_000A || id == 0x8400_0001 || id == 0xC400_0001
  || id == 0x8400_0002 || id == 0x8400_0003 || id == 0xC400_0003 || id == 0x8400_0004
  || id == 0xC400_0004 || id == 0x8400_0006 || id == 0x8400_0008 || id == 0x8400_0009

def psciAffinityInfo (target level : UInt64) : UInt64 :=
  let ignored : Option UInt64 := match level with
    | 0 => some 0 | 1 => some 0xFF | 2 => some 0xFFFF | 3 => some 0xFF_FFFF | _ => none
  match ignored with
  | none => PSCI_INVALID_PARAMETERS
  | some ig => if target &&& ~~~ig == PSCI_AFFINITY &&& ~~~ig then 0 else PSCI_INVALID_PARAMETERS

def psciCall (st : St) (pc : UInt64) : St :=
  let rf := st.rf
  let fn_ := rr rf 0 &&& 0xFFFF_FFFF
  let smc64 := fn_ &&& 0x4000_0000 != 0
  let a1 := if smc64 then rr rf 1 else rr rf 1 &&& 0xFFFF_FFFF
  let a2 := if smc64 then rr rf 2 else rr rf 2 &&& 0xFFFF_FFFF
  let set (v : UInt64) : St := nx { st with rf := wr rf 0 v } pc
  if fn_ == 0x8400_0000 then set 0x0001_0001
  else if fn_ == 0x8400_000A then set (if psciSupported (a1 &&& 0xFFFF_FFFF) then 0 else PSCI_NOT_SUPPORTED)
  else if fn_ == 0x8400_0001 || fn_ == 0xC400_0001 then
    if a1 &&& 0xFFFE_0000 != 0 then set PSCI_INVALID_PARAMETERS
    else
      -- entered as standby: wait for an interrupt (`begin_wait`)
      let st := nx { st with rf := wr rf 0 0 } pc
      { st with sys := { st.sys with activity := 1 }, flow := 2 }
  else if fn_ == 0x8400_0002 then raise st pc { ec := STOP_CPU_OFF }
  else if fn_ == 0x8400_0003 || fn_ == 0xC400_0003 then
    set (if a1 != PSCI_AFFINITY then PSCI_INVALID_PARAMETERS
         else if a2 &&& 3 != 0 then PSCI_INVALID_ADDRESS else PSCI_ALREADY_ON)
  else if fn_ == 0x8400_0004 || fn_ == 0xC400_0004 then set (psciAffinityInfo a1 (a2 &&& 0xFFFF_FFFF))
  else if fn_ == 0x8400_0006 then set 2
  else if fn_ == 0x8400_0008 then raise st pc { ec := STOP_POWER_OFF }
  else if fn_ == 0x8400_0009 then raise st pc { ec := STOP_RESET }
  else set PSCI_NOT_SUPPORTED

-- ── system instructions (system.rs) ───────────────────────────────────
/-- `ESR_ELx.ISS` of a trapped MSR/MRS/SYS (sysreg.rs `trap_iss`). -/
def trapIss (w : UInt32) (read : Bool) : UInt64 :=
  let k := (w >>> 5) &&& 0xFFFF
  let op0 := (k >>> 14).toUInt64
  let op1 := ((k >>> 11) &&& 7).toUInt64
  let crn := ((k >>> 7) &&& 15).toUInt64
  let crm := ((k >>> 3) &&& 15).toUInt64
  let op2 := (k &&& 7).toUInt64
  let rt := (w &&& 31).toUInt64
  (op0 <<< 20) ||| (op2 <<< 17) ||| (op1 <<< 14) ||| (crn <<< 10) ||| (rt <<< 5) ||| (crm <<< 1)
  ||| (if read then 1 else 0)

def sysFault (w : UInt32) (read : Bool) (e : SysErr) : Fault :=
  match e with
  | .undefined => undefinedF
  | .trap => { ec := EC_SYSREG, iss := trapIss w read }
  | .fpTrap => fpTrapF

/-- A cache maintenance by VA's translation (mmu.rs
`maintenance_translation`): `need` is the access kind it checks (none: 3). -/
def maintenance (st : St) (va : UInt64) (need : UInt8) : St × Option Fault :=
  let sys := st.sys
  if !mmuOn sys then
    let upper := (va >>> 55) &&& 1 == 1
    let addr := if tbi sys upper then va &&& 0x00FF_FFFF_FFFF_FFFF else va
    if addr >>> PA_BITS != 0 then (st, some (dataAbort sys va ⟨fscAddressSize 0, true⟩ true)) else (st, none)
  else
    let (sys, r) := tlbEntry sys st.mem va st.icnt false
    let st := { st with sys := sys }
    match r with
    | .error f => (st, some (dataAbort sys va ⟨f, true⟩ true))
    | .ok e =>
      if need != 3 && e &&& neededPerm need sys.el == 0 then
        (st, some (dataAbort sys va ⟨fscPermission ((e >>> ATTR_LEVEL_SHIFT) &&& 3), true⟩ true))
      else (st, none)

/-- The 64 zero bytes `DC ZVA` writes. -/
def zeroBlock : ByteArray := zeros 64

/-- mmu.rs `dc_zva`: 64 bytes of zeros, one copy when the block is in RAM,
eight doubleword bus writes otherwise. -/
def dcZva (st : St) (va : UInt64) : St × Option Fault :=
  let base := va &&& ~~~63
  let (sys, r) := translate st.sys st.mem base st.icnt AK_WRITE st.sys.el true
  let st := { st with sys := sys }
  match r with
  | .error a => (st, some (dataAbort st.sys va a))
  | .ok (pa, _) =>
    let st := noteStore st pa 64
    if inRam pa 64 then
      ({ st with mem := zeroBlock.copySlice 0 st.mem (pa - RAM_BASE).toNat 64 }, none)
    else
    let rec go (st : St) (i : Nat) : St × Option Fault :=
      if i ≥ 8 then (st, none) else
      let at_ := pa + 8 * i.toUInt64
      if inRam at_ 8 then go { st with mem := st64 st.mem (at_ - RAM_BASE).toUSize 0 } (i + 1)
      else
        let sys := st.sys
        let board := sys.board
        let sys := { sys with board := default }
        let (board, ok) := board.write at_ 3 0 st.icnt
        let st := { st with sys := { sys with board := board } }
        if ok then go st (i + 1) else (st, some (externalAbort st.sys va true))
    termination_by 8 - i
    go st 0

/-- system.rs `exec_system`, `exec_hint`, `exec_exception_gen`, ERET. -/
def execSys (st : St) (pc : UInt64) (w : UInt32) : St :=
  let sys := st.sys
  let el0 := sys.el == 0
  let rt := (w &&& 31).toUInt8
  let xt := if rt == 31 then 0 else rr st.rf rt
  if w &&& 0xFF00_0000 == 0xD400_0000 then                  -- exception generation
    let opc := (w >>> 21) &&& 7
    let ll := w &&& 3
    let imm := ((w >>> 5) &&& 0xFFFF).toUInt64
    if opc == 0 && ll == 1 then raise st pc { ec := EC_SVC, iss := imm, next := true }
    else if opc == 0 && ll == 2 then (if el0 then raise st pc undefinedF else psciCall st pc)
    else if opc == 0 && ll == 3 then raise st pc undefinedF   -- SMC: the conduit is HVC
    else raise st pc { ec := EC_BRK, iss := imm }
  else if w &&& 0xFFFF_FFFF == 0xD69F_03E0 then              -- ERET
    if el0 then raise st pc undefinedF else exceptionReturn st
  else
  let l := (w >>> 21) &&& 1
  let op0 := (w >>> 19) &&& 3
  let op1 := (w >>> 16) &&& 7
  let crn := (w >>> 12) &&& 15
  let crm := (w >>> 8) &&& 15
  let op2 := (w >>> 5) &&& 7
  if l == 0 && op0 == 0 then
    if crn == 2 then                                         -- WFE WFI SEV SEVL
      if op2 == 2 then
        let (sys, ev) := takeEvent sys st.icnt
        let st := { st with sys := sys }
        if ev then nx st pc
        else if el0 && sys.sctlr &&& SCTLR_NTWE == 0 then raise st pc { ec := EC_WFX, iss := ((1 : UInt64) <<< 24) ||| ((0xE : UInt64) <<< 20) ||| 1 }
        else if sys.monArmed then nx st pc
        else { nx st pc with sys := { sys with activity := 2 }, flow := 2 }
      else if op2 == 3 then
        if sys.irqLine || sys.fiqLine then nx st pc
        else if el0 && sys.sctlr &&& SCTLR_NTWI == 0 then raise st pc { ec := EC_WFX, iss := ((1 : UInt64) <<< 24) ||| ((0xE : UInt64) <<< 20) }
        else { nx st pc with sys := { sys with activity := 1 }, flow := 2 }
      else nx { st with sys := { sys with eventSet := true } } pc
    else if crn == 3 then                                    -- CLREX
      nx { st with sys := { sys with monArmed := false } } pc
    else                                                     -- MSR (immediate)
      let operand := crm &&& 1 != 0
      if op1 == 0 && op2 == 5 then
        if el0 then raise st pc undefinedF else
        let (rf, sys) := switchSp st.rf sys sys.el operand
        { nx { st with rf := rf, sys := sys } pc with flow := 3 }
      else if op1 == 3 && (op2 == 6 || op2 == 7) then
        if el0 && sys.sctlr &&& SCTLR_UMA == 0 then raise st pc { ec := EC_SYSREG, iss := trapIss w false }
        else
          let bits := crm.toUInt64 <<< 6
          let d := if op2 == 6 then sys.daif ||| bits else sys.daif &&& ~~~bits
          { nx { st with sys := { sys with daif := d } } pc with flow := 1 }
      else raise st pc undefinedF
  else if op0 == 1 then
    if l == 1 then raise st pc undefinedF                    -- SYSL
    else if crn == 11 || crn == 15 then raise st pc undefinedF   -- IMPLEMENTATION DEFINED space
    else if op1 == 0 && crn == 7 && (crm == 1 || crm == 5) && op2 == 0 then   -- IC IALLUIS, IALLU
      if el0 then raise st pc undefinedF else nx st pc
    else if op1 == 3 && crn == 7 && (crm == 5 || crm == 10 || crm == 11 || crm == 14) && op2 == 1 then
      -- IC IVAU, DC CVAC, DC CVAU, DC CIVAC
      if el0 && sys.sctlr &&& SCTLR_UCI == 0 then raise st pc { ec := EC_SYSREG, iss := trapIss w false } else
      let need : UInt8 := if crm == 5 then 3 else if sys.el == 0 then AK_READ else 3
      let (st, f) := maintenance st xt need
      match f with | some f => raise st pc f | none => nx st pc
    else if op1 == 3 && crn == 7 && (crm == 12 || crm == 13) && op2 == 1 then raise st pc undefinedF  -- DC CVAP, CVADP
    else if op1 == 3 && crn == 7 && crm == 3 then raise st pc undefinedF   -- prediction restrictions
    else if op1 == 3 && crn == 7 && crm == 4 && op2 == 1 then   -- DC ZVA
      if el0 && sys.sctlr &&& SCTLR_DZE == 0 then raise st pc { ec := EC_SYSREG, iss := trapIss w false } else
      let (st, f) := dcZva st xt
      match f with | some f => raise st pc f | none => nx st pc
    else if op1 == 0 && crn == 7 && crm == 6 && op2 == 1 then   -- DC IVAC
      if el0 then raise st pc undefinedF else
      let (st, f) := maintenance st xt AK_WRITE
      match f with | some f => raise st pc f | none => nx st pc
    else if op1 == 0 && crn == 7 && (crm == 6 || crm == 10 || crm == 14) && op2 == 2 then   -- DC ISW CSW CISW
      if el0 then raise st pc undefinedF else nx st pc
    else if op1 == 0 && crn == 7 && ((crm == 8 && op2 <= 3) || (crm == 9 && op2 <= 1)) then
      if el0 || crm == 9 then raise st pc undefinedF
      else raise st pc (unmodelled s!"AT at {hex pc}")
    else if op1 == 0 && crn == 8 then                        -- TLBI (EL1)
      let ok := (crm == 3 || crm == 7) && (op2 <= 3 || op2 == 5 || op2 == 7)
      if el0 || !ok then raise st pc undefinedF
      else { nx { st with sys := flushTlb sys } pc with flow := 3 }
    else raise st pc undefinedF
  else
    -- MSR / MRS (register)
    let k := (w >>> 5) &&& 0xFFFF
    if l == 1 then
      match sysregRead sys st.rf st.nzcv st.icnt k with
      | .ok (sys, v) =>
        let v := if k == K_SP_EL0 && !sys.spsel then v else v
        nx { st with sys := sys, rf := if rt == 31 then st.rf else wr st.rf rt v } pc
      | .error e => raise st pc (sysFault w true e)
    else
      match sysregWrite sys st.nzcv st.icnt k xt with
      | .ok (sys, nzcv, eff) =>
        let sys := if eff == 1 then { sys with timerDirty := true }
          else if eff == 2 then flushTlb sys else sys
        let st := nx { st with sys := sys, nzcv := nzcv } pc
        if eff == 3 && debugArmed st.sys then raise st pc (unmodelled s!"debug controls armed at {hex pc}")
        -- a timer write goes back to the run loop; a change of the translation
        -- context empties the micro-TLBs; anything else ends the trace
        else { st with flow := if eff == 1 then 2 else if eff == 2 then 3 else 1 }
      | .error e => raise st pc (sysFault w false e)

end Leanbox
