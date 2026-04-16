//! 16-bit control transfer instructions for x86 CPU emulation
//!
//! Based on Bochs ctrl_xfer16.cc

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

    /// Branch to a near 16-bit address
    /// Matching C++ ctrl_xfer16.cc branch_near16
    pub(super) fn branch_near16(&mut self, new_ip: u16) -> super::Result<()> { // Check CS limit (matching C++ line 32-36)
    // Bochs: exception(BX_GP_EXCEPTION, 0) which longjmps
    let limit = self.get_segment_limit(BxSegregs::Cs);
    if (new_ip as u32) > limit {
        tracing::error!(
            "branch_near16: offset {:#06x} outside of CS limits {:#010x}",
            new_ip,
            limit
        );
        return self.exception(super::cpu::Exception::Gp, 0);
    }
    
    // Matching C++ line 38: EIP = new_IP;
    self.set_eip(new_ip as u32);
    
    // Matching C++ lines 40-43: Set STOP_TRACE when handlers chaining is disabled
    self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
    Ok(()) }

    // =========================================================================
    // Flag getters for conditional jumps
    // =========================================================================

    // Get Carry Flag
    // Flag getters (get_cf, get_zf, get_sf, get_of, get_pf, get_af) are defined in ctrl_xfer32.rs
    // to avoid duplicate definitions across multiple impl blocks

    // =========================================================================
    // JMP instructions
    // =========================================================================

    /// JMP rel8 - Short jump with 8-bit signed displacement
    pub fn jmp_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8;
    let ip = self.get_ip();
    let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
    self.branch_near16(new_ip)?;
    self.on_ucnear_branch(super::instrumentation::BranchType::Jmp, self.rip()); Ok(()) }

    /// JMP rel16 - Near jump with 16-bit signed displacement
    pub fn jmp_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16;
    let ip = self.get_ip();
    let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
    self.branch_near16(new_ip)?;
    self.on_ucnear_branch(super::instrumentation::BranchType::Jmp, self.rip()); Ok(()) }

    /// JMP r16 - Indirect jump through register (register form)
    /// Matching Bochs ctrl_xfer16.cc JMP_EwR
    pub fn jmp_ew_r(&mut self, instr: &Instruction) -> super::Result<()> { let dst = instr.dst() as usize;
    let new_ip = self.get_gpr16(dst);
    self.branch_near16(new_ip)?;
    self.on_ucnear_branch(super::instrumentation::BranchType::JmpIndirect, self.rip()); Ok(()) }

    /// JMP m16 - Indirect jump through memory (memory form)
    /// Matching Bochs ctrl_xfer16.cc JMP_EwM
    pub fn jmp_ew_m(&mut self, instr: &Instruction) -> super::Result<()> { let eaddr = self.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let new_ip = self.v_read_word(seg, eaddr)?;
    self.branch_near16(new_ip)?;
    self.on_ucnear_branch(super::instrumentation::BranchType::JmpIndirect, self.rip()); Ok(()) }

    /// JMP r/m16 - Unified dispatch (checks mod_c0)
    pub fn jmp_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.jmp_ew_r(instr)
        } else {
            self.jmp_ew_m(instr)
        }
    }

    // =========================================================================
    // CALL instructions
    // =========================================================================

    /// CALL rel16 - Near call with 16-bit displacement
    pub fn call_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16;
    let ip = self.get_ip();
    
    // Push return address
    self.push_16(ip)?;
    
    let new_ip = (ip as i32).wrapping_add(disp as i32) as u16;
    self.branch_near16(new_ip)?;
    self.on_ucnear_branch(super::instrumentation::BranchType::Call, self.rip()); Ok(()) }

    /// CALL r16 - Indirect call through register (register form)
    /// Matching Bochs ctrl_xfer16.cc CALL_EwR
    pub fn call_ew_r(&mut self, instr: &Instruction) -> super::Result<()> { let dst = instr.dst() as usize;
    let new_ip = self.get_gpr16(dst);
    let ip = self.get_ip();
    
    self.push_16(ip)?;
    self.branch_near16(new_ip)?;
    self.on_ucnear_branch(super::instrumentation::BranchType::CallIndirect, self.rip()); Ok(()) }

    /// CALL m16 - Indirect call through memory (memory form)
    /// Matching Bochs ctrl_xfer16.cc CALL_EwM
    pub fn call_ew_m(&mut self, instr: &Instruction) -> super::Result<()> { let eaddr = self.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let new_ip = self.v_read_word(seg, eaddr)?;
    let ip = self.get_ip();
    
    self.push_16(ip)?;
    self.branch_near16(new_ip)?;
    self.on_ucnear_branch(super::instrumentation::BranchType::CallIndirect, self.rip()); Ok(()) }

    /// CALL r/m16 - Unified dispatch (checks mod_c0)
    pub fn call_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.call_ew_r(instr)
        } else {
            self.call_ew_m(instr)
        }
    }

    // =========================================================================
    // RET instructions
    // =========================================================================

    /// RET near - Return from procedure (16-bit)
    pub fn ret_near16(&mut self, _instr: &Instruction) -> super::Result<()> { let return_ip = self.pop_16()?;
    self.branch_near16(return_ip)?;
    self.on_ucnear_branch(super::instrumentation::BranchType::Ret, self.rip()); Ok(()) }

    /// RET near imm16 - Return and pop imm16 bytes (16-bit)
    pub fn ret_near16_iw(&mut self, instr: &Instruction) -> super::Result<()> { let return_ip = self.pop_16()?;
    let imm16 = instr.iw();
    
    self.branch_near16(return_ip)?;
    
    // Pop additional bytes from stack
    let ss_d_b = self.get_segment_d_b(BxSegregs::Ss);
    if ss_d_b {
        let esp = self.get_gpr32(4);
        self.set_gpr32(4, esp.wrapping_add(imm16 as u32));
    } else {
        let sp = self.get_gpr16(4);
        self.set_gpr16(4, sp.wrapping_add(imm16));
    }
    self.on_ucnear_branch(super::instrumentation::BranchType::Ret, self.rip()); Ok(()) }

    // =========================================================================
    // Conditional JMP instructions (16-bit)
    // =========================================================================

    /// JO rel8 - Jump if overflow (OF=1)
    pub fn jo_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_of(), new_ip) }

    /// JNO rel8 - Jump if not overflow (OF=0)
    pub fn jno_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_of(), new_ip) }

    /// JB/JC/JNAE rel8 - Jump if below/carry (CF=1)
    pub fn jb_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_cf(), new_ip) }

    /// JNB/JNC/JAE rel8 - Jump if not below/no carry (CF=0)
    pub fn jnb_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_cf(), new_ip) }

    /// JZ/JE rel8 - Jump if zero/equal (ZF=1)
    pub fn jz_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_zf(), new_ip) }

    /// JNZ/JNE rel8 - Jump if not zero/not equal (ZF=0)
    pub fn jnz_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_zf(), new_ip) }

    /// JBE/JNA rel8 - Jump if below or equal (CF=1 or ZF=1)
    pub fn jbe_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_cf() || self.get_zf(), new_ip) }

    /// JNBE/JA rel8 - Jump if not below or equal/above (CF=0 and ZF=0)
    pub fn jnbe_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_cf() && !self.get_zf(), new_ip) }

    /// JS rel8 - Jump if sign (SF=1)
    pub fn js_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_sf(), new_ip) }

    /// JNS rel8 - Jump if not sign (SF=0)
    pub fn jns_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_sf(), new_ip) }

    /// JP/JPE rel8 - Jump if parity/parity even (PF=1)
    pub fn jp_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_pf(), new_ip) }

    /// JNP/JPO rel8 - Jump if no parity/parity odd (PF=0)
    pub fn jnp_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_pf(), new_ip) }

    /// JL/JNGE rel8 - Jump if less (SF != OF)
    pub fn jl_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_sf() != self.get_of(), new_ip) }

    /// JNL/JGE rel8 - Jump if not less/greater or equal (SF == OF)
    pub fn jnl_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_sf() == self.get_of(), new_ip) }

    /// JLE/JNG rel8 - Jump if less or equal (ZF=1 or SF!=OF)
    pub fn jle_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_zf() || (self.get_sf() != self.get_of()), new_ip) }

    /// JNLE/JG rel8 - Jump if not less or equal/greater (ZF=0 and SF==OF)
    pub fn jnle_jb(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_zf() && (self.get_sf() == self.get_of()), new_ip) }

    // =========================================================================
    // Conditional JMP instructions (16-bit displacement)
    // =========================================================================

    /// JZ/JE rel16 - Jump if zero/equal (ZF=1)
    pub fn jz_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_zf(), new_ip) }

    /// JNZ/JNE rel16 - Jump if not zero/not equal (ZF=0)
    pub fn jnz_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_zf(), new_ip) }

    /// JO rel16 - Jump if overflow (OF=1)
    pub fn jo_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_of(), new_ip) }

    /// JNO rel16 - Jump if not overflow (OF=0)
    pub fn jno_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_of(), new_ip) }

    /// JB/JC/JNAE rel16 - Jump if below/carry (CF=1)
    pub fn jb_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_cf(), new_ip) }

    /// JNB/JNC/JAE rel16 - Jump if not below/no carry (CF=0)
    pub fn jnb_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_cf(), new_ip) }

    /// JBE/JNA rel16 - Jump if below or equal (CF=1 or ZF=1)
    pub fn jbe_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_cf() || self.get_zf(), new_ip) }

    /// JNBE/JA rel16 - Jump if not below or equal/above (CF=0 and ZF=0)
    pub fn jnbe_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_cf() && !self.get_zf(), new_ip) }

    /// JS rel16 - Jump if sign (SF=1)
    pub fn js_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_sf(), new_ip) }

    /// JNS rel16 - Jump if not sign (SF=0)
    pub fn jns_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_sf(), new_ip) }

    /// JP/JPE rel16 - Jump if parity/parity even (PF=1)
    pub fn jp_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_pf(), new_ip) }

    /// JNP/JPO rel16 - Jump if no parity/parity odd (PF=0)
    pub fn jnp_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_pf(), new_ip) }

    /// JL/JNGE rel16 - Jump if less (SF != OF)
    pub fn jl_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_sf() != self.get_of(), new_ip) }

    /// JNL/JGE rel16 - Jump if not less/greater or equal (SF == OF)
    pub fn jnl_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_sf() == self.get_of(), new_ip) }

    /// JLE/JNG rel16 - Jump if less or equal (ZF=1 or SF!=OF)
    pub fn jle_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(self.get_zf() || (self.get_sf() != self.get_of()), new_ip) }

    /// JNLE/JG rel16 - Jump if not less or equal/greater (ZF=0 and SF==OF)
    pub fn jnle_jw(&mut self, instr: &Instruction) -> super::Result<()> { let disp = instr.iw() as i16; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(!self.get_zf() && (self.get_sf() == self.get_of()), new_ip) }

    // =========================================================================
    // LOOP instructions
    // =========================================================================

    /// LOOP rel8 - Decrement CX/ECX, jump if not zero
    /// Bochs ctrl_xfer16.cc: counter must NOT be written back before
    /// branch_near16 is known to succeed (exception safety).
    pub fn loop16_jb(&mut self, instr: &Instruction) -> super::Result<()> {
        let as32l = instr.as32_l() != 0;

        if as32l {
            let count = self.get_gpr32(1).wrapping_sub(1);

            { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(count != 0, new_ip)?; }

            self.set_gpr32(1, count);
        } else {
            let count = self.get_gpr16(1).wrapping_sub(1);

            { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(count != 0, new_ip)?; }

            self.set_gpr16(1, count);
        }
        Ok(())
    }

    /// LOOPE/LOOPZ rel8 - Decrement CX/ECX, jump if not zero and ZF=1
    /// Counter written after branch attempt (exception safety, matching Bochs).
    pub fn loope16_jb(&mut self, instr: &Instruction) -> super::Result<()> {
        let as32l = instr.as32_l() != 0;

        if as32l {
            let count = self.get_gpr32(1).wrapping_sub(1);

            { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(count != 0 && self.get_zf(), new_ip)?; }

            self.set_gpr32(1, count);
        } else {
            let count = self.get_gpr16(1).wrapping_sub(1);

            { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(count != 0 && self.get_zf(), new_ip)?; }

            self.set_gpr16(1, count);
        }
        Ok(())
    }

    /// LOOPNE/LOOPNZ rel8 - Decrement CX/ECX, jump if not zero and ZF=0
    /// Counter written after branch attempt (exception safety, matching Bochs).
    pub fn loopne16_jb(&mut self, instr: &Instruction) -> super::Result<()> {
        let as32l = instr.as32_l() != 0;

        if as32l {
            let count = self.get_gpr32(1).wrapping_sub(1);

            { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(count != 0 && !self.get_zf(), new_ip)?; }

            self.set_gpr32(1, count);
        } else {
            let count = self.get_gpr16(1).wrapping_sub(1);

            { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(count != 0 && !self.get_zf(), new_ip)?; }

            self.set_gpr16(1, count);
        }
        Ok(())
    }

    /// JCXZ rel8 - Jump if CX is zero
    pub fn jcxz_jb(&mut self, instr: &Instruction) -> super::Result<()> {
        let as32l = instr.as32_l() != 0;
        let count = if as32l {
            self.get_gpr32(1)
        } else {
            self.get_gpr16(1) as u32
        };

        { let disp = instr.ib() as i8; let ip = self.get_ip(); let new_ip = (ip as i32).wrapping_add(disp as i32) as u16; self.conditional_branch16(count == 0, new_ip)?; }
        Ok(())
    }

    /// JECXZ rel8 - Jump if ECX is zero (32-bit operand-size form)
    /// Matching C++ ctrl_xfer32.cc JECXZ_Jb
    /// NOTE: counter is ECX (as32L check per Bochs), target is 32-bit EIP
    pub fn jecxz_jb(&mut self, instr: &Instruction) -> super::Result<()> {
        // Bochs: if (i->as32L()) use ECX else use CX
        let count = if instr.as32_l() != 0 {
            self.get_gpr32(1)
        } else {
            self.get_gpr16(1) as u32
        };

        let disp = instr.id() as i32; // sign-extended byte displacement
        let eip = self.eip();
        let new_eip = (eip as i32).wrapping_add(disp) as u32;
        self.conditional_branch32(count == 0, new_eip)?;
        Ok(())
    }

    // =========================================================================
    // Helper function for loading segment register in real mode
    // =========================================================================

    // Load segment register in real mode (matching load_seg_reg for real mode)
    // load_seg_reg_real_mode is defined in ctrl_xfer32.rs to avoid duplicate definitions

    // =========================================================================
    // Far jump/call helpers
    // =========================================================================

    /// Far jump 16-bit (matching C++ jmp_far16)
    /// Called by JMP16_Ap and JMP16_Ep
    pub(super) fn jmp_far16(
        &mut self,
        _instr: &Instruction,
        cs_raw: u16,
        disp16: u16,
    ) -> Result<()> {
        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        if self.protected_mode() {
            // Protected mode (includes long modes) - use jump_protected
            self.jump_protected(cs_raw, disp16 as u64)?;
        } else {
            // Real mode or V8086 mode
            let limit = self.get_segment_limit(BxSegregs::Cs);
            if (disp16 as u32) > limit {
                tracing::error!(
                    "jmp_far16: offset {:#06x} outside of CS limits {:#010x}",
                    disp16,
                    limit
                );
                return Err(CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: 0,
                });
            }

            self.load_seg_reg_real_mode(BxSegregs::Cs, cs_raw);
            self.set_eip(disp16 as u32);
        }

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// Far call 16-bit (matching C++ call_far16)
    /// Called by CALL16_Ap and CALL16_Ep
    fn call_far16(&mut self, _instr: &Instruction, cs_raw: u16, disp16: u16) -> Result<()> {
        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        if self.protected_mode() {
            return self.call_protected(cs_raw, disp16 as u32, false);
        } else {
            // Real mode or V8086 mode
            let limit = self.get_segment_limit(BxSegregs::Cs);
            if (disp16 as u32) > limit {
                tracing::error!(
                    "call_far16: offset {:#06x} outside of CS limits {:#010x}",
                    disp16,
                    limit
                );
                return Err(CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: 0,
                });
            }

            // Push return address (CS:IP)
            let cs_value = self.sregs[BxSegregs::Cs as usize].selector.value;
            let ip = self.get_ip();
            self.push_16(cs_value)?;
            self.push_16(ip)?;

            self.load_seg_reg_real_mode(BxSegregs::Cs, cs_raw);
            self.set_eip(disp16 as u32);
        }

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // Far CALL instructions (16-bit)
    // =========================================================================

    /// CALL16_Ap - Far call with absolute pointer (16-bit)
    /// Matching C++ ctrl_xfer16.cc
    pub fn call16_ap(&mut self, instr: &Instruction) -> Result<()> {
        let disp16 = instr.iw();
        let cs_raw = instr.iw2();
        self.call_far16(instr, cs_raw, disp16)
    }

    /// CALL16_Ep - Far call indirect (16-bit)
    /// Matching C++ ctrl_xfer16.cc
    pub fn call16_ep(&mut self, instr: &Instruction) -> Result<()> {
        // Resolve effective address
        let eaddr = self.resolve_addr(instr);

        // Read offset and segment from memory
        let seg = BxSegregs::from(instr.seg());
        let op1_16 = self.v_read_word(seg, eaddr)?;
        let asize_mask = if instr.as32_l() == 0 {
            0xFFFF
        } else {
            0xFFFFFFFF
        };
        let cs_raw = self.v_read_word(seg, (eaddr.wrapping_add(2)) & asize_mask)?;

        self.call_far16(instr, cs_raw, op1_16)
    }

    // =========================================================================
    // Far JMP instructions (16-bit)
    // =========================================================================

    /// JMP16_Ap - Far jump with absolute pointer (16-bit)
    /// Matching C++ ctrl_xfer16.cc (similar to CALL16_Ap but for jump)
    pub fn jmp16_ap(&mut self, instr: &Instruction) -> Result<()> {
        let disp16 = instr.iw();
        let cs_raw = instr.iw2();
        self.jmp_far16(instr, cs_raw, disp16)
    }

    /// JMP16_Ep - Far jump indirect (16-bit)
    /// Matching C++ ctrl_xfer16.cc
    pub fn jmp16_ep(&mut self, instr: &Instruction) -> Result<()> {
        // Resolve effective address
        let eaddr = self.resolve_addr(instr);

        // Read offset and segment from memory
        let seg = BxSegregs::from(instr.seg());
        let op1_16 = self.v_read_word(seg, eaddr)?;
        let asize_mask = if instr.as32_l() == 0 {
            0xFFFF
        } else {
            0xFFFFFFFF
        };
        let cs_raw = self.v_read_word(seg, (eaddr.wrapping_add(2)) & asize_mask)?;

        self.jmp_far16(instr, cs_raw, op1_16)
    }

    // =========================================================================
    // Far RET instructions (16-bit)
    // =========================================================================

    /// RETfar16 - Far return without immediate (16-bit)
    /// Matching C++ ctrl_xfer16.cc (similar to RETfar16_Iw but without imm16)
    pub fn retfar16(&mut self, _instr: &Instruction) -> Result<()> {
        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        if self.protected_mode() {
            return self.return_protected(0, false);
        } else {
            // Real mode or V8086 mode
            let ip = self.pop_16()?;
            let cs_raw = self.pop_16()?;

            // Check CS limit
            let limit = self.get_segment_limit(BxSegregs::Cs);
            if (ip as u32) > limit {
                tracing::error!(
                    "retfar16: offset {:#06x} outside of CS limits {:#010x}",
                    ip,
                    limit
                );
                return self.exception(Exception::Gp, 0);
            }


            self.load_seg_reg_real_mode(BxSegregs::Cs, cs_raw);
            self.set_eip(ip as u32);
        }

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// RETfar16_Iw - Far return with immediate (16-bit)
    /// Matching C++ ctrl_xfer16.cc
    pub fn retfar16_iw(&mut self, instr: &Instruction) -> Result<()> {
        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        let imm16 = instr.iw();

        if self.protected_mode() {
            return self.return_protected(imm16, false);
        } else {
            // Real mode or V8086 mode
            let ip = self.pop_16()?;
            let cs_raw = self.pop_16()?;

            // Check CS limit
            let limit = self.get_segment_limit(BxSegregs::Cs);
            if (ip as u32) > limit {
                tracing::error!(
                    "retfar16_iw: offset {:#06x} outside of CS limits {:#010x}",
                    ip,
                    limit
                );
                return Err(CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: 0,
                });
            }

            self.load_seg_reg_real_mode(BxSegregs::Cs, cs_raw);
            self.set_eip(ip as u32);

            // Pop additional bytes from stack
            let ss_d_b = self.get_segment_d_b(BxSegregs::Ss);
            if ss_d_b {
                let esp = self.get_gpr32(4);
                self.set_gpr32(4, esp.wrapping_add(imm16 as u32));
            } else {
                let sp = self.get_gpr16(4);
                self.set_gpr16(4, sp.wrapping_add(imm16));
            }
        }

        // Set STOP_TRACE to break trace loop
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // Helper functions for memory access
    // =========================================================================

    // Helper methods (resolve_addr32, read_virtual_word) are defined in logical16.rs to avoid duplicate definitions
}
