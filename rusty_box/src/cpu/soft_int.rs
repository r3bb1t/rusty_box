//! Software interrupt instructions for x86 CPU emulation
//!
//! Based on Bochs soft_int.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements INT, INT3, INTO, IRET instructions

use super::{
    cpu::{BxCpuC, CpuActivityState, BX_ASYNC_EVENT_STOP_TRACE},
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
    // BOUND - Check Array Index Against Bounds
    // Based on Bochs soft_int.cc BOUND_GwMa and BOUND_GdMa
    // =========================================================================

    /// BOUND r16, m16&16 - Check 16-bit register against bounds in memory
    ///
    /// Compares the signed value in r16 against the signed lower and upper bounds
    /// at memory location. If the index is out of bounds, generates #BR exception.
    pub fn bound_gw_ma(&mut self, instr: &BxInstructionGenerated) {
        // Get the 16-bit register value (signed)
        let op1_16 = self.get_gpr16(instr.dst() as usize) as i16;

        // Calculate effective address
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);

        // Read lower and upper bounds from memory (2 words)
        let bound_min = self.read_virtual_word(seg, eaddr) as i16;
        let bound_max = self.read_virtual_word(seg, eaddr.wrapping_add(2)) as i16;

        tracing::trace!(
            "BOUND r16: value={}, min={}, max={}",
            op1_16, bound_min, bound_max
        );

        // Check if value is outside bounds
        if op1_16 < bound_min || op1_16 > bound_max {
            tracing::debug!("BOUND: fails bounds test (value {} not in [{}, {}])",
                op1_16, bound_min, bound_max);
            // Generate #BR exception (Bound Range Exceeded, vector 5)
            self.interrupt_real_mode(5);
        }
    }

    /// BOUND r32, m32&32 - Check 32-bit register against bounds in memory
    ///
    /// Compares the signed value in r32 against the signed lower and upper bounds
    /// at memory location. If the index is out of bounds, generates #BR exception.
    pub fn bound_gd_ma(&mut self, instr: &BxInstructionGenerated) {
        // Get the 32-bit register value (signed)
        let op1_32 = self.get_gpr32(instr.dst() as usize) as i32;

        // Calculate effective address
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);

        // Read lower and upper bounds from memory (2 dwords)
        let bound_min = self.read_virtual_dword(seg, eaddr) as i32;
        let bound_max = self.read_virtual_dword(seg, eaddr.wrapping_add(4)) as i32;

        tracing::trace!(
            "BOUND r32: value={}, min={}, max={}",
            op1_32, bound_min, bound_max
        );

        // Check if value is outside bounds
        if op1_32 < bound_min || op1_32 > bound_max {
            tracing::debug!("BOUND: fails bounds test (value {} not in [{}, {}])",
                op1_32, bound_min, bound_max);
            // Generate #BR exception (Bound Range Exceeded, vector 5)
            self.interrupt_real_mode(5);
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
    pub(super) fn interrupt_real_mode(&mut self, vector: u8) {
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

        // Boot diagnostic: if we ever vector to 0000:0000 in real mode, BIOS likely
        // hit an unexpected exception/IRQ before IVT was initialized (or IVT reads are broken).
        // Emit a one-time marker to port 0xE9 so the host can see it in headless mode.
        if new_ip == 0 && new_cs == 0 && (self.boot_debug_flags & 0x02) == 0 {
            self.boot_debug_flags |= 0x02;
            self.debug_puts(b"[IVT->0000:0000]\n");
        }
        
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
        
        // Only log non-exception interrupts to reduce spam (exceptions are logged in exception.rs)
        if vector != 0x0d && vector != 0x0e && vector != 0x08 && vector < 0x20 {
            tracing::debug!("INT {:#04x}: vector at {:04x}:{:04x}", vector, new_cs, new_ip);
        }
    }

    // =========================================================================
    // HLT - Halt instruction
    // =========================================================================
    
    /// HLT - Halt CPU until interrupt
    /// In Bochs: Sets activity_state to ActivityStateHlt and raises async_event
    pub fn hlt(&mut self, _instr: &BxInstructionGenerated) {
        // Check if interrupts are disabled (IF=0) - matches Bochs proc_ctrl.cc:206
        if self.get_if() == 0 {
            tracing::warn!("HLT: CPU halted with IF=0 (interrupts disabled) - CPU will be stuck!");
        }
        
        tracing::debug!("HLT: CPU halted, IF={}", self.get_b_if());
        
        // Set activity state to halted (matches Bochs proc_ctrl.cc:203)
        self.activity_state = CpuActivityState::Hlt;
        
        // Set async event to indicate we need to sync and check for interrupts
        // In Bochs, this causes the CPU to return from cpu_loop and check for interrupts
        self.async_event |= BX_ASYNC_EVENT_STOP_TRACE;
    }

    /// CPUID - CPU Identification
    /// Original: bochs/cpu/proc_ctrl.cc:101-131
    /// Returns CPU identification and feature information in EAX, EBX, ECX, EDX
    /// Input: EAX = function number, ECX = sub-function (for some functions)
    pub fn cpuid(&mut self, _instr: &BxInstructionGenerated) {
        let function = self.eax();
        let sub_function = self.ecx();

        let (eax, ebx, ecx, edx) = self.cpuid.get_cpuid_leaf(function, sub_function);
        self.set_eax(eax);
        self.set_ebx(ebx);
        self.set_ecx(ecx);
        self.set_edx(edx);

        tracing::trace!("CPUID(EAX={:#x}, ECX={:#x}): -> EAX={:#x}, EBX={:#x}, ECX={:#x}, EDX={:#x}",
            function, sub_function, eax, ebx, ecx, edx);
    }
}
