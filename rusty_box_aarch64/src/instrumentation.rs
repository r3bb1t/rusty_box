//! Watching what the guest does.
//!
//! An observer is a type, not a callback list: it rides along in the
//! machine's own type (`VirtMachine<S, I>`), every hook has a do-nothing
//! default, and dispatch is static (R8). A machine built with
//! [`NoInstrumentation`] — the default — compiles every hook away, so
//! observing nothing costs nothing; an observer pays only for the hooks it
//! overrides.
//!
//! The hooks come in three families:
//!
//! - **execution** — [`Instrumentation::before_execution`],
//!   [`Instrumentation::after_execution`], [`Instrumentation::branch`],
//!   [`Instrumentation::wait_for_interrupt`];
//! - **exceptions** — [`Instrumentation::exception`], for every synchronous
//!   exception, IRQ and FIQ the processor takes;
//! - **memory** — [`Instrumentation::memory_access`], for every data access
//!   that reaches the physical bus.

use crate::bus::{AccessWidth, PhysAddr};
use crate::cpu::{El, ExceptionClass};

/// Which way a data access went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Read,
    Write,
}

/// One data access that reached the physical bus and completed.
///
/// Physical, because that is where a load or store becomes an access: one
/// architectural access may become several here (a 128-bit register is two
/// 8-byte halves, an access that crosses a page is one byte at a time), and
/// a table walk's descriptor reads and instruction fetches are not data
/// accesses, so neither appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemoryAccess {
    pub address: PhysAddr,
    pub width: AccessWidth,
    pub direction: Direction,
    /// The value read, or the value written, in the low `width` bytes.
    pub value: u64,
}

/// What made the processor take an exception.
///
/// Closed (R5): an observer that dispatches on it decides every kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExceptionKind {
    /// Raised by the instruction at the preferred return address (or, for
    /// `SVC`, the one before it).
    Synchronous(ExceptionClass),
    /// The interrupt controller's IRQ output, taken at an instruction
    /// boundary.
    Irq,
    /// The interrupt controller's FIQ output, taken at an instruction
    /// boundary.
    Fiq,
}

/// An exception the processor has just taken to EL1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExceptionTaken {
    pub kind: ExceptionKind,
    /// The exception level it was taken from.
    pub from: El,
    /// What `ELR_EL1` now holds: where an `ERET` returns to.
    pub return_address: u64,
    /// The vector the processor now executes from.
    pub vector: u64,
    /// What `ESR_EL1` now holds for a synchronous exception; zero for an
    /// interrupt, which leaves it unchanged.
    pub syndrome: u64,
    /// The address `FAR_EL1` now holds, for an exception that reports one:
    /// an instruction or data abort, or a PC alignment fault.
    pub fault_address: Option<u64>,
}

/// Hooks a machine calls as its processor runs. Every one defaults to
/// nothing.
#[allow(unused_variables)]
pub trait Instrumentation {
    /// The instruction at `pc` was fetched and is about to execute.
    #[inline(always)]
    fn before_execution(&mut self, pc: u64, insn: u32) {}

    /// The instruction at `pc` retired. Not called for one that raised an
    /// exception instead: [`Instrumentation::exception`] reports that.
    #[inline(always)]
    fn after_execution(&mut self, pc: u64, insn: u32) {}

    /// The instruction at `from` transferred control to `to`: a taken
    /// branch, or an exception return.
    #[inline(always)]
    fn branch(&mut self, from: u64, to: u64) {}

    /// The processor took an exception.
    #[inline(always)]
    fn exception(&mut self, taken: ExceptionTaken) {}

    /// A data access completed on the physical bus.
    #[inline(always)]
    fn memory_access(&mut self, access: MemoryAccess) {}

    /// The instruction at `pc` (`WFI`, or a PSCI `CPU_SUSPEND`) put the
    /// processor to sleep until an interrupt.
    #[inline(always)]
    fn wait_for_interrupt(&mut self, pc: u64) {}
}

/// The observer of a machine nobody is watching: every hook is the default,
/// so every call compiles away.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoInstrumentation;

impl Instrumentation for NoInstrumentation {}
