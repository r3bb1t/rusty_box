//! Typed command-line and TOML configuration runner for Rusty Box.
//!
//! `rusty_box_gui` is the user-facing emulator launcher crate. It keeps CLI
//! parsing, TOML loading, validation, and emulator startup separate from the
//! core `rusty_box` emulator library so future GUI frontends can share one
//! resolved runner configuration.
//!
//! The runner supports terminal and headless display backends today. The
//! `gui-egui` feature only forwards the matching core emulator feature; it does
//! not install an egui window yet.
//!
//! Configuration is resolved in this order:
//!
//! 1. built-in defaults,
//! 2. `rusty_box.toml` from the current directory, or `--config PATH`,
//! 3. explicit CLI overrides.
//!
//! Use [`Args`] for Clap parsing, [`FileConfig`] for TOML input,
//! [`ResolvedConfig`] for validated startup state, and [`run`] or
//! [`run_resolved`] to start the emulator.

pub mod args;
pub mod config;
pub mod error;
pub mod runner;

pub use args::{Args, BootDevice, DiskGeometry, DisplayBackend};
pub use config::{FileConfig, ResolvedConfig};
pub use error::RunError;
pub use runner::{run, run_resolved, RunSummary};
