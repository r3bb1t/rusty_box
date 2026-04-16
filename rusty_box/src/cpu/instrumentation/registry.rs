//! The `InstrumentationRegistry` — combines BOCHS-style trait instrumentation
//! with Unicorn-style closure hooks.
//!
//! Lives inside `BxCpuC` when the `instrumentation` feature is enabled. The
//! CPU hot path fires events through this registry; registration is done
//! via `Emulator::hook_add_*` / `Emulator::set_instrumentation`.
//!
//! ## Hot path contract
//!
//! Every `fire_*` method:
//! 1. Is `#[inline]` so that the bitmask short-circuit in `has_*()` can
//!    be hoisted by LLVM and combined with the outer callsite guard.
//! 2. Mutates at most `self.bochs` or one of the hook `Vec`s — never
//!    both at once — so that callsites can keep the `&mut BxCpuC` borrow
//!    that wraps the registry without re-entering any other CPU state.
//! 3. Does not allocate.
//!
//! The outer callsite pattern is:
//! ```ignore
//! #[cfg(feature = "instrumentation")]
//! if self.instrumentation.active.has_exec() {
//!     self.instrumentation.fire_before_execution(rip, instr);
//! }
//! ```

use alloc::{boxed::Box, vec::Vec};
use core::ops::RangeBounds;

use crate::cpu::decoder::Instruction;

use super::bochs::Instrumentation;
use super::hooks::{
    AddrRange, BranchHook, CodeHook, ExceptionHook, HwIntrHook, IntrHook, IoHook, MemHook,
};
use super::types::{
    BranchEvent, BranchType, CacheCntrl, CodeSize, HookHandle, HookMask, HwInterruptEvent,
    IoHookEvent, IoHookType, MemAccessRW, MemHookEvent, MemHookType, MemType, MwaitFlags,
    PrefetchHint, ResetType, TlbCntrl,
};

/// Error returned by registry mutation methods.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InstrumentationError {
    #[error("hook handle {0:#x} is invalid or already removed")]
    InvalidHandle(u64),
}

/// Registry holding the BOCHS trait object plus per-category closure vecs.
///
/// Feature-gated: absent entirely when `instrumentation` is disabled.
pub struct InstrumentationRegistry {
    /// Cheap bitmask querying whether any hook of a given category is registered.
    /// Callers check this before invoking `fire_*` to keep the hot path empty.
    pub active: HookMask,

    /// BOCHS-style trait instrumentation, if installed.
    pub(crate) bochs: Option<Box<dyn Instrumentation>>,

    pub(crate) code_hooks: Vec<CodeHook>,
    pub(crate) code_after_hooks: Vec<CodeHook>,
    pub(crate) mem_hooks: Vec<MemHook>,
    pub(crate) intr_hooks: Vec<IntrHook>,
    pub(crate) hw_intr_hooks: Vec<HwIntrHook>,
    pub(crate) exception_hooks: Vec<ExceptionHook>,
    pub(crate) io_hooks: Vec<IoHook>,
    pub(crate) branch_hooks: Vec<BranchHook>,

    /// Monotonic handle counter. Starts at 1; zero is reserved as "never
    /// returned" so future sentinel use is possible.
    next_handle: u64,
}

impl Default for InstrumentationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl InstrumentationRegistry {
    /// Create an empty registry. Zero allocations until a hook is registered.
    pub const fn new() -> Self {
        Self {
            active: HookMask::empty(),
            bochs: None,
            code_hooks: Vec::new(),
            code_after_hooks: Vec::new(),
            mem_hooks: Vec::new(),
            intr_hooks: Vec::new(),
            hw_intr_hooks: Vec::new(),
            exception_hooks: Vec::new(),
            io_hooks: Vec::new(),
            branch_hooks: Vec::new(),
            next_handle: 1,
        }
    }

    fn mint_handle(&mut self) -> HookHandle {
        let id = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(1);
        HookHandle::new(id)
    }

    /// Recompute `active` from the current hook vecs + BOCHS trait presence.
    fn recompute_active(&mut self) {
        let mut m = HookMask::empty();
        if self.bochs.is_some() {
            m |= HookMask::BOCHS_TRAIT;
        }
        if !self.code_hooks.is_empty() || !self.code_after_hooks.is_empty() {
            m |= HookMask::EXEC;
        }
        if !self.mem_hooks.is_empty() {
            m |= HookMask::MEM;
        }
        if !self.branch_hooks.is_empty() {
            m |= HookMask::BRANCH;
        }
        if !self.intr_hooks.is_empty() {
            m |= HookMask::INTERRUPT;
        }
        if !self.hw_intr_hooks.is_empty() {
            m |= HookMask::HW_INTERRUPT;
        }
        if !self.exception_hooks.is_empty() {
            m |= HookMask::EXCEPTION;
        }
        if !self.io_hooks.is_empty() {
            m |= HookMask::IO;
        }
        self.active = m;
    }

    // ─────────────────── BOCHS trait management ───────────────────

    pub fn set_bochs(
        &mut self,
        instr: Box<dyn Instrumentation>,
    ) -> Option<Box<dyn Instrumentation>> {
        let prev = self.bochs.replace(instr);
        self.recompute_active();
        prev
    }

    pub fn clear_bochs(&mut self) -> Option<Box<dyn Instrumentation>> {
        let prev = self.bochs.take();
        self.recompute_active();
        prev
    }

    // ─────────────────── Hook registration ───────────────────

    pub fn add_code<R: RangeBounds<u64>>(
        &mut self,
        range: R,
        cb: Box<dyn FnMut(u64, &Instruction) + Send>,
    ) -> HookHandle {
        let handle = self.mint_handle();
        self.code_hooks.push(CodeHook {
            handle,
            range: AddrRange::<u64>::from_bounds(range),
            cb,
        });
        self.active |= HookMask::EXEC;
        handle
    }

    pub fn add_code_after<R: RangeBounds<u64>>(
        &mut self,
        range: R,
        cb: Box<dyn FnMut(u64, &Instruction) + Send>,
    ) -> HookHandle {
        let handle = self.mint_handle();
        self.code_after_hooks.push(CodeHook {
            handle,
            range: AddrRange::<u64>::from_bounds(range),
            cb,
        });
        self.active |= HookMask::EXEC;
        handle
    }

    pub fn add_mem<R: RangeBounds<u64>>(
        &mut self,
        kind: MemHookType,
        range: R,
        cb: Box<dyn FnMut(&MemHookEvent) + Send>,
    ) -> HookHandle {
        let handle = self.mint_handle();
        self.mem_hooks.push(MemHook {
            handle,
            kind,
            range: AddrRange::<u64>::from_bounds(range),
            cb,
        });
        self.active |= HookMask::MEM;
        handle
    }

    pub fn add_interrupt(&mut self, cb: Box<dyn FnMut(u8) + Send>) -> HookHandle {
        let handle = self.mint_handle();
        self.intr_hooks.push(IntrHook { handle, cb });
        self.active |= HookMask::INTERRUPT;
        handle
    }

    pub fn add_hw_interrupt(
        &mut self,
        cb: Box<dyn FnMut(&HwInterruptEvent) + Send>,
    ) -> HookHandle {
        let handle = self.mint_handle();
        self.hw_intr_hooks.push(HwIntrHook { handle, cb });
        self.active |= HookMask::HW_INTERRUPT;
        handle
    }

    pub fn add_exception(&mut self, cb: Box<dyn FnMut(u8, u32) + Send>) -> HookHandle {
        let handle = self.mint_handle();
        self.exception_hooks.push(ExceptionHook { handle, cb });
        self.active |= HookMask::EXCEPTION;
        handle
    }

    pub fn add_io<R: RangeBounds<u16>>(
        &mut self,
        kind: IoHookType,
        range: R,
        cb: Box<dyn FnMut(&IoHookEvent) + Send>,
    ) -> HookHandle {
        let handle = self.mint_handle();
        self.io_hooks.push(IoHook {
            handle,
            kind,
            range: AddrRange::<u16>::from_bounds(range),
            cb,
        });
        self.active |= HookMask::IO;
        handle
    }

    pub fn add_branch<R: RangeBounds<u64>>(
        &mut self,
        range: R,
        cb: Box<dyn FnMut(&BranchEvent) + Send>,
    ) -> HookHandle {
        let handle = self.mint_handle();
        self.branch_hooks.push(BranchHook {
            handle,
            range: AddrRange::<u64>::from_bounds(range),
            cb,
        });
        self.active |= HookMask::BRANCH;
        handle
    }

    /// Remove any hook by handle. Searches every category; returns
    /// `Err(InvalidHandle)` if not found.
    pub fn remove(&mut self, handle: HookHandle) -> Result<(), InstrumentationError> {
        let target = handle;
        // Try every category; stop as soon as one hits.
        macro_rules! try_remove {
            ($vec:expr) => {{
                if let Some(pos) = $vec.iter().position(|h| h.handle == target) {
                    $vec.swap_remove(pos);
                    self.recompute_active();
                    return Ok(());
                }
            }};
        }
        try_remove!(self.code_hooks);
        try_remove!(self.code_after_hooks);
        try_remove!(self.mem_hooks);
        try_remove!(self.intr_hooks);
        try_remove!(self.hw_intr_hooks);
        try_remove!(self.exception_hooks);
        try_remove!(self.io_hooks);
        try_remove!(self.branch_hooks);
        Err(InstrumentationError::InvalidHandle(handle.raw()))
    }

    // ─────────────────── Fire methods (hot path) ───────────────────
    //
    // Each `fire_*` is called at every matching CPU event when its HookMask
    // bit is set. The outer guard in CPU code must check the mask first —
    // these methods assume at least one hook is interested.
    //
    // We iterate the bochs trait first (single indirect call), then walk
    // the closure vec. Removing a hook during dispatch would violate the
    // borrow checker (it would need `&mut self` on the registry while we
    // already hold a mutable borrow on one of its vecs) — users can't do
    // that anyway because hook_del requires `&mut Emulator`.

    #[inline]
    pub fn fire_reset(&mut self, reset_type: ResetType) {
        if let Some(b) = self.bochs.as_mut() {
            b.reset(reset_type);
        }
    }

    #[inline]
    pub fn fire_before_execution(&mut self, rip: u64, instr: &Instruction) {
        if let Some(b) = self.bochs.as_mut() {
            b.before_execution(rip, instr);
        }
        for h in &mut self.code_hooks {
            if h.range.contains(rip) {
                (h.cb)(rip, instr);
            }
        }
    }

    #[inline]
    pub fn fire_after_execution(&mut self, rip: u64, instr: &Instruction) {
        if let Some(b) = self.bochs.as_mut() {
            b.after_execution(rip, instr);
        }
        for h in &mut self.code_after_hooks {
            if h.range.contains(rip) {
                (h.cb)(rip, instr);
            }
        }
    }

    #[inline]
    pub fn fire_repeat_iteration(&mut self, rip: u64, instr: &Instruction) {
        if let Some(b) = self.bochs.as_mut() {
            b.repeat_iteration(rip, instr);
        }
    }

    #[inline]
    pub fn fire_opcode(&mut self, rip: u64, instr: &Instruction, bytes: &[u8], size: CodeSize) {
        if let Some(b) = self.bochs.as_mut() {
            b.opcode(rip, instr, bytes, size);
        }
    }

    #[inline]
    pub fn fire_hlt(&mut self) {
        if let Some(b) = self.bochs.as_mut() {
            b.hlt();
        }
    }

    #[inline]
    pub fn fire_mwait(&mut self, addr: u64, len: u32, flags: MwaitFlags) {
        if let Some(b) = self.bochs.as_mut() {
            b.mwait(addr, len, flags);
        }
    }

    #[inline]
    pub fn fire_cnear_branch_taken(&mut self, branch_rip: u64, new_rip: u64) {
        if let Some(b) = self.bochs.as_mut() {
            b.cnear_branch_taken(branch_rip, new_rip);
        }
        if !self.branch_hooks.is_empty() {
            let ev = BranchEvent::CnearTaken {
                src_rip: branch_rip,
                dst_rip: new_rip,
            };
            for h in &mut self.branch_hooks {
                if h.range.contains(branch_rip) {
                    (h.cb)(&ev);
                }
            }
        }
    }

    #[inline]
    pub fn fire_cnear_branch_not_taken(&mut self, branch_rip: u64, fallthrough_rip: u64) {
        if let Some(b) = self.bochs.as_mut() {
            b.cnear_branch_not_taken(branch_rip);
        }
        if !self.branch_hooks.is_empty() {
            let ev = BranchEvent::CnearNotTaken {
                src_rip: branch_rip,
                fallthrough_rip,
            };
            for h in &mut self.branch_hooks {
                if h.range.contains(branch_rip) {
                    (h.cb)(&ev);
                }
            }
        }
    }

    #[inline]
    pub fn fire_ucnear_branch(&mut self, what: BranchType, branch_rip: u64, new_rip: u64) {
        if let Some(b) = self.bochs.as_mut() {
            b.ucnear_branch(what, branch_rip, new_rip);
        }
        if !self.branch_hooks.is_empty() {
            let ev = BranchEvent::Ucnear {
                kind: what,
                src_rip: branch_rip,
                dst_rip: new_rip,
            };
            for h in &mut self.branch_hooks {
                if h.range.contains(branch_rip) {
                    (h.cb)(&ev);
                }
            }
        }
    }

    #[inline]
    pub fn fire_far_branch(
        &mut self,
        what: BranchType,
        prev_cs: u16,
        prev_rip: u64,
        new_cs: u16,
        new_rip: u64,
    ) {
        if let Some(b) = self.bochs.as_mut() {
            b.far_branch(what, prev_cs, prev_rip, new_cs, new_rip);
        }
        if !self.branch_hooks.is_empty() {
            let ev = BranchEvent::Far {
                kind: what,
                src_cs: prev_cs,
                src_rip: prev_rip,
                dst_cs: new_cs,
                dst_rip: new_rip,
            };
            for h in &mut self.branch_hooks {
                if h.range.contains(prev_rip) {
                    (h.cb)(&ev);
                }
            }
        }
    }

    #[inline]
    pub fn fire_interrupt(&mut self, vector: u8) {
        if let Some(b) = self.bochs.as_mut() {
            b.interrupt(vector);
        }
        for h in &mut self.intr_hooks {
            (h.cb)(vector);
        }
    }

    #[inline]
    pub fn fire_exception(&mut self, vector: u8, error_code: u32) {
        if let Some(b) = self.bochs.as_mut() {
            b.exception(vector, error_code);
        }
        for h in &mut self.exception_hooks {
            (h.cb)(vector, error_code);
        }
    }

    #[inline]
    pub fn fire_hwinterrupt(&mut self, vector: u8, cs: u16, rip: u64) {
        if let Some(b) = self.bochs.as_mut() {
            b.hwinterrupt(vector, cs, rip);
        }
        if !self.hw_intr_hooks.is_empty() {
            let ev = HwInterruptEvent { vector, cs, rip };
            for h in &mut self.hw_intr_hooks {
                (h.cb)(&ev);
            }
        }
    }

    #[inline]
    pub fn fire_lin_access(
        &mut self,
        lin: u64,
        phy: u64,
        len: usize,
        memtype: MemType,
        rw: MemAccessRW,
    ) {
        if let Some(b) = self.bochs.as_mut() {
            b.lin_access(lin, phy, len, memtype, rw);
        }
        if !self.mem_hooks.is_empty() {
            let ev = MemHookEvent {
                access: rw,
                addr: lin,
                size: len,
                value: None,
                phys_addr: phy,
                memtype,
            };
            for h in &mut self.mem_hooks {
                if h.kind.matches(rw) && h.range.contains(lin) {
                    (h.cb)(&ev);
                }
            }
        }
    }

    #[inline]
    pub fn fire_phy_access(&mut self, phy: u64, len: usize, memtype: MemType, rw: MemAccessRW) {
        if let Some(b) = self.bochs.as_mut() {
            b.phy_access(phy, len, memtype, rw);
        }
    }

    #[inline]
    pub fn fire_inp(&mut self, port: u16, len: u8) {
        if let Some(b) = self.bochs.as_mut() {
            b.inp(port, len);
        }
    }

    #[inline]
    pub fn fire_inp2(&mut self, port: u16, len: u8, val: u32) {
        if let Some(b) = self.bochs.as_mut() {
            b.inp2(port, len, val);
        }
        if !self.io_hooks.is_empty() {
            let ev = IoHookEvent {
                port,
                size: len,
                value: val,
                access: MemAccessRW::Read,
            };
            for h in &mut self.io_hooks {
                if h.kind.matches(MemAccessRW::Read) && h.range.contains(port) {
                    (h.cb)(&ev);
                }
            }
        }
    }

    #[inline]
    pub fn fire_outp(&mut self, port: u16, len: u8, val: u32) {
        if let Some(b) = self.bochs.as_mut() {
            b.outp(port, len, val);
        }
        if !self.io_hooks.is_empty() {
            let ev = IoHookEvent {
                port,
                size: len,
                value: val,
                access: MemAccessRW::Write,
            };
            for h in &mut self.io_hooks {
                if h.kind.matches(MemAccessRW::Write) && h.range.contains(port) {
                    (h.cb)(&ev);
                }
            }
        }
    }

    #[inline]
    pub fn fire_tlb_cntrl(&mut self, what: TlbCntrl) {
        if let Some(b) = self.bochs.as_mut() {
            b.tlb_cntrl(what);
        }
    }

    #[inline]
    pub fn fire_cache_cntrl(&mut self, what: CacheCntrl) {
        if let Some(b) = self.bochs.as_mut() {
            b.cache_cntrl(what);
        }
    }

    #[inline]
    pub fn fire_clflush(&mut self, laddr: u64, paddr: u64) {
        if let Some(b) = self.bochs.as_mut() {
            b.clflush(laddr, paddr);
        }
    }

    #[inline]
    pub fn fire_prefetch_hint(&mut self, what: PrefetchHint, seg: u8, offset: u64) {
        if let Some(b) = self.bochs.as_mut() {
            b.prefetch_hint(what, seg, offset);
        }
    }

    #[inline]
    pub fn fire_cpuid(&mut self) {
        if let Some(b) = self.bochs.as_mut() {
            b.cpuid();
        }
    }

    #[inline]
    pub fn fire_wrmsr(&mut self, msr: u32, value: u64) {
        if let Some(b) = self.bochs.as_mut() {
            b.wrmsr(msr, value);
        }
    }

    #[inline]
    pub fn fire_vmexit(&mut self, reason: u32, qualification: u64) {
        if let Some(b) = self.bochs.as_mut() {
            b.vmexit(reason, qualification);
        }
    }
}
