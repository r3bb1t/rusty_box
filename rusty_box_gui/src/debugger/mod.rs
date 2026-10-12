//! Pausing a running VM and reading it while it is paused.
//!
//! `control` is the one channel between the shell and a VM's run thread for
//! that, and the only writer of the run's stop flag. `inspect` reads what the
//! Debugger page shows; `dump` writes a VM's RAM to a file; `serve` is what a
//! run thread does with a machine that is between runs.

pub(crate) mod control;
pub(crate) mod dump;
pub(crate) mod inspect;
pub(crate) mod serve;
