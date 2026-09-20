//! CPU instrumentation — an observer is a type, not a registration.
//!
//! ## Installing a tracer
//!
//! Implement [`Instrumentation`] on your own type and hand one to the
//! machine; it becomes the machine's `T`, and the CPU carries it inside
//! itself for the machine's whole life. There is no registration call and no
//! handle to keep:
//!
//! ```ignore
//! #[derive(Default)]
//! struct CountInstructions { executed: u64 }
//!
//! impl Instrumentation for CountInstructions {
//!     fn active_hooks(&self) -> HookMask { HookMask::EXEC }
//!     fn before_execution(&mut self, _rip: u64, _i: &Instruction) { self.executed += 1 }
//! }
//!
//! let mut machine = MachineBuilder::new(config)
//!     .tracer(CountInstructions::default())
//!     .build()?;
//! // …run…
//! let executed = machine.instrumentation().executed;
//! ```
//!
//! An SMP machine needs one tracer per processor, so it takes a factory —
//! `MachineBuilder::tracer_factory(CountInstructions::default)` — rather than
//! one instance to share. [`Emulator::new_with_mode_and_instrumentation`] is
//! the short form for a machine with no firmware.
//!
//! Every callback defaults to a no-op, so a tracer names only the hooks it
//! wants. Watch two things at once by making the tracer a tuple: `(A, B)`
//! implements the trait when both `A` and `B` do, and every hook reaches both.
//!
//! ## Not observing anything
//!
//! `()` is the default `T` and the trait's no-op implementation. Its
//! [`active_hooks`](Instrumentation::active_hooks) is empty, which is what
//! makes an uninstrumented machine free: the CPU tests one bitmask at each
//! hook site and takes a predicted-not-taken branch. There is no feature flag
//! to set — instrumentation is always compiled, and the cost of not using it
//! is that branch.
//!
//! ## The shape of a callback
//!
//! Multi-argument hooks take `&Event` structs (e.g. [`OpcodeEvent`],
//! [`LinAccess`], [`BranchEvent`]) with named fields; the 0–2 argument hooks
//! stay positional. See [`bochs`] for the full callback design and for what
//! each hook promises.
//!
//! ## The hook that can change what happens
//!
//! [`Instrumentation::pre_syscall`] receives a [`HookCtx`] (register + memory
//! r/w, stop) and returns an [`InstrAction`] (`Continue` / `Skip` / `Stop` /
//! `SkipAndStop`). It is the only hook that alters architectural effects: the
//! rest observe. It is OS-agnostic — the library makes no assumption about
//! syscall conventions.
//!
//! [`Emulator::new_with_mode_and_instrumentation`]: crate::emulator::Emulator::new_with_mode_and_instrumentation

pub mod bochs;
pub mod ctx;
pub mod registry;
pub mod types;

pub use bochs::Instrumentation;
pub use ctx::{CpuAccess, HookCtx};
pub use registry::InstrumentationRegistry;
pub use types::{
    BranchEvent, BranchType, CacheCntrl, CodeSize, CpuSetupMode, CpuSnapshot, EmuStopReason,
    ExitSet, HookMask, HwInterruptEvent, InstrAction, InvEptType, InvPcidType, IoHookEvent,
    LinAccess, MemAccessRW, MemHookEvent, MemPermViolation, MemPerms, MemType, MemUnmapped,
    MwaitEvent, MwaitFlags, OpcodeEvent, PhyAccess, PrefetchEvent, PrefetchHint, ResetType,
    TlbCntrl, X86Reg,
};
