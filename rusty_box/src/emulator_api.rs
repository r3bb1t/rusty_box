//! Public API extensions for [`Emulator`]: Unicorn-style hook registration,
//! register/memory read-write, `emu_start`/`emu_stop` execution control, and
//! direct-binary CPU-mode builders.
//!
//! This module lives in its own file to keep the core orchestration in
//! `emulator.rs` readable. Everything here operates purely on the public
//! `&mut Emulator` surface.

#[cfg(feature = "alloc")]
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "instrumentation")]
use core::ops::RangeBounds;

#[cfg(feature = "instrumentation")]
use crate::cpu::decoder::Instruction;
#[cfg(feature = "alloc")]
use crate::cpu::instrumentation::EmuStopReason;
#[cfg(feature = "alloc")]
use crate::emulator::StopReason;
#[cfg(feature = "instrumentation")]
use crate::cpu::instrumentation::{
    BranchEvent, HookHandle, HwInterruptEvent, InstrumentationError, IoHookEvent, IoHookType,
    MemHookEvent, MemHookType,
};
use crate::cpu::api_bridge::{ScaledLimit, SegmentSize};
use crate::cpu::instrumentation::{CpuSetupMode, CpuSnapshot, X86Reg};
#[cfg(feature = "alloc")]
use crate::cpu::ResetReason;
use crate::emulator::Emulator;
#[cfg(feature = "alloc")]
use crate::emulator::EmulatorConfig;
use crate::iodev::devices::DeviceManager;
use crate::iodev::serial::{SerialTxDrain, SERIAL_PORT_COUNT};
use crate::iodev::{BxDevicesC, DebugconDrain};
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
#[cfg(feature = "alloc")]
#[derive(Clone)]
pub struct StopHandle(pub(crate) Arc<AtomicBool>);

#[cfg(feature = "alloc")]
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

// ─────────────────────────── Role handles ───────────────────────────
//
// Transient `&mut` borrows of one device role, obtained from the machine and
// dropped at the end of the expression. Machine internals stay crate-private
// (doctrine R3) — this is the supported path to device state. Handles are
// deliberately minimal in this first cut; the automation phase grows them
// (input injection, display readback, serial `send`) without renaming.

/// Transient borrow of one UART, from [`Emulator::serial`].
pub struct Serial<'m> {
    devices: &'m mut DeviceManager,
    port: usize,
}

impl Serial<'_> {
    /// Drain the bytes the guest has transmitted on this port, in write order.
    ///
    /// The buffer is bounded, so a host that never drains loses the oldest
    /// bytes rather than growing without limit.
    #[inline]
    pub fn take_output(&mut self) -> SerialTxDrain<'_> {
        self.devices.drain_serial_tx(self.port)
    }
}

/// Transient borrow of the port-0xE9 debug console, from
/// [`Emulator::debug_port`].
///
/// Bochs `unmapped.cc` `port_e9_hack` — optional upstream, always present
/// here. BIOS/VGABIOS message ports (0x400-0x403, 0x500-0x503) are a separate
/// stream and never appear on this one, matching `biosdev.cc`.
pub struct DebugPort<'m> {
    devices: &'m mut BxDevicesC,
}

impl DebugPort<'_> {
    /// Drain the bytes the guest has written to port 0xE9, in write order.
    #[inline]
    pub fn take_output(&mut self) -> DebugconDrain<'_> {
        self.devices.drain_port_e9_output()
    }
}

impl<T: crate::cpu::instrumentation::Instrumentation> Emulator<T> {
    /// Borrow UART `port` (0-based; COM1 is 0). `None` when the index is
    /// outside the modelled set.
    #[inline]
    pub fn serial(&mut self, port: usize) -> Option<Serial<'_>> {
        (port < SERIAL_PORT_COUNT).then(|| Serial {
            devices: &mut self.device_manager,
            port,
        })
    }

    /// Borrow the port-0xE9 debug console. Present on every profile — it is a
    /// chipset facility, not a device that can be left unattached.
    #[inline]
    pub fn debug_port(&mut self) -> DebugPort<'_> {
        DebugPort {
            devices: &mut self.devices,
        }
    }

    /// Share this machine's stop flag with another thread, replacing the one
    /// it was built with. The GUI path uses this so its own reset/close
    /// controls break the run loop.
    ///
    /// Prefer [`Emulator::stop_handle`] when a fresh handle is all that is
    /// needed; this exists for the case where the *caller* already owns the
    /// flag that other code watches.
    #[cfg(feature = "alloc")]
    #[inline]
    pub fn set_stop_flag(&mut self, flag: Arc<AtomicBool>) {
        self.stop_flag = flag;
    }

    /// Read access to the stop flag. Same shape under every feature setting
    /// (doctrine R0): `alloc` shares an `Arc`, no-alloc owns the atomic, and
    /// both hand out `&AtomicBool`.
    #[inline]
    pub fn stop_flag(&self) -> &AtomicBool {
        &self.stop_flag
    }

    /// A20 gate state (Bochs `pc_system.cc` `get_enable_a20`).
    #[inline]
    pub fn get_enable_a20(&self) -> bool {
        self.pc_system.get_enable_a20()
    }

}

// ─────────────────────────── Hook registration ───────────────────────────
//
// All hook_add_* methods require the `instrumentation` feature because they
// populate the [`InstrumentationRegistry`] on the CPU, which is itself
// feature-gated. When the feature is off, the methods simply do not exist.

#[cfg(all(feature = "instrumentation", feature = "alloc"))]
impl<'a, T: crate::cpu::instrumentation::Instrumentation> Emulator<T> {
    /// Register a hook fired before each instruction whose RIP is in `range`.
    /// Callback receives `(rip, &Instruction)`.
    pub fn hook_add_code<R, F>(&mut self, range: R, cb: F) -> HookHandle
    where
        R: RangeBounds<u64>,
        F: FnMut(u64, &Instruction) + Send + 'static,
    {
        self.cpu_mut().instrumentation.add_code(range, Box::new(cb))
    }

    /// Register a hook fired AFTER each instruction whose RIP is in `range`.
    pub fn hook_add_code_after<R, F>(&mut self, range: R, cb: F) -> HookHandle
    where
        R: RangeBounds<u64>,
        F: FnMut(u64, &Instruction) + Send + 'static,
    {
        self.cpu_mut().instrumentation.add_code_after(range, Box::new(cb))
    }

    /// Register a memory access hook.
    pub fn hook_add_mem<R, F>(&mut self, hook_type: MemHookType, range: R, cb: F) -> HookHandle
    where
        R: RangeBounds<u64>,
        F: FnMut(&MemHookEvent) + Send + 'static,
    {
        self.cpu_mut().instrumentation.add_mem(hook_type, range, Box::new(cb))
    }

    /// Register a software-interrupt hook (INT n / INT3 / INTO).
    /// Callback receives the vector.
    pub fn hook_add_interrupt<F>(&mut self, cb: F) -> HookHandle
    where
        F: FnMut(u8) + Send + 'static,
    {
        self.cpu_mut().instrumentation.add_interrupt(Box::new(cb))
    }

    /// Register a hardware-interrupt hook (external IRQ delivery).
    pub fn hook_add_hwinterrupt<F>(&mut self, cb: F) -> HookHandle
    where
        F: FnMut(&HwInterruptEvent) + Send + 'static,
    {
        self.cpu_mut().instrumentation.add_hw_interrupt(Box::new(cb))
    }

    /// Register a CPU-exception hook.
    /// Callback receives `(vector, error_code)`.
    pub fn hook_add_exception<F>(&mut self, cb: F) -> HookHandle
    where
        F: FnMut(u8, u32) + Send + 'static,
    {
        self.cpu_mut().instrumentation.add_exception(Box::new(cb))
    }

    /// Register an I/O port hook (IN/OUT instructions).
    pub fn hook_add_io<R, F>(&mut self, hook_type: IoHookType, range: R, cb: F) -> HookHandle
    where
        R: RangeBounds<u16>,
        F: FnMut(&IoHookEvent) + Send + 'static,
    {
        self.cpu_mut().instrumentation.add_io(hook_type, range, Box::new(cb))
    }

    /// Register a branch hook. Fires for conditional, unconditional, and
    /// far branches; the variant in [`BranchEvent`] tells them apart.
    pub fn hook_add_branch<R, F>(&mut self, range: R, cb: F) -> HookHandle
    where
        R: RangeBounds<u64>,
        F: FnMut(&BranchEvent) + Send + 'static,
    {
        self.cpu_mut().instrumentation.add_branch(range, Box::new(cb))
    }

    /// Register a block hook. Fires at the start of each basic block (trace)
    /// whose RIP is in range.
    pub fn hook_add_block<R, F>(&mut self, range: R, cb: F) -> HookHandle
    where
        R: RangeBounds<u64>,
        F: FnMut(u64, u16) + Send + 'static,
    {
        self.cpu_mut().instrumentation.add_block(range, Box::new(cb))
    }

    /// Register an invalid-instruction hook. Fires before #UD for
    /// unrecognized opcodes. Return `true` from the callback to suppress
    /// the exception.
    pub fn hook_add_invalid_insn<F>(&mut self, cb: F) -> HookHandle
    where
        F: FnMut(u64) -> bool + Send + 'static,
    {
        self.cpu_mut().instrumentation.add_invalid_insn(Box::new(cb))
    }

    /// Register an unmapped-memory hook. Fires before page fault for
    /// not-present pages. Return `true` to suppress the fault.
    pub fn hook_add_mem_unmapped<F>(&mut self, cb: F) -> HookHandle
    where
        F: FnMut(u64, usize, crate::cpu::instrumentation::MemAccessRW) -> bool + Send + 'static,
    {
        self.cpu_mut().instrumentation.add_mem_unmapped(Box::new(cb))
    }

    /// Remove a previously registered hook.
    /// Returns `Err(InvalidHandle)` if the handle was already removed or
    /// never valid.
    pub fn hook_del(
        &mut self,
        handle: HookHandle,
    ) -> core::result::Result<(), InstrumentationError> {
        self.cpu_mut().instrumentation.remove(handle)
    }

    /// Direct typed reference to the installed tracer. Zero-cost field access.
    /// Panics only if called while a hook is mid-dispatch (the tracer is
    /// temporarily taken for borrow-splitting) — user code can't observe this.
    pub fn instrumentation(&self) -> &T {
        self.cpu()
            .instrumentation
            .tracer
            .as_ref()
            .expect("tracer absent only during hook dispatch")
    }

    /// Mutable reference to the installed tracer.
    pub fn instrumentation_mut(&mut self) -> &mut T {
        self.cpu_mut()
            .instrumentation
            .tracer
            .as_mut()
            .expect("tracer absent only during hook dispatch")
    }

    /// Recompute the active hook mask from the tracer's `active_hooks()`.
    /// Call this after mutating tracer state that changes which categories are active.
    pub fn refresh_hook_mask(&mut self) {
        self.cpu_mut().instrumentation.refresh_active();
    }
}

// ─────────────────────────── reg_read / reg_write ───────────────────────────

impl<'a, T: crate::cpu::instrumentation::Instrumentation> Emulator<T> {
    /// Read any register by enum tag. Narrower registers are zero-extended
    /// into the returned `u64`.
    pub fn reg_read(&self, reg: X86Reg) -> u64 {
        self.cpu().api_reg_read(reg)
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
        self.cpu_mut().api_reg_write(reg, val);
    }

    /// Read an MSR by index. Returns Err if the MSR is not modeled.
    pub fn msr_read(&self, msr: u32) -> Result<u64> {
        self.cpu().read_msr_for_api(msr).map_err(Error::Cpu)
    }

    /// Write an MSR by index. Returns Err if the MSR is not writable.
    pub fn msr_write(&mut self, msr: u32, val: u64) -> Result<()> {
        self.cpu_mut().write_msr_for_api(msr, val).map_err(Error::Cpu)
    }

    /// Build a CPU snapshot on demand. Not invoked by instrumentation — see
    /// [`CpuSnapshot`] docs for the design rationale (callbacks use
    /// primitives, not snapshots).
    pub fn cpu_snapshot(&self) -> CpuSnapshot {
        let cpu = self.cpu();
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

impl<'a, T: crate::cpu::instrumentation::Instrumentation> Emulator<T> {
    /// Read an x87 FPU register as 10 bytes (80-bit extended precision).
    /// `reg` must be Fpr0..Fpr7.
    pub fn reg_read_fp80(&self, reg: X86Reg) -> [u8; 10] {
        let index = match reg {
            X86Reg::Fpr0 => 0,
            X86Reg::Fpr1 => 1,
            X86Reg::Fpr2 => 2,
            X86Reg::Fpr3 => 3,
            X86Reg::Fpr4 => 4,
            X86Reg::Fpr5 => 5,
            X86Reg::Fpr6 => 6,
            X86Reg::Fpr7 => 7,
            _ => return [0u8; 10],
        };
        self.cpu().fpu_read_st(index)
    }

    pub fn reg_write_fp80(&mut self, reg: X86Reg, val: [u8; 10]) {
        let index = match reg {
            X86Reg::Fpr0 => 0,
            X86Reg::Fpr1 => 1,
            X86Reg::Fpr2 => 2,
            X86Reg::Fpr3 => 3,
            X86Reg::Fpr4 => 4,
            X86Reg::Fpr5 => 5,
            X86Reg::Fpr6 => 6,
            X86Reg::Fpr7 => 7,
            _ => return,
        };
        self.cpu_mut().fpu_write_st(index, val);
    }

    pub fn reg_read_xmm(&self, reg: X86Reg) -> [u8; 16] {
        let index = match reg {
            X86Reg::Xmm0 => 0,
            X86Reg::Xmm1 => 1,
            X86Reg::Xmm2 => 2,
            X86Reg::Xmm3 => 3,
            X86Reg::Xmm4 => 4,
            X86Reg::Xmm5 => 5,
            X86Reg::Xmm6 => 6,
            X86Reg::Xmm7 => 7,
            X86Reg::Xmm8 => 8,
            X86Reg::Xmm9 => 9,
            X86Reg::Xmm10 => 10,
            X86Reg::Xmm11 => 11,
            X86Reg::Xmm12 => 12,
            X86Reg::Xmm13 => 13,
            X86Reg::Xmm14 => 14,
            X86Reg::Xmm15 => 15,
            _ => return [0u8; 16],
        };
        self.cpu().xmm_read_for_api(index)
    }

    pub fn reg_write_xmm(&mut self, reg: X86Reg, val: [u8; 16]) {
        let index = match reg {
            X86Reg::Xmm0 => 0,
            X86Reg::Xmm1 => 1,
            X86Reg::Xmm2 => 2,
            X86Reg::Xmm3 => 3,
            X86Reg::Xmm4 => 4,
            X86Reg::Xmm5 => 5,
            X86Reg::Xmm6 => 6,
            X86Reg::Xmm7 => 7,
            X86Reg::Xmm8 => 8,
            X86Reg::Xmm9 => 9,
            X86Reg::Xmm10 => 10,
            X86Reg::Xmm11 => 11,
            X86Reg::Xmm12 => 12,
            X86Reg::Xmm13 => 13,
            X86Reg::Xmm14 => 14,
            X86Reg::Xmm15 => 15,
            _ => return,
        };
        self.cpu_mut().xmm_write_for_api(index, val);
    }

    pub fn reg_read_ymm(&self, reg: X86Reg) -> [u8; 32] {
        let index = match reg {
            X86Reg::Ymm0 => 0,
            X86Reg::Ymm1 => 1,
            X86Reg::Ymm2 => 2,
            X86Reg::Ymm3 => 3,
            X86Reg::Ymm4 => 4,
            X86Reg::Ymm5 => 5,
            X86Reg::Ymm6 => 6,
            X86Reg::Ymm7 => 7,
            X86Reg::Ymm8 => 8,
            X86Reg::Ymm9 => 9,
            X86Reg::Ymm10 => 10,
            X86Reg::Ymm11 => 11,
            X86Reg::Ymm12 => 12,
            X86Reg::Ymm13 => 13,
            X86Reg::Ymm14 => 14,
            X86Reg::Ymm15 => 15,
            _ => return [0u8; 32],
        };
        self.cpu().ymm_read_for_api(index)
    }

    pub fn reg_write_ymm(&mut self, reg: X86Reg, val: [u8; 32]) {
        let index = match reg {
            X86Reg::Ymm0 => 0,
            X86Reg::Ymm1 => 1,
            X86Reg::Ymm2 => 2,
            X86Reg::Ymm3 => 3,
            X86Reg::Ymm4 => 4,
            X86Reg::Ymm5 => 5,
            X86Reg::Ymm6 => 6,
            X86Reg::Ymm7 => 7,
            X86Reg::Ymm8 => 8,
            X86Reg::Ymm9 => 9,
            X86Reg::Ymm10 => 10,
            X86Reg::Ymm11 => 11,
            X86Reg::Ymm12 => 12,
            X86Reg::Ymm13 => 13,
            X86Reg::Ymm14 => 14,
            X86Reg::Ymm15 => 15,
            _ => return,
        };
        self.cpu_mut().ymm_write_for_api(index, val);
    }

    pub fn reg_read_zmm(&self, reg: X86Reg) -> [u8; 64] {
        let index = match reg {
            X86Reg::Zmm0 => 0,
            X86Reg::Zmm1 => 1,
            X86Reg::Zmm2 => 2,
            X86Reg::Zmm3 => 3,
            X86Reg::Zmm4 => 4,
            X86Reg::Zmm5 => 5,
            X86Reg::Zmm6 => 6,
            X86Reg::Zmm7 => 7,
            X86Reg::Zmm8 => 8,
            X86Reg::Zmm9 => 9,
            X86Reg::Zmm10 => 10,
            X86Reg::Zmm11 => 11,
            X86Reg::Zmm12 => 12,
            X86Reg::Zmm13 => 13,
            X86Reg::Zmm14 => 14,
            X86Reg::Zmm15 => 15,
            X86Reg::Zmm16 => 16,
            X86Reg::Zmm17 => 17,
            X86Reg::Zmm18 => 18,
            X86Reg::Zmm19 => 19,
            X86Reg::Zmm20 => 20,
            X86Reg::Zmm21 => 21,
            X86Reg::Zmm22 => 22,
            X86Reg::Zmm23 => 23,
            X86Reg::Zmm24 => 24,
            X86Reg::Zmm25 => 25,
            X86Reg::Zmm26 => 26,
            X86Reg::Zmm27 => 27,
            X86Reg::Zmm28 => 28,
            X86Reg::Zmm29 => 29,
            X86Reg::Zmm30 => 30,
            X86Reg::Zmm31 => 31,
            _ => return [0u8; 64],
        };
        self.cpu().zmm_read_for_api(index)
    }

    pub fn reg_write_zmm(&mut self, reg: X86Reg, val: [u8; 64]) {
        let index = match reg {
            X86Reg::Zmm0 => 0,
            X86Reg::Zmm1 => 1,
            X86Reg::Zmm2 => 2,
            X86Reg::Zmm3 => 3,
            X86Reg::Zmm4 => 4,
            X86Reg::Zmm5 => 5,
            X86Reg::Zmm6 => 6,
            X86Reg::Zmm7 => 7,
            X86Reg::Zmm8 => 8,
            X86Reg::Zmm9 => 9,
            X86Reg::Zmm10 => 10,
            X86Reg::Zmm11 => 11,
            X86Reg::Zmm12 => 12,
            X86Reg::Zmm13 => 13,
            X86Reg::Zmm14 => 14,
            X86Reg::Zmm15 => 15,
            X86Reg::Zmm16 => 16,
            X86Reg::Zmm17 => 17,
            X86Reg::Zmm18 => 18,
            X86Reg::Zmm19 => 19,
            X86Reg::Zmm20 => 20,
            X86Reg::Zmm21 => 21,
            X86Reg::Zmm22 => 22,
            X86Reg::Zmm23 => 23,
            X86Reg::Zmm24 => 24,
            X86Reg::Zmm25 => 25,
            X86Reg::Zmm26 => 26,
            X86Reg::Zmm27 => 27,
            X86Reg::Zmm28 => 28,
            X86Reg::Zmm29 => 29,
            X86Reg::Zmm30 => 30,
            X86Reg::Zmm31 => 31,
            _ => return,
        };
        self.cpu_mut().zmm_write_for_api(index, val);
    }

    // ── Exit set API ─────────────────────────────────────────────────────

    pub fn set_exits(&mut self, addrs: &[u64]) {
        self.exit_set.set(addrs);
    }
    pub fn clear_exits(&mut self) {
        self.exit_set.clear();
    }
    pub fn add_exit(&mut self, addr: u64) -> bool {
        self.exit_set.add(addr)
    }
    pub fn remove_exit(&mut self, addr: u64) -> bool {
        self.exit_set.remove(addr)
    }

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
        self.cpu_mut().mmio.map(addr, size, read_cb, write_cb);
    }

    /// Remove MMIO regions overlapping [addr, addr+size).
    #[cfg(feature = "alloc")]
    pub fn mmio_unmap(&mut self, addr: u64, size: u64) {
        self.cpu_mut().mmio.unmap(addr, size);
    }
}

// ─────────────────────────── mem_read / mem_write ───────────────────────────

impl<'a, T: crate::cpu::instrumentation::Instrumentation> Emulator<T> {
    /// Read bytes from guest physical memory into the caller's buffer.
    /// Returns the number of bytes read (always `buf.len()` on success).
    /// Bypasses MMIO handlers — matches Unicorn `uc_mem_read` semantics.
    pub fn mem_read(&mut self, addr: u64, buf: &mut [u8]) -> Result<()> {
        let copied = self.memory.read_ram(
            addr,
            buf,
        )?;
        if copied != buf.len() {
            return Err(Error::Memory(crate::memory::MemoryError::ReadPhysicalPage {
                addr,
                len: buf.len(),
            }));
        }
        Ok(())
    }

    #[cfg(feature = "alloc")]
    pub fn mem_read_vec(&mut self, addr: u64, size: usize) -> Result<Vec<u8>> {
        let mut v = alloc::vec![0u8; size];
        self.mem_read(addr, &mut v)?;
        Ok(v)
    }

    /// Write bytes to guest physical RAM, including swapped blocks.
    pub fn mem_write(&mut self, addr: u64, data: &[u8]) -> Result<()> {
        let copied = self.memory.write_ram(
            addr,
            data,
        )?;
        if copied != data.len() {
            return Err(Error::Memory(crate::memory::MemoryError::WritePhysicalPage {
                addr,
                len: data.len(),
            }));
        }
        Ok(())
    }

    pub fn mem_fill(&mut self, addr: u64, size: usize, byte: u8) -> Result<()> {
        let chunk = [byte; 4096];
        let mut offset = 0usize;
        while offset < size {
            let count = (size - offset).min(chunk.len());
            self.mem_write(
                addr.checked_add(u64::try_from(offset)?)
                    .ok_or(Error::Memory(crate::memory::MemoryError::WritePhysicalPage {
                        addr,
                        len: size,
                    }))?,
                &chunk[..count],
            )?;
            offset += count;
        }
        Ok(())
    }

    /// Guest memory size in bytes.
    pub fn mem_size(&self) -> usize {
        self.memory.get_memory_len()
    }

    /// Set memory permissions for a physical address range.
    /// Creates the permissions bitmap on first call, sizing it to physical memory.
    #[cfg(feature = "instrumentation")]
    pub fn mem_protect(
        &mut self,
        addr: u64,
        size: usize,
        perms: crate::cpu::instrumentation::MemPerms,
    ) {
        let mem_len = self.memory.get_memory_len();
        let pp = self.cpu_mut().page_permissions.get_or_insert_with(|| {
            crate::memory::permissions::PagePermissions::new(mem_len as u64)
        });
        pp.set(addr, size, perms);
    }

    // Typed helpers — reduce noise when loaders build page tables, GDT/IDT
    // entries, stack frames, or TEB/PEB scaffolding.

    pub fn mem_read_u8(&mut self, addr: u64) -> Result<u8> {
        let mut b = [0u8; 1];
        self.mem_read(addr, &mut b)?;
        Ok(b[0])
    }

    pub fn mem_read_u16_le(&mut self, addr: u64) -> Result<u16> {
        let mut b = [0u8; 2];
        self.mem_read(addr, &mut b)?;
        Ok(u16::from_le_bytes(b))
    }

    pub fn mem_read_u32_le(&mut self, addr: u64) -> Result<u32> {
        let mut b = [0u8; 4];
        self.mem_read(addr, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    pub fn mem_read_u64_le(&mut self, addr: u64) -> Result<u64> {
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
    ///
    /// Takes `&mut self` because the walk reads the guest's paging structures
    /// the same way an executing walk does — through the routed physical path,
    /// which can page a block back in under partial residency.
    pub fn virt_to_phys(&mut self, vaddr: u64) -> Result<u64> {
        self.exec_ctx(0)
            .translate_linear_system_read(vaddr)
            .map_err(Error::Cpu)
    }

    /// Read bytes from guest VIRTUAL memory. Translates through current
    /// page tables, then reads from the resulting physical address.
    /// Handles page-crossing reads by translating each page separately.
    pub fn virt_read(&mut self, vaddr: u64, buf: &mut [u8]) -> Result<()> {
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

    /// Read bytes from guest virtual memory using an explicit CR3 for
    /// translation instead of the current CR3. Useful for reading user-space
    /// strings after the kernel has swapped CR3 (KPTI).
    /// Long mode only. Returns `Err` on translation failure.
    pub fn virt_read_with_cr3(&mut self, vaddr: u64, cr3: u64, buf: &mut [u8]) -> Result<()> {
        let mut offset = 0;
        while offset < buf.len() {
            let va = vaddr + offset as u64;
            let page_offset = (va & 0xFFF) as usize;
            let chunk = (0x1000 - page_offset).min(buf.len() - offset);
            let pa = self
                .exec_ctx(0)
                .translate_linear_with_cr3(va, cr3)
                .ok_or_else(|| Error::Memory(crate::memory::MemoryError::PageNotPresent))?;
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
    #[cfg(feature = "alloc")]
    pub fn virt_read_vec(&mut self, vaddr: u64, size: usize) -> Result<Vec<u8>> {
        let mut v = alloc::vec![0u8; size];
        self.virt_read(vaddr, &mut v)?;
        Ok(v)
    }

    pub fn virt_read_u8(&mut self, vaddr: u64) -> Result<u8> {
        let mut b = [0u8; 1];
        self.virt_read(vaddr, &mut b)?;
        Ok(b[0])
    }

    pub fn virt_read_u16_le(&mut self, vaddr: u64) -> Result<u16> {
        let mut b = [0u8; 2];
        self.virt_read(vaddr, &mut b)?;
        Ok(u16::from_le_bytes(b))
    }

    pub fn virt_read_u32_le(&mut self, vaddr: u64) -> Result<u32> {
        let mut b = [0u8; 4];
        self.virt_read(vaddr, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    pub fn virt_read_u64_le(&mut self, vaddr: u64) -> Result<u64> {
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

impl<'a, T: crate::cpu::instrumentation::Instrumentation> Emulator<T> {
    /// Obtain a cross-thread [`StopHandle`] that breaks the `emu_start` loop
    /// at its next batch boundary.
    #[cfg(feature = "alloc")]
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
    #[cfg(feature = "alloc")]
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
        self.cpu_mut().api_reg_write(X86Reg::Rip, begin);

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

        // An address the caller wants execution to stop AT can only be
        // honoured by looking after every instruction: a batch that runs past
        // it leaves RIP somewhere else and the address is simply missed. So a
        // run that watches addresses single-steps, and one that does not keeps
        // the full batch. The caller opts into the cost by asking for the
        // precision.
        let watching_addresses = until.is_some() || !self.exit_set.is_empty();
        let stride = if watching_addresses { 1 } else { BATCH };

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
            if self.cpu().is_in_shutdown() {
                return Ok(EmuStopReason::Shutdown);
            }

            let budget = match count {
                Some(c) => stride.min(c - executed),
                None => stride,
            };
            let outcome = if watching_addresses {
                self.step_exactly(budget)?
            } else {
                self.step_batch(budget)?
            };
            executed = executed.saturating_add(outcome.executed);

            // The addresses this wrapper watches are its own business, and they
            // outrank the batch's verdict: reaching `until` is why the caller
            // asked to run at all, so it is reported even on a batch that also
            // ran out of budget.
            if until.is_some_and(|a| self.cpu().rip() == a) {
                return Ok(EmuStopReason::ReachedUntil);
            }
            if !self.exit_set.is_empty() {
                let rip = self.cpu().rip();
                if self.exit_set.contains(rip) {
                    return Ok(EmuStopReason::ReachedExit(rip));
                }
            }

            // Everything else the batch already determined. This used to be
            // inferred as `executed == 0 && is_waiting_for_event()`, which
            // could not see a guest power-off at all and could not tell a
            // halted machine from a batch that simply retired nothing.
            match outcome.stop {
                StopReason::GuestPowerOff | StopReason::StopRequested => {
                    return Ok(EmuStopReason::Stopped)
                }
                StopReason::CpuShutdown => return Ok(EmuStopReason::Shutdown),
                StopReason::Halted => return Ok(EmuStopReason::Halted),
                StopReason::BudgetExhausted => {}
            }
        }
    }

    /// Execute exactly one instruction from the current RIP.
    #[cfg(feature = "alloc")]
    pub fn step_one(&mut self) -> Result<()>
    where
        'a: 'static,
    {
        self.stop_flag.store(false, Ordering::Relaxed);
        // The outcome is machine state the caller can read back at will — the
        // retired count through the CPU's instruction counter, the stop cause
        // through the activity state and the stop flag — so nothing is lost by
        // not returning it from a one-instruction step. It goes through the
        // strict path: `step_batch(1)` would treat the 1 as an inner batch and
        // keep running for its wall-clock budget, which is not a step.
        self.step_exactly(1)?;
        Ok(())
    }
}

// ─────────────────────────── CpuSetupMode builders ───────────────────────────

#[cfg(feature = "alloc")]
impl<'a> Emulator<()> {
    /// Create a new emulator with guest memory allocated but no BIOS loaded,
    /// pre-configured for the given CPU mode. See [`CpuSetupMode`].
    ///
    /// Returns `Box<Self>` because `Emulator` is ~1.4 MB — stack allocation
    /// would silently overflow on most platforms.
    pub fn new_with_mode(config: EmulatorConfig, mode: CpuSetupMode) -> Result<Box<Self>> {
        let mut emu = Self::new(config)?;
        // Minimal init: memory + CPU registers + async event flags. We skip
        // load_bios + pc_system.start etc. since the user will not run a BIOS.
        emu.init_memory_and_pc_system()?;
        // Bring every configured CPU to reset state before applying the mode to the BSP.
        emu.reset(ResetReason::Hardware)?;
        emu.setup_cpu_mode(mode)?;
        Ok(emu)
    }
}

#[cfg(feature = "alloc")]
impl<'a, T: crate::cpu::instrumentation::Instrumentation> Emulator<T> {
    /// Create a new emulator pre-configured for the given CPU mode with a
    /// monomorphized tracer. Combines `new_with_instrumentation` + `setup_cpu_mode`.
    pub fn new_with_mode_and_instrumentation(
        config: EmulatorConfig,
        mode: CpuSetupMode,
        tracer: T,
    ) -> Result<Box<Self>> {
        let mut emu = Self::with_tracer(config, tracer)?;
        emu.init_memory_and_pc_system()?;
        emu.reset(crate::cpu::ResetReason::Hardware)?;
        emu.setup_cpu_mode(mode)?;
        Ok(emu)
    }
}

impl<'a, T: crate::cpu::instrumentation::Instrumentation> Emulator<T> {
    /// Reconfigure an existing emulator for the given CPU mode, skipping BIOS.
    /// The machine must already have memory and a PC system, which is what
    /// [`Emulator::new_with_mode`] arranges.
    pub fn setup_cpu_mode(&mut self, mode: CpuSetupMode) -> Result<()> {
        match mode {
            CpuSetupMode::RealMode => self.setup_real_mode(),
            CpuSetupMode::Protected16 => self.setup_protected16(),
            CpuSetupMode::FlatProtected32 => self.setup_flat_protected32(),
            CpuSetupMode::FlatLong64 => self.setup_flat_long64(),
        }
    }

    /// Real mode with every segment based at zero, A20 enabled and EFLAGS
    /// sane — a machine where an address a caller loads code at is the address
    /// the guest fetches from.
    ///
    /// Reset alone is not enough, and the difference is not cosmetic. The
    /// architectural power-on value of CS is selector 0xF000 with base
    /// 0xFFFF0000, so a caller that loads code low and sets RIP fetches from
    /// the top of the ROM aperture instead. That aperture is filled with 0xFF,
    /// and `FF FF` is an invalid opcode, so such a guest takes #UD on its very
    /// first instruction and never executes a byte of what was loaded — while
    /// looking, from the outside, like a guest that ran and vectored somewhere.
    /// Every documented use of this mode (MBR, DOS binaries, real-mode
    /// shellcode) loads low, so the reset CS is wrong for all of them.
    fn setup_real_mode(&mut self) -> Result<()> {
        // Selector 0, base 0, the 64 KiB limit real mode gives every segment,
        // and 16-bit sizing on all six — Bochs cpu.cc `reset`. SS is the one
        // that bites: with B set, a push writes at ESP rather than SP, walks
        // straight off the 64 KiB limit and raises #SS, so the machine cannot
        // take an interrupt or make a call at all.
        for reg in [
            X86Reg::Cs,
            X86Reg::Ds,
            X86Reg::Es,
            X86Reg::Ss,
            X86Reg::Fs,
            X86Reg::Gs,
        ] {
            self.cpu_mut()
                .set_seg_for_api(reg, 0, 0, 0xFFFF, SegmentSize::Bits16);
        }
        self.memory.set_a20_mask(0xFFFFFFFFFFFFFFFF);
        self.cpu_mut().set_rflags_for_api(0x0000_0202); // IF=1, bit1 reserved=1
        Ok(())
    }

    /// Build a minimal GDT, set descriptor caches, and switch to CR0.PE=1
    /// with 16-bit limits. Rarely used — most callers want `FlatProtected32`.
    fn setup_protected16(&mut self) -> Result<()> {
        self.install_flat_segments(FlatSegments::PROTECTED16)?;
        self.cpu_mut().enter_protected_mode_for_api();
        self.memory.set_a20_mask(0xFFFFFFFFFFFFFFFF);
        self.cpu_mut().set_rflags_for_api(0x0000_0202);
        Ok(())
    }

    /// Flat 32-bit protected mode: CR0.PE=1, segments base=0 limit=4GB,
    /// 32-bit default operand/address size.
    fn setup_flat_protected32(&mut self) -> Result<()> {
        self.install_flat_segments(FlatSegments::FLAT_PROTECTED32)?;
        self.cpu_mut().enter_protected_mode_for_api();
        self.memory.set_a20_mask(0xFFFFFFFFFFFFFFFF);
        self.cpu_mut().set_rflags_for_api(0x0000_0202);
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

        self.install_flat_segments(FlatSegments::FLAT_LONG64)?;
        self.cpu_mut().enter_long_mode_for_api(PML4);
        self.memory.set_a20_mask(0xFFFFFFFFFFFFFFFF);
        self.cpu_mut().set_rflags_for_api(0x0000_0202);
        Ok(())
    }

    /// Install a flat GDT at 0x800 and load every segment's descriptor cache
    /// from the same description, so the two can never disagree about the
    /// machine they describe.
    ///
    /// Both halves in one place is the point. A guest that reloads a segment
    /// register reads the GDT; everything before that runs off the caches.
    /// When the two were written independently, a 16-bit setup handed its
    /// guest 16-bit caches over 32-bit descriptors, and the machine changed
    /// width the first time it touched its own segments.
    fn install_flat_segments(&mut self, segments: FlatSegments) -> Result<()> {
        const GDT_BASE: u64 = 0x0800;
        const CODE_SELECTOR: u16 = 0x08;
        const DATA_SELECTOR: u16 = 0x10;
        // Present, DPL 0, S=1; type 1010 = code exec/read non-conforming,
        // type 0010 = data read/write. Bochs descriptor.h access byte.
        const CODE_ACCESS: u8 = 0x9A;
        const DATA_ACCESS: u8 = 0x92;

        self.mem_write_u64_le(GDT_BASE, 0)?; // null descriptor
        self.mem_write_u64_le(
            GDT_BASE + CODE_SELECTOR as u64,
            flat_descriptor(CODE_ACCESS, segments.code, segments.limit),
        )?;
        self.mem_write_u64_le(
            GDT_BASE + DATA_SELECTOR as u64,
            flat_descriptor(DATA_ACCESS, segments.data, segments.limit),
        )?;
        self.cpu_mut().set_gdtr_base_for_api(GDT_BASE);
        self.cpu_mut().set_gdtr_limit_for_api(0x1F);

        self.cpu_mut().set_seg_for_api(
            X86Reg::Cs,
            CODE_SELECTOR,
            0,
            segments.limit,
            segments.code,
        );
        for reg in [X86Reg::Ds, X86Reg::Es, X86Reg::Ss, X86Reg::Fs, X86Reg::Gs] {
            self.cpu_mut()
                .set_seg_for_api(reg, DATA_SELECTOR, 0, segments.limit, segments.data);
        }
        Ok(())
    }
}

/// The flat segment layout a [`CpuSetupMode`] installs.
///
/// `code` and `data` share a type and long mode is exactly where they differ,
/// so passing them positionally would let a swap compile into a machine whose
/// stack is the wrong width. Naming them — and carrying the limit they share —
/// makes one value the whole description, which is what lets the GDT and the
/// descriptor caches be written from the same source.
#[derive(Clone, Copy)]
struct FlatSegments {
    code: SegmentSize,
    /// Also SS, so this is what decides between `SP` and `ESP`.
    data: SegmentSize,
    /// Byte limit for every segment: the last addressable offset.
    limit: u32,
}

impl FlatSegments {
    const PROTECTED16: Self = Self {
        code: SegmentSize::Bits16,
        data: SegmentSize::Bits16,
        limit: 0xFFFF,
    };
    const FLAT_PROTECTED32: Self = Self {
        code: SegmentSize::Bits32,
        data: SegmentSize::Bits32,
        limit: 0xFFFF_FFFF,
    };
    /// Long mode's data segments stay 32-bit — the descriptors a 64-bit
    /// operating system loads carry D/B set and L clear, because L belongs to
    /// code segments alone.
    const FLAT_LONG64: Self = Self {
        code: SegmentSize::Long64,
        data: SegmentSize::Bits32,
        limit: 0xFFFF_FFFF,
    };
}

/// Compose a base-zero GDT descriptor. Bochs descriptor.h `parse_descriptor`
/// read in reverse: limit low 16, access byte at 40, limit high 4 at 48, then
/// AVL/L/D/G, then base high — all of base being zero here.
fn flat_descriptor(access: u8, size: SegmentSize, byte_limit: u32) -> u64 {
    let limit = ScaledLimit::of(byte_limit);
    u64::from(limit.field & 0xFFFF)
        | (u64::from(access) << 40)
        | ((u64::from(limit.field >> 16) & 0xF) << 48)
        | (u64::from(size.long64()) << 53)
        | (u64::from(size.d_b()) << 54)
        | (u64::from(limit.page_granular) << 55)
}

// ─────────────────────────── Tests ───────────────────────────

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::emulator::MemorySize;

    /// Reg read/write round-trip on a fresh emulator.
    #[test]
    fn reg_read_write_round_trip() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::new(config).unwrap();
                emu.reg_write(X86Reg::Rax, 0xDEAD_BEEF_CAFE_BABE);
                assert_eq!(emu.reg_read(X86Reg::Rax), 0xDEAD_BEEF_CAFE_BABE);
                assert_eq!(emu.reg_read(X86Reg::Eax), 0xCAFE_BABE);
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
                let mut emu = Emulator::new(config).unwrap();
                emu.initialize().unwrap();
                let data: [u8; 16] = [
                    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D,
                    0x0E, 0x0F, 0x10,
                ];
                emu.mem_write(0x20_000, &data).unwrap();
                let mut buf = [0u8; 16];
                emu.mem_read(0x20_000, &mut buf).unwrap();
                assert_eq!(buf, data);

                // Typed helpers
                emu.mem_write_u64_le(0x20_000, 0xCAFE_BABE_DEAD_BEEF)
                    .unwrap();
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
                let emu = Emulator::new(config).unwrap();
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
                let emu =
                    Emulator::new_with_mode(cfg, CpuSetupMode::FlatProtected32)
                        .unwrap();
                // CR0.PE should be set
                let cr0 = emu.reg_read(X86Reg::Cr0);
                assert!(
                    cr0 & 0x1 != 0,
                    "CR0.PE not set after FlatProtected32 setup: {:#x}",
                    cr0
                );
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
                let emu = Emulator::new_with_mode(cfg, CpuSetupMode::FlatLong64)
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
                let mut emu = Emulator::new(cfg).unwrap();
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
                let mut emu = Emulator::new(cfg).unwrap();
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
                let mut emu = Emulator::new(cfg).unwrap();
                let val: [u8; 16] = [
                    0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE, 0x01, 0x02, 0x03, 0x04, 0x05,
                    0x06, 0x07, 0x08,
                ];
                emu.reg_write_xmm(X86Reg::Xmm5, val);
                assert_eq!(
                    emu.reg_read_xmm(X86Reg::Xmm5),
                    val,
                    "XMM5 round-trip failed"
                );
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
                let mut emu = Emulator::new(cfg).unwrap();
                let mut val = [0u8; 32];
                for (i, b) in val.iter_mut().enumerate() {
                    *b = i as u8;
                }
                emu.reg_write_ymm(X86Reg::Ymm3, val);
                assert_eq!(
                    emu.reg_read_ymm(X86Reg::Ymm3),
                    val,
                    "YMM3 round-trip failed"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn vperm2f128_ubuntu_sha512_bytes_execute_bochs_ordering() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let cfg = EmulatorConfig::default();
                let mut emu =
                    Emulator::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                        .unwrap();
                let code_addr = 0x20_0000;

                // CR4.OSFXSR | CR4.OSXSAVE, then XSETBV to put XCR0 at
                // FPU|SSE|YMM. Both are needed: with CR4.OSXSAVE set but XCR0
                // still at its reset value, every VEX encoding #UDs — that is
                // what Bochs `BxNoAVX` tests, and rusty_box applies the same
                // gate at icache fill.
                emu.reg_write(
                    X86Reg::Cr4,
                    emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
                );
                emu.reg_write(X86Reg::Rax, 0x7);
                emu.reg_write(X86Reg::Rcx, 0);
                emu.reg_write(X86Reg::Rdx, 0);
                emu.mem_write(code_addr, &[0x0F, 0x01, 0xD1]).unwrap();
                emu.emu_start(code_addr, None, None, Some(1)).unwrap();

                let mut ymm6 = [0u8; 32];
                let mut ymm7 = [0u8; 32];
                for i in 0..32 {
                    ymm6[i] = 0x20 + i as u8;
                    ymm7[i] = 0xa0 + i as u8;
                }
                emu.reg_write_ymm(X86Reg::Ymm6, ymm6);
                emu.reg_write_ymm(X86Reg::Ymm7, ymm7);

                emu.mem_write(code_addr, &[0xC4, 0xE3, 0x45, 0x06, 0xC6, 0x03])
                    .unwrap();
                emu.emu_start(code_addr, None, None, Some(1)).unwrap();

                let mut expected = [0u8; 32];
                expected[..16].copy_from_slice(&ymm6[16..32]);
                expected[16..32].copy_from_slice(&ymm7[..16]);
                assert_eq!(emu.reg_read_ymm(X86Reg::Ymm0), expected);
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
                let mut emu = Emulator::new(cfg).unwrap();
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
                let mut emu = Emulator::new(cfg).unwrap();
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
                let mut emu = Emulator::new(cfg).unwrap();
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
        use crate::cpu::instrumentation::MemPerms;
        use crate::memory::permissions::PagePermissions;
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

    /// Every modelled UART is reachable and nothing past the set is.
    ///
    /// The index bound is the only real logic in `serial()` — everything else
    /// delegates — so this is where an off-by-one would land, and an
    /// out-of-range index used to panic on a slice index instead of answering
    /// `None`.
    #[test]
    fn serial_handle_covers_exactly_the_modelled_ports() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::new(EmulatorConfig::default()).unwrap();
                for port in 0..SERIAL_PORT_COUNT {
                    assert!(
                        emu.serial(port).is_some(),
                        "COM{} is modelled and must be reachable",
                        port + 1
                    );
                }
                assert!(emu.serial(SERIAL_PORT_COUNT).is_none());
                assert!(emu.serial(usize::MAX).is_none());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Bytes the guest writes to port 0xE9 come back out of the debug-port
    /// handle, in write order.
    ///
    /// Asserts the guest-visible property (doctrine R9) rather than the
    /// buffer that currently carries it: the guest executes real `OUT`
    /// instructions and the host reads them through the public role handle,
    /// so a mis-wired handle or a lost byte fails here.
    #[test]
    fn debug_port_handle_returns_guest_written_bytes() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut emu =
                    Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
                        .unwrap();
                let code_addr = 0x20_0000;

                // mov al,'O'; out 0xe9,al; mov al,'K'; out 0xe9,al
                emu.mem_write(
                    code_addr,
                    &[0xB0, b'O', 0xE6, 0xE9, 0xB0, b'K', 0xE6, 0xE9],
                )
                .unwrap();
                emu.emu_start(code_addr, None, None, Some(4)).unwrap();

                let out: Vec<u8> = emu.debug_port().take_output().collect();
                assert_eq!(out, b"OK", "port-0xE9 writes must survive in order");

                // Draining is destructive — a second read sees nothing new.
                assert!(emu.debug_port().take_output().next().is_none());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// MMIO registry map/unmap.
    #[test]
    fn mmio_registry_map_unmap() {
        use crate::memory::mmio::MmioRegistry;
        let mut reg = MmioRegistry::new();
        assert!(reg.is_empty());
        reg.map(
            0xFEC0_0000,
            0x1000,
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

    /// Guest RAM larger than host RAM, with code and data in different guest
    /// blocks: every data access relocates a block, and the code block itself
    /// is a legal eviction victim. Forward progress here rests on the residency
    /// epoch (memory/mod.rs `swap_epoch`) retiring the stale fetch window.
    ///
    /// `setup_flat_long64` owns 0x1000..0x6000 for the page tables, so guest
    /// code goes above them — code written over the PML4 unmaps the address it
    /// is executing from and triple-faults before any of this is exercised.
    #[test]
    fn swap_regime_executes_across_blocks() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                // 4 MiB guest over 2 MiB host = two resident 1 MiB slots.
                let cfg = EmulatorConfig {
                    memory: MemorySize::partially_resident(4 * MIB, 2 * MIB),
                    memory_block_size: MIB,
                    ..Default::default()
                };
                let mut emu =
                    Emulator::new_with_mode(cfg, CpuSetupMode::FlatLong64).unwrap();
                let code = 0x10000u64;
                let resident_epoch = emu.memory.swap_epoch();
                emu.mem_write(
                    code,
                    &[
                        0x8A, 0x04, 0x25, 0x00, 0x00, 0x10, 0x00, // mov al,[0x100000] (block 1)
                        0x8A, 0x04, 0x25, 0x00, 0x00, 0x20, 0x00, // mov al,[0x200000] (block 2)
                        0xBB, 0xED, 0x5E, 0x00, 0x00, // mov ebx,0x5EED
                        0xF4, // hlt
                    ],
                )
                .unwrap();
                assert_eq!(
                    emu.emu_start(code, None, None, Some(8)).unwrap(),
                    EmuStopReason::Halted
                );
                assert_eq!(emu.reg_read(X86Reg::Ebx), 0x5EED);
                assert!(
                    emu.memory.swap_epoch() > resident_epoch,
                    "the run must actually have relocated blocks, else this \
                     configuration is not testing the swap regime at all"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// The same regime driven hard: four times as many guest blocks as resident
    /// slots, in a loop, so the code block is evicted and reloaded repeatedly
    /// and every data byte makes a round trip through the overflow file.
    #[test]
    fn swap_regime_survives_repeated_code_block_eviction() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                // 8 MiB guest over 2 MiB host = eight blocks, two slots.
                let cfg = EmulatorConfig {
                    memory: MemorySize::partially_resident(8 * MIB, 2 * MIB),
                    memory_block_size: MIB,
                    ..Default::default()
                };
                let mut emu =
                    Emulator::new_with_mode(cfg, CpuSetupMode::FlatLong64).unwrap();

                // One marker per non-code block, so a byte that survives an
                // eviction round trip is the only way to read it back.
                for block in 1..8u64 {
                    emu.mem_write_u8(block * MIB as u64, 0x10 * block as u8 + block as u8)
                        .unwrap();
                }

                let code = 0x10000u64;
                let mut program = vec![0xB9, 0x03, 0x00, 0x00, 0x00]; // mov ecx,3
                for block in 1..8u64 {
                    // mov al,[block * 1 MiB]
                    program.extend_from_slice(&[0x8A, 0x04, 0x25]);
                    program.extend_from_slice(&((block * MIB as u64) as u32).to_le_bytes());
                }
                program.extend_from_slice(&[0xFF, 0xC9]); // dec ecx
                let back = -((program.len() + 2 - 5) as i64) as i8; // to the loop top
                program.extend_from_slice(&[0x75, back as u8]); // jnz top
                program.extend_from_slice(&[0xBB, 0xED, 0x5E, 0x00, 0x00]); // mov ebx,0x5EED
                program.push(0xF4); // hlt
                emu.mem_write(code, &program).unwrap();

                assert_eq!(
                    emu.emu_start(code, None, None, Some(200)).unwrap(),
                    EmuStopReason::Halted
                );
                assert_eq!(emu.reg_read(X86Reg::Ebx), 0x5EED);
                assert_eq!(emu.reg_read(X86Reg::Ecx), 0, "the loop must have run to zero");
                assert_eq!(
                    emu.reg_read(X86Reg::Rax) & 0xFF,
                    0x77,
                    "the last load must read the marker its block was swapped out with"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// The two residency topologies the tests above do not reach: a single
    /// resident slot, where code, page tables and operands all take turns in
    /// the same 1 MiB of host RAM; and code living in a guest block that is
    /// neither block 0 nor the block holding the page tables, so one fetch
    /// needs the page-table block and the code block resident in sequence.
    ///
    /// Capacity 1 is the interesting bound: every fetch is guaranteed to evict
    /// what the previous access just paged in, so nothing but the epoch
    /// handshake keeps the loop moving forward.
    #[test]
    fn swap_regime_converges_at_minimum_residency() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                // (guest MiB, host MiB, code address, marker address, marker)
                let cases = [
                    (4usize, 1usize, 0x10000u64, 3 * MIB as u64, 0x33u8),
                    (8, 2, 5 * MIB as u64 + 0x10000, 7 * MIB as u64, 0x77),
                ];
                for (guest_mib, host_mib, code, marker_addr, marker) in cases {
                    let cfg = EmulatorConfig {
                        memory: MemorySize::partially_resident(guest_mib * MIB, host_mib * MIB),
                        memory_block_size: MIB,
                        ..Default::default()
                    };
                    let mut emu =
                        Emulator::new_with_mode(cfg, CpuSetupMode::FlatLong64).unwrap();
                    emu.mem_write_u8(marker_addr, marker).unwrap();
                    let far = (marker_addr as u32).to_le_bytes();
                    let mut program = vec![
                        0xB9, 0x02, 0x00, 0x00, 0x00, // mov ecx,2
                        0x8A, 0x04, 0x25, 0x00, 0x00, 0x10, 0x00, // mov al,[0x100000]
                        0x8A, 0x04, 0x25, // mov al,[marker]
                    ];
                    program.extend_from_slice(&far);
                    program.extend_from_slice(&[
                        0xFF, 0xC9, // dec ecx
                        0x75, 0xF3, // jnz back to the first load
                        0xBB, 0xED, 0x5E, 0x00, 0x00, // mov ebx,0x5EED
                        0xF4, // hlt
                    ]);
                    emu.mem_write(code, &program).unwrap();

                    assert_eq!(
                        emu.emu_start(code, None, None, Some(100)).unwrap(),
                        EmuStopReason::Halted,
                        "{guest_mib} MiB guest over {host_mib} MiB host must still retire"
                    );
                    assert_eq!(emu.reg_read(X86Reg::Ebx), 0x5EED);
                    assert_eq!(
                        emu.reg_read(X86Reg::Rax) & 0xFF,
                        u64::from(marker),
                        "the marker must survive its block's eviction round trip"
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// An instruction whose own bytes straddle a guest-block boundary, at every
    /// residency from one slot to full. This is the `boundary_fetch` path: it
    /// copies the head of the instruction out of the current window, then calls
    /// `prefetch` again for the tail — a call that may itself relocate blocks
    /// while the instruction is half-fetched.
    ///
    /// The 2 MiB mark is the boundary to use: it crosses a 4 KiB page, a 2 MiB
    /// large page and a 1 MiB guest block at once. The 1 MiB mark would not —
    /// it is the top of the legacy 0xA0000..0xFFFFF VGA/BIOS window, which is
    /// not plain RAM, so code placed there never lands.
    #[test]
    fn swap_regime_fetches_instruction_across_a_block_boundary() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                for host_mib in [1usize, 2, 4] {
                    let cfg = EmulatorConfig {
                        memory: MemorySize::partially_resident(4 * MIB, host_mib * MIB),
                        memory_block_size: MIB,
                        ..Default::default()
                    };
                    let mut emu =
                        Emulator::new_with_mode(cfg, CpuSetupMode::FlatLong64).unwrap();
                    emu.mem_write_u8(3 * MIB as u64, 0x33).unwrap();
                    // The load ends at 2 MiB - 2, so `mov ebx,imm32` straddles
                    // the block 1 / block 2 boundary and the HLT lands past it.
                    let code = 2 * MIB as u64 - 9;
                    emu.mem_write(
                        code,
                        &[
                            0x8A, 0x04, 0x25, 0x00, 0x00, 0x30, 0x00, // mov al,[0x300000]
                            0xBB, 0xED, 0x5E, 0x00, 0x00, // mov ebx,0x5EED (straddles)
                            0xF4, // hlt
                        ],
                    )
                    .unwrap();
                    assert_eq!(
                        emu.emu_start(code, None, None, Some(20)).unwrap(),
                        EmuStopReason::Halted,
                        "{host_mib} MiB host: the split instruction must retire"
                    );
                    assert_eq!(
                        emu.reg_read(X86Reg::Ebx),
                        0x5EED,
                        "{host_mib} MiB host: the immediate comes from the second block"
                    );
                    assert_eq!(emu.reg_read(X86Reg::Rax) & 0xFF, 0x33);
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// A triple fault raised while FETCHING (not while executing) still stops
    /// the CPU. Writing code over the PML4 unmaps the very page RIP points at,
    /// so the fetch page walk faults, delivering #PF faults again on the IDT
    /// read, and the resulting #DF faults once more: shutdown before a single
    /// instruction retires.
    ///
    /// Bochs signals this through `enter_sleep_state` (proc_ctrl.cc), which
    /// raises the generic `async_event` flag so the top of `cpu_loop` observes
    /// the non-ACTIVE activity state no matter which longjmp arrived there.
    #[test]
    fn triple_fault_during_fetch_reports_shutdown() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                let cfg = EmulatorConfig {
                    memory: MemorySize::bytes(4 * MIB),
                    memory_block_size: MIB,
                    ..Default::default()
                };
                let mut emu =
                    Emulator::new_with_mode(cfg, CpuSetupMode::FlatLong64).unwrap();
                // 0x1000 is the PML4 root: this write makes PML4[0] non-present.
                let code = 0x1000u64;
                emu.mem_write(code, &[0xBB, 0xED, 0x5E, 0x00, 0x00, 0xF4])
                    .unwrap();
                assert_eq!(
                    emu.emu_start(code, None, None, Some(8)).unwrap(),
                    EmuStopReason::Shutdown,
                    "a triple fault during instruction fetch must stop the CPU"
                );
                assert!(emu.cpu().is_in_shutdown());
                // Bochs `enter_sleep_state` clears IF for SHUTDOWN, so nothing
                // but NMI/SMI/INIT can wake the CPU again.
                assert_eq!(
                    emu.reg_read(X86Reg::Eflags) & 0x200,
                    0,
                    "shutdown must mask interrupts"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
