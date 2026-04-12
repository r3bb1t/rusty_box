//! 64-bit control transfer instructions for x86 CPU emulation
//!
//! Based on Bochs ctrl_xfer64.cc

use super::{
    cpu::{BxCpuC, Exception},
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    error::{CpuError, Result},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Helper functions for branching
    // =========================================================================

    /// Branch to a near 64-bit address
    /// Matching C++  branch_near64
    pub(super) fn branch_near64(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.rip().wrapping_add(instr.id() as i32 as u64);

        // Check canonical address (matching C++ line 33-36)
        if !self.is_canonical(new_rip) {
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(new_rip);

        // Matching C++ lines 40-43: Set STOP_TRACE when handlers chaining is disabled
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // Flag getters for conditional jumps
    // =========================================================================

    // Get Carry Flag
    // Flag getters (get_cf, get_zf, get_sf, get_of, get_pf, get_af) are defined in ctrl_xfer32.rs
    // to avoid duplicate definitions across multiple impl blocks

    // =========================================================================
    // CALL instructions (64-bit)
    // =========================================================================

    /// Near call with 64-bit displacement
    /// Matching C++  CALL_Jq
    pub fn call_jq(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.rip().wrapping_add(instr.id() as i32 as u64);

        // RSP_SPECULATIVE (matching C++ line 112)
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp();

        // Push BEFORE canonical check (matching C++ line 115)
        self.push_64(self.rip())?;

        if !self.is_canonical(new_rip) {
            self.set_rsp(self.prev_rsp);
            self.speculative_rsp = false;
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(new_rip);

        // RSP_COMMIT (matching C++ line 128)
        self.speculative_rsp = false;
        Ok(())
    }

    /// Near call indirect (64-bit register)
    /// Matching C++  CALL_EqR
    pub fn call_eq_r(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.get_gpr64(instr.dst() as usize);

        // RSP_SPECULATIVE (matching C++ line 143)
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp();

        // Push BEFORE canonical check (matching C++ line 146)
        self.push_64(self.rip())?;

        if !self.is_canonical(new_rip) {
            self.set_rsp(self.prev_rsp);
            self.speculative_rsp = false;
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(new_rip);

        // RSP_COMMIT (matching C++ line 160)
        self.speculative_rsp = false;
        Ok(())
    }

    /// Far call indirect (64-bit)
    /// Matching C++  CALL64_Ep
    pub fn call64_ep(&mut self, instr: &Instruction) -> Result<()> {
        // Invalidate prefetch queue (matching C++ line 173)
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        // Resolve effective address
        let eaddr = self.resolve_addr64(instr);

        // Read offset and segment from memory (matching C++ lines 184-185)
        // pointer, segment address pair
        let seg = BxSegregs::from(instr.seg());
        let op1_64 = self.read_virtual_qword_64(seg, eaddr)?;
        let asize_mask = if instr.as64_l() != 0 {
            0xFFFFFFFFFFFFFFFFu64
        } else {
            0xFFFFFFFF
        };
        let cs_raw = self.read_virtual_word_64(
            seg,
            (eaddr.wrapping_add(8)) & asize_mask,
        )?;

        // BX_ASSERT(protected_mode()) — in 64-bit mode we are always in protected mode
        // (matching C++ line 187)

        // RSP_SPECULATIVE (matching C++ line 189)
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp();

        // call_protected dispatches through the protected mode call mechanism
        // (matching C++ line 191)
        self.call_protected_64(instr, cs_raw, op1_64)?;

        // RSP_COMMIT (matching C++ line 193)
        self.speculative_rsp = false;

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // JMP instructions (64-bit)
    // =========================================================================

    /// Near jump with 64-bit displacement
    /// Matching C++  JMP_Jq
    pub fn jmp_jq(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.rip().wrapping_add(instr.id() as i32 as u64);

        if !self.is_canonical(new_rip) {
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(new_rip);

        // BX_LINK_TRACE(i) — without handler chaining, equivalent to BX_NEXT_TRACE (STOP_TRACE)
        // Matching C++ 
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// Near jump indirect (64-bit register)
    /// Matching C++  JMP_EqR
    pub fn jmp_eq_r(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.get_gpr64(instr.dst() as usize);

        if !self.is_canonical(new_rip) {
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(new_rip);

        // BX_NEXT_TRACE(i) — matching C++ 
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// Far jump indirect (64-bit)
    /// Matching C++  JMP64_Ep
    pub fn jmp64_ep(&mut self, instr: &Instruction) -> Result<()> {
        // Invalidate prefetch queue (matching C++ line 432)
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        // Resolve effective address
        let eaddr = self.resolve_addr64(instr);

        // Read offset and segment from memory (matching C++ lines 438-439)
        let seg = BxSegregs::from(instr.seg());
        let op1_64 = self.read_virtual_qword_64(seg, eaddr)?;
        let asize_mask = if instr.as64_l() != 0 {
            0xFFFFFFFFFFFFFFFFu64
        } else {
            0xFFFFFFFF
        };
        let cs_raw = self.read_virtual_word_64(
            seg,
            (eaddr.wrapping_add(8)) & asize_mask,
        )?;

        // BX_ASSERT(protected_mode()) — in 64-bit mode we are always in protected mode
        // (matching C++ line 441)

        // jump_protected dispatches through the protected mode jump mechanism
        // (matching C++ line 443)
        self.jump_protected(cs_raw, op1_64)?;

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // RET instructions (64-bit)
    // =========================================================================

    /// Near call indirect (64-bit memory form)
    /// Matching C++ ctrl_xfer64.cc — LOAD_Eq + CALL_EqR pattern
    pub fn call_eq_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let new_rip = self.read_virtual_qword_64(seg, eaddr)?;

        // RSP_SPECULATIVE — matching CALL_EqR pattern (C++ )
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp();

        // Push BEFORE canonical check — matching CALL_EqR (C++ )
        self.push_64(self.rip())?;

        if !self.is_canonical(new_rip) {
            self.set_rsp(self.prev_rsp);
            self.speculative_rsp = false;
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(new_rip);

        // RSP_COMMIT — matching CALL_EqR (C++ )
        self.speculative_rsp = false;
        Ok(())
    }

    /// Near call indirect (64-bit unified dispatcher)
    pub fn call_eq(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.call_eq_r(instr)
        } else {
            self.call_eq_m(instr)
        }
    }

    /// Near jump indirect (64-bit memory form)
    /// Matching C++ ctrl_xfer64.cc — LOAD_Eq + JMP_EqR pattern
    pub fn jmp_eq_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let new_rip = self.read_virtual_qword_64(seg, eaddr)?;

        if !self.is_canonical(new_rip) {
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(new_rip);

        // BX_NEXT_TRACE(i) — matching JMP_EqR pattern (C++ )
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// Near jump indirect (64-bit unified dispatcher)
    pub fn jmp_eq(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.jmp_eq_r(instr)
        } else {
            self.jmp_eq_m(instr)
        }
    }

    /// Near return with immediate (64-bit)
    /// Matching C++  RETnear64_Iw
    pub fn retnear64_iw(&mut self, instr: &Instruction) -> Result<()> {
        // RSP_SPECULATIVE (matching C++ line 52)
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp();

        let return_rip = self.pop_64()?;

        if !self.is_canonical(return_rip) {
            // Restore RSP before exception (RSP_SPECULATIVE rollback)
            self.set_rsp(self.prev_rsp);
            self.speculative_rsp = false;
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(return_rip);
        self.set_rsp(self.rsp().wrapping_add(instr.iw() as u64));

        // RSP_COMMIT (matching C++ line 71)
        self.speculative_rsp = false;
        Ok(())
    }

    /// Far return with immediate (64-bit)
    /// Matching C++  RETfar64_Iw
    /// Note: return_protected is RSP safe
    pub fn retfar64_iw(&mut self, instr: &Instruction) -> Result<()> {
        // Invalidate prefetch queue (matching C++ line 80)
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        // BX_ASSERT(protected_mode()) — in 64-bit mode we are always in protected mode
        // (matching C++ line 88)

        // RSP_SPECULATIVE (matching C++ line 90)
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp();

        // return_protected is RSP safe (matching C++ line 93)
        self.return_protected_64(instr, instr.iw())?;

        // RSP_COMMIT (matching C++ line 95)
        self.speculative_rsp = false;

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
    /// Matching C++  IRET64
    pub fn iret64(&mut self, instr: &Instruction) -> Result<()> {
        // Invalidate prefetch queue (matching C++ line 458)
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        // VMX: nmi_unblocking_iret = true (matching C++ line 471)
        // (We don't have VMX guest mode, but set for completeness)

        // Unmask NMI (matching C++ line 478)
        self.unmask_event(Self::BX_EVENT_NMI);

        // BX_ASSERT(long_mode()) — matching C++ line 488

        // RSP_SPECULATIVE (matching C++ line 490)
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp();

        // long_iret dispatches the long mode IRET (matching C++ line 492)
        self.long_iret(instr)?;

        // RSP_COMMIT (matching C++ line 494)
        self.speculative_rsp = false;

        // VMX: nmi_unblocking_iret = false (matching C++ line 497, AFTER RSP_COMMIT)
        self.nmi_unblocking_iret = false;

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // Conditional JMP instructions (64-bit displacement, Jq variants)
    // =========================================================================
    // Note: trace can continue over non-taken branch (matching C++ comment)

    /// Jump if overflow (OF=1)
    pub fn jo_jq(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_of() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if not overflow (OF=0)
    pub fn jno_jq(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_of() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if below/carry (CF=1)
    pub fn jb_jq(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_cf() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if not below/no carry (CF=0)
    pub fn jnb_jq(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_cf() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if zero/equal (ZF=1)
    pub fn jz_jq(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_zf() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if not zero/not equal (ZF=0)
    pub fn jnz_jq(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_zf() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if below or equal (CF=1 or ZF=1)
    pub fn jbe_jq(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_cf() || self.get_zf() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if not below or equal/above (CF=0 and ZF=0)
    pub fn jnbe_jq(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_cf() && !self.get_zf() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if sign (SF=1)
    pub fn js_jq(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_sf() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if not sign (SF=0)
    pub fn jns_jq(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_sf() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if parity/parity even (PF=1)
    pub fn jp_jq(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_pf() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if no parity/parity odd (PF=0)
    pub fn jnp_jq(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_pf() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if less (SF != OF)
    pub fn jl_jq(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_sf() != self.get_of() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if not less/greater or equal (SF == OF)
    pub fn jnl_jq(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_sf() == self.get_of() { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if less or equal (ZF=1 or SF!=OF)
    pub fn jle_jq(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_zf() || (self.get_sf() != self.get_of()) { self.branch_near64(instr)?; }
        Ok(())
    }

    /// Jump if not less or equal/greater (ZF=0 and SF==OF)
    pub fn jnle_jq(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_zf() && (self.get_sf() == self.get_of()) { self.branch_near64(instr)?; }
        Ok(())
    }

    // =========================================================================
    // LOOP instructions (64-bit mode)
    // =========================================================================

    /// Decrement RCX, jump if not zero (64-bit mode)
    /// Matching C++  LOOP64_Jb
    /// Note: There is some weirdness in LOOP instructions definition. If an exception
    /// was generated during the instruction execution (for example #GP fault
    /// because EIP was beyond CS segment limits) CPU state should restore the
    /// state prior to instruction execution.
    /// The final point that we are not allowed to decrement RCX register before
    /// it is known that no exceptions can happen.
    pub fn loop64_jb(&mut self, instr: &Instruction) -> Result<()> {
        if instr.as64_l() != 0 {
            let count = self.get_gpr64(1).wrapping_sub(1);
            if count != 0 { self.branch_near64(instr)?; }
            self.set_gpr64(1, count);
        } else {
            let count = self.get_gpr32(1).wrapping_sub(1);
            if count != 0 { self.branch_near64(instr)?; }
            self.set_gpr32(1, count);
        }
        Ok(())
    }

    /// Decrement RCX, jump if not zero and ZF=1 (64-bit mode)
    /// Matching C++  LOOPE64_Jb
    pub fn loope64_jb(&mut self, instr: &Instruction) -> Result<()> {
        if instr.as64_l() != 0 {
            let count = self.get_gpr64(1).wrapping_sub(1);
            if count != 0 && self.get_zf() { self.branch_near64(instr)?; }
            self.set_gpr64(1, count);
        } else {
            let count = self.get_gpr32(1).wrapping_sub(1);
            if count != 0 && self.get_zf() { self.branch_near64(instr)?; }
            self.set_gpr32(1, count);
        }
        Ok(())
    }

    /// Decrement RCX, jump if not zero and ZF=0 (64-bit mode)
    /// Matching C++  LOOPNE64_Jb
    pub fn loopne64_jb(&mut self, instr: &Instruction) -> Result<()> {
        if instr.as64_l() != 0 {
            let count = self.get_gpr64(1).wrapping_sub(1);
            if count != 0 && !self.get_zf() { self.branch_near64(instr)?; }
            self.set_gpr64(1, count);
        } else {
            let count = self.get_gpr32(1).wrapping_sub(1);
            if count != 0 && !self.get_zf() { self.branch_near64(instr)?; }
            self.set_gpr32(1, count);
        }
        Ok(())
    }

    /// Jump if RCX is zero (64-bit)
    /// Matching C++  JRCXZ_Jb
    pub fn jrcxz_jb(&mut self, instr: &Instruction) -> Result<()> {
        let temp_rcx = if instr.as64_l() != 0 {
            self.get_gpr64(1)
        } else {
            self.get_gpr32(1) as u64
        };
        if temp_rcx == 0 { self.branch_near64(instr)?; }
        Ok(())
    }

}
