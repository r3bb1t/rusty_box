//! An AArch64 (ARMv8.0-A) interpreter and a board compatible with QEMU's
//! `virt` machine, built the way the rest of rusty_box is: deterministic,
//! `no_std`, allocation-free, and without `unsafe`.
//!
//! Determinism comes from time being a count of retired instructions
//! (`rusty_box_core::VmInstant`) at a stated rate (`ClockHz`), exactly as Bochs
//! keeps `ticks_total` at `ips`. The generic timer's counter, the RTC and every
//! timer deadline are functions of that count, never of the host clock, so the
//! same firmware and the same inputs produce the same run instruction for
//! instruction regardless of how a caller slices it into budgets.
//!
//! Memory is borrowed, not owned: a caller lends guest RAM and the two flash
//! banks as slices (`VirtMemory`), so the machine itself never allocates.
//!
//! The processor model is a Cortex-A57-shaped ARMv8.0 core with EL0 and EL1
//! only (no EL2, no EL3, no AArch32), FP and Advanced SIMD, a 4 KiB-granule
//! stage-1 MMU and a GICv2. `cpu::id` records every identification value the
//! guest can read, and every instruction outside the features those values
//! advertise is UNDEFINED, as the architecture requires.
//!
//! Provenance (rusty_box R7): no Bochs upstream exists for any of this. The
//! instruction semantics follow the Arm Architecture Reference Manual for
//! A-profile (DDI 0487) pseudocode; the board and its devices follow QEMU's
//! `hw/arm/virt.c` and the device models it instantiates, cited per file.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "std")]
extern crate std;

pub mod board;
pub mod bus;
pub mod cpu;
pub mod devices;
pub mod machine;
mod softfloat;

pub use bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
pub use cpu::{Cpu, CpuConfig, El};
pub use machine::{RunBudget, StopReason, VirtConfig, VirtMachine, VirtMemory, VirtBuildError};
