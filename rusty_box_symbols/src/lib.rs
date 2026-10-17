//! Debug information for a module identified by its exact build: where it is
//! (your build folders, a symbol store, Microsoft's symbol server, debuginfod)
//! and what it says (names, source lines and structure layouts, from a PDB or
//! from DWARF). Nothing here touches an emulator or a user interface: a
//! module's identity comes in as bytes, answers go out as plain data, and
//! every side effect sits behind a port.
//!
//! Nothing here panics. Every file this crate reads may be hostile (a
//! download, a file in a shared folder, bytes a guest laid out), so every
//! format is read by this crate's own code, through the checked cursor in
//! [`read`], and the lints below make an index, an unchecked sum or an
//! `unwrap` a build error. Test code may use them.
#![forbid(unsafe_code)]
#![deny(
    unused_must_use,
    clippy::arithmetic_side_effects,
    clippy::expect_used,
    clippy::get_unwrap,
    clippy::impl_trait_in_params,
    clippy::indexing_slicing,
    clippy::let_underscore_must_use,
    clippy::must_use_candidate,
    clippy::panic,
    clippy::panic_in_result_fn,
    clippy::string_slice,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable,
    clippy::unwrap_used,
    clippy::wildcard_enum_match_arm
)]
#![cfg_attr(
    test,
    allow(
        clippy::arithmetic_side_effects,
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::must_use_candidate,
        clippy::panic,
        clippy::panic_in_result_fn,
        clippy::unwrap_used,
        clippy::wildcard_enum_match_arm
    )
)]

pub mod cab;
pub mod decompress;
pub mod dwarf;
pub mod elf;
pub mod fs_store;
pub mod http;
pub mod identity;
mod line_index;
#[cfg(any(test, feature = "fakes"))]
pub mod memory_store;
pub mod pdb;
pub mod pe;
pub mod projects;
pub mod read;
#[cfg(any(test, feature = "fakes"))]
pub mod scripted_http;
pub mod service;
pub mod source_path;
pub mod store;
pub mod symbol_path;
pub mod tables;
pub mod units;
pub mod ureq_http;
