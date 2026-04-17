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
    BranchType, CacheCntrl, CodeSize, HookMask, MemAccessRW, MemPerms, MemType, MwaitFlags, PrefetchHint,
    ResetType, TlbCntrl,
};
#[cfg(feature = "instrumentation")]
use super::types::{BranchEvent, HookHandle, HwInterruptEvent, IoHookEvent, IoHookType, MemHookEvent, MemHookType};

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

    /// Monomorphized tracer — zero-cost when `T = ()`.
    pub(crate) tracer: T,

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
            tracer,
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
        let mut m = self.tracer.active_hooks();

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
        self.tracer.reset(reset_type);
    }

    #[inline]
    pub fn fire_before_execution(&mut self, rip: u64, instr: &Instruction) {
        self.tracer.before_execution(rip, instr);
        #[cfg(feature = "instrumentation")]
        for h in &mut self.code_hooks {
            if h.range.contains(rip) {
                (h.cb)(rip, instr);
            }
        }
    }

    #[inline]
    pub fn fire_after_execution(&mut self, rip: u64, instr: &Instruction) {
        self.tracer.after_execution(rip, instr);
        #[cfg(feature = "instrumentation")]
        for h in &mut self.code_after_hooks {
            if h.range.contains(rip) {
                (h.cb)(rip, instr);
            }
        }
    }

    #[inline]
    pub fn fire_repeat_iteration(&mut self, rip: u64, instr: &Instruction) {
        self.tracer.repeat_iteration(rip, instr);
    }

    #[inline]
    pub fn fire_opcode(&mut self, rip: u64, instr: &Instruction, bytes: &[u8], size: CodeSize) {
        self.tracer.opcode(rip, instr, bytes, size);
    }

    #[inline]
    pub fn fire_hlt(&mut self) {
        self.tracer.hlt();
    }

    #[inline]
    pub fn fire_mwait(&mut self, addr: u64, len: u32, flags: MwaitFlags) {
        self.tracer.mwait(addr, len, flags);
    }

    #[inline]
    pub fn fire_cnear_branch_taken(&mut self, branch_rip: u64, new_rip: u64) {
        self.tracer.cnear_branch_taken(branch_rip, new_rip);
        #[cfg(feature = "instrumentation")]
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
    pub fn fire_cnear_branch_not_taken(&mut self, branch_rip: u64, #[allow(unused)] fallthrough_rip: u64) {
        self.tracer.cnear_branch_not_taken(branch_rip);
        #[cfg(feature = "instrumentation")]
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
        self.tracer.ucnear_branch(what, branch_rip, new_rip);
        #[cfg(feature = "instrumentation")]
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
        self.tracer.far_branch(what, prev_cs, prev_rip, new_cs, new_rip);
        #[cfg(feature = "instrumentation")]
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
        self.tracer.interrupt(vector);
        #[cfg(feature = "instrumentation")]
        for h in &mut self.intr_hooks {
            (h.cb)(vector);
        }
    }

    #[inline]
    pub fn fire_exception(&mut self, vector: u8, error_code: u32) {
        self.tracer.exception(vector, error_code);
        #[cfg(feature = "instrumentation")]
        for h in &mut self.exception_hooks {
            (h.cb)(vector, error_code);
        }
    }

    #[inline]
    pub fn fire_hwinterrupt(&mut self, vector: u8, cs: u16, rip: u64) {
        self.tracer.hwinterrupt(vector, cs, rip);
        #[cfg(feature = "instrumentation")]
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
        self.tracer.lin_access(lin, phy, len, memtype, rw);
        #[cfg(feature = "instrumentation")]
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
        self.tracer.phy_access(phy, len, memtype, rw);
    }

    #[inline]
    pub fn fire_inp(&mut self, port: u16, len: u8) {
        self.tracer.inp(port, len);
    }

    #[inline]
    pub fn fire_inp2(&mut self, port: u16, len: u8, val: u32) {
        self.tracer.inp2(port, len, val);
        #[cfg(feature = "instrumentation")]
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
        self.tracer.outp(port, len, val);
        #[cfg(feature = "instrumentation")]
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
        self.tracer.tlb_cntrl(what);
    }

    #[inline]
    pub fn fire_cache_cntrl(&mut self, what: CacheCntrl) {
        self.tracer.cache_cntrl(what);
    }

    #[inline]
    pub fn fire_clflush(&mut self, laddr: u64, paddr: u64) {
        self.tracer.clflush(laddr, paddr);
    }

    #[inline]
    pub fn fire_prefetch_hint(&mut self, what: PrefetchHint, seg: u8, offset: u64) {
        self.tracer.prefetch_hint(what, seg, offset);
    }

    #[inline]
    pub fn fire_cpuid(&mut self) {
        self.tracer.cpuid();
    }

    #[inline]
    pub fn fire_wrmsr(&mut self, msr: u32, value: u64) {
        self.tracer.wrmsr(msr, value);
    }

    #[inline]
    pub fn fire_vmexit(&mut self, reason: u32, qualification: u64) {
        self.tracer.vmexit(reason, qualification);
    }

    #[inline]
    pub fn fire_block_start(&mut self, rip: u64, block_size: u16) {
        self.tracer.block_start(rip, block_size);
        #[cfg(feature = "instrumentation")]
        for hook in &mut self.block_hooks {
            if hook.range.contains(rip) {
                (hook.cb)(rip, block_size);
            }
        }
    }

    #[inline]
    pub fn fire_invalid_instruction(&mut self, rip: u64) -> bool {
        if self.tracer.invalid_instruction(rip) {
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
    pub fn fire_mem_unmapped(&mut self, laddr: u64, size: usize, rw: MemAccessRW) -> bool {
        if self.tracer.mem_unmapped(laddr, size, rw) {
            return true;
        }
        #[cfg(feature = "instrumentation")]
        for hook in &mut self.mem_unmapped_hooks {
            if (hook.cb)(laddr, size, rw) {
                return true;
            }
        }
        false
    }

    #[inline]
    pub fn fire_mem_perm_violation(&mut self, laddr: u64, size: usize, rw: MemAccessRW, required: MemPerms) -> bool {
        self.tracer.mem_perm_violation(laddr, size, rw, required)
        // No closure hooks for perm violation — trait-only for now
    }
}

impl<T: Instrumentation + Default> InstrumentationRegistry<T> {
    /// Create an empty registry with a default tracer. Zero allocations until
    /// a hook is registered.
    pub fn new() -> Self {
        Self::with_tracer(T::default())
    }
}
