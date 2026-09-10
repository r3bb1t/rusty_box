# Declared Bochs parity divergences

Deviations from `cpp_orig/bochs/` are bugs. This file is the registry of the
handful that are **not** — each one deliberate, argued, and measured, so the
question does not have to be re-researched.

Doctrine: R7 (parity provenance) in `docs/safety-doctrine.md`. Rules for adding
an entry:

- Cite the Bochs file + symbol the divergence departs from (never a line number).
- State what the guest can observe. If the answer is "nothing", say why.
- Record the measurement or constraint that justifies it. An entry with no
  evidence is a bug report, not a divergence.
- If closing it is merely expensive rather than impossible, price it here.

---

## D1 — A halted machine's virtual time is advanced by the scheduler, not by an in-CPU spin

**Bochs:** `cpu/event.cc handleWaitForEvent`. A single CPU idles inside the
function: `while (1) { …check wake conditions…; BX_TICKN(10); }`, returning to
`cpu_loop` only under `if (BX_SMP_PROCESSORS > 1)`.

**rusty_box:** `BxCpuC::handle_wait_for_event` always returns — Bochs's
`BX_SMP_PROCESSORS > 1` branch, taken unconditionally. `cpu_loop` returns to the
scheduler, which advances virtual time via
`Emulator::hlt_wait_step_ticks` (`emulator/scheduler.rs`) and re-enters. That
step runs to the *earliest exact timer deadline* rather than in ten-tick granules.

### What the guest observes

Nothing. Timers fire at their exact deadlines inside `tickn` whichever loop
advances the clock, and a fully idle machine has no other event source. Host
input is pumped before each halted step, so input latency is bounded by the next
timer deadline — on any booted machine the PIT, ~1 ms.

The same argument already governs the SMP fast-forward in
`can_fast_forward_bsp_hlt`, which skips the empty rounds Bochs grinds through
when every AP is idle.

### Why it is not closed: measured cost

Adopting Bochs's granularity is one expression —
`hlt_wait_step_ticks().min(10)`, which still never overshoots a nearer deadline.
It was implemented and measured (2026-08-15, release builds, same machine,
interleaved):

| workload | exact deadline | `.min(10)` granularity |
|---|---|---|
| `cargo test -p rusty_box --lib --features std` | 2.33 s / 2.39 s | **19.0 s** (~8×) |
| DLX Linux headless boot gate | ~55 s | **did not finish in 600 s** (>10×) |

The cost is structural, not incidental: Bochs's granule is an inner loop inside
the CPU and costs almost nothing, while here every granule is a full scheduler
round-trip (batch wiring set up and torn down). Ten-tick stepping therefore buys
a wake-check cadence no guest can observe at an order of magnitude on every real
workload, against an emulator already 1.92× behind Bochs.

**If the cadence is ever wanted cheaply**, the fix is not the cap: move the
granule loop inside `handle_wait_for_event` where Bochs keeps it (bounded, so
cooperative hosts still get control back), which restructures who owns time
advancement across the scheduler, the slowdown path and the SMP fast-forward.
Not attempted — it is a rework of the time model for no guest-visible gain.

### Why the spin cannot simply be adopted

`examples/rusty_box_web` drives the emulator cooperatively — one `step_batch()`
per frame. A blocking wait never yields to the browser event loop, so the tab
freezes and input can never arrive to end the wait. The egui GUI needs the same
frame pumping.

Researched 2026-08-15; blocking under WASM *is* achievable, at these prices:

- **Web Worker + `Atomics.wait`** (`wasm_thread`, `wasm_safe_thread`): blocking
  works off the main thread and panics on it. Needs `SharedArrayBuffer`, so
  COOP/COEP cross-origin isolation on every host serving the app (GitHub Pages
  cannot set headers; needs a service-worker shim), plus
  `-C target-feature=+atomics,+bulk-memory,+mutable-globals` and
  `-Z build-std=panic_abort,std` — `target_feature = "atomics"` on wasm32 is
  still nightly-only, so the `wasm target check` gate would move to nightly.
- **Binaryen Asyncify** (`wasm-opt --asyncify`): unwinds/rewinds the stack so
  blocking code yields. ~70% `.wasm` size growth; overhead near zero only when
  `--asyncify-ignore-indirect` is safe; reported Chrome stack-exhaustion issues.

Both make the emulator *able* to block without changing anything the guest can
observe, so neither is worth its price. Revisit only if a guest-visible timing
difference is ever demonstrated.

**Status:** open and deliberate. Reversing it means paying the table above.

---

## D2 — CPUID leaf 0xB sub-leaf 1 reports the core-level shift over the whole package

**Bochs:** `cpu/cpuid.cc bx_cpuid_t::get_std_cpuid_extended_topology_leaf`,
`case 1`: `leaf->eax = ilog2(ncores-1)+1` — the width of the core field alone.

**rusty_box:** `cpu/soft_int.rs`, `CPUID_TOPOLOGY_SUBLEAF_CORE`:
`bochs_topology_shift(package_logical_count())`, i.e. `ilog2(ncores*nthreads-1)+1`
— the width of the core field *plus* the SMT field. EBX, ECX and EDX match
upstream exactly; only EAX differs, and only when `nthreads > 1`.

### Why upstream is wrong here

Intel SDM Vol 2, CPUID leaf 0BH: EAX[4:0] at sub-leaf *m* is "the number of bits
to shift right on x2APIC ID to get a unique topology ID of the **next** level
type". The next level above *core* is the package, so the shift has to clear
both the SMT bits and the core bits. Linux consumes it exactly that way —
`detect_extended_topology` stores the core sub-leaf's EAX in a variable named
`core_plus_mask_width` ("core **plus**") and computes
`phys_proc_id = initial_apicid >> core_plus_mask_width`.

Bochs assigns APIC ids densely from the CPU index
(`cpu/apic.cc bx_local_apic_c::bx_local_apic_c`, `apic_id = id`), so on a
2 x 4 x 2 machine they pack as `socket:1 | core:2 | thread:1`. Shifting by the
core width alone leaves the top core bit inside the package id.

### What the guest observes

Measured by `cpuid_leaf_b_core_shift_separates_sockets_as_software_reads_it`
(`cpu/soft_int.rs`), which derives the package id for all 16 logical processors
the way Linux does. Upstream's formula yields:

| formula | package id per APIC id 0..15 | sockets seen |
|---|---|---|
| Bochs `ilog2(ncores-1)+1` | `0,0,0,0,1,1,1,1,2,2,2,2,3,3,3,3` | **4** |
| ours `ilog2(ncores*nthreads-1)+1` | `0,0,0,0,0,0,0,0,1,1,1,1,1,1,1,1` | 2 |

A 2-socket guest enumerating as 4 packages misplaces every scheduler and NUMA
decision built on `phys_proc_id`. The divergence is therefore also the *less*
detectable answer: real hardware follows the SDM, so matching Bochs would be the
fingerprint, not avoiding it.

The test fails against the upstream formula — swap
`package_logical_count()` for `n_cores()` to reproduce.

**Status:** open and deliberate. Single-threaded topologies (`nthreads == 1`)
are bit-identical to Bochs, so the divergence is invisible on the default
uniprocessor configuration and on every current boot gate. Written up for
upstream in `docs/bochs-upstream-bugs.md`.

---

## D3 — A fast REP string burst stops at the next timer deadline

**Bochs:** `cpu/io.cc FastRepINSW` / `FastRepOUTSW` and
`cpu/faststring.cc FastRepMOVSB` bound a burst by ECX and by the elements that
fit in the destination page — nothing else. Virtual time does not move during
the burst: `cpu/io.cc INSW32_YwDX` calls `BX_TICKN(wordCount-1)` only *after*
`FastRepINSW` returns. The `if (BX_CPU_THIS_PTR async_event) break;` inside the
inner loop therefore only catches an event that was already pending on entry —
a deadline that falls inside the burst cannot be reached, because the clock that
would reach it is frozen.

**rusty_box:** `cpu/io.rs`, the three fast-REP loops, seed
`event_words_remaining` from `ticks_left_next_event()` and fold it into the
per-chunk `min`. `tickn_fastrep` runs inside the loop, so time advances as the
burst proceeds and the chunk ends exactly on the deadline.

### What the guest observes

Interrupt latency. A `REP INSW` of 2048 words with a timer deadline 300 ticks
away delivers that interrupt 1748 ticks late under upstream's model and on time
here.

### Why the divergence is the correct side

x86 REP string instructions are architecturally interruptible *between
iterations*: RIP stays on the prefix and RCX/RSI/RDI carry the progress, so a
pending interrupt is taken at the next iteration boundary rather than deferred
to the end of the instruction. Real hardware therefore behaves as this port
does. Upstream's atomic burst is a speed shortcut whose cost is guest-visible
timing, which makes matching it the fingerprint rather than avoiding it — the
same reasoning as D2.

### Known cost, not yet paid down

The loop guard is `while cx != 0 && event_words_remaining != 0`, so when the
deadline has already been reached the fast path is skipped entirely and the
instruction falls back to per-element processing until the boundary is serviced.
A `.max(1)` floor would keep the fast path alive at a bounded one-element
overrun. Not applied: it changes boot-path timing and would need its own boot
gates plus an A/B, and the cliff is a throughput dip, not a correctness problem.

**Status:** open and deliberate. Written up for upstream in
`docs/bochs-upstream-bugs.md`.

---

## D4 — SYSRET writes the flat SS the SDM describes

**Bochs:** `cpu/proc_ctrl.cc SYSRET` updates the SS selector and writes
`valid`, `p`, `dpl`, `segment` and `type` into the cache, with the comment
"SS base, limit, attributes unchanged" — base, limit, D/B and G keep whatever
the previous stack segment held. That is AMD's reading of SYSRET (the APM
leaves the SS descriptor cache alone); Intel's SDM (vol. 2, SYSRET operation)
writes the whole flat segment: `SS.Base := 0; SS.Limit := FFFFFH; SS.B := 1;
SS.G := 1` alongside the type, DPL and P that Bochs does set.

**rusty_box:** `cpu/proc_ctrl.rs sysret` calls `setup_flat_ss(3)` in both the
64-bit and the compatibility arm — the same fixed flat setup this port's own
`SYSCALL` already performs (`setup_flat_ss(0)`), so the pair is symmetric and
Intel-correct. The modelled processor is an Intel one.

### What the guest observes

On the interpreter: a guest returning to 32-bit compatibility mode via SYSRET
gets the architectural 4-GiB flat stack segment instead of whichever limit the
previous SS happened to hold, so stack-limit checks after the return match
Intel silicon. In 64-bit mode nothing is observable — limits are not checked.

### Why the divergence is the correct side

Measured, not hypothetical: under the hypervisor engine, SS crosses the seam
as the full descriptor cache and the platform validates it against the
architecture's entry checks. Windows 7's first SYSRET to user mode inherited a
stack segment the platform's null-SS convention had left as effective limit
`0xFFFFFFFF` with byte granularity — a combination no descriptor can encode —
and `WHvRunVirtualProcessor` refused the processor outright
(`InvalidVpRegisterValue`, the state dump showing
`ss attr=0x00f3 limit=0xffffffff`). Intel's fixed flat write cannot produce an
unencodable segment, whatever SS held before.

### Price of closing it

Restoring Bochs's arms would re-introduce the un-enterable state under the
hypervisor engine and the wrong compat-mode stack limit under the interpreter;
there is nothing on the other side of the trade.

---

## D5 — Setting TF raises `async_event` without clearing the rest of the word

**Bochs:** `cpu/flag_ctrl_pro.cc setEFlags`: `if (get_TF()) async_event = 1;`,
and the `assert_TF` / `set_TF` accessors in `cpu/cpu.h` store the same
constant. An assignment, not an OR, over a word whose own declaration says it
is kept 32-bit so that `BX_ASYNC_EVENT_STOP_TRACE` (bit 31) fits in it — so
upstream's store already discards its own trace-stop hint. Upstream can afford
to: its `cpu_loop` asks the word only whether it is non-zero, and `1` answers
that exactly as `1 | (1 << 31)` does.

**rusty_box:** `cpu/flag_ctrl_pro.rs set_eflags_internal` and
`cpu/api_bridge.rs set_rflags_for_api` call `raise_async_event`, which is
`async_event |= 1`. The bit Bochs sets is set; the bits Bochs's store would
have cleared survive.

### What the guest observes

Under upstream's form, in this port: nothing a guest can observe at the
interpreter's site. At the API site there is one difference, on the hypervisor
engine, and it comes from a stall the bit causes there rather than from the
flags write (below). The reason differs by site, and one site also has a
host-side reach.

This port's word carries a bit Bochs has no counterpart for,
`BX_ASYNC_EVENT_SCHEDULER_BOUNDARY` (`cpu/cpu.rs`). `sync_lapic_events`
(`cpu/cpu.rs`) and `PcIo::sync_io_events` (`emulator/io.rs`) raise it when a
device has work only the machine's boundary can do: a completion timer armed
while answering a port write, a PAM flip, a relocated BAR. A literal `= 1`
under TF would zero it. Whether that matters depends on who reads the bit.

The interpreter's site, `set_eflags_internal`, has no reachable clobber, and its
`|=` is defensive. The trace loop leaves the trace the moment the word is
non-zero (`cpu_loop_n_impl`, the post-instruction `async_event != 0` break).
The loop head then hands the processor to the scheduler on the boundary bit
before another instruction runs. So no `POPF` or `IRET` can retire with the
bit already latched, and no single instruction both raises the bit and writes
TF.

The API site, `set_rflags_for_api`, is reached on the interpreter only from the
host, between slices. The callers that can carry TF are the public register
write (`api_bridge.rs`, `X86Reg::Rflags` and its narrower forms) and
`import_arch_state`. The slice loop takes the bit after every slice
(`take_scheduler_boundary_request`, `emulator/scheduler.rs`). But two setters
of the same API raise it again between slices, both through
`sync_lapic_events`: `set_cr8_for_api` and the `IA32_TSC_DEADLINE` arm of its
MSR write. So a host that writes one of them and then writes RFLAGS with TF
set would, under the literal store, clear the request. The next slice would
then run instead of returning at its loop head for the boundary. That is a
sequence of host calls, not guest behaviour.

On the hypervisor engine the site is reached from guest code, on every exit.
`Servicer::answer` (`rusty_box_whp_engine/src/vcpu_thread.rs`) first takes the
exit header: `Exchange::take_header` (`exchange.rs`) →
`BxCpuC::take_exit_header` → `take_rip_and_flags` → `set_rflags_for_api`
(`cpu/arch_state.rs`). An errand whose exit class imports
`ArchGroups::RIP_RFLAGS` goes through it again (`Exchange::import_for` →
`import_arch_groups` → `take_rip_and_flags`). A stepping guest's header has TF
set. The bit can be on the shadow's word at that moment, because `answer` ends
every exit with `PcIo::sync_io_events`.

Nothing on that engine reads the bit as a request, though. The machine's
boundary runs whatever the word holds: at the head of every exit's service
(`device_thread::service_once` → `Emulator::service_device_time`), and at every
device deadline on the device thread. It drains the device-side queues
(`scheduler_boundary_work_pending`, `emulator/scheduler.rs`), never the
processor's word. `wants_a_machine_boundary` has no caller in
`rusty_box_whp_engine`. `WhpEngine::run_slice` refuses, so the only function
that takes the bit, the scheduler's `take_scheduler_boundary_request`, is
never reached there. On this engine the latched bit therefore delays no
device.

It does stop the shadow. The bit has one reader on that engine: the
interpreter loop head inside `PcIo::emulate_one` (`cpu_loop_n_impl`,
`cpu/cpu.rs`), which returns without executing anything while the bit is set.
`PcIo::finish_the_instruction` parks the bit around an errand's instruction
(`park_trace_bookkeeping`). `take_the_signalled_smi` (`vcpu_thread.rs`) and
`run_the_shadow_out_of_smm` (`engine.rs`) call `emulate_one` without parking
it. So while the bit is latched, `take_the_signalled_smi` returns before the
signalled SMI is processed and hands the processor back with the SMI still
pending on the shadow. Once a handler has been entered, a latched bit leaves
`run_the_shadow_out_of_smm` retiring nothing until `SMM_HANDLER_CEILING` ends
it with an error. `take_the_signalled_smi` runs `PcIo::sync_io_events` between
its two calls, which can latch the bit at exactly that point.

The two forms differ there only by accident. `Servicer::answer` writes the
exit header, and the flags with it, before that exit's `sync_io_events`. With
TF set, the literal store would clear a bit latched at an earlier exit, and
the system-management path would then run unless something latched the bit
again in between; `|=` keeps it. The stall belongs to the latch, not to the
flags write. It happens whatever TF holds, and neither form removes it.

Under this port's form: nothing that differs from Bochs. The word is non-zero
after the write, which is all Bochs's loop asks; `STOP_TRACE` surviving means
the trace is not chained across the flags write, and Bochs's loop does not
chain it either, because its word is non-zero too.

### Why the divergence is the correct side

The assignment is a Bochs idiom for "make the word non-zero", written when the
word held nothing else worth keeping. Read as that intent, `|= 1` is the
faithful port. Read as the literal store, it ports an upstream defect (the
bit-31 clobber is real there, merely harmless) onto a word that carries a bit
Bochs's word does not.

### Price of closing it

At the interpreter's site nothing a guest can see is bought either way: guest
code cannot reach a latched bit there. On the hypervisor engine the latched
bit delays no device but does stall the shadow's system-management paths
(above), and the literal store would lift that stall for a stepping guest
alone. That is chance, not a closure: the stall is the latch's, and neither
form removes it. What `|=` keeps is that a
TF write never lowers the boundary bit, which the interpreter's loop head and
its slice loop both rely on. Without it, every flags write would have to be
audited against them.

The other closure is to move trace bookkeeping and the scheduler boundary out
of `async_event` into a word of their own, so that `= 1` could be written
literally and mean what Bochs means. That is the OPEN QUESTION below, and it
touches every `async_event` site in `cpu/`.

**Status:** open and deliberate. The literal `= 1` stores that remain in this
port (`signal_event`, `unmask_event`, and the FRED, SVM and task-switch paths)
are Bochs-faithful, carry the same hazard, and are outside this entry.

---

## D6 — The 8259's INTR reaches a processor through LVT0, and the bootstrap processor comes out of reset holding the virtual wire

**Bochs:** `pc_system.cc bx_pc_system_c::raise_INTR` and `lower_INTR`, reached
from `iodev/pic.cc bx_pic_c::service_master_pic` through the `BX_RAISE_INTR`
macro in `bochs.h`. They call `BX_CPU(0)->signal_event(BX_EVENT_PENDING_INTR)`
and the matching clear — straight at the processor's event word. Nothing on
that path reads `cpu/apic.cc`'s `lvt[APIC_LVT0]`. The local APIC's LINT0 entry
is written by the guest, saved, restored and returned on a read, and consulted
by nothing: grepping `apic.cc` for `APIC_LVT0` finds the register table and the
reset loop, and no reader. `cpu/event.cc handleAsyncEvent` then takes the vector
with `DEV_pic_iac()` on the strength of the event bit alone.

Symmetrically, `bx_local_apic_c::reset` masks every LVT entry including LINT0,
and the BIOS in `bios/` never writes offset `0x350` — `rombios.h` does not even
name it. So under Bochs a guest boots with LINT0 masked and takes legacy
interrupts anyway, and the mask it writes later changes nothing.

**rusty_box:** `cpu/cpu.rs BxCpuC::set_legacy_intr_level` is the one place the
8259's level becomes a processor event, and it asks
`cpu/apic.rs BxLocalApic::lint0_admits_ext_int` first. The INTA in
`cpu/event.rs acknowledge_external_interrupt` asks the same question, because
that body is reachable with only a *LAPIC* event pending and would otherwise
walk around the raise-side gate. `BxLocalApic::preset_lint0` gives APIC id 0 —
and only id 0 — `LVT0 = 0x700` at reset and whenever its identity is assigned.

The predicate is KVM's `kvm_apic_accept_pic_intr` (`arch/x86/kvm/lapic.c`): a
hardware-disabled local APIC admits the line unconditionally, because the SDM
(Vol. 3, "Local APIC Status and Location") says LINT0 and LINT1 then behave as
the INTR and NMI pins of a processor with no APIC at all; an enabled one admits
it only through an unmasked LVT0 in ExtINT delivery mode. A software disable
needs no term of its own — it masks every LVT entry where it happens
(`write_spurious_interrupt_register`), so the mask bit already carries it.

**Provenance (R7):** two upstreams, for the two halves.

- The routing is the SDM's ("Local Vector Table", LVT0 gates the LINT0 pin) and
  KVM's `kvm_apic_accept_pic_intr`. Xen's `vlapic.c` and QEMU's TCG local APIC
  both apply the same gate.
- The preset is `KVM_X86_QUIRK_LINT0_REENABLED`, enabled by default and
  self-described upstream as a deliberate spec deviation, applied in
  `kvm_lapic_reset` to the reset BSP only. QEMU's `apic_reset_common` carried
  the same literal `0x700` until it was removed in 2015, which broke coreboot
  within days and is why KVM's quirk still exists.

The Bochs side of it is filed in `docs/bochs-upstream-bugs.md` ("The legacy
8259's INTR bypasses the local APIC, so LVT0 (LINT0) gates nothing").

### What the guest observes

Firmware that programs virtual wire mode itself — EDK2/OVMF, coreboot, SeaBIOS
— observes nothing: it writes `0x700` into LVT0 during POST, which is the value
already there.

What changes is the answer to a question Linux asks on every boot.
`arch/x86/kernel/apic/io_apic.c check_timer()` decides whether the I/O APIC's
timer route works by masking LINT0 (`disable_8259A_irq` / the `unlock_ExtINT`
sequence) and watching whether the tick survives. Under the Bochs rule the tick
survives no matter what, because the 8259 is still wired straight at the
processor — so a machine whose I/O APIC timer route is broken reports itself
healthy, and the real failure surfaces later and somewhere else. Under this
rule the mask is honoured, the probe gets its true answer, and Linux falls
through to the next attempt as it does on hardware.

The second half is what makes the mask survivable: a refused line is **not**
consumed. The 8259 keeps its level and its edge bookkeeping — `acknowledge_
external_interrupt` reconciles a deasserted pin only when the pin really is
low — so the interrupt the guest is owed arrives at the next boundary after it
restores the wire, rather than being lost to the mask.

Without the preset the same change would be catastrophic rather than merely
visible: the BIOS this port ships with never writes `0x350`, so an honoured
mask with a masked reset value means every guest is deaf to the 8259 for its
whole life.

One image is affected: a v3 snapshot written before this rule restores LVT0
verbatim (`BxLocalApic::restore_snapshot_v3_body`), and a guest saved before it
had programmed the register carries the old masked reset value. Restored now,
such a guest is deaf to the 8259 until it programs LINT0 itself. There is no
migration to write, because nothing in the image distinguishes "masked because
reset never preset it" from "masked because the guest masked it" — and the
second must be honoured. In practice the window is early boot only: a guest
that has finished `check_timer()` and moved to I/O APIC delivery has LINT0
masked and wants nothing from the 8259.

Application processors are unaffected in the direction that matters: their
LINT0 stays masked, which is what the SDM asks for (at most one processor
carries an ExtINT entry) and what Linux's `setup_local_APIC` programs anyway.
An AP that drains the bus latch mid-slice now declines the line instead of
taking it; the level is not spent, and the next scheduler boundary republishes
it onto the bootstrap processor, where the wire is.

### Why the divergence is the correct side

Bochs is wrong here in the plain sense: it models the register and ignores it.
Every other x86 emulator and hypervisor that models a local APIC at all applies
the gate, and the guest software that matters was written against machines that
do. Keeping Bochs's behaviour means keeping a hole that makes a broken I/O APIC
look healthy — which is a fault this port cannot afford, because the fast engine
is where I/O APIC routing is most likely to be wrong.

The preset is a deviation from the SDM's reset state and is registered as one.
It buys the same thing it buys KVM: firmware that predates virtual wire mode,
or omits it as this port's BIOS does, still gets its legacy interrupts. The
alternative — teaching `bios/rombios.c` to write `0x350` — is a change to
vendored upstream firmware that every future BIOS refresh would have to carry.

The two halves are one entry because neither is safe alone. The routing without
the preset makes every guest deaf; the preset without the routing is a value
nothing reads.

### Price of closing it

Reverting the routing restores the upstream hole and costs Linux its
`check_timer()` answer. Reverting the preset requires the firmware change above.

The one cost of keeping it: reset leaves `LVT0 = 0x700` while the local APIC is
still software-disabled, a state real hardware cannot be in — a software disable
forces every LVT mask bit set. KVM has exactly the same inconsistency, and for
the same reason: the masking lives in its `kvm_lapic_reg_write` `APIC_SPIV`
arm, and `kvm_lapic_reset` presets LVT0 and then calls `apic_set_spiv`
directly, bypassing it. This port's `BxLocalApic::reset` clears
`software_enabled` by assignment for the same reason, and
`a_software_disable_closes_the_wire_but_reset_does_not`
(`cpu/apic.rs`) is the test that pins the distinction. Once the guest performs
a real software disable, the wire closes as hardware would.

**Status:** open and deliberate; both engines. The fast engine asks the same
question of whoever owns LVT0 in its topology — the local APIC is the
hypervisor's there, so the answer comes from the shadow the fabric keeps
(`IrqFabric::lint0_admits_ext_int`, `iodev/irq.rs`), consulted in
`rusty_box_whp_engine/src/vcpu_thread.rs stage_the_legacy_interrupt`.

---

## D7 — The CPUID frequency leaves default to "not enumerated"

**Bochs:** `config.cc` declares `cpu: cpuid_freq=hardware|none|ips` with
`hardware` as its default, and `cpu/cpuid.cc bx_cpuid_t::get_freq_leaf_15` /
`get_freq_leaf_16` answer the model's hardware dump in that mode. For
`corei7_skylake_x` (`cpu/cpudb/intel/corei7_skylake-x.cc`) the dump is leaf
0x15 EAX/EBX/ECX = 2 / 292 / 0 and leaf 0x16 = 3500 / 4000 / 100 MHz. The
option reached upstream through PR #792, the fix for bochs-emu/Bochs#791.

**rusty_box:** `CpuidFreq` (`cpu/cpuid.rs`) defaults to `None`, and so do
`EmulatorConfig::default` (`emulator/mod.rs`) and the `rusty_box_gui` launcher
(`--cpuid-freq`, `[emulator] cpuid_freq`; `rusty_box_gui/src/config.rs`). Only
the default differs. Each mode gives Bochs's answer —
`Hardware` the dump, `None` all-zero leaves, `Ips` a 1/1 crystal ratio at `ips`
Hz and the rate rounded to MHz — in `Corei7SkylakeX::get_freq_leaf_15` /
`get_freq_leaf_16` (`cpu/cpudb/intel/core_i7_skylake.rs`).

### What the guest observes

On the default model, leaves 0x15 and 0x16 read all zero. The SDM gives leaf
0x15 EBX = 0 the meaning "TSC/crystal ratio not enumerated", so Linux
calibrates the TSC against the PIT and measures the rate the counter really
runs at. `skylake_x_cpuid_freq_default_reports_leaves_not_enumerated`
(`core_i7_skylake.rs`) and
`cpuid_freq_config_reaches_every_cpu_through_cpuid_instruction`
(`emulator/tests.rs`) pin the zero leaves.

Under Bochs's default the same guest reads a 3.5 GHz declaration while the
emulated TSC advances one tick per retired instruction, `ips` ticks per
emulated second. Linux 4.8 and later take the frequency from these leaves
instead of the PIT (`cpu_khz_from_cpuid`, Linux commit `aa297292d708`), and
from 5.3 derive 3,499,912 kHz. Everything the kernel scales by the TSC —
`sched_clock` and printk timestamps, `udelay` / `mdelay`, `loops_per_jiffy`,
TSC-deadline arithmetic — then runs slow by the declared rate over `ips`, while
the PIT, HPET, RTC and PM timer keep correct emulated time. The factor is 875×
at `ips = 4,000,000`, the upstream default when #791 was filed, and 70× at the
50,000,000 that both Bochs `config.cc` and this port (`Ips::BOCHS_DEFAULT`)
default to now.

### Why the divergence is the correct side

Measured by the #791 reproduction on stock Bochs master `70da922c` (Ubuntu
26.04 live-server, kernel 7.0.0-14-generic, `cpu: model=corei7_skylake_x,
count=1, ips=4000000`; the issue carries the full kernel log): the kernel logs
`tsc: Detected 3499.912 MHz TSC` and `lpj=3499912`, never logs
`Fast TSC calibration using PIT`, and keeps the TSC as its clocksource. The
same guest on the Bochs build that became PR #792, with the leaves reported as
not enumerated, logs `Fast TSC calibration using PIT` and
`Detected 3.999 MHz processor`: the true rate.

`hardware` declares a rate the emulated TSC never runs at. `none` declares
nothing and lets the guest measure, which is the SDM's own opt-out and, as
#791 records, what QEMU does. A guest-visible clock that disagrees with every
other clock in the machine by a factor of 70 is a worse fingerprint than an
unenumerated leaf.

### Price of closing it

Three behavioural edits, in two files.

- `#[default]` moves to `CpuidFreq::Hardware` in `rusty_box/src/cpu/cpuid.rs`.
  That changes `EmulatorConfig::default` and nothing else.
- The `rusty_box_gui` launcher does not read that default.
  `rusty_box_gui/src/config.rs` resolves an absent value through its own arm,
  `None | Some("none") => CpuidFreq::None`, and that arm has to send an absent
  value to `Hardware`.
- The save arm in the same file leaves out whichever value it treats as the
  default (`CpuidFreq::None => None`). It has to leave out `Hardware` and write
  `none` out instead. Otherwise a saved `none` reloads as `hardware`.

Plus the consequential edits, which change no behaviour. Three pieces of text
state today's default and would become false: the `cpuid_freq` doc string in
`rusty_box_gui/src/args.rs` (`Default: none`), the doc string on the file
configuration's `cpuid_freq` field in `rusty_box_gui/src/config.rs`
(`Default: "none"`), and the comment on that file's resolve arm. Two tests pin
today's default and would fail: `cpuid_freq_defaults_to_none_and_parses_all_modes`
(`config.rs`), which asserts `CpuidFreq::None` for an absent value, and the
`EmulatorConfig::default` half of
`cpuid_freq_config_reaches_every_cpu_through_cpuid_instruction`
(`emulator/tests.rs`). `skylake_x_cpuid_freq_default_reports_leaves_not_enumerated`
keeps passing. It reads the model's `INIT` placeholder (`CpuidFreq::None`,
which every `Emulator` construction path replaces), not the default, so its
name and comment would have to follow.

After that, every Linux 4.8+ guest on the default model runs its TSC-derived
time 70× slow at the default `ips`. A caller who wants Bochs's answer asks for
it: `cpuid_freq: CpuidFreq::Hardware`, or `--cpuid-freq hardware`.

**Status:** open and deliberate; the default only. Filed upstream as
bochs-emu/Bochs#791.

---

## D8 — KSHIFTLW and KSHIFTRW shift by 15

**Bochs:** `cpu/avx/avx512_mask16.cc BX_CPU_C::KSHIFTLW_KGwKEwIbR` and
`KSHIFTRW_KGwKEwIbR` shift only `if (count < 15)`, so a count of 15 writes
zero. The other widths bound at their own width: `count < 8` in
`avx512_mask8.cc`, `< 32` in `avx512_mask32.cc`, `< 64` in `avx512_mask64.cc`.

**rusty_box:** `cpu/avx512_mask.rs kshiftlw_kgw_kew_ib_r` and
`kshiftrw_kgw_kew_ib_r` write zero only for `count >= 16`.

### What the guest observes

A count of exactly 15. `KSHIFTLW k1, k2, 15` with bit 0 of `k2` set leaves
`k1 = 0x8000` here and `0` under Bochs; `KSHIFTRW k1, k2, 15` with bit 15 set
leaves `1` here and `0` under Bochs. Every other count gives the same result on
both. The instructions are AVX-512F, so any guest on the default Skylake-X model
that has enabled the opmask state can reach them.

### Why the divergence is the correct side

The SDM's operation for both instructions shifts whenever the count is at most
15 and zeroes only above that — the bound Bochs itself applies at the byte,
dword and qword widths. The word width's `< 15` is the lone exception, so
matching it would reproduce an off-by-one rather than a modelling choice.

### Price of closing it

One literal, and it buys a wrong answer at one count. No test pins the count-15
case in either direction.

**Status:** open and deliberate. Written up for upstream in
`docs/bochs-upstream-bugs.md`.

---

## D9 — A quadword shift by exactly 64 in the XMM-register forms

**Bochs:** `cpu/simd_int.h xmm_psrlq` clears the register only when
`shift_64 > 64`, and `xmm_psravq` sign-fills an element only when `shift > 64`.
A count of exactly 64 falls through to a shift by the operand's full width —
`op->xmm64u(n) >>= shift` and `op1->xmm64s(n) >> shift` — which C++ leaves
undefined, so the answer is whatever the compiler that built Bochs made of it.
`xmm_psrlq` serves every XMM-register PSRLQ form: SSE2, VEX and EVEX, with
the count in a register or an immediate (`cpu/decoder/ia_opcodes.def`,
`cpu/decoder/ia_opcodes_evex.def`). `xmm_psravq` serves EVEX VPSRAVQ. Every
sibling helper — `xmm_psrlvq`, `xmm_psraq`, `xmm_psllq`, `xmm_psllvq` — bounds
at `> 63`.

**rusty_box:** `cpu/sse.rs psrlq_vdq_wdq` / `psrlq_udq_ib` (SSE2) and
`cpu/avx.rs vpsrlq_reg` / `vpsrlq_imm` (VEX) shift only below 64 and otherwise
write zero; `cpu/avx512.rs evex_vpsrlq_imm` / `evex_vpsrlq_reg` write zero, and
`evex_vpsravq` writes each element's sign, for a count of 64 or more.

### What the guest observes

At a count of exactly 64, a logical shift yields zero and VPSRAVQ yields each
element's sign bit replicated — the SDM's results for any count above 63.
Bochs yields an undefined result. Every other count agrees.

### Why the divergence is the correct side

There is no defined Bochs behaviour to match. Reproducing it would mean picking
one compiler's lowering of undefined code and calling it the machine.

### Price of closing it

None to pay. `rusty_box/tests/sse_qword_shift_count.rs` pins the SSE2 forms:
PSRLQ by register and by immediate zeroes both quadwords at a count of 64 and
still shifts at 63. The same file pins PSLLQ at both counts, which is plain
parity with `xmm_psllq`. No test pins the VEX or EVEX forms, or VPSRAVQ, at a
count of 64.

**Status:** open and deliberate. Written up for upstream in
`docs/bochs-upstream-bugs.md`.

---

## D10 — CMOS Status Register A accepts the divider-chain TEST values

**Bochs:** `iodev/cmos.cc bx_cmos_c::write`, `case REG_STAT_A`: a divider-chain
control of 3, 4 or 5 (the MC146818's TEST settings) raises
`BX_PANIC(("CRA: divider chain control 0x%02x", dcc))` before the register is
stored. A panic's built-in action is to ask the user on a GUI build and to quit
otherwise (`logio.cc logfunctions::default_onoff`), and the sample `.bochsrc`
sets `panic: action=ask`. A run that continues past the panic stores
`value & 0x7f` beneath the read-only UIP bit and calls `CRA_change`.

**rusty_box:** the `REG_STAT_A` arm of `BxCmosC::write` (`iodev/cmos.rs`)
stores the same bits and calls `cra_change` (Bochs `CRA_change`), with no
panic.

### What the guest observes

A guest that writes a TEST value keeps running, in the state Bochs reaches when
it continues past its own panic. `CRA_change` and `cra_change` both treat a
divider with bit 1 or bit 2 set as running, so the periodic interrupt keeps the
rate its low nibble selects. `bx_cmos_c::one_second_timer` and
`BxCmosC::one_second_timer` both stop the clock only for the divider-reset
values 6 and 7, so the update cycle keeps running too.

### Why the divergence is the correct side

The panic is a guest-triggerable halt of the host process, or a modal prompt on
a GUI build, reachable with two `OUT` instructions from any guest with port
access. No guest can depend on it. The only continuation Bochs defines is the
one this port takes, and the two are bit-identical from there on.

### Price of closing it

Reproducing it means giving the guest a register write that stops the
emulator. No test writes a TEST value.

**Status:** open and deliberate.

---

## D11 — A host byte reaches the UART's receiver at once, not one character time later

**Bochs:** `iodev/serial.cc bx_serial_c::rx_timer` polls the port's backend — a
terminal, socket, pipe, raw serial device or the serial mouse — from a
one-shot that re-arms every `databyte_usec`, the character time at the
programmed baud rate and word length. Without a FIFO it takes a byte only while
`rxdata_ready` is clear, polls again at four times the character time while it
is set, and polls an empty backend again after 100 ms. Each fire hands
`rx_fifo_enq` at most one byte.

**rusty_box:** `Emulator::pump_gui_input` (`emulator/run.rs`) passes every byte
the front end has queued (`BxGui::get_pending_serial_input`) to
`BxSerialC::receive_byte` (`iodev/serial.rs`) in one pass, and each goes
straight to `rx_fifo_enq`. From there the receive path — FIFO trigger levels,
the character timeout, overrun, LSR — is Bochs's.

### What the guest observes

Bytes arrive back to back, and a burst larger than the receiver holds loses
bytes. With the FIFO enabled, bytes queue until it holds 16 (`FIFO_SIZE`), and
each byte after that sets the overrun error and is dropped. Without a FIFO,
every byte after the first overwrites RBR and sets the overrun error. Under
Bochs the backend keeps what the receiver has no room for: without a FIFO
nothing is taken while `rxdata_ready` is set, and with one a byte arrives only
once per character time, so a guest that reads its UART promptly loses nothing.
A guest timing the gap between characters sees none here.

### What justifies it

The owner's ruling, recorded when the transmit side was paced to the baud rate:
commit `db43214` states that RX byte arrival "intentionally stays immediate".
No measurement stands behind the ruling. The structural difference it rests on
is real: this port has no backend for a timer to poll. Its only serial source is
the front end's queue, which host input fills, so pacing it means holding bytes
in the device rather than in the host.

### Price of closing it

A per-port queue of host bytes, drained by a receive one-shot at
`databyte_usec` as Bochs's `rx_timer` is, built the way the transmit one-shot
already is (`TimerOwner::SerialTx`, `pc_system.rs`), and carried in the serial
snapshot section.

**Status:** open and deliberate.

---

## D12 — HPET routes 16–23 reach only the IOAPIC

**Bochs:** `iodev/hpet.cc bx_hpet_c::update_irq` raises and lowers a timer's
interrupt through `DEV_pic_raise_irq` / `DEV_pic_lower_irq` unless the timer is
FSB-routed. Except for timers 0 and 1 in legacy-replacement mode (routed to 0
and `RTC_ISA_IRQ`), the line is `timer_int_route`, the five-bit route field of
the timer's configuration register, so routes 16–23 are reachable in either
mode. The macros reach
`iodev/pic.cc bx_pic_c::raise_irq` / `bx_pic_c::lower_irq` with
`BX_IRQ_TYPE_ISA`. Both functions assume a legacy line: they pick
`(irq_no < 8) ? master_pic : slave_pic` and index `IRQ_in[irq_no & 7]`. So a
route of 16–23 lands on slave line `route & 7`, ISA IRQ 8–15. The forward to
IOAPIC pin `route` sits inside the same function, after the slave line's
bookkeeping.

The routes are advertised and writable. `hpet.cc` puts `HPET_ROUTING_CAP`
(`0xffffff`, all 24 IOAPIC inputs) in every timer's capability field, and
`HPET_TN_CFG_WRITE_MASK` (`0x7f4e`, `iodev/hpet.h`) keeps every bit of
`HPET_TN_INT_ROUTE_MASK` writable. This port's `iodev/hpet.rs` carries both
constants unchanged.

**rusty_box:** `Emulator::drain_hpet_pending` (`emulator/timers.rs`) sends a
route below 16 through `set_isa_level`. That is the legacy path, and like
Bochs's it also forwards to the IOAPIC. A route of 16–23 goes to
`IrqFabric::set_ioapic_pin` (`iodev/irq.rs`) and nowhere else. A route of 24
or above is logged and dropped. The capability field does not advertise those
routes, but the route field can hold them. Bochs folds them onto the slave PIC
in the same way, and its `iodev/ioapic.cc bx_ioapic_c::set_irq_level` ignores
a pin at or above `BX_IOAPIC_NUM_PINS` (24).

### What the guest observes

A timer routed to GSI 16–23 raises that IOAPIC pin and nothing else. An
APIC-mode OS may pick such a GSI from the routing-capability field. Under
Bochs the same timer also drives an unrelated ISA line on the slave 8259:

| HPET route | slave line (`route & 7`) | ISA IRQ |
|---|---|---|
| 16 | 0 | 8 (RTC) |
| 20 | 4 | 12 (PS/2 mouse) |
| 22 | 6 | 14 (primary IDE) |

`pic.cc` then produces four faults:

- **A phantom edge.** An edge-triggered fire is `lower_irq` then `raise_irq`,
  so the 8259 records an edge on IRQ 8, 12 or 14 that no device raised.
- **A device's own assertion cleared.** The ISA bit of that `IRQ_in` slot is
  shared with the device that owns the line. An HPET deassert
  (`update_irq(timer, 0)`: the guest clearing a level timer's status bit, a
  configuration write to an enabled timer whose status bit is clear, or
  `hpet_del_timer` when the HPET is disabled or reset) runs `lower_irq`. That
  clears the shared bit and, once `IRQ_in` is empty, the device's pending IRR
  request with it. To the 8259 the device's line then reads low until the
  device raises it again. The lower half of an edge does the same for an
  instant, but the raise half sets the bit and a request again at once, so
  the device's request merges with the HPET's rather than being lost.
- **A lost HPET interrupt.** `raise_irq` forwards to IOAPIC pin `route` only
  on the branch that finds the slave line's IRR bit clear. A level-triggered
  timer is raised with no lower first. So while that ISA line already has a
  request in the slave's IRR, the pin the guest programmed is never raised.
- **A host panic.** When the slave line already carries a non-ISA assertion,
  `raise_irq` hits `BX_PANIC("ISA IRQ %d lost")`. D10 describes what a panic
  does to the run.

### Why the divergence is the correct side

A route of 16 or above names an input that exists only on the IOAPIC. The 8259
pair has lines 0–15. The `& 7` fold is an out-of-range index into a
two-controller model, not a modelling choice. What it produces is the four
faults above, and no guest can depend on any of them. Matching Bochs would
give the guest a phantom IRQ, a cleared device line, a lost timer interrupt,
and a register write that stops the emulator. The owner ratified the port's
behaviour on 2026-07-25 as a closed decision, as the comment at the
`drain_hpet_pending` site records.

### Price of closing it

Folding routes 16–23 onto the legacy path the way Bochs does. That buys the
four faults and nothing else. No test pins a route of 16 or above in either
direction.

**Status:** deliberate. The owner ratified it on 2026-07-25 as a closed
decision. Written up for upstream as entry 1 of `docs/bochs-upstream-bugs.md`.

---

## D13 — A GeForce access past the end of VRAM or PCI config space reads 0 and drops its write

**Bochs:** `iodev/display/geforce.cc bx_geforce_c::vram_read8` through
`vram_read64` and `vram_write8` through `vram_write64` index
`s.memory[address + n]` with no bound, and `bx_geforce_c::svga_init_members`
allocates `s.memory` at exactly `s.memsize` bytes. A caller that bounds an
access by its first byte leaves the rest of it free to run past the end.
`bx_geforce_c::svga_read`'s RMA data read (CRTC `0x38` index 2) checks
`offset < s.memsize` and then calls `vram_read32(offset)`, so an offset in the
last three bytes of VRAM reads up to three bytes past the allocation.

`bx_geforce_c::register_read32`'s `0x1800`–`0x18FF` arm mirrors PCI config
space into BAR0. It assembles a dword from `pci_conf[offset + 0]` through
`pci_conf[offset + 3]`, and `pci_conf` is the 256-byte array of
`bx_pci_device_c` (`iodev/iodev.h`). So a dword at `0x18FD`–`0x18FF` reads up
to three bytes past the array. `bx_geforce_c::register_write32` hands the same
offset to `bx_geforce_c::pci_write_handler`, which stores
`pci_conf[address + i]` past it too. C++ leaves each of those accesses
undefined.

**rusty_box:** `rusty_box_devices/src/display/geforce.rs`
`BxGeForceC::vram_load` / `vram_store`, which every `vram_read*` /
`vram_write*` goes through, read a byte past the end of VRAM as 0 and drop a
write to one. `register_read32`'s `0x1800` arm reads a byte past `pci_conf` as
0, and `register_write32`'s drops it. Bytes inside VRAM are read and written as
in Bochs, and bytes inside config space read as in Bochs.

### What the guest observes

Only an access that runs past the end. An RMA dword read two bytes before the
end of VRAM returns the two bytes that exist in its low half and zero above
them. A BAR0 dword read at `0x18FE` returns config bytes `0xFE` and `0xFF` and
zero above them. Under Bochs the same reads return whatever the build placed
after the allocation or after `pci_conf` (on a typical build, heap memory or
the start of the next member, `pci_bar`), and the config-space write
overwrites it.

No guest reaches either path today, because no machine instantiates the card.
`rusty_box/src/iodev/mod.rs` re-exports `BxGeForceC` as a model ported ahead
of its wiring. The entry settles the ported-ahead model's rule before the card
is wired.

### Why the divergence is the correct side

There is no defined Bochs behaviour to match. Reproducing it would mean picking
one compiler's lowering of undefined code and calling it the card, as in D9.
Rust cannot index past the end without a panic, and a guest-triggerable host
abort is not an answer either (D10 makes the same argument). Reading 0 and
dropping the write leaves the in-range bytes of a straddling access exactly as
Bochs has them.

### Price of closing it

Nothing to buy.
`an_rma_read_straddling_the_end_of_vram_returns_the_bytes_that_exist` and
`a_dword_read_at_the_end_of_pci_config_space_completes` (`geforce.rs`) pin the
rule. They pin the read side only; no test writes past either end.

**Status:** open and deliberate; the ported-ahead GeForce model only.

# Hypervisor-engine divergences (`H<n>`)

A machine running its guest on `rusty_box_whp_engine` executes on the host's
own processor, and there are things a hypervisor will not let a port control.
These are numbered `H<n>` rather than `D<n>` because they hold **only** for a
machine on that engine — the interpreter has none of them, and a machine on
the interpreter is the reference both are measured against.

The mixed-engine equality tests in `rusty_box_whp_engine` are how each entry
here stays honest: anything a guest can observe differently between the two
engines and is NOT listed here is a bug.

## H1 — A20 is recorded but never masks an address

**Bochs:** `memory/misc_mem.cc` applies the A20 mask to every physical access,
so a guest with the gate closed sees addresses at and above 1 MiB wrap.

**rusty_box on the hypervisor:** the guest-physical map installed in the
partition is derived by `memory/plan.rs`, which does not model A20. The gate is
still recorded — `pc_system`'s A20 state, both controller mirrors and the
port-92 and keyboard paths all work exactly as they do under the interpreter —
but the hardware serves the unmasked address.

### What the guest observes

A guest that closes the gate and reads at 1 MiB reads the byte at 1 MiB rather
than the one at 0. Real-mode software that relies on the wrap to address the
high memory area is the case that notices.

### Why it is not merely unfixed

No hypervisor exposes A20: there is no A20 in KVM's or WHP's interfaces, QEMU's
accelerators contain no `a20` handling at all, and VirtualBox's NEM backend
documents the same absence. The map is the only lever an engine has, and
expressing a wrap in a map means aliasing every page above 1 MiB to its
counterpart below — which the plan's window model cannot say and the platform
would charge a mapping for.

### Price of closing it

The guest-physical map would have to carry aliases, and every window above
1 MiB would need re-mapping on each gate change. A BIOS toggles A20 during
early boot, so the cost lands exactly where boot time is measured. Not paid
because the guests this engine targets enable A20 and leave it enabled.

**Status:** open and deliberate.

## H2 — The time-stamp counter is the host's

**Bochs:** `cpu/proc_ctrl.cc get_tsc` derives the counter from the emulated
processor's own retired-instruction count, so guest time and the TSC advance
together and both are this port's.

**rusty_box on the hypervisor:** neither `RDTSC` nor `RDMSR` of
`IA32_TIME_STAMP_COUNTER` exits, so both answer from the host's counter. Every
other model-specific register IS taken — see `TRAPPED_MSRS` in the engine —
which is what makes this one entry rather than a category.

### What the guest observes

A TSC that runs at the host's rate and starts wherever the host's was, rather
than one that counts this machine's own instructions. A guest calibrating the
TSC against the PIT gets a host-derived frequency.

### Why the divergence is *this* shape rather than the other

Taking the TSC without taking `RDTSC` would be worse, and taking both would be
worse still. The shadow processor retires almost no instructions while the
guest runs on hardware, so a TSC derived from its instruction count barely
moves; a guest reading a frozen TSC either divides by zero calibrating or spins
forever waiting for it to advance. Two clocks that disagree — a trapped MSR
read answering from the frozen shadow while `RDTSC` answers from the host — is
the worst of the three. So both stay with the hardware, together.

### Price of closing it

A cross-engine TSC bridge: the engine would have to drive this port's counter
from elapsed host time at the machine's own rate, the way it already converts a
slice's duration into ticks, and then take `X64RdtscExit` plus the two TSC
bits in the MSR exit bitmap. `X64RdtscExit` also puts an exit on the hottest
instruction a calibrating guest executes, so it needs a measurement before it
is worth having. Both bits are one named constant away in
`rusty_box_whp::MsrExits`.

**Status:** open and deliberate. The lever exists and is named; what is missing
is the bridge and the measurement.

## H3 — `CPUID` withholds VMX and SVM

**Bochs:** `cpu/cpuid.cc bx_cpuid_t::get_std_cpuid_leaf_1_ecx` sets leaf 1
`ECX[5]` (`BX_CPUID_STD1_ECX_VMX`) when the model enables `BX_ISA_VMX`.
`bx_cpuid_t::get_ext_cpuid_leaf_1_ecx` sets leaf `0x80000001` `ECX[2]`
(`BX_CPUID_EXT1_ECX_SVM`) when the model enables `BX_ISA_SVM`.
`cpu/cpudb/intel/corei7_skylake-x.cc` enables VMX in a build with
`BX_SUPPORT_VMX >= 2`, and `cpu/cpudb/amd/ryzen.cc` enables SVM in a build
with `BX_SUPPORT_SVM`. This port's models give the same answers:
`LEAF1_ECX_BASE` (`cpu/cpudb/intel/core_i7_skylake.rs`) includes `VMX`, and
`LEAF8_0000_0001_ECX_BASE` (`cpu/cpudb/amd/amd_ryzen.rs`) includes `SVM`.

**rusty_box on the hypervisor:** `withhold_virtualisation_from`
(`rusty_box_whp_engine/src/engine.rs`) clears those two bits. Leaves 1 and
`0x80000001` are both in `TRAPPED_CPUID_LEAVES`, so a guest's `CPUID` of
either one exits. `Servicer::finish_the_errand`
(`rusty_box_whp_engine/src/vcpu_thread.rs`) runs the instruction on the shadow
through the interpreter's own handler. It then clears the bit in the shadow's
`RCX`, before `Exchange::export_imported` writes the registers back, so the
shadow and the partition hold the same answer. The interpreter answers from
the model unchanged.

### What the guest observes

On the default Skylake-X model, leaf 1 `ECX[5]` reads clear. On the Ryzen
model, leaf `0x80000001` `ECX[2]` reads clear. The guest is told it has no
hardware virtualisation. So the Bochs BIOS's `smp_probe` (`bios/rombios32.c`)
skips its `IA32_FEATURE_CONTROL` write, which it gates on
`cpuid_ext_features & CPUID_EXT_VMX`, and a guest hypervisor finds nothing to
enable.

The withholding has two limits:

- **Only the `CPUID` bits change.** The model still enables its
  virtualisation extension on the shadow (`IsaVmx` on Skylake-X, `IsaSvm` on
  Ryzen). `CpuCapabilities::narrow` (`rusty_box_gui/src/config.rs`), which the
  GUI applies for `--engine whp`, excludes neither.
- **Only `finish_the_errand` withholds.** A `CPUID` that never passes through
  it gets the unwithheld answer. The case that exists is a `CPUID` inside a
  system-management handler, which `take_the_signalled_smi`
  (`vcpu_thread.rs`) runs through `run_the_shadow_out_of_smm` (`engine.rs`).

Outside a system-management handler, no single guest sees both answers today,
because no engine transfer is built (H7). The difference otherwise shows only
between two machines, one on each engine.

### Why the divergence is the correct side

It follows H4's rule: advertise nothing the guest cannot execute. A guest told
it has VMX or SVM will use it, and this engine can honour neither:

- The seam cannot carry it. `VcpuArchState` has no place for the VMCS or VMCB
  caches (`cpu/arch_state.rs`, module documentation), so a guest running a
  hypervisor of its own could not be serviced across an exit.
- `rusty_box_whp_engine` asks the platform for no nested-virtualisation
  property on its partition.

The measurement recorded when the withholding was added
(`docs/whp-guest-capabilities.md`, 2026-08-31): with `ECX[5]` advertised, the
Bochs BIOS believed it and wrote `IA32_FEATURE_CONTROL` to lock VMX on. The
platform refused the write, in firmware that had no interrupt descriptor table
yet, and the guest triple-faulted at `0xE1E80` in `rombios32` before the boot
loader ran.

### Price of closing it

Offering VMX or SVM on this engine takes both halves above: a partition
configured for nested virtualisation, and an architectural-state seam that
carries the VMCS and VMCB caches. Neither exists.

A narrower change would keep the bits withheld and make both engines agree,
in line with H4's rule that the processor is a machine setting, not an engine
one. `CpuCapabilities::narrow` would exclude `IsaVmx` and `IsaSvm`, and the
`CPUID` handler (`cpu/soft_int.rs`, `cpuid`) would clear `ECX[5]` / `ECX[2]`
when the extension is absent, as it already clears leaf 1 `ECX[3]` for
`IsaMonitorMwait`. Not done.

**Status:** open and deliberate; hypervisor engine only. No test exercises the
withholding yet.
`a_guest_on_hardware_is_told_the_same_processor_the_interpreter_tells_it`
(`rusty_box_whp_engine/src/lib.rs`) runs leaf 1 but compares only `EAX` and
`ECX[31]`.
`a_cpuid_exit_on_the_thread_answers_on_the_shadow_and_exports_the_answer`
runs leaf 0, which the function leaves alone. A gated test would pin it by
running leaf 1 on the thread and asserting that `ECX[5]` reaches the guest
clear.

## OPEN QUESTION — what may cut short a trapped instruction

Not a divergence yet, and recorded here so it is not silently settled by
whoever next reads the code.

An engine that traps an instruction hands it to the shadow processor, and the
interpreter's REP handlers (`cpu/string.rs`, e.g. `rep_movsb16`) stop between
items on `if self.async_event != 0`. That word holds two different kinds of
thing:

- **Deliverable events** — an interrupt with IF, an NMI, an SMI. x86 says a
  repeated instruction may be interrupted between items, so these must stop it.
- **Trace bookkeeping** — `BX_ASYNC_EVENT_STOP_TRACE` set by any taken branch,
  by self-modifying code, and by `tickn_fastrep` reaching a device deadline;
  `BX_ASYNC_EVENT_SCHEDULER_BOUNDARY` set by a device latching machine work.

For the interpreter, stopping for the second kind is free and right: it services
the bookkeeping in the next breath. For an engine servicing a trap it is
ruinous and pointless — the machine cannot act on a queued boundary until the
whole slice ends, so all the stop achieves is to end the instruction after ONE
item, and the guest re-traps for the next one. A `REP INSW` reading one disk
sector then costs 256 exits and 512 whole-architectural-state exchanges instead
of one.

**Decided provisionally** (2026-08-28): `PcIo::finish_the_instruction` parks the
bookkeeping bits for the duration of a trapped instruction and puts them back
after, so only a deliverable event stops it — plus one deliberate exception, a
device deadline coming due, which is asked directly of `pc_system` rather than
through the shared `STOP_TRACE` bit. That exception exists so divergence **D3**
keeps working on both engines: this port delivers a timer interrupt in the
middle of a long string burst rather than at the end of it, and an engine that
ran on would be the one diverging.

**What to revisit.** Whether the two kinds should share a bit at all. Splitting
them — a `TraceControl` word separate from `async_event` — would remove the
parking, remove the need to ask `pc_system` a second question that the
`STOP_TRACE` bit was already answering, and make both engines say what they
mean. It touches every `async_event` site in `cpu/`, which is why it was not
done under a boot bring-up.

## H4 — A machine may be narrowed to the processor every engine can run

**Off by default.** `--cpu-capabilities preset` is the default and answers
`CPUID` byte for byte as Bochs does; the divergence exists only when a caller
asks for `host-shared`.

### What the guest observes

Fewer feature bits than this port's `cpudb` model describes. On the host this
was measured against, a 12th Gen Core i5-12450H: no AVX-512 (the silicon has
none) and no `MONITOR`/`MWAIT` (the platform has no way to give a partition
either instruction — `WHV_PROCESSOR_FEATURES` has no such field).

### Why it is not merely a smaller machine

Because the alternative is not a larger machine, it is a dead one. A guest reads
`CPUID` once and commits: Linux selects `mwait_idle` at boot and never
reconsiders, and enables `XCR0` components the instant it sees the bits. Measured
consequences of advertising what could not be executed — an
`WHV_E_INVALID_VP_STATE` on the next register write for AVX-512, and
`Oops: invalid opcode` in `swapper/0` ending in "Attempted to kill the idle task"
for `MWAIT`. Neither failure names its cause.

Every hypervisor narrows `CPUID` for this reason; `CPUID` exits unconditionally
under VMX, so none of them has the option not to. KVM masks to host support;
VMware's EVC masks to a cluster baseline precisely so a guest survives the
hardware under it changing, which here is a switch between engines.

### Why it is a machine setting and not an engine one

A guest keeps what it enabled across a switch from the hypervisor to the
interpreter. If the two engines offered different processors, the switch would
change the hardware under a running guest — and the switch is the point: drive
the guest fast on hardware, then interpret it.

### Price of closing it

Nothing to close on the interpreter, which is exact under the default. Closing it
on the hypervisor would need the platform to gain what it lacks — AVX-512 on
silicon that has none is not a software matter, and `MONITOR`/`MWAIT` has no
partition property at all.

Full reasoning, the three terms that bound what may be offered, the two `CPUID`
answer paths, and what other hypervisors do: `docs/whp-guest-capabilities.md`.

## H5 — Device time is host time, and a tick is a unit rather than an instruction

**Fast mode and precise mode.** In this registry, *fast mode* is a machine on
`rusty_box_whp_engine`, and *precise mode* is the same machine on the
interpreter. H5 onward use the two terms.

**Bochs:** `pc_system.h bx_pc_system_c::tick1` is called once per retired
instruction and `pc_system.cc bx_pc_system_c::countdownEvent` advances
`ticksTotal` by the period it just crossed, so the machine's tick count IS its
instruction count. Every device deadline is stored as an absolute `ticksTotal`
value, and `pc_system.cc bx_pc_system_c::time_usec` divides that count by the
configured `ips` to answer in microseconds. An emulated second is however long
the host takes to retire `ips` instructions.

**rusty_box in fast mode:** `ticks_total` (`pc_system.rs`) is advanced by the
device thread from elapsed host time multiplied by `ips`. The guest's
instructions retire on the host's processor and contribute nothing. `ips` names
the size of a tick — how many ticks a host second is worth — and no longer names
a speed. Precise mode is unchanged: `tick1` there is still one tick per retired
instruction.

### What the guest observes

A machine whose second is the host's second. Every counter the guest can read —
the PIT, the ACPI PM timer, the HPET, the RTC — advances at its architectural
frequency against wall time, and `RDTSC`, which is already the host's counter
(H2), advances against those counters at the true ratio. Under the interpreter's
rule the same guest sees all of them advance at whatever rate instructions
happen to retire.

Deadlines are unaffected in the unit they are expressed in. A timer armed at
absolute tick N fires at tick N under either driver, so the per-timer snapshot
record — flags, period, absolute `time_to_fire`, owner, id — means the same thing
on both. The snapshot section also carries `ips` and its restore rejects a
mismatch, so an image restores only into a machine whose tick is the same size.

### Why the divergence is the correct side

Every guest-visible clock must be a fixed-ratio function of ONE monotonic time
base. On this engine two of them are settled before the device clock gets a
vote: the TSC is the host's (H2) and the LAPIC timer is the hypervisor's. Putting
the device clock on any other base is the two-clocks failure, enumerated in
`docs/research/whp-2026-09-03/05-guest-timekeeping.md` §4 — Linux's `check_timer`
panics with "IO-APIC + timer doesn't work!" when the PIT lags TSC-time;
`pit_hpet_ptimer_calibrate_cpu` reports "PIT calibration deviates" and keeps a
`tsc_khz` that is wrong by the PIT/TSC ratio, which then scales `udelay`,
`loops_per_jiffy` and every driver timeout; the clocksource watchdog demotes the
TSC on skew above 0.4 % per half second. Windows 7 calibrates the TSC against
the platform timers and, on an MP guest, bugchecks 0x101
(`CLOCK_WATCHDOG_TIMEOUT`) when a secondary processor misses its clock tick.

### Price of closing it

None, because closing it IS precise mode. A machine that wants Bochs's
instruction-count clock runs on the interpreter, where it is exact. Asking the
hypervisor for the same thing means counting the guest's retired instructions,
which is precisely what handing the guest to hardware gives up.

**Status:** open and deliberate; fast mode only. The driver is the device
thread, `rusty_box_whp_engine/src/device_thread.rs`, introduced by commit
`a80b61d`: it sleeps until the machine's next device deadline and catches the
wheel up to the host clock. The catch-up is pinned by
`service_once_catches_the_wheel_up_to_the_clock_and_names_the_next_deadline`
(same file), which drives an interpreter machine and so runs on any host.

## H6 — The 8042's serial delay is a one-shot

**Bochs:** `iodev/keyboard.cc bx_keyb_c::init` registers the controller's timer
continuous and already active, at the `serial_delay` the configuration names
(`config.cc`, 150 µs by default). `bx_keyb_c::timer_handler` calls
`bx_keyb_c::periodic(1)` on every fire; `periodic` collects and clears whichever
of `irq1_requested` / `irq12_requested` is latched, raises it, and then returns
having done nothing further when `timer_pending` is zero. A byte moved into the
output buffer during one fire has its interrupt raised on the next, one period
later.

**rusty_box in fast mode:** the same 150 µs delay (`KBD_SERIAL_DELAY_USEC`,
`iodev/keyboard.rs`) is armed as a one-shot from ONE choke point (R5): every time
the device-time service runs, it reads the controller's state —
`timer_pending != 0 || irq1_requested || irq12_requested`, the predicate
`BxKeyboardC::needs_serial_tick` — and arms the one-shot for one period when that
state holds and the timer is not already armed. Whatever latches host input pokes
the device thread when it does so, so the evaluation happens then rather than at
the thread's next unrelated deadline. Precise mode keeps Bochs's continuous
registration.

Arming at the sites that latch the work instead would be wrong, and structurally
so. `activate_timer` has six callers in `iodev/keyboard.rs`, and two of them —
`kbd_enq` and `mouse_enq` — are reached when the HOST queues a keystroke or a
mouse packet, a path no guest port write passes through. An implementation that
armed only where the guest touches port 0x60/0x64 would leave that byte latched
with nothing armed to carry it, and IRQ1 would not fire until some unrelated port
access happened to arm the one-shot; at an idle shell prompt that is a dead
keyboard. The single site sits downstream of all six, and of any latch site added
later.

### What the guest observes

IRQ1 and IRQ12 at the same delay after the byte that causes them, to within one
150 µs period — which is the tolerance the continuous timer already imposes,
since a latch can land anywhere inside a period. Nothing else differs: the
controller's registers, its status bits, the output-port mirrors and the order in
which bytes move are the `periodic` path in both forms.

Both directions of the arming hold, and both are worth stating because only one
of them is obvious. Work always gets a fire: the state the one-shot is armed from
is the state every latch path has already written when the service reads it, and
the service reads it on every run. A fire that is not scheduled is one that would
have found nothing to do: the predicate is the same state `periodic` acts on, and
`periodic` returns early when it is clear.

### Why the divergence is the correct side

On host time the continuous form is a real cost rather than a bookkeeping one,
and the reason is structural rather than statistical: while that timer is
registered, an idle 8042 puts a deadline on the wheel at least every 150 µs for
the life of the machine regardless of what else is armed, so the device thread
can never sleep past 150 µs whatever the guest is doing. The machine's floor on
wake rate is then set by a controller with nothing to say rather than by any
deadline a guest asked for. Under the interpreter the same deadline is an
instruction count the CPU walks past anyway, which is why upstream can afford to
write the timer this way; on a host clock it is a thread that has to be woken.

**The cost of one such wake is not measured here.** No figure in this tree
covers it — the ≈ 4 µs in
`docs/research/whp-2026-09-03/05-guest-timekeeping.md` is the cost of a halt exit
to the VMM and does not transfer to a device-thread wake — so this entry rests on
the structural claim above and on nothing numeric. A measured wake cost, or a
wake count observed over a boot, would settle it permanently; until one exists,
read the justification as being about which party sets the floor, not about how
expensive the floor is.

### Price of closing it

Those wakes. Restoring the continuous timer on host time costs a device-thread
wake every 150 µs for the life of the machine and buys nothing a guest can see.

**Status:** open and deliberate; fast mode only. **Implemented.** The reset
arming is conditional in `Emulator::rearm_device_timers_after_hardware_reset`
(`emulator/timers.rs`), the one-shot is armed from
`Emulator::service_device_time` through `arm_the_serial_delay_if_owed`, and the
question it asks is `BxKeyboardC::needs_serial_tick` (`iodev/keyboard.rs`).

**The arming site is the whole of it, and it is not where a guest touches a
port.** `activate_timer` has six callers in `keyboard.rs`, and two — `kbd_enQ`
and `mouse_enQ` — are reached when the HOST queues a keystroke or a mouse
packet, which no guest port write passes through. Arming from the port-dispatch
tail would therefore drop host input entirely whenever the guest is idle: the
byte sits latched, the one-shot is never armed, and IRQ1 never fires until some
unrelated port access happens along. At an idle shell prompt that is a dead
keyboard. Evaluating on every service covers all six latch paths and any added
later, which a port-dispatch tail structurally cannot (R5). Pinned by
`a_keystroke_the_host_queues_arms_the_8042_although_the_guest_touched_no_port`
(`rusty_box_whp_engine/src/fast_machine.rs`), whose guest is `jmp $` — it
touches nothing, so no port write could have armed anything.

## H7 — The time-stamp counter's rate changes across an engine transfer

**Bochs:** `cpu/proc_ctrl.cc BX_CPU_C::get_TSC` returns
`bx_pc_system.time_ticks() + tsc_adjust` — the machine's own tick count, which is
one per retired instruction. There is a single rate and it never changes.

**rusty_box:** in fast mode the counter is the host's (H2).

No engine transfer exists. Neither `FastMachine::into_precise` nor
`FastMachine::from_precise` is built, so a running guest cannot move between the
engines, and this entry registers the rate change that such a transfer
produces. The seam carries the counter in neither direction today:

- The whole-state read (`rusty_box_whp_engine/src/state.rs`) stores
  `WHvX64RegisterTsc` in the architectural state's `msrs.tsc`, but
  `BxCpuC::import_arch_state` (`cpu/arch_state.rs`) does not consume that
  field, so the shadow's counter is not continued from the hardware's.
- The whole-state write never reaches the register. `Reg::Tsc` sits last in
  `MSR_REGS`, at `TSC_SLOT`, and the write skips that one index rather than
  truncating a list; `the_time_stamp_counter_is_never_written_to_the_processor`
  (`state.rs`) asserts it on the recorder rather than on a read-back.

A transfer has to carry the value both ways. Into precise mode, the
interpreter's counter continues from where the hardware's stood and then
advances at one tick per retired instruction. Back into fast mode, the
interpreter's value is written into the partition, so the hardware's counter
resumes from where the interpreter left it and returns to the host's rate.

### What the guest observes

A change of rate at the instant of a transfer, with no step and no reversal: the
counter is continuous and monotonic across the boundary, and the counts per
second before it differ from the counts per second after. Time per instruction
changes with it, for the same reason.

A guest that has calibrated the TSC against a platform timer holds a frequency
that was true of the engine it calibrated on. It learns the new rate at its next
calibration; in between, Linux's clocksource watchdog is the thing that notices,
and what it does about a TSC that no longer agrees with the HPET or the PM timer
is demote it.

### Why the divergence is the correct side

The rate is not a property of the counter but of which engine holds the guest,
and no counter can be both the host's cycle count and this machine's instruction
count at once. Fast mode takes the host's for the reasons H2 records, and the
discontinuity follows from that choice — from where fast mode's TSC comes from,
not from having a precise mode at all. The part that is not up for negotiation is
the value: continuity is taken, because a counter that stepped backwards across a
transfer would break every guest that treats the TSC as monotonic.

### Price of closing it

Buyable, and H2 has already priced it. The rate a guest can compare is the TSC
against its own timers, never against host seconds. That ratio is `cpu_hz : ips`
in fast mode — H2 keeps the TSC on host cycles while H5 puts device time on host
time × `ips` — and `1 : 1` in precise mode, where both are the instruction count.
The difference between those two ratios IS the observable this entry registers,
and H2's named cross-engine TSC bridge removes it: drive this port's counter from
elapsed host time at the machine's own rate and fast mode's ratio is `1 : 1` as
well, leaving a guest no rate change to see across a transfer — only the uniform
dilation of the whole machine that H5 already accepts for every other clock.

So the price is H2's, quoted there: an `X64RdtscExit` on the hottest instruction
a calibrating guest executes, the two TSC bits in the MSR exit bitmap
(`rusty_box_whp::MsrExits`), and a measurement before it is worth having. What
stays unbuyable is a fast-mode TSC that is both the host's cycle count and the
machine's instruction count; the bridge closes the discontinuity by giving the
first of those up.

**Status:** open and deliberate; both engines, since the divergence is the
boundary between them. The engine transfer (`FastMachine::into_precise` /
`from_precise`) is not built, and neither is the TSC hand-off in either
direction, so nothing asserts the continuity of the value today. The transfer
has to hold it against a live-transfer criterion: Alpine at `login:`,
fast → precise → fast, with `date` still advancing at wall rate and no
clocksource demotion in `dmesg`.

## H8 — A level-triggered IOAPIC entry is re-serviced on the guest's EOI

**Bochs:** `iodev/ioapic.cc bx_ioapic_c::receive_eoi` is a single `BX_DEBUG` and
does nothing else. The remote-IRR accessors in `iodev/ioapic.h`
(`set_remote_irr`, `clear_remote_irr`) have no callers, so the bit is never set
and never consulted. `bx_ioapic_c::service_ioapic` keeps a level entry's `irr`
bit through delivery — it clears the bit only when `entry->trigger_mode()` is 0 —
and `bx_ioapic_c::set_irq_level` clears it only when the line itself drops. A
level line still asserted after the handler's EOI is therefore re-serviced, but
not until some unrelated event runs the next `service_ioapic` scan.

**rusty_box in precise mode:** identical. `iodev/ioapic.rs receive_eoi` logs and
returns; `set_remote_irr` has no caller here either; `service` clears `irr` for a
pin only when its trigger mode is edge, and `set_pin_level` clears it for a level
pin only on deassert.

**rusty_box in fast mode:** the platform owns the LAPIC and reports the EOI of a
level-triggered vector as an `X64ApicEoi` exit carrying
`ApicEoi.InterruptVector`. `IrqFabric::resample_on_eoi(vector)` (`iodev/irq.rs`)
answers it by re-servicing every level entry whose vector matches and whose line
is still asserted, re-issuing the request at once.

**Provenance (R7):** QEMU `hw/intc/ioapic.c ioapic_eoi_broadcast`, which QEMU's
WHP accelerator calls from exactly this exit. Bochs has no counterpart symbol to
depart from, because nothing in Bochs is told that an EOI happened.

### What the guest observes

In fast mode: a level interrupt whose line is still asserted when the handler
writes EOI is re-delivered immediately, instead of waiting for a scan that some
other device's activity happens to trigger. That is what the hardware does — a
level entry re-asserts for as long as the line is held, which is how a shared PCI
line drives its handler around the loop until every device on it is quiet. Under
the Bochs rule the second delivery arrives late and at an unrelated moment, and a
driver that quiesces its own device inside the handler and expects the next
assertion promptly waits.

Nothing changes for an edge entry, which produces no EOI exit at all — and edge
is what the ISA lines are under normal programming. Nothing changes for a line
the handler did quiesce: the resample finds it low and issues nothing.

### Why the divergence is the correct side

The engine is the only place in this port that is told an EOI occurred, and it is
told for exactly the level vectors that need it. Declining to act on that means
the faster engine deliberately holding an interrupt the hardware would have
delivered — a divergence in the direction of being wrong, whose symptom is a hung
device rather than a message. Every hypervisor-backed VMM that owns an IOAPIC
behind a platform LAPIC resolves it the same way, which is why QEMU's function
exists to be borrowed.

### Price of closing it

Paid in the other direction: the exit arrives whether or not the resample runs,
so ignoring it restores the Bochs timing at no saving. The scan the resample runs
is the same `service` the fabric already runs on every other request.

**The cost of keeping it is a guard this port does not borrow.** QEMU's
`ioapic_eoi_broadcast` is not a bare re-service: it defers roughly 10 ms
(`timer_mod_anticipate`) once about ten thousand successive interrupts have
arrived on the same vector (`SUCCESSIVE_IRQ_MAX_COUNT`), recorded in
`docs/research/whp-2026-09-03/02-qemu-whpx.md`. This port borrows the function
without that back-off, and that omission is deliberate rather than overlooked, so
its consequence belongs here: a level line that the handler cannot quiesce
becomes an EOI → re-deliver loop with nothing damping it, where the Bochs rule
would have spaced the re-deliveries out by whatever unrelated event ran the next
scan. Whoever finds such a loop should add the back-off as part of the borrowed
behaviour under R7, not treat the resample itself as the defect.

**Status:** open and deliberate; fast mode only. Introduced by commit
`a614588`: `IrqFabric::resample_on_eoi` (`rusty_box/src/iodev/irq.rs`), called
from the vCPU thread's `ExitReason::ApicEoi` arm
(`rusty_box_whp_engine/src/vcpu_thread.rs`).
`an_eoi_re_services_a_level_entry_whose_line_is_still_asserted`
(`iodev/irq.rs`) pins the resample without a hypervisor. No test yet drives a
level-triggered EOI exit on hardware.

## H9 — The legacy 8259 line is placed as a pending event, not taken by an INTA at the processor

Fast mode only (`DeviceClock::HostTime`). The interpreter is unaffected and keeps
Bochs's behaviour exactly.

### What the guest observes

Bochs raises a flag and reads the vector only once the processor is ready to take
it: `pc_system.cc raise_INTR` carries no vector, and `cpu/event.cc` calls
`DEV_pic_iac()` at the moment of delivery. This engine acknowledges earlier. The
vCPU thread tests the guest's readiness from its last exit header — `IF` set, no
interrupt shadow, no delivery already in flight — and, if the guest can take one,
performs the acknowledge and places the resolved vector as a
`WHvX64PendingEventExtInt` before re-entering the partition.

So the vector leaves the 8259 a few instructions earlier than Bochs would take
it. A guest that masks that IRQ in the window between the acknowledge and the
delivery still receives it.

### Why the divergence is the correct side

There is no INTA cycle to join: the processor is inside the hypervisor, and the
platform offers no verb for the wire. `WHV_INTERRUPT_TYPE` has no ExtINT(7) and
no LocalInt0(8), so LINT0 cannot be asserted at all.

The obvious alternative is measured impossible rather than merely worse.
`WHvRequestInterrupt` refuses vector `0x08` with `0xC0350005`
(`ERROR_HV_INVALID_PARAMETER`), because a local APIC takes no vector below 16 —
this port's own `BX_LAPIC_FIRST_VECTOR`, and Bochs `cpu/apic.cc trigger_irq`,
reject the same. Remapped above the floor it is accepted and the vector vanishes,
because the partition's APIC is software-disabled at reset and a legacy guest
never enables it.

QEMU places the identical event for the identical reason
(`target/i386/whpx/whpx-all.c whpx_vcpu_pre_run`), so this is the platform's
intended path rather than this port's invention.

### Price of closing it

Not closable while the guest runs on the hypervisor: closing it means reading the
vector at delivery, and the delivery happens inside hardware this process does
not observe instruction by instruction. Single-stepping to recover the INTA
moment is measured 416× slower and would defeat the reason for using the
hypervisor at all.

The same acknowledge-early behaviour is already this port's answer for an I/O
APIC entry in ExtINT mode, which is the other way a PC wires this controller, so
closing it here alone would make the two paths disagree.

**Status:** open and deliberate; fast mode only. The placement is
`stage_the_legacy_interrupt` (`rusty_box_whp_engine/src/vcpu_thread.rs`). A
cancel fetches a processor out of its run only when `VcpuControl::in_run` says
it is inside one (commit `a5a5336`): the platform latches a cancel sent to a
processor outside a run and spends it on the next entry, which then retires
nothing. Two hardware tests in `rusty_box_whp_engine/src/lib.rs` prove the
delivery: `a_legacy_8259_vector_reaches_a_hardware_guest_as_a_placed_ext_int`
for a guest spinning with IF set, and `a_halted_guest_still_takes_its_legacy_tick`
(commit `8d334de`) for one parked in `HLT`. Both return without asserting on a
host where the hypervisor platform is unavailable.
