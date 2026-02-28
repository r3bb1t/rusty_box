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

use super::{cpu::BxCpuC, cpuid::BxCpuIdTrait, decoder::BxSegregs};

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

    /// Push a 16-bit value onto the stack.
    pub fn push_16(&mut self, value: u16) -> super::Result<()> {
        if self.is_stack_32bit() {
            let esp = self.esp();
            let new_esp = esp.wrapping_sub(2);
            self.stack_write_word(new_esp as u32, value)?;
            self.set_esp(new_esp);
        } else {
            let sp = self.sp();
            let new_sp = sp.wrapping_sub(2);
            self.stack_write_word(new_sp as u32, value)?;
            self.set_sp(new_sp);
        }
        Ok(())
    }

    /// Pop a 16-bit value from the stack.
    pub fn pop_16(&mut self) -> super::Result<u16> {
        if self.is_stack_32bit() {
            let esp = self.esp();
            let value = self.stack_read_word(esp as u32)?;
            self.set_esp(esp.wrapping_add(2));
            Ok(value)
        } else {
            let sp = self.sp();
            let value = self.stack_read_word(sp as u32)?;
            self.set_sp(sp.wrapping_add(2));
            Ok(value)
        }
    }

    // =========================================================================
    // 32-bit push/pop primitives
    // =========================================================================

    /// Push a 32-bit value onto the stack.
    pub fn push_32(&mut self, value: u32) -> super::Result<()> {
        if self.is_stack_32bit() {
            let esp = self.esp();
            let new_esp = esp.wrapping_sub(4);
            self.stack_write_dword(new_esp, value)?;
            self.set_esp(new_esp);
        } else {
            let sp = self.sp();
            let new_sp = sp.wrapping_sub(4);
            self.stack_write_dword(new_sp as u32, value)?;
            self.set_sp(new_sp);
        }
        Ok(())
    }

    /// Pop a 32-bit value from the stack.
    pub fn pop_32(&mut self) -> super::Result<u32> {
        if self.is_stack_32bit() {
            let esp = self.esp();
            let value = self.stack_read_dword(esp)?;
            self.set_esp(esp.wrapping_add(4));
            Ok(value)
        } else {
            let sp = self.sp();
            let value = self.stack_read_dword(sp as u32)?;
            self.set_sp(sp.wrapping_add(4));
            Ok(value)
        }
    }

    // =========================================================================
    // Stack memory access functions
    // =========================================================================

    /// Write a 16-bit value to stack at SS:offset.
    pub(super) fn stack_write_word(&mut self, offset: u32, value: u16) -> super::Result<()> {
        let laddr = self.get_laddr32(BxSegregs::Ss as usize, offset) as u64;
        let paddr = self.translate_data_write(laddr)?;
        self.mem_write_word(paddr, value);
        Ok(())
    }

    /// Write a 32-bit value to stack at SS:offset.
    pub(super) fn stack_write_dword(&mut self, offset: u32, value: u32) -> super::Result<()> {
        let laddr = self.get_laddr32(BxSegregs::Ss as usize, offset) as u64;
        let paddr = self.translate_data_write(laddr)?;
        self.mem_write_dword(paddr, value);
        Ok(())
    }

    /// Read a 16-bit value from stack at SS:offset.
    pub(super) fn stack_read_word(&mut self, offset: u32) -> super::Result<u16> {
        let laddr = self.get_laddr32(BxSegregs::Ss as usize, offset) as u64;
        let paddr = self.translate_data_read(laddr)?;
        Ok(self.mem_read_word(paddr))
    }

    /// Read a 32-bit value from stack at SS:offset.
    pub(super) fn stack_read_dword(&mut self, offset: u32) -> super::Result<u32> {
        let laddr = self.get_laddr32(BxSegregs::Ss as usize, offset) as u64;
        let paddr = self.translate_data_read(laddr)?;
        Ok(self.mem_read_dword(paddr))
    }
}
