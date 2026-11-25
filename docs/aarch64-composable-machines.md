# Compose an AArch64 machine

The embedded controller is a separate crate that uses only public APIs. Its AArch64 firmware configures a temperature threshold, waits in `WFI`, then reads the alarm and acknowledges the IRQ. The handler reads the GPIO inputs, writes an output, and sends that value over a digital link. The second controller wakes for the incoming frame and writes the value to its own GPIO.

```sh
cargo run --locked --release -p rusty_box_embedded_controller
cargo test --locked --release -p rusty_box_embedded_controller
cargo test --locked --release -p rusty_box_aarch64 --features custom-board --doc
```

The demo prints `Controller A GPIO 95, controller B GPIO 95`. Start with the executable for assembly, host control and the connection. The controller and its digital protocol are purpose-built examples. The guest tests follow the documented register map and AArch64 instruction semantics. [Executable](../examples/embedded_controller/src/main.rs) · [Board and firmware](../examples/embedded_controller/src/lib.rs)

## Assemble a board

Enable the AArch64 crate's `custom-board` feature. It works without `std` and `alloc`. The controller crate enables it on its dependency. A standalone build of the built-in VirtMachine can leave the feature off.

1. Construct `Ram` and optional `Rom` from caller-owned slices. `AddressRange` checks each extent. `MappedDevice` pairs a model with a range and one naturally aligned scalar access width.
2. Join devices with `DevicePair::new` and put the tree into `AddressSpace::new`. Both consume their inputs and validate overlaps before any guest executes. The type records the exact mounted model at every position.
3. Move the checked board and `Cpu` into `Aarch64Machine::new`. Call `step` with `RunBudget::Ticks` to advance virtual time even during `WFI`. `RunBudget::Instructions` bounds retirement, so a waiting processor with recurring deadlines may keep advancing time with that budget. The runtime settles IRQs, architectural outputs, PMU bounds, and device deadlines, then runs the existing static trace engine. A guest PSCI reset request returns control to the host, which can decide whether to call `reset`.

Use `MachineBoard` to supply another board policy. Its `PhysBus` handles data accesses, instruction fetch and translation descriptors. The runtime passes timer, PMU and DCC outputs to the board at synchronization boundaries. The board chooses which physical and virtual interrupt levels to return. The supplied `AddressSpace` uses a level-OR direct IRQ for its mapped devices. A board that needs an interrupt controller can implement its own routing with `MachineBoard`.

RAM and ROM may supply stable instruction fetches. MMIO fetches and page-table reads fault, and a descriptor compare-and-swap uses the exclusive bus loan for both operations. Guest stores use the CPU's usual cache-coherence path. `with_board_mut` clears cached code, translations and the exclusive reservation before the host edits the board. A custom bus that writes memory on behalf of a device marks that effect in `BoardInputs::memory_changed` and leaves `devices_touched()` set if a guest access triggered it. That causes invalidation before the next guest instruction. A subsequent machine run always settles any host-driven IRQ or deadline changes.

## Implement and control a device

Implement `IrqMmioModel` for scalar registers. `WindowOffset` is relative to the mount, so the model works at any aligned base. `ClockedDevice::advance` receives the machine clock and a single interrupt output. Consume any work due at that time and return the next strictly future deadline. A deadline still due after synchronization produces `InvalidDeadline`. The machine can advance to the earliest scheduled deadline while the processor waits. `reset` runs at the current virtual time and the map clears each mounted IRQ. The example resets its peripheral registers and keeps its RAM.

Use `with_board_mut` and `devices_mut` to select the device on a specific machine. Each view holds the exclusive borrow of that mounted instance. It can reach model state while the checked placement stays fixed. These views have no storable runtime ID or detach operation. A future detachable map would need a checked instance and generation identity. The built-in mapping grants the model IRQ and time capabilities, with no DMA or machine loan. Use `.with_events(sink)` for an infallible `Emit` sink, including `Subscriptions` or an app with an infallible delivery type. The register write commits before subscribers run. A subscriber can ask the runtime to stop at that boundary.

## Connect two machines

A protocol marker names its frame type. A device owns a `Transmit<Protocol>` or `Receive<Protocol>` register, and `Link::new` connects an outgoing register to an incoming register for the same protocol. `Port::new` takes shared and exclusive projections onto the same device field. The link owns the connection and the pair owns both machines.

`ConnectedPair::run` advances the first machine, then the second, for one tick quantum at a time. Both must reach the same target time before any frame moves. First-to-second delivery precedes an optional return transfer. Each direction can move one pending frame, and a full receiver leaves it queued at the sender. Guests can consume incoming data on the next round. A terminal stop ends the round without transferring, and clock skew or incompatible rates produce an error. Choose a quantum for the delivery granularity, since this link does not model sub-quantum propagation.

### Control external inputs in virtual time

The custom runner and connection use virtual ticks for every deadline and delivery. To replay a run, start from the same machine and link state, keep the same clock rates and quantum, and apply the same host edits in the same order at the same ticks between runs. A board must consume work due at a tick once, even when it synchronizes more than once there. Devices, event sinks and observers that affect execution must use the supplied virtual time and explicit inputs. If an application uses a live source, it can record the sampled value and the virtual tick where it applies it.

Call `step` with a bounded tick budget or return a synchronous stop request at a committed device effect to regain control. The custom runner has no asynchronous stop flag, so host thread timing cannot select the guest instruction that ends a batch. The connection test applies identical inputs and compares one two-round call with two one-round calls.

The current slice supports two custom EL1 AArch64 machines with an equal tick rate. The board runner itself also accepts AArch64 CPUs with EL2. The checked map covers one RAM bank, one optional ROM bank, fixed scalar MMIO mounts and direct IRQ. Multiple programmable interrupt domains, hot-plug, a device DMA loan on the supplied map, fallible event sinks, and mixed-architecture scheduling are outside this slice. The x86 PC and AArch64 VirtMachine retain their board-specific APIs. [Safety doctrine](safety-doctrine.md) · [LLVMKit Type-Safety Doctrine](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/README.md#type-safety-doctrine)

## Performance checks

The controller benchmark warms the guest loop, rotates the case order, and prints all seven samples. RAM and MMIO use the same load, add, store, and branch loop. The MMIO comparison reaches the last slot in the three-device case. The idle sample measures a 32-tick slice with the CPU asleep and no external device. The timed IRQ sample measures one sensor deadline, the real guest handler, and the return to idle.

```sh
taskset -c "$CPU" cargo bench --locked -p rusty_box_embedded_controller --bench controller
```

On the same host and logical CPU as the baseline comparison below, two runs of the final controller benchmark produced these samples, rounded to three decimal places. The custom board has no equivalent on the starting revision, so these cases compare configurations of the new runtime.

| Workload | Unit | First run | Second run | Medians |
| --- | --- | --- | --- | ---: |
| RAM only | ns per instruction | `[13.758, 12.725, 13.127, 12.790, 12.687, 13.276, 13.398]` | `[12.640, 12.916, 13.368, 13.844, 15.035, 12.697, 12.828]` | 13.127 / 12.916 |
| RAM with three mounts | ns per instruction | `[13.835, 15.597, 15.174, 14.728, 14.433, 14.315, 14.197]` | `[14.037, 14.156, 14.578, 14.933, 14.202, 14.604, 14.428]` | 14.433 / 14.428 |
| MMIO, only mount | ns per instruction | `[24.273, 30.323, 25.755, 23.889, 24.188, 24.412, 24.339]` | `[24.957, 24.845, 24.500, 32.649, 25.366, 25.443, 24.378]` | 24.339 / 24.957 |
| MMIO, third of three mounts | ns per instruction | `[28.060, 27.344, 27.544, 28.468, 26.990, 28.389, 27.018]` | `[26.686, 29.560, 26.585, 26.344, 28.846, 26.649, 26.742]` | 27.544 / 26.686 |
| Idle without mounts | ns per 32-tick batch | `[17.939, 18.786, 20.127, 18.806, 18.072, 19.497, 18.300]` | `[18.151, 19.321, 19.312, 18.146, 20.301, 18.425, 19.832]` | 18.786 / 19.312 |
| Timed wake and guest IRQ | ns per 32-tick batch | `[2303.263, 2488.675, 2543.303, 2309.267, 2239.273, 2335.323, 2232.021]` | `[2506.938, 2221.176, 2246.858, 2371.516, 2425.133, 2401.211, 2268.963]` | 2309.267 / 2371.516 |

Three mounts raised the custom RAM median by 10–12% in these runs. The RAM path returns before device lookup, and the mounted and empty runners are separately compiled with different code sizes and link addresses. These measurements do not isolate the cost of any particular instruction. The third-device MMIO case adds address checks and measured 7–13% higher than the single mount. The timed batch includes guest handler execution.

Build `aarch64_perfbench` in each revision with `cargo build --locked --release -p rusty_box_aarch64 --features std --example aarch64_perfbench`. Alternate the resulting binaries pinned to one logical CPU with `PERFBENCH_HOOKS=plain`, `PERFBENCH_WORKLOADS=alu,memcpy,fnv,fp,mmio`, and `PERFBENCH_SCALE=1`. Compare retired instructions, stop and state hashes as well as runtime. Use the existing x86 dense, mapped and PC regression procedure with the same release flags. [x86 procedure](vnext/api-consolidation.md#performance-review-procedure)

### Existing interpreter paths

The comparison uses Rust 1.100.0, release builds, logical CPU 2 on an Intel Xeon Platinum 8573C, and baseline `9e5357c`. Both builds warm up before seven alternating rounds. Other local test and compiler jobs were paused during measurement. The AArch64 run verified matching retired counts, stop reasons, state hashes, and snapshot hashes for each guest.

The first build that always linked the new board code made the VirtMachine copy workload about 8% slower on paired medians. Restoring the original scheduling expressions recovered the GICv2 hot loop's normalized disassembly. The loop still linked 16 bytes earlier, and a double-length follow-up measured a 6.78% slowdown. Making the board feature opt-in returned the ordinary build's loop to the original relative link address and size. The focused case then measured −0.23%. These tests support layout sensitivity on this host. The measurements below cover the standalone VirtMachine build.

**VirtMachine, `std` with `custom-board` disabled, ns per retired instruction**

| Workload | Base samples | Current samples | Median paired change |
| --- | --- | --- | ---: |
| alu | `[5.904, 7.509, 6.211, 6.606, 5.955, 6.142, 5.946]` | `[6.64, 6.739, 5.956, 6.043, 6.108, 6.015, 6.428]` | -2.07% |
| memcpy | `[10.45, 11.918, 10.651, 10.545, 10.63, 10.514, 10.693]` | `[10.572, 13.751, 11.325, 10.508, 10.67, 11.104, 10.53]` | +1.17% |
| fnv | `[7.037, 7.832, 7.446, 7.125, 7.029, 7.069, 7.824]` | `[10.841, 8.527, 7.243, 7.105, 7.271, 7.083, 7.373]` | +0.20% |
| fp | `[12.343, 16.179, 12.481, 12.473, 12.356, 12.511, 12.359]` | `[12.713, 13.912, 12.461, 12.438, 13.3, 13.168, 13.321]` | +3.00% |
| mmio | `[54.585, 54.705, 51.573, 52.247, 54.201, 53.419, 56.948]` | `[53.591, 64.103, 53.821, 59.418, 52.91, 53.476, 52.687]` | +0.11% |
| memcpy, double work | `[10.797, 10.948, 10.672, 11.183, 12.115, 11.202, 13.046]` | `[10.772, 11.403, 10.803, 10.716, 11.123, 11.418, 11.678]` | -0.23% |

**x86, 80 million dispatch counts per sample, ns per count**

| Workload | Base samples | Current samples | Median paired change |
| --- | --- | --- | ---: |
| pc-alu | `[6.35, 6.25, 6.29, 6.32, 6.23, 6.29, 6.41]` | `[6.34, 6.3, 6.36, 6.29, 6.41, 6.31, 6.23]` | +0.32% |
| pc-mixed | `[8.25, 8.02, 8.02, 7.94, 8.09, 8.22, 8.02]` | `[8.08, 8.04, 8.24, 8.08, 7.97, 8.19, 8.13]` | +0.25% |
| pc-string | `[11.49, 11.56, 11.43, 13.1, 14.39, 12.22, 11.58]` | `[11.41, 14.48, 11.79, 11.75, 12.46, 11.62, 11.73]` | -0.70% |
| dense | `[10.391, 10.799, 11.063, 11.257, 10.283, 10.411, 11.285]` | `[10.427, 10.834, 10.417, 10.331, 12.0, 10.749, 10.363]` | +0.32% |
| mapped | `[11.082, 10.785, 10.192, 10.256, 10.301, 10.477, 10.672]` | `[10.318, 10.198, 10.837, 10.272, 10.381, 10.243, 10.238]` | -2.23% |

The final paired medians stayed within approximately 3% for VirtMachine and 2.3% for x86. The timing distributions vary across rounds, so these results support comparable performance on the measured configurations.

## Portability and regression checks

The controller tests cover guest IRQ assertion and acknowledgement, reset, RAM and ROM permissions, modified instruction fetch, device-triggered memory changes, and explicit deadline errors. The compile-fail fixtures cover device borrows, capability access, and link protocol and direction. The existing AArch64 machine API and x86 external-memory and hardware tests also passed in release mode.

```sh
cargo check --locked --release -p rusty_box_embedded_controller --lib --target x86_64-unknown-none
cargo check --locked --release -p rusty_box --no-default-features --target x86_64-unknown-none
cargo test --locked --release -p rusty_box_aarch64 --test api
cargo test --locked --release -p rusty_box --no-default-features --features std \
  --test external_memory --test composed_hardware
```
