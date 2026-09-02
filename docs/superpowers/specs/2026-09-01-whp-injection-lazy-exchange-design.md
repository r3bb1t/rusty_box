# WHP engine: interrupt injection and lazy state exchange

**Date:** 2026-09-01 · **Status:** approved design, pre-plan · **Branch:** `wip/atom-execctx`

## Goal

VMware-league OS boot and install throughput on `--engine whp`. Two structural
costs go away: a deliverable external interrupt stops ending the hardware slice
and stops taking two full state exchanges through the shadow interpreter; and a
slice boundary that exists only to run device timers stops exchanging the
processor at all.

Measured baseline this design must beat:

- Alpine boot to `login:`: 575 s on WHP against 75 s on the all-shadow
  interpreter.
- Windows 7 boot to the installer UI: ~6–8 min; a full install is untried and
  projected to hours.
- The engine's own irq tracing reports a **158 µs median vector latency**
  (engine.rs:1607-1613's instrumentation) — every interrupt is a slice end plus
  a shadow `emulate_one` plus a full ~52-register + XSAVE exchange each way.
- DLX control: 2.0 s (the low-variance canary; must not regress).

## Rejected approaches

- **Partition-local APIC** (`LocalApicMode::XApic`): H0 measured that a halted
  processor under XApic never returns from the run call (probe finding 10),
  stranding the timer wheel; and LAPIC state would live in two places, which
  engine switching and snapshots cannot tolerate.
- **Trim taxes only** (flush precision, `CpuidResultList`, slimmer readbacks):
  keeps the per-interrupt slice end, so its ceiling is "a faster interpreter
  with commuting costs." Its independent pieces survive as Stage 0 here.
- **Boot on the interpreter, switch after:** rejected by the workload — the
  target is booting and installing OSes, which is paid at least once per image
  and would take hours interpreted.

## Provenance

- **QEMU `whpx-all.c` is the template.** Its userspace-irqchip path (the
  equivalent of our `LocalApicMode::None`) injects via
  `WHvRegisterPendingInterruption`, arms `DeliverabilityNotifications` for the
  interrupt window, refuses to stage over an in-flight delivery by caching the
  exit header's `InterruptionPending` bit, refreshes TPR from the header's
  `Cr8`, and syncs registers lazily behind a dirty flag. Every protocol below
  is that design adapted to this engine's seams.
- **Microsoft's documentation** scopes `WHvRequestInterrupt` to APIC
  emulation; with `LocalApicMode::None` the register path is the only path.
  `WHV_INTERRUPT_TYPE` carries no ExtINT (our own vcpu.rs:199-201 records it).
- **`PendingInterruption` over `PendingEvent`:** the pending flag of
  `PendingInterruption` is mirrored into every exit's `ExecutionState` bit 6 —
  the bit the existing cancel re-enter rule keys on. `PendingEvent`'s is not
  mirrored, so an in-flight one would be invisible to that rule.
- **The primitive is pre-proven on this host:** `rusty_box_whp`'s probe
  (`examples/whp_probe.rs:457-482`) already injected
  `PendingInterruption{Interrupt, 0x40}` successfully, and the wrapper verb
  exists (`Partition::inject`, partition.rs:783).
- **A provenance correction this design carries:** engine.rs:1501 attributes
  the cancel-with-pending re-enter rule to wtf; wtf has no such rule (its
  Canceled arm stops and restores). The true precedent is QEMU. Stage 0 fixes
  the citation (R7).

## Architecture overview

Three stages, strictly ordered. Stage 1 before Stage 2 because injection
removes the dominant *forced* exchanges, leaving Stage 2's savings cleanly
attributable — and Stage 1 is safe under today's eager exchange only because
of the existing re-enter rule, while Stage 2 built first would need to solve
in-flight-preservation before injection exists to define it.

Each stage lands only through its measurement gate (below), `cargo xtask ci`,
and the standing rule that the boot oracle's guest-visible behaviour is
unchanged unless the stage's purpose is to change it.

---

## Stage 0 — extraction, probes, counters (no behaviour change)

1. **Extract `pop_deliverable_vector`** from `handle_async_event`'s
   priority-5 chain (`cpu/event.rs:375-564`) as one shared body the
   interpreter also calls (R5 — one implementation, never a copy). The chain,
   preserved exactly: LAPIC first (`acknowledge_int` + `sync_lapic_events` +
   the posted-interrupt fold, vector>0 gate), then
   `irq.int_pin_asserted()` → `irq.acknowledge()` with the spurious vector
   delivered as a real INTA would deliver it, deasserted-pin
   `reconcile_deasserted_intr` + clear `PENDING_INTR`, and the wake to
   `Active`. Unit tests pin: LAPIC-before-PIC priority, the vector-0 gate,
   spurious vectors, deasserted-pin reconciliation.
2. **Add probes** beside `has_an_event_to_deliver` (arch_state.rs:551-571):
   `has_deliverable_ext_int` (unmasked `PENDING_INTR | PENDING_LAPIC_INTR |
   VMX_VIRTUAL_INTR` — exactly `DeliverableInterrupt::MASK`,
   arch_state.rs:61-63) and `has_non_ext_int_event` (everything else:
   NMI, SMI, INIT, shutdown).
3. **Add `set_lapic_tpr_from_cr8`** — guest `MOV CR8` never exits, so the
   LAPIC's TPR must be refreshable from the exit header before any delivery
   decision.
4. **Census additions:** injected-count per vector, windows-armed,
   window-exits, and the invariant counter pair injected-vs-acked
   (`IrqFabric::acknowledge_count` + histogram, irq.rs:238-245, is the
   existing half).
5. **Fix the wtf→QEMU provenance** at engine.rs:1501.
6. **Partition-setup audit:** set `ProcessorFeatures` appropriately (wtf
   issue #252 class: unset, a guest `wrmsr IA32_SPEC_CTRL` #GPs on hardware
   only) and verify snapshot-restore zeroes `PendingInterruption` /
   `PendingEvent` (wtf `LoadState` precedent).

**Gate:** identical `acknowledge_count` and vector histogram on the
fixed-budget DLX/Alpine boot oracle; interpreter delivery provably routed
through the extracted popper; ci green.

---

## Stage 1 — injection

New engine state on `Started`:

```rust
struct InjectState {
    in_flight: bool,      // last exit's ExecutionState bit 6
    if_flag: bool,        // last exit's RFLAGS bit 9
    cr8: u8,              // last exit header's Cr8 field
    window: Option<u8>,   // armed DeliverabilityNotifications priority
}
```

All four fields refresh from exit headers, never from register reads.

### A. The one choke point — `stage_injection` (R5)

Called from exactly two places: the slice head and the post-exit tail (the
window exit falls through to the tail). Nowhere else.

```
1. if inject.in_flight: return Nothing            // never stage over an in-flight delivery
2. cpu.set_lapic_tpr_from_cr8(inject.cr8)         // MOV CR8 never exits
3. if !cpu.has_deliverable_ext_int(): return Nothing
                                                  // the MASK subset ONLY; NMI/SMI/INIT
                                                  // must never reach this function
4. if started.shadowed || !inject.if_flag:
     if window not armed at >= this priority:     // dedup, QEMU's window_registered
       write DeliverabilityNotifications{InterruptNotification=1,
                                          InterruptPriority=vector>>4}
       inject.window = Some(priority)
     return Windowed
5. vector = cpu.pop_deliverable_vector(io)        // THE INTA MOMENT — the 8259 is acked
                                                  // here, at injection time, never when
                                                  // the line first rose (QEMU's rule)
6. if none: return Nothing                        // probe-vs-ack races resolve in the popper
7. partition.inject(BOOT_VP, PendingInterruption{Interrupt, vector})
   inject.in_flight = true
   return Injected
```

### B. Slice head (replaces the `emulate_one` delivery block, engine.rs:888-926)

```
1. io.sync_io_events(cpu)                          // line levels current BEFORE deciding
2. if !shadowed && cpu.has_non_ext_int_event():
     io.emulate_one(cpu); io.sync_io_events(cpu)   // NMI/SMI keep today's shadow delivery
3. run_the_shadow_out_of_smm(cpu, io)              // unchanged
4. install_the_shadow(started, cpu)                // state first — the exchange set never
                                                   // touches PendingInterruption, so safe
4b. if a shadow stretch consumed `shadowed` last slice: write InterruptState = 0
                                                   // Risk 7: the partition still holds the
                                                   // inhibit for an instruction the shadow
                                                   // already retired; NmiMasked is the
                                                   // sharper edge of the same register
5. stage_injection(...)
6. run
```

The comment at engine.rs:878-880 ("this engine never writes
`WHvRegisterPendingInterruption`") is the design assertion this stage retires.

### C. Post-exit tail (replaces the EventToDeliver slice-ender, engine.rs:1604-1619)

```
1. refresh InjectState from the exit header (free):
   in_flight = execution_state bit 6; shadowed hint = bit 12;
   if_flag = vp.rflags bit 9; cr8 = vp.cr8
2. io.sync_io_events(cpu)
3. if cpu.wants_a_machine_boundary() || io.needs_boundary(): return Boundary
4. if cpu.has_non_ext_int_event(): return Boundary(EventToDeliver)
                                                   // NMI/SMI/INIT still end the slice
5. stage_injection(...); continue the loop
   // the 158 µs median latency and the one-full-exchange-per-interrupt both die here
```

### D. InterruptWindow arm (replaces `unserviced` at engine.rs:1539)

```
ExitReason::InterruptWindow => { counts.window += 1; inject.window = None; }
// fall through to tail C — the gate re-runs with the fresh header and injects;
// if the line dropped meanwhile, the popper injects the spurious vector or nothing.
// No injection logic lives in this arm (QEMU/applepie convergence).
```

### E. Cancel-with-pending

The re-enter rule at engine.rs:1489-1511 stands, re-scoped: injection now
*creates* exits with bit 6 set (a cancel landing before the delivering entry),
so the rule protects our own injections too. It MUST stay while slice
boundaries still perform the full exchange — the boundary rewrites RFLAGS/RSP
under the delivery (the measured `Oops: int3` class). The warning at
engine.rs:1431-1437 ("the reasons the engine services by rewriting cannot
carry the bit") becomes false after this stage: bit 6 on a serviced exit then
means "our injection has not landed yet," and such an exit routes straight
back into the partition (like the Canceled re-enter) before any shadow
servicing.

### F. What survives untouched

`Started.shadowed` for all three shadow-execution paths;
`park_deliverable_interrupt`/`resume` for `finish_the_instruction`;
`EventDelivery::Engine` (only its doc's "delivering through the interpreter"
rationale rewords); the slice-head `emulate_one` for the non-ExtInt subset.

**Gate:** (a) the 158 µs median vector latency collapses to ~one exit round
trip, same irq tracing; (b) `ended_event_to_deliver` census ≈ 0 on
interrupt-driven stretches; (c) no `hda: unexpected_intr` or unclaimed-`int3`
guest signatures; (d) DLX/Alpine reach the same boot milestones; (e)
interleaved A/B wall-clock not worse (perf methodology).

---

## Stage 2 — lazy state exchange

### The state machine

```rust
enum ShadowFreshness {
    /// After a read-back or any shadow execution; the shadow is authoritative.
    /// The next install flushes (the held-comparison already dedups the write).
    ShadowCurrent,
    /// Hardware ran since the last read-back.
    PlatformCurrent { mirror: MirrorValidity },
}
enum MirrorValidity {
    /// rip/rflags/cs/cr8/cpl/pe/lma/bit6/bit12 answerable from the last exit.
    ExitHeader,
    /// Targeted registers were written since the exit (service_port_access's
    /// Rip+Rax write) — QEMU's get_pc names this third state explicitly.
    Invalidated,
}
```

The write direction is already lazy: `install_the_shadow`'s held-comparison
is better than a dirty flag because the machine is a second writer the
comparison catches. It stays, and doubles as the dirty bit.

### The single choke point

`Started::current_shadow(&mut self, cpu) -> Result<ShadowCurrent<'_>>` — a
witness type. Every consumer obtains it or provably reads only the exit
mirror. It wraps today's full read-back (export + XSAVE refresh +
InterruptState read + `discard_decoded_traces` + TSC rebase) and no-ops when
already `ShadowCurrent`. The witness is the proof the read-back ran; the
icache flush rides it, so the flush tax shrinks to shadow-execution events
automatically — no dirty-bitmap machinery needed.

### Mini-import (free, every slice end)

Copy `{rip, rflags, cr8}` from the last exit header into the shadow before
returning the processor. This keeps every verified between-slice
ARCHITECTURAL consumer truthful with zero platform calls: the scheduler's
runnable predicate (`interrupts_enabled()`, scheduler.rs:47), HLT
fast-forward (run.rs:751/755 — the Halt exit's own header is the freshest
possible IF), and the interactive gates. `activity_state` needs nothing (the
engine writes it via `record_halt`); the machine-delivery arm is compiled out
under `EventDelivery::Engine`; `has_an_event_to_deliver`'s only
platform-owned input is IF.

### Triggers that MUST fault the full state in

1. Any shadow execution (finish, burst, slice conversion, `emulate_one`, SMM
   run-out) — unchanged full read-back including the flush and TSC rebase.
2. Snapshot save. Restore additionally invalidates `held` (forces a full
   impose) and zeroes `PendingInterruption`/`PendingEvent` (wtf `LoadState`
   precedent).
3. Machine architectural writes — `deliver_sipi`, `inject_interrupt`
   (scheduler.rs:490-494), SMI/NMI/INIT via `apply_lapic_cpu_event`: fault in
   FIRST, then write, or two divergent copies merge. Event-bit writes
   (`sync_final_event_levels`, `sync_io_events`) are exempt:
   `pending_event`/`event_mask`/`async_event` live only on the shadow.
4. Diagnostics: the `&self` accessors cannot fault in — their contract is
   documented (rip/rflags/cr8 fresh via the mini-import, the rest
   as-of-last-shadow-event), and the two paths that would lie dangerously
   force a read-back: the slice-death report and `report_the_fault` (the
   latter already does).
5. TSC leaves the runtime path entirely (QEMU's `tsc_valid`): read once per
   stop/diagnostic, written only on restore under a partition time-suspend
   bracket. `install_the_shadow` already excludes it from comparison — the
   seam half-exists.

### Answerable from the exit context, never a read-back

RIP, RFLAGS/IF, CS, CPL, CR0.PE, EFER.LMA, CR8, InstructionLength,
InterruptionPending, InterruptShadow (header); RAX for non-string port I/O;
MSR rax/rdx; CPUID rax/rcx plus the hypervisor's default-result values.
QEMU's FAST_RUNTIME register tier is the template if shadow servicing later
shrinks further; `service_port_access` is the local existence proof.

**Aliasing rule** (QEMU's vmport hazard): if a port or MMIO handler forces a
full fault-in mid-service, results must be written into the *shadow*, not the
partition, or the next flush clobbers them — `service_port_access`'s targeted
Rip+Rax write checks freshness first.

**Gate:** (a) full-exchange count per second during steady boot drops from
≥1/slice to ≈ shadow-execution events (both directions counted separately);
(b) interleaved A/B boot wall-clock improvement; (c) snapshot
save→restore→save byte-equality; (d) the Risk-2 watchdog silent over a full
Alpine boot; (e) same boot milestones.

---

## Risks, ranked, each with its tripwire

1. **Lost or double delivery around the INTA moment** — per-vector
   `injected_count` must equal `acknowledge_count`; guest signatures
   `hda: unexpected_intr`, unclaimed `int3`.
2. **Wedge from a stale gate** (line asserted, no injection, no window) —
   QEMU's invariant as a watchdog: at each boundary assert
   `has_deliverable_ext_int ⇒ (in_flight ∨ window armed ∨ just injected)`;
   census `window == 0` while a boot stalls in HLT.
3. **Non-ExtInt leakage into the popper** — debug assert that the popped
   source bit ∈ `DeliverableInterrupt::MASK`; unit test: pending SMI +
   pending INTR must deliver SMI on the shadow, INTR by injection.
4. **Stage-2 machine writes onto a stale shadow** — held-comparison firing
   with multi-field diffs after a timers-only boundary; a shadow-generation
   counter assert.
5. **Cancel clobbering an in-flight injection** if the re-enter rule is ever
   weakened — the documented text-poke `#BP` signature returns.
6. **Window cache desync** — windows-armed vs window-exits census pair;
   a storm of InterruptWindow exits.
7. **Stale partition `InterruptState`** after the shadow retires a shadowing
   instruction (`state::import` never writes it) — protocol step B.4b clears
   it; NmiMasked is the sharper edge; NMI-in-NMI test.
8. **Spurious-vector mishandling in the extracted popper** —
   `vectors_acknowledged[spurious]` rising without guest "spurious interrupt"
   logs, or vice versa; parity unit test against `handle_async_event`.
9. **Diagnostics lying under lazy exchange** — identical RIP across distinct
   slice deaths; fixed by trigger 4's forced read-back.
10. **Partition-setup gap** (`ProcessorFeatures` unset) — Exception exits on
    modern guests that the all-shadow build never shows; Stage 0's audit.

## Key files

`rusty_box_whp_engine/src/engine.rs` (seams at 878-930, 1489-1511, 1539,
1589-1619, 1110-1157, 1200-1266) · `rusty_box_whp_engine/src/state.rs` (the
exchange set; carries neither InterruptState nor PendingInterruption) ·
`rusty_box_whp/src/partition.rs:783` and `src/vcpu.rs:173-222` (injection
verbs, already bound) · `rusty_box/src/cpu/event.rs:375-564` (the popper
source) · `rusty_box/src/cpu/arch_state.rs:48-63, 536-571` ·
`rusty_box/src/iodev/irq.rs:196-218` · `rusty_box/src/emulator/io.rs:96-126,
217-256` · `rusty_box/src/emulator/scheduler.rs:28-50` ·
`rusty_box/src/emulator/run.rs:741-798`.
