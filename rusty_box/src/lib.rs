#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;


pub mod error;
pub use error::{Error, Result};

mod config;
pub mod cpu;
mod crc;
pub mod emulator;
pub mod gui;
pub mod iodev;
pub mod memory;
mod misc;
pub mod params;
pub mod pc_system;
pub mod snapshot;

// Re-export commonly used types
pub use emulator::{Emulator, EmulatorConfig};
