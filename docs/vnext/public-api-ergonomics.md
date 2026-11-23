# Public x86 execution API

This milestone adds typed register views and an owning, firmware-free x86 entry point. The base is `4f3bdb064505ff1c682c668a3a30f818153ca7cd` plus the [session stabilization and external MMIO milestone](stabilize-session-hardware.md). The patch is relative to that combined base.

## What carried over from llvmkit

LLVMKit’s D1–D11 doctrine makes two distinctions that transfer directly. Static width and write capability belong in types, while validation against runtime state belongs at the boundary that has that state. Its typed integer values and `IrBuilder::int_add` let widths follow operands. The erased forms remain explicit for data selected at runtime. Its owned modules and short views show how a session can outlive any particular operation or borrow. The lifter retains useful state across steps, the batch example owns independent modules, and the derived-struct example exposes domain-specific types at the call site. [Doctrine](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/README.md) · [LLVM comparison](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/docs/type-safety-vs-llvm.md) · [Inkwell migration](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/docs/inkwell-migration.md)

LLVMKit’s builder, module and pass code shows where stronger machinery earns its keep. An IR value can belong to a particular module, and a mutating pass can invalidate borrowed IR or analyses. An x86 descriptor is reusable across processors. It holds no register contents. The short register view already associates an operation with one processor. A private descriptor constructor and a sealed value implementation give the needed guarantees here. The compile-fail fixtures exercise public calls, and guest tests cover hardware semantics that depend on runtime state. [Lifter session](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/crates/llvmkit-ir/examples/lifter_session.rs) · [Module per batch](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/crates/llvmkit-ir/examples/module_per_batch.rs) · [Derived struct function](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/crates/llvmkit-ir/examples/derived_struct_function.rs) · [Typed values](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/crates/llvmkit-ir/src/value.rs) · [Builder](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/crates/llvmkit-ir/src/ir_builder.rs) · [Module](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/crates/llvmkit-ir/src/module.rs) · [Pass manager](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/crates/llvmkit-ir/src/pass_manager.rs)

## Audit and the two candidates

| Existing API | Evidence from the implementation |
|---|---|
| `BxCpuBuilder`, `ExecCtx`, `MemorySession` | The builder places the real CPU, and the session assembles disjoint CPU and storage loans. That is useful for external memory and avoids a PC dependency. Even a simple call currently requires users to allocate RAM, size stamps, construct a memory map, reset the CPU and keep the session alive for warm caches. `ExecCtx` is already crate-private. |
| `FlatMemory`, `MappedMemory`, `MemoryLoan` | The exclusive loan protects translations and tracked code writes. A new caller-owned loan is deliberately a cold bind because host addresses can be reused. The manual stamp allocation and loan declaration are unnecessary for an owned dense board. |
| `X86Reg` and software `Emulator::reg_*` | The scalar APIs take `u64` even for a byte alias. `MemorySession` refuses unsupported operations at runtime, while full-system methods have separate scalar, FP and vector entry points. Several invalid calls are distinguishable at compile time. |
| Execution and stops | `run_controlled` already distinguishes observer, breakpoint, activity and engine exits. `step` is one dispatch under the engine’s count rules. Scalar I/O completion already holds the checked request and the restart position. These semantics can be used directly by another frontend. |
| Addresses and instrumentation | `BxPhyAddress`, `BxLinAddress` and `IoPort` already distinguish address spaces. Instruction offsets are CS-relative. Physical host views read backed bytes and have no guest translation or device side effects. Generic `Instrumentation` is already available at CPU construction. |
| PC devices | `MmioDevice` is synchronous and page-relative. Attach/detach already scopes the device loan and invalidates affected mappings. Board-managed DMA and IRQ ordering is a separate problem. |

1. **Typed views on borrowed sessions alone.** This supplies compile-time widths and capabilities and reuses the existing warm session. It keeps caller control of placement and can run without an allocator. First-time CPU-only callers must still manage code stamps and an execution loan.
2. **Typed views plus a dense owner.** Adopted. `CpuEmulator` builds on exactly the same session and views. The owned option hides allocation and assembly, while `MemorySession` remains the direct entry point for external flat and sparse memory. The cost is a small private reborrow at each operation and owned allocations at construction.

## Contract

`Register<V, A>` binds the value type and write capability at compile time. Public constants cover GPR aliases, IP, EFLAGS, core control/segment inspection and the first SIMD register of each width. Checked SIMD selectors cover all 32 modeled storage slots. The existing bridges supply high-byte aliasing and 32-bit GPR zero-extension. Writing XMM preserves the upper bits, writing YMM clears the upper 256 bits, and writing ZMM replaces all 512 bits. These are host snapshot/edit operations. Guest opcodes still check the current architectural mode, register availability and feature bits.

An IP edit updates the restart offset, discards prefetched state, and rearms a previously reported breakpoint. It preserves HLT. Reset reactivates the core. GPR edits preserve data and fetch mappings. EFLAGS reads return `EFlags`. Writes affect the documented status/control subset and preserve system fields. CR0, CR3, CR4, EFER and segment selectors are read-only through descriptor writes. `set_page_table_root` is a checked legacy non-PAE CR3 operation outside virtualization and SMM. It keeps PWT and PCD and invalidates all cached translations, including fetch and cross-page traces. PAE and long-mode root changes still use guest instructions or the established advanced interfaces. Page-table contents and backing are checked by guest walks.

Runtime-selected scalars use `read_dynamic` and `write_dynamic`, with explicit refusal of unsupported or read-only operations. Scalar narrowing uses the established low-bit convention. The older session methods and raw CR3 operation keep their contracts. The software PC exposes the same typed view and retains its existing methods. Alternate engines and AArch64 keep their own interfaces.

The owner keeps disjoint private CPU, fixed-size RAM and code-stamp allocations. Each call builds a short `ExecCtx` with current bases and uses offsets cached against the fixed backing. The owner never replaces that backing or exposes a mutable RAM slice. It retains the code-write sequence across calls, so a tracked host edit invalidates previously decoded instructions. Opening a new externally borrowed session still invalidates all backing-dependent state. No extra allocation is performed by run, step or register access.

The typed physical memory view accepts `BxPhyAddress` and checks a complete backed read or writable RAM operation. Guest page-table edits still require invalidation. Full-system virtual inspection continues to use the existing translation APIs. `SessionStop`, the dispatch-count meaning of `step`, and typed port identity are sufficient for the demonstrated flow. The existing scalar I/O callback retains its width and direction fields. Its saved request survives read-only inspection and is discarded on successful state edits. Keeping that protocol avoids a second exit representation while scalar completion is the only proven case.

The borrowed session still exposes the restricted unpaged scalar MOV external-page path. General physical exits during fetch, page walks, REP and RMW need a separate restart contract. External PC devices still attach through the existing MMIO interface. Future IRQ and DMA extensions can pass explicit typed dependencies to the board and process effects after releasing the device borrow. There is no shared device context or ISA-wide CPU trait in this change.

## Downstream usage

The two examples execute the same x86 function, change an argument between dispatches, inspect the return value 23 at a scalar OUT exit, complete it once, and halt. The owned example also executes a separate RAM write against a caller-owned sparse map.

| Operation | Existing borrowed entry point | Owned typed entry point |
|---|---|---|
| Setup | `BxCpuBuilder::new().build()?`, caller bytes and stamps, `FlatMemory::new`, `MemorySession::new`, `reset_at` | `CpuEmulator::builder().entry(0x1000).stack(0x2ffc).build()?` |
| Load code | `session.write_ram(physical, code)?` | `emu.memory().write_phys(physical, code)?` |
| Change an argument | `session.write_register(X86Reg::Edx, 10)?` | `emu.regs().write(EDX, 10)` |
| Read result | `session.read_register(X86Reg::Eax)?` returns `u64` | `emu.regs().read(EAX)` returns `u32` |
| Run and service OUT | `session.run_controlled(16)`, then `session.service_scalar_io(...)` | `emu.run(16)`, then `emu.service_scalar_io(...)` |

Run `cargo run --locked --release -p rusty_box --no-default-features --features alloc --example cpu_only` to lend the CPU a RAM slice. Use `--example cpu_facade` for a CPU that owns its memory. The typed views also work without `std` or `alloc` through caller-provided storage, as exercised by the bare-metal smoke target. [Borrowed example](../../rusty_box/examples/cpu_only/main.rs) · [Owned example](../../rusty_box/examples/cpu_facade/main.rs) · [External and sparse RAM](../../examples/execution_workflows/README.md#lend-custom-ram-to-the-x86-interpreter)

## Validation

| Release suite | Passed | Failed | Ignored |
|---|---:|---:|---:|
| x86 `std`, including integration and doc tests | 1,273 | 0 | 8 |
| AArch64 without default features | 1,178 | 0 | 8 |
| AArch64 `std` | 1,284 | 0 | 8 |
| x86 allocation-free memory and borrowing suite | 5 | 0 | 0 |

All 12 compile-fail fixtures passed, including the new wrong scalar and SIMD widths, the read-only control register, and both lifetime errors. Runtime tests exercise high-byte aliases, 32-bit zero-extension observed by 64-bit guest code, SIMD aliasing, flags, checked CR3 edits with warm translations, warm cache reuse, self-modifying code, breakpoints, halted restart and scalar I/O completion. The full-system suite also exercised the existing external MMIO device.

Host checks passed with neither feature and with `alloc` alone. The bare-metal no-default build, WebAssembly `alloc` check and bare-metal allocation-free smoke link passed. The smoke executable has no Rust allocator symbols. The owned example also compiled and ran as a separate Rust crate linked against the public library. The hash-checked DLX Linux image reached login at 383 million engine counts and opened its Linux 1.3.89 root shell.

## Performance

The software PC comparison uses the previous milestone as the base and a locked release build with `std` and no default features for both binaries. Both ran on CPU 0 of an AMD EPYC 9V74 VM after validation finished, with alternating order and six 300 million-count samples per configuration. The table shows median wall ns per engine count and the observed ranges.

| Workload | Previous milestone | With typed API | Median change |
|---|---:|---:|---:|
| Mixed ALU, load and store | 8.14 [7.64–8.38] | 8.18 [7.89–8.76] | +0.4% |
| REP MOVSB and STOSB | 10.22 [9.79–11.04] | 10.48 [9.89–10.88] | +2.5% |
| ALU | 6.47 [6.17–7.01] | 6.48 [6.22–6.82] | +0.1% |

The ranges overlap in each workload. After normalizing branch and RIP-relative relocations, the regular CPU loop, dispatcher, and selected dword and qword memory helpers have identical instruction sequences and sizes. Their addresses moved by `0x260`, which also changes their cache-line and page offsets. The comparison leaves the contribution of binary placement and host variability to the REP median difference unresolved.

A separate downstream CPU-only benchmark runs the same guest load, increment and store loop through the borrowed session and the owner in one executable. Each configuration ran six times on CPU 0 with order alternated. The loop warms the engine before timing and verifies the final guest value. Construction and host edits are outside the timed interval.

| Budget per call | Borrowed, warm (ns/dispatch) | Owned, warm (ns/dispatch) |
|---|---:|---:|
| 1 | 38.19 [36.87–45.49] | 60.64 [57.41–72.79] |
| 256 | 16.21 [15.16–16.77] | 16.44 [14.84–18.10] |

The owned setup adds about 22 ns at a one-dispatch boundary in this test. At a budget of 256 the median gap is about 0.24 ns per dispatch. The two profiles show how the dispatch budget changes the cost of entering and leaving the facade.
