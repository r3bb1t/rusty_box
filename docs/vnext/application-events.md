# Typed application events

`rusty_box::events::App` composes handlers for independently defined event types. The module is also available as `rusty_box_core::events`. Choose the state type when composing and move the value into the application with `.with_state()`.

Start with the short `typed_events` example for a single handler and its state. [Beginner walkthrough](../../rusty_box/examples/README.md#typed-events)

```rust
use rusty_box::events::App;

struct ProcessCreated(u32);
struct RegisterChanged(u8);

struct AppState {
    processes: Vec<u32>,
    writes: u64,
}

fn record_process(&ProcessCreated(pid): &ProcessCreated, state: &mut AppState) {
    state.processes.push(pid);
}

fn record_write(&RegisterChanged(value): &RegisterChanged, state: &mut AppState) {
    state.writes += 1;
    println!("New register value = {value}");
}

let mut app = App::new()
    .route(record_process)
    .route(record_write)
    .with_state(AppState { processes: Vec::new(), writes: 0 });
println!("Process created: {:?}", app.dispatch(&ProcessCreated(42)));
println!("Register changed: {:?}", app.dispatch(&RegisterChanged(3)));
let state = app.into_state();
println!("Processes: {:?}, writes: {}", state.processes, state.writes);
```

`route(handler)` takes the event type from the handler's first argument. Handlers can be ordinary functions or `FnMut` closures. Annotate closure arguments to select the signature.

Use `App::new()` for delivery with no errors and `App::fallible()` when a handler or extractor can return an error. The app infers its state type from `.with_state(...)`. Use `.with_state(())` for stateless delivery. A handler can return `()`, `ExecutionRequest`, or `Result<ExecutionRequest, Error>`.

Each handler that takes `&mut State` gets a fresh exclusive borrow. State and captured values can themselves contain borrows. `state()` lends a shared reference between deliveries, and `into_state()` recovers the value and drops the handlers. The application imposes no `Clone`, `Send`, `Sync`, allocator, or async runtime bounds. Existing callers such as PC endpoint placement can impose their own bounds.

## Extractors

An extractor supplies a typed value on demand. Implement `FromEvent<Event, State>` on the extractor type. The handler's signature tells `.route()` which event and extractor to use.

| Handler arguments | What the app lends |
| --- | --- |
| `(&Event)` | The event |
| `(&Event, &mut State)` | The event and exclusive state |
| `(&Event, &Extractor)` | A result from `FromEvent<Event, State>`, which may borrow from either input |
| `(&Event, &Extractor, &mut State)` | A result from `FromEvent<Event>` and exclusive state |

The last form gives the extractor a separate unit value, even when the app has state. The extractor may lend data from the event, so the handler can safely mutate application state. An extractor that borrows application state must run in its own handler. Register a separate state handler for later mutations, or return a single borrowed view when a handler needs several related values. The app supports one extractor argument per handler.

The app matches the event type, then calls `from_event` for each matching handler that asks for the extractor. An extractor can borrow from the event or state and can prepare a value in an event-local cache so several handlers share the work. The borrow lasts for that invocation. If extraction fails, the error converts into the application error and delivery stops before calling the handler.

In the Linux COM1 example, `LoadedProcess` implements `FromEvent<ExecReported>`. The handler reads the parsed process and updates a separate app-owned counter. Run with `--events-only` to see delivery without extraction. The host-side example lends a paused guest as state so two handlers can share a single executable mapping walk. [COM1 workflow](../../rusty_box/examples/typed_event_workflow.rs) · [Host-side workflow](../../examples/execution_workflows/src/bin/linux_host_observe.rs) · [Extractor trait](../../rusty_box_core/src/events/app.rs)

The downstream image workflow shares short-lived binary metadata across observers using a pause in execution to prepare it. [Linux observation example](linux-observation-libafl.md)

## Delivery and device integration

A delivery runs matching handlers synchronously in registration order. An unmatched event succeeds with `ExecutionRequest::None`. A stop is accumulated while handlers continue, and the publisher decides when to honor it. The first error is returned unchanged, takes precedence over any accumulated stop, and skips all remaining handlers. Earlier state changes and effects are retained. Delivery has no queue or automatic replay. Panics follow the caller's Rust panic strategy. The active app is exclusively borrowed for dispatch. A handler can synchronously publish to a separate sink that it owns or borrows.

`ReadyApp` implements `Emit` and works directly with `publish_events`. An existing `Subscriptions` route can use `.forward(app)` alongside local handlers. The ready app owns its state while the publisher keeps lending the delivery capability to the handlers wired to receive it. For example, a device endpoint can forward notifications to the app and then run a low-level handler that drives its IRQ output. Both deliveries obey the same stop and error rules. Passing the app to a producer or attachment transfers or lends it according to that producer's existing ownership interface. [Existing device contracts](device-capabilities-and-events.md) · [PC integration test](../../rusty_box/tests/composed_hardware.rs)

`Subscriptions::on_with()` registers a low-level handler that takes the event plus a capability lent by its publisher, such as an interrupt line. Use `App::route()` for handlers that use the app's own state.

## Reusable composition

Accept the existing `App<S, X, H>` with `H: AppHandlers<S, X>` and return `App<S, X, impl AppHandlers<S, X>>`. An integration that works with several application state types can take a function that borrows just the fields it needs. The caller can add more handlers to the returned builder before supplying state.

The `typed_event_workflow` example boots Debian Linux and installs the app as the sink of an x86 I/O hook. The hook emits exec reports while the guest supervisor writes them to COM1. [Live Linux example](../../rusty_box/examples/typed_event_workflow.rs)

## Engineering report

The reviewed branch head, `88357d9`, provides local, single-event `Subscriptions<E, X, H>`. Its `.on`, `.try_on`, and `.on_with` append typed handlers, and `.forward` adds another `Emit<E, C>` sink. `EventEndpoint` owns a device and its subscribers before PC placement. A model commits a write, releases its model and DMA borrows, and yields notifications. The endpoint lends its assigned IRQ to the subscribers through the existing publisher. The PC's topology determines placement, resource ownership, and execution boundaries. [Event primitives](../../rusty_box_core/src/events.rs) · [Model contracts](../../rusty_box_devices/src/composition.rs) · [PC adapter](../../rusty_box/src/emulator/hardware/event_endpoint.rs)

| Design | Inference and storage | Lifetimes, cost, and compatibility |
| --- | --- | --- |
| A. Standalone typed composition builder | Store each registration in a heterogeneous chain, select the handler shape from the signature, and move state into a separate executable value. Exact runtime type matching allows an open set of events. A closed enum avoids type erasure and requires a shared list of event types. | A safe open match using `Any` requires `'static` event types. Static storage needs no heap. The builder would need its own traversal and stop/error policy, plus an `Emit` bridge for existing producers. |
| B. Application facade over subscription delivery | Use the same state finalization and signature inference, and store adapters in the existing `Then` subscription chain. Each adapter restores a typed event reference before calling the concrete handler. | Reuses the subscription traversal and policy, and implements the existing `Emit` contract. Only the facade applies the `'static` constraint. The low-level event and capability APIs still support their existing borrowed payloads. Both layers work without `std` or `alloc`. |

Design B adds two types to represent composition and ownership. `App` has no dispatch entry point. `ReadyApp` stores `S` by value beside the handlers and dispatches via `&mut self`, so the state is reborrowed only for each invocation. The builder and ready app share the named chain types. Downstream composition helpers can hide their heterogeneous return type behind `AppHandlers`.

The `FnMut` shapes require distinct trait parameters. On stable Rust, blanket implementations that differ only in their callable bounds conflict with E0119. Internal marker tuples distinguish the implementations. The first handler argument identifies the event, and a second shared reference identifies the extractor.

```rust,compile_fail
trait Handler<E, S> {}
impl<E, S, F: FnMut(&E)> Handler<E, S> for F {}
impl<E, S, F: FnMut(&E, &mut S)> Handler<E, S> for F {}
```

The opted-in facade uses one borrowed `Any` view for exact type matching. Each registration may do a type check until a handler fails, so worst-case delivery cost is linear in the number of registrations. The chain and calls to the concrete handlers are statically dispatched. An empty chain returns immediately. The existing per-source paths do not enter the facade, and no dispatch hook was added to CPU execution. Use `Subscriptions` directly for fully static event selection or payload types containing short borrows. The `'static` bound on a facade event describes its type. The actual event value is still lent only for the call.

### Validation commands

```sh
cargo test --locked --release -p rusty_box_core --no-default-features
cargo test --locked --release -p rusty_box --no-default-features --features std
cargo build --locked --release -p rusty_box_no_alloc_smoke --target x86_64-unknown-none
cargo run --locked --release -p rusty_box --no-default-features --example typed_events
cargo test --locked --release -p rusty_box --no-default-features --features std --example typed_event_workflow
```

For the live boot, use the Debian kernel and supervisor initramfs. [Run the Linux example](../../rusty_box/examples/README.md#extract-process-metadata-while-linux-runs)

| Release suite | Passed | Ignored |
| --- | ---: | ---: |
| x86 with `std` and default features disabled | 1,301 | 8 |
| x86 default-feature unit tests | 1,110 | 3 |
| x86 `no_std_memory` with default features disabled | 2 | 0 |
| Core and devices with default features disabled | 56 | 0 |
| AArch64 with default features disabled | 1,180 | 8 |
| AArch64 with `std` | 1,286 | 8 |

All 18 compile-fail fixtures passed, as did the standalone typed events example and the downstream execution-workflow tests with LibAFL enabled. The AArch64 runs used `--skip-aarchmrs` because the external specification data was unavailable.

The compile-fail suite checks that a builder cannot dispatch, a callback cannot retain a state reference, and a state loan prevents another delivery. The guest integration checks that after an application handler requests stop, a subsequent device subscriber can still drive the IRQ line visible to the guest. Temporarily making the shared chain short-circuit on stop caused both the order and error precedence tests to fail.

The x86 host checks passed with default features disabled, with and without `alloc`. The bare-metal and UEFI targets passed without default features, and `wasm32-unknown-unknown` passed with `alloc`. The UEFI example linked, and the headless DLX boot reached its login prompt at 383 million instructions.

The repository gate completed all 38 steps with `--skip-aarchmrs`. The rerun temporarily allowed the pre-existing `chunks_exact_to_as_chunks` Clippy lint for `rusty_box_symbols` only. The AArch64 instruction-count step skipped measurement because Valgrind was unavailable.

The x86 bare-metal consumer linked without Rust allocator symbols. With Rust 1.100.0 in release mode, the compiler reduced its `publish_app` function to a register move and return. This verifies that the type checks fold away for that concrete two-handler call site. [Compiler fixtures](../../rusty_box/tests/compile_fail) · [Bare-metal consumer](../../examples/no_alloc_smoke/src/main.rs)
