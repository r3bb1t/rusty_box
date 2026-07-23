#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(not(feature = "alloc"))]
extern crate self as tracing;

#[cfg(not(feature = "alloc"))]
#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {{
        core::format_args!($($arg)*);
    }};
}

#[cfg(not(feature = "alloc"))]
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {{
        core::format_args!($($arg)*);
    }};
}

#[cfg(not(feature = "alloc"))]
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {{
        core::format_args!($($arg)*);
    }};
}

#[cfg(not(feature = "alloc"))]
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {{
        core::format_args!($($arg)*);
    }};
}

#[cfg(not(feature = "alloc"))]
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {{
        core::format_args!($($arg)*);
    }};
}

#[cfg(not(feature = "alloc"))]
#[macro_export]
macro_rules! enabled {
    ($($arg:tt)*) => {
        false
    };
}

#[cfg(not(feature = "alloc"))]
#[allow(non_camel_case_types)]
pub enum Level {
    TRACE,
    DEBUG,
    INFO,
    WARN,
    ERROR,
}

// Always available: core emulation modules (no alloc needed)
pub mod config;
pub mod cpu;
mod crc;
pub mod error;
pub use error::{Error, Result};
pub mod boot;
pub mod dma;
pub mod memory;
pub mod params;
pub mod pc_system;
pub mod pic;
pub mod ring_buffer;
pub(crate) mod vec_diag;

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
#[cfg(feature = "alloc")]
pub use emulator::Emulator;
pub use cpu::CpuidFreq;
pub use emulator::EmulatorConfig;
