//! 64-bit control transfer instructions for x86 CPU emulation
//!
//! Based on Bochs ctrl_xfer64.cc

use alloc::string::ToString;

use super::{
    cpu::{BxCpuC, Exception},
    cpuid::BxCpuIdTrait,
    decoder::{Instruction, BxSegregs},
    error::{CpuError, Result},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Helper functions for branching
    // =========================================================================

    /// Branch to a near 64-bit address
    /// Matching C++ ctrl_xfer64.cc:29-44 branch_near64
    fn branch_near64(&mut self, instr: &Instruction) {
        let new_rip = self.rip().wrapping_add(instr.id() as i32 as u64);

        // Check canonical address (matching C++ line 33-36)
        if !self.is_canonical(new_rip) {
            tracing::error!("branch_near64: canonical RIP violation");
            // In C++, this calls exception(BX_GP_EXCEPTION, 0) which doesn't return
        }

        self.set_rip(new_rip);

        // Matching C++ lines 40-43: Set STOP_TRACE when handlers chaining is disabled
        // Since we don't have handlers chaining yet, we always set it
        // assert magic async_event to stop trace execution
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
    }

    // =========================================================================
    // Flag getters for conditional jumps
    // =========================================================================

    /// Get Carry Flag
    // Flag getters (get_cf, get_zf, get_sf, get_of, get_pf, get_af) are defined in ctrl_xfer32.rs
    // to avoid duplicate definitions across multiple impl blocks

    // =========================================================================
    // CALL instructions (64-bit)
    // =========================================================================

    /// Near call with 64-bit displacement
    /// Matching C++ ctrl_xfer64.cc:104-133 CALL_Jq
    pub fn call_jq(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.rip().wrapping_add(instr.id() as i32 as u64);

        // Check canonical address (matching C++ line 121-124)
        if !self.is_canonical(new_rip) {
            tracing::error!("call_jq: canonical RIP violation");
            return Err(CpuError::BadVector { vector: Exception::Gp });
        }

        // Push 64 bit EA of next instruction (matching C++ line 115)
        self.push_64(self.rip());

        self.set_rip(new_rip);
        Ok(())
    }

    /// Near call indirect (64-bit register)
    /// Matching C++ ctrl_xfer64.cc:135-169 CALL_EqR
    pub fn call_eq_r(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.get_gpr64(instr.dst() as usize);

        // Check canonical address (matching C++ line 152-156)
        if !self.is_canonical(new_rip) {
            tracing::error!("call_eq_r: canonical RIP violation");
            return Err(CpuError::BadVector { vector: Exception::Gp });
        }

        // Push 64 bit EA of next instruction (matching C++ line 146)
        self.push_64(self.rip());

        self.set_rip(new_rip);
        Ok(())
    }

    /// Far call indirect (64-bit)
    /// Matching C++ ctrl_xfer64.cc:171-200 CALL64_Ep
    pub fn call64_ep(&mut self, instr: &Instruction) -> Result<()> {
        // Invalidate prefetch queue (matching C++ line 173)
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        // Resolve effective address
        let eaddr = self.resolve_addr64(instr);

        // Read offset and segment from memory (matching C++ lines 184-185)
        // pointer, segment address pair
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let _op1_64 = self.read_linear_qword(seg, laddr);
        let asize_mask = if instr.as64_l() != 0 { 0xFFFFFFFFFFFFFFFF } else { 0xFFFFFFFF };
        let _cs_raw = self.read_linear_word(seg, self.get_laddr64(seg_idx, (eaddr.wrapping_add(8)) & asize_mask));

        // TODO: Implement call_protected for protected mode (matching C++ line 191)
        // For now, return error if not in real mode
        if !self.real_mode() {
            return Err(CpuError::UnimplementedOpcode {
                opcode: "call64_ep protected mode".to_string(),
            });
        }

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // JMP instructions (64-bit)
    // =========================================================================

    /// Near jump with 64-bit displacement
    /// Matching C++ ctrl_xfer64.cc:202-216 JMP_Jq
    pub fn jmp_jq(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.rip().wrapping_add(instr.id() as i32 as u64);

        // Check canonical address (matching C++ line 206-209)
        if !self.is_canonical(new_rip) {
            tracing::error!("jmp_jq: canonical RIP violation");
            return Err(CpuError::BadVector { vector: Exception::Gp });
        }

        self.set_rip(new_rip);
        Ok(())
    }

    /// Near jump indirect (64-bit register)
    /// Matching C++ ctrl_xfer64.cc:410-427 JMP_EqR
    pub fn jmp_eq_r(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.get_gpr64(instr.dst() as usize);

        // Check canonical address (matching C++ line 414-417)
        if !self.is_canonical(new_rip) {
            tracing::error!("jmp_eq_r: canonical RIP violation");
            return Err(CpuError::BadVector { vector: Exception::Gp });
        }

        self.set_rip(new_rip);
        Ok(())
    }

    /// Far jump indirect (64-bit)
    /// Matching C++ ctrl_xfer64.cc:430-450 JMP64_Ep
    pub fn jmp64_ep(&mut self, instr: &Instruction) -> Result<()> {
        // Invalidate prefetch queue (matching C++ line 432)
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        // Resolve effective address
        let eaddr = self.resolve_addr64(instr);

        // Read offset and segment from memory (matching C++ lines 438-439)
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let _op1_64 = self.read_linear_qword(seg, laddr);
        let asize_mask = if instr.as64_l() != 0 { 0xFFFFFFFFFFFFFFFF } else { 0xFFFFFFFF };
        let _cs_raw = self.read_linear_word(seg, self.get_laddr64(seg_idx, (eaddr.wrapping_add(8)) & asize_mask));

        // TODO: Implement jump_protected for protected mode (matching C++ line 443)
        // For now, return error if not in real mode
        if !self.real_mode() {
            return Err(CpuError::UnimplementedOpcode {
                opcode: "jmp64_ep protected mode".to_string(),
            });
        }

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // RET instructions (64-bit)
    // =========================================================================

    /// Near return with immediate (64-bit)
    /// Matching C++ ctrl_xfer64.cc:46-76 RETnear64_Iw
    pub fn retnear64_iw(&mut self, instr: &Instruction) -> Result<()> {
        let return_rip = self.pop_64();

        // Check canonical address (matching C++ line 63-66)
        if !self.is_canonical(return_rip) {
            tracing::error!("retnear64_iw: canonical RIP violation");
            return Err(CpuError::BadVector { vector: Exception::Gp });
        }

        self.set_rip(return_rip);
        // Pop additional bytes from stack (matching C++ line 69)
        self.set_rsp(self.rsp().wrapping_add(instr.iw() as u64));

        Ok(())
    }

    /// Far return with immediate (64-bit)
    /// Matching C++ ctrl_xfer64.cc:78-102 RETfar64_Iw
    /// Note: return_protected is RSP safe
    pub fn retfar64_iw(&mut self, _instr: &Instruction) -> Result<()> {
        // Invalidate prefetch queue (matching C++ line 80)
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        // TODO: Implement return_protected for protected mode (matching C++ line 93)
        // For now, return error if not in real mode
        if !self.real_mode() {
            return Err(CpuError::UnimplementedOpcode {
                opcode: "retfar64_iw protected mode".to_string(),
            });
        }

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// Far return without immediate (64-bit)
    pub fn retfar64(&mut self, instr: &Instruction) -> Result<()> {
        // Same as RETfar64_Iw but without imm16
        self.retfar64_iw(instr)
    }

    /// Interrupt return (64-bit)
    /// Matching C++ ctrl_xfer64.cc:456-505 IRET64
    pub fn iret64(&mut self, _instr: &Instruction) -> Result<()> {
        // Invalidate prefetch queue (matching C++ line 458)
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        // TODO: Implement long_iret for long mode (matching C++ line 492)
        // For now, return error
        Err(CpuError::UnimplementedOpcode {
            opcode: "iret64 long mode".to_string(),
        })
    }

    // =========================================================================
    // Conditional JMP instructions (64-bit displacement, Jq variants)
    // =========================================================================
    // Note: trace can continue over non-taken branch (matching C++ comment)

    /// Jump if overflow (OF=1)
    /// Matching C++ ctrl_xfer64.cc:218-228 JO_Jq
    pub fn jo_jq(&mut self, instr: &Instruction) {
        if self.get_of() {
            self.branch_near64(instr);
            tracing::trace!("JO rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if not overflow (OF=0)
    /// Matching C++ ctrl_xfer64.cc:230-240 JNO_Jq
    pub fn jno_jq(&mut self, instr: &Instruction) {
        if !self.get_of() {
            self.branch_near64(instr);
            tracing::trace!("JNO rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if below/carry (CF=1)
    /// Matching C++ ctrl_xfer64.cc:242-252 JB_Jq
    pub fn jb_jq(&mut self, instr: &Instruction) {
        if self.get_cf() {
            self.branch_near64(instr);
            tracing::trace!("JB/JC rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if not below/no carry (CF=0)
    /// Matching C++ ctrl_xfer64.cc:254-264 JNB_Jq
    pub fn jnb_jq(&mut self, instr: &Instruction) {
        if !self.get_cf() {
            self.branch_near64(instr);
            tracing::trace!("JNB/JNC rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if zero/equal (ZF=1)
    /// Matching C++ ctrl_xfer64.cc:266-276 JZ_Jq
    pub fn jz_jq(&mut self, instr: &Instruction) {
        if self.get_zf() {
            self.branch_near64(instr);
            tracing::trace!("JZ/JE rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if not zero/not equal (ZF=0)
    /// Matching C++ ctrl_xfer64.cc:278-288 JNZ_Jq
    pub fn jnz_jq(&mut self, instr: &Instruction) {
        if !self.get_zf() {
            self.branch_near64(instr);
            tracing::trace!("JNZ/JNE rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if below or equal (CF=1 or ZF=1)
    /// Matching C++ ctrl_xfer64.cc:290-300 JBE_Jq
    pub fn jbe_jq(&mut self, instr: &Instruction) {
        if self.get_cf() || self.get_zf() {
            self.branch_near64(instr);
            tracing::trace!("JBE/JNA rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if not below or equal/above (CF=0 and ZF=0)
    /// Matching C++ ctrl_xfer64.cc:302-312 JNBE_Jq
    pub fn jnbe_jq(&mut self, instr: &Instruction) {
        if !self.get_cf() && !self.get_zf() {
            self.branch_near64(instr);
            tracing::trace!("JNBE/JA rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if sign (SF=1)
    /// Matching C++ ctrl_xfer64.cc:314-324 JS_Jq
    pub fn js_jq(&mut self, instr: &Instruction) {
        if self.get_sf() {
            self.branch_near64(instr);
            tracing::trace!("JS rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if not sign (SF=0)
    /// Matching C++ ctrl_xfer64.cc:326-336 JNS_Jq
    pub fn jns_jq(&mut self, instr: &Instruction) {
        if !self.get_sf() {
            self.branch_near64(instr);
            tracing::trace!("JNS rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if parity/parity even (PF=1)
    /// Matching C++ ctrl_xfer64.cc:338-348 JP_Jq
    pub fn jp_jq(&mut self, instr: &Instruction) {
        if self.get_pf() {
            self.branch_near64(instr);
            tracing::trace!("JP/JPE rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if no parity/parity odd (PF=0)
    /// Matching C++ ctrl_xfer64.cc:350-360 JNP_Jq
    pub fn jnp_jq(&mut self, instr: &Instruction) {
        if !self.get_pf() {
            self.branch_near64(instr);
            tracing::trace!("JNP/JPO rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if less (SF != OF)
    /// Matching C++ ctrl_xfer64.cc:362-372 JL_Jq
    pub fn jl_jq(&mut self, instr: &Instruction) {
        if self.get_sf() != self.get_of() {
            self.branch_near64(instr);
            tracing::trace!("JL/JNGE rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if not less/greater or equal (SF == OF)
    /// Matching C++ ctrl_xfer64.cc:374-384 JNL_Jq
    pub fn jnl_jq(&mut self, instr: &Instruction) {
        if self.get_sf() == self.get_of() {
            self.branch_near64(instr);
            tracing::trace!("JNL/JGE rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if less or equal (ZF=1 or SF!=OF)
    /// Matching C++ ctrl_xfer64.cc:386-396 JLE_Jq
    pub fn jle_jq(&mut self, instr: &Instruction) {
        if self.get_zf() || (self.get_sf() != self.get_of()) {
            self.branch_near64(instr);
            tracing::trace!("JLE/JNG rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    /// Jump if not less or equal/greater (ZF=0 and SF==OF)
    /// Matching C++ ctrl_xfer64.cc:398-408 JNLE_Jq
    pub fn jnle_jq(&mut self, instr: &Instruction) {
        if !self.get_zf() && (self.get_sf() == self.get_of()) {
            self.branch_near64(instr);
            tracing::trace!("JNLE/JG rel64 taken: RIP = {:#x}", self.rip());
        }
    }

    // =========================================================================
    // LOOP instructions (64-bit mode)
    // =========================================================================

    /// Decrement RCX, jump if not zero (64-bit mode)
    /// Matching C++ ctrl_xfer64.cc:608-642 LOOP64_Jb
    /// Note: There is some weirdness in LOOP instructions definition. If an exception
    /// was generated during the instruction execution (for example #GP fault
    /// because EIP was beyond CS segment limits) CPU state should restore the
    /// state prior to instruction execution.
    /// The final point that we are not allowed to decrement RCX register before
    /// it is known that no exceptions can happen.
    pub fn loop64_jb(&mut self, instr: &Instruction) {
        if instr.as64_l() != 0 {
            let count = self.get_gpr64(1).wrapping_sub(1);

            if count != 0 {
                self.branch_near64(instr);
            }

            self.set_gpr64(1, count);
        } else {
            let count = self.get_gpr32(1).wrapping_sub(1);

            if count != 0 {
                self.branch_near64(instr);
            }

            self.set_gpr32(1, count);
        }
    }

    /// Decrement RCX, jump if not zero and ZF=1 (64-bit mode)
    /// Matching C++ ctrl_xfer64.cc:572-603 LOOPE64_Jb
    pub fn loope64_jb(&mut self, instr: &Instruction) {
        if instr.as64_l() != 0 {
            let count = self.get_gpr64(1).wrapping_sub(1);

            if count != 0 && self.get_zf() {
                self.branch_near64(instr);
            }

            self.set_gpr64(1, count);
        } else {
            let count = self.get_gpr32(1).wrapping_sub(1);

            if count != 0 && self.get_zf() {
                self.branch_near64(instr);
            }

            self.set_gpr32(1, count);
        }
    }

    /// Decrement RCX, jump if not zero and ZF=0 (64-bit mode)
    /// Matching C++ ctrl_xfer64.cc:536-570 LOOPNE64_Jb
    /// Note: There is some weirdness in LOOP instructions definition. If an exception
    /// was generated during the instruction execution (for example #GP fault
    /// because EIP was beyond CS segment limits) CPU state should restore the
    /// state prior to instruction execution.
    /// The final point that we are not allowed to decrement RCX register before
    /// it is known that no exceptions can happen.
    pub fn loopne64_jb(&mut self, instr: &Instruction) {
        if instr.as64_l() != 0 {
            let count = self.get_gpr64(1).wrapping_sub(1);

            if count != 0 && !self.get_zf() {
                self.branch_near64(instr);
            }

            self.set_gpr64(1, count);
        } else {
            let count = self.get_gpr32(1).wrapping_sub(1);

            if count != 0 && !self.get_zf() {
                self.branch_near64(instr);
            }

            self.set_gpr32(1, count);
        }
    }

    /// Jump if RCX is zero (64-bit)
    /// Matching C++ ctrl_xfer64.cc:507-524 JRCXZ_Jb
    pub fn jrcxz_jb(&mut self, instr: &Instruction) {
        let temp_rcx = if instr.as64_l() != 0 {
            self.get_gpr64(1) // RCX
        } else {
            self.get_gpr32(1) as u64 // ECX
        };

        if temp_rcx == 0 {
            self.branch_near64(instr);
            tracing::trace!("JRCXZ taken: RIP = {:#x}", self.rip());
        }
    }

    // =========================================================================
    // Helper functions for memory access
    // =========================================================================

    /// Resolve effective address in 64-bit mode (matches BX_CPU_RESOLVE_ADDR_64)
    // resolve_addr64 is defined in data_xfer64.rs and is now pub(crate)
    // Since both are impl BxCpuC, we can call it directly

    /// Read 64-bit qword from linear address (matches read_linear_qword)
    pub(super) fn read_linear_qword(&self, _seg: BxSegregs, laddr: u64) -> u64 {
        self.mem_read_qword(laddr)
    }

    /// Read 16-bit word from linear address (matches read_linear_word)
    pub(super) fn read_linear_word(&self, _seg: BxSegregs, laddr: u64) -> u16 {
        self.mem_read_word(laddr)
    }
    
    /// Write 16-bit word to linear address (matches write_linear_word)
    pub(super) fn write_linear_word(&mut self, _seg: BxSegregs, laddr: u64, value: u16) {
        self.mem_write_word(laddr, value)
    }
}
