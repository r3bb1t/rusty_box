# Linux image observation and LibAFL execution

This milestone starts at `0c1eeefddfb2d6cbdb165b76ed65cb2c1c3691c4`. The integrations live in an independent host-side workspace. The core API addition is `CpuEmulator::instrumentation()`, the immutable observer view used to read the result after a CPU-only run. It adds no instruction-loop hook and matches the existing full-system observer accessor. [Example workspace](../../examples/execution_workflows/README.md) · [Owned CPU facade](../../rusty_box/src/cpu/memory_session/owned.rs)

## Linux image observation

The DLX guest boots and accepts host-driven shell input. Its tools are a.out, and the Bochs disk contains no ELF loader module. A size-checked ELF upload was refused by the guest. The reproducible ELF workflow uses the Debian 12 i386 kernel and a minimal BusyBox initramfs. DLX still serves as the unchanged BIOS boot regression.

The example uses the same direct-boot entry as the Alpine example. Alpine's syscall tracing hooks fast system calls before transition and can inspect a saved address-space root. An entry hook is too early to report a successful `execve` or find all its demand-paged bytes. Linux's ELF loader installs `PT_LOAD` mappings and arranges the return to the new entry. The controlled ELF calls i386 `getpid` as its first operation. A debugger breakpoint at the next user instruction means the entry page has executed and the real kernel has returned the PID in `EAX`. Typed registers identify the running privilege level and CR3. Debugger translation follows that live address space without constructing an x86 page-table walker in the example. [Alpine syscall trace](../../rusty_box/examples/alpine/alpine_strace.rs) · [Linux ELF loader](https://github.com/torvalds/linux/blob/v6.1/fs/binfmt_elf.c) · [i386 syscall table](https://github.com/torvalds/linux/blob/v6.1/arch/x86/entry/syscalls/syscall_32.tbl)

The guest shell returns the actual child PID from `$!`. A FIFO keeps that child from reaching `exec` until the host arms the breakpoint. `ProcessCreated` identifies the child, `ProcessImageMapped` records its new executable image, and `ProcessExited` comes from that child's shell `wait`. The driver cross-checks the three PIDs. COM1 carries the replacement after a guest-produced readiness line. The original `cat` is saved by renaming its BusyBox link in the guest root. The replacement is staged and size-checked using guest commands, then renamed into place. The same guest restores the original link and proves it by reading a known file. [Guest executable](../../examples/execution_workflows/guest/replacement.S) · [Host driver](../../examples/execution_workflows/src/bin/linux_cat.rs)

### Resolver and identity

`ImageObservers::on` is an infallible event-only subscription. `on_binary` adds a `Resolved<'_, AnalyzedBinary>` argument and can request an execution stop. Preparing a paused observation runs the expensive resolver only when a binary handler is registered. The prepared object owns its metadata. Delivery borrows that object, with the emulator loan already released. Handlers run in registration order, share one resolution and finish delivery before the driver handles the stop. A failed analysis produces a host diagnostic and leaves plain event delivery available.

The ELF parser reads bounded headers and resident executable bytes at guest virtual addresses. It validates class, data encoding, machine, object type, program headers, segment extents and entry. The demonstrated image is static ELF32 with zero load bias. The file preview supplies the breakpoint, while the mapped memory supplies the reported headers and executable SHA-256. File-only section tables, symbols and nonresident bytes are outside this result. A missing translation or invalid header produces an explicit diagnostic.

Every observation has a generation distinct within the host process even if the OS reuses a PID or page-table root. There is no cross-event cache. The result describes the code read at that boundary, so later writes and unmaps require a fresh observation. The image-relative lookup takes both the address-space identity and a PC and covers only the measured file-backed executable ranges. A target collector can key a sample by the loaded-code digest, image generation and relative PC. Ongoing identity tracking across general exec, mappings and scheduler switches remains deferred. [Resolver and ownership tests](../../examples/execution_workflows/src/linux.rs)

### API decisions

The useful llvmkit principles are explicit ownership and erasure, a borrow limited to the operation that can honor it, and a test for mistakes the compiler can rule out. A compile-fail doctest protects the resolver's borrowed metadata from escaping its callback. Ordinary event subscribers keep the existing typed `Subscriptions` path. The binary helper has one additional concrete closure shape with private host-owned storage. An extractor registry would add indirection before another real dependency is needed. [llvmkit doctrine](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/README.md)

## LibAFL execution and reset

The optional feature pins LibAFL 0.16.1. The adapter implements LibAFL's `Executor` and `HasObservers`. `OwnedMapObserver`, `MaxMapFeedback`, `StdFuzzer` and the upstream in-memory corpus and queue scheduler perform the interesting-input decision. The sample feeds a bounded set of controlled byte inputs and checks real guest branch coverage. `--replay` prints the first byte, guest exit classification, PC, engine counts, target-owned tally and edge map. Unexpected stop diagnostics include input, PC and engine count. [Executor and fixture](../../examples/execution_workflows/src/fixture.rs) · [LibAFL example](../../examples/execution_workflows/src/bin/libafl_fixture.rs)

The fixed guest uses five explicit source and destination edges. Branch and reset hooks are the only active categories. One-byte inputs change the branch path. The timeout records observed branch presence through the bounded prefix. Engine counts are dispatch counts, including the semantics of the existing CPU session. Ordinary guest rejection is an application result. A budget is a LibAFL timeout, and the adapter treats an unexpected interpreter or harness failure as a host error. Feedback sees only the intended fixture outcomes.

Each case restores all 16 KiB of RAM, rebuilds the firmware-free entry state with `reset_at`, clears branch state and delivers input. The CPU allocation stays owned by the harness. The RAM restoration goes through the existing code-stamp API and the reset clears CPU translation and decode state. Tests warm the modified comparison, restore its original bytes, then interleave all input classes and compare each replay. The fixture leaves paging disabled and uses no devices, IRQs, external physical accesses, timer inputs, or time-reading instructions. The engine count is measured as a delta. The fixture's deterministic outcomes do not depend on the object's cumulative counters.

Full RAM restoration is cheap enough for this fixture and accounts for every byte a guest can mutate. Dirty-page tracking needs to account for every memory writer and code invalidation, including DMA and writable sparse backing. Copy-on-write has a mapping and ownership cost with no demonstrated benefit at this RAM size. Reconstructing the CPU per input is a useful isolation oracle and adds allocation and decode-cache construction that the tested reset avoids.

### Full-system isolation

For an arbitrary composed machine, use reconstruction and bounded replay from a declared initial state. A factory must recreate the CPU and memory, topology and mapping ownership, disk/firmware inputs, device-local state, DMA and IRQ state, timers, queues, subscribers and their captured host state. External services must be replaceable or driven from recorded responses. The Linux example starts with a new initramfs and board every time. RTC and kernel entropy can differ between boots, so it verifies equivalent behavior for this workflow without claiming complete machine determinism.

A full-system checkpoint requires quiescence and a checkpoint/reset contract for every mutable owner, including storage overlays and host participants. Current snapshots refuse mounted host MMIO. Detach and remount preserve those devices' live state, so that sequence alone cannot restore a checkpoint. Guest observation provides target identity, while repeatable input delivery and whole-machine state ownership remain separate requirements. [Snapshot guard](../../rusty_box/src/snapshot.rs) · [Current preview boundaries](preview-0.1.md)

## Validation

- Release x86 library and integration suites passed 1,275 tests, including the existing compile-fail checks. The x86 doctests passed 25 more. AArch64 passed 1,180 default-feature tests and 1,286 `std` tests. Ignored tests retain their existing status.
- The x86 `std`, allocation-only and no-allocation checks passed. The bare-metal and WebAssembly builds and the five CPU-only no-allocation tests passed. The existing downstream CPU, event and nested topology examples built and ran. The new workspace passed with and without LibAFL, including the borrowed-result compile-fail check, and passed its own Clippy check.
- The BIOS DLX regression reached a root shell on Linux 1.3.89. Three fresh boots of Debian 6.1.0-50-686 reached the COM1 readiness signal, ran the uploaded ELF, matched the mapped digest and live PID, printed `RB_REPLACEMENT_OK`, returned status zero and verified the restored BusyBox `cat`. The observed mapped-code digest was `89ff5f2132656d8e4654123d445bc5f008c8ee331b39724c8ffc9c84c3713aa8`.
- The top-level `xtask ci --skip-aarchmrs` run reached the existing `rusty_box_symbols` Clippy failures for `chunks_exact_to_as_chunks`. The inherited symbols code is unchanged. The optional external AARCHMRS dataset was unavailable.

## Performance

On an AMD EPYC 9V74 host, each of two release sessions ran six paired rounds of 20,000 cases per mode on the same pinned logical CPU, alternating which mode went first. Each mode ran the same rotating inputs. The reported throughput covers the Rusty Box harness before LibAFL feedback bookkeeping. The table shows the final session's median and range.

| Mode | Restore 16 KiB, reset CPU and inject input | Execute and read result | Cases/s |
| --- | ---: | ---: | ---: |
| Instrumentation disabled | 5.09 µs (4.38–6.32) | 28.72 µs (25.08–35.95) | 29,745 (23,603–33,859) |
| Exact branch edges | 4.59 µs (4.34–6.35) | 26.60 µs (25.36–36.77) | 31,985 (23,154–33,586) |

The median of the paired execution-time changes was +9.0% in the first session and −5.2% in the final session, with individual changes across the two sessions from −25.8% to +40.8%. Median reset costs ranged from 4.54 µs to 5.09 µs across both modes and sessions. The measured variation prevented a reproducible estimate of coverage overhead and leaves the earlier full-system results unresolved. The engine change adds a read-only accessor outside the instruction loop.

The live Debian observation took 4.8, 4.9 and 5.9 µs, including PID checks, debugger translations, ELF validation and hashing. The final synthetic analysis and delivery probe used six rounds on the same pinned CPU. First analysis of resident fixture bytes took 317 ns (293–391), the cached borrowed lookup took 0.8–0.9 ns and dispatch to three handlers took 4.4 ns (4.3–6.1). The cached readings include the timing loop's measurement floor.

## Remaining scope

The implementation establishes executable substitution, observation of a controlled Linux exec checkpoint, typed lazy mapped-ELF analysis, and repeatable CPU-only LibAFL evaluation. The next Linux milestone can validate an observation point independent of the test executable's entry prefix and collect samples across scheduler switches. That work also needs explicit image-lifetime invalidation to support reusable resolution across events. General Linux process discovery, demand-paging completion, shared-object identity, symbols, full-system fuzzing and third-party checkpoint support remain outside this slice.
