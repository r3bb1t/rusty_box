# Toward rusty_box vNext 0.1

This milestone starts at `dae31a8441061036d5f16faff5a717e2bca5dfb6` on `experimental/6-x86-cpu-only-memory`. The preview is a source-level integration target. Public APIs remain experimental.

## Address-space ownership

A host MMIO reservation includes the entire installed interval, including pages in a nested root and bytes covered by a legacy callback. Placement checks built-in MMIO, the active xAPIC page and the architectural LAPIC reset page. It reserves the complete I/O APIC range from `0xFEC0_0000` through `0xFEC1_0BFF`. PIIX3 selects one of 64 positions at 1 KiB increments, and the last 4 KiB window crosses into the next page. The existing I/O APIC config and interrupt paths continue to own that range.

| Reconfiguration | Conflicting request | Accepted request |
|---|---|---|
| PCI VGA memory BAR0 or BAR2 | Check the full sized interval at the existing machine boundary. Cancel that pending relocation and restore the committed BAR value and type flags. The old decoder stays installed and the next guest config read sees its value. | Apply the board's existing map move and invalidate all processors' cached host mappings and decoded code. Zero disables a BAR and a sizing probe updates the sizing readback. |
| `IA32_APIC_BASE` enabling xAPIC | A guest `WRMSR` takes `#GP(0)` before changing the MSR or decoder. The host's MSR API returns `InvalidValue`. | Update the MSR and decoder together, invalidate host mappings and decoded code on that CPU and synchronize pending LAPIC work. Disabled and x2APIC modes have no LAPIC MMIO reservation at the programmed address. |
| Snapshot save or restore | Refuse up front while a host MMIO mapping is installed, including callbacks and borrowed devices. A restore can otherwise introduce board ranges that were absent at mount time. | Continue using snapshots for built-in state. Callers may detach the host devices and remount them after a restore, with placement revalidated. |

Giving the latest decoder unconditional priority would let an ordinary probing sequence hide live devices or change which machine owns DMA targets. Refusal preserves the owner and makes the disposition visible through behavior available to each guest protocol. A BAR write has config readback, while `WRMSR` has an architectural fault path. The preexisting retry and diagnostic behavior for conflicting built-in VGA mappings stays in place. The tested PC topology is one software CPU. [Guest regressions](../../rusty_box/tests/hardware_topology.rs) · [Parity decision](../bochs-parity-divergences.md#host-mmio-ownership-across-guest-reconfiguration)

The guest tests probe BAR size and attempt a move whose last page overlaps a live nested root. They then exercise both mounted devices, typed subscribers, DMA to guest RAM, and the PIC. They also check BAR2, a valid move after refusal and placement after detach. The LAPIC fixture installs a guest IDT and returns from an actual `#GP(0)`. It reads the old MSR and decoder, verifies the external register and subscriber, then moves the LAPIC into a warmed RAM page and verifies that it returns to RAM after moving back.

## Developer workflows and boundaries

| Task | Start here |
|---|---|
| Run firmware-free x86 and inspect state | Use `CpuEmulator::builder()`, load code through `memory()`, use typed `regs()` and handle `SessionStop`. `service_scalar_io` completes the supported I/O request before resuming. |
| Supply memory | Own flat or sparse backing in the application and keep its `MemorySession` loan while running. The low-level constructor still asks the caller for code stamps. Use the owned facade for a dense default. |
| Author a register device | Write ordinary methods operating on `WindowOffset` and `MmioWrite`. Use `MmioWindow::registers` for PC placement. For events, return a typed notification or sequence and register local closures with `.on` or `.try_on`. |
| Supply DMA or IRQ | Borrow only `DmaWrite` or `InterruptOutput` where needed. The board decides which hardware routes supply those capabilities. DMA checks the whole destination before changing memory, and a successful write takes part in code-cache invalidation. |
| Compose a nested PC topology | Combine windows with `.then()` and install the root at one physical base. Join and mount refusals preserve owned values. Static construction also supports caller-owned storage through `install_borrowed`. |
| Reuse a device on AArch64 | Compile the exact same `RegisterBank` and `RegisterChanged`. An application-owned `PhysBus` places two instances, subscribes independently and projects one GIC SPI as `InterruptOutput`. A companion test overlays one instance on the existing native `VirtBoard` and checks guest-visible RAM, register and GIC accesses. |

llvmkit's D2–D4 point to ownership recovery at mount boundaries, explicit erasure for dynamic storage, and typed registers and events. D8 is useful at checks that can actually be discharged, such as exclusive loans or width selection. BAR and interrupt ownership depend on current hardware state and belong at runtime. The ARM proof reuses event and capability contracts with native physical address, layout and interrupt types. Application routing stays architecture-specific. [llvmkit doctrine](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/README.md) · [CPU example](../../rusty_box/examples/vnext/cpu.rs) · [Register example](../../rusty_box/examples/hardware_topology/register_window.rs) · [Event example](../../rusty_box/examples/vnext/event_device.rs) · [Nested example](../../rusty_box/examples/hardware_topology/main.rs) · [AArch64 example](../../rusty_box_aarch64/examples/register_device.rs)

Subscribers that return a control result now use `ExecutionRequest::None` or `ExecutionRequest::Stop` in place of `Reaction::Continue` or `Reaction::Stop`. A stop requests that the execution driver yield at its next supported boundary. Remaining subscribers still receive the event, in order, unless one returns an error. An error takes precedence over an earlier stop request and leaves committed effects in place. Ordinary `.on(...)` observers return `()` and need no migration. [Public event contracts](../../rusty_box_core/src/events.rs)

## Proposed 0.1 definition of done

| Scope | Status and evidence |
|---|---|
| x86 CPU-only execution, convenient owned RAM, caller-owned flat and sparse backing, typed registers | Implemented. Downstream examples and execution, ownership and compile-fail fixtures exercise the public API. |
| Third-party models, typed per-source subscriptions, DMA and IRQ effects | Implemented for the demonstrated scalar interactions. The x86 nested guest fixture checks DMA, cache coherence, IRQ and committed stop or failure outcomes. |
| Safe guest reconfiguration with a live x86 aperture | Implemented for VGA memory BARs, LAPIC and the PIIX3 I/O APIC range. |
| One model reused on both architectures | Implemented for the register and event model, with native AArch64 `PhysBus`/GIC regression and downstream example. The example owns its bus. Public extension of the stock `VirtMachine` scheduler remains deferred. |
| Understood stop and failure boundaries | Implemented for documented routes. Subscribers run in registration order after the device operation. Stops aggregate through delivery and are observed at the execution boundary. A subscriber error ends that event's delivery and leaves committed DMA, IRQ or register effects in place. The PC path may complete later fragments of the instruction, and REP can resume at the same IP after a committed element. The ARM example stops after the retired scalar instruction and resumes from its next PC. |
| Portability and reproducibility | The release suites, compile-fail, no-allocation runtime and bare-metal link, WebAssembly builds, executable downstream examples and DLX login passed. The validation archive records commands, input hashes and results. |
| General physical-exit completion, user-mode ABI, advanced instructions | Deferred. The CPU-only resumable external page is restricted to validated scalar MOV. General MMIO completion, paging exits, SIMD, implicit system accesses and restartable atomic or REP protocols need their own contracts. Existing full-system scalar RMW and REP guarantees apply only to their tested paths. |
| Guest-configurable topology and device snapshots | Deferred. PC topology remains host-placed, with existing built-in PCI behavior. Snapshots require host MMIO mappings to be removed. Public AArch64 mounting and snapshot ownership need their own design. |

**Next milestone.** Validate one real x86 user-mode entry, fault and syscall round trip using the existing CPU facade and caller-owned memory. That workload can establish which stop and resume operations deserve a stable host contract. Keep the AArch64 public mounting decision tied to a concrete second device or a consumer that needs the stock scheduler.

## Validation

| Release suite | Passed | Failed | Ignored |
|---|---:|---:|---:|
| x86 `std`, unit, integration and doc tests | 1,300 | 0 | 8 |
| x86 default-feature unit tests | 1,110 | 0 | 3 |
| AArch64 without default features | 1,180 | 0 | 8 |
| AArch64 `std` | 1,286 | 0 | 8 |
| Core and device libraries without default features | 52 | 0 | 0 |
| x86 allocation-free runtime tests | 5 | 0 | 0 |

All 15 x86 compile-fail cases and 14 topology cases passed. The snapshot consistency fixture now round-trips a valid host LAPIC relocation and then corrupts the serialized decoder to check rejection of a mismatched snapshot. The previous fixture produced the mismatch by writing the host MSR alone. The eight ignored tests in each AArch64 suite require the external AARCHMRS dataset.

Both architectures passed their bare-metal and WebAssembly checks. The x86 bare-metal smoke executable linked without Rust allocator symbols. Four x86 examples and the AArch64 register example ran as standalone downstream executables. The three device models also compiled in an independent `no_std` library with only `rusty_box_devices` as an emulator dependency. DLX reached login at 383 million engine counts and opened a Linux 1.3.89 root shell.

The repository-wide `cargo xtask ci --skip-aarchmrs` passed the doctrine and format gates, both AArch64 matrices, the WHP checks and the decoder and symbols tests. It stopped at six `clippy::chunks_exact_to_as_chunks` diagnostics under `-D warnings` in `rusty_box_symbols` on Rust 1.100.0. The same six errors reproduced on the untouched baseline. The later required emulator checks ran separately. The optional AArch64 Callgrind comparison was skipped because Valgrind was unavailable.

## Performance

The baseline and patch use the same Rust 1.100.0 toolchain, lockfile and release settings. Trials alternated build order on a pinned core of the AMD EPYC 9V74 VM while other builds were paused. The full-system workloads used six trials per build at 300 million engine counts. Direct MMIO used six trials at six million writes, with a state check after each run. The mixed-workload recheck used five trials at 600 million engine counts on a second core.

Rebuilding `perfbench` after the snapshot fixture correction produced a byte-identical executable to the measured patch.

| Workload | Baseline median [min, max] | Patch median [min, max] | Median change |
|---|---:|---:|---:|
| PC mixed | 7.730 [7.64, 8.06] | 7.940 [7.74, 8.69] | +2.7% |
| PC mixed, second core and longer run | 7.490 [7.41, 7.67] | 7.760 [7.56, 8.12] | +3.6% |
| PC string | 9.650 [9.30, 9.75] | 9.390 [9.18, 9.54] | −2.7% |
| PC ALU | 6.015 [5.94, 6.11] | 5.985 [5.89, 6.24] | −0.5% |
| Direct MMIO, no observers | 34.865 [33.16, 40.80] | 33.523 [33.21, 34.12] | −3.8% |
| Direct MMIO, two observers | 34.803 [33.97, 40.55] | 35.492 [33.50, 39.85] | +2.0% |

PC measurements are nanoseconds per engine count. MMIO measurements are nanoseconds per guest write and loop branch. The mixed medians were higher in both runs, and the sample ranges overlapped in every workload. Normalized assembly matched for the CPU loop, dispatcher and dword and qword RAM helpers. Their 64-byte alignment matched as well, while their placements within pages shifted by `0x180`. The workloads issue no BAR or LAPIC moves, so the new ownership checks do not execute there. The data leave binary placement, other paths and host variation open as contributors. The milestone leaves the RAM hot paths alone. [x86 benchmark workloads](../../rusty_box/examples/perfbench/perfbench.rs)
