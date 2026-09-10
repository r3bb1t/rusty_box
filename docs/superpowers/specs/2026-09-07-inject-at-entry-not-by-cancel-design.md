# The legacy interrupt is injected at entry, not fetched out by a cancel

**Status:** design, awaiting review
**Date:** 2026-09-07
**Applies to:** `rusty_box_whp_engine`, fast mode only (`DeviceClock::HostTime`)
**Supersedes:** `2026-09-07-the-8259-through-the-hypervisors-apic-design.md`, which
is refuted by measurement (see "What the previous design got wrong")

## The problem

A legacy 8259 interrupt does not reach a guest running on the hypervisor. DLX on
WHP is **0/3** milestones and has been for the whole campaign.

## What is measured, and how

Two facts decide this design. Both were established on this host, and the first
was mutation-proven because the test that carries it can pass vacuously.

**1. `WHvX64PendingEventExtInt` DELIVERS.** At `0329d76` the test
`a_legacy_8259_vector_reaches_a_hardware_guest_as_a_placed_ext_int` drives a
real-mode guest — IVT[8] → ISR, IRQ0 unmasked, PIT ch0 mode 2, `sti`, then an
**exit-free** busy loop (`inc ax; jmp $-1`), so the only way in is a placed
event — and the guest's own ISR writes its MARK three times.

That test is gated on `hypervisor_here()` with a bare `return`, so a skip is
indistinguishable from a pass and it finishes in 0.03 s either way (3 PIT ticks
at 858 µs really is ~2.6 ms). It was therefore **mutation-proven**: raising the
assertion to `injected >= 300_000` fails with `injected 3, of vector 8 3`.

**2. Routing through the hypervisor's APIC CANNOT work**, for two independent
reasons: `WHvRequestInterrupt` refuses vector `0x08` with `0xC0350005`
(`ERROR_HV_INVALID_PARAMETER`) because a local APIC takes no vector below 16
(this port's own `BX_LAPIC_FIRST_VECTOR = 0x10`; Bochs `cpu/apic.cc trigger_irq`
rejects the same); and remapped to `0x20` the call is accepted while the vector
**vanishes**, because the partition's APIC is software-disabled at reset and a
legacy guest never enables it.

## The root cause

`ext_int_request` issues `WHvCancelRunVirtualProcessor` on the pin's rise
**without knowing whether the vCPU is inside `WHvRunVirtualProcessor`**. The run
loop tracks `in_run_nanos`, a duration — never a boolean the canceller can
consult. A cancel issued between runs is latched by the platform and consumes the
NEXT entry, retiring zero instructions.

This explains both observations at once, which is what makes it a root cause
rather than a theory:

| | guest | vCPU usually | the cancel | outcome |
|---|---|---|---|---|
| the unit test | exit-free busy loop | INSIDE the run | lands | delivers, passes |
| DLX BIOS | 6.6M port exits / 40 s | BETWEEN runs | latched | livelock at one RIP |

Microsoft's documentation does not specify what a cancel does to a processor that
is not running, so this is undocumented platform behaviour, not a misreading.

## What QEMU does, and what we take from it

QEMU's `whpx_vcpu_pre_run` (`target/i386/whpx/whpx-all.c`) **never cancels in
order to inject**. It injects immediately before entering the run, gated on state
it already holds from the previous exit:

```c
if (!vcpu->interruption_pending && vcpu->interruptable &&
    (env->eflags & IF_MASK) && (vcpu->tpr < irr || irr == 0))
```

where `interruptable` is `!ExecutionState.InterruptShadow`. Under the
kernel-irqchip — the hypervisor-APIC mode this engine selects in fast mode — it
places exactly the event this port already used:

```c
reg_values[reg_count].ExtIntEvent = (WHV_X64_PENDING_EXT_INT_EVENT) {
    .EventPending = 1, .EventType = WHvX64PendingEventExtInt, .Vector = irq,
};
```

So the event type was already right. Only the trigger was wrong.

QEMU also carries a fact about HLT: **`WHvRegisterPendingEvent` does not reset
the HLT state**, so QEMU added `whpx_vcpu_kick_out_of_hlt()`.

**This port already had that, and had measured it independently.** The deleted
staging wrote the whole internal-activity register before entering:

```rust
// Load-bearing, not redundant: measured, a processor parked in `HLT`
// under an emulated APIC sets `halt_suspend`, produces no halt exit, and
// does NOT run the handler for a placed event until the suspend is cleared.
const RUNNING: InternalActivity =
    InternalActivity { startup_suspend: false, halt_suspend: false, idle_suspend: false };
vcpu.set_internal_activity(RUNNING)?;
```

The same is true of the readiness gate: `inject.permits_ext_int()` already tested
IF, the interrupt shadow and an in-flight delivery from the exit header — QEMU's
gate, arrived at independently.

**So the research corroborates the deleted code rather than correcting it.** The
staging was right in every part except one, and that is what makes this change
small: restore it, and replace the trigger.

## The design

**Injection happens at entry. A cancel is only ever issued to a processor that is
provably running.**

### The trigger

The raiser records the request and cancels only when the processor is inside a
run:

```rust
pending.store(true, Ordering::SeqCst);
if in_run.load(Ordering::SeqCst) {
    cancel.cancel()?;
}
```

The thread already calls `stage_the_legacy_interrupt()` before each entry, and
that body is kept **unchanged** — its LINT0 gate, its `permits_ext_int` readiness
test, its counted INTA, its `PendingExtIntEvent` write, its `set_internal_activity`
HLT kick and its `sync_io_events` all stay exactly as they were. What is added
around it is the `in_run` window and the re-check that closes the lost-wakeup
race:

```rust
if let Continue::Park(why) = self.stage_the_legacy_interrupt() {
    return self.park(why);
}
self.control.in_run.store(true, Ordering::SeqCst);
// The raiser may have set the flag in the window between the staging above
// and this store, and read `in_run` as false, so it issued no cancel. Enter
// only when nothing is owed; otherwise go round and stage it.
if self.control.ext_int_pending.load(Ordering::SeqCst) {
    self.control.in_run.store(false, Ordering::SeqCst);
    continue;
}
let exit = self.vcpu.run();
self.control.in_run.store(false, Ordering::SeqCst);
```

The store/load pairs are `SeqCst` on both sides deliberately: this is Dekker's
pattern, and it is the only thing preventing a lost wakeup when the raiser reads
`in_run` as false in the window before the thread sets it. Weaker orderings admit
the interleaving where neither party acts.

`can_take_it` mirrors QEMU's gate, read from the last exit's context rather than
from a fresh register import: no interruption already pending, no interrupt
shadow, and `RFLAGS.IF` set.

### Why this is fast

The common path issues **no cancel at all**. DLX takes 6.6 million port exits in
40 seconds; every one of them is an injection opportunity that costs two atomic
stores and a load. A cancel is reached only when the pin rises while the guest is
genuinely inside a long run, which is the case the old code got right.

### What is kept from the refuted design

Nothing, and that is deliberate. All three of its commits are reverted, so the
tree lands exactly on `0329d76` — the state in which the delivery test is proven
to pass.

`owns_the_guests_local_apic` was a nicer seam than the `pic_pin_changed` it
replaced, and it is tempting to keep. It cannot be kept cheaply:
`pic_pin_changed` was the ONLY caller of `raise_ext_int`, so keeping the new seam
means inventing a new trigger path inside the very step whose purpose is to
reach a *known-good* baseline. That would put unproven code underneath the test
that is supposed to be proving the baseline. Re-introducing it afterwards is a
separate change that can be verified on its own merits.

The I/O APIC path (`route_ioapic_delivery` → `deliver_ioapic_to_lapics`) is
untouched by any of this. That is the path Alpine uses once booted, and it works.

## Performance is a first-class requirement, not a side effect

This campaign exists to run guests fast on hardware. The slice model gave the
guest **under 1% of wall time**; the VMM shape exists to fix that. So:

- **Alpine must not regress.** Alpine reaches userspace over the I/O APIC, which
  this design does not touch, but the trigger runs at every machine boundary and
  must not become a cost. Measured with `alpine_bench`, before and after.
- **The in-run share must stay high.** `fast_machine`'s existing assertion —
  that a 10 ms step spends at least half its time inside the partition — is the
  standing guard and must keep passing.
- **The cancel count is the headline number.** It was 104,563,825 for a boot
  that never progressed. The acceptance assertion is a small constant.

## Testing

1. **The trigger, hypervisor-free.** `ext_int_request`'s successor is a pure
   function over two atomics and a `CancelRun`, exactly as the deleted one was,
   so both branches are unit-testable without a partition: a raise while
   `in_run` is false cancels nothing and leaves the request set; a raise while it
   is true cancels once.
2. **Delivery, on hardware.** The retired test returns unchanged in subject: the
   guest's own ISR must run three times from an exit-free loop. **It must be
   mutation-checked** — flip its threshold, see it fail, restore — because it
   passes vacuously without a hypervisor.
3. **The halted guest.** A guest that executes `sti; hlt` and nothing else must
   still take its tick. The restored `set_internal_activity(RUNNING)` is what
   makes this pass; the test exists to keep it, because it is the one line whose
   necessity is invisible from the code and would be an easy "redundant write"
   for a later reader to delete. QEMU needed the same thing.
4. **End to end.** `dlx_whp` reaches 3/3 milestones, three runs, with the cancel
   count at or below the step's own pauses.
5. **Alpine.** `alpine_bench` throughput within noise of `c9d2fbd`.

## What this does not address

The deliverability notification is measured inert on this host (probe P10), so a
guest that spins with `IF=0`, takes no exits, and never halts still cannot be
reached; the `in_run` cancel is what covers it. If that proves insufficient it is
a separate investigation and gets its own spec rather than a patch here.
