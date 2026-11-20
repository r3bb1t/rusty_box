# vNext architecture review

Reviewed from `d7492382bad47c0b2e8ab7fe2b948675e3cf4bf4` on `experimental/6-x86-cpu-only-memory`.

## What should remain

| Boundary | Keep |
|---|---|
| Shared host and model primitives | Keep typed `Emit`, `Subscriptions`, ordered delivery, clocks, budgets, progress, and host mapping descriptions in `rusty_box_core`. The existing `Machine` trait supports host automation through budgets, serial, and keyboard. Each machine supplies its own outcome and error. The useful shared device currency is a local `WindowOffset`, a scalar `MmioWrite`, and narrow `DmaWrite` and `InterruptOutput` capabilities. |
| x86 execution | Keep one interpreter. `CpuEmulator` owns dense RAM and CPU state, and `MemorySession` borrows caller-owned flat or sparse memory. Internal execution borrows preserve the CPU’s relationship to its backing, TLB entries, and code stamps. Typed register descriptors carry width and mutability. An explicitly dynamic lookup is available when the register name is runtime data. |
| PC hardware | Keep ISA wiring, DMA address policy, topology placement, scheduler effects, and subscriber-error translation in the PC. `MmioWindow` uses static model and handler types and erases the root at registry insertion. `PcAttachment`, explicit subscriptions, and borrowed storage are useful advanced routes. |
| AArch64 | Keep `Cpu<L>`, `PhysBus`, `VirtBoard`, and the board’s interrupt, storage, and boot contracts native to that architecture. Shared host automation already works through `Machine`. A second use of a device capability is a better reason to share it than a similar-looking method name. |

llvmkit’s D2–D4 distinguish owned or borrowed handles, explicit erasure, and precise operand types. Its lifter session demonstrates persistent owned state with a short-lived working view. The equivalent useful choices here are short register and memory borrows, typed event handlers, and returning both owned windows on refusal. Dynamic IRQ conflicts and backing bounds belong at the runtime validation boundary, where they can be reported with the original values. [llvmkit doctrine](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/README.md) · [Lifter session](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/crates/llvmkit-ir/examples/lifter_session.rs)

## What should be simpler

- Use `CpuEmulator::builder()`, `regs()`, and `memory()` as the default x86 entry point. The owned facade’s `run` and the loan’s `run_controlled` both report a `SessionRun`. The older loan `run` returns a raw engine count. Document this distinction prominently because that count may include REP and exception handling. Keep the low-level builder, stamps, and dynamic register compatibility path for integration and allocation-free callers.
- Point model authors at ordinary register methods or the existing shared model contracts. Show applications `MmioWindow::registers`, `with_model`, and `.on` first. Explicit `EventEndpoint`, `Subscriptions`, `PcAttachment`, legacy callbacks, and borrowed placement solve different integration needs. Each example should expose only the capabilities it uses.
- Document the host memory boundary clearly. The CPU facade’s physical view accesses backed RAM and ROM. `Emulator::mem_read` and `mem_write` inspect or load backing without dispatching guest MMIO. `WindowOffset` is device-local. The existing `BxPhyAddress` naming is historical. A cross-architecture migration can wait for an actual shared caller.

[CPU-only example](../../rusty_box/examples/vnext/cpu.rs) · [Register-only device](../../rusty_box/examples/hardware_topology/register_window.rs) · [Typed event device](../../rusty_box/examples/vnext/event_device.rs) · [Nested topology](../../rusty_box/examples/hardware_topology/main.rs)

## Correctness decisions

The qword read-modify-write slow path bypassed the external registry and accessed backing even when a guest operand belonged to a device. The baseline regression recorded zero device reads. The fix routes single-page qwords at width eight and preserves bytewise order for split qwords. The direct-RAM path still uses its cached pointer. All scalar RMW widths now check host write permissions before reading the operand, including cases where a warmed translation survives a permission edit. The split qword performs both page translations and permission checks before a device call. Page-walk A/D updates retain their normal semantics.

Two more tests pin the existing effect boundary. A split store can commit both byte effects even when both subscribers fail. The host sees the first diagnostic and its physical address, and the next run starts after the store. REP can exit at the same IP between elements. Its count and destination already reflect the committed element, so resuming completes the remaining operations without replaying the committed write. A subscriber error describes delivery after a device effect. A CPU-only physical-bus refusal from `step_with_external_page` instead promises that its external operation had no effect.

The existing topology tests cover mount preflight over the whole aperture and IRQ mask, duplicate join recovery, DMA rejection across the complete aperture and coherent writes into decoded RAM, and attachment and detachment with warm mappings. The PC tracks board and composed IRQ assertions separately and clears composed claims on detach. Per-event ordering remains registration order, with stop collected across delivery and first error ending that event’s delivery. Split writes can complete later fragments of the already-started instruction.

[Guest regression fixtures](../../rusty_box/tests/hardware_topology.rs) · [Scheduler effect boundary](../../rusty_box/src/emulator/scheduler.rs)

## Current limits and next milestones

CPU-only external memory completion is a guarded scalar MOV path, and scalar port completion has its own x86 protocol. SIMD, implicit system accesses, general atomic transactions, and OS user-mode syscall completion need their own evidence. PC topologies use scalar external routes and a fixed aperture on one software CPU. The caller reserves that aperture against later BAR and LAPIC placement. Composed devices must be detached for snapshots. Detached roots keep their storage erased and can be remounted.

1. **Enforce PC aperture ownership throughout reconfiguration.** Carry the effective reserved ranges through board BAR moves and LAPIC base changes. Prove rejection or explicit precedence with live external windows and warm mappings before attempting PCI enumeration.
2. **Prove a small x86 user-mode program.** Specify fault, syscall, and resume ownership using a real guest ABI and keep dispatch counts distinct from instruction retirement. Let that example decide which existing host exit shapes merit consolidation.
3. **Reuse one model on a second board.** Put a register or DMA model and its typed notifications on AArch64’s own physical bus. This will show whether there is enough common placement or capability policy to extract a small shared helper.

## Validation

| Release suite | Passed | Failed | Ignored |
|---|---:|---:|---:|
| x86 `std`, including integration and doc tests | 1,296 | 0 | 8 |
| AArch64 without default features | 1,178 | 0 | 8 |
| AArch64 `std` | 1,284 | 0 | 8 |
| Core and device libraries without default features | 52 | 0 | 0 |
| x86 allocation-free runtime tests | 5 | 0 | 0 |

All 15 x86 compile-fail fixtures passed. The four examples above each ran as a standalone downstream Rust executable. The register bank, mailbox, and GPU also compiled together as an independent `#![no_std]` library whose only emulator dependency is `rusty_box_devices`. Host checks with and without `alloc`, bare-metal x86, WebAssembly, and formatting passed. The bare-metal smoke binary contains the borrowed topology and has no allocator symbols. DLX reached login at 383 million engine counts and opened a Linux 1.3.89 root shell.

## Performance history

The sparse store penalty included a reproducible extra code-stamp lookup. In the longer same-code comparison, caching the stamp index in the DTLB cut the sparse median by 9.4% versus restoring the lookup. The resulting sparse median was still 5.2% above dense in that fixture. The old PC mixed deltas behaved differently. Relinking identical objects shifted timing materially, and later 7% mixed and 3.7% nested slowdowns failed to recur in their follow-up probes. Preserve the lookup optimization and base future hot-path work on a focused comparison with fixed build inputs. [Recoverable physical-access timings](recoverable-physical-access.md#performance) · [Link placement controls](stabilize-session-hardware.md#binary-placement-and-mixed-workload-timing) · [Recent consolidation timings](hardware-api-consolidation.md#performance)

## Current baseline comparison

The baseline and patch use Rust 1.100.0 and the release lockfile. Samples alternate on CPU 0 of the AMD EPYC 9V74 VM after validation. The first three rows measure nanoseconds per engine count. CPU-only rows measure nanoseconds per dispatch at the indicated batch size. Device and qword RMW rows measure nanoseconds per guest operation plus its loop branch. Each profile uses six samples per executable, with eight for the warm RMW probe.

| Workload | Baseline median [min, max] | Patch median [min, max] | Median change |
|---|---:|---:|---:|
| PC mixed | 7.890 [7.56, 8.18] | 7.675 [7.53, 8.62] | −2.7% |
| PC string | 9.960 [9.48, 10.14] | 9.800 [9.45, 10.15] | −1.6% |
| PC ALU | 6.490 [6.08, 6.83] | 6.310 [6.15, 6.94] | −2.8% |
| CPU-only borrowed, 1 | 39.143 [37.42, 43.75] | 39.350 [36.52, 41.59] | +0.5% |
| CPU-only owned, 1 | 60.500 [58.30, 67.55] | 57.921 [57.83, 62.07] | −4.3% |
| CPU-only borrowed, 256 | 14.800 [14.36, 15.71] | 14.926 [14.58, 17.01] | +0.8% |
| CPU-only owned, 256 | 15.339 [14.35, 16.03] | 15.320 [14.81, 15.55] | −0.1% |
| Direct MMIO, zero observers | 35.784 [34.56, 44.31] | 36.043 [33.75, 40.15] | +0.7% |
| Direct MMIO, two observers | 35.038 [34.04, 38.96] | 33.812 [33.50, 36.97] | −3.5% |
| Three flat devices | 39.198 [37.67, 48.37] | 39.982 [38.16, 41.50] | +2.0% |
| Three nested devices | 37.130 [35.89, 37.99] | 37.044 [36.28, 45.55] | −0.2% |
| Warm qword RMW | 19.608 [18.77, 22.04] | 19.778 [19.36, 23.16] | +0.9% |

All sample ranges overlap. The inspected loop, dispatcher, ordinary dword and qword RAM helpers, and direct and nested MMIO adapters match the baseline after relocation normalization. The warm RMW loop exercises the new permission check with the optional protection bitmap absent. Its median difference and spread give no reason to change the implementation.
