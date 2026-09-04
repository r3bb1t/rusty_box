# WHP engine: the VMM shape — design

Date: 2026-09-03. Branch: `wip/atom-execctx`. Status: approved section by section in the
brainstorm that produced it; 219 of its claims were then verified against their sources
(151 verdicts: 76 confirmed, 69 imprecise, 6 wrong — the findings are archived at
`docs/research/whp-2026-09-03/07-spec-review-findings.md`) and every upheld correction is
applied below; awaiting the user's review of this written form.

Research record: six read-only reports, archived at `docs/research/whp-2026-09-03/`
(`01-whp-platform.md` SDK header + docs, `02-qemu-whpx.md`, `03-vbox-nem-vmware.md`,
`04-openvmm-hyperlight-wtf.md`, `05-guest-timekeeping.md`, `06-our-engine-as-built.md`).
Every factual claim below that is not measured in this tree cites one of them as `[NN §x]`.
No build, test or guest run was performed while producing this design.

## 0. Summary

The WHP engine is slow because it keeps Bochs's lockstep loop: it asks the hypervisor to
run the guest only until the machine's next device timer, and that stretch is almost always
too short to be worth entering. Measured at campaign head: 3.8 % of wall time inside
`WHvRunVirtualProcessor`, 86 % of slices never entering the partition, and every read-back —
every errand finished or burst on the shadow, and every slice end whose hardware ran since the
last read-back (`engine.rs:1367-1370`) — costing a 52-register exchange plus an 872-byte XSAVE
read plus a linear walk of the 64 Ki-entry (1.5 MiB) decoded-trace cache [06 §1, §4]. Every
fast VMM on WHP runs the vCPU thread unbounded [02 §1, 03 Q1, 04 §6]. QEMU (with its default
in-hypervisor LAPIC), OpenVMM, crosvm and Hyperlight drive device timers from host time on
another thread and deliver IOAPIC/MSI-routed interrupts to a running vCPU through the
hypervisor LAPIC; only 8259/ExtINT interrupts still cost a cancel plus an interrupt-window exit
[02 §1, §2a; 04 §6]. VirtualBox keeps its own LAPIC and a host-time clock but runs its timer
queues on the vCPU thread between exits, kicked by a 10 ms watchdog, and injects in software
after an exit or cancel [03 Q1, Q3, Rule 2] — the counterexample this design avoids. The one
project that kept Bochs's device stepping on the vCPU thread under WHP (applepie — its wheel is
already driven from wall time at a fixed ips rate) cancels the vCPU a thousand times a second
and its author's verdict is "as if we wrote our own scheduler" [04 §4.3, §4.7, §7].

This design replaces the slice model with that VMM shape in a distinct **fast mode**, keeps
the interpreter's **precise mode** exactly as it is, and makes execution transferable between
the two at an instruction boundary. Devices, the timer wheel, the interrupt fabric and the
CPU model are shared and unchanged in behaviour; what changes is who drives them and how a
tick is earned.

## 1. Goal and gates (user decisions)

| Decision | Choice |
|---|---|
| Success bar | Within 2× of VMware Workstation measured on THIS host with the SAME Alpine and Windows 7 images (Alpine ISO → `login:`; W7 ISO → edition-selection wizard), plus ≥ 90 % of busy wall time inside `WHvRunVirtualProcessor`. |
| Live engine transfer | Ships in this plan, both directions, at an instruction boundary. |
| vCPU count | Structure for N (thread per vCPU, per-vCPU state, APIC-destination routing); gates run 1. |
| Hyper-V enlightenments | On by default in fast mode; a machine-profile knob hides them. |
| Enlightenments vs transfer | The precise mode gains a minimal Hyper-V (Hv#1) component in this plan so an enlightened guest transfers both ways; no profile split. |
| Approach | A, "full VMM shape" (below). B (threads but our own LAPIC, VirtualBox's way) and C (tune the slice engine, applepie's way) were rejected with the evidence in §2. |
| API | Breaking public-API changes are welcome; there are no external users. |

## 2. Root cause and why the alternatives lose

**Baseline at head (`7728086`), the numbers the plan measures from — not the earlier
ones.** Alpine on the WHP engine does NOT reach `login:` within 300 s (`alpine_probe`, one
run at head; the pre-injection binary at `daeaa75` — the last Stage 0 commit of the injection
campaign — reached it in 150/162/165 s on a no-quiet copy of the ISO, so the tree carries an
open regression as well as the structural ceiling). DLX boots in ~2.0 s on WHP against 10.6 s
interpreted. Windows 7 reached its edition-selection wizard 6–8 minutes after the boot logo in
the GUI on the pre-campaign tree (`a32567d`); it has never been timed at head. The interpreter
booted Alpine to `login:` in 69–71 s on 2026-08-29 (`alpine_bench`) and has not been
re-measured since; the 75 s figure sometimes quoted is `WHP_ALL_SHADOW=1` reaching OpenRC — a
different engine and a different milestone. Every stage gate below is stated against G0
(VMware) and against these, and the first stage must re-measure all of them with one harness
and one ISO before anything is built on them.

**Root cause.** Three couplings, each measured in this tree [06]:
1. The scheduler clamps every uniprocessor batch to the wheel's countdown
   (`scheduler.rs:204-208`), the WHP engine converts any budget below 3 µs of host time
   (default; `WHP_MIN_SLICE_US` overrides) to the shadow interpreter (`engine.rs:1201-1211`,
   floor at `engine.rs:2911-2919`), and the wheel does not move while a slice runs. The
   8042's continuous 150 µs timer — Bochs-faithful, registered at `keyboard.cc:123-125` — is
   the only always-on sub-millisecond deadline and alone caps every idle-guest slice at 150 µs
   of machine time: 45,000 ticks at the 300 M ips these measurements use = 4.7 µs of host
   time at the 32× fast-forward [06 §3].
2. Interrupts reach the guest only through the machine: an IOAPIC delivery queued mid-slice
   waits for a scheduler boundary; a LAPIC vector waits for the next slice head or exit tail
   [06 §6]. The partition has no LAPIC (`LocalApicMode::None`, `engine.rs:876`), so no
   window ever backs the LAPIC page and every LAPIC register access is a nested page fault
   finished on the shadow. One Alpine boot took 353,705 memory exits in total — every
   unmapped access (LAPIC page, VGA text aperture, IOAPIC and other MMIO); the tally has no
   per-address breakdown, so the LAPIC's share is unmeasured [06 §6]. (The plan's explicit
   `LOCAL_APIC_REGION` carve-out, `plan.rs:45,115`, is inert: no candidate window reaches
   0xFEE0_0000 — RAM stops at `BX_PCI_HOLE_START` = 0xC000_0000, resumes at 4 GiB, and the
   BIOS candidate starts at or above 0xFFC0_0000.)
3. Guest time is earned two ways into one counter — wall time inside the run × ips × 32 for
   hardware, one tick per instruction on a converted shadow slice, and nothing at all for
   shadow instructions retired as errands inside a hardware slice — while the guest's TSC is the host's, its
   TSC-deadline is armed as an absolute machine tick, and PIT/PM-timer reads on a raw port
   exit see the wheel frozen at slice start [06 §2(d)].

**B loses** because a VMM-owned LAPIC cannot be reached while the vCPU runs: every device
interrupt costs a cancel plus a software injection, every LAPIC access stays an exit, a
halted vCPU exits to us, and Hyper-V offers the enlightenments only with its own LAPIC
[02 §2b, §10; 01 C.3; 04 §1.3–§1.5, §5.2]. VirtualBox does exactly this and is usable, so B is
viable [03 Rule 6]; it loses on the grounds above, not on a name (VirtualBox's "Snail execution
mode" label covers its whole WHP backend and is attributed to WHv exit cost, not to LAPIC
ownership [03 Q6]). **C loses** because a forced exit per device deadline
and a full state exchange per slice remain, and interrupts still cannot reach a running vCPU
[04 §7].

## 3. Design

Terminology: **fast mode** = the WHP engine's resident driver described here; **precise
mode** = the interpreter's lockstep scheduler, unchanged; **shadow** = the `BxCpuC` the fast
mode keeps for the guest's processor, which is the same type and instance the precise mode
executes; **transfer** = moving execution between the two drivers.

### 3.1 Threads and ownership

Fast mode has three thread roles with fixed ownership:

- **vCPU thread (one per processor).** Owns the partition's VP. Sits in
  `WHvRunVirtualProcessor` and leaves only for guest exits or an explicit
  `WHvCancelRunVirtualProcessor`. Services exits. The only thread that reads or writes VP
  registers or VP state (`WHV_E_INVALID_VP_STATE` exists for accesses from elsewhere
  [01 D.4]) — a property of the type: the `Vcpu` handle that carries every VP-state verb is
  owned by the thread and is neither `Copy` nor `Sync`; other threads hold only the narrow
  `Canceller`, `InterruptRequester` and counters tokens. Checks an exit-request flag before every entry and, when it is set, cancels its
  own VP so the next run returns at once and a request raised between runs is never lost
  (QEMU's pattern, which relies on cancel stickiness [02 §1, 01 D.1]); treats a `Canceled`
  exit no outstanding request explains as stale and re-enters (Hyperlight's pattern
  [04 §2.5]). Cancel stickiness is undocumented and treated as sticky [01 D.1].
- **Device thread (one).** Owns time and the device models: the Bochs timer wheel
  (`BxPcSystemC`), every `TimedDevice`, the interrupt fabric's PIC and IOAPIC. Sleeps on a
  high-resolution host timer until the earliest wheel deadline, catches the wheel up to
  VmTime (§3.2), fires due timers, raises interrupts (§3.3).
- **Front-end thread.** Owns display refresh, input, pause/resume/stop. Never touches VP
  state; reaches devices only through the machine lock.

Devices are reached from two threads (port/MMIO exits on a vCPU thread; timers on the device
thread), so they sit behind **one machine lock taken per access** — one device port or MMIO
dispatch, one timer-fire batch, one fabric operation (raise/lower/INTA/EOI), one memory-map
change, one front-end input injection or display read — in the machine layer, not
in device code — the devices crate stays free of synchronisation, as rust-vmm's
`Mutex<T: MutDeviceMmio>`-implements-`DeviceMmio` composition (display-seam spec 2026-08-22)
and QEMU's per-access BQL acquire in `prepare_mmio_access` do [02 §1]. Cross-thread producers set atomics and wake; they never
write VP registers. Teardown orders the canceller before the partition: a cancel racing
`WHvDeletePartition` is a use-after-free [04 §2.5].

Precise mode is untouched: the machine runs on one thread in lockstep and holds no lock of
its own — the code that runs today; its only cross-thread state is the stop flag and, under
the egui GUI, the display mutex the sink locks per call, which is front-end state.

The WHP engine crate is std-only (it depends on `rusty_box` with `features = ["std"]` and
uses `std::thread`/`std::sync` unconditionally) and `rusty_box` never depends on it (REPLAN
decision 12 — "no frontend ever depends on a backend crate; selection happens in the
binary" — kept). A threaded design confined to the engine crate does not reach the no_std/UEFI builds.
What reaches the core crates is the seam: a machine that can be driven by a resident driver
instead of its own scheduler loop, and a device clock that can be host time.

### 3.2 Time

One physical clock, three views:

- **Guest TSC** = host TSC + a hypervisor-kept offset. Never written by us except at reset
  and restore/transfer, and then only with every VP stopped and partition time suspended
  (QEMU `whpx_set_tsc`) [01 B.1, 02 §5]. Runs at `ProcessorClockFrequency` Hz; there is no
  scaling knob [01 B.1]. `RDTSC` is never trapped.
- **VmTime** (OpenVMM's shape [04 §1.8]): while running, an offset from the host monotonic
  clock; while paused, a frozen value. Advances only while the guest may run. Realised as a
  `VmClockSource` whose `now()` answers the tree's `VmInstant` (ticks), earned from the host
  clock at `ips` while Running and frozen while Stopped — no new time type.
- **The tick.** Kept as the unit devices program and the snapshot serialises. In fast mode a
  tick is earned as VmTime at the machine's nominal rate (`ips`), not as a retired
  instruction. A deadline armed at tick N means tick N under either driver, so the v3
  per-timer wire record (`pc_system.rs:1463-1469`: flags, period, absolute `time_to_fire`,
  owner, id) is unchanged and the in-tree reading of REPLAN decision 1 (`engine.rs:2977-2981`:
  "a device deadline armed under one is the same deadline under the other") survives with a
  different way of earning it. The section also carries `ips` and its restore rejects a
  mismatch (`pc_system.rs:1500-1502`), so a fast-mode snapshot restores only into a machine
  configured with the identical `ips`. `ips` stops meaning
  speed and means only the tick's unit; the 32× `HARDWARE_SPEED` fast-forward is removed
  (honest time is what the gate measures against).

Mechanics:
- The device thread advances the wheel by `tickn(elapsed)` on each wake (`elapsed` in `u32`
  chunks, `pc_system.rs:692`). `tickn`'s outer loop runs one `countdown_event` per deadline
  spanned (`pc_system.rs:694-699`), and each event fires every distinct timer that is due —
  so several deadlines are caught up in one call with no per-tick loop and no periodic
  kicker. `countdown_event`'s inner `while time_to_fire <= ticks_total` loop
  (`pc_system.rs:744-746`) COALESCES, it does not replay: a continuous timer overshot by N
  periods is recorded as one fire. Catch-up therefore always goes through `tickn`, never a
  single `countdown_event`, so a periodic deadline is crossed one period at a time.
- Its wake is armed at the earliest wheel deadline and re-armed when a device arms an
  earlier one — including from a vCPU thread during exit handling (a notify, not a
  cancel). The alarm thread's measured two-phase wait (sleep the bulk, spin the last 2 ms)
  is the mechanism, with the spin margin revisited once deadlines are milliseconds rather
  than microseconds.
- A guest read of the PIT, PM timer or HPET on a vCPU thread computes the wheel's position
  from VmTime at that instant, so counters no longer freeze for a slice.
- **Pause** = stop VmTime and `WHvSuspendPartitionTime` together, with no VP running;
  **resume** = re-anchor VmTime and run (partition time resumes on the next run) [01 B.1].
  The guest never sees its TSC advance against its timers across a pause.
- The **LAPIC timer**, including TSC-deadline mode, is the hypervisor's and runs on the guest
  TSC (`WHvX64RegisterTscDeadline`, `TscDeadlineTmrSupport` [01 B.1]). This removes the
  second two-clock hazard (`apic.rs:2032-2055` arming an MSR value computed from the host
  TSC as an absolute machine tick [06 §2(b)]).

Why this is sufficient for the guests' own checks [05 §1, §4]: Linux cross-checks its
PIT-derived TSC calibration against HPET/PM inside a 10 % window, refines against HPET/PM
within 1 % a second later, and its watchdog demotes the TSC on > 0.4 % skew per half second
(500 ppm once both clocksources are calibrated); Windows 7 calibrates the TSC against the
platform timers and, on an MP guest, bugchecks (0x101) when a secondary processor misses its
clock tick. Every one of those is now one physical clock compared with itself. With the
Hyper-V platform detected (Hv#1 plus the HYPERCALL and VP_INDEX MSRs) Linux skips the IRQ0
probe (`no_timer_check = 1`); with the frequency MSRs also exposed it skips PIT/HPET/PM TSC
calibration and LAPIC calibration outright, and takes its clocksource from the reference TSC
page or a TSC it is told is invariant [05 §1(a), §1(e)].

Divergences to register in `docs/bochs-parity-divergences.md` (entries, not comments):
- **8042 serial delay as a one-shot.** Bochs registers a continuous 150 µs timer
  (`keyboard.cc:123-125`) whose handler, on every fire, first collects and clears any latched
  IRQ1/IRQ12 request and raises it, then returns when no transfer is pending
  (`keyboard.cc:1025-1049`); a byte moved to the output buffer on one fire has its IRQ raised
  on the next, one period later. On host time that is ~6,667 device-thread wakes a second,
  almost all with nothing to do. keyboard.rs arms nothing today — the single arm is the
  continuous registration at `emulator/timers.rs:214-221`, and five latch sites
  (`controller_enq`, `kbd_enq_imm`, the CCB write, and `periodic`'s own transfer) set an IRQ
  request without touching `activate_timer` at all. Fast mode therefore keys the one-shot on
  STATE, not on sites: after every 8042 port dispatch and after every fire, the 150 µs
  one-shot is armed while `timer_pending != 0 || irq1_requested || irq12_requested` holds and
  the timer is idle. Guest-visible timing of IRQ1/IRQ12 differs by less than one period.
  Precise mode keeps Bochs's continuous timer.
- **Tick rate is a unit, not a speed**, in fast mode. Divergence H2 (TSC is the host's)
  becomes the design rather than a divergence in fast mode; D1 (halted time advanced by the
  scheduler) and D3 (fast-REP burst stops at the next deadline) do not apply in fast mode.
- **The transfer-visible TSC rate change** (§3.5): not yet in the registry — H2 records only
  the steady-state host-rate TSC, and the registry's two uses of "fingerprint" (D2, D3)
  concern matching upstream Bochs. A new entry cites Bochs `cpu/proc_ctrl.cc get_TSC` as the
  symbol departed from and states what the guest observes across a transfer.
- **The EOI resample** (§3.3): a level-triggered IOAPIC entry re-serviced on the guest's EOI,
  which neither this port nor Bochs does today; provenance QEMU `ioapic_eoi_broadcast`.

### 3.3 Interrupts and the LAPIC

The partition is created with `LocalApicEmulationMode::X2Apic`, falling back to `XApic` where
`WHV_CAPABILITY_FEATURES.LocalApicEmulation` names only that (QEMU master uses X2Apic; our
probe used XApic) [01 A.1, 02 §2]. Hyper-V then owns the LAPIC page, every LAPIC register,
the timer, IPIs, EOI and TPR; the VMM sees only `X64ApicEoi` for level-triggered EOIs, the
optional LINT0/LINT1/SVR/LDR/DFR write traps, `Cr8` in every exit header, and APIC-base MSR
writes if trapped [01 A.1]. Consequences and rules:

- Setting the emulation mode is what makes the platform serve the LAPIC page and retires
  those exits; the now-dead `LOCAL_APIC_REGION` carve-out (`plan.rs:45,115`) is removed as
  cleanup, and removing it maps nothing new (§2, coupling 2).
- `X64MsrExitBitmap.ApicBaseMsrWrite` stays on: Hyper-V reports CPUID[1].EDX.APIC
  unconditionally, and the base write is how the guest relocates or disables the APIC
  [01 A.1].
- **Our LAPIC model (`cpu/apic.rs`) is unchanged as the precise mode's LAPIC and becomes a
  mirror in fast mode**: loaded from the 4 KiB `InterruptControllerState2` page at pause,
  snapshot and transfer-out; written back at resume and transfer-in. The page's layout is
  QEMU's `whpx_lapic_state`: one 32-bit register per 16-byte slot indexed by `offset >> 4`, so a
  register's byte offset in the page equals its xAPIC MMIO offset; first 1 KiB meaningful; QEMU and OpenVMM
  agree on it and both require the full 4096-byte buffer [01 A.3]. Hyperlight reports that
  individual APIC register writes through `WHvSetVirtualProcessorRegisters` fail with
  `ACCESS_DENIED` on its hosts while emulation is on and uses the page as its only write path
  [04 §2.2]; the SDK header advertises the named `WHvX64RegisterApic*` registers as well, so
  P2 also records whether a named APIC register write is refused on this host.
- **IOAPIC → LAPIC gains a backend.** `deliver_ioapic_to_lapics` (`scheduler.rs:714-773`),
  the one place an IOAPIC message is routed into a LAPIC [06 §6] — the IRR write itself is
  `deliver_lapic_bus_interrupt` → `lapic.deliver` (`scheduler.rs:574-597`), shared with the
  ICR-IPI path and kept — becomes in fast mode a `WHvRequestInterrupt{vector, destination,
  destination mode, delivery mode, trigger}` issued from whichever thread serviced the
  device, without touching the vCPU. The trigger comes from the redirection entry
  (`PendingIoApicDelivery.trigger_mode`), never from device identity. PIC, IOAPIC redirection
  state (`ioredtbl`/`intin`/`irr`), device `IrqSink` calls: unchanged.
- **Level-triggered entries** request `TriggerModeLevel`; the `X64ApicEoi` exit (vector in
  `ApicEoi.InterruptVector`) must make the IOAPIC re-service every level entry with that
  vector whose line is still asserted — re-issuing the request — which is QEMU's
  `ioapic_eoi_broadcast` [02 §2a]. This is NEW behaviour: `IrqFabric::receive_eoi` is a
  logging no-op in this port and in Bochs (`ioapic.rs:756-761`, `ioapic.cc receive_eoi`),
  remote-IRR is never set in either tree, and today a level line is re-serviced only by the
  next `service_ioapic` scan. It is declared with QEMU provenance (R7) and registered as a
  divergence (§3.2). Edge entries (`trigger_mode() == 0`, the default — in practice the ISA
  lines under normal programming) produce no EOI exit.
- **Legacy 8259 path** (BIOS, ISOLINUX, DOS, a kernel before it programs the IOAPIC).
  `WHV_INTERRUPT_TYPE` has no ExtINT or LINT0 [01 A.2]. The fabric marks an ExtINT pending
  for the boot processor and wakes its vCPU thread (a cancel only if it is inside the run).
  The vCPU thread decides from the **exit header alone** — IF, `InterruptShadow`,
  `InterruptionPending`, pending event — whether delivery is permitted. Permitted: it performs
  the counted INTA (`IrqFabric::acknowledge`, `iodev/irq.rs`) and writes `PendingEvent{ExtInt,
  vector}` together
  with `InternalActivityState.HaltSuspend = 0` in one register batch. Not permitted: it arms
  `DeliverabilityNotifications.InterruptNotification` and injects on the `X64InterruptWindow`
  exit. This is QEMU's protocol [02 §2a], crosvm's readiness test from the exit context
  [04 §5.1], and OpenVMM's window arming plus `PendingEvent{ExtInt}` with
  `InternalActivityState = 0` [04 §1.5] (OpenVMM itself fetches IF/shadow/pending with a
  batched register read). This week's injection machinery (`InjectState`, `stage_injection`,
  the header cache, the freshness contract) already implements it for every maskable external
  vector, LAPIC and 8259 alike, acknowledged LAPIC-first through `pop_deliverable_vector` —
  that code sheds its LAPIC half and keeps its 8259 half as the ExtINT path. The freshness
  contract stands: the irreversible acknowledge happens only on positive evidence.
- **LVT0 fidelity.** The platform is not known to honour the guest's LINT0 programming for
  VMM-injected ExtINT events — QEMU and OpenVMM inject without consulting it; unverified
  [05 §6, open points] — so the fabric tracks LVT0 through `X64ApicWriteLint0ExitTrap` and
  withholds ExtINT while LINT0 is masked or not ExtINT-mode; P5 also records whether a
  pending ExtINT is delivered while the guest's LVT0 is masked.
- **NMI** = `WHvRequestInterrupt(LocalInt1)`. **SMI** = `X64ApicSmiTrap` exit → the SMI
  delivered on the shadow by the interpreter (`emulate_one`, as the slice head does today)
  and the handler run to `RSM` by the existing drain (`run_the_shadow_out_of_smm`). **INIT/SIPI** are the hypervisor's (SMP later;
  `X64ApicInitSipiExitTrap` optional for observation).
- **Halt** is never handled by us in fast mode: a halted VP sleeps inside the run call in
  `HaltSuspend` and wakes on anything the hypervisor LAPIC delivers or on a cancel
  [01 A.5]. A VMM-written pending event does not wake it, hence the `HaltSuspend` clear
  above — the one behaviour Probe P1 (§3.7) must confirm, since the earlier probe found the
  register unwritable under `None` (`docs/whp-platform-probe-2026-08-27.md` finding 3, and
  only on a fallback branch the committed probe never took — P1 attempts the write
  unconditionally) [05 §6]. The machine's HLT fast-forward path (run.rs, interactive.rs),
  which serves both engines today, is unreachable in fast mode.
- **Enlightenments** (`SyntheticProcessorFeaturesBanks`, set before setup): OpenVMM's VTL0
  set — `HypervisorPresent, Hv1, AccessVpRunTimeReg, AccessPartitionReferenceCounter,
  AccessHypercallRegs, AccessVpIndex, AccessPartitionReferenceTsc, AccessSynicRegs,
  AccessSyntheticTimerRegs, FastHypercallOutput, ExtendedProcessorMasks, SyntheticClusterIpi,
  NotifyLongSpinWait, QueryNumaDistance, SignalEvents, RetargetDeviceInterrupt,
  TbFlushHypercalls, AccessGuestIdleReg, AccessFrequencyRegs,
  EnableExtendedGvaRangesForFlushVirtualAddressList, AccessIntrCtrlRegs, DirectSyntheticTimers`
  [01 C.3, 04 §1.2]. The hypervisor then implements the Hv#1 CPUID leaves, TIME_REF_COUNT,
  the reference TSC page, STIMER0–3, the SynIC registers, the VP assist page (lazy EOI), the
  frequency MSRs and the flush/IPI hypercalls with no exit to us [01 C.2]. We answer
  `WHvRunVpExitReasonHypercall` for the HvPostMessage / HvSignalEvent /
  HvRetargetDeviceInterrupt class (no VMBus: refuse with the TLFS status) and ignore stray
  synthetic-MSR writes / read 0 (Linux writes `VP_ASSIST_PAGE` even when not exposed; Linux
  and Windows both read `GUEST_IDLE`, Windows 11 25H2 even when not advertised) [05 §1(e)].
  The machine-profile knob hides the whole bank, in which case CPUID.1:ECX[31] is authored by
  us (WHP does not set it without `HypervisorPresent` [01 C.3]). Requires the hypervisor LAPIC
  [01 C.3] and a Windows Server 2022 / build 20348 or later host (a QEMU source comment, not
  MS docs) [01 F]; this host is build 26200 and passes the version gate — acceptance of the
  bank here is P6, not yet measured.

### 3.4 Exits: long runs on either side, never per-instruction ping-pong

The vCPU runs on hardware for as long as the guest lets it; exits become rare because the
LAPIC, its timer and the enlightened clocks are the hypervisor's and the framebuffer is
direct-mapped. The exit path is deliberately **not** where this design earns its speed —
the 3.8 % measurement says the exit path was the wrong target.

- **A trapped instruction is finished by the existing interpreter on the shadow**, exactly
  as `finish_the_instruction` does today. The WHP engine grows no decoder or emulator of its
  own (unlike QEMU's `target/i386/emulate` and OpenVMM's `x86emu`); WHP hands us instruction
  bytes and the interpreter already decodes them.
- **The exchange around it is narrowed by an externalised mask** (VirtualBox's `fExtrn`
  [03 Q2]): one bit per register group meaning "the live value is in the partition". Every
  exit begins by copying the header's free fields (CS, RIP, RFLAGS, interrupt shadow, CR8).
  Each exit class imports only the groups its emulation needs and exports exactly the groups
  it imported, in one batched read and one batched write. A plain port access imports
  nothing and writes RIP and RAX, as today. The XSAVE area crosses only when the decoded
  instruction touches vector state. QEMU measured 4× (2 → 8 → 2 minutes) on a guest boot on a
  Windows 10 host between a fuller per-exit exchange and the narrowed on-demand one — a
  recovery to its earlier baseline, not a gain over it [02 §4, §8].
- **The decoded-trace cache is invalidated by an epoch stamp** on entries (applepie's
  `hypervisor_context_switches` [04 §4.5]) before any multi-instruction interpreter run,
  replacing the 65,536-entry walk (`icache.rs:349-368`). Single-instruction finishing from
  exit-provided bytes does not consult the cache at all.
- **Exit clustering** (VirtualBox's EM exit history [03 Q7]) replaces the fixed rule of
  eight consecutive MMIO exits. A table keyed by (RIP, exit type) notices a hot site, probes
  it by interpreting a stretch on the shadow with a full import/export around it, and if the
  probe saved exits keeps handing that site to the interpreter for up to 8,192 instructions,
  stopping after 32 instructions without an exit; records that probe uselessly are demoted.
  This is where the planar VGA aperture, the BIOS and ISOLINUX phases and the Windows 7 boot
  logo get their speed — VirtualBox took a Windows 2000 boot-and-shutdown run from 32 min 12 s
  to 58.66 s on plain WHP with it, within 1 % of its native VMM [03 Q6]. The rule in both directions: hardware runs long, or the
  interpreter runs long.
- **Memory**: RAM mapped once read-write-execute; ROM read-execute; the linear framebuffer
  as a `DirectMapped` window mapped as RAM with `TrackDirtyPages`, its dirtiness delivered
  to each refresh as `Dirt::Pages` from `WHvQueryGpaRangeDirtyBitmap` (the display-seam
  spec; `Dirt::Pages` exists in `rusty_box_devices/src/display/card.rs`, the `DirectMapped`
  mapping itself is unbuilt — P9 per the devices-crate spec); only the planar VGA window,
  IOAPIC, HPET and PCI configuration stay trapped. Protection changes are
  unmap-then-remap-on-fault, never back-to-back [03 Q5]. SMM phases run on the shadow as
  today. A20 stays as divergence H1 records it — the hardware serves the unmasked address —
  because nothing on the WHP engine models A20 today; fast mode does not change that.
- **Unfinishable exits** (`UnsupportedFeature`, `InvalidVpRegisterValue`,
  `UnrecoverableException`): a bounded interpreter stretch and re-entry (the clustering path);
  a recurrence at the same RIP surfaces as an engine fault with the refused-state dump.

### 3.5 Transferring execution between engines

Both drivers share one machine: the shadow is the precise mode's `BxCpuC`; memory, devices,
wheel and fabric are one set of objects. A transfer changes who owns execution at an
instruction boundary, in either direction, and is built as the same operation a snapshot
restore performs.

**Fast → precise.** (1) Pause: set the exit-request flag and cancel; if the exit header shows
`InterruptionPending`, re-enter until it lands (the existing delivery-in-flight rule) so the
boundary is clean; stop the device thread's wake; freeze VmTime and suspend partition time
together. (2) Import the whole processor into the shadow: every register group, the XSAVE
area, `InterruptState`; the LAPIC by reading the state page into `cpu.lapic` through a new
re-arm entry that takes the page's current count and divide, sets
`ticks_initial = now − (timer_initial − ccr) × divide` so `get_current_timer_count` and the
snapshot's epoch check (`apic.rs:2620-2637`) stay coherent, and arms `now + ccr × divide`
(in TSC-deadline mode the count is 0 and the re-arm converts the deadline MSR, a guest-TSC
value, to a machine tick); the Hv#1 component's state (§3.6).
(3) Read `WHvX64RegisterTsc` and `set_tsc(tsc, ticks_now)` so RDTSC continues from the same
value. (4) The wheel is already in ticks and continues; PIC and IOAPIC never left our process;
a pending ExtINT is fabric state. (5) The scheduler runs lockstep on that processor.

**Precise → fast.** (1) Stop at a batch boundary. (2) Export the processor (the existing
`export_arch_state` + XSAVE + interrupt-shadow bit). (3) Build the LAPIC state page from
`cpu.lapic` (timer current count from the wheel deadline) and write it. (4) Write the TSC
under a partition-time suspend. (5) Anchor VmTime at the current tick; start the device
thread (earliest wheel deadline) and the vCPU thread(s).

Guest-visible across a transfer: the TSC rate changes between the host's rate and one per
instruction, and time per instruction changes. Both are inherent to a precise mode. They are
recorded in the project's memory notes as the fingerprint of switching but not yet in the
divergence registry; §3.2 lists the entry to add.

### 3.6 The precise mode's Hyper-V (Hv#1) component

A guest that has taken the enlightenments depends on the hypervisor keeping its reference
TSC page current and firing its synthetic timers; transferred to an interpreter without them,
its clock freezes and its tick stops. So the precise mode gains a minimal Hv#1 component,
active only when the machine profile exposes the enlightenments, implemented on the tick
clock exactly as Bochs would have implemented a device: CPUID leaves 0x40000000–0x40000006
(with the same feature/privilege bits the bank advertised, read back via
`WHvGetVirtualProcessorCpuidOutput` so both modes agree bit for bit); `HV_X64_MSR_GUEST_OS_ID`,
`HYPERCALL` (hypercall page; TLB-flush and cluster-IPI calls complete trivially on one vCPU,
correctly on N), `VP_INDEX`, `TIME_REF_COUNT` (ticks → 100 ns), `REFERENCE_TSC` (page kept
current with scale/offset derived from the tick clock, `TscSequence` bumped on every change),
`TSC_FREQUENCY`/`APIC_FREQUENCY`, `VP_ASSIST_PAGE` (lazy EOI bit honoured by the LAPIC
model's EOI path), `STIMER0–3_CONFIG/COUNT` (four wheel timers per vCPU, direct mode asserting
the configured vector into the LAPIC model; message mode posting to the SynIC message page
for older Windows), the SynIC registers (`SCONTROL`, `SVERSION`, `SIEFP`, `SIMP`, `EOM`,
`SINT0–15`), `GUEST_IDLE` (read = HLT ignoring IF, as QEMU handles it [02 §5]). Its state is
a snapshot section and crosses the transfer with the rest of the processor; on transfer-in
the partition's `SynicTimerState` and the reference-TSC registers are written from it, and on
transfer-out read from them [01 A.3, C.2]. Provenance under R7: declared as "Hyper-V TLFS",
not Bochs.

### 3.7 Public surface, probes, gates, failures, tests

**Public surface.** The machine stays one type built by `MachineBuilder::build_on::<E>()`
with the engine as its type parameter; the engine chooses the driver. `start`, `pause` and
`resume` are NEW verbs (today's host controls are `emu_start`/`emu_stop`, `StopHandle::stop`
and `set_stop_flag`), the same in both modes. `step(RunBudget::Ticks(n))` in fast mode =
resume, wait until VmTime has advanced n ticks, pause — so `alpine_probe`, which already
steps in ticks, keeps its shape; `step(RunBudget::Instructions(_))` stays refused, as it is
today for any engine whose progress unit is ticks (the two WASM loops and the UEFI loop step
in instructions and never host WHP). `Emulator::run_interactive` (`interactive.rs:39`), which
the GUI runner and seven `rusty_box/examples` binaries call, becomes `start` plus the
front-end thread's own work: input into devices under the machine lock, display refresh every
40 ms or sooner on a dirty text screen (today's `GUI_UPDATE_INTERVAL` rule) from the
framebuffer's dirty bitmap. Guest power-off arrives from the device thread as
`StopReason::GuestPowerOff`. `save_snapshot` = pause + the export of §3.5; `restore_snapshot`
= the import + resume; the v3 container (`RBXSNAP1`, version 8) unchanged. `SliceEngine`
remains the interpreter's contract; the fast engine implements the resident-driver contract
instead of `run_slice` — the exact trait shape is the plan's to choose, with the constraint
that the 21 `impl<…, E: SliceEngine<…>> Emulator<…, E>` blocks across 8 files [06 §5] collapse
onto the shared machine rather than being duplicated per driver.

**Probes first (hypervisor-present tests, results recorded in `docs/whp-platform-probe-*.md`):**
- P1 `InternalActivityState.HaltSuspend` is writable in X2Apic/XApic mode on this host, and
  clearing it makes a pending ExtINT event wake a halted VP.
- P2 `InterruptControllerState2` round-trips: page → `cpu/apic.rs` model → page, bit-identical
  for the registers the model owns; set requires exactly 4096 bytes.
- P3 Whether `X64Halt` exits ever occur in APIC mode (both reference VMMs keep a handler).
- P4 Cancel stickiness: a cancel issued while the VP is not in Run makes the next Run return
  `Canceled` immediately.
- P5 `X64ApicWriteLint0ExitTrap` is accepted and fires on the guest's LVT0 writes.
- P6 `SyntheticProcessorFeaturesBanks` with the §3.3 set is accepted before setup on this
  host and a guest reading CPUID 0x40000000–3 sees "Microsoft Hv"/"Hv#1" with the
  corresponding bits.
- P7 The hypervisor LAPIC exposes TSC-deadline (`TscDeadlineTmrSupport`) and its bus clock
  (`InterruptClockFrequency`) equals what the Hv#1 `APIC_FREQUENCY` MSR reports.
- P8 `WHvSuspendPartitionTime` freezes the guest TSC across a pause (read TSC before and
  after a suspended interval).

**Gates.**
- G0 VMware Workstation on this host, same ISOs, same memory: Alpine → `login:`; W7 →
  edition-selection wizard. Three runs each, medians recorded.
- G1 Mechanism: the vCPU thread's time inside `WHvRunVirtualProcessor`, measured as host
  `Instant` deltas around the call, ≥ 90 % of wall over the kernel phase (the `"Linux version"`
  milestone → `login:`); cross-checked against `WHvGetVirtualProcessorCounters`, where guest
  time is `TotalRuntime100ns − HypervisorRuntime100ns` (the hypervisor counter is its OVERHEAD
  share, and neither counter is wall time — a halted or descheduled VP accrues nothing);
  idle-at-login exits/s in the low thousands or fewer.
- G2 Alpine → `login:` ≤ 2 × G0. G3 W7 → wizard ≤ 2 × G0.
- G4 Live transfer: Alpine at `login:`, fast → precise → fast, guest keeps time (`date`
  advances at wall rate before and after; `dmesg` on the serial console shows no clocksource
  demotion), `uname -a` answers.
- G5 DLX boots on fast mode and on precise mode (regression control).
- G6 `cargo xtask ci` green including the doctrine ratchets; `Send` by derivation for
  everything that now crosses threads (no `unsafe impl Send`); the `unsafe` token baselines
  in `xtask/src/ci.rs` do not rise (`rusty_box_whp/src` 38 — the `sys/windows.rs` blocks plus
  the one `unsafe fn map_borrowed` signature — and `rusty_box_whp_engine/src` 1 — its single
  discharge in `map_window`).

All throughput claims come from the step-driven `alpine_probe`, interleaved A/B, normalised
by guest ticks, with every probe verified to fire before a zero is believed.

**Failure handling.** A platform refusal on a vCPU thread pauses the machine and surfaces as
an engine fault carrying the refused-state dump that exists today; the exit-history trail is
kept. Cancel/teardown ordering as in §3.1. An unfinishable exit as in §3.4. A panic on any
machine thread aborts the process: the workspace's release profile is `panic = "abort"` (root
`Cargo.toml`), and a half-serviced device is not a machine anyone should resume. Under the
unwind profile `cargo test` builds with, the poisoned machine lock parks the peer thread with an
engine fault instead of propagating the panic.

**Tests (R9: guest-visible properties).** Hypervisor-free: VmTime arithmetic
(Started/Stopped, tick conversion at every rate), wheel catch-up over several missed
deadlines, externalised-mask algebra (import ∩ externalised, export = imported), the
clustering table's state machine, LAPIC page ↔ model conversion (pure structs), the ExtINT
readiness predicate from header bits, the Hv#1 component's MSR/CPUID surface and stimer
firing on the tick clock. Hypervisor-present (ignored without WHP): P1–P8; a halted guest
woken by a device-thread `WHvRequestInterrupt`; level-triggered EOI resample; the ExtINT
window protocol against a guest that toggles IF; the transfer round trip on a small guest
that reads RDTSC and a PIT counter across it.

## 4. Decisions this design reverses, and why the justifications lapsed

| Decision | Recorded justification | Why it lapses |
|---|---|---|
| REPLAN v4 #6 — `LocalApicEmulationMode = None`, Alpine straight to `None` + our own `cpu/apic.rs` | "under `XApic` a halted processor never leaves `WHvRunVirtualProcessor`, which would strand the timer wheel on a thread that never returns" (the comment at `engine.rs:870-875`, above `.local_apic(LocalApicMode::None)`) | The wheel no longer lives on the vCPU thread (§3.1). |
| REPLAN v4 #9 — one thread, "exactly two cross-thread pokes", machine holds zero locks | "exactly today's runner topology; `&mut` is the exclusivity proof; `Partition` `Send + Sync` by derivation", amended 2026-08-27: "holds only because Alpine runs under `None`; under `XApic` a third poke and a device thread would be needed" | That threading model is the design; `&mut` exclusivity survives inside the machine lock, `Send` stays derived, devices stay lock-free. |
| REPLAN v4 #1 (part) — "rate == ips under WHP, host ns → ticks" | one tick clock on both engines, v3 format unchanged | Kept in unit and format; changed in how a tick is earned (VmTime, not run-time × 32). |
| Injection spec 2026-09-01, rejected approach #1 (partition-local APIC) | halted VP strands the wheel; "LAPIC state would live in two places, which engine switching and snapshots cannot tolerate" | Wheel on its own thread; the state page is the single source at every pause, snapshot and transfer (§3.3, §3.5). |
| Fast-path spec 2026-08-29 §6 — cross-engine restore "given up" | tick and host-time clocks differ | The tick is the shared unit; VmTime earns it (§3.2); transfer is the restore (§3.5). |

Kept: REPLAN #5 (the shadow `BxCpuC` is mandatory — a trapped memory exit reports
`InstructionLength = 0`, does not advance RIP, and for a read-only window carries no
instruction bytes: `docs/whp-platform-probe-2026-08-27.md` finding 2; the context struct is
in [01 E]); #12 ("no frontend ever depends on a backend crate"); #13;
the fast-path spec's decision 1 ("on the fast path guest time equals host time; device
timers move to a host clock, off the run thread") and 7 (no_std is interpreter-only).

## 5. Safety doctrine mapping

R0 named returns (the transfer result, the probe results, the census are structs, no
tuples). R1 `unsafe` stays where it is: the WinHvPlatform blocks in `sys/windows.rs`, the
`map_borrowed` signature, and its one discharge in the engine crate; the ratchet counts
`unsafe` tokens per crate (baselines 38 and 1), and file confinement inside `rusty_box_whp` is
the workspace `deny(unsafe_code)` lifted only by `windows.rs` and that signature — the engine
crate does not opt into the workspace lint table, so the plan adds `[lints] workspace = true`
there before it grows threads.
R2 states are types: `VmTime::{Started, Stopped}`, the driver ownership (`Resident`/`Lockstep`)
and the externalised mask are types, not flags. R3 the machine lock is assembled by the
machine; drivers receive guards, never loose parts. R4 units: the tree's `VmInstant`/`VmDuration`
(ticks, 1/ips), `HostInstant` (host nanoseconds) and `std::time::Instant` are distinct types with
named conversions; "VmTime" in this document is the `VmInstant` a Started/Stopped clock source
answers, not a fourth type. R5 one choke point per
hazard: the exit-request flag + cancel; the INTA; the pause/resume pair; the LAPIC page
import/export. R6 `Send` derived for every part that crosses threads; no `unsafe impl`.
R7 Bochs provenance for devices and the LAPIC model; "Hyper-V TLFS" provenance declared for
the Hv#1 component; the divergences listed in §3.2 (including the EOI resample of §3.3) registered. R8 no `dyn` in signatures; the
IOAPIC backend is a generic parameter. R9 tests assert guest-visible properties (§3.7).

## 6. Blast radius (measured, from [06 §5–§6 and the table])

| Component | Files | Sites (what was counted) |
|---|---|---|
| Timer-wheel API (`pc_system.` lines outside pc_system.rs and emulator/tests.rs; inline test bodies and ~50 comment lines included) | 26 | 175 lines — most unchanged (the wheel stays; its driver changes) |
| Timer arm/disarm (non-test call sites) | 11 | 33 — unchanged API; the 8042 one-shot rule lands on its continuous arming at `timers.rs:215` plus new state-keyed arming, not on any of these |
| `DeviceCtx` clock/timer consumers | 8 | 27 — unchanged (`VmClock` is the unit; its source changes) |
| `.lapic` outside apic.rs and emulator/tests.rs | 17 | 169 lines (57 in scheduler.rs, one under `#[cfg(test)]`) — the IOAPIC→LAPIC routing and the timer-request drain gain a backend; the model's readers are unchanged |
| `impl<…, E: SliceEngine<…>> Emulator<…, E>` | 8 | 21 blocks + the `Power` role handle + 2 trait impls + 1 test engine — collapse onto the shared machine |
| Front-end drivers | 3 loop shapes, ~20 call sites | `run_interactive` (GUI runner + 7 `rusty_box/examples`), `step` loops (2 WASM crates, the UEFI app, `snapshot_resume`, 5 WHP-engine examples incl. `alpine_probe`), `emu_start` (perfbench, shellcode_trace) |
| WHP engine crate | engine.rs 3180 lines + alarm.rs 230 | the slice loop, conversion, fast-forward and per-slice exchange (engine.rs) and the alarm thread (alarm.rs) are replaced; `state.rs`, `xsave.rs`, the injection protocol, the census and the refused-state diagnostics are kept |

## 7. Risks and open questions

- P1 fails (halt-suspend unwritable in APIC mode on this host): the legacy 8259 path then
  needs QEMU's workaround for Windows 10 hosts — user-mode LAPIC by default whenever the
  machine has a PIC — which is approach B for legacy phases only. Decide after the probe, not
  before.
- Whether `CpuidResultList2` can override a hypervisor-authored 0x4000xxxx leaf when the bank
  is on (needed only if the knob wants partial identity) [01 "Not verified"].
- Windows 7's consumption of the reference TSC page and synthetic timers as a guest is not
  established by the sources gathered; unless it adopts a synthetic-timer tick, its RTC 64 Hz
  tick costs 2 PIO per tick [05 §3, §5(c)].
- WHP exit throughput dropped ~70 % on one build between June and October 2018, which
  VirtualBox attributed to suspected security and/or microcode updates; on a later build it
  measured one speculation-mitigation bit alone at 2.4× [03 Q6]. QEMU's `ssd=off` bundle
  (`SeparateSecurityDomain = 0` plus clearing the IBRS/STIBP/IBPB/SSBD feature bits) is
  worth measuring, not assuming [02 §4].
- The 8042 one-shot divergence must be registered with its evidence before the code changes.
- Exit clustering interacts with the interrupt fabric: a long interpreter stretch must still
  deliver the fabric's interrupts to the shadow (the precise mode's delivery path) and
  reconcile with the hypervisor LAPIC on re-entry via the state page — the plan must name
  where that reconciliation happens (transfer-in rules of §3.5 apply per stretch).

## 8. Staging the plan should follow

The plan is one redesign but lands in gated stages, each leaving a bootable tree:

- **Stage 0 — probes and baseline.** P1–P8 as hypervisor-present tests with recorded
  answers; G0 (VMware on this host); the head baseline re-measured with `alpine_probe`.
  Nothing else is built until P1 has an answer, because it decides the legacy-interrupt path.
- **Stage 1 — the fast-mode core.** vCPU thread + device thread + machine lock; VmTime
  and the tick as a unit; hypervisor LAPIC with the IOAPIC backend and the ExtINT window
  protocol; the externalised mask; the 8042 one-shot divergence registered; the slice loop,
  alarm, conversion and fast-forward removed; the EOI resample built. Gate: DLX and Alpine boot
  on fast mode; Alpine faster than the interpreter re-measured at head by Stage 0 with the same
  harness and ISO; G1's 90 % during the kernel phase.
- **Stage 2 — long runs.** Exit clustering; enlightenments on with the knob; icache epoch.
  Gate: G1–G3 against G0.
- **Stage 3 — transfer.** Pause/resume/snapshot as the shared export/import; the LAPIC page
  round trip; the precise-mode Hv#1 component; transfer both ways. Gate: G4, G5.
- **Stage 4 — surface.** The engine-chooses-driver API, `run_interactive` on the front-end
  thread, `step(Ticks)` semantics, the doc and memory updates. Gate: G6 and the whole suite.

## 9. Out of scope

SMP guests (structure only); MSI-capable PCI devices; paravirtual devices (virtio/VMBus);
a coalesced-MMIO substitute (WHP has none — only doorbell kicks [01 E]); nested
virtualisation; migration; the RE platform's precise-mode features beyond the transfer
itself; the unrelated display-seam and devices-crate specs of 2026-08-22 (this design
consumes the display-seam spec's `Dirt::Pages` and `WindowAccess::DirectMapped` decisions;
the devices-crate spec only defers direct-mapped windows to P9).
