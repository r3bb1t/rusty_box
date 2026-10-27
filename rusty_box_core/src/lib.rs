//! Architecture-neutral foundations for the rusty_box emulator.
//!
//! Nothing here knows what an x86 processor is or how a guest is executed.
//! The rule that keeps it that way: a type belongs in this crate only if a
//! device model, an execution engine and every machine — the x86 PC and the
//! AArch64 `virt` board alike — could all name it without any of them naming
//! each other. The PC keyboard's keys in [`input`] pass that test: the x86
//! machine renders them as PS/2 scancodes in its own crate, and the AArch64
//! machine sends them as a serial terminal's bytes ([`input::terminal`]),
//! which belong to any machine with a UART console, not to a processor.
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

pub mod engine;
pub mod float;
pub mod input;
pub mod machine;
pub mod reset;
pub mod ring_buffer;
pub mod run;
pub mod snap;
pub mod time;

pub use engine::{
    EngineCaps, EngineFault, EngineFaultKind, GpaOverlap, GpaPerms, GpaPlan, GpaPlanError,
    GpaWindow, HostOffset, VpIndex, GUEST_PAGE,
};
pub use float::FloatExt;
pub use reset::ResetReason;
pub use ring_buffer::RingBuffer;
pub use run::{MixedProgress, Progress, RunBudget};
pub use snap::{
    SnapError, SnapRead, SnapResult, SnapWrite, SnapshotSection, MAX_COUNT, MAX_SECTION_LEN,
};
pub use time::{ClockHz, Cycles, HostClock, HostInstant, MicrosPhase, TimeOverflow, VmClock, VmDuration, VmInstant};
