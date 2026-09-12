//! Typed command-line, TOML, desktop egui, Android egui, and browser egui
//! runner for Rusty Box.
//!
//! `rusty_box_gui` is the user-facing emulator launcher crate. Native builds keep
//! CLI parsing, TOML loading, validation, and emulator startup separate from the
//! core `rusty_box` emulator library. The egui shell supports desktop, Android
//! and browser targets with target-specific runtime paths; Android enters
//! through [`android::main`].
//!
//! Native builds use the egui/eframe backend by default. Headless and terminal
//! backends remain available through `display.backend`; building with
//! `--no-default-features` removes egui support and falls back to terminal.
//!
//! Browser builds start an egui shell through `eframe::WebRunner` and avoid host
//! filesystem access.

#[cfg(all(target_os = "android", feature = "gui-egui"))]
pub mod android;
#[cfg(all(feature = "gui-egui", any(target_os = "android", test)))]
mod android_support;
#[cfg(feature = "gui-egui")]
pub mod app;
pub mod args;
#[cfg(feature = "gui-egui")]
pub(crate) mod shell;
pub mod config;
mod disk_images;
#[cfg(not(target_arch = "wasm32"))]
pub mod library;
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
#[cfg(all(feature = "gui-egui", not(target_arch = "wasm32")))]
pub use runner::{LaunchSource, LaunchVm, ShellOpening, ShellStart};
#[cfg(all(feature = "gui-egui", not(target_arch = "wasm32"), not(target_os = "android")))]
pub use runner::run_shell;
