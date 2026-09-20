# Windows Hypervisor Platform: the VMM-shape questions, measured

Produced by `cargo run --release -p rusty_box_whp --example whp_probe` on the
same host as [the 2026-08-27 probe](whp-platform-probe-2026-08-27.md). Every
line below is a measurement; re-run the probe to refresh it on another host.

The experiments were first run on 2026-09-03, run again on 2026-09-05 when P2's
register attempts were added, and run a third time the same day once two of the
probe's own answer texts were corrected. **The tables below are that third run,
whose complete unedited output is reproduced at the end of this document**, so
every figure here can be traced to a line of it. All three reached the same
answer in every section; where they differ it is in durations and in the
free-running counters of P8, never in an outcome. The third run was taken while
another project's build occupied this machine, and its durations run two to
three-and-a-half times the ones
[the 2026-08-27 probe](whp-platform-probe-2026-08-27.md) published for the same
experiments — see "Host conditions" below, and note that no answer here rests on
a duration. The first two runs' logs were not kept, which is why
this document reproduces the third's in full.

These nine questions are the ones the VMM-shape design rests on. That design is
the project owner's plan for this engine, and it is kept outside this
repository. Where an answer below says what it changes, it names the code that
carries the change (the vCPU thread's run loop, the legacy 8259 path, a
conversion of the APIC state page) rather than a plan task, and the code is
the authority on what was built.

Two of the nine questions decide the shape of later work rather than merely
informing it, and both are answered here without hedging:

- **P1 says a parked processor CAN be woken for a legacy interrupt**, so the
  legacy 8259 path (a host-placed ExtINT) stands as designed.
- **P3 says a halt produces NO exit under an emulated APIC**, so the run
  loop's halt arm is unreachable in that mode, and the machine thread has to
  be taken back by force.

Three answers contradict what was written down before. One contradicts the
earlier probe document, which recorded a mode-specific refusal as a fact about
the host. One contradicts this probe's own first LINT0 attempt, whose control
turned out not to be a control. One contradicts **this document's own first
draft**, which concluded from a single refused register that the APIC state page
was the only way to write an offloaded APIC; the two registers that carry what
the page cannot are both writable. See P2.

## The host

| | |
|---|---|
| CPU | 12th Gen Intel Core i5-12450H, 8 cores / 12 logical |
| OS | Windows 11 Home Single Language, build 26200 |
| features word | `0x00000000000002ff` |
| partial unmap | yes |
| local APIC emulation | yes |
| dirty page tracking | yes |
| idle suspend | yes |
| physical address width | 39 bits |
| **extended-exits word, raw** | **`0x0000000000007fff`** — fifteen bit positions offered |
| extended exits this port names | thirteen of the fifteen: cpuid, msr, exception, rdtsc, apic_smi_trap, hypercall, apic_init_sipi_trap, apic_write_lint0_trap, apic_write_lint1_trap, apic_write_svr_trap, apic_write_ldr_trap, apic_write_dfr_trap, gpa_access_fault |
| offered and unnamed by this port | `0x0000000000000c00` — bits 10 and 11, `UnknownSynicConnection` and `RetargetUnknownVpciDevice` |
| processor features | `0x3e1bfbcfe7f7859f` |
| synthetic features (bank 0) | `0x0000006fff448fff` |
| processor clock | 2 496 000 499 Hz |
| interrupt clock | 200 000 000 Hz |
| TSC-deadline timer | yes |

**The exits row corrects the earlier document rather than the host, and the raw
word is why the correction is checkable.** The 2026-08-27 table lists seven
exits and omits the five APIC traps and the INIT/SIPI trap. The host has not
grown them: the *port* had not yet named those bit positions.
`ExtendedVmExits`'s `apic_write_*` and `apic_init_sipi_trap` fields were first
transcribed in commit `0e4974d`, which landed after that probe ran, so the
earlier list was the shape of `from_word` and not the shape of the host's
answer.

A decoded list can only ever report the bits its decoder has fields for, so a
list on its own can never say whether an absent exit was absent from the host.
`Capabilities` therefore keeps the whole `WHV_EXTENDED_VM_EXITS` word beside the
decode, and the probe prints both. The two numbers above are what make the
thirteen checkable: the host offers **fifteen**, this port names thirteen, and
the two it does not name are visible as `0xc00` rather than silently gone. That
is the failure mode of 2026-08-27 caught at the source instead of re-diagnosed
later.

### Host conditions

This machine was shared with another project's builds throughout. The condition
is recorded because one class of number depends on it and the rest do not:

| | |
|---|---|
| host CPU utilisation, sampled immediately after the run | 84 % |
| build processes present (cargo / rustc / cl / link) | 12 |
| scheduling delay measured by the probe itself, during the run | 406.43 µs mean overshoot of ten 5 ms sleeps |
| available parallelism | 12 |

**No answer below rests on a duration.** Each rests on a column the probe records
directly: the exit reason, and the `rescued` flag, which says whether a run ended
on its own or had to be cancelled from another thread. The load moves the
durations and moves them a long way — every cost in §"the earlier ten" is between
two and three-and-a-half times what the same probe published on 2026-08-27, and
the mapping-churn figure fourteen times. What load cannot move is *whether the run
returned at all*, which is what each answer turns on. No figure in this document
is offered as a *latency* of the platform.

## Answers

### P1. A parked processor CAN be woken, and clearing the halt suspend is what does it

**The legacy interrupt path stands.** Research said Windows 10 hosts
fail this and Windows 11 hosts pass; this host passes.

Guest: `sti; hlt; mov byte [RESUMED],0x11; hlt`, with a real-mode handler on
vector 0x20 that stores `0xA5` and `iret`s. Partition:
`LocalApicEmulationMode::X2Apic`. Between the two runs the host writes
`WHvRegisterPendingEvent` with an ExtINT for vector 0x20, then
`WHvRegisterInternalActivityState` with `halt_suspend: false`.

| run | exit reason | elapsed | rescued | handler ran | rip | len |
|---|---|---|---|---|---|---|
| 1: `sti; hlt` | `Canceled(0)` | 256.6 ms | yes | no | `0x1002` | 0 |
| 2: after the host's two writes | `Canceled(0)` | 248.8 ms | yes | **YES** | `0x1008` | 0 |

- activity once run 1 ended: `InternalActivity { startup_suspend: false, halt_suspend: true, idle_suspend: false }`
- `WHvRegisterPendingEvent` (ExtINT 0x20): **accepted**
- `WHvRegisterInternalActivityState` (`halt_suspend=false`): **accepted**
- the guest reached the store past its first `HLT`: **yes**

Only `halt_suspend` differs from the running state in that activity reading, so
the write that follows moves exactly one bit. `idle_suspend` — a feature this
host offers — is already clear, which is what makes "clearing the halt suspend
is what does it" exact rather than merely correlated.

Run 2 still ends at the rescue because the guest halts a *second* time after its
handler returns, and a second halt parks exactly like the first. That is the
same finding twice, not a failure — the marker byte and the resumed-past-the-halt
byte are the measurement.

**Control A — the same, with the pending event placed and NO activity write:**

| run | exit reason | elapsed | rescued | handler ran | rip | len |
|---|---|---|---|---|---|---|
| 1: `sti; hlt` | `Canceled(0)` | 246.9 ms | yes | no | `0x1002` | 0 |
| 2: after the pending-event write alone | `Canceled(0)` | 261.3 ms | yes | **no** | `0x1002` | 0 |

- activity once run 1 ended: `InternalActivity { startup_suspend: false, halt_suspend: true, idle_suspend: false }`
- `WHvRegisterPendingEvent` (ExtINT 0x20): **accepted**
- `WHvRegisterInternalActivityState`: **not attempted (control)**
- the guest reached the store past its first `HLT`: **no**

Those three lines are the control, not the table above them: they are what show
that control A differs from the main case in exactly one variable. The pending
event was placed and accepted here too, and the run still came back with RIP
standing at `0x1002` — the same address it parked at the first time. So the
pending event alone is *not* enough. Clearing the halt suspend is the operative
step.

**Control B — `LocalApicEmulationMode::None`:**

| run | exit reason | elapsed | rescued | handler ran | rip | len |
|---|---|---|---|---|---|---|
| 1: `sti; hlt` | `Halt` | 42.9 ms | no | no | `0x1002` | 0 |
| 2: after the host's writes | **`InvalidVpRegisterValue`** | 48.2 ms | no | no | `0x1002` | 0 |

- activity once run 1 ended: `InternalActivity { startup_suspend: false, halt_suspend: false, idle_suspend: false }`
- `WHvRegisterPendingEvent` (ExtINT 0x20): **accepted**
- `WHvRegisterInternalActivityState`: **refused, `0xC0350006`** (`ERROR_HV_ACCESS_DENIED`)

**Correction to the 2026-08-27 document.** Its finding 3 says `HaltSuspend` "is
never set here, so there is nothing to clear", and that
`WHvRegisterInternalActivityState` "**cannot be written at all** on this host",
concluding that "any design that plans to clear halt suspend must be abandoned,
not merely skipped". Both statements are true only of
`LocalApicEmulationMode::None`. Under `X2Apic`, on the same host, the bit **is**
set when a processor parks and the register **is** writable. The refusal was
mode-specific and was recorded as a property of the host.

**A second thing control B settles, which was not asked.** Under `None` the
pending-event ExtINT is accepted by the register write and then rejected by the
run itself, with `InvalidVpRegisterValue`. Since the activity write was refused
in that arm, it changed nothing, and the pending event is the only state that
differs between its two runs — so the rejection is attributable to it.
`WHvRegisterPendingEvent`'s ExtINT member is usable only where the hypervisor
emulates an APIC; with no APIC, the route is `WHvRegisterPendingInterruption`
(2026-08-27 finding 3), not this one.

**What it changes.** The legacy ExtINT path keeps its designed shape. The wake
protocol, measured end to end, is:

1. stop the processor;
2. write the ExtINT into `WHvRegisterPendingEvent`;
3. clear `halt_suspend`;
4. re-enter.

The steps must run in that order, and both writes require a stopped processor.
That is the order the vCPU thread's pre-run staging follows
(`rusty_box_whp_engine/src/vcpu_thread.rs`). QEMU's
`whpx_vcpu_kick_out_of_hlt` workaround is not needed on this host.

### P2. The state page round-trips exactly — and it is NOT the only write path

An `X2Apic` processor's page, read → written back → read again. The processor
had been created and never run — no guest instruction had executed — so
everything below stands at the platform's reset state; that is the only
condition this section was measured under.

| | |
|---|---|
| first KiB identical | **yes** |
| whole 4 KiB identical | **yes** |
| version register | `0x00050014` |
| spurious register | `0x000000FF` |
| destination format | `0xFFFFFFFF` |

Then the named registers, each read → written → read again on that same
processor — stopped, and still never run — so an accepted-and-ignored write is
distinguishable from an applied one:

| register | before | written | write | after | took? |
|---|---|---|---|---|---|
| `WHvX64RegisterApicTpr` | **refused**, `0xC0350005` | `0x20` | **refused, `0xC0350005`** | **refused**, `0xC0350005` | — |
| `WHvX64RegisterApicBase` | `0x00000000FEE00900` | `0x00000000FEE00D00` | **accepted** | `0x00000000FEE00D00` | **yes** |
| `WHvX64RegisterCr8` | `0x0000000000000000` | `0x0000000000000002` | **accepted** | `0x0000000000000002` | **yes** |

Both accepted writes asked for a value other than the one already standing, so
the read-back proves something: a write of the value already there would agree
with itself whatever the platform did with it.

- `0xFEE00900` is the architectural APIC base `0xFEE00000` with the BSP bit (8)
  and the global enable (11) set. The write adds the x2APIC enable (bit 10) —
  the one transition the architecture permits from any starting state, chosen
  deliberately so that a refusal could not be a refusal of the *value* dressed
  up as a refusal of the *register*. It was not refused, and the read-back
  carries it.
- `CR8` carries the task-priority *class*, the top four bits of the TPR. It
  moved from 0 to 2 and read back as 2.

`0xC0350005` is `ERROR_HV_INVALID_PARAMETER`, not the `ACCESS_DENIED` Hyperlight
reports for these on its hosts — a different refusal. Note that the TPR refusal
is symmetric: the register cannot be **read** either.

**Correction to this document's own first draft.** It concluded, from the TPR
refusal alone, that "the state page is the only way in or out of an offloaded
APIC here", so that "whatever the page does not carry is lost". That was a
universal drawn from one sample and it is false. `ApicRegister` declares no
task-priority field and no APIC-base field, so the page carries neither — and
both of the registers that do carry them are writable while the hypervisor owns
the APIC.

**What it changes.** A conversion between the hypervisor's APIC state page and
this port's `BxLocalApic` (not built yet) has an escape hatch, and it carries
two things: the guest's `IA32_APIC_BASE` and the top four bits of its task
priority.
It needs the first — the APIC base, with its APIC-enable and x2APIC-enable bits,
has to cross the seam somehow and no word of the page can carry it. It crosses by
name, through `WHvX64RegisterApicBase`, which the port already exchanges
(`ALL_REGS`, and the engine's own MSR batch). The task-priority *class* —
`TPR[7:4]`, which is all `CR8` holds — crosses through `CR8`, likewise already
exchanged. Neither route carries `TPR[3:0]`, and neither does the page.

Claim nothing wider than the three names above: they are what was attempted. The
SDK advertises further APIC register names and this host's answer for them is
unmeasured. What **is** established is that the refusal is not a blanket policy
against writing APIC state on an offloaded APIC: of the three names attempted,
two were accepted and one was refused. `WHvX64RegisterApicTpr` specifically is
refused in both directions; `WHvX64RegisterApicBase` and `CR8` are both accepted
and both read back the value written. Whether any register not attempted here
shares the TPR refusal is unmeasured, and nothing above should be read as a
claim about one. Note that `CR8` is an architectural control register rather than
an APIC-register name, which is the likeliest reason it goes through where
`ApicTpr` does not — but `WHvX64RegisterApicBase` carries the same `Apic` prefix
as `ApicTpr` and went through as well, so the two names that were accepted do not
divide from the one that was refused along that line alone. And since `CR8` holds
only the priority class, the low four bits of the guest's TPR are carried by
neither the page nor `CR8`.

### P2b. A software-disabled APIC DROPS a requested vector, silently

The stronger of the two readings the earlier document left open. Vector 0x43,
requested into an `X2Apic` processor that has never run:

| step | call returned | request bitmap (word 0 … word 7) |
|---|---|---|
| request with the APIC software-**disabled** | **ACCEPTED** | `00000000 00000000 00000000 …` |
| (no request) after the host sets the enable bit | — | `00000000 00000000 00000000 …` |
| request with the APIC software-**enabled** | ACCEPTED | `00000000 00000000 00000008 …` |

Spurious register at reset `0x000000FF`; after the host sets bit 8,
`0x000001FF`. `0x43` is bit 3 of word 2, which is exactly where the second
request lands.

So the vector is **dropped**, not latched: the bitmap does not fill in when the
APIC is later enabled, and the call reports success either way. The
2026-08-27 document's wording — a fixed vector is "gated by" the software
enable — is consistent with latching, and latching is not what happens.

**What it changes.** **A page conversion must carry the spurious register's
enable bit, or restoring a page silently disables the guest's APIC** and every
vector delivered afterwards is discarded with a success return. This is also
why every experiment in this document that needs a vector to land sets the
enable bit from the host first: otherwise this answer becomes their unstated
premise.

### P3. A halt produces NO exit under an emulated APIC

Guest `cli; hlt` — `cli` at `0x1000`, `hlt` at `0x1001` — run with a 200 ms
rescue cancel.

| partition | exit reason | elapsed | rescued | rip | len |
|---|---|---|---|---|---|
| `X2Apic` | **`Canceled(0)`** | 253.7 ms | **yes** | `0x1002` | 0 |
| `None` (control) | `Halt` | 30.8 ms | no | `0x1002` | 0 |

**The processor genuinely reached the halt.** A rescue timeout on its own is an
absence of evidence — it cannot tell "no halt exit" from "the processor never
got there". Two things close that gap, and neither is the timeout:

1. **The RIP the cancelled run came back with is `0x1002`**, at or past the
   `HLT` at `0x1001`. The processor executed the instruction and parked in it,
   so the absent exit is an absent exit rather than an unreached instruction.
2. **P1 is the positive control this experiment does not itself contain.** Under
   the same `X2Apic` mode, a guest parked in `sti; hlt` resumed *past* its halt
   once the host cleared `halt_suspend`, writing the `RESUMED` byte from the
   instruction after it. A processor that never reached a halt cannot resume
   past one. That is direct evidence of execution, not an inference from
   silence.

Between them P1 and P3 also cover both `IF` states — `cli; hlt` and `sti; hlt`
both park — which is worth saying out loud, since an unwakeable halt is exactly
the case a platform might special-case into an exit.

The cross-mode control carries the rest of the argument: the same guest, the same
rescue, and — when the hypervisor owns no APIC — a `Halt` that the run returns by
itself, with `rescued` reading false. Only `local_apic` differs, and `cli; hlt`
does not execute differently because a
hypervisor is emulating an APIC. It also reproduces 2026-08-27 finding 10, which
saw the same parking under `XApic` with a different guest.

**What it changes.** The run loop's `Halt` arm is **unreachable** whenever the
partition has an emulated APIC. A run loop that waits for `ExitReason::Halt` to
notice an idle guest will wait forever. Idleness has to be noticed from outside
the run, and together with P4 this makes the cancel the machine thread's only
way back. The vCPU thread (`rusty_box_whp_engine/src/vcpu_thread.rs`) turns a
`Halt` arriving there into an engine fault, `X64Halt under the hypervisor
APIC`, and the comment on that arm describes this measurement, so an occurrence
can be diagnosed rather than being a mystery.

### P4. A cancel is sticky under X2Apic too

`WHvCancelRunVirtualProcessor` issued while the processor was **not** running,
then a guest of `jmp $` which exits for nothing:

| run | exit reason | elapsed | rescued | rip |
|---|---|---|---|---|
| spin, after a cancel issued while stopped | `Canceled(0)` | **44.5 ms** | **no** | `0x1000` |

`rescued` reads false, and that is the measurement: the guest is `jmp $`, which
exits for nothing, so without the earlier cancel this run ends only when the
rescue fires at 200 ms — and here it ended on its own. The platform latches the
request, exactly as 2026-08-27 finding 8 measured under `None` (which this run
reproduces, ending its own spin in 28.9 µs against a 250 ms rescue).

**What it changes.** Together with P3 this closes the run-loop shape: a device
thread that wants the machine thread back issues a cancel, and it cannot lose
the race — a cancel issued a moment before the run begins is still seen. The
acquire-check every reference VMM performs before entering a run guards a race
this platform already handles.

### P5. The LINT0 write traps; a host-placed ExtINT is NOT gated by it

Partition `X2Apic` with `ExtendedVmExits { apic_write_lint0_trap: true }` — the
host advertises the bit. The guest writes LVT0 through the memory-mapped page
with `mov [ds:0x0350], eax`, `DS` given base `0xFEE00000` directly (a base no
real-mode selector can express; the platform writes the segment cache, so it is
accepted). The host software-enables the APIC before each run, without which a
disabled APIC forces every LVT mask bit and the two attempts would be the same
case twice.

**Both attempts in full**, three runs each. The `handler` column reads the
marker byte at the end of every run and the byte persists, so it means "the
marker is present as of this run" — which is why runs 1 and 2 matter: they read
`false`, and that is what makes run 3's `YES` attributable to run 3.

*LINT0 masked — the guest writes `0x00010700`:*

| run | exit reason | elapsed | rescued | handler | rip | len |
|---|---|---|---|---|---|---|
| 1: write LINT0, then `sti; hlt` | `ApicWriteTrap(Lint0=0x10700)` | 40.0 ms | no | no | `0x100a` | 0 |
| 2: `sti; hlt` | `Canceled(0)` | 277.2 ms | yes | no | `0x100c` | 0 |
| 3: after the host places an ExtINT | `Canceled(0)` | 262.4 ms | yes | **YES** | `0x1012` | 0 |

- LINT0 in the state page afterwards: `0x00010700`
- pending-event write **accepted** / activity write **accepted**

*LINT0 unmasked (the control) — the guest writes `0x00000700`:*

| run | exit reason | elapsed | rescued | handler | rip | len |
|---|---|---|---|---|---|---|
| 1: write LINT0, then `sti; hlt` | `ApicWriteTrap(Lint0=0x700)` | 48.6 ms | no | no | `0x100a` | 0 |
| 2: `sti; hlt` | `Canceled(0)` | 267.7 ms | yes | no | `0x100c` | 0 |
| 3: after the host places an ExtINT | `Canceled(0)` | 261.7 ms | yes | **YES** | `0x1012` | 0 |

- LINT0 in the state page afterwards: `0x00000700`
- pending-event write **accepted** / activity write **accepted**

**The trap.** The store begins at `0x1006` and is four bytes long, so the
`rip=0x100a` both attempts report is **past** it: **an APIC write trap arrives
with RIP already advanced**, while reporting `instruction_length = 0`. That is
the opposite of the memory-access exits in 2026-08-27 finding 2, where RIP still
points at the faulting instruction. A trap handler must **not** advance RIP by
recomputing the end of the store from a decode — it would skip the following
instruction. (Adding the reported length is harmless, because the reported
length is zero; it is the recomputation that overshoots.)

The trapped value is also **applied**: the state page reads back `0x00010700`
and `0x00000700` respectively. The trap notifies; it does not withhold.

**The gating answer: a host-placed pending ExtINT is NOT gated by LINT0.** It
reached the guest with the entry masked exactly as with it unmasked. QEMU and
OpenVMM inject without consulting LVT0, and this platform gives them no reason
to. The platform also makes this the only route there is to test:
`InterruptKind` offers Fixed, LowestPriority, Nmi, Init, Sipi and LocalInt1, and
no ExtINT delivery kind for `WHvRequestInterrupt` — so `WHvRegisterPendingEvent`
is not one of several ways a host can place an ExtINT, it is the way, and "not
gated" covers the whole surface the legacy path can use.

**A correction to this probe's own first attempt.** The first run of this
experiment did not software-enable the APIC, and reported "not gated" off a
comparison that was not one: with the APIC software-disabled the emulated APIC
**forces the mask bit of every LVT entry**, so the guest's unmasked write read
back as `0x00010700` — the same value as the masked case. Both attempts were
the masked case. The experiment now enables the APIC from the host first, and
the two attempts leave genuinely different values standing; the answer survived,
but it was not evidence until it did. The probe carries a `really_differed`
check so the confounded shape cannot be reported as an answer again.

**What it changes.** The fabric's LVT0 tracking is a **correctness
requirement**, not a fidelity nicety: the platform will deliver a masked
ExtINT, so the fabric must consult LINT0 itself before placing one. The code
does so in three places:

- `IrqFabric::set_bsp_lint0` and `lint0_admits_ext_int`
  (`rusty_box/src/iodev/irq.rs`) keep the fabric's copy of LINT0 and consult
  it before placing an ExtINT;
- the vCPU thread feeds that copy from this trap without advancing RIP;
- two tests pin the gate: `lvt0_gates_the_legacy_path_the_way_the_lapic_would`
  (`irq.rs`) and the gated `a_masked_lvt0_keeps_the_legacy_path_closed`
  (`rusty_box_whp_engine/src/lib.rs`).

The trap's already-advanced RIP is a hazard its handler must know about.

### P6. The synthetic bank is accepted whole, and the guest sees Hv#1

`SyntheticFeatures::OPENVMM_VTL0` was accepted **whole** — the host's bank
(`0x0000006fff448fff`) is a superset of what the composite asks for
(`0x000000007f448fff`), and the difference is zero. The host also sets
`0x0000006f80000000` worth of bits this port's header does not name, which
`from_bits_retain` keeps rather than truncating.

Guest CPUID, executed on the hardware and read back through a port exit:

| leaf | eax | ebx | ecx | edx |
|---|---|---|---|---|
| `0x40000000` | `0x40000006` | `0x7263694d` | `0x666f736f` | `0x76482074` |
| `0x40000001` | `0x31237648` | 0 | 0 | 0 |
| `0x40000003` | `0x00000e7f` | `0x00088020` | `0x00000002` | `0x000ec1b0` |

Which reads as vendor `"Microsoft Hv"`, interface signature `"Hv#1"`, and a
feature leaf with **hypercall MSRs (bit 5) set and VP index (bit 6) set** —
`0xe7f` is bits 0–6 and 9–11. A Linux guest that ignores the leaves without
those two bits will not ignore these.

### P7. Both clocks agree with what the guest is told

| | capability answer | guest's enlightened MSR | |
|---|---|---|---|
| processor clock | 2 496 000 499 Hz | `HV_X64_MSR_TSC_FREQUENCY` (0x40000022) = 2 496 000 499 Hz | **agree** |
| interrupt clock | 200 000 000 Hz | `HV_X64_MSR_APIC_FREQUENCY` (0x40000023) = 200 000 000 Hz | **agree** |
| TSC-deadline timer | supported | | |

Both MSRs answered rather than faulting — the guest ran with a `#GP` handler
installed on vector 13 precisely so a refusal would be visible, and it was not
taken. The APIC bus clock is exactly 200 MHz, which is the number an APIC-timer
model must divide down from.

### P8. Suspending partition time freezes the guest's TSC

Partition `None`, so the guest's halt produces an exit and the processor is
stopped where the clock can be read.

| observation | guest TSC | partition reference (100 ns) |
|---|---|---|
| after the halt | 23 904 073 | 1 584 |
| +50 ms, partition time **running** | 177 522 123 | 617 174 |
| partition time suspended | 177 675 734 | 617 486 |
| +50 ms, partition time **suspended** | **177 675 734** | **617 486** |
| resumed, and the guest ran again | 177 815 490 | 618 450 |

The TSC moved **153 618 050** counts across the running window and **0** across
the suspended one. Both clocks hold still, and both pick up afterwards. Strictly
what is measured is that no *reader* can see the clock move while partition time
is suspended; no guest can observe the difference between a frozen clock and a
frozen read while its processor is stopped, so the two readings are equivalent
for every use the design has.

**The control is the part worth keeping.** The TSC advances while the processor
is merely *stopped* — 153 million counts across a window in which no guest
instruction executed. A host that wants a guest not to see its own downtime must
**suspend the clock**, not merely refrain from running. This extends to the
TSC, the clock a guest actually reads, what
`the_partitions_reference_clock_starts_with_the_guest_and_a_suspend_holds_it`
(`rusty_box_whp/src/partition.rs`) pins for the partition's reference clock:
it reads zero until a processor has run, and a suspend holds it still.

### P9. In-service and trigger-mode are NOT swapped — pinned twice, asymmetrically

The order of these two bitmaps survives every symmetric test, because a
conversion that reads and writes through one wrong mapping agrees with itself.
Two observations were taken in which the **platform** treats the two fields
differently. They agree.

Vector `0x41` is bit 1 of word 2, so the predicted word is
`… 00000002 …` in position 2.

**Probe one — trigger mode.** The trigger-mode bit is set on acceptance into the
request bitmap, so a level delivery marks it with the processor never having run
and an edge delivery of the same vector does not:

| request | request bitmap | in-service | trigger-mode |
|---|---|---|---|
| level-triggered `0x41` | `… 00000002 …` | all zero | **`… 00000002 …`** |
| edge-triggered `0x41` | `… 00000002 …` | all zero | all zero |

Under the swapped hypothesis word 13 would be in-service, and a
requested-but-never-accepted vector is by definition not in service, so the swap
predicts it clear in both rows. It was set in one.

**Probe two — acceptance.** A guest spinning with interrupts enabled (not
halting, so this depends on nothing P1 also measures) accepts the vector and
stops **inside its handler**, before any end-of-interrupt:

| | request bitmap | in-service | trigger-mode |
|---|---|---|---|
| accepted, not acknowledged | all zero | **`… 00000002 …`** | all zero |

Under the swapped hypothesis word 5 would be trigger-mode, which an **edge**
delivery must leave clear. It was set. Refuted again, by a different mechanism.

The vector left the request bitmap and appeared in the field the layout calls
in-service; the field the layout calls trigger-mode stayed clear for an
edge-triggered delivery and was set for a level-triggered one. **The two are the
right way round.** `ApicVector::InService` at word 5 and
`ApicVector::TriggerMode` at word 13 stand as transcribed, now by measurement.
Words 5..29 hold exactly three eight-word bitmaps. `Request` is already pinned
at word 21, by `a_requested_vector_appears_in_the_pages_request_bitmap`
(`rusty_box_whp/src/partition.rs`). With `Request` fixed, only two orderings
remain, and both observations select the same one. There is no third field the
bit could belong to.

**The whole page, as the platform left it after the acceptance** — recorded so
what is still unpinned is on the record as data rather than as prose:

```
words  0..: 00000000 00050014 00000000 ffffffff 000001ff 00000000 00000000 00000002
words  8..: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
words 16..: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
words 24..: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
words 32..: 00010000 00010000 00010000 00010000 00010000 00010000 00000000 00000000
words 40..: 00000000 00000000 00000000 00000000
```

Word 4 is `0x000001ff`, the spurious register with the software enable the probe
set; words 32–37 are six masked LVT entries; word 7 is `0x00000002`, which is
in-service word 2, bit 1, for vector `0x41`. Those cross-checks are what make
the dump a reading rather than a transcription.

**Still unpinned, and recorded as such.** Words 29, 30 and 31 — which the
layout calls `Esr`, `IcrHigh` and `IcrLow` — read zero in every page this probe
took, so nothing here distinguishes them, and in particular nothing says which
of the ICR's two halves comes first (the xAPIC memory map has the low half
first; this structure is transcribed with the high half first). Words 38–43
(`LvtCmci`, `ErrorStatus`, `InitialCount`, `CounterValue`,
`DivideConfiguration`, `RemoteRead`) likewise read zero throughout and are
transcribed rather than measured.

**A duplication to resolve when that tail is pinned:** `ApicRegister` declares
**two** error-status fields — `Esr` at word 29 and `ErrorStatus` at word 39.
Both may be genuine — Hyper-V's interrupt-controller structure could plausibly
declare a latched ESR early and an error-status word after the LVT block — but
nothing here settles it, and it must be settled against the SDK's own field
order before any conversion of the APIC state page reads or writes those words. Neither is
exercised by anything today, which is why the duplication has survived.

### The earlier ten, re-run

The per-exit costs are load-bound, and this run was taken under the load recorded
above. They are reported for completeness, not as platform latencies:

| | 2026-08-27 | 2026-09-05, this run |
|---|---|---|
| port I/O per exit | 4.02–4.16 µs | 9.31 µs |
| MMIO per exit | 4.30–4.40 µs | 9.45 µs |
| halt per exit | 3.99–4.14 µs | 10.03 µs |
| 2 MiB map | 30 µs | 98.1 µs |
| permission flip | 2.6 µs | 7.53 µs |
| separate security domain | no measurable difference | −0.2 %, inside the noise |
| mappings before a refusal | 8 192, none | 8 192, none |
| map cost, first 512 / thereafter | 13.3 µs / 61.0 µs | 30.7 µs / 853.0 µs |
| cancel stickiness (`None`) | sticky, 11–22 µs | sticky, 28.9 µs |

Every cost is between two and three-and-a-half times its 2026-08-27 figure, and
the mapping-churn row fourteen times; twelve build processes were resident at
84 % CPU while this run was taken, and the probe's own scheduling-delay indicator
read 406.43 µs, which the appendix carries. The ratio is the machine, not a
platform change — which is why the rows that are *answers* rather than durations
are the ones that carry: the separate security domain still makes no measurable
difference, 8 192 mappings still bring no refusal, and a cancel issued while the
processor is stopped is still latched.

The four answers that are not costs also reproduced, and the appendix carries
the lines: Q1's conclusion is unchanged (`GpaAccessFaultExit` still faults an
instruction fetch from a range mapped read, write and execute, and still stays
off); Q2 still reports RIP standing on the faulting instruction with
`instruction_length = 0` for both memory exits, and no instruction bytes for the
read-only case; Q3 still resumes a `None`-mode halt on `PendingInterruption`
alone; Q7 still shows CPUID answered in both directions and `ProcessorCount`
refused after setup while `ExtendedVmExits` is accepted; Q10 still shows the
`XApic` NMI waking a parked processor and the Fixed vector dropped by the
guest's own disabled APIC.

## What this changes

| Item | Status |
|---|---|
| **P1** — clear halt suspend and wake a parked processor for an ExtINT | **YES on this host.** The legacy ExtINT path keeps its designed shape; QEMU's Windows-10 workaround is not needed |
| **P3** — halt exits under an emulated APIC | **NO.** The run loop's `Halt` arm is unreachable with an APIC; idleness must be noticed from outside the run |
| 2026-08-27 finding 3, "`HaltSuspend` is never set and the register cannot be written **at all** on this host" | **Corrected — mode-specific.** True under `None`; false under `X2Apic`, where the bit is set and the register is writable |
| 2026-08-27 finding 10, "direct injection is unreachable in this mode" | **Its reasoning is refuted.** A cancel *does* stop a parked processor (P3, P4) and the register writes then go through (P1). Measured under `X2Apic`; under `XApic` itself the route is untested, so the conclusion is unestablished rather than disproved |
| 2026-08-27 host table's seven extended exits | **Corrected — a port artefact.** The raw word is `0x7fff`: the host offers fifteen positions, this port names thirteen, and `0xc00` is the unnamed remainder |
| **P2** — is the state page the only write path | **NO.** `WHvX64RegisterApicTpr` is refused for read AND write (`0xC0350005`), but `WHvX64RegisterApicBase` and `CR8` are both writable under `X2Apic` and both read back the value written, on a processor that had been created and never run. A page conversion carries the APIC base through `WHvX64RegisterApicBase` and the task-priority *class* (`TPR[7:4]`, all `CR8` holds) through `CR8`; the page carries neither, and no measured path carries `TPR[3:0]` |
| **P2b** — a software-disabled APIC's handling of a requested vector | **Dropped, silently.** A page conversion MUST carry the spurious register's enable bit |
| **P4** — cancel stickiness under `X2Apic` | Sticky, as under `None`. A pre-run cancel cannot be lost |
| **P5** — the LINT0 write trap | Offered and working; the trapped value is applied, and **RIP arrives already advanced** with `instruction_length = 0` |
| **P5** — is a host-placed ExtINT gated by LINT0 | **Not gated.** The fabric's LVT0 tracking is a correctness requirement; it is built and tested (`IrqFabric::lint0_admits_ext_int`) |
| **P6** — the synthetic bank and Hv#1 | Accepted whole; the guest reads `"Microsoft Hv"`, `"Hv#1"`, hypercall bit 5 and VP-index bit 6 both set |
| **P7** — clocks | Capability answers and the guest's enlightened MSRs agree exactly; APIC bus clock is 200 MHz |
| **P8** — partition time and the TSC | Freezes; and the TSC advances while the processor is merely stopped, so a suspend is the only way to hide host time |
| **P9** — in-service versus trigger-mode | **Not swapped**, pinned by two independent asymmetric observations. Words 29–31 and 38–43 remain transcribed, and `ApicRegister`'s two error-status fields must be resolved before any page conversion uses them |

## Appendix: the probe's output, unedited

Every table above is drawn from this. It is reproduced in full rather than
summarised because the summaries in an earlier draft dropped exactly the rows
that carried the weight — control A's call outcomes, and runs 1 and 2 of each
LINT0 attempt — and the log they were dropped from was not kept.

```
== host ==
  features word            0x00000000000002ff
  partial unmap            true
  local APIC emulation     true
  dirty page tracking      true
  idle suspend             true
  physical address width   39 bits
  exits word               0x0000000000007fff
  exits offered and unnamed by this port 0x0000000000000c00
  exits offered            ExtendedVmExits { cpuid: true, msr: true, exception: true, rdtsc: true, apic_smi_trap: true, hypercall: true, apic_init_sipi_trap: true, apic_write_lint0_trap: true, apic_write_lint1_trap: true, apic_write_svr_trap: true, apic_write_ldr_trap: true, apic_write_dfr_trap: true, gpa_access_fault: true }
  processor features       0x3e1bfbcfe7f7859f
  synthetic features       0x0000006fff448fff
  processor clock          2496000499 Hz
  interrupt clock          200000000 Hz
  TSC deadline timer       true
  scheduling delay         406.43µs (mean overshoot of ten 5 ms sleeps)
  available parallelism    12

== answers ==

Q1  Does a guest write to a read-and-execute window exit, and is
    ExtendedVmExits.GpaAccessFaultExit needed for it?
  -> YES, and the bit must stay OFF — a write to a mapped read-only window exits on its own, while setting GpaAccessFaultExit also faults accesses the mapping permits.
     without the bit: MemoryAccess gpa=0x10000 access=Write gpa_unmapped=false instruction_length=0 rip=0x1005
     with the bit:    MemoryAccess gpa=0x1000 access=Execute gpa_unmapped=false instruction_length=0 rip=0x1000
     (the code page at 0x1000 is mapped read+write+execute, so a fault there is one the mapping permitted)

Q2  Does a memory-access exit carry the instruction bytes, and does
    WHvTranslateGva work in real mode?
  -> NO for write to a read-only window and write to an unmapped range — RIP still points at the faulting instruction and the exit reports no length, so a host MUST decode it to make progress. And for write to a read-only window, not even the instruction bytes come with the exit, so the decoder must fetch them from guest memory too.
     write to a read-only window  rip=0x1005 (instruction at 0x1005, platform advanced it: false), instruction_length=0, no instruction bytes
     write to an unmapped range   rip=0x1007 (instruction at 0x1007, platform advanced it: false), instruction_length=0, bytes[16]=[26 a2 00 00 eb fa 00 00 00 00 00 00 00 00 00 00]
     port write                   rip=0x1003 (instruction at 0x1003, platform advanced it: false), instruction_length=1
     halt                         rip=0x1001 (instruction at 0x1000, platform advanced it: true), instruction_length=0
     WHvTranslateGva(0x10000) with paging off -> result_code=0 gpa=0x10000

Q3  Under LocalApicEmulationMode::None, does a halted processor resume
    on PendingInterruption alone, or must HaltSuspend be cleared?
  -> PendingInterruption ALONE resumes the processor — HaltSuspend does not have to be cleared.
     at the halt: rip=0x1002 instruction_length=0 activity=InternalActivity { startup_suspend: false, halt_suspend: false, idle_suspend: false }
     after injecting vector 0x40: Halt at rip=0x1008; handler ran=true resumed past the halt=true

Q4  What does one exit cost?
  -> port I/O 9.31 us, MMIO 9.45 us, halt 10.03 us per exit
     20000 exits each, round-trip out of and back into the guest, with no host register writes in between.
     port I/O: 186.1442ms
     MMIO:     188.9163ms
     halt:     200.6919ms

Q5  What does mapping cost, and can a sub-range be unmapped?
  -> a 2048 KiB map takes 98.1 us; a permission flip takes 7.53 us; a partial unmap was accepted
     WHvMapGpaRange of 512 pages: 98.1µs
     2000 permission flips of one page: 15.0655ms
     capability says partial unmap is true; attempting one: accepted

Q6  What does SeparateSecurityDomain buy?
  -> MMIO exits cost 9.87 us with a separate domain and 9.86 us without (-0.2%)
     best of two runs each, interleaved.
     separate=true:  227.7438ms then 197.4783ms
     separate=false: 219.5856ms then 197.1454ms

Q7  Can CPUID leaves be answered by the host, including the
    hypervisor-present bit, and are properties really pre-setup-only?
  -> YES — the host authors CPUID leaf 1 outright: it can set the hypervisor-present bit and clear it again, which is the stealth lever. Properties, however, are NOT uniformly pre-setup-only.
     leaf 0x1: EXITED at rip=0x1006 (instruction_length=2); the host's own answer would have been eax=0x000906a3 ebx=0x00000800 ecx=0x76da3203 edx=0x0f8bfbff
                answered with the hypervisor-present bit 1: guest read eax=0x000906a3 ecx=0xf6da3203 (bit reads 1), then Halt
                answered with the hypervisor-present bit 0: guest read eax=0x000906a3 ecx=0x76da3203 (bit reads 0), then Halt
     leaf 0x40000000: EXITED at rip=0x1006 (instruction_length=2); the host's own answer would have been eax=0x00000000 ebx=0x00000000 ecx=0x00000000 edx=0x00000000
                answered with the hypervisor-present bit 1: guest read eax=0x00000000 ecx=0x80000000 (bit reads 1), then Halt
                answered with the hypervisor-present bit 0: guest read eax=0x00000000 ecx=0x00000000 (bit reads 0), then Halt
     ProcessorCount: refused after setup (WHvSetPartitionProperty failed: HRESULT 0x80070057)
     ExtendedVmExits: ACCEPTED after setup

Q8  Is WHvCancelRunVirtualProcessor sticky when the processor is not
    running?
  -> YES, STICKY — a cancel issued while the processor was stopped ended the next run immediately.
     run returned Canceled { reason: 0 } after 28.9µs (the rescue fires at 250ms)
     rescue cancel result: accepted

Q9  How many separate mappings does a partition tolerate, and what
    does each cost?
  -> at least 8192 separate ranges — no limit reached, but the cost per map grows with the count: 30.7 us for the first 512, 853.0 us thereafter
     mapped 8192 single-page ranges in 6.5669919s (first 512 in 15.7152ms)
     unmapped 8192 of them
     the cap of 8192 was the experiment's, not the host's — no surrogate process is needed at a PC chipset's scale

Q10 Under LocalApicEmulationMode::XApic — the mode a guest with a real
    kernel needs — can the host still wake a halted processor, and by
    which route?
  -> YES, from another thread, and the delivery MECHANISM is proven: a mid-run NMI wakes the parked processor and its handler runs. The Fixed vector was dropped by the guest's own APIC, which a real-mode guest never enables — not by the platform. A halted processor does not leave `WHvRunVirtualProcessor` under XApic on its own, so a register write cannot reach it until a cancel stops the run first (P4 measures the cancel, P1 the writes that follow it, both under X2Apic); a userspace 8259 can deliver across threads without one.
     XApic, stop-then-inject: the run NEVER RETURNED on its own — with this APIC mode the hypervisor parks a halted processor instead of exiting
       => `WHvRegisterPendingInterruption` cannot be applied to a run in
          progress: it is a register write, which needs a stopped processor,
          and this halted processor did not leave the run on its own. A
          partition-level call delivers without stopping it; a cancel stops
          it (P4), and whether the writes are then accepted is measured only
          under X2Apic (P1).
     XApic, Fixed vector 0x40 mid-run:  the handler did NOT run; the run then parked again and was rescued
     XApic, NMI mid-run:                the handler RAN; the run then parked again and was rescued
       => the mechanism WORKS; the Fixed vector was dropped by the guest's own
          APIC, which a real-mode guest never enabled. A kernel that enables it
          would receive the vector.
     None + WHvRequestInterrupt (control): refused, as it should be — WHvRequestInterrupt failed: HRESULT 0xc0350008

P1  Under X2Apic, is HaltSuspend clearable, and does clearing it wake a
    parked processor so it takes a host-placed pending ExtINT?
  -> YES — under X2Apic the activity register IS writable, and clearing HaltSuspend is what wakes the parked processor: with the ExtINT placed but the suspend left standing, the handler never ran.
     X2Apic, pending ExtINT then halt_suspend=false:
       run                                        exit reason                  elapsed  rescued  handler      rip  len
       run 1: sti; hlt                            Canceled(0)                  256.6ms     true    false   0x1002    0
       run 2: after the host's writes             Canceled(0)                  248.8ms     true     true   0x1008    0
       activity once run 1 ended: InternalActivity { startup_suspend: false, halt_suspend: true, idle_suspend: false }
       WHvRegisterPendingEvent (ExtINT 0x20): ACCEPTED
       WHvRegisterInternalActivityState (halt_suspend=false): ACCEPTED
       guest reached the store past its first HLT: true
     
     control A — X2Apic, pending ExtINT and NO activity write:
       run                                        exit reason                  elapsed  rescued  handler      rip  len
       run 1: sti; hlt                            Canceled(0)                  246.9ms     true    false   0x1002    0
       run 2: after the host's writes             Canceled(0)                  261.3ms     true    false   0x1002    0
       activity once run 1 ended: InternalActivity { startup_suspend: false, halt_suspend: true, idle_suspend: false }
       WHvRegisterPendingEvent (ExtINT 0x20): ACCEPTED
       WHvRegisterInternalActivityState (halt_suspend=false): not attempted (control)
       guest reached the store past its first HLT: false
     
     control B — None, the mode the earlier probe refused in:
       run                                        exit reason                  elapsed  rescued  handler      rip  len
       run 1: sti; hlt                            Halt                          42.9ms    false    false   0x1002    0
       run 2: after the host's writes             InvalidVpRegisterValue        48.2ms    false    false   0x1002    0
       activity once run 1 ended: InternalActivity { startup_suspend: false, halt_suspend: false, idle_suspend: false }
       WHvRegisterPendingEvent (ExtINT 0x20): ACCEPTED
       WHvRegisterInternalActivityState (halt_suspend=false): REFUSED (WHvSetVirtualProcessorRegisters failed: HRESULT 0xc0350006)
       guest reached the store past its first HLT: false

P3  Under X2Apic, does a HLT produce an exit at all?
  -> NO — a HLT under X2Apic produces no exit; the hypervisor parks the processor inside WHvRunVirtualProcessor and only a cancel from another thread ends the run. The run loop's Halt arm is unreachable in this mode. The processor genuinely reached the halt: the cancelled run reports rip=0x1002, at or past the HLT at 0x1001.
       run                                        exit reason                  elapsed  rescued  handler      rip  len
       X2Apic: cli; hlt                           Canceled(0)                  253.7ms     true    false   0x1002    0
       None (control): cli; hlt                   Halt                          30.8ms    false    false   0x1002    0
       the rescue cancel fires at 200ms; a row at that elapsed with rescued=true is a run that never returned on its own
       the guest is `cli` at 0x1000 then `hlt` at 0x1001; the X2Apic run was cancelled at rip=0x1002, which is AT OR PAST the halt — the processor executed and parked in it, so the absent exit is an absent exit and not an unreached instruction
       positive corroboration is P1's, not this experiment's: there a guest parked in `sti; hlt` under the same mode resumed PAST its halt once the host cleared halt_suspend, writing the RESUMED byte. A processor that never reached a halt cannot resume past one.

P4  Under X2Apic, is a cancel issued while the processor is stopped still
    sticky?
  -> YES, STICKY under X2Apic too — a cancel issued while the processor was stopped ended the next run immediately, so the acquire-check every reference VMM performs before a run guards a race the platform already handles.
       run                                        exit reason                  elapsed  rescued  handler      rip  len
       spin, after a cancel issued while stopped  Canceled(0)                   44.5ms    false    false   0x1000    0
       the guest is `jmp $`, which exits for nothing; without the earlier cancel this run ends only when the rescue fires at 200ms

P5  Does a guest write to LINT0 trap, and is a host-placed pending ExtINT
    gated by what LINT0 holds?
  -> The write TRAPS: ExitReason::ApicWriteTrap names LINT0 and carries the value the guest wrote. A host-placed pending ExtINT is NOT gated by LINT0: it reached the guest with the entry masked just as with it unmasked, so a fabric that must honour the mask has to consult LINT0 itself.
     the host advertises ExtendedVmExits.apic_write_lint0_trap: true; the APIC is software-enabled by the host before each run, without which a disabled APIC forces every LVT mask bit and the two attempts would be the same case twice
     
     LINT0 masked: (guest writes 0x00010700)
       run                                        exit reason                  elapsed  rescued  handler      rip  len
       run 1: write LINT0, then sti; hlt          ApicWriteTrap(Lint0=0x10700)    40.0ms    false    false   0x100a    0
       run 2: sti; hlt                            Canceled(0)                  277.2ms     true    false   0x100c    0
       run 3: after the host places an ExtINT     Canceled(0)                  262.4ms     true     true   0x1012    0
       the store begins at 0x1006; run 1 came back at rip=0x100a with instruction_length=0
       LINT0 in the state page afterwards: 0x00010700
       pending-event write ACCEPTED / activity write ACCEPTED
     
     LINT0 unmasked (control): (guest writes 0x00000700)
       run                                        exit reason                  elapsed  rescued  handler      rip  len
       run 1: write LINT0, then sti; hlt          ApicWriteTrap(Lint0=0x700)    48.6ms    false    false   0x100a    0
       run 2: sti; hlt                            Canceled(0)                  267.7ms     true    false   0x100c    0
       run 3: after the host places an ExtINT     Canceled(0)                  261.7ms     true     true   0x1012    0
       the store begins at 0x1006; run 1 came back at rip=0x100a with instruction_length=0
       LINT0 in the state page afterwards: 0x00000700
       pending-event write ACCEPTED / activity write ACCEPTED

P6  Is the synthetic feature bank accepted, and does the guest then see
    the Hv#1 interface?
  -> YES — the guest reads vendor "Microsoft Hv" and interface "Hv#1", and the feature leaf reports hypercall MSRs true and VP index true. A kernel that gates on those two bits WILL take the enlightenments.
     host allows 0x0000006fff448fff; OPENVMM_VTL0 asks for 0x000000007f448fff; asked-for and not allowed 0x0000000000000000; allowed and unnamed by this port 0x0000006f80000000
     the host accepted OPENVMM_VTL0 whole
     leaf 0x40000000: left through IoPortAccess(0xe9) — eax=0x40000006 ebx=0x7263694d ecx=0x666f736f edx=0x76482074
     leaf 0x40000001: left through IoPortAccess(0xe9) — eax=0x31237648 ebx=0x00000000 ecx=0x00000000 edx=0x00000000
     leaf 0x40000003: left through IoPortAccess(0xe9) — eax=0x00000e7f ebx=0x00088020 ecx=0x00000002 edx=0x000ec1b0
     vendor string "Microsoft Hv", interface signature "Hv#1", feature leaf EAX 0x00000e7f (hypercall MSRs true, VP index true)

P7  What are the platform's clocks, and do the guest's enlightened
    frequency MSRs agree with the capability answers?
  -> TSC-deadline timer true; processor clock 2496000499 Hz and the guest's HV_X64_MSR_TSC_FREQUENCY 2496000499 Hz (AGREE); interrupt clock 200000000 Hz and the guest's HV_X64_MSR_APIC_FREQUENCY 200000000 Hz (AGREE).
     capability answers: processor clock 2496000499 Hz, interrupt clock 200000000 Hz, TSC-deadline timer true
     guest RDMSR 0x40000022: answered — eax=0x94c5f1f3 edx=0x00000000 (2496000499 Hz), left through IoPortAccess(0xe9)
     guest RDMSR 0x40000023: answered — eax=0x0bebc200 edx=0x00000000 (200000000 Hz), left through IoPortAccess(0xe9)

P8  Does suspending partition time freeze the guest's TSC?
  -> YES — suspending partition time freezes the guest's TSC exactly, and resuming lets it advance again. Note the control: the TSC advances while the processor is merely STOPPED, so a host that wants a guest not to see its own downtime must suspend the clock, not merely refrain from running.
     the guest reached its halt: Halt; the second run returned Halt
       observation                                               guest TSC    reference 100ns
       after the halt                                             23904073               1584
       +50 ms, partition time RUNNING                            177522123             617174
       partition time suspended                                  177675734             617486
       +50 ms, partition time SUSPENDED                          177675734             617486
       resumed, and the guest ran again                          177815490             618450
       the TSC moved 153618050 counts across the running window and 0 across the suspended one; it advances while the processor is merely stopped: true

P9  Which of the state page's two remaining 256-bit bitmaps is in-service
    and which is trigger-mode?
  -> CONFIRMED as this port has it — an accepted, unacknowledged vector appears in the field the layout calls IN-SERVICE and not in the one it calls trigger-mode. The two are not swapped.
     vector 0x41 is bit 1 of word 2, so the word this port predicts is 00000000 00000000 00000002 00000000 00000000 00000000 00000000 00000000
     
     level-triggered request of 0x41, processor never run:
       request      00000000 00000000 00000002 00000000 00000000 00000000 00000000 00000000
       in-service   00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
       trigger-mode 00000000 00000000 00000002 00000000 00000000 00000000 00000000 00000000
     
     edge-triggered request of 0x41, processor never run:
       request      00000000 00000000 00000002 00000000 00000000 00000000 00000000 00000000
       in-service   00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
       trigger-mode 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
     
     accepted and NOT acknowledged (guest stopped inside its handler):
       delivery ACCEPTED / handler ran true / exit IoPortAccess(0xe9) / rescued false
       request      00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
       in-service   00000000 00000000 00000002 00000000 00000000 00000000 00000000 00000000
       trigger-mode 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
     
     the whole page as the platform left it after the acceptance, so the words still unpinned are on the record as data:
       words  0..: 00000000 00050014 00000000 ffffffff 000001ff 00000000 00000000 00000002
       words  8..: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
       words 16..: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
       words 24..: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
       words 32..: 00010000 00010000 00010000 00010000 00010000 00010000 00000000 00000000
       words 40..: 00000000 00000000 00000000 00000000

P2  Does the APIC state page round-trip through the platform, and is it
    the ONLY write path — or are the named APIC registers writable too?
  -> The page round-trips bit-identically over its first KiB. The named registers, one at a time on an X2Apic partition: WHvX64RegisterApicTpr write REFUSED (WHvSetVirtualProcessorRegisters failed: HRESULT 0xc0350005), read REFUSED (WHvGetVirtualProcessorRegisters failed: HRESULT 0xc0350005); ApicBase write ACCEPTED and the read-back carries it; Cr8 write ACCEPTED and the read-back carries it. So the page is NOT the only write path: state the page carries no field for still crosses through the named registers that were accepted.
     the page of a processor created and never run — no guest instruction has executed, so this is the platform's reset state: version 0x00050014, spurious 0x000000ff, destination format 0xffffffff
     read -> write -> read: first KiB identical true, whole 4 KiB identical true
     WHvX64RegisterApicTpr write of 0x20: REFUSED (WHvSetVirtualProcessorRegisters failed: HRESULT 0xc0350005)
     WHvX64RegisterApicTpr read: REFUSED (WHvGetVirtualProcessorRegisters failed: HRESULT 0xc0350005)
     the two registers the page carries no field for, each read -> written -> read again on that same X2Apic processor — stopped, and still never run:
       ApicBase   before 0x00000000fee00900                       write 0x00000000fee00d00 -> ACCEPTED                                 after 0x00000000fee00d00
       Cr8        before 0x0000000000000000                       write 0x0000000000000002 -> ACCEPTED                                 after 0x0000000000000002
       the write asked for a value other than the one already standing — ApicBase: true, Cr8: true
     the page as read:
       words  0..: 00000000 00050014 00000000 ffffffff 000000ff 00000000 00000000 00000000
       words  8..: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
       words 16..: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
       words 24..: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
       words 32..: 00010000 00010000 00010000 00010000 00010000 00010000 00000000 00000000
       words 40..: 00000000 00000000 00000000 00000000

P2b Does a software-disabled APIC DROP a requested vector, or latch it for
    later delivery?
  -> DROPPED, and silently: WHvRequestInterrupt returns SUCCESS with the APIC software-disabled and the request bitmap stays EMPTY; the same call after the host sets the spurious register's enable bit lands the vector. A page conversion must carry the spurious register's enable bit, or restoring a page silently disables the guest's APIC.
     spurious register at reset 0x000000ff (software enable is bit 8), after the host sets the enable 0x000001ff
       step                                             call       request bitmap
       request of 0x43 with the APIC DISABLED           ACCEPTED   00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
       (no request) after the host enables the APIC     -          00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
       request of 0x43 with the APIC ENABLED            ACCEPTED   00000000 00000000 00000008 00000000 00000000 00000000 00000000 00000000
       the word this port predicts for 0x43 is 00000000 00000000 00000008 00000000 00000000 00000000 00000000 00000000
```
