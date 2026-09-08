# Alpine reaches `login:` on the hypervisor

**Status:** design, awaiting review
**Date:** 2026-09-08
**Applies to:** `rusty_box_whp_engine`, fast mode only (`DeviceClock::HostTime`)

## Where things stand

The chipset-SMI wall is down (`73f3c3a`, `1d4f324`). DLX went 0/3 → **2/3** on
every run: "the BIOS is alive" lands in 1.6–5.2 s and LILO in 4.9–10.3 s, with
the in-run share at **97.7–98.8 %** — past the governing spec's ≥90 % bar.

Two walls remain, and they are different in kind.

**Alpine dies of a harness bug.** It reaches BIOS (2.1–2.3 s) and ISOLINUX
(5.4–5.7 s), then faults at 8–10 s: `WHvSetVirtualProcessorRegisters` →
`0xC0350005`, writing back a 64-bit kernel state with `xcr0 = 0xe7` — AVX-512
state bits — to a partition on an i5-12450H that has no AVX-512. The RIP
(`0xffffffff9e2a368e`) is a kernel address, so Alpine's kernel is up and running.
`alpine_bench` builds from `EmulatorConfig::default()` and never applies the
`host_shared` CPUID clamp that `alpine_probe.rs` and the GUI both do.

**DLX dies of an engine defect.** Linux 1.3.89 boots, finds `hda`, mounts root,
then loops `hda: irq timeout` → `ide0: reset` → `end_request: I/O error` →
VFS/EXT2 panic. IRQ 0 (PIT, master 8259) is injected at ~100/s for the whole run;
IRQ 14 (IDE, **slave via cascade**) works for a few commands and then never
again.

## The mechanism that fits, and why it is not yet a finding

A slave IRQ sets the in-service bit on the slave **and** on the master's cascade
line, IRQ2, and master IRQ2 stays set until the guest EOIs the master (OSDev's
8259 page and Linux's `arch/x86/kernel/i8259.c` both state this; the guest must
EOI slave then master). So **one slave vector that is acknowledged but never
delivered wedges the cascade permanently** — while IRQ 0, being higher priority
than IRQ2, keeps flowing untouched. That is exactly the observed asymmetry.

Where such a loss could come from is specific to this engine. On the interpreter
the INTA and the jump to the handler are effectively atomic. Here they are
separated by a VM entry: `stage_the_legacy_interrupt` acknowledges at the
machine's own PIC, places a `WHvX64PendingEventExtInt`, and only then enters. The
guard against acknowledging a second vector before the first is taken is
`InjectState::in_flight`, which `refresh_from` derives from
`WHV_X64_VP_EXECUTION_STATE` bit 6, `InterruptionPending`. That bit tracks
`WHvRegisterPendingInterruption` — **a different register from the
`WHvRegisterPendingEvent` this engine writes.** If it does not reflect a placed
ExtINT, the staging can acknowledge a second vector and overwrite the first.

**This is a hypothesis, not a measurement.** Two designs in this campaign have
already been refuted after being built on reasoning of exactly this shape. It
gets tested before anything is fixed.

## The design

**Fix what is known. Measure what is not.**

### 1. The clamp (known defect, no diagnosis needed)

`alpine_bench` gains the same `host_shared` narrowing `alpine_probe.rs` applies:
drop AVX-512 when the host's `CPUID.(EAX=0xD,ECX=0)` does not carry its XSAVE
state, drop AVX likewise, and drop MONITOR/MWAIT unconditionally. This is a
correction to a benchmark, not to the engine, and it unblocks Alpine as far as
whatever wall it meets next — most likely the same IRQ-14 one DLX sits at.

### 2. The invariant that names the DLX defect

The retired hardware test already asserted the property that decides this:

> one INTA cycle at this machine's own controllers per placed vector — a
> mismatch is a vector taken from the 8259 and never delivered, or one delivered
> that was never taken

`dlx_whp` prints `injected` and `windows_armed` but not `injected_per_vector`,
which the census already collects, and never reads the fabric's
`acknowledge_count`. Printing both turns a 6-minute run into a verdict:

- **`acknowledge_count > injected`** → vectors are acknowledged and not
  delivered. The window is real, and the cascade wedge follows from it.
- **counts equal, `injected_per_vector[14]` non-zero then static** → delivery is
  fine and the IDE stopped raising. A device-side investigation, not an
  interrupt one.
- **`injected_per_vector[14]` zero throughout** → the vector is never
  acknowledged at all: the pin never rises, or a gate refuses it.

Three distinct causes, one run, no guessing.

### 3. What is deliberately NOT in this design

No fix for the window. Not because it is unlikely, but because it has not been
measured, and this campaign has already spent a full cycle implementing a design
that measurement then refuted. The fix follows the verdict.

If the verdict is "vectors acknowledged and not delivered", the principled repair
is applepie's model — deliver interrupts on the shadow, building the frame there
so the acknowledge and the delivery are one step, then export the post-interrupt
state — rather than adding another guard around a window that should not exist.
applepie's README states it handles all interrupts in Bochs emulation itself for
exactly this reason. That is a separate spec, written against the verdict.

## Success criteria

1. `alpine_bench`'s hypervisor arm no longer faults on `xcr0`, and reports how
   far Alpine then gets.
2. One `dlx_whp` run prints `acknowledge_count`, `injected`, and the non-zero
   entries of `injected_per_vector`, and the report states which of the three
   causes above it selects.

The user's bar — Alpine reaching `login:` on the hypervisor faster than the
interpreter — is **not** claimed by this work. It is the next spec's, and it
needs the verdict first.

## Measurement discipline this host forces

Any timing number quoted from this laptop must carry the host's power state
beside it. Measured on 2026-09-08: one DLX run spent 245 s of its 360 s in
Modern Standby, and another ran 91 s on battery at half clock (BogoMIPS 2254 vs
~4600–5560). Neither announces itself in the benchmark output — a half-slept run
looks exactly like a slow guest. The interpreter's own Alpine time swung
**110.8 s to 149.6 s** across two consecutive clean runs, so the 76.4 s figure
quoted earlier in this campaign is not a reproducible baseline and must not be
used as one.
