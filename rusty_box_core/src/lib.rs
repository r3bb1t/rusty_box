//! Architecture-neutral foundations for the rusty_box emulator.
//!
//! Nothing here knows what an x86 is, what a PC is, or how a guest is executed.
//! The rule that keeps it that way: a type belongs in this crate only if a
//! device model, an execution engine and a future non-x86 machine could all
//! name it without any of them naming each other.
//!
//! `no_std` and allocation-free unconditionally; `alloc` and `std` are additive
//! features that add conveniences and never change the shape of what exists
//! without them.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

/// Linked so [`float::FloatExt`]'s `std` arms resolve to the inherent `f32` /
/// `f64` methods. Without it those arms name the trait method they are
/// defining and recurse — the crate is `#![no_std]` unconditionally, so
/// enabling the feature is not by itself enough to put the inherent methods in
/// scope.
#[cfg(feature = "std")]
extern crate std;

pub mod float;
pub mod reset;
pub mod ring_buffer;
pub mod snap;
pub mod time;

pub use float::FloatExt;
pub use reset::ResetReason;
pub use ring_buffer::RingBuffer;
pub use snap::{SnapError, SnapRead, SnapResult, SnapWrite, SnapshotSection};
pub use time::{ClockHz, HostClock, HostInstant, MicrosPhase, TimeOverflow, VmClock, VmDuration, VmInstant};
