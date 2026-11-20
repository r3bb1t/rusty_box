# Device capabilities and event batches

Base commit `ddb5036da48c560f3b3c75a72ddabe0f736d0fbf` on `experimental/6-x86-cpu-only-memory`.

## Capability choice

`MmioModel` gave every model a DMA writer and an interrupt output. That signature is useful for the GPU, which needs both, but the mailbox had to import `DmaWrite` and discard the argument. The board already acquires its capabilities separately, and `EventEndpoint` already centralizes event routing. The extra parameter belonged to the adapter rather than the mailbox.

| Approach | Device author | Application author |
|---|---|---|
| Existing explicit interface | Implement `MmioModel` and accept both DMA and IRQ | Install with `PcAttachment::with_model` |
| Selected narrow interface | Implement `IrqMmioModel` when the model only drives an interrupt. Use `MmioModel` when the model needs DMA and IRQ. | Use the same constructor and subscriptions |

The blanket adapter is compiled with the model. It reuses the existing endpoint and erases the model at the same optional PC registry boundary. A model chooses one of the two interfaces. If a caller imports both traits and invokes an IRQ-only model directly, it can select `IrqMmioModel::write` or `IrqMmioModel::read` explicitly. The example only imports the narrow interface.

Automatic argument selection would require a way to identify every supported signature and borrow its resources. These examples need one narrow adapter and an ordinary function for the IRQ subscriber. A plain register device can already use the existing `MmioDevice` attachment, whose read and write methods take neither resource. That path has no automatic notifications.

```rust
// Mailbox implementation
fn write(&mut self, access: MmioWrite, irq: &mut impl InterruptOutput) -> Self::Events

// GPU implementation
fn write(
    &mut self,
    access: MmioWrite,
    dma: &mut impl DmaWrite,
    irq: &mut impl InterruptOutput,
) -> Self::Events
```

The application still wires the resources at placement. Simple handlers receive only the event. A handler that acknowledges an interrupt can use a normal function taking `&mut impl InterruptOutput`, with a closure at the existing `.on_with_irq` boundary.

```rust
PcAttachment::with_model(page, line, Mailbox::default())
    .on(|event| println!("{event:?}"))
    .on_event(MailboxEvent::posted, |message| println!("{}", message.0))
    .install(&mut pc)?;
```

[Downstream application](../../rusty_box/examples/composed_hardware/main.rs) · [Mailbox model](../../rusty_box/examples/composed_hardware/mailbox.rs) · [GPU model](../../rusty_box/examples/composed_hardware/gpu.rs)

## Event ownership and boundaries

Model writes now return an owned `IntoIterator` of notifications. A model that previously supplied `type Event = E` and returned `Option<Self::Event>` can supply `type Events = Option<E>` and return `Self::Events`. The mailbox returns two event values with distinct typed payloads from one write using an optional fixed array. Its enum belongs to the device module, and `.on_event` supports typed selection while `.on` observes the source's whole stream.

`publish_events` takes an owned sequence, an existing `Emit` sink, and the capability offered to handlers. An independently owned model can return notifications from an ordinary host method and publish them to the same local subscriptions. The example uses this for `Mailbox::take`. The event sink can also be used directly for any event type. Mounted endpoints still deliver their MMIO notifications through the board adapter. Host-owned and independently scheduled producers manage their own delivery boundary.

The model commits effects before returning its sequence. Subscribers borrow one notification and the passed capability at a time, after the model's operation has released its loans. Events run in iterator order, with subscribers running in registration order for each event. A stop is accumulated through the batch, then handled by the caller at its normal execution boundary. A subscriber error takes precedence over a stop, returns immediately, and drops remaining notifications. Earlier device and subscriber effects stay committed. The static path needs no heap queue.

## PC IRQ routing

Attachment still rejects built-in, programmed, or asserted IRQ claims and IRQs held by another composed endpoint. If the guest routes a board interrupt onto a reserved line later, the PC fabric tracks the built-in board and the composed endpoint's assertions separately. The PIC and I/O APIC see the combined wire level. Lowering either output or detaching the endpoint preserves a high assertion from the other output. A wired shared line can merge assertions, so independent edge delivery still requires distinct routes.

The guest HPET regression mounts first, then programs and reads back the route and lowers the HPET output while the endpoint holds the line high. A fabric test checks the reverse direction and detachment at both controllers. Board reset clears the two assertion masks while retaining the placements. Save and restore return `Unsupported` before touching the stream while composed endpoints are mounted, because the snapshot format has no endpoint state or separate IRQ assertions. Detach the endpoints before saving or restoring.

The two-input change preserves the board's existing arbitration between built-in sources. MMIO placement remains subject to the board's existing address-window constraints. `Emit` and `publish_events` are in `rusty_box_core`. Register models and their capability traits are in `rusty_box_devices`. The PC alone chooses how to map RAM, place pages, route ISA interrupts, and surface execution stops. AArch64 has no source changes.

[Typed event implementation](../../rusty_box_core/src/events.rs) · [Device contracts](../../rusty_box_devices/src/composition.rs) · [Integration tests](../../rusty_box/tests/composed_hardware.rs)

## Validation

| Release suite | Passed | Failed | Ignored |
|---|---:|---:|---:|
| x86 `std`, including integration and doc tests | 1,286 | 0 | 8 |
| AArch64 without default features | 1,178 | 0 | 8 |
| AArch64 `std` | 1,284 | 0 | 8 |
| Core and device libraries without default features | 52 | 0 | 0 |
| x86 allocation-free runtime tests | 5 | 0 | 0 |

All 15 x86 compile-fail fixtures passed. The standalone downstream application ran real x86 with both devices. The GPU committed DMA data before its callback requested a stop, and the guest read the data after resuming. The mailbox emitted both payloads in order from one write and reported a host-side take on the same type of local route. The separate model library compiled with `#![no_std]` using only `rusty_box_devices`. The CPU facade example ran with both owned and caller-supplied RAM.

Formatting, host checks with and without `alloc`, bare-metal x86, and `wasm32-unknown-unknown` checks passed. The bare-metal smoke binary linked the typed IRQ-only attachment without Rust allocator symbols. The hash-checked DLX disks and firmware booted to login at 383 million engine counts and opened a Linux 1.3.89 root shell.

## Performance

Both revisions used the lockfile and Rust 1.100.0 with x86 `std` and no default features. The benchmark drivers pinned every process to CPU 0 of an AMD EPYC 9V74 VM, alternated executable order, and took six samples per executable. The PC rows are wall nanoseconds per engine count over 300 million counts per sample with no mounted endpoint. The separate MMIO rows are wall nanoseconds per store and following branch over six million stores per sample. The MMIO model produces one notification per write in each revision.

| Workload | Baseline median [min, max] | Patch median [min, max] | Median change |
|---|---:|---:|---:|
| PC mixed | 8.070 [7.65, 8.34] | 8.635 [7.85, 8.96] | +7.0% |
| PC string | 10.370 [10.00, 11.32] | 10.390 [10.09, 10.43] | +0.2% |
| PC ALU | 6.290 [6.12, 6.68] | 6.290 [6.06, 6.54] | 0.0% |
| MMIO, zero subscribers | 34.940 [32.53, 40.09] | 33.855 [32.68, 37.93] | −3.1% |
| MMIO, two subscribers | 34.564 [32.98, 51.30] | 33.646 [33.32, 34.39] | −2.7% |

The two MMIO write adapters produce identical disassembly for each single-event profile. That includes the zero-subscriber adapter, which updates its counter and returns directly. The end-to-end timing ranges overlap. The x86 dispatcher, interpreter loop, and inspected dword and qword virtual memory helpers also match after relocation normalization. Their addresses all shifted by 80 bytes with the `.text` section. These checks establish code placement changed and constrain where an overhead could arise.

The follow-up mixed test interleaved the original pair with a relink of the patch. The relink put all six inspected routines exactly one page after the baseline addresses, restoring both the cache-line and page offsets. The baseline measured 8.090 ns/count [7.86, 9.02], the original patch 7.975 [7.62, 8.25], and the relink 7.935 [7.89, 8.64]. The original patch's first-run slowdown did not reproduce, and the relink and original patch were similar in the follow-up. The whole binaries differ in placement and the VM samples vary across runs. These measurements support no causal assignment to text alignment or the event adapter.
