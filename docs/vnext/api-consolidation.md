# API consolidation and performance review

Continue the Rust emulator API work on `refactor/redesign-apis-v2`. The consolidation point is `546810b`, the direct-memory implementation. Its parent `e059ca0` is the exact pre-direct-memory comparison revision.

## Branch ancestry

The following published tips were checked with `git merge-base --is-ancestor` against `546810b`. All incorporated tips are retained in the consolidation branch's ancestry. Their old experimental refs are candidates for deletion from the perspective of commit reachability.

| Experimental branch | Tip at consolidation | Relationship |
| --- | --- | --- |
| `1-arm-support` | `c4adb38` | Incorporated |
| `2-multiarch-gui` | `a583971` | Incorporated |
| `3-aarch64-speed` | `5ea7a07` | Incorporated |
| `4-aarch64-el2` | `d1c37cd` | Incorporated |
| `5-lean-aarch64` | `12145ce` | Separate history branching from `d1c37cd` |
| `6-x86-cpu-only-memory` | `e4c4761` | Incorporated |
| `7-linux-semantic-observation` | `e48b73c` | Incorporated |
| `8-host-side-linux-observation` | `e0638c4` | Incorporated |
| `9-user-mode-foundation` | `e059ca0` | Incorporated, including the earlier user-mode implementation `4e4f835` |
| `10-direct-memory-backing` | `546810b` | Consolidation point |

The Lean branch has 37 commits outside this ancestry, all under `lean_aarch64/`. It is a separate research tree with its own toolchain, interpreter, proofs, and differential fixtures. Branch deletion remains an owner decision.

## Memory profiles

| Profile | Who supplies memory and when to choose it |
| --- | --- |
| `CpuEmulator` | The emulator owns contiguous RAM and stamp storage. Choose this for ordinary firmware-free x86 code, short host calls and warm caches between runs. Construction uses `alloc`. |
| `FlatMemory` | The caller owns one dense slice and stamps. Choose this to control storage ownership with the same dense physical placement. |
| `MappedMemory` | The caller supplies one arena and sorted sparse guest windows into it. Choose this for sparse placement when the backing already fits a single arena. |
| `DirectMemory` | The caller lends up to 16 independent page-aligned guest regions, each with its own bytes and stamps. Choose this when storage already lives in separate allocations. Host alignment is unrestricted. |
| `IndirectRam` | The caller attaches one disjoint infallible data aperture to a borrowed loan. Choose this for representations that cannot lend direct bytes. Page walks may use it. Instruction fetch requires direct backing. |
| Software PC memory | The board owns firmware placement, chipset holes, device routing, A20 and residency. Choose this when the guest needs a PC and its services. |

`MemorySession` joins an initialized CPU and an exclusive `MemoryLoan` without allocating. Prefer `memory().read_phys()` and `memory().write_phys()` for checked host access, with the existing physical-read and RAM-write conveniences available. Host transfers must fit one backing and can report an unavailable window. The guest path independently applies paging and device policy. In CPU-only memory a physical data hole reads as open bus, while refused instruction fetch returns a typed host exit. Architectural faults such as a guest page fault use normal guest delivery. The preflighted scalar I/O and external-page steps have their own narrow completion rules. [CPU-only control](cpu-only-control.md) · [External data completion](recoverable-physical-access.md)

Cache hits carry coordinates into the live loan. Arena offsets use the primary base, and tagged coordinates select the original independent slice. Whole-page checks precede scalar DTLB fills. Writes dirty per-backing stamps and publish coherence records in guest-physical coordinates. Host page-table writes still require `invalidate_translations()` before execution uses the new translations. End the session before replacing a map or any backing. A new session drops inherited mappings, fetch windows, and decoded code. The PC additionally handles residency changes. [Direct backing contract](direct-x86-backing.md)

The current concrete memory profiles preserve distinct invariants. A trait returning copy-based reads and writes would still need a direct-fetch and cached-pointer contract, stamp identity, permission checks, and explicit split or partial effects. Monomorphizing an engine over each profile could remove profile branches at the cost of larger interpreter builds and a wider implementation boundary. The small layout changes in this branch keep one interpreter, one-word cache entries, and static direct access. The indirect provider call stays in the opted-in physical data path. [Backing dispatch](../../rusty_box/src/memory/execution.rs)

An arena can end at a partial page. Independent direct regions cover whole pages and have per-region stamp slices. The physical routing arms spell out these different bounds and share the indirect loan and code-write bookkeeping. `with_indirect_ram` can attach to either backing shape. Its `Send + Sync` bounds preserve the existing `MemoryLoan` auto traits. Checked reads and writes are the place to consolidate terminology first, using the same `PhysicalMemory` view across the CPU-only ownership models.

## API readiness

| Area | Ready to build against | Friction and boundary |
| --- | --- | --- |
| CPU construction and registers | `CpuEmulator::builder()` with owned RAM, or `BxCpuBuilder` and `MemorySession` with borrowed RAM. Typed scalar and SIMD register views apply x86 width rules. | Convenience entry covers real mode and flat protected mode. Long mode and custom device-free state require architectural setup. A checked mode/bootstrap operation needs an actual downstream use case. |
| Execution and debug | `run_controlled` and `step` return `SessionRun` with dispatch counts and a typed stop. Breakpoints stop before execution and permit one resume through a hit. Static instrumentation works on both CPU-only and PC profiles. | Prefer the controlled interface for new callers. It reports engine exits first, then breakpoints, observer stops, inactivity, and budget or yield. `run` returns the accounting delta alone on success. REP and guest exceptions complicate retirement. Only specific typed exits have a replay or completion contract. |
| Application composition | `App::on()` composes open event types. `with_state()` moves state into `ReadyApp`. Delivery is synchronous, ordered, and reborrows state exclusively for each handler. | Facade event types satisfy `Any`. Use `Subscriptions` for scoped borrowed event types and temporary capabilities. A stop is accumulated, and the first error wins and stops later delivery. These two layers have different consumers and share delivery machinery. |
| Device composition | An external device can use a typed `MmioWindow`, report events, and receive the board's scoped DMA and interrupt capabilities. The topology owns placement and lends or owns the endpoint. | PC placement and IRQ routing are board-specific. Composed MMIO and guarded external-page operations have distinct lifetimes and effect contracts. General resumable or multi-fragment external transactions need explicit commit semantics. |
| AArch64 | Its CPU and board use their own static `PhysBus`, with clocked accesses, fetch-source policy, and translation descriptor swaps. Both architectures can reuse typed events and host budget/progress concepts. | x86 byte streams, permission checks, and trace coherence differ from AArch64 fetch and descriptor semantics. Their register, translation, and effect APIs should remain architecture-specific. |

Keep `ExecCtx`, `ExecutionMemory`, TLB and trace entries, raw host coordinates, and scheduler ownership private. The public contracts should describe what the caller lends, which access is permitted, and how a run can resume. They can evolve without lending cache or device internals. [Typed application events](application-events.md) · [Hardware composition](hardware-api-ergonomics.md) · [AArch64 bus](../../rusty_box_aarch64/src/bus.rs)

## Remaining work in priority order

1. Make execution stop and completion rules easier to find. Keep `run_controlled` the common teaching path and distinguish dispatch budgets, pre-execution exits, and effectful completions in each architecture.
2. Keep the fast-path comparison current as storage and tracing evolve. The independent-backing proof keeps cached coordinates within one loan. Live remaps would also need a specific invalidation and ownership contract.
3. Add checked architecture-specific boot or mode transitions when callers need them. Reuse the current register types and preserve translation and breakpoint invariants.
4. Extend external device completion only with a concrete consumer that can define the effect and failure behavior for split operands, RMW, REP, or implicit accesses. Keep device placement and DMA policy with the board.

## x86 hot-path investigation

In the dense load/store workload, the published direct-memory build moved the CPU pointer from displacement `0x50` to `0x380` in the compiled execution context. The dword store also decoded a niche-packed backing enum with a byte load, subtraction, zero-extension, and conditional move before branching on the resulting tag. The same enum participates in the trace cache's sequence and residency check. The tagged fetch coordinate is resolved at fetch-window materialization. A warm decoded trace chiefly sees the sequence and epoch check.

The fix lays out the private CPU pointer and primary bases first and gives the backing enum an explicit byte tag. It also uses the width-aware DTLB hit as the proof that a scalar store fits its stamped page. The slot for the access end cannot match the first-page tag on a split access. Routed writes still check both fragments. The split-instruction test warms both writable translations, patches the immediate with a guest cross-page store, and verifies the next fetch in mapped and independent RAM. The remaining direct-data selection is one primary-base check per warm scalar transfer. [Cached store and backing dispatch](../../rusty_box/src/memory/execution.rs) · [Split-store test](../../rusty_box/tests/external_memory.rs)

Valgrind 3.24.0 Callgrind counted host instructions and simulated conditional branches in the dense workload. The values below subtract the one-million-count sample from the five-million-count sample and divide by four million engine counts. The subtraction excludes nearly all process startup and warmup. Run with `valgrind --tool=callgrind --branch-sim=yes` and use the `Ir` and `Bc` fields of the output summaries. Disassembly is from the same release configuration, using `nm -nS -C` and `objdump -d` on the artifact.

| Build | Host instructions/count | Conditional branches/count |
| --- | ---: | ---: |
| Pre-direct-memory `e059ca0` | 161.50 | 17.75 |
| Direct-memory `546810b` | 165.00 | 18.50 |
| Explicit tag and hot fields | 163.50 | 18.50 |
| Refactor with cached stamp proof | 162.75 | 18.25 |

The structural changes reclaim most of the added instructions without growing the TLB entries. Keeping a tag and selecting the original allocation still avoids storing address pointers across reborrows. The host instruction counts explain part of the steady-state overhead. Wall time also depends on instruction placement, data layout, and host scheduling.

### Borrowed-memory wall time

All three revisions were rebuilt with Rust 1.100.0, the same locked release profiles and `std` with default features disabled. Seven 100-million-count samples per revision rotated on logical CPU 2 of an AMD EPYC 9V74 worker. Values are median ns/count with min–max ranges. Other build work was moved to CPU 0. The shared worker still recorded some cgroup throttling.

| Workload | Pre-direct `e059ca0` | Direct `546810b` | Refactor |
| --- | ---: | ---: | ---: |
| Paged dense | 10.212 [9.601–10.762] | 10.868 [10.432–12.624] | 10.025 [8.876–12.371] |
| Paged mapped | 11.212 [9.331–12.905] | 10.530 [9.405–11.409] | 11.111 [9.334–12.389] |

The published direct build was slower than the pre-direct build in all seven matched dense rounds, with a 6.4% median increase. The refactor was faster than the published direct build in six of seven rounds. An earlier interleaved run put the published dense increase near 0.8%, so the historical 15% estimate varies across runs. Mapped timing varied in both directions across runs. The smaller host instruction count is stronger attribution for the layout and stamp changes than any one timing run.

### Full-system wall time

The full-system comparison used the same rotation, CPU and 100-million-count budget. All three builds used the single-vCPU long-mode software PC benchmark. The mixed case includes ALU instructions, reads, and stores. The string case exercises REP MOVSB and STOSB.

| Workload | Pre-direct `e059ca0` | Direct `546810b` | Refactor |
| --- | ---: | ---: | ---: |
| ALU | 6.540 [6.260–7.720] | 6.480 [5.950–7.750] | 6.250 [5.660–6.940] |
| Mixed | 8.030 [7.490–9.300] | 7.520 [7.370–8.030] | 7.970 [7.190–8.420] |
| REP strings | 10.790 [10.130–11.320] | 10.080 [9.700–11.090] | 9.720 [9.010–10.350] |

### Longer timing check

The first PC mixed comparison put the refactor about 6% above the published direct build. A second rotation used seven samples of 160 million counts per revision with the build and test jobs finished. The refactor's median was lower than both earlier builds in all three workloads. The published dense median changed direction relative to the pre-direct build, weakening the case for a persistent 15% increase. The host instruction comparison above was consistent across the layout and journal builds.

| Workload | Pre-direct `e059ca0` | Direct `546810b` | Refactor |
| --- | ---: | ---: | ---: |
| Paged dense | 9.796 [9.035–9.995] | 9.539 [9.161–10.564] | 9.218 [8.993–10.035] |
| Paged mapped | 9.969 [9.270–10.501] | 9.974 [9.262–10.391] | 9.571 [9.098–10.350] |
| Full-system mixed | 7.980 [7.770–8.480] | 8.050 [7.750–8.740] | 7.910 [7.040–8.410] |

### Code-write journal rollover

The checked-write/refetch workload found a separate cliff present in the pre-direct revision. The 64-entry journal kept its oldest records and discarded all later writes until a scheduler drain. The persistent CPU-only session therefore did a full trace-cache flush on every later code patch. The journal now retains a contiguous new batch and records the sequence of the discarded prefix. A reader at that watermark can apply individual new writes. A reader that missed the prefix still performs a full flush, including a lagging PC processor. The full-journal path stays on the cold code-write path with the same fixed storage. [Journal and watermark test](../../rusty_box/src/memory/code_writes.rs)

At 50,000 two-dispatch batches per sample, checked patch and refetch fell from **23.14 µs** [21.02–28.79] to **0.226 µs** [0.204–0.330] per batch. The only engine change between these two executables was the journal rollover. The old binary measured about 0.235 µs per batch when stopped at 64 patches, which located the cliff at the journal boundary. The benchmark checks the immediate read back into EAX after each batch. [CPU-only benchmark](../../examples/execution_workflows/src/bin/cpu_perf.rs)

### Additional baselines

The standalone runs used seven rotated samples per mode on the same CPU. The first five rows use one paged load/store workload in `external_ram --bench`. The ALU and fetch rows use `cpu_perf`. Long runs use 40 million counts. Short runs use 50,000 batches.

| Workload | Median [min–max] | Unit |
| --- | ---: | --- |
| Paged dense | 9.40 [9.02–10.02] | ns/count |
| Paged mapped arena | 10.21 [9.02–14.18] | ns/count |
| Independent direct regions | 11.44 [10.29–11.65] | ns/count |
| Indirect copy | 28.58 [25.93–31.79] | ns/count |
| Indirect byte lanes | 28.56 [27.48–31.14] | ns/count |
| CPU-only ALU with execution hooks disabled | 6.56 [5.79–7.12] | ns/count |
| CPU-only ALU with a counting hook | 6.97 [5.98–8.40] | ns/count |
| Warm fetch | 71.46 [66.94–75.37] | ns/two-count batch |
| Checked patch and refetch | 225.63 [203.57–330.15] | ns/two-count batch |

## Performance review procedure

Use the same Rust release and flags, match the binaries' feature sets, and put each revision in a detached worktree with its own `CARGO_TARGET_DIR`. The latter keeps Cargo's output names from overwriting another revision's executables. From each worktree, build the borrowed workload and the full-system executable.

```sh
cargo test --locked --release -p rusty_box --no-default-features --features std \
  --test external_memory --no-run --message-format=json
cargo build --locked --release -p rusty_box --no-default-features --features std \
  --example perfbench
```

Use the `external_memory` artifact's `executable` field and the `release/examples/perfbench` executable from each target directory. The review script runs an unrecorded process for each revision and workload, rotates measurement order, pins the benchmark process to one allowed logical CPU, and verifies each budget. Use `--json` to retain the raw samples. The borrowed-memory workload additionally warms the guest loop inside each sample. The full-system loop spends essentially the whole large sample running the same guest loop after its initial fetch.

```sh
python3 scripts/x86_perf_review.py --cpu 2 --rounds 7 --counts 100000000 \
  --revision before "$BASE_MEMORY_TEST" "$BASE_PERFBENCH" \
  --revision after "$NEW_MEMORY_TEST" "$NEW_PERFBENCH"
```

Run the other two workloads from the standalone workspace. `external_ram --bench` compares dense, sparse arena, multiple direct regions, indirect copies, and indirect byte lanes with identical code. `cpu_perf` runs an ordinary ALU loop with the same tracer type and with its execution mask disabled or enabled. It also measures warm fetch and a checked write followed by refetch, with the same two-dispatch batches and a verified result after each batch. The latter includes host view, run, and register-access costs.

```sh
cargo build --locked --release --manifest-path examples/execution_workflows/Cargo.toml \
  --target-dir "$EXAMPLE_TARGET" --bin external_ram --bin cpu_perf
RB_MEMORY_COUNT=40000000 taskset -c 2 "$EXAMPLE_TARGET/release/external_ram" --bench
taskset -c 2 "$EXAMPLE_TARGET/release/cpu_perf"
```

Report medians, ranges, build and host information, and paired values when a result is disputed. Use a quiet host and repeat comparisons with a longer budget if I/O, scheduling or clock variation dominates. A repeatable change around 5% is a review trigger. Check attribution with host instruction counts and disassembly before adjusting code or promising a regression. Wall-time ranges can overlap even when a paired effect is consistent. Keep this a local review procedure without a timing pass/fail threshold on shared CI. Counts are engine dispatch-accounting units. In particular, the REP workload should not be read as a precise architectural retirement count.

## Validation

The x86 release library suite passed 1,066 active tests, including the two journal watermark cases. The external-memory suite passed 19 tests, including the new split-store case. The selected CPU control, borrowed and flat memory, typed register, and composed hardware suites passed. All 18 compiler fixtures and 28 active x86 doctests passed. The standalone direct, indirect, and typed-application examples ran successfully.

Host `no_std` checks passed with and without `alloc`, WebAssembly passed with `alloc`, and the `x86_64-unknown-none` smoke executable linked without Rust allocator symbols. The standard AArch64 suites passed 1,180 tests without default features and 1,286 with `std`. Each configuration left eight optional tests ignored.

DLX reached the login prompt at the 383-million-count marker on the final build. Alpine 3.24.1 x86_64 reached its login prompt by the 5.6-billion-count marker on the layout patch before journal rollover. The full-system timing cases and the journal unit test exercise the finalized code.

The repository gate with `--skip-aarchmrs` passed its doctrine, formatting, core, device, AArch64, WHP, decoder, and symbol test stages. It then stopped at six `chunks_exact_to_as_chunks` Clippy errors in the unchanged `rusty_box_symbols` sources. The separate x86, portability, and boot checks cover the paths described above.
