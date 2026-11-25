//! Typed, single-frame endpoints. The caller chooses when delivery occurs.

/// A marker for the protocol on the wire. Two protocols may use the same
/// frame representation and still cannot be wired together.
pub trait Protocol {
    type Frame;
}

/// One projection that selects an endpoint from a caller's loan.
///
/// The projection type fixes both the loan and the endpoint that can be
/// selected. A connection should keep the result of one call for the
/// pending check and transfer.
pub struct Port<P>(P);

impl<P> Port<P> {
    pub const fn new(project: P) -> Self {
        Self(project)
    }

    pub fn project<L, T>(&self, loan: L) -> T
    where
        P: Fn(L) -> T,
    {
        (self.0)(loan)
    }
}

/// One pending outgoing frame, owned by a guest peripheral.
pub struct Transmit<P: Protocol>(Option<P::Frame>);

impl<P: Protocol> Default for Transmit<P> {
    fn default() -> Self {
        Self(None)
    }
}

impl<P: Protocol> Transmit<P> {
    pub fn pending(&self) -> bool {
        self.0.is_some()
    }

    /// Queue a frame, returning it to the sender if the register is full.
    pub fn send(&mut self, frame: P::Frame) -> Result<(), P::Frame> {
        if self.pending() {
            return Err(frame);
        }
        self.0 = Some(frame);
        Ok(())
    }

    /// Move one pending frame into an empty receive register. A full receiver
    /// leaves both endpoints unchanged.
    pub fn deliver_to(&mut self, receiver: &mut Receive<P>) -> bool {
        if receiver.pending() {
            return false;
        }
        receiver.0 = self.0.take();
        receiver.pending()
    }
}

/// One pending incoming frame, owned by a guest peripheral.
pub struct Receive<P: Protocol>(Option<P::Frame>);

impl<P: Protocol> Default for Receive<P> {
    fn default() -> Self {
        Self(None)
    }
}

impl<P: Protocol> Receive<P> {
    pub fn pending(&self) -> bool {
        self.0.is_some()
    }

    pub fn take(&mut self) -> Option<P::Frame> {
        self.0.take()
    }
}

#[cfg(test)]
mod tests {
    use super::{Protocol, Receive, Transmit};

    struct Digital;
    struct Frame(u32);

    impl Protocol for Digital {
        type Frame = Frame;
    }

    #[test]
    fn a_full_receiver_leaves_the_next_frame_with_the_sender() {
        let mut out = Transmit::<Digital>::default();
        let mut input = Receive::default();
        assert!(!out.deliver_to(&mut input));
        assert!(out.send(Frame(1)).is_ok());
        assert!(out.deliver_to(&mut input));
        assert!(out.send(Frame(2)).is_ok());
        assert!(matches!(out.send(Frame(3)), Err(Frame(3))));
        assert!(!out.deliver_to(&mut input));
        assert!(matches!(input.take(), Some(Frame(1))));
        assert!(out.deliver_to(&mut input));
        assert!(!out.deliver_to(&mut input));
        assert!(matches!(input.take(), Some(Frame(2))));
        assert!(input.take().is_none());
    }
}
