# WHP fast path and the two-API redesign

> Date: 2026-08-29. Status: DESIGN, awaiting approval. Breaking changes are
> authorised by the owner ("we have no users yet").
>
> This supersedes REPLAN v4's assumption that one engine seam serves every
> consumer identically. It does not supersede REPLAN v4's crate graph, its
> arch-neutral ambition, or the doctrine rules R0–R9.

## 1. The problem, measured

The hypervisor engine is **slower than the interpreter**, which defeats its
purpose. The cause is not exit cost. It is that the engine barely runs the
guest at all.

| Measurement | Value | Source |
|---|---|---|
| Time inside `WHvRunVirtualProcessor` per slice | **4.4 µs** | `SliceOutcome.ran`, this tree |
| Wall time per slice | **26.5 µs** | same |
| One bare WHP round trip, no host work | **4.0–4.4 µs** | `docs/whp-platform-probe-2026-08-27.md`, finding 4 |

Two exits cannot fit in 4.4 µs. Therefore:

> **The average slice contains at most ONE exit, and the guest's execution time
> per slice is below the resolution of the measurement. The engine pays 26.5 µs
> of wall time to buy one VM entry and one VM exit.**

The mechanism is in `run_the_exit_loop`: after servicing any exit,
`io.sync_io_events(cpu)` runs, and `sync_io_events` exists precisely to make
`needs_boundary()` and `has_an_event_to_deliver()` true — which ends the slice.
Note 26.5 µs is *below* `SLICE_RESOLUTION = 50 µs`, so slices are not ending on
the alarm or the tick budget; they end on `Boundary`.

### The break-even that governs everything

One exit costs 4.4 µs. **This port's own interpreter rate, measured, spans a
wide band and the band matters:**

| Workload | Rate | Source |
|---|---|---|
| Branch-heavy microbench | 157 M ips (6.4 ns/insn) | `docs/perf/PERFORMANCE-INVESTIGATION.md:80` |
| Alpine boot, 4 G instructions | 83.5 M ips (12.0 ns/insn) | `:146` |
| DLX headless boot to login | **30 M ips** (450 M insns / 15.0 s) | this session |

The DLX figure is the honest one for a device-heavy boot, and the one the M1
gate uses. At 30–157 M ips the interpreter retires **130–690 instructions** in
one exit's time.

> **The hypervisor loses to the interpreter whenever the guest exits more often
> than once per ~130–690 instructions.** With today's 22.1 µs of per-slice
> overhead folded in, that worsens to roughly one exit per 660–3 500.

### What a fixed engine should achieve

wtf measured whv at **15× bochscpu** (0.1 → 1.5 exec/s over ~195 M instructions
per execution), and its author called that disappointing against an expected
100×. Converting: **bochscpu ≈ 19.5 M ips, whv ≈ 292 M ips.**

Note bochscpu is a *different emulator*, not this one: at 30–157 M ips this
port's interpreter is already faster than the baseline wtf improved on. So the
realistic WHP target here is **a few times our own interpreter, not 100×** —
and on a device-heavy boot, less than on wtf's device-free workload.

The counter-datum matters as much: on *short* executions, same machine, same
target, wtf reports **bochscpu 11 300 exec/s against KVM 2 400 — the emulator
4.7× faster** (`wtf/linux_mode/README.md:241` vs `:261`). Hardware wins on long
device-free stretches and loses on short ones. There is no configuration in
which one engine wins everywhere.

### The precedent that the fix works

VirtualBox, same WinHv API, Windows 2000 boot-and-shutdown
(`NEMR3Native-win.cpp:3036-3078`, mechanism at `:2341-2349`):

| Configuration | Time |
|---|---|
| WinHv API, naive | **32 min 12 s** |
| WinHv API + exit elision | **58.66 s** |
| Native AMD-V | 58.09 s |

**33× from exit elision alone** — detecting blocks with excessive MMIO/PIO exits
and emulating several instructions per exit — reaching parity with native
virtualisation. This is our exact problem, already solved on the same API.

## 2. Decisions taken

1. **On the fast path, guest time equals host time.** Device timers move to a
   host clock, off the run thread. The closed loop (guest clock → ticks → run
   time → next device deadline → slice length) is what forces microsecond
   slices, and it goes.
2. **Two APIs, split by audience, both fast, over one engine.** A PRECISE API
   for reverse engineering, and a REGULAR API that is a VMware/Bochs
   replacement.
3. **The regular API drops Bochs parity.** It must boot real operating systems
   and be fast. Parity stays where it is load-bearing: the precise API.
4. **Replay is snapshot + input log, replayed on the interpreter.** Hardware
   never needs an instruction counter.
5. **The timing model is a policy, not a constant.** PC-on-WHP wants host time,
   PC-on-interpreter wants instruction-counted determinism, and a console (the
   owner's hypothetical) would want cycle accuracy. Three answers, so it cannot
   be hardcoded.
6. **The engine seam becomes arch-neutral.** It is being reshaped for
   performance anyway; reshaping it twice is worse. Nothing here builds another
   architecture — the test is only "does this preclude one".
7. **`no_std` / `no_alloc` is load-bearing, not legacy.** Pre-boot malware
   analysis runs in UEFI, where there is no OS and no hypervisor. That
   deployment is interpreter-only and cannot register a closure.

## 3. Which engine serves which consumer

The oracle crossover was computed from this tree's own numbers (22.1 µs fixed
cost; interpreter at 6.4 ns/insn branch-heavy, 12.0 ns/insn on a real boot):

> **Crossover ≈ 2 000–4 000 guest instructions.** Below that the interpreter
> wins.

And the measured burst length for the actual consumer, counted over
`bin_lift/examples/files/vmp_trace.asm` (17 883 instructions, 139 `jmp r10` VM
dispatches): **~130 instructions per burst** — 15–30× below the crossover.

Worse for hardware, and decisively:

> **Single-stepping on hardware costs 4.4 µs/instruction against the
> interpreter's 6.4 ns — ~690× slower.** On hardware, observation *is* exits.
> This is not a crossover to optimise past; it is a structural exclusion.

Additionally, hardware here is **actively anti-deterministic**: `state.rs:87-100`
gives the TSC to the hardware and never writes it back, so the same probe from
the same checkpoint is not guaranteed to return the same answer.

| Consumer | Deployment | Engine | Why |
|---|---|---|---|
| Regular VM | host + GUI | **WHP** | long device-free stretches |
| Snapshot fuzzer | host | **WHP** | bursts of 10⁵–10⁸ instructions |
| bin_lift oracle | host | **Interpreter** | ~130-instruction bursts; needs determinism |
| Stepping / TTD | host | **Interpreter** | 690× |
| Pre-boot scanning | UEFI, `no_alloc` | **Interpreter** | no OS, no hypervisor |
| WASM | browser | **Interpreter** | no hypervisor |

**WHP's place in the RE stack is fast-forward** — reach the interesting code at
native speed, snapshot, hand to the interpreter — not analysis.

## 4. The fast path

Four parts, in dependency order.

**4.1 Let the guest run.** Stop ending the slice on every exit. Service the exit
and re-enter. Leave the partition only when the machine genuinely must act.
This is the change that breaks the one-exit-per-slice regime, and it requires
4.2 to be safe.

**4.2 Host-anchored device time.** Device deadlines are host instants, serviced
by the machine when the guest exits anyway or when a host timer says so. This
severs the coupling that sets slice length.

**4.3 Exit elision** (VirtualBox's 33×). Track exits per guest region; when a
region exceeds a threshold, run it on the shadow for several instructions
instead of returning to hardware. The machinery exists —
`PcIo::finish_the_instruction` — built for `REP`; this generalises it from "the
current instruction" to "this hot block".

**4.4 Cheapen the exchange.** QEMU's WHPX reads **16** registers on a vmexit,
not 42, and takes `eip`/`eflags` from the exit context with no call at all
(`whpx-all.c:139`, `:710`); `whpx_vcpu_post_run` makes **zero** API calls
(`:2116`). Specifically here:

- Delete the `InterruptState` read: `ExitContext.ExecutionState` already carries
  `InterruptShadow` (bit 12) and `InterruptionPending` (bit 6) in a word we
  already read. One of five out-calls, free.
- A dirty flag (QEMU's `CPUState::vcpu_dirty`): a slice that ends without the
  machine touching CPU state needs no read-back at all.
- Stop copying the vector file and FPU in-process — ~2.4 KiB each way, twice a
  slice, for state `state.rs` already declines to send to the platform.
- Stop `tlb_flush()` + `i_cache.break_links()` on every import: 3 072 TLB
  entries, which evicts the shadow's own working set so it starts cold on the
  next trapped instruction.

**Sizing (inference):** 4.4 alone attacks the 22.1 µs and might halve it —
roughly 26.5 µs → 10–14 µs, ~2×. It does not touch the slice count. **4.1–4.3
are where the order of magnitude is.**

**Risk, stated:** every partial state exchange is a chance to skip a
derivation. `import_arch_state`'s own doc warns that assigning fields and
returning "is how a processor ends up describing one state while behaving as
another". This is the failure class that produced the TSC-stomp bug found this
session — wrong on 278 069 of 278 069 slices, and caught only by diffing the
round trip field by field. Under doctrine R2, staleness must be a **type**, not
a bool.

## 5. The API design

### 5.1 Layers

Declared in the core so every engine and deployment can satisfy them.

- **L0 — machine primitives** (`no_std`, `no_alloc`, arch-neutral): registers,
  memory, step, snapshot, PC and memory watchpoints. *The UEFI scanner uses
  this.*
- **L1 — x86/PC semantics**: syscall and interrupt events, calling conventions,
  page-table walks.
- **L2 — guest-OS introspection** (optional, per-OS): modules, threads,
  processes, symbols, syscall names. Requires walking guest structures (PEB/LDR,
  PE headers).
- **L3 — ergonomics**: Frida-shaped `Interceptor`/`Stalker` equivalents over
  L0–L2. *The IDA plugin uses this.*

The regular API is a thin opinionated facade over L0–L1. The precise API exposes
L0–L3.

### 5.2 Capability is a type, never a predicate

The central rule, and the one failure this design must not reproduce.

sogen ships `supports_global_memory_execution_hooks() == true` on its KVM
backend while its own comment says those hooks never fire, and its docs concede
"KVM may not fire read/write, execution, basic-block, or symbol callbacks". You
register a hook, get no error, and are never called.

llvmkit's D8 already solves this: `Inspect` has no `mutate()`, so over-claiming
is a compile error rather than a lie. Applied here:

```rust
pub trait Engine {
    /// What this engine can observe. A type, so it costs nothing at run time
    /// and works where a closure cannot be allocated.
    type Watch: WatchGranularity;
}

impl Engine for SoftwareEngine { type Watch = PerInstruction; }
impl Engine for WhpEngine      { type Watch = PerPage; }

impl<E: Engine<Watch = PerInstruction>> Machine<E> {
    pub fn on_instruction(&mut self, …) -> WatchId { … }
}
```

`machine.on_instruction(…)` on a WHP machine is **E0599 — no such method**.
Not a bool, not a runtime refusal, not a silent no-op. Each such rule gets a
`trybuild` fixture naming its error code, as llvmkit does; a rule without a
fixture is not a rule.

**Honest carve-outs are documented in llvmkit's register**, e.g. "wrong brand is
a compile error; wrong module under the same brand is a checked run-time
rejection". Where WHP offers only page granularity, the docs say exactly that
rather than implying instruction granularity.

### 5.3 Observability without paying for it

Frida's `Interceptor` costs ~6 µs per `onEnter` and ~11 µs with both callbacks,
because it crosses a JS boundary and must not disturb a live process. We have
neither constraint: **PC-compare at fetch**, no trampoline, no patching,
invisible to the guest, and it works in ring 0.

The model is *arm what you watch*, the inverse of Frida's follow-everything-and-
`exclude`:

| Tier | Mechanism | Cost |
|---|---|---|
| Free run | nothing armed | native |
| Watched | page permissions dropped on hooked pages (sogen's `automatic` mode); instruction-class exits | proportional to hook density, not to code executed |
| Exact | interpreter, bounded window | interpreter speed, inside that window only |

**`int3` patching is rejected** — guest-visible, and this platform studies
anti-analysis software. sogen offers it and hides it from guest reads via
`overlay_patched_breakpoints`; we decline the trade.

**Costing note from sogen, worth knowing before copying:** a permission-stripped
page runs the *entire page* at single-step speed — 2 VM exits plus 2 unmap/map
pairs per instruction executed on it, whether or not a callback matches. Cheap
for a cold page, ruinous for a hot one. Their own comment records that per-page
flushing "stalls for seconds" and coalescing "cuts the per-page cost by ~70x".

### 5.4 In `no_alloc`, the observer is a type

Runtime-registered closures need `Box<dyn Fn>`. The `no_alloc` answer is not a
degraded API but a different binding time: the observer is the compile-time
tracer parameter `T: Instrumentation`. Same visibility, chosen at build time.

**Already done (2026-08-29).** The closure-hook API is deleted: `hooks.rs`, the
`Vec<*Hook>` fields, `add_*`/`remove`, `HookHandle`, `MemHookType`,
`IoHookType`, `InstrumentationError`, and 12 public `hook_add_*` methods. The
`instrumentation` feature is gone.

That feature was itself an instance of the capability lie this design forbids:
it implied `["alloc"]`, every fire site was gated on it, but the registry field
and the `T: Instrumentation` parameter were **not** — so a `--no-default-features`
build accepted a fully-implemented tracer, stored it, and never fired a single
callback. The module doc asserted "the registry field does not exist on
`BxCpuC`", which was false. That defeated the pre-boot analysis target
precisely. The tracer now fires in every build.

## 6. What the fast path gives up

Each item is a deliberate acceptance, listed so it can be rejected individually.

| Given up | Where | Note |
|---|---|---|
| Instruction-counted guest time | regular API only | precise API keeps it |
| Bochs-exact device timing | regular API only | registered divergence |
| TSC as this port's own | WHP | already true; hardware owns it |
| Determinism / reproducibility | WHP | already true, and now stated |
| Instruction-granular observation | WHP | page granularity; a compile error to ask for more |
| Cross-engine snapshot restore | fast path | tick and host-time clocks differ |

## 7. Sequencing

**Measure before building.** The ranking in §1 rests on a deduction from two
numbers.

- **E1 — confirm the deduction.** Slice count, an exits-per-slice histogram, and
  a tally of why each slice ended, split by which check fired. **Free
  cross-check:** `WHvGetVirtualProcessorCounters` with
  `WHvProcessorCounterSetIntercepts` returns `{Count, Time100ns}` per intercept
  class, and `WHvProcessorCounterSetRuntime` gives `TotalRuntime100ns` /
  `HypervisorRuntime100ns` — the hypervisor's own accounting, no instrumentation
  needed. Predicted: median 1 exit/slice, >70% ending `Boundary`. **If instead
  the median is 5+ and slices end on `Budget`, §1's ranking is wrong and 4.4
  rises above 4.1.**
- **E2 — bucket the 22.1 µs** with `Instant::now()` for one boot: `partition.run`,
  `export_arch_state`, `state::import`, `state::export`, the `InterruptState`
  read. Nobody has ever published the cost of `WHvGet/SetVirtualProcessorRegisters`;
  it must be measured, not looked up.

Then: **M1** = 4.1 + 4.2 (the smallest change that could beat the interpreter on
a DLX boot), **M2** = 4.3, **M3** = 4.4, **M4** = the API split.

**M1's gate:** DLX boots to login on WHP *faster than the interpreter's 15.0 s*,
with `cargo xtask ci` green.

## 8. Deliberately not in scope

- Any other architecture. The test is only that nothing here precludes one.
- Interpreter throughput (a JIT). Booting Windows in WASM is interpreter-bound
  and the WHP work does nothing for it — a separate project with its own
  evidence.
- Taint tracking. Natural given we see every memory access, and PANDA proved it,
  but L0's memory-watch shape must merely not preclude it.
- The C ABI surface for IDA and friends. Its own spec, depending on this one.
- Tenet trace emission. See §9.

## 9. Relationship to REPLAN v4

This does not replace the plan. It corrects one assumption inside it and gives
several of its remaining units a purpose they did not have.

### What still stands, untouched

The crate graph (`rusty_box_core` / `_devices` / `_whp` / arch crates), the
arch-neutral ambition, doctrine R0–R9, the sealing policy, the gate suite, and
every unit already delivered — A (MMIO windows), Z (core seed), B (display
seam), C (devices crate), D (`DeviceCtx` v2 / `VmClock`), E (`IrqFabric`), H0
(the WHP probe) and H2a (`VcpuArchState`).

### What it corrects

REPLAN v4 assumed one engine seam serves every consumer on equal terms, with
`SoftwareEngine` and `WhpEngine` as interchangeable implementors. §3 shows they
are not interchangeable: they win in different regimes, separated by a
measurable crossover. The seam stays; the idea that a caller should be
indifferent to which side of it they are on does not.

It also retires "the hypervisor is the fast one" as an unqualified claim. It is
the fast one **for long device-free stretches**, which is the regular API's
workload and not the precise API's.

### Where the remaining units land

| Unit | Status under this design |
|---|---|
| **H2b** (engine seam, `Emulator<P>`, `SoftwareEngine`) | Still required, and now also the place the timing policy (§ decision 5) and `Engine::Watch` (§5.2) are declared |
| **H3** (`WhpEngine`) | Largely built. §4 is its rework; §7's M1 is the gate it never had |
| **H4** (DLX-on-WHP gate) | Unchanged as a correctness gate; §7 adds a *speed* gate beside it |
| **H5** (egui on WHP) | **This is the regular API's frontend** — see below |
| **F, G** (devices onto the API, declarations) | Unchanged |
| **L** (the device-crate move) | Unchanged |
| **P6** (API lockdown, snapshot v4) | Absorbs §5's surface; `Profile` unseals here with its full type set |
| **P7** (host-seam backends) | Unchanged |
| **P8** (`forbid(unsafe_code)`) | Unchanged |
| **P9b** (Alpine on WHP) | Now gated on §4 landing, else it inherits the same ceiling |
| **P9c** (KVM) | The second hypervisor. §5.2's capability types are what stop it repeating sogen's lie |

### H5 is not a separate feature — it is the regular API

The VMware/Bochs replacement of §2 decision 2 *is* the egui shell: build a
machine, attach a disk, power on, see the console, type into it. Everything H5
already specifies is that API's frontend:

- `rusty_box_gui` feature `hv-whp` (Windows, non-wasm, **not** default —
  `rusty_box_android` pulls default features), and `--engine whp`.
- `MachineRunner` as the std actor that owns the `SharedDisplay` mutex and
  drains `pending_*` into `keyboard().scancodes()` / `mouse().motion()`, honouring
  the `#[must_use]` accepted counts so **keystrokes are never dropped** while
  frames may be.
- The waker kicking after input and stop.
- `Box<dyn BxGui>`, `gui_trait.rs`'s `Box<dyn Fn()>` and `pump_gui_input`
  leaving the machine — two unregistered R8 erasures that this API split is the
  occasion to close.

Two consequences worth stating:

1. **H5 is where the fast path is judged by a human.** A GUI at today's speed is
   the complaint that started this redesign; §7's M1 should be visible on screen
   before H5 is called done.
2. **`MachineRunner` must stay optional.** It owns a thread, and §2 decision 7's
   deployments cannot: wasm and UEFI call `step()` directly, and an IDA plugin
   must never have the emulator own its loop. The runner is a convenience over
   the pump, never the model.

## 10. Open questions

1. **Trace format.** Tenet cannot represent flags, segments, CRs, MSRs, SIMD,
   CPL, CR3, threads, interrupts, port I/O or DMA, and carries **no instruction
   bytes** — it reads them from the IDB, so self-modifying and virtualised code
   is wrong, which is exactly the VMProtect case. A parse error silently
   truncates a 65 535-instruction segment. At ~50 B/instruction, one wtf-scale
   execution is ~10 GB. **Recommendation: Tenet as a bounded-window export, not
   an interface**; bin_lift consumes the live oracle. Not yet decided.
2. **The oracle's shape.** bin_lift's `Oracle` is pull-based per site, but the
   interpreter already *pushes* the same facts: `BranchEvent` carries
   `CnearTaken`/`CnearNotTaken`/`Ucnear`/`Far` at every control transfer. Adapter,
   or change the consumer? Note `read_memory`, `snapshot` and `restore` have zero
   live callers in `lift-core` today.
3. **Where the timing policy is declared** — on the engine, the machine, or the
   profile.
4. **Whether the regular API keeps a strict-parity switch.** Deferred; adding it
   later is additive, removing it is not.
