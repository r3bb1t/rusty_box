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
