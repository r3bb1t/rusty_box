//! Device models for the rusty_box emulator.
//!
//! A model here answers a guest: it decodes ports and physical windows, keeps
//! the state a guest can observe, and saves that state. What it deliberately
//! cannot do is reach the machine around it — there is no CPU, no scheduler,
//! no emulator and no host in scope, and no bus to call. A device states what
//! it answers on and is handed what it needs at the moment of the access.
//!
//! That restriction is the point. The same models have to run under the
//! software CPU and, later, under a real hypervisor, where the machine around
//! them is a different thing entirely. Until a model actually compiled in a
//! crate that cannot name those things, "it only depends on the foundations"
//! was an intention; this crate's dependency list is the compiler checking it.
//!
//! `no_std` and allocation-free unconditionally, matching `rusty_box_core`;
//! `alloc` and `std` are additive and never change the shape of what exists
//! without them (R0).

#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

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


pub mod api;
pub mod display;
pub mod pci;
