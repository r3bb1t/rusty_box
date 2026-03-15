//! Software interrupt instructions for x86 CPU emulation
//!
//! Based on Bochs soft_int.cc
//! Copyright (C) 2001-2018 The Bochs Project
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

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Unified interrupt dispatch — matches Bochs interrupt() in exception.cc
    // =========================================================================

    /// Unified interrupt dispatch based on CPU mode.
    ///
    /// Mirrors Bochs `BX_CPU_C::interrupt()` in exception.cc:762-839.
    /// Dispatches to real_mode_int or protected_mode_int based on current CPU mode.
    /// After delivery, invalidates prefetch and returns CpuLoopRestart to
    /// restart the trace (matching Bochs BX_NEXT_TRACE).
    pub(super) fn interrupt(
        &mut self,
        vector: u8,
        soft_int: bool,
        push_error: bool,
        error_code: u16,
    ) -> super::Result<()> {
        tracing::debug!(
            "interrupt(): vector={:#04x} soft_int={} mode={}",
            vector,
            soft_int,
            if self.real_mode() {
                "real"
            } else {
                "protected"
            }
        );

        // Discard any traps and inhibits for new context (matches Bochs line 800-801)
        self.debug_trap = 0;
        self.inhibit_mask = 0;

        // Invalidate prefetch queue (matches Bochs line 777)
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        // RSP_SPECULATIVE — mark speculative RSP so exceptions during delivery
        // can restore the original value (matches Bochs line 807)
        self.speculative_rsp = true;
        self.prev_rsp = self.esp() as u64;
        self.prev_ssp = 0; // no shadow stack

        if self.real_mode() {
            self.interrupt_real_mode(vector)?;
        } else {
            // V8086 mode software interrupt: try VME redirect first
            // Bochs exception.cc: v86_redirect_interrupt checked before protected_mode_int
            if self.v8086_mode() && soft_int {
                if self.v86_redirect_interrupt(vector)? {
                    // Interrupt was redirected through virtual IVT
                    self.speculative_rsp = false;
                    self.ext = false;
                    self.async_event |= BX_ASYNC_EVENT_STOP_TRACE;
                    return Err(super::error::CpuError::CpuLoopRestart);
                }
            }

            // Long mode: dispatch through 16-byte IDT entries
            // Protected mode (or V86 non-redirected): dispatch through 8-byte IDT entries
            let delivery_result = if self.long_mode() {
                self.long_mode_int(vector, soft_int, push_error, error_code)
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
                    tracing::warn!(
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
    /// Based on Bochs INT_Ib in soft_int.cc:127-161
    pub fn int_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let vector = instr.ib();
        tracing::debug!("INT {:#04x}", vector);
        // BX_SOFTWARE_INTERRUPT → soft_int=true, no error code
        self.interrupt(vector, true, false, 0)
    }

    /// INT3 - Breakpoint interrupt (vector 3)
    /// Based on Bochs INT3 in soft_int.cc:98-124
    pub fn int3(&mut self, _instr: &Instruction) -> super::Result<()> {
        tracing::debug!("INT3 (breakpoint)");
        // BX_SOFTWARE_EXCEPTION → soft_int=true, no error code
        self.interrupt(3, true, false, 0)
    }

    /// INTO - Interrupt on overflow (vector 4, only if OF=1)
    /// Based on Bochs INTO in soft_int.cc:163-189
    pub fn into(&mut self, _instr: &Instruction) -> super::Result<()> {
        if self.get_of() {
            tracing::debug!("INTO: overflow detected, calling INT 4");
            // BX_SOFTWARE_EXCEPTION → soft_int=true, no error code
            return self.interrupt(4, true, false, 0);
        }
        Ok(())
    }

    /// INT1 (ICEBP) - In-circuit emulator breakpoint (vector 1)
    /// Based on Bochs INT1 in soft_int.cc:68-96
    pub fn int1(&mut self, _instr: &Instruction) -> super::Result<()> {
        tracing::warn!(
            "INT1 (ICEBP) at RIP={:#x} CS={:#x}",
            self.rip(),
            self.sregs[crate::cpu::decoder::BxSegregs::Cs as usize]
                .selector
                .value
        );
        // BX_PRIVILEGED_SOFTWARE_INTERRUPT → soft_int=false (privileged bypass DPL check)
        // Bochs sets EXT=1 before calling interrupt() for INT1
        self.ext = true;
        self.interrupt(1, false, false, 0)
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
        let asize_mask: u64 = if self.long64_mode() {
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
            tracing::debug!(
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
        let asize_mask: u64 = if self.long64_mode() {
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
            tracing::debug!(
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
    /// Based on Bochs ctrl_xfer16.cc IRET16 (lines 520-590)
    pub fn iret16(&mut self, _instr: &Instruction) -> super::Result<()> {
        // Invalidate prefetch queue at entry (Bochs ctrl_xfer16.cc:524)
        self.invalidate_prefetch_q();

        // Unmask NMI on every IRET (Bochs ctrl_xfer16.cc:542)
        self.unmask_event(Self::BX_EVENT_NMI);

        // RSP_SPECULATIVE before all mode branches (Bochs ctrl_xfer16.cc:552)
        self.speculative_rsp = true;
        self.prev_rsp = self.esp() as u64;

        // Protected mode dispatch (Bochs ctrl_xfer16.cc:554)
        // Bochs checks protected_mode() first, which includes protected+long modes
        // but NOT V8086. V8086 is handled inside iret_protected via
        // iret16_stack_return_from_v86.
        if self.protected_mode() {
            return self.iret_protected_16();
        }

        // V8086 mode IRET (Bochs ctrl_xfer16.cc:558-561)
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
            let cs_val = self.sregs[super::decoder::BxSegregs::Cs as usize].selector.value;
            if cs_val == 0xF000 {
                let cf = new_flags & 1;
                tracing::warn!(
                    "IRET from BIOS: CS:IP={:04x}:{:04x} → {:04x}:{:04x} FLAGS={:04x} CF={} AH={:#04x} icount={}",
                    cs_val, self.rip() as u16, new_cs, new_ip, new_flags, cf, self.ah(), self.icount
                );
            }
        }

        // CS limit check (Bochs ctrl_xfer16.cc:568-571)
        let limit = self.get_segment_limit(BxSegregs::Cs);
        if (new_ip as u32) > limit {
            tracing::error!(
                "iret16: offset {:#06x} outside of CS limits {:#010x}",
                new_ip,
                limit
            );
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Load CS with load_seg_reg (Bochs ctrl_xfer16.cc:573)
        self.load_seg_reg_real_mode(BxSegregs::Cs, new_cs);

        // Set IP (Bochs ctrl_xfer16.cc:574)
        self.set_eip(new_ip as u32);

        // write_flags with change_IOPL=true, change_IF=true (Bochs ctrl_xfer16.cc:575)
        self.write_flags(new_flags, true, true);

        // RSP_COMMIT
        self.speculative_rsp = false;

        tracing::debug!(
            "IRET16: returning to {:04x}:{:04x}, flags={:04x}",
            new_cs,
            new_ip,
            new_flags
        );
        Ok(())
    }

    /// IRET - Return from interrupt (32-bit operand size)
    /// Based on Bochs ctrl_xfer32.cc IRET32 (lines 540-612)
    pub fn iret32(&mut self, _instr: &Instruction) -> super::Result<()> {
        // Invalidate prefetch queue at entry (Bochs ctrl_xfer32.cc:546)
        self.invalidate_prefetch_q();

        // Unmask NMI on every IRET (Bochs ctrl_xfer32.cc:564)
        self.unmask_event(Self::BX_EVENT_NMI);

        // RSP_SPECULATIVE before all mode branches (Bochs ctrl_xfer32.cc:574)
        self.speculative_rsp = true;
        self.prev_rsp = self.esp() as u64;

        // Protected mode dispatch (Bochs ctrl_xfer32.cc:576)
        if self.protected_mode() {
            return self.iret_protected();
        }

        // V8086 mode IRET (Bochs ctrl_xfer32.cc:580-583)
        if self.v8086_mode() {
            return self.iret32_stack_return_from_v86();
        }

        // Real mode: Pop EIP, CS, EFLAGS from stack
        let new_eip = self.pop_32()?;
        let new_cs = self.pop_32()? as u16;
        let new_eflags = self.pop_32()?;

        // CS limit check (Bochs ctrl_xfer32.cc:589-593)
        let limit = self.get_segment_limit(BxSegregs::Cs);
        if new_eip > limit {
            tracing::error!(
                "iret32: offset {:#010x} outside of CS limits {:#010x}",
                new_eip,
                limit
            );
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Load CS with load_seg_reg (Bochs ctrl_xfer32.cc:595)
        self.load_seg_reg_real_mode(BxSegregs::Cs, new_cs);

        // Set EIP (Bochs ctrl_xfer32.cc:596)
        self.set_eip(new_eip);

        // writeEFlags with VIF, VIP, VM unchanged (Bochs ctrl_xfer32.cc:597)
        self.write_eflags(new_eflags, EFlags::IRET32_REAL_CHANGE.bits());

        // RSP_COMMIT
        self.speculative_rsp = false;

        tracing::debug!(
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
        // Based on Bochs iret.cc:44-95
        if self.eflags.contains(EFlags::NT) {
            tracing::debug!("IRET: nested task return (NT=1)");

            // Read back-link selector from current TSS offset 0
            let tss_base = unsafe { self.tr.cache.u.segment.base };
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

        // RSP_SPECULATIVE (Bochs iret.cc:107)
        self.speculative_rsp = true;
        self.prev_rsp = self.esp() as u64;

        // Peek at stack without modifying ESP
        let temp_esp = if self.is_stack_32bit() {
            self.esp()
        } else {
            self.sp() as u32
        };

        let new_eip = self.stack_read_dword(temp_esp + 0)?;
        let raw_cs_raw = self.stack_read_dword(temp_esp + 4)? as u16;
        let new_eflags = self.stack_read_dword(temp_esp + 8)?;

        // If VM bit is set in the saved EFLAGS and CPL==0, stack-return to V86 mode.
        // Bochs iret.cc:121-131
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
        // Based on Bochs iret.cc:148-167
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
            tracing::debug!(
                "IRET32(PM): same-priv return to CS={:#06x} EIP={:#010x} EFLAGS={:#010x}",
                raw_cs_raw,
                new_eip,
                new_eflags
            );

            // Load CS from GDT descriptor (sets CS.base from descriptor, NOT << 4)
            self.branch_far(
                &mut cs_selector,
                &mut cs_descriptor,
                new_eip as u64,
                new_cpl,
            )?;

            // Restore EFLAGS with proper side effects (Bochs iret.cc:197)
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
            tracing::debug!(
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

            // Load CS (sets new CPL = new_cpl)
            self.branch_far(
                &mut cs_selector,
                &mut cs_descriptor,
                new_eip as u64,
                new_cpl,
            )?;

            // Restore EFLAGS with proper side effects (Bochs iret.cc:318)
            self.write_eflags(new_eflags, change_mask);

            // Load SS and restore ESP
            self.load_ss(&mut ss_selector, &mut ss_descriptor, new_cpl)?;
            if self.is_stack_32bit() {
                self.set_esp(new_esp);
            } else {
                self.set_sp(new_esp as u16);
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
            tracing::debug!("IRET16(PM): nested task return (NT=1)");
            let tss_base = unsafe { self.tr.cache.u.segment.base };
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
        self.prev_rsp = self.esp() as u64;

        // Peek at stack — 16-bit reads (6 bytes total)
        let temp_esp = if self.is_stack_32bit() {
            self.esp()
        } else {
            self.sp() as u32
        };

        let new_ip = self.stack_read_word(temp_esp + 0)? as u32;
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
            self.branch_far(&mut cs_selector, &mut cs_descriptor, new_ip as u64, new_cpl)?;

            // write_flags for 16-bit (Bochs iret.cc:201)
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

            self.branch_far(&mut cs_selector, &mut cs_descriptor, new_ip as u64, new_cpl)?;

            // write_flags for 16-bit (Bochs iret.cc:294)
            self.write_flags(new_flags as u16, cpl == 0, cpl <= iopl);

            self.load_ss(&mut ss_selector, &mut ss_descriptor, new_cpl)?;
            self.set_sp(new_sp as u16);

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
    /// Based on Bochs exception.cc:731-760 real_mode_int()
    pub(super) fn interrupt_real_mode(&mut self, vector: u8) -> super::Result<()> {
        // Bochs exception.cc:733-736: IDTR limit check
        if (vector as u32 * 4 + 3) > self.idtr.limit as u32 {
            tracing::error!("interrupt(real mode) vector > idtr.limit");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Save current FLAGS, CS, IP on stack
        // Bochs exception.cc:738-740: push FLAGS, CS, IP
        let flags = (self.eflags.bits() & 0xFFFF) as u16;
        let cs = self.sregs[BxSegregs::Cs as usize].selector.value;
        let ip = self.get_ip();

        self.push_16(flags)?;
        self.push_16(cs)?;
        self.push_16(ip)?;

        // Bochs exception.cc:742: read new IP from IVT using system_read_word (paging-aware)
        let ivt_addr = self.idtr.base + (vector as u64) * 4;
        let new_ip = self.system_read_word(ivt_addr)?;

        // Bochs exception.cc:744-747: CS limit check on loaded IP
        let cs_limit = self.get_segment_limit(BxSegregs::Cs);
        if (new_ip as u32) > cs_limit {
            tracing::error!(
                "interrupt(real mode): instruction pointer not within code segment limits"
            );
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Bochs exception.cc:749: read new CS from IVT
        let new_cs = self.system_read_word(ivt_addr + 2)?;

        // Boot diagnostic: if we ever vector to 0000:0000 in real mode, BIOS likely
        // hit an unexpected exception/IRQ before IVT was initialized (or IVT reads are broken).
        if new_ip == 0 && new_cs == 0 && (self.boot_debug_flags & 0x02) == 0 {
            self.boot_debug_flags |= 0x02;
            self.debug_puts(b"[IVT->0000:0000]\n");
        }

        // Bochs exception.cc:750-751: load CS:IP from IVT
        let cs_index = BxSegregs::Cs as usize;
        parse_selector(new_cs, &mut self.sregs[cs_index].selector);
        unsafe {
            self.sregs[cs_index].cache.u.segment.base = (new_cs as u64) << 4;
        }
        self.set_ip(new_ip);

        // Bochs exception.cc:754-759: clear IF, TF, AC, RF
        self.eflags
            .remove(EFlags::IF_ | EFlags::TF | EFlags::AC | EFlags::RF);
        self.handle_interrupt_mask_change();

        // Invalidate prefetch
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        // Only log non-exception interrupts to reduce spam (exceptions are logged in exception.rs)
        if vector != 0x0d && vector != 0x0e && vector != 0x08 && vector < 0x20 {
            tracing::debug!(
                "INT {:#04x}: vector at {:04x}:{:04x}",
                vector,
                new_cs,
                new_ip
            );
        }
        // Log INT 15h calls (memory detection) — AH=88h returns extended memory in AX
        if vector == 0x15 {
            tracing::debug!(
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
    /// Based on Bochs proc_ctrl.cc:197-235
    pub fn hlt(&mut self, _instr: &Instruction) -> super::Result<()> {
        // CPL is always 0 in real mode
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            tracing::debug!("HLT: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Check if interrupts are disabled (IF=0) - matches Bochs proc_ctrl.cc:206
        if !self.eflags.contains(EFlags::IF_) {
            tracing::warn!("HLT: CPU halted with IF=0 (interrupts disabled) - CPU will be stuck!");
        }

        if !self.diag_first_pm_hlt_captured && self.protected_mode() {
            self.diag_first_pm_hlt_captured = true;
            self.diag_first_pm_hlt_icount = self.icount;
            self.diag_first_pm_hlt_rip = self.eip();
            self.diag_first_pm_hlt_regs = [
                self.eax(), self.ecx(), self.edx(), self.ebx(),
                self.esp(), self.ebp(), self.esi(), self.edi(),
            ];
            self.diag_first_pm_hlt_cs = self.sregs[BxSegregs::Cs as usize].selector.value;
            self.diag_first_pm_hlt_ss = self.sregs[BxSegregs::Ss as usize].selector.value;
            self.diag_first_pm_hlt_eflags = self.eflags.bits();
            // Read 16 dwords from stack
            let esp = self.esp();
            for i in 0..16u32 {
                self.diag_first_pm_hlt_stack[i as usize] = self
                    .stack_read_dword(esp.wrapping_add(i * 4))
                    .unwrap_or(0xDEADDEAD);
            }
        }

        // Set activity state to halted (matches Bochs enter_sleep_state)
        self.activity_state = CpuActivityState::Hlt;

        // Bochs proc_ctrl.cc:181 enter_sleep_state: sets `async_event = 1` (not |= STOP_TRACE).
        // The value 1 (BX_ASYNC_EVENT_SLEEP bit) survives the `&= ~STOP_TRACE` clearing
        // at line 226 of Bochs cpu.cc, ensuring the outer cpu_loop calls handle_async_event
        // on the next iteration to wait for a wake event (interrupt/NMI/SIPI).
        // Without this persistent bit, the CPU would skip the sleep check and execute
        // the instruction after HLT instead of sleeping.
        self.async_event |= BX_ASYNC_EVENT_STOP_TRACE | Self::BX_ASYNC_EVENT_SLEEP;
        Ok(())
    }

    /// XSAVE state component sizes and offsets (Bochs crregs.h:254-282)
    /// Index: XCR0 bit number. (len, offset) for each component.
    const XSAVE_COMPONENTS: [(u32, u32); 10] = [
        (160, 0),      // 0: FPU (x87)
        (256, 160),    // 1: SSE (XMM)
        (256, 576),    // 2: YMM (AVX)
        (0, 0),        // 3: BNDREGS (deprecated MPX)
        (0, 0),        // 4: BNDCFG (deprecated MPX)
        (64, 1088),    // 5: OPMASK (AVX-512)
        (512, 1152),   // 6: ZMM_HI256 (AVX-512)
        (1024, 1664),  // 7: HI_ZMM (AVX-512)
        (0, 0),        // 8: PT (Processor Trace, not implemented)
        (8, 2688),     // 9: PKRU
    ];

    /// Compute max XSAVE area size for given feature bitmap (standard layout).
    /// Bochs cpuid.cc:180-190 xsave_max_size_required_by_features()
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
    /// Bochs cpuid.cc:192-204 xsave_max_size_required_by_xsaves_features()
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
    /// Original: bochs/cpu/proc_ctrl.cc:101-131
    /// Returns CPU identification and feature information in EAX, EBX, ECX, EDX
    /// Input: EAX = function number, ECX = sub-function (for some functions)
    pub fn cpuid(&mut self, _instr: &Instruction) {
        let function = self.eax();
        let sub_function = self.ecx();

        let (mut eax, mut ebx, mut ecx, mut edx) =
            self.cpuid.get_cpuid_leaf(function, sub_function);

        // Dynamic fixups — Bochs computes these from CPU state at runtime.
        // Our static trait returns base values; we patch them here.
        match function {
            0x00000001 => {
                // ECX bit 27 (OSXSAVE): set only when CR4.OSXSAVE is enabled
                // Bochs cpuid.cc:586-588 — base value does NOT include this bit
                if self.cr4.osxsave() {
                    ecx |= 1 << 27; // set OSXSAVE
                }

                // EDX bit 9 (APIC): cleared when APIC is globally disabled
                // Bochs cpuid.cc:651-657
                if self.lapic.get_mode() == super::apic::ApicMode::GloballyDisabled {
                    edx &= !(1 << 9); // clear APIC feature bit
                }
            }
            0x0000000D => {
                if sub_function == 0 {
                    // Subleaf 0: EAX = xcr0_suppmask, ECX = max size for all features
                    // EBX = max size for currently enabled features (current xcr0)
                    // Bochs cpuid.cc:216-224
                    eax = self.xcr0_suppmask;
                    ebx = self.xsave_max_size_for_features(self.xcr0.get32());
                    ecx = self.xsave_max_size_for_features(self.xcr0_suppmask);
                } else if sub_function == 1 {
                    // Subleaf 1 EAX: XSAVE feature flags (Bochs cpuid.cc:234-240)
                    // Bit 0: XSAVEOPT, Bit 1: XSAVEC, Bit 2: XGETBV_ECX1, Bit 3: XSAVES
                    eax = 0x0000000F; // Skylake-X supports all four
                    // Subleaf 1 EBX: size for XSAVES (XCR0 | IA32_XSS)
                    // Bochs cpuid.cc:244-245
                    ebx = self.xsave_compacted_size_for_features(
                        self.xcr0.get32() | (self.msr.ia32_xss as u32),
                    );
                    ecx = self.ia32_xss_suppmask;
                } else if sub_function >= 2 && sub_function < 19 {
                    // Per-component sub-leaves: check if component is supported
                    let support_mask = self.xcr0_suppmask | self.ia32_xss_suppmask;
                    if support_mask & (1 << sub_function) == 0 {
                        eax = 0;
                        ebx = 0;
                        ecx = 0;
                        edx = 0;
                    } else {
                        // ECX bit 0: managed via IA32_XSS (not XCR0)
                        ecx = u32::from(self.ia32_xss_suppmask & (1 << sub_function) != 0);
                    }
                }
            }
            0x80000001 => {
                // EDX bit 11 (SYSCALL/SYSRET): only in long mode
                // Bochs cpuid.cc:860-861
                if self.long64_mode() {
                    edx |= 1 << 11; // BX_CPUID_EXT1_EDX_SYSCALL_SYSRET
                }
            }
            _ => {}
        }

        // Bochs proc_ctrl.cc:124-127: RAX = leaf.eax (writes 64-bit, zero-extending)
        self.set_rax(eax as u64);
        self.set_rbx(ebx as u64);
        self.set_rcx(ecx as u64);
        self.set_rdx(edx as u64);

    }
}
