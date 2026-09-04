# 03 — VirtualBox NEM/win and VMware ULM on WHP: prior art for time / interrupt / state-sync

Sources (all read from local copies pulled with curl into `scratchpad/research/src/`, because the
virtualbox.org SVN export now 308-redirects to GitHub and WebFetch truncates 3000-line files):
`mirror/vbox@master` — `VMMR3/NEMR3Native-win.cpp` (3019 l), `VMMAll/NEMAllNativeTemplate-win.cpp.h` (3038 l),
`include/NEMInternal.h`, `VMMR3/NEMR3.cpp`, `VMMR3/EMR3Nem.cpp`, `VMMAll/EMAll.cpp`, `VMMR3/TM.cpp`,
`VMMAll/TMAll.cpp`, `VMMAll/TMAllVirtual.cpp`, `VMMAll/TMAllCpu.cpp`, `VMMR3/VMEmt.cpp`, `include/VBox/vmm/vm.h`,
`cpumctx-x86-amd64.h`, `iem-x86-amd64.h`, `Devices/Graphics/DevVGA.cpp`, `VMMR3/PGM.cpp`, `VMMR3/PGMPhys.cpp`;
pre-removal ring-0 tree at `mirror/vbox@e6f9972` (= svn r93115, parent of r93351 "Kicked out most of the ring-0
code because bugref:10118 + bugref:10162 means we won't use it again", 2022-01-20). Line numbers below refer to
those files. Web: VMware/Broadcom docs, QEMU hyperv docs, Linux `virt/hyperv/clocks`, TLFS `timers.md`,
Bruce Dawson, VirtualBox changelog/forum.

## Part 1 — VirtualBox NEM, Windows backend (WHP)

### Q1. The EMT loop: `emR3NemExecute` → `NEMR3RunGC` → `nemR3NativeRunGC` → `nemHCWinRunGC`

Outer loop `EMR3Nem.cpp:369-521` (`emR3NemExecute`): `NEMR3CanExecuteGuest` (else `VINF_EM_RESCHEDULE_REM`),
high-priority pre FFs (`emR3NemForcedActions` = only PGM handy pages / no-memory), `NEMR3RunGC`, post FFs,
`emR3NemHandleRC`, then `TMTimerPollVoid(pVM, pVCpu)` and `emR3ForcedActions` if any `VM_FF_ALL_MASK` /
`VMCPU_FF_ALL_MASK` bit is set (that is where expired timers are run: `VMCPU_FF_TIMER` → `TMR3TimerQueuesDo`).
On leaving: `if (pVCpu->cpum.GstCtx.fExtrn) NEMImportStateOnDemand(pVCpu, pVCpu->cpum.GstCtx.fExtrn)`.

Inner loop `NEMAllNativeTemplate-win.cpp.h:2505-2718` (`nemHCWinRunGC`), verbatim skeleton:
```c
if (VMCPU_CMPXCHG_STATE(pVCpu, VMCPUSTATE_STARTED_EXEC_NEM, VMCPUSTATE_STARTED)) { /* likely */ }
else { VMCPU_CMPXCHG_STATE(pVCpu, VMCPUSTATE_STARTED_EXEC_NEM, VMCPUSTATE_STARTED_EXEC_NEM_CANCELED);
       LogFlow(("NEM/%u: returning immediately because canceled\n", ...)); return VINF_SUCCESS; }
for (unsigned iLoop = 0;; iLoop++) {
    pVCpu->nem.s.fDesiredInterruptWindows = 0;
    if (VMCPU_FF_IS_ANY_SET(pVCpu, VMCPU_FF_INTERRUPT_APIC | VMCPU_FF_UPDATE_APIC | VMCPU_FF_INTERRUPT_PIC
                                 | VMCPU_FF_INTERRUPT_NMI  | VMCPU_FF_INTERRUPT_SMI))
        rcStrict = nemHCWinHandleInterruptFF(pVM, pVCpu, &pVCpu->nem.s.fDesiredInterruptWindows); /* break on !=VINF_SUCCESS */
#ifndef NEM_WIN_WITH_A20   /* Do not execute in hyper-V if the A20 isn't enabled. */
    if (!PGMPhysIsA20Enabled(pVCpu)) { rcStrict = VINF_EM_RESCHEDULE_REM; break; }
#endif
    /* Ensure that hyper-V has the whole state. (We always update the interrupt windows settings when active
       as hyper-V seems to forget about it after an exit.) */
    if ((fExtrn & (CPUMCTX_EXTRN_ALL | CPUMCTX_EXTRN_NEM_WIN_MASK)) != (CPUMCTX_EXTRN_ALL | CPUMCTX_EXTRN_NEM_WIN_MASK)
        || fDesiredInterruptWindows || fCurrentInterruptWindows != fDesiredInterruptWindows)
        nemHCWinCopyStateToHyperV(pVM, pVCpu);
    /* Poll timers and run for a bit.  With the VID approach (ring-0 or ring-3) we can specify a timeout here,
       so we take the time of the next timer event and uses that as a deadline. ... */
    uint64_t const nsNextTimerEvt = TMTimerPollGIP(pVM, pVCpu, &offDeltaIgnored); NOREF(nsNextTimerEvt);
    if (   !VM_FF_IS_ANY_SET(pVM, VM_FF_EMT_RENDEZVOUS | VM_FF_TM_VIRTUAL_SYNC)
        && !VMCPU_FF_IS_ANY_SET(pVCpu, VMCPU_FF_HM_TO_R3_MASK)) {
        if (VMCPU_CMPXCHG_STATE(pVCpu, VMCPUSTATE_STARTED_EXEC_NEM_WAIT, VMCPUSTATE_STARTED_EXEC_NEM)) {
            TMNotifyStartOfExecution(pVM, pVCpu);
            HRESULT hrc = WHvRunVirtualProcessor(pVM->nem.s.hPartition, pVCpu->idCpu, &ExitReason, sizeof(ExitReason));
            VMCPU_CMPXCHG_STATE(pVCpu, VMCPUSTATE_STARTED_EXEC_NEM, VMCPUSTATE_STARTED_EXEC_NEM_WAIT);
            TMNotifyEndOfExecution(pVM, pVCpu, ASMReadTSC());
            rcStrict = nemR3WinHandleExit(pVM, pVCpu, &ExitReason);            /* break on != VINF_SUCCESS (StatBreakOnStatus) */
            if (   !VM_FF_IS_ANY_SET(pVM,   !fSingleStepping ? VM_FF_HP_R0_PRE_HM_MASK    : VM_FF_HP_R0_PRE_HM_STEP_MASK)
                && !VMCPU_FF_IS_ANY_SET(pVCpu, !fSingleStepping ? VMCPU_FF_HP_R0_PRE_HM_MASK : VMCPU_FF_HP_R0_PRE_HM_STEP_MASK))
                continue;
            /** @todo Try handle pending flags, not just return to EM loops. ... */   STAT BreakOnFFPost
        } else { STAT BreakOnCancel; }         /* "breaking: canceled %d (pre exec)" */
    } else { STAT BreakOnFFPre; }
    break;
}
if (!VMCPU_CMPXCHG_STATE(pVCpu, VMCPUSTATE_STARTED, VMCPUSTATE_STARTED_EXEC_NEM))
    VMCPU_CMPXCHG_STATE(pVCpu, VMCPUSTATE_STARTED, VMCPUSTATE_STARTED_EXEC_NEM_CANCELED);
/* import-on-return, see Q2 */
```
Facts extracted:
- **No deadline bounds the run.** `TMTimerPollGIP`'s returned deadline is `NOREF`'d; the comment says a timeout was
  only possible with the (now removed) direct VID `VidMessageSlotHandleAndGetNext` path. `WHvRunVirtualProcessor`
  runs until an exit or a cancel. Already-expired timers are caught by the poll (it sets `VMCPU_FF_TIMER`, which is
  in `VMCPU_FF_HM_TO_R3_MASK = VMCPU_FF_TO_R3 | VMCPU_FF_TIMER | VMCPU_FF_PDM_CRITSECT | VMCPU_FF_IEM | VMCPU_FF_IOM`,
  vm.h:706) → pre-exec break to EM. After an exit the loop `continue`s unless `VM_FF_HP_R0_PRE_HM_MASK`
  (`VM_FF_HM_TO_R3_MASK | VM_FF_REQUEST | VM_FF_PGM_POOL_FLUSH_PENDING | VM_FF_PDM_DMA`) or
  `VMCPU_FF_HP_R0_PRE_HM_MASK` (`HM_TO_R3 | PGM_SYNC_CR3(_NON_GLOBAL) | REQUEST | VMX_*`) is set — interrupt FFs are
  NOT in these masks, so a pending interrupt does not leave the inner loop; it is injected at the top of the next
  iteration. Timers/DMA/requests do leave to EM.
- **Who kicks a vCPU that is inside WHvRunVirtualProcessor.** `nemR3NativeNotifyFF` (NEMR3Native-win.cpp:1690):
  `WHvCancelRunVirtualProcessor(pVM->nem.s.hPartition, pVCpu->idCpu, 0)`. It is reached from
  `VMR3NotifyCpuFFU` → halt-method notifier: `vmR3HaltGlobal1NotifyCpuFF` (VMEmt.cpp:889-927) forwards only when
  `enmState == VMCPUSTATE_STARTED_EXEC_NEM || VMCPUSTATE_STARTED_EXEC_NEM_WAIT` **and**
  `((fFlags & VMNOTIFYFF_FLAGS_POKE) || !(fFlags & VMNOTIFYFF_FLAGS_DONE_REM))`; `vmR3DefaultNotifyCpuFF` (1045-1063)
  forwards unconditionally for those two states. Producers: TM's watchdog `tmR3TimerCallback` (TM.cpp:2327-2356) sets
  `VMCPU_FF_TIMER` and calls `VMR3NotifyCpuFFU(..., VMNOTIFYFF_FLAGS_DONE_REM | VMNOTIFYFF_FLAGS_POKE)`; the watchdog
  period is `/TM/TimerMillies`: `u32Millies = VM_IS_HM_ENABLED(pVM) ? 1000 : 10;` (TM.cpp:677) → **10 ms under NEM**.
  `tmScheduleNotify` (TMAll.cpp:356-371) and `tmVirtualSyncGetHandleCatchUpLocked` (TMAllVirtual.cpp:491-501) notify
  with `DONE_REM` only (no POKE) → under the default Global1 halt method they do NOT cancel a running vCPU. So the
  timer-latency floor for a compute-bound guest under NEM/win is the 10 ms watchdog.
- **Cancel race.** In the 7.x ring-3 code nothing sets `VMCPUSTATE_STARTED_EXEC_NEM_CANCELED` any more (it was the own-run-API
  path); the two `CMPXCHG` fallbacks are vestigial. A cancel that lands before the run makes the next
  `WHvRunVirtualProcessor` return `WHvRunVpExitReasonCanceled`, handled as `VINF_SUCCESS` (dispatcher :2337-2339), then the
  post-exit FF check decides. Doc comment (NEMR3Native-win.cpp:2107-2114): "Other threads can interrupt the execution by
  using WHvCancelVirtualProcessor, which since or about build 17757 uses VidMessageSlotHandleAndGetNext to do the work ...
  While there is certainly a race between cancelation and the CPU causing a natural VMEXIT, it is not known whether this
  still causes extra work on subsequent WHvRunVirtualProcessor calls (it did in and earlier than 17134)." and :2194-2200
  "WHvCancelVirtualProcessor seems to cause a lot more spurious WHvRunVirtualProcessor returns ... the subsequent call to
  WHvRunVirtualProcessor would return immediately."
- **HLT.** Dispatcher returns `VINF_EM_HALT` (no handler, :2330-2335); EM calls `VMR3WaitHalted` →
  `vmR3HaltGlobal1Halt` (VMEmt.cpp:747-837): `TMR3TimerQueuesDo`, `u64GipTime = TMTimerPollGIP(pVM, pVCpu, &u64Delta)`,
  then if `u64Delta >= cNsSpinBlockThresholdCfg` (default 50 µs / res/4 / 2 µs by host timer resolution) a ring-0 timed
  block `VMMR0_DO_GVMM_SCHED_HALT, u64GipTime` woken by `GVMM_SCHED_WAKE_UP` from the notifier; else spin, polling GVMM
  every 0x2000 loops. Overslept/insomnia are counted (`StatHaltBlockOverslept` >50 µs).

### Q2. Lazy state exchange (`CPUMCTX::fExtrn`)

Bit semantics (cpumctx-x86-amd64.h): a set bit means "the value is kept externally (in Hyper-V)".
`CPUMCTX_EXTRN_KEEPER_NEM = 0x2`, `CPUMCTX_EXTRN_ALL = 0x00000ffffffffffc`, `CPUMCTX_EXTRN_INHIBIT_INT = 1<<42`,
`INHIBIT_NMI = 1<<43`, `APIC_TPR = 1<<27`, keeper-private `CPUMCTX_EXTRN_NEM_WIN_EVENT_INJECT = CPUMCTX_EXTRN_NEM_WIN_MASK = 1<<48`
("NEM/Win: Event injection (known was interruption) pending state").
Doc (NEMR3Native-win.cpp:2639-2645): "Since the CPU state needs to live in Hyper-V when executing, we probably should not
transfer more than necessary when handling VMEXITs. To help us manage this CPUMCTX got a new field CPUMCTX::fExtrn".

**Export** `nemHCWinCopyStateToHyperV` (template :110-428): `fWhat = ~fExtrn & (CPUMCTX_EXTRN_ALL | CPUMCTX_EXTRN_NEM_WIN_MASK)`;
early return if `!fWhat && fCurrentInterruptWindows == fDesiredInterruptWindows`. Only groups whose bit is *clear* (i.e. were
imported, hence possibly dirty) are added to one `WHvSetVirtualProcessorRegisters` call (arrays of 128). Rules inside:
`// WHvX64RegisterTsc - don't touch`; `CPUMCTX_EXTRN_APIC_TPR → WHvX64RegisterCr8 = CPUMGetGuestCR8`; `OTHER_MSRS` includes
`WHvX64RegisterApicBase = APICGetBaseMsrNoCheck` + PAT + MTRRs; `NEM_WIN_EVENT_INJECT → ADD_REG64(WHvRegisterPendingInterruption, 0)`
("event injection (clear it)"); interrupt state: only if both INHIBIT bits are local → `WHvRegisterInterruptState{InterruptShadow, NmiMasked}`,
else if only INHIBIT_INT is local and (last shadow || shadow now) → write shadow, "/** @todo Retrieve NMI state, currently assuming
it's zero. (yes this may happen on I/O) */"; `/* Interrupt windows. Always set if active as Hyper-V seems to be forgetful. */`
→ `WHvX64RegisterDeliverabilityNotifications = fDesiredIntWin` whenever non-zero or changed. On success:
`fExtrn |= CPUMCTX_EXTRN_ALL | CPUMCTX_EXTRN_NEM_WIN_MASK | CPUMCTX_EXTRN_KEEPER_NEM` (everything is external again).

**Import** `nemHCWinCopyStateFromHyperV(pVM, pVCpu, fWhat)` (:431-1044): `fWhat &= fExtrn` (never re-fetch what is local),
one `WHvGetVirtualProcessorRegisters`; CR0/CR4 change → `PGMChangeMode`, CR3 change → `PGMUpdateCR3`; APIC_BASE change →
`APICSetBaseMsr`; TR forced BUSY ("AMD-V likes loading TR with in AVAIL state, whereas intel insists on BUSY"); interrupt state
→ `CPUMUpdateInterruptShadowEx` / `CPUMUpdateInterruptInhibitingByNmi`; then `fExtrn &= ~fWhat`, and if no ALL/NEM bit
remains, `fExtrn = 0`. `nemHCWinImportStateIfNeededStrict` (:1430) skips the call when `!(fExtrn & fWhat)`.
IEM pulls missing state itself through `NEMImportStateOnDemand` (:1054, `StatImportOnDemand`).

**Free state from the exit header** `nemR3WinCopyStateFromX64Header` (:1451-1465): CS, RIP, RFLAGS,
`fLastInterruptShadow = CPUMUpdateInterruptShadowEx(..., ExecutionState.InterruptShadow, Rip)`, `APICSetTpr(pVCpu, Cr8 << 4)`;
clears `RIP|RFLAGS|CS|INHIBIT_INT|APIC_TPR` — zero API calls. Every handler starts with this.

**Per-exit import masks** (all in template file):
| exit | import | executor |
|---|---|---|
| MemoryAccess (:1477) | header + `NEM_WIN_CPUMCTX_EXTRN_MASK_FOR_IEM \| CPUMCTX_EXTRN_DS \| CPUMCTX_EXTRN_ES` | `IEMExecOneWithPrefetchedByPC(rip, InstructionBytes, InstructionByteCount)` else `IEMExecOne` |
| IoPort, non-string (:1599) | header only; RAX from `IoPortAccess.Rax`, write back `rax`, clear `EXTRN_RAX` | none: `IOMIOPortRead/Write` + `nemR3WinAdvanceGuestRipAndClearRF` |
| IoPort, string (:1643) | header + RAX/RCX/RDI/RSI/DS/ES from exit ctx + `NEM_WIN_CPUMCTX_EXTRN_MASK_FOR_IEM` | `IEMExecOne` |
| InterruptWindow (:1736) | header only | nothing; `/** @todo call nemHCWinHandleInterruptFF */` (loop top does it) |
| Cpuid (:1772) | header + `IEM_CPUMCTX_EXTRN_EXEC_DECODED_NO_MEM_MASK \| CPUMCTX_EXTRN_CR3`; RAX..RBX from ctx | `IEMExecDecodedCpuid(InstructionLength)` |
| MsrAccess (:1843) | header + `(pExitRec ? IEM_CPUMCTX_EXTRN_MUST_MASK : 0) \| CPUMCTX_EXTRN_ALL_MSRS \| CR0 \| CR3 \| CR4` | `CPUMSetGuestMsr` / `CPUMQueryGuestMsr` directly; CPL!=0 → `IEMInjectTrap(#GP)` |
| Exception (:2132) | header + `NEM_WIN_CPUMCTX_EXTRN_MASK_FOR_IEM` (+DR0-3/6/7 for #DB) | #UD VMCALL/VMMCALL → `IEMExecOneWithPrefetchedByPC`; else `IEMInjectTrap` |
| UnrecoverableException (:2264) | header + `NEM_WIN_CPUMCTX_EXTRN_MASK_FOR_IEM \| CPUMCTX_EXTRN_ALL` | `IEMExecOne` ("Let IEM decide whether this is really it") |
| Halt / Canceled | nothing | `VINF_EM_HALT` / `VINF_SUCCESS` |
Masks: `NEM_WIN_CPUMCTX_EXTRN_MASK_FOR_IEM = IEM_CPUMCTX_EXTRN_MUST_MASK | INHIBIT_INT | INHIBIT_NMI`;
`IEM_CPUMCTX_EXTRN_MUST_MASK = GPRS | RIP | RFLAGS | SS | CS | CR0 | CR3 | CR4 | APIC_TPR | EFER | DR7` (iem-x86-amd64.h:100);
`_XCPT_MASK = MUST | CR2 | SREG_MASK | TABLE_MASK`; `_EXEC_DECODED_NO_MEM_MASK = RIP|RFLAGS|SS|CS|CR0|EFER|DR7|CR4`.
**Event-inject rule** (:1483-1487, :1578-1582): "Whatever we do, we must clear pending event injection upon resume." — if
`VpContext.ExecutionState.InterruptionPending`, clear the `EVENT_INJECT` extrn bit so the next export writes
`WHvRegisterPendingInterruption = 0` (IEM re-raises the fault itself when it emulates the instruction).
**Import-on-return** (:2681-2713): "Try anticipate what we might need": `fImport = IEM_CPUMCTX_EXTRN_MUST_MASK | INHIBIT_INT | INHIBIT_NMI`;
EM status or failure → `CPUMCTX_EXTRN_ALL`; pending interrupt FFs → add `IEM_CPUMCTX_EXTRN_XCPT_MASK`; skipped entirely if nothing
needed (`StatImportOnReturnSkipped`).

### Q3. Interrupt delivery — VBox's own PDM APIC, software injection, hardware-armed windows

**APIC is VBox's.** No `LocalApicEmulationMode`, `WHvRequestInterrupt`, or `WHvX64RegisterDeliverabilityNotifications`-based
in-hypervisor APIC anywhere (grep over current and r93115 trees: zero hits for `LocalApicEmulationMode`/`WHvRequestInterrupt`).
The loop calls `APICUpdatePendingInterrupts`, `PDMGetInterrupt`, `APICGetTpr`, `APICSetTpr`, `APICSetBaseMsr`. X2APIC is forced
off at init: `nemR3WinDisableX2Apic` (NEMR3Native-win.cpp:1204) rewrites CFGM "Mode" → `PDMAPICMODE_APIC`: "X2APIC is not
supported by the WinHvPlatform API!"; reason in the doc block (:2178-2191): guest writes to IA32_APIC_BASE EN/base bits are
ignored, "Attempts by the guest to set the EXTD bit (X2APIC) result in #GP(0), while the VMM ends up with with
ERROR_HV_INVALID_PARAMETER. Seems there is no way to support X2APIC." Also `nemR3DisableCpuIsaExt(pVM, "MONITOR")`
"/* MONITOR is not supported by Hyper-V (MWAIT is sometimes). */" (:1321).

**Injection = IEM software delivery, not `WHvRegisterPendingInterruption`.** `nemHCWinHandleInterruptFF` (template :2384-2495):
```c
if (VMCPU_FF_TEST_AND_CLEAR(pVCpu, VMCPU_FF_UPDATE_APIC)) { APICUpdatePendingInterrupts(pVCpu); if (no INT FF) return VINF_SUCCESS; }
AssertReturn(!VMCPU_FF_IS_SET(pVCpu, VMCPU_FF_INTERRUPT_SMI), VERR_NEM_IPE_0);           /* We don't currently implement SMIs. */
fNeedExtrn = CPUMCTX_EXTRN_INHIBIT_INT | CPUMCTX_EXTRN_RIP | CPUMCTX_EXTRN_RFLAGS | (fPendingNmi ? CPUMCTX_EXTRN_INHIBIT_NMI : 0);
if (fExtrn & fNeedExtrn) nemHCWinImportStateIfNeededStrict(pVCpu, NEM_WIN_CPUMCTX_EXTRN_MASK_FOR_IEM_XCPT, "IntFF");
/* NMI? Try deliver it first. */
if (fPendingNmi) { if (!CPUMIsInInterruptShadow(...) && !CPUMAreInterruptsInhibitedByNmi(...)) {
        import XCPT mask; VMCPU_FF_CLEAR(NMI); rcStrict = IEMInjectTrap(pVCpu, X86_XCPT_NMI, TRPM_HARDWARE_INT, 0, 0, 0); return; }
    *pfInterruptWindows |= NEM_WIN_INTW_F_NMI; }
/* APIC or PIC interrupt? */
if (VMCPU_FF_IS_ANY_SET(pVCpu, VMCPU_FF_INTERRUPT_APIC | VMCPU_FF_INTERRUPT_PIC)) {
    /** @todo check NMI inhibiting here too! */
    if (!CPUMIsInInterruptShadow(...) && pVCpu->cpum.GstCtx.rflags.Bits.u1IF) {
        AssertCompile(NEM_WIN_CPUMCTX_EXTRN_MASK_FOR_IEM_XCPT & CPUMCTX_EXTRN_APIC_TPR);   import XCPT mask;
        rc = PDMGetInterrupt(pVCpu, &bInterrupt);
        if (RT_SUCCESS(rc))            rcStrict = IEMInjectTrap(pVCpu, bInterrupt, TRPM_HARDWARE_INT, 0, 0, 0);
        else if (rc == VERR_APIC_INTR_MASKED_BY_TPR) *pfInterruptWindows |= ((bInterrupt >> 4) << NEM_WIN_INTW_F_PRIO_SHIFT) | NEM_WIN_INTW_F_REGULAR;
        return rcStrict; }
    if (APIC && !PIC) { /* If only an APIC interrupt is pending, we need to know its priority. Otherwise we'll likely get
                           pointless deliverability notifications with IF=1 but TPR still too high. */
        APICGetTpr(pVCpu, &bTpr, &fPendingIntr, &bPendingIntr);
        *pfInterruptWindows |= ((bPendingIntr >> 4) << NEM_WIN_INTW_F_PRIO_SHIFT) | NEM_WIN_INTW_F_REGULAR; }
    else *pfInterruptWindows |= NEM_WIN_INTW_F_REGULAR; }
```
`IEMInjectTrap` ("Injects a trap, fault, abort, software interrupt or external interrupt", IEMAll.cpp:10410) performs the
delivery in VBox's memory view (stack push, IDT vectoring, CS:RIP load); the resulting RIP/RSP/CS/RFLAGS/SS are then exported
because their `fExtrn` bits are clear. Consequence: the guest's interrupt frame is written by ring-3 code, and the
IDT/GDT/TSS are read through PGM — the import mask is `IEM_CPUMCTX_EXTRN_XCPT_MASK` (adds CR2, all SREGs, GDTR/IDTR/LDTR/TR).
`WHvRegisterPendingInterruption` is only ever written as 0 (clear). `/// @todo WHvRegisterPendingEvent` (template :403, :1013).

**Interrupt window = `WHvX64RegisterDeliverabilityNotifications`.** `NEM_WIN_INTW_F_NMI=0x01, _REGULAR=0x02, _PRIO_MASK=0x3c,
_PRIO_SHIFT=2` map 1:1 onto `DeliverabilityNotifications.{NmiNotification, InterruptNotification, InterruptPriority}`
(asserted at template :398-400). `fDesiredInterruptWindows` is recomputed from scratch every loop iteration and re-sent on every
entry while non-zero ("Hyper-V seems to be forgetful"). The `X64InterruptWindow` exit (`DeliverableType` PendingInterrupt or
PendingNmi) only copies the header; because interrupt FFs are outside `VMCPU_FF_HP_R0_PRE_HM_MASK` the loop `continue`s and the
top of the next iteration injects. Interrupt shadow: taken from `VpContext.ExecutionState.InterruptShadow` on every exit
(header copy) and from `WHvRegisterInterruptState` on full import; exported back as shadow/NmiMasked (Q2). So the "cannot inject
into an interrupt shadow" problem is solved by VBox tracking the shadow itself and requesting a window instead.

**TPR path.** `APICSetTpr(pVCpu, pExitCtx->Cr8 << 4)` on every exit header (template :1462); export `WHvX64RegisterCr8 =
CPUMGetGuestCR8` when `CPUMCTX_EXTRN_APIC_TPR` is local (:236-237). No TPR-threshold exit is used; a TPR-masked pending vector
becomes a priority-qualified window request as above.

**Cross-thread IRQ → running vCPU.** Devices set `VMCPU_FF_INTERRUPT_APIC/PIC`; the EMT sees it either at the next natural exit
(post-exit check → `continue` → top → inject) or when some notifier cancels the run (Q1). There is no dedicated "IRQ raised →
cancel" path in NEM/win; latency to a compute-bound guest is bounded by the next exit or the 10 ms TM watchdog.

### Q4. Time: Hyper-V owns the TSC; VBox's clocks are host-wall-clock based

- **TSC mode is forced.** `NEMR3NeedSpecialTscMode` returns true whenever NEM is enabled (NEMR3.cpp:363-372); TM.cpp:382
  `enmTSCMode = NEMR3NeedSpecialTscMode(pVM) ? TMTSCMODE_NATIVE_API : ...`; explicit `/TM/TSCMode` is overridden
  ("TM: NEM overrides the /TM/TSCMode=%s settings."), `TSCModeSwitchAllowed` forced false (:418-421), `/TM/TSCTicksPerSecond`
  overridden to `cTSCTicksPerSecondHost * u8TSCMultiplier` (:466-472) — i.e. the guest TSC frequency is the host's, because
  Hyper-V runs RDTSC natively (benchmark: ~100 M RDTSC/s under WinHv vs 93 M native VBox, doc block :2671-2676).
  `/TM/TSCTiedToExecution is not supported in NEM mode!` (:485-486).
- **Reading it** `tmCpuTickGetInternal` (TMAllCpu.cpp:504-510): `case TMTSCMODE_NATIVE_API: NEMHCQueryCpuTick(pVCpu, &u64, NULL)`
  → `WHvGetVirtualProcessorRegisters(hPartition, idCpu, {WHvX64RegisterTsc, WHvX64RegisterTscAux}, 2, ...)` (template :1069-1090,
  `StatQueryCpuTick`), then `u64 -= offTSCRawSrc`, monotonic clamp `u64TSCLastSeen += 64` on underflow. Export never writes TSC
  (`// WHvX64RegisterTsc - don't touch`).
- **Pause/resume/restore** `NEMHCResumeCpuTickOnAll(pVM, pVCpu, uPausedTscValue)` (template :1103-1140), "called by TM when the VM
  is started, restored, resumed or similar": `RTThreadYield()` if SMP, write `WHvX64RegisterTsc = uPausedTscValue` on CPU 0, then for
  each other CPU `Value.Reg64 = uPausedTscValue + (ASMReadTSC() - uFirstTsc)` — "keeping finger crossed that we don't introduce too
  much drift here." Doc complaint (:2164-2168): "We need a way to directly modify the TSC offset (or bias if you like). The current
  approach of setting the WHvX64RegisterTsc register one by one on each virtual CPU in sequence will introduce random
  inaccuracies, especially if the thread doing the job is reschduled at a bad time."
- **Not used:** `WHvPartitionPropertyCodeReferenceTime`, `WHvSuspendPartitionTime`/`WHvResumePartitionTime` — zero hits in current
  and r93115 trees. Partition properties set are exactly: `ProcessorCount`, `ExtendedVmExits{X64CpuidExit, X64MsrExit,
  ExceptionExit}` (:1160-1168), `ProcessorClFlushSize`, `ExceptionExitBitmap = #DB | #BP | #UD (+#GP for the mesa hack)`,
  `ProcessorFeatures` (:1448-1486). Guest paused ⇒ Hyper-V's TSC keeps counting; VBox hides it by rewriting TSC on resume.
- **VBox clocks** (TM.cpp:28-127 header): `TMCLOCK_VIRTUAL` = "an offset to a monotonic, high resolution, wall clock ... paused when
  the VM isn't in the running state" (it keeps running during exits/ring-3 handling); `TMCLOCK_VIRTUAL_SYNC` = "tied to the
  virtual clock except that it will take into account timer delivery lag caused by host scheduling. It will normally never
  advance beyond the head timer, and when lagging too far behind it will gradually speed up to catch up ... All devices
  implementing time sources accessible to and used by the guest is using this clock". Catch-up math
  (`tmVirtualSyncGetHandleCatchUpLocked`, TMAllVirtual.cpp:410-511): `off -= u64Delta * u32VirtualSyncCatchUpPercentage / 100`
  until `off <= offVirtualSyncGivenUp`; when `u64 >= u64Expire` the clock **stops** at the head timer (`fVirtualSyncTicking = false`,
  `VM_FF_TM_VIRTUAL_SYNC`, `VMCPU_FF_TIMER`, `VMR3NotifyCpuFFU(DONE_REM)`). Defaults: `ScheduleSlack 100 µs`,
  `CatchUpStopThreshold 0.5 ms`, `CatchUpGiveUpThreshold 60 s` (TM.cpp:503-525). Under NEM the guest TSC (Hyper-V) and
  `TMCLOCK_VIRTUAL` both advance in real time while `VIRTUAL_SYNC` (device timers) may lag — the same three-clock split Bochs lacks.
- **EMT time accounting** `TMNotifyStartOfExecution/EndOfExecution` (TMAll.cpp:171-276) bracket `WHvRunVirtualProcessor` and
  only account `cNsExecuting` (host TSC delta → ns); they touch the guest TSC only in the unsupported `fTSCTiedToExecution` mode.

### Q5. Memory: one RWX RAM range, lazy page remaps, WHP dirty bitmap for VRAM, no A20 under WHP

- **PGM "NEM mode"** (`PGMR3EnableNemMode`, PGM.cpp:736-744: "simplified memory managment mode"; NEMR3Native-win.cpp:1322) —
  RAM ranges are allocated as one contiguous host allocation (`SUPR3PageAlloc(..., fUseLargePages ? SUP_PAGE_ALLOC_F_LARGE_PAGES : 0,
  &pNew->pbR3)`, PGMPhys.cpp:1903-1906) and mapped once: `NEMR3NotifyPhysRamRegister` → `WHvMapGpaRange(hPartition, pvR3, GCPhys,
  cb, Read|Write|Execute)`, `*pu2State = NEM_WIN_PAGE_STATE_WRITABLE` (:1734-1760). Changelog 6.1.32: "Changed the guest RAM
  management when using Hyper-V to be more compatible with HVCI" — this replaced the per-page hypercall scheme
  (`NEM_WIN_USE_HYPERCALLS_FOR_PAGES`, r93115 NEMInternal.h:56, `#error "VBOX_WITH_PGM_NEM_MODE cannot be used together with
  NEM_WIN_USE_HYPERCALLS_FOR_PAGES"`).
- **ROM**: `NEMR3NotifyPhysRomRegisterLate` maps `Read|Execute` (:1955-1980); the early hook does nothing ("We'll protection change
  notifications for each page and if not we'll map them lazily").
- **MMIO2 / VRAM**: `NEMR3NotifyPhysMmioExMapEarly` (:1770-1828) unmaps replaced RAM, maps MMIO2 `Read|Write|Execute` and adds
  `WHvMapGpaRangeFlagTrackDirtyPages` when the device asked for `NEM_NOTIFY_PHYS_MMIO_EX_F_TRACK_DIRTY_PAGES` and
  `g_pfnWHvQueryGpaRangeDirtyBitmap != NULL` (i.e. build ≥17763). DevVGA registers VRAM with
  `PGMPHYS_MMIO2_FLAGS_TRACK_DIRTY_PAGES` (DevVGA.cpp:6725) and per refresh calls `PDMDevHlpMmio2QueryAndResetDirtyBitmap`
  (`vgaR3UpdateDirtyBitsAndResetMonitoring`, :401-440) → `NEMR3PhysMmio2QueryAndResetDirtyBitmap` → `WHvQueryGpaRangeDirtyBitmap`
  (:1896-1912). Pure MMIO regions stay **unmapped** → `WHvRunVpExitReasonMemoryAccess`. Doc (:2227-2229): the 17757 dirty-logging API
  "could help tracking dirty VGA pages, while being useless for shadow ROM and devices trying catch the guest updating descriptors".
- **Protection / backing changes** (`NEMHCNotifyPhysPageProtChanged`, `NEMHCNotifyPhysPageChanged`, `nemHCNativeNotifyPhysPageAllocated`,
  template :2956-3021) all reduce to `nemHCJustUnmapPageFromHyperV` — unmap the single page, remember `UNMAPPED` in PGM's 2-bit
  `u2NemState`. The page is re-mapped lazily on the next `MemoryAccess` exit by `nemHCWinHandleMemoryAccessPageCheckerCallback`
  (:1291-1417) → `nemHCNativeSetPhysPage` (:2824-2926): "Looks like we need to unmap a page before we can change the backing or even
  modify the protection. This is going to be *REALLY* efficient." Doc (:2203-2243): "There is no API for modifying protection of a
  page within a GPA range ... the only way ... is to first unmap the range and then remap it"; "Observed problems doing
  WHvUnmapGpaRange immediately followed by WHvMapGpaRange ... we've ended up looping forever with the same write to readonly
  memory VMEXIT ... Workaround: Insert a WHvRunVirtualProcessor call and make sure to get a GPA unmapped exit between the two
  calls." (also :1324-1331 "/** @todo Someone at microsoft please explain: ... DSL 4.4.1 ... we no longer pre-map anything, just
  unmap stuff and do it lazily"). Quota: "quota restrictions makes sense ... causes us to exceed our quota before we've even mapped
  a default sized (128MB) VRAM page-by-page" (:2096-2099, :2215-2219).
- **A20**: `NEM_WIN_WITH_A20` is not defined anywhere (grep: headers, .cpp, Makefile.kmk, Config.kmk). Live code:
  `NEMR3CanExecuteGuest` = `PGMPhysIsA20Enabled(pVCpu)` — "Only execute when the A20 gate is enabled because this lovely Hyper-V
  blackbox does not seem to have any way to enable or disable A20" (:1669-1675); `nemHCWinRunGC` breaks with
  `VINF_EM_RESCHEDULE_REM` → **the whole guest runs in IEM while A20 is off**. Doc (:2246-2254): "Implementing A20 gate behavior is
  tedious, where as correctly emulating the A20M# pin (present on 486 and later) is near impossible for SMP setups ... Workaround #1
  (obsolete): Only do A20 on CPU 0, restricting the emulation to HMA. We unmap all pages related to HMA (0x100000..0x10ffff) when the
  A20 state changes, lazily syncing the right pages back when accessed. Workaround #2 (used): Use IEM when the A20 gate is disabled."
  The ifdef'd #1 code (`NEMR3NotifySetA20` unmapping 16 pages, `fA20Fixed` after INIT IPI on CPU>0) is still in the tree.
- **Large pages** (bs3-memalloc-1, 48 GiB, doc :2450-2599): first-touch allocation under Hyper-V 7,794 ns/page (501 MB/s) vs native
  VBox 66 ns/page (58,713 MB/s) — "sucks hundredfold in the setting up phase"; steady-state access equal (~9 ns/page) once
  Hyper-V has large pages in place; without MEM_LARGE_PAGES the 2nd access is 308 ns/page (12.7 GB/s) vs 9 ns/page.

### Q6. Performance: what VirtualBox measured and said

- `NEMR3Init` (NEMR3.cpp:184-192): `/* The WHv* API is extremely slow at handling VM exits. The AppleHv and KVM APIs are much
  faster, thus the different mode name. :-) */` → "NEM: NEMR3Init: Snail execution mode is active!" (Windows) vs "Turtle" elsewhere.
- Doc :2127-2161: "The VMEXIT performance is dismal (build 17134). Our proof of concept implementation with a kernel runloop ...
  delivers 9-10% of the port I/O performance and only 6-7% of the MMIO performance that we have with our own hypervisor. When
  using the offical WinHvPlatform API, the numbers are %3 for port I/O and 5% for MMIO." "Windows 2000 boot screen animation
  overloads us with MMIO exits and won't even boot"; "Update: Security fixes during the summer of 2018 caused the performance to
  dropped even more."
- bootsector2-test1, 2018-06-22, build 17134, Threadripper 1950X (:2663-2750) — WinHv API / hypercalls+VID ring-0 / native AMD-V:
  CPUID 108,874 / 123,602 / 1,305,113 ins/s; 32-bit IN 104,649 / 123,513 / 1,075,831; IN-to-ring-3 105,697 / 104,471 / 213,216;
  MMIO read 57,687 / 69,136 / 690,548; MMIO read-to-ring-3 57,958 / 55,432 / 160,505; RDTSC ~99.6 M everywhere;
  **Read CR4 2,156,102 vs 369,009,009 native** ("Hyper-V doesn't let the guest read CR4 but triggers exits all the time").
  Summary: "10 to 12 times slower for exits we can handle directly in ring-0 ... 2 to 3 times slower for exits we have to go to
  ring-3"; ring-0 hypercalls gained "between 13% and 20%".
- 2018-10-02 retest, same 17134 fully patched (:2758-2801): CPUID 33,270 ins/s, IN 32,739, MMIO 20,042 — "dropping around 70%";
  "Suspects are security updates and/or microcode updates ... The issue is probably in the thread / process switching area ...
  Really wish this thread ping-pong going on in VID.SYS could be eliminated!"
- Build 17763 (:2804-2836): CPUID 54,145 ins/s; clearing bit 20 of `nt!KiSpeculationFeatures` (checked by
  `nt!KePrepareToDispatchVirtualProcessor`, called from `winhvr!WinHvpVpDispatchLoop` before hypercall 0xc2) → 130,076 (2.4×).
  So the ceiling is ~50 k exits/s per vCPU (PIO/CPUID) and ~28 k/s (MMIO) on a 3.4 GHz host, dominated by NT-side
  speculation mitigations + VID thread hand-off, not by hvax64.
- Windows 2000 boot+shutdown (:2839-2884): 32 min 12 s (WinHv API), 3 min 23 s (ring-0 hypercalls), 58.09 s native,
  **58.66 s WinHv API with exit-history optimizations**, 58.94 s hypercalls + optimizations — "The 13%-20% exit performance increase
  ... pays off a lot here ... windows 2000 doing a lot of waiting during boot".
- NetPerf NAT (:2887-3016): latency 152 µs native vs 342 µs WinHv API (225%), 318 µs with exit optimizations; throughput 64-70%
  of native receiving, 91-93% sending.
- **Ring-0 path**: existed (`NEMR0Native-win.cpp`, 3220 lines at r93115; `NEM_WIN_WITH_RING0_RUNLOOP`, `NEM_WIN_USE_HYPERCALLS_FOR_*`,
  `/NEM/UseRing0Runloop`), talked to VID.SYS via I/O controls whose numbers "shifted a little" between builds and were discovered by
  hooking `NtDeviceIoControlFile` (:2615-2636), fished the partition HANDLE out of WinHvPlatform's struct (`((HANDLE *)hPartition)[1]`,
  "Hysterical raisins", :1501-1506, still present). Changelog 6.1.4 (2020-02-19): "Windows host: Restore the ability to run VMs
  through Hyper-V, at the expense of performance"; Klaus (forum t=90853): "what 6.1.4 effectively does is disabling the (apparently
  no longer working, probably needs to be improved) optimization which avoids going always to usermode". Removed r93351/r93352
  (2022-01-20) "because bugref:10118 + bugref:10162 means we won't use it again"; 6.1.32 HVCI-compatible RAM management.
  Later changelog: 6.1.22 "Improved performance of 64-bit Windows and Solaris guests when Hyper-V is used"; 6.1.36 "Fixed regression
  in 6.1.32 leading to guest hangs when Hyper-V is used".

### Q7. MMIO / PIO emulation and exit clustering

- **One exit per MMIO/PIO instruction on the plain path.** MMIO (`nemR3WinHandleExitMemory`, template :1477-1560): first
  `PGMPhysNemPageInfoChecker` with the lazy-map callback; if the page is really RAM/MMIO2 that was merely unmapped, map it and
  **restart the instruction** (`fCanResume` → `VINF_SUCCESS`, no emulation). Otherwise `EMHistoryAddExit(MMIO_READ/WRITE, flatPC)`,
  header copy, import `MASK_FOR_IEM | DS | ES`, then `IEMExecOneWithPrefetchedByPC(pVCpu, Rip, MemoryAccess.InstructionBytes,
  InstructionByteCount)` — the exit context carries up to 16 instruction bytes, so no guest code fetch — else `IEMExecOne`.
- **PIO fast path** (:1599-1642): no register API call at all. `IOMIOPortWrite(pVM, pVCpu, PortNumber, (uint32_t)Rax & fAndMask,
  AccessSize)` / `IOMIOPortRead`; on success `rax = (Rax & ~fAndMask) | (uValue & fAndMask)`, clear `EXTRN_RAX`, header copy,
  `rip += InstructionLength; rflags.RF = 0; CPUMClearInterruptShadow`. String I/O goes to IEM because "The I/O port exit context
  information seems to be missing the address size information needed for correct string I/O emulation" (:2334-2341, :1648-1657).
- **CPUID**: `IEMExecDecodedCpuid` with RAX..RBX seeded from the exit; **MSR**: `CPUMSetGuestMsr`/`CPUMQueryGuestMsr` directly,
  `rip += 2`; CPL≠0 → `IEMInjectTrap(#GP)`. **#UD** for VMCALL/VMMCALL → IEM (GIM hypercalls); other #UD/#DB/#BP re-injected via
  `IEMInjectTrap`; **#GP** intercept only for the mesa vmwgfx backdoor (`IN EAX,DX` port 0x5658 magic 0x564d5868) — skipped with
  `rip += 1`.
- **Batching = EM exit history** (EMAll.cpp:406-502, :717-746; EM.cpp:164-203). Every EM-kind exit is hashed by
  `(flatPC, type)`; a hot record turns into `EMEXITACTION_EXEC_PROBE`, which runs `IEMExecForExits(pVCpu, fWillExit,
  cHistoryProbeMinInstructions = (32+1)*3, cHistoryExecMaxInstructions = 8192, cHistoryProbeMaxInstructionsWithoutExit = 32 for NEM,
  &ExecStats)`; if the probe sees ≥2 exits it becomes `EMEXITACTION_EXEC_WITH_MAX` with `cMaxInstructionsWithoutExit =
  cMaxExitDistance` (≤32): "Executes multiple instruction stopping only when we've gone a given number without perceived exits."
  Records that probe uselessly 512 times are demoted to `NORMAL_PROBED`. `StatHistoryExecSavedExits` counts exits avoided.
  `emR3NemExecuteIOInstruction` resumes an interrupted history run (`idxContinueExitRec`). Doc :2149-2155 predicted exactly this:
  "detect blocks with excessive MMIO and port I/O exits and emulate instructions to cover multiple exits before letting Hyper-V have
  a go ... there will only be real gains if the exitting instructions are tightly packed." Measured gain: W2K boot 32 min → 58.66 s.

## Part 2 — VMware Workstation on WHP (ULM / "Host VBS mode")

Facts with sources (no source code; all public statements are qualitative):
- VMware blog 2020-05-28 ("Workstation 15.5 Now Supports Host Hyper-V Mode"): "changing our VMM to run at user level instead of in
  privileged mode" and modifying it "to use the WHP APIs to manage the execution of a guest instead of using the underlying hardware
  directly"; minimum host "Windows 10 20H1 build 19041.264", Workstation/Player ≥15.5.5. Tech Preview 20H1 (2020-01-21):
  "Performance of virtual machine have been greatly improved" vs the VMworld 2019 demo but "still not as good as you expect it to
  be"; requires Haswell+/Bulldozer+.
- Broadcom TechDocs WS 17 "Host VBS Mode on Workstation": "uses a set of newly introduced Windows 10 features (Windows Hypervisor
  Platform) that permits the use of VT/AMD-V features, which enables Workstation Pro and Hyper-V to coexist." "Limitations of Host
  VBS Mode": "Depending on the workload, a Host VBS Mode VM can run slower when compared to a VM in traditional mode."; "x86
  virtualization features (Intel VT / AMD-V) are unavailable to a guest"; "x86 performance monitoring counters (PMCs) are
  unavailable"; RTM/HLE "not available"; "User-mode protection keys capability is not available." — exactly the set WHP's
  `WHV_PROCESSOR_FEATURES`/no-PMU exposure cannot provide, consistent with a pure-WHP ULM.
- Diagnostics: `vmware.log` prints `Monitor Mode: ULM` (WHP) vs `Monitor Mode: CPL0` (own ring-0 VMM) (woshub, smhk.net,
  Broadcom community thread "Disabling Hyper-V hypervisor on Windows 11 Pro host (to get VMware 17's CPL0 vs. ULM monitor mode)").
- **Numbers: none published.** Searched VMware/Broadcom docs, blogs, KB, community (communities.vmware.com now 301s to
  community.broadcom.com and the migrated thread is truncated), woshub, nakivo, syselement, yunolay: only "significantly slower" /
  "severely degraded" / "extra level of nesting". The only hard datapoint anywhere in this research is VirtualBox's own table above
  (WHP exits 3-12× slower than native ring-0 VMM; ~30-55 k PIO/CPUID exits/s per vCPU). VMware's 7-month public beta
  (Jan→May 2020) and the "far below expectation" feedback request suggest they hit the same wall and tuned around it.
- Which WHP features VMware uses (own vs hypervisor APIC, WHvRequestInterrupt, ReferenceTime, synthetic features): **not public**.
  The hard dependency on a specific 20H1 cumulative build (19041.264) indicates reliance on WHP additions/fixes shipped in 20H1;
  WinHvPlatform.h gates by build the in-hypervisor APIC modes, `WHvRequestInterrupt`, interrupt-controller state get/set,
  `WHvRegisterPendingEvent`, partition time suspend/resume — which of these the ULM uses cannot be established from public sources.
- Default virtual hardware for a new Windows guest (Broadcom KBs/TechDocs, GOSIG): NIC **E1000E** for Windows 8+/Windows 11
  (VMXNET3 optional, paravirtual); storage **LSI Logic SAS** for Windows 2008+/Windows 10, **NVMe 1.3c** default for Windows 11 /
  Server 2022 on hardware version 21 (WS 17.5+); graphics VMware SVGA II with `svga.vramSize` (default 268 MB in WS16; VRAM is a
  direct-mapped PCI BAR the guest driver writes into, with FIFO-command-driven update rectangles — under WHP the only way to keep
  that direct is RAM-mapping the BAR, plausibly with `WHvMapGpaRangeFlagTrackDirtyPages` for VGA-mode; inference, not sourced).
- Microsoft's companion post ("VMware Workstation and Hyper-V", techcommunity 1419928) could not be fetched (JS-rendered; archive
  blocked); its title-level claim is only that WHP made the coexistence possible.

## Part 3 — Windows guests' timer behaviour (what decides timer-exit rate)

- Default clock interrupt: "the default interval has been 15.625 ms (1,000 ms divided by 64)" (Dawson, "Windows Timer Resolution:
  The Great Rule Change"); MS docs: default system-wide timer resolution 15.6 ms; `timeBeginPeriod(1)` "activates per-millisecond
  interrupts". Pre-Windows 10 2004: "whatever still running program has specified the smallest timer interrupt duration in an
  outstanding call to timeBeginPeriod gets to set the global timer interrupt interval" → any 1 ms requester (installers, browsers,
  audio) drives the whole guest at 1 kHz. Windows 10 2004+: per-process; "processes are no longer affected by other processes
  calling timeBeginPeriod".
- Timer source, Windows 7 and earlier: "Microsoft used PIT/RTC (Programmable Interval Timer/Real Time Clock) for system clock
  interrupts in Windows 7 and before" (Blur Busters timers thread); `useplatformtick` "disables TSC tick and uses the platform
  source tick instead (RTC)". Red Hat KVM guide: "Windows Server 2008, Windows Server 2008 R2, and Windows 7 do not use the TSC as a
  time source if the hypervisor-present bit is set" → QPC falls back to HPET if present else ACPI PM timer (port I/O) — every
  `QueryPerformanceCounter` becomes an exit unless an enlightened clocksource exists. Windows 8+: "Together with the move to a
  tickless kernel [Dynamic Ticks], Microsoft changed the preferred time source for System Clock Interrupts to the LAPIC timer"
  (TSC-deadline mode where available); `bcdedit /set disabledynamictick yes` restores periodic.
- Hyper-V (Linux `virt/hyperv/clocks`): "Hyper-V provides virtualized versions of the PIT (in Hyper-V Generation 1 VMs only), local
  APIC timer, and RTC. Hyper-V does not provide a virtualized HPET in guest VMs." Reference TSC page: "the guest reads the TSC and
  then applies the scale and offset ... The resulting value advances at a constant 10 MHz frequency"; adjusted on migration.
  TLFS `timers.md`: services = "A per-partition reference time counter" (MSR 0x40000020), "Four synthetic timers per virtual
  processor" (STIMER0-3, periodic/one-shot, lazy, auto-enable, "Direct Mode - Assert and interrupt upon timer expiration" with
  ApicVector), "One virtual APIC timer per virtual processor"; counts are 100 ns units.
- QEMU hyperv docs: `hv-stimer` — "certain Windows versions revert to using HPET (or even RTC when HPET is unavailable) extensively
  when this enlightenment is not provided; this can lead to significant CPU consumption, even when virtual CPU is idle";
  `hv-time` — "Reference TSC page clocksource allows for exit-less time stamp readings"; `hv-vapic` — VP-assist page for
  exit-less EOI; `hv-relaxed` — disables watchdog timeouts; `hv-frequencies` — TSC/APIC frequency MSRs "without doing measurements".
- Consequence for a W7 install on our WHP engine without enlightenments: ≥64 clock interrupts/s per vCPU from the RTC (each is a
  device-timer deadline → EMT wake + injection + at least the RTC status-register read PIO exits), 1000/s whenever any process holds a
  1 ms resolution, plus one PM-timer PIO exit per QPC. A W10/11 guest uses the LAPIC timer (our own LAPIC → MMIO or MSR exits per
  re-arm in TSC-deadline mode, one per tick when tickless) and, if we advertised Hyper-V CPUID leaves, would switch to STIMER0 +
  reference-TSC page and stop trapping on time reads altogether.

## Rules extracted for our design

1. **Run unbounded, kick from outside.** VBox never bounds `WHvRunVirtualProcessor` by the next device deadline (Q1: deadline
   `NOREF`'d, no WHP timeout exists); expired timers are caught by a pre-entry poll and by an external `WHvCancelRunVirtualProcessor`.
   Our slice-per-deadline loop is the thing to delete: 86% of our slices never enter the partition.
2. **Deadline-armed kicker, not a 10 ms watchdog.** VBox's only steady-state kicker is the TM watchdog at 10 ms under NEM
   (`TimerMillies`), and non-POKE notifiers don't cancel; that is why 1 kHz Windows timers feel late under NEM. Arm one host
   high-resolution waitable timer at the earliest device deadline and cancel the vCPU from it; re-arm on every deadline change.
3. **Pre-entry poll + FF gate.** Before each entry: poll timers (set FF if expired), inject/arm interrupts, then enter only if no
   to-ring-3 FF is set; after an exit, `continue` unless a timer/DMA/request FF is set — interrupts alone must not leave the loop.
4. **One 64-bit "externalized" mask, import-what-you-need, export-what-you-imported.** Header registers (CS/RIP/RFLAGS/shadow/CR8) come
   free from the exit context; PIO needs zero register calls; MMIO needs `MUST_MASK|DS|ES`; only EM-level returns import everything.
5. **Interrupt shadow is the guest's, tracked by us.** Read `ExecutionState.InterruptShadow` on every exit, export
   `WHvRegisterInterruptState` when we own it; if not injectable, arm `WHvX64RegisterDeliverabilityNotifications` with
   `InterruptPriority = vector >> 4` and re-arm on every entry (Hyper-V forgets it).
6. **Own APIC is proven viable** (VBox does exactly LocalApicEmulationMode None); mirror TPR via CR8 both ways, export APIC_BASE,
   disable X2APIC and MONITOR. Cost: window exits + software (interpreter) delivery, which we already have.
7. **Clear pending event injection whenever we emulate the exiting instruction** (`InterruptionPending` at exit → write
   `WHvRegisterPendingInterruption = 0` before resume) or the fault is delivered twice.
8. **Let Hyper-V own the TSC**: never write it during normal operation, read it via `WHvX64RegisterTsc` only when a device or RDTSC
   emulation needs it, report host TSC Hz, restore per-CPU on resume with a host-TSC-delta correction (or use
   `WHvSuspendPartitionTime`/`ReferenceTime`, which VBox never adopted).
9. **Three clocks**: real-time virtual clock, device "virtual-sync" clock that stops at the head timer and catches up by a percentage
   (defaults 100 µs slack, 0.5 ms stop, 60 s give-up), guest TSC free-running — Bochs' single lockstep clock cannot express lag.
10. **Memory**: one RWX `WHvMapGpaRange` for all RAM at start (large pages for the steady state), ROM RX, VRAM RWX +
    `TrackDirtyPages` with one `WHvQueryGpaRangeDirtyBitmap` per frame, pure MMIO unmapped. Protection changes = unmap, resume, remap on
    the fault — never unmap+map back-to-back.
11. **A20 off ⇒ interpreter** (VBox's chosen workaround #2); do not try to alias HMA pages in the partition.
12. **Exit clustering**: keep a (flatPC, exit-type) history; hot sites run the interpreter for up to N instructions with ≤32 between
    exits. This — not ring-0 tricks — closed VBox's 32-min-vs-58-s gap on a VGA-hammering boot.
13. **Use the exit's instruction bytes** for MMIO/#UD/#GP emulation; only string PIO needs a real decode (address-size missing).
14. **Budget ~30-55 k PIO/CPUID exits/s and ~20-28 k MMIO exits/s per vCPU** on WHP (post-2018 mitigations); device models must be
    exit-frugal (MSI, descriptor rings, no VGA text/graphics through MMIO on hot paths, e1000e/NVMe-class devices as VMware does).
15. **Do not build a ring-0/VID bypass**: VBox's broke on Windows updates, was disabled in 6.1.4 and deleted in 2022 (HVCI).

## Traps (VirtualBox's WHP warnings, NEMR3Native-win.cpp doc block unless noted)

- Exit cost regressed ~70% between June and October 2018 on the same build from security/microcode updates; bit 20 of
  `nt!KiSpeculationFeatures` alone costs 2.4×. Measure on the host you ship on; old numbers do not transfer.
- `WHvCancelRunVirtualProcessor` races with natural exits and "seems to cause a lot more spurious WHvRunVirtualProcessor returns";
  treat `WHvRunVpExitReasonCanceled` as a no-op and let the FF check decide.
- Hyper-V "seems to forget" `WHvX64RegisterDeliverabilityNotifications` after an exit — resend on every entry while pending.
- No protection-change API; unmap immediately followed by map at the same GPA looped forever on a write-to-readonly exit — a run
  must sit between them. Page-granular GPA ranges blow the mapping quota before 128 MB of VRAM is mapped.
- Guest writes to IA32_APIC_BASE (EN, base) are silently ignored and not interceptable; setting EXTD → #GP(0); no X2APIC.
  `WHvX64RegisterMsrMtrrCap` inaccessible; `IA32_MTRR_PHYSMASK0` write → unrecoverable exception on Ryzen (17134).
- MONITOR unsupported (MWAIT "sometimes"); CPUID/MSR/exception exits must be requested via `ExtendedVmExits`/`ExceptionExitBitmap`,
  and `WHvCapabilityCodeExceptionExitBitmap` reported zero on 17134 although intercepts worked.
- Hyper-V exits on every CR4 read (~2 M/s ceiling) — irrelevant for modern kernels that cache CR4, fatal for code that doesn't.
- Wrong `InstructionLength` in unmapped-GPA exits on 17115/AMD ("PUSH CS" reported as 2 bytes); I/O exit lacks address size for
  string ops; `DebugActive` semantics unclear ("Does DebugActive this only reflect DR7?").
- Setting TSC per vCPU introduces skew; there is no TSC-offset register — hence VBox's yield + delta dance on resume.
- Interrupt-state export only knows the shadow; NMI-blocking is assumed 0 on partial exports ("yes this may happen on I/O").
- Pending interruption left in Hyper-V while you emulate the faulting instruction delivers the fault twice — clear it.
- TR type must be normalised to BUSY on import (AMD-V loads AVAIL); `PGMChangeMode`/`PGMUpdateCR3` must follow CR0/CR4/CR3 imports.
- `WHvSetVirtualProcessorRegisters` translates names through tables and may fall back to `VidSetVirtualProcessorState`; batch all
  registers into one call (VBox uses arrays of 128) and never call it when nothing is dirty.
- The partition device HANDLE is fished from `((HANDLE *)hPartition)[1]` and VID ioctl numbers shift between builds — anything
  below WinHvPlatform.dll is unstable across Windows updates.
- First-touch page allocation under Hyper-V is ~100× slower than native (7.8 µs/page); pre-touch or accept a slow first pass.
- Under NEM `TimerMillies` is 10 ms: VBox accepts ≤10 ms timer latency for a compute-bound guest; 1 kHz Windows timers will drift
  unless the kicker is deadline-driven (Rule 2).
