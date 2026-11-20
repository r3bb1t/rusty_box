# Hardware composition API consolidation

Base commit `8432228e3b75949690731ba019fc283f6f40b2eb` on `experimental/6-x86-cpu-only-memory`.

## Construction and subscriptions

The nested topology example had to construct a typed `Subscriptions` value for each instance, wrap active models in `EventEndpoint`, and then wrap those endpoints in an IRQ window. Passive devices similarly passed their own subscription storage to `register_window`. The ordinary application only needed the model, its callbacks, and its placement.

| Approach | Example application code | Consequence |
|---|---|---|
| Existing explicit adapters | `MmioWindow::with_irq(pages, line, EventEndpoint::with_subscriptions(gpu, subscriptions))` | The application chooses the storage shape and event route explicitly |
| Selected window constructors | `MmioWindow::with_model(line, gpu).on(observe)` | The model infers the event type, and the window owns a statically dispatched route |
| A separate pre-placement endpoint builder | `endpoint(gpu).on(observe).into_window(line)` | Adds an intermediate application-facing type. Sharing that type across boards would require deciding how each board supplies callback capabilities and translates errors. |

Both simple constructors start with one page. A leaf can choose its width with `with_pages`. Register-only models supply ordinary read and write functions, and the notification type comes from the write's return value. A write can return `Option<Event>` for an occasional notification or `core::iter::empty::<()>()` for an operation with no notifications.

```rust
let bank = MmioWindow::registers(RegisterBank::new(7), RegisterBank::read, RegisterBank::write)
    .on(|event| println!("{}", event.value));
let gpu = MmioWindow::with_model(line, Gpu::default())
    .try_on(|_| Ok(Reaction::Stop));
bank.then(gpu)?.install(&mut pc, base)?;
```

The simple path needs no explicit `Subscriptions`, adapter storage types, or event type arguments. A plain observer receives the event. A fallible callback can return a stop or a diagnostic. An active leaf can also lend its selected IRQ with `on_with_irq`. The existing explicit `register_window`, `EventEndpoint::with_subscriptions`, `with_irq`, and `PcAttachment` entry points still work. The old single-device attachment and the new window methods register active handlers through the same endpoint implementation. Callers that place a root in named bare-metal storage can still spell its concrete type.

[Register-only example](../../rusty_box/examples/hardware_topology/register_window.rs) · [Mixed topology example](../../rusty_box/examples/hardware_topology/main.rs)

## Recovering a refused join

`then` now returns `MmioJoinError<L, R>`, which owns both original windows inline and carries the existing `MmioTopologyError` in `reason`. Both duplicate IRQ and size-overflow checks finish before either child is unpacked. The old `Err(MmioTopologyError::DuplicateIrq)` match becomes a match on `error.reason`. A caller can use `?` to propagate the owning error or recover the two windows and retry.

```rust
let root = match left.then(right) {
    Ok(root) => root,
    Err(failed) => failed.left.then(failed.right.with_irq_line(spare_line))?,
};
root.install(&mut pc, base)?;
```

`with_irq_line` changes the board wiring of an unmounted leaf. Its model and handler captures move intact. The guest regression confirms that both models retain their initial registers, both original callbacks receive their values, and the right callback acknowledges the rewired line while the left line stays asserted. Both models and callbacks are destroyed exactly once when the detached root is dropped.

[Recover and retry example](../../rusty_box/examples/hardware_topology/recover_topology.rs) · [Ownership regression](../../rusty_box/tests/hardware_topology.rs)

## Model and board boundary

`rusty_box_core::events` owns event delivery and ordering. `rusty_box_devices::composition` owns the shared register access and DMA and interrupt capability contracts. The model uses local offsets and ordinary Rust state. `rusty_box::emulator::hardware` owns PC windows, ISA wiring, DMA address resolution, and the interpreter's stop and error boundary. The consolidated methods build the existing adapters and keep the established route through one erased call at the PC root. Register callbacks have only their local model and access. Active device callbacks can borrow one explicitly assigned IRQ, and device code still receives separate DMA and IRQ loans according to its existing model contract.

The aperture remains a fixed, page-aligned map on the single-CPU software PC. Hardware models compile independently of that board and its placement types.

## Validation

| Release suite | Passed | Failed | Ignored |
|---|---:|---:|---:|
| x86 `std`, including integration and doc tests | 1,291 | 0 | 8 |
| AArch64 without default features | 1,178 | 0 | 8 |
| AArch64 `std` | 1,284 | 0 | 8 |
| Core and device libraries without default features | 52 | 0 | 0 |
| x86 allocation-free runtime tests | 5 | 0 | 0 |

All 15 compile-fail fixtures passed. The register-only, mixed topology, and recovery examples each ran as a workspace example and a standalone downstream executable. The GPU, mailbox, and register bank also compiled in an independent `#![no_std]` library that depends only on `rusty_box_devices`. The original composed-hardware and CPU facade examples ran successfully.

Host builds with and without `alloc`, the WebAssembly check, and the bare-metal build passed. The bare-metal smoke binary contains the borrowed topology and has no allocator symbols. DLX reached login at 383 million engine counts and opened a Linux 1.3.89 root shell.

## Performance

The release builds used the lockfile and Rust 1.100.0 with x86 `std` and default features disabled. Each profile has six samples per executable, pinned to CPU 0 of the AMD EPYC 9V74 VM with alternating order. PC timings are wall nanoseconds per engine count over 300 million counts. MMIO timings are wall nanoseconds per guest store and following branch over six million stores.

| Workload | Baseline median [min, max] | Patch median [min, max] | Median change |
|---|---:|---:|---:|
| PC mixed | 8.145 [7.70, 8.35] | 8.305 [7.81, 8.62] | +2.0% |
| PC string | 9.725 [9.44, 10.58] | 9.790 [9.34, 10.32] | +0.7% |
| PC ALU | 6.095 [5.93, 6.56] | 6.170 [6.04, 6.38] | +1.2% |
| Direct MMIO, zero observers | 35.404 [33.89, 40.87] | 34.590 [34.23, 35.73] | −2.3% |
| Direct MMIO, two observers | 34.730 [33.52, 46.25] | 35.432 [33.92, 37.50] | +2.0% |
| Three flat devices | 38.285 [36.79, 51.70] | 38.186 [36.53, 39.96] | −0.3% |
| Three nested devices | 37.885 [35.91, 44.86] | 39.278 [36.05, 39.97] | +3.7% |

All timing ranges overlap. The nested workload repeat measured 37.611 ns for the baseline and 37.179 ns for the patch, a 1.2% lower median. The initial nested difference did not reproduce.

The PC binaries have byte-identical `.text` sections at the same ELF address. The inspected direct adapter methods and the timed nested write route match after relocation normalization. The untimed nested read route remains 28 bytes in both builds, with different branch lowering. The timing comparisons cover whole guest runs and do not isolate code placement or host variance.
