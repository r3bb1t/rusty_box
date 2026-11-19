# Recoverable x86 physical access

This milestone starts at `9d8e09217ab7340692e04cafefcc13ae46e82b9b`. It adds a guarded scalar step to the existing `MemorySession`. The step uses the real x86 decoder, segment checks, dispatcher and MOV handlers. The device implementation stays with the caller. Sparse RAM/ROM, PC execution and the AArch64 memory model retain their existing entry points.

## Where errors and effects enter today

| Path | Current behavior and consequence |
|---|---|
| Scalar data | Warm DTLB hits use checked, backing-relative RAM offsets. Cold accesses reach MMIO or the physical helpers in `access.rs`. A byte read can turn a routed error into zero. Wider accesses may retry as bytes, changing callback width and potentially replaying part of an access. Byte writes log failures after trying the route. Borrowed holes have a deliberate open-bus read/discard-write policy. |
| Split scalar | `read_linear_split` and `write_linear_split` translate both pages and check both permissions first. They then consume or write bytes in order. This protects against an architectural fault on the second page, but existing physical callbacks have no way to promise that a later byte can complete or to describe earlier effects. |
| Instruction fetch | Fetch uses a stable allocation window. A split instruction with missing physical backing reports `VetoedDirectRead` and resets the IP. An initial fetch into a borrowed hole previously retried until `CpuNotInitialized`; it now reports the physical address with `VetoedDirectRead`. A guest translation fault still enters the MMU exception path during ordinary execution. |
| Page walks | Guest data walks use `page_walk_read/write_{dword,qword}`, whose slow paths inherit the infallible physical helpers. A failed read can therefore become a descriptor value and cause a misleading architectural fault. Other walk helpers return `Result` but collapse some routed errors into `MemoryError::PageNotPresent`. EPT/NPT and system/descriptor paths require their own audit. A/D writes can occur before a later walk or operand fails and can invalidate decoded code. |
| RMW | Prepare checks write permissions and records a direct pointer or one/two physical fragments. The read can already consume a device value. The writeback helpers return unit, and some handlers update flags or registers before writeback. Returning a retryable error from writeback would risk repeating a destructive read or leaving guest state partially committed. |
| REP | Scalar iteration commits count and index progress after completed elements. MOVS has independent source and destination accesses inside each element; a destructive source read can precede a destination failure. The fast paths use proven direct-RAM spans, chunk bounds and SMC guards. Whole-instruction retry would replay completed elements or consume the current source more than once. |

## Decision and boundary

`MemorySession::step_with_external_page` designates one page-aligned, unbacked physical page for one step. It supports unpaged 8-, 16- and 32-bit integer MOV forms with one memory operand. It checks the instruction class before dispatch and refuses paging, virtualization modes, implicit event delivery, active observers, breakpoints and device paths that would make replay ambiguous. The scope ends when the call returns. Ordinary `run` continues to use the loan's RAM, ROM and hole policies.

After segmentation, alignment and both page permission checks, the scalar path returns a single `PhysicalRequest` for the contained operand or external fragment. Request values are little-endian. The companion fragment may use RAM, ROM or the existing hole policy. It cannot use a second device. For a split store, the device fragment commits first and the RAM fragment follows. Neither operand half commits when the device reports pending or failure. Each device request must commit in its entirety or leave no effect, including destructive reads. A device that cannot handle a fragment width can refuse it.

The host callback runs outside the CPU loop with the exclusive loan still held. Returning `Pending` or `Failed(code)` promises no device effect and leaves RIP at the instruction with the same retirement count. The caller can prepare the device externally and submit the step again. Returning `Complete(value)` represents a committed device effect. The engine immediately re-enters the same instruction with the exact request and completion carried locally. The replay matches address, operation, width and store data, and cannot invoke the host again. Completion stays local to the same call and exclusive loan. The guard excludes paging, event delivery and observers, so the replay sees the same instruction and operand. It consumes the completion, applies the RAM fragment if present, and retires once.

`PhysicalAccessPending`, `PhysicalBusFailure`, and `GuestFaultBeforeDelivery` distinguish the three outcomes. The guest-fault result carries the vector and error code before an exception frame is built. The existing host-page permission policy produces #GP, and the guarded step exposes that existing decision. Failed device or fetch accesses do not write CR2 or synthesize #PF. Executable device memory and general fallible PC routing remain outside the slice.

## Next correctness gates

- **Paging and fetch.** Thread typed physical failures through both walker families and implicit system accesses. Decide first whether external page tables can return side-effectful reads or require stable ordinary RAM. Test absent backing versus a genuinely nonpresent PTE, second-page #PF before any device request, A/D stores that touch cached code, and restart at a split fetch. A replay proof must cover a walk changing the instruction bytes before the first exit.
- **REP.** Use the existing element boundary. Before a destructive source read, prove the destination can commit or keep a completion for the current element. A test must exit after successful elements and again between the read and write of one element. Verify exactly-once device calls, IP, RCX, RSI/RDI, direction and count width, bytes already committed, and that direct-RAM bulk execution stops before the external range.
- **RMW.** Keep the read value and lock/ownership across the paired write, or obtain a device operation whose completion commits the RMW atomically. Guest register and flag changes must follow the commit or be restorable. Test split operands, a destructive device read followed by a refused write, locked ownership, and a write that invalidates a running trace. The scalar slice uses a fixed-size request and a statically dispatched, single-use host callback.

## RAM, SMC and debugger review

A warm writable DTLB entry for borrowed RAM already names the arena page. Use that page for the SMC stamp index on direct scalar stores. The invalidation event still names the guest physical page. This relies on the non-aliased, immutable map and on rebind invalidating all backing-dependent DTLB entries. PC keeps its existing stamp hash. Slow writes and bulk/RMW paths keep their original proofs. A regression caches a guest's code in an arena slot different from its physical page number, then patches it on a warm store and executes the new bytes.

`MemorySession::cpu()` already permits immutable register inspection without a rebind. A future stopped-session editor can reuse the existing register adapters and architectural setters. A GPR write needs no backing-cache flush. An IP write needs the appropriate fetch-window and stop-state bookkeeping, while control registers, segment state and paging changes need their existing mode, translation or decode invalidations. Tracked RAM edits can keep using `write_ram`, with TLB invalidation when the debugger changes page tables. Provide typed operations at that boundary instead of a general mutable CPU borrow or a replacement memory loan for each edit.

## Validation

All builds used the checked-in lockfile and Rust 1.100.0 on Linux x86-64. The portability checks covered x86 host without default features, host with `alloc`, bare metal without default features, wasm with `alloc`, and a linked `rusty_box_no_alloc_smoke` on `x86_64-unknown-none`. Nightly rustfmt passed for the x86 package.

The headless DLX example used `RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=450000000`. It reached `login:` at the 383M checkpoint and opened the Linux 1.3.89 root shell. The ROMs came from the [Bochs 3.1 source](https://github.com/bochs-emu/Bochs/tree/30cfd566ecc8deb33e56e801e32b444a7041b6d2/bochs/bios). The DLX disk was extracted, without running the installer, from the [official Bochs 3.1 release](https://github.com/bochs-emu/Bochs/releases/tag/REL_3_1_FINAL) after checking the release's SHA-256. The pre-boot input hashes and guest output are in the validation evidence.

| Release check | Passed | Ignored |
|---|---:|---:|
| x86 with `std`, including integration and doc tests | 1,256 | 8 |
| AArch64 without default features | 1,178 | 8 |
| AArch64 with `std` | 1,284 | 8 |
| x86 no-allocation runtime storage/borrow tests | 5 | 0 |

All suites had zero failures. The x86 total includes the harness that verified all seven compile-fail fixtures. The AArch64 ignored tests require the AARCHMRS catalog. The x86 tests include a second-fragment permission denial before a first-fragment device write, both orders of RAM/device boundaries, a host refusal with untouched RAM, a device pending read with unchanged registers and IP, and exactly one retirement after completion. The existing paged split-store and REP #PF/IRET retry tests also passed.

## Performance

The three builds used the same release/std features and lockfile. Samples were interleaved on CPU 0 of an AMD EPYC 9V74 VM. Remaining validation jobs were confined to CPUs 1–3. Times are median wall-clock ns per emulator count with min–max in brackets. These loops exercise warm paths. They do not measure device service latency, unpaged workloads dominated by split operands, or sustained SMC.

Full PC example, five rounds of 200 million counts per case:

| Workload | Original `d0fb512` | Previous `9d8e092` | Patch | Patch vs previous |
|---|---:|---:|---:|---:|
| Mixed ALU/load/store | 9.17 [8.14–10.74] | 9.59 [8.62–10.39] | 9.57 [7.99–11.34] | −0.2% |
| ALU | 7.66 [6.94–7.78] | 7.46 [6.78–8.62] | 6.96 [6.17–7.74] | −6.7% |
| REP MOVSB/STOSB | 11.19 [9.92–11.41] | 11.12 [10.07–14.18] | 12.11 [10.61–13.54] | +8.9% |

Paged CPU-only store-heavy loop, five rounds of 120 million counts per case:

| Build | Dense | Sparse | Sparse vs dense |
|---|---:|---:|---:|
| Previous milestone | 13.110 [11.130–17.557] | 13.871 [13.026–16.652] | +5.8% |
| Patch with physical stamp lookup restored | 12.531 [10.239–14.556] | 14.374 [12.918–16.945] | +14.7% |
| Patch with cached DTLB stamp index | 12.194 [11.047–13.364] | 13.109 [10.351–14.650] | +7.5% |

The lookup control keeps the patch and substitutes `mem.stamp_index(addr)` for the direct-path choice in `ExecutionMemory::smc_dec_write_stamp`. The 120M run measured an 8.8% reduction in sparse median time versus that control, with overlapping ranges. It isolates the stamp lookup more directly than comparing against the earlier reported 12.7% dense/sparse gap. The original baseline only enters the full PC comparison because it has no borrowed-memory workload. `CpuError` and `Result<()>` are each 32 bytes in both the previous and patched release build.

A second sparse run used five rounds of 300 million counts for the control and patch:

| Build | Dense | Sparse | Sparse vs dense |
|---|---:|---:|---:|
| Physical stamp lookup restored | 10.744 [10.149–11.781] | 12.396 [11.961–14.547] | +15.4% |
| Cached DTLB stamp index | 10.679 [9.879–11.830] | 11.229 [10.912–12.248] | +5.2% |

The longer run reduced sparse median time by **9.4%** versus the same-code lookup control. Median whole-process CPU time, which includes setup, fell from 12.093 to 11.046 ns/count. This attributes a substantial portion of the warm sparse store penalty to the second map lookup. It leaves a residual sparse/dense difference on this workload.

The 400M-count REP confirmation ran after validation finished. Medians were **10.03** [9.82–10.60] for the original, **10.15** [9.72–10.72] for the previous milestone and **9.89** [9.40–10.10] for the patch. The short run's +8.9% median did not recur. These ranges support keeping the warm paths as implemented and leave the cost of unpaged split-heavy workloads open.

The evidence archive contains the raw timing samples, executable SHA-256 hashes, the sparse benchmark driver, and validation logs. Full-system reproduction uses `docs/vnext/memory_perf_compare.py --full-system-only` and the separately built `perfbench` executables. The sparse driver invokes the existing ignored `timed_borrowed_memory` test with the dense and sparse placements.
