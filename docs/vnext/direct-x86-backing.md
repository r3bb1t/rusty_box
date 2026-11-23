# Independently owned direct x86 memory

`DirectMemory` lets a CPU-only x86 `MemorySession` execute from several separately owned RAM and ROM slices. An outside provider chooses the physical addresses, initializes the bytes, and recovers them after the session. Paging, instruction fetch, and ordinary scalar data hits use the existing interpreter and caches. The provider lends slices and code stamps without copying its memory into an arena. The API works without `std` or an allocator. [Public API](../../rusty_box/src/memory/direct.rs) · [Downstream example](../../examples/execution_workflows/src/bin/external_ram.rs)

## Choosing the backing contract

| Design | Cost and safety |
| --- | --- |
| Encode a region index and local offset in the existing cache words | Keeps the TLB entry size and preserves base-plus-offset resolution for a single arena. A tagged hit selects its allocation by index. A new session invalidates those coordinates before using its own table. |
| Store a host pointer in each cache entry | Pointer reconstruction can be cheap inside a loan, but stored pointers must remain valid through memory and debugger reborrows, CPU/PC reattachment, and changes in allocation residency. Replacing the current PC offsets with pointers would change those safety proofs. Storing both pointer and source would grow the caches. |
| Parameterize the interpreter over a general bus | Allows static specialization by provider. A bus would still need a contract for direct pointers, cache identity, relocation, and write effects. Specializing the instruction engine for each memory type would increase the maintenance and compilation cost for independently owned slices. |

The implementation uses tagged offsets for direct CPU-only regions. On a single-arena hit the branch selects the existing address computation. A multi-region hit resolves directly against one borrowed slice with no search by guest address or dispatch into a user provider. The physical map lookup stays on fills and routed accesses. The table reborrows each descriptor's fields into fixed-size storage. This preserves the original descriptors' lifetimes and keeps the table out of the per-CPU TLB and out of the heap. The larger execution-loan value and the added tests on shared cache paths are potential costs. [Backing selection](../../rusty_box/src/cpu/exec_ctx.rs) · [TLB fill](../../rusty_box/src/cpu/paging.rs) · [Direct map and coordinates](../../rusty_box/src/memory/direct.rs)

## Ownership and coherence

- Supply 1–16 regions in increasing guest physical address order. Each region covers whole 4 KiB pages and carries at least one `u32` stamp per page. Slices themselves can be host-unaligned. The per-region cap is available as `DirectMemory::MAX_REGION_LEN` and is just under 128 MiB on a 32-bit host. Construction checks alignment, bounds, overlap, and stamp capacity before initializing stamps. Exclusive byte and stamp loans prevent safe callers from aliasing the same host bytes through the map.
- A cached region number identifies a slice only inside its session. Fetch is bounded by the current page and its allocation. An instruction split across two mappings uses a separate bounded read of each. Scalar DTLB hits require one fully backed writable page and the ordinary x86 permission checks. ROM is readable and fetchable and ignores guest writes. CPU-local MMIO still takes precedence.
- Each region indexes code stamps by its own local page. Guest stores and checked host writes publish invalidation at the guest physical addresses used by decoded traces. Different linear aliases of the same physical page therefore share its stamps. Host-side reads and writes use `session.memory()`; one checked transfer must fit within one backing. Guest page-walk accessed and dirty updates go through the existing physical write path.
- Change guest page tables through the checked host view and call `invalidate_translations()` before resuming if the guest has not already invalidated those mappings. To replace a slice or change the physical map, end the session and create a new loan. This clears the data and instruction TLBs, fetch window, and decoded code even when the new mapping uses the same physical or host addresses. [Tests](../../rusty_box/tests/external_memory.rs) · [Cache adoption](../../rusty_box/src/cpu/cpu.rs)

## Where this API applies

`MemoryLoan::with_indirect_ram` remains available on flat, mapped, and direct loans. It provides a disjoint, data-only range with its own storage. Exclusive execution itself does not require that provider to cross threads. Removing its attachment bounds from the current erased representation would weaken `MemoryLoan`'s existing `Send` and `Sync` auto traits, so the public bounds stay in place. The event, register, and run/stop APIs keep their existing roles. [Indirect RAM](../../rusty_box/src/memory/indirect.rs) · [Auto-trait test](../../rusty_box/tests/external_memory.rs)

Independent regions use the paged scalar cache path. Unpaged data operations, page walks, bulk strings, prepared read-modify-write operations, and shadow-stack atomic operations use the routed physical path where the existing fast path assumes a primary allocation. Direct SVM VMCB placement also requires the existing primary-allocation profile. Physical-map edits take effect at session boundaries. The full-system PC retains its device and residency policies, and AArch64 retains its separate physical bus contract. [API readiness](external-memory-providers.md#api-readiness)

## Reproduce

```sh
cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml --bin external_ram -- --direct
RB_MEMORY_COUNT=40000000 cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml --bin external_ram -- --bench
cargo test --locked --release -p rusty_box --no-default-features --features std --test external_memory
cargo test --locked --release -p rusty_box --no-default-features --features std --doc
cargo build --locked --release -p rusty_box_no_alloc_smoke --target x86_64-unknown-none
```

## Comparing backing types

The downstream benchmark rotates the same paged load-and-store loop through five backing types in one release binary. Seven samples per type each execute 40 million counts after warmup and check the resulting guest register value. The code and virtual addresses are identical. Indirect copy uses ordinary slice copies, and the byte-lane version adds its storage-layout cost. Values are median nanoseconds per count with observed ranges. [Benchmark source](../../examples/execution_workflows/src/bin/external_ram.rs)

| Backing | ns/count |
| --- | ---: |
| Dense single arena | 10.15 [9.33–11.65] |
| Mapped single arena | 9.90 [9.34–11.01] |
| Multi-backing direct | 11.35 [10.42–12.21] |
| Indirect copy | 25.78 [25.31–29.66] |
| Indirect byte lanes | 28.78 [27.11–30.91] |

Multi-backing direct took about 15% more time per count than mapped RAM and about 44% of the time used by indirect copy. A repeat during concurrent validation had wider ranges and a multi-backing median of 12.10 ns/count. These figures cover warm scalar transfers in this loop.

## Existing execution paths

For each workload, seven pairs of 80 million counts alternated between the Milestone 9 base (`e059ca0`) and this implementation. Both binaries used Rust 1.100.0 release builds and ran pinned to the same AMD EPYC 9V74 logical CPU. Values are medians in nanoseconds per engine count, with observed ranges. The ranges overlap in all three workloads. [Full-system benchmark](../../rusty_box/examples/perfbench/perfbench.rs)

| Workload | Milestone 9 | Multi-backing build |
| --- | ---: | ---: |
| Mixed | 7.69 [7.36–8.15] | 7.61 [7.30–8.53] |
| String | 10.75 [9.71–11.75] | 9.92 [9.24–10.61] |
| ALU | 6.27 [5.91–6.98] | 6.14 [5.95–7.36] |

The CPU-only comparison used the same protocol on the existing paged load-and-store loop. The mapped medians were almost identical. Dense RAM had a roughly 15% higher median, with overlap between the sample ranges. This remains a performance gap on an existing path. [CPU-only benchmark](../../rusty_box/tests/external_memory.rs)

| Existing CPU-only path | Milestone 9 | Multi-backing build |
| --- | ---: | ---: |
| Paged dense RAM | 9.510 [9.041–10.319] | 10.932 [9.864–13.685] |
| Paged mapped RAM | 10.731 [9.759–11.802] | 10.736 [9.822–14.389] |

Run the same full-system modes on each revision with `PERFBENCH_MODE=mixed`, `string`, or `alu` and `PERFBENCH_INSN=80000000`. For the CPU-only comparison, run `timed_borrowed_memory` with `--ignored --nocapture` in the `external_memory` suite. Set `BORROWED_BENCH_COUNT=80000000`, `BORROWED_BENCH_CONTROLLED=1`, and `BORROWED_BENCH_MAP=dense` or `mapped`. Pin both revisions to the same host CPU.

## Validation

The standalone workspace ran both the independent code/data/stack example and the existing indirect byte-lane example. The x86 suite passed 1,065 active library tests, 18 external-memory tests, and the selected CPU control, register, memory, and hardware integration tests. All 18 existing compiler fixtures and both new compile-fail doctests passed. The new tests exercise cross-region fetch, guest and host code writes, page-table accessed and dirty bits, deliberate translation invalidation, replacement backing, and rejection of invalid maps. [Integration tests](../../rusty_box/tests/external_memory.rs) · [Borrow checks](../../rusty_box/src/memory/direct.rs)

The host `no_std` checks passed with and without `alloc`, and the WebAssembly check passed with `alloc`. The `x86_64-unknown-none` smoke binary linked with both direct and indirect backing paths and contained no Rust allocator symbols. Standard release test executables from the unchanged AArch64 sources passed 1,179 tests without default features and 1,265 with `std`. Both configurations left eight optional tests ignored. [Freestanding smoke test](../../examples/no_alloc_smoke/src/main.rs)

Headless DLX reached its login prompt at the 383-million-count marker. Alpine 3.24.1 x86_64, using direct kernel boot from the distribution ISO, reached `localhost login:` by the 5.6-billion-count marker and completed a 7.2-billion-count run. The Alpine kernel was 6.18.35-0-virt. The guest fixtures validate both 32-bit and long-mode full-system execution through the software PC. [DLX harness](../../rusty_box/examples/dlxlinux/dlxlinux.rs) · [Alpine harness](../../rusty_box/examples/alpine/alpine_direct.rs)

The repository doctrine and formatting stages passed. The full gate has a pre-existing Rust 1.100.0 blocker. Its `rusty_box_symbols` Clippy command reproduced six `chunks_exact_to_as_chunks` errors in the unchanged symbol engine. The relevant memory, compiler, portability, and boot checks were run separately. [Milestone 9 gate result](user-mode-foundation.md#validation)
