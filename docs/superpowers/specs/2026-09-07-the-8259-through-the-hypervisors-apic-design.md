# The 8259 reaches a hardware guest through the hypervisor's APIC

**Status:** design, awaiting review
**Date:** 2026-09-07
**Applies to:** `rusty_box_whp_engine`, fast mode only (`DeviceClock::HostTime`)

## The problem

A machine on the hypervisor cannot deliver a legacy 8259 interrupt. DLX does not
boot: after the CPUID retirement fix (`0b3f283`) the guest reaches a legitimate
BIOS `RIP` and then spins forever with `injected 0`.

The current design hand-places a `WHvX64PendingEventExtInt` on the processor,
and gates that placement on positive evidence that the guest can take it. The
gate is the problem. Two measured failures:

| retry rule | cancels | port exits | in-run share | outcome |
|---|---|---|---|---|
| re-cancel at every boundary | 104,563,825 | 17,909 | 21% | frozen at one `RIP` |
| re-cancel once per entry | 124,552,733 | 20,357 | 22% | frozen at one `RIP` |
| no re-cancel at all | 561 | 6,607,555 | 86% | executes, never gets its interrupt |

The mechanism of the first two: `WHvCancelRunVirtualProcessor` is **sticky and
armed before the entry**, so a cancel issued to a processor that has not run
returns the next run without retiring a single instruction. The guest can never
execute its way to the `STI` that would let it accept the vector. Bounding the
retry by the entry count does not help, because a cancelled entry is still an
entry.

The reason no bound works is not a coding error. The platform primitive designed
to answer "the guest can take it now" — the deliverability notification — is
**measured dead** under a hypervisor APIC: accepted, read back exactly as
written, and never producing a window exit across sixteen thousand `STI`s
(probe P10, `docs/whp-interrupt-window-2026-09-06.md`, with four theories
already refuted). We are synthesising that signal from cross-thread counters,
and no correct proxy exists.

## The rule Bochs follows

Bochs answers the same question twice, differently, and the difference is the
whole design.

When the 8259 is wired straight at the processor, it raises a FLAG and reads no
vector (`pc_system.cc bx_pc_system_c::raise_INTR`):

```c
void bx_pc_system_c::raise_INTR(void) {
  BX_CPU(BX_BOOTSTRAP_PROCESSOR)->raise_INTR();
}
```

The number is fetched only when the processor is ready to take it
(`cpu/event.cc`):

```c
    vector = DEV_pic_iac(); // may set INTR with next interrupt
```

When the same 8259 travels through the I/O APIC, Bochs reads the number
immediately and hands it over (`iodev/ioapic.cc service_ioapic`):

```c
        if (entry->delivery_mode() == 7) {
          vector = DEV_pic_iac();
        } else {
          vector = entry->vector();
        }
        bool done = apic_bus_deliver_interrupt(vector, entry->destination(), ...);
```

**If the interrupt travels as a wire, defer the acknowledge. If it travels as a
number, acknowledge now.** An APIC is handed numbers, not wires.

## Why that decides it here

`WHV_INTERRUPT_TYPE` offers Fixed(0), LowestPriority(1), Nmi(4), Init(5),
Sipi(6) and LocalInt1(9). There is no ExtINT(7) and no LocalInt0(8). Under a
hypervisor APIC there is no way to assert the LINT0 wire at all, so the
interrupt must travel as a number — which is Bochs's second branch, for Bochs's
own reason.

The mechanism is already in the tree and already passing. An I/O APIC entry in
ExtINT mode resolves its vector through the fabric's counted acknowledge and is
delivered by `WhpEngine::route_ioapic_delivery` as `InterruptKind::Fixed`;
`an_ioapic_edge_is_delivered_by_the_hypervisor_apic_without_an_exit` covers it.
`requested_kind` already carries the explanation for why ExtINT becomes Fixed:
the acknowledge has already happened, so there is nothing ExtINT-shaped left to
carry. QEMU's `whpx-apic.c` calls the same `WHvRequestInterrupt` for MSI,
passing the delivery mode straight through.

So the fix is not new machinery. It is routing the 8259's pin through the path
that already works, and deleting the path that does not.

## The design

**The machine acknowledges; the engine delivers.** The 8259 belongs to the
machine, so the INTA stays on the machine's side of the seam, exactly where the
I/O APIC's ExtINT acknowledge already happens. The engine receives a vector and
a destination, which is all `WHvRequestInterrupt` wants.

The site is the tail of `Emulator::sync_final_event_levels` — where the pin's
level is published today, and the one place the machine samples it (R5). When
that boundary finds the 8259's INT pin asserted, and only when
`IrqFabric::lint0_admits_ext_int` says the guest's LINT0 still admits the legacy
line (divergence D6), the machine performs the counted acknowledge and routes
the resulting vector to the boot processor as a Fixed, physical, edge delivery —
through the same `DeliveryRoute` the I/O APIC's messages take.

This runs only for a machine whose ENGINE owns the guest's local APIC. Where
this machine's own model APIC is the guest's — the interpreter, and any WHP
machine on `DeviceClock::Ticks` — the pin reaches the processor through
`set_legacy_intr_level` exactly as it does today, and none of this applies.

**The machine must know which of the two it is BEFORE it acknowledges**, or a
route that came back `Model` would leave the vector already taken and the model
path would acknowledge a second time. So `pic_pin_changed` is replaced by a
question rather than an action:

```rust
/// Whether this engine, not this machine's own `cpu/apic.rs`, is the local
/// APIC the guest reads. Answering `true` moves the legacy 8259 line onto
/// `route_ioapic_delivery`, and with it the acknowledge that resolves its
/// vector.
fn owns_the_guests_local_apic(&self) -> bool { false }
```

The default is `false`, so an engine that says nothing keeps the model path and
the deferred acknowledge — which is what `SoftwareEngine` and the test engines
want, and means they need no change beyond dropping the removed method.

The hypervisor's APIC then holds the vector in its IRR and delivers it when the
guest becomes ready. That is the component designed to answer the readiness
question, and it answers it without anyone being fetched out of a run.

**Runaway is prevented by the 8259 itself, not by a new gate.** Its own
in-service and priority logic stops it presenting a second interrupt of equal or
lower priority until the guest writes EOI, and that EOI is a port write the
machine already services. A boundary that finds the pin low acknowledges
nothing. This is the same shape as `service_ioapic`, which services a pin
whenever its IRR bit is set and it is not masked.

### What is deleted

- `VcpuControl::{ext_int_pending, ext_int_blocked}` and `raise_ext_int`
- `ext_int_request` and its two tests
- `VcpuThread::stage_the_legacy_interrupt` and its call from the run loop
- the `PendingExtIntEvent` write and the `Reg::PendingEvent` path for ExtINT
- `SliceEngine::pic_pin_changed` **entirely**, from the trait and from all four
  implementations (`engine.rs`'s default, `WhpEngine`, and the two test engines
  in `emulator/tests.rs`). With the acknowledge and the routing on the machine's
  side there is nothing left an engine needs to be TOLD about a pin level — only
  something it must be ASKED, which is `owns_the_guests_local_apic`. It is also
  the last caller that fetches a processor out of its run for an interrupt. The
  `pic_pin_published` bookkeeping that fed it goes with it.

### What is kept

- `IrqFabric::lint0_admits_ext_int` — divergence D6 is unchanged; a guest that
  masks LINT0 still takes no legacy interrupt, and the line is not spent
- the fabric's counted acknowledge, so `InjectCensus::injected` stays comparable
  with it
- `route_ioapic_delivery` and `requested_kind` exactly as they are

## The divergence this introduces

The 8259 hands over its vector when the pin asserts rather than when the
processor is ready. If a guest masks that IRQ in the gap, the vector has already
left the controller and is delivered anyway.

This is registered rather than hidden, and it is the branch Bochs itself takes
whenever the line travels through an APIC — so the machine is not doing
something Bochs would not. It is also already this port's behaviour for an I/O
APIC entry in ExtINT mode, which is the other way a PC wires the same
controller. Fast mode only: the interpreter's machine keeps Bochs's deferred
acknowledge, because its processor is on the same thread and can be asked.

## Testing

1. **Hypervisor-free**: a boundary that finds the pin asserted with LINT0
   admitting produces exactly one counted acknowledge and one delivery to the
   boot processor; one that finds LINT0 masked produces neither, and does not
   spend the line. Both drive `IrqFabric` and the delivery route directly.
2. **On hardware, gated**: `a_legacy_8259_vector_reaches_a_hardware_guest_as_a_placed_ext_int`
   is rewritten rather than retired — its subject, a legacy vector reaching a
   guest running on hardware, survives; only the means changes. It moves onto
   `FastMachine`, loses "as a placed ext int" from its name, and gains the
   assertion the old mechanism could not make: `census.exits.canceled` at or
   below the step's own pauses. That number was 104 million.
3. **End to end**: `dlx_whp` reaches 3/3 milestones, three runs. This is the
   acceptance criterion Task 1.8 could not meet and the reason this work exists.

## What this does not address

`injected 0` is the symptom being fixed, but the run that produced it also
showed 6.6 million port exits in 40 seconds with the guest making no visible
progress. If the guest still fails to boot once its interrupt arrives, that is a
separate defect and gets its own investigation rather than a patch here.
