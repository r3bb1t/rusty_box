//! Public API extensions for [`Emulator`]: Unicorn-style hook registration,
//! register/memory read-write, `emu_start`/`emu_stop` execution control, and
//! direct-binary CPU-mode builders.
//!
//! This module lives in its own file to keep the core orchestration in
//! `emulator.rs` readable. Everything here operates purely on the public
//! `&mut Emulator` surface.

use alloc::{boxed::Box, vec::Vec};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "instrumentation")]
use core::ops::RangeBounds;

#[cfg(feature = "instrumentation")]
use crate::cpu::decoder::Instruction;
#[cfg(feature = "instrumentation")]
use crate::cpu::instrumentation::{
    BranchEvent, HookHandle, HwInterruptEvent, InstrumentationError, IoHookEvent, IoHookType,
    MemHookEvent, MemHookType,
};
use crate::cpu::instrumentation::{CpuSetupMode, CpuSnapshot, EmuStopReason, X86Reg};
use crate::cpu::{BxCpuIdTrait, ResetReason};
use crate::emulator::{Emulator, EmulatorConfig};
use crate::{Error, Result};


// ─────────────────────────── StopHandle ───────────────────────────

/// Clonable cross-thread handle that stops a running [`Emulator`].
///
/// Obtain one with [`Emulator::stop_handle`] on the owning thread, move the
/// clone to another thread, and call [`StopHandle::stop`] to break the next
/// batch boundary of `emu_start`.
///
/// Backed by `Arc<AtomicBool>` with `Ordering::Relaxed` — single mov on x86
/// with no fence. See the plan's "Atomic Performance Analysis" section.
#[derive(Clone)]
pub struct StopHandle(pub(crate) Arc<AtomicBool>);

impl StopHandle {
    /// Signal the Emulator to stop at the next batch boundary. Non-blocking.
    #[inline]
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Clear the stop signal (rare — intended for scenarios where the same
    /// Emulator is reused after a stop).
    #[inline]
    pub fn reset(&self) {
        self.0.store(false, Ordering::Relaxed);
    }

    /// True once `stop()` has been called and the flag hasn't been cleared.
    #[inline]
    pub fn is_stopping(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

// ─────────────────────────── Hook registration ───────────────────────────
//
// All hook_add_* methods require the `instrumentation` feature because they
// populate the [`InstrumentationRegistry`] on the CPU, which is itself
// feature-gated. When the feature is off, the methods simply do not exist.

#[cfg(feature = "instrumentation")]
impl<'a, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> Emulator<'a, I, T> {
    /// Register a hook fired before each instruction whose RIP is in `range`.
    /// Callback receives `(rip, &Instruction)`.
    pub fn hook_add_code<R, F>(&mut self, range: R, cb: F) -> HookHandle
    where
        R: RangeBounds<u64>,
        F: FnMut(u64, &Instruction) + Send + 'static,
    {
        self.cpu.instrumentation.add_code(range, Box::new(cb))
    }

    /// Register a hook fired AFTER each instruction whose RIP is in `range`.
    pub fn hook_add_code_after<R, F>(&mut self, range: R, cb: F) -> HookHandle
    where
        R: RangeBounds<u64>,
        F: FnMut(u64, &Instruction) + Send + 'static,
    {
        self.cpu.instrumentation.add_code_after(range, Box::new(cb))
    }

    /// Register a memory access hook.
    pub fn hook_add_mem<R, F>(
        &mut self,
        hook_type: MemHookType,
        range: R,
        cb: F,
    ) -> HookHandle
    where
        R: RangeBounds<u64>,
        F: FnMut(&MemHookEvent) + Send + 'static,
    {
        self.cpu
            .instrumentation
            .add_mem(hook_type, range, Box::new(cb))
    }

    /// Register a software-interrupt hook (INT n / INT3 / INTO).
    /// Callback receives the vector.
    pub fn hook_add_interrupt<F>(&mut self, cb: F) -> HookHandle
    where
        F: FnMut(u8) + Send + 'static,
    {
        self.cpu.instrumentation.add_interrupt(Box::new(cb))
    }

    /// Register a hardware-interrupt hook (external IRQ delivery).
    pub fn hook_add_hwinterrupt<F>(&mut self, cb: F) -> HookHandle
    where
        F: FnMut(&HwInterruptEvent) + Send + 'static,
    {
        self.cpu.instrumentation.add_hw_interrupt(Box::new(cb))
    }

    /// Register a CPU-exception hook.
    /// Callback receives `(vector, error_code)`.
    pub fn hook_add_exception<F>(&mut self, cb: F) -> HookHandle
    where
        F: FnMut(u8, u32) + Send + 'static,
    {
        self.cpu.instrumentation.add_exception(Box::new(cb))
    }

    /// Register an I/O port hook (IN/OUT instructions).
    pub fn hook_add_io<R, F>(
        &mut self,
        hook_type: IoHookType,
        range: R,
        cb: F,
    ) -> HookHandle
    where
        R: RangeBounds<u16>,
        F: FnMut(&IoHookEvent) + Send + 'static,
    {
        self.cpu
            .instrumentation
            .add_io(hook_type, range, Box::new(cb))
    }

    /// Register a branch hook. Fires for conditional, unconditional, and
    /// far branches; the variant in [`BranchEvent`] tells them apart.
    pub fn hook_add_branch<R, F>(&mut self, range: R, cb: F) -> HookHandle
    where
        R: RangeBounds<u64>,
        F: FnMut(&BranchEvent) + Send + 'static,
    {
        self.cpu.instrumentation.add_branch(range, Box::new(cb))
    }

    /// Register a block hook. Fires at the start of each basic block (trace)
    /// whose RIP is in range.
    pub fn hook_add_block<R, F>(&mut self, range: R, cb: F) -> HookHandle
    where
        R: RangeBounds<u64>,
        F: FnMut(u64, u16) + Send + 'static,
    {
        self.cpu.instrumentation.add_block(range, Box::new(cb))
    }

    /// Register an invalid-instruction hook. Fires before #UD for
    /// unrecognized opcodes. Return `true` from the callback to suppress
    /// the exception.
    pub fn hook_add_invalid_insn<F>(&mut self, cb: F) -> HookHandle
    where
        F: FnMut(u64) -> bool + Send + 'static,
    {
        self.cpu.instrumentation.add_invalid_insn(Box::new(cb))
    }

    /// Register an unmapped-memory hook. Fires before page fault for
    /// not-present pages. Return `true` to suppress the fault.
    pub fn hook_add_mem_unmapped<F>(&mut self, cb: F) -> HookHandle
    where
        F: FnMut(u64, usize, crate::cpu::instrumentation::MemAccessRW) -> bool + Send + 'static,
    {
        self.cpu.instrumentation.add_mem_unmapped(Box::new(cb))
    }

    /// Remove a previously registered hook.
    /// Returns `Err(InvalidHandle)` if the handle was already removed or
    /// never valid.
    pub fn hook_del(&mut self, handle: HookHandle) -> core::result::Result<(), InstrumentationError> {
        self.cpu.instrumentation.remove(handle)
    }

    /// Direct typed reference to the installed tracer. Zero-cost field access.
    pub fn instrumentation(&self) -> &T {
        &self.cpu.instrumentation.tracer
    }

    /// Mutable reference to the installed tracer. Zero-cost field access.
    pub fn instrumentation_mut(&mut self) -> &mut T {
        &mut self.cpu.instrumentation.tracer
    }

    /// Recompute the active hook mask from the tracer's `active_hooks()`.
    /// Call this after mutating tracer state that changes which categories are active.
    pub fn refresh_hook_mask(&mut self) {
        self.cpu.instrumentation.refresh_active();
    }
}

// ─────────────────────────── reg_read / reg_write ───────────────────────────

impl<'a, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> Emulator<'a, I, T> {
    /// Read any register by enum tag. Narrower registers are zero-extended
    /// into the returned `u64`.
    pub fn reg_read(&self, reg: X86Reg) -> u64 {
        let cpu = &self.cpu;
        let v: u64 = match reg {
            X86Reg::Rax => cpu.rax(),
            X86Reg::Rcx => cpu.rcx(),
            X86Reg::Rdx => cpu.rdx(),
            X86Reg::Rbx => cpu.rbx(),
            X86Reg::Rsp => cpu.rsp(),
            X86Reg::Rbp => cpu.rbp(),
            X86Reg::Rsi => cpu.rsi(),
            X86Reg::Rdi => cpu.rdi(),
            X86Reg::R8 => cpu.r8(),
            X86Reg::R9 => cpu.r9(),
            X86Reg::R10 => cpu.r10(),
            X86Reg::R11 => cpu.r11(),
            X86Reg::R12 => cpu.r12(),
            X86Reg::R13 => cpu.r13(),
            X86Reg::R14 => cpu.r14(),
            X86Reg::R15 => cpu.r15(),

            X86Reg::Eax => cpu.rax() as u32 as u64,
            X86Reg::Ecx => cpu.rcx() as u32 as u64,
            X86Reg::Edx => cpu.rdx() as u32 as u64,
            X86Reg::Ebx => cpu.rbx() as u32 as u64,
            X86Reg::Esp => cpu.rsp() as u32 as u64,
            X86Reg::Ebp => cpu.rbp() as u32 as u64,
            X86Reg::Esi => cpu.rsi() as u32 as u64,
            X86Reg::Edi => cpu.rdi() as u32 as u64,
            X86Reg::R8d => cpu.r8() as u32 as u64,
            X86Reg::R9d => cpu.r9() as u32 as u64,
            X86Reg::R10d => cpu.r10() as u32 as u64,
            X86Reg::R11d => cpu.r11() as u32 as u64,
            X86Reg::R12d => cpu.r12() as u32 as u64,
            X86Reg::R13d => cpu.r13() as u32 as u64,
            X86Reg::R14d => cpu.r14() as u32 as u64,
            X86Reg::R15d => cpu.r15() as u32 as u64,

            X86Reg::Ax => cpu.get_gpr16(0) as u64,
            X86Reg::Cx => cpu.get_gpr16(1) as u64,
            X86Reg::Dx => cpu.get_gpr16(2) as u64,
            X86Reg::Bx => cpu.get_gpr16(3) as u64,
            X86Reg::Sp => cpu.get_gpr16(4) as u64,
            X86Reg::Bp => cpu.get_gpr16(5) as u64,
            X86Reg::Si => cpu.get_gpr16(6) as u64,
            X86Reg::Di => cpu.get_gpr16(7) as u64,
            X86Reg::R8w => cpu.get_gpr16(8) as u64,
            X86Reg::R9w => cpu.get_gpr16(9) as u64,
            X86Reg::R10w => cpu.get_gpr16(10) as u64,
            X86Reg::R11w => cpu.get_gpr16(11) as u64,
            X86Reg::R12w => cpu.get_gpr16(12) as u64,
            X86Reg::R13w => cpu.get_gpr16(13) as u64,
            X86Reg::R14w => cpu.get_gpr16(14) as u64,
            X86Reg::R15w => cpu.get_gpr16(15) as u64,

            X86Reg::Al => cpu.get_gpr8(0) as u64,
            X86Reg::Cl => cpu.get_gpr8(1) as u64,
            X86Reg::Dl => cpu.get_gpr8(2) as u64,
            X86Reg::Bl => cpu.get_gpr8(3) as u64,
            X86Reg::Ah => cpu.get_gpr8(4) as u64,
            X86Reg::Ch => cpu.get_gpr8(5) as u64,
            X86Reg::Dh => cpu.get_gpr8(6) as u64,
            X86Reg::Bh => cpu.get_gpr8(7) as u64,
            X86Reg::Spl => cpu.get_gpr8(4) as u64,
            X86Reg::Bpl => cpu.get_gpr8(5) as u64,
            X86Reg::Sil => cpu.get_gpr8(6) as u64,
            X86Reg::Dil => cpu.get_gpr8(7) as u64,
            X86Reg::R8b => cpu.get_gpr8(8) as u64,
            X86Reg::R9b => cpu.get_gpr8(9) as u64,
            X86Reg::R10b => cpu.get_gpr8(10) as u64,
            X86Reg::R11b => cpu.get_gpr8(11) as u64,
            X86Reg::R12b => cpu.get_gpr8(12) as u64,
            X86Reg::R13b => cpu.get_gpr8(13) as u64,
            X86Reg::R14b => cpu.get_gpr8(14) as u64,
            X86Reg::R15b => cpu.get_gpr8(15) as u64,

            X86Reg::Rip => cpu.rip(),
            X86Reg::Eip => cpu.eip() as u64,
            X86Reg::Ip => (cpu.rip() as u16) as u64,

            X86Reg::Rflags => cpu.rflags_for_api(),
            X86Reg::Eflags => (cpu.rflags_for_api() as u32) as u64,
            X86Reg::Flags => (cpu.rflags_for_api() as u16) as u64,

            X86Reg::Cs => cpu.get_cs_selector() as u64,
            X86Reg::Ss => cpu.get_ss_selector() as u64,
            X86Reg::Ds => cpu.get_ds_selector() as u64,
            X86Reg::Es => cpu.seg_selector_for_api(0) as u64, // ES
            X86Reg::Fs => cpu.seg_selector_for_api(4) as u64, // FS
            X86Reg::Gs => cpu.seg_selector_for_api(5) as u64, // GS

            X86Reg::FsBase => cpu.msr_fsbase(),
            X86Reg::GsBase => cpu.msr_gsbase(),

            X86Reg::Cr0 => cpu.get_cr0_val() as u64,
            X86Reg::Cr2 => cpu.cr2_for_api(),
            X86Reg::Cr3 => cpu.get_cr3_val(),
            X86Reg::Cr4 => cpu.cr4_for_api(),
            X86Reg::Cr8 => cpu.cr8_for_api(),

            X86Reg::Dr0 => cpu.dr_for_api(0),
            X86Reg::Dr1 => cpu.dr_for_api(1),
            X86Reg::Dr2 => cpu.dr_for_api(2),
            X86Reg::Dr3 => cpu.dr_for_api(3),
            X86Reg::Dr6 => cpu.dr6_for_api(),
            X86Reg::Dr7 => cpu.dr7_for_api(),

            X86Reg::GdtrBase => cpu.gdtr_base_for_api(),
            X86Reg::GdtrLimit => cpu.gdtr_limit_for_api(),
            X86Reg::IdtrBase => cpu.idtr_base_for_api(),
            X86Reg::IdtrLimit => cpu.idtr_limit_for_api(),
            X86Reg::LdtrSelector => cpu.ldtr_selector_for_api() as u64,
            X86Reg::TrSelector => cpu.tr_selector_for_api() as u64,

            X86Reg::Tsc => cpu.tsc_for_api(),
            X86Reg::Efer => cpu.efer_for_api(),

            // FPU scalar registers
            X86Reg::FpSw => cpu.fpu_sw_for_api() as u64,
            X86Reg::FpCw => cpu.fpu_cw_for_api() as u64,
            X86Reg::FpTag => cpu.fpu_tag_for_api() as u64,
            X86Reg::Mxcsr => cpu.mxcsr_for_api() as u64,
            X86Reg::Opmask0 => cpu.opmask_read_for_api(0),
            X86Reg::Opmask1 => cpu.opmask_read_for_api(1),
            X86Reg::Opmask2 => cpu.opmask_read_for_api(2),
            X86Reg::Opmask3 => cpu.opmask_read_for_api(3),
            X86Reg::Opmask4 => cpu.opmask_read_for_api(4),
            X86Reg::Opmask5 => cpu.opmask_read_for_api(5),
            X86Reg::Opmask6 => cpu.opmask_read_for_api(6),
            X86Reg::Opmask7 => cpu.opmask_read_for_api(7),

            // Wide registers handled by dedicated methods — return 0 from scalar path
            X86Reg::Fpr0 | X86Reg::Fpr1 | X86Reg::Fpr2 | X86Reg::Fpr3
            | X86Reg::Fpr4 | X86Reg::Fpr5 | X86Reg::Fpr6 | X86Reg::Fpr7
            | X86Reg::Xmm0 | X86Reg::Xmm1 | X86Reg::Xmm2 | X86Reg::Xmm3
            | X86Reg::Xmm4 | X86Reg::Xmm5 | X86Reg::Xmm6 | X86Reg::Xmm7
            | X86Reg::Xmm8 | X86Reg::Xmm9 | X86Reg::Xmm10 | X86Reg::Xmm11
            | X86Reg::Xmm12 | X86Reg::Xmm13 | X86Reg::Xmm14 | X86Reg::Xmm15
            | X86Reg::Ymm0 | X86Reg::Ymm1 | X86Reg::Ymm2 | X86Reg::Ymm3
            | X86Reg::Ymm4 | X86Reg::Ymm5 | X86Reg::Ymm6 | X86Reg::Ymm7
            | X86Reg::Ymm8 | X86Reg::Ymm9 | X86Reg::Ymm10 | X86Reg::Ymm11
            | X86Reg::Ymm12 | X86Reg::Ymm13 | X86Reg::Ymm14 | X86Reg::Ymm15
            | X86Reg::Zmm0 | X86Reg::Zmm1 | X86Reg::Zmm2 | X86Reg::Zmm3
            | X86Reg::Zmm4 | X86Reg::Zmm5 | X86Reg::Zmm6 | X86Reg::Zmm7
            | X86Reg::Zmm8 | X86Reg::Zmm9 | X86Reg::Zmm10 | X86Reg::Zmm11
            | X86Reg::Zmm12 | X86Reg::Zmm13 | X86Reg::Zmm14 | X86Reg::Zmm15
            | X86Reg::Zmm16 | X86Reg::Zmm17 | X86Reg::Zmm18 | X86Reg::Zmm19
            | X86Reg::Zmm20 | X86Reg::Zmm21 | X86Reg::Zmm22 | X86Reg::Zmm23
            | X86Reg::Zmm24 | X86Reg::Zmm25 | X86Reg::Zmm26 | X86Reg::Zmm27
            | X86Reg::Zmm28 | X86Reg::Zmm29 | X86Reg::Zmm30 | X86Reg::Zmm31 => 0,
        };
        v
    }

    /// Write any register by enum tag. Width semantics:
    /// - 64-bit GPRs: replace full 64 bits.
    /// - 32-bit GPRs: replace low 32 bits, zero upper (x86-64 rule).
    /// - 16-bit GPRs: replace low 16 bits, preserve upper bits.
    /// - 8-bit GPRs: replace low/high byte, preserve rest.
    /// - RIP/EIP/IP: written to `rip`, truncated per width.
    /// - Segment selectors: updated without descriptor-cache reload.
    ///   For correct protected-mode operation use
    ///   `setup_cpu_mode` instead.
    pub fn reg_write(&mut self, reg: X86Reg, val: u64) {
        let cpu = &mut self.cpu;
        match reg {
            X86Reg::Rax => cpu.set_rax(val),
            X86Reg::Rcx => cpu.set_rcx(val),
            X86Reg::Rdx => cpu.set_rdx(val),
            X86Reg::Rbx => cpu.set_rbx(val),
            X86Reg::Rsp => cpu.set_rsp(val),
            X86Reg::Rbp => cpu.set_rbp(val),
            X86Reg::Rsi => cpu.set_rsi(val),
            X86Reg::Rdi => cpu.set_rdi(val),
            X86Reg::R8 => cpu.set_r8(val),
            X86Reg::R9 => cpu.set_r9(val),
            X86Reg::R10 => cpu.set_r10(val),
            X86Reg::R11 => cpu.set_r11(val),
            X86Reg::R12 => cpu.set_r12(val),
            X86Reg::R13 => cpu.set_r13(val),
            X86Reg::R14 => cpu.set_r14(val),
            X86Reg::R15 => cpu.set_r15(val),

            X86Reg::Eax => cpu.set_rax((val as u32) as u64),
            X86Reg::Ecx => cpu.set_rcx((val as u32) as u64),
            X86Reg::Edx => cpu.set_rdx((val as u32) as u64),
            X86Reg::Ebx => cpu.set_rbx((val as u32) as u64),
            X86Reg::Esp => cpu.set_rsp((val as u32) as u64),
            X86Reg::Ebp => cpu.set_rbp((val as u32) as u64),
            X86Reg::Esi => cpu.set_rsi((val as u32) as u64),
            X86Reg::Edi => cpu.set_rdi((val as u32) as u64),
            X86Reg::R8d => cpu.set_r8((val as u32) as u64),
            X86Reg::R9d => cpu.set_r9((val as u32) as u64),
            X86Reg::R10d => cpu.set_r10((val as u32) as u64),
            X86Reg::R11d => cpu.set_r11((val as u32) as u64),
            X86Reg::R12d => cpu.set_r12((val as u32) as u64),
            X86Reg::R13d => cpu.set_r13((val as u32) as u64),
            X86Reg::R14d => cpu.set_r14((val as u32) as u64),
            X86Reg::R15d => cpu.set_r15((val as u32) as u64),

            X86Reg::Ax => cpu.set_gpr16(0, val as u16),
            X86Reg::Cx => cpu.set_gpr16(1, val as u16),
            X86Reg::Dx => cpu.set_gpr16(2, val as u16),
            X86Reg::Bx => cpu.set_gpr16(3, val as u16),
            X86Reg::Sp => cpu.set_gpr16(4, val as u16),
            X86Reg::Bp => cpu.set_gpr16(5, val as u16),
            X86Reg::Si => cpu.set_gpr16(6, val as u16),
            X86Reg::Di => cpu.set_gpr16(7, val as u16),
            X86Reg::R8w => cpu.set_gpr16(8, val as u16),
            X86Reg::R9w => cpu.set_gpr16(9, val as u16),
            X86Reg::R10w => cpu.set_gpr16(10, val as u16),
            X86Reg::R11w => cpu.set_gpr16(11, val as u16),
            X86Reg::R12w => cpu.set_gpr16(12, val as u16),
            X86Reg::R13w => cpu.set_gpr16(13, val as u16),
            X86Reg::R14w => cpu.set_gpr16(14, val as u16),
            X86Reg::R15w => cpu.set_gpr16(15, val as u16),

            X86Reg::Al => cpu.set_gpr8(0, val as u8),
            X86Reg::Cl => cpu.set_gpr8(1, val as u8),
            X86Reg::Dl => cpu.set_gpr8(2, val as u8),
            X86Reg::Bl => cpu.set_gpr8(3, val as u8),
            X86Reg::Ah => cpu.set_gpr8(4, val as u8),
            X86Reg::Ch => cpu.set_gpr8(5, val as u8),
            X86Reg::Dh => cpu.set_gpr8(6, val as u8),
            X86Reg::Bh => cpu.set_gpr8(7, val as u8),
            X86Reg::Spl => cpu.set_gpr8(4, val as u8),
            X86Reg::Bpl => cpu.set_gpr8(5, val as u8),
            X86Reg::Sil => cpu.set_gpr8(6, val as u8),
            X86Reg::Dil => cpu.set_gpr8(7, val as u8),
            X86Reg::R8b => cpu.set_gpr8(8, val as u8),
            X86Reg::R9b => cpu.set_gpr8(9, val as u8),
            X86Reg::R10b => cpu.set_gpr8(10, val as u8),
            X86Reg::R11b => cpu.set_gpr8(11, val as u8),
            X86Reg::R12b => cpu.set_gpr8(12, val as u8),
            X86Reg::R13b => cpu.set_gpr8(13, val as u8),
            X86Reg::R14b => cpu.set_gpr8(14, val as u8),
            X86Reg::R15b => cpu.set_gpr8(15, val as u8),

            X86Reg::Rip => cpu.set_rip(val),
            X86Reg::Eip => cpu.set_eip(val as u32),
            X86Reg::Ip => {
                // Preserve upper bits of RIP when writing 16-bit IP.
                let upper = cpu.rip() & !0xFFFF;
                cpu.set_rip(upper | (val & 0xFFFF));
            }

            X86Reg::Rflags => cpu.set_rflags_for_api(val),
            X86Reg::Eflags => cpu.set_rflags_for_api(val as u32 as u64),
            X86Reg::Flags => {
                let preserved = cpu.rflags_for_api() & !0xFFFF;
                cpu.set_rflags_for_api(preserved | (val & 0xFFFF));
            }

            X86Reg::Cs
            | X86Reg::Ss
            | X86Reg::Ds
            | X86Reg::Es
            | X86Reg::Fs
            | X86Reg::Gs => {
                // Raw selector write without descriptor-cache reload. Callers
                // should use `setup_cpu_mode` for correct protected-mode setup.
                cpu.set_seg_selector_raw_for_api(reg, val as u16);
            }

            X86Reg::FsBase => cpu.set_msr_fsbase(val),
            X86Reg::GsBase => cpu.set_msr_gsbase(val),

            X86Reg::Cr0 => cpu.set_cr0_raw_for_api(val as u32),
            X86Reg::Cr2 => cpu.set_cr2_for_api(val),
            X86Reg::Cr3 => cpu.set_cr3_raw_for_api(val),
            X86Reg::Cr4 => cpu.set_cr4_raw_for_api(val as u32),
            X86Reg::Cr8 => cpu.set_cr8_for_api(val),

            X86Reg::Dr0 => cpu.set_dr_for_api(0, val),
            X86Reg::Dr1 => cpu.set_dr_for_api(1, val),
            X86Reg::Dr2 => cpu.set_dr_for_api(2, val),
            X86Reg::Dr3 => cpu.set_dr_for_api(3, val),
            X86Reg::Dr6 => cpu.set_dr6_for_api(val),
            X86Reg::Dr7 => cpu.set_dr7_for_api(val),

            X86Reg::GdtrBase => cpu.set_gdtr_base_for_api(val),
            X86Reg::GdtrLimit => cpu.set_gdtr_limit_for_api(val as u32),
            X86Reg::IdtrBase => cpu.set_idtr_base_for_api(val),
            X86Reg::IdtrLimit => cpu.set_idtr_limit_for_api(val as u32),
            X86Reg::LdtrSelector => cpu.set_ldtr_selector_for_api(val as u16),
            X86Reg::TrSelector => cpu.set_tr_selector_for_api(val as u16),

            X86Reg::Tsc => cpu.set_tsc_for_api(val),
            X86Reg::Efer => cpu.set_efer_for_api(val),

            // FPU scalar registers
            X86Reg::FpSw => cpu.set_fpu_sw_for_api(val as u16),
            X86Reg::FpCw => cpu.set_fpu_cw_for_api(val as u16),
            X86Reg::FpTag => cpu.set_fpu_tag_for_api(val as u16),
            X86Reg::Mxcsr => cpu.set_mxcsr_for_api(val as u32),
            X86Reg::Opmask0 => cpu.opmask_write_for_api(0, val),
            X86Reg::Opmask1 => cpu.opmask_write_for_api(1, val),
            X86Reg::Opmask2 => cpu.opmask_write_for_api(2, val),
            X86Reg::Opmask3 => cpu.opmask_write_for_api(3, val),
            X86Reg::Opmask4 => cpu.opmask_write_for_api(4, val),
            X86Reg::Opmask5 => cpu.opmask_write_for_api(5, val),
            X86Reg::Opmask6 => cpu.opmask_write_for_api(6, val),
            X86Reg::Opmask7 => cpu.opmask_write_for_api(7, val),

            // Wide registers — use dedicated methods, ignore from scalar path
            X86Reg::Fpr0 | X86Reg::Fpr1 | X86Reg::Fpr2 | X86Reg::Fpr3
            | X86Reg::Fpr4 | X86Reg::Fpr5 | X86Reg::Fpr6 | X86Reg::Fpr7
            | X86Reg::Xmm0 | X86Reg::Xmm1 | X86Reg::Xmm2 | X86Reg::Xmm3
            | X86Reg::Xmm4 | X86Reg::Xmm5 | X86Reg::Xmm6 | X86Reg::Xmm7
            | X86Reg::Xmm8 | X86Reg::Xmm9 | X86Reg::Xmm10 | X86Reg::Xmm11
            | X86Reg::Xmm12 | X86Reg::Xmm13 | X86Reg::Xmm14 | X86Reg::Xmm15
            | X86Reg::Ymm0 | X86Reg::Ymm1 | X86Reg::Ymm2 | X86Reg::Ymm3
            | X86Reg::Ymm4 | X86Reg::Ymm5 | X86Reg::Ymm6 | X86Reg::Ymm7
            | X86Reg::Ymm8 | X86Reg::Ymm9 | X86Reg::Ymm10 | X86Reg::Ymm11
            | X86Reg::Ymm12 | X86Reg::Ymm13 | X86Reg::Ymm14 | X86Reg::Ymm15
            | X86Reg::Zmm0 | X86Reg::Zmm1 | X86Reg::Zmm2 | X86Reg::Zmm3
            | X86Reg::Zmm4 | X86Reg::Zmm5 | X86Reg::Zmm6 | X86Reg::Zmm7
            | X86Reg::Zmm8 | X86Reg::Zmm9 | X86Reg::Zmm10 | X86Reg::Zmm11
            | X86Reg::Zmm12 | X86Reg::Zmm13 | X86Reg::Zmm14 | X86Reg::Zmm15
            | X86Reg::Zmm16 | X86Reg::Zmm17 | X86Reg::Zmm18 | X86Reg::Zmm19
            | X86Reg::Zmm20 | X86Reg::Zmm21 | X86Reg::Zmm22 | X86Reg::Zmm23
            | X86Reg::Zmm24 | X86Reg::Zmm25 | X86Reg::Zmm26 | X86Reg::Zmm27
            | X86Reg::Zmm28 | X86Reg::Zmm29 | X86Reg::Zmm30 | X86Reg::Zmm31 => {},
        }

    }

    /// Read an MSR by index. Returns Err if the MSR is not modeled.
    pub fn msr_read(&self, msr: u32) -> Result<u64> {
        self.cpu.read_msr_for_api(msr).map_err(Error::Cpu)
    }

    /// Write an MSR by index. Returns Err if the MSR is not writable.
    pub fn msr_write(&mut self, msr: u32, val: u64) -> Result<()> {
        self.cpu.write_msr_for_api(msr, val).map_err(Error::Cpu)
    }

    /// Build a CPU snapshot on demand. Not invoked by instrumentation — see
    /// [`CpuSnapshot`] docs for the design rationale (callbacks use
    /// primitives, not snapshots).
    pub fn cpu_snapshot(&self) -> CpuSnapshot {
        let cpu = &self.cpu;
        CpuSnapshot {
            rax: cpu.rax(),
            rbx: cpu.rbx(),
            rcx: cpu.rcx(),
            rdx: cpu.rdx(),
            rsi: cpu.rsi(),
            rdi: cpu.rdi(),
            rbp: cpu.rbp(),
            rsp: cpu.rsp(),
            r8: cpu.r8(),
            r9: cpu.r9(),
            r10: cpu.r10(),
            r11: cpu.r11(),
            r12: cpu.r12(),
            r13: cpu.r13(),
            r14: cpu.r14(),
            r15: cpu.r15(),
            rip: cpu.rip(),
            eflags: cpu.rflags_for_api() as u32,
            cs: cpu.get_cs_selector(),
            ss: cpu.get_ss_selector(),
            ds: cpu.get_ds_selector(),
            es: cpu.seg_selector_for_api(0),
            fs: cpu.seg_selector_for_api(4),
            gs: cpu.seg_selector_for_api(5),
            cr0: cpu.get_cr0_val() as u64,
            cr2: cpu.cr2_for_api(),
            cr3: cpu.get_cr3_val(),
            cr4: cpu.cr4_for_api(),
            cpl: cpu.cpl_for_api(),
            icount: cpu.icount_for_api(),
            fpu_regs: core::array::from_fn(|i| cpu.fpu_read_st(i)),
            fpu_sw: cpu.fpu_sw_for_api(),
            fpu_cw: cpu.fpu_cw_for_api(),
            mxcsr: cpu.mxcsr_for_api(),
            xmm: core::array::from_fn(|i| cpu.xmm_read_for_api(i)),
        }
    }

    /// Restore CPU state from a previously captured snapshot.
    /// Writes all registers that `cpu_snapshot` captures.
    /// Note: `icount` and `cpl` are not restored (icount is monotonic,
    /// cpl is derived from segment descriptor state).
    pub fn restore_cpu_snapshot(&mut self, snap: &CpuSnapshot) {
        self.reg_write(X86Reg::Rax, snap.rax);
        self.reg_write(X86Reg::Rbx, snap.rbx);
        self.reg_write(X86Reg::Rcx, snap.rcx);
        self.reg_write(X86Reg::Rdx, snap.rdx);
        self.reg_write(X86Reg::Rsi, snap.rsi);
        self.reg_write(X86Reg::Rdi, snap.rdi);
        self.reg_write(X86Reg::Rbp, snap.rbp);
        self.reg_write(X86Reg::Rsp, snap.rsp);
        self.reg_write(X86Reg::R8, snap.r8);
        self.reg_write(X86Reg::R9, snap.r9);
        self.reg_write(X86Reg::R10, snap.r10);
        self.reg_write(X86Reg::R11, snap.r11);
        self.reg_write(X86Reg::R12, snap.r12);
        self.reg_write(X86Reg::R13, snap.r13);
        self.reg_write(X86Reg::R14, snap.r14);
        self.reg_write(X86Reg::R15, snap.r15);
        self.reg_write(X86Reg::Rip, snap.rip);
        self.reg_write(X86Reg::Rflags, snap.eflags as u64);
        self.reg_write(X86Reg::Cs, snap.cs as u64);
        self.reg_write(X86Reg::Ss, snap.ss as u64);
        self.reg_write(X86Reg::Ds, snap.ds as u64);
        self.reg_write(X86Reg::Es, snap.es as u64);
        self.reg_write(X86Reg::Fs, snap.fs as u64);
        self.reg_write(X86Reg::Gs, snap.gs as u64);
        self.reg_write(X86Reg::Cr0, snap.cr0);
        self.reg_write(X86Reg::Cr2, snap.cr2);
        self.reg_write(X86Reg::Cr3, snap.cr3);
        self.reg_write(X86Reg::Cr4, snap.cr4);
    }
}

// ─────────────────────────── Wide register read/write ───────────────────────────

impl<'a, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> Emulator<'a, I, T> {
    /// Read an x87 FPU register as 10 bytes (80-bit extended precision).
    /// `reg` must be Fpr0..Fpr7.
    pub fn reg_read_fp80(&self, reg: X86Reg) -> [u8; 10] {
        let index = match reg {
            X86Reg::Fpr0 => 0, X86Reg::Fpr1 => 1, X86Reg::Fpr2 => 2, X86Reg::Fpr3 => 3,
            X86Reg::Fpr4 => 4, X86Reg::Fpr5 => 5, X86Reg::Fpr6 => 6, X86Reg::Fpr7 => 7,
            _ => return [0u8; 10],
        };
        self.cpu.fpu_read_st(index)
    }

    pub fn reg_write_fp80(&mut self, reg: X86Reg, val: [u8; 10]) {
        let index = match reg {
            X86Reg::Fpr0 => 0, X86Reg::Fpr1 => 1, X86Reg::Fpr2 => 2, X86Reg::Fpr3 => 3,
            X86Reg::Fpr4 => 4, X86Reg::Fpr5 => 5, X86Reg::Fpr6 => 6, X86Reg::Fpr7 => 7,
            _ => return,
        };
        self.cpu.fpu_write_st(index, val);
    }

    pub fn reg_read_xmm(&self, reg: X86Reg) -> [u8; 16] {
        let index = match reg {
            X86Reg::Xmm0 => 0, X86Reg::Xmm1 => 1, X86Reg::Xmm2 => 2, X86Reg::Xmm3 => 3,
            X86Reg::Xmm4 => 4, X86Reg::Xmm5 => 5, X86Reg::Xmm6 => 6, X86Reg::Xmm7 => 7,
            X86Reg::Xmm8 => 8, X86Reg::Xmm9 => 9, X86Reg::Xmm10 => 10, X86Reg::Xmm11 => 11,
            X86Reg::Xmm12 => 12, X86Reg::Xmm13 => 13, X86Reg::Xmm14 => 14, X86Reg::Xmm15 => 15,
            _ => return [0u8; 16],
        };
        self.cpu.xmm_read_for_api(index)
    }

    pub fn reg_write_xmm(&mut self, reg: X86Reg, val: [u8; 16]) {
        let index = match reg {
            X86Reg::Xmm0 => 0, X86Reg::Xmm1 => 1, X86Reg::Xmm2 => 2, X86Reg::Xmm3 => 3,
            X86Reg::Xmm4 => 4, X86Reg::Xmm5 => 5, X86Reg::Xmm6 => 6, X86Reg::Xmm7 => 7,
            X86Reg::Xmm8 => 8, X86Reg::Xmm9 => 9, X86Reg::Xmm10 => 10, X86Reg::Xmm11 => 11,
            X86Reg::Xmm12 => 12, X86Reg::Xmm13 => 13, X86Reg::Xmm14 => 14, X86Reg::Xmm15 => 15,
            _ => return,
        };
        self.cpu.xmm_write_for_api(index, val);
    }

    pub fn reg_read_ymm(&self, reg: X86Reg) -> [u8; 32] {
        let index = match reg {
            X86Reg::Ymm0 => 0, X86Reg::Ymm1 => 1, X86Reg::Ymm2 => 2, X86Reg::Ymm3 => 3,
            X86Reg::Ymm4 => 4, X86Reg::Ymm5 => 5, X86Reg::Ymm6 => 6, X86Reg::Ymm7 => 7,
            X86Reg::Ymm8 => 8, X86Reg::Ymm9 => 9, X86Reg::Ymm10 => 10, X86Reg::Ymm11 => 11,
            X86Reg::Ymm12 => 12, X86Reg::Ymm13 => 13, X86Reg::Ymm14 => 14, X86Reg::Ymm15 => 15,
            _ => return [0u8; 32],
        };
        self.cpu.ymm_read_for_api(index)
    }

    pub fn reg_write_ymm(&mut self, reg: X86Reg, val: [u8; 32]) {
        let index = match reg {
            X86Reg::Ymm0 => 0, X86Reg::Ymm1 => 1, X86Reg::Ymm2 => 2, X86Reg::Ymm3 => 3,
            X86Reg::Ymm4 => 4, X86Reg::Ymm5 => 5, X86Reg::Ymm6 => 6, X86Reg::Ymm7 => 7,
            X86Reg::Ymm8 => 8, X86Reg::Ymm9 => 9, X86Reg::Ymm10 => 10, X86Reg::Ymm11 => 11,
            X86Reg::Ymm12 => 12, X86Reg::Ymm13 => 13, X86Reg::Ymm14 => 14, X86Reg::Ymm15 => 15,
            _ => return,
        };
        self.cpu.ymm_write_for_api(index, val);
    }

    pub fn reg_read_zmm(&self, reg: X86Reg) -> [u8; 64] {
        let index = match reg {
            X86Reg::Zmm0 => 0, X86Reg::Zmm1 => 1, X86Reg::Zmm2 => 2, X86Reg::Zmm3 => 3,
            X86Reg::Zmm4 => 4, X86Reg::Zmm5 => 5, X86Reg::Zmm6 => 6, X86Reg::Zmm7 => 7,
            X86Reg::Zmm8 => 8, X86Reg::Zmm9 => 9, X86Reg::Zmm10 => 10, X86Reg::Zmm11 => 11,
            X86Reg::Zmm12 => 12, X86Reg::Zmm13 => 13, X86Reg::Zmm14 => 14, X86Reg::Zmm15 => 15,
            X86Reg::Zmm16 => 16, X86Reg::Zmm17 => 17, X86Reg::Zmm18 => 18, X86Reg::Zmm19 => 19,
            X86Reg::Zmm20 => 20, X86Reg::Zmm21 => 21, X86Reg::Zmm22 => 22, X86Reg::Zmm23 => 23,
            X86Reg::Zmm24 => 24, X86Reg::Zmm25 => 25, X86Reg::Zmm26 => 26, X86Reg::Zmm27 => 27,
            X86Reg::Zmm28 => 28, X86Reg::Zmm29 => 29, X86Reg::Zmm30 => 30, X86Reg::Zmm31 => 31,
            _ => return [0u8; 64],
        };
        self.cpu.zmm_read_for_api(index)
    }

    pub fn reg_write_zmm(&mut self, reg: X86Reg, val: [u8; 64]) {
        let index = match reg {
            X86Reg::Zmm0 => 0, X86Reg::Zmm1 => 1, X86Reg::Zmm2 => 2, X86Reg::Zmm3 => 3,
            X86Reg::Zmm4 => 4, X86Reg::Zmm5 => 5, X86Reg::Zmm6 => 6, X86Reg::Zmm7 => 7,
            X86Reg::Zmm8 => 8, X86Reg::Zmm9 => 9, X86Reg::Zmm10 => 10, X86Reg::Zmm11 => 11,
            X86Reg::Zmm12 => 12, X86Reg::Zmm13 => 13, X86Reg::Zmm14 => 14, X86Reg::Zmm15 => 15,
            X86Reg::Zmm16 => 16, X86Reg::Zmm17 => 17, X86Reg::Zmm18 => 18, X86Reg::Zmm19 => 19,
            X86Reg::Zmm20 => 20, X86Reg::Zmm21 => 21, X86Reg::Zmm22 => 22, X86Reg::Zmm23 => 23,
            X86Reg::Zmm24 => 24, X86Reg::Zmm25 => 25, X86Reg::Zmm26 => 26, X86Reg::Zmm27 => 27,
            X86Reg::Zmm28 => 28, X86Reg::Zmm29 => 29, X86Reg::Zmm30 => 30, X86Reg::Zmm31 => 31,
            _ => return,
        };
        self.cpu.zmm_write_for_api(index, val);
    }

    // ── Exit set API ─────────────────────────────────────────────────────

    pub fn set_exits(&mut self, addrs: &[u64]) { self.exit_set.set(addrs); }
    pub fn clear_exits(&mut self) { self.exit_set.clear(); }
    pub fn add_exit(&mut self, addr: u64) -> bool { self.exit_set.add(addr) }
    pub fn remove_exit(&mut self, addr: u64) -> bool { self.exit_set.remove(addr) }

    // ── MMIO API ─────────────────────────────────────────────────────────

    /// Register an MMIO region. Physical addresses in [addr, addr+size)
    /// dispatch to callbacks instead of RAM.
    #[cfg(feature = "alloc")]
    pub fn mmio_map(
        &mut self,
        addr: u64,
        size: u64,
        read_cb: alloc::boxed::Box<dyn FnMut(u64, usize) -> u64 + Send>,
        write_cb: alloc::boxed::Box<dyn FnMut(u64, usize, u64) + Send>,
    ) {
        self.cpu.mmio.map(addr, size, read_cb, write_cb);
    }

    /// Remove MMIO regions overlapping [addr, addr+size).
    #[cfg(feature = "alloc")]
    pub fn mmio_unmap(&mut self, addr: u64, size: u64) {
        self.cpu.mmio.unmap(addr, size);
    }
}

// ─────────────────────────── mem_read / mem_write ───────────────────────────

impl<'a, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> Emulator<'a, I, T> {
    /// Read bytes from guest physical memory into the caller's buffer.
    /// Returns the number of bytes read (always `buf.len()` on success).
    /// Bypasses MMIO handlers — matches Unicorn `uc_mem_read` semantics.
    pub fn mem_read(&self, addr: u64, buf: &mut [u8]) -> Result<()> {
        let ram = self.memory.ram_slice();
        let start = addr as usize;
        let end = start
            .checked_add(buf.len())
            .ok_or_else(|| Error::Memory(crate::memory::MemoryError::ReadPhysicalPage {
                addr: addr as u64,
                len: buf.len(),
            }))?;
        if end > ram.len() {
            return Err(Error::Memory(crate::memory::MemoryError::ReadPhysicalPage {
                addr: addr as u64,
                len: buf.len(),
            }));
        }
        buf.copy_from_slice(&ram[start..end]);
        Ok(())
    }

    /// Read `size` bytes into a freshly-allocated `Vec`.
    pub fn mem_read_vec(&self, addr: u64, size: usize) -> Result<Vec<u8>> {
        let mut v = alloc::vec![0u8; size];
        self.mem_read(addr, &mut v)?;
        Ok(v)
    }

    /// Write bytes to guest physical memory.
    /// Bypasses MMIO handlers — matches Unicorn `uc_mem_write` semantics.
    pub fn mem_write(&mut self, addr: u64, data: &[u8]) -> Result<()> {
        let (ptr, cap) = self.memory.get_ram_base_ptr();
        let start = addr as usize;
        let end = start.checked_add(data.len()).ok_or_else(|| {
            Error::Memory(crate::memory::MemoryError::WritePhysicalPage {
                addr: addr as u64,
                len: data.len(),
            })
        })?;
        if end > cap {
            return Err(Error::Memory(crate::memory::MemoryError::WritePhysicalPage {
                addr: addr as u64,
                len: data.len(),
            }));
        }
        // SAFETY: bounds-checked above; write to owned guest RAM.
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), ptr.add(start), data.len());
        }
        Ok(())
    }

    /// Fill `size` bytes of guest memory with `byte`.
    pub fn mem_fill(&mut self, addr: u64, size: usize, byte: u8) -> Result<()> {
        let (ptr, cap) = self.memory.get_ram_base_ptr();
        let start = addr as usize;
        let end = start.checked_add(size).ok_or_else(|| {
            Error::Memory(crate::memory::MemoryError::WritePhysicalPage {
                addr: addr as u64,
                len: size,
            })
        })?;
        if end > cap {
            return Err(Error::Memory(crate::memory::MemoryError::WritePhysicalPage {
                addr: addr as u64,
                len: size,
            }));
        }
        // SAFETY: bounds-checked above; write to owned guest RAM.
        unsafe { core::ptr::write_bytes(ptr.add(start), byte, size) };
        Ok(())
    }

    /// Guest memory size in bytes.
    pub fn mem_size(&self) -> usize {
        self.memory.ram_slice().len()
    }

    /// Set memory permissions for a physical address range.
    /// Creates the permissions bitmap on first call, sizing it to physical memory.
    #[cfg(feature = "instrumentation")]
    pub fn mem_protect(&mut self, addr: u64, size: usize, perms: crate::cpu::instrumentation::MemPerms) {
        let mem_len = self.memory.get_memory_len();
        let pp = self.cpu.page_permissions.get_or_insert_with(|| {
            crate::memory::permissions::PagePermissions::new(mem_len as u64)
        });
        pp.set(addr, size, perms);
    }

    // Typed helpers — reduce noise when loaders build page tables, GDT/IDT
    // entries, stack frames, or TEB/PEB scaffolding.

    pub fn mem_read_u8(&self, addr: u64) -> Result<u8> {
        let mut b = [0u8; 1];
        self.mem_read(addr, &mut b)?;
        Ok(b[0])
    }

    pub fn mem_read_u16_le(&self, addr: u64) -> Result<u16> {
        let mut b = [0u8; 2];
        self.mem_read(addr, &mut b)?;
        Ok(u16::from_le_bytes(b))
    }

    pub fn mem_read_u32_le(&self, addr: u64) -> Result<u32> {
        let mut b = [0u8; 4];
        self.mem_read(addr, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    pub fn mem_read_u64_le(&self, addr: u64) -> Result<u64> {
        let mut b = [0u8; 8];
        self.mem_read(addr, &mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    pub fn mem_write_u8(&mut self, addr: u64, val: u8) -> Result<()> {
        self.mem_write(addr, &[val])
    }

    pub fn mem_write_u16_le(&mut self, addr: u64, val: u16) -> Result<()> {
        self.mem_write(addr, &val.to_le_bytes())
    }

    pub fn mem_write_u32_le(&mut self, addr: u64, val: u32) -> Result<()> {
        self.mem_write(addr, &val.to_le_bytes())
    }

    pub fn mem_write_u64_le(&mut self, addr: u64, val: u64) -> Result<()> {
        self.mem_write(addr, &val.to_le_bytes())
    }

    // ── Virtual (linear) memory access ──────────────────────────────────

    /// Translate a guest virtual address to guest physical address using
    /// the current page tables (CR3). Returns Err on page fault.
    pub fn virt_to_phys(&self, vaddr: u64) -> Result<u64> {
        self.cpu.translate_linear_for_api(vaddr).map_err(Error::Cpu)
    }

    /// Read bytes from guest VIRTUAL memory. Translates through current
    /// page tables, then reads from the resulting physical address.
    /// Handles page-crossing reads by translating each page separately.
    pub fn virt_read(&self, vaddr: u64, buf: &mut [u8]) -> Result<()> {
        let mut offset = 0;
        while offset < buf.len() {
            let va = vaddr + offset as u64;
            let page_offset = (va & 0xFFF) as usize;
            let chunk = (0x1000 - page_offset).min(buf.len() - offset);
            let pa = self.virt_to_phys(va)?;
            self.mem_read(pa, &mut buf[offset..offset + chunk])?;
            offset += chunk;
        }
        Ok(())
    }

    /// Write bytes to guest VIRTUAL memory. Translates through current
    /// page tables, then writes to the resulting physical address.
    /// Handles page-crossing writes by translating each page separately.
    pub fn virt_write(&mut self, vaddr: u64, data: &[u8]) -> Result<()> {
        let mut offset = 0;
        while offset < data.len() {
            let va = vaddr + offset as u64;
            let page_offset = (va & 0xFFF) as usize;
            let chunk = (0x1000 - page_offset).min(data.len() - offset);
            let pa = self.virt_to_phys(va)?;
            self.mem_write(pa, &data[offset..offset + chunk])?;
            offset += chunk;
        }
        Ok(())
    }

    /// Read bytes from guest virtual memory into a Vec.
    pub fn virt_read_vec(&self, vaddr: u64, size: usize) -> Result<Vec<u8>> {
        let mut v = alloc::vec![0u8; size];
        self.virt_read(vaddr, &mut v)?;
        Ok(v)
    }

    pub fn virt_read_u8(&self, vaddr: u64) -> Result<u8> {
        let mut b = [0u8; 1];
        self.virt_read(vaddr, &mut b)?;
        Ok(b[0])
    }

    pub fn virt_read_u16_le(&self, vaddr: u64) -> Result<u16> {
        let mut b = [0u8; 2];
        self.virt_read(vaddr, &mut b)?;
        Ok(u16::from_le_bytes(b))
    }

    pub fn virt_read_u32_le(&self, vaddr: u64) -> Result<u32> {
        let mut b = [0u8; 4];
        self.virt_read(vaddr, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    pub fn virt_read_u64_le(&self, vaddr: u64) -> Result<u64> {
        let mut b = [0u8; 8];
        self.virt_read(vaddr, &mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    pub fn virt_write_u8(&mut self, vaddr: u64, val: u8) -> Result<()> {
        self.virt_write(vaddr, &[val])
    }

    pub fn virt_write_u16_le(&mut self, vaddr: u64, val: u16) -> Result<()> {
        self.virt_write(vaddr, &val.to_le_bytes())
    }

    pub fn virt_write_u32_le(&mut self, vaddr: u64, val: u32) -> Result<()> {
        self.virt_write(vaddr, &val.to_le_bytes())
    }

    pub fn virt_write_u64_le(&mut self, vaddr: u64, val: u64) -> Result<()> {
        self.virt_write(vaddr, &val.to_le_bytes())
    }

    /// Fill `size` bytes of guest virtual memory with `byte`.
    /// Translates through current page tables, handles page crossings.
    pub fn virt_fill(&mut self, vaddr: u64, size: usize, byte: u8) -> Result<()> {
        let mut offset = 0;
        while offset < size {
            let va = vaddr + offset as u64;
            let page_offset = (va & 0xFFF) as usize;
            let chunk = (0x1000 - page_offset).min(size - offset);
            let pa = self.virt_to_phys(va)?;
            self.mem_fill(pa, chunk, byte)?;
            offset += chunk;
        }
        Ok(())
    }
}

// ─────────────────────────── emu_start / emu_stop ───────────────────────────

impl<'a, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> Emulator<'a, I, T> {
    /// Obtain a cross-thread [`StopHandle`] that breaks the `emu_start` loop
    /// at its next batch boundary.
    pub fn stop_handle(&self) -> StopHandle {
        StopHandle(self.stop_flag.clone())
    }

    /// Signal the running `emu_start` to stop. Call from within a hook
    /// callback (same thread that owns `&mut self`) or via `StopHandle` from
    /// another thread.
    pub fn emu_stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }

    /// Execute starting at `begin`. Every limit is optional — pass `None`
    /// for "no limit". Returns when:
    /// - RIP reaches `until` (if set)
    /// - `count` instructions executed (if set)
    /// - `timeout` wall-clock elapsed (if set, std-only)
    /// - `emu_stop`/`StopHandle::stop` was called
    /// - CPU enters HLT/MWAIT with no pending interrupts
    /// - CPU triple-faults into shutdown
    pub fn emu_start(
        &mut self,
        begin: u64,
        until: Option<u64>,
        timeout: Option<core::time::Duration>,
        count: Option<u64>,
    ) -> Result<EmuStopReason>
    where
        'a: 'static,
    {
        // Reset any prior stop signal and jump to entry.
        self.stop_flag.store(false, Ordering::Relaxed);
        self.cpu.set_rip(begin);

        #[cfg(feature = "std")]
        let start = std::time::Instant::now();
        #[cfg(not(feature = "std"))]
        {
            if timeout.is_some() {
                return Err(Error::Cpu(crate::cpu::CpuError::UnimplementedInstruction));
            }
        }

        let mut executed: u64 = 0;
        const BATCH: u64 = 4096;

        loop {
            if self.stop_flag.load(Ordering::Relaxed) {
                return Ok(EmuStopReason::Stopped);
            }
            if count.is_some_and(|c| executed >= c) {
                return Ok(EmuStopReason::CountExhausted);
            }
            #[cfg(feature = "std")]
            if timeout.is_some_and(|t| start.elapsed() >= t) {
                return Ok(EmuStopReason::TimedOut);
            }
            if self.cpu.is_in_shutdown() {
                return Ok(EmuStopReason::Shutdown);
            }

            let budget = match count {
                Some(c) => BATCH.min(c - executed),
                None => BATCH,
            };
            let (n, _shutdown) = self.step_batch(budget)?;
            executed = executed.saturating_add(n);

            if until.is_some_and(|a| self.cpu.rip() == a) {
                return Ok(EmuStopReason::ReachedUntil);
            }
            // Check exit addresses
            if !self.exit_set.is_empty() {
                let rip = self.cpu.rip();
                if self.exit_set.contains(rip) {
                    return Ok(EmuStopReason::ReachedExit(rip));
                }
            }
            if n == 0 && self.cpu.is_waiting_for_event() {
                return Ok(EmuStopReason::Halted);
            }
        }
    }

    /// Execute exactly one instruction from the current RIP.
    pub fn step_one(&mut self) -> Result<()>
    where
        'a: 'static,
    {
        self.stop_flag.store(false, Ordering::Relaxed);
        // step_batch respects the budget
        let _ = self.step_batch(1)?;
        Ok(())
    }
}

// ─────────────────────────── CpuSetupMode builders ───────────────────────────

impl<'a, I: BxCpuIdTrait> Emulator<'a, I, ()> {
    /// Create a new emulator with guest memory allocated but no BIOS loaded,
    /// pre-configured for the given CPU mode. See [`CpuSetupMode`].
    ///
    /// Returns `Box<Self>` because `Emulator` is ~1.4 MB — stack allocation
    /// would silently overflow on most platforms.
    pub fn new_with_mode(config: EmulatorConfig, mode: CpuSetupMode) -> Result<Box<Self>> {
        let mut emu = Self::new(config)?;
        // Minimal init: memory + CPU registers + async event flags. We skip
        // load_bios + pc_system.start etc. since the user will not run a BIOS.
        let cfg = emu.config_ref().clone();
        emu.memory.init_memory(
            cfg.guest_memory_size,
            cfg.host_memory_size,
            cfg.memory_block_size,
        )?;
        emu.memory.set_a20_mask(emu.pc_system.a20_mask());
        emu.pc_system.initialize(cfg.ips);
        // Bring the CPU to its reset state before applying the mode.
        emu.cpu.reset(ResetReason::Hardware);
        emu.setup_cpu_mode(mode)?;
        Ok(emu)
    }
}

impl<'a, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> Emulator<'a, I, T> {
    /// Create a new emulator pre-configured for the given CPU mode with a
    /// monomorphized tracer. Combines `new_with_instrumentation` + `setup_cpu_mode`.
    pub fn new_with_mode_and_instrumentation(
        config: EmulatorConfig,
        mode: CpuSetupMode,
        tracer: T,
    ) -> Result<Box<Self>> {
        let mut emu = Self::new_with_instrumentation(config, tracer)?;
        let cfg = emu.config_ref().clone();
        emu.memory.init_memory(
            cfg.guest_memory_size,
            cfg.host_memory_size,
            cfg.memory_block_size,
        )?;
        emu.memory.set_a20_mask(emu.pc_system.a20_mask());
        emu.pc_system.initialize(cfg.ips);
        emu.cpu.reset(crate::cpu::ResetReason::Hardware);
        emu.setup_cpu_mode(mode)?;
        Ok(emu)
    }
}

impl<'a, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> Emulator<'a, I, T> {

    /// Reconfigure an existing emulator for the given CPU mode, skipping BIOS.
    /// Must be called after `initialize()` (or from `new_with_mode`).
    pub fn setup_cpu_mode(&mut self, mode: CpuSetupMode) -> Result<()> {
        match mode {
            CpuSetupMode::RealMode => self.setup_real_mode(),
            CpuSetupMode::Protected16 => self.setup_protected16(),
            CpuSetupMode::FlatProtected32 => self.setup_flat_protected32(),
            CpuSetupMode::FlatLong64 => self.setup_flat_long64(),
        }
    }

    /// Real mode (default after reset). Ensures A20 enabled and EFLAGS sane.
    fn setup_real_mode(&mut self) -> Result<()> {
        // Reset already puts us in real mode; just enable A20 and IF.
        self.memory.set_a20_mask(0xFFFFFFFFFFFFFFFF);
        self.cpu.set_rflags_for_api(0x0000_0202); // IF=1, bit1 reserved=1
        Ok(())
    }

    /// Build a minimal GDT, set descriptor caches, and switch to CR0.PE=1
    /// with 16-bit limits. Rarely used — most callers want `FlatProtected32`.
    fn setup_protected16(&mut self) -> Result<()> {
        self.install_flat_gdt()?;
        // CS 16-bit, limit 64KB
        self.cpu.set_seg_for_api(
            crate::cpu::instrumentation::X86Reg::Cs,
            0x08,
            0,
            0xFFFF,
            /*code16*/ true,
            /*long*/ false,
        );
        // Data selectors, 16-bit
        for reg in [
            X86Reg::Ds, X86Reg::Es, X86Reg::Ss, X86Reg::Fs, X86Reg::Gs,
        ] {
            self.cpu.set_seg_for_api(reg, 0x10, 0, 0xFFFF, false, false);
        }
        self.cpu.enter_protected_mode_for_api();
        self.memory.set_a20_mask(0xFFFFFFFFFFFFFFFF);
        self.cpu.set_rflags_for_api(0x0000_0202);
        Ok(())
    }

    /// Flat 32-bit protected mode: CR0.PE=1, segments base=0 limit=4GB,
    /// 32-bit default operand/address size.
    fn setup_flat_protected32(&mut self) -> Result<()> {
        self.install_flat_gdt()?;
        // CS 32-bit, 4GB flat
        self.cpu.set_seg_for_api(
            X86Reg::Cs,
            0x08,
            0,
            0xFFFFFFFF,
            /*code16*/ false,
            /*long*/ false,
        );
        for reg in [
            X86Reg::Ds, X86Reg::Es, X86Reg::Ss, X86Reg::Fs, X86Reg::Gs,
        ] {
            self.cpu
                .set_seg_for_api(reg, 0x10, 0, 0xFFFFFFFF, false, false);
        }
        self.cpu.enter_protected_mode_for_api();
        self.memory.set_a20_mask(0xFFFFFFFFFFFFFFFF);
        self.cpu.set_rflags_for_api(0x0000_0202);
        Ok(())
    }

    /// Flat 64-bit long mode with identity-mapped 2 MiB pages at CR3.
    /// Suitable for PE64, ELF64, kernel snapshots.
    fn setup_flat_long64(&mut self) -> Result<()> {
        // Page-table layout: at `PT_BASE` we place PML4, PDPT, then 4 PDs
        // (each covering 1 GiB, giving 4 GiB of identity-mapped RAM).
        const PT_BASE: u64 = 0x1000;
        const PML4: u64 = PT_BASE;
        const PDPT: u64 = PT_BASE + 0x1000;
        const PD0: u64 = PT_BASE + 0x2000;

        // PML4[0] = PDPT | P | RW
        self.mem_write_u64_le(PML4, PDPT | 0x3)?;

        // PDPT[0..4] = PD_i | P | RW  (covers 4 GiB)
        for i in 0..4u64 {
            self.mem_write_u64_le(PDPT + i * 8, (PD0 + i * 0x1000) | 0x3)?;
        }

        // Each PD has 512 entries of 2 MiB pages: P | RW | PS
        for i in 0..4u64 {
            let pd = PD0 + i * 0x1000;
            for j in 0..512u64 {
                let phys = (i * 512 + j) * 0x0020_0000;
                self.mem_write_u64_le(pd + j * 8, phys | 0x83)?;
            }
        }

        self.install_flat_gdt()?;
        // CS in long mode: L=1, D=0
        self.cpu.set_seg_for_api(
            X86Reg::Cs,
            0x08,
            0,
            0xFFFFFFFF,
            /*code16*/ false,
            /*long*/ true,
        );
        for reg in [
            X86Reg::Ds, X86Reg::Es, X86Reg::Ss, X86Reg::Fs, X86Reg::Gs,
        ] {
            self.cpu
                .set_seg_for_api(reg, 0x10, 0, 0xFFFFFFFF, false, false);
        }

        self.cpu.enter_long_mode_for_api(PML4);
        self.memory.set_a20_mask(0xFFFFFFFFFFFFFFFF);
        self.cpu.set_rflags_for_api(0x0000_0202);
        Ok(())
    }

    /// Install a minimal flat GDT at 0x800 with null/code/data/TSS
    /// descriptors. Shared by all protected-mode setups.
    fn install_flat_gdt(&mut self) -> Result<()> {
        const GDT_BASE: u64 = 0x0800;

        // Null descriptor
        self.mem_write_u64_le(GDT_BASE, 0)?;

        // Code selector at index 1 (selector 0x08):
        // base=0 limit=0xFFFFF G=1 (4 KiB pages → 4 GiB) P=1 DPL=0 S=1
        // type=1010 (code, readable, non-conforming) D=1 L=0 AVL=0
        // For FlatLong64 we overwrite this entry below in enter_long_mode path
        // via set_seg_for_api which writes descriptor caches directly — the
        // GDT itself needs a plausible entry so IRET/syscall paths succeed.
        let code_desc: u64 = 0x00CF9A000000FFFF;
        self.mem_write_u64_le(GDT_BASE + 0x08, code_desc)?;

        // Data selector at index 2 (selector 0x10):
        // base=0 limit=0xFFFFF G=1 P=1 DPL=0 S=1 type=0010 (data, writable)
        let data_desc: u64 = 0x00CF92000000FFFF;
        self.mem_write_u64_le(GDT_BASE + 0x10, data_desc)?;

        self.cpu.set_gdtr_base_for_api(GDT_BASE);
        self.cpu.set_gdtr_limit_for_api(0x1F);
        Ok(())
    }
}


// ─────────────────────────── Tests ───────────────────────────

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::cpu::core_i7_skylake::Corei7SkylakeX;

    /// Reg read/write round-trip on a fresh emulator.
    #[test]
    fn reg_read_write_round_trip() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.reg_write(X86Reg::Rax, 0xDEAD_BEEF_CAFE_BABE);
                assert_eq!(
                    emu.reg_read(X86Reg::Rax),
                    0xDEAD_BEEF_CAFE_BABE
                );
                assert_eq!(
                    emu.reg_read(X86Reg::Eax),
                    0xCAFE_BABE
                );
                assert_eq!(emu.reg_read(X86Reg::Ax), 0xBABE);
                assert_eq!(emu.reg_read(X86Reg::Al), 0xBE);
                assert_eq!(emu.reg_read(X86Reg::Ah), 0xBA);
                emu.reg_write(X86Reg::Rip, 0x1234);
                assert_eq!(emu.reg_read(X86Reg::Rip), 0x1234);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// mem_write then mem_read returns the same bytes.
    #[test]
    fn mem_read_write_round_trip() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.initialize().unwrap();
                let data: [u8; 16] = [
                    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
                    0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
                ];
                emu.mem_write(0x20_000, &data).unwrap();
                let mut buf = [0u8; 16];
                emu.mem_read(0x20_000, &mut buf).unwrap();
                assert_eq!(buf, data);

                // Typed helpers
                emu.mem_write_u64_le(0x20_000, 0xCAFE_BABE_DEAD_BEEF).unwrap();
                assert_eq!(
                    emu.mem_read_u64_le(0x20_000).unwrap(),
                    0xCAFE_BABE_DEAD_BEEF
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// StopHandle is Send + Sync + Clone. Static assertion.
    #[test]
    fn stop_handle_trait_bounds() {
        fn assert_send_sync<T: Send + Sync + Clone>() {}
        assert_send_sync::<StopHandle>();
    }

    /// Emulator is Send. Static assertion that `stop_handle` returns the
    /// Arc<AtomicBool> backing store (not a borrow).
    #[test]
    fn stop_handle_stops_flag() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                let handle = emu.stop_handle();
                assert!(!handle.is_stopping());
                handle.stop();
                assert!(handle.is_stopping());
                assert!(emu.stop_flag.load(Ordering::Relaxed));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// CpuSetupMode::FlatProtected32 puts the CPU into PM with flat segments.
    #[test]
    fn flat_protected32_setup() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let cfg = EmulatorConfig::default();
                let emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    cfg,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                // CR0.PE should be set
                let cr0 = emu.reg_read(X86Reg::Cr0);
                assert!(cr0 & 0x1 != 0, "CR0.PE not set after FlatProtected32 setup: {:#x}", cr0);
                // CS should be 0x08, DS 0x10
                assert_eq!(emu.reg_read(X86Reg::Cs), 0x08);
                assert_eq!(emu.reg_read(X86Reg::Ds), 0x10);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// CpuSetupMode::FlatLong64 enables CR0.PG, CR4.PAE, EFER.LME/LMA, CS.L=1.
    #[test]
    fn flat_long64_setup() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let cfg = EmulatorConfig::default();
                let emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    cfg,
                    CpuSetupMode::FlatLong64,
                )
                .unwrap();
                let cr0 = emu.reg_read(X86Reg::Cr0);
                assert!(cr0 & 0x1 != 0, "CR0.PE not set");
                assert!(cr0 & 0x8000_0000 != 0, "CR0.PG not set: {:#x}", cr0);
                let cr4 = emu.reg_read(X86Reg::Cr4);
                assert!(cr4 & (1 << 5) != 0, "CR4.PAE not set: {:#x}", cr4);
                let efer = emu.reg_read(X86Reg::Efer);
                assert!(efer & (1 << 8) != 0, "EFER.LME not set: {:#x}", efer);
                assert!(efer & (1 << 10) != 0, "EFER.LMA not set: {:#x}", efer);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Hook registration and deletion round-trip.
    #[cfg(feature = "instrumentation")]
    #[test]
    fn hook_add_del_roundtrip() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let cfg = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(cfg).unwrap();
                let h = emu.hook_add_code(.., |_, _| {});
                assert!(emu.hook_del(h).is_ok());
                assert!(emu.hook_del(h).is_err(), "double-delete must fail");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// FPU register round-trip: write FP80 bytes, read back.
    #[test]
    fn fpu_reg_round_trip() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let cfg = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(cfg).unwrap();
                let val: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 0x00, 0x40]; // ~2.0 in FP80
                emu.reg_write_fp80(X86Reg::Fpr0, val);
                let read_back = emu.reg_read_fp80(X86Reg::Fpr0);
                assert_eq!(read_back, val, "FPU ST(0) round-trip failed");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// XMM register round-trip.
    #[test]
    fn xmm_reg_round_trip() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let cfg = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(cfg).unwrap();
                let val: [u8; 16] = [0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE,
                                      0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
                emu.reg_write_xmm(X86Reg::Xmm5, val);
                assert_eq!(emu.reg_read_xmm(X86Reg::Xmm5), val, "XMM5 round-trip failed");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// YMM register round-trip (256-bit).
    #[test]
    fn ymm_reg_round_trip() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let cfg = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(cfg).unwrap();
                let mut val = [0u8; 32];
                for (i, b) in val.iter_mut().enumerate() { *b = i as u8; }
                emu.reg_write_ymm(X86Reg::Ymm3, val);
                assert_eq!(emu.reg_read_ymm(X86Reg::Ymm3), val, "YMM3 round-trip failed");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// ExitSet basic operations.
    #[test]
    fn exit_set_operations() {
        use crate::cpu::instrumentation::ExitSet;
        let mut es = ExitSet::new();
        assert!(es.is_empty());
        assert!(es.add(0x1000));
        assert!(es.add(0x2000));
        assert!(!es.is_empty());
        assert!(es.contains(0x1000));
        assert!(es.contains(0x2000));
        assert!(!es.contains(0x3000));
        assert!(es.remove(0x1000));
        assert!(!es.contains(0x1000));
        es.clear();
        assert!(es.is_empty());
    }

    /// Multiple exits via set_exits.
    #[test]
    fn exit_set_bulk() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let cfg = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(cfg).unwrap();
                emu.set_exits(&[0x1000, 0x2000, 0x3000]);
                emu.remove_exit(0x2000);
                emu.add_exit(0x4000);
                emu.clear_exits();
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Block hook registration round-trip.
    #[cfg(feature = "instrumentation")]
    #[test]
    fn hook_add_block_round_trip() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let cfg = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(cfg).unwrap();
                let h = emu.hook_add_block(.., |_rip, _size| {});
                assert!(emu.hook_del(h).is_ok());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Invalid instruction hook registration.
    #[cfg(feature = "instrumentation")]
    #[test]
    fn hook_add_invalid_insn() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let cfg = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(cfg).unwrap();
                let h = emu.hook_add_invalid_insn(|_rip| false);
                assert!(emu.hook_del(h).is_ok());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Memory permissions basic operations.
    #[cfg(feature = "instrumentation")]
    #[test]
    fn mem_permissions_basic() {
        use crate::memory::permissions::PagePermissions;
        use crate::cpu::instrumentation::MemPerms;
        let mut pp = PagePermissions::new(0x10_0000); // 1MB
        // Default: all permissions
        assert!(pp.check(0x1000, MemPerms::READ));
        assert!(pp.check(0x1000, MemPerms::WRITE));
        assert!(pp.check(0x1000, MemPerms::EXEC));
        // Restrict to read-only
        pp.set(0x1000, 0x1000, MemPerms::READ);
        assert!(pp.check(0x1000, MemPerms::READ));
        assert!(!pp.check(0x1000, MemPerms::WRITE));
        assert!(!pp.check(0x1000, MemPerms::EXEC));
    }

    /// MMIO registry map/unmap.
    #[test]
    fn mmio_registry_map_unmap() {
        use crate::memory::mmio::MmioRegistry;
        let mut reg = MmioRegistry::new();
        assert!(reg.is_empty());
        reg.map(0xFEC0_0000, 0x1000,
            Box::new(|_addr, _size| 0),
            Box::new(|_addr, _size, _val| {}),
        );
        assert!(!reg.is_empty());
        assert!(reg.find_mut(0xFEC0_0000).is_some());
        assert!(reg.find_mut(0xFEC0_0FFF).is_some());
        assert!(reg.find_mut(0xFEC0_1000).is_none()); // past end
        reg.unmap(0xFEC0_0000, 0x1000);
        assert!(reg.is_empty());
    }
}