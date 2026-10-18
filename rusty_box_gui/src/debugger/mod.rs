//! Pausing a running VM and reading it while it is paused.
//!
//! `control` is the one channel between the shell and a VM's run thread for
//! that, and the only writer of the run's stop flag. `inspect` reads what the
//! Debugger page shows; `search` finds bytes in guest memory; `windows` lists
//! a 64-bit Windows guest's processes and kernel modules; `dump` writes a
//! VM's RAM to a file; `serve` is what a run thread does with a machine that
//! is between runs.

// Every walk here reads memory a guest controls, so nothing here may panic
// (spec §11): no indexing, no unchecked arithmetic, no unwrap. Test code is
// free to; `gui clippy` in `cargo xtask ci` holds the library to it.
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented,
        clippy::arithmetic_side_effects,
        clippy::string_slice,
        clippy::get_unwrap,
        clippy::panic_in_result_fn
    )
)]

pub(crate) mod control;
pub(crate) mod drivers;
pub(crate) mod dump;
#[cfg(test)]
pub(crate) mod fake_windows;
pub(crate) mod frames;
pub(crate) mod guest;
pub(crate) mod inspect;
pub(crate) mod kernel_layout;
pub(crate) mod memory_map;
pub(crate) mod minimap;
pub(crate) mod module_identity;
pub(crate) mod pe;
pub(crate) mod search;
pub(crate) mod serve;
pub(crate) mod source_breakpoints;
pub(crate) mod ssdt;
pub(crate) mod stepping;
#[cfg(test)]
pub(crate) mod test_frame;
pub(crate) mod threads;
pub(crate) mod unwind;
pub(crate) mod vad;
pub(crate) mod variables;
pub(crate) mod watch;
pub(crate) mod windows;
pub(crate) mod xref;
