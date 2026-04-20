//! The `InstrumentationRegistry` — combines a monomorphized generic tracer
//! with Unicorn-style closure hooks.
//!
//! Lives inside `BxCpuC` when the `instrumentation` feature is enabled. The
//! CPU hot path fires events through this registry; registration is done
//! via `Emulator::hook_add_*`.
//!
//! ## Hot path contract
//!
//! Every `fire_*` method:
//! 1. Is `#[inline]` so that the bitmask short-circuit in `has_*()` can
//!    be hoisted by LLVM and combined with the outer callsite guard.
//! 2. Calls `self.tracer.method()` first (zero-cost when `T = ()`),
//!    then walks the closure vec (when the `alloc` feature is enabled).
//! 3. Does not allocate.
//!
//! The outer callsite pattern is:
//! ```ignore
//! #[cfg(feature = "instrumentation")]
//! if self.instrumentation.active.has_exec() {
//!     self.instrumentation.fire_before_execution(rip, instr);
//! }
//! ```

#[cfg(feature = "instrumentation")]
use alloc::{boxed::Box, vec::Vec};
#[cfg(feature = "instrumentation")]
use core::ops::RangeBounds;

use crate::cpu::decoder::Instruction;

use super::bochs::Instrumentation;
#[cfg(feature = "instrumentation")]
use super::hooks::{
    AddrRange, BlockHook, BranchHook, CodeHook, ExceptionHook, HwIntrHook, IntrHook,
    InvalidInsnHook, IoHook, MemHook, MemUnmappedHook,
};
use super::types::{
    BranchEvent, CacheCntrl, HookMask, HwInterruptEvent, IoHookEvent,
    LinAccess, MemPermViolation, MemUnmapped,
    MwaitEvent, OpcodeEvent, PhyAccess, PrefetchEvent, ResetType, TlbCntrl,
};
#[cfg(feature = "instrumentation")]
use super::types::{HookHandle, IoHookType, MemAccessRW, MemHookEvent, MemHookType};

/// Error returned by registry mutation methods.
#[cfg(feature = "instrumentation")]
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InstrumentationError {
    #[error("hook handle {0:#x} is invalid or already removed")]
    InvalidHandle(u64),
}

/// Registry holding the monomorphized tracer plus per-category closure vecs.
///
/// Feature-gated: absent entirely when `instrumentation` is disabled.
pub struct InstrumentationRegistry<T: Instrumentation = ()> {
    /// Cheap bitmask querying whether any hook of a given category is registered.
    /// Callers check this before invoking `fire_*` to keep the hot path empty.
    pub active: HookMask,

    /// Cooperative stop request. When a hook sets this to `true`, the CPU
    /// loop exits at the next trace boundary and `step_batch` returns. This is
    /// the Rust analogue of Bochs's `bx_pc_system.kill_bochs_request`, scoped
    /// to instrumentation so hooks can stop execution without global state.
    /// Single-threaded — plain `bool`, no atomic.
    pub stop_request: bool,

    /// Monomorphized tracer — zero-cost when `T = ()`. Wrapped in `Option`
    /// so `fire_*` methods that need `&mut HookCtx` can `take()` the tracer
    /// out temporarily, build a ctx over the rest of the CPU state, and put
    /// the tracer back. `None` is only observable mid-dispatch.
    pub(crate) tracer: Option<T>,

    #[cfg(feature = "instrumentation")]
    pub(crate) code_hooks: Vec<CodeHook>,
    #[cfg(feature = "instrumentation")]
    pub(crate) code_after_hooks: Vec<CodeHook>,
    #[cfg(feature = "instrumentation")]
    pub(crate) mem_hooks: Vec<MemHook>,
    #[cfg(feature = "instrumentation")]
    pub(crate) intr_hooks: Vec<IntrHook>,
    #[cfg(feature = "instrumentation")]
    pub(crate) hw_intr_hooks: Vec<HwIntrHook>,
    #[cfg(feature = "instrumentation")]
    pub(crate) exception_hooks: Vec<ExceptionHook>,
    #[cfg(feature = "instrumentation")]
    pub(crate) io_hooks: Vec<IoHook>,
    #[cfg(feature = "instrumentation")]
    pub(crate) branch_hooks: Vec<BranchHook>,
    #[cfg(feature = "instrumentation")]
    pub(crate) block_hooks: Vec<BlockHook>,
    #[cfg(feature = "instrumentation")]
    pub(crate) invalid_insn_hooks: Vec<InvalidInsnHook>,
    #[cfg(feature = "instrumentation")]
    pub(crate) mem_unmapped_hooks: Vec<MemUnmappedHook>,

    /// Monotonic handle counter. Starts at 1; zero is reserved as "never
    /// returned" so future sentinel use is possible.
    #[cfg(feature = "instrumentation")]
    next_handle: u64,

}

impl<T: Instrumentation + Default> Default for InstrumentationRegistry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Instrumentation> InstrumentationRegistry<T> {
    /// Create a registry with the given tracer. Zero allocations until a hook
    /// is registered.
    pub fn with_tracer(tracer: T) -> Self {
        let mut reg = Self {
            active: HookMask::empty(),
            stop_request: false,
            tracer: Some(tracer),
            #[cfg(feature = "instrumentation")]
            code_hooks: Vec::new(),
            #[cfg(feature = "instrumentation")]
            code_after_hooks: Vec::new(),
            #[cfg(feature = "instrumentation")]
            mem_hooks: Vec::new(),
            #[cfg(feature = "instrumentation")]
            intr_hooks: Vec::new(),
            #[cfg(feature = "instrumentation")]
            hw_intr_hooks: Vec::new(),
            #[cfg(feature = "instrumentation")]
            exception_hooks: Vec::new(),
            #[cfg(feature = "instrumentation")]
            io_hooks: Vec::new(),
            #[cfg(feature = "instrumentation")]
            branch_hooks: Vec::new(),
            #[cfg(feature = "instrumentation")]
            block_hooks: Vec::new(),
            #[cfg(feature = "instrumentation")]
            invalid_insn_hooks: Vec::new(),
            #[cfg(feature = "instrumentation")]
            mem_unmapped_hooks: Vec::new(),
            #[cfg(feature = "instrumentation")]
            next_handle: 1,
        };
        reg.refresh_active();
        reg
    }

    /// Recompute `active` from the tracer's active hooks and closure vec
    /// occupancy. Call after any mutation that changes what's installed.
    pub fn refresh_active(&mut self) {
        #[allow(unused_mut)]
        let mut m = self.tracer.as_ref().map_or(HookMask::empty(), |t| t.active_hooks());

        #[cfg(feature = "instrumentation")]
        {
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
            if !self.block_hooks.is_empty() {
                m |= HookMask::BLOCK;
            }
            if !self.invalid_insn_hooks.is_empty() {
                m |= HookMask::INVALID_INSN;
            }
            if !self.mem_unmapped_hooks.is_empty() {
                m |= HookMask::MEM_UNMAPPED;
            }
        }

        self.active = m;
    }

    // ─────────────────── Hook registration (alloc only) ───────────────────

    #[cfg(feature = "instrumentation")]
    fn mint_handle(&mut self) -> HookHandle {
        let id = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(1);
        HookHandle::new(id)
    }

    #[cfg(feature = "instrumentation")]
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

    #[cfg(feature = "instrumentation")]
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

    #[cfg(feature = "instrumentation")]
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

    #[cfg(feature = "instrumentation")]
    pub fn add_interrupt(&mut self, cb: Box<dyn FnMut(u8) + Send>) -> HookHandle {
        let handle = self.mint_handle();
        self.intr_hooks.push(IntrHook { handle, cb });
        self.active |= HookMask::INTERRUPT;
        handle
    }

    #[cfg(feature = "instrumentation")]
    pub fn add_hw_interrupt(
        &mut self,
        cb: Box<dyn FnMut(&HwInterruptEvent) + Send>,
    ) -> HookHandle {
        let handle = self.mint_handle();
        self.hw_intr_hooks.push(HwIntrHook { handle, cb });
        self.active |= HookMask::HW_INTERRUPT;
        handle
    }

    #[cfg(feature = "instrumentation")]
    pub fn add_exception(&mut self, cb: Box<dyn FnMut(u8, u32) + Send>) -> HookHandle {
        let handle = self.mint_handle();
        self.exception_hooks.push(ExceptionHook { handle, cb });
        self.active |= HookMask::EXCEPTION;
        handle
    }

    #[cfg(feature = "instrumentation")]
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

    #[cfg(feature = "instrumentation")]
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

    #[cfg(feature = "instrumentation")]
    pub fn add_block<R: RangeBounds<u64>>(
        &mut self,
        range: R,
        cb: Box<dyn FnMut(u64, u16) + Send>,
    ) -> HookHandle {
        let handle = self.mint_handle();
        self.block_hooks.push(BlockHook {
            handle,
            range: AddrRange::<u64>::from_bounds(range),
            cb,
        });
        self.active |= HookMask::BLOCK;
        handle
    }

    #[cfg(feature = "instrumentation")]
    pub fn add_invalid_insn(
        &mut self,
        cb: Box<dyn FnMut(u64) -> bool + Send>,
    ) -> HookHandle {
        let handle = self.mint_handle();
        self.invalid_insn_hooks.push(InvalidInsnHook { handle, cb });
        self.active |= HookMask::INVALID_INSN;
        handle
    }

    #[cfg(feature = "instrumentation")]
    pub fn add_mem_unmapped(
        &mut self,
        cb: Box<dyn FnMut(u64, usize, MemAccessRW) -> bool + Send>,
    ) -> HookHandle {
        let handle = self.mint_handle();
        self.mem_unmapped_hooks.push(MemUnmappedHook { handle, cb });
        self.active |= HookMask::MEM_UNMAPPED;
        handle
    }

    /// Remove any hook by handle. Searches every category; returns
    /// `Err(InvalidHandle)` if not found.
    #[cfg(feature = "instrumentation")]
    pub fn remove(&mut self, handle: HookHandle) -> Result<(), InstrumentationError> {
        let target = handle;
        // Try every category; stop as soon as one hits.
        macro_rules! try_remove {
            ($vec:expr) => {{
                if let Some(pos) = $vec.iter().position(|h| h.handle == target) {
                    $vec.swap_remove(pos);
                    self.refresh_active();
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
        try_remove!(self.block_hooks);
        try_remove!(self.invalid_insn_hooks);
        try_remove!(self.mem_unmapped_hooks);
        Err(InstrumentationError::InvalidHandle(handle.raw()))
    }

    // ─────────────────── Fire methods (hot path) ───────────────────
    //
    // Each `fire_*` is called at every matching CPU event when its HookMask
    // bit is set. The outer guard in CPU code must check the mask first —
    // these methods assume at least one hook is interested.
    //
    // We call the tracer first (monomorphized, zero dispatch), then walk
    // the closure vec (when `alloc` is enabled).

    #[inline]
    pub fn fire_reset(&mut self, reset_type: ResetType) {
        if let Some(t) = self.tracer.as_mut() { t.reset(reset_type); }
    }

    #[inline]
    pub fn fire_before_execution(&mut self, rip: u64, instr: &Instruction) {
        if let Some(t) = self.tracer.as_mut() { t.before_execution(rip, instr); }
        #[cfg(feature = "instrumentation")]
        for h in &mut self.code_hooks {
            if h.range.contains(rip) {
                (h.cb)(rip, instr);
            }
        }
    }

    #[inline]
    pub fn fire_after_execution(&mut self, rip: u64, instr: &Instruction) {
        if let Some(t) = self.tracer.as_mut() { t.after_execution(rip, instr); }
        #[cfg(feature = "instrumentation")]
        for h in &mut self.code_after_hooks {
            if h.range.contains(rip) {
                (h.cb)(rip, instr);
            }
        }
    }

    #[inline]
    pub fn fire_repeat_iteration(&mut self, rip: u64, instr: &Instruction) {
        if let Some(t) = self.tracer.as_mut() { t.repeat_iteration(rip, instr); }
    }

    #[inline]
    pub fn fire_opcode(&mut self, ev: &OpcodeEvent) {
        if let Some(t) = self.tracer.as_mut() { t.opcode(ev); }
    }

    #[inline]
    pub fn fire_hlt(&mut self) {
        if let Some(t) = self.tracer.as_mut() { t.hlt(); }
    }

    #[inline]
    pub fn fire_mwait(&mut self, ev: &MwaitEvent) {
        if let Some(t) = self.tracer.as_mut() { t.mwait(ev); }
    }

    /// Unified branch-event fire. Replaces the 4 Bochs-style callbacks
    /// (cnear_taken/not_taken, ucnear, far) — callers construct the
    /// appropriate `BranchEvent` variant at the callsite.
    #[inline]
    pub fn fire_branch(&mut self, ev: &BranchEvent) {
        if let Some(t) = self.tracer.as_mut() { t.branch(ev); }
        #[cfg(feature = "instrumentation")]
        if !self.branch_hooks.is_empty() {
            let src_rip = ev.src_rip();
            for h in &mut self.branch_hooks {
                if h.range.contains(src_rip) {
                    (h.cb)(ev);
                }
            }
        }
    }

    #[inline]
    pub fn fire_interrupt(&mut self, vector: u8) {
        if let Some(t) = self.tracer.as_mut() { t.interrupt(vector); }
        #[cfg(feature = "instrumentation")]
        for h in &mut self.intr_hooks {
            (h.cb)(vector);
        }
    }

    #[inline]
    pub fn fire_exception(&mut self, vector: u8, error_code: u32) {
        if let Some(t) = self.tracer.as_mut() { t.exception(vector, error_code); }
        #[cfg(feature = "instrumentation")]
        for h in &mut self.exception_hooks {
            (h.cb)(vector, error_code);
        }
    }

    #[inline]
    pub fn fire_hwinterrupt(&mut self, ev: &HwInterruptEvent) {
        if let Some(t) = self.tracer.as_mut() { t.hwinterrupt(ev); }
        #[cfg(feature = "instrumentation")]
        for h in &mut self.hw_intr_hooks {
            (h.cb)(ev);
        }
    }

    #[inline]
    pub fn fire_lin_access(&mut self, ev: &LinAccess) {
        if let Some(t) = self.tracer.as_mut() { t.lin_access(ev); }
        #[cfg(feature = "instrumentation")]
        if !self.mem_hooks.is_empty() {
            // Map small accesses (≤8 bytes) to an integer value for closure hooks.
            let value = match ev.data.len() {
                1 => Some(u64::from(ev.data[0])),
                2 => Some(u64::from(u16::from_le_bytes([ev.data[0], ev.data[1]]))),
                4 => {
                    let mut b = [0u8; 4];
                    b.copy_from_slice(&ev.data[..4]);
                    Some(u64::from(u32::from_le_bytes(b)))
                }
                8 => {
                    let mut b = [0u8; 8];
                    b.copy_from_slice(&ev.data[..8]);
                    Some(u64::from_le_bytes(b))
                }
                _ => None,
            };
            let hev = MemHookEvent {
                access: ev.rw,
                addr: ev.lin,
                size: ev.data.len(),
                value,
                phys_addr: ev.phy,
                memtype: ev.memtype,
            };
            for h in &mut self.mem_hooks {
                if h.kind.matches(ev.rw) && h.range.contains(ev.lin) {
                    (h.cb)(&hev);
                }
            }
        }
    }

    #[inline]
    pub fn fire_phy_access(&mut self, ev: &PhyAccess) {
        if let Some(t) = self.tracer.as_mut() { t.phy_access(ev); }
    }

    #[inline]
    pub fn fire_inp(&mut self, port: u16, size: u8) {
        if let Some(t) = self.tracer.as_mut() { t.inp(port, size); }
    }

    #[inline]
    pub fn fire_inp2(&mut self, ev: &IoHookEvent) {
        if let Some(t) = self.tracer.as_mut() { t.inp2(ev); }
        #[cfg(feature = "instrumentation")]
        for h in &mut self.io_hooks {
            if h.kind.matches(ev.access) && h.range.contains(ev.port) {
                (h.cb)(ev);
            }
        }
    }

    #[inline]
    pub fn fire_outp(&mut self, ev: &IoHookEvent) {
        if let Some(t) = self.tracer.as_mut() { t.outp(ev); }
        #[cfg(feature = "instrumentation")]
        for h in &mut self.io_hooks {
            if h.kind.matches(ev.access) && h.range.contains(ev.port) {
                (h.cb)(ev);
            }
        }
    }

    #[inline]
    pub fn fire_tlb_cntrl(&mut self, what: TlbCntrl) {
        if let Some(t) = self.tracer.as_mut() { t.tlb_cntrl(what); }
    }

    #[inline]
    pub fn fire_cache_cntrl(&mut self, what: CacheCntrl) {
        if let Some(t) = self.tracer.as_mut() { t.cache_cntrl(what); }
    }

    #[inline]
    pub fn fire_clflush(&mut self, laddr: u64, paddr: u64) {
        if let Some(t) = self.tracer.as_mut() { t.clflush(laddr, paddr); }
    }

    #[inline]
    pub fn fire_prefetch_hint(&mut self, ev: &PrefetchEvent) {
        if let Some(t) = self.tracer.as_mut() { t.prefetch_hint(ev); }
    }

    #[inline]
    pub fn fire_cpuid(&mut self) {
        if let Some(t) = self.tracer.as_mut() { t.cpuid(); }
    }

    #[inline]
    pub fn fire_wrmsr(&mut self, msr: u32, value: u64) {
        if let Some(t) = self.tracer.as_mut() { t.wrmsr(msr, value); }
    }

    #[inline]
    pub fn fire_vmexit(&mut self, reason: u32, qualification: u64) {
        if let Some(t) = self.tracer.as_mut() { t.vmexit(reason, qualification); }
    }

    #[inline]
    pub fn fire_block_start(&mut self, rip: u64, block_size: u16) {
        if let Some(t) = self.tracer.as_mut() { t.block_start(rip, block_size); }
        #[cfg(feature = "instrumentation")]
        for hook in &mut self.block_hooks {
            if hook.range.contains(rip) {
                (hook.cb)(rip, block_size);
            }
        }
    }

    #[inline]
    pub fn fire_invalid_instruction(&mut self, rip: u64) -> bool {
        if self.tracer.as_mut().map_or(false, |t| t.invalid_instruction(rip)) {
            return true;
        }
        #[cfg(feature = "instrumentation")]
        for hook in &mut self.invalid_insn_hooks {
            if (hook.cb)(rip) {
                return true;
            }
        }
        false
    }

    #[inline]
    pub fn fire_mem_unmapped(&mut self, ev: &MemUnmapped) -> bool {
        if self.tracer.as_mut().map_or(false, |t| t.mem_unmapped(ev)) {
            return true;
        }
        #[cfg(feature = "instrumentation")]
        for hook in &mut self.mem_unmapped_hooks {
            if (hook.cb)(ev.laddr, ev.size, ev.rw) {
                return true;
            }
        }
        false
    }

    #[inline]
    pub fn fire_mem_perm_violation(&mut self, ev: &MemPermViolation) -> bool {
        self.tracer.as_mut().map_or(false, |t| t.mem_perm_violation(ev))
    }
}

impl<T: Instrumentation + Default> InstrumentationRegistry<T> {
    /// Create an empty registry with a default tracer. Zero allocations until
    /// a hook is registered.
    pub fn new() -> Self {
        Self::with_tracer(T::default())
    }
}
