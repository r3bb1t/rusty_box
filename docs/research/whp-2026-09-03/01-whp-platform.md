# WHP platform contract — research report (2026-09-02)

Ground truth read in full: `WinHvPlatformDefs.h` (4321 lines) and `WinHvPlatform.h` (1183 lines) from SDK 10.0.26100.0. Docs: learn.microsoft.com WHP pages (retrieved 2026-06-11 snapshot). Reference VMMs: QEMU `target/i386/whpx/*` (master, 2026), OpenVMM `vm/virt_whp` + `vm/whp` (rustdoc source mirror). Where docs and header disagree, the header wins and the disagreement is listed at the end. "Confidence" = how directly the primary source supports the claim.

## A. Local APIC emulation

### A.1 What the hypervisor owns in XApic/X2Apic mode

Header:
```c
typedef enum WHV_X64_LOCAL_APIC_EMULATION_MODE
{
    WHvX64LocalApicEmulationModeNone,
    WHvX64LocalApicEmulationModeXApic,
    WHvX64LocalApicEmulationModeX2Apic
} WHV_X64_LOCAL_APIC_EMULATION_MODE;
```
Capability gate: `WHV_CAPABILITY_FEATURES.LocalApicEmulation : 1` (bit 1), plus `ApicRemoteRead : 1` (bit 5) and `IdleSuspend : 1` (bit 6). Set via `WHvPartitionPropertyCodeLocalApicEmulationMode = 0x00001005` before `WHvSetupPartition` (QEMU and OpenVMM both set it on the pre-setup config; docs: processor-related properties "can only be configured during the initial creation of the partition, prior to calling WHvSetupPartition").

Docs (overview, "Interrupt controller virtualization"): "Optionally, the hypervisor platform can emulate a local APIC interrupt controller. For virtual machines where an APIC is required, using the platform's built-in emulation yields the best performance." "When this functionality is enabled, these functions can be used to request virtual interrupts and to query and set interrupt controller state." `WHvGetInterruptTargetVpSet`: "If the partition is not configured to emulate the local APIC, the function returns HRESULT_FROM_WIN32(ERROR_HV_OPERATION_DENIED)."

What is inside the hypervisor (evidence: the APIC registers exist as hypervisor VP registers; the APIC counters are hypervisor counters; the only APIC exits are EOI-of-level and the optional traps):
```c
    // APIC state (also accessible via WHv(Get/Set)VirtualProcessorInterruptControllerState)
    WHvX64RegisterApicId           = 0x00003002,
    WHvX64RegisterApicVersion      = 0x00003003,
    // X2APIC state (also accessible via WHv(Get/Set)VirtualProcessorInterruptControllerState)
    WHvX64RegisterApicTpr          = 0x00003008,
    WHvX64RegisterApicPpr          = 0x0000300A,
    WHvX64RegisterApicEoi          = 0x0000300B,
    WHvX64RegisterApicLdr          = 0x0000300D,
    WHvX64RegisterApicSpurious     = 0x0000300F,
    WHvX64RegisterApicIsr0..7      = 0x00003010..0x00003017,
    WHvX64RegisterApicTmr0..7      = 0x00003018..0x0000301F,
    WHvX64RegisterApicIrr0..7      = 0x00003020..0x00003027,
    WHvX64RegisterApicEse          = 0x00003028,
    WHvX64RegisterApicIcr          = 0x00003030,
    WHvX64RegisterApicLvtTimer     = 0x00003032,
    WHvX64RegisterApicLvtThermal   = 0x00003033,
    WHvX64RegisterApicLvtPerfmon   = 0x00003034,
    WHvX64RegisterApicLvtLint0     = 0x00003035,
    WHvX64RegisterApicLvtLint1     = 0x00003036,
    WHvX64RegisterApicLvtError     = 0x00003037,
    WHvX64RegisterApicInitCount    = 0x00003038,
    WHvX64RegisterApicCurrentCount = 0x00003039,
    WHvX64RegisterApicDivide       = 0x0000303E,
    WHvX64RegisterApicSelfIpi      = 0x0000303F,
```
```c
typedef struct WHV_PROCESSOR_APIC_COUNTERS
{
    UINT64 MmioAccessCount;
    UINT64 EoiAccessCount;
    UINT64 TprAccessCount;
    UINT64 SentIpiCount;
    UINT64 SelfIpiCount;
} WHV_PROCESSOR_APIC_COUNTERS;
```
So in XApic/X2Apic mode the hypervisor owns: the LAPIC MMIO page (MmioAccessCount), all LAPIC registers incl. the timer (InitCount/CurrentCount/Divide/LvtTimer are hypervisor registers; `WHvX64RegisterTscDeadline = 0x00002095` and `TscDeadlineTmrSupport` in `WHV_X64_PROCESSOR_FEATURES1`), IPIs (SentIpiCount/SelfIpiCount, INIT/SIPI wait-for-SIPI state as `StartupSuspend`), EOI (EoiAccessCount), TPR/CR8 (TprAccessCount; `WHV_X64_VP_EXIT_CONTEXT.Cr8 : 4` reports it on every exit), x2APIC MSRs (X2Apic mode; QEMU sets `WHvX64LocalApicEmulationModeX2Apic`), and the APIC timer bus clock (`WHvCapabilityCodeInterruptClockFrequency`; QEMU: "When not using the Hyper-V APIC, the frequency is 1 GHz", otherwise it takes the capability value). The APIC base MSR remains writable by the VMM as `WHvX64RegisterApicBase = 0x00002003` and observable via `WHV_X64_MSR_EXIT_BITMAP.ApicBaseMsrWrite`; QEMU 2026: "Hyper-V always has CPUID[1:EDX].APIC set, even when the APIC isn't enabled yet. Work around this by also using the APICBASE trap for kernel-irqchip=on." Confidence: high (header + two VMMs), except "timer is in the hypervisor" which is inferred from the register set and QEMU pulling `initial_count`/`divide_conf` out of the hypervisor state.

What the VMM still sees: `X64ApicEoi` (level-triggered EOIs only), `X64InterruptWindow` (only if requested via DeliverabilityNotifications), optional traps below, ApicBase MSR writes (if bitmap bit set), CR8 in every exit context. Nothing else APIC-related exits.

APIC-related exit reasons and their enabling bits (header, x64):
```c
    WHvRunVpExitReasonX64InterruptWindow     = 0x00000007,
    WHvRunVpExitReasonX64Halt                = 0x00000008,
    WHvRunVpExitReasonX64ApicEoi             = 0x00000009,
    WHvRunVpExitReasonSynicSintDeliverable   = 0x0000000A,
    WHvRunVpExitReasonX64ApicSmiTrap         = 0x00001004,
    WHvRunVpExitReasonX64ApicInitSipiTrap    = 0x00001006,
    WHvRunVpExitReasonX64ApicWriteTrap       = 0x00001007,
```
```c
        UINT64 X64ApicSmiExitTrap         : 1; // WHvRunVpExitReasonX64ApicSmiTrap supported
        UINT64 HypercallExit              : 1; // WHvRunVpExitReasonHypercall supported
        UINT64 X64ApicInitSipiExitTrap    : 1; // WHvRunVpExitReasonX64ApicInitSipiTrap supported
        UINT64 X64ApicWriteLint0ExitTrap  : 1; // WHvRunVpExitReasonX64ApicWriteTrap supported
        UINT64 X64ApicWriteLint1ExitTrap  : 1; // WHvRunVpExitReasonX64ApicWriteTrap supported
        UINT64 X64ApicWriteSvrExitTrap    : 1; // WHvRunVpExitReasonX64ApicWriteTrap supported
        UINT64 UnknownSynicConnection     : 1; // WHvRunVpExitReasonHypercall supported for unknown synic connection to HvSignalEvent
        UINT64 RetargetUnknownVpciDevice  : 1; // WHvRunVpExitReasonHypercall supported for unknown device to HvRetargetDeviceInterrupt
        UINT64 X64ApicWriteLdrExitTrap    : 1; // WHvRunVpExitReasonX64ApicWriteTrap supported
        UINT64 X64ApicWriteDfrExitTrap    : 1; // WHvRunVpExitReasonX64ApicWriteTrap supported
        UINT64 GpaAccessFaultExit         : 1; // WHvRunVpExitReasonMemoryAccess supported for second-level page faults
```
There is NO ICR-write trap and NO TPR-write trap in this header (only Ldr/Dfr/Svr/Lint0/Lint1).
```c
// Context data for an exit caused by an APIC EOI of a level-triggered
// interrupt (WHvRunVpExitReasonX64ApicEoi)
typedef struct WHV_X64_APIC_EOI_CONTEXT { UINT32 InterruptVector; } WHV_X64_APIC_EOI_CONTEXT;
typedef struct WHV_X64_APIC_SMI_CONTEXT { UINT64 ApicIcr; } WHV_X64_APIC_SMI_CONTEXT;
typedef struct WHV_X64_APIC_INIT_SIPI_CONTEXT { UINT64 ApicIcr; } WHV_X64_APIC_INIT_SIPI_CONTEXT;
typedef enum WHV_X64_APIC_WRITE_TYPE
{
    WHvX64ApicWriteTypeLdr   = 0xD0,
    WHvX64ApicWriteTypeDfr   = 0xE0,
    WHvX64ApicWriteTypeSvr   = 0xF0,
    WHvX64ApicWriteTypeLint0 = 0x350,
    WHvX64ApicWriteTypeLint1 = 0x360
} WHV_X64_APIC_WRITE_TYPE;
typedef struct WHV_X64_APIC_WRITE_CONTEXT
{
    WHV_X64_APIC_WRITE_TYPE Type;
    UINT32 Reserved;
    UINT64 WriteValue;
} WHV_X64_APIC_WRITE_CONTEXT;
```
### A.2 WHvRequestInterrupt, level/EOI, ExtINT and the 8259 path

Header:
```c
typedef enum WHV_INTERRUPT_TYPE
{
    WHvX64InterruptTypeFixed            = 0,
    WHvX64InterruptTypeLowestPriority   = 1,
    WHvX64InterruptTypeNmi              = 4,
    WHvX64InterruptTypeInit             = 5,
    WHvX64InterruptTypeSipi             = 6,
    WHvX64InterruptTypeLocalInt1        = 9,
} WHV_INTERRUPT_TYPE;
typedef enum WHV_INTERRUPT_DESTINATION_MODE { WHvX64InterruptDestinationModePhysical, WHvX64InterruptDestinationModeLogical, } WHV_INTERRUPT_DESTINATION_MODE;
typedef enum WHV_INTERRUPT_TRIGGER_MODE { WHvX64InterruptTriggerModeEdge, WHvX64InterruptTriggerModeLevel, } WHV_INTERRUPT_TRIGGER_MODE;
typedef struct WHV_INTERRUPT_CONTROL
{
    UINT64 Type : 8;             // WHV_INTERRUPT_TYPE
    UINT64 DestinationMode : 4;  // WHV_INTERRUPT_DESTINATION_MODE
    UINT64 TriggerMode : 4;      // WHV_INTERRUPT_TRIGGER_MODE
    UINT64 TargetVtl : 8;        // WHV_VTL
    UINT64 Reserved : 40;
    UINT32 Destination;
    UINT32 Vector;
} WHV_INTERRUPT_CONTROL;
```
The gaps 2 (Smi), 3 (RemoteRead), 7 (ExtInt), 8 (LocalInt0) of Hyper-V's `HV_INTERRUPT_TYPE` are absent: WHP exposes no ExtINT/LINT0 delivery through `WHvRequestInterrupt`. Both reference VMMs pass the raw x86 delivery-mode field as `Type` anyway (QEMU `whpx_send_msi`: "/* Values correspond to delivery modes */ .Type = delivery,"; OpenVMM: "// WHP interrupt type has the same format as mshv interrupt type."), so an IOAPIC RTE programmed ExtINT would send 7 — whether the hypervisor accepts 7 is unverified.

Threading: docs say only "requests that an interrupt described by the WHV_INTERRUPT_CONTROL structure be delivered to the partition" (min. Windows 10 1809). De facto it is called concurrently with running VPs: QEMU calls it from the MSI/IOAPIC path (`whpx_apic_mem_write -> whpx_send_msi`) on the I/O thread; OpenVMM from device tasks (`apic.rs interrupt()`); triggers (G) exist to do the same from kernel mode. Confidence: high (practice), docs silent.

Level-triggered → EOI exit: header comment "Context data for an exit caused by an APIC EOI of a level-triggered interrupt (WHvRunVpExitReasonX64ApicEoi)". QEMU: `case WHvRunVpExitReasonX64ApicEoi: assert(whpx_irqchip_in_kernel()); ioapic_eoi_broadcast(vcpu->exit_ctx.ApicEoi.InterruptVector);`. OpenVMM: `dev.handle_eoi(info.InterruptVector)`. This is the IOAPIC EOI-resample path; edge requests produce no EOI exit. Confidence: high.

LINT1/NMI pin: `WHvX64InterruptTypeLocalInt1` is present; OpenVMM `lint()` for index 1 in offloaded mode calls `whp.interrupt(WHvX64InterruptTypeLocalInt1, Physical, Edge, vp_index, 0)`.

8259 PIC / ExtINT with the hypervisor APIC: delivered by the VMM through the pending-event register, gated by the interrupt-window exit:
```c
typedef enum WHV_X64_PENDING_EVENT_TYPE
{
    WHvX64PendingEventException = 0,
    WHvX64PendingEventExtInt    = 5,
    WHvX64PendingEventSvmNestedExit = 7,
    WHvX64PendingEventVmxNestedExit = 8,
} WHV_X64_PENDING_EVENT_TYPE;
typedef union WHV_X64_PENDING_EXT_INT_EVENT
{
    struct
    {
        UINT64 EventPending     : 1;
        UINT64 EventType        : 3; // Must be WHvX64PendingEventExtInt
        UINT64 Reserved0        : 4;
        UINT64 Vector           : 8;
        UINT64 Reserved1        : 48;
        UINT64 Reserved2;
    };
    WHV_UINT128 AsUINT128;
} WHV_X64_PENDING_EXT_INT_EVENT;
typedef union WHV_DELIVERABILITY_NOTIFICATIONS_REGISTER
{
    struct
    {
        UINT64 NmiNotification:1;
        UINT64 InterruptNotification:1;
        UINT64 InterruptPriority:4;
        UINT64 Reserved:42;
        UINT64 Sint:16;
    };
    UINT64 AsUINT64;
} WHV_DELIVERABILITY_NOTIFICATIONS_REGISTER, WHV_X64_DELIVERABILITY_NOTIFICATIONS_REGISTER;
```
QEMU `whpx_vcpu_pre_run` (kernel-irqchip on): on `WHvRunVpExitReasonX64InterruptWindow` it sets `ready_for_pic_interrupt = 1`; then `if (vcpu->ready_for_pic_interrupt && cpu_test_interrupt(cpu, CPU_INTERRUPT_HARD)) { irq = cpu_get_pic_interrupt(env); ... reg_names[reg_count] = WHvRegisterPendingEvent; reg_values[reg_count].ExtIntEvent = (WHV_X64_PENDING_EXT_INT_EVENT){ .EventPending = 1, .EventType = WHvX64PendingEventExtInt, .Vector = irq }; ... if (whpx_irqchip_in_kernel()) { whpx_vcpu_kick_out_of_hlt(cpu); } }` and registers the window with `DeliverabilityNotifications = { .InterruptNotification = 1, .InterruptPriority = irr >> 4 }`. OpenVMM `inject_extint()` does the same readiness test itself (`pending_interruption.interruption_pending() || interrupt_state.interrupt_shadow() || !rflags.interrupt_enable() || pending_event.event_pending()` → request `with_interrupt_notification(true).with_interrupt_priority(0)` and return), then acknowledges its PIC and writes `(PendingEvent, ext-int event), (InternalActivityState, 0)` in one `set_registers!` call with the comment "TEMPORARY: force the processor out of the halted state--otherwise, the hypervisor delays injecting the interrupt." Both do this on the VP thread. Confidence: high.

QEMU docs (whpx.html) known issue: "On Windows 10, a legacy PIC interrupt injected does not wake the guest from an HLT when using the Hyper-V provided interrupt controller." "As such, on Windows 10, using the Hyper-V interrupt controller is disabled by default. You can enable it via -M q35,pic=off which disables the PIC." — i.e. the HaltSuspend-clearing workaround (A.5) works on Windows 11, not on Windows 10.

### A.3 Save/restore of the in-hypervisor LAPIC
No layout struct exists in either header (`HV_X64_INTERRUPT_CONTROLLER_STATE` is not defined). Docs (State2): "retrieves the local APIC state of the specified virtual processor in the standard external state format. It supersedes the deprecated WHvGetVirtualProcessorInterruptControllerState, which returns a legacy format that packs the interrupt-request, in-service, and trigger-mode vectors differently from the standard external state format and does not include the processor priority register." "The function requires emulation of the local APIC to be configured for the partition." Errors: `WHV_E_INSUFFICIENT_BUFFER`, `WHV_E_VP_DOES_NOT_EXIST`, `WHV_E_INVALID_VP_STATE` ("cannot be accessed in its current state"). Both State/State2 functions are `#pragma deprecated` in WinHvPlatform.h; use `WHvGetVirtualProcessorState(..., WHvVirtualProcessorStateTypeInterruptControllerState2 = 0x00001000, ...)`; set "must be exactly the size of the state".

The format, from code: QEMU `struct whpx_lapic_state { struct { uint32_t data; uint32_t padding[3]; } fields[256]; };` (4096 bytes; index = xAPIC MMIO offset >> 4: 0x2 ID (`s->id << 24` in xAPIC, raw x2APIC id when EXTD), 0x3 version, 0x8 TPR, 0x9 arbitration id (read-only), 0xd LDR (`log_dest << 24`), 0xe DFR (`dest_mode << 28 | 0x0fffffff`), 0xf SVR, 0x10–0x17 ISR, 0x18–0x1f TMR, 0x20–0x27 IRR, 0x28 ESR, 0x30/0x31 ICR lo/hi, 0x32.. LVTs, 0x38 initial count, 0x3e divide). OpenVMM `get_apic()`: "Only the first 1024 bytes are used but the API requires a full page, for some unknown reason." Also: `WHvVirtualProcessorStateTypeSynicTimerState = 0x00000002` (opaque; the hypervisor owns the synthetic timers), `SynicMessagePage = 0`, `SynicEventFlagPage = 1`, `XsaveState = 0x1001`, `NestedState = 0x1002`. Confidence: high for layout (two independent VMMs agree).

### A.4 Other APIC-related knobs
`WHvPartitionPropertyCodeApicRemoteReadSupport = 0x00001009` (`BOOL ApicRemoteRead`, capability bit `ApicRemoteRead`) — the xAPIC remote-read ICR mode; no docs prose. `WHvX64RegisterInitialApicId = 0x0000200C` — OpenVMM sets it when `apic_id != vp_index` for the offloaded APIC, so APIC ID ≠ VP index is possible (docs: "The index of the virtual processor is used to set the APIC ID"). `WHvX64RegisterApicBase = 0x00002003` + `X64MsrExitBitmap.ApicBaseMsrWrite`. `WHvRegisterPendingInterruption` (D.3) still works with the hypervisor APIC (QEMU injects NMIs that way in both modes; OpenVMM `inject_nmi` too); `WHvRegisterDeliverabilityNotifications = WHvX64RegisterDeliverabilityNotifications = 0x80000004`.

### A.5 Halt semantics
Header:
```c
typedef union WHV_INTERNAL_ACTIVITY_REGISTER
{
    struct { UINT64 StartupSuspend : 1; UINT64 HaltSuspend : 1; UINT64 IdleSuspend : 1; UINT64 Reserved:61; };
    UINT64 AsUINT64;
} WHV_INTERNAL_ACTIVITY_REGISTER;   // WHvRegisterInternalActivityState = 0x80000005
```
Docs say nothing about HLT in APIC mode. Evidence: (1) QEMU comment (2025, kick-out-of-HLT patch): "When the Hyper-V APIC is enabled, to get out of HLT we either have to request an interrupt or manually get it away from HLT. We also manually do inject some interrupts via WHvRegisterPendingEvent instead of WHVRequestInterrupt, which does not reset the HLT state. ... Keep it this way for now, with perhaps adding a heartbeat later so that we get the CPU time savings from having Hyper-V handle HLT instead of going away from it as soon as possible." and `whpx_vcpu_kick_out_of_hlt`: read `WHvRegisterInternalActivityState`, if `HaltSuspend` clear it and write back; the 2020 kernel-irqchip patch adds `if (cpu->halted && !whpx_apic_in_platform())` so QEMU only parks the thread on halt when the APIC is NOT in the platform. (2) OpenVMM `process_apic()` decides run-readiness as `halted = lapic.is_some_and(|lapic| self.state.halted || lapic.startup_suspend); !halted` — with the offloaded APIC (`lapic == None`) the VMM never treats the VP as halted and always re-enters `WHvRunVirtualProcessor`, leaving the sleep to the hypervisor. (3) Our probe: under XApic a halted VP never leaves the run call; under None it exits `X64Halt`. Conclusion: in XApic/X2Apic the VP sleeps inside `WHvRunVirtualProcessor` in `HaltSuspend`; it is woken by anything the hypervisor APIC delivers (WHvRequestInterrupt, triggers, IPIs, APIC/synthetic timers) or by `WHvCancelRunVirtualProcessor`; a VMM-written `PendingInterruption`/`PendingEvent` does not clear `HaltSuspend`, so the VMM must clear it. Whether an `X64Halt` exit can still occur in APIC mode is unproven by docs (both VMMs keep a handler; OpenVMM's just sets a flag the offloaded path ignores). Confidence: high on behaviour, medium on "never exits".

## B. Time and clocks

### B.1 TSC registers, frequencies, reference time, suspend/resume
Header registers: `WHvX64RegisterTsc = 0x00002000`, `WHvX64RegisterTscAux = 0x0000207B`, `WHvX64RegisterTscVirtualOffset = 0x00002087`, `WHvX64RegisterTscDeadline = 0x00002095`, `WHvX64RegisterTscAdjust = 0x00002096`, `WHvRegisterVpRuntime = 0x00005000`, `WHvRegisterReferenceTsc = 0x00005017`, `WHvRegisterReferenceTscSequence = 0x0000501A`. Feature bits (bank1): `TscInvariantSupport`, `TscDeadlineTmrSupport`, `TscAdjustSupport`. There is no `WHvRegisterTscFrequency` and no TSC-scaling knob anywhere in the header.

Frequencies: `WHvCapabilityCodeProcessorClockFrequency = 0x00001004` / `WHvPartitionPropertyCodeProcessorClockFrequency = 0x00001007` (`UINT64 ProcessorClockFrequency`), `WHvCapabilityCodeInterruptClockFrequency = 0x00001005` / `WHvPartitionPropertyCodeInterruptClockFrequency = 0x00001008` (`UINT64 InterruptClockFrequency`). Docs give no units. QEMU: `env->tsc_khz = freq / 1000; /* Hz to KHz */` from `WHvCapabilityCodeProcessorClockFrequency` (comment: "vcpu's TSC frequency is either specified by user, or use the value provided by Hyper-V"), and `env->apic_bus_freq = freq` from `WHvCapabilityCodeInterruptClockFrequency` (default `HYPERV_APIC_BUS_FREQUENCY`; "When not using the Hyper-V APIC, the frequency is 1 GHz" — QEMU's own model). OpenVMM: `tsc_frequency() = get_property(ProcessorClockFrequency)`, fed to its Hv#1 emulator as the guest TSC frequency. So: ProcessorClockFrequency = guest/host TSC rate in Hz; InterruptClockFrequency = hypervisor APIC timer bus clock in Hz. The property being in `WHV_PARTITION_PROPERTY` suggests settability, but no VMM sets it and docs list only ExtendedVmExits/ExceptionExitBitmap/X64MsrExitBitmap/CpuidExitList as post-setup settable — treat the guest TSC as running at host rate (TscInvariant) with an offset. Confidence: high on units, unverified on scaling.

TSC write semantics: QEMU `whpx_set_tsc()`: "Suspend the partition prior to setting the TSC to reduce the variance in TSC across vCPUs. When the first vCPU runs post suspend, the partition is automatically resumed." then `WHvSetVirtualProcessorRegisters(..., WHvX64RegisterTsc, ...)` per VP; its commit: "Setting TSC at runtime is heavy and additionally can have side effects on the guest, which are not very resilient to variances in the TSC." — QEMU only writes TSC on full-state load/reset, never per slice. `TscVirtualOffset`: appears in the RDTSC exit context alongside `Tsc` and `ReferenceTime` (B.2); its semantics (guest TSC = hardware TSC + VirtualOffset) are inferred from the name and the Hyper-V `HvX64RegisterTscVirtualOffset` register; no docs. Unverified.

ReferenceTime: `WHvPartitionPropertyCodeReferenceTime = 0x0000100B` (`UINT64 ReferenceTime`), read with `WHvGetPartitionProperty`; docs give no prose. OpenVMM `reference_time()` returns it as the Hv#1 reference time in 100 ns units. TLFS (the same counter the guest reads as `HV_X64_MSR_TIME_REF_COUNT`): "successive accesses to it return strictly monotonically increasing (time) values as seen by any and all virtual processors of a partition ... rate constant ... initialized to zero when the partition is created ... The reference counter continues to count up as long as at least one virtual processor is not explicitly suspended." Units 100 ns (TLFS: "normalized reference time since partition creation, in 100nS units"; Linux: "advances at a constant 10 MHz frequency"). Confidence: high.

Suspend/resume (docs, min. Windows 10 1903): "suspends time for the partition. No virtual processor may be running when this is called. Time will resume when WHvResumePartitionTime or WHvRunVirtualProcessor is called." `WHvResetPartition` (Windows 11 21H2): "blocks all of the partition's virtual processors and freezes partition time; time is thawed again when a virtual processor is next run." What "time" covers is not enumerated; QEMU relies on it freezing the per-VP TSC so multi-VP writes stay coherent. Migration: `WHvStartPartitionMigration` "serializes the partition's state, properties, virtual processors, doorbells, and virtual PCI devices"; "Triggers are not preserved across live migration." Nothing is said about TSC offsetting across migration.

### B.2 RDTSC exit and MSR exit bitmap (verbatim)
```c
typedef union WHV_X64_RDTSC_INFO { struct { UINT64 IsRdtscp:1; UINT64 Reserved:63; }; UINT64 AsUINT64; } WHV_X64_RDTSC_INFO;
typedef struct WHV_X64_RDTSC_CONTEXT
{
    UINT64 TscAux;
    UINT64 VirtualOffset;
    UINT64 Tsc;
    UINT64 ReferenceTime;
    WHV_X64_RDTSC_INFO RdtscInfo;
} WHV_X64_RDTSC_CONTEXT;
```
Docs: "Exits for the rdtsc and rdtscp instructions are only generated if they are enabled by setting the WHV_EXTENDED_VM_EXITS.X64RdtscExit property for the partition." The context hands the VMM an atomic (guest `Tsc`, partition `ReferenceTime`) pair — the one documented place where the two clocks are sampled together.
```c
typedef union WHV_X64_MSR_EXIT_BITMAP
{
    UINT64 AsUINT64;
    struct
    {
        UINT64 UnhandledMsrs:1;
        UINT64 TscMsrWrite:1;
        UINT64 TscMsrRead:1;
        UINT64 ApicBaseMsrWrite:1;
        UINT64 MiscEnableMsrRead:1;
        UINT64 McUpdatePatchLevelMsrRead:1;
        UINT64 Reserved:58;
    };
} WHV_X64_MSR_EXIT_BITMAP;   // WHvPartitionPropertyCodeX64MsrExitBitmap = 0x00000005, needs ExtendedVmExits.X64MsrExit
```
Related, newer (header only, no docs prose): `WHvPartitionPropertyCodeMsrActionList = 0x0000100F` with `WHV_MSR_ACTION_ENTRY { UINT32 Index; UINT8 ReadAction; UINT8 WriteAction; UINT16 Reserved; }` and `WHvPartitionPropertyCodeUnimplementedMsrAction = 0x00001010` with `WHV_MSR_ACTION { WHvMsrActionArchitectureDefault = 0, WHvMsrActionIgnoreWriteReadZero = 1, WHvMsrActionExit = 2 }` — per-MSR policy applied in the hypervisor without an exit.

### B.3 Clocks a VMM can drive device timers from, with a fixed relation to the guest TSC
1. Partition `ReferenceTime` (100 ns, partition-wide, starts at 0, frozen only by Suspend/Reset or when every VP is suspended) — the same counter an enlightened guest reads via `HV_X64_MSR_TIME_REF_COUNT` and via the reference-TSC page (`ReferenceTime = ((VirtualTsc * TscScale) >> 64) + TscOffset`, hypervisor-maintained). Polling it costs a `WHvGetPartitionProperty` call; there is no host-side mapping of it.
2. Host `QueryPerformanceCounter`/`rdtsc` plus `ProcessorClockFrequency`: the guest TSC runs at the host TSC rate (TscInvariant), so host TSC deltas equal guest TSC deltas; the offset is fixed once by reading `WHvX64RegisterTsc` on a stopped VP (or from one RDTSC exit's `(Tsc, ReferenceTime)` pair with `X64RdtscExit` temporarily enabled).
3. Hypervisor-owned timers that need no VMM involvement at all: the LAPIC timer (incl. TSC-deadline) in XApic/X2Apic mode, and synthetic timers STIMER0–3 (100 ns units against the reference counter, direct mode asserts an APIC vector) when `AccessSyntheticTimerRegs`/`DirectSyntheticTimers` are granted (C).
4. `WHV_PROCESSOR_RUNTIME_COUNTERS { TotalRuntime100ns; HypervisorRuntime100ns; }` per VP — accounting only, not a timebase.

## C. Hyper-V enlightenments through WHP

### C.1 The feature bank (verbatim, x64 branch, comments kept)
`WHvCapabilityCodeSyntheticProcessorFeaturesBanks = 0x00001008` (read what the host allows) / `WHvPartitionPropertyCodeSyntheticProcessorFeaturesBanks = 0x0000100C` (set before setup). `WHV_SYNTHETIC_PROCESSOR_FEATURES_BANKS { UINT32 BanksCount; UINT32 Reserved0; union { struct { WHV_SYNTHETIC_PROCESSOR_FEATURES Bank0; }; UINT64 AsUINT64[1]; }; }` (16 bytes, `WHV_SYNTHETIC_PROCESSOR_FEATURES_BANKS_COUNT 1`; "BanksCount must be populated before calling WHP APIs" — header comment on the processor-features banks). Header comment: "Synthetic processor features for exo partitions."
```c
        // Report a hypervisor is present. CPUID leaves 0x40000000 and 0x40000001 are supported.
        UINT64 HypervisorPresent:1;
        // Report support for Hv1 (CPUID leaves 0x40000000 - 0x40000006).
        UINT64 Hv1:1;
        // Access to HV_X64_MSR_VP_RUNTIME. Corresponds to AccessVpRunTimeReg privilege.
        UINT64 AccessVpRunTimeReg:1;
        // Access to HV_X64_MSR_TIME_REF_COUNT. Corresponds to AccessPartitionReferenceCounter privilege.
        UINT64 AccessPartitionReferenceCounter:1;
        // Access to SINT-related registers (HV_X64_MSR_SCONTROL through HV_X64_MSR_EOM and HV_X64_MSR_SINT0 through HV_X64_MSR_SINT15). Corresponds to AccessSynicRegs privilege.
        UINT64 AccessSynicRegs:1;
        // Access to synthetic timers and associated MSRs (HV_X64_MSR_STIMER0_CONFIG through HV_X64_MSR_STIMER3_COUNT). Corresponds to AccessSyntheticTimerRegs privilege.
        UINT64 AccessSyntheticTimerRegs:1;
        // On AMD64 and ARM64, access to the the VP assist page.  On AMD64, also access to APIC MSRs (HV_X64_MSR_EOI, HV_X64_MSR_ICR and HV_X64_MSR_TPR). Corresponds to AccessIntrCtrlRegs privilege.
        UINT64 AccessIntrCtrlRegs:1;
        // Access to registers associated with hypercalls (HV_X64_MSR_GUEST_OS_ID and HV_X64_MSR_HYPERCALL). Corresponds to AccessHypercallMsrs privilege.
        UINT64 AccessHypercallRegs:1;
        // VP index can be queried. Corresponds to AccessVpIndex privilege.
        UINT64 AccessVpIndex:1;
        // Access to the reference TSC. Corresponds to AccessPartitionReferenceTsc privilege.
        UINT64 AccessPartitionReferenceTsc:1;
        // Partition has access to the guest idle reg. Corresponds to AccessGuestIdleReg privilege.
        UINT64 AccessGuestIdleReg:1;
        // Partition has access to frequency regs. Corresponds to AccessFrequencyRegs privilege.
        UINT64 AccessFrequencyRegs:1;
        UINT64 ReservedZ12:1; UINT64 ReservedZ13:1; UINT64 ReservedZ14:1;
        // Extended GVA ranges for HvCallFlushVirtualAddressList hypercall. Corresponds to privilege.
        UINT64 EnableExtendedGvaRangesForFlushVirtualAddressList:1;
        UINT64 ReservedZ16:1; UINT64 ReservedZ17:1;
        // Use fast hypercall output. Corresponds to privilege.
        UINT64 FastHypercallOutput:1;
        UINT64 ReservedZ19:1; UINT64 ReservedZ20:1; UINT64 ReservedZ21:1;
        // Synthetic timers in direct mode.
        UINT64 DirectSyntheticTimers:1;
        UINT64 ReservedZ23:1;
        // Use extended processor masks.
        UINT64 ExtendedProcessorMasks:1;
        // On AMD64, HvCallFlushVirtualAddressSpace / HvCallFlushVirtualAddressList are supported, on ARM64 HvCallFlushVirtualAddressSpace / HvCallFlushTlb are supported.
        UINT64 TbFlushHypercalls:1;
        // HvCallSendSyntheticClusterIpi is supported.
        UINT64 SyntheticClusterIpi:1;
        // HvCallNotifyLongSpinWait is supported.
        UINT64 NotifyLongSpinWait:1;
        // HvCallQueryNumaDistance is supported.
        UINT64 QueryNumaDistance:1;
        // HvCallSignalEvent is supported. Corresponds to privilege.
        UINT64 SignalEvents:1;
        // HvCallRetargetDeviceInterrupt is supported.
        UINT64 RetargetDeviceInterrupt:1;
        // HvCallRestorePartitionTime is supported.
        UINT64 RestoreTime:1;
        // EnlightenedVmcs nested enlightenment is supported.
        UINT64 EnlightenedVmcs:1;
        // Non-zero values can be written to DEBUG_CTL.
        UINT64 NestedDebugCtl:1;
        // Synthetic time-unhalted timer MSRs are supported.
        UINT64 SyntheticTimeUnhaltedTimer : 1;
        // SPEC_CTRL MSR behavior when the VP is idle
        UINT64 IdleSpecCtrl:1;
        // Register intercepts are not supported.
        UINT64 ReservedZ36 : 1;
        UINT64 WakeVps:1;
        UINT64 AccessVpRegs : 1;
        UINT64 ReservedZ39 : 1;   // ARM64: SyncContext
        UINT64 ReservedZ40 : 1;
        UINT64 Reserved:23;
```
Absent from this bank: any reenlightenment/TSC-emulation control (`AccessReenlightenmentControls`), `AccessResetReg`, `AccessStatsReg`. OpenVMM's copy of this ABI has an `InterceptSystemReset` bit this header lacks — a newer SDK adds it (F). QEMU comment: "WHvCapabilityCodeProcessorPerfmonFeatures and WHvPartitionPropertyCodeSyntheticProcessorFeaturesBanks are implemented starting from Windows Server 2022 (build 20348)."

### C.2 Hypervisor-implemented vs VMM-serviced
Each bit "Corresponds to ... privilege" = a bit of the TLFS `HV_PARTITION_PRIVILEGE_MASK` reported in CPUID 0x40000003, and each privilege "controls access to a set of synthetic MSRs and/or hypercalls" that the hypervisor implements. Consequently, with the bit set, the hypervisor itself services — with zero VMM involvement — TIME_REF_COUNT, the reference-TSC page (scale/offset/sequence maintained by the hypervisor), STIMER0–3 (message or direct-vector delivery into the hypervisor APIC, state saved via `SynicTimerState`), the synic registers/pages (SCONTROL, SIEFP, SIMP, EOM, SINT0–15 — `WHvRegisterSint0..15 = 0x4000..0x400F`, `Scontrol = 0x4010`, `Sversion = 0x4011`, `Siefp = 0x4012`, `Simp = 0x4013`, `Eom = 0x4014` exist as VP registers for save/restore), VP index/runtime, hypercall page + GuestOsId (`WHvX64RegisterHypercall = 0x5001`, `WHvRegisterGuestOsId = 0x5002`), VP assist page (`WHvRegisterVpAssistPage = 0x5013`) with APIC EOI/ICR/TPR MSRs, guest-idle and frequency MSRs, and the pure-hypervisor hypercalls (TLB flush, cluster IPI, NotifyLongSpinWait, QueryNumaDistance). Accounting for these is exposed as `WHV_PROCESSOR_SYNTHETIC_FEATURES_COUNTERS { SyntheticInterruptsCount; LongSpinWaitHypercallsCount; OtherHypercallsCount; SyntheticInterruptHypercallsCount; VirtualInterruptHypercallsCount; VirtualMmuHypercallsCount; }` (`WHvProcessorCounterSetSyntheticFeatures = 4`, header only, not in docs).

What reaches the VMM: (a) `WHvRunVpExitReasonHypercall` (needs `ExtendedVmExits.HypercallExit`) with
```c
typedef struct WHV_X64_HYPERCALL_CONTEXT
{
    UINT64 Rax; UINT64 Rbx; UINT64 Rcx; UINT64 Rdx; UINT64 R8; UINT64 Rsi; UINT64 Rdi; UINT64 Reserved0;
    WHV_UINT128 XmmRegisters[WHV_HYPERCALL_CONTEXT_MAX_XMM_REGISTERS /* 6 */];
    UINT64 Reserved1[2];
} WHV_X64_HYPERCALL_CONTEXT, WHV_HYPERCALL_CONTEXT;   // 176 bytes
```
OpenVMM's dispatcher (the set it must answer itself on WHP): HvPostMessage, HvSignalEvent, HvRetargetDeviceInterrupt, HvGetVpRegisters, HvSetVpRegisters, HvVtlReturn, HvInstallIntercept, HvX64TranslateVirtualAddress(Ex), HvAssertVirtualInterrupt, HvPostMessageDirect, HvSignalEventDirect, HvX64EnableVpVtl, HvX64StartVirtualProcessor, HvModifyVtlProtectionMask, HvGetVpIndexFromApicId, HvAcceptGpaPages, HvModifySparseGpaPageHostVisibility, HvVbsVmCallReport; results written back with `WHvSetVirtualProcessorRegisters`. TLB-flush / cluster-IPI / spin-wait hypercalls are not in that list: the hypervisor consumes them. `UnknownSynicConnection` / `RetargetUnknownVpciDevice` make HvSignalEvent / HvRetargetDeviceInterrupt reach the VMM only for connection IDs / devices the hypervisor does not know (notification ports and VPCI devices are the known ones). (b) `WHvRunVpExitReasonSynicSintDeliverable` with `WHV_SYNIC_SINT_DELIVERABLE_CONTEXT { UINT16 DeliverableSints; UINT16 Reserved1; UINT32 Reserved2; }` — raised for SINTs the VMM asked about through `DeliverabilityNotifications.Sint:16`, i.e. only for messages/events the VMM itself wants to post (`WHvPostVirtualProcessorSynicMessage`: "delivers a message to the SynIC message page of the target virtual processor on the specified SINT, as though the message had been sent by the hypervisor ... Each SINT has a single message slot. If the previous message on that SINT has not yet been consumed by the guest, the call fails with ERROR_HV_OBJECT_IN_USE"; `WHvSignalVirtualProcessorSynicEvent`: "sets the event flag ... and delivers an edge-triggered interrupt on the specified SINT", `ERROR_HV_INVALID_SYNIC_STATE` if the SINT is masked). Timer expirations never reach the VMM. Confidence: high.

### C.3 Identity and the hypervisor-present bit
Header: `HypervisorPresent` → "CPUID leaves 0x40000000 and 0x40000001 are supported"; `Hv1` → "CPUID leaves 0x40000000 - 0x40000006", so the hypervisor authors them, and per the TLFS they read "Microsoft Hv" / "Hv#1". Without `HypervisorPresent`, WHP does NOT set CPUID.1:ECX[31]: QEMU's 2018 patch "Implements the CPUID trap for CPUID 1 to include the CPUID_EXT_HYPERVISOR flag in the ECX results. This was preventing some older linux kernels from booting", and current QEMU "CPUID_EXT_HYPERVISOR and CPUID_HT should be considered present always, so report them as unconditionally supported here." QEMU (enlightenments off) rewrites leaf 0x40000000 through a CPUID exit to report "KVMKVMKVM" or the VMware signature + leaf 0x40000010 (TSC/APIC kHz) — a vendor override is possible when the hypervisor is not authoring the leaves. No x64 vendor-id knob exists in the header (ARM64 has partition-wide `WHvRegisterHypervisorVersion/PrivilegesAndFeaturesInfo/...` overrides; x64 only has `CpuidResultList2` with masks — whether it beats a hypervisor-authored 0x4000xxxx leaf is unverified). Guest features that light up with OpenVMM's VTL0 set (`HypervisorPresent | Hv1 | AccessVpRunTimeReg | AccessPartitionReferenceCounter | AccessHypercallRegs | AccessVpIndex | AccessPartitionReferenceTsc | AccessSynicRegs | AccessSyntheticTimerRegs | FastHypercallOutput | ExtendedProcessorMasks | SyntheticClusterIpi | NotifyLongSpinWait | QueryNumaDistance | SignalEvents | RetargetDeviceInterrupt | TbFlushHypercalls | AccessGuestIdleReg | AccessFrequencyRegs | EnableExtendedGvaRangesForFlushVirtualAddressList | AccessIntrCtrlRegs | DirectSyntheticTimers`, the last two "if vtl == Vtl::Vtl0"): Linux `hyperv_clocksource_tsc_page` (10 MHz) + stimer0 direct-mode clockevent ("Linux prefers to use Direct Mode when available"), Windows' reference-time/stimer/APIC-assist (lazy EOI via the VP assist page — OpenVMM `sync_lazy_eoi()` before each run), TLB-flush and cluster-IPI hypercalls, spinlock notify, guest-idle. OpenVMM enables this block only when `offload_enlightenments && !user_mode_apic` — the enlightenments presuppose the hypervisor APIC. QEMU issue #2063 measured the opposite case: a WHPX Windows 10 guest with none of these is "essentially unusable, compared to same image running under Hyper-V on the same host system" and "hv-XXX cpu options do not appear applicable to -accel WHPX"; QEMU has since grown a `hyperv_enlightenments_enabled` path (handles `HV_X64_MSR_GUEST_IDLE` reads as halt: "Windows 11 25H2 uses it even when not advertised", ignores `HV_X64_MSR_VP_ASSIST_PAGE` writes: "Linux tries to use it anyway even when not exposed").

## D. Run loop, cancel, VP state

### D.1 Run / cancel
Docs (WHvRunVirtualProcessor): "The call blocks synchronously until either the virtual processor performs an operation that the virtualization stack must handle (for example, accessing memory in the GPA space that is not mapped or not accessible) or the virtualization stack explicitly requests an exit of the function (for example, to inject an interrupt for the virtual processor or to change the state of the VM)." Overview: "To run a virtual processor, a thread in the virtualization-stack process issues a blocking call to execute the virtual processor in the hypervisor." with the 6-step per-thread loop (create → set state incl. pending interrupts → run → handle exit → repeat → delete). Cancel (docs, `Flags` "Unused, must be zero"): "allows an application to abort the call to run the virtual processor by another thread, and to return the control to that thread." Stickiness is not documented; the only hint is `WHvResetPartition`: "per-virtual-processor state such as pending suspend, cancel, and dispatch-notification state is cleared" — a pending cancel is persisted per-VP state, so a cancel issued while the VP is not in Run most likely makes the next Run return immediately. QEMU relies on exactly that (`whpx_vcpu_kick` = `WHvCancelRunVirtualProcessor` called from `whpx_vcpu_run` right before `WHvRunVirtualProcessor` when `exit_request` is set). Confidence: medium (inferred).
```c
typedef enum WHV_RUN_VP_CANCEL_REASON { WHvRunVpCancelReasonUser = 0 } WHV_RUN_VP_CANCEL_REASON; // Execution canceled by WHvCancelRunVirtualProcessor
typedef struct WHV_RUN_VP_CANCELED_CONTEXT { WHV_RUN_VP_CANCEL_REASON CancelReason; } WHV_RUN_VP_CANCELED_CONTEXT;
// WHvRunVpExitReasonCanceled = 0x00002001 on x64 (0xFFFFFFFF on ARM64)
```

### D.2 Exit context header (verbatim)
```c
typedef union WHV_X64_VP_EXECUTION_STATE
{
    struct
    {
        UINT16 Cpl : 2;
        UINT16 Cr0Pe : 1;
        UINT16 Cr0Am : 1;
        UINT16 EferLma : 1;
        UINT16 DebugActive : 1;
        UINT16 InterruptionPending : 1;
        UINT16 Reserved0 : 5;
        UINT16 InterruptShadow : 1;
        UINT16 Reserved1 : 3;
    };
    UINT16 AsUINT16;
} WHV_X64_VP_EXECUTION_STATE;
typedef struct WHV_X64_VP_EXIT_CONTEXT
{
    WHV_X64_VP_EXECUTION_STATE ExecutionState;
    UINT8 InstructionLength : 4;
    UINT8 Cr8 : 4;
    UINT8 Reserved;
    UINT32 Reserved2;
    WHV_X64_SEGMENT_REGISTER Cs;
    UINT64 Rip;
    UINT64 Rflags;
} WHV_X64_VP_EXIT_CONTEXT, WHV_VP_EXIT_CONTEXT;      // 40 bytes; WHV_RUN_VP_EXIT_CONTEXT is 224 bytes
```
QEMU derives `vcpu->interruptable = !exit_ctx.VpContext.ExecutionState.InterruptShadow` and `interruption_pending` from this header after every exit, so the readiness test for its own injections needs no register read.

### D.3 Interrupt/event registers (verbatim)
```c
typedef union WHV_X64_INTERRUPT_STATE_REGISTER            // WHvRegisterInterruptState = 0x80000001
{ struct { UINT64 InterruptShadow:1; UINT64 NmiMasked:1; UINT64 Reserved:62; }; UINT64 AsUINT64; } WHV_X64_INTERRUPT_STATE_REGISTER;
typedef union WHV_X64_PENDING_INTERRUPTION_REGISTER       // WHvRegisterPendingInterruption = 0x80000000
{
    struct
    {
        UINT32 InterruptionPending:1;
        UINT32 InterruptionType:3;  // WHV_X64_PENDING_INTERRUPTION_TYPE: WHvX64PendingInterrupt=0, WHvX64PendingNmi=2, WHvX64PendingException=3
        UINT32 DeliverErrorCode:1;
        UINT32 InstructionLength:4;
        UINT32 NestedEvent:1;
        UINT32 Reserved:6;
        UINT32 InterruptionVector:16;
        UINT32 ErrorCode;
    };
    UINT64 AsUINT64;
} WHV_X64_PENDING_INTERRUPTION_REGISTER;
typedef union WHV_X64_PENDING_EXCEPTION_EVENT             // WHvRegisterPendingEvent = 0x80000002 (128-bit); PendingEvent1 = 0x80000003, PendingEvent2/3 = 0x80000007/8 (nested exits)
{
    struct
    {
        UINT32 EventPending         : 1;
        UINT32 EventType            : 3; // Must be WHvX64PendingEventException
        UINT32 Reserved0            : 4;
        UINT32 DeliverErrorCode     : 1;
        UINT32 Reserved1            : 7;
        UINT32 Vector               : 16;
        UINT32 ErrorCode;
        UINT64 ExceptionParameter;
    };
    WHV_UINT128 AsUINT128;
} WHV_X64_PENDING_EXCEPTION_EVENT;
typedef union WHV_X64_PENDING_DEBUG_EXCEPTION               // WHvX64RegisterPendingDebugException = 0x80000006
{ UINT64 AsUINT64; struct { UINT64 Breakpoint0 : 1; UINT64 Breakpoint1 : 1; UINT64 Breakpoint2 : 1; UINT64 Breakpoint3 : 1; UINT64 SingleStep : 1; UINT64 Reserved0 : 59; }; } WHV_X64_PENDING_DEBUG_EXCEPTION;
typedef struct WHV_X64_INTERRUPTION_DELIVERABLE_CONTEXT { WHV_X64_PENDING_INTERRUPTION_TYPE DeliverableType; } WHV_X64_INTERRUPTION_DELIVERABLE_CONTEXT;  // X64InterruptWindow exit
```
(`WHV_X64_PENDING_EXT_INT_EVENT` and `WHV_DELIVERABILITY_NOTIFICATIONS_REGISTER` are quoted in A.2.) Docs for the window exit: "an exit that occurs when the interruptibility state of the virtual processor would allow delivery of a given interrupt". `PendingInterruption` is one slot: QEMU replays an interrupt "that was overwritten" when `InterruptionPending` is still set at the next pre-run; OpenVMM injects a #GP for unhandled MSRs through `PendingEvent` (exception type, `DeliverErrorCode`).

### D.4 Register batching, register count, state APIs
No documented maximum for `RegisterCount` (docs list only the parameters). The x64 `WHV_REGISTER_NAME` enum in this header has 255 names / 254 distinct values (`WHvX64RegisterDeliverabilityNotifications` and `WHvRegisterDeliverabilityNotifications` are both `0x80000004`). Every value is a 16-byte `WHV_REGISTER_VALUE` union. QEMU's runtime sync array `whpx_register_names[]` carries 38 names in one call; OpenVMM batches via ArrayVec. Threading constraints are undocumented; `WHV_E_INVALID_VP_STATE` on the APIC-state getter shows the hypervisor rejects some accesses to a VP "in its current state"; both VMMs touch VP registers only from the VP's own thread or after a cancel. `WHvGetVirtualProcessorXsaveState`/`Set...` (1809, deprecated) → `WHvGetVirtualProcessorState(..., WHvVirtualProcessorStateTypeXsaveState = 0x1001, ...)`; size discovery: "call the function with a buffer that is too small and read the value returned in BytesWritten when the function returns WHV_E_INSUFFICIENT_BUFFER". State types (x64): SynicMessagePage 0, SynicEventFlagPage 1, SynicTimerState 2, InterruptControllerState2 0x1000, XsaveState 0x1001, NestedState 0x1002 (`WHV_X64_NESTED_STATE` = 8192 bytes).

### D.5 Multi-VP and scheduling knobs
Each `WHvRunVirtualProcessor(Partition, VpIndex, ...)` is independent (one blocking thread per VP; `WHvCreateVirtualProcessor2` docs: "On x64, the index of the virtual processor is used to set the APIC ID"; `WHvVirtualProcessorPropertyCodeNumaNode`: "When this property is not supplied, the virtual processor is placed on the ideal NUMA node of the calling thread."). `WHvPartitionPropertyCodeProcessorCount = 0x00001fff` (`UINT32`, pre-setup; `WHV_CAPABILITY_FEATURES.VpHotAddRemove`). No host-CPU affinity API; the "integrated scheduler" runs the VP on the caller's thread. Scheduler knobs (header, gated by `WHvCapabilityCodeSchedulerFeatures`):
```c
typedef union WHV_SCHEDULER_FEATURES { struct { UINT64 CpuReserve: 1; UINT64 CpuCap: 1; UINT64 CpuWeight: 1; UINT64 CpuGroupId: 1; UINT64 DisableSmt: 1; UINT64 Reserved: 59; }; UINT64 AsUINT64; } WHV_SCHEDULER_FEATURES;
// properties: CpuReserve (UINT32) = 0x7, CpuCap (UINT32) = 0x8, CpuWeight (UINT32) = 0x9, CpuGroupId (UINT64) = 0xa, ProcessorFrequencyCap (UINT32) = 0xb, DisableSmt (BOOL) = 0xd, PrimaryNumaNode (USHORT) = 0x6, SeparateSecurityDomain (BOOL) = 0x3
typedef struct WHV_CAPABILITY_PROCESSOR_FREQUENCY_CAP { UINT32 IsSupported:1; UINT32 Reserved:31; UINT32 HighestFrequencyMhz; UINT32 NominalFrequencyMhz; UINT32 LowestFrequencyMhz; UINT32 FrequencyStepMhz; } WHV_CAPABILITY_PROCESSOR_FREQUENCY_CAP;
```
QEMU exposes `-accel whpx,ssd=off` (SeparateSecurityDomain off) "for performance". IPIs between VPs are hypervisor-internal in APIC mode (`SentIpiCount`); in None mode the VMM must route them itself.

## E. Memory

Header:
```c
typedef enum WHV_MAP_GPA_RANGE_FLAGS
{
    WHvMapGpaRangeFlagNone             = 0x00000000,
    WHvMapGpaRangeFlagRead             = 0x00000001,
    WHvMapGpaRangeFlagWrite            = 0x00000002,
    WHvMapGpaRangeFlagExecute          = 0x00000004,
    WHvMapGpaRangeFlagTrackDirtyPages  = 0x00000008,
} WHV_MAP_GPA_RANGE_FLAGS;
typedef enum WHV_ADVISE_GPA_RANGE_CODE { WHvAdviseGpaRangeCodePopulate = 0, WHvAdviseGpaRangeCodePin = 1, WHvAdviseGpaRangeCodeUnpin = 2, } WHV_ADVISE_GPA_RANGE_CODE;
typedef union WHV_ADVISE_GPA_RANGE_POPULATE_FLAGS { UINT32 AsUINT32; struct { UINT32 Prefetch:1; UINT32 AvoidHardFaults:1; UINT32 Reserved:30; }; } WHV_ADVISE_GPA_RANGE_POPULATE_FLAGS;
typedef struct WHV_ADVISE_GPA_RANGE_POPULATE { WHV_ADVISE_GPA_RANGE_POPULATE_FLAGS Flags; WHV_MEMORY_ACCESS_TYPE AccessType; } WHV_ADVISE_GPA_RANGE_POPULATE;
typedef union WHV_MEMORY_ACCESS_INFO
{
    struct {
        UINT32 AccessType  : 2;  // WHV_MEMORY_ACCESS_TYPE (Read=0, Write=1, Execute=2)
        UINT32 GpaUnmapped : 1;
        UINT32 GvaValid    : 1;
        UINT32 Reserved    : 28;
    };
    UINT32 AsUINT32;
} WHV_MEMORY_ACCESS_INFO;
typedef struct WHV_MEMORY_ACCESS_CONTEXT
{
    // Context of the virtual processor
    UINT8 InstructionByteCount;
    UINT8 Reserved[3];
    UINT8 InstructionBytes[16];
    // Memory access info
    WHV_MEMORY_ACCESS_INFO AccessInfo;
    WHV_GUEST_PHYSICAL_ADDRESS Gpa;
    WHV_GUEST_VIRTUAL_ADDRESS Gva;
} WHV_MEMORY_ACCESS_CONTEXT;     // 40 bytes
typedef struct WHV_DOORBELL_MATCH_DATA
{
    WHV_GUEST_PHYSICAL_ADDRESS GuestAddress;
    UINT64 Value;
    UINT32 Length;
    UINT32 MatchOnValue:1;
    UINT32 MatchOnLength:1;
    UINT32 Reserved:30;
} WHV_DOORBELL_MATCH_DATA;
typedef struct WHV_PARTITION_MEMORY_COUNTERS { UINT64 Mapped4KPageCount; UINT64 Mapped2MPageCount; UINT64 Mapped1GPageCount; } WHV_PARTITION_MEMORY_COUNTERS;
#define WHV_READ_WRITE_GPA_RANGE_MAX_SIZE 16
```
Mapping: `WHvMapGpaRange(Partition, SourceAddress, GuestAddress, SizeInBytes, Flags)` — "page-aligned address of the memory region in the caller's process"; "The operation replaces any previous mappings for the specified GPA pages." `WHvMapGpaRange2` (20H2) adds a `Process` handle ("PROCESS_VM_READ, PROCESS_VM_WRITE, and PROCESS_VM_OPERATION") and documents `E_INVALIDARG` for non-page-aligned source/GPA, zero size, overflow, or a flag "combination of access permissions that cannot be applied to the page". `WHvUnmapGpaRange`: "Any further access by a virtual processor to the range will result in a memory access exit." `WHV_CAPABILITY_FEATURES.PartialUnmap` (bit 0) gates unmapping part of a mapped range. Large pages: only the counters above prove 2M/1G mappings exist; no flag selects them (presumably follows host large-page backing — unverified). Dirty tracking: map with `TrackDirtyPages`, then `WHvQueryGpaRangeDirtyBitmap` (1809): "one bit per each page, rounded up to an 8-byte value"; `BitmapSizeInBytes == 0` "indicates that the range's dirty state should be cleared without querying the current state"; `WHV_E_GPA_RANGE_NOT_FOUND` if not tracked. Advise (20H2): Populate "commits the backing memory for the specified ranges ahead of access by a virtual processor" (ranges need not be page-aligned); Pin/Unpin need page-aligned ranges.

Access-fault exits: `WHvRunVpExitReasonMemoryAccess = 1` covers unmapped GPAs (`GpaUnmapped=1`) and, when `ExtendedVmExits.GpaAccessFaultExit` is set, "second-level page faults" on mapped-but-protected pages (`GpaUnmapped=0`). OpenVMM sets it always on x64: "Request GPA access fault exits here because WHP tries to handle these for ROM regions, resulting in an extra syscall and C++ exception for each such exit. We know locally whether memory is supposed to be mapped writable, so we can avoid this." Docs: "A common use case for memory access exits is the emulation of MMIO device operations, where unmapped regions of the partition's GPA space are used for the MMIO space of an emulated device". `InstructionByteCount`/`InstructionBytes[16]` carry the faulting instruction for the VMM's emulator; operands are reached with `WHvTranslateGva` + `WHvReadGpaRange`/`WHvWriteGpaRange` (20H2, ≤16 bytes, "suitable for emulating instruction operands rather than bulk memory transfer", "If the targeted page is not yet resident, the function makes the page resident and retries the access"). QEMU's emulator limitation: "does not support guests that use MMX, SSE or AVX instructions for access to MMIO memory ranges." Both VMMs' exit rate on MMIO is one exit per access; there is no batching.

Coalesced MMIO: none. The only exit-less guest-write facility is the doorbell (`WHvRegisterPartitionDoorbellEvent`, 2004, deprecated in favour of `WHvCreateNotificationPort(WHvNotificationPortTypeDoorbell)`, 20H2): "When a virtual processor performs a matching write, the event is signaled and the virtual processor does not exit with WHvRunVpExitReasonMemoryAccess. The hypervisor only recognizes certain store instructions for the triggering write; if the guest performs the write using an unrecognized instruction, the event is not signaled and the virtual processor exits with WHvRunVpExitReasonMemoryAccess." "If the matching guest address falls within a page previously mapped with WHvMapGpaRange, registering the doorbell effectively unmaps that page for the lifetime of the registration. Non-matching writes to the page cause the virtual processor to exit with WHvRunVpExitReasonMemoryAccess as though the page were unmapped." "The registered write must not straddle a page boundary; there are no other alignment requirements." Match modes: any write (`Length`/`Value` zero), size only (1/2/4/8), value+size; `MatchOnValue` requires `MatchOnLength`; multiple doorbells per address only when each matches a distinct `Value` of the same `Length`. It carries no data — a pure "kick" (virtio-style notify), not a coalesced ring. Confidence: high.

## F. Version gating (from each function's docs "Minimum supported Windows"; host is Windows 11 26200 with SDK 26100 — everything below is available on the host)

| API | Min. Windows (x64) |
|---|---|
| WHvGetCapability, WHvCreatePartition, WHvSetupPartition, WHvDeletePartition, WHvGet/SetPartitionProperty, WHvMapGpaRange, WHvUnmapGpaRange, WHvTranslateGva, WHvCreate/Delete/RunVirtualProcessor, WHvCancelRunVirtualProcessor, WHvGet/SetVirtualProcessorRegisters | Windows 10 1803 |
| WHvRequestInterrupt, WHvGet/SetVirtualProcessorInterruptControllerState (deprecated), WHvGet/SetVirtualProcessorXsaveState (deprecated), WHvQueryGpaRangeDirtyBitmap, WHvGetPartitionCounters, WHvGetVirtualProcessorCounters | Windows 10 1809 |
| WHvSuspendPartitionTime, WHvResumePartitionTime | Windows 10 1903 |
| ExtendedVmExits / ExceptionExitBitmap / X64MsrExitBitmap / CpuidExitList settable after WHvSetupPartition | "Insider Preview Builds (19H2)" |
| WHvGet/SetVirtualProcessorInterruptControllerState2 (deprecated), WHvRegister/UnregisterPartitionDoorbellEvent (deprecated) | Windows 10 2004 |
| WHvGet/SetVirtualProcessorState, WHvCreateTrigger, WHvUpdateTriggerParameters, WHvDeleteTrigger, WHvCreateNotificationPort, WHvSetNotificationPortProperty, WHvDeleteNotificationPort, WHvSignalVirtualProcessorSynicEvent, WHvPostVirtualProcessorSynicMessage, WHvMapGpaRange2, WHvAdviseGpaRange, WHvRead/WriteGpaRange, WHvCreateVirtualProcessor2, WHvGetVirtualProcessorCpuidOutput, WHvGetInterruptTargetVpSet, WHvStart/Accept/Complete/CancelPartitionMigration | Windows 10 20H2 |
| WHvPartitionPropertyCodeSyntheticProcessorFeaturesBanks, WHvCapability/PartitionPropertyCodeProcessorPerfmonFeatures | Windows Server 2022 / build 20348 (QEMU source comment; not in MS docs) |
| WHvResetPartition | Windows 11 21H2 |
| Arm64 support for the whole API | Windows 11 24H2 build 26100.3915 |
| WHvPartitionPropertyCodeMsrActionList / UnimplementedMsrAction, WHvPartitionPropertyCodeCpuidResultList2, WHvProcessorCounterSetSyntheticFeatures, WHvRegisterPendingEvent2/3, VTL types | in the 26100 header, no docs page found — version unknown |

`WinHvPlatform.h` marks with `#pragma deprecated` (NTDDI ≥ VB/FE): the two InterruptControllerState functions, the two InterruptControllerState2 functions, the two XsaveState functions, and the two doorbell functions. Absent from this header but referenced by OpenVMM's `whp` crate (newer SDK): a `WHV_SYNTHETIC_PROCESSOR_FEATURES::InterceptSystemReset` bit (aarch64) and the VTL/VSM entry points (`WHvQueryVtlProtectionMaskRange`, `WHvSetVtlProtectionMaskRange`, enable/disable VTL) whose data types (`WHV_VTL_PERMISSION_SET`, `WHV_ENABLE_PARTITION_VTL_FLAGS`, `WHV_INITIAL_VP_CONTEXT`, `WHV_DISABLE_VP_VTL_FLAGS`) are in the 26100 Defs header while the function prototypes are not. The windows-rs metadata listing (current) shows the same partition-property, capability, exit-reason, interrupt-type, trigger-type and state-type constant sets as this header, so no newer x64 knobs are published there either. QEMU's WHPX docs put the accelerator's minimum at "Windows 10 version 2004" for x64.

## G. Everything else a fast VMM wants

Triggers (20H2) — the asynchronous interrupt primitive:
```c
typedef enum WHV_TRIGGER_TYPE
{
    WHvTriggerTypeInterrupt = 0,        // x64 only
    WHvTriggerTypeSynicEvent = 1,
    WHvTriggerTypeDeviceInterrupt = 2,
} WHV_TRIGGER_TYPE;
typedef struct WHV_TRIGGER_PARAMETERS
{
    WHV_TRIGGER_TYPE TriggerType;
    UINT32 Reserved;
    union
    {
        WHV_INTERRUPT_CONTROL Interrupt;
        WHV_SYNIC_EVENT_PARAMETERS SynicEvent;      // { UINT32 VpIndex; UINT8 TargetSint; WHV_VTL TargetVtl; UINT16 FlagNumber; }
        struct { UINT64 LogicalDeviceId; UINT64 MsiAddress; UINT32 MsiData; UINT32 Reserved; } DeviceInterrupt;
    };
} WHV_TRIGGER_PARAMETERS;   // 32 bytes
HRESULT WHvCreateTrigger(Partition, const WHV_TRIGGER_PARAMETERS* Parameters, WHV_TRIGGER_HANDLE* TriggerHandle, HANDLE* EventHandle);
```
Docs: "The caller activates the trigger by signaling this event — for example, with SetEvent — which causes the configured action in Parameters to be delivered to the partition." "the host signals the returned EventHandle — for example, from an I/O completion routine or another thread — to deliver the configured interrupt or event to the partition without making a separate API call or hypercall on each delivery." "WHvTriggerTypeInterrupt (x64 only) injects a virtual interrupt described by the WHV_INTERRUPT_CONTROL structure — the same effect as WHvRequestInterrupt, but pre-armed so that it can be re-delivered each time the event is signaled." "Delivery is asynchronous. Multiple signals that arrive before a delivery completes may coalesce into a single delivery." `WHvUpdateTriggerParameters`: "The update is applied atomically with respect to delivery, so a signal that races with the update observes either the old or the new parameters." "Triggers are not preserved across live migration." The event handle and the trigger have independent lifetimes. `TargetVtl` for SynicEvent "must be VTL 0". This requires the hypervisor APIC (the `Interrupt` action is `WHvRequestInterrupt`).

Notification ports (20H2): `WHvNotificationPortTypeEvent = 2` ("signals EventHandle when the guest invokes the HvCallSignalEvent hypercall with a connection ID equal to the Event.ConnectionId member") and `WHvNotificationPortTypeDoorbell = 4` (E). Properties: `WHvNotificationPortPropertyPreferredTargetVp = 1` (default `WHV_ANY_VP = 0xFFFFFFFF`) and `WHvNotificationPortPropertyPreferredTargetDuration = 5` ("duration, in 100-nanosecond units, for which the preferred target virtual processor remains the affinity target", default `WHV_NOTIFICATION_PORT_PREFERRED_DURATION_MAX`). `ConnectionVtl` "Must be zero". Create only after `WHvSetupPartition`. OpenVMM builds VMBus host event ports on these (`create_notification_port(Event { connection_id }, event)`), with a fallback: "notification ports are not supported; TODO-remove once old Iron builds age out."

SynIC delivery from the host: `WHvSignalVirtualProcessorSynicEvent(Partition, WHV_SYNIC_EVENT_PARAMETERS, BOOL* NewlySignaled)` and `WHvPostVirtualProcessorSynicMessage(Partition, VpIndex, SintIndex, Message, MessageSizeInBytes ≤ WHV_SYNIC_MESSAGE_SIZE 256)` (C.2). State pages: `WHvVirtualProcessorStateTypeSynicMessagePage/EventFlagPage/TimerState`.

Counters: `WHvGetPartitionCounters(WHvPartitionCounterSetMemory)`; `WHvGetVirtualProcessorCounters` sets `Runtime = 0` (`TotalRuntime100ns`, `HypervisorRuntime100ns`), `Intercepts = 1` (`WHV_PROCESSOR_INTERCEPT_COUNTER { UINT64 Count; UINT64 Time100ns; }` × PageInvalidations, ControlRegisterAccesses, IoInstructions, HaltInstructions, CpuidInstructions, MsrAccesses, OtherIntercepts, PendingInterrupts, EmulatedInstructions, DebugRegisterAccesses, PageFaultIntercepts, NestedPageFaultIntercepts, Hypercalls, RdpmcInstructions — 224 bytes), `Events = 2` (`PageFaultCount, ExceptionCount, InterruptCount`), `Apic = 3` (A.1), `SyntheticFeatures = 4` (C.2).

CPUID control without exits: `WHvPartitionPropertyCodeCpuidResultList = 0x1004` (`WHV_X64_CPUID_RESULT { UINT32 Function; UINT32 Reserved[3]; UINT32 Eax, Ebx, Ecx, Edx; }`, whole leaf, all subleaves) and `CpuidResultList2 = 0x100D`:
```c
typedef enum WHV_X64_CPUID_RESULT2_FLAGS { WHvX64CpuidResult2FlagSubleafSpecific = 0x1, WHvX64CpuidResult2FlagVpSpecific = 0x2, } WHV_X64_CPUID_RESULT2_FLAGS;
typedef struct WHV_X64_CPUID_RESULT2 { UINT32 Function; UINT32 Index; UINT32 VpIndex; WHV_X64_CPUID_RESULT2_FLAGS Flags; WHV_CPUID_OUTPUT Output; WHV_CPUID_OUTPUT Mask; } WHV_X64_CPUID_RESULT2;  // 48 bytes
```
plus `CpuidExitList = 0x1003` (per-leaf `WHvRunVpExitReasonX64Cpuid` with `WHV_X64_CPUID_ACCESS_CONTEXT { Rax, Rcx, Rdx, Rbx, DefaultResultRax, DefaultResultRcx, DefaultResultRdx, DefaultResultRbx }` — the hypervisor's own answer is handed in, so the VMM only patches). `WHvGetVirtualProcessorCpuidOutput(Partition, VpIndex, Eax, Ecx, WHV_CPUID_OUTPUT*)`: "reflects the virtual processor's current extended-state configuration and any CPUID result overrides registered for the partition through WHvPartitionPropertyCodeCpuidResultList, so it represents the value the guest would actually see". `WHvGetPartitionProperty` cannot read back `CpuidExitList`/`CpuidResultList`. Processor feature banks: `WHvPartitionPropertyCodeProcessorFeaturesBanks = 0x100A` (2 banks, `WHV_PROCESSOR_FEATURES_BANKS_COUNT 2`), `ProcessorXsaveFeatures = 0x1006`, `ProcessorPerfmonFeatures = 0x100E` (`PmuSupport`, `LbrSupport`), `PhysicalAddressWidth = 0x1011`, `ProcessorClFlushSize = 0x1002`. QEMU 2026 "enable all supported host features": reads `WHvCapabilityCodeProcessorFeaturesBanks`, sets `NestedVirtualization` when `Bank1.NestedVirtSupport`, then sets `ProcessorFeaturesBanks` — "to follow the MSHV example".

Exceptions: `ExtendedVmExits.ExceptionExit` + `WHvPartitionPropertyCodeExceptionExitBitmap = 0x2` (`UINT64`, bit per `WHV_EXCEPTION_TYPE` vector) → `WHvRunVpExitReasonException` with `WHV_VP_EXCEPTION_CONTEXT { InstructionByteCount; InstructionBytes[16]; ExceptionInfo{ErrorCodeValid,SoftwareException}; ExceptionType; ErrorCode; ExceptionParameter; }`. OpenVMM traps #GP when it emulates the APIC itself: "Enable #GP faults to get synic MSR accesses, for which which the hypervisor incorrectly fails to exit to the parent."

Nested virtualization: `WHvPartitionPropertyCodeNestedVirtualization = 0x4` (BOOL), capabilities `WHvCapabilityCodeVmxBasic..VmxTrueEntryCtls = 0x2000..0x2010`, VMX/SVM capability MSRs as registers (`0x20A1..0x20B4`), `WHvVirtualProcessorStateTypeNestedState`, `WHvX64RegisterNestedGuestState/NestedCurrentVmGpa/NestedVmxInvEpt/NestedVmxInvVpid`, `PendingEvent2/3` for nested exits, `WHvX64PendingEventSvmNestedExit/VmxNestedExit`. OpenVMM: "synic ports are not supported with nested virtualization on Windows" and a workaround masking the flush-GPA enlightenment bits because "the hypercall is actually rejected at dispatch for exo (WHP) partitions".

Migration (20H2): `WHvStartPartitionMigration` → `HANDLE` → other process `WHvAcceptPartitionMigration` → source `WHvCompletePartitionMigration` → destination `WHvSetupPartition`; the destination partition "is created in a migrating state in which only WHvSetupPartition and WHvDeletePartition are permitted until the migration completes". Also: `WHvGetInterruptTargetVpSet` (resolves an APIC destination to VP indices, needs APIC emulation; `VpCount` must be ≥ processor count), VPCI device assignment (`WHvAllocateVpciResource` … `WHvRequestVpciDeviceInterrupt`, `WHvCreateVpciDeviceFlagUseLogicalInterrupts`), `WHV_CAPABILITY_FEATURES` bits `VirtualPciDeviceSupport`, `IommuSupport`, `DeviceAccessTracking`, `SpeculationControl`, `DirtyPageTracking`, `Xsave`.

## Design implications

1. Choose `WHvX64LocalApicEmulationModeX2Apic` (fall back to `XApic` if `WHV_CAPABILITY_FEATURES.LocalApicEmulation` is clear). It moves the APIC page, all APIC registers, the APIC timer (incl. TSC-deadline), IPIs, EOI and TPR into the hypervisor; the VMM's shadow APIC becomes a save/restore mirror (4096-byte external state page) and nothing more.
2. With the hypervisor APIC, device interrupts are `WHvRequestInterrupt(WHV_INTERRUPT_CONTROL)` from any host thread — no VP exit, no slice boundary. Level-triggered IOAPIC lines request `TriggerModeLevel` and are re-asserted from the `X64ApicEoi` exit (vector in `ApicEoi.InterruptVector`); edge lines never produce an EOI exit.
3. A halted VP in APIC mode sleeps inside `WHvRunVirtualProcessor` (`HaltSuspend`). It wakes on anything the hypervisor APIC delivers (RequestInterrupt, trigger, APIC/synthetic timer, IPI) or on `WHvCancelRunVirtualProcessor`. A VMM-written `PendingInterruption`/`PendingEvent` does not wake it: clear `HaltSuspend` in `WHvRegisterInternalActivityState` in the same `WHvSetVirtualProcessorRegisters` batch (OpenVMM writes `InternalActivityState = 0` with the event; QEMU clears only the bit). Never park the VP thread on halt in this mode (OpenVMM's readiness predicate ignores `halted` when the APIC is offloaded).
4. The 8259 path is the expensive one: `WHvRequestInterrupt` has no ExtINT/LINT0 type. PIC delivery = cancel the VP (if it is in Run) → `WHvRegisterPendingEvent` with `WHV_X64_PENDING_EXT_INT_EVENT{EventType=5, Vector}` → clear `HaltSuspend` → Run; gate on `DeliverabilityNotifications.InterruptNotification=1` and the `X64InterruptWindow` exit (`DeliverableType == WHvX64PendingInterrupt`) when `InterruptShadow`/IF/pending slot say "not ready" (QEMU and OpenVMM implement the identical protocol). The NMI pin is fine: `WHvX64InterruptTypeLocalInt1`, Physical, Edge, `Destination = vp index`. Real-mode BIOS/DOS boot therefore still costs one cancel+re-run per PIC interrupt; protected-mode OSes that switch to the IOAPIC/x2APIC cost nothing.
5. Device timers should be driven on a host thread from host time and fire through triggers (`WHvCreateTrigger(WHvTriggerTypeInterrupt)` + `SetEvent`) or `WHvRequestInterrupt`; slice boundaries bounded by the next device deadline are unnecessary. The relation to guest time is fixed: guest TSC runs at `ProcessorClockFrequency` Hz (host TSC rate, no scaling knob), offset fixed once; partition `ReferenceTime` (100 ns, monotonic across VPs, frozen only by Suspend/Reset) is the same counter an enlightened guest reads. Never write `WHvX64RegisterTsc` per slice — only on reset/restore, with `WHvSuspendPartitionTime` around the per-VP writes (all VPs stopped; time resumes on the next Run).
6. The guest's own clocks and timers can be made exit-free by setting `WHvPartitionPropertyCodeSyntheticProcessorFeaturesBanks` before setup (OpenVMM's VTL0 set is the reference list; needs Windows Server 2022+/Windows 11 host; requires the hypervisor APIC): TIME_REF_COUNT, reference-TSC page, STIMER0–3 (direct mode), VP assist page/APIC-assist EOI, TLB-flush and cluster-IPI hypercalls are then implemented by the hypervisor. The VMM must answer `WHvRunVpExitReasonHypercall` for HvPostMessage/HvSignalEvent/HvRetargetDeviceInterrupt (and the VTL/VSM set if used) and `SynicSintDeliverable` only for messages it posts itself. Without `HypervisorPresent`, WHP does not set CPUID.1:ECX[31]; the VMM decides the identity via a leaf-1 and 0x40000000 CPUID exit (QEMU) or `CpuidResultList2`.
7. Prefer `CpuidResultList2` (static, masked, subleaf/VP-specific) over `CpuidExitList`; the CPUID exit context already carries the hypervisor's default answer so a patch is cheap when an exit is unavoidable. Use `WHvGetVirtualProcessorCpuidOutput` to audit what the guest sees.
8. MSRs: keep `X64MsrExit` on with a minimal `X64MsrExitBitmap` (`UnhandledMsrs`, `ApicBaseMsrWrite`); use `MsrActionList`/`UnimplementedMsrAction = IgnoreWriteReadZero` to make unknown MSRs exit-free where the guest tolerates it. Also trap APIC-base writes even in APIC mode: the hypervisor reports CPUID[1:EDX].APIC unconditionally.
9. Cancel is the only cross-thread way to stop a VP; treat a pending cancel as sticky per-VP state (reset clears "pending suspend, cancel, and dispatch-notification state") and keep a VMM-side exit-request flag consumed before each Run (QEMU pattern) so a cancel issued between Runs is not lost and a stale one is recognised.
10. All register/state access happens on the VP's thread or after a cancel has returned; `WHV_E_INVALID_VP_STATE` exists for accesses "in its current state". Cross-thread producers (devices) set atomics/wake the VP thread (OpenVMM `extint_pending` + wake) — they never write VP registers.
11. Every exit costs a user/kernel round trip; there is no batching of MMIO (no coalesced-MMIO ring). Reduce MMIO exits by mapping framebuffer-like regions as RAM with `TrackDirtyPages` and scanning `WHvQueryGpaRangeDirtyBitmap`, by using doorbell notification ports for pure-kick registers (recognised store instructions only; the doorbell page becomes unmapped for every other access), and by setting `GpaAccessFaultExit` so ROM/write-protect faults come straight to the VMM instead of being emulated inside WHP (OpenVMM's reason).
12. Multi-VP: one blocking thread per VP; APIC-mode IPIs never leave the hypervisor; VP index = APIC ID unless `WHvX64RegisterInitialApicId` is set; INIT/SIPI are hypervisor-internal in APIC mode (optionally observed via `X64ApicInitSipiExitTrap`), but in `None` mode the VMM implements them itself (write CS, clear `StartupSuspend`). `WHvGetInterruptTargetVpSet` resolves logical/physical destinations with the hypervisor's rules.
13. Save/restore of the offloaded APIC: `WHvGetVirtualProcessorState(InterruptControllerState2)` needs a 4096-byte buffer (first 1 KiB meaningful, xAPIC layout at offset<<4); set requires the exact size; synthetic timers via `SynicTimerState`; XSAVE via `XsaveState` with size discovery through `WHV_E_INSUFFICIENT_BUFFER`. Triggers are not migrated.
14. Instrument with `WHvGetVirtualProcessorCounters`: `Apic` (MmioAccessCount/EoiAccessCount/TprAccessCount/SentIpiCount/SelfIpiCount) and `SyntheticFeatures` prove the offload is being used; `Runtime.HypervisorRuntime100ns` vs `TotalRuntime100ns` shows hypervisor-side overhead; `Intercepts.*.Time100ns` shows where exits spend time.
15. Scheduler tuning is limited to `CpuReserve/CpuCap/CpuWeight/CpuGroupId/DisableSmt` (only if `WHvCapabilityCodeSchedulerFeatures` allows), `ProcessorFrequencyCap`, NUMA placement; host-thread affinity is the VMM's own business; `SeparateSecurityDomain=FALSE` is QEMU's documented perf knob.
16. The header is ahead of the docs: rely on `WinHvPlatformDefs.h` for exit reasons (SynicSintDeliverable, Hypercall, the three APIC traps), counters, MsrActionList, PendingEvent2/3; the docs' enum listings are stale.

## Contradictions found (docs vs header vs VMM code vs our probe notes)

1. Docs `WHvRunVirtualProcessor` page lists exit reasons only up to `X64Rdtsc` and a smaller `WHV_RUN_VP_EXIT_CONTEXT` union; the header adds `SynicSintDeliverable = 0xA`, `X64ApicSmiTrap = 0x1004`, `Hypercall = 0x1005`, `X64ApicInitSipiTrap = 0x1006`, `X64ApicWriteTrap = 0x1007` and their contexts (the exit-context data-types page does have them). Header wins.
2. Docs `WHvGetVirtualProcessorCounters` show 11 intercept counters and 4 counter sets; header has 14 counters (`NestedPageFaultIntercepts`, `Hypercalls`, `RdpmcInstructions`; `C_ASSERT == 224`) and `WHvProcessorCounterSetSyntheticFeatures = 4`. Header wins.
3. Overview text names `RunVpExitLegacyFpError` as an exit with unused buffer; no such value exists in the header.
4. Docs call the InterruptControllerState2 buffer "the standard external state format" and never define it; only QEMU (`fields[256]{data,padding[3]}`) and OpenVMM ("Only the first 1024 bytes are used but the API requires a full page, for some unknown reason.") give the layout. Code wins.
5. Docs "Partition Property Data Types": processor-related properties "can only be configured ... prior to calling WHvSetupPartition"; `WHvSetupPartition`/`WHvSetPartitionProperty` pages: since 19H2 `ExtendedVmExits`, `ExceptionExitBitmap`, `X64MsrExitBitmap`, `CpuidExitList` may be set afterwards. Both true; the general sentence is the stale one.
6. `WHV_INTERRUPT_TYPE` in header/docs omits Hyper-V's `Smi=2`, `RemoteRead=3`, `ExtInt=7`, `LocalInt0=8`; QEMU and OpenVMM nonetheless pass raw delivery-mode numbers ("Values correspond to delivery modes" / "same format as mshv interrupt type"). Whether values outside the enum are accepted is unverified.
7. QEMU whpx docs give the accelerator's minimum host as Windows 10 2004; MS docs give 1803 for the base API. The enlightenment/perfmon properties need build 20348 per a QEMU source comment that MS docs never state.
8. Our earlier probe note "HaltInstructions.Count is always ZERO": consistent with HLT being consumed inside the hypervisor in APIC mode, but the docs never say what that counter counts, and it should also be checked in `None` mode where `X64Halt` exits do occur. Unverified.
9. Our probe "under XApic a halted VP NEVER leaves the run call": corroborated by OpenVMM (offloaded APIC never treated as halted; always re-enters Run), the QEMU 2020 guard `if (cpu->halted && !whpx_apic_in_platform())`, and the QEMU 2025 comment about "having Hyper-V handle HLT". But QEMU and OpenVMM still dispatch an `X64Halt` exit in that mode, so "never" is not proven by docs — only "the VMM must not depend on it".
10. QEMU whpx docs: on Windows 10 "a legacy PIC interrupt injected does not wake the guest from an HLT when using the Hyper-V provided interrupt controller" (so the Hyper-V APIC is off by default there); on Windows 11 QEMU enables it and clears `HaltSuspend` manually. Host-version-dependent behaviour the MS docs do not mention.
11. QEMU 2026: "Hyper-V always has CPUID[1:EDX].APIC set, even when the APIC isn't enabled yet" — the hypervisor's CPUID does not track the APIC-enable state; nothing in the docs says so.
12. OpenVMM: "Enable #GP faults to get synic MSR accesses, for which which the hypervisor incorrectly fails to exit to the parent." — with the user-mode APIC and no synthetic features, guest writes to synic MSRs #GP inside the hypervisor instead of producing `X64MsrAccess` exits. Header/docs silent.
13. OpenVMM: the L0 hypervisor "advertises the enlightened guest-physical-address flush hypercall ... whenever a partition is nested-capable, but the hypercall is actually rejected at dispatch for exo (WHP) partitions". Header/docs silent.
14. `WHvX64RegisterDeliverabilityNotifications` and `WHvRegisterDeliverabilityNotifications` are two names for `0x80000004` (docs list both without saying so).
15. QEMU issue #2063 vs docs: WHP guests without the synthetic feature bank see almost empty 0x4000xxxx leaves and perform far worse than the same image under Hyper-V; the docs never connect `SyntheticProcessorFeaturesBanks` to guest performance.
16. OpenVMM's `whp` ABI has `InterceptSystemReset` and VTL entry points absent from SDK 26100's headers — a newer header exists than the one on this machine.

### Not verified (needs a probe)
- Whether `X64Halt` exits ever occur in XApic/X2Apic mode (see 9).
- Whether a `WHvCancelRunVirtualProcessor` issued while the VP is not in Run makes the next Run return `Canceled` immediately (inferred from Reset docs only).
- Maximum `RegisterCount` per get/set call.
- Whether `ProcessorClockFrequency` is writable (TSC scaling) — no VMM does it.
- Semantics of `WHvX64RegisterTscVirtualOffset` (guest TSC = host TSC + offset is an inference).
- Whether `CpuidResultList(2)` overrides a hypervisor-authored 0x4000xxxx leaf when `Hv1` is set.
- Whether `WHvRequestInterrupt` with `Type = 7` (ExtInt) is accepted.
- How 2 MiB/1 GiB GPA mappings are obtained (only the counters prove they exist).

