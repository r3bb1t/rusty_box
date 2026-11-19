# Hardware attachment and event ergonomics

The base is `4f3bdb064505ff1c682c668a3a30f818153ca7cd` plus the session stabilization, public API, and typed hardware patches. The new patch is incremental from that combined tree.

## What changed

The old `GpuEndpoint` repeated the model call, event forwarding, and PC error policy. Its `GpuAttachment` repeated a higher-ranked sink bound and rebuilt the endpoint for every registration method. `Subscriptions` already supplied ordering, borrowing, and fallible delivery. `PcAttachment` already supplied placement and layering. The duplication sat between those APIs.

| | Previous GPU adapter | Shared API |
|---|---|---|
| Define a device | Model plus `GpuEndpoint` and `GpuAttachment` | Implement `MmioModel::read` and `write` on the model |
| Choose notifications | Two separately wired sinks and a manual dispatch match | Return a device-owned event, with an enum for a model with several outcomes |
| Register a handler | Device-specific `.on` and `.on_rejected` wrappers | `.on` for the model event or `.on_event` with a typed projection |
| Handle a subscriber failure | Log inside the adapter, change guest status, request a stop | Return `SubscriberError` to the host with the MMIO address |

```rust
PcAttachment::with_model(page, line, Gpu::default())
    .on_event(GpuEvent::written, |tile| println!("{:?}", tile.pixels))
    .on_event(GpuEvent::rejected, |failure| eprintln!("{failure:?}"))
    .install(&mut pc)?;

PcAttachment::with_model(mailbox_page, mailbox_line, Mailbox::default())
    .on(|message| println!("{}", message.0))
    .install(&mut pc)?;
```

`MmioModel::Event` is the result of the model's write operation. The GPU's result distinguishes completed and rejected transfers, while the independent mailbox returns a `MessagePosted`. Device authors can use ordinary enums and accessor functions. Each attachment owns its subscriptions, and the optional `.on_with_irq` borrows that attachment's assigned line for one callback. `EventEndpoint` supplies the reusable PC adapter. A device with a different operation shape can still implement `PcMmioDevice` directly.

## Effects and errors

The neutral `write_and_publish` helper calls the model first, then lends its notification and IRQ output to subscribers. The model's borrow ends before notification delivery, and the DMA loan stays outside the subscriber interface. Registration order and early return on the first error follow `Subscriptions`. A stop requested by one handler allows later handlers to run. A later error takes precedence over that stop. The same rules apply with or without an optional write observer.

The model and subscribers can commit DMA and IRQ changes before an error. The PC records the first callback failure, lets the interpreter reach its existing dispatch boundary, and services the scheduler before returning `CpuError::MmioSubscriber` to the host. This diagnostic carries the physical address and a static message, so the path can work without allocation. A competing CPU or scheduler error keeps precedence. Guest code can resume at its resulting IP without replaying the tested scalar store or receiving the diagnostic again. Compound x86 instructions retain their existing dispatch granularity. General event rollback and recovery from host panics remain outside this contract.

The experimental low-level `PcMmioDevice::write` now returns `Result<Reaction, SubscriberError>`. Custom endpoints can wrap their previous return values in `Ok`. The optional write observer gets that same result. The architecture-independent `Emit` primitive continues to allow caller-chosen error types.

## Placement and IRQ ownership

Owned installation checks the PC page, registry capacity, and IRQ before boxing the endpoint. Refusal returns the full typed `PcAttachment`, including its model and captured handlers. The caller can recover it with `into_parts` and try another placement. The stable borrowed endpoint route still avoids allocation. After a successful mount, detachment returns the existing erased storage so the host can move or drop it. Recovering arbitrary concrete types after detachment would require a separate type-identity contract or a mount handle with stale-handle protection.

The PC rejects the fixed PIT, keyboard, UART, RTC, ACPI SCI, mouse, and IDE lines, even when idle. Attachment also checks current PIRQ and HPET routes, asserted board lines, and other composed endpoints. The guest regression starts from the full board initializer and reads back both programmed routes before trying attachments. This check applies at installation. Guest PIRQ or HPET changes after installation still require the board owner to keep the chosen line clear. Sharing a line safely through later remaps would require per-source assertion tracking. Detachment uses the existing PC line-deassertion semantics.

## Reuse and scope

Typed delivery and ordering live in `rusty_box_core`. `DmaWrite`, `InterruptOutput`, `MmioModel`, and `write_and_publish` live in `rusty_box_devices` and compile independently of the PC. A future AArch64 board can reuse those pieces with its own DMA mapping, interrupt routing, and scheduler policy. PC placement, ISA claims, dispatch error handling, and cache synchronization stay with x86. This milestone keeps runtime `on_any` and post-mount registration deferred.

[Downstream application](../../rusty_box/examples/composed_hardware/main.rs) · [GPU model](../../rusty_box/examples/composed_hardware/gpu.rs) · [Mailbox model](../../rusty_box/examples/composed_hardware/mailbox.rs) · [Reusable PC adapter](../../rusty_box/src/emulator/hardware/event_endpoint.rs) · [Guest and isolation tests](../../rusty_box/tests/composed_hardware.rs)

## Validation

| Release suite | Passed | Failed | Ignored |
|---|---:|---:|---:|
| x86 `std`, integration and doc tests included | 1,283 | 0 | 8 |
| AArch64 without default features | 1,178 | 0 | 8 |
| AArch64 `std` | 1,284 | 0 | 8 |
| Core and device libraries without default features | 51 | 0 | 0 |
| x86 allocation-free storage and loan tests | 5 | 0 | 0 |

The x86 run includes all 15 compile-fail fixtures and ten composed hardware tests. The two models compile in a separate library with only `rusty_box_devices` as an emulator dependency. The standalone application ran both devices on x86. The guest verified typed refusal and retry, subscriber failure after successful DMA into decoded code, preserved IRQs, and execution without replay. With the built-in routing initialized, the guest programmed and read back PIRQ and HPET routes before the attachment checks rejected them.

Formatting, host checks both with and without `alloc`, the bare-metal x86 check and smoke link, and the `wasm32-unknown-unknown` check passed. The smoke binary includes the typed attachment path and has no Rust allocator symbols. The hash-checked DLX regression reached login at 383 million engine counts and opened a Linux 1.3.89 root shell.

## Performance

The previous typed-hardware milestone is the baseline. Both builds used the lockfile, Rust 1.100.0, and x86 `std` without default features. Each sample ran on CPU 0 of the AMD EPYC 9V74 VM with validation jobs idle. The drivers alternated the two binaries.

The normal PC probe ran without an attached endpoint. Four inspected dword and qword RAM helpers match after relocation normalization. The interpreter loop has the same instruction count and size, with different error-tag constants. The dispatcher and text placement also changed. The timings cover the whole executable and cannot attribute differences to the adapter. Each row contains six 300-million-count samples per binary, in wall nanoseconds per engine count.

| PC workload | Previous median [range] | Refined median [range] | Median change |
|---|---:|---:|---:|
| Mixed ALU, load and store | 8.235 [7.64–8.68] | 8.410 [7.77–8.59] | +2.1% |
| REP MOVSB and STOSB | 10.545 [9.86–11.01] | 10.395 [9.99–11.00] | −1.4% |
| ALU | 6.695 [6.39–6.99] | 6.220 [6.08–6.64] | −7.1% |

The attached probe executes a warmed MMIO store and backward branch through both the previous handwritten counter endpoint and the new model adapter. It verifies the detached counter before printing each result. The two-handler profile delivers the same typed event to two observers. The measured interval excludes mounting and has no IRQ or DMA activity.

| Attached workload | Previous median [range] | Refined median [range] | Median change |
|---|---:|---:|---:|
| Zero subscribers | 37.02 [34.16–60.93] | 36.70 [34.69–40.28] | −0.9% |
| Two subscribers | 36.33 [35.36–38.60] | 38.40 [35.51–43.87] | +5.7% |

Each attached row compares six samples of six million writes per binary, in wall nanoseconds per store and branch. The two-subscriber median increased by 2.06 ns. All paired ranges overlap. These runs leave the contribution of code placement, host variability, and result-handling overhead unresolved.
