//! Closure-based hook storage (Unicorn-style).
//!
//! Each hook category has a small struct holding:
//! - `handle: HookHandle` — the opaque ID returned to the user
//! - `range` — address range the hook fires in (stored as `(start, end)`
//!   after `RangeBounds` is resolved once at registration time)
//! - `cb: Box<dyn FnMut(...) + Send>` — user closure
//!
//! The registry stores these in per-category `Vec`s. Dispatch walks the
//! vec and fires every matching hook. The fast path (no hooks) is
//! short-circuited by `HookMask` one level up.

use alloc::boxed::Box;

use crate::cpu::decoder::Instruction;

use super::types::{
    BranchEvent, HookHandle, HwInterruptEvent, IoHookEvent, IoHookType, MemHookEvent,
    MemHookType,
};

/// Closed range `[start, end]` in u64 address space.
/// Stored in resolved form so the hot path does not re-evaluate
/// `RangeBounds` at every callsite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AddrRange<T: Copy + Ord> {
    pub(crate) start: T,
    pub(crate) end: T,
}

impl<T: Copy + Ord> AddrRange<T> {
    #[inline]
    pub(crate) fn contains(&self, addr: T) -> bool {
        addr >= self.start && addr <= self.end
    }
}

impl AddrRange<u64> {
    /// Resolve an arbitrary `RangeBounds<u64>` into an inclusive `[start, end]`.
    pub(crate) fn from_bounds<R: core::ops::RangeBounds<u64>>(range: R) -> Self {
        let start = match range.start_bound() {
            core::ops::Bound::Included(&v) => v,
            core::ops::Bound::Excluded(&v) => v.saturating_add(1),
            core::ops::Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            core::ops::Bound::Included(&v) => v,
            core::ops::Bound::Excluded(&v) => v.saturating_sub(1),
            core::ops::Bound::Unbounded => u64::MAX,
        };
        Self { start, end }
    }
}

impl AddrRange<u16> {
    pub(crate) fn from_bounds<R: core::ops::RangeBounds<u16>>(range: R) -> Self {
        let start = match range.start_bound() {
            core::ops::Bound::Included(&v) => v,
            core::ops::Bound::Excluded(&v) => v.saturating_add(1),
            core::ops::Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            core::ops::Bound::Included(&v) => v,
            core::ops::Bound::Excluded(&v) => v.saturating_sub(1),
            core::ops::Bound::Unbounded => u16::MAX,
        };
        Self { start, end }
    }
}

// ─────────────────────────── hook record structs ───────────────────────────

/// Code hook: fires before or after each instruction whose RIP is in range.
pub(crate) struct CodeHook {
    pub(crate) handle: HookHandle,
    pub(crate) range: AddrRange<u64>,
    pub(crate) cb: Box<dyn FnMut(u64, &Instruction) + Send>,
}

/// Memory access hook.
pub(crate) struct MemHook {
    pub(crate) handle: HookHandle,
    pub(crate) kind: MemHookType,
    pub(crate) range: AddrRange<u64>,
    pub(crate) cb: Box<dyn FnMut(&MemHookEvent) + Send>,
}

/// Software interrupt hook (INT n).
pub(crate) struct IntrHook {
    pub(crate) handle: HookHandle,
    pub(crate) cb: Box<dyn FnMut(u8) + Send>,
}

/// Hardware interrupt hook.
pub(crate) struct HwIntrHook {
    pub(crate) handle: HookHandle,
    pub(crate) cb: Box<dyn FnMut(&HwInterruptEvent) + Send>,
}

/// Exception hook.
pub(crate) struct ExceptionHook {
    pub(crate) handle: HookHandle,
    pub(crate) cb: Box<dyn FnMut(u8, u32) + Send>,
}

/// I/O port hook.
pub(crate) struct IoHook {
    pub(crate) handle: HookHandle,
    pub(crate) kind: IoHookType,
    pub(crate) range: AddrRange<u16>,
    pub(crate) cb: Box<dyn FnMut(&IoHookEvent) + Send>,
}

/// Branch hook.
pub(crate) struct BranchHook {
    pub(crate) handle: HookHandle,
    pub(crate) range: AddrRange<u64>,
    pub(crate) cb: Box<dyn FnMut(&BranchEvent) + Send>,
}
