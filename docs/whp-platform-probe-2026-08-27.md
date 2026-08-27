# Windows Hypervisor Platform: measured answers

Produced by `cargo run --release -p rusty_box_whp --example whp_probe` on
2026-08-27. Every line below is a measurement, not a reading of the
documentation; re-run the probe to refresh it on another host.

These answers exist to unblock the WHP engine's signatures. Four of them
contradict what the plan assumed, and one closes a question the documentation
answers ambiguously.

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

## Answers

### 1. A write to a read-only window exits, and `GpaAccessFaultExit` must stay OFF

A page mapped `Read|Execute` and written by the guest produces
`MemoryAccess gpa=0x10000 access=Write gpa_unmapped=false` with **no partition
property set**. Shadowed ROM and write-ignored PAM regions therefore work with
a plain `R|X` mapping.

**Correction to the plan.** REPLAN §7 Q1 asked whether
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
decode the instruction to make progress — which is exactly REPLAN decision 5's
shadow `BxCpuC`, now established as mandatory rather than preferred. Worse for
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

### 4. An exit costs about 4 µs

| exit kind | per exit |
|---|---|
| port I/O | 4.02–4.16 µs |
| MMIO (unmapped range) | 4.30–4.40 µs |
| halt | 3.99–4.14 µs |

20 000 exits each, round trip out of and back into the guest with no host
register writes in between. REPLAN risk 5 budgeted 2–20 µs against an estimated
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
default. REPLAN §7 Q6 is closed.

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

**Ambiguity resolved.** REPLAN §7 Q7 asked whether processor properties are
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

## What this changes

| Plan item | Status |
|---|---|
| §7 Q1 `GpaAccessFaultExit` needed for read-only windows | **Refuted** — not needed, and harmful |
| §7 Q2 instruction bytes / `WHvTranslateGva` in real mode | Answered; translation works, bytes are absent for the case that needs them most |
| §7 Q3 clearing `HaltSuspend` | **Refuted** — never set, and unwritable |
| §7 Q4 per-exit latency | 4 µs, inside the assumed range |
| §7 Q5 map cost and `PartialUnmap` | Answered; both cheap |
| §7 Q6 `SeparateSecurityDomain` gain | **Closed** — no measurable cost either way |
| §7 Q7 CPUID authorship and property timing | Answered; properties are per-property, not uniformly pre-setup |
| §7 Q8 cancel stickiness | Answered — sticky |
| Decision 5, the shadow CPU | **Promoted from preferred to mandatory** by finding 2 |
