//! 64-bit Stack operations for x86 CPU emulation
//!
//! Based on Bochs stack64.cc
//! Copyright (C) 2001-2018 The Bochs Project

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // 64-bit PUSH/POP primitives
    // Based on Bochs stack64.cc
    // =========================================================================

    /// Push a 64-bit value onto the stack
    /// Based on BX_CPU_C::push_64 in stack.h (64-bit mode)
    pub fn push_64(&mut self, value: u64) -> super::Result<()> {
        let rsp = self.rsp();
        let new_rsp = rsp.wrapping_sub(8);
        self.stack_write_qword_64(new_rsp, value)?;
        self.set_rsp(new_rsp);
        Ok(())
    }

    /// Pop a 64-bit value from the stack
    /// Based on BX_CPU_C::pop_64 in stack.h (64-bit mode)
    pub fn pop_64(&mut self) -> super::Result<u64> {
        let rsp = self.rsp();
        let value = self.stack_read_qword_64(rsp)?;
        self.set_rsp(rsp.wrapping_add(8));
        Ok(value)
    }

    // =========================================================================
    // 64-bit PUSH instructions
    // Based on Bochs stack64.cc
    // =========================================================================

    /// PUSH r64 - Push 64-bit register
    /// Based on Bochs stack64.cc PUSH_EqR
    pub fn push_eq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let dst = instr.dst() as usize;
        let value = self.get_gpr64(dst);
        self.push_64(value)?;
        Ok(())
    }

    /// PUSH imm64 (sign-extended from 32-bit)
    /// Based on Bochs stack64.cc PUSH_Iq
    pub fn push_iq(&mut self, instr: &Instruction) -> super::Result<()> {
        let value = instr.id() as i32 as i64 as u64;
        self.push_64(value)?;
        Ok(())
    }

    // =========================================================================
    // 64-bit POP instructions
    // Based on Bochs stack64.cc
    // =========================================================================

    /// POP r64 - Pop into 64-bit register
    /// Based on Bochs stack64.cc POP_EqR
    pub fn pop_eq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let dst = instr.dst() as usize;
        let value = self.pop_64()?;
        self.set_gpr64(dst, value);
        Ok(())
    }

    /// PUSH m64 - Push 64-bit value from memory
    /// Based on Bochs stack64.cc PUSH_EqM
    pub fn push_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = crate::cpu::decoder::BxSegregs::from(instr.seg());
        let val64 = self.read_virtual_qword_64(seg, eaddr)?;
        self.push_64(val64)?;
        Ok(())
    }

    /// POP m64 - Pop into 64-bit memory location
    /// Based on Bochs stack64.cc POP_EqM
    pub fn pop_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let val64 = self.pop_64()?;
        let eaddr = self.resolve_addr64(instr);
        let seg = crate::cpu::decoder::BxSegregs::from(instr.seg());
        self.write_virtual_qword_64(seg, eaddr, val64)?;
        Ok(())
    }

    /// PUSH r/m64 - Unified dispatch based on mod_c0()
    pub fn push_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.push_eq_r(instr)
        } else {
            self.push_eq_m(instr)
        }
    }

    /// POP r/m64 - Unified dispatch based on mod_c0()
    pub fn pop_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.pop_eq_r(instr)
        } else {
            self.pop_eq_m(instr)
        }
    }

    // =========================================================================
    // PUSHFQ/POPFQ instructions (64-bit)
    // Based on Bochs flag_ctrl.cc
    // =========================================================================

    /// PUSHFQ - Push flags (64-bit)
    pub fn pushf_fq(&mut self, _instr: &Instruction) -> super::Result<()> {
        // VM & RF flags cleared in image stored on the stack
        let flags = (self.eflags.bits() & 0x00FCFFFF) as u64;
        self.push_64(flags)?;
        Ok(())
    }

    /// POPFQ - Pop flags (64-bit)
    /// Based on Bochs flag_ctrl.cc POPF_Fq (lines 357-385)
    pub fn popf_fq(&mut self, _instr: &Instruction) -> super::Result<()> {
        // Base changeMask: OSZAPC + TF + DF + NT + RF + AC + ID
        let mut change_mask = EFlags::OSZAPC
            .union(EFlags::TF)
            .union(EFlags::DF)
            .union(EFlags::NT)
            .union(EFlags::RF)
            .union(EFlags::AC)
            .union(EFlags::ID);

        // RF is always zero after POPF
        let eflags32 = (self.pop_64()? as u32) & !EFlags::RF.bits();

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl == 0 {
            change_mask = change_mask.union(EFlags::IOPL_MASK);
        }
        if cpl <= self.eflags.iopl() {
            change_mask = change_mask.union(EFlags::IF_);
        }

        // VIF, VIP, VM are unaffected
        self.write_eflags(eflags32, change_mask.bits());
        Ok(())
    }

    // =========================================================================
    // ENTER (64-bit)
    // Based on Bochs stack64.cc ENTER64_IwIb
    // =========================================================================

    /// ENTER64 IwIb - Make Stack Frame (64-bit)
    /// Matching Bochs stack64.cc ENTER64_IwIb
    pub fn enter64_iw_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let level = instr.ib2() & 0x1F;

        let mut temp_rsp = self.rsp();
        let mut temp_rbp = self.get_gpr64(5); // RBP

        // Push RBP
        temp_rsp = temp_rsp.wrapping_sub(8);
        self.stack_write_qword_64(temp_rsp, temp_rbp)?;

        let frame_ptr64 = temp_rsp;

        if level > 0 {
            let mut lvl = level;
            // do level-1 times
            while { lvl -= 1; lvl } != 0 {
                temp_rbp = temp_rbp.wrapping_sub(8);
                let temp64 = self.stack_read_qword_64(temp_rbp)?;
                temp_rsp = temp_rsp.wrapping_sub(8);
                self.stack_write_qword_64(temp_rsp, temp64)?;
            }

            // push(frame pointer)
            temp_rsp = temp_rsp.wrapping_sub(8);
            self.stack_write_qword_64(temp_rsp, frame_ptr64)?;
        }

        temp_rsp = temp_rsp.wrapping_sub(instr.iw() as u64);

        // Probe final stack location (Bochs: read_RMW_linear_qword touch)
        let _ = self.read_rmw_virtual_qword_64(BxSegregs::Ss, temp_rsp)?;
        // Write back unchanged (no actual modification)
        // Bochs does the read but doesn't write back — it's just a probe.
        // Our RMW path sets up address_xlation but we don't call write_back.

        self.set_gpr64(5, frame_ptr64); // RBP = frame_ptr64
        self.set_rsp(temp_rsp);         // RSP = temp_rsp
        Ok(())
    }

    // =========================================================================
    // LEAVE (64-bit)
    // Based on Bochs stack64.cc LEAVE64
    // =========================================================================

    /// LEAVE64 - High Level Procedure Exit (64-bit)
    /// Matching C++ stack64.cc LEAVE64
    pub fn leave64(&mut self, _instr: &Instruction) -> super::Result<()> {
        // RSP = RBP, then POP RBP
        let rbp = self.get_gpr64(5); // RBP
        let value = self.stack_read_qword_64(rbp)?;
        self.set_gpr64(4, rbp.wrapping_add(8)); // RSP = RBP + 8
        self.set_gpr64(5, value); // RBP = [old RBP]
        Ok(())
    }

    // =========================================================================
    // PUSH/POP segment selectors (64-bit)
    // Based on Bochs stack64.cc PUSH_Op64_Sw / POP_Op64_Sw
    // =========================================================================

    /// PUSH segment selector (64-bit) — pushes 16-bit selector zero-extended to 64 bits
    pub fn push_op64_sw(&mut self, instr: &Instruction) -> super::Result<()> {
        let seg_idx = instr.src() as usize;
        let selector = self.sregs[seg_idx].selector.value as u64;
        self.push_64(selector)?;
        Ok(())
    }

    /// POP segment selector (64-bit) — pops 64-bit value, loads low 16 bits as selector
    pub fn pop_op64_sw(&mut self, instr: &Instruction) -> super::Result<()> {
        let selector_64 = self.pop_64()?;
        let seg_idx = BxSegregs::from(instr.dst());
        self.load_seg_reg(seg_idx, selector_64 as u16)?;
        Ok(())
    }

    /// PUSH imm8 sign-extended to 64-bit
    /// Matching Bochs PUSH_Op64_sIb — same as push_iq, imm is already sign-extended by decoder
    pub fn push_op64_sib(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm = instr.id() as i32 as u64;
        self.push_64(imm)
    }
}
