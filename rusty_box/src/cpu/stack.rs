//! Common stack operations for x86 CPU emulation
//!
//! Based on Bochs stack.cc and stack.h
//!
//! This module contains common stack primitives (push/pop) and stack memory
//! access functions. Instruction-specific implementations are in:
//! - stack16.rs: 16-bit stack instructions (PUSH/POP r16, PUSHA16, POPA16, etc.)
//! - stack32.rs: 32-bit stack instructions (PUSH/POP r32, PUSHAD, POPAD, etc.)
//! - stack64.rs: 64-bit stack instructions (PUSH/POP r64, etc.)

use super::{cpu::BxCpuC, cpuid::BxCpuIdTrait, decoder::BxSegregs};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // =========================================================================
    // Helper functions for stack operations
    // Based on Bochs stack.h and stack.cc
    // =========================================================================

    /// Check if using 32-bit stack (SS.D_B flag)
    /// Based on Bochs BX_CPU_THIS_PTR sregs[BX_SEG_REG_SS].cache.u.segment.d_b
    #[inline]
    pub(super) fn is_stack_32bit(&self) -> bool {
        self.sregs[BxSegregs::Ss as usize].cache.u.segment_d_b()
    }

    // =========================================================================
    // 16-bit push/pop primitives
    // Based on Bochs stack.h
    // =========================================================================

    /// Push a 16-bit value onto the stack.
    /// Bochs stack.h — three paths: long64 (RSP), d_b=1 (ESP), d_b=0 (SP)
    pub fn push_16(&mut self, value: u16) -> super::Result<()> {
        if self.long64_mode() {
            let rsp = self.rsp();
            let new_rsp = rsp.wrapping_sub(2);
            self.stack_write_word_64(new_rsp, value)?;
            self.set_rsp(new_rsp);
        } else if self.is_stack_32bit() {
            let esp = self.esp();
            let new_esp = esp.wrapping_sub(2);
            self.stack_write_word(new_esp, value)?;
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
    /// Bochs stack.h — three paths: long64 (RSP), d_b=1 (ESP), d_b=0 (SP)
    pub fn pop_16(&mut self) -> super::Result<u16> {
        if self.long64_mode() {
            let rsp = self.rsp();
            let value = self.stack_read_word_64(rsp)?;
            self.set_rsp(rsp.wrapping_add(2));
            Ok(value)
        } else if self.is_stack_32bit() {
            let esp = self.esp();
            let value = self.stack_read_word(esp)?;
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
    /// Bochs stack.h — three paths: long64 (RSP), d_b=1 (ESP), d_b=0 (SP)
    pub fn push_32(&mut self, value: u32) -> super::Result<()> {
        if self.long64_mode() {
            let rsp = self.rsp();
            let new_rsp = rsp.wrapping_sub(4);
            self.stack_write_dword_64(new_rsp, value)?;
            self.set_rsp(new_rsp);
        } else if self.is_stack_32bit() {
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
    /// Bochs stack.h — three paths: long64 (RSP), d_b=1 (ESP), d_b=0 (SP)
    pub fn pop_32(&mut self) -> super::Result<u32> {
        if self.long64_mode() {
            let rsp = self.rsp();
            let value = self.stack_read_dword_64(rsp)?;
            self.set_rsp(rsp.wrapping_add(4));
            Ok(value)
        } else if self.is_stack_32bit() {
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

    // Bochs stack.cc `stack_write_word` / `stack_read_word` (and the dword
    // siblings) take a fast path through the stack prefetch window, whose
    // `stackPrefetch(offset, len)` guarantees the window covers the whole
    // access — so that path can never span two pages — and otherwise fall
    // back to `write_virtual_word(BX_SEG_REG_SS, offset, data)`.
    //
    // rusty_box has no stack prefetch window, so the fallback is the whole
    // implementation. Translating once and then issuing a PHYSICAL access of
    // the full width (the previous shape) silently treated a word or dword
    // straddling a page boundary as physically contiguous: the bytes past the
    // boundary landed in whatever frame happened to follow, and a fault on
    // the second page was never raised.

    /// Write a 16-bit value to stack at SS:offset.
    pub(super) fn stack_write_word(&mut self, offset: u32, value: u16) -> super::Result<()> {
        self.write_virtual_word(BxSegregs::Ss, offset, value)
    }

    /// Write a 32-bit value to stack at SS:offset.
    pub(super) fn stack_write_dword(&mut self, offset: u32, value: u32) -> super::Result<()> {
        self.write_virtual_dword(BxSegregs::Ss, offset, value)
    }

    /// Read a 16-bit value from stack at SS:offset.
    pub(super) fn stack_read_word(&mut self, offset: u32) -> super::Result<u16> {
        self.read_virtual_word(BxSegregs::Ss, offset)
    }

    /// Read a 32-bit value from stack at SS:offset.
    pub(super) fn stack_read_dword(&mut self, offset: u32) -> super::Result<u32> {
        self.read_virtual_dword(BxSegregs::Ss, offset)
    }
}
