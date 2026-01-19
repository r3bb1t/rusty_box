//! Control transfer instructions for x86 CPU emulation
//!
//! Based on Bochs ctrl_xfer16.cc and ctrl_xfer32.cc
//! Copyright (C) 2001-2019 The Bochs Project

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxInstructionGenerated, BxSegregs},
    segment_ctrl_pro::parse_selector,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Helper functions for branching
    // =========================================================================

    /// Branch to a near 16-bit address
    /// Matching C++ ctrl_xfer16.cc:27-44 branch_near16
    fn branch_near16(&mut self, new_ip: u16) {
        // Check CS limit (matching C++ line 32-36)
        let limit = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.limit_scaled };
        if (new_ip as u32) > limit {
            tracing::error!("branch_near16: offset {:#06x} outside of CS limits {:#010x}", new_ip, limit);
            // In C++, this calls exception(BX_GP_EXCEPTION, 0) which doesn't return
            // In Rust, we should handle this properly
        }
        
        // Matching C++ line 38: EIP = new_IP;
        self.set_eip(new_ip as u32);
        
        // Matching C++ lines 40-43: Set STOP_TRACE when handlers chaining is disabled
        // In C++, this is conditional on BX_SUPPORT_HANDLERS_CHAINING_SPEEDUPS == 0
        // Since we don't have handlers chaining yet, we always set it
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        // Note: C++ branch_near16/32 don't call invalidate_prefetch_q() - only far jumps do
        // The STOP_TRACE flag is enough to break the trace loop, and getICacheEntry will fetch from new location
    }

    /// Branch to a near 32-bit address
    /// Matching C++ ctrl_xfer32.cc:29-46 branch_near32
    fn branch_near32(&mut self, new_eip: u32) {
        // Check CS limit (matching C++ line 34-38)
        let limit = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.limit_scaled };
        if new_eip > limit {
            tracing::error!("branch_near32: offset {:#010x} outside of CS limits {:#010x}", new_eip, limit);
            // In C++, this calls exception(BX_GP_EXCEPTION, 0) which doesn't return
        }
        
        // Matching C++ line 40: EIP = new_EIP;
        self.set_eip(new_eip);
        
        // Matching C++ lines 42-45: Set STOP_TRACE when handlers chaining is disabled
        // In C++, this is conditional on BX_SUPPORT_HANDLERS_CHAINING_SPEEDUPS == 0
        // Since we don't have handlers chaining yet, we always set it
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        // Note: C++ branch_near16/32 don't call invalidate_prefetch_q() - only far jumps do
        // The STOP_TRACE flag is enough to break the trace loop, and getICacheEntry will fetch from new location
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

    /// JMP rel8 - Short jump with 8-bit signed displacement
    pub fn jmp_jb(&mut self, instr: &BxInstructionGenerated) {
        let disp = instr.ib() as i8;
        let ip = self.get_ip();
        let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
        self.branch_near16(new_ip);
        tracing::trace!("JMP rel8: IP = {:#06x}", new_ip);
    }

    /// JMP rel16 - Near jump with 16-bit signed displacement
    pub fn jmp_jw(&mut self, instr: &BxInstructionGenerated) {
        let disp = instr.iw() as i16;
        let ip = self.get_ip();
        let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
        self.branch_near16(new_ip);
        tracing::trace!("JMP rel16: IP = {:#06x}", new_ip);
    }

    /// JMP rel32 - Near jump with 32-bit signed displacement
    pub fn jmp_jd(&mut self, instr: &BxInstructionGenerated) {
        let disp = instr.id() as i32;
        let eip = self.eip();
        let new_eip = (eip as i32).wrapping_add(disp) as u32;
        self.branch_near32(new_eip);
        tracing::trace!("JMP rel32: EIP = {:#010x}", new_eip);
    }

    /// JMP r/m16 - Indirect jump through register
    pub fn jmp_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let new_ip = self.get_gpr16(dst);
        self.branch_near16(new_ip);
        tracing::trace!("JMP r/m16: IP = {:#06x}", new_ip);
    }

    /// JMP r/m32 - Indirect jump through register
    pub fn jmp_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let new_eip = self.get_gpr32(dst);
        self.branch_near32(new_eip);
        tracing::trace!("JMP r/m32: EIP = {:#010x}", new_eip);
    }

    // =========================================================================
    // CALL instructions
    // =========================================================================

    /// CALL rel16 - Near call with 16-bit displacement
    pub fn call_jw(&mut self, instr: &BxInstructionGenerated) {
        let disp = instr.iw() as i16;
        let ip = self.get_ip();
        
        // Push return address
        self.push_16(ip);
        
        let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
        self.branch_near16(new_ip);
        tracing::trace!("CALL rel16: IP = {:#06x}, ret = {:#06x}", new_ip, ip);
    }

    /// CALL rel32 - Near call with 32-bit displacement
    pub fn call_jd(&mut self, instr: &BxInstructionGenerated) {
        let disp = instr.id() as i32;
        let eip = self.eip();
        
        // Push return address
        self.push_32(eip);
        
        let new_eip = (eip as i32).wrapping_add(disp) as u32;
        self.branch_near32(new_eip);
        tracing::trace!("CALL rel32: EIP = {:#010x}, ret = {:#010x}", new_eip, eip);
    }

    /// CALL r/m16 - Indirect call through register
    pub fn call_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let new_ip = self.get_gpr16(dst);
        let ip = self.get_ip();
        
        self.push_16(ip);
        self.branch_near16(new_ip);
        tracing::trace!("CALL r/m16: IP = {:#06x}, ret = {:#06x}", new_ip, ip);
    }

    /// CALL r/m32 - Indirect call through register
    pub fn call_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let new_eip = self.get_gpr32(dst);
        let eip = self.eip();
        
        self.push_32(eip);
        self.branch_near32(new_eip);
        tracing::trace!("CALL r/m32: EIP = {:#010x}, ret = {:#010x}", new_eip, eip);
    }

    // =========================================================================
    // RET instructions
    // =========================================================================

    /// RET near - Return from procedure (16-bit)
    pub fn ret_near16(&mut self, _instr: &BxInstructionGenerated) {
        let return_ip = self.pop_16();
        self.branch_near16(return_ip);
        tracing::trace!("RET near16: IP = {:#06x}", return_ip);
    }

    /// RET near imm16 - Return and pop imm16 bytes (16-bit)
    pub fn ret_near16_iw(&mut self, instr: &BxInstructionGenerated) {
        let return_ip = self.pop_16();
        let imm16 = instr.iw();
        
        self.branch_near16(return_ip);
        
        // Pop additional bytes from stack
        let ss_d_b = unsafe { self.sregs[BxSegregs::Ss as usize].cache.u.segment.d_b };
        if ss_d_b {
            let esp = self.get_gpr32(4);
            self.set_gpr32(4, esp.wrapping_add(imm16 as u32));
        } else {
            let sp = self.get_gpr16(4);
            self.set_gpr16(4, sp.wrapping_add(imm16));
        }
        tracing::trace!("RET near16 imm16: IP = {:#06x}, pop = {}", return_ip, imm16);
    }

    /// RET near - Return from procedure (32-bit)
    pub fn ret_near32(&mut self, _instr: &BxInstructionGenerated) {
        let return_eip = self.pop_32();
        self.branch_near32(return_eip);
        tracing::trace!("RET near32: EIP = {:#010x}", return_eip);
    }

    /// RET near imm16 - Return and pop imm16 bytes (32-bit)
    pub fn ret_near32_iw(&mut self, instr: &BxInstructionGenerated) {
        let return_eip = self.pop_32();
        let imm16 = instr.iw();
        
        self.branch_near32(return_eip);
        
        let ss_d_b = unsafe { self.sregs[BxSegregs::Ss as usize].cache.u.segment.d_b };
        if ss_d_b {
            let esp = self.get_gpr32(4);
            self.set_gpr32(4, esp.wrapping_add(imm16 as u32));
        } else {
            let sp = self.get_gpr16(4);
            self.set_gpr16(4, sp.wrapping_add(imm16));
        }
        tracing::trace!("RET near32 imm16: EIP = {:#010x}, pop = {}", return_eip, imm16);
    }

    // =========================================================================
    // Conditional JMP instructions (16-bit)
    // =========================================================================

    /// JO rel8 - Jump if overflow (OF=1)
    pub fn jo_jb(&mut self, instr: &BxInstructionGenerated) {
        if self.get_of() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JO taken: IP = {:#06x}", new_ip);
        }
    }

    /// JNO rel8 - Jump if not overflow (OF=0)
    pub fn jno_jb(&mut self, instr: &BxInstructionGenerated) {
        if !self.get_of() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JNO taken: IP = {:#06x}", new_ip);
        }
    }

    /// JB/JC/JNAE rel8 - Jump if below/carry (CF=1)
    pub fn jb_jb(&mut self, instr: &BxInstructionGenerated) {
        if self.get_cf() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JB/JC taken: IP = {:#06x}", new_ip);
        }
    }

    /// JNB/JNC/JAE rel8 - Jump if not below/no carry (CF=0)
    pub fn jnb_jb(&mut self, instr: &BxInstructionGenerated) {
        if !self.get_cf() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JNB/JNC taken: IP = {:#06x}", new_ip);
        }
    }

    /// JZ/JE rel8 - Jump if zero/equal (ZF=1)
    pub fn jz_jb(&mut self, instr: &BxInstructionGenerated) {
        if self.get_zf() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JZ/JE taken: IP = {:#06x}", new_ip);
        }
    }

    /// JNZ/JNE rel8 - Jump if not zero/not equal (ZF=0)
    pub fn jnz_jb(&mut self, instr: &BxInstructionGenerated) {
        if !self.get_zf() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JNZ/JNE taken: IP = {:#06x}", new_ip);
        }
    }

    /// JBE/JNA rel8 - Jump if below or equal (CF=1 or ZF=1)
    pub fn jbe_jb(&mut self, instr: &BxInstructionGenerated) {
        if self.get_cf() || self.get_zf() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JBE/JNA taken: IP = {:#06x}", new_ip);
        }
    }

    /// JNBE/JA rel8 - Jump if not below or equal/above (CF=0 and ZF=0)
    pub fn jnbe_jb(&mut self, instr: &BxInstructionGenerated) {
        if !self.get_cf() && !self.get_zf() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JNBE/JA taken: IP = {:#06x}", new_ip);
        }
    }

    /// JS rel8 - Jump if sign (SF=1)
    pub fn js_jb(&mut self, instr: &BxInstructionGenerated) {
        if self.get_sf() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JS taken: IP = {:#06x}", new_ip);
        }
    }

    /// JNS rel8 - Jump if not sign (SF=0)
    pub fn jns_jb(&mut self, instr: &BxInstructionGenerated) {
        if !self.get_sf() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JNS taken: IP = {:#06x}", new_ip);
        }
    }

    /// JP/JPE rel8 - Jump if parity/parity even (PF=1)
    pub fn jp_jb(&mut self, instr: &BxInstructionGenerated) {
        if self.get_pf() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JP/JPE taken: IP = {:#06x}", new_ip);
        }
    }

    /// JNP/JPO rel8 - Jump if no parity/parity odd (PF=0)
    pub fn jnp_jb(&mut self, instr: &BxInstructionGenerated) {
        if !self.get_pf() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JNP/JPO taken: IP = {:#06x}", new_ip);
        }
    }

    /// JL/JNGE rel8 - Jump if less (SF != OF)
    pub fn jl_jb(&mut self, instr: &BxInstructionGenerated) {
        if self.get_sf() != self.get_of() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JL/JNGE taken: IP = {:#06x}", new_ip);
        }
    }

    /// JNL/JGE rel8 - Jump if not less/greater or equal (SF == OF)
    pub fn jnl_jb(&mut self, instr: &BxInstructionGenerated) {
        if self.get_sf() == self.get_of() {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JNL/JGE taken: IP = {:#06x}", new_ip);
        }
    }

    /// JLE/JNG rel8 - Jump if less or equal (ZF=1 or SF!=OF)
    pub fn jle_jb(&mut self, instr: &BxInstructionGenerated) {
        if self.get_zf() || (self.get_sf() != self.get_of()) {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JLE/JNG taken: IP = {:#06x}", new_ip);
        }
    }

    /// JNLE/JG rel8 - Jump if not less or equal/greater (ZF=0 and SF==OF)
    pub fn jnle_jb(&mut self, instr: &BxInstructionGenerated) {
        if !self.get_zf() && (self.get_sf() == self.get_of()) {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JNLE/JG taken: IP = {:#06x}", new_ip);
        }
    }

    // =========================================================================
    // Conditional JMP instructions (16-bit displacement)
    // =========================================================================

    /// JZ/JE rel16 - Jump if zero/equal (ZF=1)
    pub fn jz_jw(&mut self, instr: &BxInstructionGenerated) {
        if self.get_zf() {
            let disp = instr.iw() as i16;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JZ/JE rel16 taken: IP = {:#06x}", new_ip);
        }
    }

    /// JNZ/JNE rel16 - Jump if not zero/not equal (ZF=0)
    pub fn jnz_jw(&mut self, instr: &BxInstructionGenerated) {
        if !self.get_zf() {
            let disp = instr.iw() as i16;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JNZ/JNE rel16 taken: IP = {:#06x}", new_ip);
        }
    }

    // =========================================================================
    // LOOP instructions
    // =========================================================================

    /// LOOP rel8 - Decrement CX/ECX, jump if not zero
    pub fn loop16_jb(&mut self, instr: &BxInstructionGenerated) {
        let as32l = instr.as32_l() != 0;
        
        if as32l {
            let ecx = self.get_gpr32(1);
            let count = ecx.wrapping_sub(1);
            self.set_gpr32(1, count);
            
            if count != 0 {
                let disp = instr.ib() as i8;
                let ip = self.get_ip();
                let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
                self.branch_near16(new_ip);
                tracing::trace!("LOOP taken (32-bit): IP = {:#06x}, ECX = {}", new_ip, count);
            }
        } else {
            let cx = self.get_gpr16(1);
            let count = cx.wrapping_sub(1);
            self.set_gpr16(1, count);
            
            if count != 0 {
                let disp = instr.ib() as i8;
                let ip = self.get_ip();
                let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
                self.branch_near16(new_ip);
                tracing::trace!("LOOP taken (16-bit): IP = {:#06x}, CX = {}", new_ip, count);
            }
        }
    }

    /// LOOPE/LOOPZ rel8 - Decrement CX/ECX, jump if not zero and ZF=1
    pub fn loope16_jb(&mut self, instr: &BxInstructionGenerated) {
        let as32l = instr.as32_l() != 0;
        
        if as32l {
            let ecx = self.get_gpr32(1);
            let count = ecx.wrapping_sub(1);
            self.set_gpr32(1, count);
            
            if count != 0 && self.get_zf() {
                let disp = instr.ib() as i8;
                let ip = self.get_ip();
                let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
                self.branch_near16(new_ip);
            }
        } else {
            let cx = self.get_gpr16(1);
            let count = cx.wrapping_sub(1);
            self.set_gpr16(1, count);
            
            if count != 0 && self.get_zf() {
                let disp = instr.ib() as i8;
                let ip = self.get_ip();
                let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
                self.branch_near16(new_ip);
            }
        }
    }

    /// LOOPNE/LOOPNZ rel8 - Decrement CX/ECX, jump if not zero and ZF=0
    pub fn loopne16_jb(&mut self, instr: &BxInstructionGenerated) {
        let as32l = instr.as32_l() != 0;
        
        if as32l {
            let ecx = self.get_gpr32(1);
            let count = ecx.wrapping_sub(1);
            self.set_gpr32(1, count);
            
            if count != 0 && !self.get_zf() {
                let disp = instr.ib() as i8;
                let ip = self.get_ip();
                let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
                self.branch_near16(new_ip);
            }
        } else {
            let cx = self.get_gpr16(1);
            let count = cx.wrapping_sub(1);
            self.set_gpr16(1, count);
            
            if count != 0 && !self.get_zf() {
                let disp = instr.ib() as i8;
                let ip = self.get_ip();
                let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
                self.branch_near16(new_ip);
            }
        }
    }

    /// JCXZ rel8 - Jump if CX is zero
    pub fn jcxz_jb(&mut self, instr: &BxInstructionGenerated) {
        let as32l = instr.as32_l() != 0;
        let count = if as32l { self.get_gpr32(1) } else { self.get_gpr16(1) as u32 };
        
        if count == 0 {
            let disp = instr.ib() as i8;
            let ip = self.get_ip();
            let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
            self.branch_near16(new_ip);
            tracing::trace!("JCXZ taken: IP = {:#06x}", new_ip);
        }
    }
}

