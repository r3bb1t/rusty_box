//! 64-bit control transfer instructions for x86 CPU emulation
//!
//! Based on Bochs ctrl_xfer64.cc

use super::{
    cpu::{BxCpuC, Exception},
    decoder::{BxSegregs, Instruction},
    error::{CpuError, Result},
};

impl<T: crate::cpu::instrumentation::Instrumentation> crate::cpu::exec_ctx::ExecCtx<'_, T> {
    // =========================================================================
    // Helper functions for branching
    // =========================================================================

    /// Branch to a near 64-bit address
    /// Matching C++ ctrl_xfer64.cc branch_near64
    pub(super) fn branch_near64(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.rip().wrapping_add(instr.id() as i32 as u64);

        // Bochs ctrl_xfer64.cc branch_near64: #GP(0) on a non-canonical target.
        if !self.is_canonical(new_rip) {
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(new_rip);

        // Bochs ctrl_xfer64.cc branch_near64: without handler chaining
        // (BX_SUPPORT_HANDLERS_CHAINING_SPEEDUPS == 0) the trace stops here.
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
    /// Matching C++ ctrl_xfer64.cc CALL_Jq
    pub fn call_jq(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.rip().wrapping_add(instr.id() as i32 as u64);

        // Bochs ctrl_xfer64.cc CALL_Jq: RSP_SPECULATIVE, then the push before
        // the canonical check, so a #GP puts RSP and SSP back.
        self.rsp_speculative();
        self.push_64(self.rip())?;
        // Bochs ctrl_xfer64.cc CALL_Jq \u2014 shadow stack push only when displacement is non-zero.
        let cpl = self.cs_rpl();
        if instr.id() != 0 && self.shadow_stack_enabled(cpl) {
            let rip = self.rip();
            self.shadow_stack_push_64(rip)?;
        }

        if !self.is_canonical(new_rip) {
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(new_rip);
        self.rsp_commit();
        self.on_ucnear_branch(super::instrumentation::BranchType::Call, new_rip);
        Ok(())
    }

    /// Near call indirect (64-bit register)
    /// Matching C++ ctrl_xfer64.cc CALL_EqR
    pub fn call_eq_r(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.get_gpr64(instr.dst() as usize);

        // Bochs ctrl_xfer64.cc CALL_EqR: RSP_SPECULATIVE, then the push before
        // the canonical check, so a #GP puts RSP and SSP back.
        self.rsp_speculative();
        self.push_64(self.rip())?;
        let cpl = self.cs_rpl();
        if self.shadow_stack_enabled(cpl) {
            let rip = self.rip();
            self.shadow_stack_push_64(rip)?;
        }

        if !self.is_canonical(new_rip) {
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(new_rip);
        self.rsp_commit();
        self.track_indirect_if_not_suppressed(instr.seg_override_cet(), cpl);
        self.on_ucnear_branch(super::instrumentation::BranchType::CallIndirect, new_rip);
        Ok(())
    }

    /// Far call indirect (64-bit)
    /// Matching C++ ctrl_xfer64.cc CALL64_Ep
    pub fn call64_ep(&mut self, instr: &Instruction) -> Result<()> {
        // Bochs ctrl_xfer64.cc CALL64_Ep: invalidate_prefetch_q.
        self.eip_fetch_window = None;
        self.eip_page_window_size = 0;

        // Resolve effective address
        let eaddr = self.resolve_addr64(instr);

        // Bochs ctrl_xfer64.cc CALL64_Ep: read the offset, then the selector.
        // pointer, segment address pair
        let seg = BxSegregs::from(instr.seg());
        let op1_64 = self.read_virtual_qword_64(seg, eaddr)?;
        let asize_mask = if instr.as64_l() != 0 {
            0xFFFFFFFFFFFFFFFFu64
        } else {
            0xFFFFFFFF
        };
        let cs_raw = self.read_virtual_word_64(seg, (eaddr.wrapping_add(8)) & asize_mask)?;

        // BX_ASSERT(protected_mode()) — in 64-bit mode we are always in protected mode

        // Bochs ctrl_xfer64.cc CALL64_Ep: RSP_SPECULATIVE around call_protected.
        self.rsp_speculative();
        self.call_protected_64(instr, cs_raw, op1_64)?;
        self.rsp_commit();

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // JMP instructions (64-bit)
    // =========================================================================

    /// Near jump with 64-bit displacement
    /// Matching C++ ctrl_xfer64.cc JMP_Jq
    pub fn jmp_jq(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.rip().wrapping_add(instr.id() as i32 as u64);

        if !self.is_canonical(new_rip) {
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(new_rip);

        // BX_LINK_TRACE(i) — without handler chaining, equivalent to BX_NEXT_TRACE (STOP_TRACE)
        // Matching C++ ctrl_xfer64.cc
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        self.on_ucnear_branch(super::instrumentation::BranchType::Jmp, new_rip);
        Ok(())
    }

    /// Near jump indirect (64-bit register)
    /// Matching C++ ctrl_xfer64.cc JMP_EqR
    pub fn jmp_eq_r(&mut self, instr: &Instruction) -> Result<()> {
        let new_rip = self.get_gpr64(instr.dst() as usize);

        if !self.is_canonical(new_rip) {
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(new_rip);

        // BX_NEXT_TRACE(i) \u2014 matching C++ ctrl_xfer64.cc
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        self.track_indirect_if_not_suppressed(instr.seg_override_cet(), self.cs_rpl());
        self.on_ucnear_branch(super::instrumentation::BranchType::JmpIndirect, new_rip);
        Ok(())
    }

    /// Far jump indirect (64-bit)
    /// Matching C++ ctrl_xfer64.cc JMP64_Ep
    pub fn jmp64_ep(&mut self, instr: &Instruction) -> Result<()> {
        // Bochs ctrl_xfer64.cc JMP64_Ep: invalidate_prefetch_q.
        self.eip_fetch_window = None;
        self.eip_page_window_size = 0;

        // Resolve effective address
        let eaddr = self.resolve_addr64(instr);

        // Bochs ctrl_xfer64.cc JMP64_Ep: read the offset, then the selector.
        let seg = BxSegregs::from(instr.seg());
        let op1_64 = self.read_virtual_qword_64(seg, eaddr)?;
        let asize_mask = if instr.as64_l() != 0 {
            0xFFFFFFFFFFFFFFFFu64
        } else {
            0xFFFFFFFF
        };
        let cs_raw = self.read_virtual_word_64(seg, (eaddr.wrapping_add(8)) & asize_mask)?;

        // Bochs ctrl_xfer64.cc JMP64_Ep: BX_ASSERT(protected_mode()) — 64-bit
        // mode is always protected mode — then jump_protected.
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

        // Bochs LOAD_Eq then CALL_EqR: the target is read first, then
        // RSP_SPECULATIVE and the push before the canonical check.
        self.rsp_speculative();
        self.push_64(self.rip())?;
        let cpl = self.cs_rpl();
        if self.shadow_stack_enabled(cpl) {
            let rip = self.rip();
            self.shadow_stack_push_64(rip)?;
        }

        if !self.is_canonical(new_rip) {
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(new_rip);
        self.rsp_commit();
        self.track_indirect_if_not_suppressed(instr.seg_override_cet(), cpl);
        self.on_ucnear_branch(super::instrumentation::BranchType::CallIndirect, new_rip);
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

        // BX_NEXT_TRACE(i) \u2014 matching JMP_EqR pattern (C++ ctrl_xfer64.cc)
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        self.track_indirect_if_not_suppressed(instr.seg_override_cet(), self.cs_rpl());
        self.on_ucnear_branch(super::instrumentation::BranchType::JmpIndirect, new_rip);
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
    /// Matching C++ ctrl_xfer64.cc RETnear64_Iw
    pub fn retnear64_iw(&mut self, instr: &Instruction) -> Result<()> {
        // Bochs ctrl_xfer64.cc RETnear64_Iw: the pop is speculative, so a #CP
        // or #GP leaves the stack where it was.
        self.rsp_speculative();
        let return_rip = self.pop_64()?;
        let cpl = self.cs_rpl();
        if self.shadow_stack_enabled(cpl) {
            let shadow_rip = self.shadow_stack_pop_64()?;
            if shadow_rip != return_rip {
                return self.exception(
                    Exception::Cp,
                    super::cpu::CpExceptionErrorCode::NearRet as u16,
                );
            }
        }

        if !self.is_canonical(return_rip) {
            self.exception(Exception::Gp, 0)?;
            return Err(CpuError::CpuLoopRestart);
        }

        self.set_rip(return_rip);
        let rsp = self.rsp().wrapping_add(instr.iw() as u64);
        self.set_rsp(rsp);
        self.rsp_commit();
        self.on_ucnear_branch(super::instrumentation::BranchType::Ret, return_rip);
        Ok(())
    }

    /// Far return with immediate (64-bit)
    /// Matching C++ ctrl_xfer64.cc RETfar64_Iw
    /// Note: return_protected is RSP safe
    pub fn retfar64_iw(&mut self, instr: &Instruction) -> Result<()> {
        // Bochs ctrl_xfer64.cc RETfar64_Iw: invalidate_prefetch_q.
        self.eip_fetch_window = None;
        self.eip_page_window_size = 0;

        // BX_ASSERT(protected_mode()) — in 64-bit mode we are always in protected mode

        // Bochs ctrl_xfer64.cc RETfar64_Iw: RSP_SPECULATIVE around return_protected.
        self.rsp_speculative();
        self.return_protected_64(instr, instr.iw())?;
        self.rsp_commit();

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// Interrupt return (64-bit)
    /// Matching C++ ctrl_xfer64.cc IRET64
    pub fn iret64(&mut self, instr: &Instruction) -> Result<()> {
        // Bochs svm.cc SVM_INTERCEPT0_IRET.
        if self.in_svm_guest && self.svm_intercept_check(super::svm::SVM_INTERCEPT0_IRET) {
            return self.svm_vmexit(super::svm::SvmVmexit::Iret as i32, 0, 0);
        }
        // Invalidate prefetch queue
        self.eip_fetch_window = None;
        self.eip_page_window_size = 0;

        // VMX: nmi_unblocking_iret = true (Bochs ctrl_xfer64.cc IRET64)
        // (We don't have VMX guest mode, but set for completeness)

        // Bochs ctrl_xfer64.cc IRET64: unmask_event(BX_EVENT_NMI).
        self.unmask_event(BxCpuC::<T>::BX_EVENT_NMI);

        // Bochs ctrl_xfer64.cc IRET64: BX_ASSERT(long_mode()).

        // Bochs ctrl_xfer64.cc IRET64: RSP_SPECULATIVE around long_iret.
        self.rsp_speculative();
        self.long_iret(instr)?;
        self.rsp_commit();

        // VMX: nmi_unblocking_iret = false, after RSP_COMMIT (Bochs
        // ctrl_xfer64.cc IRET64).
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
        self.conditional_branch64(self.get_of(), instr)
    }

    /// Jump if not overflow (OF=0)
    pub fn jno_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(!self.get_of(), instr)
    }

    /// Jump if below/carry (CF=1)
    pub fn jb_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(self.get_cf(), instr)
    }

    /// Jump if not below/no carry (CF=0)
    pub fn jnb_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(!self.get_cf(), instr)
    }

    /// Jump if zero/equal (ZF=1)
    pub fn jz_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(self.get_zf(), instr)
    }

    /// Jump if not zero/not equal (ZF=0)
    pub fn jnz_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(!self.get_zf(), instr)
    }

    /// Jump if below or equal (CF=1 or ZF=1)
    pub fn jbe_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(self.get_cf() || self.get_zf(), instr)
    }

    /// Jump if not below or equal/above (CF=0 and ZF=0)
    pub fn jnbe_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(!self.get_cf() && !self.get_zf(), instr)
    }

    /// Jump if sign (SF=1)
    pub fn js_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(self.get_sf(), instr)
    }

    /// Jump if not sign (SF=0)
    pub fn jns_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(!self.get_sf(), instr)
    }

    /// Jump if parity/parity even (PF=1)
    pub fn jp_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(self.get_pf(), instr)
    }

    /// Jump if no parity/parity odd (PF=0)
    pub fn jnp_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(!self.get_pf(), instr)
    }

    /// Jump if less (SF != OF)
    pub fn jl_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(self.get_sf() != self.get_of(), instr)
    }

    /// Jump if not less/greater or equal (SF == OF)
    pub fn jnl_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(self.get_sf() == self.get_of(), instr)
    }

    /// Jump if less or equal (ZF=1 or SF!=OF)
    pub fn jle_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(self.get_zf() || (self.get_sf() != self.get_of()), instr)
    }

    /// Jump if not less or equal/greater (ZF=0 and SF==OF)
    pub fn jnle_jq(&mut self, instr: &Instruction) -> Result<()> {
        self.conditional_branch64(!self.get_zf() && (self.get_sf() == self.get_of()), instr)
    }

    // =========================================================================
    // LOOP instructions (64-bit mode)
    // =========================================================================

    /// Decrement RCX, jump if not zero (64-bit mode)
    /// Matching C++ ctrl_xfer64.cc LOOP64_Jb
    /// Note: There is some weirdness in LOOP instructions definition. If an exception
    /// was generated during the instruction execution (for example #GP fault
    /// because EIP was beyond CS segment limits) CPU state should restore the
    /// state prior to instruction execution.
    /// The final point that we are not allowed to decrement RCX register before
    /// it is known that no exceptions can happen.
    pub fn loop64_jb(&mut self, instr: &Instruction) -> Result<()> {
        if instr.as64_l() != 0 {
            let count = self.get_gpr64(1).wrapping_sub(1);
            self.conditional_branch64(count != 0, instr)?;
            self.set_gpr64(1, count);
        } else {
            let count = self.get_gpr32(1).wrapping_sub(1);
            self.conditional_branch64(count != 0, instr)?;
            self.set_gpr32(1, count);
        }
        Ok(())
    }

    /// Decrement RCX, jump if not zero and ZF=1 (64-bit mode)
    /// Matching C++ ctrl_xfer64.cc LOOPE64_Jb
    pub fn loope64_jb(&mut self, instr: &Instruction) -> Result<()> {
        if instr.as64_l() != 0 {
            let count = self.get_gpr64(1).wrapping_sub(1);
            self.conditional_branch64(count != 0 && self.get_zf(), instr)?;
            self.set_gpr64(1, count);
        } else {
            let count = self.get_gpr32(1).wrapping_sub(1);
            self.conditional_branch64(count != 0 && self.get_zf(), instr)?;
            self.set_gpr32(1, count);
        }
        Ok(())
    }

    /// Decrement RCX, jump if not zero and ZF=0 (64-bit mode)
    /// Matching C++ ctrl_xfer64.cc LOOPNE64_Jb
    pub fn loopne64_jb(&mut self, instr: &Instruction) -> Result<()> {
        if instr.as64_l() != 0 {
            let count = self.get_gpr64(1).wrapping_sub(1);
            self.conditional_branch64(count != 0 && !self.get_zf(), instr)?;
            self.set_gpr64(1, count);
        } else {
            let count = self.get_gpr32(1).wrapping_sub(1);
            self.conditional_branch64(count != 0 && !self.get_zf(), instr)?;
            self.set_gpr32(1, count);
        }
        Ok(())
    }

    /// Jump if RCX is zero (64-bit)
    /// Matching C++ ctrl_xfer64.cc JRCXZ_Jb
    pub fn jrcxz_jb(&mut self, instr: &Instruction) -> Result<()> {
        let temp_rcx = if instr.as64_l() != 0 {
            self.get_gpr64(1)
        } else {
            self.get_gpr32(1) as u64
        };
        self.conditional_branch64(temp_rcx == 0, instr)?;
        Ok(())
    }
}
