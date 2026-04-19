#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

// Always available: core emulation modules (no alloc needed)
pub mod cpu;
pub mod config;
mod crc;
pub mod error;
pub use error::{Error, Result};
pub mod memory;
mod misc;
pub mod params;
pub mod pc_system;
pub mod boot;
pub mod pic;
pub mod dma;
pub mod ring_buffer;

// Emulator modules — core types always available,
// alloc-dependent methods gated internally per-method.
pub mod emulator;
pub mod emulator_api;
#[cfg(feature = "alloc")]
pub use emulator_api::StopHandle;
#[cfg(feature = "alloc")]
pub mod gui;
pub mod iodev;
#[cfg(feature = "std")]
pub mod snapshot;

// Re-export commonly used types
pub use emulator::EmulatorConfig;
#[cfg(feature = "alloc")]
pub use emulator::Emulator;