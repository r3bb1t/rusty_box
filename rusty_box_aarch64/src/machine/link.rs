//! Deterministic point-to-point links between two custom AArch64 machines.

use core::fmt;

use rusty_box_core::{ClockHz, RunBudget, VmDuration, VmInstant};

use super::StopReason;
use super::custom::{Aarch64Machine, InvalidDeadline, MachineBoard};
use crate::cpu::NoEl2;

/// A marker for the protocol on the wire. Two protocols may use the same
/// frame representation and still cannot be wired together.
pub trait Protocol {
    type Frame;
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

/// The two projections of a particular port on a board. These functions
/// must select the same model in shared and mutable views.
pub struct Port<B, T> {
    inspect: fn(&B) -> &T,
    edit: fn(&mut B) -> &mut T,
}

impl<B, T> Port<B, T> {
    pub const fn new(inspect: fn(&B) -> &T, edit: fn(&mut B) -> &mut T) -> Self {
        Self { inspect, edit }
    }
}

/// A unidirectional connection owned by the orchestrator.
///
/// ```compile_fail,E0308
/// use rusty_box_aarch64::machine::link::{Link, Port, Protocol, Receive, Transmit};
/// struct Digital;
/// struct Serial;
/// impl Protocol for Digital { type Frame = u32; }
/// impl Protocol for Serial { type Frame = u32; }
/// let tx = Port::<Transmit<Digital>, _>::new(|tx| tx, |tx| tx);
/// let rx = Port::<Receive<Serial>, _>::new(|rx| rx, |rx| rx);
/// let _link = Link::new(tx, rx);
/// ```
///
/// ```compile_fail,E0308
/// use rusty_box_aarch64::machine::link::{Link, Port, Protocol, Transmit};
/// struct Digital;
/// impl Protocol for Digital { type Frame = u32; }
/// let a = Port::<Transmit<Digital>, _>::new(|tx| tx, |tx| tx);
/// let b = Port::<Transmit<Digital>, _>::new(|tx| tx, |tx| tx);
/// let _link = Link::new(a, b);
/// ```
pub struct Link<A, B, P: Protocol> {
    from: Port<A, Transmit<P>>,
    to: Port<B, Receive<P>>,
}

impl<A, B, P: Protocol> Link<A, B, P> {
    pub const fn new(from: Port<A, Transmit<P>>, to: Port<B, Receive<P>>) -> Self {
        Self { from, to }
    }

    fn transfer(
        &self,
        from: &mut Aarch64Machine<A, NoEl2>,
        to: &mut Aarch64Machine<B, NoEl2>,
    ) -> bool
    where
        A: MachineBoard,
        B: MachineBoard,
    {
        if !(self.from.inspect)(from.board()).pending() || (self.to.inspect)(to.board()).pending() {
            return false;
        }
        to.with_board_mut(|board| {
            let input = (self.to.edit)(board);
            if input.pending() {
                return false;
            }
            input.0 = from.with_board_mut(|board| (self.from.edit)(board).0.take());
            input.pending()
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    First,
    Second,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairError {
    RateMismatch {
        first: ClockHz,
        second: ClockHz,
    },
    ClockMismatch {
        first: VmInstant,
        second: VmInstant,
    },
    BarrierNotReached {
        actual: VmInstant,
        expected: VmInstant,
    },
    ZeroQuantum,
    TimeOverflow,
    Deadline {
        side: Side,
        error: InvalidDeadline,
    },
}

impl fmt::Display for PairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "machine pair could not reach its next barrier: {self:?}")
    }
}

impl core::error::Error for PairError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairStop {
    Completed,
    Machine { side: Side, reason: StopReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct PairOutcome {
    pub rounds: u32,
    pub frames: u64,
    pub stop: PairStop,
}

/// Two independent machines and their wire. Both must reach the same
/// virtual tick before any frame moves. At that barrier, first-to-second
/// moves before second-to-first, if configured. The peripherals see the
/// incoming frames on their next run. A full receiving register keeps the
/// outgoing frame queued. A terminal stop leaves the barrier incomplete.
pub struct ConnectedPair<A, B, P: Protocol> {
    first: Aarch64Machine<A, NoEl2>,
    second: Aarch64Machine<B, NoEl2>,
    forward: Link<A, B, P>,
    reverse: Option<Link<B, A, P>>,
}

impl<A: MachineBoard, B: MachineBoard, P: Protocol> ConnectedPair<A, B, P> {
    pub fn new(
        first: Aarch64Machine<A, NoEl2>,
        second: Aarch64Machine<B, NoEl2>,
        forward: Link<A, B, P>,
    ) -> Result<Self, PairError> {
        let pair = Self {
            first,
            second,
            forward,
            reverse: None,
        };
        pair.aligned()?;
        Ok(pair)
    }

    pub fn with_return_link(mut self, link: Link<B, A, P>) -> Self {
        self.reverse = Some(link);
        self
    }

    pub fn first(&self) -> &Aarch64Machine<A, NoEl2> {
        &self.first
    }

    pub fn second(&self) -> &Aarch64Machine<B, NoEl2> {
        &self.second
    }

    pub fn first_mut(&mut self) -> &mut Aarch64Machine<A, NoEl2> {
        &mut self.first
    }

    pub fn second_mut(&mut self) -> &mut Aarch64Machine<B, NoEl2> {
        &mut self.second
    }

    fn aligned(&self) -> Result<VmInstant, PairError> {
        let first_clock = self.first.clock();
        let second_clock = self.second.clock();
        if first_clock.rate() != second_clock.rate() {
            return Err(PairError::RateMismatch {
                first: first_clock.rate(),
                second: second_clock.rate(),
            });
        }
        let first = first_clock.now();
        let second = second_clock.now();
        if first != second {
            return Err(PairError::ClockMismatch { first, second });
        }
        Ok(first)
    }

    /// Advance at most `rounds` equal tick barriers. Timing faults or an
    /// early stop can leave the two clocks at different instants. The
    /// machines remain available for inspection and host recovery.
    pub fn run(&mut self, rounds: u32, quantum: VmDuration) -> Result<PairOutcome, PairError> {
        if quantum.ticks() == 0 {
            return Err(PairError::ZeroQuantum);
        }
        let mut outcome = PairOutcome {
            rounds: 0,
            frames: 0,
            stop: PairStop::Completed,
        };
        for _ in 0..rounds {
            let target = self
                .aligned()?
                .ticks()
                .checked_add(quantum.ticks())
                .map(VmInstant::from_ticks)
                .ok_or(PairError::TimeOverflow)?;
            let budget = RunBudget::Ticks(quantum.ticks());
            let first = self
                .first
                .step(budget)
                .map_err(|error| PairError::Deadline {
                    side: Side::First,
                    error,
                })?;
            if first.is_terminal() {
                outcome.stop = PairStop::Machine {
                    side: Side::First,
                    reason: first.stop,
                };
                break;
            }
            let second = self
                .second
                .step(budget)
                .map_err(|error| PairError::Deadline {
                    side: Side::Second,
                    error,
                })?;
            if second.is_terminal() {
                outcome.stop = PairStop::Machine {
                    side: Side::Second,
                    reason: second.stop,
                };
                break;
            }
            let reached = self.aligned()?;
            if reached != target {
                return Err(PairError::BarrierNotReached {
                    actual: reached,
                    expected: target,
                });
            }
            outcome.frames += u64::from(self.forward.transfer(&mut self.first, &mut self.second));
            if let Some(reverse) = &self.reverse {
                outcome.frames += u64::from(reverse.transfer(&mut self.second, &mut self.first));
            }
            outcome.rounds += 1;
        }
        Ok(outcome)
    }
}
