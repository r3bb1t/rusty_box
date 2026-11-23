use core::any::Any;
use core::convert::Infallible;
use core::marker::PhantomData;

use super::delivery::Deliver;
use super::{Emit, ExecutionRequest, Then};

mod erased {
    use core::any::Any;

    // Application events erase only a borrowed payload for exact type matching.
    // The handler chain and each handler remain statically dispatched.
    pub struct EventRef<'a>(pub(super) &'a dyn Any);
}
use erased::EventRef;

/// A type selector for [`App::route`]. Create one with [`event`].
pub struct Event<E>(PhantomData<fn() -> E>);

/// Select events of type `E` when registering a handler.
pub const fn event<E>() -> Event<E> {
    Event(PhantomData)
}

/// Typed event handlers awaiting application state.
///
/// Register handlers with [`route`](Self::route) and supply the state once with
/// [`with_state`](Self::with_state). Use `App::<()>` for stateless delivery.
/// Handler storage is static and allocation-free. Event types must be
/// `'static` for exact type matching. Their values are borrowed during
/// delivery. Use [`Subscriptions`](super::Subscriptions) for event types with
/// borrowed fields or handlers that need publisher-provided capabilities.
///
/// ```
/// use rusty_box_core::events::{App, ExecutionRequest, event};
///
/// struct RegisterChanged(u8);
/// struct DeviceRegistered;
/// struct State(u8);
///
/// let handlers = App::<State>::new()
///     .route(
///         event::<RegisterChanged>(),
///         |event: &RegisterChanged, state: &mut State| {
///             state.0 = event.0;
///         },
///     )
///     .route(event::<DeviceRegistered>(), |_: &DeviceRegistered| {});
/// let mut app = handlers.with_state(State(0));
/// assert_eq!(
///     app.dispatch(&RegisterChanged(42)),
///     Ok(ExecutionRequest::None)
/// );
/// assert_eq!(app.into_state().0, 42);
/// ```
pub struct App<S, X = Infallible, H = ()> {
    handlers: H,
    signature: PhantomData<fn(&mut S) -> X>,
}

impl<S, X> App<S, X> {
    /// Start composing handlers. `X` is the application's delivery error type.
    pub const fn new() -> Self {
        Self {
            handlers: (),
            signature: PhantomData,
        }
    }
}

impl<S, X> Default for App<S, X> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S, X, H: AppHandlers<S, X>> App<S, X, H> {
    /// Append a handler for the selected event. Its first argument must be
    /// `&E`. It can also take `&mut S` or `Resolved<'_, D>`, and it can be a
    /// `FnMut` closure. A dependency resolves only for a matching event.
    /// A handler can return `()`, [`ExecutionRequest`], or
    /// `Result<ExecutionRequest, X>`. Each invocation borrows state afresh.
    ///
    /// ```compile_fail,E0631
    /// use rusty_box_core::events::{App, event};
    ///
    /// struct Started;
    /// struct Finished;
    /// fn report(_: &Finished) {}
    ///
    /// let _app = App::<()>::new().route(event::<Started>(), report);
    /// ```
    pub fn route<E, F, Args>(
        self,
        _: Event<E>,
        handler: F,
    ) -> App<S, X, Then<H, Registered<F, Args>>>
    where
        F: AppHandler<S, X, Args, Event = E>,
    {
        self.on(handler)
    }

    /// Append a handler, inferring the event type from its first argument.
    /// See [`route`](Self::route) for the supported handler signatures.
    pub fn on<F: AppHandler<S, X, Args>, Args>(
        self,
        handler: F,
    ) -> App<S, X, Then<H, Registered<F, Args>>> {
        App {
            handlers: Then(self.handlers, Registered(handler, PhantomData)),
            signature: PhantomData,
        }
    }

    /// Finish composition and transfer ownership of the state into the app.
    pub fn with_state(self, state: S) -> ReadyApp<S, X, H> {
        ReadyApp { app: self, state }
    }
}

/// A composed application that owns its state and can receive events.
///
/// Matching handlers run synchronously in registration order. A stop is
/// collected while delivery continues. The first error takes precedence over
/// a stop and skips later handlers. Earlier effects stay committed. Delivery
/// neither queues nor retries an event. The active app is exclusively borrowed
/// through delivery. A handler can publish synchronously to a separate sink.
pub struct ReadyApp<S, X = Infallible, H = ()> {
    app: App<S, X, H>,
    state: S,
}

impl<S, X, H> ReadyApp<S, X, H> {
    /// Inspect application state between deliveries.
    pub fn state(&self) -> &S {
        &self.state
    }

    /// Recover the owned application state, dropping the handlers.
    pub fn into_state(self) -> S {
        self.state
    }
}

impl<S, X, H: AppHandlers<S, X>> ReadyApp<S, X, H> {
    /// Lend one event to the app. An event with no matching handlers returns
    /// `Ok(ExecutionRequest::None)`.
    #[inline]
    pub fn dispatch<E: Any>(&mut self, event: &E) -> Result<ExecutionRequest, X> {
        self.app.handlers.deliver(&EventRef(event), &mut self.state)
    }
}

/// Application handlers use owned state. The publisher's delivery capability
/// remains available to any low-level subscribers wired alongside the app.
impl<E: Any, C: ?Sized, S, X, H: AppHandlers<S, X>> Emit<E, C> for ReadyApp<S, X, H> {
    type Error = X;

    #[inline]
    fn emit(&mut self, event: &E, _: &mut C) -> Result<ExecutionRequest, X> {
        self.dispatch(event)
    }
}

/// The statically composed handler storage of an [`App`].
///
/// Use `App<S, X, impl AppHandlers<S, X>>` as the return type of a function
/// that adds handlers before the caller supplies state. This trait is sealed.
pub trait AppHandlers<S, X = Infallible>: for<'a> Deliver<EventRef<'a>, S, X> {}

impl<S, X, H: for<'a> Deliver<EventRef<'a>, S, X>> AppHandlers<S, X> for H {}

/// Provide a borrowed dependency for handlers of `E` that ask for
/// [`Resolved<'_, D>`](Resolved). Implement this on application state.
///
/// Each matching handler invokes the resolver separately. A provider can
/// share work across handlers using a cache scoped to the event. A resolver
/// error is converted to the application's error and stops delivery before
/// the handler runs.
pub trait Resolve<E, D: ?Sized> {
    type Error;

    fn resolve(&mut self, event: &E) -> Result<&D, Self::Error>;
}

/// A dependency borrowed for one handler invocation.
///
/// A handler cannot keep that borrow for a later delivery.
///
/// ```compile_fail,E0521
/// use core::convert::Infallible;
/// use rusty_box_core::events::{App, Resolve, Resolved};
///
/// struct Tick;
/// struct State(u8);
/// impl Resolve<Tick, u8> for State {
///     type Error = Infallible;
///
///     fn resolve(&mut self, _: &Tick) -> Result<&u8, Infallible> {
///         Ok(&self.0)
///     }
/// }
///
/// let mut retained = None;
/// let mut app = App::<State>::new()
///     .on(|_: &Tick, Resolved(value): Resolved<'_, u8>| retained = Some(value))
///     .with_state(State(0));
/// let _delivery = app.dispatch(&Tick);
/// ```
pub struct Resolved<'a, D: ?Sized>(pub &'a D);

#[doc(hidden)]
pub struct Registered<F, Args>(F, PhantomData<fn() -> Args>);

impl<S, X, F: AppHandler<S, X, Args>, Args> Deliver<EventRef<'_>, S, X> for Registered<F, Args> {
    #[inline]
    fn deliver(&mut self, event: &EventRef<'_>, state: &mut S) -> Result<ExecutionRequest, X> {
        match event.0.downcast_ref() {
            Some(event) => self.0.call(event, state),
            None => Ok(ExecutionRequest::None),
        }
    }
}

// Distinct tuple parameters keep the two blanket implementations disjoint.
#[doc(hidden)]
pub trait AppHandler<S, X, Args> {
    type Event: 'static;

    fn call(&mut self, event: &Self::Event, state: &mut S) -> Result<ExecutionRequest, X>;
}

impl<E, S, X, F, R> AppHandler<S, X, (E,)> for F
where
    E: 'static,
    F: FnMut(&E) -> R,
    R: HandlerResult<X>,
{
    type Event = E;

    fn call(&mut self, event: &E, _: &mut S) -> Result<ExecutionRequest, X> {
        self(event).into_result()
    }
}

impl<E, S, X, F, R> AppHandler<S, X, (E, S)> for F
where
    E: 'static,
    F: FnMut(&E, &mut S) -> R,
    R: HandlerResult<X>,
{
    type Event = E;

    fn call(&mut self, event: &E, state: &mut S) -> Result<ExecutionRequest, X> {
        self(event, state).into_result()
    }
}

impl<E, S, X, D, F, R> AppHandler<S, X, (E, S, fn(&D))> for F
where
    E: 'static,
    D: ?Sized,
    S: Resolve<E, D>,
    X: From<S::Error>,
    F: FnMut(&E, Resolved<'_, D>) -> R,
    R: HandlerResult<X>,
{
    type Event = E;

    fn call(&mut self, event: &E, state: &mut S) -> Result<ExecutionRequest, X> {
        let dependency = state.resolve(event)?;
        self(event, Resolved(dependency)).into_result()
    }
}

trait HandlerResult<X> {
    fn into_result(self) -> Result<ExecutionRequest, X>;
}

impl<X> HandlerResult<X> for () {
    fn into_result(self) -> Result<ExecutionRequest, X> {
        Ok(ExecutionRequest::None)
    }
}

impl<X> HandlerResult<X> for ExecutionRequest {
    fn into_result(self) -> Result<ExecutionRequest, X> {
        Ok(self)
    }
}

impl<X> HandlerResult<X> for Result<ExecutionRequest, X> {
    fn into_result(self) -> Result<ExecutionRequest, X> {
        self
    }
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use super::*;

    struct Tick(u8);
    struct Reset;

    #[test]
    fn dependency_is_borrowed_only_for_matching_handlers() {
        struct State<'a> {
            label: &'a str,
            lookups: u8,
        }

        impl Resolve<Tick, str> for State<'_> {
            type Error = &'static str;

            fn resolve(&mut self, event: &Tick) -> Result<&str, Self::Error> {
                self.lookups += 1;
                if event.0 == 0 {
                    return Err("missing label");
                }
                Ok(self.label)
            }
        }

        let bytes = *b"guest";
        let seen = Cell::new(0);
        let mut app = App::<State<'_>, &str>::new()
            .on(|_: &Tick, Resolved(label): Resolved<'_, str>| seen.set(label.len()))
            .on(|_: &Tick, state: &mut State<'_>| state.lookups += 10)
            .with_state(State {
                label: core::str::from_utf8(&bytes).unwrap(),
                lookups: 0,
            });

        assert_eq!(app.dispatch(&Reset), Ok(ExecutionRequest::None));
        assert_eq!(app.state().lookups, 0);
        assert_eq!(app.dispatch(&Tick(1)), Ok(ExecutionRequest::None));
        assert_eq!(seen.get(), 5);
        assert_eq!(app.state().lookups, 11);
        assert_eq!(app.dispatch(&Tick(0)), Err("missing label"));
        assert_eq!(app.state().lookups, 12);
    }

    #[test]
    fn matching_handlers_share_local_state_in_registration_order() {
        struct State<'a> {
            value: u8,
            output: &'a Cell<u8>,
        }

        let mut seen = 0;
        let handlers = App::<State<'_>>::new()
            .on(|event: &Tick, state: &mut State<'_>| {
                state.value += event.0;
                ExecutionRequest::Stop
            })
            .on(|_: &Reset, state: &mut State<'_>| state.value = 0)
            .on(|event: &Tick| seen += event.0)
            .on(|_: &Tick, state: &mut State<'_>| state.output.set(state.value * 2));

        let output = Cell::new(0);
        let mut app = handlers.with_state(State {
            value: 1,
            output: &output,
        });
        assert_eq!(app.dispatch(&Tick(3)), Ok(ExecutionRequest::Stop));
        assert_eq!(app.state().value, 4);
        assert_eq!(output.get(), 8);
        assert_eq!(app.dispatch(&3_u8), Ok(ExecutionRequest::None));
        assert_eq!(app.dispatch(&Reset), Ok(ExecutionRequest::None));
        assert_eq!(app.into_state().value, 0);
        assert_eq!(seen, 3);
    }

    #[test]
    fn later_error_wins_over_stop_and_short_circuits_without_retry() {
        let mut app = App::<u8, &str>::new()
            .on(|_: &Tick, state: &mut u8| {
                *state += 1;
                Ok(ExecutionRequest::Stop)
            })
            .on(|_: &Tick, state: &mut u8| {
                *state *= 10;
                Err("observer refused")
            })
            .on(|_: &Tick, state: &mut u8| *state += 100)
            .with_state(0);

        assert_eq!(app.dispatch(&Tick(1)), Err("observer refused"));
        assert_eq!(app.into_state(), 10);
    }

    #[test]
    fn independent_app_can_receive_nested_delivery() {
        let mut inner = App::<u8>::new()
            .on(|tick: &Tick, state: &mut u8| {
                *state += tick.0;
                ExecutionRequest::Stop
            })
            .with_state(0);
        let mut outer = App::<()>::new()
            .on(|_: &Reset| inner.dispatch(&Tick(7)))
            .with_state(());

        assert_eq!(outer.dispatch(&Reset), Ok(ExecutionRequest::Stop));
        drop(outer);
        assert_eq!(inner.into_state(), 7);
    }
}
