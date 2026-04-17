//! BOCHS-style instrumentation trait (primary API).
//!
//! Full-fidelity port of the C++ BOCHS instrumentation callbacks
//! (cpp_orig/bochs/instrument/stubs/instrument.h). All methods have
//! default no-op implementations — override only the hooks you need.
//!
//! ## Design differences from BOCHS C++
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
    BranchType, CacheCntrl, CodeSize, MemAccessRW, MemType, MwaitFlags,
    PrefetchHint, ResetType, TlbCntrl,
};
use crate::cpu::decoder::Instruction;

/// BOCHS-compatible instrumentation trait. Implement only the callbacks
/// you need — everything defaults to a no-op.
///
/// # Thread safety
///
/// The `Send` bound lets the trait object move between threads with the
/// `Emulator`. The trait is never shared across threads simultaneously
/// (the emulator is `!Sync` on purpose).
///
/// # Typed access
///
/// The `Any` supertrait lets [`Emulator`](crate::emulator::Emulator) hand
/// you a `&mut YourTracer` after installation — see
/// `Emulator::instrumentation_mut::<T>()`. Zero-cost: monomorphized to a
/// single `TypeId` compare-and-branch, no `unsafe` for the caller, no
/// `Arc<Mutex<...>>` to share state with the outer loop.
#[allow(unused_variables)]
pub trait Instrumentation: core::any::Any + Send {
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
}
