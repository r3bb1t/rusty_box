# Typed application events

`rusty_box::events::App` composes handlers for independently defined event types. The module is also available as `rusty_box_core::events`. Choose the state type when composing and move the value into the application with `.with_state()`.

```rust
use rusty_box::events::{App, ExecutionRequest};

struct ProcessCreated(u32);
struct RegisterChanged(u8);

struct AppState {
    processes: Vec<u32>,
    writes: u64,
}

let handlers = App::<AppState>::new()
    .on(|event: &ProcessCreated, state: &mut AppState| state.processes.push(event.0))
    .on(|_: &RegisterChanged, state: &mut AppState| state.writes += 1)
    .on(|event: &RegisterChanged| println!("New register value = {}", event.0));

let mut app = handlers.with_state(AppState { processes: Vec::new(), writes: 0 });
assert_eq!(app.dispatch(&ProcessCreated(42)), Ok(ExecutionRequest::None));
assert_eq!(app.dispatch(&RegisterChanged(3)), Ok(ExecutionRequest::None));
assert_eq!(app.state().writes, 1);
let state = app.into_state();
assert_eq!(state.processes, [42]);
```

Handlers can be ordinary functions or `FnMut` closures. Annotate the event and state arguments of closures so Rust can select the signature. `App::<State, Error>::new()` selects a shared delivery error type and `App::<State>::new()` defaults to `Infallible`. A handler can return `()`, `ExecutionRequest`, or `Result<ExecutionRequest, Error>`. An application with only event observers can use `App::<()>::new().on(observer).with_state(())`.

Each matching handler gets a fresh exclusive state borrow. State and captured values can themselves contain borrows. `state()` lends a shared reference between deliveries, and `into_state()` recovers the value and drops the handlers. The application imposes no `Clone`, `Send`, `Sync`, allocator, or async runtime bounds. Existing callers such as PC endpoint placement can impose their own bounds.

## Delivery and device integration

A delivery runs matching handlers synchronously in registration order. An unmatched event succeeds with `ExecutionRequest::None`. A stop is accumulated while handlers continue, and the publisher decides when to honor it. The first error is returned unchanged, takes precedence over any accumulated stop, and skips all remaining handlers. Earlier state changes and effects are retained. Delivery has no queue or automatic replay. Panics follow the caller's Rust panic strategy. The active app is exclusively borrowed for dispatch. A handler can synchronously publish to a separate sink that it owns or borrows.

`ReadyApp` implements `Emit` and works directly with `publish_events`. An existing `Subscriptions` route can use `.forward(app)` alongside local handlers. The ready app owns its state while the publisher keeps lending the delivery capability to the handlers wired to receive it. For example, a device endpoint can forward notifications to the app and then run a low-level handler that drives its IRQ output. Both deliveries obey the same stop and error rules. Passing the app to a producer or attachment transfers or lends it according to that producer's existing ownership interface. [Existing device contracts](device-capabilities-and-events.md) · [PC integration test](../../rusty_box/tests/composed_hardware.rs)

## Reusable composition

Accept the existing `App<S, X, H>` with `H: AppHandlers<S, X>` and return `App<S, X, impl AppHandlers<S, X>>`. An integration can take a function that borrows just the part of application state it needs. The process route takes `|state| &mut state.catalog`, so its module can be composed with a different application state type. The application can add more handlers to the returned builder and still owns the choice of state value. [Process route module](../../examples/execution_workflows/src/bin/typed_events/process.rs)

The downstream `typed_events` workflow uses event type as the route key and puts an ordered `Subscriptions` chain behind its `RunRequested` route. Like a Tower service wrapper, the middleware owns its inner sink. It implements `Emit` and forwards a synchronous call with a borrowed capability. `BudgetLimit` validates the budget before forwarding. `Audit` wraps the guard and records the outcome after calling the inner sink, including early errors. The layers and their order are composed where the route is registered. [Route assembly](../../examples/execution_workflows/src/bin/typed_events/main.rs) · [Middleware and extraction](../../examples/execution_workflows/src/bin/typed_events/run.rs)

## Engineering report

The reviewed branch head, `88357d9`, provides local, single-event `Subscriptions<E, X, H>`. Its `.on`, `.try_on`, and `.on_with` append typed handlers, and `.forward` adds another `Emit<E, C>` sink. `EventEndpoint` owns a device and its subscribers before PC placement. A model commits a write, releases its model and DMA borrows, and yields notifications. The endpoint lends its assigned IRQ to the subscribers through the existing publisher. The PC's topology determines placement, resource ownership, and execution boundaries. [Event primitives](../../rusty_box_core/src/events.rs) · [Model contracts](../../rusty_box_devices/src/composition.rs) · [PC adapter](../../rusty_box/src/emulator/hardware/event_endpoint.rs)

| Design | Inference and storage | Lifetimes, cost, and compatibility |
| --- | --- | --- |
| A. Standalone typed composition builder | Store each registration in a heterogeneous chain, select the handler shape from the signature, and move state into a separate executable value. Exact runtime type matching allows an open set of events. A closed enum avoids type erasure and requires a shared list of event types. | A safe open match using `Any` requires `'static` event types. Static storage needs no heap. The builder would need its own traversal and stop/error policy, plus an `Emit` bridge for existing producers. |
| B. Application facade over subscription delivery | Use the same state finalization and signature inference, and store adapters in the existing `Then` subscription chain. Each adapter restores a typed event reference before calling the concrete handler. | Reuses the subscription traversal and policy, and implements the existing `Emit` contract. Only the facade applies the `'static` constraint. The low-level event and capability APIs still support their existing borrowed payloads. Both layers work without `std` or `alloc`. |

Design B adds two types to represent composition and ownership. `App` has no dispatch entry point. `ReadyApp` stores `S` by value beside the handlers and dispatches via `&mut self`, so the state is reborrowed only for each invocation. The builder and ready app share the named chain types. Downstream composition helpers can hide their heterogeneous return type behind `AppHandlers`.

The two `FnMut` shapes require distinct trait parameters. On stable Rust, blanket implementations that differ only in their callable bounds conflict with E0119. Tuple markers `(E,)` and `(E, S)` distinguish the implementations, and the associated event type identifies what to match. This keeps `.on()` generic without depending on nightly specialization or on inferring types through arbitrary extractor projections.

```rust,compile_fail
trait Handler<E, S> {}
impl<E, S, F: FnMut(&E)> Handler<E, S> for F {}
impl<E, S, F: FnMut(&E, &mut S)> Handler<E, S> for F {}
```

The opted-in facade uses one borrowed `Any` view for exact type matching. Each registration may do a type check until a handler fails, so worst-case delivery cost is linear in the number of registrations. The chain and calls to the concrete handlers are statically dispatched. An empty chain returns immediately. The existing per-source paths do not enter the facade, and no dispatch hook was added to CPU execution. Use `Subscriptions` directly for fully static event selection or payload types containing short borrows. The `'static` bound on a facade event describes its type. The actual event value is still lent only for the call.

### Dependency providers

The two supported `App::on()` signatures keep selection and invocation explicit. The typed workflow shows a local analogue to Axum's `FromRequestParts`. `TargetProcess::from_parts` reads the requested selector and application catalog, caches the resolved identity or error for this delivery, and lends `&mut Process` to one handler at a time. The next delivery resolves against the current selection. This extraction belongs to the route, so unrelated events and rejected runs do no target lookups. [Scoped target extraction](../../examples/execution_workflows/src/bin/typed_events/run.rs)

The downstream image workflow demonstrates another use of publisher-scoped preparation. It shares short-lived binary metadata across observers without adding an executable-format dependency to the event core. [Linux observation example](linux-observation-libafl.md)

### Validation commands

```sh
cargo test --locked --release -p rusty_box_core --no-default-features
cargo test --locked --release -p rusty_box --no-default-features --features std
cargo build --locked --release -p rusty_box_no_alloc_smoke --target x86_64-unknown-none
cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml --bin typed_events
cargo test --locked --release --manifest-path examples/execution_workflows/Cargo.toml --bin typed_events
```

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
