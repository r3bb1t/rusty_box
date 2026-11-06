import Leanbox.Board.Basic
import Leanbox.Isa.Op
/-!
# The processor's state

The processor's rarely changing state (cpu/mod.rs `Cpu`, `SysRegs`,
`Pstate` but NZCV) for a Cortex-A57 without EL2 (`NoEl2`), with the
machine's tables; the state a slow-path step works on (`St`); and the
register file.
-/
namespace Leanbox

/-- The trace cache's entries (a power of two). -/
def CACHE : Nat := 16384
/-- TLB sets: each holds a VA page tag and its translation. -/
def TLB_SETS : Nat := 1024

/-- The processor state the hot loop does not carry: PSTATE but NZCV, the
inactive stack pointers, the system registers, and the machine's tables. -/
structure Sys where
  -- PSTATE (cpu/mod.rs `Pstate`); `daif` in its SPSR position (bits 9:6)
  el : UInt8
  spsel : Bool
  daif : UInt64
  il : Bool
  ss : Bool
  /-- The stack pointers not in use: the active one is register slot 31. -/
  sp0 : UInt64
  sp1 : UInt64
  -- cpu/mod.rs `SysRegs`
  sctlr : UInt64
  actlr : UInt64
  cpacr : UInt64
  ttbr0 : UInt64
  ttbr1 : UInt64
  tcr : UInt64
  spsr : UInt64
  elr : UInt64
  esr : UInt64
  far : UInt64
  afsr0 : UInt64
  afsr1 : UInt64
  par : UInt64
  mair : UInt64
  amair : UInt64
  vbar : UInt64
  contextidr : UInt64
  tpidr0 : UInt64
  tpidrro0 : UInt64
  tpidr1 : UInt64
  csselr : UInt64
  cntkctl : UInt64
  cntfrq : UInt64
  cntpCtl : UInt64
  cntpCval : UInt64
  cntvCtl : UInt64
  cntvCval : UInt64
  mdscr : UInt64
  mdccint : UInt64
  oslsr : UInt64
  osdlr : UInt64
  oseccr : UInt64
  dbgprcr : UInt64
  dbgclaim : UInt64
  dbgbvr0 : UInt64
  dbgbvr1 : UInt64
  dbgbcr0 : UInt64
  dbgbcr1 : UInt64
  dbgwvr0 : UInt64
  dbgwvr1 : UInt64
  dbgwcr0 : UInt64
  dbgwcr1 : UInt64
  dtrrx : UInt64
  dtrtx : UInt64
  rvbar : UInt64
  rmr : UInt64
  fpcr : UInt64
  fpsr : UInt64
  -- wait.rs `EventRegister`
  eventSet : Bool
  eventFolded : UInt64
  -- mod.rs `ExclusiveMonitor`
  monArmed : Bool
  monAddr : UInt64
  monVa : UInt64
  monBytes : UInt64
  /-- 0 running, 1 waiting for an interrupt (WFI, PSCI CPU_SUSPEND), 2
  waiting for an event (WFE). -/
  activity : UInt8
  irqLine : Bool
  fiqLine : Bool
  timerDirty : Bool
  /-- Ticks that passed while the processor waited: retired = icount - waited. -/
  waited : UInt64
  /-- machine.rs `next_event`. -/
  nextEvent : UInt64
  /-- 0 running, 1 powered off, 2 processor off, 3 reset requested, 4 stopped
  on something this machine does not model. -/
  power : UInt8
  /-- Why the machine stopped, when `power` is 4. -/
  stopWhy : String
  /-- The stage 1 TLB: per set, the VA page (`MISS` when empty) and the PA
  page with the leaf's attributes in bits 11:0 (`tlbAttrs`). -/
  tlb : Array UInt64
  /-- The trace cache: the physical address each entry holds, and its ops. -/
  ctag : Array UInt64
  ctr : Array (Array Op)
  /-- Per 128-byte line of RAM, whether a cached trace holds code from it. -/
  cmark : ByteArray
  /-- Counter reads whose poll walk failed (Engine/Poll.lean), by PC. -/
  noPoll : Array UInt64
  board : Board
  deriving Inhabited

/-- Bits of a TLB entry's low half: the six permissions, Device memory, the
leaf's level. -/
def PERM_EL1_R : UInt64 := 1
def PERM_EL1_W : UInt64 := 2
def PERM_EL1_X : UInt64 := 4
def PERM_EL0_R : UInt64 := 8
def PERM_EL0_W : UInt64 := 16
def PERM_EL0_X : UInt64 := 32
def PERMS_ALL : UInt64 := 63
def ATTR_DEVICE : UInt64 := 64
def ATTR_LEVEL_SHIFT : UInt64 := 7

-- id.rs: the Cortex-A57's identification and reset values
def MIDR_CORTEX_A57 : UInt64 := 0x411F_D070
def MPIDR_EL1 : UInt64 := 0x8000_0000
def CTR_EL0 : UInt64 := 0x8444_C004
def DCZID_BLOCK_LOG2_WORDS : UInt64 := 4
def CLIDR_EL1 : UInt64 := 0x0A20_0023
def CCSIDR_L1D : UInt64 := 0x701F_E00A
def CCSIDR_L1I : UInt64 := 0x201F_E012
def CCSIDR_L2 : UInt64 := 0x70FF_E07A
def SCTLR_EL1_RESET : UInt64 := 0x00C5_0838
def OSLSR_EL1_RESET : UInt64 := 0b1010
/-- id.rs `id_aa64*` of `Features::CORTEX_A57` without EL2. -/
def ID_AA64PFR0 : UInt64 := 0x11
def ID_AA64DFR0 : UInt64 := 0x0010_1006
def ID_AA64ISAR0 : UInt64 := 0x1_1120
def ID_AA64MMFR0 : UInt64 := 0x0F00_0024
def DBGAUTHSTATUS : UInt64 := 0b1010
/-- id.rs `pa_bits`: PARange 0b0100. -/
def PA_BITS : UInt64 := 44

-- sctlr.rs
def SCTLR_M : UInt64 := 1
def SCTLR_A : UInt64 := 2
def SCTLR_C : UInt64 := 4
def SCTLR_SA : UInt64 := 8
def SCTLR_SA0 : UInt64 := 16
def SCTLR_UMA : UInt64 := (1 : UInt64) <<< 9
def SCTLR_DZE : UInt64 := (1 : UInt64) <<< 14
def SCTLR_UCT : UInt64 := (1 : UInt64) <<< 15
def SCTLR_NTWI : UInt64 := (1 : UInt64) <<< 16
def SCTLR_NTWE : UInt64 := (1 : UInt64) <<< 18
def SCTLR_WXN : UInt64 := (1 : UInt64) <<< 19
def SCTLR_UCI : UInt64 := (1 : UInt64) <<< 26

/-- state.rs `reset_keeping_time` with mod.rs `SysRegs::reset` and
`Pstate::reset::<NoEl2>`, at tick `icount`. -/
def Sys.reset (resetPc icount : UInt64) (board : Board) : Sys :=
  { el := 1, spsel := true, daif := 0x3C0, il := false, ss := false
    sp0 := 0, sp1 := 0
    sctlr := SCTLR_EL1_RESET, actlr := 0, cpacr := 0, ttbr0 := 0, ttbr1 := 0, tcr := 0
    spsr := 0, elr := 0, esr := 0, far := 0, afsr0 := 0, afsr1 := 0, par := 0, mair := 0
    amair := 0, vbar := 0, contextidr := 0, tpidr0 := 0, tpidrro0 := 0, tpidr1 := 0
    csselr := 0, cntkctl := 0, cntfrq := COUNTER_HZ.toUInt64
    cntpCtl := 0, cntpCval := 0, cntvCtl := 0, cntvCval := 0
    mdscr := 0, mdccint := 0, oslsr := OSLSR_EL1_RESET, osdlr := 0, oseccr := 0
    dbgprcr := 0, dbgclaim := 0
    dbgbvr0 := 0, dbgbvr1 := 0, dbgbcr0 := 0, dbgbcr1 := 0
    dbgwvr0 := 0, dbgwvr1 := 0, dbgwcr0 := 0, dbgwcr1 := 0
    dtrrx := 0, dtrtx := 0, rvbar := resetPc, rmr := 0, fpcr := 0, fpsr := 0
    eventSet := false
    eventFolded := (icount.toNat * COUNTER_HZ / IPS).toUInt64
    monArmed := false, monAddr := 0, monVa := 0, monBytes := 0
    activity := 0, irqLine := false, fiqLine := false, timerDirty := false
    waited := 0, nextEvent := MISS, power := 0, stopWhy := ""
    tlb := Array.replicate (2 * TLB_SETS) MISS
    ctag := Array.replicate CACHE MISS
    ctr := Array.replicate CACHE #[]
    cmark := zeros (RAM_SIZE.toNat / 128)
    noPoll := Array.replicate 512 MISS
    board := board }

/-- The state a slow-path step works on and returns. -/
structure St where
  rf : ByteArray
  mem : ByteArray
  sys : Sys
  /-- The PC of the next instruction. -/
  pc : UInt64
  nzcv : UInt64
  icnt : UInt64
  /-- 0: the next instruction follows in the trace; 1: go on at `pc` through
  a trace lookup; 3: the same after a change of translation, exception
  level or interrupt masks (the micro-TLBs are emptied); 2: go back to the
  run loop (a device, a timer write, a wait, a stop). -/
  flow : UInt8
  deriving Inhabited

-- ── register file ─────────────────────────────────────────────────────
@[inline] def rr (rf : ByteArray) (s : UInt8) : UInt64 := ld64 rf (s.toUSize * 8)
@[inline] def wr (rf : ByteArray) (s : UInt8) (v : UInt64) : ByteArray := st64 rf (s.toUSize * 8) v
@[inline] def vlo (rf : ByteArray) (v : UInt8) : UInt64 := ld64 rf (VBASE + v.toUSize * 16)
@[inline] def vhi (rf : ByteArray) (v : UInt8) : UInt64 := ld64 rf (VBASE + v.toUSize * 16 + 8)
/-- Write a whole SIMD&FP register (low and high halves). -/
@[inline] def wv (rf : ByteArray) (v : UInt8) (lo hi : UInt64) : ByteArray :=
  st64 (st64 rf (VBASE + v.toUSize * 16) lo) (VBASE + v.toUSize * 16 + 8) hi

/-- `SP_EL0` and `SP_EL1` as they are, whichever is in use. -/
def spEl0 (rf : ByteArray) (sys : Sys) : UInt64 :=
  if sys.el == 0 || !sys.spsel then rr rf SP else sys.sp0
def spEl1 (rf : ByteArray) (sys : Sys) : UInt64 :=
  if sys.el == 1 && sys.spsel then rr rf SP else sys.sp1

/-- Move to exception level `el` with stack pointer selection `spsel`:
the stack pointer in use is saved and the new one loaded into slot 31. -/
def switchSp (rf : ByteArray) (sys : Sys) (el : UInt8) (spsel : Bool) : ByteArray × Sys :=
  let cur := rr rf SP
  let sys := if sys.el == 1 && sys.spsel then { sys with sp1 := cur } else { sys with sp0 := cur }
  let sys := { sys with el := el, spsel := spsel }
  let nsp := if el == 1 && spsel then sys.sp1 else sys.sp0
  (wr rf SP nsp, sys)

end Leanbox
