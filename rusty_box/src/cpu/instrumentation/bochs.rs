//! Monomorphized instrumentation trait (primary API).
//!
//! Full-fidelity port of the C++ BOCHS instrumentation callbacks
//! (cpp_orig/bochs/instrument/stubs/instrument.h). All methods have
//! default no-op implementations — override only the hooks you need.
//!
//! ## Design
//!
//! The trait is generic (`T: Instrumentation`) rather than object-safe
//! (`Box<dyn Instrumentation>`). Composition is achieved through tuple
//! types — `(A, B)` implements `Instrumentation` when both `A` and `B`
//! do. No `Any` downcasting, no vtable dispatch on the hot path.
//!
//! `active_hooks()` returns a `HookMask` so the CPU hot path can skip
//! categories with no active hooks (predicted-not-taken branch, zero
//! cost when no instrumentation is attached).
//!
//! ## Callback design
//!
//! **0–2 arg hooks** take positional parameters (`exception(vector, error_code)`,
//! `clflush(laddr, paddr)`, `wrmsr(msr, value)`, …).
//!
//! **3+ arg hooks** take `&Event` structs with named fields:
//! [`OpcodeEvent`](super::types::OpcodeEvent),
//! [`MwaitEvent`](super::types::MwaitEvent),
//! [`HwInterruptEvent`](super::types::HwInterruptEvent),
//! [`LinAccess`](super::types::LinAccess),
//! [`PhyAccess`](super::types::PhyAccess),
//! [`IoHookEvent`](super::types::IoHookEvent),
//! [`PrefetchEvent`](super::types::PrefetchEvent),
//! [`MemUnmapped`](super::types::MemUnmapped),
//! [`MemPermViolation`](super::types::MemPermViolation),
//! [`BranchEvent`](super::types::BranchEvent). Adding a field is not a breaking
//! change, and call sites are self-documenting.
//!
//! **Consolidated branch hook.** BOCHS has four branch callbacks
//! (`cnear_taken`, `cnear_not_taken`, `ucnear`, `far`). They collapse into one
//! `fn branch(&mut self, ev: &BranchEvent)`; the [`BranchEvent`](super::types::BranchEvent)
//! variant carries the distinction.
//!
//! **Memory hooks carry `&[u8]`.** [`LinAccess`](super::types::LinAccess) and
//! [`PhyAccess`](super::types::PhyAccess) expose `data: &[u8]` — length is
//! implicit in the slice, and the actual bytes are available without a second
//! memory read.
//!
//! **Syscall hook is OS-agnostic.** [`pre_syscall`](Instrumentation::pre_syscall)
//! receives [`&mut HookCtx`](super::ctx::HookCtx) and returns
//! [`InstrAction`](super::types::InstrAction). The hook reads whichever
//! registers its target OS convention uses — the library itself assumes
//! nothing about syscall ABIs. `HookCtx` also provides memory r/w and stop.
//!
//! **Idiomatic enums instead of raw ints.** `TlbCntrl`, `CacheCntrl`,
//! `MwaitFlags`, `CodeSize`, `MemType`, `MemAccessRW` — every BOCHS `unsigned`
//! that carried a finite variant set is an enum or bitflags here.

use super::ctx::HookCtx;
use super::types::{
    BranchEvent, CacheCntrl, HookMask, HwInterruptEvent, InstrAction, IoHookEvent, LinAccess,
    MemPermViolation, MemUnmapped, MwaitEvent, OpcodeEvent, PhyAccess, PrefetchEvent, ResetType,
    TlbCntrl,
};
use crate::cpu::decoder::Instruction;

/// Instrumentation trait. Implement only the callbacks you need —
/// everything defaults to a no-op.
///
/// `active_hooks()` declares which hook categories this implementation
/// cares about, enabling the CPU to skip dispatch for inactive
/// categories. The default returns `HookMask::all()` (conservative).
#[allow(unused_variables)]
pub trait Instrumentation {
    /// Declare which hook categories this implementation uses.
    /// The CPU skips dispatch for categories not in the returned mask.
    fn active_hooks(&self) -> HookMask { HookMask::all() }

    // ── Lifecycle ──────────────────────────────────────────────────────────

    /// CPU reset.
    fn reset(&mut self, reset_type: ResetType) {}

    // ── Execution (hot path) ──────────────────────────────────────────────────

    /// Before each instruction executes.
    fn before_execution(&mut self, rip: u64, instr: &Instruction) {}

    /// After each instruction executes successfully.
    fn after_execution(&mut self, rip: u64, instr: &Instruction) {}

    /// Start of each REP / REPE / REPNE iteration.
    fn repeat_iteration(&mut self, rip: u64, instr: &Instruction) {}

    /// The decoder produced an instruction. `ev.bytes` is the raw opcode
    /// as it appeared in memory; `ev.instr` is the decoded form.
    fn opcode(&mut self, ev: &OpcodeEvent) {}

    // ── CPU state ─────────────────────────────────────────────────────────

    /// HLT instruction.
    fn hlt(&mut self) {}

    /// MWAIT / MWAITX.
    fn mwait(&mut self, ev: &MwaitEvent) {}

    // ── Branch (unified) ─────────────────────────────────────────────────

    /// Any branch (conditional near, unconditional near, or far). The
    /// variant of `BranchEvent` tells them apart — match on it if you only
    /// care about one kind.
    fn branch(&mut self, ev: &BranchEvent) {}

    // ── Syscall (can alter architectural effects) ───────────────────────────────────

    /// Fires on SYSCALL / SYSENTER just before the architectural CS/RIP
    /// transition. `ctx` exposes full CPU access (register r/w, memory r/w,
    /// stop). OS-agnostic — the hook reads whichever registers its target
    /// OS convention uses.
    ///
    /// Returns an [`InstrAction`] controlling what happens next:
    /// - [`InstrAction::Continue`]: architectural SYSCALL proceeds.
    /// - [`InstrAction::Skip`]: skip the transition; RIP advances past the
    ///   opcode only.
    /// - [`InstrAction::Stop`]: transition runs, then CPU stops.
    /// - [`InstrAction::SkipAndStop`]: both.
    fn pre_syscall(&mut self, ctx: &mut HookCtx) -> InstrAction {
        let _ = ctx;
        InstrAction::Continue
    }

    // ── Interrupts / Exceptions ────────────────────────────────────────────────

    /// Software interrupt (INT n).
    fn interrupt(&mut self, vector: u8) {}

    /// Exception delivery.
    fn exception(&mut self, vector: u8, error_code: u32) {}

    /// Hardware interrupt delivery.
    fn hwinterrupt(&mut self, ev: &HwInterruptEvent) {}

    // ── Memory ─────────────────────────────────────────────────────────────

    /// Linear memory access. `ev.data` is the actual bytes touched.
    fn lin_access(&mut self, ev: &LinAccess) {}

    /// Physical memory access (page-table walks etc.). `ev.data` is the
    /// actual bytes touched.
    fn phy_access(&mut self, ev: &PhyAccess) {}

    // ── I/O ─────────────────────────────────────────────────────────────────

    /// I/O port read — fires BEFORE the read, value is unknown.
    fn inp(&mut self, port: u16, size: u8) {}

    /// I/O port read — fires AFTER the read, value is known.
    fn inp2(&mut self, ev: &IoHookEvent) {}

    /// I/O port write.
    fn outp(&mut self, ev: &IoHookEvent) {}

    // ── TLB / Cache ────────────────────────────────────────────────────────

    /// TLB control operation (MOV to CR0/CR3/CR4, task/context switch, INVLPG...).
    fn tlb_cntrl(&mut self, what: TlbCntrl) {}

    /// Cache control (INVD / WBINVD).
    fn cache_cntrl(&mut self, what: CacheCntrl) {}

    /// CLFLUSH instruction.
    fn clflush(&mut self, laddr: u64, paddr: u64) {}

    /// Prefetch hint.
    fn prefetch_hint(&mut self, ev: &PrefetchEvent) {}

    // ── Other ───────────────────────────────────────────────────────────────────

    /// CPUID instruction.
    fn cpuid(&mut self) {}

    /// WRMSR instruction.
    fn wrmsr(&mut self, msr: u32, value: u64) {}

    /// VMX exit.
    fn vmexit(&mut self, reason: u32, qualification: u64) {}

    // ── Unicorn-inspired hooks ────────────────────────────────────────────────

    /// Start of a basic block (trace).
    fn block_start(&mut self, rip: u64, block_size: u16) {}

    /// Before an undefined/unrecognized instruction raises #UD. Return
    /// `true` to suppress.
    fn invalid_instruction(&mut self, rip: u64) -> bool { false }

    /// Access to a not-present page. Return `true` to suppress the fault.
    fn mem_unmapped(&mut self, ev: &MemUnmapped) -> bool { false }

    /// Access denied by the `PagePermissions` bitmap. Return `true` to
    /// suppress the fault.
    fn mem_perm_violation(&mut self, ev: &MemPermViolation) -> bool { false }
}

// ── Unit impl (no-op sentinel) ───────────────────────────────────────────────

impl Instrumentation for () {
    /// Zero-cost no-op observer — tell the CPU to skip every dispatch.
    fn active_hooks(&self) -> HookMask { HookMask::empty() }
}

// ── Tuple composition ───────────────────────────────────────────────────

macro_rules! impl_instrumentation_tuple {
    ($($T:ident),+) => {
        #[allow(non_snake_case)]
        impl<$($T: Instrumentation),+> Instrumentation for ($($T,)+) {
            fn active_hooks(&self) -> HookMask {
                let ($($T,)+) = self;
                HookMask::empty() $(| $T.active_hooks())+
            }

            fn reset(&mut self, reset_type: ResetType) {
                let ($($T,)+) = self;
                $($T.reset(reset_type);)+
            }

            fn before_execution(&mut self, rip: u64, instr: &Instruction) {
                let ($($T,)+) = self;
                $($T.before_execution(rip, instr);)+
            }

            fn after_execution(&mut self, rip: u64, instr: &Instruction) {
                let ($($T,)+) = self;
                $($T.after_execution(rip, instr);)+
            }

            fn repeat_iteration(&mut self, rip: u64, instr: &Instruction) {
                let ($($T,)+) = self;
                $($T.repeat_iteration(rip, instr);)+
            }

            fn opcode(&mut self, ev: &OpcodeEvent) {
                let ($($T,)+) = self;
                $($T.opcode(ev);)+
            }

            fn hlt(&mut self) {
                let ($($T,)+) = self;
                $($T.hlt();)+
            }

            fn mwait(&mut self, ev: &MwaitEvent) {
                let ($($T,)+) = self;
                $($T.mwait(ev);)+
            }

            fn branch(&mut self, ev: &BranchEvent) {
                let ($($T,)+) = self;
                $($T.branch(ev);)+
            }

            fn pre_syscall(&mut self, ctx: &mut HookCtx) -> InstrAction {
                let ($($T,)+) = self;
                let mut action = InstrAction::Continue;
                $( action = action.combine($T.pre_syscall(ctx)); )+
                action
            }

            fn interrupt(&mut self, vector: u8) {
                let ($($T,)+) = self;
                $($T.interrupt(vector);)+
            }

            fn exception(&mut self, vector: u8, error_code: u32) {
                let ($($T,)+) = self;
                $($T.exception(vector, error_code);)+
            }

            fn hwinterrupt(&mut self, ev: &HwInterruptEvent) {
                let ($($T,)+) = self;
                $($T.hwinterrupt(ev);)+
            }

            fn lin_access(&mut self, ev: &LinAccess) {
                let ($($T,)+) = self;
                $($T.lin_access(ev);)+
            }

            fn phy_access(&mut self, ev: &PhyAccess) {
                let ($($T,)+) = self;
                $($T.phy_access(ev);)+
            }

            fn inp(&mut self, port: u16, size: u8) {
                let ($($T,)+) = self;
                $($T.inp(port, size);)+
            }

            fn inp2(&mut self, ev: &IoHookEvent) {
                let ($($T,)+) = self;
                $($T.inp2(ev);)+
            }

            fn outp(&mut self, ev: &IoHookEvent) {
                let ($($T,)+) = self;
                $($T.outp(ev);)+
            }

            fn tlb_cntrl(&mut self, what: TlbCntrl) {
                let ($($T,)+) = self;
                $($T.tlb_cntrl(what);)+
            }

            fn cache_cntrl(&mut self, what: CacheCntrl) {
                let ($($T,)+) = self;
                $($T.cache_cntrl(what);)+
            }

            fn clflush(&mut self, laddr: u64, paddr: u64) {
                let ($($T,)+) = self;
                $($T.clflush(laddr, paddr);)+
            }

            fn prefetch_hint(&mut self, ev: &PrefetchEvent) {
                let ($($T,)+) = self;
                $($T.prefetch_hint(ev);)+
            }

            fn cpuid(&mut self) {
                let ($($T,)+) = self;
                $($T.cpuid();)+
            }

            fn wrmsr(&mut self, msr: u32, value: u64) {
                let ($($T,)+) = self;
                $($T.wrmsr(msr, value);)+
            }

            fn vmexit(&mut self, reason: u32, qualification: u64) {
                let ($($T,)+) = self;
                $($T.vmexit(reason, qualification);)+
            }

            fn block_start(&mut self, rip: u64, block_size: u16) {
                let ($($T,)+) = self;
                $($T.block_start(rip, block_size);)+
            }

            fn invalid_instruction(&mut self, rip: u64) -> bool {
                let ($($T,)+) = self;
                false $(|| $T.invalid_instruction(rip))+
            }

            fn mem_unmapped(&mut self, ev: &MemUnmapped) -> bool {
                let ($($T,)+) = self;
                false $(|| $T.mem_unmapped(ev))+
            }

            fn mem_perm_violation(&mut self, ev: &MemPermViolation) -> bool {
                let ($($T,)+) = self;
                false $(|| $T.mem_perm_violation(ev))+
            }
        }
    }
}

impl_instrumentation_tuple!(A);
impl_instrumentation_tuple!(A, B);
impl_instrumentation_tuple!(A, B, C);
impl_instrumentation_tuple!(A, B, C, D);
impl_instrumentation_tuple!(A, B, C, D, E);
impl_instrumentation_tuple!(A, B, C, D, E, F);
impl_instrumentation_tuple!(A, B, C, D, E, F, G);
impl_instrumentation_tuple!(A, B, C, D, E, F, G, H);
