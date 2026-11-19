# Stabilize x86 sessions and attach a PC device

The starting revision is `4f3bdb064505ff1c682c668a3a30f818153ca7cd` on `experimental/6-x86-cpu-only-memory`. This work extends the existing x86 session and the single-CPU software PC. AArch64 uses its own architecture and device interfaces.

## Session boundaries

`step()` budgets one engine dispatch. It can stop at a breakpoint, deliver a guest fault and leave the IP at the handler, or execute several REP elements. `SessionRun::instructions` uses the engine's counts. A breakpoint at the boundary of an observer stop takes precedence so the caller sees the hit before its one-resume state is consumed. Editing IP preserves HLT. Reset changes activity back to active. GPR edits keep the warm mappings validated by the previous milestone's cache test.

`EntryMode::Protected32WithGdt` accepts a physical GDT base. The whole 24-byte table must fit in one writable RAM window and within the 32-bit address space. The session installs it before changing CPU state and sets the exact descriptor limit. `Protected32` retains the 0x800 convenience layout. A sparse test supplies only the page at 0x2000 and reloads DS through its GDT at 0x2800.

### Scalar port completion

The CPU's existing I/O gate already performs privilege and intercept checks. A scalar `IoUnavailable` records the checked port, width, masked OUT data, decoded instruction and actual next RIP at the existing restart branch. The outer run still reports the original typed exit, with RIP restored and the instruction unretired.

`MemorySession::service_scalar_io` hands that request to one host callback. `None` keeps the request pending and requires a device with no committed effect. `Some` commits the already-checked instruction: IN writes the exact accumulator view, OUT accepts the host acknowledgement, and the recorded next IP becomes current. It charges one instruction count and emits the execution and I/O observer callbacks for completion. The callback runs before the guest commit, so I/O instrumentation observes an already serviced transfer. The session keeps the continuation under its exclusive loan. Execution, register edits, successful memory writes, translation invalidation, breakpoint reconfiguration and reset discard it. A host panic discards it as well.

The completion avoids re-decoding or re-running a privilege check after the device may have changed state. Tests cover all three IN widths, a wrapped IP, accepted and pending completion, register and code edits that invalidate the request, and string I/O refusal. The downstream function example uses the returned OUT request to acknowledge the trampoline instruction.

## PC device attachment

`MmioDevice` supplies synchronous little-endian read and write methods on page-relative offsets. An external developer implements the trait on an ordinary Rust value and calls `Emulator::attach_mmio_device` with a page base and an exclusive stable device loan. The existing MMIO registry owns that loan alongside its callback regions and dispatches only at a mapped access. The caller can detach by base to recover the loan. The failure result returns the loan as well. A platform with no allocator can supply a device in statically placed storage; the bare-metal smoke binary links the attachment method.

The slice uses one page per attachment on the single-CPU software PC. Attach rejects existing registry, chipset and LAPIC overlaps and invalidates backing-dependent mappings. Detach does the same invalidation before RAM is exposed to the CPU again. Keep the attached page separate from subsequently relocated board windows. The existing callback MMIO API reserves device pages by rejecting an overlapping legacy callback registration with its existing panic-style error contract. Its unmap operation leaves loan recovery to explicit detach. The existing registry conservatively disables direct data mappings while populated. Snapshots do not serialize external device state.

The downstream register device has independent tests and a real PC integration test. The guest warms a RAM lookup, loads and writes the attached register, and returns to the original RAM after detach. Host RAM inspection during attachment bypasses the device. The slice uses the existing scalar MMIO route. IRQ and DMA need a board-owned effect boundary after the device borrow ends, with explicit interrupt-line or memory dependencies and ordering relative to CPU progress. The device API carries no PC-wide context in anticipation of those features.

## Binary placement and mixed-workload timing

Built the recoverable-memory revision (`88b31be`), the control-API revision (`4f3bdb0`) and this working patch with the lockfile, Rust 1.100.0/LLVM 23.1.5, LLD 23.1.5, and release `std` with default features disabled. Benchmarks were interleaved and pinned to CPU 0 on the AMD EPYC 9V74 VM. Validation jobs during the first comparisons were confined to CPUs 1–3. Timing is median wall ns/count [min–max]. Each row below uses five 300M-count samples.

| Full PC workload | Recoverable-memory base | Control-API base | Patch |
|---|---:|---:|---:|
| Mixed ALU/load/store | 9.00 [8.56–10.39] | 9.54 [9.21–10.85] | 8.85 [8.49–9.65] |
| ALU | 7.27 [6.68–7.37] | 6.89 [6.53–7.90] | 7.11 [6.81–10.19] |
| REP MOVSB/STOSB | 11.51 [10.19–12.24] | 11.52 [10.11–14.02] | 11.81 [10.15–13.07] |

The patch changes CPU and execution-context field offsets. Its regular CPU-loop symbol is 2,665 bytes versus 2,741 at the control-API base. The dispatcher retains its symbol size but uses different field offsets. The comparison above measures the whole executable with no external device attached. It does not establish device overhead or a general speedup.

To isolate placement, a compiler linker wrapper linked the control-API benchmark inputs as-is and with 16, 48 and 256 bytes in a section before `.text`. Cargo compiled the Rust objects once for this experiment. The section moves each inspected function by the requested byte count. The regular run loop, dispatcher, dword load and dword store have the same symbol sizes and normalized disassembly in all four links and at the recoverable-memory base.

| Mixed binary | Loop address mod 64 | Loop page offset | Median [min–max] |
|---|---:|---:|---:|
| Recoverable-memory base | 32 | 0xba0 | 9.49 [8.64–10.29] |
| Control-API, default link | 16 | 0xfd0 | 11.87 [9.11–12.51] |
| Control-API, +16 | 32 | 0xfe0 | 11.39 [8.96–11.66] |
| Control-API, +48 | 0 | 0x000 | 9.34 [7.73–10.23] |
| Control-API, +256 | 16 | 0x0d0 | 9.93 [9.50–10.29] |

Relinking alone materially changes the timing. Matching the base's mod-64 alignment (+16) did not reproduce its median. Preserving the default link's mod-64 alignment (+256) still changed timing. This rules out the four inspected instruction sequences and a single function's cache-line alignment as sufficient explanations for every result. Larger placement effects include page-offset and cache-placement differences. Linux denied user-space requests for cycle, instruction and L1 instruction-cache miss counters, so the experiment cannot assign the difference to a specific cache or predictor mechanism. The sample ranges also show host variability.


A second mixed run used five 400M-count samples after the validation jobs finished. The median was 7.92 [7.87–8.18] for recoverable memory, 8.51 [7.56–9.92] for the default control-API link, 8.15 [7.98–8.82] for +48, 8.48 [7.95–8.68] for +256, and 8.16 [7.85–8.30] for the patch. Thus +48 recovered most of the median gap in this run, while +256 was close to the default control. The patch was 4.1% faster than the control-API base and 3.0% slower than recoverable memory here. Changing link placement alone can produce a difference of the reported size. The exact cause of the previous result and the size of any residual code cost remain unresolved.

The archive includes the linker wrapper and its scripts, the assembly comparison, exact binary hashes and raw interleaved samples. The benchmark driver accepts case names, count, rounds, modes and CPU affinity. The link controls are experimental binaries confined to the evidence workspace.

## Validation

| Release suite | Passed | Ignored |
|---|---:|---:|
| x86 `std`, including integration and doc tests | 1,267 | 8 |
| AArch64 without default features | 1,178 | 8 |
| AArch64 `std` | 1,284 | 8 |
| x86 no-allocation runtime storage/borrow tests | 5 | 0 |

All suites had zero failures. The x86 run verified all seven compile-fail fixtures. AArch64's ignored tests require the AARCHMRS catalog. The host no-default and alloc checks, bare-metal no-default check, alloc wasm check, no-allocation bare-metal link and nightly formatting check passed. The smoke executable retains both session execution and device attachment.

The downstream CPU-only example returned 23 at the port exit, completed OUT through the new API and halted. The standalone MMIO and full-PC tests run with `cargo test --locked --release -p rusty_box --no-default-features --features std --test external_mmio_device`. The hash-checked DLX regression reached `login:` at 383M counts and opened a Linux 1.3.89 root shell.
