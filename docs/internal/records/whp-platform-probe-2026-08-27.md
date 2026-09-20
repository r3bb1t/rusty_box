# Windows Hypervisor Platform: measured answers

Produced by `cargo run --release -p rusty_box_whp --example whp_probe` on
2026-08-27. Every line below is a measurement, not a reading of the
documentation; re-run the probe to refresh it on another host.

These answers exist to unblock the WHP engine's signatures. Four of them
contradict what the plan assumed, and one closes a question the documentation
answers ambiguously.

"The plan" and "REPLAN" below mean the project owner's hypervisor plan, which
is kept outside this repository. Where this document cites one of its open
questions (§7 Q1–Q8), a decision or a risk, it also states what that item
asked or decided, so the plan itself is never needed to follow the argument.
For what was actually built, the code is the authority.

**Three entries below have since been superseded**, by
[the 2026-09-03 probe](whp-platform-probe-2026-09-03.md) on this same host: the
host table's extended-exit list, finding 3's claim that
`WHvRegisterInternalActivityState` cannot be written, and finding 10's
conclusion that direct injection is unreachable. Each carries a note at the
point where it is wrong. Everything else here stands, and the 2026-09-03 probe
re-ran all ten of these and reproduced them.

## The host

| | |
|---|---|
| features word | `0x00000000000002ff` |
| partial unmap | yes |
| local APIC emulation | yes |
| dirty page tracking | yes |
| idle suspend | yes |
| physical address width | 39 bits |
| extended exits offered | cpuid, msr, exception, rdtsc, apic_smi_trap, hypercall, gpa_access_fault |

**Superseded — the exits row was this port's vocabulary, not the host's answer.**
The host offers fifteen bit positions, `0x7fff`; on the day this table was
written `ExtendedVmExits` declared fields for only seven of them, and
`from_word` can report no bit it has no field for, so the six APIC traps the
host was already offering dropped out silently. Their names were transcribed
later, in commit `0e4974d`. `Capabilities` now keeps the raw
`WHV_EXTENDED_VM_EXITS` word beside the decode for exactly this reason, and
[the 2026-09-03 probe](whp-platform-probe-2026-09-03.md) prints both. The
correct row is there.

## Answers

### 1. A write to a read-only window exits, and `GpaAccessFaultExit` must stay OFF

A page mapped `Read|Execute` and written by the guest produces
`MemoryAccess gpa=0x10000 access=Write gpa_unmapped=false` with **no partition
property set**. Shadowed ROM and write-ignored PAM regions therefore work with
a plain `R|X` mapping.

**Correction to the plan.** The plan's §7 Q1 asked whether
`ExtendedVmExits.GpaAccessFaultExit` was *needed*, on the strength of OpenVMM
setting it. It is not needed, and it is actively harmful: with the bit set, the
first exit is `MemoryAccess gpa=0x1000 access=Execute gpa_unmapped=false` — an
**instruction fetch from a range mapped read, write and execute**. The bit
reports second-level faults the hypervisor would otherwise resolve itself, so
every page's first touch becomes a host round trip. It stays clear.

### 2. A trapped access cannot be finished without a decoder

| exit | RIP | platform advanced it | `instruction_length` | instruction bytes |
|---|---|---|---|---|
| write to a read-only window | `0x1005` | no | 0 | **none** |
| write to an unmapped range | `0x1007` | no | 0 | 16 bytes supplied |
| port write | `0x1003` | no | 1 | — |
| halt | `0x1001` | **yes** | 0 | — |

`WHvTranslateGva(0x10000)` with paging off returns `result_code=0
gpa=0x10000`, so the call works in real mode.

**This is the load-bearing finding.** For both memory exits, RIP still points
at the faulting instruction and the exit reports no length, so a host **must**
decode the instruction to make progress. That is exactly the plan's decision 5
— a shadow `BxCpuC` that decodes and finishes each trapped instruction — now
established as mandatory rather than preferred. Worse for
the read-only case: the exit carries no instruction bytes either, so the
decoder must fetch them from guest memory first. Port I/O is the exception the
plan expected — `instruction_length=1` means non-string PIO needs no decoder.

### 3. Injection alone resumes a halted processor

Under `LocalApicEmulationMode::None`, a guest that runs `sti; hlt` exits with
`Halt` at **rip past the `hlt`** and `InternalActivity { startup_suspend:
false, halt_suspend: false, idle_suspend: false }`. Writing
`WHvRegisterPendingInterruption` with vector `0x40` and re-entering runs the
real-mode handler, returns through `IRET`, and continues to the following
instruction — verified by both marker bytes the guest writes.

**Two corrections.** `HaltSuspend` is never set here, so there is nothing to
clear; and `WHvRegisterInternalActivityState` **cannot be written at all** on
this host — the attempt returns `0xC0350006` (`ERROR_HV_ACCESS_DENIED`). Any
design that plans to clear halt suspend must be abandoned, not merely skipped.

**Superseded — the refusal belongs to the mode, not to the host.** Both
observations above are real, and both are `LocalApicEmulationMode::None`'s: this
experiment used no emulated APIC, and that is the one mode in which the question
does not arise. [The 2026-09-03 probe](whp-platform-probe-2026-09-03.md) P1 ran
the same guest under `LocalApicEmulationMode::X2Apic` on this same host and
found `halt_suspend` **set** when a processor parks and the register **writable**
— and the clear is what wakes the parked processor. Its control B reproduces the
`0xC0350006` here under `None`, which is what identifies the difference as the
mode. So the last sentence is wrong as written: a design that plans to clear
halt suspend is sound wherever the hypervisor owns the APIC, and merely has
nothing to do where it does not.

### 4. An exit costs about 4 µs

| exit kind | per exit |
|---|---|
| port I/O | 4.02–4.16 µs |
| MMIO (unmapped range) | 4.30–4.40 µs |
| halt | 3.99–4.14 µs |

20 000 exits each, round trip out of and back into the guest with no host
register writes in between. The plan's risk 5 budgeted 2–20 µs against an estimated
2·10⁴–10⁵ exits per text-mode boot; at 4 µs that is 0.08–0.4 s of pure exit
cost, which the budget absorbs.

### 5. Mapping is cheap; a permission flip is cheaper; partial unmap works

- `WHvMapGpaRange` of 512 pages (2 MiB): **30 µs**.
- 2 000 permission flips of one page: **2.6 µs each**. The BIOS performs three
  PAM flips, so shadow-RAM routing costs microseconds per boot.
- A sub-range unmap of a larger mapping is **accepted**, matching the
  `PartialUnmap` capability bit.

### 6. `SeparateSecurityDomain` is free

MMIO exits cost 4.38 µs with a separate domain and 4.36 µs without — inside the
noise of interleaved best-of-two runs. The documented speedup is not
measurable here, so the mitigation costs nothing and the property stays on by
default. The plan's §7 Q6, which asked what `SeparateSecurityDomain` costs, is
closed.

### 7. CPUID is fully the host's to author, and properties are not uniformly pre-setup

With `CpuidExitList = [1, 0x40000000]` and `ExtendedVmExits.cpuid`, both leaves
exit at `instruction_length=2`, and the exit carries what the hypervisor would
have answered:

- leaf 1: `eax=0x000906a3 ebx=0x00000800 ecx=0x76da3203 edx=0x0f8bfbff`
- leaf 0x40000000: all zero — WHP exposes no hypervisor vendor leaf by default.

Control was demonstrated **in both directions**, which matters because this
host's own answer already has ECX bit 31 clear: answering with the bit set
makes the guest read `0xf6da3203`, and answering with it clear makes the guest
read `0x76da3203`. The stealth lever works.

**Ambiguity resolved.** The plan's §7 Q7 asked whether processor properties are
truly pre-setup-only, noting that the docs conflict. They are **per-property**:
after `WHvSetupPartition`, `ProcessorCount` is refused with `0x80070057`
(`E_INVALIDARG`) while `ExtendedVmExits` is **accepted**. A design may therefore
change its exit set on a live partition but not its processor count.

### 8. Cancellation is sticky

`WHvCancelRunVirtualProcessor` issued while the processor was **not** running
made the next `WHvRunVirtualProcessor` return `Canceled { reason: 0 }` after
11–22 µs, against a rescue cancel that would not have fired for 250 ms. The
platform latches the request. QEMU's acquire-check before every run remains
worth copying, but it guards a race the platform already handles.

### 9. Mappings are effectively unlimited, but not free at scale

8 192 separate single-page ranges all mapped and all unmapped again, with no
refusal. The cost per map grows with the number already mapped: **13.3 µs for
the first 512, 61.0 µs thereafter**.

A PC chipset needs a dozen ranges, so no surrogate process is required — the
shape Microsoft's Hyperlight adopts for its own workload does not apply here.
A design that mapped thousands of small ranges would pay for it.

### 10. Under `XApic`, a halted processor never exits — and that changes the threading model

This is the mode a guest with a real kernel needs, and it behaves nothing like
`None`.

| attempt | result |
|---|---|
| stop, then `WHvRegisterPendingInterruption` | **the run never returned** — the hypervisor parks a halted processor instead of exiting |
| `WHvRequestInterrupt`, Fixed vector 0x40, mid-run from another thread | handler did **not** run |
| `WHvRequestInterrupt`, NMI, mid-run from another thread | handler **RAN** |
| control: `None` + `WHvRequestInterrupt` | refused, `0xC0350008` `ERROR_HV_OPERATION_DENIED` |

Read together:

- **Direct injection is unreachable in this mode.**
  `WHvRegisterPendingInterruption` is a register write, which requires a
  stopped processor; under `XApic` a halted processor never stops. The route
  that works for DLX does not exist for Alpine.
- **Mid-run delivery does work.** The NMI is the proof, and it has to be: a
  `Fixed` vector is gated by the guest's own APIC software-enable, which a
  real-mode guest never sets, so the Fixed failure says nothing about the
  platform. An NMI is not gated by that bit. It woke the parked processor and
  its handler ran. (The run then parked again at the guest's *second* `hlt`,
  which is why it still needed the rescue — that is the same finding twice,
  not a failure.)
- The control shows the refusals mean something: without an APIC the same call
  is denied outright.

**Superseded — the first bullet's reasoning does not hold.** What this
experiment observed is exact: the run never returned on its own, so the
processor never stopped *by itself*. The step from there to "direct injection is
unreachable" assumes a halted processor cannot be stopped at all, and it can —
by a cancel from another thread.
[The 2026-09-03 probe](whp-platform-probe-2026-09-03.md) measures the whole
route under `LocalApicEmulationMode::X2Apic`, the sibling emulated-APIC mode:
P3 shows the cancel is what ends a parked run, P4 shows a cancel cannot lose the
race against the run beginning, and P1 then performs two register writes on the
stopped processor — a pending ExtINT and a cleared `halt_suspend` — and the
guest resumes past its halt and runs the handler. Injection through a register
write is therefore reachable on a parked processor, contrary to the bullet.

The measurement was taken under `X2Apic`, not under the `XApic` this finding
used, and the route was not re-run there. So what is established is that the
*argument* fails; the `XApic` conclusion is untested rather than disproved, and
should not be cited as a platform limit. The threading consequence below is
unaffected either way — it follows from the halted processor not exiting, which
still stands.

**The consequence for the design is bigger than the answer.** Under `XApic` the
machine thread sits inside `WHvRunVirtualProcessor` for as long as the guest
runs, and a halted guest never gives it back. So the timer wheel and the device
models cannot live on that thread: nothing would tick, no IRQ0 would ever be
raised, and the guest would idle forever. `XApic` therefore forces **a second
thread** — either one that owns the devices and delivers through
`WHvRequestInterrupt`, or one that periodically cancels the run to hand the
machine thread back.

`LocalApicEmulationMode::None` has no such problem, because a halt exits. It is
also the stealth target, our `cpu/apic.rs` already exists, and the `IrqFabric`
(`rusty_box/src/iodev/irq.rs`) already owns EOI. **That argues for taking
Alpine straight to the plan's stage 2 — `None` plus our own LAPIC behind the
shadow CPU — rather than through the `XApic` stage it sequences first.** That
first stage is viable, but it buys a threading model that the plan's
decision 9 (one machine thread with two cross-thread pokes) spent effort
avoiding.

**As built,** the engine took the second thread after all, and with it both
modes. `choose_the_local_apic` in `rusty_box_whp_engine/src/engine.rs`
chooses by device clock:

- When the machine's devices run in ticks, it asks for `None`.
- When a device thread drives them on host time, it asks for the hypervisor's
  `X2Apic`, falling back to `XApic`. The vCPU thread
  (`rusty_box_whp_engine/src/vcpu_thread.rs`) is then the thread that stays
  inside the run.

## What this changes

| Plan item | Status |
|---|---|
| §7 Q1 `GpaAccessFaultExit` needed for read-only windows | **Refuted** — not needed, and harmful |
| §7 Q2 instruction bytes / `WHvTranslateGva` in real mode | Answered; translation works, bytes are absent for the case that needs them most |
| §7 Q3 clearing `HaltSuspend` | **Refuted** — never set, and unwritable. *Superseded:* true only under `LocalApicEmulationMode::None`; under `X2Apic` the bit is set and the register is writable, and clearing it is what wakes a parked processor ([2026-09-03](whp-platform-probe-2026-09-03.md) P1) |
| §7 Q4 per-exit latency | 4 µs, inside the assumed range |
| §7 Q5 map cost and `PartialUnmap` | Answered; both cheap |
| §7 Q6 `SeparateSecurityDomain` gain | **Closed** — no measurable cost either way |
| §7 Q7 CPUID authorship and property timing | Answered; properties are per-property, not uniformly pre-setup |
| §7 Q8 cancel stickiness | Answered — sticky |
| Decision 5, the shadow CPU | **Promoted from preferred to mandatory** by finding 2 |
| Decision 6, `XApic`-first staging for Alpine | **Questioned** by finding 10 — viable, but it forces a second thread that `None` does not |
| Decision 9, one thread and two cross-thread pokes | **Three under `XApic`**: stop flag, waker, and interrupt delivery. Unchanged under `None` |
