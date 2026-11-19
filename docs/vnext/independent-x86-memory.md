# vNext: independent x86 memory environments

The real x86 interpreter now accepts either existing PC memory or caller-borrowed dense RAM. Flat execution constructs no PC, devices, BIOS/ROM storage, residency map or overflow file. AArch64 internals are unchanged. This is an internal execution seam, not a stabilized public CPU-machine API.

## Baseline and decision

Work continues on `codex/vnext-x86-cpu-only`, starting at `1f90a285fa98c7cfb523c2769a3002975afc5361`. Original comparison revision: `d0fb51225ef49baee4cc0b36e2c4d43b28ebcef6`. The full Composable Emulation Architecture brief and safety doctrine govern this change. No PR or remote write.

The previous implementation separated hardware but still required `BxMemC`. Scalar physical helpers, page walks, DTLB fills and bulk pointers also independently assumed PC legacy holes and PCI-aperture stretching. Changing the constructor alone would preserve those accidental policies in flat execution.

| Essential x86 requirement | Accidental PC coupling |
|---|---|
| Guest page tables, protection/NX, A/D bits, faults and restart | VGA/BIOS apertures, SMRAM routing, ROM allocation |
| Checked backing spans, TLB/cache validity, SMC invalidation | Resident-block map, overflow file, PCI-hole stretching |
| Local APIC architectural decode and MSRs | PIC, chipset, DMA controller and machine timer services |

Only two designs were considered:

| Boundary | Decision |
|---|---|
| Concrete borrowed `ExecutionMemory::{Pc, Flat}` with named `CpuMemory` projection | Selected. Mapping and slow-dispatch decisions stay outside ISA handlers. Cached RAM hits retain typed offsets; no new RAM vtable, mandatory allocation or handler-wide type parameter. This private enum is migration machinery, not a proposed closed registry for third-party devices. |
| Generic `ExecCtx<M>` with narrowly specified memory operations | Deferred. Could specialize dispatch, but spreads a parameter/bound through ISA-wide impl declarations before the backing/fault contract is proven. Not implemented or timed; no claimed performance superiority. |

## Code and invariants

- `memory/flat.rs`: exclusively borrowed bytes and one caller-supplied `u32` code stamp per 4 KiB page. No alignment or `'static` requirement. Host read/write checks the full range before modifying bytes; no mutable backing slice escapes.
- `memory/execution.rs`: PC delegates to `BxMemC`; flat `0..len` is dense RAM, including legacy apertures. It preserves PC SMRAM fetch priority over VGA. Separate RAM/allocation offsets and PC residency epochs remain intact.
- `memory/code_writes.rs`: shared SMC queue, stamp guard and overflow behavior (R5). PC retains its stamp hash and cross-CPU drain; flat indexes its supplied stamps directly. Host/DMA-style writes between runs and guest writes invalidate the same real instruction cache.
- `ExecCtx::assemble` / backing adoption: memory-map policy is part of cache identity. Every flat bind flushes backing-dependent caches, even at the same address/length. Runs inside that exclusive loan stay warm. Rebinding resets owner-specific SMC watermarks and synchronizes A20. Safety does not depend on a destructor or global identity counter.
- `access.rs` / `paging.rs`: one direct-RAM gate serves scalar helpers, page walks, bulk pointers and DTLB fill. Direct pages exclude CPU-local/board MMIO and LAPIC. Warm DTLB bulk hits retain the existing numerical placement/bounds check and trust eligibility proved at fill, as scalar hits do. Mapping changes invalidate those entries. Fetch windows stop at backing and MMIO ownership boundaries; partial pages cannot become full-page host mappings.
- Split scalar reads and word/dword/qword stores validate both page translations and permissions before consuming MMIO or committing operand bytes. Guest page-table A/D side effects remain possible during a fault. Contained MMIO keeps its width; ownership-boundary accesses split.
- Callback mapping changes and guest LAPIC relocation invalidate mappings. LAPIC relocation stops the trace and preserves the existing post-retirement machine-work exit. CPU-only execution does not silently service board effects.
- Typed REP STOS chunks validate host alignment before forming typed slices. Unaligned host or guest storage uses existing scalar unaligned-safe accesses; aligned PC bulk fills remain available.

Physical holes are not guest page faults. Flat unassigned physical data reads explicitly return open-bus `0xff`; writes have no target. Host loading/inspection stays strict. Unbacked instruction fetch exits without successful retirement. Guest paging failures deliver real #PF; host `PagePermissions` denial still delivers #GP. A strict host-fault backend requires a genuinely fallible physical-access contract, not fallback zeros or fabricated #PFs.

The flat loan is exclusive: tracked host/DMA writes occur between runs. This is not concurrent DMA, SMP RAM ownership, an IOMMU, a sparse mapper or a universal bus. Existing optional observers/callback adapters remain; no new dynamic dispatch, dependency or allocation is introduced for the two RAM implementations.

## Validation and performance

Validation host: Intel Xeon Platinum 8370C, Linux x86_64, Rust 1.100.0 (`0ef1bc386`), nightly rustfmt 1.102.0-nightly (`65ce8611f`). All three performance revisions use the same checked-in Cargo.lock and release/std configuration. There are no new dependencies or ARM source changes.

| Check | Result |
|---|---|
| Full release x86/std suite, including integrations, compile-fail fixtures and doc tests | 1,244 passed, 7 ignored across 27 suite results; includes 1,060 unit tests. Run after the functional changes. |
| Final-source release x86/std unit suite | 1,060 passed, 3 ignored. Includes all 16 new execution test functions. |
| Final targeted release regressions | 22 passed across seven harnesses: compile-fail, flat storage, large-page INVLPG, prefetch wrap, shadow-stack rollback, SSE alignment and stack rollback. |
| AArch64 without default features | 1,178 passed, 8 ignored across 25 suite results. |
| AArch64/std | 1,284 passed, 8 ignored across 26 suite results. |
| No-allocation storage/borrow runtime tests | 5 passed with `--no-default-features`. |
| Release x86 checks: no defaults, alloc, bare-metal no defaults, wasm alloc | Passed. Targets: `x86_64-unknown-none`, `wasm32-unknown-unknown`. |
| Linked `rusty_box_no_alloc_smoke`, bare metal | Passed; now constructs and writes stack-backed `FlatMemory` with a partial last page. |
| AArch64 bare-metal alloc check; x86 debug all-targets/all-features check | Passed. |
| Final-source doctrine ratchets | Passed, including zero production `.unwrap()`/`.expect()`, unsafe ceilings, no-panic canary and tier-P coverage. The final rerun deliberately stops after that gate, before the broader CI matrix. |

The 16 added execution test functions exercise both storage implementations where policy overlaps. They cover real 32-bit and long-mode decode/execute; translation and A/D bits; scalar split-store #PF without partial operand writes; #PF/#GP before destructive MMIO reads; actual exception frames, PTE repair and guest IRETD/IRETQ retry; NX; INVLPG; warm DTLB/MMIO separation; LAPIC relocation and post-retirement exit; tracked host/DMA-style writes and guest SMC; queue overflow; same-address backing reuse; partial last-page open bus and failed fetch without retirement; and typed REP STOS with unaligned host/guest storage. Public integration tests check strict bounds/metadata sizing, and a compile-fail fixture rejects backing mutation while borrowed.

REP validation checks guest RIP, RCX, RDI, bytes and total ticks. A dispatch budget is not a universal retired-element count: the existing scalar REP path can charge multiple iterations to `icount`, while FastRep places surplus work in ticks. The test accounts for that existing distinction without changing accounting to make the assertion pass.

The eight ignored ARM tests require the external AARCHMRS dataset (`Features.json`, `Instructions.json`, `Registers.json`). The x86 ignored cases are the existing manual instruction benchmark, the new memory benchmark and cold-rebind benchmark; four doc examples remain ignored.

The boot probe was invoked with `--disk dlxlinux/hd10meg.img --chs 306,4,17 --memory-mib 16 --seconds 1` and returned ENOENT. All three required assets are absent: `cpp_orig/bochs/bochs/bios/BIOS-bochs-latest`, `cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin`, and `dlxlinux/hd10meg.img`. No firmware/Linux boot pass is claimed.

`cargo xtask ci --skip-boot --skip-aarchmrs` passed the doctrine ratchets, formatting, xtask tests, core and devices matrices. I stopped that broader run during its duplicate ARM suite so the timing run could proceed without compiler load; it is not a whole-workspace CI pass. Separately, the untouched previous milestone reproduces six `clippy::chunks_exact_to_as_chunks` errors in `rusty_box_symbols` with CI's exact Clippy command on Rust 1.100.0 (`cab.rs`, `decompress/zstd.rs`, `pdb/dbi.rs`, `pe.rs`). Those unrelated files are unchanged. A clean whole-workspace CI result remains unverified.

Full test commands were `cargo test --release -p rusty_box --no-default-features --features std --no-fail-fast` and `cargo test --release -p rusty_box_aarch64 --no-default-features` with and without `--features std`. The review archive retains stdout/stderr, build-check commands, the baseline Clippy failure and raw benchmark samples. Its broad matrix manifest preserves an initial, superseded REP test assertion failure; the succeeding full x86 run and final reruns are separately identified.

The workspace reset during the task; the exact previous commit was restored from its saved patch and this milestone reconstructed. Lost runs and timings from the former host are excluded from the reported evidence.

### Measured hot paths

The final six-way run pins each child to host CPU 0, rotates/reverses sample order, and runs seven samples of 200 million emulator-counted instructions for each workload. Construction is excluded. Baselines have separate executables with recorded SHA-256 hashes. These synthetic, cache-warm loops do not measure boot, MMIO-heavy programs, paging churn, sustained SMC or a complete application. The shared host still shows substantial variation after our compiler jobs stop; ranges and raw output are retained, rather than claiming zero overhead or a statistically established speedup.

Full-system `perfbench`, median ns/count [min–max]:

| Workload | Original `d0fb5122` | Previous `1f90a285` | This patch | Change vs previous |
|---|---:|---:|---:|---:|
| Mixed ALU/load/store | 11.19 [10.41–13.94] | 12.62 [10.56–15.95] | 13.96 [9.77–15.31] | +10.6% |
| ALU | 8.84 [8.05–19.85] | 8.41 [7.61–9.58] | 8.16 [7.54–10.06] | −3.0% |
| REP MOVSB/STOSB, 64 bytes each | 13.61 [13.04–14.86] | 14.16 [13.12–16.82] | 13.75 [12.84–19.27] | −2.9% |

Both memory implementations in the **same final-source CPU-only test binary**, median ns/count:

| Workload | PC memory | Page-aligned flat | Flat with host offset 1 | Aligned flat vs PC |
|---|---:|---:|---:|---:|
| Mixed | 10.868 | 10.212 | 10.689 | −6.0% |
| ALU | 8.741 | 8.088 | 7.653 | −7.5% |
| REP | 12.811 | 13.466 | 13.898 | +5.1% |

Compare within each group: the CPU-only harness has different guest addresses, RAM size, execution driver and test-build configuration from full-system `perfbench`. Flat is not uniformly faster. Its aligned REP sample range is 12.833–14.676 ns versus PC's 12.265–15.028 ns. These overlaps and the mixed-workload variability prevent a blanket no-regression claim.

An earlier implementation had a repeatable ~20% full-system REP penalty. Disassembly exposed additional calls in the SMC/alignment guards and loss of the previous 64-bit STOS specialization. The final code combines write/sequence observation under one backend match, uses the equivalent shared low-RAM bounds check before selecting high-address policy, and inlines those small guards and the STOS64 chunk proof. The final REP run above does not reproduce that penalty. Permission, MMIO, code-write and residency checks remain active; no unchecked indexing or pointer shortcut was added.

Cold flat binding plus one real instruction measures **76.1 µs/bind** over 200 binds. Every bind flushes backing-dependent caches; keep one exclusive execution loan across repeated run/step calls. This safety cost must be visible in a future public API.

The longer mixed-workload confirmation uses **five samples of one billion counts per revision** with the same rotating order and CPU affinity:

| Original | Previous | This patch | Patch vs previous | Patch vs original |
|---:|---:|---:|---:|---:|
| 10.19 [9.93–10.45] | 10.60 [10.35–10.70] | 10.68 [10.55–11.02] | +0.8% | +4.8% |

The shorter run's +10.6% mixed median did not persist in the longer comparison. The measured incremental mixed cost is small; it is not zero. The remaining flat REP median difference and cold-bind cost are also explicit above. This supports retaining the private boundary for this incremental change, not freezing it as the eventual public provider design or claiming universally preserved performance.

`memory-perf-final-code.json` contains all six-way samples. `memory-perf-long-mixed.json` uses the same tool with `--modes mixed --full-system-only --instructions 1000000000 --rounds 5`. Native profiling/counter tools were unavailable. Timing alone does not identify every remaining dispatch/layout cost; repeat on a stable host before setting a strict performance gate.

## Remaining coupling and next step

`FlatMemory` is public storage; the execution constructor and borrowed view remain crate-private, as the primary brief permits. Separate-module tests exercise the real CPU over externally supplied storage; downstream tests prove the public storage/borrow contract. Arbitrary third-party physical buses and a public run/inspection surface are not yet delivered.

External dense-memory owners can supply their own buffers without PC allocations. An eventual public provider can lend validated storage/code-write parts or typed physical outcomes at this boundary without editing x86 instruction bodies. The current enum does not establish the permanent extension mechanism. Proving an open contract with sparse/fallible/MMIO mappings remains the next experiment.

The CPU's large fixed caches, host permission bitmap, x86-specific policy/address types, `Hardware` and `PcIo` remain. Existing PC lifetime/snapshot/scheduler ownership still controls PC backing management; pointer/length identity alone is not claimed to detect arbitrary internal destruction/recreation of an entire PC object. Caller-borrowed flat backing explicitly avoids that assumption.

The internal CPU-only paths retain the bounded SMC event history. Overflow conservatively flushes the decoded cache on subsequent code-write events; only the PC scheduler currently drains shared events after all CPUs observe them. This preserves correctness but is not a tuned design for sustained JIT/SMC or concurrent DMA. Those workloads and frequent short-lived flat loans need separate performance work.

AArch64 retains its own `PhysBus`, descriptor operations, fetch epochs, GIC and scheduler. No universal CPU trait, Requires system, topology macro, device rewrite or mandatory observer is added.
