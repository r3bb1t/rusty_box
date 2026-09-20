//! The `InstrumentationRegistry` — where a processor keeps its tracer.
//!
//! One lives inside every [`BxCpuC`](crate::cpu::BxCpuC), holding the tracer
//! the machine was built with, the [`HookMask`] it declared, and the stop
//! request a hook can raise. The CPU fires events through it; nothing is
//! registered at run time, because a tracer is the machine's type parameter.
//!
//! ## Hot path contract
//!
//! Every `fire_*` method:
//! 1. Is `#[inline]`, so the bitmask test in `has_*()` can be hoisted by LLVM
//!    and combined with the outer callsite guard.
//! 2. Calls the tracer directly — monomorphized, so `T = ()` compiles to
//!    nothing at all.
//! 3. Does not allocate.
//!
//! The outer callsite pattern is:
//! ```ignore
//! if self.instrumentation.active.has_exec() {
//!     self.instrumentation.fire_before_execution(rip, instr);
//! }
//! ```
//!
//! The guard is not optional: `active` is what a tracer's `active_hooks()`
//! answered, and a call site that skipped it would run the hook for a tracer
//! that asked not to see it.

use crate::cpu::decoder::Instruction;

use super::bochs::Instrumentation;
use super::types::{
    BranchEvent, CacheCntrl, HookMask, HwInterruptEvent, IoHookEvent, LinAccess, MemPermViolation,
    MemUnmapped, MwaitEvent, OpcodeEvent, PhyAccess, PrefetchEvent, ResetType, TlbCntrl,
};

/// The tracer a processor carries, with the mask it declared and the stop
/// request its hooks can raise.
pub struct InstrumentationRegistry<T: Instrumentation = ()> {
    /// The hook categories the tracer declared in `active_hooks`. Callers
    /// check this before invoking `fire_*` to keep the hot path empty.
    pub active: HookMask,

    /// Cooperative stop request. When a hook sets this to `true`, the CPU
    /// loop exits at the next trace boundary. This is the Rust analogue of
    /// Bochs's `bx_pc_system.kill_bochs_request`, scoped to instrumentation so
    /// hooks can stop execution without global state. Single-threaded — plain
    /// `bool`, no atomic.
    ///
    /// This is the inbound half: the CPU loop consumes it, so a request never
    /// outlives the slice that honours it and cannot starve a processor that
    /// keeps re-entering. What the machine above learns is [`Self::stop_honored`].
    pub stop_request: bool,

    /// The outbound half of [`Self::stop_request`]: set by the CPU loop at the
    /// moment it honours a request, so the machine driving the loop can see
    /// that a hook — not a budget — ended the slice.
    ///
    /// Two fields rather than one because they travel in opposite directions.
    /// A single self-clearing flag would tell the CPU to stop and tell the
    /// machine nothing, so `Emulator::step` would re-enter the loop and lose
    /// the stop; a single sticky flag would stop the machine but re-break
    /// every slice of any processor whose flag no one had cleared. The
    /// machine drains this one at the end of each batch and raises its own
    /// stop flag, which is what every run loop already honours.
    pub(crate) stop_honored: bool,

    /// Monomorphized tracer — zero-cost when `T = ()`.
    ///
    /// Always a tracer. A hook that needs `&mut HookCtx` moves this one out
    /// for the duration of the call and puts it back after, leaving a default
    /// in the slot meanwhile — which is why [`Instrumentation`] requires
    /// `Default`. A hook fired re-entrantly during that window therefore
    /// reaches a throwaway tracer whose effects are dropped, the same nothing
    /// that an empty slot produced, without anyone having to ask whether the
    /// slot is empty.
    pub(crate) tracer: T,
}

impl<T: Instrumentation + Default> Default for InstrumentationRegistry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Instrumentation> InstrumentationRegistry<T> {
    /// Hold the given tracer, and take the hook mask it declares.
    pub fn with_tracer(tracer: T) -> Self {
        let mut reg = Self {
            active: HookMask::empty(),
            stop_request: false,
            stop_honored: false,
            tracer,
        };
        reg.refresh_active();
        reg
    }

    /// Re-ask the tracer which hook categories it wants. A tracer whose
    /// `active_hooks()` answer depends on its own state must be followed by
    /// this call, or the CPU keeps skipping a category it has started caring
    /// about.
    pub fn refresh_active(&mut self) {
        self.active = self.tracer.active_hooks();
    }

    // ─────────────────── Fire methods (hot path) ───────────────────
    //
    // Each `fire_*` is called at every matching CPU event when its HookMask
    // bit is set. The outer guard in CPU code must check the mask first —
    // these methods assume the tracer is interested.

    #[inline]
    pub fn fire_reset(&mut self, reset_type: ResetType) {
        self.tracer.reset(reset_type);
    }

    #[inline]
    pub fn fire_before_execution(&mut self, rip: u64, instr: &Instruction) {
        self.tracer.before_execution(rip, instr);
    }

    #[inline]
    pub fn fire_after_execution(&mut self, rip: u64, instr: &Instruction) {
        self.tracer.after_execution(rip, instr);
    }

    #[inline]
    pub fn fire_repeat_iteration(&mut self, rip: u64, instr: &Instruction) {
        self.tracer.repeat_iteration(rip, instr);
    }

    #[inline]
    pub fn fire_opcode(&mut self, ev: &OpcodeEvent) {
        self.tracer.opcode(ev);
    }

    #[inline]
    pub fn fire_hlt(&mut self) {
        self.tracer.hlt();
    }

    #[inline]
    pub fn fire_mwait(&mut self, ev: &MwaitEvent) {
        self.tracer.mwait(ev);
    }

    /// Unified branch-event fire. Replaces the 4 Bochs-style callbacks
    /// (cnear_taken/not_taken, ucnear, far) — callers construct the
    /// appropriate `BranchEvent` variant at the callsite.
    #[inline]
    pub fn fire_branch(&mut self, ev: &BranchEvent) {
        self.tracer.branch(ev);
    }

    #[inline]
    pub fn fire_interrupt(&mut self, vector: u8) {
        self.tracer.interrupt(vector);
    }

    #[inline]
    pub fn fire_exception(&mut self, vector: u8, error_code: u32) {
        self.tracer.exception(vector, error_code);
    }

    #[inline]
    pub fn fire_hwinterrupt(&mut self, ev: &HwInterruptEvent) {
        self.tracer.hwinterrupt(ev);
    }

    #[inline]
    pub fn fire_lin_access(&mut self, ev: &LinAccess) {
        self.tracer.lin_access(ev);
    }

    #[inline]
    pub fn fire_phy_access(&mut self, ev: &PhyAccess) {
        self.tracer.phy_access(ev);
    }

    #[inline]
    pub fn fire_inp(&mut self, port: u16, size: u8) {
        self.tracer.inp(port, size);
    }

    #[inline]
    pub fn fire_inp2(&mut self, ev: &IoHookEvent) {
        self.tracer.inp2(ev);
    }

    #[inline]
    pub fn fire_outp(&mut self, ev: &IoHookEvent) {
        self.tracer.outp(ev);
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
    pub fn fire_prefetch_hint(&mut self, ev: &PrefetchEvent) {
        self.tracer.prefetch_hint(ev);
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
    }

    #[inline]
    pub fn fire_invalid_instruction(&mut self, rip: u64) -> bool {
        if self.tracer.invalid_instruction(rip) {
            return true;
        }
        false
    }

    #[inline]
    pub fn fire_mem_unmapped(&mut self, ev: &MemUnmapped) -> bool {
        if self.tracer.mem_unmapped(ev) {
            return true;
        }
        false
    }

    #[inline]
    pub fn fire_mem_perm_violation(&mut self, ev: &MemPermViolation) -> bool {
        self.tracer.mem_perm_violation(ev)
    }

    /// Hold a default-constructed tracer.
    pub fn new() -> Self {
        Self::with_tracer(T::default())
    }
}

impl InstrumentationRegistry<()> {
    /// A power-on registry for the no-op tracer, constructible in a const
    /// context.
    ///
    /// Scoped to `T = ()` on purpose. Static (`.bss`) placement is the
    /// no_alloc path, and a statically placed CPU carries the unit tracer.
    /// Making this generic would mean requiring `const INIT: Self` from every
    /// `Instrumentation` implementor, which a tracer holding a `String` could
    /// not supply.
    ///
    /// `with_tracer` cannot be const: it ends by calling `refresh_active`,
    /// which asks the tracer through a trait method. For the unit tracer that
    /// answer is statically `HookMask::empty()`, which is what makes the mask
    /// below correct rather than merely plausible — a test pins it against the
    /// runtime constructor.
    pub const fn const_new() -> Self {
        Self {
            active: HookMask::empty(),
            stop_request: false,
            stop_honored: false,
            tracer: (),
        }
    }
}

#[cfg(test)]
mod const_constructor_tests {
    use super::*;

    /// `const_new` hand-writes the mask that `with_tracer` derives by asking
    /// the tracer at run time. For the unit tracer that answer is statically
    /// empty, but nothing in the type system says so — if `()` ever gained a
    /// hook, the const would silently disagree with every other construction
    /// path and the CPU would skip dispatches it should make.
    #[test]
    fn const_new_matches_the_runtime_constructor() {
        let runtime: InstrumentationRegistry<()> = InstrumentationRegistry::new();
        let constructed = InstrumentationRegistry::<()>::const_new();

        assert_eq!(constructed.active, runtime.active);
        assert_eq!(constructed.stop_request, runtime.stop_request);
        assert_eq!(constructed.tracer, runtime.tracer);

    }
}
