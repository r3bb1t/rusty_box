//! 32-bit Stack operations for x86 CPU emulation
//!
//! Based on Bochs stack32.cc
//! Copyright (C) 2001-2018 The Bochs Project

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::BxInstructionGenerated,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // 32-bit PUSH instructions
    // Based on Bochs stack32.cc:27-70
    // =========================================================================

    /// PUSH r32 - Push 32-bit register
    /// Based on Bochs stack32.cc PUSH_EdR
    pub fn push_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let value = self.get_gpr32(dst);
        self.push_32(value);
        tracing::trace!("PUSH r32 (reg {}): {:#010x}", dst, value);
    }

    /// PUSH imm32
    /// Based on Bochs stack32.cc PUSH_Id
    pub fn push_id(&mut self, instr: &BxInstructionGenerated) {
        let value = instr.id();
        self.push_32(value);
        tracing::trace!("PUSH imm32: {:#010x}", value);
    }

    // =========================================================================
    // 32-bit POP instructions
    // Based on Bochs stack32.cc:72-118
    // =========================================================================

    /// POP r32 - Pop into 32-bit register
    /// Based on Bochs stack32.cc POP_EdR
    pub fn pop_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let value = self.pop_32();
        self.set_gpr32(dst, value);
        tracing::trace!("POP r32 (reg {}): {:#010x}", dst, value);
    }

    // =========================================================================
    // PUSHAD/POPAD instructions
    // Based on Bochs stack32.cc:120-193
    // =========================================================================

    /// PUSHAD - Push all 32-bit general registers
    /// Push order: EAX, ECX, EDX, EBX, ESP (original), EBP, ESI, EDI
    /// Based on Bochs stack32.cc:120-151
    pub fn pusha32(&mut self, _instr: &BxInstructionGenerated) {
        // Get register values before any pushes
        let eax = self.eax();
        let ecx = self.ecx();
        let edx = self.edx();
        let ebx = self.ebx();
        let ebp = self.ebp();
        let esi = self.esi();
        let edi = self.edi();

        if self.is_stack_32bit() {
            let temp_esp = self.esp();

            // Write all registers to stack at their final positions
            self.stack_write_dword(temp_esp.wrapping_sub(4), eax);
            self.stack_write_dword(temp_esp.wrapping_sub(8), ecx);
            self.stack_write_dword(temp_esp.wrapping_sub(12), edx);
            self.stack_write_dword(temp_esp.wrapping_sub(16), ebx);
            self.stack_write_dword(temp_esp.wrapping_sub(20), temp_esp);
            self.stack_write_dword(temp_esp.wrapping_sub(24), ebp);
            self.stack_write_dword(temp_esp.wrapping_sub(28), esi);
            self.stack_write_dword(temp_esp.wrapping_sub(32), edi);

            self.set_esp(temp_esp.wrapping_sub(32));
        } else {
            let temp_sp = self.sp();
            let temp_esp = self.esp();

            // Write all registers to stack at their final positions
            self.stack_write_dword(temp_sp.wrapping_sub(4) as u32, eax);
            self.stack_write_dword(temp_sp.wrapping_sub(8) as u32, ecx);
            self.stack_write_dword(temp_sp.wrapping_sub(12) as u32, edx);
            self.stack_write_dword(temp_sp.wrapping_sub(16) as u32, ebx);
            self.stack_write_dword(temp_sp.wrapping_sub(20) as u32, temp_esp);
            self.stack_write_dword(temp_sp.wrapping_sub(24) as u32, ebp);
            self.stack_write_dword(temp_sp.wrapping_sub(28) as u32, esi);
            self.stack_write_dword(temp_sp.wrapping_sub(32) as u32, edi);

            self.set_sp(temp_sp.wrapping_sub(32));
        }

        tracing::trace!("PUSHAD: EAX={:08x} ECX={:08x} EDX={:08x} EBX={:08x} EBP={:08x} ESI={:08x} EDI={:08x}",
            eax, ecx, edx, ebx, ebp, esi, edi);
    }

    /// POPAD - Pop all 32-bit general registers
    /// Pop order: EDI, ESI, EBP, (skip ESP), EBX, EDX, ECX, EAX
    /// Based on Bochs stack32.cc:153-193
    pub fn popa32(&mut self, _instr: &BxInstructionGenerated) {
        let (edi, esi, ebp, ebx, edx, ecx, eax) = if self.is_stack_32bit() {
            let temp_esp = self.esp();

            let edi = self.stack_read_dword(temp_esp);
            let esi = self.stack_read_dword(temp_esp.wrapping_add(4));
            let ebp = self.stack_read_dword(temp_esp.wrapping_add(8));
            // Skip reading ESP at offset +12 (it's discarded)
            let _ = self.stack_read_dword(temp_esp.wrapping_add(12));
            let ebx = self.stack_read_dword(temp_esp.wrapping_add(16));
            let edx = self.stack_read_dword(temp_esp.wrapping_add(20));
            let ecx = self.stack_read_dword(temp_esp.wrapping_add(24));
            let eax = self.stack_read_dword(temp_esp.wrapping_add(28));

            self.set_esp(temp_esp.wrapping_add(32));

            (edi, esi, ebp, ebx, edx, ecx, eax)
        } else {
            let temp_sp = self.sp();

            let edi = self.stack_read_dword(temp_sp as u32);
            let esi = self.stack_read_dword(temp_sp.wrapping_add(4) as u32);
            let ebp = self.stack_read_dword(temp_sp.wrapping_add(8) as u32);
            // Skip reading ESP at offset +12 (it's discarded)
            let _ = self.stack_read_dword(temp_sp.wrapping_add(12) as u32);
            let ebx = self.stack_read_dword(temp_sp.wrapping_add(16) as u32);
            let edx = self.stack_read_dword(temp_sp.wrapping_add(20) as u32);
            let ecx = self.stack_read_dword(temp_sp.wrapping_add(24) as u32);
            let eax = self.stack_read_dword(temp_sp.wrapping_add(28) as u32);

            self.set_sp(temp_sp.wrapping_add(32));

            (edi, esi, ebp, ebx, edx, ecx, eax)
        };

        // Update all registers
        self.set_edi(edi);
        self.set_esi(esi);
        self.set_ebp(ebp);
        self.set_ebx(ebx);
        self.set_edx(edx);
        self.set_ecx(ecx);
        self.set_eax(eax);

        tracing::trace!("POPAD: EDI={:08x} ESI={:08x} EBP={:08x} EBX={:08x} EDX={:08x} ECX={:08x} EAX={:08x}",
            edi, esi, ebp, ebx, edx, ecx, eax);
    }

    // =========================================================================
    // PUSHFD/POPFD instructions (32-bit)
    // Based on Bochs flag_ctrl.cc (but traditionally in stack32.cc)
    // =========================================================================

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
}
