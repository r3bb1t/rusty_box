//! Software interrupt instructions for x86 CPU emulation
//!
//! Based on Bochs soft_int.cc
//!
//! Implements INT, INT3, INTO, IRET instructions

use super::{
    cpu::{BxCpuC, CpuActivityState, Exception, BX_ASYNC_EVENT_STOP_TRACE},
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    descriptor::BxSelector,
    eflags::EFlags,
    segment_ctrl_pro::parse_selector,
};

/// `CPUID.7:0:EBX` — AVX512F, DQ, IFMA, PF, ER, CD, BW and VL.
const CPUID_LEAF7_EBX_AVX512: u32 = (1 << 16)
    | (1 << 17)
    | (1 << 21)
    | (1 << 26)
    | (1 << 27)
    | (1 << 28)
    | (1 << 30)
    | (1 << 31);
/// `CPUID.7:0:ECX` — VBMI, VBMI2, VNNI, BITALG and VPOPCNTDQ.
const CPUID_LEAF7_ECX_AVX512: u32 = (1 << 1) | (1 << 6) | (1 << 11) | (1 << 12) | (1 << 14);
/// `CPUID.7:0:EDX` — 4VNNIW, 4FMAPS, VP2INTERSECT and FP16.
const CPUID_LEAF7_EDX_AVX512: u32 = (1 << 2) | (1 << 3) | (1 << 8) | (1 << 23);

const CPUID_LEAF_FEATURE_INFO: u32 = 0x0000_0001;
const CPUID_LEAF_EXTENDED_TOPOLOGY: u32 = 0x0000_000B;
const CPUID_OSXSAVE_ECX_BIT: u32 = 1 << 27;
const CPUID_APIC_EDX_BIT: u32 = 1 << 9;
/// `CPUID.1:ECX[3]`, `MONITOR`/`MWAIT`.
const CPUID_MONITOR_ECX_BIT: u32 = 1 << 3;
const CPUID_LEAF1_EBX_LOW_FIELDS_MASK: u32 = 0x0000_FFFF;
const CPUID_LEAF1_LOGICAL_COUNT_SHIFT: u32 = 16;
const CPUID_LEAF1_APIC_ID_SHIFT: u32 = 24;
const CPUID_APIC_ID_BYTE_MASK: u32 = 0xFF;
const CPUID_TOPOLOGY_SUBLEAF_SMT: u32 = 0;
const CPUID_TOPOLOGY_SUBLEAF_CORE: u32 = 1;
const CPUID_TOPOLOGY_SUBLEAF_PACKAGE: u32 = 2;
const CPUID_TOPOLOGY_LEVEL_TYPE_SHIFT: u32 = 8;
const CPUID_TOPOLOGY_LEVEL_TYPE_SMT: u32 = 1;
const CPUID_TOPOLOGY_LEVEL_TYPE_CORE: u32 = 2;
const CPUID_TOPOLOGY_LEVEL_TYPE_PACKAGE: u32 = 3;

#[inline]
fn topology_level_ecx(subleaf: u32, level_type: u32) -> u32 {
    subleaf | (level_type << CPUID_TOPOLOGY_LEVEL_TYPE_SHIFT)
}

/// `floor(log2(x))`, and 0 for `x == 0`.
///
/// Bochs cpuid.cc `ilog2` counts how many times `x` can be shifted right before
/// it reaches zero, so `ilog2(0) == 0` — not a mathematical log, and the reason
/// [`bochs_topology_shift`] does not special-case a single processor.
#[inline]
fn ilog2(x: u32) -> u32 {
    if x == 0 {
        0
    } else {
        u32::BITS - 1 - x.leading_zeros()
    }
}

/// The shift CPUID leaf 0xB reports in EAX for a topology level holding
/// `logical_count` logical processors.
///
/// Bochs cpuid.cc `bx_cpuid_t::get_std_cpuid_extended_topology_leaf` spells this
/// `ilog2(n-1)+1` with no guard for small `n`, so a level holding exactly one
/// logical processor reports 1 rather than 0. Written the same way here so the
/// two cannot drift: the `+1` and the `ilog2(0) == 0` convention are what make
/// the `n == 1` answer come out as Bochs has it.
pub(crate) fn bochs_topology_shift(logical_count: u32) -> u32 {
    ilog2(logical_count.wrapping_sub(1)) + 1
}

impl<T: crate::cpu::instrumentation::Instrumentation> crate::cpu::exec_ctx::ExecCtx<'_, T> {
    // =========================================================================
    // Unified interrupt dispatch — matches Bochs interrupt() in exception.cc
    // =========================================================================

    /// Unified interrupt dispatch based on CPU mode.
    ///
    /// Mirrors Bochs `BX_CPU_C::interrupt()` in exception.cc.
    /// Dispatches to real_mode_int or protected_mode_int based on current CPU mode.
    /// After delivery, invalidates prefetch and returns CpuLoopRestart to
    /// restart the trace (matching Bochs BX_NEXT_TRACE).
    pub(super) fn interrupt(
        &mut self,
        vector: u8,
        event_type: super::exception::InterruptType,
        soft_int: bool,
        push_error: bool,
        error_code: u16,
    ) -> super::Result<()> {
        tracing::trace!(
            "interrupt(): vector={:#04x} soft_int={} mode={}",
            vector,
            soft_int,
            if self.real_mode() {
                "real"
            } else {
                "protected"
            }
        );
        // BOCHS BX_INSTR_INTERRUPT(cpu_id, vector)
        if self.instrumentation.active.has_interrupt() {
            self.instrumentation.fire_interrupt(vector);
        }

        // Taking an interrupt always resumes execution, so a CPU that was
        // halted or in MWAIT is running again from here on. Bochs exception.cc
        // interrupt() sets this before delivery because delivery can leave via
        // a fault or VM exit and never fall back to handleAsyncEvent's tail.
        self.activity_state = super::cpu::CpuActivityState::Active;

        // Discard any traps and inhibits for new context (matches Bochs line 800-801)
        self.debug_trap = 0;
        self.inhibit_mask = 0;

        // Invalidate prefetch queue (matches Bochs line 777)
        self.eip_fetch_window = None;
        self.eip_page_window_size = 0;

        // RSP_SPECULATIVE — mark speculative RSP so exceptions during delivery
        // can restore the original value (matches Bochs line 807)
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp(); // Bochs: prev_rsp = RSP (full 64-bit)
        self.prev_ssp = 0; // no shadow stack

        if self.real_mode() {
            self.interrupt_real_mode(vector)?;
        } else {
            // V8086 mode software interrupt: try VME redirect first
            // Bochs exception.cc: v86_redirect_interrupt checked before protected_mode_int
            if self.v8086_mode() && soft_int && self.v86_redirect_interrupt(vector)? {
                // Interrupt was redirected through virtual IVT
                self.speculative_rsp = false;
                self.ext = false;
                self.async_event |= BX_ASYNC_EVENT_STOP_TRACE;
                return Err(super::error::CpuError::CpuLoopRestart);
            }

            // Long mode: dispatch through 16-byte IDT entries
            // Protected mode (or V86 non-redirected): dispatch through 8-byte IDT entries
            let delivery_result = if self.long_mode() {
                if self.cr4.fred() {
                    self.fred_event_delivery(vector, event_type, error_code)
                } else {
                    self.long_mode_int(vector, soft_int, push_error, error_code)
                }
            } else {
                self.protected_mode_int(vector, soft_int, push_error, error_code)
            };
            match delivery_result {
                Ok(()) => {}
                Err(super::error::CpuError::BadVector {
                    vector: new_vector,
                    error_code: new_error_code,
                }) => {
                    // Delivery failed — raise the indicated exception.
                    tracing::trace!(
                        "interrupt({:#04x}) PM delivery failed, raising {:?} error_code={:#x}; icount={}",
                        vector,
                        new_vector,
                        new_error_code,
                        self.icount
                    );
                    return self.exception(new_vector, new_error_code);
                }
                Err(e) => return Err(e),
            }
        }

        // RSP_COMMIT (matches Bochs line 828)
        self.speculative_rsp = false;

        // EXT = 0 after delivery (matches Bochs line 838)
        self.ext = false;

        // Software interrupts cause trace restart (matches Bochs BX_NEXT_TRACE)
        self.async_event |= BX_ASYNC_EVENT_STOP_TRACE;
        Err(super::error::CpuError::CpuLoopRestart)
    }

    // =========================================================================
    // INT - Software Interrupt
    // =========================================================================

    /// INT imm8 - Software interrupt with immediate vector
    /// Based on Bochs INT_Ib in soft_int.cc
    pub fn int_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        // Bochs svm.cc SVM_INTERCEPT0_SOFTINT — EXITINFO1 carries the vector.
        if self.in_svm_guest && self.svm_intercept_check(super::svm::SVM_INTERCEPT0_SOFTINT) {
            let vec = instr.ib() as u64;
            return self.svm_vmexit(super::svm::SvmVmexit::SoftwareInterrupt as i32, vec, 0);
        }
        let vector = instr.ib();
        tracing::trace!("INT {:#04x}", vector);
        // BX_SOFTWARE_INTERRUPT → soft_int=true, no error code
        self.interrupt(
            vector,
            super::exception::InterruptType::SoftwareInterrupt,
            true,
            false,
            0,
        )
    }

    /// INT3 - Breakpoint interrupt (vector 3)
    /// Based on Bochs INT3 in soft_int.cc
    pub fn int3(&mut self, _instr: &Instruction) -> super::Result<()> {
        tracing::trace!("INT3 (breakpoint)");
        // BX_SOFTWARE_EXCEPTION → soft_int=true, no error code
        self.interrupt(
            3,
            super::exception::InterruptType::SoftwareException,
            true,
            false,
            0,
        )
    }

    /// INTO - Interrupt on overflow (vector 4, only if OF=1)
    /// Based on Bochs INTO in soft_int.cc
    ///
    /// Named `into_overflow` rather than `into`: the mnemonic collides with
    /// `Into::into`, which every type implements. As an inherent method on the
    /// CPU it still won resolution, but reached through a `Deref` — as the
    /// dispatcher now does — the blanket trait method wins instead, and the
    /// call silently stops being this handler.
    pub fn into_overflow(&mut self, _instr: &Instruction) -> super::Result<()> {
        if self.get_of() {
            tracing::trace!("INTO: overflow detected, calling INT 4");
            // BX_SOFTWARE_EXCEPTION → soft_int=true, no error code
            return self.interrupt(
                4,
                super::exception::InterruptType::SoftwareException,
                true,
                false,
                0,
            );
        }
        Ok(())
    }

    /// INT1 (ICEBP) - In-circuit emulator breakpoint (vector 1)
    /// Based on Bochs INT1 in soft_int.cc
    pub fn int1(&mut self, _instr: &Instruction) -> super::Result<()> {
        tracing::trace!(
            "INT1 (ICEBP) at RIP={:#x} CS={:#x}",
            self.rip(),
            self.sregs[crate::cpu::decoder::BxSegregs::Cs as usize]
                .selector
                .value
        );
        // BX_PRIVILEGED_SOFTWARE_INTERRUPT → soft_int=false (privileged bypass DPL check)
        // Bochs sets EXT=1 before calling interrupt() for INT1
        self.ext = true;
        self.interrupt(
            1,
            super::exception::InterruptType::PrivilegedSoftwareInterrupt,
            false,
            false,
            0,
        )
    }

    // =========================================================================
    // BOUND - Check Array Index Against Bounds
    // Based on Bochs soft_int.cc BOUND_GwMa and BOUND_GdMa
    // =========================================================================

    /// BOUND r16, m16&16 - Check 16-bit register against bounds in memory
    ///
    /// Compares the signed value in r16 against the signed lower and upper bounds
    /// at memory location. If the index is out of bounds, generates #BR exception.
    pub fn bound_gw_ma(&mut self, instr: &Instruction) -> super::Result<()> {
        // Get the 16-bit register value (signed)
        let op1_16 = self.get_gpr16(instr.dst() as usize) as i16;

        // Calculate effective address
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);

        // Bochs: (eaddr+2) & i->asize_mask() — mask for 16-bit address wrap
        let asize_mask: u64 = if instr.as64_l() != 0 {
            0xFFFF_FFFF_FFFF_FFFF
        } else if instr.as32_l() == 0 {
            0xFFFF
        } else {
            0xFFFF_FFFF
        };

        // Read lower and upper bounds from memory (2 words)
        let bound_min = self.v_read_word(seg, eaddr)? as i16;
        let bound_max = self.v_read_word(seg, eaddr.wrapping_add(2) & asize_mask)? as i16;

        // Check if value is outside bounds
        if op1_16 < bound_min || op1_16 > bound_max {
            tracing::trace!(
                "BOUND: fails bounds test (value {} not in [{}, {}])",
                op1_16,
                bound_min,
                bound_max
            );
            // Generate #BR exception (Bound Range Exceeded, vector 5)
            // Bochs calls exception(BX_BR_EXCEPTION, 0) — NOT interrupt()
            return self.exception(Exception::Br, 0);
        }
        Ok(())
    }

    /// BOUND r32, m32&32 - Check 32-bit register against bounds in memory
    ///
    /// Compares the signed value in r32 against the signed lower and upper bounds
    /// at memory location. If the index is out of bounds, generates #BR exception.
    pub fn bound_gd_ma(&mut self, instr: &Instruction) -> super::Result<()> {
        // Get the 32-bit register value (signed)
        let op1_32 = self.get_gpr32(instr.dst() as usize) as i32;

        // Calculate effective address
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);

        // Bochs: (eaddr+4) & i->asize_mask() — mask for 16-bit address wrap
        let asize_mask: u64 = if instr.as64_l() != 0 {
            0xFFFF_FFFF_FFFF_FFFF
        } else if instr.as32_l() == 0 {
            0xFFFF
        } else {
            0xFFFF_FFFF
        };

        // Read lower and upper bounds from memory (2 dwords)
        let bound_min = self.v_read_dword(seg, eaddr)? as i32;
        let bound_max = self.v_read_dword(seg, eaddr.wrapping_add(4) & asize_mask)? as i32;

        // Check if value is outside bounds
        if op1_32 < bound_min || op1_32 > bound_max {
            tracing::trace!(
                "BOUND: fails bounds test (value {} not in [{}, {}])",
                op1_32,
                bound_min,
                bound_max
            );
            // Generate #BR exception (Bound Range Exceeded, vector 5)
            // Bochs calls exception(BX_BR_EXCEPTION, 0) — NOT interrupt()
            return self.exception(Exception::Br, 0);
        }
        Ok(())
    }

    // =========================================================================
    // IRET - Interrupt Return
    // =========================================================================

    /// IRET - Return from interrupt (16-bit operand size)
    /// Based on Bochs ctrl_xfer16.cc IRET16
    pub fn iret16(&mut self, _instr: &Instruction) -> super::Result<()> {
        // Bochs svm.cc SVM_INTERCEPT0_IRET.
        if self.in_svm_guest && self.svm_intercept_check(super::svm::SVM_INTERCEPT0_IRET) {
            return self.svm_vmexit(super::svm::SvmVmexit::Iret as i32, 0, 0);
        }
        // Invalidate prefetch queue at entry (Bochs ctrl_xfer16.cc)
        self.invalidate_prefetch_q();

        // Unmask NMI on every IRET (Bochs ctrl_xfer16.cc)
        self.unmask_event(BxCpuC::<T>::BX_EVENT_NMI);

        // RSP_SPECULATIVE before all mode branches (Bochs ctrl_xfer16.cc)
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp(); // Bochs: prev_rsp = RSP (full 64-bit)

        // Protected mode dispatch (Bochs ctrl_xfer16.cc)
        // Bochs checks protected_mode() first, which includes protected+long modes
        // but NOT V8086. V8086 is handled inside iret_protected via
        // iret16_stack_return_from_v86.
        if self.protected_mode() {
            return self.iret_protected_16();
        }

        // V8086 mode IRET (Bochs ctrl_xfer16.cc)
        if self.v8086_mode() {
            return self.iret16_stack_return_from_v86();
        }

        // Real mode: Pop IP, CS, FLAGS from stack
        let new_ip = self.pop_16()?;
        let new_cs = self.pop_16()?;
        let new_flags = self.pop_16()?;

        // Trace IRET from BIOS INT 13h handler during ISOLINUX window
        // The INT 13h wrapper at 0x7F0D calls INT 13h; IRET returns to 0x7F0F
        if self.icount > 1_768_000 && self.icount < 1_772_000 {
            let cs_val = self.sregs[super::decoder::BxSegregs::Cs as usize]
                .selector
                .value;
            if cs_val == 0xF000 {
                let cf = new_flags & 1;
                tracing::trace!(
                    "IRET from BIOS: CS:IP={:04x}:{:04x} → {:04x}:{:04x} FLAGS={:04x} CF={} AH={:#04x} icount={}",
                    cs_val, self.rip() as u16, new_cs, new_ip, new_flags, cf, self.ah(), self.icount
                );
            }
        }

        // CS limit check (Bochs ctrl_xfer16.cc)
        let limit = self.get_segment_limit(BxSegregs::Cs);
        if (new_ip as u32) > limit {
            tracing::error!(
                "iret16: offset {:#06x} outside of CS limits {:#010x}",
                new_ip,
                limit
            );
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Load CS with load_seg_reg (Bochs ctrl_xfer16.cc)
        self.load_seg_reg_real_mode(BxSegregs::Cs, new_cs);

        // Set IP (Bochs ctrl_xfer16.cc)
        self.set_eip(new_ip as u32);

        // write_flags with change_IOPL=true, change_IF=true (Bochs ctrl_xfer16.cc)
        self.write_flags(new_flags, true, true);

        // RSP_COMMIT
        self.speculative_rsp = false;

        tracing::trace!(
            "IRET16: returning to {:04x}:{:04x}, flags={:04x}",
            new_cs,
            new_ip,
            new_flags
        );
        Ok(())
    }

    /// IRET - Return from interrupt (32-bit operand size)
    /// Based on Bochs ctrl_xfer32.cc IRET32
    pub fn iret32(&mut self, _instr: &Instruction) -> super::Result<()> {
        // Bochs svm.cc SVM_INTERCEPT0_IRET.
        if self.in_svm_guest && self.svm_intercept_check(super::svm::SVM_INTERCEPT0_IRET) {
            return self.svm_vmexit(super::svm::SvmVmexit::Iret as i32, 0, 0);
        }
        // Invalidate prefetch queue at entry (Bochs ctrl_xfer32.cc)
        self.invalidate_prefetch_q();

        // Unmask NMI on every IRET (Bochs ctrl_xfer32.cc)
        self.unmask_event(BxCpuC::<T>::BX_EVENT_NMI);

        // RSP_SPECULATIVE before all mode branches (Bochs ctrl_xfer32.cc)
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp(); // Bochs: prev_rsp = RSP (full 64-bit)

        // Protected mode dispatch (Bochs ctrl_xfer32.cc)
        if self.protected_mode() {
            return self.iret_protected();
        }

        // V8086 mode IRET (Bochs ctrl_xfer32.cc)
        if self.v8086_mode() {
            return self.iret32_stack_return_from_v86();
        }

        // Real mode: Pop EIP, CS, EFLAGS from stack
        let new_eip = self.pop_32()?;
        let new_cs = self.pop_32()? as u16;
        let new_eflags = self.pop_32()?;

        // CS limit check (Bochs ctrl_xfer32.cc)
        let limit = self.get_segment_limit(BxSegregs::Cs);
        if new_eip > limit {
            tracing::error!(
                "iret32: offset {:#010x} outside of CS limits {:#010x}",
                new_eip,
                limit
            );
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Load CS with load_seg_reg (Bochs ctrl_xfer32.cc)
        self.load_seg_reg_real_mode(BxSegregs::Cs, new_cs);

        // Set EIP (Bochs ctrl_xfer32.cc)
        self.set_eip(new_eip);

        // writeEFlags with VIF, VIP, VM unchanged (Bochs ctrl_xfer32.cc)
        self.write_eflags(new_eflags, EFlags::IRET32_REAL_CHANGE.bits());

        // RSP_COMMIT
        self.speculative_rsp = false;

        tracing::trace!(
            "IRET32: returning to {:04x}:{:08x}, eflags={:08x}",
            new_cs,
            new_eip,
            new_eflags
        );
        Ok(())
    }

    /// IRET in protected mode (32-bit operand size)
    ///
    /// Based on Bochs iret.cc:iret_protected() with os32=true.
    /// Reads EIP/CS/EFLAGS from stack WITHOUT advancing ESP first, then after all
    /// validation passes loads CS from the GDT (NOT real-mode segment << 4).
    fn iret_protected(&mut self) -> super::Result<()> {
        use super::cpu::Exception;

        // Nested Task (NT) — task-switch IRET
        // Based on Bochs iret.cc
        if self.eflags.contains(EFlags::NT) {
            tracing::trace!("IRET: nested task return (NT=1)");

            // Read back-link selector from current TSS offset 0
            let tss_base = self.tr.cache.u.segment_base();
            let raw_link_selector = self.system_read_word(tss_base)?;

            let mut link_selector = BxSelector::default();
            parse_selector(raw_link_selector, &mut link_selector);

            // Must specify global (TI=0)
            if link_selector.ti != 0 {
                tracing::error!("iret: link selector.ti=1");
                return self.exception(Exception::Ts, raw_link_selector & 0xfffc);
            }

            let (dword1, dword2) = match self.fetch_raw_descriptor(&link_selector) {
                Ok(v) => v,
                Err(_) => {
                    return self.exception(Exception::Ts, raw_link_selector & 0xfffc);
                }
            };
            let tss_descriptor = match self.parse_descriptor(dword1, dword2) {
                Ok(v) => v,
                Err(_) => {
                    return self.exception(Exception::Ts, raw_link_selector & 0xfffc);
                }
            };

            // Must be a busy TSS
            if tss_descriptor.valid == 0 || tss_descriptor.segment {
                tracing::error!("iret: TSS selector points to bad TSS");
                return self.exception(Exception::Ts, raw_link_selector & 0xfffc);
            }
            if tss_descriptor.r#type != 0x3 && tss_descriptor.r#type != 0xB {
                // Must be busy 286 (0x3) or busy 386 (0xB)
                tracing::error!("iret: TSS not busy type={:#x}", tss_descriptor.r#type);
                return self.exception(Exception::Ts, raw_link_selector & 0xfffc);
            }
            if !tss_descriptor.p {
                tracing::error!("iret: task descriptor.p == 0");
                return self.exception(Exception::Np, raw_link_selector & 0xfffc);
            }

            // Switch tasks (without nesting) to TSS specified by back link selector
            return self.task_switch(
                &link_selector,
                &tss_descriptor,
                super::tasking::BX_TASK_FROM_IRET,
                dword1,
                dword2,
                false,
                0,
            );
        }

        // RSP_SPECULATIVE (Bochs iret.cc)
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp(); // Bochs: prev_rsp = RSP (full 64-bit)

        // Peek at stack without modifying ESP
        let temp_esp = if self.is_stack_32bit() {
            self.esp()
        } else {
            self.sp() as u32
        };

        let new_eip = self.stack_read_dword(temp_esp)?;
        let raw_cs_raw = self.stack_read_dword(temp_esp + 4)? as u16;
        let new_eflags = self.stack_read_dword(temp_esp + 8)?;

        // If VM bit is set in the saved EFLAGS and CPL==0, stack-return to V86 mode.
        // Bochs iret.cc
        if (new_eflags & EFlags::VM.bits()) != 0 {
            let current_cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            if current_cpl == 0 {
                self.stack_return_to_v86(new_eip, raw_cs_raw as u32, new_eflags)?;
                self.speculative_rsp = false;
                return Ok(());
            } else {
                tracing::error!("iret_protected: VM bit set but CPL={} != 0", current_cpl);
                return self.exception(Exception::Gp, 0);
            }
        }

        // Return CS selector must be non-null
        if (raw_cs_raw & 0xfffc) == 0 {
            tracing::error!(
                "iret_protected: return CS selector null, ESP={:#x} icount={}",
                temp_esp,
                self.icount
            );
            return self.exception(Exception::Gp, 0);
        }

        // Parse CS selector and fetch/validate descriptor from GDT
        let mut cs_selector = BxSelector::default();
        parse_selector(raw_cs_raw, &mut cs_selector);

        let (dword1, dword2) = match self.fetch_raw_descriptor(&cs_selector) {
            Ok(v) => v,
            Err(_) => return self.exception(Exception::Gp, raw_cs_raw & 0xfffc),
        };
        let mut cs_descriptor = match self.parse_descriptor(dword1, dword2) {
            Ok(v) => v,
            Err(_) => return self.exception(Exception::Gp, raw_cs_raw & 0xfffc),
        };

        // Return CS selector RPL must be >= CPL
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cs_selector.rpl < cpl {
            tracing::error!(
                "iret_protected: return selector RPL ({}) < CPL ({})",
                cs_selector.rpl,
                cpl
            );
            return self.exception(Exception::Gp, raw_cs_raw & 0xfffc);
        }

        // Validate code-segment descriptor
        // check_cs calls exception() directly: Gp for type/DPL errors, Np for not-present
        self.check_cs(&cs_descriptor, raw_cs_raw, 0, cs_selector.rpl)?;

        // Compute EFLAGS changeMask based on OLD CPL (before loading new CS)
        // Based on Bochs iret.cc
        let iopl = self.eflags.iopl();
        let mut change_mask = EFlags::OSZAPC
            .union(EFlags::TF)
            .union(EFlags::DF)
            .union(EFlags::NT)
            .union(EFlags::RF)
            .union(EFlags::AC)
            .union(EFlags::ID);
        if cpl <= iopl {
            change_mask = change_mask.union(EFlags::IF_);
        }
        if cpl == 0 {
            change_mask = change_mask
                .union(EFlags::IOPL_MASK)
                .union(EFlags::VIF)
                .union(EFlags::VIP);
        }
        let change_mask = change_mask.bits();

        let new_cpl = cs_selector.rpl;
        if new_cpl == cpl {
            // ── Same privilege level ─────────────────────────────────────────
            tracing::trace!(
                "IRET32(PM): same-priv return to CS={:#06x} EIP={:#010x} EFLAGS={:#010x}",
                raw_cs_raw,
                new_eip,
                new_eflags
            );

            // Bochs iret.cc iret_protected \u2014 same-priv shadow-stack restore.
            if self.shadow_stack_enabled(cpl) {
                let return_lip =
                    (cs_descriptor.u.segment_base().wrapping_add(new_eip as u64)) & 0xFFFF_FFFF;
                let prev_ssp = self.shadow_stack_restore_lip(raw_cs_raw, return_lip)?;
                self.set_ssp(prev_ssp);
            }

            // Load CS from GDT descriptor (sets CS.base from descriptor, NOT << 4)
            self.branch_far(
                &mut cs_selector,
                &mut cs_descriptor,
                new_eip as u64,
                new_cpl,
            )?;

            // Restore EFLAGS with proper side effects (Bochs iret.cc)
            self.write_eflags(new_eflags, change_mask);

            // Advance ESP by 12 (EIP + CS-dword + EFLAGS = 3 × 4 bytes)
            if self.is_stack_32bit() {
                let esp = self.esp();
                self.set_esp(esp.wrapping_add(12));
            } else {
                let sp = self.sp();
                self.set_sp(sp.wrapping_add(12));
            }
        } else {
            // ── Privilege change (returning to outer/less-privileged ring) ────
            tracing::trace!(
                "IRET32(PM): privilege change to CS={:#06x} EIP={:#010x} EFLAGS={:#010x}",
                raw_cs_raw,
                new_eip,
                new_eflags
            );

            // Read new ESP and SS from stack at ESP+12 and ESP+16
            let new_esp = self.stack_read_dword(temp_esp + 12)?;
            let raw_ss_raw = self.stack_read_dword(temp_esp + 16)? as u16;

            if (raw_ss_raw & 0xfffc) == 0 {
                tracing::error!("iret_protected: SS selector null");
                return self.exception(Exception::Gp, 0);
            }

            let mut ss_selector = BxSelector::default();
            parse_selector(raw_ss_raw, &mut ss_selector);

            if ss_selector.rpl != cs_selector.rpl {
                tracing::error!("iret_protected: SS.rpl != CS.rpl");
                return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
            }

            let (ss_dw1, ss_dw2) = match self.fetch_raw_descriptor(&ss_selector) {
                Ok(v) => v,
                Err(_) => return self.exception(Exception::Gp, raw_ss_raw & 0xfffc),
            };
            let mut ss_descriptor = match self.parse_descriptor(ss_dw1, ss_dw2) {
                Ok(v) => v,
                Err(_) => return self.exception(Exception::Gp, raw_ss_raw & 0xfffc),
            };

            // SS must be a writable data segment
            if ss_descriptor.valid == 0
                || !ss_descriptor.segment
                || ss_descriptor.r#type >= 8       // code segment
                || (ss_descriptor.r#type & 2) == 0
            // not writable
            {
                tracing::error!("iret_protected: SS not writable data segment");
                return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
            }
            if ss_descriptor.dpl != cs_selector.rpl {
                tracing::error!("iret_protected: SS.dpl != CS.rpl");
                return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
            }
            if !ss_descriptor.p {
                tracing::error!("iret_protected: SS not present");
                return self.exception(Exception::Np, raw_ss_raw & 0xfffc);
            }

            // Bochs iret.cc iret_protected \u2014 outer-priv shadow-stack restore.
            // Capture prev CPL and pop the saved SSP token (when not returning to
            // ring 3) before branch_far reloads CS.
            let prev_cpl = cpl;
            let mut new_ssp_cet = self.msr.ia32_pl_ssp[3];
            if self.shadow_stack_enabled(cpl) {
                if self.ssp() & 0x7 != 0 {
                    tracing::error!("iret_protected: SSP not 8-byte aligned");
                    self.exception(Exception::Cp, super::cet::BX_CP_FAR_RET_IRET)?;
                }
                if cs_selector.rpl != 3 {
                    let return_lip =
                        (cs_descriptor.u.segment_base().wrapping_add(new_eip as u64)) & 0xFFFF_FFFF;
                    new_ssp_cet = self.shadow_stack_restore_lip(raw_cs_raw, return_lip)?;
                }
            }

            // Load CS (sets new CPL = new_cpl)
            self.branch_far(
                &mut cs_selector,
                &mut cs_descriptor,
                new_eip as u64,
                new_cpl,
            )?;

            // Restore EFLAGS with proper side effects (Bochs iret.cc)
            self.write_eflags(new_eflags, change_mask);

            // Load SS and restore ESP
            self.load_ss(&mut ss_selector, &mut ss_descriptor, new_cpl)?;
            if self.is_stack_32bit() {
                self.set_esp(new_esp);
            } else {
                self.set_sp(new_esp as u16);
            }

            // Bochs iret.cc iret_protected \u2014 install new SSP and clear busy on prev SSP.
            let old_ssp = self.ssp();
            let new_cpl_cet = self.cs_rpl();
            if self.shadow_stack_enabled(new_cpl_cet) {
                if (new_ssp_cet >> 32) != 0 {
                    tracing::error!("iret_protected: 64-bit SSP in legacy mode");
                    return self.exception(Exception::Gp, 0);
                }
                self.set_ssp(new_ssp_cet);
            }
            if self.shadow_stack_enabled(prev_cpl) {
                self.shadow_stack_atomic_clear_busy(old_ssp, prev_cpl)?;
            }

            // validate_seg_regs(): null out DS/ES/FS/GS if no longer accessible
            // (needed for ring-0→ring-3 transitions to prevent leaking kernel selectors)
            self.validate_seg_regs();
        }

        // RSP_COMMIT
        self.speculative_rsp = false;
        Ok(())
    }

    /// IRET in protected mode with 16-bit operand size.
    /// Based on Bochs iret.cc:iret_protected() with os32=false.
    /// Reads 16-bit IP/CS/FLAGS from stack instead of 32-bit values.
    fn iret_protected_16(&mut self) -> super::Result<()> {
        use super::cpu::Exception;

        // Nested Task (NT) — same as 32-bit path
        if self.eflags.contains(EFlags::NT) {
            tracing::trace!("IRET16(PM): nested task return (NT=1)");
            let tss_base = self.tr.cache.u.segment_base();
            let raw_link_selector = self.system_read_word(tss_base)?;
            let mut link_selector = BxSelector::default();
            parse_selector(raw_link_selector, &mut link_selector);
            if link_selector.ti != 0 {
                return self.exception(Exception::Ts, raw_link_selector & 0xfffc);
            }
            let (dword1, dword2) = match self.fetch_raw_descriptor(&link_selector) {
                Ok(v) => v,
                Err(_) => return self.exception(Exception::Ts, raw_link_selector & 0xfffc),
            };
            let tss_descriptor = match self.parse_descriptor(dword1, dword2) {
                Ok(v) => v,
                Err(_) => return self.exception(Exception::Ts, raw_link_selector & 0xfffc),
            };
            if tss_descriptor.valid == 0 || tss_descriptor.segment {
                return self.exception(Exception::Ts, raw_link_selector & 0xfffc);
            }
            if tss_descriptor.r#type != 0x3 && tss_descriptor.r#type != 0xB {
                return self.exception(Exception::Ts, raw_link_selector & 0xfffc);
            }
            if !tss_descriptor.p {
                return self.exception(Exception::Np, raw_link_selector & 0xfffc);
            }
            return self.task_switch(
                &link_selector,
                &tss_descriptor,
                super::tasking::BX_TASK_FROM_IRET,
                dword1,
                dword2,
                false,
                0,
            );
        }

        // RSP_SPECULATIVE
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp(); // Bochs: prev_rsp = RSP (full 64-bit)

        // Peek at stack — 16-bit reads (6 bytes total)
        let temp_esp = if self.is_stack_32bit() {
            self.esp()
        } else {
            self.sp() as u32
        };

        let new_ip = self.stack_read_word(temp_esp)? as u32;
        let raw_cs_raw = self.stack_read_word(temp_esp + 2)?;
        let new_flags = self.stack_read_word(temp_esp + 4)? as u32;

        // Return CS selector must be non-null
        if (raw_cs_raw & 0xfffc) == 0 {
            self.speculative_rsp = false;
            return self.exception(Exception::Gp, 0);
        }

        let mut cs_selector = BxSelector::default();
        parse_selector(raw_cs_raw, &mut cs_selector);

        let (dword1, dword2) = match self.fetch_raw_descriptor(&cs_selector) {
            Ok(v) => v,
            Err(_) => {
                self.speculative_rsp = false;
                return self.exception(Exception::Gp, raw_cs_raw & 0xfffc);
            }
        };
        let mut cs_descriptor = match self.parse_descriptor(dword1, dword2) {
            Ok(v) => v,
            Err(_) => {
                self.speculative_rsp = false;
                return self.exception(Exception::Gp, raw_cs_raw & 0xfffc);
            }
        };

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cs_selector.rpl < cpl {
            self.speculative_rsp = false;
            return self.exception(Exception::Gp, raw_cs_raw & 0xfffc);
        }

        // check_cs calls exception() directly: Gp for type/DPL errors, Np for not-present
        self.check_cs(&cs_descriptor, raw_cs_raw, 0, cs_selector.rpl)?;

        let new_cpl = cs_selector.rpl;
        let iopl = self.eflags.iopl();

        if new_cpl == cpl {
            // Same privilege — 16-bit
            // Bochs iret.cc iret_protected (16-bit) \u2014 same-priv shadow-stack restore.
            if self.shadow_stack_enabled(cpl) {
                let return_lip =
                    (cs_descriptor.u.segment_base().wrapping_add(new_ip as u64)) & 0xFFFF_FFFF;
                let prev_ssp = self.shadow_stack_restore_lip(raw_cs_raw, return_lip)?;
                self.set_ssp(prev_ssp);
            }

            self.branch_far(&mut cs_selector, &mut cs_descriptor, new_ip as u64, new_cpl)?;

            // write_flags for 16-bit (Bochs iret.cc)
            self.write_flags(new_flags as u16, cpl == 0, cpl <= iopl);

            // Advance ESP by 6 (IP + CS + FLAGS = 3 × 2 bytes)
            if self.is_stack_32bit() {
                let esp = self.esp();
                self.set_esp(esp.wrapping_add(6));
            } else {
                let sp = self.sp();
                self.set_sp(sp.wrapping_add(6));
            }
        } else {
            // Outer privilege — 16-bit
            let new_sp = self.stack_read_word(temp_esp + 6)? as u32;
            let raw_ss_raw = self.stack_read_word(temp_esp + 8)?;

            if (raw_ss_raw & 0xfffc) == 0 {
                self.speculative_rsp = false;
                return self.exception(Exception::Gp, 0);
            }

            let mut ss_selector = BxSelector::default();
            parse_selector(raw_ss_raw, &mut ss_selector);

            if ss_selector.rpl != cs_selector.rpl {
                self.speculative_rsp = false;
                return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
            }

            let (ss_dw1, ss_dw2) = match self.fetch_raw_descriptor(&ss_selector) {
                Ok(v) => v,
                Err(_) => {
                    self.speculative_rsp = false;
                    return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
                }
            };
            let mut ss_descriptor = match self.parse_descriptor(ss_dw1, ss_dw2) {
                Ok(v) => v,
                Err(_) => {
                    self.speculative_rsp = false;
                    return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
                }
            };

            if ss_descriptor.valid == 0
                || !ss_descriptor.segment
                || ss_descriptor.r#type >= 8
                || (ss_descriptor.r#type & 2) == 0
            {
                self.speculative_rsp = false;
                return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
            }
            if ss_descriptor.dpl != cs_selector.rpl {
                self.speculative_rsp = false;
                return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
            }
            if !ss_descriptor.p {
                self.speculative_rsp = false;
                return self.exception(Exception::Np, raw_ss_raw & 0xfffc);
            }

            // Bochs iret.cc iret_protected (16-bit) \u2014 outer-priv shadow-stack restore.
            let prev_cpl = cpl;
            let mut new_ssp_cet = self.msr.ia32_pl_ssp[3];
            if self.shadow_stack_enabled(cpl) {
                if self.ssp() & 0x7 != 0 {
                    tracing::error!("iret_protected_16: SSP not 8-byte aligned");
                    self.exception(Exception::Cp, super::cet::BX_CP_FAR_RET_IRET)?;
                }
                if cs_selector.rpl != 3 {
                    let return_lip =
                        (cs_descriptor.u.segment_base().wrapping_add(new_ip as u64)) & 0xFFFF_FFFF;
                    new_ssp_cet = self.shadow_stack_restore_lip(raw_cs_raw, return_lip)?;
                }
            }

            self.branch_far(&mut cs_selector, &mut cs_descriptor, new_ip as u64, new_cpl)?;

            // write_flags for 16-bit (Bochs iret.cc)
            self.write_flags(new_flags as u16, cpl == 0, cpl <= iopl);

            self.load_ss(&mut ss_selector, &mut ss_descriptor, new_cpl)?;
            self.set_sp(new_sp as u16);

            // Bochs iret.cc iret_protected (16-bit) \u2014 install new SSP and clear busy on prev SSP.
            let old_ssp = self.ssp();
            let new_cpl_cet = self.cs_rpl();
            if self.shadow_stack_enabled(new_cpl_cet) {
                if (new_ssp_cet >> 32) != 0 {
                    tracing::error!("iret_protected_16: 64-bit SSP in legacy mode");
                    return self.exception(Exception::Gp, 0);
                }
                self.set_ssp(new_ssp_cet);
            }
            if self.shadow_stack_enabled(prev_cpl) {
                self.shadow_stack_atomic_clear_busy(old_ssp, prev_cpl)?;
            }

            self.validate_seg_regs();
        }

        // RSP_COMMIT
        self.speculative_rsp = false;
        Ok(())
    }

    // =========================================================================
    // Real Mode Interrupt Handler
    // =========================================================================

    /// Handle interrupt in real mode using IVT
    /// Based on Bochs exception.cc real_mode_int()
    pub(super) fn interrupt_real_mode(&mut self, vector: u8) -> super::Result<()> {
        // Bochs exception.cc: IDTR limit check
        if (vector as u32 * 4 + 3) > self.idtr.limit as u32 {
            tracing::error!("interrupt(real mode) vector > idtr.limit");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Save current FLAGS, CS, IP on stack
        // Bochs exception.cc: push FLAGS, CS, IP
        let flags = (self.read_eflags() & 0xFFFF) as u16;
        let cs = self.sregs[BxSegregs::Cs as usize].selector.value;
        let ip = self.get_ip();

        self.push_16(flags)?;
        self.push_16(cs)?;
        self.push_16(ip)?;

        // Bochs exception.cc: read new IP from IVT using system_read_word (paging-aware)
        let ivt_addr = self.idtr.base + (vector as u64) * 4;
        let new_ip = self.system_read_word(ivt_addr)?;

        // Bochs exception.cc: CS limit check on loaded IP
        let cs_limit = self.get_segment_limit(BxSegregs::Cs);
        if (new_ip as u32) > cs_limit {
            tracing::error!(
                "interrupt(real mode): instruction pointer not within code segment limits"
            );
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Bochs exception.cc: read new CS from IVT
        let new_cs = self.system_read_word(ivt_addr + 2)?;

        // Boot diagnostic: if we ever vector to 0000:0000 in real mode, BIOS likely
        // hit an unexpected exception/IRQ before IVT was initialized (or IVT reads are broken).
        if new_ip == 0 && new_cs == 0 && (self.boot_debug_flags & 0x02) == 0 {
            self.boot_debug_flags |= 0x02;
            self.debug_puts(b"[IVT->0000:0000]\n");
        }

        // Bochs exception.cc: load CS:IP from IVT
        let cs_index = BxSegregs::Cs as usize;
        parse_selector(new_cs, &mut self.sregs[cs_index].selector);
        self.sregs[cs_index]
            .cache
            .u
            .set_segment_base((new_cs as u64) << 4);
        self.set_ip(new_ip);

        // Bochs exception.cc — clear IF, TF, AC, RF
        self.eflags.remove(EFlags::IF_ | EFlags::TF | EFlags::AC);
        self.clear_rf();
        self.handle_interrupt_mask_change();

        // Invalidate prefetch
        self.eip_fetch_window = None;
        self.eip_page_window_size = 0;

        // Only log non-exception interrupts to reduce spam (exceptions are logged in exception.rs)
        if vector != 0x0d && vector != 0x0e && vector != 0x08 && vector < 0x20 {
            tracing::trace!(
                "INT {:#04x}: vector at {:04x}:{:04x}",
                vector,
                new_cs,
                new_ip
            );
        }
        // Log INT 15h calls (memory detection) — AH=88h returns extended memory in AX
        if vector == 0x15 {
            tracing::trace!(
                "INT 15h: AH={:#04x} AX={:#06x} → handler at {:04x}:{:04x}, caller was {:04x}:{:04x}",
                self.ah(), self.ax(), new_cs, new_ip, cs, ip
            );
        }
        Ok(())
    }

    // =========================================================================
    // HLT - Halt instruction
    // =========================================================================

    /// HLT - Halt CPU until interrupt
    /// Based on Bochs proc_ctrl.cc
    pub fn hlt(&mut self, _instr: &Instruction) -> super::Result<()> {
        // CPL is always 0 in real mode
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            tracing::trace!("HLT: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        // Bochs svm.cc SVM_INTERCEPT0_HLT.
        if self.in_svm_guest && self.svm_intercept_check(super::svm::SVM_INTERCEPT0_HLT) {
            return self.svm_vmexit(super::svm::SvmVmexit::Hlt as i32, 0, 0);
        }
        // Bochs vmx.cc VMexit_HLT.
        if self.in_vmx_guest && self.vmexit_check_hlt()? {
            return Ok(());
        }

        // Check if interrupts are disabled (IF=0) - matches Bochs proc_ctrl.cc
        if !self.eflags.contains(EFlags::IF_) {
            tracing::trace!("HLT: CPU halted with IF=0 (interrupts disabled) - CPU will be stuck!");
        }

        #[cfg(debug_assertions)]
        {
            if !self.diag_first_pm_hlt_captured && self.protected_mode() {
                self.diag_first_pm_hlt_captured = true;
                self.diag_first_pm_hlt_icount = self.icount;
                self.diag_first_pm_hlt_rip = self.eip();
                self.diag_first_pm_hlt_regs = [
                    self.eax(),
                    self.ecx(),
                    self.edx(),
                    self.ebx(),
                    self.esp(),
                    self.ebp(),
                    self.esi(),
                    self.edi(),
                ];
                self.diag_first_pm_hlt_cs = self.sregs[BxSegregs::Cs as usize].selector.value;
                self.diag_first_pm_hlt_ss = self.sregs[BxSegregs::Ss as usize].selector.value;
                self.diag_first_pm_hlt_eflags = self.read_eflags();
                // Read 16 dwords from stack
                let esp = self.esp();
                for i in 0..16u32 {
                    self.diag_first_pm_hlt_stack[i as usize] = self
                        .stack_read_dword(esp.wrapping_add(i * 4))
                        .unwrap_or(0xDEADDEAD);
                }
            }
        }

        // Bochs proc_ctrl.cc HLT: enter_sleep_state(BX_ACTIVITY_STATE_HLT).
        self.enter_sleep_state(CpuActivityState::Hlt);
        Ok(())
    }

    const XSAVE_COMPONENTS: [(u32, u32); 10] = [
        (160, 0),     // 0: FPU (x87)
        (256, 160),   // 1: SSE (XMM)
        (256, 576),   // 2: YMM (AVX)
        (0, 0),       // 3: BNDREGS (deprecated MPX)
        (0, 0),       // 4: BNDCFG (deprecated MPX)
        (64, 1088),   // 5: OPMASK (AVX-512)
        (512, 1152),  // 6: ZMM_HI256 (AVX-512)
        (1024, 1664), // 7: HI_ZMM (AVX-512)
        (0, 0),       // 8: PT (Processor Trace, not implemented)
        (8, 2688),    // 9: PKRU
    ];

    /// Compute max XSAVE area size for given feature bitmap (standard layout).
    /// Bochs cpuid.cc xsave_max_size_required_by_features()
    fn xsave_max_size_for_features(&self, features: u32) -> u32 {
        // Legacy area (x87 + SSE header) is always 576 bytes minimum
        let mut max_size: u32 = 576;
        for n in 2..Self::XSAVE_COMPONENTS.len() {
            if features & (1 << n) != 0 {
                let (len, offset) = Self::XSAVE_COMPONENTS[n];
                if len > 0 {
                    let end = offset + len;
                    if end > max_size {
                        max_size = end;
                    }
                }
            }
        }
        max_size
    }

    /// Compute XSAVE area size for compacted (XSAVEC/XSAVES) layout.
    /// Bochs cpuid.cc xsave_max_size_required_by_xsaves_features()
    fn xsave_compacted_size_for_features(&self, features: u32) -> u32 {
        // Legacy area + XSAVE header = 576
        let mut max_size: u32 = 576;
        for n in 2..Self::XSAVE_COMPONENTS.len() {
            if features & (1 << n) != 0 {
                let (len, _) = Self::XSAVE_COMPONENTS[n];
                max_size += len;
            }
        }
        max_size
    }

    /// CPUID - CPU Identification
    /// Original: bochs/cpu/proc_ctrl.cc
    /// Returns CPU identification and feature information in EAX, EBX, ECX, EDX
    /// Input: EAX = function number, ECX = sub-function (for some functions)
    pub fn cpuid(&mut self, _instr: &Instruction) -> super::Result<()> {
        // BOCHS BX_INSTR_CPUID(cpu_id)
        if self.instrumentation.active.has_cpuid_msr() {
            self.instrumentation.fire_cpuid();
        }

        // Bochs svm.cc SVM_INTERCEPT0_CPUID — delivered before reading any
        // registers so the guest sees the original RAX/RCX on re-entry. The
        // VMEXIT unwinds via Err(CpuLoopRestart), which MUST propagate so the
        // CPU loop restarts decode at the host RIP.
        if self.in_svm_guest && self.svm_intercept_check(super::svm::SVM_INTERCEPT0_CPUID) {
            self.svm_vmexit(super::svm::SvmVmexit::Cpuid as i32, 0, 0)?;
            return Ok(());
        }
        // Bochs vmx.cc VMexit_CPUID — unconditional when in VMX guest.
        if self.in_vmx_guest {
            self.vmx_vmexit(super::vmx::VmxVmexitReason::Cpuid, 0)?;
            return Ok(());
        }

        let function = self.eax();
        let sub_function = self.ecx();

        let (mut eax, mut ebx, mut ecx, mut edx) =
            self.cpuid.get_cpuid_leaf(function, sub_function);

        // Dynamic fixups — Bochs computes these from CPU state at runtime.
        // Our static trait returns base values; we patch them here.
        match function {
            CPUID_LEAF_FEATURE_INFO => {
                // ECX bit 27 (OSXSAVE): set only when CR4.OSXSAVE is enabled
                // Bochs cpuid.cc — base value does NOT include this bit
                if self.cr4.osxsave() {
                    ecx |= CPUID_OSXSAVE_ECX_BIT;
                }

                // EDX bit 9 (APIC): cleared when APIC is globally disabled
                // Bochs cpuid.cc
                if self.lapic.get_mode() == super::apic::ApicMode::GloballyDisabled {
                    edx &= !CPUID_APIC_EDX_BIT;
                }

                // ECX bit 3 (MONITOR), withdrawn with the instructions it
                // promises. A guest believes this bit for the rest of its life:
                // Linux picks `mwait_idle` at boot and idles there forever, so
                // a `MONITOR` that is advertised and then faults does not
                // produce a retry, it produces an invalid opcode in the idle
                // task and a kernel that panics with "Attempted to kill the
                // idle task".
                if !self.bx_cpuid_support_isa_extension(
                    super::decoder::features::X86Feature::IsaMonitorMwait,
                ) {
                    ecx &= !CPUID_MONITOR_ECX_BIT;
                }

                let topology = self.cpu_topology();
                ebx = (ebx & CPUID_LEAF1_EBX_LOW_FIELDS_MASK)
                    | ((topology.package_logical_count() & CPUID_APIC_ID_BYTE_MASK)
                        << CPUID_LEAF1_LOGICAL_COUNT_SHIFT)
                    | ((self.bx_cpuid & CPUID_APIC_ID_BYTE_MASK) << CPUID_LEAF1_APIC_ID_SHIFT);
            }
            CPUID_LEAF_EXTENDED_TOPOLOGY => {
                let topology = self.cpu_topology();
                // Bochs cpuid.cc `get_std_cpuid_extended_topology_leaf` seeds
                // every subfunction — valid level or not — with the x2APIC id
                // in EDX and the subfunction echoed in ECX, then fills in the
                // level only when one exists. Software enumerates the leaf by
                // walking ECX until EBX reads zero, so an invalid level still
                // has to identify its logical processor.
                eax = 0;
                ebx = 0;
                ecx = sub_function;
                edx = self.bx_cpuid;
                match sub_function {
                    CPUID_TOPOLOGY_SUBLEAF_SMT => {
                        eax = bochs_topology_shift(topology.n_threads());
                        ebx = topology.n_threads();
                        ecx = topology_level_ecx(
                            CPUID_TOPOLOGY_SUBLEAF_SMT,
                            CPUID_TOPOLOGY_LEVEL_TYPE_SMT,
                        );
                    }
                    CPUID_TOPOLOGY_SUBLEAF_CORE => {
                        eax = bochs_topology_shift(topology.package_logical_count());
                        ebx = topology.package_logical_count();
                        ecx = topology_level_ecx(
                            CPUID_TOPOLOGY_SUBLEAF_CORE,
                            CPUID_TOPOLOGY_LEVEL_TYPE_CORE,
                        );
                    }
                    CPUID_TOPOLOGY_SUBLEAF_PACKAGE => {
                        // Bochs reports the package level only on a
                        // multi-socket topology; one socket leaves the level
                        // invalid, which is how software stops enumerating.
                        if topology.n_processors() > 1 {
                            eax = bochs_topology_shift(topology.n_processors());
                            ebx = topology.cpu_count();
                            ecx = topology_level_ecx(
                                CPUID_TOPOLOGY_SUBLEAF_PACKAGE,
                                CPUID_TOPOLOGY_LEVEL_TYPE_PACKAGE,
                            );
                        }
                    }
                    _ => {}
                }
            }
            0x00000007 if sub_function == 0 => {
                // The AVX-512 promises, withdrawn together with the register
                // file behind them. Subleaf 0 of leaf `D` above answers from
                // `xcr0_suppmask`, which is derived from the ISA bitmask, so
                // a feature bit left standing here would tell a guest it has
                // instructions whose state `XSETBV` then refuses. A guest
                // believes the feature bit: it enables the component, takes a
                // fault on the first use, and under a hypervisor hands back a
                // processor state the platform will not accept at all.
                if !self
                    .bx_cpuid_support_isa_extension(super::decoder::features::X86Feature::IsaAvx512)
                {
                    ebx &= !CPUID_LEAF7_EBX_AVX512;
                    ecx &= !CPUID_LEAF7_ECX_AVX512;
                    edx &= !CPUID_LEAF7_EDX_AVX512;
                }
            }

            0x0000000D => {
                if sub_function == 0 {
                    // Subleaf 0: EAX = xcr0_suppmask, ECX = max size for all features
                    // EBX = max size for currently enabled features (current xcr0)
                    // Bochs cpuid.cc
                    eax = self.xcr0_suppmask;
                    ebx = self.xsave_max_size_for_features(self.xcr0.get32());
                    ecx = self.xsave_max_size_for_features(self.xcr0_suppmask);
                } else if sub_function == 1 {
                    // Subleaf 1 EAX: XSAVE feature flags (Bochs cpuid.cc)
                    // Bit 0: XSAVEOPT, Bit 1: XSAVEC, Bit 2: XGETBV_ECX1, Bit 3: XSAVES
                    eax = 0x0000000F; // Skylake-X supports all four
                    // Subleaf 1 EBX: size for XSAVES (XCR0 | IA32_XSS)
                    // Bochs cpuid.cc
                    ebx = self.xsave_compacted_size_for_features(
                        self.xcr0.get32() | (self.msr.ia32_xss as u32),
                    );
                    ecx = self.ia32_xss_suppmask;
                } else if sub_function >= 2 && sub_function < Self::XSAVE_COMPONENTS.len() as u32 {
                    // Per-component sub-leaves (Bochs cpuid.cc):
                    // EAX = size, EBX = offset, ECX = flags, EDX = 0
                    let support_mask = self.xcr0_suppmask | self.ia32_xss_suppmask;
                    if support_mask & (1 << sub_function) == 0 {
                        eax = 0;
                        ebx = 0;
                        ecx = 0;
                        edx = 0;
                    } else {
                        let (size, offset) = Self::XSAVE_COMPONENTS[sub_function as usize];
                        eax = size;
                        ebx = offset;
                        // ECX bit 0: managed via IA32_XSS (not XCR0)
                        // Bochs cpuid.cc: ecx = (ia32_xss_suppmask & (1 << subfunction)) != 0
                        ecx = u32::from(self.ia32_xss_suppmask & (1 << sub_function) != 0);
                        edx = 0;
                    }
                } else if sub_function >= Self::XSAVE_COMPONENTS.len() as u32 && sub_function < 19 {
                    eax = 0;
                    ebx = 0;
                    ecx = 0;
                    edx = 0;
                }
            }
            0x80000001
                // EDX bit 11 (SYSCALL/SYSRET): only in long mode
                // Bochs cpuid.cc
                if self.long64_mode() => {
                    edx |= 1 << 11; // BX_CPUID_EXT1_EDX_SYSCALL_SYSRET
                }
            _ => {}
        }

        // Bochs proc_ctrl.cc: RAX = leaf.eax (writes 64-bit, zero-extending)
        self.set_rax(eax as u64);
        self.set_rbx(ebx as u64);
        self.set_rcx(ecx as u64);
        self.set_rdx(edx as u64);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::BxParams;

    const CONFIGURED_APIC_ID: u32 = 5;
    const LEAF1_TEST_APIC_ID: u32 = 7;

    #[test]
    fn cpuid_leaf_b_uses_configured_topology_and_apic_id() {
        let topology = BxParams::default()
            .with_topology(2, 4, 2)
            .unwrap()
            .cpu_topology();
        let mut machine = crate::cpu::exec_ctx::TestMachine::new();
        let mut cpu = machine.ctx();
        cpu.configure_smp(CONFIGURED_APIC_ID, topology);
        let instr = Instruction::default();

        cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
        cpu.set_ecx(CPUID_TOPOLOGY_SUBLEAF_SMT);
        cpu.cpuid(&instr).unwrap();
        assert_eq!(cpu.eax(), 1);
        assert_eq!(cpu.ebx(), 2);
        assert_eq!(
            cpu.ecx(),
            topology_level_ecx(CPUID_TOPOLOGY_SUBLEAF_SMT, CPUID_TOPOLOGY_LEVEL_TYPE_SMT)
        );
        assert_eq!(cpu.edx(), CONFIGURED_APIC_ID);

        cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
        cpu.set_ecx(CPUID_TOPOLOGY_SUBLEAF_CORE);
        cpu.cpuid(&instr).unwrap();
        // 4 cores x 2 threads = 8 logical processors below the package, so the
        // shift to the next level is 3 bits. A literal, not a call to the code
        // under test — the assertion has to be able to disagree with it.
        assert_eq!(cpu.eax(), 3);
        assert_eq!(cpu.ebx(), 8);
        assert_eq!(
            cpu.ecx(),
            topology_level_ecx(CPUID_TOPOLOGY_SUBLEAF_CORE, CPUID_TOPOLOGY_LEVEL_TYPE_CORE)
        );
        assert_eq!(cpu.edx(), CONFIGURED_APIC_ID);

        // Two sockets, so the package level exists: shifting the x2APIC id by
        // 1 leaves the socket id, and 2 x 4 x 2 = 16 logical processors sit
        // below it. Literals, for the reason given above.
        cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
        cpu.set_ecx(CPUID_TOPOLOGY_SUBLEAF_PACKAGE);
        cpu.cpuid(&instr).unwrap();
        assert_eq!(cpu.eax(), 1);
        assert_eq!(cpu.ebx(), 16);
        assert_eq!(
            cpu.ecx(),
            topology_level_ecx(
                CPUID_TOPOLOGY_SUBLEAF_PACKAGE,
                CPUID_TOPOLOGY_LEVEL_TYPE_PACKAGE
            )
        );
        assert_eq!(cpu.edx(), CONFIGURED_APIC_ID);

        // An invalid level still echoes the subfunction and the x2APIC id, and
        // reports EBX = 0 — the terminator software enumerates on.
        const INVALID_SUBLEAF: u32 = 3;
        cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
        cpu.set_ecx(INVALID_SUBLEAF);
        cpu.cpuid(&instr).unwrap();
        assert_eq!(cpu.eax(), 0);
        assert_eq!(cpu.ebx(), 0);
        assert_eq!(cpu.ecx(), INVALID_SUBLEAF);
        assert_eq!(cpu.edx(), CONFIGURED_APIC_ID);
    }

    /// Leaf 0xB's core-level EAX, checked the way software actually consumes
    /// it rather than against either implementation's arithmetic.
    ///
    /// SDM Vol 2, CPUID leaf 0BH: EAX[4:0] at sub-leaf *m* is the shift that
    /// extracts the id of the *next higher* level — so at the core level it
    /// must cover the SMT bits as well as the core bits. Linux
    /// `detect_extended_topology` names that value `core_plus_mask_width`
    /// ("core PLUS") and derives `phys_proc_id = initial_apicid >> it`.
    ///
    /// Bochs cpuid.cc `get_std_cpuid_extended_topology_leaf` reports
    /// `ilog2(ncores-1)+1`, which counts the core bits alone. APIC ids are
    /// assigned densely from the CPU index (Bochs apic.cc
    /// `bx_local_apic_c::bx_local_apic_c`), so on 2 x 4 x 2 they pack thread
    /// into bit 0, core into bits 1-2 and socket into bit 3 — and shifting by
    /// the core width alone strands the top core bit inside the package id,
    /// splitting each socket in two. This test therefore fails against the
    /// upstream formula; see docs/bochs-parity-divergences.md.
    #[test]
    fn cpuid_leaf_b_core_shift_separates_sockets_as_software_reads_it() {
        const PACKAGES: u32 = 2;
        const CORES: u32 = 4;
        const THREADS: u32 = 2;
        const LOGICAL: u32 = PACKAGES * CORES * THREADS;
        let topology = BxParams::default()
            .with_topology(PACKAGES, CORES, THREADS)
            .unwrap()
            .cpu_topology();
        let mut machine = crate::cpu::exec_ctx::TestMachine::new();
        let instr = Instruction::default();

        let mut package_of = Vec::new();
        for apic_id in 0..LOGICAL {
            let mut cpu = machine.ctx();
            cpu.configure_smp(apic_id, topology);
            cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
            cpu.set_ecx(CPUID_TOPOLOGY_SUBLEAF_CORE);
            cpu.cpuid(&instr).unwrap();
            let core_plus_mask_width = cpu.eax() & 0x1F;
            package_of.push(apic_id >> core_plus_mask_width);
        }

        let expected: Vec<u32> = (0..LOGICAL).map(|id| id / (CORES * THREADS)).collect();
        assert_eq!(
            package_of, expected,
            "all 8 logical processors of a socket must derive the same package id"
        );
    }

    /// One socket leaves the package level invalid, exactly as Bochs
    /// cpuid.cc guards `case 2` with `nprocessors > 1`.
    #[test]
    fn cpuid_leaf_b_package_level_is_invalid_on_a_single_socket() {
        let topology = BxParams::default()
            .with_topology(1, 4, 2)
            .unwrap()
            .cpu_topology();
        let mut machine = crate::cpu::exec_ctx::TestMachine::new();
        let mut cpu = machine.ctx();
        cpu.configure_smp(CONFIGURED_APIC_ID, topology);
        let instr = Instruction::default();

        cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
        cpu.set_ecx(CPUID_TOPOLOGY_SUBLEAF_PACKAGE);
        cpu.cpuid(&instr).unwrap();
        assert_eq!(cpu.eax(), 0);
        assert_eq!(cpu.ebx(), 0, "no package level to enumerate");
        assert_eq!(cpu.ecx(), CPUID_TOPOLOGY_SUBLEAF_PACKAGE);
        assert_eq!(cpu.edx(), CONFIGURED_APIC_ID);
    }

    /// Bochs cpuid.cc computes leaf 0xB EAX as `ilog2(n-1)+1` with no guard for
    /// `n == 1`, and its `ilog2(0)` is 0 — so a level holding one logical
    /// processor reports a shift of 1, NOT 0.
    ///
    /// The values below are literals for that reason. Asserting against
    /// `bochs_topology_shift` would make the test agree with the code whatever
    /// the code said, which is how this case stayed wrong.
    #[test]
    fn cpuid_leaf_b_shift_is_one_on_a_uniprocessor_topology() {
        let topology = BxParams::default()
            .with_topology(1, 1, 1)
            .unwrap()
            .cpu_topology();
        let mut machine = crate::cpu::exec_ctx::TestMachine::new();
        let mut cpu = machine.ctx();
        cpu.configure_smp(0, topology);
        let instr = Instruction::default();

        cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
        cpu.set_ecx(CPUID_TOPOLOGY_SUBLEAF_SMT);
        cpu.cpuid(&instr).unwrap();
        assert_eq!(cpu.eax(), 1, "one thread per core still shifts by 1");
        assert_eq!(cpu.ebx(), 1);

        cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
        cpu.set_ecx(CPUID_TOPOLOGY_SUBLEAF_CORE);
        cpu.cpuid(&instr).unwrap();
        assert_eq!(cpu.eax(), 1, "one logical processor per package likewise");
        assert_eq!(cpu.ebx(), 1);
    }

    /// The shift is a pure function of the level's width; pin the whole small
    /// range against Bochs `ilog2(n-1)+1` rather than against ourselves.
    #[test]
    fn topology_shift_matches_bochs_ilog2_form() {
        for (logical_count, expected) in
            [(1u32, 1u32), (2, 1), (3, 2), (4, 2), (5, 3), (8, 3), (9, 4)]
        {
            assert_eq!(
                bochs_topology_shift(logical_count),
                expected,
                "leaf 0xB shift for {logical_count} logical processors"
            );
        }
    }

    #[test]
    fn cpuid_leaf_1_reports_package_logical_count_and_apic_id() {
        let topology = BxParams::default()
            .with_topology(2, 2, 2)
            .unwrap()
            .cpu_topology();
        let mut machine = crate::cpu::exec_ctx::TestMachine::new();
        let mut cpu = machine.ctx();
        cpu.initialize(BxParams::default()).unwrap();
        cpu.configure_smp(LEAF1_TEST_APIC_ID, topology);
        cpu.reset(crate::cpu::ResetReason::Hardware);
        let instr = Instruction::default();

        cpu.set_eax(CPUID_LEAF_FEATURE_INFO);
        cpu.set_ecx(0);
        cpu.cpuid(&instr).unwrap();

        assert_eq!(
            (cpu.ebx() >> CPUID_LEAF1_LOGICAL_COUNT_SHIFT) & CPUID_APIC_ID_BYTE_MASK,
            topology.package_logical_count()
        );
        assert_eq!(
            (cpu.ebx() >> CPUID_LEAF1_APIC_ID_SHIFT) & CPUID_APIC_ID_BYTE_MASK,
            LEAF1_TEST_APIC_ID
        );
        assert_eq!(cpu.edx() & CPUID_APIC_EDX_BIT, CPUID_APIC_EDX_BIT);
    }
}
