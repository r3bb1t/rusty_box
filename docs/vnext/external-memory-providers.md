# External x86 memory providers and API readiness

The framework's reusable parts are the execution engines and their public contracts. User mode, full-system machines, Linux observation and device models are consumers with different needs. This change extends the existing x86 CPU-only memory boundary with one independently implemented RAM range and preserves the user-mode work in its own commit. [Downstream provider](../../examples/execution_workflows/src/bin/external_ram.rs) · [User-mode integration](user-mode-foundation.md)

## Why the boundary matters

`ExecutionMemory::{Pc, Borrowed}` selects the chipset-backed PC or an exclusively borrowed CPU-only map. The x86 engine caches proven offsets into one stable allocation. It translates and checks access permissions before caching a RAM page. Instruction fetch remembers a bounded window and code stamps use arena page indexes while invalidation events name guest physical addresses. The PC can relocate resident memory under an epoch. A new external loan invalidates allocation-dependent caches, including when the arena address stays the same. Within the loan, checked physical writes and guest stores use the existing code-write tracking. Page-table edits by the host require the session's explicit translation invalidation. [Execution backing](../../rusty_box/src/memory/execution.rs) · [TLB fill](../../rusty_box/src/cpu/paging.rs) · [Backing adoption](../../rusty_box/src/cpu/exec_ctx.rs)

| Design | Benefit | Limit at the current boundary |
| --- | --- | --- |
| Validate one direct arena and caller-defined regions | Direct fetch and warm TLB hits use the existing offset fast paths. Mapping is chosen once with no per-access provider dispatch. | Independently held, encoded or interleaved storage cannot appear as one stable slice. |
| Parameterize the interpreter over a general physical bus | Allows fully static dispatch and could express a device-oriented or faulting implementation. | The bus would have to preserve direct memory and fetch proofs, bind cached offsets to backing identity, and define failure and restart for split operands, RMW, REP and implicit system accesses. Parameterizing the current execution context would require substantial engine work and evidence about code size and runtime cost. |
| Compose direct memory with a borrowed indirect RAM provider | External storage serves real data accesses and page-table walks. The existing direct cache proofs and full-system routes continue to apply. | Indirect RAM pays for byte transfers and one dynamic call per physical fragment. It offers an infallible RAM contract. |

The third design is the smallest additional contract that runs with independent storage. `MemoryLoan::with_indirect_ram` attaches one nonempty, page-aligned, disjoint range to a flat or mapped loan. The provider is a normal Rust type and supplies byte transfers through `IndirectRam`. It can manage its own allocations or use stack storage. Its exclusive borrow lasts for the whole session. `Send` and `Sync` on the accepted provider preserve the original loan's auto traits. The single private erasure point is documented in the safety doctrine. Only misses of direct backing can invoke the provider. [Public loan](../../rusty_box/src/memory/mapped.rs) · [Provider contract](../../rusty_box/src/memory/indirect.rs) · [Erasure policy](../safety-doctrine.md#r8--erasure-is-opt-in-and-named)

## Access and coherence

- The provider receives relative byte offsets bounded by the range. Ordinary RAM reads preserve logical contents and writes commit the supplied range. The emulator owns transfer widths and page splits. Checked host reads and writes must fit a single mapping. CPU-local routes retain their original precedence. Outside mapped backing, CPU-only data accesses retain open-bus behavior. [Physical routing](../../rusty_box/src/memory/execution.rs)
- Indirect pages yield no direct address for cached loads, stores, or instruction fetch. Physical overlap with direct RAM and ROM is forbidden, so provider writes cannot modify a stamped code page through a second address in the map. A provider may back guest page tables. Guest-side accessed and dirty updates reach the provider, while host page-table edits use `invalidate_translations()` before resuming. [Page-table tests](../../rusty_box/tests/external_memory.rs)
- The existing code cache is still active for executable direct memory. The downstream example warms a direct translation, releases the session, and binds the same arena with the data GPA supplied by the byte-lane provider. It verifies the new data through guest execution. A tracked write to already decoded code changes the next run's result. The example also exercises a cross-page word and a fetch refusal inside indirect RAM. [Independent storage](../../examples/execution_workflows/src/bin/external_ram.rs)
- Side-effecting devices retain separate semantics. The full-system [`MmioWindow`](hardware-api-ergonomics.md) path carries typed events and narrow hardware capabilities. The x86 [`step_with_external_page`](recoverable-physical-access.md) path is a preflighted, replayable scalar MOV, with explicit pending and refusal outcomes. An attached RAM range excludes that single-page service. A fallible or asynchronous general bus needs a contract for effects already committed by split and implicit accesses.

## API readiness

| Area | Ready for downstream use | Remaining gap |
| --- | --- | --- |
| x86 CPU-only | `CpuEmulator` for owned dense RAM, `MemorySession` for external loans, typed register access and static instrumentation. | The owned convenience facade carries dense RAM. Independent providers attach to a borrowed session. Multiple independently lent executable allocations need direct-source identity and lifetime proofs in the caches. |
| Run and resume | `SessionRun` and typed stops distinguish a budget, breakpoint, observer request and engine exit. Scalar port completion has an explicit pre-retirement continuation. | Engine counts include REP and fault restarts. The raw borrowed `run` and `run_controlled` serve different callers. A shared outcome must preserve architecture-specific side effects and retry rules. |
| Typed events and state | `App::on()` and `with_state()` compose typed handlers and exclusive application state. Low-level `Subscriptions` remain usable without allocation. | Device-local and application-level delivery carry different capabilities. Cross-device transaction and ordering rules belong to their owner. |
| External hardware | The software PC offers checked placement, `MmioWindow`, and narrow DMA and IRQ capabilities. | Board-specific topology and live map changes require explicit ownership and coherence rules. Raw indirect RAM carries no device effects. |
| AArch64 reuse | The board and CPU use their own `PhysBus`, with typed physical addresses, fetch-source and descriptor-swap semantics. Both architectures share useful host-level budgets, progress, and typed event concepts. | A shared physical bus would need real callers that can agree on fetch, atomic descriptor updates, faults and timing. The x86 allocation-offset proof and the AArch64 bus are different contracts. |

The central gaps are direct backing identity across independent allocations, runtime remap ownership, and restart semantics for general fallible or side-effecting memory. These determine how far a backend can participate in execution. Another guest service or a common trait name would leave those questions open. [Prior public API review](architecture-stabilization.md) · [AArch64 physical bus](../../rusty_box_aarch64/src/bus.rs)

## Validation

The downstream example and its scoped Clippy check passed. The x86 release library suite passed 1,065 tests, and the CPU-only control, external and flat memory, hardware topology, composed hardware, and 18 existing compiler fixtures passed. The new compile-fail doctest rejects simultaneous use of the provider and its loan. The page-table test observes old data while a translation is warm, updates the provider's table through the checked host view, then observes new data after invalidating translations. It also checks accessed and dirty bits, bound failures, and exclusion from the guarded scalar service.

The software PC reached the DLX login marker around 383 million counts. The preserved Linux/i386 user-mode fixtures and typed-event test passed on the updated x86 engine. Rerunning the standard AArch64 test executables from the unchanged AArch64 sources produced 1,179 passes without default features and 1,265 with `std`, with the optional specification tests still ignored. The interpreter checked under `no_std` on the host with and without `alloc`, on freestanding x86 without allocation, and on WebAssembly with `alloc`. The freestanding smoke executable includes a stack-backed indirect provider and linked without Rust allocator symbols.

The follow-up repeated the repository doctrine and formatting stages. The previous commit's broader gate reached six existing Rust 1.100.0 `rusty_box_symbols` Clippy errors. The symbol-engine sources are identical for this change. [Full gate result and preserved milestone](user-mode-foundation.md#validation)

## Performance

Measurements used Rust 1.100.0 on an AMD EPYC 9V74 Linux worker. Binaries ran with affinity to the same host CPU. Each before-and-after row alternated the base `4e4f835` and the provider patch over seven pairs of 80 million engine counts. The longer ALU check used 240 million counts per sample. Values are median ns/count with the sample range in brackets.

| Existing path | Base | Provider patch |
| --- | ---: | ---: |
| Full-system mixed | 7.69 [7.28–8.06] | 7.82 [7.62–8.77] |
| Full-system string | 9.62 [9.47–10.22] | 9.80 [9.27–10.37] |
| Full-system ALU | 6.09 [5.98–6.83] | 6.52 [6.06–6.92] |
| Full-system ALU, longer run | 6.35 [5.99–6.94] | 6.29 [5.90–6.55] |
| CPU-only paged dense | 9.739 [9.149–10.218] | 10.027 [9.087–10.447] |
| CPU-only paged mapped | 9.429 [9.274–10.258] | 9.542 [9.160–10.464] |

All before-and-after ranges overlap. The shorter ALU run had the largest median gap. The longer run measured close medians in the opposite order. The full-system CPU loop symbols retained their sizes. The timing evidence covers these workloads and leaves smaller effects and other access patterns open. [Full-system workload](../../rusty_box/examples/perfbench/perfbench.rs) · [CPU-only workload](../../rusty_box/tests/external_memory.rs)

The downstream benchmark rotated four paths through seven samples of 12 million counts per path in one binary. The code and virtual addresses are identical, and the test checks the same load result and count. Data accesses are direct for dense and mapped RAM, while both providers use the slow physical path. The copy provider charges the slow path and dispatch with ordinary slice copies. The byte-lane provider also charges its storage layout.

| Same paged downstream loop | ns/count |
| --- | ---: |
| Dense | 9.24 [9.02–9.68] |
| Mapped | 9.41 [9.00–11.12] |
| Indirect copy | 25.42 [24.21–28.18] |
| Indirect byte lanes | 28.07 [25.83–29.14] |

Indirect data costs about 2.7–3.0 times the direct path in this load-and-store loop. That cost applies to memory that opts into the provider. Hot executable and ordinary RAM windows can use direct backing in the same session. [Reproduce the downstream timings](../../examples/execution_workflows/README.md#lend-custom-ram-to-the-x86-interpreter)
