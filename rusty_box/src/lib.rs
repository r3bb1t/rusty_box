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
pub(crate) mod convert;
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
/// The ring buffer every fixed-capacity device FIFO is built on. It is
/// arch-neutral, so it lives in the core crate; re-exported under its
/// established path because `RingBuffer<T, N>` appears in public device
/// signatures.
pub use rusty_box_core::ring_buffer;
pub(crate) mod vec_diag;

// Emulator modules — core types always available,
// alloc-dependent methods gated internally per-method.
pub mod emulator;
pub mod emulator_api;
#[cfg(feature = "alloc")]
pub use emulator_api::StopHandle;
/// Device role handles. Transient `&mut` borrows of one role, obtained from
/// the machine — the supported path to device state now that machine parts are
/// crate-private (doctrine R3).
pub use emulator_api::{DebugPort, Serial};
#[cfg(feature = "alloc")]
pub mod gui;
pub mod iodev;
#[cfg(feature = "std")]
pub mod snapshot;

// Re-export commonly used types
pub use cpu::CpuidFreq;
/// The guest-physical map and the vocabulary it is written in. The window
/// types come from the core crate because a permission on guest memory means
/// the same thing to every execution engine; the derivation is this machine's.
pub use memory::plan::{MemoryPlan, MemoryPlanError};
pub use rusty_box_core::{GpaPerms, GpaPlan, GpaPlanError, GpaWindow, HostOffset, GUEST_PAGE};

pub use emulator::Emulator;
pub use emulator::EmulatorConfig;
