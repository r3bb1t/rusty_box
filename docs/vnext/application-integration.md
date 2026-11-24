# Embedding Rusty Box

Three examples cover the public API at the points where an application supplies memory, observes execution, or handles a device event.

| Example | What the application owns | What it uses |
| --- | --- | --- |
| [CPU-only guest function](../../rusty_box/examples/cpu_facade/main.rs) | An x86 processor and dense RAM | Typed registers, checked code writes, an instruction hook, step, a scalar I/O exit, and resume through halt |
| [Custom PC devices](../../rusty_box/examples/composed_hardware/main.rs) | Device models and the app's tile counter | MMIO attachment, DMA, typed notifications, a stop request, and the endpoint's short IRQ loan |
| [Host-side Linux observation](../../examples/execution_workflows/src/bin/linux_host_observe.rs) | The paused guest loan for one observation | A lightweight process handler and two image handlers that share a mapping walk |

```sh
cargo run --locked --release -p rusty_box --no-default-features --features std --example cpu_facade
cargo run --locked --release -p rusty_box --no-default-features --features std --example composed_hardware
```

The CPU example maps a contiguous block of owned RAM. The downstream RAM example shows explicit physical regions backed by three caller-owned allocations and a separate indirect provider. [Custom memory walkthrough](../../examples/execution_workflows/README.md#lend-custom-ram-to-the-x86-interpreter)

The Linux example uses ordinary guest commands. COM1 carries the commands and their output. Task identity, CR3, the entry address, and executable mappings come from host reads at the existing breakpoint for the pinned Debian kernel. Use `--process-only` to register only the lightweight handler. The default run adds two handlers asking for the same `&ExecutableSample`. The first extractor invocation walks the mappings, and the second borrows the cached image. [Guest setup and run commands](../../examples/execution_workflows/README.md#sample-linux-tasks-from-the-emulator-host) · [Observation boundary](linux-host-observation.md)

## Registration and ownership

`.on()` is a natural name for subscribing to a known producer, and it matches `Subscriptions` and PC attachment methods. `.route()` expresses selection by the first argument's event type across a heterogeneous app and already has downstream callers. Keep `.route()` as the sole app registration method. A rename would change working call sites, and an alias would give new callers two entry points for the same operation.

Start with `App::new()` or `App::fallible()`, add ordinary functions with `.route(handler)`, then move state into `.with_state(state)`. A composition helper can take the builder and return it with new handlers. Use `AppHandlers` in a helper signature when that helper accepts or returns an open chain. `ReadyApp` owns the chain and state, and its `Emit` implementation lets it plug into a producer without exposing either. The handler trait, signature markers, and chain nodes are implementation details. [Handler signatures and delivery rules](application-events.md)

`Subscriptions::forward(app)` connects the app to a device. The application handler mutates its own state and can request an execution stop. A subscriber registered with `.on_with()` receives the board's IRQ loan. The endpoint can still acknowledge that IRQ after an earlier handler requests stop. Moving the endpoint into the PC moves the app with it and requires the model and subscribers to meet the attachment's ownership bounds.

## Extraction rules

Handlers taking `(&Event, &Extractor, &mut State)` use `FromEvent<Event>`. This is useful for the live COM1 example, where the extractor parses an event and the same handler updates its counter. The extractor receives a separate unit value. Its result can borrow the event while the handler holds exclusive application state. [Live COM1 handler](../../rusty_box/examples/typed_event_workflow.rs)

`(&Event, &Extractor)` can use `FromEvent<Event, State>` to lend a value from the state itself. Giving that handler another exclusive loan of the same state would permit aliases. Put the mutation in a later state handler, or expose a single borrowed view containing all the fields the handler needs. Two independent event-only extractions could use separate unit values. General state-backed extraction would require a way to prove that inputs are disjoint or use shared state. The observed use cases fit one extractor argument per handler.

Extraction begins only for a matching route that requests it. The cache lives in the event or application state and has an explicit delivery boundary. The first error ends delivery, even after an earlier stop request. Mutations and device effects already committed remain visible. The core tests and compile-fail examples exercise ordering, partial progress, escaped references, and invalid handler signatures.

## API readiness

| Area | Ready to embed | Remaining API work |
| --- | --- | --- |
| x86 CPU and memory | Use `CpuEmulator` for owned dense RAM. Use `MemorySession` for caller-owned flat RAM, sparse mappings, and independent direct or indirect backing. `Registers` and the checked memory view use short exclusive loans. | `CpuEmulator::run` reports a typed stop. `MemorySession::run_controlled` is its borrowed counterpart. The older count-returning session `run` remains easy to confuse with it. Only specific exits, including scalar I/O, have service and resume contracts. |
| Instrumentation | A typed tracer can count or observe instructions. Install hooks when needed and use app delivery at a meaningful source boundary, such as a device notification or sampled process. | Running every instruction through an app would add routing work and couple hooks to app ownership. Fast hooks and application events have separate entry points. |
| Hardware | A core-independent device model can use scoped DMA and IRQ capabilities and publish notifications. Board placement and the endpoint own their respective resources. | Inline attachment methods suit small observers. Reusing a stateful app takes an explicit endpoint and subscription chain. General restartable device transactions still need a defined effect and resume contract. |
| AArch64 | The shared event API works with either architecture. The AArch64 board and CPU have their own static bus, register, and translation contracts. | A shared instruction or memory interface would need demonstrated semantics on both architectures. Keep translation and cache bookkeeping private. |

[Memory and execution contracts](api-consolidation.md#api-readiness) · [CPU stops and restart](cpu-only-control.md) · [Device ownership](hardware-api-ergonomics.md)

## Performance checks

Run the allocation-free core benchmark with default features disabled. It times one million events per case, warms every case once, then rotates their order over nine samples. The event variants use the same report and parse workload. All app storage, input values, and caches fit on the stack. Printing and timing use the benchmark executable's standard library.

```sh
taskset -c "$CPU" cargo bench --locked -p rusty_box_core --no-default-features --bench app_delivery
```

The existing `cpu_perf` harness compares the same x86 instruction loop with its EXEC hook disabled and enabled. The x86 revision comparison uses release builds of `external_memory` and `perfbench`, pins a logical CPU, warms each revision, and rotates the sample order. Report dispatch counts for the CPU-side work and avoid inferring retirement rates for REP instructions. [CPU instrumentation harness](../../examples/execution_workflows/src/bin/cpu_perf.rs) · [Revision comparison commands and review policy](api-consolidation.md#performance-review-procedure)

### Measured delivery and hook costs

Rust 1.100.0 release builds ran on logical CPU 2 of a Linux Xeon Platinum 8573C host. Concurrent builds and guest tests from this run were paused during measurement.

| Case | Median ns/event | Range |
| --- | ---: | ---: |
| One route | 0.699 | 0.686–0.857 |
| One matching route plus three unrelated types | 0.701 | 0.668–0.788 |
| Same report, extraction unrequested | 1.597 | 1.400–2.015 |
| Report parsed by one extractor handler | 86.327 | 84.819–88.806 |
| Two handlers share the parsed report | 87.796 | 84.244–109.503 |

Disassembly of the typed tick fixture shows one add in the loop and no event type tests. That describes this optimized call site. The application implementation still supports open event types and traverses the registered chain. The report cases include constructing each input and its empty cache. Parsing six hexadecimal fields dominates the extraction cases. Both handlers use that same parsed value. The core benchmark needs no allocator for its handlers, reports, or caches. Host-side Linux observation separately allocates its task names and mapping vector. [Benchmark source](../../rusty_box_core/benches/app_delivery.rs)

| Existing `cpu_perf` workload | Median | Range |
| --- | ---: | ---: |
| ALU loop, hooks disabled | 7.00 ns/count | 6.79–7.49 |
| Same loop, counting EXEC hook | 7.38 ns/count | 7.27–8.12 |
| Warm instruction fetch | 68.23 ns/two-count batch | 66.77–70.49 |
| Checked patch and refetch | 244.62 ns/two-count batch | 229.61–249.52 |

The CPU harness used seven samples per case, 40 million counts per ALU sample, and 50,000 batches per fetch sample. These paths use the existing instrumentation mask and checked memory writes. They never enter application routing.

### x86 revision comparison

The baseline was built from `202ef7f` in a detached checkout. Both revisions used the same release toolchain and `--no-default-features --features std`. The existing review script alternated the builds on CPU 2 for seven samples of 80 million counts per workload.

| Workload | Base ns/count [range] | Current ns/count [range] | Median paired change |
| --- | ---: | ---: | ---: |
| Borrowed dense RAM | 10.268 [10.213–10.993] | 10.258 [10.114–11.261] | +0.44% |
| Sparse mapped RAM | 10.263 [10.093–11.177] | 10.223 [10.077–11.117] | −0.27% |
| Full PC, ALU | 6.36 [6.21–6.54] | 6.34 [6.24–6.98] | −0.47% |
| Full PC, mixed | 8.34 [8.09–9.71] | 8.28 [8.14–8.41] | −0.24% |
| Full PC, string | 11.63 [11.36–17.24] | 11.59 [11.34–12.12] | −0.77% |

All five paired medians stayed within 1% of the baseline. The full-system string baseline had two slow samples. The distributions support comparable execution times on these workloads. The change adds an application handler adapter and leaves the CPU, memory, and instrumentation implementation intact.
