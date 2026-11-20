# Composable hardware topology

Base commit `1b6a92c8515d66cd88ea4d1e37078da02732f8ff` on `experimental/6-x86-cpu-only-memory`.

## Scope and choice

The PC has two MMIO maps. The chipset map associates BAR and legacy ranges with board tokens. The external registry associates physical regions with software endpoints. The old composed path installed one page and one ISA line per endpoint. Its DMA loan excluded that page and the other registered regions. The existing PCI configuration router dispatches a fixed set of devfuncs on bus 0, and the built-in models return chipset-specific effects for BAR and mapping updates.

| Design | Application wiring | Tradeoff |
|---|---|---|
| Guest-programmable PCI hierarchy | Install functions behind an emulated PCI bridge and let the guest assign BARs and bridge windows | Adds discovery, config forwarding, transactional aperture movement, interrupt swizzling, and reset and snapshot state |
| Fixed nested memory aperture | Combine adjacent typed windows and place the root once | Proves independent ownership, address translation, capability projection, and real guest execution with a host-described map |

The second design reuses the existing scalar dispatch and stop boundary. The register model and GPU remain independent from both boards and buses. The aperture shows how a PCI-style memory hierarchy can forward relative offsets to child windows. Guest PCI enumeration and guest-programmable bridge windows are separate work.

## Authoring and mounting

Previously an application called `PcAttachment::with_model(page, line, device)` for each active endpoint. That path still works. The topology example mounts one root containing a two-page register bank and a downstream branch with two GPU instances.

```rust
let registers = register_window(
    NonZeroU16::new(2).unwrap(),
    RegisterBank::new(0),
    RegisterBank::read,
    RegisterBank::write,
    Subscriptions::<RegisterChanged, SubscriberError>::new().on(log_register),
);
let downstream = first_gpu_window.then(second_gpu_window)?;
registers.then(downstream)?.install(&mut pc, base)?;
```

The register bank declares ordinary methods taking state, window offsets, and writes. The board supplies those arguments through function parameters. The passive model and its subscription route have no DMA or IRQ capability. A device that needs both keeps its existing `MmioModel` implementation. `EventEndpoint::with_subscriptions` pairs an active model with its own handlers before the application wires it to a selected line with `MmioWindow::with_irq`. An IRQ-only model can still use `IrqMmioModel`. The topology adds no model trait.

Children are ordinary owned values stored side by side. Each pair subtracts its left span before forwarding to the right. All child boundaries are page-aligned. The board's aperture handler uses the scalar byte path for an access that straddles a boundary, with each byte reaching its own child in guest order. One external registry entry and one erased call reach the root. Routing and subscriber delivery inside the tree are statically dispatched. An owned root uses one allocation. The borrowed root uses stable caller storage, with named storage types for declaring it on bare metal.

[Downstream topology example](../../rusty_box/examples/hardware_topology/main.rs) · [Plain register model](../../rusty_box/examples/hardware_topology/register_bank.rs) · [GPU model](../../rusty_box/examples/composed_hardware/gpu.rs)

## Ownership and execution

Mount preflight checks the complete aperture for wraparound and conflicts with the board, local APIC, and external registry. It also checks every line before claiming any of them. Duplicate lines within a tree are rejected during composition. A refusal returns the typed subtree and its subscribers. Detachment releases all claimed lines and returns an erased root that can be remounted. Guest IRQ remaps retain the previous milestone's separate board and composed assertions.

Active leaves borrow DMA and exactly their chosen IRQ output. The DMA loan excludes the complete root aperture and existing device mappings. DMA to ordinary guest RAM retains instruction-cache invalidation. Attachment and detachment invalidate CPU host mappings, including pages that were previously cached as RAM. A subscriber can request an execution stop, or report a host error containing the physical address. Either returns at the normal interpreter boundary after committed effects. A stop during an instruction split across windows still allows the other fragment to run.

The topology is for a single-CPU software PC. Child windows are adjacent and fixed at mount time. The board owner places the root outside any aperture the guest may later configure for a built-in BAR or LAPIC mapping. Snapshot save and restore require detachment even for a passive-only root, since its state lives in external models. Detached storage stays erased through remounting. A machine reset clears the routed IRQ assertions and leaves the external models in place. The PC hosts the new wiring, while the models compile against the shared device contracts.

[Topology routing and capability projection](../../rusty_box/src/emulator/hardware/topology.rs) · [Physical registration](../../rusty_box/src/memory/mmio.rs) · [Guest regression fixtures](../../rusty_box/tests/hardware_topology.rs)

## Validation

| Release suite | Passed | Failed | Ignored |
|---|---:|---:|---:|
| x86 `std`, including integration and doc tests | 1,290 | 0 | 8 |
| AArch64 without default features | 1,178 | 0 | 8 |
| AArch64 `std` | 1,284 | 0 | 8 |
| Core and device libraries without default features | 52 | 0 | 0 |
| x86 allocation-free runtime tests | 5 | 0 | 0 |

The x86 run includes all 15 compile-fail fixtures and four new topology regressions. The guest verifies distinct IRQs through both PICs, separate subscriber state, DMA into previously decoded code, refusal to DMA into a sibling window, cross-window reads and writes, and resume after a committed stop or failure. A separate test verifies whole-aperture and IRQ preflight and retries with the returned typed subtree.

The topology example ran as both a workspace example and a standalone downstream executable. The two model implementations compiled as a separate `#![no_std]` library with only `rusty_box_devices` as an emulator dependency. The CPU facade and original composed-hardware examples ran successfully. Host checks with and without `alloc`, the bare-metal x86 check and smoke link, and the WebAssembly check passed. The smoke binary retained the borrowed nested route without Rust allocator symbols. The hash-checked DLX boot reached login at 383 million engine counts and opened a Linux 1.3.89 root shell.

## Performance

The release builds used the lockfile and Rust 1.100.0 with x86 `std` and default features disabled. The drivers ran on CPU 0 of the AMD EPYC 9V74 VM and alternated process order for six samples per profile. PC timings are wall nanoseconds per engine count over 300 million counts per sample. MMIO timings are wall nanoseconds per guest store and following branch over six million stores per sample.

| Workload | Baseline median [min, max] | Patch median [min, max] | Median change |
|---|---:|---:|---:|
| PC mixed | 7.735 [7.69, 8.34] | 7.840 [7.73, 8.28] | +1.4% |
| PC string | 9.655 [9.62, 10.08] | 9.755 [9.31, 10.27] | +1.0% |
| PC ALU | 6.635 [6.01, 7.67] | 6.590 [6.10, 8.13] | −0.7% |
| Legacy MMIO, zero subscribers | 35.726 [32.33, 42.26] | 35.731 [33.05, 40.70] | +0.02% |
| Legacy MMIO, two subscribers | 33.686 [32.35, 38.43] | 34.302 [33.83, 34.81] | +1.8% |

The same-binary topology probe compared three independent attachments with the same three models behind a nested window. Each configuration exercised the same guest address on the third model and checked its final count. Separate attachments measured 39.598 ns [37.71, 47.88], and the nested window measured 38.672 ns [36.76, 47.51], a 2.3% lower median. The ranges overlap in every comparison.

The dispatcher, interpreter loop, four virtual RAM helpers, and both legacy MMIO adapter profiles match the baseline after relocation normalization. Their code is placed at different addresses. The measurements cover guest execution as a whole and do not isolate the cost of routing or code placement.
