//! CPU instrumentation — BOCHS-compatible trait + Unicorn-style closure hooks.
//!
//! Two complementary APIs share one registry on the CPU:
//!
//! ## BOCHS-style trait (primary)
//!
//! Implement [`Instrumentation`] and install with
//! `Emulator::set_instrumentation(Box::new(MyInstr))`. All callbacks default
//! to no-ops; override only what you need. Full-fidelity port of the C++
//! BOCHS callbacks (see `bochs::Instrumentation` docs for the faithfulness
//! audit).
//!
//! ## Unicorn-style closures (secondary)
//!
//! Register closures with `Emulator::hook_add_*` methods. Each returns a
//! [`HookHandle`]; pass it to `Emulator::hook_del` to remove the hook.
//! Closures receive event structs (or tagged enums) rather than flat
//! parameter lists so the call site is self-documenting.
//!
//! Both APIs can be active simultaneously: BOCHS trait fires first, then
//! closures walk in registration order.
//!
//! ## Feature gate
//!
//! The entire module is gated on `instrumentation`. When the feature is
//! disabled, no code is generated at any hook site and the registry field
//! does not exist on [`BxCpuC`](crate::cpu::BxCpuC).
//!
//! ## Performance
//!
//! The hot path checks a single bitmask ([`HookMask`]) before doing any
//! work. With the feature enabled but no hooks registered, every callsite
//! collapses to one predictable branch-not-taken.

pub mod bochs;
pub mod hooks;
pub mod registry;
pub mod types;

pub use bochs::Instrumentation;
pub use registry::{InstrumentationError, InstrumentationRegistry};
pub use types::{
    BranchEvent, BranchType, CacheCntrl, CodeSize, CpuSetupMode, CpuSnapshot, EmuStopReason,
    HookHandle, HookMask, HwInterruptEvent, InvEptType, InvPcidType, IoHookEvent, IoHookType,
    MemAccessRW, MemHookEvent, MemHookType, MemType, MwaitFlags, PrefetchHint, ResetType,
    TlbCntrl, X86Reg,
};
