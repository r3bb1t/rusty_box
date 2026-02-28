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

    // =========================================================================
    // 64-bit stack memory access functions
    // Based on Bochs stack.cc
    // =========================================================================

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

    // =========================================================================
    // 64-bit PUSH instructions
    // Based on Bochs stack64.cc
    // =========================================================================

    /// PUSH r64 - Push 64-bit register
    /// Based on Bochs stack64.cc PUSH_EqR
    pub fn push_eq_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let value = self.get_gpr64(dst);
        self.push_64(value);
        tracing::trace!("PUSH r64 (reg {}): {:#018x}", dst, value);
    }

    /// PUSH imm64 (sign-extended from 32-bit)
    /// Based on Bochs stack64.cc PUSH_Iq
    pub fn push_iq(&mut self, instr: &Instruction) {
        // Sign extend 32-bit immediate to 64-bit
        let value = instr.id() as i32 as i64 as u64;
        self.push_64(value);
        tracing::trace!("PUSH imm64: {:#018x}", value);
    }

    // =========================================================================
    // 64-bit POP instructions
    // Based on Bochs stack64.cc
    // =========================================================================

    /// POP r64 - Pop into 64-bit register
    /// Based on Bochs stack64.cc POP_EqR
    pub fn pop_eq_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let value = self.pop_64();
        self.set_gpr64(dst, value);
        tracing::trace!("POP r64 (reg {}): {:#018x}", dst, value);
    }

    // =========================================================================
    // PUSHFQ/POPFQ instructions (64-bit)
    // Based on Bochs flag_ctrl.cc
    // =========================================================================

    /// PUSHFQ - Push flags (64-bit)
    pub fn pushf_fq(&mut self, _instr: &Instruction) {
        // VM & RF flags cleared in image stored on the stack
        let flags = (self.eflags.bits() & 0x00FCFFFF) as u64;
        self.push_64(flags);
        tracing::trace!("PUSHFQ: {:#018x}", flags);
    }

    /// POPFQ - Pop flags (64-bit)
    pub fn popf_fq(&mut self, _instr: &Instruction) {
        let flags = self.pop_64();

        // RF is always zero after POPF
        // VM, VIP, VIF are unaffected
        const CHANGE_MASK: u32 = 0x00244FD5;

        self.eflags = EFlags::from_bits_retain(
            (self.eflags.bits() & !CHANGE_MASK) | ((flags as u32) & CHANGE_MASK),
        );
        tracing::trace!("POPFQ: {:#018x}", flags);
    }
}
