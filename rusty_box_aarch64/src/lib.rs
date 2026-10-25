//! An AArch64 (ARMv8.0-A) interpreter and a board compatible with QEMU's
//! `virt` machine, built the way the rest of rusty_box is: deterministic,
//! `no_std`, allocation-free at its core, and without `unsafe`.
//!
//! Determinism comes from time being a count of retired instructions
//! (`rusty_box_core::VmInstant`) at a stated rate (`ClockHz`), exactly as Bochs
//! keeps `ticks_total` at `ips`. The generic timer's counter, the RTC and every
//! timer deadline are functions of that count, never of the host clock, so the
//! same firmware and the same inputs produce the same run instruction for
//! instruction regardless of how a caller slices it into budgets.
//!
//! A machine is assembled by [`machine::MachineBuilder`], whose type records
//! how the guest starts (UEFI firmware, a Linux kernel loaded directly, or
//! nothing) and which builds either a machine that owns its memory (with
//! `alloc`) or one that runs in buffers a caller lends it. The machine is
//! driven the way an x86 rusty_box machine is — `step` with a
//! [`RunBudget`], serial and keyboard handles, registers and memory by name,
//! snapshots, and an observer type parameter — and the guide in
//! `docs/aarch64-automation.md` walks through it.
//!
//! The processor model is a Cortex-A57-shaped ARMv8.0 core with EL0 and EL1
//! only (no EL2, no EL3, no AArch32), FP and Advanced SIMD, the
//! Cryptographic Extension (AES, 64-bit PMULL, SHA-1, SHA-256) and CRC32, a
//! 4 KiB-granule stage-1 MMU and a GICv2. `cpu::id` records every
//! identification value the guest can read, and every instruction outside
//! the features those values advertise is UNDEFINED, as the architecture
//! requires.
//!
//! Provenance (rusty_box R7): no Bochs upstream exists for any of this. The
//! instruction semantics follow the Arm Architecture Reference Manual for
//! A-profile (DDI 0487) pseudocode; the board and its devices follow QEMU's
//! `hw/arm/virt.c` and the device models it instantiates, cited per file.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(any(test, feature = "std"))]
extern crate std;

pub mod board;
pub mod boot;
pub mod bus;
pub mod cpu;
pub mod devices;
#[cfg(feature = "alloc")]
pub mod disk;
pub mod instrumentation;
pub mod machine;
mod softfloat;
pub mod storage;

pub use bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
pub use cpu::{Cpu, CpuConfig, CpuState, El, Pstate, Reg, SysReg, SysRegRefusal, VReg};
pub use instrumentation::{Instrumentation, NoInstrumentation};
#[cfg(feature = "alloc")]
pub use machine::StopHandle;
pub use machine::{
    BatchOutcome, BuildError, MachineBuilder, MemoryError, MemoryLoan, OnResetRequest, PowerState, RamSize,
    StopReason, VirtConfig, VirtMachine,
};
pub use board::{Plugged, VirtioDevices};
#[cfg(feature = "alloc")]
pub use disk::{Access, Disk};
#[cfg(feature = "alloc")]
pub use machine::Display;
pub use machine::{Keyboard, Pointer};
pub use rusty_box_core::input::pointer::{PointerButton, PointerInput, PointerPos, WheelSteps};
pub use rusty_box_core::input::{Key, KeyAction, KeyboardInput, Typed};
/// What the display shows through: the size of a picture and the sink a
/// host implements to receive its frames, with every type its methods take,
/// shared with the x86 machine.
pub use rusty_box_devices::display::sink::{
    CursorPos, Dimensions, DisplaySink, NullDisplay, Redraw, Refreshed, Rgb, TilePos,
};
pub use rusty_box_devices::display::vga::VgaTextModeInfo;
pub use rusty_box_devices::display::Resolution;
/// The verbs every rusty_box machine shares, whatever it emulates.
pub use rusty_box_core::machine::{Machine, RunOutcome, SerialPort};
pub use rusty_box_core::{ClockHz, Progress, RunBudget};
#[cfg(feature = "alloc")]
pub use storage::OwnedStorage;
pub use storage::{GuestStorage, LentStorage};

/// The automation guide's Rust blocks are compiled by `cargo test --doc
/// --features std`, so the guide cannot promise a signature the crate does
/// not have.
#[cfg(all(doctest, feature = "std"))]
#[doc = include_str!("../../docs/aarch64-automation.md")]
struct AutomationGuide;
