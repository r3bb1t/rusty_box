//! What every interpreted rusty_box machine does for a host, whatever
//! processor it emulates: run under a budget, show its serial console, take
//! keys.
//!
//! Each machine keeps its own richer surface — an x86 machine has a screen
//! and MSRs, an AArch64 one system registers and PSCI — and its own closed
//! enum of stop reasons. These traits are the part they share, so a host can
//! write "boot until the console says `login:`, then type `root`" once.
//!
//! A machine on a hypervisor is not one of them. It runs on the host's
//! processor through a driver of its own, and fitting it under these verbs
//! would only give it a `step` that refuses at run time; leaving it out makes
//! that mistake a compile error instead.
//!
//! ```
//! use rusty_box_core::input::{KeyboardInput, Typed};
//! use rusty_box_core::machine::{Machine, RunOutcome, SerialPort};
//! use rusty_box_core::run::RunBudget;
//!
//! /// Step until `needle` appears on serial port 0, for at most `rounds`
//! /// steps; then log in.
//! fn log_in<M: Machine>(machine: &mut M, needle: &[u8], rounds: usize) -> Result<bool, M::Error> {
//!     let mut seen = Vec::new();
//!     for _ in 0..rounds {
//!         let outcome = machine.step(RunBudget::Ticks(5_000_000))?;
//!         if let Some(mut port) = machine.serial(0) {
//!             seen.extend(port.take_output());
//!         }
//!         if seen.windows(needle.len()).any(|w| w == needle) {
//!             return Ok(matches!(machine.keyboard().type_text("root\n"), Typed::All { .. }));
//!         }
//!         if outcome.is_terminal() {
//!             break;
//!         }
//!     }
//!     Ok(false)
//! }
//! ```

use crate::input::KeyboardInput;
use crate::run::{Progress, RunBudget};

/// How one run of a machine ended.
pub trait RunOutcome {
    /// How far the guest got.
    fn progress(&self) -> Progress;

    /// Whether the machine should not be stepped again without host action:
    /// the guest switched itself off, the host asked to stop, or the
    /// processor met something it will meet again. A run that merely spent
    /// its budget, or found the guest waiting, is not terminal.
    fn is_terminal(&self) -> bool;
}

/// A machine's serial port, as a host drives it.
pub trait SerialPort {
    /// What [`SerialPort::take_output`] drains.
    type Output<'a>: Iterator<Item = u8>
    where
        Self: 'a;

    /// The bytes the guest has transmitted, oldest first. Draining is
    /// destructive.
    fn take_output(&mut self) -> Self::Output<'_>;

    /// Send bytes for the guest to receive, returning how many the machine
    /// took; `bytes[taken..]` is what to send again once the guest reads.
    #[must_use = "a short count means the rest never reached the guest"]
    fn send(&mut self, bytes: &[u8]) -> usize;
}

/// The verbs every interpreted rusty_box machine answers.
pub trait Machine {
    /// How a run ended: the machine's own outcome, with its own closed set
    /// of stop reasons.
    type Outcome: RunOutcome;
    /// What can go wrong running it. `core::convert::Infallible` for a
    /// machine whose runs always end with an outcome.
    type Error;
    type Serial<'a>: SerialPort
    where
        Self: 'a;
    type Keyboard<'a>: KeyboardInput
    where
        Self: 'a;

    /// Run until `budget` is spent or something stops the machine.
    ///
    /// # Errors
    ///
    /// Whatever the machine reports instead of an outcome.
    fn step(&mut self, budget: RunBudget) -> Result<Self::Outcome, Self::Error>;

    /// Guest time since the machine was built, in ticks.
    fn ticks(&self) -> u64;

    /// Serial port `port` (0-based), if the machine built it.
    fn serial(&mut self, port: usize) -> Option<Self::Serial<'_>>;

    /// The machine's keyboard: whatever keyboard the machine has, in the
    /// shared key vocabulary.
    fn keyboard(&mut self) -> Self::Keyboard<'_>;
}

/// A boxed machine is a machine: builders that hand back `Box<_>` (the x86
/// machine is large) need no unboxing to reach shared code.
#[cfg(feature = "alloc")]
impl<M: Machine + ?Sized> Machine for alloc::boxed::Box<M> {
    type Outcome = M::Outcome;
    type Error = M::Error;
    type Serial<'a>
        = M::Serial<'a>
    where
        Self: 'a;
    type Keyboard<'a>
        = M::Keyboard<'a>
    where
        Self: 'a;

    fn step(&mut self, budget: RunBudget) -> Result<Self::Outcome, Self::Error> {
        (**self).step(budget)
    }

    fn ticks(&self) -> u64 {
        (**self).ticks()
    }

    fn serial(&mut self, port: usize) -> Option<Self::Serial<'_>> {
        (**self).serial(port)
    }

    fn keyboard(&mut self) -> Self::Keyboard<'_> {
        (**self).keyboard()
    }
}
