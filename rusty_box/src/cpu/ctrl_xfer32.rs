//! 32-bit control transfer instructions for x86 CPU emulation
//!
//! Based on Bochs ctrl_xfer32.cc

use super::{
    cpu::{BxCpuC, Exception},
    cpuid::BxCpuIdTrait,
    decoder::{BxInstructionGenerated, BxSegregs},
    error::{CpuError, Result},
    segment_ctrl_pro::parse_selector,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Helper functions for branching
    // =========================================================================

    /// Branch to a near 32-bit address
    /// Matching C++ ctrl_xfer32.cc:29-46 branch_near32
    fn branch_near32(&mut self, new_eip: u32) -> Result<()> {
        // Check CS limit (matching C++ line 33-37)
        // Original: Bochs cpu/ctrl_xfer32.cc:33-37
        let limit = self.get_segment_limit(BxSegregs::Cs);
        if new_eip > limit {
            tracing::error!("branch_near32: offset {:#010x} outside of CS limits {:#010x}", new_eip, limit);
            // Original: Bochs calls exception(BX_GP_EXCEPTION, 0) which doesn't return
            return Err(CpuError::BadVector { vector: Exception::Gp });
        }

        // Matching C++ line 39: EIP = new_EIP;
        self.set_eip(new_eip);

        // Matching C++ lines 41-44: Set STOP_TRACE when handlers chaining is disabled
        // In C++, this is conditional on BX_SUPPORT_HANDLERS_CHAINING_SPEEDUPS == 0
        // Since we don't have handlers chaining yet, we always set it
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        // Note: C++ branch_near16/32 don't call invalidate_prefetch_q() - only far jumps do
        // The STOP_TRACE flag is enough to break the trace loop, and getICacheEntry will fetch from new location

        Ok(())
    }

    // =========================================================================
    // Flag getters for conditional jumps
    // =========================================================================

    /// Get Carry Flag
    pub fn get_cf(&self) -> bool {
        (self.eflags & (1 << 0)) != 0
    }

    /// Get Zero Flag
    pub fn get_zf(&self) -> bool {
        (self.eflags & (1 << 6)) != 0
    }

    /// Get Sign Flag
    pub fn get_sf(&self) -> bool {
        (self.eflags & (1 << 7)) != 0
    }

    /// Get Overflow Flag
    pub fn get_of(&self) -> bool {
        (self.eflags & (1 << 11)) != 0
    }

    /// Get Parity Flag
    pub fn get_pf(&self) -> bool {
        (self.eflags & (1 << 2)) != 0
    }

    /// Get Auxiliary Flag (not directly used in conditionals, but useful)
    pub fn get_af(&self) -> bool {
        (self.eflags & (1 << 4)) != 0
    }

    // =========================================================================
    // JMP instructions
    // =========================================================================

    /// JMP rel32 - Near jump with 32-bit signed displacement
    pub fn jmp_jd(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let disp = instr.id() as i32;
        let eip = self.eip();
        let new_eip = (eip as i32).wrapping_add(disp) as u32;
        self.branch_near32(new_eip)?;
        tracing::trace!("JMP rel32: EIP = {:#010x}", new_eip);
        Ok(())
    }

    /// JMP r/m32 - Indirect jump through register
    pub fn jmp_ed_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let dst = instr.dst() as usize;
        let new_eip = self.get_gpr32(dst);
        self.branch_near32(new_eip)?;
        tracing::trace!("JMP r/m32: EIP = {:#010x}", new_eip);
        Ok(())
    }

    // =========================================================================
    // CALL instructions
    // =========================================================================

    /// CALL rel32 - Near call with 32-bit displacement
    pub fn call_jd(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let disp = instr.id() as i32;
        let eip = self.eip();

        // Push return address
        let esp_before = self.esp();
        self.push_32(eip);
        let esp_after = self.esp();

        let new_eip = (eip as i32).wrapping_add(disp) as u32;

        if eip == 0 {
            tracing::error!("CALL rel32 SUSPICIOUS: pushing return_addr={:#x}, ESP {:#x}->{:#x}, target={:#x}",
                eip, esp_before, esp_after, new_eip);
        }

        self.branch_near32(new_eip)?;
        tracing::trace!("CALL rel32: EIP = {:#010x}, ret = {:#010x}", new_eip, eip);
        Ok(())
    }

    /// CALL r/m32 - Indirect call through register
    pub fn call_ed_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let dst = instr.dst() as usize;
        let new_eip = self.get_gpr32(dst);
        let eip = self.eip();

        self.push_32(eip);
        self.branch_near32(new_eip)?;
        tracing::trace!("CALL r/m32: EIP = {:#010x}, ret = {:#010x}", new_eip, eip);
        Ok(())
    }

    // =========================================================================
    // RET instructions
    // =========================================================================

    /// RET near - Return from procedure (32-bit)
    pub fn ret_near32(&mut self, _instr: &BxInstructionGenerated) -> Result<()> {
        let current_eip = self.eip();
        let esp_before = self.esp();

        // Read what's on stack before popping
        let stack_val = self.stack_read_dword(esp_before);

        let return_eip = self.pop_32();

        if return_eip == 0 || return_eip > 0xfffff {
            tracing::error!("RET near32 SUSPICIOUS: current_eip={:#x}, return_eip={:#x}, ESP before={:#x}, stack_val={:#x}",
                current_eip, return_eip, esp_before, stack_val);
        }

        self.branch_near32(return_eip)?;
        tracing::trace!("RET near32: EIP = {:#010x}", return_eip);
        Ok(())
    }

    /// RET near imm16 - Return and pop imm16 bytes (32-bit)
    pub fn ret_near32_iw(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let return_eip = self.pop_32();
        let imm16 = instr.iw();

        self.branch_near32(return_eip)?;

        let ss_d_b = self.get_segment_d_b(BxSegregs::Ss);
        if ss_d_b {
            let esp = self.get_gpr32(4);
            self.set_gpr32(4, esp.wrapping_add(imm16 as u32));
        } else {
            let sp = self.get_gpr16(4);
            self.set_gpr16(4, sp.wrapping_add(imm16));
        }
        tracing::trace!("RET near32 imm16: EIP = {:#010x}, pop = {}", return_eip, imm16);
        Ok(())
    }

    // =========================================================================
    // Conditional JMP instructions (32-bit displacement, Jd variants)
    // =========================================================================

    /// JO rel32 - Jump if overflow (OF=1)
    pub fn jo_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if self.get_of() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JO rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JNO rel32 - Jump if not overflow (OF=0)
    pub fn jno_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if !self.get_of() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JNO rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JB/JC/JNAE rel32 - Jump if below/carry (CF=1)
    pub fn jb_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if self.get_cf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JB/JC rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JNB/JNC/JAE rel32 - Jump if not below/no carry (CF=0)
    pub fn jnb_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if !self.get_cf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JNB/JNC rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JZ/JE rel32 - Jump if zero/equal (ZF=1)
    pub fn jz_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if self.get_zf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JZ/JE rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JNZ/JNE rel32 - Jump if not zero/not equal (ZF=0)
    pub fn jnz_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if !self.get_zf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JNZ/JNE rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JBE/JNA rel32 - Jump if below or equal (CF=1 or ZF=1)
    pub fn jbe_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if self.get_cf() || self.get_zf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JBE/JNA rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JNBE/JA rel32 - Jump if not below or equal/above (CF=0 and ZF=0)
    pub fn jnbe_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if !self.get_cf() && !self.get_zf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JNBE/JA rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JS rel32 - Jump if sign (SF=1)
    pub fn js_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if self.get_sf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JS rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JNS rel32 - Jump if not sign (SF=0)
    pub fn jns_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if !self.get_sf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JNS rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JP/JPE rel32 - Jump if parity/parity even (PF=1)
    pub fn jp_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if self.get_pf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JP/JPE rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JNP/JPO rel32 - Jump if no parity/parity odd (PF=0)
    pub fn jnp_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if !self.get_pf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JNP/JPO rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JL/JNGE rel32 - Jump if less (SF != OF)
    pub fn jl_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if self.get_sf() != self.get_of() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JL/JNGE rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JNL/JGE rel32 - Jump if not less/greater or equal (SF == OF)
    pub fn jnl_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if self.get_sf() == self.get_of() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JNL/JGE rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JLE/JNG rel32 - Jump if less or equal (ZF=1 or SF!=OF)
    pub fn jle_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if self.get_zf() || (self.get_sf() != self.get_of()) {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JLE/JNG rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    /// JNLE/JG rel32 - Jump if not less or equal/greater (ZF=0 and SF==OF)
    pub fn jnle_jd(&mut self, instr: &BxInstructionGenerated)  -> Result<()> {
        if !self.get_zf() && (self.get_sf() == self.get_of()) {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("JNLE/JG rel32 taken: EIP = {:#010x}", new_eip);
        }
        Ok(())
    }

    // =========================================================================
    // LOOP instructions (32-bit mode)
    // =========================================================================

    /// LOOP32 rel8 - Decrement ECX, jump if not zero (32-bit mode)
    /// Matching C++ ctrl_xfer32.cc (similar to LOOP16_Jb but 32-bit)
    pub fn loop32_jb(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let ecx = self.get_gpr32(1);
        let count = ecx.wrapping_sub(1);
        self.set_gpr32(1, count);

        if count != 0 {
            let disp = instr.ib() as i8;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp as i32) as u32;
            self.branch_near32(new_eip)?;
            tracing::trace!("LOOP32 taken: EIP = {:#010x}, ECX = {}", new_eip, count);
        }
        Ok(())
    }

    /// LOOPE32/LOOPZ32 rel8 - Decrement ECX, jump if not zero and ZF=1 (32-bit mode)
    pub fn loope32_jb(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let ecx = self.get_gpr32(1);
        let count = ecx.wrapping_sub(1);
        self.set_gpr32(1, count);

        if count != 0 && self.get_zf() {
            let disp = instr.ib() as i8;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp as i32) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// LOOPNE32/LOOPNZ32 rel8 - Decrement ECX, jump if not zero and ZF=0 (32-bit mode)
    pub fn loopne32_jb(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let ecx = self.get_gpr32(1);
        let count = ecx.wrapping_sub(1);
        self.set_gpr32(1, count);

        if count != 0 && !self.get_zf() {
            let disp = instr.ib() as i8;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp as i32) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    // =========================================================================
    // Helper function for loading segment register in real mode
    // =========================================================================

    /// Load segment register in real mode (matching load_seg_reg for real mode)
    pub(super) fn load_seg_reg_real_mode(&mut self, seg: BxSegregs, selector: u16) {
        parse_selector(selector, &mut self.sregs[seg as usize].selector);
        self.set_segment_base(seg, (selector as u64) << 4);
    }

    // =========================================================================
    // Far jump/call helpers (32-bit)
    // =========================================================================

    /// Far jump 32-bit (matching C++ jmp_far32)
    /// Called by JMP32_Ap and JMP32_Ep
    pub(super) fn jmp_far32(&mut self, instr: &BxInstructionGenerated, cs_raw: u16, disp32: u32) -> Result<()> {
        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        if !self.real_mode() {
            // Protected mode: use jump_protected
            tracing::info!("jmp_far32: cs={:#06x}, disp={:#010x}, real_mode={}, cpu_mode={:?}",
                          cs_raw, disp32, self.real_mode(), self.cpu_mode);
            tracing::info!("Calling jump_protected");
            self.jump_protected(cs_raw, disp32 as u64)?;
        } else {
            // Real mode
            let limit = self.get_segment_limit(BxSegregs::Cs);
            if disp32 > limit {
                tracing::error!("jmp_far32: offset {:#010x} outside of CS limits {:#010x}", disp32, limit);
                return Err(CpuError::BadVector { vector: Exception::Gp });
            }

            self.load_seg_reg_real_mode(BxSegregs::Cs, cs_raw);
            self.set_eip(disp32);
        }

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// Far call 32-bit (matching C++ call_far32)
    /// Called by CALL32_Ap and CALL32_Ep
    fn call_far32(&mut self, instr: &BxInstructionGenerated, cs_raw: u16, disp32: u32) -> Result<()> {
        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        if !self.real_mode() {
            // TODO: Implement call_protected for protected mode
            return Err(CpuError::UnimplementedOpcode {
                opcode: "call_far32 protected mode".to_string(),
            });
        } else {
            // Real mode
            let limit = self.get_segment_limit(BxSegregs::Cs);
            if disp32 > limit {
                tracing::error!("call_far32: offset {:#010x} outside of CS limits {:#010x}", disp32, limit);
                return Err(CpuError::BadVector { vector: Exception::Gp });
            }

            // Push return address (CS:EIP)
            let cs_value = self.sregs[BxSegregs::Cs as usize].selector.value;
            let eip = self.eip();
            self.push_32(cs_value as u32);
            self.push_32(eip);

            self.load_seg_reg_real_mode(BxSegregs::Cs, cs_raw);
            self.set_eip(disp32);
        }

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // Far CALL instructions (32-bit)
    // =========================================================================

    /// CALL32_Ap - Far call with absolute pointer (32-bit)
    /// Matching C++ ctrl_xfer32.cc:219-229
    pub fn call32_ap(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let cs_raw = instr.iw2();
        let disp32 = instr.id();
        self.call_far32(instr, cs_raw, disp32)
    }

    /// CALL32_Ep - Far call indirect (32-bit)
    /// Matching C++ ctrl_xfer32.cc (similar to CALL16_Ep but 32-bit)
    pub fn call32_ep(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // Resolve effective address
        let eaddr = self.resolve_addr32(instr);
        
        // Read offset and segment from memory
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.read_virtual_dword(seg, eaddr);
        let cs_raw = self.read_virtual_word(seg, (eaddr.wrapping_add(4)) & (if instr.as32_l() == 0 { 0xFFFF } else { 0xFFFFFFFF }));
        
        self.call_far32(instr, cs_raw, op1_32)
    }

    // =========================================================================
    // Far JMP instructions (32-bit)
    // =========================================================================

    /// JMP32_Ap - Far jump with absolute pointer (32-bit)
    /// Matching C++ ctrl_xfer32.cc (similar to CALL32_Ap but for jump)
    pub fn jmp32_ap(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let cs_raw = instr.iw2();
        let disp32 = instr.id();
        self.jmp_far32(instr, cs_raw, disp32)
    }

    /// JMP32_Ep - Far jump indirect (32-bit)
    /// Matching C++ ctrl_xfer32.cc (similar to JMP16_Ep but 32-bit)
    pub fn jmp32_ep(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // Resolve effective address
        let eaddr = self.resolve_addr32(instr);
        
        // Read offset and segment from memory
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.read_virtual_dword(seg, eaddr);
        let cs_raw = self.read_virtual_word(seg, (eaddr.wrapping_add(4)) & (if instr.as32_l() == 0 { 0xFFFF } else { 0xFFFFFFFF }));
        
        self.jmp_far32(instr, cs_raw, op1_32)
    }

    // =========================================================================
    // Far RET instructions (32-bit)
    // =========================================================================

    /// RETfar32 - Far return without immediate (32-bit)
    /// Matching C++ ctrl_xfer32.cc (similar to RETfar16 but 32-bit)
    pub fn retfar32(&mut self, _instr: &BxInstructionGenerated) -> Result<()> {
        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        if !self.real_mode() {
            // TODO: Implement return_protected for protected mode
            return Err(CpuError::UnimplementedOpcode {
                opcode: "retfar32 protected mode".to_string(),
            });
        } else {
            // Real mode - pop EIP and CS (32-bit pop, MSW discarded for CS)
            let eip = self.pop_32();
            let cs_raw = self.pop_32() as u16; // 32-bit pop, MSW discarded

            // Check CS limit
            let limit = self.get_segment_limit(BxSegregs::Cs);
            if eip > limit {
                tracing::error!("retfar32: offset {:#010x} outside of CS limits {:#010x}", eip, limit);
                return Err(CpuError::BadVector { vector: Exception::Gp });
            }

            self.load_seg_reg_real_mode(BxSegregs::Cs, cs_raw);
            self.set_eip(eip);
        }

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// RETfar32_Iw - Far return with immediate (32-bit)
    /// Matching C++ ctrl_xfer32.cc:149-192
    pub fn retfar32_iw(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        let imm16 = instr.iw() as i16;

        if !self.real_mode() {
            // TODO: Implement return_protected for protected mode
            return Err(CpuError::UnimplementedOpcode {
                opcode: "retfar32_iw protected mode".to_string(),
            });
        } else {
            // Real mode - pop EIP and CS (32-bit pop, MSW discarded for CS)
            let eip = self.pop_32();
            let cs_raw = self.pop_32() as u16; // 32-bit pop, MSW discarded

            // Check CS limit
            let limit = self.get_segment_limit(BxSegregs::Cs);
            if eip > limit {
                tracing::error!("retfar32_iw: offset {:#010x} outside of CS limits {:#010x}", eip, limit);
                return Err(CpuError::BadVector { vector: Exception::Gp });
            }

            self.load_seg_reg_real_mode(BxSegregs::Cs, cs_raw);
            self.set_eip(eip);

            // Pop additional bytes from stack
            let ss_d_b = self.get_segment_d_b(BxSegregs::Ss);
            if ss_d_b {
                let esp = self.get_gpr32(4);
                self.set_gpr32(4, esp.wrapping_add(imm16 as u32));
            } else {
                let sp = self.get_gpr16(4);
                self.set_gpr16(4, sp.wrapping_add(imm16 as u16));
            }
        }

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // Helper functions for memory access
    // =========================================================================

    // Helper methods (resolve_addr32, read_virtual_word) are defined in logical32.rs/logical16.rs to avoid duplicate definitions

    // read_virtual_dword is defined in logical32.rs to avoid duplicate definitions
}
