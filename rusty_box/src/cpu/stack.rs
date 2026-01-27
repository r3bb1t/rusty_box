//! Common stack operations for x86 CPU emulation
//!
//! Based on Bochs stack.cc and stack.h
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! This module contains common stack primitives (push/pop) and stack memory
//! access functions. Instruction-specific implementations are in:
//! - stack16.rs: 16-bit stack instructions (PUSH/POP r16, PUSHA16, POPA16, etc.)
//! - stack32.rs: 32-bit stack instructions (PUSH/POP r32, PUSHAD, POPAD, etc.)
//! - stack64.rs: 64-bit stack instructions (PUSH/POP r64, etc.)

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::BxSegregs,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Helper functions for stack operations
    // Based on Bochs stack.h and stack.cc
    // =========================================================================

    /// Check if using 32-bit stack (SS.D_B flag)
    /// Based on Bochs BX_CPU_THIS_PTR sregs[BX_SEG_REG_SS].cache.u.segment.d_b
    #[inline]
    pub(super) fn is_stack_32bit(&self) -> bool {
        unsafe { self.sregs[BxSegregs::Ss as usize].cache.u.segment.d_b }
    }

    // =========================================================================
    // 16-bit push/pop primitives
    // Based on Bochs stack.h:27-79
    // =========================================================================

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
            // Debug: log first 100 push operations or when SP wraps
            if self.icount < 100 || (sp < 0x10 && new_sp > 0xFFF0) {
                let ss = self.sregs[BxSegregs::Ss as usize].selector.value;
                tracing::info!("PUSH16[{}]: SP {:04x}->{:04x}, val={:04x}, SS={:04x}",
                    self.icount, sp, new_sp, value, ss);
            }
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
            // Debug: log when popping 0 (potential issue indicator)
            if value == 0 && sp < 0x100 {
                let ss = self.sregs[BxSegregs::Ss as usize].selector.value;
                let laddr = self.get_laddr32(BxSegregs::Ss as usize, sp as u32);
                tracing::warn!("POP16: popped 0 from SS:SP={:04x}:{:04x} (laddr={:#x})", ss, sp, laddr);
            }
            self.set_sp(sp.wrapping_add(2));
            value
        };
        tracing::trace!("POP16: value {:#x} read from stack", value);
        value
    }

    // =========================================================================
    // 32-bit push/pop primitives
    // Based on Bochs stack.h:48-103
    // =========================================================================

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
    // Stack memory access functions
    // Based on Bochs stack.cc:161-313
    // =========================================================================

    /// Write a 16-bit value to stack at given offset (SS:offset)
    /// Based on BX_CPU_C::stack_write_word in stack.cc:161
    pub(super) fn stack_write_word(&mut self, offset: u32, value: u16) {
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
    pub(super) fn stack_read_word(&self, offset: u32) -> u16 {
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
}
