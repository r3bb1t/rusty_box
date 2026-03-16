//! 32-bit control transfer instructions for x86 CPU emulation
//!
//! Based on Bochs ctrl_xfer32.cc

use super::{
    cpu::{BxCpuC, Exception},
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
    error::{CpuError, Result},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Helper functions for branching
    // =========================================================================

    /// Branch to a near 32-bit address
    /// Matching C++ ctrl_xfer32.cc:29-46 branch_near32
    pub(super) fn branch_near32(&mut self, new_eip: u32) -> Result<()> {
        // DIAG: detect 32-bit branch in 64-bit mode (would corrupt upper RIP bits)
        if self.long64_mode() && self.icount > 1_500_000_000 {
            static B32_IN_64: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
            let c = B32_IN_64.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if c < 5 {
                eprintln!("[BRANCH32-IN-64] new_eip={:#x} prev_rip={:#x} RIP={:#x} icount={}",
                    new_eip, self.prev_rip, self.rip(), self.icount);
            }
        }
        // Check CS limit (matching C++ line 33-37)
        // Original: Bochs cpu/ctrl_xfer32.cc:33-37
        let limit = self.get_segment_limit(BxSegregs::Cs);
        if new_eip > limit {
            tracing::error!(
                "branch_near32: offset {:#010x} outside of CS limits {:#010x}",
                new_eip,
                limit
            );
            // Original: Bochs calls exception(BX_GP_EXCEPTION, 0) which doesn't return
            return Err(CpuError::BadVector {
                vector: Exception::Gp,
                error_code: 0,
            });
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
        self.eflags.contains(EFlags::CF)
    }

    /// Get Zero Flag
    pub fn get_zf(&self) -> bool {
        self.eflags.contains(EFlags::ZF)
    }

    /// Get Sign Flag
    pub fn get_sf(&self) -> bool {
        self.eflags.contains(EFlags::SF)
    }

    /// Get Overflow Flag
    pub fn get_of(&self) -> bool {
        self.eflags.contains(EFlags::OF)
    }

    /// Get Parity Flag
    pub fn get_pf(&self) -> bool {
        self.eflags.contains(EFlags::PF)
    }

    /// Get Auxiliary Flag (not directly used in conditionals, but useful)
    pub fn get_af(&self) -> bool {
        self.eflags.contains(EFlags::AF)
    }

    // =========================================================================
    // JMP instructions
    // =========================================================================

    /// JMP rel32 - Near jump with 32-bit signed displacement
    pub fn jmp_jd(&mut self, instr: &Instruction) -> Result<()> {
        let disp = instr.id() as i32;
        let eip = self.eip();
        let new_eip = (eip as i32).wrapping_add(disp) as u32;
        self.branch_near32(new_eip)?;
        Ok(())
    }

    /// JMP r32 - Indirect jump through register (register form)
    /// Matching Bochs ctrl_xfer32.cc JMP_EdR
    pub fn jmp_ed_r(&mut self, instr: &Instruction) -> Result<()> {
        let dst = instr.dst() as usize;
        let new_eip = self.get_gpr32(dst);
        if new_eip > 0x1000_0000 {
            tracing::debug!("JMP r32: EIP={:#010x} from RIP={:#010x} reg={}", new_eip, self.prev_rip, dst);
        }
        self.branch_near32(new_eip)?;
        Ok(())
    }

    /// JMP m32 - Indirect jump through memory (memory form)
    /// Matching Bochs ctrl_xfer32.cc JMP_EdM
    pub fn jmp_ed_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let new_eip = self.v_read_dword(seg, eaddr)?;
        if new_eip > 0x1000_0000 {
            tracing::debug!("JMP m32: [{:?}:{:#010x}] -> EIP={:#010x} from RIP={:#010x}", seg, eaddr, new_eip, self.prev_rip);
        }
        self.branch_near32(new_eip)?;
        Ok(())
    }

    /// JMP r/m32 - Unified dispatch (checks mod_c0)
    pub fn jmp_ed(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.jmp_ed_r(instr)
        } else {
            self.jmp_ed_m(instr)
        }
    }

    // =========================================================================
    // CALL instructions
    // =========================================================================

    /// CALL rel32 - Near call with 32-bit displacement
    pub fn call_jd(&mut self, instr: &Instruction) -> Result<()> {
        let disp = instr.id() as i32;
        let eip = self.eip();

        // Push return address
        self.push_32(eip)?;

        let new_eip = (eip as i32).wrapping_add(disp) as u32;

        self.branch_near32(new_eip)?;
        Ok(())
    }

    /// CALL r32 - Indirect call through register (register form)
    /// Matching Bochs ctrl_xfer32.cc CALL_EdR
    pub fn call_ed_r(&mut self, instr: &Instruction) -> Result<()> {
        let dst = instr.dst() as usize;
        let new_eip = self.get_gpr32(dst);
        let eip = self.eip();

        self.push_32(eip)?;
        self.branch_near32(new_eip)?;
        Ok(())
    }

    /// CALL m32 - Indirect call through memory (memory form)
    /// Matching Bochs ctrl_xfer32.cc CALL_EdM
    pub fn call_ed_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let new_eip = self.v_read_dword(seg, eaddr)?;
        let eip = self.eip();

        if new_eip > 0x1000_0000 {
            tracing::debug!("CALL m32: [{:?}:{:#010x}] -> EIP={:#010x} from RIP={:#010x}", seg, eaddr, new_eip, self.prev_rip);
        }
        self.push_32(eip)?;
        self.branch_near32(new_eip)?;
        Ok(())
    }

    /// CALL r/m32 - Unified dispatch (checks mod_c0)
    pub fn call_ed(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.call_ed_r(instr)
        } else {
            self.call_ed_m(instr)
        }
    }

    // =========================================================================
    // RET instructions
    // =========================================================================

    /// RET near - Return from procedure (32-bit)
    pub fn ret_near32(&mut self, _instr: &Instruction) -> Result<()> {
        let return_eip = self.pop_32()?;
        if return_eip > 0x1000_0000 {
            tracing::debug!("RET32: return_eip={:#010x} from RIP={:#010x} ESP={:#010x}", return_eip, self.prev_rip, self.get_gpr32(4));
        }
        self.branch_near32(return_eip)?;
        Ok(())
    }

    /// RET near imm16 - Return and pop imm16 bytes (32-bit)
    pub fn ret_near32_iw(&mut self, instr: &Instruction) -> Result<()> {
        let return_eip = self.pop_32()?;
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
        Ok(())
    }

    // =========================================================================
    // Conditional JMP instructions (32-bit displacement, Jd variants)
    // =========================================================================

    /// JO rel32 - Jump if overflow (OF=1)
    pub fn jo_jd(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_of() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JNO rel32 - Jump if not overflow (OF=0)
    pub fn jno_jd(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_of() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JB/JC/JNAE rel32 - Jump if below/carry (CF=1)
    pub fn jb_jd(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_cf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JNB/JNC/JAE rel32 - Jump if not below/no carry (CF=0)
    pub fn jnb_jd(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_cf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JZ/JE rel32 - Jump if zero/equal (ZF=1)
    pub fn jz_jd(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_zf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JNZ/JNE rel32 - Jump if not zero/not equal (ZF=0)
    pub fn jnz_jd(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_zf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JBE/JNA rel32 - Jump if below or equal (CF=1 or ZF=1)
    pub fn jbe_jd(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_cf() || self.get_zf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JNBE/JA rel32 - Jump if not below or equal/above (CF=0 and ZF=0)
    pub fn jnbe_jd(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_cf() && !self.get_zf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JS rel32 - Jump if sign (SF=1)
    pub fn js_jd(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_sf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JNS rel32 - Jump if not sign (SF=0)
    pub fn jns_jd(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_sf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JP/JPE rel32 - Jump if parity/parity even (PF=1)
    pub fn jp_jd(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_pf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JNP/JPO rel32 - Jump if no parity/parity odd (PF=0)
    pub fn jnp_jd(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_pf() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JL/JNGE rel32 - Jump if less (SF != OF)
    pub fn jl_jd(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_sf() != self.get_of() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JNL/JGE rel32 - Jump if not less/greater or equal (SF == OF)
    pub fn jnl_jd(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_sf() == self.get_of() {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JLE/JNG rel32 - Jump if less or equal (ZF=1 or SF!=OF)
    pub fn jle_jd(&mut self, instr: &Instruction) -> Result<()> {
        if self.get_zf() || (self.get_sf() != self.get_of()) {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    /// JNLE/JG rel32 - Jump if not less or equal/greater (ZF=0 and SF==OF)
    pub fn jnle_jd(&mut self, instr: &Instruction) -> Result<()> {
        if !self.get_zf() && (self.get_sf() == self.get_of()) {
            let disp = instr.id() as i32;
            let eip = self.eip();
            let new_eip = (eip as i32).wrapping_add(disp) as u32;
            self.branch_near32(new_eip)?;
        }
        Ok(())
    }

    // =========================================================================
    // JmpfAp - Far jump absolute pointer (16/32-bit operand size dispatch)
    // =========================================================================

    /// JmpfAp - Far jump with absolute pointer, dispatches 16/32-bit based on os32_l
    pub fn jmpf_ap(&mut self, instr: &Instruction) -> Result<()> {
        let segment = instr.iw2();
        if instr.os32_l() != 0 {
            let offset32 = instr.id();
            tracing::debug!("JmpfAp: FAR JMP 32-BIT to {:04x}:{:08x}", segment, offset32);
            self.jmp_far32(instr, segment, offset32)?;
        } else {
            let offset16 = instr.iw();
            tracing::debug!("JmpfAp: FAR JMP 16-BIT to {:04x}:{:04x}", segment, offset16);
            self.jmp_far16(instr, segment, offset16)?;
        }
        Ok(())
    }

    // =========================================================================
    // LOOP instructions (32-bit mode)
    // =========================================================================

    /// LOOP32 rel8 - Decrement ECX, jump if not zero (32-bit mode)
    /// LOOP32 rel8 — Bochs ctrl_xfer32.cc:733-748
    /// Decrements ECX or CX (based on as32L), jumps if nonzero.
    pub fn loop32_jb(&mut self, instr: &Instruction) -> Result<()> {
        if instr.as32_l() != 0 {
            let count = self.ecx().wrapping_sub(1);
            self.set_ecx(count);
            if count != 0 {
                let new_eip = (self.eip() as i32).wrapping_add(instr.ib() as i8 as i32) as u32;
                self.branch_near32(new_eip)?;
            }
        } else {
            let count = self.cx().wrapping_sub(1);
            self.set_cx(count);
            if count != 0 {
                let new_eip = (self.eip() as i32).wrapping_add(instr.ib() as i8 as i32) as u32;
                self.branch_near32(new_eip)?;
            }
        }
        Ok(())
    }

    /// LOOPE32/LOOPZ32 rel8 — Bochs ctrl_xfer32.cc:750-766
    pub fn loope32_jb(&mut self, instr: &Instruction) -> Result<()> {
        if instr.as32_l() != 0 {
            let count = self.ecx().wrapping_sub(1);
            self.set_ecx(count);
            if count != 0 && self.get_zf() {
                let new_eip = (self.eip() as i32).wrapping_add(instr.ib() as i8 as i32) as u32;
                self.branch_near32(new_eip)?;
            }
        } else {
            let count = self.cx().wrapping_sub(1);
            self.set_cx(count);
            if count != 0 && self.get_zf() {
                let new_eip = (self.eip() as i32).wrapping_add(instr.ib() as i8 as i32) as u32;
                self.branch_near32(new_eip)?;
            }
        }
        Ok(())
    }

    /// LOOPNE32/LOOPNZ32 rel8 — Bochs ctrl_xfer32.cc:768-784
    pub fn loopne32_jb(&mut self, instr: &Instruction) -> Result<()> {
        if instr.as32_l() != 0 {
            let count = self.ecx().wrapping_sub(1);
            self.set_ecx(count);
            if count != 0 && !self.get_zf() {
                let new_eip = (self.eip() as i32).wrapping_add(instr.ib() as i8 as i32) as u32;
                self.branch_near32(new_eip)?;
            }
        } else {
            let count = self.cx().wrapping_sub(1);
            self.set_cx(count);
            if count != 0 && !self.get_zf() {
                let new_eip = (self.eip() as i32).wrapping_add(instr.ib() as i8 as i32) as u32;
                self.branch_near32(new_eip)?;
            }
        }
        Ok(())
    }

    // =========================================================================
    // Helper function for loading segment register in real mode
    // =========================================================================

    /// Load segment register in real mode
    /// Based on Bochs segment_ctrl_pro.cc:163-209 load_seg_reg() real/v8086 path
    pub(super) fn load_seg_reg_real_mode(&mut self, seg: BxSegregs, selector: u16) {
        let seg_idx = seg as usize;

        self.sregs[seg_idx].selector.value = selector;
        // Bochs: RPL = 0 in real mode, 3 in v8086
        self.sregs[seg_idx].selector.rpl = if self.real_mode() { 0 } else { 3 };
        self.sregs[seg_idx].cache.valid = super::descriptor::SEG_VALID_CACHE;
        unsafe {
            self.sregs[seg_idx].cache.u.segment.base = (selector as u64) << 4;
        }
        self.sregs[seg_idx].cache.segment = true;
        self.sregs[seg_idx].cache.p = true;

        // Bochs: "Do not modify segment limit and AR bytes when in real mode"
        // "Support for big real mode"
        // Only set these fields in v8086 mode
        if !self.real_mode() {
            // v8086 mode — Bochs segment_ctrl_pro.cc:194-209
            self.sregs[seg_idx].cache.r#type = 3; // DATA_READ_WRITE_ACCESSED
            self.sregs[seg_idx].cache.dpl = 3;
            unsafe {
                self.sregs[seg_idx].cache.u.segment.limit_scaled = 0xFFFF;
                self.sregs[seg_idx].cache.u.segment.g = false;
                self.sregs[seg_idx].cache.u.segment.d_b = false;
                self.sregs[seg_idx].cache.u.segment.avl = false;
                self.sregs[seg_idx].cache.u.segment.l = false; // Bochs line 194
            }
        }

        if seg as usize == BxSegregs::Cs as usize {
            self.invalidate_prefetch_q();
            // Bochs: updateFetchModeMask() updates icache hash + user_pl
            self.update_fetch_mode_mask();
            self.handle_alignment_check();
        }

        if seg as usize == BxSegregs::Ss as usize {
            self.invalidate_stack_cache();
        }
    }

    // =========================================================================
    // Far jump/call helpers (32-bit)
    // =========================================================================

    /// Far jump 32-bit (matching C++ jmp_far32)
    /// Called by JMP32_Ap and JMP32_Ep
    pub(super) fn jmp_far32(
        &mut self,
        _instr: &Instruction,
        cs_raw: u16,
        disp32: u32,
    ) -> Result<()> {
        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        if self.protected_mode() {
            // Protected mode (includes long modes): use jump_protected
            self.jump_protected(cs_raw, disp32 as u64)?;
        } else {
            // Real mode or V8086 mode
            let limit = self.get_segment_limit(BxSegregs::Cs);
            if disp32 > limit {
                tracing::error!(
                    "jmp_far32: offset {:#010x} outside of CS limits {:#010x}",
                    disp32,
                    limit
                );
                return Err(CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: 0,
                });
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
    fn call_far32(&mut self, _instr: &Instruction, cs_raw: u16, disp32: u32) -> Result<()> {
        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        if self.protected_mode() {
            return self.call_protected(cs_raw, disp32, true);
        } else {
            // Real mode or V8086 mode
            let limit = self.get_segment_limit(BxSegregs::Cs);
            if disp32 > limit {
                tracing::error!(
                    "call_far32: offset {:#010x} outside of CS limits {:#010x}",
                    disp32,
                    limit
                );
                return Err(CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: 0,
                });
            }

            // Push return address (CS:EIP)
            let cs_value = self.sregs[BxSegregs::Cs as usize].selector.value;
            let eip = self.eip();
            self.push_32(cs_value as u32)?;
            self.push_32(eip)?;

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
    pub fn call32_ap(&mut self, instr: &Instruction) -> Result<()> {
        let cs_raw = instr.iw2();
        let disp32 = instr.id();
        self.call_far32(instr, cs_raw, disp32)
    }

    /// CALL32_Ep - Far call indirect (32-bit)
    /// Matching C++ ctrl_xfer32.cc (similar to CALL16_Ep but 32-bit)
    pub fn call32_ep(&mut self, instr: &Instruction) -> Result<()> {
        // Resolve effective address
        let eaddr = self.resolve_addr(instr);

        // Read offset and segment from memory
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.v_read_dword(seg, eaddr)?;
        let cs_raw = self.v_read_word(
            seg,
            (eaddr.wrapping_add(4))
                & (if instr.as32_l() == 0 {
                    0xFFFF
                } else {
                    0xFFFFFFFF
                }),
        )?;

        self.call_far32(instr, cs_raw, op1_32)
    }

    // =========================================================================
    // Far JMP instructions (32-bit)
    // =========================================================================

    /// JMP32_Ap - Far jump with absolute pointer (32-bit)
    /// Matching C++ ctrl_xfer32.cc (similar to CALL32_Ap but for jump)
    pub fn jmp32_ap(&mut self, instr: &Instruction) -> Result<()> {
        let cs_raw = instr.iw2();
        let disp32 = instr.id();
        self.jmp_far32(instr, cs_raw, disp32)
    }

    /// JMP32_Ep - Far jump indirect (32-bit)
    /// Matching C++ ctrl_xfer32.cc (similar to JMP16_Ep but 32-bit)
    pub fn jmp32_ep(&mut self, instr: &Instruction) -> Result<()> {
        // Resolve effective address
        let eaddr = self.resolve_addr(instr);

        // Read offset and segment from memory
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.v_read_dword(seg, eaddr)?;
        let cs_raw = self.v_read_word(
            seg,
            (eaddr.wrapping_add(4))
                & (if instr.as32_l() == 0 {
                    0xFFFF
                } else {
                    0xFFFFFFFF
                }),
        )?;

        self.jmp_far32(instr, cs_raw, op1_32)
    }

    // =========================================================================
    // Far RET instructions (32-bit)
    // =========================================================================

    /// RETfar32 - Far return without immediate (32-bit)
    /// Matching C++ ctrl_xfer32.cc (similar to RETfar16 but 32-bit)
    pub fn retfar32(&mut self, _instr: &Instruction) -> Result<()> {
        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        if self.protected_mode() {
            return self.return_protected(0, true);
        } else {
            // Real mode or V8086 mode - pop EIP and CS (32-bit pop, MSW discarded for CS)
            let eip = self.pop_32()?;
            let cs_raw = self.pop_32()? as u16; // 32-bit pop, MSW discarded

            // Check CS limit
            let limit = self.get_segment_limit(BxSegregs::Cs);
            if eip > limit {
                tracing::error!(
                    "retfar32: offset {:#010x} outside of CS limits {:#010x}",
                    eip,
                    limit
                );
                return Err(CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: 0,
                });
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
    pub fn retfar32_iw(&mut self, instr: &Instruction) -> Result<()> {
        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        let imm16 = instr.iw();

        if self.protected_mode() {
            return self.return_protected(imm16, true);
        } else {
            // Real mode or V8086 mode - pop EIP and CS (32-bit pop, MSW discarded for CS)
            let eip = self.pop_32()?;
            let cs_raw = self.pop_32()? as u16; // 32-bit pop, MSW discarded

            // Check CS limit
            let limit = self.get_segment_limit(BxSegregs::Cs);
            if eip > limit {
                tracing::error!(
                    "retfar32_iw: offset {:#010x} outside of CS limits {:#010x}",
                    eip,
                    limit
                );
                return Err(CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: 0,
                });
            }

            self.load_seg_reg_real_mode(BxSegregs::Cs, cs_raw);
            self.set_eip(eip);

            // Pop additional bytes from stack (unsigned addition)
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
