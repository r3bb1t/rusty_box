//! The debugger's symbols: what the engine has found for each module a
//! guest has loaded. The shell asks for modules and draws names from the
//! hub's published view; the workers look modules up, download them, and
//! publish what they found.

// Shared state other threads write, and names drawn from files a server
// sent: nothing here may panic (spec §11). Test code is free to.
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

pub(crate) mod hub;
pub(crate) mod kernel;
pub(crate) mod names;
pub(crate) mod wanted;
pub(crate) mod worker;
