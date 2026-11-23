# Public x86 CPU-only control

This milestone continues on the local `work` branch. Its comparison base is the recoverable-physical-access patch on `9d8e09217ab7340692e04cafefcc13ae46e82b9b`. The public API lives on the existing `MemorySession`, with the processor and immutable memory-map loan still borrowed separately.

## Decision

The published construction path already supplies a real CPU through `BxCpuBuilder`. Building with `alloc` uses a boxed CPU. An allocation-free caller supplies an already initialized CPU and caller-owned RAM, page stamps, and region descriptors. `reset_at` adds firmware-free entry with either zero-based real-mode segments or unpaged flat protected-mode segments. Protected entry installs the shared 24-byte GDT in one checked RAM write at physical `0x800`, then resets the CPU. It uses the same descriptor encoding and cache setup as the full-system emulator. Triple fault enters shutdown.

The existing CPU-only loop already supports a strict dispatch budget, breakpoint reporting, observer requests, idle states, typed CPU errors, and a retirement counter. `step` and `run_controlled` expose this from outside the engine. Each returns progress alongside the reason for stopping, including progress made before a physical fetch or I/O exit. A zero budget performs no work. A step budgets one dispatch, so a REP may still execute several elements and exception delivery can consume the budget. Breakpoint stops occur before execution, with one resume allowed at a reported hit. An IP edit re-arms the address and preserves HLT or other activity state. The existing static instrumentation implementation supplied via `build_with_tracer` works through the same loop and reports a cooperative observer stop.

The register API reuses the engine's existing scalar register views and rejects unsupported widths and writes. It applies state changes according to the affected cache:

| Edit | Effect |
|---|---|
| GPR view | Apply existing x86 width rules. Retain cache mappings and decoded traces. |
| IP, EIP, RIP | Update the execution and restart pointers, invalidate the fetch window, and reset breakpoint resume bookkeeping. Retain DTLB entries. |
| Ordinary flags | Apply the architectural flag writer, including lazy condition codes, interrupt masking, single-step, alignment, and RF behavior. Preserve system and reserved bits. |
| CR2, CR3 | CR2 records an address only. CR3 uses the existing raw root setter, which invalidates translations, fetch and stack windows, and cross-page trace links. Supply an architecturally valid root for the current mode. |
| Host memory writes | `write_ram` retains code-stamp tracking. Call `invalidate_translations` after host page-table edits. |
| Segment selectors, CR0, CR4, EFER | Read-only through this session API. Coherent updates to these fields need a mode or architectural-state operation. |

`read_physical` reads backed RAM or ROM, and `write_ram` writes one checked RAM window. The APIs use physical addresses. The live session retains its borrow across reads, register edits, runs, and writes. Flat and sparse storage use the same control methods. The recoverable external-page step retains its single unpaged MOV scope.

## Downstream use

`cargo run --locked --release -p rusty_box --no-default-features --features alloc --example cpu_only` runs x86 code over a caller-owned RAM slice. The guest updates EAX and stores the result, which the caller reads directly from the same bytes after ending the memory session. To step a guest function, edit registers and resume through a typed port-output exit, run `cpu_facade`. Integration tests also exercise a sparse ROM and fetch hole, a breakpoint and its resume rule, a static SYSENTER observer, and edits to a warm paged address space. The cache test checks that a GPR edit preserves warm data and fetch mappings.

This is an x86 session API. AArch64 retains its own setup, state, bus and scheduling contracts. The firmware-free entry helpers cover two unpaged layouts. Advanced entry, including long mode, uses guest setup or preconfigured CPU state. The session has physical byte access and a scalar register view. Virtual debugger reads, vector registers, interrupt injection, general MMIO, paging-related physical exits, REP device completion and RMW device transactions require separate contracts.

## Validation

All suites below used Rust 1.100.0 and the workspace lockfile.

| Release suite | Passed | Ignored |
|---|---:|---:|
| x86 with `std`, including integration and doc tests | 1,261 | 8 |
| AArch64 without default features | 1,178 | 8 |
| AArch64 with `std` | 1,284 | 8 |
| x86 no-allocation runtime storage/borrow suites | 5 | 0 |

All suites had zero failures. The x86 suite includes the harness covering all seven compile-fail fixtures. AArch64 catalog-dependent tests were ignored. The x86 host no-default/alloc checks, alloc wasm check, bare-metal no-default check, and no-allocation bare-metal link passed. The linked smoke binary retained the actual session execution path and CPU loop. Nightly rustfmt checked both changed packages. The downstream example ran with `alloc` alone and with `std`, returning 23 after an intermediate value of 13.

The DLX regression used fresh, hash-checked Bochs 3.1 ROMs and disk from the recoverable-memory evidence. The headless run reached the `login:` marker at 383M counts and the Linux 1.3.89 root shell.

## Performance

Five interleaved samples per case were pinned to CPU 0 on the AMD EPYC 9V74 VM. The reported medians are wall time in ns per count with min–max in brackets. The borrowed benchmark times a 120M-count batch after warmup. The step benchmark times 20M individual calls over a simple INC/JMP loop with equal final register state. Remaining build/test jobs were confined to CPUs 1–3.

| CPU-only workload | Previous raw run | Patch raw run | Patch controlled run |
|---|---:|---:|---:|
| Paged dense memory-heavy loop | 11.605 [9.551–12.402] | 11.991 [10.329–12.146] | 11.286 [10.230–13.551] |
| Paged sparse memory-heavy loop | 10.965 [9.504–12.008] | 11.655 [9.376–13.502] | 10.639 [10.131–11.263] |
| One dispatch at a time, flat protected mode | — | 24.648 [19.020–30.669] | 24.367 [22.804–33.470] |

The controlled path samples overlap the raw path. These measurements do not establish an incremental slowdown for the wrapper. The CPU-only driver also records whole-process CPU time. The new checks execute once per run or step, outside the dispatch loop.

Longer full-system `perfbench` confirmation with the same build flags for the original baseline, recoverable-memory milestone and this patch:

| Workload | Counts per sample | Original `d0fb512` | Recoverable memory | Patch | Patch vs recoverable |
|---|---:|---:|---:|---:|---:|
| Mixed ALU, loads and stores | 500M | 8.56 [8.37–10.22] | 9.18 [8.43–10.32] | 10.12 [9.79–10.62] | +10.2% |
| ALU | 500M | 7.72 [6.85–8.58] | 7.89 [7.32–8.24] | 8.04 [7.14–9.93] | +1.9% |
| REP MOVSB/STOSB | 400M | 12.03 [10.72–13.77] | 11.36 [11.00–12.24] | 11.92 [10.46–12.87] | +4.9% |

The first, 200M-count comparison produced +10.8% mixed, +12.0% ALU and +4.9% REP medians against the recoverable-memory binary. Normalized disassembly of the usual PC run loop, dispatcher and dword load/store functions matches exactly between the two binaries after removing relocated addresses. Those symbols moved by `0x12430`, so binary placement differs. The data establishes a mixed timing difference with concurrent validation on this host. It does not identify its cause.

The mixed rerun after validation finished measured **7.94** [7.89–8.59] on the original, **7.96** [7.84–8.27] on recoverable memory, and **8.36** [8.20–8.48] on this patch. The patch median remained **5.0% slower than recoverable memory** under these conditions, smaller than the concurrent runs. The generated-code comparison narrows the investigation, but the full-system mixed timing difference remains unresolved.

The evidence archive includes executable SHA-256 hashes, all raw samples, the step benchmark and timing driver, and the regression logs. Full-system reproduction uses `docs/vnext/memory_perf_compare.py --full-system-only`. The borrowed runs use the existing `timed_borrowed_memory` workload, with `BORROWED_BENCH_CONTROLLED=1` to exercise the new wrapper.
