# Extensible x86 physical memory

The starting revision is `2fe7ef81` on `codex/vnext-x86-cpu-only`. The original performance comparison uses `d0fb5122`.

Before this change, `ExecCtx` could borrow PC memory or a dense flat slice. An external crate could create `FlatMemory`, but could neither run the processor directly nor supply a different physical layout. The PC path supplied A20, chipset MMIO, ROM and block residency. The engine itself needs guest translation and permissions, a physical response for each access, and a proof that cached offsets and decoded bytes still belong to the current backing.

| Boundary | Decision |
|---|---|
| Borrow a validated physical map and its storage at session entry | Selected. A caller assembles regions in ordinary Rust, and the engine resolves them to its existing typed offsets. The map is read-only during execution. A downstream crate can already implement sparse placement, ROM, and replacement. |
| Parameterize instruction execution with a physical bus | Deferred. The current instruction, page-table, system and RMW helpers mix fallible and infallible reads and writes. A useful arbitrary bus would require deciding how failures propagate through each, which effects may already have happened, and which register state is restartable. Monomorphizing the execution context also needs independent code-size and timing evidence. |

`MemorySession::new(&mut cpu, memory.loan())` is the CPU-only entry point. Both `FlatMemory` and `MappedMemory` use it. The downstream integration crate owns the packed arena and stamps, chooses the placement of each physical window, and creates the loan. Its high reset ROM and remapped low pages occupy a handful of host pages. There is no public trait or per-access vtable. The remaining internal PC/borrowed distinction supplies the old PC services and its residency protocol. Adding another provider of compatible maps requires no engine edits.

| Access or lifetime | Contract |
|---|---|
| CPU linear access | Existing x86 MMU walks guest tables and checks guest permissions. CR2, exception frames, and IRET are owned by the processor. |
| Physical RAM, ROM, holes | RAM reads and writes use arena offsets. ROM reads and fetches use its bytes, and guest writes are discarded. Unmapped data reads yield `0xff` and writes are discarded. An unavailable instruction fetch returns a host exit with a physical address. |
| Physical overlap and partial effects | Guest and backing ranges must both be disjoint and page aligned. Scalar split operands are translated and checked before the bytes are accessed. A physical split with a ROM or hole can complete with writes to its RAM portion. REP preserves completed elements across a guest fault. |
| Fast access and invalidation | RAM has direct data mappings, while ROM and holes cannot enter the writable DTLB path. Fetch caches complete RAM and ROM pages. Stamps index arena pages and invalidation events retain guest physical tags. |
| External writes | `MemorySession::write_ram` modifies one checked RAM window between runs and publishes SMC invalidations. Guest table changes still require guest TLB invalidation. The exclusive borrow prevents a provider from changing backing or region descriptors during the session. |
| Replacement | Every new borrowed session retires prior translations, stack/fetch windows, decoded traces and VMCB host offsets, including when both maps reuse one arena at the same address. Keep a session across runs when its map is stable. |

The provider's data operations are total by construction. An absent page in this physical map has an explicit bus policy and cannot produce a provider error for the scalar helpers to drop. Those legacy helpers still have genuine error-loss cases for PC routing and residency failures. For example, byte reads can fall back to zero, wider accesses can retry at narrower widths, and some page-table and virtualization helpers translate errors to sentinel values. The new contract does not offer fallible callbacks through those paths. Extending it to those devices needs a typed physical result and a restart design for scalar, page-table, exception-frame, RMW and REP accesses before exposing the callbacks.

The integration tests cover sparse RAM and ROM across page boundaries, rejected overlapping maps, same-address remaps, guest SMC and tracked host writes. Real guest firmware installs paging and an IDT, then takes #PF during a split scalar store and during REP MOVSB. A fetch straddling a physical hole reports the missing page and restores the start of the instruction so the host can complete the map and retry. The compile-fail fixture pins map immutability during execution.

This contract covers page-granular, non-aliased physical windows over one arena. Map construction uses pairwise checks for backing overlap and lookup uses binary search by guest address. Independent allocations, MMIO devices supplied by the provider, fallible buses, shared DMA and mapping changes during a live execution borrow are separate design work. The next useful experiment is a recoverable physical exit with explicit preflight and commit semantics, exercised by scalar split writes and REP before generalizing the bus. The AArch64 engine retains its existing bus and execution model.

## Validation

Host was Linux x86-64 on an AMD EPYC 9V74 VM, with Rust 1.100.0
(`0ef1bc386`) and nightly rustfmt (`65ce8611f`). The checkout's upstream
`HEAD` was `d0fb5122`. The checked-out memory milestone was recovered from
its verified Git bundle before making changes.

The headless DLX gate ran on this patch and the immediately preceding
revision. Both reached the Linux 1.3.89 login prompt at the 383M count
checkpoint and then the root shell. Each run started with a fresh copy of
the disk image and used `RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=450000000`.
The ROMs came from [Bochs 3.1](https://github.com/bochs-emu/Bochs/tree/30cfd566ecc8deb33e56e801e32b444a7041b6d2/bochs/bios)
and the [official DLX download](https://bochs.sourceforge.io/diskimages.html)
supplied the image. The evidence includes input hashes and both guest logs.

These build and portability checks passed:

```sh
cargo check --release -p rusty_box --no-default-features
cargo check --release -p rusty_box --no-default-features --features alloc
cargo check --release -p rusty_box --no-default-features --target x86_64-unknown-none
cargo build --release -p rusty_box_no_alloc_smoke --target x86_64-unknown-none
cargo +nightly fmt --package rusty_box --package rusty_box_no_alloc_smoke -- --check
```

The smoke example type-checks the public execution call with a borrowed arena
and no allocator. The execution/exception evidence comes from the host-run
integration tests and the full-system DLX run.

| Release suite | Passed | Ignored |
|---|---:|---:|
| x86 with `std`, including integration and doc tests | 1,250 | 8 |
| AArch64 with no default features | 1,178 | 8 |
| AArch64 with `std` | 1,284 | 8 |

All had zero failures. The x86 run includes 1,060 unit tests and six downstream
memory tests. The AArch64 ignored tests need the external AARCHMRS catalog.
Commands used `cargo test --release -p rusty_box --no-default-features
--features std --no-fail-fast` and `cargo test --release -p rusty_box_aarch64
--no-default-features` with and without `--features std`. The x86 fixture was
generated with `TRYBUILD=overwrite` and reviewed before the targeted rerun.
The targeted rerun passed the six downstream tests and all seven
compile-fail fixtures with the checked-in diagnostics. These results cover
the requested regression builds and the DLX gate.

## Performance

Each revision was built separately with the same compiler, lockfile,
`--release --no-default-features --features std`, and the existing
`perfbench` example. The harness pinned every process to the same host CPU
and rotated the order. Each row uses five rounds of 200 million
emulator-counted instructions. Values are median ns/count with min–max in
brackets. Lower is faster.

| Full PC workload | Original `d0fb5122` | Previous `2fe7ef81` | This patch | vs previous |
|---|---:|---:|---:|---:|
| Mixed ALU/load/store | 8.12 [7.59–8.97] | 7.59 [7.22–8.44] | 7.82 [7.70–8.59] | +3.0% |
| ALU | 6.56 [6.43–6.88] | 6.31 [5.91–7.16] | 6.45 [6.23–6.59] | +2.2% |
| REP MOVSB/STOSB | 9.98 [9.62–11.23] | 10.00 [9.91–10.35] | 10.17 [9.86–10.87] | +1.7% |

The PC sample ranges overlap. These measurements are from an AMD VM. The
previous milestone's published Intel-host run has a different timing baseline.
The longer mixed confirmation used five rounds of 700 million counts.
It measured **7.69** [7.40–8.03] on the original revision, **7.47**
[7.33–7.74] on the previous milestone, and **7.68** [7.62–8.37] on this
patch. The patch's median was +2.8% versus the previous milestone and
−0.1% versus the original. That is a small repeatable median difference
against the immediate predecessor in these two runs, with overlapping
ranges. Its statistical significance and application-level impact remain
unmeasured.

Warm DTLB loads still resolve directly to arena offsets. In this change a
sparse store must also resolve the physical page to its arena stamp index
for SMC. The extra map check in bulk-pointer resolution grew the compiled
read resolver from 732 to 847 bytes and the write resolver from 767 to 881
bytes. The individual runtime cost of each change remains unmeasured.

The separate CPU-only comparison uses the same protected-mode paged
program and the same integration-test executable for dense and sparse
loans. Each sample runs 120 million counts after warmup. The five
interleaved rounds measured dense **9.930 ns/count** (9.380–10.977) and
sparse **11.187 ns/count** (10.798–22.283). The sparse median was **12.7%
slower** on this store-heavy workload. Reusing the cached arena-page
offset for SMC indexing on direct stores is a focused optimization to
test after pinning its invariants across remaps and partial pages.

Raw samples, binary hashes for the full-system comparison, and the
symbol sizes are in the review archive. Reproduce the full-system run
with `docs/vnext/memory_perf_compare.py --full-system-only --rounds 5`
and paths to the three examples and the test binary. Add
`--modes mixed --instructions 700000000` for the longer run. The archive's
`measure_mapped.py` drives `timed_borrowed_memory` in
`cargo test --release -p rusty_box --no-default-features --features std
--test external_memory`.
