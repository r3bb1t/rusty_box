//! Stack operations for x86 CPU emulation
//!
//! Based on Bochs stack16.cc and stack32.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! NOTE: This implementation uses simple register-based stack tracking.
//! Full memory access would require integration with the memory subsystem.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxInstructionGenerated, BxSegregs},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Helper functions for stack operations
    // =========================================================================
    
    /// Check if using 32-bit stack (SS.D_B flag)
    #[inline]
    fn is_stack_32bit(&self) -> bool {
        unsafe { self.sregs[BxSegregs::Ss as usize].cache.u.segment.d_b }
    }

    /// Push a 16-bit value onto the stack
    /// Based on BX_CPU_C::push_16 in stack.h:27
    pub fn push_16(&mut self, value: u16) {
        if self.is_stack_32bit() {
            let esp = self.esp();
            let new_esp = esp.wrapping_sub(2);
            self.stack_write_word(new_esp as u32, value);
            self.set_esp(new_esp);
        } else {
            let sp = self.sp();
            let new_sp = sp.wrapping_sub(2);
            self.stack_write_word(new_sp as u32, value);
            self.set_sp(new_sp);
        }
        tracing::trace!("PUSH16: value {:#x} written to stack", value);
    }

    /// Pop a 16-bit value from the stack
    /// Based on BX_CPU_C::pop_16 in stack.h:81
    pub fn pop_16(&mut self) -> u16 {
        let value = if self.is_stack_32bit() {
            let esp = self.esp();
            let value = self.stack_read_word(esp as u32);
            self.set_esp(esp.wrapping_add(2));
            value
        } else {
            let sp = self.sp();
            let value = self.stack_read_word(sp as u32);
            self.set_sp(sp.wrapping_add(2));
            value
        };
        tracing::trace!("POP16: value {:#x} read from stack", value);
        value
    }

    /// Push a 32-bit value onto the stack
    /// Based on BX_CPU_C::push_32 in stack.h:48
    pub fn push_32(&mut self, value: u32) {
        if self.is_stack_32bit() {
            let esp = self.esp();
            let new_esp = esp.wrapping_sub(4);
            self.stack_write_dword(new_esp, value);
            self.set_esp(new_esp);
        } else {
            let sp = self.sp();
            let new_sp = sp.wrapping_sub(4);
            self.stack_write_dword(new_sp as u32, value);
            self.set_sp(new_sp);
        }
        tracing::trace!("PUSH32: value {:#x} written to stack", value);
    }

    /// Pop a 32-bit value from the stack
    /// Based on BX_CPU_C::pop_32 in stack.h:105
    pub fn pop_32(&mut self) -> u32 {
        let value = if self.is_stack_32bit() {
            let esp = self.esp();
            let value = self.stack_read_dword(esp);
            self.set_esp(esp.wrapping_add(4));
            value
        } else {
            let sp = self.sp();
            let value = self.stack_read_dword(sp as u32);
            self.set_sp(sp.wrapping_add(4));
            value
        };
        tracing::trace!("POP32: value {:#x} read from stack", value);
        value
    }

    // =========================================================================
    // 16-bit PUSH instructions
    // =========================================================================

    /// PUSH r16 - Push 16-bit register
    pub fn push_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let value = self.get_gpr16(dst);
        self.push_16(value);
        tracing::trace!("PUSH r16 (reg {}): {:#06x}", dst, value);
    }

    /// PUSH Sw - Push segment register
    pub fn push16_sw(&mut self, instr: &BxInstructionGenerated) {
        let src = instr.src() as usize;
        let value = self.sregs[src].selector.value;
        self.push_16(value);
        tracing::trace!("PUSH Sw (seg {}): {:#06x}", src, value);
    }

    /// PUSH imm16
    pub fn push_iw(&mut self, instr: &BxInstructionGenerated) {
        let value = instr.iw();
        self.push_16(value);
        tracing::trace!("PUSH imm16: {:#06x}", value);
    }

    // =========================================================================
    // 16-bit POP instructions
    // =========================================================================

    /// POP r16 - Pop into 16-bit register
    pub fn pop_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let value = self.pop_16();
        self.set_gpr16(dst, value);
        tracing::trace!("POP r16 (reg {}): {:#06x}", dst, value);
    }

    /// POP Sw - Pop into segment register
    pub fn pop16_sw(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let value = self.pop_16();
        
        // Load segment register (simplified for real mode)
        super::segment_ctrl_pro::parse_selector(value, &mut self.sregs[dst].selector);
        self.sregs[dst].cache.u.segment.base = (value as u64) << 4;
        
        tracing::trace!("POP Sw (seg {}): {:#06x}", dst, value);
    }

    // =========================================================================
    // 32-bit PUSH instructions
    // =========================================================================

    /// PUSH r32 - Push 32-bit register
    pub fn push_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let value = self.get_gpr32(dst);
        self.push_32(value);
        tracing::trace!("PUSH r32 (reg {}): {:#010x}", dst, value);
    }

    /// PUSH imm32
    pub fn push_id(&mut self, instr: &BxInstructionGenerated) {
        let value = instr.id();
        self.push_32(value);
        tracing::trace!("PUSH imm32: {:#010x}", value);
    }

    // =========================================================================
    // 32-bit POP instructions
    // =========================================================================

    /// POP r32 - Pop into 32-bit register
    pub fn pop_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let value = self.pop_32();
        self.set_gpr32(dst, value);
        tracing::trace!("POP r32 (reg {}): {:#010x}", dst, value);
    }

    // =========================================================================
    // PUSHA/POPA instructions (stub implementations)
    // =========================================================================

    /// PUSHA - Push all 16-bit general registers
    pub fn pusha16(&mut self, _instr: &BxInstructionGenerated) {
        // Simplified - just adjust SP/ESP
        if self.is_stack_32bit() {
            let esp = self.esp();
            self.set_esp(esp.wrapping_sub(16));
        } else {
            let sp = self.sp();
            self.set_sp(sp.wrapping_sub(16));
        }
        tracing::trace!("PUSHA16 (stub)");
    }

    /// POPA - Pop all 16-bit general registers
    pub fn popa16(&mut self, _instr: &BxInstructionGenerated) {
        if self.is_stack_32bit() {
            let esp = self.esp();
            self.set_esp(esp.wrapping_add(16));
        } else {
            let sp = self.sp();
            self.set_sp(sp.wrapping_add(16));
        }
        tracing::trace!("POPA16 (stub)");
    }

    // =========================================================================
    // PUSHF/POPF instructions
    // =========================================================================

    /// PUSHF - Push flags (16-bit)
    pub fn pushf_fw(&mut self, _instr: &BxInstructionGenerated) {
        let flags = (self.eflags & 0xFFFF) as u16;
        self.push_16(flags);
        tracing::trace!("PUSHF: {:#06x}", flags);
    }

    /// POPF - Pop flags (16-bit)
    pub fn popf_fw(&mut self, _instr: &BxInstructionGenerated) {
        let flags = self.pop_16();
        
        // Mask to preserve certain bits
        // Changeable: CF, PF, AF, ZF, SF, TF, DF, OF, NT
        const CHANGE_MASK: u32 = 0x0FD5; // bits 0,2,4,6,7,8,9,10,14
        
        self.eflags = (self.eflags & !CHANGE_MASK) | ((flags as u32) & CHANGE_MASK);
        tracing::trace!("POPF: {:#06x}", flags);
    }

    /// PUSHFD - Push flags (32-bit)
    pub fn pushf_fd(&mut self, _instr: &BxInstructionGenerated) {
        // VM & RF flags cleared in image stored on the stack
        let flags = self.eflags & 0x00FCFFFF;
        self.push_32(flags);
        tracing::trace!("PUSHFD: {:#010x}", flags);
    }

    /// POPFD - Pop flags (32-bit)
    pub fn popf_fd(&mut self, _instr: &BxInstructionGenerated) {
        let flags = self.pop_32();
        
        // RF is always zero after POPF
        // VM, VIP, VIF are unaffected in protected mode
        const CHANGE_MASK: u32 = 0x00244FD5;
        
        self.eflags = (self.eflags & !CHANGE_MASK) | (flags & CHANGE_MASK);
        tracing::trace!("POPFD: {:#010x}", flags);
    }

    // =========================================================================
    // Stack memory access functions
    // =========================================================================

    /// Write a 16-bit value to stack at given offset (SS:offset)
    /// Based on BX_CPU_C::stack_write_word in stack.cc:161
    fn stack_write_word(&mut self, offset: u32, value: u16) {
        // Get linear address from SS:offset using get_laddr32
        let laddr = self.get_laddr32(BxSegregs::Ss as usize, offset);
        // Write through memory subsystem
        self.mem_write_word(laddr as u64, value);
    }

    /// Write a 32-bit value to stack at given offset (SS:offset)
    /// Based on BX_CPU_C::stack_write_dword in stack.cc:194
    pub(super) fn stack_write_dword(&mut self, offset: u32, value: u32) {
        // Get linear address from SS:offset using get_laddr32
        let laddr = self.get_laddr32(BxSegregs::Ss as usize, offset);
        // Write through memory subsystem
        self.mem_write_dword(laddr as u64, value);
    }

    /// Read a 16-bit value from stack at given offset (SS:offset)
    /// Based on BX_CPU_C::stack_read_word in stack.cc:282
    fn stack_read_word(&self, offset: u32) -> u16 {
        // Get linear address from SS:offset using get_laddr32
        let laddr = self.get_laddr32(BxSegregs::Ss as usize, offset);
        // Read through memory subsystem
        self.mem_read_word(laddr as u64)
    }

    /// Read a 32-bit value from stack at given offset (SS:offset)
    /// Based on BX_CPU_C::stack_read_dword in stack.cc:313
    pub(super) fn stack_read_dword(&self, offset: u32) -> u32 {
        // Get linear address from SS:offset using get_laddr32
        let laddr = self.get_laddr32(BxSegregs::Ss as usize, offset);
        // Read through memory subsystem
        self.mem_read_dword(laddr as u64)
    }

    // =========================================================================
    // 64-bit stack operations
    // =========================================================================

    /// Push a 64-bit value onto the stack
    /// Based on BX_CPU_C::push_64 in stack.h (64-bit mode)
    pub fn push_64(&mut self, value: u64) {
        // In 64-bit mode, stack is always 64-bit
        let rsp = self.rsp();
        let new_rsp = rsp.wrapping_sub(8);
        self.stack_write_qword(new_rsp, value);
        self.set_rsp(new_rsp);
        tracing::trace!("PUSH64: value {:#x} written to stack", value);
    }

    /// Pop a 64-bit value from the stack
    /// Based on BX_CPU_C::pop_64 in stack.h (64-bit mode)
    pub fn pop_64(&mut self) -> u64 {
        // In 64-bit mode, stack is always 64-bit
        let rsp = self.rsp();
        let value = self.stack_read_qword(rsp);
        self.set_rsp(rsp.wrapping_add(8));
        tracing::trace!("POP64: value {:#x} read from stack", value);
        value
    }

    /// Write a 64-bit value to stack at given offset (SS:offset)
    pub(super) fn stack_write_qword(&mut self, offset: u64, value: u64) {
        // Get linear address from SS:offset
        let laddr = self.get_laddr64(BxSegregs::Ss as usize, offset);
        // Write through memory subsystem
        self.mem_write_qword(laddr, value);
    }

    /// Read a 64-bit value from stack at given offset (SS:offset)
    pub(super) fn stack_read_qword(&self, offset: u64) -> u64 {
        // Get linear address from SS:offset
        let laddr = self.get_laddr64(BxSegregs::Ss as usize, offset);
        // Read through memory subsystem
        self.mem_read_qword(laddr)
    }
}
