# Typed hardware composition and subscriptions

This milestone starts from the public x86 API and register milestone on `experimental/6-x86-cpu-only-memory`. The applied base is `4f3bdb064505ff1c682c668a3a30f818153ca7cd` plus the previous stabilization and public API patches.

## Current hardware seams

The PC routes chipset MMIO through a physical map of window tokens and binds each token through `DeviceManager`. Its converted board devices receive clock, IRQ and timer loans in the existing `DeviceCtx`. That model works for the device set the PC owns. It assumes a PC-managed timer and ISA-style IRQ service, and it erases the capabilities even when a device needs only one.

The separately added `memory::mmio::MmioRegistry` already accepts independently implemented devices. It stores a fixed number of scalar callbacks or device loans and holds the legacy runtime routing contract. Those devices only see an offset, width and value. They cannot borrow RAM or the interrupt fabric. Attaching or detaching invalidates cached mappings. An occupied registry currently inhibits some direct data mappings across the whole CPU, including for legacy attachments.

`Emulator` lends processor, memory, bus, board devices, and scheduling fields separately through `PcIo` and `ExecCtx`. This is the existing safe point to project disjoint resources on the cold device path. The 8237’s channel callbacks and HRQ/HLDA protocol are specific to that controller. The board interrupt fabric already drives the PIC and I/O APIC together and publishes their state at the scheduler boundary. Host RAM methods can complete partial copies across pages, so the DMA capability needs a stronger one-fragment contract.

CPU instrumentation already uses typed callback data, a static tracer when supplied, and some explicit erased callbacks. Its stop path distinguishes an honored CPU stop from ordinary progress. `MemorySession` and `CpuEmulator` use the same interpreter with caller-supplied or owned RAM. Their restricted external scalar access path has a different restart contract from a PC MMIO operation.

## Two approaches

1. **Typed per-attachment handlers, delivered at dispatch.** The device library owns its event types and registration methods. The application configures the attachment before mounting it. At runtime the board routes the MMIO access, lends only the DMA and IRQ capabilities, and the adapter publishes the device’s typed result after the model method returns. Static handler and event composition work without a registry or heap. Runtime device placement still uses the existing kind of erased MMIO call. This is the implemented approach.

   ```rust
   let gpu = GpuAttachment::new()
       .on(|event| println!("{event:?}"))
       .on_with_irq(|_, irq| {
           irq.set_level(false);
           ExecutionRequest::Stop
       })
       .on_rejected(|event| eprintln!("{event:?}"))
       .at(page, irq_line);
   gpu.install(&mut machine)?;
   ```

2. **Runtime registration with queued delivery at a machine boundary.** A machine-level API could use `machine.on(gpu, callback)` and `machine.on_any::<TileWritten>(callback)` after mounting. Producer identity would need a lifetime or a generation that survives detach and prevents callbacks reaching a replacement. Heterogeneous events would need explicit erasure or separate typed queues, and queue capacity, errors and delivery scope would become board policy. The interpreter would need to yield at the relevant MMIO operation for timely DMA and IRQ visibility. Same-event subscriptions across unrelated mounted producers and post-mount registration remain a separate opt-in design. Existing typed observers can be registered on each selected attachment.

## Contract

The public application example registers a plain closure on a typed GPU attachment. Its other methods add a handler for rejection, an IRQ-aware handler, and an optional MMIO write observer. The GPU model defines both notification types using the architecture-independent device crate. Model authors can wire typed `Emit` sinks. Application code never constructs a route or publishes a device event.

`Subscriptions` is a local value that owns its handlers or captures their ordinary Rust borrows. It can be used by independent tools and CPU tracers as well as device adapters. A publisher lends an event, so event types need neither cloning nor a static lifetime. Handlers run in registration order. A stop is accumulated while remaining handlers run. The first error takes priority over a stop, returns unchanged to the publisher, and skips later handlers. Earlier effects stay committed. Nested publication is synchronous through a separate sink. The active route is exclusively borrowed, so safe code cannot enter it again or keep an IRQ loan from delivery. There is no queue capacity or overflow path. Panics follow normal Rust unwinding. Restart after a host panic is outside this contract.

The PC projects the selected endpoint, DMA destination and interrupt fabric as disjoint short borrows. DMA writes preflight one RAM page below 3 GiB, reject address aliasing and MMIO ownership, obtain complete backing, copy once and mark the existing code stamps. In the example, a successful DMA updates device status and asserts the IRQ before publishing completion. The model operation has released its borrows before subscribers run. A subscriber may drive the board-assigned IRQ, and it may request a stop. A failed DMA publishes a distinct rejection and leaves the destination bytes unchanged. The guest test verifies that a rejection subscriber can assert IRQ5 while the device leaves the line alone. Changing the line and acknowledging a pending PIC request are separate operations. The example reports subscriber errors, records a failure status and requests a host stop. The PC services DMA coherence and IRQ routing at its existing scheduler boundary. For the tested scalar store, a subscriber stop leaves execution after the completed store and before the next guest instruction. Compound x86 instructions retain their existing dispatch granularity.

On an empty event route, the sink is zero-sized and delivery returns immediately. On ordinary PC RAM access with no attachment, the existing routing gate is still the only gate. A legacy `MmioDevice` remains available for runtime device attachment. The composed route offers either caller-provided stable storage or optional owned allocation at mount. An IRQ is assigned at attachment, later lowered on detach, and must be clear of built-in device use. Duplicate composed claims and page overlaps return the endpoint along with the refusal.

## Scope

The integration covers one-page endpoints on the single-CPU software PC and checked writes within one conventional RAM page. Reads use the existing scalar MMIO model. General DMA spans, IRQ sharing, timers for external composed devices, externally owned snapshot state, and direct static dispatch across the runtime address registry remain open. A CPU borrowed out of a PC refuses CPU-only execution with a composed device installed. AArch64 internals use their existing execution and device paths.

[Application](../../rusty_box/examples/composed_hardware/main.rs) · [Independent device](../../rusty_box/examples/composed_hardware/gpu.rs) · [Current PC adapter](../../rusty_box/src/emulator/hardware/event_endpoint.rs) · [Typed subscription implementation](../../rusty_box_core/src/events.rs) · [Real guest tests](../../rusty_box/tests/composed_hardware.rs)

## Validation

Seven downstream hardware tests cover isolated DMA, IRQ and event sinks, two independent handlers, per-attachment identity with two GPUs, one-page refusal without partial writes, conflicting placements, guest-visible device and subscriber IRQs, resume after a host stop, and DMA invalidation of decoded code. Three compile-fail tests show that one event type cannot enter another type's route, a handler cannot retain the IRQ loan, and an active subscription cannot be borrowed again for delivery.

The application built and ran as a separate downstream crate. The GPU model compiled independently with the device library as its sole emulator dependency. The hash-checked DLX boot reached login at 383 million engine counts and opened a Linux 1.3.89 root shell. The bare-metal smoke executable retains the composed attachment and typed publication functions and links without Rust allocator symbols.

| Release suite | Passed | Failed | Ignored |
|---|---:|---:|---:|
| x86 `std`, including integration and doc tests | 1,280 | 0 | 8 |
| AArch64 without default features | 1,178 | 0 | 8 |
| AArch64 `std` | 1,284 | 0 | 8 |
| Core and device crates without default features | 51 | 0 | 0 |
| x86 allocation-free storage and loan tests | 5 | 0 | 0 |

The x86 run checked all 15 compile-fail fixtures. Host checks with neither feature and with `alloc` alone, the bare-metal no-default check, the WebAssembly `alloc` check, and formatting validation passed.

## Performance

On the AMD EPYC 9V74 VM, both release binaries use the lockfile, Rust 1.100.0, and x86 `std` with default features disabled. The baseline is the previous public API milestone. Workloads ran in alternating order on CPU 0 after validation jobs finished. Sample ranges and binary hashes are in the archive.

For an empty subscription, the compiler emitted the same function as a direct operation in the downstream code generation check. The regular PC loop, dispatcher, and selected dword and qword RAM helpers retain matching normalized instruction sequences and symbol sizes. Their placement changed.

The full PC ran with no external endpoint attached. Each row reports six 300 million-count samples per binary in wall nanoseconds per engine count.

| PC workload | Previous milestone, median [range] | This patch, median [range] | Median change |
|---|---:|---:|---:|
| Mixed ALU, load and store | 8.17 [7.62–9.29] | 7.85 [7.58–8.86] | −4.0% |
| REP MOVSB and STOSB | 9.96 [9.80–10.57] | 9.98 [9.77–10.46] | +0.2% |
| ALU | 6.45 [6.09–6.81] | 6.19 [6.09–6.68] | −4.0% |

All three pairs have overlapping sample ranges. The timings leave the contribution of placement and host variability unresolved.

The downstream CPU-only benchmark uses a warm load, increment and store loop, verifies the guest result, and excludes construction. The results are in wall nanoseconds per dispatch over six runs per profile and binary.

| CPU-only interface and budget | Previous milestone, median [range] | This patch, median [range] |
|---|---:|---:|
| Borrowed, 1 | 39.72 [37.81–48.46] | 41.89 [38.67–50.02] |
| Owned, 1 | 60.99 [55.88–67.69] | 65.58 [56.27–73.19] |
| Borrowed, 256 | 16.20 [15.13–17.98] | 15.64 [14.63–17.34] |
| Owned, 256 | 17.40 [15.30–19.13] | 15.24 [15.01–15.60] |

The single-dispatch medians increased by 2.16 ns for borrowed and 4.60 ns for owned execution. The new session-entry check for an attached PC device runs once per call. All measured ranges overlap, so these trials leave the contribution of that check unresolved.
