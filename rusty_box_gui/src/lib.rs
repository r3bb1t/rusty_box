//! Typed command-line, TOML, desktop egui, and browser egui runner for Rusty Box.
//!
//! `rusty_box_gui` is the user-facing emulator launcher crate. Native builds keep
//! CLI parsing, TOML loading, validation, and emulator startup separate from the
//! core `rusty_box` emulator library. The egui shell supports both desktop and
//! browser targets with target-specific runtime paths.
//!
//! Native builds use the egui/eframe backend by default. Headless and terminal
//! backends remain available through `display.backend`; building with
//! `--no-default-features` removes egui support and falls back to terminal.
//!
//! Browser builds start an egui shell through `eframe::WebRunner` and avoid host
//! filesystem access.

#[cfg(feature = "gui-egui")]
pub mod app;
pub mod args;
pub mod config;
mod disk_images;
pub mod error;
#[cfg(all(feature = "guest-trace", not(target_arch = "wasm32")))]
pub mod guest_trace;
#[cfg(not(target_arch = "wasm32"))]
pub mod runner;

pub use args::{Args, BootDevice, DiskGeometry, DisplayBackend};
pub use config::{FileConfig, ResolvedConfig};
pub use error::RunError;
#[cfg(not(target_arch = "wasm32"))]
pub use runner::{run, run_resolved, RunSummary};
