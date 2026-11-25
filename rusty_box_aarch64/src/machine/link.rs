//! Deterministic point-to-point links between two custom AArch64 machines.

use core::fmt;

use rusty_box_core::connection::Port as EndpointPort;
pub use rusty_box_core::connection::{Protocol, Receive, Transmit};
use rusty_box_core::{ClockHz, RunBudget, VmDuration, VmInstant};

use super::StopReason;
use super::custom::{Aarch64Machine, InvalidDeadline};
use crate::board::address_space::{AddressSpace, DeviceMap, DevicesMut};
use crate::cpu::NoEl2;

/// The single projection that identifies an endpoint in the fixed device
/// tree. Its input can reach peripheral models, with no access to guest RAM,
/// mappings or CPU state.
pub type Port<D, T> = EndpointPort<for<'a> fn(DevicesMut<'a, D>) -> &'a mut T>;

/// A unidirectional connection between two checked device trees.
///
/// ```compile_fail,E0308
/// use rusty_box_aarch64::machine::link::{Link, Port, Protocol, Receive, Transmit};
/// struct Digital;
/// struct Serial;
/// impl Protocol for Digital { type Frame = u32; }
/// impl Protocol for Serial { type Frame = u32; }
/// fn mismatched<A, B>(tx: Port<A, Transmit<Digital>>, rx: Port<B, Receive<Serial>>) {
///     let _link = Link::new(tx, rx);
/// }
/// ```
///
/// ```compile_fail,E0308
/// use rusty_box_aarch64::machine::link::{Link, Port, Protocol, Transmit};
/// struct Digital;
/// impl Protocol for Digital { type Frame = u32; }
/// fn reversed<A, B>(tx: Port<A, Transmit<Digital>>, other_tx: Port<B, Transmit<Digital>>) {
///     let _link = Link::new(tx, other_tx);
/// }
/// ```
pub struct Link<A, B, P: Protocol> {
    from: Port<A, Transmit<P>>,
    to: Port<B, Receive<P>>,
}

impl<A, B, P: Protocol> Link<A, B, P> {
    pub const fn new(from: Port<A, Transmit<P>>, to: Port<B, Receive<P>>) -> Self {
        Self { from, to }
    }

    /// Deliver at most one frame through exclusive loans to both endpoints.
    pub fn transfer(&self, from: DevicesMut<'_, A>, to: DevicesMut<'_, B>) -> bool {
        let output = self.from.project(from);
        output.pending() && output.deliver_to(self.to.project(to))
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
pub struct ConnectedPair<'first, 'second, A, B, P: Protocol> {
    first: Aarch64Machine<AddressSpace<'first, A>, NoEl2>,
    second: Aarch64Machine<AddressSpace<'second, B>, NoEl2>,
    forward: Link<A, B, P>,
    reverse: Option<Link<B, A, P>>,
}

impl<'first, 'second, A: DeviceMap, B: DeviceMap, P: Protocol>
    ConnectedPair<'first, 'second, A, B, P>
{
    pub fn new(
        first: Aarch64Machine<AddressSpace<'first, A>, NoEl2>,
        second: Aarch64Machine<AddressSpace<'second, B>, NoEl2>,
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

    pub fn first(&self) -> &Aarch64Machine<AddressSpace<'first, A>, NoEl2> {
        &self.first
    }

    pub fn second(&self) -> &Aarch64Machine<AddressSpace<'second, B>, NoEl2> {
        &self.second
    }

    pub fn first_mut(&mut self) -> &mut Aarch64Machine<AddressSpace<'first, A>, NoEl2> {
        &mut self.first
    }

    pub fn second_mut(&mut self) -> &mut Aarch64Machine<AddressSpace<'second, B>, NoEl2> {
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
            outcome.frames += u64::from(
                self.forward
                    .transfer(self.first.device_view(), self.second.device_view()),
            );
            if let Some(reverse) = &self.reverse {
                outcome.frames += u64::from(
                    reverse.transfer(self.second.device_view(), self.first.device_view()),
                );
            }
            outcome.rounds += 1;
        }
        Ok(outcome)
    }
}
