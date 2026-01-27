//! 16-bit Stack operations for x86 CPU emulation
//!
//! Based on Bochs stack16.cc
//! Copyright (C) 2001-2018 The Bochs Project

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::BxInstructionGenerated,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // 16-bit PUSH instructions
    // Based on Bochs stack16.cc:27-70
    // =========================================================================

    /// PUSH r16 - Push 16-bit register
    /// Based on Bochs stack16.cc PUSH_EwR
    pub fn push_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let value = self.get_gpr16(dst);
        self.push_16(value);
        tracing::trace!("PUSH r16 (reg {}): {:#06x}", dst, value);
    }

    /// PUSH Sw - Push segment register
    /// Based on Bochs stack16.cc PUSH16_Sw
    pub fn push16_sw(&mut self, instr: &BxInstructionGenerated) {
        let src = instr.src() as usize;
        let value = self.sregs[src].selector.value;
        self.push_16(value);
        tracing::trace!("PUSH Sw (seg {}): {:#06x}", src, value);
    }

    /// PUSH imm16
    /// Based on Bochs stack16.cc PUSH_Iw
    pub fn push_iw(&mut self, instr: &BxInstructionGenerated) {
        let value = instr.iw();
        self.push_16(value);
        tracing::trace!("PUSH imm16: {:#06x}", value);
    }

    // =========================================================================
    // 16-bit POP instructions
    // Based on Bochs stack16.cc:72-101
    // =========================================================================

    /// POP r16 - Pop into 16-bit register
    /// Based on Bochs stack16.cc POP_EwR
    pub fn pop_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let value = self.pop_16();
        self.set_gpr16(dst, value);
        tracing::trace!("POP r16 (reg {}): {:#06x}", dst, value);
    }

    /// POP Sw - Pop into segment register
    /// Based on Bochs stack16.cc POP16_Sw
    pub fn pop16_sw(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let value = self.pop_16();

        // Load segment register (simplified for real mode)
        super::segment_ctrl_pro::parse_selector(value, &mut self.sregs[dst].selector);
        self.sregs[dst].cache.u.segment.base = (value as u64) << 4;

        tracing::trace!("POP Sw (seg {}): {:#06x}", dst, value);
    }

    // =========================================================================
    // PUSHA16/POPA16 instructions
    // Based on Bochs stack16.cc:103-176
    // =========================================================================

    /// PUSHA - Push all 16-bit general registers
    /// Push order: AX, CX, DX, BX, SP (original), BP, SI, DI
    /// Based on Bochs stack16.cc:103-134
    pub fn pusha16(&mut self, _instr: &BxInstructionGenerated) {
        // Get register values before any pushes
        let ax = self.ax();
        let cx = self.cx();
        let dx = self.dx();
        let bx = self.bx();
        let bp = self.bp();
        let si = self.si();
        let di = self.di();

        if self.is_stack_32bit() {
            let temp_esp = self.esp();
            let temp_sp = self.sp();

            // Write all registers to stack at their final positions
            self.stack_write_word(temp_esp.wrapping_sub(2), ax);
            self.stack_write_word(temp_esp.wrapping_sub(4), cx);
            self.stack_write_word(temp_esp.wrapping_sub(6), dx);
            self.stack_write_word(temp_esp.wrapping_sub(8), bx);
            self.stack_write_word(temp_esp.wrapping_sub(10), temp_sp);
            self.stack_write_word(temp_esp.wrapping_sub(12), bp);
            self.stack_write_word(temp_esp.wrapping_sub(14), si);
            self.stack_write_word(temp_esp.wrapping_sub(16), di);

            self.set_esp(temp_esp.wrapping_sub(16));
        } else {
            let temp_sp = self.sp();

            // Write all registers to stack at their final positions
            self.stack_write_word(temp_sp.wrapping_sub(2) as u32, ax);
            self.stack_write_word(temp_sp.wrapping_sub(4) as u32, cx);
            self.stack_write_word(temp_sp.wrapping_sub(6) as u32, dx);
            self.stack_write_word(temp_sp.wrapping_sub(8) as u32, bx);
            self.stack_write_word(temp_sp.wrapping_sub(10) as u32, temp_sp);
            self.stack_write_word(temp_sp.wrapping_sub(12) as u32, bp);
            self.stack_write_word(temp_sp.wrapping_sub(14) as u32, si);
            self.stack_write_word(temp_sp.wrapping_sub(16) as u32, di);

            self.set_sp(temp_sp.wrapping_sub(16));
        }

        tracing::trace!("PUSHA16: AX={:04x} CX={:04x} DX={:04x} BX={:04x} BP={:04x} SI={:04x} DI={:04x}",
            ax, cx, dx, bx, bp, si, di);
    }

    /// POPA - Pop all 16-bit general registers
    /// Pop order: DI, SI, BP, (skip SP), BX, DX, CX, AX
    /// Based on Bochs stack16.cc:136-176
    pub fn popa16(&mut self, _instr: &BxInstructionGenerated) {
        let (di, si, bp, bx, dx, cx, ax) = if self.is_stack_32bit() {
            let temp_esp = self.esp();

            let di = self.stack_read_word(temp_esp);
            let si = self.stack_read_word(temp_esp.wrapping_add(2));
            let bp = self.stack_read_word(temp_esp.wrapping_add(4));
            // Skip reading SP at offset +6 (it's discarded)
            let _ = self.stack_read_word(temp_esp.wrapping_add(6));
            let bx = self.stack_read_word(temp_esp.wrapping_add(8));
            let dx = self.stack_read_word(temp_esp.wrapping_add(10));
            let cx = self.stack_read_word(temp_esp.wrapping_add(12));
            let ax = self.stack_read_word(temp_esp.wrapping_add(14));

            self.set_esp(temp_esp.wrapping_add(16));

            (di, si, bp, bx, dx, cx, ax)
        } else {
            let temp_sp = self.sp();

            let di = self.stack_read_word(temp_sp as u32);
            let si = self.stack_read_word(temp_sp.wrapping_add(2) as u32);
            let bp = self.stack_read_word(temp_sp.wrapping_add(4) as u32);
            // Skip reading SP at offset +6 (it's discarded)
            let _ = self.stack_read_word(temp_sp.wrapping_add(6) as u32);
            let bx = self.stack_read_word(temp_sp.wrapping_add(8) as u32);
            let dx = self.stack_read_word(temp_sp.wrapping_add(10) as u32);
            let cx = self.stack_read_word(temp_sp.wrapping_add(12) as u32);
            let ax = self.stack_read_word(temp_sp.wrapping_add(14) as u32);

            self.set_sp(temp_sp.wrapping_add(16));

            (di, si, bp, bx, dx, cx, ax)
        };

        // Update all registers
        self.set_di(di);
        self.set_si(si);
        self.set_bp(bp);
        self.set_bx(bx);
        self.set_dx(dx);
        self.set_cx(cx);
        self.set_ax(ax);

        tracing::trace!("POPA16: DI={:04x} SI={:04x} BP={:04x} BX={:04x} DX={:04x} CX={:04x} AX={:04x}",
            di, si, bp, bx, dx, cx, ax);
    }

    // =========================================================================
    // PUSHF/POPF instructions (16-bit)
    // Based on Bochs flag_ctrl.cc (but traditionally in stack16.cc)
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
}
