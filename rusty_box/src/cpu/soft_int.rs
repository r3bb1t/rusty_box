//! Software interrupt instructions for x86 CPU emulation
//!
//! Based on Bochs soft_int.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements INT, INT3, INTO, IRET instructions

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxInstructionGenerated, BxSegregs},
    segment_ctrl_pro::parse_selector,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // INT - Software Interrupt
    // =========================================================================
    
    /// INT imm8 - Software interrupt with immediate vector
    pub fn int_ib(&mut self, instr: &BxInstructionGenerated) {
        let vector = instr.ib();
        tracing::debug!("INT {:#04x}", vector);
        
        // In real mode, use IVT (Interrupt Vector Table) at 0000:0000
        self.interrupt_real_mode(vector);
    }

    /// INT3 - Breakpoint interrupt (vector 3)
    pub fn int3(&mut self, _instr: &BxInstructionGenerated) {
        tracing::debug!("INT3 (breakpoint)");
        self.interrupt_real_mode(3);
    }

    /// INTO - Interrupt on overflow (vector 4, only if OF=1)
    pub fn into(&mut self, _instr: &BxInstructionGenerated) {
        if self.get_of() {
            tracing::debug!("INTO: overflow detected, calling INT 4");
            self.interrupt_real_mode(4);
        }
    }

    // =========================================================================
    // IRET - Interrupt Return
    // =========================================================================
    
    /// IRET - Return from interrupt (16-bit operand size)
    pub fn iret16(&mut self, _instr: &BxInstructionGenerated) {
        // Pop IP, CS, FLAGS from stack
        let new_ip = self.pop_16();
        let new_cs = self.pop_16();
        let new_flags = self.pop_16();
        
        // Load CS with new selector (real mode)
        let cs_index = BxSegregs::Cs as usize;
        parse_selector(new_cs, &mut self.sregs[cs_index].selector);
        unsafe {
            self.sregs[cs_index].cache.u.segment.base = (new_cs as u64) << 4;
        }
        
        // Set IP
        self.set_ip(new_ip);
        
        // Update FLAGS (preserve some bits)
        self.eflags = (self.eflags & 0xFFFF0000) | (new_flags as u32);
        
        // Invalidate prefetch
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;
        
        tracing::debug!("IRET16: returning to {:04x}:{:04x}, flags={:04x}", new_cs, new_ip, new_flags);
    }

    /// IRET - Return from interrupt (32-bit operand size)
    pub fn iret32(&mut self, _instr: &BxInstructionGenerated) {
        // Pop EIP, CS, EFLAGS from stack
        let new_eip = self.pop_32();
        let new_cs = self.pop_32() as u16;
        let new_eflags = self.pop_32();
        
        // Load CS with new selector (real mode)
        let cs_index = BxSegregs::Cs as usize;
        parse_selector(new_cs, &mut self.sregs[cs_index].selector);
        unsafe {
            self.sregs[cs_index].cache.u.segment.base = (new_cs as u64) << 4;
        }
        
        // Set EIP
        self.set_eip(new_eip);
        
        // Update EFLAGS
        self.eflags = new_eflags;
        
        // Invalidate prefetch
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;
        
        tracing::debug!("IRET32: returning to {:04x}:{:08x}, eflags={:08x}", new_cs, new_eip, new_eflags);
    }

    // =========================================================================
    // Real Mode Interrupt Handler
    // =========================================================================
    
    /// Handle interrupt in real mode using IVT
    fn interrupt_real_mode(&mut self, vector: u8) {
        // Save current FLAGS, CS, IP on stack
        let flags = (self.eflags & 0xFFFF) as u16;
        let cs = self.sregs[BxSegregs::Cs as usize].selector.value;
        let ip = self.get_ip();
        
        // Push FLAGS, CS, IP
        self.push_16(flags);
        self.push_16(cs);
        self.push_16(ip);
        
        // Clear IF and TF
        self.eflags &= !((1 << 9) | (1 << 8)); // Clear IF (bit 9) and TF (bit 8)
        
        // Read interrupt vector from IVT at 0000:vector*4
        let ivt_offset = (vector as u64) * 4;
        let new_ip = self.mem_read_word(ivt_offset);
        let new_cs = self.mem_read_word(ivt_offset + 2);
        
        // Load CS:IP from IVT
        let cs_index = BxSegregs::Cs as usize;
        parse_selector(new_cs, &mut self.sregs[cs_index].selector);
        unsafe {
            self.sregs[cs_index].cache.u.segment.base = (new_cs as u64) << 4;
        }
        self.set_ip(new_ip);
        
        // Invalidate prefetch
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;
        
        tracing::debug!("INT {:#04x}: vector at {:04x}:{:04x}", vector, new_cs, new_ip);
    }

    // =========================================================================
    // HLT - Halt instruction
    // =========================================================================
    
    /// HLT - Halt CPU until interrupt
    pub fn hlt(&mut self, _instr: &BxInstructionGenerated) {
        tracing::debug!("HLT: CPU halted");
        // In a real emulator, we'd set a flag to indicate halted state
        // For now, just log it - activity_state would need proper CpuActivityState handling
    }
}
