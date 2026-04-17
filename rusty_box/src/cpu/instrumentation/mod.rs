//! CPU instrumentation — monomorphized generic `T: Instrumentation` + optional closure hooks.
//!
//! ## Trait-based instrumentation (primary)
//!
//! Implement [`Instrumentation`] and supply it as the type parameter to
//! [`InstrumentationRegistry<T>`]. All callbacks default to no-ops; override
//! only what you need. The trait is monomorphized — zero virtual dispatch
//! overhead when the concrete type is known at compile time.
//!
//! ## Unicorn-style closures (secondary, requires `alloc`)
//!
//! Register closures with `Emulator::hook_add_*` methods. Each returns a
//! [`HookHandle`]; pass it to `Emulator::hook_del` to remove the hook.
//! Closures receive event structs (or tagged enums) rather than flat
//! parameter lists so the call site is self-documenting.
//!
//! Both APIs can be active simultaneously: trait fires first, then
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
pub use registry::InstrumentationRegistry;
#[cfg(feature = "instrumentation")]
pub use registry::InstrumentationError;
pub use types::{
    BranchEvent, BranchType, CacheCntrl, CodeSize, CpuSetupMode, CpuSnapshot, EmuStopReason,
    ExitSet, HookMask, HwInterruptEvent, InvEptType, InvPcidType, IoHookEvent,
    MemAccessRW, MemHookEvent, MemPerms, MemType, MwaitFlags, PrefetchHint, ResetType,
    TlbCntrl, X86Reg,
};
#[cfg(feature = "instrumentation")]
pub use types::{HookHandle, IoHookType, MemHookType};
