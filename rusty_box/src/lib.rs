#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

// Always available: CPU instrumentation types and trait (no alloc needed)
pub mod cpu;

// Everything below requires heap allocation
#[cfg(feature = "alloc")]
pub mod error;
#[cfg(feature = "alloc")]
pub use error::{Error, Result};

#[cfg(feature = "alloc")]
mod config;
#[cfg(feature = "alloc")]
mod crc;
#[cfg(feature = "alloc")]
pub mod emulator;
#[cfg(feature = "alloc")]
pub mod emulator_api;
#[cfg(feature = "alloc")]
pub use emulator_api::StopHandle;
#[cfg(feature = "alloc")]
pub mod gui;
#[cfg(feature = "alloc")]
pub mod iodev;
#[cfg(feature = "alloc")]
pub mod memory;
#[cfg(feature = "alloc")]
mod misc;
#[cfg(feature = "alloc")]
pub mod params;
#[cfg(feature = "alloc")]
pub mod pc_system;
#[cfg(feature = "alloc")]
pub mod snapshot;

// Re-export commonly used types (alloc-gated since Emulator needs alloc)
#[cfg(feature = "alloc")]
pub use emulator::{Emulator, EmulatorConfig};
