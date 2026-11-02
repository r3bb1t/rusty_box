import Leanbox.Cpu.Bus
/-
The generic timer (cpu/timer.rs), the event register (cpu/wait.rs) and the
system registers an `MRS`/`MSR` reaches (cpu/sysreg.rs `sysreg_read_as`,
`sysreg_write_as`), for a Cortex-A57 without EL2: no EL2 control traps, no
counter offsets, no hypervisor timer.
-/
namespace Leanbox

-- ── time ──────────────────────────────────────────────────────────────
/-- timer.rs `counter_at`: floor(icount * counter_hz / ips). -/
@[inline] def counterAt (icnt : UInt64) : UInt64 :=
  -- counter_hz / ips = 5/4 exactly, so this is floor(icnt * 5 / 4)
  (icnt >>> 2) * (5 : UInt64) + (((icnt &&& 3) * (5 : UInt64)) >>> 2)

/-- rusty_box_core time.rs `saturate`. -/
def saturate (n : Nat) : UInt64 := if n ≥ UInt64.size then MISS else n.toUInt64

/-- timer.rs `tick_reaching`: the first tick whose count reaches `count`
(`ticks_from_cycles_ceil`), `none` at u64::MAX. -/
def tickReaching (count : UInt64) : Option UInt64 :=
  let t := saturate ((count.toNat * IPS + COUNTER_HZ - 1) / COUNTER_HZ)
  if t == MISS then none else some t

/-- timer.rs `timer_change` for a timer with no offset. -/
def timerChange (ctl cval now : UInt64) : Option UInt64 :=
  if ctl &&& 1 == 0 || ctl &&& 2 != 0 then none
  else if now < cval then
    let target := now.toNat + (cval - now).toNat
    if target ≥ UInt64.size then none else tickReaching target.toUInt64
  else none

/-- timer.rs `next_timer_deadline`. -/
def nextTimerDeadline (sys : Sys) (icnt : UInt64) : Option UInt64 :=
  let now := counterAt icnt
  minDeadline (timerChange sys.cntpCtl sys.cntpCval now) (timerChange sys.cntvCtl sys.cntvCval now)

/-- timer.rs `condition_met`. -/
@[inline] def conditionMet (ctl cval count : UInt64) : Bool := ctl &&& 1 != 0 && count >= cval
/-- timer.rs `timer_line`. -/
@[inline] def timerLine (ctl cval count : UInt64) : Bool := ctl &&& 2 == 0 && conditionMet ctl cval count
/-- timer.rs `timer_ctl_read`. -/
def timerCtlRead (ctl cval count : UInt64) : UInt64 :=
  (ctl &&& 3) ||| (if conditionMet ctl cval count then 4 else 0)

-- ── the event register: wait.rs ───────────────────────────────────────
/-- wait.rs `stream_edge_after` for the virtual stream (no offset). -/
def streamEdgeAfter (ctl after : UInt64) : Option UInt64 :=
  if ctl &&& 4 == 0 then none else
  let n := (ctl >>> 4) &&& 0xF
  let half := (1 : UInt64) <<< n
  let period := half <<< 1
  let edge := if ctl &&& 8 == 0 then half else 0
  let phase := edge &&& (period - 1)
  let candidate := after - after % period + phase
  if candidate > after then some candidate
  else if candidate.toNat + period.toNat ≥ UInt64.size then none
  else some (candidate + period)

/-- wait.rs `fold_event_stream`. -/
def foldEventStream (sys : Sys) (icnt : UInt64) : Sys :=
  let now := counterAt icnt
  let sys := match streamEdgeAfter sys.cntkctl sys.eventFolded with
    | some c => if c <= now then { sys with eventSet := true } else sys
    | none => sys
  { sys with eventFolded := now }

/-- wait.rs `take_event`. -/
def takeEvent (sys : Sys) (icnt : UInt64) : Sys × Bool :=
  let sys := foldEventStream sys icnt
  ({ sys with eventSet := false }, sys.eventSet)

-- ── system registers: sysreg.rs ───────────────────────────────────────
@[inline] def key (op0 op1 crn crm op2 : UInt32) : UInt32 :=
  (op0 <<< 14) ||| (op1 <<< 11) ||| (crn <<< 7) ||| (crm <<< 3) ||| op2

def K_MIDR := key 3 0 0 0 0
def K_MPIDR := key 3 0 0 0 5
def K_REVIDR := key 3 0 0 0 6
def K_PFR0 := key 3 0 0 4 0
def K_PFR1 := key 3 0 0 4 1
def K_ZFR0 := key 3 0 0 4 4
def K_SMFR0 := key 3 0 0 4 5
def K_DFR0 := key 3 0 0 5 0
def K_ISAR0 := key 3 0 0 6 0
def K_ISAR1 := key 3 0 0 6 1
def K_ISAR2 := key 3 0 0 6 2
def K_MMFR0 := key 3 0 0 7 0
def K_MMFR1 := key 3 0 0 7 1
def K_MMFR2 := key 3 0 0 7 2
def K_CCSIDR := key 3 1 0 0 0
def K_CLIDR := key 3 1 0 0 1
def K_AIDR := key 3 1 0 0 7
def K_CSSELR := key 3 2 0 0 0
def K_CTR := key 3 3 0 0 1
def K_DCZID := key 3 3 0 0 7
def K_SCTLR := key 3 0 1 0 0
def K_ACTLR := key 3 0 1 0 1
def K_CPACR := key 3 0 1 0 2
def K_TTBR0 := key 3 0 2 0 0
def K_TTBR1 := key 3 0 2 0 1
def K_TCR := key 3 0 2 0 2
def K_SPSR := key 3 0 4 0 0
def K_ELR := key 3 0 4 0 1
def K_SP_EL0 := key 3 0 4 1 0
def K_SPSEL := key 3 0 4 2 0
def K_CURRENT_EL := key 3 0 4 2 2
def K_NZCV := key 3 3 4 2 0
def K_DAIF := key 3 3 4 2 1
def K_FPCR := key 3 3 4 4 0
def K_FPSR := key 3 3 4 4 1
def K_AFSR0 := key 3 0 5 1 0
def K_AFSR1 := key 3 0 5 1 1
def K_ESR := key 3 0 5 2 0
def K_FAR := key 3 0 6 0 0
def K_PAR := key 3 0 7 4 0
def K_MAIR := key 3 0 10 2 0
def K_AMAIR := key 3 0 10 3 0
def K_VBAR := key 3 0 12 0 0
def K_RVBAR := key 3 0 12 0 1
def K_RMR := key 3 0 12 0 2
def K_ISR := key 3 0 12 1 0
def K_CONTEXTIDR := key 3 0 13 0 1
def K_TPIDR_EL1 := key 3 0 13 0 4
def K_TPIDR_EL0 := key 3 3 13 0 2
def K_TPIDRRO_EL0 := key 3 3 13 0 3
def K_CNTKCTL := key 3 0 14 1 0
def K_CNTFRQ := key 3 3 14 0 0
def K_CNTPCT := key 3 3 14 0 1
def K_CNTVCT := key 3 3 14 0 2
def K_CNTP_TVAL := key 3 3 14 2 0
def K_CNTP_CTL := key 3 3 14 2 1
def K_CNTP_CVAL := key 3 3 14 2 2
def K_CNTV_TVAL := key 3 3 14 3 0
def K_CNTV_CTL := key 3 3 14 3 1
def K_CNTV_CVAL := key 3 3 14 3 2
def K_MDCCINT := key 2 0 0 2 0
def K_MDSCR := key 2 0 0 2 2
def K_OSECCR := key 2 0 0 6 2
def K_MDCCSR := key 2 3 0 1 0
def K_OSDTRRX := key 2 0 0 0 2
def K_OSDTRTX := key 2 0 0 3 2
def K_DBGDTR := key 2 3 0 4 0
def K_DBGDTRTX := key 2 3 0 5 0
def K_MDRAR := key 2 0 1 0 0
def K_OSLAR := key 2 0 1 0 4
def K_OSLSR := key 2 0 1 1 4
def K_OSDLR := key 2 0 1 3 4
def K_DBGPRCR := key 2 0 1 4 4
def K_DBGCLAIMSET := key 2 0 7 8 6
def K_DBGCLAIMCLR := key 2 0 7 9 6
def K_DBGAUTHSTATUS := key 2 0 7 14 6

-- sysreg.rs constants
def EL0PCTEN : UInt64 := 1
def EL0VCTEN : UInt64 := 2
def EL0VTEN : UInt64 := (1 : UInt64) <<< 8
def EL0PTEN : UInt64 := (1 : UInt64) <<< 9
def CNTKCTL_BASE : UInt64 := 0x3FF
def MDSCR_SS : UInt64 := 1
def MDSCR_TDCC : UInt64 := (1 : UInt64) <<< 12
def MDSCR_KDE : UInt64 := (1 : UInt64) <<< 13
def MDSCR_MDE : UInt64 := (1 : UInt64) <<< 15
def DCC_TXFULL : UInt64 := (1 : UInt64) <<< 29
def DCC_RXFULL : UInt64 := (1 : UInt64) <<< 30
def DCC_FLAGS : UInt64 := DCC_TXFULL ||| DCC_RXFULL
def MDSCR_SAVE_RESTORE : UInt64 :=
  DCC_FLAGS ||| ((1 : UInt64) <<< 27) ||| ((1 : UInt64) <<< 26) ||| ((3 : UInt64) <<< 22) ||| ((1 : UInt64) <<< 21) ||| ((1 : UInt64) <<< 14) ||| ((1 : UInt64) <<< 6)
def MDSCR_FIELDS : UInt64 := MDSCR_SAVE_RESTORE ||| MDSCR_SS ||| MDSCR_TDCC ||| MDSCR_KDE ||| MDSCR_MDE
def OSLSR_OSLK : UInt64 := 2
def DBGBCR_FIELDS : UInt64 := 0x00FF_E1E7
def DBGWCR_FIELDS : UInt64 := 0x1F1F_FFFF
def DBGCLAIM_TAGS : UInt64 := 0xFF
def SCTLR_MTE : UInt64 :=
  ((1 : UInt64) <<< 37) ||| ((3 : UInt64) <<< 38) ||| ((3 : UInt64) <<< 40) ||| ((1 : UInt64) <<< 42) ||| ((1 : UInt64) <<< 43) ||| ((1 : UInt64) <<< 58) ||| ((1 : UInt64) <<< 59)
def CPACR_FPEN : UInt64 := (3 : UInt64) <<< 20
/-- state.rs `spsr_mask` of a processor with none of BTI, PAN, UAO, DIT, SSBS. -/
def SPSR_MASK : UInt64 := 0xF030_03DF
/-- state.rs `fpcr_mask` without FP16 or AFP. -/
def FPCR_MASK : UInt64 := 0x07C0_0000
/-- state.rs `FPSR_MASK`. -/
def FPSR_MASK : UInt64 := 0xF800_009F

/-- What a system register access ends in. -/
inductive SysErr where
  | undefined
  /-- A trap to EL1 with EC 0x18 (sysreg.rs `Route::Class`). -/
  | trap
  /-- A SIMD&FP access trap (`fp_trap`). -/
  | fpTrap

/-- system.rs `cpacr_fp_enabled_at`. -/
def fpEnabled (sys : Sys) : Bool :=
  match (sys.cpacr &&& CPACR_FPEN) >>> 20 with
  | 3 => true
  | 1 => sys.el == 1
  | _ => false

/-- sysreg.rs `DebugPoint::of`: (0 BVR, 1 BCR, 2 WVR, 3 WCR; n). -/
def debugPoint (k : UInt32) : Option (UInt8 × UInt8) :=
  let op0 := k >>> 14
  let op1 := (k >>> 11) &&& 7
  let crn := (k >>> 7) &&& 15
  let crm := (k >>> 3) &&& 15
  let op2 := k &&& 7
  if op0 != 2 || op1 != 0 || crn != 0 || crm > 1 || op2 < 4 then none
  else some ((op2 - 4).toUInt8, crm.toUInt8)

def ccsidr (sys : Sys) : UInt64 :=
  match sys.csselr &&& 0xF with
  | 0 => CCSIDR_L1D | 1 => CCSIDR_L1I | 2 => CCSIDR_L2 | _ => 0

/-- sysreg.rs `sysreg_read_as` for a guest access at the current EL, with
the side effect of `sysreg_mrs`; `sp` is SP_EL0 when it is not in use. -/
def sysregRead (sys : Sys) (rf : ByteArray) (nzcv icnt : UInt64) (k : UInt32) : Except SysErr (Sys × UInt64) :=
  let el0 := sys.el == 0
  let ok (v : UInt64) : Except SysErr (Sys × UInt64) := .ok (sys, v)
  let trap1 (c : Bool) (v : Unit → UInt64) : Except SysErr (Sys × UInt64) :=
    if c then .error .trap else .ok (sys, v ())
  let count := counterAt icnt
  if k == K_NZCV then ok (nzcv <<< 28)
  else if k == K_DAIF then trap1 (el0 && sys.sctlr &&& SCTLR_UMA == 0) (fun _ => sys.daif)
  else if k == K_FPCR || k == K_FPSR then
    if !fpEnabled sys then .error .fpTrap else ok (if k == K_FPCR then sys.fpcr else sys.fpsr)
  else if k == K_CTR then trap1 (el0 && sys.sctlr &&& SCTLR_UCT == 0) (fun _ => CTR_EL0)
  else if k == K_DCZID then
    ok (DCZID_BLOCK_LOG2_WORDS ||| (if el0 && sys.sctlr &&& SCTLR_DZE == 0 then 16 else 0))
  else if k == K_TPIDR_EL0 then ok sys.tpidr0
  else if k == K_TPIDRRO_EL0 then ok sys.tpidrro0
  else if k == K_CNTFRQ then trap1 (el0 && sys.cntkctl &&& (EL0PCTEN ||| EL0VCTEN) == 0) (fun _ => sys.cntfrq)
  else if k == K_CNTPCT then trap1 (el0 && sys.cntkctl &&& EL0PCTEN == 0) (fun _ => count)
  else if k == K_CNTVCT then trap1 (el0 && sys.cntkctl &&& EL0VCTEN == 0) (fun _ => count)
  else if k == K_CNTP_CTL || k == K_CNTP_CVAL || k == K_CNTP_TVAL then
    trap1 (el0 && sys.cntkctl &&& EL0PTEN == 0) fun _ =>
      if k == K_CNTP_CTL then timerCtlRead sys.cntpCtl sys.cntpCval count
      else if k == K_CNTP_CVAL then sys.cntpCval
      else (sys.cntpCval - count) &&& 0xFFFF_FFFF
  else if k == K_CNTV_CTL || k == K_CNTV_CVAL || k == K_CNTV_TVAL then
    trap1 (el0 && sys.cntkctl &&& EL0VTEN == 0) fun _ =>
      if k == K_CNTV_CTL then timerCtlRead sys.cntvCtl sys.cntvCval count
      else if k == K_CNTV_CVAL then sys.cntvCval
      else (sys.cntvCval - count) &&& 0xFFFF_FFFF
  else if k == K_MDCCSR then trap1 (el0 && sys.mdscr &&& MDSCR_TDCC != 0) (fun _ => sys.mdscr &&& DCC_FLAGS)
  else if k == K_DBGDTR || k == K_DBGDTRTX then
    if el0 && sys.mdscr &&& MDSCR_TDCC != 0 then .error .trap else
    let v : UInt64 := if sys.mdscr &&& DCC_RXFULL == 0 then 0
      else if k == K_DBGDTR then (sys.dtrtx <<< 32) ||| sys.dtrrx else sys.dtrrx
    -- sysreg_mrs: the read empties DTRRX
    .ok ({ sys with mdscr := sys.mdscr &&& ~~~DCC_RXFULL }, v)
  else if el0 then .error .undefined
  else
  let op0 := k >>> 14
  let op1 := (k >>> 11) &&& 7
  let crn := (k >>> 7) &&& 15
  let crm := (k >>> 3) &&& 15
  -- the IMPLEMENTATION DEFINED space, and EL2's registers below EL2
  if (op0 == 1 || op0 == 3) && (crn == 11 || crn == 15) then .error .undefined
  else if op1 == 4 then .error .undefined
  else
  let v : Option UInt64 :=
    if k == K_MIDR then some MIDR_CORTEX_A57
    else if k == K_MPIDR then some MPIDR_EL1
    else if k == K_REVIDR then some 0
    else if k == K_PFR0 then some ID_AA64PFR0
    else if k == K_PFR1 || k == K_ZFR0 || k == K_SMFR0 then some 0
    else if k == K_DFR0 then some ID_AA64DFR0
    else if k == K_ISAR0 then some ID_AA64ISAR0
    else if k == K_ISAR1 || k == K_ISAR2 then some 0
    else if k == K_MMFR0 then some ID_AA64MMFR0
    else if k == K_MMFR1 || k == K_MMFR2 then some 0
    else if k == K_CCSIDR then some (ccsidr sys)
    else if k == K_CLIDR then some CLIDR_EL1
    else if k == K_AIDR then some 0
    else if k == K_CSSELR then some sys.csselr
    else if k == K_SCTLR then some sys.sctlr
    else if k == K_ACTLR then some sys.actlr
    else if k == K_CPACR then some sys.cpacr
    else if k == K_TTBR0 then some sys.ttbr0
    else if k == K_TTBR1 then some sys.ttbr1
    else if k == K_TCR then some sys.tcr
    else if k == K_SPSR then some sys.spsr
    else if k == K_ELR then some sys.elr
    else if k == K_SP_EL0 then (if !sys.spsel then none else some sys.sp0)
    else if k == K_SPSEL then some (if sys.spsel then 1 else 0)
    else if k == K_CURRENT_EL then some (sys.el.toUInt64 <<< 2)
    else if k == K_AFSR0 then some sys.afsr0
    else if k == K_AFSR1 then some sys.afsr1
    else if k == K_ESR then some sys.esr
    else if k == K_FAR then some sys.far
    else if k == K_PAR then some sys.par
    else if k == K_MAIR then some sys.mair
    else if k == K_AMAIR then some sys.amair
    else if k == K_VBAR then some sys.vbar
    else if k == K_RVBAR then some sys.rvbar
    else if k == K_RMR then some (1 ||| sys.rmr)
    else if k == K_ISR then some ((if sys.irqLine then 0x80 else 0) ||| (if sys.fiqLine then 0x40 else 0))
    else if k == K_CONTEXTIDR then some sys.contextidr
    else if k == K_TPIDR_EL1 then some sys.tpidr1
    else if k == K_CNTKCTL then some sys.cntkctl
    else if k == K_MDCCINT then some sys.mdccint
    else if k == K_MDSCR then some sys.mdscr
    else if k == K_OSDTRRX then some sys.dtrrx
    else if k == K_OSDTRTX then some sys.dtrtx
    else if k == K_OSECCR then some sys.oseccr
    else if k == K_OSLSR then some sys.oslsr
    else if k == K_OSDLR then some sys.osdlr
    else if k == K_DBGPRCR then some sys.dbgprcr
    else if k == K_MDRAR then some 0
    else if k == K_DBGCLAIMSET then some DBGCLAIM_TAGS
    else if k == K_DBGCLAIMCLR then some sys.dbgclaim
    else if k == K_DBGAUTHSTATUS then some DBGAUTHSTATUS
    else match debugPoint k with
      | some (r, n) =>
        some (match r, n with
          | 0, 0 => sys.dbgbvr0 | 0, _ => sys.dbgbvr1
          | 1, 0 => sys.dbgbcr0 | 1, _ => sys.dbgbcr1
          | 2, 0 => sys.dbgwvr0 | 2, _ => sys.dbgwvr1
          | _, 0 => sys.dbgwcr0 | _, _ => sys.dbgwcr1)
      | none =>
        -- the unallocated ID register space reads as zero
        if op0 == 3 && op1 == 0 && crn == 0 && crm >= 1 && crm <= 7 then some 0 else none
  match v with
  | some v => .ok (sys, v)
  | none => .error .undefined

/-- What an `MSR` changes beyond the register: 0 nothing, 1 the timers, 2
the translation context (the TLB is emptied), 3 the debug controls. -/
def sysregWrite (sys : Sys) (nzcv icnt : UInt64) (k : UInt32) (value : UInt64) :
    Except SysErr (Sys × UInt64 × UInt8) :=
  let el0 := sys.el == 0
  let count := counterAt icnt
  let done (s : Sys) : Except SysErr (Sys × UInt64 × UInt8) := .ok (s, nzcv, 0)
  if k == K_NZCV then .ok (sys, (value >>> 28) &&& 0xF, 0)
  else if k == K_DAIF then
    if el0 && sys.sctlr &&& SCTLR_UMA == 0 then .error .trap
    else done { sys with daif := value &&& 0x3C0 }
  else if k == K_FPCR || k == K_FPSR then
    if !fpEnabled sys then .error .fpTrap
    else if k == K_FPCR then done { sys with fpcr := value &&& FPCR_MASK }
    else done { sys with fpsr := value &&& FPSR_MASK }
  else if k == K_TPIDR_EL0 then done { sys with tpidr0 := value }
  else if k == K_CNTP_CTL || k == K_CNTP_CVAL || k == K_CNTP_TVAL then
    if el0 && sys.cntkctl &&& EL0PTEN == 0 then .error .trap else
    let sys :=
      if k == K_CNTP_CTL then { sys with cntpCtl := value &&& 3 }
      else if k == K_CNTP_CVAL then { sys with cntpCval := value }
      else { sys with cntpCval := count + sext (value &&& 0xFFFF_FFFF) 32 }
    .ok (sys, nzcv, 1)
  else if k == K_CNTV_CTL || k == K_CNTV_CVAL || k == K_CNTV_TVAL then
    if el0 && sys.cntkctl &&& EL0VTEN == 0 then .error .trap else
    let sys :=
      if k == K_CNTV_CTL then { sys with cntvCtl := value &&& 3 }
      else if k == K_CNTV_CVAL then { sys with cntvCval := value }
      else { sys with cntvCval := count + sext (value &&& 0xFFFF_FFFF) 32 }
    .ok (sys, nzcv, 1)
  else if k == K_DBGDTR || k == K_DBGDTRTX then
    if el0 && sys.mdscr &&& MDSCR_TDCC != 0 then .error .trap else
    let sys := if sys.mdscr &&& DCC_TXFULL == 0 then
        let sys := { sys with dtrtx := value &&& 0xFFFF_FFFF }
        if k == K_DBGDTR then { sys with dtrrx := value >>> 32 } else sys
      else sys
    done { sys with mdscr := sys.mdscr ||| DCC_TXFULL }
  else if el0 then .error .undefined
  else
  let op0 := k >>> 14
  let op1 := (k >>> 11) &&& 7
  let crn := (k >>> 7) &&& 15
  if (op0 == 1 || op0 == 3) && (crn == 11 || crn == 15) then .error .undefined
  else if op1 == 4 then .error .undefined
  else
  -- a write of a translation register that changes it changes the context
  let ctx (s : Sys) (changed : Bool) : Except SysErr (Sys × UInt64 × UInt8) :=
    .ok (s, nzcv, if changed then 2 else 0)
  if k == K_CSSELR then done { sys with csselr := value &&& 0xF }
  else if k == K_SCTLR then
    let v := value &&& ~~~SCTLR_MTE
    ctx { sys with sctlr := v } (v != sys.sctlr)
  else if k == K_ACTLR then done { sys with actlr := value }
  else if k == K_CPACR then done { sys with cpacr := value &&& CPACR_FPEN }
  else if k == K_TTBR0 then ctx { sys with ttbr0 := value } (value != sys.ttbr0)
  else if k == K_TTBR1 then ctx { sys with ttbr1 := value } (value != sys.ttbr1)
  else if k == K_TCR then ctx { sys with tcr := value } (value != sys.tcr)
  else if k == K_SPSR then done { sys with spsr := value &&& SPSR_MASK }
  else if k == K_ELR then done { sys with elr := value }
  else if k == K_SP_EL0 then
    if !sys.spsel then .error .undefined else done { sys with sp0 := value }
  else if k == K_AFSR0 then done { sys with afsr0 := value }
  else if k == K_AFSR1 then done { sys with afsr1 := value }
  else if k == K_ESR then done { sys with esr := value &&& 0xFFFF_FFFF }
  else if k == K_FAR then done { sys with far := value }
  else if k == K_PAR then done { sys with par := value }
  else if k == K_MAIR then ctx { sys with mair := value } (value != sys.mair)
  else if k == K_AMAIR then done { sys with amair := value }
  else if k == K_VBAR then done { sys with vbar := value &&& ~~~0x7FF }
  else if k == K_RMR then done { sys with rmr := value &&& 2 }
  else if k == K_CONTEXTIDR then done { sys with contextidr := value &&& 0xFFFF_FFFF }
  else if k == K_TPIDR_EL1 then done { sys with tpidr1 := value }
  else if k == K_TPIDRRO_EL0 then done { sys with tpidrro0 := value }
  else if k == K_CNTKCTL then
    let sys := foldEventStream sys icnt
    done { sys with cntkctl := value &&& CNTKCTL_BASE }
  else if k == K_CNTFRQ then done { sys with cntfrq := value &&& 0xFFFF_FFFF }
  else if k == K_MDCCINT then done { sys with mdccint := value }
  else if k == K_MDSCR then
    let kept := if sys.oslsr &&& OSLSR_OSLK != 0 then 0 else MDSCR_SAVE_RESTORE
    .ok ({ sys with mdscr := (value &&& MDSCR_FIELDS &&& ~~~kept) ||| (sys.mdscr &&& kept) }, nzcv, 3)
  else if k == K_OSDTRRX then done { sys with dtrrx := value &&& 0xFFFF_FFFF }
  else if k == K_OSDTRTX then done { sys with dtrtx := value &&& 0xFFFF_FFFF }
  else if k == K_OSECCR then
    done (if sys.oslsr &&& OSLSR_OSLK != 0 then { sys with oseccr := value &&& 0xFFFF_FFFF } else sys)
  else if k == K_OSLAR then done { sys with oslsr := (sys.oslsr &&& ~~~OSLSR_OSLK) ||| ((value &&& 1) <<< 1) }
  else if k == K_OSDLR then done { sys with osdlr := value &&& 1 }
  else if k == K_DBGPRCR then done { sys with dbgprcr := value &&& 1 }
  else if k == K_DBGCLAIMSET then done { sys with dbgclaim := sys.dbgclaim ||| (value &&& DBGCLAIM_TAGS) }
  else if k == K_DBGCLAIMCLR then done { sys with dbgclaim := sys.dbgclaim &&& ~~~(value &&& DBGCLAIM_TAGS) }
  else match debugPoint k with
    | some (r, n) =>
      let sys := match r, n with
        | 0, 0 => { sys with dbgbvr0 := value } | 0, _ => { sys with dbgbvr1 := value }
        | 1, 0 => { sys with dbgbcr0 := value &&& DBGBCR_FIELDS }
        | 1, _ => { sys with dbgbcr1 := value &&& DBGBCR_FIELDS }
        | 2, 0 => { sys with dbgwvr0 := value &&& ~~~3 } | 2, _ => { sys with dbgwvr1 := value &&& ~~~3 }
        | _, 0 => { sys with dbgwcr0 := value &&& DBGWCR_FIELDS }
        | _, _ => { sys with dbgwcr1 := value &&& DBGWCR_FIELDS }
      .ok (sys, nzcv, if r == 1 || r == 3 then 3 else 0)
    | none => .error .undefined

/-- debug.rs `derive_debug_controls`: whether breakpoints, watchpoints or
software step are armed, which this machine does not model. -/
def debugArmed (sys : Sys) : Bool :=
  let mde := sys.mdscr &&& MDSCR_MDE != 0
  sys.mdscr &&& MDSCR_SS != 0
  || (mde && (sys.dbgbcr0 &&& 1 != 0 || sys.dbgbcr1 &&& 1 != 0
              || sys.dbgwcr0 &&& 1 != 0 || sys.dbgwcr1 &&& 1 != 0))

end Leanbox
