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
2. Join devices with `DevicePair::new` and put the tree into `AddressSpace::new`. Both validate before mounting. If validation fails, `MappingRefused` gives you a `reason` and the original `resources`. `MappedDevice::new` also returns the original model if its range and access width are incompatible.
3. Move the checked board and `Cpu` into `Aarch64Machine::new`. Call `step` with `RunBudget::Ticks` to advance virtual time even during `WFI`. `RunBudget::Instructions` bounds retirement, so a waiting processor with recurring deadlines may keep advancing time with that budget. The runtime settles IRQs, architectural outputs, PMU bounds, and device deadlines, then runs the existing static trace engine. A guest PSCI reset request returns control to the host, which can decide whether to call `reset`.

Use `MachineBoard` to supply another board policy. Its `PhysBus` handles data accesses, instruction fetch and translation descriptors. The runtime passes timer, PMU and DCC outputs to the board at synchronization boundaries. The board chooses which physical and virtual interrupt levels to return. The supplied `AddressSpace` uses a level-OR direct IRQ for its mapped devices. A board that needs an interrupt controller can implement its own routing with `MachineBoard`.

RAM and ROM may supply stable instruction fetches. MMIO fetches and page-table reads fault, and a descriptor compare-and-swap uses the exclusive bus loan for both operations. Guest stores use the CPU's usual cache-coherence path. `with_board_mut` clears cached code, translations and the exclusive reservation before the host edits the board. A custom bus that writes memory on behalf of a device marks that effect in `BoardInputs::memory_changed` and leaves `devices_touched()` set if a guest access triggered it. That causes invalidation before the next guest instruction. A subsequent machine run always settles any host-driven IRQ or deadline changes.

## Implement and control a device

Implement `IrqMmioModel` for scalar registers. `WindowOffset` is relative to the mount, so the model works at any aligned base. `ClockedDevice::advance` receives the machine clock and a single interrupt output. Consume any work due at that time and return the next strictly future deadline. A deadline still due after synchronization produces `InvalidDeadline`. The machine can advance to the earliest scheduled deadline while the processor waits. `reset` runs at the current virtual time and the map clears each mounted IRQ. The example resets its peripheral registers and keeps its RAM.

Name slots once where you define the tree. For a layout of `(sensor, (gpio, wire))`, the host can edit models like this.

```rust
use rusty_box_aarch64::board::address_space::{First, Mounted, Second};

type TemperatureSlot = First<Mounted>;
type GpioSlot = Second<First<Mounted>>;

let clock = controller.clock();
controller.with_devices_mut(|mut devices| {
    devices.get::<TemperatureSlot>().set(90, clock);
    devices.get::<GpioSlot>().set_input(5);
});
```

`with_devices_mut` lends only mounted models. It preserves warmed code, translations and the exclusive reservation. The next run settles the resulting IRQs and deadlines before executing a guest instruction. Use `with_board_mut` when the edit needs memory or other board state. `First` and `Second` describe positions in a static tree, so two instances of the same model can have distinct names. Names select a position within the machine you borrow. Placement stays fixed and there are no detachable handles. The [15-peripheral test](../examples/embedded_controller/tests/large_board.rs) assembles three reusable banks alongside the original controller, reads two GPIO instances from guest firmware, and publishes a typed event from its IRQ handler.

The map grants device models IRQ and time capabilities. Use `.with_events(sink)` for an infallible `Emit` sink, including `Subscriptions` or an app with an infallible delivery type. The register write commits before subscribers run. A subscriber can ask the runtime to stop at that boundary.

## Connect two machines

A protocol marker names its frame type. `Protocol`, `Transmit`, `Receive` and the generic `Port` projection live in `rusty_box_core::connection`. Any caller with exclusive loans to matching registers can use `Transmit::deliver_to` at a delivery boundary it controls. The AArch64 link module re-exports the endpoint types and specializes `Port` to the checked board loan.

For checked AArch64 boards, `Link::new` connects two device-tree ports. `Port::new` takes one mutable projection through `DevicesMut`. It runs once for the sender and, when a frame is pending, once for the receiver. The same selected endpoints are used for backpressure and moving the frame. Pumping the link uses only device loans and preserves each processor's cached state. The projection adapter is specific to the checked board loan, while the endpoints and transfer rule are shared.

```rust
let link = Link::new(
    Port::new(|devices| &mut devices.select::<WireSlot>().tx),
    Port::new(|devices| &mut devices.select::<WireSlot>().rx),
);
```

`ConnectedPair::run` advances the first machine, then the second, for one tick quantum at a time. Both must reach the same target time before any frame moves. First-to-second delivery precedes an optional return transfer. Each direction can move one pending frame, and a full receiver leaves it queued at the sender. Guests can consume incoming data on the next round. A terminal stop ends the round without transferring, and clock skew or incompatible rates produce an error. Choose a quantum for the delivery granularity, since this link does not model sub-quantum propagation.

### Control external inputs in virtual time

The custom runner and connection use virtual ticks for every deadline and delivery. To replay a run, start from the same machine and link state, keep the same clock rates and quantum, and apply the same host edits in the same order at the same ticks between runs. A board must consume work due at a tick once, even when it synchronizes more than once there. Devices, event sinks and observers that affect execution must use the supplied virtual time and explicit inputs. If an application uses a live source, it can record the sampled value and the virtual tick where it applies it.

Call `step` with a bounded tick budget or return a synchronous stop request at a committed device effect to regain control. The custom runner has no asynchronous stop flag, so host thread timing cannot select the guest instruction that ends a batch. The connection test applies identical inputs and compares one two-round call with two one-round calls.

`ConnectedPair` supports two checked EL1 AArch64 `AddressSpace` machines with an equal tick rate. Custom `MachineBoard` implementations can use the shared registers under their own board-specific delivery boundary. The board runner itself also accepts AArch64 CPUs with EL2. The checked map covers one RAM bank, one optional ROM bank, fixed scalar MMIO mounts and direct IRQ. Multiple programmable interrupt domains, hot-plug, a device DMA loan on the supplied map, fallible event sinks, and mixed-architecture scheduling are outside this slice. [Safety doctrine](safety-doctrine.md) · [LLVMKit Type-Safety Doctrine](https://github.com/r3bb1t/llvmkit/blob/21b7e165b4b99e87eaa23b78fc68d12ba14957f5/README.md#type-safety-doctrine)

## Performance checks

The controller benchmark warms the guest loop, rotates the case order, and prints all seven samples. RAM and MMIO use the same load, add, store and branch loop, with four million retired instructions per sample. The link cases each measure 50,000 two-tick barriers using two running cores. The pending case delivers one frame per barrier. The idle machine sample measures a 32-tick slice with the CPU asleep and no external devices. The timed IRQ sample includes a sensor deadline, the guest IRQ handler and the return to idle.

```sh
taskset -c "$CPU" cargo bench --locked -p rusty_box_embedded_controller --bench controller
```

### Comparison with `85b281b`

The table compares release builds on logical CPU 2 of an Intel Xeon Platinum 8573C, using Rust 1.100.0. Both binaries warmed before the seven measured pairs. The order within each pair alternated and other local jobs paused during sampling. The harness verifies retired instruction counts and guest results. For the baseline build, copy the harness into a detached `85b281b` checkout and adapt only the two `Port::new` calls to the older shared and mutable projections. Run the resulting binaries directly with `taskset -c "$CPU"` and `RB_CONTROLLER_CASE` set to the zero-based case index for alternating comparisons or profiling.

```rust
// link_case in the 85b281b checkout
type Board<'a> = AddressSpace<'a, MappedDevice<Wire>>;
let wire: Link<Board<'_>, Board<'_>, Digital> = Link::new(
    Port::new(
        |board| &board.devices().device().tx,
        |board| &mut board.devices_mut().device().tx,
    ),
    Port::new(
        |board| &board.devices().device().rx,
        |board| &mut board.devices_mut().device().rx,
    ),
);
```

| Workload | Unit | Baseline samples | Current samples | Median paired change |
| --- | --- | --- | --- | ---: |
| RAM only | ns/instruction | `[14.187, 13.872, 13.990, 14.410, 13.818, 13.710, 13.876]` | `[13.602, 13.488, 13.891, 13.373, 13.873, 13.865, 15.259]` | -0.71% |
| RAM, one device mounted | ns/instruction | `[15.327, 15.294, 15.026, 15.006, 14.962, 14.660, 16.845]` | `[13.926, 13.796, 13.738, 13.118, 13.611, 13.750, 14.002]` | -9.14% |
| RAM, three devices mounted | ns/instruction | `[15.782, 13.836, 15.650, 14.812, 15.023, 15.022, 15.189]` | `[13.139, 13.617, 13.946, 13.288, 13.800, 12.999, 13.643]` | -10.29% |
| MMIO, one device | ns/instruction | `[25.081, 26.499, 27.383, 26.745, 26.892, 26.472, 25.822]` | `[27.248, 25.250, 25.440, 25.954, 25.149, 25.766, 26.070]` | -2.96% |
| MMIO, third of three devices | ns/instruction | `[28.352, 28.943, 27.616, 28.451, 27.365, 28.333, 29.116]` | `[28.806, 26.881, 26.628, 27.897, 28.613, 27.238, 27.379]` | -3.58% |
| RAM, fifteen devices mounted | ns/instruction | `[14.719, 14.369, 14.344, 13.928, 14.474, 14.302, 14.383]` | `[13.458, 14.350, 13.914, 13.353, 13.790, 13.461, 13.234]` | -4.73% |
| MMIO, first of fifteen devices | ns/instruction | `[30.589, 30.353, 30.217, 30.649, 31.615, 30.014, 32.088]` | `[29.220, 30.250, 29.095, 30.240, 30.112, 28.549, 28.929]` | -4.48% |
| MMIO, last of fifteen devices | ns/instruction | `[45.208, 44.729, 40.072, 40.195, 40.496, 40.929, 40.619]` | `[37.635, 39.010, 39.720, 38.703, 41.635, 39.051, 41.914]` | -3.71% |
| idle link | ns/barrier | `[86.742, 72.983, 81.234, 86.027, 85.256, 78.908, 73.567]` | `[76.174, 78.456, 78.073, 73.029, 74.520, 86.043, 85.559]` | -3.89% |
| pending link | ns/barrier | `[359.089, 372.573, 351.648, 340.693, 363.590, 336.503, 346.127]` | `[167.381, 177.306, 184.160, 165.688, 213.371, 172.144, 159.385]` | -51.37% |
| idle, no devices | ns/32-tick batch | `[20.017, 20.388, 18.620, 28.016, 19.946, 19.207, 18.762]` | `[19.396, 19.673, 21.590, 20.614, 27.218, 19.126, 20.500]` | -0.42% |
| timed wake + guest IRQ | ns/32-tick batch | `[2278.233, 2249.043, 2350.978, 2079.851, 2119.483, 2074.509, 2094.208]` | `[2256.809, 2259.956, 2100.709, 2487.348, 2257.505, 2183.148, 2080.766]` | +0.49% |

Before outlining MMIO, larger static trees changed the generated normal RAM path even when it never reached the device decoder. Callgrind counted 853.10, 865.22 and 884.41 million host instructions with zero, three and fifteen mounts. In the three-device build, the normal store went through an out-of-line bus method. In the fifteen-device build, the RAM-hit route through the bus read saved and restored three extra registers. The translation helpers executed the same instruction counts. The specialized run loops also differed in size and address, so the earlier 10–12% wall-time gap cannot be assigned entirely to these instructions.

Moving only the MMIO fallback out of line reduced the three- and fifteen-mount bus reads to 312 bytes each. Callgrind then counted 853.10, 860.17, 860.17 and 861.18 million instructions with zero, one, three and fifteen mounts. For fifteen mounts it simulated 1.01 million fewer conditional branches and found essentially the same misprediction count. This supports less executed work in the RAM path. Simulated counts cannot attribute the remaining timing spread to hardware caches or predictors.

The three-mount RAM median fell from 15.023 to 13.617 ns/insn. The four current RAM medians span about 3%. Last-slot MMIO still checks more ranges than first-slot MMIO, and its timings are correspondingly higher. The pending link now avoids a code and translation flush on each transfer, cutting the paired median by roughly half in this workload. The device tree still dispatches statically, with no per-instruction allocation or new synchronization mechanism.

Build `aarch64_perfbench` in each revision with `cargo build --locked --release -p rusty_box_aarch64 --features std --example aarch64_perfbench`. Alternate the resulting binaries pinned to one logical CPU with `PERFBENCH_HOOKS=plain`, `PERFBENCH_WORKLOADS=alu,memcpy,fnv,fp,mmio`, and `PERFBENCH_SCALE=1`. Compare retired instructions, stop and state hashes as well as runtime. Use the existing x86 dense, mapped and PC regression procedure with the same release flags. [x86 procedure](vnext/api-consolidation.md#performance-review-procedure)

### Standalone interpreter comparison with `85b281b`

These five alternating pairs use the same logical CPU and release compiler. VirtMachine units are ns per retired instruction. The x86 workloads use 80 million dispatch counts per sample. VirtMachine retired counts, stop reasons and guest-state and snapshot hashes matched. The x86 binaries reached the requested counts.

| Workload | Baseline samples | Current samples | Median paired change |
| --- | --- | --- | ---: |
| VirtMachine, alu | `[6.100, 6.110, 6.020, 6.050, 5.990]` | `[6.060, 7.270, 6.040, 6.490, 6.430]` | +7.27% |
| VirtMachine, memcpy | `[10.707, 10.778, 11.216, 11.287, 10.860]` | `[11.582, 11.063, 10.626, 11.256, 11.348]` | +2.64% |
| x86 PC, alu | `[6.500, 6.530, 6.710, 6.840, 6.750]` | `[6.660, 6.730, 6.750, 6.530, 6.630]` | +0.60% |
| x86 PC, mixed | `[8.830, 8.990, 8.580, 8.540, 9.140]` | `[8.730, 8.450, 9.120, 8.640, 8.460]` | -1.13% |
| x86 PC, string | `[12.460, 12.090, 12.420, 12.650, 11.890]` | `[12.420, 12.490, 12.400, 12.230, 12.390]` | -0.16% |
| x86 CPU-only, dense | `[16.535, 15.718, 15.527, 13.621, 12.457]` | `[15.184, 15.671, 16.175, 13.692, 12.378]` | -0.30% |
| x86 CPU-only, mapped | `[15.138, 12.102, 15.195, 14.972, 15.683]` | `[12.377, 13.109, 11.821, 13.932, 15.264]` | -6.95% |

The x86 PC and CPU-only benchmark binaries were byte-for-byte identical. The standalone VirtMachine binaries had identical `.text` sections, including layout. The measured AArch64 spread, including the ALU median difference, occurred with the same executable instructions.

### Existing interpreter paths from the preceding revision

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

The controller's 14 integration tests cover guest IRQ assertion and acknowledgement, reset, RAM and ROM permissions, modified instruction fetch, device-triggered memory changes, deadline errors, and recovery of refused resources. A guest load verifies that device edits keep a warmed translation, a store-exclusive verifies the reservation, and a trace-cache test checks decoded code before and after a RAM edit. The shared connection test exercises backpressure with non-`Clone` frames. Seven compile-fail fixtures cover device borrows, missing slots, capability access, and link protocol and direction. The example still prints `Controller A GPIO 95, controller B GPIO 95`.

The release AArch64 suite without `std` or `alloc` passed with the 2026-09 AARCHMRS data, including `every_system_access_takes_the_exception_arms_accessor_takes`. Its two opt-in whole-encoding-space sweeps remained ignored. Both x86 regression binaries passed, as did the controller and x86 bare-metal build checks. The full repository gate ended during the separate AArch64 `std` build, so the later gate steps and DLX boot were outside that run. Running the gate's `rusty_box_symbols` Clippy command separately reproduced its six existing diagnostics on Rust 1.100.0.

```sh
cargo check --locked --release -p rusty_box_embedded_controller --lib --target x86_64-unknown-none
cargo check --locked --release -p rusty_box --no-default-features --target x86_64-unknown-none
cargo test --locked --release -p rusty_box_aarch64 --test api
cargo test --locked --release -p rusty_box --no-default-features --features std \
  --test external_memory --test composed_hardware
```
