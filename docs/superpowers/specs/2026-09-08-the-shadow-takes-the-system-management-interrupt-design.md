# The shadow takes the system-management interrupt

**Status:** design, awaiting review
**Date:** 2026-09-08
**Applies to:** `rusty_box_whp_engine`, fast mode only (`DeviceClock::HostTime`)

## The problem

Every BIOS-booted guest on the hypervisor stalls in the same place. DLX is 0/3
milestones on three identical runs; Alpine is 0/3 in 300 s. Both sit at RIP
`0xe0438` for the whole run, which is `rombios32.c smm_init`:

```c
outb(0xb3, 0x01); outb(0xb2, 0x00); while (inb(0xb3) != 0x00);
```

The BIOS raises an SMI through the APM command port and spins until its own SMM
relocation handler clears port `0xb3`. The handler never runs, so the BIOS never
reaches its first `printf` — the screen stays blank and even the first milestone
cannot land.

## The break, traced end to end

1. `out 0xb2` is a port exit, serviced **on the vCPU thread**; the write reaches
   `acpi.generate_smi` and sets `acpi.smi_request_pending` (`iodev/acpi.rs`).
2. The **device thread** drains it: `device_thread::service_once` →
   `service_device_time` → `service_scheduler_boundary` (`emulator/scheduler.rs`),
   which calls `self.cpu_mut_at(0).deliver_smi()`.
3. `deliver_smi` (`cpu/event.rs`) does one thing: `signal_event(BX_EVENT_SMI)` —
   it sets a bit in the **shadow** CPU's `pending_event` word.
4. **Nothing on the hypervisor path ever reads that word.** `BX_EVENT_SMI` and
   `pending_event` have zero occurrences in `rusty_box_whp_engine`. The guest is
   executing inside the partition; the shadow's flag is inert.

The same is true of the other producer, `LocalApicCpuEvent::Smi`
(`emulator/scheduler.rs`), which routes an APIC-bus SMI to the same bit.

## The machinery is not missing — only its trigger is

`vcpu_thread.rs`'s `ExitReason::ApicSmiTrap` arm already does the whole thing,
correctly, and it works today:

```rust
self.exchange.import_everything(self.vcpu, cpu, self.xsave)?;
io.deliver_smi(cpu);
io.emulate_one(cpu)?;                  // the shadow takes the SMI, enters SMM
io.sync_io_events(cpu);
run_the_shadow_out_of_smm(cpu, io)?;   // engine.rs; runs to RSM under a ceiling
self.exchange.export_imported(self.vcpu, cpu, self.xsave)?;
```

That arm fires only for an SMI raised by the *guest's own APIC*. A chipset SMI
has no route to it. So this design adds a trigger and reuses the sequence
unchanged.

## Prior art: this is applepie's model, and its stated reason

`gamozolabs/applepie` is the closest existing system to this one — a Rust DLL
loaded by a patched Bochs that runs Bochs's CPU on WHP against a shadow CPU. Its
README says (recorded in `docs/research/whp-2026-09-03/04-openvmm-hyperlight-wtf.md` §4.4):

> "Rather than scheduling interrupts to be delivered to the hypervisor we handle
> *all* interrupts in Bochs emulation itself … This also gives us features that
> WHVP doesn't support, like SMIs (for SMM)."

Mechanically it raises `async_event`, runs `handleAsyncEvent()` on the Bochs CPU
so the IDT walk and frame push happen there, and ships the post-interrupt state
into the partition on the next `set_context`. The `ApicSmiTrap` sequence above is
that model already. The gap is only that a chipset SMI never reaches it.

This is also why the fix is a *trigger* and not a mechanism: the hard part —
entering SMM on the shadow, running the handler to `RSM` under a ceiling, and
exporting the result — is built, tested and in use.

## The design

**Before it re-enters the partition, the vCPU thread asks whether the shadow owes
a system-management interrupt, and if so runs the existing sequence.**

`rusty_box` gains one public predicate, because `BX_EVENT_SMI` and
`pending_event` are `pub(crate)` and the engine cannot ask today:

```rust
/// Whether this processor has been signalled a system-management interrupt it
/// has not yet taken.
///
/// The one question an engine that runs the guest elsewhere must ask: an SMI
/// signalled here is a bit in this processor's event word, and a guest running
/// on a hypervisor never consults it.
#[must_use]
pub fn owes_a_system_management_interrupt(&self) -> bool {
    (self.pending_event & Self::BX_EVENT_SMI) != 0
}
```

The `ApicSmiTrap` arm's body is factored into one method so the exit path and the
entry path run the identical sequence rather than two copies that can drift:

```rust
fn take_the_system_management_interrupt(&mut self, cpu: &mut BxCpuC<T>, io: &mut PcIo<'_>)
    -> Result<()>
```

and the entry path runs it **before** the legacy-interrupt staging that already
lives there:

```rust
// An SMI outranks the 8259's line, exactly as Bochs orders them
// (`cpu/event.cc handleAsyncEvent`: SMI is tested before INTR). Taking the
// legacy vector first would push an interrupt frame onto a processor that is
// about to enter system-management mode, and SMM would then resume into the
// handler rather than into the instruction the guest was executing.
if cpu.owes_a_system_management_interrupt() {
    self.take_the_system_management_interrupt(cpu, io)?;
}
if let Continue::Park(why) = self.stage_the_legacy_interrupt() { … }
```

The ordering is not a preference. Bochs tests SMI before INTR in
`cpu/event.cc handleAsyncEvent`, and this port must match that or a guest can
observe an interrupt frame built on the wrong side of an SMM entry.

### Why ask rather than be told

Two alternatives were considered and rejected.

**Telling the engine, as the 8259's pin does** — a `SliceEngine::smi_raised`
that the boundary calls — re-adds a trait method of the kind just deleted, and
needs its own `in_run`/Dekker pairing to avoid a lost wakeup. More machinery for
the same outcome.

**Draining `smi_request_pending` on the vCPU thread** at the `out 0xb2` exit is
thread-local and immediate, but creates a *second* drain site — R5 requires one
choke point per hazard — and misses the APIC-bus producer entirely.

Asking about the *state* rather than intercepting an *event* cannot miss a
producer: both `scheduler.rs` sites set the same bit, and any later one will too.

### Scope: SMI only, and why that is complete rather than partial

`INIT` and `SIPI` have the same structural gap but are out of scope by
construction: the engine configures `processor_count(1)` and its own comment
records that "SMP under a hypervisor is its own unit". With no APs there is
nothing for them to start.

`NMI` has no producer in a uniprocessor Alpine boot, and would not use this path
anyway — `WHV_INTERRUPT_TYPE` has `Nmi(4)`, so it is an injection rather than a
shadow run.

## What this does and does not promise

It removes the wall at `0xe0438`. **It does not promise that Alpine boots**:
Alpine has never reached `login:` on `FastMachine`, because the slice engine that
last booted it was deleted at Task 1.7 and every attempt since has died at this
wall or the cancel livelock before it. There may be another wall behind this one.
The honest sequence is: remove this one, boot, and let the guest say what is
next.

## Testing

1. **Hypervisor-free**: a processor signalled an SMI answers
   `owes_a_system_management_interrupt`; one that has taken it answers false.
   Drives `BxCpuC` directly.
2. **On hardware**: a guest that writes `0xb2` with `APMC_EN` set has its SMM
   handler run and its `0xb3` status cleared, from inside the partition. This is
   the property `rombios32` waits on, expressed as a test.
3. **End to end**: `dlx_whp` past the `smm_init` wait — the first milestone,
   "the BIOS is alive", must land. Report the milestone count truthfully; do not
   adjust it.
4. **Alpine**: `alpine_bench` on both engines. The bar for this work is
   **Alpine reaches `login:` on the hypervisor faster than the interpreter's
   76.4 s**. VMware parity (the governing spec's "within 2×") is explicitly
   deferred to later work.

## Banked for the performance stage, not built here

applepie runs **250 instructions on the emulator after each MMIO/PIO exit**
(`EMULATE_STEPS = 250`, tuned: "<10 is unusable, >1000 introduces latency")
rather than re-entering immediately, "due to the API costs of entering and
exiting the hypervisor, and the likelihood that similar MMIO operations occur
next to others". DLX takes **70,074,467** port exits in 360 s. If Alpine is
similarly exit-bound, this is the largest known lever, and it is measured prior
art rather than a guess. It is deliberately not part of this change: a
performance change designed before the guest boots would be tuned against a
profile nobody has seen.
