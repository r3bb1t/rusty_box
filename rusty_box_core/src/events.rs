//! Local, typed event publication and ordered subscriptions.
//!
//! [`App`] composes handlers across event types and owns state after
//! finalization. [`Subscriptions`] builds a per-source route for publishers
//! that lend an event and its delivery capability through [`Emit`].

use core::convert::Infallible;
use core::marker::PhantomData;

mod app;
pub use app::{App, AppHandlers, Event, ReadyApp, Registered, Resolve, Resolved, event};

/// A subscriber's request to the execution driver after successful delivery.
/// Requests never interrupt delivery to the remaining subscribers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[must_use]
pub enum ExecutionRequest {
    /// Delivery made no request of the execution driver.
    #[default]
    None,
    /// Stop execution at the publisher's next supported boundary.
    Stop,
}

impl ExecutionRequest {
    fn merge(self, other: Self) -> Self {
        if self == Self::Stop { self } else { other }
    }
}

/// An event sink. The publisher owns the event and lends it during delivery.
/// The caller supplies the capability available to that delivery, if any.
///
/// An error is returned to the publisher in place of an execution request.
/// Effects already performed by handlers remain committed. The publisher
/// decides how to surface the error and when it can honor a stop.
pub trait Emit<E, C: ?Sized = ()> {
    type Error;

    fn emit(&mut self, event: &E, capability: &mut C) -> Result<ExecutionRequest, Self::Error>;
}

/// Deliver owned events in iterator order. A stop is collected while
/// remaining notifications are delivered. The first error ends delivery
/// and drops the remaining iterator. Earlier effects stay committed.
pub fn publish_events<E, C: ?Sized, S: Emit<E, C>>(
    events: impl IntoIterator<Item = E>,
    sink: &mut S,
    capability: &mut C,
) -> Result<ExecutionRequest, S::Error> {
    let mut request = ExecutionRequest::None;
    for event in events {
        request = request.merge(sink.emit(&event, capability)?);
    }
    Ok(request)
}

/// A locally owned, allocation-free set of subscribers to one event type.
///
/// Each registration owns its handler or takes the borrow captured by that
/// handler. Delivery runs in registration order. A stop is collected while
/// remaining handlers run. The first error returns immediately and skips the
/// rest. Nothing is queued or retried. Handlers can publish to independent
/// sinks they own or borrow. The active subscription cannot be entered again
/// while it is exclusively borrowed for delivery.
pub struct Subscriptions<E, X = Infallible, H = ()> {
    handlers: H,
    event: PhantomData<fn(&E) -> X>,
}

impl<E, X> Subscriptions<E, X> {
    pub const fn new() -> Self {
        Self {
            handlers: (),
            event: PhantomData,
        }
    }
}

impl<E, X> Default for Subscriptions<E, X> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E, X, H> Subscriptions<E, X, H> {
    /// Observe an event with an infallible function or closure returning `()`.
    pub fn on<F: FnMut(&E)>(self, handler: F) -> Subscriptions<E, X, Then<H, Observe<F>>> {
        Subscriptions {
            handlers: Then(self.handlers, Observe(handler)),
            event: PhantomData,
        }
    }

    /// Deliver to an existing typed route in this registration order.
    pub fn forward<S>(self, sink: S) -> Subscriptions<E, X, Then<H, Forward<S>>> {
        Subscriptions {
            handlers: Then(self.handlers, Forward(sink)),
            event: PhantomData,
        }
    }

    /// Subscribe a handler that can request a stop or return an error.
    pub fn try_on<F: FnMut(&E) -> Result<ExecutionRequest, X>>(
        self,
        handler: F,
    ) -> Subscriptions<E, X, Then<H, Fallible<F>>> {
        Subscriptions {
            handlers: Then(self.handlers, Fallible(handler)),
            event: PhantomData,
        }
    }

    /// Subscribe a function or closure with one explicitly wired capability.
    /// Delivery borrows the same capability separately for each handler.
    /// The sink checks the signature when it is used, including capabilities
    /// that carry a fresh lifetime on each delivery.
    pub fn on_with<F>(self, handler: F) -> Subscriptions<E, X, Then<H, WithCapability<F>>> {
        Subscriptions {
            handlers: Then(self.handlers, WithCapability(handler)),
            event: PhantomData,
        }
    }
}

// Concrete return types of the subscription builder. The constructors remain
// private so the builder is the only way to insert a handler.
#[doc(hidden)]
pub struct Then<A, B>(A, B);
#[doc(hidden)]
pub struct Observe<F>(F);
#[doc(hidden)]
pub struct Fallible<F>(F);
#[doc(hidden)]
pub struct Forward<S>(S);
#[doc(hidden)]
pub struct WithCapability<F>(F);

mod delivery {
    use super::ExecutionRequest;

    pub trait Deliver<E, C: ?Sized, X> {
        fn deliver(&mut self, event: &E, capability: &mut C) -> Result<ExecutionRequest, X>;
    }
}
use delivery::Deliver;

impl<E, C: ?Sized, X> Deliver<E, C, X> for () {
    #[inline]
    fn deliver(&mut self, _: &E, _: &mut C) -> Result<ExecutionRequest, X> {
        Ok(ExecutionRequest::None)
    }
}

impl<E, C: ?Sized, X, F: FnMut(&E)> Deliver<E, C, X> for Observe<F> {
    #[inline]
    fn deliver(&mut self, event: &E, _: &mut C) -> Result<ExecutionRequest, X> {
        self.0(event);
        Ok(ExecutionRequest::None)
    }
}

impl<E, C: ?Sized, X, S: Emit<E, C, Error = X>> Deliver<E, C, X> for Forward<S> {
    #[inline]
    fn deliver(&mut self, event: &E, capability: &mut C) -> Result<ExecutionRequest, X> {
        self.0.emit(event, capability)
    }
}

impl<E, C: ?Sized, X, F: FnMut(&E) -> Result<ExecutionRequest, X>> Deliver<E, C, X>
    for Fallible<F>
{
    #[inline]
    fn deliver(&mut self, event: &E, _: &mut C) -> Result<ExecutionRequest, X> {
        self.0(event)
    }
}

impl<E, C: ?Sized, X, F: FnMut(&E, &mut C) -> Result<ExecutionRequest, X>> Deliver<E, C, X>
    for WithCapability<F>
{
    #[inline]
    fn deliver(&mut self, event: &E, capability: &mut C) -> Result<ExecutionRequest, X> {
        self.0(event, capability)
    }
}

impl<E, C: ?Sized, X, A: Deliver<E, C, X>, B: Deliver<E, C, X>> Deliver<E, C, X> for Then<A, B> {
    #[inline]
    fn deliver(&mut self, event: &E, capability: &mut C) -> Result<ExecutionRequest, X> {
        let first = self.0.deliver(event, capability)?;
        let next = self.1.deliver(event, capability)?;
        Ok(first.merge(next))
    }
}

impl<E, C: ?Sized, X, H: Deliver<E, C, X>> Emit<E, C> for Subscriptions<E, X, H> {
    type Error = X;

    #[inline]
    fn emit(&mut self, event: &E, capability: &mut C) -> Result<ExecutionRequest, X> {
        self.handlers.deliver(event, capability)
    }
}

#[cfg(test)]
mod tests {
    use core::convert::Infallible;

    use super::*;

    #[test]
    fn ordered_handlers_nest_a_different_route_and_collect_stop() {
        struct Complete(u8);
        struct Trace(u8);
        let mut traces = Subscriptions::<Trace, Infallible>::new().on_with(
            |event: &Trace, output: &mut [u8; 3]| {
                output[1] = event.0;
                Ok(ExecutionRequest::None)
            },
        );
        let mut route = Subscriptions::<Complete, Infallible>::new()
            .on_with(|event: &Complete, output: &mut [u8; 3]| {
                output[0] = event.0;
                assert_eq!(
                    traces.emit(&Trace(event.0 + 1), output),
                    Ok(ExecutionRequest::None)
                );
                Ok(ExecutionRequest::Stop)
            })
            .on_with(|event: &Complete, output: &mut [u8; 3]| {
                assert_eq!(output[1], event.0 + 1);
                output[2] = event.0 + 2;
                Ok(ExecutionRequest::None)
            });
        let mut output = [0; 3];
        assert_eq!(
            route.emit(&Complete(4), &mut output),
            Ok(ExecutionRequest::Stop)
        );
        assert_eq!(output, [4, 5, 6]);
    }

    #[test]
    fn an_error_returns_unchanged_and_does_not_undo_prior_handlers() {
        let mut called = 0;
        let mut route = Subscriptions::<u8, &str>::new()
            .try_on(|event| {
                called += u32::from(*event);
                Ok(ExecutionRequest::Stop)
            })
            .try_on(|_| Err("subscriber refused"))
            .try_on(|_| panic!("delivery ends at the first error"));
        assert_eq!(route.emit(&3, &mut ()), Err("subscriber refused"));
        assert_eq!(called, 3);
    }

    #[test]
    fn a_batch_collects_stop_and_returns_the_first_error_without_replaying_events() {
        let mut route = Subscriptions::<u8, &str>::new().on_with(|event: &u8, seen: &mut u16| {
            *seen = *seen * 10 + u16::from(*event);
            match event {
                1 => Ok(ExecutionRequest::Stop),
                3 => Err("subscriber refused"),
                _ => Ok(ExecutionRequest::None),
            }
        });
        let mut seen = 0;
        assert_eq!(
            publish_events([1, 2], &mut route, &mut seen),
            Ok(ExecutionRequest::Stop)
        );
        assert_eq!(seen, 12);
        assert_eq!(
            publish_events([1, 3, 4], &mut route, &mut seen),
            Err("subscriber refused")
        );
        assert_eq!(seen, 1213);
    }

    #[test]
    fn an_empty_route_is_zero_sized_and_accepts_noncopy_events() {
        struct Notification<'a>(&'a mut u8);

        let mut empty = Subscriptions::<Notification<'_>, Infallible>::new();
        let mut state = 0;
        let event = Notification(&mut state);
        assert_eq!(core::mem::size_of_val(&empty), 0);
        assert_eq!(empty.emit(&event, &mut ()), Ok(ExecutionRequest::None));
        assert_eq!(*event.0, 0);
    }
}
