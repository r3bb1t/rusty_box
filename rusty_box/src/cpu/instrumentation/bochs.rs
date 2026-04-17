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
//! ## Callback design (same as BOCHS)
//!
//! **Category 1 — explicit parameters replacing globals.** BOCHS callbacks
//! access `BX_CPU(cpu_id)->field` through globals. Rust has no globals,
//! so the data is passed explicitly:
//! - `before_execution`, `after_execution`, `repeat_iteration`, `opcode`
//!   take `rip: u64` directly (BOCHS reads `prev_rip` from the global).
//! - `opcode` receives `bytes: &[u8]` because the decoded `Instruction`
//!   does not retain the raw opcode bytes.
//!
//! **Category 2 — idiomatic Rust shapes for the same data.** No data
//! is dropped or added; the shape is just clearer:
//! - `tlb_cntrl(what: TlbCntrl)` fuses BOCHS's `(what, new_cr_value)` where
//!   `new_cr_value` was undefined for several `what` variants.
//! - `mwait(..., flags: MwaitFlags)` is a `bitflags` instead of raw `u32`.
//! - `opcode(size: CodeSize)` replaces `(is32: bool, is64: bool)` —
//!   three valid states instead of four with one invalid.
//! - `lin_access/phy_access(memtype: MemType, rw: MemAccessRW)` are
//!   enums for what BOCHS passes as `unsigned` integers.
//!
//! Every callback receives primitives only — never a `CpuSnapshot` or
//! `&CpuState`. Hook implementations that need register values should
//! maintain their own state, or call `Emulator::reg_read` between
//! execution batches. This mirrors BOCHS exactly (their callbacks read
//! registers through globals).

use super::types::{
    BranchType, CacheCntrl, CodeSize, HookMask, MemAccessRW, MemPerms, MemType, MwaitFlags,
    PrefetchHint, ResetType, TlbCntrl,
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

    // ── Lifecycle ───────────────────────────────────────────────────────

    /// Called on CPU reset. BOCHS: `BX_INSTR_RESET(cpu_id, type)`.
    fn reset(&mut self, reset_type: ResetType) {}

    // ── Execution (hot path) ────────────────────────────────────────────

    /// Called before each instruction executes.
    /// BOCHS: `BX_INSTR_BEFORE_EXECUTION(cpu_id, i)`.
    fn before_execution(&mut self, rip: u64, instr: &Instruction) {}

    /// Called after each instruction executes successfully.
    /// BOCHS: `BX_INSTR_AFTER_EXECUTION(cpu_id, i)`.
    fn after_execution(&mut self, rip: u64, instr: &Instruction) {}

    /// Called at the start of each REP/REPE/REPNE iteration.
    /// BOCHS: `BX_INSTR_REPEAT_ITERATION(cpu_id, i)`.
    fn repeat_iteration(&mut self, rip: u64, instr: &Instruction) {}

    /// Called when the decoder produces an opcode.
    /// BOCHS: `BX_INSTR_OPCODE(cpu_id, i, opcode, len, is32, is64)`.
    fn opcode(&mut self, rip: u64, instr: &Instruction, bytes: &[u8], size: CodeSize) {}

    // ── CPU state ───────────────────────────────────────────────────────

    /// Called on HLT instruction. BOCHS: `BX_INSTR_HLT(cpu_id)`.
    fn hlt(&mut self) {}

    /// Called on MWAIT/MWAITX instruction.
    /// BOCHS: `BX_INSTR_MWAIT(cpu_id, addr, len, flags)`.
    fn mwait(&mut self, addr: u64, len: u32, flags: MwaitFlags) {}

    // ── Branches ────────────────────────────────────────────────────────

    /// Conditional near branch taken. BOCHS: `BX_INSTR_CNEAR_BRANCH_TAKEN`.
    fn cnear_branch_taken(&mut self, branch_rip: u64, new_rip: u64) {}

    /// Conditional near branch not taken. BOCHS: `BX_INSTR_CNEAR_BRANCH_NOT_TAKEN`.
    fn cnear_branch_not_taken(&mut self, branch_rip: u64) {}

    /// Unconditional near branch (JMP/CALL/RET). BOCHS: `BX_INSTR_UCNEAR_BRANCH`.
    fn ucnear_branch(&mut self, what: BranchType, branch_rip: u64, new_rip: u64) {}

    /// Far branch (segment change). BOCHS: `BX_INSTR_FAR_BRANCH`.
    fn far_branch(
        &mut self,
        what: BranchType,
        prev_cs: u16,
        prev_rip: u64,
        new_cs: u16,
        new_rip: u64,
    ) {
    }

    // ── Interrupts / Exceptions ─────────────────────────────────────────

    /// Software interrupt delivery. BOCHS: `BX_INSTR_INTERRUPT(cpu_id, vector)`.
    fn interrupt(&mut self, vector: u8) {}

    /// Exception delivery. BOCHS: `BX_INSTR_EXCEPTION(cpu_id, vector, error_code)`.
    fn exception(&mut self, vector: u8, error_code: u32) {}

    /// Hardware interrupt delivery.
    /// BOCHS: `BX_INSTR_HWINTERRUPT(cpu_id, vector, cs, eip)`.
    fn hwinterrupt(&mut self, vector: u8, cs: u16, rip: u64) {}

    // ── Memory ──────────────────────────────────────────────────────────

    /// Linear memory access. BOCHS: `BX_INSTR_LIN_ACCESS`.
    fn lin_access(
        &mut self,
        lin: u64,
        phy: u64,
        len: usize,
        memtype: MemType,
        rw: MemAccessRW,
    ) {
    }

    /// Physical memory access. BOCHS: `BX_INSTR_PHY_ACCESS`.
    /// Matches BOCHS signature — no `lin` parameter.
    fn phy_access(&mut self, phy: u64, len: usize, memtype: MemType, rw: MemAccessRW) {}

    // ── I/O ─────────────────────────────────────────────────────────────

    /// I/O port read before the value is read. BOCHS: `BX_INSTR_INP(addr, len)`.
    fn inp(&mut self, port: u16, len: u8) {}

    /// I/O port read after the value is read. BOCHS: `BX_INSTR_INP2(addr, len, val)`.
    fn inp2(&mut self, port: u16, len: u8, val: u32) {}

    /// I/O port write. BOCHS: `BX_INSTR_OUTP(addr, len, val)`.
    fn outp(&mut self, port: u16, len: u8, val: u32) {}

    // ── TLB / Cache ─────────────────────────────────────────────────────

    /// TLB control operation. BOCHS: `BX_INSTR_TLB_CNTRL(cpu_id, what, new_cr)`.
    fn tlb_cntrl(&mut self, what: TlbCntrl) {}

    /// Cache control (INVD/WBINVD). BOCHS: `BX_INSTR_CACHE_CNTRL(cpu_id, what)`.
    fn cache_cntrl(&mut self, what: CacheCntrl) {}

    /// CLFLUSH instruction. BOCHS: `BX_INSTR_CLFLUSH(cpu_id, laddr, paddr)`.
    fn clflush(&mut self, laddr: u64, paddr: u64) {}

    /// Prefetch hint. BOCHS: `BX_INSTR_PREFETCH_HINT(cpu_id, what, seg, offset)`.
    fn prefetch_hint(&mut self, what: PrefetchHint, seg: u8, offset: u64) {}

    // ── Other ───────────────────────────────────────────────────────────

    /// CPUID instruction. BOCHS: `BX_INSTR_CPUID(cpu_id)`.
    fn cpuid(&mut self) {}

    /// WRMSR instruction. BOCHS: `BX_INSTR_WRMSR(cpu_id, addr, value)`.
    fn wrmsr(&mut self, msr: u32, value: u64) {}

    /// VMX exit event. BOCHS: `BX_INSTR_VMEXIT(cpu_id, reason, qualification)`.
    fn vmexit(&mut self, reason: u32, qualification: u64) {}

    // ── Block ──
    /// Called at start of each basic block.
    fn block_start(&mut self, rip: u64, block_size: u16) {}

    // ── Invalid instruction ──
    /// Called before #UD is raised. Return true to suppress the exception.
    fn invalid_instruction(&mut self, rip: u64) -> bool { false }

    // ── Unmapped memory ──
    /// Called when accessing unmapped memory. Return true to suppress fault.
    fn mem_unmapped(&mut self, laddr: u64, size: usize, rw: MemAccessRW) -> bool { false }

    // ── Permission violation ──
    /// Called on memory permission violation. Return true to suppress fault.
    fn mem_perm_violation(&mut self, laddr: u64, size: usize, rw: MemAccessRW, required: MemPerms) -> bool { false }
}

// ── Unit impl (no-op sentinel) ──────────────────────────────────────────

impl Instrumentation for () {
    fn active_hooks(&self) -> HookMask { HookMask::empty() }
    fn block_start(&mut self, _rip: u64, _block_size: u16) {}
    fn invalid_instruction(&mut self, _rip: u64) -> bool { false }
    fn mem_unmapped(&mut self, _laddr: u64, _size: usize, _rw: MemAccessRW) -> bool { false }
    fn mem_perm_violation(&mut self, _laddr: u64, _size: usize, _rw: MemAccessRW, _required: MemPerms) -> bool { false }
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

            fn opcode(&mut self, rip: u64, instr: &Instruction, bytes: &[u8], size: CodeSize) {
                let ($($T,)+) = self;
                $($T.opcode(rip, instr, bytes, size);)+
            }

            fn hlt(&mut self) {
                let ($($T,)+) = self;
                $($T.hlt();)+
            }

            fn mwait(&mut self, addr: u64, len: u32, flags: MwaitFlags) {
                let ($($T,)+) = self;
                $($T.mwait(addr, len, flags);)+
            }

            fn cnear_branch_taken(&mut self, branch_rip: u64, new_rip: u64) {
                let ($($T,)+) = self;
                $($T.cnear_branch_taken(branch_rip, new_rip);)+
            }

            fn cnear_branch_not_taken(&mut self, branch_rip: u64) {
                let ($($T,)+) = self;
                $($T.cnear_branch_not_taken(branch_rip);)+
            }

            fn ucnear_branch(&mut self, what: BranchType, branch_rip: u64, new_rip: u64) {
                let ($($T,)+) = self;
                $($T.ucnear_branch(what, branch_rip, new_rip);)+
            }

            fn far_branch(
                &mut self,
                what: BranchType,
                prev_cs: u16,
                prev_rip: u64,
                new_cs: u16,
                new_rip: u64,
            ) {
                let ($($T,)+) = self;
                $($T.far_branch(what, prev_cs, prev_rip, new_cs, new_rip);)+
            }

            fn interrupt(&mut self, vector: u8) {
                let ($($T,)+) = self;
                $($T.interrupt(vector);)+
            }

            fn exception(&mut self, vector: u8, error_code: u32) {
                let ($($T,)+) = self;
                $($T.exception(vector, error_code);)+
            }

            fn hwinterrupt(&mut self, vector: u8, cs: u16, rip: u64) {
                let ($($T,)+) = self;
                $($T.hwinterrupt(vector, cs, rip);)+
            }

            fn lin_access(
                &mut self,
                lin: u64,
                phy: u64,
                len: usize,
                memtype: MemType,
                rw: MemAccessRW,
            ) {
                let ($($T,)+) = self;
                $($T.lin_access(lin, phy, len, memtype, rw);)+
            }

            fn phy_access(&mut self, phy: u64, len: usize, memtype: MemType, rw: MemAccessRW) {
                let ($($T,)+) = self;
                $($T.phy_access(phy, len, memtype, rw);)+
            }

            fn inp(&mut self, port: u16, len: u8) {
                let ($($T,)+) = self;
                $($T.inp(port, len);)+
            }

            fn inp2(&mut self, port: u16, len: u8, val: u32) {
                let ($($T,)+) = self;
                $($T.inp2(port, len, val);)+
            }

            fn outp(&mut self, port: u16, len: u8, val: u32) {
                let ($($T,)+) = self;
                $($T.outp(port, len, val);)+
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

            fn prefetch_hint(&mut self, what: PrefetchHint, seg: u8, offset: u64) {
                let ($($T,)+) = self;
                $($T.prefetch_hint(what, seg, offset);)+
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

            fn mem_unmapped(&mut self, laddr: u64, size: usize, rw: MemAccessRW) -> bool {
                let ($($T,)+) = self;
                false $(|| $T.mem_unmapped(laddr, size, rw))+
            }

            fn mem_perm_violation(&mut self, laddr: u64, size: usize, rw: MemAccessRW, required: MemPerms) -> bool {
                let ($($T,)+) = self;
                false $(|| $T.mem_perm_violation(laddr, size, rw, required))+
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
