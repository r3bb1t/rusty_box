# The interrupt-deliverability window under an emulated local APIC

**Question.** When the WHP partition runs with `LocalApicEmulationMode = X2Apic`/`XApic`, how does
a VMM deliver a legacy 8259 ExtINT into a guest that is momentarily unable to take one (`IF=0`, an
interrupt shadow, or a delivery already in flight)?

**Answer, in one line.** It cannot use `WHvX64RegisterDeliverabilityNotifications`. The register is
accepted and reads back, and never produces an interrupt-window exit. Measured here with a working
control; corroborated by what two shipping VMMs do.

This matters because it is the only mechanism that could tell the engine "the guest is ready now"
without the guest exiting for some other reason. Without it, a vector owed to a guest that becomes
ready without exiting is owed indefinitely.

---

## 1. MEASURED on this host (probe P10, `rusty_box_whp/examples/whp_probe.rs`)

One guest, four arms. The guest spins with interrupts off, then `sti` and spins forever — so the
thing being asked for is a report of the TRANSITION into interruptibility, which is exactly what
the legacy path needs.

```
None    armed 0x0002  read back 0x0002  ->  InterruptWindow (left on its own)   ** WINDOW **
X2Apic  armed 0x0002  read back 0x0002  ->  Canceled (rescued)
X2Apic  armed 0x003e  ->  platform REFUSED the write: HRESULT 0xC0350005
XApic   armed 0x0002  read back 0x0002  ->  Canceled (rescued)
```

`0x0002` is `InterruptNotification = 1, InterruptPriority = 0`; `0x003e` adds priority 15.

**The `None` arm is the control and it fired**, on its own, without the rescue. Same guest, same
arming, same code — so the guest is right and the arming is right, and the APIC mode is the only
variable. This supersedes nothing: an earlier measurement of 0 window exits across 16,384 guest
`STI`s and 16,401 re-arms said the same thing without a control. The control is what makes it
evidence rather than an absence.

**New, and it suggests the mechanism.** Under `X2Apic` a NON-ZERO `InterruptPriority` is refused
outright with `0xC0350005` — the same code that refuses `WHvX64RegisterApicTpr`
([probe P2](whp-platform-probe-2026-09-03.md)). Once
the hypervisor owns the APIC it owns interrupt priority, and this register is priority-qualified.
The notification looks like a mechanism built for a VMM that owns its own APIC, being asked of one
that has given the APIC away.

**Untested hypothesis, recorded so nobody assumes it either way:** the notification may report only
on an interrupt the hypervisor's OWN APIC holds — "tell me when I can deliver what I have" — rather
than on guest interruptibility as such. If that is what it means, it is structurally unusable for a
PIC ExtINT the VMM places itself, and no arming pattern would help. Deciding this needs a probe
that requests a fixed vector via `WHvRequestInterrupt` while `IF=0`, arms the notification, and
sees whether a window exit follows.

---

## 2. DOCUMENTED: nothing

Microsoft's WHP reference for the interrupt registers, `WHvRequestInterrupt`,
`WHvSetVirtualProcessorRegisters` and `WHvRunVirtualProcessor` specifies no preconditions, no error
conditions, and no interaction between `DeliverabilityNotifications` and `LocalApicEmulationMode`.
The register-datatypes page's entire Remarks section is one sentence about what the data types are.

So the behaviour is **undocumented, not a documented limitation** — which establishes only that no
contract was published. It does not license the usage, and it does not condemn it.

---

## 3. OBSERVED in shipped source (verified 3-0 unless noted)

**QEMU uses the in-hypervisor APIC and depends on the window inside it, with no fallback.**
`whpx_accel_init` sets `WHvX64LocalApicEmulationModeX2Apic` (never XApic) — that is what QEMU calls
`kernel-irqchip`. It arms `InterruptNotification = 1` in BOTH APIC modes; the arming block in
`whpx_vcpu_pre_run` is **not** gated on `whpx_irqchip_in_kernel()`. Under the emulated APIC it
writes the ExtINT as `WHvRegisterPendingEvent`/`WHvX64PendingEventExtInt` — the same mechanism this
port uses — but only when `vcpu->ready_for_pic_interrupt` is set, and that flag is written in
exactly one place, the `WHvRunVpExitReasonX64InterruptWindow` handler, and cleared every pre-run.
**If the window never fires, QEMU cannot deliver a PIC interrupt at all in that configuration.**
It also ships a `whpx-apic` device that mirrors every LVT entry into the hypervisor's APIC page, so
LVT0 — the ExtINT gate — is state the hypervisor holds.

**QEMU does back away from the combination, but only on old hosts.** The gate is
`kernel_irqchip_allowed && !(whpx_is_legacy_os() && pic_enabled && !kernel_irqchip_required) && …`.
`whpx_is_legacy_os()` means the perfmon capability query failed — i.e. a pre-Windows-Server-2022
(build 20348) **host**, not a legacy guest. On a modern host the guard does not fire. (The `isapc`
machine type forces `kernel_irqchip_allowed = false` outright.)

**VirtualBox routes around the mechanism entirely.** Its whole notification/re-arm block sits inside
`if (!pVM->nem.s.fLocalApicEmulation)`, and the interrupt-force-flag path is gated the same way with
a separate `else`. Its emulated-APIC enablement is hard-disabled in trunk by an `&& 0` carrying
`/** @todo Fix issues in Hyper-V APIC backend before activating. */`. It injects ExtINT via
`WHvRegisterPendingEvent` and never calls `WHvRequestInterrupt` in that file at all. *(2-1 on the
gating claim; the `&& 0` is corroborating verifier evidence.)*

**Hyper-V forgets an armed notification across exits**, and VirtualBox re-writes the register on
every entry while a window is wanted. *(2-1, OBSERVED not documented.)* This does not rescue us:
the earlier measurement already re-armed 16,401 times, and VirtualBox only applies it when the APIC
is NOT emulated.

**`WHvRequestInterrupt` is not the alternative.** Neither VMM uses it for the legacy PIC. QEMU calls
it in exactly two roles — IPI (`ApicInitSipiTrap`) and MSI (`whpx_send_msi`); VirtualBox's NEM-win
file never calls it. *(3-0 on shipped usage. The stronger claim that the public `WHV_INTERRUPT_TYPE`
enum's missing values 7/8 are themselves documentation of the gap was REFUTED — see below.)*

**The pending-event register is an injection slot, not a queue.** Neither VMM ever writes an ExtINT
pending event while the VP is un-interruptible, and such an event does not clear a halted state —
QEMU clears `InternalActivityState.HaltSuspend` by hand. That matches probe P1 exactly, and this
port already does it, so it is not the missing piece.

**No shipped VMM uses a periodic forced cancel or timer heartbeat.** The workarounds that actually
ship are configuration-level fallback away from the emulated APIC, manual `HaltSuspend` clearing,
and unconditional re-arming (VirtualBox, non-emulated path only). No source measures the cost of
any of them. *(medium confidence; three heartbeat/doc-defect claims were refuted 0-3.)*

---

## 4. REFUTED — do not re-chase these

Of 25 claims verified adversarially, **13 were killed**. The attractive-but-false ones:

- That the missing `ExtINT` (7) and `LocalInt0` (8) values in `WHV_INTERRUPT_TYPE` are *documented*
  evidence of a structural gap. The enum reading was over-interpreted. **0-3.**
- That QEMU's docs record a concrete Windows-10 defect where a PIC interrupt fails to wake a halted
  guest under the in-hypervisor controller. **0-3.**
- That QEMU disables the emulated APIC by default and requires `-M q35,pic=off` to opt in. **0-3.**
- That a QEMU developer proposed a periodic "heartbeat" forced wake as the accepted workaround.
  **0-3.** There is no such accepted workaround in any source found.
- That the VirtualBox NEM-win path has moved to `target-x86/NEMR3Native-win-x86.cpp`. **Refuted** —
  the original path still exists (confirmed independently by a shallow clone here).

---

## 5. What this decides

The deliverability window cannot be part of this design. The legacy ExtINT path must find its own
way to notice that a blocked guest became ready, and the only signal available is **the guest's own
next exit**.

That makes the fix the cheap one: **drop the `ext_int_request` dedup while a vector is owed**, so
every boundary that finds the 8259's INT pin still high re-cancels the run rather than returning
early. It is bounded by the guest's own interrupt rate, needs no timer, and the machine already
republishes the level at every boundary. The alternative — a periodic forced exit — is what no
shipped VMM does, and this engine does not do it either.

**As built** (`ext_int_request` in `rusty_box_whp_engine/src/vcpu_thread.rs`), the dedup is
dropped at a narrower point than "owed":

- While a vector is owed and no staging has yet found the guest unable to take it, the dedup
  stands, and one cancel serves every republication of the pin.
- Once the pre-run staging finds the guest unable to take it (`IF` clear, an interrupt shadow, or a
  delivery in flight), `ext_int_blocked` is set, and each later republication cancels the run
  again until a staging finds the guest ready.
- A run is cancelled only while the processor is inside one.

Both other VMMs' behaviour is consistent with that reading: QEMU depends on a mechanism that does
not work here, and VirtualBox disabled the whole configuration citing unfixed backend issues.
